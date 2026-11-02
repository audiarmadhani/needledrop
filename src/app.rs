use std::collections::HashSet;
use std::future::Future;
use std::time::Instant;

use anyhow::{Result, bail};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::widgets::{ListState, TableState};
use tokio::sync::mpsc::UnboundedSender;

use crate::api::{self, Account};
use crate::art::{self, AlbumArt};
use crate::event::Event;
use crate::lyrics::{self, Lyrics};
use crate::media::{self, Command};
use crate::model::{Playlist, Track};
use crate::palette::Palette;
use crate::player::{Player, PlayerEvent};
use crate::retro::ArtStyle;
use crate::streams::{self, Streams};
use crate::visualizer::Visualizer;

const SEEK_STEP: f64 = 5.0;
const STATUS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
/// Browse id of the Liked Music auto-playlist.
const LIKED_MUSIC: &str = "VLLM";
const VOLUME_STEP: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
    Search,
    Library,
    Tracks,
    Queue,
}

impl Focus {
    fn next(self) -> Self {
        match self {
            Focus::Library => Focus::Tracks,
            Focus::Tracks => Focus::Queue,
            Focus::Queue | Focus::Search => Focus::Library,
        }
    }

    fn previous(self) -> Self {
        match self {
            Focus::Library | Focus::Search => Focus::Queue,
            Focus::Tracks => Focus::Library,
            Focus::Queue => Focus::Tracks,
        }
    }
}

pub enum Library {
    SignedOut,
    Loading,
    Loaded(Vec<Playlist>),
    Failed(String),
}

impl Library {
    pub fn playlists(&self) -> &[Playlist] {
        match self {
            Library::Loaded(playlists) => playlists,
            _ => &[],
        }
    }
}

/// The middle pane: search results or a playlist's tracks.
pub struct TrackList {
    pub title: String,
    pub tracks: Vec<Track>,
    pub state: TableState,
    pub loading: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    Off,
    /// Start the queue over after the last track.
    All,
    /// Replay the current track when it ends.
    One,
}

impl Repeat {
    fn cycle(self) -> Self {
        match self {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        }
    }
}

/// The queue entry handed to mpv to play after the current one.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Preloaded {
    index: usize,
    video_id: String,
}

/// Why playback is moving to another track.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Advance {
    /// The track played to the end (honors repeat-one).
    Ended,
    /// The user asked for the next track.
    Skip,
}

pub enum LyricsState {
    Loading,
    Found(Lyrics),
    Missing,
    Failed(String),
}

#[derive(Debug)]
pub struct Playback {
    pub position: f64,
    /// When mpv last reported `position`, to interpolate between reports.
    pub position_at: Instant,
    pub duration: f64,
    pub paused: bool,
    pub volume: f64,
}

impl Playback {
    /// The position right now, extrapolated from mpv's last report.
    pub fn current_position(&self) -> f64 {
        if self.paused {
            self.position
        } else {
            self.position + self.position_at.elapsed().as_secs_f64()
        }
    }
}

