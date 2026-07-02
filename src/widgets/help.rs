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
        let rows = (n + 1) / 2;
        let height = (rows as u16 + 2).min(area.height);

        // Find max width for each column to calculate total width
        let midpoint = (n + 1) / 2;
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
            .title(Line::from(vec![
                Span::raw(" ℹ️ Help "),
            ]))
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::Gray));

        let inner_area = block.inner(popup_area);
        block.render(popup_area, buf);

        let layout = Layout::horizontal([Constraint::Length(left_width), Constraint::Length(2), Constraint::Length(right_width)])
            .split(inner_area);

        render_column(left_col, layout[0], buf);
        render_column(right_col, layout[2], buf);
    }
}

fn render_column(items: &[(&str, &str)], area: Rect, buf: &mut Buffer) {
    let mut y = area.y;
    for (key, desc) in items {
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
        y += 1;
    }
}
