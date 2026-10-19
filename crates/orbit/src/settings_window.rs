//! The settings window (ORB-010): a normal GPUI toplevel, Orbit-themed,
//! keyboard-complete and labelled like the overview. Every row edits the
//! settings file through the daemon ([`crate::daemon::commit_settings`]),
//! which applies the change at once; the file is the source of truth, so
//! `orbit config set` and this window never disagree.

use std::time::Duration;

use gpui::{
    App, Context, FocusHandle, FontWeight, KeyBinding, KeyDownEvent, ObjectFit, Role, SharedString,
    Window, actions, div, img, prelude::*, px,
};
use orbit_core::{
    Settings,
    settings::{
        Columns, Corner, Monitors, RailPosition, SearchScope, Shortcut, ThemeChoice, Titles,
        UpdateChannel, UpdateMode,
    },
};
use orbit_ui::theme::{self, Look, Theme};

use crate::{daemon, prefs::Prefs, updates::UpdateStatus};

actions!(
    orbit_settings,
    [
        Next, Previous, Up, Down, Left, Right, Activate, Backspace, Close
    ]
);

const CONTEXT: &str = "OrbitSettings";
pub const WIDTH: f32 = 780.;
pub const HEIGHT: f32 = 900.;

pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("tab", Next, Some(CONTEXT)),
        KeyBinding::new("shift-tab", Previous, Some(CONTEXT)),
        KeyBinding::new("down", Down, Some(CONTEXT)),
        KeyBinding::new("up", Up, Some(CONTEXT)),
        KeyBinding::new("left", Left, Some(CONTEXT)),
        KeyBinding::new("right", Right, Some(CONTEXT)),
        KeyBinding::new("enter", Activate, Some(CONTEXT)),
        KeyBinding::new("space", Activate, Some(CONTEXT)),
        KeyBinding::new("backspace", Backspace, Some(CONTEXT)),
        KeyBinding::new("escape", Close, Some(CONTEXT)),
    ]);
}

/// What a row edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Key {
    Shortcut,
    DoubleTap,
    Corner,
    Dwell,
    CornerCloses,
    CornerMonitors,
    Theme,
    Blur,
    Tint,
    ReducedMotion,
    Columns,
    Titles,
    Special,
    Rail,
    Follow,
    CloseOnActivate,
    SearchScope,
    Refresh,
    CaptureInactive,
    NeverCapture,
    IconTheme,
    LaunchAtLogin,
    UpdateStatus,
    UpdateMode,
    UpdateChannel,
    UpdateInterval,
    UpdateNotify,
    UpdateCheck,
    UpdateApply,
    UpdateSkip,
    UpdateNotes,
    AddInclude,
    Reset,
    Close,
}

enum Control {
    Toggle(bool),
    Choice {
        options: Vec<&'static str>,
        index: usize,
    },
    Number {
        value: i64,
        unit: &'static str,
    },
    Shortcut(String),
    Text(String),
    Button(&'static str),
}

struct Row {
    key: Key,
    label: &'static str,
    help: String,
    control: Control,
}

/// Where typed text goes while a row is being edited or a key recorded.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Capture {
    None,
    Recording,
    Text { key: Key, text: String },
    ConfirmInclude,
}

pub struct SettingsView {
    focus_handle: FocusHandle,
    focused: usize,
    capture: Capture,
    message: Option<(String, bool)>,
}

