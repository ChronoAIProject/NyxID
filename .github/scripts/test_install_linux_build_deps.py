import hashlib
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / ".github/scripts/install-linux-build-deps.sh"


class BuildDependencyTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory(prefix="ci dependencies ")
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        for directory in (".github/scripts", "scripts", "security", "bin", "keyrings"):
            (self.root / directory).mkdir(parents=True)
        shutil.copyfile(SCRIPT, self.root / ".github/scripts" / SCRIPT.name)
        self.calls = self.root / "calls"
        self.arguments = self.root / "arguments"
        # Run the real helper AND snapshot installer. Redirect only their two
        # system filesystem destinations; all commands that could affect the
        # host are replaced with fixtures. Linux PR replay tests authenticate
        # real signatures/indexes/archives separately.
        installer = (ROOT / "scripts/apt-snapshot-install.sh").read_text()
        for before, after in (
            ('key=/usr/share/keyrings/$distro-archive-keyring.gpg',
             'key="$TEST_ROOT/keyrings/$distro-archive-keyring.gpg"'),
            ('evidence=/usr/share/nyxid/build-inputs/apt/$root_digest',
             'evidence="$TEST_ROOT/evidence/$root_digest"'),
        ):
            self.assertEqual(installer.count(before), 1)
            installer = installer.replace(before, after)
        (self.root / "scripts/apt-snapshot-install.sh").write_text(installer)
        shutil.copyfile(ROOT / "scripts/apt-snapshot-select.awk", self.root / "scripts/apt-snapshot-select.awk")
        shutil.copyfile(ROOT / "security/apt-transport-ca.pem", self.root / "security/apt-transport-ca.pem")
        shutil.copyfile(ROOT / "security/apt-transport-ca.tsv", self.root / "security/apt-transport-ca.tsv")
        shutil.copyfile(ROOT / "scripts/apt-transport-ca.awk", self.root / "scripts/apt-transport-ca.awk")
        metadata = b"fixture signed metadata\n"
        digest = hashlib.sha256(metadata).hexdigest()
        catalog = (ROOT / "security/apt-snapshots.tsv").read_text().splitlines()
        (self.root / "security/apt-snapshots.tsv").write_text("\n".join(
            line if line.startswith("#") else line.rsplit("\t", 1)[0] + "\t" + digest
            for line in catalog) + "\n")
        (self.root / "keyrings/ubuntu-archive-keyring.gpg").write_text("fixture keyring\n")
        self.harness = self.root / "commands.sh"
        self.harness.write_text('''
function .() {
  if [[ "$1" == /etc/os-release ]]; then
    ID=ubuntu
    VERSION_CODENAME=noble
  else
    builtin . "$@"
  fi
}
sudo() {
  [[ "$1" == -n && "$2" == sh && "$3" == "$TEST_ROOT/scripts/apt-snapshot-install.sh" &&
     "$4" == --install-only && "$5" == "$TEST_ROOT/security/apt-snapshots.tsv" &&
     "$6" == ubuntu:noble && "$7" == -- ]] || return 99
  shift
  command "$@"
}
''')
        self.stub("id", 'printf "0\\n"\n')
        self.stub("dpkg", '''case "$1" in
  --print-architecture) printf 'amd64\\n' ;;
  --print-foreign-architectures) : ;;
  *) exit 99 ;;
esac
''')
        self.stub("dpkg-query", "printf 'fixture-base\\t1\\tamd64\\tinstalled\\n'\n")
        self.stub("apt-get", '''import os
from pathlib import Path
import sys
args = sys.argv[1:]
options = dict(args[i + 1].split("=", 1) for i, arg in enumerate(args) if arg == "-o")
assert options["APT::Get::AllowUnauthenticated"] == "false"
assert options["Acquire::AllowInsecureRepositories"] == "false"
assert options["Acquire::https::Verify-Peer"] == "true"
assert options["Acquire::https::Verify-Host"] == "true"
original_ca = Path(os.environ["TEST_ROOT"]) / "security/apt-transport-ca.pem"
ca = Path(options["Acquire::https::CaInfo"])
assert ca != original_ca
assert ca == Path(options["Dir::State::lists"]).parent / "transport-ca.pem"
assert ca.read_bytes() == original_ca.read_bytes()
assert ca.stat().st_mode & 0o777 == 0o644
assert ca.parent.stat().st_mode & 0o777 == 0o755
operation = "update" if "update" in args else ("download" if "--download-only" in args else "install")
if operation != "update":
    assert "install" in args and "--no-install-recommends" in args
    assert ("--download-only" in args) != ("--no-download" in args)
with open(os.environ["TEST_ARGUMENTS"], "a") as output:
    output.write("\\n".join(args) + "\\n")
with open(os.environ["TEST_CALLS"], "a") as output:
    output.write(operation + "\\n")
if operation == os.environ["TEST_FAIL"]:
    sys.exit(17)
if operation == "update":
    sources = Path(options["Dir::Etc::sourcelist"]).read_text()
    assert "https://" not in sources and "http://snapshot.ubuntu.com/" in sources
    assert "signed-by=" in sources and "check-valid-until=no" in sources
    lists = Path(options["Dir::State::lists"])
    catalog = Path(os.environ["TEST_ROOT"]) / "security/apt-snapshots.tsv"
    for line in catalog.read_text().splitlines():
        if line.startswith("#"):
            continue
        distro, codename, suite, components, url, digest = line.split("\\t")
        if (distro, codename) != ("ubuntu", "noble"):
            continue
        prefix = (url.removeprefix("https://") + "/dists/" + suite + "/").replace("/", "_")
        (lists / (prefix + "InRelease")).write_bytes(b"fixture signed metadata\\n")
        for component in components.split(","):
            (lists / (prefix + component + "_binary-amd64_Packages")).write_text("fixture index\\n")
''', python=True)

    def stub(self, name, source, python=False):
        path = self.root / "bin" / name
        path.write_text(("#!/usr/bin/env python3\n" if python else "#!/bin/sh\nset -eu\n") + source)
        path.chmod(0o755)

    def invoke(self, failure=""):
        return subprocess.run(
            ["bash", str(self.root / ".github/scripts" / SCRIPT.name)],
            env={**os.environ, "BASH_ENV": str(self.harness), "TEST_ROOT": str(self.root),
                 "PATH": str(self.root / "bin") + os.pathsep + os.environ["PATH"],
                 "TEST_CALLS": str(self.calls), "TEST_ARGUMENTS": str(self.arguments), "TEST_FAIL": failure},
            capture_output=True, text=True, timeout=10,
        )

    def test_success_requires_update_and_install(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ["update", "download", "install"])
        self.assertEqual(len(list((self.root / "evidence").iterdir())), 1)

    def test_install_includes_backend_boundary_scanner(self):
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.arguments.read_text().splitlines().count("ripgrep"), 2)

    def test_private_input_parent_uses_verified_readable_transport_copy(self):
        self.root.chmod(0o700)
        (self.root / 'security').chmod(0o700)
        (self.root / 'security/apt-transport-ca.pem').chmod(0o600)
        result = self.invoke()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ['update', 'download', 'install'])

    def test_changed_transport_copy_stops_before_update(self):
        self.stub('cp', '''/bin/cp "$@"
case "$2" in */transport-ca.pem) printf corrupt >> "$2" ;; esac
''')
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn('APT transport CA copy checksum mismatch', result.stderr)
        self.assertFalse(self.calls.exists())

    def test_update_failure_stops_before_install(self):
        result = self.invoke("update")
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ["update"])
        self.assertFalse((self.root / "evidence").exists())

    def test_install_failure_is_not_ignored(self):
        result = self.invoke("install")
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ["update", "download", "install"])

    def test_corrupt_transport_ca_stops_before_update(self):
        (self.root / "security/apt-transport-ca.pem").write_text("corrupt transport trust")
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("transport CA bundle checksum mismatch", result.stderr)
        self.assertFalse(self.calls.exists())

    def test_missing_transport_ca_stops_before_update(self):
        (self.root / "security/apt-transport-ca.pem").unlink()
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("transport CA bundle missing", result.stderr)
        self.assertFalse(self.calls.exists())

    def test_missing_transport_provenance_stops_before_update(self):
        (self.root / "security/apt-transport-ca.tsv").unlink()
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("transport CA provenance missing", result.stderr)
        self.assertFalse(self.calls.exists())

    def test_http_transport_provenance_is_rejected_before_update(self):
        pins = self.root / "security/apt-transport-ca.tsv"
        pins.write_text(pins.read_text().replace("https://curl.se/", "http://curl.se/"))
        result = self.invoke()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("invalid APT transport CA provenance", result.stderr)
        self.assertFalse(self.calls.exists())

    def test_download_failure_stops_before_offline_install(self):
        result = self.invoke("download")
        self.assertEqual(result.returncode, 17, result.stderr)
        self.assertEqual(self.calls.read_text().splitlines(), ["update", "download"])


if __name__ == "__main__":
    unittest.main()
