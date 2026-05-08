use std::collections::HashMap;

use ratatui::{buffer::Buffer, layout::Rect, style::Color, widgets::Widget};

use crate::app::span::Span;
use crate::widgets::span::{SpanWidget, SubcellAlign, flush_subcell_tracker};

pub struct UnitWidget<'a> {
    pub spans: &'a mut [Span],
    pub selected_span_index: Option<usize>,
    pub total_duration: f64,
    pub start_time: f64,
    // terminal cell (col, row) -> span index.
    pub cell_map: &'a mut HashMap<(u16, u16), usize>,
}

impl<'a> Widget for UnitWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.width == 0 || self.total_duration <= 0.0 {
            return;
        }

        let time_per_col = self.total_duration / area.width as f64;
        let mut subcell_tracker: HashMap<(u16, u16), (f64, SubcellAlign, Color, usize)> =
            HashMap::new();

        // Per-span core bounds (core_x_start, core_x_end) used to clamp children.
        // A span's children are only rendered if this entry is Some.
        let mut core_bounds: Vec<Option<(u16, u16)>> = vec![None; self.spans.len()];

        for i in 0..self.spans.len() {
            // Reset render state from the previous frame
            self.spans[i].has_core_cells = false;
            self.spans[i].was_displayed = false;

            // Determine the x-clamp range from the parent's core bounds.
            // Skip this span entirely if the parent had no core cells.
            let clamp: (u16, u16) = if let Some(pi) = self.spans[i].contained_by_index {
                match core_bounds[pi] {
                    Some(b) => b,
                    None => continue, // parent had no core cells
                }
            } else {
                // Root span: unclamped
                (area.x, area.right())
            };

            let depth = self.spans[i].depth;
            let y = area.y + depth as u16;
            if y >= area.bottom() {
                continue;
            }

            let sf = (self.spans[i].start_time - self.start_time) / time_per_col;
            let ef = (self.spans[i].start_time + self.spans[i].duration - self.start_time)
                / time_per_col;
            let x_start = (area.x as i32 + sf.round() as i32)
                .max(clamp.0 as i32)
                .min(clamp.1 as i32) as u16;
            let x_end = (area.x as i32 + ef.round() as i32)
                .max(clamp.0 as i32)
                .min(clamp.1 as i32) as u16;
            let width = x_end.saturating_sub(x_start);

            let allowed_area = Rect::new(x_start, y, width, 1);

            // Sibling position for checkerboard coloring
            let index_in_depth: usize = if let Some(pi) = self.spans[i].contained_by_index {
                self.spans[pi]
                    .contains_indices
                    .iter()
                    .position(|&ci| ci == i)
                    .unwrap_or(0)
            } else {
                0
            };

            let widget = SpanWidget {
                span: &self.spans[i],
                span_index: i,
                index_in_depth,
                flamegraph_area: area,
                allowed_area,
                time_per_col,
                start_time: self.start_time,
                selected_span_index: self.selected_span_index,
            };

            let span_core_bounds = widget.render_with_tracker(buf, &mut subcell_tracker);

            // Record core cells in the cell map immediately.
            if let Some((cx_start, cx_end)) = span_core_bounds {
                for x in cx_start..cx_end {
                    self.cell_map.insert((x, y), i);
                }
            }
            self.spans[i].has_core_cells = span_core_bounds.is_some();
            core_bounds[i] = span_core_bounds;
        }

        // Settle subcell claims: only spans that actually won a cell are marked displayed.
        let subcell_winners = flush_subcell_tracker(buf, &subcell_tracker);
        for i in 0..self.spans.len() {
            if self.spans[i].has_core_cells || subcell_winners.values().any(|&si| si == i) {
                self.spans[i].was_displayed = true;
            }
        }
        // Merge subcell winners into cell map.
        self.cell_map.extend(subcell_winners);
    }
}
