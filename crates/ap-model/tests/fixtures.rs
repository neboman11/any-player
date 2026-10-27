//! Golden vectors generated from the Android implementation (`fixtures/`).

mod common;

use ap_model::json::JsonElement;
use ap_model::models::PlaylistTrack;
use ap_model::text::sha256_hex;
use ap_model::transfer_crypto::{self, NONCE_LEN};
use ap_model::wire::{
    self, AnyPlayerStateEnvelope, ConfigCustomPlaylist, EncryptedStateFile, StateFileError,
    decode_and_verify,
};
use ap_model::{SourceType, Track, dedupe, name_uuid_from_bytes, signatures, volume_normalization};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use common::{fixture, fixture_json};

/// Parses a fixture with literal-preserving numbers.
fn fixture_tree(name: &str) -> JsonElement {
    JsonElement::parse(&fixture(name)).expect("fixture is JSON")
}

fn field<'a>(v: &'a JsonElement, key: &str) -> &'a JsonElement {
    v.as_object()
        .and_then(|o| o.get(key))
        .unwrap_or_else(|| panic!("missing {key}"))
}

fn text<'a>(v: &'a JsonElement, key: &str) -> &'a str {
    match field(v, key) {
        JsonElement::String(s) => s,
        other => panic!("{key} is not a string: {other}"),
    }
}

fn opt_text<'a>(v: &'a JsonElement, key: &str) -> Option<&'a str> {
    match field(v, key) {
        JsonElement::Null => None,
        JsonElement::String(s) => Some(s),
        other => panic!("{key} is not a string or null: {other}"),
    }
}

fn track(v: &JsonElement) -> Track {
    serde_json::from_str(&v.to_string()).expect("track")
}

#[test]
fn sync_signatures_app_state() {
    let root = fixture_tree("sync_signatures.json");
    let cases = field(&root, "app_state").as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let name = text(case, "name");
        let current = match field(case, "current") {
            JsonElement::Null => None,
            v => Some(track(v)),
        };
        let queue: Vec<Track> = field(case, "queue")
            .as_array()
            .unwrap()
            .iter()
            .map(track)
            .collect();
        let shuffle = match field(case, "shuffle") {
            JsonElement::Null => None,
            JsonElement::Bool(b) => Some(*b),
            other => panic!("{name}: shuffle {other}"),
        };
        let actual = signatures::app_state(current.as_ref(), &queue, shuffle, text(case, "repeat"));
        assert_eq!(actual, text(case, "expected"), "app_state {name}");
    }
}

/// The exact signature input: `input_text` (original literal text) when present, else `input`.
fn signature_input(case: &JsonElement) -> JsonElement {
    match case.as_object().and_then(|o| o.get("input_text")) {
        Some(JsonElement::String(raw)) => JsonElement::parse(raw).expect("input_text is JSON"),
        _ => field(case, "input").clone(),
    }
}

#[test]
fn sync_signatures_playlists() {
    let root = fixture_tree("sync_signatures.json");
    let cases = field(&root, "playlists").as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let input = signature_input(case);
        let input = if input == JsonElement::Null {
            None
        } else {
            Some(&input)
        };
        assert_eq!(
            signatures::playlists(input),
            text(case, "expected"),
            "playlists {}",
            text(case, "name")
        );
    }
}

#[test]
fn sync_signatures_settings() {
    let root = fixture_tree("sync_signatures.json");
    let cases = field(&root, "settings").as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        assert_eq!(
            signatures::settings(&signature_input(case)),
            text(case, "expected"),
            "settings {}",
            text(case, "name")
        );
    }
    // The pushed settings object hashes like its fixture twin.
    let both_false = cases
        .iter()
        .find(|c| text(c, "name") == "both_false")
        .unwrap();
    assert_eq!(
        signatures::settings(&signatures::settings_json(false, false)),
        text(both_false, "expected")
    );
}

#[test]
fn sync_signatures_base_target() {
    let root = fixture_tree("sync_signatures.json");
    let cases = field(&root, "base_target").as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let (target, token) = (text(case, "server_target"), text(case, "auth_token"));
        assert_eq!(
            signatures::base_target(target, token),
            text(case, "expected"),
            "{target:?} / {token:?}"
        );
    }
}

