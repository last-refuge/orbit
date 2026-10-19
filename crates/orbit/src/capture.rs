//! `orbit capture`: captures Orbit's own fixture windows and reports whether
//! the pixels are right and what they cost (ORB-004). Other windows are
//! counted, never captured, and their app ids and titles are not recorded.

use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, Result, bail};
use orbit_capture::{Capture, Capturer, Failure, Frame, Stats, Toplevel, session::TIMEOUT};
use orbit_hyprland::{Instance, Snapshot, snapshot::Client};
use serde::Serialize;

use crate::fixture::{ACCENTS, APP_ID_PREFIX, accent_for};

pub struct Options {
    pub json: bool,
    pub record: Option<PathBuf>,
    pub label: String,
    pub repeat: usize,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            json: false,
            record: None,
            label: "run".into(),
            repeat: 20,
        }
    }
}

#[derive(Serialize)]
struct Report {
    label: String,
    hyprland_version: Option<String>,
    address_mapping: bool,
    /// All toplevels the compositor lists, fixtures included.
    toplevels_listed: usize,
    rss_kib_before: Option<u64>,
    rss_kib_after: Option<u64>,
    windows: Vec<WindowReport>,
}

#[derive(Serialize)]
pub struct Placement {
    pub workspace: String,
    /// The workspace is shown on some monitor (special workspaces only when open).
    pub workspace_visible: bool,
    pub floating: bool,
    pub fullscreen: bool,
    pub xwayland: bool,
    pub hidden: bool,
    pub grouped: bool,
    /// Other fixtures whose rectangles overlap this one on the same workspace.
    pub overlaps: Vec<String>,
}

#[derive(Serialize)]
struct WindowReport {
    name: String,
    address: Option<String>,
    placement: Option<Placement>,
    /// Session creation to complete constraints, per fresh session.
    setup_ms: Option<Percentiles>,
    shm_formats: Vec<String>,
    dmabuf_formats: usize,
    width: u32,
    height: u32,
    stride: u32,
    format: String,
    transform: String,
    buffer_bytes: usize,
    captures: usize,
    failures: Vec<String>,
    capture_ms: Option<Percentiles>,
    convert_ms: Option<Percentiles>,
    /// What a second capture of the unchanged window on one session does.
    repeat_without_damage: Option<String>,
    stats: Option<Stats>,
    own_accent_fraction: f64,
    other_accent_fraction: f64,
    verdict: Verdict,
}

#[derive(Serialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "kebab-case")]
pub enum Verdict {
    /// The fixture's own pixels came back.
    Ok,
    /// Frames arrived but are (nearly) black or empty.
    Blank,
    /// Frames arrived but look like a different window.
    WrongContent,
    Failed,
}

#[derive(Serialize, Clone, Debug, PartialEq)]
pub struct Percentiles {
    pub p50: f64,
    pub p95: f64,
    pub max: f64,
}

impl Percentiles {
    pub fn of(samples: &[f64]) -> Option<Self> {
        if samples.is_empty() {
            return None;
        }
        let mut sorted = samples.to_vec();
        sorted.sort_by(f64::total_cmp);
        let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
        Some(Self {
            p50: at(0.5),
            p95: at(0.95),
            max: sorted[sorted.len() - 1],
        })
    }
}

/// Judges a frame against the fixture's known look: a Slate card with a
/// 4px border in the fixture's accent.
/// Windows without a fixture card (XWayland, the preview) only need to be
/// non-blank.
pub fn judge(name: &str, frame: &Frame) -> (Verdict, f64, f64) {
    if matches!(name, X11_NAME | PREVIEW_NAME) {
        let verdict = if frame.stats().lit_fraction < 0.05 {
            Verdict::Blank
        } else {
            Verdict::Ok
        };
        return (verdict, 0.0, 0.0);
    }
    let own = accent_for(name);
    let own_fraction = frame.color_fraction(own.hex, 24);
    let other_fraction = ACCENTS
        .iter()
        .filter(|a| a.hex != own.hex)
        .map(|a| frame.color_fraction(a.hex, 24))
        .fold(0.0, f64::max);
    let verdict = if frame.stats().lit_fraction < 0.05 {
        Verdict::Blank
    } else if own_fraction < 0.005 || other_fraction > own_fraction {
        Verdict::WrongContent
    } else {
        Verdict::Ok
    };
    (verdict, own_fraction, other_fraction)
}

