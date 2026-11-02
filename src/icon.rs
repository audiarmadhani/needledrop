//! needledrop's app icon: a vinyl record with the tonearm dropping its
//! needle, as 32×32 pixel art in the PICO-8 palette, on a rounded square.
//! Drawn in code so the source of truth is reviewable, and scaled up with
//! nearest-neighbour so the pixels stay crisp at every icon size.

use image::{Rgba, RgbaImage};

pub const GRID: u32 = 32;

const SKY_TOP: Rgba<u8> = Rgba([0x29, 0x36, 0x6F, 255]);
const SKY: Rgba<u8> = Rgba([0x1D, 0x2B, 0x53, 255]);
const EDGE: Rgba<u8> = Rgba([0x12, 0x1A, 0x33, 255]);
const VINYL: Rgba<u8> = Rgba([0x0E, 0x0E, 0x14, 255]);
const GROOVE: Rgba<u8> = Rgba([0x2C, 0x2C, 0x3A, 255]);
const SHINE: Rgba<u8> = Rgba([0x83, 0x76, 0x9C, 255]);
const LABEL: Rgba<u8> = Rgba([0xFF, 0x00, 0x4D, 255]);
const LABEL_LIGHT: Rgba<u8> = Rgba([0xFF, 0x77, 0xA8, 255]);
const SPINDLE: Rgba<u8> = Rgba([0xFF, 0xF1, 0xE8, 255]);
const ARM: Rgba<u8> = Rgba([0xC2, 0xC3, 0xC7, 255]);
const ARM_SHADE: Rgba<u8> = Rgba([0x5F, 0x57, 0x4F, 255]);
const HEAD: Rgba<u8> = Rgba([0xFF, 0xA3, 0x00, 255]);

/// The icon at its native 32×32. The background runs edge to edge: macOS
/// rounds icons itself, and boxes in any icon that brings its own shape.
pub fn pixels() -> RgbaImage {
    let mut img = RgbaImage::from_fn(GRID, GRID, |_, y| if y < 2 { SKY_TOP } else { SKY });

    // The record, left of center and low, leaving the top right for the arm.
    let (cx, cy) = (14.5, 17.0);
    for y in 0..GRID {
        for x in 0..GRID {
            let d = ((x as f32 - cx).powi(2) + (y as f32 - cy).powi(2)).sqrt();
            let color = match d {
                d if d < 1.3 => SPINDLE,
                d if d < 4.2 => {
                    // The label, lit from the top left.
                    if (x as f32) < cx && (y as f32) < cy {
                        LABEL_LIGHT
                    } else {
                        LABEL
                    }
                }
                d if d < 11.6 => {
                    let groove = (d - 6.8).abs() < 0.45 || (d - 9.4).abs() < 0.45;
                    if groove { GROOVE } else { VINYL }
                }
                d if d < 12.4 => EDGE,
                _ => continue,
            };
            img.put_pixel(x, y, color);
        }
    }
    // A glint on the vinyl.
    for (x, y) in [(7, 11), (8, 10), (9, 9), (6, 13)] {
        img.put_pixel(x, y, SHINE);
    }

    // The tonearm: pivot top right, arm sweeping down onto the record.
    for (x, y) in [(26, 4), (27, 4), (26, 5), (27, 5)] {
        img.put_pixel(x, y, ARM);
    }
    for (x, y) in [(25, 4), (28, 5), (26, 6), (27, 6)] {
        img.put_pixel(x, y, ARM_SHADE);
    }
    for (x, y) in [(26, 7), (26, 8), (25, 9), (25, 10), (24, 11), (24, 12)] {
        img.put_pixel(x, y, ARM);
        img.put_pixel(x + 1, y, ARM_SHADE);
    }
    // The headshell, with the needle just touching down.
    for (x, y) in [(22, 13), (23, 13), (23, 14), (22, 14)] {
        img.put_pixel(x, y, HEAD);
    }
    img.put_pixel(21, 15, SPINDLE);
    img
}

/// The icon at `size`×`size`, pixels kept square and sharp.
pub fn scaled(size: u32) -> RgbaImage {
    image::imageops::resize(&pixels(), size, size, image::imageops::FilterType::Nearest)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn background_is_edge_to_edge_and_the_record_has_its_label() {
        let icon = pixels();
        assert!(
            icon.pixels().all(|p| p.0[3] == 255),
            "no transparency: macOS does the rounding"
        );
        assert_eq!(icon.get_pixel(14, 17), &SPINDLE);
        assert_eq!(icon.get_pixel(14, 14), &LABEL_LIGHT);
    }

    #[test]
    fn scaling_keeps_pixels_sharp() {
        let big = scaled(GRID * 4);
        assert_eq!(big.get_pixel(14 * 4, 16 * 4), &SPINDLE);
        assert_eq!(
            big.get_pixel(14 * 4 + 3, 16 * 4 + 3),
            &SPINDLE,
            "a whole 4×4 block, not a blur"
        );
    }
}
