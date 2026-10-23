//! The overview surface (plan §04, brand guide hero, ORB-008 round 3): the
//! whole output becomes frosted glass (a blurred capture of the desktop
//! under a dark tint), with the guide's search bar, workspace rail and a
//! fixed three-column grid of large, bare window previews laid out on it
//! at the hero's proportions. Rendering reads the [`Desk`]; input becomes
//! [`Intent`]s through [`orbit_core::Overview`], so pointer and keyboard
//! paths share its rules, and the keyboard reaches everything a pointer
//! can ([`orbit_core::KEYBOARD_PATHS`]).
//!
//! Everything is positioned absolutely from [`Metrics`], so sizes land on
//! whole device pixels at fractional scales (multiples of 4 logical px are
//! integral at 1.25x; hairlines are one device pixel), previews are drawn
//! at the device size they were captured to (nothing is resampled twice),
//! and windows can fly from their real screen rectangles into the grid on
//! open and back on close. The measurable targets are in
//! `docs/orb-008-design-targets.md`.
//!
//! Every tile, card, preview and the search field carries an accessible
//! role and label (GPUI's AccessKit integration); the status line is a
//! polite live region for previews, results, moves and failures.

use std::{
    cell::RefCell,
    collections::HashMap,
    ops::Range,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

use gpui::{
    App, Bounds, BoxShadow, Context, DragMoveEvent, ElementInputHandler, Entity,
    EntityInputHandler, FocusHandle, FontWeight, Hsla, KeyBinding, Keystroke, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ObjectFit, Pixels, PlatformInput,
    Point, RenderImage, Rgba, Role, SharedString, Size, Subscription, UTF16Selection, Window,
    accesskit, actions, canvas, div, ease_out_quint, img, linear_color_stop, linear_gradient,
    point, prelude::*, px,
};
use orbit_core::{Card, Dest, Intent, Model, Region, Tile, Tone};

use crate::{
    desk::{Desk, Link, PreviewBoxes},
    theme::{self, Look, Theme, Token, UpdateBadge},
};

actions!(
    orbit_overview,
    [
        Dismiss,
        Next,
        Previous,
        ReorderLeft,
        ReorderRight,
        Up,
        Down,
        First,
        Last,
        Activate,
        ActivateFollow,
        TabForward,
        TabBack,
        MoveMenu,
        FocusSearch,
        Backspace,
        Delete,
        ToggleHints,
        OpenSettings
    ]
);

const CONTEXT: &str = "OrbitOverview";

// Composition (docs/orb-008-design-targets.md). Logical px unless noted.
/// Side padding as a share of the output width; top and bottom padding.
const PAD_FRACTION: f32 = 0.042;
const PAD_TOP: f32 = 40.;
const PAD_BOTTOM: f32 = 40.;
/// Between the search bar, the rail and the grid.
const ROW_GAP: f32 = 24.;
/// Columns in the grid; two when there are only one or two windows.
const COLUMNS: usize = 3;
/// Cards grow to fill the width, up to this.
const CARD_MAX: f32 = 760.;
const CARD_MIN: f32 = 320.;
const GAP: f32 = 28.;
const ROW_GAP_GRID: f32 = 32.;
/// Preview slot aspect (16:10); the window image keeps its own aspect
/// inside it.
const PREVIEW_ASPECT: f32 = 0.625;
const PREVIEW_RADIUS: f32 = 16.;
/// The ring around a preview or tile, device px; its inner edge is one
/// device px.
const RING_DEVICE: u32 = 2;
/// Under the preview: 12 px gap, then a 44 px row for icon, name and title.
const LABEL_GAP: f32 = 12.;
const LABEL_HEIGHT: f32 = 44.;
const ICON: f32 = 36.;
/// Rail thumbnails (16:10, the guide's 176x110 made integral at 1.25x)
/// with the workspace number under each and a caption row for the
/// focused tile. The strip scrolls when they do not all fit.
const THUMB_WIDTH: f32 = 176.;
const THUMB_HEIGHT: f32 = 112.;
const THUMB_GAP: f32 = 32.;
const THUMB_LABEL: f32 = 20.;
const THUMB_CAPTION: f32 = 16.;
const RAIL_HEIGHT: f32 = THUMB_HEIGHT + 8. + THUMB_LABEL + THUMB_CAPTION;
const SEARCH_HEIGHT: f32 = 56.;
/// The mark (the settings entry) at the top left: the brand at a size that
/// reads from across the room, centred on the search row, with the
/// wordmark beside it.
const MARK_SIZE: f32 = 48.;
const MARK_GAP: f32 = 14.;
/// The wordmark's height against the symbol's, as in the brand lockup.
const WORDMARK_SIZE: f32 = 26.;
const SEARCH_MAX_WIDTH: f32 = 1280.;
const SEARCH_WIDTH_FRACTION: f32 = 0.56;
/// The status toast and the hints bar (shown with Ctrl+/ or F1).
const BAR_HEIGHT: f32 = 36.;
/// Glows (plan §04: restrained).
const GLOW_BLUR: f32 = 8.;
const GLOW_ALPHA: f32 = 0.35;
/// Entry and exit: windows fly between their real rectangles and the grid
/// (plan §04: 160–220 ms); the glass fades in with them.
const FLY_MS: u64 = 220;
const SEARCH_PLACEHOLDER: &str = "Search windows, apps, or workspaces…";

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("escape", Dismiss, Some(CONTEXT)),
        KeyBinding::new("right", Next, Some(CONTEXT)),
        KeyBinding::new("left", Previous, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-left", ReorderLeft, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-right", ReorderRight, Some(CONTEXT)),
        KeyBinding::new("up", Up, Some(CONTEXT)),
        KeyBinding::new("down", Down, Some(CONTEXT)),
        KeyBinding::new("home", First, Some(CONTEXT)),
        KeyBinding::new("end", Last, Some(CONTEXT)),
        KeyBinding::new("enter", Activate, Some(CONTEXT)),
        KeyBinding::new("shift-enter", ActivateFollow, Some(CONTEXT)),
        KeyBinding::new("tab", TabForward, Some(CONTEXT)),
        KeyBinding::new("shift-tab", TabBack, Some(CONTEXT)),
        KeyBinding::new("ctrl-shift-m", MoveMenu, Some(CONTEXT)),
        KeyBinding::new("ctrl-k", FocusSearch, Some(CONTEXT)),
        KeyBinding::new("backspace", Backspace, Some(CONTEXT)),
        KeyBinding::new("delete", Delete, Some(CONTEXT)),
        KeyBinding::new("ctrl-/", ToggleHints, Some(CONTEXT)),
        KeyBinding::new("f1", ToggleHints, Some(CONTEXT)),
        KeyBinding::new("ctrl-,", OpenSettings, Some(CONTEXT)),
    ]);
}

/// Rounds down to a multiple of 4 logical px: integral device px at 1x,
/// 1.25x, 1.5x and 2x.
fn round4(value: f32) -> f32 {
    (value / 4.).floor().max(1.) * 4.
}

/// Rounds to the nearest multiple of 4 logical px.
fn near4(value: f32) -> f32 {
    (value / 4.).round().max(1.) * 4.
}

/// A rectangle in logical px.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    fn lerp(a: Rect, b: Rect, t: f32) -> Rect {
        Rect {
            x: a.x + (b.x - a.x) * t,
            y: a.y + (b.y - a.y) * t,
            w: a.w + (b.w - a.w) * t,
            h: a.h + (b.h - a.h) * t,
        }
    }
}

/// The surface's layout for an output, in logical px, plus the device
/// scale it is drawn at. Pure, so the targets are unit-tested.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Metrics {
    pub viewport: (f32, f32),
    pub scale: f32,
    pub cards: usize,
    pub tiles: usize,
    pub columns: usize,
    /// Card (and preview slot) width.
    pub card: f32,
    /// Preview slot height (16:10 of the card).
    pub slot: f32,
    pub search: Rect,
    /// The rail strip: as many whole tiles as fit, centered.
    pub rail: Rect,
    pub rail_fit: usize,
    /// The grid area between the rail and the bottom padding.
    pub grid: Rect,
}

impl Metrics {
    pub fn for_output(width: f32, height: f32, scale: f32, cards: usize, tiles: usize) -> Self {
        Self::for_output_with(width, height, scale, cards, tiles, None, false)
    }

    /// With the settings: a fixed column count and the rail at the bottom.
    pub fn for_output_with(
        width: f32,
        height: f32,
        scale: f32,
        cards: usize,
        tiles: usize,
        columns: Option<usize>,
        rail_at_bottom: bool,
    ) -> Self {
        let device = |v: f32| (v * scale).round();
        let logical = |d: f32| d / scale;
        let pad = round4(width * PAD_FRACTION);
        let inner = width - 2. * pad;
        let search_w = round4(width * SEARCH_WIDTH_FRACTION).min(SEARCH_MAX_WIDTH);
        let search = Rect {
            x: logical(((device(width) - device(search_w)) / 2.).floor()),
            y: PAD_TOP,
            w: search_w,
            h: SEARCH_HEIGHT,
        };
        let rail_fit = (((inner + THUMB_GAP) / (THUMB_WIDTH + THUMB_GAP)).floor() as usize)
            .clamp(1, tiles.max(1));
        let rail_w = rail_fit as f32 * THUMB_WIDTH + (rail_fit - 1) as f32 * THUMB_GAP;
        let rail = Rect {
            x: logical(((device(width) - device(rail_w)) / 2.).floor()),
            y: if rail_at_bottom {
                height - PAD_BOTTOM - RAIL_HEIGHT
            } else {
                PAD_TOP + SEARCH_HEIGHT + ROW_GAP
            },
            w: rail_w,
            h: RAIL_HEIGHT,
        };
        let columns =
            columns
                .map(|c| c.clamp(1, 4))
                .unwrap_or(if cards <= 2 { 2 } else { COLUMNS });
        let card =
            round4((inner - (columns - 1) as f32 * GAP) / columns as f32).clamp(CARD_MIN, CARD_MAX);
        let slot = near4(card * PREVIEW_ASPECT);
        // With no tiles (a search that matches no workspace) the grid moves
        // up under the search bar instead of leaving the rail's band empty.
        let (grid_top, grid_bottom) = if rail_at_bottom {
            (
                PAD_TOP + SEARCH_HEIGHT + ROW_GAP,
                if tiles == 0 {
                    height - PAD_BOTTOM
                } else {
                    rail.y - ROW_GAP
                },
            )
        } else if tiles == 0 {
            (rail.y, height - PAD_BOTTOM)
        } else {
            (rail.y + RAIL_HEIGHT + ROW_GAP, height - PAD_BOTTOM)
        };
        let grid = Rect {
            x: pad,
            y: grid_top,
            w: inner,
            h: (grid_bottom - grid_top).max(0.),
        };
        Self {
            viewport: (width, height),
            scale,
            cards,
            tiles,
            columns,
            card,
            slot,
            search,
            rail,
            rail_fit,
            grid,
        }
    }

    /// One grid row: the preview slot, the gap and the label row.
    pub fn row_height(&self) -> f32 {
        self.slot + LABEL_GAP + LABEL_HEIGHT
    }

    pub fn rows(&self) -> usize {
        self.cards.div_ceil(self.columns).max(1)
    }

    /// The grid's content height for all rows.
    pub fn content_height(&self) -> f32 {
        let rows = self.rows() as f32;
        rows * self.row_height() + (rows - 1.) * ROW_GAP_GRID
    }

    /// Logical px to whole device px.
    pub fn device(&self, logical: f32) -> u32 {
        (logical * self.scale).round().max(0.) as u32
    }

    /// Whole device px back to logical px.
    pub fn logical(&self, device: u32) -> f32 {
        device as f32 / self.scale
    }

    /// Where card `index` sits: rows fill the grid from the top, centered
    /// vertically while they fit; a partial last row is centered. On whole
    /// device pixels.
    pub fn card_rect(&self, index: usize) -> Rect {
        let (col, row) = (index % self.columns, index / self.columns);
        let in_row = if row + 1 == self.rows() {
            self.cards - row * self.columns
        } else {
            self.columns
        }
        .max(1);
        let row_w = in_row as f32 * self.card + (in_row - 1) as f32 * GAP;
        let x = self.grid.x
            + self.logical(self.device(self.grid.w).saturating_sub(self.device(row_w)) / 2)
            + col as f32 * (self.card + GAP);
        let spare = self
            .device(self.grid.h)
            .saturating_sub(self.device(self.content_height()));
        let y =
            self.grid.y + self.logical(spare / 2) + row as f32 * (self.row_height() + ROW_GAP_GRID);
        Rect {
            x,
            y,
            w: self.card,
            h: self.row_height(),
        }
    }