/// The XWayland fixture's class, from scripts/fixtures.sh.
pub const X11_FIXTURE_CLASS: &str = "OrbitFixtureX11";
/// `orbit preview`'s app id, so the check can capture the preview itself.
pub const PREVIEW_APP_ID: &str = "dev.orbit.preview";
const X11_NAME: &str = "x11";
pub const PREVIEW_NAME: &str = "preview";

/// Names Orbit's own windows; every other window is off limits.
pub fn fixture_name(toplevel: &Toplevel) -> Option<&str> {
    match toplevel.app_id.as_str() {
        X11_FIXTURE_CLASS => Some(X11_NAME),
        PREVIEW_APP_ID => Some(PREVIEW_NAME),
        app_id => app_id.strip_prefix(APP_ID_PREFIX),
    }
}

pub fn placement(snapshot: &Snapshot, client: &Client) -> Placement {
    let visible = snapshot.monitors.iter().any(|m| {
        m.active_workspace.id == client.workspace.id
            || m.special_workspace.id == client.workspace.id
    });
    let overlaps = snapshot
        .clients
        .iter()
        .filter(|other| {
            other.address != client.address
                && other.workspace.id == client.workspace.id
                && other.class.starts_with(APP_ID_PREFIX)
                && intersects(client, other)
        })
        .map(|other| other.class.trim_start_matches(APP_ID_PREFIX).to_owned())
        .collect();
    Placement {
        workspace: client.workspace.name.clone(),
        workspace_visible: visible,
        floating: client.floating,
        fullscreen: client.is_fullscreen(),
        xwayland: client.xwayland,
        hidden: client.hidden,
        grouped: !client.grouped.is_empty(),
        overlaps,
    }
}

fn intersects(a: &Client, b: &Client) -> bool {
    let (ax, ay, aw, ah) = (a.at[0], a.at[1], a.size[0], a.size[1]);
    let (bx, by, bw, bh) = (b.at[0], b.at[1], b.size[0], b.size[1]);
    ax < bx + bw && bx < ax + aw && ay < by + bh && by < ay + ah
}

pub fn rss_kib() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .trim()
        .trim_end_matches("kB")
        .trim()
        .parse()
        .ok()
}

pub fn run(options: Options) -> Result<()> {
    let rss_before = rss_kib();
    let snapshot = Instance::from_env()
        .and_then(|instance| Snapshot::take(&instance))
        .map_err(|error| log::warn!("no Hyprland snapshot: {error:#}"))
        .ok();
    let mut capturer = Capturer::connect()?;
    let toplevels = capturer.toplevels()?;
    let fixtures: Vec<_> = toplevels
        .iter()
        .filter(|t| fixture_name(t).is_some())
        .collect();
    if fixtures.is_empty() {
        bail!("no fixture windows are open; run scripts/fixtures.sh spawn first");
    }

    let dir = options.record.as_ref().map(|d| d.join(&options.label));
    if let Some(dir) = &dir {
        fs::create_dir_all(dir).with_context(|| format!("creating {}", dir.display()))?;
    }

    let mut windows = Vec::new();
    for toplevel in fixtures {
        let name = fixture_name(toplevel).unwrap_or_default().to_owned();
        let client = snapshot.as_ref().and_then(|s| {
            let address = toplevel.address_hex()?;
            s.clients
                .iter()
                .find(|c| c.address == address)
                .map(|c| (s, c))
        });
        let report = measure(
            &mut capturer,
            toplevel,
            &name,
            options.repeat,
            dir.as_deref(),
        );
        windows.push(WindowReport {
            address: toplevel.address_hex(),
            placement: client.map(|(s, c)| placement(s, c)),
            ..report
        });
    }
    windows.sort_by(|a, b| a.name.cmp(&b.name));

    let report = Report {
        label: options.label.clone(),
        hyprland_version: snapshot.as_ref().map(|s| s.version.tag.clone()),
        address_mapping: capturer.has_address_mapping(),
        toplevels_listed: toplevels.len(),
        rss_kib_before: rss_before,
        rss_kib_after: rss_kib(),
        windows,
    };
    if let Some(dir) = &dir {
        fs::write(
            dir.join("report.json"),
            serde_json::to_string_pretty(&report)? + "\n",
        )?;
    }
    if options.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_summary(&report);
    }
    if report.windows.iter().any(|w| w.verdict != Verdict::Ok) {
        std::process::exit(1);
    }
    Ok(())
}

