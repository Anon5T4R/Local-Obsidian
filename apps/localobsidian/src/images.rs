//! Textures for the images a note shows.
//!
//! A note writes `![[foto.png]]` or `![alt](_attachments/foto.png)`; the reading view draws it. The
//! file is decoded here, once per change: the cache is keyed by path and remembers the file's
//! modification time, so replacing the picture on disk shows up on the next frame that draws it.
//!
//! Nothing is fetched. A destination that is not a file inside the vault simply has no texture, and
//! the reading view shows the link instead - a note with a broken image stays readable.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The biggest edge an image is decoded at. A 6000 px photo is 144 MB in memory and about thirty
/// pixels tall on screen; scaling down at load time keeps a vault of them usable.
const MAX_EDGE: u32 = 1600;

/// One decoded picture, and the file version it came from.
struct Cached {
    mtime_ms: u64,
    texture: egui::TextureHandle,
    size: [usize; 2],
}

/// The pictures the reading view has loaded.
#[derive(Default)]
pub struct Images {
    cache: HashMap<PathBuf, Cached>,
    /// Files that failed to decode: tried once, never retried every frame.
    failed: std::collections::HashSet<PathBuf>,
}

impl Images {
    /// The texture for `path`, decoding it when it is new or changed. `None` when the file cannot
    /// be read or is not an image this build understands.
    pub fn texture(&mut self, ctx: &egui::Context, path: &Path) -> Option<(egui::TextureHandle, [usize; 2])> {
        if self.failed.contains(path) {
            return None;
        }
        let mtime_ms = modified_ms(path)?;
        if let Some(cached) = self.cache.get(path)
            && cached.mtime_ms == mtime_ms
        {
            return Some((cached.texture.clone(), cached.size));
        }

        match decode(path) {
            Some((size, rgba)) => {
                let image = egui::ColorImage::from_rgba_unmultiplied(size, &rgba);
                let texture = ctx.load_texture(format!("image:{}", path.display()), image, egui::TextureOptions::LINEAR);
                self.cache.insert(path.to_path_buf(), Cached { mtime_ms, texture: texture.clone(), size });
                Some((texture, size))
            }
            None => {
                log::warn!("image: {} could not be decoded", path.display());
                self.failed.insert(path.to_path_buf());
                None
            }
        }
    }
}

fn modified_ms(path: &Path) -> Option<u64> {
    let time = std::fs::metadata(path).ok()?.modified().ok()?;
    time.duration_since(std::time::UNIX_EPOCH).ok().map(|elapsed| elapsed.as_millis() as u64)
}

/// `(size, rgba8 pixels)` of the file, scaled down to [`MAX_EDGE`].
fn decode(path: &Path) -> Option<([usize; 2], Vec<u8>)> {
    let image = image::open(path).ok()?;
    let image = if image.width() > MAX_EDGE || image.height() > MAX_EDGE { image.thumbnail(MAX_EDGE, MAX_EDGE) } else { image };
    let rgba = image.to_rgba8();
    let (width, height) = rgba.dimensions();
    Some(([width as usize, height as usize], rgba.into_raw()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_that_does_not_exist_has_no_texture() {
        let mut images = Images::default();
        let ctx = egui::Context::default();
        assert!(images.texture(&ctx, Path::new("nao-existe.png")).is_none());
    }

    #[test]
    fn a_file_that_is_not_an_image_is_marked_failed() {
        let mut images = Images::default();
        let path = std::env::temp_dir().join(format!("localobsidian-not-image-{}.png", std::process::id()));
        std::fs::write(&path, b"isto nao e um png").expect("write");
        let ctx = egui::Context::default();
        assert!(images.texture(&ctx, &path).is_none());
        assert!(images.failed.contains(&path));
        let _ = std::fs::remove_file(&path);
    }
}
