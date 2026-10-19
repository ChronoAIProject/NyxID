#!/usr/bin/env python3
"""Verify hosted Docs/Sheets/Slides operations against Google's live discovery APIs.

Read-only: fetches public metadata and checks the committed scope evidence. It never
calls document operations, changes overlays, or requests account credentials.
"""

import json
from pathlib import Path
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
DRIVE = "https://www.googleapis.com/auth/drive"
HTTP_METHODS = {"get", "post", "put", "patch", "delete", "head", "options"}


def discovery_methods(resource):
    yield from resource.get("methods", {}).values()
    for child in resource.get("resources", {}).values():
        yield from discovery_methods(child)


def main():
    evidence = json.loads(
        (ROOT / "backend/specs/fixtures/google-editor-scope-acceptance.json").read_text()
    )
    for slug, proof in evidence.items():
        with urllib.request.urlopen(proof["discovery_url"], timeout=45) as response:
            discovery = json.load(response)
        assert discovery["rootUrl"].rstrip("/") == proof["origin"], slug
        methods = {
            (method["httpMethod"], "/" + method["path"].replace("{+", "{")): method
            for method in discovery_methods(discovery)
        }
        overlay = json.loads(
            (ROOT / f"backend/specs/catalog/{slug.removeprefix('api-')}.openapi.json").read_text()
        )
        published = set()
        for path, item in overlay["paths"].items():
            for verb, operation in item.items():
                if verb not in HTTP_METHODS:
                    continue
                operation_id = operation["operationId"]
                method = methods[(verb.upper(), path)]
                pinned = proof["operations"][operation_id]
                assert method["id"] == pinned["discovery_id"], operation_id
                assert (verb.upper(), path) == (pinned["method"], pinned["path"]), operation_id
                assert DRIVE in method["scopes"], f"{operation_id} no longer accepts Drive scope"
                assert set(method["scopes"]) == set(pinned["accepted_scopes"]), (
                    f"{operation_id}: accepted scopes changed; review the evidence"
                )
                for parameter in operation.get("parameters", []):
                    definition = method.get("parameters", {}).get(parameter["name"])
                    if definition is None:
                        definition = discovery.get("parameters", {}).get(parameter["name"])
                    assert definition is not None, f"{operation_id}: unknown parameter {parameter['name']}"
                    assert definition["location"] == parameter["in"], operation_id
                published.add(operation_id)
                print(f"{slug} {operation_id}: Drive accepted (discovery {discovery['revision']})")
        assert published == set(proof["operations"]), f"{slug}: evidence does not cover the overlay"


if __name__ == "__main__":
    main()
