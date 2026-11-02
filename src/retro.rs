//! Turns album art into 8-bit-style pixel art: punch up the colors, then
//! snap every pixel to a retro console palette with ordered (Bayer)
//! dithering, the checkerboard shading old games used to fake in-between
//! colors. Ordered dithering depends only on a pixel's position, so the art
//! never shimmers between frames.

use image::{Rgb, RgbImage};
use serde::{Deserialize, Serialize};

use crate::palette::Palette;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ArtStyle {
    /// The terminal theme's own colors (needs a readable Ghostty theme).
    #[default]
    Terminal,
    /// PICO-8's 16-color palette.
    Pico8,
    /// The original Game Boy's four greens.
    GameBoy,
    /// The Commodore 64's 16 colors.
    C64,
    /// The cover's own colors, just pixelated.
    Original,
}

impl ArtStyle {
    /// The next style, skipping `Terminal` when the theme is unknown.
    pub fn next(self, have_palette: bool) -> Self {
        match self {
            ArtStyle::Terminal => ArtStyle::Pico8,
            ArtStyle::Pico8 => ArtStyle::GameBoy,
            ArtStyle::GameBoy => ArtStyle::C64,
            ArtStyle::C64 => ArtStyle::Original,
            ArtStyle::Original if have_palette => ArtStyle::Terminal,
            ArtStyle::Original => ArtStyle::Pico8,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ArtStyle::Terminal => "terminal theme",
            ArtStyle::Pico8 => "PICO-8",
            ArtStyle::GameBoy => "Game Boy",
            ArtStyle::C64 => "C64",
            ArtStyle::Original => "original colors",
        }
    }
}

const PICO8: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0x1D, 0x2B, 0x53],
    [0x7E, 0x25, 0x53],
    [0x00, 0x87, 0x51],
    [0xAB, 0x52, 0x36],
    [0x5F, 0x57, 0x4F],
    [0xC2, 0xC3, 0xC7],
    [0xFF, 0xF1, 0xE8],
    [0xFF, 0x00, 0x4D],
    [0xFF, 0xA3, 0x00],
    [0xFF, 0xEC, 0x27],
    [0x00, 0xE4, 0x36],
    [0x29, 0xAD, 0xFF],
    [0x83, 0x76, 0x9C],
    [0xFF, 0x77, 0xA8],
    [0xFF, 0xCC, 0xAA],
];

/// Darkest to lightest.
const GAME_BOY: [[u8; 3]; 4] = [
    [0x0F, 0x38, 0x0F],
    [0x30, 0x62, 0x30],
    [0x8B, 0xAC, 0x0F],
    [0x9B, 0xBC, 0x0F],
];

const C64: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0xFF, 0xFF, 0xFF],
    [0x88, 0x00, 0x00],
    [0xAA, 0xFF, 0xEE],
    [0xCC, 0x44, 0xCC],
    [0x00, 0xCC, 0x55],
    [0x00, 0x00, 0xAA],
    [0xEE, 0xEE, 0x77],
    [0xDD, 0x88, 0x55],
    [0x66, 0x44, 0x00],
    [0xFF, 0x77, 0x77],
    [0x33, 0x33, 0x33],
    [0x77, 0x77, 0x77],
    [0xAA, 0xFF, 0x66],
    [0x00, 0x88, 0xFF],
    [0xBB, 0xBB, 0xBB],
];

/// 4×4 Bayer matrix: the order in which pixels in a 4×4 tile switch to the
/// next color as brightness rises.
const BAYER: [[u8; 4]; 4] = [[0, 8, 2, 10], [12, 4, 14, 6], [3, 11, 1, 9], [15, 7, 13, 5]];

const SATURATION: f32 = 1.3;
const CONTRAST: f32 = 1.15;
/// How far (in 0–255 units) dithering may nudge a pixel toward a neighboring color.
const DITHER_SPREAD: f32 = 48.0;

/// Returns `image` restyled. `Original` only gets the color boost;
/// `Terminal` without a palette falls back to PICO-8.
pub fn stylize(image: &RgbImage, style: ArtStyle, palette: Option<&Palette>) -> RgbImage {
    let theme_colors = palette.map(Palette::colors);
    let style = if style == ArtStyle::Terminal && theme_colors.is_none() {
        ArtStyle::Pico8
    } else {
        style
    };
    RgbImage::from_fn(image.width(), image.height(), |x, y| {
        let pixel = punch(image.get_pixel(x, y).0);
        let threshold = (BAYER[y as usize % 4][x as usize % 4] as f32 + 0.5) / 16.0 - 0.5;
        Rgb(match style {
            ArtStyle::Original => pixel.map(|c| c.round().clamp(0.0, 255.0) as u8),
            ArtStyle::Terminal => {
                let colors = theme_colors.as_deref().unwrap_or(&PICO8);
                nearest(colors, pixel.map(|c| c + threshold * DITHER_SPREAD))
            }
            ArtStyle::Pico8 => nearest(&PICO8, pixel.map(|c| c + threshold * DITHER_SPREAD)),
            ArtStyle::C64 => nearest(&C64, pixel.map(|c| c + threshold * DITHER_SPREAD)),
            ArtStyle::GameBoy => {
                // Four shades of one hue: map brightness, dithered between shades.
                let shades = GAME_BOY.len() as f32 - 1.0;
                let level = (luma(pixel) / 255.0 * shades + threshold).round().clamp(0.0, shades);
                GAME_BOY[level as usize]
            }
        })
    })
}

