//! Blocking decoder-thread HTTP source, bounded read-ahead and persistent disk LRU.
use crate::ports::StreamRequest;
use reqwest::blocking::{Client, Response};
use reqwest::header::{CONTENT_LENGTH, CONTENT_RANGE, RANGE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::VecDeque;
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use symphonia::core::io::MediaSource;
pub const RANGE_BYTES: usize = 256 * 1024;
pub const READ_AHEAD_BYTES: usize = 2 * 1024 * 1024;
pub const CACHE_BYTES: u64 = 500 * 1024 * 1024;
#[derive(Serialize, Deserialize)]
struct Entry {
    key: String,
    size: u64,
    touched: u64,
}
/// Shared index lock prevents concurrent prefetch and decoder cache commits racing.
#[derive(Clone)]
pub struct AudioCache {
    root: PathBuf,
    lock: Arc<Mutex<()>>,
    limit: u64,
}
impl AudioCache {
    pub fn new(root: PathBuf) -> io::Result<Self> {
        fs::create_dir_all(&root)?;
        Ok(Self {
            root,
            lock: Arc::default(),
            limit: CACHE_BYTES,
        })
    }
    fn hash(key: &str) -> String {
        format!("{:x}", Sha256::digest(key.as_bytes()))
    }
    fn index(&self) -> Vec<Entry> {
        fs::read(self.root.join("index.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }
    fn save_index(&self, entries: &[Entry]) -> io::Result<()> {
        fs::write(self.root.join("index.tmp"), serde_json::to_vec(entries)?)?;
        fs::rename(self.root.join("index.tmp"), self.root.join("index.json"))
    }
    fn touch(entries: &[Entry]) -> u64 {
        entries
            .iter()
            .map(|e| e.touched)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }
    pub fn open(&self, key: &str) -> io::Result<Option<File>> {
        let _guard = self
            .lock
            .lock()
            .map_err(|_| io::Error::other("cache lock poisoned"))?;
        let hash = Self::hash(key);
        let mut entries = self.index();
        let touched = Self::touch(&entries);
        if let Some(entry) = entries.iter_mut().find(|e| e.key == hash) {
            match File::open(self.root.join(&hash)) {
                Ok(file) => {
                    entry.touched = touched;
                    self.save_index(&entries)?;
                    return Ok(Some(file));
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => {
                    entries.retain(|e| e.key != hash);
                    self.save_index(&entries)?;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(None)
    }
    fn commit(&self, key: &str, temporary: &Path, size: u64) -> io::Result<()> {
        if size > self.limit {
            return Ok(());
        }
        let _guard = self
            .lock
            .lock()
            .map_err(|_| io::Error::other("cache lock poisoned"))?;
        let hash = Self::hash(key);
        let mut entries = self.index();
        let touched = Self::touch(&entries);
        entries.retain(|e| e.key != hash);
        fs::copy(temporary, self.root.join(&hash))?;
        entries.push(Entry {
            key: hash,
            size,
            touched,
        });
        entries.sort_by_key(|e| e.touched);
        let mut total: u64 = entries.iter().map(|e| e.size).sum();
        while total > self.limit && !entries.is_empty() {
            let entry = entries.remove(0);
            total -= entry.size;
            match fs::remove_file(self.root.join(entry.key)) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
        }
        self.save_index(&entries)
    }
}
/// Range chunks are held in an eight-chunk sliding window. Servers ignoring Range retain a
/// sequential response; forward seeks discard bytes, backward seeks reopen from zero.
pub struct HttpSource {
    client: Client,
    request: StreamRequest,
    position: u64,
    length: Option<u64>,
    chunks: VecDeque<(u64, Vec<u8>)>,
    sequential: Option<Response>,
    sequential_at: u64,
    cache: AudioCache,
    key: String,
    spool: File,
    spool_path: PathBuf,
    contiguous: u64,
    cached: Option<File>,
    committed: bool,
    read_ahead_chunks: usize,
}
impl HttpSource {
    pub fn new(request: StreamRequest, key: String, cache: AudioCache) -> io::Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let cached = cache.open(&key)?;
        let length = cached
            .as_ref()
            .and_then(|f| f.metadata().ok())
            .map(|m| m.len());
        let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let spool_path = cache
            .root
            .join(format!(".download-{}-{nonce}", std::process::id()));
        let spool = File::create(&spool_path)?;
        Ok(Self {
            client: Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .danger_accept_invalid_certs(request.allow_invalid_certs)
                .build()
                .map_err(io::Error::other)?,
            request,
            position: 0,
            length,
            chunks: VecDeque::new(),
            sequential: None,
            sequential_at: 0,
            cache,
            key,
            spool,
            spool_path,
            contiguous: 0,
            cached,
            committed: false,
            read_ahead_chunks: READ_AHEAD_BYTES / RANGE_BYTES,
        })
    }
    fn request_at(&mut self, start: u64) -> io::Result<Vec<u8>> {
        let mut response = self
            .client
            .get(self.request.url.clone())
            .headers(self.request.headers.clone())
            .header(
                RANGE,
                format!("bytes={start}-{}", start + RANGE_BYTES as u64 - 1),
            )
            .send()
            .map_err(io::Error::other)?;
        if response.status() == reqwest::StatusCode::RANGE_NOT_SATISFIABLE {
            self.length = Some(start);
            return Ok(vec![]);
        }
        if !response.status().is_success() {
            return Err(io::Error::other(format!(
                "Audio HTTP {}",
                response.status()
            )));
        }
        if response.status() == reqwest::StatusCode::PARTIAL_CONTENT {
            if let Some(value) = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|h| h.to_str().ok())
            {
                let prefix = format!("bytes {start}-");
                if !value.starts_with(&prefix) {
                    return Err(io::Error::other("Invalid audio Content-Range"));
                }
                self.length = value.rsplit('/').next().and_then(|v| v.parse().ok());
            }
            let mut bytes = Vec::with_capacity(RANGE_BYTES);
            response.take(RANGE_BYTES as u64).read_to_end(&mut bytes)?;
            Ok(bytes)
        } else {
            self.length = response
                .headers()
                .get(CONTENT_LENGTH)
                .and_then(|h| h.to_str().ok())
                .and_then(|v| v.parse().ok());
            self.sequential_at = 0;
            let skipped = io::copy(&mut response.by_ref().take(start), &mut io::sink())?;
            self.sequential_at = skipped;
            self.sequential = Some(response);
            self.read_sequential()
        }
    }
    fn read_sequential(&mut self) -> io::Result<Vec<u8>> {
        let mut bytes = Vec::with_capacity(RANGE_BYTES);
        if let Some(response) = &mut self.sequential {
            response.take(RANGE_BYTES as u64).read_to_end(&mut bytes)?;
        }
        self.sequential_at += bytes.len() as u64;
        Ok(bytes)
    }
    fn fill(&mut self, start: u64) -> io::Result<()> {
        if let Some(response) = &mut self.sequential
            && self.sequential_at < start
        {
            let skipped = io::copy(
                &mut response.take(start - self.sequential_at),
                &mut io::sink(),
            )?;
            self.sequential_at += skipped;
        }
        let bytes = if self.sequential.is_some() && self.sequential_at == start {
            self.read_sequential()?
        } else {
            self.request_at(start)?
        };
        if start == self.contiguous {
            self.spool.write_all(&bytes)?;
            self.contiguous += bytes.len() as u64;
            if self.length == Some(self.contiguous) && !self.committed {
                self.spool.flush()?;
                self.cache
                    .commit(&self.key, &self.spool_path, self.contiguous)?;
                self.committed = true;
            }
        }
        if bytes.is_empty() {
            self.length = Some(start);
        }
        if self.chunks.len() == READ_AHEAD_BYTES / RANGE_BYTES {
            self.chunks.pop_front();
        }
        self.chunks.push_back((start, bytes));
        Ok(())
    }
    /// Fetch first MiB, or the whole object when its declared size is less than 50 MiB.
    pub fn prefetch(&mut self) -> io::Result<()> {
        self.read_ahead_chunks = 1024 * 1024 / RANGE_BYTES;
        let mut buffer = vec![0_u8; 32 * 1024];
        let mut read = 0;
        loop {
            let n = self.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            read += n;
            let target = self
                .length
                .filter(|n| *n < 50 * 1024 * 1024)
                .unwrap_or(1024 * 1024);
            if read as u64 >= target {
                break;
            }
        }
        self.seek(SeekFrom::Start(0))?;
        self.read_ahead_chunks = READ_AHEAD_BYTES / RANGE_BYTES;
        Ok(())
    }
}
impl Drop for HttpSource {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.spool_path);
    }
}
impl Read for HttpSource {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() {
            return Ok(0);
        }
        if let Some(file) = &mut self.cached {
            let n = file.read(output)?;
            self.position += n as u64;
            return Ok(n);
        }
        if self.length.is_some_and(|len| self.position >= len) {
            return Ok(0);
        }
        let start = self.position / RANGE_BYTES as u64 * RANGE_BYTES as u64;
        if !self.chunks.iter().any(|(at, _)| *at == start) {
            self.chunks.clear();
            for chunk in 0..self.read_ahead_chunks {
                let at = start + (chunk * RANGE_BYTES) as u64;
                if chunk > 0 && self.length.is_some_and(|len| at >= len) {
                    break;
                }
                self.fill(at)?;
                if self
                    .chunks
                    .back()
                    .is_some_and(|(_, bytes)| bytes.is_empty())
                {
                    break;
                }
            }
        }
        let (_, bytes) = self
            .chunks
            .iter()
            .find(|(at, _)| *at == start)
            .expect("filled");
        let offset = usize::try_from(self.position - start).map_err(io::Error::other)?;
        let n = output.len().min(bytes.len().saturating_sub(offset));
        output[..n].copy_from_slice(&bytes[offset..offset + n]);
        self.position += n as u64;
        Ok(n)
    }
}
impl Seek for HttpSource {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        if let Some(file) = &mut self.cached {
            self.position = file.seek(from)?;
            return Ok(self.position);
        }
        let position = match from {
            SeekFrom::Start(n) => i128::from(n),
            SeekFrom::Current(n) => i128::from(self.position) + i128::from(n),
            SeekFrom::End(n) => {
                i128::from(self.length.ok_or_else(|| {
                    io::Error::new(io::ErrorKind::Unsupported, "Unknown stream length")
                })?) + i128::from(n)
            }
        };
        self.position = u64::try_from(position)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "Invalid audio seek"))?;
        Ok(self.position)
    }
}
impl MediaSource for HttpSource {
    fn is_seekable(&self) -> bool {
        true
    }
    fn byte_len(&self) -> Option<u64> {
        self.length
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disk_cache_is_lru_and_reopens_persisted_index() {
        let root = std::env::temp_dir().join(format!("ap-cache-lru-{}", std::process::id()));
        let mut cache = AudioCache::new(root.clone()).unwrap();
        cache.limit = 8;
        let source = root.join("test-input");
        fs::write(&source, [1_u8; 4]).unwrap();
        cache.commit("PLEX:a", &source, 4).unwrap();
        cache.commit("PLEX:b", &source, 4).unwrap();
        assert!(cache.open("PLEX:a").unwrap().is_some());
        cache.commit("JELLYFIN:c", &source, 4).unwrap();
        assert!(cache.open("PLEX:b").unwrap().is_none());
        assert!(cache.open("PLEX:a").unwrap().is_some());
        let reopened = AudioCache::new(root.clone()).unwrap();
        assert!(reopened.open("JELLYFIN:c").unwrap().is_some());
        assert_eq!(reopened.index().iter().map(|e| e.size).sum::<u64>(), 8);
        assert!(root.join(AudioCache::hash("PLEX:a")).is_file());
        fs::remove_dir_all(root).unwrap();
    }
}
