//! Credentials in the OS keyring (design §6.4), service `any-player`, same accounts and value
//! formats as the legacy desktop:
//!
//! | account | value |
//! |---|---|
//! | `spotify-token` | rspotify 0.16 `Token` JSON, stored verbatim (see [`SecretAccount::SpotifyToken`]) |
//! | `jellyfin-url`, `jellyfin-api-key` | raw strings |
//! | `plex-url`, `plex-token` | raw strings |
//! | `sync-token` | sync bearer token (new) |
//!
//! [`Secrets`] loads every account once on a blocking thread and exposes the result as
//! observable in-memory state with [`Secrets::await_loaded`]. Before loading completes the
//! snapshot is empty with `loaded == false`, which must not be read as "disconnected". A
//! failing keyring is reported per account in [`SecretsState::errors`] (the app shows a
//! warning with Retry = [`Secrets::load`]); nothing ever falls back to plaintext storage.
//!
//! The backend is a trait so tests run against [`MemoryBackend`] without a keyring daemon.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, PoisonError};

use thiserror::Error;
use tokio::sync::watch;

/// Keyring service name (legacy).
pub const SERVICE: &str = "any-player";

/// A keyring account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum SecretAccount {
    /// JSON of the legacy rspotify 0.16 `Token`, read and written unchanged:
    /// `{"access_token":"…","expires_in":3600,"expires_at":"2024-05-01T12:00:00.123456789Z",
    /// "refresh_token":"…"|null,"scope":"a b c"}`. Note the key is `scope` (a space-separated
    /// string), as rspotify renames the `scopes` field. `ap-providers` owns the typed form.
    SpotifyToken,
    JellyfinUrl,
    JellyfinApiKey,
    PlexUrl,
    PlexToken,
    SyncToken,
}

impl SecretAccount {
    pub const ALL: [Self; 6] = [
        Self::SpotifyToken,
        Self::JellyfinUrl,
        Self::JellyfinApiKey,
        Self::PlexUrl,
        Self::PlexToken,
        Self::SyncToken,
    ];

    /// The keyring account (user) name.
    pub const fn account(self) -> &'static str {
        match self {
            Self::SpotifyToken => "spotify-token",
            Self::JellyfinUrl => "jellyfin-url",
            Self::JellyfinApiKey => "jellyfin-api-key",
            Self::PlexUrl => "plex-url",
            Self::PlexToken => "plex-token",
            Self::SyncToken => "sync-token",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SecretsError {
    #[error("keyring access for {account} failed: {message}")]
    Keyring {
        account: &'static str,
        message: String,
    },
    #[error("the keyring task failed")]
    TaskFailed,
}

/// Raw credential storage. Implementations are blocking.
pub trait SecretBackend: Send + Sync + 'static {
    /// `Ok(None)` when the entry does not exist.
    fn get(&self, account: SecretAccount) -> Result<Option<String>, SecretsError>;
    fn set(&self, account: SecretAccount, value: &str) -> Result<(), SecretsError>;
    /// Deleting a missing entry succeeds.
    fn delete(&self, account: SecretAccount) -> Result<(), SecretsError>;
}

/// The platform keyring via the `keyring` crate (Linux: kernel keyutils in front of the
/// Secret Service, so entries the legacy `linux-native` build left in keyutils are still read;
/// macOS Keychain; Windows Credential Manager).
#[derive(Debug, Clone, Copy, Default)]
pub struct KeyringBackend;

impl KeyringBackend {
    fn entry(account: SecretAccount) -> Result<keyring::Entry, SecretsError> {
        keyring::Entry::new(SERVICE, account.account()).map_err(|e| keyring_error(account, &e))
    }
}

fn keyring_error(account: SecretAccount, e: &keyring::Error) -> SecretsError {
    SecretsError::Keyring {
        account: account.account(),
        message: e.to_string(),
    }
}

impl SecretBackend for KeyringBackend {
    fn get(&self, account: SecretAccount) -> Result<Option<String>, SecretsError> {
        match Self::entry(account)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(keyring_error(account, &e)),
        }
    }

    fn set(&self, account: SecretAccount, value: &str) -> Result<(), SecretsError> {
        Self::entry(account)?
            .set_password(value)
            .map_err(|e| keyring_error(account, &e))
    }

    fn delete(&self, account: SecretAccount) -> Result<(), SecretsError> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(keyring_error(account, &e)),
        }
    }
}

/// In-memory backend for tests, with switchable failure.
#[derive(Debug, Default)]
pub struct MemoryBackend {
    values: Mutex<HashMap<SecretAccount, String>>,
    failing: Mutex<bool>,
}

impl MemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-populates `account`.
    #[must_use]
    pub fn with(self, account: SecretAccount, value: impl Into<String>) -> Self {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(account, value.into());
        self
    }

    /// Makes every operation fail (simulates a missing Secret Service).
    pub fn set_failing(&self, failing: bool) {
        *self.failing.lock().unwrap_or_else(PoisonError::into_inner) = failing;
    }

    /// The stored raw value, bypassing failure simulation.
    pub fn raw(&self, account: SecretAccount) -> Option<String> {
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&account)
            .cloned()
    }

    fn check(&self, account: SecretAccount) -> Result<(), SecretsError> {
        if *self.failing.lock().unwrap_or_else(PoisonError::into_inner) {
            Err(SecretsError::Keyring {
                account: account.account(),
                message: "keyring unavailable".into(),
            })
        } else {
            Ok(())
        }
    }
}

