#!/usr/bin/env bash
# End-to-end check of staying up to date (ORB-011), in a throwaway HOME:
#
#   scripts/update-check.sh [path/to/orbit]
#
# The published alpha.1 has no updater, so both sides come from the working
# tree: the "old" install is the tree's binary at its real version V, and
# the "new" release is a wrapper around the same binary that reports V.9
# through ORBIT_VERSION_OVERRIDE (and names the installed path through
# ORBIT_INSTALLED_BINARY, since the wrapper execs the real file). Both
# hooks exist for exactly this.
#
# 1. install.sh installs V from a fixture release server (real download
#    path, real checksum verification).
# 2. `orbit update check` sees V.9; `update apply` swaps it in and keeps
#    orbit.prev; `update rollback` goes back; apply again.
# 3. The daemon on V, under the fake Hyprland, is asked to restart on V.9
#    after a CLI apply: it must answer afterwards and report the update.
# 4. A broken release (reports V.99, dies as a daemon) goes in through the
#    running daemon; the watcher must roll back and bring the daemon up.
#
# Never touches the real HOME, config, state or data; the daemon talks to
# fake-hyprland.py, never to the live compositor. Needs python3, curl, tar,
# sha256sum, jq.
set -uo pipefail

orbit=$(realpath "${1:-target/release/orbit}")
here=$(cd "$(dirname "$0")/.." && pwd)
[[ -x $orbit ]] || { echo "build first: cargo build --release" >&2; exit 1; }
old_version=$("$orbit" --version | sed 's/^orbit //')
new_version=$old_version.9
broken_version=$old_version.99
arch=$(uname -m)

T=$(mktemp -d /tmp/orbit-update-XXXX)
real_runtime=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
export HOME=$T XDG_CONFIG_HOME=$T/cfg XDG_CACHE_HOME=$T/cache XDG_DATA_HOME=$T/data \
    XDG_STATE_HOME=$T/state XDG_RUNTIME_DIR=$T/rt HYPRLAND_INSTANCE_SIGNATURE=fake-update
