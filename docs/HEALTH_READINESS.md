# Health and readiness contract

Implements [NyxID #2715](https://github.com/ChronoAIProject/NyxID/issues/2715)
for [chrono-probe #21](https://github.com/ChronoAIProject/chrono-probe/issues/21).
Implementation: [PR #2723](https://github.com/ChronoAIProject/NyxID/pull/2723).
Release status: **unreleased**. A merged PR is not evidence that public ingress
has deployed these routes. Record the released image tag/digest and public
verification before enabling fixed production probes.

## Probe targets

| Environment | Backend base URL | Frontend base URL |
| --- | --- | --- |
| Production | `https://api.nyxid.ai` | `https://nyxid.ai` |
| Staging | `https://staging-api.nyxid.ai` | `https://staging.nyxid.ai` |
| Container | `http://backend:3001` | `http://frontend:8080` |
| Local production Compose host | internal backend; probe inside its container | `http://127.0.0.1:3000` |

Staging URLs match `.github/scripts/deploy_staging.py`. Custom deployments use
`BASE_URL` and `FRONTEND_URL`. Requests are direct, never NyxID UserService/proxy
requests. None of these GET endpoints requires login, a token or an Agent Key;
invalid or expired credentials do not change their probe result.

| Target | GET path | Success assertion | Failure assertion | Caller timeout |
| --- | --- | --- | --- | --- |
| Backend liveness | `/health` | 200, JSON `status == "ok"`; existing `version`, `commit`, `encryption` retained | transport/HTTP/body mismatch | 3 s |
| Backend orchestration | `/ready` | 200, **empty body** | 503, **empty body**, only while draining | 3 s |
| Backend dependency diagnostic | `/ready/dependencies` | 200, JSON exactly `{"status":"ok"}` | 503, `{"status":"unavailable","failing":["mongodb"]}` or `failing:["draining"]` | 3 s |
| Frontend liveness | `/frontend/health` | 200, JSON exactly `{"status":"ok","component":"frontend"}` | transport/HTTP/body mismatch | 3 s |
| Frontend static readiness | `/frontend/ready` | 200, JSON exactly `{"status":"ok","component":"frontend"}` | 503, JSON `status == "unavailable"`, `component == "frontend"`, nonempty `failing` | 4 s |

All responses send `Cache-Control: no-store`, including failures. JSON endpoints
require `Content-Type: application/json`. Reject redirects, HTML/login pages,
unexpected bodies and cached responses. Disable redirect following, caching and
conditional-request headers in chrono-probe and any CDN/Ingress on these paths.
A status-code-only assertion is insufficient for JSON endpoints. The four
chrono-probe targets are backend `/health` and `/ready/dependencies`, and the two
frontend probes. Probe every 10–30 seconds. Keep orchestration on `/ready`.

## Backend boundaries and database cost

`/health` and `/ready` perform no database I/O. `/ready` retains its original
meaning: this replica is accepting traffic until graceful draining starts.
MongoDB latency or pool pressure must not evict every replica from the load
balancer simultaneously. The dependency endpoint is diagnostic, **not a new
load-balancer, Kubernetes readiness or automatic restart signal**.

Dependency failures are the closed set `mongodb`, `draining`. Draining takes
precedence, bypasses cached results, and immediately cancels dependency reads
and waiters. Otherwise a read-only MongoDB `ping` uses the application's actual
database/client. A one-second deadline covers both waiting for another probe
and server selection/pool checkout/the command. Errors expose no URI or raw
exception.

Each application instance permits at most one in-flight dependency read. Requests
share its result for at most one second **from read admission**, not completion;
a slow read never gains an extra second of cached success. Both success and
failure are reused. A cancelled leader leaves a pessimistic failure until the
same window expires, preventing disconnect-driven repeated pings. No background
probe loop is started and no success can be reused after its one-second bound.
These bounded internal observations are distinct from HTTP caching, which stays
disabled. Once the window expires, a successful new check returns 200.

MongoDB is the only live dependency shared by core authentication, authorization
and credential persistence. JWT/encryption configuration and initial database
setup already gate listener startup. Optional KMS providers, billing/Lago, email,
social providers, nodes, model/tool downstreams and feature rollout readiness
are excluded. `ping` proves reachability, not arbitrary writes, replica-set
transactions or every background job; retain Integrity/security diagnostics.

## Frontend boundaries and supervision

Frontend failures are the closed set `entry_html`, `static_assets`, `manifest`,
`checker`, `overloaded`. Missing/unreadable HTML and assets may report both first
two items in alphabetical order. Invalid inventory reports only `manifest`;
an unavailable, crashed or stalled checker reports only `checker`. Rate or
concurrency rejection reports only `overloaded`.

The production image freezes an inventory outside the document root at build
time. Every admitted readiness request reopens entry HTML and all built JS/CSS,
including lazy chunks, as nginx UID 101. Files must be nonempty readable regular
files of their built size, confined to the document root; entry HTML must match
its built SHA-256. Removal, truncation, permission loss or generic replacement
HTML fails the next admitted check; restoration recovers without restarting.
This proves static delivery, not browser execution or complete asset integrity.
Optional remote fonts, backend connectivity, login and transactions are excluded.

The Python standard-library checker uses a private Unix socket and opens no TCP
listener. The entrypoint supervises and reaps it, restarting after exit with
backoff of 1, 2, 4 and at most 8 seconds; 30 seconds of stable runtime resets the
backoff. Nginx caps checker reads at two seconds and returns JSON 503 while it
is unavailable. No successful marker is persisted. `/frontend/health` is answered
by nginx independently of the checker and remains available during recovery.

To bound public probe work, `/frontend/ready` has an instance-wide nginx budget
of **10 requests/second, burst 20, at most two active requests**. Excess requests
receive uncached JSON 503 `failing:["overloaded"]` instead of queuing unbounded
file scans. `/frontend/health` is cheap and has no such limiter. Ordinary 10–30
second monitoring remains well within this budget. A stopped nginx fails both
probes at transport level. Custom deployments must package the inventory,
checker, supervisor and nginx limits together.

The older `frontend/scripts/production-policy-server.py` fixture overrides the
image entrypoint for CSP/browser tests and does not launch this checker;
`/frontend/ready` correctly reports `checker` there. Use
`test-nginx-readiness.py` or the default production entrypoint for readiness.

## Routes, existing consumers and rollout

Frontend `/health` preserves the backend/version proxy consumed by SPA release
detection. `/ready` proxies the backend's drain-only route;
`/ready/dependencies` explicitly proxies its diagnostic route. These are not
frontend static-delivery checks. Backend transport failures stay nginx 502/504.
Use the direct backend base URL for chrono-probe. Probe routes are explicit;
suffixes such as `/frontend/ready/`, `/frontend/ready.js`, `/ready/` and
`/ready/dependencies/` return 404, never SPA `200 index.html`.

**Staging's chrono-cd backend readiness probe and load balancer already use
`/ready` as a drain signal. Keep that configuration unchanged.** The related
multi-replica drain contract is described in `DEPLOYMENT.md` and #2277. The
release-version sampling in `.github/scripts/deploy_staging.py` uses `/health`
and is a separate consumer. Production currently probes backend `/health`;
production Compose retains that dependency-free backend healthcheck. Frontend
Compose adds `/frontend/ready`. The repository does not maintain chrono-cd's
external readiness/load-balancer manifests, so this PR does not change them.

1. Roll out backend first, retaining staging chrono-cd `/ready` routing and
   production `/health` settings. Verify `/ready` stays 200 with an empty body
   during isolated MongoDB failure and becomes 503 immediately on drain.
2. Verify `/ready/dependencies` separately: MongoDB failure/timeout returns JSON
   503; recovery returns 200 after at most the remaining one-second reuse window.
   Do not wire dependency failures into replica eviction or automatic restarts.
3. Deploy the matching frontend image with its default supervised entrypoint.
   Route frontend probes to port 8080 with no auth, SPA rewrite, cache or redirect.
   Verify checker crash recovery, static fault/recovery and overload rejection.
4. Enable the matching frontend Compose healthcheck only with that image.
   Existing orchestration `/ready` consumers keep their current path/body contract.
5. Verify all four chrono-probe paths from its network, with redirects, caching
   and conditional headers disabled. Record image tag/digest and public
   status/body/content-type/no-store plus fault/recovery evidence, then enable
   monitoring in the parent issue. Inject faults on isolated staging replicas;
   never stop production MongoDB merely to test a probe.

Example dependency assertion (no authentication or redirect following):

```bash
curl --silent --show-error --max-time 3 --dump-header /tmp/ready.headers \
  https://api.nyxid.ai/ready/dependencies > /tmp/ready.json
# Require HTTP 200, application/json and Cache-Control: no-store in headers.
jq -e '. == {"status":"ok"}' /tmp/ready.json
```

## Reproducible evidence

```bash
cargo test -p nyxid readiness_
cargo test -p nyxid shutdown_serves_new_requests_health_200_ready_503
python3 frontend/scripts/test-frontend-readiness.py
python3 frontend/scripts/test-nginx-policy.py
python3 frontend/scripts/test-nginx-readiness.py
```

Rust tests cover the production router with anonymous/invalid credentials,
MongoDB success/failure, drain-only orchestration, single-flight sharing,
start-bounded reuse, leader cancellation, lock/command deadlines and drain
interruption. `NYXID_TEST_DATABASE_URL` selects the test database. Run the full
backend suite through the existing `backend-tests.yml` reusable workflow; use
its Rust toolchain, replica-set setup and `RUST_MIN_STACK=8388608` settings.

The exact nginx 1.27 production runtime suite covers static failures/recovery,
backend independence, explicit routes, six credential variants, bounded checker
failure, supervised restart/reaping and overload. Checker unit tests (11), nginx
policy tests (6), and all eight runtime groups pass locally on 2026-11-19. A real
Vite build (480 inventory files) was also checked through the packaged runtime.
The PR records final backend/full-suite results and their exact commit scope.

Public-route release verification is pending. Read-only requests on this date
to the production API health/ready, production frontend health/ready and staging
ready targets returned edge HTTP 403, `text/plain; charset=UTF-8`,
`error code: 1010`. Those are not application probe success. The new dependency
path is unreleased and not included in that historical evidence. Deployment
owners must verify edge access from chrono-probe's network after rollout.
This PR does not deploy NyxID or modify chrono-probe scheduling/alerts.
