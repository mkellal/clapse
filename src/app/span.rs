use std::path::Path;

use crate::traces::event::TraceData;
use ratatui::style::Color;

pub enum SpanType {
    Unit,
    Source,
    Template,
    Class,
    Task,
}

impl SpanType {
    /// Base color for this span type (Catppuccin palette).
    pub fn base_color(&self) -> Color {
        match self {
            SpanType::Unit => Color::Rgb(250, 179, 135), // Catppuccin Peach
            SpanType::Source => Color::Rgb(116, 199, 236), // Catppuccin Sapphire
            SpanType::Class => Color::Rgb(203, 166, 247), // Catppuccin Mauve
            SpanType::Template => Color::Rgb(249, 226, 175), // Catppuccin Yellow
            SpanType::Task => Color::Rgb(172, 176, 190), // Catppuccin Subtext0
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
    pub index_in_unit: usize,
    pub depth: usize,
    /// Set after each render: true if this span occupied at least one full terminal cell.
    pub has_core_cells: bool,
    /// Set after each render: true if this span was rendered at all (including partial chars).
    pub was_displayed: bool,
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
        let (type_, identifier, label, sublabel): (SpanType, String, String, Option<String>) =
            match name.as_str() {
                "Source" => (
                    SpanType::Source,
                    args_detail.clone(),
                    clean_identifier(&args_detail, build_dir),
                    Some("Inclusion".to_string()),
                ),
                "ParseClass" => (
                    SpanType::Class,
                    args_detail.clone(),
                    clean_identifier(&args_detail, build_dir),
                    Some("Parsing".to_string()),
                ),
                "ParseTemplate" => (
                    SpanType::Template,
                    args_detail.clone(),
                    clean_identifier(&args_detail, build_dir),
                    Some("Parsing".to_string()),
                ),
                "InstantiateClass" => (
                    SpanType::Class,
                    args_detail.clone(),
                    clean_identifier(&args_detail, build_dir),
                    Some("Instantiation".to_string()),
                ),
                "InstantiateTemplate" => (
                    SpanType::Template,
                    args_detail.clone(),
                    clean_identifier(&args_detail, build_dir),
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
            index_in_unit: 0,
            depth: 0,
            has_core_cells: false,
            was_displayed: false,
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
    for (i, span) in spans[1..].iter_mut().enumerate() {
        span.parent_index = None;
        span.children_indices.clear();
        span.index_in_unit = i + 1; // +1 because we skipped the root span at index 0
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
