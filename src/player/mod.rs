//! Playback: the queue, shuffle/repeat, autoplay radio, and the mpv engine.

pub mod mpris;
pub mod mpv;

use crate::downloads::SharedIndex;
use material_3_ui::sys::task::Task;
use crate::yt::model::Track;
use crate::yt::Client;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Repeat {
    #[default]
    Off,
    All,
    One,
}

/// Something the UI should tell the user.
pub enum Notice {
    Error(String),
    Info(String),
}

pub struct Player {
    ctx: egui::Context,
    client: Client,
    downloads: SharedIndex,
    mpv: Option<mpv::Mpv>,
    pub queue: Vec<Track>,
    pub index: Option<usize>,
    pub shuffle: bool,
    /// Queue order before shuffling, restored when shuffle is turned off.
    unshuffled: Option<Vec<Track>>,
    pub repeat: Repeat,
    pub volume: f64,
    pub autoplay: bool,
    /// Last known engine state, refreshed every frame.
    pub state: mpv::State,
    radio: Task<anyhow::Result<Vec<Track>>>,
    /// The radio fetch should start playing when it arrives.
    radio_play: bool,
    notices: Vec<Notice>,
    /// Bumped whenever the current track changes (for MPRIS and art).
    pub generation: u64,
    /// Extra mpv options from the config.
    pub mpv_args: Vec<String>,
}

impl Player {
    pub fn new(ctx: egui::Context, client: Client, downloads: SharedIndex, volume: f64, mpv_args: Vec<String>) -> Self {
        Self {
            ctx,
            client,
            downloads,
            mpv: None,
            queue: Vec::new(),
            index: None,
            shuffle: false,
            unshuffled: None,
            repeat: Repeat::Off,
            volume,
            autoplay: true,
            state: mpv::State {
                idle: true,
                volume,
                ..Default::default()
            },
            radio: Task::default(),
            radio_play: false,
            mpv_args,
            notices: Vec::new(),
            generation: 0,
        }
    }

    pub fn current(&self) -> Option<&Track> {
        self.index.and_then(|i| self.queue.get(i))
    }

    pub fn is_playing(&self) -> bool {
        self.current().is_some() && !self.state.paused && !self.state.idle
    }

    pub fn take_notices(&mut self) -> Vec<Notice> {
        std::mem::take(&mut self.notices)
    }

    fn engine(&mut self) -> Option<&mut mpv::Mpv> {
        if self.mpv.as_mut().is_some_and(|m| !m.alive()) {
            self.mpv = None;
        }
        if self.mpv.is_none() {
            let cookies = crate::yt::auth::sync_cookie_file(self.client.auth().as_ref());
            match mpv::Mpv::spawn(self.ctx.clone(), cookies, self.volume, &self.mpv_args) {
                Ok(m) => self.mpv = Some(m),
                Err(e) => {
                    self.notices.push(Notice::Error(format!("{e}. Install mpv and yt-dlp to play music.")));
                    return None;
                }
            }
        }
        self.mpv.as_mut()
    }

    /// Restart mpv on next play (after signing in or out, so it picks up
    /// the new cookies).
    pub fn reset_engine(&mut self) {
        let was = self.state.time_pos;
        let playing = self.current().is_some();
        self.mpv = None;
        self.state.idle = true;
        if playing {
            self.state.paused = true;
            self.state.time_pos = was;
        }
    }

    /// What mpv should open for a track: the file, a finished download, or
    /// the YouTube Music URL.
    fn source(&self, track: &Track) -> Option<String> {
        if let Some(p) = &track.path {
            return Some(p.to_string_lossy().into_owned());
        }
        let id = track.video_id.as_ref()?;
        if let Some(path) = self.downloads.lock().ok().and_then(|d| d.get(id).cloned()) {
            if path.is_file() {
                return Some(path.to_string_lossy().into_owned());
            }
        }
        track.watch_url()
    }

