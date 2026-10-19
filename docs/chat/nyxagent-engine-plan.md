# NyxAgent assistant engine: retained design record

Status: retained design record (2026-09-17), superseded by the implemented
normative contract in [08-nyxagent-engine.md](08-nyxagent-engine.md).
This file preserves the original decisions and verification requirements.

## Goal

The browser assistant at `/assistant` currently sends every turn to the
platform-managed Aevatar service. Replace that default with NyxAgent
(<https://github.com/ChronoAIProject/NxyAgent>, clone at `/tmp/nxyagent`,
commit `58f647e4`), which is already registered on this NyxID deployment as the
public, auto-connected catalog service **`llm-nyx`** (a second public row
`llm-nyx-spec` only publishes its OpenAPI document). NyxAgent runs a Codex
agent loop with six NyxID MCP tools (`nyx__search_tools`, `nyx__call_tool`,
`nyx__discover_services`, `nyx__list_connected_services`,
`nyx__connect_service`, `nyx__wait_for_connection`), so through it the
assistant can inspect and use everything the user has connected on NyxID,
create hosted connect links for new services, help with channel bots, keys and
nodes, and call connected tools.

NyxAgent is deployed today with `STORAGE_BACKEND=memory` and no Redis: every
NyxAgent restart (including each upgrade) erases its sessions. The NyxID
integration must keep working across such restarts without user-visible
breakage beyond a transparent "context was reset" notice, and must keep
working unchanged when NyxAgent later gains S3/GCS + Redis.

## NyxAgent facts the design depends on (verified against the clone)

- `POST /v1/responses` (JSON or SSE), `GET /v1/responses/{id}`, `GET /v1/models`,
  `POST/GET/PATCH/DELETE /v1/sessions/{id}` (aliases under `/v1/conversations`),
  `GET /healthz`, `/readyz`, `/openapi.json`. See `docs/api.md` and
  `docs/openapi.json` in the clone.
- Authentication (`src/nyxid.rs::authenticate`): the request MUST carry
  `Authorization: Bearer <NyxID agent key>` whose value starts with `nyxid_`
  or `nyx_`. Any JWT (session, delegated, relay) is rejected with 401
  `agent_key_required`. NyxAgent validates the key by calling NyxID
  `POST /mcp` (`initialize`) with the key in `x-api-key`, and uses the same
  key for every tool call (`tools/call`) and for model inference through
  `NYXID_BASE_URL + /api/v1/proxy/s/chrono-llm-public/responses`.
- Session owner = HMAC(NYXID_BASE_URL, raw key). The **same raw key** must be
  presented on every turn of a conversation; a different key cannot see or
  continue the session. Key rotation therefore orphans NyxAgent sessions.
- Request body (closed set): `input` (string or array of user text messages),
  `model` (alias: `nyxagent/chat`, `nyxagent/research`, `nyxagent/channel`;
  omitted = session profile or default), `instructions` (per request, not
  carried forward), `stream`, `store`, `previous_response_id`, `conversation`
  (session id), `metadata`, `session_id`. Unknown fields fail with 400.
  `conversation` and `previous_response_id` cannot be combined. Only the
  current head can be continued; a stale head returns 409 `stale_response`.
  History cannot be injected as `input`; only `instructions` can carry a
  recap.
- `Idempotency-Key` header: a logical-turn key; retrying with the same key is
  safe; a changed body with the same key returns 409.
- Error codes: `session_busy` (retry with jittered backoff), `capacity_exceeded`
  (retry with the same idempotency key), `stale_response`, `outcome_unknown`
  (do not retry; start a new session), `not_found` (wrong binding, or
  erased/expired session, which is exactly what happens after a memory-mode
  restart), `agent_key_required`, `nyxid_error`, `model_not_configured`.
- SSE: `response.created`, `response.in_progress`, output item / content part
  lifecycle events, `response.output_text.delta`, `response.output_text.done`,
  `response.completed` | `response.failed`; monotonic `sequence_number`;
  15-second keepalives; completion is emitted after durable commit. Tool
  activity is not exposed as public output items. The completed
  `response` object carries `id` and `conversation.id`.
- `nyx__connect_service` (NyxID `services/mcp_service.rs::connect_service`)
  returns `{status:"pending_connection", connect_url, connect_link_id, ...}`;
  NyxAgent surfaces the URL to the user in text. The URL is a same-origin
  NyxID hosted connect link (`connect_link_service::build_connect_url`).
