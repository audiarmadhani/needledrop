use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, LineGauge, List, ListItem, ListState, Paragraph, Row, Table};

use unicode_width::UnicodeWidthStr;

use crate::app::{App, Focus, Library, LyricsState, Repeat};
use crate::art::PixelArt;
use crate::cards;
use crate::lyrics::Lyrics;
use crate::minimal;
use crate::pages::Content;
use crate::theme::Theme;
use crate::visualizer::Visual;

pub(crate) const THEME: Theme = Theme::TERMINAL;
const NOW_PLAYING_HEIGHT: u16 = 12;
/// Below this width, open lyrics hide the side panels entirely.
const LYRICS_FULL_WIDTH_BELOW: u16 = 90;

pub fn render(app: &mut App, frame: &mut Frame) {
    if app.minimal {
        return minimal::render(app, frame);
    }
    let [search, main, now_playing, footer] = Layout::vertical([
        Constraint::Length(3),
        Constraint::Min(6),
        Constraint::Length(NOW_PLAYING_HEIGHT),
        Constraint::Length(1),
    ])
    .areas(frame.area());
    let (library_width, queue_width) = side_panel_widths(app.show_lyrics, main.width);
    let [library, tracks, queue] = Layout::horizontal([
        Constraint::Length(library_width),
        Constraint::Fill(1),
        Constraint::Length(queue_width),
    ])
    .areas(main);

    render_search(app, frame, search);
    if !library.is_empty() {
        render_library(app, frame, library);
    }
    render_tracks(app, frame, tracks);
    if !queue.is_empty() {
        render_queue(app, frame, queue);
    }
    render_now_playing(app, frame, now_playing);
    render_footer(app, frame, footer);
    if app.picker.is_some() {
        render_picker(app, frame);
    }
}

/// The add-to-playlist dialog, centered over everything.
fn render_picker(app: &App, frame: &mut Frame) {
    let Some(picker) = &app.picker else { return };
    let area = frame.area();
    let width = 56.min(area.width.saturating_sub(4));
    let height = (picker.playlists.len() as u16 + 6)
        .min(area.height.saturating_sub(4))
        .max(8);
    let dialog = Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, dialog);
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(THEME.accent())
        .title(Span::styled(" Add to playlist ", THEME.title(true)))
        .title_bottom(
            Line::from(if picker.naming.is_some() {
                vec![
                    Span::styled(" enter", THEME.accent()),
                    Span::styled(" create ", THEME.muted()),
                    Span::styled(" esc", THEME.accent()),
                    Span::styled(" back ", THEME.muted()),
                ]
            } else {
                vec![
                    Span::styled(" enter", THEME.accent()),
                    Span::styled(" add ", THEME.muted()),
                    Span::styled(" esc", THEME.accent()),
                    Span::styled(" cancel ", THEME.muted()),
                ]
            })
            .alignment(Alignment::Right),
        );
    let inner = block.inner(dialog);
    frame.render_widget(block, dialog);
    let [song, _, list] =
        Layout::vertical([Constraint::Length(1), Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    let song_line = Line::from(vec![
        Span::styled("♪ ", THEME.accent()),
        Span::styled(picker.track.title.clone(), THEME.text().add_modifier(Modifier::BOLD)),
        Span::styled(format!("  {}", picker.track.artists), THEME.muted()),
    ]);
    frame.render_widget(Paragraph::new(song_line), song);

    if let Some(name) = &picker.naming {
        let field = Line::from(vec![
            Span::styled("New playlist name: ", THEME.muted()),
            Span::styled(name.clone(), THEME.text()),
        ]);
        frame.render_widget(Paragraph::new(field), list);
        let x = list.x + "New playlist name: ".len() as u16 + name.width() as u16;
        frame.set_cursor_position(Position::new(x.min(list.right().saturating_sub(1)), list.y));
        return;
    }
    let new = ListItem::new(Line::from(vec![
        Span::styled("+ ", THEME.accent()),
        Span::styled("New playlist", THEME.text()),
    ]));
    let items = std::iter::once(new).chain(picker.playlists.iter().map(|playlist| {
        // "Audi Armadhani • 123 tracks" → "123 tracks"
        let count = playlist.subtitle.rsplit(" • ").next().unwrap_or_default().to_owned();
        ListItem::new(Line::from(vec![
            Span::styled("≡ ", THEME.muted()),
            Span::styled(playlist.title.clone(), THEME.text()),
            Span::styled(format!("  {count}"), THEME.muted()),
        ]))
    }));
    let mut state = ListState::default();
    state.select(Some(picker.selected));
    frame.render_stateful_widget(List::new(items).highlight_style(THEME.selection()), list, &mut state);
}

/// Library and queue widths. Lyrics need room more than the side panels
/// do, so they shrink while lyrics are open, and vanish in narrow panes.
fn side_panel_widths(lyrics: bool, width: u16) -> (u16, u16) {
    match (lyrics, width) {
        (false, _) => (28, width * 28 / 100),
        (true, width) if width < LYRICS_FULL_WIDTH_BELOW => (0, 0),
        (true, _) => (18, 24),
    }
}

/// Word-wraps `text` to `width` columns; words longer than a line are split.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut lines = vec![];
    let mut line = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_owned();
        while word.width() > width {
            let split = word
                .char_indices()
                .map(|(i, _)| i)
                .take_while(|&i| word[..i].width() <= width)
                .last()
                .unwrap_or(1);
            let rest = word.split_off(split.max(1));
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            lines.push(std::mem::replace(&mut word, rest));
        }
        let needed = if line.is_empty() {
            word.width()
        } else {
            line.width() + 1 + word.width()
        };
        if needed > width && !line.is_empty() {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&word);
    }
    if !line.is_empty() || lines.is_empty() {
        lines.push(line);
    }
    lines
}

