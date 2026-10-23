#!/usr/bin/env bash
# Native GPUI input checks in a private headless sway, with fake Hyprland.
# Requires sway, jq and Python. SWAY_BIN may name an unpacked sway binary.
# Optional screenshots capture only this isolated fixture desktop (requires grim).
# Usage: bash scripts/workspace-reorder-check.sh [binary] [evidence-directory]
set -euo pipefail
here=$(cd "$(dirname "$0")/.." && pwd)
orbit=$(realpath "${1:-$here/target/release/orbit}")
out=$(realpath -m "${2:-$(mktemp -d /tmp/orbit-reorder.XXXXXX)}")
mkdir -p "$out"
# AF_UNIX paths are limited to 108 bytes on Linux. CI evidence directories
# can be deeply nested, so keep the fixture's sockets in a short private path.
fixture_runtime=$(mktemp -d /tmp/orbit-ws.XXXXXX)
export XDG_RUNTIME_DIR=$fixture_runtime XDG_CONFIG_HOME=$out/config XDG_STATE_HOME=$out/state XDG_DATA_HOME=$out/data XDG_CACHE_HOME=$out/cache
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME" "$XDG_STATE_HOME" "$XDG_DATA_HOME" "$XDG_CACHE_HOME"
chmod 700 "$XDG_RUNTIME_DIR"
export HYPRLAND_INSTANCE_SIGNATURE=orbit-reorder-fixture
export ORBIT_CONFIG=$out/config.toml ORBIT_UPDATE_START_DELAY_MS=3600000
export DBUS_SESSION_BUS_ADDRESS=unix:path=$out/no-bus
unset DISPLAY WAYLAND_DISPLAY
cat > "$out/sway.conf" <<'CONF'
output HEADLESS-1 resolution 1280x720
seat seat0 fallback true
CONF
cat > "$ORBIT_CONFIG" <<'CONF'
launch_at_login = false
[update]
notify = false
CONF
WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 "${SWAY_BIN:-sway}" -c "$out/sway.conf" > "$out/sway.log" 2>&1 &
sway_pid=$!
fake_pid= daemon_pid=
cleanup() {
    [[ -z $daemon_pid ]] || kill "$daemon_pid" 2>/dev/null || true
    [[ -z $fake_pid ]] || kill "$fake_pid" 2>/dev/null || true
    kill "$sway_pid" 2>/dev/null || true
    [[ -z $daemon_pid ]] || wait "$daemon_pid" 2>/dev/null || true
    [[ -z $fake_pid ]] || wait "$fake_pid" 2>/dev/null || true
    wait "$sway_pid" 2>/dev/null || true
    rm -rf "$fixture_runtime"
}
trap cleanup EXIT
for _ in $(seq 50); do
    socket=$(find "$XDG_RUNTIME_DIR" -maxdepth 1 -type s -name 'wayland-*' -printf '%f\n' | head -1)
    [[ -n $socket ]] && break
    sleep .1
done
[[ -n ${socket:-} ]] || { cat "$out/sway.log"; exit 1; }
export WAYLAND_DISPLAY=$socket
python3 "$here/scripts/fake-hyprland.py" serve --monitor HEADLESS-1 1280 720 > "$out/fake.log" 2>&1 &
fake_pid=$!
state() { "$orbit" state; }
drive() { "$orbit" drive "$@" >/dev/null; }
wait_for() {
    for _ in $(seq 60); do
        if state > "$out/state.json" 2>/dev/null && jq -e "$1" "$out/state.json" >/dev/null; then return; fi
        sleep .1
    done
    printf 'FAIL: %s\nEvidence: %s\n' "$1" "$out" >&2
    cat "$out/state.json" >&2
    exit 1
}
order_is() { wait_for "[.overview.rail[].label] == $1 and .visible"; }
start() {
    local motion_args=()
    [[ ${ORBIT_TEST_REDUCED_MOTION:-0} != 1 ]] || motion_args+=(--reduced-motion)
    RUST_LOG=warn "$orbit" daemon --test-hooks "${motion_args[@]}" >> "$out/orbit.log" 2>&1 &
    daemon_pid=$!
    wait_for '.sync == "Live"'
    "$orbit" show >/dev/null
    wait_for '.visible and .overview != null'
    sleep .4
}
stop() { "$orbit" quit >/dev/null; wait "$daemon_pid"; daemon_pid=; }
shot() { if command -v grim >/dev/null; then grim "$out/$1.png"; fi; }
start
initial='["1","91","92","93","orbitfx","New"]'
order_is "$initial"
python3 "$here/scripts/fake-hyprland.py" ctl -j clients > "$out/clients-before.json"
layout_wait() {
    for _ in $(seq 60); do
        "$orbit" drive layout > "$out/layout.json"
        jq -e "$1" "$out/layout.json" >/dev/null && return
        sleep .025
    done
    cat "$out/layout.json" >&2; exit 1
}
# Hold the pointer still after bumping: animated visuals must not change the target.
drive start-workspace-drag 93 91 before
layout_wait '.workspace_offsets[] | select(.destination == "Existing(WorkspaceId(91))") | .x > 5'
shot bump-outbound
layout_wait '[.workspace_offsets[] | select(.destination == "Existing(WorkspaceId(91))")][0].x as $x | ($x - 2*(.tile[0]+.tile_gap) | fabs) < 0.2'
"$orbit" drive layout | jq -e '.workspace_drop.destination == "Existing(WorkspaceId(91))"' >/dev/null
if [[ ${ORBIT_TEST_REDUCED_MOTION:-0} == 1 ]]; then
    jq -e 'all(.workspace_offsets[]; .y == 0)' "$out/layout.json" >/dev/null
