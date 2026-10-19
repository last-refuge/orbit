#!/usr/bin/env bash
# Opens or closes Orbit's owned test windows in the current Hyprland session.
#
#   scripts/fixtures.sh spawn [path/to/orbit]   open the fixture set
#   scripts/fixtures.sh list                    show fixture windows
#   scripts/fixtures.sh clean                   close fixture windows only
#
# Fixtures go to workspaces 91-93 and special:orbitfx so they stay out of the
# way of real work, and are opened silently so focus doesn't move. Only windows
# whose class starts with dev.orbit.fixture. (or the XWayland fixture class) are
# ever touched. Needs hyprctl and jq.
#
# Hyprland's Lua dispatch (`hl.dsp.*`, 0.56+) is tried first; the legacy
# hyprlang dispatcher syntax is the fallback for older sessions.
set -euo pipefail

prefix=dev.orbit.fixture.
x11_class=OrbitFixtureX11

# Runs a dispatch and fails loudly unless Hyprland answers "ok".
dispatch() {
    local reply
    reply=$(hyprctl dispatch "$@" 2>&1) || true
    [[ $reply == ok ]] && return 0
    echo "hyprctl dispatch $* -> $reply" >&2
    return 1
}

# exec_silent WORKSPACE EXTRA_LUA_RULES EXTRA_LEGACY_RULES COMMAND
exec_silent() {
    local workspace=$1 lua_rules=$2 legacy_rules=$3 command=$4
    [[ $command != *"'"* ]] || { echo "command must not contain single quotes: $command" >&2; return 1; }
    dispatch "hl.dsp.exec_cmd('$command', { workspace = '$workspace silent', no_initial_focus = true$lua_rules })" 2>/dev/null ||
        dispatch exec "[workspace $workspace silent$legacy_rules] $command"
}

close_window() {
    dispatch "hl.dsp.window.close({ window = 'address:$1' })" 2>/dev/null ||
        dispatch closewindow "address:$1"
}

fixture_query() {
    hyprctl -j clients | jq -r --arg prefix "$prefix" --arg x11 "$x11_class" \
        ".[] | select((.class | startswith(\$prefix)) or .class == \$x11) | $1"
}

case ${1:-} in
spawn)
    orbit=$(realpath "${2:-target/release/orbit}")
    [[ -x $orbit ]] || { echo "build first: cargo build --release" >&2; exit 1; }
    exec_silent 91 "" "" "$orbit fixture --name tiled-a"
    exec_silent 91 "" "" "$orbit fixture --name tiled-b"
    exec_silent 92 ", float = true" "; float" "$orbit fixture --name floating"
    exec_silent 93 "" "; fullscreen" "$orbit fixture --name fullscreen"
    exec_silent special:orbitfx "" "" "$orbit fixture --name special"
    if command -v xterm >/dev/null; then
        exec_silent 92 "" "" "xterm -class $x11_class -T Orbit-fixture-x11"
    else
        echo "xterm not installed: skipping the XWayland fixture (sudo pacman -S xterm)" >&2
    fi
    # exec_cmd's fullscreen rule is ignored on 0.56, so fullscreen the window
    # by address once it has mapped.
    for _ in $(seq 30); do
        address=$(fixture_query "select(.class == \"${prefix}fullscreen\") | .address")
        [[ -n $address ]] && break
        sleep 0.1
    done
    if [[ -n ${address:-} ]]; then
        dispatch "hl.dsp.window.fullscreen({ window = 'address:$address', mode = 'fullscreen', action = 'set' })" ||
            echo "could not fullscreen the fullscreen fixture" >&2
    fi
    sleep 0.5
    "$0" list
    ;;
list)
    fixture_query '"\(.address)\t\(.workspace.name)\t\(.class)\tfloating=\(.floating) fullscreen=\(.fullscreen) xwayland=\(.xwayland)"'
    ;;
clean)
    fixture_query '.address' | while read -r address; do
        close_window "$address"
    done
    ;;
*)
    sed -n '2,15p' "$0" | sed 's/^# \{0,1\}//'
    exit 2
    ;;
esac
