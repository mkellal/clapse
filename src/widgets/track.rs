use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

use crate::app::span::Span;
use crate::app::unit::SpanView;

use super::unit::UnitWidget;

/// Pre-resolved spans and views for a single unit to display within a track.
pub struct UnitEntry<'a> {
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

        for (unit_index, entry) in self.units.into_iter().enumerate() {
            UnitWidget {
                spans: entry.spans,
                views: entry.views,
                selected_span_index: entry.selected_span_index,
                total_duration: self.total_duration,
                start_time: self.start_time,
                unit_index,
                cell_map: self.cell_map,
            }
            .render(content_area, buf);
        }
    }
}
