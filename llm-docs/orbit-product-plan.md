# Orbit — Product & Implementation Plan

**A higher view for your desktop.**

See everything. Switch anywhere. Stay in flow.

**Planning baseline:** October 18, 2026 · **Status:** proposed, before implementation · **Platform:** Omarchy / Hyprland / Wayland · **Stack:** Rust + GPUI

## 01 · Product direction

Orbit is a native desktop overview for Omarchy. One invocation reveals the user's workspaces and the windows on the current workspace. Users can recognize a window visually, activate it, move it to another workspace, or switch context without remembering where everything lives. The product should feel like an integrated part of the desktop: immediate, quiet, and predictable.

The long-term goal is Mission Control's complete workflow coverage translated into Linux behavior. The first usable release must deliver the four requested essentials together: workspace overview, current-workspace window overview, drag-and-drop window transfer, and workspace switching. Broader parity remains explicit in the roadmap; a smaller alpha is not a claim of full parity.

**Primary users:** Omarchy users who distribute development, communication, research, and personal work across several workspaces. Keyboard users and pointer users receive equivalent paths. The core unit is a **window**: dragging one browser window does not silently move every window of that application.

**Product principles**

- Preserve context. Stable layout, recognizable previews, and persistent workspace identities make location learnable.
- Make the compositor authoritative. Show pending operations until Hyprland confirms the resulting state.
- Open quickly and disappear cleanly. Never leave an invisible surface intercepting input.
- Be useful when previews fail. Titles, application icons, workspace membership, and controls remain available.
- Keep work local. No account, cloud service, screenshot uploads, or telemetry required.

**Scope assumption:** Omarchy on Hyprland is the first supported environment. Other Linux compositors require separate capability-tested adapters; “Linux application” does not imply GNOME, KDE, Sway, or X11 session support at launch. XWayland application windows inside Hyprland are part of the initial test scope.

## 02 · Mission Control coverage

Apple's overview and Spaces documentation establish the reference workflows. Orbit will translate their intent rather than copy macOS-specific desktop mechanics. [Apple: Mission Control](https://support.apple.com/en-sg/guide/mac-help/mh35798/27/mac/27), [Apple: Spaces](https://support.apple.com/guide/mac-help/work-in-multiple-spaces-mh14112/mac).

| Capability | Orbit behavior | Delivery / acceptance |
| --- | --- | --- |
| See every workspace | A workspace rail shows discovered and configured empty destinations, with names, monitor ownership, counts, and active indicators. | Alpha: numeric, named, empty, and special workspaces represented without duplicates. |
| See current workspace windows | An uncluttered, non-overlapping grid of previews with app icons and titles. Floating, tiled, and fullscreen windows remain identifiable. | Alpha: every eligible window appears once; Orbit and shell surfaces are excluded. |
| Activate a window or workspace | Click or keyboard selection switches context, gives focus to the intended window, and dismisses Orbit. | Alpha: repeated activation reaches the exact target; cancellation preserves context. |
| Move windows between workspaces | Drag a window onto a destination or use “Move to workspace.” Remain in the overview by default. | Alpha: only the selected window moves; the destination is confirmed before success is shown. |
| App Exposé and grouping | Filter to the previously focused app; optionally group windows by app while retaining individual selection. | Beta: matching uses reliable app identity with user overrides for ambiguous XWayland classes. |
| Workspace creation and removal | Add a logical destination; retire an Orbit-managed destination after moving its windows to a chosen survivor. | Beta: never close windows as a side effect; handle partial migration explicitly. |
| Workspace ordering | Persist display order in Orbit; expose compositor order changes only where supported. | Beta: rail ordering never silently renumbers workspaces or rewrites numeric shortcuts. |
| Fullscreen and Split View equivalent | Show fullscreen status; offer explicit pair-and-tile workflows using the supported Hyprland layout. | Parity phase: preserve/restore prior state; document differences from macOS Spaces. |
| Show desktop and restore | Provide a reversible desktop-reveal command with a tested compositor strategy. | Parity phase: exact surviving windows restore; no permanent layout damage after interruption. |
| App-to-workspace assignment | Support “open on this workspace” rules; expose “all workspaces” only for supported window types. | Parity phase: clarify Hyprland pinning limits and preview proposed rule changes. |
| Gestures, corners, and edge activation | Optional compositor-bound gestures, hot corners, and drag dwell entry points. | Parity phase: no collision with existing bindings; no invisible input capture while closed. |
| Multiple displays | Show all workspace groups and their monitor labels; open on the invocation monitor. | Beta: crossing monitor groups and hotplug preserve identities and correct scaling. |
| Cross-app content transfer | Allow copy → overview → activate → paste naturally; investigate file drag-through and desktop drops. | Basic switching supports clipboard workflows; external drag-through is a parity feasibility gate. |

