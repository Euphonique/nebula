//! Windows file type icons: the same badges as in the file list
//! (`ui::icons::file`), drawn with egui without a window and rasterized in
//! software, then saved as .ico files the registry can point to.

use std::path::PathBuf;

use eframe::egui::{self, Color32, Pos2, pos2, vec2};

/// Sizes Explorer asks for (small list … extra large icons).
const SIZES: [u32; 8] = [16, 20, 24, 32, 40, 48, 64, 256];
/// Side of the drawing area in points; the page itself is 13 x 16.
const CANVAS: f32 = 17.0;

/// Folder for the generated icons (survives updates of the program file).
pub fn dir() -> PathBuf {
    PathBuf::from(std::env::var_os("LOCALAPPDATA").unwrap_or_default()).join("Nebula").join("FileIcons")
}

/// Draws the icons; one per batch, so the fonts are loaded only once.
pub struct IconPainter {
    ctx: egui::Context,
    /// The font texture, kept up to date across frames.
    atlas: Option<egui::ColorImage>,
}

impl IconPainter {
    pub fn new() -> Self {
        let ctx = egui::Context::default();
        crate::fonts::apply(&ctx, "");
        Self { ctx, atlas: None }
    }

    /// Writes the icon for `ext` and returns its path.
    pub fn write(&mut self, ext: &str) -> anyhow::Result<PathBuf> {
        let dir = dir();
        std::fs::create_dir_all(&dir)?;
        let path = dir.join(format!("{ext}.ico"));
        let images: Vec<(u32, Vec<u8>)> = SIZES.iter().map(|&s| (s, self.render(ext, s))).collect();
        std::fs::write(&path, crate::icon::ico_from_rgba(&images, true))?;
        Ok(path)
    }

    /// The file icon for `ext` at `size` x `size` pixels, straight RGBA.
    pub fn render(&mut self, ext: &str, size: u32) -> Vec<u8> {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, vec2(CANVAS, CANVAS))),
            ..Default::default()
        };
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(size as f32 / CANVAS);
        let name = format!("x.{ext}");
        let mut output = self.ctx.run_ui(input, |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            crate::ui::icons::file(&painter, pos2(CANVAS / 2.0, CANVAS / 2.0), &name);
        });
        let ppp = output.pixels_per_point;
        self.update_atlas(&output.textures_delta);
        output.textures_delta.clear();
        let mut canvas = vec![[0.0f32; 4]; (size * size) as usize];
        for prim in self.ctx.tessellate(output.shapes, ppp) {
            if let egui::epaint::Primitive::Mesh(mesh) = prim.primitive {
                let texture = if mesh.texture_id == egui::TextureId::default() { self.atlas.as_ref() } else { None };
                for tri in mesh.indices.chunks_exact(3) {
                    let v = [0, 1, 2].map(|i| &mesh.vertices[tri[i] as usize]);
                    fill_triangle(&mut canvas, size as usize, ppp, v, texture);
                }
            }
        }
        // Premultiplied to straight alpha.
        canvas
            .iter()
            .flat_map(|p| {
                let a = p[3].clamp(0.0, 1.0);
                let c = |x: f32| if a > 0.0 { (x / a * 255.0).round().clamp(0.0, 255.0) as u8 } else { 0 };
                [c(p[0]), c(p[1]), c(p[2]), (a * 255.0).round() as u8]
            })
            .collect()
    }

    /// Applies the frame's changes to the font texture (whole or in patches).
    fn update_atlas(&mut self, delta: &egui::epaint::textures::TexturesDelta) {
        let Some(deltas) = delta.set.get(&egui::TextureId::default()) else { return };
        for d in deltas {
            let egui::ImageData::Color(image) = &d.image;
            match (d.pos, self.atlas.as_mut()) {
                (Some([x0, y0]), Some(a)) => {
                    for y in 0..image.size[1] {
                        for x in 0..image.size[0] {
                            a.pixels[(y0 + y) * a.size[0] + x0 + x] = image.pixels[y * image.size[0] + x];
                        }
                    }
                }
                _ => self.atlas = Some((**image).clone()),
            }
        }
    }
}

fn rgba(c: Color32) -> [f32; 4] {
    [c.r(), c.g(), c.b(), c.a()].map(|x| x as f32 / 255.0)
}

