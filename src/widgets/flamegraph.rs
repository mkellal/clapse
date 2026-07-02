use std::collections::HashMap;
use std::io::Write;
use std::rc::Rc;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{self, Line};
use ratatui::widgets::{Block, Borders, Widget};

use super::track::{TrackInput, track_content_height};
use crate::app::span::Span;
use crate::app::view::{
    AggregateSpanView, FollowingSpanDirection, HorizontalDirection, OrderBy, SpanView,
    build_track_views, get_following_span_index, schedule_spans,
};
use crate::widgets::span_details::SpanDetails;
use crate::widgets::time_range::DurationRange;

// ---------------------------------------------------------------------------
// Zoom direction (private to this module)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy)]
enum ZoomDirection {
    In,
    Out,
}

// ---------------------------------------------------------------------------
// FlamegraphWidget — self-contained interactive flamegraph component
// ---------------------------------------------------------------------------

/// Owns all flamegraph state: spans, tracks, zoom, scroll, selection, search.
/// Handles rendering, keyboard navigation, mouse interaction, and search.
pub struct FlamegraphWidget {
    pub spans: Rc<[Span]>,
    tracks_start_time: Vec<Vec<SpanView>>,
    tracks_by_duration: Vec<Vec<SpanView>>,
    root_track_map: HashMap<usize, usize>,
    zoom: f64,
    start_time: f64,
    pub selected_span: Option<usize>,
    cell_span_map: HashMap<(u16, u16), usize>,
    order_by: OrderBy,
    vertical_scroll: u16,
    viewport_height: u16,
    viewport_width: u16,
    content_height: u16,
    search_query: Option<String>,
    /// When `Some`, the details panel shows aggregated counts (Sources/Templates tabs).
    counts: Option<Vec<usize>>,
    /// Labels for each track (e.g. "Thread 0", "Sources", "Templates").
    track_labels: Vec<String>,
    /// Whether sort-mode toggling is enabled (shows `<m>` title in border).
    sortable: bool,
}

impl FlamegraphWidget {
    // ── Construction ────────────────────────────────────────────────────

    /// Build a new `FlamegraphWidget` from raw spans.
    ///
    /// * `track_labels` — one label per track. If `None`, auto-generates "Thread N".
    /// * `order_by` — initial sort order.
    /// * `counts` — per-span occurrence counts for aggregated views (Sources/Templates).
    /// * `sortable` — if true, sort mode can be toggled and `<m>` title is shown.
    pub fn new(
        raw_spans: Rc<[Span]>,
        track_labels: Option<Vec<String>>,
        order_by: OrderBy,
        counts: Option<Vec<usize>>,
        sortable: bool,
    ) -> Self {
        let mut track_roots = schedule_spans(&raw_spans);

        // Sort tracks: longest total duration first.
        track_roots.sort_by(|a, b| {
            let dur =
                |roots: &Vec<usize>| -> f64 { roots.iter().map(|&i| raw_spans[i].duration).sum() };
            dur(b)
                .partial_cmp(&dur(a))
                .unwrap_or(std::cmp::Ordering::Equal)
        });

        let (tracks_start_time, tracks_by_duration) = build_track_views(&raw_spans, &track_roots);

        let mut root_track_map = HashMap::new();
        for (ti, roots) in track_roots.iter().enumerate() {
            for &root in roots {
                root_track_map.insert(root, ti);
            }
        }

        let labels = track_labels.unwrap_or_else(|| {
            (0..tracks_start_time.len())
                .map(|i| format!("Thread {}", i))
                .collect()
        });

        Self {
            spans: raw_spans,
            tracks_start_time,
            tracks_by_duration,
            root_track_map,
            zoom: 1.0,
            start_time: 0.0,
            selected_span: None,
            cell_span_map: HashMap::new(),
            order_by,
            vertical_scroll: 0,
            viewport_height: 0,
            viewport_width: 0,
            content_height: 0,
            search_query: None,
            counts,
            track_labels: labels,
            sortable,
        }
    }

    // ── Accessors ───────────────────────────────────────────────────────

    fn current_tracks(&self) -> &[Vec<SpanView>] {
        match self.order_by {
            OrderBy::StartTime => &self.tracks_start_time,
            OrderBy::Duration => &self.tracks_by_duration,
        }
    }

    fn track_index_for_span(&self, span_index: usize) -> Option<usize> {
        let root = self.spans.get(span_index)?.root_span_index;
        self.root_track_map.get(&root).copied()
    }

    pub fn total_duration(&self) -> f64 {
        self.spans
            .iter()
            .filter(|s| s.parent_index.is_none() && s.duration.is_finite())
            .map(|s| s.start_time + s.duration)
            .fold(0.0f64, f64::max)
    }

    fn visible_duration(&self) -> f64 {
        self.total_duration() / self.zoom
    }

