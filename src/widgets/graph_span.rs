use std::collections::HashMap;

use ratatui::{buffer::Buffer, layout::Rect, style::{Color, Style}};

use crate::app::span::Span;

#[derive(Clone, Copy)]
pub enum SubcellAlign {
    Left,
    Right,
}

pub struct GraphSpan<'a> {
    pub spans: &'a [Span],
    pub time_per_col: f64,
    pub flamegraph_area: Rect,
    pub start_time: f64,
}

impl<'a> GraphSpan<'a> {
    pub fn render_span(
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
                    let cs = (child.start_time - self.start_time) / self.time_per_col;
                    let ce = (child.start_time + child.duration - self.start_time) / self.time_per_col;
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
