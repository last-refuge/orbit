//! Seeded simulation: a fake compositor with lost, late and reordered
//! events, stale query answers, lost acks, rejected requests, window churn
//! with reused addresses, and other clients moving windows. Checks that the
//! store always converges to the compositor's truth, finishes every move
//! exactly once, and only confirms moves that really happened.

use std::collections::{BTreeMap, HashMap, VecDeque};

use orbit_core::*;

/// Small deterministic generator (SplitMix64); no external crates.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n
    }
    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

#[derive(Clone)]
struct TrueWindow {
    workspace: i64,
    pid: i64,
}

struct Compositor {
    windows: BTreeMap<u64, TrueWindow>,
    next_pid: i64,
    /// Orders placements and requests (the clock alone has ties).
    seq: u64,
    /// (seq, address, pid, workspace) for every placement.
    history: Vec<(u64, u64, i64, i64)>,
}

impl Compositor {
    fn place(&mut self, address: u64, workspace: i64) {
        let w = self.windows.get_mut(&address).unwrap();
        w.workspace = workspace;
        self.history.push((self.seq, address, w.pid, workspace));
        self.seq += 1;
    }

    fn refresh(&self, kinds: Kinds) -> Refresh {
        let workspaces = (1..=WORKSPACES)
            .map(|id| Workspace {
                id: WorkspaceId(id),
                name: id.to_string(),
                monitor: Some("M".into()),
            })
            .collect();
        let windows = self
            .windows
            .iter()
            .map(|(address, w)| WindowInfo {
                address: *address,
                workspace: WorkspaceId(w.workspace),
                class: "app".into(),
                title: String::new(),
                pid: w.pid,
                floating: false,
                fullscreen: false,
                xwayland: false,
                hidden: false,
                at: (0, 0),
                size: (0, 0),
            })
            .collect();
        Refresh {
            monitors: kinds.contains(Kinds::MONITORS).then(|| {
                vec![Monitor {
                    id: 0,
                    name: "M".into(),
                    focused: true,
                    active_workspace: WorkspaceId(1),
                    special_workspace: None,
                    width: 1920.0,
                    height: 1080.0,
                    x: 0.0,
                    y: 0.0,
                    scale: 1.0,
                }]
            }),
            workspaces: kinds.contains(Kinds::WORKSPACES).then_some(workspaces),
            windows: kinds.contains(Kinds::WINDOWS).then_some(windows),
            active_window: kinds.contains(Kinds::WINDOWS).then_some(None),
        }
    }
}

const WORKSPACES: i64 = 4;
const ADDRESSES: u64 = 6;

struct Sim {
    rng: Rng,
    now: u64,
    store: Store,
    epoch: Epoch,
    truth: Compositor,
    /// `None` marks lost events, which the adapter reports as a gap.
    events: VecDeque<Option<Event>>,
    /// Answers already taken from the compositor, delivered later.
    answers: VecDeque<Refresh>,
    /// Kinds asked for and not yet captured into an answer.
    asked: VecDeque<Kinds>,
    dispatches: VecDeque<(OpId, Command)>,
    /// op → (request seq, address, pid, destination)
    ops: HashMap<OpId, (u64, u64, i64, i64)>,
    finished: HashMap<OpId, Outcome>,
    lossy: bool,
}

impl Sim {
    fn new(seed: u64) -> Self {
        let mut store = Store::new(Config {
            op_timeout_ms: 300,
            health_interval_ms: 2_000,
            sync_buffer: 16,
        });
        let (epoch, effects) = store.connect(0);
        let mut sim = Self {
            rng: Rng(seed),
            now: 0,
            store,
            epoch,
            truth: Compositor {
                windows: BTreeMap::new(),
                next_pid: 1,
                seq: 0,
                history: Vec::new(),
            },
            events: VecDeque::new(),
            answers: VecDeque::new(),
            asked: VecDeque::new(),
            dispatches: VecDeque::new(),
            ops: HashMap::new(),
            finished: HashMap::new(),
            lossy: true,
        };
        sim.handle(effects);
        sim
    }

