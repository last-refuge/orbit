#!/usr/bin/env bash
# Takes the README screenshots of Orbit's own surfaces on a live Hyprland
# session, with Orbit's fixture windows as the only content.
#
#   scripts/readme-shots.sh [path/to/orbit]
#
# What it does: starts a daemon of its own (with a scratch HOME so the
# settings window shows ~/.config paths that are nobody's), opens the
# fixture windows on workspaces 91-93, switches to 91, opens the overview
# and captures it, a search, the settings window and (when ydotool's daemon
# is running) a drag in progress. Every capture goes through
# `crates/orbit-ui/examples/shot.rs`, which paints the bar and the
# workspace rail in midnight, since those show the person's real
# workspaces, and scales to 1600 px wide. Raw captures live in a temp dir
# that is deleted on exit; only the masked files land in docs/readme/.
# Then it quits its daemon, closes the fixtures and returns to the
# workspace it started on.
#
# Refuses to run while another Orbit daemon is up. Needs hyprctl, jq, grim.
set -euo pipefail

orbit=$(realpath "${1:-target/release/orbit}")
here=$(cd "$(dirname "$0")/.." && pwd)
out=$here/docs/readme
work=$(mktemp -d)
daemon_pid=
original_ws=$(hyprctl -j activeworkspace | jq -r .name)
monitor=$(hyprctl -j monitors | jq -r '.[] | select(.focused) | .name')
scale=$(hyprctl -j monitors | jq -r '.[] | select(.focused) | .scale')
width=1600

say() { printf '\033[1;35mshots\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror\033[0m %s\n' "$*" >&2; exit 1; }

cleanup() {
    "$orbit" quit >/dev/null 2>&1 || true
    [[ -n $daemon_pid ]] && wait "$daemon_pid" 2>/dev/null || true
    "$here/scripts/fixtures.sh" clean >/dev/null 2>&1 || true
    hyprctl dispatch "hl.dsp.focus({ workspace = '$original_ws' })" >/dev/null 2>&1 || true
    rm -rf "${work:?}"
}
trap cleanup EXIT

for tool in hyprctl jq grim; do command -v "$tool" >/dev/null || die "$tool is needed"; done
[[ -x $orbit ]] || die "build first: cargo build --release"
"$orbit" status >/dev/null 2>&1 && die "an Orbit daemon is already running; quit it first (orbit quit)"
mkdir -p "$out"

# The shot tool.
(cd "$here" && cargo build -q --release -p orbit-ui --example shot) || die "building examples/shot.rs failed"
shot_tool=$here/target/release/examples/shot

# Device-pixel rectangle from logical x,y,w,h.
dev() { awk -v s="$scale" -v x="$1" -v y="$2" -v w="$3" -v h="$4" 'BEGIN { printf "%d,%d,%d,%d", x*s, y*s, w*s+1, h*s+1 }'; }
state() { "$orbit" state; }
wait_for() { for _ in $(seq 80); do state | jq -e "$1" >/dev/null 2>&1 && return 0; sleep 0.05; done; return 1; }

# Masks: the bar along the top (Omarchy's, 26 logical px plus a margin) and
# the workspace rail (its tiles show the person's real workspaces).
masks() {
    local layout=$1 bar rail
    bar=$(dev 0 0 "$(echo "$layout" | jq -r '.viewport[0]')" 30)
    rail=$(echo "$layout" | jq -r '.rail | "\(.x - 8) \(.y - 8) \(.width + 16) \(.height + 16)"')
    # shellcheck disable=SC2086
    rail=$(dev $rail)
    echo "--mask $bar --mask $rail"
}

# A scratch HOME for the daemon: its settings and snippet go there, and the
# settings window shows ~/.config paths that belong to nobody.
demo_home=$work/home
mkdir -p "$demo_home/.config/orbit" "$demo_home/.config/hypr"
cat > "$demo_home/.config/orbit/config.toml" <<CFG
[behaviour]
search_scope = "workspace"

