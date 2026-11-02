import importlib.util
from pathlib import Path
import tempfile
import unittest

SCRIPT = Path(__file__).resolve().parents[1] / "container/hosted-account-pool.py"
spec = importlib.util.spec_from_file_location("pool", SCRIPT)
pool = importlib.util.module_from_spec(spec)
spec.loader.exec_module(pool)


class AccountPoolTests(unittest.TestCase):
    def fixture(self, root):
        (root / "etc").mkdir()
        (root / "etc/passwd").write_text("root:x:0:0:root:/root:/bin/sh\nagent:x:1000:1000::/home/agent:/bin/sh\n")
        (root / "etc/group").write_text("root:x:0:\nagent:x:1000:\n")
        (root / "etc/shadow").write_text("root:!:1:0:99999:7:::\nagent:!:1:0:99999:7:::\n")

    def test_supply_is_bounded_locked_and_has_no_home_or_login(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            self.fixture(root)
            pool.install(root)
            users = [line.split(":") for line in (root / "etc/passwd").read_text().splitlines()][2:]
            self.assertEqual(len(users), 2048)
            self.assertEqual([int(row[2]) for row in users], list(range(20000, 22048)))
            self.assertTrue(all(row[2] == row[3] and row[5:] == ["/nonexistent", "/usr/sbin/nologin"] for row in users))
            self.assertTrue(all(line.split(":")[1] == "!" for line in (root / "etc/shadow").read_text().splitlines()))
            self.assertFalse((root / "nonexistent").exists())
            before = (root / "etc/passwd").read_bytes()
            with self.assertRaisesRegex(AssertionError, "collision"):
                pool.install(root)
            self.assertEqual((root / "etc/passwd").read_bytes(), before)

    def test_collisions_fail_before_any_database_write(self):
        for row in ["other:x:20000:20000::/home/other:/bin/sh\n", "nyxc20000:x:99:99::/:/bin/sh\n", "agent:x:99:99::/:/bin/sh\n", "other:x:99:20000::/:/bin/sh\n"]:
            with self.subTest(row=row), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                self.fixture(root)
                with (root / "etc/passwd").open("a") as stream:
                    stream.write(row)
                before = {name: (root / "etc" / name).read_bytes() for name in ["passwd", "group", "shadow"]}
                with self.assertRaises(AssertionError):
                    pool.install(root)
                self.assertEqual(before, {name: (root / "etc" / name).read_bytes() for name in before})


if __name__ == "__main__":
    unittest.main()
