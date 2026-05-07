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
use crate::widgets::flame_graph::Flamegraph;

pub struct App {
    units: Vec<Unit>,
    zoom: f64,
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let total_duration = self.units.first().map(|u| u.total_duration).unwrap_or(0.0);
        let visible_duration = total_duration / self.zoom;
        let flamegraph = Flamegraph {
            spans: &self
                .units
                .first()
                .map(|u| u.spans.as_slice())
                .unwrap_or(&[]),
            total_duration: visible_duration,
        };
        flamegraph.render(area, buf);
    }
}

impl Default for App {
    fn default() -> Self {
        let cli = cli::Cli::parse();
        let units: Vec<Unit> = get_units(&cli.build_dir);
        Self { units, zoom: 1.0 }
    }
}

impl App {
    pub fn run(mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        loop {
            terminal.draw(|frame| frame.render_widget(&mut self, frame.area()))?;
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                        KeyCode::Char('+') | KeyCode::Char('=') => {
                            self.zoom *= 1.25;
                        }
                        KeyCode::Char('-') => {
                            self.zoom /= 1.25;
                            if self.zoom < 0.25 {
                                self.zoom = 0.25;
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
    }
}
