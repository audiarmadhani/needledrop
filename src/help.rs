//! The `?` help screen: every key, grouped, in a centered dialog. Two
//! columns when there's room; `j`/`k` scroll it when there isn't.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::ui::THEME;

/// A titled group of (key, what it does).
type Section = (&'static str, &'static [(&'static str, &'static str)]);

/// Every key needledrop understands, by what it's for.
pub const SECTIONS: &[Section] = &[
    (
        "Moving around",
        &[
            ("tab / shift-tab", "switch pane"),
            ("j k  ↓ ↑", "move"),
            ("h l  ← →", "move along a shelf (card pages)"),
            ("g / G", "jump to top / bottom"),
            ("enter", "play a song, or open it"),
            ("esc / backspace", "back"),
            ("/", "search (ctrl-u clears it)"),
            ("ctrl-r", "refresh your library"),
        ],
    ),
    (
        "Playback",
        &[
            ("space", "pause / play"),
            ("n / p", "next / previous"),
            (", / .", "seek back / forward 5 s"),
            ("+ / -", "volume"),
            ("s", "shuffle"),
            ("r", "repeat: off → all → one"),
        ],
    ),
    (
        "Songs",
        &[
            ("f", "like / unlike"),
            ("a", "add to the queue"),
            ("N", "play next"),
            ("P", "add to a playlist"),
            ("R", "start a radio"),
            ("i / o", "go to artist / album"),
            ("d", "remove from your playlist"),
        ],
    ),
    (
        "Queue",
        &[
            ("J / K", "move down / up"),
            ("d", "remove"),
            ("X", "clear all but the playing song"),
        ],
    ),
    (
        "Views",
        &[
            ("L", "lyrics"),
            ("M", "minimal mode"),
            ("V", "visualizer style"),
            ("A", "album art style"),
            ("?", "this help"),
            ("q", "quit (ctrl-c anywhere)"),
        ],
    ),
];

/// Width of the key column.
const KEY_WIDTH: usize = 17;
/// Two columns need about this much room.
const TWO_COLUMN_WIDTH: u16 = 104;

fn section_lines(sections: &[Section]) -> Vec<Line<'static>> {
    let mut lines = vec![];
    for (i, (title, keys)) in sections.iter().enumerate() {
        if i > 0 {
            lines.push(Line::raw(""));
        }
        lines.push(Line::styled(*title, THEME.accent().add_modifier(Modifier::BOLD)));
        for (key, action) in *keys {
            lines.push(Line::from(vec![
                Span::styled(format!("  {key:<KEY_WIDTH$}"), THEME.accent()),
                Span::styled(*action, THEME.text()),
            ]));
        }
    }
    lines
}

/// Splits the sections into two columns of about the same height.
fn split_columns() -> (&'static [Section], &'static [Section]) {
    let height = |sections: &[Section]| section_lines(sections).len();
    let total = height(SECTIONS);
    let split = (1..SECTIONS.len())
        .min_by_key(|&at| height(&SECTIONS[..at]).abs_diff(total - height(&SECTIONS[..at])))
        .unwrap_or(SECTIONS.len());
    SECTIONS.split_at(split)
}

/// Draws the dialog. `scroll` is clamped to what can scroll.
pub fn render(frame: &mut Frame, scroll: &mut u16) {
    let area = frame.area();
    let two_columns = area.width >= TWO_COLUMN_WIDTH + 4;
    let (left, right) = if two_columns {
        split_columns()
    } else {
        (SECTIONS, &[][..])
    };
    let (left, right) = (section_lines(left), section_lines(right));
    let content_height = left.len().max(right.len()) as u16;
    let width = if two_columns {
        TWO_COLUMN_WIDTH
    } else {
        52.min(area.width.saturating_sub(2))
    };
    let height = (content_height + 2).min(area.height.saturating_sub(2));
    let dialog = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    let inner_height = height.saturating_sub(2);
    *scroll = (*scroll).min(content_height.saturating_sub(inner_height));

    frame.render_widget(Clear, dialog);
    let more = if content_height > inner_height {
        " j/k scroll "
    } else {
        ""
    };
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(THEME.accent())
        .title(Span::styled(" Keys ", THEME.title(true)))
        .title_bottom(Line::from(vec![
            Span::styled(more, THEME.muted()),
            Span::styled(" ? / esc", THEME.accent()),
            Span::styled(" close ", THEME.muted()),
        ]));
    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);
    let [left_area, right_area] = if two_columns {
        Layout::horizontal([Constraint::Fill(1), Constraint::Fill(1)]).areas(inner)
    } else {
        [inner, Rect::default()]
    };
    frame.render_widget(Paragraph::new(left).scroll((*scroll, 0)), left_area);
    if two_columns {
        frame.render_widget(Paragraph::new(right).scroll((*scroll, 0)), right_area);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_split_into_similar_heights() {
        let (left, right) = split_columns();
        assert!(!left.is_empty() && !right.is_empty());
        let (l, r) = (section_lines(left).len(), section_lines(right).len());
        assert!(l.abs_diff(r) <= 8, "{l} vs {r} lines");
        assert_eq!(left.len() + right.len(), SECTIONS.len());
    }

    #[test]
    fn descriptions_fit_a_column() {
        // Each column: half the dialog, less borders and padding, less the keys.
        let room = (TWO_COLUMN_WIDTH as usize - 2) / 2 - 2 - KEY_WIDTH;
        for (_, keys) in SECTIONS {
            for (_, action) in *keys {
                assert!(action.chars().count() <= room, "{action:?} would be cut off");
            }
        }
    }

    #[test]
    fn keys_fit_their_column() {
        for (_, keys) in SECTIONS {
            for (key, _) in *keys {
                assert!(
                    key.chars().count() < KEY_WIDTH,
                    "{key:?} is too long for the key column"
                );
            }
        }
    }
}
