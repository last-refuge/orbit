//! The resident process: overlay lifecycle (ORB-003) and the overview
//! wired to Hyprland and capture (ORB-006).
//!
//! `orbit daemon` owns one GPUI application and a private control socket.
//! Requests arriving together are applied in order to a desired visibility
//! and then reconciled once, so a burst of toggles always settles on the
//! state its parity implies, with at most one surface ever open.
//!
//! The model stays warm while hidden: a [`Link`] thread keeps the store in
//! step with Hyprland, and the overlay only adds the view and previews.
//!
//! ORB-010: the daemon also owns the settings (loaded from the config file,
//! reloaded when it changes, applied to the look, the capture worker, the
//! live shortcut bind, the hot corner surfaces and the login unit), the
//! settings window, and the shortcut's double tap.

use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    env,
    fs::{self, DirBuilder},
    io::{BufRead, BufReader, Write},
    os::unix::{
        fs::DirBuilderExt,
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    rc::Rc,
    sync::{Arc, mpsc},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context as _, Result, bail};
use futures::{StreamExt, channel::mpsc as async_mpsc};
use gpui::{
    App, AsyncApp, Bounds, DisplayId, Entity, Pixels, PlatformDisplay, QuitMode, Size,
    WindowBackgroundAppearance, WindowBounds, WindowHandle, WindowKind, WindowOptions,
    layer_shell::*, point, prelude::*, px, size,
};
use orbit_core::{
    Intent, OpId, Settings,
    settings::{Columns, Shortcut, ThemeChoice, Titles},
};
use orbit_hyprland::{
    Instance,
    control::{Control, Dest},
    link::{FromLink, Link, ToLink},
};
use orbit_ui::{
    desk::{Desk, Outbound},
    icons::Icons,
    overview::{self, OverviewView},
    theme::{self, Look, OverviewPrefs},
};

use crate::{
    capture::X11_FIXTURE_CLASS,
    corner::Corners,
    engage::{self, LiveBind},
    fixture::APP_ID_PREFIX,
    prefs::Prefs,
    previews::{Captured, Previews},
    settings_window::{self, SettingsView},
};

/// Two shortcut presses this close together are a double tap.
const DOUBLE_TAP: Duration = Duration::from_millis(350);
/// How often the settings file is checked for changes.
const CONFIG_POLL: Duration = Duration::from_millis(500);

/// How often the store's clock advances (move timeouts, health checks).
const TICK: Duration = Duration::from_millis(100);

const REPLY_TIMEOUT: Duration = Duration::from_secs(3);

/// A control request, already parsed. `output: None` means the monitor that
/// had focus when the request arrived (or the compositor's choice).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    Show {
        output: Option<String>,
    },
    Hide,
    Toggle {
        output: Option<String>,
    },
    Status,
    Quit,
    /// The overview's state as JSON, for scripted checks (no titles).
    State,
    /// A test hook: keys, clicks or a drag through GPUI's input dispatch.
    /// Needs `orbit daemon --test-hooks`.
    Drive(String),
    /// Open the settings window (closing the overview first).
    Settings,
    /// `update check|apply|skip|rollback|notes|reload|restart|seen`.
    Update(String),
}

impl Request {
    pub fn parse(line: &str) -> Result<Self, String> {
        if let Some(rest) = line.trim().strip_prefix("drive ") {
            return Ok(Request::Drive(rest.trim().to_owned()));
        }
        if let Some(rest) = line.trim().strip_prefix("update ") {
            return Ok(Request::Update(rest.trim().to_owned()));
        }
        let mut words = line.split_whitespace();
        let command = words.next().unwrap_or_default();
        let output = words.next().map(str::to_owned);
        if words.next().is_some() {
            return Err("too many arguments".into());
        }
        match (command, output) {
            ("show", output) => Ok(Request::Show { output }),
            ("toggle", output) => Ok(Request::Toggle { output }),
            ("hide", None) => Ok(Request::Hide),
            ("status", None) => Ok(Request::Status),
            ("quit", None) => Ok(Request::Quit),
            ("state", None) => Ok(Request::State),
            ("settings", None) => Ok(Request::Settings),
            ("", _) => Err("empty request".into()),
            (other, _) => Err(format!("unknown request `{other}`")),
        }
    }

    pub fn encode(&self) -> String {
        match self {
            Request::Show { output } => join("show", output),
            Request::Toggle { output } => join("toggle", output),
            Request::Hide => "hide".into(),
            Request::Status => "status".into(),
            Request::Quit => "quit".into(),
            Request::State => "state".into(),
            Request::Settings => "settings".into(),
            Request::Drive(command) => format!("drive {command}"),
            Request::Update(words) => format!("update {words}"),
        }
    }
}

fn join(command: &str, output: &Option<String>) -> String {
    match output {
        Some(output) => format!("{command} {output}"),
        None => command.into(),
    }
}

/// What a batch of requests asks for, given the visibility before it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Desired {
    pub visible: bool,
    pub output: Option<String>,
    /// Logical size of `output`, when the compositor reported one.
    pub size: Option<(f32, f32)>,
    pub quit: bool,
}

impl Desired {
    pub fn apply(&mut self, request: &Request, size: Option<(f32, f32)>) {
        match request {
            Request::Show { output } => {
                self.visible = true;
                self.output = output.clone();
                self.size = size;
            }
            Request::Hide => self.visible = false,
            Request::Toggle { output } => {
                self.visible = !self.visible;
                if self.visible {
                    self.output = output.clone();
                    self.size = size;
                }
            }
            Request::Status | Request::State | Request::Drive(_) | Request::Update(_) => {}
            // The overview holds the keyboard; the settings window needs it.
            Request::Settings => self.visible = false,
            Request::Quit => {
                self.visible = false;
                self.quit = true;
            }
        }
    }
}

