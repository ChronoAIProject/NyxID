#!/usr/bin/env python3
"""Render the Claude and Codex plugin packages from integrations/plugin-source/.

The packages share one version, MCP URL, set of listing links and one skill.
Edit integrations/plugin-source/ and run this script; never edit the generated
files in the package folders by hand.

  sync-plugins.py           write the generated files
  sync-plugins.py --check   fail if any generated file differs (used by CI)

Generated per store:
  claude: .claude-plugin/plugin.json, .mcp.json, skills/nyxid/SKILL.md
  codex:  plugin.json, mcp.json, skills/nyxid/SKILL.md

Generated marketplaces (what `claude|codex plugin marketplace add
ChronoAIProject/NyxID` reads):
  .claude-plugin/marketplace.json   `nyxid` (the Claude package) and
                                    `nyxid-cli` (the CLI skill bundle at the
                                    repository root, described by its own
                                    .claude-plugin/plugin.json)
  .agents/plugins/marketplace.json  `nyxid` (the Codex package)

README, LICENSE and assets stay hand-written in each package folder.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
SOURCE = REPO / "integrations" / "plugin-source"
PLACEHOLDER = re.compile(r"\{\{([a-z_]+)\}\}")


def dump(value: object) -> str:
    return json.dumps(value, indent=2, ensure_ascii=False) + "\n"


def render_skill(template: str, values: dict[str, str]) -> str:
    def substitute(match: re.Match[str]) -> str:
        key = match.group(1)
        if key not in values:
            raise SystemExit(f"skill template uses unknown placeholder {{{{{key}}}}}")
        return values[key]

    return PLACEHOLDER.sub(substitute, template)


def claude_files(src: dict, store: dict, skill: str) -> dict[str, str]:
    manifest = {
        "name": src["name"],
        "version": src["version"],
        "description": store["description"],
        "author": src["author"],
        "homepage": src["homepage"],
        "repository": src["repository"],
        "license": src["license"],
        "keywords": store["keywords"],
        "icon": store["icon"],
        "documentationUrl": store["documentation_url"],
        "supportUrl": src["support_url"],
        "privacyPolicyUrl": src["privacy_url"],
        "termsOfServiceUrl": src["terms_url"],
    }
    mcp = {"mcpServers": {src["name"]: {"type": "http", "url": src["mcp_url"]}}}
    return {
        ".claude-plugin/plugin.json": dump(manifest),
        ".mcp.json": dump(mcp),
        "skills/nyxid/SKILL.md": skill,
    }


def codex_files(src: dict, store: dict, skill: str) -> dict[str, str]:
    interface = dict(store["interface"])
    links = {
        "websiteURL": src["homepage"],
        "supportURL": src["support_url"],
        "privacyPolicyURL": src["privacy_url"],
        "termsOfServiceURL": src["terms_url"],
    }
    # Keep the portal's field order: identity, links, prompts, icons.
    ordered = {k: interface.pop(k) for k in list(interface) if k not in ("defaultPrompt", "composerIcon", "logo")}
    ordered.update(links)
    ordered.update(interface)
    manifest = {
        "$schema": "https://agent-plugins.org/schemas/1.0.0/plugin.schema.json",
        "name": src["name"],
        "version": src["version"],
        "description": store["description"],
        "author": src["author"],
        "homepage": src["homepage"],
        "repository": src["repository"],
        "license": src["license"],
        "keywords": store["keywords"],
        "extensions": {
            "com.openai": {
                "interface": ordered,
                "publication": {"release_notes": store["release_notes"]},
            }
        },
    }
    mcp = {
        "$schema": "https://agent-plugins.org/schemas/1.0.0/mcp.schema.json",
        "mcpServers": {src["name"]: {"type": "streamable-http", "url": src["mcp_url"]}},
    }
    return {
        "plugin.json": dump(manifest),
        "mcp.json": dump(mcp),
        "skills/nyxid/SKILL.md": skill,
    }


RENDERERS = {"claude": claude_files, "codex": codex_files}


def marketplace_files(src: dict) -> dict[Path, str]:
    claude = src["stores"]["claude"]
    codex = src["stores"]["codex"]
    cli = json.loads((REPO / ".claude-plugin" / "plugin.json").read_text())
    owner = {"name": src["author"]["name"]}
    claude_marketplace = {
        "name": src["name"],
        "description": "NyxID plugins: the hosted MCP connection and the CLI skill edition.",
        "owner": owner,
        "plugins": [
            {
                "name": src["name"],
                "description": claude["description"],
                "version": src["version"],
                "source": "./" + claude["package"],
                "author": owner,
                "homepage": src["homepage"],
            },
            {
                "name": cli["name"],
                "description": cli["description"],
                "version": cli["version"],
                "source": "./",
                "author": owner,
                "homepage": cli["homepage"],
            },
        ],
    }
    codex_marketplace = {
        "name": src["name"],
        "interface": {"displayName": codex["interface"]["displayName"]},
        "plugins": [
            {
                "name": src["name"],
                "source": {"source": "local", "path": "./" + codex["package"]},
                "policy": {"installation": "AVAILABLE", "authentication": "ON_INSTALL"},
                "category": "Coding",
            }
        ],
    }
    return {
        REPO / ".claude-plugin" / "marketplace.json": dump(claude_marketplace),
        REPO / ".agents" / "plugins" / "marketplace.json": dump(codex_marketplace),
    }


def generated() -> dict[Path, str]:
    src = json.loads((SOURCE / "plugin.json").read_text())
    template = (SOURCE / "skills" / "nyxid" / "SKILL.md").read_text()
    if set(src["stores"]) != set(RENDERERS):
        raise SystemExit(f"plugin-source stores must be exactly {sorted(RENDERERS)}")
    files: dict[Path, str] = {}
    for name, store in src["stores"].items():
        skill = render_skill(
            template,
            {
                "clients": store["clients"],
                "assistant": store["assistant"],
                "mcp_url": src["mcp_url"],
                "homepage": src["homepage"],
                "manage_access_url": src["manage_access_url"],
            },
        )
        package = REPO / store["package"]
        for relative, content in RENDERERS[name](src, store, skill).items():
            files[package / relative] = content
    files.update(marketplace_files(src))
    return files


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--check", action="store_true", help="fail on drift instead of writing")
    args = parser.parse_args()

    stale = []
    for path, content in generated().items():
        current = path.read_text() if path.exists() else None
        if current == content:
            continue
        if args.check:
            stale.append(path.relative_to(REPO))
        else:
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content)
            print(f"wrote {path.relative_to(REPO)}")

    if stale:
        print("Plugin packages are out of date with integrations/plugin-source/:", file=sys.stderr)
        for path in stale:
            print(f"  {path}", file=sys.stderr)
        print("Run: python3 scripts/sync-plugins.py", file=sys.stderr)
        return 1
    if args.check:
        print("Plugin packages match integrations/plugin-source/")
    return 0


if __name__ == "__main__":
    sys.exit(main())
