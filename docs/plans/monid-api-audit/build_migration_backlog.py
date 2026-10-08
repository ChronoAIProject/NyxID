#!/usr/bin/env python3
"""Build a planning backlog from the audited inventories, without network calls."""

import argparse
import csv
import hashlib
import json
from collections import Counter, defaultdict
from pathlib import Path


SOURCE_TO_HOSTED = {"contextdev": "context.dev", "magic-hour": "magichour"}
WAVES = {
    "01-search-pilot": {"tinyfish", "firecrawl", "exa"},
    "02-source-data": {
        "ahrefs", "akta", "apollo", "contactout", "context.dev", "dataforseo",
        "fundable", "hunterio", "litescrape", "mrscraper", "octen", "opoint",
        "pdl", "search1api", "surf", "vaquill",
    },
    "03-data-jobs": {"apify", "clay", "cloro", "orbit", "ploid"},
    "04-generation": {
        "alibaba", "bytedance", "kling", "magichour", "minimax", "suzanne",
        "openai", "gemini", "elevenlabs",
    },
    "05-owned-services": {"saperly", "agentmail", "sfs", "smolmachine", "x402.browserbase.com"},
}
EXISTING_OVERLAP_CANDIDATES = {"firecrawl", "openai", "gemini", "elevenlabs"}


def read_csv(path):
    with path.open(newline="", encoding="utf-8") as handle:
        return list(csv.DictReader(handle))


