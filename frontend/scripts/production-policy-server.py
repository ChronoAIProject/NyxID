#!/usr/bin/env python3
"""Serve the real production dist through generated nginx; mock only transport.

This local fixture has no credentials or production connections. Start it before
`playwright test --config playwright.production.config.ts`. Container validation
uses the same renderer/template/dist in the pinned production nginx runtime.
"""
import argparse
import base64
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
from pathlib import Path
import signal
import socket
import subprocess
import threading
import time
import uuid
from urllib.parse import urlsplit, parse_qs
from urllib.request import urlopen


def transport(dist):
    manifest = json.loads((dist / "release-integrity/releases.json").read_text())
    scripts = [a for a in manifest["artifacts"] if a["role"] == "credential_accept_script"]
    for artifact in manifest["artifacts"]:
        content = (dist / artifact["path"].lstrip("/")).read_bytes()
        assert len(content) == artifact["size_bytes"]
        assert hashlib.sha384(content).hexdigest() == artifact["sha384_hex"]
    csp = "default-src 'none'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; form-action 'none'; script-src 'self'" + "".join(" '" + a["sha384_sri"] + "'" for a in scripts) + "; connect-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:"

    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *_):
            pass

        def respond(self, body, status=200, content_type="application/json", policy="default-src 'none'; frame-ancestors 'none'"):
            data = body if isinstance(body, bytes) else json.dumps(body).encode()
            self.send_response(status)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(data)))
            self.send_header("Content-Security-Policy", policy)
            self.send_header("Referrer-Policy", "no-referrer")
            self.send_header("Permissions-Policy", "camera=(), microphone=(), geolocation=()")
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            path = urlsplit(self.path).path
            if path == "/api/v1/policy-upstream-close":
                self.connection.shutdown(socket.SHUT_RDWR)
                self.close_connection = True
            elif self.headers.get("Upgrade", "").lower() == "websocket":
                key = self.headers.get("Sec-WebSocket-Key", "")
                accept = base64.b64encode(hashlib.sha1((key + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").encode()).digest()).decode()
                self.send_response(101)
                self.send_header("Upgrade", "websocket")
                self.send_header("Connection", "Upgrade")
                self.send_header("Sec-WebSocket-Accept", accept)
                self.end_headers()
                self.wfile.write(b"\x81\x09connected")
                self.wfile.flush()
                self.close_connection = True
            elif path.startswith("/credential-accept/assets/"):
                entry = next((a for a in scripts if a["path"] == path), None)
                if entry:
                    self.respond((dist / path.lstrip("/")).read_bytes(), content_type="text/javascript", policy=csp)
                else:
                    self.respond({}, 404)
            elif path.endswith("/accept") and path.startswith("/api/v1/nodes/"):
                self.respond((dist / "credential-accept/credential-accept.html").read_bytes(), content_type="text/html", policy=csp)
            elif path.endswith("policy-probe.js"):
                status = int(parse_qs(urlsplit(self.path).query).get("status", ["200"])[0])
                self.respond({"transport": "backend", "path": path}, status, policy="default-src 'none'; script-src 'sha256-dGVzdA=='; frame-ancestors 'none'")
            elif path == "/api/v1/public/config":
                self.respond({"email_auth_enabled": True, "social_providers": ["google"], "invite_code_required": False,
                              "telemetry_dsn": "", "telemetry_share_analytics": False})
            elif path == "/api/v1/users/me":
                self.respond({"error": "unauthorized", "message": "Test transport has no session", "error_code": 1001}, 401)
            elif path == "/health":
                self.respond({"status": "ok", "version": "fixture"})
            else:
                self.respond({})

        def do_POST(self):
            length = int(self.headers.get("Content-Length", "0"))
            remaining = length
            while remaining:
                data = self.rfile.read(min(remaining, 65536))
                if not data:
                    return
                remaining -= len(data)
            self.respond({"transport": "backend", "path": urlsplit(self.path).path, "size": length})

    return Handler


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    runtime = parser.add_mutually_exclusive_group(required=True)
    runtime.add_argument("--nginx", help="native nginx executable (recorded separately from production 1.27)")
    runtime.add_argument("--frontend-image", help="actual built frontend image; Linux Docker host network, no bind mounts")
    parser.add_argument("--mime-types", type=Path, help="required for native nginx")
    parser.add_argument("--evidence-dir", type=Path, required=True)
    parser.add_argument("--port", type=int, default=18089)
    parser.add_argument("--upstream-port", type=int, default=18489)
    parser.add_argument("--public-origin")
    parser.add_argument("--connect-origins", help="explicit HTTP(S) deployment origins; defaults include fixture API and upstream WebSocket origin")
    args = parser.parse_args()
    if args.connect_origins is None:
        args.connect_origins = f"https://api.nyxid.example http://127.0.0.1:{args.upstream_port}"
    frontend = Path(__file__).resolve().parents[1]
    dist = frontend / "dist"
    output = args.evidence_dir.resolve()
    output.mkdir(parents=True, exist_ok=True)
    container_name = "nyxid-policy-" + uuid.uuid4().hex[:12]
    container_log = None
    if args.frontend_image:
        # Inspect the actual shipped files, not a separately rebuilt page. The
        # same tests find asset names in local dist, so enforce byte parity.
        subprocess.run(["docker", "create", "--name", container_name, args.frontend_image], check=True, capture_output=True)
        try:
            image_dist = output / "image-dist"
            image_dist.mkdir(exist_ok=True)
            subprocess.run(["docker", "cp", container_name + ":/usr/share/nginx/html/.", str(image_dist)], check=True)
            expected = {str(p.relative_to(dist)): hashlib.sha256(p.read_bytes()).hexdigest()
                        for p in dist.rglob("*") if p.is_file()}
            actual = {str(p.relative_to(image_dist)): hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in image_dist.rglob("*") if p.is_file()}
            if actual != expected:
                raise ValueError("actual frontend image dist differs from the production test build")
            (output / "image-dist-sha256.json").write_text(json.dumps(actual, sort_keys=True, indent=2) + "\n")
            subprocess.run(["docker", "image", "inspect", args.frontend_image], check=True,
                           stdout=(output / "image-inspect.json").open("w"))
        finally:
            subprocess.run(["docker", "rm", container_name], check=True, capture_output=True)
    spec = importlib.util.spec_from_file_location("policy", frontend / "scripts/render-nginx.py")
    policy = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(policy)
    template = (frontend / "nginx.conf.template").read_text().replace("/var/log/nginx/access.log", str(output / "access.log"))
    files = policy.render({"BACKEND_URL": f"http://127.0.0.1:{args.upstream_port}",
                           "NYXID_PUBLIC_ORIGIN": args.public_origin or f"http://127.0.0.1:{args.port}",
                           "NYXID_BROWSER_CONNECT_ORIGINS": args.connect_origins},
                          template, output, dist, dist / "legal-cache.conf", args.port)
    for name, data in files.items():
        (output / name).write_text(data)
    if args.frontend_image:
        version = subprocess.run(["docker", "run", "--rm", "--entrypoint", "nginx", args.frontend_image, "-v"],
                                 capture_output=True, text=True, check=True)
        if "nginx/1.27." not in version.stderr + version.stdout:
            raise ValueError("production matrix requires actual nginx 1.27")
        container_log = (output / "access.log").open("w")
        command = ["docker", "run", "--rm", "--name", container_name, "--network", "host", "--read-only",
                   "--tmpfs", "/tmp", "--tmpfs", "/etc/nginx/nyxid/generated:uid=101,gid=101,mode=0700",
                   "--env", f"BACKEND_URL=http://127.0.0.1:{args.upstream_port}",
                   "--env", f"NYXID_PUBLIC_ORIGIN={args.public_origin or f'http://127.0.0.1:{args.port}'}",
                   "--env", f"NYXID_BROWSER_CONNECT_ORIGINS={args.connect_origins}",
                   "--entrypoint", "/bin/sh", args.frontend_image, "-c",
                   'python3 /etc/nginx/nyxid/render-nginx.py --port "$1" && exec nginx -g "daemon off;"',
                   "--", str(args.port)]
    else:
        if not args.mime_types:
            parser.error("--mime-types is required with --nginx")
        config = "\n".join(["worker_processes 1;", f"pid {policy.path_literal(output / 'nginx.pid')};",
            f"error_log {policy.path_literal(output / 'error.log')};", "events { worker_connections 128; }",
            "http {", f"include {policy.path_literal(args.mime_types)};", "default_type application/octet-stream;",
            f"client_body_temp_path {policy.path_literal(output / 'client_temp')};",
            f"proxy_temp_path {policy.path_literal(output / 'proxy_temp')};",
            f"include {policy.path_literal(output / 'default.conf')};", "}"])
        (output / "nginx.conf").write_text(config)
        version = subprocess.run([args.nginx, "-v"], capture_output=True, text=True, check=True)
        subprocess.run([args.nginx, "-t", "-p", str(output) + "/", "-c", str(output / "nginx.conf")], check=True)
        command = [args.nginx, "-p", str(output) + "/", "-c", str(output / "nginx.conf"), "-g", "daemon off;"]
    (output / "nginx-version.txt").write_text(version.stderr + version.stdout)
    server = ThreadingHTTPServer(("127.0.0.1", args.upstream_port), transport(dist))
    threading.Thread(target=server.serve_forever, daemon=True).start()
    process = subprocess.Popen(command, stdout=container_log, stderr=container_log)
    def stop(*_):
        if args.frontend_image:
            subprocess.run(["docker", "stop", "--time", "5", container_name], capture_output=True)
        else:
            process.terminate()
    signal.signal(signal.SIGTERM, stop)
    signal.signal(signal.SIGINT, stop)
    try:
        for _ in range(100):
            if process.poll() is not None:
                raise RuntimeError("nginx exited before readiness")
            try:
                with urlopen(f"http://127.0.0.1:{args.port}/build-version.json", timeout=1) as response:
                    assert response.status == 200
                break
            except OSError:
                time.sleep(0.1)
        else:
            raise RuntimeError("nginx readiness deadline exceeded")
        print(f"production dist ready http://127.0.0.1:{args.port}; {version.stderr.strip()}", flush=True)
        process.wait()
    finally:
        stop() if process.poll() is None else None
        process.wait(timeout=5)
        server.shutdown()
        server.server_close()
        if container_log:
            container_log.close()


if __name__ == "__main__":
    main()
