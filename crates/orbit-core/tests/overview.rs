//! The overview's selection, navigation and move intents (ORB-006), and
//! search, labels and keyboard parity (ORB-007).

use orbit_core::*;

fn ws(id: i64, name: &str) -> Workspace {
    Workspace {
        id: WorkspaceId(id),
        name: name.into(),
        monitor: Some("eDP-1".into()),
    }
}

fn win(address: u64, workspace: i64) -> WindowInfo {
    WindowInfo {
        address,
        workspace: WorkspaceId(workspace),
        class: format!("app{address:x}"),
        title: format!("title {address:x}"),
        pid: address as i64,
        floating: false,
        fullscreen: false,
        xwayland: false,
        hidden: false,
        at: (0, 0),
        size: (0, 0),
    }
}

/// Workspaces 1, 2, 5, "web" and special:scratch; 1 is active with 0xa
/// (focused) and 0xb, 2 has 0xc, the scratchpad has 0xd.
fn store() -> (Store, Epoch) {
    let mut store = Store::new(Config::default());
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
                    width: 2304.0,
                    height: 1440.0,
                    x: 0.0,
                    y: 0.0,
                    scale: 1.25,
                }],
                vec![
                    ws(5, "5"),
                    ws(1, "1"),
                    ws(-98, "special:scratch"),
                    ws(-1337, "web"),
                    ws(2, "2"),
                ],
                vec![win(0xa, 1), win(0xb, 1), win(0xc, 2), win(0xd, -98)],
                Some(0xb),
            ),
        )
        .unwrap();
    (store, epoch)
}

fn id(store: &Store, address: u64) -> WindowId {
    store.model().windows[&address].id
}

#[test]
fn opens_on_the_active_workspace_with_the_focused_window() {
    let (store, _) = store();
    let model = store.model();
    let overview = Overview::open(model, None);
    assert_eq!(overview.previewed, Some(WorkspaceId(1)));
    assert_eq!(overview.selected, Some(id(&store, 0xb)));
    assert_eq!(overview.invocation.window, Some(id(&store, 0xb)));

    let cards = overview.cards(model);
    assert_eq!(
        cards.iter().map(|c| c.window.address).collect::<Vec<_>>(),
        [0xa, 0xb]
    );
    assert!(cards[1].selected && !cards[0].selected);
}

#[test]
fn rail_orders_numbered_named_special_then_new() {
    let (store, _) = store();
    let overview = Overview::open(store.model(), None);
    let tiles = overview.tiles(store.model());
    let labels: Vec<_> = tiles.iter().map(|t| t.label.as_str()).collect();
    assert_eq!(labels, ["1", "2", "5", "web", "scratch", "New"]);
    assert_eq!(tiles.last().unwrap().dest, Dest::NewNumbered(6));
    assert!(tiles[0].active && tiles[0].previewed);
    assert_eq!(tiles[0].windows, 2);
    assert!(!tiles[1].active);
}

#[test]
fn arrows_in_the_rail_preview_without_switching() {
    let (store, _) = store();
    let model = store.model();
    let mut overview = Overview::open(model, None);
    overview.tab(true);
    overview.step(model, 1);
    assert_eq!(overview.previewed, Some(WorkspaceId(2)));
    assert_eq!(overview.selected, Some(id(&store, 0xc)));
    // Enter in the rail commits the switch.
    assert_eq!(
        overview.enter(model, false),
        Some(Intent::Switch(Dest::Existing(WorkspaceId(2))))
    );
    // Previewing the "New" tile keeps the last real preview.
    overview.step(model, 10);
    assert_eq!(overview.rail_focus, Some(Dest::NewNumbered(6)));
    assert_eq!(overview.previewed, Some(WorkspaceId(2)));
}

#[test]
fn enter_in_the_grid_activates_the_selection() {
    let (store, _) = store();
    let model = store.model();
    let mut overview = Overview::open(model, None);
    overview.step(model, -1);
    assert_eq!(
        overview.enter(model, false),
        Some(Intent::Activate(id(&store, 0xa)))
    );
    overview.step(model, -5);
    assert_eq!(overview.selected, Some(id(&store, 0xa)));
}

