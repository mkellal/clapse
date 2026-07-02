use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::time::Instant;

use crate::app::span::{Span, SpanType};
use crate::app::tabs::Tab;
use crate::app::view::OrderBy;
use crate::widgets::flamegraph::{self, FlamegraphWidget};
use crate::widgets::pch_candidates::{PchCandidate, CandidatesWidget, CopyMode};

pub struct TemplatesTab {
    flamegraph: FlamegraphWidget,
    /// Extern candidates: top 10 slowest concrete template instantiations.
    pub extern_candidates: Vec<PchCandidate>,
    /// Maps identifier → list of span indices for cross-referencing.
    pub extern_span_map: HashMap<String, Vec<usize>>,
    pub extern_scroll_offset: u16,
    pub extern_selected_index: Option<usize>,
    pch_rect: Rect,
    copy_confirmed_at: Option<Instant>,
}

impl TemplatesTab {
    pub fn new(raw_spans: Rc<[Span]>) -> Self {
        let (aggregated, counts) = aggregate_templates(&raw_spans);
        let spans: Rc<[Span]> = Rc::from(aggregated);

        // --- Compute extern candidates: leaf template spans (concrete instantiations) ---
        let (extern_candidates, extern_span_map) = compute_extern_candidates(&spans, &counts);

        let flamegraph = FlamegraphWidget::new(
            spans,
            Some(vec!["Templates".to_string()]),
            OrderBy::Duration,
            Some(counts.clone()),
        );

        Self {
            flamegraph,
            extern_candidates,
            extern_span_map,
            extern_scroll_offset: 0,
            extern_selected_index: None,
            pch_rect: Rect::default(),
            copy_confirmed_at: None,
        }
    }

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
        let _ = flamegraph::write_to_clipboard(&text);
        self.copy_confirmed_at = Some(Instant::now());
    }
}

impl Tab for TemplatesTab {
    fn get_label(&self) -> &str {
        "Templates"
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Extra key: Ctrl+Y → copy extern #includes
        if key.code == KeyCode::Char('y') && ctrl {
            self.copy_externs_to_clipboard();
            return false;
        }
        self.flamegraph.handle_key_event(key)
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
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
                                    self.flamegraph.selected_span = Some(si);
                                    self.flamegraph.zoom_to_selected(None);
                                    self.flamegraph.center_selected_track();
                                }
                            }
                        }
                        return;
                    }
                }

                // Fallthrough: delegate to flamegraph
                self.flamegraph.handle_mouse_event(mouse);
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
                    self.extern_scroll_offset =
                        self.extern_scroll_offset.saturating_sub(1).min(max_scroll);
                    return;
                }
                self.flamegraph.handle_mouse_event(mouse);
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
                    self.extern_scroll_offset =
                        self.extern_scroll_offset.saturating_add(1).min(max_scroll);
                    return;
                }
                self.flamegraph.handle_mouse_event(mouse);
            }
            _ => {}
        }
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        // --- Layout: left (flamegraph), right (extern candidates full-height) ---
        let pch_width: u16 = if area.width >= 80 { 40 } else { 0 };
        let left_width = area.width.saturating_sub(pch_width);
        let left_area = Rect::new(area.x, area.y, left_width, area.height);

        // Render flamegraph in left area
        self.flamegraph.render(left_area, buf);

        // --- Right section: extern candidates panel ---
        let pch_area = if pch_width > 0 {
            Rect::new(area.x + left_width, area.y, pch_width, area.height)
        } else {
            Rect::default()
        };
        self.pch_rect = pch_area;

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
    }

    fn get_help(&self) -> Vec<(&str, &str)> {
        let mut help = self.flamegraph.get_help();
        help.push(("Ctrl + Y", "Copy extern #includes"));
        help
    }

    fn set_search_query(&mut self, query: String) {
        self.flamegraph.set_search_query(query);
    }

    fn select_next_match(&mut self) {
        self.flamegraph.select_next_match();
    }

    fn select_previous_match(&mut self) {
        self.flamegraph.select_previous_match();
    }

    fn match_count(&self) -> usize {
        self.flamegraph.match_count()
    }

    fn current_match_index(&self) -> Option<usize> {
        self.flamegraph.current_match_index()
    }
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
            // is a wildcard (has its own children), flatten.
            if new_spans[top_idx].children_indices.len() == 1 {
                let child_idx = new_spans[top_idx].children_indices[0];
                let grandchildren: Vec<usize> = new_spans[child_idx].children_indices.clone();
                if !grandchildren.is_empty() {
                    for &gc in &grandchildren {
                        new_spans[gc].parent_index = Some(top_idx);
                        decrement_depths(&mut new_spans, gc);
                    }
                    new_spans[top_idx].children_indices = grandchildren;
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

/// Compute extern candidates: top 10 leaf template spans (concrete instantiations)
/// appearing in 2+ TUs, sorted by duration descending.
fn compute_extern_candidates(
    spans: &[Span],
    counts: &[usize],
) -> (Vec<PchCandidate>, HashMap<String, Vec<usize>>) {
    let mut map: HashMap<String, (f64, usize)> = HashMap::new();
    let mut span_map: HashMap<String, Vec<usize>> = HashMap::new();

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
