//! Java `Instant` text compatibility for persisted DJ state and receipts.

use chrono::{DateTime, SecondsFormat, Utc};

/// Java `Instant.toString()`: `2026-10-01T00:00:00Z`, with 3/6/9 fraction digits only when nonzero.
pub(crate) fn format_instant(instant: DateTime<Utc>) -> String {
    instant.to_rfc3339_opts(SecondsFormat::AutoSi, true)
}

/// Java `Instant.parse` for the ISO-8601 instants this crate (or Android) writes.
pub(crate) fn parse_instant(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text)
        .ok()
        .map(|t| t.with_timezone(&Utc))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_java_instant() {
        let whole = parse_instant("2026-10-01T00:00:00Z").unwrap();
        assert_eq!(format_instant(whole), "2026-10-01T00:00:00Z");
        let millis = whole + chrono::Duration::milliseconds(120);
        assert_eq!(format_instant(millis), "2026-10-01T00:00:00.120Z");
        let micros = whole + chrono::Duration::microseconds(1);
        assert_eq!(format_instant(micros), "2026-10-01T00:00:00.000001Z");
    }
}
