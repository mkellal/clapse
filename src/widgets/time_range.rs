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
    // if us >= 1_000_000.0 {
    //     format!("{:.2}s", us / 1_000_000.0)
    // } else
     if us >= 1_000.0 {
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

        let scroll_y = area.y;
        let track_y = area.y + 1;
        let w = area.width as f64;
        let interval = tick_interval(self.visible_duration);

        // ── Track row ────────────────────────────────────────────────────────
        // Full width = [start, start + visible_duration].
        // Ticks at multiples of interval; label placed immediately right of tick.
        let first_k = (self.start / interval).ceil() as i64;
        let last_k = ((self.start + self.visible_duration) / interval).floor() as i64;

        for k in first_k..=last_k {
            let t = k as f64 * interval;
            let frac = (t - self.start) / self.visible_duration;
            let tick_x = area.x + (frac * w).round() as u16;
            let tick_x = tick_x.clamp(area.x, area.x + area.width.saturating_sub(1));

            if let Some(cell) = buf.cell_mut((tick_x, track_y)) {
                cell.set_symbol("|");
                cell.set_fg(Color::DarkGray);
            }

            let label = format_time(t);
            let label_len = label.chars().count() as u16;
            // Draw label immediately right of tick if it fits
            let label_x = tick_x + 1;
            if label_x + label_len <= area.x + area.width {
                buf.set_stringn(
                    label_x,
                    track_y,
                    &label,
                    label_len as usize,
                    Style::default().fg(Color::Gray),
                );
            }
        }

        // ── Scrollbar row ────────────────────────────────────────────────────
        // Full width = [0, total_duration]. Thumb covers the visible range.
        for x in 0..area.width {
            if let Some(cell) = buf.cell_mut((area.x + x, scroll_y)) {
                cell.set_symbol("▔");
                cell.set_fg(Color::DarkGray);
            }
        }

        let thumb_start = ((self.start / self.total_duration) * w).round() as u16;
        let thumb_end =
            (((self.start + self.visible_duration) / self.total_duration) * w).round() as u16;
        let thumb_start = thumb_start.min(area.width);
        let thumb_end = thumb_end.clamp(thumb_start, area.width);

        for x in thumb_start..thumb_end {
            if let Some(cell) = buf.cell_mut((area.x + x, scroll_y)) {
                cell.set_symbol("▀");
                cell.set_fg(Color::White);
            }
        }

        if thumb_start >= thumb_end {
            let x = thumb_start.min(area.width.saturating_sub(1));
            if let Some(cell) = buf.cell_mut((area.x + x, scroll_y)) {
                cell.set_symbol("▀");
                cell.set_fg(Color::White);
            }
        }
    }
}
