# Workspace swap validation

## Animation preview — alpha.8

The release build was exercised in a private headless sway with fake Hyprland
IPC, in both normal and reduced-motion modes. The native GPUI checks cover:

- Bumping a target into the source slot while keeping the pointer still; the
  target remains stable as the thumbnail moves.
- Moving back to the source, leaving the rail, and rapidly changing direction;
  all displaced thumbnails return to their original slots.
- No compositor mutation during any of those previews.
- Drop, swap back, keyboard parity, Escape/outside-drop cancellation, edge
  scrolling, restart, and the individual-window drag path.

Screenshots of outbound, settled, and returned thumbnails were inspected for
placement and clipping. Reduced-motion mode reaches the same preview positions
without vertical motion. Both modes run in CI via
`scripts/workspace-reorder-check.sh`.

This validates the native rendering and input path against fixture data. The
compositor swap implementation is unchanged from alpha.7; its real Hyprland
verification is recorded below.

## Compositor swaps — alpha.7

Verified on Hyprland 0.56.2 with the release binary and owned fixture windows
inside a private compositor. No user desktop windows were used.

- Native Orbit drag: workspace 91 (three dwindle tiles) ↔ 92 (floating window).
- Native Orbit drag: workspace 91 ↔ 93 (fullscreen window).
- Both swaps reversed successfully, with exact equality of window positions,
  sizes, floating/fullscreen and client fullscreen state, monitor, groups, and
  pinned state against the original compositor snapshots.
- Five additional round trips with workspace 91 active preserved geometry and
  kept the active workspace attached to its contents as its number changed.
- Private sway plus fake IPC: drag/drop, keyboard parity, Escape/outside-drop
  cancellation, edge scrolling, restart, and individual-window drag regression.
- Production Lua with fault injection: refusal at each of the three ID changes,
  rollback, occupied temporary ID, stale membership, and absent API support.
- 139 Rust tests pass; formatting, Clippy with warnings denied, and dependency
  inventory checks pass.

The real compositor test used an isolated Weston parent and a headless Hyprland
output. Aquamarine's unconditional parent protocol v6 binds required a temporary
v5 negotiation shim for that test parent. The shim does not touch Hyprland's
workspace implementation or Orbit and is not part of the product. Native tests
are in `scripts/workspace-swap-native-check.sh`; fault injection is in
`scripts/workspace-swap-lua-check.lua`.

Scope: numbered workspaces with `hl.dsp.workspace.change_id`. Named and special
workspaces cannot use that primitive and are excluded. Workspace-specific rules
still apply to the new numbers; arbitrary user rule combinations, alternate
tilers, XWayland, and multi-monitor geometry have not been validated here.
