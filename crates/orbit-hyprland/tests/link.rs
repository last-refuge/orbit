//! The link thread against fake Hyprland sockets: it subscribes, picks the
//! dispatch syntax from the version, answers queries, and reports a
//! dispatch's events before its result (the store's ordering contract).

use std::{
    io::{Read, Write},
    os::unix::net::{UnixListener, UnixStream},
    path::PathBuf,
    sync::mpsc,
    thread,
    time::Duration,
};

use orbit_core::{Event, Kinds, WorkspaceId};
use orbit_hyprland::{
    Instance,
    control::{Control, Dest, Syntax},
    link::{FromLink, Link, ToLink},
};

fn temp_dir() -> PathBuf {
    let dir = std::env::temp_dir().join(format!("orbit-link-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn reports_events_before_the_dispatch_result() {
    let dir = temp_dir();
    let requests = UnixListener::bind(dir.join(".socket.sock")).unwrap();
    let events = UnixListener::bind(dir.join(".socket2.sock")).unwrap();
    let (event_conn_tx, event_conn_rx) = mpsc::channel::<UnixStream>();
    thread::spawn(move || {
        let (conn, _) = events.accept().unwrap();
        event_conn_tx.send(conn).unwrap();
    });
    let (seen_tx, seen) = mpsc::channel::<String>();
    thread::spawn(move || {
        let mut event_conn: Option<UnixStream> = None;
        for conn in requests.incoming() {
            let mut conn = conn.unwrap();
            let mut buf = [0u8; 4096];
            let n = conn.read(&mut buf).unwrap();
            let message = String::from_utf8_lossy(&buf[..n]).into_owned();
            seen_tx.send(message.clone()).unwrap();
            let reply = match message.as_str() {
                "j/version" => r#"{"version":"0.56.2","tag":"v0.56.2"}"#.to_string(),
                "j/workspaces" => r#"[{"id":91,"name":"91","monitor":"DP-1"}]"#.to_string(),
                m if m.starts_with("dispatch ") => {
                    // Hyprland writes the move's events, then replies.
                    let events = event_conn.get_or_insert_with(|| event_conn_rx.recv().unwrap());
                    events
                        .write_all(b"movewindow>>abc,91\nmovewindowv2>>abc,91,91\n")
                        .unwrap();
                    "ok".to_string()
                }
                _ => "[]".to_string(),
            };
            conn.write_all(reply.as_bytes()).unwrap();
        }
    });

    let (out_tx, out) = mpsc::channel::<FromLink>();
    let instance = Instance {
        signature: "test".into(),
        dir: dir.clone(),
    };
    let link = Link::spawn(
        move || Ok(instance.clone()),
        move |m| {
            let _ = out_tx.send(m);
        },
    )
    .unwrap();
    let next = || {
        out.recv_timeout(Duration::from_secs(3))
            .expect("link output")
    };

    let FromLink::Connected {
        link: id, syntax, ..
    } = next()
    else {
        panic!("expected Connected first");
    };
    assert_eq!(syntax, Syntax::Lua);
    assert_eq!(seen.recv().unwrap(), "j/version");

    link.send(ToLink::Query(Kinds::WORKSPACES));
    let FromLink::Refresh { link: l, refresh } = next() else {
        panic!("expected a refresh");
    };
    assert_eq!(l, id);
    assert_eq!(refresh.kinds(), Kinds::WORKSPACES);
    assert_eq!(refresh.workspaces.unwrap()[0].id, WorkspaceId(91));

    link.send(ToLink::Dispatch {
        ticket: 7,
        control: Control::MoveSilent {
            address: 0xabc,
            dest: Dest::Numbered(91),
        },
    });
    // The legacy movewindow line is ignored; the v2 one arrives first.
    match next() {
        FromLink::Event {
            event: Event::WindowMoved { address, .. },
            ..
        } => assert_eq!(address, 0xabc),
        other => panic!("expected the move event before the result, got {other:?}"),
    }
    match next() {
        FromLink::Dispatched { ticket, result, .. } => {
            assert_eq!(ticket, 7);
            assert_eq!(result, Ok(()));
        }
        other => panic!("expected the dispatch result, got {other:?}"),
    }
    assert_eq!(seen.recv().unwrap(), "j/workspaces");
    assert_eq!(
        seen.recv().unwrap(),
        "dispatch hl.dsp.window.move({ window = 'address:0xabc', workspace = '91', follow = false })"
    );
    drop(link);
}
