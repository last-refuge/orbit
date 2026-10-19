//! Brand tokens from the Orbit Desktop Brand Guide (product plan §04).

use std::sync::Arc;

use gpui::{Image, ImageFormat, Rgba, rgb};

/// A named brand color, kept as a raw hex value so it can live in `const`s.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Token {
    pub name: &'static str,
    pub hex: u32,
}

impl Token {
    pub fn rgba(self) -> Rgba {
        rgb(self.hex)
    }

    /// The color at `alpha` opacity, for layered surfaces and glows.
    pub fn alpha(self, alpha: f32) -> Rgba {
        Rgba {
            a: alpha,
            ..self.rgba()
        }
    }
}

/// Main canvas and overlay backing.
pub const MIDNIGHT: Token = Token {
    name: "Midnight",
    hex: 0x0B1020,
};
/// Selected workspace, primary action, restrained glow.
pub const VIOLET: Token = Token {
    name: "Violet",
    hex: 0x7C3AED,
};
/// Errors and destructive-action accents.
pub const CORAL: Token = Token {
    name: "Coral",
    hex: 0xFF6B6B,
};
/// Warnings and pending attention.
pub const AMBER: Token = Token {
    name: "Amber",
    hex: 0xF59E0B,
};
/// Keyboard focus and informational accents.
pub const SKY: Token = Token {
    name: "Sky",
    hex: 0x38BDF8,
};
/// Cards and raised surfaces.
pub const SLATE: Token = Token {
    name: "Slate",
    hex: 0x0F172A,
};
/// Dividers and neutral controls.
pub const GRAY: Token = Token {
    name: "Gray",
    hex: 0x334155,
};
/// Secondary labels on dark surfaces.
pub const SILVER: Token = Token {
    name: "Silver",
    hex: 0x94A3B8,
};
/// Primary body text on dark surfaces.
pub const CLOUD: Token = Token {
    name: "Cloud",
    hex: 0xE2E8F0,
};
/// High-emphasis labels and light variants.
pub const WHITE: Token = Token {
    name: "White",
    hex: 0xFFFFFF,
};

pub const PALETTE: [Token; 10] = [
    MIDNIGHT, VIOLET, CORAL, AMBER, SKY, SLATE, GRAY, SILVER, CLOUD, WHITE,
];

/// Brand tokens by role. Text roles meet 4.5:1 and state boundaries
/// (selection, focus, preview) 3:1 against every surface they sit on; the
/// tests below check the pairs the overview draws. Idle card and tile
/// borders in the standard theme are decorative (their text labels
/// identify them); the high-contrast theme makes them 3:1 too.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    pub name: &'static str,
    /// The overlay backing; opaque, so composited colors are these.
    pub canvas: Token,
    /// Cards, tiles and the search field.
    pub surface: Token,
    /// Idle borders and dividers.
    pub border: Token,
    /// Body text.
    pub text: Token,
    /// Secondary labels.
    pub muted: Token,
    /// Headings and high-emphasis labels.
    pub strong: Token,
    /// Selection and the previewed workspace.
    pub selection: Token,
    /// Keyboard focus rings, the caret and informational badges.
    pub focus: Token,
    pub error: Token,
    pub warning: Token,
    /// Opacity of the scrim over the desktop and of the panel over the
    /// scrim (plan §04: translucent and layered, with an opaque fallback).
    pub scrim_alpha: f32,
    pub panel_alpha: f32,
    /// Opacity of the dark bars that sit on the panel (the search field,
    /// the status line): secondary text only ever sits on these.
    pub bar_alpha: f32,
    /// Opacity of white hairline borders on glass surfaces.
    pub hairline_alpha: f32,
    /// Opacity of the white inner edge of the selection ring. The glass
    /// theme needs it: over a bright desktop its Violet ring alone falls
    /// under 3:1, and the edge keeps a 3:1 contour on every desktop.
    pub ring_edge_alpha: f32,
}

