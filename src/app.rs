//! App state, theme, layout, keyboard shortcuts and action dispatch.

use crate::config::Config;
use crate::downloads::Downloads;
use crate::library::Library;
use crate::local::Local;
use crate::player::mpris::{self, Mpris};
use crate::player::{Notice, Player, Repeat};
use crate::thumbs::Thumbs;
use crate::ui::pages::{self, States};
use crate::ui::tracks::View;
use crate::ui::{self, Action, Actions, Page, ToastKind};
use crate::yt::model::{Kind, Track};
use crate::yt::Client;
use egui::{Color32, CornerRadius, Frame, Margin};
use material_3_ui::keys::take_key;
use material_3_ui::sys::arch::{self, AurHelper};
use material_3_ui::sys::task::Task;
use material_3_ui::sys::term;
use material_3_ui::theme::palette::to_hex;
use material_3_ui::theme::{style, ColorScheme, M3Palette, ThemeManager};
use material_3_ui::toasts::{Toast, Toasts};
use material_3_ui::widgets;
use std::path::PathBuf;
use std::time::Duration;

pub struct App {
    pub ctx: egui::Context,
    /// What's on screen: the theme's palette, or the album art's.
    pub palette: M3Palette,
    pub theme: ThemeManager,
    pub config: Config,
    pub client: Client,
    pub player: Player,
    mpris: Mpris,
    pub library: Library,
    pub local: Local,
    pub downloads: Downloads,
    pub thumbs: Thumbs,
    pub page: Page,
    history: Vec<Page>,
    pub toasts: Toasts<Action>,
    pub actions: Actions,
    pub pages: States,
    /// Collections being fetched so they can be played or downloaded.
    collection_jobs: Vec<(Task<anyhow::Result<Vec<Track>>>, CollectionJob)>,
    /// Name prompt for a new playlist.
    new_playlist: Option<(String, Vec<Track>)>,
    system_dark: Option<bool>,
    art_seed: Option<Color32>,
    last_seek_signal: f64,
}

