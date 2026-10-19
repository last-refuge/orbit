//! Per-window capture over `ext-image-copy-capture-v1` with shared-memory
//! buffers (Gate B, ORB-004), plus a one-shot output capture for the
//! overview's frosted backdrop (ORB-008).
//!
//! The capturer owns its own Wayland connection and event queue: object ids
//! from GPUI's connection mean nothing here. Toplevels come from
//! `ext-foreign-toplevel-list-v1`; Hyprland window addresses come from
//! `hyprland-toplevel-mapping-v1` when the compositor offers it. Window titles
//! are never stored.

use std::{
    collections::HashMap,
    fs::File,
    os::{fd::AsFd, unix::fs::FileExt},
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail};
use rustix::{
    event::{PollFd, PollFlags, Timespec, poll},
    fs::{MemfdFlags, ftruncate, memfd_create},
};
use serde::Serialize;
use wayland_client::{
    Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum,
    backend::ObjectId,
    event_created_child,
    globals::{GlobalListContents, registry_queue_init},
    protocol::{wl_buffer, wl_output, wl_registry, wl_shm, wl_shm_pool},
};
use wayland_protocols::ext::{
    foreign_toplevel_list::v1::client::{
        ext_foreign_toplevel_handle_v1::{self, ExtForeignToplevelHandleV1},
        ext_foreign_toplevel_list_v1::{self, ExtForeignToplevelListV1},
    },
    image_capture_source::v1::client::{
        ext_foreign_toplevel_image_capture_source_manager_v1::ExtForeignToplevelImageCaptureSourceManagerV1,
        ext_image_capture_source_v1::ExtImageCaptureSourceV1,
        ext_output_image_capture_source_manager_v1::ExtOutputImageCaptureSourceManagerV1,
    },
    image_copy_capture::v1::client::{
        ext_image_copy_capture_frame_v1::{self, ExtImageCopyCaptureFrameV1},
        ext_image_copy_capture_manager_v1::{ExtImageCopyCaptureManagerV1, Options},
        ext_image_copy_capture_session_v1::{self, ExtImageCopyCaptureSessionV1},
    },
};

use crate::{
    frame::{Frame, pick_format},
    hyprland_mapping::client::{
        hyprland_toplevel_mapping_manager_v1::HyprlandToplevelMappingManagerV1,
        hyprland_toplevel_window_mapping_handle_v1::{self, HyprlandToplevelWindowMappingHandleV1},
    },
};

/// How long to wait for constraints or a first frame.
pub const TIMEOUT: Duration = Duration::from_secs(2);

/// A window the compositor lists, without its title.
#[derive(Clone, Debug, Serialize)]
pub struct Toplevel {
    /// Stable for the window's lifetime (ext-foreign-toplevel-list).
    pub identifier: String,
    pub app_id: String,
    /// Hyprland's window address, when the mapping protocol resolved it.
    pub address: Option<u64>,
    #[serde(skip)]
    handle: ExtForeignToplevelHandleV1,
}

