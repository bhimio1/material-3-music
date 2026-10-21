//! What the app knows about songs, albums, artists and playlists, whether
//! they come from YouTube Music or from files on disk.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    pub name: String,
    /// Browse id (`UC…` artist, `MPREb_…` album), when linked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl Link {
    pub fn new(name: impl Into<String>, id: Option<String>) -> Self {
        Self {
            name: name.into(),
            id,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Track {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub video_id: Option<String>,
    /// A local file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    pub title: String,
    #[serde(default)]
    pub artists: Vec<Link>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub album: Option<Link>,
    /// Seconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration: Option<u32>,
    /// Image URL (`https://…` or `file://…`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// Identifies this entry inside a playlist (needed to remove it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set_video_id: Option<String>,
    /// A music video rather than an audio track.
    #[serde(default)]
    pub is_video: bool,
    /// Position on its album (local files).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub track_no: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub year: Option<u32>,
}

impl Track {
    pub fn artist_line(&self) -> String {
        if self.artists.is_empty() {
            return String::new();
        }
        self.artists
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>()
            .join(", ")
    }

    /// A stable key for "is this the same song" (queue highlighting etc.).
    pub fn key(&self) -> String {
        match (&self.video_id, &self.path) {
            (Some(id), _) => format!("yt:{id}"),
            (None, Some(p)) => format!("file:{}", p.display()),
            _ => format!("title:{}", self.title),
        }
    }

    pub fn watch_url(&self) -> Option<String> {
        self.video_id
            .as_ref()
            .map(|id| format!("https://music.youtube.com/watch?v={id}"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    Album,
    Single,
    Artist,
    Playlist,
    /// A podcast show (`MPSP…`); its tracks are episodes.
    Podcast,
}

/// An album, artist or playlist shown as a card or row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Collection {
    pub kind: Kind,
    /// Browse id: `MPREb_…`, `UC…`, `VL…`.
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumbnail: Option<String>,
    /// Playlist id for playing an album or playlist directly (`OLAK5uy_…`,
    /// `PL…`), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub play_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum Item {
    Track(Track),
    Collection(Collection),
}

/// A titled row of items (a home-page carousel, a search section, an
/// artist's "Albums").
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Shelf {
    pub title: String,
    pub items: Vec<Item>,
    /// Browse id of the "More" link, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub more: Option<String>,
}

/// An album, playlist or artist page.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Detail {
    pub id: String,
    pub title: String,
    /// "Album • Artist • 2021", owner, subscriber count…
    pub subtitle: String,
    pub description: String,
    pub thumbnail: Option<String>,
    pub tracks: Vec<Track>,
    /// Further sections (artist pages).
    pub shelves: Vec<Shelf>,
    /// Token for loading more tracks.
    pub continuation: Option<String>,
    /// Playlist id for radio / editing (`PL…`, `OLAK5uy_…`, `LM`).
    pub playlist_id: Option<String>,
    /// The signed-in user can add and remove songs.
    pub editable: bool,
    pub radio_id: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Lyrics {
    pub text: String,
    pub source: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Account {
    pub name: String,
    pub handle: String,
    pub photo: Option<String>,
}

/// `3:45` / `1:02:03` → seconds.
/// "3:45", "1:02:03", or podcast-style "1 hr 23 min" / "45 min" / "30 sec".
pub fn parse_duration(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    if !text.contains(':') {
        return parse_spelled_duration(text);
    }
    let mut total = 0u32;
    for part in text.split(':') {
        if part.is_empty() || part.len() > 2 || !part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        total = total * 60 + part.parse::<u32>().ok()?;
    }
    Some(total)
}

fn parse_spelled_duration(text: &str) -> Option<u32> {
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() || !words.len().is_multiple_of(2) {
        return None;
    }
    let mut total = 0u32;
    for pair in words.chunks(2) {
        let n: u32 = pair[0].parse().ok()?;
        total += n * match pair[1].trim_end_matches(',') {
            "hr" | "hrs" | "hour" | "hours" | "h" => 3600,
            "min" | "mins" | "minute" | "minutes" | "m" => 60,
            "sec" | "secs" | "second" | "seconds" | "s" => 1,
            _ => return None,
        };
    }
    Some(total)
}

pub fn format_duration(secs: f64) -> String {
    let secs = if secs.is_finite() && secs > 0.0 { secs as u64 } else { 0 };
    let (h, m, s) = (secs / 3600, (secs / 60) % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations() {
        assert_eq!(parse_duration("3:45"), Some(225));
        assert_eq!(parse_duration("1:02:03"), Some(3723));
        assert_eq!(parse_duration("Song"), None);
        assert_eq!(parse_duration("12 views"), None);
        assert_eq!(parse_duration("2021"), None);
        assert_eq!(parse_duration("1 hr 23 min"), Some(4980));
        assert_eq!(parse_duration("45 min"), Some(2700));
        assert_eq!(parse_duration("2M views"), None);
        assert_eq!(parse_duration("1d ago"), None);
        assert_eq!(format_duration(225.4), "3:45");
        assert_eq!(format_duration(3723.0), "1:02:03");
        assert_eq!(format_duration(f64::NAN), "0:00");
    }
}
