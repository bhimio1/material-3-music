//! Music files on disk: scanning library folders (tags and cover art via
//! lofty), caching the results, and rescanning when files change.

use crate::sys::task::Task;
use crate::yt::model::{Link, Track};
use lofty::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, UNIX_EPOCH};

const AUDIO_EXTS: &[&str] = &[
    "mp3", "flac", "ogg", "oga", "opus", "m4a", "aac", "wav", "wma", "aiff", "aif", "wv", "ape", "mka",
];

pub fn is_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| AUDIO_EXTS.contains(&e.to_ascii_lowercase().as_str()))
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct Cached {
    mtime: u64,
    size: u64,
    track: Track,
}

fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| crate::sys::paths::home().join(".cache"))
        .join("m3-music")
}

fn index_path() -> PathBuf {
    cache_dir().join("local.json")
}

fn hash_str(s: &str) -> String {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    format!("{:016x}", h.finish())
}

fn file_stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let mtime = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?.as_secs();
    Some((mtime, meta.len()))
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 12 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        if e.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        match e.file_type() {
            Ok(ft) if ft.is_dir() => walk(&path, out, depth + 1),
            Ok(ft) if ft.is_file() && is_audio(&path) => out.push(path),
            _ => {}
        }
    }
}

/// Cover art for a file: the embedded picture (written once per album into
/// the cache) or a cover/folder image next to it.
fn cover_for(path: &Path, tag: Option<&lofty::tag::Tag>, album_key: &str) -> Option<String> {
    if let Some(pic) = tag.and_then(|t| t.pictures().first()) {
        let ext = match pic.mime_type() {
            Some(lofty::picture::MimeType::Png) => "png",
            _ => "jpg",
        };
        let dest = cache_dir().join("covers").join(format!("{}.{ext}", hash_str(album_key)));
        if !dest.is_file() {
            let _ = std::fs::create_dir_all(dest.parent()?);
            std::fs::write(&dest, pic.data()).ok()?;
        }
        return Some(format!("file://{}", dest.display()));
    }
    let dir = path.parent()?;
    for name in ["cover.jpg", "cover.png", "folder.jpg", "folder.png", "Cover.jpg", "Folder.jpg", "front.jpg"] {
        let p = dir.join(name);
        if p.is_file() {
            return Some(format!("file://{}", p.display()));
        }
    }
    None
}

pub fn read_track(path: &Path) -> Track {
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut track = Track {
        path: Some(path.to_path_buf()),
        title: stem,
        ..Default::default()
    };
    let Ok(file) = lofty::read_from_path(path) else {
        track.thumbnail = cover_for(path, None, &path.to_string_lossy());
        return track;
    };
    let secs = file.properties().duration().as_secs();
    if secs > 0 {
        track.duration = Some(secs as u32);
    }
    let tag = file.primary_tag().or_else(|| file.first_tag());
    if let Some(tag) = tag {
        if let Some(t) = tag.title().filter(|t| !t.trim().is_empty()) {
            track.title = t.to_string();
        }
        if let Some(a) = tag.artist().filter(|a| !a.trim().is_empty()) {
            track.artists = a
                .split(['/', ';'])
                .map(|n| Link::new(n.trim(), None))
                .filter(|l| !l.name.is_empty())
                .collect();
        }
        if let Some(al) = tag.album().filter(|a| !a.trim().is_empty()) {
            track.album = Some(Link::new(al.to_string(), None));
        }
        track.track_no = tag.track();
        track.year = tag.year();
    }
    let album_key = format!(
        "{}\u{1f}{}",
        track.artists.first().map(|a| a.name.as_str()).unwrap_or(""),
        track.album.as_ref().map(|a| a.name.as_str()).unwrap_or(&path.to_string_lossy())
    );
    track.thumbnail = cover_for(path, tag, &album_key);
    track
}

