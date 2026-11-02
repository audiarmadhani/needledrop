//! A small client for YouTube Music's private InnerTube API.
//!
//! The response shapes come from reverse engineering (see the Python
//! `ytmusicapi` project), so parsing is defensive: anything unexpected is
//! skipped instead of failing the whole request.

use std::time::Duration;

use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::auth::{Credentials, ORIGIN};
use crate::model::{Playlist, Track};

const ENDPOINT: &str = "https://music.youtube.com/youtubei/v1";
const CLIENT_NAME: &str = "WEB_REMIX";
const CLIENT_VERSION: &str = "1.20251020.01.00";
const USER_AGENT: &str = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 \
                          (KHTML, like Gecko) Chrome/130.0 Safari/537.36";
/// Search `params` value that restricts results to songs.
const SONGS_FILTER: &str = "EgWKAQIIAWoMEA4QChADEAQQCRAF";
/// Upper bound on continuation pages followed per request (100 tracks each).
const MAX_PAGES: usize = 100;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Account {
    pub name: String,
    /// The email or channel handle, to tell same-named accounts apart.
    pub detail: Option<String>,
}

impl std::fmt::Display for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.detail {
            Some(detail) => write!(f, "{} ({detail})", self.name),
            None => f.write_str(&self.name),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WatchNext {
    pub radio: Vec<Track>,
    pub lyrics_id: Option<String>,
}

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    credentials: Option<Arc<Credentials>>,
}

impl Client {
    pub fn new(credentials: Option<Credentials>) -> Result<Self> {
        let http = reqwest::Client::builder().user_agent(USER_AGENT).build()?;
        Ok(Self { http, credentials: credentials.map(Arc::new) })
    }

    pub fn is_signed_in(&self) -> bool {
        self.credentials.is_some()
    }

    pub async fn search_songs(&self, query: &str) -> Result<Vec<Track>> {
        let response = self.post("search", &[], json!({ "query": query, "params": SONGS_FILTER })).await?;
        Ok(parse_search(&response))
    }

    /// The signed-in account, or `None` if YouTube treats the request as
    /// signed out (for example, expired cookies).
    pub async fn account(&self) -> Result<Option<Account>> {
        let response = self.post("account/account_menu", &[], json!({})).await?;
        let header = &response["actions"][0]["openPopupAction"]["popup"]["multiPageMenuRenderer"]
            ["header"]["activeAccountHeaderRenderer"];
        let Some(name) = header["accountName"]["runs"][0]["text"].as_str() else { return Ok(None) };
        let detail = ["email", "channelHandle"]
            .iter()
            .find_map(|key| header[*key]["runs"][0]["text"].as_str())
            .map(str::to_owned);
        Ok(Some(Account { name: name.to_owned(), detail }))
    }

    /// Liked Music and the user's playlists, in library order.
    pub async fn library_playlists(&self) -> Result<Vec<Playlist>> {
        let mut response = self.post("browse", &[], json!({ "browseId": "FEmusic_liked_playlists" })).await?;
        let mut grid = response["contents"]["singleColumnBrowseResultsRenderer"]["tabs"][0]["tabRenderer"]
            ["content"]["sectionListRenderer"]["contents"][0]["gridRenderer"]
            .take();
        let mut playlists = Vec::new();
        for _ in 0..MAX_PAGES {
            playlists.extend(parse_grid_playlists(&grid));
            let Some(token) = grid["continuations"][0]["nextContinuationData"]["continuation"].as_str() else {
                break;
            };
            let query = [("ctoken", token), ("continuation", token), ("type", "next")];
            let mut response = self.post("browse", &query, json!({})).await?;
            grid = response["continuationContents"]["gridContinuation"].take();
        }
        Ok(playlists)
    }

