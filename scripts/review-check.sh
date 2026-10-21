#!/usr/bin/env bash
# ORB-007 automated review: search, labels and keyboard parity, through the
# overview's real input path (GPUI keys, text input, clicks and drags via
# `orbit drive`), checked against Hyprland's own state. ORB-008 adds the
# measurable design targets (docs/orb-008-design-targets.md): panel bounds
# and centering, the fixed grid, previews drawn at their captured device
# size, the theme following Hyprland's blur, and that only fixture windows
# are ever captured. The native visual and screen-reader review is manual;
# see README "Brand, search and keyboard (ORB-007)" and "Brand fidelity".
#
#   scripts/review-check.sh [path/to/orbit]          on Hyprland, with fixtures
#   scripts/review-check.sh --fake [path/to/orbit]   against fake-hyprland.py
#
# Starts its own `orbit daemon --test-hooks`. Only windows whose class starts
# with dev.orbit.fixture. are ever moved, focused or clicked; before every
# activation the script checks the selection is a fixture. It never takes
# screenshots and prints no titles (`orbit state` and `drive a11y` have
# none). Results go to docs/evidence/orb-007/<host>/. Needs jq; on Hyprland
# also hyprctl.
set -uo pipefail

fake=false
[[ ${1:-} == --fake ]] && { fake=true; shift; }
orbit=$(realpath "${1:-target/release/orbit}")
[[ -x $orbit ]] || { echo "build first: cargo build --release" >&2; exit 1; }
here=$(cd "$(dirname "$0")/.." && pwd)
host=$(hostname -s)
$fake && host=$host-fake
out=$here/docs/evidence/orb-007/$host
mkdir -p "$out"
results=$out/results.txt
: > "$results"

if $fake; then
    hypr() { python3 "$here/scripts/fake-hyprland.py" ctl "$@"; }
else
    hypr() { hyprctl "$@"; }
fi

failures=0
report() { # report PASS|FAIL|SKIP message
    printf '%s  %s\n' "$1" "$2" | tee -a "$results"
    [[ $1 == FAIL ]] && failures=$((failures + 1))
    return 0
}
check() { # check "message" command...
    local message=$1; shift
    if "$@"; then report PASS "$message"; else report FAIL "$message"; fi
}
must() { # like check, but stops the run on failure
    local message=$1; shift
    if "$@"; then report PASS "$message"; else report FAIL "$message (stopping)"; finish; fi
}

prefix=dev.orbit.fixture.
fixture() { hypr -j clients | jq -r --arg c "$prefix$1" '.[] | select(.class == $c) | .address'; }
workspace_of() { hypr -j clients | jq -r --arg a "$1" '.[] | select(.address == $a) | .workspace.name'; }
active_window() { hypr -j activewindow | jq -r '.address // "none"'; }
active_workspace() { hypr -j activeworkspace | jq -r '.name'; }
is_fixture() { hypr -j clients | jq -e --arg a "$1" --arg p "$prefix" \
    'any(.[]; .address == $a and (.class | startswith($p)))' >/dev/null; }
focus_workspace() {
    if $fake; then hypr dispatch "hl.dsp.focus({ workspace = '$1' })" >/dev/null
    else hyprctl dispatch "hl.dsp.focus({ workspace = '$1' })" >/dev/null 2>&1 || hyprctl dispatch workspace "$1" >/dev/null; fi
}

