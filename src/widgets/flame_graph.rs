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

struct GraphSpan<'a> {
    spans: &'a [Span],
    time_per_col: f64,
    flamegraph_area: Rect,
}

impl<'a> GraphSpan<'a> {
    fn render_span(
        &self,
        span_idx: usize,
        sibling_index: usize,
        allowed_area: Rect,
        buf: &mut Buffer,
        subcell_tracker: &mut HashMap<(u16, u16), (f64, SubcellAlign, Color)>,
    ) {
        if allowed_area.width == 0 {
            return;
        }

        let span = &self.spans[span_idx];
        let y = allowed_area.y;
        let bg_color = span.get_checkerboard_color(sibling_index);
        let fa = self.flamegraph_area;

        let start_float = span.start_time / self.time_per_col;
        let end_float = (span.start_time + span.duration) / self.time_per_col;
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
            return;
        }

        if prefrac < 1.0 {
            try_claim(subcell_tracker, start_col, prefrac, SubcellAlign::Right);
        }

        let core_x_start = (fa.x as i32 + start_float.ceil() as i32)
            .max(allowed_area.x as i32)
            .min(allowed_area.right() as i32) as u16;
        let core_x_end = (fa.x as i32 + end_float.floor() as i32)
            .max(allowed_area.x as i32)
            .min(allowed_area.right() as i32) as u16;
        let core_width = core_x_end.saturating_sub(core_x_start);

        if core_width > 0 {
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
            buf.set_stringn(
                core_x_start,
                y,
                &display_text,
                w,
                Style::default().fg(fg_color).bg(bg_color),
            );

            let child_y = y + 1;
            if child_y < fa.bottom() {
                for (sibling_i, &child_idx) in span.contains_indices.iter().enumerate() {
                    let child = &self.spans[child_idx];
                    let cs = child.start_time / self.time_per_col;
                    let ce = (child.start_time + child.duration) / self.time_per_col;
                    let cx_start = (fa.x as i32 + cs.round() as i32)
                        .max(core_rect.x as i32)
                        .min(core_rect.right() as i32) as u16;
                    let cx_end = (fa.x as i32 + ce.round() as i32)
                        .max(core_rect.x as i32)
                        .min(core_rect.right() as i32) as u16;
                    let cw = cx_end.saturating_sub(cx_start);
                    if cw == 0 {
                        continue;
                    }
                    self.render_span(
                        child_idx,
                        sibling_i,
                        Rect::new(cx_start, child_y, cw, 1),
                        buf,
                        subcell_tracker,
                    );
                }
            }
        }

        if postfrac > 0.0 {
            try_claim(subcell_tracker, end_col, postfrac, SubcellAlign::Left);
        }
    }
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