pub fn socket_path() -> Result<PathBuf> {
    let runtime = env::var_os("XDG_RUNTIME_DIR").context("XDG_RUNTIME_DIR is not set")?;
    Ok(PathBuf::from(runtime).join("orbit").join("orbit.sock"))
}

/// Sends one request to the running daemon and returns its reply.
pub fn send(request: &Request) -> Result<String> {
    let path = socket_path()?;
    let mut stream = UnixStream::connect(&path).with_context(|| {
        format!(
            "no Orbit daemon at {} (start one with `orbit daemon`)",
            path.display()
        )
    })?;
    stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
    writeln!(stream, "{}", request.encode())?;
    let mut reply = String::new();
    BufReader::new(stream).read_line(&mut reply)?;
    let reply = reply.trim_end().to_owned();
    match reply.strip_prefix("error: ") {
        Some(error) => bail!("{error}"),
        None => Ok(reply),
    }
}

pub(crate) struct Envelope {
    pub(crate) request: Request,
    pub(crate) size: Option<(f32, f32)>,
    pub(crate) reply: mpsc::Sender<String>,
}

/// The overlay, reachable from the settings window's commits.
#[derive(Clone)]
pub(crate) struct OverlayRef(pub(crate) Rc<RefCell<Overlay>>);

impl gpui::Global for OverlayRef {}

#[derive(Default)]
pub struct Options {
    /// Accept `drive` requests (scripted end-to-end checks).
    pub test_hooks: bool,
    /// Started by the updater on a new binary: the version it replaced.
    pub after_update: Option<String>,
    /// The high-contrast theme (plan §04).
    pub high_contrast: bool,
    /// No travel animation.
    pub reduced_motion: bool,
    /// Opaque layers instead of translucent ones over the desktop.
    pub opaque: bool,
    /// The frosted-glass layers (the default); `Some(false)` keeps the
    /// denser standard layers.
    pub glass: Option<bool>,
}

/// What a dispatch ticket was for, to route its result.
enum Ticket {
    SwapWorkspaces((u64, orbit_core::Epoch)),
    Move(OpId),
    Control,
}

pub fn run(options: Options) -> Result<()> {
    let listener = bind()?;
    let (sender, receiver) = async_mpsc::unbounded::<Envelope>();
    let sender_for_desk = sender.clone();
    thread::Builder::new()
        .name("orbit-control".into())
        .spawn(move || accept(listener, sender))?;

    // Hidden is the daemon's resting state, so closing the last surface must
    // not end the process.
    gpui_platform::application()
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx: &mut App| {
            overview::bind_keys(cx);
            settings_window::bind_keys(cx);
            if let Err(error) = Look::register_fonts(cx) {
                log::warn!("loading the bundled Inter failed: {error:#}");
            }
            let font = Look::pick_font(cx);
            match font.as_deref() {
                Some("Inter") => {}
                Some(other) => log::info!("Inter is unavailable; using {other}"),
                None => log::info!("Inter is unavailable; using the default UI font"),
            }
            // Settings: the file, or defaults written on first run (with a
            // shortcut nothing else uses).
            let instance = Instance::from_env().ok();
            let path = Settings::path();
            let mut prefs = match path.as_deref().map(Settings::load) {
                Some(Ok(settings)) => Prefs::new(settings, path.clone()),
                Some(Err(error)) => {
                    log::warn!("settings not loaded, using defaults: {error}");
                    let mut prefs = Prefs::new(Settings::default(), path.clone());
                    prefs.error = Some(error);
                    prefs
                }
                None => Prefs::new(Settings::default(), None),
            };
            if let Some(path) = &path
                && !path.exists()
                && prefs.error.is_none()
            {
                let settings = Settings {
                    shortcut: engage::pick_default(instance.as_ref()),
                    ..Settings::default()
                };
                if let Err(error) = prefs.save(settings) {
                    log::warn!("settings not written: {error}");
                }
            }
            cx.set_global(prefs);
            // A placeholder look until the settings are applied below.
            cx.set_global(Look::with_icons(
                theme::GLASS,
                font.clone(),
                options.reduced_motion,
                Arc::new(Icons::new(None, Vec::new())),
            ));
            let (desk, previews) = match start_workers(cx, sender_for_desk.clone()) {
                Ok((desk, previews)) => {
                    if options.test_hooks {
                        // Scripted runs open the overview over the user's
                        // real workspaces: capture only Orbit's own windows.
                        desk.update(cx, |desk, _| {
                            desk.restrict_capture(|class| {
                                class.starts_with(APP_ID_PREFIX) || class == X11_FIXTURE_CLASS
                            })
                        });
                    }
                    (desk, previews)
                }
                Err(error) => {
                    eprintln!("orbit daemon: {error:#}");
                    cx.quit();
                    return;
                }
            };
            let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("orbit"));
            let overlay = Rc::new(RefCell::new(Overlay {
                desk,
                test_hooks: options.test_hooks,
                window: None,
                opens: 0,
                last_open: Rc::default(),
                settings_window: None,
                corners: Corners::default(),
                bind: LiveBind::default(),
                surface: None,
                instance,
                exe,
                sender: sender_for_desk.clone(),
                previews,
                font,
                options: LookOptions {
                    high_contrast: options.high_contrast,
                    opaque: options.opaque,
                    glass: options.glass,
                    reduced_motion: options.reduced_motion,
                },
                icon_override: None,
                launch_applied: None,
                last_engage: None,
            }));
            cx.set_global(OverlayRef(overlay.clone()));
            cx.set_global(crate::updates::UpdateStatus::new(
                options.after_update.as_deref(),
            ));
            cx.set_global(orbit_ui::theme::UpdateBadge::default());
            apply_settings(&mut overlay.borrow_mut(), cx);
            crate::updates::start(OverlayRef(overlay.clone()), cx);
            // Hot-reload the settings file.
            let watched = overlay.clone();
            cx.spawn(async move |cx| {
                loop {
                    cx.background_executor().timer(CONFIG_POLL).await;
                    cx.update(|cx| {
                        let changed = cx.global_mut::<Prefs>().reload_if_changed();
                        if changed {
                            log::info!("settings file changed; applying");
                            apply_settings(&mut watched.borrow_mut(), cx);
                        } else if let Ok(mut overlay) = watched.try_borrow_mut() {
                            // Monitors reported since the last apply (the
                            // first one runs before any are known).
                            let corners = sync_corners(&mut overlay, cx);
                            let prefs = cx.global_mut::<Prefs>();
                            if prefs.corners != corners {
                                prefs.corners = corners;
                            }
                        }
                    });
                }
            })
            .detach();
            cx.spawn(async move |cx| serve(receiver, overlay, cx).await)
                .detach();
        });
    if let Ok(path) = socket_path() {
        let _ = fs::remove_file(path);
    }
    Ok(())
}

