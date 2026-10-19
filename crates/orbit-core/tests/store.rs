//! Deterministic tests for snapshot/event sync and window moves (ORB-005).

use orbit_core::*;

fn ws(id: i64, name: &str) -> Workspace {
    Workspace {
        id: WorkspaceId(id),
        name: name.into(),
        monitor: Some("eDP-1".into()),
    }
}

fn win(address: u64, workspace: i64, pid: i64) -> WindowInfo {
    WindowInfo {
        address,
        workspace: WorkspaceId(workspace),
        class: format!("app{pid}"),
        title: String::new(),
        pid,
        floating: false,
        fullscreen: false,
        xwayland: false,
        hidden: false,
        at: (0, 0),
        size: (0, 0),
    }
}

fn monitor() -> Monitor {
    Monitor {
        id: 0,
        name: "eDP-1".into(),
        focused: true,
        active_workspace: WorkspaceId(1),
        special_workspace: None,
        width: 2304.0,
        height: 1440.0,
        x: 0.0,
        y: 0.0,
        scale: 1.25,
    }
}

fn snapshot(windows: Vec<WindowInfo>) -> Refresh {
    Refresh::full(
        vec![monitor()],
        vec![ws(1, "1"), ws(2, "2"), ws(-98, "special:scratch")],
        windows,
        None,
    )
}

fn windows_only(windows: Vec<WindowInfo>) -> Refresh {
    Refresh {
        windows: Some(windows),
        active_window: Some(None),
        ..Default::default()
    }
}

/// A store that is live with windows 0xa (ws 1) and 0xb (ws 2).
fn live() -> (Store, Epoch) {
    let mut store = Store::new(Config::default());
    let (epoch, effects) = store.connect(0);
    assert_eq!(effects, vec![Effect::Query(Kinds::ALL)]);
    let effects = store
        .on_refresh(epoch, snapshot(vec![win(0xa, 1, 10), win(0xb, 2, 11)]))
        .unwrap();
    assert!(effects.is_empty(), "{effects:?}");
    assert_eq!(store.sync(), Sync::Live);
    (store, epoch)
}

fn id(store: &Store, address: u64) -> WindowId {
    store.model().windows[&address].id
}

fn moved(address: u64, workspace: i64) -> Event {
    Event::WindowMoved {
        address,
        workspace: WorkspaceId(workspace),
        workspace_name: workspace.to_string(),
    }
}

fn finished(effects: &[Effect]) -> Vec<(OpId, Outcome)> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Finished { op, outcome } => Some((*op, outcome.clone())),
            _ => None,
        })
        .collect()
}

fn dispatched(effects: &[Effect]) -> Vec<OpId> {
    effects
        .iter()
        .filter_map(|e| match e {
            Effect::Dispatch { op, .. } => Some(*op),
            _ => None,
        })
        .collect()
}

#[test]
fn events_during_first_sync_are_not_replayed_but_requeried() {
    let mut store = Store::new(Config::default());
    let (epoch, _) = store.connect(0);
    // The snapshot query is out; a move event arrives before its answer.
    assert!(store.on_event(epoch, moved(0xa, 2)).unwrap().is_empty());
    // The answer may predate the move: apply it as is, then re-query.
    let effects = store
        .on_refresh(epoch, snapshot(vec![win(0xa, 1, 10)]))
        .unwrap();
    assert_eq!(store.model().windows[&0xa].workspace, WorkspaceId(1));
    assert_eq!(
        effects,
        vec![Effect::Query(Kinds::WINDOWS | Kinds::WORKSPACES)]
    );
    store
        .on_refresh(
            epoch,
            Refresh {
                workspaces: Some(vec![ws(1, "1"), ws(2, "2")]),
                ..windows_only(vec![win(0xa, 2, 10)])
            },
        )
        .unwrap();
    assert_eq!(store.model().windows[&0xa].workspace, WorkspaceId(2));
}

#[test]
fn events_racing_a_live_query_trigger_one_more_query() {
    let (mut store, epoch) = live();
    let effects = store.on_gap(epoch).unwrap();
    assert_eq!(effects, vec![Effect::Query(Kinds::ALL)]);
    // Applied immediately while live...
    store.on_event(epoch, moved(0xb, 1)).unwrap();
    assert_eq!(store.model().windows[&0xb].workspace, WorkspaceId(1));
    // ...but the in-flight answer may be older, so windows are asked again.
    let effects = store
        .on_refresh(epoch, snapshot(vec![win(0xa, 1, 10), win(0xb, 2, 11)]))
        .unwrap();
    assert_eq!(
        effects,
        vec![Effect::Query(Kinds::WINDOWS | Kinds::WORKSPACES)]
    );
    // A quiet answer settles it.
    let effects = store
        .on_refresh(
            epoch,
            Refresh {
                workspaces: Some(vec![ws(1, "1"), ws(2, "2")]),
                ..windows_only(vec![win(0xa, 1, 10), win(0xb, 1, 11)])
            },
        )
        .unwrap();
    assert!(effects.is_empty());
    assert_eq!(store.inflight(), Kinds::NONE);
}

