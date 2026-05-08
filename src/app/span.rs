use crate::traces::event::TraceData;

pub enum SpanType {
    Unit,
    Source,
    Template,
    Class,
    Task,
}
pub struct Span {
    pub type_: SpanType,
    pub label: String,
    pub details: Option<String>,
    pub start_time: f64,
    pub duration: f64,
    pub contained_by_index: Option<usize>,
    pub contains_indices: Vec<usize>,
    pub index_in_unit: usize,
    pub depth: usize,
    /// Set after each render: true if this span occupied at least one full terminal cell.
    pub has_core_cells: bool,
    /// Set after each render: true if this span was rendered at all (including partial chars).
    pub was_displayed: bool,
}

pub fn add_spans(spans: &mut Vec<Span>, data: &TraceData) {
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
        let (type_, label, details): (SpanType, String, Option<String>) = match name.as_str() {
            "Source" => (
                SpanType::Source,
                args_detail.clone(),
                Some("Inclusion".to_string()),
            ),
            "ParseClass" => (SpanType::Class, args_detail, Some("Parsing".to_string())),
            "ParseTemplate" => (SpanType::Template, args_detail, Some("Parsing".to_string())),
            "InstantiateClass" => (
                SpanType::Class,
                args_detail,
                Some("Instantiation".to_string()),
            ),
            "InstantiateTemplate" => (
                SpanType::Template,
                args_detail,
                Some("Instantiation".to_string()),
            ),
            "Frontend"
            | "Backend"
            | "PerformPendingInstantiations"
            | "CodeGen Function"
            | "DebugType" => (SpanType::Task, name, Some(args_detail.clone())),
            _ => return None,
        };
        Some(Span {
            type_,
            label,
            details,
            start_time: event.ts,
            duration: event.dur.unwrap_or(0.0),
            contains_indices: Vec::new(),
            contained_by_index: None,
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
        span.contained_by_index = None;
        span.contains_indices.clear();
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
        spans[i].contained_by_index = Some(parent_idx);
        spans[parent_idx].contains_indices.push(i);
        spans[i].depth = active_parents.len(); // root is depth 0, its children depth 1, etc.

        active_parents.push(i);
    }

    if spans.len() > 1 {
        // Finalise root span bounds from its direct children
        let min_start = spans[1..]
            .iter()
            .filter(|s| s.contained_by_index == Some(0))
            .map(|s| s.start_time)
            .fold(f64::INFINITY, f64::min);
        let max_end = spans[1..]
            .iter()
            .filter(|s| s.contained_by_index == Some(0))
            .map(|s| s.start_time + s.duration)
            .fold(f64::NEG_INFINITY, f64::max);
        if min_start.is_finite() && max_end.is_finite() {
            spans[0].start_time = min_start;
            spans[0].duration = max_end - min_start;
        }
        // set
    }
}