impl Theme {
    /// The same theme with every layer opaque.
    pub const fn opaque(self) -> Self {
        Self {
            name: "standard",
            scrim_alpha: 1.0,
            panel_alpha: 1.0,
            bar_alpha: 1.0,
            ..self
        }
    }

    /// The frosted-glass presentation (Orbit's own blurred backdrop under
    /// the tint; see `docs/orb-008-design-targets.md`).
    pub fn is_glass(&self) -> bool {
        self.name == GLASS.name
    }

    /// The tint at `percent` of its strength (80–120), for the settings'
    /// "tint strength"; alphas never exceed 1. Under 100 % the contrast
    /// guarantee over a bright desktop no longer holds, which the settings
    /// window says.
    pub fn tinted(self, percent: u32) -> Self {
        if self.is_opaque() || percent == 100 {
            return self;
        }
        let f = (percent.clamp(80, 120) as f32) / 100.;
        Self {
            scrim_alpha: (self.scrim_alpha * f).min(1.0),
            panel_alpha: (self.panel_alpha * f).min(1.0),
            bar_alpha: (self.bar_alpha * f).min(1.0),
            ..self
        }
    }

    pub fn is_opaque(&self) -> bool {
        self.scrim_alpha >= 1.0 && self.panel_alpha >= 1.0
    }

    /// The darkest and lightest the panel can look once composited over
    /// any desktop: the colors text actually sits on.
    pub fn panel_extremes(&self) -> [Token; 2] {
        [
            Token {
                name: "Black",
                hex: 0,
            },
            WHITE,
        ]
        .map(|desktop| {
            let scrim = over(self.canvas, self.scrim_alpha, desktop);
            over(self.surface, self.panel_alpha, scrim)
        })
    }

    /// The same for the dark bars on the panel.
    pub fn bar_extremes(&self) -> [Token; 2] {
        self.panel_extremes()
            .map(|panel| over(self.canvas, self.bar_alpha, panel))
    }

    /// The dimmest the scrim gets over any desktop.
    pub fn scrim_extremes(&self) -> [Token; 2] {
        [
            Token {
                name: "Black",
                hex: 0,
            },
            WHITE,
        ]
        .map(|desktop| over(self.canvas, self.scrim_alpha, desktop))
    }
}

/// `top` at `alpha` composited over an opaque `bottom` (sRGB, as the
/// compositor blends).
pub fn over(top: Token, alpha: f32, bottom: Token) -> Token {
    let mix = |shift: u32| {
        let t = f64::from((top.hex >> shift) & 0xFF);
        let b = f64::from((bottom.hex >> shift) & 0xFF);
        ((t * f64::from(alpha) + b * (1.0 - f64::from(alpha))).round() as u32) << shift
    };
    Token {
        name: "composited",
        hex: mix(16) | mix(8) | mix(0),
    }
}

pub const STANDARD: Theme = Theme {
    name: "standard",
    canvas: MIDNIGHT,
    surface: SLATE,
    border: GRAY,
    text: CLOUD,
    muted: SILVER,
    strong: WHITE,
    selection: VIOLET,
    focus: SKY,
    error: CORAL,
    warning: AMBER,
    // Without compositor blur: the lowest alphas that keep every text role
    // at 4.5:1 and the Violet ring at 3:1 over a white desktop.
    scrim_alpha: 0.80,
    panel_alpha: 0.90,
    bar_alpha: 0.85,
    hairline_alpha: 0.14,
    ring_edge_alpha: 0.0,
};

/// The frosted-glass presentation (plan §04, brand guide hero): a light
/// scrim and a translucent panel over a backdrop the compositor blurs.
/// Chosen when Hyprland's blur is on (or `orbit daemon --glass`). Body
/// text stays 4.5:1 on the panel over any desktop; secondary text sits on
/// the darker bars; the selection ring carries a white inner edge.
pub const GLASS: Theme = Theme {
    name: "glass",
    scrim_alpha: 0.40,
    panel_alpha: 0.60,
    bar_alpha: 0.88,
    hairline_alpha: 0.14,
    ring_edge_alpha: 0.7,
    ..STANDARD
};

