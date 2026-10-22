//! Image tabs: PNG, JPG, GIF, WebP, BMP and ICO files are shown as a picture
//! (with size and dimensions) instead of being refused as binary.

use std::sync::Mutex;

use eframe::egui::{self, Align2, Color32, FontId, Rect, Sense, TextureHandle, pos2, vec2};

use crate::theme::Theme;

/// Larger images are not decoded (memory).
pub const MAX_BYTES: usize = 64 * 1024 * 1024;

pub struct Image {
    pub width: u32,
    pub height: u32,
    rgba: Vec<u8>,
    /// File size in bytes.
    pub size: usize,
    pub format: &'static str,
    texture: Mutex<Option<TextureHandle>>,
    /// Shown at 100 % instead of fitted (toggled by a click).
    actual_size: Mutex<bool>,
}

/// Image formats by file name.
pub fn is_image_name(name: &str) -> bool {
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico")
}

pub fn decode(bytes: &[u8]) -> Result<Image, String> {
    if bytes.len() > MAX_BYTES {
        return Err(t!("Das Bild ist zu groß für die Vorschau.").into());
    }
    if bytes.is_empty() {
        return Err(t!("Die Datei ist leer.").into());
    }
    let format = image::guess_format(bytes).map_err(|_| t!("Unbekanntes Bildformat").to_string())?;
    let img = match format {
        image::ImageFormat::Ico => decode_ico(bytes),
        _ => image::load_from_memory_with_format(bytes, format),
    }
    .map_err(|e| t!("Bild nicht lesbar: {e}", e = e))?;
    let rgba = img.to_rgba8();
    let name = match format {
        image::ImageFormat::Png => "PNG",
        image::ImageFormat::Jpeg => "JPEG",
        image::ImageFormat::Gif => "GIF",
        image::ImageFormat::WebP => "WebP",
        image::ImageFormat::Bmp => "BMP",
        image::ImageFormat::Ico => "ICO",
        _ => t!("Bild"),
    };
    Ok(Image {
        width: rgba.width(),
        height: rgba.height(),
        rgba: rgba.into_raw(),
        size: bytes.len(),
        format: name,
        texture: Mutex::new(None),
        actual_size: Mutex::new(false),
    })
}

/// The largest picture of an icon file. PNG entries are decoded directly, because
/// the `image` crate's ICO decoder only accepts them in RGBA (favicon tools often
/// write RGB or palette PNGs).
fn decode_ico(bytes: &[u8]) -> image::ImageResult<image::DynamicImage> {
    let u16_at = |i: usize| bytes.get(i..i + 2).map(|b| u16::from_le_bytes([b[0], b[1]]));
    let u32_at = |i: usize| bytes.get(i..i + 4).map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]));
    let count = u16_at(4).unwrap_or(0) as usize;
    let best = (0..count)
        .filter_map(|n| {
            let e = 6 + n * 16;
            // 0 means 256 in the directory.
            let w = match *bytes.get(e)? { 0 => 256, v => v as u32 };
            let h = match *bytes.get(e + 1)? { 0 => 256, v => v as u32 };
            let bpp = u16_at(e + 6)?;
            let (len, offset) = (u32_at(e + 8)? as usize, u32_at(e + 12)? as usize);
            Some((w * h, bpp, offset, len))
        })
        .max_by_key(|(area, bpp, ..)| (*area, *bpp));
    if let Some((_, _, offset, len)) = best
        && let Some(data) = bytes.get(offset..offset.saturating_add(len))
        && data.starts_with(b"\x89PNG")
    {
        return image::load_from_memory_with_format(data, image::ImageFormat::Png);
    }
    image::load_from_memory_with_format(bytes, image::ImageFormat::Ico)
}

fn human_size(n: usize) -> String {
    match n {
        0..1024 => format!("{n} B"),
        1024..1_048_576 => format!("{:.1} KB", n as f64 / 1024.0),
        _ => format!("{:.1} MB", n as f64 / 1_048_576.0),
    }
}