pub struct App {
    pub focus: Focus,
    pub query: String,
    pub account: Option<Account>,
    pub library: Library,
    pub library_state: ListState,
    /// Video ids in Liked Music, to show hearts and toggle likes.
    pub liked: HashSet<String>,
    pub list: TrackList,
    pub queue: Vec<Track>,
    pub queue_state: ListState,
    /// Index into `queue` of the track loaded in mpv.
    pub current: Option<usize>,
    pub playback: Playback,
    pub shuffle: bool,
    pub repeat: Repeat,
    /// Whether the middle pane shows lyrics instead of the track list.
    pub show_lyrics: bool,
    /// Whether the full-screen minimal now-playing view is up.
    pub minimal: bool,
    /// Lyrics and the video id they belong to.
    pub lyrics: Option<(String, LyricsState)>,
    /// Scroll offset for plain (unsynced) lyrics.
    pub lyrics_scroll: u16,
    pub visualizer: Visualizer,
    /// Album art for the current track.
    pub art: Option<AlbumArt>,
    pub art_style: ArtStyle,
    /// The terminal theme's colors, when they can be read.
    pub palette: Option<Palette>,
    /// A message for the footer; cleared `STATUS_TIMEOUT` after it changes.
    pub status: Option<String>,
    /// The status last seen by a tick, and since when.
    status_seen: Option<(String, Instant)>,
    pub should_quit: bool,
    /// Id of the latest track-list load; older responses are ignored.
    request: u64,
    /// Direct stream URLs resolved ahead of time, for gapless playback.
    streams: Streams,
    /// Video ids being resolved right now.
    resolving: HashSet<String>,
    /// The queue entry mpv has queued to play next, if any.
    preloaded: Option<Preloaded>,
    /// Id of the latest radio load. Bumped whenever the queue is replaced,
    /// so a radio meant to extend the old queue is ignored.
    radio_request: u64,
    /// Whether the latest radio request is still in flight.
    radio_loading: bool,
    /// Set when the queue ran out while a radio was still loading: play from
    /// this index once it arrives.
    resume_at: Option<usize>,
    api: api::Client,
    player: Player,
    media: media::Handle,
    events: UnboundedSender<Event>,
}

impl App {
    pub fn new(api: api::Client, player: Player, media: media::Handle, events: UnboundedSender<Event>) -> Self {
        let signed_in = api.is_signed_in();
        let mut app = Self {
            focus: if signed_in { Focus::Library } else { Focus::Search },
            query: String::new(),
            account: None,
            library: if signed_in {
                Library::Loading
            } else {
                Library::SignedOut
            },
            library_state: ListState::default(),
            liked: HashSet::new(),
            list: TrackList {
                title: "tracks".into(),
                tracks: Vec::new(),
                state: TableState::default(),
                loading: false,
            },
            queue: Vec::new(),
            queue_state: ListState::default(),
            current: None,
            playback: Playback {
                position: 0.0,
                position_at: Instant::now(),
                duration: 0.0,
                paused: false,
                volume: 100.0,
            },
            shuffle: false,
            repeat: Repeat::Off,
            show_lyrics: false,
            minimal: false,
            lyrics: None,
            lyrics_scroll: 0,
            visualizer: Visualizer::new(),
            art: None,
            art_style: ArtStyle::Terminal,
            palette: None,
            status: None,
            status_seen: None,
            should_quit: false,
            request: 0,
            streams: Streams::default(),
            resolving: HashSet::new(),
            preloaded: None,
            radio_request: 0,
            radio_loading: false,
            resume_at: None,
            api,
            player,
            media,
            events,
        };
        if signed_in {
            app.load_library();
        }
        app
    }

    pub fn now_playing(&self) -> Option<&Track> {
        self.current.and_then(|i| self.queue.get(i))
    }

    /// Whether the screen changes on its own (the visualizer is moving, or
    /// a status message is waiting to expire).
    pub fn is_animating(&self) -> bool {
        (self.current.is_some() && !self.playback.paused) || self.visualizer.is_animating() || self.status.is_some()
    }

    fn expire_status(&mut self) {
        match (&self.status, &self.status_seen) {
            (None, _) => self.status_seen = None,
            (Some(status), Some((seen, since))) if status == seen => {
                if since.elapsed() >= STATUS_TIMEOUT {
                    self.status = None;
                }
            }
            (Some(status), _) => self.status_seen = Some((status.clone(), Instant::now())),
        }
    }

    pub async fn handle(&mut self, event: Event) -> Result<()> {
        self.handle_event(event).await?;
        // Whatever just happened (a skip, a shuffle, a removal, a radio
        // landing), make sure mpv has the right song queued next.
        self.refresh_preload().await
    }