- Registration contract (`docs/nyxid-integration.md`): `auth_method: none`,
  `forward_access_token: true`, no delegation-token injection, no shared
  catalog credential, `requires_user_credential: false`. The deployment sets
  `NYXAGENT_SERVICE_SLUG=llm-nyx` for its recursion guard.

## Architecture decisions (do not re-litigate; implement)

### D1. NyxAgent becomes the default engine behind a flag that defaults ON

Add feature flag `assistant:nyxagent-engine` (backend registry +
`frontend/src/lib/feature-flags.ts`), `default_enabled: true`, description
"Routes assistant chat through NyxAgent (catalog slug llm-nyx) instead of
Aevatar." Engine precedence for a new draft or an unselected conversation:
NyxAgent (if enabled) > Direct (`experimental:direct-chat-engine`) > Aevatar.
Existing `nyxid-chat-*` and `chatc-*` conversations stay readable/deletable
through the retained Aevatar reader exactly as they do under the Direct flag.
The backend NyxAgent routes enforce the flag independently of the frontend
(not-found-shaped when disabled, mirroring `assistant_direct.rs`).

The Aevatar engine code is retained in this change as the flag-off fallback.
Do not delete it here; do not add any NyxAgent -> Aevatar or Aevatar ->
NyxAgent fallback at runtime.

### D2. Per-user system-managed assistant Agent Key, encrypted at rest

NyxAgent only accepts an original agent key and binds sessions to it, so NyxID
must present one stable key per user. Provision lazily on the first NyxAgent
turn:

- Create an ordinary `ApiKey` through `key_service` (the scope-authorized
  creation path), owned by the acting person (`user_id` = the human), with:
  name `NyxID Assistant`, `platform = "nyxid-assistant"`,
  `allow_all_services = true`, `allow_all_nodes = true`,
  `allow_auto_connected_services = true`, `purpose = General`, no expiry,
  and the scopes required for `/mcp` initialize + `tools/call`, the REST
  proxy (`proxy`), and the LLM proxy (`llm:proxy`) — determine the exact
  scope string from `mw/auth.rs` and `handlers/mcp_transport.rs` and test it.
- New collection `assistant_agent_credentials` (model + `COLLECTION_NAME`,
  UUID `_id`, unique index on `user_id`): `user_id`, `api_key_id`,
  `key_ciphertext: Vec<u8>` (raw key encrypted with `EncryptionKeys`, same
  pattern as `Trigger.verification_secret_encrypted`), `created_at`,
  `last_used_at`. Redacted `Debug`. This is the one deliberate exception to
  "only hashes of API keys are stored", justified by NyxAgent's contract;
  document it in CLAUDE.md Rule 5 and in the new chat doc. The ciphertext
  must never be returned by any API, logged, or audited.
- On every NyxAgent turn: load the credential row, verify the `ApiKey` row is
  still active/not expired/not rotated (`validate_api_key`-equivalent check
  on the stored hash or key id). If the key is missing, revoked, expired or
  rotated by the user: delete the credential row, provision a fresh key, and
  mark every NyxAgent-bound conversation of that user as needing a session
  rebind (clear `nyxagent_session_id`; set `context_reset_reason =
  "credential_replaced"`). Emit metadata-only audit
  `assistant_agent_credential_provisioned` / `..._replaced`.
- Hook the existing key delete / revoke / rotate paths so the credential row
  is removed when its `ApiKey` goes away (no dangling ciphertext).
- The key appears in the user's API-key list like any other key with platform
  `nyxid-assistant`; the frontend key table shows a small "system-managed by
  the NyxID assistant" hint for that platform. Deleting it is allowed and
  simply causes re-provisioning on the next turn (with the context-reset
  notice). Do not add a separate hidden-key concept.

### D3. NyxID owns the conversation index and transcript; NyxAgent owns live context

NyxAgent has no session-list endpoint and loses memory-mode sessions on
restart, so NyxID persists:

- `assistant_conversations`: `_id` = `nyxa-{32 lowercase hex}` (new prefix
  `nyxa-`, routed by `frontend/src/lib/assistant/conversation-ids.ts`),
  `user_id`, `title` (first user message, 40-char rule as Direct), `model`
  (NyxAgent alias, default `nyxagent/chat`), `nyxagent_session_id:
  Option<String>`, `nyxagent_last_response_id: Option<String>`,
  `credential_api_key_id`, `message_count`, `active_turn: Option<{turn_id,
  started_at}>`, `context_reset_at`/`context_reset_reason: Option`,
  `created_at`, `updated_at`. Indexes: `(user_id, updated_at desc)`.
