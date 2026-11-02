//! Parsing for YouTube Music's browse pages: Home, artists, and albums, plus
//! the generic rows and tiles they're built from.
//!
//! Pages are lists of shelves ("Quick picks", "Albums", …) holding either
//! tiles (`musicTwoRowItemRenderer`) or rows (`musicResponsiveListItemRenderer`).
//! Both link to what they are: a browse id with a page type (album, artist,
//! playlist) or a video id (a song).

use serde_json::Value;

use crate::api::{column_text, parse_duration, runs_text, thumbnail_url};
use crate::model::{Album, Artist, Item, Shelf, Track};

/// Labels some lists put before the artist, as in "Song • Olivia Dean".
const TYPE_LABELS: [&str; 7] = ["Song", "Video", "Album", "Single", "EP", "Episode", "Playlist"];

/// The first artist (`UC…`) and album (`MPREb_…`) browse ids anywhere in a
/// row or tile: in its text runs or its menu ("Go to album").
pub fn links(item: &Value) -> (Option<String>, Option<String>) {
    fn walk(value: &Value, artist: &mut Option<String>, album: &mut Option<String>) {
        match value {
            Value::Object(map) => {
                if let Some(Value::String(id)) = map.get("browseId") {
                    if artist.is_none() && id.starts_with("UC") {
                        *artist = Some(id.clone());
                    } else if album.is_none() && id.starts_with("MPREb") {
                        *album = Some(id.clone());
                    }
                }
                map.values().for_each(|v| walk(v, artist, album));
            }
            Value::Array(items) => items.iter().for_each(|v| walk(v, artist, album)),
            _ => {}
        }
    }
    let (mut artist, mut album) = (None, None);
    walk(item, &mut artist, &mut album);
    (artist, album)
}

/// The text of the run that links to the album, in any column.
fn album_title(item: &Value) -> Option<String> {
    item["flexColumns"].as_array()?.iter().find_map(|column| {
        column["musicResponsiveListItemFlexColumnRenderer"]["text"]["runs"]
            .as_array()?
            .iter()
            .find(|run| {
                run["navigationEndpoint"]["browseEndpoint"]["browseId"]
                    .as_str()
                    .is_some_and(|id| id.starts_with("MPREb"))
            })
            .and_then(|run| run["text"].as_str())
            .map(str::to_owned)
    })
}

/// The artists in a byline like "Olivia Dean • 2.1M plays" or
/// "Song • Sam Fender & Olivia Dean • 3:18".
fn byline_artists(byline: &str) -> String {
    let mut parts = byline.split(" • ");
    let first = parts.next().unwrap_or_default();
    let artists = if TYPE_LABELS.contains(&first) {
        parts.next().unwrap_or_default()
    } else {
        first
    };
    artists.to_owned()
}

/// A browse target from a navigation endpoint: its id, and what kind of page
/// it opens (from the page type, else the id's prefix).
fn browse_target(endpoint: &Value) -> Option<(String, &'static str)> {
    let browse = &endpoint["browseEndpoint"];
    let id = browse["browseId"].as_str()?;
    let page_type = browse["browseEndpointContextSupportedConfigs"]["browseEndpointContextMusicConfig"]["pageType"]
        .as_str()
        .unwrap_or_default();
    let kind = match page_type {
        "MUSIC_PAGE_TYPE_ALBUM" => "album",
        "MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL" => "artist",
        "MUSIC_PAGE_TYPE_PLAYLIST" => "playlist",
        _ if id.starts_with("MPREb") => "album",
        _ if id.starts_with("UC") => "artist",
        _ if id.starts_with("VL") => "playlist",
        _ => return None,
    };
    Some((id.to_owned(), kind))
}

fn browse_item(id: String, kind: &str, title: String, subtitle: String, thumbnail: Option<String>) -> Item {
    match kind {
        "album" => Item::Album {
            browse_id: id,
            title,
            subtitle,
            thumbnail,
        },
        "artist" => Item::Artist {
            browse_id: id,
            name: title,
            subtitle,
            thumbnail,
        },
        _ => Item::Playlist {
            browse_id: id,
            title,
            subtitle,
            thumbnail,
        },
    }
}

/// The largest image in a tile's thumbnail.
fn tile_thumbnail(tile: &Value) -> Option<String> {
    tile["thumbnailRenderer"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]
        .as_array()?
        .last()?["url"]
        .as_str()
        .map(str::to_owned)
}

