#!/usr/bin/env bash
# ORB-004 capture check on a live Hyprland session. Captures only Orbit's own
# windows (fixtures and the preview); nothing else is captured or recorded.
#
#   scripts/capture-check.sh [path/to/orbit]
#
# Scenarios, each recorded under docs/evidence/orb-004/<host>/<scenario>/:
#   inactive          you stay where you are; fixtures sit on hidden workspaces
#   visible-occluded  workspace 91 shown; the two tiled fixtures overlap
#   visible-fullscreen workspace 93 shown with its fullscreen fixture
#   preview           `orbit preview` on workspace 94, captured to prove the
#                     GPUI render of the previews
# Your original workspace is restored after each switch. Needs hyprctl and jq.
set -uo pipefail

orbit=$(realpath "${1:-target/release/orbit}")
[[ -x $orbit ]] || { echo "build first: cargo build --release" >&2; exit 1; }
here=$(cd "$(dirname "$0")" && pwd)
out="docs/evidence/orb-004/$(hostname -s)"
rm -rf "$out"
mkdir -p "$out"
results="$out/results.txt"
: > "$results"
failures=0

log() { echo "$*" | tee -a "$results"; }

dispatch() {
    local reply
    reply=$(hyprctl dispatch "$@" 2>&1) || true
    [[ $reply == ok ]]
}

focus_workspace() {
    dispatch "hl.dsp.focus({ workspace = '$1' })" || dispatch workspace "$1"
}

original=$(hyprctl -j activeworkspace | jq -r '.id')
restore() { focus_workspace "$original" >/dev/null || true; }
trap restore EXIT

spawned=0
if [[ -z $("$here/fixtures.sh" list) ]]; then
    "$here/fixtures.sh" spawn "$orbit" > "$out/fixtures.txt" 2>&1 || true
    spawned=1
    sleep 1
else
    "$here/fixtures.sh" list > "$out/fixtures.txt"
fi

scenario() {
    local label=$1
    log "== $label"
    if "$orbit" capture --record "$out" --label "$label" --repeat 20 >> "$results" 2>&1; then
        log "PASS $label"
    else
        log "FAIL $label"
        failures=$((failures + 1))
    fi
}

scenario inactive

focus_workspace 91 && sleep 0.6 && scenario visible-occluded
restore; sleep 0.3
focus_workspace 93 && sleep 0.6 && scenario visible-fullscreen
restore; sleep 0.3

# The preview window renders fixture captures through GPUI; capturing it
# shows what Orbit drew without screenshotting the desktop.
preview_cmd="$orbit preview --interval-ms 500 --exit-after-ms 30000 2>$PWD/$out/preview.log"
dispatch "hl.dsp.exec_cmd('$preview_cmd', { workspace = '94 silent', no_initial_focus = true })" ||
    dispatch exec "[workspace 94 silent] $preview_cmd"
preview_query() {
    hyprctl -j clients | jq -r ".[] | select(.class == \"dev.orbit.preview\") | $1" | head -n1
}
for _ in $(seq 50); do
    [[ -n $(preview_query .address) ]] && break
    sleep 0.1
done
if [[ -z $(preview_query .address) ]]; then
    log "FAIL preview: the preview window did not open (see preview.log)"
    failures=$((failures + 1))
else
    focus_workspace 94 && sleep 3
    preview_pid=$(preview_query .pid)
    scenario preview
    log "preview RSS: $(awk '/VmRSS/ {print $2, $3}' "/proc/$preview_pid/status" 2>/dev/null)"
    address=$(preview_query .address)
    dispatch "hl.dsp.window.close({ window = 'address:$address' })" ||
        dispatch closewindow "address:$address" || true
fi
restore

if ((spawned)); then
    "$here/fixtures.sh" clean
fi
log "== done: $failures failing scenario(s)"
exit $((failures > 0))