#[test]
fn state_transfer_round_trips_byte_exact() {
    let root = fixture_tree("state_transfer.json");
    let cases = field(&root, "cases").as_array().unwrap();
    assert!(
        cases
            .iter()
            .any(|c| opt_text(c, "encrypted_file").is_some()),
        "a PRIVATE case exists"
    );
    for case in cases {
        let name = text(case, "name");
        let envelope_json = text(case, "envelope_json");
        let envelope: AnyPlayerStateEnvelope = serde_json::from_str(envelope_json).unwrap();
        assert_eq!(envelope.to_json(), envelope_json, "{name}: re-encode");
        let seed = AnyPlayerStateEnvelope {
            integrity: wire::Integrity {
                sha256: String::new(),
            },
            ..envelope.clone()
        };
        assert_eq!(
            seed.to_json(),
            text(case, "integrity_input"),
            "{name}: integrity input"
        );
        assert_eq!(
            sha256_hex(text(case, "integrity_input")),
            text(case, "sha256"),
            "{name}: sha"
        );
        assert_eq!(
            envelope.compute_integrity(),
            text(case, "sha256"),
            "{name}: compute"
        );
        assert_eq!(
            envelope.integrity.sha256,
            text(case, "sha256"),
            "{name}: stored"
        );
        let rebuilt = AnyPlayerStateEnvelope::new(
            envelope.created_at.clone(),
            envelope.source_app.clone(),
            envelope.data.clone(),
        );
        assert_eq!(
            rebuilt.to_json(),
            envelope_json,
            "{name}: rebuilt from parts"
        );
        assert_eq!(
            decode_and_verify(envelope_json, None).unwrap(),
            envelope,
            "{name}: verify"
        );

        let Some(encrypted) = opt_text(case, "encrypted_file") else {
            continue;
        };
        let passphrase = opt_text(case, "passphrase").expect("PRIVATE case has a passphrase");
        let file: EncryptedStateFile = serde_json::from_str(encrypted).unwrap();
        assert_eq!(
            serde_json::to_string(&file).unwrap(),
            encrypted,
            "{name}: encrypted re-encode"
        );
        assert_eq!(
            transfer_crypto::decrypt(&file, passphrase).unwrap(),
            envelope_json,
            "{name}: decrypt"
        );
        let salt = STANDARD.decode(&file.salt_base64).unwrap();
        let nonce: [u8; NONCE_LEN] = STANDARD
            .decode(&file.nonce_base64)
            .unwrap()
            .try_into()
            .unwrap();
        let again = transfer_crypto::encrypt_with(
            envelope_json,
            passphrase,
            u32::try_from(file.kdf_iterations).unwrap(),
            &salt,
            &nonce,
        )
        .unwrap();
        assert_eq!(
            serde_json::to_string(&again).unwrap(),
            encrypted,
            "{name}: deterministic encrypt"
        );
        assert_eq!(
            decode_and_verify(encrypted, Some(passphrase)).unwrap(),
            envelope,
            "{name}: verify encrypted"
        );
        assert!(matches!(
            decode_and_verify(encrypted, None),
            Err(StateFileError::PassphraseRequired)
        ));
        assert!(matches!(
            decode_and_verify(encrypted, Some("wrong")),
            Err(StateFileError::Crypto(_))
        ));
    }
}

#[test]
fn legacy_state_file_verifies() {
    let raw = fixture("legacy-state.json");
    let envelope = decode_and_verify(&raw, None).unwrap();
    assert_eq!(envelope.source_app.platform, "android");
    assert_eq!(envelope.data, wire::AnyPlayerStateData::default());
    assert_eq!(envelope.to_json(), raw.trim_end());
}

