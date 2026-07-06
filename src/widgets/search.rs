use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};

use crate::app::tabs::Tab;

#[derive(Default)]
pub struct SearchState {
    pub query: String,
    pub visible: bool,
    pub locked: bool,
}

impl SearchState {
    pub fn open(&mut self, tab: &mut dyn Tab) {
        self.visible = true;
        self.locked = false;
        self.query.clear();
        tab.set_search_query(String::new());
    }

    /// Returns `true` if the key was consumed.
    pub fn handle_key(&mut self, key: KeyEvent, tab: &mut dyn Tab) -> bool {
        if !self.visible {
            return false;
        }

        if self.locked {
            match key.code {
                KeyCode::Esc => {
                    self.locked = false;
                    return true;
                }
                KeyCode::Char('n') => {
                    tab.select_next_match();
                    return true;
                }
                KeyCode::Char('p') => {
                    tab.select_previous_match();
                    return true;
                }
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::Esc => {
                    self.visible = false;
                    self.locked = false;
                    self.query.clear();
                    tab.set_search_query(String::new());
                    return true;
                }
                KeyCode::Enter => {
                    self.locked = true;
                    return true;
                }
                KeyCode::Char(c) => {
                    self.query.push(c);
                    tab.set_search_query(self.query.clone());
                    return true;
                }
                KeyCode::Backspace => {
                    self.query.pop();
                    tab.set_search_query(self.query.clone());
                    return true;
                }
                _ => {}
            }
        }

        // Non-search keys fall through to tab handler.
        false
    }

    /// Render the search box. `area` should be the bottom 3 rows of the terminal.
    pub fn render(&self, area: Rect, buf: &mut Buffer, tab: &dyn Tab) {
        SearchBoxWidget {
            query: &self.query,
            match_count: tab.match_count(),
            current_match: tab.current_match_index(),
            has_query: !self.query.is_empty(),
            locked: self.locked,
        }
        .render(area, buf);
    }
}

// ── Internal widget ──────────────────────────────────────────────────────────

struct SearchBoxWidget<'a> {
    query: &'a str,
    match_count: usize,
    current_match: Option<usize>,
    has_query: bool,
    locked: bool,
}

fn key_hint(key: &str, desc: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled("<", Color::DarkGray),
        Span::styled(key.to_string(), Color::Red),
        Span::styled("> ", Color::DarkGray),
        Span::raw(format!("{}  ", desc)),
    ]
}

