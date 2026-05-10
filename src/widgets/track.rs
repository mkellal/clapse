use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

use crate::app::span::Span;
use crate::app::unit::{SpanView, Unit};

use super::unit::UnitWidget;

/// Returns the number of content rows needed to display the track.
/// Height is based on the deepest span in the full track, not the current viewport.
pub fn track_content_height(
    track_units: &[usize],
    units: &[Unit],
    _start_time: f64,
    visible_duration: f64,
    area_width: u16,
) -> u16 {
    let cell_duration = visible_duration / area_width as f64;
    track_units
        .iter()
        .filter_map(|&ui| units.get(ui))
        .map(|u| {
            u.spans
                .iter()
                .filter_map(|s| (s.duration > cell_duration).then_some(s.depth))
                .max()
                .unwrap_or(0) as u16
                + 1
        })
        .max()
        .unwrap_or(1)
}

/// Pre-resolved spans and views for a single unit to display within a track.
pub struct UnitEntry<'a> {
    /// Global index of this unit in `App::units` — used as the `unit_index`
    /// key stored in the cell map so mouse clicks resolve correctly.
    pub unit_index: usize,
    pub spans: &'a mut [Span],
    pub views: &'a [SpanView],
    pub selected_span_index: Option<usize>,
}

/// A track represents a track in unit scheduling.
///
/// It optionally shows a muted label on its first row, then renders each unit
/// using [`UnitWidget`] across the remaining area.
pub struct TrackWidget<'a> {
    pub label: Option<&'a str>,
    pub units: Vec<UnitEntry<'a>>,
    pub total_duration: f64,
    pub start_time: f64,
    /// Rows to skip from the top of this track (for partial-track scrolling).
    pub row_skip: u16,
    /// Shared cell map: terminal cell (col, row) → (unit_index, span_index).
    pub cell_map: &'a mut HashMap<(u16, u16), (usize, usize)>,
}

impl<'a> Widget for TrackWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }

        let has_selected_unit = self
            .units
            .iter()
            .any(|entry| entry.selected_span_index.is_some());

        let label_rows: u16 = if self.label.is_some() { 1 } else { 0 };
        let (content_area, unit_row_skip) = if self.row_skip < label_rows {
            // Label row is visible (row_skip == 0 since label_rows <= 1).
            let y = area.y;
            let label_style = if has_selected_unit {
                Style::default().fg(Color::White)
            } else {
                Style::default().fg(Color::DarkGray)
            };
            let prefix = "─ ";
            let suffix_char = '─';
            let label_with_space = format!("{} ", self.label.unwrap_or(""));
            let prefix_len = prefix.chars().count() as u16;
            let label_len = label_with_space.chars().count() as u16;
            let used = prefix_len + label_len;
            let suffix_len = area.width.saturating_sub(used);
            buf.set_string(area.x, y, prefix, label_style);
            buf.set_stringn(
                area.x + prefix_len,
                y,
                &label_with_space,
                label_len as usize,
                label_style,
            );
            let suffix: String = std::iter::repeat(suffix_char)
                .take(suffix_len as usize)
                .collect();
            buf.set_string(area.x + prefix_len + label_len, y, &suffix, label_style);
            let content_y = area.y + label_rows;
            let content_h = area.height.saturating_sub(label_rows);
            (Rect::new(area.x, content_y, area.width, content_h), 0u16)
        } else {
            // Label is scrolled off; pass remaining skip to UnitWidget.
            (area, self.row_skip - label_rows)
        };

        for entry in self.units.into_iter() {
            UnitWidget {
                spans: entry.spans,
                views: entry.views,
                selected_span_index: entry.selected_span_index,
                total_duration: self.total_duration,
                start_time: self.start_time,
                unit_index: entry.unit_index,
                row_skip: unit_row_skip,
                cell_map: self.cell_map,
            }
            .render(content_area, buf);
        }
    }
}
