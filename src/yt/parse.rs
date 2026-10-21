//! Turning InnerTube (YouTube Music web API) responses into the app's model.
//!
//! The responses are deeply nested renderer trees that YouTube reshuffles
//! often, so the parsers look for renderers anywhere in the tree rather than
//! at fixed paths, and skip anything they don't understand.

use super::json::{arr_at, at, find, find_all, find_any, runs, str_at, text, text_at, thumbnail};
use super::model::{parse_duration, Account, Collection, Detail, Item, Kind, Link, Lyrics, Shelf, Track};
use serde_json::Value;

const PAGE_TYPE: &str = "browseEndpointContextSupportedConfigs/browseEndpointContextMusicConfig/pageType";

/// Words YouTube puts in front of the artist in subtitles.
const TYPE_WORDS: &[&str] = &[
    "Song", "Video", "Album", "EP", "Single", "Playlist", "Artist", "Episode", "Podcast", "Profile",
];

/// (browseId, pageType) of a run or renderer's browse endpoint.
fn browse_target(node: &Value) -> Option<(&str, &str)> {
    let be = at(node, "navigationEndpoint/browseEndpoint")?;
    let id = str_at(be, "browseId")?;
    Some((id, str_at(be, PAGE_TYPE).unwrap_or("")))
}

fn kind_for(page_type: &str, subtitle: &str) -> Option<Kind> {
    match page_type {
        "MUSIC_PAGE_TYPE_ALBUM" | "MUSIC_PAGE_TYPE_AUDIOBOOK" => {
            let first = subtitle.split(" • ").next().unwrap_or("");
            Some(if first == "Single" || first == "EP" { Kind::Single } else { Kind::Album })
        }
        "MUSIC_PAGE_TYPE_ARTIST" | "MUSIC_PAGE_TYPE_USER_CHANNEL" | "MUSIC_PAGE_TYPE_LIBRARY_ARTIST" => {
            Some(Kind::Artist)
        }
        "MUSIC_PAGE_TYPE_PLAYLIST" => Some(Kind::Playlist),
        "MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE" => Some(Kind::Podcast),
        _ => None,
    }
}

/// Library artist entries point at `MPLAUC…`; the artist page is `UC…`.
fn artist_id(id: &str) -> String {
    id.strip_prefix("MPLA").unwrap_or(id).to_string()
}

fn play_playlist_id(node: &Value) -> Option<String> {
    find(node, "watchPlaylistEndpoint")
        .and_then(|e| str_at(e, "playlistId"))
        .map(str::to_string)
}

/// Fill artist/album/duration from subtitle-style runs
/// (`Song • Artist • Album • 3:45`).
fn read_meta_runs(runs: &[Value], track: &mut Track, loose: &mut Vec<String>) {
    for run in runs {
        let t = run.get("text").and_then(Value::as_str).unwrap_or("").trim();
        if t.is_empty() || t == "•" || t == "&" || t == "," {
            continue;
        }
        match browse_target(run) {
            Some((id, pt)) if kind_for(pt, "") == Some(Kind::Artist) => {
                track.artists.push(Link::new(t, Some(artist_id(id))));
            }
            Some((id, "MUSIC_PAGE_TYPE_ALBUM")) => track.album = Some(Link::new(t, Some(id.to_string()))),
            // An episode's show stands in for the artist.
            Some((id, "MUSIC_PAGE_TYPE_PODCAST_SHOW_DETAIL_PAGE")) => {
                track.artists.push(Link::new(t, Some(id.to_string())));
            }
            _ => {
                if let Some(d) = parse_duration(t) {
                    track.duration = Some(d);
                } else {
                    loose.push(t.to_string());
                }
            }
        }
    }
}

/// Unlinked artists: the first loose text that isn't a type word, a view
/// count or a year.
fn guess_artist(loose: &[String]) -> Option<String> {
    loose
        .iter()
        .flat_map(|t| t.split(" • "))
        .map(str::trim)
        .find(|t| {
            !t.is_empty()
                && !TYPE_WORDS.contains(t)
                && !t.ends_with("views")
                && !t.ends_with("plays")
                && !t.ends_with(" ago")
                && !looks_like_date(t)
                && !(t.len() == 4 && t.chars().all(|c| c.is_ascii_digit()))
        })
        .map(str::to_string)
}

