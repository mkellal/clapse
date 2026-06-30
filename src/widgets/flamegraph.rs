use std::collections::HashMap;

use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

use crate::app::span::Span;
use super::track::{TrackInput, TrackWidget};

/// Renders a vertically-scrollable list of [`TrackWidget`]s.
pub struct FlamegraphWidget<'a> {
    pub tracks: Vec<TrackInput<'a>>,
    /// Global flat spans array (read-only; shared refs are Copy).
    pub spans: &'a [Span],
    pub total_duration: f64,
    pub start_time: f64,
    /// Vertical scroll offset in rows from the top of the virtual canvas.
    pub scroll_offset: u16,
    /// Terminal cell (col, row) → global span index.
    pub cell_map: &'a mut HashMap<(u16, u16), usize>,
    pub selected_span: Option<usize>,
    pub search_query: Option<&'a str>,
}

impl FlamegraphWidget<'_> {
    /// Total virtual height of all tracks combined.
    pub fn total_height(tracks: &[TrackInput<'_>]) -> u16 {
        tracks.iter().map(|t| t.intrinsic_height).sum()
    }
}

impl<'a> Widget for FlamegraphWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 || area.width == 0 {
            return;
        }

        let viewport_top = self.scroll_offset;
        let viewport_bottom = self.scroll_offset.saturating_add(area.height);

        let mut virtual_y: u16 = 0;

        for track in self.tracks {
            let track_top = virtual_y;
            let track_bottom = virtual_y.saturating_add(track.intrinsic_height);

            virtual_y = track_bottom;

            if track_bottom <= viewport_top {
                continue;
            }
            if track_top >= viewport_bottom {
                break;
            }

            let overlap_start = track_top.max(viewport_top);
            let overlap_end = track_bottom.min(viewport_bottom);
            let visible_rows = overlap_end - overlap_start;

            let row_skip = overlap_start - track_top;
            let render_y = area.y + (overlap_start - viewport_top);

            let track_area = Rect::new(area.x, render_y, area.width, visible_rows);
            let label = track.label.as_deref();

            TrackWidget {
                label,
                spans: self.spans,
                views: track.views,
                total_duration: self.total_duration,
                start_time: self.start_time,
                row_skip,
                selected_span: self.selected_span,
                cell_map: self.cell_map,
                search_query: self.search_query,
            }
            .render(track_area, buf);
        }
    }
}
