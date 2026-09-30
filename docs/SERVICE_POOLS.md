# Service pools

A service pool gives several connections one stable proxy route. A priority pool
can prefer a platform connection and try a BYOK connection when the provider
rejects a request or a configured transport failure occurs. Each member remains
an ordinary `UserService`, with its own endpoint, credential, authorization and
billing classification. Pools belong to a person or organization.

## Choose a strategy and contract

| Strategy | Selection | Retry and cooldown |
|---|---|---|
| `round_robin` | Rotate through enabled, authorized members | One member per request; existing behavior |
| `weighted` | Select members according to weight | One member per request; existing behavior |
| `priority` | Lower member priority first; balance within each visited tier | Bounded failover and durable cooldown |

Priority pools have a separate `tier_balance` of `round_robin` or `weighted`.
Backup tiers advance only when visited, so intermittent primary failures still
distribute work across backups. Members have weight 1–1000 and may be disabled
without deleting their connection.

A priority pool uses one of two request contracts:

- **Same API (`same_api`)** forwards the existing HTTP wire format. Members must
  accept the same methods, paths and bodies. Connections to the same known catalog
  API can use its metadata. Custom or different catalog APIs require
  `same_api_compatible: true` on every affected member. A declaration cannot
  override a known inference protocol mismatch, including Responses versus
  Completions. Use `ai_chat` for supported cross-protocol translation.
- **AI chat (`ai_chat`)** accepts OpenAI chat-completions requests. Each member
  specifies its provider model. Authoritative catalog inference metadata selects
  OpenAI Completions, OpenAI Responses, or Anthropic Messages conversion. Supported
  tools, text, images and streams are converted; unsupported or lossy request
  features are rejected before credential decryption or dispatch. Embeddings and
  WebSocket upgrades are not AI chat operations. Model mapping applies only to AI
  chat pools. Anthropic requests default to `max_tokens: 4096` when neither
  `max_tokens` nor `max_completion_tokens` is supplied.

AI members keep their configured connection URL. A root base URL appends
`v1/chat/completions`, `v1/messages`, or `v1/responses`. A non-root base URL is an
explicit API prefix; the native operation is appended once without inserting
another `v1`. Examples: `/v1beta/openai/chat/completions`,
`/openai/deployments/name/chat/completions`, and `/backend-api/codex/responses`.
The caller's query string is forwarded through the existing proxy rules. Health
and operation authorization use the same relative native path as dispatch.
Codex uses SSE transport and its required headers while preserving the configured
URL and an explicit custom User-Agent. Pool attempts request identity encoding;
compressed successful responses from an upstream that ignores this are rejected
before commitment. Compressed error responses retain their original status and
encoding and are not parsed for token usage.

Creation and atomic edits validate the merged configuration. Execution checks
live metadata again. Editing a contract and its member models must be one update.

## Platform first, BYOK fallback

First configure both connections under the same owner using AI Services or the
existing service commands. `nyxid keys` shows configured connections and their
credential bindings. The platform connection needs a live public/restricted grant;
an organization pool requires eligible organization-owned connections.

For two connections to the same API, create `members.json` (replace these slugs
with your existing connection slugs):

```json
[
  {"user_service_id":"platform-chat","priority":0,"weight":1,"enabled":true},
  {"user_service_id":"my-chat-key","priority":10,"weight":1,"enabled":true}
]
```

```bash
nyxid pool candidates --contract same_api --method POST --path /chat/completions
nyxid pool create --slug reliable-chat --name "Reliable chat" \
  --strategy priority --contract same_api --members-file members.json
nyxid proxy request reliable-chat /chat/completions -m POST \
  -d '{"model":"your-model","messages":[{"role":"user","content":"Hello"}]}'
nyxid pool show reliable-chat --method POST --path /chat/completions
```

Member inputs accept an owned connection slug or UUID; responses contain canonical
UUIDs. Use `--org <UUID|slug|name>` consistently for organization management. The
CLI resolves the pool within that owner, so a same-slug personal pool is distinct.
Foreign connection UUIDs cannot be inserted.

## AI chat aliases

For different supported provider protocols, use an AI chat pool and explicit
models in `ai-members.json`:

