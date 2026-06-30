use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use crate::app::tabs::Tab;

pub struct SourcesTab;

impl SourcesTab {
    pub fn new() -> Self {
        Self
    }
}

impl Tab for SourcesTab {
    fn get_label(&self) -> &str {
        "Sources"
    }

    fn handle_key_event(&mut self, _key: crossterm::event::KeyEvent) -> bool {
        false
    }

    fn handle_mouse_event(&mut self, _mouse: crossterm::event::MouseEvent) {}

    fn render(&mut self, _area: Rect, _buf: &mut Buffer) {
        // Empty for now
    }
}
