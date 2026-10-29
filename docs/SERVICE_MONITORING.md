# Connected-service monitoring

The Monitoring card on `/keys/{id}` reads and manages one exact UserService
through fixed NyxID routes. chrono-probe owns configuration, check resolution,
scheduled observations, and discovered paths. NyxID stores no monitoring
configuration or results, runs no status ingestion job, and adds no monitoring
work to normal REST, LLM, or MCP invocations.

## Deployment

Set the backend's optional `CHRONO_PROBE_URL` to the internal HTTP(S) origin of
chrono-probe, for example `http://chrono-probe:8080`. Use a trusted private
network for HTTP, or HTTPS where the network is not trusted. The origin must
have no path, userinfo, query, or fragment. Redirects and ambient HTTP proxies
are disabled. Unset or unavailable chrono-probe affects only Monitoring.

Deploy a chrono-probe version implementing
[chrono-probe #8](https://github.com/ChronoAIProject/chrono-probe/issues/8)
([PR #9](https://github.com/ChronoAIProject/chrono-probe/pull/9)). The existing
owner config and on-demand probe endpoints alone are insufficient: exact
status reads and draft validation must also exist. This integration uses that
PR's `interval_ms` / `stale_at` status contract. It never uses the full-inventory
`/status` endpoint or a platform probe key.

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

Page load and Refresh perform scoped GETs only. Status uses existing scheduled
observations. A missing snapshot remains unknown, including personal services
without scheduled observations. Existing scheduled evidence expires at
`stale_at`; stale, missing, invalid-time, and `no_health_path` evidence never
renders a current healthy/unhealthy badge. Readiness without configured checks
is labeled Not configured only when that is known.

Check now and Test draft render their responses in component memory for the
current page. Manual evidence expires after 60 seconds. These responses do not
replace a scheduled observation or create a NyxID history. Query data is removed
when the page's last observer unmounts; there is no status polling. A local
10-second display timer only ages badges and makes no network request.

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
