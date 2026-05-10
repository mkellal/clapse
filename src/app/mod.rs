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
use crate::widgets::span_details::SpanDetails;
use crate::widgets::time_range::DurationRange;
use crate::widgets::track::{TrackWidget, UnitEntry, thread_content_height};

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
    /// Thread schedule: each entry is a list of unit indices (into `units`).
    threads: Vec<Vec<usize>>,
    zoom: f64,
    start_time: f64,
    selected_indexes: Option<(usize, usize)>, // (unit index, span index)
    /// Maps terminal cell (col, row) → (unit_index, span_index). Rebuilt every frame.
    cell_span_map: HashMap<(u16, u16), (usize, usize)>,
    order_by: OrderBy,
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
        let num_tracks = self.threads.len().max(1) as u16;
        // Compute the intrinsic height of each track (content rows + label row).
        let label_height: u16 = 1;
        let track_heights: Vec<u16> = self
            .threads
            .iter()
            .map(|t| thread_content_height(t, &self.units) + label_height)
            .collect();
        let total_tracks_height: u16 = track_heights.iter().copied().sum();
        // Scale down uniformly if tracks don't fit the available graph area.
        let scale = if total_tracks_height > 0 && total_tracks_height > graph_height {
            graph_height as f64 / total_tracks_height as f64
        } else {
            1.0
        };
        let _ = num_tracks; // used indirectly via track_heights

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
        let mut track_y = area.y + scrollbar_height;
        for (track_idx, thread_units) in self.threads.iter().enumerate() {
            let intrinsic = *track_heights.get(track_idx).unwrap_or(&1);
            let this_height = ((intrinsic as f64 * scale).round() as u16).max(1);
            // Clamp so we don't exceed the graph area.
            let remaining = (area.y + scrollbar_height + graph_height).saturating_sub(track_y);
            let this_height = this_height.min(remaining);
            if this_height == 0 {
                break;
            }
            let track_area = Rect::new(area.x, track_y, area.width, this_height);
            let label = format!("Thread {}", track_idx);
            let order_by = self.order_by;
            let units_entries: Vec<UnitEntry> = thread_units
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
            TrackWidget {
                label: Some(label.as_str()),
                units: units_entries,
                total_duration: visible_duration,
                start_time,
                cell_map: &mut self.cell_span_map,
            }
            .render(track_area, buf);
            track_y += this_height;
        }

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
        let threads = schedule_units(&units);
        Self {
            units,
            threads,
            zoom: 1.0,
            start_time: 0.0,
            selected_indexes: None,
            cell_span_map: HashMap::new(),
            order_by: OrderBy::StartTime,
        }
    }
}

impl App {
    fn total_duration(&self) -> f64 {
        self.units
            .first()
            .and_then(|u| u.spans.first())
            .map(|s| s.duration)
            .unwrap_or(0.0)
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
        let unit = &self.units[ui];
        let views = unit.views(self.order_by);
        let new_si = match direction {
            FollowingSpanDirection::Next | FollowingSpanDirection::Previous => {
                let horiz = match direction {
                    FollowingSpanDirection::Next => HorizontalDirection::Next,
                    _ => HorizontalDirection::Previous,
                };
                unit.get_following_span_index(si, horiz, views)
            }
            FollowingSpanDirection::Parent => unit
                .get_parent_span(&unit.spans[si])
                .map(|s| s.index_in_unit),
            FollowingSpanDirection::Child => views
                .iter()
                .find(|e| {
                    unit.spans[e.span_index].parent_index == Some(si)
                        && unit.spans[e.span_index].was_displayed
                })
                .map(|e| e.span_index),
        };
        if let Some(next_si) = new_si {
            self.selected_indexes = Some((ui, next_si));
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
                        let step = self.visible_duration() * 0.1;
                        self.start_time = (self.start_time - step).max(0.0);
                    }
                    // Plain scroll down / right → pan forward
                    MouseEventKind::ScrollDown | MouseEventKind::ScrollRight => {
                        let step = self.visible_duration() * 0.1;
                        let max_start = (self.total_duration() - self.visible_duration()).max(0.0);
                        self.start_time = (self.start_time + step).min(max_start);
                    }
                    _ => {}
                },
                _ => {}
            }
        }
    }
}