alive() { [[ -n $daemon_pid ]] && kill -0 "$daemon_pid" 2>/dev/null; }
state() {
    "$orbit" state 2>/dev/null && return 0
    alive || { report FAIL "the daemon exited (see $out/daemon.log)"; finish; }
    return 1
}
field() { state | jq -r "$1"; }
wait_for() { # wait_for JQ_EXPR: up to 3 s for `orbit state | jq -e EXPR`
    for _ in $(seq 60); do state | jq -e "$1" >/dev/null 2>&1 && return 0; sleep 0.05; done
    return 1
}
drive() { "$orbit" drive "$@" >/dev/null; }
key() { drive key "$1"; }
guard() { # guard ADDRESS: refuse anything that is not a fixture
    is_fixture "$1" || { report FAIL "refusing to touch non-fixture window $1"; finish; }
}
selected_is() { # the selection is exactly this fixture, so Enter is safe
    wait_for ".overview.selected == \"$1\""
}
focused() { # focused ADDRESS: up to 2 s for Hyprland to report it active
    for _ in $(seq 40); do [[ $(active_window) == "$1" ]] && return 0; sleep 0.05; done
    return 1
}
press_until() { # press_until KEY JQ_EXPR: at most 12 presses
    for _ in $(seq 12); do
        state | jq -e "$2" >/dev/null && return 0
        key "$1" || return 1
    done
    state | jq -e "$2" >/dev/null
}
preview() { # previews workspace NAME from the rail, then focuses the windows
    press_until tab '.overview.region == "Rail"' >/dev/null
    press_until left '.overview.rail_focus == (.overview.rail[0].label)' >/dev/null
    press_until right ".overview.previewed == \"$1\"" &&
        key tab && [[ $(field .overview.region) == Grid ]]
}
open() { # reopens the overview and waits for it
    "$orbit" show >/dev/null
    wait_for '.visible and .overview != null'
}
closed() { wait_for '.visible == false'; }

daemon_pid=
original_ws=
# The daemon and `orbit config` use a scratch settings file, never the
# person's own (ORBIT_CONFIG); the live bind it applies is released on quit.
export ORBIT_CONFIG=$out/config.toml
rm -f "$ORBIT_CONFIG"
# Release checks go to a fixture server, never to GitHub, during the run.
fixture_port=$((19500 + RANDOM % 400))
fixture_dir=$out/fixture-releases
mkdir -p "$fixture_dir/releases/v99.0.0"
touch "$fixture_dir/releases/v99.0.0/prerelease"
printf '## 99.0.0\n\nA fixture release for the review checks.\n' > "$fixture_dir/releases/v99.0.0/notes.md"
: > "$fixture_dir/releases/v99.0.0/orbit-99.0.0-$(uname -m)-unknown-linux-gnu.tar.gz"
: > "$fixture_dir/releases/v99.0.0/orbit-99.0.0-$(uname -m)-unknown-linux-gnu.tar.gz.sha256"
python3 -I "$here/scripts/fake-release-server.py" serve "$fixture_dir" "$fixture_port" >"$out/fixture-releases.log" 2>&1 &
fixture_pid=$!
export ORBIT_API_BASE=http://127.0.0.1:$fixture_port
export ORBIT_UPDATE_START_DELAY_MS=3600000
export XDG_STATE_HOME=$out/state
rm -rf "$XDG_STATE_HOME/orbit"
start_daemon() { # start_daemon [daemon options…]
    RUST_LOG=${RUST_LOG:-warn,orbit=debug} "$orbit" daemon --test-hooks "$@" >> "$out/daemon.log" 2>&1 &
    daemon_pid=$!
    for _ in $(seq 60); do "$orbit" status >/dev/null 2>&1 && break; sleep 0.05; done
}
stop_daemon() {
    "$orbit" hide >/dev/null 2>&1
    "$orbit" quit >/dev/null 2>&1
    [[ -n $daemon_pid ]] && wait "$daemon_pid" 2>/dev/null
    daemon_pid=
}
finish() {
    stop_daemon
    [[ -n ${fixture_pid:-} ]] && kill "$fixture_pid" 2>/dev/null
    if ! $fake; then "$here/scripts/fixtures.sh" clean >/dev/null 2>&1; fi
    [[ -n $original_ws ]] && focus_workspace "$original_ws"
    echo "$failures failure(s); results in $results"
    exit $((failures > 0))
}
trap finish INT TERM

# --- setup ------------------------------------------------------------------
: > "$out/daemon.log"
original_ws=$(active_workspace)
if ! $fake; then
    "$here/scripts/fixtures.sh" spawn "$orbit" >/dev/null || { report FAIL "fixtures spawn"; finish; }
