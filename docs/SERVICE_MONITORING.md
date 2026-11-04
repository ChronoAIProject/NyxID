# Connected-service monitoring

The Monitoring card on `/keys/{id}` reads and manages one exact UserService
through fixed NyxID routes. chrono-probe owns configuration, check resolution,
latest check results, and discovered paths. NyxID stores no monitoring
configuration or raw check results and runs no status ingestion job. Successful
REST, LLM and MCP invocations do not contact monitoring. Eligible failed
invocations can add the bounded diagnostic evidence described below.

## Deployment

The `services:monitoring` feature flag is default off. Enable it for the acting
person through the existing global, organization or user overrides. Resolution
includes live organization membership overrides, matching `/users/me` and the
frontend flag mechanism. When off, all five routes return the same generic
404 `not_found` before service lookup, authorization details, callback-token
minting or chrono-probe traffic. The frontend hides Monitoring and sends no
monitoring request while the flag is off.

Set the backend's optional `CHRONO_PROBE_URL` to the internal HTTP(S) origin of
chrono-probe, for example `http://chrono-probe:8080`. Use a trusted private
network for HTTP, or HTTPS where the network is not trusted. The origin must
have no path, userinfo, query, or fragment. Redirects and ambient HTTP proxies
are disabled. When the flag is on, an unset or invalid origin returns 404
`monitoring_not_configured` (13301) on every monitoring route. The frontend hides
the section after this read and does not poll or retry it. A configured but
unreachable or failing chrono-probe instead returns 503 `monitoring_unavailable`
(13300), retaining the “Monitoring is unavailable” message. These conditions
affect only Monitoring.

