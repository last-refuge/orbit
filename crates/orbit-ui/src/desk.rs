//! The daemon's long-lived state: the store, the open overview (if any)
//! and its preview textures. Platform workers feed it compositor input and
//! capture frames; it answers with [`Outbound`] requests. No I/O happens
//! here or in rendering.

use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::Instant,
};

use gpui::{Context, RenderImage};
use orbit_core::{
    Command, Config, Dest, Effect, Epoch, Event, Intent, Model, OpId, Outcome, Overview, Refresh,
    RequestError, Store, Sync, Target, WorkspaceKind,
};
use smallvec::SmallVec;

/// Finished operations kept for `orbit state`.
const LOG_KEPT: usize = 32;
/// Focus is sent once Hyprland reports the overlay's layer closed (closing
/// it hands focus back to the window that had it, which would undo a focus
/// sent earlier), or after this long without that event.
const LAYER_CLOSE_WAIT_MS: u64 = 200;
/// How long a focus may go unobserved before it is sent again, and how many
/// times; and how long it must hold once observed.
const FOCUS_RETRY_MS: u64 = 120;
const FOCUS_TRIES: u8 = 5;
const FOCUS_SETTLE_MS: u64 = 300;
/// The overlay's layer-shell namespace.
const LAYER_NAMESPACE: &str = "orbit";

/// Decides, by window class, whether a window may be captured.
type ClassFilter = Box<dyn Fn(&str) -> bool>;

/// Where the time went in the last activation, in ms after the click (or
/// Enter), for `orbit state` (`last_activation`). The focus and raise go
/// out at once; the overlay's close and Hyprland's layer-closed event
/// follow; the focus is sent again once the layer is gone (closing a layer
/// hands focus back) and confirmed when Hyprland reports the window active.
#[derive(Clone, Debug, Default)]
pub struct Activation {
    pub window: String,
    pub focus_sent_ms: Option<u64>,
    pub closed_ms: Option<u64>,
    pub layer_gone_ms: Option<u64>,
    pub resent_ms: Option<u64>,
    pub active_ms: Option<u64>,
    pub settled_ms: Option<u64>,
    pub sends: u8,
    clicked_at: u64,
}

impl Activation {
    pub fn json(&self) -> serde_json::Value {
        serde_json::json!({
            "window": self.window,
            "focus_sent_ms": self.focus_sent_ms,
            "closed_ms": self.closed_ms,
            "layer_gone_ms": self.layer_gone_ms,
            "resent_ms": self.resent_ms,
            "active_ms": self.active_ms,
            "settled_ms": self.settled_ms,
            "sends": self.sends,
        })
    }
}

/// A window the user asked to focus, until Hyprland reports it active and
/// it stays active for [`FOCUS_SETTLE_MS`].
struct FocusGoal {
    window: orbit_core::WindowId,
    /// When the overlay closed; `None` while it is open.
    closed_at: Option<u64>,
    /// Hyprland reported the overlay's layer gone.
    layer_gone: bool,
    /// When it was last sent; `None` until then.
    sent_at: Option<u64>,
    /// When Hyprland first reported it active since the last send.
    seen_at: Option<u64>,
    tries: u8,
}

/// A workspace as a dispatch names it: Hyprland id and name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Place {
    pub id: i64,
    pub name: String,
}

/// Requests for the platform side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outbound {
    SwapWorkspaces {
        source: orbit_core::WorkspaceId,
        target: orbit_core::WorkspaceId,
        source_windows: Vec<u64>,
        target_windows: Vec<u64>,
        origin: (u64, Epoch),
    },
    /// Query and report with [`Desk::refreshed`].
    Query(orbit_core::Kinds),
    /// Move silently, then report with [`Desk::move_dispatched`].
    Move { op: OpId, address: u64, to: Place },
    /// Focus a window (switching to its workspace).
    Focus { address: u64 },
    /// Put a window on top of the floating stack, sent right after each
    /// focus once the overlay is gone and once more when the focus has
    /// settled: focus alone leaves an all-floating window behind the one
    /// that had focus.
    Raise { address: u64 },
    /// Switch the focused monitor to a workspace.
    Switch { to: Place },
    /// Capture these windows while the overview is open, each to fit the
    /// device-pixel box it is drawn in; empty stops.
    Previews(Vec<Want>),
    /// Capture the output once and frost it to `size` (device px) for the
    /// overview's backdrop; report with [`Desk::set_backdrop`].
    Backdrop {
        output: Option<String>,
        size: (u32, u32),
    },
    /// Open the settings window (the mark or Ctrl+, in the overview).
    OpenSettings,
}

