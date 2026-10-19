# ORB-008 design targets (round 2)

The measurable targets the overview is built to, taken from
`docs/orb-008-design-review-findings.md` (the round-1 review against the
brand guide), with how each is checked. Logical px unless noted; "device"
means output pixels (logical × scale). Jason's monitor: 2304x1440 logical
at 1.25x (2880x1800).

## Round 3 (current): full-screen glass at the hero's proportions

Jason's verdict on round 2 ("cute, not nearly as polished and beautiful"
as the concept; Mission Control is the bar) replaced the bounded panel
with the hero's composition on a full-screen frosted surface. These are
the targets now in force; the round-2 tables below are kept for history.

| Target | Value | Checked by |
| --- | --- | --- |
| Surface | the whole output; no panel element, no wallpaper margin | `review-check.sh` "layout: the surface is full screen with no panel element" |
| Material | Orbit's own frost: one output capture per open, point-sampled to 1/6 (480x300 here) and box-blurred 3 passes radius 7 (≈ 42 device px) in ≈ 4 ms, drawn edge to edge under the glass tint (scrim Midnight 40 % then Slate→Midnight 60 %), lit top edge (white 10 % over 2 px); solid Slate when capture is unavailable or restricted | `orbit::backdrop::tests`; `orbit state .backdrop`; `review-check.sh` "privacy: no desktop backdrop is captured in test-hooks mode" |
| Search bar | 56 % of the width capped at 1280, 56 tall, radius 18, 17 px text, 40 from the top, centered to the device pixel | `review-check.sh` "layout: the search bar is centered at 56 tall"; metrics tests |
| Rail | 176 x 112 tiles, gap 32, number under each (15 px), caption only on the focused tile; as many whole tiles as fit (10 here), centered; scrolls to keep the previewed/focused tile in view; tiles composite the backdrop and the workspace's windows from real geometry, filled with cached previews on the previewed workspace, outlines elsewhere | `review-check.sh` "layout: rail tiles are at least 160 wide…", "rail: only the previewed workspace's windows are captured"; `rail_keeps_the_shown_tile_in_the_strip` |
| Grid | fixed 3 columns (2 for one or two windows); card = (inner − gaps)/columns floored to a multiple of 4, clamped 320–760 (684 here, 760 for two); slot 16:10 (428); gaps 28/32; rows fill the height from the top, centered while they fit, scrolling past it; partial last row centered | `review-check.sh` "layout: cards fill the width…", "layout: the grid rows fit the grid area"; metrics tests |
| Previews | captured once to the slot minus ring and edge (849 x 529 device here), drawn at exactly that size, real aspect (within 2 %), radius 16, one-device-pixel edge, shadow 0/12/40 at 45 %, no frame fill | `review-check.sh` "previews: …" (four checks) |
| Labels | 36 px icon + 18 px SemiBold name under the image's left edge; title on the selected card only; nothing else on the surface (no badges, place label or hints; hints behind Ctrl+/ or F1; only failures and the link toast) | `review-check.sh` "layout: hints are hidden until asked for", "hints: Ctrl+/ shows the key hints" |
| Selection | one Violet ring (2 device px) with the glow when keyboard focus is in the grid; Sky ring on the focused tile | theme test; captures |
| Motion | previews fly from their real window rectangles into the grid over 220 ms ease-out on open and back on close; the glass fades with them; the layer closes after the exit flight; `--reduced-motion` fades only | manual; `slice-check.sh` still passes (the hand-off waits for the layer either way) |
| Device alignment | every size a multiple of 4 logical px; search, rail, grid and image origins on whole device pixels | metrics tests; `review-check.sh` "…whole device pixels" |

Side-by-side evidence: `docs/evidence/orb-008/djinn/round3/side-by-side-hero.png`
(the hero over Orbit's five-card search view at the same width; test mode,
so the glass is the solid fallback and the previews are fixture windows).

## ORB-010 additions