**Orbit additions:** search across app names, window titles, and workspace labels; keyboard move commands; preview privacy controls; a capability report explaining unavailable features. Historical Dashboard widgets are outside the current Mission Control reference scope.

**Parity accounting:** every row gets a scenario, evidence, supported version range, and status of implemented, adapted, blocked, or unverified. Do not market full parity while required workflows remain blocked. Desktop file drops also depend on a desktop surface that accepts files; an Omarchy wallpaper alone is not that surface.

## 03 · The interaction model

### Open, explore, select

Propose **Super + Tab** as an onboarding suggestion, subject to a binding conflict check. `orbit toggle` is the stable integration command. Save the invoking monitor, its active workspace, and the focused window before opening. Initial view: all workspace groups in the rail, windows from the invocation monitor's active workspace in the main area, and the previously focused window selected.

Workspace tiles distinguish **currently active**, **being previewed**, and **keyboard focused** states with labels and borders. Hover does not change the desktop. Arrow navigation previews a workspace in Orbit; Enter or a click commits switching and dismisses. If a workspace is already visible on another monitor, select that monitor rather than unexpectedly relocating the workspace. Define special-workspace activation separately from ordinary workspace switching.

Window cards preserve aspect ratio, show an app icon and readable title, and carry fullscreen, floating, pinned, or grouped badges where relevant. Start with deterministic row packing, preserve relative order during updates, and avoid animated reshuffling while a pointer or keyboard selection is active. Scroll or virtualize crowded layouts rather than shrinking windows into illegible postage stamps.

### Drag a window

1. Pointer movement past a small threshold starts a drag. The source remains visible with a pending placeholder; a lightweight preview follows the pointer.
2. Valid workspace targets highlight in violet with an explicit destination label. The rail scrolls near its edges. A proposed 450 ms hover dwell previews a target inside Orbit without switching the real workspace.
3. Drop emits one move command for the original window identity. Moves stay on the current real workspace by default; an explicit “Move and follow” alternative is available.
4. Wait for an event or refreshed compositor snapshot confirming membership. Then settle the card, update counts, and announce “Moved to Research.”
5. Escape before drop, an invalid target, or a closed window cancels cleanly. Failure restores the authoritative view. If the target vanishes, report it and refresh; never redirect the move to an arbitrary workspace.

Window or workspace changes during dragging freeze only the relevant interaction geometry; incoming model updates continue. A move whose acknowledgement is lost is reconciled before retry. Dragging to the existing workspace is a no-op. App groups expose a separately labeled bulk move with per-window results.

### Keyboard, search, and dismissal

| Input | Proposed action |
| --- | --- |
| Global configurable shortcut | Toggle overview; repeating it closes the overview. |
| Tab / Shift + Tab | Move among search, monitor selector, workspace rail, and window grid. |
| Arrow keys | Spatially navigate the focused region; arrows in search retain text-editing behavior. |
| Enter | Activate the selected window or commit the selected workspace. |
| Ctrl + K | Focus search; do not reserve this globally. |
| Ctrl + Shift + M | Open “Move to workspace” for the selected window; searchable destinations and Enter to confirm. |
| Escape | Cancel drag or close a submenu; then clear a search; then dismiss. |

Search initially spans all windows and workspaces, with optional current-workspace and app filters. Results display their workspace and monitor. Selecting a remote result switches and focuses it. Matching and sorting run outside rendering. Composition-aware text input must not activate a result when Enter is committing IME text.

Dismissal without a committed action returns focus to the invoking window if it still exists and its context remains valid. Respect external user-driven workspace changes while Orbit was open. Empty backgrounds can dismiss; the Add workspace tile is a distinct control. Empty, loading, disconnected, unavailable-preview, and no-results states get deliberate copy and keyboard paths.