fi
a=$(fixture tiled-a); b=$(fixture tiled-b)
check "fixtures present" test -n "$a" -a -n "$b"
[[ -n $a && -n $b ]] || finish
guard "$a"; guard "$b"
start_daemon
check "daemon connects to Hyprland" wait_for '.sync == "Live"'

# --- brand and labels -----------------------------------------------------------
must "open: overview visible, focus on the windows" open
check "open: keyboard focus starts on the windows" wait_for '.overview.region == "Grid"'
check "look: a translucent brand theme, full motion" \
    wait_for '(.look.theme == "standard" or .look.theme == "glass") and .look.opaque == false and .look.reduced_motion == false'
check "look: the frosted glass theme is the default" wait_for '.look.theme == "glass" and .look.glass'
check "icons: an icon theme is in use" wait_for '.look.icon_theme | length > 0'
check "privacy: capture is restricted to fixture windows in test-hooks mode" \
    wait_for '.capture_restricted and ([.capturing[].class] | all(startswith("dev.orbit.fixture.") or . == "OrbitFixtureX11"))'
check "privacy: no desktop backdrop is captured in test-hooks mode (solid fallback)" \
    wait_for '.backdrop == false'
check "look: the overview uses the bundled Inter" wait_for '.look.font == "Inter"'
check "rail: every workspace with windows has a cover window" \
    wait_for '[.overview.rail[] | select(.windows > 0) | .cover != null] | all'
check "rail: only the previewed workspace's windows are captured (tiles elsewhere are outlines)" \
    wait_for '[.overview.rail[] | select(.previewed | not) | select(.label == "91" or .label == "92" or .label == "93") | .cover_preview | not] | all'
check "labels: every card names its app and workspace" \
    wait_for '[.overview.cards[].labelled] | all'
check "labels: every tile says what it is, in words" \
    wait_for '[.overview.rail[] | .accessible_label | length > 0] | all'
check "labels: the active workspace's tile says active" \
    wait_for '[.overview.rail[] | select(.active) | .accessible_label | contains("active")] | all'
check "labels: the special workspace reads as special" \
    wait_for '[.overview.rail[] | select(.label == "orbitfx") | .accessible_label | startswith("Special workspace orbitfx")] | all'
check "labels: the add tile reads as New workspace" \
    wait_for '.overview.rail[-1].accessible_label == "New workspace"'
check "parity: seven pointer actions each list a keyboard path" \
    wait_for '[.overview.keyboard_paths[] | .keys | length > 0] | (length == 7 and all)'

# --- keyboard regions -------------------------------------------------------------
key tab
check "keys: Tab from the windows goes to search" wait_for '.overview.region == "Search"'
key tab
check "keys: Tab from search goes to the workspaces" wait_for '.overview.region == "Rail"'
key tab
check "keys: Tab from the workspaces goes to the windows" wait_for '.overview.region == "Grid"'
key shift-tab
check "keys: Shift+Tab goes back to the workspaces" wait_for '.overview.region == "Rail"'
key up
check "keys: Up from the workspaces goes to search" wait_for '.overview.region == "Search"'
key down
key down
check "keys: Down walks search, workspaces, windows" wait_for '.overview.region == "Grid"'
key ctrl-k
check "keys: Ctrl+K focuses search" wait_for '.overview.region == "Search"'
key tab
drive click-search
check "pointer: clicking the search field focuses it (same as Ctrl+K)" \
    wait_for '.overview.region == "Search"'

# --- search -------------------------------------------------------------------------
key tab; key tab   # back on the windows: typing anywhere searches
drive type tiled-a
check "search: typing from the windows starts a search" \
    wait_for '.overview.region == "Search" and .overview.search == "tiled-a" and .overview.searching'
check "search: finds tiled-a on another workspace" \
    wait_for "[.overview.cards[].address] == [\"$a\"] and .overview.cards[0].workspace == \"91\""
