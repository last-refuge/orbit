//! User settings (ORB-010): everything a person may want to change about
//! Orbit, with a default for each, read from and written to
//! `$XDG_CONFIG_HOME/orbit/config.toml`. Pure: file I/O is here but
//! nothing compositor- or GPUI-specific; the daemon applies the values.
//!
//! Unknown keys are kept when the file is rewritten (the document is
//! edited in place), so a newer Orbit's keys survive an older one.

use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

pub const FILE_NAME: &str = "config.toml";

/// The key combination that engages Orbit, in Hyprland's terms: modifier
/// names (`SUPER`, `SHIFT`, `CTRL`, `ALT`) and an xkb key name.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Shortcut {
    pub mods: Vec<String>,
    pub key: String,
}

/// The human label for an xkb keysym name: `grave` is shown as `` ` ``,
/// `Return` as `Enter`, `space` as `Space`. Letters are shown upper-case,
/// anything unknown as the keysym itself. Only for display: the keysym
/// stays in `config.toml` and in the `hl.bind` string.
pub fn key_label(keysym: &str) -> String {
    let named = match keysym {
        "grave" => "`",
        "Return" | "KP_Enter" => "Enter",
        "space" => "Space",
        "BackSpace" => "Backspace",
        "Escape" => "Esc",
        "Prior" => "PageUp",
        "Next" => "PageDown",
        "minus" => "-",
        "equal" => "=",
        "comma" => ",",
        "period" => ".",
        "slash" => "/",
        "backslash" => "\\",
        "semicolon" => ";",
        "apostrophe" => "'",
        "bracketleft" => "[",
        "bracketright" => "]",
        "asciitilde" => "~",
        "Up" => "\u{2191}",
        "Down" => "\u{2193}",
        "Left" => "\u{2190}",
        "Right" => "\u{2192}",
        _ => "",
    };
    if !named.is_empty() {
        return named.to_owned();
    }
    let mut chars = keysym.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if c.is_ascii_alphabetic() => c.to_ascii_uppercase().to_string(),
        _ => keysym.to_owned(),
    }
}

impl Default for Shortcut {
    fn default() -> Self {
        Self {
            mods: vec!["SUPER".into()],
            key: "grave".into(),
        }
    }
}

impl Shortcut {
    /// `SUPER+grave`, as Hyprland's Lua `hl.bind` takes it.
    pub fn lua_keys(&self) -> String {
        let mut parts: Vec<String> = self.mods.iter().map(|m| m.to_uppercase()).collect();
        parts.push(self.key.clone());
        parts.join("+")
    }

    /// `SUPER, grave`, as the `bind` keyword takes it.
    pub fn keyword_keys(&self) -> String {
        let mods: Vec<String> = self.mods.iter().map(|m| m.to_uppercase()).collect();
        format!("{}, {}", mods.join(" "), self.key)
    }

    /// Hyprland's modifier mask for the mods (SHIFT 1, CTRL 4, ALT 8,
    /// SUPER 64), to compare with `hyprctl binds`.
    pub fn modmask(&self) -> u32 {
        self.mods
            .iter()
            .map(|m| match m.to_uppercase().as_str() {
                "SHIFT" => 1,
                "CAPS" => 2,
                "CTRL" | "CONTROL" => 4,
                "ALT" => 8,
                "MOD2" => 16,
                "MOD3" => 32,
                "SUPER" | "WIN" | "LOGO" | "MOD4" => 64,
                "MOD5" => 128,
                _ => 0,
            })
            .fold(0, |a, b| a | b)
    }

    /// How people read it: `Super+Tab`, `Super+\``. The key is shown
    /// through [`key_label`]; `config.toml` and the bind keep the keysym.
    pub fn label(&self) -> String {
        let mut parts: Vec<String> = self
            .mods
            .iter()
            .map(|m| {
                let m = m.to_uppercase();
                let mut c = m.chars();
                match c.next() {
                    Some(f) => f.to_string() + &c.as_str().to_lowercase(),
                    None => String::new(),
                }
            })
            .collect();
        parts.push(key_label(&self.key));
        parts.join("+")
    }

    pub fn is_valid(&self) -> bool {
        !self.key.is_empty()
            && self
                .key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_')
            && self.mods.iter().all(|m| self.mods_known(m))
    }

