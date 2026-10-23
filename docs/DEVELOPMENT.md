# Orbit development guide

The engineering notes that used to be the README: how the crates fit together, how each slice was checked, and the evidence behind it. Visitors start at [README.md](../README.md).

## Layout

| Crate | Role |
| --- | --- |
| `orbit` | Binary, CLI and process lifecycle |
| `orbit-ui` | GPUI views, focus, accessibility, theme tokens |
| `orbit-core` | Framework-free model, layout, commands, state transitions |
| `orbit-hyprland` | Hyprland snapshots, events, typed dispatch, reconciliation |
| `orbit-capture` | Preview capture protocols, buffers, preview cache |

## Pins

GPUI, `gpui_platform` and Zed's crates.io patches all come from Zed revision
`56934ec0560bce8929025497411de7f5322fb6ca` (2026-10-17). `rust-toolchain.toml`
pins Rust 1.98.1 to match that revision. Bump them together: update `rev` in
`Cargo.toml`, re-copy the relevant `[patch.crates-io]` entries from Zed's root
`Cargo.toml`, match its `rust-toolchain.toml`, then rerun every check below.

Only the `wayland` backend is compiled; X11 sessions are out of scope.

## Build and check

Arch / Omarchy build dependencies:

    sudo pacman -S --needed rustup base-devel wayland libxkbcommon vulkan-icd-loader fontconfig freetype2

Then:

    cargo build --release
    cargo fmt --all --check
    cargo clippy --workspace --all-targets -- -D warnings
    cargo test --workspace
    scripts/license-inventory.py --check

## Smoke checks

On the target host, from a Hyprland session:

    target/release/orbit smoke             # plain window; Escape closes
    target/release/orbit smoke --overlay   # full-output layer-shell overlay; Escape closes

`--exit-after-ms N` makes either one quit on its own. `scripts/smoke-headless.sh`
runs both inside a headless sway session and saves screenshots; CI uses it. It
proves rendering on Wayland, not Hyprland behavior.

## Overlay lifecycle (ORB-003)

Workspace swapping has an isolated native input regression check (fake compositor):

    bash scripts/workspace-reorder-check.sh target/release/orbit

It starts headless sway and fake Hyprland in a private runtime and config
directory. It checks real GPUI drag input, keyboard parity, cancellation,
edge scrolling, persistence across restart, failed saves, and the existing
window drag path. Needs sway, jq and Python; optional grim screenshots contain
only the fixture desktop. `SWAY_BIN` can point to a locally unpacked sway.

    target/release/orbit daemon &          # resident process, hidden at rest
    target/release/orbit toggle            # on the focused monitor
    target/release/orbit show --output DP-1
    target/release/orbit status            # visibility, opens, last open time
    target/release/orbit hide | quit

The overlay is a full-output, non-reserving `zwlr_layer_shell_v1` surface on
the overlay layer with exclusive keyboard focus; Escape dismisses it. The
daemon listens on `$XDG_RUNTIME_DIR/orbit/orbit.sock` in a 0700 directory.
Requests that arrive together are applied in order and reconciled once, so a
burst of toggles settles by parity with at most one surface open.

    scripts/overlay-lifecycle.sh [--stacking]   # automated checks on Hyprland

## Capability report and fixtures

    target/release/orbit doctor                       # human-readable report
    target/release/orbit doctor --json                # same, as JSON
    target/release/orbit doctor --record docs/evidence/doctor/$(hostname)

`doctor` reports Orbit and GPUI versions, session variables, Omarchy and
Hyprland versions, GPUs, monitors, workspace and window counts, and every
Wayland global with the ones Gates A and B depend on highlighted. `--record`
also saves each Hyprland snapshot reply as a fixture for tests. Titles,
initial titles, last-window titles, XDG descriptions and monitor serials are
replaced with `<redacted len=N id=…>`; ids are keyed per run so they only
correlate within one recording.

