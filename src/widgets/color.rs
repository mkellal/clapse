use colors_transform::{Color as _, Hsl, Rgb};
use ratatui::style::Color;

/// Derive a display color from a `base` color, the span's `depth` row,
/// and its `horizontal_index` among siblings.
///
/// Odd depth → slightly lighter; odd horizontal index → slight hue shift.
/// This is the single source of truth used by both span widgets and unit root spans.
pub fn span_color(base: Color, depth: usize, horizontal_index: usize) -> Color {
    let (r0, g0, b0) = match base {
        Color::Rgb(r, g, b) => (r, g, b),
        _ => (128, 128, 128),
    };
    let hsl = Rgb::from(r0 as f32, g0 as f32, b0 as f32).to_hsl();

    let hue = if horizontal_index % 2 != 0 {
        hsl.get_hue().clamp(0.0, 359.0)
    } else {
        (hsl.get_hue() + 10.0).clamp(0.0, 359.0)
    };

    let lightness = if depth % 2 != 0 {
        hsl.get_lightness().clamp(0.0, 100.0)
    } else {
        (hsl.get_lightness() - 10.0).clamp(0.0, 100.0)
    };

    let rgb = Hsl::from(hue, hsl.get_saturation(), lightness).to_rgb();
    Color::Rgb(
        rgb.get_red() as u8,
        rgb.get_green() as u8,
        rgb.get_blue() as u8,
    )
}