#[test]
fn move_menu_picks_a_valid_destination_and_escape_backs_out() {
    let (store, _) = store();
    let model = store.model();
    let mut overview = Overview::open(model, None);
    assert!(overview.begin_move(model));
    assert_eq!(overview.region, Region::Rail);
    // Workspace 1 holds the window, so the menu starts on 2 and never
    // lands on 1.
    assert_eq!(overview.rail_focus, Some(Dest::Existing(WorkspaceId(2))));
    overview.step(model, -1);
    assert_eq!(overview.rail_focus, Some(Dest::Existing(WorkspaceId(2))));
    overview.step(model, 1);
    // Choosing a destination does not change the preview.
    assert_eq!(overview.previewed, Some(WorkspaceId(1)));
    assert_eq!(overview.escape(model), None);
    assert_eq!(overview.region, Region::Grid);
    assert_eq!(overview.escape(model), Some(Intent::Dismiss));

    // Reopening the menu starts where it was left (5).
    assert!(overview.begin_move(model));
    assert_eq!(
        overview.enter(model, true),
        Some(Intent::Move {
            window: id(&store, 0xb),
            dest: Dest::Existing(WorkspaceId(5)),
            follow: true
        })
    );
    assert_eq!(overview.moving, None);
}

#[test]
fn drops_share_the_availability_check() {
    let (store, _) = store();
    let model = store.model();
    let mut overview = Overview::open(model, None);
    let a = id(&store, 0xa);
    // Dropping on its own workspace is a quiet no-op.
    assert_eq!(
        overview.drop_on(model, a, Dest::Existing(WorkspaceId(1)), false),
        None
    );
    assert_eq!(overview.announcement, None);
    assert_eq!(
        overview.drop_on(model, a, Dest::Existing(WorkspaceId(-98)), false),
        Some(Intent::Move {
            window: a,
            dest: Dest::Existing(WorkspaceId(-98)),
            follow: false
        })
    );
    assert_eq!(
        overview.drop_on(model, a, Dest::NewNumbered(6), false),
        Some(Intent::Move {
            window: a,
            dest: Dest::NewNumbered(6),
            follow: false
        })
    );
    // A vanished destination is reported, never redirected.
    assert_eq!(
        overview.drop_on(model, a, Dest::Existing(WorkspaceId(77)), false),
        None
    );
    assert_eq!(overview.announcement.as_ref().unwrap().tone, Tone::Error);
    // A stale window id (address reused) is refused.
    let stale = WindowId {
        address: 0xa,
        generation: 999,
    };
    assert_eq!(
        overview.move_target(model, stale, Dest::NewNumbered(6)),
        Err(Unavailable::WindowGone)
    );
}

#[test]
fn tracks_moves_until_the_store_finishes_them() {
    let (mut store, epoch) = store();
    let mut overview = Overview::open(store.model(), None);
    let b = id(&store, 0xb);
    let Some(Intent::Move {
        window,
        dest,
        follow,
    }) = overview.drop_on(store.model(), b, Dest::Existing(WorkspaceId(2)), true)
    else {
        panic!("expected a move");
    };
    let (op, effects) = store.request_move(window, dest.target(), 0).unwrap();
    assert!(matches!(effects[0], Effect::Dispatch { .. }));
    overview.track(store.model(), op, window, dest, follow);
    let card = overview
        .cards(store.model())
        .into_iter()
        .find(|c| c.window == b)
        .unwrap();
    assert_eq!(card.moving_to.as_deref(), Some("2"));
    assert_eq!(overview.announcement.as_ref().unwrap().text, "Moving to 2…");

    store.on_dispatch_result(op, Ok(())).unwrap();
    let effects = store
        .on_event(
            epoch,
            Event::WindowMoved {
                address: 0xb,
                workspace: WorkspaceId(2),
                workspace_name: "2".into(),
            },
        )
        .unwrap();
    let outcome = effects
        .iter()
        .find_map(|e| match e {
            Effect::Finished { op: done, outcome } if *done == op => Some(outcome.clone()),
            _ => None,
        })
        .expect("confirmed by the event");
    assert_eq!(overview.finished(op, &outcome), Some(Intent::Activate(b)));
    assert_eq!(overview.announcement.as_ref().unwrap().text, "Moved to 2");

    // The window left the previewed workspace: selection moves on.
    overview.reconcile(store.model());
    assert_eq!(overview.selected, Some(id(&store, 0xa)));
    assert_eq!(overview.pending_moves(), 0);
}

