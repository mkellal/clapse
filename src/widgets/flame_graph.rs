use std::collections::HashMap;

use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};

use crate::{
    app::span::{Span, SpanType},
    widgets::graph_span::{GraphSpan, SubcellAlign},
};

pub struct Flamegraph<'a> {
    pub spans: &'a [Span],
    pub total_duration: f64,
}

impl<'a> Widget for Flamegraph<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let time_per_col = self.total_duration / area.width as f64;
        let mut subcell_tracker: HashMap<(u16, u16), (f64, SubcellAlign, Color)> = HashMap::new();

        let graph = GraphSpan {
            spans: self.spans,
            time_per_col,
            flamegraph_area: area,
        };

        for (i, span) in self.spans.iter().enumerate() {
            if span.contained_by_index.is_some() {
                continue;
            }
            let sf = span.start_time / time_per_col;
            let ef = (span.start_time + span.duration) / time_per_col;
            let x_start = (area.x as i32 + sf.round() as i32)
                .max(area.x as i32)
                .min(area.right() as i32) as u16;
            let x_end = (area.x as i32 + ef.round() as i32)
                .max(area.x as i32)
                .min(area.right() as i32) as u16;
            let width = x_end.saturating_sub(x_start);
            if width == 0 {
                continue;
            }
            graph.render_span(
                i,
                i,
                Rect::new(x_start, area.y, width, 1),
                buf,
                &mut subcell_tracker,
            );
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
            brightness_shift -= 40; // Odd rows are slightly darker
        }

        // 2. Shift based on horizontal sibling position
        if horizontal_index % 2 != 0 {
            brightness_shift += 20; // Odd siblings are slightly lighter
        }

        // 3. Apply the shift and clamp to valid u8 RGB limits (0-255)
        let r = (base.0 as i16 + brightness_shift).clamp(0, 255) as u8;
        let g = (base.1 as i16 + brightness_shift).clamp(0, 255) as u8;
        let b = (base.2 as i16 + brightness_shift).clamp(0, 255) as u8;

        Color::Rgb(r, g, b)
    }
}
