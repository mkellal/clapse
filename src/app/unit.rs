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

/// Shorten a raw span identifier for display only when it exceeds MAX_LABEL_LEN.
/// - File paths: strip build dir prefix for local files; for system paths
///   detect std/boost and prefix with `[std]` / `[boost]` + filename.
/// - C++ names: collapse template argument lists with `<…>` only if too long.
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
    // Project-local: strip build dir prefix.
    if let Ok(rel) = p.strip_prefix(build_dir) {
        let s = rel.to_string_lossy().into_owned();
        if s.chars().count() <= MAX_LABEL_LEN {
            return s;
        }
        // Still too long: keep the last 4 components.
        return last_n_components(rel, 4);
    }
    // System / third-party path: keep last 4 components, prefix with lib tag.
    let short = last_n_components(p, 4);
    short
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

/// Collapse every `<…>` template argument list (including nested ones) to `<…>`.
fn collapse_template_args(s: &str) -> String {
    let mut result = String::new();
    let mut depth = 0usize;
    for c in s.chars() {
        match c {
            '<' => {
                if depth == 0 {
                    result.push_str("<…>");
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

pub struct Unit {
    pub name: String,
    pub trace_file: std::path::PathBuf,
    pub spans: Vec<Span>,
    pub absolute_start_time: f64,
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
        span.contained_by_index
            .and_then(|parent_index| self.spans.get(parent_index))
    }

    pub fn get_child_spans(&self, span: &Span, only_displayed: bool) -> Vec<&Span> {
        span.contains_indices
            .iter()
            .filter_map(|&child_index| self.spans.get(child_index))
            .filter(|s| !only_displayed || s.was_displayed)
            .collect()
    }

    pub fn get_following_span_index(
        &self,
        span_index: usize,
        direction: HorizontalDirection,
        only_displayed: bool,
    ) -> Option<usize> {
        // get sibling spans (those with the same parent) and find the one immediately after `span`.
        let span = self.spans.get(span_index)?;
        let parent_index = span.contained_by_index?;
        let parent = self.spans.get(parent_index)?;
        let siblings = self.get_child_spans(parent, only_displayed);
        let pos = siblings
            .iter()
            .position(|&s| s.index_in_unit == span_index)?;
        let shift = match direction {
            HorizontalDirection::Next => 1,
            HorizontalDirection::Previous => -1,
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
                    self.get_following_span_index(parent_index, direction, only_displayed)
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
            // Seed the vec with the root Unit span; add_spans will populate the
            // rest and finalise the root's start_time / duration.
            let mut spans = vec![Span {
                type_: SpanType::Unit,
                identifier: trace_file_str.clone(),
                label: name.clone(),
                sublabel: Some("Translation Unit".to_string()),
                start_time: 0.0,
                duration: f64::INFINITY,
                contained_by_index: None,
                contains_indices: Vec::new(),
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

            // Replace the default label (raw identifier clone) with the cleaned display label.
            for span in spans[1..].iter_mut() {
                span.label = clean_identifier(&span.identifier, build_dir);
            }

            Some(Unit {
                name,
                trace_file: trace_file.clone(),
                spans,
                absolute_start_time: data.beginning_of_time,
            })
        })
        .collect()
}
