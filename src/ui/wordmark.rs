//! The "nebula" wordmark in the title bar, as vector outlines (traced from
//! the Agency FB letters, font units, y up) rasterized at the exact pixel
//! size so it stays sharp at every display scale.

use eframe::egui::{self, Color32, Pos2, Rect, TextureHandle, pos2, vec2};

enum Seg {
    M(f32, f32),
    L(f32, f32),
    Q(f32, f32, f32, f32),
    Z,
}
use Seg::*;

/// (advance width, outline) per letter.
const GLYPHS: [(f32, &[Seg]); 6] = [
    // n
    (774.0, &[M(668.0, 0.0), L(535.0, 0.0), L(535.0, 854.0), L(242.0, 846.0), L(242.0, 0.0), L(109.0, 0.0), L(109.0, 973.0), L(242.0, 973.0), L(242.0, 946.0), Q(320.0, 956.0, 399.0, 967.0), Q(495.0, 979.0, 557.0, 979.0), Q(668.0, 979.0, 668.0, 870.0), L(668.0, 0.0), Z]),
    // e
    (764.0, &[M(653.0, 113.0), Q(653.0, 0.0, 541.0, 0.0), L(221.0, 0.0), Q(109.0, 0.0, 109.0, 113.0), L(109.0, 860.0), Q(109.0, 973.0, 221.0, 973.0), L(541.0, 973.0), Q(653.0, 973.0, 653.0, 860.0), L(653.0, 510.0), L(608.0, 463.0), L(238.0, 463.0), L(238.0, 111.0), L(524.0, 111.0), L(524.0, 289.0), L(653.0, 289.0), L(653.0, 113.0), Z, M(524.0, 569.0), L(524.0, 862.0), L(238.0, 862.0), L(238.0, 569.0), L(524.0, 569.0), Z]),
    // b
    (776.0, &[M(668.0, 209.0), Q(668.0, 119.0, 609.0, 59.5), Q(550.0, 0.0, 461.0, 0.0), L(109.0, 0.0), L(109.0, 1565.0), L(242.0, 1565.0), L(242.0, 946.0), Q(321.0, 956.0, 400.0, 966.0), Q(500.0, 979.0, 557.0, 979.0), Q(668.0, 979.0, 668.0, 870.0), L(668.0, 209.0), Z, M(535.0, 229.0), L(535.0, 854.0), L(242.0, 846.0), L(242.0, 117.0), L(426.0, 117.0), Q(535.0, 117.0, 535.0, 229.0), Z]),
    // u
    (774.0, &[M(666.0, 0.0), L(535.0, 0.0), L(535.0, 27.0), Q(455.0, 16.0, 376.0, 6.0), Q(280.0, -6.0, 217.0, -6.0), Q(109.0, -6.0, 109.0, 102.0), L(109.0, 973.0), L(240.0, 973.0), L(240.0, 119.0), L(535.0, 127.0), L(535.0, 973.0), L(666.0, 973.0), L(666.0, 0.0), Z]),
    // l
    (365.0, &[M(248.0, 0.0), L(117.0, 0.0), L(117.0, 1565.0), L(248.0, 1565.0), L(248.0, 0.0), Z]),
    // a
    (768.0, &[M(659.0, 0.0), L(528.0, 0.0), L(528.0, 27.0), Q(449.0, 16.0, 370.0, 6.0), Q(272.0, -6.0, 213.0, -6.0), Q(104.0, -6.0, 104.0, 102.0), L(104.0, 457.0), Q(104.0, 569.0, 217.0, 569.0), L(530.0, 569.0), L(530.0, 862.0), L(244.0, 862.0), L(244.0, 696.0), L(115.0, 696.0), L(115.0, 860.0), Q(115.0, 973.0, 227.0, 973.0), L(547.0, 973.0), Q(659.0, 973.0, 659.0, 860.0), L(659.0, 0.0), Z, M(530.0, 121.0), L(530.0, 465.0), L(233.0, 465.0), L(233.0, 115.0), L(530.0, 121.0), Z]),
];

/// Extra space between letters, in font units (the spaced-out look).
const TRACKING: f32 = 550.0;
/// Height of `b` and `l` in font units; the wordmark's height.
const ASCENT: f32 = 1565.0;
const DESCENT: f32 = 6.0;

