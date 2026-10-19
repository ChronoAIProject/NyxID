import hashlib
import importlib.util
import io
import json
from pathlib import Path
import struct
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("dist_artifacts", Path(__file__).with_name("check-dist-artifacts.py"))
artifacts = importlib.util.module_from_spec(spec)
spec.loader.exec_module(artifacts)


class DistArtifactTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.output = self.root / "output"
        self.output.mkdir()
        self.subject = {"name": "nyxid-cli-aarch64-apple-darwin.tar.gz", "binary": "nyxid", "target": "aarch64-apple-darwin",
                        "assets": [{"path": "nyxid"}, {"path": "LICENSE"}]}
        self.header = b"\xcf\xfa\xed\xfe" + struct.pack("<III", 0x0100000C, 0, 2) + b"\x00" * 48

    def archive(self, members):
        path = self.root / self.subject["name"]
        with tarfile.open(path, "w:gz") as archive:
            for name, data, kind in members:
                member = tarfile.TarInfo(name)
                member.type = kind
                member.size = len(data)
                if kind == tarfile.SYMTYPE:
                    member.linkname = "/outside"
                archive.addfile(member, io.BytesIO(data))
        return path

    def inspect(self, members):
        return artifacts.inspect_archive(self.archive(members), self.subject, self.output)

    def good(self):
        return [("nyxid", self.header, tarfile.REGTYPE), ("LICENSE", b"license", tarfile.REGTYPE)]

    def test_exact_flat_and_prefixed_archives_bind_archive_and_members(self):
        for prefix in ("", self.subject["name"][:-7] + "/"):
            receipt = self.inspect([(prefix + name, data, kind) for name, data, kind in self.good()])
            self.assertEqual(receipt["digest"], artifacts.file_digest(self.root / self.subject["name"]))
            binary = next(item for item in receipt["assets"] if item["path"] == "nyxid")
            self.assertEqual(binary["sha256"], hashlib.sha256(self.header).hexdigest())

    def test_missing_duplicate_extra_link_and_traversal_assets_fail(self):
        good = self.good()
        for members in (good[:1], good + [good[0]], good + [("unexpected", b"extra", tarfile.REGTYPE)],
                        [("nyxid", b"", tarfile.SYMTYPE), good[1]],
                        [("../nyxid", self.header, tarfile.REGTYPE), good[1]],
                        [("nested//nyxid", self.header, tarfile.REGTYPE), good[1]]):
            with self.assertRaises(ValueError):
                self.inspect(members)

    def test_wrong_binary_architecture_or_format_fails(self):
        for data in (b"not executable", self.header[:4] + struct.pack("<I", 0x01000007) + self.header[8:]):
            with self.assertRaisesRegex(ValueError, "architecture/format"):
                self.inspect([("nyxid", data, tarfile.REGTYPE), self.good()[1]])

    def test_elf_kind_and_gnu_interpreter_are_required(self):
        def elf(interpreter, kind=3):
            header = bytearray(64)
            header[:6] = b"\x7fELF\x02\x01"
            struct.pack_into("<HH", header, 16, kind, 183)
            struct.pack_into("<QQ", header, 24, 4096, 64)
            struct.pack_into("<HH", header, 54, 56, 1)
            return header + struct.pack("<IIQQQQQQ", 3, 4, 120, 0, 0, len(interpreter), len(interpreter), 1) + interpreter
        binary = self.root / "linux-binary"
        binary.write_bytes(elf(b"/lib/ld-linux-aarch64.so.1\x00"))
        artifacts.check_binary_target(binary, "aarch64-unknown-linux-gnu")
        for data in (elf(b"/lib/ld-musl-aarch64.so.1\x00"), elf(b"/lib/ld-linux-aarch64.so.1\x00", 1)):
            binary.write_bytes(data)
            with self.assertRaises(ValueError):
                artifacts.check_binary_target(binary, "aarch64-unknown-linux-gnu")

    def test_archive_sbom_keeps_binary_and_archive_subjects_distinct(self):
        inspection = self.inspect(self.good())
        binary_ref = "sha256:" + hashlib.sha256(self.header).hexdigest()
        binary_bom = {"metadata": {"component": {"type": "application", "name": "nyxid", "bom-ref": binary_ref,
                      "properties": [{"name": "nyxid:source-commit", "value": "a" * 40}]}},
                      "components": [], "dependencies": [{"ref": binary_ref, "dependsOn": []}]}
        document = artifacts.archive_sbom(binary_bom, self.subject, inspection)
        self.assertEqual(document["metadata"]["component"]["bom-ref"], inspection["digest"])
        self.assertEqual(document["components"][0]["bom-ref"], binary_ref)
        self.assertIn(binary_ref, document["dependencies"][-1]["dependsOn"])
        self.assertEqual(binary_bom["metadata"]["component"]["bom-ref"], binary_ref)


if __name__ == "__main__":
    unittest.main()
