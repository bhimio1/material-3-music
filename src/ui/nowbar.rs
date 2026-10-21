//! The bottom bar: current song, transport controls, seek, volume.

use super::{art, Action, Page};
use material_3_ui::widgets;
use crate::app::App;
use crate::player::Repeat;
use crate::yt::model::format_duration;
use egui::{pos2, vec2, Align2, CornerRadius, FontId, Frame, Id, Rect, Response, Sense, Ui, UiBuilder};
use material_3_ui::icons::icons::*;

/// A thin M3 slider. Returns the value while dragging (`live`) or on
/// release, depending on the caller.
fn bar(ui: &mut Ui, id: Id, value: f32, width: f32) -> (Response, Option<f32>) {
    let p = widgets::palette(ui);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 20.0), Sense::hover());
    let resp = ui.interact(rect, id, Sense::click_and_drag());
    let pointer = resp.interact_pointer_pos().map(|pos| ((pos.x - rect.left()) / rect.width()).clamp(0.0, 1.0));
    let shown = if resp.dragged() { pointer.unwrap_or(value) } else { value }.clamp(0.0, 1.0);
    let track = Rect::from_center_size(rect.center(), vec2(rect.width(), 4.0));
    let split = track.left() + track.width() * shown;
    ui.painter().rect_filled(track, CornerRadius::same(2), p.secondary_container);
    ui.painter().rect_filled(
        Rect::from_min_max(track.min, pos2(split, track.max.y)),
        CornerRadius::same(2),
        p.primary,
    );
    let active = resp.hovered() || resp.dragged();
    let knob_r = if active { 7.0 } else { 5.0 };
    ui.painter().circle_filled(pos2(split, rect.center().y), knob_r, p.primary);
    let changed = if resp.dragged() || resp.drag_stopped() || resp.clicked() { pointer } else { None };
    (resp.on_hover_cursor(egui::CursorIcon::PointingHand), changed)
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let mut actions = Vec::new();
    Frame::new()
        .fill(p.surface_container)
        .corner_radius(CornerRadius::same(24))
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            let (full, _) = ui.allocate_exact_size(vec2(ui.available_width(), 72.0), Sense::hover());
            let side_w = (full.width() * 0.3).clamp(200.0, 360.0);
            let left = Rect::from_min_size(full.min, vec2(side_w, full.height()));
            let right = Rect::from_min_max(pos2(full.right() - side_w, full.top()), full.max);
            let center = Rect::from_min_max(pos2(left.right() + 12.0, full.top()), pos2(right.left() - 12.0, full.bottom()));

            let track = app.player.current().cloned();

            // Left: art and titles.
            let lui = ui.new_child(UiBuilder::new().max_rect(left));
            let art_rect = Rect::from_min_size(pos2(left.left(), left.center().y - 28.0), vec2(56.0, 56.0));
            let art_resp = lui.interact(art_rect, Id::unique("nowbar-art"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
            art::paint(&lui, &mut app.thumbs, track.as_ref().and_then(|t| t.thumbnail.as_deref()), art_rect, 10, ICON_MUSIC_NOTE);
            if art_resp.clicked() && track.is_some() {
                actions.push(Action::Goto(Page::NowPlaying));
            }
            let tx = art_rect.right() + 12.0;
            let tw = left.right() - tx - 8.0;
            match &track {
                Some(t) => {
                    let g = widgets::truncated_galley(&lui, &t.title, widgets::title_font(14.5), p.on_surface, tw);
                    let title_rect = Rect::from_min_size(pos2(tx, left.center().y - 19.0), g.size());
                    lui.painter().galley(title_rect.min, g, p.on_surface);
                    let sub = t.artist_line();
                    let g = widgets::truncated_galley(&lui, &sub, FontId::proportional(12.5), p.on_surface_variant, tw);
                    let sub_rect = Rect::from_min_size(pos2(tx, left.center().y + 3.0), g.size());
                    lui.painter().galley(sub_rect.min, g, p.on_surface_variant);
                    if let Some(aid) = t.album.as_ref().and_then(|a| a.id.clone()) {
                        if lui.interact(title_rect, Id::unique("nowbar-title"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            actions.push(Action::Goto(Page::Detail(aid)));
                        }
                    }
                    if let Some(arid) = t.artists.first().and_then(|a| a.id.clone()) {
                        if lui.interact(sub_rect, Id::unique("nowbar-artist"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                            actions.push(Action::Goto(Page::Detail(arid)));
                        }
                    }
                }
                None => {
                    lui.painter().text(pos2(tx, left.center().y), Align2::LEFT_CENTER, "Nothing playing", widgets::title_font(14.0), p.on_surface_variant);
                }
            }

            // Center: transport and seek.
            let mut cui = ui.new_child(UiBuilder::new().max_rect(center));
            let has = track.is_some();
            let controls_w = 5.0 * 40.0 + 16.0 + 8.0;
            let row = Rect::from_center_size(pos2(center.center().x, center.top() + 22.0), vec2(controls_w, 44.0));
            let mut row_ui = cui.new_child(UiBuilder::new().max_rect(row).layout(egui::Layout::left_to_right(egui::Align::Center)));
            row_ui.spacing_mut().item_spacing.x = 4.0;
            let player = &mut app.player;
            if widgets::icon_button_sized(&mut row_ui, ICON_SHUFFLE, "Shuffle", has, 36.0, player.shuffle).clicked() {
                let on = !player.shuffle;
                player.set_shuffle(on);
            }
            if widgets::icon_button_sized(&mut row_ui, ICON_SKIP_PREVIOUS, "Previous (Ctrl+←)", has, 40.0, false).clicked() {
                player.previous();
            }
            // Big play/pause.
            let (pr, presp) = row_ui.allocate_exact_size(vec2(48.0, 48.0), Sense::click());
            let presp = presp.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text("Play/pause (Space)");
            let fill = if has { widgets::state_layer(p.primary, p.on_primary, &presp) } else { p.on_surface.gamma_multiply(0.12) };
            row_ui.painter().rect_filled(pr, CornerRadius::same(if player.is_playing() { 16 } else { 24 }), fill);
            let loading = player.state.loading && has && !player.state.paused;
            if loading || player.radio_loading() {
                row_ui.put(Rect::from_center_size(pr.center(), vec2(22.0, 22.0)), egui::Spinner::new().color(p.on_primary));
            } else {
                let icon = if player.is_playing() { ICON_PAUSE } else { ICON_PLAY_ARROW };
                widgets::paint_icon(&row_ui, icon, pr.center(), 28.0, if has { p.on_primary } else { p.on_surface.gamma_multiply(0.38) });
            }
            if presp.clicked() {
                player.toggle_pause();
            }
            if widgets::icon_button_sized(&mut row_ui, ICON_SKIP_NEXT, "Next (Ctrl+→)", has, 40.0, false).clicked() {
                player.next();
            }
            let (ricon, rtip) = match player.repeat {
                Repeat::Off => (ICON_REPEAT, "Repeat: off"),
                Repeat::All => (ICON_REPEAT, "Repeat: all"),
                Repeat::One => (ICON_REPEAT_ONE, "Repeat: one"),
            };
            if widgets::icon_button_sized(&mut row_ui, ricon, rtip, true, 36.0, player.repeat != Repeat::Off).clicked() {
                player.cycle_repeat();
            }

            let pos = player.state.time_pos;
            let dur = player.state.duration;
            let seek_w = (center.width() - 110.0).clamp(120.0, 560.0);
            let seek_row = Rect::from_center_size(pos2(center.center().x, center.bottom() - 12.0), vec2(seek_w + 104.0, 20.0));
            let mut sui = cui.new_child(UiBuilder::new().max_rect(seek_row).layout(egui::Layout::left_to_right(egui::Align::Center)));
            let small = FontId::proportional(11.5);
            sui.add_sized(vec2(44.0, 20.0), egui::Label::new(egui::RichText::new(format_duration(pos)).font(small.clone()).color(p.on_surface_variant)));
            let frac = if dur > 0.0 { (pos / dur) as f32 } else { 0.0 };
            let (sresp, val) = bar(&mut sui, Id::unique("seek"), frac, seek_w);
            if let Some(v) = val {
                if sresp.dragged() {
                    sresp.clone().on_hover_text_at_pointer(format_duration(v as f64 * dur));
                } else if dur > 0.0 {
                    player.seek(v as f64 * dur);
                }
            }
            sui.add_sized(vec2(44.0, 20.0), egui::Label::new(egui::RichText::new(format_duration(dur)).font(small).color(p.on_surface_variant)));

            // Right: like, download, volume, lyrics, queue.
            let mut rui = ui.new_child(UiBuilder::new().max_rect(right).layout(egui::Layout::right_to_left(egui::Align::Center)));
            rui.spacing_mut().item_spacing.x = 2.0;
            let on_now = app.page == Page::NowPlaying;
            if widgets::icon_button_sized(&mut rui, ICON_QUEUE_MUSIC, "Queue", true, 36.0, on_now && !app.pages.now.lyrics).clicked() {
                app.pages.now.lyrics = false;
                actions.push(if on_now && !app.pages.now.lyrics { Action::Back } else { Action::Goto(Page::NowPlaying) });
            }
            if widgets::icon_button_sized(&mut rui, ICON_LYRICS, "Lyrics", has, 36.0, on_now && app.pages.now.lyrics).clicked() {
                app.pages.now.lyrics = true;
                actions.push(Action::Goto(Page::NowPlaying));
            }
            rui.add_space(6.0);
            let (_, vol) = bar(&mut rui, Id::unique("volume"), (app.player.volume / 100.0) as f32, 96.0);
            if let Some(v) = vol {
                app.player.set_volume(v as f64 * 100.0);
            }
            let muted = app.player.volume <= 0.5;
            let vicon = if muted { ICON_VOLUME_OFF } else if app.player.volume < 50.0 { ICON_VOLUME_DOWN } else { ICON_VOLUME_UP };
            if widgets::icon_button_sized(&mut rui, vicon, if muted { "Unmute" } else { "Mute" }, true, 36.0, false).clicked() {
                let id = Id::unique("premute-volume");
                if muted {
                    let back = rui.ctx().data(|d| d.get_temp::<f64>(id)).unwrap_or(80.0);
                    app.player.set_volume(back);
                } else {
                    let cur = app.player.volume;
                    rui.ctx().data_mut(|d| d.insert_temp(id, cur));
                    app.player.set_volume(0.0);
                }
            }
            if let Some(t) = &track {
                if let Some(vid) = t.video_id.clone() {
                    let downloaded = app.downloads.is_downloaded(&vid);
                    let busy = app.downloads.job_for(&vid).is_some();
                    let (icon, tip) = if downloaded {
                        (ICON_DOWNLOAD_DONE, "Downloaded")
                    } else if busy {
                        (ICON_DOWNLOADING, "Downloading…")
                    } else {
                        (ICON_DOWNLOAD, "Download")
                    };
                    if widgets::icon_button_sized(&mut rui, icon, tip, !downloaded && !busy, 36.0, downloaded).clicked() {
                        actions.push(Action::Download(vec![t.clone()]));
                    }
                    if app.client.signed_in() {
                        let liked = app.library.is_liked(t);
                        let (r, resp) = rui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                        let resp = resp.on_hover_text(if liked { "Remove from liked" } else { "Like" }).on_hover_cursor(egui::CursorIcon::PointingHand);
                        if resp.hovered() {
                            rui.painter().circle_filled(r.center(), 18.0, p.on_surface.gamma_multiply(0.08));
                        }
                        let glyph = if liked { ICON_FAVORITE.filled() } else { ICON_FAVORITE.outlined() };
                        rui.painter().text(r.center(), Align2::CENTER_CENTER, glyph.codepoint, FontId::new(21.0, glyph.font_family()), if liked { p.primary } else { p.on_surface_variant });
                        if resp.clicked() {
                            actions.push(Action::ToggleLike(t.clone()));
                        }
                    }
                }
            }
        });
    app.actions.extend(actions);
}
