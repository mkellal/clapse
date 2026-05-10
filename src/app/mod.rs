use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEventKind};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use ratatui::DefaultTerminal;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;
use std::collections::HashMap;

pub mod span;
pub mod unit;
use self::unit::Unit;
use crate::app::unit::{FollowingSpanDirection, HorizontalDirection, OrderBy, get_units, schedule_units};
use crate::cli;
use crate::widgets::flamegraph::{FlamegraphWidget, TrackInput};
use crate::widgets::span_details::SpanDetails;
use crate::widgets::time_range::DurationRange;
use crate::widgets::track::UnitEntry;

/// RAII guard that enables mouse capture on creation and disables it on drop.
struct MouseCaptureGuard;

impl MouseCaptureGuard {
    fn enable() -> std::io::Result<Self> {
        execute!(std::io::stdout(), EnableMouseCapture)?;
        Ok(Self)
    }
}

impl Drop for MouseCaptureGuard {
    fn drop(&mut self) {
        let _ = execute!(std::io::stdout(), DisableMouseCapture);
    }
}

pub struct App {
    units: Vec<Unit>,
    /// Track schedule: each entry is a list of unit indices (into `units`).
    tracks: Vec<Vec<usize>>,
    zoom: f64,
    start_time: f64,
    selected_indexes: Option<(usize, usize)>, // (unit index, span index)
    /// Maps terminal cell (col, row) → (unit_index, span_index). Rebuilt every frame.
    cell_span_map: HashMap<(u16, u16), (usize, usize)>,
    order_by: OrderBy,
    /// Vertical scroll offset (rows) into the flamegraph virtual canvas.
    vertical_scroll: u16,
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let total_duration = self
            .units
            .iter()
            .filter_map(|u| u.spans.first())
            .filter(|r| r.duration.is_finite())
            .map(|r| r.start_time + r.duration)
            .fold(0.0f64, f64::max);
        let visible_duration = total_duration / self.zoom;

        let scrollbar_height = 2;
        let details_height: u16 = if let Some((ui, si)) = self.selected_indexes {
            self.units[ui]
                .spans
                .get(si)
                .map(|span| {
                    let parent_duration = span.parent_index
                        .and_then(|pi| self.units[ui].spans.get(pi))
                        .map(|p| p.duration);
                    SpanDetails { span, parent_duration, total_duration }.required_height(area.width)
                })
                .unwrap_or(0)
        } else {
            0
        };
        let graph_height = area
            .height
            .saturating_sub(scrollbar_height + details_height);

        let scrollbar_area = Rect::new(area.x, area.y, area.width, scrollbar_height);
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
        let graph_area = Rect::new(area.x, area.y + scrollbar_height, area.width, graph_height);
        let order_by = self.order_by;
        let label_height: u16 = 1;
        let track_inputs: Vec<TrackInput> = self
            .tracks
            .iter()
            .enumerate()
            .map(|(track_idx, track_units)| {
                use crate::widgets::track::track_content_height;
                let intrinsic_height =
                    track_content_height(track_units, &self.units) + label_height;
                let label = Some(format!("Track {}", track_idx));
                let units_entries: Vec<UnitEntry> = track_units
                    .iter()
                    .filter_map(|&ui| {
                        let unit = self.units.get_mut(ui)?;
                        let views: &[crate::app::unit::SpanView] = match order_by {
                            OrderBy::StartTime => unit.views_by_start_time.as_slice(),
                            OrderBy::Duration => unit.views_by_duration.as_slice(),
                        };
                        let selected_span_index = self
                            .selected_indexes
                            .filter(|&(suu, _)| suu == ui)
                            .map(|(_, si)| si);
                        // Safety: we hold a unique &mut self, so we can extend the
                        // lifetime of views to match the unit borrow.
                        let views: &'_ [crate::app::unit::SpanView] =
                            unsafe { std::mem::transmute(views) };
                        let spans: &'_ mut [crate::app::span::Span] =
                            unsafe { std::mem::transmute(unit.spans.as_mut_slice()) };
                        Some(UnitEntry {
                            unit_index: ui,
                            spans,
                            views,
                            selected_span_index,
                        })
                    })
                    .collect();
                TrackInput { label, units: units_entries, intrinsic_height }
            })
            .collect();
        FlamegraphWidget {
            tracks: track_inputs,
            total_duration: visible_duration,
            start_time,
            scroll_offset: self.vertical_scroll,
            cell_map: &mut self.cell_span_map,
        }
        .render(graph_area, buf);

        if let Some((ui, si)) = self.selected_indexes {
            if let Some(span) = self.units[ui].spans.get(si) {
                let parent_duration = span.parent_index
                    .and_then(|pi| self.units[ui].spans.get(pi))
                    .map(|p| p.duration);
                SpanDetails { span, parent_duration, total_duration }.render(details_area, buf);
            }
        }
    }
}

