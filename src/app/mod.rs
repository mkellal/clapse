use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind};
use ratatui::DefaultTerminal;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

pub mod span;
pub mod unit;
use self::unit::Unit;
use crate::app::unit::get_units;
use crate::cli;
use crate::widgets::duration_range::{tick_interval, DurationRange};
use crate::widgets::flame_graph::Flamegraph;

pub struct App {
    units: Vec<Unit>,
    zoom: f64,
    start_time: f64,
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let total_duration = self.units.first().map(|u| u.total_duration).unwrap_or(0.0);
        let visible_duration = total_duration / self.zoom;

        let scrollbar_height = 2;
        let graph_height = area.height.saturating_sub(scrollbar_height);

        let graph_area = Rect::new(area.x, area.y, area.width, graph_height);
        let scrollbar_area = Rect::new(area.x, area.y + graph_height, area.width, scrollbar_height);

        let flamegraph = Flamegraph {
            spans: self
                .units
                .first()
                .map(|u| u.spans.as_slice())
                .unwrap_or(&[]),
            total_duration: visible_duration,
            start_time: self.start_time,
        };
        flamegraph.render(graph_area, buf);

        let scrollbar = DurationRange {
            total_duration,
            start: self.start_time,
            visible_duration,
        };
        scrollbar.render(scrollbar_area, buf);
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
        }
    }
}

impl App {
    fn total_duration(&self) -> f64 {
        self.units.first().map(|u| u.total_duration).unwrap_or(0.0)
    }

    fn visible_duration(&self) -> f64 {
        self.total_duration() / self.zoom
    }

    pub fn run(mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        loop {
            terminal.draw(|frame| frame.render_widget(&mut self, frame.area()))?;
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                        KeyCode::Char('+') | KeyCode::Char('=') => {
                            let center = self.start_time + self.visible_duration() / 2.0;
                            self.zoom *= 1.25;
                            let new_half = self.visible_duration() / 2.0;
                            self.start_time = (center - new_half).max(0.0);
                            let max_start = (self.total_duration() - self.visible_duration()).max(0.0);
                            self.start_time = self.start_time.min(max_start);
                        }
                        KeyCode::Char('-') => {
                            self.zoom /= 1.25;
                            if self.zoom < 1.0 {
                                self.zoom = 1.0;
                                self.start_time = 0.0;
                            } else {
                                let max_start = (self.total_duration() - self.visible_duration()).max(0.0);
                                self.start_time = self.start_time.min(max_start);
                            }
                        }
                        KeyCode::Left => {
                            let interval = tick_interval(self.visible_duration());
                            let k = (self.start_time / interval).round() as i64;
                            self.start_time = ((k - 1) as f64 * interval).max(0.0);
                        }
                        KeyCode::Right => {
                            let interval = tick_interval(self.visible_duration());
                            let k = (self.start_time / interval).round() as i64;
                            let max_start = (self.total_duration() - self.visible_duration()).max(0.0);
                            self.start_time = ((k + 1) as f64 * interval).min(max_start);
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