    fn handle(&mut self, effects: Vec<Effect>) {
        for effect in effects {
            match effect {
                Effect::Query(kinds) => self.asked.push_back(kinds),
                Effect::Dispatch { op, command } => self.dispatches.push_back((op, command)),
                Effect::Finished { op, outcome } => {
                    assert!(
                        self.finished.insert(op, outcome.clone()).is_none(),
                        "{op:?} finished twice"
                    );
                }
            }
        }
    }

    /// A confirmed move must find that same window on the destination when
    /// it was requested or at some point after.
    fn check_confirmed(&self, op: OpId) {
        let (at, address, pid, dest) = self.ops[&op];
        let placements: Vec<(u64, i64)> = self
            .truth
            .history
            .iter()
            .filter(|&&(_, a, p, _)| a == address && p == pid)
            .map(|&(t, _, _, w)| (t, w))
            .collect();
        let at_request = placements.iter().rev().find(|(t, _)| *t <= at).map(|p| p.1);
        let happened =
            at_request == Some(dest) || placements.iter().any(|&(t, w)| t > at && w == dest);
        assert!(
            happened,
            "{op:?} confirmed but never observed on {dest}: requested at {at}, placements {placements:?}"
        );
    }

    fn emit(&mut self, event: Event) {
        // Late but in order: Hyprland's event socket is a stream. Losses
        // come from reconnects or overflow, which the adapter detects.
        if self.lossy && self.rng.chance(10) {
            self.events.push_back(None);
        } else {
            self.events.push_back(Some(event));
        }
    }

    fn step(&mut self) {
        match self.rng.below(10) {
            0 => self.open_or_close(),
            1 => self.external_move(),
            2 | 3 => self.request_move(),
            4 | 5 => self.deliver_event(),
            6 => self.capture_answer(),
            7 => self.deliver_answer(),
            8 => self.run_dispatch(),
            _ => {
                self.now += self.rng.below(200);
                let effects = self.store.tick(self.now);
                self.handle(effects);
            }
        }
    }

    fn open_or_close(&mut self) {
        let address = 0x100 + self.rng.below(ADDRESSES);
        if self.truth.windows.remove(&address).is_some() {
            self.emit(Event::WindowClosed { address });
        } else {
            let workspace = 1 + self.rng.below(WORKSPACES as u64) as i64;
            let pid = self.truth.next_pid;
            self.truth.next_pid += 1;
            self.truth
                .windows
                .insert(address, TrueWindow { workspace, pid });
            self.truth
                .history
                .push((self.truth.seq, address, pid, workspace));
            self.truth.seq += 1;
            self.emit(Event::WindowOpened {
                address,
                workspace_name: workspace.to_string(),
                class: "app".into(),
                title: String::new(),
            });
        }
    }

    fn external_move(&mut self) {
        let Some(address) = self.pick_true_window() else {
            return;
        };
        let workspace = 1 + self.rng.below(WORKSPACES as u64) as i64;
        self.truth.place(address, workspace);
        self.emit(moved(address, workspace));
    }

    fn request_move(&mut self) {
        let known: Vec<WindowId> = self.store.model().windows.values().map(|w| w.id).collect();
        if known.is_empty() {
            return;
        }
        let window = known[self.rng.below(known.len() as u64) as usize];
        let dest = 1 + self.rng.below(WORKSPACES as u64) as i64;
        let pid = self.store.model().windows[&window.address].pid;
        let seq = self.truth.seq;
        self.truth.seq += 1;
        if let Ok((op, effects)) = self
            .store
            .request_move(window, Target::Numbered(dest), self.now)
        {
            self.ops.insert(op, (seq, window.address, pid, dest));
            self.handle(effects);
        }
    }

    fn deliver_event(&mut self) {
        let effects = match self.events.pop_front() {
            Some(Some(event)) => self.store.on_event(self.epoch, event).unwrap(),
            Some(None) => self.store.on_gap(self.epoch).unwrap(),
            None => return,
        };
        self.handle(effects);
    }

