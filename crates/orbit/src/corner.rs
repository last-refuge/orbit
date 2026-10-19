//! Hot corners (ORB-010). Hyprland has none, so Orbit keeps a tiny
//! transparent layer-shell surface (overlay layer, no exclusive zone, 2x2
//! logical px) in the chosen corner of each monitor it watches. When the
//! pointer enters it and rests there for the dwell time, with no button
//! held and the overview closed, the surface asks the daemon to toggle the
//! overview on that monitor. The overview covers the corner while it is
//! open, so closing from the corner is the overview's own job (see
//! `corner_closes` there).

use std::{cell::RefCell, rc::Rc, sync::mpsc, time::Duration};

use futures::channel::mpsc as async_mpsc;
use gpui::{
    App, Bounds, Context, DisplayId, MouseExitEvent, MouseMoveEvent, Window,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions, canvas, div,
    layer_shell::*, point, prelude::*, px, size,
};
use orbit_core::settings::{Corner, Monitors};

use crate::daemon::{Envelope, Request};

pub const NAMESPACE: &str = "orbit-corner";
/// The surface's size in logical px.
const SIZE: f32 = 2.;

/// One watched corner on one monitor.
pub struct Watch {
    pub monitor: String,
    pub corner: Corner,
    handle: WindowHandle<CornerView>,
}

/// What the surfaces were last brought in line with: the settings and the
/// monitors (`(name, focused)`). The monitors are part of it because the
/// daemon applies its settings once at startup, before the Hyprland link
/// has reported any monitor; the sync must run again when they appear
/// (and on hotplug), or a corner configured before launch never exists.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SyncKey {
    pub corner: Corner,
    pub dwell_ms: u32,
    pub scope: Monitors,
    pub monitors: Vec<(String, bool)>,
}

impl SyncKey {
    /// The monitors that get a surface: all of them, or the focused one
    /// (else the first). None while no monitor is known yet.
    pub fn wanted(&self) -> Vec<&str> {
        if self.corner == Corner::Off {
            return Vec::new();
        }
        match self.scope {
            Monitors::All => self.monitors.iter().map(|(name, _)| name.as_str()).collect(),
            Monitors::Primary => self
                .monitors
                .iter()
                .find(|(_, focused)| *focused)
                .or_else(|| self.monitors.first())
                .map(|(name, _)| name.as_str())
                .into_iter()
                .collect(),
        }
    }
}

/// The daemon's set of corner surfaces, kept equal to the settings.
#[derive(Default)]
pub struct Corners {
    watches: Vec<Watch>,
    key: SyncKey,
}

impl Corners {
    /// Whether `key` is what the surfaces were last synced to (the
    /// cheap check the daemon runs every poll).
    pub fn in_sync(&self, key: &SyncKey) -> bool {
        self.key == *key
    }

    /// Brings the surfaces in line with `key`; returns how many are in
    /// place. Cheap when nothing changed and every surface still exists.
    pub fn sync(
        &mut self,
        key: SyncKey,
        sender: async_mpsc::UnboundedSender<Envelope>,
        display_for: &dyn Fn(&str, &App) -> Option<DisplayId>,
        cx: &mut App,
    ) -> usize {
        let unchanged = self.in_sync(&key)
            && self
                .watches
                .iter()
                .all(|w| w.handle.update(cx, |_, _, _| ()).is_ok());
        if unchanged {
            return self.watches.len();
        }
        self.close_all(cx);
        let corner = key.corner;
        let dwell_ms = key.dwell_ms;
        let wanted: Vec<String> = key.wanted().into_iter().map(str::to_owned).collect();
        self.key = key;
        for name in &wanted {
            let display_id = display_for(name, cx);
            let sender = sender.clone();
            let monitor = name.clone();
            let dwell = Duration::from_millis(u64::from(dwell_ms));
            let options = corner_options(corner, display_id);
            match cx.open_window(options, |window, cx| {
                cx.new(|cx| CornerView::new(monitor.clone(), dwell, sender, window, cx))
            }) {
                Ok(handle) => {
                    log::info!("hot corner {corner:?} on {name}");
                    self.watches.push(Watch {
                        monitor: name.clone(),
                        corner,
                        handle,
                    })
                }
                Err(error) => log::warn!("hot corner on {name}: {error:#}"),
            }
        }
        self.watches.len()
    }

    pub fn close_all(&mut self, cx: &mut App) {
        for watch in self.watches.drain(..) {
            let _ = watch
                .handle
                .update(cx, |_, window, _| window.remove_window());
        }
    }

    /// Test hook: as if the pointer entered the first watched corner and
    /// rested there (the dwell still applies).
    pub fn simulate_enter(&self, cx: &mut App) -> Result<String, String> {
        let watch = self.watches.first().ok_or("no hot corner is watched")?;
        watch
            .handle
            .update(cx, |view, window, cx| view.entered(false, window, cx))
            .map_err(|_| "the corner surface is gone".to_string())?;
        Ok(format!("entered {:?} on {}", watch.corner, watch.monitor))
    }
}

