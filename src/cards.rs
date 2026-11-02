//! Shelves pages (Home, artists, search) drawn as rows of cards, like
//! YouTube Music's carousels: a pixel-art cover, then a title and a detail.
//!
//! ```text
//! Listen again                                        1/15 ›
//! ╭────────────────╮
//! │▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀│ ▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀   ▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
//! │▀▀▀▀ cover ▀▀▀▀▀│ ▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀   ▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀▀
//! │Liked Music     │ Olivia Dean        Indonesian Clas…
//! │≡ Auto playlist │ ◆ 1.17M subscri…   ≡ Chrisye, Utha…
//! ╰────────────────╯
//! ```
//!
//! The selected card gets a border; the others keep the same spacing, so
//! nothing shifts as the selection moves.

use std::collections::HashSet;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::art::{PixelArt, Thumbnails};
use crate::model::Item;
use crate::pages::Shelves;
use crate::palette::Palette;
use crate::retro::ArtStyle;
use crate::ui::THEME;

/// Everything a card needs besides the shelves themselves.
pub struct Look<'a> {
    pub focused: bool,
    pub art_style: ArtStyle,
    pub palette: Option<Palette>,
    pub playing_id: Option<&'a str>,
    pub liked: &'a HashSet<String>,
}

/// Card sizes: covers are `art` columns wide and `art / 2` rows tall (one
/// cell holds two pixels, so that's square).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Size {
    art: u16,
}

impl Size {
    /// Bigger covers when there's room for at least four of them.
    fn for_width(width: u16) -> Self {
        Self {
            art: if width >= 4 * (20 + GAP) { 18 } else { 12 },
        }
    }

    /// Border, cover, border.
    fn card_width(self) -> u16 {
        self.art + 2
    }

    /// Border, cover rows, title, detail, border.
    fn card_height(self) -> u16 {
        1 + self.art / 2 + 2 + 1
    }

    /// Heading, card, a blank line.
    fn shelf_height(self) -> u16 {
        1 + self.card_height() + 1
    }
}

const GAP: u16 = 1;

pub fn render(frame: &mut Frame, area: Rect, shelves: &mut Shelves, thumbs: &mut Thumbnails, look: &Look) {
    let size = Size::for_width(area.width);
    let visible = (area.height / size.shelf_height()).max(1) as usize;
    // Scroll just enough to keep the selected shelf on screen.
    if shelves.shelf < shelves.scroll {
        shelves.scroll = shelves.shelf;
    } else if shelves.shelf >= shelves.scroll + visible {
        shelves.scroll = shelves.shelf + 1 - visible;
    }
    let per_row = ((area.width + GAP) / (size.card_width() + GAP)).max(1) as usize;
    let mut y = area.y;
    for index in shelves.scroll..shelves.shelves.len() {
        if y + size.shelf_height() > area.bottom() + 1 {
            break;
        }
        let shelf = &shelves.shelves[index];
        let column = shelves.column(index);
        let selected = (index == shelves.shelf).then_some(column);
        // Slide the row along so the remembered card is always in view.
        let first = column.saturating_sub(per_row - 1);

        let heading_area = Rect { y, height: 1, ..area };
        let style = if selected.is_some() && look.focused {
            THEME.accent().add_modifier(Modifier::BOLD)
        } else {
            THEME.text().add_modifier(Modifier::BOLD)
        };
        frame.render_widget(Paragraph::new(Line::styled(shelf.title.clone(), style)), heading_area);
        if shelf.items.len() > per_row {
            let more = format!("{}/{} ›", column + 1, shelf.items.len());
            frame.render_widget(
                Paragraph::new(Line::styled(more, THEME.muted())).alignment(Alignment::Right),
                heading_area,
            );
        }

        for (slot, item) in shelf.items.iter().enumerate().skip(first).take(per_row) {
            let x = area.x + (slot - first) as u16 * (size.card_width() + GAP);
            let card = Rect {
                x,
                y: y + 1,
                width: size.card_width(),
                height: size.card_height(),
            };
            render_card(frame, card, size, item, selected == Some(slot), thumbs, look);
        }
        y += size.shelf_height();
    }
}

