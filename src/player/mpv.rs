//! mpv as the audio engine, driven over its JSON IPC socket. mpv plays local
//! files directly and YouTube URLs through yt-dlp.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct State {
    pub time_pos: f64,
    pub duration: f64,
    pub paused: bool,
    pub volume: f64,
    pub idle: bool,
    /// Buffering / opening a stream.
    pub loading: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    FileLoaded,
    /// `reason` is mpv's: `eof`, `stop`, `quit`, `error`, `redirect`.
    EndFile { reason: String, error: Option<String> },
    Exited,
}

pub struct Mpv {
    child: Child,
    writer: UnixStream,
    socket: PathBuf,
    pub state: Arc<Mutex<State>>,
    pub events: flume::Receiver<Event>,
}

const OBSERVED: &[&str] = &["time-pos", "duration", "pause", "volume", "idle-active", "paused-for-cache"];

impl Mpv {
    pub fn spawn(ctx: egui::Context, cookies: Option<PathBuf>, volume: f64, extra: &[String]) -> Result<Self> {
        let dir = dirs::runtime_dir().unwrap_or_else(std::env::temp_dir);
        let socket = dir.join(format!("m3-music-{}.sock", std::process::id()));
        let _ = std::fs::remove_file(&socket);
        let mut cmd = Command::new("mpv");
        cmd.args([
            "--idle=yes",
            "--no-video",
            "--no-terminal",
            "--audio-display=no",
            "--force-window=no",
            "--ytdl-format=bestaudio/best",
            "--keep-open=no",
            "--cache=yes",
            "--demuxer-max-bytes=64MiB",
        ])
        .arg(format!("--input-ipc-server={}", socket.display()))
        .arg(format!("--volume={volume:.0}"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
        if let Some(c) = cookies {
            cmd.arg(format!("--ytdl-raw-options=cookies={}", c.display()));
        }
        cmd.args(extra);
        let mut child = cmd.spawn().context("couldn't start mpv")?;

        let started = Instant::now();
        let stream = loop {
            match UnixStream::connect(&socket) {
                Ok(s) => break s,
                Err(_) if started.elapsed() < Duration::from_secs(5) => {
                    if let Ok(Some(status)) = child.try_wait() {
                        return Err(anyhow!("mpv exited at startup ({status})"));
                    }
                    std::thread::sleep(Duration::from_millis(30));
                }
                Err(e) => {
                    let _ = child.kill();
                    return Err(anyhow!("couldn't connect to mpv: {e}"));
                }
            }
        };
        let reader = stream.try_clone()?;
        let state = Arc::new(Mutex::new(State {
            volume,
            idle: true,
            ..Default::default()
        }));
        let (tx, events) = flume::unbounded();
        {
            let state = state.clone();
            std::thread::spawn(move || read_loop(reader, state, tx, ctx));
        }
        let mut mpv = Self {
            child,
            writer: stream,
            socket,
            state,
            events,
        };
        for (i, prop) in OBSERVED.iter().enumerate() {
            mpv.command(json!(["observe_property", i + 1, prop]))?;
        }
        Ok(mpv)
    }

    pub fn command(&mut self, args: Value) -> Result<()> {
        let mut line = json!({ "command": args }).to_string();
        line.push('\n');
        self.writer
            .write_all(line.as_bytes())
            .context("mpv isn't responding")
    }

    pub fn load(&mut self, target: &str) -> Result<()> {
        if let Ok(mut s) = self.state.lock() {
            s.loading = true;
            s.time_pos = 0.0;
            s.duration = 0.0;
        }
        self.command(json!(["loadfile", target, "replace"]))?;
        self.set_pause(false)
    }

    pub fn stop(&mut self) -> Result<()> {
        self.command(json!(["stop"]))
    }

    pub fn set_pause(&mut self, paused: bool) -> Result<()> {
        self.command(json!(["set_property", "pause", paused]))
    }

    pub fn seek(&mut self, secs: f64) -> Result<()> {
        self.command(json!(["seek", secs.max(0.0), "absolute"]))
    }

    pub fn set_volume(&mut self, volume: f64) -> Result<()> {
        self.command(json!(["set_property", "volume", volume.clamp(0.0, 130.0)]))
    }

    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    pub fn snapshot(&self) -> State {
        self.state.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

impl Drop for Mpv {
    fn drop(&mut self) {
        let _ = self.command(json!(["quit"]));
        std::thread::sleep(Duration::from_millis(50));
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket);
    }
}

fn read_loop(stream: UnixStream, state: Arc<Mutex<State>>, tx: flume::Sender<Event>, ctx: egui::Context) {
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
        if let Some(event) = apply(&msg, &state) {
            let _ = tx.send(event);
        }
        ctx.request_repaint();
    }
    let _ = tx.send(Event::Exited);
    ctx.request_repaint();
}

/// Apply one IPC message to the state; returns an event worth reacting to.
pub fn apply(msg: &Value, state: &Mutex<State>) -> Option<Event> {
    match msg.get("event").and_then(Value::as_str)? {
        "property-change" => {
            let name = msg.get("name").and_then(Value::as_str)?;
            let data = msg.get("data").unwrap_or(&Value::Null);
            let mut s = state.lock().ok()?;
            match name {
                "time-pos" => {
                    if let Some(t) = data.as_f64() {
                        s.time_pos = t;
                        s.loading = false;
                    }
                }
                "duration" => s.duration = data.as_f64().unwrap_or(0.0),
                "pause" => s.paused = data.as_bool().unwrap_or(false),
                "volume" => s.volume = data.as_f64().unwrap_or(s.volume),
                "idle-active" => s.idle = data.as_bool().unwrap_or(false),
                "paused-for-cache" => s.loading = data.as_bool().unwrap_or(false),
                _ => {}
            }
            None
        }
        "file-loaded" => Some(Event::FileLoaded),
        "end-file" => Some(Event::EndFile {
            reason: msg.get("reason").and_then(Value::as_str).unwrap_or("").to_string(),
            error: msg.get("file_error").and_then(Value::as_str).map(str::to_string),
        }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ipc_messages() {
        let state = Mutex::new(State::default());
        let msg = |s: &str| serde_json::from_str::<Value>(s).unwrap();
        assert_eq!(apply(&msg(r#"{"event":"property-change","id":1,"name":"time-pos","data":12.5}"#), &state), None);
        apply(&msg(r#"{"event":"property-change","id":2,"name":"duration","data":200.0}"#), &state);
        apply(&msg(r#"{"event":"property-change","id":3,"name":"pause","data":true}"#), &state);
        apply(&msg(r#"{"event":"property-change","id":1,"name":"time-pos"}"#), &state);
        let s = state.lock().unwrap().clone();
        assert_eq!((s.time_pos, s.duration, s.paused), (12.5, 200.0, true));
        assert_eq!(
            apply(&msg(r#"{"event":"end-file","reason":"error","file_error":"loading failed"}"#), &state),
            Some(Event::EndFile { reason: "error".into(), error: Some("loading failed".into()) })
        );
        assert_eq!(apply(&msg(r#"{"request_id":0,"error":"success"}"#), &state), None);
    }
}
