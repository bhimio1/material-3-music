//! Downloads: active jobs with progress, and songs saved for offline use.

use super::scroll;
use material_3_ui::widgets;
use crate::app::App;
use crate::downloads::Status;
use material_3_ui::common;
use crate::ui::tracks::{self, empty, ListOpts};
use crate::ui::{art, Action};
use crate::yt::model::Track;
use egui::{vec2, CornerRadius, Frame, Margin, RichText, Ui};
use material_3_ui::icons::icons::*;

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let root = app.config.download_root();
    let mut cancel = None;
    let mut retry = None;
    let mut clear = false;
    let jobs: Vec<(u64, Track, Status)> = app.downloads.jobs.iter().map(|j| (j.id, j.track.clone(), j.status.clone())).collect();
    let mut saved: Vec<(i64, Track, u64)> = app
        .downloads
        .records
        .values()
        .map(|r| {
            let mut t = r.track.clone();
            t.path = None;
            (r.at, t, std::fs::metadata(&r.path).map(|m| m.len()).unwrap_or(0))
        })
        .collect();
    saved.sort_by_key(|s| std::cmp::Reverse(s.0));
    let total: u64 = saved.iter().map(|s| s.2).sum();
    let (mut v, _) = app.split();
    scroll(ui, "downloads", |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Downloads").size(28.0).color(p.on_surface));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::button(ui, widgets::ButtonKind::Text, Some(ICON_FOLDER_OPEN), "Open folder").clicked() {
                    let _ = std::fs::create_dir_all(&root);
                    v.push(Action::OpenPath(root.clone()));
                }
                if jobs.iter().any(|j| j.2.finished()) && widgets::button(ui, widgets::ButtonKind::Text, Some(ICON_CLEAR_ALL), "Clear finished").clicked() {
                    clear = true;
                }
            });
        });
        ui.label(RichText::new(format!("{} songs • {} • saved to {}", saved.len(), material_3_ui::format::file_size(total), material_3_ui::sys::paths::tilde(&root))).color(p.on_surface_variant));
        crate::ui::require(ui, v.actions, "yt-dlp", "yt-dlp", "Downloads use yt-dlp.");
        let active: Vec<_> = jobs.iter().filter(|j| !matches!(j.2, Status::Done(_))).collect();
        if !active.is_empty() {
            common::card(ui, "In progress", |ui| {
                for (id, t, status) in active {
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), egui::Sense::hover());
                        art::paint(ui, v.thumbs, t.thumbnail.as_deref(), rect, 8, ICON_MUSIC_NOTE);
                        ui.vertical(|ui| {
                            ui.set_width((ui.available_width() - 140.0).max(120.0));
                            ui.label(widgets::title_text(&t.title, 14.0).color(p.on_surface));
                            let (frac, line) = match status {
                                Status::Queued => (None, "Waiting…".to_string()),
                                Status::Running { percent, speed, eta } => (Some(percent / 100.0), format!("{percent:.0}% • {speed} • {eta} left")),
                                Status::Converting => (None, "Converting and tagging…".into()),
                                Status::Failed(e) => (Some(0.0), format!("Failed: {e}")),
                                Status::Cancelled => (Some(0.0), "Cancelled".into()),
                                Status::Done(_) => (Some(1.0), "Done".into()),
                            };
                            let color = if matches!(status, Status::Failed(_)) { p.error } else { p.on_surface_variant };
                            ui.label(RichText::new(line).size(12.5).color(color));
                            if !matches!(status, Status::Failed(_) | Status::Cancelled) {
                                let w = ui.available_width();
                                widgets::linear_progress(ui, frac, w);
                            }
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| match status {
                            Status::Failed(_) | Status::Cancelled => {
                                if widgets::button(ui, widgets::ButtonKind::Tonal, Some(ICON_REFRESH), "Retry").clicked() {
                                    retry = Some(*id);
                                }
                            }
                            _ => {
                                if widgets::icon_button(ui, ICON_CLOSE, "Cancel", true).clicked() {
                                    cancel = Some(*id);
                                }
                            }
                        });
                    });
                    ui.add_space(6.0);
                }
            });
        }
        ui.add_space(12.0);
        if saved.is_empty() {
            empty(ui, ICON_DOWNLOAD, "No downloads yet", "Use Download on a song, album or playlist to listen offline.");
            return;
        }
        let list: Vec<Track> = saved.into_iter().map(|s| s.1).collect();
        tracks::shelf_header(ui, &mut v, "Saved for offline", None);
        ui.horizontal(|ui| {
            if widgets::button(ui, widgets::ButtonKind::Filled, Some(ICON_PLAY_ARROW), "Play all").clicked() {
                v.push(Action::Play(list.clone(), 0));
            }
            if widgets::button(ui, widgets::ButtonKind::Tonal, Some(ICON_SHUFFLE), "Shuffle").clicked() {
                v.push(Action::Play(super::library::shuffled(list.clone()), 0));
            }
        });
        ui.add_space(8.0);
        Frame::new().corner_radius(CornerRadius::same(16)).inner_margin(Margin::ZERO).show(ui, |ui| {
            tracks::track_list(ui, &mut v, "saved", &list, ListOpts::default());
        });
    });
    if let Some(id) = cancel {
        app.downloads.cancel(id);
    }
    if let Some(id) = retry {
        app.downloads.retry(id);
    }
    if clear {
        app.downloads.clear_finished();
    }
}
