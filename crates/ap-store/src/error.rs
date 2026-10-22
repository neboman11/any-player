use thiserror::Error;

/// Errors of the database layer and its repositories.
#[derive(Debug, Error)]
pub enum StoreError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    /// The database was written by a newer app version. Startup must abort; never downgrade it.
    #[error("database schema version {found} is newer than the supported version {supported}")]
    SchemaTooNew { found: i64, supported: i64 },
    #[error("migration to schema version {version} failed: {source}")]
    Migration {
        version: i64,
        #[source]
        source: Box<StoreError>,
    },
    #[error("not found: {0}")]
    NotFound(String),
    #[error("invalid operation: {0}")]
    Invalid(String),
    #[error("corrupt data: {0}")]
    Corrupt(String),
    #[error("no {0} directory is available on this platform")]
    NoPlatformDir(&'static str),
    #[error("the database thread has stopped")]
    Closed,
    #[error("a database task panicked")]
    Panicked,
}

pub type Result<T, E = StoreError> = std::result::Result<T, E>;
