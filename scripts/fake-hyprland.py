#!/usr/bin/env python3
"""A small stand-in for Hyprland 0.56's IPC, for running Orbit's daemon
end to end where Hyprland isn't available (headless sway in CI or the
cloud). It is not a compositor: windows are entries in a table.

    fake-hyprland.py serve --monitor NAME WIDTH HEIGHT
        Serves $XDG_RUNTIME_DIR/hypr/$HYPRLAND_INSTANCE_SIGNATURE/.socket.sock
        and .socket2.sock with fixture-like windows on workspaces 1, 91-93
        and special:orbitfx, until killed.
    fake-hyprland.py ctl [-j] COMMAND...
        A `hyprctl` replacement for scripts: `ctl -j clients`,
        `ctl dispatch "hl.dsp.…"`.

Understands the read queries Orbit makes and four Lua dispatches:
hl.dsp.window.move (with an explicit window), hl.dsp.focus({ window }),
hl.dsp.focus({ workspace }) and hl.dsp.window.alter_zorder({ mode, window }).
Like Hyprland, it answers `ok` to a move of an unknown address and does
nothing. Events follow each change on the event socket in Hyprland's line
format. Focus does not raise: like an all-floating Hyprland, a focused
window stays where it was in the stack until alter_zorder moves it, so a
test can tell that Orbit sent the raise. Two queries of its own:
`ctl -j fake/zorder` (addresses, top first) and `ctl -j fake/dispatches`
(every dispatch received, in order).
"""

import json
import os
import re
import socket
import sys
import threading

lock = threading.Lock()
listeners = []

FIXTURES = [
    # address, class, workspace name, floating
    (0x5a0001, "dev.orbit.fixture.tiled-a", "91", False),
    (0x5a0002, "dev.orbit.fixture.tiled-b", "91", False),
    (0x5a0003, "dev.orbit.fixture.floating", "92", True),
    (0x5a0004, "dev.orbit.fixture.fullscreen", "93", False),
    (0x5a0005, "dev.orbit.fixture.special", "special:orbitfx", False),
    (0x5a0010, "fake.terminal", "1", False),
]


def sockdir():
    return os.path.join(
        os.environ["XDG_RUNTIME_DIR"], "hypr", os.environ["HYPRLAND_INSTANCE_SIGNATURE"]
    )


