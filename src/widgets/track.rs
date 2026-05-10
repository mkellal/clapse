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

/// Returns the number of content rows needed to display the units in `track_units`:
/// `max(span.depth) + 1` across all spans in the track (minimum 1).
pub fn track_content_height(track_units: &[usize], units: &[Unit]) -> u16 {
    track_units
        .iter()
        .filter_map(|&ui| units.get(ui))
        .flat_map(|u| u.spans.iter())
        .map(|s| s.depth)
        .max()
        .map(|d| d as u16 + 1)
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
    /// Shared cell map: terminal cell (col, row) → (unit_index, span_index).
    pub cell_map: &'a mut HashMap<(u16, u16), (usize, usize)>,
}

impl<'a> Widget for TrackWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }

        let content_area = if let Some(label) = self.label {
            let y = area.y;
            let muted = Style::default().fg(Color::DarkGray);
            // "─ Label ─────────────────"
            // Left padding: one mid-line char + space before the label.
            let prefix = "─ ";
            let suffix_char = '─';
            let label_with_space = format!("{} ", label);
            let prefix_len = prefix.chars().count() as u16;
            let label_len = label_with_space.chars().count() as u16;
            let used = prefix_len + label_len;
            let suffix_len = area.width.saturating_sub(used);

            buf.set_string(area.x, y, prefix, muted);
            buf.set_stringn(
                area.x + prefix_len,
                y,
                &label_with_space,
                label_len as usize,
                muted,
            );
            let suffix: String = std::iter::repeat(suffix_char)
                .take(suffix_len as usize)
                .collect();
            buf.set_string(area.x + prefix_len + label_len, y, &suffix, muted);

            Rect::new(area.x, area.y + 1, area.width, area.height.saturating_sub(1))
        } else {
            area
        };

        for entry in self.units.into_iter() {
            UnitWidget {
                spans: entry.spans,
                views: entry.views,
                selected_span_index: entry.selected_span_index,
                total_duration: self.total_duration,
                start_time: self.start_time,
                unit_index: entry.unit_index,
                cell_map: self.cell_map,
            }
            .render(content_area, buf);
        }
    }
}
