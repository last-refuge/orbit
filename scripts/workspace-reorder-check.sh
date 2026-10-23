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
export XDG_RUNTIME_DIR=$out/runtime XDG_CONFIG_HOME=$out/config XDG_STATE_HOME=$out/state XDG_DATA_HOME=$out/data XDG_CACHE_HOME=$out/cache
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
    RUST_LOG=warn "$orbit" daemon --test-hooks >> "$out/orbit.log" 2>&1 &
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
reordered='["1","93","91","92","orbitfx","New"]'
order_is "$initial"
python3 "$here/scripts/fake-hyprland.py" ctl -j clients > "$out/clients-before.json"
python3 "$here/scripts/fake-hyprland.py" ctl -j fake/dispatches > "$out/dispatches-before.json"
drive start-workspace-drag 93 91 before
sleep .15
"$orbit" drive layout | jq -e '.workspace_dragging and .workspace_drop != null' >/dev/null
shot dragging
drive drop-workspace
order_is "$reordered"
wait_for '.overview.rail_focus == "93" and .overview.previewed == "1"'
python3 "$here/scripts/fake-hyprland.py" ctl -j clients > "$out/clients-after.json"
python3 "$here/scripts/fake-hyprland.py" ctl -j fake/dispatches > "$out/dispatches-after.json"
cmp "$out/clients-before.json" "$out/clients-after.json"
cmp "$out/dispatches-before.json" "$out/dispatches-after.json"
printf 'PASS drag reorder, focus, unchanged compositor state\n'
shot reordered

drive key ctrl-shift-right
order_is '["1","91","93","92","orbitfx","New"]'
drive key ctrl-shift-left
order_is "$reordered"
printf 'PASS keyboard parity\n'

drive start-workspace-drag 93 92 after
drive key escape
drive drop-workspace
order_is "$reordered"
drive drag-workspace 93 outside
sleep .4
order_is "$reordered"
printf 'PASS Escape and outside-drop cancellation\n'

# Six tiles on a five-tile rail: holding the right edge exposes New.
drive start-workspace-drag 93 right-edge
sleep .8
"$orbit" drive layout | jq -e '.rail_drag_first > 0' >/dev/null
shot edge-scroll
drive drop-workspace
order_is '["1","91","92","orbitfx","93","New"]'
printf 'PASS edge scroll and move to end; New remains last\n'

stop
start
order_is '["1","91","92","orbitfx","93","New"]'
printf 'PASS persisted order across daemon restart\n'

# Invalid edits must be preserved, and a failed save must leave the rail alone.
cp "$ORBIT_CONFIG" "$out/valid.toml"
printf '[layout\ninvalid' > "$ORBIT_CONFIG"
drive drag-workspace 91 1 before
wait_for '.overview.announcement | startswith("Could not save workspace order:")'
order_is '["1","91","92","orbitfx","93","New"]'
test "$(cat "$ORBIT_CONFIG")" = $'[layout\ninvalid'
cp "$out/valid.toml" "$ORBIT_CONFIG"
printf 'PASS save failure leaves order and invalid file intact\n'

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
