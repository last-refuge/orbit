//! `orbit doctor`: records the host's versions, topology and protocol support
//! for the M0 version matrix (ORB-002). Never prints or records window titles.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
};

use anyhow::{Context, Result};
use orbit_capture::protocols::{self, CapturePath, Gate};
use orbit_hyprland::{Instance, Query, Snapshot, redact::Redactor};
use serde::Serialize;

/// The Zed revision GPUI is pinned to; keep in sync with Cargo.toml.
const GPUI_REV: &str = "56934ec0560bce8929025497411de7f5322fb6ca";

#[derive(Default)]
pub struct Options {
    pub json: bool,
    pub record: Option<PathBuf>,
}

#[derive(Serialize)]
struct Report {
    orbit: &'static str,
    gpui_rev: &'static str,
    session: Session,
    gpus: Vec<Gpu>,
    omarchy: Option<String>,
    hyprland: Result<Snapshot, String>,
    wayland: Result<protocols::Report, String>,
}

#[derive(Serialize)]
struct Session {
    session_type: Option<String>,
    current_desktop: Option<String>,
    wayland_display: bool,
    hyprland_instance: bool,
}

#[derive(Serialize)]
struct Gpu {
    card: String,
    vendor: String,
    driver: Option<String>,
}

pub fn run(options: Options) -> Result<()> {
    let instance = Instance::from_env();
    let report = Report {
        orbit: env!("CARGO_PKG_VERSION"),
        gpui_rev: GPUI_REV,
        session: Session {
            session_type: env::var("XDG_SESSION_TYPE").ok(),
            current_desktop: env::var("XDG_CURRENT_DESKTOP").ok(),
            wayland_display: env::var_os("WAYLAND_DISPLAY").is_some(),
            hyprland_instance: env::var_os("HYPRLAND_INSTANCE_SIGNATURE").is_some(),
        },
        gpus: gpus(),
        omarchy: omarchy_version(),
        hyprland: instance
            .as_ref()
            .map_err(|error| format!("{error:#}"))
            .and_then(|instance| Snapshot::take(instance).map_err(|error| format!("{error:#}"))),
        wayland: protocols::probe().map_err(|error| format!("{error:#}")),
    };

    if let Some(dir) = &options.record {
        record(dir, &report, instance.ok().as_ref())?;
    }
    if options.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        print_text(&report);
        if let Some(dir) = &options.record {
            println!("\nRecorded to {}", dir.display());
        }
    }
    Ok(())
}

/// Writes the report plus every raw IPC reply, titles redacted, as fixtures.
fn record(dir: &Path, report: &Report, instance: Option<&Instance>) -> Result<()> {
    let hypr_dir = dir.join("hyprland");
    fs::create_dir_all(&hypr_dir).with_context(|| format!("creating {}", hypr_dir.display()))?;
    fs::write(
        dir.join("report.json"),
        serde_json::to_string_pretty(report)?,
    )?;
    if let Some(instance) = instance {
        let redactor = Redactor::default();
        for query in Query::ALL {
            let mut value = instance.query(query)?;
            redactor.redact(&mut value);
            fs::write(
                hypr_dir.join(format!("{}.json", query.label())),
                serde_json::to_string_pretty(&value)?,
            )?;
        }
    }
    Ok(())
}

fn print_text(report: &Report) {
    println!("Orbit {} · GPUI {}", report.orbit, &report.gpui_rev[..12]);
    let session = &report.session;
    println!(
        "Session: {} / {} · WAYLAND_DISPLAY {} · Hyprland instance {}",
        session.session_type.as_deref().unwrap_or("?"),
        session.current_desktop.as_deref().unwrap_or("?"),
        if session.wayland_display {
            "set"
        } else {
            "unset"
        },
        if session.hyprland_instance {
            "set"
        } else {
            "unset"
        },
    );
    println!(
        "Omarchy: {}",
        report.omarchy.as_deref().unwrap_or("not detected")
    );
    for gpu in &report.gpus {
        println!(
            "GPU: {} {} ({})",
            gpu.card,
            gpu.vendor,
            gpu.driver.as_deref().unwrap_or("no driver")
        );
    }

    println!();
    match &report.hyprland {
        Ok(snapshot) => print_hyprland(snapshot),
        Err(error) => println!("Hyprland: unavailable: {error}"),
    }

    println!();
    match &report.wayland {
        Ok(wayland) => print_wayland(wayland),
        Err(error) => println!("Wayland: unavailable: {error}"),
    }
}