class World:
    def __init__(self, monitor, width, height):
        self.monitor = monitor
        self.width, self.height = width, height
        # Bottom to top; focus never changes it (see the module doc).
        self.zorder = [address for address, *_ in FIXTURES]
        self.dispatches = []
        self.special_ids = {"special:orbitfx": -98}
        self.windows = {}
        for i, (address, cls, ws, floating) in enumerate(FIXTURES):
            self.windows[address] = {
                "class": cls,
                "title": f"Fake {cls.rsplit('.', 1)[-1]}",
                "workspace": ws,
                "floating": floating,
                "pid": 4000 + i,
            }
        self.active_ws = "1"
        self.active_window = 0x5a0010
        # Last focused window per workspace, for focus-on-switch.
        self.last_on = {"1": 0x5a0010}

    def ws_id(self, name):
        if name.startswith("special"):
            return self.special_ids.setdefault(name, -99 - len(self.special_ids))
        if name.isdigit():
            return int(name)
        return -1337 - sum(map(ord, name))

    def workspaces(self):
        names = {w["workspace"] for w in self.windows.values()} | {self.active_ws, "1"}
        return sorted(names, key=lambda n: (n.startswith("special"), self.ws_id(n)))

    def clients(self):
        out = []
        for address, w in sorted(self.windows.items()):
            out.append(
                {
                    "address": hex(address),
                    "mapped": True,
                    "hidden": False,
                    "at": [0, 0],
                    "size": [800, 600],
                    "workspace": {"id": self.ws_id(w["workspace"]), "name": w["workspace"]},
                    "floating": w["floating"],
                    "pseudo": False,
                    "monitor": 0,
                    "class": w["class"],
                    "title": w["title"],
                    "pid": w["pid"],
                    "xwayland": False,
                    "pinned": False,
                    "fullscreen": 2 if "fullscreen" in w["class"] else 0,
                    "grouped": [],
                }
            )
        return out

    def query(self, what):
        if what == "version":
            return {"version": "0.56.2", "tag": "v0.56.2", "commit": "fake", "branch": "fake"}
        if what in ("monitors", "monitors all"):
            return [
                {
                    "id": 0,
                    "name": self.monitor,
                    "width": self.width,
                    "height": self.height,
                    "refreshRate": 60.0,
                    "scale": 1.0,
                    "transform": 0,
                    "focused": True,
                    "disabled": False,
                    "activeWorkspace": {"id": self.ws_id(self.active_ws), "name": self.active_ws},
                    "specialWorkspace": {"id": 0, "name": ""},
                }
            ]
        if what == "workspaces":
            return [
                {
                    "id": self.ws_id(n),
                    "name": n,
                    "monitor": self.monitor,
                    "windows": sum(1 for w in self.windows.values() if w["workspace"] == n),
                    "hasfullscreen": False,
                    "ispersistent": n == "1",
                }
                for n in self.workspaces()
            ]
        if what == "activeworkspace":
            return {"id": self.ws_id(self.active_ws), "name": self.active_ws}
        if what == "clients":
            return self.clients()
        if what == "activewindow":
            if self.active_window in self.windows:
                return {"address": hex(self.active_window), "class": self.windows[self.active_window]["class"]}
            return {}
        if what == "workspacerules":
            return []
        if what == "fake/zorder":
            return [hex(a) for a in reversed(self.zorder)]
        if what == "fake/dispatches":
            return list(self.dispatches)
        return None

    def swap(self, lua):
        self.dispatches.append(lua.strip())
        match = re.search(r"local source, target = (\d+), (\d+)", lua)
        if not match:
            return "error: unsupported eval", []
        a, b = match.groups()
        if a not in self.workspaces() or b not in self.workspaces():
            return "error: workspace disappeared", []
        for label, workspace in [("a", a), ("b", b)]:
            group = re.search(r"check\(" + label + r", \{(.*?)\}\)", lua).group(1)
            expected = {int(address, 16) for address in re.findall(r"address:(0x[0-9a-f]+)", group)}
            actual = {address for address, w in self.windows.items() if w["workspace"] == workspace}
            if actual != expected:
                return "error: Workspace contents changed; try again", []
        events = []
        for old, new in [(a, "4294967294"), (b, a), ("4294967294", b)]:
            for w in self.windows.values():
                if w["workspace"] == old: w["workspace"] = new
            if self.active_ws == old: self.active_ws = new
            if old in self.last_on: self.last_on[new] = self.last_on.pop(old)
            events.append(f"changeworkspaceid>>{old},{new}")
        return "ok", events

    def dispatch(self, lua):
        events = []
        self.dispatches.append(lua.strip())
        before = set(self.workspaces())
        m = re.fullmatch(r"hl\.dsp\.window\.move\(\{(.*)\}\)", lua.strip())
        f = re.fullmatch(r"hl\.dsp\.focus\(\{(.*)\}\)", lua.strip())
        z = re.fullmatch(r"hl\.dsp\.window\.alter_zorder\(\{(.*)\}\)", lua.strip())
        body = (m or f or z).group(1) if (m or f or z) else None
        if body is None:
            return "error: fake-hyprland does not understand this dispatch", []
        args = dict(re.findall(r"(\w+)\s*=\s*'([^']*)'", body))
        if z:
            window = args.get("window", "")
            if not window.startswith("address:"):
                return "error: fake-hyprland: alter_zorder without an explicit window", []
            address = int(window[len("address:"):], 16)
            if args.get("mode") not in ("top", "bottom"):
                return "error: fake-hyprland: alter_zorder mode must be top or bottom", []
            if address in self.windows:
                self.zorder = [a for a in self.zorder if a != address]
                if args["mode"] == "top":
                    self.zorder.append(address)
                else:
                    self.zorder.insert(0, address)
        elif m:
            window = args.get("window", "")
            if not window.startswith("address:"):
                # Real Hyprland acts on the focused window here; refuse loudly
                # so tests catch a missing window argument.
                return "error: fake-hyprland: move without an explicit window", []
            address = int(window[len("address:"):], 16)
            target = args.get("workspace", "")
            target = target[len("name:"):] if target.startswith("name:") else target
            if address in self.windows and target:
                self.windows[address]["workspace"] = target
                wid = self.ws_id(target)
                events += [f"movewindow>>{address:x},{target}", f"movewindowv2>>{address:x},{wid},{target}"]
        elif "window" in args:
            address = int(args["window"][len("address:"):], 16)
            if address in self.windows:
                ws = self.windows[address]["workspace"]
                if not ws.startswith("special") and ws != self.active_ws:
                    self.active_ws = ws
                    events += [f"workspace>>{ws}", f"workspacev2>>{self.ws_id(ws)},{ws}"]
                self.active_window = address
                self.last_on[ws] = address
                events += [f"activewindowv2>>{address:x}"]
        elif "workspace" in args:
            ws = args["workspace"]
            ws = ws[len("name:"):] if ws.startswith("name:") else ws
            if ws != self.active_ws:
                self.active_ws = ws
                events += [f"workspace>>{ws}", f"workspacev2>>{self.ws_id(ws)},{ws}"]
                focus = self.last_on.get(ws)
                if focus not in self.windows or self.windows[focus]["workspace"] != ws:
                    focus = next((a for a, w in sorted(self.windows.items()) if w["workspace"] == ws), None)
                self.active_window = focus
                events += [f"activewindowv2>>{focus:x}" if focus else "activewindowv2>>"]
        after = set(self.workspaces())
        created = [f"createworkspacev2>>{self.ws_id(n)},{n}" for n in sorted(after - before)]
        destroyed = [f"destroyworkspacev2>>{self.ws_id(n)},{n}" for n in sorted(before - after)]
        return "ok", created + events + destroyed


