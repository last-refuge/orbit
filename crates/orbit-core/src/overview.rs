//! The overview's own state (plan §03 and §06): which workspace is
//! previewed, what is selected, what is searched for, and which moves are
//! in flight. Pure like the store. The UI renders [`Tile`]s and [`Card`]s
//! from it and turns pointer and keyboard input into the [`Intent`]s it
//! returns, so drag, menu and keys share one availability check
//! ([`Overview::move_target`]), and every pointer action has a keyboard
//! path ([`KEYBOARD_PATHS`]).

use std::collections::BTreeMap;

use crate::{
    model::{Model, OpId, Window, WindowId, WorkspaceId, WorkspaceKind},
    search::{Field, Query, app_name},
    store::{Failure, Outcome, RequestError, Target},
};

/// Every pointer action and its keyboard equivalent (plan §01: keyboard
/// and pointer users receive equivalent paths). The overview's hint bar
/// and `orbit state` show it; tests and `scripts/review-check.sh` walk it.
pub const KEYBOARD_PATHS: [(&str, &str); 8] = [
    (
        "drag a numbered workspace to swap",
        "Tab to the workspaces, Ctrl+Shift+Left/Right",
    ),
    ("click a window", "arrows to select it, Enter"),
    ("click a workspace", "Tab to the workspaces, arrows, Enter"),
    (
        "drag a window onto a workspace",
        "Ctrl+Shift+M, arrows or type a name, Enter",
    ),
    (
        "Shift-drop a window (move and follow)",
        "Ctrl+Shift+M, choose, Shift+Enter",
    ),
    ("click the search field", "Ctrl+K, or type anywhere"),
    ("click a search result", "type, arrows to choose, Enter"),
    ("hover a workspace (no effect)", "none needed"),
];

/// A rail entry: a workspace that exists, or a fresh numbered one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Dest {
    Existing(WorkspaceId),
    /// The first unused number after the highest numbered workspace.
    NewNumbered(i64),
}

impl Dest {
    pub fn target(self) -> Target {
        match self {
            Dest::Existing(id) => Target::Workspace(id),
            Dest::NewNumbered(n) => Target::Numbered(n),
        }
    }
}

/// Where keyboard focus is, in Tab order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    Search,
    Rail,
    Grid,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tile {
    pub dest: Dest,
    pub label: String,
    pub kind: WorkspaceKind,
    pub monitor: Option<String>,
    pub windows: usize,
    /// Shown on a monitor right now.
    pub active: bool,
    /// Its windows fill the grid.
    pub previewed: bool,
    /// Keyboard focus is on it.
    pub focused: bool,
    /// The window whose preview stands for the workspace in the rail.
    pub cover: Option<WindowId>,
}

impl Tile {
    pub fn is_new(&self) -> bool {
        matches!(self.dest, Dest::NewNumbered(_))
    }

    /// The line under the name: counts and states in words, so they do not
    /// rely on color.
    pub fn caption(&self) -> String {
        if self.is_new() {
            return "Add workspace".into();
        }
        let count = match self.windows {
            0 => "Empty".to_string(),
            1 => "1 window".into(),
            n => format!("{n} windows"),
        };
        match (self.active, self.previewed) {
            (true, _) => format!("Active · {count}"),
            (false, true) => format!("Previewing · {count}"),
            (false, false) => count,
        }
    }

    /// What assistive technology reads for the tile.
    pub fn accessible_label(&self) -> String {
        if self.is_new() {
            return "New workspace".into();
        }
        let kind = match self.kind {
            WorkspaceKind::Special => "Special workspace",
            _ => "Workspace",
        };
        let mut parts = vec![format!("{kind} {}", self.label), self.windows_text()];
        if self.active {
            parts.push("active".into());
        }
        if self.previewed {
            parts.push("previewed".into());
        }
        if let Some(monitor) = &self.monitor {
            parts.push(format!("on {monitor}"));
        }
        parts.join(", ")
    }

