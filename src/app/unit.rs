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
