#!/usr/bin/env python3
"""Validate the NyxID Cursor plugin using only the Python standard library."""

from __future__ import annotations

import json
import re
import sys
from pathlib import Path


VAR_RE = re.compile(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}")
NAME_RE = re.compile(r"^[a-z0-9]+(?:-[a-z0-9]+)*$")
SEMVER_RE = re.compile(r"^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$")


class ValidationError(Exception):
    """A user-facing validation failure."""


def fail(message: str) -> None:
    raise ValidationError(message)


def load_json(path: Path) -> object:
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        fail(f"{path}: invalid JSON ({exc})")


def require_mapping(value: object, label: str) -> dict:
    if not isinstance(value, dict):
        fail(f"{label} must be a JSON object")
    return value


def validate_relative_path(root: Path, raw: object, label: str) -> None:
    if not isinstance(raw, str) or not raw:
        fail(f"{label} must be a non-empty relative path")
    path = Path(raw)
    if path.is_absolute() or ".." in path.parts:
        fail(f"{label} must be relative and must not contain '..': {raw!r}")
    target = root / path
    if not target.exists():
        fail(f"{label} does not exist: {raw}")


def validate_manifest(root: Path) -> tuple[dict, dict]:
    manifest_path = root / ".cursor-plugin" / "plugin.json"
    if not manifest_path.is_file():
        fail(f"missing required manifest: {manifest_path}")
    manifest = require_mapping(load_json(manifest_path), "plugin.json")

    name = manifest.get("name")
    if not isinstance(name, str) or not NAME_RE.fullmatch(name):
        fail("plugin name must be lowercase kebab-case")
    if not isinstance(manifest.get("description"), str) or not manifest["description"].strip():
        fail("plugin description is required")
    if "version" in manifest and (
        not isinstance(manifest["version"], str) or not SEMVER_RE.fullmatch(manifest["version"])
    ):
        fail("plugin version must be semantic versioning")
    if not isinstance(manifest.get("license"), str) or not manifest["license"].strip():
        fail("plugin license is required")

    validate_relative_path(root, manifest.get("logo"), "logo")
    if not (root / "README.md").is_file():
        fail("README.md is required")

    for field in ("rules", "agents", "skills", "commands", "hooks"):
        value = manifest.get(field, [])
        if isinstance(value, str):
            value = [value]
        if not isinstance(value, list):
            fail(f"manifest field {field} must be a path or array of paths")
        for index, item in enumerate(value):
            validate_relative_path(root, item, f"{field}[{index}]")
        manifest[field] = value

    mcp_ref = manifest.get("mcpServers")
    if isinstance(mcp_ref, str):
        validate_relative_path(root, mcp_ref, "mcpServers")
    elif mcp_ref is not None and not isinstance(mcp_ref, (dict, list)):
        fail("mcpServers must be a path, object, or array")
    if not isinstance(mcp_ref, str):
        fail("this plugin must declare mcpServers as the mcp.json path")

    variables = require_mapping(manifest.get("variables", {}), "variables")
    properties = variables.get("properties", {})
    if not isinstance(properties, dict):
        fail("variables.properties must be an object")
    for variable_name, schema in properties.items():
        if not isinstance(variable_name, str) or not isinstance(schema, dict):
            fail("variables.properties entries must be named schema objects")
        if not isinstance(schema.get("type"), str):
            fail(f"variable {variable_name} must declare a type")
    return manifest, properties


def frontmatter(path: Path) -> dict:
    try:
        lines = path.read_text(encoding="utf-8").splitlines()
    except OSError as exc:
        fail(f"cannot read {path}: {exc}")
    if not lines or lines[0].strip() != "---":
        fail(f"{path}: missing YAML frontmatter")
    try:
        end = next(index for index, line in enumerate(lines[1:], 1) if line.strip() == "---")
    except StopIteration:
        fail(f"{path}: unterminated YAML frontmatter")
    result: dict[str, object] = {}
    for line in lines[1:end]:
        if not line.strip() or line.lstrip().startswith("#"):
            continue
        if ":" not in line or line[:1].isspace():
            fail(f"{path}: unsupported frontmatter line: {line!r}")
        key, value = line.split(":", 1)
        key = key.strip()
        value = value.strip()
        if value in {"true", "false"}:
            result[key] = value == "true"
        elif (value.startswith("\"") and value.endswith("\"")) or (
            value.startswith("'") and value.endswith("'")
        ):
            result[key] = value[1:-1]
        else:
            result[key] = value
    return result


def validate_components(root: Path, manifest: dict) -> None:
    for relative in manifest.get("skills", []):
        path = root / relative
        skill_files = [path / "SKILL.md"] if path.is_dir() else [path]
        for skill in skill_files:
            fields = frontmatter(skill)
            for field in ("name", "description"):
                if not isinstance(fields.get(field), str) or not fields[field].strip():
                    fail(f"{skill}: skill frontmatter requires {field}")

    for relative in manifest.get("rules", []):
        fields = frontmatter(root / relative)
        if not isinstance(fields.get("description"), str) or not fields["description"].strip():
            fail(f"{root / relative}: rule frontmatter requires description")
        if not isinstance(fields.get("alwaysApply"), bool):
            fail(f"{root / relative}: rule frontmatter requires boolean alwaysApply")
        if "globs" not in fields or not str(fields["globs"]).strip():
            fail(f"{root / relative}: rule frontmatter requires globs")

    for relative in manifest.get("commands", []):
        fields = frontmatter(root / relative)
        for field in ("name", "description"):
            if not isinstance(fields.get(field), str) or not fields[field].strip():
                fail(f"{root / relative}: command frontmatter requires {field}")


def validate_mcp(root: Path, properties: dict) -> None:
    mcp_path = root / "mcp.json"
    mcp_text = mcp_path.read_text(encoding="utf-8")
    mcp = require_mapping(load_json(mcp_path), "mcp.json")
    servers = require_mapping(mcp.get("mcpServers"), "mcpServers")
    if "nyxid" not in servers:
        fail("mcp.json must declare an nyxid server")
    server = require_mapping(servers["nyxid"], "mcpServers.nyxid")
    if not isinstance(server.get("url"), str) or not server["url"].strip():
        fail("mcpServers.nyxid.url is required")
    used = set(VAR_RE.findall(mcp_text))
    declared = set(properties)
    missing = sorted(used - declared)
    if missing:
        fail(f"mcp.json uses undeclared variables: {', '.join(missing)}")


def main() -> int:
    default_root = Path(__file__).resolve().parents[1] / "integrations" / "cursor-plugin"
    root = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else default_root
    try:
        manifest, properties = validate_manifest(root)
        validate_components(root, manifest)
        validate_mcp(root, properties)
    except ValidationError as exc:
        print(f"Cursor plugin validation failed: {exc}", file=sys.stderr)
        return 1
    print(f"Cursor plugin validation passed: {root}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