```json
[
  {"user_service_id":"platform-openai","priority":0,"model":"gpt-model"},
  {"user_service_id":"my-anthropic","priority":10,"model":"claude-model"}
]
```

```bash
nyxid pool create --slug assistant --name "Assistant" --strategy priority \
  --contract ai_chat --members-file ai-members.json
nyxid pool candidates --pool assistant
nyxid pool show assistant
nyxid proxy request assistant /chat/completions -m POST \
  -d '{"messages":[{"role":"user","content":"Hello"}],"stream":true}'
```

The same contract is available through the gateway:

```http
POST /api/v1/llm/gateway/v1/chat/completions
Authorization: Bearer <your NyxID credential>
Content-Type: application/json

{"model":"pool:assistant","messages":[{"role":"user","content":"Hello"}]}
```

`GET /api/v1/llm/pools?offset=0&limit=100` lists caller-accessible AI aliases,
with `next_offset` for additional pages. Discovery reads metadata without
materializing credentials. A key scoped only to `llm:proxy` can use the gateway
alias; the ordinary slug route still requires REST proxy access. The selected
connection's custom URL remains the destination, including for a custom-named
connection bound to the Codex catalog. Pool adapters do not replace it with a
provider's default URL.

## Retry, deadlines and streams

Priority pools with omitted or null `failover` use these defaults:

| Setting | Default |
|---|---|
| Maximum attempts | 3; server maximum 5 |
| Per-attempt timeout | 60,000 ms |
| Overall preparation and attempt deadline | 120,000 ms |
| Replay body limit | 8 MiB; existing ingress size limit also applies |
| Retry causes | `connect_error`, `node_offline`, `timeout`, `http_429`, `http_502`, `http_503`, `http_504`, `http_529` |
| Ambiguous dispatch replay | Off |
| Cooldown | 5 seconds, exponential to 300 seconds, after 1 failure |
| Honor provider Retry-After | Yes, within the configured maximum |

The configured causes are necessary but not sufficient to replay a request.
Proven unsent failures and explicit upstream 429 rejection can fall back before
commitment. Timeouts, post-dispatch connection failures and 5xx responses may
follow completed provider work. Unsafe methods such as POST require
`retry_ambiguous_dispatch: true` for those ambiguous failures. Enabling that option
can duplicate work and charges. Node dispatch retains its stricter ambiguity
rules. NyxID's own admission, rate limit, approval and billing errors are terminal.
Caller cancellation cancels work and never starts a backup. It does not cool a
healthy member, and neither does loss of a billing lease. Only upstream failures
whose causes are enabled in `retry_on` affect failure cooldown; this is independent
of whether the current method can safely replay. Retry-After is a lower bound
alongside exponential backoff, capped at the configured cooldown maximum.

A body larger than the replay limit makes one attempt. Each member is visited at
most once. The overall deadline bounds preparation, attempts and acknowledged
cleanup, including policy and human approval waits. A preparation timeout is
terminal and does not cool a member. Each attempt has one deadline through headers, first data and rejected
body drain. Once the first nonempty body frame is returned to the caller, later
stream errors propagate as errors and never splice another member's output.
Native AI error/truncation is not converted into a successful empty response or
fabricated completion marker.

Configure policy directly from the CLI:

```bash
nyxid pool set-failover my-llm --retry-on 429,5xx,timeout,node_offline --max-attempts 3
nyxid pool set-failover my-llm --per-attempt-timeout-ms 10000 \
  --overall-deadline-ms 30000 --max-replay-body-bytes 1048576
nyxid pool set-failover my-llm --cooldown-base-ms 5000 --cooldown-max-ms 120000 \
  --cooldown-failures-to-open 2 --honor-retry-after
```

Inline flags merge **only supplied fields** into the saved policy, including
individual cooldown fields. The CLI reads policy, pool ID and revision in one
snapshot, then sends one revision-checked PUT to that ID. A concurrent edit returns
409; the CLI does not retry with a newer revision. Omitted or null saved policy
starts from server defaults. Add `--org <UUID|slug|name>` for an organization pool.