/// The lyric text to show for a line (`♪` for instrumental gaps).
pub(crate) fn lyric_text(text: &str) -> &str {
    if text.is_empty() { "♪" } else { text }
}

/// The lyrics for what's playing, when loaded.
pub(crate) fn current_lyrics(app: &App) -> Option<&Lyrics> {
    let track = app.now_playing()?;
    match &app.lyrics {
        Some((id, LyricsState::Found(lyrics))) if *id == track.video_id => Some(lyrics),
        _ => None,
    }
}

pub(crate) fn visual(app: &App) -> Visual<'_> {
    Visual {
        visualizer: &app.visualizer,
        colors: &THEME.spectrum,
        accent: THEME.text,
    }
}

fn panel(title: impl std::fmt::Display, focused: bool) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(THEME.border(focused))
        .title(Span::styled(format!(" {title} "), THEME.title(focused)))
}

fn render_search(app: &App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Search;
    let mut brand = vec![
        Span::styled(" ◉ ", THEME.accent()),
        Span::styled("needledrop ", THEME.title(true)),
    ];
    if let Some(account) = &app.account {
        brand.push(Span::styled(format!("· {} ", account.name), THEME.muted()));
    }
    let block = panel("search", focused).title(Line::from(brand).alignment(Alignment::Right));
    let text = if app.query.is_empty() && !focused {
        Line::styled("Press / to search YouTube Music", THEME.muted())
    } else {
        Line::from(vec![
            Span::styled("› ", THEME.accent()),
            Span::styled(&app.query, THEME.text()),
        ])
    };
    let inner = block.inner(area);
    frame.render_widget(Paragraph::new(text).block(block), area);
    if focused {
        let x = inner.x + 2 + app.query.chars().count() as u16;
        frame.set_cursor_position(Position::new(x.min(inner.right().saturating_sub(1)), inner.y));
    }
}

