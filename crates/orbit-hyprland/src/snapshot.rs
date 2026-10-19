//! Typed view of the snapshot queries, tolerant of fields that differ between
//! Hyprland releases. Titles are read for display only: they are never
//! serialized, and their `Debug` form is redacted.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use orbit_core::{Kinds, Refresh, WindowInfo, WorkspaceId};

use crate::{Instance, Query};

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Version {
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub tag: String,
    #[serde(default)]
    pub commit: String,
    #[serde(default)]
    pub branch: String,
}

impl Version {
    /// Best human label: the release version, else the git tag, else the commit.
    pub fn label(&self) -> String {
        [&self.version, &self.tag, &self.commit]
            .into_iter()
            .find(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| "unknown".into())
    }
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct WorkspaceRef {
    pub id: i64,
    #[serde(default)]
    pub name: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub width: i64,
    #[serde(default)]
    pub height: i64,
    #[serde(default)]
    pub refresh_rate: f64,
    #[serde(default)]
    pub scale: f64,
    #[serde(default)]
    pub transform: i64,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub disabled: bool,
    /// Logical position in the global space.
    #[serde(default)]
    pub x: i64,
    #[serde(default)]
    pub y: i64,
    #[serde(default)]
    pub active_workspace: WorkspaceRef,
    #[serde(default)]
    pub special_workspace: WorkspaceRef,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub id: i64,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub monitor: String,
    #[serde(default)]
    pub windows: i64,
    #[serde(default, rename = "hasfullscreen")]
    pub has_fullscreen: bool,
    #[serde(default, rename = "ispersistent")]
    pub is_persistent: bool,
}

impl Workspace {
    /// Special workspaces have negative ids and `special:` names.
    pub fn is_special(&self) -> bool {
        self.id < 0 || self.name.starts_with("special:")
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Client {
    pub address: String,
    #[serde(default = "yes")]
    pub mapped: bool,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub workspace: WorkspaceRef,
    #[serde(default)]
    pub monitor: i64,
    #[serde(default)]
    pub class: String,
    #[serde(default)]
    pub floating: bool,
    #[serde(default)]
    pub pinned: bool,
    #[serde(default)]
    pub xwayland: bool,
    /// An integer fullscreen mode in current releases, a bool in older ones.
    #[serde(default)]
    pub fullscreen: Value,
    #[serde(default)]
    pub grouped: Vec<String>,
    /// Logical position and size.
    #[serde(default)]
    pub at: [i64; 2],
    #[serde(default)]
    pub size: [i64; 2],
    #[serde(default)]
    pub pid: i64,
    #[serde(default, skip_serializing)]
    pub title: Title,
}

/// A window title. Shown in the overview, never printed or recorded.
#[derive(Clone, Default, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct Title(pub String);

impl std::fmt::Debug for Title {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "<title len={}>", self.0.len())
    }
}

/// Parses `0x55d0c1a2b3c0` as `hyprctl` prints window addresses.
pub fn parse_address(text: &str) -> Option<u64> {
    u64::from_str_radix(text.strip_prefix("0x").unwrap_or(text), 16).ok()
}

fn yes() -> bool {
    true
}

impl Client {
    pub fn is_fullscreen(&self) -> bool {
        match &self.fullscreen {
            Value::Bool(value) => *value,
            Value::Number(mode) => mode.as_i64().is_some_and(|mode| mode != 0),
            _ => false,
        }
    }
}

/// One pass over the snapshot queries. Not atomic: the compositor can change
/// between queries, which reconciliation (ORB-005) has to account for.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Snapshot {
    pub version: Version,
    pub monitors: Vec<Monitor>,
    pub workspaces: Vec<Workspace>,
    pub clients: Vec<Client>,
    pub active_workspace: Option<WorkspaceRef>,
    pub active_window: Option<String>,
    pub workspace_rule_count: usize,
}

impl Snapshot {
    pub fn take(instance: &Instance) -> anyhow::Result<Self> {
        Self::from_values(|query| instance.query(query))
    }