    fn windows_text(&self) -> String {
        match self.windows {
            0 => "no windows".into(),
            1 => "1 window".into(),
            n => format!("{n} windows"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Card {
    pub window: WindowId,
    pub class: String,
    /// [`app_name`] of the class.
    pub app: String,
    pub title: String,
    /// The label of the workspace it is on, and that workspace's monitor;
    /// search results show both.
    pub workspace: String,
    pub monitor: Option<String>,
    pub floating: bool,
    pub fullscreen: bool,
    pub xwayland: bool,
    pub selected: bool,
    /// Label of the destination of a move still in flight.
    pub moving_to: Option<String>,
    /// The window's logical size, `(0, 0)` when unknown; previews keep
    /// its aspect.
    pub size: (i64, i64),
}

impl Card {
    /// The title, or the app name for windows without one.
    pub fn heading(&self) -> &str {
        if self.title.is_empty() {
            &self.app
        } else {
            &self.title
        }
    }

    pub fn badges(&self) -> Vec<&'static str> {
        let mut badges = Vec::new();
        if self.fullscreen {
            badges.push("Fullscreen");
        }
        if self.floating {
            badges.push("Floating");
        }
        if self.xwayland {
            badges.push("XWayland");
        }
        badges
    }

    /// What assistive technology reads for the card: the app, the full
    /// title, where it is and its states.
    pub fn accessible_label(&self) -> String {
        let mut parts = vec![if self.title.is_empty() {
            self.app.clone()
        } else {
            format!("{}: {}", self.app, self.title)
        }];
        parts.push(format!("workspace {}", self.workspace));
        if let Some(monitor) = &self.monitor {
            parts.push(format!("on {monitor}"));
        }
        parts.extend(self.badges().iter().map(|b| b.to_lowercase()));
        if let Some(to) = &self.moving_to {
            parts.push(format!("moving to {to}"));
        }
        parts.join(", ")
    }
}

/// What the user asked for, for the platform side to carry out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Intent {
    /// Exchange complete numbered workspaces in the compositor.
    SwapWorkspaces {
        source: WorkspaceId,
        target: WorkspaceId,
    },
    /// Focus this window (switching to its workspace) and dismiss.
    Activate(WindowId),
    /// Switch to this workspace and dismiss.
    Switch(Dest),
    /// Move without leaving the overview; with `follow`, activate it once
    /// the move is confirmed.
    Move {
        window: WindowId,
        dest: Dest,
        follow: bool,
    },
    Dismiss,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    Info,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    pub text: String,
    pub tone: Tone,
}

/// Why a move cannot be offered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unavailable {
    WindowGone,
    SameWorkspace,
    DestinationGone,
}

#[derive(Clone, Debug)]
struct Pending {
    window: WindowId,
    label: String,
    follow: bool,
}

/// Where the overview was opened from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Invocation {
    pub monitor: Option<String>,
    pub workspace: Option<WorkspaceId>,
    pub window: Option<WindowId>,
}

#[derive(Clone, Debug)]
pub struct Overview {
    pub invocation: Invocation,
    pub previewed: Option<WorkspaceId>,
    pub region: Region,
    /// Keyboard focus in the rail.
    pub rail_focus: Option<Dest>,
    pub selected: Option<WindowId>,
    /// Choosing a destination for this window ("Move to workspace").
    pub moving: Option<WindowId>,
    pub announcement: Option<Announcement>,
    /// Searches all windows and workspaces while it has text.
    pub search: Field,
    /// Narrows the move menu's destinations.
    pub destination: Field,
    /// Matches for [`Self::search`], ranked; kept up to date by every edit
    /// and [`Self::reconcile`], so rendering never searches.
    hits: Hits,
    pending: BTreeMap<OpId, Pending>,
    /// Settings: leave special workspaces out of the rail; search only
    /// the previewed workspace.
    pub hide_special: bool,
    pub search_this_workspace: bool,
    pub swapping: bool,
}

#[derive(Clone, Debug, Default)]
struct Hits {
    windows: Vec<WindowId>,
    workspaces: Vec<WorkspaceId>,
}