fi
shot bump-settled
drive move-workspace-drag 93
layout_wait '.workspace_drop == null and all(.workspace_offsets[]; (.x | fabs) < 0.2 and (.y | fabs) < 0.2)'
shot bump-returned
# Reverse again before the first spring settles; then cancel the gesture.
drive move-workspace-drag 91
sleep .05
drive move-workspace-drag 92
sleep .05
drive move-workspace-drag outside
layout_wait '.workspace_drop == null and all(.workspace_offsets[]; (.x | fabs) < 0.2 and (.y | fabs) < 0.2)'
drive move-workspace-drag 91
sleep .05
drive move-workspace-drag 93
layout_wait '.workspace_drop == null and all(.workspace_offsets[]; (.x | fabs) < 0.2 and (.y | fabs) < 0.2)'
drive drop-workspace
python3 "$here/scripts/fake-hyprland.py" ctl -j clients > "$out/clients-preview.json"
cmp "$out/clients-before.json" "$out/clients-preview.json"
printf 'PASS bump preview, return, rail exit, rapid reversal, fixed drop targets; no compositor mutation before drop\n'

drive start-workspace-drag 93 91 before
sleep .15
"$orbit" drive layout | jq -e '.workspace_dragging and .workspace_drop != null' >/dev/null
shot dragging
drive drop-workspace
wait_for '.overview.announcement == "Workspaces swapped; window layouts preserved" and .overview.rail_focus == "91"'
order_is "$initial"
python3 "$here/scripts/fake-hyprland.py" ctl -j clients > "$out/clients-after.json"
python3 - "$out" <<'PYTEST'
import json,sys
from pathlib import Path
p=Path(sys.argv[1]); before=json.loads((p/'clients-before.json').read_text()); after=json.loads((p/'clients-after.json').read_text())
for a,b in zip(before,after):
    expected={91:93,93:91}.get(a['workspace']['id'],a['workspace']['id'])
    assert b['workspace']['id']==expected
    assert {k:v for k,v in a.items() if k!='workspace'}=={k:v for k,v in b.items() if k!='workspace'}
PYTEST
printf 'PASS drag swaps compositor contents, preserves geometry, rail follows workspace numbers\n'
shot swapped

drive drag-workspace 91 93
wait_for '.overview.rail_focus == "93"'
python3 "$here/scripts/fake-hyprland.py" ctl -j clients > "$out/clients-restored.json"
cmp "$out/clients-before.json" "$out/clients-restored.json"
printf 'PASS swap back restores every window\n'

drive key ctrl-shift-left
wait_for '.overview.rail_focus == "92"'
drive key ctrl-shift-right
wait_for '.overview.rail_focus == "93"'
python3 "$here/scripts/fake-hyprland.py" ctl -j clients > "$out/clients-keyboard.json"
cmp "$out/clients-before.json" "$out/clients-keyboard.json"
printf 'PASS keyboard parity and round trip\n'

drive start-workspace-drag 93 92 after
drive key escape
drive drop-workspace
drive drag-workspace 93 outside
sleep .4
python3 "$here/scripts/fake-hyprland.py" ctl -j clients > "$out/clients-cancelled.json"
cmp "$out/clients-before.json" "$out/clients-cancelled.json"
printf 'PASS Escape and outside-drop cancellation\n'

drive start-workspace-drag 93 right-edge
sleep .8
"$orbit" drive layout | jq -e '.rail_drag_first > 0' >/dev/null
shot edge-scroll
drive key escape
drive drop-workspace
printf 'PASS edge scroll; unsupported targets do not swap\n'

stop
start
order_is "$initial"
printf 'PASS restart reads compositor order\n'

# Original window drag path still moves only the fixture window.
drive key shift-tab,home,right,tab
wait_for '.overview.previewed == "91" and .overview.region == "Grid"'
drive drag 0x5a0001 92
for _ in $(seq 30); do
    moved=$(python3 "$here/scripts/fake-hyprland.py" ctl -j clients | jq -r '.[] | select(.address == "0x5a0001") | .workspace.name')
    [[ $moved == 92 ]] && break
    sleep .1
done
test "$moved" = 92
printf 'PASS window drag regression\n'
stop
printf 'All workspace reorder checks passed. Evidence: %s\n' "$out"
