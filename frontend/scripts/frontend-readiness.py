#!/usr/bin/env python3
"""Check this image's static files as the nginx user; expose only a private Unix socket.

The inventory is frozen at image build time, outside the served document root.
Each request reopens files (no cached success). Assets require a readable regular
file of the built size; HTML also requires the built SHA-256. This is a
delivery check, not full asset integrity verification or a browser transaction.
"""
import argparse
import hashlib
from html.parser import HTMLParser
from http.server import BaseHTTPRequestHandler
import json
import os
import re
from pathlib import Path, PurePosixPath
from socketserver import UnixStreamServer
import stat

MAX_FILES = 4096
MAX_MANIFEST_BYTES = 1024 * 1024
MAX_ENTRY_BYTES = 1024 * 1024


class EntryAssets(HTMLParser):
    def __init__(self):
        super().__init__()
        self.paths = set()

    def handle_starttag(self, tag, attributes):
        attrs = dict(attributes)
        path = None
        if tag == "script" and attrs.get("type") == "module":
            path = attrs.get("src")
        elif tag == "link" and attrs.get("rel") in ("stylesheet", "modulepreload"):
            path = attrs.get("href")
        # Optional remote fonts are outside this image's static delivery check.
        if path and tag == "link" and path.startswith("https://"):
            return
        if path:
            if not path.startswith("/assets/") or "?" in path or "#" in path:
                raise ValueError("entry assets must use local build paths")
            self.paths.add(path.lstrip("/"))


def inventory(root):
    entry = root / "index.html"
    parser = EntryAssets()
    if not entry.is_file() or not 0 < entry.stat().st_size <= MAX_ENTRY_BYTES:
        raise ValueError("built entry HTML is required")
    parser.feed(entry.read_text())
    metadata_path = root / "build-version.json"
    metadata_required = not any(path.endswith(".css") for path in parser.paths)
    if metadata_required:
        # The public/app dispatcher loads its CSS with the selected entry. Use
        # the producer's app-entry closure to require that CSS before readiness.
        try:
            metadata = json.loads(metadata_path.read_text())
            assets = metadata.get("assets") if isinstance(metadata, dict) else None
            if (not isinstance(assets, list) or not 1 <= len(assets) <= 256
                    or not all(isinstance(asset, str) and
                               re.fullmatch(r"assets/[a-zA-Z0-9_.-]+\.(js|css)", asset)
                               for asset in assets)
                    or not parser.paths.issubset(assets)):
                raise ValueError("invalid app-entry build metadata")
            parser.paths.update(assets)
        except (OSError, ValueError) as error:
            raise ValueError("built app-entry metadata is required") from error
    if not all(any(path.endswith(suffix) for path in parser.paths) for suffix in (".js", ".css")):
        raise ValueError("built entry JS and CSS are required")
    paths = sorted({"index.html", *(str(p.relative_to(root)) for p in root.rglob("*")
                                    if p.suffix in (".html", ".js", ".css")),
                    *(("build-version.json",) if metadata_required else ())})
    if (not parser.paths.issubset(paths) or len(paths) > MAX_FILES
            or not any(p.endswith(".js") for p in parser.paths)
            or not any(p.endswith(".css") for p in parser.paths)):
        raise ValueError("entry assets missing or inventory too large")
    files = [{"path": path, "size": (root / path).stat().st_size,
              **({"sha256": hashlib.sha256((root / path).read_bytes()).hexdigest()}
                 if path.endswith(".html") else {})} for path in paths]
    result = {"files": files}
    if check(root, result):
        raise ValueError("built static files must be readable and nonempty")
    return result


def valid_inventory(data):
    if not isinstance(data, dict):
        return False
    files = data.get("files")
    if not isinstance(files, list) or not 3 <= len(files) <= MAX_FILES:
        return False
    paths = set()
    for item in files:
        if not isinstance(item, dict):
            return False
        path, size = item.get("path"), item.get("size")
        if (not isinstance(path, str) or not path or PurePosixPath(path).is_absolute()
                or ".." in PurePosixPath(path).parts or path in paths
                or not isinstance(size, int) or size <= 0):
            return False
        paths.add(path)
        if path.endswith(".html"):
            digest = item.get("sha256")
            if (not isinstance(digest, str) or len(digest) != 64
                    or any(char not in "0123456789abcdef" for char in digest)):
                return False
    return ("index.html" in paths and any(p.endswith(".js") for p in paths)
            and any(p.endswith(".css") for p in paths))


def check(root, data):
    if not valid_inventory(data):
        return ["manifest"]
    root = root.resolve()
    failing = set()
    for item in data["files"]:
        path = root / item["path"]
        reason = "entry_html" if item["path"].endswith(".html") else "static_assets"
        try:
            if not path.resolve().is_relative_to(root):
                raise OSError("outside document root")
            descriptor = os.open(path, os.O_RDONLY | os.O_NONBLOCK)
            try:
                info = os.fstat(descriptor)
                if not stat.S_ISREG(info.st_mode) or info.st_size != item["size"]:
                    raise OSError("not the built regular file")
                if reason == "entry_html":
                    if (info.st_size > MAX_ENTRY_BYTES
                            or hashlib.sha256(os.read(descriptor, MAX_ENTRY_BYTES + 1)).hexdigest()
                            != item["sha256"]):
                        raise OSError("entry differs from build")
                elif not os.read(descriptor, 1):
                    raise OSError("unreadable asset")
            finally:
                os.close(descriptor)
        except OSError:
            failing.add(reason)
    return sorted(failing)


def serve(root, manifest, socket_path):
    try:
        data = json.loads(manifest.read_bytes()[:MAX_MANIFEST_BYTES + 1]) if manifest.stat().st_size <= MAX_MANIFEST_BYTES else None
    except (OSError, ValueError):
        data = None

    class Handler(BaseHTTPRequestHandler):
        def setup(self):
            self.request.settimeout(1)
            super().setup()

        def log_message(self, *_):
            pass  # Never log file paths, exceptions, requests or configuration.

        def do_GET(self):
            failures = check(root, data)
            body = {"status": "unavailable" if failures else "ok", "component": "frontend"}
            if failures:
                body["failing"] = failures
            encoded = json.dumps(body, separators=(",", ":")).encode()
            self.send_response(503 if failures else 200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Cache-Control", "no-store")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)

    # The entrypoint and nginx run as UID 101. No TCP port is opened, and the
    # socket is private to that user. A dead/stalled checker fails closed at nginx.
    os.umask(0o077)
    socket_path.unlink(missing_ok=True)
    with UnixStreamServer(str(socket_path), Handler) as server:
        server.serve_forever()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=Path("/usr/share/nginx/html"))
    parser.add_argument("--manifest", type=Path, default=Path("/etc/nginx/nyxid/static-readiness.json"))
    parser.add_argument("--socket", type=Path, default=Path("/tmp/nyxid-frontend-readiness.sock"))
    parser.add_argument("--inventory", action="store_true")
    args = parser.parse_args()
    if args.inventory:
        args.manifest.write_text(json.dumps(inventory(args.root), separators=(",", ":")))
    else:
        serve(args.root.resolve(), args.manifest, args.socket)


if __name__ == "__main__":
    main()
