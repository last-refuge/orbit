# Changelog

All notable changes to Orbit are recorded here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
[Semantic Versioning](https://semver.org/).

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

[0.1.0-alpha.1]: https://github.com/last-refuge/orbit/releases/tag/v0.1.0-alpha.1
