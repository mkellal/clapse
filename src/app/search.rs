use crossterm::event::{KeyCode, KeyEvent};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};

use crate::app::tabs::Tab;

pub struct SearchState {
    pub query: String,
    pub visible: bool,
    pub locked: bool,
}

impl Default for SearchState {
    fn default() -> Self {
        Self { query: String::new(), visible: false, locked: false }
    }
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

        let text_color = if self.locked { Color::Black } else { Color::LightGreen };

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
            Style::default()
                .fg(text_color)
                .bg(if self.locked { Color::LightGreen } else { Color::Reset }),
        );
    }
}
