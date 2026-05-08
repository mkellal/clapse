use std::collections::HashMap;

use colors_transform::{Color as _, Hsl, Rgb};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

use crate::app::span::{Span, SpanType};

#[derive(Clone, Copy)]
pub enum SubcellAlign {
    Left,
    Right,
}

pub struct SpanWidget<'a> {
    pub span: &'a Span,
    pub span_index: usize,
    pub index_in_depth: usize,
    pub flamegraph_area: Rect,
    pub allowed_area: Rect,
    pub time_per_col: f64,
    pub start_time: f64,
    pub selected_span_index: Option<usize>,
}

impl<'a> SpanWidget<'a> {
    pub fn render_with_tracker(
        self,
        buf: &mut Buffer,
        subcell_tracker: &mut HashMap<(u16, u16), (f64, SubcellAlign, Color)>,
    ) -> (bool, Option<(u16, u16)>) {
        if self.allowed_area.width == 0 {
            return (false, None);
        }

        let span = self.span;
        let is_selected = Some(self.span_index) == self.selected_span_index;
        let y = self.allowed_area.y;
        let bg_color = if is_selected {
            Color::Rgb(255, 255, 255)
        } else {
            span.get_checkerboard_color(self.index_in_depth)
        };
        let fa = self.flamegraph_area;

        let start_float = (span.start_time - self.start_time) / self.time_per_col;
        let end_float = (span.start_time + span.duration - self.start_time) / self.time_per_col;
        let start_col = start_float.floor() as i32;
        let end_col = end_float.floor() as i32;
        let startfrac = start_float.fract();
        let prefrac = 1.0 - startfrac;
        let postfrac = end_float.fract();

        let try_claim = |tracker: &mut HashMap<(u16, u16), (f64, SubcellAlign, Color)>,
                         col: i32,
                         fraction: f64,
                         align: SubcellAlign| {
            let x = fa.x as i32 + col;
            if x >= fa.x as i32 && x < fa.right() as i32 {
                let coord = (x as u16, y);
                let current = tracker.get(&coord).map(|(f, _, _)| *f).unwrap_or(0.0);
                if fraction > current {
                    tracker.insert(coord, (fraction, align, bg_color));
                }
            }
        };

        if start_col == end_col {
            let exact = end_float - start_float;
            let align = if startfrac > 0.5 {
                SubcellAlign::Right
            } else {
                SubcellAlign::Left
            };
            try_claim(subcell_tracker, start_col, exact, align);
            return (true, None);
        }

        if prefrac < 1.0 {
            try_claim(subcell_tracker, start_col, prefrac, SubcellAlign::Right);
        }

        let core_x_start = (fa.x as i32 + start_float.ceil() as i32)
            .max(self.allowed_area.x as i32)
            .min(self.allowed_area.right() as i32) as u16;
        let core_x_end = (fa.x as i32 + end_float.floor() as i32)
            .max(self.allowed_area.x as i32)
            .min(self.allowed_area.right() as i32) as u16;
        let core_width = core_x_end.saturating_sub(core_x_start);

        let core_bounds = if core_width > 0 {
            let fg_color = match bg_color {
                Color::DarkGray => Color::White,
                _ => Color::Black,
            };

            let core_rect = Rect::new(core_x_start, y, core_width, 1);
            buf.set_style(core_rect, Style::default().bg(bg_color));

            let w = core_width as usize;
            let label_len = span.label.chars().count();
            let display_text = if w == 1 {
                "𝅏".to_string()
            } else if label_len > w {
                span.label.chars().take(w - 1).collect::<String>() + "…"
            } else {
                span.label.clone()
            };
            let mut text_style = Style::default().fg(fg_color).bg(bg_color);
            if is_selected {
                text_style = text_style.bold();
            }
            buf.set_stringn(core_x_start, y, &display_text, w, text_style);

            Some((core_x_start, core_x_end))
        } else {
            None
        };

        if postfrac > 0.0 {
            try_claim(subcell_tracker, end_col, postfrac, SubcellAlign::Left);
        }

        (true, core_bounds)
    }
}

impl<'a> Widget for SpanWidget<'a> {
    fn render(self, _area: Rect, buf: &mut Buffer) {
        let mut tracker = HashMap::new();
        self.render_with_tracker(buf, &mut tracker);
        flush_subcell_tracker(buf, &tracker);
    }
}

pub fn flush_subcell_tracker(
    buf: &mut Buffer,
    tracker: &HashMap<(u16, u16), (f64, SubcellAlign, Color)>,
) {
    for ((x, y), (fraction, align, color)) in tracker {
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

impl Span {
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

    pub fn get_checkerboard_color(&self, horizontal_index: usize) -> Color {
        let (r0, g0, b0) = self.base_rgb();
        let hsl = Rgb::from(r0 as f32, g0 as f32, b0 as f32).to_hsl();

        // Horizontal variation: slight hue shift for odd siblings
        let hue = if horizontal_index % 2 != 0 {
            hsl.get_hue().clamp(0.0, 359.0)
        } else {
            (hsl.get_hue() + 10.0).clamp(0.0, 359.0)
        };

        // Vertical variation: lightness shift for odd depth rows
        let lightness = if self.depth % 2 != 0 {
            hsl.get_lightness().clamp(0.0, 100.0)
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
