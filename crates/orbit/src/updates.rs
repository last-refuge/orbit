//! Staying up to date, daemon side (ORB-011): the scheduled check, what
//! the overview and the settings window show, the socket commands, the
//! in-place update and the restart after it, and the crash-loop guard at
//! start. The work itself lives in `orbit-update`.

use std::{
    process::{Command, Stdio},
    time::Duration,
};

use gpui::{App, AsyncApp, Global};
use orbit_core::settings::{Settings, UpdateChannel, UpdateMode};
use orbit_ui::theme::UpdateBadge;
use orbit_update::{Checker, Http, InstallInfo, Latest, Outcome, UpdateState};

use crate::{
    daemon::{OverlayRef, Request},
    prefs::Prefs,
};

/// How often the scheduler looks at the clock.
const TICK: Duration = Duration::from_secs(60);
/// The first check waits this long after the daemon starts (plus jitter),
/// so a login is never slowed by it. `ORBIT_UPDATE_START_DELAY_MS`
/// overrides it for the checks.
const START_DELAY: Duration = Duration::from_secs(120);
const START_JITTER_MAX: Duration = Duration::from_secs(600);
/// A daemon that dies this soon after an update, this many times, is rolled back.
const CRASH_WINDOW_SECS: u64 = 120;
const CRASH_LIMIT: u32 = 3;
/// How long the restart watcher gives the new daemon to answer.
const HEALTH_WAIT: Duration = Duration::from_secs(10);
/// Automatic updates wait for this much quiet (no overview, no interaction).
const QUIET: Duration = Duration::from_secs(120);

/// The running build's version. `ORBIT_VERSION_OVERRIDE` replaces it, for
/// the scripted update checks only (they need two builds of the same tree
/// to report different versions); the installer and users never set it.
pub fn current() -> &'static str {
    static CURRENT: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    CURRENT.get_or_init(|| {
        std::env::var("ORBIT_VERSION_OVERRIDE")
            .ok()
            .filter(|v| !v.is_empty() && semver::Version::parse(v).is_ok())
            .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_owned())
    })
}

/// The binary the install marker describes: `ORBIT_INSTALLED_BINARY` when
/// set (the restart watcher and the scripted checks), else this executable.
pub fn installed_exe() -> std::path::PathBuf {
    std::env::var_os("ORBIT_INSTALLED_BINARY")
        .filter(|v| !v.is_empty())
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .unwrap_or_default()
}

/// What the daemon knows about updates, as a GPUI global for the views
/// (`orbit state` reads it too).
#[derive(Clone, Debug)]
pub struct UpdateStatus {
    pub current: String,
    pub state: UpdateState,
    pub checking: bool,
    pub applying: bool,
    /// The last thing worth telling: text, and whether it is an error.
    pub last_result: Option<(String, bool)>,
    /// Whether this binary may replace itself, or why not.
    pub updatable: Result<(), String>,
    /// "Updated to X", shown until the person has seen it once.
    pub applied_notice: Option<String>,
    pub revision: u64,
}

impl Global for UpdateStatus {}

impl UpdateStatus {
    pub fn new(after_update: Option<&str>) -> Self {
        let mut state = UpdateState::load();
        let now = orbit_update::now();
        let mut applied_notice = None;
        if let Some(from) = after_update {
            state.applied = Some(orbit_update::state::Applied {
                version: current().into(),
                from: from.into(),
                at: now,
            });
            state.starts_since_update = 0;
            state.failed = None;
            applied_notice = Some(format!("Updated to {}", current()));
            let _ = state.save();
        }
        let updatable =
            InstallInfo::updatable(InstallInfo::load().as_ref(), &installed_exe()).map(|_| ());
        Self {
            current: current().into(),
            state,
            checking: false,
            applying: false,
            last_result: None,
            updatable,
            applied_notice,
            revision: 0,
        }
    }

    pub fn current_version() -> semver::Version {
        semver::Version::parse(current()).unwrap_or_else(|_| semver::Version::new(0, 0, 0))
    }

