//! The database handle (design §4, D7).
//!
//! One dedicated OS thread owns the only writer connection. [`Db::call`] ships a closure to it
//! over a channel and awaits the result, so writes are serialized without holding async locks.
//! [`Db::read`] runs a closure on one of [`READER_COUNT`] read-only connections inside
//! `spawn_blocking`; WAL lets readers proceed while the writer commits.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use rusqlite::{Connection, OpenFlags};
use tokio::sync::{Semaphore, oneshot};

use crate::error::{Result, StoreError};
use crate::migrations;

/// Number of pooled read-only connections.
pub const READER_COUNT: usize = 3;
const BUSY_TIMEOUT: Duration = Duration::from_secs(5);

type Job = Box<dyn FnOnce(&mut Connection) + Send + 'static>;

/// Cloneable handle to the database. The DB thread exits once every handle is dropped and the
/// queued jobs have run.
#[derive(Clone)]
pub struct Db {
    inner: Arc<Inner>,
}

struct Inner {
    path: PathBuf,
    jobs: mpsc::Sender<Job>,
    readers: Mutex<Vec<Connection>>,
    permits: Arc<Semaphore>,
}

impl std::fmt::Debug for Db {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Db")
            .field("path", &self.inner.path)
            .finish_non_exhaustive()
    }
}

impl Db {
    /// Opens (creating if needed) the database at `path`, enables WAL, runs the schema
    /// migrations on the DB thread and opens the reader pool. A migration failure is returned
    /// here and leaves the file untouched (each step is one transaction).
    pub async fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        let (ready_tx, ready_rx) = oneshot::channel();
        let (jobs_tx, jobs_rx) = mpsc::channel::<Job>();
        let writer_path = path.clone();
        std::thread::Builder::new()
            .name("ap-store-db".into())
            .spawn(move || match open_writer(&writer_path) {
                Ok(conn) => {
                    let _ = ready_tx.send(Ok(()));
                    run_writer(conn, &jobs_rx);
                }
                Err(e) => {
                    let _ = ready_tx.send(Err(e));
                }
            })?;
        ready_rx.await.map_err(|_| StoreError::Panicked)??;

        let reader_path = path.clone();
        let readers = tokio::task::spawn_blocking(move || {
            (0..READER_COUNT)
                .map(|_| open_reader(&reader_path))
                .collect::<Result<Vec<_>>>()
        })
        .await
        .map_err(|_| StoreError::Panicked)??;

        Ok(Self {
            inner: Arc::new(Inner {
                path,
                jobs: jobs_tx,
                readers: Mutex::new(readers),
                permits: Arc::new(Semaphore::new(READER_COUNT)),
            }),
        })
    }

    pub fn path(&self) -> &Path {
        &self.inner.path
    }

    fn submit<F, R>(&self, f: F, reply: impl FnOnce(Result<R>) + Send + 'static) -> Result<()>
    where
        F: FnOnce(&mut Connection) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let job: Job = Box::new(move |conn| {
            let result =
                catch_unwind(AssertUnwindSafe(|| f(conn))).unwrap_or(Err(StoreError::Panicked));
            if result.is_err() && !conn.is_autocommit() {
                let _ = conn.execute_batch("ROLLBACK");
            }
            reply(result);
        });
        self.inner.jobs.send(job).map_err(|_| StoreError::Closed)
    }

    /// Runs `f` on the writer connection (DB thread) and returns its result.
    pub async fn call<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut Connection) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.submit(f, move |r| {
            let _ = tx.send(r);
        })?;
        rx.await.map_err(|_| StoreError::Closed)?
    }

    /// Blocking variant of [`Db::call`] for non-async contexts (e.g. a final save on quit).
    /// Must not be called from the DB thread itself or from inside an async task.
    pub fn call_blocking<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&mut Connection) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = mpsc::sync_channel(1);
        self.submit(f, move |r| {
            let _ = tx.send(r);
        })?;
        rx.recv().map_err(|_| StoreError::Closed)?
    }

    /// Runs `f` on a pooled read-only connection inside one read transaction (a consistent
    /// snapshot across statements). Writes fail with `SQLITE_READONLY`.
    pub async fn read<F, R>(&self, f: F) -> Result<R>
    where
        F: FnOnce(&Connection) -> Result<R> + Send + 'static,
        R: Send + 'static,
    {
        let permit = Arc::clone(&self.inner.permits)
            .acquire_owned()
            .await
            .map_err(|_| StoreError::Closed)?;
        let inner = Arc::clone(&self.inner);
        tokio::task::spawn_blocking(move || {
            let conn = inner
                .readers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop()
                .expect("a semaphore permit guarantees a free reader");
            let result = catch_unwind(AssertUnwindSafe(|| -> Result<R> {
                conn.execute_batch("BEGIN")?;
                match f(&conn) {
                    Ok(value) => {
                        conn.execute_batch("COMMIT")?;
                        Ok(value)
                    }
                    Err(error) => {
                        conn.execute_batch("ROLLBACK")?;
                        Err(error)
                    }
                }
            }))
            .unwrap_or(Err(StoreError::Panicked));
            if !conn.is_autocommit() {
                let _ = conn.execute_batch("ROLLBACK");
            }
            inner
                .readers
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(conn);
            drop(permit);
            result
        })
        .await
        .map_err(|_| StoreError::Panicked)?
    }
}

fn open_writer(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
        std::fs::create_dir_all(parent)?;
    }
    let mut conn = Connection::open(path)?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    let mode: String = conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get(0))?;
    if !mode.eq_ignore_ascii_case("wal") {
        tracing::warn!(mode, "SQLite refused WAL journal mode");
    }
    conn.pragma_update(None, "synchronous", "NORMAL")?;
    migrations::migrate(&mut conn)?;
    Ok(conn)
}

fn open_reader(path: &Path) -> Result<Connection> {
    let conn = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_NO_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )?;
    conn.busy_timeout(BUSY_TIMEOUT)?;
    Ok(conn)
}

fn run_writer(mut conn: Connection, jobs: &mpsc::Receiver<Job>) {
    while let Ok(job) = jobs.recv() {
        job(&mut conn);
    }
    // Last handle dropped: fold the WAL back so the file is self-contained.
    if let Err(e) = conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE)") {
        tracing::debug!(error = %e, "WAL checkpoint on close failed");
    }
}

/// Runs `f` atomically. Uses a savepoint, so it starts a transaction when none is open and
/// nests inside the caller's transaction otherwise. Rolled back when `f` fails.
pub fn in_tx<T>(conn: &Connection, f: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
    conn.execute_batch("SAVEPOINT ap_store_tx")?;
    let rollback = || {
        let _ = conn.execute_batch("ROLLBACK TO ap_store_tx; RELEASE ap_store_tx");
    };
    match catch_unwind(AssertUnwindSafe(|| f(conn))) {
        Ok(Ok(v)) => match conn.execute_batch("RELEASE ap_store_tx") {
            Ok(()) => Ok(v),
            Err(e) => {
                rollback();
                Err(e.into())
            }
        },
        Ok(Err(e)) => {
            rollback();
            Err(e)
        }
        Err(panic) => {
            rollback();
            std::panic::resume_unwind(panic)
        }
    }
}
