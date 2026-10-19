#!/usr/bin/env python3
"""Read-only structural audit of all 38 Google operations; not execution evidence.

Use --snapshot-dir to audit saved public Discovery responses without network access.
This command never calls account APIs or modifies the source overlays.
"""

import argparse
import hashlib
import json
from pathlib import Path
import re
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
HTTP_METHODS = {"get", "post", "put", "patch", "delete", "head", "options"}
DISCOVERY_URLS = {
    "drive": "https://www.googleapis.com/discovery/v1/apis/drive/v3/rest",
    "docs": "https://docs.googleapis.com/$discovery/rest?version=v1",
    "sheets": "https://sheets.googleapis.com/$discovery/rest?version=v4",
    "slides": "https://slides.googleapis.com/$discovery/rest?version=v1",
    "calendar": "https://www.googleapis.com/discovery/v1/apis/calendar/v3/rest",
    "gmail": "https://gmail.googleapis.com/$discovery/rest?version=v1",
}


def discovery_methods(resource):
    yield from resource.get("methods", {}).values()
    for child in resource.get("resources", {}).values():
        yield from discovery_methods(child)


def route_matches(template, path):
    parts = re.split(r"(\{[^}]+\})", template.replace("{+", "{"))
    pattern = "".join(r"[^/]+" if part.startswith("{") else re.escape(part) for part in parts)
    return re.fullmatch(pattern, path) is not None


def method_definitions(discovery):
    base = "/" + discovery["servicePath"].strip("/") if discovery.get("servicePath") else ""
    for method in discovery_methods(discovery):
        yield method["httpMethod"], base + "/" + method["path"].lstrip("/"), method, False
        upload = method.get("mediaUpload", {}).get("protocols", {}).get("simple", {}).get("path")
        if upload:
            yield method["httpMethod"], upload, method, True


def audit_operation(product, path, verb, operation, discovery, definitions):
    matches = [
        definition for definition in definitions
        if definition[0] == verb.upper() and route_matches(definition[1], path)
    ]
    row = {
        "product": product,
        "operation": operation["operationId"],
        "method": verb.upper(),
        "path": path,
        "discovery_matches": [definition[2]["id"] for definition in matches],
        "issues": [],
    }
    issues = row["issues"]
    if len(matches) != 1:
        issues.append("expected exactly one discovery match")
        return row

    _, _, method, is_upload = matches[0]
    if is_upload and method.get("mediaUpload", {}).get("protocols", {}).get("simple", {}).get("multipart") is not True:
        issues.append("simple multipart uploads not supported by Discovery")

    parameters = discovery.get("parameters", {}) | method.get("parameters", {})
    published = {parameter["name"]: parameter for parameter in operation.get("parameters", [])}
    for name, parameter in published.items():
        definition = parameters.get(name)
        if definition is None:
            # Discovery describes upload mode in mediaUpload, not parameters.
            if name == "uploadType" and is_upload:
                if not set(parameter.get("schema", {}).get("enum", [])) <= {"media", "multipart"}:
                    issues.append("unsupported simple uploadType")
                continue
            issues.append("unknown parameter " + name)
            continue
        if parameter["in"] != definition["location"]:
            issues.append("parameter location " + name)
        schema = parameter.get("schema", {})
        if definition.get("type") and schema.get("type") != definition["type"]:
            issues.append("parameter type " + name)
        if definition.get("enum") and set(schema.get("enum", [])) - set(definition["enum"]):
            issues.append("parameter enum " + name)

    for name, definition in method.get("parameters", {}).items():
        if definition.get("required") and name not in published:
            if "{" + name + "}" in path:
                issues.append("required parameter not published " + name)
            elif definition.get("location") != "path":
                issues.append("required query parameter not published " + name)

    request_ref = method.get("request", {}).get("$ref")
    content = operation.get("requestBody", {}).get("content", {})
    body = content.get("application/json", {}).get("schema", {})
    if request_ref and body.get("properties"):
        properties = discovery["schemas"][request_ref].get("properties", {})
        for name, schema in body["properties"].items():
            if name not in properties:
                issues.append("unknown body property " + name)
            elif schema.get("type") and properties[name].get("type") and schema["type"] != properties[name]["type"]:
                issues.append("body property type " + name)

    row["body_content_types"] = list(content)
    row["scopes"] = method.get("scopes", [])
    return row


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--snapshot-dir", type=Path, help="Directory containing nyxid-google-discovery-PRODUCT.json")
    parser.add_argument("--output", type=Path, help="Write the audit evidence JSON")
    args = parser.parse_args()
    revisions = {}
    rows = []
    for product, url in DISCOVERY_URLS.items():
        if args.snapshot_dir:
            raw = (args.snapshot_dir / f"nyxid-google-discovery-{product}.json").read_bytes()
        else:
            with urllib.request.urlopen(url, timeout=45) as response:
                raw = response.read()
        discovery = json.loads(raw)
        revisions[product] = {
            "revision": discovery["revision"],
            "discovery_url": url,
            "snapshot_sha256": hashlib.sha256(raw).hexdigest(),
        }
        overlay = json.loads((ROOT / f"backend/specs/catalog/google-{product}.openapi.json").read_text())
        definitions = list(method_definitions(discovery))
        for path, item in overlay["paths"].items():
            for verb, operation in item.items():
                if verb in HTTP_METHODS:
                    rows.append(audit_operation(product, path, verb, operation, discovery, definitions))

    evidence = {
        "discovery": revisions,
        "operation_count": len(rows),
        "operations": rows,
        "limits": "Structural metadata only; does not prove examples, MIME serialization, permissions, API enablement or execution.",
    }
    if args.output:
        args.output.write_text(json.dumps(evidence, indent=2) + "\n")
    for product, revision in revisions.items():
        print(product, "Discovery revision", revision["revision"])
    for row in rows:
        print(row["product"], row["operation"], row["method"], row["path"], row["issues"] or "OK")
    failures = sum(bool(row["issues"]) for row in rows)
    print("TOTAL", len(rows), "WITH_ISSUES", failures)
    print("LIMITS:", evidence["limits"])
    if len(rows) != 38 or failures:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
