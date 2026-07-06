use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Widget},
};

use crate::app::span::{Span, SpanType};
use crate::app::view::DetailProvider;
use crate::widgets::time_range::format_time;

pub struct SpanDetails<'a, V: DetailProvider> {
    pub spans: &'a [Span],
    pub view: &'a V,
    pub parent_duration: Option<f64>,
    pub total_duration: f64,
}

fn wrap_text(text: &str, width: usize) -> Vec<String> {
    if width == 0 {
        return vec![];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if current.is_empty() {
            // Word longer than width: hard-wrap it.
            if word.chars().count() > width {
                let mut chunk = String::new();
                for c in word.chars() {
                    if chunk.chars().count() == width {
                        lines.push(chunk.clone());
                        chunk.clear();
                    }
                    chunk.push(c);
                }
                if !chunk.is_empty() {
                    current = chunk;
                }
                continue;
            }
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
        (Color::Black, self.get_color(None, None))
    }
}

impl<'a, V: DetailProvider> SpanDetails<'a, V> {
    /// Compute the total height (including borders) needed to display all content at the given width.
    pub fn required_height(&self, area_width: u16) -> u16 {
        if area_width < 4 {
            return 3;
        }
        let span = &self.spans[self.view.span_index()];
        let inner_width = (area_width - 2) as usize;
        let label_lines = wrap_text(&span.label, inner_width).len() as u16;
        let identifier_lines = if span.identifier != span.label {
            wrap_text(&span.identifier, inner_width).len() as u16
        } else {
            0
        };
        let operation_lines = span
            .sublabel
            .as_deref()
            .map(|d| wrap_text(d, inner_width).len() as u16)
            .unwrap_or(0);
        // 2 borders + 1 row (badge + pills) + label rows + identifier rows + operation rows
        2 + label_lines + identifier_lines + operation_lines
    }
}

impl<'a, V: DetailProvider> Widget for SpanDetails<'a, V> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height < 3 || area.width == 0 {
            return;
        }

        // Clear every cell in the overlay area so the flamegraph behind is hidden.
        for y in area.y..area.bottom() {
            for x in area.x..area.right() {
                buf[(x, y)].reset();
            }
        }

        let border_style = Style::default().fg(Color::Rgb(88, 91, 112));
        Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .style(Style::default())
            .render(area, buf);

        let inner_x = area.x + 1;
        let inner_right = area.right().saturating_sub(1);
        if inner_right <= inner_x {
            return;
        }
        let inner_width = (inner_right - inner_x) as usize;
        let span = &self.spans[self.view.span_index()];

        // ── Row 0: badge + time pills ────────────────────────────────────────
        let y0 = area.y + 1;
        let mut x = inner_x;

        let (fg, bg) = span.type_.badge_colors();
        let badge = format!(" {} ", span.type_.label());
        let badge_len = badge.chars().count() as u16;
        if x + badge_len <= inner_right {
            buf.set_stringn(
                x,
                y0,
                &badge,
                badge_len as usize,
                Style::default().fg(fg).bg(bg),
            );
            x += badge_len + 1;
        }

        // Operation tag (e.g. "Parsing", "Instantiation")
        if let Some(op) = &span.sublabel {
            let tag = format!(" {} ", op);
            let tag_len = tag.chars().count() as u16;
            if x + tag_len <= inner_right {
                buf.set_stringn(
                    x,
                    y0,
                    &tag,
                    tag_len as usize,
                    Style::default()
                        .fg(Color::Rgb(198, 208, 245))
                        .bg(Color::Rgb(51, 54, 74)),
                );
                x += tag_len + 1;
            }
        }

        // Time pills
        let start = span.start_time;
        let end = span.start_time + span.duration;
        let mut pills: Vec<String> = Vec::new();

        if let Some(count) = self.view.count() {
            pills.push(format!(" {} times ", count));
            pills.push(format!(
                " ⏱ avg {} ",
                format_time(span.duration / count as f64)
            ));
            pills.push(format!(" ⏱ sum {} ", format_time(span.duration)));
        } else {
            pills.push(format!(" ⏱ {} ", format_time(span.duration)));
        }

        if let Some(pd) = self.parent_duration {
            if pd > 0.0 {
                let pct = span.duration / pd * 100.0;
                pills.push(format!(" {:.1}% of parent ", pct));
            }
        }
        if self.total_duration > 0.0 {
            let pct = span.duration / self.total_duration * 100.0;
            pills.push(format!(" {:.3}% of total ", pct));
        }
        if self.view.count().is_none() {
            pills.push(format!(" {} → {} ", format_time(start), format_time(end)));
        }
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
                inner_x,
                y,
                line,
                inner_width,
                Style::default().add_modifier(Modifier::BOLD),
            );
            y += 1;
        }

        // ── Full identifier (wrapped, muted) — only when different from label ─
        if span.identifier != span.label {
            for line in wrap_text(&span.identifier, inner_width) {
                if y >= area.bottom().saturating_sub(1) {
                    break;
                }
                buf.set_stringn(
                    inner_x,
                    y,
                    &line,
                    inner_width,
                    Style::default().fg(Color::Rgb(147, 153, 178)),
                );
                y += 1;
            }
        }
    }
}
