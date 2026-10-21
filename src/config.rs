//! Preferences (`~/.config/m3-music/config.toml`).

use material_3_ui::theme::{ColorScheme, ThemeSettings, ThemeSource};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

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
    /// Seed the palette from the cover of the song that's playing (the
    /// `appearance` source is used while nothing is).
    pub album_art_colors: bool,
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
    /// Extra options passed to mpv (e.g. `--ao=pipewire`, `--af=loudnorm`).
    pub mpv_args: Vec<String>,
    /// Color source, light/dark, seed color, icons and scale, shared with
    /// the other Material 3 apps. Last, as a TOML table.
    pub appearance: ThemeSettings,
}

/// The appearance fields as 0.1 stored them, at the top level.
#[derive(Deserialize)]
struct Legacy {
    theme_source: Option<LegacySource>,
    color_scheme: Option<ColorScheme>,
    seed_color: Option<String>,
    filled_icons: Option<bool>,
    ui_scale: Option<f32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum LegacySource {
    Auto,
    Seed,
    #[serde(rename = "album-art")]
    AlbumArt,
}

pub fn music_dir() -> PathBuf {
    dirs::audio_dir().unwrap_or_else(|| material_3_ui::sys::paths::home().join("Music"))
}

impl Default for Config {
    fn default() -> Self {
        Self {
            album_art_colors: false,
            library_folders: vec![material_3_ui::sys::paths::tilde(&music_dir())],
            download_dir: String::new(),
            download_format: AudioFormat::Opus,
            concurrent_downloads: 2,
            volume: 80.0,
            autoplay: true,
            sync_minutes: 30,
            oauth_client_id: String::new(),
            oauth_client_secret: String::new(),
            terminal: String::new(),
            mpv_args: Vec::new(),
            appearance: ThemeSettings::default(),
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
            .and_then(|s| Self::parse(&s))
            .unwrap_or_default()
    }

    /// Reads the file, moving 0.1's top-level appearance fields into
    /// `appearance` (the album-art source becomes `album_art_colors` over
    /// the seed color, which it fell back to before).
    fn parse(text: &str) -> Option<Self> {
        let mut config: Config = toml::from_str(text).ok()?;
        let table: toml::Table = toml::from_str(text).ok()?;
        if !table.contains_key("appearance") {
            if let Ok(old) = toml::from_str::<Legacy>(text) {
                let a = &mut config.appearance;
                match old.theme_source {
                    Some(LegacySource::Auto) | None => {}
                    Some(LegacySource::Seed) => a.source = ThemeSource::Seed,
                    Some(LegacySource::AlbumArt) => {
                        a.source = ThemeSource::Seed;
                        config.album_art_colors = true;
                    }
                }
                a.scheme = old.color_scheme.unwrap_or(a.scheme);
                a.seed_color = old.seed_color.unwrap_or(std::mem::take(&mut a.seed_color));
                a.filled_icons = old.filled_icons.unwrap_or(a.filled_icons);
                a.ui_scale = old.ui_scale.unwrap_or(a.ui_scale);
            }
        }
        Some(config)
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
            material_3_ui::sys::paths::expand(self.download_dir.trim())
        }
    }

    pub fn folders(&self) -> Vec<PathBuf> {
        self.library_folders
            .iter()
            .map(|f| material_3_ui::sys::paths::expand(f.trim()))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip() {
        let c = Config::parse("volume = 50.0\nalbum_art_colors = true\n[appearance]\nscheme = \"light\"\n").unwrap();
        assert_eq!(c.volume, 50.0);
        assert!(c.album_art_colors);
        assert_eq!(c.appearance.scheme, ColorScheme::Light);
        assert_eq!(c.concurrent_downloads, 2);
        let text = toml::to_string_pretty(&c).unwrap();
        assert_eq!(Config::parse(&text).unwrap(), c);
    }

    #[test]
    fn old_appearance_fields_carry_over() {
        let old = "theme_source = \"album-art\"\ncolor_scheme = \"dark\"\nseed_color = \"#C2185B\"\nfilled_icons = false\nui_scale = 1.25\nvolume = 40.0\n";
        let c = Config::parse(old).unwrap();
        assert!(c.album_art_colors);
        assert_eq!(c.appearance.source, ThemeSource::Seed);
        assert_eq!(c.appearance.scheme, ColorScheme::Dark);
        assert_eq!(c.appearance.seed_color, "#C2185B");
        assert!(!c.appearance.filled_icons);
        assert_eq!(c.appearance.ui_scale, 1.25);
        assert_eq!(c.volume, 40.0);

        let c = Config::parse("theme_source = \"seed\"\n").unwrap();
        assert_eq!(c.appearance.source, ThemeSource::Seed);
        assert!(!c.album_art_colors);
        let c = Config::parse("color_scheme = \"light\"\n").unwrap();
        assert_eq!(c.appearance.source, ThemeSource::Auto);
        assert_eq!(c.appearance.scheme, ColorScheme::Light);
        assert_eq!(c.appearance.seed_color, material_3_ui::theme::palette::DEFAULT_SEED);

        // Once saved in the new layout, stray old keys don't override it.
        let c = Config::parse("color_scheme = \"dark\"\n[appearance]\nscheme = \"light\"\n").unwrap();
        assert_eq!(c.appearance.scheme, ColorScheme::Light);
    }
}