    fn start(&mut self, index: usize) {
        let Some(track) = self.queue.get(index).cloned() else { return };
        self.index = Some(index);
        self.generation += 1;
        let Some(src) = self.source(&track) else {
            self.notices.push(Notice::Error(format!("Can't play “{}”", track.title)));
            return;
        };
        if let Some(m) = self.engine() {
            if let Err(e) = m.load(&src) {
                self.notices.push(Notice::Error(e.to_string()));
                self.mpv = None;
            }
        }
        self.state.time_pos = 0.0;
        self.state.duration = track.duration.unwrap_or(0) as f64;
        self.state.paused = false;
        self.state.idle = false;
        self.state.loading = true;
    }

    /// Replace the queue and start at `start`.
    pub fn play_list(&mut self, tracks: Vec<Track>, start: usize) {
        if tracks.is_empty() {
            return;
        }
        let start = start.min(tracks.len() - 1);
        self.queue = tracks;
        self.unshuffled = None;
        if self.shuffle {
            self.unshuffled = Some(self.queue.clone());
            let first = self.queue.remove(start);
            fastrand::shuffle(&mut self.queue);
            self.queue.insert(0, first);
            self.start(0);
        } else {
            self.start(start);
        }
    }

    pub fn play_one(&mut self, track: Track) {
        self.play_list(vec![track], 0);
    }

    /// Start a radio from a song: play it now and fill the queue with
    /// related songs.
    pub fn start_radio(&mut self, track: Track) {
        self.play_one(track.clone());
        if let Some(id) = track.video_id.clone() {
            self.fetch_radio(id, false);
        }
    }

    fn fetch_radio(&mut self, video_id: String, play: bool) {
        if self.radio.running() {
            return;
        }
        let client = self.client.clone();
        self.radio_play = play;
        self.radio.spawn(&self.ctx, move || client.radio(Some(&video_id), None).map(|(t, _)| t));
    }

    pub fn play_next(&mut self, tracks: Vec<Track>) {
        let at = self.index.map(|i| i + 1).unwrap_or(0);
        let was_empty = self.queue.is_empty();
        for (n, t) in tracks.into_iter().enumerate() {
            self.queue.insert(at + n, t);
        }
        if was_empty {
            self.start(0);
        }
    }

    pub fn enqueue(&mut self, tracks: Vec<Track>) {
        let was_empty = self.queue.is_empty();
        self.queue.extend(tracks);
        if was_empty && !self.queue.is_empty() {
            self.start(0);
        }
    }

    pub fn jump(&mut self, index: usize) {
        if index < self.queue.len() {
            self.start(index);
        }
    }

    pub fn remove(&mut self, index: usize) {
        if index >= self.queue.len() {
            return;
        }
        self.queue.remove(index);
        match self.index {
            Some(i) if i == index => {
                if self.queue.is_empty() {
                    self.stop();
                } else {
                    self.start(index.min(self.queue.len() - 1));
                }
            }
            Some(i) if i > index => self.index = Some(i - 1),
            _ => {}
        }
    }

    /// Move a queue entry from `from` to `to`.
    pub fn reorder(&mut self, from: usize, to: usize) {
        if from >= self.queue.len() || to >= self.queue.len() || from == to {
            return;
        }
        let t = self.queue.remove(from);
        self.queue.insert(to, t);
        if let Some(i) = self.index {
            self.index = Some(if i == from {
                to
            } else if from < i && to >= i {
                i - 1
            } else if from > i && to <= i {
                i + 1
            } else {
                i
            });
        }
    }

    /// Drop everything after the current song.
    pub fn clear_upcoming(&mut self) {
        if let Some(i) = self.index {
            self.queue.truncate(i + 1);
        } else {
            self.queue.clear();
        }
        self.unshuffled = None;
    }

    pub fn stop(&mut self) {
        if let Some(m) = self.mpv.as_mut() {
            let _ = m.stop();
        }
        self.index = None;
        self.queue.clear();
        self.state.idle = true;
        self.generation += 1;
    }

    pub fn toggle_pause(&mut self) {
        if self.current().is_none() {
            return;
        }
        if self.state.idle || self.mpv.is_none() {
            // Engine was restarted or stopped: reload the current song.
            if let Some(i) = self.index {
                let pos = self.state.time_pos;
                self.start(i);
                if pos > 1.0 {
                    self.seek(pos);
                }
            }
            return;
        }
        let paused = !self.state.paused;
        self.set_pause(paused);
    }