    async fn handle_event(&mut self, event: Event) -> Result<()> {
        match event {
            Event::Key(key) => self.handle_key(key).await?,
            Event::Resize => {}
            Event::InputClosed => self.should_quit = true,
            Event::Media(command) => self.handle_media(command).await?,
            Event::Tick => {
                let playing = self.current.is_some() && !self.playback.paused;
                self.visualizer.tick(playing.then(|| self.playback.current_position()));
                self.expire_status();
            }
            Event::Radio {
                request,
                replace,
                result,
            } => self.handle_radio(request, replace, result).await?,
            Event::Lyrics { video_id, result } => {
                if self.lyrics.as_ref().is_some_and(|(id, _)| *id == video_id) {
                    let state = match result {
                        Ok(Some(lyrics)) => LyricsState::Found(lyrics),
                        Ok(None) => LyricsState::Missing,
                        Err(error) => LyricsState::Failed(error),
                    };
                    self.lyrics = Some((video_id, state));
                }
            }
            Event::Liked(ids) => self.liked = ids.into_iter().collect(),
            Event::Palette(palette) => self.palette = palette,
            Event::LikeFailed { video_id, liked, error } => {
                if liked {
                    self.liked.remove(&video_id);
                } else {
                    self.liked.insert(video_id);
                }
                self.status = Some(format!("Couldn't update your likes: {error}"));
            }
            Event::Resolved { video_id, result } => {
                self.resolving.remove(&video_id);
                if let Ok(url) = result {
                    self.streams.insert(video_id, url);
                }
            }
            Event::Art { video_id, image } => {
                if self.now_playing().is_some_and(|t| t.video_id == video_id) {
                    self.art = Some(AlbumArt::new(image));
                }
            }
            Event::Player(event) => self.handle_player(event).await?,
            Event::Library { account, result } => {
                self.account = account;
                self.library = match result {
                    Ok(playlists) => {
                        self.library_state.select((!playlists.is_empty()).then_some(0));
                        Library::Loaded(playlists)
                    }
                    Err(error) => Library::Failed(error),
                };
            }
            Event::Tracks { request, result } => {
                if request != self.request {
                    return Ok(());
                }
                self.list.loading = false;
                match result {
                    Ok(tracks) => {
                        self.status = tracks.is_empty().then(|| format!("No tracks in {}", self.list.title));
                        self.list.state.select((!tracks.is_empty()).then_some(0));
                        self.list.tracks = tracks;
                    }
                    Err(error) => self.status = Some(format!("Couldn't load {}: {error}", self.list.title)),
                }
            }
        }
        Ok(())
    }

    async fn handle_player(&mut self, event: PlayerEvent) -> Result<()> {
        match event {
            PlayerEvent::Position(seconds) => {
                let jumped = (seconds - self.playback.current_position()).abs() > 1.5;
                self.playback.position = seconds;
                self.playback.position_at = Instant::now();
                if jumped {
                    self.report_playback();
                }
            }
            PlayerEvent::Stream(url) => self.visualizer.start(url),
            PlayerEvent::Duration(seconds) => self.playback.duration = seconds,
            PlayerEvent::Paused(paused) => {
                // Fold the time played since the last report in before freezing.
                self.playback.position = self.playback.current_position();
                self.playback.position_at = Instant::now();
                self.playback.paused = paused;
                self.report_playback();
            }
            PlayerEvent::Volume(volume) => self.playback.volume = volume,
            PlayerEvent::TrackEnded => match self.preloaded.take() {
                // mpv has already moved on to the queued song: just follow it.
                Some(next) if self.queue.get(next.index).is_some_and(|t| t.video_id == next.video_id) => {
                    self.enter_track(next.index);
                }
                _ => self.advance(Advance::Ended).await?,
            },
            PlayerEvent::TrackFailed => {
                let Some(track) = self.now_playing().cloned() else {
                    return Ok(());
                };
                if self.streams.get(&track.video_id).is_some() {
                    // A pre-resolved URL can expire (after a long pause, say).
                    // Retry once through mpv's own resolution.
                    self.streams.forget(&track.video_id);
                    return self.play(self.current.expect("a track is playing")).await;
                }
                self.status = Some(format!("Couldn't play “{}”, skipping", track.title));
                self.advance(Advance::Skip).await?;
            }
            PlayerEvent::Exited => bail!("mpv exited unexpectedly"),
        }
        Ok(())
    }