/// "Jul 21", "Jun 20, 2025": episode dates in subtitles.
fn looks_like_date(t: &str) -> bool {
    const MONTHS: &[&str] = &["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    let mut words = t.split_whitespace();
    let (Some(month), Some(day)) = (words.next(), words.next()) else {
        return false;
    };
    MONTHS.contains(&month) && day.trim_end_matches(',').parse::<u8>().is_ok()
}

fn music_video_type(node: &Value) -> Option<&str> {
    find(node, "musicVideoType").and_then(Value::as_str)
}

/// `musicResponsiveListItemRenderer`: rows in search results, playlists,
/// albums, library lists.
pub fn list_item(r: &Value) -> Option<Item> {
    let cols = arr_at(r, "flexColumns");
    let col = |i: usize| cols.get(i).and_then(|c| at(c, "musicResponsiveListItemFlexColumnRenderer/text"));
    let title = col(0).map(text).unwrap_or_default();
    if title.is_empty() {
        return None;
    }

    if let Some((id, pt)) = browse_target(r) {
        let subtitle = (1..cols.len())
            .filter_map(|i| col(i).map(text))
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" • ");
        let kind = kind_for(pt, &subtitle)?;
        let id = if kind == Kind::Artist { artist_id(id) } else { id.to_string() };
        return Some(Item::Collection(Collection {
            kind,
            id,
            title,
            subtitle,
            thumbnail: thumbnail(at(r, "thumbnail").unwrap_or(&Value::Null)),
            play_id: play_playlist_id(at(r, "overlay").unwrap_or(&Value::Null)),
        }));
    }

    let video_id = str_at(r, "playlistItemData/videoId")
        .or_else(|| {
            runs(col(0))
                .iter()
                .find_map(|run| str_at(run, "navigationEndpoint/watchEndpoint/videoId"))
        })
        .or_else(|| find(at(r, "overlay")?, "watchEndpoint").and_then(|w| str_at(w, "videoId")))?
        .to_string();
    let mut track = Track {
        video_id: Some(video_id),
        title,
        thumbnail: thumbnail(at(r, "thumbnail").unwrap_or(&Value::Null)),
        set_video_id: str_at(r, "playlistItemData/playlistSetVideoId").map(str::to_string),
        is_video: music_video_type(r).is_some_and(|t| t != "MUSIC_VIDEO_TYPE_ATV"),
        ..Default::default()
    };
    let mut loose = Vec::new();
    for i in 1..cols.len() {
        read_meta_runs(runs(col(i)), &mut track, &mut loose);
    }
    for fixed in arr_at(r, "fixedColumns") {
        let t = text_at(fixed, "musicResponsiveListItemFixedColumnRenderer/text");
        if let Some(d) = parse_duration(&t) {
            track.duration = Some(d);
        }
    }
    if track.artists.is_empty() {
        if let Some(name) = guess_artist(&loose) {
            track.artists.push(Link::new(name, None));
        }
    }
    Some(Item::Track(track))
}

/// `musicTwoRowItemRenderer`: cards in carousels and library grids.
pub fn two_row(r: &Value) -> Option<Item> {
    let title = text_at(r, "title");
    if title.is_empty() {
        return None;
    }
    let subtitle = text_at(r, "subtitle");
    let thumb = thumbnail(at(r, "thumbnailRenderer").unwrap_or(r));
    if let Some((id, pt)) = browse_target(r) {
        let kind = kind_for(pt, &subtitle)?;
        let id = if kind == Kind::Artist { artist_id(id) } else { id.to_string() };
        return Some(Item::Collection(Collection {
            kind,
            id,
            title,
            subtitle,
            thumbnail: thumb,
            play_id: play_playlist_id(at(r, "thumbnailOverlay").unwrap_or(&Value::Null)),
        }));
    }
    let video_id = str_at(r, "navigationEndpoint/watchEndpoint/videoId")?.to_string();
    let mut track = Track {
        video_id: Some(video_id),
        title,
        thumbnail: thumb,
        is_video: music_video_type(r).is_some_and(|t| t != "MUSIC_VIDEO_TYPE_ATV"),
        ..Default::default()
    };
    let mut loose = Vec::new();
    read_meta_runs(runs(at(r, "subtitle")), &mut track, &mut loose);
    if track.artists.is_empty() {
        if let Some(name) = guess_artist(&loose) {
            track.artists.push(Link::new(name, None));
        }
    }
    Some(Item::Track(track))
}