fn corner_options(corner: Corner, display_id: Option<DisplayId>) -> WindowOptions {
    let anchor = match corner {
        Corner::TopLeft | Corner::Off => Anchor::TOP | Anchor::LEFT,
        Corner::TopRight => Anchor::TOP | Anchor::RIGHT,
        Corner::BottomLeft => Anchor::BOTTOM | Anchor::LEFT,
        Corner::BottomRight => Anchor::BOTTOM | Anchor::RIGHT,
    };
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            size(px(SIZE), px(SIZE)),
        ))),
        titlebar: None,
        app_id: Some("orbit".into()),
        display_id,
        window_background: WindowBackgroundAppearance::Transparent,
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: NAMESPACE.into(),
            layer: Layer::Overlay,
            anchor,
            // -1: not pushed aside by other surfaces' exclusive zones (a bar
            // along the top edge would otherwise move the corner down).
            exclusive_zone: Some(px(-1.)),
            keyboard_interactivity: KeyboardInteractivity::None,
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The surface's view: a dwell timer that fires a toggle.
pub struct CornerView {
    monitor: String,
    dwell: Duration,
    sender: async_mpsc::UnboundedSender<Envelope>,
    /// Bumped on enter and leave; the timer fires only for its own entry.
    entry: Rc<RefCell<u64>>,
    inside: bool,
}

impl CornerView {
    fn new(
        monitor: String,
        dwell: Duration,
        sender: async_mpsc::UnboundedSender<Envelope>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Self {
        Self {
            monitor,
            dwell,
            sender,
            entry: Rc::default(),
            inside: false,
        }
    }

    /// The pointer came in (or moved inside). `held`: a button is down,
    /// which never engages (a drag passing through the corner).
    fn entered(&mut self, held: bool, _window: &mut Window, cx: &mut Context<Self>) {
        if self.inside || held {
            return;
        }
        self.inside = true;
        *self.entry.borrow_mut() += 1;
        let entry = *self.entry.borrow();
        let dwell = self.dwell;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(dwell).await;
            let _ = this.update(cx, |view, cx| {
                if view.inside && *view.entry.borrow() == entry {
                    view.engage(cx);
                }
            });
        })
        .detach();
    }

    fn left(&mut self) {
        self.inside = false;
        *self.entry.borrow_mut() += 1;
    }

    fn engage(&mut self, cx: &mut Context<Self>) {
        // A toggle while open would close, which a hot corner on an open
        // overview cannot reach anyway (the overview covers the corner), so
        // this only ever shows.
        //
        // The request takes the same road as one from the socket: Hyprland
        // names the monitor's logical size (width / scale). Sending it
        // unsized would leave the overlay to GPUI's display size, which is
        // wrong at fractional scale (a small overview on djinn at 1.25).
        // The query blocks, so it runs off the UI thread.
        let sender = self.sender.clone();
        let request = Request::Show {
            output: Some(self.monitor.clone()),
        };
        cx.background_executor()
            .spawn(async move {
                let (request, size) = crate::daemon::resolve_output(request);
                let (reply, _) = mpsc::channel();
                let _ = sender.unbounded_send(Envelope {
                    request,
                    size,
                    reply,
                });
            })
            .detach();
        self.left();
    }
}

impl Render for CornerView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let view = cx.entity();
        div()
            .size_full()
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                this.entered(event.pressed_button.is_some(), window, cx);
            }))
            .child(
                canvas(
                    |_, _, _| {},
                    move |_, _, window, _| {
                        let view = view.clone();
                        window.on_mouse_event(move |_: &MouseExitEvent, _, _, cx| {
                            view.update(cx, |this, _| this.left());
                        });
                    },
                )
                .size_full(),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// djinn, 2026-10-18: the daemon started with `corner = "top-left"`
    /// already in config.toml, synced once before Hyprland had reported
    /// any monitor, and never got a surface. The key must say "out of
    /// sync" once the monitors arrive, so the poll re-runs the sync.
    #[test]
    fn a_corner_synced_before_monitors_are_known_resyncs_when_they_arrive() {
        let at_startup = SyncKey {
            corner: Corner::TopLeft,
            dwell_ms: 150,
            scope: Monitors::Primary,
            monitors: Vec::new(),
        };
        assert!(at_startup.wanted().is_empty(), "nothing to watch yet");
        let corners = Corners {
            key: at_startup.clone(),
            ..Corners::default()
        };
        assert!(corners.in_sync(&at_startup));

        let linked = SyncKey {
            monitors: vec![("eDP-1".into(), true)],
            ..at_startup.clone()
        };
        assert!(!corners.in_sync(&linked), "the monitors arriving must trigger a sync");
        assert_eq!(linked.wanted(), ["eDP-1"]);

        let two = SyncKey {
            monitors: vec![("DP-1".into(), false), ("eDP-1".into(), true)],
            ..linked.clone()
        };
        assert_eq!(two.wanted(), ["eDP-1"], "primary: the focused monitor");
        let all = SyncKey {
            scope: Monitors::All,
            ..two.clone()
        };
        assert_eq!(all.wanted(), ["DP-1", "eDP-1"]);
        let off = SyncKey {
            corner: Corner::Off,
            ..two
        };
        assert!(off.wanted().is_empty());
    }
}