impl Overview {
    /// Opens on `monitor` (default: the focused one), previewing its active
    /// workspace with the focused window selected.
    pub fn open(model: &Model, monitor: Option<&str>) -> Self {
        let monitor = model
            .monitors
            .iter()
            .find(|m| Some(m.name.as_str()) == monitor)
            .or_else(|| model.monitors.iter().find(|m| m.focused))
            .or_else(|| model.monitors.first());
        let workspace = monitor.map(|m| m.active_workspace);
        let mut overview = Self {
            invocation: Invocation {
                monitor: monitor.map(|m| m.name.clone()),
                workspace,
                window: model.active_window,
            },
            previewed: workspace,
            region: Region::Grid,
            rail_focus: workspace.map(Dest::Existing),
            selected: None,
            moving: None,
            announcement: None,
            search: Field::default(),
            destination: Field::default(),
            hits: Hits::default(),
            pending: BTreeMap::new(),
            hide_special: false,
            search_this_workspace: false,
            swapping: false,
        };
        overview.selected = model
            .active_window
            .filter(|id| overview.shows(model, *id))
            .or_else(|| overview.first_card(model));
        overview
    }

    /// True while the search field has terms: the grid and rail show
    /// matches from every workspace instead of the previewed one.
    pub fn searching(&self) -> bool {
        !Query::parse(self.search.text()).is_empty()
    }

    /// The field typing goes to: the move menu's filter while it is open.
    pub fn editing(&self) -> &Field {
        if self.moving.is_some() {
            &self.destination
        } else {
            &self.search
        }
    }

    /// Rail entries. Normally every workspace in rail order (numbered, then
    /// named, then special, then a fresh numbered destination); while
    /// searching, the matching workspaces best first; in the move menu, the
    /// destinations its filter matches.
    pub fn tiles(&self, model: &Model) -> Vec<Tile> {
        let all = self.all_tiles(model);
        if self.moving.is_some() {
            let query = Query::parse(self.destination.text());
            return all
                .into_iter()
                .filter(|t| query.score(&[&t.label]).is_some())
                .collect();
        }
        if self.searching() {
            return self
                .hits
                .workspaces
                .iter()
                .filter_map(|id| all.iter().find(|t| t.dest == Dest::Existing(*id)))
                .cloned()
                .collect();
        }
        all
    }

    /// Exchange complete numbered workspaces, preserving compositor layout objects.
    pub fn reorder_workspace(
        &self,
        model: &Model,
        source: WorkspaceId,
        target: Dest,
        _after: bool,
    ) -> Option<Intent> {
        let Dest::Existing(target) = target else {
            return None;
        };
        if self.searching()
            || self.moving.is_some()
            || self.swapping
            || !self.pending.is_empty()
            || source == target
        {
            return None;
        }
        let numbered = |id| {
            model
                .workspaces
                .get(&id)
                .is_some_and(|w| w.kind() == WorkspaceKind::Numbered && id.0 < u32::MAX as i64)
        };
        (numbered(source) && numbered(target)).then_some(Intent::SwapWorkspaces { source, target })
    }

    /// Workspace identity stays intact when Hyprland changes its number.
    pub fn workspace_id_changed(&mut self, old: WorkspaceId, new: WorkspaceId) {
        if self.previewed == Some(old) {
            self.previewed = Some(new);
        }
        if self.rail_focus == Some(Dest::Existing(old)) {
            self.rail_focus = Some(Dest::Existing(new));
        }
        if self.invocation.workspace == Some(old) {
            self.invocation.workspace = Some(new);
        }
    }

    pub fn reorder_focused(&self, model: &Model, delta: isize) -> Option<Intent> {
        if self.region != Region::Rail || delta == 0 {
            return None;
        }
        let Dest::Existing(source) = self.rail_focus? else {
            return None;
        };
        let tiles = self.tiles(model);
        let index = tiles
            .iter()
            .position(|t| t.dest == Dest::Existing(source))?;
        let next = index.checked_add_signed(delta.signum())?;
        let target = tiles.get(next)?;
        if target.is_new() {
            return None;
        }
        self.reorder_workspace(model, source, target.dest, delta > 0)
    }

