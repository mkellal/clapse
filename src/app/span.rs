use std::path::Path;

use crate::traces::event::TraceData;
use ratatui::style::Color;

#[derive(PartialEq, Clone, Copy)]
pub enum SpanType {
    Unit,
    Source,
    Template,
    Class,
    Task,
}

impl SpanType {
    /// Base color for this span type (Catppuccin palette).
    pub fn get_color(&self, horizontal_index: Option<usize>, depth: Option<usize>) -> Color {
        let color = match self {
            SpanType::Unit => {
                let peach = Color::Rgb(255, 169, 127); // Catppuccin Peach
                if let Some(i) = horizontal_index {
                    if i % 2 == 0 {
                        peach
                    } else {
                        Color::Rgb(255, 182, 107) // Darker Peach
                    }
                } else {
                    peach // Default to Peach
                }
            }
            SpanType::Source => Color::Rgb(116, 199, 236), // Catppuccin Sapphire
            SpanType::Class => Color::Rgb(203, 166, 247),  // Catppuccin Mauve
            SpanType::Template => Color::Rgb(249, 226, 175), // Catppuccin Yellow
            SpanType::Task => Color::Rgb(172, 176, 190),   // Catppuccin Surface 2
        };
        if let (Some(i), Some(d)) = (horizontal_index, depth)
            && *self != SpanType::Unit
        {
            crate::widgets::color::span_color(color, d, i)
        } else {
            color
        }
    }
}

pub struct Span {
    pub type_: SpanType,
    pub identifier: String,
    pub label: String,
    pub sublabel: Option<String>,
    pub start_time: f64,
    pub duration: f64,
    pub parent_index: Option<usize>,
    pub children_indices: Vec<usize>,
    /// Global index of the root ancestor (self if root).
    pub root_span_index: usize,
    pub depth: usize,
}

pub fn add_spans(spans: &mut Vec<Span>, data: &TraceData, build_dir: &std::path::PathBuf) {
    spans.extend(data.trace_events.iter().filter_map(|event| {
        if event.ph != "X" {
            return None;
        }
        let name = event.name.clone().unwrap_or_default();
        let args_detail = event
            .args
            .as_ref()
            .and_then(|a| a.detail.clone())
            .unwrap_or_default();
        let cleaned = clean_identifier(&args_detail, build_dir);
        let (type_, identifier, label, sublabel): (SpanType, String, String, Option<String>) =
            match name.as_str() {
                "Source" => (
                    SpanType::Source,
                    args_detail.clone(),
                    cleaned,
                    Some("Inclusion".to_string()),
                ),
                "ParseClass" => (
                    SpanType::Class,
                    args_detail.clone(),
                    cleaned,
                    Some("Parsing".to_string()),
                ),
                "ParseTemplate" => (
                    SpanType::Template,
                    args_detail.clone(),
                    cleaned,
                    Some("Parsing".to_string()),
                ),
                "InstantiateClass" => (
                    SpanType::Class,
                    args_detail.clone(),
                    cleaned,
                    Some("Instantiation".to_string()),
                ),
                "InstantiateTemplate" => (
                    SpanType::Template,
                    args_detail.clone(),
                    cleaned,
                    Some("Instantiation".to_string()),
                ),
                "Frontend"
                | "Backend"
                | "PerformPendingInstantiations"
                | "CodeGen Function"
                | "DebugType" => (SpanType::Task, args_detail.clone(), name, None),
                _ => return None,
            };

        Some(Span {
            type_,
            identifier,
            label,
            sublabel,
            start_time: event.ts,
            duration: event.dur.unwrap_or(0.0),
            children_indices: Vec::new(),
            parent_index: None,
            root_span_index: 0,
            depth: 0,
        })
    }));

    link_spans(spans);
}

