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

/// Returns the number of content rows needed to display the units in `thread_units`:
/// `max(span.depth) + 1` across all spans in the thread (minimum 1).
pub fn thread_content_height(thread_units: &[usize], units: &[Unit]) -> u16 {
    thread_units
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

/// A track represents a thread in unit scheduling.
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
            let label_area = Rect::new(area.x, area.y, area.width, 1);
            buf.set_stringn(
                label_area.x,
                label_area.y,
                label,
                label_area.width as usize,
                Style::default().fg(Color::DarkGray),
            );
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