/// How long the damage probe waits for a repeat frame of a static window.
const PROBE_WAIT: Duration = Duration::from_millis(250);

/// Each sample opens a fresh session, as opening the overview would, so it
/// measures a first frame. A repeat frame is only answered once the window
/// has new damage, which the probe at the end records.
fn measure(
    capturer: &mut Capturer,
    toplevel: &Toplevel,
    name: &str,
    repeat: usize,
    dir: Option<&Path>,
) -> WindowReport {
    let mut report = WindowReport {
        name: name.to_owned(),
        address: None,
        placement: None,
        setup_ms: None,
        shm_formats: Vec::new(),
        dmabuf_formats: 0,
        width: 0,
        height: 0,
        stride: 0,
        format: String::new(),
        transform: String::new(),
        buffer_bytes: 0,
        captures: 0,
        failures: Vec::new(),
        capture_ms: None,
        convert_ms: None,
        repeat_without_damage: None,
        stats: None,
        own_accent_fraction: 0.0,
        other_accent_fraction: 0.0,
        verdict: Verdict::Failed,
    };

    let mut setup_ms = Vec::new();
    let mut capture_ms = Vec::new();
    let mut convert_ms = Vec::new();
    let mut first: Option<Capture> = None;
    for _ in 0..repeat.max(1) {
        let mut session = match capturer.open(toplevel) {
            Ok(session) => session,
            Err(error) => {
                report.failures.push(format!("{error:#}"));
                break;
            }
        };
        setup_ms.push(session.setup_ms);
        report.shm_formats = session
            .constraints
            .shm_formats
            .iter()
            .map(|f| format!("{f:?}"))
            .collect();
        report.dmabuf_formats = session.constraints.dmabuf_formats;
        let result = capturer.grab(&mut session, TIMEOUT);
        capturer.close(session);
        match result {
            Ok(capture) => {
                capture_ms.push(capture.timing.capture_ms);
                convert_ms.push(capture.timing.convert_ms);
                first.get_or_insert(capture);
            }
            Err((failure, error)) => {
                report.failures.push(format!("{error:#}"));
                if matches!(failure, Failure::Stopped) {
                    break;
                }
            }
        }
    }
    if report.failures.is_empty() {
        report.repeat_without_damage = Some(probe_repeat(capturer, toplevel));
    }

    report.captures = capture_ms.len();
    report.setup_ms = Percentiles::of(&setup_ms);
    report.capture_ms = Percentiles::of(&capture_ms);
    report.convert_ms = Percentiles::of(&convert_ms);
    let Some(capture) = first else {
        return report;
    };
    let (verdict, own, other) = judge(name, &capture.frame);
    report.verdict = if report.failures.is_empty() {
        verdict
    } else {
        Verdict::Failed
    };
    report.own_accent_fraction = own;
    report.other_accent_fraction = other;
    report.stats = Some(capture.frame.stats());
    report.width = capture.width;
    report.height = capture.height;
    report.stride = capture.stride;
    report.format = capture.format.clone();
    report.transform = capture.transform.clone();
    report.buffer_bytes = capture.stride as usize * capture.height as usize;
    if let Some(dir) = dir {
        let path = dir.join(format!("{name}.png"));
        if let Err(error) = save_png(&capture.frame, &path) {
            report
                .failures
                .push(format!("saving {}: {error:#}", path.display()));
        }
    }
    report
}

