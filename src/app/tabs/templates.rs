use crossterm::event::{KeyCode, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use crate::app::ZoomDirection;
use crate::app::span::{Span, SpanType};
use crate::app::tabs::Tab;
use crate::app::view::{
    AggregateSpanView, FollowingSpanDirection, HorizontalDirection, SpanView, build_track_views,
    get_following_span_index, schedule_spans,
};
use crate::widgets::flamegraph::FlamegraphWidget;
use crate::widgets::pch_candidates::{PchCandidate, CandidatesWidget, CopyMode};
use crate::widgets::span_details::SpanDetails;
use crate::widgets::time_range::DurationRange;
use crate::widgets::track::TrackInput;

pub struct TemplatesTab {
    pub spans: Rc<[Span]>,
    pub tracks_by_duration: Vec<Vec<SpanView>>,
    pub root_track_map: HashMap<usize, usize>,
    pub zoom: f64,
    pub start_time: f64,
    pub selected_span: Option<usize>,
    pub cell_span_map: HashMap<(u16, u16), usize>,
    pub vertical_scroll: u16,
    pub viewport_height: u16,
    pub viewport_width: u16,
    pub content_height: u16,
    pub counts: Vec<usize>,
    pub search_query: Option<String>,
    /// Extern candidates: top 10 slowest concrete template instantiations.
    pub extern_candidates: Vec<PchCandidate>,
    /// Maps identifier → list of span indices for cross-referencing.
    pub extern_span_map: HashMap<String, Vec<usize>>,
    pub extern_scroll_offset: u16,
    pub extern_selected_index: Option<usize>,
    pch_rect: Rect,
    copy_confirmed_at: Option<Instant>,
}

// ---------------------------------------------------------------------------
// Identifier parsing
// ---------------------------------------------------------------------------

fn parse_template_identifier(identifier: &str) -> (&str, Vec<String>) {
    match (identifier.find('<'), identifier.rfind('>')) {
        (Some(open), Some(close)) if close > open => {
            let name = &identifier[..open];
            let args_str = &identifier[open + 1..close];
            let args: Vec<String> = args_str.split(',').map(|s| s.trim().to_string()).collect();
            (name, args)
        }
        _ => (identifier, Vec::new()),
    }
}

fn concrete_identifier(name: &str, args: &[String]) -> String {
    if args.is_empty() {
        name.to_string()
    } else {
        format!("{}<{}>", name, args.join(", "))
    }
}

fn build_wildcard_identifier(name: &str, prefix: &[String], total_args: usize) -> String {
    if total_args == 0 {
        return name.to_string();
    }
    let mut id = format!("{}<", name);
    for (i, p) in prefix.iter().enumerate() {
        if i > 0 {
            id.push_str(", ");
        }
        id.push_str(p);
    }
    for i in prefix.len()..total_args {
        if i > 0 {
            id.push_str(", ");
        }
        id.push('*');
    }
    id.push('>');
    id
}

// ---------------------------------------------------------------------------
// Aggregation data
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AggEntry {
    duration: f64,
    occurrences: usize,
    tu_count: usize,
    span_type: SpanType,
    sublabel: String,
}

// ---------------------------------------------------------------------------
// Span builder
// ---------------------------------------------------------------------------

fn push_span(
    new_spans: &mut Vec<Span>,
    counts: &mut Vec<usize>,
    identifier: String,
    label: String,
    duration: f64,
    count: usize,
    depth: usize,
    parent_index: Option<usize>,
    span_type: SpanType,
    sublabel: String,
) -> usize {
    let index = new_spans.len();
    new_spans.push(Span {
        type_: span_type,
        identifier,
        label,
        sublabel: Some(sublabel),
        start_time: 0.0,
        duration,
        parent_index,
        children_indices: Vec::new(),
        root_span_index: 0,
        depth,
    });
    counts.push(count);
    index
}

// ---------------------------------------------------------------------------
// Arg-based wildcard hierarchy builder
// ---------------------------------------------------------------------------

fn build_template_tree(
    name: &str,
    entries: &HashMap<Vec<String>, AggEntry>,
    prefix: &[String],
    visual_depth: usize,
    parent_index: Option<usize>,
    new_spans: &mut Vec<Span>,
    counts: &mut Vec<usize>,
) -> usize {
    let total_args = entries.keys().next().map(|a| a.len()).unwrap_or(0);
    let pos = prefix.len();

    let matching: Vec<(&Vec<String>, &AggEntry)> = entries
        .iter()
        .filter(|(args, _)| args.len() == total_args && args[..pos] == *prefix)
        .collect();

    if matching.len() == 1 {
        let (args, entry) = matching[0];
        let identifier = concrete_identifier(name, args);
        let label = identifier.clone();
        return push_span(
            new_spans,
            counts,
            identifier,
            label,
            entry.duration,
            entry.tu_count,
            visual_depth,
            parent_index,
            entry.span_type,
            entry.sublabel.clone(),
        );
    }

    let mut groups: HashMap<&String, Vec<(&Vec<String>, &AggEntry)>> = HashMap::new();
    for item in &matching {
        groups.entry(&item.0[pos]).or_default().push(*item);
    }

    if groups.len() == 1 {
        let val = groups.keys().next().unwrap();
        let mut new_prefix = prefix.to_vec();
        new_prefix.push((*val).clone());
        return build_template_tree(
            name,
            entries,
            &new_prefix,
            visual_depth,
            parent_index,
            new_spans,
            counts,
        );
    }

    let total_dur: f64 = matching.iter().map(|(_, e)| e.duration).sum();
    let total_tus: usize = matching.iter().map(|(_, e)| e.tu_count).sum();
    let identifier = build_wildcard_identifier(name, prefix, total_args);
    let label = identifier.clone();

    let dominant = matching
        .iter()
        .max_by(|a, b| {
            a.1.duration
                .partial_cmp(&b.1.duration)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(_, e)| (e.span_type, e.sublabel.clone()))
        .unwrap_or((SpanType::Template, "Instantiation".to_string()));

    let index = push_span(
        new_spans,
        counts,
        identifier,
        label,
        total_dur,
        total_tus,
        visual_depth,
        parent_index,
        dominant.0,
        dominant.1,
    );

    let mut sorted_keys: Vec<&String> = groups.keys().copied().collect();
    sorted_keys.sort_by(|a, b| {
        let da: f64 = groups[a].iter().map(|(_, e)| e.duration).sum();
        let db: f64 = groups[b].iter().map(|(_, e)| e.duration).sum();
        db.partial_cmp(&da).unwrap_or(std::cmp::Ordering::Equal)
    });

    for val in sorted_keys {
        let mut new_prefix = prefix.to_vec();
        new_prefix.push(val.clone());
        let child_idx = build_template_tree(
            name,
            entries,
            &new_prefix,
            visual_depth + 1,
            Some(index),
            new_spans,
            counts,
        );
        new_spans[index].children_indices.push(child_idx);
    }

    index
}

// ---------------------------------------------------------------------------
// Aggregation
// ---------------------------------------------------------------------------

/// Shift all depths in the subtree rooted at `root` down by 1.
fn decrement_depths(spans: &mut [Span], root: usize) {
    let mut stack = vec![root];
    while let Some(i) = stack.pop() {
        spans[i].depth = spans[i].depth.saturating_sub(1);
        for &c in &spans[i].children_indices {
            stack.push(c);
        }
    }
}

fn is_instantiation(span: &Span) -> bool {
    span.sublabel.as_deref() == Some("Instantiation")
}

fn is_template_or_class(st: SpanType) -> bool {
    st == SpanType::Template || st == SpanType::Class
}

/// A root instantiation is a template/class instantiation whose immediate
/// parent is NOT a template/class span (i.e. it's the top of the chain).
fn is_root_instantiation(raw_spans: &[Span], idx: usize) -> bool {
    let span = &raw_spans[idx];
    if !is_instantiation(span) || !is_template_or_class(span.type_) {
        return false;
    }
    match span.parent_index {
        Some(pi) => !is_template_or_class(raw_spans[pi].type_),
        None => true,
    }
}

fn aggregate_templates(raw_spans: &[Span]) -> (Vec<Span>, Vec<usize>) {
    // ── Collect root instantiations, grouped by (base_name, arg_count) ────
    let mut by_key: HashMap<(String, usize), HashMap<Vec<String>, AggEntry>> = HashMap::new();

    for (_i, span) in raw_spans.iter().enumerate() {
        if !is_root_instantiation(raw_spans, _i) {
            continue;
        }
        let (name, args) = parse_template_identifier(&span.identifier);
        let key = (name.to_string(), args.len());
        let arg_map = by_key.entry(key).or_default();
        let entry = arg_map.entry(args.clone()).or_insert_with(|| AggEntry {
            duration: 0.0,
            occurrences: 0,
            tu_count: 0,
            span_type: span.type_,
            sublabel: span.sublabel.clone().unwrap_or_default(),
        });
        entry.duration += span.duration;
        entry.occurrences += 1;
    }

    // ── Count distinct TUs per (name, args) ──────────────────────────────
    let mut tu_sets: HashMap<(String, Vec<String>), HashSet<usize>> = HashMap::new();
    for (_i, span) in raw_spans.iter().enumerate() {
        if !is_root_instantiation(raw_spans, _i) {
            continue;
        }
        let (name, args) = parse_template_identifier(&span.identifier);
        let set = tu_sets.entry((name.to_string(), args.clone())).or_default();
        set.insert(span.root_span_index);
    }
    for ((name, args), set) in &tu_sets {
        let key = (name.clone(), args.len());
        if let Some(arg_map) = by_key.get_mut(&key) {
            if let Some(entry) = arg_map.get_mut(args) {
                entry.tu_count = set.len();
            }
        }
    }

    // ── Group (name, arg_count) entries by bare name ─────────────────────
    let mut by_base: HashMap<String, HashMap<usize, HashMap<Vec<String>, AggEntry>>> = HashMap::new();
    for ((name, arg_count), arg_map) in by_key {
        by_base.entry(name).or_default().insert(arg_count, arg_map);
    }

    let mut new_spans: Vec<Span> = Vec::new();
    let mut counts: Vec<usize> = Vec::new();

    // Sort base names by total duration desc.
    let mut sorted_bases: Vec<(String, f64)> = by_base
        .iter()
        .map(|(name, ac_map)| {
            let dur: f64 = ac_map.values().flat_map(|m| m.values()).map(|e| e.duration).sum();
            (name.clone(), dur)
        })
        .collect();
    sorted_bases.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    for (base_name, _) in &sorted_bases {
        let mut ac_map = by_base.remove(base_name).unwrap();

        if ac_map.len() == 1 {
            // Single arg count → no top-level wrapper needed.
            let (_, arg_map) = ac_map.into_iter().next().unwrap();
            build_template_tree(base_name, &arg_map, &[], 0, None, &mut new_spans, &mut counts);
        } else {
            // Multiple arg counts → create bare-name top-level parent.
            let total_dur: f64 = ac_map.values().flat_map(|m| m.values()).map(|e| e.duration).sum();
            let total_tus: usize = ac_map.values().flat_map(|m| m.values()).map(|e| e.tu_count).sum();
            let dominant = ac_map.values().flat_map(|m| m.values())
                .max_by(|a, b| a.duration.partial_cmp(&b.duration).unwrap_or(std::cmp::Ordering::Equal))
                .map(|e| (e.span_type, e.sublabel.clone()))
                .unwrap_or((SpanType::Template, "Instantiation".to_string()));
            let label = base_name.clone();

            let top_idx = push_span(
                &mut new_spans, &mut counts,
                base_name.clone(), label,
                total_dur, total_tus,
                0, None,
                dominant.0, dominant.1,
            );

            // Sort arg counts by total duration desc.
            let mut sorted_ac: Vec<(usize, f64)> = ac_map.iter()
                .map(|(ac, m)| (*ac, m.values().map(|e| e.duration).sum()))
                .collect();
            sorted_ac.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

            for (arg_count, _) in sorted_ac {
                let arg_map = ac_map.remove(&arg_count).unwrap();
                let root_idx = build_template_tree(
                    base_name, &arg_map, &[],
                    1, Some(top_idx),
                    &mut new_spans, &mut counts,
                );
                new_spans[top_idx].children_indices.push(root_idx);
            }

            // If the bare-name parent has exactly ONE child and that child
            // is a wildcard (has its own children), flatten: promote the
            // wildcard's children to the bare-name parent, skip the wildcard.
            if new_spans[top_idx].children_indices.len() == 1 {
                let child_idx = new_spans[top_idx].children_indices[0];
                let grandchildren: Vec<usize> = new_spans[child_idx].children_indices.clone();
                if !grandchildren.is_empty() {
                    // Reparent grandchildren to top_idx, adjust their depths.
                    for &gc in &grandchildren {
                        new_spans[gc].parent_index = Some(top_idx);
                        // Shift all depths in the gc subtree down by 1.
                        decrement_depths(&mut new_spans, gc);
                    }
                    new_spans[top_idx].children_indices = grandchildren;
                    // The orphaned wildcard span remains in new_spans but
                    // is no longer referenced.  schedule_spans ignores it
                    // because it has parent_index set and is not a root.
                }
            }
        }
    }

    // ── Assign start_time positions ──────────────────────────────────────
    let mut root_offset = 0.0f64;
    for i in 0..new_spans.len() {
        if new_spans[i].parent_index.is_none() {
            new_spans[i].start_time = root_offset;
            root_offset += new_spans[i].duration;
        }
    }
    for i in 0..new_spans.len() {
        let parent_start = new_spans[i].start_time;
        let mut cursor = parent_start;
        let children: Vec<usize> = new_spans[i].children_indices.clone();
        for &ci in &children {
            new_spans[ci].start_time = cursor;
            cursor += new_spans[ci].duration;
        }
    }
    for i in 0..new_spans.len() {
        let mut curr = i;
        while let Some(pi) = new_spans[curr].parent_index {
            curr = pi;
        }
        new_spans[i].root_span_index = curr;
    }

    (new_spans, counts)
}

// ---------------------------------------------------------------------------
// TemplatesTab impl
// ---------------------------------------------------------------------------

impl TemplatesTab {
    pub fn new(raw_spans: Rc<[Span]>) -> Self {
        let (aggregated, counts) = aggregate_templates(&raw_spans);
        let spans: Rc<[Span]> = Rc::from(aggregated);

        let mut track_roots = schedule_spans(&spans);
        track_roots.sort_by(|a, b| {
            let dur =
                |roots: &Vec<usize>| -> f64 { roots.iter().map(|&i| spans[i].duration).sum() };
            dur(b)
                .partial_cmp(&dur(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let (_tracks_start_time, tracks_by_duration) = build_track_views(&spans, &track_roots);

        let mut root_track_map = HashMap::new();
        for (ti, roots) in track_roots.iter().enumerate() {
            for &root in roots {
                root_track_map.insert(root, ti);
            }
        }

        // --- Compute extern candidates: leaf template spans (concrete instantiations) ---
        let (extern_candidates, extern_span_map) = compute_extern_candidates(&spans, &counts);

        Self {
            spans,
            tracks_by_duration,
            root_track_map,
            zoom: 1.0,
            start_time: 0.0,
            selected_span: None,
            cell_span_map: HashMap::new(),
            vertical_scroll: 0,
            viewport_height: 0,
            viewport_width: 0,
            content_height: 0,
            counts,
            search_query: None,
            extern_candidates,
            extern_span_map,
            extern_scroll_offset: 0,
            extern_selected_index: None,
            pch_rect: Rect::default(),
            copy_confirmed_at: None,
        }
    }

    fn current_tracks(&self) -> &[Vec<SpanView>] {
        &self.tracks_by_duration
    }

    fn track_index_for_span(&self, span_index: usize) -> Option<usize> {
        let root = self.spans.get(span_index)?.root_span_index;
        self.root_track_map.get(&root).copied()
    }

    fn total_duration(&self) -> f64 {
        self.spans
            .iter()
            .filter(|s| s.parent_index.is_none() && s.duration.is_finite())
            .map(|s| s.start_time + s.duration)
            .fold(0.0f64, f64::max)
    }

    fn visible_duration(&self) -> f64 {
        self.total_duration() / self.zoom
    }

    fn move_selection(&mut self, direction: FollowingSpanDirection) {
        let si = match self.selected_span {
            Some(idx) => idx,
            None => {
                let first = self
                    .current_tracks()
                    .iter()
                    .flat_map(|t| t.iter())
                    .find(|v| v.was_displayed)
                    .map(|v| v.span_index)
                    .unwrap_or(0);
                self.selected_span = Some(first);
                return;
            }
        };

        match direction {
            FollowingSpanDirection::Next | FollowingSpanDirection::Previous => {
                let horiz = match direction {
                    FollowingSpanDirection::Next => HorizontalDirection::Next,
                    _ => HorizontalDirection::Previous,
                };

                if self.spans[si].parent_index.is_none() {
                    let Some(ti) = self.track_index_for_span(si) else {
                        return;
                    };
                    let new_si = {
                        let track_views = &self.tracks_by_duration[ti];
                        let mut seen = std::collections::HashSet::new();
                        let roots: Vec<usize> = track_views
                            .iter()
                            .filter(|v| {
                                self.spans[v.span_index].parent_index.is_none() && v.was_displayed
                            })
                            .map(|v| v.span_index)
                            .filter(|&x| seen.insert(x))
                            .collect();
                        let pos = roots.iter().position(|&idx| idx == si);
                        pos.and_then(|pos| {
                            let shift: isize = match horiz {
                                HorizontalDirection::Next => 1,
                                HorizontalDirection::Previous => -1,
                            };
                            roots.get((pos as isize + shift) as usize).copied()
                        })
                    };
                    if let Some(new_si) = new_si {
                        self.selected_span = Some(new_si);
                    }
                } else {
                    let new_si = {
                        let ti = match self.track_index_for_span(si) {
                            Some(ti) => ti,
                            None => return,
                        };
                        let views = &self.tracks_by_duration[ti];
                        get_following_span_index(&self.spans, views, si, horiz)
                    };
                    if let Some(new_si) = new_si {
                        self.selected_span = Some(new_si);
                    }
                }
            }
            FollowingSpanDirection::Parent => {
                if let Some(pi) = self.spans[si].parent_index {
                    self.selected_span = Some(pi);
                }
            }
            FollowingSpanDirection::Child => {
                let new_si = {
                    let ti = match self.track_index_for_span(si) {
                        Some(ti) => ti,
                        None => return,
                    };
                    let views = &self.tracks_by_duration[ti];
                    views
                        .iter()
                        .find(|v| {
                            self.spans[v.span_index].parent_index == Some(si) && v.was_displayed
                        })
                        .map(|v| v.span_index)
                };
                if let Some(new_si) = new_si {
                    self.selected_span = Some(new_si);
                }
            }
        }
    }

    fn compute_track_positions(&self) -> Vec<(u16, u16)> {
        use crate::widgets::track::track_content_height;
        if self.viewport_width == 0 {
            return Vec::new();
        }
        let visible_duration = self.visible_duration();
        let label_height: u16 = 1;
        let mut positions = Vec::new();
        let mut virtual_y: u16 = 0;
        for views in self.current_tracks().iter() {
            let track_height =
                track_content_height(views, &self.spans, visible_duration, self.viewport_width)
                    + label_height;
            let track_start = virtual_y;
            let track_end = virtual_y.saturating_add(track_height);
            positions.push((track_start, track_end));
            virtual_y = track_end;
        }
        positions
    }

    fn center_track(&mut self, track_idx: usize) {
        let n_tracks = self.current_tracks().len();
        if track_idx >= n_tracks || self.viewport_height == 0 {
            return;
        }
        let positions = self.compute_track_positions();
        if track_idx >= positions.len() {
            return;
        }
        let (track_start, track_end) = positions[track_idx];
        let total_height = positions.last().map(|(_, end)| *end).unwrap_or(0);
        if total_height <= self.viewport_height {
            self.vertical_scroll = 0;
            return;
        }
        let track_center = track_start.saturating_add(track_end.saturating_sub(track_start) / 2);
        let max_scroll = total_height.saturating_sub(self.viewport_height);
        self.vertical_scroll = track_center
            .saturating_sub(self.viewport_height / 2)
            .min(max_scroll);
    }

    fn center_selected_track(&mut self) {
        let Some(si) = self.selected_span else { return };
        let Some(ti) = self.track_index_for_span(si) else {
            return;
        };
        self.center_track(ti);
    }

    fn switch_track(&mut self, dir: HorizontalDirection) {
        let n = self.current_tracks().len();
        if n == 0 {
            return;
        }
        let current_ti = self
            .selected_span
            .and_then(|si| self.track_index_for_span(si))
            .unwrap_or(0);
        let new_ti = match dir {
            HorizontalDirection::Next => (current_ti + 1) % n,
            HorizontalDirection::Previous => (current_ti + n - 1) % n,
        };
        let first_visible = {
            let views = &self.tracks_by_duration[new_ti];
            let mut seen = std::collections::HashSet::new();
            views
                .iter()
                .filter(|v| self.spans[v.span_index].parent_index.is_none())
                .map(|v| v.span_index)
                .filter(|&x| seen.insert(x))
                .find(|&root| {
                    views
                        .iter()
                        .any(|v| v.span_index == root && v.was_displayed)
                })
                .or_else(|| {
                    let mut seen2 = std::collections::HashSet::new();
                    views
                        .iter()
                        .filter(|v| self.spans[v.span_index].parent_index.is_none())
                        .map(|v| v.span_index)
                        .find(|&x| seen2.insert(x))
                })
        };
        if let Some(new_si) = first_visible {
            self.selected_span = Some(new_si);
            self.center_track(new_ti);
        }
    }

    fn zoom_around_center(&mut self, factor: f64) {
        let center = self.start_time + self.visible_duration() / 2.0;
        self.zoom = (self.zoom * factor).max(1.0);
        let new_half = self.visible_duration() / 2.0;
        self.start_time = (center - new_half).max(0.0);
        let max_start = (self.total_duration() - self.visible_duration()).max(0.0);
        self.start_time = self.start_time.min(max_start);
        self.center_selected_track();
    }

    fn zoom_to_selected(&mut self, factor: Option<f64>) {
        let si = match self.selected_span {
            Some(idx) => idx,
            None => return,
        };
        let span_duration = match self.spans.get(si) {
            Some(s) => s.duration,
            None => return,
        };
        let effective_start = {
            let ti = match self.track_index_for_span(si) {
                Some(ti) => ti,
                None => return,
            };
            let views = &self.tracks_by_duration[ti];
            match views.iter().find(|e| e.span_index == si) {
                Some(e) => e.effective_start,
                None => return,
            }
        };
        let span_center = effective_start + span_duration / 2.0;
        let new_visible = span_duration / 0.75;
        let total = self.total_duration();
        self.zoom = factor.unwrap_or(total / new_visible).max(1.0);
        let actual_visible = total / self.zoom;
        self.start_time = (span_center - actual_visible / 2.0)
            .max(0.0)
            .min((total - actual_visible).max(0.0));
        self.center_selected_track();
    }

    fn zoom(&mut self, factor: f64, direction: ZoomDirection) {
        let current_zoom = self.zoom;
        let mut zoom_fn = |fc| match self.selected_span {
            Some(_) => self.zoom_to_selected(Some(current_zoom * fc)),
            None => self.zoom_around_center(fc),
        };
        match direction {
            ZoomDirection::In => zoom_fn(factor),
            ZoomDirection::Out => {
                zoom_fn(1.0 / factor);
                if self.zoom < 1.0 {
                    self.zoom = 1.0;
                    self.start_time = 0.0;
                }
            }
        }
    }
}

impl Tab for TemplatesTab {
    fn get_label(&self) -> &str {
        "Templates"
    }

    fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) -> bool {
        let ctrl = key
            .modifiers
            .contains(crossterm::event::KeyModifiers::CONTROL);
        let shift = key
            .modifiers
            .contains(crossterm::event::KeyModifiers::SHIFT);
        let pan_factor = if shift { 0.05 } else { 0.25 };

        match key.code {
            KeyCode::Up if ctrl => self.zoom(1.1, ZoomDirection::In),
            KeyCode::Down if ctrl => self.zoom(1.1, ZoomDirection::Out),
            KeyCode::PageUp => self.zoom(2.0, ZoomDirection::In),
            KeyCode::PageDown => self.zoom(2.0, ZoomDirection::Out),
            KeyCode::Left if ctrl => {
                let step = self.visible_duration() * pan_factor;
                self.start_time = (self.start_time - step).max(0.0);
            }
            KeyCode::Right if ctrl => {
                let step = self.visible_duration() * pan_factor;
                let max_start = (self.total_duration() - self.visible_duration()).max(0.0);
                self.start_time = (self.start_time + step).min(max_start);
            }
            KeyCode::Left => self.move_selection(FollowingSpanDirection::Previous),
            KeyCode::Right => self.move_selection(FollowingSpanDirection::Next),
            KeyCode::Up => self.move_selection(FollowingSpanDirection::Parent),
            KeyCode::Down => self.move_selection(FollowingSpanDirection::Child),
            KeyCode::Char(' ') if ctrl => {
                self.zoom = 1.0;
                self.start_time = 0.0;
                self.center_selected_track();
            }
            KeyCode::Char(' ') => self.zoom_to_selected(None),
            KeyCode::Esc => self.selected_span = None,
            KeyCode::Tab => self.switch_track(HorizontalDirection::Next),
            KeyCode::BackTab => self.switch_track(HorizontalDirection::Previous),
            KeyCode::Char('y') if ctrl => self.copy_externs_to_clipboard(),
            _ => {}
        }
        false
    }

    fn handle_mouse_event(&mut self, mouse: crossterm::event::MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                let coord = (mouse.column, mouse.row);

                // Check extern panel copy button
                let pch_rect = self.pch_rect;
                if pch_rect.width > 0 {
                    let copy_confirmed = self
                        .copy_confirmed_at
                        .map_or(false, |t| Instant::now().duration_since(t).as_secs() < 3);
                    let widget = CandidatesWidget {
                        title: "Extern Candidates",
                        candidates: &self.extern_candidates,
                        scroll_offset: self.extern_scroll_offset,
                        selected_index: self.extern_selected_index,
                        copy_confirmed,
                        copy_mode: CopyMode::ExternTemplate,
                    };
                    if widget.hit_copy_button(pch_rect, coord.0, coord.1) {
                        self.copy_externs_to_clipboard();
                        return;
                    }
                }

                // Check extern candidate list clicks
                if pch_rect.width > 0 {
                    let block = ratatui::widgets::Block::default()
                        .borders(ratatui::widgets::Borders::ALL);
                    let inner = block.inner(pch_rect);
                    let list_top = inner.y + CandidatesWidget::HEADER_HEIGHT;
                    if coord.0 >= inner.x
                        && coord.0 < inner.x + inner.width
                        && coord.1 >= list_top
                        && coord.1 < inner.bottom()
                    {
                        let row_in_list = coord.1 - list_top;
                        let idx = self.extern_scroll_offset as usize
                            + (row_in_list / CandidatesWidget::CANDIDATE_ROWS) as usize;
                        if idx < self.extern_candidates.len() {
                            self.extern_selected_index = Some(idx);
                            let ident = self.extern_candidates[idx].identifier.clone();
                            if let Some(indices) = self.extern_span_map.get(&ident) {
                                if let Some(&si) = indices.first() {
                                    self.selected_span = Some(si);
                                    self.zoom_to_selected(None);
                                    self.center_selected_track();
                                }
                            }
                        }
                        return;
                    }
                }

                // Fallthrough: flamegraph click
                if let Some(&si) = self.cell_span_map.get(&coord) {
                    self.selected_span = Some(si);
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft => {
                let pch_rect = self.pch_rect;
                if pch_rect.width > 0 && mouse.column >= pch_rect.x {
                    let visible_count = pch_rect
                        .height
                        .saturating_sub(2)
                        .saturating_sub(CandidatesWidget::HEADER_HEIGHT)
                        / CandidatesWidget::CANDIDATE_ROWS;
                    let max_scroll = self
                        .extern_candidates
                        .len()
                        .saturating_sub(visible_count as usize) as u16;
                    self.extern_scroll_offset = self.extern_scroll_offset.saturating_sub(1).min(max_scroll);
                    return;
                }
                let max_scroll = self.content_height.saturating_sub(self.viewport_height);
                self.vertical_scroll = self.vertical_scroll.saturating_sub(3).min(max_scroll);
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollRight => {
                let pch_rect = self.pch_rect;
                if pch_rect.width > 0 && mouse.column >= pch_rect.x {
                    let visible_count = pch_rect
                        .height
                        .saturating_sub(2)
                        .saturating_sub(CandidatesWidget::HEADER_HEIGHT)
                        / CandidatesWidget::CANDIDATE_ROWS;
                    let max_scroll = self
                        .extern_candidates
                        .len()
                        .saturating_sub(visible_count as usize) as u16;
                    self.extern_scroll_offset = self.extern_scroll_offset.saturating_add(1).min(max_scroll);
                    return;
                }
                let max_scroll = self.content_height.saturating_sub(self.viewport_height);
                self.vertical_scroll = self.vertical_scroll.saturating_add(3).min(max_scroll);
            }
            _ => {}
        }
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let total_duration = self.total_duration();
        let visible_duration = total_duration / self.zoom;

        // --- Layout: left (templates), right (extern candidates full-height) ---
        let pch_width: u16 = if area.width >= 80 { 40 } else { 0 };
        let left_width = area.width.saturating_sub(pch_width);
        let left_area = Rect::new(area.x, area.y, left_width, area.height);

        let left_block = ratatui::widgets::Block::default()
            .borders(ratatui::widgets::Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let left_inner = left_block.inner(left_area);

        let scrollbar_height = 2;
        let details_height: u16 = if let Some(si) = self.selected_span {
            self.spans
                .get(si)
                .map(|span| {
                    let parent_duration = span
                        .parent_index
                        .and_then(|pi| self.spans.get(pi))
                        .map(|p| p.duration);
                    let view = AggregateSpanView {
                        view: SpanView {
                            span_index: si,
                            ..Default::default()
                        },
                        count: self.counts[si],
                    };
                    SpanDetails {
                        spans: &self.spans,
                        view: &view,
                        parent_duration,
                        total_duration,
                    }
                    .required_height(left_inner.width)
                })
                .unwrap_or(0)
        } else {
            0
        };
        let graph_height = left_inner.height.saturating_sub(scrollbar_height + details_height);
        let vscrollbar_width: u16 = if left_inner.width > 1 { 1 } else { 0 };
        let graph_width = left_inner.width.saturating_sub(vscrollbar_width);

        let scrollbar_area = Rect::new(left_inner.x, left_inner.y, left_inner.width, scrollbar_height);
        let graph_area = Rect::new(left_inner.x, left_inner.y + scrollbar_height, graph_width, graph_height);
        let vscrollbar_area = Rect::new(
            left_inner.x + graph_width,
            left_inner.y + scrollbar_height,
            vscrollbar_width,
            graph_height,
        );
        let details_area = Rect::new(
            left_inner.x,
            left_inner.y + scrollbar_height + graph_height,
            left_inner.width,
            details_height,
        );

        let pch_area = if pch_width > 0 {
            Rect::new(area.x + left_width, area.y, pch_width, area.height)
        } else {
            Rect::default()
        };
        self.pch_rect = pch_area;

        // Render left border
        left_block.render(left_area, buf);

        // Timeline
        DurationRange {
            total_duration,
            start: self.start_time,
            visible_duration,
        }
        .render(scrollbar_area, buf);

        // Flamegraph
        self.cell_span_map.clear();
        self.viewport_height = graph_height;
        self.viewport_width = graph_area.width;
        let selected_span = self.selected_span;
        let start_time = self.start_time;

        use crate::widgets::track::track_content_height;
        let label_height: u16 = 1;

        let heights: Vec<u16> = self
            .tracks_by_duration
            .iter()
            .map(|views| {
                track_content_height(views, &self.spans, visible_duration, graph_area.width)
                    + label_height
            })
            .collect();

        let track_inputs: Vec<TrackInput> = self
            .tracks_by_duration
            .iter_mut()
            .zip(heights.iter().copied())
            .map(|(views, intrinsic_height)| TrackInput {
                views: views.as_mut_slice(),
                label: Some("Templates".to_string()),
                intrinsic_height,
            })
            .collect();

        self.content_height = FlamegraphWidget::total_height(&track_inputs);
        let max_scroll = self.content_height.saturating_sub(graph_area.height);
        self.vertical_scroll = self.vertical_scroll.min(max_scroll);

        FlamegraphWidget {
            tracks: track_inputs,
            spans: &self.spans,
            total_duration: visible_duration,
            start_time,
            scroll_offset: self.vertical_scroll,
            cell_map: &mut self.cell_span_map,
            selected_span,
            search_query: self.search_query.as_deref(),
        }
        .render(graph_area, buf);

        // VScrollbar
        if vscrollbar_area.width > 0 && vscrollbar_area.height > 0 {
            let muted_style = Style::default().fg(Color::DarkGray);
            let active_style = Style::default().fg(Color::White);
            for y in vscrollbar_area.y..vscrollbar_area.bottom() {
                buf.set_string(vscrollbar_area.x, y, " ", muted_style);
            }
            let thumb_height = if self.content_height <= vscrollbar_area.height {
                vscrollbar_area.height
            } else {
                ((vscrollbar_area.height as f64 * vscrollbar_area.height as f64)
                    / self.content_height as f64)
                    .round() as u16
            }
            .clamp(1, vscrollbar_area.height);
            let thumb_start = if max_scroll == 0 {
                0
            } else {
                ((self.vertical_scroll as f64 / max_scroll as f64)
                    * (vscrollbar_area.height.saturating_sub(thumb_height) as f64))
                    .round() as u16
            };
            for y in 0..thumb_height {
                buf.set_string(
                    vscrollbar_area.x,
                    vscrollbar_area.y + thumb_start + y,
                    "\u{2503}",
                    active_style,
                );
            }
        }

        // Extern candidates panel
        if pch_width > 0 && pch_area.height > 0 {
            let now = Instant::now();
            let copy_confirmed = self
                .copy_confirmed_at
                .map_or(false, |t| now.duration_since(t).as_secs() < 3);

            CandidatesWidget {
                title: "Extern Candidates",
                candidates: &self.extern_candidates,
                scroll_offset: self.extern_scroll_offset,
                selected_index: self.extern_selected_index,
                copy_confirmed,
                copy_mode: CopyMode::ExternTemplate,
            }
            .render(pch_area, buf);
        }

        // Details
        if let Some(si) = self.selected_span {
            if let Some(span) = self.spans.get(si) {
                let parent_duration = span.parent_index.and_then(|pi| self.spans.get(pi)).map(|p| p.duration);
                let view = AggregateSpanView {
                    view: SpanView { span_index: si, ..Default::default() },
                    count: self.counts[si],
                };
                SpanDetails {
                    spans: &self.spans,
                    view: &view,
                    parent_duration,
                    total_duration,
                }
                .render(details_area, buf);
            }
        }
    }

    fn get_help(&self) -> Vec<(&str, &str)> {
        vec![
            ("Ctrl + Up", "Zoom in"),
            ("Ctrl + Down", "Zoom out"),
            ("PageUp", "Zoom in (fast)"),
            ("PageDown", "Zoom out (fast)"),
            ("Ctrl + Left", "Pan left"),
            ("Ctrl + Right", "Pan right"),
            ("Left", "Previous sibling"),
            ("Right", "Next sibling"),
            ("Up", "Parent span"),
            ("Down", "Child span"),
            ("Ctrl + Space", "Reset zoom"),
            ("Space", "Zoom to selection"),
            ("Esc", "Clear selection"),
            ("Tab", "Next track"),
            ("Shift + Tab", "Previous track"),
            ("Ctrl + Y", "Copy extern #includes"),
        ]
    }

    fn set_search_query(&mut self, query: String) {
        if query.is_empty() {
            self.search_query = None;
        } else {
            self.search_query = Some(query);
        }
    }

    fn select_next_match(&mut self) {
        self.navigate_match(1);
    }

    fn select_previous_match(&mut self) {
        self.navigate_match(-1);
    }

    fn match_count(&self) -> usize {
        self.find_matches().len()
    }

    fn current_match_index(&self) -> Option<usize> {
        let matches = self.find_matches();
        self.selected_span
            .and_then(|sel| matches.iter().position(|&m| m == sel))
    }
}

impl TemplatesTab {
    fn copy_externs_to_clipboard(&mut self) {
        let text = CandidatesWidget {
            title: "",
            candidates: &self.extern_candidates,
            scroll_offset: 0,
            selected_index: None,
            copy_confirmed: false,
            copy_mode: CopyMode::ExternTemplate,
        }
        .build_copy_text();
        let _ = write_to_clipboard(&text);
        self.copy_confirmed_at = Some(Instant::now());
    }

    fn find_matches(&self) -> Vec<usize> {
        let query = match &self.search_query {
            Some(q) if !q.is_empty() => q.to_lowercase(),
            _ => return Vec::new(),
        };
        self.spans
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                s.identifier.to_lowercase().contains(&query)
                    || s.label.to_lowercase().contains(&query)
                    || s.sublabel
                        .as_ref()
                        .map(|sl| sl.to_lowercase().contains(&query))
                        .unwrap_or(false)
            })
            .map(|(i, _)| i)
            .collect()
    }

    fn navigate_match(&mut self, direction: isize) {
        let matches = self.find_matches();

        if matches.is_empty() {
            return;
        }

        let current_pos = self
            .selected_span
            .and_then(|sel| matches.iter().position(|&m| m == sel));

        let new_pos = match current_pos {
            Some(p) => {
                let n = matches.len() as isize;
                (p as isize + direction).rem_euclid(n) as usize
            }
            None => {
                if direction > 0 { 0 } else { matches.len() - 1 }
            }
        };

        self.selected_span = Some(matches[new_pos]);
        self.zoom_to_selected(None);
        self.center_selected_track();
    }
}

/// Compute extern candidates: top 10 leaf template spans (concrete instantiations)
/// appearing in 2+ TUs, sorted by duration descending.
fn compute_extern_candidates(
    spans: &[Span],
    counts: &[usize],
) -> (Vec<PchCandidate>, HashMap<String, Vec<usize>>) {
    let mut map: HashMap<String, (f64, usize)> = HashMap::new();
    let mut span_map: HashMap<String, Vec<usize>> = HashMap::new();

    // Only consider leaf spans (concrete instantiations, no children).
    // Also skip lambda types — each TU gets its own lambda type, so extern is useless.
    for (i, span) in spans.iter().enumerate() {
        if !span.children_indices.is_empty() {
            continue;
        }
        if span.identifier.to_lowercase().contains("(lambda") {
            continue;
        }
        let entry = map.entry(span.identifier.clone()).or_insert((0.0, 0));
        entry.0 += span.duration;
        entry.1 += counts[i];
        span_map.entry(span.identifier.clone()).or_default().push(i);
    }

    let mut candidates: Vec<PchCandidate> = map
        .into_iter()
        .filter(|(_, (_, total_count))| *total_count > 1)
        .map(|(ident, (total_dur, total_count))| {
            let label = spans
                .iter()
                .find(|s| s.identifier == ident)
                .map(|s| s.label.clone())
                .unwrap_or_else(|| ident.clone());
            PchCandidate::new(ident, label, total_dur, total_count)
        })
        .collect();

    candidates.sort_by(|a, b| {
        b.total_duration
            .partial_cmp(&a.total_duration)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(10);

    let top_idents: HashSet<String> = candidates.iter().map(|c| c.identifier.clone()).collect();
    span_map.retain(|k, _| top_idents.contains(k));

    (candidates, span_map)
}

fn write_to_clipboard(text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let encoded = base64_encode(text);
    let mut stdout = std::io::stdout().lock();
    write!(stdout, "\x1b]52;c;{}\x07", encoded)?;
    stdout.flush()?;
    Ok(())
}

fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((n >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((n >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(n & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}
