use ratatui::{buffer::Buffer, layout::Rect};

pub mod flamegraph;
pub mod sources;
pub mod templates;

pub trait Tab {
    fn get_label(&self) -> &str;
    fn handle_key_event(&mut self, key: crossterm::event::KeyEvent) -> bool;
    fn handle_mouse_event(&mut self, mouse: crossterm::event::MouseEvent);

    fn render(&mut self, area: Rect, buf: &mut Buffer);

    fn get_help(&self) -> Vec<(&str, &str)>;
}