/// Boosts saturation and contrast so covers don't turn to mud in a small palette.
fn punch([r, g, b]: [u8; 3]) -> [f32; 3] {
    let rgb = [r as f32, g as f32, b as f32];
    let gray = luma(rgb);
    rgb.map(|c| ((gray + (c - gray) * SATURATION) - 128.0) * CONTRAST + 128.0)
}

fn luma([r, g, b]: [f32; 3]) -> f32 {
    0.299 * r + 0.587 * g + 0.114 * b
}

/// The palette color closest to `pixel`, by the "redmean" approximation of
/// perceived difference (cheap, and much closer to the eye than plain RGB).
fn nearest(palette: &[[u8; 3]], pixel: [f32; 3]) -> [u8; 3] {
    let distance = |color: &[u8; 3]| {
        let [r, g, b] = color.map(|c| c as f32);
        let mean_red = (pixel[0] + r) / 2.0;
        let (dr, dg, db) = (pixel[0] - r, pixel[1] - g, pixel[2] - b);
        (2.0 + mean_red / 256.0) * dr * dr + 4.0 * dg * dg + (2.0 + (255.0 - mean_red) / 256.0) * db * db
    };
    *palette
        .iter()
        .min_by(|a, b| distance(a).total_cmp(&distance(b)))
        .expect("palettes are not empty")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient() -> RgbImage {
        RgbImage::from_fn(16, 16, |x, y| {
            Rgb([(x * 16) as u8, (y * 16) as u8, ((x + y) * 8) as u8])
        })
    }

    #[test]
    fn every_pixel_comes_from_the_palette() {
        for (style, palette) in [
            (ArtStyle::Pico8, &PICO8[..]),
            (ArtStyle::GameBoy, &GAME_BOY[..]),
            (ArtStyle::C64, &C64[..]),
        ] {
            let out = stylize(&gradient(), style, None);
            assert!(
                out.pixels().all(|p| palette.contains(&p.0)),
                "{style:?} used an off-palette color"
            );
        }
    }

    #[test]
    fn exact_palette_colors_survive_and_black_stays_black() {
        let red = RgbImage::from_pixel(4, 4, Rgb([0xFF, 0x00, 0x4D]));
        assert!(
            stylize(&red, ArtStyle::Pico8, None)
                .pixels()
                .all(|p| p.0 == [0xFF, 0x00, 0x4D])
        );
        let black = RgbImage::from_pixel(4, 4, Rgb([0, 0, 0]));
        assert!(
            stylize(&black, ArtStyle::GameBoy, None)
                .pixels()
                .all(|p| p.0 == GAME_BOY[0])
        );
    }

    #[test]
    fn mid_gray_dithers_between_shades() {
        let gray = RgbImage::from_pixel(4, 4, Rgb([128, 128, 128]));
        let shades: std::collections::HashSet<_> =
            stylize(&gray, ArtStyle::GameBoy, None).pixels().map(|p| p.0).collect();
        assert!(shades.len() >= 2, "expected a dither pattern, got {shades:?}");
    }

    #[test]
    fn styles_cycle_and_skip_terminal_without_a_theme() {
        let cycle = |have_palette| {
            let mut style = ArtStyle::Pico8;
            let mut seen = vec![];
            for _ in 0..5 {
                style = style.next(have_palette);
                seen.push(style);
            }
            seen
        };
        use ArtStyle::*;
        assert_eq!(cycle(true), vec![GameBoy, C64, Original, Terminal, Pico8]);
        assert_eq!(cycle(false), vec![GameBoy, C64, Original, Pico8, GameBoy]);
    }

    #[test]
    fn terminal_style_uses_only_theme_colors() {
        let mut ansi = [[0u8; 3]; 16];
        ansi[4] = [0xbd, 0x93, 0xf9];
        ansi[5] = [0xff, 0x79, 0xc6];
        let palette = Palette {
            ansi,
            background: [0x28, 0x2a, 0x36],
            foreground: [0xf8, 0xf8, 0xf2],
        };
        let colors = palette.colors();
        let out = stylize(&gradient(), ArtStyle::Terminal, Some(&palette));
        assert!(out.pixels().all(|p| colors.contains(&p.0)));
        // Without a theme it falls back to PICO-8.
        assert!(
            stylize(&gradient(), ArtStyle::Terminal, None)
                .pixels()
                .all(|p| PICO8.contains(&p.0))
        );
    }
}
