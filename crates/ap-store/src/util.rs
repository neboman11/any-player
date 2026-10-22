use std::time::{SystemTime, UNIX_EPOCH};

fn since_epoch() -> std::time::Duration {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
}

/// Wall clock in epoch seconds (custom playlist timestamps; legacy and wire format).
pub fn now_secs() -> i64 {
    i64::try_from(since_epoch().as_secs()).unwrap_or(i64::MAX)
}

/// Wall clock in epoch milliseconds (catalog cache and gain timestamps).
pub fn now_ms() -> i64 {
    i64::try_from(since_epoch().as_millis()).unwrap_or(i64::MAX)
}

/// Converts a length or index to an SQLite integer.
pub(crate) fn to_i64(v: usize) -> i64 {
    i64::try_from(v).expect("length fits i64")
}