    /// The newer release on offer, if any.
    pub fn available(&self) -> Option<&Latest> {
        self.state.available(&Self::current_version())
    }

    /// The pill's text, if there is anything to say.
    pub fn badge(&self, settings: &Settings) -> Option<UpdateBadge> {
        if let Some(version) = &self.state.failed {
            return Some(UpdateBadge {
                text: format!("Update to {version} failed, rolled back").into(),
                detail: "Open the settings for details".into(),
                failed: true,
            });
        }
        if let Some(notice) = &self.applied_notice {
            return Some(UpdateBadge {
                text: notice.clone().into(),
                detail: "Release notes are in the settings".into(),
                failed: false,
            });
        }
        if settings.update.mode == UpdateMode::Off {
            return None;
        }
        let latest = self.available()?;
        Some(UpdateBadge {
            text: format!("{} available", latest.version).into(),
            detail: if self.updatable.is_ok() {
                "Update from the settings (Ctrl+,)".into()
            } else {
                "Run the install line to update".into()
            },
            failed: false,
        })
    }

    pub fn json(&self) -> serde_json::Value {
        let latest = self.state.latest.as_ref();
        serde_json::json!({
            "current": self.current,
            "latest": latest.map(|l| l.version.clone()),
            "latest_tag": latest.map(|l| l.tag.clone()),
            "notes_url": latest.map(|l| l.html_url.clone()),
            "available": self.available().is_some(),
            "updatable": self.updatable.is_ok(),
            "why_not": self.updatable.as_ref().err(),
            "checking": self.checking,
            "applying": self.applying,
            "last_check": self.state.last_check,
            "skipped": self.state.skipped,
            "failed": self.state.failed,
            "applied": self.state.applied.as_ref().map(|a| serde_json::json!({"version": a.version, "from": a.from, "at": a.at})),
            "last_result": self.last_result.as_ref().map(|(t, _)| t.clone()),
            "last_error": self.last_result.as_ref().filter(|(_, e)| *e).map(|(t, _)| t.clone()),
            "revision": self.revision,
        })
    }
}

fn checker(settings: &Settings) -> Checker {
    Checker {
        api_base: orbit_update::api_base(),
        arch: orbit_update::arch().to_owned(),
        channel: match settings.update.channel {
            UpdateChannel::Prerelease => orbit_update::Channel::Prerelease,
            UpdateChannel::Stable => orbit_update::Channel::Stable,
        },
        current: UpdateStatus::current_version(),
        http: Http::new(current()),
    }
}

/// Publishes the status to the views: the badge global and a bumped
/// revision (both views observe it).
fn publish(cx: &mut App) {
    let settings = cx.global::<Prefs>().settings.clone();
    let badge = cx.global::<UpdateStatus>().badge(&settings);
    cx.set_global(badge.unwrap_or_default());
    cx.global_mut::<UpdateStatus>().revision += 1;
}

/// The crash-loop guard, before anything else at daemon start: a daemon
/// that keeps dying right after an update goes back to the previous
/// binary. Returns only when it is safe to go on.
pub fn guard_start(after_update: Option<&str>) {
    let mut state = UpdateState::load();
    let now = orbit_update::now();
    if after_update.is_some() {
        return;
    }
    let Some(applied) = &state.applied else {
        return;
    };
    if now.saturating_sub(applied.at) > CRASH_WINDOW_SECS {
        return;
    }
    state.starts_since_update += 1;
    let _ = state.save();
    if state.starts_since_update < CRASH_LIMIT {
        return;
    }
    log::error!(
        "{} starts within {CRASH_WINDOW_SECS} s of updating to {}: rolling back",
        state.starts_since_update,
        applied.version
    );
    let exe = installed_exe();
    match InstallInfo::updatable(InstallInfo::load().as_ref(), &exe)
        .and_then(|install| orbit_update::rollback(&install))
    {
        Ok(version) => {
            state.failed = Some(applied.version.clone());
            state.applied = None;
            state.starts_since_update = 0;
            let _ = state.save();
            notify(
                &format!(
                    "Orbit {} crashed at start",
                    state.failed.clone().unwrap_or_default()
                ),
                &format!("Back on {version}. The failed version will not be installed again."),
            );
            exec_self(
                &exe,
                without_after_update(&std::env::args().skip(1).collect::<Vec<_>>()),
            );
        }
        Err(error) => log::error!("rollback failed: {error}"),
    }
}

