#!/usr/bin/env python3
"""HTTP fault/recovery tests using the production Dockerfile's exact runtime stage.

Build inputs are a tiny static fixture; no npm install, user credentials or
external service connections are required. Docker is required (including in CI).
"""
import json
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import unittest
from urllib.error import HTTPError, URLError
from urllib.request import urlopen
import uuid

FRONTEND = Path(__file__).resolve().parents[1]


def docker(*args, **kwargs):
    return subprocess.run(["docker", *args], check=True, capture_output=True, text=True, **kwargs).stdout.strip()


class NginxReadiness(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.name = "nyxid-readiness-" + uuid.uuid4().hex[:12]
        cls.image = cls.name + ":test"
        with tempfile.TemporaryDirectory() as directory:
            context = Path(directory)
            dist = context / "dist"
            (dist / "assets").mkdir(parents=True)
            (dist / "index.html").write_text('<html><script type="module" src="/assets/app.js"></script><link rel="stylesheet" href="/assets/app.css"></html>')
            for filename in ["app.js", "lazy.js", "app.css"]:
                (dist / "assets" / filename).write_text("/* static fixture */")
            (dist / "legal-cache.conf").write_text('map $uri $nyxid_legal_etag { default ""; }\n')
            for filename in ["nginx.conf.template", "nginx-default.conf", "scripts/render-nginx.py",
                             "scripts/frontend-readiness.py", "scripts/nginx-policy-entrypoint.sh"]:
                target = context / "frontend" / filename
                target.parent.mkdir(parents=True, exist_ok=True)
                shutil.copyfile(FRONTEND / filename, target)
            runtime = (FRONTEND / "Dockerfile").read_text().split("# ── Stage 2: Runtime", 1)[1]
            runtime = runtime[runtime.index("FROM "):]
            (context / "Dockerfile").write_text("FROM scratch AS builder\nCOPY dist /build/frontend/dist\n" + runtime)
            build = subprocess.run(["docker", "build", "-t", cls.image, str(context)], text=True,
                                   stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
            if build.returncode:
                raise RuntimeError(build.stdout)
        cls.addClassCleanup(lambda: docker("image", "rm", cls.image))

    def setUp(self):
        self.name = "nyxid-readiness-" + uuid.uuid4().hex[:12]
        docker("run", "-d", "--name", self.name, "-p", "127.0.0.1::8080",
               "-e", "BACKEND_URL=http://127.0.0.1:18081", self.image)
        self.addCleanup(lambda: docker("rm", "-f", self.name))
        info = json.loads(docker("inspect", self.name))[0]
        if "8080/tcp" not in info["NetworkSettings"]["Ports"]:
            raise RuntimeError("frontend exited: " + docker("logs", self.name))
        binding = info["NetworkSettings"]["Ports"]["8080/tcp"][0]
        self.base = "http://127.0.0.1:" + binding["HostPort"]
        for _ in range(100):
            try:
                if self.request("/frontend/ready")[0] == 200:
                    break
            except (URLError, ConnectionError):
                pass
            time.sleep(0.1)
        else:
            raise RuntimeError("frontend failed to start: " + docker("logs", self.name))

    def request(self, path):
        try:
            response = urlopen(self.base + path, timeout=4)
        except HTTPError as error:
            response = error
        with response:
            return response.status, response.headers, response.read()

    def probe(self, path, status, body):
        actual_status, headers, data = self.request(path)
        self.assertEqual(actual_status, status, data)
        self.assertEqual(headers["Content-Type"], "application/json")
        self.assertEqual(headers.get_all("Cache-Control"), ["no-store"])
        self.assertNotIn("Location", headers)
        self.assertEqual(json.loads(data), body)

    def healthy(self):
        self.probe("/frontend/health", 200, {"status": "ok", "component": "frontend"})
        self.probe("/frontend/ready", 200, {"status": "ok", "component": "frontend"})

    def test_01_backend_down_cannot_mask_frontend_health_or_readiness(self):
        self.healthy()
        self.assertEqual(self.request("/health")[0], 502)
        self.assertEqual(self.request("/ready")[0], 502)

    def test_02_missing_or_unreadable_artifacts_fail_then_recover(self):
        for path, reason in [("index.html", "entry_html"), ("assets/app.js", "static_assets"),
                             ("assets/app.css", "static_assets"), ("assets/lazy.js", "static_assets")]:
            target = "/usr/share/nginx/html/" + path
            for operation in ["missing", "unreadable"]:
                with self.subTest(path=path, operation=operation):
                    if operation == "missing":
                        docker("exec", "--user", "0", self.name, "mv", target, target + ".saved")
                    else:
                        docker("exec", "--user", "0", self.name, "chmod", "000", target)
                    try:
                        self.probe("/frontend/health", 200, {"status": "ok", "component": "frontend"})
                        self.probe("/frontend/ready", 503, {"status": "unavailable", "component": "frontend", "failing": [reason]})
                        self.assertIn(self.request("/" + path)[0], [403, 404, 500])
                    finally:
                        if operation == "missing":
                            docker("exec", "--user", "0", self.name, "mv", target + ".saved", target)
                        else:
                            docker("exec", "--user", "0", self.name, "chmod", "644", target)
                    self.healthy()

    def test_03_explicit_routes_do_not_use_spa_fallback(self):
        for path in ["/frontend/ready/", "/frontend/ready.js", "/frontend/health/", "/frontend/health.css", "/ready/", "/ready.js"]:
            with self.subTest(path=path):
                status, _, body = self.request(path)
                self.assertEqual(status, 404)
                self.assertNotIn(b"static fixture", body)
        self.healthy()

    def test_04_legacy_health_and_backend_ready_are_forwarded(self):
        source = '''from http.server import BaseHTTPRequestHandler, HTTPServer
class Handler(BaseHTTPRequestHandler):
 def do_GET(self):
  self.send_response(200 if self.path == "/health" else 503)
  self.send_header("Content-Type", "application/json")
  self.send_header("Cache-Control", "max-age=86400")
  self.end_headers()
  self.wfile.write(b'{"status":"ok","version":"legacy"}' if self.path == "/health" else b'{"status":"unavailable","failing":["mongodb"]}')
HTTPServer(("127.0.0.1", 18081), Handler).serve_forever()
'''
        docker("exec", "-d", self.name, "python3", "-c", source)
        for _ in range(50):
            if self.request("/health")[0] == 200:
                break
            time.sleep(0.1)
        self.probe("/health", 200, {"status": "ok", "version": "legacy"})
        self.probe("/ready", 503, {"status": "unavailable", "failing": ["mongodb"]})
        self.healthy()
        target = "/usr/share/nginx/html/index.html"
        docker("exec", "--user", "0", self.name, "mv", target, target + ".saved")
        try:
            self.probe("/health", 200, {"status": "ok", "version": "legacy"})
            self.probe("/frontend/ready", 503, {"status": "unavailable", "component": "frontend", "failing": ["entry_html"]})
        finally:
            docker("exec", "--user", "0", self.name, "mv", target + ".saved", target)
        self.healthy()

    def test_05_checker_unavailable_is_bounded_and_never_cached(self):
        process = docker("exec", self.name, "python3", "-c", '''import pathlib
for p in pathlib.Path("/proc").glob("[0-9]*/cmdline"):
 if p.read_bytes().split(b"\\0")[:2] == [b"python3", b"/etc/nginx/nyxid/frontend-readiness.py"]:
  print(p.parent.name)
''')
        self.assertTrue(process)
        docker("exec", self.name, "kill", "-STOP", process)
        try:
            start = time.monotonic()
            self.probe("/frontend/ready", 503, {"status": "unavailable", "component": "frontend", "failing": ["checker"]})
            self.assertLess(time.monotonic() - start, 3.5)
        finally:
            docker("exec", self.name, "kill", "-CONT", process)
        self.healthy()
        docker("exec", self.name, "rm", "/tmp/nyxid-frontend-readiness.sock")
        self.probe("/frontend/ready", 503, {"status": "unavailable", "component": "frontend", "failing": ["checker"]})
        self.probe("/frontend/health", 200, {"status": "ok", "component": "frontend"})
        docker("stop", "--time", "2", self.name)
        with self.assertRaises((URLError, ConnectionError)):
            self.request("/frontend/health")


if __name__ == "__main__":
    unittest.main(verbosity=2)
