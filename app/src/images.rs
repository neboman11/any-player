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
            let image = reader
                .decode()
                .map_err(|e| ServiceError(e.to_string()))?
                .thumbnail(640, 640)
                .to_rgba8();
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

/// 100 MiB disk LRU. Cache hits are touched; eviction and filesystem metadata run off-thread.
pub struct DiskImageCache {
    directory: std::path::PathBuf,
    client: reqwest::Client,
    writer: tokio::sync::Mutex<()>,
}
impl DiskImageCache {
    pub fn new(directory: std::path::PathBuf) -> Self {
        Self {
            directory,
            client: reqwest::Client::new(),
            writer: tokio::sync::Mutex::new(()),
        }
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
        let mut response = self
            .client
            .get(url)
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
                if total <= 100 * 1024 * 1024 {
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
    use slint::ComponentHandle;
    let directory = dirs::cache_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join("any-player/images");
    let loader = ImageLoader::new(Arc::new(DiskImageCache::new(directory)));
    let weak = ui.as_weak();
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