fn link_spans(spans: &mut Vec<Span>) {
    // Sort everything after index 0 (the root Unit span stays at the front)
    spans[1..].sort_unstable_by(|a, b| {
        a.start_time
            .partial_cmp(&b.start_time)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.duration
                    .partial_cmp(&a.duration)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    // Reset linkage on all child spans (root at 0 is already clean)
    for span in spans[1..].iter_mut() {
        span.parent_index = None;
        span.children_indices.clear();
        span.depth = 0;
    }

    // active_parents starts with the root span so every top-level span is
    // linked as a child of it automatically.
    let mut active_parents: Vec<usize> = vec![0];

    for i in 1..spans.len() {
        let current_start = spans[i].start_time;

        // Pop parents whose window has closed, but never pop the root (index 0)
        while active_parents.len() > 1 {
            let top_idx = *active_parents.last().unwrap();
            let top_end = spans[top_idx].start_time + spans[top_idx].duration;
            if top_end > current_start + f64::EPSILON {
                break;
            }
            active_parents.pop();
        }

        let parent_idx = *active_parents.last().unwrap();
        spans[i].parent_index = Some(parent_idx);
        spans[parent_idx].children_indices.push(i);
        spans[i].depth = active_parents.len(); // root is depth 0, its children depth 1, etc.

        active_parents.push(i);
    }

    if spans.len() > 1 && spans[0].duration == f64::INFINITY {
        // Finalise root span bounds from its direct children
        let min_start = spans[1..]
            .iter()
            .filter(|s| s.parent_index == Some(0))
            .map(|s| s.start_time)
            .fold(f64::INFINITY, f64::min);
        let max_end = spans[1..]
            .iter()
            .filter(|s| s.parent_index == Some(0))
            .map(|s| s.start_time + s.duration)
            .fold(f64::NEG_INFINITY, f64::max);
        if min_start.is_finite() && max_end.is_finite() {
            spans[0].start_time = min_start;
            spans[0].duration = max_end - min_start;
        }
    }

    // When we have a ninja-derived start time, child span timestamps
    // are relative to the trace start (~0 µs) while the root is at
    // `unit_start` (build-relative µs).  Shift all non-root spans so
    // every span lives in the same coordinate system.
    let unit_start = spans.first().map(|s| s.start_time).unwrap_or(0.0);
    for span in spans[1..].iter_mut() {
        if unit_start != 0.0 {
            span.start_time += unit_start;
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    // ── helpers ──

    fn make_span(type_: SpanType, name: &str, start: f64, dur: f64) -> Span {
        Span {
            type_,
            identifier: name.to_string(),
            label: name.to_string(),
            sublabel: None,
            start_time: start,
            duration: dur,
            parent_index: None,
            children_indices: Vec::new(),
            root_span_index: 0,
            depth: 0,
        }
    }

    // ── last_n_components ──

    #[test]
    fn test_last_n_fewer_than_n() {
        let p = Path::new("/a/b");
        // Absolute path keeps the root "/"
        assert_eq!(last_n_components(p, 5), "/a/b");
    }

    #[test]
    fn test_last_n_exactly_n() {
        let p = Path::new("/a/b/c/d");
        assert_eq!(last_n_components(p, 4), "a/b/c/d");
    }

    #[test]
    fn test_last_n_more_than_n() {
        let p = Path::new("/a/b/c/d/e/f");
        assert_eq!(last_n_components(p, 3), "d/e/f");
    }

    #[test]
    fn test_last_n_single_component() {
        let p = Path::new("/file.cpp");
        // Absolute path keeps the root "/"
        assert_eq!(last_n_components(p, 2), "/file.cpp");
    }

    // ── collapse_template_args ──

    #[test]
    fn test_collapse_no_templates() {
        assert_eq!(collapse_template_args("foo"), "foo");
    }

    #[test]
    fn test_collapse_simple_template() {
        assert_eq!(collapse_template_args("std::vector<int>"), "std::vector<…>");
    }

    #[test]
    fn test_collapse_nested() {
        let input = "std::map<int, std::string>";
        assert_eq!(collapse_template_args(input), "std::map<…>");
    }

    #[test]
    fn test_collapse_multiple_top_level() {
        let input = "A<int>::B<float>";
        assert_eq!(collapse_template_args(input), "A<…>::B<…>");
    }

    #[test]
    fn test_collapse_short_noop() {
        // Under 30 chars, clean_identifier returns as-is; collapse itself
        // doesn't care about length — test that short strings with templates
        // still get collapsed.
        assert_eq!(collapse_template_args("Vec<int>"), "Vec<…>");
    }

    // ── clean_identifier ──

    #[test]
    fn test_clean_identifier_short_non_path() {
        let build = PathBuf::from("/build");
        assert_eq!(clean_identifier("foo", &build), "foo");
    }

    #[test]
    fn test_clean_identifier_long_non_path() {
        let build = PathBuf::from("/build");
        let input = "some_very_long_identifier_that_exceeds_30_chars<int>";
        let expected = "some_very_long_identifier_that_exceeds_30_chars<…>";
        assert_eq!(clean_identifier(input, &build), expected);
    }

    #[test]
    fn test_clean_identifier_path_inside_build_dir() {
        let build = PathBuf::from("/build");
        assert_eq!(clean_identifier("/build/src/file.cpp", &build), "src/file.cpp");
    }

    #[test]
    fn test_clean_identifier_path_outside_build_dir() {
        let build = PathBuf::from("/build");
        let result = clean_identifier("/other/a/b/c/d/e/f/g.cpp", &build);
        assert_eq!(result, "d/e/f/g.cpp");
    }

    // ── clean_path_label ──

    #[test]
    fn test_clean_path_label_inside_short() {
        let build = PathBuf::from("/build");
        assert_eq!(clean_path_label("/build/src/a.cpp", &build), "src/a.cpp");
    }

    #[test]
    fn test_clean_path_label_inside_long() {
        let build = PathBuf::from("/build");
        // Path has 17 chars → under 30 → returns full relative path, not truncated
        let result = clean_path_label("/build/a/b/c/d/e/f/g.cpp", &build);
        assert_eq!(result, "a/b/c/d/e/f/g.cpp");
    }

    #[test]
    fn test_clean_path_label_outside() {
        let build = PathBuf::from("/build");
        let result = clean_path_label("/other/x/y/z/w/v/u/t/s/file.cpp", &build);
        assert_eq!(result, "u/t/s/file.cpp");
    }

    // ── SpanType::get_color ──

    #[test]
    fn test_get_color_unit_default() {
        assert_eq!(SpanType::Unit.get_color(None, None), Color::Rgb(255, 169, 127));
    }

    #[test]
    fn test_get_color_unit_even_index() {
        // Even index → Peach (same as default)
        assert_eq!(
            SpanType::Unit.get_color(Some(0), None),
            Color::Rgb(255, 169, 127)
        );
    }

    #[test]
    fn test_get_color_unit_odd_index() {
        // Odd index → Darker Peach
        assert_eq!(
            SpanType::Unit.get_color(Some(1), None),
            Color::Rgb(255, 182, 107)
        );
    }

    #[test]
    fn test_get_color_non_unit_with_depth() {
        // Source with depth and index → should pass through span_color
        // (we just verify it differs from base Sapphire)
        let base = SpanType::Source.get_color(None, None);
        let modified = SpanType::Source.get_color(Some(0), Some(2));
        // With depth+index, non-Unit types get modified by span_color
        assert_ne!(base, modified);
    }

    #[test]
    fn test_get_color_non_unit_no_depth() {
        // No depth → returns base color unchanged
        assert_eq!(
            SpanType::Source.get_color(None, None),
            Color::Rgb(116, 199, 236)
        );
    }

    // ── link_spans ──

    #[test]
    fn test_link_spans_root_only() {
        let mut spans = vec![make_span(SpanType::Unit, "root", 0.0, f64::INFINITY)];
        link_spans(&mut spans);
        assert_eq!(spans.len(), 1);
        assert!(spans[0].children_indices.is_empty());
        assert_eq!(spans[0].depth, 0);
        assert!(spans[0].parent_index.is_none());
    }

    #[test]
    fn test_link_spans_root_one_child() {
        let mut spans = vec![
            make_span(SpanType::Unit, "root", 0.0, f64::INFINITY),
            make_span(SpanType::Source, "child", 10.0, 5.0),
        ];
        link_spans(&mut spans);
        assert_eq!(spans[1].parent_index, Some(0));
        assert_eq!(spans[1].depth, 1);
        assert_eq!(spans[0].children_indices, vec![1]);
    }

    #[test]
    fn test_link_spans_two_siblings() {
        let mut spans = vec![
            make_span(SpanType::Unit, "root", 0.0, f64::INFINITY),
            make_span(SpanType::Source, "child1", 10.0, 3.0),   // ends at 13
            make_span(SpanType::Source, "child2", 15.0, 4.0),   // starts at 15, after child1
        ];
        link_spans(&mut spans);
        // Both should be children of root
        assert_eq!(spans[1].parent_index, Some(0));
        assert_eq!(spans[2].parent_index, Some(0));
        assert!(spans[0].children_indices.contains(&1));
        assert!(spans[0].children_indices.contains(&2));
    }

    #[test]
    fn test_link_spans_nested() {
        let mut spans = vec![
            make_span(SpanType::Unit, "root", 0.0, f64::INFINITY),
            make_span(SpanType::Source, "parent", 10.0, 50.0),  // ends at 60
            make_span(SpanType::Class, "child", 20.0, 10.0),    // inside parent
        ];
        link_spans(&mut spans);
        // After sort: root(0), parent(1), child(2) since parent starts before child
        // child should be nested under parent
        let parent_idx = spans[2].parent_index;
        assert!(parent_idx.is_some(), "child should have a parent");
        let parent = &spans[parent_idx.unwrap()];
        assert_eq!(parent.identifier, "parent");
        assert_eq!(spans[2].depth, 2); // root=0, parent=1, child=2
    }

    #[test]
    fn test_link_spans_root_inf_duration_adjusted() {
        let mut spans = vec![
            make_span(SpanType::Unit, "root", 0.0, f64::INFINITY),
            make_span(SpanType::Source, "child1", 10.0, 5.0),  // ends at 15
            make_span(SpanType::Source, "child2", 20.0, 8.0),  // ends at 28
        ];
        link_spans(&mut spans);
        // Root should have its start_time and duration computed from children
        assert!((spans[0].start_time - 10.0).abs() < 0.001, "root start should be min child start");
        assert!((spans[0].duration - 18.0).abs() < 0.001, "root dur should be 28-10=18");
    }

    #[test]
    fn test_link_spans_shift_with_unit_start() {
        let mut spans = vec![
            make_span(SpanType::Unit, "root", 1000.0, f64::INFINITY),
            make_span(SpanType::Source, "child", 10.0, 5.0),
        ];
        link_spans(&mut spans);
        // INF duration adjustment runs first: root.start = min child start (10.0),
        // then shift uses adjusted root.start (10.0): child.start = 10.0 + 10.0 = 20.0
        assert!((spans[1].start_time - 20.0).abs() < 0.001);
    }

    #[test]
    fn test_link_spans_unit_start_zero_no_shift() {
        let mut spans = vec![
            make_span(SpanType::Unit, "root", 0.0, 50.0),
            make_span(SpanType::Source, "child", 10.0, 5.0),
        ];
        link_spans(&mut spans);
        // unit_start = 0, no shift
        assert!((spans[1].start_time - 10.0).abs() < 0.001);
    }
}
