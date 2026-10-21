#!/usr/bin/env python3
"""Adversarial regressions for the build-input admission boundary."""
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tarfile
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def module(name):
    spec = importlib.util.spec_from_file_location(name.replace("-", "_"), ROOT / "scripts" / (name + ".py"))
    result = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(result)
    return result


policy = module("check-build-inputs")
bootstrap = module("bootstrap-build-tool")


class LockedNextestBootstrap(unittest.TestCase):
    def test_nextest_archive_installs_and_queries_its_cargo_subcommand(self):
        # cargo-nextest is a Cargo plugin: plain --version is not its CLI.
        binary = b'#!/bin/sh\n[ "$#" = 2 ] && [ "$1" = nextest ] && [ "$2" = --version ] || exit 41\necho "cargo-nextest 0.9.146"\n'
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "security").mkdir()
            shutil.copyfile(ROOT / "security/build-tools.json", root / "security/build-tools.json")
            archive = root / "nextest.tar.gz"
            with tarfile.open(archive, "w:gz") as package:
                member = tarfile.TarInfo("cargo-nextest")
                member.size = len(binary)
                package.addfile(member, io.BytesIO(binary))
            destination = root / "bin"
            with patch.object(bootstrap, "ROOT", root), \
                    patch.object(bootstrap, "host_target", return_value="x86_64-unknown-linux-gnu"), \
                    patch.object(bootstrap, "download_verified", side_effect=lambda _, path: shutil.copyfile(archive, path)), \
                    patch.object(sys, "argv", ["bootstrap-build-tool.py", "cargo-nextest", "--bin-dir", str(destination)]):
                bootstrap.main()
            self.assertEqual((destination / "cargo-nextest").read_bytes(), binary)
            self.assertEqual(subprocess.check_output([str(destination / "cargo-nextest"), "nextest", "--version"], text=True).strip(),
                             "cargo-nextest 0.9.146")