#[test]
fn failures_are_announced_without_following() {
    let (mut store, epoch) = store();
    let mut overview = Overview::open(store.model(), None);
    let b = id(&store, 0xb);
    let (op, _) = store
        .request_move(b, Target::Workspace(WorkspaceId(2)), 0)
        .unwrap();
    overview.track(store.model(), op, b, Dest::Existing(WorkspaceId(2)), true);
    let effects = store
        .on_event(epoch, Event::WindowClosed { address: 0xb })
        .unwrap();
    let Some(Effect::Finished { outcome, .. }) = effects.into_iter().next() else {
        panic!("expected the move to fail");
    };
    assert_eq!(overview.finished(op, &outcome), None);
    let announcement = overview.announcement.clone().unwrap();
    assert_eq!(announcement.text, "The window closed before it moved");
    assert_eq!(announcement.tone, Tone::Error);
}

#[test]
fn reconcile_survives_a_vanished_preview() {
    let (mut store, epoch) = store();
    let mut overview = Overview::open(store.model(), None);
    overview.tab(true);
    overview.step(store.model(), 1);
    assert_eq!(overview.previewed, Some(WorkspaceId(2)));
    store
        .on_event(epoch, Event::WindowClosed { address: 0xc })
        .unwrap();
    store
        .on_event(epoch, Event::WorkspaceDestroyed { id: WorkspaceId(2) })
        .unwrap();
    overview.reconcile(store.model());
    assert_eq!(overview.previewed, Some(WorkspaceId(1)));
    assert_eq!(overview.rail_focus, Some(Dest::Existing(WorkspaceId(1))));
    assert!(overview.selected.is_some());
}

#[test]
fn tab_cycles_search_rail_grid_and_arrows_cross_regions() {
    let (store, _) = store();
    let model = store.model();
    let mut overview = Overview::open(model, None);
    assert_eq!(overview.region, Region::Grid);
    overview.tab(false);
    assert_eq!(overview.region, Region::Search);
    overview.tab(false);
    assert_eq!(overview.region, Region::Rail);
    overview.tab(true);
    assert_eq!(overview.region, Region::Search);
    // Down walks search → rail → grid; Up walks back.
    overview.down(model, 3);
    assert_eq!(overview.region, Region::Rail);
    overview.down(model, 3);
    assert_eq!(overview.region, Region::Grid);
    overview.up(model, 3);
    overview.up(model, 3);
    assert_eq!(overview.region, Region::Search);
    // The move menu holds focus on its destinations.
    assert!(overview.begin_move(model));
    overview.tab(false);
    overview.up(model, 3);
    assert_eq!(overview.region, Region::Rail);
}

#[test]
fn typing_anywhere_searches_every_workspace() {
    let (store, _) = store();
    let model = store.model();
    let mut overview = Overview::open(model, None);
    // 0xc lives on workspace 2, not the previewed 1.
    overview.type_text(model, "appc");
    assert_eq!(overview.region, Region::Search);
    assert!(overview.searching());
    let cards = overview.cards(model);
    assert_eq!(
        cards.iter().map(|c| c.window.address).collect::<Vec<_>>(),
        [0xc]
    );
    assert_eq!(cards[0].workspace, "2");
    assert_eq!(cards[0].monitor.as_deref(), Some("eDP-1"));
    assert_eq!(overview.selected, Some(id(&store, 0xc)));
    // Enter takes the selected match: focusing it switches to its workspace.
    assert_eq!(
        overview.enter(model, false),
        Some(Intent::Activate(id(&store, 0xc)))
    );
    // Arrows in search edit text; they do not move the selection.
    overview.horizontal(model, -1);
    overview.type_text(model, "x");
    assert_eq!(overview.search.text(), "appxc");
    assert!(overview.cards(model).is_empty());
    assert_eq!(overview.announcement.as_ref().unwrap().text, "No matches");
    assert_eq!(overview.enter(model, false), None);
}