    /// The device-pixel box card previews are captured to: the slot minus
    /// the ring and its edge on each side.
    pub fn preview_boxes(&self) -> PreviewBoxes {
        let frame = 2 * (RING_DEVICE + 1);
        PreviewBoxes {
            card: (
                self.device(self.card).saturating_sub(frame).max(1),
                self.device(self.slot).saturating_sub(frame).max(1),
            ),
        }
    }

    /// Where rail tile `index` sits when the strip's first visible tile is
    /// `first`; tiles outside the strip are clipped.
    pub fn tile_rect(&self, index: usize, first: usize) -> Rect {
        Rect {
            x: self.rail.x + (index as f32 - first as f32) * (THUMB_WIDTH + THUMB_GAP),
            y: self.rail.y,
            w: THUMB_WIDTH,
            h: RAIL_HEIGHT,
        }
    }

    /// The first visible tile so that `shown` is in the strip, keeping it
    /// as central as the ends allow.
    pub fn rail_first(&self, shown: usize) -> usize {
        let n = self.tiles.max(1);
        let fit = self.rail_fit.max(1);
        let max_first = n.saturating_sub(fit);
        shown.saturating_sub(fit / 2).min(max_first)
    }
}

/// The value carried by a window drag.
#[derive(Clone)]
pub struct DraggedWindow {
    window: orbit_core::WindowId,
    app: SharedString,
    /// The card's captured frame and its device size, when one has arrived.
    preview: Option<(Arc<RenderImage>, (u32, u32))>,
    /// The window's real size in logical px (0,0 when unknown).
    size: (f32, f32),
}

/// The ghost's width in logical px over the grid; its height follows the
/// window's aspect.
const GHOST_WIDTH: f32 = 264.;
/// Within this many px of the rail the ghost starts shrinking (ORB-012).
const GHOST_APPROACH: f32 = 96.;
/// How long the ghost takes to settle at a new size.
const GHOST_TWEEN_MS: f32 = 120.;
/// The ghost never shrinks below this width.
const GHOST_MIN_WIDTH: f32 = 28.;

/// How far the ghost has shrunk toward its in-tile size: 0 away from the
/// rail, 1 inside it, by the pointer's distance to the rail's edge.
fn ghost_progress(pointer: (f32, f32), rail: Rect, approach: f32) -> f32 {
    let (x, y) = pointer;
    let dx = (rail.x - x).max(x - (rail.x + rail.w)).max(0.);
    let dy = (rail.y - y).max(y - (rail.y + rail.h)).max(0.);
    let distance = (dx * dx + dy * dy).sqrt();
    (1. - distance / approach.max(1.)).clamp(0., 1.)
}

/// The ghost's size at `progress`: the full frame at 0, and at 1 the size
/// the window would have inside a rail tile (its real geometry at the
/// tile's composite scale, the tile's width over the output's).
fn ghost_size(progress: f32, aspect: f32, window: (f32, f32), viewport_w: f32) -> (f32, f32) {
    let aspect = if aspect > 0. { aspect } else { 16. / 10. };
    let scale = THUMB_WIDTH / viewport_w.max(1.);
    let small_w = if window.0 > 0. {
        (window.0 * scale).clamp(GHOST_MIN_WIDTH, THUMB_WIDTH)
    } else {
        THUMB_WIDTH * 0.5
    };
    let w = GHOST_WIDTH + (small_w - GHOST_WIDTH) * progress.clamp(0., 1.);
    let h = (w / aspect).max(GHOST_MIN_WIDTH / aspect);
    (w.round(), h.round())
}

/// Follows the pointer during a drag: the window itself (its capture at
/// its real aspect, scaled down, lifted by a shadow), or, while no frame
/// has arrived, a chip with the app's name.
struct Ghost {
    app: SharedString,
    preview: Option<(Arc<RenderImage>, (u32, u32))>,
    offset: Point<Pixels>,
    theme: Theme,
    /// The window's real size, for its size inside a tile.
    size: (f32, f32),
    /// The last frame's layout: the rail's place and the output's width.
    layout: Rc<RefCell<Layout>>,
    reduced_motion: bool,
    /// Where the shrink is now (0 full, 1 in-tile), eased toward the target.
    displayed: f32,
    last_frame: Instant,
}

impl Render for Ghost {
    fn render(&mut self, window: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        let t = self.theme;
        let aspect = match &self.preview {
            Some((_, (dw, dh))) if *dw > 0 && *dh > 0 => *dw as f32 / *dh as f32,
            _ if self.size.0 > 0. && self.size.1 > 0. => self.size.0 / self.size.1,
            _ => 16. / 10.,
        };
        // The pointer against the rail: shrink as it arrives, grow as it leaves.
        let pointer = window.mouse_position();
        let (rail, viewport_w) = {
            let layout = self.layout.borrow();
            match layout.metrics {
                Some(m) => (Some(m.rail), m.viewport.0),
                None => (None, 1920.),
            }
        };
        let target = rail
            .map(|rail| {
                ghost_progress(
                    (f32::from(pointer.x), f32::from(pointer.y)),
                    rail,
                    GHOST_APPROACH,
                )
            })
            .unwrap_or(0.);
        let now = Instant::now();
        let dt_ms = now.duration_since(self.last_frame).as_secs_f32() * 1000.;
        self.last_frame = now;
        if self.reduced_motion {
            self.displayed = target;
        } else {
            let step = (dt_ms / GHOST_TWEEN_MS).clamp(0., 1.);
            self.displayed += (target - self.displayed) * step;
            if (target - self.displayed).abs() < 0.005 {
                self.displayed = target;
            }
        }
        if (target - self.displayed).abs() > 0.001 {
            window.request_animation_frame();
        }
        let (w, h) = if self.preview.is_some() || self.size.0 > 0. {
            ghost_size(self.displayed, aspect, self.size, viewport_w)
        } else {
            (180., 56.)
        };
        self.layout.borrow_mut().ghost = Some((w, h, self.displayed));
        let body = match self.preview.clone() {
            Some((image, _)) => div()
                .w(px(w))
                .h(px(h))
                .rounded(px(12.))
                .overflow_hidden()
                .border_2()
                .border_color(t.selection.rgba())
                .bg(t.canvas.rgba())
                .shadow(vec![soft_shadow(0.55)])
                .child(img(image).size_full().object_fit(ObjectFit::Fill)),
            None => div()
                .w(px(w))
                .h(px(h))
                .rounded(px(12.))
                .border_2()
                .border_color(t.selection.rgba())
                .bg(t.canvas.rgba())
                .shadow(vec![soft_shadow(0.55)])
                .text_color(t.strong.rgba())
                .text_size(px(15.))
                .font_weight(FontWeight::MEDIUM)
                .flex()
                .items_center()
                .justify_center()
                .child(self.app.clone()),
        };
        // Centred under the pointer.
        div()
            .pl(self.offset.x - px(w / 2.))
            .pt(self.offset.y - px(h / 2.))
            .child(body)
    }
}

/// A drawn preview: its bounds, the device size of the texture it shows,
/// and the window's real aspect (w / h) if known.
type DrawnPreview = (Bounds<Pixels>, (u32, u32), Option<f32>);

#[derive(Clone)]
struct DraggedWorkspace {
    id: orbit_core::WorkspaceId,
    epoch: orbit_core::Epoch,
    label: String,
    composite: Composite,
}

struct WorkspaceGhost {
    dragged: DraggedWorkspace,
    offset: Point<Pixels>,
}

impl Render for WorkspaceGhost {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let t = Look::get(cx).theme;
        let composite = &self.dragged.composite;
        div()
            .pl(self.offset.x - px(THUMB_WIDTH / 2.))
            .pt(self.offset.y - px(THUMB_HEIGHT / 2.))
            .child(
                div()
                    .w(px(THUMB_WIDTH))
                    .rounded(px(10.))
                    .bg(t.canvas.rgba())
                    .border_2()
                    .border_color(t.selection.rgba())
                    .shadow(vec![soft_shadow(0.55)])
                    .overflow_hidden()
                    .child(
                        div()
                            .relative()
                            .w_full()
                            .h(px(THUMB_HEIGHT))
                            .when_some(composite.backdrop.clone(), |d, image| {
                                d.child(img(image).size_full().object_fit(ObjectFit::Cover))
                            })
                            .children(composite.windows.iter().map(|w| {
                                let sx = THUMB_WIDTH / composite.monitor.0.max(1.);
                                let sy = THUMB_HEIGHT / composite.monitor.1.max(1.);
                                div()
                                    .absolute()
                                    .left(px(w.rect.x * sx))
                                    .top(px(w.rect.y * sy))
                                    .w(px((w.rect.w * sx).max(2.)))
                                    .h(px((w.rect.h * sy).max(2.)))
                                    .bg(t.surface.rgba())
                                    .border_1()
                                    .border_color(t.muted.alpha(0.5))
                                    .when_some(w.image.clone(), |d, image| {
                                        d.child(img(image).size_full().object_fit(ObjectFit::Fill))
                                    })
                            })),
                    )
                    .child(
                        div()
                            .p(px(8.))
                            .text_size(px(14.))
                            .text_color(t.strong.rgba())
                            .child(self.dragged.label.clone()),
                    ),
            )
    }
}

/// Where things were laid out in the last frame, for test hooks that drive
/// the real input path, for keyboard rows, and for placing the input
/// method's candidate window.
#[derive(Default)]
struct Layout {
    cards: HashMap<u64, Bounds<Pixels>>,
    tiles: HashMap<Dest, Bounds<Pixels>>,
    /// Each drawn preview image: its bounds, the device size of the
    /// texture it shows, and the window's real aspect (w / h) if known.
    previews: HashMap<u64, DrawnPreview>,
    search: Option<Bounds<Pixels>>,
    metrics: Option<Metrics>,
    backdrop: bool,
    hints: bool,
    /// The drag ghost as last drawn: width, height, shrink progress
    /// (0 over the grid, 1 in-tile size). Kept after the drop for the
    /// scripted checks; cleared when the next drag starts.
    ghost: Option<(f32, f32, f32)>,
}

pub struct OverviewView {
    desk: Entity<Desk>,
    focus_handle: FocusHandle,
    layout: Rc<RefCell<Layout>>,
    first_frame: Option<Box<dyn FnOnce()>>,
    opened_at: Instant,
    /// Set when a dismissal is animating out; the window closes after it.
    closing: Option<Instant>,
    show_hints: bool,
    /// The pointer is over the mark (shows its tooltip).
    mark_hovered: bool,
    /// The pointer rests in the hot corner (settings: the corner also
    /// closes); bumped on enter and leave so a dwell timer fires only for
    /// its own entry.
    corner_entry: u64,
    corner_inside: bool,
    workspace_drag: Option<DraggedWorkspace>,
    workspace_drop: Option<(Dest, bool)>,
    rail_drag_first: Option<usize>,
    workspace_drag_generation: u64,
    _observe: Subscription,
}

impl OverviewView {
    /// `session` is what [`Desk::open`] returned; the desk closes it when
    /// this view goes away, however the surface was dismissed.
    pub fn new(
        desk: Entity<Desk>,
        session: u64,
        first_frame: Box<dyn FnOnce()>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        cx.observe_global::<UpdateBadge>(|_, cx| cx.notify())
            .detach();
        let observe = cx.observe_in(&desk, window, |this, desk, window, cx| {
            if desk.update(cx, |desk, _| desk.take_dismiss()) {
                // A committed activation (the shortcut's double tap): the
                // window is what matters now, not the fly-out.
                this.begin_close_with(true, window, cx);
            }
            cx.notify();
        });
        let closing = desk.clone();
        cx.on_release(move |_, cx| closing.update(cx, |desk, cx| desk.close(session, cx)))
            .detach();
        Self {
            desk,
            focus_handle,
            layout: Rc::default(),
            first_frame: Some(first_frame),
            opened_at: Instant::now(),
            closing: None,
            show_hints: false,
            mark_hovered: false,
            corner_entry: 0,
            corner_inside: false,
            workspace_drag: None,
            workspace_drop: None,
            rail_drag_first: None,
            workspace_drag_generation: 0,
            _observe: observe,
        }
    }

