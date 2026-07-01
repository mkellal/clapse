use ratatui::{
    buffer::Buffer,
    layout::{Alignment, Rect},
    style::{Color, Style},
    text::{Line, Text},
    widgets::{Block, Gauge, Paragraph, Widget},
};

/// Full-screen loading widget with ASCII art logo and progress bar.
pub struct StartScreenWidget {
    /// Progress in range 0.0 ..= 1.0
    pub progress: f64,
    /// Optional status message below the bar.
    pub message: String,
}

const CLAPSE_ASCII: &str = r#"
 ____ _     ____  ____  ____  _____
/   _Y \   /  _ \/  __\/ ___\/  __/
|  / | |   | / \||  \/||    \|  \  
|  \_| |_/\| |-|||  __/\___ ||  /_ 
\____|____/\_/ \|\_/   \____/\____\
                                   
"#;

impl Widget for StartScreenWidget {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || area.height == 0 {
            return;
        }

        // Clear background
        Block::new().style(Style::default()).render(area, buf);

        // Center the content vertically and horizontally
        let ascii_lines: Vec<&str> = CLAPSE_ASCII.lines().collect();
        let ascii_height = ascii_lines.len() as u16;
        let content_height = ascii_height + 2 + 1 + 1; // ascii + gap + gauge + message
        let vertical_pad = area.height.saturating_sub(content_height) / 2;
        let horizontal_pad = (area.width.saturating_sub(60) / 2).min(area.width / 4);

        let inner = Rect {
            x: area.x + horizontal_pad,
            y: area.y + vertical_pad,
            width: area.width.saturating_sub(horizontal_pad * 2),
            height: content_height.min(area.height),
        };

        if inner.width < 20 || inner.height < ascii_height {
            // Terminal too small — show minimal
            let text = Text::from("Clapse").centered();
            Paragraph::new(text)
                .style(Style::default().fg(Color::Green))
                .render(
                    Rect {
                        x: area.x,
                        y: area.y + area.height / 2,
                        width: area.width,
                        height: 1,
                    },
                    buf,
                );
            return;
        }

        let mut y = inner.y;

        // ASCII art logo
        for line in ascii_lines {
            if y >= area.bottom() {
                break;
            }
            let logo_line = Line::from(line.to_string())
                .style(Style::default().fg(Color::Green))
                .alignment(Alignment::Center);
            logo_line.render(
                Rect {
                    x: inner.x,
                    y,
                    width: inner.width,
                    height: 1,
                },
                buf,
            );
            y += 1;
        }

        y += 1; // gap

        // Progress bar
        let gauge_area = Rect {
            x: inner.x,
            y,
            width: inner.width,
            height: 1,
        };
        let label = format!("{:.0}%", (self.progress * 100.0));
        Gauge::default()
            .gauge_style(Style::default().fg(Color::Green).bg(Color::DarkGray))
            .ratio(self.progress.clamp(0.0, 1.0))
            .label(label)
            .render(gauge_area, buf);

        y += 2; // gap

        // Status message
        if y < area.bottom() {
            let msg = if self.message.is_empty() {
                "Loading trace files...".to_string()
            } else {
                self.message
            };
            let msg_line = Line::from(msg)
                .style(Style::default().fg(Color::Gray))
                .alignment(Alignment::Center);
            msg_line.render(
                Rect {
                    x: inner.x,
                    y,
                    width: inner.width,
                    height: 1,
                },
                buf,
            );
        }
    }
}
