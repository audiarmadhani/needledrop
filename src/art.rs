//! Album art, drawn as pixel art from half-block characters: each terminal
//! cell shows two vertically stacked pixels (`▀` with the top pixel as the
//! foreground and the bottom one as the background). Cells are about twice
//! as tall as wide, so the pixels come out square.
//!
//! Being plain colored text, the art survives resizes and works in every
//! terminal and multiplexer, unlike graphics protocols.

use std::sync::LazyLock;

use anyhow::Result;
use image::imageops::FilterType;
use image::{DynamicImage, RgbImage};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::StatefulWidget;

use crate::palette::Palette;
use crate::retro::{self, ArtStyle};

/// Source resolution kept in memory; far more than any terminal shows.
const SOURCE_SIZE: u32 = 128;

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(reqwest::Client::new);

pub async fn fetch(url: &str) -> Result<DynamicImage> {
    let bytes = HTTP
        .get(high_res_url(url))
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    let image = image::load_from_memory(&bytes)?;
    let side = image.width().min(image.height());
    let square = image.crop_imm((image.width() - side) / 2, (image.height() - side) / 2, side, side);
    Ok(square.resize_exact(SOURCE_SIZE, SOURCE_SIZE, FilterType::Lanczos3))
}

fn high_res_url(url: &str) -> String {
    sized_url(url, SOURCE_SIZE)
}

/// YouTube Music returns 60–120 px thumbnails. Google's image server takes
/// the size in the URL suffix (`=w120-h120-l90-rj`), so ask for `size` px.
pub fn sized_url(url: &str, size: u32) -> String {
    match url.rfind('=') {
        Some(i) if url.contains("googleusercontent.com") => format!("{}=w{size}-h{size}-l90-rj", &url[..i]),
        _ => url.to_owned(),
    }
}

/// What the cached pixels were made for.
type CacheKey = (u32, ArtStyle, Option<Palette>);

/// A cover image plus its restyled pixels for the last size, style, and
/// theme drawn.
pub struct AlbumArt {
    source: RgbImage,
    pixels: Option<(CacheKey, RgbImage)>,
}

impl AlbumArt {
    pub fn new(image: DynamicImage) -> Self {
        Self {
            source: image.into_rgb8(),
            pixels: None,
        }
    }

    /// The image at `side`×`side` pixels in `style`, reusing the last result.
    fn pixels(&mut self, side: u32, style: ArtStyle, palette: Option<Palette>) -> &RgbImage {
        let key = (side, style, palette);
        if self.pixels.as_ref().is_none_or(|(cached, _)| *cached != key) {
            // Averaging (not nearest-neighbour) keeps each block's color true to the cover.
            let scaled = image::imageops::resize(&self.source, side, side, FilterType::Triangle);
            self.pixels = Some((key, retro::stylize(&scaled, style, palette.as_ref())));
        }
        &self.pixels.as_ref().expect("just filled").1
    }
}

/// Draws `AlbumArt` as the largest square that fits, centered.
pub struct PixelArt {
    pub style: ArtStyle,
    /// The terminal theme, for `ArtStyle::Terminal`.
    pub palette: Option<Palette>,
}

impl StatefulWidget for PixelArt {
    type State = AlbumArt;

    fn render(self, area: Rect, buf: &mut Buffer, art: &mut AlbumArt) {
        // One cell is one pixel wide and two pixels tall.
        let side = area.width.min(area.height * 2) as u32;
        if side == 0 {
            return;
        }
        let pixels = art.pixels(side, self.style, self.palette);
        let rows = side.div_ceil(2) as u16;
        let left = area.x + (area.width - side as u16) / 2;
        let top = area.y + (area.height - rows) / 2;
        let color = |x: u32, y: u32| {
            let [r, g, b] = pixels.get_pixel(x, y).0;
            Color::Rgb(r, g, b)
        };
        for row in 0..rows {
            for x in 0..side {
                let upper = color(x, row as u32 * 2);
                // An odd height leaves the last row's lower half empty.
                let lower = (row as u32 * 2 + 1 < side).then(|| color(x, row as u32 * 2 + 1));
                let cell = &mut buf[(left + x as u16, top + row)];
                cell.set_symbol("▀").set_fg(upper);
                if let Some(lower) = lower {
                    cell.set_bg(lower);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asks_google_for_a_larger_thumbnail() {
        assert_eq!(
            high_res_url("https://lh3.googleusercontent.com/abc=w120-h120-l90-rj"),
            "https://lh3.googleusercontent.com/abc=w128-h128-l90-rj"
        );
        let video = "https://i.ytimg.com/vi/x/sddefault.jpg";
        assert_eq!(high_res_url(video), video);
    }

    #[test]
    fn draws_two_pixels_per_cell_as_a_centered_square() {
        // Top row PICO-8 red, bottom row PICO-8 blue: already the drawn size
        // and already in the palette, so they come through unchanged.
        let image = RgbImage::from_fn(2, 2, |_, y| {
            if y == 0 {
                image::Rgb([0xFF, 0x00, 0x4D])
            } else {
                image::Rgb([0x29, 0xAD, 0xFF])
            }
        });
        let mut art = AlbumArt::new(DynamicImage::ImageRgb8(image));
        let area = Rect::new(0, 0, 6, 1);
        let mut buf = Buffer::empty(area);
        PixelArt {
            style: ArtStyle::Pico8,
            palette: None,
        }
        .render(area, &mut buf, &mut art);
        // A 1-row area fits a 2×2 pixel square, centered in 6 columns.
        let cells: Vec<_> = (0..6)
            .map(|x| (buf[(x, 0)].symbol().to_owned(), buf[(x, 0)].fg, buf[(x, 0)].bg))
            .collect();
        let expected = (
            "▀".to_owned(),
            Color::Rgb(0xFF, 0x00, 0x4D),
            Color::Rgb(0x29, 0xAD, 0xFF),
        );
        assert_eq!(cells[2], expected);
        assert_eq!(cells[3], expected);
        assert_eq!(cells[0].0, " ");
        assert_eq!(cells[5].0, " ");
    }
}
