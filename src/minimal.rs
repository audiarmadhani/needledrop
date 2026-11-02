//! Minimal mode (`M`): a full-screen, borderless now-playing view in the
//! spirit of Spotify's Car Thing. A big cover, the title, the line being
//! sung, the visualizer, and the progress bar and controls along the bottom.
//!
//! Wide panes put the cover on the left and everything else beside it; tall
//! panes stack the cover on top.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Layout, Margin, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::app::{App, Repeat};
use crate::art::PixelArt;
use crate::ui::{THEME, current_lyrics, format_time, lyric_text, render_art_placeholder, visual, wrap};

/// Rows for the sung line and the one after it.
const LYRIC_ROWS: u16 = 3;
/// Columns between the cover and the text beside it.
const GUTTER: u16 = 5;
/// The narrowest text column worth putting beside the cover.
const MIN_TEXT_WIDTH: u16 = 30;
/// Beyond this height the visualizer looks heavy; extra room becomes space.
const MAX_VISUALIZER_ROWS: u16 = 12;

pub fn render(app: &mut App, frame: &mut Frame) {
    let [body, hint] = Layout::vertical([Constraint::Fill(1), Constraint::Length(1)]).areas(frame.area());
    render_hint(app, frame, hint);
    let margin = if body.width >= 60 { Margin::new(4, 1) } else { Margin::new(1, 0) };
    let body = body.inner(margin);

    if app.now_playing().is_none() {
        return render_idle(frame, body);
    }
    let [top, _, progress, _, controls] = Layout::vertical([
        Constraint::Fill(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(body);

    // The cover is a square `rows` tall and 2 × `rows` wide; landscape when a
    // readable column of text still fits beside it.
    if top.width >= top.height * 2 + GUTTER + MIN_TEXT_WIDTH {
        render_landscape(app, frame, top);
    } else {
        render_portrait(app, frame, top);
    }
    frame.render_widget(Paragraph::new(progress_line(app, progress.width)), progress);
    frame.render_widget(Paragraph::new(controls_line(app)).alignment(Alignment::Center), controls);
}

fn render_landscape(app: &mut App, frame: &mut Frame, area: Rect) {
    let art_width = (area.height * 2).min(area.width / 2);
    let [art, _, text] =
        Layout::horizontal([Constraint::Length(art_width), Constraint::Length(GUTTER), Constraint::Fill(1)]).areas(area);
    render_art(app, frame, art);

    // Text hangs from the top of the cover; the visualizer sits on its bottom edge.
    let lyric_rows = if has_synced_lyrics(app) { LYRIC_ROWS } else { 0 };
    let [title, byline, _, lyrics, _, rest] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(lyric_rows),
        Constraint::Length(if lyric_rows > 0 { 1 } else { 0 }),
        Constraint::Fill(1),
    ])
    .areas(text);
    let [_, visualizer] =
        Layout::vertical([Constraint::Fill(1), Constraint::Max(MAX_VISUALIZER_ROWS)]).areas(rest);
    render_titles(app, frame, title, byline, Alignment::Left);
    render_lyric(app, frame, lyrics, Alignment::Left);
    frame.render_widget(visual(app), visualizer);
}

fn render_portrait(app: &mut App, frame: &mut Frame, area: Rect) {
    let lyric_rows = if has_synced_lyrics(app) { LYRIC_ROWS } else { 0 };
    // The cover takes what's left after the text, up to the pane's width.
    let text_rows = 2 + 1 + lyric_rows + 1;
    let art_rows = (area.height.saturating_sub(text_rows + 4)).min(area.width / 2).max(4);
    let [art, _, title, byline, _, lyrics, _, visualizer] = Layout::vertical([
        Constraint::Length(art_rows),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Length(lyric_rows),
        Constraint::Length(if lyric_rows > 0 { 1 } else { 0 }),
        Constraint::Fill(1),
    ])
    .areas(area);
    render_art(app, frame, art);
    render_titles(app, frame, title, byline, Alignment::Center);
    render_lyric(app, frame, lyrics, Alignment::Center);
    let [visualizer, _] =
        Layout::vertical([Constraint::Max(MAX_VISUALIZER_ROWS), Constraint::Fill(1)]).areas(visualizer);
    frame.render_widget(visual(app), visualizer);
}

fn render_art(app: &mut App, frame: &mut Frame, area: Rect) {
    match &mut app.art {
        Some(art) => frame.render_stateful_widget(PixelArt { style: app.art_style, palette: app.palette }, area, art),
        None => {
            // Keep the placeholder square, like the cover will be.
            let side = area.width.min(area.height * 2);
            let square = Rect { width: side, height: side / 2, ..area };
            render_art_placeholder(frame, square);
        }
    }
}

fn render_titles(app: &App, frame: &mut Frame, title: Rect, byline: Rect, alignment: Alignment) {
    let Some(track) = app.now_playing() else { return };
    let title_line = Line::styled(track.title.as_str(), THEME.text().add_modifier(Modifier::BOLD));
    frame.render_widget(Paragraph::new(title_line).alignment(alignment), title);
    let mut by = track.artists.clone();
    if let Some(album) = &track.album {
        by.push_str(&format!(" · {album}"));
    }
    frame.render_widget(Paragraph::new(Line::styled(by, THEME.muted())).alignment(alignment), byline);
}

fn has_synced_lyrics(app: &App) -> bool {
    current_lyrics(app).is_some_and(|lyrics| lyrics.synced)
}

/// The line being sung, highlighted, then the next one dimmed, wrapped to fit.
fn render_lyric(app: &App, frame: &mut Frame, area: Rect, alignment: Alignment) {
    let Some(lyrics) = current_lyrics(app).filter(|l| l.synced) else { return };
    if area.is_empty() {
        return;
    }
    let current = lyrics.current_line(app.playback.current_position());
    let (now, next) = match current {
        Some(i) => (lyrics.lines.get(i), lyrics.lines.get(i + 1)),
        // Before the first line: show it as what's coming.
        None => (None, lyrics.lines.first()),
    };
    let width = area.width as usize;
    let mut rows: Vec<Line> = vec![];
    if let Some(line) = now {
        let style = THEME.accent().add_modifier(Modifier::BOLD);
        rows.extend(wrap(lyric_text(&line.text), width).into_iter().map(|r| Line::styled(r, style)));
    }
    if let Some(line) = next {
        rows.extend(wrap(lyric_text(&line.text), width).into_iter().map(|r| Line::styled(r, THEME.muted())));
    }
    rows.truncate(area.height as usize);
    frame.render_widget(Paragraph::new(rows).alignment(alignment), area);
}

/// `1:16 ━━━━━━━━━━●────────── 5:18`
fn progress_line(app: &App, width: u16) -> Line<'static> {
    let playback = &app.playback;
    let position = playback.current_position().min(playback.duration.max(0.0));
    let (elapsed, total) = (format_time(position), format_time(playback.duration));
    let bar = (width as usize).saturating_sub(elapsed.len() + total.len() + 2);
    let ratio = if playback.duration > 0.0 { (position / playback.duration).clamp(0.0, 1.0) } else { 0.0 };
    let filled = ((ratio * bar as f64).round() as usize).min(bar.saturating_sub(1));
    Line::from(vec![
        Span::styled(format!("{elapsed} "), THEME.muted()),
        Span::styled("━".repeat(filled), THEME.accent()),
        Span::styled(if bar > 0 { "●" } else { "" }, THEME.accent()),
        Span::styled("─".repeat(bar.saturating_sub(filled + 1)), THEME.muted()),
        Span::styled(format!(" {total}"), THEME.muted()),
    ])
}

