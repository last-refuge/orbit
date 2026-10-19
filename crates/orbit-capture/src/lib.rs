//! Window preview capture: protocol negotiation, buffers and the bounded
//! preview cache (Gate B, ORB-004).

pub mod frame;
mod hyprland_mapping;
pub mod protocols;
pub mod session;

pub use frame::{Frame, Stats};
pub use session::{Capture, Capturer, Failure, Session, Toplevel};
