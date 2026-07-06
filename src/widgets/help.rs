use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Widget};

pub struct HelpPopup<'a> {
    pub combinations: Vec<(&'a str, &'a str)>,
}

impl<'a> HelpPopup<'a> {
    pub fn new(combinations: Vec<(&'a str, &'a str)>) -> Self {
        Self { combinations }
    }
}

impl Widget for HelpPopup<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let mut all_combinations = vec![
            ("?", "Toggle help"),
            ("q", "Quit"),
            ("s", "Search"),
            ("Alt + 1/2/3", "Jump to tab"),
            ("Alt + t", "Next tab"),
            ("Esc", "Close help / deselect"),
        ];
        all_combinations.extend(self.combinations.iter().copied());

        let n = all_combinations.len();
        let rows = n.div_ceil(2);
        let height = (rows as u16 + 2).min(area.height);

        // Find max width for each column to calculate total width
        let midpoint = n.div_ceil(2);
        let left_col = &all_combinations[..midpoint];
        let right_col = &all_combinations[midpoint..];

        let calc_width = |items: &[(&str, &str)]| -> u16 {
            items
                .iter()
                .map(|(k, d)| k.len() as u16 + d.len() as u16 + 5) // "<" + k + "> " + d
                .max()
                .unwrap_or(0)
        };

        let left_width = calc_width(left_col);
        let right_width = calc_width(right_col);
        let width = (left_width + right_width + 4).min(area.width); // +4 for borders and spacing

        let popup_area = Rect {
            x: area.x + (area.width.saturating_sub(width)) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        };

        Clear.render(popup_area, buf);

        let block = Block::default()
            .title(Line::from(vec![Span::raw(" ℹ️ Help ")]))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray));

        let inner_area = block.inner(popup_area);
        block.render(popup_area, buf);

        let layout = Layout::horizontal([
            Constraint::Length(left_width),
            Constraint::Length(2),
            Constraint::Length(right_width),
        ])
        .split(inner_area);

        render_column(left_col, layout[0], buf);
        render_column(right_col, layout[2], buf);
    }
}

fn render_column(items: &[(&str, &str)], area: Rect, buf: &mut Buffer) {
    for (i, (key, desc)) in items.iter().enumerate() {
        let y = area.y + i as u16;
        if y >= area.bottom() {
            break;
        }

        let line = Line::from(vec![
            Span::styled("<", Color::DarkGray),
            Span::styled(*key, Color::Red),
            Span::styled("> ", Color::DarkGray),
            Span::raw(*desc),
        ]);
        line.render(Rect::new(area.x, y, area.width, 1), buf);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Collect all text from a buffer row as a String.
    fn row_text(buf: &Buffer, y: u16, x_start: u16, width: u16) -> String {
        (x_start..x_start + width)
            .map(|x| buf.cell((x, y)).map_or(" ", |c| c.symbol()).to_string())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    /// Check if the full buffer text (all rows concatenated) contains `needle`.
    fn buffer_contains(buf: &Buffer, needle: &str) -> bool {
        for y in 0..buf.area.height {
            let row: String = (0..buf.area.width)
                .map(|x| buf.cell((x, y)).map_or(" ", |c| c.symbol()))
                .collect();
            if row.contains(needle) {
                return true;
            }
        }
        false
    }

    // ── render_column ──

    #[test]
    fn test_render_column_single_item() {
        let area = Rect::new(0, 0, 30, 5);
        let mut buf = Buffer::empty(area);
        let items = vec![("q", "Quit")];
        render_column(&items, area, &mut buf);

        let line = row_text(&buf, 0, 0, 30);
        assert!(line.contains("<q>"), "should contain <q>, got: {line}");
        assert!(line.contains("Quit"), "should contain Quit, got: {line}");
    }

    #[test]
    fn test_render_column_multiple_items() {
        let area = Rect::new(0, 0, 30, 5);
        let mut buf = Buffer::empty(area);
        let items = vec![("a", "Alpha"), ("b", "Beta"), ("c", "Gamma")];
        render_column(&items, area, &mut buf);

        assert!(row_text(&buf, 0, 0, 30).contains("Alpha"));
        assert!(row_text(&buf, 1, 0, 30).contains("Beta"));
        assert!(row_text(&buf, 2, 0, 30).contains("Gamma"));
    }

    #[test]
    fn test_render_column_clipped() {
        let area = Rect::new(0, 0, 30, 2);
        let mut buf = Buffer::empty(area);
        let items = vec![("a", "First"), ("b", "Second"), ("c", "Third")];
        render_column(&items, area, &mut buf);

        // First two should render, third should be clipped
        assert!(row_text(&buf, 0, 0, 30).contains("First"));
        assert!(row_text(&buf, 1, 0, 30).contains("Second"));
        // Row 2 doesn't exist (area height is 2, rows 0 and 1)
    }

    // ── HelpPopup widget ──

    #[test]
    fn test_help_popup_renders_defaults() {
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        let popup = HelpPopup::new(vec![]);
        popup.render(area, &mut buf);

        // All 6 defaults should appear
        assert!(buffer_contains(&buf, "Quit"), "should contain Quit");
        assert!(buffer_contains(&buf, "Search"), "should contain Search");
        assert!(
            buffer_contains(&buf, "Toggle help"),
            "should contain Toggle help"
        );
        assert!(
            buffer_contains(&buf, "Jump to tab"),
            "should contain Jump to tab"
        );
        assert!(buffer_contains(&buf, "Next tab"), "should contain Next tab");
        assert!(
            buffer_contains(&buf, "Close help"),
            "should contain Close help"
        );
    }

    #[test]
    fn test_help_popup_with_custom_combinations() {
        let area = Rect::new(0, 0, 80, 20);
        let mut buf = Buffer::empty(area);
        let popup = HelpPopup::new(vec![("x", "Custom action")]);
        popup.render(area, &mut buf);

        // Defaults still present
        assert!(buffer_contains(&buf, "Quit"));
        // Custom entry added
        assert!(
            buffer_contains(&buf, "Custom action"),
            "should contain custom entry"
        );
    }

    #[test]
    fn test_help_popup_too_small_area_still_renders() {
        // Very small area — should still render without panic
        let area = Rect::new(0, 0, 20, 5);
        let mut buf = Buffer::empty(area);
        let popup = HelpPopup::new(vec![]);
        popup.render(area, &mut buf);

        // Just verify it didn't panic — at minimum the title should render
        let has_content = (0..area.height).any(|y| {
            (0..area.width).any(|x| {
                buf.cell((x, y))
                    .map_or(false, |c| !c.symbol().trim().is_empty())
            })
        });
        assert!(has_content, "buffer should have some rendered content");
    }
}
