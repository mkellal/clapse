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
    pub spans: Vec<Span>,
    pub views_by_start_time: Vec<SpanView>,
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
    let ninja_log_path = build_dir.join(".ninja_logss");
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

            let data = match parse_trace_file(trace_file) {
                Some(d) => d,
                None => return None,
            };

            // Ninja log times are in ms; trace timestamps are in µs.
            let (unit_start, unit_duration) = match ninja_match {
                Some(nt) => (
                    nt.start_ms as f64 * 1000.0,
                    (nt.end_ms - nt.start_ms) as f64 * 1000.0,
                ),
                None => (data.beginning_of_time, f64::INFINITY),
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

            add_spans(&mut spans, &data, build_dir);

            let views_start_time = build_views_by_start_time(&spans);
            let views_duration = build_views_by_duration(&spans);

            Some(Unit {
                spans,
                views_by_start_time: views_start_time,
                views_by_duration: views_duration,
            })
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Scheduling
// ---------------------------------------------------------------------------

/// Mimics ninja's greedy scheduling.
///
/// Each unit's root span (`spans[0]`) carries the ninja-log start time and
/// duration. Units without a ninja-log match (`duration == f64::INFINITY`) are
/// skipped.
///
/// Returns a vector of threads, where each thread is a vector of unit indices
/// (into `units`) ordered by their assignment time.
pub fn schedule_units(units: &[Unit]) -> Vec<Vec<usize>> {
    // Collect schedulable units: (start, end, original index).
    let mut jobs: Vec<(f64, f64, usize)> = units
        .iter()
        .enumerate()
        .filter_map(|(i, u)| {
            let root = u.spans.first()?;
            if root.duration.is_infinite() {
                return None;
            }
            Some((root.start_time, root.start_time + root.duration, i))
        })
        .collect();

    // Sort by start time, then by end time for stable ordering.
    jobs.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });

    // Each entry tracks the end time of the last job assigned to that thread.
    let mut thread_ends: Vec<f64> = Vec::new();
    let mut threads: Vec<Vec<usize>> = Vec::new();

    for (start, end, unit_idx) in jobs {
        // Find the thread that became free earliest and is free by `start`.
        let slot = thread_ends
            .iter()
            .enumerate()
            .filter(|&(_, te)| *te <= start)
            .min_by(|&(_, a), &(_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i);

        match slot {
            Some(i) => {
                thread_ends[i] = end;
                threads[i].push(unit_idx);
            }
            None => {
                thread_ends.push(end);
                threads.push(vec![unit_idx]);
            }
        }
    }

    threads
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::span::SpanType;

    fn make_unit(start: f64, duration: f64) -> Unit {
        let spans = vec![Span {
            type_: SpanType::Unit,
            identifier: String::new(),
            label: String::new(),
            sublabel: None,
            start_time: start,
            duration,
            parent_index: None,
            children_indices: Vec::new(),
            index_in_unit: 0,
            depth: 0,
            has_core_cells: false,
            was_displayed: false,
        }];
        let views_start_time = build_views_by_start_time(&spans);
        let views_duration = build_views_by_duration(&spans);
        Unit {
            spans,
            views_by_start_time: views_start_time,
            views_by_duration: views_duration,
        }
    }

    #[test]
    fn empty_input() {
        assert!(schedule_units(&[]).is_empty());
    }

    #[test]
    fn single_unit() {
        let units = vec![make_unit(0.0, 10.0)];
        let threads = schedule_units(&units);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0], vec![0]);
    }

    #[test]
    fn sequential_units_fit_on_one_thread() {
        // [0..10], [10..20], [20..30] — no overlap, should all land on one thread.
        let units = vec![
            make_unit(0.0, 10.0),
            make_unit(10.0, 10.0),
            make_unit(20.0, 10.0),
        ];
        let threads = schedule_units(&units);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0], vec![0, 1, 2]);
    }

    #[test]
    fn parallel_units_get_separate_threads() {
        // [0..10], [0..10] — fully overlapping, need two threads.
        let units = vec![make_unit(0.0, 10.0), make_unit(0.0, 10.0)];
        let threads = schedule_units(&units);
        assert_eq!(threads.len(), 2);
    }

    #[test]
    fn interleaved_scheduling() {
        // unit 0: [0..10], unit 1: [5..10], unit 2: [10..20], unit 3: [15..25]
        //
        // unit 0 → Thread 0 (only thread, ends 10)
        // unit 1 → Thread 1 (Thread 0 still busy at t=5, ends 10)
        // unit 2 → Thread 0 (both free at t=10, pick earliest-ending = Thread 0, ends 20)
        // unit 3 → Thread 1 (Thread 1 free at t=15 (ends 10), Thread 0 busy (ends 20))
        //
        // Expected: Thread 0 = [0, 2], Thread 1 = [1, 3]
        let units = vec![
            make_unit(0.0, 10.0),  // unit 0: [0..10]
            make_unit(5.0, 5.0),   // unit 1: [5..10]
            make_unit(10.0, 10.0), // unit 2: [10..20]
            make_unit(15.0, 10.0), // unit 3: [15..25]
        ];
        let threads = schedule_units(&units);
        assert_eq!(threads.len(), 2);
        assert_eq!(threads[0], vec![0, 2]);
        assert_eq!(threads[1], vec![1, 3]);
    }

    #[test]
    fn units_without_ninja_log_are_skipped() {
        let units = vec![
            make_unit(0.0, 10.0),
            make_unit(0.0, f64::INFINITY), // no ninja log match
        ];
        let threads = schedule_units(&units);
        // Only the first unit is scheduled.
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0], vec![0]);
    }
}
