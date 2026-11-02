//! Direct stream URLs, resolved ahead of time for gapless playback.
//!
//! Given a watch URL, mpv runs yt-dlp to find the audio stream, which takes
//! about four seconds: silence between songs. Resolving the next song while
//! the current one plays, and queueing the direct URL in mpv's own playlist,
//! lets mpv flow from one song into the next with no gap.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use tokio::process::Command;

use crate::model::Track;

/// Google's stream URLs stop working after a few hours; stay well inside that.
const TTL: Duration = Duration::from_secs(60 * 60);

/// Resolved stream URLs by video id.
#[derive(Default)]
pub struct Streams {
    urls: HashMap<String, (String, Instant)>,
}

impl Streams {
    /// A still-fresh stream URL for the track, if one was resolved.
    pub fn get(&self, video_id: &str) -> Option<&str> {
        self.urls.get(video_id).filter(|(_, at)| at.elapsed() < TTL).map(|(url, _)| url.as_str())
    }

    pub fn insert(&mut self, video_id: String, url: String) {
        self.urls.retain(|_, (_, at)| at.elapsed() < TTL);
        self.urls.insert(video_id, (url, Instant::now()));
    }

    /// Drops a URL that turned out not to play, so the next try goes
    /// through mpv's own resolution.
    pub fn forget(&mut self, video_id: &str) {
        self.urls.remove(video_id);
    }
}

/// Asks yt-dlp for the track's audio stream URL, with the same format choice
/// mpv uses.
pub async fn resolve(track: &Track) -> Result<String> {
    let output = Command::new("yt-dlp")
        .args(["--format", "bestaudio/best", "--get-url", "--no-playlist", "--no-warnings", "--quiet"])
        .arg(track.url())
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true)
        .output()
        .await
        .context("could not run yt-dlp")?;
    if !output.status.success() {
        bail!("yt-dlp couldn't resolve {}", track.video_id);
    }
    parse_url(&String::from_utf8_lossy(&output.stdout)).with_context(|| format!("no stream URL for {}", track.video_id))
}

/// One URL per line; a format with separate audio and video would give two,
/// and the first is the one to play.
fn parse_url(stdout: &str) -> Option<String> {
    stdout.lines().map(str::trim).find(|line| line.starts_with("https://")).map(str::to_owned)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn takes_the_first_url_line() {
        assert_eq!(parse_url("https://a.googlevideo.com/x\nhttps://b/y\n").as_deref(), Some("https://a.googlevideo.com/x"));
        assert_eq!(parse_url("WARNING: something\nhttps://a/x\n").as_deref(), Some("https://a/x"));
        assert_eq!(parse_url("ERROR: unavailable\n"), None);
    }

    #[test]
    fn cache_returns_fresh_urls_and_forgets_on_request() {
        let mut streams = Streams::default();
        streams.insert("v".into(), "https://a/x".into());
        assert_eq!(streams.get("v"), Some("https://a/x"));
        streams.forget("v");
        assert_eq!(streams.get("v"), None);
        streams.urls.insert("old".into(), ("https://a/y".into(), Instant::now() - TTL));
        assert_eq!(streams.get("old"), None, "expired");
    }
}
