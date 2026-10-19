//! Real app icons for cards and rail tiles (ORB-008 P0-4).
//!
//! A window's class (its Wayland app id, or X11 class) is matched to an XDG
//! desktop entry, whose `Icon=` is looked up in the icon theme (and the
//! themes it inherits, then `hicolor`, then `pixmaps`), following the
//! freedesktop Desktop Entry and Icon Theme specifications closely enough
//! for installed applications. Everything is read from disk once and
//! cached per class; a class with no entry or no icon file gets `None`, and
//! the overview draws its letter tile instead.
//!
//! No process is spawned here and nothing is written. Entry names are read
//! only to match classes; they are never logged.

use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use gpui::{Image, ImageFormat};

/// The size icons are rendered at (logical px); files are picked for it.
pub const ICON_SIZE: u32 = 36;
/// What to ask the theme for: the next common size up, so the file is
/// downscaled once at 1x and barely at 1.25x–1.5x, never upscaled.
const WANTED_PX: u32 = 48;

/// One desktop entry's matching fields.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Entry {
    /// The file name without `.desktop` (the desktop file id).
    stem: String,
    name: String,
    icon: String,
    wm_class: Option<String>,
}

/// One directory of an icon theme, from its `index.theme`.
#[derive(Clone, Debug)]
struct ThemeDir {
    path: String,
    size: u32,
    scale: u32,
    kind: DirKind,
    min: u32,
    max: u32,
    threshold: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DirKind {
    Fixed,
    Scalable,
    Threshold,
}

impl ThemeDir {
    /// Distance from the wanted size, after the spec's matching rules;
    /// `None` when the directory cannot serve that size at all.
    fn distance(&self, wanted: u32) -> Option<u32> {
        let size = |n: u32| n * self.scale;
        match self.kind {
            DirKind::Fixed => Some(size(self.size).abs_diff(wanted)),
            DirKind::Scalable => {
                if wanted < size(self.min) {
                    Some(size(self.min) - wanted)
                } else if wanted > size(self.max) {
                    Some(wanted - size(self.max))
                } else {
                    Some(0)
                }
            }
            DirKind::Threshold => {
                let (lo, hi) = (
                    size(self.size.saturating_sub(self.threshold)),
                    size(self.size + self.threshold),
                );
                if wanted < lo {
                    Some(lo - wanted)
                } else if wanted > hi {
                    Some(wanted - hi)
                } else {
                    Some(0)
                }
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
struct IconTheme {
    inherits: Vec<String>,
    dirs: Vec<ThemeDir>,
}

/// The icon lookup: data directories, the theme name, lazily built
/// indexes and a per-class cache of decoded-on-demand images.
pub struct Icons {
    theme: String,
    data_dirs: Vec<PathBuf>,
    entries: OnceLock<Vec<Entry>>,
    themes: Mutex<HashMap<String, Option<IconTheme>>>,
    cache: Mutex<HashMap<String, Option<Arc<Image>>>>,
}

impl std::fmt::Debug for Icons {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Icons")
            .field("theme", &self.theme)
            .field("data_dirs", &self.data_dirs)
            .finish()
    }
}

impl Icons {
    /// `theme` is the icon theme name (`hicolor` when `None`); `data_dirs`
    /// are the XDG data directories to search, most specific first.
    pub fn new(theme: Option<String>, data_dirs: Vec<PathBuf>) -> Self {
        Self {
            theme: theme
                .filter(|t| !t.is_empty() && !t.contains('/'))
                .unwrap_or_else(|| "hicolor".into()),
            data_dirs,
            entries: OnceLock::new(),
            themes: Mutex::new(HashMap::new()),
            cache: Mutex::new(HashMap::new()),
        }
    }

    /// The XDG data directories of this session: `$XDG_DATA_HOME` (or
    /// `~/.local/share`), then `$XDG_DATA_DIRS` (or the two system
    /// defaults), plus the Flatpak exports if present.
    pub fn xdg_data_dirs() -> Vec<PathBuf> {
        let mut dirs = Vec::new();
        match std::env::var_os("XDG_DATA_HOME").filter(|v| !v.is_empty()) {
            Some(home) => dirs.push(PathBuf::from(home)),
            None => {
                if let Some(home) = std::env::var_os("HOME") {
                    dirs.push(PathBuf::from(home).join(".local/share"));
                }
            }
        }
        if let Some(home) = std::env::var_os("HOME") {
            dirs.push(PathBuf::from(&home).join(".local/share/flatpak/exports/share"));
        }
        let system = std::env::var("XDG_DATA_DIRS")
            .ok()
            .filter(|v| !v.is_empty())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into());
        dirs.extend(
            system
                .split(':')
                .filter(|d| !d.is_empty())
                .map(PathBuf::from),
        );
        dirs.push(PathBuf::from("/var/lib/flatpak/exports/share"));
        dirs
    }

    /// The icon theme name from GTK's settings files (`gtk-icon-theme-name`),
    /// or `None` when they do not say.
    pub fn theme_from_gtk_settings() -> Option<String> {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"]
            .iter()
            .filter_map(|rel| fs::read_to_string(config.join(rel)).ok())
            .find_map(|text| {
                text.lines().find_map(|line| {
                    let (key, value) = line.split_once('=')?;
                    (key.trim() == "gtk-icon-theme-name")
                        .then(|| value.trim().trim_matches('"').to_owned())
                        .filter(|v| !v.is_empty())
                })
            })
    }

    pub fn theme_name(&self) -> &str {
        &self.theme
    }

    /// The icon image for a window class, if an installed application
    /// claims the class and its icon exists. Cached per class.
    pub fn icon_for(&self, class: &str) -> Option<Arc<Image>> {
        if class.is_empty() {
            return None;
        }
        if let Some(hit) = self.cache.lock().ok()?.get(class) {
            return hit.clone();
        }
        let image = self
            .resolve(class)
            .and_then(|path| load(&path).map(Arc::new));
        if let Ok(mut cache) = self.cache.lock() {
            cache.insert(class.to_owned(), image.clone());
        }
        image
    }

    /// True once `class` has been looked up, whether or not it has an icon.
    pub fn has_icon(&self, class: &str) -> bool {
        self.icon_for(class).is_some()
    }

    /// The icon file for a class: desktop entry, then theme lookup.
    pub fn resolve(&self, class: &str) -> Option<PathBuf> {
        let entry = self.entry_for(class)?;
        self.icon_file(&entry.icon)
    }

    fn entries(&self) -> &[Entry] {
        self.entries.get_or_init(|| {
            let mut entries = Vec::new();
            for dir in &self.data_dirs {
                scan_applications(&dir.join("applications"), 0, &mut entries);
            }
            entries
        })
    }

    /// Matches a class to an entry the way desktop environments do: by
    /// desktop file id, then `StartupWMClass`, then the reverse-DNS last
    /// segment, then the visible name. Case-insensitive.
    fn entry_for(&self, class: &str) -> Option<Entry> {
        let entries = self.entries();
        let eq = |a: &str, b: &str| a.eq_ignore_ascii_case(b);
        let last = |s: &str| {
            s.rsplit('.')
                .find(|p| !p.is_empty())
                .unwrap_or(s)
                .to_owned()
        };
        let class_last = last(class);
        entries
            .iter()
            .find(|e| e.stem == class)
            .or_else(|| entries.iter().find(|e| eq(&e.stem, class)))
            .or_else(|| {
                entries
                    .iter()
                    .find(|e| e.wm_class.as_deref().is_some_and(|w| eq(w, class)))
            })
            .or_else(|| {
                entries.iter().find(|e| {
                    eq(&last(&e.stem), &class_last)
                        || e.wm_class
                            .as_deref()
                            .is_some_and(|w| eq(&last(w), &class_last))
                })
            })
            .or_else(|| entries.iter().find(|e| eq(&e.name, class)))
            .cloned()
    }

    /// Looks `icon` up: an absolute path as is, else through the theme
    /// chain, else `pixmaps`.
    fn icon_file(&self, icon: &str) -> Option<PathBuf> {
        if icon.is_empty() {
            return None;
        }
        let path = Path::new(icon);
        if path.is_absolute() {
            return path.is_file().then(|| path.to_owned());
        }
        // The spec says bare names; tolerate a trailing extension.
        let name = icon
            .strip_suffix(".png")
            .or_else(|| icon.strip_suffix(".svg"))
            .unwrap_or(icon);
        let mut seen = Vec::new();
        let mut queue = vec![self.theme.clone()];
        let mut best: Option<(u32, PathBuf)> = None;
        while let Some(theme) = queue.pop() {
            if seen.contains(&theme) {
                continue;
            }
            seen.push(theme.clone());
            let Some(index) = self.theme_index(&theme) else {
                continue;
            };
            for dir in &index.dirs {
                let Some(distance) = dir.distance(WANTED_PX) else {
                    continue;
                };
                if best.as_ref().is_some_and(|(d, _)| *d <= distance) {
                    continue;
                }
                for data in &self.data_dirs {
                    let base = data.join("icons").join(&theme).join(&dir.path);
                    for ext in ["png", "svg"] {
                        let file = base.join(format!("{name}.{ext}"));
                        if file.is_file() {
                            best = Some((distance, file));
                            break;
                        }
                    }
                    if best.as_ref().is_some_and(|(d, _)| *d == distance) {
                        break;
                    }
                }
            }
            if best.as_ref().is_some_and(|(d, _)| *d == 0) {
                return best.map(|(_, p)| p);
            }
            // Parents are searched after the theme itself, in order.
            for parent in index.inherits.iter().rev() {
                queue.push(parent.clone());
            }
            if queue.is_empty() && !seen.iter().any(|t| t == "hicolor") {
                queue.push("hicolor".into());
            }
        }
        if let Some((_, path)) = best {
            return Some(path);
        }
        for data in &self.data_dirs {
            for ext in ["png", "svg"] {
                let file = data.join("pixmaps").join(format!("{name}.{ext}"));
                if file.is_file() {
                    return Some(file);
                }
            }
        }
        None
    }

    fn theme_index(&self, theme: &str) -> Option<IconTheme> {
        if let Some(cached) = self.themes.lock().ok()?.get(theme) {
            return cached.clone();
        }
        let index = self
            .data_dirs
            .iter()
            .map(|data| data.join("icons").join(theme).join("index.theme"))
            .find_map(|path| fs::read_to_string(path).ok())
            .map(|text| parse_index_theme(&text));
        if let Ok(mut themes) = self.themes.lock() {
            themes.insert(theme.to_owned(), index.clone());
        }
        index
    }
}

/// Reads an icon file into a GPUI image (decoded lazily when drawn).
fn load(path: &Path) -> Option<Image> {
    let format = match path.extension()?.to_str()? {
        "png" => ImageFormat::Png,
        "svg" => ImageFormat::Svg,
        _ => return None,
    };
    let bytes = fs::read(path).ok()?;
    Some(Image::from_bytes(format, bytes))
}

/// Collects application entries under `dir` (and one level of
/// subdirectories, as the spec allows), skipping hidden ones.
fn scan_applications(dir: &Path, depth: u8, out: &mut Vec<Entry>) {
    let Ok(read) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            if depth == 0 {
                scan_applications(&path, 1, out);
            }
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
            continue;
        }
        let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
            continue;
        };
        let Ok(text) = fs::read_to_string(&path) else {
            continue;
        };
        if let Some(entry) = parse_desktop_entry(stem, &text)
            && !out.iter().any(|e| e.stem == entry.stem)
        {
            out.push(entry);
        }
    }
}

