#!/usr/bin/env python3
"""Check exact reviewed dependency lifecycle denials without executing scripts."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REQUIRED_ROOTS = {"frontend", "sdk", ".github/tools/claude", "security/npm-tools", "cli/tests/updater-npm"}


def check(root, required_roots=REQUIRED_ROOTS):
    errors = []
    policy = json.loads((root / "security/node-install-policy.json").read_text())
    paths = [item["path"] for item in policy["roots"]]
    if policy.get("schema") != "nyxid.node-install-policy.v1" or set(paths) != set(required_roots) or len(paths) != len(set(paths)):
        errors.append("script policy must cover every required root exactly once")
    for item in policy["roots"]:
        path = root / item["path"]
        package = json.loads((path / "package.json").read_text())
        lock = json.loads((path / "package-lock.json").read_text())
        config = dict(line.strip().split("=", 1) for line in (path / ".npmrc").read_text().splitlines()
                      if line.strip() and not line.lstrip().startswith(("#", ";")))
        if config.get("ignore-scripts") != "true":
            errors.append(item["path"] + ": dependency lifecycle execution must be disabled")
        reviewed = {entry["lock_path"]: entry for entry in item["install_scripts"]}
        expected_names = {}
        for name, value in lock["packages"].items():
            if not value.get("hasInstallScript"):
                continue
            identity = value.get("name", name.rsplit("node_modules/", 1)[-1])
            review = reviewed.pop(name, None)
            expected = {"name": identity, "version": value.get("version"), "integrity": value.get("integrity")}
            if not review or any(review.get(key) != data for key, data in expected.items()) or \
                    review.get("allow") is not False or not review.get("reason", "").strip():
                errors.append(item["path"] + ": missing exact script-denial review: " + name)
            expected_names[identity] = False
        if reviewed:
            errors.append(item["path"] + ": stale script reviews: " + ", ".join(sorted(reviewed)))
        if package.get("allowScripts") != expected_names:
            errors.append(item["path"] + ": allowScripts must match explicit reviewed denials")
    return errors


if __name__ == "__main__":
    problems = check(ROOT)
    if problems:
        raise SystemExit("\n".join(problems))
    print("Reviewed dependency script denials match all configured Node roots")
