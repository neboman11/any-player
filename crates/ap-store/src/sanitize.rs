//! Credential stripping before anything is persisted (design §6.2: `track_json` never stores
//! auth headers or tokens).

use std::borrow::Cow;

use ap_model::Track;

const PLEX_TOKEN_PARAM: &str = "X-Plex-Token";

/// Removes every `X-Plex-Token` query parameter (name compared case-insensitively) from `url`.
/// Other parameters, their order and any `#fragment` are kept. Borrowed when nothing changes.
pub fn strip_plex_token(url: &str) -> Cow<'_, str> {
    let Some(q) = url.find('?') else {
        return Cow::Borrowed(url);
    };
    let (base, rest) = url.split_at(q);
    let rest = &rest[1..];
    let (query, fragment) = match rest.find('#') {
        Some(h) => (&rest[..h], Some(&rest[h..])),
        None => (rest, None),
    };
    let is_token = |param: &str| {
        let name = param.split_once('=').map_or(param, |(n, _)| n);
        name.eq_ignore_ascii_case(PLEX_TOKEN_PARAM)
    };
    if !query.split('&').any(is_token) {
        return Cow::Borrowed(url);
    }
    let kept: Vec<&str> = query
        .split('&')
        .filter(|p| !p.is_empty() && !is_token(p))
        .collect();
    let mut out = String::with_capacity(url.len());
    out.push_str(base);
    if !kept.is_empty() {
        out.push('?');
        out.push_str(&kept.join("&"));
    }
    if let Some(fragment) = fragment {
        out.push_str(fragment);
    }
    Cow::Owned(out)
}

pub(crate) fn strip_opt(url: Option<&str>) -> Option<String> {
    url.map(|u| strip_plex_token(u).into_owned())
}

/// The JSON stored in `track_json` columns: the kotlinx-compatible [`Track`] shape with Plex
/// tokens removed from `url` and `image_url`.
pub fn persisted_track_json(track: &Track) -> serde_json::Result<String> {
    let dirty = |u: &Option<String>| {
        u.as_deref()
            .is_some_and(|u| matches!(strip_plex_token(u), Cow::Owned(_)))
    };
    if dirty(&track.url) || dirty(&track.image_url) {
        let clean = Track {
            url: strip_opt(track.url.as_deref()),
            image_url: strip_opt(track.image_url.as_deref()),
            ..track.clone()
        };
        serde_json::to_string(&clean)
    } else {
        serde_json::to_string(track)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ap_model::SourceType;

    #[test]
    fn strips_only_the_token_parameter() {
        assert_eq!(strip_plex_token("http://h/a?X-Plex-Token=t"), "http://h/a");
        assert_eq!(
            strip_plex_token("http://h/a?x=1&x-plex-token=t&y=2"),
            "http://h/a?x=1&y=2"
        );
        assert_eq!(
            strip_plex_token("http://h/a?X-Plex-Token=t#f"),
            "http://h/a#f"
        );
        assert!(matches!(
            strip_plex_token("http://h/a?x=1"),
            Cow::Borrowed(_)
        ));
        assert!(matches!(strip_plex_token("http://h/a"), Cow::Borrowed(_)));
    }

    #[test]
    fn persisted_json_has_no_token() {
        let t = Track {
            url: Some("http://plex/l/1?X-Plex-Token=secret".into()),
            image_url: Some("http://plex/t?X-Plex-Token=secret&w=1".into()),
            ..Track::new("1", "T", "A", SourceType::Plex)
        };
        let json = persisted_track_json(&t).unwrap();
        assert!(!json.contains("secret"), "{json}");
        let back: Track = serde_json::from_str(&json).unwrap();
        assert_eq!(back.url.as_deref(), Some("http://plex/l/1"));
        assert_eq!(back.image_url.as_deref(), Some("http://plex/t?w=1"));
    }
}
