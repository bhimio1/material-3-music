//! Track rows, context menus, cards and carousels shared by every page.

use super::{art, Action, Actions, Page};
use material_3_ui::widgets;
use crate::downloads::{Downloads, Status};
use crate::library::Library;
use crate::thumbs::Thumbs;
use crate::yt::model::{format_duration, Collection, Item, Kind, Shelf, Track};
use crate::yt::Client;
use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Id, Rect, RichText, Sense, Ui};
use material_3_ui::icons::{icons::*, MaterialIcon};

/// What pages need from the app, borrowed field by field.
pub struct View<'a> {
    pub thumbs: &'a mut Thumbs,
    pub library: &'a Library,
    pub downloads: &'a Downloads,
    pub client: &'a Client,
    /// Key of the playing track.
    pub current: Option<String>,
    pub playing: bool,
    pub actions: &'a mut Actions,
    pub ctx: egui::Context,
}

impl View<'_> {
    pub fn push(&mut self, a: Action) {
        self.actions.push(a);
    }
}

#[derive(Clone, Copy, Default)]
pub struct ListOpts<'a> {
    /// Rows can be removed from this (editable) playlist.
    pub playlist_id: Option<&'a str>,
    /// Show track numbers instead of art (albums).
    pub numbered: bool,
    pub hide_album: bool,
}

/// The ⋮ menu / right-click menu for a song.
pub fn track_menu(ui: &mut Ui, v: &mut View, track: &Track, opts: ListOpts) {
    let one = || vec![track.clone()];
    if widgets::menu_item(ui, Some(ICON_QUEUE_PLAY_NEXT), "Play next", "", false) {
        v.push(Action::PlayNext(one()));
    }
    if widgets::menu_item(ui, Some(ICON_ADD_TO_QUEUE), "Add to queue", "", false) {
        v.push(Action::Enqueue(one()));
    }
    if track.video_id.is_some() {
        if widgets::menu_item(ui, Some(ICON_RADIO), "Start radio", "", false) {
            v.push(Action::StartRadio(track.clone()));
        }
        ui.separator();
        if v.client.signed_in() {
            let liked = v.library.is_liked(track);
            let (icon, label) = if liked { (ICON_HEART_BROKEN, "Remove from liked") } else { (ICON_FAVORITE, "Like") };
            if widgets::menu_item(ui, Some(icon), label, "", false) {
                v.push(Action::ToggleLike(track.clone()));
            }
            ui.menu_button(
                (widgets::icon_text(ICON_PLAYLIST_ADD, 18.0).color(widgets::palette(ui).on_surface_variant), "Add to playlist"),
                |ui| {
                    if widgets::menu_item(ui, Some(ICON_ADD), "New playlist…", "", false) {
                        v.push(Action::NewPlaylist(one()));
                    }
                    ui.separator();
                    for pl in v.library.own_playlists() {
                        if widgets::menu_item(ui, Some(ICON_QUEUE_MUSIC), &pl.title, "", false) {
                            v.actions.push(Action::AddToPlaylist(pl.clone(), one()));
                        }
                    }
                },
            );
        }
        let id = track.video_id.clone().unwrap_or_default();
        if v.downloads.is_downloaded(&id) {
            if widgets::menu_item(ui, Some(ICON_DELETE), "Remove download", "", false) {
                v.push(Action::RemoveDownload(id.clone()));
            }
        } else if v.downloads.job_for(&id).is_none() && widgets::menu_item(ui, Some(ICON_DOWNLOAD), "Download", "", false) {
            v.push(Action::Download(one()));
        }
    }
    let album = track.album.as_ref().and_then(|a| a.id.clone());
    let artists: Vec<_> = track.artists.iter().filter(|a| a.id.is_some()).cloned().collect();
    if album.is_some() || !artists.is_empty() {
        ui.separator();
    }
    if let Some(id) = album {
        if widgets::menu_item(ui, Some(ICON_ALBUM), "Go to album", "", false) {
            v.push(Action::Goto(Page::Detail(id)));
        }
    }
    for a in artists {
        let label = if track.artists.len() > 1 { format!("Go to {}", a.name) } else { "Go to artist".into() };
        if widgets::menu_item(ui, Some(ICON_PERSON), &label, "", false) {
            v.push(Action::Goto(Page::Detail(a.id.clone().unwrap_or_default())));
        }
    }
    if let (Some(pl), Some(_)) = (opts.playlist_id, &track.set_video_id) {
        ui.separator();
        if widgets::menu_item(ui, Some(ICON_PLAYLIST_REMOVE), "Remove from this playlist", "", false) {
            v.push(Action::RemoveFromPlaylist(pl.to_string(), track.clone()));
        }
    }
    if let Some(url) = track.watch_url() {
        if widgets::menu_item(ui, Some(ICON_LINK), "Copy link", "", false) {
            v.push(Action::CopyText(url));
        }
    }
    if let Some(path) = &track.path {
        if widgets::menu_item(ui, Some(ICON_FOLDER_OPEN), "Show in folder", "", false) {
            v.push(Action::OpenPath(path.parent().map(|p| p.to_path_buf()).unwrap_or_default()));
        }
    }
}

