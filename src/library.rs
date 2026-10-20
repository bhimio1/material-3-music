//! The signed-in user's library: cached on disk so it shows instantly and
//! offline, re-synced in the background, and edited optimistically (the UI
//! changes at once and rolls back if YouTube Music refuses).

use crate::sys::task::Task;
use crate::yt::model::{Collection, Item, Kind, Track};
use crate::yt::{auth, Client, LibraryData};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, Instant};

fn cache_path() -> PathBuf {
    auth::data_dir().join("library.json")
}

enum Undo {
    Like { track: Box<Track>, was_liked: bool },
    Nothing,
}

struct Mutation {
    task: Task<anyhow::Result<Option<String>>>,
    undo: Undo,
    /// Toast on success (empty = silent).
    done: String,
    what: String,
    /// A playlist was created: add it to the list once its id is known.
    created: Option<(String, Vec<Track>)>,
}

pub struct Library {
    pub data: LibraryData,
    pub liked_ids: HashSet<String>,
    sync: Task<LibraryData>,
    last_sync: Option<Instant>,
    mutations: Vec<Mutation>,
    /// Playlist ids whose cached detail page is stale.
    pub changed_playlists: Vec<String>,
}

impl Library {
    pub fn load() -> Self {
        let data: LibraryData = std::fs::read_to_string(cache_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let mut me = Self {
            data,
            liked_ids: HashSet::new(),
            sync: Task::default(),
            last_sync: None,
            mutations: Vec::new(),
            changed_playlists: Vec::new(),
        };
        me.reindex();
        me
    }

    fn reindex(&mut self) {
        self.liked_ids = self.data.liked.iter().filter_map(|t| t.video_id.clone()).collect();
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_vec(&self.data) {
            let _ = auth::write_private(&cache_path(), &json);
        }
    }

    pub fn clear(&mut self) {
        self.data = LibraryData::default();
        self.reindex();
        let _ = std::fs::remove_file(cache_path());
    }

    pub fn syncing(&self) -> bool {
        self.sync.running()
    }

    pub fn sync_now(&mut self, ctx: &egui::Context, client: &Client) {
        if self.sync.running() || !client.signed_in() {
            return;
        }
        let client = client.clone();
        self.last_sync = Some(Instant::now());
        self.sync.spawn(ctx, move || client.sync_library());
    }

    /// Start a background sync when due.
    pub fn maybe_sync(&mut self, ctx: &egui::Context, client: &Client, every_minutes: u32) {
        let due = match self.last_sync {
            None => true,
            Some(_) if every_minutes == 0 => false,
            Some(at) => at.elapsed() > Duration::from_secs(every_minutes as u64 * 60),
        };
        if due && client.signed_in() {
            self.sync_now(ctx, client);
        }
    }

    pub fn is_liked(&self, track: &Track) -> bool {
        track.video_id.as_ref().is_some_and(|id| self.liked_ids.contains(id))
    }

    /// Playlists the user can add songs to.
    pub fn own_playlists(&self) -> Vec<&Collection> {
        self.data
            .playlists
            .iter()
            .filter_map(|i| match i {
                Item::Collection(c) if c.kind == Kind::Playlist && c.id != "VLLM" && !c.id.starts_with("VLRD") => Some(c),
                _ => None,
            })
            .collect()
    }

    fn mutate(
        &mut self,
        ctx: &egui::Context,
        what: &str,
        done: String,
        undo: Undo,
        f: impl FnOnce() -> anyhow::Result<Option<String>> + Send + 'static,
    ) -> &mut Mutation {
        let mut task = Task::default();
        task.spawn(ctx, f);
        self.mutations.push(Mutation {
            task,
            undo,
            done,
            what: what.to_string(),
            created: None,
        });
        self.mutations.last_mut().expect("just pushed")
    }

    pub fn toggle_like(&mut self, ctx: &egui::Context, client: &Client, track: &Track) {
        let Some(id) = track.video_id.clone() else { return };
        let was_liked = self.liked_ids.contains(&id);
        if was_liked {
            self.liked_ids.remove(&id);
            self.data.liked.retain(|t| t.video_id.as_deref() != Some(&id));
        } else {
            self.liked_ids.insert(id.clone());
            self.data.liked.insert(0, track.clone());
        }
        let c = client.clone();
        let like = !was_liked;
        self.mutate(
            ctx,
            if like { "like" } else { "unlike" },
            String::new(),
            Undo::Like {
                track: Box::new(track.clone()),
                was_liked,
            },
            move || c.like(&id, like).map(|_| None),
        );
    }

    pub fn add_to_playlist(&mut self, ctx: &egui::Context, client: &Client, playlist: &Collection, tracks: Vec<Track>) {
        let ids: Vec<String> = tracks.iter().filter_map(|t| t.video_id.clone()).collect();
        if ids.is_empty() {
            return;
        }
        let c = client.clone();
        let pl = playlist.id.clone();
        self.changed_playlists.push(pl.clone());
        let n = ids.len();
        self.mutate(
            ctx,
            "add to the playlist",
            format!(
                "Added {} to {}",
                if n == 1 { "1 song".to_string() } else { format!("{n} songs") },
                playlist.title
            ),
            Undo::Nothing,
            move || c.add_to_playlist(&pl, &ids).map(|_| None),
        );
    }

    pub fn remove_from_playlist(&mut self, ctx: &egui::Context, client: &Client, playlist_id: &str, track: &Track) {
        let (Some(vid), Some(set)) = (track.video_id.clone(), track.set_video_id.clone()) else { return };
        let c = client.clone();
        let pl = playlist_id.to_string();
        self.changed_playlists.push(format!("VL{}", pl.trim_start_matches("VL")));
        self.mutate(
            ctx,
            "remove from the playlist",
            "Removed from the playlist".into(),
            Undo::Nothing,
            move || c.remove_from_playlist(&pl, &vid, &set).map(|_| None),
        );
    }

    pub fn create_playlist(&mut self, ctx: &egui::Context, client: &Client, title: String, tracks: Vec<Track>) {
        let ids: Vec<String> = tracks.iter().filter_map(|t| t.video_id.clone()).collect();
        let c = client.clone();
        let t = title.clone();
        let m = self.mutate(
            ctx,
            "create the playlist",
            format!("Created {title}"),
            Undo::Nothing,
            move || c.create_playlist(&t, &ids).map(Some),
        );
        m.created = Some((title, tracks));
    }

    /// Apply finished syncs and mutations. Returns (message, is_error).
    pub fn poll(&mut self) -> Vec<(String, bool)> {
        let mut notes = Vec::new();
        if self.sync.poll() {
            if let Some(data) = self.sync.value.take() {
                let errors = data.errors.clone();
                // Keep cached sections that failed this time.
                let old = std::mem::take(&mut self.data);
                let failed = |name: &str| errors.iter().any(|e| e.starts_with(name));
                self.data = LibraryData {
                    playlists: if failed("Playlists") { old.playlists } else { data.playlists },
                    liked: if failed("Liked") { old.liked } else { data.liked },
                    albums: if failed("Albums") { old.albums } else { data.albums },
                    artists: if failed("Artists") { old.artists } else { data.artists },
                    history: if failed("History") { old.history } else { data.history },
                    account: data.account.or(old.account),
                    synced_at: data.synced_at,
                    errors: errors.clone(),
                };
                self.reindex();
                self.save();
                if !errors.is_empty() {
                    notes.push((format!("Sync incomplete — {}", errors.join("; ")), true));
                }
            }
        }
        let mut finished = Vec::new();
        for (i, m) in self.mutations.iter_mut().enumerate() {
            if m.task.poll() {
                finished.push(i);
            }
        }
        for i in finished.into_iter().rev() {
            let mut m = self.mutations.remove(i);
            match m.task.value.take() {
                Some(Ok(new_id)) => {
                    if let (Some(id), Some((title, tracks))) = (new_id, m.created.take()) {
                        self.data.playlists.push(Item::Collection(Collection {
                            kind: Kind::Playlist,
                            id: format!("VL{id}"),
                            title,
                            subtitle: format!("Playlist • {} songs", tracks.len()),
                            thumbnail: tracks.first().and_then(|t| t.thumbnail.clone()),
                            play_id: Some(id),
                        }));
                    }
                    self.save();
                    if !m.done.is_empty() {
                        notes.push((m.done, false));
                    }
                }
                Some(Err(e)) => {
                    if let Undo::Like { track, was_liked } = m.undo {
                        let id = track.video_id.clone().unwrap_or_default();
                        if was_liked {
                            self.liked_ids.insert(id);
                            self.data.liked.insert(0, *track);
                        } else {
                            self.liked_ids.remove(&id);
                            self.data.liked.retain(|t| t.video_id.as_deref() != Some(&id));
                        }
                    }
                    notes.push((format!("Couldn't {}: {e}", m.what), true));
                }
                None => {}
            }
        }
        notes
    }

    pub fn last_synced(&self) -> String {
        if self.data.synced_at == 0 {
            return "Not synced yet".into();
        }
        let ago = chrono::Utc::now().timestamp() - self.data.synced_at;
        match ago {
            i64::MIN..=59 => "Synced just now".into(),
            60..=3599 => format!("Synced {} min ago", ago / 60),
            3600..=86_399 => format!("Synced {} h ago", ago / 3600),
            _ => format!("Synced {} days ago", ago / 86_400),
        }
    }
}