    /// Every playable track in a playlist, following pagination.
    pub async fn playlist_tracks(&self, browse_id: &str) -> Result<Vec<Track>> {
        let mut response = self.post("browse", &[], json!({ "browseId": browse_id })).await?;
        let mut items = response["contents"]["twoColumnBrowseResultsRenderer"]["secondaryContents"]
            ["sectionListRenderer"]["contents"][0]["musicPlaylistShelfRenderer"]["contents"]
            .take();
        let mut tracks = Vec::new();
        for _ in 0..MAX_PAGES {
            let (page, next) = parse_playlist_page(&items);
            tracks.extend(page);
            let Some(token) = next else { break };
            response = self.post("browse", &[], json!({ "continuation": token })).await?;
            items = response["onResponseReceivedActions"][0]["appendContinuationItemsAction"]
                ["continuationItems"]
                .take();
        }
        Ok(tracks)
    }

    /// The automix radio for a track (starting with the track itself), and
    /// the browse id of its lyrics if YouTube has any.
    pub async fn watch_next(&self, video_id: &str) -> Result<WatchNext> {
        let body = json!({ "videoId": video_id, "playlistId": format!("RDAMVM{video_id}"), "isAudioOnly": true });
        let response = self.post("next", &[], body).await?;
        Ok(parse_watch_next(&response))
    }

    /// Plain lyrics text and its source credit (e.g. "Source: LyricFind").
    pub async fn lyrics(&self, browse_id: &str) -> Result<Option<(String, String)>> {
        let response = self.post("browse", &[], json!({ "browseId": browse_id })).await?;
        let shelf = &response["contents"]["sectionListRenderer"]["contents"][0]["musicDescriptionShelfRenderer"];
        let text = runs_text(&shelf["description"]).replace('\r', "");
        Ok((!text.trim().is_empty()).then(|| (text, runs_text(&shelf["footer"]))))
    }

    /// Adds a track to, or removes it from, the user's Liked Music.
    pub async fn set_liked(&self, video_id: &str, liked: bool) -> Result<()> {
        if !self.is_signed_in() {
            bail!("sign in to like songs");
        }
        let endpoint = if liked { "like/like" } else { "like/removelike" };
        self.post(endpoint, &[], json!({ "target": { "videoId": video_id } })).await?;
        Ok(())
    }

    /// POSTs to an InnerTube endpoint. `body` is merged with the client context.
    async fn post(&self, endpoint: &str, query: &[(&str, &str)], mut body: Value) -> Result<Value> {
        body["context"] = json!({ "client": {
            "clientName": CLIENT_NAME,
            "clientVersion": CLIENT_VERSION,
            "hl": "en",
            "gl": "US",
        }});
        let mut request = self
            .http
            .post(format!("{ENDPOINT}/{endpoint}"))
            .query(&[("prettyPrint", "false")])
            .query(query)
            .header("Origin", ORIGIN)
            .header("X-Origin", ORIGIN)
            .json(&body);
        if let Some(credentials) = &self.credentials {
            request = request
                .header("Cookie", credentials.cookie_header())
                .header("Authorization", credentials.authorization())
                .header("X-Goog-AuthUser", credentials.account.to_string());
        }
        let response = request.send().await?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            bail!("YouTube rejected the saved login ({status}); run `needledrop login` again");
        }
        Ok(response.error_for_status()?.json().await?)
    }
}

fn parse_search(response: &Value) -> Vec<Track> {
    let sections = &response["contents"]["tabbedSearchResultsRenderer"]["tabs"][0]["tabRenderer"]
        ["content"]["sectionListRenderer"]["contents"];
    sections
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|section| section["musicShelfRenderer"]["contents"].as_array())
        .flatten()
        .filter_map(|item| parse_song(&item["musicResponsiveListItemRenderer"]))
        .collect()
}

/// Parses one result row. Column 0 is the title; column 1 is
/// `artists • album • m:ss`.
fn parse_song(item: &Value) -> Option<Track> {
    let video_id = item["playlistItemData"]["videoId"].as_str()?.to_owned();
    let title = column_text(item, 0);
    let meta = column_text(item, 1);

    let mut parts: Vec<&str> = meta.split(" • ").collect();
    let duration = parts.last().and_then(|last| parse_duration(last));
    if duration.is_some() {
        parts.pop();
    }
    let mut parts = parts.into_iter();
    let artists = parts.next().unwrap_or_default().to_owned();
    let album = parts.next().map(str::to_owned);

    Some(Track { video_id, title, artists, album, duration, thumbnail: thumbnail_url(item) })
}

