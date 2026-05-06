use clap::Parser;
use ratatui::DefaultTerminal;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, List, ListDirection, Widget};

mod span;
mod unit;
use self::unit::Unit;
use crate::app::unit::get_units;
use crate::cli;

pub struct App {
    units: Vec<Unit>,
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let items: Vec<String> = self.units.iter().map(|t| t.name.clone()).collect();
        let list = List::new(items)
            .block(Block::bordered().title(format!(
                "Units ({}), Spans ({})",
                self.units.len(),
                self.units.iter().map(|u| u.spans.len()).sum::<usize>()
            )))
            .style(Style::new().white())
            .highlight_style(Style::new().bold())
            .repeat_highlight_symbol(true)
            .direction(ListDirection::BottomToTop);
        list.render(area, buf);
    }
}

impl Default for App {
    fn default() -> Self {
        let cli = cli::Cli::parse();
        let units: Vec<Unit> = get_units(&cli.build_dir);
        Self { units }
    }
}

impl App {
    pub fn run(mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        loop {
            terminal.draw(|frame| frame.render_widget(&mut self, frame.area()))?;
            if crossterm::event::read()?.is_key_press() {
                break Ok(());
            }
        }
    }
}
