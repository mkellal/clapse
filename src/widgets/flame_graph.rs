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

impl<'a> Widget for Flamegraph<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let time_per_col = self.total_duration / area.width as f64;

        // Track the largest fraction drawn into any specific (X, Y) cell
        let mut subcell_tracker: HashMap<(u16, u16), f64> = HashMap::new();

        for (i, span) in self.spans.iter().enumerate() {
            let y = area.y + span.depth as u16;

            // 1. Cull if completely below the screen
            if y >= area.bottom() {
                continue;
            }

            let bg_color = span.get_checkerboard_color(i);

            let exact_width_cols = span.duration / time_per_col;
            let start_float = span.start_time / time_per_col;

            // ==========================================
            // PATH A: THE SUB-CELL RENDERER (Duration < 1 Column)
            // ==========================================
            if exact_width_cols < 1.0 {
                // Because it's smaller than a grid cell, it might straddle a boundary (0.9 to 1.1).
                // To decide which cell gets the block, we find the span's exact center.
                let center_float = start_float + (exact_width_cols / 2.0);

                // Snap the center to our grid
                let grid_x = area.x as i32 + center_float.floor() as i32;

                // Cull if off-screen
                if grid_x < area.x as i32 || grid_x >= area.right() as i32 || y >= area.bottom() {
                    continue;
                }

                let visible_x = grid_x as u16;
                let bg_color = span.get_checkerboard_color(i);
                let cell_coord = (visible_x, y);

                // Track the largest sub-cell (using your exact exact_width_cols)
                let current_max = subcell_tracker.get(&cell_coord).copied().unwrap_or(0.0);
                if exact_width_cols > current_max {
                    subcell_tracker.insert(cell_coord, exact_width_cols);

                    let partial_char = if exact_width_cols < 0.125 {
                        "▏"
                    } else if exact_width_cols < 0.25 {
                        "▎"
                    } else if exact_width_cols < 0.375 {
                        "▍"
                    } else if exact_width_cols < 0.5 {
                        "▌"
                    } else if exact_width_cols < 0.625 {
                        "▋"
                    } else if exact_width_cols < 0.75 {
                        "▊"
                    } else if exact_width_cols < 0.875 {
                        "▉"
                    } else {
                        "█"
                    };

                    if let Some(cell) = buf.cell_mut(cell_coord) {
                        cell.set_symbol(partial_char);
                        cell.set_fg(bg_color);
                    }
                }
                continue; // Done rendering this tiny span!
            }

            // ==========================================
            // PATH B: THE STANDARD RENDERER (Duration >= 1 Column)
            // ==========================================
            // It's a full block! Now we use .round() on start and end independently
            // to guarantee parents and children perfectly align without spilling.
            let end_float = (span.start_time + span.duration) / time_per_col;

            let x_start_i32 = area.x as i32 + start_float.round() as i32;
            let x_end_i32 = area.x as i32 + end_float.round() as i32;

            // Cull if off-screen
            if x_end_i32 <= area.x as i32
                || x_start_i32 >= area.right() as i32
                || y >= area.bottom()
            {
                continue;
            }

            // Clip to screen
            let visible_start = x_start_i32.max(area.x as i32) as u16;
            let visible_end = x_end_i32.min(area.right() as i32) as u16;
            let width = visible_end.saturating_sub(visible_start);

            // Prevent weird integer collapse on exact boundary lines
            if width == 0 {
                continue;
            }
            
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
                let truncated: String = span.label.chars().take(width_usize - 1).collect();
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
