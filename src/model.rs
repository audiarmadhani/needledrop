use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Track {
    pub video_id: String,
    pub title: String,
    pub artists: String,
    pub album: Option<String>,
    pub duration: Option<Duration>,
    pub thumbnail: Option<String>,
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
