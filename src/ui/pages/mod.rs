//! One module per page; [`States`] holds what each keeps between frames.

pub mod detail;
pub mod downloads;
pub mod home;
pub mod library;
pub mod local;
pub mod nowplaying;
pub mod search;
pub mod settings;

use super::{widgets, Page};
use crate::app::App;
use egui::{Frame, Margin, Ui};
use std::collections::HashMap;

#[derive(Default)]
pub struct States {
    pub home: home::State,
    pub search: search::State,
    pub details: HashMap<String, detail::State>,
    pub library: library::State,
    pub local: local::State,
    pub now: nowplaying::State,
    pub settings: settings::State,
}

/// A full-width scrolling page body.
pub fn scroll(ui: &mut Ui, id: impl std::hash::Hash + std::fmt::Debug, body: impl FnOnce(&mut Ui)) {
    egui::ScrollArea::vertical()
        .id_salt(id)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            Frame::new().inner_margin(Margin::symmetric(28, 22)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                body(ui);
                ui.add_space(24.0);
            });
        });
}

pub fn show(ui: &mut Ui, app: &mut App) {
    match app.page.clone() {
        Page::Home => home::show(ui, app),
        Page::Search => search::show(ui, app),
        Page::Library(tab) => library::show(ui, app, tab),
        Page::Detail(id) => detail::show(ui, app, &id),
        Page::Local => local::show(ui, app),
        Page::Downloads => downloads::show(ui, app),
        Page::NowPlaying => nowplaying::show(ui, app),
        Page::Settings => settings::show(ui, app),
    }
}