    pub fn set_pause(&mut self, paused: bool) {
        if let Some(m) = self.mpv.as_mut() {
            let _ = m.set_pause(paused);
        }
        self.state.paused = paused;
    }

    pub fn seek(&mut self, secs: f64) {
        if let Some(m) = self.mpv.as_mut() {
            let _ = m.seek(secs);
        }
        self.state.time_pos = secs;
    }

    pub fn set_volume(&mut self, v: f64) {
        self.volume = v.clamp(0.0, 130.0);
        if let Some(m) = self.mpv.as_mut() {
            let _ = m.set_volume(self.volume);
        }
    }

    pub fn next(&mut self) {
        match self.index {
            Some(i) if i + 1 < self.queue.len() => self.start(i + 1),
            Some(_) if self.repeat == Repeat::All => self.start(0),
            Some(i) => {
                if self.autoplay {
                    if let Some(id) = self.queue.get(i).and_then(|t| t.video_id.clone()) {
                        self.fetch_radio(id, true);
                        return;
                    }
                }
                self.set_pause(true);
                self.state.idle = true;
            }
            None => {}
        }
    }

    pub fn previous(&mut self) {
        match self.index {
            _ if self.state.time_pos > 3.0 => self.seek(0.0),
            Some(i) if i > 0 => self.start(i - 1),
            Some(_) if self.repeat == Repeat::All && !self.queue.is_empty() => self.start(self.queue.len() - 1),
            Some(_) => self.seek(0.0),
            None => {}
        }
    }

    pub fn set_shuffle(&mut self, on: bool) {
        if on == self.shuffle {
            return;
        }
        self.shuffle = on;
        let current = self.current().map(Track::key);
        if on {
            self.unshuffled = Some(self.queue.clone());
            let from = self.index.map(|i| i + 1).unwrap_or(0);
            if from < self.queue.len() {
                fastrand::shuffle(&mut self.queue[from..]);
            }
        } else if let Some(orig) = self.unshuffled.take() {
            // Keep songs added while shuffled.
            let mut restored = orig;
            for t in &self.queue {
                if !restored.iter().any(|o| o.key() == t.key()) {
                    restored.push(t.clone());
                }
            }
            self.queue = restored;
            self.index = current.and_then(|k| self.queue.iter().position(|t| t.key() == k));
        }
    }

    pub fn cycle_repeat(&mut self) {
        self.repeat = match self.repeat {
            Repeat::Off => Repeat::All,
            Repeat::All => Repeat::One,
            Repeat::One => Repeat::Off,
        };
    }

    /// Call once per frame: pick up engine state and events.
    pub fn poll(&mut self) {
        if let Some(m) = &self.mpv {
            let snap = m.snapshot();
            self.state.time_pos = snap.time_pos;
            if snap.duration > 0.0 {
                self.state.duration = snap.duration;
            }
            self.state.paused = snap.paused;
            self.state.volume = snap.volume;
            self.state.idle = snap.idle && self.state.idle;
            self.state.loading = snap.loading;
            let events: Vec<mpv::Event> = m.events.try_iter().collect();
            for ev in events {
                match ev {
                    mpv::Event::FileLoaded => {
                        self.state.idle = false;
                    }
                    mpv::Event::EndFile { reason, error } => match reason.as_str() {
                        "eof" => {
                            if self.repeat == Repeat::One {
                                if let Some(i) = self.index {
                                    self.start(i);
                                }
                            } else {
                                self.next();
                            }
                        }
                        "error" => {
                            let title = self.current().map(|t| t.title.clone()).unwrap_or_default();
                            self.notices.push(Notice::Error(format!(
                                "Couldn't play “{title}”: {}",
                                explain_error(&error.unwrap_or_else(|| "unknown error".into()))
                            )));
                            // A broken audio device fails every song: don't skip through the queue.
                            let systemic = reason_is_systemic(self.notices.last());
                            if !systemic && self.index.is_some_and(|i| i + 1 < self.queue.len()) {
                                self.next();
                            } else {
                                self.state.idle = true;
                                self.state.paused = true;
                            }
                        }
                        _ => {}
                    },
                    mpv::Event::Exited => {
                        self.mpv = None;
                        self.state.idle = true;
                        self.state.paused = true;
                        break;
                    }
                }
            }
        }
        if self.radio.poll() {
            match self.radio.value.take() {
                Some(Ok(tracks)) => {
                    let known: std::collections::HashSet<String> = self.queue.iter().map(Track::key).collect();
                    let new: Vec<Track> = tracks.into_iter().filter(|t| !known.contains(&t.key())).collect();
                    let first_new = self.queue.len();
                    if new.is_empty() {
                        self.notices.push(Notice::Info("No more songs to play".into()));
                    }
                    self.queue.extend(new);
                    if self.radio_play && first_new < self.queue.len() {
                        self.start(first_new);
                    }
                }
                Some(Err(e)) => self.notices.push(Notice::Error(format!("Couldn't load related songs: {e}"))),
                None => {}
            }
            self.radio_play = false;
        }
    }