fn render_library(app: &mut App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Library;
    let block = panel("library", focused);
    let message = match &app.library {
        Library::Loaded(playlists) if !playlists.is_empty() => None,
        Library::Loaded(_) => Some(("Your library is empty".to_owned(), THEME.muted())),
        Library::Loading => Some(("Loading…".to_owned(), THEME.muted())),
        Library::Failed(error) => Some((format!("Couldn't load library: {error}"), Style::new().fg(THEME.error))),
        Library::SignedOut => Some((
            "Not signed in.\n\nRun\nneedledrop login --browser <name>\nto see your playlists.".to_owned(),
            THEME.muted(),
        )),
    };
    if let Some((text, style)) = message {
        let paragraph = Paragraph::new(text)
            .style(style)
            .wrap(ratatui::widgets::Wrap { trim: true });
        frame.render_widget(paragraph.block(block), area);
        return;
    }
    let home = ListItem::new(Line::from(vec![Span::styled("⌂ ", THEME.accent()), Span::raw("Home")]));
    let items = std::iter::once(home).chain(
        app.library
            .playlists()
            .iter()
            .map(|playlist| ListItem::new(playlist.title.as_str())),
    );
    let list = List::new(items)
        .block(block)
        .highlight_style(if focused { THEME.selection() } else { Style::new() });
    frame.render_stateful_widget(list, area, &mut app.library_state);
}

fn render_lyrics(app: &App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Tracks;
    let track = app.now_playing();
    let mut block = panel(
        format!("lyrics{}", track.map(|t| format!(" · {}", t.title)).unwrap_or_default()),
        focused,
    );
    let inner = block.inner(area);
    let message = |text: &str| {
        Paragraph::new(text.to_owned())
            .style(THEME.muted())
            .alignment(Alignment::Center)
    };
    let centered = |height: u16| {
        let [_, middle, _] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(height), Constraint::Fill(1)]).areas(inner);
        middle
    };

    let lyrics = match (track, &app.lyrics) {
        (None, _) => {
            frame.render_widget(block, area);
            frame.render_widget(message("Play something to see its lyrics"), centered(1));
            return;
        }
        (Some(track), Some((id, LyricsState::Found(lyrics)))) if *id == track.video_id => lyrics,
        (Some(_), state) => {
            let text = match state {
                Some((_, LyricsState::Missing)) => "No lyrics for this song".to_owned(),
                Some((_, LyricsState::Failed(error))) => format!("Couldn't load lyrics: {error}"),
                _ => "Loading lyrics…".to_owned(),
            };
            frame.render_widget(block, area);
            frame.render_widget(message(&text).wrap(ratatui::widgets::Wrap { trim: true }), centered(2));
            return;
        }
    };

    block = block.title_bottom(Line::styled(format!(" {} ", lyrics.source), THEME.muted()).alignment(Alignment::Right));
    frame.render_widget(block, area);

    let width = inner.width as usize;
    let wrapped = |text: &str, style: Style| -> Vec<Line<'static>> {
        wrap(lyric_text(text), width)
            .into_iter()
            .map(|row| Line::styled(row, style))
            .collect()
    };
    if !lyrics.synced {
        let lines: Vec<Line> = lyrics
            .lines
            .iter()
            .flat_map(|l| wrapped(&l.text, THEME.text()))
            .collect();
        let paragraph = Paragraph::new(lines)
            .alignment(Alignment::Center)
            .scroll((app.lyrics_scroll, 0));
        frame.render_widget(paragraph, inner);
        return;
    }
    // Synced: past lines dimmed, the sung line highlighted, upcoming ones plain.
    let current = lyrics.current_line(app.playback.current_position());
    let mut rows: Vec<Line> = vec![];
    let mut current_row = 0;
    for (i, line) in lyrics.lines.iter().enumerate() {
        let style = match current.map(|c| i.cmp(&c)) {
            Some(std::cmp::Ordering::Less) => THEME.muted(),
            Some(std::cmp::Ordering::Equal) => {
                current_row = rows.len();
                THEME.accent().add_modifier(Modifier::BOLD)
            }
            // Before the first line, everything is still to come; show it dimmed.
            None => THEME.muted(),
            Some(std::cmp::Ordering::Greater) => THEME.text(),
        };
        rows.extend(wrapped(&line.text, style));
    }
    // Keep the sung line a third of the way down, with what's coming below it.
    let top = current_row.saturating_sub(inner.height as usize / 3);
    let rows: Vec<Line> = rows.into_iter().skip(top).take(inner.height as usize).collect();
    frame.render_widget(Paragraph::new(rows).alignment(Alignment::Center), inner);
}

