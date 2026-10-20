//! Cover art and thumbnails: fetched on worker threads, cached on disk
//! (`~/.cache/m3-music/thumbs`) and as GPU textures in an LRU.

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use lru::LruCache;
use std::hash::{Hash, Hasher};
use std::num::NonZeroUsize;
use std::path::PathBuf;

const MAX_SIZE: u32 = 544;
const WORKERS: usize = 4;

enum Slot {
    Loading,
    Ready(TextureHandle, Color32),
    Failed,
}

pub struct Thumbs {
    ctx: egui::Context,
    cache: LruCache<String, Slot>,
    requests: flume::Sender<String>,
    results: flume::Receiver<(String, Option<(ColorImage, Color32)>)>,
}

fn cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| crate::sys::paths::home().join(".cache"))
        .join("m3-music/thumbs")
}

fn hash(s: &str) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// Ask Google's image servers for a bigger square version.
pub fn upscale(url: &str) -> String {
    let is_google = url.contains("googleusercontent.com") || url.contains("ggpht.com");
    if is_google {
        if let Some(i) = url.rfind('=') {
            if url[i..].starts_with("=w") || url[i..].starts_with("=s") {
                return format!("{}=w{MAX_SIZE}-h{MAX_SIZE}-l90-rj", &url[..i]);
            }
        }
    }
    url.to_string()
}

