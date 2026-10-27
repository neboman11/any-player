//! Remote AI DJ for Any Player desktop (design §14): passage lookup and receipts, server-side
//! break generation, the `VoiceProcessor` finishing chain, and the diagnostics ring.
//!
//! Ports of Android `:dj` (remote path only). [`DjCoordinator`] owns scheduling and delivery
//! lifetimes; settings and passage persistence are injected through small store ports.

pub mod account;
mod audio_files;
pub mod coordinator;
pub mod diagnostics;
pub mod passages;
pub mod remote;
mod time;
pub mod voice_processor;

pub use account::DjAccount;
pub use coordinator::{
    CoordinatorError, DjController, DjCoordinator, DjCoordinatorDeps, DjSettings, DjSettingsStore,
    DjStatus, MemoryDjSettingsStore, PreparationStatus,
};
pub use diagnostics::{DjDiagnostics, DjLogEntry};
pub use passages::{
    MemoryPassageStateStore, Passage, PassageClient, PassageStateStore, Passages, StoreError,
    StoreFuture, estimate_tokens, fit_to_budget,
};
pub use remote::{
    CatalogError, RemoteBreakGenerator, VoiceCatalog, VoiceCatalogClient, VoiceDescriptor,
};
pub use voice_processor::{WavError, Wave};