#[test]
fn search_matches_titles_and_workspace_labels() {
    let (store, _) = store();
    let model = store.model();
    let mut overview = Overview::open(model, None);
    overview.type_text(model, "title d");
    assert_eq!(
        overview
            .cards(model)
            .iter()
            .map(|c| c.window.address)
            .collect::<Vec<_>>(),
        [0xd]
    );
    // A workspace match with no window matches: Enter switches to it.
    overview.edit(model, Field::clear);
    overview.type_text(model, "web");
    assert!(overview.cards(model).is_empty());
    let tiles = overview.tiles(model);
    assert_eq!(
        tiles.iter().map(|t| t.label.as_str()).collect::<Vec<_>>(),
        ["web"]
    );
    assert_eq!(
        overview.enter(model, false),
        Some(Intent::Switch(Dest::Existing(WorkspaceId(-1337))))
    );
    // Workspace "2" matches its windows (they are on it) and the tile.
    overview.edit(model, Field::clear);
    overview.type_text(model, "2");
    assert_eq!(overview.tiles(model)[0].label, "2");
    assert!(
        overview
            .cards(model)
            .iter()
            .any(|c| c.window.address == 0xc)
    );
}

#[test]
fn escape_closes_the_menu_then_clears_search_then_dismisses() {
    let (store, _) = store();
    let model = store.model();
    let mut overview = Overview::open(model, None);
    overview.type_text(model, "app");
    assert!(overview.begin_move(model));
    assert_eq!(overview.escape(model), None);
    assert!(overview.moving.is_none() && overview.searching());
    assert_eq!(overview.escape(model), None);
    assert!(!overview.searching());
    assert_eq!(overview.cards(model).len(), 2);
    assert_eq!(overview.escape(model), Some(Intent::Dismiss));
}

#[test]
fn enter_waits_for_the_input_method_to_commit() {
    let (store, _) = store();
    let model = store.model();
    let mut overview = Overview::open(model, None);
    overview.type_text(model, "app");
    overview.edit(model, |f| f.compose_utf16(None, "ｃ", None));
    assert!(overview.search.composing());
    assert_eq!(overview.enter(model, false), None);
    overview.edit(model, |f| f.replace_utf16(None, "c"));
    assert_eq!(
        overview.enter(model, false),
        Some(Intent::Activate(id(&store, 0xc)))
    );
}

#[test]
fn the_move_menu_filters_destinations_by_typing() {
    let (store, _) = store();
    let model = store.model();
    let mut overview = Overview::open(model, None);
    assert!(overview.begin_move(model));
    overview.type_text(model, "we");
    // Typing in the menu filters destinations, not the search.
    assert!(overview.search.is_empty());
    assert_eq!(overview.destination.text(), "we");
    assert_eq!(
        overview.rail_focus,
        Some(Dest::Existing(WorkspaceId(-1337)))
    );
    // The filter never offers the window's own workspace.
    overview.backspace(model);
    overview.backspace(model);
    overview.type_text(model, "1");
    assert_eq!(overview.rail_focus, None);
    assert_eq!(overview.enter(model, false), None);
}