/// Closed polygons of the word in font units, curves flattened.
fn polygons() -> Vec<Vec<Pos2>> {
    let mut out = Vec::new();
    let mut x0 = -(GLYPHS[0].1.iter().filter_map(|s| if let M(x, _) | L(x, _) = s { Some(*x) } else { None }).fold(f32::MAX, f32::min));
    for (advance, segs) in GLYPHS {
        let mut poly: Vec<Pos2> = Vec::new();
        for s in segs {
            match *s {
                M(x, y) => poly = vec![pos2(x0 + x, y)],
                L(x, y) => poly.push(pos2(x0 + x, y)),
                Q(cx, cy, x, y) => {
                    let p0 = *poly.last().unwrap();
                    let (c, p1) = (pos2(x0 + cx, cy), pos2(x0 + x, y));
                    for k in 1..=8 {
                        let t = k as f32 / 8.0;
                        let u = 1.0 - t;
                        poly.push(pos2(u * u * p0.x + 2.0 * u * t * c.x + t * t * p1.x, u * u * p0.y + 2.0 * u * t * c.y + t * t * p1.y));
                    }
                }
                Z => out.push(std::mem::take(&mut poly)),
            }
        }
        x0 += advance + TRACKING;
    }
    out
}

/// Coverage mask (0–255) of the word `height` pixels tall; returns (width, mask).
fn rasterize(height: usize) -> (usize, Vec<u8>) {
    let polys = polygons();
    let right = polys.iter().flatten().map(|p| p.x).fold(0.0, f32::max);
    let scale = height as f32 / (ASCENT + DESCENT);
    let width = (right * scale).ceil() as usize + 1;
    const SS: usize = 4;
    let mut acc = vec![0u16; width * height];
    let edges: Vec<(Pos2, Pos2)> = polys
        .iter()
        .flat_map(|p| p.iter().zip(p.iter().cycle().skip(1)).map(|(a, b)| (*a, *b)))
        // To pixels, y down.
        .map(|(a, b)| (pos2(a.x * scale, (ASCENT - a.y) * scale), pos2(b.x * scale, (ASCENT - b.y) * scale)))
        .collect();
    let mut crossings: Vec<(f32, i32)> = Vec::new();
    for row in 0..height * SS {
        let y = (row as f32 + 0.5) / SS as f32;
        crossings.clear();
        for (a, b) in &edges {
            if (a.y <= y) != (b.y <= y) {
                let x = a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x);
                crossings.push((x, if b.y > a.y { 1 } else { -1 }));
            }
        }
        crossings.sort_by(|a, b| a.0.total_cmp(&b.0));
        // Non-zero winding: fill between crossings while the count is not 0.
        let mut wind = 0;
        for w in crossings.windows(2) {
            wind += w[0].1;
            if wind == 0 {
                continue;
            }
            let (from, to) = ((w[0].0 * SS as f32).round() as i64, (w[1].0 * SS as f32).round() as i64);
            for sx in from.max(0)..to.min((width * SS) as i64) {
                acc[(row / SS) * width + sx as usize / SS] += 1;
            }
        }
    }
    let max = (SS * SS) as u16;
    (width, acc.into_iter().map(|c| (c.min(max) as u32 * 255 / max as u32) as u8).collect())
}

/// Draws the wordmark centered at `center`, `height` points tall, but not
/// left of `min_left` (keeps it clear of the logo in narrow sidebars).
/// `cache` keeps one texture per pixel height (title bar, welcome page).
pub fn paint(painter: &egui::Painter, cache: &mut Vec<(usize, TextureHandle)>, center: Pos2, min_left: f32, height: f32, color: Color32) {
    let ppp = painter.ctx().pixels_per_point();
    let px = (height * ppp).round().max(4.0) as usize;
    let tex = match cache.iter().find(|(h, _)| *h == px) {
        Some((_, tex)) => tex.clone(),
        None => {
            let (w, mask) = rasterize(px);
            let image = egui::ColorImage::from_rgba_unmultiplied([w, px], &mask.iter().flat_map(|a| [255, 255, 255, *a]).collect::<Vec<_>>());
            let tex = painter.ctx().load_texture("wordmark", image, egui::TextureOptions::LINEAR);
            // Old sizes (after a change of display scale) are dropped.
            if cache.len() >= 4 {
                let _ = cache.remove(0);
            }
            cache.push((px, tex.clone()));
            tex
        }
    };
    let size = vec2(tex.size()[0] as f32, tex.size()[1] as f32) / ppp;
    // Snap to whole pixels so the letters stay crisp.
    let mut min = center - size / 2.0;
    min.x = min.x.max(min_left);
    let min = (min * ppp).round() / ppp;
    painter.image(tex.id(), Rect::from_min_size(min, size), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), color);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_are_filled_with_holes() {
        let (w, mask) = rasterize(40);
        assert!(w > 100);
        let at = |x: usize, y: usize| mask[y * w + x];
        // The stem of `n` (left edge of the first letter) is solid.
        assert_eq!(at(2, 30), 255);
        // Above the x-height of `n` is empty.
        assert_eq!(at(2, 5), 0);
        // Inside the counter of `e` (hole) stays empty: e starts after n.
        let scale = 40.0 / (ASCENT + DESCENT);
        let e_hole_x = ((774.0 + TRACKING - 109.0 + 380.0) * scale) as usize;
        let e_hole_y = ((ASCENT - 715.0) * scale) as usize;
        assert_eq!(at(e_hole_x, e_hole_y), 0);
    }
}
