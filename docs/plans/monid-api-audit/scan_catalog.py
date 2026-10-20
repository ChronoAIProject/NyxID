#!/usr/bin/env python3
"""Audit compiled Monid definitions and a public hosted-catalog snapshot offline.

This generates research artifacts. It does not activate services, load secrets,
execute hook functions, or make provider calls. Run fetch_public_catalog.py first
for a hosted snapshot, and Monid's compiler for the source bundle.
"""
import argparse
import collections
import copy
import csv
import hashlib
import json
import pathlib
import re
import tarfile
import urllib.parse

MONID_COMMIT = "c57aa3d4b2036f518cbc7d3879a097ac1c8af4ea"
WORKFLOW_KEYS = {"type", "enum", "properties", "required", "items", "additionalProperties",
                 "title", "description", "default", "example", "examples", "deprecated"}
SOURCE_PROVIDER_ALIASES = {"contextdev": "context.dev", "magic-hour": "magichour"}


def write_json(path, value):
    path.write_text(json.dumps(value, indent=2, ensure_ascii=False) + "\n")


def write_csv(path, rows):
    with path.open("w", newline="") as stream:
        writer = csv.DictWriter(stream, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


def schema_nodes(schema):
    if not isinstance(schema, dict):
        return
    yield schema
    for key in ("properties", "patternProperties", "$defs", "definitions", "dependentSchemas"):
        for child in schema.get(key, {}).values():
            yield from schema_nodes(child)
    for key in ("items", "additionalProperties", "not", "if", "then", "else", "contains", "propertyNames"):
        child = schema.get(key)
        if isinstance(child, list):
            for item in child:
                yield from schema_nodes(item)
        else:
            yield from schema_nodes(child)
    for key in ("oneOf", "anyOf", "allOf", "prefixItems"):
        for child in schema.get(key, []):
            yield from schema_nodes(child)


def auth_mapping(doc, functions):
    ref = doc["auth"]["inject"]["$fn"]
    fn = functions[ref["key"]]
    provenance, source = fn["provenance"], fn["src"]
    if provenance == "presets#auth.bearer":
        return "bearer", "Authorization", "apiKey"
    if provenance == "presets#auth.header":
        return "header", ref["args"][0], "apiKey"
    if provenance == "connectors/dataforseo/provider.ts#auth.inject":
        return "basic", "Authorization", "login+password"
    if "data.params.workApiKey" in source and "token:" in source:
        return "header", "token", "workApiKey"
    if "data.params.personalApiKey" in source and "token:" in source:
        return "header", "token", "personalApiKey"
    if source == "({data})=>data.request":
        return "no_injection", "", ""
    if 'Authorization:"Token "+data.params.apiKey' in source:
        return "header_with_token_prefix", "Authorization", "apiKey"
    return "custom_review", "", ""


def json_schema(schema):
    schema = copy.deepcopy(schema)
    for node in schema_nodes(schema):
        node.pop("$schema", None)
    return schema


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--bundle", type=pathlib.Path, required=True)
    parser.add_argument("--hosted", type=pathlib.Path, required=True)
    parser.add_argument("--release", type=pathlib.Path, required=True)
    parser.add_argument("--output", type=pathlib.Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    bundle = json.loads(args.bundle.read_text())
    hosted = json.loads(args.hosted.read_text())
    functions, docs = bundle["fnTable"], list(bundle["endpoints"].values())
    hosted_map = {(item["provider"], item["endpoint"]): item for item in hosted["items"]}
    details = {(d["provider"], d["endpoint"]): d for d in hosted["details"]}
    with tarfile.open(args.release) as archive:
        latest = json.load(archive.extractfile("./latest.json"))
        release = json.load(archive.extractfile("./" + latest["manifestKey"]))
    release_ids = {row["id"] for row in release["docs"] if row["kind"] == "endpoint"}
    rows, proof_groups = [], collections.defaultdict(list)
    for doc in docs:
        method, url = doc["request"]["method"], doc["request"]["url"]
        parsed = urllib.parse.urlsplit(url)
        origin = parsed.scheme + "://" + parsed.netloc
        auth, auth_key, credential_field = auth_mapping(doc, functions)
        schemas = doc["input"]["schema"]
        nodes = [node for schema in schemas.values() for node in schema_nodes(schema)]
        keywords = sorted({key for node in nodes for key in node if key not in WORKFLOW_KEYS})
        unions = any(isinstance(node.get("type"), list) for node in nodes)
        complex_query = [name for name, schema in schemas.get("queryParams", {}).get("properties", {}).items()
                         if any(node.get("type") in ("array", "object") or isinstance(node.get("type"), list)
                                for node in schema_nodes(schema))]
        resource = bool(doc.get("resources"))
        start = bool(doc.get("lifecycle"))
        poll = "poll" in doc.get("lifecycle", {})
        transform = "toRequest" in doc["input"]
        if resource:
            category = "resource_ownership_adapter"
        elif poll:
            category = "polling_lifecycle_adapter"
        elif start:
            category = "start_hook_adapter"
        elif transform:
            category = "request_transform_adapter"
        else:
            category = "plain_http_request_candidate"
        plain = category == "plain_http_request_candidate"
        response_hook = "fromResponse" in doc["output"]
        error_hook = "fromError" in doc["output"]
        consolidate = "consolidate" in doc["usage"]
        hook_free = plain and not response_hook and not error_hook and not consolidate
        proof = hook_free and not complex_query and auth in ("bearer", "header", "basic")
        identity = (doc["provider"], doc["endpoint"])
        alias_identity = (SOURCE_PROVIDER_ALIASES.get(doc["provider"], doc["provider"]), doc["endpoint"])
        matching_hosted = hosted_map.get(identity) or hosted_map.get(alias_identity)
        source_path = next((functions[ref["$fn"]["key"]]["provenance"].split("#")[0]
                            for ref in (doc["usage"]["estimate"], doc["usage"]["evidence"])
                            if functions[ref["$fn"]["key"]]["provenance"].startswith("connectors/")),
                           "connectors/" + doc["provider"] + "/provider.ts")
        rows.append({
            "id": doc["id"], "provider": doc["provider"], "endpoint": doc["endpoint"],
            "display_name": doc["meta"]["displayName"], "method": method, "upstream_url": url,
            "origin": origin, "auth": auth, "auth_key": auth_key, "credential_field": credential_field,
            "credential_schema_fields": "|".join(doc["auth"]["credentials"].get("properties", {})),
            "input_locations": "|".join(schemas), "class": category,
            "input_transform": transform, "lifecycle_start": start, "lifecycle_poll": poll,
            "lifecycle_stop": "stop" in doc.get("lifecycle", {}), "owned_resource": resource,
            "response_transform": response_hook, "error_transform": error_hook,
            "usage_consolidate": consolidate, "usage_kind": doc["usage"]["model"]["kind"],
            "complex_query_fields": "|".join(complex_query), "workflow_extra_keywords": "|".join(keywords),
            "workflow_type_union": unions, "hook_free_http_shape": hook_free,
            "conservative_conversion_example": proof, "present_in_release": doc["id"] in release_ids,
            "hosted_exact_match": identity in hosted_map,
            "hosted_match_with_declared_provider_alias": bool(matching_hosted),
            "categories": "|".join(doc["meta"].get("categories", [])),
            "docs_url": doc["meta"].get("docsUrl", ""),
            "source_url": "https://github.com/monid-ai/monid/blob/" + MONID_COMMIT + "/" + source_path,
        })
        if proof:
            proof_groups[(doc["provider"], origin, auth, auth_key, credential_field)].append(doc)
    write_csv(args.output / "source-endpoints.csv", rows)

    resources = collections.defaultdict(list)
    for resource in hosted["resources"]["items"]:
        for endpoint in resource.get("endpoints", []):
            resources[(resource["provider"], endpoint["endpoint"])].append(resource["resource"])
    hosted_rows = []
    for item in sorted(hosted["items"], key=lambda item: (item["provider"], item["endpoint"])):
        identity = (item["provider"], item["endpoint"])
        record = details.get(identity, {})
        detail = record.get("detail", {})
        hosted_rows.append({
            "provider": item["provider"], "endpoint": item["endpoint"],
            "display_name": item["displayName"], "method": detail.get("method", ""),
            "price_json": json.dumps(item.get("price", {}), separators=(",", ":")),
            "tags": "|".join(item.get("tags", [])), "categories": "|".join(item.get("categories", [])),
            "owned_resources": "|".join(resources[identity]),
            "x402_networks": "|".join(item.get("supportedX402Networks", [])),
            "detail_error": record.get("error", ""),
            "public_input_schema_present": bool(detail.get("input") or detail.get("inputSchema")),
            "docs_url": detail.get("docUrl", ""),
            "public_detail_url": "https://api.monid.ai/public/v1/providers/" +
                                 urllib.parse.quote(item["provider"], safe="") + "/endpoints" +
                                 urllib.parse.quote(item["endpoint"], safe="/"),
        })
    write_csv(args.output / "hosted-endpoints.csv", hosted_rows)
    provider_rows = []
    source_slugs = {SOURCE_PROVIDER_ALIASES.get(provider, provider) for provider in bundle["providers"]}
    for provider in sorted({item["provider"] for item in hosted["items"]} | source_slugs):
        source = [row for row in rows if SOURCE_PROVIDER_ALIASES.get(row["provider"], row["provider"]) == provider]
        live = [row for row in hosted_rows if row["provider"] == provider]
        provider_rows.append({
            "provider": provider, "source_provider_slugs": "|".join(sorted({row["provider"] for row in source})),
            "hosted_endpoints": len(live), "source_endpoints": len(source),
            "http_request_candidates": sum(row["class"] == "plain_http_request_candidate" for row in source),
            "request_transforms": sum(row["input_transform"] for row in source),
            "start_hooks": sum(row["lifecycle_start"] for row in source),
            "poll_hooks": sum(row["lifecycle_poll"] for row in source),
            "resource_operations_in_source": sum(row["owned_resource"] for row in source),
            "free_source_models": sum(row["usage_kind"] == "FREE" for row in source),
            "source_auth": "|".join(sorted({row["auth"] for row in source})),
        })
    write_csv(args.output / "providers.csv", provider_rows)

    proof_dir = args.output / "openapi-examples"
    proof_dir.mkdir(exist_ok=True)
    manifest = []
    for index, (group, group_docs) in enumerate(sorted(proof_groups.items())):
        provider, origin, auth, auth_key, credential_field = group
        scheme = {"type": "http", "scheme": auth} if auth in ("bearer", "basic") else {
            "type": "apiKey", "in": "header", "name": auth_key}
        spec = {"openapi": "3.1.0", "info": {"title": provider + " research import example", "version": "0.0.0"},
                "servers": [{"url": origin}], "paths": {}, "components": {"securitySchemes": {"providerCredential": scheme}},
                "security": [{"providerCredential": []}]}
        for doc in group_docs:
            path = urllib.parse.urlsplit(doc["request"]["url"]).path or "/"
            operation = {"operationId": re.sub(r"[^a-zA-Z0-9_]", "_", doc["id"]),
                         "summary": doc["meta"]["displayName"], "description": doc["meta"].get("description", ""),
                         "x-monid-source-id": doc["id"], "x-nyxid-audit-status": "candidate_requires_live_validation",
                         "responses": {"200": {"description": "Upstream response", "content": {
                             "application/json": {"schema": json_schema(doc["output"].get("schema", {}))}}}}}
            parameters = []
            for section, location in (("pathParams", "path"), ("queryParams", "query")):
                schema = doc["input"]["schema"].get(section, {})
                for name, field in schema.get("properties", {}).items():
                    parameters.append({"name": name, "in": location,
                                       "required": location == "path" or name in schema.get("required", []),
                                       "schema": json_schema(field)})
            if parameters:
                operation["parameters"] = parameters
            body = doc["input"]["schema"].get("body")
            if body is not None:
                operation["requestBody"] = {"required": bool(body.get("required")),
                                            "content": {"application/json": {"schema": json_schema(body)}}}
            method = doc["request"]["method"].lower()
            if method in spec["paths"].setdefault(path, {}):
                raise RuntimeError("OpenAPI method/path collision: " + doc["id"])
            spec["paths"][path][method] = operation
        file = provider + "-" + str(index + 1) + ".json"
        write_json(proof_dir / file, spec)
        manifest.append({"file": file, "provider": provider, "base_url": origin,
                         "auth_method": auth, "auth_key_name": auth_key,
                         "credential_shape_field": credential_field,
                         "default_request_headers": group_docs[0]["request"].get("headers", {}),
                         "operation_ids": [doc["id"] for doc in group_docs], "activate": False})
    write_json(proof_dir / "candidate-manifest.json", manifest)
    summary = {
        "source_commit": MONID_COMMIT,
        "hosted_snapshot_sha256": hashlib.sha256(args.hosted.read_bytes()).hexdigest(),
        "source_bundle_sha256": hashlib.sha256(args.bundle.read_bytes()).hexdigest(),
        "source_providers": len(bundle["providers"]), "source_endpoints": len(docs),
        "source_fn_table_entries": len(functions), "source_origins": len({row["origin"] for row in rows}),
        "source_methods": dict(collections.Counter(row["method"] for row in rows)),
        "source_classes": dict(collections.Counter(row["class"] for row in rows)),
        "source_flag_counts": {flag: sum(bool(row[flag]) for row in rows) for flag in (
            "input_transform", "lifecycle_start", "lifecycle_poll", "owned_resource", "response_transform",
            "error_transform", "usage_consolidate", "hook_free_http_shape", "conservative_conversion_example")},
        "source_complex_query_endpoints": sum(bool(row["complex_query_fields"]) for row in rows),
        "source_workflow_extra_keyword_endpoints": sum(bool(row["workflow_extra_keywords"]) for row in rows),
        "source_workflow_union_endpoints": sum(row["workflow_type_union"] for row in rows),
        "source_schema_combinator_endpoints": sum(any(
            any(key in node for key in ("anyOf", "oneOf", "allOf", "$ref"))
            for schema in doc["input"]["schema"].values() for node in schema_nodes(schema)) for doc in docs),
        "source_usage_models": dict(collections.Counter(row["usage_kind"] for row in rows)),
        "release_tag": release["catalogVersion"], "release_endpoints": len(release_ids),
        "release_sha256": hashlib.sha256(args.release.read_bytes()).hexdigest(),
        "source_not_in_release": sorted(set(bundle["endpoints"]) - release_ids),
        "release_not_in_current_source": sorted(release_ids - set(bundle["endpoints"])),
        "hosted_reported_stats": hosted["stats"], "hosted_final_stats": hosted["final_stats"],
        "hosted_enumerated_endpoints": len(hosted_rows), "hosted_enumerated_providers": len({row["provider"] for row in hosted_rows}),
        "hosted_detail_errors": sum(bool(row["detail_error"]) for row in hosted_rows),
        "hosted_public_input_schemas": sum(row["public_input_schema_present"] for row in hosted_rows),
        "hosted_resources": len(hosted["resources"]["items"]),
        "hosted_resource_linked_endpoints": sum(bool(row["owned_resources"]) for row in hosted_rows),
        "hosted_methods": dict(collections.Counter(row["method"] for row in hosted_rows)),
        "hosted_zero_price_per_call": sum(item.get("price", {}).get("type") == "PER_CALL"
            and item["price"].get("amount", {}).get("value") == 0 for item in hosted["items"]),
        "hosted_price_types": dict(collections.Counter(item.get("price", {}).get("type", "missing") for item in hosted["items"])),
        "source_hosted_exact_matches": sum(row["hosted_exact_match"] for row in rows),
        "source_hosted_matches_with_provider_alias": sum(row["hosted_match_with_declared_provider_alias"] for row in rows),
        "provider_aliases": SOURCE_PROVIDER_ALIASES,
        "hosted_snapshot_started_at": hosted["started_at"], "hosted_snapshot_finished_at": hosted["finished_at"],
        "conversion_spec_files": len(manifest),
    }
    write_json(args.output / "summary.json", summary)
    print(json.dumps({key: value for key, value in summary.items()
                      if key not in ("source_not_in_release", "release_not_in_current_source")}, indent=2))


if __name__ == "__main__":
    main()