/// The icon theme: `$ORBIT_ICON_THEME`, else GTK's settings files, else
/// the GNOME setting Omarchy writes (`gsettings`, asked once), else hicolor.
fn icon_theme() -> Option<String> {
    if let Some(theme) = env::var("ORBIT_ICON_THEME").ok().filter(|t| !t.is_empty()) {
        return Some(theme);
    }
    if let Some(theme) = Icons::theme_from_gtk_settings() {
        return Some(theme);
    }
    let output = std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "icon-theme"])
        .output()
        .ok()?;
    let text = String::from_utf8(output.stdout).ok()?;
    let theme = text.trim().trim_matches('\'').trim();
    (!theme.is_empty()).then(|| theme.to_owned())
}

/// Starts the Hyprland link and the capture worker and connects them to a
/// new [`Desk`]. Worker output reaches the desk on the UI thread.
fn start_workers(
    cx: &mut App,
    requests: async_mpsc::UnboundedSender<Envelope>,
) -> Result<(Entity<Desk>, Previews)> {
    let (from_link, mut link_rx) = async_mpsc::unbounded::<FromLink>();
    let link = Link::spawn(orbit_hyprland::Instance::from_env, move |message| {
        let _ = from_link.unbounded_send(message);
    })
    .context("starting the Hyprland link")?;
    let (frames, mut frame_rx) = async_mpsc::unbounded::<Captured>();
    let previews = Previews::spawn(move |captured| {
        let _ = frames.unbounded_send(captured);
    })
    .context("starting the capture worker")?;

    let tickets: Rc<RefCell<HashMap<u64, Ticket>>> = Rc::default();
    let next_ticket = Rc::new(Cell::new(0u64));
    let worker = previews.clone();
    let desk = cx.new({
        let tickets = tickets.clone();
        let previews = previews.clone();
        move |_| {
            Desk::new(move |out| {
                let dispatch = |ticket: Ticket, control: Control| {
                    let id = next_ticket.get() + 1;
                    next_ticket.set(id);
                    tickets.borrow_mut().insert(id, ticket);
                    link.send(ToLink::Dispatch {
                        ticket: id,
                        control,
                    });
                };
                match out {
                    Outbound::SwapWorkspaces {
                        source,
                        target,
                        source_windows,
                        target_windows,
                        origin,
                    } => dispatch(
                        Ticket::SwapWorkspaces(origin),
                        Control::SwapWorkspaces {
                            source: source.0,
                            target: target.0,
                            source_windows,
                            target_windows,
                        },
                    ),
                    Outbound::Query(kinds) => link.send(ToLink::Query(kinds)),
                    Outbound::Move { op, address, to } => dispatch(
                        Ticket::Move(op),
                        Control::MoveSilent {
                            address,
                            dest: Dest::from_workspace(to.id, &to.name),
                        },
                    ),
                    Outbound::Focus { address } => {
                        dispatch(Ticket::Control, Control::FocusWindow { address })
                    }
                    Outbound::Raise { address } => {
                        dispatch(Ticket::Control, Control::RaiseWindow { address })
                    }
                    Outbound::Switch { to } => dispatch(
                        Ticket::Control,
                        Control::FocusWorkspace {
                            dest: Dest::from_workspace(to.id, &to.name),
                        },
                    ),
                    Outbound::Previews(addresses) => previews.want(addresses),
                    Outbound::Backdrop { output, size } => previews.backdrop(output, size),
                    Outbound::OpenSettings => {
                        let (reply, _) = mpsc::channel();
                        let _ = requests.unbounded_send(Envelope {
                            request: Request::Settings,
                            size: None,
                            reply,
                        });
                    }
                }
            })
        }
    });

    let weak = desk.downgrade();
    cx.spawn(async move |cx| {
        while let Some(message) = link_rx.next().await {
            let delivered = weak.update(cx, |desk, cx| match message {
                FromLink::Connected {
                    link,
                    version,
                    syntax,
                } => {
                    log::info!("connected to Hyprland {version} ({syntax:?} dispatch)");
                    desk.connected(link, version, cx);
                }
                FromLink::Event { link, event } => desk.event(link, event, cx),
                FromLink::Gap { link } => desk.gap(link, cx),
                FromLink::Refresh { link, refresh } => desk.refreshed(link, refresh, cx),
                FromLink::Dispatched { ticket, result, .. } => {
                    // Release the borrow first: handling the result can
                    // dispatch again (a confirmed "move and follow").
                    let ticket = tickets.borrow_mut().remove(&ticket);
                    match ticket {
                        Some(Ticket::SwapWorkspaces(origin)) => {
                            desk.workspace_swapped(origin, result, cx)
                        }
                        Some(Ticket::Move(op)) => desk.move_dispatched(op, result, cx),
                        Some(Ticket::Control) => {
                            if let Err(error) = result {
                                desk.control_failed(error, cx);
                            }
                        }
                        None => {}
                    }
                }
                FromLink::Disconnected { link, error } => {
                    log::warn!("lost Hyprland: {error}");
                    desk.disconnected(link, error, cx);
                }
            });
            if delivered.is_err() {
                break;
            }
        }
    })
    .detach();

    let weak = desk.downgrade();
    cx.spawn(async move |cx| {
        while let Some(captured) = frame_rx.next().await {
            let delivered = weak.update(cx, |desk, cx| match captured {
                Captured::Window { address, image } => desk.set_preview(address, image, cx),
                Captured::Backdrop { image } => desk.set_backdrop(image, cx),
            });
            if delivered.is_err() {
                break;
            }
        }
    })
    .detach();

    let weak = desk.downgrade();
    cx.spawn(async move |cx| {
        loop {
            cx.background_executor().timer(TICK).await;
            if weak.update(cx, |desk, cx| desk.tick(cx)).is_err() {
                break;
            }
        }
    })
    .detach();
    Ok((desk, worker))
}

