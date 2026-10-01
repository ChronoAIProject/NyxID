# Service pools

Use a pool when callers need one stable route across compatible configured
connections. Members are owner-scoped UserServices, including platform and BYOK
bindings. Discover existing connections with `nyxid keys`; never request or expose
stored provider credentials.

## Management

```bash
nyxid pool list
nyxid pool candidates --contract same_api --method POST --path /chat/completions
nyxid pool create --slug stable-chat --name "Stable chat" --strategy priority \
  --contract same_api --members-file members.json
nyxid pool show stable-chat --method POST --path /chat/completions
nyxid pool update stable-chat --file update.json
nyxid pool add-member stable-chat --service backup-slug --priority 10
nyxid pool remove-member stable-chat --service backup-slug
nyxid pool health stable-chat --method POST --path /chat/completions
nyxid pool reset-health stable-chat --service backup-slug
nyxid pool set-failover stable-chat --retry-on 429,5xx,timeout,node_offline --max-attempts 3
nyxid pool set-failover stable-chat --defaults
nyxid pool set-strategy stable-chat priority --tier-balance weighted
```

For org ownership, add `--org <UUID|slug|name>` to each management command. The
CLI resolves the pool within that owner; member slugs resolve within the pool's
owner. Member JSON is an array of objects with `user_service_id` (UUID or owned
slug), `priority`, `weight`, `enabled`, optional `model`, and
`same_api_compatible`. Lower priority goes first. Weights apply within a tier
when `tier_balance` is `weighted`.

`same_api` requires matching methods/paths/wire format. Custom or different
catalog APIs need an explicit declaration on each affected member, and known
protocol mismatches remain invalid. Do not claim that equal protocol alone proves
API equivalence. `ai_chat` instead requires each member's model and authoritative
catalog inference metadata, then translates the stable OpenAI chat contract.

Use one `update` JSON object for contract/model/member changes. Include the
`expected_revision` read with the draft; a 409 requires reloading and reviewing.
Omitted values preserve existing settings; null clears nullable values. Null
`failover` restores priority defaults. `set-failover --disable` means one attempt.
`add-member` also edits existing members; omitted flags preserve their values.
Use `--priority`, `--weight`, `--enabled true|false`, `--model`, `--clear-model`,
and `--same-api-compatible true|false`. Use atomic update JSON for contract/member
model changes, pool `is_active`, or multiple settings together.

`set-failover` inline flags merge only supplied fields into one saved snapshot and
send one PUT with that snapshot's pool ID and revision. A 409 requires reloading;
the CLI never retries with a newer revision. Available flags: `--retry-on`,
`--max-attempts`, `--per-attempt-timeout-ms`, `--overall-deadline-ms`,
`--max-replay-body-bytes`, `--retry-ambiguous-dispatch`, `--cooldown-base-ms`,
`--cooldown-max-ms`, `--cooldown-failures-to-open`, `--honor-retry-after`.
Individual cooldown flags preserve other cooldown fields. Booleans accept a bare
flag for true or `=false`; `--retry-non-idempotent` aliases the explicit ambiguity
opt-in. It can duplicate work/charges; selecting `5xx` never enables it implicitly.

Retry causes accept canonical names `connect_error,node_offline,transport_error,
timeout,http_401,http_403,http_408,http_429,http_500,http_502,http_503,http_504,http_529`
and numeric aliases `401,403,408,429,500,502,503,504,529`. `5xx` expands **only** to
`http_500,http_502,http_503,http_504,http_529`. Duplicates are removed; unknown/unsupported
codes are errors. `--retry-on none` alone clears the list. JSON uses canonical names.
`--file policy.json` replaces the saved override; omitted fields use server
defaults rather than saved custom values. `--defaults` (or a null file) clears the
override. `--disable` replaces it with `{"max_attempts":1}`; inline
`--max-attempts 1` retains other settings. These modes cannot be combined, and no
mode/setting is a usage error. All modes support `--org` owner resolution.

Candidates support `--pool`, `--contract`, `--strategy`, `--method`, `--path`,
`--search`, `--limit` and `--after`. Follow pagination. Omitting contract/path for
an existing AI pool derives its chat operation. `show` includes member cooldowns.

## Calling a pool

```bash
nyxid proxy request stable-chat /chat/completions -m POST \
  -d '{"model":"provider-model","messages":[{"role":"user","content":"Hello"}]}'
```

Raw HTTP uses `/api/v1/proxy/s/{pool_slug}/{path}`. AI pools also accept
`POST /api/v1/llm/gateway/v1/chat/completions` with
`{"model":"pool:<slug>","messages":[...]}`. Discover authorized aliases through
`GET /api/v1/llm/pools`, following `next_offset`. A gateway `llm:proxy` credential
does not grant generic REST slug access.

Priority defaults try at most 3 members, within 60 seconds per attempt and
120 seconds overall. They retry configured connect/node failures and upstream
429/502/503/504/529/timeouts subject to dispatch safety. POST timeout/5xx replay needs
explicit `retry_ambiguous_dispatch`; it can duplicate work and charges. Local
approval/admission/billing errors and caller cancellation are terminal. Once
response data reaches the client, no backup stream is appended. Oversized replay
bodies make one attempt. Round-robin/weighted pools remain single-attempt.

Provider failures create operation/credential-scoped cooldown. All-cooled returns
503 and Retry-After. Reset is owner-authorized and makes no provider request.
The normal scope, org membership, native policy, approval and live platform-grant
gates apply to the exact member before decryption. Never broaden a key to make
an excluded member eligible. Node priority routing needs live `http_cancellation`
support; upgrade and reconnect old agents.

Every attempt has independent billing; reported consumption is charged even when
a later member succeeds. Unknown failed usage remains unknown. Async Location
URLs bind the selected member with normal `_nyxid_via` authorization; follow those
URLs unchanged instead of replacing the member slug with the pool slug.

Before rollout, upgrade all replicas and clients. Before a downgrade, convert or
delete **every priority pool**, including disabled pools. Disabling alone cannot
make old backends deserialize the new strategy. Exact-billing migration downgrade
restrictions apply separately. See `docs/SERVICE_POOLS.md` for policy fields and
`docs/SERVICE_POOL_ROUTING_PROOF.md` for lifecycle and persistence details.

AI member base URLs: a root URL appends `v1/<native operation>`; an explicit path
is an API prefix and appends only the native operation (including `/openai`,
`/v1beta/openai`, deployment prefixes and `/backend-api/codex`). Configured URLs
and custom User-Agent remain authoritative. Anthropic defaults omitted token
limits to 4096. Same API declarations cannot override Responses/Completions
protocol mismatch; use supported `ai_chat` translation.

Budgets include preparation and approval. Pre-dispatch timeout is terminal and
does not cool the member. Health failures require configured causes, separately
from method replay safety. Caller cancellation and lease loss do not cool a
healthy provider. Retry-After cannot shorten configured backoff. `--disable`
means one eligible attempt; priority and cooldown still choose that member.
Inspect the displayed method/path: Same API `POST /` is not global health.
All-cooled responses use 503 plus Retry-After; deadline expiry uses 504 with safe
attempt summaries. Pool accounting unavailability uses 503 and stops fallback.
