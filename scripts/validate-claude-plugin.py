#!/usr/bin/env python3
"""Validate the NyxID Claude plugin against the Claude directory's submission rules.

Uses only the Python standard library. Covers the manifest, remote MCP server,
skill frontmatter, README/LICENSE, and the directory's bundle limits (file
count, file size, symlinks, blocked file types). When the `claude` CLI is on
PATH, its strict manifest validator runs as well.
"""

from __future__ import annotations

import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent / "integrations" / "claude-plugin"

NAME_RE = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
SEMVER_RE = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$")
URL_FIELDS = ("homepage", "documentationUrl", "supportUrl", "privacyPolicyUrl", "termsOfServiceUrl")
MAX_FILES = 512
MAX_FILE_BYTES = 256 * 1024
IMAGE_SUFFIXES = {".png", ".jpg", ".jpeg", ".gif", ".webp", ".svg"}
BLOCKED_SUFFIXES = {".ico", ".pdf", ".zip"}
BLOCKED_NAMES = {".DS_Store"}
MIN_README_WORDS = 40


class ValidationError(Exception):
    """A user-facing validation failure."""


def fail(message: str) -> None:
    raise ValidationError(message)


def load_json(path: Path) -> dict:
    try:
        value = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"{path}: invalid JSON ({exc})")
    if not isinstance(value, dict):
        fail(f"{path}: must be a JSON object")
    return value


def validate_manifest() -> None:
    manifest = load_json(ROOT / ".claude-plugin" / "plugin.json")
    if not isinstance(manifest.get("name"), str) or not NAME_RE.fullmatch(manifest["name"]):
        fail("plugin name must be lowercase kebab-case")
    for field in ("description", "license"):
        if not isinstance(manifest.get(field), str) or not manifest[field].strip():
            fail(f"plugin {field} is required")
    if not isinstance(manifest.get("version"), str) or not SEMVER_RE.fullmatch(manifest["version"]):
        fail("plugin version must be semantic versioning")
    author = manifest.get("author")
    if not isinstance(author, dict) or not str(author.get("name", "")).strip():
        fail("plugin author.name is required")
    for field in URL_FIELDS:
        value = manifest.get(field)
        if not isinstance(value, str) or not value.startswith("https://"):
            fail(f"plugin {field} must be an https URL")
    icon = manifest.get("icon")
    if not isinstance(icon, str) or Path(icon).is_absolute() or ".." in Path(icon).parts:
        fail("plugin icon must be a relative path inside the plugin")
    if not (ROOT / icon).is_file():
        fail(f"plugin icon does not exist: {icon}")


def validate_mcp() -> None:
    servers = load_json(ROOT / ".mcp.json").get("mcpServers")
    if not isinstance(servers, dict) or not servers:
        fail(".mcp.json must declare at least one server in mcpServers")
    for name, server in servers.items():
        if not isinstance(server, dict):
            fail(f".mcp.json server {name} must be an object")
        if server.get("type") != "http":
            fail(f".mcp.json server {name} must use type \"http\" (remote MCP)")
        if not str(server.get("url", "")).startswith("https://"):
            fail(f".mcp.json server {name} must use an https url")
        if server.get("headers"):
            fail(f".mcp.json server {name} must not embed headers; authentication is OAuth")


def frontmatter(path: Path) -> dict[str, str]:
    text = path.read_text(encoding="utf-8")
    match = re.match(r"^---\n(.*?)\n---\n", text, re.DOTALL)
    if not match:
        fail(f"{path}: missing YAML frontmatter")
    fields = {}
    for line in match.group(1).splitlines():
        key, sep, value = line.partition(":")
        if sep:
            fields[key.strip()] = value.strip()
    return fields


def validate_skills() -> None:
    skills = sorted((ROOT / "skills").glob("*/SKILL.md"))
    if not skills:
        fail("plugin must bundle at least one skill under skills/<name>/SKILL.md")
    for skill in skills:
        fields = frontmatter(skill)
        for field in ("name", "description"):
            if not fields.get(field):
                fail(f"{skill}: frontmatter requires {field}")
        if fields["name"] != skill.parent.name:
            fail(f"{skill}: frontmatter name must match its directory")


def validate_docs() -> None:
    readme = ROOT / "README.md"
    if not readme.is_file():
        fail("README.md is required in the plugin folder")
    if len(readme.read_text(encoding="utf-8").split()) < MIN_README_WORDS:
        fail(f"README.md must contain at least {MIN_README_WORDS} words")
    if not (ROOT / "LICENSE").is_file():
        fail("LICENSE is required in the plugin folder")


def validate_bundle_limits() -> None:
    files = []
    for path in ROOT.rglob("*"):
        if path.is_symlink():
            fail(f"symlinks are not allowed: {path.relative_to(ROOT)}")
        if path.is_file():
            files.append(path)
    if len(files) > MAX_FILES:
        fail(f"plugin has {len(files)} files; the directory limit is {MAX_FILES}")
    for path in files:
        relative = path.relative_to(ROOT)
        if path.name in BLOCKED_NAMES or path.suffix.lower() in BLOCKED_SUFFIXES:
            fail(f"file type is not allowed in the plugin bundle: {relative}")
        if path.suffix.lower() not in IMAGE_SUFFIXES and path.stat().st_size > MAX_FILE_BYTES:
            fail(f"{relative} exceeds {MAX_FILE_BYTES} bytes")


def validate_with_claude_cli() -> None:
    claude = shutil.which("claude")
    if claude is None:
        print("note: claude CLI not found; skipped `claude plugin validate --strict`")
        return
    result = subprocess.run(
        [claude, "plugin", "validate", "--strict", str(ROOT)],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        fail(f"claude plugin validate --strict failed:\n{result.stdout}{result.stderr}")


def main() -> int:
    try:
        validate_manifest()
        validate_mcp()
        validate_skills()
        validate_docs()
        validate_bundle_limits()
        validate_with_claude_cli()
    except ValidationError as exc:
        print(f"Claude plugin validation failed: {exc}", file=sys.stderr)
        return 1
    print("Claude plugin validation passed")
    return 0


if __name__ == "__main__":
    sys.exit(main())
