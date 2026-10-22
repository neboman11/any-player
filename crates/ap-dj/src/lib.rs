//! Remote AI DJ for Any Player desktop (design §14): passage lookup and receipts, server-side
//! break generation, the `VoiceProcessor` finishing chain, and the diagnostics ring.
//!
//! Ports of Android `:dj` (remote path only). The schedule coordinator lives here too once the
//! playback engine exists; persistence is injected through [`PassageStateStore`].

pub mod account;
pub mod diagnostics;
pub mod passages;
pub mod remote;
mod time;
pub mod voice_processor;

pub use account::DjAccount;
pub use diagnostics::{DjDiagnostics, DjLogEntry};
pub use passages::{
    MemoryPassageStateStore, Passage, PassageClient, PassageStateStore, Passages, StoreError,
    StoreFuture, estimate_tokens, fit_to_budget,
};
pub use remote::{
    CatalogError, RemoteBreakGenerator, VoiceCatalog, VoiceCatalogClient, VoiceDescriptor,
};
pub use voice_processor::{WavError, Wave};
