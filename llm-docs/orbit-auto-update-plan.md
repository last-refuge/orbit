# Orbit auto-update plan (ORB-011)

Goal: after the one-line install, a user never runs the installer again. Orbit notices new GitHub releases, tells the user inside Orbit, and can replace itself in place, restart, and roll back if the new build is broken. Optional fully automatic mode.

## 1. Update source and policy

- Source: GitHub Releases API for last-refuge/orbit (`/repos/last-refuge/orbit/releases?per_page=10`), newest non-draft release whose assets include `orbit-<version>-<arch>-unknown-linux-gnu.tar.gz` and `.sha256`. The "latest" endpoint is not used (it hides pre-releases, which bit the installer).
- Channel setting `update.channel = "prerelease" | "stable"` (default `prerelease` while 0.x; flips to `stable` at 1.0). Stable ignores releases marked pre-release.
- Version comparison with the `semver` crate (already in the dependency graph; becomes a direct dependency, inventory regenerated), including pre-release ordering (0.1.0-alpha.2 > 0.1.0-alpha.1).
- Rate limits: unauthenticated API allows 60 requests/hour per IP. Check at most every 6 h (`update.check_interval_hours`, min 1), with a conditional request (`If-None-Match` ETag) so unchanged answers cost nothing, 0–10 min jitter on start-up, exponential back-off on 403/429, and never block the UI on the network.
- HTTP: Orbit has no HTTP or TLS crate. The client runs `curl` as a subprocess (`install.sh` already requires curl, so it is on every machine Orbit was installed on), HTTPS only, hosts limited to `api.github.com`, `github.com` and `objects.githubusercontent.com`. Pulling in `ureq`/rustls instead is a later decision if the subprocess proves awkward.
- Only installs made by `install.sh` are updatable in place. The installer writes `~/.local/share/orbit/install.json` (`method = release|source`, `prefix`, `installed_version`, `installed_at`). Builds run from `target/` or without the marker only show "new version available" plus the install line; they never self-replace.

## 2. Checking and informing