mkdir -p "$XDG_RUNTIME_DIR"
# The daemon draws through GPUI, which opens $XDG_RUNTIME_DIR/$WAYLAND_DISPLAY:
# link the session's Wayland socket in. Orbit's socket and the fake
# Hyprland's live in this directory and nowhere else.
if [[ -n ${WAYLAND_DISPLAY:-} && ${WAYLAND_DISPLAY} != /* && -S $real_runtime/$WAYLAND_DISPLAY ]]; then
    ln -s "$real_runtime/$WAYLAND_DISPLAY" "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY"
fi
unset ORBIT_CONFIG ORBIT_VERSION_OVERRIDE ORBIT_INSTALLED_BINARY
port=$((19000 + RANDOM % 900))
export ORBIT_API_BASE=http://127.0.0.1:$port
export ORBIT_DOWNLOAD_BASE=$ORBIT_API_BASE/download   # install.sh builds asset URLs from this
export ORBIT_UPDATE_START_DELAY_MS=3600000   # the daemon never checks on its own here
bin=$T/.local/bin/orbit
pids=()
failures=0
pass() { printf 'PASS  %s\n' "$1"; }
fail() { printf 'FAIL  %s\n' "$1"; failures=$((failures + 1)); }
check() { local msg=$1; shift; if "$@" >/dev/null 2>&1; then pass "$msg"; else fail "$msg"; fi; }
wait_until() { local tries=$1; shift; for _ in $(seq "$tries"); do "$@" >/dev/null 2>&1 && return 0; sleep 0.1; done; return 1; }
cleanup() {
    "$bin" quit >/dev/null 2>&1 || true
    sleep 0.3
    for p in "${pids[@]}"; do kill "$p" 2>/dev/null || true; done
    rm -rf "${T:?}"
}
trap cleanup EXIT

# --- the fixture server -------------------------------------------------------
fx=$T/fixture
package() { # package VERSION BINARY: a release tarball + sha256 into the fixture
    local version=$1 binary=$2 name="orbit-$1-$arch-unknown-linux-gnu" dir
    dir=$fx/releases/v$version
    mkdir -p "$dir/$name/share"
    touch "$dir/prerelease"
    cp "$binary" "$dir/$name/orbit"
    "$orbit" assets install "$dir/$name/share" >/dev/null
    (cd "$dir" && tar -czf "$name.tar.gz" "$name" && sha256sum "$name.tar.gz" > "$name.tar.gz.sha256" && rm -rf "$name")
}
wrapper() { # wrapper VERSION OUT: the tree's binary reporting VERSION
    cat > "$2" <<SH
#!/bin/sh
exec env ORBIT_VERSION_OVERRIDE=$1 ORBIT_INSTALLED_BINARY="\${ORBIT_INSTALLED_BINARY:-\$0}" "$orbit" "\$@"
SH
    chmod +x "$2"
}
package "$old_version" "$orbit"
python3 -I "$here/scripts/fake-release-server.py" serve "$fx" "$port" >"$T/fixture.log" 2>&1 &
pids+=($!)
python3 -I "$here/scripts/fake-hyprland.py" serve --monitor FAKE-1 1920 1080 >"$T/fake-hyprland.log" 2>&1 &
pids+=($!)
sleep 0.8
check "fixture server answers" curl -fsS "$ORBIT_API_BASE/releases?per_page=10" -o /dev/null

# --- 1. install V through the installer --------------------------------------
bash "$here/install.sh" >"$T/install.log" 2>&1 || { echo "install.sh failed:"; tail -5 "$T/install.log"; }
check "install.sh put $old_version in place from the fixture" test "$("$bin" --version)" = "orbit $old_version"
check "install.sh wrote the install marker" jq -e '.method == "release"' "$XDG_DATA_HOME/orbit/install.json"
check "update check with nothing newer says so" bash -c '"$0" update check | grep -q "is the latest"' "$bin"

# --- 2. check, apply, rollback from the command line --------------------------
wrapper "$new_version" "$T/new-orbit"
package "$new_version" "$T/new-orbit"
printf '## %s\n\nA fake release built from the working tree.\n' "$new_version" > "$fx/releases/v$new_version/notes.md"
out=$("$bin" update check 2>&1)
check "update check sees $new_version" grep -q "$new_version is available" <<<"$out"
check "update notes prints the fake notes" bash -c '"$0" update notes | grep -q "fake release"' "$bin"
check "state records the ETag" jq -e '.etag != null' "$XDG_STATE_HOME/orbit/update.json"
"$bin" update apply >"$T/apply.log" 2>&1
check "update apply installed $new_version" test "$("$bin" --version)" = "orbit $new_version"
check "the previous binary is kept" test "$("$T/.local/bin/orbit.prev" --version)" = "orbit $old_version"
check "the desktop entry was refreshed" test -f "$T/.local/share/applications/orbit.desktop"
check "install marker follows the version" jq -e --arg v "$new_version" '.installed_version == $v' "$XDG_DATA_HOME/orbit/install.json"
check "update apply on the latest says so" bash -c '"$0" update apply 2>&1 | grep -q "already installed"' "$bin"
check "update rollback restores $old_version" bash -c '"$0" update rollback >/dev/null 2>&1 && test "$("$0" --version)" = "orbit '"$old_version"'"' "$bin"
check "nothing left to roll back to" bash -c '! "$0" update rollback >/dev/null 2>&1' "$bin"

# --- 3. the running daemon restarts on the new binary -------------------------
RUST_LOG=warn "$bin" daemon --test-hooks >"$T/daemon.log" 2>&1 &
check "daemon ($old_version) answers under fake Hyprland" wait_until 100 "$bin" status
check "orbit state reports the update section" bash -c '"$0" state | jq -e --arg v "$1" ".update.current == \$v"' "$bin" "$old_version"
"$bin" update check >/dev/null 2>&1
check "the daemon re-reads the state after a CLI check" wait_until 30 bash -c '"$0" state | jq -e ".update.available == true"' "$bin"
"$bin" update apply >"$T/apply2.log" 2>&1
sleep 1.5
check "daemon answers after the restart" wait_until 80 "$bin" status
check "daemon now runs $new_version" bash -c '"$0" state | jq -e --arg v "$1" ".update.current == \$v"' "$bin" "$new_version"
check "daemon knows it was updated from $old_version" bash -c '"$0" state | jq -e --arg v "$1" ".update.applied.from == \$v"' "$bin" "$old_version"
check "the daemon shows the updated-to notice" bash -c '"$0" state | jq -e ".update.applied.version != null"' "$bin"

# --- 4. a broken release is rolled back by the watcher ------------------------
broken=$T/broken-orbit
cat > "$broken" <<SH
#!/bin/sh
case "\$1" in --version) echo "orbit $broken_version";; assets) exit 0;; *) exit 3;; esac
SH
chmod +x "$broken"
package "$broken_version" "$broken"
"$bin" update check >/dev/null 2>&1
check "the broken release is on offer" wait_until 30 bash -c '"$0" state | jq -e --arg v "$1" ".update.latest == \$v"' "$bin" "$broken_version"
"$bin" update apply >"$T/apply-broken.log" 2>&1
check "the broken binary went in" test "$("$bin" --version)" = "orbit $broken_version"
check "the watcher rolled back within its window" wait_until 200 bash -c 'test "$("$0" --version)" = "orbit $1"' "$bin" "$new_version"
check "the daemon is back on $new_version" wait_until 100 "$bin" status
check "the failed version is recorded and not offered again" bash -c '"$0" state | jq -e --arg v "$1" ".update.failed == \$v and .update.available == false"' "$bin" "$broken_version"

echo
echo "$failures failure(s)"
exit $((failures > 0))
