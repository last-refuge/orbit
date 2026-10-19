#!/usr/bin/env python3
"""A stand-in for GitHub's releases API and download host, for the update
checks (ORB-011). Serves a directory of releases:

    fake-release-server.py serve DIR PORT

DIR/releases/<tag>/        one directory per release, e.g. v0.1.0-alpha.2/
    notes.md               the release body (optional)
    prerelease             present: marked pre-release (optional)
    draft                  present: marked draft (optional)
    *.tar.gz, *.sha256, *.minisig   the assets (served)
    remote.json            optional: {"assets": [{"name": …, "url": …}]}
                           assets hosted elsewhere (a real GitHub URL), listed
                           instead of local files

Endpoints, as Orbit's client uses them:
    GET /releases?per_page=N   newest first by semver; ETag; If-None-Match → 304
    GET /releases/latest       404, like GitHub for a repo whose releases are all pre-releases
    GET /download/<tag>/<asset>

Point Orbit at it with ORBIT_API_BASE=http://127.0.0.1:PORT.
"""

import hashlib
import json
import os
import re
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer


def semver_key(tag):
    core, _, pre = tag.lstrip("v").partition("-")
    nums = [int(x) if x.isdigit() else 0 for x in core.split(".")]
    while len(nums) < 3:
        nums.append(0)
    if not pre:
        return (nums, 1, [])
    parts = []
    for p in pre.split("."):
        parts.append((0, int(p)) if p.isdigit() else (1, p))
    return (nums, 0, parts)


def releases_json(root, base):
    out = []
    rel_dir = os.path.join(root, "releases")
    for tag in sorted(os.listdir(rel_dir), key=semver_key, reverse=True):
        d = os.path.join(rel_dir, tag)
        if not os.path.isdir(d):
            continue
        assets = []
        remote = os.path.join(d, "remote.json")
        if os.path.exists(remote):
            with open(remote) as f:
                for a in json.load(f).get("assets", []):
                    assets.append({"name": a["name"], "browser_download_url": a["url"], "size": 0})
        for name in sorted(os.listdir(d)):
            if name.endswith((".tar.gz", ".sha256", ".minisig")):
                assets.append({
                    "name": name,
                    "browser_download_url": f"{base}/download/{tag}/{name}",
                    "size": os.path.getsize(os.path.join(d, name)),
                })
        notes = ""
        if os.path.exists(os.path.join(d, "notes.md")):
            with open(os.path.join(d, "notes.md")) as f:
                notes = f.read()
        out.append({
            "tag_name": tag,
            "prerelease": os.path.exists(os.path.join(d, "prerelease")),
            "draft": os.path.exists(os.path.join(d, "draft")),
            "html_url": f"{base}/html/{tag}",
            "body": notes,
            "published_at": "2026-10-19T12:00:00Z",
            "assets": assets,
        })
    return json.dumps(out)


class Handler(BaseHTTPRequestHandler):
    root = "."
    base = ""
    requests = []

    def log_message(self, *_):
        pass

    def send(self, status, body=b"", content_type="application/json", extra=None):
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        for k, v in (extra or {}).items():
            self.send_header(k, v)
        self.end_headers()
        if body:
            self.wfile.write(body)

    def do_GET(self):
        Handler.requests.append(self.path)
        path = self.path.split("?")[0]
        if path == "/releases":
            body = releases_json(self.root, self.base).encode()
            etag = 'W/"%s"' % hashlib.sha256(body).hexdigest()[:32]
            if self.headers.get("If-None-Match") == etag:
                self.send(304, extra={"ETag": etag})
                return
            self.send(200, body, extra={"ETag": etag})
            return
        if path == "/releases/latest":
            self.send(404, b'{"message": "Not Found"}')
            return
        if path == "/requests":
            self.send(200, json.dumps(Handler.requests).encode())
            return
        m = re.fullmatch(r"/download/([^/]+)/([^/]+)", path)
        if m:
            file = os.path.join(self.root, "releases", m.group(1), m.group(2))
            if os.path.isfile(file):
                with open(file, "rb") as f:
                    data = f.read()
                self.send(200, data, "application/octet-stream")
                return
        self.send(404, b'{"message": "Not Found"}')

    def do_HEAD(self):
        self.do_GET()


def main():
    if len(sys.argv) != 4 or sys.argv[1] != "serve":
        print(__doc__)
        sys.exit(2)
    Handler.root = os.path.abspath(sys.argv[2])
    port = int(sys.argv[3])
    Handler.base = f"http://127.0.0.1:{port}"
    server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
    print(f"fake releases on {Handler.base} from {Handler.root}", flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
