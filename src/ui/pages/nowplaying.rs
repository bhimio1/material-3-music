//! The expanded player: big cover art, the queue, and lyrics.

use super::widgets;
use crate::app::App;
use crate::sys::task::Task;
use crate::ui::common;
use crate::ui::tracks::{empty, track_menu, ListOpts};
use crate::ui::{art, Action};
use crate::yt::model::{format_duration, Lyrics};
use egui::{pos2, vec2, Align2, Color32, CornerRadius, FontId, Id, Rect, RichText, Sense, Ui, UiBuilder};
use egui_material_icons::icons::*;
use std::collections::HashMap;

#[derive(Default)]
pub struct State {
    /// Showing lyrics instead of the queue.
    pub lyrics: bool,
    lyrics_cache: HashMap<String, Task<anyhow::Result<Option<Lyrics>>>>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Tab {
    Queue,
    Lyrics,
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let Some(track) = app.player.current().cloned() else {
        empty(ui, ICON_MUSIC_NOTE, "Nothing playing", "Pick something from Home, Search or your library.");
        return;
    };
    let full = ui.available_rect_before_wrap().shrink(24.0);
    let wide = full.width() > 900.0;
    let left_w = if wide { (full.width() * 0.42).min(520.0) } else { full.width() };
    let left = Rect::from_min_size(full.min, vec2(left_w, if wide { full.height() } else { full.height() * 0.45 }));
    let right = if wide {
        Rect::from_min_max(pos2(left.right() + 32.0, full.top()), full.max)
    } else {
        Rect::from_min_max(pos2(full.left(), left.bottom() + 16.0), full.max)
    };
    let mut actions = Vec::new();

    // Left: art + titles.
    {
        let mut lui = ui.new_child(UiBuilder::new().max_rect(left));
        if widgets::icon_button(&mut lui, ICON_EXPAND_MORE, "Close", true).clicked() {
            actions.push(Action::Back);
        }
        let size = (left.width()).min(left.height() - 150.0).max(120.0);
        let art_rect = Rect::from_min_size(pos2(left.center().x - size / 2.0, left.top() + 44.0), vec2(size, size));
        art::paint(&lui, &mut app.thumbs, track.thumbnail.as_deref(), art_rect, 24, ICON_MUSIC_NOTE);
        let y = art_rect.bottom() + 18.0;
        let g = widgets::truncated_galley(&lui, &track.title, widgets::title_font(24.0), p.on_surface, left.width());
        lui.painter().galley(pos2(left.center().x - g.size().x / 2.0, y), g, p.on_surface);
        let mut sub = track.artist_line();
        if let Some(a) = &track.album {
            sub = if sub.is_empty() { a.name.clone() } else { format!("{sub} • {}", a.name) };
        }
        let g = widgets::truncated_galley(&lui, &sub, FontId::proportional(14.0), p.on_surface_variant, left.width());
        lui.painter().galley(pos2(left.center().x - g.size().x / 2.0, y + 34.0), g, p.on_surface_variant);
        let src = if track.path.is_some() || app.player.local_path(&track).is_some() { "Playing from your files" } else { "Streaming from YouTube Music" };
        lui.painter().text(pos2(left.center().x, y + 62.0), Align2::CENTER_TOP, src, FontId::proportional(12.0), p.on_surface_variant.gamma_multiply(0.8));
    }

    // Right: queue or lyrics.
    let mut rui = ui.new_child(UiBuilder::new().max_rect(right));
    let mut tab = if app.pages.now.lyrics { Tab::Lyrics } else { Tab::Queue };
    rui.horizontal(|ui| {
        if widgets::segmented(ui, &mut tab, &[(Tab::Queue, ICON_QUEUE_MUSIC, "Up next"), (Tab::Lyrics, ICON_LYRICS, "Lyrics")]) {
            app.pages.now.lyrics = tab == Tab::Lyrics;
        }
        if tab == Tab::Queue {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if widgets::button(ui, widgets::ButtonKind::Text, Some(ICON_CLEAR_ALL), "Clear").clicked() {
                    app.player.clear_upcoming();
                }
                if app.player.autoplay {
                    ui.label(RichText::new("Autoplay on").size(12.0).color(p.on_surface_variant));
                }
            });
        }
    });
    rui.add_space(10.0);

    match tab {
        Tab::Lyrics => {
            let Some(vid) = track.video_id.clone() else {
                empty(&mut rui, ICON_LYRICS, "No lyrics", "Lyrics come from YouTube Music, so local files don't have them.");
                app.actions.extend(actions);
                return;
            };
            let client = app.client.clone();
            let entry = app.pages.now.lyrics_cache.entry(vid.clone()).or_default();
            entry.ensure(&app.ctx, move || client.lyrics(&vid));
            egui::ScrollArea::vertical().id_salt("lyrics").auto_shrink([false, false]).show(&mut rui, |ui| match &entry.value {
                None => common::loading(ui, "Finding lyrics…"),
                Some(Err(e)) => common::banner(ui, ICON_CLOUD_OFF, &format!("Couldn't load lyrics. {e}"), true, |_| {}),
                Some(Ok(None)) => empty(ui, ICON_LYRICS, "No lyrics for this song", ""),
                Some(Ok(Some(l))) => {
                    ui.add(egui::Label::new(RichText::new(&l.text).size(20.0).line_height(Some(32.0)).color(p.on_surface)).wrap());
                    ui.add_space(16.0);
                    ui.label(RichText::new(&l.source).size(12.0).color(p.on_surface_variant));
                }
            });
        }
        Tab::Queue => {
            let current = app.player.index;
            let queue = app.player.queue.clone();
            let mut jump = None;
            let mut remove = None;
            let mut swap: Option<(usize, usize)> = None;
            let (mut v, _) = app.split();
            egui::ScrollArea::vertical().id_salt("queue").auto_shrink([false, false]).show_rows(&mut rui, 56.0, queue.len(), |ui, range| {
                for i in range {
                    let t = &queue[i];
                    let w = ui.available_width();
                    let (rect, _) = ui.allocate_exact_size(vec2(w, 56.0), Sense::hover());
                    let row = ui.interact(rect, Id::unique(("queue", i)), Sense::click());
                    let is_cur = Some(i) == current;
                    let fill = if is_cur {
                        p.secondary_container
                    } else if row.hovered() {
                        p.on_surface.gamma_multiply(0.06)
                    } else {
                        Color32::TRANSPARENT
                    };
                    ui.painter().rect_filled(rect, CornerRadius::same(12), fill);
                    let a = Rect::from_min_size(pos2(rect.left() + 8.0, rect.center().y - 20.0), vec2(40.0, 40.0));
                    art::paint(ui, v.thumbs, t.thumbnail.as_deref(), a, 6, ICON_MUSIC_NOTE);
                    if is_cur {
                        ui.painter().rect_filled(a, CornerRadius::same(6), Color32::from_black_alpha(110));
                        widgets::paint_icon(ui, if v.playing { ICON_EQUALIZER } else { ICON_PAUSE }, a.center(), 22.0, Color32::WHITE);
                    }
                    let tw = rect.width() - 200.0;
                    let fg = if is_cur { p.on_secondary_container } else { p.on_surface };
                    let g = widgets::truncated_galley(ui, &t.title, widgets::title_font(14.0), fg, tw);
                    ui.painter().galley(pos2(a.right() + 12.0, rect.center().y - 17.0), g, fg);
                    let g = widgets::truncated_galley(ui, &t.artist_line(), FontId::proportional(12.5), p.on_surface_variant, tw);
                    ui.painter().galley(pos2(a.right() + 12.0, rect.center().y + 2.0), g, p.on_surface_variant);
                    let dur = t.duration.map(|d| format_duration(d as f64)).unwrap_or_default();
                    ui.painter().text(rect.right_center() - vec2(12.0, 0.0), Align2::RIGHT_CENTER, dur, FontId::proportional(12.5), p.on_surface_variant);
                    if row.hovered() && !is_cur {
                        let mut bx = rect.right() - 64.0;
                        for (icon, tip, act) in [(ICON_CLOSE, "Remove", 0), (ICON_ARROW_DOWNWARD, "Move down", 1), (ICON_ARROW_UPWARD, "Move up", 2)] {
                            let r = Rect::from_center_size(pos2(bx - 16.0, rect.center().y), vec2(32.0, 32.0));
                            let b = ui.interact(r, Id::unique(("qbtn", i, act)), Sense::click()).on_hover_text(tip);
                            if b.hovered() {
                                ui.painter().circle_filled(r.center(), 16.0, p.on_surface.gamma_multiply(0.08));
                            }
                            widgets::paint_icon(ui, icon, r.center(), 18.0, p.on_surface_variant);
                            if b.clicked() {
                                match act {
                                    0 => remove = Some(i),
                                    1 if i + 1 < queue.len() => swap = Some((i, i + 1)),
                                    2 if i > 0 => swap = Some((i, i - 1)),
                                    _ => {}
                                }
                            }
                            bx -= 34.0;
                        }
                    }
                    row.context_menu(|ui| track_menu(ui, &mut v, t, ListOpts::default()));
                    if row.clicked() && !is_cur {
                        jump = Some(i);
                    }
                }
            });
            drop(v);
            if let Some(i) = jump {
                app.player.jump(i);
            }
            if let Some(i) = remove {
                app.player.remove(i);
            }
            if let Some((a, b)) = swap {
                app.player.reorder(a, b);
            }
        }
    }
    app.actions.extend(actions);
}
