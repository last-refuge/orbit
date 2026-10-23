//! Typed control requests (Gate C, ORB-006).
//!
//! Hyprland 0.56 replaced the hyprlang dispatchers with a Lua API; the
//! request socket takes `dispatch <lua>` and evaluates it as
//! `return hl.dispatch(<lua>)`. Older releases take `dispatch <name> <args>`.
//! The syntax is chosen from the running version.
//!
//! Every request names its window explicitly: a Lua dispatch without a
//! `window` acts on the focused window, which may not be Orbit's to move.
//! Values interpolated into a request are a hex address, a number, or a
//! workspace name checked by [`safe_name`]; titles never are.
//!
//! A reply of `ok` only means the request parsed. Hyprland also answers
//! `ok` for an address that does not exist, so callers verify the
//! resulting state (the store does this for moves).

use std::fmt::Write as _;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Syntax {
    /// `hl.dsp.*`, Hyprland 0.56 and later (verified on 0.56.2).
    Lua,
    /// Hyprlang dispatchers (0.54 reference).
    Legacy,
}

impl Syntax {
    /// Picks by release version such as `0.56.2` or tag `v0.56.2`. Unknown
    /// versions get the current syntax.
    pub fn for_version(version: &str) -> Self {
        let mut parts = version
            .trim_start_matches('v')
            .split('.')
            .map(|part| part.parse::<u32>().ok());
        match (parts.next().flatten(), parts.next().flatten()) {
            (Some(0), Some(minor)) if minor < 56 => Syntax::Legacy,
            _ => Syntax::Lua,
        }
    }
}

/// A workspace as a dispatch names it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Dest {
    Numbered(i64),
    Named(String),
    /// The name without its `special:` prefix.
    Special(String),
}

impl Dest {
    /// From a Hyprland workspace id and name, as the model has them.
    pub fn from_workspace(id: i64, name: &str) -> Self {
        if let Some(special) = name.strip_prefix("special:") {
            Dest::Special(special.to_owned())
        } else if name == "special" {
            Dest::Special(String::new())
        } else if id > 0 {
            Dest::Numbered(id)
        } else {
            Dest::Named(name.to_owned())
        }
    }

    fn selector(&self) -> Result<String, String> {
        match self {
            Dest::Numbered(n) if *n > 0 => Ok(n.to_string()),
            Dest::Numbered(n) => Err(format!("invalid workspace number {n}")),
            Dest::Named(name) => Ok(format!("name:{}", safe_name(name)?)),
            Dest::Special(name) if name.is_empty() => Ok("special".into()),
            Dest::Special(name) => Ok(format!("special:{}", safe_name(name)?)),
        }
    }
}