/// Draws the image centered in `rect` on a checkerboard, with an info line.
pub fn show(ui: &mut egui::Ui, rect: Rect, image: &Image, theme: &Theme, id: egui::Id) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme.background);
    let texture = image
        .texture
        .lock()
        .unwrap()
        .get_or_insert_with(|| {
            let color = egui::ColorImage::from_rgba_unmultiplied([image.width as usize, image.height as usize], &image.rgba);
            ui.ctx().load_texture("image-preview", color, egui::TextureOptions::LINEAR)
        })
        .clone();

    let info_h = 30.0;
    let area = Rect::from_min_max(rect.min + vec2(20.0, 20.0), pos2(rect.right() - 20.0, rect.bottom() - info_h - 10.0));
    let natural = vec2(image.width as f32, image.height as f32) / ui.ctx().pixels_per_point();
    let mut actual = image.actual_size.lock().unwrap();
    // Fit into the area, but never enlarge small images.
    let fit = (area.width() / natural.x).min(area.height() / natural.y).min(1.0);
    let scale = if *actual { 1.0 } else { fit };
    let size = natural * scale;
    let shown = Rect::from_center_size(area.center(), size);

    // Checkerboard for transparent pixels.
    let cell = 8.0;
    let clip = painter.with_clip_rect(shown.intersect(area));
    let (light, dark) = if theme.dark { (Color32::from_gray(58), Color32::from_gray(46)) } else { (Color32::from_gray(245), Color32::from_gray(225)) };
    clip.rect_filled(shown, 0.0, light);
    let cols = (shown.width() / cell).ceil() as i32;
    let rows = (shown.height() / cell).ceil() as i32;
    if cols * rows < 40_000 {
        for y in 0..rows {
            for x in (y % 2..cols).step_by(2) {
                let r = Rect::from_min_size(shown.min + vec2(x as f32 * cell, y as f32 * cell), vec2(cell, cell));
                clip.rect_filled(r, 0.0, dark);
            }
        }
    }
    clip.image(texture.id(), shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);

    // A click switches between fitted and 100 % (only matters for large images).
    let resp = ui.interact(area, id.with("image"), Sense::click());
    if fit < 1.0 {
        if resp.hovered() {
            ui.ctx().set_cursor_icon(if *actual { egui::CursorIcon::ZoomOut } else { egui::CursorIcon::ZoomIn });
        }
        if resp.clicked() {
            *actual = !*actual;
        }
    }

    let percent = (scale * 100.0).round();
    let info = t!("{width} × {height} px  ·  {format}  ·  {human_size}  ·  {percent} %", width = image.width, height = image.height, format = image.format, human_size = human_size(image.size), percent = percent);
    painter.text(pos2(rect.center().x, rect.bottom() - info_h / 2.0 - 4.0), Align2::CENTER_CENTER, info, FontId::proportional(12.0), theme.text_muted);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_png() {
        let mut bytes = Vec::new();
        let img = image::RgbaImage::from_pixel(3, 2, image::Rgba([255, 0, 0, 128]));
        img.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).unwrap();
        let decoded = decode(&bytes).unwrap();
        assert_eq!((decoded.width, decoded.height, decoded.format), (3, 2, "PNG"));
        assert!(decode(b"not an image").is_err());
        assert!(is_image_name("Logo.PNG") && is_image_name("a.webp") && !is_image_name("a.svg"));
        assert_eq!(decode(b"").err().as_deref(), Some("Die Datei ist leer."));
    }

    #[test]
    fn decodes_ico_with_rgb_png() {
        // An icon with 16 px and 32 px RGB (not RGBA) PNG entry.
        let png = |size: u32| {
            let mut out = Vec::new();
            image::RgbImage::from_pixel(size, size, image::Rgb([0, 128, 255]))
                .write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png)
                .unwrap();
            out
        };
        let (small, large) = (png(16), png(32));
        let mut ico = vec![0, 0, 1, 0, 2, 0];
        let mut offset = 6 + 2 * 16;
        for (size, data) in [(16u8, &small), (32u8, &large)] {
            ico.extend([size, size, 0, 0, 1, 0, 24, 0]);
            ico.extend((data.len() as u32).to_le_bytes());
            ico.extend((offset as u32).to_le_bytes());
            offset += data.len();
        }
        ico.extend(&small);
        ico.extend(&large);
        let decoded = decode(&ico).unwrap();
        assert_eq!((decoded.width, decoded.height, decoded.format), (32, 32, "ICO"));
    }
}