/// A vivid representative color: hue buckets weighted by saturation and
/// brightness, averaged within the winning bucket.
pub fn seed_color(img: &ColorImage) -> Color32 {
    let mut buckets = [(0.0f32, 0.0f32, 0.0f32, 0.0f32); 12];
    let step = (img.pixels.len() / 4096).max(1);
    for px in img.pixels.iter().step_by(step) {
        let (r, g, b) = (px.r() as f32 / 255.0, px.g() as f32 / 255.0, px.b() as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        let chroma = max - min;
        if chroma < 0.08 || max < 0.15 {
            continue;
        }
        let hue = if max == r {
            ((g - b) / chroma).rem_euclid(6.0)
        } else if max == g {
            (b - r) / chroma + 2.0
        } else {
            (r - g) / chroma + 4.0
        } * 60.0;
        let w = chroma * max;
        let bucket = &mut buckets[((hue / 30.0) as usize).min(11)];
        bucket.0 += r * w;
        bucket.1 += g * w;
        bucket.2 += b * w;
        bucket.3 += w;
    }
    let best = buckets.iter().max_by(|a, b| a.3.total_cmp(&b.3)).copied().unwrap_or_default();
    if best.3 <= 0.0 {
        return Color32::from_rgb(0x42, 0x85, 0xF4);
    }
    let c = |v: f32| ((v / best.3) * 255.0).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(c(best.0), c(best.1), c(best.2))
}

/// Development stand-in art: a gradient derived from the URL.
fn fixture_image(url: &str) -> ColorImage {
    let h = hash(url);
    let hue = (h % 360) as f32;
    let to_rgb = |hue: f32, s: f32, v: f32| {
        let c = v * s;
        let x = c * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
        let (r, g, b) = match (hue / 60.0) as u32 {
            0 => (c, x, 0.0),
            1 => (x, c, 0.0),
            2 => (0.0, c, x),
            3 => (0.0, x, c),
            4 => (x, 0.0, c),
            _ => (c, 0.0, x),
        };
        let m = v - c;
        [((r + m) * 255.0) as u8, ((g + m) * 255.0) as u8, ((b + m) * 255.0) as u8]
    };
    let size = 128;
    let mut pixels = Vec::with_capacity(size * size);
    for y in 0..size {
        for x in 0..size {
            let t = (x + y) as f32 / (2 * size) as f32;
            let d = (((x as f32 - 40.0).powi(2) + (y as f32 - 40.0).powi(2)).sqrt() / 180.0).min(1.0);
            let [r, g, b] = to_rgb((hue + t * 50.0) % 360.0, 0.55 + 0.3 * d, 0.95 - 0.45 * t);
            pixels.push(Color32::from_rgb(r, g, b));
        }
    }
    ColorImage::new([size, size], pixels)
}

fn decode(bytes: &[u8]) -> Option<ColorImage> {
    let img = image::load_from_memory(bytes).ok()?;
    let img = if img.width() > MAX_SIZE || img.height() > MAX_SIZE {
        img.thumbnail(MAX_SIZE, MAX_SIZE)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    Some(ColorImage::from_rgba_unmultiplied(
        [rgba.width() as usize, rgba.height() as usize],
        rgba.as_raw(),
    ))
}

fn load(agent: &ureq::Agent, url: &str, fixtures: bool) -> Option<ColorImage> {
    if let Some(path) = url.strip_prefix("file://") {
        return decode(&std::fs::read(path).ok()?);
    }
    if fixtures {
        return Some(fixture_image(url));
    }
    let disk = cache_dir().join(format!("{:016x}", hash(url)));
    if let Ok(bytes) = std::fs::read(&disk) {
        if let Some(img) = decode(&bytes) {
            return Some(img);
        }
    }
    let resp = agent.get(url).call().ok()?;
    let mut bytes = Vec::new();
    std::io::Read::read_to_end(&mut resp.into_reader().take(8 << 20), &mut bytes).ok()?;
    let img = decode(&bytes)?;
    let _ = std::fs::create_dir_all(cache_dir());
    let _ = std::fs::write(&disk, &bytes);
    Some(img)
}

use std::io::Read as _;

impl Thumbs {
    pub fn new(ctx: egui::Context, agent: ureq::Agent, fixtures: bool) -> Self {
        let (requests, req_rx) = flume::unbounded::<String>();
        let (res_tx, results) = flume::unbounded();
        for _ in 0..WORKERS {
            let rx = req_rx.clone();
            let tx = res_tx.clone();
            let ctx = ctx.clone();
            let agent = agent.clone();
            std::thread::spawn(move || {
                for url in rx.iter() {
                    let img = load(&agent, &url, fixtures).map(|i| {
                        let seed = seed_color(&i);
                        (i, seed)
                    });
                    let _ = tx.send((url, img));
                    ctx.request_repaint();
                }
            });
        }
        Self {
            ctx,
            cache: LruCache::new(NonZeroUsize::new(400).expect("non-zero")),
            requests,
            results,
        }
    }

    pub fn poll(&mut self) {
        for (url, img) in self.results.try_iter() {
            let slot = match img {
                Some((img, seed)) => {
                    let tex = self.ctx.load_texture(&url, img, TextureOptions::LINEAR);
                    Slot::Ready(tex, seed)
                }
                None => Slot::Failed,
            };
            self.cache.put(url, slot);
        }
    }

    fn request(&mut self, url: &str) -> Option<&Slot> {
        let url = upscale(url);
        if !self.cache.contains(&url) {
            self.cache.put(url.clone(), Slot::Loading);
            let _ = self.requests.send(url.clone());
        }
        self.cache.get(&url)
    }

    pub fn get(&mut self, url: &str) -> Option<TextureHandle> {
        match self.request(url)? {
            Slot::Ready(t, _) => Some(t.clone()),
            _ => None,
        }
    }

    /// Seed color of a loaded image (for album-art theming).
    pub fn seed(&mut self, url: &str) -> Option<Color32> {
        match self.request(url)? {
            Slot::Ready(_, c) => Some(*c),
            _ => None,
        }
    }

}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upscaling() {
        assert_eq!(
            upscale("https://lh3.googleusercontent.com/abc=w60-h60-l90-rj"),
            "https://lh3.googleusercontent.com/abc=w544-h544-l90-rj"
        );
        assert_eq!(upscale("https://i.ytimg.com/vi/x/sddefault.jpg"), "https://i.ytimg.com/vi/x/sddefault.jpg");
    }

    #[test]
    fn seed_prefers_vivid_colors() {
        let mut px = vec![Color32::from_gray(20); 900];
        px.extend(vec![Color32::from_rgb(200, 30, 40); 100]);
        let img = ColorImage::new([1000, 1], px);
        let c = seed_color(&img);
        assert!(c.r() > 150 && c.g() < 80);
        let grey = ColorImage::new([1, 1], vec![Color32::from_gray(128)]);
        assert_eq!(seed_color(&grey), Color32::from_rgb(0x42, 0x85, 0xF4));
    }
}
