use std::collections::HashMap;

use colors_transform::{Color as _, Hsl, Rgb};
use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

use crate::app::span::Span;

/// Splits `label` into `(section_text, following_delimiter)` pairs.
/// Delimiters are `::` and `/`.
fn split_sections(label: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut remaining = label;
    loop {
        let slash = remaining.find('/').map(|p| (p, 1usize));
        let colons = remaining.find("::").map(|p| (p, 2usize));
        let next = match (slash, colons) {
            (Some(s), Some(c)) => Some(if s.0 <= c.0 { s } else { c }),
            (Some(s), None) => Some(s),
            (None, Some(c)) => Some(c),
            (None, None) => None,
        };
        match next {
            Some((pos, len)) => {
                result.push((
                    remaining[..pos].to_string(),
                    remaining[pos..pos + len].to_string(),
                ));
                remaining = &remaining[pos + len..];
            }
            None => {
                result.push((remaining.to_string(), String::new()));
                break;
            }
        }
    }
    result
}

/// Shorten `label` to fit in `w` characters.
///
/// Removes leading sections (delimited by `::` or `/`) from the front until
/// the remainder fits, replacing the last removed section with as many of its
/// characters as possible followed by `…`.
fn shorten_label(label: &str, w: usize) -> String {
    if label.chars().count() <= w {
        return label.to_string();
    }
    if w == 0 {
        return String::new();
    }

    let sections = split_sections(label);

    if sections.len() == 1 {
        return label.chars().take(w.saturating_sub(1)).collect::<String>() + "…";
    }

    for remove_up_to in 1..=sections.len() {
        let suffix: String = sections[remove_up_to..]
            .iter()
            .flat_map(|(s, d)| [s.as_str(), d.as_str()])
            .collect();
        let last_removed = &sections[remove_up_to - 1];
        let connector = &last_removed.1;
        // Minimum: "…" + connector + suffix (zero prefix characters)
        let min_len = 1 + connector.chars().count() + suffix.chars().count();
        if min_len <= w {
            let available = w - min_len;
            let prefix: String = last_removed.0.chars().take(available).collect();
            return format!("{}…{}{}", prefix, connector, suffix);
        }
    }

    // Unreachable: when remove_up_to == sections.len(), min_len == 1 <= w
    "…".to_string()
}

#[derive(Clone, Copy)]
pub enum SubcellAlign {
    Left,
    Right,
}

pub struct SpanWidget<'a> {
    pub span: &'a Span,
    pub span_index: usize,
    pub index_in_parent: usize,
    pub display_area: Rect,
    pub allowed_area: Rect,
    pub time_per_col: f64,
    pub start_time: f64,
    /// Effective start time for positioning. Equals span.start_time in StartTime mode;
    /// overridden to a virtual position in Duration mode.
    pub effective_start: f64,
    pub selected_span_index: Option<usize>,
}

impl<'a> SpanWidget<'a> {
    pub fn render_with_tracker(
        self,
        buf: &mut Buffer,
        subcell_tracker: &mut HashMap<(u16, u16), (f64, SubcellAlign, Color, usize)>,
    ) -> Option<(u16, u16)> {
        if self.allowed_area.width == 0 {
            return None;
        }

        let span = self.span;
        let is_selected = Some(self.span_index) == self.selected_span_index;
        let y = self.allowed_area.y;
        let bg_color = if is_selected {
            Color::Rgb(255, 255, 255)
        } else {
            span.get_checkerboard_color(self.index_in_parent)
        };
        let area = self.display_area;

        let start_float = (self.effective_start - self.start_time) / self.time_per_col;
        let end_float = (self.effective_start + span.duration - self.start_time) / self.time_per_col;
        let start_col = start_float.floor() as i32;
        let end_col = end_float.floor() as i32;
        let startfrac = start_float.fract();
        let prefrac = 1.0 - startfrac;
        let postfrac = end_float.fract();

        let span_index = self.span_index;
        let try_claim = |tracker: &mut HashMap<(u16, u16), (f64, SubcellAlign, Color, usize)>,
                         col: i32,
                         fraction: f64,
                         align: SubcellAlign| {
            let x = area.x as i32 + col;
            if x >= area.x as i32 && x < area.right() as i32 {
                let coord = (x as u16, y);
                let current = tracker.get(&coord).map(|(f, _, _, _)| *f).unwrap_or(0.0);
                if fraction > current {
                    tracker.insert(coord, (fraction, align, bg_color, span_index));
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
            return None;
        }

        if prefrac < 1.0 {
            try_claim(subcell_tracker, start_col, prefrac, SubcellAlign::Right);
        }

        let core_x_start = (area.x as i32 + start_float.ceil() as i32)
            .max(self.allowed_area.x as i32)
            .min(self.allowed_area.right() as i32) as u16;
        let core_x_end = (area.x as i32 + end_float.floor() as i32)
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
            let display_text = if w == 1 {
                "𝅏".to_string()
            } else {
                shorten_label(&span.label, w)
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

        core_bounds
    }
}

impl<'a> Widget for SpanWidget<'a> {
    fn render(self, _area: Rect, buf: &mut Buffer) {
        let mut tracker = HashMap::new();
        self.render_with_tracker(buf, &mut tracker);
        flush_subcell_tracker(buf, &tracker);
    }
}

/// Flush the subcell tracker into the buffer and return a map of cell coordinates
/// to the span index that won that cell.
pub fn flush_subcell_tracker(
    buf: &mut Buffer,
    tracker: &HashMap<(u16, u16), (f64, SubcellAlign, Color, usize)>,
) -> HashMap<(u16, u16), usize> {
    let mut winners: HashMap<(u16, u16), usize> = HashMap::new();
    for ((x, y), (fraction, align, color, span_index)) in tracker {
        winners.insert((*x, *y), *span_index);
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
    winners
}

impl Span {
    pub fn get_checkerboard_color(&self, horizontal_index: usize) -> Color {
        let base = self.type_.base_color();
        let (r0, g0, b0) = match base {
            Color::Rgb(r, g, b) => (r, g, b),
            _ => (128, 128, 128),
        };
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
