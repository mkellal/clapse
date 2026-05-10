use std::collections::HashMap;

use ratatui::{buffer::Buffer, layout::Rect, widgets::Widget};

use super::track::{TrackWidget, UnitEntry};

/// One track's data ready to pass to the flamegraph renderer.
pub struct TrackInput<'a> {
    pub label: Option<String>,
    pub units: Vec<UnitEntry<'a>>,
    /// Pre-computed intrinsic height: label row (if any) + content rows.
    pub intrinsic_height: u16,
}

/// Renders a vertically-scrollable list of [`TrackWidget`]s.
///
/// Each track occupies its full intrinsic height. The viewport shows
/// `area.height` rows starting at `scroll_offset` rows from the top of the
/// virtual canvas. Tracks that are partially scrolled are clipped cleanly via
/// `row_skip`.
pub struct FlamegraphWidget<'a> {
    pub tracks: Vec<TrackInput<'a>>,
    pub total_duration: f64,
    pub start_time: f64,
    /// Vertical scroll offset in rows from the top of the virtual canvas.
    pub scroll_offset: u16,
    /// Shared cell map: terminal cell (col, row) → (unit_index, span_index).
    pub cell_map: &'a mut HashMap<(u16, u16), (usize, usize)>,
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

            // Advance cursor before any early-continue.
            virtual_y = track_bottom;

            // Skip tracks fully above the viewport.
            if track_bottom <= viewport_top {
                continue;
            }
            // Stop once we're past the viewport.
            if track_top >= viewport_bottom {
                break;
            }

            // Overlap: [overlap_start, overlap_end) in virtual coordinates.
            let overlap_start = track_top.max(viewport_top);
            let overlap_end = track_bottom.min(viewport_bottom);
            let visible_rows = overlap_end - overlap_start;

            // How many rows of this track are above the viewport top.
            let row_skip = overlap_start - track_top;
            // Terminal y where this track's visible portion starts.
            let render_y = area.y + (overlap_start - viewport_top);

            let track_area = Rect::new(area.x, render_y, area.width, visible_rows);
            let label = track.label.as_deref();

            TrackWidget {
                label,
                units: track.units,
                total_duration: self.total_duration,
                start_time: self.start_time,
                row_skip,
                cell_map: self.cell_map,
            }
            .render(track_area, buf);
        }
    }
}