check "search: the real workspace did not change" test "$(active_workspace)" = "$original_ws"
check "search: results are captured for previews" wait_for '[.overview.cards[].capturing] | all'
key left
drive type Z
check "search: arrows in search move the caret, not the selection" \
    wait_for '.overview.search == "tiled-Za" and .overview.region == "Search"'
key backspace
key end
check "search: Backspace and End edit the text" \
    wait_for "(.overview.search == \"tiled-a\") and ([.overview.cards[].address] == [\"$a\"])"
key escape
check "search: Escape clears the search first" \
    wait_for '.visible and .overview.search == "" and (.overview.searching | not)'
check "search: clearing restores the active workspace's windows" \
    wait_for ".overview.previewed == \"$original_ws\""

drive type orbitfx
check "search: matches workspace names" \
    wait_for '[.overview.rail[].label] == ["orbitfx"] and (.overview.cards | length >= 1)'
key escape
drive type zzqqxxjj
check "search: no matches is announced" \
    wait_for '(.overview.cards | length == 0) and (.overview.rail | length == 0) and .overview.announcement == "No matches"'
key enter
sleep 0.2
check "search: Enter with no matches does nothing" wait_for '.visible'
key escape

# Input method composition: Enter belongs to the IME until it commits.
drive compose tiled
check "ime: composing text is marked" wait_for '.overview.composing and .overview.search == "tiled"'
key enter
sleep 0.2
check "ime: Enter while composing does not activate" \
    test "$(field '.visible')" = true -a "$(active_window)" != "$a" -a "$(active_window)" != "$b"
drive commit tiled-b
check "ime: the commit replaces the composition" \
    wait_for "(.overview.composing | not) and .overview.search == \"tiled-b\" and ([.overview.cards[].address] == [\"$b\"])"

# --- activation: keyboard and pointer paths ---------------------------------------------
guard "$b"
must "keys: the search result is the selected fixture" selected_is "$b"
key enter
check "keys: Enter on a remote result closes the overview" closed
check "keys: tiled-b has focus" focused "$b"
check "keys: its workspace 91 is active" test "$(active_workspace)" = 91

must "pointer: reopen" open
drive type tiled-a
must "pointer: tiled-a found" wait_for "[.overview.cards[].address] == [\"$a\"]"
guard "$a"
drive click "$a"
check "pointer: clicking the result closes the overview" closed
check "pointer: tiled-a has focus" focused "$a"
check "pointer: the focus and raise went out within 20 ms of the click, and the window was confirmed active" \
    wait_for '.last_activation.focus_sent_ms <= 20 and .last_activation.active_ms != null'
if $fake; then
    # Focus does not raise in the fake (nor, with every window floating,
    # reliably in Hyprland): Orbit must send alter_zorder after the focus.
    check "pointer: the activated window is raised to the top of the stack" \
        bash -c 'for _ in $(seq 20); do test "$(python3 "$0/scripts/fake-hyprland.py" ctl -j fake/zorder | jq -r ".[0]")" = "$1" && exit 0; sleep 0.1; done; exit 1' "$here" "$a"
    check "pointer: the raise follows the focus in the dispatch sequence" \
        bash -c 'python3 "$0/scripts/fake-hyprland.py" ctl -j fake/dispatches | jq -e --arg a "$1" "[.[] | select(test(\"focus.*address:\" + \$a) or test(\"alter_zorder.*address:\" + \$a))] | length >= 2 and (.[-1] | test(\"alter_zorder\"))" >/dev/null' "$here" "$a"
fi

# --- moving: filtered menu by keys, Shift-drop by pointer -----------------------------
must "move: reopen" open
drive type tiled-a
must "move: tiled-a selected" selected_is "$a"
key ctrl-shift-m
must "move: Ctrl+Shift+M opens the menu for tiled-a" wait_for ".overview.moving == \"$a\""
drive type 93
check "move: typing filters destinations, not the search" \
    wait_for '.overview.destination_filter == "93" and .overview.search == "tiled-a" and [.overview.rail[].label] == ["93"]'
