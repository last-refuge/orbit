#!/usr/bin/env bash
# ORB-006 vertical slice: open -> inspect -> move -> select -> close, through
# the overview's real input path (GPUI key, click and drag dispatch via
# `orbit drive`), checked against Hyprland's own state.
#
#   scripts/slice-check.sh [path/to/orbit]          on Hyprland, with fixtures
#   scripts/slice-check.sh --fake [path/to/orbit]   against fake-hyprland.py
#
# Starts its own `orbit daemon --test-hooks`. Only windows whose class starts
# with dev.orbit.fixture. are ever moved, focused or clicked; the script
# refuses any other. It never takes screenshots and prints no titles
# (`orbit state` has none). Results go to docs/evidence/orb-006/<host>/.
# Needs jq; on Hyprland also hyprctl.
set -uo pipefail

fake=false
[[ ${1:-} == --fake ]] && { fake=true; shift; }
orbit=$(realpath "${1:-target/release/orbit}")
[[ -x $orbit ]] || { echo "build first: cargo build --release" >&2; exit 1; }
here=$(cd "$(dirname "$0")/.." && pwd)
host=$(hostname -s)
$fake && host=$host-fake
out=$here/docs/evidence/orb-006/$host
mkdir -p "$out"
results=$out/results.txt
: > "$results"

if $fake; then
    hypr() { python3 "$here/scripts/fake-hyprland.py" ctl "$@"; }
else
    hypr() { hyprctl "$@"; }
fi

