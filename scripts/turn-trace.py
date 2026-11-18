#!/usr/bin/env python3
"""Read metadata-only turn trace NDJSON exported from service stdout.

No network, credentials, database, or third-party Python packages are required.
Unknown fields are discarded. Invalid records are counted, never echoed.
"""
from __future__ import annotations

import argparse
from collections import OrderedDict
import datetime as dt
import json
import sys
import uuid
from pathlib import Path

TARGET = "nyxid::turn_trace"
MAX_LINE = 2048
MAX_RECORDS = 20_000
MAX_LINES = 1_000_000
MAX_BYTES = 128 * 1024 * 1024
SERVICES = {"nyxid", "nyxagent", "cma"}
STAGES = {"ingress", "admission", "execution", "engine", "first_output", "mcp", "proxy", "proxy_headers", "proxy_body", "node_headers", "capacity_wait", "channel", "channel_part", "replay", "gateway", "model", "tool", "platform"}
OUTCOMES = {"completed", "failed", "cancelled", "unknown", "streamed", "silent", "confirmed", "rejected", "partial"}
PHASES = {"start", "end", "link", "milestone"}


def identifier(value: object) -> str | None:
    if not isinstance(value, str) or len(value) not in (32, 36):
        return None
    try:
        parsed = uuid.UUID(value)
        return str(parsed) if parsed.version == 4 else None
    except ValueError:
        return None


def span(value: object) -> str | None:
    if isinstance(value, str) and len(value) == 16 and all(c in "0123456789abcdef" for c in value) and int(value, 16):
        return value
    return None


def clean_record(raw: object) -> dict | None:
    if not isinstance(raw, dict) or raw.get("target") != TARGET or (type(raw.get("schema_version")) is not int or raw["schema_version"] != 1):
        return None
    if any(not isinstance(raw.get(key), str) or raw[key] not in allowed for key, allowed in (("service", SERVICES), ("stage", STAGES), ("phase", PHASES))):
        return None
    turn = identifier(raw.get("turn_id"))
    if (turn is None and (raw["stage"] != "ingress" or raw.get("turn_id") is not None or raw.get("trace_id") is not None)) or identifier(raw.get("trace_id")) != turn:
        return None
    instance, request, current = identifier(raw.get("instance_id")), identifier(raw.get("request_id")), span(raw.get("span_id"))
    if not instance or not request or not current:
        return None
    try:
        stamp = dt.datetime.fromisoformat(raw["timestamp"].replace("Z", "+00:00"))
        if stamp.utcoffset() is None:
            return None
        stamp = stamp.astimezone(dt.timezone.utc).isoformat()
    except (KeyError, ValueError, TypeError, AttributeError, OverflowError):
        return None
    result = {"schema_version": 1, "target": TARGET, "service": raw["service"], "instance_id": instance,
              "timestamp": stamp, "turn_id": turn, "trace_id": turn.replace("-", "") if turn else None, "span_id": current,
              "request_id": request, "stage": raw["stage"], "phase": raw["phase"]}
    for key in ("ingress_request_id", "related_turn_id"):
        if raw.get(key) is not None:
            value = identifier(raw[key])
            if not value:
                return None
            result[key] = value
    if raw.get("parent_span_id") is not None:
        parent = span(raw["parent_span_id"])
        if not parent:
            return None
        result["parent_span_id"] = parent
    for key, maximum in (("duration_ms", 2**64 - 1), ("http_status", 599), ("error_code", 2**32 - 1), ("part_index", 2**32 - 1)):
        if raw.get(key) is not None:
            value = raw[key]
            if type(value) is not int or not 0 <= value <= maximum:
                return None
            result[key] = value
    if raw.get("outcome") is not None:
        if not isinstance(raw["outcome"], str) or raw["outcome"] not in OUTCOMES:
            return None
        result["outcome"] = raw["outcome"]
    if raw.get("gap_reason") is not None:
        reason = raw["gap_reason"]
        if not isinstance(reason, str) or reason not in {"missing_parent", "invalid_parent", "mismatched_turn"}:
            return None
        result["gap_reason"] = reason
    return result