fn subtitle(track: &Track) -> String {
    let mut s = track.artist_line();
    if track.is_video {
        s = if s.is_empty() { "Video".into() } else { format!("Video • {s}") };
    }
    s
}

/// Rows of songs. Clicking one plays the whole list from there.
pub fn track_list(ui: &mut Ui, v: &mut View, id: impl std::hash::Hash + std::fmt::Debug, tracks: &[Track], opts: ListOpts) {
    let p = widgets::palette(ui);
    let width = ui.available_width();
    let show_album = !opts.hide_album && width > 720.0 && tracks.iter().any(|t| t.album.is_some());
    let base = Id::unique(("tracks", id));
    for (i, track) in tracks.iter().enumerate() {
        let (rect, _) = ui.allocate_exact_size(vec2(width, 56.0), Sense::hover());
        if !ui.is_rect_visible(rect) {
            continue;
        }
        let row = ui.interact(rect, base.with(i), Sense::click());
        let is_current = v.current.as_deref() == Some(track.key().as_str());
        let hovered = row.hovered() || row.context_menu_opened();
        let fill = if is_current {
            p.secondary_container.gamma_multiply(0.6)
        } else if hovered {
            p.on_surface.gamma_multiply(0.06)
        } else {
            Color32::TRANSPARENT
        };
        ui.painter().rect_filled(rect, CornerRadius::same(12), fill);

        let lead = Rect::from_min_size(pos2(rect.left() + 8.0, rect.center().y - 20.0), vec2(40.0, 40.0));
        if opts.numbered {
            let label = if is_current && v.playing {
                None
            } else {
                Some(track.track_no.map(|n| n as usize).unwrap_or(i + 1).to_string())
            };
            match label {
                Some(n) if !hovered => {
                    ui.painter().text(lead.center(), Align2::CENTER_CENTER, n, FontId::proportional(14.0), p.on_surface_variant);
                }
                _ => widgets::paint_icon(ui, if is_current && v.playing { ICON_EQUALIZER } else { ICON_PLAY_ARROW }, lead.center(), 22.0, p.primary),
            }
        } else {
            art::paint(ui, v.thumbs, track.thumbnail.as_deref(), lead, 6, ICON_MUSIC_NOTE);
            if hovered || is_current {
                ui.painter().rect_filled(lead, CornerRadius::same(6), Color32::from_black_alpha(110));
                let icon = if is_current && v.playing { ICON_EQUALIZER } else { ICON_PLAY_ARROW };
                widgets::paint_icon(ui, icon, lead.center(), 22.0, Color32::WHITE);
            }
        }

        // Trailing controls, right to left: menu, duration, like, download.
        let mut x = rect.right() - 8.0;
        let more_rect = Rect::from_center_size(pos2(x - 18.0, rect.center().y), vec2(36.0, 36.0));
        x -= 40.0;
        let dur = track.duration.map(|d| format_duration(d as f64)).unwrap_or_default();
        ui.painter().text(pos2(x - 4.0, rect.center().y), Align2::RIGHT_CENTER, &dur, FontId::proportional(13.0), p.on_surface_variant);
        x -= 52.0;
        let liked = v.library.is_liked(track);
        let heart_rect = Rect::from_center_size(pos2(x - 16.0, rect.center().y), vec2(32.0, 32.0));
        let can_like = track.video_id.is_some() && v.client.signed_in();
        x -= 36.0;
        let dl_center = pos2(x - 12.0, rect.center().y);
        x -= 28.0;

        let text_left = lead.right() + 14.0;
        let album_w = if show_album { (width * 0.28).min(280.0) } else { 0.0 };
        let text_w = (x - text_left - album_w - 8.0).max(40.0);
        let title_color = if is_current { p.primary } else { p.on_surface };
        let g = widgets::truncated_galley(ui, &track.title, widgets::title_font(14.0), title_color, text_w);
        let sub = subtitle(track);
        if sub.is_empty() {
            ui.painter().galley(pos2(text_left, rect.center().y - g.size().y / 2.0), g, title_color);
        } else {
            ui.painter().galley(pos2(text_left, rect.center().y - 17.0), g, title_color);
            let s = widgets::truncated_galley(ui, &sub, FontId::proportional(12.5), p.on_surface_variant, text_w);
            ui.painter().galley(pos2(text_left, rect.center().y + 2.0), s, p.on_surface_variant);
        }
        if show_album {
            if let Some(al) = &track.album {
                let g = widgets::truncated_galley(ui, &al.name, FontId::proportional(13.0), p.on_surface_variant, album_w - 12.0);
                ui.painter().galley(pos2(x - album_w, rect.center().y - g.size().y / 2.0), g, p.on_surface_variant);
            }
        }

        // Download state.
        if let Some(vid) = track.video_id.as_deref() {
            if v.downloads.is_downloaded(vid) {
                widgets::paint_icon(ui, ICON_DOWNLOAD_DONE, dl_center, 18.0, p.primary);
            } else if let Some(job) = v.downloads.job_for(vid) {
                let frac = match &job.status {
                    Status::Running { percent, .. } => percent / 100.0,
                    Status::Converting => 1.0,
                    _ => 0.0,
                };
                ui.painter().circle_stroke(dl_center, 9.0, egui::Stroke::new(2.0, p.secondary_container));
                let n = 24;
                let pts: Vec<_> = (0..=((n as f32 * frac) as usize))
                    .map(|k| {
                        let a = -std::f32::consts::FRAC_PI_2 + k as f32 / n as f32 * std::f32::consts::TAU;
                        dl_center + vec2(a.cos(), a.sin()) * 9.0
                    })
                    .collect();
                if pts.len() > 1 {
                    ui.painter().add(egui::Shape::line(pts, egui::Stroke::new(2.0, p.primary)));
                }
            }
        } else if track.path.is_some() {
            widgets::paint_icon(ui, ICON_FOLDER, dl_center, 16.0, p.on_surface_variant.gamma_multiply(0.7));
        }

        if can_like && (hovered || liked) {
            let r = ui.interact(heart_rect, base.with((i, "like")), Sense::click());
            if r.hovered() {
                ui.painter().circle_filled(heart_rect.center(), 16.0, p.on_surface.gamma_multiply(0.08));
            }
            let icon = ICON_FAVORITE;
            let col = if liked { p.primary } else { p.on_surface_variant };
            paint_outline_aware(ui, icon, heart_rect.center(), 20.0, col, liked);
            if r.on_hover_text(if liked { "Remove from liked" } else { "Like" }).clicked() {
                v.push(Action::ToggleLike(track.clone()));
            }
        }
        let more = ui.interact(more_rect, base.with((i, "more")), Sense::click());
        if hovered || more.hovered() {
            if more.hovered() {
                ui.painter().circle_filled(more_rect.center(), 18.0, p.on_surface.gamma_multiply(0.08));
            }
            widgets::paint_icon(ui, ICON_MORE_VERT, more_rect.center(), 20.0, p.on_surface_variant);
        }
        egui::Popup::menu(&more).show(|ui| track_menu(ui, v, track, opts));
        row.context_menu(|ui| track_menu(ui, v, track, opts));
        if row.clicked() && !more.clicked() {
            v.push(Action::Play(tracks.to_vec(), i));
        }
    }
}