fn print_hyprland(snapshot: &Snapshot) {
    println!("Hyprland {}", snapshot.version.label());
    for monitor in &snapshot.monitors {
        println!(
            "  monitor {} {}x{}@{:.2} scale {} transform {}{}{} · workspace {}",
            monitor.name,
            monitor.width,
            monitor.height,
            monitor.refresh_rate,
            monitor.scale,
            monitor.transform,
            if monitor.focused { " · focused" } else { "" },
            if monitor.disabled { " · disabled" } else { "" },
            monitor.active_workspace.name,
        );
    }
    let special = snapshot
        .workspaces
        .iter()
        .filter(|w| w.is_special())
        .count();
    let persistent = snapshot
        .workspaces
        .iter()
        .filter(|w| w.is_persistent)
        .count();
    println!(
        "  workspaces: {} live ({special} special, {persistent} persistent) · {} workspace rules",
        snapshot.workspaces.len(),
        snapshot.workspace_rule_count,
    );
    let clients = &snapshot.clients;
    let count = |predicate: fn(&&orbit_hyprland::snapshot::Client) -> bool| {
        clients.iter().filter(predicate).count()
    };
    println!(
        "  windows: {} ({} XWayland, {} floating, {} fullscreen, {} pinned, {} grouped, {} hidden)",
        clients.len(),
        count(|c| c.xwayland),
        count(|c| c.floating),
        count(|c| c.is_fullscreen()),
        count(|c| c.pinned),
        count(|c| !c.grouped.is_empty()),
        count(|c| c.hidden),
    );
    let fixtures = clients
        .iter()
        .filter(|c| c.class.starts_with(crate::fixture::APP_ID_PREFIX))
        .count();
    if fixtures > 0 {
        println!("  Orbit fixture windows: {fixtures}");
    }
}

fn print_wayland(report: &protocols::Report) {
    println!("Wayland: {} globals", report.globals.len());
    for gate in [Gate::Overlay, Gate::Capture] {
        println!(
            "  {}:",
            match gate {
                Gate::Overlay => "Gate A, overlay",
                Gate::Capture => "Gate B, capture",
            }
        );
        for finding in report.findings.iter().filter(|f| f.gate == gate) {
            let status = match (finding.version, finding.required) {
                (Some(version), _) => format!("v{version}"),
                (None, true) => "MISSING (required)".into(),
                (None, false) => "absent".into(),
            };
            println!(
                "    {:<54} {:<20} {}",
                finding.interface, status, finding.purpose
            );
        }
    }
    println!(
        "  candidate capture path: {}",
        match report.capture_path {
            CapturePath::ImageCopyCapture => "ext-image-copy-capture + Hyprland mapping",
            CapturePath::ToplevelExport => "hyprland-toplevel-export",
            CapturePath::MetadataOnly => "none (metadata cards only)",
        }
    );
}

fn gpus() -> Vec<Gpu> {
    let Ok(entries) = fs::read_dir("/sys/class/drm") else {
        return Vec::new();
    };
    let mut gpus: Vec<Gpu> = entries
        .flatten()
        .filter_map(|entry| {
            let card = entry.file_name().into_string().ok()?;
            if !card.starts_with("card") || card.contains('-') {
                return None;
            }
            let device = entry.path().join("device");
            let vendor = fs::read_to_string(device.join("vendor")).ok()?;
            Some(Gpu {
                vendor: match vendor.trim() {
                    "0x8086" => "Intel".into(),
                    "0x1002" => "AMD".into(),
                    "0x10de" => "NVIDIA".into(),
                    other => other.into(),
                },
                driver: fs::read_link(device.join("driver"))
                    .ok()
                    .and_then(|path| Some(path.file_name()?.to_string_lossy().into_owned())),
                card,
            })
        })
        .collect();
    gpus.sort_by(|a, b| a.card.cmp(&b.card));
    gpus
}

/// Omarchy ships an `omarchy-version` helper; fall back to its git checkout.
fn omarchy_version() -> Option<String> {
    let output = |command: &mut Command| {
        command
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned())
            .filter(|text| !text.is_empty())
    };
    output(&mut Command::new("omarchy-version")).or_else(|| {
        let checkout = PathBuf::from(env::var_os("HOME")?).join(".local/share/omarchy");
        output(
            Command::new("git")
                .arg("-C")
                .arg(checkout)
                .args(["describe", "--tags", "--always"]),
        )
    })
}