impl SettingsView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);
        cx.observe_global::<Prefs>(|_, cx| cx.notify()).detach();
        cx.observe_global::<UpdateStatus>(|_, cx| cx.notify())
            .detach();
        Self {
            focus_handle,
            focused: 0,
            capture: Capture::None,
            message: None,
        }
    }

    fn settings(cx: &App) -> Settings {
        cx.global::<Prefs>().settings.clone()
    }

    fn rows(cx: &App) -> Vec<Row> {
        let p = cx.global::<Prefs>();
        let u = cx.global::<UpdateStatus>();
        let s = &p.settings;
        let choice = |options: &[&'static str], index: usize| Control::Choice {
            options: options.to_vec(),
            index,
        };
        let bind_help = match (&p.bind, p.conflicts.is_empty()) {
            (Ok(()), _) => "Bound in Hyprland now. Press to record a new combination.".to_string(),
            (Err(_), false) => format!(
                "Not bound: already used by {}. Press to record another combination.",
                p.conflicts
                    .iter()
                    .map(|c| c.description.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            (Err(e), true) => format!("Not bound: {e}. Press to record a new combination."),
        };
        let include_help = match (&p.snippet_path, p.include_present) {
            (Some(path), true) => format!(
                "Permanent: hyprland.lua already requires {} (hypr/orbit.lua).",
                tilde(path)
            ),
            (Some(path), false) => format!(
                "To keep the shortcut after Hyprland restarts, hyprland.lua needs one line: \
                 {}  (the bind lives in {}).",
                crate::engage::INCLUDE_LINE,
                tilde(path)
            ),
            (None, _) => "No config directory found.".into(),
        };
        vec![
            Row {
                key: Key::Shortcut,
                label: "Engage Orbit",
                help: bind_help,
                control: Control::Shortcut(s.shortcut.label()),
            },
            Row {
                key: Key::AddInclude,
                label: "Keep it after restarts",
                help: include_help,
                control: Control::Button(if p.include_present {
                    "Added"
                } else {
                    "Add it for me…"
                }),
            },
            Row {
                key: Key::DoubleTap,
                label: "Double tap switches to the last window",
                help: "Pressing the shortcut twice within 350 ms goes straight back.".into(),
                control: Control::Toggle(s.behaviour.double_tap_last_window),
            },
            Row {
                key: Key::Corner,
                label: "Hot corner",
                help: format!(
                    "Resting the pointer in the corner opens Orbit ({} corner surface{} in place).",
                    p.corners,
                    if p.corners == 1 { "" } else { "s" }
                ),
                control: choice(
                    &Corner::ALL.map(Corner::label),
                    Corner::ALL
                        .iter()
                        .position(|c| *c == s.hot_corner.corner)
                        .unwrap_or(0),
                ),
            },
            Row {
                key: Key::Dwell,
                label: "Corner dwell",
                help: "How long the pointer rests before Orbit opens.".into(),
                control: Control::Number {
                    value: i64::from(s.hot_corner.dwell_ms),
                    unit: "ms",
                },
            },
            Row {
                key: Key::CornerCloses,
                label: "Corner also closes",
                help: "Resting in the corner while Orbit is open closes it.".into(),
                control: Control::Toggle(s.hot_corner.corner_closes),
            },
            Row {
                key: Key::CornerMonitors,
                label: "Corner on",
                help: "The focused monitor, or every monitor.".into(),
                control: choice(
                    &["Primary", "All"],
                    match s.hot_corner.monitors {
                        Monitors::Primary => 0,
                        Monitors::All => 1,
                    },
                ),
            },
            Row {
                key: Key::Theme,
                label: "Look",
                help: "Auto is glass when the desktop can be captured.".into(),
                control: choice(
                    &ThemeChoice::ALL.map(ThemeChoice::label),
                    ThemeChoice::ALL
                        .iter()
                        .position(|c| *c == s.appearance.theme)
                        .unwrap_or(0),
                ),
            },
            Row {
                key: Key::Blur,
                label: "Blur strength",
                help: "Backdrop blur, 1–20.".into(),
                control: Control::Number {
                    value: i64::from(s.appearance.blur),
                    unit: "",
                },
            },
            Row {
                key: Key::Tint,
                label: "Tint strength",
                help: "80–120 %. Under 100 % the text contrast guarantee over a bright wallpaper no longer holds.".into(),
                control: Control::Number {
                    value: i64::from(s.appearance.tint_percent),
                    unit: "%",
                },
            },
            Row {
                key: Key::ReducedMotion,
                label: "Reduced motion",
                help: "Fade only; no fly-in.".into(),
                control: Control::Toggle(s.appearance.reduced_motion),
            },
            Row {
                key: Key::Columns,
                label: "Columns",
                help: "Auto is three, two for one or two windows.".into(),
                control: choice(
                    &Columns::ALL.map(Columns::label),
                    Columns::ALL
                        .iter()
                        .position(|c| *c == s.layout.columns)
                        .unwrap_or(0),
                ),
            },
            Row {
                key: Key::Titles,
                label: "Window titles",
                help: "Under the app name.".into(),
                control: choice(
                    &Titles::ALL.map(Titles::label),
                    Titles::ALL
                        .iter()
                        .position(|c| *c == s.layout.titles)
                        .unwrap_or(0),
                ),
            },
            Row {
                key: Key::Special,
                label: "Show special workspaces",
                help: "Scratchpads in the rail.".into(),
                control: Control::Toggle(s.layout.special_workspaces),
            },
            Row {
                key: Key::Rail,
                label: "Rail position",
                help: "Workspaces above or below the windows.".into(),
                control: choice(
                    &RailPosition::ALL.map(RailPosition::label),
                    match s.layout.rail {
                        RailPosition::Top => 0,
                        RailPosition::Bottom => 1,
                    },
                ),
            },
            Row {
                key: Key::Follow,
                label: "Follow a moved window",
                help: "Plain drop or Enter moves and follows; Shift does the opposite.".into(),
                control: Control::Toggle(s.behaviour.follow_on_move),
            },
            Row {
                key: Key::CloseOnActivate,
                label: "Close when a window opens",
                help: "Off keeps Orbit open after choosing a window.".into(),
                control: Control::Toggle(s.behaviour.close_on_activate),
            },
            Row {
                key: Key::SearchScope,
                label: "Search",
                help: "Where typing looks.".into(),
                control: choice(
                    &SearchScope::ALL.map(SearchScope::label),
                    match s.behaviour.search_scope {
                        SearchScope::All => 0,
                        SearchScope::Workspace => 1,
                    },
                ),
            },
            Row {
                key: Key::Refresh,
                label: "Preview refresh",
                help: "100–5000 ms between frames.".into(),
                control: Control::Number {
                    value: i64::from(s.previews.refresh_ms),
                    unit: "ms",
                },
            },
            Row {
                key: Key::CaptureInactive,
                label: "Capture other workspaces",
                help: "Real content in every rail tile; every window is read as soon as Orbit opens.".into(),
                control: Control::Toggle(s.previews.capture_inactive_workspaces),
            },
            Row {
                key: Key::NeverCapture,
                label: "Never capture",
                help: "Window classes (comma separated) whose previews stay as icons.".into(),
                control: Control::Text(s.previews.never_capture.join(", ")),
            },
            Row {
                key: Key::IconTheme,
                label: "Icon theme",
                help: "Empty uses the desktop's icon theme.".into(),
                control: Control::Text(s.icons.theme.clone()),
            },
            Row {
                key: Key::LaunchAtLogin,
                label: "Launch at login",
                help: "A systemd user unit (orbit.service) starts the daemon with your session.".into(),
                control: Control::Toggle(s.startup.launch_at_login),
            },
            Row {
                key: Key::UpdateStatus,
                label: "Version",
                help: update_help(u, s),
                control: Control::Text(format!("Orbit {}", u.current)),
            },
            Row {
                key: Key::UpdateCheck,
                label: "Check for updates",
                help: match &u.last_result {
                    Some((text, _)) if !u.checking => text.clone(),
                    _ if u.checking => "Asking GitHub for the releases list…".into(),
                    _ => "Asks GitHub now, whatever the schedule says.".into(),
                },
                control: Control::Button(if u.checking { "Checking…" } else { "Check now" }),
            },
            Row {
                key: Key::UpdateApply,
                label: "Update now",
                help: match (u.available(), &u.updatable) {
                    (Some(latest), Ok(())) => format!(
                        "Downloads {}, verifies it, swaps it in and restarts Orbit. The previous build is kept for a rollback.",
                        latest.version
                    ),
                    (Some(_), Err(why)) => format!("Cannot update in place: {why}."),
                    (None, _) => "Nothing newer is known.".into(),
                },
                control: Control::Button(if u.applying { "Installing…" } else { "Update" }),
            },
            Row {
                key: Key::UpdateSkip,
                label: "Skip this version",
                help: "Stops offering the version above until a newer one appears.".into(),
                control: Control::Button("Skip"),
            },
            Row {
                key: Key::UpdateNotes,
                label: "Release notes",
                help: u
                    .state
                    .latest
                    .as_ref()
                    .map(|l| {
                        let first = l.notes.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
                        format!("{}: {}", l.tag, first.trim_start_matches('#').trim())
                    })
                    .unwrap_or_else(|| "Opens the release page in your browser.".into()),
                control: Control::Button("Open"),
            },
            Row {
                key: Key::UpdateMode,
                label: "New releases",
                help: if u.state.applied.is_some() && !u.state.auto_offered {
                    "That update went in by hand. Pick Install automatically and Orbit will do the next one itself, at a quiet moment, and tell you afterwards."
                        .into()
                } else {
                    "Tell me shows a pill in the overview and a notification; Install automatically applies them at a quiet moment and tells you afterwards."
                        .into()
                },
                control: Control::Choice {
                    options: UpdateMode::ALL.map(UpdateMode::label).to_vec(),
                    index: UpdateMode::ALL
                        .iter()
                        .position(|m| *m == s.update.mode)
                        .unwrap_or(0),
                },
            },
            Row {
                key: Key::UpdateChannel,
                label: "Channel",
                help: "Pre-releases are the alpha and beta builds; Stable only waits for releases not marked pre-release."
                    .into(),
                control: Control::Choice {
                    options: UpdateChannel::ALL.map(UpdateChannel::label).to_vec(),
                    index: UpdateChannel::ALL
                        .iter()
                        .position(|c| *c == s.update.channel)
                        .unwrap_or(0),
                },
            },
            Row {
                key: Key::UpdateInterval,
                label: "Check every",
                help: "Hours between checks of GitHub's releases list (1–168). Unchanged answers cost nothing."
                    .into(),
                control: Control::Number {
                    value: i64::from(s.update.check_interval_hours),
                    unit: "h",
                },
            },
            Row {
                key: Key::UpdateNotify,
                label: "Desktop notification",
                help: "One notification when a new version is first seen.".into(),
                control: Control::Toggle(s.update.notify),
            },
            Row {
                key: Key::Reset,
                label: "Reset to defaults",
                help: "Every setting back to its default (the shortcut is picked afresh).".into(),
                control: Control::Button("Reset"),
            },
            Row {
                key: Key::Close,
                label: "Close",
                help: "Esc closes too.".into(),
                control: Control::Button("Close"),
            },
        ]
    }

    fn section_of(key: Key) -> &'static str {
        match key {
            Key::Shortcut | Key::AddInclude | Key::DoubleTap => "Shortcut",
            Key::Corner | Key::Dwell | Key::CornerCloses | Key::CornerMonitors => "Hot corner",
            Key::Theme | Key::Blur | Key::Tint | Key::ReducedMotion => "Appearance",
            Key::Columns | Key::Titles | Key::Special | Key::Rail => "Layout",
            Key::Follow | Key::CloseOnActivate | Key::SearchScope => "Behaviour",
            Key::Refresh | Key::CaptureInactive | Key::NeverCapture => "Previews and privacy",
            Key::IconTheme => "Icons",
            Key::LaunchAtLogin => "Startup",
            Key::UpdateStatus
            | Key::UpdateCheck
            | Key::UpdateApply
            | Key::UpdateSkip
            | Key::UpdateNotes
            | Key::UpdateMode
            | Key::UpdateChannel
            | Key::UpdateInterval
            | Key::UpdateNotify => "Updates",
            Key::Reset | Key::Close => "",
        }
    }

    fn commit(&mut self, settings: Settings, cx: &mut Context<Self>) {
        match daemon::commit_settings(settings, cx) {
            Ok(()) => self.message = Some(("Saved".into(), false)),
            Err(error) => self.message = Some((error, true)),
        }
        cx.notify();
    }

    /// A step on a row: +1/-1 for numbers and choices, a flip for toggles.
    fn adjust(&mut self, key: Key, delta: i64, cx: &mut Context<Self>) {
        let mut s = Self::settings(cx);
        let cycle = |index: usize, len: usize| -> usize {
            ((index as i64 + delta).rem_euclid(len as i64)) as usize
        };
        match key {
            Key::DoubleTap => {
                s.behaviour.double_tap_last_window = !s.behaviour.double_tap_last_window
            }
            Key::Corner => {
                let i = Corner::ALL
                    .iter()
                    .position(|c| *c == s.hot_corner.corner)
                    .unwrap_or(0);
                s.hot_corner.corner = Corner::ALL[cycle(i, Corner::ALL.len())];
            }
            Key::Dwell => {
                s.hot_corner.dwell_ms =
                    (i64::from(s.hot_corner.dwell_ms) + 50 * delta).clamp(50, 2000) as u32
            }
            Key::CornerCloses => s.hot_corner.corner_closes = !s.hot_corner.corner_closes,
            Key::CornerMonitors => {
                s.hot_corner.monitors = match s.hot_corner.monitors {
                    Monitors::Primary => Monitors::All,
                    Monitors::All => Monitors::Primary,
                }
            }
            Key::Theme => {
                let i = ThemeChoice::ALL
                    .iter()
                    .position(|c| *c == s.appearance.theme)
                    .unwrap_or(0);
                s.appearance.theme = ThemeChoice::ALL[cycle(i, ThemeChoice::ALL.len())];
            }
            Key::Blur => {
                s.appearance.blur = (i64::from(s.appearance.blur) + delta).clamp(1, 20) as u32
            }
            Key::Tint => {
                s.appearance.tint_percent =
                    (i64::from(s.appearance.tint_percent) + 5 * delta).clamp(80, 120) as u32
            }
            Key::ReducedMotion => s.appearance.reduced_motion = !s.appearance.reduced_motion,
            Key::Columns => {
                let i = Columns::ALL
                    .iter()
                    .position(|c| *c == s.layout.columns)
                    .unwrap_or(0);
                s.layout.columns = Columns::ALL[cycle(i, Columns::ALL.len())];
            }
            Key::Titles => {
                let i = Titles::ALL
                    .iter()
                    .position(|c| *c == s.layout.titles)
                    .unwrap_or(0);
                s.layout.titles = Titles::ALL[cycle(i, Titles::ALL.len())];
            }
            Key::Special => s.layout.special_workspaces = !s.layout.special_workspaces,
            Key::Rail => {
                s.layout.rail = match s.layout.rail {
                    RailPosition::Top => RailPosition::Bottom,
                    RailPosition::Bottom => RailPosition::Top,
                }
            }
            Key::Follow => s.behaviour.follow_on_move = !s.behaviour.follow_on_move,
            Key::CloseOnActivate => s.behaviour.close_on_activate = !s.behaviour.close_on_activate,
            Key::SearchScope => {
                s.behaviour.search_scope = match s.behaviour.search_scope {
                    SearchScope::All => SearchScope::Workspace,
                    SearchScope::Workspace => SearchScope::All,
                }
            }
            Key::Refresh => {
                s.previews.refresh_ms =
                    (i64::from(s.previews.refresh_ms) + 100 * delta).clamp(100, 5000) as u32
            }
            Key::CaptureInactive => {
                s.previews.capture_inactive_workspaces = !s.previews.capture_inactive_workspaces
            }
            Key::LaunchAtLogin => s.startup.launch_at_login = !s.startup.launch_at_login,
            Key::UpdateMode => {
                let i = UpdateMode::ALL
                    .iter()
                    .position(|m| *m == s.update.mode)
                    .unwrap_or(0);
                s.update.mode = UpdateMode::ALL[cycle(i, UpdateMode::ALL.len())];
                // The one-time offer has been answered either way.
                let status = cx.global_mut::<UpdateStatus>();
                if !status.state.auto_offered {
                    status.state.auto_offered = true;
                    let _ = status.state.save();
                }
            }
            Key::UpdateChannel => {
                let i = UpdateChannel::ALL
                    .iter()
                    .position(|c| *c == s.update.channel)
                    .unwrap_or(0);
                s.update.channel = UpdateChannel::ALL[cycle(i, UpdateChannel::ALL.len())];
            }
            Key::UpdateInterval => {
                s.update.check_interval_hours =
                    (i64::from(s.update.check_interval_hours) + delta).clamp(1, 168) as u32
            }
            Key::UpdateNotify => s.update.notify = !s.update.notify,
            Key::Shortcut
            | Key::NeverCapture
            | Key::IconTheme
            | Key::AddInclude
            | Key::UpdateStatus
            | Key::UpdateCheck
            | Key::UpdateApply
            | Key::UpdateSkip
            | Key::UpdateNotes
            | Key::Reset
            | Key::Close => return,
        }
        self.commit(s, cx);
    }

    /// Enter or a click on a row.
    fn activate_row(&mut self, key: Key, window: &mut Window, cx: &mut Context<Self>) {
        match key {
            Key::Shortcut => {
                self.capture = Capture::Recording;
                self.message = Some(("Press the new key combination (Esc cancels)".into(), false));
                cx.notify();
            }
            Key::NeverCapture => {
                let s = Self::settings(cx);
                self.capture = Capture::Text {
                    key,
                    text: s.previews.never_capture.join(", "),
                };
                cx.notify();
            }
            Key::IconTheme => {
                let s = Self::settings(cx);
                self.capture = Capture::Text {
                    key,
                    text: s.icons.theme,
                };
                cx.notify();
            }
            Key::AddInclude => {
                if cx.global::<Prefs>().include_present {
                    self.message = Some(("Already in hyprland.lua".into(), false));
                } else {
                    self.capture = Capture::ConfirmInclude;
                }
                cx.notify();
            }
            Key::Reset => {
                let s = Settings {
                    shortcut: daemon::default_shortcut(),
                    ..Settings::default()
                };
                self.commit(s, cx);
            }
            Key::Close => window.remove_window(),
            Key::UpdateStatus => {}
            Key::UpdateCheck => {
                let reply = crate::updates::command("check", cx);
                self.message = Some((
                    if reply == "checking" {
                        "Checking for updates…".into()
                    } else {
                        reply
                    },
                    false,
                ));
                cx.notify();
            }
            Key::UpdateApply => {
                let reply = crate::updates::command("apply", cx);
                let error = reply.starts_with("error")
                    || reply.contains("failed")
                    || reply.starts_with("Cannot");
                self.message = Some((reply, error));
                cx.notify();
            }
            Key::UpdateSkip => {
                let reply = crate::updates::command("skip", cx);
                self.message = Some((reply.clone(), reply.starts_with("error")));
                cx.notify();
            }
            Key::UpdateNotes => {
                self.message = Some(match crate::updates::open_notes(cx) {
                    Ok(()) => ("Opening the release page".into(), false),
                    Err(error) => (error, true),
                });
                cx.notify();
            }
            other => self.adjust(other, 1, cx),
        }
    }

    fn finish_text(&mut self, cx: &mut Context<Self>) {
        if let Capture::Text { key, text } = std::mem::replace(&mut self.capture, Capture::None) {
            let mut s = Self::settings(cx);
            match key {
                Key::NeverCapture => {
                    s.previews.never_capture = text
                        .split(',')
                        .map(|c| c.trim().to_owned())
                        .filter(|c| !c.is_empty())
                        .collect()
                }
                Key::IconTheme => s.icons.theme = text.trim().to_owned(),
                _ => {}
            }
            self.commit(s, cx);
        }
    }

    fn row_count(cx: &App) -> usize {
        Self::rows(cx).len()
    }

    fn move_focus(&mut self, delta: isize, cx: &mut Context<Self>) {
        let n = Self::row_count(cx) as isize;
        self.focused = ((self.focused as isize + delta).rem_euclid(n)) as usize;
        cx.notify();
    }

    fn focused_key(&self, cx: &App) -> Option<Key> {
        Self::rows(cx).get(self.focused).map(|r| r.key)
    }

    // Actions.

    fn next(&mut self, _: &Next, _: &mut Window, cx: &mut Context<Self>) {
        if self.capture == Capture::None {
            self.move_focus(1, cx);
        }
    }

    fn previous(&mut self, _: &Previous, _: &mut Window, cx: &mut Context<Self>) {
        if self.capture == Capture::None {
            self.move_focus(-1, cx);
        }
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if self.capture == Capture::None {
            self.move_focus(1, cx);
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if self.capture == Capture::None {
            self.move_focus(-1, cx);
        }
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.capture == Capture::None
            && let Some(key) = self.focused_key(cx)
        {
            self.adjust(key, -1, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.capture == Capture::None
            && let Some(key) = self.focused_key(cx)
        {
            self.adjust(key, 1, cx);
        }
    }

    fn activate(&mut self, _: &Activate, window: &mut Window, cx: &mut Context<Self>) {
        match self.capture.clone() {
            Capture::Text { .. } => self.finish_text(cx),
            Capture::ConfirmInclude => {
                self.capture = Capture::None;
                match daemon::append_include(cx) {
                    Ok(()) => self.message = Some(("Added to hyprland.lua".into(), false)),
                    Err(error) => self.message = Some((error, true)),
                }
                cx.notify();
            }
            Capture::Recording => {}
            Capture::None => {
                if let Some(key) = self.focused_key(cx) {
                    self.activate_row(key, window, cx);
                }
            }
        }
    }

    fn backspace(&mut self, _: &Backspace, _: &mut Window, cx: &mut Context<Self>) {
        if let Capture::Text { text, .. } = &mut self.capture {
            text.pop();
            cx.notify();
        }
    }

    fn close(&mut self, _: &Close, window: &mut Window, cx: &mut Context<Self>) {
        if self.capture != Capture::None {
            self.capture = Capture::None;
            self.message = None;
            cx.notify();
            return;
        }
        window.remove_window();
    }

    /// Raw keys: the shortcut recorder and text rows.
    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        match &mut self.capture {
            Capture::Recording => {
                let ks = &event.keystroke;
                if ks.key == "escape" {
                    return;
                }
                let mut mods = Vec::new();
                if ks.modifiers.platform {
                    mods.push("SUPER".to_string());
                }
                if ks.modifiers.control {
                    mods.push("CTRL".to_string());
                }
                if ks.modifiers.alt {
                    mods.push("ALT".to_string());
                }
                if ks.modifiers.shift {
                    mods.push("SHIFT".to_string());
                }
                let key = xkb_name(&ks.key);
                if is_modifier_key(&ks.key) {
                    return;
                }
                if !mods.iter().any(|m| m != "SHIFT") {
                    self.message = Some(("Use Super, Ctrl or Alt with the key".into(), true));
                    cx.notify();
                    return;
                }
                let shortcut = Shortcut { mods, key };
                self.capture = Capture::None;
                let mut s = Self::settings(cx);
                s.shortcut = shortcut;
                self.commit(s, cx);
            }
            Capture::Text { text, .. } => {
                if let Some(c) = event.keystroke.key_char.as_deref()
                    && !event.keystroke.modifiers.control
                    && !event.keystroke.modifiers.platform
                    && c.chars().all(|ch| !ch.is_control())
                {
                    text.push_str(c);
                    cx.notify();
                }
            }
            _ => {}
        }
    }
}

/// GPUI's key names to xkb keysym names Hyprland binds.
fn xkb_name(key: &str) -> String {
    match key {
        "tab" => "Tab".into(),
        "space" => "space".into(),
        "enter" => "Return".into(),
        "backspace" => "BackSpace".into(),
        "delete" => "Delete".into(),
        "escape" => "Escape".into(),
        "up" => "Up".into(),
        "down" => "Down".into(),
        "left" => "Left".into(),
        "right" => "Right".into(),
        "home" => "Home".into(),
        "end" => "End".into(),
        "pageup" => "Prior".into(),
        "pagedown" => "Next".into(),
        "insert" => "Insert".into(),
        "`" => "grave".into(),
        "-" => "minus".into(),
        "=" => "equal".into(),
        "," => "comma".into(),
        "." => "period".into(),
        "/" => "slash".into(),
        ";" => "semicolon".into(),
        "'" => "apostrophe".into(),
        "[" => "bracketleft".into(),
        "]" => "bracketright".into(),
        "\\" => "backslash".into(),
        k if k.len() >= 2 && k.starts_with('f') && k[1..].chars().all(|c| c.is_ascii_digit()) => {
            k.to_uppercase()
        }
        k => k.to_owned(),
    }
}

fn is_modifier_key(key: &str) -> bool {
    matches!(
        key,
        "shift" | "control" | "ctrl" | "alt" | "platform" | "super" | "meta" | "win" | "capslock"
    )
}

impl Render for SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let look = Look::get(cx);
        let t: Theme = look.theme;
        let rows = Self::rows(cx);
        let prefs = cx.global::<Prefs>().clone();
        let focused = self.focused.min(rows.len().saturating_sub(1));
        let capture = self.capture.clone();
        let mut last_section = "";
        let mut body: Vec<gpui::AnyElement> = Vec::new();
        for (i, row) in rows.iter().enumerate() {
            let section = Self::section_of(row.key);
            if section != last_section {
                last_section = section;
                body.push(
                    div()
                        .mt(px(18.))
                        .mb(px(6.))
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.muted.rgba())
                        .child(section.to_uppercase())
                        .into_any_element(),
                );
            }
            body.push(self.row(t, row, i == focused, &capture, cx));
        }
        let message = self.message.clone();
        let confirm = (capture == Capture::ConfirmInclude).then(|| {
            div()
                .mt(px(12.))
                .p(px(14.))
                .rounded(px(12.))
                .bg(t.canvas.rgba())
                .border_1()
                .border_color(t.warning.alpha(0.6))
                .flex()
                .flex_col()
                .gap(px(8.))
                .child(
                    div()
                        .text_size(px(14.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(t.strong.rgba())
                        .child(format!(
                            "Append this to {}?",
                            prefs
                                .hyprland_lua
                                .as_ref()
                                .map(|p| tilde(p))
                                .unwrap_or_default()
                        )),
                )
                .child(
                    div()
                        .p(px(10.))
                        .rounded(px(8.))
                        .bg(t.surface.rgba())
                        .text_size(px(13.))
                        .text_color(t.text.rgba())
                        .whitespace_normal()
                        .child(crate::engage::include_block().trim().to_owned()),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(t.muted.rgba())
                        .child("Nothing else in the file changes. Enter appends it; Esc cancels."),
                )
                .child(
                    div()
                        .flex()
                        .gap(px(10.))
                        .child(button(t, "Append (Enter)", true).on_click(
                            cx.listener(|this, _, window, cx| this.activate(&Activate, window, cx)),
                        ))
                        .child(button(t, "Cancel (Esc)", false).on_click(
                            cx.listener(|this, _, window, cx| this.close(&Close, window, cx)),
                        )),
                )
        });
        div()
            .id("orbit-settings")
            .role(Role::Dialog)
            .aria_label("Orbit settings")
            .key_context(CONTEXT)
            .track_focus(&self.focus_handle)
            .on_action(cx.listener(Self::next))
            .on_action(cx.listener(Self::previous))
            .on_action(cx.listener(Self::down))
            .on_action(cx.listener(Self::up))
            .on_action(cx.listener(Self::left))
            .on_action(cx.listener(Self::right))
            .on_action(cx.listener(Self::activate))
            .on_action(cx.listener(Self::backspace))
            .on_action(cx.listener(Self::close))
            .on_key_down(cx.listener(Self::key_down))
            .when_some(look.font.clone(), |d, font| d.font_family(font))
            .size_full()
            .bg(t.canvas.rgba())
            .text_color(t.text.rgba())
            .flex()
            .flex_col()
            .child(
                div()
                    .id("header")
                    .role(Role::Heading)
                    .aria_level(1)
                    .aria_label("Orbit settings")
                    .flex()
                    .items_center()
                    .gap(px(12.))
                    .px(px(28.))
                    .py(px(20.))
                    .border_b_1()
                    .border_color(theme::WHITE.alpha(t.hairline_alpha))
                    .child(img(look.mark.clone()).size(px(36.)))
                    .child(match look.wordmark.clone() {
                        Some((wordmark, aspect)) => div()
                            .h(px(20.))
                            .w(px((20. * aspect).round()))
                            .child(img(wordmark).size_full().object_fit(ObjectFit::Fill)),
                        None => div()
                            .text_size(px(22.))
                            .font_weight(FontWeight::BOLD)
                            .text_color(t.strong.rgba())
                            .child("Orbit"),
                    })
                    .child(
                        div()
                            .text_size(px(15.))
                            .text_color(t.muted.rgba())
                            .child("Settings"),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(t.muted.rgba())
                            .child(prefs.path.as_ref().map(|p| tilde(p)).unwrap_or_default()),
                    ),
            )
            .child(
                div()
                    .id("rows")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .px(px(28.))
                    .pb(px(16.))
                    .flex()
                    .flex_col()
                    .children(body)
                    .children(confirm),
            )
            .child(
                div()
                    .id("status")
                    .role(Role::Status)
                    .aria_label(SharedString::from(
                        message
                            .as_ref()
                            .map(|(m, _)| m.clone())
                            .unwrap_or_else(|| "Ready".into()),
                    ))
                    .px(px(28.))
                    .py(px(12.))
                    .border_t_1()
                    .border_color(theme::WHITE.alpha(t.hairline_alpha))
                    .text_size(px(13.))
                    .text_color(match &message {
                        Some((_, true)) => t.error.rgba(),
                        _ => t.muted.rgba(),
                    })
                    .child(message.map(|(m, _)| m).unwrap_or_else(|| {
                        "Tab moves, Enter or Space changes, arrows adjust, Esc closes.".into()
                    })),
            )
    }
}

impl SettingsView {
    fn row(
        &self,
        t: Theme,
        row: &Row,
        focused: bool,
        capture: &Capture,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = row.key;
        let editing_text = match capture {
            Capture::Text { key: k, text } if *k == key => Some(text.clone()),
            _ => None,
        };
        let recording = focused && *capture == Capture::Recording && key == Key::Shortcut;
        let value_text: String = match &row.control {
            Control::Toggle(on) => if *on { "on" } else { "off" }.into(),
            Control::Choice { options, index } => options.get(*index).copied().unwrap_or("").into(),
            Control::Number { value, unit } => format!("{value} {unit}").trim().to_owned(),
            Control::Shortcut(label) => label.clone(),
            Control::Text(text) => text.clone(),
            Control::Button(label) => (*label).into(),
        };
        let control: gpui::AnyElement = match &row.control {
            Control::Toggle(on) => {
                let on = *on;
                div()
                    .w(px(44.))
                    .h(px(24.))
                    .rounded_full()
                    .bg(if on {
                        t.selection.rgba()
                    } else {
                        t.border.rgba()
                    })
                    .p(px(3.))
                    .flex()
                    .when(on, |d| d.justify_end())
                    .child(div().size(px(18.)).rounded_full().bg(t.strong.rgba()))
                    .into_any_element()
            }
            Control::Choice { options, index } => div()
                .flex()
                .gap(px(4.))
                .children(options.iter().enumerate().map(|(i, option)| {
                    let selected = i == *index;
                    div()
                        .px(px(10.))
                        .py(px(4.))
                        .rounded(px(8.))
                        .text_size(px(13.))
                        .bg(if selected {
                            t.selection.rgba()
                        } else {
                            t.surface.rgba()
                        })
                        .text_color(if selected {
                            t.strong.rgba()
                        } else {
                            t.text.rgba()
                        })
                        .child((*option).to_string())
                }))
                .into_any_element(),
            Control::Number { .. } => div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(
                    small_button(t, "−")
                        .on_click(cx.listener(move |this, _, _, cx| this.adjust(key, -1, cx))),
                )
                .child(
                    div()
                        .min_w(px(70.))
                        .text_size(px(14.))
                        .text_color(t.strong.rgba())
                        .flex()
                        .justify_center()
                        .child(value_text.clone()),
                )
                .child(
                    small_button(t, "+")
                        .on_click(cx.listener(move |this, _, _, cx| this.adjust(key, 1, cx))),
                )
                .into_any_element(),
            Control::Shortcut(label) => div()
                .px(px(12.))
                .py(px(6.))
                .rounded(px(10.))
                .bg(t.surface.rgba())
                .border_1()
                .border_color(if recording {
                    t.focus.rgba()
                } else {
                    theme::WHITE.alpha(t.hairline_alpha)
                })
                .text_size(px(14.))
                .font_weight(FontWeight::MEDIUM)
                .text_color(t.strong.rgba())
                .child(if recording {
                    "Press keys…".to_string()
                } else {
                    label.clone()
                })
                .into_any_element(),
            Control::Text(text) => div()
                .min_w(px(220.))
                .max_w(px(320.))
                .px(px(12.))
                .py(px(6.))
                .rounded(px(10.))
                .bg(t.surface.rgba())
                .border_1()
                .border_color(if editing_text.is_some() {
                    t.focus.rgba()
                } else {
                    theme::WHITE.alpha(t.hairline_alpha)
                })
                .text_size(px(14.))
                .text_color(t.text.rgba())
                .truncate()
                .child(match &editing_text {
                    Some(edit) => format!("{edit}|"),
                    None if text.is_empty() => "—".to_string(),
                    None => text.clone(),
                })
                .into_any_element(),
            Control::Button(label) => button(t, label, key == Key::AddInclude).into_any_element(),
        };
        div()
            .id(SharedString::from(format!("row-{key:?}")))
            .role(Role::Button)
            .aria_label(SharedString::from(format!("{}: {}", row.label, value_text)))
            .when(focused, |d| d.aria_active_descendant())
            .flex()
            .items_center()
            .gap(px(16.))
            .px(px(14.))
            .py(px(10.))
            .rounded(px(12.))
            .border_1()
            .border_color(if focused {
                t.focus.rgba()
            } else {
                gpui::transparent_black().into()
            })
            .bg(if focused {
                t.surface.alpha(0.6)
            } else {
                gpui::transparent_black().into()
            })
            .cursor_pointer()
            .on_click(cx.listener(move |this, _, window, cx| {
                this.focused = this.focused_index_of(key, cx).unwrap_or(this.focused);
                this.activate_row(key, window, cx);
            }))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(px(15.))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(t.strong.rgba())
                            .child(row.label),
                    )
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(t.muted.rgba())
                            .whitespace_normal()
                            .child(row.help.clone()),
                    ),
            )
            .child(control)
            .into_any_element()
    }

    fn focused_index_of(&self, key: Key, cx: &App) -> Option<usize> {
        Self::rows(cx).iter().position(|r| r.key == key)
    }
}