/// Heart icons: the outline glyph must stay outlined even with filled icons.
fn paint_outline_aware(ui: &Ui, icon: MaterialIcon, at: egui::Pos2, size: f32, color: Color32, filled: bool) {
    let glyph = if filled { icon.filled() } else { icon.outlined() };
    ui.painter().text(at, Align2::CENTER_CENTER, glyph.codepoint, FontId::new(size, glyph.font_family()), color);
}

fn collection_icon(kind: Kind) -> MaterialIcon {
    match kind {
        Kind::Album | Kind::Single => ICON_ALBUM,
        Kind::Artist => ICON_PERSON,
        Kind::Playlist => ICON_QUEUE_MUSIC,
        Kind::Podcast => ICON_PODCASTS,
    }
}

pub fn collection_menu(ui: &mut Ui, v: &mut View, c: &Collection) {
    if widgets::menu_item(ui, Some(ICON_PLAY_ARROW), "Play", "", false) {
        v.push(Action::PlayCollection { collection: c.clone(), shuffle: false });
    }
    if widgets::menu_item(ui, Some(ICON_SHUFFLE), "Shuffle", "", false) {
        v.push(Action::PlayCollection { collection: c.clone(), shuffle: true });
    }
    // Artists and podcast shows can be huge; download their songs or
    // episodes one by one instead.
    if !matches!(c.kind, Kind::Artist | Kind::Podcast) && widgets::menu_item(ui, Some(ICON_DOWNLOAD), "Download", "", false) {
        v.push(Action::DownloadCollection(c.id.clone()));
    }
    if widgets::menu_item(ui, Some(ICON_OPEN_IN_NEW), "Open", "", false) {
        v.push(Action::Goto(Page::Detail(c.id.clone())));
    }
}