/// Device pixels per blurred-backdrop pixel (the worker blurs at this
/// size; the GPU upscales).
pub const BACKDROP_DOWNSCALE: u32 = 6;

/// One window to capture and the largest device-pixel size its preview is
/// drawn at, so the worker downscales once to what the screen shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Want {
    pub address: u64,
    pub width: u32,
    pub height: u32,
}

/// Device-pixel box card previews are drawn in. The view reports it from
/// its layout. Rail tiles reuse the card previews of the previewed
/// workspace and draw other workspaces' windows as outlines, so nothing
/// off the previewed workspace is ever captured.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PreviewBoxes {
    pub card: (u32, u32),
}

impl Default for PreviewBoxes {
    fn default() -> Self {
        // Until the first frame lays the overview out: a 3-column grid on
        // a 2304-wide output at 1.25x.
        Self { card: (849, 529) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Connecting,
    Live { version: String },
    Lost { error: String },
}

pub struct Preview {
    pub image: Arc<RenderImage>,
    pub size: (u32, u32),
}

pub struct Desk {
    store: Store,
    link: Option<u64>,
    epoch: Option<Epoch>,
    pub status: Link,
    overview: Option<Overview>,
    previews: HashMap<u64, Preview>,
    /// The frosted output capture behind the glass; `None` until it
    /// arrives, and when capture is unavailable or restricted (then the
    /// overview draws a solid backdrop).
    backdrop: Option<Arc<RenderImage>>,
    /// A backdrop was asked for and has not answered yet.
    backdrop_pending: bool,
    wanted: Vec<Want>,
    boxes: PreviewBoxes,
    /// Only windows whose class passes are ever captured (test runs name
    /// Orbit's fixture classes, so no real window of the user's is).
    capture_filter: Option<ClassFilter>,
    /// Classes the person never wants captured (settings).
    never_capture: Vec<String>,
    /// The window that was active before the current one, for the
    /// shortcut's double tap.
    last_active: Option<orbit_core::WindowId>,
    current_active: Option<orbit_core::WindowId>,
    /// Settings: activating a window keeps the overview open when false;
    /// windows off the previewed workspace are captured too when true.
    close_on_activate: bool,
    capture_inactive: bool,
    hide_special: bool,
    search_this_workspace: bool,
    swapping: Option<(u64, Epoch)>,
    out: Box<dyn Fn(Outbound)>,
    clock: Instant,
    log: VecDeque<(OpId, String)>,
    /// Set when the overview should close (a committed activation).
    dismiss: bool,
    /// Counts opens, so a late close from a replaced surface is ignored.
    session: u64,
    focus: Option<FocusGoal>,
    activation: Option<Activation>,
}

impl Desk {
    pub fn new(out: impl Fn(Outbound) + 'static) -> Self {
        Self {
            store: Store::new(Config::default()),
            link: None,
            epoch: None,
            status: Link::Connecting,
            overview: None,
            previews: HashMap::new(),
            backdrop: None,
            backdrop_pending: false,
            wanted: Vec::new(),
            boxes: PreviewBoxes::default(),
            capture_filter: None,
            never_capture: Vec::new(),
            last_active: None,
            current_active: None,
            close_on_activate: true,
            capture_inactive: false,
            hide_special: false,
            search_this_workspace: false,
            swapping: None,
            out: Box::new(out),
            clock: Instant::now(),
            log: VecDeque::new(),
            dismiss: false,
            session: 0,
            focus: None,
            activation: None,
        }
    }

    /// Stamps one field of the current activation with the time since its
    /// click.
    fn stamp(&mut self, field: fn(&mut Activation) -> &mut Option<u64>) {
        let now = self.now();
        if let Some(activation) = &mut self.activation {
            let since = now.saturating_sub(activation.clicked_at);
            let slot = field(activation);
            if slot.is_none() {
                *slot = Some(since);
            }
        }
    }

    pub fn store(&self) -> &Store {
        &self.store
    }

    pub fn overview(&self) -> Option<&Overview> {
        self.overview.as_ref()
    }

    /// Runs a navigation step on the open overview, then brings previews in
    /// step: moving between workspaces changes which cards need capturing.
    pub fn navigate(
        &mut self,
        f: impl FnOnce(&mut Overview, &Model) -> Option<Intent>,
        cx: &mut Context<Self>,
    ) -> Option<Intent> {
        let overview = self.overview.as_mut()?;
        let intent = f(overview, self.store.model());
        self.changed(cx);
        intent
    }

    pub fn preview(&self, address: u64) -> Option<&Preview> {
        self.previews.get(&address)
    }

    /// The frosted backdrop for the open overview, once captured.
    pub fn backdrop(&self) -> Option<&Arc<RenderImage>> {
        self.backdrop.as_ref()
    }

    pub fn backdrop_pending(&self) -> bool {
        self.backdrop_pending
    }

    /// The worker's answer to [`Outbound::Backdrop`]; `None` means the
    /// output could not be captured, so the solid fallback stays.
    pub fn set_backdrop(&mut self, bgra: Option<image::RgbaImage>, cx: &mut Context<Self>) {
        self.backdrop_pending = false;
        if self.overview.is_none() {
            return;
        }
        if let Some(old) = self.backdrop.take() {
            cx.drop_image(old, None);
        }
        self.backdrop = bgra.map(|bgra| {
            Arc::new(RenderImage::new(SmallVec::from_elem(
                image::Frame::new(bgra),
                1,
            )))
        });
        cx.notify();
    }

    /// Restricts capture to windows whose class passes `filter`, for the
    /// life of the desk. Scripted runs pass the fixture classes.
    pub fn restrict_capture(&mut self, filter: impl Fn(&str) -> bool + 'static) {
        self.capture_filter = Some(Box::new(filter));
    }

    pub fn capture_restricted(&self) -> bool {
        self.capture_filter.is_some()
    }

    /// Classes never captured, from the settings; previews show the icon.
    pub fn set_never_capture(&mut self, classes: Vec<String>) {
        if self.never_capture != classes {
            self.never_capture = classes;
            self.sync_previews();
        }
    }

    pub fn request_settings(&self) {
        (self.out)(Outbound::OpenSettings);
    }

    pub fn workspace_swapped(
        &mut self,
        origin: (u64, Epoch),
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if self.swapping != Some(origin) {
            return;
        }
        self.swapping = None;
        if let Some(o) = &mut self.overview {
            o.swapping = false;
            if origin == (self.session, self.store.epoch()) {
                let (text, tone) = match result {
                    Ok(()) => (
                        "Workspaces swapped; window layouts preserved".into(),
                        orbit_core::Tone::Info,
                    ),
                    Err(error) => (
                        format!("Could not swap workspaces: {error}"),
                        orbit_core::Tone::Error,
                    ),
                };
                o.announcement = Some(orbit_core::Announcement { text, tone });
            }
        }
        self.changed(cx);
    }

    pub fn set_close_on_activate(&mut self, close: bool) {
        self.close_on_activate = close;
    }

    /// Settings the open overview reads: no special workspaces in the
    /// rail; search limited to the previewed workspace.
    pub fn set_overview_prefs(&mut self, hide_special: bool, search_this_workspace: bool) {
        self.hide_special = hide_special;
        self.search_this_workspace = search_this_workspace;
        if let Some(o) = &mut self.overview {
            o.hide_special = hide_special;
            o.search_this_workspace = search_this_workspace;
        }
    }

    pub fn set_capture_inactive(&mut self, capture: bool) {
        if self.capture_inactive != capture {
            self.capture_inactive = capture;
            self.sync_previews();
        }
    }

    /// The window active before the current one, if still around.
    pub fn last_active(&self) -> Option<orbit_core::WindowId> {
        self.last_active
            .filter(|id| self.store.model().window(*id).is_some())
    }

    /// The view reports the device-pixel boxes previews are drawn in; a
    /// change re-requests captures at the new size. Called from rendering,
    /// so it does not notify.
    pub fn set_preview_boxes(&mut self, boxes: PreviewBoxes) {
        if self.boxes != boxes {
            self.boxes = boxes;
            self.sync_previews();
        }
    }

    pub fn preview_boxes(&self) -> PreviewBoxes {
        self.boxes
    }

    /// True once after a committed action asked the overview to close.
    pub fn take_dismiss(&mut self) -> bool {
        std::mem::take(&mut self.dismiss)
    }

    fn now(&self) -> u64 {
        self.clock.elapsed().as_millis() as u64
    }

    // Compositor input. Anything from a link other than the current one
    // is from a replaced connection and ignored.

    pub fn connected(&mut self, link: u64, version: String, cx: &mut Context<Self>) {
        let (epoch, effects) = self.store.connect(self.now());
        self.link = Some(link);
        self.epoch = Some(epoch);
        self.status = Link::Live { version };
        self.run(effects, cx);
        if self.overview.is_some() {
            let effects = self.store.set_visible(true, self.now());
            self.run(effects, cx);
        }
        self.changed(cx);
    }

    pub fn disconnected(&mut self, link: u64, error: String, cx: &mut Context<Self>) {
        if self.link == Some(link) {
            if let Some(origin) = self.swapping {
                self.workspace_swapped(
                    origin,
                    Err("Connection lost; refresh to check the result before retrying".into()),
                    cx,
                );
            }
            self.store.disconnect();
            self.status = Link::Lost { error };
            self.changed(cx);
        }
    }

    pub fn event(&mut self, link: u64, event: Event, cx: &mut Context<Self>) {
        let Some(epoch) = self.current(link) else {
            return;
        };
        if let Event::LayerClosed { namespace } = &event
            && namespace == LAYER_NAMESPACE
            && let Some(goal) = &mut self.focus
        {
            // Closing the layer hands focus back to the window that had
            // it: send the focus again now, not after a retry interval.
            goal.layer_gone = true;
            goal.sent_at = None;
            goal.seen_at = None;
            self.stamp(|a| &mut a.layer_gone_ms);
            self.pursue_focus();
        }
        if let Event::WorkspaceIdChanged { old, new } = &event
            && let Some(o) = &mut self.overview
        {
            o.workspace_id_changed(*old, *new);
        }
        if let Ok(effects) = self.store.on_event(epoch, event) {
            self.run(effects, cx);
            self.changed(cx);
        }
        if self.focus.is_some() {
            // Confirm (or retry) on the event itself, not on the next tick.
            self.pursue_focus();
        }
    }

    pub fn gap(&mut self, link: u64, cx: &mut Context<Self>) {
        let Some(epoch) = self.current(link) else {
            return;
        };
        if let Ok(effects) = self.store.on_gap(epoch) {
            self.run(effects, cx);
            self.changed(cx);
        }
    }

    pub fn refreshed(&mut self, link: u64, refresh: Refresh, cx: &mut Context<Self>) {
        let Some(epoch) = self.current(link) else {
            return;
        };
        if let Ok(effects) = self.store.on_refresh(epoch, refresh) {
            self.run(effects, cx);
            self.changed(cx);
        }
    }

    pub fn move_dispatched(
        &mut self,
        op: OpId,
        result: Result<(), String>,
        cx: &mut Context<Self>,
    ) {
        if let Ok(effects) = self.store.on_dispatch_result(op, result) {
            self.run(effects, cx);
            self.changed(cx);
        }
    }

    /// A focus or switch request failed (the overview is usually gone).
    pub fn control_failed(&mut self, error: String, cx: &mut Context<Self>) {
        log::warn!("control request failed: {error}");
        self.push_log(OpId(0), format!("control failed: {error}"));
        cx.notify();
    }

    pub fn tick(&mut self, cx: &mut Context<Self>) {
        let effects = self.store.tick(self.now());
        if !effects.is_empty() {
            self.run(effects, cx);
            self.changed(cx);
        }
        self.pursue_focus();
    }

    /// Sends a pending focus once the overlay is gone, and again while
    /// Hyprland does not show the window active (a reply of `ok` does not
    /// mean it happened).
    fn pursue_focus(&mut self) {
        let now = self.now();
        let Some(goal) = &mut self.focus else { return };
        let model = self.store.model();
        if model.window(goal.window).is_none() {
            self.focus = None;
            return;
        }
        let closed_at = match goal.closed_at {
            Some(at) => at,
            // Still open: the first send goes out at once; re-sends wait
            // for the close.
            None if goal.tries == 0 => now,
            None => return,
        };
        if goal.sent_at.is_none()
            && goal.tries > 0
            && !goal.layer_gone
            && now.saturating_sub(closed_at) < LAYER_CLOSE_WAIT_MS
        {
            return;
        }
        if goal.sent_at.is_some() && model.active_window == Some(goal.window) {
            let first_seen = goal.seen_at.is_none();
            let seen = *goal.seen_at.get_or_insert(now);
            let settled = now.saturating_sub(seen) >= FOCUS_SETTLE_MS;
            let address = goal.window.address;
            if first_seen {
                self.stamp(|a| &mut a.active_ms);
            }
            if settled {
                // Settled: one last raise, after whatever the compositor
                // did on the layer's close.
                self.focus = None;
                self.stamp(|a| &mut a.settled_ms);
                (self.out)(Outbound::Raise { address });
            }
            return;
        }
        goal.seen_at = None;
        let due = goal
            .sent_at
            .is_none_or(|at| now.saturating_sub(at) >= FOCUS_RETRY_MS);
        if !due {
            return;
        }
        if goal.tries >= FOCUS_TRIES {
            log::warn!("focus was not observed after {FOCUS_TRIES} requests");
            self.push_log(OpId(0), "focus not observed".into());
            self.focus = None;
            return;
        }
        goal.tries += 1;
        goal.sent_at = Some(now);
        let address = goal.window.address;
        let after_layer = goal.layer_gone;
        if let Some(activation) = &mut self.activation {
            activation.sends += 1;
        }
        self.stamp(|a| &mut a.focus_sent_ms);
        if after_layer {
            self.stamp(|a| &mut a.resent_ms);
        }
        (self.out)(Outbound::Focus { address });
        (self.out)(Outbound::Raise { address });
    }

    // The overview's lifecycle.

    /// Opens the overview on `monitor`; returns the session to close.
    pub fn open(&mut self, monitor: Option<&str>, cx: &mut Context<Self>) -> u64 {
        self.session += 1;
        log::debug!("overview session {} opened", self.session);
        let mut overview = Overview::open(self.store.model(), monitor);
        overview.hide_special = self.hide_special;
        overview.search_this_workspace = self.search_this_workspace;
        overview.swapping = self.swapping.is_some();
        self.overview = Some(overview);
        self.dismiss = false;
        // The frosted backdrop: one output capture per open. Restricted
        // capture (scripted runs) never captures the desktop, so the
        // overview draws its solid fallback instead.
        if self.capture_filter.is_none() {
            let model = self.store.model();
            let output = self
                .overview
                .as_ref()
                .and_then(|o| o.invocation.monitor.clone());
            let device = output
                .as_deref()
                .and_then(|name| model.monitors.iter().find(|m| m.name == name))
                .map(|m| {
                    (
                        (m.width * m.scale).round() as u32,
                        (m.height * m.scale).round() as u32,
                    )
                })
                .unwrap_or((2880, 1800));
            self.backdrop_pending = true;
            (self.out)(Outbound::Backdrop {
                output,
                size: (
                    (device.0 / BACKDROP_DOWNSCALE).max(1),
                    (device.1 / BACKDROP_DOWNSCALE).max(1),
                ),
            });
        }
        let effects = self.store.set_visible(true, self.now());
        self.run(effects, cx);
        self.changed(cx);
        self.session
    }

    pub fn close(&mut self, session: u64, cx: &mut Context<Self>) {
        if session != self.session || self.overview.is_none() {
            return;
        }
        log::debug!("overview session {session} closed");
        self.overview = None;
        let effects = self.store.set_visible(false, self.now());
        self.run(effects, cx);
        // The overlay's atlas went with its window; just forget the images.
        self.previews.clear();
        self.backdrop = None;
        self.backdrop_pending = false;
        let now = self.now();
        if let Some(goal) = &mut self.focus {
            goal.closed_at = Some(now);
        }
        self.stamp(|a| &mut a.closed_ms);
        self.changed(cx);
        self.pursue_focus();
    }

    /// Carries out an intent from the overview. Returns true when the
    /// overview should close now.
    pub fn perform(&mut self, intent: Intent, cx: &mut Context<Self>) -> bool {
        // Keep an in-flight swap isolated from other UI mutations.
        if self.swapping.is_some() && !matches!(intent, Intent::Dismiss) {
            return false;
        }
        let dismiss = match intent {
            Intent::SwapWorkspaces { source, target } => {
                if self.store.sync() == Sync::Live
                    && self.swapping.is_none()
                    && !self.store.has_pending_moves()
                    && self.overview.as_ref().is_some_and(|o| {
                        o.reorder_workspace(
                            self.store.model(),
                            source,
                            Dest::Existing(target),
                            false,
                        )
                        .is_some()
                    })
                {
                    let origin = (self.session, self.store.epoch());
                    self.swapping = Some(origin);
                    if let Some(o) = &mut self.overview {
                        o.swapping = true;
                        o.rail_focus = Some(Dest::Existing(source));
                        o.region = orbit_core::Region::Rail;
                    }
                    let addresses = |id| {
                        self.store
                            .model()
                            .windows_on(id)
                            .map(|w| w.id.address)
                            .collect()
                    };
                    (self.out)(Outbound::SwapWorkspaces {
                        source,
                        target,
                        source_windows: addresses(source),
                        target_windows: addresses(target),
                        origin,
                    });
                }
                false
            }
            Intent::Dismiss => true,
            Intent::Activate(window) => {
                if self.store.model().window(window).is_some() {
                    // The focus and raise go out now, before the overlay
                    // is even gone: nothing in the hand-off needs the
                    // layer closed first. Closing it hands focus back to
                    // the previous window, so `pursue_focus` sends again
                    // when Hyprland reports the layer closed (or after
                    // `LAYER_CLOSE_WAIT_MS`), and keeps on until the
                    // window is reported active.
                    let now = self.now();
                    self.activation = Some(Activation {
                        window: format!("{:#x}", window.address),
                        clicked_at: now,
                        ..Activation::default()
                    });
                    self.focus = Some(FocusGoal {
                        window,
                        closed_at: (!self.close_on_activate).then_some(now),
                        layer_gone: !self.close_on_activate,
                        sent_at: None,
                        seen_at: None,
                        tries: 0,
                    });
                    self.pursue_focus();
                    self.close_on_activate
                } else {
                    if let Some(overview) = &mut self.overview {
                        overview.refused(&RequestError::StaleWindow);
                    }
                    false
                }
            }
            Intent::Switch(dest) => {
                if let Some(to) = self.place(dest) {
                    let special = matches!(dest, Dest::Existing(id) if self.store.model().workspaces.get(&id).is_some_and(|w| w.kind() == WorkspaceKind::Special));
                    let shown = matches!(dest, Dest::Existing(id) if self.store.model().monitors.iter().any(|m| m.special_workspace == Some(id)));
                    // Toggling a special workspace that is already open would hide it.
                    if !(special && shown) {
                        (self.out)(Outbound::Switch { to });
                    }
                }
                true
            }
            Intent::Move {
                window,
                dest,
                follow,
            } => {
                match self.store.request_move(window, dest.target(), self.now()) {
                    Ok((op, effects)) => {
                        if let Some(overview) = &mut self.overview {
                            overview.track(self.store.model(), op, window, dest, follow);
                        }
                        self.run(effects, cx);
                    }
                    Err(error) => {
                        if let Some(overview) = &mut self.overview {
                            overview.refused(&error);
                        }
                    }
                }
                false
            }
        };
        self.changed(cx);
        dismiss
    }

    /// A captured thumbnail (BGRA) for a card or a rail tile's cover.
    pub fn set_preview(&mut self, address: u64, bgra: image::RgbaImage, cx: &mut Context<Self>) {
        if !self.wanted.iter().any(|w| w.address == address) {
            return;
        }
        let size = (bgra.width(), bgra.height());
        let image = Arc::new(RenderImage::new(SmallVec::from_elem(
            image::Frame::new(bgra),
            1,
        )));
        if let Some(old) = self.previews.insert(address, Preview { image, size }) {
            // Free the replaced texture now instead of leaving it in the atlas.
            cx.drop_image(old.image, None);
        }
        cx.notify();
    }

    fn current(&self, link: u64) -> Option<Epoch> {
        (self.link == Some(link)).then_some(self.epoch).flatten()
    }

    fn place(&self, dest: Dest) -> Option<Place> {
        match dest {
            Dest::Existing(id) => self.store.model().workspaces.get(&id).map(|w| Place {
                id: id.0,
                name: w.name.clone(),
            }),
            Dest::NewNumbered(n) => Some(Place {
                id: n,
                name: n.to_string(),
            }),
        }
    }

    fn run(&mut self, effects: Vec<Effect>, cx: &mut Context<Self>) {
        for effect in effects {
            match effect {
                Effect::Query(kinds) => (self.out)(Outbound::Query(kinds)),
                Effect::Dispatch {
                    op,
                    command: Command::MoveToWorkspaceSilent { window, target },
                } => {
                    let to = match target {
                        Target::Workspace(id) => self.place(Dest::Existing(id)),
                        Target::Numbered(n) => self.place(Dest::NewNumbered(n)),
                    };
                    match to {
                        Some(to) => (self.out)(Outbound::Move {
                            op,
                            address: window.address,
                            to,
                        }),
                        // The store checked the destination; it can only
                        // vanish if the model changed in between.
                        None => {
                            let effects = self
                                .store
                                .on_dispatch_result(op, Err("destination vanished".into()))
                                .unwrap_or_default();
                            self.run(effects, cx);
                        }
                    }
                }
                Effect::Finished { op, outcome } => {
                    self.push_log(op, describe(&outcome));
                    let follow = self
                        .overview
                        .as_mut()
                        .and_then(|overview| overview.finished(op, &outcome));
                    if let Some(intent) = follow
                        && self.perform(intent, cx)
                    {
                        self.dismiss = true;
                    }
                }
            }
        }
    }

    /// Remembers the previously active window as focus moves.
    fn track_active(&mut self) {
        let active = self.store.model().active_window;
        if active != self.current_active {
            if let Some(previous) = self.current_active {
                self.last_active = Some(previous);
            }
            self.current_active = active;
        }
    }

    fn push_log(&mut self, op: OpId, outcome: String) {
        self.log.push_back((op, outcome));
        while self.log.len() > LOG_KEPT {
            self.log.pop_front();
        }
    }

    /// After any change: keep the overview valid and previews in step.
    fn changed(&mut self, cx: &mut Context<Self>) {
        if let Some(overview) = &mut self.overview {
            overview.reconcile(self.store.model());
        }
        self.track_active();
        self.sync_previews();
        cx.notify();
    }

    /// Asks the capture worker for exactly the windows on screen (the
    /// cards), at the card box; filtered when capture is restricted.
    fn sync_previews(&mut self) {
        let model = self.store.model();
        let mut wanted: Vec<Want> = Vec::new();
        if let Some(o) = &self.overview {
            let allowed = |id: orbit_core::WindowId| {
                model.window(id).is_some_and(|w| {
                    self.capture_filter.as_ref().is_none_or(|f| f(&w.class))
                        && !self
                            .never_capture
                            .iter()
                            .any(|c| c.eq_ignore_ascii_case(&w.class))
                })
            };
            for card in o.cards(model) {
                if allowed(card.window) {
                    wanted.push(Want {
                        address: card.window.address,
                        width: self.boxes.card.0,
                        height: self.boxes.card.1,
                    });
                }
            }
            if self.capture_inactive {
                // Every other workspace's windows, small, for the rail.
                for tile in o.tiles(model) {
                    if let orbit_core::Dest::Existing(ws) = tile.dest {
                        for w in model.windows_on(ws).filter(|w| !w.hidden) {
                            if allowed(w.id) && !wanted.iter().any(|x| x.address == w.id.address) {
                                wanted.push(Want {
                                    address: w.id.address,
                                    width: 220,
                                    height: 138,
                                });
                            }
                        }
                    }
                }
            }
        }
        if wanted != self.wanted {
            self.previews
                .retain(|address, _| wanted.iter().any(|w| w.address == *address));
            self.wanted = wanted.clone();
            (self.out)(Outbound::Previews(wanted));
        }
    }

    /// The overview as test scripts see it: identities, workspace names and
    /// outcomes, never titles.
    pub fn state_json(&self) -> serde_json::Value {
        use serde_json::json;
        let model = self.store.model();
        let overview = self.overview.as_ref().map(|o| {
            json!({
                "previewed": o.previewed.and_then(|id| model.workspaces.get(&id)).map(|w| w.name.clone()),
                "region": format!("{:?}", o.region),
                "rail_focus": o.rail_focus.map(|d| orbit_core::overview::dest_label(model, d)),
                "selected": o.selected.map(|w| format!("{:#x}", w.address)),
                "moving": o.moving.map(|w| format!("{:#x}", w.address)),
                "pending": o.pending_moves(),
                "announcement": o.announcement.as_ref().map(|a| a.text.clone()),
                // What was typed; never a window title.
                "search": o.search.text(),
                "searching": o.searching(),
                "destination_filter": o.destination.text(),
                "composing": o.editing().composing(),
                "cards": o.cards(model).iter().map(|c| json!({
                    "address": format!("{:#x}", c.window.address),
                    "class": c.class,
                    "workspace": c.workspace,
                    "selected": c.selected,
                    // Labels hold titles, so only whether they say enough.
                    "labelled": !c.app.is_empty()
                        && c.accessible_label().contains(&format!("workspace {}", c.workspace)),
                    "preview": self.previews.contains_key(&c.window.address),
                    "preview_size": self.previews.get(&c.window.address).map(|p| [p.size.0, p.size.1]),
                    "capturing": self.wanted.iter().any(|w| w.address == c.window.address),
                    "moving_to": c.moving_to,
                })).collect::<Vec<_>>(),
                "rail": o.tiles(model).iter().map(|t| json!({
                    "label": t.label,
                    "windows": t.windows,
                    "active": t.active,
                    "previewed": t.previewed,
                    "focused": t.focused,
                    "cover": t.cover.map(|w| format!("{:#x}", w.address)),
                    "cover_preview": t.cover.is_some_and(|w| self.previews.contains_key(&w.address)),
                    "accessible_label": t.accessible_label(),
                })).collect::<Vec<_>>(),
                "keyboard_paths": orbit_core::KEYBOARD_PATHS
                    .iter()
                    .map(|(pointer, keys)| json!({"pointer": pointer, "keys": keys}))
                    .collect::<Vec<_>>(),
            })
        });
        json!({
            "link": match &self.status {
                Link::Connecting => "connecting".to_string(),
                Link::Live { version } => format!("live {version}"),
                Link::Lost { error } => format!("lost: {error}"),
            },
            "sync": format!("{:?}", self.store.sync()),
            "epoch": self.store.epoch().0,
            "windows": model.windows.len(),
            "capture_restricted": self.capture_filter.is_some(),
            "backdrop": self.backdrop.is_some(),
            "capturing": self.wanted.iter().map(|w| json!({
                "address": format!("{:#x}", w.address),
                "box": [w.width, w.height],
                // The class says whether a captured window is one of Orbit's own.
                "class": model.windows.get(&w.address).map(|x| x.class.clone()),
            })).collect::<Vec<_>>(),
            "overview": overview,
            "finished": self.log.iter().map(|(op, outcome)| json!({"op": op.0, "outcome": outcome})).collect::<Vec<_>>(),
            "last_activation": self.activation.as_ref().map(Activation::json),
        })
    }

    pub fn is_live(&self) -> bool {
        self.store.sync() == Sync::Live
    }
}

fn describe(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Confirmed => "confirmed".into(),
        Outcome::Failed(failure) => format!("failed: {failure:?}"),
    }
}