    fn with_overview<R: Default>(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut orbit_core::Overview, &orbit_core::Model) -> R,
    ) -> R {
        if self.closing.is_some() {
            return R::default();
        }
        let mut result = None;
        self.desk.update(cx, |desk, cx| {
            desk.navigate(
                |o, m| {
                    result = Some(f(o, m));
                    None
                },
                cx,
            )
        });
        result.unwrap_or_default()
    }

    fn perform(&mut self, intent: Option<Intent>, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing.is_some() {
            return;
        }
        // Activating or switching: the surface goes at once. The fly-out
        // cost 220 ms before the window could even be handed focus.
        let instant = matches!(intent, Some(Intent::Activate(_) | Intent::Switch(_)));
        if let Some(intent) = intent
            && self.desk.update(cx, |desk, cx| desk.perform(intent, cx))
        {
            self.begin_close_with(instant, window, cx);
        }
    }

    /// Closes the surface after the windows have flown back (a dismissal),
    /// or at once with reduced motion.
    fn begin_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.begin_close_with(false, window, cx);
    }

    /// Closes the surface: at once (`instant`, or reduced motion), else
    /// after the windows have flown back.
    fn begin_close_with(&mut self, instant: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing.is_some() {
            return;
        }
        if instant || Look::get(cx).reduced_motion {
            window.remove_window();
            return;
        }
        self.closing = Some(Instant::now());
        cx.notify();
        cx.spawn_in(window, async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(FLY_MS))
                .await;
            let _ = this.update_in(cx, |_, window, _| window.remove_window());
        })
        .detach();
    }

    fn columns(&self) -> isize {
        self.layout.borrow().metrics.map_or(COLUMNS, |m| m.columns) as isize
    }

    fn dismiss(&mut self, _: &Dismiss, window: &mut Window, cx: &mut Context<Self>) {
        // Escape cancels a drag first; the window stays where it was.
        if cx.stop_active_drag(window) {
            self.clear_workspace_drag(cx);
            return;
        }
        if self.show_hints {
            self.show_hints = false;
            cx.notify();
            return;
        }
        let intent = self.with_overview(cx, |o, m| o.escape(m));
        self.perform(intent, window, cx);
    }

    fn next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        self.with_overview(cx, |o, m| o.horizontal(m, 1));
    }

    fn reorder_left(&mut self, _: &ReorderLeft, window: &mut Window, cx: &mut Context<Self>) {
        let intent = self.with_overview(cx, |o, m| o.reorder_focused(m, -1));
        self.perform(intent, window, cx);
    }

    fn reorder_right(&mut self, _: &ReorderRight, window: &mut Window, cx: &mut Context<Self>) {
        let intent = self.with_overview(cx, |o, m| o.reorder_focused(m, 1));
        self.perform(intent, window, cx);
    }

    fn clear_workspace_drag(&mut self, cx: &mut Context<Self>) {
        self.workspace_drag_generation += 1;
        self.workspace_drag = None;
        self.workspace_drop = None;
        self.rail_drag_first = None;
        cx.notify();
    }

    fn workspace_target(
        &self,
        dragged: &DraggedWorkspace,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<(Dest, bool)> {
        let desk = self.desk.read(cx);
        if !matches!(desk.status, Link::Live { .. }) || desk.store().epoch() != dragged.epoch {
            return None;
        }
        let layout = self.layout.borrow();
        let m = layout.metrics?;
        let x = f32::from(position.x);
        let y = f32::from(position.y);
        if x < m.rail.x || x > m.rail.x + m.rail.w || y < m.rail.y || y > m.rail.y + m.rail.h {
            return None;
        }
        let (dest, bounds) = layout.tiles.iter().min_by(|(_, a), (_, b)| {
            (f32::from(a.center().x) - x)
                .abs()
                .total_cmp(&(f32::from(b.center().x) - x).abs())
        })?;
        let after = matches!(dest, Dest::Existing(_)) && position.x > bounds.center().x;
        desk.overview()?
            .reorder_workspace(desk.store().model(), dragged.id, *dest, after)?;
        Some((*dest, after))
    }

    fn start_workspace_drag(
        &mut self,
        dragged: DraggedWorkspace,
        first: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.workspace_drag_generation += 1;
        let generation = self.workspace_drag_generation;
        self.workspace_drag = Some(dragged);
        self.rail_drag_first = Some(first);
        self.workspace_drop = None;
        // A timer also clears a drag released outside the rail. Holding at an
        // edge scrolls through workspaces that don't fit on the output.
        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(350))
                    .await;
                let keep = this.update_in(cx, |this, window, cx| {
                    if this.workspace_drag_generation != generation {
                        return false;
                    }
                    if !cx.has_active_drag() || this.workspace_drag.is_none() {
                        this.clear_workspace_drag(cx);
                        return false;
                    }
                    let position = window.mouse_position();
                    let metrics = this.layout.borrow().metrics;
                    if let Some(m) = metrics {
                        let desk = this.desk.read(cx);
                        let count = desk
                            .overview()
                            .map_or(0, |o| o.tiles(desk.store().model()).len());
                        let x = f32::from(position.x);
                        let y = f32::from(position.y);
                        let first = this.rail_drag_first.unwrap_or(0);
                        let next = if y >= m.rail.y && y <= m.rail.y + m.rail.h {
                            if x >= m.rail.x && x < m.rail.x + 36. {
                                first.saturating_sub(1)
                            } else if x > m.rail.x + m.rail.w - 36. && x <= m.rail.x + m.rail.w {
                                (first + 1).min(count.saturating_sub(m.rail_fit))
                            } else {
                                first
                            }
                        } else {
                            first
                        };
                        if next != first {
                            this.rail_drag_first = Some(next);
                            this.workspace_drop = None;
                            cx.notify();
                        } else if let Some(dragged) = &this.workspace_drag {
                            let target = this.workspace_target(dragged, position, cx);
                            if target != this.workspace_drop {
                                this.workspace_drop = target;
                                cx.notify();
                            }
                        }
                    }
                    true
                });
                if !matches!(keep, Ok(true)) {
                    break;
                }
            }
        })
        .detach();
        cx.notify();
    }

    fn previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        self.with_overview(cx, |o, m| o.horizontal(m, -1));
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        let columns = self.columns();
        self.with_overview(cx, |o, m| o.up(m, columns));
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        let columns = self.columns();
        self.with_overview(cx, |o, m| o.down(m, columns));
    }

    fn first(&mut self, _: &First, _: &mut Window, cx: &mut Context<Self>) {
        self.with_overview(cx, |o, m| o.home(m));
    }

    fn last(&mut self, _: &Last, _: &mut Window, cx: &mut Context<Self>) {
        self.with_overview(cx, |o, m| o.end(m));
    }

    fn activate(&mut self, _: &Activate, window: &mut Window, cx: &mut Context<Self>) {
        let follow = Look::get(cx).prefs.follow_on_move;
        let intent = self.with_overview(cx, |o, m| o.enter(m, follow));
        self.perform(intent, window, cx);
    }

    fn activate_follow(&mut self, _: &ActivateFollow, window: &mut Window, cx: &mut Context<Self>) {
        let follow = !Look::get(cx).prefs.follow_on_move;
        let intent = self.with_overview(cx, |o, m| o.enter(m, follow));
        self.perform(intent, window, cx);
    }

    /// The mark or Ctrl+,: the settings window opens and the overview
    /// closes (it holds the keyboard while it is up).
    fn open_settings(&mut self, _: &OpenSettings, window: &mut Window, cx: &mut Context<Self>) {
        if self.closing.is_some() {
            return;
        }
        self.desk.read(cx).request_settings();
        self.begin_close(window, cx);
    }

    /// The pointer moved on the surface: the hot corner can close the
    /// overview when the settings say so.
    fn pointer_moved(&mut self, at: Point<Pixels>, held: bool, cx: &mut Context<Self>) {
        let Some((right, bottom, dwell_ms)) = Look::get(cx).prefs.corner_closes else {
            return;
        };
        let Some(m) = self.layout.borrow().metrics else {
            return;
        };
        let zone = 6.;
        let x = f32::from(at.x);
        let y = f32::from(at.y);
        let inside = (if right {
            x >= m.viewport.0 - zone
        } else {
            x <= zone
        }) && (if bottom {
            y >= m.viewport.1 - zone
        } else {
            y <= zone
        });
        if inside == self.corner_inside {
            return;
        }
        self.corner_inside = inside;
        self.corner_entry += 1;
        if !inside || held || self.closing.is_some() {
            return;
        }
        let entry = self.corner_entry;
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(u64::from(dwell_ms)))
                .await;
            let _ = this.update(cx, |view, cx| {
                if view.corner_inside && view.corner_entry == entry && view.closing.is_none() {
                    let intent = view.with_overview(cx, |o, m| o.escape(m));
                    if matches!(intent, Some(Intent::Dismiss)) {
                        view.desk.update(cx, |desk, cx| {
                            desk.perform(Intent::Dismiss, cx);
                        });
                    }
                }
            });
        })
        .detach();
    }

    fn tab_forward(&mut self, _: &TabForward, _: &mut Window, cx: &mut Context<Self>) {
        self.with_overview(cx, |o, _| o.tab(false));
    }

    fn tab_back(&mut self, _: &TabBack, _: &mut Window, cx: &mut Context<Self>) {
        self.with_overview(cx, |o, _| o.tab(true));
    }

    fn move_menu(&mut self, _: &MoveMenu, _: &mut Window, cx: &mut Context<Self>) {
        self.with_overview(cx, |o, m| o.begin_move(m));
    }

    fn focus_search(&mut self, _: &FocusSearch, _: &mut Window, cx: &mut Context<Self>) {
        self.with_overview(cx, |o, _| o.focus_search());
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        self.with_overview(cx, |o, m| o.backspace(m));
    }

    fn delete(&mut self, _: &Delete, _: &mut Window, cx: &mut Context<Self>) {
        self.with_overview(cx, |o, m| o.delete(m));
    }

    fn toggle_hints(&mut self, _: &ToggleHints, _: &mut Window, cx: &mut Context<Self>) {
        self.show_hints = !self.show_hints;
        cx.notify();
    }

    fn editing<R>(&self, cx: &App, f: impl FnOnce(&orbit_core::Field) -> R) -> Option<R> {
        self.desk.read(cx).overview().map(|o| f(o.editing()))
    }

    /// Test hooks (`orbit drive …`): they go through GPUI's own input
    /// dispatch, so they exercise the same bindings, text input, clicks and
    /// drops as a person would. Call it from a window update that does not
    /// hold the view, since the dispatched input updates it.
    pub fn drive(
        view: &Entity<Self>,
        command: &str,
        window: &mut Window,
        cx: &mut App,
    ) -> Result<String, String> {
        // Text arguments run to the end of the line and may hold spaces.
        if let Some(text) = command.strip_prefix("type ") {
            for c in text.chars() {
                window.dispatch_keystroke(typed(c), cx);
            }
            return Ok("ok".into());
        }
        if let Some(text) = command.strip_prefix("compose ") {
            // What an input method sends while composing: marked text.
            view.update(cx, |view, cx| {
                view.replace_and_mark_text_in_range(None, text, None, window, cx)
            });
            return Ok("ok".into());
        }
        if let Some(text) = command.strip_prefix("commit ") {
            view.update(cx, |view, cx| {
                view.replace_text_in_range(None, text, window, cx)
            });
            return Ok("ok".into());
        }
        let mut words = command.split_whitespace();
        match (words.next(), words.next(), words.next(), words.next()) {
            (Some("key"), Some(keys), None, None) => {
                for key in keys.split(',') {
                    let keystroke = Keystroke::parse(key).map_err(|e| format!("{key}: {e}"))?;
                    window.dispatch_keystroke(keystroke, cx);
                }
                Ok("ok".into())
            }
            (Some("click"), Some(address), None, None) => {
                let at = view.read(cx).card_center(address)?;
                click(window, cx, at);
                Ok("ok".into())
            }
            (Some("click-workspace"), Some(name), None, None) => {
                let at = view.read(cx).tile_center(name, cx)?;
                click(window, cx, at);
                Ok("ok".into())
            }
            (Some("click-search"), None, None, None) => {
                let at = view
                    .read(cx)
                    .layout
                    .borrow()
                    .search
                    .map(|b| b.center())
                    .ok_or("the search field is not laid out")?;
                click(window, cx, at);
                Ok("ok".into())
            }
            (Some("drag"), Some(address), Some(name), follow) => {
                let shift = match follow {
                    None => false,
                    Some("follow") => true,
                    Some(other) => return Err(format!("unexpected {other}")),
                };
                let from = view.read(cx).card_center(address)?;
                let to = view.read(cx).tile_center(name, cx)?;
                drag(window, cx, from, to, shift);
                Ok("ok".into())
            }
            (Some(command @ ("drag-workspace" | "start-workspace-drag")), Some(source), Some(target), side) => {
                let from = view.read(cx).tile_center(source, cx)?;
                let offset = match side {
                    None | Some("before") => -THUMB_WIDTH / 3.,
                    Some("after") => THUMB_WIDTH / 3.,
                    Some(other) => return Err(format!("unexpected {other}")),
                };
                let to = if matches!(target, "outside" | "left-edge" | "right-edge") {
                    let m = view.read(cx).layout.borrow().metrics.ok_or("not laid out")?;
                    match target {
                        "outside" => point(px(m.grid.x + 10.), px(m.grid.y + 10.)),
                        "left-edge" => point(px(m.rail.x + 4.), px(m.rail.y + 30.)),
                        _ => point(px(m.rail.x + m.rail.w - 4.), px(m.rail.y + 30.)),
                    }
                } else {
                    let mut to = view.read(cx).tile_center(target, cx)?;
                    to.x += px(offset);
                    to
                };
                start_drag(window, cx, from, to, Modifiers::default());
                if command == "drag-workspace" { release_drag(window, cx, to, Modifiers::default()); }
                Ok("ok".into())
            }
            (Some("drop-workspace"), None, None, None) => {
                release_drag(window, cx, window.mouse_position(), Modifiers::default());
                Ok("ok".into())
            }
            (Some("a11y"), None, None, None) => Ok(a11y_summary(window).to_string()),
            (Some("layout"), None, None, None) => Ok(view.read(cx).layout_summary().to_string()),
            _ => Err(
                "usage: key K[,K…] | type TEXT | compose TEXT | commit TEXT | click 0xADDR \
                      | click-workspace NAME | click-search | drag 0xADDR NAME [follow] | a11y \
                      | drag-workspace SOURCE TARGET [before|after] | start-workspace-drag SOURCE TARGET [before|after] | drop-workspace | layout"
                    .into(),
            ),
        }
    }

    /// The last frame's measurable layout (`orbit drive layout`): the
    /// surface, search, rail and grid areas in logical and device px, and
    /// each preview's drawn size against its texture and the window's real
    /// aspect. No titles.
    fn layout_summary(&self) -> serde_json::Value {
        use serde_json::json;
        let layout = self.layout.borrow();
        let Some(m) = layout.metrics else {
            return json!({ "laid_out": false });
        };
        let bounds = |b: &Bounds<Pixels>| {
            json!({
                "x": f32::from(b.origin.x), "y": f32::from(b.origin.y),
                "width": f32::from(b.size.width), "height": f32::from(b.size.height),
                "device": [
                    f32::from(b.origin.x) * m.scale, f32::from(b.origin.y) * m.scale,
                    f32::from(b.size.width) * m.scale, f32::from(b.size.height) * m.scale,
                ],
            })
        };
        let rect = |r: Rect| json!({ "x": r.x, "y": r.y, "width": r.w, "height": r.h });
        json!({
            "laid_out": true,
            "viewport": [m.viewport.0, m.viewport.1],
            "scale": m.scale,
            "full_screen": true,
            "panel": serde_json::Value::Null,
            "backdrop": layout.backdrop,
            "hints_shown": layout.hints,
            "mark": {
                "x": m.grid.x,
                "y": PAD_TOP + (SEARCH_HEIGHT - MARK_SIZE) / 2.,
                "size": MARK_SIZE,
                "wordmark": true,
            },
            "columns": m.columns,
            "card": m.card,
            "slot": m.slot,
            "rows": m.rows(),
            "search": layout.search.as_ref().map(bounds),
            "search_rect": rect(m.search),
            "rail": rect(m.rail),
            "rail_fit": m.rail_fit,
            "tile": [THUMB_WIDTH, THUMB_HEIGHT],
            "ghost": layout.ghost.map(|(w, h, p)| json!({"width": w, "height": h, "progress": p})),
            "tile_gap": THUMB_GAP,
            "grid": rect(m.grid),
            "content_height": m.content_height(),
            "cards": layout.cards.len(),
            "tiles": layout.tiles.len(),
            "workspace_dragging": self.workspace_drag.is_some(),
            "workspace_drop": self.workspace_drop.map(|(dest, after)| serde_json::json!({ "destination": format!("{dest:?}"), "after": after })),
            "rail_drag_first": self.rail_drag_first,
            "previews": layout.previews.iter().map(|(address, (b, size, aspect))| json!({
                "address": format!("{address:#x}"),
                "image": [size.0, size.1],
                "shown": [
                    f32::from(b.size.width) * m.scale,
                    f32::from(b.size.height) * m.scale,
                ],
                "origin": [f32::from(b.origin.x) * m.scale, f32::from(b.origin.y) * m.scale],
                "window_aspect": aspect,
            })).collect::<Vec<_>>(),
        })
    }

    fn card_center(&self, address: &str) -> Result<Point<Pixels>, String> {
        let address = u64::from_str_radix(address.trim_start_matches("0x"), 16)
            .map_err(|_| format!("bad address {address}"))?;
        self.layout
            .borrow()
            .cards
            .get(&address)
            .map(|b| b.center())
            .ok_or_else(|| format!("no card for {address:#x} on screen"))
    }

    fn tile_center(&self, name: &str, cx: &App) -> Result<Point<Pixels>, String> {
        let desk = self.desk.read(cx);
        let model = desk.store().model();
        let dest = desk
            .overview()
            .and_then(|o| {
                o.tiles(model)
                    .into_iter()
                    .find(|t| t.label == name || (name == "new" && t.is_new()))
            })
            .map(|t| t.dest)
            .ok_or_else(|| format!("no workspace tile {name}"))?;
        self.layout
            .borrow()
            .tiles
            .get(&dest)
            .map(|b| b.center())
            .ok_or_else(|| format!("tile {name} not laid out (scrolled out of the strip?)"))
    }

    /// The guide's search bar: a dark rounded bar centered on the glass,
    /// magnifier, placeholder and a key chip.
    fn search_field(
        &self,
        t: Theme,
        m: Metrics,
        field: &orbit_core::Field,
        focused: bool,
        moving: Option<&str>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let layout = self.layout.clone();
        let text = field.text();
        let caret = field.caret();
        let (label, placeholder): (SharedString, SharedString) = match moving {
            Some(app) => (
                format!("Move {app} to a workspace").into(),
                "Type to filter workspaces…".into(),
            ),
            None => ("Search".into(), SEARCH_PLACEHOLDER.into()),
        };
        let content = if text.is_empty() {
            div()
                .flex()
                .items_center()
                .when(focused, |d| d.child(caret_bar(t)))
                .child(div().text_color(t.muted.rgba()).child(placeholder.clone()))
        } else {
            div()
                .flex()
                .items_center()
                .child(text[..caret].to_owned())
                .when(focused, |d| d.child(caret_bar(t)))
                .child(text[caret..].to_owned())
        };
        div()
            .id("search")
            .role(Role::SearchInput)
            .aria_label(label.clone())
            .aria_value(SharedString::from(text.to_owned()))
            .aria_placeholder(placeholder)
            .aria_keyshortcuts("Control+K")
            .when(focused, |d| d.aria_active_descendant())
            .absolute()
            .left(px(m.search.x))
            .top(px(m.search.y))
            .w(px(m.search.w))
            .h(px(m.search.h))
            .flex()
            .items_center()
            .gap(px(14.))
            .px(px(20.))
            .rounded(px(18.))
            .bg(t.canvas.alpha(t.bar_alpha))
            .border(hair(m))
            .border_color(hairline(t))
            .shadow(vec![soft_shadow(0.3)])
            .when(focused, |d| {
                d.border(ring(m))
                    .border_color(t.focus.rgba())
                    .shadow(vec![glow(t.focus, GLOW_ALPHA, GLOW_BLUR)])
            })
            .text_size(px(17.))
            .text_color(t.text.rgba())
            .cursor_text()
            .child(search_icon(if focused { t.focus } else { t.muted }))
            .when_some(moving.map(|_| label), |d, label| {
                d.child(
                    div()
                        .flex_none()
                        .text_color(t.strong.rgba())
                        .font_weight(FontWeight::MEDIUM)
                        .child(label),
                )
            })
            .child(div().flex_1().overflow_hidden().child(content))
            .child(keycap(t, m, "Ctrl K"))
            .on_click(cx.listener(|this, _, _, cx| {
                this.with_overview(cx, |o, _| o.focus_search());
            }))
            .child(
                canvas(
                    move |bounds, _, _| layout.borrow_mut().search = Some(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }

    /// A workspace in the rail: the desktop composited small (the frosted
    /// backdrop with the workspace's windows placed from their real
    /// geometry, filled with captured previews where there are any) and
    /// its number below. Counts and states live in the accessible label
    /// and in a caption shown only for the focused tile.
    #[allow(clippy::too_many_arguments)]
    fn tile(
        &self,
        t: Theme,
        m: Metrics,
        tile: Tile,
        rect: Rect,
        composite: Composite,
        at: usize,
        of: usize,
        moving: bool,
        keyboard: bool,
        first: usize,
        reorder: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let dest = tile.dest;
        let dragged = match dest {
            Dest::Existing(id) if reorder && id.0 > 0 => Some(DraggedWorkspace {
                id,
                epoch: self.desk.read(cx).store().epoch(),
                label: tile.label.clone(),
                composite: composite.clone(),
            }),
            _ => None,
        };
        let weak = cx.weak_entity();
        let swap_target = self
            .workspace_drop
            .filter(|(target, _)| *target == dest)
            .map(|(_, after)| after);
        let layout = self.layout.clone();
        let chosen = moving && tile.focused;
        let marked = chosen || (tile.previewed && !moving);
        let focused = tile.focused && keyboard;
        let ring_color = if focused || swap_target.is_some() {
            Some(t.focus)
        } else if marked {
            Some(t.selection)
        } else {
            None
        };
        let frame = 2. * f32::from(ring(m) + hair(m));
        let (inner_w, inner_h) = (THUMB_WIDTH - frame, THUMB_HEIGHT - frame);
        let thumb = div()
            .relative()
            .w(px(inner_w))
            .h(px(inner_h))
            .rounded(px(10.) - ring(m))
            .overflow_hidden()
            .bg(t.surface.rgba())
            .border(hair(m))
            .border_color(if marked && t.ring_edge_alpha > 0. {
                theme::WHITE.alpha(t.ring_edge_alpha)
            } else if tile.is_new() {
                t.muted.alpha(0.6)
            } else {
                hairline(t)
            })
            .when_some(composite.backdrop.clone(), |d, image| {
                d.child(
                    img(image)
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .object_fit(ObjectFit::Cover),
                )
            })
            // A light tint so the small windows read on top.
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(t.canvas.alpha(0.25)),
            )
            .children(composite.windows.iter().map(|w| {
                let scale_x = inner_w / composite.monitor.0.max(1.);
                let scale_y = inner_h / composite.monitor.1.max(1.);
                div()
                    .absolute()
                    .left(px((w.rect.x * scale_x).round()))
                    .top(px((w.rect.y * scale_y).round()))
                    .w(px((w.rect.w * scale_x).round().max(2.)))
                    .h(px((w.rect.h * scale_y).round().max(2.)))
                    .rounded(px(2.))
                    .overflow_hidden()
                    .bg(t.surface.alpha(0.92))
                    .border(hair(m))
                    .border_color(theme::WHITE.alpha(0.18))
                    .when_some(w.image.clone(), |d, image| {
                        d.child(img(image).size_full().object_fit(ObjectFit::Fill))
                    })
                    .into_any_element()
            }))
            .when(tile.is_new(), |d| {
                d.flex().items_center().justify_center().child(
                    div()
                        .text_size(px(28.))
                        .text_color(t.text.rgba())
                        .child("+"),
                )
            })
            .when(chosen, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full()
                        .bg(t.selection.alpha(0.35)),
                )
            });
        let name = if tile.is_new() {
            "New".to_string()
        } else {
            tile.label.clone()
        };
        let caption = (tile.focused && keyboard && !tile.is_new()).then(|| tile.caption());
        div()
            .id(SharedString::from(format!("tile-{dest:?}")))
            .role(Role::Button)
            .aria_label(tile.accessible_label())
            .aria_selected(if moving { tile.focused } else { tile.previewed })
            .aria_position_in_set(at + 1)
            .aria_size_of_set(of)
            .when(tile.focused, |d| d.aria_active_descendant())
            .absolute()
            .left(px(rect.x))
            .top(px(rect.y))
            .w(px(rect.w))
            .h(px(rect.h))
            .flex()
            .flex_col()
            .items_center()
            .cursor_pointer()
            .when(
                self.workspace_drag
                    .as_ref()
                    .is_some_and(|d| Dest::Existing(d.id) == dest),
                |d| d.opacity(0.45),
            )
            .when_some(dragged, |d, dragged| {
                d.on_drag(dragged, move |dragged, offset, window, cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.start_workspace_drag(dragged.clone(), first, window, cx)
                    });
                    cx.new(|_| WorkspaceGhost {
                        dragged: dragged.clone(),
                        offset,
                    })
                })
            })
            .child(div().w(px(THUMB_WIDTH)).h(px(THUMB_HEIGHT)).child(ring_box(
                t,
                m,
                ring_color,
                marked || focused || swap_target.is_some(),
                10.,
                thumb,
            )))
            .child(
                div()
                    .mt(px(8.))
                    .h(px(THUMB_LABEL))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .text_size(px(15.))
                    .text_color(
                        if tile.active || marked {
                            t.strong
                        } else {
                            t.text
                        }
                        .rgba(),
                    )
                    .font_weight(if tile.active || marked {
                        FontWeight::SEMIBOLD
                    } else {
                        FontWeight::MEDIUM
                    })
                    .child(name)
                    // The active workspace also gets a shape, not only words.
                    .when(tile.active, |d| {
                        d.child(div().size(px(6.)).rounded_full().bg(t.focus.rgba()))
                    }),
            )
            .child(
                div()
                    .h(px(THUMB_CAPTION))
                    .text_size(px(12.))
                    .text_color(t.text.rgba())
                    .max_w(px(THUMB_WIDTH))
                    .truncate()
                    .children(caption),
            )
            .drag_over::<DraggedWindow>(move |style, _, _, _| {
                style.bg(t.selection.alpha(0.25)).rounded(px(14.))
            })
            .on_drop(
                cx.listener(move |this, dragged: &DraggedWindow, window, cx| {
                    let follow = window.modifiers().shift != Look::get(cx).prefs.follow_on_move;
                    let intent =
                        this.with_overview(cx, |o, m| o.drop_on(m, dragged.window, dest, follow));
                    this.perform(intent, window, cx);
                }),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.perform(Some(Intent::Switch(dest)), window, cx);
            }))
            .child(
                canvas(
                    move |bounds, _, _| {
                        layout.borrow_mut().tiles.insert(dest, bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }

    fn icons(&self, cx: &App) -> Arc<crate::icons::Icons> {
        Look::get(cx).icons
    }

    /// A window card at its grid rectangle: the bare preview image at its
    /// captured device size, rounded, with a soft shadow, and the app icon
    /// and name under it. Selection is one Violet ring; keyboard focus on
    /// it adds the small glow. On open the preview flies in from the
    /// window's real rectangle (`from`), and on close back to it.
    #[allow(clippy::too_many_arguments)]
    fn card(
        &self,
        t: Theme,
        m: Metrics,
        card: Card,
        rect: Rect,
        from: Option<Rect>,
        motion: Motion,
        at: usize,
        of: usize,
        keyboard: bool,
        searching: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = card.window;
        let address = id.address;
        let layout = self.layout.clone();
        let desk = self.desk.read(cx);
        let preview = desk.preview(address).map(|p| (p.image.clone(), p.size));
        let aspect =
            (card.size.0 > 0 && card.size.1 > 0).then(|| card.size.0 as f32 / card.size.1 as f32);
        let focused = card.selected && keyboard;
        let dragged = DraggedWindow {
            window: id,
            app: card.app.clone().into(),
            preview: preview.clone(),
            size: (card.size.0.max(0) as f32, card.size.1.max(0) as f32),
        };
        let ghost_layout = self.layout.clone();
        let preview_label: SharedString = if preview.is_some() {
            format!("Preview of {}", card.app).into()
        } else {
            "Preview unavailable".into()
        };
        // Selection is the guide's Violet ring; keyboard focus on it adds
        // the glow (the focus color is for the rail and search).
        let ring_color = card.selected.then_some(t.selection);
        let icons = self.icons(cx);
        // The image, or a placeholder slot while no frame has arrived.
        let frame = 2 * (RING_DEVICE + 1);
        let boxed = m.preview_boxes().card;
        let (image_w, image_h) = match (&preview, aspect) {
            (Some((_, size)), _) => (size.0, size.1),
            (None, Some(aspect)) => fit_box(aspect, boxed),
            (None, None) => boxed,
        };
        let picture = div()
            .id("preview")
            .role(Role::Image)
            .aria_label(preview_label.clone())
            .relative()
            .size_full()
            .rounded(px(PREVIEW_RADIUS) - ring(m))
            .overflow_hidden()
            .border(hair(m))
            .border_color(if card.selected && t.ring_edge_alpha > 0. {
                theme::WHITE.alpha(t.ring_edge_alpha)
            } else {
                theme::WHITE.alpha(t.hairline_alpha * 0.6)
            })
            .when(preview.is_none(), |d| {
                d.bg(t.surface.alpha(glass(t, 0.85)))
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .items_center()
                    .justify_center()
                    .child(app_icon(&icons, &card.class, &card.app, 44., &m))
                    .child(
                        div()
                            .text_color(t.text.rgba())
                            .text_size(px(13.))
                            .child(preview_label),
                    )
            })
            .when_some(preview.clone(), |d, (image, size)| {
                let layout = layout.clone();
                d.child(img(image).size_full().object_fit(ObjectFit::Fill))
                    .child(
                        canvas(
                            move |bounds, _, _| {
                                // Absolute children sit inside the border,
                                // so these are the image's own bounds.
                                layout
                                    .borrow_mut()
                                    .previews
                                    .insert(address, (bounds, size, aspect));
                            },
                            |_, _, _, _| {},
                        )
                        .absolute()
                        .top_0()
                        .left_0()
                        .size_full(),
                    )
            });
        // Second label line: the title on the selected card, or where a
        // search result lives.
        let titles = Look::get(cx).prefs.titles;
        let second: Option<String> = if searching {
            Some(match &card.monitor {
                Some(monitor) => format!("Workspace {} · {monitor}", card.workspace),
                None => format!("Workspace {}", card.workspace),
            })
        } else if !card.title.is_empty() && (titles == 2 || (titles == 1 && card.selected)) {
            Some(card.title.clone())
        } else {
            None
        };
        // Where the ring box (image plus ring and edge) rests in the slot,
        // centered on whole device pixels.
        let ring_w = image_w + frame;
        let ring_h = image_h + frame;
        let rest = Rect {
            x: rect.x + m.logical(m.device(m.card).saturating_sub(ring_w) / 2),
            y: rect.y + m.logical(m.device(m.slot).saturating_sub(ring_h) / 2),
            w: m.logical(ring_w),
            h: m.logical(ring_h),
        };
        let start = from.unwrap_or(rest);
        let (box_rect, label_opacity) = match motion {
            Motion::Settled => (rest, 1.),
            Motion::Entering(delta) => (Rect::lerp(start, rest, delta), delta),
            Motion::Leaving(delta) => (Rect::lerp(rest, start, delta), 1. - delta),
        };
        div()
            .id(SharedString::from(format!("card-{address:x}")))
            .role(Role::Button)
            .aria_label(card.accessible_label())
            .aria_selected(card.selected)
            .aria_position_in_set(at + 1)
            .aria_size_of_set(of)
            .when(focused, |d| d.aria_active_descendant())
            .absolute()
            .left(px(rect.x))
            .top(px(rect.y))
            .w(px(rect.w))
            .h(px(rect.h))
            .cursor_pointer()
            .child(
                div()
                    .absolute()
                    .left(px(box_rect.x - rect.x))
                    .top(px(box_rect.y - rect.y))
                    .w(px(box_rect.w))
                    .h(px(box_rect.h))
                    .child(ring_box(t, m, ring_color, focused, PREVIEW_RADIUS, picture)),
            )
            // The label sits under the image's left edge, as the hero's do.
            .child(
                div()
                    .absolute()
                    .left(px(rest.x - rect.x))
                    .top(px(m.slot + LABEL_GAP))
                    .w(px(rest.w))
                    .h(px(LABEL_HEIGHT))
                    .px(px(4.))
                    .opacity(label_opacity)
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .child(app_icon(&icons, &card.class, &card.app, ICON, &m))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .flex()
                            .flex_col()
                            .child(
                                div()
                                    .text_size(px(18.))
                                    .line_height(px(22.))
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(t.strong.rgba())
                                    .truncate()
                                    .child(card.app.clone()),
                            )
                            .children(second.map(|line| {
                                div()
                                    .text_size(px(13.))
                                    .line_height(px(16.))
                                    .text_color(t.text.rgba())
                                    .truncate()
                                    .child(line)
                            })),
                    ),
            )
            .on_click(cx.listener(move |this, _, window, cx| {
                this.perform(Some(Intent::Activate(id)), window, cx);
            }))
            .on_drag(dragged, move |dragged: &DraggedWindow, offset, _, cx| {
                let reduced_motion = Look::get(cx).reduced_motion;
                ghost_layout.borrow_mut().ghost = None;
                let layout = ghost_layout.clone();
                cx.new(|_| Ghost {
                    app: dragged.app.clone(),
                    preview: dragged.preview.clone(),
                    offset,
                    theme: t,
                    size: dragged.size,
                    layout,
                    reduced_motion,
                    displayed: 0.,
                    last_frame: Instant::now(),
                })
            })
            .child(
                canvas(
                    move |bounds, _, _| {
                        layout.borrow_mut().cards.insert(address, bounds);
                    },
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
    }
}

/// The largest size with the given aspect (w / h) inside `max`.
fn fit_box(aspect: f32, max: (u32, u32)) -> (u32, u32) {
    let (w, h) = (max.0.max(1) as f32, max.1.max(1) as f32);
    if w / h > aspect {
        ((h * aspect).round().max(1.) as u32, max.1.max(1))
    } else {
        (max.0.max(1), (w / aspect).round().max(1.) as u32)
    }
}

/// Where a card is in its entry or exit flight.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Motion {
    Settled,
    Entering(f32),
    Leaving(f32),
}

/// Text input: typing anywhere goes to the search field (or the move
/// menu's filter), with input-method composition.
impl EntityInputHandler for OverviewView {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        adjusted_range: &mut Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<String> {
        let (text, range) = self.editing(cx, |f| f.text_utf16(range))?;
        *adjusted_range = Some(range);
        Some(text)
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let caret = self.editing(cx, |f| f.caret_utf16())?;
        Some(UTF16Selection {
            range: caret..caret,
            reversed: false,
        })
    }

    fn marked_text_range(&self, _: &mut Window, cx: &mut Context<Self>) -> Option<Range<usize>> {
        self.editing(cx, |f| f.marked_utf16()).flatten()
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.with_overview(cx, |o, m| o.edit(m, |f| f.unmark()));
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        self.with_overview(cx, |o, m| o.edit(m, |f| f.replace_utf16(range, &text)));
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text: String = text.chars().filter(|c| !c.is_control()).collect();
        self.with_overview(cx, |o, m| {
            o.edit(m, |f| f.compose_utf16(range, &text, selected))
        });
    }

    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // Candidate windows open by the search field.
        self.layout.borrow().search
    }

    fn character_index_for_point(
        &mut self,
        _: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }

    fn set_selected_text_range(
        &mut self,
        range: Range<usize>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.with_overview(cx, |o, m| o.edit(m, |f| f.set_caret_utf16(range.end)));
    }

    fn text_length_utf16(&mut self, _: &mut Window, cx: &mut Context<Self>) -> Option<usize> {
        self.editing(cx, |f| f.len_utf16())
    }
}

/// A keystroke that types `c`, as a keyboard would send it.
fn typed(c: char) -> Keystroke {
    let key = match c {
        ' ' => "space".to_string(),
        c => c.to_lowercase().collect(),
    };
    Keystroke {
        modifiers: Modifiers {
            shift: c.is_uppercase(),
            ..Modifiers::default()
        },
        key,
        key_char: Some(c.to_string()),
    }
}

/// The accessibility tree of the last frame, reduced to what scripts can
/// check without reading any label (labels hold window titles).
fn a11y_summary(window: &Window) -> serde_json::Value {
    use serde_json::{Value, json};
    let Some(tree) = window
        .debug_a11y_tree_json()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
    else {
        // AccessKit builds the tree only while assistive technology listens.
        return json!({ "active": window.is_a11y_active(), "tree": false });
    };
    let nodes = tree["nodes"].as_object().cloned().unwrap_or_default();
    let mut roles: HashMap<String, usize> = HashMap::new();
    let mut unlabelled = 0;
    for node in nodes.values() {
        let role = node["aria"]["role"].as_str().unwrap_or("").to_owned();
        let labelled = node["aria"]["label"]
            .as_str()
            .is_some_and(|l| !l.is_empty());
        if matches!(
            role.as_str(),
            "Button" | "SearchInput" | "Image" | "List" | "Status" | "Dialog"
        ) && !labelled
        {
            unlabelled += 1;
        }
        *roles.entry(role).or_default() += 1;
    }
    let role_of = |key: &Value| {
        key.as_str()
            .and_then(|k| nodes.get(k))
            .and_then(|n| n["aria"]["role"].as_str())
            .map(str::to_owned)
    };
    json!({
        "active": true,
        "tree": true,
        "nodes": nodes.len(),
        "roles": roles,
        "unlabelled": unlabelled,
        "focus": role_of(&tree["gpui_focus"]),
        "active_descendant": role_of(&tree["active_descendant_focus"]),
    })
}

/// What a rail tile composites: the backdrop and the workspace's windows
/// in monitor-relative logical coordinates, with cached previews.
#[derive(Clone)]
struct Composite {
    backdrop: Option<Arc<RenderImage>>,
    monitor: (f32, f32),
    windows: Vec<SmallWindow>,
}

#[derive(Clone)]
struct SmallWindow {
    rect: Rect,
    image: Option<Arc<RenderImage>>,
}

/// A window's rectangle relative to the overview's monitor, if the model
/// knows its geometry.
fn local_rect(model: &Model, monitor: Option<&str>, w: &orbit_core::Window) -> Option<Rect> {
    if w.size.0 <= 0 || w.size.1 <= 0 {
        return None;
    }
    let (mx, my) = monitor
        .and_then(|name| model.monitors.iter().find(|m| m.name == name))
        .map_or((0., 0.), |m| (m.x as f32, m.y as f32));
    Some(Rect {
        x: w.at.0 as f32 - mx,
        y: w.at.1 as f32 - my,
        w: w.size.0 as f32,
        h: w.size.1 as f32,
    })
}

/// Layer opacity in the theme: glass surfaces stay opaque in opaque themes.
fn glass(t: Theme, alpha: f32) -> f32 {
    if t.is_opaque() { 1.0 } else { alpha }
}

/// The thin light edge glass surfaces get in the guide.
fn hairline(t: Theme) -> Rgba {
    theme::WHITE.alpha(t.hairline_alpha)
}

/// One device pixel, in logical px.
fn hair(m: Metrics) -> Pixels {
    px(1. / m.scale)
}

/// The selection ring's width, in logical px.
fn ring(m: Metrics) -> Pixels {
    px(RING_DEVICE as f32 / m.scale)
}

/// A soft colored glow around selected and focused elements.
fn glow(color: Token, alpha: f32, blur: f32) -> BoxShadow {
    BoxShadow::new(px(0.), px(0.), Hsla::from(color.alpha(alpha))).blur_radius(px(blur))
}

/// The drop shadow previews and bars cast on the glass.
fn soft_shadow(alpha: f32) -> BoxShadow {
    BoxShadow::new(px(0.), px(12.), gpui::black().opacity(alpha)).blur_radius(px(40.))
}

/// The one selection treatment: a ring in `color` (none: a transparent
/// ring of the same width, so nothing moves), the guide's small glow when
/// `glowing`, plus the preview's drop shadow. The child is the element
/// inside it.
fn ring_box(
    t: Theme,
    m: Metrics,
    color: Option<Token>,
    glowing: bool,
    radius: f32,
    child: impl IntoElement,
) -> impl IntoElement {
    let shadow = soft_shadow(0.45);
    div()
        .size_full()
        .rounded(px(radius))
        .border(ring(m))
        .border_color(match color {
            Some(color) => color.rgba(),
            None => gpui::transparent_black().into(),
        })
        .shadow(match color {
            Some(color) if glowing => vec![shadow, glow(color, GLOW_ALPHA, GLOW_BLUR)],
            _ => vec![shadow],
        })
        .when(color.is_none() && !t.is_opaque(), |d| {
            // Hover: the edge brightens (the ring stays transparent).
            d.hover(|s| s.border_color(theme::WHITE.alpha(0.2)))
        })
        .child(child)
}

fn caret_bar(t: Theme) -> impl IntoElement {
    div().w(px(2.)).h(px(20.)).mx(px(1.)).bg(t.focus.rgba())
}

/// A small keyboard key label.
fn keycap(t: Theme, m: Metrics, keys: &str) -> impl IntoElement {
    div()
        .flex_none()
        .px(px(8.))
        .py(px(3.))
        .rounded(px(10.))
        .bg(theme::WHITE.alpha(0.08))
        .border(hair(m))
        .border_color(hairline(t))
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.muted.rgba())
        .child(keys.to_owned())
}