| Target | Value | Checked by |
| --- | --- | --- |
| Mark | 48 px brand symbol (`assets/brand/orbit-symbol.png`) with the brand wordmark cut from the lockup, 26 px tall, tinted to the theme's text colour, 14 px gap; at the grid's left margin, centred on the search row (top 44); soft shadow, lifts on hover; tooltip "Settings · Ctrl+," on hover; click or Ctrl+, opens settings | `drive layout .mark` (`{x, y, size, wordmark}`); `review-check.sh` "layout settings: …mark.size >= 40", "settings window: opens on request" |
| Drag ghost | the dragged card's capture at its real aspect, 264 wide, radius 12, selection border, soft shadow, centred under the pointer; the app-name chip only while no frame has arrived | manual review |
| Activation | click or Enter: focus is sent once Orbit's layer is reported closed, each focus is followed by `alter_zorder top` on the same window, and one more raise once the focus has settled (all windows float on djinn, so focus alone need not bring the window forward) | `orbit_hyprland::control::tests`; `review-check.sh --fake` "pointer: …raised to the top", "…raise follows the focus" |
| Settings file | `$XDG_CONFIG_HOME/orbit/config.toml`, every key with a default, atomic writes, unknown keys kept, hot-reload ≤ 0.5 s | `orbit_core::settings::tests`; `review-check.sh` "config: …", "hot corner: the daemon picks the change up" |
| Shortcut | live `hl.bind` over IPC, released on quit; snippet `hypr/orbit.lua`; conflicts reported and not bound; `require("hypr.orbit")` appended only after confirmation | `orbit::engage::tests`; `review-check.sh` "shortcut: …" |
| Hot corner | 2x2 logical px overlay surface at the true corner (exclusive zone −1), dwell 50–2000 ms, no engage while a button is held | `review-check.sh` "hot corner: …" (surface at 0,0; `drive hot-corner` opens the overview) |
| Layout settings | columns 2/3/4 and rail bottom apply live | `review-check.sh` "layout settings: …"; metrics tests |

## Round 2 targets (superseded)

## Composition (P0-1)

| Target | Value | Checked by |
| --- | --- | --- |
| Panel is bounded and centered, wallpaper visible around it | width ≤ 78 % of the output, left and right margins equal to the device pixel | `review-check.sh` "layout: panel …" via `orbit drive layout` |
| Panel width at 2304 wide | 1696 (3 × 520 + 2 × 28 + 2 × 40) | `orbit_ui::overview::tests::metrics_follow_the_design_targets` |
| Panel height | content-sized (search + rail + N grid rows), clamped to 82 % of the output height (1180 here); the grid scrolls past that | same test; `review-check.sh` "layout: panel height" |
| Panel radius, padding, row gap | 44, 40, 24 | source constants |
| Nothing inside the panel but search, rail and grid | no header, wordmark, place label or footer; the status line and key hints are a 36 px bar under the panel | `review-check.sh` a11y roles (one Status region) and manual review |

## Material (P0-2)

Two translucent presentations, chosen at daemon start:

| Theme | When | Scrim (Midnight) | Panel (Slate→Midnight) | Bars (Midnight) | Hairline |
| --- | --- | --- | --- | --- | --- |
| `standard` | Hyprland blur off (default when `decoration:blur:enabled` is false), or `--no-glass` | 80 % | 90 % | 85 % | white 14 % |
| `glass` | Hyprland blur on, or `--glass` | 40 % | 60 % | 88 % | white 14 % |
| `standard` (opaque) | `--opaque` | 100 % | 100 % | 100 % | white 14 % |
| `high-contrast` | `--high-contrast` | 100 % | 100 % | 100 % | white 50 % |

Rules both translucent themes meet, checked by
`orbit_ui::theme::tests::text_and_state_boundaries_meet_their_ratios`
against the panel composited over a black and over a white desktop:

- Body text (Cloud) and names (White) on the bare panel: ≥ 4.5:1.
- Secondary text (Silver), the caret, badges and status messages sit only
  on the dark bars (search field, status line, badge chips): ≥ 4.5:1 there.
- Keyboard focus (Sky) ring on the panel: ≥ 3:1.
- Selection (Violet) ring on the panel: ≥ 3:1, or, where it is not (glass
  over a bright desktop), the ring's one-device-pixel white inner edge
  (70 %) is ≥ 3:1, so the selected card keeps a visible contour anywhere.

