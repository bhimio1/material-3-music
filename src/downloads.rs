//! Downloading songs with yt-dlp, and remembering which YouTube songs are
//! on disk so they play offline.

use crate::config::{AudioFormat, Config};
use crate::yt::auth;
use crate::yt::model::Track;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};

/// videoId → downloaded file, shared with the player.
pub type SharedIndex = Arc<Mutex<HashMap<String, PathBuf>>>;

#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Queued,
    Running { percent: f32, speed: String, eta: String },
    Converting,
    Done(PathBuf),
    Failed(String),
    Cancelled,
}

impl Status {
    pub fn finished(&self) -> bool {
        matches!(self, Status::Done(_) | Status::Failed(_) | Status::Cancelled)
    }
}

enum Update {
    Progress { percent: f32, speed: String, eta: String },
    Converting,
    File(PathBuf),
    Exit(Result<(), String>),
}

pub struct Job {
    pub id: u64,
    pub track: Track,
    pub status: Status,
    child: Arc<Mutex<Option<Child>>>,
    updates: Option<flume::Receiver<Update>>,
    file: Option<PathBuf>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Record {
    pub track: Track,
    pub path: PathBuf,
    /// Unix seconds.
    pub at: i64,
}

pub struct Downloads {
    ctx: egui::Context,
    pub jobs: Vec<Job>,
    /// Finished downloads by videoId.
    pub records: BTreeMap<String, Record>,
    pub index: SharedIndex,
    next_id: u64,
}

fn records_path() -> PathBuf {
    auth::data_dir().join("downloads.json")
}

/// A file/folder name without path separators or characters that upset
/// file managers.
pub fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            c if c.is_control() => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_matches('.').trim();
    let mut out: String = trimmed.chars().take(120).collect();
    if out.is_empty() {
        out = "Unknown".into();
    }
    out
}

/// `<root>/<Artist>/<Album>/` for a track.
pub fn folder_for(root: &Path, track: &Track) -> PathBuf {
    let artist = track
        .artists
        .first()
        .map(|a| a.name.as_str())
        .unwrap_or("Unknown Artist");
    let album = track.album.as_ref().map(|a| a.name.as_str()).unwrap_or("Singles");
    root.join(sanitize(artist)).join(sanitize(album))
}

