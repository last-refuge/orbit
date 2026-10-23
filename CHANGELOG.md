# Changelog

All notable changes to Orbit are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

## [0.1.0-alpha.8] - 2026-10-23

### Added

- Workspace swap previews now hop and bounce: hovering over another workspace
  pushes its thumbnail into the dragged workspace's spot. Moving back or away
  springs it home, including when you reverse direction mid-animation.
- Drop targets stay fixed while thumbnails move. Windows are swapped only when
  you drop, and reduced-motion mode shows the preview without animation.

## [0.1.0-alpha.7] - 2026-10-23

### Fixed

- Dragging a numbered workspace now swaps its actual contents with the target
  workspace. Hyprland keeps the workspace objects and their tiling trees, floating
  geometry, fullscreen state, and monitor assignments intact. Swapping back
  restores the original workspace numbers.
- The rail stays in numeric order; the visual-only order saved by alpha.6 is
  ignored. The target tile is highlighted, and Ctrl+Shift+Left/Right performs
  the same swap with a neighboring numbered workspace.
- Swaps validate current window membership, roll back refused ID changes, and
  verify the compositor snapshot before reporting success. Named/special/New
  targets are excluded; Hyprland's Lua `workspace.change_id` support is required.

## [0.1.0-alpha.6] - 2026-10-23

### Added
- Drag workspace tiles to reorder Orbit's rail, with an insertion marker,
  edge scrolling, Escape to cancel, and Ctrl+Shift+Left/Right as a keyboard
  equivalent. The order is saved between sessions without renumbering
  Hyprland workspaces or moving their windows.

## [0.1.0-alpha.5] - 2026-10-22

### Fixed
- Settings window: the − and + buttons of number rows (corner dwell, blur,
  tint, preview refresh, update interval) also triggered the row's own
  click, which steps up; − did nothing and + stepped twice. The corner
  dwell can be stepped down again.
- The install marker (`~/.local/share/orbit/install.json`) is written only
  for the install being changed; the test suite can no longer rewrite the
  machine's own marker. `orbit update check` notes a marker that disagrees
  with the binary.

## [0.1.0-alpha.4] - 2026-10-21

### Changed
- Dragging a card to the workspace rail: the ghost shrinks smoothly to the
  size the window will have inside the tile as the pointer reaches the
  rail, stays centred under the pointer, and grows back when it leaves
  (reduced motion snaps). The tile no longer disappears under it.

## [0.1.0-alpha.3] - 2026-10-19

The first build with an aarch64 tarball, and the first the updater
requires to be signed.

### Added
- aarch64 release tarballs (alpha.2's arm build did not publish).

### Changed
- Releases from this version on must carry a valid minisign signature;
  the updater refuses an unsigned or badly signed one before unpacking.

### Fixed
- The updater scrubs its own test-only override variables from the
  processes it launches (the new binary's version check, the desktop
  entry refresh, the restarted daemon), so a stray value in the
  environment can never make a correct download look wrong.
- The blur timing test allows a cold CI runner (2 s budget in debug
  builds), which is what failed the aarch64 job.

## [0.1.0-alpha.2] - 2026-10-19

Staying up to date without the installer.

### Added
- Release checks: the daemon asks GitHub's releases list (conditionally,
  with an ETag; every `update.check_interval_hours`, 6 by default; never
  blocking the UI) and `orbit update check` asks at once. Channels:
  pre-releases (the default while 0.x) or stable only.
- The pill beside the logo in the overview when a newer version exists
  (or after an update, or after a rollback), opening the settings.
- An Updates section in the settings window: version and last check,
  Check now, Update, Skip this version, Release notes, mode, channel,
  interval, desktop notification.
- In-place updates: `orbit update apply` (and the settings window's
  Update) downloads the release tarball beside the binary, verifies its
  sha256, sanity-runs the new binary, keeps the old one as `orbit.prev`,
  swaps atomically, refreshes the desktop entry and icons, and restarts
  the daemon (through the login unit, or by replacing the process). A
  watcher rolls back and relaunches if the new daemon does not answer
  within 10 s; a crash-loop guard at start does the same after three
  early exits. `orbit update rollback` by hand. A failed version is never
  offered again.
- Automatic mode (`update.mode = "auto"`): installs at a quiet moment
  and says what was installed; offered once after the first manual update,
  never turned on silently.
- One desktop notification per new version.
- Signed releases: the workflow signs each tarball with minisign when the
  repository has the key; the client verifies the signature when the
  public key is embedded.
- aarch64 release builds.
- The installer records how Orbit was installed (`install.json`); only a
  release install replaces itself, a source build is told to rebuild.

### Changed
- Version 0.1.0-alpha.2; the first build that can update itself. Users of
  alpha.1 run the install line once more.

## [0.1.0-alpha.1] - 2026-10-19

The first tagged build: the overview works end to end on Hyprland 0.56 and
is ready for other people to try.

### Added
- The overview: a full-screen layer-shell surface with a workspace rail,
  a window grid with live previews, search over windows, apps and
  workspaces, and full keyboard and pointer paths.
- Live window previews through `ext-image-copy-capture`, with a capability
  report (`orbit doctor`) and an in-house frosted-glass backdrop.
- Moving windows by drag or from a filtered menu, verified against
  Hyprland's events; "move and follow" with Shift.
- Activation that brings the window forward: focus and raise are sent the
  moment a card is clicked, re-sent once the overlay is gone, confirmed on
  Hyprland's events, and timed (`orbit state`, `last_activation`).
- Settings in `$XDG_CONFIG_HOME/orbit/config.toml`, hot-reloaded; a settings
  window (Ctrl+, inside the overview); `orbit config` on the command line.
- A live shortcut bind over IPC (default Super+`), a generated
  `hypr/orbit.lua` snippet, conflict detection, and an opt-in `require`
  line for `hyprland.lua` that is only ever written after confirmation.
- A hot corner on any corner of the primary or every monitor.
- Launch at login through a generated systemd user unit (opt-in).
- The brand: the Orbit symbol and wordmark in the overview and the settings
  window, and the application icon for the desktop entry.
- `orbit assets install` for the desktop entry and hicolor icons; the
  one-line installer (`install.sh`) and the release workflow.

### Known limitations
- Hyprland 0.56 or newer only (the Lua configuration API).
- Captures of other workspaces are off by default; their rail tiles show
  window placement rather than content until enabled.
- No vector logo source yet; the brand files are rasters.

[0.1.0-alpha.5]: https://github.com/last-refuge/orbit/releases/tag/v0.1.0-alpha.5
[0.1.0-alpha.4]: https://github.com/last-refuge/orbit/releases/tag/v0.1.0-alpha.4
[0.1.0-alpha.3]: https://github.com/last-refuge/orbit/releases/tag/v0.1.0-alpha.3
[0.1.0-alpha.2]: https://github.com/last-refuge/orbit/releases/tag/v0.1.0-alpha.2
[0.1.0-alpha.1]: https://github.com/last-refuge/orbit/releases/tag/v0.1.0-alpha.1