Owned test windows have the app id `dev.orbit.fixture.NAME`. Tests may move,
focus or close those and nothing else.

    scripts/fixtures.sh spawn     # tiled, floating, fullscreen, special (+ XWayland via xterm)
    scripts/fixtures.sh list
    scripts/fixtures.sh clean     # closes fixture windows only

Fixtures open silently on workspaces 91–93 and `special:orbitfx`.

## Window previews (ORB-004)

    target/release/orbit capture                      # capture and check every fixture
    target/release/orbit capture --record DIR --label NAME --repeat 20
    target/release/orbit preview                      # live fixture previews in a window
    scripts/capture-check.sh                          # all scenarios, on Hyprland

Capture uses `ext-image-copy-capture-v1` with toplevel sources from
`ext-foreign-toplevel-list-v1` and shared-memory buffers, on its own Wayland
connection. `hyprland-toplevel-mapping-v1` maps each toplevel to its Hyprland
window address. In the ORB-004 checks only Orbit's own windows (fixtures and
the preview) are ever captured; titles are never read into the capture
state. The running overview captures the windows it shows and, since
ORB-008, one frame of the output for its backdrop (see "Brand fidelity").

`capture` checks each frame against the fixture's known look (Slate card,
accent border), so a black frame or another window's pixels fail the run.
`preview` keeps one session per window, converts and downscales off the UI
thread to at most 640x400, and frees each replaced texture from GPUI's atlas.
The protocol flow is covered by `crates/orbit-capture/tests/fake_compositor.rs`,
an in-process compositor speaking the same protocols.

## State and reconciliation (ORB-005)

`orbit-core::Store` is a synchronous, deterministic state machine. Adapters
feed it events, query answers, dispatch results and a millisecond clock; it
returns effects (queries to run, requests to send, finished operations).

- Identity: an epoch per compositor connection, and windows as address plus
  a local generation, because addresses are reused. Input from an older
  epoch or for a replaced window is rejected.
- Sync: events never replay over a query answer. If events touch a kind
  while its query is in flight, that kind is queried again once the answer
  lands. Lost events (`on_gap`) and buffer overflow force a full query.
- Moves: one typed request per window at a time (later ones queue), pending
  until the compositor state shows the window on the destination. Events
  count only after the compositor accepted the request (adapters deliver all
  readable events before reporting it); a timeout re-queries and either
  confirms or fails with `NotObserved`. Windows known only from an open event
  can't be moved until a query confirms their identity.

`crates/orbit-core/tests/simulation.rs` runs seeded chaos (lost and late
events, stale answers, lost acks, rejected requests, address reuse) and checks
convergence, exactly-once completion and that every confirmed move happened.

    target/release/orbit events --duration-ms 10000    # titles redacted
    scripts/record-events.sh                            # records a replay fixture

## The overview slice (ORB-006)

`orbit daemon` now keeps the store live from Hyprland while hidden, and
`orbit show`/`toggle` open the overview: a workspace rail above window cards
for the previewed workspace, with live previews from the capture worker.

- Open, inspect: opens on the invoking monitor's workspace with the focused
  window selected. Tab moves between the grid and the rail; arrows in the
  rail preview a workspace without switching to it. (ORB-007 adds search to
  the Tab order; see below.)
- Move: drag a card onto a workspace tile, or Ctrl+Shift+M, choose, Enter.
  Moves are silent; Shift on drop or Shift+Enter moves and follows (focus
  goes to the window once the move is confirmed). Cards show "Moving to…"
  until the store confirms; failures are announced, never redirected.
- Select: Enter or a click on a card focuses that window (switching to its
  workspace) and closes; a click on a tile switches workspace.
- Close: Escape (it first leaves the move menu).

