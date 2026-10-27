//! Artwork decoding stays off the UI thread; Slint images remain main-thread-only.
use crate::vm::{Result, ServiceError};
use async_trait::async_trait;
use lru::LruCache;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::{cell::RefCell, num::NonZeroUsize, rc::Rc, sync::Arc};

/// Disk-cache/network adapter supplied by application startup. Bytes must be capped at 8 MiB.
#[async_trait]
pub trait ImageCache: Send + Sync {
    async fn load(&self, url: &str) -> Result<Vec<u8>>;
}
#[derive(Clone)]
pub struct ImageLoader {
    memory: Rc<RefCell<LruCache<String, Image>>>,
    cache: Arc<dyn ImageCache>,
}
impl ImageLoader {
    pub fn new(cache: Arc<dyn ImageCache>) -> Self {
        Self {
            memory: Rc::new(RefCell::new(LruCache::new(
                NonZeroUsize::new(200).expect("nonzero capacity"),
            ))),
            cache,
        }
    }
    pub fn cached(&self, url: &str) -> Option<Image> {
        self.memory.borrow_mut().get(url).cloned()
    }
    pub fn remember(&self, url: String, image: Image) {
        self.memory.borrow_mut().put(url, image);
    }
    pub async fn decode(
        cache: Arc<dyn ImageCache>,
        url: String,
    ) -> Result<SharedPixelBuffer<Rgba8Pixel>> {
        Self::decode_sized(cache, url, 640).await
    }
    async fn decode_sized(
        cache: Arc<dyn ImageCache>,
        url: String,
        size: u32,
    ) -> Result<SharedPixelBuffer<Rgba8Pixel>> {
        let bytes = cache.load(&url).await?;
        if bytes.len() > 8 * 1024 * 1024 {
            return Err(ServiceError("Artwork exceeds 8 MiB".into()));
        }
        tokio::task::spawn_blocking(move || {
            let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes))
                .with_guessed_format()
                .map_err(|e| ServiceError(e.to_string()))?;
            let mut limits = image::Limits::default();
            limits.max_image_width = Some(4096);
            limits.max_image_height = Some(4096);
            limits.max_alloc = Some(64 * 1024 * 1024);
            reader.limits(limits);
            let mut image = reader
                .decode()
                .map_err(|e| ServiceError(e.to_string()))?
                .thumbnail(size, size)
                .to_rgba8();
            round_corners(&mut image);
            Ok(SharedPixelBuffer::clone_from_slice(
                image.as_raw(),
                image.width(),
                image.height(),
            ))
        })
        .await
        .map_err(|e| ServiceError(e.to_string()))?
    }
    pub fn backing_cache(&self) -> Arc<dyn ImageCache> {
        self.cache.clone()
    }
}

// Bake the mask once off-thread: native clipping backends do not all honor
// rounded rectangle clips for image children.
fn round_corners(image: &mut image::RgbaImage) {
    let (width, height) = image.dimensions();
    let radius = width.min(height) / 20;
    for y in 0..radius {
        for x in 0..radius {
            // Four-by-four subpixel coverage, with integer arithmetic.
            let mut coverage = 0_u16;
            let scaled_radius = i64::from(radius) * 8;
            for sy in [1, 3, 5, 7] {
                for sx in [1, 3, 5, 7] {
                    let dx = i64::from(radius - x) * 8 - sx;
                    let dy = i64::from(radius - y) * 8 - sy;
                    coverage += u16::from(dx * dx + dy * dy <= scaled_radius * scaled_radius);
                }
            }
            for (px, py) in [
                (x, y),
                (width - 1 - x, y),
                (x, height - 1 - y),
                (width - 1 - x, height - 1 - y),
            ] {
                let pixel = image.get_pixel_mut(px, py);
                pixel.0[3] = u8::try_from(u16::from(pixel.0[3]) * coverage / 16)
                    .expect("coverage cannot increase alpha");
            }
        }
    }
}

