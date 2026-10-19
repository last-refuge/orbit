//! `$XDG_STATE_HOME/orbit/update.json`: what the last check found, what
//! the person skipped, what was applied, and the back-off after errors.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use crate::release::Candidate;

/// The newest release the last check found.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Latest {
    pub version: String,
    pub tag: String,
    pub html_url: String,
    pub tarball_url: String,
    pub sha256_url: String,
    pub signature_url: Option<String>,
    pub published_at: String,
    pub notes: String,
}

impl From<&Candidate> for Latest {
    fn from(c: &Candidate) -> Self {
        Self {
            version: c.version.to_string(),
            tag: c.tag.clone(),
            html_url: c.html_url.clone(),
            tarball_url: c.tarball.url.clone(),
            sha256_url: c.sha256.url.clone(),
            signature_url: c.signature.as_ref().map(|a| a.url.clone()),
            published_at: c.published_at.clone(),
            notes: c.notes.clone(),
        }
    }
}

/// An update that was installed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Applied {
    pub version: String,
    pub from: String,
    pub at: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdateState {
    /// When the releases list was last fetched (Unix seconds).
    pub last_check: Option<u64>,
    /// The ETag of that list, for conditional requests.
    pub etag: Option<String>,
    pub latest: Option<Latest>,
    /// A version the person chose to skip; offered again only when a newer
    /// one appears.
    pub skipped: Option<String>,
    /// A version that failed to start after an update and was rolled back.
    pub failed: Option<String>,
    /// The last update that went in.
    pub applied: Option<Applied>,
    /// Not before this time (Unix seconds) after rate limiting or errors.
    pub backoff_until: Option<u64>,
    pub failures: u32,
    /// Daemon starts since the last update, for the crash-loop guard.
    pub starts_since_update: u32,
    /// The automatic mode was offered once after a manual update.
    pub auto_offered: bool,
}

impl UpdateState {
    /// `$XDG_STATE_HOME/orbit/update.json`, else `~/.local/state/orbit/update.json`.
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_STATE_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/state")))?;
        Some(base.join("orbit").join("update.json"))
    }

    pub fn load() -> Self {
        Self::path()
            .map(|p| Self::load_from(&p))
            .unwrap_or_default()
    }

    pub fn load_from(path: &Path) -> Self {
        fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no state directory (HOME is unset)")?;
        self.save_to(&path)
    }

    /// Atomic: a temp file beside the target, then a rename.
    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let dir = path.parent().ok_or("state path has no parent")?;
        fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        let temp = dir.join(format!(".update.json.{}.tmp", std::process::id()));
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        {
            let mut file =
                fs::File::create(&temp).map_err(|e| format!("writing {}: {e}", temp.display()))?;
            file.write_all(text.as_bytes())
                .and_then(|_| file.sync_all())
                .map_err(|e| format!("writing {}: {e}", temp.display()))?;
        }
        fs::rename(&temp, path).map_err(|e| {
            let _ = fs::remove_file(&temp);
            format!("replacing {}: {e}", path.display())
        })
    }

    /// The newest known release when it is newer than `current` and not
    /// the skipped one.
    pub fn available(&self, current: &semver::Version) -> Option<&Latest> {
        let latest = self.latest.as_ref()?;
        let version = semver::Version::parse(&latest.version).ok()?;
        if version <= *current {
            return None;
        }
        if self.skipped.as_deref() == Some(latest.version.as_str())
            || self.failed.as_deref() == Some(latest.version.as_str())
        {
            return None;
        }
        Some(latest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_knows_what_is_available() {
        let dir = std::env::temp_dir().join(format!("orbit-update-state-{}", std::process::id()));
        let path = dir.join("update.json");
        let mut state = UpdateState {
            last_check: Some(1),
            etag: Some("\"abc\"".into()),
            latest: Some(Latest {
                version: "0.1.0-alpha.2".into(),
                tag: "v0.1.0-alpha.2".into(),
                ..Latest::default()
            }),
            ..UpdateState::default()
        };
        state.save_to(&path).unwrap();
        assert_eq!(UpdateState::load_from(&path), state);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "no temp file left");
        let current = semver::Version::parse("0.1.0-alpha.1").unwrap();
        assert_eq!(
            state.available(&current).map(|l| l.tag.as_str()),
            Some("v0.1.0-alpha.2")
        );
        let newer = semver::Version::parse("0.1.0-alpha.2").unwrap();
        assert!(
            state.available(&newer).is_none(),
            "not newer than what runs"
        );
        state.skipped = Some("0.1.0-alpha.2".into());
        assert!(state.available(&current).is_none(), "skipped");
        state.skipped = None;
        state.failed = Some("0.1.0-alpha.2".into());
        assert!(
            state.available(&current).is_none(),
            "failed and rolled back: never again"
        );
        assert_eq!(
            UpdateState::load_from(&dir.join("missing.json")),
            UpdateState::default()
        );
        let _ = fs::remove_dir_all(dir);
    }
}
