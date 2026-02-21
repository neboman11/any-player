# Contributing to Any Player

Thanks for helping improve Any Player. This guide covers the current development workflow, quality checks, and contribution expectations for this repo.

## Before you start

1. Read [README.md](README.md) for setup and app overview.
2. Read [copilot-instructions.md](.github/copilot/copilot-instructions.md) for repository-specific coding rules.
3. Install Tauri prerequisites for your OS: https://tauri.app/start/prerequisites/

## Development setup

```bash
pnpm install
```

Run the desktop app:

```bash
pnpm tauri dev
```

Frontend-only mode (optional):

```bash
pnpm dev
```

## Recommended contributor workflow

1. Create a feature branch.
2. Keep changes scoped to one purpose per PR.
3. Run the checks below before opening a PR.
4. Update docs when user-visible behavior or developer workflow changes.
5. Open a PR with a clear description, validation steps, and screenshots/GIFs for UI changes.

## Quality checks

Run these before submitting:

```bash
pnpm lint
cd src-tauri && cargo clippy --all-targets --all-features -- -D warnings
cd src-tauri && cargo test
```

Notes:
- There is currently no standard `pnpm test` script in this repo.
- Manual validation is expected for UI, provider auth flows, and playback behavior.

## Project architecture (current)

- Frontend (`src/`): React pages/components, hooks, and typed Tauri API wrappers.
- Backend (`src-tauri/src/`): Tauri commands, provider integrations, playback engine, cache/state, and SQLite persistence.
- App communication: frontend uses typed invokes in `src/api.ts`; backend exposes commands under `src-tauri/src/commands/`.

## Coding guidelines

### Rust
- Use `snake_case` for functions/modules/variables and `PascalCase` for types.
- Prefer focused modules (`commands`, `providers`, `playback`, `database`, `state`).
- Return explicit errors; propagate with `?` and include context in messages.
- Keep async lock scopes tight (`Arc<Mutex<T>>`) to reduce contention.
- Use `tracing` for operationally useful logs.

### TypeScript/React
- Use `camelCase` for variables/functions and `PascalCase` for components/types.
- Keep components functional and hook-based.
- Put shared contracts in `src/types.ts` and keep them aligned with backend payloads.
- Route backend calls through `src/api.ts` (avoid ad hoc `invoke` usage in components).
- Favor small, composable hooks for auth/playback/playlist/search logic.

### General
- Match existing patterns before introducing new abstractions.
- Avoid unrelated refactors in feature/fix PRs.
- Keep naming and folder placement consistent with nearby code.

## Security & data handling

- Never commit secrets or provider tokens.
- Do not store credentials in plaintext; preserve existing secure storage patterns.
- Validate and sanitize user-provided values in command handlers.
- Preserve safe handling of exported configuration/state payloads.

## Documentation expectations

Update relevant docs when behavior changes:
- [README.md](README.md) for user/developer workflows and feature scope.
- [docs/android-companion-implementation-spec.md](docs/android-companion-implementation-spec.md) for Android companion scope/contracts (when applicable).
- Inline doc comments for non-obvious logic in Rust/TypeScript modules.

## Pull request checklist

- [ ] Changes are scoped and follow existing architecture.
- [ ] `pnpm lint` passes.
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes.
- [ ] Relevant `cargo test` checks pass.
- [ ] Manual verification completed for affected flows.
- [ ] Docs updated if behavior/workflow changed.

## Questions

- Open an issue for discussion.
- Reference existing patterns in the codebase and docs.
- Ask maintainers in the PR thread when design tradeoffs are unclear.

Thanks again for contributing.
