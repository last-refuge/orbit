#!/usr/bin/env bash
# ORB-003 overlay lifecycle checks against the running Hyprland session.
#
#   scripts/overlay-lifecycle.sh [--stacking] [path/to/orbit]
#
# Starts its own `orbit daemon`, then checks: no surface while hidden, one
# overlay-layer surface on the focused monitor while shown, focus returned on
# hide, warm-open timings, toggle bursts, every output, crash cleanup, stale
# socket recovery and quit. --stacking also opens the fixture windows and
# briefly switches to workspace 93 to show the overlay above a fullscreen
# window, then switches back. Results and screenshots go to
# docs/evidence/orb-003/<host>/. Needs hyprctl, jq and grim.
set -uo pipefail

stacking=false
[[ ${1:-} == --stacking ]] && { stacking=true; shift; }
orbit=$(realpath "${1:-target/release/orbit}")
[[ -x $orbit ]] || { echo "build first: cargo build --release" >&2; exit 1; }
here=$(cd "$(dirname "$0")/.." && pwd)
out=$here/docs/evidence/orb-003/$(hostname)
mkdir -p "$out"
results=$out/results.txt
: > "$results"

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

orbit_layers() { # prints "monitor level width height" per Orbit layer surface
    hyprctl -j layers | jq -r 'to_entries[] | .key as $m | .value.levels | to_entries[]
        | .key as $l | .value[] | select(.namespace == "orbit") | "\($m) \($l) \(.w) \(.h)"'
}
layer_count() { orbit_layers | grep -c . ; }
active_window() { hyprctl -j activewindow | jq -r '.address // "none"'; }
active_workspace() { hyprctl -j activeworkspace | jq -r '.id'; }
focused_monitor() { hyprctl -j monitors | jq -r '.[] | select(.focused) | .name'; }
wait_layers() { # wait_layers N: wait up to 2 s for exactly N Orbit surfaces
    for _ in $(seq 40); do [[ $(layer_count) == "$1" ]] && return 0; sleep 0.05; done
    return 1
}
ms_of() { sed -n 's/.*last_open_ms=\([0-9.]*\).*/\1/p' <<< "$1"; }

daemon_pid=
start_daemon() {
    # Capped: a failing renderer can log millions of protocol errors.
    "$orbit" daemon > >(head -c 20000000 >> "$out/daemon.log") 2>&1 &
    daemon_pid=$!
    for _ in $(seq 50); do "$orbit" status >/dev/null 2>&1 && return 0; sleep 0.1; done
    return 1
}
cleanup() {
    "$orbit" quit >/dev/null 2>&1
    [[ -n $daemon_pid ]] && kill "$daemon_pid" 2>/dev/null
    $stacking && "$here/scripts/fixtures.sh" clean >/dev/null 2>&1
}
trap cleanup EXIT

{
    echo "orbit: $("$orbit" --version)"
    echo "hyprland: $(hyprctl -j version | jq -r .version)"
    echo "monitors: $(hyprctl -j monitors | jq -r '[.[] | "\(.name) \(.width)x\(.height)@\(.scale)"] | join(", ")')"
    echo "date: $(date -Iseconds)"
} | tee -a "$results"

before_window=$(active_window)
before_workspace=$(active_workspace)
monitor=$(focused_monitor)

check "daemon starts" start_daemon
check "no Orbit surface while hidden" wait_layers 0

"$orbit" show >/dev/null 2>"$out/show-error.txt"
check "show opens exactly one surface" wait_layers 1
if [[ $(layer_count) != 1 ]]; then
    echo "first show failed: $(cat "$out/show-error.txt")" | tee -a "$results"
    echo "stopping early; see $out/daemon.log" | tee -a "$results"
    exit 1
fi
read -r m level w h <<< "$(orbit_layers | head -n1)"
check "surface is on the focused monitor ($monitor)" test "$m" = "$monitor"
check "surface is on the overlay layer (level 3, got $level)" test "$level" = 3
mon_size=$(hyprctl -j monitors | jq -r --arg m "$monitor" '.[] | select(.name == $m) | "\((.width / .scale) | floor) \((.height / .scale) | floor)"')
read -r mw mh <<< "$mon_size"
check "surface covers the output (${w}x${h} of ${mw}x${mh} logical)" \
    test $((w - mw)) -ge -1 -a $((w - mw)) -le 1 -a $((h - mh)) -ge -1 -a $((h - mh)) -le 1
# Screenshots only while the overlay is confirmed up, so they never capture
# the user's own windows.
[[ $(layer_count) == 1 ]] && grim -o "$monitor" "$out/overlay-$monitor.png" 2>/dev/null
"$orbit" hide >/dev/null
check "hide removes the surface" wait_layers 0
sleep 0.2
check "focus returns to the same window after hide" test "$(active_window)" = "$before_window"
check "workspace unchanged after hide" test "$(active_workspace)" = "$before_workspace"