/// Disk LRU (100 MiB by default). Cache hits are touched; eviction and filesystem metadata run
/// off-thread. Provider artwork auth is resolved per request and never becomes part of the key.
pub struct DiskImageCache {
    directory: std::path::PathBuf,
    client: reqwest::Client,
    writer: tokio::sync::Mutex<()>,
    providers: Option<Arc<crate::services::providers::ProvidersAdapter>>,
    capacity: u64,
}
impl DiskImageCache {
    pub fn new(directory: std::path::PathBuf) -> Self {
        Self {
            directory,
            client: reqwest::Client::new(),
            writer: tokio::sync::Mutex::new(()),
            providers: None,
            capacity: 100 * 1024 * 1024,
        }
    }
    pub fn authenticated(
        directory: std::path::PathBuf,
        providers: Arc<crate::services::providers::ProvidersAdapter>,
    ) -> Self {
        Self {
            providers: Some(providers),
            ..Self::new(directory)
        }
    }
    #[must_use]
    pub fn with_capacity(mut self, bytes: u64) -> Self {
        self.capacity = bytes;
        self
    }
}
#[async_trait]
impl ImageCache for DiskImageCache {
    async fn load(&self, url: &str) -> Result<Vec<u8>> {
        use sha2::{Digest, Sha256};
        let key = format!("{:x}", Sha256::digest(url.as_bytes()));
        let path = self.directory.join(key);
        let _writer = self.writer.lock().await;
        if let Ok(bytes) = tokio::fs::read(&path).await
            && bytes.len() <= 8 * 1024 * 1024
        {
            let touch = path.clone();
            let _ = tokio::task::spawn_blocking(move || {
                std::fs::OpenOptions::new()
                    .write(true)
                    .open(touch)
                    .and_then(|f| f.set_modified(std::time::SystemTime::now()))
            })
            .await;
            return Ok(bytes);
        }
        let request = self.providers.as_ref().and_then(|p| p.image_request(url));
        let (url, headers) = request.map_or_else(
            || {
                (
                    ap_providers::provider::strip_url_tokens(url),
                    reqwest::header::HeaderMap::new(),
                )
            },
            |request| (request.url.to_string(), request.headers),
        );
        let mut response = self
            .client
            .get(url)
            .headers(headers)
            .timeout(std::time::Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| ServiceError(e.to_string()))?
            .error_for_status()
            .map_err(|e| ServiceError(e.to_string()))?;
        if response
            .content_length()
            .is_some_and(|n| n > 8 * 1024 * 1024)
        {
            return Err(ServiceError("Artwork exceeds 8 MiB".into()));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| ServiceError(e.to_string()))?
        {
            if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err(ServiceError("Artwork exceeds 8 MiB".into()));
            }
            bytes.extend_from_slice(&chunk);
        }
        tokio::fs::create_dir_all(&self.directory)
            .await
            .map_err(|e| ServiceError(e.to_string()))?;
        tokio::fs::write(&path, &bytes)
            .await
            .map_err(|e| ServiceError(e.to_string()))?;
        let directory = self.directory.clone();
        let capacity = self.capacity;
        tokio::task::spawn_blocking(move || {
            let mut entries: Vec<_> = std::fs::read_dir(directory)
                .into_iter()
                .flatten()
                .filter_map(std::result::Result::ok)
                .filter_map(|e| {
                    e.metadata()
                        .ok()
                        .filter(std::fs::Metadata::is_file)
                        .map(|m| {
                            (
                                m.modified().unwrap_or(std::time::UNIX_EPOCH),
                                m.len(),
                                e.path(),
                            )
                        })
                })
                .collect();
            entries.sort_by_key(|e| e.0);
            let mut total: u64 = entries.iter().map(|e| e.1).sum();
            for (_, len, path) in entries {
                if total <= capacity {
                    break;
                }
                if std::fs::remove_file(path).is_ok() {
                    total = total.saturating_sub(len);
                }
            }
        })
        .await
        .map_err(|e| ServiceError(e.to_string()))?;
        Ok(bytes)
    }
}

pub(crate) fn attach(ui: &crate::MainWindow) {
    let directory = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("any-player/images");
    attach_cache(ui, Arc::new(DiskImageCache::new(directory)));
}

fn visible_urls(model: &slint::ModelRc<crate::TrackRow>, first: i32, last: i32) -> Vec<String> {
    use slint::Model;
    let start = usize::try_from(first.max(0)).unwrap_or(0).min(model.row_count());
    let end = usize::try_from(last.max(first).max(0)).unwrap_or(0).min(model.row_count());
    (start..end)
        .filter_map(|index| model.row_data(index))
        .filter(|row| !row.image_url.is_empty())
        .map(|row| row.image_url.to_string())
        .collect()
}

fn apply_thumbnail(ui: &crate::MainWindow, url: &str, image: &Image) {
    use slint::Model;
    if ui.get_artwork_url() == url {
        ui.set_thumbnail(image.clone());
        ui.set_thumbnail_present(true);
    }
    let rows = ui.get_queue();
    let art = ui.get_queue_art();
    let start = usize::try_from(ui.get_queue_first().max(0)).unwrap_or(0).min(rows.row_count());
    let end = usize::try_from(ui.get_queue_last().max(0)).unwrap_or(0).min(rows.row_count());
    for index in start..end {
        if rows.row_data(index).is_some_and(|row| row.image_url == url) {
            art.set_row_data(index, crate::generated::QueueArt { image: image.clone(), present: true });
        }
    }
}

