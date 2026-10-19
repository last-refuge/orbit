# ORB-004 capture report (Gate B)

Host: djinn, Omarchy 4.0.5, Hyprland 0.56.2, Intel Arc (xe), eDP-1 2880x1800 at
1.25. Run: `scripts/capture-check.sh`, 2026-10-18. Evidence:
`docs/evidence/orb-004/djinn/` (results.txt, per-scenario report.json and
fixture PNGs, preview/preview.png). Only Orbit's own windows were captured.

## Path

`ext-image-copy-capture-v1` + `ext-foreign-toplevel-image-capture-source-manager-v1`
+ `ext-foreign-toplevel-list-v1`, wl_shm buffers, on a dedicated Wayland
connection. `hyprland-toplevel-mapping-v1` resolved every toplevel to its
Hyprland address. Hyprland offers shm Argb8888 and Xrgb8888 plus two DMA-BUF
formats; Orbit uses Argb8888, transform Normal, stride = width x 4.

## Results (all scenarios pass, verdict ok on every window)

| Case | Result |
| --- | --- |
| Inactive workspace (91-93) | Correct pixels |
| Hidden special workspace | Correct pixels |
| Fullscreen (2880x1800) | Correct pixels |
| Occluded (tiled-a under tiled-b, both visible) | Own pixels only, no bleed from the covering window (other-accent fraction 0.0) |
| Same window across scenarios | Byte-identical PNGs: capture is the window's buffer, not the screen |
| GPUI render | `orbit preview` shows all five fixtures as live cards |

Fresh session per capture (what opening the overview does):

| Window | Setup p50 | Capture p50 / p95 | Convert p50 | Buffer |
| --- | --- | --- | --- | --- |
| 600x400 | 0.2-0.6 ms | 6-8 / 7-12 ms | 0.3-1.8 ms | 0.92 MiB |
| 2880x1800 fullscreen | 2.5-2.8 ms | 23.5-24.2 / 25-30 ms | 13-18 ms | 19.8 MiB |
| 1375x900 preview | 0.6 ms | 6.2 / 9.6 ms | 1.7 ms | 4.7 MiB |

Memory: capture process RSS 6 MiB to 28 MiB during a full pass (dominated by
the fullscreen buffer plus its converted copy). Preview process 128-168 MiB
RSS visible, 4.6 MiB of thumbnail textures for five cards (thumbnails capped at
640x400 and freed from GPUI's atlas when replaced).

## Findings that shape the design

- Repeat frames on one session are damage-driven: Hyprland answers the next
  frame only after new damage, and whether a static window gets one within
  250 ms varied run to run. Orbit treats a late repeat as "unchanged" and
  keeps the last image; a fresh session always returns a first frame in
  about 7 ms (25 ms fullscreen).
- Full-output windows are the cost driver: 24 ms capture + 15 ms convert at
  2880x1800. Overview open must show metadata cards first and fill previews
  asynchronously, largest windows last, with conversion/downscale off the UI
  thread (as `preview` does). Per-window capture fits the 150 ms warm-open
  budget only when not on the critical path.
- A window can close between listing and capture. Sending a destroyed handle
  is fatal in wayland-client, so `open` checks liveness first (regression test
  in `tests/fake_compositor.rs`).
- Jason's config floats every window, so "tiled" fixtures are floating; the
  occlusion case is two overlapping floating windows.

## Unsupported or untested

- XWayland windows: untested (xterm not installed on djinn).
- Grouped windows: untested (no group fixture yet).
- DMA-BUF capture: offered (2 formats), not attempted; deferred until format,
  modifier and GPUI import are measured.
- `hyprland-toplevel-export-v1` compatibility path: not implemented; the
  preferred path is present on the target host.
- Capture denial and window-closed-during-capture: covered against the fake
  compositor only; not provoked on Hyprland.
- Multi-monitor, other scales (1.0/1.5/2.0), rotated outputs, non-Normal
  transforms: untested on hardware.