/// Opaque/high-contrast presentation (plan §04): brighter idle borders and
/// text, same meanings.
pub const HIGH_CONTRAST: Theme = Theme {
    name: "high-contrast",
    canvas: Token {
        name: "Black",
        hex: 0x000000,
    },
    surface: MIDNIGHT,
    border: SILVER,
    text: WHITE,
    muted: CLOUD,
    strong: WHITE,
    selection: VIOLET,
    focus: SKY,
    error: CORAL,
    warning: AMBER,
    scrim_alpha: 1.0,
    panel_alpha: 1.0,
    bar_alpha: 1.0,
    hairline_alpha: 0.5,
    ring_edge_alpha: 0.0,
};

/// WCAG relative luminance of an sRGB color.
pub fn luminance(token: Token) -> f64 {
    let channel = |shift: u32| {
        let c = f64::from((token.hex >> shift) & 0xFF) / 255.0;
        if c <= 0.04045 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(16) + 0.7152 * channel(8) + 0.0722 * channel(0)
}

/// WCAG contrast ratio, 1 to 21.
pub fn contrast(a: Token, b: Token) -> f64 {
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contrast_matches_known_values() {
        assert!(
            (contrast(
                WHITE,
                Token {
                    name: "Black",
                    hex: 0
                }
            ) - 21.0)
                .abs()
                < 1e-9
        );
        assert!((contrast(CLOUD, CLOUD) - 1.0).abs() < 1e-9);
        assert_eq!(
            over(
                WHITE,
                0.5,
                Token {
                    name: "Black",
                    hex: 0
                }
            )
            .hex,
            0x808080
        );
        assert!(HIGH_CONTRAST.is_opaque() && !STANDARD.is_opaque());
    }

    #[test]
    fn text_and_state_boundaries_meet_their_ratios() {
        for theme in [STANDARD, STANDARD.opaque(), GLASS, HIGH_CONTRAST] {
            let [dark, light] = theme.panel_extremes();
            // Body text and names sit on the bare panel.
            for ground in [theme.canvas, theme.surface, dark, light] {
                for text in [theme.text, theme.strong] {
                    let ratio = contrast(text, ground);
                    assert!(
                        ratio >= 4.5,
                        "{}: {} on {} is {ratio:.2}",
                        theme.name,
                        text.name,
                        ground.name
                    );
                }
                // Keyboard focus is a Sky ring on the panel.
                let ratio = contrast(theme.focus, ground);
                assert!(
                    ratio >= 3.0,
                    "{}: {} boundary on {} is {ratio:.2}",
                    theme.name,
                    theme.focus.name,
                    ground.name
                );
                // Selection is a Violet ring; where that alone falls under
                // 3:1 the ring's white inner edge must carry the contour.
                let ring = contrast(theme.selection, ground);
                let edge = over(WHITE, theme.ring_edge_alpha, ground);
                assert!(
                    ring >= 3.0 || (theme.ring_edge_alpha > 0.0 && contrast(edge, ground) >= 3.0),
                    "{}: {} ring on {} is {ring:.2} with no 3:1 edge",
                    theme.name,
                    theme.selection.name,
                    ground.name
                );
            }
            // Secondary text, the caret and state messages sit on bars.
            for ground in theme.bar_extremes() {
                for text in [
                    theme.text,
                    theme.muted,
                    theme.strong,
                    theme.focus,
                    theme.error,
                    theme.warning,
                ] {
                    let ratio = contrast(text, ground);
                    assert!(
                        ratio >= 4.5,
                        "{}: {} on bar {} is {ratio:.2}",
                        theme.name,
                        text.name,
                        ground.name
                    );
                }
            }
            // Text on a selected (violet) tile in the move menu.
            assert!(contrast(theme.strong, theme.selection) >= 4.5);
        }
        // Without blur the standard theme is the most translucent that keeps
        // Silver and the Violet ring legible over white on the bare panel.
        let [_, light] = STANDARD.panel_extremes();
        assert!(contrast(STANDARD.muted, light) >= 4.5);
        assert!(contrast(STANDARD.selection, light) >= 3.0);
        assert!(GLASS.is_glass() && !STANDARD.is_glass() && !GLASS.opaque().is_glass());
        let tinted = GLASS.tinted(120);
        assert!(tinted.scrim_alpha > GLASS.scrim_alpha && tinted.panel_alpha <= 1.0);
        assert_eq!(GLASS.tinted(100), GLASS);
        assert_eq!(HIGH_CONTRAST.tinted(80), HIGH_CONTRAST);
        // High contrast also makes idle boundaries 3:1.
        for ground in [HIGH_CONTRAST.canvas, HIGH_CONTRAST.surface] {
            assert!(contrast(HIGH_CONTRAST.border, ground) >= 3.0);
        }
    }
}

/// What the overview's update pill says (ORB-011), set by the daemon.
/// Empty text means no pill.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UpdateBadge {
    pub text: gpui::SharedString,
    /// The tooltip.
    pub detail: gpui::SharedString,
    /// A failed update that was rolled back: shown in the error colour.
    pub failed: bool,
}

