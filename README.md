<div align="center">

<img src="docs/assets/icon.png" width="128" height="128" alt="needledrop icon: a pixel-art vinyl record with the needle dropping">

# needledrop

**A good-looking terminal client for YouTube Music, written in Rust.**

[![CI](https://github.com/audiarmadhani/needledrop/actions/workflows/ci.yml/badge.svg)](https://github.com/audiarmadhani/needledrop/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/audiarmadhani/needledrop?color=ff004d)](https://github.com/audiarmadhani/needledrop/releases)
[![License](https://img.shields.io/badge/license-MIT%20%2F%20Apache--2.0-29366f)](#license)
![Platforms](https://img.shields.io/badge/platform-macOS%20%7C%20Linux-ffa300)
[![Rust](https://img.shields.io/badge/rust-1.88%2B-83769c)](https://www.rust-lang.org)

[Website](https://audiarmadhani.github.io/needledrop/) ·
[Install](#install) ·
[Keys](#keys) ·
[How it works](#how-it-works)

<img src="docs/assets/demo.gif" alt="needledrop searching for a song, playing it with a visualizer, cycling album art palettes, showing synced lyrics, and switching to minimal mode">
- Home: your Quick picks, mixes, Listen again, and recommendations, as rows of pixel-art cover cards
- Artist and album pages, and search across songs, albums, artists, and playlists
- Your YouTube Music library: Liked Music and playlists, with likes you can toggle
- Search, a queue with shuffle and repeat, and endless autoplay (radio) when the queue runs out
- Album covers as 8-bit pixel art, in your terminal theme's colors or the PICO-8, Game Boy, or C64 palettes
- Visualizers synced to the music, after Windows Media Player's: spectrum bars, bars with falling peaks, and an oscilloscope
- A minimal, full-screen now-playing mode
- Follows your terminal's color theme, live
- Time-synced lyrics
- Media keys and the macOS Now Playing widget (MPRIS on Linux)

</div>

## Features

|  |  |
|---|---|
| 🏠 **Home** | Your Quick picks, mixes, Listen again, and recommendations |
| 🔎 **Browse and search** | Artist and album pages; search across songs, albums, artists, and playlists |
| 💜 **Your library** | Liked Music and playlists, with likes you can toggle |
| 📻 **Endless queue** | Shuffle, repeat, and autoplay (radio) when the queue runs out, with gapless playback |
| 👾 **8-bit album art** | Covers as pixel art in your terminal theme's colors, or the PICO-8, Game Boy, or C64 palettes |
| 📊 **Visualizers** | Synced to the music, after Windows Media Player's: spectrum bars, bars with falling peaks, and an oscilloscope |
| 🎤 **Lyrics** | Time-synced, from LRCLIB, with YouTube Music's own as a fallback |
| 🖥️ **Minimal mode** | Cover, lyrics, visualizer, and progress, full screen |
| 🎨 **Follows your theme** | Draws with your terminal's palette and recolors live when you switch themes |
| ⏯️ **Media keys** | The macOS Now Playing widget, or MPRIS on Linux |

## Install

needledrop runs on macOS and Linux. It needs [`mpv`](https://mpv.io) for
playback, [`yt-dlp`](https://github.com/yt-dlp/yt-dlp) (which mpv uses to
resolve YouTube streams), and [`ffmpeg`](https://ffmpeg.org) for the
visualizer.

```sh
# 1. Dependencies (macOS; use your package manager on Linux)
brew install mpv yt-dlp ffmpeg

# 2. needledrop
cargo install --locked --git https://github.com/audiarmadhani/needledrop

# 3. Sign in with a browser where you're signed in to YouTube Music (optional)
needledrop login --browser chrome   # or arc, brave, edge, firefox, zen, vivaldi, …

# 4. Play
needledrop
```

Without signing in, search and playback still work.

> [!TIP]
> If playback stops working, update yt-dlp first. YouTube changes often, and
> yt-dlp usually fixes it within days.

<details>
<summary><b>macOS: install as an app</b></summary>

```sh
needledrop install-app
```

This wraps needledrop in `~/Applications/needledrop.app` (marked as a music
app, with no Dock icon) and links `~/.local/bin/needledrop` to it. Control
Center's Now Playing works either way, but notch and menu bar utilities such as
Vorssaint only list players that are music *apps*, and a plain binary started
from a terminal isn't one. After that, any other copy of needledrop you run
(a Homebrew upgrade, a fresh build) updates the app and runs from it.

</details>

<details>
<summary><b>More about signing in</b></summary>

If the browser is signed in to several Google accounts, `login` lists them and
asks which one has your library (or pass `--account N`). On macOS, expect a
Keychain prompt so needledrop can decrypt the browser's cookies.

The login is saved to `~/.config/needledrop/` with owner-only permissions.
Each launch re-reads fresh cookies from the same browser, since the browser
keeps rotating them while you use YouTube.
`needledrop logout` deletes it. If your browser isn't supported, use
`needledrop login --paste` and paste a `Cookie` request header copied from
devtools.

If login says the cookies look signed out, open music.youtube.com in that
browser first. YouTube expires sessions that sit unused.

</details>

<details>
<summary><b>Build from source</b></summary>

```sh
git clone https://github.com/audiarmadhani/needledrop
cd needledrop
cargo run --release
```

</details>

## Keys

<table>
<tr><th>Navigate</th><th>Play</th><th>View</th></tr>
<tr valign="top"><td>

| Key | Action |
|---|---|
| `tab` / `shift-tab` | Next / previous pane |
| `j` / `k` | Move down / up |
| `g` / `G` | Top / bottom |
| `enter` | Play, or open |
| `esc` / `backspace` | Back |
| `i` / `o` | Go to artist / album |
| `/` | Search |
| `ctrl-r` | Refresh library |
| `q`, `ctrl-c` | Quit |

</td><td>

| Key | Action |
|---|---|
| `space` | Pause / resume |
| `n` / `p` | Next / previous |
| `←` / `→` (`h` / `l`) | Seek 5 s |
| `tab` / `shift-tab` | Move between library, tracks, and queue |
| `j` / `k`, `g` / `G` | Move (between shelves on card pages), jump to top or bottom |
| `h` / `l`, `←` / `→` | On card pages: move along a shelf |
| `enter` | Play a song (queueing its list or shelf), or open an album, artist, or playlist |
| `esc` / `backspace` | Back to the previous page |
| `i` / `o` | Open the selected (or playing) song's artist / album |
| `/` | Search |
| `a` | Add the selected track to the queue |
| `d` | Remove the selected track from the queue |
| `space` | Pause or resume |
| `n` / `p` | Next or previous track |
| `,` / `.` | Seek 5 s (also `←` / `→` and `h` / `l` outside card pages) |
| `+` / `-` | Volume |
| `a` / `d` | Add to / remove from queue |
| `s` | Shuffle the rest |
| `r` | Repeat: off → all → one |
| `R` | Radio from this track |
| `f` | Like / unlike |

</td><td>

| Key | Action |
|---|---|
| `L` | Lyrics |
| `M` | Minimal mode |
| `V` | Visualizer: bars → peaks → scope |
| `A` | Art: theme → PICO-8 → Game Boy → C64 → original |

</td></tr>
</table>

`enter` on a song plays it and queues the rest of its list or shelf. `f` in the
library pane likes the playing song. When the last track in the queue starts
and repeat is off, needledrop appends a radio based on it, so the music keeps
going.

## Picking up where you left off

needledrop remembers your settings (art style, visualizer, volume, shuffle,
repeat) in `~/.config/needledrop/settings.json`, and your queue, song, and
position in `~/.local/state/needledrop/session.json`. On launch, the song you
were on is loaded paused where you stopped: press space to carry on.

## Colors

needledrop draws with your terminal's palette colors rather than its own, so
it matches your terminal theme, and when you switch themes it recolors
immediately, with no restart.

In [Ghostty](https://ghostty.org), the album art can also use the theme's
colors (the default art style): needledrop reads the theme from Ghostty's
config, including `config-file` includes and `palette` overrides, and checks
for a switch every 2 seconds. In other terminals, the art starts in the PICO-8
palette.

## Lyrics

Time-synced lyrics come from [LRCLIB](https://lrclib.net), an open lyrics
database; otherwise needledrop shows YouTube Music's own lyrics. Lyrics are
only fetched while the lyrics view is open, so LRCLIB sees a song's title,
artist, album, and length only when you look at its lyrics.

## How it works

<details>
<summary>A tour of the source</summary>

- `src/auth.rs`: reads login cookies from your browser and signs requests the
  way YouTube's web app does (`SAPISIDHASH`).
- `src/api.rs`: talks to YouTube Music's private InnerTube API, the same one
  the web app uses. [ytmusicapi](https://github.com/sigma67/ytmusicapi) is the
  reference for its response shapes.
- `src/player.rs`: runs `mpv` as a child process and controls it over its JSON
  IPC socket.
- `src/streams.rs`: gapless playback. While a song plays, needledrop resolves
  the next one's stream URL with yt-dlp and queues it in mpv's playlist, so
  mpv plays straight into it instead of pausing ~5 seconds to look it up.
- `src/visualizer.rs`: decodes the same stream again with ffmpeg and runs an
  FFT at mpv's playback position, so the bars stay in sync through seeks.
- `src/media.rs`: OS media controls. On macOS they need the main thread's run
  loop, so the TUI runs on a worker thread.
- `src/app.rs`: app state and key handling. `src/ui.rs` renders it with
  [ratatui](https://ratatui.rs), using the colors in `src/theme.rs`.

</details>

Windows isn't supported yet, because the player uses a Unix socket.

Playback is always signed out: handing your login cookies to yt-dlp risks
YouTube flagging the session, so there's no Premium audio quality.

## Disclaimer

needledrop is not affiliated with YouTube or Google. It uses unofficial APIs
that can change without notice.

## License

Licensed under either of [Apache License 2.0](LICENSE-APACHE) or
[MIT](LICENSE-MIT), at your option.
