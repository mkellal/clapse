use crossterm::event::{KeyCode, KeyEvent, MouseEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use std::rc::Rc;

use crate::app::span::Span;
use crate::app::tabs::Tab;
use crate::app::view::OrderBy;
use crate::widgets::flamegraph::FlamegraphWidget;

pub struct FlameGraphTab {
    flamegraph: FlamegraphWidget,
}

impl FlameGraphTab {
    pub fn new(raw_spans: Rc<[Span]>) -> Self {
        Self {
            flamegraph: FlamegraphWidget::new(raw_spans, None, OrderBy::StartTime, None, true),
        }
    }
}

impl Tab for FlameGraphTab {
    fn get_label(&self) -> &str {
        "Flamegraph"
    }

    fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        // Extra key: toggle sort mode
        if key.code == KeyCode::Char('m') {
            self.flamegraph.toggle_sort_mode();
            return false;
        }
        self.flamegraph.handle_key_event(key)
    }

    fn handle_mouse_event(&mut self, mouse: MouseEvent) {
        self.flamegraph.handle_mouse_event(mouse);
    }

    fn render(&mut self, area: Rect, buf: &mut Buffer) {
        self.flamegraph.render(area, buf);
    }

    fn get_help(&self) -> Vec<(&str, &str)> {
        let mut help = self.flamegraph.get_help();
        help.push(("m", "Toggle sort mode"));
        help
    }

    fn set_search_query(&mut self, query: String) {
        self.flamegraph.set_search_query(query);
    }

    fn select_next_match(&mut self) {
        self.flamegraph.select_next_match();
    }

    fn select_previous_match(&mut self) {
        self.flamegraph.select_previous_match();
    }

    fn match_count(&self) -> usize {
        self.flamegraph.match_count()
    }

    fn current_match_index(&self) -> Option<usize> {
        self.flamegraph.current_match_index()
    }
}