impl gpui::Global for UpdateBadge {}

/// Jason's brand files (`crates/orbit-ui/assets/brand/`, see its README).
pub const BRAND_SYMBOL_PNG: &[u8] = include_bytes!("../assets/brand/orbit-symbol.png");
pub const BRAND_LOCKUP_PNG: &[u8] = include_bytes!("../assets/brand/orbit-logo-dark.png");
pub const BRAND_APP_ICON_PNG: &[u8] = include_bytes!("../assets/brand/orbit-app-icon.png");

/// The application icon (the symbol on its midnight tile) encoded as a
/// PNG of `size` px, for the desktop entry's icon theme.
pub fn app_icon_png(size: u32) -> Option<Vec<u8>> {
    let full = image::load_from_memory(BRAND_APP_ICON_PNG).ok()?;
    let icon = full.resize_exact(size, size, image::imageops::FilterType::Lanczos3);
    let mut out = std::io::Cursor::new(Vec::new());
    icon.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(out.into_inner())
}

/// The symbol at the size the app draws it (the overview's 48 px lockup is
/// 60 device px at 1.25), decoded and downscaled once.
pub fn brand_symbol() -> Arc<Image> {
    static SYMBOL: std::sync::OnceLock<Arc<Image>> = std::sync::OnceLock::new();
    SYMBOL
        .get_or_init(|| {
            let bytes = image::load_from_memory(BRAND_SYMBOL_PNG)
                .ok()
                .map(|full| {
                    let small = full.resize(256, 256, image::imageops::FilterType::Lanczos3);
                    let mut out = std::io::Cursor::new(Vec::new());
                    small
                        .write_to(&mut out, image::ImageFormat::Png)
                        .map(|_| out.into_inner())
                        .unwrap_or_else(|_| BRAND_SYMBOL_PNG.to_vec())
                })
                .unwrap_or_else(|| BRAND_SYMBOL_PNG.to_vec());
            Arc::new(Image::from_bytes(ImageFormat::Png, bytes))
        })
        .clone()
}

/// The "Orbit" wordmark cut out of the lockup and tinted to `color`
/// (the file's is midnight, for light surfaces; the overview's glass is
/// dark). Returns the image and its width / height; `None` if the lockup
/// could not be read, and the caller sets the word in Inter instead.
pub fn brand_wordmark(color: Token) -> Option<(Arc<Image>, f32)> {
    static WORDMARKS: std::sync::Mutex<Vec<(u32, Arc<Image>, f32)>> =
        std::sync::Mutex::new(Vec::new());
    let mut cache = WORDMARKS.lock().ok()?;
    if let Some((_, image, aspect)) = cache.iter().find(|(hex, _, _)| *hex == color.hex) {
        return Some((image.clone(), *aspect));
    }
    let (bytes, aspect) = cut_wordmark(BRAND_LOCKUP_PNG, color.hex)?;
    let image = Arc::new(Image::from_bytes(ImageFormat::Png, bytes));
    cache.push((color.hex, image.clone(), aspect));
    Some((image, aspect))
}