    /// Converts to the core model's input. Unmapped clients are left out.
    pub fn to_refresh(&self) -> orbit_core::Refresh {
        let active = self.active_window.as_deref().and_then(parse_address);
        orbit_core::Refresh::full(
            core_monitors(&self.monitors),
            core_workspaces(&self.workspaces),
            core_windows(&self.clients),
            active,
        )
    }

    /// Builds a snapshot from any source of query results, such as recorded
    /// fixtures.
    pub fn from_values(
        mut fetch: impl FnMut(Query) -> anyhow::Result<Value>,
    ) -> anyhow::Result<Self> {
        let active_window = fetch(Query::ActiveWindow)?;
        Ok(Self {
            version: serde_json::from_value(fetch(Query::Version)?)?,
            monitors: serde_json::from_value(fetch(Query::Monitors)?)?,
            workspaces: serde_json::from_value(fetch(Query::Workspaces)?)?,
            clients: serde_json::from_value(fetch(Query::Clients)?)?,
            active_workspace: serde_json::from_value(fetch(Query::ActiveWorkspace)?).ok(),
            active_window: active_window["address"].as_str().map(str::to_owned),
            workspace_rule_count: fetch(Query::WorkspaceRules)?.as_array().map_or(0, Vec::len),
        })
    }
}

pub fn core_monitors(monitors: &[Monitor]) -> Vec<orbit_core::Monitor> {
    monitors
        .iter()
        .filter(|m| !m.disabled)
        .map(|m| {
            let scale = if m.scale > 0.0 { m.scale } else { 1.0 };
            orbit_core::Monitor {
                id: m.id,
                name: m.name.clone(),
                focused: m.focused,
                active_workspace: WorkspaceId(m.active_workspace.id),
                special_workspace: (m.special_workspace.id != 0)
                    .then_some(WorkspaceId(m.special_workspace.id)),
                x: m.x as f64,
                y: m.y as f64,
                width: m.width as f64 / scale,
                height: m.height as f64 / scale,
                scale,
            }
        })
        .collect()
}

pub fn core_workspaces(workspaces: &[Workspace]) -> Vec<orbit_core::Workspace> {
    workspaces
        .iter()
        .map(|w| orbit_core::Workspace {
            id: WorkspaceId(w.id),
            name: w.name.clone(),
            monitor: (!w.monitor.is_empty()).then(|| w.monitor.clone()),
        })
        .collect()
}

pub fn core_windows(clients: &[Client]) -> Vec<WindowInfo> {
    clients
        .iter()
        .filter(|c| c.mapped)
        .filter_map(|c| {
            Some(WindowInfo {
                address: parse_address(&c.address)?,
                workspace: WorkspaceId(c.workspace.id),
                class: c.class.clone(),
                title: c.title.0.clone(),
                pid: c.pid,
                floating: c.floating,
                fullscreen: c.is_fullscreen(),
                xwayland: c.xwayland,
                hidden: c.hidden,
                at: (c.at[0], c.at[1]),
                size: (c.size[0], c.size[1]),
            })
        })
        .collect()
}

