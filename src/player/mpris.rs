//! MPRIS (`org.mpris.MediaPlayer2.m3music`) so the desktop's media widgets,
//! playerctl and media keys see and control playback.
//!
//! The D-Bus connection lives on its own thread. The UI publishes a
//! [`Snapshot`] each frame; method calls come back as [`Command`]s.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use zbus::zvariant::{ObjectPath, OwnedValue, Value};

const PATH: &str = "/org/mpris/MediaPlayer2";

#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    PlayPause,
    Play,
    Pause,
    Stop,
    Next,
    Previous,
    /// Relative, microseconds.
    Seek(i64),
    /// Absolute, microseconds.
    SetPosition(i64),
    /// 0.0–1.0.
    SetVolume(f64),
    SetShuffle(bool),
    SetLoop(String),
    Raise,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub status: &'static str,
    pub track_id: u64,
    pub title: String,
    pub artists: Vec<String>,
    pub album: String,
    pub art_url: String,
    pub url: String,
    pub length_us: i64,
    pub position_us: i64,
    pub volume: f64,
    pub can_next: bool,
    pub can_previous: bool,
    pub shuffle: bool,
    pub loop_status: &'static str,
}

impl Snapshot {
    /// Equal apart from the playback position (which isn't signalled).
    fn same_except_position(&self, other: &Snapshot) -> bool {
        Snapshot {
            position_us: 0,
            ..self.clone()
        } == Snapshot {
            position_us: 0,
            ..other.clone()
        }
    }
}

enum Notify {
    Changed,
    Seeked(i64),
}

pub struct Mpris {
    shared: Arc<Mutex<Snapshot>>,
    notify: flume::Sender<Notify>,
    pub commands: flume::Receiver<Command>,
}

struct Shared {
    snap: Arc<Mutex<Snapshot>>,
    tx: flume::Sender<Command>,
    ctx: egui::Context,
}

impl Shared {
    fn send(&self, c: Command) {
        let _ = self.tx.send(c);
        self.ctx.request_repaint();
    }

