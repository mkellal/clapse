use std::collections::HashMap;

use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};

use crate::app::span::Span;
use crate::app::unit::SpanView;
use crate::widgets::span::{SpanWidget, SubcellAlign, flush_subcell_tracker};

pub struct UnitWidget<'a> {
    pub spans: &'a mut [Span],
    pub views: &'a [SpanView],
    pub selected_span_index: Option<usize>,
    pub total_duration: f64,
    pub start_time: f64,
    pub unit_index: usize,
    /// Number of depth rows to skip from the top (for vertical scrolling).
    /// Spans at depth < row_skip are not rendered but their x-bounds are still
    /// computed so their children can use them for clamping.
    pub row_skip: u16,
    // terminal cell (col, row) -> (unit_index, span_index).
    pub cell_map: &'a mut HashMap<(u16, u16), (usize, usize)>,
}

impl<'a> Widget for UnitWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || self.total_duration <= 0.0 {
            return;
        }

        let time_per_col = self.total_duration / area.width as f64;
        let mut subcell_tracker: HashMap<(u16, u16), (f64, SubcellAlign, Color, usize)> =
            HashMap::new();

        // Per-span core bounds indexed by original span index.
        let mut core_bounds: Vec<Option<(u16, u16)>> = vec![None; self.spans.len()];

        // Reset render state for all spans before iterating.
        for span in self.spans.iter_mut() {
            span.has_core_cells = false;
            span.was_displayed = false;
        }

        for entry in self.views {
            let i = entry.span_index;
            let span = &self.spans[i];

            // Determine the x-clamp range from the parent's core bounds.
            // Skip this span entirely if the parent had no core cells.
            let clamp: (u16, u16) = if let Some(pi) = span.parent_index {
                match core_bounds[pi] {
                    Some(b) => b,
                    None => continue,
                }
            } else {
                (area.x, area.right())
            };

            let depth = span.depth;

            // Compute x extent for every span, even those above the viewport,
            // so that their children can use the bounds for clamping.
            let sf = (entry.effective_start - self.start_time) / time_per_col;
            let ef = (entry.effective_start + span.duration - self.start_time) / time_per_col;
            let x_start = (area.x as i32 + sf.round() as i32)
                .max(clamp.0 as i32)
                .min(clamp.1 as i32) as u16;
            let x_end = (area.x as i32 + ef.round() as i32)
                .max(clamp.0 as i32)
                .min(clamp.1 as i32) as u16;
            let width = x_end.saturating_sub(x_start);

            // Spans above the viewport (due to row_skip): propagate bounds but don't render.
            if (depth as u16) < self.row_skip {
                core_bounds[i] = if width > 0 { Some((x_start, x_end)) } else { None };
                continue;
            }

            let y = area.y + depth as u16 - self.row_skip;
            if y >= area.bottom() {
                continue;
            }

            let allowed_area = Rect::new(x_start, y, width, 1);

            let widget = SpanWidget {
                span,
                span_index: i,
                index_in_parent: entry.index_in_parent,
                display_area: area,
                allowed_area,
                time_per_col,
                start_time: self.start_time,
                effective_start: entry.effective_start,
                selected_span_index: self.selected_span_index,
            };

            let span_core_bounds = widget.render_with_tracker(buf, &mut subcell_tracker);

            if let Some((cx_start, cx_end)) = span_core_bounds {
                for x in cx_start..cx_end {
                    self.cell_map.insert((x, y), (self.unit_index, i));
                }
            }
            self.spans[i].has_core_cells = span_core_bounds.is_some();
            core_bounds[i] = span_core_bounds;
        }

        // Settle subcell claims.
        let subcell_winners = flush_subcell_tracker(buf, &subcell_tracker);
        for i in 0..self.spans.len() {
            if self.spans[i].has_core_cells || subcell_winners.values().any(|&si| si == i) {
                self.spans[i].was_displayed = true;
            }
        }
        let ui = self.unit_index;
        self.cell_map
            .extend(subcell_winners.into_iter().map(|(k, si)| (k, (ui, si))));
    }
}
