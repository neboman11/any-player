# App changelog

## Unreleased

- Normal launch now assembles the real SQLite, keyring, provider, local/Spotify playback, library, transfer, DJ and sync services. `--demo` remains an independent offline mode.
- Startup migrates legacy state in place, restores playback paused, bounds provider restoration, and offers per-step Retry and Continue without services. Logs rotate daily with seven retained files; fatal startup errors use a native dialog.
- Artwork loads through a 100 MiB on-disk LRU; Jellyfin/Plex auth headers are attached per request and never stored in cache keys. Streams from servers marked self-signed accept invalid certificates in the local HTTP source.
- Queue artwork loads only around the visible rows, with off-thread thumbnail decoding; the compact player shows current artwork. Demo artwork is available offline.
- DJ, catalog refresh and sync start only after the window is first shown or playback is requested.
- The Now Playing album/source/quality line is hidden when nothing is playing.
- Added tray controls, close-to-tray with an initial notice and unavailable-host fallback, single-instance focus, `--minimized`, and User-origin OS media controls with track metadata and saved state on Quit.
- Library playback now delegates occurrence selection and distinct handling to the repository instead of rebuilding a separate UI queue.
- Refined native light/dark music-player layout, accessible SVG transport controls, keyboard focus rings, active navigation, rounded embedded artwork, uniform queue rows with quality and duration, playlist card grid, responsive track tables, and compact persistent playback bar.
- Kept service ports and action callbacks unchanged. Track presentation rows now include a display-only quality string.
- Added demo screenshot routes: `--demo --demo-page now-playing|playlists|playlist|union|search|settings|providers|dj`. Add `--demo-small` for the 800×560 viewport. These switches only operate in offline demo mode.
- UI smoke coverage exercises accessible icon labels and navigation at the minimum window size.
- The "sync settings didn't carry over" banner appears only after an upgrade from the legacy desktop (an unmigrated legacy database or a migrated `playback_state.json`) while no sync target is set, and stays dismissed once dismissed. Fresh installs never show it.
- Playback failures now appear as dismissible messages and notifications; Spotify startup shows a Starting indicator. Use `--demo --demo-playback-error` to preview the error presentation.
- Configuring a sync target immediately hides and permanently dismisses the legacy sync reminder.
- Track tables share one column-width model between header and rows (lead #/drag and actions columns included); headers are plain sortable column titles with a ▲/▼ indicator. Narrow tables hide Album, then Via, and fold row actions into a single "More actions" menu.
- Playlist detail: compact Back button beside the title, compact action row with Play All as the primary action, compact union source rows; banner and notification dismiss buttons are compact and right-aligned.
