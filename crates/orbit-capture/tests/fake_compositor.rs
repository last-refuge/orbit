//! Drives the capturer against a minimal in-process compositor that speaks
//! the same protocols as Hyprland 0.56: toplevel list, toplevel capture
//! sources, ext-image-copy-capture with wl_shm buffers and the Hyprland
//! address mapping. Proves the request/event flow and pixel handling without
//! a real session.

#![allow(unsafe_code)] // only inside the generated protocol tables below

use std::{
    collections::HashMap,
    fs::File,
    os::{fd::OwnedFd, unix::fs::FileExt, unix::net::UnixStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use orbit_capture::{Capturer, Failure};
use wayland_protocols::ext::{
    foreign_toplevel_list::v1::server::{
        ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
        ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
    },
    image_capture_source::v1::server::{
        ext_foreign_toplevel_image_capture_source_manager_v1::{
            self, ExtForeignToplevelImageCaptureSourceManagerV1,
        },
        ext_image_capture_source_v1::{self, ExtImageCaptureSourceV1},
    },
    image_copy_capture::v1::server::{
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
        ext_image_copy_capture_manager_v1::{self, ExtImageCopyCaptureManagerV1},
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};
use wayland_server::{
    Client, DataInit, Dispatch, Display, DisplayHandle, GlobalDispatch, New, Resource,
    backend::{ClientData, ClientId, DisconnectReason, ObjectId},
    protocol::{wl_buffer, wl_output, wl_shm, wl_shm_pool},
};

#[allow(
    dead_code,
    non_camel_case_types,
    non_upper_case_globals,
    unused_imports,
    clippy::all
)]
mod mapping {
    pub mod server {
        use wayland_protocols::ext::foreign_toplevel_list::v1::server::*;
        use wayland_protocols_wlr::foreign_toplevel::v1::server::*;
        use wayland_server;
        use wayland_server::protocol::*;

        pub mod __interfaces {
            use wayland_protocols::ext::foreign_toplevel_list::v1::server::__interfaces::*;
            use wayland_protocols_wlr::foreign_toplevel::v1::server::__interfaces::*;
            use wayland_server::protocol::__interfaces::*;
            wayland_scanner::generate_interfaces!("protocols/hyprland-toplevel-mapping-v1.xml");
        }
        use self::__interfaces::*;

        wayland_scanner::generate_server_code!("protocols/hyprland-toplevel-mapping-v1.xml");
    }
}
use mapping::server::{
    hyprland_toplevel_mapping_manager_v1::{self, HyprlandToplevelMappingManagerV1},
    hyprland_toplevel_window_mapping_handle_v1::HyprlandToplevelWindowMappingHandleV1,
};

const WIDTH: u32 = 64;
const HEIGHT: u32 = 32;
const ADDRESS: u64 = 0x5555_dead_bee0;

/// A window the fake compositor lists.
#[derive(Clone)]
struct Window {
    identifier: &'static str,
    app_id: &'static str,
    /// Capture of this window is refused (session stopped immediately).
    denied: bool,
    /// Never damaged: only a session's first frame is answered, as Hyprland
    /// does for a static window.
    static_content: bool,
    /// The window closes as soon as someone tries to capture it.
    closes: bool,
}

struct Pool {
    fd: Arc<OwnedFd>,
}

struct Buffer {
    fd: Arc<OwnedFd>,
    offset: i32,
    width: i32,
    height: i32,
    stride: i32,
    format: wl_shm::Format,
}

#[derive(Default)]
struct FrameData {
    buffer: Mutex<Option<wl_buffer::WlBuffer>>,
    /// Waits for damage that never comes.
    silent: bool,
}

struct State {
    windows: Vec<Window>,
    handles: HashMap<&'static str, ExtForeignToplevelHandleV1>,
    frames_per_session: HashMap<ObjectId, usize>,
}

struct NoopClient;
impl ClientData for NoopClient {
    fn initialized(&self, _: ClientId) {}
    fn disconnected(&self, _: ClientId, _: DisconnectReason) {}
}

fn spawn_compositor(windows: Vec<Window>) -> (UnixStream, Arc<AtomicBool>) {
    let (server_end, client_end) = UnixStream::pair().unwrap();
    let stop = Arc::new(AtomicBool::new(false));
    let stop_flag = stop.clone();
    thread::spawn(move || {
        let mut display: Display<State> = Display::new().unwrap();
        let dh = display.handle();
        dh.create_global::<State, wl_shm::WlShm, ()>(1, ());
        dh.create_global::<State, ExtForeignToplevelListV1, ()>(1, ());
        dh.create_global::<State, ExtForeignToplevelImageCaptureSourceManagerV1, ()>(1, ());
        dh.create_global::<State, ExtImageCopyCaptureManagerV1, ()>(1, ());
        dh.create_global::<State, HyprlandToplevelMappingManagerV1, ()>(1, ());
        display
            .handle()
            .insert_client(server_end, Arc::new(NoopClient))
            .unwrap();
        let mut state = State {
            windows,
            handles: HashMap::new(),
            frames_per_session: HashMap::new(),
        };
        while !stop_flag.load(Ordering::Relaxed) {
            if display.dispatch_clients(&mut state).is_err() {
                break;
            }
            let _ = display.flush_clients();
            thread::sleep(Duration::from_millis(1));
        }
    });
    (client_end, stop)
}

fn capturer(windows: Vec<Window>) -> (Capturer, Arc<AtomicBool>) {
    let (stream, stop) = spawn_compositor(windows);
    let connection = wayland_client::Connection::from_socket(stream).unwrap();
    (Capturer::with_connection(connection).unwrap(), stop)
}

fn window(identifier: &'static str, app_id: &'static str) -> Window {
    Window {
        identifier,
        app_id,
        denied: false,
        static_content: false,
        closes: false,
    }
}

#[test]
fn lists_maps_and_captures_a_toplevel() {
    let (mut capturer, stop) = capturer(vec![
        window("id-a", "dev.orbit.fixture.tiled-a"),
        window("id-b", "org.example.other"),
    ]);
    assert!(capturer.has_address_mapping());
    let toplevels = capturer.toplevels().unwrap();
    assert_eq!(toplevels.len(), 2);
    let a = toplevels.iter().find(|t| t.identifier == "id-a").unwrap();
    assert_eq!(a.app_id, "dev.orbit.fixture.tiled-a");
    assert_eq!(a.address, Some(ADDRESS));
    assert_eq!(a.address_hex().as_deref(), Some("0x5555deadbee0"));

    let mut session = capturer.open(a).unwrap();
    assert_eq!(
        (session.constraints.width, session.constraints.height),
        (WIDTH, HEIGHT)
    );
    for _ in 0..3 {
        let capture = capturer
            .grab(&mut session, Duration::from_millis(500))
            .map_err(|(_, e)| e)
            .unwrap();
        assert_eq!(
            (capture.width, capture.height, capture.stride),
            (WIDTH, HEIGHT, WIDTH * 4)
        );
        assert_eq!(capture.format, "Xrgb8888");
        assert_eq!(capture.transform, "Normal");
        // The fake paints B = x, G = y, R = 0x80 with a garbage X byte.
        let frame = &capture.frame;
        let at = |x: u32, y: u32| {
            let i = ((y * WIDTH + x) * 4) as usize;
            &frame.bgra[i..i + 4]
        };
        assert_eq!(at(0, 0), [0, 0, 0x80, 0xff]);
        assert_eq!(at(5, 7), [5, 7, 0x80, 0xff]);
        assert_eq!(at(WIDTH - 1, HEIGHT - 1), [63, 31, 0x80, 0xff]);
    }
    capturer.close(session);
    stop.store(true, Ordering::Relaxed);
}

#[test]
fn denied_capture_is_an_error_not_a_hang() {
    let (mut capturer, stop) = capturer(vec![Window {
        denied: true,
        ..window("id-x", "dev.orbit.fixture.denied")
    }]);
    let toplevels = capturer.toplevels().unwrap();
    let error = capturer.capture_once(&toplevels[0]).unwrap_err();
    assert!(format!("{error:#}").contains("stopped"), "{error:#}");
    stop.store(true, Ordering::Relaxed);
}

#[test]
fn repeat_frames_wait_for_damage() {
    let (mut capturer, stop) = capturer(vec![Window {
        static_content: true,
        ..window("id-s", "dev.orbit.fixture.static")
    }]);
    let toplevels = capturer.toplevels().unwrap();
    let mut session = capturer.open(&toplevels[0]).unwrap();
    assert!(
        capturer
            .grab(&mut session, Duration::from_millis(50))
            .is_ok()
    );
    let (failure, _) = capturer
        .grab(&mut session, Duration::from_millis(50))
        .unwrap_err();
    assert!(matches!(failure, Failure::Unchanged), "{failure:?}");
    capturer.close(session);
    // A fresh session gets a first frame again.
    assert!(capturer.capture_once(&toplevels[0]).is_ok());
    stop.store(true, Ordering::Relaxed);
}

#[test]
fn a_window_closing_mid_capture_is_an_error_not_a_crash() {
    let (mut capturer, stop) = capturer(vec![Window {
        closes: true,
        ..window("id-c", "dev.orbit.fixture.closing")
    }]);
    let stale = capturer.toplevels().unwrap();
    assert!(capturer.capture_once(&stale[0]).is_err());
    // The handle is gone now; reusing the stale copy must not panic.
    let error = capturer.capture_once(&stale[0]).unwrap_err();
    assert!(format!("{error:#}").contains("closed"), "{error:#}");
    assert!(capturer.toplevels().unwrap().is_empty());
    stop.store(true, Ordering::Relaxed);
}

// --- fake compositor -------------------------------------------------------

macro_rules! bind_plain {
    ($($ty:ty),*) => {$(
        impl GlobalDispatch<$ty, ()> for State {
            fn bind(_: &mut Self, _: &DisplayHandle, _: &Client, resource: New<$ty>, _: &(), init: &mut DataInit<'_, Self>) {
                init.init(resource, ());
            }
        }
    )*};
}
bind_plain!(
    wl_shm::WlShm,
    ExtForeignToplevelImageCaptureSourceManagerV1,
    ExtImageCopyCaptureManagerV1,
    HyprlandToplevelMappingManagerV1
);

impl GlobalDispatch<ExtForeignToplevelListV1, ()> for State {
    fn bind(
        state: &mut Self,
        dh: &DisplayHandle,
        client: &Client,
        resource: New<ExtForeignToplevelListV1>,
        _: &(),
        init: &mut DataInit<'_, Self>,
    ) {
        let list = init.init(resource, ());
        for window in state.windows.clone() {
            let handle = client
                .create_resource::<ExtForeignToplevelHandleV1, Window, State>(dh, 1, window.clone())
                .unwrap();
            list.toplevel(&handle);
            handle.identifier(window.identifier.into());
            handle.app_id(window.app_id.into());
            handle.title("private title".into());
            handle.done();
            state.handles.insert(window.identifier, handle);
        }
    }
}

impl Dispatch<wl_shm::WlShm, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_shm::WlShm,
        request: wl_shm::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm::Request::CreatePool { id, fd, .. } = request {
            init.init(id, Pool { fd: Arc::new(fd) });
        }
    }
}