/// A square card (round for artists) with a hover play button.
pub fn card(ui: &mut Ui, v: &mut View, item: &Item, width: f32, id: Id) {
    let p = widgets::palette(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, width + 56.0), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let resp = ui.interact(rect, id, Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let art_rect = Rect::from_min_size(rect.min, vec2(width, width));
    let (thumb, round, icon, title, sub) = match item {
        Item::Track(t) => (t.thumbnail.as_deref(), 12, ICON_MUSIC_NOTE, t.title.as_str(), subtitle(t)),
        Item::Collection(c) => (
            c.thumbnail.as_deref(),
            if c.kind == Kind::Artist { (width / 2.0) as u8 } else { 12 },
            collection_icon(c.kind),
            c.title.as_str(),
            c.subtitle.clone(),
        ),
    };
    art::paint(ui, v.thumbs, thumb, art_rect, round, icon);
    let hovered = resp.hovered() || resp.context_menu_opened();
    let mut fab_clicked = false;
    if hovered {
        ui.painter().rect_filled(art_rect, CornerRadius::same(round), Color32::from_black_alpha(50));
        let fab = Rect::from_center_size(art_rect.right_bottom() - vec2(30.0, 30.0), vec2(44.0, 44.0));
        let fr = ui.interact(fab, id.with("play"), Sense::click());
        let fill = widgets::state_layer(p.primary_container, p.on_primary_container, &fr);
        ui.painter().circle_filled(fab.center(), 22.0, fill);
        widgets::paint_icon(ui, ICON_PLAY_ARROW, fab.center(), 26.0, p.on_primary_container);
        fab_clicked = fr.clicked();
    }
    let tg = widgets::truncated_galley(ui, title, widgets::title_font(14.0), p.on_surface, width - 4.0);
    ui.painter().galley(pos2(rect.left() + 2.0, art_rect.bottom() + 8.0), tg, p.on_surface);
    let sg = widgets::truncated_galley(ui, &sub, FontId::proportional(12.5), p.on_surface_variant, width - 4.0);
    ui.painter().galley(pos2(rect.left() + 2.0, art_rect.bottom() + 28.0), sg, p.on_surface_variant);

    match item {
        Item::Track(t) => {
            resp.context_menu(|ui| track_menu(ui, v, t, ListOpts::default()));
            if resp.clicked() || fab_clicked {
                v.push(Action::StartRadio(t.clone()));
            }
        }
        Item::Collection(c) => {
            resp.context_menu(|ui| collection_menu(ui, v, c));
            if fab_clicked {
                v.push(Action::PlayCollection { collection: c.clone(), shuffle: false });
            } else if resp.clicked() {
                v.push(Action::Goto(Page::Detail(c.id.clone())));
            }
        }
    }
}

pub fn shelf_header(ui: &mut Ui, v: &mut View, title: &str, more: Option<&str>) {
    let p = widgets::palette(ui);
    ui.add_space(20.0);
    ui.horizontal(|ui| {
        ui.label(widgets::title_text(title, 20.0).color(p.on_surface));
        if let Some(id) = more {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::button(ui, widgets::ButtonKind::Text, None, "More").clicked() {
                    v.push(Action::Goto(Page::Detail(id.to_string())));
                }
            });
        }
    });
    ui.add_space(8.0);
}