/// Workspace names go inside a quoted Lua string and a comma-separated
/// hyprlang argument, so refuse anything that could end either.
pub fn safe_name(name: &str) -> Result<&str, String> {
    let bad = |c: char| c.is_control() || matches!(c, '\'' | '"' | '\\' | ',' | '[' | ']');
    if name.is_empty() || name.len() > 64 || name.chars().any(bad) {
        Err("Orbit can't address a workspace with this name".into())
    } else {
        Ok(name)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Control {
    /// Swap complete numbered workspace objects, never reconstruct window layouts.
    SwapWorkspaces {
        source: i64,
        target: i64,
        source_windows: Vec<u64>,
        target_windows: Vec<u64>,
    },
    /// Move a window without following it or changing focus.
    MoveSilent { address: u64, dest: Dest },
    /// Focus a window, switching to its workspace.
    FocusWindow { address: u64 },
    /// Put a window on top of the floating stack. Focus alone does not
    /// promise it: with every window floating, an activated window could
    /// stay behind the one that had focus.
    RaiseWindow { address: u64 },
    /// Switch the focused monitor to a workspace (or the monitor showing it).
    FocusWorkspace { dest: Dest },
}

impl Control {
    pub fn verify_swap(&self, refresh: &orbit_core::Refresh) -> Result<(), String> {
        let Self::SwapWorkspaces {
            source,
            target,
            source_windows,
            target_windows,
        } = self
        else {
            return Ok(());
        };
        let windows = refresh
            .windows
            .as_ref()
            .ok_or("Missing workspace verification snapshot")?;
        for (addresses, destination) in [(source_windows, target), (target_windows, source)] {
            if addresses.iter().any(|address| {
                !windows
                    .iter()
                    .any(|w| w.address == *address && w.workspace.0 == *destination)
            }) {
                return Err(
                    "Workspace contents changed before verification; refresh to check the result"
                        .into(),
                );
            }
        }
        Ok(())
    }

    /// The text after `dispatch ` on the request socket.
    pub fn encode(&self, syntax: Syntax) -> Result<String, String> {
        if let Self::SwapWorkspaces {
            source,
            target,
            source_windows,
            target_windows,
        } = self
        {
            if syntax != Syntax::Lua
                || *source <= 0
                || *target <= 0
                || source == target
                || *source >= u32::MAX as i64
                || *target >= u32::MAX as i64
            {
                return Err(
                    "Workspace swapping requires two numbered workspaces and Hyprland 0.56+".into(),
                );
            }
            let addresses = |windows: &[u64]| {
                windows
                    .iter()
                    .map(|a| format!("'address:{a:#x}'"))
                    .collect::<Vec<_>>()
                    .join(",")
            };
            let chunk = include_str!("swap.lua")
                .replace("@SOURCE@", &source.to_string())
                .replace("@TARGET@", &target.to_string())
                .replace("@SOURCE_WINDOWS@", &addresses(source_windows))
                .replace("@TARGET_WINDOWS@", &addresses(target_windows));
            return Ok(chunk);
        }
        let mut out = String::new();
        match (self, syntax) {
            (Self::SwapWorkspaces { .. }, _) => unreachable!(),
            (Control::MoveSilent { address, dest }, Syntax::Lua) => write!(
                out,
                "hl.dsp.window.move({{ window = 'address:{address:#x}', workspace = '{}', follow = false }})",
                dest.selector()?
            ),
            (Control::MoveSilent { address, dest }, Syntax::Legacy) => write!(
                out,
                "movetoworkspacesilent {},address:{address:#x}",
                dest.selector()?
            ),
            (Control::FocusWindow { address }, Syntax::Lua) => {
                write!(out, "hl.dsp.focus({{ window = 'address:{address:#x}' }})")
            }
            (Control::FocusWindow { address }, Syntax::Legacy) => {
                write!(out, "focuswindow address:{address:#x}")
            }
            (Control::RaiseWindow { address }, Syntax::Lua) => write!(
                out,
                "hl.dsp.window.alter_zorder({{ mode = 'top', window = 'address:{address:#x}' }})"
            ),
            (Control::RaiseWindow { address }, Syntax::Legacy) => {
                write!(out, "alterzorder top,address:{address:#x}")
            }
            (Control::FocusWorkspace { dest }, Syntax::Lua) => match dest {
                Dest::Special(name) => write!(
                    out,
                    "hl.dsp.workspace.toggle_special('{}')",
                    if name.is_empty() { "" } else { safe_name(name)? }
                ),
                dest => write!(out, "hl.dsp.focus({{ workspace = '{}' }})", dest.selector()?),
            },
            (Control::FocusWorkspace { dest }, Syntax::Legacy) => match dest {
                Dest::Special(name) => write!(
                    out,
                    "togglespecialworkspace {}",
                    if name.is_empty() { "" } else { safe_name(name)? }
                ),
                dest => write!(out, "workspace {}", dest.selector()?),
            },
        }
        .expect("writing to a String");
        Ok(out)
    }
}

/// Reads a dispatch reply: `ok`, or an error text.
pub fn check_reply(reply: &str) -> Result<(), String> {
    let reply = reply.trim();
    if reply == "ok" {
        Ok(())
    } else {
        // Lua errors quote the request, which holds only checked values.
        Err(reply.strip_prefix("error: ").unwrap_or(reply).to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_swaps_require_supported_ids_and_verified_contents() {
        let swap = Control::SwapWorkspaces {
            source: 1,
            target: 2,
            source_windows: vec![0xa],
            target_windows: vec![0xb],
        };
        let lua = swap.encode(Syntax::Lua).unwrap();
        assert!(lua.contains("local source, target = 1, 2"));
        assert!(lua.contains("check(a, {'address:0xa'})"));
        assert!(!lua.contains('@'));
        assert!(swap.encode(Syntax::Legacy).is_err());
        for (source, target) in [(0, 1), (-98, 1), (1, 1), (1, u32::MAX as i64)] {
            assert!(
                Control::SwapWorkspaces {
                    source,
                    target,
                    source_windows: vec![],
                    target_windows: vec![]
                }
                .encode(Syntax::Lua)
                .is_err()
            );
        }
        let clients: Vec<crate::snapshot::Client> = serde_json::from_value(serde_json::json!([
            { "address":"0xa", "mapped":true, "workspace":{"id":2,"name":"2"} },
            { "address":"0xb", "mapped":true, "workspace":{"id":1,"name":"1"} }
        ]))
        .unwrap();
        let mut refresh = orbit_core::Refresh::full(
            vec![],
            vec![],
            crate::snapshot::core_windows(&clients),
            None,
        );
        assert!(swap.verify_swap(&refresh).is_ok());
        refresh.windows.as_mut().unwrap()[0].workspace = orbit_core::WorkspaceId(1);
        assert!(swap.verify_swap(&refresh).is_err());
        refresh.windows = None;
        assert!(swap.verify_swap(&refresh).is_err());
    }

    #[test]
    fn picks_syntax_by_version() {
        assert_eq!(Syntax::for_version("0.56.2"), Syntax::Lua);
        assert_eq!(Syntax::for_version("v0.57.0"), Syntax::Lua);
        assert_eq!(Syntax::for_version("0.54.0"), Syntax::Legacy);
        assert_eq!(Syntax::for_version("v0.41.2"), Syntax::Legacy);
        assert_eq!(Syntax::for_version("unknown"), Syntax::Lua);
    }

    #[test]
    fn encodes_lua_with_an_explicit_window() {
        let moved = Control::MoveSilent {
            address: 0x55966ddf5cd0,
            dest: Dest::Numbered(91),
        };
        assert_eq!(
            moved.encode(Syntax::Lua).unwrap(),
            "hl.dsp.window.move({ window = 'address:0x55966ddf5cd0', workspace = '91', follow = false })"
        );
        assert_eq!(
            Control::FocusWindow { address: 0xab }
                .encode(Syntax::Lua)
                .unwrap(),
            "hl.dsp.focus({ window = 'address:0xab' })"
        );
        assert_eq!(
            Control::FocusWorkspace {
                dest: Dest::Named("web".into())
            }
            .encode(Syntax::Lua)
            .unwrap(),
            "hl.dsp.focus({ workspace = 'name:web' })"
        );
        // Hyprland 0.56.2: hl.dsp.window.alter_zorder takes `mode` and an
        // explicit `window`; without one it would act on the focused window.
        assert_eq!(
            Control::RaiseWindow { address: 0xab }
                .encode(Syntax::Lua)
                .unwrap(),
            "hl.dsp.window.alter_zorder({ mode = 'top', window = 'address:0xab' })"
        );
        assert_eq!(
            Control::RaiseWindow { address: 0xab }
                .encode(Syntax::Legacy)
                .unwrap(),
            "alterzorder top,address:0xab"
        );
        assert_eq!(
            Control::MoveSilent {
                address: 1,
                dest: Dest::Special("orbitfx".into())
            }
            .encode(Syntax::Lua)
            .unwrap(),
            "hl.dsp.window.move({ window = 'address:0x1', workspace = 'special:orbitfx', follow = false })"
        );
    }

    #[test]
    fn encodes_legacy() {
        let moved = Control::MoveSilent {
            address: 0xab,
            dest: Dest::Numbered(3),
        };
        assert_eq!(
            moved.encode(Syntax::Legacy).unwrap(),
            "movetoworkspacesilent 3,address:0xab"
        );
        assert_eq!(
            Control::FocusWorkspace {
                dest: Dest::Numbered(2)
            }
            .encode(Syntax::Legacy)
            .unwrap(),
            "workspace 2"
        );
    }

    #[test]
    fn refuses_names_that_could_break_out() {
        for name in ["a'b", "a\\b", "a,b", "a\nb", "", "x]"] {
            let control = Control::MoveSilent {
                address: 1,
                dest: Dest::Named(name.into()),
            };
            assert!(control.encode(Syntax::Lua).is_err(), "{name:?}");
            assert!(control.encode(Syntax::Legacy).is_err(), "{name:?}");
        }
        assert!(
            Control::MoveSilent {
                address: 1,
                dest: Dest::Numbered(0)
            }
            .encode(Syntax::Lua)
            .is_err()
        );
    }

    #[test]
    fn maps_model_workspaces() {
        assert_eq!(Dest::from_workspace(4, "4"), Dest::Numbered(4));
        assert_eq!(
            Dest::from_workspace(-1337, "web"),
            Dest::Named("web".into())
        );
        assert_eq!(
            Dest::from_workspace(-98, "special:orbitfx"),
            Dest::Special("orbitfx".into())
        );
    }

    #[test]
    fn reads_replies() {
        assert_eq!(check_reply("ok\n"), Ok(()));
        assert_eq!(
            check_reply("error: [string \"…\"]:1: unexpected symbol"),
            Err("[string \"…\"]:1: unexpected symbol".into())
        );
    }
}
