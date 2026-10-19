//! A themed surface used to verify GPUI on the target host (ORB-001), and the
//! starting point for the layer-shell lifecycle spike (ORB-003).

use std::time::Duration;

use gpui::{
    App, Bounds, Context, FocusHandle, KeyBinding, Window, WindowBackgroundAppearance,
    WindowBounds, WindowKind, WindowOptions, actions, div, layer_shell::*, prelude::*, px, size,
};
use orbit_ui::theme;

actions!(orbit_smoke, [Dismiss]);

#[derive(Default)]
pub struct Options {
    pub overlay: bool,
    pub exit_after: Option<Duration>,
}

pub fn run(options: Options) {
    gpui_platform::application().run(move |cx: &mut App| {
        cx.bind_keys([
            KeyBinding::new("escape", Dismiss, None),
            KeyBinding::new("ctrl-q", Dismiss, None),
        ]);
        cx.on_action(|_: &Dismiss, cx| cx.quit());

        let window_options = if options.overlay {
            overlay_options()
        } else {
            window_options(cx)
        };
        let overlay = options.overlay;
        cx.open_window(window_options, |window, cx| {
            cx.new(|cx| Smoke::new(overlay, window, cx))
        })
        .expect("failed to open the smoke surface");
        cx.activate(true);

        if let Some(delay) = options.exit_after {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(delay).await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
}

fn window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(720.), px(420.)),
            cx,
        ))),
        app_id: Some("orbit".into()),
        ..Default::default()
    }
}

/// A non-reserving overlay covering the whole output, taking keyboard focus
/// only while it exists.
fn overlay_options() -> WindowOptions {
    WindowOptions {
        titlebar: None,
        app_id: Some("orbit".into()),
        window_background: WindowBackgroundAppearance::Transparent,
        kind: WindowKind::LayerShell(LayerShellOptions {
            namespace: "orbit".into(),
            layer: Layer::Overlay,
            anchor: Anchor::TOP | Anchor::BOTTOM | Anchor::LEFT | Anchor::RIGHT,
            exclusive_zone: Some(px(-1.)),
            keyboard_interactivity: KeyboardInteractivity::Exclusive,
            ..Default::default()
        }),
        ..Default::default()
    }
}

struct Smoke {
    overlay: bool,
    focus_handle: FocusHandle,
}

impl Smoke {
    fn new(overlay: bool, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self {
            overlay,
            focus_handle,
        }
    }
}

impl Render for Smoke {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let mode = if self.overlay {
            "layer-shell overlay"
        } else {
            "window"
        };

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_4()
            .bg(theme::MIDNIGHT.rgba())
            .text_color(theme::CLOUD.rgba())
            .child(div().text_size(px(28.)).child("Orbit"))
            .child(
                div()
                    .text_size(px(14.))
                    .text_color(theme::SILVER.rgba())
                    .child("A higher view for your desktop."),
            )
            .child(
                div()
                    .flex()
                    .gap_2()
                    .p_3()
                    .rounded(px(14.))
                    .bg(theme::SLATE.rgba())
                    .border_1()
                    .border_color(theme::VIOLET.rgba())
                    .children(theme::PALETTE.iter().map(|token| {
                        div()
                            .size(px(28.))
                            .rounded(px(8.))
                            .border_1()
                            .border_color(theme::GRAY.rgba())
                            .bg(token.rgba())
                    })),
            )
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(theme::SILVER.rgba())
                    .child(format!("GPUI smoke check · {mode} · Escape to close")),
            )
    }
}