/// Asks the same session for a second frame of an unchanged window.
fn probe_repeat(capturer: &mut Capturer, toplevel: &Toplevel) -> String {
    let mut session = match capturer.open(toplevel) {
        Ok(session) => session,
        Err(error) => return format!("error: {error:#}"),
    };
    let outcome = match capturer.grab(&mut session, TIMEOUT) {
        Err((_, error)) => format!("error: {error:#}"),
        Ok(_) => match capturer.grab(&mut session, PROBE_WAIT) {
            Ok(capture) => format!("new frame after {:.1} ms", capture.timing.capture_ms),
            Err((Failure::Unchanged, _)) => {
                format!("waits for damage (none within {PROBE_WAIT:?})")
            }
            Err((_, error)) => format!("error: {error:#}"),
        },
    };
    capturer.close(session);
    outcome
}

pub fn save_png(frame: &Frame, path: &Path) -> Result<()> {
    let image = image::RgbaImage::from_raw(frame.width, frame.height, frame.to_rgba())
        .context("frame size does not match its pixels")?;
    image.save(path)?;
    Ok(())
}

fn print_summary(report: &Report) {
    println!(
        "Capture check `{}` · Hyprland {} · address mapping {} · {} toplevels listed",
        report.label,
        report.hyprland_version.as_deref().unwrap_or("?"),
        if report.address_mapping { "yes" } else { "no" },
        report.toplevels_listed,
    );
    for w in &report.windows {
        let place = w
            .placement
            .as_ref()
            .map(|p| {
                let mut flags = vec![format!("ws {}", p.workspace)];
                flags.push(
                    if p.workspace_visible {
                        "visible"
                    } else {
                        "inactive"
                    }
                    .into(),
                );
                for (on, label) in [
                    (p.floating, "floating"),
                    (p.fullscreen, "fullscreen"),
                    (p.xwayland, "xwayland"),
                    (p.grouped, "grouped"),
                    (p.hidden, "hidden"),
                ] {
                    if on {
                        flags.push(label.into());
                    }
                }
                if !p.overlaps.is_empty() {
                    flags.push(format!("overlaps {}", p.overlaps.join(",")));
                }
                flags.join(" ")
            })
            .unwrap_or_else(|| "placement unknown".into());
        let timing = w
            .capture_ms
            .as_ref()
            .map(|t| format!("capture p50 {:.1} ms p95 {:.1} ms", t.p50, t.p95))
            .unwrap_or_default();
        println!(
            "  {:<12} {:?} · {place} · {}x{} {} · {timing} · {} fresh sessions",
            w.name, w.verdict, w.width, w.height, w.format, w.captures
        );
        if let Some(probe) = &w.repeat_without_damage {
            println!("      repeat frame without damage: {probe}");
        }
        for failure in &w.failures {
            println!("      failure: {failure}");
        }
    }
    if let (Some(before), Some(after)) = (report.rss_kib_before, report.rss_kib_after) {
        println!("  RSS {before} KiB → {after} KiB");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use orbit_ui::theme;

    fn card(width: u32, height: u32, border: u32, accent: u32) -> Frame {
        let mut bgra = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let edge = x < border || y < border || x >= width - border || y >= height - border;
                let rgb = if edge { accent } else { theme::SLATE.hex };
                bgra.extend_from_slice(&[rgb as u8, (rgb >> 8) as u8, (rgb >> 16) as u8, 255]);
            }
        }
        Frame {
            width,
            height,
            bgra,
        }
    }

    #[test]
    fn recognizes_a_fixture_by_its_accent() {
        let a = accent_for("tiled-a").hex;
        let b = accent_for("tiled-b").hex;
        assert_ne!(a, b);
        assert_eq!(judge("tiled-a", &card(120, 80, 5, a)).0, Verdict::Ok);
        assert_eq!(
            judge("tiled-a", &card(120, 80, 5, b)).0,
            Verdict::WrongContent
        );
        let black = Frame {
            width: 4,
            height: 4,
            bgra: vec![0; 64],
        };
        assert_eq!(judge("tiled-a", &black).0, Verdict::Blank);
    }

    #[test]
    fn percentiles() {
        let p = Percentiles::of(&[5.0, 1.0, 3.0, 2.0, 4.0]).unwrap();
        assert_eq!((p.p50, p.p95, p.max), (3.0, 5.0, 5.0));
        assert!(Percentiles::of(&[]).is_none());
    }
}
