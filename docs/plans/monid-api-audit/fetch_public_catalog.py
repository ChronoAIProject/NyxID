"""Read Monid's public website catalog; never execute a provider endpoint."""
import argparse
import concurrent.futures
import datetime
import hashlib
import json
import pathlib
import threading
import time
import urllib.error
import urllib.parse
import urllib.request

BASE = "https://api.monid.ai/public/v1"


def read_json(path):
    if not path.startswith("/"):
        raise ValueError("expected public catalog path")
    for attempt in range(4):
        try:
            request = urllib.request.Request(BASE + path, headers={"User-Agent": "NyxID-public-catalog-study/1.0"})
            with urllib.request.urlopen(request, timeout=40) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            if error.code not in (429, 500, 502, 503, 504) or attempt == 3:
                raise
            time.sleep(min(10, max(1, int(error.headers.get("Retry-After", 2 ** attempt)))))
        except (TimeoutError, urllib.error.URLError):
            if attempt == 3:
                raise
            time.sleep(2 ** attempt)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", type=pathlib.Path, required=True)
    parser.add_argument("--workers", type=int, default=8)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    detail_dir = args.output / "details"
    detail_dir.mkdir(exist_ok=True)
    started = datetime.datetime.now(datetime.timezone.utc).isoformat()
    stats = read_json("/stats?include_all=true")
    categories = read_json("/categories?include_all=true")
    resources = read_json("/resources?include_all=true")
    items, cursor, seen_cursors, pages, totals = [], None, set(), 0, []
    while True:
        query = {"limit": 100, "include_all": "true"}
        if cursor:
            query["cursor"] = cursor
        page = read_json("/endpoints?" + urllib.parse.urlencode(query))
        items.extend(page["items"])
        pages += 1
        totals.append(page.get("total"))
        print(json.dumps({"page": pages, "items": len(items), "reported_total": page.get("total")}), flush=True)
        cursor = page.get("cursor")
        if not cursor:
            break
        if cursor in seen_cursors:
            raise RuntimeError("cursor loop")
        seen_cursors.add(cursor)
    identities = [(item["provider"], item["endpoint"]) for item in items]
    if len(set(identities)) != len(items):
        raise RuntimeError("duplicate provider/endpoint identities in pagination")
    provider_counts = {}
    for item in items:
        provider_counts[item["provider"]] = provider_counts.get(item["provider"], 0) + 1

    def reconcile(provider):
        page = read_json("/endpoints?" + urllib.parse.urlencode({
            "provider": provider, "limit": 100, "include_all": "true"}))
        record = {"provider": provider, "enumerated": provider_counts[provider],
                  "reported_total": page.get("total")}
        if record["enumerated"] != record["reported_total"]:
            scoped_items = list(page["items"])
            cursor = page.get("cursor")
            cursors = set()
            while cursor:
                if cursor in cursors:
                    raise RuntimeError("provider cursor loop")
                cursors.add(cursor)
                page = read_json("/endpoints?" + urllib.parse.urlencode({
                    "provider": provider, "limit": 100, "include_all": "true", "cursor": cursor}))
                scoped_items.extend(page["items"])
                cursor = page.get("cursor")
            known = set(identities)
            record["missing_items"] = [item for item in scoped_items
                                       if (item["provider"], item["endpoint"]) not in known]
            record["provider_scoped_returned"] = len(scoped_items)
        return record

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as executor:
        reconciliation = list(executor.map(reconcile, sorted(provider_counts)))
    for record in reconciliation:
        for item in record.get("missing_items", []):
            items.append(item)
    (args.output / "reconciliation.json").write_text(json.dumps(reconciliation, indent=2) + "\n")
    index = {"started_at": started, "stats": stats, "categories": categories,
             "resources": resources, "items": items, "pages": pages, "page_totals": totals,
             "provider_reconciliation": reconciliation}
    (args.output / "catalog-index.json").write_text(json.dumps(index, indent=2) + "\n")
    completed = 0
    lock = threading.Lock()

    def inspect(item):
        nonlocal completed
        identity = item["provider"] + "#" + item["endpoint"].lstrip("/")
        file = detail_dir / (hashlib.sha256(identity.encode()).hexdigest() + ".json")
        result = {"id": identity, "provider": item["provider"], "endpoint": item["endpoint"]}
        if file.exists():
            result.update(json.loads(file.read_text()))
        else:
            path = "/providers/" + urllib.parse.quote(item["provider"], safe="")
            path += "/endpoints" + urllib.parse.quote(item["endpoint"], safe="/")
            try:
                result["detail"] = read_json(path)
            except Exception as error:
                result["error"] = str(error)
            file.write_text(json.dumps(result, separators=(",", ":")) + "\n")
        with lock:
            completed += 1
            if completed % 100 == 0 or completed == len(items):
                print(json.dumps({"details_completed": completed, "total": len(items)}), flush=True)
        return result

    with concurrent.futures.ThreadPoolExecutor(max_workers=args.workers) as executor:
        details = list(executor.map(inspect, items))
    finished_stats = read_json("/stats?include_all=true")
    snapshot = {**index, "finished_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
                "final_stats": finished_stats, "details": details}
    output = args.output / "hosted-catalog.json"
    output.write_text(json.dumps(snapshot, indent=2) + "\n")
    print(json.dumps({"output": str(output), "endpoints": len(items),
                      "providers": len(set(item["provider"] for item in items)),
                      "detail_errors": sum("error" in d for d in details),
                      "final_stats": finished_stats}), flush=True)


if __name__ == "__main__":
    main()
