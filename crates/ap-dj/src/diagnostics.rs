//! In-memory AI DJ log (Android `DjDiagnostics`) backing the Settings → AI DJ Logs panel.
//! No secrets, passage text or HTTP bodies are ever logged here.

use std::collections::VecDeque;
use std::sync::Arc;

use tokio::sync::watch;

/// Maximum number of retained entries; older entries are dropped first.
pub const MAX_ENTRIES: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DjLogEntry {
    /// ISO-8601 instant, formatted like Java `Instant.toString()`.
    pub timestamp: String,
    pub tag: String,
    pub message: String,
}

/// Cheap to clone; all clones share one ring.
#[derive(Clone)]
pub struct DjDiagnostics {
    entries: Arc<watch::Sender<VecDeque<DjLogEntry>>>,
    debug: Arc<watch::Sender<bool>>,
}

impl Default for DjDiagnostics {
    fn default() -> Self {
        Self::new()
    }
}

impl DjDiagnostics {
    pub fn new() -> Self {
        Self {
            entries: Arc::new(watch::Sender::new(VecDeque::with_capacity(MAX_ENTRIES))),
            debug: Arc::new(watch::Sender::new(false)),
        }
    }

    /// Observes the ring (oldest first).
    pub fn subscribe(&self) -> watch::Receiver<VecDeque<DjLogEntry>> {
        self.entries.subscribe()
    }

    pub fn entries(&self) -> Vec<DjLogEntry> {
        self.entries.borrow().iter().cloned().collect()
    }

    pub fn debug_logging(&self) -> bool {
        *self.debug.borrow()
    }

    pub fn subscribe_debug_logging(&self) -> watch::Receiver<bool> {
        self.debug.subscribe()
    }

    pub fn set_debug_logging(&self, enabled: bool) {
        self.debug.send_replace(enabled);
    }

    pub fn clear(&self) {
        self.entries.send_modify(VecDeque::clear);
    }

    /// Records an entry. `error_kind` is a short, secret-free classification (Android appends
    /// the exception's simple class name) and is appended as `": <kind>"`.
    pub fn log(&self, tag: &str, message: &str, error_kind: Option<&str>) {
        self.append(tag, message, error_kind);
    }

    /// Records an entry only while debug logging is enabled.
    pub fn debug(&self, tag: &str, message: &str) {
        if self.debug_logging() {
            self.append(tag, message, None);
        }
    }

    fn append(&self, tag: &str, message: &str, error_kind: Option<&str>) {
        let entry = DjLogEntry {
            timestamp: crate::time::format_instant(chrono::Utc::now()),
            tag: tag.to_owned(),
            message: match error_kind {
                Some(kind) => format!("{message}: {kind}"),
                None => message.to_owned(),
            },
        };
        self.entries.send_modify(|entries| {
            if entries.len() == MAX_ENTRIES {
                entries.pop_front();
            }
            entries.push_back(entry);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_keeps_last_two_hundred() {
        let diagnostics = DjDiagnostics::new();
        for i in 0..250 {
            diagnostics.log("T", &i.to_string(), None);
        }
        let entries = diagnostics.entries();
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries[0].message, "50");
        assert_eq!(entries[199].message, "249");
        diagnostics.clear();
        assert!(diagnostics.entries().is_empty());
    }

    #[test]
    fn debug_entries_respect_toggle_and_errors_get_kind() {
        let diagnostics = DjDiagnostics::new();
        diagnostics.debug("T", "hidden");
        diagnostics.set_debug_logging(true);
        diagnostics.debug("T", "shown");
        diagnostics.log("T", "failed", Some("Timeout"));
        let messages: Vec<_> = diagnostics
            .entries()
            .into_iter()
            .map(|e| e.message)
            .collect();
        assert_eq!(messages, ["shown", "failed: Timeout"]);
    }
}