/// Each pointer action in KEYBOARD_PATHS produces the same intent from the
/// keyboard.
#[test]
fn every_pointer_action_has_a_keyboard_path() {
    let (store, _) = store();
    let model = store.model();
    let a = id(&store, 0xa);
    let two = Dest::Existing(WorkspaceId(2));

    // Click a window ⇔ arrows, Enter.
    let mut keys = Overview::open(model, None);
    keys.step(model, -1);
    assert_eq!(keys.enter(model, false), Some(Intent::Activate(a)));

    // Click a workspace ⇔ Tab to the rail, arrows, Enter.
    let mut keys = Overview::open(model, None);
    keys.tab(true);
    keys.step(model, 1);
    assert_eq!(keys.enter(model, false), Some(Intent::Switch(two)));

    // Drag onto a workspace (plain and Shift) ⇔ Ctrl+Shift+M, choose, Enter.
    for follow in [false, true] {
        let mut pointer = Overview::open(model, None);
        let dropped = pointer.drop_on(model, a, two, follow);
        let mut keys = Overview::open(model, None);
        keys.select(model, a);
        assert!(keys.begin_move(model));
        keys.type_text(model, "2");
        assert_eq!(keys.enter(model, follow), dropped);
    }

    // Click the search field ⇔ Ctrl+K.
    let mut keys = Overview::open(model, None);
    keys.focus_search();
    assert_eq!(keys.region, Region::Search);

    // Click a search result ⇔ type, arrows, Enter.
    let mut keys = Overview::open(model, None);
    keys.type_text(model, "app");
    keys.tab(false);
    keys.tab(false);
    assert_eq!(keys.region, Region::Grid);
    keys.end(model);
    let last = keys.cards(model).last().unwrap().window;
    assert_eq!(keys.enter(model, false), Some(Intent::Activate(last)));
    // Drag a workspace ⇔ Ctrl+Shift+Right on the focused rail tile.
    let mut keys = Overview::open(model, None);
    keys.tab(true);
    assert_eq!(
        keys.reorder_focused(model, 1),
        keys.reorder_workspace(model, WorkspaceId(1), two, true)
    );
    assert_eq!(KEYBOARD_PATHS.len(), 8);
}

#[test]
fn workspace_swap_keeps_layout_identity_and_roundtrips() {
    let (mut store, epoch) = store();
    let before = store.model().clone();
    let mut overview = Overview::open(&before, None);
    let selected = overview.selected;
    assert_eq!(
        overview.reorder_workspace(
            &before,
            WorkspaceId(1),
            Dest::Existing(WorkspaceId(2)),
            false
        ),
        Some(Intent::SwapWorkspaces {
            source: WorkspaceId(1),
            target: WorkspaceId(2)
        })
    );
    for (old, new) in [(1, 999), (2, 1), (999, 2)] {
        let (old, new) = (WorkspaceId(old), WorkspaceId(new));
        store
            .on_event(epoch, Event::WorkspaceIdChanged { old, new })
            .unwrap();
        overview.workspace_id_changed(old, new);
        overview.reconcile(store.model());
    }
    assert_eq!(overview.previewed, Some(WorkspaceId(2)));
    assert_eq!(overview.selected, selected);
    assert_eq!(store.model().monitors[0].active_workspace, WorkspaceId(2));
    assert_eq!(
        store.model().window(selected.unwrap()).unwrap().workspace,
        WorkspaceId(2)
    );
    assert_eq!(store.model().windows_on(WorkspaceId(1)).count(), 1);
    assert_eq!(
        overview
            .tiles(store.model())
            .iter()
            .map(|t| t.label.as_str())
            .collect::<Vec<_>>(),
        ["1", "2", "5", "web", "scratch", "New"]
    );
    for (old, new) in [(2, 999), (1, 2), (999, 1)] {
        store
            .on_event(
                epoch,
                Event::WorkspaceIdChanged {
                    old: WorkspaceId(old),
                    new: WorkspaceId(new),
                },
            )
            .unwrap();
    }
    assert_eq!(store.model(), &before);
}

#[test]
fn workspace_swap_rejects_named_new_and_inflight_targets() {
    let (store, _) = store();
    let model = store.model();
    let mut o = Overview::open(model, None);
    for dest in [
        Dest::Existing(WorkspaceId(-1337)),
        Dest::Existing(WorkspaceId(-98)),
        Dest::NewNumbered(6),
    ] {
        assert_eq!(
            o.reorder_workspace(model, WorkspaceId(1), dest, false),
            None
        );
    }
    o.swapping = true;
    assert_eq!(
        o.reorder_workspace(model, WorkspaceId(1), Dest::Existing(WorkspaceId(2)), true),
        None
    );
}

