#!/usr/bin/env python3
"""Check actual HTTP policy responses from the production-policy server.

--https-configured describes the trusted public origin behind a TLS terminator;
the loopback harness transport stays HTTP. This does not claim a TLS handshake.
"""
import argparse
import http.client
import json
from urllib.parse import urlsplit


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base-url", default="http://127.0.0.1:18089")
    parser.add_argument("--https-configured", action="store_true")
    args = parser.parse_args()
    base = urlsplit(args.base_url)
    assert base.scheme == "http" and base.path in ("", "/")
    results = []

    def check(path, status=200, method="GET", body=None, declared_length=None, extra=None):
        conn = http.client.HTTPConnection(base.hostname, base.port, timeout=10)
        conn.putrequest(method, path, skip_host=True)
        conn.putheader("Host", "spoofed.example")
        conn.putheader("X-Forwarded-Proto", "https" if not args.https_configured else "http")
        if body is not None or declared_length is not None:
            conn.putheader("Content-Length", str(declared_length if declared_length is not None else len(body)))
        for key, value in (extra or {}).items():
            conn.putheader(key, value)
        conn.endheaders()
        if body is not None:
            conn.send(body)
        response = conn.getresponse()
        headers = response.getheaders()
        values = {key.lower(): value for key, value in headers}
        content = response.read()
        assert response.status == status, (path, response.status, status)
        for key in ["content-security-policy", "referrer-policy", "permissions-policy", "x-frame-options", "x-content-type-options"]:
            assert sum(name.lower() == key for name, _ in headers) == 1, (path, key, headers)
        assert values["x-frame-options"] == "DENY"
        assert values["x-content-type-options"] == "nosniff"
        assert "frame-ancestors 'none'" in values["content-security-policy"]
        assert values["referrer-policy"] == "no-referrer" or (status in (502, 413) and values["referrer-policy"] == "strict-origin-when-cross-origin")
        assert values.get("strict-transport-security") == ("max-age=31536000" if args.https_configured else None)
        assert sum(name.lower() == "strict-transport-security" for name, _ in headers) == int(args.https_configured)
        results.append({"method": method, "path": path, "status": status, "headers": values})
        conn.close()
        return values, content

    for path in ["/", "/login", "/docs", "/channel-bots/?%63laim=FIXTURE", "/%63hannel-bots?%63laim=FIXTURE", "/build-version.json"]:
        check(path)
    for path in ["/privacy", "/terms"]:
        headers, _ = check(path)
        check(path, 304, extra={"If-None-Match": headers["etag"]})
        check(path, 405, method="POST")
    check("/assets/missing-policy-fixture.js", 404)
    check("/api/v1/policy-probe.js")
    check("/oauth/policy-probe.js")
    check("/api/v1/policy-probe.js?status=500", 500)
    check("/api/v1/policy-upstream-close", 502)
    check("/api/v1/policy-too-large", 413, method="POST", declared_length=1024 * 1024 + 1)
    for route in ["reply", "thread-reply"]:
        _, body = check(f"/api/v1/channel-relay/{route}", method="POST", body=b"x" * (2 * 1024 * 1024))
        assert json.loads(body)["size"] == 2 * 1024 * 1024
        check(f"/api/v1/channel-relay/{route}", 413, method="POST", declared_length=33 * 1024 * 1024)
    print(json.dumps({"https_configured": args.https_configured, "checks": len(results), "responses": results}, indent=2))


if __name__ == "__main__":
    main()
