//! The event socket (`.socket2.sock`): one `EVENT>>DATA` line per change.
//! <https://wiki.hypr.land/IPC/>
//!
//! Hyprland sends most events twice, a legacy form and a `v2` form with ids.
//! Orbit reads the v2 forms and ignores the rest. Fields are comma
//! separated, but titles and workspace names may themselves contain commas,
//! so each event is split with a fixed field count and free text last (or
//! the trailing monitor name is split off from the right).

use std::{
    io::{BufRead, BufReader},
    os::unix::net::UnixStream,
    time::Duration,
};

use anyhow::{Context, Result};
use orbit_core::{Event, WorkspaceId};

use crate::{Instance, redact::Redactor};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub event: String,
    pub reason: &'static str,
}

impl std::fmt::Display for ParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "malformed `{}` event: {}", self.event, self.reason)
    }
}

impl std::error::Error for ParseError {}

/// Parses one line. `Ok(None)` for events Orbit does not use.
pub fn parse(line: &str) -> Result<Option<Event>, ParseError> {
    let line = line.strip_suffix('\n').unwrap_or(line);
    let (name, data) = line.split_once(">>").ok_or(ParseError {
        event: String::new(),
        reason: "missing `>>`",
    })?;
    let err = |reason| ParseError {
        event: name.to_owned(),
        reason,
    };
    let fields = |n: usize| -> Result<Vec<&str>, ParseError> {
        let parts: Vec<&str> = data.splitn(n, ',').collect();
        if parts.len() == n {
            Ok(parts)
        } else {
            Err(err("too few fields"))
        }
    };
    let address = |s: &str| {
        u64::from_str_radix(s.trim_start_matches("0x"), 16).map_err(|_| err("bad address"))
    };
    let workspace = |s: &str| {
        s.parse()
            .map(WorkspaceId)
            .map_err(|_| err("bad workspace id"))
    };
    // `NAME...,MONITOR`: monitor names have no commas, the rest might.
    let with_monitor = || -> Result<(&str, String), ParseError> {
        data.rsplit_once(',')
            .map(|(rest, monitor)| (rest, monitor.to_owned()))
            .ok_or(err("too few fields"))
    };

    let event = match name {
        "openwindow" => {
            let f = fields(4)?;
            Event::WindowOpened {
                address: address(f[0])?,
                workspace_name: f[1].to_owned(),
                class: f[2].to_owned(),
                title: f[3].to_owned(),
            }
        }
        "closewindow" => Event::WindowClosed {
            address: address(data)?,
        },
        "movewindowv2" => {
            let f = fields(3)?;
            Event::WindowMoved {
                address: address(f[0])?,
                workspace: workspace(f[1])?,
                workspace_name: f[2].to_owned(),
            }
        }
        "windowtitlev2" => {
            let f = fields(2)?;
            Event::WindowTitle {
                address: address(f[0])?,
                title: f[1].to_owned(),
            }
        }
        "activewindowv2" => Event::ActiveWindow {
            address: match data.trim_matches(',') {
                "" => None,
                a => Some(address(a)?),
            },
        },
        "changefloatingmode" => {
            let f = fields(2)?;
            Event::Floating {
                address: address(f[0])?,
                floating: f[1] == "1",
            }
        }
        "fullscreen" => Event::Fullscreen,
        "createworkspacev2" => {
            let f = fields(2)?;
            Event::WorkspaceCreated {
                id: workspace(f[0])?,
                name: f[1].to_owned(),
            }
        }
        "destroyworkspacev2" => {
            let f = fields(2)?;
            Event::WorkspaceDestroyed {
                id: workspace(f[0])?,
            }
        }
        "renameworkspace" => {
            let f = fields(2)?;
            Event::WorkspaceRenamed {
                id: workspace(f[0])?,
                name: f[1].to_owned(),
            }
        }
        "moveworkspacev2" => {
            let (rest, monitor) = with_monitor()?;
            let (id, _) = rest.split_once(',').ok_or(err("too few fields"))?;
            Event::WorkspaceMoved {
                id: workspace(id)?,
                monitor,
            }
        }
        "workspacev2" => {
            let f = fields(2)?;
            Event::ActiveWorkspace {
                id: workspace(f[0])?,
            }
        }
        "focusedmonv2" => {
            let f = fields(2)?;
            Event::FocusedMonitor {
                monitor: f[0].to_owned(),
                workspace: workspace(f[1])?,
            }
        }
        "activespecialv2" => {
            let (rest, monitor) = with_monitor()?;
            let (id, _) = rest.split_once(',').ok_or(err("too few fields"))?;
            Event::ActiveSpecial {
                monitor,
                workspace: match id {
                    "" | "0" => None,
                    id => Some(workspace(id)?),
                },
            }
        }
        "monitoraddedv2" => {
            let f = fields(3)?;
            Event::MonitorAdded {
                name: f[1].to_owned(),
            }
        }
        "monitorremovedv2" => {
            let f = fields(3)?;
            Event::MonitorRemoved {
                name: f[1].to_owned(),
            }
        }
        "configreloaded" => Event::ConfigReloaded,
        "closelayer" => Event::LayerClosed {
            namespace: data.to_owned(),
        },
        _ => return Ok(None),
    };
    Ok(Some(event))
}

