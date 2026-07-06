use std::path::Path;

use crate::{
    app::span::{Span, SpanType, add_spans},
    traces::{
        event::parse_trace_file,
        file::{clean_trace_file_path, get_trace_files},
    },
};

// ---------------------------------------------------------------------------
// OrderBy + SpanView
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum OrderBy {
    #[default]
    StartTime,
    Duration,
}

/// Positioning data for one span, precomputed for a given OrderBy mode.
#[derive(Clone, Default)]
pub struct SpanView {
    /// Global index into the flat spans array.
    pub span_index: usize,
    /// Start time to use for x-positioning.
    pub effective_start: f64,
    /// Position among siblings (for checkerboard coloring).
    pub index_in_parent: usize,
    /// Set after each render: true if this span occupied at least one full terminal cell.
    pub has_core_cells: bool,
    /// Set after each render: true if this span was rendered at all (including partial chars).
    pub was_displayed: bool,
}

#[derive(Clone, Default)]
pub struct AggregateSpanView {
    pub view: SpanView,
    pub count: usize,
}

pub trait DetailProvider {
    fn span_index(&self) -> usize;
    fn count(&self) -> Option<usize>;
}

impl DetailProvider for SpanView {
    fn span_index(&self) -> usize {
        self.span_index
    }
    fn count(&self) -> Option<usize> {
        None
    }
}

