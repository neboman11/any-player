# Contributing

Any Player is a Rust workspace with a Slint UI. Read [docs/DESIGN.md](docs/DESIGN.md) first; this file is the short
list of rules.

## Setup

See the prerequisites in [README.md](README.md). Then:

```sh
cargo build --workspace
cargo run -p any-player -- --demo   # fake data; no servers or keyring needed
```

## Crate layering

```text
app -> ap-dj, ap-sync, ap-platform
ap-sync -> ap-playback, ap-library
ap-dj -> ap-playback, ap-library
ap-playback -> ap-library, ap-store, ap-model
ap-library -> ap-providers, ap-store
ap-providers -> ap-store, ap-model
ap-store -> ap-model
```

- `ap-model` has no IO and no async runtime; every other crate may depend on it.
- Dependencies point downward only. Do not add upward or sideways edges.
- `app` consumes service handles and state snapshots. It never touches SQL, `reqwest`, or audio types.
- Every service exposes `tokio::sync::watch` (state) and `broadcast` (events) receivers. Mutable state stays private.

## Threading

- Slint UI and view-model state run on the main thread. Services update the UI only through
  `slint::invoke_from_event_loop` / `Weak::upgrade_in_event_loop`.
- Network, sync, providers and orchestration run on the tokio runtime owned by `app`.
- The playback engine, Spotify backend and DJ scheduler are single tokio actor tasks driven by `mpsc` commands.
- Local audio decode is one OS thread per decoder feeding an `rtrb` ring; the `cpal` output callback never blocks
  or allocates.
- SQLite goes through the dedicated DB thread (writer) and the read-only pool. Never open ad-hoc connections.
- Large list transforms (≥1k rows) and DSP use `tokio::task::spawn_blocking`.
- Never block the main thread or a tokio worker with IO or heavy computation.

## Wire compatibility

Sync signatures, the `any-player-state` transfer file, config files, break schedule and dedupe behavior must match
the Android app byte-for-byte. The golden vectors in `fixtures/` are generated from the Android app's production
code; Rust must reproduce them. Never hand-edit a fixture to make a test pass. If Android behavior changes,
regenerate them:

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

Adjust the final path to where your desktop checkout lives. Details are in
[fixtures/README.md](fixtures/README.md).

## Testing

- `cargo test --workspace` must pass.
- Tests are offline and deterministic: no real network, no real keyring, no wall-clock sleeps, no ambient
  randomness (inject a seed or clock). Use fakes and local mock servers.
- No `#[ignore]` tests. A test either runs everywhere in CI or does not exist.
- Bug fixes come with a regression test; new behavior with tests at the lowest crate that owns it.

## Lint and format

```sh
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

Clippy pedantic is enabled workspace-wide (see `[workspace.lints]` in `Cargo.toml`); `unsafe` is denied. Do not
leave `dbg!`, `todo!` or `unimplemented!` behind. CI runs `cargo fmt --all -- --check`, clippy and
`cargo test --workspace --locked`.

## Commits

Use [Conventional Commits](https://www.conventionalcommits.org): `type(scope): summary`, with types such as `feat`,
`fix`, `refactor`, `perf`, `test`, `docs`, `build`, `ci`, `chore`. Use the crate name as scope when it fits
(`fix(ap-sync): ...`). Keep commits focused; mark breaking changes with `!`.