fn button(t: Theme, label: &str, primary: bool) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(format!("button-{label}")))
        .px(px(14.))
        .py(px(6.))
        .rounded(px(10.))
        .bg(if primary {
            t.selection.rgba()
        } else {
            t.surface.rgba()
        })
        .border_1()
        .border_color(theme::WHITE.alpha(t.hairline_alpha))
        .text_size(px(13.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(t.strong.rgba())
        .cursor_pointer()
        .child(label.to_owned())
}

fn small_button(t: Theme, label: &str) -> gpui::Stateful<gpui::Div> {
    div()
        .id(SharedString::from(format!("small-{label}")))
        .size(px(28.))
        .rounded(px(8.))
        .bg(t.surface.rgba())
        .border_1()
        .border_color(theme::WHITE.alpha(t.hairline_alpha))
        .text_size(px(16.))
        .text_color(t.strong.rgba())
        .flex()
        .items_center()
        .justify_center()
        .cursor_pointer()
        .child(label.to_owned())
}

/// Nothing here keeps time; the type is for callers that debounce saves.
#[allow(dead_code)]
pub const SAVE_DEBOUNCE: Duration = Duration::from_millis(0);

/// A path for people to read: the home directory as `~`.
fn tilde(path: &std::path::Path) -> String {
    let shown = path.display().to_string();
    match std::env::var("HOME") {
        Ok(home) if !home.is_empty() && shown.starts_with(&home) => {
            format!("~{}", &shown[home.len()..])
        }
        _ => shown,
    }
}

/// The Version row's help: what is known about newer releases.
fn update_help(u: &UpdateStatus, s: &Settings) -> String {
    if s.update.mode == UpdateMode::Off {
        return "Update checks are off.".into();
    }
    let when = match u.state.last_check {
        Some(at) => {
            let ago = orbit_update::now().saturating_sub(at);
            if ago < 90 {
                "just now".to_owned()
            } else if ago < 5400 {
                format!("{} min ago", ago / 60)
            } else if ago < 172_800 {
                format!("{} h ago", ago / 3600)
            } else {
                format!("{} days ago", ago / 86_400)
            }
        }
        None => "never".to_owned(),
    };
    match (u.available(), &u.state.failed) {
        (_, Some(failed)) => {
            format!("Update to {failed} failed and was rolled back. Last check {when}.")
        }
        (Some(latest), _) => format!("{} is available. Last check {when}.", latest.version),
        (None, _) => format!(
            "Latest on the {} channel. Last check {when}.",
            s.update.channel.name()
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_names_map_to_xkb() {
        assert_eq!(xkb_name("tab"), "Tab");
        assert_eq!(xkb_name("`"), "grave");
        assert_eq!(xkb_name("f12"), "F12");
        assert_eq!(xkb_name("a"), "a");
        assert_eq!(xkb_name("enter"), "Return");
        assert!(is_modifier_key("shift") && !is_modifier_key("q"));
    }
}
