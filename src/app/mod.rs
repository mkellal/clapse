use clap::Parser;
use ratatui::DefaultTerminal;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::widgets::{Block, List, ListDirection, Widget};
use std::path::PathBuf;

use crate::cli;
use crate::traces::file::get_trace_files;

pub struct App {
    trace_files: Vec<PathBuf>,
}

impl Widget for &mut App {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let items: Vec<String> = self
            .trace_files
            .iter()
            .map(|p| p.to_str().unwrap_or("<...>").to_string())
            .collect();
        let list = List::new(items)
            .block(Block::bordered().title(format!("Trace Files ({})", self.trace_files.len())))
            .style(Style::new().white())
            .highlight_style(Style::new().bold())
            // .highlight_symbol(">>")
            .repeat_highlight_symbol(true)
            .direction(ListDirection::BottomToTop);
        list.render(area, buf);
    }
}

impl Default for App {
    fn default() -> Self {
        let cli = cli::Cli::parse();
        let trace_files = get_trace_files(&cli.build_dir);
        Self { trace_files }
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