/// A magnifier drawn from two shapes, 2 px strokes with rounded ends.
fn search_icon(color: Token) -> impl IntoElement {
    div()
        .relative()
        .flex_none()
        .size(px(22.))
        .child(
            div()
                .absolute()
                .top_0()
                .left_0()
                .size(px(16.))
                .rounded_full()
                .border_2()
                .border_color(color.rgba()),
        )
        .child(
            div()
                .absolute()
                .top(px(14.))
                .left(px(14.))
                .size(px(6.))
                .rounded(px(2.))
                .bg(color.rgba()),
        )
}

/// The app's icon from its desktop entry and the icon theme; when there
/// is none, its initial on a Gray rounded square (the brand accents stay
/// for state). The letter keeps 4.5:1 on its square.
fn app_icon(
    icons: &crate::icons::Icons,
    class: &str,
    app: &str,
    size: f32,
    m: &Metrics,
) -> gpui::AnyElement {
    if let Some(image) = icons.icon_for(class) {
        return img(image)
            .flex_none()
            .size(px(size))
            .rounded(px(round4(size * 0.22).max(4.) - 2.))
            .object_fit(ObjectFit::Contain)
            .into_any_element();
    }
    let initial: String = app
        .chars()
        .next()
        .map(|c| c.to_string())
        .unwrap_or_default();
    div()
        .flex_none()
        .size(px(size))
        .rounded(px(size * 0.28))
        .bg(linear_gradient(
            160.,
            linear_color_stop(theme::GRAY.rgba(), 0.),
            linear_color_stop(theme::SLATE.rgba(), 1.),
        ))
        .border(hair(*m))
        .border_color(theme::WHITE.alpha(0.12))
        .text_color(theme::CLOUD.rgba())
        .text_size(px(size * 0.5))
        .font_weight(FontWeight::BOLD)
        .flex()
        .items_center()
        .justify_center()
        .child(initial)
        .into_any_element()
}

