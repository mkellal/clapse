use crossterm::event::{KeyCode, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::widgets::Widget;
use std::collections::HashMap;
use std::rc::Rc;

use crate::app::ZoomDirection;
use crate::app::span::{Span, SpanType};
use crate::app::tabs::Tab;
use crate::app::view::{
    AggregateSpanView, FollowingSpanDirection, HorizontalDirection, SpanView, build_track_views,
    get_following_span_index, schedule_spans,
};
use crate::widgets::flamegraph::FlamegraphWidget;
use crate::widgets::span_details::SpanDetails;
use crate::widgets::time_range::DurationRange;
use crate::widgets::track::TrackInput;

pub struct TemplatesTab {
    pub spans: Rc<[Span]>,
    pub tracks_start_time: Vec<Vec<SpanView>>,
    pub tracks_by_duration: Vec<Vec<SpanView>>,
    /// root_span_index → track index, for O(1) track lookup.
    pub root_track_map: HashMap<usize, usize>,
    pub zoom: f64,
    pub start_time: f64,
    pub selected_span: Option<usize>,
    /// Maps terminal cell (col, row) → global span index.
    pub cell_span_map: HashMap<(u16, u16), usize>,
    pub vertical_scroll: u16,
    pub viewport_height: u16,
    pub viewport_width: u16,
    pub content_height: u16,
    pub counts: Vec<usize>,
}

// ---------------------------------------------------------------------------
// Template identifier parsing
// ---------------------------------------------------------------------------

/// Parse a template identifier like `"temp1<char>"` into `("temp1", ["char"])`
/// or `"temp3<char, int>"` into `("temp3", ["char", "int"])`.
/// For identifiers without angle brackets (e.g. `"_M_cache"`), returns the
/// whole string as the name with an empty args vector.
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

/// Build the identifier string for a concrete template span.
fn concrete_identifier(name: &str, args: &[String]) -> String {
    if args.is_empty() {
        name.to_string()
    } else {
        format!("{}<{}>", name, args.join(", "))
    }
}
/// Build a wildcard identifier from a prefix of known args.
/// e.g. `prefix=["char"]`, `total_args=2` → `"temp3<char, *>"`
/// If total_args == 0, returns just the name.
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

/// Aggregated data for a group of template/class spans with the same args.
#[derive(Clone)]
struct AggEntry {
    duration: f64,
    count: usize,
    span_type: SpanType,
    sublabel: String,
}

impl AggEntry {
    fn merge(&mut self, other: &AggEntry) {
        self.duration += other.duration;
        self.count += other.count;
        // Keep the sublabel from the entry with the larger individual duration.
        if other.duration > self.duration - other.duration {
            self.sublabel = other.sublabel.clone();
        }
    }
}

/// Helper: push a new span and return its index.
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

/// Recursively build a hierarchical aggregation for one template name.
///
/// `entries`: map from full arg tuple → AggEntry
/// `prefix`: arg values fixed so far (used for matching, NOT visual depth)
/// `visual_depth`: nesting level of the next created node
/// Returns the span index of the node created (or a child if compression occurred).
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
    let pos = prefix.len(); // which arg position we are grouping on

    // Collect entries matching the current prefix.
    let matching: Vec<(&Vec<String>, &AggEntry)> = entries
        .iter()
        .filter(|(args, _)| args.len() == total_args && args[..pos] == *prefix)
        .collect();

    // Single concrete variant → leaf span.
    if matching.len() == 1 {
        let (args, entry) = matching[0];
        let identifier = concrete_identifier(name, args);
        return push_span(
            new_spans, counts,
            identifier.clone(), identifier,
            entry.duration, entry.count,
            visual_depth, parent_index,
            entry.span_type, entry.sublabel.clone(),
        );
    }

    // Group by the next arg value.
    let mut groups: HashMap<&String, Vec<(&Vec<String>, &AggEntry)>> = HashMap::new();
    for item in &matching {
        groups.entry(&item.0[pos]).or_default().push(*item);
    }

    // All share the same next arg → compress: no node created here,
    // recurse with same visual_depth so the eventual leaf lands at the right level.
    if groups.len() == 1 {
        let val = groups.keys().next().unwrap();
        let mut new_prefix = prefix.to_vec();
        new_prefix.push((*val).clone());
        return build_template_tree(
            name, entries, &new_prefix,
            visual_depth, parent_index,
            new_spans, counts,
        );
    }

    // Multiple distinct values at this position → create a wildcard aggregate node.
    let total_duration: f64 = matching.iter().map(|(_, e)| e.duration).sum();
    let total_count: usize = matching.iter().map(|(_, e)| e.count).sum();
    let identifier = build_wildcard_identifier(name, prefix, total_args);

    // Pick dominant type/sublabel from the largest child.
    let dominant = matching
        .iter()
        .max_by(|a, b| a.1.duration.partial_cmp(&b.1.duration).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(_, e)| (e.span_type, e.sublabel.clone()))
        .unwrap_or((SpanType::Template, "Template".to_string()));

    let index = push_span(
        new_spans, counts,
        identifier.clone(), identifier,
        total_duration, total_count,
        visual_depth, parent_index,
        dominant.0, dominant.1,
    );

    // Recurse for each distinct value group (longest duration first).
    let mut sorted_keys: Vec<&String> = groups.keys().copied().collect();
    sorted_keys.sort_by(|a, b| {
        let dur_a: f64 = groups[a].iter().map(|(_, e)| e.duration).sum();
        let dur_b: f64 = groups[b].iter().map(|(_, e)| e.duration).sum();
        dur_b.partial_cmp(&dur_a).unwrap_or(std::cmp::Ordering::Equal)
    });

    for val in sorted_keys {
        let mut new_prefix = prefix.to_vec();
        new_prefix.push(val.clone());
        let child_idx = build_template_tree(
            name, entries, &new_prefix,
            visual_depth + 1, Some(index),
            new_spans, counts,
        );
        new_spans[index].children_indices.push(child_idx);
    }

    index
}

