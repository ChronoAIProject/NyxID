# Wire Log Rework: Conversation-Scoped, Lazy-Loaded

Implementation plan. Branch: `feat/wire-log-conversation-scoped-lazy` (cut from `origin/main` @ `56ffa0d0`). This document is self-contained: an implementer needs no other conversation context. All file/line references were verified against `origin/main` @ `56ffa0d0`; line numbers may drift, symbol names should not.

## 1. Problem statement and verified findings

Prod (`https://nyx.chrono-ai.fun`) returns **502 on `GET /api/v1/assistant/conversations`** for callers that (a) send `X-NyxID-Debug-Upstream: 1`, (b) have the `aevatar_chat_wire_log` feature flag on, and (c) have the wire-log panel's capture toggle on. Without the header the identical request is a clean 200. This is admin-debug-only breakage, not an outage.

Verified mechanism (do not re-derive):

- `backend/src/handlers/assistant.rs` attaches `x-nyxid-debug-upstream-log` — a base64 JSON array of "upstream echoes" — via `attach_upstream_echoes()`, capped at `DEBUG_UPSTREAM_HEADER_MAX_BYTES = 12 * 1024` (line ~56).
- `list_conversations` drains two upstream conversation families (`pageSize=50`, up to `MAX_HISTORY_INDEX_PAGES = 40` pages each) and appends **one echo per upstream page call**, so this one route emits a multi-KB header. Measured sizes at the 12 KiB budget: 1 echo = 656 B, 16 = 10,356 B, 24 = 12,040 B. The rung-degradation encoder (`encode_echo_header_with_limit`) is correct and never panics.
- A proxy hop between Cloudflare and the frontend nginx pod has a response-header buffer under ~13 KB (reproduced locally: nginx `proxy_buffer_size 4k` + 12,288-byte header → 502 "upstream sent too big header"). Backend panic, Cloudflare, and the frontend nginx were each ruled out with evidence. **This plan must not depend on any infra fix.**

Design defects being fixed:

1. **No conversation scoping.** Neither `frontend/src/schemas/assistant-wire-log.ts` nor `frontend/src/stores/assistant-wire-log-store.ts` carries a `conversationId`; the only discriminator is `kind: "sse" | "header"`. The panel (`frontend/src/components/assistant/assistant-wire-log-panel.tsx`) has no filter.
2. **Blanket header attachment.** `assistantApi.get/post/del` in `frontend/src/lib/assistant/aevatar-transport.ts` (~lines 202–230) splice in `assistantWireLogOptions()` on every assistant call — including `listConversations`, the sidebar index, which belongs to no conversation.
3. **Self-poisoning.** `CONVERSATION_LIST_TTL_MS = 5_000` plus per-turn-event list re-projection means the sidebar refetch repeatedly writes list exchanges into the 100-entry FIFO (`MAX_ENTRIES = 100`), evicting the chat-turn exchanges the panel exists to debug.
4. **Header is the wrong transport.** The FE already tolerates large payloads (`MAX_CAPTURE_BYTES = 4 MiB` in-memory, `MAX_PERSISTED_BYTES = 2 MiB` localStorage); only the header leg is constrained to 12 KiB.

## 2. Target architecture

### 2.1 Summary

Wire-log payloads move out of the response header into a short-TTL MongoDB collection. The response carries only a ~36-byte id header; the panel lazily fetches the payload when a row is expanded. Entries are tagged with a conversation id and the panel filters to the active conversation. The conversation-list route stops participating in the wire log entirely.

**Decisions (made; do not relitigate):**