    fn all_tiles(&self, model: &Model) -> Vec<Tile> {
        let shown = |id: WorkspaceId| {
            model
                .monitors
                .iter()
                .any(|m| m.active_workspace == id || m.special_workspace == Some(id))
        };
        let mut tiles: Vec<Tile> = rail_order(model)
            .into_iter()
            .filter_map(|id| model.workspaces.get(&id))
            .filter(|w| !(self.hide_special && w.kind() == WorkspaceKind::Special))
            .map(|w| Tile {
                dest: Dest::Existing(w.id),
                label: label(&w.name),
                kind: w.kind(),
                monitor: w.monitor.clone(),
                windows: model.windows_on(w.id).filter(|w| !w.hidden).count(),
                active: shown(w.id),
                previewed: self.previewed == Some(w.id),
                focused: self.region == Region::Rail
                    && self.rail_focus == Some(Dest::Existing(w.id)),
                cover: cover(model, w.id),
            })
            .collect();
        let next = fresh_number(model);
        tiles.push(Tile {
            dest: Dest::NewNumbered(next),
            label: "New".into(),
            kind: WorkspaceKind::Numbered,
            monitor: None,
            windows: 0,
            active: false,
            previewed: false,
            focused: self.region == Region::Rail
                && self.rail_focus == Some(Dest::NewNumbered(next)),
            cover: None,
        });
        tiles
    }

    /// Windows to capture while open: every card, then each rail tile's
    /// cover, without repeats.
    pub fn wanted(&self, model: &Model) -> Vec<WindowId> {
        let mut wanted: Vec<WindowId> = self.cards(model).iter().map(|c| c.window).collect();
        for tile in self.tiles(model) {
            if let Some(cover) = tile.cover
                && !wanted.contains(&cover)
            {
                wanted.push(cover);
            }
        }
        wanted
    }

    /// Windows on the previewed workspace, oldest first, so cards keep
    /// their places as windows come and go; while searching, the matching
    /// windows from every workspace, best first.
    pub fn cards(&self, model: &Model) -> Vec<Card> {
        let windows: Vec<&Window> = if self.searching() {
            self.hits
                .windows
                .iter()
                .filter_map(|id| model.window(*id))
                .filter(|w| !self.search_this_workspace || Some(w.workspace) == self.previewed)
                .collect()
        } else {
            let Some(workspace) = self.previewed else {
                return Vec::new();
            };
            let mut windows: Vec<_> = model.windows_on(workspace).filter(|w| !w.hidden).collect();
            windows.sort_by_key(|w| w.id.generation);
            windows
        };
        windows
            .into_iter()
            .map(|w| {
                let workspace = model.workspaces.get(&w.workspace);
                Card {
                    window: w.id,
                    class: w.class.clone(),
                    app: app_name(&w.class),
                    title: w.title.clone(),
                    workspace: workspace
                        .map_or_else(|| w.workspace.0.to_string(), |ws| label(&ws.name)),
                    monitor: workspace.and_then(|ws| ws.monitor.clone()),
                    floating: w.floating,
                    fullscreen: w.fullscreen,
                    xwayland: w.xwayland,
                    selected: self.selected == Some(w.id),
                    moving_to: self
                        .pending
                        .values()
                        .find(|p| p.window == w.id)
                        .map(|p| p.label.clone()),
                    size: w.size,
                }
            })
            .collect()
    }

    /// Keeps selection, preview and search results valid after the model
    /// changed.
    pub fn reconcile(&mut self, model: &Model) {
        if self
            .previewed
            .is_some_and(|id| !model.workspaces.contains_key(&id))
        {
            self.previewed = self
                .invocation
                .workspace
                .filter(|id| model.workspaces.contains_key(id))
                .or_else(|| {
                    model
                        .monitors
                        .iter()
                        .find(|m| m.focused)
                        .map(|m| m.active_workspace)
                });
        }
        self.hits = self.find(model);
        if let Some(Dest::Existing(id)) = self.rail_focus
            && !model.workspaces.contains_key(&id)
        {
            self.rail_focus = self.previewed.map(Dest::Existing);
        }
        if let Some(Dest::NewNumbered(n)) = self.rail_focus
            && fresh_number(model) != n
        {
            self.rail_focus = Some(Dest::NewNumbered(fresh_number(model)));
        }
        if self.selected.is_none_or(|id| !self.shows(model, id)) {
            self.selected = self.first_card(model);
        }
        if self.moving.is_some_and(|id| model.window(id).is_none()) {
            self.close_menu();
        }
    }