    async fn handle_key(&mut self, key: KeyEvent) -> Result<()> {
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.should_quit = true;
            return Ok(());
        }
        if self.minimal {
            if matches!(key.code, KeyCode::Char('M') | KeyCode::Esc) {
                self.minimal = false;
                return Ok(());
            }
            // Only playback keys: the browsing panes aren't on screen.
            let playback = matches!(
                key.code,
                KeyCode::Char('q' | ' ' | 'n' | 'p' | 'l' | 'h' | '+' | '=' | '-' | 's' | 'r' | 'f' | 'V' | 'A')
                    | KeyCode::Left
                    | KeyCode::Right
            );
            if !playback {
                return Ok(());
            }
        }
        if self.focus == Focus::Search {
            match key.code {
                KeyCode::Enter => {
                    self.submit_search();
                    self.focus = Focus::Tracks;
                }
                KeyCode::Esc | KeyCode::Tab => self.focus = Focus::Tracks,
                KeyCode::Backspace => {
                    self.query.pop();
                }
                KeyCode::Char(c) => self.query.push(c),
                _ => {}
            }
            return Ok(());
        }

        match key.code {
            KeyCode::Char('q') => self.should_quit = true,
            KeyCode::Char('/') => self.focus = Focus::Search,
            KeyCode::Tab => self.focus = self.focus.next(),
            KeyCode::BackTab => self.focus = self.focus.previous(),
            KeyCode::Char('j') | KeyCode::Down if self.lyrics_have_focus() => {
                self.lyrics_scroll = self.lyrics_scroll.saturating_add(1)
            }
            KeyCode::Char('k') | KeyCode::Up if self.lyrics_have_focus() => {
                self.lyrics_scroll = self.lyrics_scroll.saturating_sub(1)
            }
            KeyCode::Char('j') | KeyCode::Down => self.move_selection(1),
            KeyCode::Char('k') | KeyCode::Up => self.move_selection(-1),
            KeyCode::Char('r') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                if self.api.is_signed_in() {
                    self.status = Some("Refreshing your library…".into());
                    self.load_library();
                }
            }
            KeyCode::Char('f') => self.toggle_like(),
            KeyCode::Char('A') => {
                self.art_style = self.art_style.next(self.palette.is_some());
                self.status = Some(format!("Album art: {}", self.art_style.name()));
            }
            KeyCode::Char('V') => {
                self.visualizer.style = self.visualizer.style.next();
                self.status = Some(format!("Visualizer: {}", self.visualizer.style.name()));
            }
            KeyCode::Char('M') => {
                self.minimal = true;
                self.load_lyrics();
            }
            KeyCode::Char('L') => {
                self.show_lyrics = !self.show_lyrics;
                self.lyrics_scroll = 0;
                self.load_lyrics();
            }
            KeyCode::Char('R') => {
                if let Some(seed) = self.target_track().cloned() {
                    self.status = Some(format!("Starting radio from “{}”…", seed.title));
                    self.start_radio(seed, true);
                }
            }
            KeyCode::Char('g') | KeyCode::Home => self.move_selection(isize::MIN),
            KeyCode::Char('G') | KeyCode::End => self.move_selection(isize::MAX),
            KeyCode::Enter if self.lyrics_have_focus() => {}
            KeyCode::Enter => self.activate_selected().await?,
            KeyCode::Char('a') if self.focus == Focus::Tracks => self.enqueue_selected().await?,
            KeyCode::Char('d') | KeyCode::Delete if self.focus == Focus::Queue => self.remove_selected().await?,
            KeyCode::Char(' ') if self.current.is_some() => self.player.toggle_pause().await?,
            KeyCode::Char('n') => self.advance(Advance::Skip).await?,
            KeyCode::Char('s') => self.set_shuffle(!self.shuffle),
            KeyCode::Char('r') => self.repeat = self.repeat.cycle(),
            KeyCode::Char('p') => self.previous().await?,
            KeyCode::Char('l') | KeyCode::Right => self.player.seek(SEEK_STEP).await?,
            KeyCode::Char('h') | KeyCode::Left => self.player.seek(-SEEK_STEP).await?,
            KeyCode::Char('+') | KeyCode::Char('=') => self.player.change_volume(VOLUME_STEP).await?,
            KeyCode::Char('-') => self.player.change_volume(-VOLUME_STEP).await?,
            _ => {}
        }
        Ok(())
    }

    fn load_library(&mut self) {
        let api = self.api.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let account = api.account().await.ok().flatten();
            let result = api.library_playlists().await.map_err(|e| format!("{e:#}"));
            let _ = events.send(Event::Library { account, result });
            if let Ok(liked) = api.playlist_tracks(LIKED_MUSIC).await {
                let _ = events.send(Event::Liked(liked.into_iter().map(|t| t.video_id).collect()));
            }
        });
    }

    /// The track `f` acts on: the selection in the track list or queue,
    /// otherwise what's playing.
    fn target_track(&self) -> Option<&Track> {
        if self.minimal {
            return self.now_playing();
        }
        match self.focus {
            Focus::Tracks if !self.show_lyrics => self.list.state.selected().and_then(|i| self.list.tracks.get(i)),
            Focus::Queue => self.queue_state.selected().and_then(|i| self.queue.get(i)),
            _ => self.now_playing(),
        }
    }

    /// Likes or unlikes the target track, updating the heart right away and
    /// undoing it if YouTube refuses.
    fn toggle_like(&mut self) {
        if !self.api.is_signed_in() {
            self.status = Some("Sign in to like songs: needledrop login --browser <name>".into());
            return;
        }
        let Some(track) = self.target_track().cloned() else {
            return;
        };
        let liked = !self.liked.contains(&track.video_id);
        if liked {
            self.liked.insert(track.video_id.clone());
        } else {
            self.liked.remove(&track.video_id);
        }
        self.status = Some(format!(
            "{} “{}”",
            if liked { "Liked" } else { "Removed like from" },
            track.title
        ));
        let api = self.api.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            if let Err(error) = api.set_liked(&track.video_id, liked).await {
                let _ = events.send(Event::LikeFailed {
                    video_id: track.video_id,
                    liked,
                    error: format!("{error:#}"),
                });
            }
        });
    }

    /// Replaces the track list with whatever `fetch` returns, in the background.
    fn load_tracks<F, Fut>(&mut self, title: String, fetch: F)
    where
        F: FnOnce(api::Client) -> Fut + Send + 'static,
        Fut: Future<Output = Result<Vec<Track>>> + Send + 'static,
    {
        self.request += 1;
        let request = self.request;
        self.list = TrackList {
            title,
            tracks: Vec::new(),
            state: TableState::default(),
            loading: true,
        };
        self.status = None;
        let api = self.api.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let result = fetch(api).await.map_err(|e| format!("{e:#}"));
            let _ = events.send(Event::Tracks { request, result });
        });
    }

    fn submit_search(&mut self) {
        let query = self.query.trim().to_owned();
        if query.is_empty() {
            return;
        }
        self.load_tracks(format!("search · {query}"), move |api| async move {
            api.search_songs(&query).await
        });
    }

    fn open_playlist(&mut self, playlist: Playlist) {
        self.focus = Focus::Tracks;
        self.load_tracks(playlist.title, move |api| async move {
            api.playlist_tracks(&playlist.browse_id).await
        });
    }

    fn move_selection(&mut self, delta: isize) {
        let step = |selected: Option<usize>, len: usize| {
            (len > 0).then(|| selected.unwrap_or(0).saturating_add_signed(delta).min(len - 1))
        };
        match self.focus {
            Focus::Library => {
                let len = self.library.playlists().len();
                self.library_state.select(step(self.library_state.selected(), len));
            }
            Focus::Tracks => self
                .list
                .state
                .select(step(self.list.state.selected(), self.list.tracks.len())),
            Focus::Queue => self
                .queue_state
                .select(step(self.queue_state.selected(), self.queue.len())),
            Focus::Search => {}
        }
    }

    /// Library: open the playlist. Tracks: replace the queue with the list
    /// and play from the selection. Queue: jump to the selection.
    async fn activate_selected(&mut self) -> Result<()> {
        match self.focus {
            Focus::Library => {
                let selected = self
                    .library_state
                    .selected()
                    .and_then(|i| self.library.playlists().get(i));
                if let Some(playlist) = selected.cloned() {
                    self.open_playlist(playlist);
                }
                Ok(())
            }
            Focus::Tracks => {
                let Some(index) = self.list.state.selected() else {
                    return Ok(());
                };
                self.replace_queue(self.list.tracks.clone());
                if self.shuffle {
                    // Start with the chosen track, then the rest in random order.
                    self.queue.swap(0, index);
                    fastrand::shuffle(&mut self.queue[1..]);
                    self.queue_state.select(Some(0));
                    return self.play(0).await;
                }
                self.queue_state.select(Some(index));
                self.play(index).await
            }
            Focus::Queue => match self.queue_state.selected() {
                Some(index) => self.play(index).await,
                None => Ok(()),
            },
            Focus::Search => Ok(()),
        }
    }

    async fn enqueue_selected(&mut self) -> Result<()> {
        let Some(track) = self.list.state.selected().and_then(|i| self.list.tracks.get(i)) else {
            return Ok(());
        };
        self.status = Some(format!("Queued “{}”", track.title));
        self.queue.push(track.clone());
        if self.current.is_none() {
            self.play(self.queue.len() - 1).await?;
        }
        Ok(())
    }

    async fn remove_selected(&mut self) -> Result<()> {
        let Some(index) = self.queue_state.selected() else {
            return Ok(());
        };
        self.queue.remove(index);
        match self.current {
            Some(current) if current == index => {
                self.current = None;
                if index < self.queue.len() {
                    self.play(index).await?;
                } else {
                    self.stop().await?;
                }
            }
            Some(current) if current > index => self.current = Some(current - 1),
            _ => {}
        }
        if self.queue.is_empty() {
            self.queue_state.select(None);
        } else {
            self.queue_state.select(Some(index.min(self.queue.len() - 1)));
        }
        Ok(())
    }

    /// Starts the queue entry at `index` in mpv: from its pre-resolved stream
    /// URL when there is one (starts at once), else its watch URL.
    async fn play(&mut self, index: usize) -> Result<()> {
        let Some(track) = self.queue.get(index).cloned() else {
            return Ok(());
        };
        let url = self
            .streams
            .get(&track.video_id)
            .map_or_else(|| track.url(), str::to_owned);
        self.enter_track(index);
        // `loadfile replace` also drops what mpv had queued.
        self.preloaded = None;
        self.player.load(&url).await
    }

    /// Makes the entry at `index` the current track in the app (art, Now
    /// Playing, lyrics, autoplay) without touching mpv. Used both when we
    /// start a track and when mpv moves into a queued one by itself.
    fn enter_track(&mut self, index: usize) {
        let Some(track) = self.queue.get(index).cloned() else {
            return;
        };
        if self.now_playing().is_none_or(|t| t.video_id != track.video_id) {
            self.art = None;
            self.fetch_art(track.clone());
        }
        self.playback.position = 0.0;
        self.playback.position_at = Instant::now();
        self.playback.duration = track.duration.map_or(0.0, |d| d.as_secs_f64());
        self.current = Some(index);
        self.visualizer.clear();
        self.media.send(media::Update::Track {
            title: track.title.clone(),
            artist: track.artists.clone(),
            album: track.album.clone(),
            cover_url: track.thumbnail.as_deref().map(|url| art::sized_url(url, 544)),
            duration: track.duration,
        });
        self.media.send(media::Update::Playing { position: 0.0 });
        self.lyrics_scroll = 0;
        self.load_lyrics();
        // Autoplay: starting the last track fetches more music to follow it.
        if self.repeat == Repeat::Off && index + 1 == self.queue.len() {
            self.start_radio(track.clone(), false);
        }
    }

    /// Keeps mpv's queued next song in line with the app's queue: the entry
    /// that will play when the current one ends, given repeat mode. Resolves
    /// its stream URL first if needed; idempotent, so it runs after every event.
    async fn refresh_preload(&mut self) -> Result<()> {
        let want = self
            .current
            .and_then(|i| next_index(i, self.queue.len(), self.repeat, Advance::Ended))
            .map(|index| Preloaded {
                index,
                video_id: self.queue[index].video_id.clone(),
            });
        match (&mut self.preloaded, &want) {
            // Same song; its position may have shifted (a removal above it).
            (Some(have), Some(want)) if have.video_id == want.video_id => {
                have.index = want.index;
                return Ok(());
            }
            (None, None) => return Ok(()),
            _ => {}
        }
        if self.preloaded.take().is_some() {
            self.player.clear_preloaded().await?;
        }
        let Some(want) = want else { return Ok(()) };
        match self.streams.get(&want.video_id) {
            Some(url) => {
                let url = url.to_owned();
                self.player.preload(&url).await?;
                self.preloaded = Some(want);
            }
            None => self.resolve(self.queue[want.index].clone()),
        }
        Ok(())
    }

    /// Resolves a track's stream URL in the background; `Event::Resolved`
    /// stores it, and the next `refresh_preload` queues it.
    fn resolve(&mut self, track: Track) {
        if !self.resolving.insert(track.video_id.clone()) {
            return;
        }
        let events = self.events.clone();
        tokio::spawn(async move {
            let result = streams::resolve(&track).await.map_err(|e| format!("{e:#}"));
            let _ = events.send(Event::Resolved {
                video_id: track.video_id,
                result,
            });
        });
    }

    fn fetch_art(&self, track: Track) {
        let Some(url) = track.thumbnail else { return };
        let events = self.events.clone();
        tokio::spawn(async move {
            if let Ok(image) = art::fetch(&url).await {
                let _ = events.send(Event::Art {
                    video_id: track.video_id,
                    image,
                });
            }
        });
    }

    async fn advance(&mut self, why: Advance) -> Result<()> {
        let Some(i) = self.current else { return Ok(()) };
        match next_index(i, self.queue.len(), self.repeat, why) {
            Some(next) => self.play(next).await,
            None => {
                let end = self.queue.len();
                self.stop().await?;
                // Autoplay is still fetching more music: carry on when it lands.
                if self.radio_loading && self.repeat == Repeat::Off {
                    self.resume_at = Some(end);
                }
                Ok(())
            }
        }
    }

    fn replace_queue(&mut self, tracks: Vec<Track>) {
        self.queue = tracks;
        self.radio_request += 1;
        self.radio_loading = false;
        self.resume_at = None;
    }

    /// Loads the radio for `seed`. `replace`: make it the queue and play it;
    /// otherwise append it (autoplay).
    fn start_radio(&mut self, seed: Track, replace: bool) {
        self.radio_request += 1;
        self.radio_loading = true;
        let request = self.radio_request;
        let api = self.api.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let result = api
                .watch_next(&seed.video_id)
                .await
                .map(|next| next.radio)
                .map_err(|e| format!("{e:#}"));
            let _ = events.send(Event::Radio {
                request,
                replace,
                result,
            });
        });
    }

    async fn handle_radio(&mut self, request: u64, replace: bool, result: Result<Vec<Track>, String>) -> Result<()> {
        if request != self.radio_request {
            return Ok(());
        }
        self.radio_loading = false;
        let tracks = match result {
            Ok(tracks) => tracks,
            Err(error) => {
                self.status = Some(format!("Couldn't load radio: {error}"));
                self.resume_at = None;
                return Ok(());
            }
        };
        if replace {
            self.status = None;
            if tracks.is_empty() {
                return Ok(());
            }
            self.replace_queue(tracks);
            self.queue_state.select(Some(0));
            return self.play(0).await;
        }
        let known: std::collections::HashSet<String> = self.queue.iter().map(|t| t.video_id.clone()).collect();
        self.queue
            .extend(tracks.into_iter().filter(|t| !known.contains(&t.video_id)));
        match self.resume_at.take() {
            Some(start) if start < self.queue.len() => self.play(start).await,
            _ => Ok(()),
        }
    }

    fn lyrics_have_focus(&self) -> bool {
        self.show_lyrics && self.focus == Focus::Tracks
    }

    /// Fetches lyrics for the current track if lyrics are on screen (the
    /// lyrics view or minimal mode) and they aren't loaded yet. Only then, so
    /// lyrics services never see what you play unless you look at lyrics.
    fn load_lyrics(&mut self) {
        let Some(track) = self.now_playing().cloned() else {
            return;
        };
        let visible = self.show_lyrics || self.minimal;
        if !visible || self.lyrics.as_ref().is_some_and(|(id, _)| *id == track.video_id) {
            return;
        }
        self.lyrics = Some((track.video_id.clone(), LyricsState::Loading));
        let api = self.api.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let result = lyrics::fetch(&api, &track).await.map_err(|e| format!("{e:#}"));
            let _ = events.send(Event::Lyrics {
                video_id: track.video_id,
                result,
            });
        });
    }

    /// Shuffles the tracks after the current one. Turning shuffle off keeps
    /// the current order.
    fn set_shuffle(&mut self, on: bool) {
        self.shuffle = on;
        if on {
            let upcoming = self.current.map_or(0, |i| i + 1);
            if upcoming < self.queue.len() {
                fastrand::shuffle(&mut self.queue[upcoming..]);
            }
        }
        self.media.send(media::Update::Shuffle(on));
    }

    async fn handle_media(&mut self, command: Command) -> Result<()> {
        if self.current.is_none() && !matches!(command, Command::SetShuffle(_)) {
            return Ok(());
        }
        match command {
            Command::Play => self.player.set_paused(false).await?,
            Command::Pause => self.player.set_paused(true).await?,
            Command::Toggle => self.player.toggle_pause().await?,
            Command::Next => self.advance(Advance::Skip).await?,
            Command::Previous => self.previous().await?,
            Command::Stop => self.stop().await?,
            Command::SeekBy(seconds) => self.player.seek(seconds).await?,
            Command::SeekTo(seconds) => self.player.seek_to(seconds).await?,
            Command::SetShuffle(on) => self.set_shuffle(on),
        }
        Ok(())
    }

    /// Tells the OS whether we're playing and where, for its progress bar.
    fn report_playback(&self) {
        if self.current.is_none() {
            return;
        }
        let position = self.playback.current_position();
        self.media.send(if self.playback.paused {
            media::Update::Paused { position }
        } else {
            media::Update::Playing { position }
        });
    }

    async fn previous(&mut self) -> Result<()> {
        match self.current {
            // Like most players: restart the track unless we're near its start.
            Some(i) if self.playback.position > 3.0 || i == 0 => self.play(i).await,
            Some(i) => self.play(i - 1).await,
            None => Ok(()),
        }
    }

    async fn stop(&mut self) -> Result<()> {
        self.current = None;
        // mpv's `stop` clears its playlist, queued song included.
        self.preloaded = None;
        self.resume_at = None;
        self.art = None;
        self.visualizer.clear();
        self.media.send(media::Update::Stopped);
        self.playback.position = 0.0;
        self.playback.duration = 0.0;
        self.player.stop().await
    }
}

/// The queue index to play after `current`, or `None` to stop.
fn next_index(current: usize, len: usize, repeat: Repeat, why: Advance) -> Option<usize> {
    match (repeat, why) {
        (Repeat::One, Advance::Ended) => Some(current),
        _ if current + 1 < len => Some(current + 1),
        (Repeat::All, _) if len > 0 => Some(0),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_track_follows_repeat_mode() {
        use Advance::{Ended, Skip};
        // Off: advance, stop at the end.
        assert_eq!(next_index(0, 3, Repeat::Off, Ended), Some(1));
        assert_eq!(next_index(2, 3, Repeat::Off, Ended), None);
        // All: wrap around.
        assert_eq!(next_index(2, 3, Repeat::All, Ended), Some(0));
        assert_eq!(next_index(2, 3, Repeat::All, Skip), Some(0));
        // One: replay when the track ends, but a skip still moves on.
        assert_eq!(next_index(1, 3, Repeat::One, Ended), Some(1));
        assert_eq!(next_index(1, 3, Repeat::One, Skip), Some(2));
        assert_eq!(next_index(2, 3, Repeat::One, Skip), None);
    }
}
