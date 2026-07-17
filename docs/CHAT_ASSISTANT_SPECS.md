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
  to appear in the `nyxid-chat` actor index (bounded backoff, ~1.4s worst case,
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
  unterminated SSE frame is flushed; bare-`\r` framing normalized; every
  `:stream` body carries a per-conversation `sessionId` (optional upstream,
  reference-aligned).
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

## Enable-ready (proven, awaiting a decision — not blocked on code)

`/api/chat` history-write is verified working. To make workflow-mode (or any
`/api/chat`) turns persist and appear in the unified history list, the only change is
the frontend sending `chatHistory:{conversationId,turnId,userText}` (all trim-nonempty)
on the run. Deferred because it changes product behavior (workflow chats currently
reset on reload by design, per Calvin's earlier waiver) and would surface workflow
conversations in the nyxid-chat history list but not the workflow transport's own
in-memory list. Decision needed: should workflow/`/api/chat` chats persist?

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
| `POST /api/v1/assistant/completions` | `v1/chat/completions` | unchanged |
| `POST /api/v1/assistant/workflow-chat` | `api/chat` | unchanged; body forwarded verbatim (TD-2) |
| `GET /api/v1/assistant/workflow-chat/ws` | `api/ws/chat` | unchanged (TD-2) |

## Chat History contract rules the frontend must honor

From the Studio Chat History API contract:

- Index: `{"conversations": [...]}` sorted by `updatedAt` desc; fields `id, title,
  serviceId, serviceKind, createdAt, updatedAt, messageCount, llmRoute?, llmModel?`.
  No pagination. Titles come from the server — no client-side title synthesis.
- Detail: flat `[StoredChatMessage]`; ids are `{turnId}:user` / `{turnId}:assistant`;
  `status` is `complete|error`; **an empty array is a valid answer** meaning
  deleted / not yet materialized / zero turns — never treat it as an error and never
  refuse to replace cached state with it.
- Delete: `200` with empty body; the read model is eventually consistent — the row
  may briefly reappear in the index. Use optimistic removal + a short client-side
  tombstone.
- No rename, no direct conversation create, no standalone message-write API. The only
  backend write path is `POST /api/chat` with the optional `chatHistory`
  `{conversationId, turnId, userText}` intent (all three trim-nonempty or the server
  returns 400 `INVALID_CHAT_HISTORY`). `nyxid-chat` conversations persist history
  server-side on their own; the intent only matters for workflow-chat runs.
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
3. Fix the keep-max guard so a legitimately empty transcript replaces cached state
   (F14).
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
  The mint-and-forward bridge stays dead (NyxID-audienced tokens are
  replayable at NyxID). Rollout: (1) Aevatar team validates
  `X-NyxID-Identity-Token` — the header already flows on every pass-through
  call; (2) flip the aevatar row to `identity_jwt_audience:"urn:aevatar:api"`,
  `inject_delegation_token:true`, `forward_access_token:false` (three-field
  drift from today's prod row). Do NOT flip before (1): Bearer forwarding is
  what keeps CLI callers working today.
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