impl Default for App {
    fn default() -> Self {
        let cli = cli::Cli::parse();
        let units: Vec<Unit> = get_units(&cli.build_dir);
        let mut tracks = schedule_units(&units);
        // Sort tracks: longest total duration (sum of root span durations) first.
        tracks.sort_by(|a, b| {
            let dur = |track: &Vec<usize>| -> f64 {
                track
                    .iter()
                    .filter_map(|&ui| units.get(ui))
                    .filter_map(|u| u.spans.first())
                    .map(|s| s.duration)
                    .sum()
            };
            dur(b).partial_cmp(&dur(a)).unwrap_or(std::cmp::Ordering::Equal)
        });
        Self {
            units,
            tracks,
            zoom: 1.0,
            start_time: 0.0,
            selected_indexes: None,
            cell_span_map: HashMap::new(),
            order_by: OrderBy::StartTime,
            vertical_scroll: 0,
        }
    }
}

impl App {
    fn total_duration(&self) -> f64 {
        self.units
            .iter()
            .filter_map(|u| u.spans.first())
            .filter(|r| r.duration.is_finite())
            .map(|r| r.start_time + r.duration)
            .fold(0.0f64, f64::max)
    }

    fn visible_duration(&self) -> f64 {
        self.total_duration() / self.zoom
    }

    pub fn run(mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        let _mouse_guard = MouseCaptureGuard::enable()?;
        self.event_loop(terminal)
    }

    fn move_selection(&mut self, direction: FollowingSpanDirection) {
        let (ui, si) = match self.selected_indexes {
            Some(idx) => idx,
            None => {
                self.selected_indexes = Some((0, 0));
                return;
            }
        };
        match direction {
            FollowingSpanDirection::Next | FollowingSpanDirection::Previous => {
                let horiz = match direction {
                    FollowingSpanDirection::Next => HorizontalDirection::Next,
                    _ => HorizontalDirection::Previous,
                };
                let is_root = self.units[ui].spans[si].parent_index.is_none();
                if is_root {
                    // Move between visible unit roots within the same track.
                    if let Some((ti, _)) = self.track_of_unit(ui) {
                        let visible: Vec<usize> = self.tracks[ti]
                            .iter()
                            .copied()
                            .filter(|&idx| {
                                self.units[idx]
                                    .spans
                                    .first()
                                    .map(|s| s.was_displayed)
                                    .unwrap_or(false)
                            })
                            .collect();
                        if let Some(pos) = visible.iter().position(|&idx| idx == ui) {
                            let shift: isize = match horiz {
                                HorizontalDirection::Next => 1,
                                HorizontalDirection::Previous => -1,
                            };
                            let new_pos = (pos as isize + shift) as usize;
                            if let Some(&new_ui) = visible.get(new_pos) {
                                self.selected_indexes = Some((new_ui, 0));
                            }
                        }
                    }
                } else {
                    let unit = &self.units[ui];
                    let views = unit.views(self.order_by);
                    if let Some(new_si) = unit.get_following_span_index(si, horiz, views) {
                        self.selected_indexes = Some((ui, new_si));
                    }
                }
            }
            FollowingSpanDirection::Parent => {
                let unit = &self.units[ui];
                if let Some(new_si) = unit.get_parent_span(&unit.spans[si]).map(|s| s.index_in_unit) {
                    self.selected_indexes = Some((ui, new_si));
                }
            }
            FollowingSpanDirection::Child => {
                let unit = &self.units[ui];
                let views = unit.views(self.order_by);
                if let Some(new_si) = views
                    .iter()
                    .find(|e| {
                        unit.spans[e.span_index].parent_index == Some(si)
                            && unit.spans[e.span_index].was_displayed
                    })
                    .map(|e| e.span_index)
                {
                    self.selected_indexes = Some((ui, new_si));
                }
            }
        }
    }

    /// Returns `(track_index, position_in_track)` for the given unit index.
    fn track_of_unit(&self, unit_index: usize) -> Option<(usize, usize)> {
        self.tracks.iter().enumerate().find_map(|(ti, units)| {
            units
                .iter()
                .position(|&ui| ui == unit_index)
                .map(|pos| (ti, pos))
        })
    }

    /// Move the selection to the first unit root of the next/previous track.
    fn switch_track(&mut self, dir: HorizontalDirection) {
        if self.tracks.is_empty() {
            return;
        }
        let current_ti = self
            .selected_indexes
            .and_then(|(ui, _)| self.track_of_unit(ui))
            .map(|(ti, _)| ti)
            .unwrap_or(0);
        let n = self.tracks.len();
        let new_ti = match dir {
            HorizontalDirection::Next => (current_ti + 1) % n,
            HorizontalDirection::Previous => (current_ti + n - 1) % n,
        };
        // First visible unit in the target track.
        let first_visible = self.tracks[new_ti]
            .iter()
            .copied()
            .find(|&idx| {
                self.units[idx]
                    .spans
                    .first()
                    .map(|s| s.was_displayed)
                    .unwrap_or(false)
            })
            .or_else(|| self.tracks[new_ti].first().copied());
        if let Some(new_ui) = first_visible {
            self.selected_indexes = Some((new_ui, 0));
        }
    }