def read_records(streams, turn: str):
    records, seen, ingress = [], set(), OrderedDict()
    invalid = scanned = byte_count = 0
    truncated = False
    for source in streams:
        while True:
            line = source.readline(MAX_LINE + 1)
            if not line:
                break
            byte_count += len(line)
            scanned += 1
            if scanned > MAX_LINES or byte_count > MAX_BYTES:
                truncated = True
                break
            if len(line) > MAX_LINE:
                # Bound even a single enormous newline-free input.
                while line and not line.endswith(b"\n"):
                    line = source.readline(MAX_LINE + 1)
                    byte_count += len(line)
                    if byte_count > MAX_BYTES:
                        truncated = True
                        break
                invalid += 1
                if truncated:
                    break
                continue
            try:
                raw = json.loads(line)
            except (ValueError, UnicodeError, RecursionError):
                invalid += 1
                continue
            record = clean_record(raw)
            if record is None:
                invalid += 1
                continue
            if record["stage"] == "ingress":
                key = (record["request_id"], record["phase"])
                ingress[key] = record
                if len(ingress) > MAX_RECORDS:
                    ingress.popitem(last=False)
                continue
            if record["turn_id"] != turn:
                continue
            key = json.dumps(record, sort_keys=True)
            if key in seen:
                continue
            if len(records) == MAX_RECORDS:
                truncated = True
                break
            seen.add(key)
            records.append(record)
        if truncated:
            break
    requests = {r["ingress_request_id"] for r in records if "ingress_request_id" in r}
    for record in ingress.values():
        if record["request_id"] in requests:
            if len(records) == MAX_RECORDS:
                truncated = True
                break
            records.append(record)
    return records, invalid, truncated


def report(turn: str, records: list[dict], invalid: int = 0, truncated: bool = False) -> dict:
    groups, milestones, links = {}, [], []
    for record in records:
        if record["phase"] == "link":
            links.append(record)
            continue
        root = record["stage"] in ("admission", "execution") and not record.get("parent_span_id")
        if record["phase"] == "milestone" and not root:
            milestones.append(record)
            continue
        key = ("root",) if root else (record["service"], record["instance_id"], record["span_id"])
        item = groups.setdefault(key, {"service": record["service"], "span_id": record["span_id"],
                    "stage": "execution" if root else record["stage"], "parent_span_id": record.get("parent_span_id"),
                    "instances": set(), "start": None, "end": None, "duration_ms": None,
                    "outcome": "unknown", "root": root, "part_index": record.get("part_index")})
        item["instances"].add(record["instance_id"])
        if record["phase"] == "start":
            item["start"] = min(item["start"] or record["timestamp"], record["timestamp"])
        if record["phase"] == "end" or (root and record["phase"] == "milestone"):
            if item["end"] is None or record["timestamp"] >= item["end"]:
                item["end"] = record["timestamp"]
                for field in ("duration_ms", "outcome", "http_status", "error_code"):
                    if field in record:
                        item[field] = record[field]
    steps = list(groups.values())
    for item in steps:
        item["instances"] = sorted(item["instances"])
        item["incomplete"] = not (item["start"] and item["end"]) or item["outcome"] == "unknown"
    steps.sort(key=lambda item: (item["start"] or item["end"] or "", item["service"], item["span_id"]))
    services = sorted({record["service"] for record in records})
    expected = {"nyxid"}
    if any(record["stage"] == "engine" for record in records):
        expected.add("nyxagent")
    if any(record["stage"] in ("gateway", "replay") or (record["stage"] == "channel" and record.get("outcome") == "streamed") for record in records):
        expected.add("cma")
    missing = sorted(expected - set(services))
    root = next((item for item in steps if item["root"]), None)
    incomplete = sum(item["incomplete"] for item in steps)
    linked = {r["ingress_request_id"] for r in links if "ingress_request_id" in r}
    observed_ingress = {r["request_id"] for r in records if r["stage"] == "ingress"}
    missing_ingress = sorted(linked - observed_ingress)
    partial = bool(not root or incomplete or missing or missing_ingress or truncated or invalid)
    return {"schema_version": 1, "turn_id": turn, "trace_id": turn.replace("-", ""),
            "coverage": {"status": "partial" if partial else "observed", "services_seen": services,
                         "services_missing": missing, "ingress_requests_missing": missing_ingress, "incomplete_spans": incomplete,
                         "invalid_or_unrecognized_lines": invalid, "truncated": truncated,
                         "note": "Collected records only; absence is not proof of success. Retention and collection are deployment-owned."},
            "execution": root, "steps": steps, "milestones": sorted(milestones, key=lambda r: r["timestamp"]),
            "links": sorted(links, key=lambda r: r["timestamp"]),
            "delivery": [item for item in steps if item["stage"] in ("channel", "channel_part", "replay", "platform")] + [item for item in milestones if item["stage"] in ("channel", "platform")],
            "longest_observed": sorted((item for item in steps if item["duration_ms"] is not None and not item["root"]),
                                       key=lambda item: item["duration_ms"], reverse=True)[:5]}


