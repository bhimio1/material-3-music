//! Home: YouTube Music's personalised shelves.

use super::scroll;
use material_3_ui::widgets;
use crate::app::App;
use material_3_ui::sys::task::Task;
use material_3_ui::common;
use crate::ui::tracks::{self, empty};
use crate::ui::{Action, Page};
use crate::yt::model::Shelf;
use chrono::Timelike;
use egui::{RichText, Ui};
use material_3_ui::icons::icons::*;

#[derive(Default)]
pub struct State {
    pub shelves: Task<anyhow::Result<Vec<Shelf>>>,
}

fn greeting() -> &'static str {
    match chrono::Local::now().hour() {
        5..=11 => "Good morning",
        12..=17 => "Good afternoon",
        _ => "Good evening",
    }
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let signed_in = app.client.signed_in();
    let account = app.library.data.account.as_ref().map(|a| a.name.clone()).filter(|n| !n.is_empty());
    let (mut v, states) = app.split();
    let state = &mut states.home;
    let client = v.client.clone();
    state.shelves.ensure(&v.ctx, move || client.home());
    let busy = state.shelves.running();
    let mut reload = false;
    scroll(ui, "home", |ui| {
        ui.horizontal(|ui| {
            let title = match &account {
                Some(n) => format!("{}, {}", greeting(), n.split_whitespace().next().unwrap_or(n)),
                None => greeting().to_string(),
            };
            ui.label(RichText::new(title).size(32.0).color(p.on_surface));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                reload = common::refresh_button(ui, busy);
            });
        });
        if !signed_in {
            common::banner(ui, ICON_ACCOUNT_CIRCLE, "Sign in to get your mixes, library and recommendations.", false, |ui| {
                if widgets::button(ui, widgets::ButtonKind::Filled, Some(ICON_LOGIN), "Sign in").clicked() {
                    v.push(Action::Goto(Page::Settings));
                }
            });
        }
        match &state.shelves.value {
            None => common::loading(ui, "Loading your home page…"),
            Some(Err(e)) => {
                common::banner(ui, ICON_CLOUD_OFF, &format!("Couldn't load the home page. {e}"), true, |ui| {
                    if widgets::button(ui, widgets::ButtonKind::Tonal, Some(ICON_REFRESH), "Retry").clicked() {
                        reload = true;
                    }
                });
                if !v.library.data.history.is_empty() {
                    let recent = crate::yt::model::Shelf {
                        title: "Recently played".into(),
                        items: v.library.data.history.iter().take(12).cloned().map(crate::yt::model::Item::Track).collect(),
                        more: None,
                    };
                    tracks::shelf(ui, &mut v, &recent, 999);
                }
            }
            Some(Ok(shelves)) if shelves.is_empty() => empty(ui, ICON_HOME, "Nothing here yet", "Play some music and come back."),
            Some(Ok(shelves)) => {
                for (i, s) in shelves.iter().enumerate() {
                    tracks::shelf(ui, &mut v, s, i);
                }
            }
        }
    });
    if reload {
        let client = v.client.clone();
        state.shelves.spawn(&v.ctx, move || client.home());
    }
}
