use rayon::prelude::*;

use crate::{
    app::span::{Span, SpanType, add_spans},
    traces::{
        event::parse_trace_file,
        file::{clean_trace_file_path, get_trace_files},
    },
};

pub struct Unit {
    pub name: String,
    pub trace_file: std::path::PathBuf,
    pub spans: Vec<Span>,
    pub absolute_start_time: f64,
}

pub enum FollowingSpanDirection {
    Next,
    Previous,
}

impl Unit {
    pub fn get_span(&self, span_index: usize) -> Option<&Span> {
        self.spans.get(span_index)
    }

    pub fn get_parent_span(&self, span: &Span) -> Option<&Span> {
        span.contained_by_index
            .and_then(|parent_index| self.spans.get(parent_index))
    }

    pub fn get_child_spans(&self, span: &Span) -> Vec<&Span> {
        span.contains_indices
            .iter()
            .filter_map(|&child_index| self.spans.get(child_index))
            .collect()
    }

    pub fn get_following_span_index(
        &self,
        span_index: usize,
        direction: FollowingSpanDirection,
    ) -> Option<usize> {
        // get sibling spans (those with the same parent) and find the one immediately after `span`.
        let span = self.spans.get(span_index)?;
        let parent_index = span.contained_by_index?;
        let parent = self.spans.get(parent_index)?;
        let siblings = self.get_child_spans(parent);
        let pos = siblings
            .iter()
            .position(|&s| s.index_in_unit == span_index)?;
        let shift = match direction {
            FollowingSpanDirection::Next => 1,
            FollowingSpanDirection::Previous => -1,
        };
        let new_pos = (pos as isize + shift) as usize;
        let subsequent = siblings.get(new_pos).copied();
        match subsequent {
            Some(s) => Some(s.index_in_unit),
            None => {
                // if there is no next sibling, return the parent (unless it's the root).
                if parent.contained_by_index.is_none() {
                    None
                } else {
                    self.get_following_span_index(parent_index, direction)
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

            // Seed the vec with the root Unit span; add_spans will populate the
            // rest and finalise the root's start_time / duration.
            let mut spans = vec![Span {
                type_: SpanType::Unit,
                label: name.clone(),
                details: Some(trace_file.to_string_lossy().to_string()),
                start_time: 0.0,
                duration: f64::INFINITY,
                contained_by_index: None,
                contains_indices: Vec::new(),
                index_in_unit: 0,
                depth: 0,
            }];

            let data = match parse_trace_file(trace_file) {
                Some(d) => d,
                None => return None,
            };
            add_spans(&mut spans, &data);

            Some(Unit {
                name,
                trace_file: trace_file.clone(),
                spans,
                absolute_start_time: data.beginning_of_time,
            })
        })
        .collect()
}