/// Starts the scheduler: periodic checks, the quiet-moment automatic
/// install, and the crash-loop counter reset once the daemon has lived.
pub fn start(overlay: OverlayRef, cx: &mut App) {
    publish(cx);
    let start_delay = std::env::var("ORBIT_UPDATE_START_DELAY_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or_else(|| {
            let jitter = (std::process::id() as u64 * 7919) % START_JITTER_MAX.as_secs();
            START_DELAY + Duration::from_secs(jitter)
        });
    let tick = std::env::var("ORBIT_UPDATE_TICK_MS")
        .ok()
        .and_then(|v| v.parse().ok())
        .map(Duration::from_millis)
        .unwrap_or(TICK);
    cx.spawn(async move |cx| {
        cx.background_executor().timer(start_delay).await;
        let mut alive = start_delay;
        loop {
            cx.update(|cx| {
                if alive >= Duration::from_secs(CRASH_WINDOW_SECS) {
                    let status = cx.global_mut::<UpdateStatus>();
                    if status.state.starts_since_update != 0 {
                        status.state.starts_since_update = 0;
                        let _ = status.state.save();
                    }
                }
                let settings = cx.global::<Prefs>().settings.clone();
                if settings.update.mode == UpdateMode::Off {
                    return;
                }
                let status = cx.global::<UpdateStatus>();
                let due = status.state.last_check.is_none_or(|at| {
                    orbit_update::now().saturating_sub(at)
                        >= u64::from(settings.update.check_interval_hours) * 3600
                });
                if due && !status.checking {
                    run_check(false, cx);
                } else if settings.update.mode == UpdateMode::Auto
                    && status.available().is_some()
                    && status.updatable.is_ok()
                    && !status.applying
                    && quiet(&overlay, cx)
                {
                    run_apply(None, cx);
                }
            });
            cx.background_executor().timer(tick).await;
            alive += tick;
        }
    })
    .detach();
}

/// No overview, no settings window, nothing moving, and no interaction
/// for a while: the moment an automatic update may restart the daemon.
fn quiet(overlay: &OverlayRef, cx: &App) -> bool {
    let Ok(overlay) = overlay.0.try_borrow() else {
        return false;
    };
    overlay.is_idle(cx, QUIET)
}

/// Runs a check off the UI thread and publishes what it found.
pub fn run_check(force: bool, cx: &mut App) {
    {
        let status = cx.global_mut::<UpdateStatus>();
        if status.checking {
            return;
        }
        status.checking = true;
        status.last_result = None;
    }
    publish(cx);
    let settings = cx.global::<Prefs>().settings.clone();
    let checker = checker(&settings);
    let mut state = cx.global::<UpdateStatus>().state.clone();
    let task = cx.background_executor().spawn(async move {
        let outcome = checker.check(&mut state, force);
        let _ = state.save();
        (state, outcome)
    });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let (state, outcome) = task.await;
        cx.update(|cx| {
            let notify_version = {
                let status = cx.global_mut::<UpdateStatus>();
                status.checking = false;
                let previous = status.state.latest.as_ref().map(|l| l.version.clone());
                status.state = state;
                match &outcome {
                    Ok(Outcome::Available(latest)) => {
                        status.last_result = Some((format!("{} is available", latest.version), false));
                        (previous.as_deref() != Some(latest.version.as_str())).then(|| latest.clone())
                    }
                    Ok(Outcome::UpToDate) => {
                        status.last_result = Some(("You are on the latest version".into(), false));
                        None
                    }
                    Ok(Outcome::Deferred { .. }) => None,
                    Err(error) => {
                        status.last_result = Some((format!("Check failed: {error}"), true));
                        None
                    }
                }
            };
            publish(cx);
            let settings = cx.global::<Prefs>().settings.clone();
            if let Some(latest) = notify_version
                && settings.update.notify
            {
                let body = format!(
                    "Orbit {} is available (you have {}). Open the settings (Ctrl+, in the overview) to update.",
                    latest.version,
                    current()
                );
                cx.background_executor()
                    .spawn(async move { notify("Orbit update available", &body) })
                    .detach();
            }
        });
    })
    .detach();
}