Deploy a chrono-probe version implementing
[chrono-probe #8](https://github.com/ChronoAIProject/chrono-probe/issues/8)
([PR #9](https://github.com/ChronoAIProject/chrono-probe/pull/9)). The existing
owner config and on-demand probe endpoints alone are insufficient: exact
status reads and draft validation must also exist. This integration uses that
PR's `interval_ms` / `stale_at` status contract. For the unified latest result and
saved-path behavior, also deploy [PR #12](https://github.com/ChronoAIProject/chrono-probe/pull/12)
before this frontend update. It never uses the full-inventory `/status` endpoint
or a platform probe key.

chrono-probe must be configured to call this NyxID installation for identity,
service inventory, and exact proxy execution. Its existing authorizer reads
`GET /api/v1/users/me` and `GET /api/v1/keys` with the callback token.

## Routes and permissions

All five routes require a first-party human browser session or first-party
account access token. Third-party OAuth clients, Agent Keys, service accounts,
relay credentials, and delegated tokens are refused. There is no new generic
proxy or MCP management tool.

| NyxID route | chrono-probe route | Authority |
| --- | --- | --- |
| `GET /api/v1/keys/{id}/monitoring` | `GET /api/v1/monitoring/services/{id}/status`; managers also read `/config` | Exact owner or allowed org admin/member |
| `PUT /api/v1/keys/{id}/monitoring` | `PUT /api/v1/monitoring/services/{id}/config` | Owner or allowed org admin |
| `DELETE /api/v1/keys/{id}/monitoring?revision=N` | `DELETE /api/v1/monitoring/services/{id}/config?revision=N` | Owner or allowed org admin |
| `POST /api/v1/keys/{id}/monitoring/validate` | `POST /api/v1/monitoring/services/{id}/config/validate` | Manager with execution authority |
| `POST /api/v1/keys/{id}/monitoring/probe` | `POST /api/v1/monitoring/services/{id}/probes` | Owner or allowed org admin/member with execution authority |

`{id}` is a UserService UUID. A slug is never accepted or joined, even when
personal and organization services share it. Viewers, scoped-out members, and
members blocked by the service's admin-only policy cannot read or check.
Inactive or non-HTTP services cannot run checks.

The server issues an internal delegated JWT with the caller's user ID,
`act.sub=chrono-probe`, a 30-second lifetime, and `account:read`. Only explicit
checks add `proxy`, after checking the caller's REST execution authority. The
exact target service allowlist and existing node/resource restrictions are
retained. A source session is bound for live revocation checks. No callback
token reaches the browser, logs, or audit metadata.

The existing `account:read` contract lets trusted chrono-probe read caller
account metadata and inventory for its authorizer; the service allowlist
constrains proxy execution, not all account metadata reads. The new Monitoring
responses project only the requested service's public result and, for managers,
its editable owner configuration. No platform inventory is returned.

## Payloads and behavior

Save sends the upstream revision and the complete owner check configuration:

```json
{
  "revision": 7,
  "health": { "path": "healthz", "expect_status": [200], "body_contains": "ok" },
  "readiness": null
}
```

Draft validation uses the same `health`/`readiness` shape without `revision`.
The browser validates safe service-relative GET paths, 1–8 unique HTTP status
codes, and an optional body substring of at most 1,024 UTF-8 bytes. chrono-probe
performs authoritative validation and owns all health recipes. Readiness is
explicit owner configuration. Draft tests execute only the submitted checks;
they do not save configuration, discover fallback paths, or publish scheduler
state. At least one draft check must be configured.

Save and clear preserve upstream revision conflicts as HTTP 409. The UI asks
the user to reload rather than silently overwriting another editor's changes.
Clear removes owner overrides; a built-in or discovered health check can take
over. It does not disable monitoring.

Page load and Refresh perform scoped GETs only. Status shows the latest completed check
for each assertion, whether initiated manually or by the scheduler. Check now publishes
its result in chrono-probe's existing status store; the UI reloads the authoritative
status and renders one Health/Readiness summary, including after browser refresh.
Saved owner paths and expectations remain visible before a first observation. A check
that has not run remains unknown, and no duration or HTTP status is fabricated.

chrono-probe orders observations by their check timestamp and fences obsolete owner
configuration revisions. Saving or clearing configuration invalidates old evidence.
Evidence expires at `stale_at`; stale, missing, invalid-time, and `no_health_path`
evidence never renders a current healthy/unhealthy badge. Readiness without configured
checks is labeled Not configured only when that is known.

Test draft remains temporary and does not publish a latest result. Its evidence expires
after 60 seconds. NyxID stores no monitoring history or backend cache. Latest observations
remain in chrono-probe's in-process store across browser refreshes, but are rebuilt after
probe process restart; deploy a single probe replica as in its supplied manifests.
There is no status polling. A local 10-second display timer only ages badges and makes
no network request.

## Bounds and errors

Requests have a 16 KiB JSON body limit. Upstream responses are bounded to
64 KiB; connections time out in 3 seconds and the whole monitoring read or
individual operation in 20 seconds. Check now and Test draft share an existing
MongoDB-backed actor token bucket (one token per 10 seconds, burst three) and
one cluster-wide execution slot per UserService. Capacity loss cancels the
upstream request. These coordination records contain no monitoring state.

Upstream bodies and transport diagnostics are never returned. Invalid
configuration maps to 400, authorization to 403 (not a NyxID session logout),
revision conflicts to 409, and throttling to 429 with Retry-After. Upstream
outages, incompatible responses, identity mismatches, oversized responses and
timeouts return `monitoring_unavailable` (503). Audits contain only action,
actor, exact UserService ID, optional revision, and existing request metadata;
never body substrings, response bodies, or tokens.

## Automatic failure diagnostics

With `CHRONO_PROBE_URL` configured and `services:monitoring` enabled for the
acting person, failed authenticated REST proxy, provider LLM, LLM gateway and
MCP service-tool calls can carry secondary health evidence. Both direct MCP
tools and `nyx__call_tool` use the same diagnostic path. This does not retry the
business operation. Local input/scope/approval/capacity/billing denials and
client cancellation do not initiate a probe; downstream 4xx (including
operation-specific 401/403/404) and 5xx responses qualify. Transport and broker
failures qualify only after an exact UserService has been resolved. Platform
or legacy routes without an exact UserService are left alone. Responses whose
stream already started cannot be decorated with later evidence.

The exact selected UserService UUID comes from execution resolution, including
the concrete member chosen by a pool. No diagnostic resolves a service by slug.
NyxID rechecks live ownership, organization role/resource access and active HTTP
service state. An ordinary access-token call forwards **only its original
request bearer** to chrono-probe. It does not derive a human/agent/delegated
replacement. chrono-probe's authorizer and its callback to NyxID retain that
credential's service, node, resource and execution restrictions. If those
restrictions prevent a check, evidence stays unknown.

Agent Keys (even when carried as Bearer), sessions authenticated by cookies,
service accounts, relay/delegated credentials and calls without an original
bearer receive `observation: unknown`, `outcome: non_delegable`; they never use
an operator monitoring key. There is no cached status fallback: the current
latest-status contract does not bind its shared observation to this caller
credential, so cached evidence cannot safely prove caller reachability.

A bounded `GET /api/v1/monitoring/services/{id}/status` first establishes that a
configured or discovered health path exists. Its cached health **result is not
used as evidence**. Without a known recipe, NyxID reports `no_health_path` and
does not intentionally initiate candidate-path discovery. It then makes at
most one `POST /api/v1/monitoring/services/{id}/probes`, using chrono-probe's
existing contract. No draft expectations or business request paths are sent.
The caller check can update chrono-probe's shared latest result, but does not
update scheduler-owned Prometheus metrics or alerts. No chrono-probe #14
change is required.

The existing probe API does not atomically pin the preflight configuration or
offer a no-discovery mode. If an owner clears a recipe between the status read
and the probe, chrono-probe can still discover candidate paths. NyxID rejects
evidence whose returned health path differs from the preflight path, but this
cannot undo upstream discovery. Eliminating this configuration race requires
an additive chrono-probe API contract.

The entire diagnostic (feature/ACL reads, rate admission, both upstream
requests and response reads) has a 750ms budget. Client disconnect cancels the
HTTP work; no detached task retains the bearer. Metrics admission has an
additional maximum of 25ms. Redirects and ambient HTTP proxies remain disabled,
and responses retain the existing 64KiB bound. A MongoDB token bucket permits
one diagnostic per acting person/exact service per ten seconds across replicas;
a process permits at most 32 simultaneous diagnostic futures. Nested pool and
gateway wrappers diagnose once, not once per attempt. chrono-probe delegated
callbacks and services on the configured monitoring origin are excluded;
reentrant caller probes also hit the same consumed rate bucket.

Raw REST/LLM status codes, bodies and forwarded headers are preserved. The
NyxID-owned `X-NyxID-Service-Diagnostic` response header contains only this
bounded JSON projection:

```json
{"observation":"caller","assessment":"operation_specific_likely","outcome":"completed"}
```

MCP errors retain their original content first, add a bounded
`structuredContent.nyxid_service_diagnostic` object, and append a separate text
content block containing the same evidence plus its interpretation.
This also works with the universal tool's notification/SSE response. The
projection never contains paths, raw bodies, free-text reasons, hostnames,
tokens, keys or internal errors.

| Live caller evidence | `assessment` |
| --- | --- |
| Reachable and healthy; readiness ready or absent | `operation_specific_likely` |
| Healthy; fresh reachable readiness assertion failed | `dependency_unavailable` |
| Reachable; health assertion failed | `health_assertion_failed` |
| Probe reports broker error | `broker_error` |
| Probe reports selected node offline | `node_offline` |
| Probe reports transport timeout | `timeout` |
| Missing/stale/mismatched/incomplete check evidence | `unknown` |

Monitoring timeout, outage, throttling and missing health paths also retain
`observation: unknown`, `assessment: unknown`. Their `outcome` is respectively
`timeout`, `unavailable`, `rate_limited` or `no_health_path`. The original
invocation error always remains authoritative. A healthy check suggests an
operation/path/input/permission-specific failure; it does not establish that
the operation is valid or authorized. An unhealthy check does not erase that
operation's error.

Operational log counter `service_invocation_diagnostics_total` has only the
finite outcome/observation/assessment labels (no caller/service labels). The
same projection uses canonical `monitoring.invocation_diagnostic` product
telemetry under the existing live consent gates. Admitted diagnostic attempts
write `service_invocation_diagnostic` audit rows with actor, exact UserService
UUID and the bounded outcome projection. Raw check results, paths and tokens
are never persisted by this feature; suppressed/non-delegable calls do not
create per-failure audit rows.
