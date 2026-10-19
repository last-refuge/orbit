//! `orbit assets install|remove [DIR]`: the desktop entry and the icon
//! theme files under a data directory (`$XDG_DATA_HOME`, default
//! `~/.local/share`). The installer and the release packaging both call
//! this, so the binary is the one source of the entry and the icons.

use std::{
    fs,
    path::{Path, PathBuf},
};

pub const DESKTOP_ENTRY: &str = "[Desktop Entry]
Type=Application
Name=Orbit
GenericName=Workspace overview
Comment=Search, preview and move windows across Hyprland workspaces
Exec=orbit toggle
Icon=orbit
Terminal=false
Categories=Utility;
Keywords=windows;workspaces;overview;hyprland;
StartupNotify=false
";

/// The icon sizes written, as hicolor expects them.
pub const ICON_SIZES: [u32; 4] = [64, 128, 256, 512];

/// `$XDG_DATA_HOME`, else `~/.local/share`.
pub fn default_dir() -> Option<PathBuf> {
    std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
}

/// Every file `install` writes under `dir`.
pub fn files(dir: &Path) -> Vec<PathBuf> {
    let mut out = vec![dir.join("applications/orbit.desktop")];
    out.extend(
        ICON_SIZES
            .iter()
            .map(|size| dir.join(format!("icons/hicolor/{size}x{size}/apps/orbit.png"))),
    );
    out
}

pub fn install(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut written = Vec::new();
    let entry = dir.join("applications/orbit.desktop");
    write(&entry, DESKTOP_ENTRY.as_bytes())?;
    written.push(entry);
    for size in ICON_SIZES {
        let png = orbit_ui::theme::app_icon_png(size)
            .ok_or_else(|| format!("could not render the {size} px icon"))?;
        let path = dir.join(format!("icons/hicolor/{size}x{size}/apps/orbit.png"));
        write(&path, &png)?;
        written.push(path);
    }
    Ok(written)
}

pub fn remove(dir: &Path) -> Result<Vec<PathBuf>, String> {
    let mut removed = Vec::new();
    for path in files(dir) {
        match fs::remove_file(&path) {
            Ok(()) => removed.push(path),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("removing {}: {e}", path.display())),
        }
    }
    Ok(removed)
}

fn write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let dir = path.parent().ok_or("asset path has no parent")?;
    fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    fs::write(path, bytes).map_err(|e| format!("writing {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installs_the_entry_and_icons_and_removes_them_again() {
        let dir = std::env::temp_dir().join(format!("orbit-assets-{}", std::process::id()));
        let written = install(&dir).unwrap();
        assert_eq!(written, files(&dir));
        assert!(
            fs::read_to_string(&written[0])
                .unwrap()
                .contains("Exec=orbit toggle")
        );
        let icon = image::load_from_memory(&fs::read(&written[3]).unwrap()).unwrap();
        assert_eq!((icon.width(), icon.height()), (256, 256));
        assert_eq!(remove(&dir).unwrap().len(), written.len());
        assert!(remove(&dir).unwrap().is_empty(), "idempotent");
        let _ = fs::remove_dir_all(dir);
    }
}