/// Downloads, verifies and installs the newest release (or `version`), then
/// restarts the daemon on the new binary.
pub fn run_apply(version: Option<String>, cx: &mut App) {
    let (latest, install) = {
        let status = cx.global_mut::<UpdateStatus>();
        if status.applying {
            return;
        }
        let latest = match (&version, status.state.latest.clone()) {
            (Some(v), Some(latest)) if latest.version == *v => latest,
            (Some(v), _) => {
                status.last_result = Some((
                    format!("{v} is not the release on offer; check first"),
                    true,
                ));
                publish(cx);
                return;
            }
            (None, Some(latest)) => latest,
            (None, None) => {
                status.last_result =
                    Some(("Nothing to install; check for updates first".into(), true));
                publish(cx);
                return;
            }
        };
        let exe = installed_exe();
        let install = match InstallInfo::updatable(InstallInfo::load().as_ref(), &exe) {
            Ok(install) => install,
            Err(why) => {
                status.last_result = Some((why, true));
                publish(cx);
                return;
            }
        };
        status.applying = true;
        status.last_result = Some((format!("Installing {}…", latest.version), false));
        (latest, install)
    };
    publish(cx);
    let http = Http::new(current());
    let task = cx
        .background_executor()
        .spawn(async move { orbit_update::apply(&install, &latest, &http).map(|a| (a, install)) });
    cx.spawn(async move |cx: &mut AsyncApp| {
        let result = task.await;
        cx.update(|cx| match result {
            Ok((applied, install)) => {
                {
                    let status = cx.global_mut::<UpdateStatus>();
                    status.applying = false;
                    status.last_result =
                        Some((format!("Installed {}; restarting", applied.version), false));
                    let _ = status.state.save();
                }
                publish(cx);
                restart(&install.binary, &applied.version, cx);
            }
            Err(error) => {
                let status = cx.global_mut::<UpdateStatus>();
                status.applying = false;
                status.last_result = Some((format!("Update failed: {error}"), true));
                publish(cx);
            }
        });
    })
    .detach();
}

/// Restarts on the new binary: under systemd through the unit, otherwise
/// by replacing this process. A detached watcher rolls back if the new
/// daemon does not answer within [`HEALTH_WAIT`].
fn restart(binary: &std::path::Path, new_version: &str, cx: &mut App) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // The watcher must run even if the new binary cannot: it starts from
    // the previous one when there is one. Its log lands beside the state
    // file, where a failed update can be read about afterwards.
    let previous = binary.with_file_name("orbit.prev");
    let watcher = if previous.is_file() {
        previous
    } else {
        binary.to_path_buf()
    };
    let log_path = UpdateState::path().map(|p| p.with_file_name("update-watch.log"));
    let open_log = || -> Option<std::fs::File> {
        let path = log_path.clone()?;
        std::fs::create_dir_all(path.parent()?).ok()?;
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .ok()
    };
    let out = open_log().map(Stdio::from).unwrap_or_else(Stdio::null);
    let err = open_log().map(Stdio::from).unwrap_or_else(Stdio::null);
    match Command::new("setsid")
        .arg(&watcher)
        .args(["update", "--watch", current(), new_version])
        .env("ORBIT_INSTALLED_BINARY", binary)
        .env("ORBIT_DAEMON_ARGS", args.join("\u{1f}"))
        .env("RUST_LOG", "warn")
        .stdin(Stdio::null())
        .stdout(out)
        .stderr(err)
        .spawn()
    {
        Ok(child) => log::info!(
            "restart watcher {} is pid {}",
            watcher.display(),
            child.id()
        ),
        Err(error) => log::error!("restart watcher {}: {error}", watcher.display()),
    }
    if under_unit() {
        let _ = Command::new("systemctl")
            .args(["--user", "restart", "orbit.service"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        return;
    }
    let overlay = cx.global::<OverlayRef>().0.clone();
    if let Ok(mut overlay) = overlay.try_borrow_mut() {
        overlay.prepare_exec(cx);
    }
    let mut args = without_after_update(&args);
    args.push("--after-update".into());
    args.push(current().into());
    exec_self(binary, args);
}

/// The daemon's own arguments without a previous `--after-update VERSION`
/// pair, for a relaunch.
fn without_after_update(args: &[String]) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut skip = false;
    for arg in args {
        if skip {
            skip = false;
            continue;
        }
        if arg == "--after-update" {
            skip = true;
            continue;
        }
        out.push(arg.clone());
    }
    out
}