/// Binds the socket inside a private directory, replacing a stale socket but
/// refusing to start a second daemon.
fn bind() -> Result<UnixListener> {
    let path = socket_path()?;
    let dir = path.parent().expect("socket path has a parent");
    DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
        .with_context(|| format!("creating {}", dir.display()))?;
    if path.exists() {
        if UnixStream::connect(&path).is_ok() {
            bail!("an Orbit daemon is already running at {}", path.display());
        }
        fs::remove_file(&path)?;
    }
    UnixListener::bind(&path).with_context(|| format!("binding {}", path.display()))
}

fn accept(listener: UnixListener, sender: async_mpsc::UnboundedSender<Envelope>) {
    for stream in listener.incoming().flatten() {
        let sender = sender.clone();
        thread::spawn(move || {
            if let Err(error) = handle(stream, &sender) {
                log::debug!("control connection: {error:#}");
            }
        });
    }
}

fn handle(stream: UnixStream, sender: &async_mpsc::UnboundedSender<Envelope>) -> Result<()> {
    stream.set_read_timeout(Some(REPLY_TIMEOUT))?;
    let mut line = String::new();
    BufReader::new(&stream).read_line(&mut line)?;
    let reply = match Request::parse(&line) {
        Ok(request) => {
            let (request, size) = resolve_output(request);
            let (reply, replied) = mpsc::channel();
            sender
                .unbounded_send(Envelope {
                    request,
                    size,
                    reply,
                })
                .context("daemon is shutting down")?;
            replied
                .recv_timeout(REPLY_TIMEOUT)
                .unwrap_or_else(|_| "error: timed out".into())
        }
        Err(error) => format!("error: {error}"),
    };
    writeln!(&stream, "{reply}")?;
    Ok(())
}

/// Resolves, off the UI thread, "the invocation monitor" (Hyprland's focused
/// monitor when the request arrived) and the target monitor's logical size.
///
/// The size matters: GPUI derives output sizes from the integer `wl_output`
/// scale, which is wrong at fractional scales, and a zero-size layer surface
/// breaks wgpu's swapchain on real GPUs (ORB-003 on djinn). Hyprland's
/// `width / scale` is the logical size the compositor will configure.
pub(crate) fn resolve_output(request: Request) -> (Request, Option<(f32, f32)>) {
    let monitors = orbit_hyprland::Instance::from_env()
        .ok()
        .and_then(|instance| instance.query(orbit_hyprland::Query::Monitors).ok());
    resolve_output_in(request, monitors.as_ref())
}

/// [`resolve_output`] over an already fetched `hyprctl -j monitors`. Every
/// way of opening the overview (the socket, the hot corner) goes through
/// this, so they all size the surface the same way.
fn resolve_output_in(
    request: Request,
    monitors: Option<&serde_json::Value>,
) -> (Request, Option<(f32, f32)>) {
    let monitors = monitors.and_then(|value| value.as_array());
    let pick = |name: Option<&str>| -> Option<(String, Option<(f32, f32)>)> {
        let monitor = monitors?.iter().find(|monitor| match name {
            Some(name) => monitor["name"] == name,
            None => monitor["focused"] == true,
        })?;
        let scale = monitor["scale"].as_f64().filter(|scale| *scale > 0.0)?;
        let logical = |key: &str| monitor[key].as_f64().map(|px| (px / scale).round() as f32);
        let size = logical("width").zip(logical("height"));
        Some((monitor["name"].as_str()?.to_owned(), size))
    };
    let resolve = |output: Option<String>| match pick(output.as_deref()) {
        Some((name, size)) => (Some(name), size),
        None => (output, None),
    };
    match request {
        Request::Show { output } => {
            let (output, size) = resolve(output);
            (Request::Show { output }, size)
        }
        Request::Toggle { output } => {
            let (output, size) = resolve(output);
            (Request::Toggle { output }, size)
        }
        other => (other, None),
    }
}

/// The command-line look overrides (scripted checks use them).
#[derive(Clone, Copy, Default)]
struct LookOptions {
    high_contrast: bool,
    opaque: bool,
    glass: Option<bool>,
    reduced_motion: bool,
}

pub(crate) struct Overlay {
    desk: Entity<Desk>,
    test_hooks: bool,
    window: Option<(WindowHandle<OverviewView>, Option<String>)>,
    opens: u32,
    last_open: Rc<Cell<Option<Duration>>>,
    settings_window: Option<WindowHandle<SettingsView>>,
    corners: Corners,
    bind: LiveBind,
    /// The open surface's requested logical size and whether it came from
    /// Hyprland (right) rather than GPUI's display (wrong at fractional
    /// scale).
    surface: Option<((f32, f32), bool)>,
    instance: Option<Instance>,
    exe: PathBuf,
    sender: async_mpsc::UnboundedSender<Envelope>,
    previews: Previews,
    font: Option<gpui::SharedString>,
    options: LookOptions,
    /// The icon theme override the current icons were built with.
    icon_override: Option<String>,
    /// The launch-at-login state last applied.
    launch_applied: Option<bool>,
    /// When the shortcut last opened the overview (the double tap).
    last_engage: Option<Instant>,
}

