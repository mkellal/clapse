use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;
use std::collections::HashMap;
use std::rc::Rc;
use std::time::Instant;

use crate::app::span::{Span, SpanType};
use crate::app::tabs::Tab;
use crate::app::view::OrderBy;
use crate::widgets::flamegraph::{self, FlamegraphWidget};
use crate::widgets::pch_candidates::{PchCandidate, CandidatesWidget, CopyMode};

pub struct SourcesTab {
    flamegraph: FlamegraphWidget,
    /// PCH candidates (sorted by total_duration descending).
    pub pch_candidates: Vec<PchCandidate>,
    /// Maps file identifier → list of span indices for cross-referencing.
    pub pch_span_map: HashMap<String, Vec<usize>>,
    pub pch_scroll_offset: u16,
    pub pch_selected_index: Option<usize>,
    /// Stored during render for mouse hit-testing.
    pch_rect: Rect,
    /// When Some, the copy button shows "✓ Copied!" until this instant.
    copy_confirmed_at: Option<Instant>,
}

impl SourcesTab {
    pub fn new(raw_spans: Rc<[Span]>) -> Self {
        let (aggregated, counts) = aggregate_sources(&raw_spans);
        let spans: Rc<[Span]> = Rc::from(aggregated);

        // --- Compute PCH candidates: group by file identifier across all TUs ---
        let (pch_candidates, pch_span_map) = compute_pch_candidates(&spans, &counts);

        let flamegraph = FlamegraphWidget::new(
            spans,
            Some(vec!["Sources".to_string()]),
            OrderBy::Duration,
            Some(counts.clone()),
            false,
        );

        Self {
            flamegraph,
            pch_candidates,
            pch_span_map,
            pch_scroll_offset: 0,
            pch_selected_index: None,
            pch_rect: Rect::default(),
            copy_confirmed_at: None,
        }
    }

    fn copy_includes_to_clipboard(&mut self) {
        let includes = CandidatesWidget {
            title: "",
            candidates: &self.pch_candidates,
            scroll_offset: 0,
            selected_index: None,
            copy_confirmed: false,
            copy_mode: CopyMode::Includes,
        }
        .build_copy_text();
        let _ = flamegraph::write_to_clipboard(&includes);
        self.copy_confirmed_at = Some(Instant::now());
    }
}

impl Tab for SourcesTab {
    fn get_label(&self) -> &str {
        "Sources"
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);

        // Extra key: Ctrl+Y → copy PCH #includes
        if key.code == KeyCode::Char('y') && ctrl {
            self.copy_includes_to_clipboard();
            return false;
        }
        self.flamegraph.handle_key_event(key)
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                let coord = (mouse.column, mouse.row);

                // Check PCH panel copy button first
                let pch_rect = self.pch_rect;
                if pch_rect.width > 0 {
                    let copy_confirmed = self
                        .copy_confirmed_at
                        .map_or(false, |t| Instant::now().duration_since(t).as_secs() < 3);
                    let widget = CandidatesWidget {
                        title: "PCH Candidates",
                        candidates: &self.pch_candidates,
                        scroll_offset: self.pch_scroll_offset,
                        selected_index: self.pch_selected_index,
                        copy_confirmed,
                        copy_mode: CopyMode::Includes,
                    };
                    if widget.hit_copy_button(pch_rect, coord.0, coord.1) {
                        self.copy_includes_to_clipboard();
                        return;
                    }
                }