/// Whether this process is the one `orbit.service` started: its
/// `INVOCATION_ID` is the unit's. (Any shell started by systemd has an
/// `INVOCATION_ID` too, so the variable alone proves nothing.)
pub fn under_unit() -> bool {
    let Some(mine) = std::env::var_os("INVOCATION_ID") else {
        return false;
    };
    Command::new("systemctl")
        .args([
            "--user",
            "show",
            "-p",
            "InvocationID",
            "--value",
            "orbit.service",
        ])
        .output()
        .ok()
        .is_some_and(|out| {
            let unit = String::from_utf8_lossy(&out.stdout).trim().to_owned();
            !unit.is_empty() && unit == mine.to_string_lossy()
        })
}

/// Replaces this process with `binary daemon ARGS…`. Returns only if exec failed.
fn exec_self(binary: &std::path::Path, args: Vec<String>) {
    use std::os::unix::process::CommandExt;
    let error = Command::new(binary)
        .env_remove("ORBIT_VERSION_OVERRIDE")
        .env_remove("ORBIT_INSTALLED_BINARY")
        .args(&args)
        .exec();
    log::error!("exec {} {:?}: {error}", binary.display(), args);
}

/// A desktop notification (blocking: call off the UI thread).
pub fn notify(summary: &str, body: &str) {
    if let Err(error) = notify_rust::Notification::new()
        .appname("Orbit")
        .summary(summary)
        .body(body)
        .icon("orbit")
        .show()
    {
        log::warn!("notification: {error}");
    }
}

/// `update …` over the socket. Returns the reply line.
pub fn command(words: &str, cx: &mut App) -> String {
    let mut parts = words.split_whitespace();
    match (parts.next().unwrap_or_default(), parts.next()) {
        ("check", _) => {
            run_check(true, cx);
            "checking".into()
        }
        ("apply", version) => {
            run_apply(version.map(str::to_owned), cx);
            match &cx.global::<UpdateStatus>().last_result {
                Some((text, _)) => text.clone(),
                None => "applying".into(),
            }
        }
        ("skip", _) => {
            let status = cx.global_mut::<UpdateStatus>();
            let Some(latest) = status.state.latest.clone() else {
                return "error: nothing to skip".into();
            };
            status.state.skipped = Some(latest.version.clone());
            let _ = status.state.save();
            status.last_result = Some((format!("Skipping {}", latest.version), false));
            publish(cx);
            format!("skipped {}", latest.version)
        }
        ("rollback", _) => {
            let exe = installed_exe();
            match InstallInfo::updatable(InstallInfo::load().as_ref(), &exe)
                .and_then(|install| orbit_update::rollback(&install).map(|v| (install, v)))
            {
                Ok((install, version)) => {
                    {
                        let status = cx.global_mut::<UpdateStatus>();
                        status.state.applied = None;
                        let _ = status.state.save();
                    }
                    restart(&install.binary, &version, cx);
                    format!("rolled back to {version}; restarting")
                }
                Err(error) => format!("error: {error}"),
            }
        }
        ("reload", _) => {
            let status = cx.global_mut::<UpdateStatus>();
            status.state = UpdateState::load();
            publish(cx);
            "reloaded".into()
        }
        ("restart", _) => {
            // The CLI replaced the binary; run the new one. Not this
            // process's own path: after the swap it resolves to orbit.prev.
            let exe = InstallInfo::load()
                .map(|info| {
                    std::path::PathBuf::from(info.prefix)
                        .join("bin")
                        .join("orbit")
                })
                .filter(|p| p.is_file())
                .unwrap_or_else(installed_exe);
            let version =
                orbit_update::apply::version_of(&exe).unwrap_or_else(|_| current().into());
            restart(&exe, &version, cx);
            "restarting".into()
        }
        ("seen", _) => {
            cx.global_mut::<UpdateStatus>().applied_notice = None;
            publish(cx);
            "ok".into()
        }
        ("notes", _) => cx
            .global::<UpdateStatus>()
            .state
            .latest
            .as_ref()
            .map(|l| l.notes.replace('\n', " "))
            .unwrap_or_else(|| "no release known yet".into()),
        ("", _) | (_, _) => "error: update check|apply [VERSION]|skip|rollback|notes".into(),
    }
}

