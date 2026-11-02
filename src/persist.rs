//! What survives a restart: preferences in the config folder, and the
//! listening session (queue, current song, position) in the state folder.
//!
//! - `$XDG_CONFIG_HOME/needledrop/settings.json` (default `~/.config/…`)
//! - `$XDG_STATE_HOME/needledrop/session.json` (default `~/.local/state/…`)
//!
//! A missing or unreadable file means defaults: a corrupt file must never
//! stop needledrop from starting.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::app::Repeat;
use crate::model::Track;
use crate::retro::ArtStyle;
use crate::visualizer;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub art_style: ArtStyle,
    pub visualizer: visualizer::Style,
    pub volume: f64,
    pub shuffle: bool,
    pub repeat: Repeat,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            art_style: ArtStyle::default(),
            visualizer: visualizer::Style::default(),
            volume: 100.0,
            shuffle: false,
            repeat: Repeat::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Session {
    pub queue: Vec<Track>,
    /// Index into `queue` of the song that was playing.
    pub current: Option<usize>,
    /// Seconds into that song.
    pub position: f64,
}

impl Settings {
    pub fn load() -> Self {
        settings_path().and_then(|path| read(&path)).unwrap_or_default()
    }

    pub fn save(&self) -> Result<()> {
        write(&settings_path().context("HOME is not set")?, self)
    }
}

impl Session {
    pub fn load() -> Option<Self> {
        read::<Self>(&session_path()?)?.usable()
    }

    /// Only a non-empty queue whose current index points into it.
    fn usable(self) -> Option<Self> {
        let valid = self.current.is_none_or(|i| i < self.queue.len());
        (valid && !self.queue.is_empty()).then_some(self)
    }

    pub fn save(&self) -> Result<()> {
        write(&session_path().context("HOME is not set")?, self)
    }
}

/// Appends a line to `$XDG_STATE_HOME/needledrop/errors.log`, for problems
/// that shouldn't interrupt the UI (a background task panicking, say).
pub fn log_error(message: &str) {
    use std::io::Write;
    let Some(path) = xdg_dir("XDG_STATE_HOME", ".local/state").map(|d| d.join("needledrop/errors.log")) else {
        return;
    };
    if let Some(dir) = path.parent() {
        let _ = fs::create_dir_all(dir);
    }
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if let Ok(mut file) = fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "[{seconds}] {message}");
    }
}

fn settings_path() -> Option<PathBuf> {
    Some(xdg_dir("XDG_CONFIG_HOME", ".config")?.join("needledrop/settings.json"))
}

fn session_path() -> Option<PathBuf> {
    Some(xdg_dir("XDG_STATE_HOME", ".local/state")?.join("needledrop/session.json"))
}

/// `$var` if set, else `~/<fallback>`.
fn xdg_dir(var: &str, fallback: &str) -> Option<PathBuf> {
    match std::env::var_os(var) {
        Some(dir) if !dir.is_empty() => Some(PathBuf::from(dir)),
        _ => Some(PathBuf::from(std::env::var_os("HOME")?).join(fallback)),
    }
}

fn read<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// Writes to a temporary file and renames it into place, so a crash
/// mid-write leaves the previous version intact.
fn write<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).with_context(|| format!("could not create {}", dir.display()))?;
    }
    let staged = path.with_extension("json.tmp");
    fs::write(&staged, serde_json::to_string_pretty(value)?)?;
    fs::rename(&staged, path).with_context(|| format!("could not write {}", path.display()))
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("needledrop-persist-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn settings_round_trip_and_read_as_kebab_case() {
        let dir = temp_dir("settings");
        let path = dir.join("nested/settings.json");
        let settings = Settings {
            art_style: ArtStyle::GameBoy,
            visualizer: visualizer::Style::Scope,
            volume: 65.0,
            shuffle: true,
            repeat: Repeat::One,
        };
        write(&path, &settings).unwrap();
        assert_eq!(read::<Settings>(&path), Some(settings));
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"game-boy\"") && text.contains("\"scope\""), "{text}");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_fields_take_defaults_and_corrupt_files_are_ignored() {
        let dir = temp_dir("lenient");
        let path = dir.join("settings.json");
        fs::write(&path, r#"{ "volume": 40 }"#).unwrap();
        let settings: Settings = read(&path).unwrap();
        assert_eq!(settings.volume, 40.0);
        assert_eq!(settings.art_style, ArtStyle::Terminal);
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(read::<Settings>(&path), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_sessions_that_point_outside_the_queue() {
        let track = Track {
            video_id: "v".into(),
            title: "T".into(),
            artists: "A".into(),
            album: None,
            duration: None,
            thumbnail: None,
            artist_id: None,
            album_id: None,
            set_video_id: None,
        };
        let session = |queue: Vec<Track>, current| Session {
            queue,
            current,
            position: 0.0,
        };
        assert!(session(vec![track.clone()], Some(0)).usable().is_some());
        assert!(session(vec![track.clone()], None).usable().is_some());
        assert!(session(vec![track], Some(1)).usable().is_none());
        assert!(session(vec![], None).usable().is_none());
    }

    #[test]
    fn session_round_trips_with_tracks() {
        let dir = temp_dir("session");
        let path = dir.join("session.json");
        let session = Session {
            queue: vec![Track {
                video_id: "v".into(),
                title: "Song".into(),
                artists: "Artist".into(),
                album: None,
                duration: Some(Duration::from_secs(200)),
                thumbnail: Some("https://t".into()),
                artist_id: None,
                album_id: None,
                set_video_id: None,
            }],
            current: Some(0),
            position: 61.5,
        };
        write(&path, &session).unwrap();
        assert_eq!(read::<Session>(&path), Some(session));
        fs::remove_dir_all(&dir).unwrap();
    }
}