/// `playlistPanelVideoRenderer`: entries of a watch queue / radio.
pub fn panel_video(r: &Value) -> Option<Track> {
    let video_id = str_at(r, "videoId")
        .or_else(|| str_at(r, "navigationEndpoint/watchEndpoint/videoId"))?
        .to_string();
    let mut track = Track {
        video_id: Some(video_id),
        title: text_at(r, "title"),
        thumbnail: thumbnail(at(r, "thumbnail").unwrap_or(&Value::Null)),
        duration: parse_duration(&text_at(r, "lengthText")),
        is_video: music_video_type(r).is_some_and(|t| t != "MUSIC_VIDEO_TYPE_ATV"),
        ..Default::default()
    };
    let mut loose = Vec::new();
    read_meta_runs(runs(at(r, "longBylineText")), &mut track, &mut loose);
    if track.artists.is_empty() {
        if let Some(name) = guess_artist(&loose) {
            track.artists.push(Link::new(name, None));
        }
    }
    Some(track)
}

/// `musicMultiRowListItemRenderer`: podcast episodes on a show page.
pub fn episode(r: &Value) -> Option<Track> {
    let video_id = str_at(r, "onTap/watchEndpoint/videoId")
        .or_else(|| find(at(r, "overlay")?, "watchEndpoint").and_then(|w| str_at(w, "videoId")))?
        .to_string();
    let title = text_at(r, "title");
    if title.is_empty() {
        return None;
    }
    let duration = at(r, "playbackProgress/musicPlaybackProgressRenderer/durationText")
        .map(text)
        .and_then(|t| parse_duration(t.trim_start_matches(|c: char| c == '•' || c.is_whitespace())));
    Some(Track {
        video_id: Some(video_id),
        title,
        thumbnail: thumbnail(at(r, "thumbnail").unwrap_or(&Value::Null)),
        duration,
        is_video: false,
        ..Default::default()
    })
}

fn item_of(c: &Value) -> Option<Item> {
    if let Some(r) = c.get("musicResponsiveListItemRenderer") {
        list_item(r)
    } else if let Some(r) = c.get("musicTwoRowItemRenderer") {
        two_row(r)
    } else if let Some(r) = c.get("musicMultiRowListItemRenderer") {
        episode(r).map(Item::Track)
    } else {
        None
    }
}

fn items_of(contents: &[Value]) -> Vec<Item> {
    contents.iter().filter_map(item_of).collect()
}

/// Search results now come as one `itemSectionRenderer` per result, with
/// no headings. The section title comes from the result's type word
/// ("Song • Artist • …"), so results can be grouped like the old shelves.
fn result_group(c: &Value) -> &'static str {
    let first = c
        .get("musicResponsiveListItemRenderer")
        .and_then(|r| arr_at(r, "flexColumns").get(1))
        .and_then(|col| runs(at(col, "musicResponsiveListItemFlexColumnRenderer/text")).first())
        .and_then(|run| run.get("text"))
        .and_then(Value::as_str)
        .unwrap_or("");
    match first {
        "Song" => "Songs",
        "Video" => "Videos",
        "Album" | "EP" | "Single" => "Albums",
        "Artist" => "Artists",
        "Playlist" => "Playlists",
        "Episode" => "Episodes",
        "Podcast" => "Podcasts",
        "Profile" => "Profiles",
        _ => "Results",
    }
}

