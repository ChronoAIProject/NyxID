# Chat Assistant Integration Specs

Status: agreed direction, 2026-07-17 (rev 2 — same day; rev 1 assumed zero backend
changes, rev 2 allows small core-feature backend additions per Calvin).
Owner: Calvin.
Scope: integrate the two Aevatar endpoint groups — the `nyxid-chat` chat surface
(reference implementation: <https://github.com/eanz17/nyxid-chat>) and the Studio
**Chat History API** — into the NyxID assistant, through admin services.
Provenance: codex `gpt-5.6-sol` read-only integration scan, 2026-07-17
(20 findings; session `019f6eaa-71e6-76d1-b612-c1dfa3929615`). Findings referenced
below as F1–F20.

## Live prod verification (2026-07-17, Calvin's token, admin by-id path `a8e4314c…`)

All probes hit the admin `aevatar` service through the generic by-id proxy with a
Bearer token (the path the shipped mount uses server-side). Results:

- **chat-history INDEX** (`GET …/chat-history`) → `200`, exact contract shape:
  `{conversations:[{id,title,serviceId,serviceKind,createdAt,updatedAt,messageCount,
  llmRoute,llmModel}]}`, real server titles, sorted, lists only **materialized**
  conversations (1 returned vs the actor index's 3). Titles are server-owned — the
  frontend's title fan-out is now dead weight.
- **chat-history DETAIL** (`GET …/chat-history/conversations/{id}`) → `200`, exact
  `StoredChatMessage` shape; a nonexistent id returns `200 []` (never 404), confirming
  the empty-array-as-missing rule.
- **`/api/chat` history-write** — POSTed with `chatHistory:{conversationId,turnId,
  userText}` and **no `scopeId` in the body**. It streamed `200`, and the turn
  materialized in **my own** chat-history within one poll; my `turnId` became the
  `{turnId}:user`/`:assistant` message-id prefix. **Conclusion: Aevatar derives the
  history-write scope from the identity token NyxID forwards, not from a body
  `scopeId`. The safe path works; no scope smuggling is required to persist.**
- **composite delete** — `/api/chat` created a chat-history row but **no** nyxid-chat
  actor (actor delete → `404`, history delete → `200`). Confirms a composite delete
  must tolerate `404` on either side.
- **Bearer works, cookie is the only 401 source** — every probe with the Bearer
  returned `200`, reconfirming TD-3 is browser-cookie-specific.

Still unverified (genuine human/second-account boundaries): the **cross-scope
security** question (can a body `scopeId` *override* the derived scope for another
user — needs a second account, cannot be tested with one token) and the browser
**cookie-401** fix (needs the Aevatar identity-token change).

## Implemented in cut 3 (2026-07-17 late — first-send fix, stream correctness, composite delete)

Reference alignment pass against `eanz17/nyxid-chat` (latest: first-party
sessions + proxy-only identity handling). Changes:

- **Backend:** `POST /assistant/conversations` now waits for the created actor
  to appear in the `nyxid-chat` actor index (bounded backoff, ~2.0s of sleeps worst case,
  best-effort — mirrors the reference `waitForConversation`) so an immediate
  first stream cannot race the async-202 materialization.
  `DELETE /assistant/conversations/{id}` is now the composite dual-delete
  (actor + chat-history row, 404-tolerant on each side, 200 `{}` on success).
- **Frontend:** first send from the "New chat" empty state auto-creates a
  conversation and streams into it (previously a silent no-op: no selected
  conversation → internal throw → composer swallowed it — the "chat looks
  dead" prod symptom). Send failures now toast (`describeSendFailure`).
  Stream handling: pre-SSE `{code,message}` / `{error,error_code,message}`
  envelopes surface as the turn error (401/403 keeps the auth-specific,
  you-are-still-signed-in copy); EOF without `RUN_FINISHED`/`RUN_ERROR` is a
  failed `stream_closed` turn (was: silently reported success); the final
  unterminated SSE frame is flushed; bare-`\r` framing normalized.
  *(Correction, 2026-07-28: this cut originally described a per-conversation
  `sessionId` on the `:stream` body. That field was never shipped, and the
  current Aevatar contract deprecates and ignores it — the body is
  `{type:"text", prompt, clientRequestId}`; see cut 6.)*
- **Reference-mandated aevatar row config** (README, nyxid-chat):
  `identity_propagation_mode:"jwt"`, `identity_jwt_audience:"urn:aevatar:api"`,
  `inject_delegation_token:true`, `forward_access_token:false`. Prod row drift
  as of 2026-07-17: audience empty, forward true, inject false — flip together
  with the Aevatar-side identity-token validation (TD-3), NOT before (Bearer
  forwarding is what keeps CLI callers working today).

## Implemented in this cut (2026-07-17, uncommitted, gates green)

- **Backend:** added `history_index_path()` (`assistant_service.rs`) and repointed
  `list_conversations` (`handlers/assistant.rs`) so `GET /api/v1/assistant/conversations`
  now reads the chat-history index. Public URL unchanged; scope still server-derived.
  Path-builder test extended. `cargo test` (3/3 assistant tests) + build green.
- **Frontend:** `Conversation` type gained optional `message_count` / `llm_route` /
  `llm_model`; `AevatarAssistantTransport.listConversations` now folds contract-B
  index rows into the mirror via `mergeIndexEntry` (server titles/timestamps/counts,
  **no per-conversation fan-out**, live turns preserved). Removed `seedPlaceholder`
  and `MAX_TITLE_HYDRATIONS`. Tests updated (no-fan-out + live-title cases). 1868 FE
  tests + lint (0 errors) + build green; wizard bundle byte-identical (no dashboard
  chunk reaches it).
- **`/api/chat`:** already wired end-to-end and proven to write history. NOT changed
  in this cut — enabling persistence on the workflow transport is a product decision
  (see "Enable-ready" below), not a wiring gap.
- **Single production transport (toggle removed 2026-07-17).** The sidebar "Chat API"
  mode toggle and the alternate `completions` / `workflow` frontend transports (plus
  the mode store) were deleted; **nyxid-chat AG-UI is the sole production chat
  transport**. The backend `/assistant/completions` and `/assistant/workflow-chat[/ws]`
  pass-through routes are retained (thin, unused by the UI) — remove them too if the
  surface should be minimal. The endpoint-map rows and workflow/completions references
  below describe those retained backend routes, not a user-facing mode.
- **Auth: real Bearer, no NyxID-side minting.** A brief interim mint-and-forward
  bridge (mint a token for cookie sessions) was added and then **reverted** on
  2026-07-17: the client must present a real Bearer instead — a `nyxid_ag_` agent
  key, or an OAuth Authorization-Code + PKCE access token carrying the aevatar
  RFC 8707 resource (re-minted from a `binding_id` via token exchange). The backend
  already supports all of that; see TD-3.

## Implemented in cut 5 (2026-07-23 — Aevatar PR #2923 transcript-shape alignment)

Aevatar PR #2923 wrapped the chat-history detail response in
`{messages, stateVersion}`, added a `conversation.minimumStateVersion`
continuation requirement to `POST /api/chat`, and added `stateVersion` to the
`aevatar.chat.context` SSE payload. No routes added or removed. What NyxID
changed (approach reviewed by codex `gpt-5.6-sol`, session
`019f8d1e-ee56-78b2-915d-2897fc5b64c0`):

- **Frontend (the only functional change).** `loadHistory` typed the body as
  `AevatarHistoryEntry[]` and called `.map` on it; the wrapper made that throw,
  and `getHistory`'s catch-all fallback served the index-merged empty mirror —
  so every conversation would have rendered **blank, with no error**. The
  transcript read now goes through one strict decoder accepting exactly the
  legacy array and the wrapper; anything else raises `AssistantProtocolError`.
- **Fallback narrowed.** `getHistory` no longer swallows every failure: a
  protocol error always surfaces, and a transient failure serves the mirror only
  when the mirror holds a real transcript. An index-only placeholder
  (`EMPTY_TURN_STATE`) can no longer be dressed up as a successful empty read.
- **`stateVersion` is ignored, deliberately** — not read, not validated, not
  stored; wrapped acceptance is keyed **only** on array-valued `messages`. It is
  the continuation watermark for `/api/chat`, and NyxID's production transport is
  `nyxid-chat/…:stream`, which has no such parameter. Storing it would be state
  nobody maintains (`mergeIndexEntry` rebuilds `StoredConversation` and would
  silently drop it); *requiring* it would turn a field with zero consumers into
  an outage. It becomes load-bearing only on an `/api/chat` migration.
- **Backend: no functional change.** `get_history` never parses the body; both
  shapes stream through `execute_proxy` unmodified. That shape-agnosticism is
  what lets Aevatar and NyxID deploy independently.
- **Legacy-array branch is intentional, bounded compat.** The deployed Aevatar
  shape could not be verified from this environment (unauthenticated probe →
  401), and the two services deploy independently: committing to one shape
  breaks chat either immediately or the moment Aevatar ships. Removal condition,
  recorded at the type: delete the array branch once **every** supported Aevatar
  environment is confirmed on the wrapper — not after one prod probe.
- **Not done (out of scope):** migrating the production transport from `:stream`
  to `POST /api/chat` + `aevatar.chat.context`. That is a transport rewrite and
  belongs with `CHAT_REWORK_SPEC.md`.

## Implemented in cut 6 (2026-07-28 — `feature/integrate` alignment: `:stop` pass-through)

The authoritative Aevatar contract is now `aevatarAI/aevatar` branch
`feature/integrate` (`docs/canon/nyxid-chat-api.md` + the
`agents/Aevatar.GAgents.NyxidChat` host), superseding the `eanz17/nyxid-chat`
sample. A full conformance pass (2026-07-27) against that branch found the
stream-body `type` discriminator gap (fixed in #1251: body is
`{type:"text", prompt, clientRequestId}`; `sessionId` is deprecated and
ignored upstream) and the missing stop control. This cut adds the stop:

- **Backend:** `POST /api/v1/assistant/conversations/{id}/stop` →
  `…/nyxid-chat/conversations/{id}:stop` (`stop_path` in
  `assistant_service.rs`, `stop_turn` in `handlers/assistant.rs`, same
  `forward()`/`execute_proxy` funnel, body forwarded verbatim). Aevatar
  answers `202 {status:"accepted", requestId, commandId, correlationId,
  stateUrl}` and commits a stop fence: no later old-plan LLM round or tool
  may start.
- **Frontend:** `cancelTurn` and the stream watchdog now fire a best-effort
  `POST …/stop` with `{turnId, stopRequestId, clientRequestId,
  expectedStateVersion: 0}` (fresh UUID control identities;
  `expectedStateVersion <= 0` skips the optimistic-concurrency fence
  upstream, which the transport does not track). Failures are swallowed —
  the pre-existing client-abort behavior is the floor. The approval pause
  (`pauseForApproval`) deliberately does **not** stop: the server turn is
  waiting for the decision. Two codex (gpt-5.6-sol) P1s hardened the
  timing: (a) follow-up sends and the composite delete serialize behind
  the in-flight stop (bounded `STOP_FENCE_WAIT_MS`) so they cannot reach
  Aevatar before the fence commits; (b) a cancel that lands before
  `RUN_STARTED` defers its abort (bounded `PRE_START_STOP_WINDOW_MS`) so
  the announcing frame can still deliver the `turnId` the stop needs.
- **Known remaining `feature/integrate` gaps (deliberate, recorded):**
  `:steer`, per-step `:retry`/`:skip`, the conditional
  `GET …/conversations/{id}/state` query, the `nyxid.task.*` /
  `nyxid.action.request` CUSTOM frames (ignored by the FE), and the
  NyxID-owned `GET /api/v1/assistant/actions` registry (schema v4) with the
  `action.continue` stream body. The registry is default-disabled on the
  Aevatar side (`Aevatar:NyxId:AssistantActions:Enabled=false`, fails closed
  `NYXID_ACTION_UNSUPPORTED`), so its absence is compatible until the
  browser-action handoff is scheduled.

## Implemented in cut 4 (2026-07-18 — TD-3 interim bridge: marker-hardened forward token)

Resurrects the mint-and-forward bridge WITH the hardening whose absence killed
it (consensus: Claude Fable 5 + codex `gpt-5.6-sol`, 2 rounds, session
`019f7252-18b2-79f1-a875-e17278cf85fa`). The recorded objection was
"NyxID-audienced tokens are replayable at NyxID"; the resurrected token is not:

- `assistant::forward` mints an **outbound-only** access token for
  cookie-session callers (`AuthMethod::Session`) and overwrites
  `Authorization` before `execute_proxy`, so the existing
  `forward_access_token` machinery presents a real NyxID Bearer to Aevatar —
  the exact caller class that was 401ing, and the exact token shape the
  prod-green CLI/Bearer matrix proved.
- The token carries `assistant_forward: true`; `crypto::jwt::verify_token`
  (the shared validator under bearer auth, token exchange, MCP transport,
  introspection) rejects it with the generic invalid-token error, so a copy
  leaked from Aevatar cannot re-enter NyxID. Belt-and-braces: `resources`
  names only the canonical aevatar proxy URI, `allowed_service_ids` is empty
  with `allow_all_services: false`, TTL `JWT_ASSISTANT_FORWARD_TTL_SECS`
  (default 300s), scope `proxy`.
- **Kill switch = the TD-3 row flip.** Minting is gated on
  `service.forward_access_token == true`. When Aevatar ships identity-token
  validation and the row flips to `forward_access_token: false`, the bridge
  retires itself with no code change (no fail-fast on purpose).
- Bearer callers (CLI login JWTs) never enter the branch; their token is
  forwarded byte-for-byte as before. Agent keys remain rejected by the
  human-only mount. No frontend change.
- Residual (unchanged, recorded): TD-1 strict admin-target mode and the
  node-routed-WS `caller_token` gap — not blockers for the browser chat
  surface, which is direct HTTP through `forward()`.

## Implemented in cut 5 (2026-07-18 — cut-4 marker → delegated access token)

Cut 4 (PR #1200) fixed the chat-entry 401 but a real prod cookie chat then
failed at the NEXT leg: `RUN_STARTED` then `RUN_ERROR (401)`. Aevatar
authenticates its OWN callbacks to NyxID by **reusing the inbound
`Authorization: Bearer`** (the prod row has `inject_delegation_token: false`,
so it has no `X-NyxID-Delegation-Token`). When Aevatar called NyxID's LLM
gateway on the user's behalf (`/api/v1/proxy/s/chrono-llm-public`), the cut-4
`assistant_forward` token — rejected by `verify_token` *everywhere* — was
refused with `1001 "Invalid token"`. The "reject everywhere" marker was a
mis-design: it never accounted for Aevatar legitimately calling NyxID's
proxy/LLM surfaces, which is how chat actually does work. **This was a NyxID
self-inflicted bug, fixable entirely NyxID-side.**

Fix (consensus: Claude Fable 5 + codex `gpt-5.6-sol` rounds 5-7):
`assistant::forward` now mints a **standard delegated access token**
(`generate_delegated_access_token`, `delegated: true`, `act.sub = aevatar`,
scope from the row's `delegation_token_scope`, restrictions from
`TokenRestrictionClaims::from_auth_user`, TTL `MCP_DELEGATION_TOKEN_TTL_SECS`
= 300s) and overwrites `Authorization`.

**Scope sourcing — SSOT with a resilience floor (round-6 SSOT + round-7 deploy-safety):**
`resolve_forward_scope` PREFERS the row's `delegation_token_scope` — the same
single source of truth the standard `inject_delegation_token` path reads (an
earlier cut hardcoded `PROXY_SCOPE`, shadowing the config field; codex round 6
flagged that FI-003/FI-005 anti-pattern). But the callback needs REST-proxy
capability (`/proxy/s/{slug}` enforces `ensure_rest_proxy_access`), and the
historical row default is `llm:proxy`, which does NOT grant it. So the code
FALLS BACK to `PROXY_SCOPE` (with a `tracing::warn!`) when the row scope is
insufficient, rather than 500-ing the whole assistant over one config field —
the minimum capability is dictated by the integration, not a free per-row
choice, so this is a resilience floor, not a policy override. Net: **the
assistant works on deploy with the current prod row unchanged** (`llm:proxy`
→ falls back to `proxy`); setting `delegation_token_scope: "proxy"` on the row
is RECOMMENDED to silence the warning and keep the `Authorization` token
capability-aligned with the future `X-NyxID-Delegation-Token` (same scope; the
JWTs differ in `iat`/`jti`). Make the TD-3 cutover ONE atomic three-field row
update — `forward_access_token: false`, `inject_delegation_token: true`,
`delegation_token_scope: "proxy"` — and the runbook must read the row back and
smoke-test one LLM callback before declaring cutover done, because after
`forward_access_token: false` the bridge stops running and the standard
`inject_delegation_token` path passes the raw row scope straight to the
generator (an unchanged `llm:proxy` would then 403 the callback).
This IS the documented platform delegation-token standard ("downstream calls
NyxID on the user's behalf"), just delivered in `Authorization` (the header
Aevatar demonstrably reuses) rather than `X-NyxID-Delegation-Token`. Replay
boundary is now the router layer: `reject_delegated_tokens` refuses the token
on every human-only + shared surface (account, admin, keys → 403) while
`api_v1_delegated` accepts it on `/llm`, `/proxy/s/*`, `/delegation/refresh`.
Strictly safer than the plain full-access token CLI/Bearer callers already
forward and prod already trusts. The effective scope must satisfy
`scope_allows_rest_proxy` because the LLM call arrives as a REST proxy
passthrough enforcing `ensure_rest_proxy_access` (`proxy` also satisfies the
`/llm/*` check); `resolve_forward_scope` guarantees that (row scope if
sufficient, else `PROXY_SCOPE` fallback).

- **Kill switch unchanged.** Gated on
  `AuthMethod::Session && service.forward_access_token`; the TD-3 row flip to
  `forward_access_token: false` (plus `inject_delegation_token: true`)
  retires the Authorization mint and hands over to the standard delegation
  header with no code change.
- **cut-4 tombstone (one deploy only).** `Claims.assistant_forward`, the
  `verify_token` rejection, `generate_assistant_forward_access_token`, and
  `JWT_ASSISTANT_FORWARD_TTL_SECS` are RETAINED this deploy: a cut-4 token
  minted just before rollout may still be live at Aevatar (≤300s + skew), and
  removing the rejection would let serde decode it as an ordinary access
  token → reopened replay hole. Follow-up issue: delete the whole marker
  machinery after all pre-migration tokens have expired (>10 min post-deploy).
- **E2E-proven — the PRIMARY dashboard conversation flow — with the current
  prod row unchanged** (`delegation_token_scope: "llm:proxy"`, so the fallback
  is exercised): list → create → stream (Aevatar replays the forwarded bearer
  to a seeded `chrono-llm-public` → mock LLM, callback 200) → history →
  composite delete, every endpoint 200/`RUN_FINISHED`. Also proven: the same
  token is rejected 403 at `/users/me`, `/api-keys`, `/connections`; the
  `forward_access_token` flip suppresses the mint; Bearer callers forward
  byte-for-byte; a `proxy:*` row scope reaches the JWT verbatim (SSOT when
  configured). NOT covered by this E2E (see the enumerated gaps below): the
  in-chat approval leg (TD-7 frontend), completions, workflow-chat, and the
  node-routed WS path.
### What works today vs. the enumerated remaining gaps (codex round-8 whole-flow audit)

**WORKS TODAY, current prod row, zero config change** — the core "ask the
assistant, get a streamed answer" experience: list, create, stream (incl. the
Aevatar→NyxID LLM callback), history, delete. All nine routes now *authenticate*
correctly through the bridge.

**Remaining gaps (recorded, NOT blockers for basic Q&A chat) — the full list so
none surface as a surprise "next broken leg":**

- **G1 [P1, frontend TD-7] In-chat approval/authorization not end-to-end.** The
  AG-UI normalizer renders only text/`RUN_ERROR`/`RUN_FINISHED`; `approval_card`
  and `AUTHORIZATION_REQUIRED` frames hit the default discard branch, and
  `decideApproval` uses the JSON `post` helper while Aevatar's `:approve`
  responds with SSE. The backend endpoint authenticates; the browser cannot yet
  surface/complete a real approval. Only bites when the assistant performs a
  NyxID-gated write. Fix = the TD-7 protocol-module port.
- **G2 [P2, TD-1] Caller-state can still divert/deny.** `execute_proxy` still
  runs per-user node routing and rejects an inactive legacy Aevatar
  `UserServiceConnection`, so one historical disconnected row = "works for
  everyone but me". Fix = strict admin-target execution mode.
- **G3 [P2] Node-routed workflow WS loses the bearer.** `/workflow-chat/ws` via
  a user node pin: the node WS path has no `caller_token` and never injects the
  forwarded bearer → 401. Direct WS is fine. Subsumed by G2's strict admin mode.
- **G4 [P2] Long-run callback refresh.** The forwarded delegated token is 300s;
  `/delegation/refresh` expects `act.sub` to be an active `OauthClient`, which
  `"aevatar"` is not, so a run that issues its first callback after 5 min can
  fail. Immediate callbacks work.
- **G5 [P2, TD-10] Workflow approval resume.** No `runs/{runId}:resume`
  pass-through; a paused workflow can't be resumed through this mount.
- **G6 SSE idle timeout.** A silent gap > `PROXY_STREAM_IDLE_TIMEOUT_SECS` (60s)
  kills the stream with no error frame; safe only if Aevatar keepalives < 60s.
- **G7 chrono-llm-public provisioning.** The callback's scope now passes, but the
  service still resolves a caller `UserService` first then the active catalog
  row — its endpoint + credential provisioning remains a runtime dependency
  (confirm it's an active public/master-credential row, class-equal to aevatar).

## Enable-ready (proven, awaiting a decision — not blocked on code)

> **Superseded in part by Aevatar PR #2923 (see the cut below).** The
> `chatHistory:{conversationId,turnId,userText}` intent described here is no
> longer the whole story: continuation on `/api/chat` now also requires
> `conversation:{conversationId, minimumStateVersion>0}`, and the caller must
> **not** splice transcript into `prompt` — Aevatar injects server history into
> the workflow context itself. The product decision below is unchanged.

`/api/chat` history-write is verified working. To make workflow-mode (or any
`/api/chat`) turns persist and appear in the unified history list, the frontend
must send the history-write intent on the run (plus the #2923 continuation
watermark above). Deferred because it changes product behavior (workflow chats
currently reset on reload by design, per Calvin's earlier waiver) and would
surface workflow conversations in the nyxid-chat history list but not the
workflow transport's own in-memory list. Decision needed: should
workflow/`/api/chat` chats persist?

## Guiding constraints (Calvin, 2026-07-17)

1. Integration must be good, quick, and **up** (live in prod).
2. Backend additions are allowed when they introduce a **core feature**. Good-to-have
   hardening is deferred to the BE engineer — recorded in TECH_DEBT below with
   owners/triggers, not silently dropped.
3. Minimal breaking changes; keep the existing `/api/v1/assistant/*` mount and both
   frontend transports.

## Architecture

- **Upstream**: the admin-seeded `aevatar` `DownstreamService` (catalog slug
  `aevatar`; internal / public / master-credential / `requires_user_credential:
  false` / identity `jwt` / `forward_access_token: true`). One upstream for all chat
  and history traffic.
- **Single client surface**: everything goes through the `/api/v1/assistant/*` mount.
  The server derives the Aevatar scope from the verified session user; no client ever
  names a scope. The rev-1 idea of calling history endpoints through the generic
  by-id proxy (`/api/v1/proxy/{id}/api/scopes/{uid}/...`) is dropped for the browser:
  it would have put the scope in a caller-controlled path. It remains a valid interim
  CLI recipe (see Phase C) because `proxy_service.rs:1538` admits any authenticated
  user to public+internal master-credential services.
- **Two core backend additions** (the only ones in scope; both are one-line-ish path
  remaps inside the existing mount, same pattern as the current 9 routes):
  1. `GET /api/v1/assistant/conversations` switches its upstream from the
     `nyxid-chat` actor index to the **chat-history index**
     (`api/scopes/{uid}/chat-history`). This is what makes server titles, timestamps
     and message counts exist at all — core to the history feature.
  2. `DELETE /api/v1/assistant/conversations/{id}` becomes a **composite delete**:
     `DELETE .../nyxid-chat/conversations/{id}` (actor) **and**
     `DELETE .../chat-history/conversations/{id}` (history row), tolerating 404 from
     either, returning 200 empty body. Matches the reference BFF's dual-delete
     (`server.mjs:~727`). Without this, "deleted" conversations resurface in the
     index — core to delete working at all.
  No new public routes; the public URL surface is unchanged.
- **CLI**: target state is a thin `nyxid assistant` subcommand hitting the mount with
  the normal `nyxid login` session JWT (the mount is human-only: `nyxid_ag_` API
  keys, service accounts, delegated and relay tokens are rejected — F8). Interim:
  `nyxid proxy request` recipes against the generic by-id proxy work in prod today.

## Endpoint map (after the two additions)

| Client call | Upstream Aevatar path | Notes |
|---|---|---|
| `POST /api/v1/assistant/conversations` | `api/scopes/{uid}/nyxid-chat/conversations` | create actor |
| `GET /api/v1/assistant/conversations` | `api/scopes/{uid}/chat-history` | **changed** — contract-B index |
| `GET /api/v1/assistant/conversations/{id}` | `api/scopes/{uid}/chat-history/conversations/{id}` | unchanged |
| `DELETE /api/v1/assistant/conversations/{id}` | actor delete **+** history-row delete | composite — **implemented** (cut 3) |
| `POST /api/v1/assistant/conversations/{id}/stream` | `...:stream` (AG-UI SSE) | unchanged |
| `POST /api/v1/assistant/conversations/{id}/approve` | `...:approve` | unchanged |
| `POST /api/v1/assistant/conversations/{id}/stop` | `...:stop` | **added** (cut 6) — 202-accepted stop fence |
| `POST /api/v1/assistant/completions` | `v1/chat/completions` | unchanged |
| `POST /api/v1/assistant/workflow-chat` | `api/chat` | unchanged; body forwarded verbatim (TD-2) |
| `GET /api/v1/assistant/workflow-chat/ws` | `api/ws/chat` | unchanged (TD-2) |

## Chat History contract rules the frontend must honor

From the Studio Chat History API contract:

- Index: `{"conversations": [...]}` sorted by `updatedAt` desc; fields `id, title,
  serviceId, serviceKind, createdAt, updatedAt, messageCount, llmRoute?, llmModel?`.
  No pagination. Titles come from the server — no client-side title synthesis.
- Detail: **two accepted shapes** — the legacy flat `[StoredChatMessage]`, and
  Aevatar PR #2923's `{messages: [StoredChatMessage], stateVersion}` wrapper.
  Entry shape is identical in both: ids are `{turnId}:user` / `{turnId}:assistant`,
  `status` is `complete|error`. Acceptance is keyed **only** on array-valued
  `messages`; `stateVersion` is ignored by this transport (see cut 5). Anything
  that is neither shape is a protocol error and must surface, not degrade to empty.
  **An empty transcript is a valid answer** (`[]` or `{"messages":[]}`) meaning
  deleted / not yet materialized / zero turns — never treat it as an error. It
  replaces cached state *except* under the deliberate keep-max guard: for a few
  hundred ms after a turn completes the read model lags the local mirror, and a
  shorter server answer there is staleness, not truth.
- Delete: `200` with empty body; the read model is eventually consistent — the row
  may briefly reappear in the index. Use optimistic removal + a short client-side
  tombstone.
- No rename, no direct conversation create, no standalone message-write API. The only
  backend write path is `POST /api/chat` with the optional `chatHistory`
  `{conversationId, turnId, userText}` intent (all three trim-nonempty or the server
  returns 400 `INVALID_CHAT_HISTORY`). `nyxid-chat` conversations persist history
  server-side on their own; the intent only matters for workflow-chat runs.
- **Continuation on `/api/chat` (PR #2923).** A new conversation passes
  `conversation:{conversationId:null}`. Continuing an existing one **must** pass
  `conversation:{conversationId, minimumStateVersion>0}`, where the watermark is the
  `stateVersion` last read from the detail endpoint or from the SSE
  `aevatar.chat.context` payload (which gained the field). Behind the watermark →
  `503 CHAT_HISTORY_RESERVATION_UNAVAILABLE` (re-read the conversation, then retry);
  unknown conversation → `404 CONVERSATION_NOT_FOUND`. Callers must stop splicing
  local transcript into `prompt` — `prompt` is this turn's user input only, and the
  server injects the history context into the workflow run.
- Errors before the SSE stream starts are a JSON envelope `{code, message}`; errors
  after the stream starts arrive as SSE frames. Parse them separately.

## Work plan

**Phase 0 — prod probes (no code, run against the generic proxy which exists in prod
today).**
1. **Cross-scope probe: user A's token against user B's `{uid}` in the path.**
   Expected: 403/empty. This calibrates how urgent TD-2 is — with the mount-only
   design the browser can't exploit it, but the workflow-chat body and the generic
   proxy still can. If Aevatar does not enforce token-subject == scope, escalate
   TD-2 to the BE engineer as a blocker rather than debt.
2. Delete a conversation, measure index-lag until it disappears; confirm detail
   returns `[]` afterward.
3. Confirm a non-admin user's token passes the by-id proxy (expected yes per
   `proxy_service.rs:1538`; needed for the interim CLI recipes).

**Phase A — backend (core additions only).**
1. `history_index_path()` builder + remap `list_conversations` to it.
2. Composite delete in `delete_conversation` (two upstream calls, 404-tolerant,
   200 empty body).
3. Update the path-builder unit tests (`assistant_service.rs` tests).
Everything else backend-shaped is TECH_DEBT for the BE engineer.

**Phase B — frontend (parallel with A; verifiable via the `vite.config.e2e.mts`
harness).**
1. Conversation list consumes contract-B index DTOs (server titles/updatedAt/
   messageCount; llmRoute/llmModel optional). Delete the up-to-20-request title
   fan-out (F13).
2. Delete: optimistic removal + short tombstone against eventual consistency (F15).
3. ~~Fix the keep-max guard so a legitimately empty transcript replaces cached
   state (F14).~~ **Not done, deliberately** — the keep-max guard is load-bearing
   against post-turn materialization lag (the server briefly answers shorter than
   the local mirror). See the empty-transcript rule under "Contract facts".
4. Parse pre-SSE `{code, message}` envelopes instead of collapsing to
   `http_<status>` (F16).
5. (Optional, only if workflow-mode history is wanted now) send `chatHistory`
   intent from the workflow transport; otherwise workflow runs stay unpersisted.

**Phase C — CLI.**
1. Now: document `nyxid proxy request` recipes (history index/detail/delete,
   `:stream` with `--stream` + explicit `Content-Type: application/json`,
   `api/chat`).
2. After A freezes: thin `nyxid assistant` subcommand (`chat`, `history
   list|show|delete`, `approve`) hitting the mount with the login JWT. CLI-repo
   change, not a backend change.

**Phase D — prod enablement ("up").**
1. Deploy the branch (assistant mount + the two Phase-A changes).
2. **External dependency, start now (longest lead time):** Aevatar team validates
   `X-NyxID-Identity-Token` (NyxID already sends it); then flip the admin row to
   `forward_access_token: false` (Mongo config, no NyxID code). Until this lands,
   browser cookie sessions get 401 from Aevatar on every pass-through surface
   (TD-3). CLI/Bearer callers are unaffected.
3. Post-deploy E2E with a **real cookie session** — the Bearer-injecting dev harness
   structurally cannot detect the cookie-401 class.

## Possible issues (verify list)

- **Aevatar scope enforcement** — Phase-0 probe 1; calibrates TD-2.
- **60s proxy idle timeout** (`PROXY_STREAM_IDLE_TIMEOUT_SECS`, F19): nyxid-chat
  emits one big delta after ~9s silence and workflow-chat ~5s; long runs may exceed
  60s of silence and get the stream killed with no error frame. Confirm Aevatar
  keepalive frames under load.
- **Eventual consistency** on delete/index (contract-documented; measure the lag).
- **`nyxid-chat` does not token-stream** (one `TEXT_MESSAGE_CONTENT` per answer);
  completions mode streams token-by-token. UX lever, not a blocker.

## TECH_DEBT

Good-to-have items consciously deferred on 2026-07-17; default owner = BE engineer
unless noted. Each item: what, risk, trigger.

- **TD-1 — Admin-path isolation leak (F1).** `execute_proxy` still runs caller-state
  resolution even for catalog-id addressing: a personal node pin
  (`resolve_node_route`) or an inactive `UserServiceConnection` ("You have
  disconnected from this service", `proxy_service.rs:~460`) changes or breaks
  assistant traffic for that one user. Fix: strict admin-target execution mode that
  bypasses UserService / node routing / legacy connections. Trigger: before GA, or
  the first "chat works for everyone but me" report.
- **TD-2 — Scope smuggling (F4, F5, F6).** (a) `/assistant/workflow-chat` forwards
  the body verbatim — a caller can send `scopeId` / `chatHistory.conversationId`;
  identity-JWT generation failures log and continue. (b) The WS twin bridges frames
  and cannot be validated at all. (c) The generic by-id proxy (used by interim CLI
  recipes) puts scope in a caller-controlled path. All rely on Aevatar enforcing
  token-subject == scope; Phase-0 probe 1 measures the real exposure. Fix: typed
  `ChatInput` DTO with server-forced scope; fail closed on identity generation; drop
  or gate the WS route. Trigger: probe failure = immediate blocker; otherwise before
  GA.
- **TD-3 — Browser auth: Aevatar validates the identity token (superseded
  "real Bearer" plan, 2026-07-17 reference alignment).** The reference client
  (`eanz17/nyxid-chat`) DELETED its developer-app OAuth/binding flow entirely
  (`/api/auth/authorize` → 410) and lands on exactly our dashboard's model:
  same-site session cookie → NyxID proxy → proxy injects short-lived
  `X-NyxID-Identity-Token` (aud `urn:aevatar:api`) + `X-NyxID-Delegation-Token`;
  Aevatar validates the identity token against NyxID's JWKS (issuer, audience,
  expiry), derives the scope from `sub`, re-checks path-scope == `sub` (403
  mismatch), and uses the delegation token to call NyxID APIs/LLM/tools on the
  user's behalf mid-run. Both prior "client presents a real Bearer"
  alternatives are structurally broken and are dropped:
  - `nyxid_ag_` agent keys are rejected by the assistant mount's human-only
    router (`routes.rs` reject layers) — they never reach the proxy here.
  - Resource-scoped OAuth tokens die in the legacy catalog branch's
    scoped-token check (`proxy.rs` "Scoped API keys must use configured
    services") before contacting Aevatar.
  ~~The mint-and-forward bridge stays dead (NyxID-audienced tokens are
  replayable at NyxID).~~ **Superseded by cut 4 (2026-07-18):** the bridge is
  back with the replay objection removed — the minted token carries
  `assistant_forward: true`, which `verify_token` rejects on any NyxID
  re-entry, so it is validatable by Aevatar but worthless at NyxID. Browser
  cookie sessions chat TODAY without waiting on the Aevatar rollout.
  Rollout (unchanged, and it doubles as the bridge kill switch): (1) Aevatar
  team validates `X-NyxID-Identity-Token` — the header already flows on every
  pass-through call; (2) flip the aevatar row to
  `identity_jwt_audience:"urn:aevatar:api"`, `inject_delegation_token:true`,
  `forward_access_token:false` (three-field drift from today's prod row).
  The `forward_access_token:false` flip also stops the cut-4 minting (gated
  on that field). Do NOT flip before (1): Bearer forwarding is what keeps CLI
  callers AND the cut-4 browser bridge working today.
- **TD-4 — Feature flag not enforced server-side (F17).** Any authenticated human can
  call `/api/v1/assistant/*` with `experimental:ai-assistant` off; the flag only
  hides navigation.
- **TD-5 — Upstream error bodies logged (F18).** Non-streaming proxy failures log up
  to 1 KiB of the response body (`proxy.rs:~2741`), which may contain prompt/history
  content — violates the metadata-only posture for chat paths.
- **TD-6 — No chat-shaped limits (F19).** No per-user concurrent-stream cap, 100 MiB
  request body cap, no per-agent rate limit for session callers, and the 60s idle
  kill produces no application error frame.
- **TD-7 — Frontend protocol gaps vs the reference (F9–F12, F20).** Owner: frontend.
  The AG-UI transport renders text only: `TOOL_APPROVAL_REQUEST`, authorization,
  tool, usage, reasoning, keepalive and all `CUSTOM aevatar.*` frames are dropped;
  there is no credential/reasoning redaction (reference `protocol.js` scrubber
  unported); mock artifact counts still render. (Fixed in cut 3: EOF without
  `RUN_FINISHED` now fails the turn; pre-SSE error envelopes surface.) Two
  reference-confirmed additions for this item: `:approve` responds with an SSE
  stream — `decideApproval` currently JSON-parses and will break the moment
  approval cards render; and `AUTHORIZATION_REQUIRED` must render a
  service-config card that preserves the failed request for an EXPLICIT retry
  only (never auto-retry a partially-executed run). Fix: port the reference
  normalizer + redactor into one shared protocol module. Trigger: before
  approvals/tool-use become user-visible chat features.
- **TD-8 — `resolve_admin_service` under-validates provisioning (F2).** It checks
  slug + `requires_user_credential:false` only — not `service_category`, master
  credential, or `identity_propagation_mode`; a misconfigured row degrades silently.
- **TD-10 — Workflow approvals impossible (F10).** No `runs/{runId}:resume`
  pass-through exists and the workflow transport throws on approvals. Add the route
  when workflow mode graduates from experiment to product.
- **TD-11 — Rename (F15, note not debt).** No upstream contract supports renaming a
  conversation. Do not build client-side rename state; wait for an explicit Aevatar
  contract.

(TD-9 from rev 1 — dedicated CLI subcommand — was promoted into the plan as
Phase C.2.)
