//! Small pure helpers (no OS calls) so they can be unit-tested on any platform.

pub fn clamp01(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

pub fn smoothstep(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp01((x - e0) / (e1 - e0));
    t * t * (3.0 - 2.0 * t)
}

/// Horizontal scroll offset (logical px) for a title that is `overflow` px too wide:
/// rest, glide to the end at a steady speed, rest, then jump back and repeat.
pub fn marquee_offset(overflow: f32, t: f32) -> f32 {
    const SPEED: f32 = 36.0; // px per second
    const PAUSE: f32 = 1.6; // seconds of rest at each end
    if overflow <= 0.0 {
        return 0.0;
    }
    let travel = overflow / SPEED;
    let ph = t.rem_euclid(2.0 * PAUSE + travel);
    if ph < PAUSE {
        0.0
    } else if ph < PAUSE + travel {
        (ph - PAUSE) * SPEED
    } else {
        overflow
    }
}

/// "3:07" or "1:02:09".
pub fn fmt_time(ms: i64) -> String {
    let s = (ms.max(0) / 1000) as u64;
    let (h, m, s) = (s / 3600, (s / 60) % 60, s % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

/// Collapse all whitespace / control characters to single spaces and cap the length (in chars).
pub fn single_line(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut n = 0usize;
    let mut last_space = true; // swallows leading whitespace
    for ch in s.chars() {
        let ch = if ch.is_whitespace() || ch.is_control() { ' ' } else { ch };
        if ch == ' ' {
            if last_space {
                continue;
            }
            last_space = true;
        } else {
            last_space = false;
        }
        out.push(ch);
        n += 1;
        if n >= max_chars {
            break;
        }
    }
    out.trim_end().to_string()
}

/// "Spotify.exe" -> "Spotify"; "Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic" -> "ZuneMusic".
pub fn friendly_app(aumid: &str) -> String {
    let base = aumid.rsplit('!').next().unwrap_or(aumid);
    let base = base
        .strip_suffix(".exe")
        .or_else(|| base.strip_suffix(".EXE"))
        .unwrap_or(base);
    let name = base.rsplit('.').next().filter(|s| !s.is_empty()).unwrap_or(base);
    let mut c = name.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}

pub fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
    let (r, g, b) = (r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let c = v * s;
    let hp = (h.rem_euclid(360.0)) / 60.0;
    let x = c * (1.0 - ((hp % 2.0) - 1.0).abs());
    let (r1, g1, b1) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    let f = |u: f32| ((u + m) * 255.0).round().clamp(0.0, 255.0) as u8;
    (f(r1), f(g1), f(b1))
}

/// Turn an average colour into a vivid UI accent: keep the hue, lift saturation and brightness.
pub fn accent_from_avg(r: u8, g: u8, b: u8) -> (u8, u8, u8) {
    let (h, s, v) = rgb_to_hsv(r, g, b);
    let s = if s < 0.12 { s } else { s.clamp(0.45, 0.85) };
    hsv_to_rgb(h, s, v.max(0.82))
}

/// Centre-crop a BGRA image to a square and convert it to RGBA (alpha is left as-is, i.e. still
/// premultiplied if the input was). Returns (pixels, side).
pub fn crop_square_bgra_to_rgba(src: &[u8], w: u32, h: u32) -> (Vec<u8>, u32) {
    let side = w.min(h);
    if side == 0 || src.len() < (w as usize) * (h as usize) * 4 {
        return (Vec::new(), 0);
    }
    let (x0, y0) = ((w - side) / 2, (h - side) / 2);
    let mut out = Vec::with_capacity((side * side * 4) as usize);
    for y in 0..side {
        let row = (((y0 + y) * w + x0) * 4) as usize;
        for px in src[row..row + (side * 4) as usize].chunks_exact(4) {
            out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
        }
    }
    (out, side)
}

/// Average colour of premultiplied RGBA pixels (sampled), un-premultiplied by total alpha.
pub fn avg_color_premul(rgba: &[u8]) -> (u8, u8, u8) {
    let (mut r, mut g, mut b, mut a) = (0u64, 0u64, 0u64, 0u64);
    for px in rgba.chunks_exact(4).step_by(3) {
        r += px[0] as u64;
        g += px[1] as u64;
        b += px[2] as u64;
        a += px[3] as u64;
    }
    if a == 0 {
        return (128, 128, 128);
    }
    let f = |v: u64| ((v * 255 + a / 2) / a).min(255) as u8;
    (f(r), f(g), f(b))
}

/// Largest size that fits `max_w` x `max_h` and keeps the aspect ratio (never upscales, never 0).
pub fn fit_dims(w: u32, h: u32, max_w: u32, max_h: u32) -> (u32, u32) {
    if w == 0 || h == 0 {
        return (1, 1);
    }
    let s = (max_w as f32 / w as f32).min(max_h as f32 / h as f32).min(1.0);
    (((w as f32 * s).round() as u32).max(1), ((h as f32 * s).round() as u32).max(1))
}

/// Straight RGBA -> premultiplied RGBA, in place (what tiny-skia pixmaps hold).
pub fn premultiply_rgba(px: &mut [u8]) {
    for p in px.chunks_exact_mut(4) {
        let a = p[3] as u32;
        if a != 255 {
            for c in p.iter_mut().take(3) {
                *c = ((*c as u32 * a + 127) / 255) as u8;
            }
        }
    }
}

/// 64-bit FNV-1a, used to recognise that the same image was copied twice.
pub fn fnv1a(data: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// "812 B", "34 KB", "1.2 MB".
pub fn fmt_bytes(n: usize) -> String {
    const KB: f64 = 1024.0;
    let f = n as f64;
    if f < KB {
        format!("{n} B")
    } else if f < KB * KB {
        format!("{:.0} KB", f / KB)
    } else {
        format!("{:.1} MB", f / (KB * KB))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumb_helpers() {
        assert_eq!(fit_dims(1920, 1080, 96, 60), (96, 54));
        assert_eq!(fit_dims(1080, 1920, 96, 60), (34, 60));
        assert_eq!(fit_dims(40, 20, 96, 60), (40, 20)); // never upscaled
        assert_eq!(fit_dims(0, 5, 96, 60), (1, 1));
        assert_eq!(fit_dims(10000, 1, 96, 60), (96, 1));

        let mut px = [200u8, 100, 50, 128, 10, 20, 30, 255, 255, 255, 255, 0];
        premultiply_rgba(&mut px);
        assert_eq!(px, [100, 50, 25, 128, 10, 20, 30, 255, 0, 0, 0, 0]);

        assert_ne!(fnv1a(b"abc"), fnv1a(b"abd"));
        assert_eq!(fnv1a(b"abc"), fnv1a(b"abc"));
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);

        assert_eq!(fmt_bytes(812), "812 B");
        assert_eq!(fmt_bytes(34 * 1024), "34 KB");
        assert_eq!(fmt_bytes(1_258_291), "1.2 MB");
    }

    #[test]
    fn square_crop_and_swizzle() {
        // 4x2 BGRA image; crop must take the centre 2x2 columns 1..3.
        let mut src = Vec::new();
        for x in 0..4u8 {
            src.extend_from_slice(&[x, 10, 20, 255]); // B,G,R,A with B = column
        }
        for x in 0..4u8 {
            src.extend_from_slice(&[x, 11, 21, 255]);
        }
        let (out, side) = crop_square_bgra_to_rgba(&src, 4, 2);
        assert_eq!(side, 2);
        assert_eq!(out.len(), 2 * 2 * 4);
        assert_eq!(&out[0..4], &[20, 10, 1, 255]); // R,G,B,A of column 1, row 0
        assert_eq!(&out[4..8], &[20, 10, 2, 255]);
        assert_eq!(&out[8..12], &[21, 11, 1, 255]);
        assert_eq!(crop_square_bgra_to_rgba(&[], 0, 0).1, 0);
    }

    #[test]
    fn average_colour() {
        let px: Vec<u8> = (0..30).flat_map(|_| [200u8, 100, 50, 255]).collect();
        assert_eq!(avg_color_premul(&px), (200, 100, 50));
        assert_eq!(avg_color_premul(&[0, 0, 0, 0]), (128, 128, 128));
    }

    #[test]
    fn marquee_rests_then_glides_then_rests() {
        assert_eq!(marquee_offset(72.0, 0.0), 0.0);
        assert_eq!(marquee_offset(72.0, 1.0), 0.0);
        let mid = marquee_offset(72.0, 1.6 + 1.0);
        assert!((mid - 36.0).abs() < 1e-3);
        assert_eq!(marquee_offset(72.0, 1.6 + 2.0 + 0.5), 72.0);
        assert_eq!(marquee_offset(0.0, 5.0), 0.0);
        for i in 0..400 {
            let o = marquee_offset(50.0, i as f32 * 0.05);
            assert!((0.0..=50.0).contains(&o));
        }
    }

    #[test]
    fn time_formatting() {
        assert_eq!(fmt_time(0), "0:00");
        assert_eq!(fmt_time(61_000), "1:01");
        assert_eq!(fmt_time(3_725_000), "1:02:05");
        assert_eq!(fmt_time(-5), "0:00");
    }

    #[test]
    fn single_line_collapses_and_caps() {
        assert_eq!(single_line("  a \n\n\tb   c ", 50), "a b c");
        assert_eq!(single_line("abcdefghij", 4), "abcd");
        assert_eq!(single_line("\n\n", 10), "");
    }

    #[test]
    fn app_names() {
        assert_eq!(friendly_app("Spotify.exe"), "Spotify");
        assert_eq!(friendly_app("chrome.exe"), "Chrome");
        assert_eq!(
            friendly_app("Microsoft.ZuneMusic_8wekyb3d8bbwe!Microsoft.ZuneMusic"),
            "ZuneMusic"
        );
        assert_eq!(friendly_app(""), "");
    }

    #[test]
    fn hsv_round_trip() {
        for &(r, g, b) in &[(255u8, 0u8, 0u8), (12, 200, 90), (30, 60, 250), (128, 128, 128)] {
            let (h, s, v) = rgb_to_hsv(r, g, b);
            let (r2, g2, b2) = hsv_to_rgb(h, s, v);
            assert!((r as i32 - r2 as i32).abs() <= 1, "{r} {r2}");
            assert!((g as i32 - g2 as i32).abs() <= 1);
            assert!((b as i32 - b2 as i32).abs() <= 1);
        }
    }

    #[test]
    fn accent_is_bright() {
        let (r, g, b) = accent_from_avg(40, 20, 10);
        assert!(r.max(g).max(b) >= 200);
    }

    #[test]
    fn smoothstep_edges() {
        assert_eq!(smoothstep(0.0, 1.0, -1.0), 0.0);
        assert_eq!(smoothstep(0.0, 1.0, 2.0), 1.0);
        assert!((smoothstep(0.0, 1.0, 0.5) - 0.5).abs() < 1e-6);
    }
}
