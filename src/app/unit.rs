use rayon::prelude::*;
use std::path::Path;

use crate::{
    app::span::{Span, SpanType, add_spans},
    traces::{
        event::parse_trace_file,
        file::{clean_trace_file_path, get_trace_files},
    },
};


// ---------------------------------------------------------------------------
// OrderBy + SpanEntry
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum OrderBy {
    #[default]
    StartTime,
    Duration,
}

/// Positioning data for one span, precomputed for a given OrderBy mode.
pub struct SpanView {
    /// Index into the unit's spans slice.
    pub span_index: usize,
    /// Start time to use for x-positioning.
    pub effective_start: f64,
    /// Position among siblings (for checkerboard coloring).
    pub index_in_parent: usize,
}

fn build_views_by_start_time(spans: &[Span]) -> Vec<SpanView> {
    (0..spans.len())
        .map(|i| {
            let index_in_parent = spans[i]
                .parent_index
                .and_then(|pi| spans[pi].children_indices.iter().position(|&ci| ci == i))
                .unwrap_or(0);
            SpanView {
                span_index: i,
                effective_start: spans[i].start_time,
                index_in_parent,
            }
        })
        .collect()
}

fn build_views_by_duration(spans: &[Span]) -> Vec<SpanView> {
    let mut entries = Vec::with_capacity(spans.len());

    let mut roots: Vec<usize> = (0..spans.len())
        .filter(|&i| spans[i].parent_index.is_none())
        .collect();
    roots.sort_by(|&a, &b| {
        spans[b]
            .duration
            .partial_cmp(&spans[a].duration)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut cursor = 0.0f64;
    for (sib_pos, &r) in roots.iter().enumerate() {
        visit_duration(spans, r, cursor, sib_pos, &mut entries);
        cursor += spans[r].duration;
    }

    entries
}

fn visit_duration(
    spans: &[Span],
    i: usize,
    virtual_start: f64,
    index_in_parent: usize,
    entries: &mut Vec<SpanView>,
) {
    entries.push(SpanView {
        span_index: i,
        effective_start: virtual_start,
        index_in_parent,
    });

    let mut children = spans[i].children_indices.clone();
    children.sort_by(|&a, &b| {
        spans[b]
            .duration
            .partial_cmp(&spans[a].duration)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut cursor = virtual_start;
    for (sib_pos, &c) in children.iter().enumerate() {
        visit_duration(spans, c, cursor, sib_pos, entries);
        cursor += spans[c].duration;
    }
}

// ---------------------------------------------------------------------------
// Unit
// ---------------------------------------------------------------------------

pub struct Unit {
    pub name: String,
    pub trace_file: std::path::PathBuf,
    pub spans: Vec<Span>,
    /// Precomputed render/navigation order for StartTime mode.
    pub views_by_start_time: Vec<SpanView>,
    /// Precomputed render/navigation order for Duration mode.
    pub views_by_duration: Vec<SpanView>,
}

impl Unit {
    pub fn views(&self, order_by: OrderBy) -> &[SpanView] {
        match order_by {
            OrderBy::StartTime => &self.views_by_start_time,
            OrderBy::Duration => &self.views_by_duration,
        }
    }
}

pub enum FollowingSpanDirection {
    Next,
    Previous,
    Parent,
    Child,
}

/// Horizontal-only direction used by `get_following_span_index`.
#[derive(Clone, Copy)]
pub enum HorizontalDirection {
    Next,
    Previous,
}

impl Unit {
    pub fn get_parent_span(&self, span: &Span) -> Option<&Span> {
        span.parent_index
            .and_then(|parent_index| self.spans.get(parent_index))
    }

    /// Returns the index of the next/previous sibling of `span_index` in the
    /// order defined by `views`, considering only displayed spans.
    pub fn get_following_span_index(
        &self,
        span_index: usize,
        direction: HorizontalDirection,
        views: &[SpanView],
    ) -> Option<usize> {
        let span = self.spans.get(span_index)?;
        let parent_index = span.parent_index?;
        let parent = self.spans.get(parent_index)?;

        // Siblings in display order, filtered to only those that were rendered.
        let siblings: Vec<usize> = views
            .iter()
            .filter(|e| {
                self.spans[e.span_index].parent_index == Some(parent_index)
                    && self.spans[e.span_index].was_displayed
            })
            .map(|e| e.span_index)
            .collect();

        let pos = siblings.iter().position(|&si| si == span_index)?;
        let shift = match direction {
            HorizontalDirection::Next => 1,
            HorizontalDirection::Previous => -1,
        };
        let new_pos = (pos as isize + shift) as usize;
        match siblings.get(new_pos).copied() {
            Some(si) => Some(si),
            None => {
                if parent.parent_index.is_none() {
                    None
                } else {
                    self.get_following_span_index(parent_index, direction, views)
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Ninja log parsing
// ---------------------------------------------------------------------------

/// A build record parsed from a `.ninja_log` file.
pub struct UnitTrace {
    /// Output path of the build target (used as the unit identifier).
    pub identifier: String,
    /// Build start time in milliseconds.
    pub start_ms: u64,
    /// Build end time in milliseconds.
    pub end_ms: u64,
}

/// Parses a ninja log file (`.ninja_log`) and returns one `UnitTrace` per
/// entry. Duplicate outputs are de-duplicated by keeping the last occurrence
/// (matching ninja's own behaviour).
///
/// The ninja log v5 format is tab-separated:
/// ```text
/// # ninja log v5
/// <start_ms>\t<end_ms>\t<restat_mtime>\t<output>\t<cmdhash>
/// ```
pub fn parse_ninja_log(path: &Path) -> Option<Vec<UnitTrace>> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines = content.lines();

    let header = lines.next()?;
    if !header.starts_with("# ninja log v") {
        return None;
    }

    // Use a map keyed by identifier to keep only the last entry per output,
    // mirroring ninja's own de-duplication behaviour.
    let mut seen: std::collections::HashMap<String, UnitTrace> = std::collections::HashMap::new();

    for line in lines {
        // Skip blank lines or comment lines.
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut fields = line.splitn(5, '\t');
        let start_ms: u64 = fields.next().and_then(|s| s.parse().ok())?;
        let end_ms: u64 = fields.next().and_then(|s| s.parse().ok())?;
        let _restat_mtime = fields.next();
        let identifier = fields.next()?.to_owned();

        seen.insert(
            identifier.clone(),
            UnitTrace {
                identifier,
                start_ms,
                end_ms,
            },
        );
    }

    let mut traces: Vec<UnitTrace> = seen.into_values().collect();
    traces.sort_by_key(|t| t.start_ms);
    Some(traces)
}

pub fn get_units(build_dir: &std::path::PathBuf) -> Vec<Unit> {
    // Parse the ninja log once before the parallel section so every thread
    // can look up timings without re-reading the file.
    let ninja_log_path = build_dir.join(".ninja_log");
    let ninja_traces: Vec<UnitTrace> = parse_ninja_log(&ninja_log_path).unwrap_or_default();

    let trace_files = get_trace_files(build_dir);
    trace_files
        .par_iter()
        .filter_map(|trace_file| {
            let name = clean_trace_file_path(trace_file, build_dir)
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let identifier = trace_file.with_extension("o").to_string_lossy().to_string();

            // Match against ninja log: unit identifier must end with the
            // ninja trace identifier (e.g. "CMakeFiles/foo.dir/foo.cpp.o").
            let ninja_match = ninja_traces
                .iter()
                .find(|nt| identifier.ends_with(&nt.identifier));

            // Ninja log times are in ms; trace timestamps are in µs.
            let (unit_start, unit_duration) = match ninja_match {
                Some(nt) => (
                    nt.start_ms as f64 * 1000.0,
                    (nt.end_ms - nt.start_ms) as f64 * 1000.0,
                ),
                None => (0.0, f64::INFINITY),
            };

            let mut spans = vec![Span {
                type_: SpanType::Unit,
                identifier,
                label: name.clone(),
                sublabel: Some("Translation Unit".to_string()),
                start_time: unit_start,
                duration: unit_duration,
                parent_index: None,
                children_indices: Vec::new(),
                index_in_unit: 0,
                depth: 0,
                has_core_cells: false,
                was_displayed: false,
            }];

            let data = match parse_trace_file(trace_file) {
                Some(d) => d,
                None => return None,
            };

            add_spans(&mut spans, &data, build_dir);

            let views_start_time = build_views_by_start_time(&spans);
            let views_duration = build_views_by_duration(&spans);

            Some(Unit {
                name,
                trace_file: trace_file.clone(),
                spans,
                views_by_start_time: views_start_time,
                views_by_duration: views_duration,
            })
        })
        .collect()
}