    /// Tab (`back`: Shift+Tab) cycles search → workspaces → windows. The
    /// move menu keeps focus on its destinations.
    pub fn tab(&mut self, back: bool) {
        if self.moving.is_some() {
            return;
        }
        let order = [Region::Search, Region::Rail, Region::Grid];
        let at = order.iter().position(|r| *r == self.region).unwrap_or(0);
        let next = if back { at + order.len() - 1 } else { at + 1 };
        self.region = order[next % order.len()];
    }

    /// Ctrl+K, or a click on the search field.
    pub fn focus_search(&mut self) {
        if self.moving.is_none() {
            self.region = Region::Search;
        }
    }

    /// Left/Right: the caret in search, else the previous or next tile or
    /// card. In the rail this previews the workspace, except while
    /// searching or choosing a move destination.
    pub fn horizontal(&mut self, model: &Model, delta: isize) {
        match self.region {
            Region::Search if delta < 0 => self.search.left(),
            Region::Search => self.search.right(),
            _ => self.step(model, delta),
        }
    }

    /// Up: a row up in the grid, from its first row to the rail, from the
    /// rail to search.
    pub fn up(&mut self, model: &Model, columns: isize) {
        let first_row = self
            .cards(model)
            .iter()
            .position(|c| c.selected)
            .is_none_or(|i| (i as isize) < columns);
        match self.region {
            Region::Grid if first_row => self.region = Region::Rail,
            Region::Grid => self.step(model, -columns),
            Region::Rail if self.moving.is_none() => self.region = Region::Search,
            Region::Rail | Region::Search => {}
        }
    }

    /// Down: from search to the rail, from the rail to the grid, then a row
    /// down.
    pub fn down(&mut self, model: &Model, columns: isize) {
        match self.region {
            Region::Search => self.region = Region::Rail,
            Region::Rail if self.moving.is_none() => self.region = Region::Grid,
            Region::Rail => {}
            Region::Grid => self.step(model, columns),
        }
    }

    /// Home/End: the start or end of the search text, or the first or last
    /// tile or card.
    pub fn home(&mut self, model: &Model) {
        match self.region {
            Region::Search => self.search.home(),
            _ => self.step(model, -isize::MAX / 2),
        }
    }

    pub fn end(&mut self, model: &Model) {
        match self.region {
            Region::Search => self.search.end(),
            _ => self.step(model, isize::MAX / 2),
        }
    }

    /// Moves keyboard focus `delta` places along the focused region.
    pub fn step(&mut self, model: &Model, delta: isize) {
        match self.region {
            Region::Search => {}
            Region::Grid => {
                let cards = self.cards(model);
                if cards.is_empty() {
                    return;
                }
                let at = cards
                    .iter()
                    .position(|c| Some(c.window) == self.selected)
                    .map_or(0, |i| clamp(i as isize + delta, cards.len()));
                self.selected = Some(cards[at].window);
            }
            Region::Rail => {
                let tiles = self.rail_choices(model);
                if tiles.is_empty() {
                    return;
                }
                let at = tiles
                    .iter()
                    .position(|d| Some(*d) == self.rail_focus)
                    .map_or(0, |i| clamp(i as isize + delta, tiles.len()));
                let dest = tiles[at];
                self.rail_focus = Some(dest);
                if self.moving.is_none() && !self.searching() {
                    self.preview(model, dest);
                }
            }
        }
    }

    /// Rail tiles keyboard focus can land on; in the move menu only valid
    /// destinations.
    fn rail_choices(&self, model: &Model) -> Vec<Dest> {
        let mut tiles: Vec<Dest> = self.tiles(model).into_iter().map(|t| t.dest).collect();
        if let Some(window) = self.moving {
            tiles.retain(|d| self.move_target(model, window, *d).is_ok());
        }
        tiles
    }