    pub fn radio_loading(&self) -> bool {
        self.radio.running()
    }

    /// The file a track would be played from, when it's on disk.
    pub fn local_path(&self, track: &Track) -> Option<PathBuf> {
        if let Some(p) = &track.path {
            return Some(p.clone());
        }
        let id = track.video_id.as_ref()?;
        self.downloads.lock().ok()?.get(id).cloned()
    }
}

fn reason_is_systemic(notice: Option<&Notice>) -> bool {
    matches!(notice, Some(Notice::Error(e)) if e.contains("audio output") || e.contains("audio device") || e.contains(BOT_CHECK))
}

const BOT_CHECK: &str = "YouTube wants to confirm you're not a bot";

/// Turns yt-dlp's errors into something actionable. YouTube's bot check
/// applies to every song, so it's also treated as systemic.
pub fn explain_error(e: &str) -> String {
    if e.contains("not a bot") {
        format!("{BOT_CHECK}. Sign in with your browser session in Settings → Account, so playback and downloads use your cookies.")
    } else {
        e.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bot_checks_stop_the_queue() {
        let e = explain_error("[youtube] FGBhQbmPwH8: Sign in to confirm you’re not a bot. Use --cookies-from-browser");
        assert!(e.contains("Settings → Account"));
        assert!(reason_is_systemic(Some(&Notice::Error(format!("Couldn't play “x”: {e}")))));
        assert!(!reason_is_systemic(Some(&Notice::Error("Couldn't play “x”: Video unavailable".into()))));
    }

    fn t(n: &str) -> Track {
        Track {
            video_id: Some(n.into()),
            title: n.into(),
            ..Default::default()
        }
    }

    fn player() -> Player {
        let mut p = Player::new(egui::Context::default(), Client::new(), Default::default(), 80.0, Vec::new());
        // Never spawn mpv in tests.
        p.queue = vec![t("a"), t("b"), t("c"), t("d")];
        p.index = Some(1);
        p
    }

    #[test]
    fn reorder_tracks_current() {
        let mut p = player();
        p.reorder(1, 3);
        assert_eq!(p.index, Some(3));
        p.reorder(0, 3);
        assert_eq!(p.index, Some(2));
        assert_eq!(p.current().unwrap().title, "b");
        p.reorder(3, 0);
        assert_eq!(p.current().unwrap().title, "b");
    }

    #[test]
    fn shuffle_keeps_current_and_restores() {
        let mut p = player();
        p.set_shuffle(true);
        assert_eq!(p.current().unwrap().title, "b");
        assert_eq!(p.queue[..2].iter().map(|t| t.title.as_str()).collect::<Vec<_>>(), ["a", "b"]);
        p.queue.push(t("e"));
        p.set_shuffle(false);
        let order: Vec<&str> = p.queue.iter().map(|t| t.title.as_str()).collect();
        assert_eq!(order, ["a", "b", "c", "d", "e"]);
        assert_eq!(p.index, Some(1));
    }

    #[test]
    fn repeat_cycles() {
        let mut p = player();
        p.cycle_repeat();
        assert_eq!(p.repeat, Repeat::All);
        p.cycle_repeat();
        p.cycle_repeat();
        assert_eq!(p.repeat, Repeat::Off);
    }
}
