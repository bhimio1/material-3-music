//! YouTube Music's web API (InnerTube, client `WEB_REMIX`).
//!
//! Every call blocks, so the UI only calls these from background tasks.
//! Setting `M3_MUSIC_FIXTURES=<dir>` answers requests from the JSON files in
//! that directory instead of the network (development and screenshots).

pub mod auth;
pub mod json;
pub mod model;
pub mod parse;

use anyhow::{anyhow, Context, Result};
use auth::{Auth, ORIGIN};
use model::{Account, Detail, Item, Lyrics, Shelf, Track};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const API: &str = "https://music.youtube.com/youtubei/v1";
const USER_AGENT: &str =
    "Mozilla/5.0 (X11; Linux x86_64; rv:131.0) Gecko/20100101 Firefox/131.0";

/// Search result filters (ytmusicapi's `params` values).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Filter {
    Songs,
    Videos,
    Albums,
    Artists,
    Playlists,
}

impl Filter {
    pub const ALL: [Filter; 5] = [Filter::Songs, Filter::Videos, Filter::Albums, Filter::Artists, Filter::Playlists];

    pub fn label(self) -> &'static str {
        match self {
            Filter::Songs => "Songs",
            Filter::Videos => "Videos",
            Filter::Albums => "Albums",
            Filter::Artists => "Artists",
            Filter::Playlists => "Playlists",
        }
    }

    fn params(self) -> &'static str {
        match self {
            Filter::Songs => "EgWKAQIIAWoMEA4QChADEAQQCRAF",
            Filter::Videos => "EgWKAQIQAWoMEA4QChADEAQQCRAF",
            Filter::Albums => "EgWKAQIYAWoMEA4QChADEAQQCRAF",
            Filter::Artists => "EgWKAQIgAWoMEA4QChADEAQQCRAF",
            Filter::Playlists => "EgWKAQIoAWoMEA4QChADEAQQCRAF",
        }
    }
}

/// The library as synced from the account.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct LibraryData {
    pub playlists: Vec<Item>,
    pub liked: Vec<Track>,
    pub albums: Vec<Item>,
    pub artists: Vec<Item>,
    pub history: Vec<Track>,
    pub account: Option<Account>,
    /// Unix seconds.
    pub synced_at: i64,
    /// Sections that failed, with the error.
    #[serde(default)]
    pub errors: Vec<String>,
}

#[derive(Clone)]
pub struct Client {
    agent: ureq::Agent,
    auth: Arc<Mutex<Option<Auth>>>,
    fixtures: Option<PathBuf>,
}

fn client_version() -> String {
    format!("1.{}.01.00", chrono::Utc::now().format("%Y%m%d"))
}

fn only_tracks(items: Vec<Item>) -> Vec<Track> {
    items
        .into_iter()
        .filter_map(|i| match i {
            Item::Track(t) => Some(t),
            _ => None,
        })
        .collect()
}

impl Client {
    pub fn new() -> Self {
        Self {
            agent: ureq::AgentBuilder::new()
                .timeout_connect(Duration::from_secs(10))
                .timeout(Duration::from_secs(30))
                .user_agent(USER_AGENT)
                .build(),
            auth: Arc::new(Mutex::new(Auth::load())),
            fixtures: std::env::var_os("M3_MUSIC_FIXTURES").map(PathBuf::from),
        }
    }

    pub fn agent(&self) -> &ureq::Agent {
        &self.agent
    }

    pub fn fixtures(&self) -> bool {
        self.fixtures.is_some()
    }

    pub fn auth(&self) -> Option<Auth> {
        self.auth.lock().ok().and_then(|a| a.clone())
    }

    pub fn signed_in(&self) -> bool {
        self.auth().is_some()
    }

    /// Replace the credentials (and persist them; `None` signs out).
    pub fn set_auth(&self, auth: Option<Auth>) -> Result<()> {
        match &auth {
            Some(a) => a.save()?,
            None => Auth::delete(),
        }
        auth::sync_cookie_file(auth.as_ref());
        if let Ok(mut slot) = self.auth.lock() {
            *slot = auth;
        }
        Ok(())
    }