impl Dispatch<wl_shm_pool::WlShmPool, Pool> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_shm_pool::WlShmPool,
        request: wl_shm_pool::Request,
        pool: &Pool,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let wl_shm_pool::Request::CreateBuffer {
            id,
            offset,
            width,
            height,
            stride,
            format,
        } = request
        {
            init.init(
                id,
                Buffer {
                    fd: pool.fd.clone(),
                    offset,
                    width,
                    height,
                    stride,
                    format: format.into_result().unwrap(),
                },
            );
        }
    }
}

impl Dispatch<wl_buffer::WlBuffer, Buffer> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &wl_buffer::WlBuffer,
        _: wl_buffer::Request,
        _: &Buffer,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        list: &ExtForeignToplevelListV1,
        request: ext_foreign_toplevel_list_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Request::Stop = request {
            list.finished();
        }
    }
}

impl Dispatch<ExtForeignToplevelHandleV1, Window> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ExtForeignToplevelHandleV1,
        _: ext_foreign_toplevel_handle_v1::Request,
        _: &Window,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ExtForeignToplevelImageCaptureSourceManagerV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ExtForeignToplevelImageCaptureSourceManagerV1,
        request: ext_foreign_toplevel_image_capture_source_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let ext_foreign_toplevel_image_capture_source_manager_v1::Request::CreateSource {
            source,
            toplevel_handle,
        } = request
        {
            let window = toplevel_handle.data::<Window>().unwrap().clone();
            init.init(source, window);
        }
    }
}

