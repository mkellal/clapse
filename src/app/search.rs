use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Widget};

pub struct SearchBox<'a> {
    pub query: &'a str,
    pub match_count: usize,
    pub current_match: Option<usize>,
    pub has_query: bool,
    pub locked: bool,
}

/// Build a styled `<key> desc` span pair.
fn key_hint(key: &str, desc: &str) -> Vec<Span<'static>> {
    vec![
        Span::styled("<", Color::DarkGray),
        Span::styled(key.to_string(), Color::Red),
        Span::styled("> ", Color::DarkGray),
        Span::raw(format!("{}  ", desc)),
    ]
}

impl<'a> Widget for SearchBox<'a> {
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
            Style::default().fg(text_color).bg(if self.locked { Color::LightGreen } else { Color::Reset }),
        );
    }
}
