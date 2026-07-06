use std::collections::HashMap;

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
    pub search_query: Option<&'a str>,
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
        let is_match = self.search_query.is_some() && {
            let q = self.search_query.unwrap().to_lowercase();
            span.identifier.to_lowercase().contains(&q)
                || span.label.to_lowercase().contains(&q)
                || span
                    .sublabel
                    .as_ref()
                    .map(|sl| sl.to_lowercase().contains(&q))
                    .unwrap_or(false)
        };

        let y = self.allowed_area.y;
        let bg_color = if is_match && is_selected {
            Color::LightGreen
        } else if is_match {
            Color::Green
        } else if is_selected {
            Color::Rgb(255, 255, 255)
        } else {
            span.type_
                .get_color(Some(self.index_in_parent), Some(span.depth))
        };
        let area = self.display_area;

        let start_float = (self.effective_start - self.start_time) / self.time_per_col;
        let end_float =
            (self.effective_start + span.duration - self.start_time) / self.time_per_col;
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
            let fg_color = if is_match && is_selected {
                Color::Black
            } else {
                match bg_color {
                    Color::DarkGray => Color::Rgb(255, 255, 255),
                    _ => Color::Black,
                }
            };

            let core_rect = Rect::new(core_x_start, y, core_width, 1);
            buf.set_style(core_rect, Style::default().bg(bg_color));

            let w = core_width as usize;
            let display_text = if w == 1 {
                "𝅏".to_string()
            } else {
                shorten_label(&span.label, w)
            };
            let text_style = Style::default().fg(fg_color).bg(bg_color);
            let text_style = if is_selected {
                text_style.bold()
            } else {
                text_style
            };
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    // ── split_sections ──

    #[test]
    fn test_split_sections_slashes() {
        let result = split_sections("a/b/c");
        assert_eq!(
            result,
            vec![
                ("a".to_string(), "/".to_string()),
                ("b".to_string(), "/".to_string()),
                ("c".to_string(), "".to_string()),
            ]
        );
    }

    #[test]
    fn test_split_sections_namespaces() {
        let result = split_sections("std::vector");
        assert_eq!(
            result,
            vec![
                ("std".to_string(), "::".to_string()),
                ("vector".to_string(), "".to_string()),
            ]
        );
    }

    #[test]
    fn test_split_sections_mixed() {
        let result = split_sections("a/b::c/d");
        assert_eq!(
            result,
            vec![
                ("a".to_string(), "/".to_string()),
                ("b".to_string(), "::".to_string()),
                ("c".to_string(), "/".to_string()),
                ("d".to_string(), "".to_string()),
            ]
        );
    }

    #[test]
    fn test_split_sections_no_delimiter() {
        let result = split_sections("foo");
        assert_eq!(result, vec![("foo".to_string(), "".to_string())]);
    }

    #[test]
    fn test_split_sections_empty() {
        let result = split_sections("");
        assert_eq!(result, vec![("".to_string(), "".to_string())]);
    }

    #[test]
    fn test_split_sections_starts_with_delim() {
        let result = split_sections("::foo");
        assert_eq!(
            result,
            vec![
                ("".to_string(), "::".to_string()),
                ("foo".to_string(), "".to_string()),
            ]
        );
    }

    #[test]
    fn test_split_sections_consecutive_delims() {
        let result = split_sections("a//b");
        assert_eq!(
            result,
            vec![
                ("a".to_string(), "/".to_string()),
                ("".to_string(), "/".to_string()),
                ("b".to_string(), "".to_string()),
            ]
        );
    }

    // ── shorten_label ──

    #[test]
    fn test_shorten_fits() {
        assert_eq!(shorten_label("hello", 10), "hello");
    }

    #[test]
    fn test_shorten_exact_fit() {
        assert_eq!(shorten_label("hello", 5), "hello");
    }

    #[test]
    fn test_shorten_w_zero() {
        assert_eq!(shorten_label("hello", 0), "");
    }

    #[test]
    fn test_shorten_single_section() {
        // "verylong" = 8 chars, w=4 → "ver…"
        assert_eq!(shorten_label("verylong", 4), "ver…");
    }

    #[test]
    fn test_shorten_multi_section_remove_first() {
        // "a/b/c/d/e" = 9 chars → sections: a,/,b,/,c,/,d,/,e
        // w=6: remove "a/", then "b/" still too long, then "c/" fits with 1 prefix char
        // → "c…/d/e"
        assert_eq!(shorten_label("a/b/c/d/e", 6), "c…/d/e");
    }

    #[test]
    fn test_shorten_too_narrow() {
        // w=1, even "…" alone is 1 char, but min_len might push past
        // For "a/b/c": sections [("a","/"), ("b","/"), ("c","")]
        // remove_up_to=3: suffix="", min_len=1 → w=1, available=0
        // → "…" (just ellipsis)
        assert_eq!(shorten_label("a/b/c", 1), "…");
    }

    #[test]
    fn test_shorten_long_path() {
        // Realistic case: long path, enough room for last 2 components
        let label = "lib/core/src/utils/string_helpers.cpp";
        let result = shorten_label(label, 20);
        assert!(result.chars().count() <= 20);
        assert!(result.ends_with("string_helpers.cpp"));
        assert!(result.contains('…'));
    }

    // ── flush_subcell_tracker ──

    fn empty_buffer(w: u16, h: u16) -> Buffer {
        Buffer::empty(Rect::new(0, 0, w, h))
    }

    #[test]
    fn test_flush_empty_tracker() {
        let mut buf = empty_buffer(10, 5);
        let tracker: HashMap<(u16, u16), (f64, SubcellAlign, Color, usize)> = HashMap::new();
        let winners = flush_subcell_tracker(&mut buf, &tracker);
        assert!(winners.is_empty());
    }

    #[test]
    fn test_flush_full_block_right() {
        let mut buf = empty_buffer(10, 5);
        let mut tracker = HashMap::new();
        tracker.insert((2, 1), (0.8, SubcellAlign::Right, Color::Red, 42));
        let winners = flush_subcell_tracker(&mut buf, &tracker);

        assert_eq!(winners.get(&(2, 1)), Some(&42));
        // Fraction 0.8 with Right align → "█"
        let cell = buf.cell((2, 1)).unwrap();
        assert_eq!(cell.symbol(), "█");
    }

    #[test]
    fn test_flush_thin_left() {
        let mut buf = empty_buffer(10, 5);
        let mut tracker = HashMap::new();
        tracker.insert((0, 0), (0.12, SubcellAlign::Left, Color::Blue, 7));
        flush_subcell_tracker(&mut buf, &tracker);

        let cell = buf.cell((0, 0)).unwrap();
        assert_eq!(cell.symbol(), "▏");
    }

    #[test]
    fn test_flush_higher_fraction_wins() {
        let mut buf = empty_buffer(10, 5);
        let mut tracker = HashMap::new();
        // Two entries for same cell — higher fraction should win
        // This mimics the try_claim logic: only insert if fraction > current
        tracker.insert((3, 2), (0.9, SubcellAlign::Left, Color::Green, 100));
        // Won't override because flush doesn't do try_claim — it just flushes whatever is in tracker
        // Actual try_claim prevents lower from entering; we just test flush renders correctly
        let winners = flush_subcell_tracker(&mut buf, &tracker);

        assert_eq!(winners.get(&(3, 2)), Some(&100));
        let cell = buf.cell((3, 2)).unwrap();
        assert_eq!(cell.symbol(), "█");
    }

    #[test]
    fn test_flush_various_fractions() {
        let mut buf = empty_buffer(20, 3);
        let mut tracker = HashMap::new();
        // Left-align fractions at various thresholds
        tracker.insert((0, 0), (0.06, SubcellAlign::Left, Color::White, 1)); // ▏ ( <0.125)
        tracker.insert((1, 0), (0.2, SubcellAlign::Left, Color::White, 2)); // ▎ (0.125-0.25)
        tracker.insert((2, 0), (0.3, SubcellAlign::Left, Color::White, 3)); // ▍ (0.25-0.375)
        tracker.insert((3, 0), (0.45, SubcellAlign::Left, Color::White, 4)); // ▌ (0.375-0.5)
        tracker.insert((4, 0), (0.55, SubcellAlign::Left, Color::White, 5)); // ▋ (0.5-0.625)
        tracker.insert((5, 0), (0.7, SubcellAlign::Left, Color::White, 6)); // ▊ (0.625-0.75)
        tracker.insert((6, 0), (0.85, SubcellAlign::Left, Color::White, 7)); // ▉ (0.75-0.875)
        tracker.insert((7, 0), (0.95, SubcellAlign::Left, Color::White, 8)); // █ (>=0.875)
        flush_subcell_tracker(&mut buf, &tracker);

        assert_eq!(buf.cell((0, 0)).unwrap().symbol(), "▏");
        assert_eq!(buf.cell((1, 0)).unwrap().symbol(), "▎");
        assert_eq!(buf.cell((2, 0)).unwrap().symbol(), "▍");
        assert_eq!(buf.cell((3, 0)).unwrap().symbol(), "▌");
        assert_eq!(buf.cell((4, 0)).unwrap().symbol(), "▋");
        assert_eq!(buf.cell((5, 0)).unwrap().symbol(), "▊");
        assert_eq!(buf.cell((6, 0)).unwrap().symbol(), "▉");
        assert_eq!(buf.cell((7, 0)).unwrap().symbol(), "█");
    }
}
