#!/usr/bin/env python3
"""Static readiness contract tests; run the companion nginx test for HTTP routing."""
import importlib.util
import copy
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

spec = importlib.util.spec_from_file_location("readiness", Path(__file__).with_name("frontend-readiness.py"))
readiness = importlib.util.module_from_spec(spec)
spec.loader.exec_module(readiness)


class StaticReadiness(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        (self.root / "assets").mkdir()
        (self.root / "index.html").write_text('<html><script type="module" src="/assets/app.js"></script><link rel="stylesheet" href="/assets/app.css"></html>')
        (self.root / "assets/app.js").write_text('import("./lazy.js")')
        (self.root / "assets/lazy.js").write_text("export const lazy = true")
        (self.root / "assets/app.css").write_text("body {color: black}")
        self.inventory = readiness.inventory(self.root)

    def test_current_build_is_ready(self):
        self.assertEqual(readiness.check(self.root, self.inventory), [])

    def test_missing_entry_and_recovery(self):
        entry = self.root / "index.html"
        content = entry.read_bytes()
        entry.unlink()
        self.assertEqual(readiness.check(self.root, self.inventory), ["entry_html"])
        entry.write_bytes(content)
        self.assertEqual(readiness.check(self.root, self.inventory), [])

    def test_generic_html_cannot_replace_current_entry(self):
        (self.root / "index.html").write_text("<html>Login</html>")
        self.assertEqual(readiness.check(self.root, self.inventory), ["entry_html"])

    def test_app_shell_and_prerendered_pages_fail_closed_and_recover(self):
        entries = [self.root / "app.html", self.root / "__public/home.html"]
        for entry in entries:
            entry.parent.mkdir(exist_ok=True)
            entry.write_text("<html>Built page</html>")
        inventory = readiness.inventory(self.root)
        for entry in entries:
            content = entry.read_bytes()
            with self.subTest(entry=entry.name):
                entry.unlink()
                self.assertEqual(readiness.check(self.root, inventory), ["entry_html"])
                entry.write_bytes(b"x" * len(content))
                self.assertEqual(readiness.check(self.root, inventory), ["entry_html"])
                entry.write_bytes(content)
                self.assertEqual(readiness.check(self.root, inventory), [])

    def test_remote_fonts_are_optional_but_local_entry_assets_are_required(self):
        entry = self.root / "index.html"
        entry.write_text(entry.read_text() + '<link rel="stylesheet" href="https://fonts.googleapis.com/css2?family=Example">')
        self.assertEqual(readiness.check(self.root, readiness.inventory(self.root)), [])

    def test_dispatcher_requires_app_css_from_matching_build_metadata(self):
        (self.root / "index.html").write_text('<html><script type="module" src="/assets/app.js"></script></html>')
        metadata = self.root / "build-version.json"
        with self.assertRaises(ValueError):
            readiness.inventory(self.root)
        for assets in [["assets/app.css"], ["assets/app.js"], ["assets/app.js", "../outside.css"]]:
            metadata.write_text(json.dumps({"assets": assets}))
            with self.subTest(assets=assets), self.assertRaises(ValueError):
                readiness.inventory(self.root)
        metadata.write_text(json.dumps({"assets": ["assets/app.js", "assets/app.css"]}))
        inventory = readiness.inventory(self.root)
        self.assertEqual(readiness.check(self.root, inventory), [])
        (self.root / "assets/app.css").unlink()
        self.assertEqual(readiness.check(self.root, inventory), ["static_assets"])
        with self.assertRaises(ValueError):
            readiness.inventory(self.root)

    def test_missing_entrypoint_stylesheet_or_lazy_chunk(self):
        for filename in ["app.js", "app.css", "lazy.js"]:
            path = self.root / "assets" / filename
            content = path.read_bytes()
            with self.subTest(filename=filename):
                path.unlink()
                self.assertEqual(readiness.check(self.root, self.inventory), ["static_assets"])
                path.write_bytes(content)
                self.assertEqual(readiness.check(self.root, self.inventory), [])

    def test_truncated_assets_fail(self):
        (self.root / "assets/app.js").write_bytes(b"")
        self.assertEqual(readiness.check(self.root, self.inventory), ["static_assets"])

    def test_unreadable_files_fail(self):
        real_open = readiness.os.open
        for filename, reason in [("index.html", "entry_html"), ("assets/app.css", "static_assets")]:
            def deny(path, flags):
                if Path(path) == (self.root / filename).resolve():
                    raise PermissionError("private filesystem details")
                return real_open(path, flags)
            with self.subTest(filename=filename), patch.object(readiness.os, "open", side_effect=deny):
                self.assertEqual(readiness.check(self.root, self.inventory), [reason])

    def test_non_regular_files_and_outside_symlinks_fail(self):
        path = self.root / "assets/app.js"
        path.unlink()
        path.mkdir()
        self.assertEqual(readiness.check(self.root, self.inventory), ["static_assets"])
        path.rmdir()
        path.symlink_to("/etc/passwd")
        self.assertEqual(readiness.check(self.root, self.inventory), ["static_assets"])

    def test_invalid_inventory_fails_closed(self):
        bad_entries = [None, {"path": "../etc/passwd", "size": 1},
                       {"path": "/etc/passwd", "size": 1}, {"path": "assets/../app.js", "size": 1},
                       {"path": "assets/app.js", "size": 0}, {"path": "assets/app.js", "size": "1"}]
        cases = [None, {}, {**self.inventory, "files": []}]
        for bad in bad_entries:
            data = copy.deepcopy(self.inventory)
            data["files"][0] = bad
            cases.append(data)
        duplicate = copy.deepcopy(self.inventory)
        duplicate["files"].append(duplicate["files"][0])
        cases.append(duplicate)
        for data in cases:
            with self.subTest(data=data):
                self.assertEqual(readiness.check(self.root, data), ["manifest"])

    def test_inventory_requires_built_entry_js_and_css(self):
        for filename in ["index.html", "assets/app.js", "assets/app.css"]:
            path = self.root / filename
            content = path.read_bytes()
            path.unlink()
            with self.subTest(filename=filename), self.assertRaises(ValueError):
                readiness.inventory(self.root)
            path.write_bytes(content)

    def test_inventory_rejects_generic_html_without_entry_assets(self):
        (self.root / "index.html").write_text("<html>Login</html>")
        with self.assertRaises(ValueError):
            readiness.inventory(self.root)


if __name__ == "__main__":
    unittest.main()
