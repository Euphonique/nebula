// App icon from `assets/nebula_logo.png`. Shared with build.rs via
// `include!`, so it may only use crates that are build-dependencies too.

const LOGO: &[u8] = include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/nebula_logo.png"));

/// The logo as straight RGBA: (pixels, width, height).
fn decode_logo() -> (Vec<u8>, usize, usize) {
    let mut decoder = png::Decoder::new(std::io::Cursor::new(LOGO));
    decoder.set_transformations(png::Transformations::normalize_to_color8() | png::Transformations::ALPHA);
    let mut reader = decoder.read_info().expect("logo png");
    let mut buf = vec![0; reader.output_buffer_size().expect("logo size")];
    let info = reader.next_frame(&mut buf).expect("logo frame");
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.color_type {
        png::ColorType::Rgba => buf[..w * h * 4].to_vec(),
        png::ColorType::GrayscaleAlpha => buf[..w * h * 2].chunks(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        _ => panic!("logo must have an alpha channel"),
    };
    (rgba, w, h)
}

/// Smallest square around the visible pixels, so small icons use all their room.
fn content_square(rgba: &[u8], w: usize, h: usize) -> (usize, usize, usize) {
    let (mut x0, mut y0, mut x1, mut y1) = (w, h, 0, 0);
    for y in 0..h {
        for x in 0..w {
            if rgba[(y * w + x) * 4 + 3] > 8 {
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if x0 > x1 {
        return (0, 0, w.min(h));
    }
    // A little air around the edges.
    let side = ((x1 - x0).max(y1 - y0) + 1) * 102 / 100;
    let side = side.min(w).min(h);
    let cx = (x0 + x1) / 2;
    let cy = (y0 + y1) / 2;
    let left = cx.saturating_sub(side / 2).min(w - side);
    let top = cy.saturating_sub(side / 2).min(h - side);
    (left, top, side)
}

/// Renders the app icon at `size` x `size` as straight (non-premultiplied) RGBA.
pub fn render_icon(size: u32) -> Vec<u8> {
    // Decoded once: the window icon, the welcome page and the .ico all need it.
    static LOGO_RGBA: std::sync::OnceLock<(Vec<u8>, usize, usize, (usize, usize, usize))> = std::sync::OnceLock::new();
    let (src, w, h, (left, top, side)) = LOGO_RGBA.get_or_init(|| {
        let (rgba, w, h) = decode_logo();
        let square = content_square(&rgba, w, h);
        (rgba, w, h, square)
    });
    let (src, w, h, left, top, side) = (src.as_slice(), *w, *h, *left, *top, *side);
    let size = size as usize;
    let mut out = vec![0u8; size * size * 4];
    let scale = side as f32 / size as f32;
    for y in 0..size {
        for x in 0..size {
            // Average the source pixels under this one (premultiplied, so
            // transparent pixels do not darken the edges).
            let sx0 = left as f32 + x as f32 * scale;
            let sy0 = top as f32 + y as f32 * scale;
            let (ix0, iy0) = (sx0.floor() as usize, sy0.floor() as usize);
            let ix1 = ((sx0 + scale).ceil() as usize).min(w).max(ix0 + 1);
            let iy1 = ((sy0 + scale).ceil() as usize).min(h).max(iy0 + 1);
            let mut acc = [0.0f32; 4];
            let mut weight = 0.0f32;
            for sy in iy0..iy1 {
                let wy = (sy as f32 + 1.0).min(sy0 + scale) - (sy as f32).max(sy0);
                for sx in ix0..ix1 {
                    let wx = (sx as f32 + 1.0).min(sx0 + scale) - (sx as f32).max(sx0);
                    let wgt = wx.max(0.0) * wy.max(0.0);
                    let i = (sy * w + sx) * 4;
                    let a = src[i + 3] as f32 / 255.0;
                    acc[0] += src[i] as f32 * a * wgt;
                    acc[1] += src[i + 1] as f32 * a * wgt;
                    acc[2] += src[i + 2] as f32 * a * wgt;
                    acc[3] += a * wgt;
                    weight += wgt;
                }
            }
            let o = (y * size + x) * 4;
            if acc[3] > 0.0 {
                out[o] = (acc[0] / acc[3]).round().clamp(0.0, 255.0) as u8;
                out[o + 1] = (acc[1] / acc[3]).round().clamp(0.0, 255.0) as u8;
                out[o + 2] = (acc[2] / acc[3]).round().clamp(0.0, 255.0) as u8;
                out[o + 3] = (acc[3] / weight.max(f32::EPSILON) * 255.0).round().clamp(0.0, 255.0) as u8;
            }
        }
    }
    out
}

/// Encodes several sizes into a Windows .ico (32-bit BMP entries).
#[allow(dead_code)]
pub fn encode_ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = sizes.iter().map(|&size| (size, render_icon(size))).collect();
    ico_from_rgba(&images, false)
}

/// Builds a .ico from square straight-RGBA images (size, pixels); with
/// `png_large` the 256 px entry is stored as PNG (much smaller).
#[allow(dead_code)]
pub fn ico_from_rgba(images: &[(u32, Vec<u8>)], png_large: bool) -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = images
        .iter()
        .map(|(size, rgba)| {
            let size = *size;
            if png_large && size >= 256 {
                let mut out = Vec::new();
                let mut encoder = png::Encoder::new(&mut out, size, size);
                encoder.set_color(png::ColorType::Rgba);
                encoder.set_depth(png::BitDepth::Eight);
                let mut writer = encoder.write_header().expect("png header");
                writer.write_image_data(rgba).expect("png data");
                writer.finish().expect("png end");
                return (size, out);
            }
            let mut bmp = Vec::new();
            // BITMAPINFOHEADER; height counts XOR + AND masks.
            bmp.extend_from_slice(&40u32.to_le_bytes());
            bmp.extend_from_slice(&(size as i32).to_le_bytes());
            bmp.extend_from_slice(&((size * 2) as i32).to_le_bytes());
            bmp.extend_from_slice(&1u16.to_le_bytes());
            bmp.extend_from_slice(&32u16.to_le_bytes());
            bmp.extend_from_slice(&[0u8; 24]);
            for y in (0..size).rev() {
                for x in 0..size {
                    let i = ((y * size + x) * 4) as usize;
                    bmp.extend_from_slice(&[rgba[i + 2], rgba[i + 1], rgba[i], rgba[i + 3]]);
                }
            }
            // AND mask: 1 bit per pixel, rows padded to 4 bytes, all opaque.
            let row = size.div_ceil(32) * 4;
            bmp.extend(std::iter::repeat_n(0u8, (row * size) as usize));
            (size, bmp)
        })
        .collect();

    let mut ico = Vec::new();
    ico.extend_from_slice(&[0, 0, 1, 0]);
    ico.extend_from_slice(&(images.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * images.len() as u32;
    for (size, data) in &images {
        let dim = if *size >= 256 { 0 } else { *size as u8 };
        ico.extend_from_slice(&[dim, dim, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&32u16.to_le_bytes());
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&offset.to_le_bytes());
        offset += data.len() as u32;
    }
    for (_, data) in images {
        ico.extend_from_slice(&data);
    }
    ico
}