- **List route: no echo at all** (not a collapsed summary). It belongs to no conversation, it was the self-poisoning source, and its pagination diagnostics are recoverable from server tracing. Backend `list_conversations` drops its collector; frontend stops sending the debug request header on list calls (belt and braces).
- **Inline header survives only as a degraded fallback** when the Mongo write fails, with the budget lowered from 12 KiB to **4 KiB** (`DEBUG_UPSTREAM_HEADER_MAX_BYTES = 4 * 1024`). 4 KiB clears every observed proxy buffer; the rung machinery already degrades gracefully to that budget. This is the "residual cap regression guard".
- **Envelope stays `version: 2`.** The echo payload shape is unchanged; only the transport changed. No schema version bump.
- **localStorage: zustand persist version 2 → 3, migrate discards old entries.** Wire logs are ephemeral diagnostics with a 15-minute server TTL; carrying v2 entries forward buys nothing and forces dual-shape rendering.
- **No new numeric error code.** The fetch endpoint returns not-found-shaped `AppError::NotFound` for missing, expired, other-owner, and flag-off cases alike (existence must not leak — same discipline as oracle task reads). Say nothing about which case occurred.
- **Rejected: gzip-compressing the header** (superseded by lazy loading — do not implement).

### 2.2 Wire contract

Request header (unchanged): `X-NyxID-Debug-Upstream: 1`.

Response headers:

- `X-NyxID-Debug-Upstream-Id: <uuid-v4>` — attached when the echo payload was stored successfully. Constant name `DEBUG_UPSTREAM_ID_RESPONSE_HEADER = "x-nyxid-debug-upstream-id"`.
- `X-NyxID-Debug-Upstream-Log: <base64 json>` — **fallback only** (Mongo write failed), max 4 KiB, same envelope as today. Never attached together with the id header.

New endpoint:

```
GET /api/v1/assistant/wire-logs/{id}
```

- Mounted on the non-billing `assistant_routes` router in `backend/src/routes.rs` (the `Router::new()` at ~line 1495 that holds `/readiness`, `/direct/skills`, etc. — **not** inside the `assistant_direct_billing_routes!` macro group; fetching a debug log must not meter billing).
- Auth: normal `AuthUser` (session or first-party JWT). Delegated/relay tokens are already rejected: `delegated_read_denied_path` in `backend/src/mw/auth.rs` (~line 393) denies the whole `assistant` first segment — verify with the existing auth tests, no deny-list change needed. Because the wire-log payload contains request bodies (a secret-adjacent, execution-shaped read), keeping it under `/assistant` is load-bearing; do not mount it anywhere else.
- Gate: `feature_flag_service::aevatar_chat_wire_log_enabled(&state.db, &auth_user.user_id.to_string())` — same flag the collector uses (`backend/src/services/feature_flag_service.rs` ~line 433). Flag off → `AppError::NotFound`.
- Ownership: the Mongo query filters `{_id: id, user_id: <caller>}`. No match (wrong owner, expired, never existed) → `AppError::NotFound("Wire log not found.")`.

Response body (dedicated response struct in the handler — never the model):

```json
{
  "id": "<uuid>",
  "conversation_id": "nyxchat-..." | null,
  "created_at": "2026-08-20T12:00:00Z",
  "payload": { "version": 2, "echoes": [ ... ], "droppedEchoCount": 0 }
}
```

`payload` is the exact `UpstreamEchoHeader` JSON that would previously have been base64'd into the header (full rung whenever it fits the storage budget, degraded rungs otherwise).

### 2.3 Mongo model

New file `backend/src/models/assistant_wire_log.rs`:

```rust
pub struct AssistantWireLog {
    #[serde(rename = "_id")]
    pub id: String,                    // UUID v4 string
    pub user_id: String,               // owner (AuthUser.user_id.to_string())
    pub conversation_id: Option<String>,
    pub payload: String,               // serialized UpstreamEchoHeader JSON
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,     // created_at + WIRE_LOG_TTL_SECS
}
impl AssistantWireLog { pub const COLLECTION_NAME: &'static str = "assistant_wire_logs"; }
```

Rules honored: plain serde struct, `COLLECTION_NAME`, no `#[serde(skip_serializing)]`, chrono helpers on both datetimes. `payload` is stored as a JSON string (not nested BSON) so the handler can pass it through with one `serde_json::from_str` and no BSON↔JSON reshaping.

