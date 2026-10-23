//! The state store: applies compositor events and query results to the
//! model, decides when to re-query, and tracks window moves until the
//! compositor is seen to have done them.
//!
//! Everything here is synchronous and deterministic. Inputs are events,
//! query results, dispatch results and a millisecond clock supplied by the
//! caller; outputs are [`Effect`]s for a platform adapter to carry out.
//!
//! Sync rules (plan §06): events arriving while a query is in flight may be
//! newer or older than its answer, so they are never replayed over it.
//! Instead the affected kinds are queried again once the answer lands.
//! While the first snapshot of an epoch is pending, events are not applied
//! at all; they only mark what to re-query.

use std::collections::{BTreeMap, VecDeque};

use crate::{
    event::Event,
    model::{Epoch, Kinds, Model, OpId, Refresh, WindowId, WindowInfo, WorkspaceId, new_window},
};

/// Finished operations remembered for status queries.
const DONE_KEPT: usize = 256;

#[derive(Clone, Copy, Debug)]
pub struct Config {
    /// How long a dispatched move may go unobserved before Orbit re-queries.
    pub op_timeout_ms: u64,
    /// Full re-query cadence while the overview is visible.
    pub health_interval_ms: u64,
    /// Events tolerated while the first snapshot is pending; beyond this the
    /// buffer counts as overflowed.
    pub sync_buffer: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            op_timeout_ms: 1_000,
            health_interval_ms: 5_000,
            sync_buffer: 1_024,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Sync {
    Disconnected,
    /// Waiting for the epoch's first full snapshot.
    Syncing,
    Live,
}

/// Where to move a window.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A workspace that exists now (numbered, named or special).
    Workspace(WorkspaceId),
    /// A numbered workspace, created on demand if empty.
    Numbered(i64),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Move without following the window or changing focus.
    MoveToWorkspaceSilent { window: WindowId, target: Target },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Query these parts of the compositor state and call
    /// [`Store::on_refresh`] with the answer.
    Query(Kinds),
    /// Send this request, then report with [`Store::on_dispatch_result`].
    Dispatch {
        op: OpId,
        command: Command,
    },
    Finished {
        op: OpId,
        outcome: Outcome,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The compositor state shows the window on the destination.
    Confirmed,
    Failed(Failure),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Failure {
    /// The window closed, or its address now belongs to another window.
    WindowGone,
    /// The destination workspace no longer exists.
    DestinationGone,
    /// The compositor connection was replaced before the move was seen.
    SessionRestarted,
    /// The compositor refused the request.
    Rejected(String),
    /// After the timeout and a fresh query, the window is not there.
    NotObserved,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpState {
    /// Waiting behind another operation on the same window.
    Queued,
    /// Dispatched; waiting to observe the result.
    Pending {
        deadline: u64,
    },
    /// Timed out; a fresh query decides.
    Unknown,
    Done(Outcome),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RequestError {
    /// No live snapshot yet (connecting or syncing).
    NotLive,
    /// The window is gone, or the id is from an earlier window or epoch.
    StaleWindow,
    /// The window is known only from its open event; its identity is
    /// confirmed by the query that event triggered.
    Unverified,
    UnknownDestination,
    AlreadyThere,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rejected {
    /// The input belongs to an earlier connection.
    StaleEpoch,
    UnknownOp,
}

/// What the model was just updated from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Event,
    /// A window query answer that no window event overtook.
    FreshQuery,
    /// A window query answer that window events overtook.
    RacedQuery,
}

#[derive(Clone, Debug)]
struct Op {
    window: WindowId,
    target: Target,
    state: OpState,
    /// A window query already in flight when the move was dispatched; its
    /// answer may predate the move and cannot confirm it.
    stale_answer: bool,
    /// The compositor accepted the request. Events read before this may
    /// predate the move, so only later ones can confirm it.
    accepted: bool,
}

impl Target {
    fn workspace(&self) -> WorkspaceId {
        match self {
            Target::Workspace(id) => *id,
            Target::Numbered(n) => WorkspaceId(*n),
        }
    }
}

pub struct Store {
    config: Config,
    epoch: Epoch,
    sync: Sync,
    model: Model,
    next_generation: u64,
    next_op: u64,
    ops: BTreeMap<OpId, Op>,
    /// Operations per window address, head first. Only the head runs.
    lanes: BTreeMap<u64, VecDeque<OpId>>,
    /// Finished operations, oldest first, kept for [`Store::op_state`].
    done: VecDeque<OpId>,
    /// Kinds queried and not yet answered.
    inflight: Kinds,
    /// Kinds changed by events while their query was in flight.
    touched: Kinds,
    buffered: usize,
    /// The window model may be wrong until a fresh window answer arrives:
    /// events were lost (it may hold a window whose address was reused), or
    /// an answer older than applied events overwrote them.
    doubtful: bool,
    visible: bool,
    last_health: u64,
    now: u64,
    revision: u64,
}

impl Store {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            epoch: Epoch(0),
            sync: Sync::Disconnected,
            model: Model::default(),
            next_generation: 0,
            next_op: 0,
            ops: BTreeMap::new(),
            lanes: BTreeMap::new(),
            done: VecDeque::new(),
            inflight: Kinds::NONE,
            touched: Kinds::NONE,
            buffered: 0,
            doubtful: false,
            visible: false,
            last_health: 0,
            now: 0,
            revision: 0,
        }
    }

    pub fn model(&self) -> &Model {
        &self.model
    }

    pub fn sync(&self) -> Sync {
        self.sync
    }

    pub fn epoch(&self) -> Epoch {
        self.epoch
    }

    /// Bumped on every model change; the UI re-renders when it moves.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn op_state(&self, op: OpId) -> Option<&OpState> {
        self.ops.get(&op).map(|op| &op.state)
    }

    pub fn has_pending_moves(&self) -> bool {
        !self.lanes.is_empty()
    }

    /// Kinds asked for and not answered yet.
    pub fn inflight(&self) -> Kinds {
        self.inflight
    }

    /// Starts a new epoch: on first connect, reconnect or compositor
    /// restart. Everything from before is invalid, including pending moves.
    /// Subscribe to events before calling this, then take the snapshot.
    pub fn connect(&mut self, now: u64) -> (Epoch, Vec<Effect>) {
        self.now = now;
        self.epoch = Epoch(self.epoch.0 + 1);
        let mut effects = Vec::new();
        let unfinished: Vec<OpId> = self
            .ops
            .iter()
            .filter(|(_, op)| !matches!(op.state, OpState::Done(_)))
            .map(|(id, _)| *id)
            .collect();
        for id in unfinished {
            self.set_done(id, Outcome::Failed(Failure::SessionRestarted), &mut effects);
        }
        self.lanes.clear();
        self.model = Model::default();
        self.sync = Sync::Syncing;
        self.inflight = Kinds::ALL;
        self.touched = Kinds::NONE;
        self.buffered = 0;
        self.doubtful = false;
        self.last_health = now;
        self.revision += 1;
        effects.push(Effect::Query(Kinds::ALL));
        (self.epoch, effects)
    }

    pub fn disconnect(&mut self) {
        self.sync = Sync::Disconnected;
        self.inflight = Kinds::NONE;
        self.touched = Kinds::NONE;
        self.revision += 1;
    }

    pub fn on_event(&mut self, epoch: Epoch, event: Event) -> Result<Vec<Effect>, Rejected> {
        self.check_epoch(epoch)?;
        let mut effects = Vec::new();
        match self.sync {
            Sync::Disconnected => {}
            Sync::Syncing => {
                self.buffered += 1;
                self.touched |= if self.buffered > self.config.sync_buffer {
                    Kinds::ALL
                } else {
                    event.touches()
                };
            }
            Sync::Live => {
                self.touched |= event.touches().and(self.inflight);
                self.apply(event, &mut effects);
                self.revision += 1;
            }
        }
        Ok(effects)
    }

    /// Events were lost (reader overflow, socket reconnect, malformed line).
    pub fn on_gap(&mut self, epoch: Epoch) -> Result<Vec<Effect>, Rejected> {
        self.check_epoch(epoch)?;
        let mut effects = Vec::new();
        if self.sync == Sync::Syncing {
            self.touched = Kinds::ALL;
        } else if self.sync == Sync::Live {
            self.doubtful = true;
            // Windows known only from an open event have no pid yet, so a
            // query could not tell them from a later window at the same
            // address. Forget them; the query brings them back.
            let unverified: Vec<WindowId> = self
                .model
                .windows
                .values()
                .filter(|w| w.pid == 0)
                .map(|w| w.id)
                .collect();
            for id in unverified {
                self.model.windows.remove(&id.address);
                self.fail_window(id, &mut effects);
            }
            self.request(Kinds::ALL, &mut effects);
        }
        Ok(effects)
    }

    pub fn on_refresh(&mut self, epoch: Epoch, refresh: Refresh) -> Result<Vec<Effect>, Rejected> {
        self.check_epoch(epoch)?;
        let mut effects = Vec::new();
        let kinds = refresh.kinds();
        if let Some(monitors) = refresh.monitors {
            self.model.monitors = monitors;
        }
        if let Some(workspaces) = refresh.workspaces {
            self.model.workspaces = workspaces.into_iter().map(|w| (w.id, w)).collect();
        }
        if let Some(windows) = refresh.windows {
            let gone = self
                .model
                .replace_windows(windows, &mut self.next_generation);
            for id in gone {
                self.fail_window(id, &mut effects);
            }
        }
        if let Some(active) = refresh.active_window {
            self.model.active_window = active
                .and_then(|a| self.model.windows.get(&a))
                .map(|w| w.id);
        }
        self.inflight = self.inflight.without(kinds);
        if self.sync == Sync::Syncing && kinds.contains(Kinds::ALL) {
            self.sync = Sync::Live;
        }
        self.revision += 1;

        if kinds.contains(Kinds::WINDOWS) {
            // An answer overtaken by window events cannot prove a move failed.
            let source = if self.touched.intersects(Kinds::WINDOWS) {
                self.doubtful = true;
                Source::RacedQuery
            } else {
                self.doubtful = false;
                Source::FreshQuery
            };
            self.resolve_ops(source, &mut effects);
        }
        let again = self.touched.and(kinds);
        self.touched = self.touched.without(kinds);
        if !again.is_empty() && self.sync == Sync::Live {
            self.request(again, &mut effects);
        }
        Ok(effects)
    }

    /// Asks to move a window. The move is dispatched at once, or queued
    /// behind earlier operations on the same window.
    pub fn request_move(
        &mut self,
        window: WindowId,
        target: Target,
        now: u64,
    ) -> Result<(OpId, Vec<Effect>), RequestError> {
        self.now = now;
        if self.sync != Sync::Live {
            return Err(RequestError::NotLive);
        }
        let current = self.model.window(window).ok_or(RequestError::StaleWindow)?;
        if current.pid == 0 {
            return Err(RequestError::Unverified);
        }
        match &target {
            Target::Workspace(id) if !self.model.workspaces.contains_key(id) => {
                return Err(RequestError::UnknownDestination);
            }
            Target::Numbered(n) if *n <= 0 => return Err(RequestError::UnknownDestination),
            _ => {}
        }
        let lane_idle = self.lanes.get(&window.address).is_none_or(|l| l.is_empty());
        if lane_idle && current.workspace == target.workspace() {
            return Err(RequestError::AlreadyThere);
        }

        self.next_op += 1;
        let id = OpId(self.next_op);
        self.ops.insert(
            id,
            Op {
                window,
                target,
                state: OpState::Queued,
                stale_answer: false,
                accepted: false,
            },
        );
        let lane = self.lanes.entry(window.address).or_default();
        lane.push_back(id);
        let mut effects = Vec::new();
        if lane.len() == 1 {
            self.start(id, &mut effects);
        }
        Ok((id, effects))
    }

    /// The adapter's answer for a dispatched request. Success only means
    /// the compositor accepted it; confirmation still waits for observation.
    ///
    /// Contract: before reporting a result, the adapter delivers every event
    /// already readable on the event socket. Hyprland writes the events of
    /// a dispatch before its reply, so the model is then current.
    pub fn on_dispatch_result(
        &mut self,
        op: OpId,
        result: Result<(), String>,
    ) -> Result<Vec<Effect>, Rejected> {
        let entry = self.ops.get_mut(&op).ok_or(Rejected::UnknownOp)?;
        let mut effects = Vec::new();
        if !matches!(entry.state, OpState::Pending { .. } | OpState::Unknown) {
            return Ok(effects);
        }
        match result {
            Err(message) => self.finish(
                op,
                Outcome::Failed(Failure::Rejected(message)),
                &mut effects,
            ),
            Ok(()) => {
                entry.accepted = true;
                // Every event up to the acceptance has been applied, so the
                // model now shows whether the move landed.
                self.resolve_ops(Source::Event, &mut effects);
            }
        }
        Ok(effects)
    }

    /// Advances the clock: times out unobserved moves and runs the health
    /// check while visible.
    pub fn tick(&mut self, now: u64) -> Vec<Effect> {
        self.now = now;
        let mut effects = Vec::new();
        let mut expired = false;
        for op in self.ops.values_mut() {
            if let OpState::Pending { deadline } = op.state
                && now >= deadline
            {
                op.state = OpState::Unknown;
                expired = true;
            }
        }
        if expired {
            // Ask again even if a query is already out: its answer may
            // predate the timeout.
            self.request(Kinds::WINDOWS, &mut effects);
        }
        if self.visible
            && self.sync == Sync::Live
            && now.saturating_sub(self.last_health) >= self.config.health_interval_ms
        {
            self.last_health = now;
            self.request(Kinds::ALL, &mut effects);
        }
        effects
    }

    /// The overview opened or closed. Opening reconciles everything.
    pub fn set_visible(&mut self, visible: bool, now: u64) -> Vec<Effect> {
        self.now = now;
        self.visible = visible;
        let mut effects = Vec::new();
        if visible && self.sync == Sync::Live {
            self.last_health = now;
            self.request(Kinds::ALL, &mut effects);
        }
        effects
    }

    fn check_epoch(&self, epoch: Epoch) -> Result<(), Rejected> {
        if epoch == self.epoch && self.sync != Sync::Disconnected {
            Ok(())
        } else {
            Err(Rejected::StaleEpoch)
        }
    }

    /// Queries `kinds`, coalescing with what is already in flight. Kinds
    /// already in flight are queried again once that answer lands.
    fn request(&mut self, kinds: Kinds, effects: &mut Vec<Effect>) {
        self.touched |= kinds.and(self.inflight);
        let new = kinds.without(self.inflight);
        if !new.is_empty() {
            self.inflight |= new;
            effects.push(Effect::Query(new));
        }
    }

    fn apply(&mut self, event: Event, effects: &mut Vec<Effect>) {
        let model = &mut self.model;
        match event {
            Event::WindowOpened {
                address,
                workspace_name,
                class,
                title,
            } => {
                if let Some(previous) = model.windows.remove(&address) {
                    self.fail_window(previous.id, effects);
                }
                let model = &mut self.model;
                match model.workspace_by_name(&workspace_name).map(|w| w.id) {
                    Some(workspace) => {
                        let window = new_window(
                            WindowInfo {
                                address,
                                workspace,
                                class,
                                title,
                                // Unknown until the next query fills it in.
                                pid: 0,
                                floating: false,
                                fullscreen: false,
                                xwayland: false,
                                hidden: false,
                                at: (0, 0),
                                size: (0, 0),
                            },
                            &mut self.next_generation,
                        );
                        model.windows.insert(address, window);
                        self.request(Kinds::WINDOWS, effects);
                    }
                    None => self.request(Kinds::WINDOWS | Kinds::WORKSPACES, effects),
                }
            }
            Event::WindowClosed { address } => {
                if let Some(window) = model.windows.remove(&address) {
                    if model.active_window == Some(window.id) {
                        model.active_window = None;
                    }
                    self.fail_window(window.id, effects);
                }
            }
            Event::WindowMoved {
                address,
                workspace,
                workspace_name,
            } => {
                let known = model.workspaces.contains_key(&workspace);
                match model.windows.get_mut(&address) {
                    Some(window) => window.workspace = workspace,
                    None => self.request(Kinds::WINDOWS, effects),
                }
                if !known {
                    self.model.workspaces.insert(
                        workspace,
                        crate::model::Workspace {
                            id: workspace,
                            name: workspace_name,
                            monitor: None,
                        },
                    );
                    self.request(Kinds::WORKSPACES, effects);
                }
                self.resolve_ops(Source::Event, effects);
            }
            Event::WindowTitle { address, title } => {
                if let Some(window) = model.windows.get_mut(&address) {
                    window.title = title;
                }
            }
            Event::ActiveWindow { address } => {
                model.active_window = address.and_then(|a| model.windows.get(&a)).map(|w| w.id);
            }
            Event::Floating { address, floating } => {
                if let Some(window) = model.windows.get_mut(&address) {
                    window.floating = floating;
                }
            }
            Event::Fullscreen => self.request(Kinds::WINDOWS, effects),
            Event::WorkspaceCreated { id, name } => {
                model.workspaces.insert(
                    id,
                    crate::model::Workspace {
                        id,
                        name,
                        monitor: None,
                    },
                );
                self.request(Kinds::WORKSPACES, effects);
            }
            Event::WorkspaceDestroyed { id } => {
                model.workspaces.remove(&id);
            }
            Event::WorkspaceRenamed { id, name } => {
                if let Some(workspace) = model.workspaces.get_mut(&id) {
                    workspace.name = name;
                }
            }
            Event::WorkspaceIdChanged { old, new } => {
                if let Some(mut workspace) = model.workspaces.remove(&old) {
                    workspace.id = new;
                    if workspace.name == old.0.to_string() {
                        workspace.name = new.0.to_string();
                    }
                    model.workspaces.insert(new, workspace);
                }
                for window in model.windows.values_mut() {
                    if window.workspace == old {
                        window.workspace = new;
                    }
                }
                for monitor in &mut model.monitors {
                    if monitor.active_workspace == old {
                        monitor.active_workspace = new;
                    }
                    if monitor.special_workspace == Some(old) {
                        monitor.special_workspace = Some(new);
                    }
                }
                self.request(Kinds::ALL, effects);
            }
            Event::WorkspaceMoved { id, monitor } => {
                if let Some(workspace) = model.workspaces.get_mut(&id) {
                    workspace.monitor = Some(monitor);
                }
                self.request(Kinds::MONITORS, effects);
            }
            Event::ActiveWorkspace { id } => {
                if let Some(monitor) = model.focused_monitor_mut() {
                    monitor.active_workspace = id;
                }
            }
            Event::FocusedMonitor { monitor, workspace } => {
                for m in &mut model.monitors {
                    m.focused = m.name == monitor;
                    if m.focused {
                        m.active_workspace = workspace;
                    }
                }
            }
            Event::ActiveSpecial { monitor, workspace } => {
                if let Some(m) = model.monitors.iter_mut().find(|m| m.name == monitor) {
                    m.special_workspace = workspace;
                }
            }
            Event::LayerClosed { .. } => {}
            Event::MonitorAdded { .. } | Event::MonitorRemoved { .. } | Event::ConfigReloaded => {
                self.request(Kinds::ALL, effects);
            }
        }
    }

    fn start(&mut self, id: OpId, effects: &mut Vec<Effect>) {
        let op = &self.ops[&id];
        let (window, target) = (op.window, op.target.clone());
        let outcome = match self.model.window(window) {
            None => Some(Outcome::Failed(Failure::WindowGone)),
            Some(_) if matches!(&target, Target::Workspace(ws) if !self.model.workspaces.contains_key(ws)) => {
                Some(Outcome::Failed(Failure::DestinationGone))
            }
            // Even if the model already shows the window there, dispatch:
            // the model may be stale, and only the compositor's answer and
            // later state count as confirmation.
            Some(_) => None,
        };
        match outcome {
            Some(outcome) => self.finish(id, outcome, effects),
            None => {
                let stale_answer = self.inflight.contains(Kinds::WINDOWS);
                let op = self.ops.get_mut(&id).expect("op exists");
                op.state = OpState::Pending {
                    deadline: self.now + self.config.op_timeout_ms,
                };
                op.stale_answer = stale_answer;
                effects.push(Effect::Dispatch {
                    op: id,
                    command: Command::MoveToWorkspaceSilent { window, target },
                });
            }
        }
    }

    /// Confirms lane heads whose window is on the destination. With a
    /// fresh window query, a timed-out move that is not there has failed.
    fn resolve_ops(&mut self, source: Source, effects: &mut Vec<Effect>) {
        let heads: Vec<OpId> = self
            .lanes
            .values()
            .filter_map(|l| l.front().copied())
            .collect();
        for id in heads {
            let op = self.ops.get_mut(&id).expect("op exists");
            if !matches!(op.state, OpState::Pending { .. } | OpState::Unknown) {
                continue;
            }
            // While the window model is doubtful, wait for a fresh answer.
            if source == Source::Event && (!op.accepted || self.doubtful) {
                continue;
            }
            if source != Source::Event && op.stale_answer {
                op.stale_answer = false;
                continue;
            }
            let op = &self.ops[&id];
            let there = self
                .model
                .window(op.window)
                .is_some_and(|w| w.workspace == op.target.workspace());
            if there {
                self.finish(id, Outcome::Confirmed, effects);
            } else if source == Source::FreshQuery && op.state == OpState::Unknown {
                self.finish(id, Outcome::Failed(Failure::NotObserved), effects);
            }
        }
    }

    /// Fails every operation on a window that no longer exists.
    fn fail_window(&mut self, window: WindowId, effects: &mut Vec<Effect>) {
        let Some(lane) = self.lanes.get(&window.address) else {
            return;
        };
        let ids: Vec<OpId> = lane
            .iter()
            .copied()
            .filter(|id| self.ops[id].window == window)
            .collect();
        for id in ids {
            self.finish(id, Outcome::Failed(Failure::WindowGone), effects);
        }
    }

    /// Ends an operation and starts the next one on the same window.
    fn finish(&mut self, id: OpId, outcome: Outcome, effects: &mut Vec<Effect>) {
        let address = self.ops[&id].window.address;
        let was_head = self.lanes.get(&address).and_then(|l| l.front()) == Some(&id);
        self.set_done(id, outcome, effects);
        if let Some(lane) = self.lanes.get_mut(&address) {
            lane.retain(|op| *op != id);
            if lane.is_empty() {
                self.lanes.remove(&address);
            } else if was_head {
                let next = *lane.front().expect("non-empty");
                self.start(next, effects);
            }
        }
    }

    fn set_done(&mut self, id: OpId, outcome: Outcome, effects: &mut Vec<Effect>) {
        let op = self.ops.get_mut(&id).expect("op exists");
        if matches!(op.state, OpState::Done(_)) {
            return;
        }
        op.state = OpState::Done(outcome.clone());
        effects.push(Effect::Finished { op: id, outcome });
        self.done.push_back(id);
        while self.done.len() > DONE_KEPT {
            let oldest = self.done.pop_front().expect("non-empty");
            self.ops.remove(&oldest);
        }
    }
}
