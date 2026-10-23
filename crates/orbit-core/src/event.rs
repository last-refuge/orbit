//! Compositor changes, already parsed by a platform adapter.

use crate::model::{Kinds, WorkspaceId};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Event {
    WindowOpened {
        address: u64,
        workspace_name: String,
        class: String,
        title: String,
    },
    WindowClosed {
        address: u64,
    },
    WindowMoved {
        address: u64,
        workspace: WorkspaceId,
        workspace_name: String,
    },
    WindowTitle {
        address: u64,
        title: String,
    },
    ActiveWindow {
        address: Option<u64>,
    },
    Floating {
        address: u64,
        floating: bool,
    },
    /// Fullscreen changed on the active window; the event does not say which.
    Fullscreen,
    WorkspaceCreated {
        id: WorkspaceId,
        name: String,
    },
    WorkspaceDestroyed {
        id: WorkspaceId,
    },
    WorkspaceRenamed {
        id: WorkspaceId,
        name: String,
    },
    WorkspaceIdChanged {
        old: WorkspaceId,
        new: WorkspaceId,
    },
    WorkspaceMoved {
        id: WorkspaceId,
        monitor: String,
    },
    /// The focused monitor now shows this workspace.
    ActiveWorkspace {
        id: WorkspaceId,
    },
    FocusedMonitor {
        monitor: String,
        workspace: WorkspaceId,
    },
    /// `None` closes the special workspace on that monitor.
    ActiveSpecial {
        monitor: String,
        workspace: Option<WorkspaceId>,
    },
    MonitorAdded {
        name: String,
    },
    MonitorRemoved {
        name: String,
    },
    ConfigReloaded,
    /// A layer-shell surface closed, such as Orbit's own overlay.
    LayerClosed {
        namespace: String,
    },
}

impl Event {
    /// The parts of the model this event changes.
    pub fn touches(&self) -> Kinds {
        use Event::*;
        match self {
            WindowOpened { .. } | WindowClosed { .. } | WindowMoved { .. } => {
                Kinds::WINDOWS | Kinds::WORKSPACES
            }
            WindowTitle { .. } | ActiveWindow { .. } | Floating { .. } | Fullscreen => {
                Kinds::WINDOWS
            }
            WorkspaceCreated { .. } | WorkspaceDestroyed { .. } | WorkspaceRenamed { .. } => {
                Kinds::WORKSPACES
            }
            WorkspaceIdChanged { .. } => Kinds::ALL,
            WorkspaceMoved { .. } => Kinds::WORKSPACES | Kinds::MONITORS,
            ActiveWorkspace { .. } | FocusedMonitor { .. } | ActiveSpecial { .. } => {
                Kinds::MONITORS
            }
            MonitorAdded { .. } | MonitorRemoved { .. } | ConfigReloaded => Kinds::ALL,
            LayerClosed { .. } => Kinds::NONE,
        }
    }
}
