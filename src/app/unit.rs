use rayon::prelude::*;

use crate::{
    app::span::{Span, get_spans},
    traces::file::{clean_trace_file_path, get_trace_files},
};

pub struct Unit {
    pub name: String,
    pub trace_file: std::path::PathBuf,
    pub spans: Vec<Span>,
}

pub fn get_units(build_dir: &std::path::PathBuf) -> Vec<Unit> {
    let trace_files = get_trace_files(build_dir);
    // let all_spans = get_spans(trace_files);
    let units: Vec<Unit> = trace_files
        .par_iter()
        .map(|trace_file| {
            let name = clean_trace_file_path(&trace_file, build_dir)
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            let spans = get_spans(trace_file);

            Unit {
                name,
                trace_file: trace_file.clone(),
                spans,
            }
        })
        .collect();
    units
}