/// The "Updated to X" notice was on screen: drop it.
pub fn notice_seen(cx: &mut App) {
    if cx.has_global::<UpdateStatus>()
        && cx
            .global_mut::<UpdateStatus>()
            .applied_notice
            .take()
            .is_some()
    {
        publish(cx);
    }
}

/// Opens the latest release's page in the default browser.
pub fn open_notes(cx: &App) -> Result<(), String> {
    let url = cx
        .global::<UpdateStatus>()
        .state
        .latest
        .as_ref()
        .map(|l| l.html_url.clone())
        .filter(|u| !u.is_empty())
        .ok_or("no release known yet")?;
    Http::allowed(&url)?;
    Command::new("xdg-open")
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("xdg-open: {e}"))
}

/// Sends `update WORDS` to the running daemon, if one answers.
pub fn tell_daemon(words: &str) -> Option<String> {
    crate::daemon::send(&Request::Update(words.to_owned())).ok()
}

// ---- the command line ------------------------------------------------------

/// `orbit update …` without a daemon in the way: check and apply run in
/// this process, then the daemon (if any) is told to re-read or restart.
pub fn cli(args: &[String]) -> Result<String, String> {
    let settings = orbit_core::Settings::path()
        .filter(|p| p.exists())
        .and_then(|p| orbit_core::Settings::load(&p).ok())
        .unwrap_or_default();
    let mut words = args.iter().map(String::as_str);
    match words.next() {
        Some("check") => {
            let mut state = UpdateState::load();
            let outcome = checker(&settings).check(&mut state, true)?;
            state.save()?;
            tell_daemon("reload");
            if let Some(info) = InstallInfo::load()
                && !info.installed_version.is_empty()
                && info.installed_version != current()
            {
                eprintln!(
                    "note: the install marker says {} but this binary is {}; the marker is informational and is set right by the next update",
                    info.installed_version,
                    current()
                );
            }
            Ok(match outcome {
                Outcome::Available(latest) => format!(
                    "orbit {}; {} is available ({})\nInstall it: orbit update apply",
                    current(),
                    latest.version,
                    latest.html_url
                ),
                Outcome::UpToDate => format!(
                    "orbit {} is the latest on the {} channel",
                    current(),
                    settings.update.channel.name()
                ),
                Outcome::Deferred { until } => format!("not checked: backing off until {until}"),
            })
        }
        Some("notes") => {
            let state = UpdateState::load();
            let latest = state
                .latest
                .ok_or("no release known yet; run: orbit update check")?;
            Ok(format!(
                "{} ({})\n\n{}",
                latest.tag, latest.html_url, latest.notes
            ))
        }
        Some("apply") => {
            let wanted: Option<String> = match (words.next(), words.next()) {
                (Some("--version"), Some(v)) => Some(v.trim_start_matches('v').to_owned()),
                (None, _) => None,
                _ => return Err("usage: orbit update apply [--version vX.Y.Z]".into()),
            };
            let exe = installed_exe();
            let install = InstallInfo::updatable(InstallInfo::load().as_ref(), &exe)?;
            let mut state = UpdateState::load();
            if state.latest.is_none() {
                checker(&settings).check(&mut state, true)?;
                state.save()?;
            }
            let latest = state.latest.clone().ok_or("no release found")?;
            if let Some(wanted) = &wanted
                && latest.version != *wanted
            {
                return Err(format!(
                    "{wanted} is not the release on offer ({})",
                    latest.version
                ));
            }
            if latest.version == current() {
                return Ok(format!("orbit {} is already installed", current()));
            }
            let applied = orbit_update::apply(&install, &latest, &Http::new(current()))?;
            state.applied = Some(orbit_update::state::Applied {
                version: applied.version.clone(),
                from: current().into(),
                at: orbit_update::now(),
            });
            let offer = if state.auto_offered {
                String::new()
            } else {
                state.auto_offered = true;
                "\nTo have Orbit install updates by itself next time: orbit config set update.mode auto"
                    .into()
            };
            state.save()?;
            let daemon = match tell_daemon("restart") {
                Some(_) => "the running daemon is restarting on it",
                None => "start it with: orbit daemon",
            };
            Ok(format!(
                "installed orbit {}; {daemon}{offer}",
                applied.version
            ))
        }
        Some("rollback") => {
            let exe = installed_exe();
            let install = InstallInfo::updatable(InstallInfo::load().as_ref(), &exe)?;
            let version = orbit_update::rollback(&install)?;
            let mut state = UpdateState::load();
            state.applied = None;
            state.save()?;
            let daemon = match tell_daemon("restart") {
                Some(_) => "the running daemon is restarting on it",
                None => "start it with: orbit daemon",
            };
            Ok(format!("back on orbit {version}; {daemon}"))
        }
        Some("skip") => {
            let mut state = UpdateState::load();
            let latest = state.latest.clone().ok_or("no release known yet")?;
            state.skipped = Some(latest.version.clone());
            state.save()?;
            tell_daemon("reload");
            Ok(format!("skipping {}", latest.version))
        }
        Some("--watch") => {
            let (Some(old), Some(new)) = (words.next(), words.next()) else {
                return Err("usage: orbit update --watch OLD NEW".into());
            };
            watch(old, new);
            Ok(String::new())
        }
        _ => Err("usage: orbit update check|notes|apply [--version vX.Y.Z]|rollback|skip".into()),
    }
}

