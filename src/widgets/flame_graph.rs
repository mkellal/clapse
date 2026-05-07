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
        let mut cell_owners: Vec<Vec<(u16, u16)>> = vec![Vec::new(); self.spans.len()];

        let try_claim = |tracker: &mut HashMap<(u16, u16), (f64, SubcellAlign, Color)>,
                         owners: &mut Vec<Vec<(u16, u16)>>,
                         span_idx: usize,
                         col: i32,
                         y: u16,
                         fraction: f64,
                         align: SubcellAlign,
                         color: Color| {
            let x = area.x as i32 + col;
            if x >= area.x as i32 && x < area.right() as i32 {
                let cell_coord = (x as u16, y);
                let current = tracker.get(&cell_coord).map(|(f, _, _)| *f).unwrap_or(0.0);
                let container_index = self.spans[span_idx].contained_by_index;
                let is_parent_top_cell_owner = if let Some(idx) = container_index
                    && y > area.y
                {
                    owners[idx].iter().any(|&coord| coord == (x as u16, y - 1))
                } else {
                    true
                };
                let parent_align = container_index.and_then(|_idx| {
                    tracker.get(&(x as u16, y - 1)).map(|(_, align, _)| *align)
                });
                if is_parent_top_cell_owner && fraction > current {
                    tracker.insert(cell_coord, (fraction, parent_align.unwrap_or(align), if is_parent_top_cell_owner { color } else { Color::Red }));
                    owners[span_idx].push(cell_coord);
                }
            }
        };

        for (i, span) in self.spans.iter().enumerate() {
            let row = area.y + span.depth as u16;

            if row >= area.bottom() {
                continue;
            }

            let bg_color = span.get_checkerboard_color(i);
            let exact_width_cols = span.duration / time_per_col;
            let start_float = span.start_time / time_per_col;
            let end_float = start_float + exact_width_cols;

            let start_col = start_float.floor() as i32;
            let end_col = end_float.floor() as i32;
            let startfrac = start_float.fract();
            let prefrac = 1.0 - startfrac;
            let postfrac = end_float.fract();

            if start_col == end_col {
                let align = if startfrac > 0.5 {
                    SubcellAlign::Right
                } else {
                    SubcellAlign::Left
                };
                try_claim(
                    &mut subcell_tracker,
                    &mut cell_owners,
                    i,
                    start_col,
                    row,
                    exact_width_cols,
                    align,
                    bg_color,
                );
                continue;
            }

            // Pre-fraction
            if prefrac < 1.0 {
                try_claim(
                    &mut subcell_tracker,
                    &mut cell_owners,
                    i,
                    start_col,
                    row,
                    prefrac,
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

                        let rect = Rect::new(visible_start, row, width, 1);
                        buf.set_style(rect, Style::default().bg(bg_color));

                        for x in visible_start..visible_end {
                            cell_owners[i].push((x, row));
                        }

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
            if postfrac > 0.0 {
                try_claim(
                    &mut subcell_tracker,
                    &mut cell_owners,
                    i,
                    end_col,
                    row,
                    postfrac,
                    SubcellAlign::Left,
                    bg_color,
                );
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