/// Events whose payload carries no user text (ids, addresses, workspace
/// and monitor names, app classes). Anything else is redacted when
/// recorded.
const SAFE_PAYLOADS: &[&str] = &[
    "closewindow",
    "movewindow",
    "movewindowv2",
    "activewindowv2",
    "changefloatingmode",
    "fullscreen",
    "workspace",
    "workspacev2",
    "createworkspace",
    "createworkspacev2",
    "destroyworkspace",
    "destroyworkspacev2",
    "renameworkspace",
    "moveworkspace",
    "moveworkspacev2",
    "focusedmon",
    "focusedmonv2",
    "activespecial",
    "activespecialv2",
    "monitoradded",
    "monitoraddedv2",
    "monitorremoved",
    "monitorremovedv2",
    "configreloaded",
    "openlayer",
    "closelayer",
    "urgent",
    "pin",
    "windowtitle",
];

/// Redacts user text from one event line for recording: titles in
/// `openwindow`, `windowtitlev2` and `activewindow`, and the whole payload
/// of any event not known to be safe.
pub fn redact_line(line: &str, redactor: &Redactor) -> String {
    let Some((name, data)) = line.split_once(">>") else {
        return redactor.redact_str(line);
    };
    // Keep `n` leading fields, redact the free text after them.
    let keep = |n: usize| -> String {
        let parts: Vec<&str> = data.splitn(n + 1, ',').collect();
        match parts.split_at_checked(n) {
            Some((head, [tail])) => format!("{},{}", head.join(","), redactor.redact_str(tail)),
            _ => redactor.redact_str(data),
        }
    };
    let data = match name {
        "openwindow" => keep(3),
        "windowtitlev2" => keep(1),
        "activewindow" => keep(1),
        name if SAFE_PAYLOADS.contains(&name) => data.to_owned(),
        _ => redactor.redact_str(data),
    };
    format!("{name}>>{data}")
}