/// A shelf: songs as a compact grid of rows, everything else as cards in a
/// horizontal carousel.
pub fn shelf(ui: &mut Ui, v: &mut View, s: &Shelf, salt: usize) {
    shelf_header(ui, v, &s.title, s.more.as_deref().filter(|m| m.starts_with("VL") || m.starts_with("UC") || m.starts_with("MPREb")));
    let songs: Vec<Track> = s
        .items
        .iter()
        .filter_map(|i| match i {
            Item::Track(t) if !t.is_video => Some(t.clone()),
            _ => None,
        })
        .collect();
    if songs.len() == s.items.len() && songs.len() > 3 {
        // Quick-picks style: columns of four rows, scrolling sideways.
        egui::ScrollArea::horizontal()
            .id_salt(("shelf", salt, &s.title))
            .show(ui, |ui| {
                ui.horizontal_top(|ui| {
                    for (c, chunk) in songs.chunks(4).enumerate() {
                        ui.vertical(|ui| {
                            ui.set_width(340.0);
                            track_list(ui, v, ("qp", salt, c), chunk, ListOpts { hide_album: true, ..Default::default() });
                        });
                    }
                });
            });
        return;
    }
    egui::ScrollArea::horizontal()
        .id_salt(("shelf", salt, &s.title))
        .show(ui, |ui| {
            ui.horizontal_top(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                for (i, item) in s.items.iter().enumerate() {
                    card(ui, v, item, 168.0, Id::unique(("card", salt, &s.title, i)));
                }
            });
        });
}

/// A wrapping grid of cards (library pages, artist discographies).
pub fn grid(ui: &mut Ui, v: &mut View, salt: &str, items: &[Item]) {
    let w = ui.available_width();
    let card_w = 168.0;
    let gap = 16.0;
    let cols = ((w + gap) / (card_w + gap)).floor().max(1.0) as usize;
    let card_w = (w - gap * (cols as f32 - 1.0)) / cols as f32;
    for (r, row) in items.chunks(cols).enumerate() {
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = gap;
            for (c, item) in row.iter().enumerate() {
                card(ui, v, item, card_w.min(220.0), Id::unique(("grid", salt, r * cols + c)));
            }
        });
        ui.add_space(8.0);
    }
}

/// Muted centered text for empty states.
pub fn empty(ui: &mut Ui, icon: MaterialIcon, title: &str, body: &str) {
    let p = widgets::palette(ui);
    ui.add_space(48.0);
    ui.vertical_centered(|ui| {
        widgets::icon(ui, icon, 48.0, p.on_surface_variant.gamma_multiply(0.6));
        ui.add_space(8.0);
        ui.label(widgets::title_text(title, 18.0).color(p.on_surface));
        ui.add_space(4.0);
        ui.label(RichText::new(body).color(p.on_surface_variant));
    });
}