#[derive(Clone, Copy)]
enum CollectionJob {
    Play { shuffle: bool },
    Download,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Vec<String>) -> Self {
        let ctx = cc.egui_ctx.clone();
        let config = Config::load();
        let client = Client::new();
        let downloads = Downloads::new(ctx.clone());
        let mut player = Player::new(ctx.clone(), client.clone(), downloads.index.clone(), config.volume, config.mpv_args.clone());
        player.autoplay = config.autoplay;
        let thumbs = Thumbs::new(ctx.clone(), client.agent().clone(), client.fixtures());
        let local = Local::new(ctx.clone(), config.folders());
        let theme = ThemeManager::new(&ctx, &config.appearance);
        let mut app = Self {
            palette: theme.palette().clone(),
            theme,
            mpris: Mpris::start(ctx.clone()),
            library: Library::load(),
            local,
            downloads,
            thumbs,
            page: Page::Home,
            history: Vec::new(),
            toasts: Toasts::default(),
            actions: Vec::new(),
            pages: States::default(),
            collection_jobs: Vec::new(),
            new_playlist: None,
            system_dark: ctx.system_theme().map(|t| t == egui::Theme::Dark),
            art_seed: None,
            last_seek_signal: 0.0,
            config,
            client,
            player,
            ctx,
        };
        app.apply_theme();
        app.open_args(open);
        // Development / screenshots: start on a given page.
        if let Ok(page) = std::env::var("M3_MUSIC_PAGE") {
            app.page = match page.split_once(':') {
                Some(("search", q)) => {
                    app.pages.search.set_query(q.to_string());
                    Page::Search
                }
                Some(("detail", id)) => Page::Detail(id.to_string()),
                _ => match page.as_str() {
                    "search" => Page::Search,
                    "playlists" => Page::Library(crate::ui::LibTab::Playlists),
                    "liked" => Page::Library(crate::ui::LibTab::Liked),
                    "albums" => Page::Library(crate::ui::LibTab::Albums),
                    "local" => Page::Local,
                    "downloads" => Page::Downloads,
                    "settings" => Page::Settings,
                    "now" => Page::NowPlaying,
                    _ => Page::Home,
                },
            };
        }
        app
    }

    /// Files and URLs from the command line.
    fn open_args(&mut self, args: Vec<String>) {
        let mut tracks = Vec::new();
        for a in args {
            if let Some(id) = video_id_from_url(&a) {
                tracks.push(Track {
                    video_id: Some(id.clone()),
                    title: a.clone(),
                    ..Default::default()
                });
            } else {
                let path = std::fs::canonicalize(&a).unwrap_or_else(|_| PathBuf::from(&a));
                if path.is_file() {
                    tracks.push(crate::local::read_track(&path));
                } else {
                    self.toast(format!("Can't open {a}"), ToastKind::Error);
                }
            }
        }
        if !tracks.is_empty() {
            self.player.play_list(tracks, 0);
            self.page = Page::NowPlaying;
        }
    }

    pub fn toast(&mut self, text: impl Into<String>, kind: ToastKind) {
        self.toasts.push(Toast::new(text, kind));
    }

    /// Everything a page needs, borrowed field by field, plus the page
    /// states.
    pub fn split(&mut self) -> (View<'_>, &mut States) {
        let current = self.player.current().map(Track::key);
        (
            View {
                thumbs: &mut self.thumbs,
                library: &self.library,
                downloads: &self.downloads,
                client: &self.client,
                current,
                playing: self.player.is_playing(),
                actions: &mut self.actions,
                ctx: self.ctx.clone(),
            },
            &mut self.pages,
        )
    }

    // ------------------------------------------------------------------ theme

    /// Apply the appearance settings (and album art colors over them).
    pub fn apply_theme(&mut self) {
        self.theme.apply(&self.config.appearance);
        self.theme_applied();
    }

    /// After the theme manager (re)applied the appearance settings.
    fn theme_applied(&mut self) {
        if let Some(e) = self.theme.take_error() {
            self.toast(e, ToastKind::Error);
        }
        self.palette = self.theme.palette().clone();
        self.apply_art_palette();
    }

    /// With album art colors on, paint over the theme with a palette
    /// seeded from the playing song's cover.
    fn apply_art_palette(&mut self) {
        let Some(seed) = self.art_seed.filter(|_| self.config.album_art_colors) else { return };
        let dark = match self.config.appearance.scheme {
            ColorScheme::Light => false,
            ColorScheme::Dark => true,
            ColorScheme::System => self.system_dark.unwrap_or(true),
        };
        let palette = M3Palette::from_seed(&to_hex(seed), dark);
        style::apply(&self.ctx, &palette);
        widgets::set_palette(&self.ctx, palette.clone());
        self.palette = palette;
    }

    fn poll_theme(&mut self) {
        if self.theme.poll(&self.config.appearance) {
            self.theme_applied();
        }
        let system_dark = self.ctx.system_theme().map(|t| t == egui::Theme::Dark);
        let mut repaint_art = false;
        if system_dark != self.system_dark {
            self.system_dark = system_dark;
            repaint_art = true;
        }
        if self.config.album_art_colors {
            let seed = self
                .player
                .current()
                .and_then(|t| t.thumbnail.clone())
                .and_then(|u| self.thumbs.seed(&u));
            if seed.is_some() && seed != self.art_seed {
                self.art_seed = seed;
                repaint_art = true;
            }
        }
        if repaint_art {
            self.apply_art_palette();
        }
    }

    // ------------------------------------------------------------- background

    fn cookies(&self) -> Option<PathBuf> {
        let auth = self.client.auth()?;
        auth.cookie()?;
        let path = crate::yt::auth::cookies_path();
        path.is_file().then_some(path)
    }

    fn poll_background(&mut self) {
        self.thumbs.poll();
        self.player.poll();
        for n in self.player.take_notices() {
            match n {
                Notice::Error(e) => self.toast(e, ToastKind::Error),
                Notice::Info(i) => self.toast(i, ToastKind::Info),
            }
        }
        for (msg, err) in self.library.poll() {
            self.toast(msg, if err { ToastKind::Error } else { ToastKind::Success });
        }
        for id in std::mem::take(&mut self.library.changed_playlists) {
            self.pages.details.remove(&id);
        }
        self.library.maybe_sync(&self.ctx, &self.client, self.config.sync_minutes);
        self.local.poll();
        let cookies = self.cookies();
        for (msg, err) in self.downloads.poll(&self.config, cookies) {
            self.toast(msg, if err { ToastKind::Error } else { ToastKind::Success });
        }
        self.poll_collections();
        self.sync_mpris();
        if self.player.is_playing() || self.downloads.active() > 0 || self.library.syncing() {
            self.ctx.request_repaint_after(Duration::from_millis(250));
        }
    }

    fn poll_collections(&mut self) {
        let mut done = Vec::new();
        for (i, (task, _)) in self.collection_jobs.iter_mut().enumerate() {
            if task.poll() {
                done.push(i);
            }
        }
        for i in done.into_iter().rev() {
            let (mut task, job) = self.collection_jobs.remove(i);
            match (task.value.take(), job) {
                (Some(Ok(tracks)), CollectionJob::Play { shuffle }) if !tracks.is_empty() => {
                    self.player.set_shuffle(shuffle);
                    let start = if shuffle { fastrand::usize(..tracks.len()) } else { 0 };
                    self.player.play_list(tracks, start);
                }
                (Some(Ok(tracks)), CollectionJob::Download) => {
                    let n = self.downloads.enqueue(&tracks);
                    self.toast(format!("Downloading {n} songs"), ToastKind::Info);
                }
                (Some(Ok(_)), _) => self.toast("Nothing to play", ToastKind::Info),
                (Some(Err(e)), _) => self.toast(e.to_string(), ToastKind::Error),
                (None, _) => {}
            }
        }
    }

    fn sync_mpris(&mut self) {
        let p = &self.player;
        let st = &p.state;
        let snap = match p.current() {
            Some(t) => mpris::Snapshot {
                status: if st.idle && st.time_pos <= 0.0 {
                    "Stopped"
                } else if st.paused {
                    "Paused"
                } else {
                    "Playing"
                },
                track_id: p.generation,
                title: t.title.clone(),
                artists: t.artists.iter().map(|a| a.name.clone()).collect(),
                album: t.album.as_ref().map(|a| a.name.clone()).unwrap_or_default(),
                art_url: t.thumbnail.as_deref().map(crate::thumbs::upscale).unwrap_or_default(),
                url: t
                    .watch_url()
                    .or_else(|| t.path.as_ref().map(|p| format!("file://{}", p.display())))
                    .unwrap_or_default(),
                length_us: (st.duration * 1e6) as i64,
                position_us: (st.time_pos * 1e6) as i64,
                volume: p.volume / 100.0,
                can_next: true,
                can_previous: true,
                shuffle: p.shuffle,
                loop_status: match p.repeat {
                    Repeat::Off => "None",
                    Repeat::All => "Playlist",
                    Repeat::One => "Track",
                },
            },
            None => mpris::Snapshot {
                status: "Stopped",
                ..Default::default()
            },
        };
        // Report jumps (seeks) so widgets resync their progress bars.
        let jumped = (st.time_pos - self.last_seek_signal).abs() > 2.0;
        self.last_seek_signal = st.time_pos;
        if jumped {
            self.mpris.seeked(snap.position_us);
        }
        self.mpris.update(snap);
        let commands: Vec<mpris::Command> = self.mpris.commands.try_iter().collect();
        for c in commands {
            match c {
                mpris::Command::PlayPause => self.player.toggle_pause(),
                mpris::Command::Play => {
                    if self.player.state.paused || self.player.state.idle {
                        self.player.toggle_pause();
                    }
                }
                mpris::Command::Pause => self.player.set_pause(true),
                mpris::Command::Stop => self.player.set_pause(true),
                mpris::Command::Next => self.player.next(),
                mpris::Command::Previous => self.player.previous(),
                mpris::Command::Seek(off) => {
                    let to = self.player.state.time_pos + off as f64 / 1e6;
                    self.player.seek(to.max(0.0));
                }
                mpris::Command::SetPosition(pos) => self.player.seek(pos as f64 / 1e6),
                mpris::Command::SetVolume(v) => self.player.set_volume(v * 100.0),
                mpris::Command::SetShuffle(s) => self.player.set_shuffle(s),
                mpris::Command::SetLoop(l) => {
                    self.player.repeat = match l.as_str() {
                        "Track" => Repeat::One,
                        "Playlist" => Repeat::All,
                        _ => Repeat::Off,
                    }
                }
                mpris::Command::Raise => self.ctx.send_viewport_cmd(egui::ViewportCommand::Focus),
            }
        }
    }

    // ---------------------------------------------------------------- actions

    fn goto(&mut self, page: Page) {
        if page != self.page {
            let prev = std::mem::replace(&mut self.page, page);
            self.history.push(prev);
            if self.history.len() > 50 {
                self.history.remove(0);
            }
        }
    }

    fn fetch_collection(&mut self, id: String, play_id: Option<String>, is_artist: bool, job: CollectionJob) {
        // Use an already-loaded page when there is one.
        if !is_artist {
            if let Some(Ok(d)) = self.pages.details.get(&id).and_then(|s| s.detail.value.as_ref()) {
                if d.continuation.is_none() && !d.tracks.is_empty() {
                    let tracks = d.tracks.clone();
                    let mut t = Task::default();
                    t.spawn(&self.ctx, move || Ok(tracks));
                    self.collection_jobs.push((t, job));
                    return;
                }
            }
        }
        let client = self.client.clone();
        let mut t = Task::default();
        t.spawn(&self.ctx, move || {
            if is_artist {
                let d = client.detail(&id)?;
                return match d.radio_id {
                    Some(r) => client.radio(None, Some(&r)).map(|(t, _)| t),
                    None => Ok(d.tracks),
                };
            }
            if let (Some(pl), CollectionJob::Play { .. }) = (play_id.as_deref(), job) {
                if let Ok((tracks, _)) = client.radio(None, Some(pl)) {
                    if !tracks.is_empty() {
                        return Ok(tracks);
                    }
                }
            }
            let d = client.detail(&id)?;
            let mut tracks = d.tracks;
            let mut token = d.continuation;
            for _ in 0..10 {
                let Some(tok) = token.take() else { break };
                let (more, next) = client.more_tracks(&tok)?;
                tracks.extend(more);
                token = next;
            }
            Ok(tracks)
        });
        self.collection_jobs.push((t, job));
    }

    fn process(&mut self, action: Action) {
        match action {
            Action::Goto(page) => self.goto(page),
            Action::Back => {
                if let Some(p) = self.history.pop() {
                    self.page = p;
                }
            }
            Action::Toast(text, kind) => self.toast(text, kind),
            Action::Install(pkg) => {
                let cmd = arch::install_command(&[&pkg], AurHelper::Auto);
                match term::run(&self.config.terminal, &cmd) {
                    Ok(t) => self.toast(format!("Installing {pkg} in {t}"), ToastKind::Info),
                    Err(e) => self.toast(e, ToastKind::Error),
                }
            }
            Action::Play(tracks, start) => self.player.play_list(tracks, start),
            Action::PlayNext(tracks) => {
                let n = tracks.len();
                self.player.play_next(tracks);
                self.toast(if n == 1 { "Playing next".to_string() } else { format!("{n} songs playing next") }, ToastKind::Info);
            }
            Action::Enqueue(tracks) => {
                let n = tracks.len();
                self.player.enqueue(tracks);
                self.toast(if n == 1 { "Added to queue".to_string() } else { format!("Added {n} songs to queue") }, ToastKind::Info);
            }
            Action::StartRadio(t) => self.player.start_radio(t),
            Action::PlayCollection { collection, shuffle } => {
                let is_artist = collection.kind == Kind::Artist;
                self.fetch_collection(collection.id, collection.play_id, is_artist, CollectionJob::Play { shuffle });
            }
            Action::ToggleLike(t) => {
                if self.client.signed_in() {
                    self.library.toggle_like(&self.ctx, &self.client, &t);
                } else {
                    self.toast("Sign in to like songs (Settings → Account)", ToastKind::Info);
                }
            }
            Action::AddToPlaylist(pl, tracks) => self.library.add_to_playlist(&self.ctx, &self.client, &pl, tracks),
            Action::NewPlaylist(tracks) => self.new_playlist = Some((String::new(), tracks)),
            Action::RemoveFromPlaylist(pl, t) => self.library.remove_from_playlist(&self.ctx, &self.client, &pl, &t),
            Action::Download(tracks) => {
                let n = self.downloads.enqueue(&tracks);
                if n == 0 {
                    self.toast("Already downloaded", ToastKind::Info);
                } else if n > 1 {
                    self.toast(format!("Downloading {n} songs"), ToastKind::Info);
                }
            }
            Action::DownloadCollection(id) => {
                let is_artist = id.starts_with("UC");
                self.fetch_collection(id, None, is_artist, CollectionJob::Download);
            }
            Action::RemoveDownload(id) => match self.downloads.remove(&id) {
                Ok(()) => self.toast("Download removed", ToastKind::Info),
                Err(e) => self.toast(e, ToastKind::Error),
            },
            Action::OpenPath(path) => {
                if let Err(e) = material_3_ui::sys::cmd::spawn_detached("xdg-open", &[&path.to_string_lossy()]) {
                    self.toast(e, ToastKind::Error);
                }
            }
            Action::OpenUrl(url) => {
                if let Err(e) = material_3_ui::sys::cmd::spawn_detached("xdg-open", &[&url]) {
                    self.toast(e, ToastKind::Error);
                }
            }
            Action::CopyText(text) => {
                self.ctx.copy_text(text);
                self.toast("Copied", ToastKind::Info);
            }
            Action::SyncNow => self.library.sync_now(&self.ctx, &self.client),
            Action::ConfigChanged => {
                if let Err(e) = self.config.save() {
                    self.toast(format!("Couldn't save preferences: {e}"), ToastKind::Error);
                }
                self.player.autoplay = self.config.autoplay;
                if self.player.mpv_args != self.config.mpv_args {
                    self.player.mpv_args = self.config.mpv_args.clone();
                    // mpv only reads options at startup.
                    self.player.reset_engine();
                }
                self.local.change_folders(self.config.folders());
                self.art_seed = None;
                self.apply_theme();
            }
        }
    }

    fn process_actions(&mut self) {
        while !self.actions.is_empty() {
            for a in std::mem::take(&mut self.actions) {
                self.process(a);
            }
        }
    }

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use egui::{Key, Modifiers};
        let typing = ctx.memory(|m| m.focused().is_some());
        if !typing && take_key(ctx, Modifiers::NONE, Key::Space) {
            self.player.toggle_pause();
        }
        if take_key(ctx, Modifiers::COMMAND, Key::ArrowRight) {
            self.player.next();
        }
        if take_key(ctx, Modifiers::COMMAND, Key::ArrowLeft) {
            self.player.previous();
        }
        if take_key(ctx, Modifiers::COMMAND, Key::F) {
            self.pages.search.focus = true;
            self.actions.push(Action::Goto(Page::Search));
        }
        if ctx.input(|i| i.pointer.button_pressed(egui::PointerButton::Extra1)) || take_key(ctx, Modifiers::ALT, Key::ArrowLeft) {
            self.actions.push(Action::Back);
        }
    }

    fn new_playlist_dialog(&mut self) {
        let Some((name, tracks)) = &mut self.new_playlist else { return };
        let mut close = false;
        let mut create = false;
        let p = self.palette.clone();
        egui::Modal::new(egui::Id::unique("new-playlist")).show(&self.ctx, |ui| {
            ui.set_width(360.0);
            ui.label(widgets::title_text("New playlist", 20.0).color(p.on_surface));
            ui.add_space(8.0);
            let n = tracks.len();
            if n > 0 {
                ui.label(egui::RichText::new(format!("With {n} song{}", if n == 1 { "" } else { "s" })).color(p.on_surface_variant));
            }
            ui.add_space(8.0);
            let r = ui.add(egui::TextEdit::singleline(name).hint_text("Name").desired_width(f32::INFINITY));
            r.request_focus();
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !name.trim().is_empty() {
                create = true;
            }
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if widgets::button_enabled(ui, widgets::ButtonKind::Filled, None, "Create", !name.trim().is_empty()).clicked() {
                        create = true;
                    }
                    if widgets::button(ui, widgets::ButtonKind::Text, None, "Cancel").clicked() {
                        close = true;
                    }
                });
            });
            if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                close = true;
            }
        });
        if create {
            if let Some((name, tracks)) = self.new_playlist.take() {
                if self.client.signed_in() {
                    self.library.create_playlist(&self.ctx, &self.client, name.trim().to_string(), tracks);
                } else {
                    self.toast("Sign in to create playlists", ToastKind::Info);
                }
            }
        } else if close {
            self.new_playlist = None;
        }
    }

    fn layout(&mut self, ui: &mut egui::Ui) {
        let p = self.palette.clone();
        let surface = Frame::new().fill(p.surface);

        egui::Panel::bottom("now-playing")
            .show_separator_line(false)
            .frame(surface.inner_margin(Margin {
                left: 12,
                right: 12,
                top: 0,
                bottom: 12,
            }))
            .show(ui, |ui| ui::nowbar::show(ui, self));

        egui::Panel::left("sidebar")
            .resizable(true)
            .default_size(256.0)
            .size_range(210.0..=340.0)
            .show_separator_line(false)
            .frame(surface.inner_margin(Margin {
                left: 12,
                right: 4,
                top: 12,
                bottom: 8,
            }))
            .show(ui, |ui| ui::sidebar::show(ui, self));

        egui::CentralPanel::default()
            .frame(surface.inner_margin(Margin {
                left: 8,
                right: 12,
                top: 12,
                bottom: 8,
            }))
            .show(ui, |ui| {
                Frame::new()
                    .fill(p.surface_container_low)
                    .corner_radius(CornerRadius::same(24))
                    .show(ui, |ui| {
                        ui.set_min_size(ui.available_size());
                        pages::show(ui, self);
                    });
            });
    }
}

/// `https://music.youtube.com/watch?v=ID`, `youtu.be/ID`, …
pub fn video_id_from_url(s: &str) -> Option<String> {
    let s = s.trim();
    let id = if let Some(i) = s.find("v=") {
        &s[i + 2..]
    } else {
        s.split("youtu.be/").nth(1)?
    };
    let id: String = id.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_').collect();
    (id.len() == 11).then_some(id)
}

impl eframe::App for App {
    fn logic(&mut self, _ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_background();
        self.poll_theme();
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = self.ctx.clone();
        self.shortcuts(&ctx);
        self.layout(ui);
        self.new_playlist_dialog();
        let fired = self.toasts.show(&ctx, &self.palette, 20.0);
        self.actions.extend(fired);
        self.process_actions();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        self.palette.surface.to_normalized_gamma_f32()
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.config.volume = self.player.volume;
        let _ = self.config.save();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(video_id_from_url("https://music.youtube.com/watch?v=dQw4w9WgXcQ&list=x").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(video_id_from_url("https://youtu.be/dQw4w9WgXcQ").as_deref(), Some("dQw4w9WgXcQ"));
        assert_eq!(video_id_from_url("/home/me/song.mp3"), None);
    }
}

