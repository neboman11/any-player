//! DJ cache and delivery ownership. Active delivery files outlive every cache operation.
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AudioKey {
    pub track_id: String,
    pub chunk_ids: Vec<i64>,
}

/// Removing ownership removes the file, including when a generation task is cancelled.
pub(crate) struct OwnedAudio(pub PathBuf);
impl Drop for OwnedAudio {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct Delivery {
    key: AudioKey,
    _file: OwnedAudio,
    started: bool,
}

#[derive(Default)]
pub(crate) struct DjAudioFiles {
    finished: VecDeque<(AudioKey, OwnedAudio)>,
    deliveries: HashMap<String, Delivery>,
}
impl DjAudioFiles {
    pub fn cached(&self, key: &AudioKey) -> Option<&Path> {
        self.finished
            .iter()
            .find(|(k, _)| k == key)
            .map(|(_, f)| f.0.as_path())
    }
    pub fn remember(&mut self, key: AudioKey, file: OwnedAudio) {
        self.finished.retain(|(k, _)| k != &key);
        self.finished.push_back((key, file));
        while self.finished.len() > 3 {
            self.finished.pop_front();
        }
    }
    pub fn forget_track(&mut self, id: &str) {
        self.finished.retain(|(k, _)| k.track_id != id);
    }
    pub fn deliver(&mut self, id: String, key: AudioKey, file: OwnedAudio) {
        self.deliveries.insert(
            id,
            Delivery {
                key,
                _file: file,
                started: false,
            },
        );
    }
    pub fn started(&mut self, id: &str) -> bool {
        let Some(delivery) = self.deliveries.get_mut(id) else {
            return false;
        };
        if delivery.started {
            return false;
        }
        delivery.started = true;
        self.finished.retain(|(key, _)| key != &delivery.key);
        true
    }
    pub fn dropped(&mut self, id: &str) -> bool {
        if self.deliveries.get(id).is_none_or(|d| d.started) {
            return false;
        }
        self.deliveries.remove(id);
        true
    }
    pub fn ended(&mut self, id: &str) {
        self.deliveries.remove(id);
    }
    pub fn clear_finished(&mut self) {
        self.finished.clear();
    }
}

/// Sweep only coordinator-owned artifacts, preserving unrelated files in this directory.
pub(crate) async fn sweep(root: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(root.join("cache")).await?;
    let mut directories = vec![root.to_path_buf()];
    while let Some(directory) = directories.pop() {
        let mut entries = tokio::fs::read_dir(directory).await?;
        while let Some(entry) = entries.next_entry().await? {
            let kind = entry.file_type().await?;
            if kind.is_dir() {
                directories.push(entry.path());
            } else if kind.is_file() {
                let path = entry.path();
                if path.extension().is_some_and(|e| e == "wav" || e == "part")
                    || matches!(
                        entry.file_name().to_str(),
                        Some("ready-track-id" | "ready-passages.json" | "ready-fact.json")
                    )
                {
                    tokio::fs::remove_file(path).await?;
                }
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn file(root: &Path, name: &str) -> OwnedAudio {
        let path = root.join(name);
        std::fs::write(&path, b"audio").unwrap();
        OwnedAudio(path)
    }
    fn key(id: &str) -> AudioKey {
        AudioKey {
            track_id: id.into(),
            chunk_ids: vec![1],
        }
    }
    #[test]
    fn started_delivery_survives_drop_clear_and_cache_eviction_until_ended() {
        let root = tempfile::tempdir().unwrap();
        let mut files = DjAudioFiles::default();
        files.remember(key("a"), file(root.path(), "cache.wav"));
        let delivered = file(root.path(), "delivery.wav");
        let path = delivered.0.clone();
        files.deliver("break".into(), key("a"), delivered);
        assert!(files.started("break"));
        assert!(!files.started("break"));
        assert!(!files.dropped("break"));
        files.clear_finished();
        assert!(path.exists());
        assert!(!root.path().join("cache.wav").exists());
        files.ended("break");
        assert!(!path.exists());
    }
    #[test]
    fn never_started_drop_deletes_delivery_but_retains_three_cache_entries() {
        let root = tempfile::tempdir().unwrap();
        let mut files = DjAudioFiles::default();
        for id in ["a", "b", "c", "d"] {
            files.remember(key(id), file(root.path(), &format!("{id}.wav")));
        }
        assert!(files.cached(&key("a")).is_none());
        assert!(!root.path().join("a.wav").exists());
        let delivery = file(root.path(), "delivery.wav");
        let path = delivery.0.clone();
        files.deliver("break".into(), key("d"), delivery);
        assert!(files.dropped("break"));
        assert!(!path.exists());
        assert!(files.cached(&key("d")).is_some());
    }
}
