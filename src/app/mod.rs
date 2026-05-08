use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, MouseEventKind};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::execute;
use ratatui::DefaultTerminal;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

pub mod span;
pub mod unit;
use self::unit::Unit;
use crate::app::unit::{FollowingSpanDirection, get_units};
use crate::cli;
use crate::widgets::flame_graph::Flamegraph;
use crate::widgets::time_range::DurationRange;

pub struct App {
    units: Vec<Unit>,
    zoom: f64,
    start_time: f64,
    selected_indexes: Option<(usize, usize)>, // (unit index, span index)
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let total_duration = self
            .units
            .first()
            .and_then(|u| u.spans.first())
            .map(|s| s.duration)
            .unwrap_or(0.0);
        let visible_duration = total_duration / self.zoom;

        let scrollbar_height = 2;
        let graph_height = area.height.saturating_sub(scrollbar_height);

        let scrollbar_area = Rect::new(area.x, area.y, area.width, scrollbar_height);
        let graph_area = Rect::new(area.x, area.y + scrollbar_height, area.width, graph_height);

        let flamegraph = Flamegraph {
            spans: self
                .units
                .first()
                .map(|u| u.spans.as_slice())
                .unwrap_or(&[]),
            selected_span_index: self.selected_indexes.map(|(_, si)| si),
            total_duration: visible_duration,
            start_time: self.start_time,
        };
        let scrollbar = DurationRange {
            total_duration,
            start: self.start_time,
            visible_duration,
        };
        scrollbar.render(scrollbar_area, buf);

        flamegraph.render(graph_area, buf);
    }
}

impl Default for App {
    fn default() -> Self {
        let cli = cli::Cli::parse();
        let units: Vec<Unit> = get_units(&cli.build_dir);
        Self {
            units,
            zoom: 1.0,
            start_time: 0.0,
            selected_indexes: None,
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
        execute!(std::io::stdout(), EnableMouseCapture)?;
        let result = self.event_loop(terminal);
        execute!(std::io::stdout(), DisableMouseCapture)?;
        result
    }

    fn zoom_around_center(&mut self, factor: f64) {
        let center = self.start_time + self.visible_duration() / 2.0;
        self.zoom = (self.zoom * factor).max(1.0);
        let new_half = self.visible_duration() / 2.0;
        self.start_time = (center - new_half).max(0.0);
        let max_start = (self.total_duration() - self.visible_duration()).max(0.0);
        self.start_time = self.start_time.min(max_start);
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
                        KeyCode::Left => {
                            let (unit_index, span_index) = match self.selected_indexes {
                                Some((ui, si)) => {
                                    let unit = &self.units[ui];
                                    let previous = unit.get_following_span_index(
                                        si,
                                        FollowingSpanDirection::Previous,
                                    );
                                    match previous {
                                        Some(prev_index) => (ui, prev_index),
                                        None => (ui, si),
                                    }
                                }
                                None => (0, 0),
                            };
                            self.selected_indexes = Some((unit_index, span_index));
                        }
                        KeyCode::Right => {
                            let (unit_index, span_index) = match self.selected_indexes {
                                Some((ui, si)) => {
                                    let unit = &self.units[ui];
                                    let next = unit
                                        .get_following_span_index(si, FollowingSpanDirection::Next);
                                    match next {
                                        Some(next_index) => (ui, next_index),
                                        None => (ui, si),
                                    }
                                }
                                None => (0, 0),
                            };
                            self.selected_indexes = Some((unit_index, span_index));
                        }
                        KeyCode::Up => {
                            let (unit_index, span_index) = match self.selected_indexes {
                                Some((ui, si)) => {
                                    let unit = &self.units[ui];
                                    let parent = unit.get_parent_span(&unit.spans[si]);
                                    match parent {
                                        Some(parent_span) => (ui, parent_span.index_in_unit),
                                        None => (ui, si),
                                    }
                                }
                                None => (0, 0),
                            };
                            self.selected_indexes = Some((unit_index, span_index));
                        }
                        KeyCode::Down => {
                            let (unit_index, span_index) = match self.selected_indexes {
                                Some((ui, si)) => {
                                    let unit = &self.units[ui];
                                    let children = unit.get_child_spans(&unit.spans[si]);
                                    if let Some(first_child) = children.first() {
                                        (ui, first_child.index_in_unit)
                                    } else {
                                        (ui, si)
                                    }
                                }
                                None => (0, 0),
                            };
                            self.selected_indexes = Some((unit_index, span_index));
                        }
                        _ => {}
                    }
                }
                Event::Mouse(mouse) => match mouse.kind {
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
