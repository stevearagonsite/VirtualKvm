//! Monochrome, high-contrast tray icon, rasterized in code (no image files).
//!
//! Glyph on a 22×22 grid: two monitors side by side and a two-way arrow below.
//! - on:  filled monitors + arrow
//! - off: monitor outlines only
//!
//! Styles:
//! - [`Style::Template`] (macOS): black + alpha. macOS recolors template images for
//!   light / dark menu bars and "Increase contrast".
//! - [`Style::HighContrast`] (Windows): white glyph with a black outline, readable on
//!   both light and dark taskbars.
//!
//! `assets/vkvm-on.svg` and `assets/vkvm-off.svg` are the same glyph as vector files.

const GRID: f32 = 22.0;
const SUPERSAMPLE: u32 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)] // each platform uses one style
pub enum Style {
    Template,
    HighContrast,
}

enum Shape {
    /// filled rectangle (x0, y0, x1, y1)
    Fill(f32, f32, f32, f32),
    /// rectangle outline with stroke width
    Stroke(f32, f32, f32, f32, f32),
    /// line segment with round caps (x0, y0, x1, y1, width)
    Line(f32, f32, f32, f32, f32),
}

impl Shape {
    fn covers(&self, x: f32, y: f32) -> bool {
        match *self {
            Shape::Fill(x0, y0, x1, y1) => x >= x0 && x <= x1 && y >= y0 && y <= y1,
            Shape::Stroke(x0, y0, x1, y1, w) => {
                let h = w / 2.0;
                let outer = x >= x0 - h && x <= x1 + h && y >= y0 - h && y <= y1 + h;
                let inner = x > x0 + h && x < x1 - h && y > y0 + h && y < y1 - h;
                outer && !inner
            }
            Shape::Line(x0, y0, x1, y1, w) => {
                let (dx, dy) = (x1 - x0, y1 - y0);
                let len2 = dx * dx + dy * dy;
                let t = if len2 == 0.0 {
                    0.0
                } else {
                    (((x - x0) * dx + (y - y0) * dy) / len2).clamp(0.0, 1.0)
                };
                let (px, py) = (x0 + t * dx - x, y0 + t * dy - y);
                px * px + py * py <= (w / 2.0) * (w / 2.0)
            }
        }
    }
}

fn shapes(on: bool) -> Vec<Shape> {
    let mut s = Vec::new();
    // monitors (left, right), stands and bases
    for x in [1.0f32, 12.0] {
        if on {
            s.push(Shape::Fill(x, 3.0, x + 9.0, 11.0));
        } else {
            s.push(Shape::Stroke(x + 0.75, 3.75, x + 8.25, 10.25, 1.5));
        }
        s.push(Shape::Fill(x + 3.75, 11.0, x + 5.25, 13.5));
        s.push(Shape::Fill(x + 2.0, 13.5, x + 7.0, 14.75));
    }
    if on {
        // two-way arrow
        let y = 18.5;
        s.push(Shape::Line(5.0, y, 17.0, y, 1.6));
        s.push(Shape::Line(5.0, y, 7.6, y - 2.2, 1.6));
        s.push(Shape::Line(5.0, y, 7.6, y + 2.2, 1.6));
        s.push(Shape::Line(17.0, y, 14.4, y - 2.2, 1.6));
        s.push(Shape::Line(17.0, y, 14.4, y + 2.2, 1.6));
    }
    s
}

/// coverage mask (0.0..=1.0) of the glyph at `size`×`size` pixels
fn mask(on: bool, size: u32) -> Vec<f32> {
    let shapes = shapes(on);
    let n = size * SUPERSAMPLE;
    let scale = GRID / n as f32;
    let mut out = vec![0f32; (size * size) as usize];
    for py in 0..size {
        for px in 0..size {
            let mut hits = 0u32;
            for sy in 0..SUPERSAMPLE {
                for sx in 0..SUPERSAMPLE {
                    let x = ((px * SUPERSAMPLE + sx) as f32 + 0.5) * scale;
                    let y = ((py * SUPERSAMPLE + sy) as f32 + 0.5) * scale;
                    if shapes.iter().any(|s| s.covers(x, y)) {
                        hits += 1;
                    }
                }
            }
            out[(py * size + px) as usize] = hits as f32 / (SUPERSAMPLE * SUPERSAMPLE) as f32;
        }
    }
    out
}

/// RGBA pixels of the icon
pub fn rgba(on: bool, size: u32, style: Style) -> Vec<u8> {
    let m = mask(on, size);
    let at = |x: i32, y: i32| -> f32 {
        if x < 0 || y < 0 || x >= size as i32 || y >= size as i32 {
            0.0
        } else {
            m[(y as u32 * size + x as u32) as usize]
        }
    };
    // outline thickness in pixels for the high contrast style
    let r = (size as i32 / 32).max(1);
    let mut px = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size as i32 {
        for x in 0..size as i32 {
            let a = at(x, y);
            match style {
                Style::Template => px.extend_from_slice(&[0, 0, 0, (a * 255.0).round() as u8]),
                Style::HighContrast => {
                    let mut dilated: f32 = 0.0;
                    for dy in -r..=r {
                        for dx in -r..=r {
                            dilated = dilated.max(at(x + dx, y + dy));
                        }
                    }
                    // white glyph over a black halo
                    let v = (a * 255.0).round() as u8;
                    px.extend_from_slice(&[v, v, v, (dilated * 255.0).round() as u8]);
                }
            }
        }
    }
    px
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn on_and_off_differ_and_have_ink() {
        let on = mask(true, 32);
        let off = mask(false, 32);
        let ink = |m: &[f32]| m.iter().sum::<f32>();
        assert!(ink(&on) > ink(&off));
        assert!(ink(&off) > 10.0);
        assert_eq!(rgba(true, 44, Style::Template).len(), 44 * 44 * 4);
        assert_eq!(rgba(false, 32, Style::HighContrast).len(), 32 * 32 * 4);
    }
}
