//! `$XDG_DATA_HOME/orbit/install.json`: how Orbit got onto this machine.
//! Only an install made by `install.sh` from a release is replaced in
//! place; a build run from a checkout is only told that a release exists.

use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InstallInfo {
    /// `release` (the installer's tarball) or `source` (a cargo build).
    pub method: String,
    /// The prefix: the binary is `PREFIX/bin/orbit`, the entry and icons
    /// under `PREFIX/share`.
    pub prefix: String,
    pub installed_version: String,
    pub installed_at: u64,
}

/// An install the updater may replace.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Install {
    pub prefix: PathBuf,
    pub binary: PathBuf,
}

impl Install {
    pub fn previous(&self) -> PathBuf {
        self.binary.with_file_name("orbit.prev")
    }

    pub fn share(&self) -> PathBuf {
        self.prefix.join("share")
    }
}

impl InstallInfo {
    /// `$XDG_DATA_HOME/orbit/install.json`, else `~/.local/share/orbit/install.json`.
    pub fn path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_DATA_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
        Some(base.join("orbit").join("install.json"))
    }

    pub fn load() -> Option<Self> {
        Self::path().and_then(|p| Self::load_from(&p))
    }

    pub fn load_from(path: &Path) -> Option<Self> {
        let text = fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = Self::path().ok_or("no data directory (HOME is unset)")?;
        self.save_to(&path)
    }

    pub fn save_to(&self, path: &Path) -> Result<(), String> {
        let dir = path.parent().ok_or("install path has no parent")?;
        fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        let text = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        fs::write(path, text).map_err(|e| format!("writing {}: {e}", path.display()))
    }

    /// Whether `exe` (the running binary) is the install this marker
    /// describes, and so may be replaced. The error says why not, in words
    /// for the settings window.
    pub fn updatable(info: Option<&InstallInfo>, exe: &Path) -> Result<Install, String> {
        let Some(info) = info else {
            return Err("not installed by install.sh: run the install line to update".into());
        };
        if info.method != "release" {
            return Err(format!(
                "installed from {}: update with git pull and cargo build",
                if info.method.is_empty() {
                    "an unknown method"
                } else {
                    &info.method
                }
            ));
        }
        if exe.components().any(|c| c.as_os_str() == "target") {
            return Err("running from a build directory, not the installed binary".into());
        }
        let prefix = PathBuf::from(&info.prefix);
        let binary = prefix.join("bin").join("orbit");
        let same = match (fs::canonicalize(exe), fs::canonicalize(&binary)) {
            (Ok(a), Ok(b)) => a == b,
            _ => exe == binary,
        };
        if !same {
            return Err(format!(
                "this binary is not the installed one ({})",
                binary.display()
            ));
        }
        Ok(Install { prefix, binary })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_release_install_of_this_very_binary_is_updatable() {
        let dir = std::env::temp_dir().join(format!("orbit-install-{}", std::process::id()));
        let bin = dir.join("bin");
        fs::create_dir_all(&bin).unwrap();
        let exe = bin.join("orbit");
        fs::write(&exe, b"#!/bin/sh\n").unwrap();
        let info = InstallInfo {
            method: "release".into(),
            prefix: dir.display().to_string(),
            installed_version: "0.1.0-alpha.1".into(),
            installed_at: 1,
        };
        let install = InstallInfo::updatable(Some(&info), &exe).unwrap();
        assert_eq!(install.binary, exe);
        assert_eq!(install.previous(), bin.join("orbit.prev"));
        assert_eq!(install.share(), dir.join("share"));
        assert!(
            InstallInfo::updatable(None, &exe)
                .unwrap_err()
                .contains("install.sh")
        );
        let source = InstallInfo {
            method: "source".into(),
            ..info.clone()
        };
        assert!(
            InstallInfo::updatable(Some(&source), &exe)
                .unwrap_err()
                .contains("cargo build")
        );
        let elsewhere = dir.join("target/release/orbit");
        assert!(
            InstallInfo::updatable(Some(&info), &elsewhere)
                .unwrap_err()
                .contains("build directory")
        );
        let other = dir.join("other-orbit");
        fs::write(&other, b"x").unwrap();
        assert!(
            InstallInfo::updatable(Some(&info), &other)
                .unwrap_err()
                .contains("not the installed one")
        );
        let path = dir.join("install.json");
        info.save_to(&path).unwrap();
        assert_eq!(InstallInfo::load_from(&path), Some(info));
        let _ = fs::remove_dir_all(dir);
    }
}
