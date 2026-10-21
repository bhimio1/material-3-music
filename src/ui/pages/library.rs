//! The synced library: playlists, liked songs, albums, artists, history.

use super::scroll;
use material_3_ui::widgets;
use crate::app::App;
use material_3_ui::common;
use crate::ui::tracks::{self, empty, ListOpts};
use crate::ui::{Action, LibTab, Page};
use crate::yt::model::{Item, Track};
use egui::{RichText, Ui};
use material_3_ui::icons::icons::*;

#[derive(Default)]
pub struct State {
    filter: String,
}

const TABS: [LibTab; 5] = [LibTab::Playlists, LibTab::Liked, LibTab::Albums, LibTab::Artists, LibTab::History];

fn matches(text: &str, filter: &str) -> bool {
    filter.is_empty() || text.to_lowercase().contains(&filter.to_lowercase())
}

pub fn show(ui: &mut Ui, app: &mut App, tab: LibTab) {
    let p = app.palette.clone();
    let signed_in = app.client.signed_in();
    let syncing = app.library.syncing();
    let synced = app.library.last_synced();
    let (mut v, states) = app.split();
    let state = &mut states.library;
    scroll(ui, "library", |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Library").size(28.0).color(p.on_surface));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if signed_in {
                    if common::refresh_button(ui, syncing) {
                        v.push(Action::SyncNow);
                    }
                    ui.label(RichText::new(&synced).size(12.5).color(p.on_surface_variant));
                }
            });
        });
        ui.add_space(8.0);
        ui.horizontal_wrapped(|ui| {
            for t in TABS {
                if widgets::filter_chip(ui, t.label(), t == tab, None).clicked() && t != tab {
                    v.push(Action::Goto(Page::Library(t)));
                }
            }
        });
        if !signed_in {
            ui.add_space(12.0);
            common::banner(ui, ICON_SYNC, "Sign in to sync your playlists, liked songs, albums and artists from YouTube Music.", false, |ui| {
                if widgets::button(ui, widgets::ButtonKind::Filled, Some(ICON_LOGIN), "Sign in").clicked() {
                    v.push(Action::Goto(Page::Settings));
                }
            });
        }
        ui.add_space(12.0);
        ui.horizontal(|ui| {
            common::search_field(ui, &mut state.filter, &format!("Filter {}", tab.label().to_lowercase()), 320.0);
        });
        ui.add_space(8.0);
        let f = state.filter.trim().to_string();
        let data = &v.library.data;
        match tab {
            LibTab::Liked | LibTab::History => {
                let list: Vec<Track> = if tab == LibTab::Liked { &data.liked } else { &data.history }
                    .iter()
                    .filter(|t| matches(&t.title, &f) || matches(&t.artist_line(), &f))
                    .cloned()
                    .collect();
                if list.is_empty() {
                    let (icon, title) = if tab == LibTab::Liked { (ICON_FAVORITE, "No liked songs") } else { (ICON_HISTORY, "No history") };
                    empty(ui, icon, title, if signed_in { "Songs show up here after a sync." } else { "Sign in to see them here." });
                    return;
                }
                if tab == LibTab::Liked {
                    ui.horizontal(|ui| {
                        if widgets::button(ui, widgets::ButtonKind::Filled, Some(ICON_PLAY_ARROW), "Play").clicked() {
                            v.push(Action::Play(list.clone(), 0));
                        }
                        if widgets::button(ui, widgets::ButtonKind::Tonal, Some(ICON_SHUFFLE), "Shuffle").clicked() {
                            v.push(Action::Play(shuffled(list.clone()), 0));
                        }
                        if widgets::button(ui, widgets::ButtonKind::Outlined, Some(ICON_DOWNLOAD), "Download all").clicked() {
                            v.push(Action::Download(list.clone()));
                        }
                    });
                    ui.add_space(8.0);
                }
                tracks::track_list(ui, &mut v, ("lib", tab.label()), &list, ListOpts::default());
            }
            LibTab::Playlists | LibTab::Albums | LibTab::Artists => {
                let src = match tab {
                    LibTab::Playlists => &data.playlists,
                    LibTab::Albums => &data.albums,
                    _ => &data.artists,
                };
                let items: Vec<Item> = src
                    .iter()
                    .filter(|i| match i {
                        Item::Collection(c) => matches(&c.title, &f) || matches(&c.subtitle, &f),
                        Item::Track(t) => matches(&t.title, &f),
                    })
                    .cloned()
                    .collect();
                if tab == LibTab::Playlists && signed_in {
                    ui.horizontal(|ui| {
                        if widgets::button(ui, widgets::ButtonKind::Tonal, Some(ICON_ADD), "New playlist").clicked() {
                            v.push(Action::NewPlaylist(Vec::new()));
                        }
                    });
                    ui.add_space(8.0);
                }
                if items.is_empty() {
                    empty(ui, ICON_LIBRARY_MUSIC, &format!("No {}", tab.label().to_lowercase()), "Sync your library or save some from search.");
                } else {
                    tracks::grid(ui, &mut v, tab.label(), &items);
                }
            }
        }
    });
}

pub fn shuffled(mut tracks: Vec<Track>) -> Vec<Track> {
    fastrand::shuffle(&mut tracks);
    tracks
}
