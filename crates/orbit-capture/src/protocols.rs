//! Lists the Wayland globals a compositor advertises and classifies the ones
//! Orbit's feasibility gates depend on. Binds nothing beyond the registry.

use anyhow::{Context, Result};
use serde::Serialize;
use wayland_client::{
    Connection, Dispatch, QueueHandle,
    globals::{GlobalListContents, registry_queue_init},
    protocol::wl_registry,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Global {
    pub interface: String,
    pub version: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Gate {
    /// Gate A: overlay surface, focus, scaling, lock safety.
    Overlay,
    /// Gate B: window previews.
    Capture,
}

/// A protocol Orbit cares about, with why.
pub struct Interest {
    pub interface: &'static str,
    pub gate: Gate,
    pub required: bool,
    pub purpose: &'static str,
}

pub const INTERESTS: &[Interest] = &[
    Interest {
        interface: "zwlr_layer_shell_v1",
        gate: Gate::Overlay,
        required: true,
        purpose: "overlay surface above windows",
    },
    Interest {
        interface: "wp_fractional_scale_manager_v1",
        gate: Gate::Overlay,
        required: false,
        purpose: "crisp rendering at fractional scales",
    },
    Interest {
        interface: "wp_viewporter",
        gate: Gate::Overlay,
        required: false,
        purpose: "fractional scale presentation",
    },
    Interest {
        interface: "ext_session_lock_manager_v1",
        gate: Gate::Overlay,
        required: false,
        purpose: "lock-screen protocol present (lock-state gating)",
    },
    Interest {
        interface: "ext_image_copy_capture_manager_v1",
        gate: Gate::Capture,
        required: false,
        purpose: "preferred window capture",
    },
    Interest {
        interface: "ext_foreign_toplevel_image_capture_source_manager_v1",
        gate: Gate::Capture,
        required: false,
        purpose: "capture source per toplevel",
    },
    Interest {
        interface: "ext_foreign_toplevel_list_v1",
        gate: Gate::Capture,
        required: false,
        purpose: "toplevel enumeration for capture handles",
    },
    Interest {
        interface: "hyprland_toplevel_mapping_manager_v1",
        gate: Gate::Capture,
        required: false,
        purpose: "map capture handles to Hyprland addresses",
    },
    Interest {
        interface: "hyprland_toplevel_export_manager_v1",
        gate: Gate::Capture,
        required: false,
        purpose: "compatibility window capture",
    },
    Interest {
        interface: "zwlr_foreign_toplevel_manager_v1",
        gate: Gate::Capture,
        required: false,
        purpose: "legacy toplevel handles for toplevel export",
    },
    Interest {
        interface: "ext_output_image_capture_source_manager_v1",
        gate: Gate::Capture,
        required: false,
        purpose: "output capture (not a window-preview substitute)",
    },
    Interest {
        interface: "zwlr_screencopy_manager_v1",
        gate: Gate::Capture,
        required: false,
        purpose: "legacy output capture (not a window-preview substitute)",
    },
    Interest {
        interface: "zwp_linux_dmabuf_v1",
        gate: Gate::Capture,
        required: false,
        purpose: "DMA-BUF buffers (deferred until measured)",
    },
    Interest {
        interface: "wl_shm",
        gate: Gate::Capture,
        required: true,
        purpose: "shared-memory capture buffers",
    },
];

#[derive(Clone, Debug, Serialize)]
pub struct Finding {
    pub interface: &'static str,
    pub gate: Gate,
    pub required: bool,
    pub purpose: &'static str,
    /// Highest advertised version, if advertised at all.
    pub version: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CapturePath {
    /// ext-image-copy-capture with a toplevel source and Hyprland mapping.
    ImageCopyCapture,
    /// hyprland-toplevel-export with foreign-toplevel handles.
    ToplevelExport,
    /// No per-window capture: metadata cards only.
    MetadataOnly,
}

#[derive(Clone, Debug, Serialize)]
pub struct Report {
    pub globals: Vec<Global>,
    pub findings: Vec<Finding>,
    pub capture_path: CapturePath,
}

impl Report {
    pub fn from_globals(mut globals: Vec<Global>) -> Self {
        globals.sort_by(|a, b| {
            a.interface
                .cmp(&b.interface)
                .then(a.version.cmp(&b.version))
        });
        let version = |name: &str| {
            globals
                .iter()
                .filter(|global| global.interface == name)
                .map(|global| global.version)
                .max()
        };
        let findings = INTERESTS
            .iter()
            .map(|interest| Finding {
                interface: interest.interface,
                gate: interest.gate,
                required: interest.required,
                purpose: interest.purpose,
                version: version(interest.interface),
            })
            .collect();
        let has = |name: &str| version(name).is_some();
        // Candidates only: Gate B still has to prove each path on real windows.
        let capture_path = if has("ext_image_copy_capture_manager_v1")
            && has("ext_foreign_toplevel_image_capture_source_manager_v1")
            && has("ext_foreign_toplevel_list_v1")
            && has("hyprland_toplevel_mapping_manager_v1")
        {
            CapturePath::ImageCopyCapture
        } else if has("hyprland_toplevel_export_manager_v1")
            && has("zwlr_foreign_toplevel_manager_v1")
        {
            CapturePath::ToplevelExport
        } else {
            CapturePath::MetadataOnly
        };
        Self {
            globals,
            findings,
            capture_path,
        }
    }

    pub fn missing_required(&self) -> impl Iterator<Item = &Finding> {
        self.findings
            .iter()
            .filter(|finding| finding.required && finding.version.is_none())
    }
}

/// Connects to `WAYLAND_DISPLAY`, reads the registry once and disconnects.
pub fn probe() -> Result<Report> {
    let connection =
        Connection::connect_to_env().context("cannot connect to the Wayland display")?;
    let (globals, _queue) =
        registry_queue_init::<Registry>(&connection).context("cannot read the Wayland registry")?;
    let globals = globals.contents().with_list(|list| {
        list.iter()
            .map(|global| Global {
                interface: global.interface.clone(),
                version: global.version,
            })
            .collect()
    });
    Ok(Report::from_globals(globals))
}

struct Registry;

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Registry {
    fn event(
        _: &mut Self,
        _: &wl_registry::WlRegistry,
        _: wl_registry::Event,
        _: &GlobalListContents,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn globals(names: &[(&str, u32)]) -> Vec<Global> {
        names
            .iter()
            .map(|(interface, version)| Global {
                interface: (*interface).into(),
                version: *version,
            })
            .collect()
    }

    #[test]
    fn prefers_image_copy_capture_when_fully_mapped() {
        let report = Report::from_globals(globals(&[
            ("wl_shm", 1),
            ("zwlr_layer_shell_v1", 5),
            ("ext_image_copy_capture_manager_v1", 1),
            ("ext_foreign_toplevel_image_capture_source_manager_v1", 1),
            ("ext_foreign_toplevel_list_v1", 1),
            ("hyprland_toplevel_mapping_manager_v1", 1),
            ("hyprland_toplevel_export_manager_v1", 2),
            ("zwlr_foreign_toplevel_manager_v1", 3),
        ]));
        assert!(matches!(report.capture_path, CapturePath::ImageCopyCapture));
        assert_eq!(report.missing_required().count(), 0);
    }

    #[test]
    fn falls_back_without_the_mapping_protocol() {
        let report = Report::from_globals(globals(&[
            ("wl_shm", 1),
            ("ext_image_copy_capture_manager_v1", 1),
            ("ext_foreign_toplevel_image_capture_source_manager_v1", 1),
            ("ext_foreign_toplevel_list_v1", 1),
            ("hyprland_toplevel_export_manager_v1", 2),
            ("zwlr_foreign_toplevel_manager_v1", 3),
        ]));
        assert!(matches!(report.capture_path, CapturePath::ToplevelExport));
        let missing: Vec<_> = report.missing_required().map(|f| f.interface).collect();
        assert_eq!(missing, ["zwlr_layer_shell_v1"]);
    }

    #[test]
    fn output_capture_alone_is_metadata_only() {
        let report =
            Report::from_globals(globals(&[("wl_shm", 1), ("zwlr_screencopy_manager_v1", 3)]));
        assert!(matches!(report.capture_path, CapturePath::MetadataOnly));
    }

    #[test]
    fn reports_highest_advertised_version() {
        let report = Report::from_globals(globals(&[("wl_shm", 1), ("wl_shm", 2)]));
        let shm = report
            .findings
            .iter()
            .find(|f| f.interface == "wl_shm")
            .unwrap();
        assert_eq!(shm.version, Some(2));
    }
}