/// A tile (`musicTwoRowItemRenderer`): an album, artist, playlist, or song.
pub fn parse_tile(tile: &Value) -> Option<Item> {
    let title = runs_text(&tile["title"]);
    let subtitle = runs_text(&tile["subtitle"]);
    let endpoint = &tile["navigationEndpoint"];
    if let Some((id, kind)) = browse_target(endpoint) {
        return Some(browse_item(id, kind, title, subtitle, tile_thumbnail(tile)));
    }
    let video_id = endpoint["watchEndpoint"]["videoId"].as_str()?;
    let (artist_id, album_id) = links(tile);
    let thumbnail = tile_thumbnail(tile);
    Some(Item::Song(Track {
        video_id: video_id.to_owned(),
        title,
        artists: byline_artists(&subtitle),
        thumbnail,
        artist_id,
        album_id,
        ..Track::default()
    }))
}

/// A row (`musicResponsiveListItemRenderer`): an album, artist, or playlist
/// when it links to a page, else a song.
pub fn parse_row(row: &Value) -> Option<Item> {
    let title = column_text(row, 0);
    if let Some((id, kind)) = browse_target(&row["navigationEndpoint"]) {
        return Some(browse_item(id, kind, title, column_text(row, 1), thumbnail_url(row)));
    }
    parse_song_row(row).map(Item::Song)
}

/// A song row in any of YouTube Music's layouts: title first, then a byline
/// with the artists, the album linked somewhere, and maybe a duration.
pub fn parse_song_row(row: &Value) -> Option<Track> {
    let video_id = row["playlistItemData"]["videoId"]
        .as_str()
        .or_else(|| {
            row["overlay"]["musicItemThumbnailOverlayRenderer"]["content"]["musicPlayButtonRenderer"]
                ["playNavigationEndpoint"]["watchEndpoint"]["videoId"]
                .as_str()
        })?
        .to_owned();
    let byline = column_text(row, 1);
    let duration_text = &row["fixedColumns"][0]["musicResponsiveListItemFixedColumnRenderer"]["text"];
    let duration =
        parse_duration(&runs_text(duration_text)).or_else(|| byline.split(" • ").last().and_then(parse_duration));
    let (artist_id, album_id) = links(row);
    Some(Track {
        video_id,
        title: column_text(row, 0),
        artists: byline_artists(&byline),
        album: album_title(row),
        duration,
        thumbnail: thumbnail_url(row),
        artist_id,
        album_id,
    })
}

/// Shelves from a section list: carousels of tiles or rows, and titled lists
/// of rows. Sections that aren't shelves (descriptions, headers) are skipped.
pub fn parse_shelves(sections: &Value) -> Vec<Shelf> {
    sections
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|section| {
            let (title, contents) = if let Some(carousel) = section.get("musicCarouselShelfRenderer") {
                let title = runs_text(&carousel["header"]["musicCarouselShelfBasicHeaderRenderer"]["title"]);
                (title, &carousel["contents"])
            } else {
                let shelf = section.get("musicShelfRenderer")?;
                (runs_text(&shelf["title"]), &shelf["contents"])
            };
            let items: Vec<Item> = contents
                .as_array()?
                .iter()
                .filter_map(|item| {
                    item.get("musicTwoRowItemRenderer")
                        .and_then(parse_tile)
                        .or_else(|| item.get("musicResponsiveListItemRenderer").and_then(parse_row))
                })
                .collect();
            (!items.is_empty()).then_some(Shelf { title, items })
        })
        .collect()
}

/// The shelves on a page of Home, and the token for the next page.
pub fn parse_home_page(section_list: &Value) -> (Vec<Shelf>, Option<String>) {
    let token = section_list["continuations"][0]["nextContinuationData"]["continuation"]
        .as_str()
        .map(str::to_owned);
    (parse_shelves(&section_list["contents"]), token)
}

pub fn parse_artist(response: &Value) -> Artist {
    let header = response["header"]
        .as_object()
        .and_then(|h| h.values().next())
        .cloned()
        .unwrap_or_default();
    let sections = &response["contents"]["singleColumnBrowseResultsRenderer"]["tabs"][0]["tabRenderer"]["content"]["sectionListRenderer"]
        ["contents"];
    let mut shelves = parse_shelves(sections);
    // "Top songs" shows five; its title links to the artist's full song list.
    if let Some(all_songs) = sections.as_array().into_iter().flatten().find_map(|s| {
        s["musicShelfRenderer"]["bottomEndpoint"]["browseEndpoint"]["browseId"]
            .as_str()
            .or_else(|| {
                s["musicShelfRenderer"]["title"]["runs"][0]["navigationEndpoint"]["browseEndpoint"]["browseId"].as_str()
            })
    }) && let Some(top) = shelves.first_mut()
        && top.items.iter().all(|item| matches!(item, Item::Song(_)))
    {
        top.items.push(Item::Playlist {
            browse_id: all_songs.to_owned(),
            title: "All songs".to_owned(),
            subtitle: String::new(),
            thumbnail: None,
        });
    }
    Artist {
        name: runs_text(&header["title"]),
        shelves,
    }
}