Constants (in the new service): `WIRE_LOG_TTL_SECS: i64 = 15 * 60`, `WIRE_LOG_MAX_PAYLOAD_BYTES: usize = 1024 * 1024` (1 MiB — far above any real echo set; request bodies are already bounded by `MAX_ASSISTANT_CHAT_REQUEST_BYTES = 256 KiB` and per-field truncation).

Index (in `backend/src/db.rs :: ensure_indexes`, matching the existing pattern used for e.g. connect links):

```rust
IndexModel::builder()
    .keys(doc! { "expires_at": 1 })
    .options(IndexOptions::builder().expire_after(Duration::from_secs(0)).build())
    .build()
```

No other index: the only read is by `_id` (+ `user_id` filter), covered by the `_id` index.

Privacy posture: payloads contain user prompts and upstream metadata but **never** injected credentials — the echo builder already whitelists request headers to `content-type` / `accept` / `idempotency-key` (`echoed_headers`) and response headers to `content-type` / `x-request-id` / `x-correlation-id` (`response_echo`), and the `forward()` bridge mints the `Authorization` value *after* the echo is built. Do not widen either whitelist. The 15-minute TTL preserves the metadata-only-durable discipline (same shape as oracle task bodies: content lives only on the TTL'd doc). Tracing/audit around the new code stays metadata-only (ids, sizes, outcomes — never payload content).

### 2.4 Frontend data flow

- **Store entry shape** (`frontend/src/stores/assistant-wire-log-store.ts`): each exchange becomes metadata-first:
  - keep `id`, `ts`, `kind`, `status`, `capture` (SSE line capture is untouched);
  - add `conversationId: string | null`, `wireLogId: string | null`, `label: string` (client-derived, e.g. `"POST /assistant/chat"` — gives rows a readable summary before any fetch);
  - `upstreamEchoes` becomes **optional** and is only populated on the legacy/fallback inline path; `droppedEchoCount` stays optional.
  - Persisted shape (localStorage `nyxid.assistant.wirelog.v1`, zustand `version: 3`): metadata only — `{id, ts, kind, status, conversationId, wireLogId, label, upstreamEchoes?, droppedEchoCount?}`. Fetched payloads are **not** persisted (server TTL makes them stale in 15 min; after reload an expired fetch renders an "expired" state).
  - `migrate` / `merge`: `version < 3` → discard entries (call the existing `removeInvalidPersistedState()` path), keep `captureEnabled`/`showResponses` defaults.
- **Recording**: `assistantWireLogOptions()` (transport, ~line 98) reads `X-NyxID-Debug-Upstream-Id` first; if present, `recordExchange` stores metadata + `wireLogId`. If absent, it falls back to decoding `X-NyxID-Debug-Upstream-Log` exactly as today (fallback path + new-FE/old-BE compat) and stores inline `upstreamEchoes` with `wireLogId: null`. Same dual read on the SSE path (`startChatStream`, ~line 4313, via `stream.headers` / `response.debugUpstream` — the stream client must expose the new header alongside `debugUpstream`).
- **Conversation attribution**:
  - SSE path: `startChatStream(conversationId, ...)` already has the id in scope — pass it through.
  - Header path: `assistantApi.get/post/del` derive it from the endpoint string with one regex (`/assistant/conversations/([^/?]+)`); `null` otherwise. If a create-turn exchange is recorded before the server id is known, add a store action `assignConversation(exchangeId, conversationId)` and call it where the transport learns the created id (it already maintains `conversationAliases`).
  - List suppression: `assistantWireLogOptions(endpoint)` returns `{}` (no header, no capture) when the endpoint is exactly the conversation list (`${ASSISTANT_PREFIX}/conversations` with no further path segment). Both list call sites (`listConversations` ~line 1484 and `fetchRawIndexMembership` ~line 1827) go quiet automatically.
- **Lazy fetch**: new TanStack Query hook `frontend/src/hooks/use-assistant-wire-log.ts`:
  - `useAssistantWireLog(wireLogId: string | null, enabled: boolean)`; key `["assistant-wire-log", wireLogId]`; `enabled: enabled && !!wireLogId`; `staleTime: Infinity` (payloads are immutable); `retry: false`; a 404 renders as "expired or unavailable". Query-cache keying gives dedup across expand/collapse for free.
  - Fetch goes through `assistantApi.get` (it must **not** itself carry the debug header — exclude `/assistant/wire-logs/` in `assistantWireLogOptions`, or the fetch would recursively log itself).
- **Panel** (`assistant-wire-log-panel.tsx`): gets the active conversation id as a prop from its mount in `frontend/src/pages/assistant.tsx`. Default filter = active conversation (plus entries with `conversationId: null` hidden behind an "all conversations" toggle). Row expansion mounts the payload via the hook; render states: loading / loaded (existing echo rendering, unchanged) / expired / error. Inline-fallback entries render directly from `upstreamEchoes` as today.

## 3. Implementation steps

Ordered so the tree builds and tests pass after every step. Steps 1–4 backend, 5–9 frontend, 10 docs. The shared contract (header names, endpoint path, response shape — §2.2) is fixed by this document; backend lands it first.

**Note on backend tests:** the backend integration suite needs a MongoDB replica set and the `NYXID_TEST_DATABASE_URL` override; ~5000 tests failing in seconds means the DB connection is missing, not a regression. The encoder tests in `handlers/assistant.rs` are pure unit tests and run without Mongo.

### Step 1 (backend): extract a storage seam from the encoder

- File: `backend/src/handlers/assistant.rs`.
- Refactor `encode_echo_header_with_limit(echoes, max_bytes)` (~line 356) so the rung-selection loop is reusable: extract `fn select_echo_header(echoes: &[UpstreamEcho], max_encoded_bytes: usize) -> Option<(UpstreamEchoHeader, EchoEncodingRung)>` returning the chosen (possibly degraded) envelope; `encode_echo_header_with_limit` becomes select + serialize + base64 + `HeaderValue`. Budget semantics must stay identical (the existing budget is measured on the base64 output; keep it that way for the header path — for the storage path in step 4 the budget is measured on plain JSON bytes, so give `select_echo_header` a byte-measuring closure or a bool flag; either is fine, keep it explicit).
- Verify: `cargo test -p` (backend package) — the existing `mod tests` around the encoder must pass unchanged.

### Step 2 (backend): model + service + TTL index

- New `backend/src/models/assistant_wire_log.rs` per §2.3; register in `backend/src/models/mod.rs`.
- New `backend/src/services/assistant_wire_log_service.rs`; register in `backend/src/services/mod.rs`:
  - `pub async fn store(db: &Database, user_id: &str, conversation_id: Option<&str>, payload_json: String) -> AppResult<String>` — generates the UUID, sets `created_at`/`expires_at`, inserts, returns the id. Rejects payloads over `WIRE_LOG_MAX_PAYLOAD_BYTES` with an `AppError::Internal` (callers degrade to the inline fallback; they never surface this to the client).
  - `pub async fn fetch_for_user(db: &Database, user_id: &str, id: &str) -> AppResult<Option<AssistantWireLog>>` — `find_one(doc! { "_id": id, "user_id": user_id })`. Belt-and-braces: also filter `"expires_at": { "$gt": now }` since Mongo TTL sweeps are minute-granular.
- `backend/src/db.rs :: ensure_indexes`: add the TTL index per §2.3.
- Verify: `cargo build`; add a service unit/integration test (store → fetch by owner returns row; fetch by other user returns `None`) guarded like the other Mongo-backed tests.

### Step 3 (backend): fetch handler + route

- `backend/src/handlers/assistant.rs`: new handler `get_wire_log(State, auth_user: AuthUser, Path(id): Path<String>) -> AppResult<Json<WireLogResponse>>`:
  - Validate `id` parses as a UUID (cheap not-found otherwise).
  - Flag check via `feature_flag_service::aevatar_chat_wire_log_enabled`; `false` or resolution error → `AppError::NotFound("Wire log not found.")` (fail closed, don't leak).
  - `fetch_for_user`; `None` → same NotFound.
  - Dedicated response struct `WireLogResponse { id: String, conversation_id: Option<String>, created_at: String /* RFC 3339 Z */, payload: serde_json::Value }`; `payload` = `serde_json::from_str(&row.payload)` (an unparseable stored payload is `AppError::Internal` — it cannot happen absent a bug).
- `backend/src/routes.rs`: add `.route("/wire-logs/{id}", get(handlers::assistant::get_wire_log))` on the non-billing `assistant_routes` router (~line 1495).
- Verify: `cargo test`; curl matrix locally: 200 owner+flag, 404 wrong-user / no-flag / random id / expired id.

### Step 4 (backend): switch emission to store-first, lower the inline cap, silence the list route

- `backend/src/handlers/assistant.rs`:
  - `DEBUG_UPSTREAM_HEADER_MAX_BYTES`: `12 * 1024` → `4 * 1024`. Update its comment (the constraint is now "regression guard under every known proxy buffer", not Node's parser).
  - New async helper replacing `attach_upstream_echoes` at all call sites:
    ```rust
    async fn attach_wire_log(
        state: &AppState, auth_user: &AuthUser,
        conversation_id: Option<&str>,
        response: Response, echoes: Option<&[UpstreamEcho]>,
    ) -> Response
    ```
    Behavior: empty/None echoes → unchanged. Otherwise `select_echo_header(echoes, WIRE_LOG_MAX_PAYLOAD_BYTES)` (plain-JSON budget) → serialize → `assistant_wire_log_service::store(...)`. On `Ok(id)` attach `x-nyxid-debug-upstream-id`. On `Err` (Mongo down, oversize) `tracing::warn!` metadata-only and fall back to the old `encode_echo_header` inline path (now 4 KiB). Storage is best-effort: it must never fail or delay-fail the user request beyond the awaited insert.
  - Call-site updates: `get_history`, `delete_conversation`, `get_state` pass `Some(&conversation_id)` (the `Path` param); `typed_chat` passes the command's conversation id where present (`TextChatCommand.conversation_id` is `Option<String>`; the resolve/steer/stop command variants have required `conversation_id` fields — add a small `conversation_id(&AssistantChatCommand) -> Option<&str>` accessor in `assistant_service` if one doesn't exist); `completions` passes `None`.
  - `list_conversations` (~line 710): delete its `upstream_echo_collector` call and every `echoes` plumbing line; pass `ForwardEcho::enabled(None, None, None)` (or a `ForwardEcho::disabled()` helper) and return the response bare. **With flag+header set, the list response must carry neither debug response header.**
  - Update the encoder unit tests that assume the 12 KiB budget (the rung-boundary fixtures shift; behavior at 4 KiB is already exercised by construction — extend with an explicit "48 list echoes at 4 KiB still encodes (Minimal/DroppedEchoes rung)" case as the regression guard).
- Verify: `cargo test`; manual: with flag+header on, `/assistant/conversations/{id}/state` responds with `x-nyxid-debug-upstream-id` only; `GET /api/v1/assistant/wire-logs/{that id}` round-trips the payload; `/assistant/conversations` carries no debug header.

### Step 5 (frontend): schema

- `frontend/src/schemas/assistant-wire-log.ts`:
  - Exchange base schema: add `conversationId: z.string().nullable()`, `wireLogId: z.string().nullable()`, `label: z.string()`; make `upstreamEchoes` optional (`.optional()` — drop the `.min(1)` coupling to presence).
  - New `assistantWireLogRecordSchema` for the fetch response: `{ id, conversation_id: z.string().nullable(), created_at: z.string(), payload: assistantUpstreamEnvelopeHeaderSchema }` (`.strict()`).
  - Persisted schema follows the exchange base. Keep `version: 2` inside the envelope schema untouched; keep the legacy array decoder.
- Update `frontend/src/schemas/assistant-wire-log.test.ts` (new fields round-trip; record schema accepts/rejects shapes; envelope schema unchanged).
- Verify: `npm run test -- assistant-wire-log` and — because CI gates on it — `npm run build` (tsc -b with `noUncheckedIndexedAccess`; `tsc --noEmit` is **not** equivalent).

### Step 6 (frontend): store

- `frontend/src/stores/assistant-wire-log-store.ts`:
  - `recordExchange(meta: { kind, status, conversationId, wireLogId, label, envelopes?, droppedEchoCount? })` — one options object instead of growing positionals; inline `envelopes` only on the fallback path.
  - New `assignConversation(exchangeId, conversationId)`.
  - Persist `version: 2` → `3`; `migrate`/`merge` discard pre-v3 entries (§2.1 decision). `persistedExchange`/byte accounting include the new metadata fields.
  - `captureAssistantWireLogHeader` grows a sibling `captureAssistantWireLogId(idHeaderValue, meta)` (or generalize into one `captureAssistantWireLogResponse`); the base64 decode path stays for fallback.
- Update `frontend/src/stores/assistant-wire-log-store.test.ts` (new shape, migration-discard case, eviction math with metadata-only entries).
- Verify: store + schema suites; `npm run build`.

### Step 7 (frontend): transport

- `frontend/src/lib/assistant/aevatar-transport.ts`:
  - Constant `DEBUG_UPSTREAM_ID_RESPONSE_HEADER = "X-NyxID-Debug-Upstream-Id"`.
  - `assistantWireLogOptions(endpoint: string)`: return `{}` for the exact list endpoint and for `/assistant/wire-logs/` (no self-logging); otherwise attach the request header and an `onResponse` that prefers the id header, falls back to the log header, derives `conversationId` from the endpoint regex, and builds `label` from method + endpoint. Update the three `assistantApi` methods to pass `endpoint` through.
  - SSE path (`startChatStream`): surface the id header from the stream client's headers result (extend the `ChatStreamRequestHandle.headers` payload where `debugUpstream` is populated — follow that existing plumbing; it lives with `chatStreamClient`), record with the in-scope `conversationId`, and call `assignConversation` on create-turn alias resolution if the recorded id was provisional.
- Update `frontend/src/lib/assistant/assistant-wire-log-transport.test.ts`: id-header preferred; fallback still decodes; list endpoint sends no header and records nothing; wire-log fetch endpoint sends no header.
- Verify: transport suite; `npm run build`.

### Step 8 (frontend): lazy-fetch hook

- New `frontend/src/hooks/use-assistant-wire-log.ts` per §2.4 (TanStack Query; parse response with `assistantWireLogRecordSchema`; treat 404 as a typed `"expired"` result rather than a thrown error so the panel can render it distinctly).
- New `frontend/src/hooks/use-assistant-wire-log.test.tsx` (loads on enable, caches, 404 → expired). Mock fetch hermetically — a live dev server on :3000/:3001 can poison vitest runs with real 401s.
- Verify: hook suite; `npm run build`.

### Step 9 (frontend): panel

- `frontend/src/components/assistant/assistant-wire-log-panel.tsx` + its mount in `frontend/src/pages/assistant.tsx`:
  - Prop `activeConversationId: string | null`; default filter to it; "all conversations" toggle (also reveals `conversationId: null` entries, e.g. `/completions`).
  - Rows render from metadata (`label`, `status`, `kind`, `ts`) immediately; expansion mounts the hook when `wireLogId` is set, else renders inline `upstreamEchoes` (fallback entries). Loading / expired / error states.
  - Existing SSE capture rendering (`AssistantWireReplayView`, raw-line windows) is untouched.
- Update `frontend/src/components/assistant/assistant-wire-log-panel.test.tsx`: filter behavior, lazy fetch on expand (mocked hook), expired state, fallback inline render.
- Verify: panel suite; full `npm run test`; `npm run lint`; `npm run build`.

### Step 10: docs

- Update `docs/assistant-wire-log.md`: new transport (id header + fetch endpoint), 4 KiB fallback, conversation scoping, list-route removal, TTL/privacy posture.

## 4. Test plan

Existing suites to keep green (and update per steps above):

- `frontend/src/schemas/assistant-wire-log.test.ts` — envelope + new record schema.
- `frontend/src/stores/assistant-wire-log-store.test.ts` — record/evict/persist/migrate.
- `frontend/src/lib/assistant/assistant-wire-log-transport.test.ts` — header capture wiring.
- `frontend/src/components/assistant/assistant-wire-log-panel.test.tsx` — rendering.
- Backend `handlers/assistant.rs :: tests` — encoder rungs (budget now 4 KiB).

New tests:

- Backend: `assistant_wire_log_service` store/fetch/owner-isolation/expiry-filter; handler auth matrix (owner+flag=200, wrong user=404, flag off=404, delegated token rejected by middleware); `attach_wire_log` fallback (storage error → inline header ≤ 4096 base64 chars); `list_conversations` emits no debug headers.
- Frontend: `use-assistant-wire-log.test.tsx`; transport list-suppression and no-self-logging cases.

Acceptance criteria:

1. With flag + header + capture on, `GET /api/v1/assistant/conversations` returns 200 with **no** `x-nyxid-debug-upstream-log` and **no** `x-nyxid-debug-upstream-id` header — the 502 is structurally impossible on this route.
2. Every other echoing assistant route responds with `x-nyxid-debug-upstream-id` (~36 bytes) and no inline log header when Mongo is healthy; the payload round-trips through `GET /api/v1/assistant/wire-logs/{id}` for the owner and 404s for anyone else, without the flag, or after ~15 minutes.
3. No response header emitted by the assistant surface can exceed 4 KiB of wire-log content under any echo count (regression guard).
4. The panel shows only the active conversation's exchanges by default, and an expanded row fetches its payload on demand exactly once.
5. `cargo test` green; `npm run test`, `npm run lint`, and `npm run build` green (CI gates on `npm run build`).

## 5. Risks and rollback

- **Kill switch (existing):** disabling the `aevatar_chat_wire_log` feature flag stops collector creation (`upstream_echo_collector` returns `None`), so no storage writes, no id headers, and the fetch route 404s. This fully disables the feature at runtime with no deploy.
- **Mongo unavailability:** storage is best-effort; failure degrades to the 4 KiB inline header, which every known proxy buffer passes. The debug feature degrades; user requests are unaffected.
- **Insert latency on the hot path:** one awaited `insert_one` per *debug-flagged* request only; normal traffic (no header) is untouched — the gate order in `upstream_echo_collector` already guarantees zero extra DB work for it.
- **Old FE / new BE skew:** an already-open old panel won't see id headers and only records fallback entries; new FE handles old BE via the retained base64 decode. Ship BE and FE in one PR; the skew window is a deploy race for an admin-only debug tool — acceptable.
- **Rollback:** revert the PR. The TTL collection self-drains within 15 minutes; localStorage v3 entries are discarded by the old code's `version: 2` mismatch handling (its migrate drops what it can't parse). No durable migration in either direction.

## 6. Non-goals

- **No gzip/compression of the header** — explicitly rejected, superseded by lazy loading.
- **No server-side SSE body capture.** This rework *unlocks* it (payloads no longer fit-constrained), but implementing it is a separate change.
- **No infra/proxy buffer changes** — infra confirmation is tracked separately; this plan stands alone.
- **No panel search/grouping beyond the conversation filter**, no admin cross-user wire-log viewer, no retention knob beyond the fixed 15-minute TTL, no new numeric error codes, no changes to the SSE line-capture pipeline (`attachWireLines` et al.), no changes to `list_conversations` pagination semantics.