    /// Takes the compositor state for the oldest question now; it is
    /// delivered later, possibly after newer events.
    fn capture_answer(&mut self) {
        if let Some(kinds) = self.asked.pop_front() {
            let answer = self.truth.refresh(kinds);
            self.answers.push_back(answer);
        }
    }

    fn deliver_answer(&mut self) {
        if let Some(answer) = self.answers.pop_front() {
            let effects = self.store.on_refresh(self.epoch, answer).unwrap();
            self.handle(effects);
        }
    }

    fn run_dispatch(&mut self) {
        let Some((op, Command::MoveToWorkspaceSilent { window, target })) =
            self.dispatches.pop_front()
        else {
            return;
        };
        let dest = match target {
            Target::Numbered(n) => n,
            Target::Workspace(id) => id.0,
        };
        let reject = self.lossy && self.rng.chance(5);
        let alive = self.truth.windows.contains_key(&window.address);
        let result = if reject || !alive {
            Err("no such window".to_string())
        } else {
            self.truth.place(window.address, dest);
            self.emit(moved(window.address, dest));
            Ok(())
        };
        if !(self.lossy && self.rng.chance(20)) {
            // Adapter contract: drain readable events before the result.
            while !self.events.is_empty() {
                self.deliver_event();
            }
            let effects = self.store.on_dispatch_result(op, result).unwrap();
            self.handle(effects);
        }
    }

    fn pick_true_window(&mut self) -> Option<u64> {
        let addresses: Vec<u64> = self.truth.windows.keys().copied().collect();
        (!addresses.is_empty()).then(|| addresses[self.rng.below(addresses.len() as u64) as usize])
    }

    /// Stops the chaos and lets everything settle.
    fn drain(&mut self) {
        self.lossy = false;
        let effects = self.store.set_visible(true, self.now);
        self.handle(effects);
        for _ in 0..200 {
            while !self.events.is_empty() {
                self.deliver_event();
            }
            while !self.dispatches.is_empty() {
                self.run_dispatch();
            }
            while !self.asked.is_empty() {
                self.capture_answer();
            }
            while !self.answers.is_empty() {
                self.deliver_answer();
            }
            self.now += 1_000;
            let effects = self.store.tick(self.now);
            self.handle(effects);
            let settled = self.events.is_empty()
                && self.dispatches.is_empty()
                && self.asked.is_empty()
                && self.answers.is_empty()
                && self.store.inflight() == Kinds::NONE
                && self.ops.keys().all(|op| self.finished.contains_key(op));
            if settled {
                return;
            }
        }
        panic!("did not settle");
    }
}

fn moved(address: u64, workspace: i64) -> Event {
    Event::WindowMoved {
        address,
        workspace: WorkspaceId(workspace),
        workspace_name: workspace.to_string(),
    }
}

#[test]
fn converges_under_chaos() {
    let mut confirmed = 0;
    let mut failed = 0;
    for seed in 0..500 {
        let mut sim = Sim::new(seed);
        for _ in 0..400 {
            sim.step();
        }
        sim.drain();

        let model: BTreeMap<u64, (i64, i64)> = sim
            .store
            .model()
            .windows
            .iter()
            .map(|(a, w)| (*a, (w.workspace.0, w.pid)))
            .collect();
        let truth: BTreeMap<u64, (i64, i64)> = sim
            .truth
            .windows
            .iter()
            .map(|(a, w)| (*a, (w.workspace, w.pid)))
            .collect();
        assert_eq!(model, truth, "seed {seed}: model diverged");
        for (op, outcome) in &sim.finished {
            if *outcome == Outcome::Confirmed {
                sim.check_confirmed(*op);
            }
        }
        for outcome in sim.finished.values() {
            match outcome {
                Outcome::Confirmed => confirmed += 1,
                Outcome::Failed(_) => failed += 1,
            }
        }
    }
    // The run must exercise both paths to mean anything.
    assert!(
        confirmed > 500 && failed > 100,
        "{confirmed} confirmed, {failed} failed"
    );
}
