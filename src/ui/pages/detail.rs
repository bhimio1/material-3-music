//! Album, playlist and artist pages.

use super::{scroll, widgets};
use crate::app::App;
use crate::sys::task::Task;
use crate::ui::common;
use crate::ui::tracks::{self, menu_item, ListOpts};
use crate::ui::{art, Action};
use crate::yt::model::{Collection, Detail, Item, Kind, Track};
use egui::{vec2, RichText, Ui};
use egui_material_icons::icons::*;

#[derive(Default)]
pub struct State {
    pub detail: Task<anyhow::Result<Detail>>,
    more: Task<anyhow::Result<(Vec<Track>, Option<String>)>>,
    filter: String,
}

fn kind_of(id: &str) -> Kind {
    if id.starts_with("UC") {
        Kind::Artist
    } else if id.starts_with("MPREb_") {
        Kind::Album
    } else {
        Kind::Playlist
    }
}

pub fn show(ui: &mut Ui, app: &mut App, id: &str) {
    let p = app.palette.clone();
    let (mut v, states) = app.split();
    let state = states.details.entry(id.to_string()).or_default();
    let client = v.client.clone();
    let owned = id.to_string();
    state.detail.ensure(&v.ctx, move || client.detail(&owned));

    // Append a finished "load more" page.
    if state.more.poll() {
        match state.more.value.take() {
            Some(Ok((tracks, next))) => {
                if let Some(Ok(d)) = state.detail.value.as_mut() {
                    d.tracks.extend(tracks);
                    d.continuation = next;
                }
            }
            Some(Err(e)) => v.push(Action::Toast(format!("Couldn't load more: {e}"), crate::ui::ToastKind::Error)),
            None => {}
        }
    }

    let kind = kind_of(id);
    let mut reload = false;
    let mut load_more: Option<String> = None;
    scroll(ui, ("detail", id), |ui| {
        if widgets::icon_button(ui, ICON_ARROW_BACK, "Back", true).clicked() {
            v.push(Action::Back);
        }
        let d = match &state.detail.value {
            None => {
                common::loading(ui, "Loading…");
                return;
            }
            Some(Err(e)) => {
                common::banner(ui, ICON_CLOUD_OFF, &format!("Couldn't load this page. {e}"), true, |ui| {
                    if widgets::button(ui, widgets::ButtonKind::Tonal, Some(ICON_REFRESH), "Retry").clicked() {
                        reload = true;
                    }
                });
                return;
            }
            Some(Ok(d)) => d,
        };
        let collection = Collection {
            kind,
            id: id.to_string(),
            title: d.title.clone(),
            subtitle: d.subtitle.clone(),
            thumbnail: d.thumbnail.clone(),
            play_id: d.playlist_id.clone(),
        };

        // Header.
        ui.add_space(8.0);
        ui.horizontal_top(|ui| {
            let size = if ui.available_width() > 700.0 { 220.0 } else { 160.0 };
            let (rect, _) = ui.allocate_exact_size(vec2(size, size), egui::Sense::hover());
            let round = if kind == Kind::Artist { (size / 2.0) as u8 } else { 16 };
            let icon = match kind {
                Kind::Artist => ICON_PERSON,
                Kind::Playlist => ICON_QUEUE_MUSIC,
                _ => ICON_ALBUM,
            };
            art::paint(ui, v.thumbs, d.thumbnail.as_deref(), rect, round, icon);
            ui.add_space(20.0);
            ui.vertical(|ui| {
                ui.set_max_width(ui.available_width());
                let label = match kind {
                    Kind::Artist => "Artist",
                    Kind::Playlist => "Playlist",
                    _ => "Album",
                };
                ui.add_space(8.0);
                ui.label(widgets::title_text(label, 13.0).color(p.primary));
                ui.add(egui::Label::new(RichText::new(&d.title).size(34.0).color(p.on_surface)).wrap());
                if !d.subtitle.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&d.subtitle).color(p.on_surface_variant)).wrap());
                }
                if !d.description.is_empty() {
                    ui.add_space(4.0);
                    let short: String = d.description.chars().take(220).collect();
                    let more = d.description.chars().count() > 220;
                    ui.add(egui::Label::new(RichText::new(if more { format!("{short}…") } else { short }).size(13.0).color(p.on_surface_variant)).wrap())
                        .on_hover_text(&d.description);
                }
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if widgets::button(ui, widgets::ButtonKind::Filled, Some(ICON_PLAY_ARROW), "Play").clicked() {
                        if kind == Kind::Artist || d.continuation.is_some() || d.tracks.is_empty() {
                            v.push(Action::PlayCollection { collection: collection.clone(), shuffle: false });
                        } else {
                            v.push(Action::Play(d.tracks.clone(), 0));
                        }
                    }
                    if widgets::button(ui, widgets::ButtonKind::Tonal, Some(ICON_SHUFFLE), "Shuffle").clicked() {
                        v.push(Action::PlayCollection { collection: collection.clone(), shuffle: true });
                    }
                    if kind == Kind::Artist {
                        if let Some(first) = d.tracks.first() {
                            if widgets::button(ui, widgets::ButtonKind::Outlined, Some(ICON_RADIO), "Radio").clicked() {
                                v.push(Action::StartRadio(first.clone()));
                            }
                        }
                    } else if widgets::button(ui, widgets::ButtonKind::Outlined, Some(ICON_DOWNLOAD), "Download").clicked() {
                        v.push(Action::DownloadCollection(id.to_string()));
                    }
                    let more = widgets::icon_button(ui, ICON_MORE_VERT, "More", true);
                    egui::Popup::menu(&more).show(|ui| {
                        if menu_item(ui, ICON_QUEUE_PLAY_NEXT, "Play next").clicked() {
                            v.push(Action::PlayNext(d.tracks.clone()));
                            ui.close();
                        }
                        if menu_item(ui, ICON_ADD_TO_QUEUE, "Add to queue").clicked() {
                            v.push(Action::Enqueue(d.tracks.clone()));
                            ui.close();
                        }
                        if v.client.signed_in() && kind != Kind::Artist {
                            ui.menu_button("Add all to playlist", |ui| {
                                if menu_item(ui, ICON_ADD, "New playlist…").clicked() {
                                    v.actions.push(Action::NewPlaylist(d.tracks.clone()));
                                    ui.close();
                                }
                                for pl in v.library.own_playlists() {
                                    if pl.id != id && menu_item(ui, ICON_QUEUE_MUSIC, &pl.title).clicked() {
                                        v.actions.push(Action::AddToPlaylist(pl.clone(), d.tracks.clone()));
                                        ui.close();
                                    }
                                }
                            });
                        }
                        let url = match kind {
                            Kind::Artist => format!("https://music.youtube.com/channel/{id}"),
                            Kind::Playlist => format!("https://music.youtube.com/playlist?list={}", id.trim_start_matches("VL")),
                            _ => format!("https://music.youtube.com/browse/{id}"),
                        };
                        if menu_item(ui, ICON_LINK, "Copy link").clicked() {
                            v.push(Action::CopyText(url.clone()));
                            ui.close();
                        }
                        if menu_item(ui, ICON_OPEN_IN_NEW, "Open in browser").clicked() {
                            v.push(Action::OpenUrl(url));
                            ui.close();
                        }
                    });
                });
            });
        });
        ui.add_space(16.0);

        // Songs.
        if !d.tracks.is_empty() {
            if kind == Kind::Artist {
                let more = d.shelves.first().and_then(|s| s.more.clone());
                tracks::shelf_header(ui, &mut v, "Top songs", more.as_deref());
            } else if d.tracks.len() > 12 {
                common::search_field(ui, &mut state.filter, "Filter songs", 280.0);
                ui.add_space(8.0);
            }
            let f = state.filter.trim().to_lowercase();
            let shown: Vec<Track> = if f.is_empty() {
                d.tracks.clone()
            } else {
                d.tracks
                    .iter()
                    .filter(|t| t.title.to_lowercase().contains(&f) || t.artist_line().to_lowercase().contains(&f))
                    .cloned()
                    .collect()
            };
            let opts = ListOpts {
                playlist_id: if d.editable { d.playlist_id.as_deref() } else { None },
                numbered: kind == Kind::Album,
                hide_album: kind == Kind::Album,
            };
            tracks::track_list(ui, &mut v, ("detail", id), &shown, opts);
            if let Some(token) = &d.continuation {
                ui.add_space(8.0);
                if state.more.running() {
                    common::loading(ui, "Loading more…");
                } else {
                    let r = ui.vertical_centered(|ui| widgets::button(ui, widgets::ButtonKind::Text, None, "Load more")).inner;
                    // Load automatically when scrolled into view.
                    if r.clicked() || ui.is_rect_visible(r.rect) {
                        load_more = Some(token.clone());
                    }
                }
            }
        }

        // Discography and related shelves.
        let skip = usize::from(kind == Kind::Artist && !d.tracks.is_empty());
        for (i, shelf) in d.shelves.iter().enumerate().skip(skip) {
            let only_collections = shelf.items.iter().all(|it| matches!(it, Item::Collection(_)));
            if only_collections || kind != Kind::Artist {
                tracks::shelf(ui, &mut v, shelf, 500 + i);
            }
        }
    });
    if let Some(token) = load_more {
        let client = v.client.clone();
        state.more.spawn(&v.ctx, move || client.more_tracks(&token));
    }
    if reload {
        let client = v.client.clone();
        let owned = id.to_string();
        state.detail.spawn(&v.ctx, move || client.detail(&owned));
    }
}
