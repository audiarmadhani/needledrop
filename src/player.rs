//! Playback through an `mpv` child process, driven over its JSON IPC socket.
//!
//! mpv resolves YouTube URLs with yt-dlp, so stream extraction stays
//! yt-dlp's problem and gets fixed by updating it.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::net::unix::OwnedWriteHalf;
use tokio::process::{Child, Command};
use tokio::sync::mpsc::UnboundedSender;

use crate::event::Event;

#[derive(Debug, Clone, PartialEq)]
pub enum PlayerEvent {
    Position(f64),
    Duration(f64),
    Paused(bool),
    Volume(f64),
    /// The current track played to the end.
    TrackEnded,
    /// mpv could not play the current track.
    TrackFailed,
    /// mpv resolved the current track to this media URL.
    Stream(String),
    /// The mpv process went away.
    Exited,
}

const OBSERVED_PROPERTIES: [&str; 5] = ["time-pos", "duration", "pause", "volume", "stream-open-filename"];

pub struct Player {
    writer: OwnedWriteHalf,
    socket: PathBuf,
    _mpv: Child,
}

impl Player {
    /// Starts mpv. Streams are fetched signed out on purpose: handing the
    /// login cookies to yt-dlp got the copied session cleared by YouTube,
    /// and yt-dlp then saved the cleared cookies back over the login.
    pub async fn spawn(events: UnboundedSender<Event>) -> Result<Self> {
        let socket = std::env::temp_dir().join(format!("needledrop-{}.sock", std::process::id()));
        let mpv = Command::new("mpv")
            .args([
                "--idle=yes",
                "--no-video",
                "--no-terminal",
                "--ytdl-format=bestaudio/best",
            ])
            .arg(format!("--input-ipc-server={}", socket.display()))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("could not start mpv; is it installed and on your PATH?")?;

        let stream = connect(&socket).await?;
        let (reader, writer) = stream.into_split();
        tokio::spawn(async move {
            let mut lines = BufReader::new(reader).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(message) = serde_json::from_str::<Value>(&line) else { continue };
                if let Some(event) = parse_event(&message) {
                    let _ = events.send(Event::Player(event));
                }
            }
            let _ = events.send(Event::Player(PlayerEvent::Exited));
        });

        let mut player = Self { writer, socket, _mpv: mpv };
        for (id, property) in OBSERVED_PROPERTIES.iter().enumerate() {
            player.command(json!(["observe_property", id + 1, property])).await?;
        }
        Ok(player)
    }

    pub async fn load(&mut self, url: &str) -> Result<()> {
        self.command(json!(["loadfile", url, "replace"])).await?;
        self.command(json!(["set_property", "pause", false])).await
    }

    pub async fn toggle_pause(&mut self) -> Result<()> {
        self.command(json!(["cycle", "pause"])).await
    }

    pub async fn set_paused(&mut self, paused: bool) -> Result<()> {
        self.command(json!(["set_property", "pause", paused])).await
    }

    pub async fn seek_to(&mut self, seconds: f64) -> Result<()> {
        self.command(json!(["seek", seconds, "absolute"])).await
    }

    pub async fn seek(&mut self, seconds: f64) -> Result<()> {
        self.command(json!(["seek", seconds, "relative"])).await
    }

    pub async fn change_volume(&mut self, delta: f64) -> Result<()> {
        self.command(json!(["add", "volume", delta])).await
    }

    pub async fn stop(&mut self) -> Result<()> {
        self.command(json!(["stop"])).await
    }

    async fn command(&mut self, args: Value) -> Result<()> {
        let mut line = json!({ "command": args }).to_string();
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .await
            .context("lost connection to mpv")
    }
}

impl Drop for Player {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.socket);
    }
}

async fn connect(socket: &PathBuf) -> Result<UnixStream> {
    for _ in 0..250 {
        if let Ok(stream) = UnixStream::connect(socket).await {
            return Ok(stream);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    bail!("mpv did not open its IPC socket at {}", socket.display())
}

fn parse_event(message: &Value) -> Option<PlayerEvent> {
    match message["event"].as_str()? {
        "property-change" => {
            let data = &message["data"];
            match message["name"].as_str()? {
                "time-pos" => data.as_f64().map(PlayerEvent::Position),
                "duration" => data.as_f64().map(PlayerEvent::Duration),
                "pause" => data.as_bool().map(PlayerEvent::Paused),
                "volume" => data.as_f64().map(PlayerEvent::Volume),
                "stream-open-filename" => data.as_str().and_then(media_url).map(PlayerEvent::Stream),
                _ => None,
            }
        }
        "end-file" => match message["reason"].as_str()? {
            "eof" => Some(PlayerEvent::TrackEnded),
            "error" => Some(PlayerEvent::TrackFailed),
            _ => None,
        },
        _ => None,
    }
}

/// mpv's ytdl hook replaces the watch URL with an EDL playlist like
/// `edl://!new_stream;…;%1223%https://…googlevideo.com/…;…`, where `%N%`
/// prefixes a value N bytes long. Returns the media URL inside it.
fn media_url(stream: &str) -> Option<String> {
    let Some(edl) = stream.strip_prefix("edl://") else {
        // Formats that aren't split into separate audio and video come through directly.
        return stream.contains(".googlevideo.com/").then(|| stream.to_owned());
    };
    let (_, rest) = edl.split_once('%')?;
    let (length, rest) = rest.split_once('%')?;
    let url = rest.get(..length.parse().ok()?)?;
    url.starts_with("https://").then(|| url.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_mpv_events() {
        let cases = [
            (json!({"event":"property-change","id":1,"name":"time-pos","data":12.5}), Some(PlayerEvent::Position(12.5))),
            (json!({"event":"property-change","id":1,"name":"time-pos"}), None),
            (json!({"event":"property-change","id":3,"name":"pause","data":true}), Some(PlayerEvent::Paused(true))),
            (json!({"event":"end-file","reason":"eof"}), Some(PlayerEvent::TrackEnded)),
            (json!({"event":"end-file","reason":"stop"}), None),
            (json!({"event":"end-file","reason":"error","file_error":"x"}), Some(PlayerEvent::TrackFailed)),
            (json!({"data":null,"error":"success","request_id":0}), None),
            (json!({"event":"property-change","name":"stream-open-filename","data":"https://music.youtube.com/watch?v=x"}), None),
            (
                json!({"event":"property-change","name":"stream-open-filename","data":"edl://!new_stream;!no_clip;%32%https://r1.googlevideo.com/a;b=1;!global_tags,x=%3%abc"}),
                Some(PlayerEvent::Stream("https://r1.googlevideo.com/a;b=1".into())),
            ),
        ];
        for (message, expected) in cases {
            assert_eq!(parse_event(&message), expected, "{message}");
        }
    }
}