    // ── Selection navigation ────────────────────────────────────────────

    pub fn move_selection(&mut self, direction: FollowingSpanDirection) {
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
                        let track_views = self.current_tracks();
                        let views = &track_views[ti];
                        let mut seen = std::collections::HashSet::new();
                        let roots: Vec<usize> = views
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
                        let track_views = self.current_tracks();
                        let views = &track_views[ti];
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
                    let track_views = self.current_tracks();
                    let views = &track_views[ti];
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

    // ── Track navigation ────────────────────────────────────────────────

    fn compute_track_positions(&self) -> Vec<(u16, u16)> {
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

    pub fn center_track(&mut self, track_idx: usize) {
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

    pub fn center_selected_track(&mut self) {
        let Some(si) = self.selected_span else { return };
        let Some(ti) = self.track_index_for_span(si) else {
            return;
        };
        self.center_track(ti);
    }

    pub fn toggle_sort_mode(&mut self) {
        if !self.sortable {
            return;
        }
        self.order_by = match self.order_by {
            OrderBy::StartTime => OrderBy::Duration,
            OrderBy::Duration => OrderBy::StartTime,
        };
        self.zoom_to_selected(Some(self.zoom));
        self.center_selected_track();
    }

    pub fn switch_track(&mut self, dir: HorizontalDirection) {
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
            let track_views = self.current_tracks();
            let views = &track_views[new_ti];
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

    // ── Zoom / pan ──────────────────────────────────────────────────────

    fn zoom_around_center(&mut self, factor: f64) {
        let center = self.start_time + self.visible_duration() / 2.0;
        self.zoom = (self.zoom * factor).max(1.0);
        let new_half = self.visible_duration() / 2.0;
        self.start_time = (center - new_half).max(0.0);
        let max_start = (self.total_duration() - self.visible_duration()).max(0.0);
        self.start_time = self.start_time.min(max_start);
        self.center_selected_track();
    }

    pub fn zoom_to_selected(&mut self, factor: Option<f64>) {
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
            let track_views = self.current_tracks();
            let views = &track_views[ti];
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

    // ── Clipboard ───────────────────────────────────────────────────────

    pub fn copy_span_identifier(&self) {
        if let Some(si) = self.selected_span {
            if let Some(span) = self.spans.get(si) {
                let _ = write_to_clipboard(&span.identifier);
            }
        }
    }

    // ── Search ──────────────────────────────────────────────────────────

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
                if direction > 0 {
                    0
                } else {
                    matches.len() - 1
                }
            }
        };
        self.selected_span = Some(matches[new_pos]);
        self.zoom_to_selected(None);
        self.center_selected_track();
    }

    pub fn set_search_query(&mut self, query: String) {
        if query.is_empty() {
            self.search_query = None;
        } else {
            self.search_query = Some(query);
        }
    }

    pub fn select_next_match(&mut self) {
        self.navigate_match(1);
    }

    pub fn select_previous_match(&mut self) {
        self.navigate_match(-1);
    }

    pub fn match_count(&self) -> usize {
        self.find_matches().len()
    }

    pub fn current_match_index(&self) -> Option<usize> {
        let matches = self.find_matches();
        self.selected_span
            .and_then(|sel| matches.iter().position(|&m| m == sel))
    }

    // ── Input handling ──────────────────────────────────────────────────

    /// Handle a key event. Returns `true` if the app should quit.
    /// Call this from your tab's `handle_key_event`, then add extra keys on top.
    pub fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
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
            KeyCode::Char('+') | KeyCode::Char('=') => self.zoom(1.1, ZoomDirection::In),
            KeyCode::Char('-') => self.zoom(1.1, ZoomDirection::Out),
            KeyCode::Char('r') => {
                self.zoom = 1.0;
                self.start_time = 0.0;
                self.center_selected_track();
            }
            KeyCode::Char(' ') => self.zoom_to_selected(None),
            KeyCode::Esc => self.selected_span = None,
            KeyCode::Tab => self.switch_track(HorizontalDirection::Next),
            KeyCode::BackTab => self.switch_track(HorizontalDirection::Previous),
            KeyCode::Char('y') => self.copy_span_identifier(),
            _ => {}
        }
        false
    }