class BuildPolicy(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        for directory in (".github/workflows", ".github/scripts"):
            shutil.copytree(ROOT / directory, self.root / directory)
        (self.root / "security").mkdir()
        # Build artifacts/caches under security/ are not policy inputs.
        for path in (ROOT / "security").iterdir():
            if path.is_file():
                shutil.copyfile(path, self.root / "security" / path.name)
        shutil.copytree(ROOT / "security/apk", self.root / "security/apk")
        (self.root / "scripts").mkdir()
        for name in ("apt-snapshot-select.awk", "apk-catalog.awk", "install-release-dependencies.sh"):
            shutil.copyfile(ROOT / "scripts" / name, self.root / "scripts" / name)
        self.dockerfiles = subprocess.check_output(["git", "ls-files", "*Dockerfile*"], cwd=ROOT, text=True).splitlines()
        for name in self.dockerfiles:
            target = self.root / name
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / name, target)
        self.metadata = json.loads(subprocess.check_output(
            ["cargo", "metadata", "--locked", "--no-deps", "--format-version=1"], cwd=ROOT, text=True))
        self.mock = patch.object(policy.subprocess, "check_output", side_effect=lambda command, **_: (
            json.dumps(self.metadata) if command[0] == "cargo" else "\n".join(self.dockerfiles)))
        self.mock.start()
        self.addCleanup(self.mock.stop)
        self.addCleanup(self.temporary.cleanup)

    def mutate(self, name, old, new, reason):
        path = self.root / name
        text = path.read_text()
        self.assertIn(old, text)
        path.write_text(text.replace(old, new, 1))
        self.assertTrue(any(reason in error for error in policy.check(self.root)))

    def test_reviewed_inputs_pass(self):
        self.assertEqual(policy.check(self.root), [])

    def test_mutable_action_and_unknown_commit_rejected(self):
        name = ".github/workflows/release.yml"
        old = "actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803"
        self.mutate(name, old, "actions/checkout@v6", "unpinned Action")
        self.mutate(name, "actions/checkout@v6", "actions/checkout@" + "a" * 40, "lacks resolution receipt")

    def test_mutable_base_rejected(self):
        self.mutate("frontend/Dockerfile", "node:22.23.2-alpine@sha256:b6f26b36c8ff49624cfdac716b8ea1138d606df02586a77d364bb5536a634f85", "node:22.23.2-alpine", "unpinned base")

    def test_raw_apt_rejected_in_any_dockerfile(self):
        self.mutate("cli/Dockerfile.node", "-- python3 libc6", "-- python3 libc6 && apt-get install curl",
                    "raw APT command bypasses")

    def test_missing_apt_parser_copy_rejected(self):
        self.mutate("cli/Dockerfile.node", "scripts/apt-snapshot-select.awk ", "", "omits installer/parser/catalog COPY")

    def test_cargo_dist_metadata_mutable_image_rejected(self):
        self.metadata["metadata"]["dist"]["github-custom-runners"]["aarch64-unknown-linux-gnu"]["container"]["image"] = "buildpack-deps:focal"
        self.assertTrue(any("Cargo dist metadata" in error and "unpinned consumed image" in error
                            for error in policy.check(self.root)))

    def test_generated_plan_mutable_image_rejected(self):
        plan = {"ci": {"github": {"artifacts_matrix": {"include": [{"container": {"image": "buildpack-deps:focal"}}]}}}}
        self.assertTrue(any("generated dist plan" in error and "unpinned consumed image" in error
                            for error in policy.check(self.root, plan)))

    def test_mutable_remote_fixture_rejected(self):
        fixtures = self.root / "security/test-images.json"
        value = json.loads(fixtures.read_text())
        value["images"]["mailbox"] = "busybox:latest"
        fixtures.write_text(json.dumps(value))
        self.assertTrue(any("test fixture mailbox: unpinned consumed image" in error
                            for error in policy.check(self.root)))

    def test_yaml_extension_action_rejected(self):
        (self.root / ".github/workflows/new.yaml").write_text("jobs: {test: {steps: [{uses: actions/checkout@v6}]}}")
        self.assertTrue(any("new.yaml: unpinned Action" in error for error in policy.check(self.root)))

    def test_generated_installer_rejected(self):
        self.mutate(".github/workflows/release.yml", "python3 scripts/bootstrap-build-tool.py cargo-dist", "${{ matrix.install_dist.run }} #", "executable generated")

    def test_generated_auditable_installer_dot_and_brackets_rejected(self):
        path = self.root / ".github/workflows/installer.yaml"
        for expression in ("matrix.install_cargo_auditable.run", "matrix['install_cargo_auditable']['run']", 'matrix["install_cargo_auditable"].run'):
            path.write_text("run: ${{ " + expression + " }}\n")
            self.assertTrue(any("installer.yaml: executable generated" in error for error in policy.check(self.root)), expression)

    def test_bootstrap_targets_preserve_libc_and_reject_unknown(self):
        catalog = json.loads((ROOT / "security/build-tools.json").read_text())
        for system, arch, libc, expected in (("Darwin", "arm64", None, "aarch64-apple-darwin"),
                ("Darwin", "x86_64", None, "x86_64-apple-darwin"),
                ("Linux", "aarch64", "gnu", "aarch64-unknown-linux-gnu"),
                ("Linux", "x86_64", "musl", "x86_64-unknown-linux-musl")):
            target = bootstrap.target_for_host(arch, system, libc)
            self.assertEqual(target, expected)
            self.assertIn(target, catalog["cargo-auditable"]["targets"])
        for arch, system, libc in (("arm64", "Linux", None), ("mips", "Linux", "gnu"), ("arm64", "Other", None)):
            with self.assertRaisesRegex(ValueError, "unsupported build host"):
                bootstrap.target_for_host(arch, system, libc)
        with self.assertRaisesRegex(ValueError, "unsupported syft host target"):
            bootstrap.target_input("syft", catalog["syft"], "aarch64-unknown-linux-musl")

    def test_auditable_uses_pinned_static_linux_tool_without_changing_build_target(self):
        catalog = json.loads((ROOT / "security/build-tools.json").read_text())
        for arch in ("aarch64", "x86_64"):
            gnu = arch + "-unknown-linux-gnu"
            musl = arch + "-unknown-linux-musl"
            self.assertEqual(bootstrap.target_for_host(arch, "Linux", "gnu"), gnu)
            for target in (gnu, musl):
                self.assertEqual(bootstrap.target_input("cargo-auditable", catalog["cargo-auditable"], target),
                                 catalog["cargo-auditable"]["targets"][musl])
            for tool in ("rustup", "cargo-dist"):
                self.assertEqual(bootstrap.target_input(tool, catalog[tool], gnu), catalog[tool]["targets"][gnu])
        for target in ("aarch64-apple-darwin", "x86_64-apple-darwin"):
            self.assertEqual(bootstrap.target_input("cargo-auditable", catalog["cargo-auditable"], target),
                             catalog["cargo-auditable"]["targets"][target])
        with self.assertRaisesRegex(ValueError, "unsupported cargo-auditable host target"):
            bootstrap.target_input("cargo-auditable", catalog["cargo-auditable"], "mips-unknown-linux-gnu")

    def test_unlocked_rust_build_rejected(self):
        self.mutate("backend/Dockerfile", "cargo auditable build --locked", "cargo auditable build", "does not enforce lockfile")

    def test_unlocked_workflow_cargo_compilation_rejected(self):
        for command in ('build', 'check', 'clippy', 'test', 'nextest archive'):
            path = self.root / '.github/workflows/unlocked.yml'
            path.write_text('jobs:\n  probe:\n    steps:\n      - run: cargo ' + command + ' -p nyxid\n')
            with self.subTest(command=command):
                self.assertTrue(any('unlocked.yml: Rust build does not enforce lockfile' in error
                                    for error in policy.check(self.root)))
        path.unlink()

    def test_missing_binary_dependency_metadata_rejected(self):
        self.mutate("backend/Dockerfile", "cargo auditable build --locked", "cargo build --locked", "omits auditable metadata")
        self.metadata["metadata"]["dist"]["cargo-auditable"] = False
        self.assertTrue(any("require cargo-auditable" in error for error in policy.check(self.root)))

    def test_dist_source_hook_precise_builds_and_guarded_execution_are_required(self):
        for field, value in (("source-tarball", True), ("precise-builds", False), ("extra-artifacts", [])):
            original = self.metadata["metadata"]["dist"][field]
            self.metadata["metadata"]["dist"][field] = value
            self.assertTrue(any("explicit verified raw-source" in error for error in policy.check(self.root)))
            self.metadata["metadata"]["dist"][field] = original
        self.mutate(".github/workflows/release.yml", "python3 scripts/run-dist-build.py", "dist build #",
                    "required locked child completion")

    def test_download_checksum_failure(self):
        response = io.BytesIO(b"well-formed executable with altered bytes")
        response.url = "https://github.com/official/release"
        with patch.object(bootstrap.urllib.request, "urlopen", return_value=response):
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                bootstrap.download_verified({"url": response.url, "sha256": hashlib.sha256(b"reviewed bytes").hexdigest()}, self.root / "download")

    def test_verified_rustup_uses_installer_basename_and_preserves_install_arguments(self):
        tools = self.root / "tools with spaces"
        verified = []

        def download(item, path):
            self.assertEqual(item, json.loads((ROOT / "security/build-tools.json").read_text())
                             ["rustup"]["targets"]["aarch64-unknown-linux-gnu"])
            path.write_bytes(b"verified installer fixture")
            verified.append(path)

        def execute(argv, **kwargs):
            self.assertEqual(Path(argv[0]), verified[0])
            self.assertEqual(Path(argv[0]).name, "rustup-init")
            self.assertEqual(Path(argv[0]).read_bytes(), b"verified installer fixture")
            self.assertEqual(Path(argv[0]).stat().st_mode & 0o777, 0o755)
            self.assertEqual(argv[1:], ["-y", "--no-modify-path", "--profile", "minimal",
                                       "--default-toolchain", json.loads((ROOT / "security/build-tools.json").read_text())["rust_toolchain"]])
            self.assertEqual(kwargs, {"check": True})

        with patch.object(sys, "argv", ["bootstrap-build-tool.py", "rustup", "--bin-dir", str(tools)]), \
                patch.object(bootstrap, "host_target", return_value="aarch64-unknown-linux-gnu"), \
                patch.object(bootstrap, "download_verified", side_effect=download), \
                patch.object(bootstrap.subprocess, "run", side_effect=execute) as run:
            bootstrap.main()
        run.assert_called_once()
        self.assertEqual(list(tools.iterdir()), [])

    def test_rustup_verification_failure_prevents_execution_and_cleans_staging(self):
        tools = self.root / "unverified tools"
        with patch.object(sys, "argv", ["bootstrap-build-tool.py", "rustup", "--bin-dir", str(tools)]), \
                patch.object(bootstrap, "host_target", return_value="aarch64-unknown-linux-gnu"), \
                patch.object(bootstrap, "download_verified", side_effect=ValueError("build tool checksum mismatch")), \
                patch.object(bootstrap.subprocess, "run") as run:
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                bootstrap.main()
        run.assert_not_called()
        self.assertEqual(list(tools.iterdir()), [])

    def test_github_cli_uses_checksum_verified_host_archive_before_install(self):
        tools = self.root / 'github-cli'
        def download(item, archive):
            self.assertIn('/cli/cli/releases/download/v2.102.0/', item['url'])
            self.assertRegex(item['sha256'], '^[0-9a-f]{64}$')
            with tarfile.open(archive, 'w:gz') as package:
                binary = tarfile.TarInfo('gh_2.102.0_linux_arm64/bin/gh')
                binary.size = len(b'synthetic gh'); package.addfile(binary, io.BytesIO(b'synthetic gh'))
        with patch.object(sys, 'argv', ['bootstrap-build-tool.py', 'gh', '--bin-dir', str(tools)]), \
                patch.object(bootstrap, 'host_target', return_value='aarch64-unknown-linux-gnu'), \
                patch.object(bootstrap, 'download_verified', side_effect=download) as fetch, \
                patch.object(bootstrap.subprocess, 'run') as run:
            bootstrap.main()
        fetch.assert_called_once()
        self.assertEqual((tools / 'gh').read_bytes(), b'synthetic gh')
        run.assert_called_once_with([str((tools / 'gh').resolve()), '--version'], check=True)

    def test_corrupt_github_cli_archive_cannot_install_or_execute(self):
        tools = self.root / 'github-cli'
        with patch.object(sys, 'argv', ['bootstrap-build-tool.py', 'gh', '--bin-dir', str(tools)]), \
                patch.object(bootstrap, 'host_target', return_value='x86_64-unknown-linux-gnu'), \
                patch.object(bootstrap, 'download_verified', side_effect=ValueError('build tool checksum mismatch')), \
                patch.object(bootstrap.subprocess, 'run') as run:
            with self.assertRaisesRegex(ValueError, 'checksum mismatch'): bootstrap.main()
        run.assert_not_called()
        self.assertEqual(list(tools.iterdir()), [])

    def test_unsafe_npm_archive_rejected(self):
        path = self.root / "npm.tar.gz"
        with tarfile.open(path, "w:gz") as archive:
            item = tarfile.TarInfo("package/../../outside")
            item.size = 1
            archive.addfile(item, io.BytesIO(b"x"))
        with self.assertRaisesRegex(ValueError, "invalid npm archive path"):
            bootstrap.unpack_npm(path, self.root / "npm")


if __name__ == "__main__":
    unittest.main()
