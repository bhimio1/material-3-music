//! Preferences (`~/.config/m3-music/config.toml`).

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ColorScheme {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ThemeSource {
    /// The end-4 shell's colors, else the seed color.
    #[default]
    Auto,
    Seed,
    /// Seeded from the cover of the song that's playing.
    #[serde(rename = "album-art")]
    AlbumArt,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    #[default]
    Opus,
    M4a,
    Mp3,
}

impl AudioFormat {
    pub fn ext(self) -> &'static str {
        match self {
            AudioFormat::Opus => "opus",
            AudioFormat::M4a => "m4a",
            AudioFormat::Mp3 => "mp3",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(default)]
pub struct Config {
    pub theme_source: ThemeSource,
    pub color_scheme: ColorScheme,
    pub seed_color: String,
    pub filled_icons: bool,
    pub ui_scale: f32,
    /// Folders scanned for local music.
    pub library_folders: Vec<String>,
    /// Where downloads go. Empty = `<music dir>/M3 Music`.
    pub download_dir: String,
    pub download_format: AudioFormat,
    pub concurrent_downloads: usize,
    pub volume: f64,
    /// Keep playing related songs when the queue runs out.
    pub autoplay: bool,
    /// Minutes between background library syncs (0 = only on start).
    pub sync_minutes: u32,
    /// OAuth client ("TVs and Limited Input devices") from the user's own
    /// Google Cloud project.
    pub oauth_client_id: String,
    pub oauth_client_secret: String,
    pub terminal: String,
}

pub fn music_dir() -> PathBuf {
    dirs::audio_dir().unwrap_or_else(|| crate::sys::paths::home().join("Music"))
}

impl Default for Config {
    fn default() -> Self {
        Self {
            theme_source: ThemeSource::Auto,
            color_scheme: ColorScheme::System,
            seed_color: crate::theme::palette::DEFAULT_SEED.into(),
            filled_icons: true,
            ui_scale: 1.0,
            library_folders: vec![crate::sys::paths::tilde(&music_dir())],
            download_dir: String::new(),
            download_format: AudioFormat::Opus,
            concurrent_downloads: 2,
            volume: 80.0,
            autoplay: true,
            sync_minutes: 30,
            oauth_client_id: String::new(),
            oauth_client_secret: String::new(),
            terminal: String::new(),
        }
    }
}

impl Config {
    pub fn path() -> Option<PathBuf> {
        dirs::config_dir().map(|d| d.join("m3-music/config.toml"))
    }

    pub fn load() -> Self {
        Self::path()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| toml::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Saved privately: it may hold the OAuth client secret.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = Self::path().ok_or_else(|| anyhow::anyhow!("no config directory"))?;
        crate::yt::auth::write_private(&path, toml::to_string_pretty(self)?.as_bytes())?;
        Ok(())
    }

    pub fn download_root(&self) -> PathBuf {
        if self.download_dir.trim().is_empty() {
            music_dir().join("M3 Music")
        } else {
            crate::sys::paths::expand(self.download_dir.trim())
        }
    }

    pub fn folders(&self) -> Vec<PathBuf> {
        self.library_folders
            .iter()
            .map(|f| crate::sys::paths::expand(f.trim()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let c: Config = toml::from_str("volume = 50.0\ntheme_source = \"album-art\"\n").unwrap();
        assert_eq!(c.volume, 50.0);
        assert_eq!(c.theme_source, ThemeSource::AlbumArt);
        assert_eq!(c.concurrent_downloads, 2);
        let back: Config = toml::from_str(&toml::to_string_pretty(&c).unwrap()).unwrap();
        assert_eq!(back, c);
    }
}