impl Toplevel {
    /// Formats the address as `hyprctl` does (`0x55d0c1a2b3c0`).
    pub fn address_hex(&self) -> Option<String> {
        self.address.map(|a| format!("{a:#x}"))
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Failure {
    Unknown,
    BufferConstraints,
    Stopped,
    Timeout,
    /// A repeat frame did not arrive in time. Compositors (Hyprland among
    /// them) answer repeat captures only once the window has new damage, so
    /// this means "unchanged", and the previous frame is still current.
    Unchanged,
}

/// How one capture went, for evidence and budgets.
#[derive(Clone, Debug, Serialize)]
pub struct Timing {
    /// Request to `ready`, including the compositor's render of the window.
    pub capture_ms: f64,
    /// Reading the buffer and converting it to BGRA.
    pub convert_ms: f64,
}

/// The constraints the compositor announced for a session.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Constraints {
    pub width: u32,
    pub height: u32,
    #[serde(serialize_with = "serialize_formats")]
    pub shm_formats: Vec<wl_shm::Format>,
    pub dmabuf_formats: usize,
}

fn serialize_formats<S: serde::Serializer>(
    formats: &[wl_shm::Format],
    serializer: S,
) -> Result<S::Ok, S::Error> {
    serializer.collect_seq(formats.iter().map(|f| format!("{f:?}")))
}

/// An open capture session for one window, with its reusable buffer.
pub struct Session {
    session: ExtImageCopyCaptureSessionV1,
    source: ExtImageCaptureSourceV1,
    buffer: Option<ShmBuffer>,
    pub constraints: Constraints,
    /// Time from `create_session` to the first complete constraint set.
    pub setup_ms: f64,
    /// Frames delivered so far on this session.
    pub frames: usize,
}

struct ShmBuffer {
    file: File,
    pool: wl_shm_pool::WlShmPool,
    buffer: wl_buffer::WlBuffer,
    width: u32,
    height: u32,
    stride: u32,
    format: wl_shm::Format,
}

impl ShmBuffer {
    fn destroy(self) {
        self.buffer.destroy();
        self.pool.destroy();
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct Capture {
    #[serde(skip)]
    pub frame: Frame,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub format: String,
    pub transform: String,
    pub timing: Timing,
}

pub struct Capturer {
    queue: EventQueue<State>,
    state: State,
}

impl Drop for Capturer {
    fn drop(&mut self) {
        // A protocol error poisons the connection; sending then would panic
        // again inside unwinding and abort.
        if !std::thread::panicking() && self.state.list.is_alive() {
            self.state.list.stop();
            let _ = self.queue.flush();
        }
    }
}

impl Capturer {
    /// Connects to `$WAYLAND_DISPLAY` and binds the capture globals. Fails
    /// when the compositor lacks the toplevel capture path.
    pub fn connect() -> Result<Self> {
        Self::with_connection(Connection::connect_to_env().context("connecting to Wayland")?)
    }

    /// Like [`Capturer::connect`] on an existing connection (tests use a
    /// socket pair to a fake compositor).
    pub fn with_connection(connection: Connection) -> Result<Self> {
        let (globals, queue) =
            registry_queue_init::<State>(&connection).context("reading the registry")?;
        let qh = queue.handle();
        let bind_err = |name: &str| format!("compositor does not offer {name}");

        let mut state = State {
            shm: globals
                .bind(&qh, 1..=1, ())
                .with_context(|| bind_err("wl_shm"))?,
            list: globals
                .bind(&qh, 1..=1, ())
                .with_context(|| bind_err("ext_foreign_toplevel_list_v1"))?,
            sources: globals.bind(&qh, 1..=1, ()).with_context(|| {
                bind_err("ext_foreign_toplevel_image_capture_source_manager_v1")
            })?,
            copy: globals
                .bind(&qh, 1..=1, ())
                .with_context(|| bind_err("ext_image_copy_capture_manager_v1"))?,
            mapping: globals.bind(&qh, 1..=1, ()).ok(),
            output_sources: globals.bind(&qh, 1..=1, ()).ok(),
            outputs: Vec::new(),
            toplevels: Vec::new(),
            pending: HashMap::new(),
            mappings: HashMap::new(),
            sessions: HashMap::new(),
            frames: HashMap::new(),
        };
        // Outputs, for the backdrop capture (v4 for their names).
        let registry = globals.registry().clone();
        let output_globals: Vec<(u32, u32)> = globals.contents().with_list(|list| {
            list.iter()
                .filter(|g| g.interface == "wl_output")
                .map(|g| (g.name, g.version.min(4)))
                .collect()
        });
        for (name, version) in output_globals {
            let output: wl_output::WlOutput = registry.bind(name, version, &qh, ());
            state.outputs.push(OutputInfo { output, name: None });
        }
        let mut capturer = Self { queue, state };
        // The list announces existing toplevels after bind; their details
        // follow in the same burst.
        capturer.roundtrip()?;
        capturer.roundtrip()?;
        capturer.resolve_addresses()?;
        Ok(capturer)
    }

    pub fn has_address_mapping(&self) -> bool {
        self.state.mapping.is_some()
    }

    /// Whether the compositor offers output capture (for the backdrop).
    pub fn has_output_capture(&self) -> bool {
        self.state.output_sources.is_some() && !self.state.outputs.is_empty()
    }

    /// Captures one frame of the output called `name` (the focused one
    /// when `None`, meaning the first listed). The frame lives only in
    /// memory; callers downscale and blur it for the overview's backdrop.
    pub fn capture_output(&mut self, name: Option<&str>) -> Result<Capture> {
        let manager = self
            .state
            .output_sources
            .clone()
            .context("compositor does not offer ext_output_image_capture_source_manager_v1")?;
        self.roundtrip()?;
        let output = self
            .state
            .outputs
            .iter()
            .find(|o| name.is_none_or(|n| o.name.as_deref() == Some(n)))
            .map(|o| o.output.clone())
            .with_context(|| format!("no output named {}", name.unwrap_or("(any)")))?;
        let qh = self.queue.handle();
        let source = manager.create_source(&output, &qh, ());
        let mut session = self.open_source(source)?;
        let result = self.grab(&mut session, TIMEOUT);
        self.close(session);
        result.map_err(|(_, e)| e)
    }

    /// Windows currently listed by the compositor.
    pub fn toplevels(&mut self) -> Result<Vec<Toplevel>> {
        self.roundtrip()?;
        self.resolve_addresses()?;
        Ok(self.state.toplevels.clone())
    }

    fn roundtrip(&mut self) -> Result<()> {
        self.queue
            .roundtrip(&mut self.state)
            .context("Wayland roundtrip")?;
        Ok(())
    }

    fn resolve_addresses(&mut self) -> Result<()> {
        let Some(mapping) = self.state.mapping.clone() else {
            return Ok(());
        };
        let qh = self.queue.handle();
        let mut requested = Vec::new();
        for toplevel in self.state.toplevels.iter().filter(|t| t.address.is_none()) {
            let handle = mapping.get_window_for_toplevel(&toplevel.handle, &qh, ());
            requested.push((toplevel.identifier.clone(), handle));
        }
        if requested.is_empty() {
            return Ok(());
        }
        self.roundtrip()?;
        for (identifier, handle) in requested {
            if let Some(Some(address)) = self.state.mappings.remove(&handle.id())
                && let Some(t) = self
                    .state
                    .toplevels
                    .iter_mut()
                    .find(|t| t.identifier == identifier)
            {
                t.address = Some(address);
            }
            handle.destroy();
        }
        Ok(())
    }

    /// Opens a session for `toplevel` and waits for its buffer constraints.
    pub fn open(&mut self, toplevel: &Toplevel) -> Result<Session> {
        // A handle destroyed after `closed` would go out as a null argument,
        // which wayland-client treats as a fatal bug.
        if !toplevel.handle.is_alive() {
            bail!("the window has closed");
        }
        let qh = self.queue.handle();
        let source = self.state.sources.create_source(&toplevel.handle, &qh, ());
        self.open_source(source)
    }

    /// Opens a session on any capture source and waits for its constraints.
    fn open_source(&mut self, source: ExtImageCaptureSourceV1) -> Result<Session> {
        let qh = self.queue.handle();
        let started = Instant::now();
        let session = self
            .state
            .copy
            .create_session(&source, Options::empty(), &qh, ());
        self.state
            .sessions
            .insert(session.id(), SessionState::default());
        let id = session.id();
        let ready = self.dispatch_until(TIMEOUT, |state| {
            let s = &state.sessions[&id];
            s.done || s.stopped
        });
        let mut opened = Session {
            session,
            source,
            buffer: None,
            constraints: Constraints::default(),
            setup_ms: ms(started.elapsed()),
            frames: 0,
        };
        if let Err(error) = ready {
            self.close(opened);
            return Err(error.context("waiting for capture constraints"));
        }
        let s = &self.state.sessions[&id];
        if s.stopped {
            self.close(opened);
            bail!("the compositor stopped the capture session (window gone or capture denied)");
        }
        opened.constraints = s.constraints.clone();
        Ok(opened)
    }

    /// Captures one frame, reusing the session's buffer when the constraints
    /// still fit. The first frame of a session waits up to [`TIMEOUT`];
    /// repeat frames wait `repeat_wait` and report [`Failure::Unchanged`]
    /// when the window had no new damage by then.
    pub fn grab(
        &mut self,
        session: &mut Session,
        repeat_wait: Duration,
    ) -> Result<Capture, (Failure, anyhow::Error)> {
        let qh = self.queue.handle();
        let state = &self.state.sessions[&session.session.id()];
        if state.stopped {
            return Err((Failure::Stopped, anyhow!("session stopped")));
        }
        session.constraints = state.constraints.clone();
        let c = &session.constraints;
        let format = pick_format(&c.shm_formats).ok_or_else(|| {
            (
                Failure::BufferConstraints,
                anyhow!("no supported shm format among {:?}", c.shm_formats),
            )
        })?;
        let fits = session
            .buffer
            .as_ref()
            .is_some_and(|b| b.width == c.width && b.height == c.height && b.format == format);
        if !fits {
            if let Some(old) = session.buffer.take() {
                old.destroy();
            }
            session.buffer = Some(
                self.allocate(c.width, c.height, format)
                    .map_err(|e| (Failure::Unknown, e))?,
            );
        }
        let buffer = session.buffer.as_ref().expect("allocated above");

        let started = Instant::now();
        let frame = session.session.create_frame(&qh, ());
        let frame_id = frame.id();
        self.state
            .frames
            .insert(frame_id.clone(), FrameState::default());
        frame.attach_buffer(&buffer.buffer);
        frame.damage_buffer(0, 0, buffer.width as i32, buffer.height as i32);
        frame.capture();
        let wait = if session.frames == 0 {
            TIMEOUT
        } else {
            repeat_wait
        };
        let waited = self.dispatch_until(wait, |s| {
            let f = &s.frames[&frame_id];
            f.ready || f.failed.is_some()
        });
        let capture_ms = ms(started.elapsed());
        frame.destroy();
        let result = self.state.frames.remove(&frame_id).unwrap_or_default();
        if let Err(error) = waited {
            let failure = if session.frames == 0 {
                Failure::Timeout
            } else {
                Failure::Unchanged
            };
            return Err((failure, error));
        }
        if let Some(reason) = result.failed {
            let failure = match reason {
                WEnum::Value(ext_image_copy_capture_frame_v1::FailureReason::BufferConstraints) => {
                    Failure::BufferConstraints
                }
                WEnum::Value(ext_image_copy_capture_frame_v1::FailureReason::Stopped) => {
                    Failure::Stopped
                }
                _ => Failure::Unknown,
            };
            return Err((failure.clone(), anyhow!("capture failed: {failure:?}")));
        }

        let converting = Instant::now();
        let mut data = vec![0u8; buffer.stride as usize * buffer.height as usize];
        buffer
            .file
            .read_exact_at(&mut data, 0)
            .map_err(|e| (Failure::Unknown, anyhow!("reading the capture buffer: {e}")))?;
        let frame = Frame::from_shm(&data, buffer.width, buffer.height, buffer.stride, format)
            .map_err(|e| (Failure::Unknown, e))?;
        session.frames += 1;
        Ok(Capture {
            width: buffer.width,
            height: buffer.height,
            stride: buffer.stride,
            format: format!("{format:?}"),
            transform: result
                .transform
                .map(|t| match t {
                    WEnum::Value(t) => format!("{t:?}"),
                    WEnum::Unknown(raw) => format!("Unknown({raw})"),
                })
                .unwrap_or_else(|| "Normal".into()),
            timing: Timing {
                capture_ms,
                convert_ms: ms(converting.elapsed()),
            },
            frame,
        })
    }

    /// Releases the session, its source and its buffer.
    pub fn close(&mut self, mut session: Session) {
        if let Some(buffer) = session.buffer.take() {
            buffer.destroy();
        }
        self.state.sessions.remove(&session.session.id());
        session.session.destroy();
        session.source.destroy();
        let _ = self.queue.flush();
    }

    /// Opens a session, grabs one frame and closes it.
    pub fn capture_once(&mut self, toplevel: &Toplevel) -> Result<Capture> {
        let mut session = self.open(toplevel)?;
        let result = self.grab(&mut session, TIMEOUT);
        self.close(session);
        result.map_err(|(_, e)| e)
    }

    fn allocate(&mut self, width: u32, height: u32, format: wl_shm::Format) -> Result<ShmBuffer> {
        if width == 0 || height == 0 || width > 16384 || height > 16384 {
            bail!("refusing a {width}x{height} capture buffer");
        }
        let stride = width * 4;
        let size = stride as u64 * u64::from(height);
        let fd = memfd_create("orbit-capture", MemfdFlags::CLOEXEC).context("memfd_create")?;
        ftruncate(&fd, size).context("sizing the capture buffer")?;
        let qh = self.queue.handle();
        let pool = self.state.shm.create_pool(fd.as_fd(), size as i32, &qh, ());
        let buffer = pool.create_buffer(
            0,
            width as i32,
            height as i32,
            stride as i32,
            format,
            &qh,
            (),
        );
        Ok(ShmBuffer {
            file: File::from(fd),
            pool,
            buffer,
            width,
            height,
            stride,
            format,
        })
    }

    /// Dispatches events until `done` holds, or fails after `timeout`.
    fn dispatch_until(&mut self, timeout: Duration, done: impl Fn(&State) -> bool) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            self.queue
                .dispatch_pending(&mut self.state)
                .context("dispatching Wayland events")?;
            if done(&self.state) {
                return Ok(());
            }
            self.queue.flush().context("flushing Wayland requests")?;
            let Some(guard) = self.queue.prepare_read() else {
                continue;
            };
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                bail!("timed out after {timeout:?}");
            }
            let fd = guard.connection_fd();
            let mut fds = [PollFd::new(&fd, PollFlags::IN)];
            let timeout = Timespec::try_from(remaining).ok();
            poll(&mut fds, timeout.as_ref()).context("polling the Wayland socket")?;
            if fds[0].revents().contains(PollFlags::IN) {
                guard.read().context("reading Wayland events")?;
            }
        }
    }
}

fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[derive(Default)]
struct PendingToplevel {
    identifier: Option<String>,
    app_id: Option<String>,
}

#[derive(Default)]
struct SessionState {
    constraints: Constraints,
    /// Constraints accumulate until `done`.
    next: Constraints,
    done: bool,
    stopped: bool,
}

#[derive(Default)]
struct FrameState {
    ready: bool,
    failed: Option<WEnum<ext_image_copy_capture_frame_v1::FailureReason>>,
    transform: Option<WEnum<wl_output::Transform>>,
}

struct OutputInfo {
    output: wl_output::WlOutput,
    name: Option<String>,
}

struct State {
    shm: wl_shm::WlShm,
    list: ExtForeignToplevelListV1,
    sources: ExtForeignToplevelImageCaptureSourceManagerV1,
    copy: ExtImageCopyCaptureManagerV1,
    mapping: Option<HyprlandToplevelMappingManagerV1>,
    output_sources: Option<ExtOutputImageCaptureSourceManagerV1>,
    outputs: Vec<OutputInfo>,
    toplevels: Vec<Toplevel>,
    pending: HashMap<ObjectId, PendingToplevel>,
    /// Mapping handle id → address, or `None` when the mapping failed.
    mappings: HashMap<ObjectId, Option<u64>>,
    sessions: HashMap<ObjectId, SessionState>,
    frames: HashMap<ObjectId, FrameState>,
}

impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
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

macro_rules! ignore_events {
    ($($ty:ty),* $(,)?) => {$(
        impl Dispatch<$ty, ()> for State {
            fn event(_: &mut Self, _: &$ty, _: <$ty as Proxy>::Event, _: &(), _: &Connection, _: &QueueHandle<Self>) {}
        }
    )*};
}

ignore_events!(
    wl_shm::WlShm,
    wl_shm_pool::WlShmPool,
    wl_buffer::WlBuffer,
    ExtForeignToplevelImageCaptureSourceManagerV1,
    ExtOutputImageCaptureSourceManagerV1,
    ExtImageCaptureSourceV1,
    ExtImageCopyCaptureManagerV1,
    HyprlandToplevelMappingManagerV1,
);

impl Dispatch<wl_output::WlOutput, ()> for State {
    fn event(
        state: &mut Self,
        output: &wl_output::WlOutput,
        event: wl_output::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_output::Event::Name { name } = event
            && let Some(info) = state.outputs.iter_mut().find(|o| o.output == *output)
        {
            info.name = Some(name);
        }
    }
}

impl Dispatch<ExtForeignToplevelListV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ExtForeignToplevelListV1,
        event: ext_foreign_toplevel_list_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let ext_foreign_toplevel_list_v1::Event::Toplevel { toplevel } = event {
            state
                .pending
                .insert(toplevel.id(), PendingToplevel::default());
        }
    }

    event_created_child!(State, ExtForeignToplevelListV1, [
        ext_foreign_toplevel_list_v1::EVT_TOPLEVEL_OPCODE => (ExtForeignToplevelHandleV1, ()),
    ]);
}