#[test]
fn queries_coalesce() {
    let (mut store, epoch) = live();
    let first = store.on_event(epoch, Event::Fullscreen).unwrap();
    assert_eq!(first, vec![Effect::Query(Kinds::WINDOWS)]);
    // A second trigger while the first is out sends nothing new.
    assert!(store.on_event(epoch, Event::Fullscreen).unwrap().is_empty());
}

#[test]
fn stale_epoch_input_is_rejected_after_restart() {
    let (mut store, old) = live();
    let window = id(&store, 0xa);
    let (op, _) = store
        .request_move(window, Target::Workspace(WorkspaceId(2)), 0)
        .unwrap();

    // Compositor restarted: new epoch, pending work invalidated.
    let (epoch, effects) = store.connect(10);
    assert_ne!(epoch, old);
    assert_eq!(
        finished(&effects),
        vec![(op, Outcome::Failed(Failure::SessionRestarted))]
    );
    assert!(store.model().windows.is_empty());
    assert_eq!(store.sync(), Sync::Syncing);

    assert_eq!(
        store.on_event(old, moved(0xa, 2)),
        Err(Rejected::StaleEpoch)
    );
    assert_eq!(
        store.on_refresh(old, snapshot(vec![win(0xa, 2, 10)])),
        Err(Rejected::StaleEpoch)
    );
    // Late acks for the old move change nothing.
    assert!(store.on_dispatch_result(op, Ok(())).unwrap().is_empty());

    store
        .on_refresh(epoch, snapshot(vec![win(0xa, 1, 10)]))
        .unwrap();
    // The same address in a new epoch is a new window identity.
    assert_ne!(id(&store, 0xa), window);
    assert_eq!(
        store.request_move(window, Target::Numbered(3), 20),
        Err(RequestError::StaleWindow)
    );
}

#[test]
fn reused_addresses_get_a_new_generation() {
    let (mut store, epoch) = live();
    let old = id(&store, 0xa);
    // Close and reopen at the same address, with the close event missed.
    store
        .on_refresh(epoch, windows_only(vec![win(0xa, 1, 99), win(0xb, 2, 11)]))
        .unwrap();
    let new = id(&store, 0xa);
    assert_eq!(new.address, old.address);
    assert_ne!(new.generation, old.generation);
    assert_eq!(
        store.request_move(old, Target::Numbered(2), 0),
        Err(RequestError::StaleWindow)
    );

    // Same via events: openwindow on a known address replaces the window.
    store
        .on_event(
            epoch,
            Event::WindowOpened {
                address: 0xb,
                workspace_name: "1".into(),
                class: "other".into(),
                title: "x".into(),
            },
        )
        .unwrap();
    assert_eq!(store.model().windows[&0xb].class, "other");
    assert_eq!(store.model().windows[&0xb].workspace, WorkspaceId(1));
}

#[test]
fn a_window_learned_from_an_event_keeps_its_identity_after_the_query() {
    let (mut store, epoch) = live();
    store
        .on_event(
            epoch,
            Event::WindowOpened {
                address: 0xc,
                workspace_name: "2".into(),
                class: "app12".into(),
                title: String::new(),
            },
        )
        .unwrap();
    let opened = id(&store, 0xc);
    assert_eq!(
        store.request_move(opened, Target::Numbered(1), 0),
        Err(RequestError::Unverified)
    );
    store
        .on_refresh(
            epoch,
            windows_only(vec![win(0xa, 1, 10), win(0xb, 2, 11), win(0xc, 2, 12)]),
        )
        .unwrap();
    assert_eq!(id(&store, 0xc), opened);
    assert_eq!(store.model().windows[&0xc].pid, 12);
    assert!(store.request_move(opened, Target::Numbered(1), 0).is_ok());
}

#[test]
fn overflow_while_syncing_requeries_everything() {
    let mut store = Store::new(Config {
        sync_buffer: 2,
        ..Config::default()
    });
    let (epoch, _) = store.connect(0);
    for _ in 0..3 {
        store
            .on_event(
                epoch,
                Event::WindowTitle {
                    address: 0xa,
                    title: "t".into(),
                },
            )
            .unwrap();
    }
    let effects = store
        .on_refresh(epoch, snapshot(vec![win(0xa, 1, 10)]))
        .unwrap();
    assert_eq!(effects, vec![Effect::Query(Kinds::ALL)]);
}