fn parse_watch_next(response: &Value) -> WatchNext {
    let tabs = &response["contents"]["singleColumnMusicWatchNextResultsRenderer"]["tabbedRenderer"]
        ["watchNextTabbedResultsRenderer"]["tabs"];
    let items = &tabs[0]["tabRenderer"]["content"]["musicQueueRenderer"]["content"]["playlistPanelRenderer"]["contents"];
    let radio = items
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let video = &item["playlistPanelVideoRenderer"];
            // Songs with a music-video counterpart come wrapped.
            let video = if video.is_null() {
                &item["playlistPanelVideoWrapperRenderer"]["primaryRenderer"]["playlistPanelVideoRenderer"]
            } else {
                video
            };
            parse_panel_video(video)
        })
        .collect();
    // The lyrics tab is the one whose browse id starts with "MPLY".
    let lyrics_id = tabs
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|tab| tab["tabRenderer"]["endpoint"]["browseEndpoint"]["browseId"].as_str())
        .find(|id| id.starts_with("MPLY"))
        .map(str::to_owned);
    WatchNext { radio, lyrics_id }
}

/// A radio entry: the byline is `artists • album • year` for songs, or
/// `artists • views` for videos.
fn parse_panel_video(video: &Value) -> Option<Track> {
    let video_id = video["videoId"].as_str()?.to_owned();
    let byline = runs_text(&video["longBylineText"]);
    let parts: Vec<&str> = byline.split(" • ").collect();
    Some(Track {
        video_id,
        title: runs_text(&video["title"]),
        artists: parts.first().copied().unwrap_or_default().to_owned(),
        album: (parts.len() >= 3).then(|| parts[1].to_owned()),
        duration: parse_duration(&runs_text(&video["lengthText"])),
        thumbnail: video["thumbnail"]["thumbnails"].as_array()?.last()?["url"].as_str().map(str::to_owned),
    })
}

fn parse_grid_playlists(grid: &Value) -> Vec<Playlist> {
    grid["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let item = &item["musicTwoRowItemRenderer"];
            // The "New playlist" tile has a create endpoint instead of a browse id.
            let browse_id = item["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str()?;
            Some(Playlist {
                browse_id: browse_id.to_owned(),
                title: runs_text(&item["title"]),
                subtitle: runs_text(&item["subtitle"]),
            })
        })
        .collect()
}

/// Parses one page of playlist rows. Returns the tracks and the token for
/// the next page, which YouTube appends as a final `continuationItemRenderer`.
fn parse_playlist_page(items: &Value) -> (Vec<Track>, Option<String>) {
    let items = items.as_array().map(Vec::as_slice).unwrap_or_default();
    let tracks = items
        .iter()
        .filter_map(|item| parse_playlist_row(&item["musicResponsiveListItemRenderer"]))
        .collect();
    let next = items
        .last()
        .and_then(|item| {
            item["continuationItemRenderer"]["continuationEndpoint"]["continuationCommand"]["token"].as_str()
        })
        .map(str::to_owned);
    (tracks, next)
}

/// Playlist rows have separate columns: title, artists, album; the duration
/// is in a fixed column. Unavailable tracks have no video id and are skipped.
fn parse_playlist_row(item: &Value) -> Option<Track> {
    let video_id = item["playlistItemData"]["videoId"].as_str()?.to_owned();
    let album = column_text(item, 2);
    let duration_text = &item["fixedColumns"][0]["musicResponsiveListItemFixedColumnRenderer"]["text"];
    let duration = duration_text["simpleText"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| runs_text(duration_text));
    Some(Track {
        video_id,
        title: column_text(item, 0),
        artists: column_text(item, 1),
        album: (!album.is_empty()).then_some(album),
        duration: parse_duration(&duration),
        thumbnail: thumbnail_url(item),
    })
}

