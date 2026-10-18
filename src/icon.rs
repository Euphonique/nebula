// App icon rasterizer. Shared with build.rs via `include!`, so it must not
// depend on any crate.

/// Renders the app icon as straight (non-premultiplied) RGBA.
pub fn render_icon(size: u32) -> Vec<u8> {
    let s = size as f32;
    let mut out = vec![0u8; (size * size * 4) as usize];
    let smooth = |edge0: f32, edge1: f32, x: f32| {
        let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let aa = 1.2 / s; // ~one pixel in normalized units
    for y in 0..size {
        for x in 0..size {
            // Normalized coordinates in [-1, 1].
            let u = (x as f32 + 0.5) / s * 2.0 - 1.0;
            let v = (y as f32 + 0.5) / s * 2.0 - 1.0;

            // Rounded square (squircle-ish) mask.
            let half = 0.9;
            let r = 0.42;
            let qx = (u.abs() - (half - r)).max(0.0);
            let qy = (v.abs() - (half - r)).max(0.0);
            let d = (qx * qx + qy * qy).sqrt() - r;
            let mask = 1.0 - smooth(-aa, aa, d);
            if mask <= 0.0 {
                continue;
            }

            // Diagonal gradient: deep indigo to teal.
            let t = ((u + v) * 0.5 + 1.0) / 2.0;
            let top = [96.0, 70.0, 230.0];
            let bottom = [20.0, 170.0, 200.0];
            let mut c = [
                top[0] + (bottom[0] - top[0]) * t,
                top[1] + (bottom[1] - top[1]) * t,
                top[2] + (bottom[2] - top[2]) * t,
            ];

            // Soft nebula glow around the center.
            let dist = (u * u + v * v).sqrt();
            let glow = (1.0 - dist / 0.75).clamp(0.0, 1.0).powf(2.0) * 0.45;
            for ch in &mut c {
                *ch += (255.0 - *ch) * glow;
            }

            // Tilted orbit ring.
            let (ca, sa) = (0.5f32.cos(), 0.5f32.sin());
            let ou = u * ca + v * sa;
            let ov = -u * sa + v * ca;
            let ring = ((ou / 0.66).powi(2) + (ov / 0.26).powi(2)).sqrt();
            let ring_w = 0.055;
            let ring_a = 1.0 - smooth(ring_w - aa * 3.0, ring_w + aa * 3.0, (ring - 1.0).abs() * 0.3);
            for ch in &mut c {
                *ch += (255.0 - *ch) * ring_a * 0.85;
            }

            // Four-pointed star in the middle.
            let star = (u.abs() * v.abs()).sqrt() * 2.2 + dist * 0.55;
            let star_a = 1.0 - smooth(0.16 - aa * 4.0, 0.16 + aa * 4.0, star);
            for ch in &mut c {
                *ch += (255.0 - *ch) * star_a;
            }

            let i = ((y * size + x) * 4) as usize;
            out[i] = c[0].clamp(0.0, 255.0) as u8;
            out[i + 1] = c[1].clamp(0.0, 255.0) as u8;
            out[i + 2] = c[2].clamp(0.0, 255.0) as u8;
            out[i + 3] = (mask * 255.0) as u8;
        }
    }
    out
}

/// Encodes several sizes into a Windows .ico (32-bit BMP entries).
#[allow(dead_code)]
pub fn encode_ico(sizes: &[u32]) -> Vec<u8> {
    let images: Vec<(u32, Vec<u8>)> = sizes
        .iter()
        .map(|&size| {
            let rgba = render_icon(size);
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