fn top_result(card: &Value) -> Vec<Item> {
    let mut items = Vec::new();
    let title_run = at(card, "title/runs/0").unwrap_or(&Value::Null);
    let title = text_at(card, "title");
    let subtitle = text_at(card, "subtitle");
    let thumb = thumbnail(at(card, "thumbnail").unwrap_or(&Value::Null));
    if let Some((id, pt)) = browse_target(title_run) {
        if let Some(kind) = kind_for(pt, &subtitle) {
            items.push(Item::Collection(Collection {
                kind,
                id: if kind == Kind::Artist { artist_id(id) } else { id.to_string() },
                title: title.clone(),
                subtitle: subtitle.clone(),
                thumbnail: thumb.clone(),
                play_id: None,
            }));
        }
    } else if let Some(vid) = str_at(title_run, "navigationEndpoint/watchEndpoint/videoId") {
        let mut track = Track {
            video_id: Some(vid.to_string()),
            title,
            thumbnail: thumb,
            is_video: music_video_type(title_run).is_some_and(|t| t != "MUSIC_VIDEO_TYPE_ATV"),
            ..Default::default()
        };
        let mut loose = Vec::new();
        read_meta_runs(runs(at(card, "subtitle")), &mut track, &mut loose);
        if track.artists.is_empty() {
            if let Some(name) = guess_artist(&loose) {
                track.artists.push(Link::new(name, None));
            }
        }
        items.push(Item::Track(track));
    }
    items.extend(items_of(arr_at(card, "contents")));
    items
}

/// Every shelf (carousel, list, grid, top-result card) in document order.
pub fn shelves(resp: &Value) -> Vec<Shelf> {
    let mut out = Vec::new();
    walk_shelves(resp, &mut out);
    out.retain(|s| !s.items.is_empty());
    for s in &mut out {
        if s.more.as_deref() == Some(GROUPED) {
            s.more = None;
        }
    }
    out
}

/// A flat search result section: one row, grouped by its type into the
/// shelf with that title (created where the type first appears).
fn push_grouped(out: &mut Vec<Shelf>, title: &str, item: Item) {
    const GROUPS: &[&str] = &["Songs", "Videos", "Albums", "Artists", "Playlists", "Episodes", "Podcasts", "Profiles", "Results"];
    debug_assert!(GROUPS.contains(&title));
    // Only merge into shelves made the same way (titled shelves from real
    // headings are left alone).
    if let Some(s) = out.iter_mut().rev().find(|s| s.title == title && s.more.as_deref() == Some(GROUPED)) {
        s.items.push(item);
    } else {
        out.push(Shelf {
            title: title.to_string(),
            items: vec![item],
            more: Some(GROUPED.to_string()),
        });
    }
}

/// Marker in `Shelf::more` for shelves built by grouping flat results;
/// cleared before returning.
const GROUPED: &str = "\u{0}grouped";

fn walk_shelves(value: &Value, out: &mut Vec<Shelf>) {
    match value {
        Value::Object(map) => {
            for (key, v) in map {
                match key.as_str() {
                    "musicCarouselShelfRenderer" | "musicImmersiveCarouselShelfRenderer" => {
                        let header = at(v, "header").unwrap_or(&Value::Null);
                        let title = find(header, "title").map(text).unwrap_or_default();
                        let more = find(header, "moreContentButton")
                            .and_then(|b| find(b, "browseEndpoint"))
                            .or_else(|| find(header, "browseEndpoint"))
                            .and_then(|e| str_at(e, "browseId"))
                            .map(str::to_string);
                        out.push(Shelf {
                            title,
                            items: items_of(arr_at(v, "contents")),
                            more,
                        });
                    }
                    "musicShelfRenderer" | "musicPlaylistShelfRenderer" => out.push(Shelf {
                        title: text_at(v, "title"),
                        items: items_of(arr_at(v, "contents")),
                        more: str_at(v, "bottomEndpoint/browseEndpoint/browseId").map(str::to_string),
                    }),
                    "gridRenderer" => out.push(Shelf {
                        title: at(v, "header").and_then(|h| find(h, "title")).map(text).unwrap_or_default(),
                        items: items_of(arr_at(v, "items")),
                        more: None,
                    }),
                    "musicCardShelfRenderer" => out.push(Shelf {
                        title: "Top result".into(),
                        items: top_result(v),
                        more: None,
                    }),
                    "itemSectionRenderer"
                        if arr_at(v, "contents").iter().any(|c| c.get("musicResponsiveListItemRenderer").is_some()) =>
                    {
                        for c in arr_at(v, "contents") {
                            if let Some(item) = item_of(c) {
                                push_grouped(out, result_group(c), item);
                            }
                        }
                    }
                    _ => walk_shelves(v, out),
                }
            }
        }
        Value::Array(items) => {
            for v in items {
                walk_shelves(v, out);
            }
        }
        _ => {}
    }
}