#[test]
fn config_ids_name_uuid() {
    let root = fixture_json("config_ids.json");
    let cases = root["name_uuid"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let key = case["key"].as_str().unwrap();
        assert_eq!(
            name_uuid_from_bytes(key.as_bytes()),
            case["uuid"].as_str().unwrap(),
            "{key}"
        );
    }
    let by_key = |k: &str| {
        cases.iter().find(|c| c["key"] == k).unwrap()["uuid"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    assert_eq!(
        wire::config_track_row_id("pl1", SourceType::Spotify, "abc", 0),
        by_key("track:pl1:SPOTIFY:abc")
    );
    assert_eq!(
        wire::config_track_row_id("pl1", SourceType::Spotify, "abc", 2),
        by_key("track:pl1:SPOTIFY:abc:occurrence:2")
    );
    assert_eq!(
        wire::config_union_source_row_id("u1", SourceType::Custom, "p2", 1),
        by_key("union:u1:CUSTOM:p2:occurrence:1")
    );
}

#[test]
fn config_export_json_round_trips_byte_exact() {
    let root = fixture_json("config_ids.json");
    let cases = root["export_json"].as_array().unwrap();
    assert!(!cases.is_empty());
    for case in cases {
        let json = case["json"].as_str().unwrap();
        let playlists: Vec<ConfigCustomPlaylist> = serde_json::from_str(json).unwrap();
        assert_eq!(
            serde_json::to_string(&playlists).unwrap(),
            json,
            "{}",
            case["name"]
        );
    }
}

#[test]
fn volume_normalization_parity() {
    let root = fixture_json("volume_normalization.json");
    let column = |key: &str| -> Vec<i64> {
        let values: Vec<i64> = root[key]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_i64().unwrap())
            .collect();
        assert_eq!(values.len(), 101, "{key}");
        values
    };
    let sources = [
        ("spotify", SourceType::Spotify),
        ("jellyfin", SourceType::Jellyfin),
        ("plex", SourceType::Plex),
        ("custom", SourceType::Custom),
    ];
    for (key, source) in sources {
        for (base, want) in (0..=100).zip(column(key)) {
            assert_eq!(
                i64::from(volume_normalization::effective(base, source, true)),
                want,
                "{key} {base}"
            );
        }
    }
    for (base, want) in (0..=100).zip(column("disabled")) {
        for (_, source) in sources {
            assert_eq!(
                i64::from(volume_normalization::effective(base, source, false)),
                want,
                "disabled {base}"
            );
        }
    }
}

fn dedupe_cases() -> Vec<serde_json::Value> {
    let root = fixture_json("dedup_spec.json");
    let cases = root["test_cases"].as_array().unwrap().clone();
    assert!(!cases.is_empty());
    cases
}

fn check_dedupe(
    case: &serde_json::Value,
    kept: usize,
    groups: &[dedupe::DuplicateGroup],
    id_at: impl Fn(usize) -> String,
) {
    let label = format!("case {} ({})", case["id"], case["description"]);
    assert_eq!(
        kept as u64,
        case["expected_deduped_count"].as_u64().unwrap(),
        "{label}: count"
    );
    let expected = case["expected_duplicate_groups"].as_array().unwrap();
    assert_eq!(groups.len(), expected.len(), "{label}: group count");
    for want in expected {
        let key = want["key"].as_str().unwrap();
        let actual = groups
            .iter()
            .find(|g| g.key == key)
            .unwrap_or_else(|| panic!("{label}: group {key} missing"));
        let first = actual.first_occurrence_index;
        assert_eq!(
            first as u64,
            want["first_occurrence_index"].as_u64().unwrap(),
            "{label}: first index"
        );
        assert_eq!(
            id_at(first),
            want["first_occurrence_id"].as_str().unwrap(),
            "{label}: first id"
        );
        let ids: Vec<&str> = actual
            .occurrences
            .iter()
            .map(|o| o.track_id.as_str())
            .collect();
        let want_ids: Vec<&str> = want["duplicate_ids"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        assert_eq!(ids, want_ids, "{label}: duplicate ids");
    }
}

fn spec_field(t: &serde_json::Value, key: &str) -> String {
    t[key].as_str().unwrap().to_owned()
}

#[test]
fn dedup_spec_playlist_tracks() {
    for case in dedupe_cases() {
        let input: Vec<PlaylistTrack> = case["input"]
            .as_array()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(i, t)| PlaylistTrack {
                id: spec_field(t, "id"),
                playlist_id: "test-playlist".into(),
                track_source: SourceType::Custom,
                track_id: spec_field(t, "id"),
                position: i32::try_from(i).unwrap(),
                added_at: "2026-01-01T00:00:00Z".into(),
                title: spec_field(t, "title"),
                artist: spec_field(t, "artist"),
                album: None,
                duration_ms: None,
                image_url: None,
                url: None,
            })
            .collect();
        let result = dedupe::deduplicate(input.clone());
        check_dedupe(&case, result.items.len(), &result.duplicate_groups, |i| {
            input[i].track_id.clone()
        });
    }
}

#[test]
fn dedup_spec_tracks() {
    for case in dedupe_cases() {
        let input: Vec<Track> = case["input"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| {
                Track::new(
                    spec_field(t, "id"),
                    spec_field(t, "title"),
                    spec_field(t, "artist"),
                    SourceType::Jellyfin,
                )
            })
            .collect();
        let result = dedupe::deduplicate_tracks(input.clone());
        check_dedupe(&case, result.items.len(), &result.duplicate_groups, |i| {
            input[i].id.clone()
        });
    }
}
