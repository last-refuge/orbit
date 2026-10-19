#!/usr/bin/env bash
# ORB-005: records Hyprland's event stream while Orbit's fixture windows open,
# change workspace, move and close, so the parser and state store can replay
# real events. Titles are redacted by `orbit events`; only fixture windows
# are touched. Window classes of anything you open meanwhile are recorded.
#
#   scripts/record-events.sh [path/to/orbit]
#
# Set ORBIT_MOVE_LUA to a Lua dispatch template with ADDR and WS placeholders
# to include a silent move, for example:
#   ORBIT_MOVE_LUA="hl.dsp.window.move({ window = 'address:ADDR', workspace = 'WS', follow = false })"
# Writes crates/orbit-hyprland/tests/fixtures/events/<host>-hyprland-<version>.txt
set -uo pipefail

orbit=$(realpath "${1:-target/release/orbit}")
[[ -x $orbit ]] || { echo "build first: cargo build --release" >&2; exit 1; }
here=$(cd "$(dirname "$0")" && pwd)
version=$(hyprctl -j version | jq -r '.tag // .version' | tr -c 'A-Za-z0-9.\n' '-')
out="crates/orbit-hyprland/tests/fixtures/events/$(hostname -s)-hyprland-$version.txt"

dispatch() {
    local reply
    reply=$(hyprctl dispatch "$@" 2>&1) || true
    [[ $reply == ok ]] || { echo "hyprctl dispatch $* -> $reply" >&2; return 1; }
}
focus_workspace() {
    dispatch "hl.dsp.focus({ workspace = '$1' })" 2>/dev/null || dispatch workspace "$1"
}

original=$(hyprctl -j activeworkspace | jq -r '.id')
trap 'focus_workspace "$original" >/dev/null 2>&1 || true' EXIT

"$orbit" events --duration-ms 20000 --record "$out" > /tmp/orbit-events-$$.log 2>&1 &
recorder=$!
sleep 0.5

"$here/fixtures.sh" spawn "$orbit" >/dev/null
sleep 1
focus_workspace 91; sleep 0.5
focus_workspace 93; sleep 0.5
focus_workspace "$original"; sleep 0.5

if [[ -n ${ORBIT_MOVE_LUA:-} ]]; then
    address=$(hyprctl -j clients | jq -r '.[] | select(.class == "dev.orbit.fixture.floating") | .address')
    if [[ -n $address ]]; then
        command=${ORBIT_MOVE_LUA//ADDR/$address}
        dispatch "${command//WS/91}" && echo "moved floating fixture to 91"
        sleep 0.5
    fi
fi

"$here/fixtures.sh" clean
wait "$recorder"
status=$?
tail -n 1 /tmp/orbit-events-$$.log
grep MALFORMED /tmp/orbit-events-$$.log || true
rm -f /tmp/orbit-events-$$.log
echo "recorded $(wc -l < "$out") lines to $out"
exit $status
