//! Music files on this computer: songs, albums and artists.

use super::scroll;
use material_3_ui::widgets;
use crate::app::App;
use material_3_ui::common;
use crate::ui::tracks::{self, empty, ListOpts};
use crate::ui::{art, Action, Page};
use crate::yt::model::Track;
use egui::{pos2, vec2, FontId, Id, RichText, Sense, Ui};
use material_3_ui::icons::icons::*;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Tab {
    #[default]
    Songs,
    Albums,
    Artists,
}

#[derive(Clone, Copy, Default, PartialEq, Eq)]
pub enum Sort {
    #[default]
    Title,
    Artist,
    Album,
    Duration,
}

#[derive(Default)]
pub struct State {
    tab: Tab,
    sort: Sort,
    filter: String,
    /// Drilled into an album or artist (by name).
    open: Option<(Tab, String)>,
}

fn album_key(t: &Track) -> String {
    t.album.as_ref().map(|a| a.name.clone()).unwrap_or_else(|| "Unknown album".into())
}

fn artist_key(t: &Track) -> String {
    t.artists.first().map(|a| a.name.clone()).unwrap_or_else(|| "Unknown artist".into())
}

/// Album order: disc/track number, then title.
fn album_order(tracks: &mut [Track]) {
    tracks.sort_by(|a, b| a.track_no.unwrap_or(u32::MAX).cmp(&b.track_no.unwrap_or(u32::MAX)).then_with(|| a.title.cmp(&b.title)));
}