fn render_tracks(app: &mut App, frame: &mut Frame, area: Rect) {
    if app.show_lyrics {
        return render_lyrics(app, frame, area);
    }
    let focused = app.focus == Focus::Tracks;
    let can_go_back = app.nav.can_go_back();
    let playing_id = app.current.and_then(|i| app.queue.get(i)).map(|t| t.video_id.as_str());
    let liked = &app.liked;
    let Some(page) = app.nav.current_mut() else {
        let block = panel("tracks", focused);
        frame.render_widget(block, area);
        return;
    };
    let count = match &page.content {
        Content::Tracks { tracks, .. } => format!(" · {}", tracks.len()),
        Content::Shelves { .. } => String::new(),
    };
    let title = if page.loading {
        format!("{} · loading…", page.title)
    } else {
        format!("{}{count}", page.title)
    };
    let mut block = panel(title, focused);
    if let Some(subtitle) = &page.subtitle {
        block = block.title(Line::styled(format!(" {subtitle} "), THEME.muted()).alignment(Alignment::Right));
    }
    if can_go_back {
        block = block.title_bottom(Line::from(vec![
            Span::styled(" esc", THEME.accent()),
            Span::styled(" back ", THEME.muted()),
        ]));
    }

    if page.is_empty() {
        let hint = if page.loading { "Loading…" } else { "Nothing here" };
        let inner = block.inner(area);
        frame.render_widget(block, area);
        let [_, middle, _] =
            Layout::vertical([Constraint::Fill(1), Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
        frame.render_widget(
            Paragraph::new(hint).style(THEME.muted()).alignment(Alignment::Center),
            middle,
        );
        return;
    }

    let highlight = if focused { THEME.selection() } else { Style::new() };
    let (tracks, state) = match &mut page.content {
        Content::Tracks { tracks, state } => (tracks, state),
        Content::Shelves(shelves) => {
            let inner = block.inner(area);
            frame.render_widget(block, area);
            let look = cards::Look {
                focused,
                art_style: app.art_style,
                palette: app.palette,
                playing_id,
                liked,
            };
            cards::render(frame, inner, shelves, &mut app.thumbnails, &look);
            return;
        }
    };

    let rows = tracks.iter().map(|track| {
        let is_playing = playing_id == Some(track.video_id.as_str());
        let marker = if is_playing {
            Span::styled("▶", Style::new().fg(THEME.playing))
        } else {
            Span::raw(" ")
        };
        let title_style = if is_playing {
            Style::new().fg(THEME.playing)
        } else {
            THEME.text()
        };
        Row::new(vec![
            Line::from(marker),
            Line::from(heart(liked.contains(&track.video_id))),
            Line::styled(track.title.as_str(), title_style),
            Line::styled(track.artists.as_str(), THEME.muted()),
            Line::styled(track.album.as_deref().unwrap_or(""), THEME.muted()),
            Line::styled(
                track.duration.map(|d| format_time(d.as_secs_f64())).unwrap_or_default(),
                THEME.muted(),
            )
            .alignment(Alignment::Right),
        ])
    });
    let header = Row::new(["", "", "Title", "Artist", "Album", "Time"])
        .style(THEME.muted().add_modifier(Modifier::BOLD))
        .bottom_margin(1);
    let table = Table::new(
        rows,
        [
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(3),
            Constraint::Fill(2),
            Constraint::Fill(2),
            Constraint::Length(7),
        ],
    )
    .header(header)
    .column_spacing(2)
    .block(block)
    .row_highlight_style(highlight);
    frame.render_stateful_widget(table, area, state);
}

fn render_queue(app: &mut App, frame: &mut Frame, area: Rect) {
    let focused = app.focus == Focus::Queue;
    let block = panel(format!("queue · {}", app.queue.len()), focused);
    let items = app.queue.iter().enumerate().map(|(i, track)| {
        let is_current = app.current == Some(i);
        let (marker, style) = if is_current {
            ("▶ ", Style::new().fg(THEME.playing))
        } else {
            ("  ", THEME.text())
        };
        ListItem::new(Line::from(vec![
            Span::styled(marker, style),
            heart(app.liked.contains(&track.video_id)),
            Span::raw(" "),
            Span::styled(track.title.as_str(), style),
            Span::styled(format!("  {}", track.artists), THEME.muted()),
        ]))
    });
    let list = List::new(items)
        .block(block)
        .highlight_style(if focused { THEME.selection() } else { Style::new() });
    frame.render_stateful_widget(list, area, &mut app.queue_state);
}

fn render_now_playing(app: &mut App, frame: &mut Frame, area: Rect) {
    let block = panel("now playing", false);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    // Terminal cells are about twice as tall as wide, so 2 columns per row is square.
    let [art_area, _, details] = Layout::horizontal([
        Constraint::Length(inner.height * 2),
        Constraint::Length(2),
        Constraint::Fill(1),
    ])
    .areas(inner);
    match &mut app.art {
        Some(art) => frame.render_stateful_widget(
            PixelArt {
                style: app.art_style,
                palette: app.palette,
                round: false,
            },
            art_area,
            art,
        ),
        None => render_art_placeholder(frame, art_area),
    }

    let [info, subtitle, _, spectrum, progress] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Fill(1),
        Constraint::Length(1),
    ])
    .areas(details);

    frame.render_widget(visual(app), spectrum);

    let Some(track) = app.now_playing() else {
        frame.render_widget(Paragraph::new("Nothing playing").style(THEME.muted()), info);
        return;
    };
    let playback = &app.playback;
    let title = Line::from(vec![
        Span::styled(if playback.paused { "⏸  " } else { "▶  " }, THEME.accent()),
        Span::styled(track.title.as_str(), THEME.text().add_modifier(Modifier::BOLD)),
        Span::raw(" "),
        heart(app.liked.contains(&track.video_id)),
    ]);
    let toggle = |on: bool| if on { THEME.accent() } else { THEME.muted() };
    let modes = Line::from(vec![
        Span::styled("⇄ ", toggle(app.shuffle)),
        Span::styled(
            if app.repeat == Repeat::One { "↻¹ " } else { "↻  " },
            toggle(app.repeat != Repeat::Off),
        ),
        Span::styled(format!("vol {:>3.0}% ", playback.volume), THEME.muted()),
    ]);
    let [title_area, modes_area] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(modes.width() as u16)]).areas(info);
    frame.render_widget(Paragraph::new(title), title_area);
    frame.render_widget(Paragraph::new(modes), modes_area);

    let mut byline = track.artists.clone();
    if let Some(album) = &track.album {
        byline.push_str(&format!(" · {album}"));
    }
    frame.render_widget(Paragraph::new(format!("   {byline}")).style(THEME.muted()), subtitle);

    let position = playback.current_position().min(playback.duration.max(0.0));
    let ratio = if playback.duration > 0.0 {
        (position / playback.duration).clamp(0.0, 1.0)
    } else {
        0.0
    };
    let label = format!("{} / {} ", format_time(position), format_time(playback.duration));
    let gauge = LineGauge::default()
        .ratio(ratio)
        .label(Span::styled(label, THEME.muted()))
        .filled_style(THEME.accent())
        .unfilled_style(Style::new().fg(THEME.border));
    frame.render_widget(gauge, progress);
}

