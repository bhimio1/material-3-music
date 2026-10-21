//! Navigation drawer: search, pages, the user's playlists, and the
//! account / sync status.

use material_3_ui::widgets::{self, nav_item, section_header};
use super::{Action, LibTab, Page};
use crate::app::App;
use crate::yt::model::Item;
use egui::{vec2, Sense, Ui};
use material_3_ui::icons::icons::*;

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let mut actions = Vec::new();
    let current = app.page.clone();

    ui.horizontal(|ui| {
        ui.add_space(12.0);
        widgets::icon(ui, ICON_MUSIC_NOTE, 26.0, p.primary);
        ui.label(widgets::title_text("Music", 20.0).color(p.on_surface));
    });
    ui.add_space(14.0);

    let width = ui.available_width() - 4.0;
    let search = &mut app.pages.search;
    let r = material_3_ui::common::search_field(ui, &mut search.query, "Search songs, albums, artists", width);
    if search.focus {
        search.focus = false;
        r.request_focus();
    }
    if r.changed() {
        search.query_changed();
        if current != Page::Search {
            actions.push(Action::Goto(Page::Search));
        }
    }
    if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
        search.submit();
        actions.push(Action::Goto(Page::Search));
    }
    ui.add_space(8.0);

    let bottom_h = 64.0;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .max_height(ui.available_height() - bottom_h)
        .show(ui, |ui| {
            let mut item = |ui: &mut Ui, icon, label: &str, page: Page, trailing: Option<&str>| {
                if nav_item(ui, icon, label, current == page, trailing).clicked() {
                    actions.push(Action::Goto(page));
                }
            };
            item(ui, ICON_HOME, "Home", Page::Home, None);
            item(ui, ICON_SEARCH, "Search", Page::Search, None);
            section_header(ui, "Library");
            item(ui, ICON_QUEUE_MUSIC, "Playlists", Page::Library(LibTab::Playlists), None);
            let liked = app.library.data.liked.len();
            let liked_s = (liked > 0).then(|| liked.to_string());
            item(ui, ICON_FAVORITE, "Liked songs", Page::Library(LibTab::Liked), liked_s.as_deref());
            item(ui, ICON_ALBUM, "Albums", Page::Library(LibTab::Albums), None);
            item(ui, ICON_PERSON, "Artists", Page::Library(LibTab::Artists), None);
            item(ui, ICON_HISTORY, "History", Page::Library(LibTab::History), None);
            section_header(ui, "On this device");
            let local = app.local.tracks.len();
            let local_s = (local > 0).then(|| local.to_string());
            item(ui, ICON_LIBRARY_MUSIC, "Local files", Page::Local, local_s.as_deref());
            let active = app.downloads.active();
            let active_s = (active > 0).then(|| active.to_string());
            item(ui, ICON_DOWNLOAD, "Downloads", Page::Downloads, active_s.as_deref());

            let playlists: Vec<_> = app
                .library
                .data
                .playlists
                .iter()
                .filter_map(|i| match i {
                    Item::Collection(c) => Some(c.clone()),
                    _ => None,
                })
                .collect();
            if !playlists.is_empty() {
                section_header(ui, "Your playlists");
                for c in playlists {
                    let page = Page::Detail(c.id.clone());
                    let r = nav_item(ui, ICON_QUEUE_MUSIC, &c.title, current == page, None);
                    if r.clicked() {
                        actions.push(Action::Goto(page));
                    }
                    r.context_menu(|ui| {
                        if widgets::menu_item(ui, Some(ICON_PLAY_ARROW), "Play", "", false) {
                            actions.push(Action::PlayCollection { collection: c.clone(), shuffle: false });
                        }
                        if widgets::menu_item(ui, Some(ICON_SHUFFLE), "Shuffle", "", false) {
                            actions.push(Action::PlayCollection { collection: c.clone(), shuffle: true });
                        }
                        if widgets::menu_item(ui, Some(ICON_DOWNLOAD), "Download", "", false) {
                            actions.push(Action::DownloadCollection(c.id.clone()));
                        }
                    });
                }
            }
        });

    // Account chip at the bottom.
    ui.add_space(4.0);
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 52.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let fill = widgets::state_layer(p.surface_container, p.on_surface, &resp);
    ui.painter().rect_filled(rect, egui::CornerRadius::same(26), fill);
    let avatar = egui::Rect::from_center_size(rect.left_center() + vec2(26.0, 0.0), vec2(36.0, 36.0));
    let signed_in = app.client.signed_in();
    let account = app.library.data.account.clone();
    match account.as_ref().and_then(|a| a.photo.clone()).filter(|_| signed_in) {
        Some(url) => super::art::paint(ui, &mut app.thumbs, Some(&url), avatar, 18, ICON_PERSON),
        None => {
            ui.painter().circle_filled(avatar.center(), 18.0, p.primary_container);
            widgets::paint_icon(ui, ICON_PERSON, avatar.center(), 20.0, p.on_primary_container);
        }
    }
    let (title, sub) = if signed_in {
        (
            account.map(|a| a.name).filter(|n| !n.is_empty()).unwrap_or_else(|| "Signed in".into()),
            if app.library.syncing() { "Syncing…".to_string() } else { app.library.last_synced() },
        )
    } else {
        ("Not signed in".to_string(), "Sign in to sync your library".to_string())
    };
    let tx = avatar.right() + 10.0;
    let max = rect.right() - tx - 40.0;
    let tg = widgets::truncated_galley(ui, &title, widgets::title_font(13.5), p.on_surface, max);
    ui.painter().galley(egui::pos2(tx, rect.center().y - 16.0), tg, p.on_surface);
    let sg = widgets::truncated_galley(ui, &sub, egui::FontId::proportional(11.5), p.on_surface_variant, max);
    ui.painter().galley(egui::pos2(tx, rect.center().y + 1.0), sg, p.on_surface_variant);
    let icon_c = rect.right_center() - vec2(24.0, 0.0);
    if app.library.syncing() {
        widgets::paint_icon(ui, ICON_SYNC, icon_c, 20.0, p.primary);
    } else {
        widgets::paint_icon(ui, ICON_SETTINGS, icon_c, 20.0, p.on_surface_variant);
    }
    if resp.clicked() {
        actions.push(Action::Goto(Page::Settings));
    }
    app.actions.extend(actions);
}