#[test]
fn move_confirmed_by_event() {
    let (mut store, epoch) = live();
    let (op, effects) = store
        .request_move(id(&store, 0xa), Target::Workspace(WorkspaceId(2)), 0)
        .unwrap();
    assert_eq!(dispatched(&effects), vec![op]);
    assert!(matches!(
        store.op_state(op),
        Some(OpState::Pending { deadline: 1000 })
    ));
    store.on_dispatch_result(op, Ok(())).unwrap();
    // Accepted is not done.
    assert!(matches!(store.op_state(op), Some(OpState::Pending { .. })));
    let effects = store.on_event(epoch, moved(0xa, 2)).unwrap();
    assert_eq!(finished(&effects), vec![(op, Outcome::Confirmed)]);
}

#[test]
fn events_read_before_acceptance_cannot_confirm() {
    let (mut store, epoch) = live();
    let (op, _) = store
        .request_move(id(&store, 0xa), Target::Numbered(2), 0)
        .unwrap();
    // Another client moved it to 2 and back before our request ran; those
    // events are read only now.
    assert!(finished(&store.on_event(epoch, moved(0xa, 2)).unwrap()).is_empty());
    store.on_event(epoch, moved(0xa, 1)).unwrap();
    // Our move lands: its event precedes the reply.
    store.on_event(epoch, moved(0xa, 2)).unwrap();
    let effects = store.on_dispatch_result(op, Ok(())).unwrap();
    assert_eq!(finished(&effects), vec![(op, Outcome::Confirmed)]);
}

#[test]
fn lost_ack_resolved_by_requery() {
    for (moved_anyway, expected) in [
        (true, Outcome::Confirmed),
        (false, Outcome::Failed(Failure::NotObserved)),
    ] {
        let (mut store, epoch) = live();
        let (op, _) = store
            .request_move(id(&store, 0xa), Target::Numbered(5), 0)
            .unwrap();
        // No ack, no event.
        assert!(store.tick(999).is_empty());
        let effects = store.tick(1000);
        assert_eq!(effects, vec![Effect::Query(Kinds::WINDOWS)]);
        assert_eq!(store.op_state(op), Some(&OpState::Unknown));
        let workspace = if moved_anyway { 5 } else { 1 };
        let effects = store
            .on_refresh(
                epoch,
                windows_only(vec![win(0xa, workspace, 10), win(0xb, 2, 11)]),
            )
            .unwrap();
        assert_eq!(finished(&effects), vec![(op, expected)]);
    }
}

#[test]
fn a_racing_answer_cannot_fail_a_timed_out_move() {
    let (mut store, epoch) = live();
    let (op, _) = store
        .request_move(id(&store, 0xa), Target::Numbered(5), 0)
        .unwrap();
    store.tick(1000);
    // A window event lands while the re-query is out: its answer is
    // ambiguous, so the move stays unknown and windows are asked again.
    store
        .on_event(
            epoch,
            Event::WindowTitle {
                address: 0xb,
                title: "t".into(),
            },
        )
        .unwrap();
    let effects = store
        .on_refresh(epoch, windows_only(vec![win(0xa, 1, 10), win(0xb, 2, 11)]))
        .unwrap();
    assert!(finished(&effects).is_empty());
    assert_eq!(effects, vec![Effect::Query(Kinds::WINDOWS)]);
    let effects = store
        .on_refresh(epoch, windows_only(vec![win(0xa, 5, 10), win(0xb, 2, 11)]))
        .unwrap();
    assert_eq!(finished(&effects), vec![(op, Outcome::Confirmed)]);
}

#[test]
fn rejected_dispatch_fails_and_starts_the_next_move() {
    let (mut store, epoch) = live();
    let window = id(&store, 0xa);
    let (first, effects) = store.request_move(window, Target::Numbered(5), 0).unwrap();
    assert_eq!(dispatched(&effects), vec![first]);
    // Conflicting moves on one window are serialized, not sent together.
    let (second, effects) = store
        .request_move(window, Target::Workspace(WorkspaceId(2)), 0)
        .unwrap();
    assert!(effects.is_empty());
    assert_eq!(store.op_state(second), Some(&OpState::Queued));

    let effects = store.on_dispatch_result(first, Err("bad".into())).unwrap();
    assert_eq!(
        finished(&effects),
        vec![(first, Outcome::Failed(Failure::Rejected("bad".into())))]
    );
    assert_eq!(dispatched(&effects), vec![second]);
    store.on_event(epoch, moved(0xa, 2)).unwrap();
    let effects = store.on_dispatch_result(second, Ok(())).unwrap();
    assert_eq!(finished(&effects), vec![(second, Outcome::Confirmed)]);
}

