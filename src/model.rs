use std::time::Duration;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Track {
    pub video_id: String,
    pub title: String,
    pub artists: String,
    pub album: Option<String>,
    pub duration: Option<Duration>,
    pub thumbnail: Option<String>,
    /// Browse id of the (first) artist's page, `UC…`.
    #[serde(default)]
    pub artist_id: Option<String>,
    /// Browse id of the album's page, `MPREb_…`.
    #[serde(default)]
    pub album_id: Option<String>,
    /// This entry's id within the playlist it was loaded from, needed to
    /// remove it (a playlist can hold the same song more than once).
    #[serde(default)]
    pub set_video_id: Option<String>,
}

impl Track {
    pub fn url(&self) -> String {
        format!("https://music.youtube.com/watch?v={}", self.video_id)
    }
}

/// A playlist in the user's library. `browse_id` is what InnerTube's
/// `browse` endpoint takes, e.g. `VLLM` for Liked Music.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Playlist {
    pub browse_id: String,
    pub title: String,
    pub subtitle: String,
}

/// Something on a Home, artist, or search page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Item {
    Song(Track),
    Album {
        browse_id: String,
        title: String,
        subtitle: String,
        thumbnail: Option<String>,
    },
    Artist {
        browse_id: String,
        name: String,
        subtitle: String,
        thumbnail: Option<String>,
    },
    Playlist {
        browse_id: String,
        title: String,
        subtitle: String,
        thumbnail: Option<String>,
    },
}

impl Item {
    pub fn title(&self) -> &str {
        match self {
            Item::Song(track) => &track.title,
            Item::Album { title, .. } | Item::Playlist { title, .. } => title,
            Item::Artist { name, .. } => name,
        }
    }

    /// Cover art URL (a portrait for artists).
    pub fn thumbnail(&self) -> Option<&str> {
        match self {
            Item::Song(track) => track.thumbnail.as_deref(),
            Item::Album { thumbnail, .. } | Item::Artist { thumbnail, .. } | Item::Playlist { thumbnail, .. } => {
                thumbnail.as_deref()
            }
        }
    }

    pub fn subtitle(&self) -> String {
        match self {
            Item::Song(track) => track.artists.clone(),
            Item::Album { subtitle, .. } | Item::Artist { subtitle, .. } | Item::Playlist { subtitle, .. } => {
                subtitle.clone()
            }
        }
    }
}

/// A titled row of items, like "Quick picks" or "Albums".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shelf {
    pub title: String,
    pub items: Vec<Item>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Album {
    pub title: String,
    /// e.g. "Olivia Dean · Album • 2025 · 12 songs • 34 minutes"
    pub subtitle: String,
    pub tracks: Vec<Track>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Artist {
    pub name: String,
    pub shelves: Vec<Shelf>,
}
