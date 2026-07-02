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

pub fn format_time(us: f64) -> String {
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
                cell.set_symbol("▀");
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

#[cfg(test)]
mod tests {
    use super::*;

    // ── helpers ──

    /// Check if any cell in buffer at row `y` has the given symbol.
    fn row_has_symbol(buf: &Buffer, y: u16, symbol: &str) -> bool {
        for x in 0..buf.area.width {
            if let Some(cell) = buf.cell((x, y)) {
                if cell.symbol() == symbol {
                    return true;
                }
            }
        }
        false
    }

    /// Check if any cell in buffer at row `y` has non-empty content.
    fn row_has_content(buf: &Buffer, y: u16) -> bool {
        for x in 0..buf.area.width {
            if let Some(cell) = buf.cell((x, y)) {
                if !cell.symbol().trim().is_empty() {
                    return true;
                }
            }
        }
        false
    }

    // ── tick_interval ──

    #[test]
    fn test_tick_interval_zero() {
        assert_eq!(tick_interval(0.0), 1.0);
    }

    #[test]
    fn test_tick_interval_microseconds() {
        // visible=100µs → target=12.5 → mag=10 → norm=1.25 → nice=1.0 → 10µs
        assert_eq!(tick_interval(100.0), 10.0);
    }

    #[test]
    fn test_tick_interval_milliseconds() {
        // visible=10_000µs → target=1250 → mag=1000 → norm=1.25 → nice=1.0 → 1000µs
        assert_eq!(tick_interval(10_000.0), 1_000.0);
    }

    #[test]
    fn test_tick_interval_2_5x() {
        // visible=20_000 → target=2500 → mag=1000 → norm=2.5 → nice=2.5 → 2500
        assert_eq!(tick_interval(20_000.0), 2_500.0);
    }

    #[test]
    fn test_tick_interval_5x() {
        // visible=40_000 → target=5000 → mag=1000 → norm=5.0 → nice=5.0 → 5000
        assert_eq!(tick_interval(40_000.0), 5_000.0);
    }

    #[test]
    fn test_tick_interval_10x() {
        // visible=80_000 → target=10000 → mag=10000 → norm=1.0 → nice=1.0 → 10000
        assert_eq!(tick_interval(80_000.0), 10_000.0);
    }

    // ── format_time ──

    #[test]
    fn test_format_time_sub_ms() {
        assert_eq!(format_time(500.0), "500µs");
    }

    #[test]
    fn test_format_time_exactly_1ms() {
        assert_eq!(format_time(1_000.0), "1.0ms");
    }

    #[test]
    fn test_format_time_milliseconds() {
        assert_eq!(format_time(5_000.0), "5.0ms");
    }

    #[test]
    fn test_format_time_near_1s() {
        // Seconds formatting is commented out; stays in ms
        assert_eq!(format_time(999_999.0), "1000.0ms");
    }

    // ── DurationRange widget ──

    #[test]
    fn test_render_zero_width() {
        let area = Rect::new(0, 0, 0, 3);
        let mut buf = Buffer::empty(area);
        let range = DurationRange {
            total_duration: 1_000.0,
            start: 0.0,
            visible_duration: 500.0,
        };
        range.render(area, &mut buf);
        // Should return early, no panic
    }

    #[test]
    fn test_render_too_short() {
        let area = Rect::new(0, 0, 80, 1);
        let mut buf = Buffer::empty(area);
        let range = DurationRange {
            total_duration: 1_000.0,
            start: 0.0,
            visible_duration: 500.0,
        };
        range.render(area, &mut buf);
        // height < 2 → early return; track row (y=1) doesn't exist
        assert!(!row_has_content(&buf, 1));
    }

    #[test]
    fn test_render_valid() {
        let area = Rect::new(0, 0, 80, 3);
        let mut buf = Buffer::empty(area);
        let range = DurationRange {
            total_duration: 1_000_000.0, // 1s total
            start: 0.0,
            visible_duration: 500_000.0, // showing first 500ms
        };
        range.render(area, &mut buf);

        // Scrollbar row (y=0) should have the thumb "▀" symbol
        assert!(row_has_symbol(&buf, 0, "▀"), "scrollbar row missing ▀");

        // Track row (y=1) should have tick marks "|"
        assert!(row_has_symbol(&buf, 1, "|"), "track row missing | ticks");
    }

    #[test]
    fn test_render_zero_total_duration() {
        let area = Rect::new(0, 0, 80, 3);
        let mut buf = Buffer::empty(area);
        let range = DurationRange {
            total_duration: 0.0,
            start: 0.0,
            visible_duration: 100.0,
        };
        range.render(area, &mut buf);
        // total_duration ≤ 0 → early return
        assert!(!row_has_content(&buf, 0));
    }
}