fn render_card(
    frame: &mut Frame,
    area: Rect,
    size: Size,
    item: &Item,
    selected: bool,
    thumbs: &mut Thumbnails,
    look: &Look,
) {
    let border = if selected && look.focused {
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(THEME.accent())
    } else if selected {
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(THEME.muted())
    } else {
        Block::new()
    };
    let inner = Rect {
        x: area.x + 1,
        y: area.y + 1,
        width: area.width - 2,
        height: area.height - 2,
    };
    frame.render_widget(border, area);

    let [cover, title, detail] = Layout::vertical([
        Constraint::Length(size.art / 2),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(inner);
    let round = matches!(item, Item::Artist { .. });
    match item.thumbnail().and_then(|url| thumbs.get_mut(url)) {
        Some(art) => {
            let widget = PixelArt {
                style: look.art_style,
                palette: look.palette,
                round,
            };
            frame.render_stateful_widget(widget, cover, art);
        }
        None => render_placeholder(frame, cover, item, round),
    }

    let playing = matches!(item, Item::Song(t) if look.playing_id == Some(t.video_id.as_str()));
    let mut title_style = if playing {
        Style::new().fg(THEME.playing)
    } else {
        THEME.text()
    };
    if selected {
        title_style = title_style.add_modifier(Modifier::BOLD);
    }
    let mut title_spans = vec![];
    if let Item::Song(track) = item
        && look.liked.contains(&track.video_id)
    {
        title_spans.push(Span::styled("♥ ", Style::new().fg(THEME.error)));
    }
    let used = title_spans.iter().map(|s| s.width()).sum::<usize>();
    title_spans.push(Span::styled(
        fit(item.title(), title.width as usize - used),
        title_style,
    ));
    frame.render_widget(Paragraph::new(Line::from(title_spans)), title);

    let detail_text = format!("{} {}", symbol(item), item.subtitle());
    frame.render_widget(
        Paragraph::new(Line::styled(fit(&detail_text, detail.width as usize), THEME.muted())),
        detail,
    );
}

/// While a cover loads (or if it can't): a dim tile with the item's symbol.
fn render_placeholder(frame: &mut Frame, area: Rect, item: &Item, round: bool) {
    let side = area.width.min(area.height * 2);
    let tile = Rect {
        width: side,
        height: side / 2,
        ..area
    };
    if let Item::Category { title, color, .. } = item {
        return render_category(frame, tile, title, *color);
    }
    if !round {
        frame.render_widget(Block::new().style(Style::new().bg(THEME.selection_bg)), tile);
    }
    let [_, middle, _] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1), Constraint::Fill(1)]).areas(tile);
    frame.render_widget(
        Paragraph::new(symbol(item))
            .style(THEME.muted())
            .alignment(Alignment::Center),
        middle,
    );
}

/// A category's tile: its name on YouTube's color for it, in black or white,
/// whichever reads better on that color.
fn render_category(frame: &mut Frame, tile: Rect, title: &str, color: Option<[u8; 3]>) {
    let Some([r, g, b]) = color else {
        frame.render_widget(Block::new().style(Style::new().bg(THEME.selection_bg)), tile);
        let [_, middle, _] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1), Constraint::Fill(1)]).areas(tile);
        let name = Paragraph::new(fit(title, tile.width as usize - 2)).alignment(Alignment::Center);
        frame.render_widget(name, middle);
        return;
    };
    let light = 0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32 > 150.0;
    let text = if light {
        Color::Rgb(0, 0, 0)
    } else {
        Color::Rgb(255, 255, 255)
    };
    frame.render_widget(Block::new().style(Style::new().bg(Color::Rgb(r, g, b))), tile);
    let [_, middle, _] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1), Constraint::Fill(1)]).areas(tile);
    let name = Paragraph::new(fit(title, tile.width.saturating_sub(2) as usize))
        .style(Style::new().fg(text).add_modifier(Modifier::BOLD))
        .alignment(Alignment::Center);
    frame.render_widget(name, middle);
}

fn symbol(item: &Item) -> &'static str {
    match item {
        Item::Song(_) => "♪",
        Item::Album { .. } => "◉",
        Item::Artist { .. } => "◆",
        Item::Playlist { .. } => "≡",
        Item::Category { .. } => "#",
    }
}

/// `text` cut to `width` columns, with an ellipsis when it doesn't fit.
fn fit(text: &str, width: usize) -> String {
    if text.width() <= width {
        return text.to_owned();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in text.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > width {
            break;
        }
        out.push(c);
        used += w;
    }
    out.push('…');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_text_with_an_ellipsis() {
        assert_eq!(fit("Liked Music", 16), "Liked Music");
        assert_eq!(fit("Indonesian Classic Disco", 16), "Indonesian Clas…");
        assert_eq!(fit("Indonesian Classic Disco", 16).width(), 16);
    }

    #[test]
    fn cards_get_smaller_in_narrow_panes() {
        assert_eq!(Size::for_width(120).art, 18);
        assert_eq!(Size::for_width(60).art, 12);
        let size = Size { art: 18 };
        assert_eq!((size.card_width(), size.card_height()), (20, 13));
    }
}
