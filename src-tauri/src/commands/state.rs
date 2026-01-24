/// Shared application state
use crate::{Database, PlaybackManager, ProviderRegistry};
use std::sync::Arc;
use tokio::sync::Mutex;

pub struct AppState {
    pub playback: Arc<Mutex<PlaybackManager>>,
    pub providers: Arc<Mutex<ProviderRegistry>>,
    pub oauth_code: Arc<Mutex<Option<String>>>,
    pub database: Arc<Mutex<Database>>,
}
