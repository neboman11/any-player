//! Grounding passages for DJ breaks (Android `PassageRepository.kt`): lookup, 30-day local
//! exclusion, token budget, and the "played" receipt queue.

use std::collections::{HashMap, HashSet};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ap_model::Track;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::account::{DjAccount, error_kind};
use crate::diagnostics::DjDiagnostics;
use crate::time::{format_instant, parse_instant};

const TAG: &str = "DjPassages";
/// Locally played passages are excluded from lookups for this long.
pub const EXCLUSION_SECONDS: i64 = 30 * 24 * 60 * 60;
/// Pending receipt events kept per account; the oldest are dropped beyond this.
pub const MAX_PENDING: usize = 512;
/// Token budget for the passages sent to break generation.
pub const DEFAULT_TOKEN_BUDGET: usize = 2500;
/// Chunk ids per receipt event.
const RECEIPT_CHUNK: usize = 16;
const LOOKUP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Passage {
    pub chunk_id: i64,
    pub text: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub source_url: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub subject: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub lang: String,
}

impl Passage {
    /// A passage with only the required fields set.
    pub fn new(chunk_id: i64, text: impl Into<String>, source: impl Into<String>) -> Self {
        Self {
            chunk_id,
            text: text.into(),
            source: source.into(),
            source_url: String::new(),
            title: String::new(),
            subject: String::new(),
            lang: String::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Passages {
    pub passages: Vec<Passage>,
}

impl Passages {
    pub fn chunk_ids(&self) -> Vec<i64> {
        self.passages.iter().map(|p| p.chunk_id).collect()
    }
}

/// Android `estimateTokens`: counts UTF-16 code units that are not Kotlin whitespace; units up
/// to U+02FF cost a third of a token (rounded up), all others one token each.
pub fn estimate_tokens(text: &str) -> usize {
    let (latin, non_latin) = text
        .encode_utf16()
        .filter(|&unit| !is_kotlin_whitespace(unit))
        .fold((0usize, 0usize), |(latin, other), unit| {
            if unit <= 0x2FF {
                (latin + 1, other)
            } else {
                (latin, other + 1)
            }
        });
    latin.div_ceil(3) + non_latin
}

/// Kotlin `Char.isWhitespace` (Java `isWhitespace || isSpaceChar`) for one UTF-16 unit.
fn is_kotlin_whitespace(unit: u16) -> bool {
    matches!(
        unit,
        0x09..=0x0D | 0x1C..=0x20 | 0xA0 | 0x1680 | 0x2000..=0x200A | 0x2028 | 0x2029 | 0x202F | 0x205F | 0x3000
    )
}

/// Android `fitToBudget`: keeps passages in rank order until the next one would exceed
/// `budget`. The first passage is always kept, even when it alone is over budget.
pub fn fit_to_budget(mut passages: Vec<Passage>, budget: usize) -> Vec<Passage> {
    let mut used = 0;
    let mut kept = 0;
    for passage in &passages {
        let cost = estimate_tokens(&passage.text);
        if kept > 0 && used + cost > budget {
            break;
        }
        kept += 1;
        used += cost;
    }
    passages.truncate(kept);
    passages
}

pub type StoreError = Box<dyn std::error::Error + Send + Sync>;
pub type StoreFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

/// Persistence for per-account passage state (`dj_passage_state` in ap-store). Values are
/// opaque JSON strings keyed by [`DjAccount::state_key`].
pub trait PassageStateStore: Send + Sync {
    fn read<'a>(&'a self, key: &'a str) -> StoreFuture<'a, Option<String>>;
    fn write<'a>(&'a self, key: &'a str, value: String) -> StoreFuture<'a, ()>;
}

/// In-memory [`PassageStateStore`] for tests and for running without persistence.
#[derive(Default)]
pub struct MemoryPassageStateStore {
    values: Mutex<HashMap<String, String>>,
}

impl MemoryPassageStateStore {
    pub fn values(&self) -> HashMap<String, String> {
        lock(&self.values).clone()
    }

    pub fn clear(&self) {
        lock(&self.values).clear();
    }
}

impl PassageStateStore for MemoryPassageStateStore {
    fn read<'a>(&'a self, key: &'a str) -> StoreFuture<'a, Option<String>> {
        let value = lock(&self.values).get(key).cloned();
        Box::pin(async move { Ok(value) })
    }

    fn write<'a>(&'a self, key: &'a str, value: String) -> StoreFuture<'a, ()> {
        lock(&self.values).insert(key.to_owned(), value);
        Box::pin(async { Ok(()) })
    }
}

/// Persisted per-account state. Field names and default omission match Android's
/// kotlinx-serialized `State`/`Played`/`Usage`.
#[derive(Debug, Default, Serialize, Deserialize)]
struct State {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    played: Vec<Played>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    usage: Vec<Usage>,
}

/// Field names are Android's persisted JSON keys.
#[allow(clippy::struct_field_names)]
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Played {
    chunk_ids: Vec<i64>,
    event_id: String,
    played_at: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Usage {
    chunk_id: i64,
    played_at: String,
}

#[derive(Deserialize)]
struct Lookup {
    status: String,
    #[serde(default)]
    passages: Vec<Passage>,
}

#[derive(Serialize)]
struct Receipt<'a> {
    event_id: &'a str,
    played_at: &'a str,
    chunk_ids: &'a [i64],
}

type Clock = Box<dyn Fn() -> DateTime<Utc> + Send + Sync>;

/// Remote passage lookup plus local exclusion and receipt queue. Cheap to clone; clones share
/// state. Background receipt flushes are aborted once the last clone is dropped (pending
/// events stay queued in the store).
#[derive(Clone)]
pub struct PassageClient {
    inner: Arc<Inner>,
    background: Arc<Mutex<tokio::task::JoinSet<()>>>,
}

struct Inner {
    http: reqwest::Client,
    store: Arc<dyn PassageStateStore>,
    diagnostics: DjDiagnostics,
    now: Clock,
    state_lock: tokio::sync::Mutex<()>,
    flush_lock: tokio::sync::Mutex<()>,
}

impl PassageClient {
    pub fn new(
        store: Arc<dyn PassageStateStore>,
        diagnostics: DjDiagnostics,
    ) -> Result<Self, reqwest::Error> {
        Self::with_clock(store, diagnostics, Utc::now)
    }

    /// Like [`PassageClient::new`] with an injected clock (for receipts and exclusion).
    pub fn with_clock(
        store: Arc<dyn PassageStateStore>,
        diagnostics: DjDiagnostics,
        now: impl Fn() -> DateTime<Utc> + Send + Sync + 'static,
    ) -> Result<Self, reqwest::Error> {
        // Retry policy belongs to this client; the 5 s timeout covers the whole call.
        let http = reqwest::Client::builder().timeout(LOOKUP_TIMEOUT).build()?;
        Ok(Self {
            inner: Arc::new(Inner {
                http,
                store,
                diagnostics,
                now: Box::new(now),
                state_lock: tokio::sync::Mutex::new(()),
                flush_lock: tokio::sync::Mutex::new(()),
            }),
            background: Arc::default(),
        })
    }

    /// `GET /v1/dj-passages`, minus passages played locally within 30 days, fitted to
    /// [`DEFAULT_TOKEN_BUDGET`]. `None` without an account, on any failure, or when nothing
    /// is left. Also starts a background flush of pending receipts (never awaited here).
    pub async fn find_passages(&self, account: &DjAccount, track: &Track) -> Option<Passages> {
        if !account.is_configured() {
            return None;
        }
        self.spawn_flush(account);
        let inner = &self.inner;
        let lookup = match inner.lookup(account, track).await {
            Ok(lookup) => lookup?,
            Err(kind) => {
                inner
                    .diagnostics
                    .log(TAG, "Passage lookup failed", Some(kind));
                return None;
            }
        };
        if lookup.status != "ok" {
            return None;
        }
        let excluded: HashSet<i64> = {
            let _guard = inner.state_lock.lock().await;
            let now = (inner.now)();
            inner
                .read(account)
                .await
                .usage
                .iter()
                .filter(|u| recent(&u.played_at, now))
                .map(|u| u.chunk_id)
                .collect()
        };
        let candidates = lookup
            .passages
            .into_iter()
            .filter(|p| !excluded.contains(&p.chunk_id))
            .collect();
        let passages = fit_to_budget(candidates, DEFAULT_TOKEN_BUDGET);
        (!passages.is_empty()).then_some(Passages { passages })
    }

    /// Queues receipts (16 chunk ids per event, newest [`MAX_PENDING`] events kept), records
    /// local usage for exclusion, and starts a background flush.
    pub async fn record_played(&self, account: &DjAccount, passages: &Passages) {
        let inner = &self.inner;
        {
            let _guard = inner.state_lock.lock().await;
            let now = (inner.now)();
            let timestamp = format_instant(now);
            let Some(state) = inner.try_read(account).await else {
                return;
            };
            let mut played = state.played;
            let chunk_ids = passages.chunk_ids();
            played.extend(chunk_ids.chunks(RECEIPT_CHUNK).map(|chunk| Played {
                chunk_ids: chunk.to_vec(),
                event_id: uuid::Uuid::new_v4().to_string(),
                played_at: timestamp.clone(),
            }));
            if played.len() > MAX_PENDING {
                inner
                    .diagnostics
                    .log(TAG, "Dropping oldest pending receipts at queue limit", None);
                played.drain(..played.len() - MAX_PENDING);
            }
            let mut usage: Vec<Usage> = state
                .usage
                .into_iter()
                .filter(|u| recent(&u.played_at, now))
                .collect();
            usage.extend(chunk_ids.iter().map(|&chunk_id| Usage {
                chunk_id,
                played_at: timestamp.clone(),
            }));
            inner.write(account, &State { played, usage }).await;
        }
        self.spawn_flush(account);
    }

    /// Sends queued receipts in order, stopping at the first transient failure. 204 and
    /// non-retryable 4xx (logged) remove an event; 408, 429, 5xx and transport errors keep it.
    pub async fn flush_pending(&self, account: &DjAccount) {
        self.inner.flush_pending(account).await;
    }

    /// Waits for every background flush started so far (e.g. before shutdown).
    pub async fn settle(&self) {
        let mut running = std::mem::take(&mut *lock(&self.background));
        while running.join_next().await.is_some() {}
    }

    fn spawn_flush(&self, account: &DjAccount) {
        let inner = Arc::clone(&self.inner);
        let account = account.clone();
        let mut background = lock(&self.background);
        while background.try_join_next().is_some() {}
        background.spawn(async move { inner.flush_pending(&account).await });
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

impl Inner {
    /// `Ok(None)`: no usable answer (non-200 after retries). `Err(kind)`: transport or decode
    /// failure, logged by the caller.
    async fn lookup(
        &self,
        account: &DjAccount,
        track: &Track,
    ) -> Result<Option<Lookup>, &'static str> {
        let mut query = vec![
            ("song", track.title.as_str()),
            ("artist", track.artist.as_str()),
        ];
        if let Some(album) = track.album.as_deref().filter(|a| !a.trim().is_empty()) {
            query.push(("album", album));
        }
        for attempt in 0..=2u32 {
            let request =
                account.authorize(self.http.get(account.url("v1/dj-passages")).query(&query));
            let response = request.send().await.map_err(|e| error_kind(&e))?;
            let status = response.status().as_u16();
            if status == 200 {
                let body = response.bytes().await.map_err(|e| error_kind(&e))?;
                return serde_json::from_slice(&body)
                    .map(Some)
                    .map_err(|_| "Decode");
            }
            if status != 503 || attempt == 2 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250 << attempt)).await;
        }
        Ok(None)
    }

    async fn flush_pending(&self, account: &DjAccount) {
        let _flush = self.flush_lock.lock().await;
        if !account.is_configured() {
            return;
        }
        let pending = {
            let _guard = self.state_lock.lock().await;
            self.read(account).await.played
        };
        for event in pending {
            let receipt = Receipt {
                event_id: &event.event_id,
                played_at: &event.played_at,
                chunk_ids: &event.chunk_ids,
            };
            let Ok(body) = serde_json::to_string(&receipt) else {
                return;
            };
            let request = account
                .authorize(self.http.post(account.url("v1/dj-passages/played")))
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
            let Ok(response) = request.send().await else {
                return;
            };
            let code = response.status().as_u16();
            drop(response);
            match code {
                204 => {}
                400..=499 if code != 408 && code != 429 => {
                    self.diagnostics.log(
                        TAG,
                        &format!("Played receipt rejected (HTTP {code}); dropping"),
                        None,
                    );
                }
                _ => return,
            }
            let _guard = self.state_lock.lock().await;
            let Some(mut state) = self.try_read(account).await else {
                return;
            };
            state.played.retain(|p| p.event_id != event.event_id);
            self.write(account, &state).await;
        }
    }

    /// Stored state; unreadable or undecodable state reads as empty (Android parity).
    async fn read(&self, account: &DjAccount) -> State {
        self.try_read(account).await.unwrap_or_default()
    }

    /// `None` only when the store itself fails, so callers never overwrite state they could
    /// not read. Undecodable JSON reads as empty state, like Android.
    async fn try_read(&self, account: &DjAccount) -> Option<State> {
        match self.store.read(&account.state_key()).await {
            Ok(Some(json)) => Some(serde_json::from_str(&json).unwrap_or_default()),
            Ok(None) => Some(State::default()),
            Err(_) => {
                self.diagnostics
                    .log(TAG, "Passage state could not be read", Some("Store"));
                None
            }
        }
    }

    async fn write(&self, account: &DjAccount, state: &State) {
        let Ok(json) = serde_json::to_string(state) else {
            return;
        };
        if self.store.write(&account.state_key(), json).await.is_err() {
            self.diagnostics
                .log(TAG, "Passage state could not be saved", Some("Store"));
        }
    }
}

/// True when `timestamp` is strictly after `now - 30 days`; unparseable means not recent.
fn recent(timestamp: &str, now: DateTime<Utc>) -> bool {
    parse_instant(timestamp).is_some_and(|t| t > now - chrono::Duration::seconds(EXCLUSION_SECONDS))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_estimate_matches_android() {
        assert_eq!(estimate_tokens("abc 漢字字"), 4);
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("a"), 1);
        assert_eq!(estimate_tokens("\u{a0}\u{3000} \t"), 0);
        // Supplementary characters count per UTF-16 unit, like Kotlin.
        assert_eq!(estimate_tokens("😀"), 2);
    }

    #[test]
    fn budget_keeps_first_and_stops_at_overflow() {
        let passages = vec![
            Passage::new(1, "y".repeat(6_000), "x"),
            Passage::new(2, "z".repeat(6_000), "x"),
            Passage::new(3, "short", "x"),
        ];
        let kept: Vec<i64> = fit_to_budget(passages, DEFAULT_TOKEN_BUDGET)
            .iter()
            .map(|p| p.chunk_id)
            .collect();
        assert_eq!(kept, [1]);
        let huge = vec![
            Passage::new(1, "漢".repeat(3_000), "x"),
            Passage::new(2, "short", "x"),
        ];
        assert_eq!(fit_to_budget(huge, DEFAULT_TOKEN_BUDGET).len(), 1);
    }

    #[test]
    fn passage_serde_omits_defaults_and_ignores_unknown() {
        let passage: Passage =
            serde_json::from_str(r#"{"chunk_id":5,"text":"t","source":"s","title":"T","extra":1}"#)
                .unwrap();
        assert_eq!(passage.title, "T");
        assert_eq!(
            serde_json::to_string(&passage).unwrap(),
            r#"{"chunk_id":5,"text":"t","source":"s","title":"T"}"#
        );
    }

    #[test]
    fn state_shape_matches_kotlinx() {
        assert_eq!(serde_json::to_string(&State::default()).unwrap(), "{}");
        let state = State {
            played: vec![Played {
                chunk_ids: vec![1],
                event_id: "e".into(),
                played_at: "p".into(),
            }],
            usage: vec![Usage {
                chunk_id: 1,
                played_at: "p".into(),
            }],
        };
        assert_eq!(
            serde_json::to_string(&state).unwrap(),
            r#"{"played":[{"chunkIds":[1],"eventId":"e","playedAt":"p"}],"usage":[{"chunkId":1,"playedAt":"p"}]}"#
        );
    }

    #[test]
    fn recent_is_strict_and_rejects_garbage() {
        let now = parse_instant("2026-10-31T00:00:00Z").unwrap();
        assert!(recent("2026-10-01T00:00:01Z", now));
        assert!(!recent("2026-10-01T00:00:00Z", now));
        assert!(!recent("yesterday", now));
    }
}
