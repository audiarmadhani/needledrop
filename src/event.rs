use ratatui::crossterm::event::{self, KeyEvent, KeyEventKind};
use tokio::sync::mpsc::UnboundedSender;

use crate::api::Account;
use crate::lyrics::Lyrics;
use crate::media;
use crate::palette::Palette;
use crate::model::{Playlist, Track};
use crate::player::PlayerEvent;

/// Everything the app reacts to, funneled through one channel.
pub enum Event {
    Key(KeyEvent),
    /// The terminal went away (closed window, killed session).
    InputClosed,
    /// A media key, headphone button, or the OS Now Playing widget.
    Media(media::Command),
    /// Animation frame.
    Tick,
    Resize,
    Player(PlayerEvent),
    /// The signed-in account and its library, loaded at startup.
    Library { account: Option<Account>, result: Result<Vec<Playlist>, String> },
    /// Tracks for the track list (search results or a playlist). `request`
    /// identifies which load this answers, so stale responses are dropped.
    Tracks { request: u64, result: Result<Vec<Track>, String> },
    /// A radio for the queue. `request` identifies which radio load this
    /// answers; `replace` says whether it replaces the queue or extends it.
    Radio { request: u64, replace: bool, result: Result<Vec<Track>, String> },
    /// Lyrics for the track with this video id (`None`: it has none).
    Lyrics { video_id: String, result: Result<Option<Lyrics>, String> },
    /// The terminal theme changed (or was read for the first time).
    Palette(Option<Palette>),
    /// Video ids in the user's Liked Music.
    Liked(Vec<String>),
    /// A like or unlike didn't go through; undo it locally.
    LikeFailed { video_id: String, liked: bool, error: String },
    /// A track's direct stream URL, resolved ahead of time.
    Resolved { video_id: String, result: Result<String, String> },
    /// Album art for the track with this video id.
    Art { video_id: String, image: image::DynamicImage },
}

/// Reads terminal input on a dedicated thread, since crossterm's reader blocks.
pub fn spawn_input_reader(events: UnboundedSender<Event>) {
    std::thread::spawn(move || {
        while let Ok(input) = event::read() {
            let event = match input {
                event::Event::Key(key) if key.kind == KeyEventKind::Press => Event::Key(key),
                event::Event::Resize(..) => Event::Resize,
                _ => continue,
            };
            if events.send(event).is_err() {
                return;
            }
        }
        let _ = events.send(Event::InputClosed);
    });
}
