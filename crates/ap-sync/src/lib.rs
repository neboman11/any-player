//! Android-compatible account sync: authenticated transport, merge bases and realtime actor.
//!
//! Construct [`SyncEngine`] with playback, library, settings, credentials and network services,
//! then call [`SyncEngine::start`] once. Settings UI persists target/credentials before
//! [`SyncEngine::connect`]; edits alone never redirect an already-confirmed account. Observe
//! [`SyncUiState`] for account conflict and playlist overwrite choices. Dismissing an account
//! conflict disables automatic synchronization until Connect or the next application start;
//! dismissing a playlist choice pauses only that domain.
//!
//! Playback pushes originate only from discrete user changes, after observing a nonempty queue
//! since restore, with a 3.5-second remote-apply echo guard and two-second minimum interval.
//! Domain exports compare against credential-keyed SQLite bases; playlist comparison, import
//! and base recording share one writer transaction. Provider URL side effects occur only after
//! its commit. Transport parses response text directly into [`ap_model::json::JsonElement`],
//! preserving numeric literals needed for Android-compatible canonical signatures.
mod client;
mod engine;
mod network;
use ap_model::json::JsonElement;
pub use client::{SyncClient, SyncResponse, SyncSocket, SyncTarget, SyncUpdate};
pub use engine::{SyncDeps, SyncEngine, SyncUiState};
pub use network::network_availability;

/// Host hook for Android `SyncImports`: a sync import changed this provider's stored base URL,
/// so the host re-validates it with the stored credential (returning `Ok` without connecting
/// when no credential is stored). `Err` carries the bare failure message; the engine reports it
/// as `Provider refresh failed: <SOURCE>: <message>` in the import status.
#[async_trait::async_trait]
pub trait ProviderRevalidation: Send + Sync {
    async fn revalidate(&self, source: ap_model::SourceType) -> Result<(), String>;
}

#[derive(Debug, thiserror::Error)]
pub enum SyncError {
    #[error(transparent)]
    Http(#[from] reqwest::Error),
    #[error(transparent)]
    Socket(Box<tokio_tungstenite::tungstenite::Error>),
    #[error(transparent)]
    Json(#[from] ap_model::json::JsonSyntaxError),
    #[error(transparent)]
    Serde(#[from] serde_json::Error),
    #[error(transparent)]
    Store(#[from] ap_store::StoreError),
    #[error(transparent)]
    Library(#[from] ap_library::LibraryError),
    #[error("invalid authorization header")]
    Authorization,
    #[error("Sync server target is not set.")]
    TargetNotSet,
    #[error("Invalid sync response")]
    InvalidResponse,
    #[error("unknown sync namespace: {0}")]
    Namespace(String),
    #[error("Sync server unavailable.")]
    Unavailable,
    #[error("credential persistence failed: {0}")]
    Credentials(String),
}
impl From<tokio_tungstenite::tungstenite::Error> for SyncError {
    fn from(error: tokio_tungstenite::tungstenite::Error) -> Self {
        Self::Socket(Box::new(error))
    }
}
pub type Result<T, E = SyncError> = std::result::Result<T, E>;
pub(crate) fn object<const N: usize>(items: [(&str, JsonElement); N]) -> JsonElement {
    JsonElement::Object(items.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}
pub(crate) fn field<'a>(value: &'a JsonElement, key: &str) -> &'a JsonElement {
    value
        .as_object()
        .and_then(|o| o.get(key))
        .unwrap_or(&JsonElement::Null)
}
pub(crate) fn text(value: &JsonElement) -> Option<&str> {
    if let JsonElement::String(s) = value {
        Some(s)
    } else {
        None
    }
}
pub(crate) fn boolean(value: &JsonElement) -> Option<bool> {
    match value {
        JsonElement::Bool(v) => Some(*v),
        JsonElement::String(v) => v.parse().ok(),
        _ => None,
    }
}