/// Queries only the parts of the compositor state in `kinds`.
pub fn query_refresh(instance: &Instance, kinds: Kinds) -> anyhow::Result<Refresh> {
    let mut refresh = Refresh::default();
    if kinds.contains(Kinds::MONITORS) {
        let monitors: Vec<Monitor> = serde_json::from_value(instance.query(Query::Monitors)?)?;
        refresh.monitors = Some(core_monitors(&monitors));
    }
    if kinds.contains(Kinds::WORKSPACES) {
        let workspaces: Vec<Workspace> =
            serde_json::from_value(instance.query(Query::Workspaces)?)?;
        refresh.workspaces = Some(core_workspaces(&workspaces));
    }
    if kinds.contains(Kinds::WINDOWS) {
        let clients: Vec<Client> = serde_json::from_value(instance.query(Query::Clients)?)?;
        let active = instance.query(Query::ActiveWindow)?;
        refresh.windows = Some(core_windows(&clients));
        refresh.active_window = Some(active["address"].as_str().and_then(parse_address));
    }
    Ok(refresh)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn fixture(query: Query) -> anyhow::Result<Value> {
        Ok(match query {
            Query::Version => {
                json!({"branch": "main", "commit": "abc", "version": "0.54.0", "tag": "v0.54.0"})
            }
            Query::Monitors => json!([{
                "id": 0, "name": "DP-1", "width": 2560, "height": 1440, "refreshRate": 143.99,
                "scale": 1.25, "transform": 0, "focused": true, "disabled": false,
                "activeWorkspace": {"id": 1, "name": "1"},
                "specialWorkspace": {"id": 0, "name": ""}
            }]),
            Query::Workspaces => json!([
                {"id": 1, "name": "1", "monitor": "DP-1", "windows": 2, "hasfullscreen": false, "ispersistent": true},
                {"id": -98, "name": "special:scratch", "monitor": "DP-1", "windows": 1, "hasfullscreen": false}
            ]),
            Query::ActiveWorkspace => json!({"id": 1, "name": "1"}),
            Query::Clients => json!([
                {"address": "0x55aa", "mapped": true, "workspace": {"id": 1, "name": "1"}, "class": "kitty",
                 "title": "~", "floating": false, "xwayland": false, "fullscreen": 2, "grouped": []},
                {"address": "0x55bb", "workspace": {"id": -98, "name": "special:scratch"}, "class": "steam",
                 "xwayland": true, "fullscreen": false, "pinned": true}
            ]),
            Query::ActiveWindow => json!({"address": "0x55aa", "class": "kitty"}),
            Query::WorkspaceRules => json!([{"workspaceString": "1", "persistent": true}]),
        })
    }

    #[test]
    fn parses_current_and_legacy_shapes() {
        let snapshot = Snapshot::from_values(fixture).unwrap();
        assert_eq!(snapshot.version.label(), "0.54.0");
        assert_eq!(snapshot.monitors[0].scale, 1.25);
        assert!(snapshot.workspaces[1].is_special());
        assert!(snapshot.clients[0].is_fullscreen());
        assert!(!snapshot.clients[1].is_fullscreen());
        assert!(snapshot.clients[1].xwayland && snapshot.clients[1].pinned);
        assert_eq!(snapshot.active_window.as_deref(), Some("0x55aa"));
        assert_eq!(snapshot.workspace_rule_count, 1);
    }

    #[test]
    fn empty_active_window_is_none() {
        let snapshot = Snapshot::from_values(|query| match query {
            Query::ActiveWindow => Ok(json!({})),
            other => fixture(other),
        })
        .unwrap();
        assert_eq!(snapshot.active_window, None);
    }

    #[test]
    fn converts_to_the_core_model() {
        use orbit_core::{Store, WorkspaceId};
        let refresh = Snapshot::from_values(fixture).unwrap().to_refresh();
        let monitors = refresh.monitors.as_ref().unwrap();
        assert_eq!((monitors[0].width, monitors[0].height), (2048.0, 1152.0));
        assert_eq!(monitors[0].special_workspace, None);
        let windows = refresh.windows.as_ref().unwrap();
        assert_eq!(windows[0].address, 0x55aa);
        assert_eq!(windows[1].workspace, WorkspaceId(-98));
        assert!(windows[0].fullscreen && windows[1].xwayland);
        assert_eq!(refresh.active_window, Some(Some(0x55aa)));

        let mut store = Store::new(Default::default());
        let (epoch, _) = store.connect(0);
        store.on_refresh(epoch, refresh).unwrap();
        assert_eq!(store.model().windows.len(), 2);
        assert_eq!(store.model().active_window.unwrap().address, 0x55aa);
    }

    #[test]
    fn serialized_snapshot_has_no_titles() {
        let snapshot = Snapshot::from_values(fixture).unwrap();
        assert_eq!(snapshot.clients[0].title.0, "~");
        assert!(!serde_json::to_string(&snapshot).unwrap().contains("title"));
        assert!(!format!("{snapshot:?}").contains("\"~\""));
        assert_eq!(snapshot.to_refresh().windows.unwrap()[0].title, "~");
    }
}