impl Dispatch<ExtImageCaptureSourceV1, Window> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &ExtImageCaptureSourceV1,
        _: ext_image_capture_source_v1::Request,
        _: &Window,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}

impl Dispatch<ExtImageCopyCaptureManagerV1, ()> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        _: &ExtImageCopyCaptureManagerV1,
        request: ext_image_copy_capture_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let ext_image_copy_capture_manager_v1::Request::CreateSession {
            session, source, ..
        } = request
        {
            let window = source.data::<Window>().unwrap().clone();
            let session = init.init(session, window.clone());
            if window.closes {
                state.handles[window.identifier].closed();
            }
            if window.denied || window.closes {
                session.stopped();
                return;
            }
            session.buffer_size(WIDTH, HEIGHT);
            session.shm_format(wl_shm::Format::Rgb565);
            session.shm_format(wl_shm::Format::Xrgb8888);
            session.done();
        }
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, Window> for State {
    fn request(
        state: &mut Self,
        _: &Client,
        session: &ExtImageCopyCaptureSessionV1,
        request: ext_image_copy_capture_session_v1::Request,
        window: &Window,
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let ext_image_copy_capture_session_v1::Request::CreateFrame { frame } = request {
            let count = state.frames_per_session.entry(session.id()).or_default();
            let silent = window.static_content && *count > 0;
            *count += 1;
            init.init(
                frame,
                FrameData {
                    silent,
                    ..Default::default()
                },
            );
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, FrameData> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        frame: &ExtImageCopyCaptureFrameV1,
        request: ext_image_copy_capture_frame_v1::Request,
        data: &FrameData,
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Request;
        match request {
            Request::AttachBuffer { buffer } => *data.buffer.lock().unwrap() = Some(buffer),
            Request::Capture if data.silent => {}
            Request::Capture => {
                let buffer = data.buffer.lock().unwrap().clone().unwrap();
                let b = buffer.data::<Buffer>().unwrap();
                if b.width as u32 != WIDTH
                    || b.height as u32 != HEIGHT
                    || b.format != wl_shm::Format::Xrgb8888
                {
                    frame.failed(ext_image_copy_capture_frame_v1::FailureReason::BufferConstraints);
                    return;
                }
                let file = File::from(b.fd.try_clone().unwrap());
                for y in 0..b.height {
                    let row: Vec<u8> = (0..b.width)
                        .flat_map(|x| [x as u8, y as u8, 0x80, 0x37])
                        .collect();
                    let at = (b.offset + y * b.stride) as u64;
                    file.write_all_at(&row, at).unwrap();
                }
                frame.transform(wl_output::Transform::Normal);
                frame.ready();
            }
            _ => {}
        }
    }
}

impl Dispatch<HyprlandToplevelMappingManagerV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &HyprlandToplevelMappingManagerV1,
        request: hyprland_toplevel_mapping_manager_v1::Request,
        _: &(),
        _: &DisplayHandle,
        init: &mut DataInit<'_, Self>,
    ) {
        if let hyprland_toplevel_mapping_manager_v1::Request::GetWindowForToplevel {
            handle, ..
        } = request
        {
            let handle = init.init(handle, ());
            handle.window_address((ADDRESS >> 32) as u32, ADDRESS as u32);
        }
    }
}

impl Dispatch<HyprlandToplevelWindowMappingHandleV1, ()> for State {
    fn request(
        _: &mut Self,
        _: &Client,
        _: &HyprlandToplevelWindowMappingHandleV1,
        _: mapping::server::hyprland_toplevel_window_mapping_handle_v1::Request,
        _: &(),
        _: &DisplayHandle,
        _: &mut DataInit<'_, Self>,
    ) {
    }
}
