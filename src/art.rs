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
    /// Clip to a circle, the way YouTube Music shows artists.
    pub round: bool,
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
        let radius = side as f32 / 2.0;
        let shown = |x: u32, y: u32| {
            y < side
                && (!self.round || {
                    let (dx, dy) = (x as f32 + 0.5 - radius, y as f32 + 0.5 - radius);
                    dx * dx + dy * dy <= radius * radius
                })
        };
        for row in 0..rows {
            for x in 0..side {
                let (top_y, bottom_y) = (row as u32 * 2, row as u32 * 2 + 1);
                let cell = &mut buf[(left + x as u16, top + row)];
                // Each cell shows two stacked pixels; draw whichever are inside.
                match (shown(x, top_y), shown(x, bottom_y)) {
                    (true, true) => {
                        cell.set_symbol("▀").set_fg(color(x, top_y)).set_bg(color(x, bottom_y));
                    }
                    (true, false) => {
                        cell.set_symbol("▀").set_fg(color(x, top_y));
                    }
                    (false, true) => {
                        cell.set_symbol("▄").set_fg(color(x, bottom_y));
                    }
                    (false, false) => {}
                }
            }
        }
    }
}

enum Thumb {
    Loading,
    Ready(Box<AlbumArt>),
    Failed,
}

/// Covers for the cards on browse pages, by URL, fetched once each.
#[derive(Default)]
pub struct Thumbnails {
    covers: std::collections::HashMap<String, Thumb>,
}

/// Plenty for several pages of shelves; beyond it the cache starts over.
const MAX_THUMBNAILS: usize = 600;

impl Thumbnails {
    /// Marks the URLs not seen before as loading and returns them, to fetch.
    pub fn claim<'a>(&mut self, urls: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        if self.covers.len() > MAX_THUMBNAILS {
            self.covers.retain(|_, thumb| matches!(thumb, Thumb::Loading));
        }
        let mut new = Vec::new();
        for url in urls {
            // Checked one at a time, so a cover on two shelves is fetched once.
            if !self.covers.contains_key(url) {
                self.covers.insert(url.to_owned(), Thumb::Loading);
                new.push(url.to_owned());
            }
        }
        new
    }

    pub fn arrived(&mut self, url: String, image: Option<DynamicImage>) {
        let thumb = image.map_or(Thumb::Failed, |image| Thumb::Ready(Box::new(AlbumArt::new(image))));
        self.covers.insert(url, thumb);
    }

    pub fn get_mut(&mut self, url: &str) -> Option<&mut AlbumArt> {
        match self.covers.get_mut(url)? {
            Thumb::Ready(art) => Some(art),
            Thumb::Loading | Thumb::Failed => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_art_leaves_the_corners_alone() {
        let image = RgbImage::from_pixel(8, 8, image::Rgb([255, 0, 0]));
        let mut art = AlbumArt::new(DynamicImage::ImageRgb8(image));
        let area = Rect::new(0, 0, 8, 4);
        let mut buf = Buffer::empty(area);
        PixelArt {
            style: ArtStyle::Original,
            palette: None,
            round: true,
        }
        .render(area, &mut buf, &mut art);
        assert_eq!(buf[(0, 0)].symbol(), " ", "corner outside the circle");
        assert_eq!(buf[(4, 2)].symbol(), "▀", "center inside");
    }

    #[test]
    fn thumbnails_are_claimed_once() {
        let mut thumbs = Thumbnails::default();
        assert_eq!(thumbs.claim(["a", "b"]), vec!["a", "b"]);
        assert_eq!(
            thumbs.claim(["a", "c", "c"]),
            vec!["c"],
            "a is on its way; c is claimed once"
        );
        assert!(thumbs.get_mut("a").is_none(), "still loading");
        thumbs.arrived("a".into(), Some(DynamicImage::new_rgb8(4, 4)));
        assert!(thumbs.get_mut("a").is_some());
        thumbs.arrived("b".into(), None);
        assert!(thumbs.get_mut("b").is_none(), "failed covers stay placeholders");
    }

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
            round: false,
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
