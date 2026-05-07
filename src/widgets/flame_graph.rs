use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

use crate::app::span::{Span, SpanType};

pub struct Flamegraph<'a> {
    pub spans: &'a [Span],
    pub total_duration: f64,
}

#[derive(Clone, Copy)]
enum SubcellAlign {
    Left,
    Right,
}

impl<'a> Widget for Flamegraph<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let time_per_col = self.total_duration / area.width as f64;
        let mut subcell_tracker: HashMap<(u16, u16), (f64, SubcellAlign, Color)> = HashMap::new();

        let try_claim = |tracker: &mut HashMap<(u16, u16), (f64, SubcellAlign, Color)>,
                         col: i32,
                         y: u16,
                         fraction: f64,
                         align: SubcellAlign,
                         color: Color| {
            let x = area.x as i32 + col;
            if x >= area.x as i32 && x < area.right() as i32 {
                let cell_coord = (x as u16, y);
                let current = tracker.get(&cell_coord).map(|(f, _, _)| *f).unwrap_or(0.0);
                if fraction > current {
                    tracker.insert(cell_coord, (fraction, align, color));
                }
            }
        };

        for (i, span) in self.spans.iter().enumerate() {
            let y = area.y + span.depth as u16;

            if y >= area.bottom() {
                continue;
            }

            let bg_color = span.get_checkerboard_color(i);
            let exact_width_cols = span.duration / time_per_col;
            let start_float = span.start_time / time_per_col;
            let end_float = start_float + exact_width_cols;

            let start_col = start_float.floor() as i32;
            let end_col = end_float.floor() as i32;
            let start_frac = start_float.fract();
            let end_frac = end_float.fract();

            if start_col == end_col {
                try_claim(
                    &mut subcell_tracker,
                    start_col,
                    y,
                    exact_width_cols,
                    SubcellAlign::Left,
                    bg_color,
                );
                continue;
            }

            // Pre-fraction
            if start_frac > 0.0 {
                try_claim(
                    &mut subcell_tracker,
                    start_col,
                    y,
                    1.0 - start_frac,
                    SubcellAlign::Right,
                    bg_color,
                );
            }

            // Core
            let core_start = start_float.ceil() as i32;
            let core_end = end_float.floor() as i32;
            let core_width = (core_end - core_start).max(0);

            if core_width > 0 {
                let x_start = area.x as i32 + core_start;
                let x_end = area.x as i32 + core_end;

                if x_end > area.x as i32 && x_start < area.right() as i32 {
                    let visible_start = x_start.max(area.x as i32) as u16;
                    let visible_end = x_end.min(area.right() as i32) as u16;
                    let width = visible_end.saturating_sub(visible_start);

                    if width > 0 {
                        let fg_color = match bg_color {
                            Color::DarkGray => Color::White,
                            _ => Color::Black,
                        };

                        let rect = Rect::new(visible_start, y, width, 1);
                        buf.set_style(rect, Style::default().bg(bg_color));

                        let width_usize = width as usize;
                        let label_len = span.label.chars().count();

                        let display_text = if width_usize == 1 {
                            "𝅏".to_string()
                        } else if label_len > width_usize {
                            let truncated: String =
                                span.label.chars().take(width_usize - 1).collect();
                            truncated + "…"
                        } else {
                            span.label.clone()
                        };

                        buf.set_stringn(
                            rect.x,
                            rect.y,
                            &display_text,
                            rect.width as usize,
                            Style::default().fg(fg_color).bg(bg_color),
                        );
                    }
                }
            }

            // Post-fraction
            if end_frac > 0.0 {
                try_claim(&mut subcell_tracker, end_col, y, end_frac, SubcellAlign::Left, bg_color);
            }
        }

        for ((x, y), (fraction, align, color)) in &subcell_tracker {
            let partial_char = if matches!(align, SubcellAlign::Right) {
                if *fraction < 0.25 {
                    "▕"
                } else if *fraction < 0.5 {
                    "▐"
                } else {
                    "█"
                }
            } else if *fraction < 0.125 {
                "▏"
            } else if *fraction < 0.25 {
                "▎"
            } else if *fraction < 0.375 {
                "▍"
            } else if *fraction < 0.5 {
                "▌"
            } else if *fraction < 0.625 {
                "▋"
            } else if *fraction < 0.75 {
                "▊"
            } else if *fraction < 0.875 {
                "▉"
            } else {
                "█"
            };

            if let Some(cell) = buf.cell_mut((*x, *y)) {
                cell.set_symbol(partial_char);
                cell.set_fg(*color);
            }
        }
    }
}

impl Span {
    /// Returns the base RGB color for this span type
    fn base_rgb(&self) -> (u8, u8, u8) {
        match self.type_ {
            // Catppuccin Sapphire (Soft Pastel Blue)
            SpanType::Source => (116, 199, 236),

            // Catppuccin Mauve (Soft Pastel Purple)
            SpanType::Class => (203, 166, 247),

            // Catppuccin Yellow (Soft Pastel Yellow)
            SpanType::Template => (249, 226, 175),

            // Catppuccin Overlay0 (Warm muted gray)
            SpanType::Task => (118, 122, 144),
        }
    }

    /// Calculates a distinct shade based on 2D position
    pub fn get_checkerboard_color(&self, horizontal_index: usize) -> Color {
        let base = self.base_rgb();

        // We use an i16 to prevent overflow/underflow when doing math on colors
        let mut brightness_shift: i16 = 0;

        // 1. Shift based on vertical depth
        if self.depth % 2 != 0 {
            brightness_shift -= 20; // Odd rows are slightly darker
        }

        // 2. Shift based on horizontal sibling position
        if horizontal_index % 2 != 0 {
            brightness_shift += 10; // Odd siblings are slightly lighter
        }

        // 3. Apply the shift and clamp to valid u8 RGB limits (0-255)
        let r = (base.0 as i16 + brightness_shift).clamp(0, 255) as u8;
        let g = (base.1 as i16 + brightness_shift).clamp(0, 255) as u8;
        let b = (base.2 as i16 + brightness_shift).clamp(0, 255) as u8;

        Color::Rgb(r, g, b)
    }
}
