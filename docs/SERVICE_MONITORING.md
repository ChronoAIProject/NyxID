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

Agent Key-scoped invocation diagnostics additionally require
[chrono-probe #16](https://github.com/ChronoAIProject/chrono-probe/issues/16),
which preserves `X-API-Key` through exact-service authorization and the caller
probe. Deploy that companion change before enabling Agent Key redemption from
NyxID #2241; human bearer diagnostics remain compatible with older probe
deployments.

chrono-probe must be configured to call this NyxID installation for identity,
service inventory, and exact proxy execution. Its existing authorizer reads
`GET /api/v1/users/me` and `GET /api/v1/keys` with a human callback token.
Agent Key caller probes read only `GET /api/v1/keys` with the same Agent Key.

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

Automatic handle issuance is disabled by default, even when `CHRONO_PROBE_URL`
is configured. Operators must explicitly set
`INVOCATION_DIAGNOSTICS_ENABLED=true` and restart the API process to enable it.
Issuance reads only that startup configuration snapshot, with no feature-flag
database lookup on the invocation path. Redemption separately revalidates the
live `services:monitoring` flag for the acting person before probing.

For a rolling deployment, upgrade **every API replica** before enabling
`INVOCATION_DIAGNOSTICS_ENABLED`: a handle issued by one replica can be redeemed
on any replica. Enable the flag only after all replicas support issuance and
redemption. Before rolling back, unset the flag on **every replica** and restart
those processes so they stop issuing handles; allow the 60-second handle lifetime
to elapse before rolling back any replica.

Before enabling, verify that the **deployed** chrono-probe returns an explicit
boolean `reachable` for each completed health/readiness check from
chrono-probe's `POST /api/v1/monitoring/services/{user_service_id}/probes`. A healthy reachable
check must include `"reachable": true`. Source compatibility alone does not
verify the deployed version. NyxID requires `health.reachable == true` for a
caller observation; a missing field leaves the diagnostic assessment `unknown`.
Keep diagnostics disabled until this deployment check passes.

When enabled and configured, downstream 5xx responses and eligible transport or
broker failures from REST proxy, provider LLM, LLM gateway and MCP service-tool
calls can carry a short-lived opaque diagnostic handle. Downstream 4xx
responses, including authentication, validation, missing-resource and throttling
responses, do not qualify. Ordinary, unconstrained access-token callers and
eligible `ApiKeyPurpose::General` Agent Keys, including live NyxBot conversation
keys, can receive handles. Scheduled, permission-bound, `PublicNyxbot`,
`SharedEntryTransport`, session, service-account, delegated and relay callers
receive no handle, make no diagnostic database write and add no MCP diagnostic
content. The original request does not run a probe. It returns its original
status and body after one bounded MongoDB insert, preserving
application headers and appending `Cache-Control: no-store` without replacing
downstream cache directives; if handle issuance cannot finish within 250ms, the
original error is returned without a handle. Local input, scope, approval,
capacity, billing and cancellation errors do not issue handles. Transport or
broker failures qualify only after execution resolved an exact UserService.
Responses whose stream already started cannot be decorated. Within each server
process, repeated eligible failures for the same caller identity and exact
UserService are suppressed for ten seconds after the first handle is issued.
Suppressed failures retain their original response without a handle and perform
no diagnostic database write. The bounded in-memory table never stores paths,
request data, response data, credentials or raw handles.

REST and LLM failures use these NyxID-owned response headers:

```text
X-NyxID-Diagnostic-Handle: nyx_diag_<opaque 256-bit value>
X-NyxID-Diagnostic-Expires-In: 60
Cache-Control: no-store
```

These header names are NyxID-owned; matching downstream headers are stripped
even when NyxID does not issue a handle.

MCP errors retain their original content first and add the same handle under
`structuredContent.nyxid_invocation_diagnostic` and a separate text content
block. The progress digest is computed from the original tool result before
adding either diagnostic field, for both JSON and SSE responses, so random
handles do not weaken the continuation loop guard. The handle contains no
UserService ID, path, token, request body or routing data. NyxID stores only its
SHA-256 hash. The record expires after 60 seconds and binds the original actor,
authentication class, parent API-key identity, child Agent Key login credential
when present, acting or OAuth client and exact execution-selected UserService,
including a concrete pool member.

Clients may explicitly redeem the handle once with the current NyxID identity:

```http
POST /api/v1/monitoring/invocation-diagnostics
Authorization: Bearer <current NyxID credential>
Content-Type: application/json

{"handle":"nyx_diag_..."}
```

MCP callers redeem inside the same authenticated request context with
`nyx__diagnose_invocation`, passing only the opaque `handle`. The native tool
retains request-local credential material only for that call and never exposes
it in tool arguments, results, logs or persisted MCP sessions.

Agent Key callers may instead use `X-API-Key: <current Agent Key>`. NyxID also
accepts a Bearer-form Agent Key at ingress, then forwards the already classified
credential to chrono-probe as `X-API-Key`; this avoids asking chrono-probe to
guess whether an opaque Bearer value is a human token or an Agent Key.

The handle stays in the body so access logs and route metrics do not capture it.
NyxID requires the same bound identity, then rechecks the current parent and
child credential, API-key purpose and execution scope, service allowlist, live
ownership or organization/shared-agent access, resource ACL, active HTTP
service state and exact UserService. Conversation keys must retain the same
live turn authority. The caller-scoped proxy request reuses the normal Agent Key
execution path, which enforces node allowlists, credential bindings and
assistant operation scopes again. A mismatch, expiry or revoked access returns
the same not-found result and does not reveal which check failed.

An ordinary access token can run a caller-scoped probe using only the bearer on
the redemption request. chrono-probe receives that full bearer credential and
must never log or persist it, including in request, error or tracing logs.
Configure access-log and observability redaction at the probe and every
intermediary hop. DPoP- and mTLS-bound tokens cannot be replayed by the probe:
they receive no new handle, and redemption of an existing handle returns
`observation: unknown` and `outcome: non_delegable` without forwarding the
token. An eligible `ApiKeyPurpose::General` Agent Key,
including a live bot/conversation key, can run the same exact-service check with
its current Agent Key authority and returns `observation: agent` when evidence
is attributable. Unsupported API-key purposes and other non-delegable identities
do not receive new handles. If a previously eligible identity becomes
non-delegable before redemption, the existing handle completes with
`observation: unknown` and `outcome: non_delegable`. No path falls back to an
owner, operator or system monitoring key. There is no cached status fallback
because the shared latest-status result is not bound to the current caller.

Redemption atomically claims `pending` work before any probe. Concurrent
redemptions cannot start a second probe; a completed result is retry-readable
for transport recovery without probing again. A worker crash leaves the claim
running until the handle expires rather than risking a duplicate probe. If the
failed request's service-concurrency lease is still active, redemption returns
`409 diagnostic_not_ready` with `Retry-After: 1` and leaves the handle pending.
A client may retry that diagnostic once, but must not automatically retry the
failed business request.

When `services:monitoring` is enabled for the acting person, a bounded
`GET /api/v1/monitoring/services/{id}/status` first establishes that a known
health path exists. Its cached health result is not evidence. Without a known
recipe, NyxID reports `no_health_path`; otherwise it makes at most one
`POST /api/v1/monitoring/services/{id}/probes`. The exact service ID comes only
from the server-side handle record. No business path or draft expectation is
sent.

The explicit probe work, including feature and ACL reads, rate admission, both
upstream requests and response reads, has a three-second budget. Redirects and
ambient HTTP proxies remain disabled, and responses retain the existing 64KiB
bound. A MongoDB token bucket permits one probe per acting person and exact
service per ten seconds across replicas and across that person's Agent Keys.
This owner-and-service key is intentional: minting multiple Agent Keys must not
bypass the probe budget. A process permits at most 32 concurrent probe futures.
chrono-probe callbacks and services on the configured monitoring origin are
excluded from recursive diagnostics.
Caller-scoped chrono-probe proxy requests carry the reserved
`X-NyxID-Invocation-Diagnostic-Probe: 1` marker. NyxID removes it before
downstream forwarding and suppresses handle issuance for that request, including
raw Agent Key callbacks that do not have the `chrono-probe` acting-client ID.
The marker is advisory and grants no execution authority. If the normal proxy
policy would require approval, NyxID refuses that diagnostic probe before
creating an approval request or sending a notification; the diagnostic remains
bounded unknown/unavailable evidence instead of leaving a pending prompt.

The existing probe API does not atomically pin the preflight configuration. If
an owner clears a recipe between the status read and probe, chrono-probe can
still discover candidate paths. NyxID rejects evidence whose returned health
path differs from the preflight path, but cannot undo upstream discovery.

Successful redemption returns only bounded enums:

```json
{"observation":"caller","assessment":"operation_specific_likely","outcome":"completed"}
```

An attributable General Agent Key uses the same finite result shape with
`"observation":"agent"`. This proves only that the current exact Agent Key
authority reached the selected service's health/readiness path. It does not
prove human-owner reachability or that the original business operation was
valid or authorized.

The projection never contains paths, raw bodies, free-text reasons, hostnames,
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
the operation is valid or authorized. Eligible errors include `AppError::Internal`,
which can originate inside NyxID. A healthy probe and
`operation_specific_likely` never rule out a NyxID-side fault. An unhealthy check
does not erase that operation's error.

Operational log counter `service_invocation_diagnostics_total` is emitted only
when a handle is freshly completed and has finite outcome, observation and
assessment labels. The same projection uses canonical
`monitoring.invocation_diagnostic` product telemetry under existing consent
gates. Actual probe attempts write `service_invocation_diagnostic` audit rows
with actor, exact UserService UUID and the bounded projection. Raw check results,
paths, handles and tokens are never written to audit or telemetry.