/// Scan folders, reusing cached tags for unchanged files.
pub fn scan(folders: &[PathBuf]) -> Vec<Track> {
    let cached: HashMap<PathBuf, Cached> = std::fs::read_to_string(index_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<Cached>>(&s).ok())
        .map(|v| {
            v.into_iter()
                .filter_map(|c| Some((c.track.path.clone()?, c)))
                .collect()
        })
        .unwrap_or_default();
    let mut files = Vec::new();
    for f in folders {
        walk(f, &mut files, 0);
    }
    files.sort();
    files.dedup();
    let mut out = Vec::with_capacity(files.len());
    for path in files {
        let Some((mtime, size)) = file_stamp(&path) else { continue };
        let track = match cached.get(&path) {
            Some(c) if c.mtime == mtime && c.size == size => c.track.clone(),
            _ => read_track(&path),
        };
        out.push(Cached { mtime, size, track });
    }
    if let Ok(json) = serde_json::to_vec(&out) {
        let _ = std::fs::create_dir_all(cache_dir());
        let _ = std::fs::write(index_path(), json);
    }
    out.into_iter().map(|c| c.track).collect()
}

pub struct Local {
    pub tracks: Vec<Track>,
    task: Task<Vec<Track>>,
    _watcher: Option<notify::RecommendedWatcher>,
    changes: flume::Receiver<()>,
    dirty: Option<Instant>,
    folders: Vec<PathBuf>,
    ctx: egui::Context,
}

impl Local {
    pub fn new(ctx: egui::Context, folders: Vec<PathBuf>) -> Self {
        let (tx, changes) = flume::unbounded();
        let mut me = Self {
            tracks: Vec::new(),
            task: Task::default(),
            _watcher: None,
            changes,
            dirty: None,
            folders: Vec::new(),
            ctx,
        };
        me.set_folders(folders, tx);
        me
    }

    fn set_folders(&mut self, folders: Vec<PathBuf>, tx: flume::Sender<()>) {
        use notify::Watcher;
        let ctx = self.ctx.clone();
        self._watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                let relevant = ev.paths.iter().any(|p| is_audio(p) || p.extension().is_none());
                if relevant && (ev.kind.is_create() || ev.kind.is_remove() || ev.kind.is_modify()) {
                    let _ = tx.send(());
                    ctx.request_repaint_after(Duration::from_secs(2));
                }
            }
        })
        .ok()
        .map(|mut w| {
            for f in &folders {
                let _ = w.watch(f, notify::RecursiveMode::Recursive);
            }
            w
        });
        self.folders = folders;
        self.rescan();
    }

    /// Point at new folders (from preferences).
    pub fn change_folders(&mut self, folders: Vec<PathBuf>) {
        if folders == self.folders {
            return;
        }
        let (tx, rx) = flume::unbounded();
        self.changes = rx;
        self.set_folders(folders, tx);
    }

    pub fn rescan(&mut self) {
        let folders = self.folders.clone();
        self.task.spawn(&self.ctx, move || scan(&folders));
    }

    pub fn scanning(&self) -> bool {
        self.task.running()
    }

    pub fn poll(&mut self) {
        if self.task.poll() {
            if let Some(t) = self.task.value.take() {
                self.tracks = t;
            }
        }
        if self.changes.try_iter().count() > 0 {
            self.dirty = Some(Instant::now());
        }
        // Debounce: rescan once files stop changing for a couple of seconds.
        if let Some(at) = self.dirty {
            if at.elapsed() > Duration::from_secs(2) && !self.task.running() {
                self.dirty = None;
                self.rescan();
            } else {
                self.ctx.request_repaint_after(Duration::from_millis(500));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_untagged_files() {
        let dir = tempfile::tempdir().unwrap();
        let album = dir.path().join("Artist/Album");
        std::fs::create_dir_all(&album).unwrap();
        std::fs::write(album.join("01 Intro.mp3"), b"not really audio").unwrap();
        std::fs::write(album.join("cover.jpg"), b"jpg").unwrap();
        std::fs::write(album.join("notes.txt"), b"x").unwrap();
        std::fs::create_dir_all(dir.path().join(".hidden")).unwrap();
        std::fs::write(dir.path().join(".hidden/x.flac"), b"x").unwrap();
        let t = read_track(&album.join("01 Intro.mp3"));
        assert_eq!(t.title, "01 Intro");
        assert!(t.thumbnail.unwrap().ends_with("cover.jpg"));
        let mut files = Vec::new();
        walk(dir.path(), &mut files, 0);
        assert_eq!(files.len(), 1);
        assert!(is_audio(Path::new("a.FLAC")));
    }
}
