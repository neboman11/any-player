# Cross-platform golden fixtures

Generated from the Kotlin Android app (`any-player-android`, branch `test/desktop-fixtures`) by the
`DesktopFixtureGenerator*Test` JVM unit tests, which call the real production functions. The Rust
`ap-model` crate must reproduce every value byte-for-byte. Values that must be byte-exact (JSON
documents, hashes) are stored as JSON strings.

| File | Source (Android) | Contents |
|---|---|---|
| `sync_signatures.json` | `app/.../sync/SyncBase.kt` — `appStateSignature`, `playlistsSignature`, `settingsSignature`, `syncBaseTarget` | `app_state`, `playlists`, `settings`, `base_target` vectors (`expected` = hex SHA-256, or `<normalized url>#<hex>`). Tracks use AppJson encoding (camelCase, lowercase source, nulls/defaults omitted). |
| `state_transfer.json` | `core/data/.../transfer` — `AnyPlayerStateEnvelope` via `AppJson`, `StateTransferCrypto`, `LegacyStateTransfer.decodeAndVerify` | `cases[]`: `envelope_json` (exact file text), `integrity_input` (envelope with `integrity.sha256 = ""`), `sha256`, `passphrase`, `encrypted_file` (exact PRIVATE file text, real `encrypt` with `kdfIterations = 1000`; random salt/nonce, so it is a decrypt vector). |
| `config_ids.json` | `core/data/.../transfer/LegacyConfigFiles.kt` | `name_uuid[]`: `UUID.nameUUIDFromBytes(key UTF-8)` (v3/MD5) as used by `stableId`; `export_json[]`: exact text of a `List<ConfigCustomPlaylist>` encoded with the sync `Json { ignoreUnknownKeys = true }` config. |
| `volume_normalization.json` | `core/model/.../VolumeNormalization.kt` | `effective(base, source, enabled=true)` for base 0..=100 per source; `disabled` = enabled=false. |
| `break_schedule.json` | `core/model/.../BreakSchedule.kt` `reduce` + `songsAway` | `cases[]`: `intervals` = scripted results of the reducer's `random.nextInt(3, 6)` draws, consumed in order (every interval is consumed); `events[]` = `{event, queue}` with event in `enable`/`disable`/`queue_replaced`/`cursor_moved`/`break_started`/`generation_failed` and the `QueueState` passed to that step (`entries[{uid, track}]`, `order`, `cursor`, `content_version`); `expected[]` per step = `songs_away`, `target_uid` plus the full schedule state (`songs_until_break`, `anchor_order_pos`, `order_version`, `failed_on_current`, `enabled`). Tracks without a playable `url` are UNPLAYABLE (`backendFor`). |
| `dedup_spec.json` | copied verbatim from `core/model/src/test/resources/dedup_spec.json` | dedupe spec vectors. |
| `legacy-state.json` | copied verbatim from `core/data/src/test/resources/legacy-state.json` | legacy (Tauri-era) state file that must import. |

Notes on observed Android behaviour encoded in the vectors:
- AppJson / sync `Json` use `encodeDefaults = false`, so fields equal to their default (incl. `null`
  defaults, `format`, `version`, `encrypted`, `algorithm`, `kdf`, `isDistinct=false`) are omitted.
- `syncBaseTarget` only lowercase-matches `http://`/`https://`; `HTTPS://x` becomes `https://HTTPS://x`.
- Playlist/settings cases carry `input_text`, the exact JSON text Android parsed and hashed (`null` for the
  missing-namespace case); hash `input_text`. `input` is the same document re-serialized for readability:
  kotlinx keeps number literal text (`1e3`, `-0`) when hashing but pretty-printing rewrites it (`1000.0`, `0`).

## Regenerate

```sh
cd any-player-android
rm -rf /tmp/ap-fixtures
JAVA_HOME=/usr/lib/jvm/java-21-openjdk ANYPLAYER_FIXTURE_OUT=/tmp/ap-fixtures ./gradlew \
  :core:model:test --tests '*DesktopFixtureGenerator*' \
  :core:data:testDebugUnitTest --tests '*DesktopFixtureGenerator*' \
  :app:testDebugUnitTest --tests '*DesktopFixtureGenerator*'
cp /tmp/ap-fixtures/*.json core/model/src/test/resources/dedup_spec.json \
  core/data/src/test/resources/legacy-state.json ../any-player-desktop-v2/fixtures/
```

`ANYPLAYER_FIXTURE_OUT` is read from the environment (Gradle test workers inherit it) or a system
property; without it the tests only self-check. `encrypted_file` changes on every run (random salt/nonce).