fn click(window: &mut Window, cx: &mut App, at: Point<Pixels>) {
    move_to(window, cx, at, None, Modifiers::default());
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }),
        cx,
    );
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position: at,
            modifiers: Modifiers::default(),
            click_count: 1,
        }),
        cx,
    );
}

fn move_to(
    window: &mut Window,
    cx: &mut App,
    at: Point<Pixels>,
    pressed: Option<MouseButton>,
    modifiers: Modifiers,
) {
    window.dispatch_event(
        PlatformInput::MouseMove(MouseMoveEvent {
            position: at,
            pressed_button: pressed,
            modifiers,
        }),
        cx,
    );
}

fn drag(window: &mut Window, cx: &mut App, from: Point<Pixels>, to: Point<Pixels>, shift: bool) {
    let modifiers = Modifiers {
        shift,
        ..Modifiers::default()
    };
    start_drag(window, cx, from, to, modifiers);
    release_drag(window, cx, to, modifiers);
}

fn start_drag(
    window: &mut Window,
    cx: &mut App,
    from: Point<Pixels>,
    to: Point<Pixels>,
    modifiers: Modifiers,
) {
    move_to(window, cx, from, None, Modifiers::default());
    window.dispatch_event(
        PlatformInput::MouseDown(MouseDownEvent {
            button: MouseButton::Left,
            position: from,
            modifiers: Modifiers::default(),
            click_count: 1,
            first_mouse: false,
        }),
        cx,
    );
    for step in 1..=8 {
        let t = step as f32 / 8.;
        let at = point(from.x + (to.x - from.x) * t, from.y + (to.y - from.y) * t);
        move_to(window, cx, at, Some(MouseButton::Left), modifiers);
    }
}

