//! Hyprland adapter: snapshots, events, typed dispatch and reconciliation over
//! the request and event sockets (Gate C, ORB-002/ORB-006).

pub mod control;
pub mod events;
pub mod ipc;
pub mod link;
pub mod redact;
pub mod snapshot;

pub use ipc::{Instance, Query};
pub use snapshot::Snapshot;
