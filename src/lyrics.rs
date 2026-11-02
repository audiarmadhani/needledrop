//! Lyrics: time-synced from LRCLIB (a free, open lyrics database) when it
//! has the song, otherwise YouTube Music's own plain lyrics.

use std::sync::LazyLock;

use anyhow::Result;
use serde_json::Value;

use crate::api;
use crate::model::Track;

#[derive(Debug, Clone, PartialEq)]
pub struct Lyrics {
    pub lines: Vec<LyricLine>,
    /// Whether every line has a timestamp.
    pub synced: bool,
    /// Credit to show, e.g. "LRCLIB" or "Source: LyricFind".
    pub source: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LyricLine {
    /// Seconds into the song; `None` for plain lyrics.
    pub at: Option<f64>,
    pub text: String,
}

impl Lyrics {
    /// Index of the line being sung at `position` seconds, for synced lyrics.
    pub fn current_line(&self, position: f64) -> Option<usize> {
        if !self.synced {
            return None;
        }
        self.lines.iter().rposition(|line| line.at.is_some_and(|at| at <= position))
    }
}

static HTTP: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .user_agent(concat!("needledrop/", env!("CARGO_PKG_VERSION"), " (https://github.com/audiarmadhani/needledrop)"))
        .build()
        .expect("static client config is valid")
});

/// Synced lyrics if LRCLIB has them, else YouTube's plain lyrics, else `None`.
pub async fn fetch(api: &api::Client, track: &Track) -> Result<Option<Lyrics>> {
    // One retry: a dropped connection shouldn't cost the synced lyrics.
    // "Not found" isn't an error, so it doesn't retry.
    for _ in 0..2 {
        match from_lrclib(track).await {
            Ok(Some(lyrics)) => return Ok(Some(lyrics)),
            Ok(None) => break,
            Err(_) => continue,
        }
    }
    let Some(lyrics_id) = api.watch_next(&track.video_id).await?.lyrics_id else { return Ok(None) };
    Ok(api.lyrics(&lyrics_id).await?.map(|(text, source)| Lyrics {
        lines: text.lines().map(|line| LyricLine { at: None, text: line.to_owned() }).collect(),
        synced: false,
        source,
    }))
}

async fn from_lrclib(track: &Track) -> Result<Option<Lyrics>> {
    // LRCLIB matches on duration (±2 s), so it can't look a song up without one.
    let Some(duration) = track.duration else { return Ok(None) };
    let duration = duration.as_secs().to_string();
    let mut query = vec![
        ("track_name", track.title.as_str()),
        ("artist_name", track.artists.as_str()),
        ("duration", duration.as_str()),
    ];
    if let Some(album) = &track.album {
        query.push(("album_name", album));
    }
    let response = HTTP.get("https://lrclib.net/api/get").query(&query).send().await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let body: Value = response.error_for_status()?.json().await?;
    let lines = body["syncedLyrics"].as_str().map(parse_lrc).unwrap_or_default();
    Ok((!lines.is_empty()).then(|| Lyrics { lines, synced: true, source: "LRCLIB".to_owned() }))
}

/// Parses LRC lines like `[01:04.07]The bottom of the sea`. A line may carry
/// several timestamps (`[00:10.00][01:20.00]Chorus`); lines without one are skipped.
fn parse_lrc(text: &str) -> Vec<LyricLine> {
    let mut lines = Vec::new();
    for raw in text.lines() {
        let mut rest = raw.trim();
        let mut stamps = Vec::new();
        while let Some(stripped) = rest.strip_prefix('[') {
            let Some((stamp, after)) = stripped.split_once(']') else { break };
            let Some(seconds) = parse_timestamp(stamp) else { break };
            stamps.push(seconds);
            rest = after;
        }
        lines.extend(stamps.into_iter().map(|at| LyricLine { at: Some(at), text: rest.trim().to_owned() }));
    }
    lines.sort_by(|a, b| a.at.partial_cmp(&b.at).unwrap_or(std::cmp::Ordering::Equal));
    lines
}

fn parse_timestamp(stamp: &str) -> Option<f64> {
    let (minutes, seconds) = stamp.split_once(':')?;
    Some(minutes.parse::<f64>().ok()? * 60.0 + seconds.parse::<f64>().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lrc_including_repeated_stamps_and_metadata() {
        let lines = parse_lrc("[ar:Radiohead]\n[00:58.19]In the deepest ocean\n[00:10.00][01:20.50]Chorus\n\n[01:04.07]\n");
        let got: Vec<_> = lines.iter().map(|l| (l.at.unwrap(), l.text.as_str())).collect();
        assert_eq!(got, vec![(10.0, "Chorus"), (58.19, "In the deepest ocean"), (64.07, ""), (80.5, "Chorus")]);
    }

    #[test]
    fn finds_the_line_being_sung() {
        let lyrics = Lyrics { lines: parse_lrc("[00:05.00]one\n[00:10.00]two"), synced: true, source: String::new() };
        assert_eq!(lyrics.current_line(2.0), None);
        assert_eq!(lyrics.current_line(5.0), Some(0));
        assert_eq!(lyrics.current_line(30.0), Some(1));
        let plain = Lyrics { synced: false, ..lyrics };
        assert_eq!(plain.current_line(30.0), None);
    }
}
