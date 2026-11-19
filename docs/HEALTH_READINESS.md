# Health and readiness contract

Implements [NyxID #2715](https://github.com/ChronoAIProject/NyxID/issues/2715)
for [chrono-probe #21](https://github.com/ChronoAIProject/chrono-probe/issues/21).
Implementation: [PR #2723](https://github.com/ChronoAIProject/NyxID/pull/2723).
Release status: **unreleased**. A merged PR is not evidence that the public
ingress has deployed these routes. Record the released image tag/digest and
public verification below before enabling fixed production probes.

## Probe targets

| Environment | Backend base URL | Frontend base URL |
| --- | --- | --- |
| Production | `https://api.nyxid.ai` | `https://nyxid.ai` |
| Staging | `https://staging-api.nyxid.ai` | `https://staging.nyxid.ai` |
| Container | `http://backend:3001` | `http://frontend:8080` |
| Local production Compose host | internal backend; probe inside its container | `http://127.0.0.1:3000` |

The staging targets match `.github/scripts/deploy_staging.py`. Deployments with
custom domains use their configured `BASE_URL` and `FRONTEND_URL`. These are
direct requests, **not** NyxID UserService/proxy requests; no token or Agent Key
is required and there is no dependency on chrono-probe itself.

| Target | GET path | Success assertion | Failure assertion | Suggested caller timeout |
| --- | --- | --- | --- | --- |
| Backend | `/health` | HTTP 200, JSON `status == "ok"`; existing `version`, `commit`, `encryption` fields retained | transport/HTTP/body mismatch | 3 s |
| Backend | `/ready` | HTTP 200, JSON exactly `{"status":"ok"}` | HTTP 503, `{"status":"unavailable","failing":["mongodb"]}` or `failing:["draining"]` | 3 s |
| Frontend | `/frontend/health` | HTTP 200, JSON exactly `{"status":"ok","component":"frontend"}` | transport/HTTP/body mismatch | 3 s |
| Frontend | `/frontend/ready` | HTTP 200, JSON exactly `{"status":"ok","component":"frontend"}` | HTTP 503, JSON `status == "unavailable"`, `component == "frontend"`, nonempty `failing` | 4 s |

All four responses send `Cache-Control: no-store`, including readiness failures.
Assertions require `Content-Type: application/json` and must reject redirects,
HTML/login pages, unexpected bodies and cached responses. Disable redirect
following and caching in chrono-probe and any CDN/Ingress on these paths. A
status-code-only 200 assertion is insufficient. Probe every 10–30 seconds;
orchestrator readiness may use a 5-second interval with failureThreshold 3.

Backend failures are the closed set `mongodb`, `draining`. Draining takes
precedence and skips/cancels dependency checks; it never waits for MongoDB.
Otherwise a read-only `ping` on the application's database/client must succeed
within **one second**, including server selection and pool checkout. Failure
and timeout both report only `mongodb`, never a URI or raw exception. No
positive result is cached. The next successful check returns 200.

MongoDB is the only live dependency shared by core authentication, authorization
and credential persistence. JWT/encryption configuration and initial database
setup already gate listener startup. Optional KMS providers, billing/Lago,
email, social OAuth providers, nodes, model/tool downstreams and feature rollout
readiness are not aggregated into this process probe: their faults affect
specific capabilities, not the readiness of every NyxID request. `ping` proves
database reachability, not arbitrary writes, replica-set transaction health or
all background jobs; retain the separate Integrity/security diagnostics.

Frontend failures are the closed set `entry_html`, `static_assets`, `manifest`,
`checker`. Missing/unreadable HTML and assets can report both first two items,
in alphabetical order. An unavailable/invalid inventory reports only `manifest`;
an unavailable, crashed or stalled checker reports only `checker`.

The production image freezes an inventory outside its document root at build
time. Every request reopens the entry HTML and all built JS/CSS (including lazy
chunks) as the nginx UID 101. Files must be nonempty readable regular files of
their built size, stay within the document root, and the entry must match its
built SHA-256. File removal, truncation, permission loss or replacing the entry
with a generic HTML page fails on the next probe; restoring files recovers
without restart. This is a low-cost delivery check, not a full cryptographic
asset audit. Optional remote fonts, backend connectivity, browser execution,
login and business transactions are outside frontend readiness.

The Python standard-library checker runs alongside nginx via the image's normal
entrypoint, uses a private Unix socket in `/tmp`, and opens no TCP listener. Nginx
caps checker reads at two seconds and converts checker errors to the JSON 503
contract. No successful marker is persisted. `/frontend/health` is answered by
nginx itself even if the checker or backend fails. A stopped frontend cannot
pass either probe. Custom nginx deployments must also package the inventory,
checker and entrypoint; merely copying the SPA into a generic server does not
provide these readiness semantics.

The older `frontend/scripts/production-policy-server.py` fixture overrides the
image entrypoint (and supports native nginx) for CSP/browser tests. It does not
launch this checker; `/frontend/ready` correctly reports `checker` there. Use
`test-nginx-readiness.py` or the image's default entrypoint for readiness tests.

## Routes, compatibility and rollout

The frontend preserves the existing `/health` backend proxy, including its
version body consumed by SPA release detection. Existing Docker/liveness
consumers can keep that path. It **does not** prove frontend static delivery.
Frontend `/ready` now explicitly proxies backend `/ready` with a two-second
upstream read timeout; backend transport failures remain nginx 502/504 rather
than masquerading as frontend failures. Use the direct backend base URL for
chrono-probe's backend target. `/frontend/{health,ready}` use exact routes and
reserve their prefixes; suffixes such as `/frontend/ready/`, `/frontend/ready.js`
and `/ready/` return 404 instead of the SPA's `200 index.html`.

Existing Kubernetes/load-balancer readiness consumers already using `/ready`
retain the 200/503 status contract and gain dependency checks plus a JSON body.
Consumers requiring an empty body must adopt the above assertions. Existing
production Compose backend health changes from `/health` to `/ready` and adds
frontend `/frontend/ready`; Compose reports unhealthy rather than automatically
restarting containers. Keep liveness on `/health` to avoid restart storms during
a database outage. The repository has no maintained Kubernetes/Ingress manifests;
the deployment owner must verify their actual rules (see `DEPLOYMENT.md`).

1. Roll out backend first. Verify direct `/health` and `/ready` on each replica
   and update body assertions/timeouts before relying on the stronger readiness.
2. Roll out the matching frontend image using its default entrypoint. It can
   serve the new frontend probes independently; its old `/health` proxy remains.
3. Route backend `/health` and `/ready` to the API listener. Route frontend
   `/frontend/health` and `/frontend/ready` to frontend port 8080, with no auth,
   SPA rewrite, CDN cache or redirect. A shared-host ingress must give the
   frontend paths priority over any generic `/frontend`/SPA rewrite.
4. Apply the matching Compose healthchecks (or orchestration probe settings)
   only after those images are deployed. Set backend readiness timeoutSeconds
   to 3, frontend to 4, and retain dependency-free liveness probes.
5. Verify from the same network as chrono-probe with redirects disabled. Record
   base URLs, HTTP status/content type/body/no-store, released image tag/digest,
   and failure/recovery evidence. Then enable the four fixed probes in the
   parent issue. Never stop production MongoDB just to test a probe: inject
   dependency faults on an isolated staging replica.

Example assertion (no authentication, no redirect following):

```bash
curl --silent --show-error --max-time 3 --dump-header /tmp/ready.headers \
  https://api.nyxid.ai/ready > /tmp/ready.json
# Require HTTP 200, application/json and Cache-Control: no-store in headers.
jq -e '. == {"status":"ok"}' /tmp/ready.json
```

## Reproducible evidence

Run from the repository root:

```bash
cargo test -p nyxid readiness_
cargo test -p nyxid shutdown_serves_new_requests_health_200_ready_503
python3 frontend/scripts/test-frontend-readiness.py
python3 frontend/scripts/test-nginx-policy.py
python3 frontend/scripts/test-nginx-readiness.py
```

The Rust tests exercise real unauthenticated HTTP handlers with a real MongoDB
and an unavailable endpoint, and use virtual time for the one-second deadline
and drain interruption. `NYXID_TEST_DATABASE_URL` can select the test MongoDB.
The Docker test builds the **exact production runtime stage** with small static
fixtures, using the pinned nginx-unprivileged 1.27 image and default entrypoint.
It verifies backend-down independence, missing and chmod-000 HTML/JS/CSS/lazy
chunks, recovery after each fault, healthy-backend/failed-frontend separation,
no-store JSON, explicit routing, checker timeout/socket loss and stopped nginx.
Both dev and full frontend CI profiles run these contracts.

Local validation date: 2026-11-19. Static checker tests (11), nginx policy tests
(6), and the five production-runtime HTTP fault/recovery groups pass. A real
Vite build (480 inventory files) also returns 200 through the packaged runtime.

The implementation PR records the commands
and results for its final commit. Public-route release verification is pending:
read-only requests on this date to the production API health/ready, production
frontend health/ready and staging ready targets all returned edge HTTP 403,
`text/plain; charset=UTF-8`, `error code: 1010`. Those responses are **not**
NyxID probe success and do not establish the deployed application contract;
the deployment owner must verify edge access from chrono-probe's network.
This PR does not deploy NyxID or modify chrono-probe scheduling/alerts. Fill in
the released version, implementation PR URL, and actual public success/failure/
recovery evidence in the parent issue after rollout.