def write_csv(path, rows):
    with path.open("w", newline="", encoding="utf-8") as handle:
        writer = csv.DictWriter(handle, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


def canonical_provider(provider):
    return SOURCE_TO_HOSTED.get(provider, provider)


def source_work(source):
    if source is None:
        return "native_contract_needed"
    return source["class"]


def blockers(source, hosted):
    items = ["native_wire_review", "account_access_review", "cost_capacity_review", "data_scope_review"]
    if source is None:
        items.insert(0, "execution_contract_missing")
    else:
        for field, label in [
            ("input_transform", "request_transform_review"),
            ("lifecycle_start", "start_hook_review"),
            ("lifecycle_poll", "job_ownership_needed"),
            ("owned_resource", "resource_ownership_needed"),
            ("response_transform", "response_contract_review"),
            ("error_transform", "error_mapping_review"),
            ("usage_consolidate", "upstream_usage_review"),
        ]:
            if source[field] == "True":
                items.append(label)
        if source["complex_query_fields"]:
            items.append("query_serialization_review")
    if hosted and hosted["owned_resources"] and "resource_ownership_needed" not in items:
        items.append("resource_ownership_needed")
    return "|".join(items)


def operation_row(provider, hosted, source, wave):
    endpoint = hosted["endpoint"] if hosted else source["endpoint"]
    return {
        "backlog_id": f"hosted:{provider}#{endpoint}" if hosted else f"source:{source['id']}",
        "coverage": "hosted_and_source" if hosted and source else "hosted_only" if hosted else "source_only",
        "provider": provider,
        "hosted_endpoint": hosted["endpoint"] if hosted else "",
        "source_id": source["id"] if source else "",
        "source_match": "exact" if hosted and source and hosted["provider"] == source["provider"] else "declared_provider_alias" if hosted and source else "none",
        "method": hosted["method"] if hosted else source["method"],
        "wave": wave,
        "contract_work": source_work(source),
        "source_origin": source["origin"] if source else "",
        "source_auth": source["auth"] if source else "unverified",
        "hosted_owned_resources": hosted["owned_resources"] if hosted else "",
        "source_owned_resource": source["owned_resource"] if source else "unverified",
        "source_usage_model": source["usage_kind"] if source else "",
        "hosted_advertised_price_json": hosted["price_json"] if hosted else "",
        "existing_nyxid_overlap": "provider_contract_candidate_only" if provider in EXISTING_OVERLAP_CANDIDATES else "not_assessed",
        "port_status": "planned",
        "account_setup": "manual_review_pending",
        "public_data_classification": "unreviewed",
        "publication": "draft",
        "blockers": blockers(source, hosted),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", type=Path, default=Path(__file__).resolve().parent)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    output = args.output or args.input
    output.mkdir(parents=True, exist_ok=True)
    providers = read_csv(args.input / "providers.csv")
    hosted = read_csv(args.input / "hosted-endpoints.csv")
    source = read_csv(args.input / "source-endpoints.csv")
    provider_ids = {row["provider"] for row in providers}
    assigned = [p for group in WAVES.values() for p in group]
    assert len(assigned) == len(set(assigned)), "Provider assigned to several waves"
    assert set(assigned) <= provider_ids, "Wave contains an unknown provider"
    waves = {p: wave for wave, group in WAVES.items() for p in group}
    waves.update({p: "06-hosted-contracts" for p in provider_ids - set(assigned)})
    source_by_identity = {(canonical_provider(r["provider"]), r["endpoint"]): r for r in source}
    assert len(source_by_identity) == len(source), "Alias merges distinct source identities"
    hosted_identities = {(r["provider"], r["endpoint"]) for r in hosted}
    assert len(hosted_identities) == len(hosted), "Duplicate hosted identity"
    assert provider_ids == {r["provider"] for r in hosted}, "Provider inventory differs"
    assert {canonical_provider(r["provider"]) for r in source} <= provider_ids
    operations = [operation_row(r["provider"], r, source_by_identity.get((r["provider"], r["endpoint"])), waves[r["provider"]]) for r in hosted]
    operations.extend(
        operation_row(canonical_provider(r["provider"]), None, r, waves[canonical_provider(r["provider"])])
        for r in source if (canonical_provider(r["provider"]), r["endpoint"]) not in hosted_identities
    )
    operations.sort(key=lambda r: (r["wave"], r["provider"], r["backlog_id"]))
    assert len({r["backlog_id"] for r in operations}) == len(operations)
    assert sum(bool(r["hosted_endpoint"]) for r in operations) == len(hosted)
    assert {r["source_id"] for r in operations if r["source_id"]} == {r["id"] for r in source}
    assert sum(bool(r["source_id"]) for r in operations) == len(source)
    by_provider = defaultdict(list)
    for row in operations:
        by_provider[row["provider"]].append(row)
    provider_rows = []
    for provider in sorted(providers, key=lambda r: (waves[r["provider"]], r["provider"])):
        name = provider["provider"]
        rows = by_provider[name]
        match = sum(r["coverage"] == "hosted_and_source" for r in rows)
        provider_rows.append({
            "provider": name,
            "source_provider_slugs": provider["source_provider_slugs"],
            "wave": waves[name],
            "hosted_tools": provider["hosted_endpoints"],
            "source_definitions": provider["source_endpoints"],
            "matched_definitions": match,
            "hosted_without_source": sum(r["coverage"] == "hosted_only" for r in rows),
            "source_without_hosted_match": sum(r["coverage"] == "source_only" for r in rows),
            "origins_in_source": "|".join(sorted({r["source_origin"] for r in rows if r["source_origin"]})),
            "source_auth": provider["source_auth"],
            "existing_nyxid_overlap": "provider_contract_candidate_only" if name in EXISTING_OVERLAP_CANDIDATES else "not_assessed",
            "account_setup": "manual_review_pending",
            "credential_verification": "not_performed",
            "contract_priority": "source_review_and_remaining_native_contracts" if int(provider["source_endpoints"]) else "native_contract_discovery",
            "ownership_work": "review_resource_mapping" if any(r["hosted_owned_resources"] or r["source_owned_resource"] == "True" for r in rows) else "review_account_and_job_scope",
            "publication": "draft",
        })
        assert int(provider["hosted_endpoints"]) == sum(bool(r["hosted_endpoint"]) for r in rows)
        assert int(provider["source_endpoints"]) == sum(bool(r["source_id"]) for r in rows)
    write_csv(output / "provider-migration-backlog.csv", provider_rows)
    write_csv(output / "tool-migration-backlog.csv", operations)
    coverage = dict(Counter(r["coverage"] for r in operations))
    assert (len(providers), len(hosted), len(source), coverage["hosted_and_source"], coverage["source_only"]) == (87, 2440, 699, 637, 62)
    summary = {
        "status": "planning_only_no_vendor_accounts_or_runtime_changes",
        "source_provider_aliases": SOURCE_TO_HOSTED,
        "provider_entries": len(providers),
        "hosted_identities_accounted": len(hosted),
        "source_identities_accounted": len(source),
        "backlog_rows": len(operations),
        "coverage": coverage,
        "waves": {
            wave: {
                "providers": len(group),
                "provider_ids": sorted(group),
                "hosted_tools": sum(bool(r["hosted_endpoint"]) for r in operations if r["wave"] == wave),
                "source_definitions": sum(bool(r["source_id"]) for r in operations if r["wave"] == wave),
            }
            for wave, group in sorted((wave, {p for p in provider_ids if waves[p] == wave}) for wave in set(waves.values()))
        },
        "input_sha256": {name: hashlib.sha256((args.input / name).read_bytes()).hexdigest() for name in ["providers.csv", "hosted-endpoints.csv", "source-endpoints.csv"]},
        "limitations": [
            "Hosted provider identities do not prove distinct vendor accounts or native API operators.",
            "Hosted prices describe Monid's offering; native upstream costs and NyxID user prices require separate review.",
            "Missing resource metadata does not prove public scope or absence of side effects.",
            "Provider waves prioritize investigation; each operation retains its own runtime and ownership dependencies.",
            "A source-only row may later reconcile to a hosted tool; these rows are not a claim of 2502 distinct production capabilities.",
            "Overlap candidates establish existing provider-level contracts, not deployed access or exact operation equivalence.",
        ],
    }
    (output / "migration-backlog-summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({k: summary[k] for k in ["provider_entries", "hosted_identities_accounted", "source_identities_accounted", "backlog_rows", "coverage", "waves"]}, indent=2))


if __name__ == "__main__":
    main()