    fn fixture_for(&self, endpoint: &str, body: &Value) -> Option<&'static str> {
        let browse = body.get("browseId").and_then(Value::as_str).unwrap_or("");
        Some(match endpoint {
            "search" => "search",
            "music/get_search_suggestions" => "suggestions",
            "next" => "next",
            "account/account_menu" => "account",
            "browse" if body.get("continuation").is_some() => "playlist_continuation",
            "browse" => match browse {
                "FEmusic_home" => "home",
                "FEmusic_liked_playlists" => "library_playlists",
                "FEmusic_liked_albums" => "library_albums",
                "FEmusic_library_corpus_artists" | "FEmusic_library_corpus_track_artists" => "library_artists",
                "FEmusic_history" => "history",
                "VLLM" => "liked",
                b if b.starts_with("MPREb_") => "album",
                b if b.starts_with("UC") => "artist",
                b if b.starts_with("MPLYt") => "lyrics",
                b if b.starts_with("VL") => "playlist",
                _ => return None,
            },
            _ => return None,
        })
    }

    /// POST an InnerTube endpoint with the standard context.
    pub fn post(&self, endpoint: &str, mut body: Value) -> Result<Value> {
        if let Some(dir) = &self.fixtures {
            std::thread::sleep(Duration::from_millis(150));
            return match self.fixture_for(endpoint, &body) {
                Some(name) => {
                    let text = std::fs::read_to_string(dir.join(format!("{name}.json")))
                        .with_context(|| format!("fixture {name}"))?;
                    Ok(serde_json::from_str(&text)?)
                }
                None => Ok(json!({"playlistId": "PLnew0000"})),
            };
        }
        body["context"] = json!({
            "client": {"clientName": "WEB_REMIX", "clientVersion": client_version(), "hl": "en", "gl": "US"},
            "user": {}
        });
        let mut req = self
            .agent
            .post(&format!("{API}/{endpoint}?alt=json&prettyPrint=false"))
            .set("Content-Type", "application/json")
            .set("Origin", ORIGIN)
            .set("Referer", &format!("{ORIGIN}/"))
            .set("X-YouTube-Client-Name", "67")
            .set("X-YouTube-Client-Version", &client_version());
        let mut save = None;
        if let Ok(mut slot) = self.auth.lock() {
            if let Some(auth) = slot.as_mut() {
                let (headers, changed) = auth.headers(&self.agent)?;
                for (k, v) in &headers {
                    req = req.set(k, v);
                }
                if changed {
                    save = Some(auth.clone());
                }
            }
        }
        if let Some(a) = save {
            let _ = a.save();
        }
        match req.send_string(&body.to_string()) {
            Ok(resp) => Ok(resp.into_json()?),
            Err(ureq::Error::Status(code, resp)) => {
                let detail: Value = resp.into_json().unwrap_or(Value::Null);
                let msg = json::str_at(&detail, "error/message").unwrap_or("").to_string();
                Err(match code {
                    401 | 403 => anyhow!("YouTube Music rejected the sign-in ({code}). Sign in again. {msg}"),
                    _ => anyhow!("YouTube Music returned {code}. {msg}"),
                })
            }
            Err(e) => Err(anyhow!("couldn't reach YouTube Music: {e}")),
        }
    }

    fn browse(&self, id: &str) -> Result<Value> {
        self.post("browse", json!({"browseId": id}))
    }

    pub fn home(&self) -> Result<Vec<Shelf>> {
        Ok(parse::shelves(&self.browse("FEmusic_home")?))
    }

    pub fn search(&self, query: &str, filter: Option<Filter>) -> Result<Vec<Shelf>> {
        let mut body = json!({"query": query});
        if let Some(f) = filter {
            body["params"] = json!(f.params());
        }
        Ok(parse::shelves(&self.post("search", body)?))
    }

    pub fn suggestions(&self, input: &str) -> Result<Vec<String>> {
        Ok(parse::suggestions(&self.post("music/get_search_suggestions", json!({"input": input}))?))
    }

    pub fn detail(&self, id: &str) -> Result<Detail> {
        let mut d = parse::detail(&self.browse(id)?, id);
        if d.title.is_empty() && d.tracks.is_empty() && d.shelves.is_empty() {
            anyhow::bail!("nothing found for {id}");
        }
        // Liked Music has no header of its own.
        if id == "VLLM" && d.title.is_empty() {
            d.title = "Liked Music".into();
        }
        Ok(d)
    }

    pub fn more_tracks(&self, token: &str) -> Result<(Vec<Track>, Option<String>)> {
        let v = self.post("browse", json!({"continuation": token}))?;
        Ok((parse::tracks(&v), parse::continuation(&v)))
    }

    /// Up to `pages` continuation pages after the first.
    fn all_tracks(&self, id: &str, pages: usize) -> Result<Vec<Track>> {
        let d = self.detail(id)?;
        let mut tracks = d.tracks;
        let mut token = d.continuation;
        for _ in 0..pages {
            let Some(t) = token.take() else { break };
            let (more, next) = self.more_tracks(&t)?;
            if more.is_empty() {
                break;
            }
            tracks.extend(more);
            token = next;
        }
        Ok(tracks)
    }

    /// A radio / watch queue starting from a song (`RDAMVM<id>`), or the
    /// tracks of a playlist when `playlist_id` is given.
    pub fn radio(&self, video_id: Option<&str>, playlist_id: Option<&str>) -> Result<(Vec<Track>, Option<String>)> {
        let mut body = json!({"isAudioOnly": true, "enablePersistentPlaylistPanel": true});
        if let Some(v) = video_id {
            body["videoId"] = json!(v);
            body["playlistId"] = json!(playlist_id.map(str::to_string).unwrap_or_else(|| format!("RDAMVM{v}")));
        } else if let Some(p) = playlist_id {
            body["playlistId"] = json!(p);
        }
        Ok(parse::next(&self.post("next", body)?))
    }

    pub fn lyrics(&self, video_id: &str) -> Result<Option<Lyrics>> {
        let (_, browse) = self.radio(Some(video_id), None)?;
        match browse {
            Some(id) => Ok(parse::lyrics(&self.browse(&id)?)),
            None => Ok(None),
        }
    }

    pub fn account(&self) -> Result<Option<Account>> {
        Ok(parse::account(&self.post("account/account_menu", json!({}))?))
    }

    /// Fetch the whole library. Sections that fail are reported in
    /// `errors` instead of failing the sync.
    pub fn sync_library(&self) -> LibraryData {
        let mut data = LibraryData {
            synced_at: chrono::Utc::now().timestamp(),
            ..Default::default()
        };
        let mut errors = Vec::new();
        match self.browse("FEmusic_liked_playlists") {
            Ok(v) => data.playlists = parse::items(&v),
            Err(e) => errors.push(section_error("Playlists", e)),
        }
        match self.all_tracks("VLLM", 20) {
            Ok(t) => data.liked = t,
            Err(e) => errors.push(section_error("Liked songs", e)),
        }
        match self.browse("FEmusic_liked_albums") {
            Ok(v) => data.albums = parse::items(&v),
            Err(e) => errors.push(section_error("Albums", e)),
        }
        match self.browse("FEmusic_library_corpus_artists") {
            Ok(v) => data.artists = parse::items(&v),
            Err(e) => errors.push(section_error("Artists", e)),
        }
        match self.browse("FEmusic_history") {
            Ok(v) => data.history = only_tracks(parse::items(&v)),
            Err(e) => errors.push(section_error("History", e)),
        }
        match self.account() {
            Ok(a) => data.account = a,
            Err(e) => errors.push(section_error("Account", e)),
        }
        data.errors = errors;
        data
    }

    pub fn like(&self, video_id: &str, like: bool) -> Result<()> {
        let endpoint = if like { "like/like" } else { "like/removelike" };
        self.post(endpoint, json!({"target": {"videoId": video_id}}))?;
        Ok(())
    }

    pub fn add_to_playlist(&self, playlist_id: &str, video_ids: &[String]) -> Result<()> {
        let actions: Vec<Value> = video_ids
            .iter()
            .map(|v| json!({"action": "ACTION_ADD_VIDEO", "addedVideoId": v, "dedupeOption": "DEDUPE_OPTION_SKIP"}))
            .collect();
        self.edit_playlist(playlist_id, actions)
    }

    pub fn remove_from_playlist(&self, playlist_id: &str, video_id: &str, set_video_id: &str) -> Result<()> {
        self.edit_playlist(
            playlist_id,
            vec![json!({"action": "ACTION_REMOVE_VIDEO", "removedVideoId": video_id, "setVideoId": set_video_id})],
        )
    }

    fn edit_playlist(&self, playlist_id: &str, actions: Vec<Value>) -> Result<()> {
        let id = playlist_id.strip_prefix("VL").unwrap_or(playlist_id);
        let v = self.post("browse/edit_playlist", json!({"playlistId": id, "actions": actions}))?;
        match json::str_at(&v, "status") {
            Some(s) if s != "STATUS_SUCCEEDED" => Err(anyhow!("YouTube Music said {s}")),
            _ => Ok(()),
        }
    }

    pub fn create_playlist(&self, title: &str, video_ids: &[String]) -> Result<String> {
        let v = self.post(
            "playlist/create",
            json!({"title": title, "description": "", "privacyStatus": "PRIVATE", "videoIds": video_ids}),
        )?;
        parse::created_playlist(&v).ok_or_else(|| anyhow!("YouTube Music didn't return the new playlist"))
    }
}

fn section_error(what: &str, e: anyhow::Error) -> String {
    format!("{what}: {e}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_client() -> Client {
        Client {
            fixtures: Some(PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/fixtures"))),
            ..Client::new()
        }
    }

    #[test]
    fn fixture_mode_end_to_end() {
        let c = fixture_client();
        assert_eq!(c.home().unwrap().len(), 4);
        assert!(!c.search("aurora", Some(Filter::Songs)).unwrap().is_empty());
        assert_eq!(c.detail("VLPLchill0001").unwrap().tracks.len(), 8);
        let lib = c.sync_library();
        assert!(lib.errors.is_empty(), "{:?}", lib.errors);
        assert_eq!(lib.liked.len(), 6);
        assert_eq!(lib.playlists.len(), 4);
        assert_eq!(lib.history.len(), 7);
        assert_eq!(lib.account.unwrap().name, "Bhima");
        assert!(c.lyrics("dQw4w9WgXcQ").unwrap().is_some());
    }
}