impl Widget for SearchBoxWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut title_spans: Vec<Span> = vec![Span::raw(" Search ")];

        if self.has_query && self.match_count == 0 {
            title_spans.push(Span::styled("(no matches) ", Color::Red));
        } else if self.match_count > 0 {
            if let Some(n) = self.current_match {
                title_spans.push(Span::raw(format!("({} of {}) ", n + 1, self.match_count)));
            } else {
                title_spans.push(Span::raw(format!("({} matches) ", self.match_count)));
            }
        }

        if self.locked {
            title_spans.extend(key_hint("n", "next"));
            title_spans.extend(key_hint("p", "prev"));
            title_spans.extend(key_hint("Esc", "edit"));
        } else {
            title_spans.extend(key_hint("\u{23ce}", "select match"));
            title_spans.extend(key_hint("Esc", "close"));
        }

        let title = Line::from(title_spans);

        let text_color = if self.locked {
            Color::Black
        } else {
            Color::LightGreen
        };

        let block = if self.locked {
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Black).bg(Color::LightGreen))
                .style(Style::default().bg(Color::LightGreen))
        } else {
            Block::default()
                .title(title)
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::LightGreen))
        };

        let inner = block.inner(area);
        block.render(area, buf);

        let cursor = if self.locked { "" } else { "█" };
        let text = format!("{}{}", self.query, cursor);
        buf.set_string(
            inner.x,
            inner.y,
            &text,
            Style::default().fg(text_color).bg(if self.locked {
                Color::LightGreen
            } else {
                Color::Reset
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::tabs::Tab;

    // ── Mock Tab ──

    struct MockTab {
        search_query: String,
        next_match_calls: usize,
        prev_match_calls: usize,
        match_count_val: usize,
        current_match_val: Option<usize>,
    }

    impl MockTab {
        fn new() -> Self {
            Self {
                search_query: String::new(),
                next_match_calls: 0,
                prev_match_calls: 0,
                match_count_val: 0,
                current_match_val: None,
            }
        }
    }

    impl Tab for MockTab {
        fn get_label(&self) -> &str {
            "mock"
        }
        fn handle_key_event(&mut self, _key: crossterm::event::KeyEvent) -> bool {
            false
        }
        fn handle_mouse_event(&mut self, _mouse: crossterm::event::MouseEvent) {}
        fn render(&mut self, _area: Rect, _buf: &mut Buffer) {}
        fn get_help(&self) -> Vec<(&str, &str)> {
            vec![]
        }
        fn set_search_query(&mut self, query: String) {
            self.search_query = query;
        }
        fn select_next_match(&mut self) {
            self.next_match_calls += 1;
        }
        fn select_previous_match(&mut self) {
            self.prev_match_calls += 1;
        }
        fn match_count(&self) -> usize {
            self.match_count_val
        }
        fn current_match_index(&self) -> Option<usize> {
            self.current_match_val
        }
    }

    // ── SearchState::handle_key ──

    #[test]
    fn test_handle_key_not_visible_returns_false() {
        let mut state = SearchState::default();
        let mut tab = MockTab::new();
        let key = KeyEvent::new(KeyCode::Char('s'), crossterm::event::KeyModifiers::NONE);
        assert!(!state.handle_key(key, &mut tab));
    }

    #[test]
    fn test_handle_key_esc_closes() {
        let mut state = SearchState {
            visible: true,
            query: "test".into(),
            locked: false,
        };
        let mut tab = MockTab::new();
        let key = KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE);
        let consumed = state.handle_key(key, &mut tab);
        assert!(consumed);
        assert!(!state.visible);
        assert!(state.query.is_empty());
        assert!(!state.locked);
    }

    #[test]
    fn test_handle_key_char_appends() {
        let mut state = SearchState {
            visible: true,
            query: String::new(),
            locked: false,
        };
        let mut tab = MockTab::new();
        let key = KeyEvent::new(KeyCode::Char('a'), crossterm::event::KeyModifiers::NONE);
        let consumed = state.handle_key(key, &mut tab);
        assert!(consumed);
        assert_eq!(state.query, "a");
        assert_eq!(tab.search_query, "a");
    }

    #[test]
    fn test_handle_key_backspace() {
        let mut state = SearchState {
            visible: true,
            query: "ab".into(),
            locked: false,
        };
        let mut tab = MockTab::new();
        let key = KeyEvent::new(KeyCode::Backspace, crossterm::event::KeyModifiers::NONE);
        let consumed = state.handle_key(key, &mut tab);
        assert!(consumed);
        assert_eq!(state.query, "a");
        assert_eq!(tab.search_query, "a");
    }

    #[test]
    fn test_handle_key_enter_locks() {
        let mut state = SearchState {
            visible: true,
            query: "foo".into(),
            locked: false,
        };
        let mut tab = MockTab::new();
        let key = KeyEvent::new(KeyCode::Enter, crossterm::event::KeyModifiers::NONE);
        let consumed = state.handle_key(key, &mut tab);
        assert!(consumed);
        assert!(state.locked);
    }

    #[test]
    fn test_handle_key_locked_esc_unlocks() {
        let mut state = SearchState {
            visible: true,
            query: "foo".into(),
            locked: true,
        };
        let mut tab = MockTab::new();
        let key = KeyEvent::new(KeyCode::Esc, crossterm::event::KeyModifiers::NONE);
        let consumed = state.handle_key(key, &mut tab);
        assert!(consumed);
        assert!(!state.locked);
        assert_eq!(state.query, "foo"); // query preserved
    }

    #[test]
    fn test_handle_key_locked_n_selects_next() {
        let mut state = SearchState {
            visible: true,
            query: "foo".into(),
            locked: true,
        };
        let mut tab = MockTab::new();
        let key = KeyEvent::new(KeyCode::Char('n'), crossterm::event::KeyModifiers::NONE);
        let consumed = state.handle_key(key, &mut tab);
        assert!(consumed);
        assert_eq!(tab.next_match_calls, 1);
    }

    #[test]
    fn test_handle_key_locked_p_selects_prev() {
        let mut state = SearchState {
            visible: true,
            query: "foo".into(),
            locked: true,
        };
        let mut tab = MockTab::new();
        let key = KeyEvent::new(KeyCode::Char('p'), crossterm::event::KeyModifiers::NONE);
        let consumed = state.handle_key(key, &mut tab);
        assert!(consumed);
        assert_eq!(tab.prev_match_calls, 1);
    }

    // ── SearchBoxWidget render ──

    #[test]
    fn test_search_box_empty_query() {
        let area = Rect::new(0, 0, 60, 3);
        let mut buf = Buffer::empty(area);
        let widget = SearchBoxWidget {
            query: "",
            match_count: 0,
            current_match: None,
            has_query: false,
            locked: false,
        };
        widget.render(area, &mut buf);

        // Cursor "█" should be present
        let content: String = (0..area.width)
            .filter_map(|x| buf.cell((x, 1)).map(|c| c.symbol()))
            .collect();
        assert!(
            content.contains('█'),
            "cursor should be visible, got: {content}"
        );
    }

    #[test]
    fn test_search_box_with_matches() {
        let area = Rect::new(0, 0, 60, 3);
        let mut buf = Buffer::empty(area);
        let widget = SearchBoxWidget {
            query: "foo",
            match_count: 5,
            current_match: Some(1), // 0-indexed, displayed as "2 of 5"
            has_query: true,
            locked: true,
        };
        widget.render(area, &mut buf);

        // Title line (y=0) should contain "2 of 5"
        let title: String = (0..area.width)
            .filter_map(|x| buf.cell((x, 0)).map(|c| c.symbol()))
            .collect();
        assert!(
            title.contains("2 of 5"),
            "title should show 2 of 5, got: {title}"
        );
    }

    #[test]
    fn test_search_box_locked_shows_nav_hints() {
        let area = Rect::new(0, 0, 80, 3);
        let mut buf = Buffer::empty(area);
        let widget = SearchBoxWidget {
            query: "bar",
            match_count: 3,
            current_match: None,
            has_query: true,
            locked: true,
        };
        widget.render(area, &mut buf);

        let title: String = (0..area.width)
            .filter_map(|x| buf.cell((x, 0)).map(|c| c.symbol()))
            .collect();
        assert!(title.contains("next"), "should show next hint");
        assert!(title.contains("prev"), "should show prev hint");
    }
}
