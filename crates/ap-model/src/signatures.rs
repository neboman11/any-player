//! Sync merge-base signatures (Android `SyncBase.kt`, design §9.4).

use std::cmp::Ordering;

use indexmap::IndexMap;

use crate::json::JsonElement;
use crate::models::Track;
use crate::text::{cmp_utf16, kotlin_is_blank, kotlin_trim, sha256_hex};

/// Queue content only (current track, queue, shuffle, repeat), normalized the way remote state
/// is applied: a current track missing from the queue is prepended. `shuffle` prints as Kotlin
/// `Boolean?.toString()`; sources print as the uppercase enum name.
#[must_use]
pub fn app_state(current: Option<&Track>, queue: &[Track], shuffle: Option<bool>, repeat: &str) -> String {
    let shuffle = match shuffle {
        Some(true) => "true",
        Some(false) => "false",
        None => "null",
    };
    let key = |t: &Track| format!("{}:{}", t.source, t.id);
    let mut parts = vec![
        shuffle.to_owned(),
        repeat.to_owned(),
        current.map(key).unwrap_or_default(),
    ];
    if let Some(c) = current.filter(|c| !queue.iter().any(|t| t.same_track(c))) {
        parts.push(key(c));
    }
    parts.extend(queue.iter().map(key));
    sha256_hex(&parts.join("\0"))
}

/// Null object members dropped (recursively; arrays keep nulls), keys sorted by Kotlin string
/// order.
#[must_use]
pub fn canonical(element: &JsonElement) -> JsonElement {
    match element {
        JsonElement::Object(map) => {
            let mut entries: Vec<(&String, &JsonElement)> =
                map.iter().filter(|(_, v)| !matches!(v, JsonElement::Null)).collect();
            entries.sort_by(|a, b| cmp_utf16(a.0, b.0));
            JsonElement::Object(entries.into_iter().map(|(k, v)| (k.clone(), canonical(v))).collect())
        }
        JsonElement::Array(items) => JsonElement::Array(items.iter().map(canonical).collect()),
        other => other.clone(),
    }
}

fn rows(element: Option<&JsonElement>) -> JsonElement {
    let mut objects: Vec<&IndexMap<String, JsonElement>> = element
        .and_then(JsonElement::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(JsonElement::as_object)
        .collect();
    objects.sort_by_key(|o| o.get("position").and_then(JsonElement::as_long_lenient));
    JsonElement::Array(
        objects
            .into_iter()
            .map(|o| {
                let mut row = o.clone();
                row.shift_remove("id");
                JsonElement::Object(row)
            })
            .collect(),
    )
}

fn playlist_id(entry: &IndexMap<String, JsonElement>) -> Option<std::borrow::Cow<'_, str>> {
    entry.get("playlist").and_then(JsonElement::as_object)?.get("id")?.content_or_null()
}

/// Playlists namespace content (config-file `custom_playlists` array), independent of the
/// exporting client's formatting: playlists ordered by id, tracks and union sources by position,
/// file-local row `id`s and null fields dropped, keys sorted. A missing or non-array namespace
/// is the empty list.
#[must_use]
pub fn playlists(playlists: Option<&JsonElement>) -> String {
    let mut entries: Vec<&IndexMap<String, JsonElement>> = playlists
        .and_then(JsonElement::as_array)
        .unwrap_or_default()
        .iter()
        .filter_map(JsonElement::as_object)
        .collect();
    entries.sort_by(|a, b| match (playlist_id(a), playlist_id(b)) {
        (None, None) => Ordering::Equal,
        (None, Some(_)) => Ordering::Less,
        (Some(_), None) => Ordering::Greater,
        (Some(x), Some(y)) => cmp_utf16(&x, &y),
    });
    let normalized = entries
        .into_iter()
        .map(|entry| {
            let mut e = entry.clone();
            let tracks = rows(entry.get("tracks"));
            let sources = rows(entry.get("union_sources"));
            e.insert("tracks".to_owned(), tracks);
            e.insert("union_sources".to_owned(), sources);
            JsonElement::Object(e)
        })
        .collect();
    sha256_hex(&canonical(&JsonElement::Array(normalized)).to_string())
}

/// The settings namespace as this device pushes it.
#[must_use]
pub fn settings_json(normalization_enabled: bool, normalization_strict: bool) -> JsonElement {
    JsonElement::Object(IndexMap::from([
        ("audio_normalization_enabled".to_owned(), JsonElement::Bool(normalization_enabled)),
        ("audio_normalization_strict_mode".to_owned(), JsonElement::Bool(normalization_strict)),
    ]))
}

/// Signature of the settings namespace.
#[must_use]
pub fn settings(settings: &JsonElement) -> String {
    sha256_hex(&canonical(settings).to_string())
}

/// Android `normalizeSyncServerBaseUrl`: trimmed, trailing slashes removed, `https://` added
/// unless it already starts (case-sensitively) with `https://` or `http://`.
#[must_use]
pub fn normalize_server_base_url(value: &str) -> String {
    let trimmed = kotlin_trim(value).trim_end_matches('/');
    if kotlin_is_blank(trimmed) {
        return String::new();
    }
    if trimmed.starts_with("https://") || trimmed.starts_with("http://") {
        trimmed.to_owned()
    } else {
        format!("https://{trimmed}")
    }
}

/// Java regex `\s`.
fn is_java_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\u{0B}' | '\u{0C}' | '\r')
}

/// Android `normalizeSyncServerAuthToken`: trimmed, a leading `Bearer\s+` (any case) removed.
#[must_use]
pub fn normalize_auth_token(value: &str) -> String {
    let trimmed = kotlin_trim(value);
    if let Some(prefix) = trimmed.get(..6).filter(|p| p.eq_ignore_ascii_case("bearer")) {
        let rest = &trimmed[prefix.len()..];
        let after = rest.trim_start_matches(is_java_space);
        if after.len() < rest.len() {
            return after.to_owned();
        }
    }
    trimmed.to_owned()
}

/// Sync base target: normalized URL + `#` + sha256 of the normalized credential.
#[must_use]
pub fn base_target(server_target: &str, auth_token: &str) -> String {
    format!(
        "{}#{}",
        normalize_server_base_url(server_target),
        sha256_hex(&normalize_auth_token(auth_token))
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_token_normalization() {
        assert_eq!(normalize_auth_token("  bearer   tok en  "), "tok en");
        assert_eq!(normalize_auth_token("Bearer"), "Bearer");
        assert_eq!(normalize_auth_token("BEARER\ttoken"), "token");
        assert_eq!(normalize_auth_token("Bearerx"), "Bearerx");
        assert_eq!(normalize_auth_token("tök€n"), "tök€n");
    }

    #[test]
    fn base_url_normalization() {
        assert_eq!(normalize_server_base_url("  http://host:8080///  "), "http://host:8080");
        assert_eq!(normalize_server_base_url("HTTPS://x"), "https://HTTPS://x");
        assert_eq!(normalize_server_base_url(" / "), "");
    }
}
