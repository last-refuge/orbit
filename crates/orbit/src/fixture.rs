//! Owned test windows (ORB-002). Tests may move, focus and close these, and
//! nothing else: they are recognized by their app id prefix.

use std::time::Duration;

use gpui::{
    App, Bounds, Context, FocusHandle, KeyBinding, SharedString, Window, WindowBounds,
    WindowOptions, actions, div, prelude::*, px, size,
};
use orbit_ui::theme;

pub const APP_ID_PREFIX: &str = "dev.orbit.fixture.";

actions!(orbit_fixture, [Close]);

/// Fixture border colors; capture checks use them to tell fixtures apart.
pub const ACCENTS: [theme::Token; 4] = [theme::VIOLET, theme::SKY, theme::AMBER, theme::CORAL];

/// A stable accent per name so fixtures are easy to tell apart in previews.
pub fn accent_for(name: &str) -> theme::Token {
    let sum: usize = name.bytes().map(usize::from).sum();
    ACCENTS[sum % ACCENTS.len()]
}

pub struct Options {
    pub name: String,
    pub exit_after: Option<Duration>,
}

/// Fixture names become part of an app id, so keep them to a safe alphabet.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 32
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn run(options: Options) {
    gpui_platform::application().run(move |cx: &mut App| {
        cx.bind_keys([KeyBinding::new("escape", Close, None)]);
        cx.on_action(|_: &Close, cx| cx.quit());

        let name: SharedString = options.name.clone().into();
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(480.), px(320.)),
                    cx,
                ))),
                app_id: Some(format!("{APP_ID_PREFIX}{name}")),
                titlebar: Some(gpui::TitlebarOptions {
                    title: Some(format!("Orbit fixture {name}").into()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            |window, cx| cx.new(|cx| Fixture::new(name, window, cx)),
        )
        .expect("failed to open the fixture window");

        if let Some(delay) = options.exit_after {
            cx.spawn(async move |cx| {
                cx.background_executor().timer(delay).await;
                cx.update(|cx| cx.quit());
            })
            .detach();
        }
    });
}

struct Fixture {
    name: SharedString,
    focus_handle: FocusHandle,
}

impl Fixture {
    fn new(name: SharedString, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        Self { name, focus_handle }
    }

    fn accent(&self) -> theme::Token {
        accent_for(&self.name)
    }
}

impl Render for Fixture {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .bg(theme::SLATE.rgba())
            .border_4()
            .border_color(self.accent().rgba())
            .text_color(theme::CLOUD.rgba())
            .child(div().text_size(px(40.)).child(self.name.clone()))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(theme::SILVER.rgba())
                    .child(format!("{APP_ID_PREFIX}{}", self.name)),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::valid_name;

    #[test]
    fn names_are_app_id_safe() {
        assert!(valid_name("alpha"));
        assert!(valid_name("ws-91-b"));
        assert!(!valid_name(""));
        assert!(!valid_name("Alpha"));
        assert!(!valid_name("a b"));
        assert!(!valid_name("a;rm"));
        assert!(!valid_name(&"x".repeat(33)));
    }
}
