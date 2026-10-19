//! Replays recorded event streams (`orbit events --record`) through the
//! parser and the state store. Recordings from real sessions live next to
//! a synthetic one in `tests/fixtures/events/`.

use std::fs;

use orbit_core::{Kinds, Monitor, Refresh, Store, Workspace, WorkspaceId};
use orbit_hyprland::events::parse;

fn recordings() -> Vec<(String, String)> {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/fixtures/events");
    let mut files: Vec<_> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "txt"))
        .collect();
    files.sort();
    files
        .into_iter()
        .map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            (name, fs::read_to_string(path).unwrap())
        })
        .collect()
}

#[test]
fn every_recorded_line_parses() {
    let recordings = recordings();
    assert!(!recordings.is_empty());
    for (name, text) in recordings {
        for (n, line) in text.lines().enumerate() {
            if let Err(error) = parse(line) {
                panic!("{name}:{}: {error}: {line}", n + 1);
            }
        }
    }
}

#[test]
fn recorded_streams_drive_the_store() {
    for (name, text) in recordings() {
        let mut store = Store::new(Default::default());
        let (epoch, _) = store.connect(0);
        store
            .on_refresh(
                epoch,
                Refresh::full(
                    vec![Monitor {
                        id: 0,
                        name: "eDP-1".into(),
                        focused: true,
                        active_workspace: WorkspaceId(1),
                        special_workspace: None,
                        x: 0.0,
                        y: 0.0,
                        width: 2304.0,
                        height: 1440.0,
                        scale: 1.25,
                    }],
                    vec![Workspace {
                        id: WorkspaceId(1),
                        name: "1".into(),
                        monitor: Some("eDP-1".into()),
                    }],
                    Vec::new(),
                    None,
                ),
            )
            .unwrap();
        let mut queried = Kinds::NONE;
        for line in text.lines() {
            if let Some(event) = parse(line).unwrap() {
                for effect in store.on_event(epoch, event).unwrap() {
                    if let orbit_core::Effect::Query(kinds) = effect {
                        queried |= kinds;
                    }
                }
            }
        }
        // Opening windows must ask for their details.
        if text.contains("openwindow>>") {
            assert!(queried.contains(Kinds::WINDOWS), "{name}");
        }
    }
}