def broadcast(lines):
    data = "".join(line + "\n" for line in lines).encode()
    for conn in list(listeners):
        try:
            conn.sendall(data)
        except OSError:
            listeners.remove(conn)


def handle_request(world, conn):
    with conn:
        message = conn.recv(65536).decode()
        for prefix in ("j/", "/"):
            if message.startswith(prefix):
                message = message[len(prefix):]
        with lock:
            if message.startswith("eval "):
                reply, events = world.swap(message[len("eval "):])
                broadcast(events)
            elif message.startswith("dispatch "):
                reply, events = world.dispatch(message[len("dispatch "):])
                # Hyprland writes a dispatch's events before its reply.
                broadcast(events)
            else:
                value = world.query(message.strip())
                reply = json.dumps(value) if value is not None else "unknown request"
        conn.sendall(reply.encode())


def serve(monitor, width, height):
    world = World(monitor, width, height)
    directory = sockdir()
    os.makedirs(directory, exist_ok=True)
    request_path = os.path.join(directory, ".socket.sock")
    event_path = os.path.join(directory, ".socket2.sock")
    for path in (request_path, event_path):
        if os.path.exists(path):
            os.unlink(path)
    requests = socket.socket(socket.AF_UNIX)
    requests.bind(request_path)
    requests.listen(16)
    events = socket.socket(socket.AF_UNIX)
    events.bind(event_path)
    events.listen(4)

    def accept_events():
        while True:
            conn, _ = events.accept()
            with lock:
                listeners.append(conn)

    threading.Thread(target=accept_events, daemon=True).start()
    print(f"fake Hyprland 0.56.2 on {directory}", flush=True)
    while True:
        conn, _ = requests.accept()
        threading.Thread(target=handle_request, args=(world, conn), daemon=True).start()


def ctl(args):
    as_json = args[:1] == ["-j"]
    if as_json:
        args = args[1:]
    message = ("j/" if as_json else "") + " ".join(args)
    with socket.socket(socket.AF_UNIX) as conn:
        conn.connect(os.path.join(sockdir(), ".socket.sock"))
        conn.sendall(message.encode())
        reply = b""
        while chunk := conn.recv(65536):
            reply += chunk
    print(reply.decode())


if __name__ == "__main__":
    if sys.argv[1:2] == ["serve"] and len(sys.argv) == 6 and sys.argv[2] == "--monitor":
        serve(sys.argv[3], int(sys.argv[4]), int(sys.argv[5]))
    elif sys.argv[1:2] == ["ctl"]:
        ctl(sys.argv[2:])
    else:
        print(__doc__, file=sys.stderr)
        sys.exit(2)