#[test]
fn reorder_rejects_invalid_filtered_and_no_op_drops_and_keyboard_edges() {
    let (store, _) = store();
    let model = store.model();
    let mut overview = Overview::open(model, None);
    let one = Dest::Existing(WorkspaceId(1));
    let two = Dest::Existing(WorkspaceId(2));
    assert_eq!(
        overview.reorder_workspace(model, WorkspaceId(1), one, true),
        None
    );
    assert_eq!(
        overview.reorder_workspace(model, WorkspaceId(1), two, false),
        Some(Intent::SwapWorkspaces {
            source: WorkspaceId(1),
            target: WorkspaceId(2)
        })
    );
    assert_eq!(
        overview.reorder_workspace(model, WorkspaceId(99), one, true),
        None
    );
    assert_eq!(
        overview.reorder_workspace(model, WorkspaceId(1), Dest::Existing(WorkspaceId(99)), true),
        None
    );
    assert_eq!(overview.reorder_focused(model, 1), None); // grid
    overview.tab(true);
    assert_eq!(overview.reorder_focused(model, -1), None);
    overview.rail_focus = Some(Dest::Existing(WorkspaceId(-98)));
    assert_eq!(overview.reorder_focused(model, 1), None); // New is fixed at end
    overview.hide_special = true;
    assert_eq!(
        overview.reorder_workspace(model, WorkspaceId(-98), one, false),
        None
    );
    overview.type_text(model, "web");
    assert_eq!(
        overview.reorder_workspace(model, WorkspaceId(1), two, true),
        None
    );
    overview.escape(model);
    overview.begin_move(model);
    assert_eq!(
        overview.reorder_workspace(model, WorkspaceId(1), two, true),
        None
    );
}

#[test]
fn labels_name_every_card_and_tile_in_words() {
    let (store, _) = store();
    let model = store.model();
    let overview = Overview::open(model, None);
    let card = &overview.cards(model)[0];
    assert_eq!(card.app, "Appa");
    assert_eq!(
        card.accessible_label(),
        "Appa: title a, workspace 1, on eDP-1"
    );
    let tiles = overview.tiles(model);
    assert_eq!(
        tiles[0].accessible_label(),
        "Workspace 1, 2 windows, active, previewed, on eDP-1"
    );
    assert_eq!(tiles[0].caption(), "Active · 2 windows");
    assert_eq!(
        tiles[4].accessible_label(),
        "Special workspace scratch, 1 window, on eDP-1"
    );
    assert_eq!(tiles[5].accessible_label(), "New workspace");
    assert_eq!(tiles[5].caption(), "Add workspace");
}

#[test]
fn previewing_announces_the_workspace() {
    let (store, _) = store();
    let model = store.model();
    let mut overview = Overview::open(model, None);
    overview.tab(true);
    overview.step(model, 1);
    assert_eq!(
        overview.announcement.as_ref().unwrap().text,
        "Previewing 2, 1 window"
    );
}

#[test]
fn rail_covers_and_captures_cover_every_workspace() {
    let (store, _) = store();
    let model = store.model();
    let overview = Overview::open(model, None);
    let tiles = overview.tiles(model);
    assert_eq!(tiles[0].cover, Some(id(&store, 0xa)));
    assert_eq!(tiles[1].cover, Some(id(&store, 0xc)));
    assert_eq!(tiles[2].cover, None); // 5 is empty
    assert_eq!(tiles.last().unwrap().cover, None);
    // Cards first, then the other workspaces' covers, once each.
    let wanted: Vec<u64> = overview.wanted(model).iter().map(|w| w.address).collect();
    assert_eq!(wanted, [0xa, 0xb, 0xc, 0xd]);
}
