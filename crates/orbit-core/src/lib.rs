//! Framework-free model, identities, events and state transitions.
//!
//! Nothing in this crate may depend on GPUI, Wayland or Hyprland. Platform
//! adapters translate into and out of the types defined here (ORB-005).

pub mod event;
pub mod model;
pub mod overview;
pub mod search;
pub mod settings;
pub mod store;

pub use event::Event;
pub use model::{
    Epoch, Kinds, Model, Monitor, OpId, Refresh, Window, WindowId, WindowInfo, Workspace,
    WorkspaceId, WorkspaceKind,
};
pub use overview::{
    Announcement, Card, Dest, Intent, KEYBOARD_PATHS, Overview, Region, Tile, Tone, Unavailable,
};
pub use search::{Field, Query, app_name};
pub use settings::Settings;
pub use store::{
    Command, Config, Effect, Failure, OpState, Outcome, Rejected, RequestError, Store, Sync, Target,
};