pub fn parse_album(response: &Value) -> Album {
    let two_column = &response["contents"]["twoColumnBrowseResultsRenderer"];
    let header = &two_column["tabs"][0]["tabRenderer"]["content"]["sectionListRenderer"]["contents"][0]["musicResponsiveHeaderRenderer"];
    let title = runs_text(&header["title"]);
    let artists = runs_text(&header["straplineTextOne"]);
    let (artist_id, _) = links(&header["straplineTextOne"]);
    let cover = header["thumbnail"]["musicThumbnailRenderer"]["thumbnail"]["thumbnails"]
        .as_array()
        .and_then(|t| t.last())
        .and_then(|t| t["url"].as_str())
        .map(str::to_owned);
    let rows = &two_column["secondaryContents"]["sectionListRenderer"]["contents"][0]["musicShelfRenderer"]["contents"];
    let tracks = rows
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|row| parse_song_row(&row["musicResponsiveListItemRenderer"]))
        .map(|track| Track {
            // Album rows leave out what the header already says.
            artists: if track.artists.is_empty() {
                artists.clone()
            } else {
                track.artists
            },
            album: Some(title.clone()),
            thumbnail: track.thumbnail.or_else(|| cover.clone()),
            artist_id: track.artist_id.or_else(|| artist_id.clone()),
            ..track
        })
        .collect();
    let details = [
        artists.as_str(),
        &runs_text(&header["subtitle"]),
        &runs_text(&header["secondSubtitle"]),
    ]
    .into_iter()
    .filter(|part| !part.is_empty())
    .collect::<Vec<_>>()
    .join(" · ");
    Album {
        title,
        subtitle: details,
        tracks,
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use serde_json::json;

    use super::*;

    fn browse(id: &str, page_type: &str) -> Value {
        json!({ "browseEndpoint": { "browseId": id, "browseEndpointContextSupportedConfigs": {
            "browseEndpointContextMusicConfig": { "pageType": page_type } } } })
    }

    fn run(text: &str, link: Option<&str>) -> Value {
        match link {
            Some(id) => json!({ "text": text, "navigationEndpoint": { "browseEndpoint": { "browseId": id } } }),
            None => json!({ "text": text }),
        }
    }

    fn column(runs: Vec<Value>) -> Value {
        json!({ "musicResponsiveListItemFlexColumnRenderer": { "text": { "runs": runs } } })
    }

    /// A Quick picks / Top songs row: title, "artist • plays", album.
    fn song_row(video_id: &str, title: &str) -> Value {
        json!({
            "playlistItemData": { "videoId": video_id },
            "flexColumns": [
                column(vec![run(title, None)]),
                column(vec![run("LANY", Some("UCartist")), run(" • ", None), run("9.7M plays", None)]),
                column(vec![run("Soft", Some("MPREb_album"))]),
            ],
        })
    }

    #[test]
    fn song_rows_pick_up_artist_and_album_links() {
        let track = parse_song_row(&song_row("v1", "Stuck")).unwrap();
        assert_eq!(track.title, "Stuck");
        assert_eq!(track.artists, "LANY");
        assert_eq!(track.album.as_deref(), Some("Soft"));
        assert_eq!(track.artist_id.as_deref(), Some("UCartist"));
        assert_eq!(track.album_id.as_deref(), Some("MPREb_album"));
        assert_eq!(track.duration, None);
    }

    #[test]
    fn byline_skips_type_labels_and_reads_a_trailing_duration() {
        assert_eq!(
            byline_artists("Song • Sam Fender & Olivia Dean • 3:18"),
            "Sam Fender & Olivia Dean"
        );
        assert_eq!(byline_artists("Olivia Dean • 259M plays"), "Olivia Dean");
        let mut row = song_row("v", "T");
        row["flexColumns"][1] = column(vec![run("Olivia Dean • The Hardest Part • 3:18", None)]);
        assert_eq!(parse_song_row(&row).unwrap().duration, Some(Duration::from_secs(198)));
    }

    #[test]
    fn tiles_become_the_right_kind_of_item() {
        let tile = |title: &str, endpoint: Value, subtitle: &str| {
            json!({ "title": { "runs": [run(title, None)] }, "subtitle": { "runs": [run(subtitle, None)] },
                    "navigationEndpoint": endpoint })
        };
        let album = parse_tile(&tile(
            "Soft",
            browse("MPREb_x", "MUSIC_PAGE_TYPE_ALBUM"),
            "Album • 2024",
        ))
        .unwrap();
        assert_eq!(
            album,
            Item::Album {
                browse_id: "MPREb_x".into(),
                title: "Soft".into(),
                subtitle: "Album • 2024".into(),
                thumbnail: None,
            }
        );
        let artist = parse_tile(&tile(
            "Olivia Dean",
            browse("UCx", "MUSIC_PAGE_TYPE_ARTIST"),
            "1M subscribers",
        ))
        .unwrap();
        assert!(matches!(artist, Item::Artist { ref name, .. } if name == "Olivia Dean"));
        let playlist = parse_tile(&tile("Chill", browse("VLRDx", "MUSIC_PAGE_TYPE_PLAYLIST"), "")).unwrap();
        assert!(matches!(playlist, Item::Playlist { ref browse_id, .. } if browse_id == "VLRDx"));
        let song = parse_tile(&tile(
            "begged",
            json!({ "watchEndpoint": { "videoId": "abc" } }),
            "Olivia Rodrigo • 17M plays",
        ))
        .unwrap();
        let Item::Song(track) = song else {
            panic!("expected a song")
        };
        assert_eq!(
            (track.video_id.as_str(), track.artists.as_str()),
            ("abc", "Olivia Rodrigo")
        );
    }

    #[test]
    fn shelves_keep_titles_and_skip_non_shelf_sections() {
        let sections = json!([
            { "musicCarouselShelfRenderer": {
                "header": { "musicCarouselShelfBasicHeaderRenderer": { "title": { "runs": [run("Quick picks", None)] } } },
                "contents": [{ "musicResponsiveListItemRenderer": song_row("v1", "Stuck") }] } },
            { "musicDescriptionShelfRenderer": { "description": {} } },
            { "musicShelfRenderer": { "title": { "runs": [run("Empty", None)] }, "contents": [] } },
        ]);
        let shelves = parse_shelves(&sections);
        assert_eq!(shelves.len(), 1);
        assert_eq!(shelves[0].title, "Quick picks");
        assert!(matches!(&shelves[0].items[0], Item::Song(t) if t.title == "Stuck"));
    }

    #[test]
    fn album_rows_inherit_artist_album_and_cover_from_the_header() {
        let mut row = song_row("v1", "The Art of Loving (Intro)");
        row["flexColumns"][1] = column(vec![]);
        row["flexColumns"][2] = column(vec![run("3.7M plays", None)]);
        row["fixedColumns"] =
            json!([{ "musicResponsiveListItemFixedColumnRenderer": { "text": { "runs": [run("0:41", None)] } } }]);
        let response = json!({ "contents": { "twoColumnBrowseResultsRenderer": {
            "tabs": [{ "tabRenderer": { "content": { "sectionListRenderer": { "contents": [{ "musicResponsiveHeaderRenderer": {
                "title": { "runs": [run("The Art of Loving", None)] },
                "straplineTextOne": { "runs": [run("Olivia Dean", Some("UCdean"))] },
                "subtitle": { "runs": [run("Album • 2025", None)] },
                "thumbnail": { "musicThumbnailRenderer": { "thumbnail": { "thumbnails": [{ "url": "small" }, { "url": "cover" }] } } },
            } }] } } } }],
            "secondaryContents": { "sectionListRenderer": { "contents": [{ "musicShelfRenderer": { "contents": [
                { "musicResponsiveListItemRenderer": row } ] } }] } },
        } } });
        let album = parse_album(&response);
        assert_eq!(album.title, "The Art of Loving");
        assert_eq!(album.subtitle, "Olivia Dean · Album • 2025");
        let track = &album.tracks[0];
        assert_eq!(track.artists, "Olivia Dean");
        assert_eq!(track.album.as_deref(), Some("The Art of Loving"));
        assert_eq!(track.thumbnail.as_deref(), Some("cover"));
        assert_eq!(track.duration, Some(Duration::from_secs(41)));
    }

    #[test]
    fn artist_top_songs_get_an_all_songs_link() {
        let response = json!({
            "header": { "musicImmersiveHeaderRenderer": { "title": { "runs": [run("Olivia Dean", None)] } } },
            "contents": { "singleColumnBrowseResultsRenderer": { "tabs": [{ "tabRenderer": { "content": {
                "sectionListRenderer": { "contents": [
                    { "musicShelfRenderer": { "title": { "runs": [run("Top songs", None)] },
                        "bottomEndpoint": { "browseEndpoint": { "browseId": "VLOLAK5" } },
                        "contents": [{ "musicResponsiveListItemRenderer": song_row("v1", "So Easy") }] } },
                ] } } } }] } },
        });
        let artist = parse_artist(&response);
        assert_eq!(artist.name, "Olivia Dean");
        let items = &artist.shelves[0].items;
        assert_eq!(items.len(), 2);
        assert!(
            matches!(&items[1], Item::Playlist { browse_id, title, .. } if browse_id == "VLOLAK5" && title == "All songs")
        );
    }
}