must "move: the filter focuses 93" wait_for '.overview.rail_focus == "93"'
key enter
check "move: tiled-a confirmed on 93" wait_for '.overview.pending == 0 and .overview.announcement == "Moved to 93"'
check "move: Hyprland shows tiled-a on 93" test "$(workspace_of "$a")" = 93
check "move: the overview stays open (silent move)" wait_for '.visible'

key escape
must "pointer: preview 93, where tiled-a now is" preview 93
guard "$a"
drive drag "$a" 92 follow
# The ghost shrinks toward its in-tile size as the pointer reaches the rail
# (ORB-012). The drive path dispatches the moves without a frame between
# them, so the record is from whatever frame the ghost last drew; when one
# exists it must fit inside a tile and be past the halfway point.
check "drag: the ghost recorded during the drag fits inside a rail tile (or no frame was drawn mid-drag)" \
    bash -c 'g=$("$0" drive layout | jq -c ".ghost"); [ "$g" = null ] || jq -e --argjson g "$g" "$g.width <= 176 and $g.height <= 112 and $g.progress > 0.5" <<<"$g" >/dev/null' "$orbit"
check "pointer: Shift-drop moves and follows, then closes" closed
check "pointer: Hyprland shows tiled-a on 92" test "$(workspace_of "$a")" = 92
check "pointer: tiled-a has focus after the follow" focused "$a"

# --- Escape order -----------------------------------------------------------------------------
must "escape: reopen" open
drive type tiled
must "escape: a fixture is selected" wait_for "(.overview.selected == \"$a\" or .overview.selected == \"$b\")"
key ctrl-shift-m
must "escape: menu open" wait_for '.overview.moving != null'
key escape
check "escape: first closes the move menu" wait_for '.overview.moving == null and .overview.search == "tiled"'
key escape
check "escape: then clears the search" wait_for '.visible and .overview.search == ""'
key escape
check "escape: then dismisses" closed

# --- accessibility tree -------------------------------------------------------------------------
must "a11y: reopen" open
sleep 0.3
tree=$("$orbit" drive a11y)
if jq -e '.tree' <<< "$tree" >/dev/null 2>&1; then
    echo "$tree" > "$out/a11y-summary.json"
    check "a11y: no unlabelled buttons, inputs, images, lists or status" \
        jq -e '.unlabelled == 0' <<< "$tree"
    check "a11y: search, buttons, lists and a status region are exposed" \
        jq -e '.roles.SearchInput >= 1 and .roles.Button >= 2 and .roles.List >= 2 and .roles.Status >= 1' <<< "$tree"
    check "a11y: the keyboard selection is the reported focus" \
        jq -e '.active_descendant == "Button"' <<< "$tree"
else
    report SKIP "a11y: no assistive technology connected, so no tree to check (see native review)"
fi
key escape
closed >/dev/null

# --- composition and previews (ORB-008 design targets) ----------------------------------------
must "layout: reopen" open
sleep 0.5
layout=$("$orbit" drive layout)
mkdir -p "$here/docs/evidence/orb-008/$host"
echo "$layout" | jq . > "$here/docs/evidence/orb-008/$host/layout.json" 2>/dev/null
lay() { jq -e "$1" <<< "$layout" >/dev/null; }
check "layout: the overview reports its layout" lay '.laid_out'
check "layout: the surface is full screen with no panel element" lay '.full_screen and .panel == null'
check "layout: no desktop backdrop in test mode (solid glass)" lay '.backdrop == false'
check "layout: hints are hidden until asked for" lay '.hints_shown == false'
check "layout: three columns with three or more windows, two below" \
    lay 'if .cards >= 3 then .columns == 3 else .columns == 2 end'
check "layout: cards fill the width (three span at least 85% of the inner width; two are at the 760 cap) and are multiples of 4" \
    lay '(.card % 4 == 0) and (if .columns == 3 then (3 * .card + 56 >= 0.85 * .grid.width) else .card >= 600 end)'
check "layout: the search bar is centered at 56 tall" \
    lay '.search_rect.height == 56 and ((.search_rect.x - (.viewport[0] - .search_rect.x - .search_rect.width)) | fabs) <= 1'