/// Arguments for yt-dlp (without the program name).
pub fn ytdlp_args(track: &Track, dir: &Path, format: AudioFormat, cookies: Option<&Path>) -> Option<Vec<String>> {
    let url = track.watch_url()?;
    let mut args: Vec<String> = [
        "--newline",
        "--no-colors",
        "--no-playlist",
        "--progress",
        "--progress-template",
        "download:M3P %(progress._percent_str)s|%(progress._speed_str)s|%(progress._eta_str)s",
        "--print",
        "after_move:M3FILE %(filepath)s",
        "-f",
        "bestaudio/best",
        "-x",
        "--audio-format",
        format.ext(),
        "--audio-quality",
        "0",
        "--embed-metadata",
        "--embed-thumbnail",
        "--convert-thumbnails",
        "jpg",
        "--parse-metadata",
        "%(track,title)s:%(meta_title)s",
        "--parse-metadata",
        "%(artist,creator,uploader)s:%(meta_artist)s",
        "--parse-metadata",
        "%(album,playlist_title)s:%(meta_album)s",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    if let Some(c) = cookies {
        args.push("--cookies".into());
        args.push(c.to_string_lossy().into_owned());
    }
    args.push("-o".into());
    args.push(
        dir.join(format!("{}.%(ext)s", sanitize(&track.title).replace('%', "%%")))
            .to_string_lossy()
            .into_owned(),
    );
    args.push("--".into());
    args.push(url);
    Some(args)
}

fn parse_line(line: &str) -> Option<Update> {
    if let Some(rest) = line.strip_prefix("M3P ") {
        let mut parts = rest.split('|');
        let percent = parts.next()?.trim().trim_end_matches('%').trim().parse::<f32>().ok()?;
        return Some(Update::Progress {
            percent,
            speed: parts.next().unwrap_or("").trim().to_string(),
            eta: parts.next().unwrap_or("").trim().to_string(),
        });
    }
    if let Some(path) = line.strip_prefix("M3FILE ") {
        return Some(Update::File(PathBuf::from(path.trim())));
    }
    if line.starts_with("[ExtractAudio]") || line.starts_with("[EmbedThumbnail]") || line.starts_with("[Metadata]") {
        return Some(Update::Converting);
    }
    None
}

impl Downloads {
    pub fn new(ctx: egui::Context) -> Self {
        let records: BTreeMap<String, Record> = std::fs::read_to_string(records_path())
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        let records: BTreeMap<String, Record> = records.into_iter().filter(|(_, r)| r.path.is_file()).collect();
        let index = Arc::new(Mutex::new(
            records.iter().map(|(k, r)| (k.clone(), r.path.clone())).collect(),
        ));
        Self {
            ctx,
            jobs: Vec::new(),
            records,
            index,
            next_id: 1,
        }
    }

    fn save(&self) {
        if let Ok(json) = serde_json::to_vec_pretty(&self.records) {
            if let Some(dir) = records_path().parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let _ = std::fs::write(records_path(), json);
        }
    }

    pub fn is_downloaded(&self, video_id: &str) -> bool {
        self.records.contains_key(video_id)
    }

    /// The active job for a song, if any.
    pub fn job_for(&self, video_id: &str) -> Option<&Job> {
        self.jobs
            .iter()
            .rev()
            .find(|j| j.track.video_id.as_deref() == Some(video_id) && !j.status.finished())
    }

    /// Queue songs; ones already downloaded or queued are skipped. Returns
    /// how many were added.
    pub fn enqueue(&mut self, tracks: &[Track]) -> usize {
        let mut added = 0;
        for t in tracks {
            let Some(id) = t.video_id.as_deref() else { continue };
            if self.is_downloaded(id) || self.job_for(id).is_some() {
                continue;
            }
            self.jobs.push(Job {
                id: self.next_id,
                track: t.clone(),
                status: Status::Queued,
                child: Arc::new(Mutex::new(None)),
                updates: None,
                file: None,
            });
            self.next_id += 1;
            added += 1;
        }
        added
    }

    pub fn cancel(&mut self, id: u64) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
            if let Ok(mut c) = job.child.lock() {
                if let Some(child) = c.as_mut() {
                    let _ = child.kill();
                }
            }
            job.status = Status::Cancelled;
            job.updates = None;
        }
    }

    pub fn retry(&mut self, id: u64) {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == id) {
            job.status = Status::Queued;
            job.file = None;
        }
    }

    pub fn clear_finished(&mut self) {
        self.jobs.retain(|j| !j.status.finished());
    }

    /// Delete a downloaded file and forget it.
    pub fn remove(&mut self, video_id: &str) -> Result<(), String> {
        if let Some(rec) = self.records.remove(video_id) {
            if rec.path.is_file() {
                std::fs::remove_file(&rec.path).map_err(|e| e.to_string())?;
            }
            if let Some(dir) = rec.path.parent() {
                // Tidy empty album/artist folders.
                let _ = std::fs::remove_dir(dir);
                if let Some(up) = dir.parent() {
                    let _ = std::fs::remove_dir(up);
                }
            }
        }
        if let Ok(mut idx) = self.index.lock() {
            idx.remove(video_id);
        }
        self.save();
        Ok(())
    }

    pub fn active(&self) -> usize {
        self.jobs.iter().filter(|j| !j.status.finished()).count()
    }

    fn start(&mut self, i: usize, config: &Config, cookies: Option<PathBuf>) {
        let job = &mut self.jobs[i];
        let dir = folder_for(&config.download_root(), &job.track);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            job.status = Status::Failed(format!("couldn't create {}: {e}", dir.display()));
            return;
        }
        let Some(args) = ytdlp_args(&job.track, &dir, config.download_format, cookies.as_deref()) else {
            job.status = Status::Failed("not a YouTube song".into());
            return;
        };
        let (tx, rx) = flume::unbounded();
        job.updates = Some(rx);
        job.status = Status::Running {
            percent: 0.0,
            speed: String::new(),
            eta: String::new(),
        };
        let slot = job.child.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let spawned = Command::new("yt-dlp")
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn();
            let mut child = match spawned {
                Ok(c) => c,
                Err(e) => {
                    let _ = tx.send(Update::Exit(Err(format!("couldn't start yt-dlp: {e}"))));
                    ctx.request_repaint();
                    return;
                }
            };
            let stdout = child.stdout.take();
            let stderr = child.stderr.take();
            if let Ok(mut s) = slot.lock() {
                *s = Some(child);
            }
            let err_lines = Arc::new(Mutex::new(Vec::<String>::new()));
            let err_thread = stderr.map(|e| {
                let lines = err_lines.clone();
                std::thread::spawn(move || {
                    for line in BufReader::new(e).lines().map_while(Result::ok) {
                        if let Ok(mut l) = lines.lock() {
                            l.push(line);
                            if l.len() > 20 {
                                l.remove(0);
                            }
                        }
                    }
                })
            });
            if let Some(out) = stdout {
                for line in BufReader::new(out).lines().map_while(Result::ok) {
                    if let Some(u) = parse_line(&line) {
                        let _ = tx.send(u);
                        ctx.request_repaint();
                    }
                }
            }
            if let Some(t) = err_thread {
                let _ = t.join();
            }
            // Poll rather than wait() so cancel() can take the lock to kill it.
            let status = loop {
                let r = slot.lock().ok().and_then(|mut c| c.as_mut().map(|c| c.try_wait()));
                match r {
                    Some(Ok(Some(s))) => break Some(s),
                    Some(Ok(None)) => std::thread::sleep(std::time::Duration::from_millis(100)),
                    _ => break None,
                }
            };
            let result = match status {
                Some(s) if s.success() => Ok(()),
                _ => {
                    let lines = err_lines.lock().map(|l| l.clone()).unwrap_or_default();
                    let msg = lines
                        .iter()
                        .rev()
                        .find(|l| l.starts_with("ERROR"))
                        .or(lines.last())
                        .cloned()
                        .unwrap_or_else(|| "yt-dlp failed".into());
                    Err(msg.trim_start_matches("ERROR: ").to_string())
                }
            };
            let _ = tx.send(Update::Exit(result));
            ctx.request_repaint();
        });
    }

    /// Process progress and start queued jobs. Returns messages for toasts.
    pub fn poll(&mut self, config: &Config, cookies: Option<PathBuf>) -> Vec<(String, bool)> {
        let mut notes = Vec::new();
        let mut finished = Vec::new();
        for job in &mut self.jobs {
            let Some(rx) = &job.updates else { continue };
            for u in rx.try_iter() {
                match u {
                    Update::Progress { percent, speed, eta } => {
                        job.status = Status::Running { percent, speed, eta };
                    }
                    Update::Converting => job.status = Status::Converting,
                    Update::File(p) => job.file = Some(p),
                    Update::Exit(Ok(())) => match job.file.clone() {
                        Some(p) if p.is_file() => {
                            job.status = Status::Done(p.clone());
                            finished.push((job.track.clone(), p));
                        }
                        _ => job.status = Status::Failed("yt-dlp didn't report the file".into()),
                    },
                    Update::Exit(Err(e)) => {
                        if job.status != Status::Cancelled {
                            notes.push((format!("Download of “{}” failed: {e}", job.track.title), true));
                            job.status = Status::Failed(e);
                        }
                    }
                }
            }
            if job.status.finished() {
                job.updates = None;
            }
        }
        if !finished.is_empty() {
            let now = chrono::Utc::now().timestamp();
            for (track, path) in finished {
                if let Some(id) = track.video_id.clone() {
                    if let Ok(mut idx) = self.index.lock() {
                        idx.insert(id.clone(), path.clone());
                    }
                    self.records.insert(id, Record { track, path, at: now });
                }
            }
            self.save();
            if self.active() == 0 {
                notes.push(("Downloads finished".into(), false));
            }
        }
        let running = self
            .jobs
            .iter()
            .filter(|j| matches!(j.status, Status::Running { .. } | Status::Converting))
            .count();
        let mut free = config.concurrent_downloads.max(1).saturating_sub(running);
        let mut i = 0;
        while free > 0 && i < self.jobs.len() {
            if self.jobs[i].status == Status::Queued {
                self.start(i, config, cookies.clone());
                free -= 1;
            }
            i += 1;
        }
        notes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::yt::model::Link;

    #[test]
    fn names_and_folders() {
        assert_eq!(sanitize("AC/DC: Live?"), "AC_DC_ Live_");
        assert_eq!(sanitize("  ..  "), "Unknown");
        let t = Track {
            video_id: Some("abc".into()),
            title: "Song 100%".into(),
            artists: vec![Link::new("Artist", None)],
            ..Default::default()
        };
        assert_eq!(folder_for(Path::new("/m"), &t), PathBuf::from("/m/Artist/Singles"));
        let args = ytdlp_args(&t, Path::new("/m/Artist/Singles"), AudioFormat::Mp3, None).unwrap();
        assert!(args.contains(&"mp3".to_string()));
        assert_eq!(args.last().unwrap(), "https://music.youtube.com/watch?v=abc");
        assert!(args.contains(&"/m/Artist/Singles/Song 100%%.%(ext)s".to_string()));
    }

    #[test]
    fn progress_lines() {
        match parse_line("M3P  42.5%|1.20MiB/s|00:03") {
            Some(Update::Progress { percent, speed, eta }) => {
                assert_eq!(percent, 42.5);
                assert_eq!(speed, "1.20MiB/s");
                assert_eq!(eta, "00:03");
            }
            _ => panic!(),
        }
        assert!(matches!(parse_line("M3FILE /m/a.opus"), Some(Update::File(_))));
        assert!(matches!(parse_line("[ExtractAudio] Destination: x"), Some(Update::Converting)));
        assert!(parse_line("[youtube] abc: Downloading webpage").is_none());
    }
}