impl SecretBackend for MemoryBackend {
    fn get(&self, account: SecretAccount) -> Result<Option<String>, SecretsError> {
        self.check(account)?;
        Ok(self.raw(account))
    }

    fn set(&self, account: SecretAccount, value: &str) -> Result<(), SecretsError> {
        self.check(account)?;
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(account, value.to_owned());
        Ok(())
    }

    fn delete(&self, account: SecretAccount) -> Result<(), SecretsError> {
        self.check(account)?;
        self.values
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&account);
        Ok(())
    }
}

/// Observable credential state.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SecretsState {
    /// `false` until the first load finished; an empty snapshot then means "unknown".
    pub loaded: bool,
    pub values: BTreeMap<SecretAccount, String>,
    /// Accounts whose last keyring access failed. Their value is unknown (absent here).
    pub errors: BTreeMap<SecretAccount, String>,
}

impl SecretsState {
    pub fn get(&self, account: SecretAccount) -> Option<&str> {
        self.values.get(&account).map(String::as_str)
    }

    pub fn has_errors(&self) -> bool {
        !self.errors.is_empty()
    }
}

/// The secrets service.
pub struct Secrets {
    backend: Arc<dyn SecretBackend>,
    state: watch::Sender<SecretsState>,
    write: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for Secrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = self.state.borrow();
        f.debug_struct("Secrets")
            .field("loaded", &s.loaded)
            .field("accounts", &s.values.keys().collect::<Vec<_>>())
            .field("errors", &s.errors)
            .finish_non_exhaustive()
    }
}

impl Secrets {
    /// A service over `backend`; nothing is read until [`Secrets::load`].
    pub fn new(backend: Arc<dyn SecretBackend>) -> Self {
        Self {
            backend,
            state: watch::Sender::new(SecretsState::default()),
            write: tokio::sync::Mutex::new(()),
        }
    }

    /// A service over the OS keyring.
    pub fn keyring() -> Self {
        Self::new(Arc::new(KeyringBackend))
    }

    /// Reads every account on a blocking thread and publishes the result (also the Retry
    /// action after a keyring failure).
    pub async fn load(&self) -> SecretsState {
        let _guard = self.write.lock().await;
        let backend = Arc::clone(&self.backend);
        let read = tokio::task::spawn_blocking(move || {
            let mut values = BTreeMap::new();
            let mut errors = BTreeMap::new();
            for account in SecretAccount::ALL {
                match backend.get(account) {
                    Ok(Some(v)) => {
                        values.insert(account, v);
                    }
                    Ok(None) => {}
                    Err(e) => {
                        tracing::warn!(account = account.account(), error = %e, "keyring read failed");
                        errors.insert(account, e.to_string());
                    }
                }
            }
            (values, errors)
        })
        .await;
        let (values, errors) = read.unwrap_or_else(|_| {
            let all = SecretAccount::ALL
                .into_iter()
                .map(|a| (a, SecretsError::TaskFailed.to_string()))
                .collect();
            (BTreeMap::new(), all)
        });
        let state = SecretsState {
            loaded: true,
            values,
            errors,
        };
        self.state.send_replace(state.clone());
        state
    }

    /// The state once the first load has finished.
    pub async fn await_loaded(&self) -> SecretsState {
        let mut rx = self.state.subscribe();
        let state = rx.wait_for(|s| s.loaded).await.map(|s| s.clone());
        state.unwrap_or_else(|_| self.snapshot())
    }

    pub fn snapshot(&self) -> SecretsState {
        self.state.borrow().clone()
    }

    pub fn get(&self, account: SecretAccount) -> Option<String> {
        self.state.borrow().values.get(&account).cloned()
    }

    pub fn subscribe(&self) -> watch::Receiver<SecretsState> {
        self.state.subscribe()
    }

    /// Writes (`Some`) or deletes (`None`) one account.
    pub async fn set(
        &self,
        account: SecretAccount,
        value: Option<String>,
    ) -> Result<(), SecretsError> {
        self.set_many(vec![(account, value)]).await
    }

    /// Writes or deletes several accounts in order on a blocking thread. The in-memory state
    /// reflects every write that succeeded, even when a later one fails.
    pub async fn set_many(
        &self,
        changes: Vec<(SecretAccount, Option<String>)>,
    ) -> Result<(), SecretsError> {
        let _guard = self.write.lock().await;
        let backend = Arc::clone(&self.backend);
        let (applied, result) = tokio::task::spawn_blocking(move || {
            let mut applied = Vec::with_capacity(changes.len());
            for (account, value) in changes {
                let r = match &value {
                    Some(v) => backend.set(account, v),
                    None => backend.delete(account),
                };
                if let Err(e) = r {
                    return (applied, Err(e));
                }
                applied.push((account, value));
            }
            (applied, Ok(()))
        })
        .await
        .map_err(|_| SecretsError::TaskFailed)?;
        self.state.send_modify(|s| {
            for (account, value) in applied {
                s.errors.remove(&account);
                match value {
                    Some(v) => s.values.insert(account, v),
                    None => s.values.remove(&account),
                };
            }
        });
        result
    }
}