def render_text(result: dict) -> str:
    coverage = result["coverage"]
    lines = [f'Turn {result["turn_id"]}', f'Exact log query: trace_id = "{result["trace_id"]}"',
             f'Coverage: {coverage["status"]}; services: {", ".join(coverage["services_seen"]) or "none"}']
    if coverage["services_missing"]:
        lines.append('Missing counterpart records: ' + ', '.join(coverage['services_missing']))
    if coverage["truncated"]:
        lines.append('Input truncated at the reader limit.')
    if not result['steps']:
        lines.append('No usable trace records. Check collection/retention; this is not a successful empty turn.')
    by_span = {item['span_id']: item for item in result['steps']}
    for item in result['steps']:
        depth, parent, visited = 0, item['parent_span_id'], {item['span_id']}
        while parent in by_span and parent not in visited and depth < 16:
            visited.add(parent)
            depth += 1
            parent = by_span[parent]['parent_span_id']
        duration = 'unknown' if item['duration_ms'] is None else str(item['duration_ms']) + ' ms'
        suffix = ' (incomplete)' if item['incomplete'] else ''
        part = f" part {item['part_index']}" if item.get('part_index') is not None else ''
        lines.append(f'{"  " * depth}{item["service"]}/{item["stage"]}{part}: {item["outcome"]}, {duration}{suffix}')
    for item in result['milestones']:
        detail = item.get('outcome', 'observed')
        if 'duration_ms' in item:
            detail += f", {item['duration_ms']} ms"
        lines.append(f"{item['service']}/{item['stage']} milestone: {detail}")
    if not any(item.get('outcome') == 'confirmed' and item.get('stage') in ('channel', 'platform') for item in result['delivery']):
        lines.append('Channel delivery: no confirmation observed for a complete send (part acknowledgements may exist; delivery may not apply).')
    for link in result['links']:
        if 'ingress_request_id' in link:
            lines.append('Linked ingress request: ' + link['ingress_request_id'])
    lines.append('Durations may overlap; child times are not summed. ' + coverage['note'])
    return '\n'.join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--trace-id', required=True, help='Turn UUID or its 32-digit W3C spelling')
    parser.add_argument('--input', nargs='+', required=True, help='NDJSON stdout exports; - reads stdin')
    parser.add_argument('--json', action='store_true', help='Emit the sanitized structured report')
    args = parser.parse_args(argv)
    turn = identifier(args.trace_id)
    if turn is None:
        parser.error('--trace-id must be a turn UUID')
    streams = []
    try:
        for path in args.input:
            streams.append(sys.stdin.buffer if path == '-' else Path(path).open('rb'))
        records, invalid, truncated = read_records(streams, turn)
        result = report(turn, records, invalid, truncated)
        print(json.dumps(result, indent=2, sort_keys=True) if args.json else render_text(result))
        return 0 if records else 2
    except OSError:
        print('Could not read the requested log input.', file=sys.stderr)
        return 2
    finally:
        for stream in streams:
            if stream is not sys.stdin.buffer:
                stream.close()


if __name__ == '__main__':
    raise SystemExit(main())