impl Overlay {
    /// A surface the user dismissed with Escape is gone even though we still
    /// hold its handle.
    fn prune(&mut self, cx: &mut App) {
        if let Some((handle, _)) = &self.window
            && handle.update(cx, |_, _, _| ()).is_err()
        {
            self.window = None;
            self.surface = None;
        }
    }

    fn desired(&self) -> Desired {
        Desired {
            visible: self.window.is_some(),
            output: self.window.as_ref().and_then(|(_, output)| output.clone()),
            size: None,
            quit: false,
        }
    }

    fn reconcile(&mut self, desired: &Desired, cx: &mut App) -> Result<(), String> {
        let current_output = self.window.as_ref().map(|(_, output)| output.clone());
        let wanted = desired.visible.then(|| desired.output.clone());
        if current_output == wanted {
            return Ok(());
        }
        // Resolve first, so a bad output name leaves the current surface alone.
        let display = match &wanted {
            Some(Some(name)) => Some(display_for(name, cx)?),
            _ => None,
        };
        let size = desired
            .size
            .map(|(width, height)| size(px(width), px(height)))
            .or_else(|| display.as_ref().map(|display| display.bounds().size))
            .or_else(|| cx.primary_display().map(|display| display.bounds().size))
            .unwrap_or(gpui::DEFAULT_WINDOW_SIZE);
        let from_hyprland = desired.size.is_some();
        let display_id = display.map(|display| display.id());
        if let Some((handle, _)) = self.window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
            // "Updated to X" has been seen once the overview it was shown in closes.
            crate::updates::notice_seen(cx);
        }
        self.surface = None;
        let Some(output) = wanted else {
            return Ok(());
        };
        let started = Instant::now();
        self.last_open.set(None);
        let last_open = self.last_open.clone();
        let desk = self.desk.clone();
        let session = desk.update(cx, |desk, cx| desk.open(output.as_deref(), cx));
        let handle = cx
            .open_window(overlay_options(display_id, size), |window, cx| {
                cx.new(|cx| {
                    // Time to first frame built, not to display: see plan §11.
                    let first_frame = Box::new(move || last_open.set(Some(started.elapsed())));
                    OverviewView::new(desk, session, first_frame, window, cx)
                })
            })
            .map_err(|error| {
                self.desk.update(cx, |desk, cx| desk.close(session, cx));
                format!("opening overlay: {error:#}")
            })?;
        self.opens += 1;
        self.window = Some((handle, output));
        self.surface = Some((
            (f32::from(size.width), f32::from(size.height)),
            from_hyprland,
        ));
        Ok(())
    }

    fn state(&self, cx: &App) -> String {
        let mut state = self.desk.read(cx).state_json();
        state["visible"] = self.window.is_some().into();
        state["surface"] = match (&self.window, self.surface) {
            (Some(_), Some(((width, height), from_hyprland))) => serde_json::json!({
                "size": [width, height],
                "size_from": if from_hyprland { "hyprland" } else { "gpui" },
            }),
            _ => serde_json::Value::Null,
        };
        state["settings"] = cx.global::<Prefs>().summary_json();
        state["update"] = cx.global::<crate::updates::UpdateStatus>().json();
        state["settings_window"] = self
            .settings_window
            .as_ref()
            .is_some_and(|w| cx.windows().contains(&(*w).into()))
            .into();
        let look = Look::get(cx);
        state["look"] = serde_json::json!({
            "theme": look.theme.name,
            "font": look.font.as_deref().unwrap_or("default"),
            "reduced_motion": look.reduced_motion,
            "opaque": look.theme.is_opaque(),
            "glass": look.theme.is_glass(),
            "icon_theme": look.icons.theme_name(),
            "scrim_alpha": look.theme.scrim_alpha,
            "panel_alpha": look.theme.panel_alpha,
        });
        // Whether each card has a real app icon (by class, never title).
        // (`get_mut`, not indexing: `IndexMut` would turn a null overview
        // into an object.)
        if let Some(cards) = state
            .get_mut("overview")
            .and_then(|o| o.get_mut("cards"))
            .and_then(|c| c.as_array_mut())
        {
            for card in cards {
                let icon = card["class"]
                    .as_str()
                    .is_some_and(|class| look.icons.has_icon(class));
                card["icon"] = icon.into();
            }
        }
        state.to_string()
    }

    /// Opens (or raises) the settings window.
    fn open_settings(&mut self, cx: &mut App) {
        if let Some(handle) = &self.settings_window
            && handle
                .update(cx, |_, window, _| window.activate_window())
                .is_ok()
        {
            return;
        }
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                None,
                size(px(settings_window::WIDTH), px(settings_window::HEIGHT)),
                cx,
            ))),
            titlebar: Some(gpui::TitlebarOptions {
                title: Some("Orbit Settings".into()),
                ..Default::default()
            }),
            app_id: Some("orbit-settings".into()),
            ..Default::default()
        };
        match cx.open_window(options, |window, cx| {
            cx.new(|cx| SettingsView::new(window, cx))
        }) {
            Ok(handle) => self.settings_window = Some(handle),
            Err(error) => log::warn!("opening the settings window: {error:#}"),
        }
    }

    /// Lets go of what the daemon put in the compositor.
    fn shutdown(&mut self, cx: &mut App) {
        if let Some(instance) = &self.instance {
            self.bind.release(instance);
        }
        self.corners.close_all(cx);
        if let Some(handle) = self.settings_window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
    }

    /// Everything the compositor must see go before this process is
    /// replaced by a new binary: the bind, the corner surfaces, the windows.
    pub(crate) fn prepare_exec(&mut self, cx: &mut App) {
        self.shutdown(cx);
        if let Some((handle, _)) = self.window.take() {
            let _ = handle.update(cx, |_, window, _| window.remove_window());
        }
        self.surface = None;
    }

    /// Nothing open and nothing happening for `quiet`: the moment an
    /// automatic update may restart the daemon.
    pub(crate) fn is_idle(&self, cx: &App, quiet: Duration) -> bool {
        if self.window.is_some() || self.settings_window.is_some() {
            return false;
        }
        // The quiet window is far longer than any move takes to confirm,
        // and every open (shortcut or corner) stamps `last_engage`.
        let _ = cx;
        self.last_engage.is_none_or(|at| at.elapsed() >= quiet)
    }

    /// A shortcut press while the overview is open counts as a double tap
    /// when it follows the opening press closely: switch to the last
    /// window instead (settings).
    fn double_tap(&mut self, request: &Request, cx: &mut App) -> bool {
        if !matches!(request, Request::Toggle { .. } | Request::Show { .. }) {
            return false;
        }
        let enabled = cx
            .global::<Prefs>()
            .settings
            .behaviour
            .double_tap_last_window;
        let now = Instant::now();
        let quick = self
            .last_engage
            .is_some_and(|at| now.duration_since(at) < DOUBLE_TAP);
        self.last_engage = Some(now);
        if !enabled || !quick || self.window.is_none() {
            return false;
        }
        let last = self.desk.read(cx).last_active();
        if let Some(window) = last {
            self.desk.update(cx, |desk, cx| {
                desk.perform(Intent::Activate(window), cx);
            });
            true
        } else {
            false
        }
    }

    fn drive(&mut self, command: &str, cx: &mut App) -> String {
        if !self.test_hooks {
            return "error: test hooks are off (start the daemon with --test-hooks)".into();
        }
        if command.trim() == "hot-corner" {
            return match self.corners.simulate_enter(cx) {
                Ok(what) => what,
                Err(error) => format!("error: {error}"),
            };
        }
        if command.trim() == "settings" {
            self.open_settings(cx);
            return "ok".into();
        }
        self.prune(cx);
        let Some((handle, _)) = &self.window else {
            return "error: the overview is not open".into();
        };
        // Not `handle.update`: that holds the view, and the input it
        // dispatches updates the view.
        match cx.update_window((*handle).into(), |root, window, cx| {
            let view = root
                .downcast::<OverviewView>()
                .map_err(|_| "unexpected root view".to_string())?;
            OverviewView::drive(&view, command, window, cx)
        }) {
            Ok(Ok(reply)) => reply,
            Ok(Err(error)) => format!("error: {error}"),
            Err(_) => "error: the overview closed".into(),
        }
    }

    fn status(&self) -> String {
        let state = match &self.window {
            Some((_, output)) => {
                format!("visible output={}", output.as_deref().unwrap_or("default"))
            }
            None => "hidden".into(),
        };
        let last = self
            .last_open
            .get()
            .map(|d| format!(" last_open_ms={:.1}", d.as_secs_f64() * 1000.0))
            .unwrap_or_default();
        format!("{state} opens={}{last}", self.opens)
    }
}

