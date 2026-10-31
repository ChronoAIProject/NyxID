#!/usr/bin/env python3
"""Build-time only. Bake locked, home-less identities into the immutable image."""
import json
from pathlib import Path
import sys

FIRST_UID = 20000
COUNT = 2048
COMMENT = "nyxid-hosted-context-v1"


def install(root: Path):
    passwd_path, group_path, shadow_path = (root / "etc" / n for n in ("passwd", "group", "shadow"))
    passwd = passwd_path.read_text()
    groups = group_path.read_text()
    shadow = shadow_path.read_text()
    users = [line.split(":") for line in passwd.splitlines()]
    group_rows = [line.split(":") for line in groups.splitlines()]
    assert all(len(row) == 7 for row in users), "invalid passwd database"
    assert all(len(row) == 4 for row in group_rows), "invalid group database"
    assert len({row[0] for row in users}) == len(users), "duplicate user name"
    assert len({row[2] for row in users}) == len(users), "duplicate user uid"
    assert len({row[0] for row in group_rows}) == len(group_rows), "duplicate group name"
    assert len({row[2] for row in group_rows}) == len(group_rows), "duplicate group gid"
    names = {row[0] for row in users + group_rows}
    ids = {int(row[2]) for row in users + group_rows}
    ids.update(int(row[3]) for row in users)
    # Refuse every collision before writing anything; never repair/rebind a user.
    assert not any(uid in ids or f"nyxc{uid}" in names for uid in range(FIRST_UID, FIRST_UID + COUNT)), "context account collision"
    assert not any(line.split(":")[0].startswith("nyxc") for line in shadow.splitlines()), "context shadow collision"
    for uid in range(FIRST_UID, FIRST_UID + COUNT):
        name = f"nyxc{uid}"
        passwd += f"{name}:x:{uid}:{uid}:{COMMENT}:/nonexistent:/usr/sbin/nologin\n"
        groups += f"{name}:x:{uid}:\n"
        shadow += f"{name}:!:1:0:99999:7:::\n"
    passwd_path.write_text(passwd)
    group_path.write_text(groups)
    shadow_path.write_text(shadow)
    manifest = root / "usr/share/nyxid/hosted-context-accounts.json"
    manifest.parent.mkdir(parents=True, exist_ok=True)
    manifest.write_text(json.dumps({"version": 1, "first_uid": FIRST_UID, "count": COUNT}) + "\n")
    manifest.chmod(0o644)


if __name__ == "__main__":
    install(Path(sys.argv[1]) if len(sys.argv) == 2 else Path("/"))
