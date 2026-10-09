# Per-person service concurrency

Admins configure **Concurrency limits** on the catalog service edit page, or use
`GET` / `PUT /api/v1/services/{catalog_service_id}/concurrency`. Both require a
platform admin; creators, org admins and operators cannot change or read this
policy. The configuration is independent of credentials, auto-connection and
NyxID delegation tokens. No policy is enabled automatically for chrono-sandbox.

```json
{
  "policy": {
    "default_limit": 4,
    "users": [{"id": "person-or-service-account-uuid", "limit": 8}],
    "orgs": [{"id": "org-user-uuid", "limit": null}]
  }
}
```

Set `policy` to `null` to remove all limits. Inside a policy, `null` explicitly
means unlimited; numbers are integers from 1 through 10000. There are at most
500 override targets in total. Targets must be unique UUIDs identifying existing
people/service accounts or organizations of the appropriate type. The UI offers
person and organization pickers; service accounts may be targeted via the API.

Precedence is an exact person/SA override, then the highest applicable organization
override, then the default. Unlimited wins among organization overrides. Org
membership is read live for each limited-service admission, including viewer
memberships; revoked memberships and inactive orgs do not qualify. Overrides can
raise or lower the default. Limits do not grant access to a service or organization.

A slot counts against the acting person's ID, including API keys, chat keys and
delegation, or against a service account's own ID. Credential owners and billing
owners do not change the counter. Different connections and aliases for one
catalog service share its capacity. Pool attempts use the selected member's
catalog policy. Separate services have separate counters.

## Dispatch and lifetime

Enforcement covers authenticated REST UUID/slug routes, MCP `tools/call` (including
universal dispatch and exact-service redemption), direct and node-routed requests,
SSE and other streaming bodies, and direct/node WebSocket passthrough. Slots are
retained through response body completion or drop, and through WebSocket closure.
MCP buffers its downstream response and retains its slot through that read and
through delivery of its JSON or inline SSE response.
Dedicated LLM provider/gateway and assistant inference dispatches use the same
policy. Native SSH exec (REST and MCP), tunnels/web terminals and realtime voice
connections also hold capacity through execution/connection closure.

Anonymous public endpoints have no acting person and retain their existing
IP/daily quota controls. Direct use of a downstream credential or an issued SSH
certificate outside NyxID is not observable by NyxID; this policy governs NyxID
execution, not the provider's own admission controls.

At capacity, admission returns `ServiceConcurrencyLimited` (12700), HTTP 429 with
`Retry-After: 1`, without queuing or contacting the provider. MCP returns an HTTP
429 JSON-RPC error with the typed key/code and the same retry header. Already
started streams cannot change their HTTP status; lease loss terminates them.
Capacity rejections are client errors in proxy telemetry, not server faults.
Grok voice starts its provider connection asynchronously after the control stream
opens. Its capacity refusal uses the existing `start_failed` event with the same
typed error; the open control stream cannot change its HTTP status.

## Replicas, crashes and policy edits

The optional `DownstreamService.concurrency_policy` travels with the catalog row
already resolved by each execution path. UserService resolution copies it in the
existing catalog authorization lookup, including its server-bound catalog ID.
Absent policies return before any concurrency database command. No policy cache,
new environment variable, membership lookup or lease query is added to services
without a policy. Old catalog rows remain unlimited; older binaries ignore the
additive field. Upgrade every serving replica before enabling policies: older
binaries cannot enforce them.

MongoDB stores one `service_concurrency_leases` document per catalog/person pair.
A unique compound index prevents duplicate scopes. Each claim transaction performs
a real write fence (`revision` increment), prunes expired leases using MongoDB
server time, checks the bounded live array and appends its unique claim only if
capacity remains. Concurrent transactions conflict on the same document; a stale
snapshot cannot over-admit. Lowering a limit counts *all* existing leases, rather
than switching to a new range of numbered slots.

Claims and renewals use majority acknowledgement. Admission confirms live ownership
again after commit, so a delayed commit acknowledgement cannot admit an expired
claim. Leases expire after 60 seconds and renew every 15 seconds. Renewal is bounded to
10 seconds; failure cancels the execution before capacity can be reclaimed.
Dropping the last guard, including cancellation or unwind, stops renewal and
schedules exact-claim release. Runtime/process failure falls back to expiry.
The TTL index removes idle scope documents; admission never depends on the TTL
sweeper's schedule. All queries use the scope, TTL, membership or existing `_id`
indexes. The lease rows contain IDs and expiry metadata only, never request
contents or credentials. Admin audit events contain service ID, enabled state,
default and target counts, without target lists.

Policy and membership changes affect newly resolved requests; running requests
keep their original leases and finish normally. If a limit is lowered below the
current count, no further request is admitted until the count falls below it.
Removing a policy permits new unlimited requests; already-held leases still clean
up normally. Requests admitted while unlimited cannot be counted retroactively.

Exact-service approvals preserve their existing single-use execution lifecycle. A
capacity refusal records `service_concurrency_limited` as a terminal execution
failure and returns 429; another execution requires a new approval.