/// The largest thumbnail offered for a result row.
fn thumbnail_url(item: &Value) -> Option<String> {
    item["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]
        .as_array()?
        .last()?["url"]
        .as_str()
        .map(str::to_owned)
}

fn column_text(item: &Value, column: usize) -> String {
    runs_text(&item["flexColumns"][column]["musicResponsiveListItemFlexColumnRenderer"]["text"])
}

fn runs_text(text: &Value) -> String {
    text["runs"].as_array().into_iter().flatten().filter_map(|run| run["text"].as_str()).collect()
}

/// Parses `m:ss` or `h:mm:ss`.
fn parse_duration(text: &str) -> Option<Duration> {
    if !text.contains(':') {
        return None;
    }
    let mut seconds = 0u64;
    for part in text.split(':') {
        seconds = seconds * 60 + part.parse::<u64>().ok()?;
    }
    Some(Duration::from_secs(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(video_id: &str, title: &str, meta_runs: &[&str]) -> Value {
        let runs: Vec<Value> = meta_runs.iter().map(|t| json!({ "text": t })).collect();
        json!({ "musicResponsiveListItemRenderer": {
            "playlistItemData": { "videoId": video_id },
            "flexColumns": [
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": title }] } } },
                { "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": runs } } },
            ],
        }})
    }

    fn response(rows: Vec<Value>) -> Value {
        json!({ "contents": { "tabbedSearchResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
            "sectionListRenderer": { "contents": [
                { "itemSectionRenderer": {} },
                { "musicShelfRenderer": { "contents": rows } },
            ]}
        }}}]}}})
    }

    #[test]
    fn parses_songs_from_search_response() {
        let mut weird_fishes = row("LUjGtyYEi90", "Weird Fishes / Arpeggi", &["Radiohead", " • ", "In Rainbows", " • ", "5:19"]);
        weird_fishes["musicResponsiveListItemRenderer"]["thumbnail"] = json!({ "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [
            { "url": "https://lh3.googleusercontent.com/t=w60-h60" },
            { "url": "https://lh3.googleusercontent.com/t=w120-h120" },
        ]}}});
        let tracks = parse_search(&response(vec![
            weird_fishes,
            row("abc", "Collab", &["A", " & ", "B", " • ", "Single", " • ", "1:02:03"]),
            json!({ "musicResponsiveListItemRenderer": {} }),
        ]));

        assert_eq!(tracks, vec![
            Track {
                video_id: "LUjGtyYEi90".into(),
                title: "Weird Fishes / Arpeggi".into(),
                artists: "Radiohead".into(),
                album: Some("In Rainbows".into()),
                duration: Some(Duration::from_secs(319)),
                thumbnail: Some("https://lh3.googleusercontent.com/t=w120-h120".into()),
            },
            Track {
                video_id: "abc".into(),
                title: "Collab".into(),
                artists: "A & B".into(),
                album: Some("Single".into()),
                duration: Some(Duration::from_secs(3723)),
                thumbnail: None,
            },
        ]);
    }

    fn playlist_row(video_id: Option<&str>, title: &str, artists: &str, album: &str, duration: &str) -> Value {
        let column = |text: &str| json!({ "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": [{ "text": text }] } } });
        let mut row = json!({
            "flexColumns": [column(title), column(artists), column(album)],
            "fixedColumns": [{ "musicResponsiveListItemFixedColumnRenderer": { "text": { "runs": [{ "text": duration }] } } }],
        });
        if let Some(id) = video_id {
            row["playlistItemData"] = json!({ "videoId": id, "playlistSetVideoId": "x" });
        }
        json!({ "musicResponsiveListItemRenderer": row })
    }

    #[test]
    fn parses_playlist_page_and_continuation() {
        let items = json!([
            playlist_row(Some("v1"), "Jazz Festival", "Straizo", "Postcards From Molde", "2:21"),
            playlist_row(None, "Deleted video", "", "", ""),
            playlist_row(Some("v2"), "No Album", "Someone", "", "3:00"),
            { "continuationItemRenderer": { "continuationEndpoint": { "continuationCommand": { "token": "NEXT" } } } },
        ]);
        let (tracks, next) = parse_playlist_page(&items);
        assert_eq!(tracks, vec![
            Track {
                video_id: "v1".into(),
                title: "Jazz Festival".into(),
                artists: "Straizo".into(),
                album: Some("Postcards From Molde".into()),
                duration: Some(Duration::from_secs(141)),
                thumbnail: None,
            },
            Track {
                video_id: "v2".into(),
                title: "No Album".into(),
                artists: "Someone".into(),
                album: None,
                duration: Some(Duration::from_secs(180)),
                thumbnail: None,
            },
        ]);
        assert_eq!(next.as_deref(), Some("NEXT"));
    }

    #[test]
    fn last_playlist_page_has_no_continuation() {
        let (tracks, next) = parse_playlist_page(&json!([playlist_row(Some("v"), "T", "A", "B", "1:00")]));
        assert_eq!(tracks.len(), 1);
        assert_eq!(next, None);
        assert_eq!(parse_playlist_page(&Value::Null), (vec![], None));
    }

    #[test]
    fn parses_library_grid_skipping_new_playlist_tile() {
        let tile = |title: &str, endpoint: Value, subtitle: &str| json!({ "musicTwoRowItemRenderer": {
            "title": { "runs": [{ "text": title }] },
            "subtitle": { "runs": [{ "text": subtitle }] },
            "navigationEndpoint": endpoint,
        }});
        let grid = json!({ "items": [
            tile("New playlist", json!({ "createPlaylistEndpoint": {} }), ""),
            tile("Liked Music", json!({ "browseEndpoint": { "browseId": "VLLM" } }), "Auto playlist"),
            tile("Chillax", json!({ "browseEndpoint": { "browseId": "VLPL1" } }), "Me • 48 tracks"),
        ]});
        assert_eq!(parse_grid_playlists(&grid), vec![
            Playlist { browse_id: "VLLM".into(), title: "Liked Music".into(), subtitle: "Auto playlist".into() },
            Playlist { browse_id: "VLPL1".into(), title: "Chillax".into(), subtitle: "Me • 48 tracks".into() },
        ]);
    }

    #[test]
    fn parses_radio_and_lyrics_tab() {
        let video = |id: &str, title: &str, byline: &[&str], length: &str| json!({
            "videoId": id,
            "title": { "runs": [{ "text": title }] },
            "longBylineText": { "runs": byline.iter().map(|t| json!({ "text": t })).collect::<Vec<_>>() },
            "lengthText": { "runs": [{ "text": length }] },
            "thumbnail": { "thumbnails": [{ "url": "small" }, { "url": "big" }] },
        });
        let response = json!({ "contents": { "singleColumnMusicWatchNextResultsRenderer": { "tabbedRenderer": {
            "watchNextTabbedResultsRenderer": { "tabs": [
                { "tabRenderer": { "content": { "musicQueueRenderer": { "content": { "playlistPanelRenderer": { "contents": [
                    { "playlistPanelVideoRenderer": video("a", "Song", &["Artist", " • ", "Album", " • ", "2007"], "5:19") },
                    { "playlistPanelVideoWrapperRenderer": { "primaryRenderer": {
                        "playlistPanelVideoRenderer": video("b", "Video", &["Band", " • ", "1M views"], "3:00") } } },
                    { "automixPreviewVideoRenderer": {} },
                ]}}}}}},
                { "tabRenderer": { "endpoint": { "browseEndpoint": { "browseId": "MPLYt_x" } } } },
                { "tabRenderer": { "endpoint": { "browseEndpoint": { "browseId": "MPTRt_x" } } } },
            ]}
        }}}});
        let next = parse_watch_next(&response);
        assert_eq!(next.lyrics_id.as_deref(), Some("MPLYt_x"));
        assert_eq!(next.radio, vec![
            Track {
                video_id: "a".into(), title: "Song".into(), artists: "Artist".into(), album: Some("Album".into()),
                duration: Some(Duration::from_secs(319)), thumbnail: Some("big".into()),
            },
            Track {
                video_id: "b".into(), title: "Video".into(), artists: "Band".into(), album: None,
                duration: Some(Duration::from_secs(180)), thumbnail: Some("big".into()),
            },
        ]);
    }

    #[test]
    fn year_is_not_mistaken_for_duration() {
        let tracks = parse_search(&response(vec![row("x", "T", &["Artist", " • ", "2019"])]));
        assert_eq!(tracks[0].duration, None);
        assert_eq!(tracks[0].album.as_deref(), Some("2019"));
    }
}