/// `⇄    ◀◀    ❚❚    ▶▶    ↻    ♥`, lit up where a mode is on.
fn controls_line(app: &App) -> Line<'static> {
    let lit = |on: bool| if on { THEME.accent() } else { THEME.muted() };
    let liked = app.now_playing().is_some_and(|t| app.liked.contains(&t.video_id));
    let gap = || Span::raw("     ");
    Line::from(vec![
        Span::styled("⇄", lit(app.shuffle)),
        gap(),
        Span::styled("◀◀", THEME.text()),
        gap(),
        Span::styled(if app.playback.paused { "▶" } else { "❚❚" }, THEME.accent().add_modifier(Modifier::BOLD)),
        gap(),
        Span::styled("▶▶", THEME.text()),
        gap(),
        Span::styled(if app.repeat == Repeat::One { "↻¹" } else { "↻" }, lit(app.repeat != Repeat::Off)),
        gap(),
        if liked { Span::styled("♥", Style::new().fg(THEME.error)) } else { Span::styled("♡", THEME.muted()) },
    ])
}

fn render_idle(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(vec![Span::styled("◉ ", THEME.accent()), Span::styled("needledrop", THEME.title(true))]),
        Line::raw(""),
        Line::styled("Nothing playing", THEME.muted()),
    ];
    let [_, middle, _] =
        Layout::vertical([Constraint::Fill(1), Constraint::Length(lines.len() as u16), Constraint::Fill(1)]).areas(area);
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), middle);
}

/// Bottom-right key hints, or the status message when there is one.
fn render_hint(app: &App, frame: &mut Frame, area: Rect) {
    let line = match &app.status {
        Some(status) => Line::styled(format!("{status} "), THEME.muted()),
        None => Line::from(
            [("M", "back"), ("V", "visualizer"), ("A", "art"), ("space", "pause"), ("n/p", "skip")]
                .iter()
                .flat_map(|(key, action)| [Span::styled(*key, THEME.accent()), Span::styled(format!(" {action}   "), THEME.muted())])
                .collect::<Vec<_>>(),
        ),
    };
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Right), area);
}