impl Dispatch<ExtForeignToplevelHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &ExtForeignToplevelHandleV1,
        event: ext_foreign_toplevel_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_foreign_toplevel_handle_v1::Event;
        let id = handle.id();
        match event {
            Event::Identifier { identifier } => {
                state.pending.entry(id).or_default().identifier = Some(identifier);
            }
            Event::AppId { app_id } => {
                if let Some(t) = state.toplevels.iter_mut().find(|t| t.handle == *handle) {
                    t.app_id = app_id;
                } else {
                    state.pending.entry(id).or_default().app_id = Some(app_id);
                }
            }
            // Titles are private; Orbit's evidence never needs them here.
            Event::Title { .. } => {}
            Event::Done => {
                if let Some(pending) = state.pending.remove(&id)
                    && let Some(identifier) = pending.identifier
                {
                    state.toplevels.push(Toplevel {
                        identifier,
                        app_id: pending.app_id.unwrap_or_default(),
                        address: None,
                        handle: handle.clone(),
                    });
                }
            }
            Event::Closed => {
                state.pending.remove(&id);
                state.toplevels.retain(|t| t.handle != *handle);
                handle.destroy();
            }
            _ => {}
        }
    }
}

impl Dispatch<HyprlandToplevelWindowMappingHandleV1, ()> for State {
    fn event(
        state: &mut Self,
        handle: &HyprlandToplevelWindowMappingHandleV1,
        event: hyprland_toplevel_window_mapping_handle_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use hyprland_toplevel_window_mapping_handle_v1::Event;
        let address = match event {
            Event::WindowAddress {
                address_hi,
                address,
            } => Some(u64::from(address_hi) << 32 | u64::from(address)),
            Event::Failed => None,
        };
        state.mappings.insert(handle.id(), address);
    }
}

