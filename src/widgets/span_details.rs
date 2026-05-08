use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Widget},
};

use crate::app::span::{Span, SpanType};
use crate::widgets::time_range::format_time;

pub struct SpanDetails<'a> {
    pub span: &'a Span,
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            current.push_str(word);
        } else if current.chars().count() + 1 + word.chars().count() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            lines.push(current.clone());
            current = word.to_string();
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    lines
}

impl SpanType {
    fn label(&self) -> &'static str {
        match self {
            SpanType::Unit => "Unit",
            SpanType::Source => "Source",
            SpanType::Template => "Template",
            SpanType::Class => "Class",
            SpanType::Task => "Task",
        }
    }

    pub fn badge_colors(&self) -> (Color, Color) {
        (Color::Black, self.base_color())
    }
}

impl<'a> SpanDetails<'a> {
    /// Compute the total height (including borders) needed to display all content at the given width.
    pub fn required_height(&self, area_width: u16) -> u16 {
        if area_width < 4 {
            return 3;
        }
        let inner_width = (area_width - 2) as usize;
        let label_lines = wrap_text(&self.span.label, inner_width).len() as u16;
        let detail_lines = self.span.details.as_deref()
            .map(|d| wrap_text(d, inner_width).len() as u16)
            .unwrap_or(0);
        // 2 borders + 1 row (badge + pills) + label rows + detail rows
        2 + 1 + label_lines + detail_lines
    }
}

impl<'a> Widget for SpanDetails<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 3 || area.width == 0 {
            return;
        }

        let border_style = Style::default().fg(Color::Rgb(88, 91, 112));
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .render(area, buf);

        let inner_x = area.x + 1;
        let inner_right = area.right().saturating_sub(1);
        if inner_right <= inner_x {
            return;
        }
        let inner_width = (inner_right - inner_x) as usize;
        let span = self.span;

        // ── Row 0: badge + time pills ────────────────────────────────────────
        let y0 = area.y + 1;
        let mut x = inner_x;

        let (fg, bg) = span.type_.badge_colors();
        let badge = format!(" {} ", span.type_.label());
        let badge_len = badge.chars().count() as u16;
        if x + badge_len <= inner_right {
            buf.set_stringn(x, y0, &badge, badge_len as usize, Style::default().fg(fg).bg(bg));
            x += badge_len + 1;
        }

        // start/end relative to unit start
        let start = span.start_time;
        let end = span.start_time + span.duration;
        let pills = [
            format!(" ⏱  {} ", format_time(span.duration)),
            format!(" {} → {} ", format_time(start), format_time(end)),
        ];
        let pill_style = Style::default()
            .fg(Color::Rgb(198, 208, 245))
            .bg(Color::Rgb(41, 44, 60));
        for pill in &pills {
            let plen = pill.chars().count() as u16;
            if x + plen > inner_right {
                break;
            }
            buf.set_stringn(x, y0, pill, plen as usize, pill_style);
            x += plen + 1;
        }

        // ── Label (wrapped, bold) ────────────────────────────────────────────
        let label_lines = wrap_text(&span.label, inner_width);
        let mut y = area.y + 2;
        for line in &label_lines {
            if y >= area.bottom().saturating_sub(1) {
                break;
            }
            buf.set_stringn(
                inner_x, y, line, inner_width,
                Style::default().add_modifier(Modifier::BOLD),
            );
            y += 1;
        }

        // ── Details (wrapped, muted) ─────────────────────────────────────────
        if let Some(details) = &span.details {
            for line in wrap_text(details, inner_width) {
                if y >= area.bottom().saturating_sub(1) {
                    break;
                }
                buf.set_stringn(
                    inner_x, y, &line, inner_width,
                    Style::default().fg(Color::Rgb(108, 112, 134)),
                );
                y += 1;
            }
        }
    }
}
