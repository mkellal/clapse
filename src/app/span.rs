use std::path::PathBuf;

use crate::traces::event::parse_trace_file;

pub enum SpanType {
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
    pub thread_id: u32,
    pub contained_by_index: Option<usize>,
    pub contains_indices: Vec<usize>,
}

pub fn get_spans(trace_file: &PathBuf) -> Vec<Span> {
    let data = parse_trace_file(&trace_file);
    if data.is_none() {
        return Vec::new();
    }
    let data = data.unwrap();
    let mut spans: Vec<Span> = data
        .trace_events
        .into_iter()
        .filter_map(|event| {
            if event.ph == "X" {
                let name = event.name.clone().unwrap_or_default();
                let args_detail = event.args.unwrap_or_default().detail.unwrap_or_default();
                let (type_, label, details): (SpanType, String, Option<String>) =
                    match name.as_str() {
                        "Source" => (SpanType::Source, args_detail, None),
                        "ParseClass" => (SpanType::Class, args_detail, Some("Parsing".to_string())),
                        "ParseTemplate" => {
                            (SpanType::Template, args_detail, Some("Parsing".to_string()))
                        }
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
                        | "DebugType" => (SpanType::Task, name, Some(args_detail)),
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
                    thread_id: event.tid,
                })
            } else {
                None
            }
        })
        .collect();
    link_spans(&mut spans);
    spans
}

fn link_spans(spans: &mut Vec<Span>) {
    spans.sort_unstable_by(|a, b| {
        a.start_time
            .partial_cmp(&b.start_time)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.duration
                    .partial_cmp(&a.duration)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    let mut active_parents: Vec<usize> = Vec::with_capacity(32);

    for i in 0..spans.len() {
        let current_start = spans[i].start_time;

        while let Some(&top_idx) = active_parents.last() {
            let top_end = spans[top_idx].start_time + spans[top_idx].duration;

            if top_end > current_start + f64::EPSILON {
                break;
            }
            active_parents.pop();
        }

        if let Some(&parent_idx) = active_parents.last() {
            spans[i].contained_by_index = Some(parent_idx);
            spans[parent_idx].contains_indices.push(i);
        }

        active_parents.push(i);
    }
}