impl Dispatch<ExtImageCopyCaptureSessionV1, ()> for State {
    fn event(
        state: &mut Self,
        session: &ExtImageCopyCaptureSessionV1,
        event: ext_image_copy_capture_session_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_session_v1::Event;
        let Some(s) = state.sessions.get_mut(&session.id()) else {
            return;
        };
        match event {
            Event::BufferSize { width, height } => {
                s.next.width = width;
                s.next.height = height;
            }
            Event::ShmFormat {
                format: WEnum::Value(format),
            } => s.next.shm_formats.push(format),
            Event::DmabufFormat { .. } => s.next.dmabuf_formats += 1,
            Event::Done => {
                s.constraints = std::mem::take(&mut s.next);
                s.done = true;
            }
            Event::Stopped => s.stopped = true,
            _ => {}
        }
    }
}

impl Dispatch<ExtImageCopyCaptureFrameV1, ()> for State {
    fn event(
        state: &mut Self,
        frame: &ExtImageCopyCaptureFrameV1,
        event: ext_image_copy_capture_frame_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        use ext_image_copy_capture_frame_v1::Event;
        let Some(f) = state.frames.get_mut(&frame.id()) else {
            return;
        };
        match event {
            Event::Transform { transform } => f.transform = Some(transform),
            Event::Ready => f.ready = true,
            Event::Failed { reason } => f.failed = Some(reason),
            _ => {}
        }
    }
}
