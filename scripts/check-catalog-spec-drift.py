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

Requires: requests, pyyaml (see .github/workflows/catalog-spec-drift.yml).
"""

import json
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

OVERLAY_DIR = "backend/specs/catalog"
HTTP_METHODS = ("get", "post", "put", "patch", "delete")


def fetch(url: str, is_yaml: bool):
    request = urllib.request.Request(url, headers={"User-Agent": "nyxid-spec-drift-check"})
    with urllib.request.urlopen(request, timeout=60) as response:
        body = response.read()
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


def main() -> int:
    drifted = False
    for overlay_name, (url, is_yaml, strip_prefix) in sorted(OFFICIAL_SPECS.items()):
        overlay = json.load(open(f"{OVERLAY_DIR}/{overlay_name}"))
        overlay_ops = operations(overlay)
        try:
            upstream = fetch(url, is_yaml)
        except Exception as error:  # noqa: BLE001 - report and continue
            print(f"WARN {overlay_name}: failed to fetch official spec {url}: {error}")
            continue
        upstream_ops = operations(upstream, strip_prefix)

        missing = sorted(overlay_ops - upstream_ops)
        if missing:
            drifted = True
            print(f"DRIFT {overlay_name} (vs {url}):")
            for method, path in missing:
                print(f"  {method} {path} no longer exists upstream")
        else:
            print(f"OK {overlay_name}: {len(overlay_ops)} operations all present upstream")

    if drifted:
        print("\nOne or more curated overlays drifted from the official spec.")
        print("Update the overlay in backend/specs/catalog/ (or this mapping) accordingly.")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