[hot_corner]
corner = "top-left"
CFG

say "opening the fixture windows"
"$here/scripts/fixtures.sh" spawn "$orbit" >/dev/null
sleep 1.5

say "starting a daemon (scratch HOME)"
HOME=$demo_home XDG_CONFIG_HOME= RUST_LOG=warn "$orbit" daemon --test-hooks >"$work/daemon.log" 2>&1 &
daemon_pid=$!
for _ in $(seq 100); do "$orbit" status >/dev/null 2>&1 && break; sleep 0.05; done
"$orbit" status >/dev/null 2>&1 || die "the daemon did not start (see $work/daemon.log)"

hyprctl dispatch "hl.dsp.focus({ workspace = '91' })" >/dev/null
sleep 0.8

capture() { # capture NAME: the focused output, masked, into docs/readme
    local name=${1:?} layout raw
    raw=${work:?}/${name}-raw.png
    layout=$("$orbit" drive layout)
    grim -o "$monitor" "$raw"
    # shellcheck disable=SC2046
    "$shot_tool" "$raw" "$out/$name.png" --width "$width" $(masks "$layout") >/dev/null
    rm -f "${raw:?}"
    say "wrote docs/readme/$name.png"
}

say "overview"
"$orbit" show >/dev/null
wait_for '.visible and .overview != null' || die "the overview did not open"
sleep 1.5
capture overview

say "search"
"$orbit" drive type tiled >/dev/null
wait_for '.overview.search == "tiled"' || die "search text did not land"
sleep 0.6
capture search
"$orbit" drive key escape >/dev/null   # clears the search, keeps the overview
sleep 0.3

if [[ -S ${YDOTOOL_SOCKET:-/run/user/$(id -u)/.ydotool_socket} || -S /tmp/.ydotool_socket ]]; then
    say "drag (ydotool)"
    layout=$("$orbit" drive layout)
    card=$(state | jq -r '.overview.cards[0].address')
    from=$(echo "$layout" | jq -r --arg a "$card" '.cards[$a] // empty | "\(.x + .width / 2) \(.y + .height / 2)"')
    to=$(echo "$layout" | jq -r '.rail | "\(.x + .width - 60) \(.y + .height / 2)"')
    if [[ -n $from && -n $to ]]; then
        raw=${work:?}/drag-raw.png
        # shellcheck disable=SC2086
        ydotool mousemove --absolute -x ${from% *} -y ${from#* }; sleep 0.2
        ydotool click 0x40; sleep 0.2                      # press
        # shellcheck disable=SC2086
        ydotool mousemove --absolute -x ${to% *} -y ${to#* }; sleep 0.5
        grim -o "$monitor" "$raw"
        # shellcheck disable=SC2086
        ydotool mousemove --absolute -x ${from% *} -y ${from#* }; sleep 0.2
        ydotool click 0x80                                 # release where it started: no move
        sleep 0.3
        # shellcheck disable=SC2046
        "$shot_tool" "$raw" "$out/drag.png" --width "$width" $(masks "$layout") >/dev/null
        rm -f "${raw:?}"
        say "wrote docs/readme/drag.png"
    else
        say "no card bounds in the layout; skipping the drag shot"
    fi
else
    say "ydotool's daemon is not running; skipping the drag shot"
fi

say "settings window"
"$orbit" settings >/dev/null
sleep 2
geo=$(hyprctl -j clients | jq -r '.[] | select(.class == "orbit-settings") | "\(.at[0]),\(.at[1]) \(.size[0])x\(.size[1])"' | head -n1)
[[ -n $geo ]] || die "no settings window"
raw=${work:?}/settings-raw.png
grim -g "$geo" "$raw"
"$shot_tool" "$raw" "$out/settings.png" --width 1100 >/dev/null
rm -f "${raw:?}"
say "wrote docs/readme/settings.png"

say "done; cleaning up"