| Inline flag | Policy field |
|---|---|
| `--retry-on CAUSE,...` (repeatable) | `retry_on` |
| `--max-attempts N` | `max_attempts` (1–5) |
| `--per-attempt-timeout-ms N` | `per_attempt_timeout_ms` (1,000–300,000) |
| `--overall-deadline-ms N` | `overall_deadline_ms` (1,000–600,000) |
| `--max-replay-body-bytes N` | `max_replay_body_bytes` (positive) |
| `--retry-ambiguous-dispatch[=true\|false]` | `retry_ambiguous_dispatch` |
| `--cooldown-base-ms N` | `cooldown.base_ms` (positive) |
| `--cooldown-max-ms N` | `cooldown.max_ms` (at least base, at most 3,600,000) |
| `--cooldown-failures-to-open N` | `cooldown.failures_to_open` (positive) |
| `--honor-retry-after[=true\|false]` | `cooldown.honor_retry_after` |

Retry causes accept the canonical names `connect_error`, `node_offline`,
`transport_error`, `timeout`, `http_401`, `http_403`, `http_408`, `http_429`,
`http_500`, `http_502`, `http_503`, `http_504`, and `http_529`. Numeric aliases are
`401,403,408,429,500,502,503,504,529`. **`5xx` expands only to
`http_500,http_502,http_503,http_504,http_529`**, the supported server-error triggers.
Duplicates are removed; unsupported values such as `501` are rejected. Use
`--retry-on none` alone to clear the cause list. JSON files use canonical names.
Credential-rejection causes refer only to upstream responses, never local denial.

Selecting `5xx` does **not** enable unsafe POST replay. The saved ambiguity setting
is preserved unless explicitly changed. `--retry-ambiguous-dispatch` (alias
`--retry-non-idempotent`) explicitly enables ambiguous replay and can duplicate
work or charges; `--retry-ambiguous-dispatch=false` turns it off. Both boolean
flags accept a bare flag for true or `=false` for false. Node dispatch's stricter
safety rules still apply.

`nyxid pool set-failover <pool> --file policy.json` **replaces** the saved override;
omitted fields use server defaults, not previous custom values. For example,
`{"max_attempts":2,"per_attempt_timeout_ms":10000}` replaces all other custom
settings with defaults. `--defaults` or a file containing `null` clears the
override. `--disable` replaces it with `{"max_attempts":1}`; use the inline
`--max-attempts 1` to preserve other settings instead. Null does **not** disable
priority fallback. One attempt still selects an eligible member by priority and
cooldown; `--disable` does not force a particular member. File, defaults, disable
and inline settings are mutually
exclusive; invoking `set-failover` without a mode or setting is a usage error.

## Inspect, edit and reset

```bash
nyxid pool list --org research
nyxid pool candidates --pool reliable-chat --method GET --path /items --limit 25
nyxid pool candidates --pool reliable-chat --after '<next_cursor>' --search chat
nyxid pool health reliable-chat --method POST --path /chat/completions
nyxid pool reset-health reliable-chat --service my-chat-key
nyxid pool reset-health reliable-chat
nyxid pool add-member reliable-chat --service another-chat --priority 20
nyxid pool add-member reliable-chat --service another-chat --weight 3 --enabled false
nyxid pool set-strategy reliable-chat priority --tier-balance weighted
nyxid pool remove-member reliable-chat --service another-chat
```

Candidates include availability, effective credential class, protocol, compatibility
requirements and node upgrade reasons. Follow `has_more`/`next_cursor`; a page is
not the complete inventory. Existing-pool inspection includes peer compatibility.
The REST candidate API additionally accepts `peer_ids` and `declared_peer_ids`
(comma-separated UUIDs, at most 50) to inspect an unsaved selection. The dashboard
sends the draft strategy/contract and confirmations. Same API inspection needs the
actual method/path; AI defaults to `POST chat/completions`. Health always uses the
saved configuration and directly fetches its members, including unavailable rows.

Save settings and members together with `nyxid pool update <pool> --file update.json`:

```json
{
  "expected_revision": 7,
  "strategy": "priority",
  "tier_balance": "weighted",
  "is_active": true,
  "description": null,
  "member_contract": "ai_chat",
  "members": [
    {"user_service_id":"platform-openai","priority":0,"model":"gpt-model"},
    {"user_service_id":"my-anthropic","priority":10,"model":"claude-model"}
  ]
}
```

