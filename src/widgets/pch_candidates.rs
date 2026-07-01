use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Modifier, Style},
    widgets::{Block, Borders, Widget},
};

use crate::widgets::time_range::format_time;

/// A single PCH candidate entry.
#[derive(Clone, Debug)]
pub struct PchCandidate {
    /// File identifier (path).
    pub identifier: String,
    /// Cleaned label for display.
    pub label: String,
    /// Total duration across all TUs.
    pub total_duration: f64,
    /// Number of occurrences (TUs this file appears in).
    pub count: usize,
    /// Average time per occurrence.
    pub avg_duration: f64,
}

impl PchCandidate {
    pub fn new(identifier: String, label: String, total_duration: f64, count: usize) -> Self {
        let avg_duration = if count > 0 { total_duration / count as f64 } else { 0.0 };
        Self { identifier, label, total_duration, count, avg_duration }
    }
}

/// Renders a scrollable list of PCH candidates.
/// Each candidate occupies 2 rows:
///   row 0: file name (left-aligned, truncated to fit)
///   row 1: total + count + avg
pub struct PchCandidatesWidget<'a> {
    pub candidates: &'a [PchCandidate],
    /// Number of *candidates* scrolled past (not rows).
    pub scroll_offset: u16,
    pub selected_index: Option<usize>,
    /// When true, the copy button shows a green "✓ Copied" state.
    pub copy_confirmed: bool,
}

impl<'a> PchCandidatesWidget<'a> {
    pub const HEADER_HEIGHT: u16 = 2;
    pub const CANDIDATE_ROWS: u16 = 2;

    pub fn hit_copy_button(&self, area: Rect, col: u16, row: u16) -> bool {
        let button_width: u16 = if self.copy_confirmed { 12 } else { 18 };
        let btn_x = area.x + area.width.saturating_sub(button_width + 1);
        let btn_y = area.y + 1;
        col >= btn_x && col < area.x + area.width.saturating_sub(1) && row == btn_y
    }

    pub fn build_includes(&self) -> String {
        self.candidates
            .iter()
            .take(10)
            .map(|c| {
                let path = &c.identifier;
                let display = &c.label;
                if path.starts_with('/') && (path.contains("/usr/") || path.contains("/include/")) {
                    format!("#include <{}>", display)
                } else {
                    format!("#include \"{}\"", display)
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Widget for PchCandidatesWidget<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width < 10 || area.height < 3 {
            return;
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let inner = block.inner(area);
        block.render(area, buf);

        if inner.width == 0 || inner.height == 0 {
            return;
        }

        // --- Title bar with copy button pinned right ---
        buf.set_string(
            inner.x,
            inner.y,
            "PCH Candidates",
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD),
        );

        let (btn_text, btn_bg) = if self.copy_confirmed {
            (" ✓ Copied ", Color::Green)
        } else {
            (" 📋  qCopy #includes ", Color::Blue)
        };
        let btn_width = btn_text.len() as u16;
        let btn_x = inner.x + inner.width.saturating_sub(btn_width);
        let btn_style = Style::default().fg(Color::Black).bg(btn_bg);
        if btn_x > inner.x {
            for (i, ch) in btn_text.chars().enumerate() {
                buf.set_string(btn_x + i as u16, inner.y, ch.to_string(), btn_style);
            }
        }

        // --- Separator ---
        if inner.height >= 2 {
            let sep_y = inner.y + 1;
            let sep = "─".repeat(inner.width as usize);
            buf.set_string(inner.x, sep_y, sep, Style::default().fg(Color::DarkGray));
        }

        // --- Candidate rows (2 rows each) ---
        let list_top = inner.y + Self::HEADER_HEIGHT;
        let list_height = inner.height.saturating_sub(Self::HEADER_HEIGHT);
        if list_height < Self::CANDIDATE_ROWS {
            return;
        }

        let visible_count = (list_height / Self::CANDIDATE_ROWS) as usize;
        let skip = self.scroll_offset as usize;

        for (row_i, (cand_idx, cand)) in self
            .candidates
            .iter()
            .enumerate()
            .skip(skip)
            .take(visible_count)
            .enumerate()
        {
            let y0 = list_top + row_i as u16 * Self::CANDIDATE_ROWS;
            let y1 = y0 + 1;
            if y1 >= inner.bottom() {
                break;
            }

            let is_selected = self.selected_index == Some(cand_idx);
            let (row_bg, row_fg) = if is_selected {
                (Color::DarkGray, Color::White)
            } else if row_i % 2 == 0 {
                (Color::Reset, Color::White)
            } else {
                (Color::Black, Color::White)
            };

            // Clear both rows background
            for y in [y0, y1] {
                for x in inner.x..inner.x + inner.width {
                    buf.set_string(x, y, " ", Style::default().bg(row_bg));
                }
            }

            // --- Row 0: file name ---
            let max_name_w = inner.width as usize;
            let file_name = if cand.label.len() > max_name_w {
                format!(
                    "…{}",
                    &cand.label[cand.label.len().saturating_sub(max_name_w.saturating_sub(1))..]
                )
            } else {
                cand.label.clone()
            };
            buf.set_string(inner.x, y0, &file_name, Style::default().fg(row_fg).bg(row_bg));

            // --- Row 1: total count avg (colored, 1 space, no overflow) ---
            let mut parts: Vec<(String, Color)> = Vec::new();
            parts.push((format_time(cand.total_duration), Color::Green));
            parts.push((format!("×{}", cand.count), Color::Yellow));
            parts.push((format!("avg {}", format_time(cand.avg_duration)), Color::Gray));

            let mut line = String::new();
            for (i, (text, _color)) in parts.iter().enumerate() {
                if i > 0 {
                    line.push(' ');
                }
                line.push_str(text);
            }
            let max_w = inner.width as usize;
            if line.len() > max_w {
                line.truncate(max_w);
            }

            // Render with per-word coloring
            let mut cx = inner.x;
            for (text, color) in &parts {
                if cx >= inner.x + inner.width {
                    break;
                }
                let style = Style::default().fg(*color).bg(row_bg);
                buf.set_string(cx, y1, text, style);
                cx += text.len() as u16 + 1; // +1 for the space between
            }
        }
    }
}