async fn serve(
    mut receiver: async_mpsc::UnboundedReceiver<Envelope>,
    overlay: Rc<RefCell<Overlay>>,
    cx: &mut AsyncApp,
) {
    while let Some(first) = receiver.next().await {
        let mut batch = vec![first];
        while let Ok(next) = receiver.try_recv() {
            batch.push(next);
        }
        let quit = cx.update(|cx| {
            let mut overlay = overlay.borrow_mut();
            overlay.prune(cx);
            let mut desired = overlay.desired();
            for envelope in &batch {
                if overlay.double_tap(&envelope.request, cx) {
                    // The activation closes the overview by itself.
                    continue;
                }
                desired.apply(&envelope.request, envelope.size);
            }
            let result = overlay.reconcile(&desired, cx);
            if batch
                .iter()
                .any(|envelope| matches!(envelope.request, Request::Settings))
            {
                overlay.open_settings(cx);
            }
            let reply = match &result {
                Ok(()) => overlay.status(),
                Err(error) => format!("error: {error}"),
            };
            for envelope in &batch {
                let reply = match &envelope.request {
                    Request::State => overlay.state(cx),
                    Request::Drive(command) => overlay.drive(command, cx),
                    Request::Update(words) => crate::updates::command(words, cx),
                    _ => reply.clone(),
                };
                let _ = envelope.reply.send(reply);
            }
            desired.quit
        });
        if quit {
            cx.update(|cx| {
                overlay.borrow_mut().shutdown(cx);
                cx.quit();
            });
            break;
        }
    }
}