/// The restart watcher: waits for the new daemon to answer; if it does not,
/// rolls back and starts the previous one again.
fn watch(old: &str, new: &str) {
    let deadline = std::time::Instant::now() + HEALTH_WAIT;
    // Give the old daemon a moment to go away first.
    std::thread::sleep(Duration::from_millis(800));
    while std::time::Instant::now() < deadline {
        if crate::daemon::send(&Request::Status).is_ok() {
            return;
        }
        std::thread::sleep(Duration::from_millis(400));
    }
    log::error!(
        "the daemon did not answer within {HEALTH_WAIT:?} after updating to {new}; rolling back to {old}"
    );
    let exe = installed_exe();
    let mut state = UpdateState::load();
    match InstallInfo::updatable(InstallInfo::load().as_ref(), &exe)
        .and_then(|install| orbit_update::rollback(&install))
    {
        Ok(version) => {
            state.failed = Some(new.to_owned());
            state.applied = None;
            let _ = state.save();
            notify(
                &format!("Orbit {new} did not start"),
                &format!(
                    "Rolled back to {version}. The failed version will not be installed again."
                ),
            );
            let args: Vec<String> = std::env::var("ORBIT_DAEMON_ARGS")
                .map(|a| {
                    a.split('\u{1f}')
                        .filter(|s| !s.is_empty())
                        .map(str::to_owned)
                        .collect()
                })
                .unwrap_or_default();
            let args = without_after_update(&args);
            if under_unit() {
                let _ = Command::new("systemctl")
                    .args(["--user", "restart", "orbit.service"])
                    .status();
            } else {
                let _ = Command::new("setsid")
                    .arg(&exe)
                    .args(&args)
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
            }
        }
        Err(error) => log::error!("rollback failed: {error}"),
    }
}
