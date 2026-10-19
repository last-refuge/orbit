# ORB-008 design review: Orbit round 1 versus the brand guide concept

Reviewed 2026-10-18 on `djinn` (eDP-1, 2880x1800 at scale 1.25 = 2304x1440
logical; Hyprland 0.56.2, Omarchy). Source of truth: `Orbit Desktop Brand
Guide.png` (1225x1284), mainly its hero mock-up, UI ELEMENTS and VISUAL
STYLE panels. Orbit was measured from the source
(`crates/orbit-ui/src/overview.rs`, `theme.rs`, `crates/orbit/src/preview.rs`),
from `orbit state`, and from a capture of Orbit's own layer open over the
fixture workspaces 91 and 93 (`docs/evidence/orb-008/djinn/`).

Concept measurements are estimates read off the raster guide (the hero is
drawn in perspective), given as ratios so they transfer to any screen.
Orbit numbers are exact unless marked "measured" (read from the capture's
pixels).

Verdict up front: round 1 implemented the guide's **tokens** (colors, Inter,
radii in the proposed 12–16 range) but not its **composition or material**.
The capture reads as a dark, flat, full-screen sheet with a couple of small
framed boxes in a sea of empty space, plus a header, a footer and badges the
concept does not have. The concept is a frosted-glass object, packed with a
fixed 3-column grid of large, bare window images with an icon and name
under each. Those two things do not look like each other, and Jason's
verdict ("a low quality version of the concept") is fair.

## Evidence and capture notes

- `overview-ws91-masked.png` and `overview-ws93-masked.png`: Orbit's layer at
  device pixels over workspaces 91 (two fixture windows) and 93 (the
  fullscreen fixture). Two bands are blacked out: the Omarchy bar (dimmed
  through the scrim, not Orbit's pixels) and the **workspace rail**, because
  the rail's tiles capture the cover window of every workspace, including
  Jason's real ones. The unmasked captures were deleted. This is itself a
  finding (see "Rail", F6): the overview captures real windows on every
  workspace as soon as it opens, which ORB-004's README statement ("only
  Orbit's own windows are ever captured") no longer describes.
- `card-ws91-closeup.png`: the selected fixture card at device pixels, for
  judging preview sharpness and the selection/focus treatment.
- No `orbit drive` runs: a daemon Jason started himself (`target/release/orbit
  daemon`, no `--test-hooks`) was already serving the socket, so I used it
  for `show`/`hide`/`state` and left it running. Focus and hover states are
  judged from the source.
- Dispatches used: `hl.dsp.focus({ workspace = '91' | '93' | '3' })` only
  (switch to a fixture workspace, capture, switch back to workspace 3).

## 1. Overall composition and panel proportion

**Concept.** One rounded glass panel floating over the wallpaper, with
wallpaper visible around it (VISUAL STYLE shows the panel at roughly
60 % x 55 % of the frame, centered; the hero shows it filling most of the
frame but still with sky and mountains at the edges). Inside, top to
bottom: search bar, workspace rail, window grid. The grid is the panel:
about 60 % of the panel's height, 3 columns x 2 rows of large cards. No
wordmark, no heading, no footer, no status text inside the panel. Panel
corner radius about 3 % of its width (about 50 px for a 1760-wide panel).

**Orbit.** The panel is the whole output minus a 40 px inset: 2224 x 1360
logical (96.5 % x 94.4 % of the screen), radius 28 (1.3 % of width), 32 px
padding, 22 px between rows. Content rows: header (36 px mark, 26 px
wordmark, "Workspace 91 · eDP-1" at right), 52 px search bar, the rail,
the grid (`flex_1`, so it takes all remaining height: about 1000 px), then
a hairline-topped footer with keycap hints. With two windows the grid holds
two 360 px cards centered in a 2160 x 1000 px void. Measured on the capture:
the two cards cover 4.8 % of the panel's area.

**Gap.** Wrong object. The concept is a bounded panel whose grid fills it;
Orbit is a full-screen sheet whose grid floats in it. The extra chrome
(header, place label, footer hints, status line) is not in the concept and
pushes the panel further from it.

**Fix.** Bound the panel: max width about 1760 and max height about 1180
logical at this screen (76 % x 82 %), centered, radius 40–48, with the
wallpaper visible around it. Size the panel to its content (search + rail
+ N rows of the fixed grid, clamped to the max), so one or two windows give
a shorter panel rather than a void. Drop the header and footer rows from the
panel (keep the hints and status as a slim line *under* the panel or on
demand; keep the wordmark out of the overview, as the guide does).

## 2. Backdrop translucency and blur

**Concept.** Frosted glass: the wallpaper's mountains and sunset glow show
through the panel, blurred (blur radius visibly large, 30–60 px), with a
dark tint. Reading the hero: the panel's fill is roughly Midnight at
55–65 % over the blurred backdrop; VISUAL STYLE's lighter variant is a
white tint of about 10–15 % over blur. Both have a 1 px lighter edge.

**Orbit.** Scrim: Midnight at 80 % (135-degree gradient to a Violet cast)
over the whole output. Panel: Slate-to-Midnight gradient at 90 % over that.
The desktop therefore shows through the panel at 2 % and through the margin
at 20 %. No blur anywhere, and `decoration.blur.enabled` is **false** in
Jason's Hyprland (Omarchy's default), so a layer blur rule would do nothing
until that is turned on. Measured: panel body composites to about
#0C1122 with a pixel standard deviation of 0.8 (flat); the 40 px margin has
a standard deviation of 7.7 (wallpaper faintly visible). Bright content
behind the panel ghosts through it: the fixture window's large text was
faintly readable through the "glass" in the capture. That is the worst of
both: not translucent enough to read as glass, not opaque enough to be
clean.