/// The matching fields of a `[Desktop Entry]` group; `None` for entries
/// that are not applications or are hidden.
fn parse_desktop_entry(stem: &str, text: &str) -> Option<Entry> {
    let mut in_group = false;
    let mut name = None;
    let mut icon = None;
    let mut wm_class = None;
    let mut kind = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        if !in_group || line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "Type" => kind = Some(value.to_owned()),
            "Name" => name = Some(value.to_owned()),
            "Icon" => icon = Some(value.to_owned()),
            "StartupWMClass" => wm_class = Some(value.to_owned()).filter(|v| !v.is_empty()),
            "NoDisplay" | "Hidden" if value.eq_ignore_ascii_case("true") => return None,
            _ => {}
        }
    }
    if kind.as_deref() != Some("Application") {
        return None;
    }
    Some(Entry {
        stem: stem.to_owned(),
        name: name.unwrap_or_default(),
        icon: icon?,
        wm_class,
    })
}

fn parse_index_theme(text: &str) -> IconTheme {
    let mut theme = IconTheme::default();
    let mut group: Option<String> = None;
    let mut current: Option<ThemeDir> = None;
    let mut listed: Vec<String> = Vec::new();
    let finish = |current: &mut Option<ThemeDir>, theme: &mut IconTheme| {
        if let Some(dir) = current.take() {
            theme.dirs.push(dir);
        }
    };
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            finish(&mut current, &mut theme);
            group = Some(name.to_owned());
            if name != "Icon Theme" {
                current = Some(ThemeDir {
                    path: name.to_owned(),
                    size: 0,
                    scale: 1,
                    kind: DirKind::Threshold,
                    min: 0,
                    max: 0,
                    threshold: 2,
                });
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        match (group.as_deref(), &mut current) {
            (Some("Icon Theme"), _) => match key {
                "Inherits" => {
                    theme.inherits = value
                        .split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty())
                        .collect()
                }
                "Directories" | "ScaledDirectories" => listed.extend(
                    value
                        .split(',')
                        .map(|s| s.trim().to_owned())
                        .filter(|s| !s.is_empty()),
                ),
                _ => {}
            },
            (_, Some(dir)) => match key {
                "Size" => dir.size = value.parse().unwrap_or(0),
                "Scale" => dir.scale = value.parse().unwrap_or(1).max(1),
                "MinSize" => dir.min = value.parse().unwrap_or(0),
                "MaxSize" => dir.max = value.parse().unwrap_or(0),
                "Threshold" => dir.threshold = value.parse().unwrap_or(2),
                "Type" => {
                    dir.kind = match value {
                        "Fixed" => DirKind::Fixed,
                        "Scalable" => DirKind::Scalable,
                        _ => DirKind::Threshold,
                    }
                }
                _ => {}
            },
            _ => {}
        }
    }
    finish(&mut current, &mut theme);
    for dir in &mut theme.dirs {
        if dir.min == 0 {
            dir.min = dir.size;
        }
        if dir.max == 0 {
            dir.max = dir.size;
        }
    }
    // Only directories the theme lists count, and with a known size.
    theme
        .dirs
        .retain(|d| d.size > 0 && (listed.is_empty() || listed.contains(&d.path)));
    theme
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "orbit-icons-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// A data directory with a theme that inherits hicolor, two apps and
    /// icons at a few sizes.
    fn fixture() -> PathBuf {
        let data = temp_dir("data");
        write(
            &data.join("applications/org.example.Editor.desktop"),
            "[Desktop Entry]\nType=Application\nName=Editor\nIcon=editor\nStartupWMClass=EditorMain\n",
        );
        write(
            &data.join("applications/web/browser.desktop"),
            "[Desktop Entry]\nType=Application\nName=Browser\nIcon=browser-icon\n",
        );
        write(
            &data.join("applications/ghost.desktop"),
            "[Desktop Entry]\nType=Application\nName=Ghost\nIcon=editor\nNoDisplay=true\n",
        );
        write(
            &data.join("applications/link.desktop"),
            "[Desktop Entry]\nType=Link\nName=Editor\nIcon=editor\n",
        );
        write(
            &data.join("icons/Mine/index.theme"),
            "[Icon Theme]\nName=Mine\nInherits=hicolor\nDirectories=48x48/apps,scalable/apps\n\n\
             [48x48/apps]\nSize=48\nType=Fixed\n\n[scalable/apps]\nSize=128\nMinSize=16\nMaxSize=512\nType=Scalable\n",
        );
        write(&data.join("icons/Mine/scalable/apps/editor.svg"), "<svg/>");
        write(
            &data.join("icons/hicolor/index.theme"),
            "[Icon Theme]\nName=Hicolor\nDirectories=48x48/apps,256x256/apps\n\n\
             [48x48/apps]\nSize=48\nType=Fixed\n\n[256x256/apps]\nSize=256\nType=Fixed\n",
        );
        write(&data.join("icons/hicolor/48x48/apps/editor.png"), "png");
        write(
            &data.join("icons/hicolor/256x256/apps/browser-icon.png"),
            "png",
        );
        data
    }

    #[test]
    fn matches_classes_to_entries_and_picks_the_closest_size() {
        let data = fixture();
        let icons = Icons::new(Some("Mine".into()), vec![data.clone()]);
        // Exact id, then StartupWMClass, then last segment, then name.
        let editor = data.join("icons/Mine/scalable/apps/editor.svg");
        assert_eq!(icons.resolve("org.example.Editor"), Some(editor.clone()));
        assert_eq!(icons.resolve("EditorMain"), Some(editor.clone()));
        assert_eq!(icons.resolve("editormain"), Some(editor.clone()));
        assert_eq!(icons.resolve("editor"), Some(editor.clone()));
        assert_eq!(icons.resolve("Editor"), Some(editor));
        // A subdirectory entry, whose icon exists only in the parent theme.
        assert_eq!(
            icons.resolve("browser"),
            Some(data.join("icons/hicolor/256x256/apps/browser-icon.png"))
        );
        assert_eq!(icons.resolve("nothing-installed"), None);
        assert_eq!(icons.resolve(""), None);
        assert!(icons.icon_for("org.example.Editor").is_some());
        assert!(icons.icon_for("ghost").is_none());
        let _ = fs::remove_dir_all(data);
    }

    #[test]
    fn hidden_and_non_application_entries_do_not_match() {
        let data = fixture();
        let icons = Icons::new(None, vec![data.clone()]);
        assert_eq!(icons.theme_name(), "hicolor");
        assert_eq!(icons.resolve("ghost"), None);
        assert_eq!(icons.resolve("link"), None);
        // hicolor alone: the fixed 48 px file wins over 256.
        assert_eq!(
            icons.resolve("org.example.Editor"),
            Some(data.join("icons/hicolor/48x48/apps/editor.png"))
        );
        let _ = fs::remove_dir_all(data);
    }

    #[test]
    fn parses_index_theme_directory_rules() {
        let theme = parse_index_theme(
            "[Icon Theme]\nInherits=A, B\nDirectories=16x16/apps,48x48@2x/apps,scalable/apps\n\
             [16x16/apps]\nSize=16\nType=Threshold\n[48x48@2x/apps]\nSize=48\nScale=2\nType=Fixed\n\
             [scalable/apps]\nSize=64\nMinSize=8\nMaxSize=512\nType=Scalable\n[unlisted]\nSize=32\n",
        );
        assert_eq!(theme.inherits, ["A", "B"]);
        assert_eq!(theme.dirs.len(), 3);
        assert_eq!(theme.dirs[0].distance(48), Some(30));
        assert_eq!(theme.dirs[1].distance(96), Some(0));
        assert_eq!(theme.dirs[2].distance(48), Some(0));
        assert_eq!(theme.dirs[2].distance(1024), Some(512));
    }

    #[test]
    fn desktop_entries_need_a_type_and_an_icon() {
        assert!(parse_desktop_entry("a", "[Desktop Entry]\nType=Application\nName=A\n").is_none());
        assert!(parse_desktop_entry("a", "[Other]\nType=Application\nIcon=a\n").is_none());
        let entry = parse_desktop_entry(
            "a",
            "[Desktop Entry]\nType=Application\nIcon=a\nStartupWMClass=\n[Desktop Action x]\nIcon=b\n",
        )
        .unwrap();
        assert_eq!(entry.icon, "a");
        assert_eq!(entry.wm_class, None);
    }
}
