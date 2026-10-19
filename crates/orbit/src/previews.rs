//! The overview's capture worker (ORB-006 on ORB-004's capture path).
//!
//! Runs on its own thread and Wayland connection. It captures only the
//! windows the overview asks for, only while asked, and sends back
//! downscaled BGRA thumbnails; on request it also captures the output once
//! and frosts it for the overview's backdrop (ORB-008). When the list
//! empties (the overview closed) it closes every session and goes idle.
//! Capture failing never blocks the overview: cards keep their last image
//! or say "Preview unavailable", and the backdrop falls back to a solid.

use std::{
    collections::HashMap,
    sync::mpsc::{self, RecvTimeoutError},
    thread,
    time::{Duration, Instant},
};

use orbit_capture::{Capturer, Failure, Session};
use orbit_ui::desk::Want;

use crate::{backdrop, preview::thumbnail};

/// What the worker sends back.
pub enum Captured {
    Window {
        address: u64,
        image: image::RgbaImage,
    },
    /// The frosted output capture, or `None` when it was unavailable.
    Backdrop { image: Option<image::RgbaImage> },
}

enum Job {
    Want(Vec<Want>),
    Backdrop {
        output: Option<String>,
        size: (u32, u32),
    },
    /// Settings: refresh cadence and backdrop blur radius.
    Tune {
        interval: Duration,
        blur: u32,
    },
}

/// Refresh cadence for visible cards (plan §06 suggests 5–10 per second as
/// a tuning target; this slice starts lower).
const INTERVAL: Duration = Duration::from_millis(400);
/// Repeat frames arrive only after new damage; don't hold the round.
const REPEAT_WAIT: Duration = Duration::from_millis(15);
/// After a failed connect, wait before trying again.
const RETRY: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct Previews {
    jobs: mpsc::Sender<Job>,
}

impl Previews {
    pub fn spawn(sink: impl Fn(Captured) + Send + 'static) -> std::io::Result<Self> {
        let (jobs, inbox) = mpsc::channel();
        thread::Builder::new()
            .name("orbit-previews".into())
            .spawn(move || run(&inbox, &sink))?;
        Ok(Self { jobs })
    }

    /// Which windows to capture, each with the device-pixel box its
    /// preview is drawn in; empty stops capturing.
    pub fn want(&self, wanted: Vec<Want>) {
        let _ = self.jobs.send(Job::Want(wanted));
    }

    /// Capture the output once and frost it to `size` for the backdrop.
    pub fn backdrop(&self, output: Option<String>, size: (u32, u32)) {
        let _ = self.jobs.send(Job::Backdrop { output, size });
    }

    /// Applies the previews settings.
    pub fn tune(&self, interval: Duration, blur: u32) {
        let _ = self.jobs.send(Job::Tune { interval, blur });
    }
}

fn run(inbox: &mpsc::Receiver<Job>, sink: &dyn Fn(Captured)) {
    let mut wanted: Vec<Want> = Vec::new();
    let mut backdrop: Option<(Option<String>, (u32, u32))> = None;
    let mut interval = INTERVAL;
    let mut blur = backdrop::RADIUS;
    let mut capturer: Option<Capturer> = None;
    let mut sessions: HashMap<u64, Session> = HashMap::new();
    // The box each open session last captured for: a new box needs a fresh
    // frame, which only a new session guarantees (repeats wait for damage).
    let mut boxes: HashMap<u64, (u32, u32)> = HashMap::new();
    let mut failed_at: Option<Instant> = None;
    loop {
        // Idle until asked; otherwise wait out the rest of the round while
        // staying responsive to new lists.
        let next = if wanted.is_empty() {
            inbox.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            inbox.recv_timeout(interval)
        };
        let mut apply = |job: Job, wanted: &mut Vec<Want>| match job {
            Job::Want(list) => *wanted = list,
            Job::Backdrop { output, size } => backdrop = Some((output, size)),
            Job::Tune {
                interval: new_interval,
                blur: new_blur,
            } => {
                interval = new_interval;
                blur = new_blur;
            }
        };
        match next {
            Ok(job) => {
                apply(job, &mut wanted);
                while let Ok(job) = inbox.try_recv() {
                    apply(job, &mut wanted);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
        if wanted.is_empty() && backdrop.is_none() {
            if let Some(capturer) = &mut capturer {
                for (_, session) in sessions.drain() {
                    capturer.close(session);
                }
            }
            // Release the connection and its buffers while hidden.
            capturer = None;
            continue;
        }
        if capturer.is_none() && failed_at.is_none_or(|at| at.elapsed() >= RETRY) {
            match Capturer::connect() {
                Ok(connected) => {
                    log::info!(
                        "previews: capture connected (address mapping: {})",
                        connected.has_address_mapping()
                    );
                    capturer = Some(connected);
                }
                Err(error) => {
                    log::info!("previews unavailable: {error:#}");
                    failed_at = Some(Instant::now());
                }
            }
        }
        let Some(active) = &mut capturer else {
            if backdrop.take().is_some() {
                sink(Captured::Backdrop { image: None });
            }
            continue;
        };
        if let Some((output, size)) = backdrop.take() {
            let image = match active.capture_output(output.as_deref()) {
                Ok(capture) => Some(backdrop::frost(&capture.frame, size, blur)),
                Err(error) => {
                    log::info!("backdrop capture unavailable: {error:#}");
                    None
                }
            };
            sink(Captured::Backdrop { image });
        }
        if wanted.is_empty() {
            continue;
        }
        if let Err(error) = round(active, &mut sessions, &mut boxes, &wanted, sink) {
            log::warn!("capture round failed: {error:#}");
            sessions.clear();
            boxes.clear();
            capturer = None;
            failed_at = Some(Instant::now());
        }
    }
}

fn round(
    capturer: &mut Capturer,
    sessions: &mut HashMap<u64, Session>,
    boxes: &mut HashMap<u64, (u32, u32)>,
    wanted: &[Want],
    sink: &dyn Fn(Captured),
) -> anyhow::Result<()> {
    let toplevels = capturer.toplevels()?;
    for address in sessions.keys().copied().collect::<Vec<_>>() {
        let same_box = wanted
            .iter()
            .any(|w| w.address == address && boxes.get(&address) == Some(&(w.width, w.height)));
        if !same_box {
            let session = sessions.remove(&address).expect("listed");
            boxes.remove(&address);
            capturer.close(session);
        }
    }
    let mut captured = 0;
    for want in wanted {
        let address = want.address;
        let Some(toplevel) = toplevels.iter().find(|t| t.address == Some(address)) else {
            log::debug!("no capture handle for {address:#x}");
            continue;
        };
        let session = match sessions.remove(&address) {
            Some(session) => session,
            None => match capturer.open(toplevel) {
                Ok(session) => session,
                Err(error) => {
                    log::debug!("no capture session for {address:#x}: {error:#}");
                    continue;
                }
            },
        };
        let mut session = session;
        match capturer.grab(&mut session, REPEAT_WAIT) {
            Ok(capture) => {
                captured += 1;
                sink(Captured::Window {
                    address,
                    image: thumbnail(&capture.frame, (want.width, want.height)),
                });
                sessions.insert(address, session);
                boxes.insert(address, (want.width, want.height));
            }
            Err((Failure::Unchanged, _)) => {
                sessions.insert(address, session);
            }
            Err((_, error)) => {
                log::debug!("capture of {address:#x} failed: {error:#}");
                capturer.close(session);
            }
        }
    }
    log::debug!(
        "capture round: {captured} new frames for {} windows ({} listed)",
        wanted.len(),
        toplevels.len()
    );
    Ok(())
}
