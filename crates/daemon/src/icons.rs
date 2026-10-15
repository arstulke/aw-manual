//! Tray icons, drawn at runtime so they can be tinted in the favorite's color.

use tauri::image::Image;

pub const SIZE: u32 = 64;
const DEFAULT_ACTIVE: [u8; 3] = [0x3B, 0x82, 0xF6];
const IDLE: [u8; 3] = [0x8A, 0x8F, 0x98];
const BADGE: [u8; 3] = [0xF5, 0x9E, 0x0B];

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum IconState {
    Idle,
    /// Running, with an optional color (`#RRGGBB` or `#RGB`).
    Active(Option<String>),
}

pub fn parse_hex(color: &str) -> Option<[u8; 3]> {
    let hex = color.strip_prefix('#')?;
    let v = |s: &str| u8::from_str_radix(s, 16).ok();
    match hex.len() {
        6 => Some([v(&hex[0..2])?, v(&hex[2..4])?, v(&hex[4..6])?]),
        3 => {
            let d = |i: usize| v(&hex[i..i + 1]).map(|x| x * 17);
            Some([d(0)?, d(1)?, d(2)?])
        }
        _ => None,
    }
}

/// Coverage (0..1) of a pixel by a disc, with 4×4 supersampling.
fn disc(px: u32, py: u32, cx: f32, cy: f32, r: f32) -> f32 {
    let mut hits = 0;
    for sy in 0..4 {
        for sx in 0..4 {
            let x = px as f32 + (sx as f32 + 0.5) / 4.0;
            let y = py as f32 + (sy as f32 + 0.5) / 4.0;
            if (x - cx).powi(2) + (y - cy).powi(2) <= r * r {
                hits += 1;
            }
        }
    }
    hits as f32 / 16.0
}

fn blend(dst: &mut [u8], color: [u8; 3], alpha: f32) {
    if alpha <= 0.0 {
        return;
    }
    let da = dst[3] as f32 / 255.0;
    let out_a = alpha + da * (1.0 - alpha);
    for i in 0..3 {
        let c = (color[i] as f32 * alpha + dst[i] as f32 * da * (1.0 - alpha)) / out_a.max(1e-6);
        dst[i] = c.round().clamp(0.0, 255.0) as u8;
    }
    dst[3] = (out_a * 255.0).round() as u8;
}

/// Renders the tray icon as RGBA pixels.
pub fn render(state: &IconState, offline: bool) -> Vec<u8> {
    let mut px = vec![0u8; (SIZE * SIZE * 4) as usize];
    let c = SIZE as f32 / 2.0;
    let outer = SIZE as f32 * 0.44;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let p = &mut px[i..i + 4];
            match state {
                IconState::Idle => {
                    // Ring.
                    let a = disc(x, y, c, c, outer) - disc(x, y, c, c, outer * 0.72);
                    blend(p, IDLE, a);
                }
                IconState::Active(color) => {
                    let rgb = color
                        .as_deref()
                        .and_then(parse_hex)
                        .unwrap_or(DEFAULT_ACTIVE);
                    blend(p, rgb, disc(x, y, c, c, outer));
                    // A white "play" dot in the middle.
                    blend(p, [255, 255, 255], disc(x, y, c, c, outer * 0.32));
                }
            }
            if offline {
                let (bx, by, br) = (SIZE as f32 * 0.78, SIZE as f32 * 0.78, SIZE as f32 * 0.2);
                let border = disc(x, y, bx, by, br + 3.0);
                // Cut out a transparent border around the badge.
                let p = &mut px[i..i + 4];
                if border > 0.0 {
                    p[3] = (p[3] as f32 * (1.0 - border)).round() as u8;
                }
                blend(p, BADGE, disc(x, y, bx, by, br));
            }
        }
    }
    px
}

pub fn image(state: &IconState, offline: bool) -> Image<'static> {
    Image::new_owned(render(state, offline), SIZE, SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colors() {
        assert_eq!(parse_hex("#E4572E"), Some([0xE4, 0x57, 0x2E]));
        assert_eq!(parse_hex("#fff"), Some([255, 255, 255]));
        assert_eq!(parse_hex("red"), None);
    }

    #[test]
    fn renders() {
        let px = render(&IconState::Active(Some("#ff0000".into())), false);
        assert_eq!(px.len(), (SIZE * SIZE * 4) as usize);
        // The center is white, a point between center and edge is red, the corner transparent.
        let at =
            |x: u32, y: u32| &px[((y * SIZE + x) * 4) as usize..((y * SIZE + x) * 4 + 4) as usize];
        assert_eq!(at(32, 32), [255, 255, 255, 255]);
        assert_eq!(at(32, 8), [255, 0, 0, 255]);
        assert_eq!(at(0, 0)[3], 0);
        let idle = render(&IconState::Idle, true);
        let at = |x: u32, y: u32| {
            &idle[((y * SIZE + x) * 4) as usize..((y * SIZE + x) * 4 + 4) as usize]
        };
        assert_eq!(at(32, 32)[3], 0, "ring is hollow");
        assert_eq!(&at(50, 50)[..3], &BADGE);
    }
}