The standard theme is the most translucent set that keeps even Silver and
Violet at their ratios on the bare panel over white, so it stays correct
without blur. The glass theme assumes the compositor blurs the backdrop
(the hero's frosted look); blur does not change luminance, which is why
the bars and the ring edge carry the contrast.

Hyprland side (not written by Orbit; Jason's `decoration.blur.enabled` is
**false**, Omarchy's default, so blur must be enabled globally for the
layer rule to do anything):

```lua
-- Hyprland 0.56 Lua config (~/.config/hypr/hyprland.lua or a required
-- module). Blur must be enabled globally for layer blur to work.
hl.config({ decoration = { blur = { enabled = true, size = 10, passes = 3,
  ignore_opacity = true, noise = 0.0117, contrast = 0.89, vibrancy = 0.17 } } })
hl.layer_rule({ match = { namespace = "^orbit$" }, blur = true, ignore_alpha = 0.2 })
```

`ignore_alpha = 0.2` leaves pixels under 20 % alpha unblurred, so the
scrim margin stays a plain dim and the panel gets the frost. Field names
are from `/usr/share/hypr/stubs/hl.meta.lua` (`HL.LayerRuleSpec`). With
these in place the daemon picks `glass` by itself (it asks
`getoption decoration:blur:enabled` once at start); `orbit state` reports
`look.glass` and the alphas in use.

Other material values: panel shadow 0/24/64 at 45 %; top-edge highlight
white 8 % over 2 px; card shadow 0/8/28 at 35 %; glows 8 px at 35 %.

## Grid and cards (P0-3, P0-5)

| Target | Value | Checked by |
| --- | --- | --- |
| Columns | 3 (2 when three 420 px cards do not fit in 76 % of the output) | `drive layout` `.columns`; metrics tests at 2304, 1920, 1366, 3440 |
| Card width | (76 % of output − 80 − 2 × 28) / 3, floored to a multiple of 4, clamped to 420–520: 520 here, 440 at 1920 | `review-check.sh` "layout: cards"; metrics tests |
| Gaps | 28 between columns, 32 between rows | source constants |
| Preview slot | card × 16:10 (520 × 324); the image keeps the window's real aspect inside it, centered on whole device pixels; never scaled to fill | `drive layout` previews `shown` vs `image` |
| Preview material | bare image, radius 16, one-device-pixel hairline, shadow 0/8/28 at 35 %; no frame fill (a Midnight 60 % placeholder only while no frame has arrived) | manual review |
| Single resample | captured once, area-averaged, to the device box (644 × 399 here: the slot minus ring and hairline), drawn at exactly that size; never upscaled | `review-check.sh` "previews: drawn at the captured device size"; `orbit::preview::tests::fits_inside_the_box_without_growing` |
| Device alignment | card, slot, panel, rail and row sizes are multiples of 4 logical px; the panel origin and each preview's origin are whole device pixels | metrics test; `review-check.sh` "layout: … whole device pixels" |
| Label | icon 36 + app name 18 px SemiBold White; a 13 px Cloud second line only for the selected card's title or a search result's workspace; badges are small chips in the preview's corner (and in the accessible label) | manual review |

## App icons (P0-4)

The window class (Wayland app id or X11 class) is matched to a desktop
entry in the XDG data directories (`$XDG_DATA_HOME`, `$XDG_DATA_DIRS`,
Flatpak exports) by file id, `StartupWMClass`, reverse-DNS last segment,
then `Name`, case-insensitively; its `Icon` is looked up at 48 px through
the icon theme (`$ORBIT_ICON_THEME`, else GTK's `settings.ini`, else the
GNOME `icon-theme` setting Omarchy writes, else `hicolor`), its parents,
`hicolor` and `pixmaps`, preferring PNG or SVG. Results are cached per
class. A class with no icon draws a Gray tile with a Cloud initial.
Checked by `orbit_ui::icons::tests` (temporary data dirs) and
`review-check.sh` "icons: …" (`look.icon_theme`, per-card `icon`;
fixtures have no desktop entry, so they report the fallback).

## Selection and focus (P0-6)

One ring: 2 device px. Cards: Violet for the selected card, and the 8 px
glow at 35 % when keyboard focus is in the grid (so on open the card
reads as the guide's violet selection; the glow says the keyboard is
there). Tiles: Violet for the previewed workspace, Sky when keyboard
focus is on a tile, glow in either case. The ring's inner edge is the
preview's hairline (white 70 % in glass). No second ring. Hover brightens
the edge to white 20 %. Checked by the theme test (ratios) and the round-2
captures.

## Rail (P1)

Tiles 176 × 112 (the guide's 176 × 110, made integral at 1.25x), radius 10,
gap 32, one 15 px label line (SemiBold White when active or previewed,
Medium Cloud otherwise; the active one keeps a Sky dot), and a 16 px
caption row that shows counts and state only for the keyboard-focused
tile. The "New" tile is a plain outlined "+".

## Privacy

With `orbit daemon --test-hooks` (every scripted run) only windows whose
class starts with `dev.orbit.fixture.` or equals `OrbitFixtureX11` are
captured, for cards and for rail covers alike; `orbit state` lists what is
being captured (`capturing[].class`) and `capture_restricted`.
`review-check.sh` fails if anything else appears there. Without
`--test-hooks` the overview captures the cover window of every workspace
it shows in the rail (the user's own session); README says so.

## Not yet (P1/P2 left)

Search bar is at the target proportions (1228 wide, 56 tall, radius 18,
17 px); type scale is 18/17/15/13. Still open: the entry motion as a
scale-in, animated rail border changes, limiting cover capture to the
previewed workspace and its neighbours outside test runs, and the mark
redraw (the header is gone, so the draft mark no longer appears).
