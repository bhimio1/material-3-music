//! Search: suggestions while typing, filter chips, and results.

use super::scroll;
use material_3_ui::widgets;
use crate::app::App;
use material_3_ui::sys::task::Task;
use material_3_ui::common;
use crate::ui::tracks::{self, empty, ListOpts};
use crate::yt::model::{Item, Track};
use crate::yt::{SearchFilter, SearchResults};
use egui::{RichText, Ui};
use material_3_ui::icons::icons::*;
use std::time::{Duration, Instant};

#[derive(Default)]
pub struct State {
    pub query: String,
    /// The query the results are for.
    submitted: String,
    /// The selected filter's `params`; `None` = all results.
    filter: Option<String>,
    /// Filters offered for the submitted query (from its unfiltered results).
    filters: Vec<SearchFilter>,
    results: Task<anyhow::Result<SearchResults>>,
    suggestions: Task<anyhow::Result<Vec<String>>>,
    edited: Option<Instant>,
    pub focus: bool,
}

impl State {
    pub fn set_query(&mut self, q: String) {
        self.query = q;
        self.submit();
    }

    pub fn query_changed(&mut self) {
        self.edited = Some(Instant::now());
    }

    pub fn submit(&mut self) {
        self.edited = None;
        self.submitted = self.query.trim().to_string();
        // A new query starts unfiltered, which also fetches its filters.
        self.filter = None;
        self.filters.clear();
        // Results are (re)loaded by the page.
        self.results = Task::default();
    }
}

pub fn show(ui: &mut Ui, app: &mut App) {
    let p = app.palette.clone();
    let (mut v, states) = app.split();
    let s = &mut states.search;

    // Debounced suggestions while typing.
    if let Some(at) = s.edited {
        if at.elapsed() > Duration::from_millis(250) && !s.query.trim().is_empty() {
            s.edited = None;
            let client = v.client.clone();
            let q = s.query.clone();
            s.suggestions.spawn(&v.ctx, move || client.suggestions(&q));
        } else {
            v.ctx.request_repaint_after(Duration::from_millis(260));
        }
    }
    s.suggestions.poll();
    if !s.submitted.is_empty() {
        let client = v.client.clone();
        let (q, f) = (s.submitted.clone(), s.filter.clone());
        s.results.ensure(&v.ctx, move || client.search(&q, f.as_deref()));
    }
    if s.filter.is_none() {
        if let Some(Ok(r)) = &s.results.value {
            if s.filters != r.filters {
                s.filters = r.filters.clone();
            }
        }
    }

    let mut submit: Option<String> = None;
    let mut set_filter: Option<Option<String>> = None;
    scroll(ui, "search", |ui| {
        ui.label(RichText::new("Search").size(28.0).color(p.on_surface));
        ui.add_space(8.0);
        let w = ui.available_width().min(640.0);
        let r = common::search_field(ui, &mut s.query, "What do you want to listen to?", w);
        if s.focus {
            s.focus = false;
            r.request_focus();
        }
        if r.changed() {
            s.query_changed();
        }
        if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            submit = Some(s.query.clone());
        }

        // Suggestions while the text differs from the searched query.
        let typing = s.query.trim() != s.submitted && !s.query.trim().is_empty();
        if typing {
            if let Some(Ok(list)) = &s.suggestions.value {
                ui.add_space(6.0);
                for sug in list.iter().take(8) {
                    if widgets::nav_item(ui, ICON_SEARCH, sug, false, None).clicked() {
                        submit = Some(sug.clone());
                    }
                }
            }
        }

        if s.submitted.is_empty() {
            if !typing {
                empty(ui, ICON_SEARCH, "Find your music", "Search for songs, albums, artists and playlists.");
            }
            return;
        }
        ui.add_space(12.0);
        ui.horizontal_wrapped(|ui| {
            if widgets::filter_chip(ui, "All", s.filter.is_none(), None).clicked() {
                set_filter = Some(None);
            }
            for f in &s.filters {
                let on = s.filter.as_deref() == Some(f.params.as_str());
                if widgets::filter_chip(ui, &f.label, on, None).clicked() {
                    set_filter = Some(Some(f.params.clone()));
                }
            }
        });
        match &s.results.value {
            None => common::loading(ui, &format!("Searching for “{}”…", s.submitted)),
            Some(Err(e)) => common::banner(ui, ICON_CLOUD_OFF, &format!("Search failed. {e}"), true, |_| {}),
            Some(Ok(r)) if r.shelves.is_empty() => empty(ui, ICON_SEARCH_OFF, "No results", "Try different words."),
            Some(Ok(r)) => {
                let shelves = &r.shelves;
                for (i, shelf) in shelves.iter().enumerate() {
                    let songs: Vec<Track> = shelf
                        .items
                        .iter()
                        .filter_map(|it| match it {
                            Item::Track(t) => Some(t.clone()),
                            _ => None,
                        })
                        .collect();
                    if shelf.title == "Top result" {
                        tracks::shelf_header(ui, &mut v, "Top result", None);
                        ui.horizontal_top(|ui| {
                            if let Some(first @ Item::Collection(_)) = shelf.items.first() {
                                tracks::card(ui, &mut v, first, 200.0, egui::Id::unique("top-card"));
                                ui.add_space(16.0);
                            }
                            ui.vertical(|ui| {
                                tracks::track_list(ui, &mut v, ("top", i), &songs, ListOpts::default());
                            });
                        });
                    } else if !songs.is_empty() && songs.len() == shelf.items.len() {
                        tracks::shelf_header(ui, &mut v, &shelf.title, None);
                        tracks::track_list(ui, &mut v, ("res", i), &songs, ListOpts::default());
                    } else if s.filter.is_some() {
                        tracks::grid(ui, &mut v, "search-grid", &shelf.items);
                    } else {
                        tracks::shelf(ui, &mut v, shelf, 100 + i);
                    }
                }
            }
        }
    });
    if let Some(f) = set_filter {
        s.filter = f;
        s.results = Task::default();
    }
    if let Some(q) = submit {
        s.set_query(q);
    }
}
