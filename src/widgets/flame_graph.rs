use std::collections::HashMap;

use colors_transform::{Color as _, Hsl, Rgb};
use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};

use crate::{
    app::span::{Span, SpanType},
    widgets::graph_span::{GraphSpan, SubcellAlign},
};

pub struct Flamegraph<'a> {
    pub spans: &'a [Span],
    pub selected_span_index: Option<usize>,
    pub total_duration: f64,
    pub start_time: f64,
}

impl<'a> Widget for Flamegraph<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let time_per_col = self.total_duration / area.width as f64;
        let mut subcell_tracker: HashMap<(u16, u16), (f64, SubcellAlign, Color)> = HashMap::new();

        let graph = GraphSpan {
            spans: self.spans,
            time_per_col,
            flamegraph_area: area,
            start_time: self.start_time,
        };

        for (i, span) in self.spans.iter().enumerate() {
            if span.contained_by_index.is_some() {
                continue;
            }
            let sf = (span.start_time - self.start_time) / time_per_col;
            let ef = (span.start_time + span.duration - self.start_time) / time_per_col;
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
                self.selected_span_index,
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
            // Catppuccin Peach (warm pastel orange)
            SpanType::Unit => (250, 179, 135),

            // Catppuccin Sapphire (Soft Pastel Blue)
            SpanType::Source => (116, 199, 236),

            // Catppuccin Mauve (Soft Pastel Purple)
            SpanType::Class => (203, 166, 247),

            // Catppuccin Yellow (Soft Pastel Yellow)
            SpanType::Template => (249, 226, 175),

            // Catppuccin Subtext0 (light muted gray)
            SpanType::Task => (172, 176, 190),
        }
    }

    /// Calculates a distinct shade based on 2D position
    pub fn get_checkerboard_color(&self, horizontal_index: usize) -> Color {
        let (r0, g0, b0) = self.base_rgb();
        let hsl = Rgb::from(r0 as f32, g0 as f32, b0 as f32).to_hsl();

        // 1. Vertical variation: desaturate odd depth rows
        let hue = if horizontal_index % 2 != 0 {
            (hsl.get_hue()).clamp(0.0, 359.0)
        } else {
            (hsl.get_hue() + 10.0).clamp(0.0, 359.0)
        };

        // 2. Horizontal variation: shift lightness for odd siblings
        let lightness = if self.depth % 2 != 0 {
            (hsl.get_lightness()).clamp(0.0, 100.0)
        } else {
            (hsl.get_lightness() - 10.0).clamp(0.0, 100.0)
        };

        let rgb = Hsl::from(hue, hsl.get_saturation(), lightness).to_rgb();
        Color::Rgb(
            rgb.get_red() as u8,
            rgb.get_green() as u8,
            rgb.get_blue() as u8,
        )
    }
}
