//! The live connection to Hyprland (ORB-006).
//!
//! One thread owns the event socket and also runs the queries and
//! dispatches it is asked for, one at a time. Everything it reports leaves
//! in the order it happened on that thread, which gives the store the
//! ordering it relies on: after a dispatch reply, every event already
//! readable is delivered before the result.
//!
//! A lost event socket, a failed query or a failed request ends the
//! connection. The next one is a new `link`, which the caller maps to a new
//! store epoch, so nothing from the old connection is trusted.

use std::{
    io::{self, Read, Write},
    os::unix::net::UnixStream,
    sync::mpsc::{self, TryRecvError},
    thread,
    time::Duration,
};

use anyhow::{Context as _, Result};
use orbit_core::{Event, Kinds, Refresh};
use rustix::event::{PollFd, PollFlags, Timespec, poll};

use crate::{
    Instance, Query,
    control::{Control, Syntax},
    events, snapshot,
};

const RETRY: Duration = Duration::from_secs(1);

pub enum ToLink {
    Query(Kinds),
    Dispatch { ticket: u64, control: Control },
}

#[derive(Debug)]
pub enum FromLink {
    Connected {
        link: u64,
        version: String,
        syntax: Syntax,
    },
    Event {
        link: u64,
        event: Event,
    },
    /// Events were lost: a line did not parse.
    Gap {
        link: u64,
    },
    Refresh {
        link: u64,
        refresh: Refresh,
    },
    Dispatched {
        link: u64,
        ticket: u64,
        result: Result<(), String>,
    },
    Disconnected {
        link: u64,
        error: String,
    },
}

/// Handle to the connection thread. Dropping it stops the thread.
pub struct Link {
    commands: mpsc::Sender<ToLink>,
    wake: UnixStream,
}

impl Link {
    /// Starts the thread. `locate` is called for every connection attempt.
    pub fn spawn(
        locate: impl Fn() -> Result<Instance> + Send + 'static,
        sink: impl Fn(FromLink) + Send + 'static,
    ) -> io::Result<Self> {
        let (commands, inbox) = mpsc::channel();
        let (wake, woken) = UnixStream::pair()?;
        woken.set_nonblocking(true)?;
        thread::Builder::new()
            .name("orbit-hyprland".into())
            .spawn(move || {
                Worker {
                    inbox,
                    woken,
                    sink: Box::new(sink),
                    link: 0,
                }
                .run(locate)
            })?;
        Ok(Self { commands, wake })
    }

    pub fn send(&self, command: ToLink) {
        if self.commands.send(command).is_ok() {
            let _ = (&self.wake).write(&[1]);
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        // The worker sees the closed channel after this wake-up.
        let _ = self.wake.shutdown(std::net::Shutdown::Both);
    }
}

struct Worker {
    inbox: mpsc::Receiver<ToLink>,
    woken: UnixStream,
    sink: Box<dyn Fn(FromLink) + Send>,
    link: u64,
}

enum End {
    Stop,
    Lost(anyhow::Error),
}

impl Worker {
    fn run(mut self, locate: impl Fn() -> Result<Instance>) {
        loop {
            match connect(&locate) {
                Ok((instance, stream, version)) => {
                    self.link += 1;
                    let syntax = Syntax::for_version(&version);
                    (self.sink)(FromLink::Connected {
                        link: self.link,
                        version,
                        syntax,
                    });
                    match self.serve(&instance, stream, syntax) {
                        End::Stop => return,
                        End::Lost(error) => (self.sink)(FromLink::Disconnected {
                            link: self.link,
                            error: format!("{error:#}"),
                        }),
                    }
                }
                Err(error) => log::debug!("hyprland link: {error:#}"),
            }
            if self.idle(RETRY) {
                return;
            }
        }
    }