/// A card for a local album or artist; returns true when clicked.
fn group_card(ui: &mut Ui, v: &mut tracks::View, title: &str, sub: &str, thumb: Option<&str>, round: bool, id: Id) -> bool {
    let p = widgets::palette(ui);
    let w = 168.0;
    let (rect, _) = ui.allocate_exact_size(vec2(w, w + 56.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return false;
    }
    let resp = ui.interact(rect, id, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let art_rect = egui::Rect::from_min_size(rect.min, vec2(w, w));
    let r = if round { (w / 2.0) as u8 } else { 12 };
    art::paint(ui, v.thumbs, thumb, art_rect, r, if round { ICON_PERSON } else { ICON_ALBUM });
    if resp.hovered() {
        ui.painter().rect_filled(art_rect, egui::CornerRadius::same(r), egui::Color32::from_black_alpha(40));
    }
    let g = widgets::truncated_galley(ui, title, widgets::title_font(14.0), p.on_surface, w - 4.0);
    ui.painter().galley(pos2(rect.left() + 2.0, art_rect.bottom() + 8.0), g, p.on_surface);
    let g = widgets::truncated_galley(ui, sub, FontId::proportional(12.5), p.on_surface_variant, w - 4.0);
    ui.painter().galley(pos2(rect.left() + 2.0, art_rect.bottom() + 28.0), g, p.on_surface_variant);
    resp.clicked()
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let scanning = app.local.scanning();
    let folders = app.config.folders();
    let mut rescan = false;
    let all = app.local.tracks.clone();
    let (mut v, states) = app.split();
    let st = &mut states.local;
    scroll(ui, "local", |ui| {
        ui.horizontal(|ui| {
            if st.open.is_some() && widgets::icon_button(ui, ICON_ARROW_BACK, "Back", true).clicked() {
                st.open = None;
            }
            ui.label(RichText::new("Local files").size(28.0).color(p.on_surface));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                rescan = common::refresh_button(ui, scanning);
                if widgets::icon_button(ui, ICON_SETTINGS, "Library folders", true).clicked() {
                    v.push(Action::Goto(Page::Settings));
                }
                if let Some(first) = folders.first() {
                    if widgets::icon_button(ui, ICON_FOLDER_OPEN, "Open music folder", true).clicked() {
                        v.push(Action::OpenPath(first.clone()));
                    }
                }
            });
        });
        let names: Vec<String> = folders.iter().map(|f| material_3_ui::sys::paths::tilde(f)).collect();
        ui.label(RichText::new(format!("{} songs from {}", all.len(), names.join(", "))).color(p.on_surface_variant));
        ui.add_space(8.0);

        if all.is_empty() {
            if scanning {
                common::loading(ui, "Scanning your music…");
            } else {
                empty(ui, ICON_LIBRARY_MUSIC, "No music found", "Add folders in Settings → Library, or download songs.");
            }
            return;
        }

        if let Some((kind, name)) = st.open.clone() {
            let mut list: Vec<Track> = all
                .iter()
                .filter(|t| if kind == Tab::Albums { album_key(t) == name } else { artist_key(t) == name })
                .cloned()
                .collect();
            album_order(&mut list);
            if kind == Tab::Artists {
                list.sort_by_key(album_key);
            }
            ui.label(widgets::title_text(&name, 22.0).color(p.on_surface));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if widgets::button(ui, widgets::ButtonKind::Filled, Some(ICON_PLAY_ARROW), "Play").clicked() {
                    v.push(Action::Play(list.clone(), 0));
                }
                if widgets::button(ui, widgets::ButtonKind::Tonal, Some(ICON_SHUFFLE), "Shuffle").clicked() {
                    v.push(Action::Play(super::library::shuffled(list.clone()), 0));
                }
            });
            ui.add_space(8.0);
            let opts = ListOpts { numbered: kind == Tab::Albums, hide_album: kind == Tab::Albums, ..Default::default() };
            tracks::track_list(ui, &mut v, ("local-open", &name), &list, opts);
            return;
        }

        ui.horizontal(|ui| {
            widgets::segmented(ui, &mut st.tab, &[(Tab::Songs, ICON_MUSIC_NOTE, "Songs"), (Tab::Albums, ICON_ALBUM, "Albums"), (Tab::Artists, ICON_PERSON, "Artists")]);
            ui.add_space(12.0);
            common::search_field(ui, &mut st.filter, "Filter", 240.0);
        });
        ui.add_space(10.0);
        let f = st.filter.trim().to_lowercase();
        let filtered: Vec<&Track> = all
            .iter()
            .filter(|t| f.is_empty() || t.title.to_lowercase().contains(&f) || t.artist_line().to_lowercase().contains(&f) || album_key(t).to_lowercase().contains(&f))
            .collect();
        match st.tab {
            Tab::Songs => {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("Sort by").color(p.on_surface_variant));
                    for (s, label) in [(Sort::Title, "Title"), (Sort::Artist, "Artist"), (Sort::Album, "Album"), (Sort::Duration, "Length")] {
                        if widgets::filter_chip(ui, label, st.sort == s, None).clicked() {
                            st.sort = s;
                        }
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if widgets::button(ui, widgets::ButtonKind::Tonal, Some(ICON_SHUFFLE), "Shuffle all").clicked() {
                            v.push(Action::Play(super::library::shuffled(filtered.iter().map(|t| (*t).clone()).collect()), 0));
                        }
                    });
                });
                ui.add_space(6.0);
                let mut list: Vec<Track> = filtered.into_iter().cloned().collect();
                match st.sort {
                    Sort::Title => list.sort_by_key(|t| t.title.to_lowercase()),
                    Sort::Artist => list.sort_by_key(|t| (artist_key(t).to_lowercase(), album_key(t), t.track_no)),
                    Sort::Album => list.sort_by_key(|t| (album_key(t).to_lowercase(), t.track_no)),
                    Sort::Duration => list.sort_by_key(|t| t.duration),
                }
                tracks::track_list(ui, &mut v, "local-songs", &list, ListOpts::default());
            }
            Tab::Albums | Tab::Artists => {
                let mut groups: BTreeMap<String, (usize, Option<String>, String)> = BTreeMap::new();
                for t in &filtered {
                    let key = if st.tab == Tab::Albums { album_key(t) } else { artist_key(t) };
                    let e = groups.entry(key).or_insert((0, None, String::new()));
                    e.0 += 1;
                    if e.1.is_none() {
                        e.1 = t.thumbnail.clone();
                    }
                    if e.2.is_empty() {
                        e.2 = if st.tab == Tab::Albums { artist_key(t) } else { String::new() };
                    }
                }
                let w = ui.available_width();
                let cols = ((w + 16.0) / (168.0 + 16.0)).floor().max(1.0) as usize;
                let entries: Vec<_> = groups.into_iter().collect();
                for (r, row) in entries.chunks(cols).enumerate() {
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 16.0;
                        for (c, (name, (count, thumb, artist))) in row.iter().enumerate() {
                            let sub = if artist.is_empty() { format!("{count} songs") } else { format!("{artist} • {count} songs") };
                            if group_card(ui, &mut v, name, &sub, thumb.as_deref(), st.tab == Tab::Artists, Id::unique(("lg", r * cols + c))) {
                                st.open = Some((st.tab, name.clone()));
                            }
                        }
                    });
                    ui.add_space(8.0);
                }
            }
        }
    });
    if rescan {
        app.local.rescan();
    }
}
