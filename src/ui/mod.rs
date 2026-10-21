//! User interface. Pages draw from app state and push [`Action`]s, which
//! the app applies after the frame.

pub mod art;
pub mod nowbar;
pub mod pages;
pub mod sidebar;
pub mod tracks;

use crate::yt::model::{Collection, Track};
use material_3_ui::common::{self, Requirement};
pub use material_3_ui::toasts::ToastKind;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LibTab {
    Playlists,
    Liked,
    Albums,
    Artists,
    History,
}

impl LibTab {
    pub fn label(self) -> &'static str {
        match self {
            LibTab::Playlists => "Playlists",
            LibTab::Liked => "Liked songs",
            LibTab::Albums => "Albums",
            LibTab::Artists => "Artists",
            LibTab::History => "History",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Page {
    Home,
    Search,
    Library(LibTab),
    /// An album (`MPREb_…`), playlist (`VL…`) or artist (`UC…`).
    Detail(String),
    Local,
    Downloads,
    NowPlaying,
    Settings,
}

#[derive(Clone, Debug)]
pub enum Action {
    Goto(Page),
    Back,
    Toast(String, ToastKind),
    /// Install a package in a terminal.
    Install(String),
    Play(Vec<Track>, usize),
    PlayNext(Vec<Track>),
    Enqueue(Vec<Track>),
    StartRadio(Track),
    PlayCollection { collection: Collection, shuffle: bool },
    ToggleLike(Track),
    AddToPlaylist(Collection, Vec<Track>),
    /// Ask for a name, then create a playlist with these songs.
    NewPlaylist(Vec<Track>),
    RemoveFromPlaylist(String, Track),
    Download(Vec<Track>),
    DownloadCollection(String),
    RemoveDownload(String),
    OpenPath(PathBuf),
    OpenUrl(String),
    CopyText(String),
    SyncNow,
    ConfigChanged,
}

pub type Actions = Vec<Action>;


/// Check that a command-line tool is available; when it isn't, show
/// [`common::require`]'s banner, whose Install button pushes
/// [`Action::Install`]. Returns whether the tool is there.
pub fn require(ui: &mut egui::Ui, actions: &mut Actions, program: &str, package: &str, purpose: &str) -> bool {
    if material_3_ui::sys::cmd::exists(program) {
        return true;
    }
    // Only read when something is missing; once it's installed the check
    // above passes.
    static INSTALLED: OnceLock<HashSet<String>> = OnceLock::new();
    let installed = INSTALLED.get_or_init(material_3_ui::sys::arch::installed_packages);
    if common::require(ui, installed, program, package, purpose) == Requirement::InstallRequested {
        actions.push(Action::Install(package.to_string()));
    }
    false
}