    /// Waits while disconnected. Queries are dropped (a reconnect queries
    /// everything again) and dispatches fail. True when told to stop.
    fn idle(&mut self, duration: Duration) -> bool {
        let deadline = std::time::Instant::now() + duration;
        loop {
            loop {
                match self.inbox.try_recv() {
                    Ok(ToLink::Query(_)) => {}
                    Ok(ToLink::Dispatch { ticket, .. }) => (self.sink)(FromLink::Dispatched {
                        link: self.link,
                        ticket,
                        result: Err("not connected to Hyprland".into()),
                    }),
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return true,
                }
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            if left.is_zero() {
                return false;
            }
            if self.wait(None, Some(left)).is_err() {
                return true;
            }
        }
    }

    /// Polls the wake socket and, when connected, the event socket. Returns
    /// whether the event socket is readable.
    fn wait(&mut self, events: Option<&UnixStream>, timeout: Option<Duration>) -> io::Result<bool> {
        let timeout = timeout.map(|d| Timespec {
            tv_sec: d.as_secs() as _,
            tv_nsec: d.subsec_nanos() as _,
        });
        let readable = {
            let mut fds = vec![PollFd::new(&self.woken, PollFlags::IN)];
            if let Some(events) = events {
                fds.push(PollFd::new(events, PollFlags::IN));
            }
            match poll(&mut fds, timeout.as_ref()) {
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => return Ok(false),
                Err(error) => return Err(error.into()),
            }
            let woken = &fds[0].revents();
            if woken.intersects(PollFlags::HUP | PollFlags::ERR) {
                return Err(io::Error::other("link handle dropped"));
            }
            fds.get(1).is_some_and(|fd| !fd.revents().is_empty())
        };
        let mut drain = [0u8; 64];
        while matches!(self.woken.read(&mut drain), Ok(n) if n > 0) {}
        Ok(readable)
    }

    fn serve(&mut self, instance: &Instance, mut stream: UnixStream, syntax: Syntax) -> End {
        let mut pending = Vec::new();
        loop {
            loop {
                let command = match self.inbox.try_recv() {
                    Ok(command) => command,
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return End::Stop,
                };
                let link = self.link;
                match command {
                    ToLink::Query(kinds) => match snapshot::query_refresh(instance, kinds) {
                        Ok(refresh) => (self.sink)(FromLink::Refresh { link, refresh }),
                        Err(error) => return End::Lost(error),
                    },
                    ToLink::Dispatch { ticket, control } => {
                        let result = match instance.dispatch(&control, syntax) {
                            Ok(result) => result,
                            Err(error) => {
                                (self.sink)(FromLink::Dispatched {
                                    link,
                                    ticket,
                                    result: Err("lost the Hyprland connection".into()),
                                });
                                return End::Lost(error);
                            }
                        };
                        // The contract: events already readable come first.
                        if let Err(error) = self.read_events(&mut stream, &mut pending) {
                            return End::Lost(error);
                        }
                        (self.sink)(FromLink::Dispatched {
                            link,
                            ticket,
                            result,
                        });
                    }
                }
            }
            match self.wait(Some(&stream), None) {
                Ok(true) => {
                    if let Err(error) = self.read_events(&mut stream, &mut pending) {
                        return End::Lost(error);
                    }
                }
                Ok(false) => {}
                Err(_) => return End::Stop,
            }
        }
    }

    /// Reads everything available without blocking and reports whole lines.
    fn read_events(&mut self, stream: &mut UnixStream, pending: &mut Vec<u8>) -> Result<()> {
        let mut chunk = [0u8; 16 * 1024];
        loop {
            match stream.read(&mut chunk) {
                Ok(0) => anyhow::bail!("Hyprland closed the event socket"),
                Ok(n) => pending.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(e).context("reading the event socket"),
            }
        }
        let mut start = 0;
        while let Some(end) = pending[start..].iter().position(|&b| b == b'\n') {
            let line = String::from_utf8_lossy(&pending[start..start + end]);
            start += end + 1;
            match events::parse(&line) {
                Ok(Some(event)) => (self.sink)(FromLink::Event {
                    link: self.link,
                    event,
                }),
                Ok(None) => {}
                Err(error) => {
                    log::warn!("{error}");
                    (self.sink)(FromLink::Gap { link: self.link });
                }
            }
        }
        pending.drain(..start);
        Ok(())
    }
}

/// Subscribes to events first, then reads the version for the syntax.
fn connect(locate: &impl Fn() -> Result<Instance>) -> Result<(Instance, UnixStream, String)> {
    let instance = locate()?;
    let path = instance.event_socket();
    let stream =
        UnixStream::connect(&path).with_context(|| format!("connecting to {}", path.display()))?;
    stream.set_nonblocking(true)?;
    let version = instance.query(Query::Version)?;
    let version = ["version", "tag"]
        .iter()
        .find_map(|key| version[key].as_str().filter(|v| !v.is_empty()))
        .unwrap_or("unknown")
        .to_owned();
    Ok((instance, stream, version))
}