#[test]
fn queued_move_to_the_same_place_is_still_dispatched() {
    let (mut store, epoch) = live();
    let window = id(&store, 0xa);
    let (first, _) = store.request_move(window, Target::Numbered(2), 0).unwrap();
    let (second, _) = store.request_move(window, Target::Numbered(2), 0).unwrap();
    store.on_dispatch_result(first, Ok(())).unwrap();
    let effects = store.on_event(epoch, moved(0xa, 2)).unwrap();
    // The model is not proof on its own; the second move goes out too.
    assert_eq!(finished(&effects), vec![(first, Outcome::Confirmed)]);
    assert_eq!(dispatched(&effects), vec![second]);
    let effects = store.on_dispatch_result(second, Ok(())).unwrap();
    assert_eq!(finished(&effects), vec![(second, Outcome::Confirmed)]);
}

#[test]
fn closing_a_window_fails_its_moves() {
    let (mut store, epoch) = live();
    let window = id(&store, 0xa);
    let (first, _) = store.request_move(window, Target::Numbered(5), 0).unwrap();
    let (second, _) = store.request_move(window, Target::Numbered(6), 0).unwrap();
    let effects = store
        .on_event(epoch, Event::WindowClosed { address: 0xa })
        .unwrap();
    assert_eq!(
        finished(&effects),
        vec![
            (first, Outcome::Failed(Failure::WindowGone)),
            (second, Outcome::Failed(Failure::WindowGone)),
        ]
    );
    assert!(dispatched(&effects).is_empty());
}

#[test]
fn destination_destroyed_before_a_queued_move_starts() {
    let (mut store, epoch) = live();
    let window = id(&store, 0xa);
    let (first, _) = store.request_move(window, Target::Numbered(5), 0).unwrap();
    let (second, _) = store
        .request_move(window, Target::Workspace(WorkspaceId(-98)), 0)
        .unwrap();
    store
        .on_event(
            epoch,
            Event::WorkspaceDestroyed {
                id: WorkspaceId(-98),
            },
        )
        .unwrap();
    store.on_dispatch_result(first, Ok(())).unwrap();
    let effects = store.on_event(epoch, moved(0xa, 5)).unwrap();
    assert_eq!(
        finished(&effects),
        vec![
            (first, Outcome::Confirmed),
            (second, Outcome::Failed(Failure::DestinationGone)),
        ]
    );
}

#[test]
fn request_validation() {
    let mut store = Store::new(Config::default());
    let ghost = WindowId {
        address: 0xa,
        generation: 1,
    };
    assert_eq!(
        store.request_move(ghost, Target::Numbered(2), 0),
        Err(RequestError::NotLive)
    );
    let (mut store, _) = live();
    let window = id(&store, 0xa);
    assert_eq!(
        store.request_move(window, Target::Numbered(1), 0),
        Err(RequestError::AlreadyThere)
    );
    assert_eq!(
        store.request_move(window, Target::Workspace(WorkspaceId(77)), 0),
        Err(RequestError::UnknownDestination)
    );
    assert_eq!(
        store.request_move(window, Target::Numbered(0), 0),
        Err(RequestError::UnknownDestination)
    );
}

#[test]
fn health_check_only_while_visible() {
    let (mut store, epoch) = live();
    assert!(store.tick(60_000).is_empty());
    assert_eq!(
        store.set_visible(true, 60_000),
        vec![Effect::Query(Kinds::ALL)]
    );
    store
        .on_refresh(epoch, snapshot(vec![win(0xa, 1, 10), win(0xb, 2, 11)]))
        .unwrap();
    assert!(store.tick(64_999).is_empty());
    assert_eq!(store.tick(65_000), vec![Effect::Query(Kinds::ALL)]);
    store.set_visible(false, 65_000);
    store
        .on_refresh(epoch, snapshot(vec![win(0xa, 1, 10), win(0xb, 2, 11)]))
        .unwrap();
    assert!(store.tick(200_000).is_empty());
}

#[test]
fn monitor_and_workspace_events() {
    let (mut store, epoch) = live();
    store
        .on_event(epoch, Event::ActiveWorkspace { id: WorkspaceId(2) })
        .unwrap();
    assert_eq!(store.model().monitors[0].active_workspace, WorkspaceId(2));
    store
        .on_event(
            epoch,
            Event::ActiveSpecial {
                monitor: "eDP-1".into(),
                workspace: Some(WorkspaceId(-98)),
            },
        )
        .unwrap();
    assert_eq!(
        store.model().monitors[0].special_workspace,
        Some(WorkspaceId(-98))
    );
    store
        .on_event(
            epoch,
            Event::WorkspaceRenamed {
                id: WorkspaceId(2),
                name: "web".into(),
            },
        )
        .unwrap();
    assert_eq!(
        store.model().workspace_by_name("web").unwrap().id,
        WorkspaceId(2)
    );
    let effects = store
        .on_event(
            epoch,
            Event::MonitorAdded {
                name: "DP-1".into(),
            },
        )
        .unwrap();
    assert_eq!(effects, vec![Effect::Query(Kinds::ALL)]);
}
