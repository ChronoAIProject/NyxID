#!/usr/bin/env python3
"""Check NyxID's curated catalog overlays against official upstream OpenAPI specs.

For every overlay in backend/specs/catalog/ that has a known official
upstream spec, verify each overlay operation (method + path) still exists
upstream. Exits non-zero when an operation has disappeared upstream so the
scheduled CI job turns red and a human updates the overlay.

Only providers that publish a machine-readable spec are checked; the rest
(Telegram, Lark/Feishu, Reddit, Spotify, Twitch, Facebook, Microsoft
Graph*, GitHub*) either publish nothing fetchable or something too large
to diff meaningfully, and are skipped by design.

Notion publishes a current spec. Its version-pinned legacy database query is
checked against the official legacy reference because it is absent from that
spec; VERSIONED_OPERATIONS documents the exact operation and version.

Requires: pyyaml (see .github/workflows/catalog-spec-drift.yml).
"""

import json
import argparse
import sys
import urllib.request

try:
    import yaml
except ImportError:  # pragma: no cover
    yaml = None

# overlay file -> (official spec URL, is_yaml, path prefix to strip from
# official paths so they align with overlay paths, which are relative to
# the seeded base_url)
OFFICIAL_SPECS = {
    "notion.openapi.json": (
        "https://developers.notion.com/openapi.json",
        False,
        "",
    ),
    "openai.openapi.json": (
        "https://raw.githubusercontent.com/openai/openai-openapi/master/openapi.yaml",
        True,
        "",
    ),
    "twitter.openapi.json": (
        "https://api.twitter.com/2/openapi.json",
        False,
        "/2",
    ),
    "discord.openapi.json": (
        "https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json",
        False,
        "",
    ),
    "discord-bot.openapi.json": (
        "https://raw.githubusercontent.com/discord/discord-api-spec/main/specs/openapi.json",
        False,
        "",
    ),
    "elevenlabs.openapi.json": (
        "https://api.elevenlabs.io/openapi.json",
        False,
        "",
    ),
    "twilio.openapi.json": (
        "https://raw.githubusercontent.com/twilio/twilio-oai/main/spec/json/twilio_api_v2010.json",
        False,
        "",
    ),
}

# Notion's 2025-09-03 API split databases from data sources. The overlay pins
# 2022-06-28, for which the old query is still documented and supported. Do not
# compare this one operation to the current-version spec or silently skip it:
# require both the overlay version and the live official legacy reference.
VERSIONED_OPERATIONS = {
    "notion.openapi.json": {
        ("POST", "/v1/databases/{database_id}/query"): (
            "2022-06-28-nyxid-overlay",
            "https://developers.notion.com/reference/post-database-query.md",
            ("versions up to and including `2022-06-28`", "notion.databases.query({"),
        ),
    },
}

OVERLAY_DIR = "backend/specs/catalog"
HTTP_METHODS = ("get", "post", "put", "patch", "delete")


def fetch_text(url: str) -> str:
    request = urllib.request.Request(url, headers={"User-Agent": "nyxid-spec-drift-check"})
    with urllib.request.urlopen(request, timeout=60) as response:
        return response.read().decode("utf-8")


def fetch(url: str, is_yaml: bool):
    body = fetch_text(url)
    if is_yaml:
        if yaml is None:
            raise RuntimeError("pyyaml is required for YAML upstream specs")
        return yaml.safe_load(body)
    return json.loads(body)


def operations(spec: dict, strip_prefix: str = "") -> set[tuple[str, str]]:
    ops = set()
    for path, item in (spec.get("paths") or {}).items():
        if strip_prefix and path.startswith(strip_prefix):
            path = path[len(strip_prefix) :] or "/"
        if not isinstance(item, dict):
            continue
        for method in HTTP_METHODS:
            if method in item:
                ops.add((method.upper(), path))
    return ops


def missing_operations(overlay_name: str, overlay: dict, upstream: dict, strip_prefix: str = "") -> set[tuple[str, str]]:
    overlay_ops = operations(overlay)
    missing = overlay_ops - operations(upstream, strip_prefix)
    for operation, (version, reference_url, markers) in VERSIONED_OPERATIONS.get(overlay_name, {}).items():
        if operation not in missing:
            continue
        if overlay.get("info", {}).get("version") != version:
            continue
        reference = fetch_text(reference_url)
        if not all(marker in reference for marker in markers):
            continue
        print(f"OK {overlay_name}: {operation[0]} {operation[1]} at {version} verified via {reference_url}")
        missing.remove(operation)
    return missing


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--overlay", choices=sorted(OFFICIAL_SPECS), help="Check one overlay")
    args = parser.parse_args()
    drifted = False
    for overlay_name, (url, is_yaml, strip_prefix) in sorted(OFFICIAL_SPECS.items()):
        if args.overlay and overlay_name != args.overlay:
            continue
        with open(f"{OVERLAY_DIR}/{overlay_name}") as overlay_file:
            overlay = json.load(overlay_file)
        overlay_ops = operations(overlay)
        try:
            upstream = fetch(url, is_yaml)
            missing = sorted(missing_operations(overlay_name, overlay, upstream, strip_prefix))
        except Exception as error:  # noqa: BLE001 - report and continue
            drifted = True
            print(f"ERROR {overlay_name}: upstream verification failed ({url}): {error}")
            continue
        if missing:
            drifted = True
            print(f"DRIFT {overlay_name} (vs {url}):")
            for method, path in missing:
                print(f"  {method} {path} no longer exists upstream")
        else:
            print(f"OK {overlay_name}: {len(overlay_ops)} operations verified against official upstream sources")

    if drifted:
        print("\nOne or more curated overlays drifted from the official spec.")
        print("Update the overlay in backend/specs/catalog/ (or this mapping) accordingly.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