check "layout: rail tiles are at least 160 wide at full size, in a centered strip" \
    lay '.tile[0] >= 160 and ((.rail.x - (.viewport[0] - .rail.x - .rail.width)) | fabs) <= 1'
check "layout: the grid rows fit the grid area" lay '.content_height <= .grid.height'
check "layout: the search bar lands on whole device pixels" \
    lay '[.search.device[0], .search.device[1]] | all(. - round | fabs < 0.02)'
drive key ctrl-/
check "hints: Ctrl+/ shows the key hints" wait_for '.visible'
layout=$("$orbit" drive layout)
check "hints: the hints bar is reported" lay '.hints_shown == true'
drive key ctrl-/
must "previews: preview workspace 91 (fixtures)" preview 91
# tiled-a has moved on by now, so 91 holds tiled-b alone.
check "previews: every fixture card on 91 has a captured frame" \
    wait_for '(.overview.cards | length) >= 1 and ([.overview.cards[].preview] | all)'
check "privacy: still only fixture windows are captured while previewing 91" \
    wait_for '[.capturing[].class] | all(startswith("dev.orbit.fixture.") or . == "OrbitFixtureX11")'
sleep 0.4
layout=$("$orbit" drive layout)
echo "$layout" | jq . > "$here/docs/evidence/orb-008/$host/layout-ws91.json" 2>/dev/null
check "previews: each card draws its image at the captured device size (one resample)" \
    lay '(.previews | length) >= 1 and ([.previews[] | (.shown[0] - .image[0] | fabs) < 0.5 and (.shown[1] - .image[1] | fabs) < 0.5] | all)'
check "previews: images fit the capture box without upscaling" \
    lay '(.card * .scale | round) as $w | (.slot * .scale | round) as $h | [.previews[] | .image[0] <= $w and .image[1] <= $h] | all'
check "previews: images keep the window's real aspect (within 2%)" \
    lay '[.previews[] | select(.window_aspect != null) | ((.image[0] / .image[1]) / .window_aspect - 1 | fabs) < 0.02] | all'
check "previews: image origins land on whole device pixels" \
    lay '[.previews[] | (.origin[0] - (.origin[0] | round) | fabs) < 0.05 and (.origin[1] - (.origin[1] | round) | fabs) < 0.05] | all'
check "icons: fixture cards fall back to the letter tile (no desktop entry)" \
    wait_for '[.overview.cards[] | select(.class | startswith("dev.orbit.fixture.")) | .icon == false] | all'
key escape
closed >/dev/null

# --- settings, shortcut and hot corner (ORB-010) -----------------------------------------------
check "settings: the scratch settings file was created with every key" \
    test -s "$ORBIT_CONFIG" -a "$("$orbit" config get appearance.blur)" = 7
check "settings: the daemon reports its settings and no load error" \
    wait_for '.settings.path != null and .settings.error == null'
check "shortcut: the chosen shortcut has no conflict and is bound live (or names its conflict)" \
    wait_for '(.settings.bind_ok and (.settings.conflicts | length == 0)) or ((.settings.bind_ok | not) and (.settings.conflicts | length > 0))'
if ! $fake; then
    check "shortcut: Hyprland lists Orbit's bind exactly when the daemon says it is bound" \
        test "$(hyprctl -j binds | jq '[.[] | select(.description == "Orbit overview")] | length')" = "$(field '.settings.bind_ok | if . then 1 else 0 end')"
fi
check "shortcut: the generated snippet exists and binds orbit toggle" \
    bash -c 'f=$("$0" state | jq -r .settings.snippet); test -s "$f" && grep -q "toggle" "$f" && grep -q "hl.bind" "$f"' "$orbit"
check "config: set and get round-trip through the file" \
    test "$("$orbit" config set hot_corner.dwell_ms 250)" = 250 -a "$("$orbit" config get hot_corner.dwell_ms)" = 250
