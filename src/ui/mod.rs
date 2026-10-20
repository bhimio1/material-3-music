//! User interface. Pages draw from app state and push [`Action`]s, which
//! the app applies after the frame.

pub mod art;
pub mod common;
pub mod nowbar;
pub mod pages;
pub mod sidebar;
pub mod toasts;
pub mod tracks;
pub mod widgets;

use crate::yt::model::{Collection, Track};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Success,
    Error,
}

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