/// Applies the current settings everywhere they matter: the look, the
/// capture worker, the desk's filters, the live bind and snippet, the hot
/// corner surfaces and the login unit. Records the outcome in [`Prefs`].
fn apply_settings(overlay: &mut Overlay, cx: &mut App) {
    let settings = cx.global::<Prefs>().settings.clone();
    let o = overlay.options;
    // Look.
    let base = match (o.high_contrast, o.opaque, settings.appearance.theme) {
        (true, _, _) | (_, _, ThemeChoice::HighContrast) => theme::HIGH_CONTRAST,
        (_, true, _) | (_, _, ThemeChoice::Solid) => theme::STANDARD.opaque(),
        _ if o.glass == Some(false) => theme::STANDARD,
        _ => theme::GLASS,
    };
    let theme = base.tinted(settings.appearance.tint_percent);
    let wanted_icons = Some(settings.icons.theme.clone()).filter(|t| !t.is_empty());
    let current = Look::get(cx);
    let icons = if overlay.icon_override != wanted_icons || current.icons.theme_name().is_empty() {
        overlay.icon_override = wanted_icons.clone();
        let icons = Arc::new(Icons::new(
            wanted_icons.or_else(icon_theme),
            Icons::xdg_data_dirs(),
        ));
        log::info!("app icons from theme {}", icons.theme_name());
        icons
    } else {
        current.icons.clone()
    };
    let corner = settings.hot_corner.corner;
    let mut look = Look::with_icons(
        theme,
        overlay.font.clone(),
        o.reduced_motion || settings.appearance.reduced_motion,
        icons,
    );
    look.prefs = OverviewPrefs {
        columns: settings.layout.columns.fixed(),
        titles: match settings.layout.titles {
            Titles::Never => 0,
            Titles::Selected => 1,
            Titles::Always => 2,
        },
        special_workspaces: settings.layout.special_workspaces,
        rail_at_bottom: settings.layout.rail == orbit_core::settings::RailPosition::Bottom,
        follow_on_move: settings.behaviour.follow_on_move,
        search_this_workspace: settings.behaviour.search_scope
            == orbit_core::settings::SearchScope::Workspace,
        corner_closes: (settings.hot_corner.corner_closes
            && corner != orbit_core::settings::Corner::Off)
            .then(|| {
                use orbit_core::settings::Corner::*;
                let right = matches!(corner, TopRight | BottomRight);
                let bottom = matches!(corner, BottomLeft | BottomRight);
                (right, bottom, settings.hot_corner.dwell_ms)
            }),
    };
    let _ = Columns::ALL;
    cx.set_global(look);
    // Previews and the desk.
    overlay.previews.tune(
        Duration::from_millis(u64::from(settings.previews.refresh_ms)),
        settings.appearance.blur,
    );
    overlay.desk.update(cx, |desk, _| {
        desk.set_never_capture(settings.previews.never_capture.clone());
        desk.set_capture_inactive(settings.previews.capture_inactive_workspaces);
        desk.set_close_on_activate(settings.behaviour.close_on_activate);
        desk.set_overview_prefs(
            !settings.layout.special_workspaces,
            settings.behaviour.search_scope == orbit_core::settings::SearchScope::Workspace,
        );
    });
    // The shortcut: live bind, conflicts, the snippet.
    // A conflicting combination is not bound: both binds would fire.
    let shortcut: Shortcut = settings.shortcut.clone();
    let (bind, conflicts) = match &overlay.instance {
        Some(instance) => {
            let conflicts = engage::conflicts(instance, &shortcut);
            let bind = if conflicts.is_empty() {
                overlay.bind.apply(instance, &shortcut, &overlay.exe)
            } else {
                overlay.bind.release(instance);
                Err(format!(
                    "{} is already used by {}",
                    shortcut.label(),
                    conflicts
                        .iter()
                        .map(|c| c.description.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
            };
            (bind, conflicts)
        }
        None => (Err("no Hyprland session".into()), Vec::new()),
    };
    let snippet_written = match engage::snippet_path() {
        Some(path) => engage::write_snippet(&path, &shortcut, &overlay.exe),
        None => Err("no config directory".into()),
    };
    let include_present = engage::hyprland_lua_path()
        .as_deref()
        .is_some_and(engage::include_present);
    // Hot corners.
    let corners = sync_corners(overlay, cx);
    // Launch at login.
    if overlay.launch_applied != Some(settings.startup.launch_at_login) {
        match set_launch_at_login(settings.startup.launch_at_login, &overlay.exe) {
            Ok(()) => overlay.launch_applied = Some(settings.startup.launch_at_login),
            Err(error) => log::warn!("launch at login: {error}"),
        }
    }
    let prefs = cx.global_mut::<Prefs>();
    prefs.bind = bind;
    prefs.conflicts = conflicts;
    prefs.snippet_written = snippet_written;
    prefs.include_present = include_present;
    prefs.corners = corners;
    prefs.revision += 1;
    cx.refresh_windows();
}

/// Saves `settings` and applies them (the settings window's commits and
/// Brings the hot-corner surfaces in line with the settings and the
/// monitors Hyprland currently reports; returns how many are in place.
/// Called from [`apply_settings`] and from the settings poll, because the
/// first apply happens before the link has reported any monitor (the
/// corner would otherwise never appear when it is configured before
/// launch), and monitors come and go.
fn sync_corners(overlay: &mut Overlay, cx: &mut App) -> usize {
    let settings = &cx.global::<Prefs>().settings;
    let key = crate::corner::SyncKey {
        corner: settings.hot_corner.corner,
        dwell_ms: settings.hot_corner.dwell_ms,
        scope: settings.hot_corner.monitors,
        monitors: overlay
            .desk
            .read(cx)
            .store()
            .model()
            .monitors
            .iter()
            .map(|m| (m.name.clone(), m.focused))
            .collect(),
    };
    let sender = overlay.sender.clone();
    overlay.corners.sync(
        key,
        sender,
        &|name, cx| display_for(name, cx).ok().map(|d| d.id()),
        cx,
    )
}

/// `orbit config set` through the file watcher both end here).
pub fn commit_settings(settings: Settings, cx: &mut App) -> Result<(), String> {
    cx.global_mut::<Prefs>().save(settings)?;
    let overlay = cx.global::<OverlayRef>().0.clone();
    let mut overlay = overlay
        .try_borrow_mut()
        .map_err(|_| "the daemon is busy; try again".to_string())?;
    apply_settings(&mut overlay, cx);
    Ok(())
}

/// Appends the `require` line to `hyprland.lua`: only the settings
/// window's confirmed button calls this.
pub fn append_include(cx: &mut App) -> Result<(), String> {
    let path = engage::hyprland_lua_path().ok_or("no config directory")?;
    engage::append_include(&path)?;
    let prefs = cx.global_mut::<Prefs>();
    prefs.include_present = engage::include_present(&path);
    prefs.revision += 1;
    cx.refresh_windows();
    Ok(())
}

/// The shortcut a fresh configuration gets.
pub fn default_shortcut() -> Shortcut {
    engage::pick_default(Instance::from_env().ok().as_ref())
}

/// Writes (or removes) the systemd user unit and enables (or disables) it.
fn set_launch_at_login(enable: bool, exe: &Path) -> Result<(), String> {
    let dir = env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .ok_or("no config directory")?
        .join("systemd/user");
    let unit = dir.join("orbit.service");
    let systemctl = |args: &[&str]| -> Result<(), String> {
        let status = std::process::Command::new("systemctl")
            .arg("--user")
            .args(args)
            .status()
            .map_err(|e| format!("systemctl: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!(
                "systemctl --user {} failed ({status})",
                args.join(" ")
            ))
        }
    };
    if enable {
        fs::create_dir_all(&dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        let content = format!(
            "[Unit]\nDescription=Orbit workspace overview\nPartOf=graphical-session.target\nAfter=graphical-session.target\n\n             [Service]\nExecStart={} daemon\nRestart=on-failure\n\n[Install]\nWantedBy=graphical-session.target\n",
            exe.display()
        );
        fs::write(&unit, content).map_err(|e| format!("writing {}: {e}", unit.display()))?;
        systemctl(&["daemon-reload"])?;
        systemctl(&["enable", "orbit.service"])
    } else {
        if !unit.exists() {
            return Ok(());
        }
        let _ = systemctl(&["disable", "orbit.service"]);
        fs::remove_file(&unit).map_err(|e| format!("removing {}: {e}", unit.display()))?;
        systemctl(&["daemon-reload"])
    }
}

/// GPUI names Wayland outputs only through a v5 UUID of the output name.
fn display_for(name: &str, cx: &App) -> Result<Rc<dyn PlatformDisplay>, String> {
    let wanted = uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_DNS, name.as_bytes());
    cx.displays()
        .into_iter()
        .find(|display| display.uuid().ok() == Some(wanted))
        .ok_or_else(|| format!("unknown output `{name}`"))
}

/// Full-output, non-reserving, above fullscreen clients, keyboard only while
/// it exists.
fn overlay_options(display_id: Option<DisplayId>, size: Size<Pixels>) -> WindowOptions {
    WindowOptions {
        // GPUI sends the window size as the layer size, which overrides the
        // anchors: the 1536x1095 default leaves wide outputs uncovered, and 0x0
        // (which asks the compositor to stretch) breaks wgpu on real GPUs. So
        // request exactly the output's logical size.
        window_bounds: Some(WindowBounds::Windowed(Bounds::new(
            point(px(0.), px(0.)),
            size,
        ))),
        titlebar: None,
        app_id: Some("orbit".into()),
        display_id,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn toggle() -> Request {
        Request::Toggle {
            output: Some("DP-1".into()),
        }
    }

    #[test]
    fn parses_and_round_trips_requests() {
        for line in [
            "show",
            "show DP-1",
            "toggle",
            "toggle eDP-1",
            "hide",
            "status",
            "quit",
        ] {
            assert_eq!(Request::parse(line).unwrap().encode(), line);
        }
        assert_eq!(
            Request::parse("show DP-1\n").unwrap(),
            Request::Show {
                output: Some("DP-1".into())
            }
        );
        assert!(Request::parse("").is_err());
        assert!(Request::parse("hide DP-1").is_err());
        assert!(Request::parse("show a b").is_err());
        assert!(Request::parse("explode").is_err());
    }

    #[test]
    fn requests_resolve_to_hyprlands_logical_size() {
        // djinn: 2880x1800 at 1.25 is 2304x1440 logical; GPUI would say otherwise.
        let monitors = serde_json::json!([
            {"name": "eDP-1", "width": 2880, "height": 1800, "scale": 1.25, "focused": false},
            {"name": "DP-1", "width": 3840, "height": 2160, "scale": 2.0, "focused": true},
        ]);
        let corner = Request::Show {
            output: Some("eDP-1".into()),
        };
        let (request, size) = resolve_output_in(corner.clone(), Some(&monitors));
        assert_eq!(request, corner);
        assert_eq!(size, Some((2304., 1440.)));
        // No output named: the focused monitor.
        let (request, size) = resolve_output_in(Request::Toggle { output: None }, Some(&monitors));
        assert_eq!(
            request,
            Request::Toggle {
                output: Some("DP-1".into())
            }
        );
        assert_eq!(size, Some((1920., 1080.)));
        // Unknown monitor, or no Hyprland: the request passes through unsized.
        let (request, size) = resolve_output_in(
            Request::Show {
                output: Some("HDMI-A-1".into()),
            },
            Some(&monitors),
        );
        assert_eq!(request.encode(), "show HDMI-A-1");
        assert_eq!(size, None);
        assert_eq!(
            resolve_output_in(Request::Hide, Some(&monitors)),
            (Request::Hide, None)
        );
        assert_eq!(resolve_output_in(corner.clone(), None).1, None);
    }

    #[test]
    fn toggle_bursts_settle_by_parity() {
        for count in 0..40 {
            let mut desired = Desired::default();
            (0..count).for_each(|_| desired.apply(&toggle(), None));
            assert_eq!(desired.visible, count % 2 == 1, "{count} toggles");
        }
    }

    #[test]
    fn last_request_wins_and_quit_hides() {
        let mut desired = Desired::default();
        desired.apply(
            &Request::Show {
                output: Some("DP-1".into()),
            },
            None,
        );
        desired.apply(
            &Request::Show {
                output: Some("HDMI-A-1".into()),
            },
            Some((1920., 1080.)),
        );
        assert_eq!(desired.output.as_deref(), Some("HDMI-A-1"));
        assert_eq!(desired.size, Some((1920., 1080.)));
        desired.apply(&Request::Hide, None);
        desired.apply(&Request::Status, None);
        assert!(!desired.visible);
        desired.apply(&toggle(), None);
        desired.apply(&Request::Quit, None);
        assert!(!desired.visible && desired.quit);
    }

    #[test]
    fn toggle_while_hidden_takes_the_new_output() {
        let mut desired = Desired {
            visible: false,
            output: Some("old".into()),
            size: None,
            quit: false,
        };
        desired.apply(&toggle(), None);
        assert_eq!(desired.output.as_deref(), Some("DP-1"));
    }
}