/// Reads raw lines from the event socket until `deadline` passes.
pub fn read_lines(
    instance: &Instance,
    duration: Duration,
    mut each: impl FnMut(&str),
) -> Result<()> {
    let path = instance.dir.join(".socket2.sock");
    let stream =
        UnixStream::connect(&path).with_context(|| format!("connecting to {}", path.display()))?;
    let deadline = std::time::Instant::now() + duration;
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    loop {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() {
            return Ok(());
        }
        reader.get_ref().set_read_timeout(Some(remaining))?;
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => return Ok(()),
            Ok(_) => each(line.trim_end_matches('\n')),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(line: &str) -> Event {
        parse(line).unwrap().unwrap()
    }

    #[test]
    fn windows() {
        assert_eq!(
            ok("openwindow>>55d0c1a2b3c0,91,dev.orbit.fixture.a,Title, with, commas"),
            Event::WindowOpened {
                address: 0x55d0c1a2b3c0,
                workspace_name: "91".into(),
                class: "dev.orbit.fixture.a".into(),
                title: "Title, with, commas".into(),
            }
        );
        assert_eq!(
            ok("closewindow>>abc"),
            Event::WindowClosed { address: 0xabc }
        );
        assert_eq!(
            ok("movewindowv2>>abc,-1337,name, with comma"),
            Event::WindowMoved {
                address: 0xabc,
                workspace: WorkspaceId(-1337),
                workspace_name: "name, with comma".into(),
            }
        );
        assert_eq!(
            ok("windowtitlev2>>abc,a,b"),
            Event::WindowTitle {
                address: 0xabc,
                title: "a,b".into()
            }
        );
        assert_eq!(
            ok("activewindowv2>>abc"),
            Event::ActiveWindow {
                address: Some(0xabc)
            }
        );
        assert_eq!(
            ok("activewindowv2>>,"),
            Event::ActiveWindow { address: None }
        );
        assert_eq!(
            ok("activewindowv2>>"),
            Event::ActiveWindow { address: None }
        );
        assert_eq!(
            ok("changefloatingmode>>abc,1"),
            Event::Floating {
                address: 0xabc,
                floating: true
            }
        );
        assert_eq!(ok("fullscreen>>1"), Event::Fullscreen);
    }

    #[test]
    fn workspaces_and_monitors() {
        assert_eq!(
            ok("createworkspacev2>>91,91"),
            Event::WorkspaceCreated {
                id: WorkspaceId(91),
                name: "91".into()
            }
        );
        assert_eq!(
            ok("destroyworkspacev2>>-98,special:orbitfx"),
            Event::WorkspaceDestroyed {
                id: WorkspaceId(-98)
            }
        );
        assert_eq!(
            ok("moveworkspacev2>>3,my, ws,DP-1"),
            Event::WorkspaceMoved {
                id: WorkspaceId(3),
                monitor: "DP-1".into()
            }
        );
        assert_eq!(
            ok("workspacev2>>2,2"),
            Event::ActiveWorkspace { id: WorkspaceId(2) }
        );
        assert_eq!(
            ok("focusedmonv2>>eDP-1,4"),
            Event::FocusedMonitor {
                monitor: "eDP-1".into(),
                workspace: WorkspaceId(4)
            }
        );
        assert_eq!(
            ok("activespecialv2>>-98,special:orbitfx,eDP-1"),
            Event::ActiveSpecial {
                monitor: "eDP-1".into(),
                workspace: Some(WorkspaceId(-98))
            }
        );
        assert_eq!(
            ok("activespecialv2>>,,eDP-1"),
            Event::ActiveSpecial {
                monitor: "eDP-1".into(),
                workspace: None
            }
        );
        assert_eq!(
            ok("monitoraddedv2>>1,DP-1,Dell Inc. U2720Q, rev 2"),
            Event::MonitorAdded {
                name: "DP-1".into()
            }
        );
        assert_eq!(ok("configreloaded>>"), Event::ConfigReloaded);
        assert_eq!(
            ok("closelayer>>orbit"),
            Event::LayerClosed {
                namespace: "orbit".into()
            }
        );
    }

    #[test]
    fn legacy_and_unknown_events_are_ignored() {
        for line in [
            "workspace>>2",
            "activewindow>>kitty,title",
            "movewindow>>abc,2",
            "newthing>>x",
        ] {
            assert_eq!(parse(line), Ok(None), "{line}");
        }
    }

    #[test]
    fn malformed_lines_are_errors() {
        for line in [
            "no separator",
            "closewindow>>zz",
            "movewindowv2>>abc",
            "workspacev2>>x,2",
            "openwindow>>abc,1,class",
            "focusedmonv2>>eDP-1,nan",
            "moveworkspacev2>>3",
        ] {
            assert!(parse(line).is_err(), "{line}");
        }
    }

    #[test]
    fn redaction_keeps_structure_and_drops_text() {
        let r = Redactor::default();
        let open = redact_line("openwindow>>abc,1,kitty,secret, title", &r);
        assert!(
            open.starts_with("openwindow>>abc,1,kitty,<redacted len=13 "),
            "{open}"
        );
        assert!(!open.contains("secret"));
        assert!(parse(&open).unwrap().is_some());

        let title = redact_line("windowtitlev2>>abc,secret", &r);
        assert!(title.starts_with("windowtitlev2>>abc,<redacted"), "{title}");
        let legacy = redact_line("activewindow>>kitty,secret", &r);
        assert!(
            legacy.starts_with("activewindow>>kitty,<redacted"),
            "{legacy}"
        );
        let unknown = redact_line("somefutureevent>>secret", &r);
        assert!(!unknown.contains("secret"), "{unknown}");
        assert_eq!(redact_line("workspacev2>>2,2", &r), "workspacev2>>2,2");
    }
}
