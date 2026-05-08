use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

pub struct DurationRange {
    pub total_duration: f64,
    pub start: f64,
    pub visible_duration: f64,
}

/// Returns a "nice" tick interval in µs for a given visible window.
/// Stable at a given zoom level: depends only on visible_duration.
pub fn tick_interval(visible_duration: f64) -> f64 {
    if visible_duration <= 0.0 {
        return 1.0;
    }
    let target = visible_duration / 8.0;
    let magnitude = 10_f64.powf(target.log10().floor());
    let normalized = target / magnitude;
    let nice = if normalized < 1.5 {
        1.0
    } else if normalized < 3.5 {
        2.5
    } else if normalized < 7.5 {
        5.0
    } else {
        10.0
    };
    nice * magnitude
}

fn format_time(us: f64) -> String {
    if us >= 1_000_000.0 {
        format!("{:.2}s", us / 1_000_000.0)
    } else if us >= 1_000.0 {
        format!("{:.1}ms", us / 1_000.0)
    } else {
        format!("{:.0}µs", us)
    }
}

impl Widget for DurationRange {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || self.total_duration <= 0.0 || area.height < 2 {
            return;
        }

        let grad_y = area.y;
        let bar_y = area.y + 1;
        let w = area.width as f64;
        let interval = tick_interval(self.visible_duration);

        // Base track
        for x in 0..area.width {
            if let Some(cell) = buf.cell_mut((area.x + x, bar_y)) {
                cell.set_symbol("▁");
                cell.set_fg(Color::DarkGray);
            }
        }

        // Tick marks (all across timeline) and labels (only for visible ticks)
        let last_k = (self.total_duration / interval).ceil() as i64;
        let mut last_bar_x: Option<u16> = None;
        let mut next_label_x: u16 = area.x;

        for k in 0..=last_k {
            let t = (k as f64 * interval).min(self.total_duration);

            let bar_x = area.x + ((t / self.total_duration) * w).round() as u16;
            let bar_x = bar_x.min(area.x + area.width.saturating_sub(1));

            if last_bar_x != Some(bar_x) {
                if let Some(cell) = buf.cell_mut((bar_x, bar_y)) {
                    cell.set_symbol("╷");
                    cell.set_fg(Color::DarkGray);
                }
                last_bar_x = Some(bar_x);
            }

            // Label only if this tick falls within the visible window
            let in_visible = t >= self.start - interval * 0.01
                && t <= self.start + self.visible_duration + interval * 0.01;
            if in_visible {
                let label = format_time(t);
                let label_len = label.chars().count() as u16;
                let draw_x = bar_x
                    .saturating_sub(label_len / 2)
                    .max(area.x)
                    .min(area.x + area.width.saturating_sub(label_len));
                if draw_x >= next_label_x {
                    buf.set_stringn(
                        draw_x,
                        grad_y,
                        &label,
                        label_len as usize,
                        Style::default().fg(Color::Gray),
                    );
                    next_label_x = draw_x + label_len + 1;
                }
            }
        }

        // Thumb overwrites tick marks in the visible range
        let thumb_start = ((self.start / self.total_duration) * w).round() as u16;
        let thumb_end =
            (((self.start + self.visible_duration) / self.total_duration) * w).round() as u16;
        let thumb_start = thumb_start.min(area.width);
        let thumb_end = thumb_end.clamp(thumb_start, area.width);

        for x in thumb_start..thumb_end {
            if let Some(cell) = buf.cell_mut((area.x + x, bar_y)) {
                cell.set_symbol("▄");
                cell.set_fg(Color::White);
            }
        }
    }
}
