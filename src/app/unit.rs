use rayon::prelude::*;
use std::path::Path;

use crate::{
    app::span::{Span, SpanType, add_spans},
    traces::{
        event::parse_trace_file,
        file::{clean_trace_file_path, get_trace_files},
    },
};

const MAX_LABEL_LEN: usize = 30;

fn clean_identifier(identifier: &str, build_dir: &Path) -> String {
    if identifier.starts_with('/') {
        return clean_path_label(identifier, build_dir);
    }
    if identifier.chars().count() <= MAX_LABEL_LEN {
        return identifier.to_owned();
    }
    collapse_template_args(identifier)
}

fn clean_path_label(path: &str, build_dir: &Path) -> String {
    let p = Path::new(path);
    if let Ok(rel) = p.strip_prefix(build_dir) {
        let s = rel.to_string_lossy().into_owned();
        if s.chars().count() <= MAX_LABEL_LEN {
            return s;
        }
        return last_n_components(rel, 4);
    }
    last_n_components(p, 4)
}

fn last_n_components(p: &Path, n: usize) -> String {
    let components: Vec<_> = p.components().collect();
    let start = components.len().saturating_sub(n);
    components[start..]
        .iter()
        .collect::<std::path::PathBuf>()
        .to_string_lossy()
        .into_owned()
}

fn collapse_template_args(s: &str) -> String {
    let mut result = String::new();
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '<' => {
                if depth == 0 {
                    result.push_str("<\u{2026}>");
                }
                depth += 1;
            }
            '>' => {
                depth = depth.saturating_sub(1);
            }
            _ if depth == 0 => result.push(c),
            _ => {}
        }
    }
    result
}

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

fn build_views_start_time(spans: &[Span]) -> Vec<SpanView> {
    (0..spans.len())
        .map(|i| {
            let index_in_parent = spans[i]
                .parent_index
                .and_then(|pi| {
                    spans[pi]
                        .children_indices
                        .iter()
                        .position(|&ci| ci == i)
                })
                .unwrap_or(0);
            SpanView {
                span_index: i,
                effective_start: spans[i].start_time,
                index_in_parent,
            }
        })
        .collect()
}

fn build_views_duration(spans: &[Span]) -> Vec<SpanView> {
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
    pub absolute_start_time: f64,
    /// Precomputed render/navigation order for StartTime mode.
    pub views_start_time: Vec<SpanView>,
    /// Precomputed render/navigation order for Duration mode.
    pub views_duration: Vec<SpanView>,
}

impl Unit {
    pub fn views(&self, order_by: OrderBy) -> &[SpanView] {
        match order_by {
            OrderBy::StartTime => &self.views_start_time,
            OrderBy::Duration => &self.views_duration,
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

pub fn get_units(build_dir: &std::path::PathBuf) -> Vec<Unit> {
    let trace_files = get_trace_files(build_dir);
    trace_files
        .par_iter()
        .filter_map(|trace_file| {
            let name = clean_trace_file_path(trace_file, build_dir)
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let trace_file_str = trace_file.to_string_lossy().to_string();
            let mut spans = vec![Span {
                type_: SpanType::Unit,
                identifier: trace_file_str.clone(),
                label: name.clone(),
                sublabel: Some("Translation Unit".to_string()),
                start_time: 0.0,
                duration: f64::INFINITY,
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
            add_spans(&mut spans, &data);

            for span in spans[1..].iter_mut() {
                span.label = clean_identifier(&span.identifier, build_dir);
            }

            let views_start_time = build_views_start_time(&spans);
            let views_duration = build_views_duration(&spans);

            Some(Unit {
                name,
                trace_file: trace_file.clone(),
                spans,
                absolute_start_time: data.beginning_of_time,
                views_start_time,
                views_duration,
            })
        })
        .collect()
}