check "config: bad values are refused" bash -c '! "$0" config set appearance.blur 99 2>/dev/null' "$orbit"
# --- updates (ORB-011): the check, the state, the pill -------------------------------------------
check "updates: orbit state has the update section with this build's version" \
    wait_for '.update.current == "'"$("$orbit" --version | sed 's/^orbit //')"'" and .update.available == false'
check "updates: orbit update check against the fixture finds 99.0.0" \
    bash -c '"$0" update check 2>&1 | grep -q "99.0.0 is available"' "$orbit"
check "updates: the daemon re-reads the state and offers it" \
    wait_for '.update.available == true and .update.latest == "99.0.0"'
check "updates: notes come from the fixture" bash -c '"$0" update notes | grep -q "fixture release"' "$orbit"
must "updates: reopen for the pill" open
check "updates: the overview shows the update pill (a11y button named for the version)" \
    bash -c '"$0" drive a11y | grep -q "99.0.0 available"' "$orbit"
key escape
closed >/dev/null
check "updates: skip stops the offer" \
    bash -c '"$0" update skip >/dev/null && for _ in $(seq 20); do "$0" state | jq -e ".update.available == false" >/dev/null && exit 0; sleep 0.1; done; exit 1' "$orbit"

"$orbit" config set hot_corner.corner top-left >/dev/null
check "hot corner: the daemon picks the change up and puts a corner surface in place" \
    wait_for '.settings.hot_corner == "TopLeft" and .settings.corners >= 1'
if ! $fake; then
    check "hot corner: the surface sits in the output's corner, not below the bar" \
        bash -c 'hyprctl -j layers | jq -e "[.[] | .levels | to_entries[] | .value[] | select(.namespace == \"orbit-corner\")] | length >= 1 and all(.[]; .x == 0 and .y == 0)" >/dev/null'
fi
drive hot-corner
check "hot corner: resting the pointer there opens the overview" wait_for '.visible and .overview != null'
check "hot corner: the overview is sized from Hyprland's logical monitor size, not GPUI's display" \
    wait_for '.visible and .surface.size_from == "hyprland"'
if ! $fake; then
    check "hot corner: the surface is exactly one monitor's logical size (width / scale)" \
        bash -c 'want=$("$0" state | jq -c ".surface.size"); hyprctl -j monitors | jq -e --argjson want "$want" "any(.[]; [(.width / .scale | round), (.height / .scale | round)] == \$want)" >/dev/null' "$orbit"
fi
key escape
closed >/dev/null
"$orbit" config set hot_corner.corner off >/dev/null
check "hot corner: off removes the surface" wait_for '.settings.corners == 0'
"$orbit" config set layout.columns four >/dev/null
"$orbit" config set layout.rail bottom >/dev/null
sleep 1.2
must "layout settings: reopen" open
sleep 0.4
layout=$("$orbit" drive layout)
check "layout settings: four columns and the rail at the bottom apply live" \
    lay '.columns == 4 and .rail.y > .grid.y and .mark.size >= 40 and .mark.wordmark'
key escape
closed >/dev/null
"$orbit" config set layout.columns auto >/dev/null
"$orbit" config set layout.rail top >/dev/null
drive settings
check "settings window: opens on request (orbit settings / the mark / Ctrl+,)" wait_for '.settings_window'
sleep 0.3

# --- presentation options ---------------------------------------------------------------------------
stop_daemon
start_daemon --opaque
must "look: daemon restarts with --opaque" wait_for '.sync == "Live"'
check "look: the opaque fallback keeps the brand theme" \
    wait_for '.look.theme == "standard" and .look.opaque'
stop_daemon
start_daemon --high-contrast --reduced-motion
must "look: daemon restarts with --high-contrast --reduced-motion" wait_for '.sync == "Live"'
check "look: high-contrast theme (opaque) and reduced motion are in effect" \
    wait_for '.look.theme == "high-contrast" and .look.opaque and .look.reduced_motion'
stop_daemon
start_daemon --no-glass
must "look: daemon restarts with --no-glass" wait_for '.sync == "Live"'
check "look: the denser standard layers can be chosen" \
    wait_for '.look.theme == "standard" and (.look.glass | not) and .look.opaque == false'

finish