timings=()
for _ in $(seq 20); do
    "$orbit" show >/dev/null
    wait_layers 1
    sleep 0.15
    ms=$(ms_of "$("$orbit" status)")
    [[ -n $ms ]] && timings+=("$ms")
    "$orbit" hide >/dev/null
    wait_layers 0
done
sorted=$(printf '%s\n' "${timings[@]}" | sort -n)
n=${#timings[@]}
p50=$(sed -n "$(( (n + 1) / 2 ))p" <<< "$sorted"); p95=$(sed -n "$(( (n * 95 + 99) / 100 ))p" <<< "$sorted")
echo "warm open to first frame built (ms), n=$n of 20: p50=$p50 p95=$p95 max=$(tail -n1 <<< "$sorted")" | tee -a "$results"
check "every warm open rendered a frame ($n of 20)" test "$n" = 20
check "warm open p95 <= 150 ms (plan budget; first frame built, not displayed)" \
    awk -v p="$p95" 'BEGIN { exit !(p <= 150) }'

burst() { # burst N: N concurrent toggles; prints how many failed
    local pids=() fails=0
    for _ in $(seq "$1"); do "$orbit" toggle >/dev/null 2>&1 & pids+=($!); done
    for pid in "${pids[@]}"; do wait "$pid" || fails=$((fails + 1)); done
    echo "$fails"
}
fails=$(burst 51)
check "51 concurrent toggles all answered (failures: $fails)" test "$fails" = 0
check "odd burst settles visible with exactly one surface" wait_layers 1
sleep 0.3
check "  ...and still exactly one after settling" test "$(layer_count)" = 1
fails=$(burst 50)
sleep 0.3
check "even burst (101 total) stays visible with one surface (failures: $fails)" \
    bash -c "'$orbit' status | grep -q '^visible' && test \"\$(hyprctl -j layers | jq '[.. | objects | select(.namespace? == \"orbit\")] | length')\" = 1"
"$orbit" hide >/dev/null
check "hidden after bursts" wait_layers 0
sleep 0.2
check "focus unchanged after bursts" test "$(active_window)" = "$before_window"

for name in $(hyprctl -j monitors | jq -r '.[].name'); do
    "$orbit" show --output "$name" >/dev/null
    wait_layers 1
    check "show --output $name lands on $name" test "$(orbit_layers | awk '{print $1}')" = "$name"
    "$orbit" hide >/dev/null
    wait_layers 0
done
check "unknown output is rejected" bash -c "! '$orbit' show --output ORBIT-NO-SUCH-OUTPUT 2>/dev/null"
check "rejected output opens nothing" wait_layers 0

if $stacking; then
    "$here/scripts/fixtures.sh" spawn "$orbit" > "$out/fixtures.txt" 2>&1
    fs=$(hyprctl -j clients | jq -r '.[] | select(.class == "dev.orbit.fixture.fullscreen") | "\(.workspace.id) \(.fullscreen)"')
    read -r fs_ws fs_mode <<< "$fs"
    check "fullscreen fixture is fullscreen (mode $fs_mode on workspace $fs_ws)" test "${fs_mode:-0}" != 0
    hyprctl dispatch "hl.dsp.focus({ workspace = '93' })" >/dev/null
    sleep 0.4
    "$orbit" show >/dev/null
    check "overlay opens over the fullscreen workspace" wait_layers 1
    sleep 0.3
    [[ $(layer_count) == 1 ]] && grim -o "$(focused_monitor)" "$out/stacking-fullscreen.png" 2>/dev/null
    "$orbit" hide >/dev/null
    wait_layers 0
    hyprctl dispatch "hl.dsp.focus({ workspace = '$before_workspace' })" >/dev/null
    sleep 0.3
    "$here/scripts/fixtures.sh" clean >/dev/null
    check "returned to workspace $before_workspace" test "$(active_workspace)" = "$before_workspace"
    echo "inspect $out/stacking-fullscreen.png: the Orbit overlay must cover the fullscreen fixture" | tee -a "$results"
fi

"$orbit" show >/dev/null
wait_layers 1
kill -9 "$daemon_pid"
wait "$daemon_pid" 2>/dev/null
check "killed daemon leaves no surface" wait_layers 0
sleep 0.2
check "focus returns after a crash" test "$(active_window)" = "$before_window"
check "daemon restarts over the stale socket" start_daemon
check "quit is acknowledged" bash -c "'$orbit' quit >/dev/null"
for _ in $(seq 30); do kill -0 "$daemon_pid" 2>/dev/null || break; sleep 0.1; done
check "daemon exits on quit" bash -c "! kill -0 $daemon_pid 2>/dev/null"
check "socket removed on quit" test ! -e "${XDG_RUNTIME_DIR}/orbit/orbit.sock"
daemon_pid=

echo "manual: run 'orbit daemon &', then 'orbit show' from a terminal, press Escape, and type at once; keys must reach the terminal" | tee -a "$results"
echo "failures: $failures" | tee -a "$results"
exit $((failures > 0))