impl DetailProvider for AggregateSpanView {
    fn span_index(&self) -> usize {
        self.view.span_index
    }
    fn count(&self) -> Option<usize> {
        Some(self.count)
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

// ---------------------------------------------------------------------------
// Navigation helpers (standalone, work with global flat spans array)
// ---------------------------------------------------------------------------

/// Returns the index of the next/previous sibling of `span_index` in `views`,
/// considering only displayed spans.
pub fn get_following_span_index(
    spans: &[Span],
    views: &[SpanView],
    span_index: usize,
    direction: HorizontalDirection,
) -> Option<usize> {
    let span = spans.get(span_index)?;
    let parent_index = span.parent_index?;
    let parent = spans.get(parent_index)?;

    let mut seen = std::collections::HashSet::new();
    let siblings: Vec<usize> = views
        .iter()
        .filter(|e| spans[e.span_index].parent_index == Some(parent_index) && e.was_displayed)
        .map(|e| e.span_index)
        .filter(|&si| seen.insert(si))
        .collect();

    let pos = siblings.iter().position(|&si| si == span_index)?;
    let shift: isize = match direction {
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
                get_following_span_index(spans, views, parent_index, direction)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// View building
// ---------------------------------------------------------------------------

fn collect_subtree(spans: &[Span], root: usize) -> Vec<usize> {
    let mut result = Vec::new();
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        result.push(i);
        for &ci in &spans[i].children_indices {
            stack.push(ci);
        }
    }
    result
}

fn build_subtree_start_time(
    spans: &[Span],
    root: usize,
    position_in_track: usize,
) -> Vec<SpanView> {
    let mut indices = collect_subtree(spans, root);
    indices.sort_unstable_by(|&a, &b| {
        spans[a]
            .start_time
            .partial_cmp(&spans[b].start_time)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    indices
        .into_iter()
        .map(|i| SpanView {
            span_index: i,
            effective_start: spans[i].start_time,
            index_in_parent: position_in_track,
            has_core_cells: false,
            was_displayed: false,
        })
        .collect()
}

fn visit_duration_global(
    spans: &[Span],
    i: usize,
    virtual_start: f64,
    index_in_parent: usize,
    _root_span_index: usize,
    _position_in_track: usize,
    entries: &mut Vec<SpanView>,
) {
    entries.push(SpanView {
        span_index: i,
        effective_start: virtual_start,
        index_in_parent,
        has_core_cells: false,
        was_displayed: false,
    });

    let mut children = spans[i].children_indices.clone();
    children.sort_unstable_by(|&a, &b| {
        spans[b]
            .duration
            .partial_cmp(&spans[a].duration)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let mut cursor = virtual_start;
    for (sib_pos, &c) in children.iter().enumerate() {
        visit_duration_global(
            spans,
            c,
            cursor,
            sib_pos,
            _root_span_index,
            _position_in_track,
            entries,
        );
        cursor += spans[c].duration;
    }
}

/// Build both orderings for all tracks from the flat spans array.
///
/// `track_roots[i]` is the list of root span global indices for track i.
/// Returns `(tracks_by_start_time, tracks_by_duration)`.
pub fn build_track_views(
    spans: &[Span],
    track_roots: &[Vec<usize>],
) -> (Vec<Vec<SpanView>>, Vec<Vec<SpanView>>) {
    let by_start_time: Vec<Vec<SpanView>> = track_roots
        .iter()
        .map(|roots| {
            roots
                .iter()
                .enumerate()
                .flat_map(|(pos, &root)| build_subtree_start_time(spans, root, pos))
                .collect()
        })
        .collect();

    let by_duration: Vec<Vec<SpanView>> = track_roots
        .iter()
        .map(|roots| {
            let mut sorted_roots: Vec<usize> = roots.clone();
            sorted_roots.sort_unstable_by(|&a, &b| {
                spans[b]
                    .duration
                    .partial_cmp(&spans[a].duration)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            let mut views = Vec::new();
            let mut cursor = 0.0f64;
            for (pos, &root) in sorted_roots.iter().enumerate() {
                let position_in_track = pos;
                visit_duration_global(spans, root, cursor, 0, root, position_in_track, &mut views);
                cursor += spans[root].duration;
            }
            views
        })
        .collect();

    (by_start_time, by_duration)
}

// ---------------------------------------------------------------------------
// Ninja log parsing
// ---------------------------------------------------------------------------

/// A build record parsed from a `.ninja_log` file.
pub struct UnitTrace {
    pub identifier: String,
    pub start_ms: u64,
    pub end_ms: u64,
}

/// Parses a ninja log file (`.ninja_log`) and returns one `UnitTrace` per entry.
pub fn parse_ninja_log(path: &Path) -> Option<Vec<UnitTrace>> {
    let content = std::fs::read_to_string(path).ok()?;
    let mut lines = content.lines();

    let header = lines.next()?;
    if !header.starts_with("# ninja log v") {
        return None;
    }

    let mut seen: std::collections::HashMap<String, UnitTrace> = std::collections::HashMap::new();

    for line in lines {
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

// ---------------------------------------------------------------------------
// Loading progress
// ---------------------------------------------------------------------------

/// Progress information sent during span loading.
#[derive(Clone, Debug)]
pub struct LoadProgress {
    /// Bytes processed so far.
    pub bytes_processed: u64,
    /// Total bytes across all trace files.
    pub total_bytes: u64,
    /// Number of files processed so far.
    pub files_processed: usize,
    /// Total number of trace files.
    pub total_files: usize,
}

// ---------------------------------------------------------------------------
// Loading all spans into a flat Vec<Span>
// ---------------------------------------------------------------------------

/// Load all trace files in `build_dir` and return a single flat `Vec<Span>`
/// with globally-consistent parent/children indices.
/// Reports progress via `progress_tx` after each file is processed.
pub fn load_spans_with_progress(
    build_dir: &std::path::Path,
    progress_tx: std::sync::mpsc::Sender<LoadProgress>,
) -> Vec<Span> {
    let ninja_log_path = build_dir.join(".ninja_log");
    let ninja_traces: Vec<UnitTrace> = parse_ninja_log(&ninja_log_path).unwrap_or_default();

    let trace_files = get_trace_files(build_dir);

    // Compute total bytes for progress tracking.
    let total_bytes: u64 = trace_files
        .iter()
        .filter_map(|p| std::fs::metadata(p).ok().map(|m| m.len()))
        .sum();

    let total_files = trace_files.len();

    // Send initial progress.
    let _ = progress_tx.send(LoadProgress {
        bytes_processed: 0,
        total_bytes,
        files_processed: 0,
        total_files,
    });

    let mut bytes_processed: u64 = 0;
    let mut unit_spans_list: Vec<Vec<Span>> = Vec::with_capacity(total_files);

    // Process files sequentially so we can report smooth progress.
    for (file_idx, trace_file) in trace_files.iter().enumerate() {
        let file_size = std::fs::metadata(trace_file)
            .ok()
            .map(|m| m.len())
            .unwrap_or(0);

        let result = (|| {
            let name = clean_trace_file_path(trace_file, build_dir)
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let identifier = trace_file.with_extension("o").to_string_lossy().to_string();

            let ninja_match = ninja_traces
                .iter()
                .find(|nt| identifier.ends_with(&nt.identifier));

            let data = parse_trace_file(trace_file)?;

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
                label: name,
                sublabel: Some("Translation Unit".to_string()),
                start_time: unit_start,
                duration: unit_duration,
                parent_index: None,
                children_indices: Vec::new(),
                root_span_index: 0,
                depth: 0,
            }];

            add_spans(&mut spans, &data, build_dir);

            Some(spans)
        })();

        bytes_processed += file_size;

        if let Some(spans) = result {
            unit_spans_list.push(spans);
        }

        let _ = progress_tx.send(LoadProgress {
            bytes_processed,
            total_bytes,
            files_processed: file_idx + 1,
            total_files,
        });
    }

    // Merge into flat array, adjusting all indices by the unit's offset.
    let mut all_spans: Vec<Span> = Vec::new();
    for mut unit_spans in unit_spans_list {
        let offset = all_spans.len();
        for span in unit_spans.iter_mut() {
            span.root_span_index = offset;
            if let Some(pi) = span.parent_index.as_mut() {
                *pi += offset;
            }
            for ci in span.children_indices.iter_mut() {
                *ci += offset;
            }
        }
        all_spans.extend(unit_spans);
    }
    all_spans
}

/// Convenience wrapper that loads spans without progress reporting.
#[allow(dead_code)]
pub fn load_spans(build_dir: &std::path::Path) -> Vec<Span> {
    let (tx, _rx) = std::sync::mpsc::channel();
    load_spans_with_progress(build_dir, tx)
}

// ---------------------------------------------------------------------------
// Scheduling
// ---------------------------------------------------------------------------

/// Greedy scheduling of root spans (those with `parent_index == None`) into
/// non-overlapping tracks.
///
/// Returns `track_roots[i]` = list of root span global indices for track `i`,
/// ordered by assignment time.  Spans with `duration == f64::INFINITY` are
/// skipped (no ninja-log match).
pub fn schedule_spans(spans: &[Span]) -> Vec<Vec<usize>> {
    let mut jobs: Vec<(f64, f64, usize)> = spans
        .iter()
        .enumerate()
        .filter(|(_, s)| s.parent_index.is_none() && s.duration.is_finite())
        .map(|(i, s)| (s.start_time, s.start_time + s.duration, i))
        .collect();

    jobs.sort_by(|a, b| {
        a.0.partial_cmp(&b.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });

    let mut track_ends: Vec<f64> = Vec::new();
    let mut tracks: Vec<Vec<usize>> = Vec::new();

    for (start, end, span_idx) in jobs {
        let slot = track_ends
            .iter()
            .enumerate()
            .filter(|&(_, te)| *te <= start)
            .min_by(|&(_, a), &(_, b)| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal))
            .map(|(i, _)| i);

        match slot {
            Some(i) => {
                track_ends[i] = end;
                tracks[i].push(span_idx);
            }
            None => {
                track_ends.push(end);
                tracks.push(vec![span_idx]);
            }
        }
    }

    tracks
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_root_span(start: f64, duration: f64) -> Span {
        Span {
            type_: SpanType::Unit,
            identifier: String::new(),
            label: String::new(),
            sublabel: None,
            start_time: start,
            duration,
            parent_index: None,
            children_indices: Vec::new(),
            root_span_index: 0,
            depth: 0,
        }
    }

    fn make_spans(starts_and_durations: &[(f64, f64)]) -> Vec<Span> {
        starts_and_durations
            .iter()
            .enumerate()
            .map(|(i, &(start, dur))| {
                let mut s = make_root_span(start, dur);
                s.root_span_index = i;
                s
            })
            .collect()
    }

    #[test]
    fn empty_input() {
        assert!(schedule_spans(&[]).is_empty());
    }

    #[test]
    fn single_span() {
        let spans = make_spans(&[(0.0, 10.0)]);
        let tracks = schedule_spans(&spans);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0], vec![0]);
    }

    #[test]
    fn sequential_spans_fit_on_one_track() {
        let spans = make_spans(&[(0.0, 10.0), (10.0, 10.0), (20.0, 10.0)]);
        let tracks = schedule_spans(&spans);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0], vec![0, 1, 2]);
    }

    #[test]
    fn parallel_spans_get_separate_tracks() {
        let spans = make_spans(&[(0.0, 10.0), (0.0, 10.0)]);
        let tracks = schedule_spans(&spans);
        assert_eq!(tracks.len(), 2);
    }

    #[test]
    fn interleaved_scheduling() {
        let spans = make_spans(&[
            (0.0, 10.0),  // 0: [0..10]
            (5.0, 5.0),   // 1: [5..10]
            (10.0, 10.0), // 2: [10..20]
            (15.0, 10.0), // 3: [15..25]
        ]);
        let tracks = schedule_spans(&spans);
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0], vec![0, 2]);
        assert_eq!(tracks[1], vec![1, 3]);
    }

    #[test]
    fn spans_without_ninja_log_are_skipped() {
        let spans = make_spans(&[(0.0, 10.0), (0.0, f64::INFINITY)]);
        let tracks = schedule_spans(&spans);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0], vec![0]);
    }
}