pub(crate) fn render_art_placeholder(frame: &mut Frame, area: Rect) {
    let block = Block::new().style(Style::new().bg(THEME.selection_bg));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let [_, middle, _] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(1), Constraint::Fill(1)]).areas(inner);
    frame.render_widget(
        Paragraph::new("♪").style(THEME.muted()).alignment(Alignment::Center),
        middle,
    );
}

fn render_footer(app: &App, frame: &mut Frame, area: Rect) {
    if let Some(status) = &app.status {
        let style = if status.starts_with("Couldn't") {
            Style::new().fg(THEME.error)
        } else {
            THEME.muted()
        };
        frame.render_widget(Paragraph::new(format!(" {status}")).style(style), area);
        return;
    }
    let keys: &[(&str, &str)] = match app.focus {
        Focus::Search => &[
            ("enter", "search"),
            ("ctrl-u", "clear"),
            ("esc", "back"),
            ("ctrl-c", "quit"),
        ],
        Focus::Library => &[
            ("enter", "open"),
            ("/", "search"),
            ("space", "pause"),
            ("n/p", "next/prev"),
            ("f", "like playing"),
            ("ctrl-r", "refresh"),
            ("tab", "tracks"),
            ("q", "quit"),
        ],
        Focus::Tracks => &[
            ("enter", "play"),
            ("a/N", "queue/next"),
            ("P", "playlist"),
            ("/", "search"),
            ("space", "pause"),
            ("n/p", "next/prev"),
            ("s", "shuffle"),
            ("r", "repeat"),
            ("f", "like"),
            ("i/o", "artist/album"),
            ("R", "radio"),
            ("L", "lyrics"),
            ("M", "minimal"),
            ("V", "visualizer"),
            ("A", "art"),
            ("q", "quit"),
        ],
        Focus::Queue => &[
            ("enter", "play"),
            ("J/K", "move"),
            ("N", "next"),
            ("d", "remove"),
            ("X", "clear"),
            ("P", "playlist"),
            ("R", "radio"),
            ("space", "pause"),
            ("n/p", "next/prev"),
            ("L", "lyrics"),
            ("M", "minimal"),
            ("q", "quit"),
        ],
    };
    let spans = keys.iter().flat_map(|(key, action)| {
        [
            Span::styled(format!(" {key}"), THEME.accent()),
            Span::styled(format!(" {action} "), THEME.muted()),
        ]
    });
    frame.render_widget(Paragraph::new(Line::from_iter(spans)), area);
}

pub(crate) fn heart(liked: bool) -> Span<'static> {
    if liked {
        Span::styled("♥", Style::new().fg(THEME.error))
    } else {
        Span::raw(" ")
    }
}

pub(crate) fn format_time(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (hours, minutes, secs) = (total / 3600, total / 60 % 60, total % 60);
    if hours > 0 {
        format!("{hours}:{minutes:02}:{secs:02}")
    } else {
        format!("{minutes}:{secs:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_words_and_splits_long_ones() {
        assert_eq!(wrap("In the deepest ocean", 10), vec!["In the", "deepest", "ocean"]);
        assert_eq!(wrap("abcdefghij", 4), vec!["abcd", "efgh", "ij"]);
        assert_eq!(wrap("", 10), vec![""]);
        assert_eq!(wrap("short", 10), vec!["short"]);
    }

    #[test]
    fn lyrics_shrink_then_hide_the_side_panels() {
        assert_eq!(side_panel_widths(false, 150), (28, 42));
        assert_eq!(side_panel_widths(true, 150), (18, 24));
        assert_eq!(side_panel_widths(true, 80), (0, 0));
    }
}
