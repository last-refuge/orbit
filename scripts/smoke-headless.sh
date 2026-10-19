#!/usr/bin/env bash
# Opens the smoke window and the layer-shell overlay inside a headless sway
# session and saves screenshots. Proves the build renders on Wayland without a
# GPU; it does not replace the native check on Hyprland.
#
# Usage: scripts/smoke-headless.sh path/to/orbit [output-dir]
# Needs: sway, grim, a Vulkan or GLES software driver (mesa).
set -euo pipefail

orbit=$(realpath "$1")
out=$(realpath -m "${2:-smoke-output}")
mkdir -p "$out"

runtime=$(mktemp -d)
chmod 700 "$runtime"
export XDG_RUNTIME_DIR=$runtime
printf 'output HEADLESS-1 resolution 1280x720\n' > "$runtime/sway.conf"

WLR_BACKENDS=headless WLR_RENDERER=pixman WLR_LIBINPUT_NO_DEVICES=1 \
    sway -c "$runtime/sway.conf" > "$out/sway.log" 2>&1 &
sway_pid=$!
trap 'kill $sway_pid 2>/dev/null || true; rm -rf "$runtime"' EXIT

for _ in $(seq 50); do
    socket=$(find "$runtime" -maxdepth 1 -name 'wayland-*' ! -name '*.lock' -printf '%f\n' | head -n1)
    [[ -n $socket ]] && break
    sleep 0.1
done
[[ -n ${socket:-} ]] || { echo "sway did not start" >&2; cat "$out/sway.log" >&2; exit 1; }
export WAYLAND_DISPLAY=$socket

check() {
    local name=$1; shift
    "$orbit" smoke "$@" --exit-after-ms 6000 > "$out/$name.log" 2>&1 &
    local pid=$!
    sleep 4
    grim "$out/$name.png"
    wait "$pid" || { echo "$name: orbit exited with $?" >&2; cat "$out/$name.log" >&2; return 1; }
    echo "$name: ok ($out/$name.png)"
}

check window
check overlay --overlay
