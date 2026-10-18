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
        return Err("Das Bild ist zu groß für die Vorschau.".into());
    }
    let format = image::guess_format(bytes).map_err(|_| "Unbekanntes Bildformat".to_string())?;
    let img = image::load_from_memory_with_format(bytes, format).map_err(|e| format!("Bild nicht lesbar: {e}"))?;
    let rgba = img.to_rgba8();
    let name = match format {
        image::ImageFormat::Png => "PNG",
        image::ImageFormat::Jpeg => "JPEG",
        image::ImageFormat::Gif => "GIF",
        image::ImageFormat::WebP => "WebP",
        image::ImageFormat::Bmp => "BMP",
        image::ImageFormat::Ico => "ICO",
        _ => "Bild",
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
    let info = format!("{} × {} px  ·  {}  ·  {}  ·  {percent} %", image.width, image.height, image.format, human_size(image.size));
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
    }
}