## 04 · Brand and visual system

The supplied [Orbit Desktop Brand Guide](../Orbit%20Desktop%20Brand%20Guide.png) is the visual source of truth. Preserve its orbital mark, wordmark, tagline, and dark, layered aesthetic. The source is a composite raster guide; production vector logos, transparent app icons, and export sizes still need preparation and visual approval.

| Token | Guide value | Planned use |
| --- | --- | --- |
| Midnight | `#0B1020` | Main canvas, overlay backing |
| Violet | `#7C3AED` | Selected workspace, primary action, restrained glow |
| Coral | `#FF6B6B` | Errors and destructive-action accents |
| Amber | `#F59E0B` | Warnings and pending attention |
| Sky | `#38BDF8` | Keyboard focus and informational accents |
| Slate | `#0F172A` | Cards and raised surfaces |
| Gray | `#334155` | Dividers and neutral controls |
| Silver | `#94A3B8` | Secondary labels on dark surfaces |
| Cloud | `#E2E8F0` | Primary body text on dark surfaces |
| White | `#FFFFFF` | High-emphasis labels and light variants |

Use **Inter** at 13–16 logical pixels for labels and body UI, 20–28 for section titles, and restrained weight changes. Keep icon strokes around 2 logical pixels, with rounded ends. Proposed spacing scale: 4 / 8 / 12 / 16 / 24 / 32; card radii 12–16; search and controls 10–12. These dimensions are design proposals, not measurements from the guide.

Use a search field above a compact workspace rail and a generous preview grid. Translucency and a dimmed wallpaper can give context, but an opaque midnight fallback must look intentional. Keep borders visible without relying on blur. Reserve the logo's multicolor treatment for branding; selection and interaction remain calm and consistent.

Target 160–220 ms entry/exit transitions with positional continuity. Reduced motion removes travel and scale animation. Selection uses both shape and color, text targets at least 4.5:1 contrast, and meaningful non-text boundaries at least 3:1. Test actual composited colors; brand values alone do not guarantee contrast. Offer an opaque/high-contrast presentation and preserve usable font scaling.

**Design deliverables:** reusable GPUI theme tokens; overview, workspace tile, window card, search, move menu, status banner, and shortcut hint components; light/dark and monochrome logo exports; empty/error/loading states; keyboard focus specs; and approved layouts at 1280×720, 1920×1080, ultrawide, and fractional scaling.

## 05 · Feasibility before polish

Three technical spikes determine the implementation boundary. They must use real GPUI surfaces and real Hyprland windows, with a recorded version matrix. Desktop control and window pixels are separate capabilities.

### Gate A — Overlay and focus