/// Search suggestions (`music/get_search_suggestions`).
pub fn suggestions(resp: &Value) -> Vec<String> {
    let mut out: Vec<String> = find_any(resp, &["searchSuggestionRenderer", "historySuggestionRenderer"])
        .into_iter()
        .map(|s| text_at(s, "suggestion"))
        .filter(|s| !s.is_empty())
        .collect();
    out.dedup();
    out
}

/// Search filter chips ("Songs", "Videos", …) with the `params` that
/// select them. Which filters exist depends on the query and region, so
/// they're read from the response rather than hardcoded.
pub fn search_filters(resp: &Value) -> Vec<(String, String)> {
    find_all(resp, "chipCloudChipRenderer")
        .into_iter()
        .filter_map(|c| {
            let label = text_at(c, "text");
            let params = str_at(c, "navigationEndpoint/searchEndpoint/params")?;
            (!label.is_empty()).then(|| (label, percent_decode(params)))
        })
        .collect()
}

/// `params` come URL-encoded in chip endpoints (`…%3D%3D`).
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(b) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Continuation token for loading more tracks of a list. Only the list's
/// own continuation counts: pages also carry a "more sections" token
/// (related playlists etc.) that yields no tracks.
pub fn continuation(resp: &Value) -> Option<String> {
    let token = |node: &Value| {
        find(node, "continuationItemRenderer")
            .and_then(|c| str_at(c, "continuationEndpoint/continuationCommand/token"))
            .or_else(|| find(node, "nextContinuationData").and_then(|c| str_at(c, "continuation")))
            .map(str::to_string)
    };
    // A continuation page: its appended items carry the next token.
    if let Some(actions) = resp.get("onResponseReceivedActions") {
        return token(actions);
    }
    if let Some(c) = resp.get("continuationContents") {
        return ["musicPlaylistShelfContinuation", "musicShelfContinuation"]
            .iter()
            .find_map(|k| c.get(*k))
            .and_then(token);
    }
    ["musicPlaylistShelfRenderer", "musicShelfRenderer"]
        .iter()
        .find_map(|k| find(resp, k))
        .and_then(token)
}

/// All tracks in a response (playlist pages and their continuations).
pub fn tracks(resp: &Value) -> Vec<Track> {
    find_any(resp, &["musicResponsiveListItemRenderer", "musicMultiRowListItemRenderer"])
        .into_iter()
        .filter_map(|r| match list_item(r) {
            Some(Item::Track(t)) => Some(t),
            Some(_) => None,
            // Podcast episodes (multi-row items have no flex columns).
            None => episode(r),
        })
        .collect()
}

/// All items (tracks and collections) in a response, in order. Used for
/// library lists, which come as grids or list shelves.
pub fn items(resp: &Value) -> Vec<Item> {
    shelves(resp).into_iter().flat_map(|s| s.items).collect()
}

const HEADERS: &[&str] = &[
    "musicResponsiveHeaderRenderer",
    "musicDetailHeaderRenderer",
    "musicImmersiveHeaderRenderer",
    "musicVisualHeaderRenderer",
];

