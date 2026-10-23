//! The daemon's view of the settings (ORB-010): the file, what was last
//! loaded from it, and what applying it produced (bind conflicts, the
//! snippet state), kept as a GPUI global so the overview and the settings
//! window read one copy. The daemon reloads the file when its modification
//! time changes.

use std::{path::PathBuf, time::SystemTime};

use orbit_core::Settings;

use crate::engage::Conflict;

#[derive(Clone, Debug)]
pub struct Prefs {
    pub settings: Settings,
    /// Where the file is (`None` without a home directory).
    pub path: Option<PathBuf>,
    /// The file's modification time when it was last read.
    pub seen: Option<SystemTime>,
    /// Why the last load failed, if it did (the previous settings stay).
    pub error: Option<String>,
    /// Other binds on the shortcut's keys.
    pub conflicts: Vec<Conflict>,
    /// Whether the live bind is in place, or why not.
    pub bind: Result<(), String>,
    pub snippet_path: Option<PathBuf>,
    pub snippet_written: Result<(), String>,
    pub hyprland_lua: Option<PathBuf>,
    /// `hyprland.lua` already requires the snippet.
    pub include_present: bool,
    /// The hot corner surfaces in place.
    pub corners: usize,
    /// Bumped on every apply, so views know to re-read.
    pub revision: u64,
}

impl gpui::Global for Prefs {}

impl Prefs {
    pub fn new(settings: Settings, path: Option<PathBuf>) -> Self {
        let seen = path.as_deref().and_then(mtime);
        Self {
            settings,
            path,
            seen,
            error: None,
            conflicts: Vec::new(),
            bind: Err("not applied yet".into()),
            snippet_path: crate::engage::snippet_path(),
            snippet_written: Err("not written yet".into()),
            hyprland_lua: crate::engage::hyprland_lua_path(),
            include_present: false,
            corners: 0,
            revision: 0,
        }
    }

    /// Reads the file again if it changed on disk. Returns true when the
    /// settings changed.
    pub fn reload_if_changed(&mut self) -> bool {
        let Some(path) = self.path.clone() else {
            return false;
        };
        let now = mtime(&path);
        if now == self.seen {
            return false;
        }
        self.seen = now;
        match Settings::load(&path) {
            Ok(settings) => {
                self.error = None;
                if settings != self.settings {
                    self.settings = settings;
                    return true;
                }
                false
            }
            Err(error) => {
                log::warn!("settings not reloaded: {error}");
                self.error = Some(error);
                false
            }
        }
    }

    /// Saves `settings` to the file and adopts it.
    pub fn save(&mut self, settings: Settings) -> Result<(), String> {
        let path = self.path.clone().ok_or("no config directory")?;
        settings.validate()?;
        settings.save(&path)?;
        self.settings = settings;
        self.seen = mtime(&path);
        self.error = None;
        Ok(())
    }

    pub fn summary_json(&self) -> serde_json::Value {
        serde_json::json!({
            "path": self.path.as_ref().map(|p| p.display().to_string()),
            "error": self.error,
            "shortcut": self.settings.shortcut.label(),
            "shortcut_lua": self.settings.shortcut.lua_keys(),
            "bind": self.bind.as_ref().err(),
            "bind_ok": self.bind.is_ok(),
            "conflicts": self.conflicts.iter().map(|c| c.description.clone()).collect::<Vec<_>>(),
            "snippet": self.snippet_path.as_ref().map(|p| p.display().to_string()),
            "snippet_ok": self.snippet_written.is_ok(),
            "include_present": self.include_present,
            "hot_corner": format!("{:?}", self.settings.hot_corner.corner),
            "corners": self.corners,
            "theme": format!("{:?}", self.settings.appearance.theme),
            "columns": self.settings.layout.columns.label(),
            "revision": self.revision,
        })
    }

    /// Save only the rail order against the latest valid file. Returns whether
    /// unrelated settings changed and also need applying to the daemon.
    pub fn save_workspace_order(&mut self, order: Vec<String>) -> Result<bool, String> {
        let path = self.path.as_ref().ok_or("no config directory")?;
        let mut settings = Settings::load(path)?;
        settings.layout.workspace_order = order.clone();
        let mut previous = self.settings.clone();
        previous.layout.workspace_order = order;
        let other_changes = settings != previous;
        self.save(settings)?;
        self.revision += 1;
        Ok(other_changes)
    }
}

fn mtime(path: &std::path::Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_order_preserves_external_edits_and_invalid_files() {
        let dir = std::env::temp_dir().join(format!("orbit-order-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        let mut settings = Settings::default();
        settings.save(&path).unwrap();
        let mut prefs = Prefs::new(settings.clone(), Some(path.clone()));
        settings.layout.special_workspaces = false;
        settings.save(&path).unwrap();
        assert!(
            prefs
                .save_workspace_order(vec!["number:2".into(), "number:1".into()])
                .unwrap()
        );
        let saved = Settings::load(&path).unwrap();
        assert!(!saved.layout.special_workspaces);
        assert_eq!(saved.layout.workspace_order, ["number:2", "number:1"]);
        assert!(
            !prefs
                .save_workspace_order(vec!["number:1".into(), "number:2".into()])
                .unwrap()
        );
        let previous = prefs.settings.clone();
        std::fs::write(&path, "[layout\ninvalid").unwrap();
        assert!(prefs.save_workspace_order(vec!["number:5".into()]).is_err());
        assert_eq!(prefs.settings, previous);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "[layout\ninvalid");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