Use GPUI's existing Wayland layer-shell support as the starting point. Its upstream example demonstrates a layer-shell window, but does not prove Orbit's fullscreen coverage, input handling, or multi-output lifecycle. [GPUI layer-shell example](https://github.com/zed-industries/zed/blob/main/crates/gpui/examples/layer_shell.rs).

Prove a non-reserving overlay surface on the selected output, stacking above ordinary/fullscreen clients, receiving keyboard input only while visible, and releasing focus on every close/error path. Verify output discovery after Wayland registry initialization, monitor selection, resize, scale changes, and hotplug. Use one surface per participating output if needed, with one active keyboard owner. Never render above the lock screen.

**Exit evidence:** repeated open/close/activate/cancel cycles with owned test windows, including fullscreen and multiple monitors. If the pinned GPUI revision lacks necessary controls, estimate a narrow platform patch and its maintenance cost before continuing. Do not assume a compositor plugin is required.

### Gate B — Real window previews

Probe the running Wayland registry. Prefer `ext-image-copy-capture-v1` with a supported toplevel image-source protocol and foreign-toplevel enumeration. Match capture handles to Hyprland window addresses through the advertised mapping protocol. These protocols exist upstream; runtime support and correct mapping remain to be verified. [Capture protocol](https://github.com/wayland-mirror/wayland-protocols/blob/main/staging/ext-image-copy-capture/ext-image-copy-capture-v1.xml), [Hyprland mapping protocol](https://github.com/hyprwm/hyprland-protocols/blob/main/protocols/hyprland-toplevel-mapping-v1.xml).

Where available, evaluate Hyprland's older toplevel-export protocol as a compatibility path. Prefer its supported handle-based capture; do not truncate a 64-bit window address into an older 32-bit request. [Hyprland toplevel export](https://github.com/hyprwm/hyprland-protocols/blob/main/protocols/hyprland-toplevel-export-v1.xml).

Prove previews for occluded, inactive-workspace, fullscreen, grouped, and XWayland windows. Verify stride, pixel format, alpha, orientation, buffer release, scaling, and capture denial. Begin with shared-memory capture and bounded GPU uploads; pursue DMA-BUF import only after format/modifier and GPUI renderer compatibility are measured. A separate capture connection needs its own handles and event loop; Wayland object IDs cannot be reused across connections.

**Fallback:** use the most recent in-memory image with a stale marker, or an icon/title card with “Preview unavailable.” Never switch through real workspaces to collect screenshots. Build workspace thumbnails from the known layout and available window images; do not present a fabricated composite as a live output capture. Full-screen screenshot cropping cannot recover obscured or inactive windows.

**Exit evidence:** capture-to-card prototype, capability report, resource measurements, and an explicit list of unsupported cases. Useful current-workspace previews are an alpha quality gate; metadata-only mode is a resilience feature, not proof of visual parity.

### Gate C — Accurate control

Use Hyprland's request socket for queries/control and event socket for changes. The documented sockets live under the session runtime directory. [Hyprland IPC](https://wiki.hypr.land/IPC/).

Verify snapshot queries for monitors, workspaces, clients, active workspace/window, and configured workspace rules on the chosen release. Validate address-specific focus and silent movement, normal workspace selection, and special-workspace handling. Dispatcher names such as `focuswindow`, `workspace`, and `movetoworkspacesilent` are documented in a versioned reference; select behavior by the tested installed version. [Hyprland 0.54 dispatcher reference](https://wiki.hypr.land/0.54.0/Configuring/Dispatchers/).

**Exit evidence:** tests moving only an owned test window, preserving unrelated focus, resolving acknowledgement loss, and recovering after compositor restart. A command returning successfully is not enough: verify the resulting compositor state.

## 06 · Architecture and data flow

Use a small Cargo workspace with a framework-free core and explicit platform boundaries. Keep GPUI rendering free of IPC, disk reads, image conversion, and parsing.

```text
orbit (binary / CLI / lifecycle)
  ├── orbit-ui          GPUI views, focus, accessibility, theme
  ├── orbit-core        model, layout, commands, state transitions
  ├── orbit-hyprland    snapshots, events, typed dispatch, reconciliation
  └── orbit-capture     protocol negotiation, buffers, preview cache

User input → Core command → Effect → Platform adapter
Compositor event / snapshot → Core update → GPUI notification
Capture frame → Bounded image cache → Changed card only
```

**Core identities:** compositor-session epoch; window address plus local lifetime generation; workspace numeric/named/special identity; output identity; operation ID. Never use titles, list indices, or process IDs alone as window identity. Keep capture protocol handles separate from compositor IDs. Reject responses from previous sessions, replaced windows, or superseded operations.

**Core state:** monitor topology, authoritative workspace/window map, previewed workspace, selected item, query/filter, drag state, pending operations, and capabilities. Model UI lifecycle as Hidden → Opening → Browsing/Dragging → Activating → Closing, with explicit disconnected and failure paths. Keep one availability function for pointer, menu, and keyboard actions.

**Snapshot/event synchronization:** subscribe to events before taking a snapshot; buffer events, then reconcile affected entities with fresh queries. Since queries are not an atomic transaction and events do not provide a guaranteed global sequence, do not blindly replay stale buffered changes over a newer snapshot. Coalesce bursts, re-query on ambiguity, reconcile when opening, and use a bounded low-frequency health check while visible. Queue overflow forces a fresh snapshot. Reconnection creates a new epoch and invalidates handles and pending work.

**Move operations:** capture the source identity and destination; validate both; send one typed request; mark pending; await observed membership or a bounded re-query. If uncertain, display an unknown/pending result and reconcile before retry. Serialize conflicting operations on the same window. Use validated identifiers and protocol encoding; never interpolate app titles into a shell command. Production control uses direct sockets, with `hyprctl -j` useful for development comparison.

**Capture scheduling:** capture only when visible, prioritizing selected and visible cards. Start with 5–10 preview updates per second and up to 15 for the selected card as tuning targets. Cap in-flight captures and memory; downscale before caching. Budget full-resolution staging buffers separately from thumbnail textures. Pause capture while hidden or locked, release GPU resources after a short grace period, and invalidate sensitive frames on privacy or session changes.

**GPUI discipline:** pin GPUI and its platform crates to the same full Zed revision and pin Rust; inspect that revision's feature flags and patches. Keep tasks and subscriptions alive intentionally; perform background work outside render; apply results on the UI thread; use stable element IDs and bounded view construction. Ship assets through the application asset source. Review upstream dependency patches and licenses before selecting the release revision. [GPUI project](https://gpui.rs/), [GPUI platform source](https://github.com/zed-industries/zed/blob/main/crates/gpui_platform/src/gpui_platform.rs).

**Resident process:** a small same-user daemon owns the model and warms the overview. Proposed commands: `orbit daemon`, `orbit toggle`, `orbit show`, `orbit hide`, `orbit doctor`, and `orbit demo`. A private per-session runtime socket forwards commands to one instance; a direct invocation can start it if needed. Use a private directory/socket and peer validation. Avoid duplicate startup paths and avoid privileges. A Rust capture worker can be split into a helper process only if profiling or fault isolation warrants it.

## 07 · Workspace and monitor semantics

“All workspaces” is a finite catalog: union of live workspaces, explicit persistent/configured destinations, and Orbit's saved destinations, plus an Add action. Hyprland can accept more workspace IDs than can usefully appear in a rail; do not invent an infinite list or hard-code ten workspaces. Unmaterialized destinations are labeled “Empty.” Glob-style configuration rules do not enumerate every possible destination.

Maintain stable labels and display order independent of temporary compositor visibility. Adding a workspace reserves an Orbit destination; materialize it when activated or receiving a window. Retiring a workspace means migrating its windows and removing Orbit's own entry. Existing persistent compositor rules may keep it alive; explain this instead of claiming deletion. A multi-window migration is not atomic: report per-window failures and retain the original entry until resolved.

Group workspaces by output with explicit active indicators. Default to the invocation monitor's current workspace, while making other monitors discoverable. Cross-monitor drag targets include both workspace and monitor. Convert pointer coordinates through logical display space with scale and transform awareness. On unplug, cancel invalid drags, query the compositor's reassignment, and rebuild surfaces. Distinguish “move this window” from “move this workspace to another monitor.”

Treat special workspaces as a separate rail group and retain their names. Pinned windows appear according to actual compositor visibility with a pin badge; do not count duplicated representations as separate windows. Grouped/tabbed and transient windows need dedicated fixtures to determine movable units and focus behavior. Do not silently break a compositor group or detach modal dialogs.

## 08 · Omarchy integration and packaging

Package an Arch-compatible build with a desktop entry, scalable/raster app icons, license notices, a documented dependency list, and shell completions. Start with a reproducible PKGBUILD and a local install test; publish to AUR only after release review. Test clean installation, upgrade, downgrade/config compatibility, and removal.

Use `~/.config/orbit/config.toml` for preferences and an XDG state location for saved labels/order. Store only preferences and necessary state with versioned, atomic writes; compositor state stays live. Screenshots and window titles are not persisted by default. Invalid configuration produces a clear error and safe defaults without destroying the original file.

Onboarding detects the installed Omarchy and Hyprland versions, protocol capabilities, existing keybindings, and supported config syntax. Current skill guidance describes Omarchy Lua configuration, while older installations use different formats; generate integration from the detected installation rather than shipping one assumed syntax. Provide a reviewable change and backup, modify only the user's configuration, and validate reload/config errors after application. Never modify packaged Omarchy defaults.

Offer an opt-in session startup entry or a correctly session-bound user service. A service must receive the right Wayland/Hyprland environment and stop with that graphical session. Avoid a lingering background process connected to an obsolete instance. Support a theme bridge through documented user hooks, with Orbit's brand theme as the default and a high-contrast option.

`orbit doctor` reports versions, output topology, advertised capabilities, selected capture path, configuration validity, and redacted errors. No window titles or thumbnails in logs by default. Demonstration mode is deterministic, runs without desktop control, and carries a visible **Demo / simulated** label.

## 09 · Reliability, privacy, and accessibility

**Privacy:** preview frames stay in memory; capture denial is respected; exclusions can hide chosen apps or titles. Sensitive-window detection is not reliable enough to promise automatic protection. A “Hide all previews” mode uses metadata cards. Clear excluded images immediately, cancel pending captures, and avoid stale textures reappearing after unlock. If screen-lock state cannot be established reliably on a supported configuration, treat that as a capture-policy release blocker.

**Reliability:** a capture failure must not break switching. A control connection loss disables mutations and shows reconnecting state. Compositor restart invalidates every identity. Surfaces disappear on process termination; test that no keyboard capture remains. Fast repeated toggles coalesce into a deterministic final visibility state.

**Accessibility:** every workspace and window is keyboard reachable, labeled, and exposed through GPUI's supported accessibility integration. Announce selected workspace, pending moves, completion, and failures. Show text alternatives for previews, visible focus rings, and full titles on focus. Verify the real AT-SPI tree and an actual screen reader. Test input methods and non-Latin titles in the real app; headless layout/input tests do not prove native behavior.

## 10 · Delivery roadmap

**Estimate:** 16–22 engineering weeks for one experienced Rust/Wayland engineer with part-time design and QA help, assuming Gate A–C succeed. This is a planning range, not a committed date. Protocol work, upstream fixes, and full parity blockers may extend it. Sequence milestones by evidence, and re-estimate after the first two weeks.

| Phase | Effort | Work and dependency | Exit gate |
| --- | --- | --- | --- |
| M0 · Prove the platform | 2–3 weeks | Gates A–C; choose version baseline; inspect real config; approve interaction and brand spec. | Overlay, exact window move, and useful previews work on owned fixtures. Record a go/no-go decision. |
| M1 · Build the foundation | 2 weeks | Core model, fake backend, snapshot reconciliation, CLI lifecycle, theme tokens, test harness. Depends on M0. | Deterministic state tests pass; native daemon toggles safely; no render-path I/O. |
| M2 · Complete the alpha | 3–4 weeks | Workspace rail, window grid, capture scheduler, activate/switch, drag/drop, keyboard move, search. Brand fidelity pass: bring the overview in line with the brand guide concept (layered dark surfaces, orbital mark, typography, card and rail treatment, motion), using approved vector logo and icon exports. Depends on M1. | All four requested workflows pass together on a single monitor with mixed Wayland/XWayland fixtures. Native side-by-side review against the brand guide passes. |
| M3 · Make the beta robust | 3–4 weeks | Multiple monitors, fractional scaling, hotplug, special/pinned/grouped windows, App Exposé, workspace lifecycle, accessibility. | Compatibility matrix passes; reconnect and stale-result faults recover; installation is reproducible. |
| M4 · Close parity gaps | 4–6 weeks | Desktop reveal, fullscreen/tiling, assignments, gestures/corners, external drag-through, parity audit. Depends on M3 and capability spikes. | Every parity row has tested behavior or a documented blocker; no unqualified full-parity claim with gaps. |
| M5 · Release and harden | 2–3 weeks | Performance tuning, soak tests, packaging, onboarding, user guide, release notes and rollback exercise. | Signed-off evidence pack and a tested supported-version matrix. |

**Critical path:** overlay/capture feasibility → reliable identity and state → four core workflows → multi-monitor correctness → parity → release. A degraded capture mode can unblock core development, but cannot bypass the visual-quality gate. Avoid a compositor plugin initially; if exact parity ultimately requires one, make its language boundary, upgrade coupling, and maintenance cost a separate architectural decision.

### First implementation backlog

1. **ORB-001:** Create Cargo workspace, toolchain pin, CI, and dependency/license inventory. Done when a clean build opens a GPUI window on the target host.
2. **ORB-002:** Implement a capability inspector and reproducible owned-window fixtures. Done when snapshots and protocol availability are recorded without leaking titles.
3. **ORB-003:** Implement the layer-shell lifecycle spike. Done when fullscreen stacking, Escape, toggle spam, output selection, and focus restoration pass.
4. **ORB-004:** Render a real captured test-window image in GPUI. Done when occlusion/inactive-workspace limitations and buffer costs are measured.
5. **ORB-005:** Implement pure state and reconciliation with fake events. Done when stale IDs, restart, event gaps/overflow, and pending moves pass deterministic tests.
6. **ORB-006:** Deliver the vertical slice: open → inspect → move → select → close. Done when the exact target window lands on the selected workspace and gains focus when requested.
7. **ORB-007:** Apply the brand system, accessibility labels, search, and equivalent keyboard workflow. Done when native visual and keyboard review pass.
8. **ORB-008:** Brand fidelity pass (M2). Prepare approved vector logo, app icons and export sizes from the guide; install the brand typeface; match the guide's surfaces, card, rail, search and motion treatment; keep ORB-007's contrast and keyboard guarantees. Done when a native side-by-side review against the brand guide passes.
9. **ORB-010:** Settings and engagement (M2, before install, since install ships the generated bind and autostart files). The Orbit mark in the overlay as the settings entry; a settings window and `orbit config` over `$XDG_CONFIG_HOME/orbit/config.toml` (hot-reloaded) covering the engage shortcut (a generated Hyprland snippet plus a live bind over IPC, with conflict reporting and an explicit, confirmed "add it for me"), a hot corner (Orbit's own corner surfaces with dwell), appearance, layout, behaviour, previews and privacy, icons and launch at login. Done when the shortcut and hot corner engage Orbit on a clean profile without hand-editing Hyprland's config, every setting round-trips through the file, and the checks cover them.
10. **ORB-009:** Produce installable alpha and evidence report. Done when a clean test profile can install, launch, configure, and uninstall without altering unrelated settings.

## 11 · Validation and release criteria

The following are **proposed targets**, not benchmark results. Establish hardware and load baselines in M0: release build, recorded CPU/GPU/driver, 1920×1080 at 60 Hz, 10 workspaces and 30 mixed windows, plus a two-monitor mixed-DPI case. Stress with 100 windows and 30 destinations. Record sample counts, percentiles, failures, and memory categories.

| Measure | Initial budget | Method |
| --- | --- | --- |
| Warm open to useful overview | p95 ≤ 150 ms | At least 100 resident-process activations; useful means navigable cards, without waiting for every new preview. |
| Cold process start to useful overview | p95 ≤ 700 ms | 30 fresh-process launches; distinguish warm OS caches from true cold boot. |
| Input to frame submission | p95 ≤ 16.7 ms at 60 Hz | Instrument input and renderer submission under drag/navigation load; do not label this display latency. |
| Move/switch confirmed | p95 ≤ 250 ms | 100 operations measured to observed compositor state; report timeouts separately. |
| Capture budget | ≤ 128 MiB thumbnail cache | Track cache plus full-resolution staging and GPU allocation separately; enforce backpressure. |
| Resident CPU while hidden | < 0.5% of one core averaged over 60 s | No capture or animation timers while hidden; event subscription may remain. |
| Process memory | Hidden RSS ≤ 100 MiB; visible RSS ≤ 300 MiB | Measure release build after warm-up; include a separate GPU-memory report and revise only with evidence. |

**Automated checks:** core state/property tests; recorded IPC fixtures and malformed-message handling; deterministic capture-buffer tests; GPUI input/focus tests; integration tests in an isolated compositor session. Run `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace` on the release tree. Keep expected protocol behavior versioned.

**Native matrix:** Intel/AMD and NVIDIA where available; native Wayland and XWayland; tiled/floating/fullscreen/transient/grouped/pinned windows; numeric/named/special/empty workspaces; one/two/three outputs; 100%, 125%, 150%, and 200% scaling; portrait/rotated output; keyboard and touchpad; reduced motion; screen reader; IME; suspend/resume; lock/unlock; compositor restart; monitor unplug during a drag.

**Fault scenarios:** window closes while selected or moving; target disappears; reused window address; timeout after accepted move; disconnected sockets; malformed event; capture failure; bounded-queue overflow; many rapid open/close requests; corrupt config; process crash mid-desktop-reveal. Never test by moving or closing unrelated user windows: use owned fixtures in a nested or dedicated session.

**Usability gate:** at least five Omarchy users complete “find a hidden window,” “move a window to another workspace,” and “switch and return” without coaching. Target at least 90% task completion across attempts, zero unintended moves, and no unrecoverable focus loss. Record observations before changing interaction defaults.

**Release gate:** all four core workflows pass; parity claims match the audit; no open critical focus, privacy, or window-movement defect; resource targets are met or explicitly revised; clean install/upgrade/uninstall passes; supported versions and degraded behavior are documented. Report each check as measured, observed, failed, or unverified. An untested GPU or compositor version is not supported by assumption.

## 12 · Decisions, risks, and source register

| Risk or decision | Default / mitigation | Revisit point |
| --- | --- | --- |
| Inactive-window previews unavailable | Keep metadata and stale-frame fallbacks; capture spike must establish usable coverage. | M0 go/no-go; before visual-parity claims. |
| GPUI API churn or missing platform controls | Pin a full revision; isolate patches; review upgrades against native tests. | M0 and each dependency update. |
| Gesture or external drag-through limitations | Delegate supported entry points to compositor integration; record unsupported paths explicitly. | M4 capability decision. |
| Show desktop cannot safely restore layout | Prototype reversible compositor behavior with interruption recovery; keep unavailable until proven. | Before enabling desktop reveal. |
| Numeric workspace ordering conflicts | Orbit ordering changes the rail only; compositor renumbering is a distinct opt-in operation. | Workspace management review. |
| Protected/private content capture | Respect denial, explicit app exclusions, metadata-only mode, lock gating, memory-only frames. | M0 capture review and release. |
| Brand assets are a raster composition | Preserve source guide; prepare approved production assets without substituting a different identity. | Before packaging. |
| Release scope and license | Recommend an open-source project; select the project license and verify all dependency/asset licenses. | Before public alpha. |

**Implementation defaults:** resident daemon, Hyprland-first backend, no compositor plugin initially, per-window moves, all-workspace search, invocation-monitor overlay, and opt-in configuration integration. Exact minimum Omarchy/Hyprland versions, GPUI commit, final shortcut, distribution channel, and production logo exports are decisions for M0, not facts invented by this plan.

**Research baseline:** sources consulted October 18, 2026. Live upstream branches can change; record commit hashes and installed versions during implementation. This document proposes architecture and verification work; it does not claim a working Orbit implementation or measured performance.

- **Product reference:** [Apple Mission Control](https://support.apple.com/en-sg/guide/mac-help/mh35798/27/mac/27) and [Spaces](https://support.apple.com/guide/mac-help/work-in-multiple-spaces-mh14112/mac).
- **Control:** [Hyprland IPC](https://wiki.hypr.land/IPC/) and [versioned dispatchers](https://wiki.hypr.land/0.54.0/Configuring/Dispatchers/). The latter is a reference snapshot, not a declared minimum supported version.
- **Rendering:** [GPUI](https://gpui.rs/), [layer-shell example](https://github.com/zed-industries/zed/blob/main/crates/gpui/examples/layer_shell.rs), and [platform entry point](https://github.com/zed-industries/zed/blob/main/crates/gpui_platform/src/gpui_platform.rs).
- **Capture:** [image-copy-capture protocol](https://github.com/wayland-mirror/wayland-protocols/blob/main/staging/ext-image-copy-capture/ext-image-copy-capture-v1.xml), [toplevel mapping](https://github.com/hyprwm/hyprland-protocols/blob/main/protocols/hyprland-toplevel-mapping-v1.xml), and [Hyprland toplevel export](https://github.com/hyprwm/hyprland-protocols/blob/main/protocols/hyprland-toplevel-export-v1.xml).
- **Brand:** supplied [Orbit Desktop Brand Guide](../Orbit%20Desktop%20Brand%20Guide.png); [Inter font project](https://rsms.me/inter/).
- **Local engineering guidance:** GPUI desktop app skill and Omarchy/Hyprland skill guidance supplied in this workspace session. Verify actual installed behavior before generating configuration.