                // Check PCH candidate list clicks
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
                        let idx = self.pch_scroll_offset as usize
                            + (row_in_list / CandidatesWidget::CANDIDATE_ROWS) as usize;
                        if idx < self.pch_candidates.len() {
                            self.pch_selected_index = Some(idx);
                            let ident = self.pch_candidates[idx].identifier.clone();
                            if let Some(indices) = self.pch_span_map.get(&ident) {
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
                        .pch_candidates
                        .len()
                        .saturating_sub(visible_count as usize) as u16;
                    self.pch_scroll_offset =
                        self.pch_scroll_offset.saturating_sub(1).min(max_scroll);
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
                        .pch_candidates
                        .len()
                        .saturating_sub(visible_count as usize) as u16;
                    self.pch_scroll_offset =
                        self.pch_scroll_offset.saturating_add(1).min(max_scroll);
                    return;
                }
                self.flamegraph.handle_mouse_event(mouse);
            }
            _ => {}
        }
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        // --- Layout: left (flamegraph), right (PCH full-height) ---
        let pch_width: u16 = if area.width >= 80 { 40 } else { 0 };
        let left_width = area.width.saturating_sub(pch_width);
        let left_area = Rect::new(area.x, area.y, left_width, area.height);

        // Render flamegraph in left area
        self.flamegraph.render(left_area, buf);

        // --- Right section: PCH panel (full height) ---
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
                title: "PCH Candidates",
                candidates: &self.pch_candidates,
                scroll_offset: self.pch_scroll_offset,
                selected_index: self.pch_selected_index,
                copy_confirmed,
                copy_mode: CopyMode::Includes,
            }
            .render(pch_area, buf);
        }
    }

    fn get_help(&self) -> Vec<(&str, &str)> {
        let mut help = self.flamegraph.get_help();
        help.push(("Ctrl + Y", "Copy PCH #includes"));
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


fn aggregate_sources(raw_spans: &[Span]) -> (Vec<Span>, Vec<usize>) {
    let mut tree: HashMap<Vec<String>, (f64, usize, String, Option<String>)> = HashMap::new();

    for span in raw_spans.iter().filter(|s| s.type_ == SpanType::Source) {
        let mut path = Vec::new();
        let mut curr = Some(span);
        while let Some(s) = curr {
            if s.type_ == SpanType::Source {
                path.push(s.identifier.clone());
            }
            curr = s.parent_index.and_then(|i| raw_spans.get(i));
        }
        path.reverse();

        let entry = tree.entry(path).or_insert((0.0, 0, span.label.clone(), span.sublabel.clone()));
        entry.0 += span.duration;
        entry.1 += 1;
    }

    let mut paths: Vec<_> = tree.keys().cloned().collect();
    paths.sort_by(|a, b| {
        if a.len() != b.len() {
            a.len().cmp(&b.len())
        } else {
            let dur_a = tree.get(a).unwrap().0;
            let dur_b = tree.get(b).unwrap().0;
            dur_b.partial_cmp(&dur_a).unwrap_or(std::cmp::Ordering::Equal)
        }
    });

    let mut new_spans: Vec<Span> = Vec::new();
    let mut counts: Vec<usize> = Vec::new();
    let mut path_to_index = HashMap::new();
    let mut current_offset_at_path: HashMap<Vec<String>, f64> = HashMap::new();

    for path in paths {
        let (duration, count, label, sublabel) = tree.get(&path).unwrap().clone();
        let parent_path = if path.len() > 1 {
            Some(path[0..path.len() - 1].to_vec())
        } else {
            None
        };

        let parent_index = parent_path
            .as_ref()
            .and_then(|p| path_to_index.get(p))
            .copied();

        let start_time = if let Some(ref pp) = parent_path {
            let p_index = parent_index.unwrap();
            let p_start = new_spans[p_index as usize].start_time;
            let offset = current_offset_at_path.entry(pp.clone()).or_insert(0.0);
            let s = p_start + *offset;
            *offset += duration;
            s
        } else {
            let offset = current_offset_at_path.entry(vec![]).or_insert(0.0);
            let s = *offset;
            *offset += duration;
            s
        };

        let depth = path.len() - 1;
        let index = new_spans.len();
        new_spans.push(Span {
            type_: SpanType::Source,
            identifier: path.last().unwrap().clone(),
            label,
            sublabel,
            start_time,
            duration,
            parent_index,
            children_indices: Vec::new(),
            root_span_index: 0,
            depth,
        });
        counts.push(count);
        path_to_index.insert(path.clone(), index);

        if let Some(pi) = parent_index {
            new_spans[pi].children_indices.push(index);
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

/// Compute PCH candidates by grouping aggregated spans by file identifier
/// (ignoring ancestor path). Returns candidates sorted by total_duration desc,
/// plus a map from identifier → list of span indices.
fn compute_pch_candidates(
    spans: &[Span],
    counts: &[usize],
) -> (Vec<PchCandidate>, HashMap<String, Vec<usize>>) {
    let mut map: HashMap<String, (f64, usize)> = HashMap::new();
    let mut span_map: HashMap<String, Vec<usize>> = HashMap::new();

    for (i, span) in spans.iter().enumerate() {
        let entry = map.entry(span.identifier.clone()).or_insert((0.0, 0));
        entry.0 += span.duration;
        entry.1 += counts[i];
        span_map
            .entry(span.identifier.clone())
            .or_default()
            .push(i);
    }

    let mut candidates: Vec<PchCandidate> = map
        .into_iter()
        .filter(|(_, (_, total_count))| *total_count > 1)
        .map(|(ident, (total_dur, total_count))| {
            PchCandidate::new(
                ident.clone(),
                span_label_for_identifier(spans, &ident),
                total_dur,
                total_count,
            )
        })
        .collect();

    candidates.sort_by(|a, b| {
        b.total_duration
            .partial_cmp(&a.total_duration)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates.truncate(10);

    let top_idents: std::collections::HashSet<String> = candidates
        .iter()
        .map(|c| c.identifier.clone())
        .collect();
    span_map.retain(|k, _| top_idents.contains(k));

    (candidates, span_map)
}

fn span_label_for_identifier(spans: &[Span], identifier: &str) -> String {
    spans
        .iter()
        .find(|s| s.identifier == identifier)
        .map(|s| s.label.clone())
        .unwrap_or_else(|| identifier.to_string())
}