Control goes through `orbit-hyprland::control`: typed requests with the
window always named explicitly (a Lua dispatch without one acts on the
focused window), Lua syntax on 0.56+ and hyprlang dispatchers before.
Hyprland answers `ok` even for an address that does not exist, so moves are
confirmed from state, not from the reply.

    scripts/slice-check.sh               # on Hyprland, fixture windows only
    scripts/slice-check.sh --fake        # against scripts/fake-hyprland.py

The check starts `orbit daemon --test-hooks`, whose `orbit drive` requests
feed keys, clicks and drags through GPUI's input dispatch, and compares the
result with `hyprctl`. Results go to `docs/evidence/orb-006/<host>/`.

## Brand, search and keyboard (ORB-007)

The overview follows the brand guide's layout: the wordmark, a search field
above the workspace rail, and the window grid. Colors come from the guide's
tokens by role (`orbit_ui::theme::Theme`); unit tests check text at 4.5:1
and selection/focus boundaries at 3:1 on every surface they sit on. The
face is the bundled Inter (see ORB-008 below).

- Search: Ctrl+K, a click on the field, or just typing anywhere. It matches
  app names, classes, titles and workspace names across every workspace,
  best first; results show their workspace and monitor, and Enter (or a
  click) focuses the window, switching to its workspace. Arrows in the field
  move the caret; Enter waits while an input method is composing.
- Keys: Tab / Shift+Tab cycle search → workspaces → windows; arrows move
  within a region and between them (Up from the first row of windows reaches
  the rail, Up from the rail reaches search); Home/End jump to the ends.
  Ctrl+Shift+M opens "Move to workspace", where typing filters destinations.
  Escape cancels a drag, then the move menu, then the search, then closes.
  `orbit_core::KEYBOARD_PATHS` lists each pointer action with its keys.
- States: selection is a violet ring, glowing while keyboard focus is in
  the grid; a focused tile gets a sky ring; the active workspace has a
  dot, and tiles say "Active" or "Previewing" in their accessible label
  and in a caption on the focused tile. The selected card shows its title.
  Window states (floating, fullscreen, XWayland) are in the accessible
  label only.
- Accessibility: tiles, cards, previews and the search field have roles and
  labels (app, full title, workspace, monitor, badges); the keyboard
  selection is the accessible focus; the status line is a polite live
  region for previews, results, moves and failures.
- `orbit daemon --high-contrast` uses a black canvas with brighter borders
  and text; `--reduced-motion` drops the entry travel (content still fades
  in over 180 ms).

    scripts/review-check.sh              # on Hyprland, fixture windows only

`orbit drive` gains `type`, `compose`/`commit` (input-method text),
`click-search`, `drag … follow` (Shift-drop) and `a11y` (a label-free
summary of the accessibility tree, which GPUI only builds while assistive
technology is connected). Results go to `docs/evidence/orb-007/<host>/`.
The native visual and screen-reader review is manual.

Not yet: a monitor selector in the Tab order (multi-monitor is M3). Real
app icons arrived with ORB-008.

Reviewed natively by Jason 2026-10-19: behaviour accepted; visual fidelity
to the brand guide moved to ORB-008 (M2), ahead of the installable alpha
(now ORB-009).

## Brand fidelity (ORB-008)

Round 3 makes the overview the brand guide's hero at Mission Control's
level of finish (the round-1 review and the round-2/3 results are in
`docs/orb-008-design-review-findings.md`; the targets and how each is
checked are in `docs/orb-008-design-targets.md`):

- Surface: the whole output is frosted glass. On open the capture worker
  takes **one** capture of the output (`ext-image-copy-capture` with an
  output source), point-samples it to a sixth, blurs it (three box passes,
  about 4 ms) and hands it to the overview, which draws it edge to edge
  under the brand tint with a lit top edge. The capture lives only in
  memory for that open and is replaced on the next one; nothing is written
  to disk. If output capture is unavailable the glass is a solid Slate
  under the same tint. With `--test-hooks` (every scripted run) the
  desktop is never captured and the solid is used, so no desktop content
  can enter the evidence. `--no-glass` keeps the denser layers,
  `--opaque` and `--high-contrast` are unchanged.
