use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Color, Style},
    widgets::Widget,
};

use crate::app::span::Span;
use crate::app::view::SpanView;
use crate::widgets::span::{SpanWidget, SubcellAlign, flush_subcell_tracker};

/// Returns the number of content rows needed to display a track's spans.
pub fn track_content_height(
    views: &[SpanView],
    spans: &[Span],
    visible_duration: f64,
    area_width: u16,
) -> u16 {
    if area_width == 0 {
        return 1;
    }
    let cell_duration = visible_duration / area_width as f64;
    views
        .iter()
        .filter_map(|v| {
            let s = spans.get(v.span_index)?;
            (s.duration > cell_duration).then_some(s.depth)
        })
        .max()
        .map(|d| d as u16 + 1)
        .unwrap_or(1)
}

/// A track's view data ready for the flamegraph renderer.
pub struct TrackInput<'a> {
    pub label: Option<String>,
    pub views: &'a mut [SpanView],
    /// Pre-computed intrinsic height: label row (if any) + content rows.
    pub intrinsic_height: u16,
}

/// Renders a single track: optional label row, then all spans via UnitWidget.
pub struct TrackWidget<'a> {
    pub label: Option<&'a str>,
    /// Global flat spans array (read-only during render).
    pub spans: &'a [Span],
    /// All SpanViews for this track; render state (was_displayed etc.) is written back here.
    pub views: &'a mut [SpanView],
    pub total_duration: f64,
    pub start_time: f64,
    /// Rows to skip from the top of this track (for partial-track scrolling).
    pub row_skip: u16,
    pub selected_span: Option<usize>,
    /// Terminal cell (col, row) → global span index.
    pub cell_map: &'a mut HashMap<(u16, u16), usize>,
    pub search_query: Option<&'a str>,
}

impl<'a> Widget for TrackWidget<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        if area.height == 0 {
            return;
        }

        let has_selected = self
            .selected_span
            .map(|si| self.views.iter().any(|v| v.span_index == si))
            .unwrap_or(false);

        let label_rows: u16 = if self.label.is_some() { 1 } else { 0 };
        let (content_area, unit_row_skip) = if self.row_skip < label_rows {
            let y = area.y;
            let (bg, fg) = if has_selected {
                (Color::Rgb(67, 69, 88), Color::Rgb(148, 152, 170))
            } else {
                (Color::Rgb(49, 50, 68), Color::Rgb(108, 111, 133))
            };
            let label_style = Style::default().fg(fg).bg(bg);
            let blank: String = std::iter::repeat(' ').take(area.width as usize).collect();
            buf.set_string(area.x, y, &blank, label_style);
            let label_text = format!(" {} ", self.label.unwrap_or(""));
            buf.set_stringn(area.x, y, &label_text, area.width as usize, label_style);
            let content_y = area.y + label_rows;
            let content_h = area.height.saturating_sub(label_rows);
            (Rect::new(area.x, content_y, area.width, content_h), 0u16)
        } else {
            (area, self.row_skip - label_rows)
        };

        render_spans(
            content_area,
            buf,
            self.spans,
            self.views,
            self.selected_span,
            self.total_duration,
            self.start_time,
            unit_row_skip,
            self.cell_map,
            self.search_query,
        );
    }
}