fn release_drag(window: &mut Window, cx: &mut App, to: Point<Pixels>, modifiers: Modifiers) {
    window.dispatch_event(
        PlatformInput::MouseUp(MouseUpEvent {
            button: MouseButton::Left,
            position: to,
            modifiers,
            click_count: 1,
        }),
        cx,
    );
}

/// What the grid shows when there are no cards: centered in the grid area,
/// with a dashed card-sized outline so the layout keeps its shape.
fn empty_state(t: Theme, m: Metrics, title: String, hint: &str) -> impl IntoElement {
    let slot = m.card_rect(0);
    div()
        .id("empty")
        .role(Role::Status)
        .aria_label(SharedString::from(format!("{title}. {hint}")))
        .absolute()
        .left(px(m.grid.x))
        .top(px(m.grid.y))
        .w(px(m.grid.w))
        .h(px(m.grid.h))
        .flex()
        .flex_col()
        .items_center()
        .justify_center()
        .gap(px(8.))
        .child(
            div()
                .absolute()
                .left(px(slot.x - m.grid.x))
                .top(px(slot.y - m.grid.y))
                .w(px(m.card))
                .h(px(m.slot))
                .rounded(px(PREVIEW_RADIUS))
                .border(hair(m))
                .border_dashed()
                .border_color(theme::WHITE.alpha(0.2)),
        )
        .child(
            div()
                .text_size(px(18.))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(t.strong.rgba())
                .child(title),
        )
        .child(
            div()
                .text_size(px(13.))
                .text_color(t.text.rgba())
                .child(hint.to_owned()),
        )
}

