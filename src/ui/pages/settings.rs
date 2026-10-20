//! Settings: account (sign-in and sync), library folders, downloads,
//! playback and appearance.

use super::{scroll, widgets};
use crate::app::App;
use crate::config::{AudioFormat, ColorScheme, ThemeSource};
use crate::sys::task::Task;
use crate::theme::palette::{parse_hex, to_hex};
use crate::ui::common::{self, card, row};
use crate::ui::widgets::{segmented, switch, ButtonKind};
use crate::ui::{Action, ToastKind};
use crate::yt::auth::{self, DeviceCode, Poll};
use egui::{vec2, Color32, CornerRadius, Frame, Margin, RichText, Ui};
use egui_material_icons::icons::*;
use std::time::{Duration, Instant};

const SEED_PRESETS: &[&str] = &["#4285F4", "#6750A4", "#B3261E", "#E8710A", "#1E8E3E", "#00897B", "#C2185B", "#795548", "#607D8B"];

#[derive(Default)]
enum SignIn {
    #[default]
    Idle,
    Requesting(Task<anyhow::Result<DeviceCode>>),
    Waiting {
        code: DeviceCode,
        poll: Task<Poll>,
        next: Instant,
        interval: u64,
        started: Instant,
    },
}

#[derive(Default)]
pub struct State {
    signin: SignIn,
    show_guide: bool,
    browser_input: String,
    new_folder: String,
    seed_text: Option<String>,
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let mut actions: Vec<Action> = Vec::new();
    let mut changed = false;
    let mut signed_in_now: Option<auth::Auth> = None;
    let mut sign_out = false;
    let theme_file = app.theme_file();
    let st = &mut app.pages.settings;
    let config = &mut app.config;
    let client = app.client.clone();
    let ctx = app.ctx.clone();

    // Drive the device flow.
    if let SignIn::Requesting(task) = &mut st.signin {
        if task.poll() {
            match task.value.take() {
                Some(Ok(code)) => {
                    ctx.copy_text(code.user_code.clone());
                    let interval = code.interval.max(1);
                    st.signin = SignIn::Waiting { code, poll: Task::default(), next: Instant::now() + Duration::from_secs(interval), interval, started: Instant::now() };
                }
                Some(Err(e)) => {
                    actions.push(Action::Toast(e.to_string(), ToastKind::Error));
                    st.signin = SignIn::Idle;
                }
                None => {}
            }
        }
    }
    let mut cancel = false;
    if let SignIn::Waiting { code, poll, next, interval, started } = &mut st.signin {
        if poll.poll() {
            match poll.value.take() {
                Some(Poll::Done(a)) => signed_in_now = Some(a),
                Some(Poll::Failed(e)) => {
                    actions.push(Action::Toast(e, ToastKind::Error));
                    cancel = true;
                }
                Some(Poll::SlowDown) => *interval += 5,
                _ => {}
            }
            *next = Instant::now() + Duration::from_secs(*interval);
        }
        let expired = code.expires_in > 0 && started.elapsed() > Duration::from_secs(code.expires_in);
        if expired {
            actions.push(Action::Toast("The sign-in code expired".into(), ToastKind::Error));
            cancel = true;
        } else if !poll.running() && Instant::now() >= *next {
            let (c, id, secret, dc) = (client.clone(), config.oauth_client_id.clone(), config.oauth_client_secret.clone(), code.device_code.clone());
            poll.spawn(&ctx, move || auth::poll_device(c.agent(), &id, &secret, &dc));
        }
        ctx.request_repaint_after(Duration::from_millis(500));
    }
    if cancel {
        st.signin = SignIn::Idle;
    }