fn render_spans(
    area: Rect,
    buf: &mut Buffer,
    spans: &[Span],
    views: &mut [SpanView],
    selected_span: Option<usize>,
    total_duration: f64,
    start_time: f64,
    row_skip: u16,
    cell_map: &mut HashMap<(u16, u16), usize>,
    search_query: Option<&str>,
) {
    if area.width == 0 || total_duration <= 0.0 {
        return;
    }

    let time_per_col = total_duration / area.width as f64;
    let mut subcell_tracker: HashMap<(u16, u16), (f64, SubcellAlign, Color, usize)> =
        HashMap::new();
    let mut core_bounds: HashMap<usize, (u16, u16)> = HashMap::new();
    // Maps parent_span_index (usize::MAX for roots) → count of visible siblings so far.
    let mut sibling_visual_counter: HashMap<usize, usize> = HashMap::new();

    for view in views.iter_mut() {
        view.has_core_cells = false;
        view.was_displayed = false;
    }

    for view_idx in 0..views.len() {
        let span_index = views[view_idx].span_index;
        let effective_start = views[view_idx].effective_start;
        let index_in_parent = views[view_idx].index_in_parent;

        let span = &spans[span_index];

        let clamp: (u16, u16) = if let Some(pi) = span.parent_index {
            match core_bounds.get(&pi).copied() {
                Some(b) => b,
                None => continue,
            }
        } else {
            (area.x, area.right())
        };

        let depth = span.depth;

        let sf = (effective_start - start_time) / time_per_col;
        let ef = (effective_start + span.duration - start_time) / time_per_col;
        let x_start = (area.x as i32 + sf.round() as i32)
            .max(clamp.0 as i32)
            .min(clamp.1 as i32) as u16;
        let x_end = (area.x as i32 + ef.round() as i32)
            .max(clamp.0 as i32)
            .min(clamp.1 as i32) as u16;
        let width = x_end.saturating_sub(x_start);

        if (depth as u16) < row_skip {
            if width > 0 {
                core_bounds.insert(span_index, (x_start, x_end));
            }
            continue;
        }

        let y = area.y + depth as u16 - row_skip;
        if y >= area.bottom() {
            continue;
        }

        // Assign a visual sibling index that only counts rendered (width > 0) spans,
        // so invisible spans don't cause same-color adjacencies.
        let visual_index = if width > 0 {
            let parent_key = span.parent_index.unwrap_or(usize::MAX);
            let counter = sibling_visual_counter.entry(parent_key).or_insert(0);
            let idx = *counter;
            *counter += 1;
            idx
        } else {
            index_in_parent
        };

        let allowed_area = Rect::new(x_start, y, width, 1);

        let widget = SpanWidget {
            span,
            span_index,
            index_in_parent: visual_index,
            display_area: area,
            allowed_area,
            time_per_col,
            start_time,
            effective_start,
            selected_span_index: selected_span,
            search_query,
        };

        let span_core_bounds = widget.render_with_tracker(buf, &mut subcell_tracker);

        if let Some((cx_start, cx_end)) = span_core_bounds {
            for x in cx_start..cx_end {
                cell_map.insert((x, y), span_index);
            }
            core_bounds.insert(span_index, (cx_start, cx_end));
        }
        views[view_idx].has_core_cells = span_core_bounds.is_some();
    }

    let subcell_winners = flush_subcell_tracker(buf, &subcell_tracker);
    let winner_indices: std::collections::HashSet<usize> =
        subcell_winners.values().copied().collect();
    for view in views.iter_mut() {
        if view.has_core_cells || winner_indices.contains(&view.span_index) {
            view.was_displayed = true;
        }
    }
    cell_map.extend(subcell_winners.into_iter().map(|(k, si)| (k, si)));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::span::{Span, SpanType};

    fn make_span(duration: f64, depth: usize) -> Span {
        Span {
            type_: SpanType::Source,
            identifier: String::new(),
            label: String::new(),
            sublabel: None,
            start_time: 0.0,
            duration,
            parent_index: None,
            children_indices: Vec::new(),
            root_span_index: 0,
            depth,
        }
    }

    fn make_view(span_index: usize) -> SpanView {
        SpanView {
            span_index,
            effective_start: 0.0,
            index_in_parent: 0,
            has_core_cells: false,
            was_displayed: false,
        }
    }

    // ── track_content_height ──

    #[test]
    fn test_content_height_zero_width() {
        let views: [SpanView; 0] = [];
        let spans: [Span; 0] = [];
        assert_eq!(track_content_height(&views, &spans, 100.0, 0), 1);
    }

    #[test]
    fn test_content_height_no_views() {
        let views: [SpanView; 0] = [];
        let spans: [Span; 0] = [];
        assert_eq!(track_content_height(&views, &spans, 100.0, 10), 1);
    }

    #[test]
    fn test_content_height_all_too_thin() {
        // cell_duration = 100.0 / 10 = 10.0µs, span duration 1.0 < 10.0 → filtered out
        let spans = vec![make_span(1.0, 3)];
        let views = vec![make_view(0)];
        assert_eq!(track_content_height(&views, &spans, 100.0, 10), 1);
    }

    #[test]
    fn test_content_height_one_visible() {
        // cell_duration = 10.0, span duration 100 > 10 → visible, depth 3 → height 4
        let spans = vec![make_span(100.0, 3)];
        let views = vec![make_view(0)];
        assert_eq!(track_content_height(&views, &spans, 100.0, 10), 4);
    }

    #[test]
    fn test_content_height_max_of_several() {
        // cell_duration = 10.0, all durations > 10 → all visible
        let spans = vec![
            make_span(100.0, 1),
            make_span(100.0, 2),
            make_span(100.0, 5),
        ];
        let views: Vec<SpanView> = (0..3).map(make_view).collect();
        // max depth = 5 → height = 6
        assert_eq!(track_content_height(&views, &spans, 100.0, 10), 6);
    }

    #[test]
    fn test_content_height_some_thin_some_thick() {
        // cell_duration = 10.0
        let spans = vec![
            make_span(1.0, 4),   // too thin, ignored
            make_span(100.0, 2), // visible, depth 2
            make_span(5.0, 10),  // too thin, ignored even though depth is high
            make_span(50.0, 3),  // visible, depth 3
        ];
        let views: Vec<SpanView> = (0..4).map(make_view).collect();
        // max visible depth = 3 → height = 4
        assert_eq!(track_content_height(&views, &spans, 100.0, 10), 4);
    }
}