- Daemon task: background check on the schedule above, state in `$XDG_STATE_HOME/orbit/update.json` (`last_check`, `etag`, `latest { version, tag, url, notes, published_at }`, `skipped_version`). Exposed in `orbit state` under `update`.
- CLI: `orbit update check` (forces a check, prints current, latest, channel), `orbit update notes` (release notes of the latest). Same noun-verb shape as `orbit config …` and `orbit assets …`.
- In-app notice: when `latest > current` and not skipped, the overview shows a small pill next to the mark and wordmark ("0.1.0-alpha.2 available"), hover reveals Update and Later. The pill uses the existing theme tokens and the 4.5:1 contrast rules; it never covers cards or the search field.
- Settings window gains an Updates section: current version, latest version, channel, mode, last check time, buttons **Check for updates**, Update now, Skip this version, Release notes (opens the GitHub release in the default browser via `xdg-open`). "Check for updates" runs the check immediately, ignoring the 6 h schedule and the ETag (a forced request), shows a spinner for its duration, then either "You are on the latest version" or the new version with Update now; it is also reachable from the keyboard (the section is a focusable row like every other) and equals `orbit update check` on the command line.
- Desktop notification (optional, `update.notify = true` by default): through the freedesktop notifications D-Bus API using `notify-rust` (already in the dependency graph through GPUI; becomes a direct dependency), when a new version is first seen, once per version. Whatever notification daemon the session runs renders it (on Omarchy that is the shell's own; mako is not installed on djinn).

## 3. Applying in place

- `orbit update apply [--version vX]` (also what the pill and settings buttons call over the daemon socket):
  1. Download tarball and `.sha256` to a temp dir on the same filesystem as the installed binary (`~/.local/bin/.orbit-update-<pid>/`). Verify sha256 before anything else; on mismatch delete and report.
  2. Verify a detached minisign signature (`.minisig`) once releases ship one (section 5). The public key is embedded in the binary; a missing signature is a hard failure once the key is set, so a tampered or wrong asset can never be installed.
  3. Sanity-run the new binary: `new/orbit --version` must print the expected version. (`orbit doctor` has no quiet mode and needs the live session; it stays a diagnostic, not a gate.)
  4. Atomic swap: keep the current binary as `orbit.prev`, `rename(2)` the new one over `~/.local/bin/orbit`. The running daemon keeps its old inode, so nothing breaks mid-swap. Then `orbit assets install` to refresh the desktop entry and icons, and update `install.json`. Hyprland's generated `hypr/orbit.lua` binds the absolute path `~/.local/bin/orbit`, so the shortcut stays valid across the swap.
  5. Restart: if running under the user unit (`INVOCATION_ID` is set by systemd), `systemctl --user restart orbit.service`; otherwise the daemon runs its normal shutdown (release the live bind, close the overlay and corner surfaces, close the socket) and execs the new binary with `--after-update <old-version>`. On restart it re-registers the shortcut and hot corner exactly as on normal start (string `hl.bind`, never Lua handles, conflict check unchanged, corners synced once Hyprland has reported the monitors).
  6. Health check: the new daemon must answer `orbit state` within 10 s and report `ok`. If not, the updater (a detached helper process started before the swap, or the systemd unit's `ExecStartPost`) renames `orbit.prev` back and restarts, then records `update.json.failed = <version>` so the pill reads "update failed, rolled back" with a link to open an issue. `orbit update rollback` does the same by hand.
- Never sudo. Nothing in `~/.config/hypr` is touched. Settings are preserved (config.toml is never rewritten by the updater).

## 4. Automatic mode

- `update.mode = "notify" | "auto" | "off"`; default `notify`.
- `auto`: when a new version is seen, apply it at a quiet moment: overview closed, no capture session open, no drag in progress, at least 2 min since the last user interaction with Orbit. Restart is a sub-second blink of the hot corner surface and the bind; the overlay is never open during it. A desktop notification says what was installed, with a Roll back action for 24 h.
- `auto` is offered once in the settings window after the first successful manual update ("Keep Orbit up to date automatically?"); it is never turned on silently.

## 5. Release side

- Release workflow signs each tarball with minisign (private key in GitHub Actions secrets, public key committed to the repo and embedded via `include_str!`), uploads `.minisig` next to `.sha256`, and rejects a tag that does not match Cargo.toml (already the case).
- CHANGELOG.md section for the tag becomes the release notes (already the case); the client shows the first 20 lines in the settings window.
- aarch64 build added to the matrix (`ubuntu-24.04-arm` runner) so update works there without compiling.

## 6. Code layout

- New crate `crates/orbit-update` (no GPUI dependency): GitHub client with ETag cache (curl subprocess), semver selection, download with resume (`curl -C -`), sha256 and minisign verification, atomic swap, rollback, `install.json` handling. Pure functions get unit tests with a fake releases JSON; the client is tested against a local HTTP server fixture.
- Daemon (`crates/orbit`): scheduler task on the existing executor, socket requests `update check`, `update apply`, `update skip`, `update rollback` (a `Request::Update` variant in the line-based protocol, next to `show`, `toggle`, `settings`), state in `orbit state`.
- UI (`crates/orbit-ui`): pill in the overview, Updates section in the settings window, a11y labels, review checks for both.
- `install.sh`: writes `install.json`; `--uninstall` also removes `orbit.prev`, the update cache and the state file.

## 7. Verification

- Unit: version selection across channels, ETag handling, checksum and signature failures, swap and rollback on a temp dir.
- End to end in a throwaway HOME on djinn: install v0.1.0-alpha.1 with the real installer, serve a fake v0.1.0-alpha.2 release from a local HTTP server (`ORBIT_API_BASE` / `ORBIT_DOWNLOAD_BASE` already exist in `install.sh` for exactly this), `orbit update apply`, confirm `orbit --version`, the pill disappearing, the bind and hot corner still working; then serve a deliberately broken binary and confirm automatic rollback within 10 s.
- review-check additions: pill appears under fake-hyprland when the fake API reports a newer version; settings section renders; `orbit state.update` fields present.

## 8. Phases and estimate

1. ORB-011a Check and inform (crate skeleton, scheduler, CLI, pill, settings section, notifications): about 1 day.
2. ORB-011b Apply, restart, rollback, install marker, installer changes: about 1 day.
3. ORB-011c Signing in the release workflow, auto mode, aarch64: about half a day plus a release to prove it end to end (0.1.0-alpha.2 is the first version that can be updated to; alpha.1 users get the pill once alpha.2 exists, but must run the install line one last time since alpha.1 has no updater).

## 9. Risks

- GitHub rate limiting on shared IPs: mitigated by ETag, 6 h interval, back-off.
- A compromised release account could ship a bad binary: signing key in Actions secrets limits this to a compromise of the secrets as well; rollback limits blast radius.
- Hyprland restart timing: re-binding on restart reuses the start-up path that is already exercised by every daemon start.
- Users who installed from source or a future package manager: shown the notice only, never self-replaced, to avoid fighting pacman.
