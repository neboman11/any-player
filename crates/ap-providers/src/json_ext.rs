//! Lenient JSON accessors mirroring Android's `text`/`number`/`obj`/`array` helpers.

use serde_json::Value;

/// A primitive's content: strings as-is, numbers and booleans as their literal text.
pub(crate) fn text(value: &Value, key: &str) -> Option<String> {
    match value.get(key)? {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// An integer, also from a numeric string (kotlinx `longOrNull`).
pub(crate) fn number(value: &Value, key: &str) -> Option<i64> {
    match value.get(key)? {
        Value::Number(n) => n.as_i64(),
        Value::String(s) => s.parse().ok(),
        _ => None,
    }
}

pub(crate) fn boolean(value: &Value, key: &str) -> Option<bool> {
    value.get(key)?.as_bool()
}

/// The object at `key`, or `Null` (whose accessors all return `None`).
pub(crate) fn obj<'a>(value: &'a Value, key: &str) -> &'a Value {
    value
        .get(key)
        .filter(|v| v.is_object())
        .unwrap_or(&Value::Null)
}

/// The objects of the array at `key`; non-object entries (e.g. Spotify's `null`s) are skipped.
pub(crate) fn array<'a>(value: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|v| v.is_object())
}

/// Raw length of the array at `key` (including non-object entries).
pub(crate) fn raw_len(value: &Value, key: &str) -> usize {
    value.get(key).and_then(Value::as_array).map_or(0, Vec::len)
}

pub(crate) fn to_i32(value: i64) -> Option<i32> {
    i32::try_from(value).ok()
}
