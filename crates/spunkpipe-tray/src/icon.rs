//! The tray icon, from the embedded picture.

const PNG: &[u8] = include_bytes!("../assets/icon.png");

/// Decodes the picture to RGBA8 at its own size.
fn decode() -> (Vec<u8>, usize, usize) {
    let mut reader = png::Decoder::new(std::io::Cursor::new(PNG)).read_info().expect("embedded icon is a valid PNG");
    let mut buf = vec![0; reader.output_buffer_size().expect("icon size is sane")];
    let info = reader.next_frame(&mut buf).expect("embedded icon decodes");
    let (w, h) = (info.width as usize, info.height as usize);
    let px = &buf[..info.buffer_size()];
    let rgba = match info.color_type {
        png::ColorType::Rgba => px.to_vec(),
        png::ColorType::Rgb => px.chunks(3).flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
        png::ColorType::Grayscale => px.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::GrayscaleAlpha => px.chunks(2).flat_map(|c| [c[0], c[0], c[0], c[1]]).collect(),
        png::ColorType::Indexed => unreachable!("expanded by default"),
    };
    (rgba, w, h)
}

/// Bilinear resize of RGBA8.
fn resize(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize) -> Vec<u8> {
    let mut out = vec![0u8; dw * dh * 4];
    for y in 0..dh {
        let fy = ((y as f32 + 0.5) * sh as f32 / dh as f32 - 0.5).clamp(0.0, (sh - 1) as f32);
        let (y0, wy) = (fy.floor() as usize, fy.fract());
        let y1 = (y0 + 1).min(sh - 1);
        for x in 0..dw {
            let fx = ((x as f32 + 0.5) * sw as f32 / dw as f32 - 0.5).clamp(0.0, (sw - 1) as f32);
            let (x0, wx) = (fx.floor() as usize, fx.fract());
            let x1 = (x0 + 1).min(sw - 1);
            for c in 0..4 {
                let p = |xx: usize, yy: usize| src[(yy * sw + xx) * 4 + c] as f32;
                let top = p(x0, y0) * (1.0 - wx) + p(x1, y0) * wx;
                let bot = p(x0, y1) * (1.0 - wx) + p(x1, y1) * wx;
                out[(y * dw + x) * 4 + c] = (top * (1.0 - wy) + bot * wy).round() as u8;
            }
        }
    }
    out
}

/// RGBA8 pixels at `size` x `size`.
pub fn rgba(size: usize) -> Vec<u8> {
    let (px, w, h) = decode();
    if w == size && h == size {
        px
    } else {
        resize(&px, w, h, size, size)
    }
}

/// ARGB32 in network byte order, as the freedesktop tray protocol wants it.
#[cfg(all(unix, not(target_os = "macos")))]
pub fn argb(size: usize) -> Vec<u8> {
    rgba(size).chunks(4).flat_map(|p| [p[3], p[0], p[1], p[2]]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_and_scales() {
        let (_, w, h) = decode();
        assert_eq!((w, h), (60, 60));
        for size in [16, 32, 64, 128] {
            let p = rgba(size);
            assert_eq!(p.len(), size * size * 4);
            assert!(p.chunks(4).all(|c| c[3] == 255), "opaque");
            assert!(p.iter().any(|&b| b != 0), "not blank");
        }
    }
}
