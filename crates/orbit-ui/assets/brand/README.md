# Orbit brand files

Jason's brand artwork, copied unchanged from `logos/` at the repository
root on 2026-10-18 (see `generation-notes.json`, also copied, for how they
were produced from the Orbit Desktop Brand Guide). They replace the
hand-traced DRAFT SVGs that lived in `..` during ORB-008.

| File | Size | Use in the app |
| --- | --- | --- |
| `orbit-symbol.png` | 1254×1254 RGBA, transparent | the mark: overview lockup (48 px), settings header; downscaled once to 256 px at load |
| `orbit-logo-dark.png` | 2060×763 RGBA, transparent, midnight wordmark | the wordmark is cropped out of this at load and tinted to the theme's text colour (the overview's glass is dark, and no transparent white-text lockup exists) |
| `orbit-logo-midnight.png` | 2060×763 RGB, opaque midnight, white wordmark | reference / packaging on a midnight ground; not drawn by the app |
| `orbit-app-icon.png` | 1254×1254 RGBA, midnight tile | the application icon for packaging (ORB-009) |

All four are rasters; no vector source exists yet. If one arrives, drop it
here and switch `theme.rs` to `ImageFormat::Svg`.