/// Draws one mesh triangle (premultiplied colors, sampled at pixel centers;
/// egui's feathering already gives the soft edges).
fn fill_triangle(canvas: &mut [[f32; 4]], size: usize, ppp: f32, v: [&egui::epaint::Vertex; 3], texture: Option<&egui::ColorImage>) {
    let p = v.map(|v| pos2(v.pos.x * ppp, v.pos.y * ppp));
    let area = (p[1].x - p[0].x) * (p[2].y - p[0].y) - (p[2].x - p[0].x) * (p[1].y - p[0].y);
    if area.abs() < 1e-6 {
        return;
    }
    let min_x = p.iter().map(|q| q.x).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
    let max_x = (p.iter().map(|q| q.x).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(size);
    let min_y = p.iter().map(|q| q.y).fold(f32::MAX, f32::min).floor().max(0.0) as usize;
    let max_y = (p.iter().map(|q| q.y).fold(f32::MIN, f32::max).ceil().max(0.0) as usize).min(size);
    let colors = v.map(|v| rgba(v.color));
    for y in min_y..max_y {
        for x in min_x..max_x {
            let (px, py) = (x as f32 + 0.5, y as f32 + 0.5);
            let w0 = ((p[1].x - px) * (p[2].y - py) - (p[2].x - px) * (p[1].y - py)) / area;
            let w1 = ((p[2].x - px) * (p[0].y - py) - (p[0].x - px) * (p[2].y - py)) / area;
            let w2 = 1.0 - w0 - w1;
            if w0 < -1e-4 || w1 < -1e-4 || w2 < -1e-4 {
                continue;
            }
            let mut src = [0.0f32; 4];
            for (i, s) in src.iter_mut().enumerate() {
                *s = colors[0][i] * w0 + colors[1][i] * w1 + colors[2][i] * w2;
            }
            if let Some(tex) = texture {
                let u = v[0].uv.x * w0 + v[1].uv.x * w1 + v[2].uv.x * w2;
                let t = v[0].uv.y * w0 + v[1].uv.y * w1 + v[2].uv.y * w2;
                let s = sample(tex, u, t);
                for i in 0..4 {
                    src[i] *= s[i];
                }
            }
            let dst = &mut canvas[y * size + x];
            for i in 0..4 {
                dst[i] = src[i] + dst[i] * (1.0 - src[3]);
            }
        }
    }
}

/// Bilinear lookup in the font texture.
fn sample(tex: &egui::ColorImage, u: f32, v: f32) -> [f32; 4] {
    let (w, h) = (tex.size[0], tex.size[1]);
    let x = (u * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let y = (v * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |x: usize, y: usize| rgba(tex.pixels[y * w + x]);
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    let mut out = [0.0; 4];
    for i in 0..4 {
        out[i] = (a[i] * (1.0 - fx) + b[i] * fx) * (1.0 - fy) + (c[i] * (1.0 - fx) + d[i] * fx) * fy;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_badge_with_label() {
        let px = IconPainter::new().render("html", 48);
        assert_eq!(px.len(), 48 * 48 * 4);
        // Center of the page is covered; the corners stay transparent.
        let at = |x: usize, y: usize| &px[(y * 48 + x) * 4..(y * 48 + x) * 4 + 4];
        assert_eq!(at(0, 0)[3], 0);
        assert!(at(10, 40)[3] > 200, "page body is opaque");
        // The white label shows up somewhere on the orange page.
        assert!(px.chunks(4).any(|p| p[3] > 200 && p[0] > 240 && p[1] > 240 && p[2] > 240), "label drawn");
    }

    /// Writes sample icons as PNG for a look: `cargo test file_icons_preview -- --ignored`.
    #[test]
    #[ignore]
    fn file_icons_preview() {
        let out = std::env::temp_dir().join("nebula-file-icons");
        std::fs::create_dir_all(&out).unwrap();
        let mut painter = IconPainter::new();
        for ext in ["html", "css", "js", "php", "json", "md", "txt", "cpp", "csv"] {
            for size in [16, 32, 256] {
                let rgba = painter.render(ext, size);
                let file = std::fs::File::create(out.join(format!("{ext}-{size}.png"))).unwrap();
                let mut enc = png::Encoder::new(file, size, size);
                enc.set_color(png::ColorType::Rgba);
                enc.set_depth(png::BitDepth::Eight);
                enc.write_header().unwrap().write_image_data(&rgba).unwrap();
            }
        }
        println!("{}", out.display());
    }
}
