#!/usr/bin/env bash
# Run ONLY in a private Hyprland with scripts/fixtures.sh's owned windows,
# an extra tiled-c fixture on 91, and an Orbit daemon started with --test-hooks.
# ORBIT_PRIVATE_TEST=1 XDG_RUNTIME_DIR=/tmp/... HYPRLAND_INSTANCE_SIGNATURE=...
# bash scripts/workspace-swap-native-check.sh [binary] [evidence-directory]
set -euo pipefail
[[ ${ORBIT_PRIVATE_TEST:-} == 1 && ${XDG_RUNTIME_DIR:-} == /tmp/* ]] || {
    echo 'Refusing: use an explicitly marked private /tmp compositor.' >&2; exit 1;
}
orbit=$(realpath "${1:-target/release/orbit}")
out=${2:-$(mktemp -d /tmp/orbit-native-swap.XXXXXX)}
mkdir -p "$out"
hyprctl -j clients > "$out/original.json"
jq -e 'length >= 5 and all(.[]; .class | startswith("dev.orbit.fixture."))' "$out/original.json" >/dev/null
"$orbit" show >/dev/null
sleep .5
snapshot() { hyprctl -j clients > "$1"; }
compare() {
    python3 - "$1" "$2" "$3" "$4" <<'PY'
import json,sys
before,after=(sorted(json.load(open(p)),key=lambda w:w['address']) for p in sys.argv[1:3])
a,b=map(int,sys.argv[3:5]); assert len(before)==len(after)
for old,new in zip(before,after):
    assert old['address']==new['address']
    expected={a:b,b:a}.get(old['workspace']['id'],old['workspace']['id'])
    assert new['workspace']['id']==expected,(old['class'],old['workspace'],new['workspace'])
    for key in ['at','size','floating','fullscreen','fullscreenClient','monitor','grouped','pinned']:
        assert old[key]==new[key],(old['class'],key,old[key],new[key])
PY
}
swap() {
    # The state snapshot can precede GPUI painting its new hit-test bounds.
    sleep .35
    "$orbit" drive drag-workspace "$1" "$2" >/dev/null
    for _ in $(seq 50); do
        if "$orbit" state | jq -e --arg to "$2" '.overview.rail_focus == $to and .overview.announcement == "Workspaces swapped; window layouts preserved"' >/dev/null; then return; fi
        sleep .1
    done
    "$orbit" state >&2; exit 1
}
for pair in '91 92' '91 93'; do
    read -r a b <<< "$pair"
    snapshot "$out/before-$a-$b.json"
    swap "$a" "$b"
    # Allow deferred workspace rule/layout refreshes to settle too.
    sleep .5
    snapshot "$out/after-$a-$b.json"
    compare "$out/before-$a-$b.json" "$out/after-$a-$b.json" "$a" "$b"
    swap "$b" "$a"
    sleep .5
    snapshot "$out/back-$a-$b.json"
    compare "$out/before-$a-$b.json" "$out/back-$a-$b.json" 0 0
    echo "PASS real Hyprland $a ↔ $b: exact geometry, floating/fullscreen state, round trip"
done
"$orbit" hide >/dev/null
hyprctl dispatch "hl.dsp.focus({ workspace = '91' })" >/dev/null
"$orbit" show >/dev/null
sleep .5
snapshot "$out/active-before.json"
for _ in $(seq 5); do
    swap 91 92
    hyprctl -j activeworkspace | jq -e '.id == 92' >/dev/null
    swap 92 91
    hyprctl -j activeworkspace | jq -e '.id == 91' >/dev/null
done
sleep .5
snapshot "$out/active-after.json"
compare "$out/active-before.json" "$out/active-after.json" 0 0
echo "PASS active workspace follows its contents; five round trips have no geometry drift"
printf 'Native evidence: %s\n' "$out"
