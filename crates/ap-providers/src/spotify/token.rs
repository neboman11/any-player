//! [`SpotifyTokenProvider`]: the in-memory Spotify token with single-flight refresh.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::TimeDelta;
use tokio::sync::Mutex;

use super::auth::{SpotifyAccounts, SpotifyToken};
use crate::error::{ProviderError, Result};

/// Refresh when the token expires within this margin.
pub const REFRESH_MARGIN: TimeDelta = TimeDelta::minutes(10);

/// Receives token changes so the app can persist them (keyring `spotify-token`). Called while the
/// provider holds its refresh lock, so implementations should be quick.
#[async_trait]
pub trait TokenSink: Send + Sync {
    /// A new token was installed or obtained by a refresh. Persist it.
    async fn token_updated(&self, token: &SpotifyToken);
    /// Spotify rejected the refresh token (HTTP 400/401): the account must be linked again.
    async fn refresh_rejected(&self) {}
}

/// Holds the current token. Refreshes are single-flight: concurrent callers wait on one lock and
/// reuse the refreshed token. [`Self::access_token`] refreshes when the token expires within
/// [`REFRESH_MARGIN`]; [`Self::refresh_after_unauthorized`] forces one refresh after a 401.
pub struct SpotifyTokenProvider {
    accounts: SpotifyAccounts,
    sink: Arc<dyn TokenSink>,
    token: Mutex<Option<SpotifyToken>>,
}

impl std::fmt::Debug for SpotifyTokenProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpotifyTokenProvider")
            .field("accounts", &self.accounts)
            .finish_non_exhaustive()
    }
}

impl SpotifyTokenProvider {
    /// `token` is the stored token (startup restore) or `None` when no account is linked.
    pub fn new(
        accounts: SpotifyAccounts,
        token: Option<SpotifyToken>,
        sink: Arc<dyn TokenSink>,
    ) -> Self {
        Self {
            accounts,
            sink,
            token: Mutex::new(token),
        }
    }

    pub fn accounts(&self) -> &SpotifyAccounts {
        &self.accounts
    }

    /// Installs a token from a completed sign-in and persists it through the sink.
    pub async fn install(&self, token: SpotifyToken) {
        let mut guard = self.token.lock().await;
        self.sink.token_updated(&token).await;
        *guard = Some(token);
    }

    /// Forgets the token (disconnect). Deleting the stored copy is the caller's job.
    pub async fn clear(&self) {
        *self.token.lock().await = None;
    }

    /// A copy of the current token.
    pub async fn current(&self) -> Option<SpotifyToken> {
        self.token.lock().await.clone()
    }

    /// A usable access token, refreshing first when it expires within [`REFRESH_MARGIN`]. A failed
    /// refresh still returns the old token while it has not expired.
    pub async fn access_token(&self) -> Result<String> {
        let mut guard = self.token.lock().await;
        let token = guard.as_ref().ok_or(ProviderError::NotLinked)?;
        let now = self.accounts.now();
        if !token.expires_within(now, REFRESH_MARGIN) {
            return Ok(token.access_token.clone());
        }
        let fallback = (!token.is_expired(now)).then(|| token.access_token.clone());
        match self.refresh_locked(&mut guard).await {
            Ok(access) => Ok(access),
            Err(error) => fallback.ok_or(error),
        }
    }

    /// After a 401 for `rejected`: refreshes once. When another caller already replaced
    /// `rejected`, the newer token is returned without another refresh.
    pub async fn refresh_after_unauthorized(&self, rejected: &str) -> Result<String> {
        let mut guard = self.token.lock().await;
        let token = guard.as_ref().ok_or(ProviderError::NotLinked)?;
        if token.access_token != rejected {
            return Ok(token.access_token.clone());
        }
        self.refresh_locked(&mut guard).await
    }

    async fn refresh_locked(&self, slot: &mut Option<SpotifyToken>) -> Result<String> {
        let Some(current) = slot.as_ref() else {
            return Err(ProviderError::NotLinked);
        };
        let Some(refresh_token) = current
            .refresh_token
            .clone()
            .filter(|t| !t.trim().is_empty())
        else {
            return Err(ProviderError::TokenRefreshFailed);
        };
        let mut fresh = match self.accounts.refresh(&refresh_token).await {
            Ok(fresh) => fresh,
            Err(ProviderError::Http {
                status: 400 | 401, ..
            }) => {
                self.sink.refresh_rejected().await;
                return Err(ProviderError::TokenRefreshFailed);
            }
            Err(_) => return Err(ProviderError::TokenRefreshFailed),
        };
        if fresh.refresh_token.is_none() {
            fresh.refresh_token = Some(refresh_token);
        }
        if fresh.scopes.is_empty() {
            fresh.scopes.clone_from(&current.scopes);
        }
        self.sink.token_updated(&fresh).await;
        let access = fresh.access_token.clone();
        *slot = Some(fresh);
        Ok(access)
    }
}