    /// Shows a workspace's windows without switching to it.
    pub fn preview(&mut self, model: &Model, dest: Dest) {
        if let Dest::Existing(id) = dest
            && let Some(workspace) = model.workspaces.get(&id)
            && self.previewed != Some(id)
        {
            self.previewed = Some(id);
            self.selected = self.first_card(model);
            let windows = model.windows_on(id).filter(|w| !w.hidden).count();
            let count = match windows {
                0 => "no windows".to_string(),
                1 => "1 window".into(),
                n => format!("{n} windows"),
            };
            self.announce(
                format!("Previewing {}, {count}", label(&workspace.name)),
                Tone::Info,
            );
        }
    }

    pub fn select(&mut self, model: &Model, window: WindowId) {
        if self.shows(model, window) {
            self.selected = Some(window);
            self.region = Region::Grid;
        }
    }

    // Typing. Text goes to the field [`Self::editing`] names; typing while
    // the rail or grid has focus starts a search.

    pub fn type_text(&mut self, model: &Model, text: &str) {
        self.edit(model, |field| field.insert(text));
    }

    pub fn backspace(&mut self, model: &Model) {
        if !self.editing().is_empty() {
            self.edit(model, Field::backspace);
        }
    }

    pub fn delete(&mut self, model: &Model) {
        if self.region == Region::Search || self.moving.is_some() {
            self.edit(model, Field::delete);
        }
    }

    /// Applies an edit (including the platform input method's) to the
    /// field being edited, then refreshes what it filters.
    pub fn edit(&mut self, model: &Model, f: impl FnOnce(&mut Field)) {
        if self.moving.is_some() {
            f(&mut self.destination);
            // Keep focus on a destination the filter still shows.
            let choices = self.rail_choices(model);
            if !self.rail_focus.is_some_and(|d| choices.contains(&d)) {
                self.rail_focus = choices.first().copied();
            }
            return;
        }
        let before = self.search.text().to_owned();
        self.region = Region::Search;
        f(&mut self.search);
        if self.search.text() == before {
            return;
        }
        self.hits = self.find(model);
        self.selected = self.first_card(model);
        if self.searching() {
            self.rail_focus = self.hits.workspaces.first().map(|id| Dest::Existing(*id));
            let text = match (self.hits.windows.len(), self.hits.workspaces.len()) {
                (0, 0) => "No matches".to_string(),
                (w, s) => format!("{} and {}", plural(w, "window"), plural(s, "workspace")),
            };
            self.announce(text, Tone::Info);
        } else {
            self.rail_focus = self.previewed.map(Dest::Existing);
        }
    }

    /// Ranks windows and workspaces against the search text. Windows match
    /// on app name, class, title and workspace; workspaces on their label.
    fn find(&self, model: &Model) -> Hits {
        let query = Query::parse(self.search.text());
        if query.is_empty() {
            return Hits::default();
        }
        let order = rail_order(model);
        let rank = |id: WorkspaceId| order.iter().position(|w| *w == id).unwrap_or(usize::MAX);
        let mut windows: Vec<_> = model
            .windows
            .values()
            .filter(|w| !w.hidden)
            .filter_map(|w| {
                let workspace = model
                    .workspaces
                    .get(&w.workspace)
                    .map(|ws| label(&ws.name))
                    .unwrap_or_default();
                let app = app_name(&w.class);
                query
                    .score(&[&app, &w.class, &w.title, &workspace])
                    .map(|score| ((score, rank(w.workspace), w.id.generation), w.id))
            })
            .collect();
        windows.sort();
        let mut workspaces: Vec<_> = order
            .iter()
            .enumerate()
            .filter_map(|(at, id)| {
                let name = label(&model.workspaces.get(id)?.name);
                query.score(&[&name]).map(|score| ((score, at), *id))
            })
            .collect();
        workspaces.sort();
        Hits {
            windows: windows.into_iter().map(|(_, id)| id).collect(),
            workspaces: workspaces.into_iter().map(|(_, id)| id).collect(),
        }
    }