    fn mods_known(&self, m: &str) -> bool {
        matches!(
            m.to_uppercase().as_str(),
            "SUPER" | "SHIFT" | "CTRL" | "CONTROL" | "ALT" | "WIN" | "LOGO" | "MOD4"
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Corner {
    #[default]
    Off,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

impl Corner {
    pub const ALL: [Corner; 5] = [
        Corner::Off,
        Corner::TopLeft,
        Corner::TopRight,
        Corner::BottomLeft,
        Corner::BottomRight,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Corner::Off => "Off",
            Corner::TopLeft => "Top left",
            Corner::TopRight => "Top right",
            Corner::BottomLeft => "Bottom left",
            Corner::BottomRight => "Bottom right",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Monitors {
    #[default]
    Primary,
    All,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct HotCorner {
    pub corner: Corner,
    /// How long the pointer must rest in the corner.
    pub dwell_ms: u32,
    /// The corner also closes an open overview.
    pub corner_closes: bool,
    pub monitors: Monitors,
}

impl Default for HotCorner {
    fn default() -> Self {
        Self {
            corner: Corner::Off,
            dwell_ms: 150,
            corner_closes: true,
            monitors: Monitors::Primary,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum ThemeChoice {
    /// Frosted glass when the desktop can be captured, else solid.
    #[default]
    Auto,
    Glass,
    Solid,
    HighContrast,
}

impl ThemeChoice {
    pub const ALL: [ThemeChoice; 4] = [
        ThemeChoice::Auto,
        ThemeChoice::Glass,
        ThemeChoice::Solid,
        ThemeChoice::HighContrast,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ThemeChoice::Auto => "Auto",
            ThemeChoice::Glass => "Glass",
            ThemeChoice::Solid => "Solid",
            ThemeChoice::HighContrast => "High contrast",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemeChoice,
    /// Backdrop blur radius per pass, in blurred-image pixels (1–20).
    pub blur: u32,
    /// Tint strength, 80–120 % of the theme's alphas (under 100 % the
    /// text contrast guarantee over a bright wallpaper no longer holds).
    pub tint_percent: u32,
    pub reduced_motion: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::Auto,
            blur: 7,
            tint_percent: 100,
            reduced_motion: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Columns {
    #[default]
    Auto,
    Two,
    Three,
    Four,
}

impl Columns {
    pub const ALL: [Columns; 4] = [Columns::Auto, Columns::Two, Columns::Three, Columns::Four];

    pub fn label(self) -> &'static str {
        match self {
            Columns::Auto => "Auto",
            Columns::Two => "2",
            Columns::Three => "3",
            Columns::Four => "4",
        }
    }

    /// The fixed count, or `None` for auto (three; two for one or two
    /// windows).
    pub fn fixed(self) -> Option<usize> {
        match self {
            Columns::Auto => None,
            Columns::Two => Some(2),
            Columns::Three => Some(3),
            Columns::Four => Some(4),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Titles {
    Always,
    #[default]
    Selected,
    Never,
}

impl Titles {
    pub const ALL: [Titles; 3] = [Titles::Always, Titles::Selected, Titles::Never];

    pub fn label(self) -> &'static str {
        match self {
            Titles::Always => "Always",
            Titles::Selected => "Selected card",
            Titles::Never => "Never",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum RailPosition {
    #[default]
    Top,
    Bottom,
}

impl RailPosition {
    pub const ALL: [RailPosition; 2] = [RailPosition::Top, RailPosition::Bottom];

    pub fn label(self) -> &'static str {
        match self {
            RailPosition::Top => "Top",
            RailPosition::Bottom => "Bottom",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Layout {
    pub columns: Columns,
    pub titles: Titles,
    pub special_workspaces: bool,
    pub rail: RailPosition,
}

impl Default for Layout {
    fn default() -> Self {
        Self {
            columns: Columns::Auto,
            titles: Titles::Selected,
            special_workspaces: true,
            rail: RailPosition::Top,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum SearchScope {
    #[default]
    All,
    Workspace,
}

impl SearchScope {
    pub const ALL: [SearchScope; 2] = [SearchScope::All, SearchScope::Workspace];

    pub fn label(self) -> &'static str {
        match self {
            SearchScope::All => "All workspaces",
            SearchScope::Workspace => "This workspace",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Behaviour {
    /// Plain drop or Enter moves and follows (Shift does the opposite).
    pub follow_on_move: bool,
    /// Opening a window closes the overview.
    pub close_on_activate: bool,
    pub search_scope: SearchScope,
    /// Pressing the shortcut twice quickly switches to the last window
    /// instead of opening the overview.
    pub double_tap_last_window: bool,
}

impl Default for Behaviour {
    fn default() -> Self {
        Self {
            follow_on_move: false,
            close_on_activate: true,
            search_scope: SearchScope::All,
            double_tap_last_window: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Previews {
    /// How often visible previews refresh.
    pub refresh_ms: u32,
    /// Also capture the windows of workspaces that are not previewed, so
    /// every rail tile shows real content (more capture work, and every
    /// window is read as soon as the overview opens).
    pub capture_inactive_workspaces: bool,
    /// Window classes never captured (previews show the app icon).
    pub never_capture: Vec<String>,
}

impl Default for Previews {
    fn default() -> Self {
        Self {
            refresh_ms: 400,
            capture_inactive_workspaces: false,
            never_capture: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Icons {
    /// Icon theme name; empty means the desktop's setting.
    pub theme: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Startup {
    /// A systemd user unit starts `orbit daemon` at login.
    pub launch_at_login: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    pub shortcut: Shortcut,
    pub hot_corner: HotCorner,
    pub appearance: Appearance,
    pub layout: Layout,
    pub behaviour: Behaviour,
    pub previews: Previews,
    pub icons: Icons,
    pub startup: Startup,
}

impl Settings {
    /// `$XDG_CONFIG_HOME/orbit/config.toml` (or `~/.config/orbit/…`);
    /// `$ORBIT_CONFIG` names another file (scripted checks use it so they
    /// never touch the person's settings).
    pub fn path() -> Option<PathBuf> {
        if let Some(path) = std::env::var_os("ORBIT_CONFIG").filter(|v| !v.is_empty()) {
            return Some(PathBuf::from(path));
        }
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(config.join("orbit").join(FILE_NAME))
    }

    /// Reads the file; defaults when it is missing. A file that does not
    /// parse is an error (the daemon then keeps the last good settings).
    pub fn load(path: &Path) -> Result<Self, String> {
        match fs::read_to_string(path) {
            Ok(text) => Self::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(format!("reading {}: {e}", path.display())),
        }
    }

    pub fn parse(text: &str) -> Result<Self, String> {
        let settings: Settings = toml::from_str(text).map_err(|e| e.to_string())?;
        settings.validate()?;
        Ok(settings)
    }

    /// Clamps and checks values; an error names the first bad one.
    pub fn validate(&self) -> Result<(), String> {
        if !self.shortcut.is_valid() {
            return Err(
                "shortcut: mods must be SUPER, SHIFT, CTRL or ALT and the key an xkb name".into(),
            );
        }
        if !(1..=20).contains(&self.appearance.blur) {
            return Err("appearance.blur must be 1–20".into());
        }
        if !(80..=120).contains(&self.appearance.tint_percent) {
            return Err("appearance.tint_percent must be 80–120".into());
        }
        if !(50..=2000).contains(&self.hot_corner.dwell_ms) {
            return Err("hot_corner.dwell_ms must be 50–2000".into());
        }
        if !(100..=5000).contains(&self.previews.refresh_ms) {
            return Err("previews.refresh_ms must be 100–5000".into());
        }
        Ok(())
    }

    /// Serializes with every key present (documented defaults included).
    pub fn to_toml(&self) -> String {
        let mut text = String::from(
            "# Orbit settings. Every key has a default; delete a key to get it back.\n\
             # Edited by the Orbit settings window and `orbit config`; hot-reloaded.\n\n",
        );
        text.push_str(&toml::to_string_pretty(self).unwrap_or_default());
        text
    }

    /// Writes atomically (temp file in the same directory, then rename),
    /// keeping keys this version does not know from the existing file.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        let dir = path.parent().ok_or("config path has no parent")?;
        fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
        let text = match fs::read_to_string(path) {
            Ok(existing) => merge_unknown(&existing, &self.to_toml()),
            Err(_) => self.to_toml(),
        };
        let temp = dir.join(format!(".{FILE_NAME}.{}.tmp", std::process::id()));
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

    /// One value by dotted key (`hot_corner.dwell_ms`), as TOML text.
    pub fn get(&self, key: &str) -> Result<String, String> {
        let doc: toml::Value = toml::from_str(&toml::to_string(self).unwrap_or_default())
            .map_err(|e| e.to_string())?;
        let mut at = &doc;
        for part in key.split('.') {
            at = at.get(part).ok_or_else(|| format!("no setting `{key}`"))?;
        }
        Ok(match at {
            toml::Value::String(s) => s.clone(),
            other => other.to_string(),
        })
    }

    /// Sets one value by dotted key from its TOML text (`true`, `150`,
    /// `"top-left"` or bare `top-left`), validating the result.
    pub fn set(&self, key: &str, value: &str) -> Result<Self, String> {
        let mut doc: toml::Value = toml::from_str(&toml::to_string(self).unwrap_or_default())
            .map_err(|e| e.to_string())?;
        let parts: Vec<&str> = key.split('.').collect();
        let (last, parents) = parts.split_last().ok_or("empty key")?;
        let mut at = &mut doc;
        for part in parents {
            at = at
                .get_mut(*part)
                .ok_or_else(|| format!("no setting `{key}`"))?;
        }
        let table = at
            .as_table_mut()
            .ok_or_else(|| format!("no setting `{key}`"))?;
        let current = table
            .get(*last)
            .ok_or_else(|| format!("no setting `{key}`"))?;
        let parsed: toml::Value = match current {
            toml::Value::String(_) => {
                toml::Value::String(value.trim().trim_matches('"').to_owned())
            }
            toml::Value::Array(_) => toml::from_str::<toml::Value>(&format!("v = {value}"))
                .ok()
                .and_then(|v| v.get("v").cloned())
                .or_else(|| {
                    Some(toml::Value::Array(
                        value
                            .split(',')
                            .map(|s| toml::Value::String(s.trim().to_owned()))
                            .filter(|s| s.as_str().is_some_and(|s| !s.is_empty()))
                            .collect(),
                    ))
                })
                .ok_or("bad list")?,
            _ => toml::from_str::<toml::Value>(&format!("v = {value}"))
                .map_err(|e| format!("bad value: {e}"))?
                .get("v")
                .cloned()
                .ok_or("bad value")?,
        };
        table.insert((*last).to_owned(), parsed);
        let text = toml::to_string(&doc).map_err(|e| e.to_string())?;
        Self::parse(&text)
    }

    /// Every dotted key with its current value, for `orbit config list`.
    pub fn entries(&self) -> Vec<(String, String)> {
        let doc: toml::Value = toml::from_str(&toml::to_string(self).unwrap_or_default())
            .unwrap_or(toml::Value::Table(Default::default()));
        let mut out = Vec::new();
        fn walk(prefix: &str, value: &toml::Value, out: &mut Vec<(String, String)>) {
            match value {
                toml::Value::Table(table) => {
                    for (k, v) in table {
                        let key = if prefix.is_empty() {
                            k.clone()
                        } else {
                            format!("{prefix}.{k}")
                        };
                        walk(&key, v, out);
                    }
                }
                toml::Value::String(s) => out.push((prefix.to_owned(), s.clone())),
                other => out.push((prefix.to_owned(), other.to_string())),
            }
        }
        walk("", &doc, &mut out);
        out
    }
}

/// Keeps tables and keys of `existing` that `fresh` does not have (another
/// version's settings), on top of `fresh`.
fn merge_unknown(existing: &str, fresh: &str) -> String {
    let (Ok(mut old), Ok(new)) = (
        toml::from_str::<toml::Value>(existing),
        toml::from_str::<toml::Value>(fresh),
    ) else {
        return fresh.to_owned();
    };
    fn overlay(old: &mut toml::Value, new: &toml::Value) {
        match (old, new) {
            (toml::Value::Table(old), toml::Value::Table(new)) => {
                for (k, v) in new {
                    match old.get_mut(k) {
                        Some(slot) if slot.is_table() && v.is_table() => overlay(slot, v),
                        _ => {
                            old.insert(k.clone(), v.clone());
                        }
                    }
                }
            }
            (old, new) => *old = new.clone(),
        }
    }
    overlay(&mut old, &new);
    let header = fresh
        .lines()
        .take_while(|l| l.starts_with('#') || l.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "{header}\n{}",
        toml::to_string_pretty(&old).unwrap_or_else(|_| fresh.to_owned())
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_round_trip_and_unknown_keys_survive() {
        let s = Settings::default();
        let text = s.to_toml();
        assert_eq!(Settings::parse(&text).unwrap(), s);
        let with_extra = format!("{text}\n[future]\nthing = 1\n");
        let merged = merge_unknown(&with_extra, &text);
        assert!(merged.contains("[future]") && merged.contains("thing = 1"));
        assert_eq!(Settings::parse(&merged).unwrap(), s);
        assert_eq!(Settings::parse("").unwrap(), s);
        assert_eq!(
            Settings::parse("[hot_corner]\ncorner = \"top-right\"\n")
                .unwrap()
                .hot_corner
                .corner,
            Corner::TopRight
        );
    }

    #[test]
    fn get_and_set_by_dotted_key() {
        let s = Settings::default();
        assert_eq!(s.get("hot_corner.dwell_ms").unwrap(), "150");
        assert_eq!(s.get("shortcut.key").unwrap(), "grave");
        let t = s.set("hot_corner.corner", "top-left").unwrap();
        assert_eq!(t.hot_corner.corner, Corner::TopLeft);
        let t = t.set("appearance.blur", "12").unwrap();
        assert_eq!(t.appearance.blur, 12);
        let t = t.set("previews.never_capture", "a, b").unwrap();
        assert_eq!(t.previews.never_capture, ["a", "b"]);
        let t = t.set("shortcut.mods", "[\"SUPER\", \"SHIFT\"]").unwrap();
        assert_eq!(t.shortcut.modmask(), 65);
        assert!(s.set("appearance.blur", "99").is_err());
        assert!(s.set("nope.key", "1").is_err());
        assert!(s.set("hot_corner.corner", "middle").is_err());
        assert!(s.entries().iter().any(|(k, _)| k == "layout.columns"));
    }

    #[test]
    fn shortcuts_render_for_hyprland() {
        let s = Shortcut::default();
        assert_eq!(s.lua_keys(), "SUPER+grave");
        assert_eq!(s.keyword_keys(), "SUPER, grave");
        assert_eq!(s.modmask(), 64);
        assert_eq!(s.label(), "Super+`");
        let tab = Shortcut {
            mods: vec!["SUPER".into()],
            key: "Tab".into(),
        };
        assert_eq!(tab.lua_keys(), "SUPER+Tab");
        assert_eq!(tab.label(), "Super+Tab");
        let backtick = Shortcut {
            mods: vec!["SUPER".into(), "SHIFT".into()],
            key: "grave".into(),
        };
        assert_eq!(backtick.label(), "Super+Shift+`");
        assert_eq!(
            backtick.lua_keys(),
            "SUPER+SHIFT+grave",
            "the bind keeps the keysym"
        );
        let bad = Shortcut {
            mods: vec!["HYPER".into()],
            key: "a".into(),
        };
        assert!(!bad.is_valid());
    }

    #[test]
    fn keysyms_get_human_labels() {
        assert_eq!(key_label("grave"), "`");
        assert_eq!(key_label("Tab"), "Tab");
        assert_eq!(key_label("Return"), "Enter");
        assert_eq!(key_label("space"), "Space");
        assert_eq!(key_label("Escape"), "Esc");
        assert_eq!(key_label("Prior"), "PageUp");
        assert_eq!(key_label("backslash"), "\\");
        assert_eq!(key_label("a"), "A");
        assert_eq!(key_label("F12"), "F12");
        assert_eq!(key_label("XF86AudioPlay"), "XF86AudioPlay");
    }

    #[test]
    fn save_is_atomic_and_readable() {
        let dir = std::env::temp_dir().join(format!("orbit-settings-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        let mut s = Settings::default();
        s.appearance.blur = 9;
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap().appearance.blur, 9);
        assert!(
            fs::read_dir(&dir).unwrap().count() == 1,
            "no temp file left"
        );
        let _ = fs::remove_dir_all(dir);
    }
}
