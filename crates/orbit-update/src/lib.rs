//! Staying up to date (ORB-011): finding Orbit's releases on GitHub,
//! deciding whether one is newer than the running build, and replacing the
//! installed binary in place with a way back.
//!
//! No UI here. The daemon schedules checks and shows the result; the
//! `orbit update` commands call the same functions from the command line.
//! The network goes through `curl` as a subprocess (the installer already
//! needs it), HTTPS only, to three GitHub hosts; a loopback `http://` base
//! is allowed so the checks can run against a local fixture server.

pub mod apply;
pub mod check;
pub mod client;
pub mod install;
pub mod release;
pub mod state;

pub use apply::{Applied, apply, rollback};
pub use check::{Checker, Outcome};
pub use client::Http;
pub use install::{Install, InstallInfo};
pub use release::{Candidate, Channel, Release, select};
pub use state::{Latest, UpdateState};

/// The GitHub repository releases come from.
pub const REPO: &str = "last-refuge/orbit";
/// The releases API for [`REPO`]; `ORBIT_API_BASE` overrides it.
pub const API_BASE: &str = "https://api.github.com/repos/last-refuge/orbit";

/// This machine's architecture as the release assets name it.
pub fn arch() -> &'static str {
    std::env::consts::ARCH
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// The API base: `ORBIT_API_BASE` when set (tests), else GitHub.
pub fn api_base() -> String {
    std::env::var("ORBIT_API_BASE")
        .ok()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| API_BASE.to_owned())
}