    /// Ctrl+Shift+M: choose a destination for the selected window.
    pub fn begin_move(&mut self, model: &Model) -> bool {
        let Some(window) = self.selected.filter(|id| model.window(*id).is_some()) else {
            return false;
        };
        self.moving = Some(window);
        self.destination.clear();
        self.region = Region::Rail;
        // Start on a valid destination: the focused tile if it is one, else
        // the first that is.
        let choices = self.rail_choices(model);
        if !self.rail_focus.is_some_and(|d| choices.contains(&d)) {
            self.rail_focus = choices.first().copied();
        }
        true
    }

    /// Enter. `follow` is Shift+Enter: when moving, follow the window.
    /// In search, Enter takes the best match (the selected window, else the
    /// first workspace); not while an input method is composing.
    pub fn enter(&mut self, model: &Model, follow: bool) -> Option<Intent> {
        match (self.region, self.moving) {
            (Region::Rail, Some(window)) => {
                let dest = self.rail_focus?;
                self.close_menu();
                self.move_intent(model, window, dest, follow)
            }
            (Region::Rail, None) => self.rail_focus.map(Intent::Switch),
            (Region::Search, _) if self.search.composing() => None,
            (Region::Search, _) if self.searching() => {
                self.activate_selected(model).or_else(|| {
                    self.hits
                        .workspaces
                        .first()
                        .map(|id| Intent::Switch(Dest::Existing(*id)))
                })
            }
            (Region::Search | Region::Grid, _) => self.activate_selected(model),
        }
    }

    fn activate_selected(&self, model: &Model) -> Option<Intent> {
        self.selected
            .filter(|id| model.window(*id).is_some())
            .map(Intent::Activate)
    }

    /// Escape: close the move menu, then clear the search, then dismiss.
    /// (A drag in progress is cancelled by the UI before this.)
    pub fn escape(&mut self, model: &Model) -> Option<Intent> {
        if self.moving.is_some() {
            self.close_menu();
            return None;
        }
        if !self.search.is_empty() {
            self.edit(model, Field::clear);
            return None;
        }
        Some(Intent::Dismiss)
    }

    fn close_menu(&mut self) {
        self.moving = None;
        self.destination.clear();
        self.region = Region::Grid;
    }

    /// A drop of `window` on a rail tile.
    pub fn drop_on(
        &mut self,
        model: &Model,
        window: WindowId,
        dest: Dest,
        follow: bool,
    ) -> Option<Intent> {
        match self.move_target(model, window, dest) {
            // Dropping on the window's own workspace is a no-op.
            Err(Unavailable::SameWorkspace) => None,
            _ => self.move_intent(model, window, dest, follow),
        }
    }

    /// The one availability check for drag, menu and keys.
    pub fn move_target(
        &self,
        model: &Model,
        window: WindowId,
        dest: Dest,
    ) -> Result<Target, Unavailable> {
        let current = model.window(window).ok_or(Unavailable::WindowGone)?;
        match dest {
            Dest::Existing(id) if !model.workspaces.contains_key(&id) => {
                Err(Unavailable::DestinationGone)
            }
            Dest::Existing(id) if current.workspace == id => Err(Unavailable::SameWorkspace),
            _ => Ok(dest.target()),
        }
    }

    fn move_intent(
        &mut self,
        model: &Model,
        window: WindowId,
        dest: Dest,
        follow: bool,
    ) -> Option<Intent> {
        match self.move_target(model, window, dest) {
            Ok(_) => Some(Intent::Move {
                window,
                dest,
                follow,
            }),
            Err(reason) => {
                self.announce(unavailable_text(&reason), Tone::Error);
                None
            }
        }
    }

    /// The store accepted a move request.
    pub fn track(&mut self, model: &Model, op: OpId, window: WindowId, dest: Dest, follow: bool) {
        let label = dest_label(model, dest);
        self.announce(format!("Moving to {label}…"), Tone::Info);
        self.pending.insert(
            op,
            Pending {
                window,
                label,
                follow,
            },
        );
    }

    /// The store refused a move request.
    pub fn refused(&mut self, error: &RequestError) {
        let text = match error {
            RequestError::NotLive => "Not connected to Hyprland yet",
            RequestError::StaleWindow => "That window is gone",
            RequestError::Unverified => "That window is still loading; try again",
            RequestError::UnknownDestination => "That workspace is gone",
            RequestError::AlreadyThere => "The window is already there",
        };
        self.announce(text.into(), Tone::Error);
    }