failures=0
report() { # report PASS|FAIL message
    printf '%s  %s\n' "$1" "$2" | tee -a "$results"
    [[ $1 == FAIL ]] && failures=$((failures + 1))
    return 0
}
check() { # check "message" command...
    local message=$1; shift
    if "$@"; then report PASS "$message"; else report FAIL "$message"; fi
}
must() { # like check, but stops the run on failure: later steps would act
    # on whatever happens to be selected, which may not be a fixture.
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
# Keyboard focus moves until the state matches, at most 12 presses.
press_until() { # press_until KEY JQ_EXPR
    for _ in $(seq 12); do
        state | jq -e "$2" >/dev/null && return 0
        drive key "$1" || return 1
    done
    state | jq -e "$2" >/dev/null
}
# Previews workspace NAME from the rail without switching to it.
preview() {
    # Shift+Tab goes from the windows back to the workspaces (Tab order is
    # search, workspaces, windows).
    [[ $(field .overview.region) == Rail ]] || drive key shift-tab
    press_until left '.overview.rail_focus == "1" or .overview.rail_focus == (.overview.rail[0].label)' >/dev/null
    press_until right ".overview.previewed == \"$1\"" &&
        drive key tab && [[ $(field .overview.region) == Grid ]]
}
select_card() { press_until right ".overview.selected == \"$1\""; }
guard() { # guard ADDRESS: refuse anything that is not a fixture
    is_fixture "$1" || { report FAIL "refusing to touch non-fixture window $1"; finish; }
}
focused() { # focused ADDRESS: up to 2 s for Hyprland to report it active
    for _ in $(seq 40); do [[ $(active_window) == "$1" ]] && return 0; sleep 0.05; done
    return 1
}
selected_if_shown() { # the invoking window is selected when it is on the grid
    local s; s=$(state)
    jq -e --arg w "$1" '.overview.selected == $w or (.overview.cards | any(.address == $w) | not)' <<< "$s" >/dev/null
}

daemon_pid=
original_ws=
finish() {
    "$orbit" hide >/dev/null 2>&1
    "$orbit" quit >/dev/null 2>&1
    [[ -n $daemon_pid ]] && wait "$daemon_pid" 2>/dev/null
    if ! $fake; then "$here/scripts/fixtures.sh" clean >/dev/null 2>&1; fi
    [[ -n $original_ws ]] && focus_workspace "$original_ws"
    echo "$failures failure(s); results in $results"
    exit $((failures > 0))
}
trap finish INT TERM

# --- setup ------------------------------------------------------------------
original_ws=$(active_workspace)
original_window=$(active_window)
if ! $fake; then
    "$here/scripts/fixtures.sh" spawn "$orbit" >/dev/null || { report FAIL "fixtures spawn"; finish; }
fi
a=$(fixture tiled-a); b=$(fixture tiled-b); fl=$(fixture floating); fs=$(fixture fullscreen)
check "fixtures present" test -n "$a" -a -n "$b" -a -n "$fl" -a -n "$fs"
[[ -n $a && -n $b && -n $fl ]] || finish
for w in "$a" "$b" "$fl"; do guard "$w"; done

RUST_LOG=${RUST_LOG:-warn,orbit=debug} "$orbit" daemon --test-hooks > "$out/daemon.log" 2>&1 &
daemon_pid=$!
for _ in $(seq 60); do "$orbit" status >/dev/null 2>&1 && break; sleep 0.05; done
check "daemon connects to Hyprland while hidden" wait_for '.sync == "Live" and .visible == false'

# --- open and inspect ---------------------------------------------------------
"$orbit" show >/dev/null
check "open: overview visible on the active workspace" \
    wait_for ".visible and .overview.previewed == \"$original_ws\""
check "open: focused window selected (if it is on this workspace)" selected_if_shown "$original_window"
check "inspect: rail lists the fixture workspaces" \
    wait_for '[.overview.rail[].label] | (index("91") != null and index("92") != null and index("93") != null and index("orbitfx") != null)'
must "inspect: arrows preview 91 without switching" preview 91
check "inspect: 91 shows both tiled fixtures" \
    wait_for "[.overview.cards[].address] | (index(\"$a\") != null and index(\"$b\") != null)"
check "inspect: the real workspace did not change" test "$(active_workspace)" = "$original_ws"
# The capture worker is asked for exactly the previewed workspace's cards.
check "inspect: previews requested for the 91 cards" \
    wait_for '[.overview.cards[].capturing] | all'
if ! $fake; then
    check "inspect: previews captured for the fixture cards" \
        wait_for "[.overview.cards[] | select(.address == \"$a\" or .address == \"$b\") | .preview] | all"
fi

# --- move by drag ----------------------------------------------------------------
guard "$a"
drive drag "$a" 92
check "drag: tiled-a confirmed on 92" wait_for '.overview.pending == 0 and .overview.announcement == "Moved to 92"'
check "drag: Hyprland shows tiled-a on 92" test "$(workspace_of "$a")" = 92
check "drag: focus and workspace unchanged (silent move)" \
    test "$(active_workspace)" = "$original_ws" -a "$(active_window)" = "$original_window"
check "drag: overview stays open" wait_for '.visible'

# Dropping on the window's own workspace is a no-op.
before=$(field '.finished | length')
guard "$b"
drive drag "$b" 91
sleep 0.3
check "drag onto its own workspace does nothing" \
    test "$(field '.finished | length')" = "$before" -a "$(workspace_of "$b")" = 91

# --- move by keyboard ------------------------------------------------------------
must "keys: select tiled-b" select_card "$b"
drive key ctrl-shift-m
must "keys: Ctrl+Shift+M opens the move menu for tiled-b" wait_for ".overview.moving == \"$b\""
must "keys: choose 93" press_until right '.overview.rail_focus == "93"'
drive key enter
check "keys: tiled-b confirmed on 93" wait_for '.overview.pending == 0 and .overview.announcement == "Moved to 93"'
check "keys: Hyprland shows tiled-b on 93" test "$(workspace_of "$b")" = 93
check "keys: focus unchanged" test "$(active_window)" = "$original_window"

# --- move and follow (gains focus when requested) ------------------------------
must "follow: preview 92" preview 92
guard "$fl"
must "follow: select the floating fixture" select_card "$fl"
drive key ctrl-shift-m
must "follow: move menu open for the floating fixture" wait_for ".overview.moving == \"$fl\""
# 91 emptied above, so Hyprland destroyed it; follow to 93.
must "follow: choose 93" press_until right '.overview.rail_focus == "93"'
drive key shift-enter
check "follow: floating confirmed on 93, then the overview closes" wait_for '.visible == false'
check "follow: Hyprland shows the floating fixture on 93" test "$(workspace_of "$fl")" = 93
check "follow: it has focus" focused "$fl"
check "follow: its workspace is active" test "$(active_workspace)" = 93

# --- select (activate by click) ---------------------------------------------------
"$orbit" show >/dev/null
must "select: reopens on the now active 93" wait_for '.visible and .overview.previewed == "93"'
guard "$b"
drive click "$b"
check "select: click closes the overview" wait_for '.visible == false'
check "select: tiled-b has focus" focused "$b"
check "select: its workspace 93 is active" test "$(active_workspace)" = 93

# --- close (cancel) ---------------------------------------------------------------
"$orbit" show >/dev/null
wait_for '.visible' >/dev/null
drive key escape
check "close: Escape dismisses" wait_for '.visible == false'
check "close: focus stays on tiled-b" test "$(active_window)" = "$b"
check "close: no move left pending" test "$(field '.overview')" = null

finish