pub fn attach_cache(ui: &crate::MainWindow, cache: Arc<dyn ImageCache>) {
    use slint::ComponentHandle;
    let loader = ImageLoader::new(cache);
    let weak = ui.as_weak();
    let thumbnails = ImageLoader::new(loader.backing_cache());
    let pending = Rc::new(RefCell::new(std::collections::HashSet::<String>::new()));
    let ready_thumbnails = thumbnails.clone();
    let ready_pending = pending.clone();
    let thumbnail_ui = ui.as_weak();
    ui.on_thumbnail_ready(move |url, image, success| {
        ready_pending.borrow_mut().remove(url.as_str());
        if success {
            ready_thumbnails.remember(url.to_string(), image.clone());
            if let Some(ui) = thumbnail_ui.upgrade() {
                apply_thumbnail(&ui, &url, &image);
            }
        }
    });
    let thumbnail_ui = ui.as_weak();
    ui.on_thumbnail_requested(move |url| {
        let Some(ui) = thumbnail_ui.upgrade() else { return };
        if let Some(image) = thumbnails.cached(&url) {
            apply_thumbnail(&ui, &url, &image);
        } else if pending.borrow_mut().insert(url.to_string()) {
            let cache = thumbnails.backing_cache();
            let weak = thumbnail_ui.clone();
            tokio::spawn(async move {
                let decoded = ImageLoader::decode_sized(cache, url.to_string(), 96).await;
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    let success = decoded.is_ok();
                    let image = decoded.map(Image::from_rgba8).unwrap_or_default();
                    ui.invoke_thumbnail_ready(url, image, success);
                });
            });
        }
    });
    let thumbnail_ui = ui.as_weak();
    ui.on_queue_window(move |first, last| {
        let Some(ui) = thumbnail_ui.upgrade() else { return };
        for url in visible_urls(&ui.get_queue(), first, last) {
            ui.invoke_thumbnail_requested(url.into());
        }
    });
    let ready = loader.clone();
    ui.on_artwork_ready(move |url, image| {
        ready.remember(url.to_string(), image.clone());
        if let Some(ui) = weak.upgrade()
            && ui.get_artwork_url() == url
        {
            ui.set_artwork(image);
            ui.set_artwork_present(true);
        }
    });
    let weak = ui.as_weak();
    ui.on_artwork_requested(move |url| {
        if let Some(image) = loader.cached(&url) {
            if let Some(ui) = weak.upgrade() {
                ui.invoke_artwork_ready(url, image);
            }
            return;
        }
        let cache = loader.backing_cache();
        let weak = weak.clone();
        tokio::spawn(async move {
            if let Ok(buffer) = ImageLoader::decode(cache, url.to_string()).await {
                let _ = weak.upgrade_in_event_loop(move |ui| {
                    ui.invoke_artwork_ready(url, Image::from_rgba8(buffer));
                });
            }
        });
    });
}
#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn only_visible_queue_window_requests_artwork() {
        use std::sync::{Arc, Mutex};
        struct RecordingCache(Mutex<Vec<String>>);
        #[async_trait::async_trait]
        impl super::ImageCache for RecordingCache {
            async fn load(&self, url: &str) -> crate::vm::Result<Vec<u8>> {
                self.0.lock().unwrap().push(url.to_owned());
                Err(crate::vm::ServiceError("test".into()))
            }
        }
        let rows: Vec<_> = (0..1000).map(|i| crate::TrackRow {
            image_url: format!("art-{i}").into(),
            ..Default::default()
        }).collect();
        let model: slint::ModelRc<_> = std::rc::Rc::new(slint::VecModel::from(rows)).into();
        let cache = Arc::new(RecordingCache(Mutex::new(Vec::new())));
        for url in super::visible_urls(&model, 120, 128) {
            let _ = super::ImageLoader::decode_sized(cache.clone(), url, 96).await;
        }
        assert_eq!(*cache.0.lock().unwrap(), (120..128).map(|i| format!("art-{i}")).collect::<Vec<_>>());
    }

    #[test]
    fn artwork_mask_rounds_corners_without_changing_center() {
        let mut image = image::RgbaImage::from_pixel(100, 80, image::Rgba([100, 150, 200, 255]));
        super::round_corners(&mut image);
        for (x, y) in [(0, 0), (99, 0), (0, 79), (99, 79)] {
            assert_eq!(image.get_pixel(x, y).0[3], 0);
        }
        assert_eq!(image.get_pixel(50, 40).0, [100, 150, 200, 255]);
        assert_eq!(image.get_pixel(50, 0).0, [100, 150, 200, 255]);
    }

    #[test]
    fn tiny_artwork_does_not_underflow() {
        let mut image = image::RgbaImage::from_pixel(1, 1, image::Rgba([1, 2, 3, 255]));
        super::round_corners(&mut image);
        assert_eq!(image.get_pixel(0, 0).0, [1, 2, 3, 255]);
    }
}