/// An album, playlist or artist page (`browse` with that id).
pub fn detail(resp: &Value, id: &str) -> Detail {
    let header = HEADERS.iter().find_map(|k| find(resp, k)).unwrap_or(&Value::Null);
    let title = text_at(header, "title");
    let subtitle = ["subtitle", "straplineTextOne", "secondSubtitle", "monthlyListenerCount"]
        .iter()
        .map(|k| text_at(header, k))
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" • ");
    let description = at(header, "description")
        .map(|d| match find(d, "description") {
            Some(inner) => text(inner),
            None => text(d),
        })
        .unwrap_or_default();
    let thumb = thumbnail(header);
    let is_artist = id.starts_with("UC") || id.starts_with("MPLAUC");
    let mut detail = Detail {
        id: id.to_string(),
        title,
        subtitle,
        description,
        thumbnail: thumb.clone(),
        continuation: continuation(resp),
        editable: find(resp, "musicEditablePlaylistDetailHeaderRenderer").is_some(),
        radio_id: find(header, "startRadioButton").and_then(play_playlist_id),
        ..Default::default()
    };

    let all = shelves(resp);
    if is_artist {
        detail.shelves = all;
        if let Some(first) = detail.shelves.first() {
            if first.items.iter().all(|i| matches!(i, Item::Track(_))) {
                detail.tracks = first
                    .items
                    .iter()
                    .filter_map(|i| match i {
                        Item::Track(t) => Some(t.clone()),
                        _ => None,
                    })
                    .collect();
            }
        }
        detail.playlist_id = detail.radio_id.clone();
        return detail;
    }

    let mut shelf_iter = all.into_iter();
    // The page's own tracks are the first list shelf; carousels after it
    // ("Other versions", "You might also like") become shelves.
    let mut tracks = Vec::new();
    for shelf in shelf_iter.by_ref() {
        let shelf_tracks: Vec<Track> = shelf
            .items
            .iter()
            .filter_map(|i| match i {
                Item::Track(t) => Some(t.clone()),
                _ => None,
            })
            .collect();
        if !shelf_tracks.is_empty() {
            tracks = shelf_tracks;
            break;
        }
        detail.shelves.push(shelf);
    }
    detail.shelves.extend(shelf_iter);

    // A podcast show's episodes: the show stands in for the artist.
    if id.starts_with("MPSP") {
        for t in &mut tracks {
            if t.artists.is_empty() {
                t.artists.push(Link::new(detail.title.clone(), Some(id.to_string())));
            }
        }
    }

    let is_album = id.starts_with("MPREb_");
    if is_album {
        let artist = runs(at(header, "straplineTextOne"))
            .iter()
            .find_map(|r| {
                let (aid, _) = browse_target(r)?;
                Some(Link::new(r.get("text")?.as_str()?, Some(artist_id(aid))))
            })
            .or_else(|| {
                let name = text_at(header, "straplineTextOne");
                (!name.is_empty()).then(|| Link::new(name, None))
            });
        for t in &mut tracks {
            if t.artists.is_empty() {
                t.artists.extend(artist.clone());
            }
            if t.thumbnail.is_none() {
                t.thumbnail = thumb.clone();
            }
            if t.album.is_none() {
                t.album = Some(Link::new(detail.title.clone(), Some(id.to_string())));
            }
        }
    }
    detail.tracks = tracks;
    detail.playlist_id = if let Some(pl) = id.strip_prefix("VL") {
        Some(pl.to_string())
    } else {
        play_playlist_id(header).or_else(|| play_playlist_id(resp))
    };
    detail
}

/// Tracks of a watch queue (`next`), and the browse id of its lyrics tab.
pub fn next(resp: &Value) -> (Vec<Track>, Option<String>) {
    let tracks = find_all(resp, "playlistPanelVideoRenderer")
        .into_iter()
        .filter_map(panel_video)
        .collect();
    let lyrics = find_all(resp, "browseEndpoint")
        .into_iter()
        .filter_map(|e| str_at(e, "browseId"))
        .find(|id| id.starts_with("MPLYt"))
        .map(str::to_string);
    (tracks, lyrics)
}

pub fn lyrics(resp: &Value) -> Option<Lyrics> {
    let shelf = find(resp, "musicDescriptionShelfRenderer")?;
    // Real lyrics come with Windows line endings.
    let text = text_at(shelf, "description").replace("\r\n", "\n");
    if text.trim().is_empty() {
        return None;
    }
    Some(Lyrics {
        text,
        source: text_at(shelf, "footer"),
    })
}

pub fn account(resp: &Value) -> Option<Account> {
    let h = find(resp, "activeAccountHeaderRenderer")?;
    Some(Account {
        name: text_at(h, "accountName"),
        handle: text_at(h, "channelHandle"),
        photo: thumbnail(at(h, "accountPhoto").unwrap_or(&Value::Null)),
    })
}

/// Playlist id returned by `playlist/create`.
pub fn created_playlist(resp: &Value) -> Option<String> {
    str_at(resp, "playlistId").map(str::to_string)
}

#[cfg(test)]
#[path = "parse_tests.rs"]
mod tests;