Use the revision from `show` for a previously read draft. If absent, the CLI reads
the current revision before submitting. A conflict returns 409; reload and review
instead of silently overwriting concurrent changes. Omitted fields preserve the
current value. Null clears description/model/failover; a members array replaces
membership while preserving omitted fields for retained IDs. The dashboard sends
one PUT, including `expected_revision`, for all edited settings and members.
`set-strategy` can also change tier balancing and uses a revision-checked PUT.
`add-member` updates existing members while preserving omitted fields; its flags
cover priority, weight, enabled state, model, `--clear-model` and
`--same-api-compatible true|false`. Contract changes that require member models,
pool Enable/Disable (`is_active`), and combined configuration changes use the
atomic update JSON above.

Cooldown is scoped to pool configuration, member, effective credential revision,
destination, native operation and model. Reset changes a durable generation, so
old attempts cannot reinstate cleared health or erase a new failure. It is allowed
for disabled or removed-service members still present in the pool. Reset is not a
probe and does not dispatch a request. If all eligible members are cooling, the
proxy returns 503 (`service_pool_cooling_down`) with a rounded-up `Retry-After`
for the earliest deadline. Expired attempt/preparation budgets return 504
(`service_pool_deadline_exceeded`) with safe attempt summaries. Failed accounting
acknowledgment returns 503 (`service_pool_infrastructure_unavailable`) and stops
fallback. Successful completed bodies are not corrupted by a later bookkeeping
error; durable settlement intent and reconciliation retain known usage.

`pool show`, `pool candidates` and `pool health` display the selected operation.
Same API health for `POST /` describes only that operation, not all API paths.
The dashboard method/path controls select the same operation-scoped view.

Responses include `x-nyxid-pool-member` and `x-nyxid-pool-attempts`. Exhaustion
preserves the final upstream response when possible. Transport-only exhaustion
returns a structured pool error with ordered safe reasons, without destination or
credential details. `service_pool_attempt` audit events identify the member,
priority, attempt, upstream status and terminal/retry reason through the normal
hash-chained audit service.

Same-origin asynchronous Location headers point to the selected member's normal
slug with its `_nyxid_via` ID. Follow those URLs unchanged: the job remains on its
creator even if the pool is reordered. Normal authorization still applies; the
internal selector is removed before forwarding upstream.

## Authorization, billing and deployment

Personal service and legacy resolution keep their precedence over organization
fallback. Organization membership, service/node scopes, native operation policy
and human approval apply to the exact selected member before credential
materialization. A human denial or approval timeout stops the request. Unsupported
exact-target, delegated/durable and WebSocket paths fail closed for priority pools.

Each attempted member gets its own billing identity and funding reservation, using
its actual credential class and owner. Known consumption is settled before the
next member; two providers reporting usage produce two charges. Unknown token
usage stays null and is never estimated from response bytes. Pool attempts have
durable leases so recovery can release unknown holds after process death; live
streams renew independently of caller polling and cancel provider work on lease
loss. Existing exact allowance → grant → wallet precedence and balanced ledger
postings are unchanged. Platform usage is billed to the acting person.

Node-routed priority members require a current node advertising `http_cancellation`,
including eligible fallback node routes. Upgrade the CLI node agent and reconnect;
inspection reports `node_upgrade_required` until the live capability is present.
Legacy round-robin/weighted eligibility remains compatible with older nodes.

Upgrade **all backend replicas and management clients before creating priority
pools**. Before downgrading to a version without this strategy, **convert or delete
every priority pool**. Disabling is insufficient: older backends deserialize
disabled pools when listing them. Conversion is an atomic update to an old strategy
with AI model/contract/failover fields cleared. For example, use the current
revision and include every retained member in one update:

```json
{
  "expected_revision": 7,
  "strategy": "round_robin",
  "tier_balance": "round_robin",
  "member_contract": "same_api",
  "failover": null,
  "members": [
    {"user_service_id":"platform-openai","priority":0,"model":null},
    {"user_service_id":"my-anthropic","priority":0,"model":null}
  ]
}
```

Legacy routing does not translate between providers; retain only members that
accept the same wire API when converting, or delete the pool instead.
Billing attempt metadata is additive
and does not introduce an unknown persisted usage status. Existing exact billing
migration restrictions still apply independently; pool conversion does not make
old binaries safe against migrated exact-accounting data.