/// Aggregate raw template/class spans into a hierarchical tree.
/// Returns (new_spans, counts).
fn aggregate_templates(raw_spans: &[Span]) -> (Vec<Span>, Vec<usize>) {
    // ── Pass 1: group all spans by bare template name ──────────────────────
    let mut by_base_name: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, span) in raw_spans.iter().enumerate() {
        if span.type_ == SpanType::Template || span.type_ == SpanType::Class {
            let (name, _) = parse_template_identifier(&span.identifier);
            by_base_name.entry(name.to_string()).or_default().push(i);
        }
    }

    let mut new_spans: Vec<Span> = Vec::new();
    let mut counts: Vec<usize> = Vec::new();

    // Sort base names for stable output (longest total duration first).
    let mut sorted_names: Vec<(String, f64)> = by_base_name
        .iter()
        .map(|(name, indices)| {
            let dur: f64 = indices.iter().map(|&i| raw_spans[i].duration).sum();
            (name.clone(), dur)
        })
        .collect();
    sorted_names.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    for (base_name, _) in &sorted_names {
        let indices = &by_base_name[base_name];

        // ── Pass 2: within this base name, group by arg_count → arg_tuple → AggEntry
        let mut by_arg_count: HashMap<usize, HashMap<Vec<String>, AggEntry>> = HashMap::new();
        for &i in indices {
            let span = &raw_spans[i];
            let (_, args) = parse_template_identifier(&span.identifier);
            let arg_map = by_arg_count.entry(args.len()).or_default();
            let entry = arg_map.entry(args).or_insert_with(|| AggEntry {
                duration: 0.0,
                count: 0,
                span_type: span.type_,
                sublabel: span.sublabel.clone().unwrap_or_default(),
            });
            entry.duration += span.duration;
            entry.count += 1;
            if span.duration > entry.duration - span.duration {
                entry.span_type = span.type_;
                entry.sublabel = span.sublabel.clone().unwrap_or_default();
            }
        }

        if by_arg_count.is_empty() {
            continue;
        }

        // ── Single arg count → existing flat behaviour (no top-level wrapper)
        if by_arg_count.len() == 1 {
            let (_, arg_groups) = by_arg_count.into_iter().next().unwrap();
            build_template_tree(base_name, &arg_groups, &[], 0, None, &mut new_spans, &mut counts);
            continue;
        }

        // ── Multiple arg counts → create top-level parent, then one child per arg count
        let top_total_dur: f64 = by_arg_count
            .values()
            .flat_map(|m| m.values())
            .map(|e| e.duration)
            .sum();
        let top_total_cnt: usize = by_arg_count
            .values()
            .flat_map(|m| m.values())
            .map(|e| e.count)
            .sum();
        // Pick dominant type/sublabel for the top-level parent.
        let top_dominant = by_arg_count
            .values()
            .flat_map(|m| m.values())
            .max_by(|a, b| a.duration.partial_cmp(&b.duration).unwrap_or(std::cmp::Ordering::Equal))
            .map(|e| (e.span_type, e.sublabel.clone()))
            .unwrap_or((SpanType::Template, "Template".to_string()));

        let top_index = push_span(
            &mut new_spans, &mut counts,
            base_name.clone(), base_name.clone(),
            top_total_dur, top_total_cnt,
            0, None,
            top_dominant.0, top_dominant.1,
        );

        // Sort arg counts by total duration (descending) for stable output.
        let mut sorted_counts: Vec<(usize, f64)> = by_arg_count
            .iter()
            .map(|(ac, m)| (*ac, m.values().map(|e| e.duration).sum()))
            .collect();
        sorted_counts.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

        for (arg_count, _) in sorted_counts {
            let arg_groups = by_arg_count.remove(&arg_count).unwrap();

            if arg_count == 0 {
                // Zero-arg: create a concrete leaf directly (no wildcard wrapper).
                let entry = arg_groups.into_values().next().unwrap();
                let child_idx = push_span(
                    &mut new_spans, &mut counts,
                    base_name.clone(), base_name.clone(),
                    entry.duration, entry.count,
                    1, Some(top_index),
                    entry.span_type, entry.sublabel,
                );
                new_spans[top_index].children_indices.push(child_idx);
            } else {
                // N-arg: create a wildcard node at depth 1, then build subtree at depth 2.
                let wildcard_id = build_wildcard_identifier(base_name, &[], arg_count);
                let child_total_dur: f64 = arg_groups.values().map(|e| e.duration).sum();
                let child_total_cnt: usize = arg_groups.values().map(|e| e.count).sum();
                let child_dominant = arg_groups
                    .values()
                    .max_by(|a, b| a.duration.partial_cmp(&b.duration).unwrap_or(std::cmp::Ordering::Equal))
                    .map(|e| (e.span_type, e.sublabel.clone()))
                    .unwrap_or((SpanType::Template, "Template".to_string()));

                let wildcard_index = push_span(
                    &mut new_spans, &mut counts,
                    wildcard_id.clone(), wildcard_id,
                    child_total_dur, child_total_cnt,
                    1, Some(top_index),
                    child_dominant.0, child_dominant.1,
                );
                new_spans[top_index].children_indices.push(wildcard_index);

                let subtree_root = build_template_tree(
                    base_name, &arg_groups, &[],
                    2, Some(wildcard_index),
                    &mut new_spans, &mut counts,
                );
                new_spans[wildcard_index].children_indices.push(subtree_root);
            }
        }
    }

    // Assign start_time positions in duration order: root spans (parent_index=None)
    // are laid out sequentially; children are positioned relative to their parent's
    // start_time.
    let mut root_offset = 0.0f64;
    for i in 0..new_spans.len() {
        if new_spans[i].parent_index.is_none() {
            new_spans[i].start_time = root_offset;
            root_offset += new_spans[i].duration;
        }
    }

    // Position children: each parent's children are laid out sequentially starting
    // from the parent's start_time.
    for i in 0..new_spans.len() {
        let parent_start = new_spans[i].start_time;
        let mut cursor = parent_start;
        let children: Vec<usize> = new_spans[i].children_indices.clone();
        for &ci in &children {
            new_spans[ci].start_time = cursor;
            cursor += new_spans[ci].duration;
        }
    }

    // Fix root_span_index: walk up to find the root ancestor.
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

        // Sort tracks: longest total duration first.
        track_roots.sort_by(|a, b| {
            let dur =
                |roots: &Vec<usize>| -> f64 { roots.iter().map(|&i| spans[i].duration).sum() };
            dur(b)
                .partial_cmp(&dur(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let (tracks_start_time, tracks_by_duration) = build_track_views(&spans, &track_roots);

        let mut root_track_map = HashMap::new();
        for (ti, roots) in track_roots.iter().enumerate() {
            for &root in roots {
                root_track_map.insert(root, ti);
            }
        }

        Self {
            spans,
            tracks_start_time,
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
        }
    }

    /// Templates tab is always ordered by duration.
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
                // Init to first displayed root span.
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
                    // Root span: navigate between visible roots in the same track.
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
        // Find first visible root span in the target track.
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
            _ => {}
        }
        false
    }

    fn handle_mouse_event(&mut self, mouse: crossterm::event::MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(crossterm::event::MouseButton::Left) => {
                let coord = (mouse.column, mouse.row);
                if let Some(&si) = self.cell_span_map.get(&coord) {
                    self.selected_span = Some(si);
                }
            }
            MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft => {
                let max_scroll = self.content_height.saturating_sub(self.viewport_height);
                self.vertical_scroll = self.vertical_scroll.saturating_sub(3).min(max_scroll);
            }
            MouseEventKind::ScrollDown | MouseEventKind::ScrollRight => {
                let max_scroll = self.content_height.saturating_sub(self.viewport_height);
                self.vertical_scroll = self.vertical_scroll.saturating_add(3).min(max_scroll);
            }
            _ => {}
        }
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let total_duration = self.total_duration();
        let visible_duration = total_duration / self.zoom;

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
                    .required_height(area.width)
                })
                .unwrap_or(0)
        } else {
            0
        };
        let graph_height = area
            .height
            .saturating_sub(scrollbar_height + details_height);
        let vertical_scrollbar_width: u16 = if area.width > 1 { 1 } else { 0 };
        let graph_width = area.width.saturating_sub(vertical_scrollbar_width);

        let scrollbar_area = Rect::new(area.x, area.y, area.width, scrollbar_height);
        let graph_area = Rect::new(area.x, area.y + scrollbar_height, graph_width, graph_height);
        let vscrollbar_area = Rect::new(
            area.x + graph_width,
            area.y + scrollbar_height,
            vertical_scrollbar_width,
            graph_height,
        );
        let details_area = Rect::new(
            area.x,
            area.y + scrollbar_height + graph_height,
            area.width,
            details_height,
        );

        let start_time = self.start_time;
        let scrollbar = DurationRange {
            total_duration,
            start: self.start_time,
            visible_duration,
        };
        scrollbar.render(scrollbar_area, buf);

        self.cell_span_map.clear();
        self.viewport_height = graph_height;
        self.viewport_width = graph_area.width;
        let selected_span = self.selected_span;

        use crate::widgets::track::track_content_height;
        let label_height: u16 = 1;

        // Compute per-track heights
        let heights: Vec<u16> = self
            .tracks_by_duration
            .iter()
            .map(|views| {
                track_content_height(views, &self.spans, visible_duration, graph_area.width)
                    + label_height
            })
            .collect();

        // Build TrackInputs (always by_duration).
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
            search_query: None,
        }
        .render(graph_area, buf);

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
                    "┃",
                    active_style,
                );
            }
        }

        if let Some(si) = self.selected_span {
            if let Some(span) = self.spans.get(si) {
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
        ]
    }

    fn set_search_query(&mut self, _query: String) {}
}
