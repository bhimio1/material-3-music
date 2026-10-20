//! Material 3 Music — a YouTube Music client and local music player for
//! Linux, themed with Material You.

mod app;
mod config;
mod downloads;
mod library;
mod local;
mod player;
mod sys;
mod theme;
mod thumbs;
mod ui;
mod yt;

const USAGE: &str = "Usage: m3-music [OPTIONS] [FILE_OR_URL…]

Plays the given audio files or YouTube Music / YouTube links.

Options:
  -h, --help   Show this help
  -V, --version";

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("warn")).init();
    let mut open = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "-h" | "--help" => {
                println!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("m3-music {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            other if other.starts_with('-') => {
                eprintln!("Unknown option: {other}\n\n{USAGE}");
                std::process::exit(2);
            }
            other => open.push(other.to_string()),
        }
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Music")
            .with_app_id("material_3_music")
            .with_inner_size([1280.0, 840.0])
            .with_min_inner_size([820.0, 560.0]),
        ..Default::default()
    };
    eframe::run_native(
        "material_3_music",
        options,
        Box::new(move |cc| Ok(Box::new(app::App::new(cc, open)))),
    )
}