- Layout, at the hero's proportions: search bar (1280 x 56, centered,
  radius 18), the workspace rail under it, and a fixed three-column grid
  of bare window previews filling the rest (two columns for one or two
  windows). Cards grow to fill the width (684 at 2304 logical, up to 760),
  rows fill the height and scroll past it, a partial last row is centered.
  Every size is a multiple of 4 logical px and origins land on whole
  device pixels at 1.25x.
- Previews are captured once to the device box they are drawn in and
  drawn at exactly that size (no second resample, no upscaling), at the
  window's real aspect, with radius 16, a one-device-pixel edge and a soft
  drop shadow; no frame fill. Under each: the app's real icon (36 px) and
  its name (18 px SemiBold), aligned with the image's left edge; the title
  only on the selected card.
- Motion: on open the previews fly from their windows' real screen
  rectangles (Hyprland's `at`/`size`) into the grid over 220 ms ease-out
  while the glass fades in; on close or activation they fly back and the
  layer closes after the animation (the focus hand-off still waits for the
  layer to close, so it is delayed by at most that). `--reduced-motion`
  keeps the fade only.
- Rail: the guide's strip of 176 x 112 tiles with the workspace number
  under each, the previewed one ringed violet, the keyboard-focused one
  sky, the active one dotted. Each tile is the desktop composited small:
  the frosted backdrop with the workspace's windows placed from their real
  geometry, filled with the previews already captured (the previewed
  workspace's) and drawn as outlines elsewhere, so no window off the
  previewed workspace is captured. With more workspaces than fit, the strip
  scrolls to keep the previewed or focused tile in view; every workspace
  stays reachable with the arrows.
- Chrome: none inside the hero's frame. State badges, the place label and
  the key hints are gone from the surface; states live in the accessible
  labels, hints appear behind Ctrl+/ (or F1), and only failures and the
  compositor link show as a toast (announcements still go to the live
  region).
- Icons: the window class is matched to an XDG desktop entry (file id,
  `StartupWMClass`, reverse-DNS segment, name) and its icon looked up in
  the icon theme (GTK settings, then the GNOME setting Omarchy writes, then
  hicolor and pixmaps), cached per class; a Gray letter tile is the
  fallback (`orbit_ui::icons`).
- Type: Inter 4.1 is bundled (`crates/orbit-ui/assets/fonts/inter/`, SIL
  Open Font License 1.1, license text alongside) and registered with GPUI at
  startup, so the overview uses it whatever is installed; Adwaita Sans is
  the fallback if loading fails. Only the static Regular, Medium, SemiBold
  and Bold faces are vendored, copied unchanged out of the release's
  `Inter.ttc`: GPUI's Linux text system does not apply variable-font axes.
  Scale: 18 (names), 17 (search), 15 (rail), 13 (secondary).
- The hand-traced mark and app icon drafts stay in `crates/orbit-ui/assets/`
  (see its README) but do not appear in the overview.

`orbit drive layout` reports the last frame's layout (search, rail and
grid areas; columns, card and slot sizes; each preview's drawn size
against its texture and the window's aspect; whether a desktop backdrop is
in use; no titles). `scripts/review-check.sh` checks the measurable
targets with it and saves the JSON under `docs/evidence/orb-008/<host>/`.

## Settings and engagement (ORB-010)

Everything a person may want to change lives in
`$XDG_CONFIG_HOME/orbit/config.toml` (every key has a default; the file is
written on first run), edited by the settings window or `orbit config`, and
hot-reloaded by the daemon within half a second:

    target/release/orbit settings                      # the settings window
    target/release/orbit config list | path
    target/release/orbit config get hot_corner.corner
    target/release/orbit config set hot_corner.corner top-left

- The **mark** at the top left of the overview is the settings entry
  (hover for the tooltip; click or Ctrl+, opens the window and closes the
  overview). It is the brand lockup: the symbol and the wordmark from
  Jason's brand files in `crates/orbit-ui/assets/brand/` (the wordmark is
  cut from the lockup at load and tinted to the theme's text colour).
- **Shortcut.** Wayland clients cannot grab global keys, so the bind is
  Hyprland's: the daemon binds it live over IPC (`hl.bind` through
  `hyprctl eval`, released on quit) and keeps a generated snippet at
  `$XDG_CONFIG_HOME/hypr/orbit.lua` so it survives restarts once
  `hyprland.lua` has `require("hypr.orbit")`. Orbit never edits
  `hyprland.lua` itself: the settings window shows the line and appends it
  only after an explicit confirmation that shows exactly what will be
  written. The default is Super+` (`grave`), or, when that is taken, the
  first of Super+Shift+grave and Super+Ctrl+Tab that nothing else uses
  (Super+Tab is never offered: Omarchy binds it); a combination another
  bind already uses is reported and not bound. The
  window records a new combination from a key press.
- **Hot corner.** Hyprland has none, so Orbit keeps a 2x2 px transparent
  layer surface in the chosen corner of the focused (or every) monitor;
  resting the pointer there for the dwell (150 ms) with no button held
  opens the overview, and, when "corner also closes" is on, resting there
  while it is open closes it.
- **Appearance:** auto/glass/solid/high-contrast, blur strength, tint
  strength (under 100 % the contrast guarantee over a bright wallpaper no
  longer holds), reduced motion. **Layout:** columns (auto/2/3/4), window
  titles (always/selected/never), special workspaces in the rail, rail at
  the top or bottom. **Behaviour:** follow a moved window by default,
  close on activate, search scope, double tap of the shortcut switches to
  the last window. **Previews:** refresh rate, capture other workspaces
  (real content in every rail tile), classes never captured. **Icons:**
  icon theme override. **Startup:** launch at login through a generated
  systemd user unit (`orbit.service`, opt-in).
- The command-line flags (`--no-glass`, `--opaque`, `--high-contrast`,
  `--reduced-motion`) still override the file, which scripted checks rely
  on; `ORBIT_CONFIG` points the daemon and `orbit config` at another file
  (the checks use a scratch one, so they never touch your settings).

`scripts/review-check.sh` covers the file round trip, the live bind (or
its reported conflict), the snippet, the hot corner surface and its
engagement through `orbit drive hot-corner`, live layout changes and the
settings window. Evidence: `docs/evidence/orb-010/djinn/settings-window.png`.

## Dependencies and licenses

`crates/orbit-capture/protocols/` vendors Hyprland protocol XML unmodified
(BSD-3-Clause, notice inside each file). `crates/orbit-ui/assets/fonts/inter/`
vendors Inter 4.1 (OFL-1.1). The inventory lists both.

`deny.toml` holds the license and source policy (`cargo deny check`).
`docs/dependency-inventory.md` is generated by `scripts/license-inventory.py`
and must be regenerated whenever `Cargo.lock` changes.

The production Lua transaction also has fault-injection checks:

    lua scripts/workspace-swap-lua-check.lua

For real layout preservation checks, start a **private** Hyprland instance with
an isolated `/tmp` runtime and its own output. Start `scripts/fixtures.sh spawn`
and one additional `dev.orbit.fixture.tiled-c` window on workspace 91, then run
Orbit with `--test-hooks` in that instance. Run:

    ORBIT_PRIVATE_TEST=1 bash scripts/workspace-swap-native-check.sh target/release/orbit

The script refuses a non-private runtime or non-fixture clients. It compares
workspace membership, exact position/size, monitor, groups, floating/fullscreen
and pinned state, including five active-workspace round trips. It does not
launch or stop the compositor; the harness owner must clean up its processes.