- `assistant_messages`: UUID `_id`, `conversation_id`, `user_id`, `seq`
  (monotonic per conversation), `turn_id`, `role` (`user` | `assistant`),
  `text`, `status` (`completed` | `failed`), `error_code: Option`,
  `created_at`. Index `(conversation_id, seq)`. Text only; no secrets, no
  tool payloads. Deleting a conversation hard-deletes both and best-effort
  calls NyxAgent `DELETE /v1/sessions/{id}` (failure logged at debug, never
  blocks the delete).

Persist the user message and the `active_turn` fence BEFORE calling NyxAgent;
persist the assistant message (completed, or failed with partial text and
`error_code`) when the upstream stream settles, then clear `active_turn`.
One active turn per conversation: a second send returns 409 with a stable
`turn_active` error code (use an existing `AppError` variant if one maps
cleanly, otherwise add one following `errors/mod.rs` conventions and reserve
its numeric code in CLAUDE.md Rule 3).

### D4. Turn execution is server-owned and survives browser disconnect

`POST /api/v1/assistant/nyxagent/turns` `{ conversation_id?: "nyxa-...",
text: string, model?: alias }` (closed body, unknown fields rejected, text
bounded like Direct's `MAX_MESSAGE_CHARS`). The handler:

1. Enforces the flag, the human-only router placement, and a per-user
   in-flight permit (reuse `mw/rate_limit.rs` `DirectChatPermit` machinery;
   rename/generalise if needed rather than duplicating).
2. Creates the conversation on first send (no separate create route needed;
   keep `POST .../conversations` out unless the frontend genuinely needs it).
3. Persists the user message + fence, then spawns a detached Tokio task that
   runs the NyxAgent turn to completion and persists the result even if the
   browser goes away. The HTTP response is an SSE stream that taps that task
   through a broadcast/watch channel; cancelling the HTTP response must not
   cancel the upstream turn (NyxAgent cannot cancel turns and will commit
   anyway; NyxID must not end up with a user message and no reply).
4. Emits NyxID-owned turn events to the browser reusing the Direct event
   grammar (`turn.status`, `message.started`, `block.started`, `block.delta`,
   `block.completed`, `message.completed`, `turn.completed`, each with a
   `cursor`) so `frontend/src/lib/assistant/direct-transport.ts` decoding and
   `chat-message.tsx` rendering are reused. Add one additive event
   `turn.notice` `{ code: "context_reset", message }` used when D5 rebinds.

Upstream call: `execute_admin_proxy` against the active `llm-nyx` catalog row
(resolved with `assistant_service::resolve_admin_service_by_slug`), path
`v1/responses`, body
`{"model": alias, "input": text, "stream": true, "store": true,
"instructions": <server prompt>, "conversation": session_id (when bound)}`,
headers `Content-Type: application/json`, `Accept: text/event-stream`,
`Idempotency-Key: <turn_id>`, and `Authorization: Bearer <assistant key>`
supplied through `extra_outbound_headers`. VERIFY (with a unit test on the
header-assembly seam) that the assistant key is the Authorization value that
reaches NyxAgent on both the direct HTTP path and the node path regardless of
whether the human authenticated with the cookie session or a bearer JWT:
`build_effective_outbound_headers` applies `extra_outbound_headers` last, but
the direct HTTP path also has the numbered [6] `forward_access_token` bearer
and [7] `auth_method` layers — confirm ordering and add a guard so a
`caller_token` can never overwrite the assistant key on this route. Strip any
caller query string (the Direct handler shows how). The row contract to check
at readiness (D7): active, `requires_user_credential=false`,
`auth_method=none`, `forward_access_token=true`,
`inject_delegation_token=false`, no master credential.

The server prompt (`instructions`) is NyxID-owned and constant: it tells the
model it is the NyxID assistant inside the NyxID web app, that the user is
already signed in, that it should use the NyxID tools to list/inspect/use
connected services, discover and connect new services via hosted connect
links (give the link to the user; never ask for raw credentials), help with
channel bots, agent keys, nodes and approvals, and answer in the user's
language. Keep it in `services/assistant_nyxagent.rs` next to the request
grammar, under a bounded length.

Stream handling: first-byte timeout and idle timeout (NyxAgent keepalives are
15 s; use >= 120 s idle), bounded total output bytes, translate
`response.output_text.delta` -> `block.delta`, `response.completed` ->
persist + `turn.completed(completed)`, `response.failed` / non-2xx / decode
error -> persist partial text with `status=failed` + `turn.completed(failed,
{code,message})`. Unknown event types are ignored. Map upstream error codes
to stable NyxID codes; never forward raw upstream bodies to the browser.

### D5. Session loss and error recovery

- On `not_found` (404) for a bound conversation: the NyxAgent session is gone
  (restart, expiry, or credential replacement). Clear the binding, set
  `context_reset_*`, emit `turn.notice(context_reset)`, and retry the same
  turn ONCE as a new session with `instructions` = server prompt + a bounded
  recap of the stored transcript (last N messages, total <= 8 KiB, clearly
  labelled as prior conversation history the model may rely on). Store the
  new `conversation.id` on success.
- `session_busy` (409): retry with jittered backoff, bounded (e.g. 5 attempts
  over ~30 s), then fail with a stable code.
- `capacity_exceeded`: retry with the same `Idempotency-Key`, bounded.
- `stale_response`: NyxID always sends `conversation` (never
  `previous_response_id`), so this should not occur; if it does, treat as a
  fenced failure, clear the binding, and surface `context_reset` on the next
  turn.
- `outcome_unknown`: fail the turn with a stable code, clear the binding
  (NyxAgent docs: start a new session), do not retry.
- `agent_key_required` / 401 / 403 from NyxAgent: treat the assistant
  credential as invalid, run the D2 replacement path, retry once.
- Anything else: fail the turn with a stable code and a generic message.
- **Any failed turn poisons the NyxAgent session** (`src/responses.rs`
  `execute`: on error it sets `session.recovery_required = true`; `prepare`
  then returns 409 `outcome_unknown` for every later request on that session,
  including after `client_disconnected` (NyxID dropped the upstream stream),
  `turn_timeout`, `lease_lost`, `session_too_large`, `idempotency_conflict`
  and agent errors). Therefore: whenever a bound turn settles as failed for
  any reason, clear `nyxagent_session_id` immediately and set
  `context_reset_reason = "turn_failed"`; the next turn starts a new session
  with the recap (D5 first bullet) and emits `turn.notice(context_reset)`.
  Do not wait for `outcome_unknown` to discover this.
- Stop button: NyxAgent cannot cancel a turn through the API, but dropping
  the upstream SSE connection cancels it (`create_response`: "Dropping the
  HTTP stream cancels the turn and kills its ephemeral runtime"). Implement
  `POST /assistant/nyxagent/conversations/{id}/stop`: it aborts the detached
  task's upstream stream, persists the partial assistant text with
  `status = "failed"`, `error_code = "cancelled"`, clears the binding as
  above, and settles the browser stream with `turn.completed(cancelled)`.
  Only the owner can stop; stopping a conversation with no active turn is a
  no-op 204.
- Idempotency detail: a first turn with `Idempotency-Key` but no
  `conversation` gets a deterministic session id `conv_<hash(owner,key)>`
  (`prepare`), so always store the returned `conversation.id` from the
  completed response, never assume it.

### D6. Routes (all inside the human-only `/api/v1/assistant` router)

- `GET  /assistant/nyxagent/conversations?limit&cursor` -> paginated index
  (`id, title, model, created_at, last_message_at, message_count,
  active_turn?, context_reset_at?`).
- `GET  /assistant/nyxagent/conversations/{id}?limit&before_seq` ->
  conversation metadata + messages (paginated, newest page last) +
  `active_turn` so a reloaded page can show a running turn and poll.
- `PATCH /assistant/nyxagent/conversations/{id}` `{title}` -> rename.
- `DELETE /assistant/nyxagent/conversations/{id}`.
- `POST /assistant/nyxagent/turns` (SSE) as in D4. Mount it with the same
  billing route policy the Direct completions route uses
  (`assistant_direct_billing_routes!` / `BillingRoutePolicy::Metered(Proxy)`),
  because the `llm-nyx` hop goes through `execute_admin_proxy`.
- `GET  /assistant/nyxagent/models` -> proxied `GET /v1/models` aliases with
  a server-side cache (60 s) so the composer can offer a profile selector;
  degrade to `[nyxagent/chat]` when upstream is unavailable.
- Every route: not-found-shaped when the flag is off for that user; owner
  scoped by `AuthUser.user_id` (never a body/query user id); unauthorized
  access to another user's conversation is not-found-shaped.

### D7. Readiness and operator visibility

Extend `GET /assistant/readiness` (`handlers/assistant_readiness.rs`) with a
`nyxagent` block: flag state, `llm-nyx` row present/active, row contract
checks from D4, and whether the caller's assistant credential exists. Add a
startup warning (like the Aevatar `forward_access_token` warning) when the
flag defaults on but the row is missing or violates the contract.

### D8. Frontend

- `frontend/src/lib/assistant/nyxagent-transport.ts` (server-backed:
  list/history/rename/delete/turn SSE via `assistant-http.ts`, identity-aware
  cache clearing on logout like Direct), `frontend/src/hooks/use-assistant-nyxagent.ts`,
  and `NyxAgentAssistantChatPage` in `assistant-chat-page.tsx` reusing the
  Direct page's layout, composer, sidebar and message components. Add the
  `nyxa-` prefix and `"nyxagent"` surface to `conversation-ids.ts` and the
  engine router in `pages/assistant.tsx`; sidebar merges conversations from
  every enabled engine sorted by `last_message_at`.
- Composer: model/profile selector fed by `/assistant/nyxagent/models`
  (persisted per conversation on the first send; later sends reuse it).
- Reload/reconnect with an `active_turn`: show the running state and poll the
  conversation every 2 s until the turn settles, then render the stored
  reply. Never resend the user message automatically.
- `turn.notice(context_reset)` renders as a subtle inline system note
  ("Conversation context was reset; the assistant was given a recap of this
  chat.").
- Assistant text is Markdown. Same-origin NyxID connect-link URLs in a reply
  render as a "Connect <service>" button that opens the link in a new tab
  (`rel="noopener noreferrer"`); all other links stay plain links with the
  same rel. Never auto-open anything.
- Delete/rename affordances in the sidebar for `nyxa-` rows; disabled while
  a turn is active.
- Empty-state copy for a new NyxAgent chat should suggest what it can do
  (see your connected services, connect a new one, set up a channel bot,
  check approvals). Follow `DESIGN.md`.
- API keys page: hint for platform `nyxid-assistant` (D2).
- Tests: vitest for the transport (SSE event decoding incl. `turn.notice`,
  pagination, error mapping, identity clearing), the hook (turn lifecycle,
  reload-poll, delete/rename, engine routing precedence), and the connect-link
  button. Update/extend the assistant e2e specs (`frontend/e2e/*.spec.ts`)
  and HTTP fixtures so the NyxAgent surface is covered and existing specs
  stay green (the fixture page must mock the new routes when the flag is on).

### D9. Docs, config, versioning

- New `docs/chat/08-nyxagent-engine.md` (normative contract for routes,
  bodies, events, persistence, recovery, credential model, row contract) and
  updates to `docs/chat/README.md` (reading order + scope), `01-architecture.md`
  (engine precedence; NyxAgent as default), `02-wire-contract.md`,
  `07-testing-and-gaps.md`. Keep prose consistent with the code; never two
  competing contracts.
- CLAUDE.md: short additions to Rule 3 (any new error code), Rule 5 (the
  encrypted assistant credential exception), Key API Routes
  (`/assistant/nyxagent/*`), and a one-paragraph "NyxAgent assistant engine"
  note pointing at the new doc. Keep it terse; CLAUDE.md is an index, not a
  spec.
- `docs/ENV.md`: no new environment variable is introduced (the slug `llm-nyx`
  and the flag are code-level); say so under the assistant section.
- Version bump to `0.24.0` in `backend/Cargo.toml`, `cli/Cargo.toml`,
  `frontend/package.json` (+ lockfiles as the repo does it; look at commit
  `bb93a548` for the 0.23.0 pattern). If any file in the CLI wizard graph
  changes, run `npm run build:wizard` in `frontend/` so the bundle-freshness
  CI check passes.

## Out of scope (state explicitly in your report)

- Deleting the Aevatar engine and its action-effect routes (follow-up once
  NyxAgent is proven in production).
- The mobile app's `useNyxChat` (`POST /chat`, a route that no longer exists
  on the backend) — pre-existing breakage, untouched.
- Changes to the NxyAgent repository.

## Verification you must run and report verbatim

- Backend: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`
  (workspace), and `NYXID_TEST_DATABASE_URL=mongodb://127.0.0.1:27019
  cargo test -p nyxid --bin nyxid-server -- --test-threads 2` (a replica-set
  MongoDB is already running in Docker on 27019; do not start another). Run
  the CLI tests too if you touch `cli/`.
- Frontend: `npm run lint`, `npm run test`, `npm run build`, and the affected
  Playwright specs (`npx playwright test frontend/e2e/<spec>` per the repo's
  e2e setup) — report exactly which ran and their results.
- A manual contract trace against the NxyAgent clone: for each request you
  build, quote the NxyAgent source line that accepts it (body fields, header
  names, error codes, SSE event names).