    fn snap(&self) -> Snapshot {
        self.snap.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

struct Root(Shared);

#[zbus::interface(name = "org.mpris.MediaPlayer2")]
impl Root {
    fn raise(&self) {
        self.0.send(Command::Raise);
    }
    fn quit(&self) {}
    #[zbus(property)]
    fn can_quit(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn can_raise(&self) -> bool {
        true
    }
    #[zbus(property)]
    fn has_track_list(&self) -> bool {
        false
    }
    #[zbus(property)]
    fn identity(&self) -> String {
        "Material 3 Music".into()
    }
    #[zbus(property)]
    fn desktop_entry(&self) -> String {
        "material_3_music".into()
    }
    #[zbus(property)]
    fn supported_uri_schemes(&self) -> Vec<String> {
        vec!["file".into(), "https".into()]
    }
    #[zbus(property)]
    fn supported_mime_types(&self) -> Vec<String> {
        ["audio/mpeg", "audio/flac", "audio/ogg", "audio/opus", "audio/mp4", "audio/x-wav"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }
}

struct Player(Shared);

fn owned<'a>(v: impl Into<Value<'a>>) -> Option<OwnedValue> {
    v.into().try_to_owned().ok()
}

#[zbus::interface(name = "org.mpris.MediaPlayer2.Player")]
impl Player {
    fn play_pause(&self) {
        self.0.send(Command::PlayPause);
    }
    fn play(&self) {
        self.0.send(Command::Play);
    }
    fn pause(&self) {
        self.0.send(Command::Pause);
    }
    fn stop(&self) {
        self.0.send(Command::Stop);
    }
    fn next(&self) {
        self.0.send(Command::Next);
    }
    fn previous(&self) {
        self.0.send(Command::Previous);
    }
    fn seek(&self, offset: i64) {
        self.0.send(Command::Seek(offset));
    }
    fn set_position(&self, _track_id: ObjectPath<'_>, position: i64) {
        self.0.send(Command::SetPosition(position));
    }
    fn open_uri(&self, _uri: String) {}

    #[zbus(signal)]
    async fn seeked(ctxt: &zbus::SignalContext<'_>, position: i64) -> zbus::Result<()>;

    #[zbus(property)]
    fn playback_status(&self) -> String {
        let s = self.0.snap().status;
        if s.is_empty() { "Stopped" } else { s }.to_string()
    }
    #[zbus(property)]
    fn loop_status(&self) -> String {
        let s = self.0.snap().loop_status;
        if s.is_empty() { "None" } else { s }.to_string()
    }
    #[zbus(property)]
    fn set_loop_status(&self, value: String) {
        self.0.send(Command::SetLoop(value));
    }
    #[zbus(property)]
    fn rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn set_rate(&self, _rate: f64) {}
    #[zbus(property)]
    fn shuffle(&self) -> bool {
        self.0.snap().shuffle
    }
    #[zbus(property)]
    fn set_shuffle(&self, value: bool) {
        self.0.send(Command::SetShuffle(value));
    }
    #[zbus(property)]
    fn metadata(&self) -> HashMap<String, OwnedValue> {
        let s = self.0.snap();
        let mut m = HashMap::new();
        let id = format!("/org/m3music/track/{}", s.track_id);
        if let Ok(path) = ObjectPath::try_from(id.as_str()) {
            if let Some(v) = owned(path) {
                m.insert("mpris:trackid".into(), v);
            }
        }
        if s.title.is_empty() {
            return m;
        }
        let mut put = |k: &str, v: Option<OwnedValue>| {
            if let Some(v) = v {
                m.insert(k.to_string(), v);
            }
        };
        put("xesam:title", owned(s.title.as_str()));
        put("xesam:artist", owned(s.artists.clone()));
        put("xesam:album", owned(s.album.as_str()));
        if s.length_us > 0 {
            put("mpris:length", owned(s.length_us));
        }
        if !s.art_url.is_empty() {
            put("mpris:artUrl", owned(s.art_url.as_str()));
        }
        if !s.url.is_empty() {
            put("xesam:url", owned(s.url.as_str()));
        }
        m
    }
    #[zbus(property)]
    fn volume(&self) -> f64 {
        self.0.snap().volume
    }
    #[zbus(property)]
    fn set_volume(&self, value: f64) {
        self.0.send(Command::SetVolume(value));
    }
    #[zbus(property(emits_changed_signal = "false"))]
    fn position(&self) -> i64 {
        self.0.snap().position_us
    }
    #[zbus(property)]
    fn minimum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn maximum_rate(&self) -> f64 {
        1.0
    }
    #[zbus(property)]
    fn can_go_next(&self) -> bool {
        self.0.snap().can_next
    }
    #[zbus(property)]
    fn can_go_previous(&self) -> bool {
        self.0.snap().can_previous
    }
    #[zbus(property)]
    fn can_play(&self) -> bool {
        !self.0.snap().title.is_empty()
    }
    #[zbus(property)]
    fn can_pause(&self) -> bool {
        !self.0.snap().title.is_empty()
    }
    #[zbus(property)]
    fn can_seek(&self) -> bool {
        self.0.snap().length_us > 0
    }
    #[zbus(property)]
    fn can_control(&self) -> bool {
        true
    }
}

impl Mpris {
    /// Connect to the session bus. Failure (no bus) just means no MPRIS.
    pub fn start(ctx: egui::Context) -> Self {
        let shared = Arc::new(Mutex::new(Snapshot::default()));
        let (cmd_tx, commands) = flume::unbounded();
        let (notify, notify_rx) = flume::unbounded::<Notify>();
        let snap = shared.clone();
        std::thread::Builder::new()
            .name("mpris".into())
            .spawn(move || {
                if let Err(e) = serve(snap, cmd_tx, ctx, notify_rx) {
                    log::warn!("MPRIS unavailable: {e}");
                }
            })
            .ok();
        Self {
            shared,
            notify,
            commands,
        }
    }

    /// Publish the current state; signals only what changed.
    pub fn update(&self, snap: Snapshot) {
        let Ok(mut cur) = self.shared.lock() else { return };
        let changed = !cur.same_except_position(&snap);
        *cur = snap;
        drop(cur);
        if changed {
            let _ = self.notify.send(Notify::Changed);
        }
    }

    pub fn seeked(&self, position_us: i64) {
        let _ = self.notify.send(Notify::Seeked(position_us));
    }
}

fn serve(
    snap: Arc<Mutex<Snapshot>>,
    tx: flume::Sender<Command>,
    ctx: egui::Context,
    notify: flume::Receiver<Notify>,
) -> zbus::Result<()> {
    let mk = || Shared {
        snap: snap.clone(),
        tx: tx.clone(),
        ctx: ctx.clone(),
    };
    let conn = zbus::blocking::connection::Builder::session()?
        .name("org.mpris.MediaPlayer2.m3music")?
        .serve_at(PATH, Root(mk()))?
        .serve_at(PATH, Player(mk()))?
        .build()?;
    let iface = conn.object_server().interface::<_, Player>(PATH)?;
    for n in notify.iter() {
        let ctxt = iface.signal_context();
        let guard = iface.get();
        let result = match n {
            Notify::Changed => zbus::block_on(async {
                guard.playback_status_changed(ctxt).await?;
                guard.metadata_changed(ctxt).await?;
                guard.volume_changed(ctxt).await?;
                guard.shuffle_changed(ctxt).await?;
                guard.loop_status_changed(ctxt).await?;
                guard.can_go_next_changed(ctxt).await?;
                guard.can_go_previous_changed(ctxt).await?;
                guard.can_seek_changed(ctxt).await?;
                guard.can_play_changed(ctxt).await
            }),
            Notify::Seeked(pos) => zbus::block_on(Player::seeked(ctxt, pos)),
        };
        if let Err(e) = result {
            log::debug!("MPRIS signal failed: {e}");
        }
    }
    Ok(())
}
