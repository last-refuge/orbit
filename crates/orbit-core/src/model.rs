//! Identities and the compositor model Orbit keeps in memory.

use std::collections::BTreeMap;

/// One connection to one compositor session. A reconnect or compositor
/// restart starts a new epoch; anything from an older epoch is rejected.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Epoch(pub u64);

/// A window as Orbit knows it: the compositor address plus a local lifetime
/// generation, because compositors reuse addresses after a window closes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId {
    pub address: u64,
    pub generation: u64,
}

/// Hyprland workspace id: positive for numbered workspaces, negative for
/// named and special ones (the name tells those apart).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WorkspaceId(pub i64);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct OpId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WorkspaceKind {
    Numbered,
    Named,
    Special,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Monitor {
    pub id: i64,
    pub name: String,
    pub focused: bool,
    pub active_workspace: WorkspaceId,
    /// The special workspace shown on this monitor, if one is open.
    pub special_workspace: Option<WorkspaceId>,
    /// Logical position in the global space and size (physical / scale).
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
    pub scale: f64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Workspace {
    pub id: WorkspaceId,
    pub name: String,
    pub monitor: Option<String>,
}

impl Workspace {
    pub fn kind(&self) -> WorkspaceKind {
        if self.name.starts_with("special") {
            WorkspaceKind::Special
        } else if self.id.0 > 0 {
            WorkspaceKind::Numbered
        } else {
            WorkspaceKind::Named
        }
    }
}

/// A window as a snapshot reports it. The store assigns generations.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowInfo {
    pub address: u64,
    pub workspace: WorkspaceId,
    pub class: String,
    /// Kept for display and search only; never logged or used as identity.
    pub title: String,
    pub pid: i64,
    pub floating: bool,
    pub fullscreen: bool,
    pub xwayland: bool,
    pub hidden: bool,
    /// Logical position (global coordinates) and size, as the compositor
    /// reports them; `(0, 0)` until a query fills them in.
    pub at: (i64, i64),
    pub size: (i64, i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    pub workspace: WorkspaceId,
    pub class: String,
    pub title: String,
    pub pid: i64,
    pub floating: bool,
    pub fullscreen: bool,
    pub xwayland: bool,
    pub hidden: bool,
    pub at: (i64, i64),
    pub size: (i64, i64),
}

impl Window {
    fn same_as(&self, info: &WindowInfo) -> bool {
        // A reused address shows up as a different process or class. A pid
        // of 0 was learned from an event and is not known yet.
        (self.pid == 0 || self.pid == info.pid) && self.class == info.class
    }

    fn update(&mut self, info: WindowInfo) {
        self.pid = info.pid;
        self.workspace = info.workspace;
        self.title = info.title;
        self.floating = info.floating;
        self.fullscreen = info.fullscreen;
        self.xwayland = info.xwayland;
        self.hidden = info.hidden;
        self.at = info.at;
        self.size = info.size;
    }
}

/// Fresh query results. A full snapshot has every part; a targeted
/// re-query carries only what was asked for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Refresh {
    pub monitors: Option<Vec<Monitor>>,
    pub workspaces: Option<Vec<Workspace>>,
    pub windows: Option<Vec<WindowInfo>>,
    /// `Some(None)` means no window is active.
    pub active_window: Option<Option<u64>>,
}

impl Refresh {
    pub fn full(
        monitors: Vec<Monitor>,
        workspaces: Vec<Workspace>,
        windows: Vec<WindowInfo>,
        active_window: Option<u64>,
    ) -> Self {
        Self {
            monitors: Some(monitors),
            workspaces: Some(workspaces),
            windows: Some(windows),
            active_window: Some(active_window),
        }
    }

    pub fn kinds(&self) -> Kinds {
        let mut kinds = Kinds::NONE;
        if self.monitors.is_some() {
            kinds |= Kinds::MONITORS;
        }
        if self.workspaces.is_some() {
            kinds |= Kinds::WORKSPACES;
        }
        if self.windows.is_some() || self.active_window.is_some() {
            kinds |= Kinds::WINDOWS;
        }
        kinds
    }
}

/// Which parts of the model a query covers or an event touched.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Kinds(u8);

impl Kinds {
    pub const NONE: Self = Self(0);
    pub const MONITORS: Self = Self(1);
    pub const WORKSPACES: Self = Self(2);
    pub const WINDOWS: Self = Self(4);
    pub const ALL: Self = Self(7);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn and(self, other: Self) -> Self {
        Self(self.0 & other.0)
    }

    pub fn without(self, other: Self) -> Self {
        Self(self.0 & !other.0)
    }
}

impl std::ops::BitOr for Kinds {
    type Output = Self;
    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl std::ops::BitOrAssign for Kinds {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

/// The authoritative picture of the compositor, as last observed.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub monitors: Vec<Monitor>,
    pub workspaces: BTreeMap<WorkspaceId, Workspace>,
    pub windows: BTreeMap<u64, Window>,
    pub active_window: Option<WindowId>,
}

impl Model {
    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows.get(&id.address).filter(|w| w.id == id)
    }

    pub fn workspace_by_name(&self, name: &str) -> Option<&Workspace> {
        self.workspaces.values().find(|w| w.name == name)
    }

    pub fn focused_monitor_mut(&mut self) -> Option<&mut Monitor> {
        self.monitors.iter_mut().find(|m| m.focused)
    }

    /// Windows on a workspace, in address order.
    pub fn windows_on(&self, workspace: WorkspaceId) -> impl Iterator<Item = &Window> {
        self.windows
            .values()
            .filter(move |w| w.workspace == workspace)
    }

    /// Replaces the window set. Returns windows that disappeared or whose
    /// address now belongs to a different window.
    pub(crate) fn replace_windows(
        &mut self,
        infos: Vec<WindowInfo>,
        next_generation: &mut u64,
    ) -> Vec<WindowId> {
        let mut gone = Vec::new();
        let mut old = std::mem::take(&mut self.windows);
        for info in infos {
            let address = info.address;
            let window = match old.remove(&address) {
                Some(mut existing) if existing.same_as(&info) => {
                    existing.update(info);
                    existing
                }
                previous => {
                    if let Some(previous) = previous {
                        gone.push(previous.id);
                    }
                    new_window(info, next_generation)
                }
            };
            self.windows.insert(address, window);
        }
        gone.extend(old.into_values().map(|w| w.id));
        gone
    }
}

pub(crate) fn new_window(info: WindowInfo, next_generation: &mut u64) -> Window {
    *next_generation += 1;
    Window {
        id: WindowId {
            address: info.address,
            generation: *next_generation,
        },
        workspace: info.workspace,
        class: info.class,
        title: info.title,
        pid: info.pid,
        floating: info.floating,
        fullscreen: info.fullscreen,
        xwayland: info.xwayland,
        hidden: info.hidden,
        at: info.at,
        size: info.size,
    }
}
