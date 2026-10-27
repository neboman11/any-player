//! Per-data-directory lock and local show-window notifications.
use fs2::FileExt;
use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions,
    tokio::{Stream, prelude::*},
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io,
    path::Path,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
    task::JoinHandle,
};

pub enum Instance {
    Primary(PrimaryInstance),
    Secondary,
}
pub struct PrimaryInstance {
    _lock: File,
    task: JoinHandle<()>,
}
impl Drop for PrimaryInstance {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub async fn acquire(data_dir: &Path, events: mpsc::UnboundedSender<()>) -> io::Result<Instance> {
    fs::create_dir_all(data_dir)?;
    let path = fs::canonicalize(data_dir)?;
    let digest = Sha256::digest(path.to_string_lossy().as_bytes());
    let name = format!("any-player-{digest:x}");
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path.join("instance.lock"))?;
    match lock.try_lock_exclusive() {
        Ok(()) => {
            let socket_name = name.to_ns_name::<GenericNamespaced>()?;
            let listener = ListenerOptions::new()
                .name(socket_name)
                .try_overwrite(true)
                .create_tokio()?;
            let task = tokio::spawn(async move {
                while let Ok(mut stream) = listener.accept().await {
                    let mut message = [0u8; 4];
                    if stream.read_exact(&mut message).await.is_ok() && &message == b"show" {
                        let _ = events.send(());
                        let _ = stream.write_all(b"ok").await;
                    }
                }
            });
            Ok(Instance::Primary(PrimaryInstance { _lock: lock, task }))
        }
        Err(e) if e.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            for attempt in 0..50 {
                let socket_name = name.as_str().to_ns_name::<GenericNamespaced>()?;
                match Stream::connect(socket_name).await {
                    Ok(mut stream) => {
                        stream.write_all(b"show").await?;
                        let mut response = [0; 2];
                        stream.read_exact(&mut response).await?;
                        return if &response == b"ok" {
                            Ok(Instance::Secondary)
                        } else {
                            Err(io::Error::other("invalid instance response"))
                        };
                    }
                    Err(e) if attempt == 49 => return Err(e),
                    Err(_) => tokio::time::sleep(Duration::from_millis(20)).await,
                }
            }
            Err(io::Error::other("unable to reach primary instance"))
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn second_instance_notifies_first() {
        let dir = tempfile::tempdir().unwrap();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let first = acquire(dir.path(), tx.clone()).await.unwrap();
        assert!(matches!(first, Instance::Primary(_)));
        assert!(matches!(
            acquire(dir.path(), tx).await.unwrap(),
            Instance::Secondary
        ));
        assert_eq!(rx.recv().await, Some(()));
        drop(first);
    }
}
