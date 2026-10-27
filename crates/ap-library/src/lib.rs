//! Offline-first catalog, custom/union playlists and Android-compatible transfer files.
//!
//! Construct [`CatalogRepository`] with authenticated provider clients, then share its clone with
//! [`PlaylistRepository`]. Observations are cache-only; opening/playing explicitly requests tracks.
//! Network futures are cancellation-safe (dropping search/materialization cancels their fetches).
//! Source edits commit before fetching, and materialization checks the saved source revision before
//! publishing. [`transfer::TransferService`] handles state/config files and sync namespaces.
//! Connection updates use the caller's credential-store adapter; playback flags are returned.
//! Wire ids and ISO precision use ap-store-owned schema columns; legacy seconds remain authoritative.

pub mod catalog;
pub mod playlists;
pub mod transfer;

pub use catalog::{CatalogRepository, ProviderRefreshFailure, SearchResult, SearchType};
pub use playlists::{PlaybackSelection, PlaylistRepository};

#[derive(Debug, thiserror::Error)]
pub enum LibraryError {
    #[error(transparent)]
    Store(#[from] ap_store::StoreError),
    #[error(transparent)]
    Provider(#[from] ap_providers::ProviderError),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    State(#[from] ap_model::wire::StateFileError),
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Search(String),
}
pub type Result<T, E = LibraryError> = std::result::Result<T, E>;