    /// Handle a mouse event for the flamegraph area only.
    /// Call this after your tab has checked for side-panel clicks.
    pub fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        match mouse.kind {
            MouseEventKind::Down(_) => {
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

    // ── Help ────────────────────────────────────────────────────────────

    /// Returns the base help entries shared by all flamegraph views.
    pub fn get_help(&self) -> Vec<(&'static str, &'static str)> {
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
            ("+/-", "Zoom in/out"),
            ("r", "Reset zoom"),
            ("Space", "Zoom to selection"),
            ("Esc", "Clear selection"),
            ("Tab", "Next track"),
            ("Shift + Tab", "Previous track"),
            ("y", "Copy span identifier"),
        ]
    }

    // ── Rendering ───────────────────────────────────────────────────────

    /// Render the full flamegraph section (scrollbar + tracks + vscrollbar + details)
    /// into the given area. The area should already include any outer border; this
    /// method renders its own inner border.
    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        let total_duration = self.total_duration();
        let visible_duration = total_duration / self.zoom;

        // ── bordered block ──────────────────────────────────────────
        let block = if self.sortable {
            let mode_label = match self.order_by {
                OrderBy::StartTime => "Start Time",
                OrderBy::Duration => "Duration",
            };
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
                .title(Line::from(vec![
                    text::Span::styled(" <", Color::DarkGray),
                    text::Span::styled("m", Color::Red),
                    text::Span::styled("> ", Color::DarkGray),
                    text::Span::raw("Sort Mode: "),
                    text::Span::raw(mode_label),
                    text::Span::raw(" "),
                ]))
                .title_alignment(Alignment::Center)
                .title_style(Style::default().fg(Color::Gray))
        } else {
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray))
        };
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        // ── compute details height ──────────────────────────────────
        let details_height = self.compute_details_height(inner.width);

        let scrollbar_height = 2;
        let graph_height = inner
            .height
            .saturating_sub(scrollbar_height + details_height);

        let scrollbar_area = Rect::new(inner.x, inner.y, inner.width, scrollbar_height);
        let graph_outer = Rect::new(
            inner.x,
            inner.y + scrollbar_height,
            inner.width,
            graph_height,
        );
        let details_area = Rect::new(
            inner.x,
            inner.y + scrollbar_height + graph_height,
            inner.width,
            details_height,
        );

        let vscrollbar_width: u16 = if graph_outer.width > 1 { 1 } else { 0 };
        let graph_width = graph_outer.width.saturating_sub(vscrollbar_width);
        let vscrollbar_area = Rect::new(
            graph_outer.x + graph_width,
            graph_outer.y,
            vscrollbar_width,
            graph_outer.height,
        );
        let graph_area = Rect::new(
            graph_outer.x,
            graph_outer.y,
            graph_width,
            graph_outer.height,
        );

        // ── timeline scrollbar ──────────────────────────────────────
        DurationRange {
            total_duration,
            start: self.start_time,
            visible_duration,
        }
        .render(scrollbar_area, buf);

        // ── flamegraph tracks ───────────────────────────────────────
        self.cell_span_map.clear();
        self.viewport_height = graph_area.height;
        self.viewport_width = graph_width;

        let label_height: u16 = 1;
        let heights: Vec<u16> = self
            .current_tracks()
            .iter()
            .map(|views| {
                track_content_height(views, &self.spans, visible_duration, graph_area.width)
                    + label_height
            })
            .collect();

        // Clone labels upfront to avoid borrow conflicts.
        let labels: Vec<Option<String>> = self
            .track_labels
            .iter()
            .enumerate()
            .map(|(i, label)| {
                if i < heights.len() {
                    Some(label.clone())
                } else {
                    None
                }
            })
            .collect();

        self.content_height = heights.iter().sum();
        let max_scroll = self.content_height.saturating_sub(graph_area.height);
        self.vertical_scroll = self.vertical_scroll.min(max_scroll);

        // Extract values into locals before mutable borrow of tracks.
        let selected_span = self.selected_span;
        let start_time = self.start_time;
        let vertical_scroll = self.vertical_scroll;
        let spans: Rc<[Span]> = Rc::clone(&self.spans);
        let search_query = self.search_query.clone();

        // Build TrackInputs via direct field access to avoid borrowing all of self.
        let tracks_mut: &mut [Vec<SpanView>] = match self.order_by {
            OrderBy::StartTime => &mut self.tracks_start_time,
            OrderBy::Duration => &mut self.tracks_by_duration,
        };

        let track_inputs: Vec<TrackInput> = tracks_mut
            .iter_mut()
            .zip(heights.iter().copied().enumerate())
            .map(|(views_vec, (track_idx, intrinsic_height))| TrackInput {
                label: labels.get(track_idx).cloned().flatten(),
                views: views_vec.as_mut_slice(),
                intrinsic_height,
            })
            .collect();

        // Render tracks. track_inputs borrows tracks_mut, so self.cell_span_map,
        // and other fields are independently accessible.
        {
            let viewport_top = vertical_scroll;
            let viewport_bottom = vertical_scroll.saturating_add(graph_area.height);
            let mut virtual_y: u16 = 0;

            for track in track_inputs {
                let track_top = virtual_y;
                let track_bottom = virtual_y.saturating_add(track.intrinsic_height);
                virtual_y = track_bottom;

                if track_bottom <= viewport_top {
                    continue;
                }
                if track_top >= viewport_bottom {
                    break;
                }

                let overlap_start = track_top.max(viewport_top);
                let overlap_end = track_bottom.min(viewport_bottom);
                let visible_rows = overlap_end - overlap_start;
                let row_skip = overlap_start - track_top;
                let render_y = graph_area.y + (overlap_start - viewport_top);
                let track_area = Rect::new(graph_area.x, render_y, graph_area.width, visible_rows);

                super::track::TrackWidget {
                    label: track.label.as_deref(),
                    spans: &spans,
                    views: track.views,
                    total_duration: visible_duration,
                    start_time,
                    row_skip,
                    selected_span,
                    cell_map: &mut self.cell_span_map,
                    search_query: search_query.as_deref(),
                }
                .render(track_area, buf);
            }
        }

        // ── vertical scrollbar ──────────────────────────────────────
        if vscrollbar_area.width > 0 && vscrollbar_area.height > 0 {
            let muted_style = Style::default().fg(Color::DarkGray);
            let active_style = Style::default().fg(Color::White);

            for y in vscrollbar_area.y..vscrollbar_area.bottom() {
                buf.set_string(vscrollbar_area.x, y, "│", muted_style);
            }

            let thumb_height = if self.content_height <= vscrollbar_area.height {
                vscrollbar_area.height
            } else {
                (((vscrollbar_area.height as u32) * (vscrollbar_area.height as u32)
                    + self.content_height as u32
                    - 1)
                    / self.content_height as u32) as u16
            }
            .clamp(1, vscrollbar_area.height);

            let thumb_start = if max_scroll == 0 {
                0
            } else {
                let max_thumb_start = vscrollbar_area.height.saturating_sub(thumb_height) as u32;
                ((self.vertical_scroll as u32) * max_thumb_start / max_scroll as u32) as u16
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

        // ── details panel ───────────────────────────────────────────
        self.render_details(details_area, buf, total_duration);
    }

    /// Compute how many rows the details panel needs at the given width.
    fn compute_details_height(&self, area_width: u16) -> u16 {
        let si = match self.selected_span {
            Some(si) => si,
            None => return 0,
        };
        let span = match self.spans.get(si) {
            Some(s) => s,
            None => return 0,
        };
        let parent_duration = span
            .parent_index
            .and_then(|pi| self.spans.get(pi))
            .map(|p| p.duration);

        match &self.counts {
            Some(counts) => {
                let view = AggregateSpanView {
                    view: SpanView {
                        span_index: si,
                        ..Default::default()
                    },
                    count: counts[si],
                };
                SpanDetails {
                    spans: &self.spans,
                    view: &view,
                    parent_duration,
                    total_duration: self.total_duration(),
                }
                .required_height(area_width)
            }
            None => {
                let view = SpanView {
                    span_index: si,
                    ..Default::default()
                };
                SpanDetails {
                    spans: &self.spans,
                    view: &view,
                    parent_duration,
                    total_duration: self.total_duration(),
                }
                .required_height(area_width)
            }
        }
    }

    /// Render the details panel for the currently selected span.
    fn render_details(&self, area: Rect, buf: &mut Buffer, total_duration: f64) {
        let si = match self.selected_span {
            Some(si) => si,
            None => return,
        };
        let span = match self.spans.get(si) {
            Some(s) => s,
            None => return,
        };
        let parent_duration = span
            .parent_index
            .and_then(|pi| self.spans.get(pi))
            .map(|p| p.duration);

        match &self.counts {
            Some(counts) => {
                let view = AggregateSpanView {
                    view: SpanView {
                        span_index: si,
                        ..Default::default()
                    },
                    count: counts[si],
                };
                SpanDetails {
                    spans: &self.spans,
                    view: &view,
                    parent_duration,
                    total_duration,
                }
                .render(area, buf);
            }
            None => {
                let view = SpanView {
                    span_index: si,
                    ..Default::default()
                };
                SpanDetails {
                    spans: &self.spans,
                    view: &view,
                    parent_duration,
                    total_duration,
                }
                .render(area, buf);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Clipboard utilities (pub(crate) so tabs can use them for side-panel copy)
// ---------------------------------------------------------------------------

/// Write text to the terminal clipboard using OSC 52 escape sequence.
pub(crate) fn write_to_clipboard(text: &str) -> std::io::Result<()> {
    let encoded = base64_encode(text);
    let mut stdout = std::io::stdout().lock();
    write!(stdout, "\x1b]52;c;{}\x07", encoded)?;
    stdout.flush()?;
    Ok(())
}

/// Simple base64 encoder (no external dependency needed).
pub(crate) fn base64_encode(input: &str) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let bytes = input.as_bytes();
    let mut result = String::new();
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
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