**Fix (P0).** Use Hyprland's layer blur for the frosting and retune the
layers for a blurred backdrop:

```lua
-- Hyprland 0.56 Lua config (Jason's ~/.config/hypr/hyprland.lua or a
-- required module). Blur must be enabled globally for layer blur to work.
hl.config({ decoration = { blur = { enabled = true, size = 10, passes = 3,
  ignore_opacity = true, noise = 0.0117, contrast = 0.89, vibrancy = 0.17 } } })
hl.layer_rule({ match = { namespace = "^orbit$" }, blur = true, ignore_alpha = 0.2 })
```

(`ignore_alpha = 0.2` leaves pixels under 20 % alpha unblurred, so the
scrim margin stays a plain dim and the panel gets the frost. Field names
are from `/usr/share/hypr/stubs/hl.meta.lua`, `HL.LayerRuleSpec`.) Orbit
side: scrim about 35–45 % Midnight, panel about 55–65 % Midnight/Slate,
hairline edge about 14 % white, plus a top-edge inner highlight (white
6–8 % gradient over the top 1–2 px). Keep `--opaque` and `--high-contrast`.
Without blur (user has it off), fall back to an opacity that still gives
4.5:1 for Cloud and Silver text over a white desktop; the theme test
already checks that, so it is a matter of picking the lowest alpha that
passes (around 0.8 combined, close to today).

## 3. Scrim