    /// A tracked move finished. Returns the follow-up for "move and follow".
    pub fn finished(&mut self, op: OpId, outcome: &Outcome) -> Option<Intent> {
        let pending = self.pending.remove(&op)?;
        let label = &pending.label;
        match outcome {
            Outcome::Confirmed => {
                self.announce(format!("Moved to {label}"), Tone::Info);
                pending.follow.then_some(Intent::Activate(pending.window))
            }
            Outcome::Failed(failure) => {
                let text = match failure {
                    Failure::WindowGone => "The window closed before it moved".into(),
                    Failure::DestinationGone => format!("{label} no longer exists"),
                    Failure::SessionRestarted => {
                        "Hyprland reconnected; the move was not confirmed".into()
                    }
                    Failure::Rejected(reason) => format!("Hyprland refused the move: {reason}"),
                    Failure::NotObserved => format!("Couldn't confirm the move to {label}"),
                };
                self.announce(text, Tone::Error);
                None
            }
        }
    }

    pub fn pending_moves(&self) -> usize {
        self.pending.len()
    }

    fn announce(&mut self, text: String, tone: Tone) {
        self.announcement = Some(Announcement { text, tone });
    }

    /// Whether the window has a card: on the previewed workspace, or a
    /// search match.
    fn shows(&self, model: &Model, id: WindowId) -> bool {
        if self.searching() {
            return self.hits.windows.contains(&id) && model.window(id).is_some();
        }
        model
            .window(id)
            .is_some_and(|w| Some(w.workspace) == self.previewed && !w.hidden)
    }

    fn first_card(&self, model: &Model) -> Option<WindowId> {
        self.cards(model).first().map(|c| c.window)
    }
}

/// How a workspace name reads in the rail and in announcements.
pub fn label(name: &str) -> String {
    name.strip_prefix("special:").unwrap_or(name).to_owned()
}

pub fn dest_label(model: &Model, dest: Dest) -> String {
    match dest {
        Dest::Existing(id) => model
            .workspaces
            .get(&id)
            .map_or_else(|| id.0.to_string(), |w| label(&w.name)),
        Dest::NewNumbered(n) => n.to_string(),
    }
}

fn unavailable_text(reason: &Unavailable) -> String {
    match reason {
        Unavailable::WindowGone => "That window is gone".into(),
        Unavailable::SameWorkspace => "The window is already there".into(),
        Unavailable::DestinationGone => "That workspace is gone".into(),
    }
}

/// The window a workspace thumbnail shows: a fullscreen one, else the
/// oldest, so the thumbnail stays put as windows come and go.
pub fn cover(model: &Model, workspace: WorkspaceId) -> Option<WindowId> {
    model
        .windows_on(workspace)
        .filter(|w| !w.hidden)
        .min_by_key(|w| (!w.fullscreen, w.id.generation))
        .map(|w| w.id)
}

/// Workspace ids in rail order: numbered, then named, then special.
fn rail_order(model: &Model) -> Vec<WorkspaceId> {
    let mut workspaces: Vec<_> = model.workspaces.values().collect();
    workspaces.sort_by_key(|w| {
        let rank = match w.kind() {
            WorkspaceKind::Numbered => 0,
            WorkspaceKind::Named => 1,
            WorkspaceKind::Special => 2,
        };
        (rank, if rank == 0 { w.id.0 } else { 0 }, w.name.clone())
    });
    workspaces.into_iter().map(|w| w.id).collect()
}

fn plural(n: usize, noun: &str) -> String {
    match n {
        0 => format!("no {noun}s"),
        1 => format!("1 {noun}"),
        n => format!("{n} {noun}s"),
    }
}

fn fresh_number(model: &Model) -> i64 {
    model
        .workspaces
        .values()
        .filter(|w| w.kind() == WorkspaceKind::Numbered)
        .map(|w| w.id.0)
        .max()
        .unwrap_or(0)
        + 1
}

fn clamp(index: isize, len: usize) -> usize {
    index.clamp(0, len as isize - 1) as usize
}
