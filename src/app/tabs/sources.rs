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
    AggregateSpanView, FollowingSpanDirection, HorizontalDirection, OrderBy, SpanView,
    build_track_views, get_following_span_index, schedule_spans,
};
use crate::widgets::flamegraph::FlamegraphWidget;
use crate::widgets::span_details::SpanDetails;
use crate::widgets::time_range::DurationRange;
use crate::widgets::track::TrackInput;

pub struct SourcesTab {
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
    pub order_by: OrderBy,
    pub vertical_scroll: u16,
    pub viewport_height: u16,
    pub viewport_width: u16,
    pub content_height: u16,
    pub counts: Vec<usize>,
    pub search_query: Option<String>,
}

impl SourcesTab {
    pub fn new(raw_spans: Rc<[Span]>) -> Self {
        let (aggregated, counts) = aggregate_sources(&raw_spans);
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
            order_by: OrderBy::Duration,
            vertical_scroll: 0,
            viewport_height: 0,
            viewport_width: 0,
            content_height: 0,
            counts,
            search_query: None,
        }
    }

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
                        let track_views = match self.order_by {
                            OrderBy::StartTime => &self.tracks_start_time[ti],
                            OrderBy::Duration => &self.tracks_by_duration[ti],
                        };
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
                        let views = match self.order_by {
                            OrderBy::StartTime => &self.tracks_start_time[ti],
                            OrderBy::Duration => &self.tracks_by_duration[ti],
                        };
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
                    let views = match self.order_by {
                        OrderBy::StartTime => &self.tracks_start_time[ti],
                        OrderBy::Duration => &self.tracks_by_duration[ti],
                    };
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
            let views = match self.order_by {
                OrderBy::StartTime => &self.tracks_start_time[new_ti],
                OrderBy::Duration => &self.tracks_by_duration[new_ti],
            };
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
            let views = match self.order_by {
                OrderBy::StartTime => &self.tracks_start_time[ti],
                OrderBy::Duration => &self.tracks_by_duration[ti],
            };
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

impl Tab for SourcesTab {
    fn get_label(&self) -> &str {
        "Sources"
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

                    use crate::app::view::{AggregateSpanView, SpanView};
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
        let order_by = self.order_by;
        let selected_span = self.selected_span;

        use crate::widgets::track::track_content_height;
        let label_height: u16 = 1;

        // Compute per-track heights
        let heights: Vec<u16> = match order_by {
            OrderBy::StartTime => self
                .tracks_start_time
                .iter()
                .map(|views| {
                    track_content_height(views, &self.spans, visible_duration, graph_area.width)
                        + label_height
                })
                .collect(),
            OrderBy::Duration => self
                .tracks_by_duration
                .iter()
                .map(|views| {
                    track_content_height(views, &self.spans, visible_duration, graph_area.width)
                        + label_height
                })
                .collect(),
        };

        // Build TrackInputs
        let track_inputs: Vec<TrackInput> = match order_by {
            OrderBy::StartTime => self
                .tracks_start_time
                .iter_mut()
                .zip(heights.iter().copied())
                .map(|(views, intrinsic_height)| TrackInput {
                    views: views.as_mut_slice(),
                    label: Some("Sources".to_string()),
                    intrinsic_height,
                })
                .collect(),
            OrderBy::Duration => self
                .tracks_by_duration
                .iter_mut()
                .zip(heights.iter().copied())
                .map(|(views, intrinsic_height)| TrackInput {
                    views: views.as_mut_slice(),
                    label: Some("Sources".to_string()),
                    intrinsic_height,
                })
                .collect(),
        };

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
                buf.set_string(vscrollbar_area.x, vscrollbar_area.y + thumb_start + y, "┃", active_style);
            }
        }

        if let Some(si) = self.selected_span {
            if let Some(span) = self.spans.get(si) {
                let parent_duration = span.parent_index.and_then(|pi| self.spans.get(pi)).map(|p| p.duration);
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

impl SourcesTab {
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