/// Finds the word to the right of the widest fully transparent column gap
/// in a lockup, crops it to its opaque bounds, and sets every pixel's
/// colour to `hex` (alpha kept, so the antialiasing survives).
fn cut_wordmark(lockup: &[u8], hex: u32) -> Option<(Vec<u8>, f32)> {
    let rgba = image::load_from_memory(lockup).ok()?.to_rgba8();
    let (width, height) = rgba.dimensions();
    let opaque_column = |x: u32| (0..height).any(|y| rgba.get_pixel(x, y)[3] > 8);
    let columns: Vec<bool> = (0..width).map(opaque_column).collect();
    let first = columns.iter().position(|c| *c)?;
    // The widest transparent run after the first ink: between symbol and word.
    let (mut best_start, mut best_len, mut run_start) = (None, 0usize, None);
    for (x, opaque) in columns.iter().enumerate().skip(first) {
        match (opaque, run_start) {
            (false, None) => run_start = Some(x),
            (true, Some(start)) => {
                if x - start > best_len {
                    best_len = x - start;
                    best_start = Some(start);
                }
                run_start = None;
            }
            _ => {}
        }
    }
    let word_left = (best_start? + best_len) as u32;
    let word_right = (word_left..width).rev().find(|x| columns[*x as usize])? + 1;
    let opaque_row = |y: u32| (word_left..word_right).any(|x| rgba.get_pixel(x, y)[3] > 8);
    let top = (0..height).find(|y| opaque_row(*y))?;
    let bottom = (0..height).rev().find(|y| opaque_row(*y))? + 1;
    let mut word =
        image::imageops::crop_imm(&rgba, word_left, top, word_right - word_left, bottom - top)
            .to_image();
    let [r, g, b] = [(hex >> 16) as u8, (hex >> 8) as u8, hex as u8];
    for pixel in word.pixels_mut() {
        pixel.0 = [r, g, b, pixel[3]];
    }
    let mut out = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(word.clone())
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()?;
    Some((out.into_inner(), word.width() as f32 / word.height() as f32))
}

/// Inter 4.1 (SIL OFL 1.1, `assets/fonts/inter/LICENSE.txt`): the static
/// weights the overview uses, taken unchanged from the release's
/// `Inter.ttc`. GPUI's Linux text system does not apply variable-font axes,
/// so the variable font would render every weight as Regular.
pub const INTER_FONTS: [&[u8]; 4] = [
    include_bytes!("../assets/fonts/inter/Inter-Regular.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-Medium.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-SemiBold.ttf"),
    include_bytes!("../assets/fonts/inter/Inter-Bold.ttf"),
];

/// The settings the overview reads at render time (ORB-010), mapped from
/// the user's settings by the daemon; the defaults match the hero.
#[derive(Clone, Debug, PartialEq)]
pub struct OverviewPrefs {
    /// A fixed column count, or `None` for auto (three; two for one or
    /// two windows).
    pub columns: Option<usize>,
    /// 0 never, 1 selected card only, 2 always.
    pub titles: u8,
    pub special_workspaces: bool,
    pub rail_at_bottom: bool,
    /// Plain drop or Enter follows the moved window (Shift inverts).
    pub follow_on_move: bool,
    /// Typing searches only the previewed workspace.
    pub search_this_workspace: bool,
    /// The hot corner, as (x at right, y at bottom) flags, when the corner
    /// should also close an open overview; with the dwell in ms.
    pub corner_closes: Option<(bool, bool, u32)>,
}

impl Default for OverviewPrefs {
    fn default() -> Self {
        Self {
            columns: None,
            titles: 1,
            special_workspaces: true,
            rail_at_bottom: false,
            follow_on_move: false,
            search_this_workspace: false,
            corner_closes: None,
        }
    }
}

