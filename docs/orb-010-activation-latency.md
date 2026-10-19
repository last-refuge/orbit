# Activation latency (ORB-010)

"Bring this window forward" is a click or Enter on a card. This is where the
time went before and after the 2026-10-18 change, and how to read the live
numbers.

## Live measurement

The daemon stamps every activation; after a click, on the device:

    target/release/orbit state | jq .last_activation

| Field | Meaning (ms after the click) |
| --- | --- |
| `focus_sent_ms` | first `hl.dsp.focus` + `alter_zorder top` written to Hyprland |
| `closed_ms` | the overlay surface removed |
| `layer_gone_ms` | Hyprland reported `closelayer>>orbit` |
| `resent_ms` | focus + raise sent again after the layer was gone (the hand-back case) |
| `active_ms` | Hyprland first reported the window active (what the eye sees) |
| `settled_ms` | active for 300 ms; one last raise sent |
| `sends` | how many focus + raise pairs went out |

`review-check.sh` asserts `focus_sent_ms <= 20` and that `active_ms` exists.

## Before (code-derived budget, Orbit-attributable)

| Step | Cost |
| --- | --- |
| Click → Activate intent → focus goal | < 1 ms |
| Fly-out animation before the surface was removed (`FLY_MS`) | 220 ms, always |
| Wait for `closelayer>>orbit` before the first focus (fallback 300 ms, checked on the 100 ms tick) | 5–400 ms |
| Focus dispatch (new socket per request, Hyprland protocol) | ~0.1 ms against the fake; ~1 ms on Hyprland |
| Confirmation only on the 100 ms tick | 0–100 ms |
| Retry interval if not observed | 200 ms |
| **Typical click → window active** | **≈ 330–520 ms** |

Plus Hyprland's own workspace-switch animation when the window lives on
another workspace (`animations:enabled = true` on djinn): not Orbit's.

## After

| Step | Cost |
| --- | --- |
| Click → focus + raise written to Hyprland, before the surface is removed | < 2 ms (`focus_sent_ms`) |
| Surface removed at once on activation (no fly-out; Escape keeps it) | same frame |
| `closelayer` → immediate re-send of focus + raise (no interval) | event latency, ≈ 1 frame |
| Confirmation on the `activewindowv2` event itself | event latency |
| Fallback if no `closelayer` arrives | 200 ms (was 300); retry 120 ms (was 200) |
| **Orbit-attributable click → window active** | **≈ 1 frame; `active_ms` on djinn is the number to watch** |

Two IPC writes per send (focus, then raise) stay separate: Hyprland's
request socket is one request per connection, and a `dispatch` takes one
dispatcher call, so there is no persistent connection or combined write to
be had; each costs ~0.1 ms against the fake (`median 0.07 ms`, n = 50).

What remains outside Orbit: Hyprland's workspace-switch animation and its
own focus hand-back on layer close (countered by the re-send).