    fn zoom_around_center(&mut self, factor: f64) {
        let center = self.start_time + self.visible_duration() / 2.0;
        self.zoom = (self.zoom * factor).max(1.0);
        let new_half = self.visible_duration() / 2.0;
        self.start_time = (center - new_half).max(0.0);
        let max_start = (self.total_duration() - self.visible_duration()).max(0.0);
        self.start_time = self.start_time.min(max_start);
    }

    /// Zoom so the selected span occupies ~60% of the viewport and center on it.
    fn zoom_to_selected(&mut self) {
        let (ui, si) = match self.selected_indexes {
            Some(idx) => idx,
            None => return,
        };
        let span_duration = match self.units[ui].spans.get(si) {
            Some(s) => s.duration,
            None => return,
        };
        let effective_start = match self.units[ui]
            .views(self.order_by)
            .iter()
            .find(|e| e.span_index == si)
        {
            Some(e) => e.effective_start,
            None => return,
        };
        let span_center = effective_start + span_duration / 2.0;

        let new_visible = span_duration / 0.75;
        let total = self.total_duration();
        self.zoom = (total / new_visible).max(1.0);
        let actual_visible = total / self.zoom;
        self.start_time = (span_center - actual_visible / 2.0)
            .max(0.0)
            .min((total - actual_visible).max(0.0));
    }

    fn event_loop(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        loop {
            let app = &mut *self;
            terminal.draw(|frame| frame.render_widget(&mut *app, frame.area()))?;
            match event::read()? {
                Event::Key(key) if key.kind == KeyEventKind::Press => {
                    let ctrl = key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::CONTROL);
                    let shift = key
                        .modifiers
                        .contains(crossterm::event::KeyModifiers::SHIFT);
                    // Ctrl+Up/Down = precise zoom (×1.1), PageUp/PageDown = fast zoom (×2)
                    // Ctrl+Shift+Left/Right = precise pan (5%), Ctrl+Left/Right = fast pan (25%)
                    let pan_factor = if shift { 0.05 } else { 0.25 };
                    match key.code {
                        KeyCode::Char('q') => break Ok(()),
                        KeyCode::Char('c') if ctrl => {
                            break Ok(());
                        }
                        // Ctrl+Up/Down = precise zoom
                        KeyCode::Up if ctrl => {
                            self.zoom_around_center(1.1);
                        }
                        KeyCode::Down if ctrl => {
                            self.zoom_around_center(1.0 / 1.1);
                            if self.zoom < 1.0 {
                                self.zoom = 1.0;
                                self.start_time = 0.0;
                            }
                        }
                        // PageUp/PageDown = fast zoom
                        KeyCode::PageUp => {
                            self.zoom_around_center(2.0);
                        }
                        KeyCode::PageDown => {
                            self.zoom_around_center(0.5);
                            if self.zoom < 1.0 {
                                self.zoom = 1.0;
                                self.start_time = 0.0;
                            }
                        }
                        // Ctrl+Left/Right = pan
                        KeyCode::Left if ctrl => {
                            let step = self.visible_duration() * pan_factor;
                            self.start_time = (self.start_time - step).max(0.0);
                        }
                        KeyCode::Right if ctrl => {
                            let step = self.visible_duration() * pan_factor;
                            let max_start =
                                (self.total_duration() - self.visible_duration()).max(0.0);
                            self.start_time = (self.start_time + step).min(max_start);
                        }

                        // span selection
                        KeyCode::Left => self.move_selection(FollowingSpanDirection::Previous),
                        KeyCode::Right => self.move_selection(FollowingSpanDirection::Next),
                        KeyCode::Up => self.move_selection(FollowingSpanDirection::Parent),
                        KeyCode::Down => self.move_selection(FollowingSpanDirection::Child),
                        KeyCode::Char(' ') if ctrl => {
                            self.zoom = 1.0;
                            self.start_time = 0.0;
                        }
                        KeyCode::Char(' ') => self.zoom_to_selected(),
                        KeyCode::Esc => self.selected_indexes = None,
                        KeyCode::Tab => self.switch_track(HorizontalDirection::Next),
                        KeyCode::BackTab => self.switch_track(HorizontalDirection::Previous),
                        KeyCode::Char('s') => {
                            self.order_by = match self.order_by {
                                OrderBy::StartTime => OrderBy::Duration,
                                OrderBy::Duration => OrderBy::StartTime,
                            };
                        }
                        _ => {}
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::Down(_) => {
                        let coord = (mouse.column, mouse.row);
                        if let Some(&(ui, si)) = self.cell_span_map.get(&coord) {
                            self.selected_indexes = Some((ui, si));
                        }
                    }
                    MouseEventKind::ScrollUp | MouseEventKind::ScrollLeft => {
                        self.vertical_scroll = self.vertical_scroll.saturating_sub(3);
                    }
                    MouseEventKind::ScrollDown | MouseEventKind::ScrollRight => {
                        self.vertical_scroll = self.vertical_scroll.saturating_add(3);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}