impl Render for OverviewView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(first_frame) = self.first_frame.take() {
            first_frame();
        }
        let look = Look::get(cx);
        let t = look.theme;
        let viewport: Size<Pixels> = window.viewport_size();
        let (card_count, tile_count) = {
            let desk = self.desk.read(cx);
            let model = desk.store().model();
            desk.overview()
                .map_or((0, 0), |o| (o.cards(model).len(), o.tiles(model).len()))
        };
        let prefs = look.prefs.clone();
        let m = Metrics::for_output_with(
            f32::from(viewport.width),
            f32::from(viewport.height),
            window.scale_factor(),
            card_count,
            tile_count,
            prefs.columns,
            prefs.rail_at_bottom,
        );
        // Captures are sized to what this frame draws.
        let boxes = m.preview_boxes();
        self.desk
            .update(cx, |desk, _| desk.set_preview_boxes(boxes));
        let desk = self.desk.read(cx);
        let model = desk.store().model();
        let Some(o) = desk.overview() else {
            return div().size_full().bg(t.canvas.rgba()).into_any_element();
        };
        let backdrop = desk.backdrop().cloned();
        {
            let mut l = self.layout.borrow_mut();
            l.metrics = Some(m);
            l.backdrop = backdrop.is_some();
            l.hints = self.show_hints;
            l.cards.clear();
            l.tiles.clear();
            l.previews.clear();
        }
        let tiles = o.tiles(model);
        let cards = o.cards(model);
        let region = o.region;
        let searching = o.searching();
        let field = o.editing().clone();
        let announcement = o.announcement.clone();
        let moving_app = o
            .moving
            .and_then(|id| model.window(id))
            .map(|w| orbit_core::app_name(&w.class));
        let previewed_label = o
            .previewed
            .and_then(|id| model.workspaces.get(&id))
            .map(|w| orbit_core::overview::label(&w.name));
        let heading = if searching {
            "Search results".to_string()
        } else {
            previewed_label
                .as_ref()
                .map(|l| format!("Workspace {l}"))
                .unwrap_or_default()
        };
        let status = match &desk.status {
            Link::Live { .. } if desk.is_live() => None,
            Link::Live { .. } | Link::Connecting => Some("Connecting to Hyprland…".to_string()),
            Link::Lost { .. } => Some("Reconnecting to Hyprland…".to_string()),
        };
        let loading = model.windows.is_empty() && model.workspaces.is_empty();
        let hints: Vec<(&str, &str)> = if moving_app.is_some() {
            vec![
                ("←→", "choose"),
                ("type", "filter"),
                ("Enter", "move"),
                ("Shift Enter", "move and follow"),
                ("Esc", "cancel"),
            ]
        } else {
            match region {
                Region::Search => vec![
                    ("Enter", "open best match"),
                    ("↓", "workspaces"),
                    ("Tab", "next"),
                    ("Esc", if field.is_empty() { "close" } else { "clear" }),
                ],
                Region::Rail => vec![
                    ("←→", "preview"),
                    ("Ctrl Shift ←→", "swap workspaces"),
                    ("Enter", "switch"),
                    ("Tab", "windows"),
                    ("Esc", "close"),
                ],
                Region::Grid => vec![
                    ("Arrows", "select"),
                    ("Enter", "open"),
                    ("Ctrl Shift M", "move"),
                    ("Ctrl K", "search"),
                    ("Esc", "close"),
                ],
            }
        };
        let keyboard_grid = region == Region::Grid;
        let keyboard_rail = region == Region::Rail;
        let rail_count = tiles.len();
        let grid_label: SharedString = if searching {
            "Search results".into()
        } else {
            format!(
                "Windows on workspace {}",
                previewed_label.clone().unwrap_or_default()
            )
            .into()
        };
        let view = cx.entity();
        let focus = self.focus_handle.clone();
        let reduced_motion = look.reduced_motion;
        let monitor = o.invocation.monitor.clone();
        let monitor_size = monitor
            .as_deref()
            .and_then(|name| model.monitors.iter().find(|m| m.name == name))
            .map_or(m.viewport, |mon| (mon.width as f32, mon.height as f32));
        // Windows fly only from where they really are: the invocation
        // workspace's windows on open.
        let flying_from = (!searching && o.previewed == o.invocation.workspace)
            .then_some(o.previewed)
            .flatten();

        // Motion: the glass fades in; cards fly in on open and back out on
        // close (fade only with reduced motion).
        let elapsed = |since: Instant| {
            (since.elapsed().as_secs_f32() / (FLY_MS as f32 / 1000.)).clamp(0., 1.)
        };
        let ease = ease_out_quint();
        let (motion, surface_opacity) = match self.closing {
            Some(started) => {
                let d = ease(elapsed(started));
                (
                    if reduced_motion {
                        Motion::Settled
                    } else {
                        Motion::Leaving(d)
                    },
                    1. - d,
                )
            }
            None => {
                let d = ease(elapsed(self.opened_at));
                (
                    if reduced_motion || d >= 1. {
                        Motion::Settled
                    } else {
                        Motion::Entering(d)
                    },
                    d,
                )
            }
        };
        if surface_opacity < 1. || self.closing.is_some() {
            // Keep frames coming while something moves.
            window.request_animation_frame();
        }

        let empty = if card_count > 0 {
            None
        } else if loading {
            Some(empty_state(
                t,
                m,
                status.clone().unwrap_or_else(|| "Loading…".into()),
                "Windows appear as soon as Hyprland answers. Esc closes.",
            ))
        } else if searching {
            Some(empty_state(
                t,
                m,
                if rail_count > 0 {
                    "No windows match".into()
                } else {
                    "No windows or workspaces match".into()
                },
                if rail_count > 0 {
                    "Press ↑ to choose a matching workspace, or Esc to clear the search."
                } else {
                    "Check the spelling, or press Esc to clear the search."
                },
            ))
        } else {
            Some(empty_state(
                t,
                m,
                format!("No windows on {heading}"),
                "Tab to the workspaces to look elsewhere, or drag a window here.",
            ))
        };

        // The rail strip: the previewed (or keyboard-focused) tile stays in
        // view; the rest scroll with the arrows.
        let shown_index = tiles
            .iter()
            .position(|tile| {
                if keyboard_rail || moving_app.is_some() {
                    tile.focused
                } else {
                    tile.previewed
                }
            })
            .unwrap_or(0);
        let first = self
            .rail_drag_first
            .unwrap_or_else(|| m.rail_first(shown_index))
            .min(rail_count.saturating_sub(m.rail_fit));
        let reorder = moving_app.is_none()
            && !o.swapping
            && !searching
            && matches!(desk.status, Link::Live { .. });
        let composites: Vec<Composite> = tiles
            .iter()
            .map(|tile| {
                let windows = match tile.dest {
                    Dest::Existing(ws) => model
                        .windows_on(ws)
                        .filter(|w| !w.hidden)
                        .filter_map(|w| {
                            Some(SmallWindow {
                                rect: local_rect(model, monitor.as_deref(), w)?,
                                image: desk.preview(w.id.address).map(|p| p.image.clone()),
                            })
                        })
                        .collect(),
                    Dest::NewNumbered(_) => Vec::new(),
                };
                Composite {
                    backdrop: backdrop.clone(),
                    monitor: monitor_size,
                    windows,
                }
            })
            .collect();
        // Where each card flies from, resolved before the closures below
        // (they need the context the model is read through).
        let froms: Vec<Option<Rect>> = cards
            .iter()
            .map(|c| {
                flying_from
                    .filter(|ws| model.window(c.window).is_some_and(|w| w.workspace == *ws))
                    .and_then(|_| model.window(c.window))
                    .and_then(|w| local_rect(model, monitor.as_deref(), w))
            })
            .collect();
        let rail = div()
            .id("rail")
            .role(Role::List)
            .aria_label(if moving_app.is_some() {
                "Destinations"
            } else if searching {
                "Matching workspaces"
            } else {
                "Workspaces"
            })
            .absolute()
            .left(px(m.rail.x))
            .top(px(m.rail.y))
            .w(px(m.rail.w))
            .h(px(m.rail.h))
            .overflow_hidden()
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<DraggedWorkspace>, _, cx| {
                    let target = this.workspace_target(event.drag(cx), event.event.position, cx);
                    if target != this.workspace_drop {
                        this.workspace_drop = target;
                        cx.notify();
                    }
                }),
            )
            .on_drop(cx.listener(|this, dragged: &DraggedWorkspace, window, cx| {
                let target = this.workspace_target(dragged, window.mouse_position(), cx);
                let intent = target.and_then(|(dest, after)| {
                    this.with_overview(cx, |o, m| o.reorder_workspace(m, dragged.id, dest, after))
                });
                this.clear_workspace_drag(cx);
                this.perform(intent, window, cx);
            }))
            .children(
                tiles
                    .into_iter()
                    .zip(composites)
                    .enumerate()
                    .filter(|(i, _)| *i >= first && *i < first + m.rail_fit)
                    .map(|(i, (tile, composite))| {
                        let rect = m.tile_rect(i, first);
                        let rect = Rect {
                            x: rect.x - m.rail.x,
                            y: 0.,
                            ..rect
                        };
                        self.tile(
                            t,
                            m,
                            tile,
                            rect,
                            composite,
                            i,
                            rail_count,
                            moving_app.is_some(),
                            keyboard_rail,
                            first,
                            reorder,
                            cx,
                        )
                        .into_any_element()
                    }),
            );

        let grid = div()
            .id("grid")
            .role(Role::List)
            .aria_label(grid_label)
            .absolute()
            .left_0()
            .top_0()
            .size_full()
            .children(
                cards
                    .into_iter()
                    .zip(froms)
                    .enumerate()
                    .map(|(i, (c, from))| {
                        let rect = m.card_rect(i);
                        self.card(
                            t,
                            m,
                            c,
                            rect,
                            from,
                            motion,
                            i,
                            card_count,
                            keyboard_grid,
                            searching,
                            cx,
                        )
                        .into_any_element()
                    }),
            )
            .children(empty);

        // The live status region (always present for assistive technology,
        // which reads every announcement); visible only as a toast for
        // failures and the compositor link, so nothing else sits on the
        // glass that the hero does not show.
        let spoken = announcement
            .as_ref()
            .map(|a| a.text.clone())
            .or(status.clone());
        let toast_text = announcement
            .as_ref()
            .filter(|a| a.tone == Tone::Error)
            .map(|a| a.text.clone())
            .or(status.clone());
        let status_line = div()
            .id("status")
            .role(Role::Status)
            .aria_label(SharedString::from(spoken.unwrap_or_else(|| "Ready".into())))
            .a11y_synthetic_children(|builder| {
                builder.parent_node().set_live(accesskit::Live::Polite);
            })
            .absolute()
            .left(px(m.search.x))
            .w(px(m.search.w))
            .top(px(m.viewport.1 - PAD_BOTTOM - BAR_HEIGHT))
            .h(px(BAR_HEIGHT))
            .flex()
            .items_center()
            .justify_center()
            .when(toast_text.is_none(), |d| d.opacity(0.))
            .child(
                div()
                    .px(px(18.))
                    .h(px(BAR_HEIGHT))
                    .rounded_full()
                    .bg(t.canvas.alpha(t.bar_alpha))
                    .border(hair(m))
                    .border_color(hairline(t))
                    .flex()
                    .items_center()
                    .text_size(px(13.))
                    .text_color(match announcement.as_ref().map(|a| a.tone) {
                        Some(Tone::Error) => t.error.rgba(),
                        Some(Tone::Info) => t.text.rgba(),
                        None => t.warning.rgba(),
                    })
                    .child(toast_text.unwrap_or_default()),
            );
        // Key hints, behind Ctrl+/ (or F1).
        let hints_bar = self.show_hints.then(|| {
            div()
                .absolute()
                .left(px(m.rail.x))
                .w(px(m.rail.w))
                .top(px(m.viewport.1 - PAD_BOTTOM - BAR_HEIGHT * 2. - 8.))
                .h(px(BAR_HEIGHT))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .px(px(18.))
                        .h(px(BAR_HEIGHT))
                        .rounded_full()
                        .bg(t.canvas.alpha(t.bar_alpha))
                        .border(hair(m))
                        .border_color(hairline(t))
                        .flex()
                        .items_center()
                        .gap(px(14.))
                        .text_size(px(13.))
                        .children(hints.into_iter().map(|(keys, what)| {
                            div()
                                .flex()
                                .items_center()
                                .gap(px(6.))
                                .text_color(t.muted.rgba())
                                .child(keycap(t, m, keys))
                                .child(what)
                        })),
                )
        });

        // The mark and wordmark, top left: the brand, and the settings
        // entry (Ctrl+, too). On the grid's left margin, centred on the
        // search row; the whole lockup is the button, and it lifts on hover.
        let mark_hovered = self.mark_hovered;
        let mark = div()
            .id("mark")
            .role(Role::Button)
            .aria_label("Orbit settings")
            .aria_keyshortcuts("Control+Comma")
            .absolute()
            .left(px(m.grid.x))
            .top(px(PAD_TOP + (SEARCH_HEIGHT - MARK_SIZE) / 2.))
            .h(px(MARK_SIZE))
            .flex()
            .items_center()
            .gap(px(MARK_GAP))
            .cursor_pointer()
            .on_hover(cx.listener(|this, hovered: &bool, _, cx| {
                this.mark_hovered = *hovered;
                cx.notify();
            }))
            .on_click(cx.listener(|this, _, window, cx| {
                this.open_settings(&OpenSettings, window, cx);
            }))
            .child(
                div()
                    .size(px(MARK_SIZE))
                    .rounded(px(MARK_SIZE * 0.3))
                    .shadow(vec![soft_shadow(if mark_hovered { 0.45 } else { 0.3 })])
                    .child(
                        img(look.mark.clone())
                            .size(px(MARK_SIZE))
                            .opacity(if mark_hovered { 1. } else { 0.92 }),
                    ),
            )
            .child(match look.wordmark.clone() {
                Some((wordmark, aspect)) => div()
                    .h(px(WORDMARK_SIZE))
                    .w(px((WORDMARK_SIZE * aspect).round()))
                    .opacity(if mark_hovered { 1. } else { 0.92 })
                    .child(img(wordmark).size_full().object_fit(ObjectFit::Fill)),
                None => div()
                    .text_size(px(WORDMARK_SIZE))
                    .line_height(px(MARK_SIZE))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(t.strong.rgba())
                    .opacity(if mark_hovered { 1. } else { 0.92 })
                    .child("Orbit"),
            })
            .when(mark_hovered, |d| {
                d.child(
                    div()
                        .ml(px(4.))
                        .px(px(10.))
                        .py(px(4.))
                        .rounded(px(8.))
                        .bg(t.canvas.alpha(t.bar_alpha))
                        .border(hair(m))
                        .border_color(hairline(t))
                        .text_size(px(13.))
                        .text_color(t.text.rgba())
                        .child("Settings · Ctrl+,"),
                )
            });

        // The update pill, right of the lockup: "0.1.1 available" (or what
        // just happened). Click opens the settings. Hidden on an output too
        // narrow to keep it clear of the search field.
        let badge = cx.try_global::<UpdateBadge>().cloned().unwrap_or_default();
        let wordmark_w = look
            .wordmark
            .as_ref()
            .map(|(_, aspect)| (WORDMARK_SIZE * aspect).round())
            .unwrap_or(72.);
        let pill_left = m.grid.x + MARK_SIZE + MARK_GAP + wordmark_w + 20.;
        let pill_w = 24. + badge.text.len() as f32 * 7.6;
        let update_pill =
            (!badge.text.is_empty() && pill_left + pill_w < m.search.x - 12.).then(|| {
                let failed = badge.failed;
                let label: SharedString = format!("{}. {}", badge.text, badge.detail).into();
                div()
                    .id("update-pill")
                    .role(Role::Button)
                    .aria_label(label)
                    .absolute()
                    .left(px(pill_left))
                    .top(px(PAD_TOP + (SEARCH_HEIGHT - 28.) / 2.))
                    .h(px(28.))
                    .px(px(12.))
                    .rounded_full()
                    .bg(if failed {
                        t.error.alpha(0.18)
                    } else {
                        t.selection.alpha(0.22)
                    })
                    .border(hair(m))
                    .border_color(if failed {
                        t.error.alpha(0.6)
                    } else {
                        t.selection.alpha(0.6)
                    })
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .text_size(px(13.))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(t.strong.rgba())
                    .cursor_pointer()
                    .child(div().size(px(8.)).rounded_full().bg(if failed {
                        t.error.rgba()
                    } else {
                        t.selection.rgba()
                    }))
                    .child(badge.text.clone())
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.open_settings(&OpenSettings, window, cx);
                    }))
            });

        // The glass: the frosted capture (or a solid), under the theme's
        // tint, with the lit top edge; nothing of the live desktop shows.
        let surface = div()
            .absolute()
            .left_0()
            .top_0()
            .size_full()
            .bg(t.surface.rgba())
            .children(backdrop.map(|image| {
                img(image)
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .object_fit(ObjectFit::Fill)
            }))
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(t.canvas.alpha(t.scrim_alpha)),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full()
                    .bg(linear_gradient(
                        180.,
                        linear_color_stop(t.surface.alpha(t.panel_alpha), 0.),
                        linear_color_stop(t.canvas.alpha(t.panel_alpha), 1.),
                    )),
            )
            .child(
                div()
                    .absolute()
                    .top_0()
                    .left_0()
                    .w_full()
                    .h(px(2.))
                    .bg(linear_gradient(
                        90.,
                        linear_color_stop(theme::WHITE.alpha(0.0), 0.),
                        linear_color_stop(theme::WHITE.alpha(0.10), 0.5),
                    )),
            );

        div()
            .id("orbit")
            .role(Role::Dialog)
            .aria_label("Orbit overview")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::dismiss))
            .on_action(cx.listener(Self::next))
            .on_action(cx.listener(Self::previous))
            .on_action(cx.listener(Self::reorder_left))
            .on_action(cx.listener(Self::reorder_right))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::first))
            .on_action(cx.listener(Self::last))
            .on_action(cx.listener(Self::activate))
            .on_action(cx.listener(Self::activate_follow))
            .on_action(cx.listener(Self::tab_forward))
            .on_action(cx.listener(Self::tab_back))
            .on_action(cx.listener(Self::move_menu))
            .on_action(cx.listener(Self::focus_search))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::delete))
            .on_action(cx.listener(Self::toggle_hints))
            .on_action(cx.listener(Self::open_settings))
            .on_mouse_move(cx.listener(|this, event: &gpui::MouseMoveEvent, _, cx| {
                this.pointer_moved(event.position, event.pressed_button.is_some(), cx);
            }))
            .when_some(look.font, |d, font| d.font_family(font))
            .relative()
            .size_full()
            .opacity(surface_opacity)
            .text_color(t.text.rgba())
            .child(surface)
            .child(mark)
            .children(update_pill)
            .child(self.search_field(
                t,
                m,
                &field,
                region == Region::Search,
                moving_app.as_deref(),
                cx,
            ))
            .child(rail)
            .child(grid)
            .child(status_line)
            .children(hints_bar)
            // Text input for the whole surface: typing anywhere searches.
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        window.handle_input(
                            &focus,
                            ElementInputHandler::new(bounds, view.clone()),
                            cx,
                        );
                    },
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ghost_shrinks_to_its_in_tile_size_as_the_pointer_reaches_the_rail() {
        let rail = Rect {
            x: 200.,
            y: 120.,
            w: 1000.,
            h: 140.,
        };
        // Far above the rail: no shrink. On its edge: full. Half way in the approach: half.
        assert_eq!(ghost_progress((600., 0.), rail, 96.), 0.);
        assert_eq!(ghost_progress((600., 130.), rail, 96.), 1.);
        assert!((ghost_progress((600., 120. - 48.), rail, 96.) - 0.5).abs() < 1e-3);
        // Beside the rail counts by distance too.
        assert!(ghost_progress((200. - 96., 150.), rail, 96.) <= 0.001);
        assert!(ghost_progress((200. - 10., 150.), rail, 96.) > 0.8);

        // A 1920x1200 window on a 2304-wide output: in a 176-wide tile it is
        // 1920 * 176 / 2304 = 146.7 px wide; the ghost goes from 264 to that.
        let (w0, h0) = ghost_size(0., 1.6, (1920., 1200.), 2304.);
        assert_eq!((w0, h0), (264., 165.));
        let (w1, h1) = ghost_size(1., 1.6, (1920., 1200.), 2304.);
        assert_eq!((w1, h1), (147., 92.));
        assert!(
            w1 <= THUMB_WIDTH && h1 <= THUMB_HEIGHT,
            "fits inside a tile"
        );
        let (wm, _) = ghost_size(0.5, 1.6, (1920., 1200.), 2304.);
        assert!(wm > w1 && wm < w0);
        // A tiny window never vanishes.
        let (ws, _) = ghost_size(1., 1.0, (40., 40.), 2304.);
        assert_eq!(ws, GHOST_MIN_WIDTH);
    }

    #[test]
    fn metrics_follow_the_hero_proportions() {
        // djinn: 2304x1440 logical at 1.25x, three windows, twelve tiles.
        let m = Metrics::for_output(2304., 1440., 1.25, 3, 12);
        assert_eq!(m.columns, 3);
        assert_eq!(m.card, 684.);
        assert_eq!(m.slot, 428.);
        assert_eq!(m.search.w, 1280.);
        assert_eq!(m.search.y, 40.);
        assert_eq!(m.rail_fit, 10);
        assert_eq!(m.rail.y, 120.);
        assert_eq!(m.grid.y, 300.);
        assert_eq!(m.grid.h, 1100.);
        assert_eq!(m.rows(), 1);
        assert!(m.content_height() <= m.grid.h);
        // Three cards span the width with the gaps, inside the padding.
        let (a, c) = (m.card_rect(0), m.card_rect(2));
        assert_eq!(c.x + c.w - a.x, 3. * 684. + 2. * GAP);
        assert!(a.x >= 96. && c.x + c.w <= 2304. - 96.);
        // Everything lands on whole device pixels at 1.25x.
        for value in [
            m.card,
            m.slot,
            m.search.x,
            m.search.w,
            m.rail.x,
            m.rail.y,
            m.grid.y,
            m.row_height(),
            a.x,
            a.y,
            c.x,
        ] {
            let device = value * 1.25;
            assert!(
                (device - device.round()).abs() < 1e-3,
                "{value} -> {device}"
            );
        }
        let boxes = m.preview_boxes();
        assert_eq!(boxes.card, (855 - 6, 535 - 6));
    }

    #[test]
    fn two_windows_get_two_big_columns_and_six_get_two_rows() {
        let two = Metrics::for_output(2304., 1440., 1.25, 2, 12);
        assert_eq!(two.columns, 2);
        assert_eq!(two.card, CARD_MAX);
        let (a, b) = (two.card_rect(0), two.card_rect(1));
        assert_eq!(b.x - a.x, CARD_MAX + GAP);
        assert_eq!(a.y, b.y);
        let one = Metrics::for_output(2304., 1440., 1.25, 1, 12);
        let only = one.card_rect(0);
        assert!((only.x + only.w / 2. - 1152.).abs() < 1.);
        let six = Metrics::for_output(2304., 1440., 1.25, 6, 12);
        assert_eq!(six.rows(), 2);
        assert!(six.content_height() <= six.grid.h);
        assert_eq!(six.card_rect(3).x, six.card_rect(0).x);
        assert!(six.card_rect(3).y > six.card_rect(0).y);
        // A partial last row is centered.
        let four = Metrics::for_output(2304., 1440., 1.25, 4, 12);
        assert!((four.card_rect(3).x + four.card / 2. - 1152.).abs() < 1.);
    }

    #[test]
    fn rail_keeps_the_shown_tile_in_the_strip() {
        let m = Metrics::for_output(2304., 1440., 1.25, 3, 12);
        assert_eq!(m.rail_first(0), 0);
        assert_eq!(m.rail_first(5), 0);
        assert_eq!(m.rail_first(11), 2);
        assert_eq!(m.rail_first(8), 2);
        let few = Metrics::for_output(2304., 1440., 1.25, 3, 5);
        assert_eq!(few.rail_fit, 5);
        assert_eq!(few.rail_first(4), 0);
        let r = m.tile_rect(3, 2);
        assert_eq!(r.x, m.rail.x + THUMB_WIDTH + THUMB_GAP);
        assert_eq!(m.tile_rect(0, 0).w, THUMB_WIDTH);
        assert!(m.tile_rect(0, 0).w >= 160. && m.rail.h > THUMB_HEIGHT);
    }

    #[test]
    fn metrics_adapt_to_other_outputs() {
        let hd = Metrics::for_output(1920., 1080., 1.0, 3, 6);
        assert_eq!(hd.columns, 3);
        assert!((CARD_MIN..=CARD_MAX).contains(&hd.card));
        assert_eq!(hd.card % 4., 0.);
        assert!(hd.content_height() <= hd.grid.h + 1.);
        let laptop = Metrics::for_output(1366., 768., 1.0, 3, 6);
        assert_eq!(laptop.columns, 3);
        assert!(laptop.card >= CARD_MIN);
        let wide = Metrics::for_output(3440., 1440., 1.0, 3, 6);
        assert_eq!(wide.card, CARD_MAX);
        let no_rail = Metrics::for_output(2304., 1440., 1.25, 5, 0);
        assert_eq!(no_rail.grid.y, no_rail.rail.y);
        let bottom = Metrics::for_output_with(2304., 1440., 1.25, 3, 12, None, true);
        assert_eq!(bottom.rail.y, 1440. - PAD_BOTTOM - RAIL_HEIGHT);
        assert_eq!(bottom.grid.y, PAD_TOP + SEARCH_HEIGHT + ROW_GAP);
        assert!(bottom.grid.y + bottom.grid.h <= bottom.rail.y);
        let four = Metrics::for_output_with(2304., 1440., 1.25, 5, 12, Some(4), false);
        assert_eq!(four.columns, 4);
        assert_eq!(four.card % 4., 0.);
        assert_eq!(fit_box(1.5, (100, 100)), (100, 67));
        assert_eq!(fit_box(0.5, (100, 100)), (50, 100));
    }
}