/// How the overview presents itself, chosen at daemon start.
#[derive(Clone)]
pub struct Look {
    pub theme: Theme,
    /// Inter (bundled), else Adwaita Sans (an Inter derivative) if loading
    /// it failed, else GPUI's default UI font.
    pub font: Option<gpui::SharedString>,
    /// No travel animation; content still fades in.
    pub reduced_motion: bool,
    /// The orbital symbol (Jason's brand file).
    pub mark: Arc<Image>,
    /// The "Orbit" wordmark in the theme's text colour, with its aspect;
    /// `None` only if the lockup could not be read.
    pub wordmark: Option<(Arc<Image>, f32)>,
    /// Real app icons by window class (letter tiles stand in otherwise).
    pub icons: Arc<crate::icons::Icons>,
    pub prefs: OverviewPrefs,
}

impl Look {
    pub fn new(theme: Theme, font: Option<gpui::SharedString>, reduced_motion: bool) -> Self {
        Self::with_icons(
            theme,
            font,
            reduced_motion,
            Arc::new(crate::icons::Icons::new(None, Vec::new())),
        )
    }

    pub fn with_icons(
        theme: Theme,
        font: Option<gpui::SharedString>,
        reduced_motion: bool,
        icons: Arc<crate::icons::Icons>,
    ) -> Self {
        Self {
            theme,
            font,
            reduced_motion,
            mark: brand_symbol(),
            wordmark: brand_wordmark(theme.strong),
            icons,
            prefs: OverviewPrefs::default(),
        }
    }
}

impl Default for Look {
    fn default() -> Self {
        Self::new(STANDARD, None, false)
    }
}

impl gpui::Global for Look {}

impl Look {
    pub fn get(cx: &gpui::App) -> Self {
        cx.try_global::<Self>().cloned().unwrap_or_default()
    }

    /// Registers the bundled Inter with GPUI, whatever the system has.
    pub fn register_fonts(cx: &gpui::App) -> anyhow::Result<()> {
        cx.text_system().add_fonts(
            INTER_FONTS
                .iter()
                .map(|bytes| std::borrow::Cow::Borrowed(*bytes))
                .collect(),
        )
    }

    /// The brand face (plan §04): the bundled Inter once registered, else
    /// the closest installed face.
    pub fn pick_font(cx: &gpui::App) -> Option<gpui::SharedString> {
        let installed = cx.text_system().all_font_names();
        ["Inter", "Inter Variable", "Adwaita Sans"]
            .into_iter()
            .find(|want| installed.iter().any(|name| name == want))
            .map(Into::into)
    }
}

#[cfg(test)]
mod brand_tests {
    use super::*;

    #[test]
    fn the_wordmark_is_cut_from_the_lockup_and_tinted() {
        let (png, aspect) = cut_wordmark(BRAND_LOCKUP_PNG, 0xFFFFFF).expect("lockup readable");
        assert!(
            aspect > 2.5 && aspect < 4.0,
            "Orbit is a wide word: {aspect}"
        );
        let word = image::load_from_memory(&png).unwrap().to_rgba8();
        assert!(
            word.width() > 500 && word.height() > 150,
            "{:?}",
            word.dimensions()
        );
        // Every visible pixel is the tint; the edges keep their alpha.
        let mut partial = 0;
        for p in word.pixels() {
            if p[3] > 0 {
                assert_eq!(&p.0[..3], &[255, 255, 255]);
            }
            if p[3] > 0 && p[3] < 255 {
                partial += 1;
            }
        }
        assert!(partial > 100, "antialiased edges survive");
        assert!(brand_wordmark(WHITE).is_some());
    }

    #[test]
    fn the_symbol_decodes() {
        let symbol = image::load_from_memory(BRAND_SYMBOL_PNG).unwrap();
        assert_eq!((symbol.width(), symbol.height()), (1254, 1254));
        let _ = brand_symbol();
    }
}