    scroll(ui, "settings", |ui| {
        ui.set_max_width(ui.available_width().min(common::PAGE_WIDTH));
        ui.horizontal(|ui| {
            widgets::icon(ui, ICON_SETTINGS, 30.0, p.primary);
            ui.label(RichText::new("Settings").size(28.0).color(p.on_surface));
        });

        // ---------------------------------------------------------- account
        card(ui, "Account", |ui| {
            match client.auth() {
                Some(a) => {
                    let name = app.library.data.account.as_ref().map(|a| a.name.clone()).filter(|n| !n.is_empty());
                    row(ui, &name.unwrap_or_else(|| "Signed in".into()), &format!("{} • {}", a.label(), app.library.last_synced()), |ui| {
                        if widgets::button(ui, ButtonKind::Text, Some(ICON_LOGOUT), "Sign out").clicked() {
                            sign_out = true;
                        }
                        if widgets::button_enabled(ui, ButtonKind::Tonal, Some(ICON_SYNC), "Sync now", !app.library.syncing()).clicked() {
                            actions.push(Action::SyncNow);
                        }
                    });
                    if !app.library.data.errors.is_empty() {
                        common::note(ui, &format!("Last sync had problems: {}", app.library.data.errors.join("; ")));
                    }
                    ui.separator();
                    row(ui, "Sync every", "Your library also syncs when the app starts.", |ui| {
                        let mut m = config.sync_minutes;
                        let r = ui.add(egui::Slider::new(&mut m, 0..=120).custom_formatter(|v, _| if v == 0.0 { "Off".into() } else { format!("{v:.0} min") }));
                        if r.changed() {
                            config.sync_minutes = m;
                        }
                        changed |= r.drag_stopped() || (r.changed() && !r.dragged());
                    });
                }
                None => {
                    common::note(ui, "Sign in to sync your playlists, liked songs, albums, subscriptions and history, and to like songs and edit playlists from here. Search and playback work without an account.");
                    // OAuth.
                    ui.add_space(4.0);
                    ui.label(widgets::title_text("Sign in with Google", 15.0).color(p.on_surface));
                    match &st.signin {
                        SignIn::Waiting { code, .. } => {
                            ui.add_space(8.0);
                            Frame::new().fill(p.surface_container_highest).corner_radius(CornerRadius::same(16)).inner_margin(Margin::same(16)).show(ui, |ui| {
                                ui.set_width(ui.available_width());
                                ui.label(RichText::new(format!("Go to {} and enter this code:", code.verification_url)).color(p.on_surface_variant));
                                ui.add_space(4.0);
                                ui.label(RichText::new(&code.user_code).size(34.0).monospace().color(p.primary));
                                ui.add_space(8.0);
                                ui.horizontal(|ui| {
                                    if widgets::button(ui, ButtonKind::Filled, Some(ICON_OPEN_IN_NEW), "Open google.com/device").clicked() {
                                        actions.push(Action::OpenUrl(code.verification_url.clone()));
                                    }
                                    if widgets::button(ui, ButtonKind::Tonal, Some(ICON_CONTENT_COPY), "Copy code").clicked() {
                                        actions.push(Action::CopyText(code.user_code.clone()));
                                    }
                                    if widgets::button(ui, ButtonKind::Text, None, "Cancel").clicked() {
                                        cancel = true;
                                    }
                                    ui.add(egui::Spinner::new().color(p.primary));
                                    ui.label(RichText::new("Waiting for you to approve…").color(p.on_surface_variant));
                                });
                            });
                        }
                        _ => {
                            ui.add_space(4.0);
                            ui.label(RichText::new("Uses an OAuth client from your own Google Cloud project (type “TVs and Limited Input devices”).").size(12.5).color(p.on_surface_variant));
                            ui.add_space(6.0);
                            egui::Grid::new("oauth").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                                ui.label("Client ID");
                                changed |= ui.add(egui::TextEdit::singleline(&mut config.oauth_client_id).desired_width(420.0).hint_text("123-abc.apps.googleusercontent.com")).lost_focus();
                                ui.end_row();
                                ui.label("Client secret");
                                changed |= ui.add(egui::TextEdit::singleline(&mut config.oauth_client_secret).password(true).desired_width(420.0)).lost_focus();
                                ui.end_row();
                            });
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                let ready = !config.oauth_client_id.trim().is_empty() && !config.oauth_client_secret.trim().is_empty();
                                let busy = matches!(st.signin, SignIn::Requesting(_));
                                if widgets::button_enabled(ui, ButtonKind::Filled, Some(ICON_LOGIN), "Sign in with Google", ready && !busy).clicked() {
                                    let (c, id) = (client.clone(), config.oauth_client_id.trim().to_string());
                                    config.oauth_client_secret = config.oauth_client_secret.trim().to_string();
                                    config.oauth_client_id = id.clone();
                                    changed = true;
                                    let mut t = Task::default();
                                    t.spawn(&ctx, move || auth::start_device_flow(c.agent(), &id));
                                    st.signin = SignIn::Requesting(t);
                                }
                                if busy {
                                    ui.add(egui::Spinner::new().color(p.primary));
                                }
                                let label = if st.show_guide { "Hide setup steps" } else { "How do I get a client?" };
                                if widgets::button(ui, ButtonKind::Text, Some(ICON_HELP), label).clicked() {
                                    st.show_guide = !st.show_guide;
                                }
                            });
                            if st.show_guide {
                                common::note(ui, "1. Open console.cloud.google.com and create a project.\n2. APIs & Services → Library → enable “YouTube Data API v3”.\n3. OAuth consent screen → External → add yourself as a test user.\n4. Credentials → Create credentials → OAuth client ID → type “TVs and Limited Input devices”.\n5. Paste the client ID and secret above, then Sign in with Google and enter the code shown at google.com/device.");
                                if widgets::button(ui, ButtonKind::Text, Some(ICON_OPEN_IN_NEW), "Open Google Cloud console").clicked() {
                                    actions.push(Action::OpenUrl("https://console.cloud.google.com/apis/credentials".into()));
                                }
                            }
                        }
                    }
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);
                    // Browser session.
                    ui.label(widgets::title_text("Use your browser session", 15.0).color(p.on_surface));
                    ui.add(egui::Label::new(RichText::new("Open music.youtube.com signed in, open developer tools (F12) → Network, click any request to “browse”, copy its request headers (or just the Cookie value) and paste them here.").size(12.5).color(p.on_surface_variant)).wrap());
                    ui.add_space(6.0);
                    ui.add(egui::TextEdit::multiline(&mut st.browser_input).desired_rows(4).desired_width(f32::INFINITY).hint_text("cookie: …; SAPISID=…; …").font(egui::TextStyle::Monospace));
                    ui.add_space(6.0);
                    if widgets::button_enabled(ui, ButtonKind::Tonal, Some(ICON_COOKIE), "Sign in with browser session", !st.browser_input.trim().is_empty()).clicked() {
                        match auth::parse_browser_input(&st.browser_input) {
                            Ok(a) => {
                                signed_in_now = Some(a);
                                st.browser_input.clear();
                            }
                            Err(e) => actions.push(Action::Toast(e.to_string(), ToastKind::Error)),
                        }
                    }
                    ui.add_space(8.0);
                }
            }
        });
        if cancel {
            st.signin = SignIn::Idle;
        }

        // ---------------------------------------------------------- library
        card(ui, "Library", |ui| {
            common::note(ui, "Folders scanned for music on this computer. Changes to files are picked up automatically.");
            let mut remove = None;
            for (i, f) in config.library_folders.iter().enumerate() {
                ui.horizontal(|ui| {
                    widgets::icon(ui, ICON_FOLDER, 20.0, p.on_surface_variant);
                    ui.label(f);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::icon_button_sized(ui, ICON_DELETE, "Remove", true, 30.0, false).clicked() {
                            remove = Some(i);
                        }
                    });
                });
            }
            if let Some(i) = remove {
                config.library_folders.remove(i);
                changed = true;
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut st.new_folder).hint_text("~/Music/More").desired_width(320.0));
                let path = crate::sys::paths::expand(st.new_folder.trim());
                let valid = !st.new_folder.trim().is_empty() && path.is_dir();
                if widgets::button_enabled(ui, ButtonKind::Tonal, Some(ICON_ADD), "Add folder", valid).clicked() {
                    config.library_folders.push(st.new_folder.trim().to_string());
                    st.new_folder.clear();
                    changed = true;
                }
                if !st.new_folder.trim().is_empty() && !valid {
                    ui.label(RichText::new("Not a folder").color(p.error));
                }
            });
            ui.add_space(8.0);
        });

        // -------------------------------------------------------- downloads
        card(ui, "Downloads", |ui| {
            row(ui, "Format", "Opus is smallest at the same quality; MP3 plays everywhere.", |ui| {
                changed |= segmented(ui, &mut config.download_format, &[(AudioFormat::Opus, ICON_GRAPHIC_EQ, "Opus"), (AudioFormat::M4a, ICON_GRAPHIC_EQ, "M4A"), (AudioFormat::Mp3, ICON_GRAPHIC_EQ, "MP3")]);
            });
            ui.separator();
            row(ui, "Download folder", &format!("Songs go in <folder>/<artist>/<album>/. Now: {}", crate::sys::paths::tilde(&config.download_root())), |ui| {
                changed |= ui.add(egui::TextEdit::singleline(&mut config.download_dir).hint_text("~/Music/M3 Music").desired_width(220.0)).lost_focus();
            });
            ui.separator();
            row(ui, "Simultaneous downloads", "", |ui| {
                let r = ui.add(egui::Slider::new(&mut config.concurrent_downloads, 1..=6));
                changed |= r.drag_stopped() || (r.changed() && !r.dragged());
            });
            common::require(ui, &mut actions, "yt-dlp", "yt-dlp", "Downloads and YouTube playback use yt-dlp.");
        });

        // --------------------------------------------------------- playback
        card(ui, "Playback", |ui| {
            row(ui, "Autoplay", "When the queue ends, keep playing related songs.", |ui| {
                changed |= switch(ui, &mut config.autoplay).changed();
            });
            ui.separator();
            row(ui, "mpv options", "Extra options for the audio engine, separated by spaces (e.g. --af=loudnorm). Changing them pauses playback; press play to continue.", |ui| {
                let id = egui::Id::unique("mpv-args-text");
                let mut text = ui.data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| config.mpv_args.join(" "));
                let r = ui.add(egui::TextEdit::singleline(&mut text).desired_width(220.0).hint_text("--ao=pipewire"));
                if r.lost_focus() {
                    config.mpv_args = text.split_whitespace().map(str::to_string).collect();
                    changed = true;
                }
                ui.data_mut(|d| d.insert_temp(id, text));
            });
            common::require(ui, &mut actions, "mpv", "mpv", "Music plays through mpv.");
        });

        // ------------------------------------------------------- appearance
        card(ui, "Appearance", |ui| {
            let desc = match (config.theme_source, &theme_file) {
                (ThemeSource::Auto, Some(f)) => format!("Using your desktop's colors from {}.", crate::sys::paths::tilde(f)),
                (ThemeSource::Auto, None) => "No end-4 / end4-pC colors found, so the seed color below is used.".into(),
                (ThemeSource::AlbumArt, _) => "Colors follow the cover of the song that's playing.".into(),
                (ThemeSource::Seed, _) => "Using a palette generated from the seed color.".into(),
            };
            row(ui, "Color source", &desc, |ui| {
                changed |= segmented(ui, &mut config.theme_source, &[(ThemeSource::Auto, ICON_AUTO_AWESOME, "Desktop"), (ThemeSource::AlbumArt, ICON_ALBUM, "Album art"), (ThemeSource::Seed, ICON_PALETTE, "Seed")]);
            });
            ui.separator();
            row(ui, "Color scheme", "Light, dark, or follow the system.", |ui| {
                changed |= segmented(ui, &mut config.color_scheme, &[(ColorScheme::System, ICON_BRIGHTNESS_AUTO, "System"), (ColorScheme::Light, ICON_LIGHT_MODE, "Light"), (ColorScheme::Dark, ICON_DARK_MODE, "Dark")]);
            });
            ui.separator();
            row(ui, "Seed color", "Every color in the interface is derived from this one.", |ui| {
                let text = st.seed_text.get_or_insert_with(|| config.seed_color.clone());
                if ui.add(egui::TextEdit::singleline(text).desired_width(92.0).char_limit(7)).changed() {
                    if let Some(c) = parse_hex(text) {
                        config.seed_color = to_hex(c);
                        changed = true;
                    }
                }
                for hex in SEED_PRESETS.iter().rev() {
                    let c = parse_hex(hex).unwrap_or(Color32::GRAY);
                    let (rect, resp) = ui.allocate_exact_size(vec2(24.0, 24.0), egui::Sense::click());
                    ui.painter().circle_filled(rect.center(), 11.0, c);
                    if config.seed_color.eq_ignore_ascii_case(hex) {
                        ui.painter().circle_stroke(rect.center(), 12.5, egui::Stroke::new(2.0, p.on_surface));
                    }
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        config.seed_color = hex.to_string();
                        *text = hex.to_string();
                        changed = true;
                    }
                }
            });
            ui.separator();
            row(ui, "Filled icons", "Use filled Material Symbols instead of outlined ones.", |ui| {
                changed |= switch(ui, &mut config.filled_icons).changed();
            });
            ui.separator();
            row(ui, "Interface scale", "Size of everything in the window.", |ui| {
                let r = ui.add(egui::Slider::new(&mut config.ui_scale, 0.75..=1.75).step_by(0.05).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)));
                changed |= r.drag_stopped() || (r.changed() && !r.dragged());
            });
        });

        card(ui, "About", |ui| {
            let path = crate::config::Config::path().unwrap_or_default();
            row(ui, "Preferences file", &path.to_string_lossy(), |_| {});
            ui.separator();
            row(ui, "Account data", &format!("{} (only you can read it)", auth::data_dir().display()), |_| {});
        });
        ui.add_space(16.0);
        ui.label(RichText::new(format!("Material 3 Music {}", env!("CARGO_PKG_VERSION"))).size(12.0).color(p.on_surface_variant));
    });
    if cancel {
        app.pages.settings.signin = SignIn::Idle;
    }

    if let Some(a) = signed_in_now {
        app.pages.settings.signin = SignIn::Idle;
        match app.client.set_auth(Some(a)) {
            Ok(()) => {
                app.player.reset_engine();
                app.library.sync_now(&app.ctx, &app.client);
                app.pages.home.shelves = Task::default();
                actions.push(Action::Toast("Signed in. Syncing your library…".into(), ToastKind::Success));
            }
            Err(e) => actions.push(Action::Toast(e.to_string(), ToastKind::Error)),
        }
    }
    if sign_out {
        let _ = app.client.set_auth(None);
        app.library.clear();
        app.player.reset_engine();
        app.pages.home.shelves = Task::default();
        actions.push(Action::Toast("Signed out".into(), ToastKind::Info));
    }
    if changed {
        actions.push(Action::ConfigChanged);
    }
    app.actions.extend(actions);
}