**Concept.** Outside the panel the wallpaper is dimmed only slightly (the
hero's sky and mountains are fully legible), maybe 20–30 %.

**Orbit.** 80 % Midnight with a violet-cast gradient. Nearly black.

**Gap / fix.** Too heavy once the panel is bounded and blurred. 35–45 %, no
gradient (the cast reads as a color error, not a design).

## 4. Header: mark and wordmark

**Concept.** The mark and wordmark live in the branding panels, not inside
the overview. The mark is a shaded globe (Sky-to-Violet gradient) with a
tapered orbit ring passing in front and behind, an Amber/Coral planet on
the ring, highlights top-left. The wordmark is Inter Bold/ExtraBold at
about 1.6x the mark's height, tight tracking.

**Orbit.** A 36 px draft mark (hand-traced; the ring is a uniform stroke,
the globe flat) plus "Orbit" at 26 px SemiBold, top-left of the panel; a
muted "Workspace 91 · eDP-1" at top-right.

**Gap.** The mark reads as a small blue ring with an orange dot; the
header row costs 60 px of vertical space the concept gives to cards.

**Fix.** Remove the header from the overview (P1). If a mark must appear,
use it at 20–24 px inside the search bar's left slot or as a corner
watermark at 20 % alpha. Redraw the mark from the guide's APP ICON panel
(shaded globe, tapered ring, 2 highlights) before it ships anywhere (P2,
needs Jason's approval either way).

## 5. Search bar

**Concept.** Centered, about 73 % of the panel's width, height about 4.3 %
of panel width (56–60 px at a 1760 panel), corner radius about 30 % of its
height (18–20 px), fill a near-opaque Slate/Midnight (darker than the
panel), 1 px hairline, magnifier 2 px stroke at about 40 % of the height
(22–24 px), placeholder in Silver at about 32–36 % of the height (18–20 px),
and a key chip ("⌘ K") at the right end: a lighter rounded square, 1 px
hairline, Silver text.

**Orbit.** Full panel width (2160 px, 97 %), 52 px tall, radius 14, fill
Midnight at 72 %, 18 px magnifier (two shapes, 2 px), 15 px text, "Ctrl K"
chip 12 px. Focus: 2 px Sky border with a Sky glow.

**Gap.** Proportions: a 41:1 strip instead of the concept's 12:1 bar.
Text and icon are small for the bar. Everything else is close.

**Fix (P1).** Max width 1280 centered (73 %), height 56, radius 18, text 17,
icon 22, chip 13 px Medium with 10 px radius; bar fill at 85–90 % so it
sits on the glass as a darker element.

## 6. Workspace rail

**Concept.** 5 tiles, each about 10 % of the panel width (about 176 px at
1760), aspect roughly 16:10 in the hero and 4:3 in UI ELEMENTS, radius
about 8–10, gap about 20 % of the tile width, 1 px hairline. Below each a
single **number** in White SemiBold at about 28 % of the tile height
(about 15–16 px). No captions. The active tile has a 2 px Violet border and
a soft Violet glow (about 8 px, 35 %). Tiles show the workspace's wallpaper
or a composite, not a single window.

**Orbit.** Tiles 152 x 86 (6.8 % of panel width, 16:9), radius 10, gap 14,
hairline; a live capture of one "cover" window stretched by `object-fit:
cover`. Below: the name at 14 px (SemiBold if active/previewed, with a
6 px Sky dot for active), then a caption line at 11 px Silver ("Active · 2
windows", "Previewing · 1 window"). Previewed: 2 px Violet + glow 55 % / 20
px; focused adds the Sky ring. A dashed "+ New" tile. 12 tiles here, so
the rail is an 1800 px-wide row.

**Gap.** Smaller tiles, two lines of text under each (the concept has one
word-sized number), a dot, dashed tiles. The row reads as a list of labels,
not a strip of thumbnails. And the cover captures are a privacy problem
(F-rail: every workspace's cover window is captured the moment the
overview opens; see the capture note above).

**Fix (P1).** Tiles 176 x 110, radius 10, gap 32, one label line (the
workspace name/number, 15 px SemiBold White for active, Medium Cloud
otherwise); move the counts and "Active/Previewing" into the accessible
label and a tooltip-style line that appears only on the focused tile.
Previewed: 2 px Violet border + 8 px glow at 35 %; active: a 2 px Sky
underline or dot *in* the label, not next to it. Make the "New" tile a
plain outlined "+" with no dashed border. Only capture covers for the
previewed workspace and its neighbours, or make cover capture opt-in
(privacy, and it halves the open-time capture work).

## 7. Window cards

**Concept.** Fixed 3 columns. Card width about 28–29 % of the panel width
(about 490 px at 1760), gap about 6 % of the card width (28 px). The card
*is* the preview: the window image with rounded corners (radius about
3.5 % of the width, 14–16 px), a 1 px hairline, a soft drop shadow (black
30–40 %, offset about 2 % of card height, blur about 6 %), no inner frame
or padding, real 16:10 window aspect. Under it, left-aligned, 12–16 px
below: a real app icon at about 9 % of the card width (36–40 px, rounded
square) and the app name in White SemiBold at about 5 % of the card width
(18–20 px). Nothing else: no title, no badges, no workspace text. Row gap
below the label about 7 % of the card width (32–36 px).

**Orbit.** Columns = floor((grid width + 20) / 380) = 5 at this width, but
`flex-wrap` + `justify-center`, so one or two cards sit centered in the
row. Card 360 wide; the preview frame is 350 x 225 (16:10) inside a 3 px
focus-ring pad + 2 px transparent ring; radius 14; fill Midnight at 60 %;
hairline; shadow black 35 % / 10 px offset / 24 px blur; the image is
`object-fit: contain` inside that frame, so a non-16:10 window shows
letterbox bars of the frame's fill. Below: 24 px letter tile (the app's
initial on a Violet/Sky/Coral/Amber square chosen by byte sum), the app
name at 15 px Medium Cloud, state badges ("Floating", "Fullscreen",
"XWayland", "Moving to…") as 11 px pills, then the title at 13 px Silver,
and in search results a 12 px "Workspace N · monitor" line. Gap 20.
Preview source: a thumbnail capped at 640 x 400, downscaled on the capture
thread, then resampled again by the GPU to 438 x 281 device pixels.

**Gap.** Size: 350 px here is 15.7 % of the panel width against the
concept's 28–29 %, so each Orbit card is about half the concept's size
relative to the panel (and far smaller relative to the screen, since the
concept's panel is itself smaller). Jason's "far bigger" reading most
likely comes from the composition (isolated boxes on a huge sheet and the
frame + ring + text stack around each one), not the pixel size; the
numbers go the other way. Columns: unbounded (5 here, 9 on an ultrawide)
instead of a fixed 3. Material: a framed box with a fill and letterbox bars
instead of a bare image. Label: a monogram instead of the app's icon, and
three to four lines of mixed text where the concept has one. Sharpness:
double resampling (capture → 640x400 → 438x281) softens the preview; at a
concept-sized card (about 600 device px wide) the 640 px thumbnail would be
near 1:1 but any larger card would be upscaled.

**Fix (P0).** Fixed 3 columns (2 on narrow panels), card width = (panel
inner width − 2 gaps) / 3 clamped to 420–520 logical; gap 28; preview at
the window's real aspect, rounded 16, hairline, shadow 0/8/28 at 35 %, no
inner fill (a Midnight 60 % placeholder only while the frame is missing);
cards never grow to fill the row and never shrink below 420. Icon 36 px
from the .desktop entry and icon theme, cached, letter tile only as a
fallback; name 18 px SemiBold White; title hidden by default, shown as a
single Silver 13 px line only on the selected card; badges become small
corner glyphs on the preview (or go to the accessible label). Capture to
the card's device size in one step (area-average), so nothing is resampled
twice or upscaled.

## 8. Selection and focus

**Concept.** One treatment: a 2 px Violet border, corner radius matching
the element, and a soft Violet glow (about 8 px, 30–40 %). The UI ELEMENTS
buttons show Active (Violet fill, White text), Default (Cloud fill) and
Hover (Cloud at a lighter tint).

**Orbit.** Selection: 2 px Violet border + glow 50 % / 26 px. Keyboard
focus: a second 2 px Sky ring 3 px outside the first, with its own glow
30 % / 14 px. The close-up shows both rings concentric: a neon double
border. Tiles get the same pair. Hover: none.

**Gap.** The double ring is loud and the glow radius is twice the concept's;
nothing responds to hover.

**Fix (P1).** Keep selection as the concept's single Violet border + 8 px
glow at 35 %. Express keyboard focus as the *same* border in Sky when
focus is in the grid (one ring, color says which), plus the existing
accessible focus; the 3:1 boundary tests still pass since Sky meets them.
Add a hover state: hairline brightens to 20 % and the preview lifts 2 px.

## 9. Shadows, glow and highlights

**Concept.** Depth comes from the wallpaper showing through, a crisp 1 px
light edge on glass, a faint lighter gradient on the top of the panel, and
soft but small shadows. Glows are restrained.

**Orbit.** One huge panel shadow (0/30, blur 80, 50 %) that is invisible
against the 80 % scrim; card shadows 24 px blur; glows 20–26 px. Hairline
at 9 % white. No top highlight.

**Fix (P1).** Panel shadow 0/24/64 at 45 % (visible once the scrim is
lighter), hairline 14 %, top highlight gradient, card shadow 0/8/28 at
35 %, glows 8 px.

## 10. Typography

**Concept.** Inter. Wordmark Bold; labels Medium/SemiBold; body Regular.
Sizes relative to the card: name about 5 % of card width, rail numbers
15–16 px, search 18–20 px. Tight, even leading.

**Orbit.** Inter 4.1 is bundled (Regular/Medium/SemiBold/Bold) and
confirmed in use (`look.font = "Inter"`). Sizes: 26 / 15 / 14 / 11 / 15 /
13 / 12 / 13 / 11 px. Six distinct sizes under 16 px on one screen.

**Gap / fix (P1).** Too many small sizes. Collapse to a scale: 18 (card
name), 17 (search), 15 (rail label, status), 13 (secondary: selected card
title, hints). Weights: SemiBold for names and the active rail label,
Medium for everything interactive, Regular for secondary text. Verify at
1.25 scale that 13 px is still 4.5:1 (it is; size does not change ratio).

## 11. Color tokens

**Concept.** Midnight #0B1020, Violet #7C3AED, Coral #FF6B6B, Amber
#F59E0B, Sky #38BDF8, Slate #0F172A, Gray #334155, Silver #94A3B8, Cloud
#E2E8F0, White #FFFFFF.

**Orbit.** Identical values in `theme.rs`; roles sensible; contrast tests
in place. The Violet-cast scrim gradient and the Violet/Sky/Coral/Amber
monogram tiles are the only off-guide uses of color.

**Fix (P2).** Drop the scrim cast; the monogram tiles disappear with real
icons (keep a Gray tile with a Cloud initial as the fallback so the brand
accents stay reserved for state).

## 12. Empty, loading and no-results states

**Concept.** Not shown.

**Orbit.** Centered 20 px White title + 14 px Silver hint in the grid area
("No windows on Workspace 91", "Loading…", "No windows match"). Correct and
accessible, but in a 2160 x 1000 void they look lost.

**Fix (P2).** With the bounded, content-sized panel these sit in a 3-column
area; add the grid's outline (one dashed card-sized placeholder at 20 %
hairline) so the empty state keeps the panel's shape.

## 13. Motion

**Concept.** "Fluid motion"; nothing measurable.

**Orbit.** Panel fades in and rises 18 px over 200 ms, ease-out quint;
exit is Hyprland's `layersOut` fade. Reduced motion keeps the fade.

**Gap / fix (P2).** Fine as is. Once the panel is bounded, scale from 0.98
instead of translating 18 px (reads as a surface settling, not a sheet
sliding); make the rail's previewed-border change animate (120 ms) so
arrowing through workspaces does not flicker.

## Rendering quality notes (fractional scale)

- Card width 360 logical = 450 device px, but the frame (350) = 437.5:
  frames and borders land on half pixels at 1.25x, so hairlines and the
  2 px borders are antialiased into 2–3 px smears (visible in the
  close-up). Choose logical sizes that are multiples of 4 (integral at
  1.25x) and draw hairlines at 0.8 logical px (1 device px).
- Previews resample twice (see 7). Capture at the card's device size.
- The hero's previews are real window content at 1:1 or downscaled once;
  that crispness is a large part of what makes the concept look "high
  quality".

## Prioritized fix list

**P0: what makes it look low quality**
1. Composition: bounded, centered, content-sized panel (max about
   1760 x 1180 logical here, radius 40–48) with wallpaper around it; no
   header/footer rows inside it. (§1)
2. Material: Hyprland layer blur + lighter scrim (35–45 %) + glass panel
   (55–65 %) with a 14 % hairline and top highlight; the two Lua lines in
   §2; keep `--opaque`/`--high-contrast`; least-opaque legible fallback
   when blur is off. (§2, §3, §9)
3. Grid: fixed 3 columns, cards 420–520 wide, 28 px gaps, bare rounded
   preview at real aspect, no frame fill, icon + name only. (§7)
4. Real app icons (36 px) from .desktop entries; letter tile fallback. (§7)
5. Single-resample previews at device size; integral device sizes. (§7,
   quality notes)

**P1**
6. Search bar proportions (73 % width, 56 tall, radius 18, 17 px text). (§5)
7. Rail: 176 x 110 tiles, one label line, 32 px gaps, restrained glow;
   limit cover capture to the previewed workspace and its neighbours. (§6)
8. One selection ring (Violet, or Sky when keyboard focus is there), 8 px
   glow; hover state. (§8)
9. Type scale 18/17/15/13 and weights. (§10)
10. Shadows and glows at concept scale. (§9)

**P2**
11. Remove the Violet scrim cast; Gray fallback tile. (§11)
12. Redraw the mark from the APP ICON panel; keep it out of the overview
    unless approved. (§4)
13. Empty-state placeholder outline; scale-in motion; animated rail
    border. (§12, §13)

## What Jason will notice first

The panel does not look like glass and does not look like the concept's
object: it is a near-black sheet covering the screen, with a brand header
and a hint bar, and the windows are small framed boxes with a letter tile
and three lines of text under them, floating in empty space. The concept is
a frosted panel over the wallpaper, packed edge to edge with large, bare
window images in three columns and a real app icon with a name under each.
Fixing composition (P0-1), material (P0-2) and the card grid (P0-3/4/5)
changes the first impression; everything in P1 and P2 is refinement on top
of that.

## Round 2 result

Reviewed 2026-10-19 on `djinn` (same output, 2304x1440 logical at 1.25x),
same method and rules as round 1: Orbit's own layer captured with `grim`
over the fixture workspaces only, daemon started with `--test-hooks` so
only fixture classes were captured (`orbit state` listed exactly the five
`dev.orbit.fixture.*` classes during every capture), the Omarchy bar band
and the whole rail band blacked out (the rail's non-fixture tiles show real
app icons), unmasked files deleted, no titles anywhere. Measurements are
from `orbit drive layout` (saved next to the captures) and from the
captures' pixels. Files, all in `docs/evidence/orb-008/djinn/round2/`:

- `std-ws91-masked.png`, `std-ws91-card-closeup.png`: standard theme
  (Hyprland blur is off here, so this is what Jason sees), workspace 91
  with the two tiled fixtures.
- `std-ws93-masked.png`, `std-ws93-card-closeup.png`: standard theme over
  the fullscreen fixture.
- `glass-ws91-masked.png`, `glass-ws91-card-closeup.png`: `--glass`
  forced, **without real blur** (blur is off), so the backdrop is sharp
  behind the panel; this shows the alphas, not the frosted result.
- `std-ws91-ring-fix-masked.png`, `std-ws91-ring-fix-closeup.png`: the
  standard theme again after the one fix made in this pass (below).

Note on the previews: the fixture windows draw their own 2 px accent
border (Violet for tiled-a, Sky for tiled-b, Coral for fullscreen), and at
device size that border is captured faithfully, so it sits just inside
Orbit's ring. What looks like a second ring on an unselected card in these
captures is the fixture's own border, not Orbit's.

### Per item: concept, now, remaining gap

**P0-1 Composition.** Concept: one bounded glass panel, wallpaper around
it, search + rail + grid only. Now: the panel is 1696 x 684 logical
(2120 x 855 device) with two windows: 73.6 % x 47.5 % of the output,
centered (left and right margins both 304 logical; device origin 380,440),
radius 44, with the wallpaper visible on all four sides on 91 (margin pixel
standard deviation 7.7 against the panel body's 0.84). No header, wordmark
or footer; the status/hints line is a separate 36 px bar under the panel.
Panel height is content-sized: one grid row here; it would be 1096 for two
rows and clamp at 1180. Gap: none in geometry. The empty columns either
side of two centered cards still read as space, but bounded space now.

**P0-2 Material.** Concept: frosted glass over the wallpaper, dark tint,
1 px light edge, soft shadow. Now, standard (blur off): panel body
composites to (14, 19, 37), flat (std 0.84), margin shows the wallpaper
dimmed to (24, 28, 47); hairline 14 %, top highlight, shadow 0/24/64 at
45 %, no violet cast. The desktop no longer ghosts through the panel
(round 1's complaint): nothing of the fixture behind it is visible on 91.
Glass (forced, no blur): panel body (25, 29, 48) with std 7.5, the sharp
fixture window shows through the panel and the margin is clearly the
wallpaper (std 23). Gap: the frosted look itself cannot be shown on this
machine until blur is enabled; without blur the standard theme is a dark
sheet with a lighter edge, which is correct but not glass. The two Lua
lines in §2 are the whole remaining step on Jason's side.

**P0-3 Grid.** Concept: fixed 3 columns, cards about 28 % of the panel
width, bare images, 16:10. Now: 3 columns, card 520 (30.7 % of the panel),
gaps 28/32, slot 520 x 324, bare image with radius 16, 1-device-px
hairline and a soft shadow; no frame fill or letterbox: the 3:2 fixtures
draw as 599 x 399 device px, the 16:10 fullscreen fixture as 638 x 399,
each at its own aspect inside the slot. Two cards cover 26 % of the panel
area (round 1: 4.8 %). Gap: with fewer than three windows the row is
centered, so one or two cards sit in the middle with empty columns beside
them; the concept never shows fewer than six.

**P0-4 Icons.** Concept: the app's real icon, about 36–40 px. Now: 36 px
icons from the desktop entry and the Yaru-magenta theme for installed
apps; the fixtures have no desktop entry, so the captures show the Gray
fallback tile with a Cloud initial (brand accents are no longer used for
monograms). Gap: none in mechanism; the captures cannot show a real icon
because only fixture windows are captured.

**P0-5 Single-step previews.** Concept: window content at 1:1 or downscaled
once. Now: each preview is captured straight to the slot's device box
(644 x 399) and drawn at exactly the texture size (`shown` equals `image`
for every preview in both layouts; origins 798,774 / 1483,774 / 1122,774
are whole device pixels). In the close-ups the fixture's 2 px border and
its small text are pixel-crisp, with none of round 1's smear. Gap: none.

**P0-6 Selection and focus.** Concept: one Violet ring with a soft glow.
Now: one 2-device-px ring with an 8 px glow at 35 %. In the first
captures the selected card's ring was Sky, because keyboard focus starts
in the grid and the round-2 build used Sky for "selected and focused", so
on every open the card looked focus-blue, not the guide's violet. Fixed in
this pass: cards are always Violet when selected and the glow marks
keyboard focus; tiles keep Violet for previewed and Sky for focused. The
ring-fix captures show the violet ring. Gap: none.

**P1 Search bar.** Concept: about 73 % of the panel, 56–60 tall, radius
18–20, 18–20 px text. Now: 1226 x 54 inside its hairline (1228 x 56
nominal), centered (673 device px from the left edge, symmetric), radius
18, 17 px text, 22 px magnifier, 13 px chip. Gap: none worth a line.

**P1 Rail.** Concept: five 176-wide tiles, one number under each. Now:
tiles have one 15 px label line and a caption only on the focused tile;
but with Jason's eleven workspaces plus the "new" tile the rail shrinks
them to 120 x 76 at a 16 px gap to keep all twelve on screen (the concept
shows five). Gap: judged from the layout only, since the rail band is
masked in the captures; the tiles are smaller than the concept's on this
machine, which is the honest consequence of twelve workspaces in a
1616-wide grid. Options for Jason: accept, cap the rail at the previewed
workspace's neighbours, or wrap to two rows.

**P1 Type, shadows, glows.** Scale 18/17/15/13, SemiBold names, restrained
glows; the hints line is 13 px Silver on a dark bar. Gap: none.

### Verdict

It now reads as the concept's object: a bounded dark panel floating over
the wallpaper with a centered search bar, a rail and a row of large, bare,
crisp window images with an icon and a name under each, one violet ring on
the selected one, and nothing else inside it. The first impression round 1
failed on (full-screen sheet, header, footer, tiny framed boxes, monogram
tiles, soft previews, double neon ring) is gone, and the numbers match the
targets. What still does not read as the concept, in order: the material
is not frosted glass until Jason enables Hyprland's blur (the standard
theme is a correct dark sheet, not glass; the `--glass` capture without
blur is see-through, not frosted); the rail is twelve small tiles rather
than five large ones because he has twelve workspaces; and with one or two
windows the grid is a centered pair in a three-column panel rather than the
hero's packed six. None of those is a quality defect in the rendering; the
first is a config step, the other two are decisions.

## Round 3 result

Jason's verdict on round 2 stood over the numbers: "cute, but not nearly
as polished and beautiful" as the concept, with Apple's Mission Control as
the quality bar, and then "I'm ok with the grid, but it MUST look like the
original concept" and "the panel is great, but it should go full screen
like Mission Control". Round 3 (2026-10-19, same host and rules) rebuilt
the surface to that brief. Captures in `docs/evidence/orb-008/djinn/round3/`
(Orbit's layer with `--test-hooks`, fixture classes only captured, the
desktop backdrop replaced by the solid fallback so no desktop content is
in the evidence, the bar band masked, unmasked files deleted, no titles):

- `search-masked.png`: the hero-like view, five fixture windows found by
  a search, three columns, two rows.
- `ws91-masked.png`, `ws93-masked.png`: workspace 91 (two windows, two big
  columns) and 93 (one), with the rail.
- `hints-masked.png`: the key hints shown with Ctrl+/.
- `side-by-side-hero.png`: the hero crop over the search view at the same
  width.

Note that the rail in these captures shows the outlines of windows on
Jason's real workspaces (their geometry only, from Hyprland's `at`/`size`;
no content is captured for them), and that the fixture windows draw their
own coloured borders inside the previews.

### What changed, item by item

**Surface.** Full screen, edge to edge, no panel, no margin, no rounded
box. The glass is Orbit's own: one capture of the output when the overview
opens (`ext-image-copy-capture` on an output source), point-sampled to a
sixth and box-blurred three times (480 x 300, 0.7 ms + 3.0 ms measured),
drawn under the tint with a lit top edge. It no longer depends on
Hyprland's blur or any config; `--no-glass` keeps the dense layers. In
test mode the backdrop is a solid, so the captures show a flat (17, 21, 44)
glass; on Jason's desktop the blurred wallpaper shows through the tint.

**Proportions.** Search 1280 x 56 at y = 40, rail at y = 120 (176 x 112
tiles, ten fit at 2304, scrolling for the rest), grid from y = 300 to the
bottom padding. Cards 684 wide in three columns (760 for one or two
windows in two columns), slot 684 x 428; previews drawn at their captured
size (fixtures: 600 x 400 at 3:2 and 849 x 529 at 16:10 for the fullscreen
one, aspect within 2 %), origins on whole device pixels. The five-card view
fills the width and two rows of the height, as the hero's six do.

**Rail.** The hero's strip: number under each tile, previewed one ringed
violet, keyboard focus sky, active dot; tiles are the desktop composited
small (backdrop plus the workspace's windows from real geometry, filled
with the previews already captured for the previewed workspace, outlines
elsewhere). Nothing off the previewed workspace is captured any more,
which also closes the round-1 privacy gap outside test mode. Twelve
workspaces scroll in a ten-tile strip that follows the previewed or
focused tile; every workspace stays reachable with the arrows.

**Chrome.** The hints bar, the state chips and the place label are gone;
states live in the accessible labels, hints sit behind Ctrl+/ (or F1), and
only failures and the compositor link show as a toast. Labels sit under
the image's left edge as in the hero. Selection is the violet ring with
the glow for keyboard focus.

**Motion.** Previews fly from their windows' real rectangles into the grid
over 220 ms and back on close; the glass fades with them; the layer closes
after the exit flight (the slice check still passes: focus hand-off waits
for the layer either way). Reduced motion fades only.

**Gates after the round.** fmt, clippy with warnings denied, the test
suite, `slice-check.sh` and `review-check.sh` (which now checks the
full-screen surface, no panel, no backdrop in test mode, hidden hints,
column count, cards filling the width, search centering, rail tile size and
centering, rows fitting the grid, previews at captured size, inside the
box, real aspect and whole-pixel origins) all pass.

### Side by side with the hero

Same: the composition (search, strip of numbered tiles with the violet
one, three columns of large bare previews with icon and name under the
left edge, one violet ring), the dark glass, the type and tokens, and the
proportions (search width, tile size, card size relative to the frame).

Different, and why: the hero's glass glows with a blurred sunset; the
captures are the solid test fallback, and on a real desktop the blur shows
through at 24 % of the tint, which is darker than the hero's backdrop but
is the most the contrast rules allow over a bright wallpaper. The hero's
previews are rich application windows; the fixtures are dark test cards
with their own borders. The hero's rail tiles are wallpaper images; ours
are composites, which in test mode are outlines. The hero is drawn in
perspective with a vignette; Orbit is flat and will stay so.

### Verdict

As a rendering, the surface now matches the hero's structure and
proportions and has the finish Mission Control sets: big, crisp, real-size
previews with soft shadows, a frosted edge-to-edge backdrop, one ring,
nothing else on the glass, and motion from and to the real windows. What
the captures cannot show, and only Jason's own open can judge, is the
frosted material over his wallpaper and real app windows with their icons;
if that still reads flat, the one knob left is the tint depth, which the
4.5:1 rule currently pins at scrim 40 % + glass 60 %.
