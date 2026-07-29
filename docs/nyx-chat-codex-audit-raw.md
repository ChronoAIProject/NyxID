# Independent conformance audit: `nyx-chat` vs NyxID Assistant Chat PRD v8

Audit basis: `docs/nyx-chat-prd.md` Draft v8 and implementation commit `5bf4c3851e14`. The audited implementation files were clean relative to that commit.

## 1. Verdict

This repository is **not a conformant reference implementation of the v8 contract**. It is a fundamentally different, pre-v8 architecture: the browser talks to demo/history/approve routes; the BFF creates Aevatar actors and opens an actor-specific `:stream`; the browser consumes AG-UI/workflow telemetry; and `connect_card` structure is invented by having the LLM embed JSON in a markdown fence which the frontend parses. It has useful integration and security work, especially around NyxID session handling, secret routing, catalog enrichment, DOM sanitization, and redaction, but those strengths do not make it a v8 implementation. Conformance requires replacing the transport, message model, event reducer, approval plane, connect choreography, reconnect/cancel semantics, and artifact surface, not renaming a few fields.

## 2. Conformance table

| Spec ref | Requirement (short) | Implementation status | Evidence (file:line) |
|---|---|---|---|
| §3.1-§3.3, G6 | Exactly three C1 endpoints: list, detail, message POST/SSE | DIVERGES | Browser paths are `/api/demo/chat`, `/api/demo/conversations[/{id}]`, `/api/demo/approve`, and DELETE (`nyx-chat/public/app.js:1299-1307`, `nyx-chat/public/app.js:1535-1547`, `nyx-chat/public/app.js:1635-1647`, `nyx-chat/public/app.js:2358-2368`); BFF uses create, `:stream`, `:approve`, chat-history, and delete upstream routes (`nyx-chat/server.mjs:766-826`, `nyx-chat/server.mjs:975-1057`, `nyx-chat/server.mjs:1060-1086`). |
| §3.0 | Every message is a typed block array in both directions | DIVERGES | User input is posted as scalar `prompt` plus optional `attachment` (`nyx-chat/public/app.js:1640-1646`); stored messages normalize to scalar `content` (`nyx-chat/public/protocol.js:321-332`); connect structure is parsed from text (`nyx-chat/public/blocks.js:20-28`, `nyx-chat/public/blocks.js:78-97`). |
| §3.5 | Server vocabulary: `text`, `connect_card`, `run`, `approval_card`, `artifact` | DIVERGES | Only a locally synthesized `connect_card` resembles a v8 block (`nyx-chat/public/blocks.js:101-132`); run/tool UI and approval UI are built from telemetry events (`nyx-chat/public/app.js:1746-1867`, `nyx-chat/public/app.js:1934-2055`, `nyx-chat/public/app.js:2302-2329`); media is not an artifact block (`nyx-chat/public/app.js:2399-2414`). |
| §3.5 | Typed client `card_action` and `control` blocks | ABSENT | The content post sends `prompt/sessionId/actorId/attachment` (`nyx-chat/public/app.js:1635-1647`); connect completion resubmits the original prompt (`nyx-chat/public/app.js:710-726`); stop aborts controllers (`nyx-chat/public/app.js:2766-2776`). |
| §3.7, A3 | Eight presentation events, cursor dedup, whole-field patches | DIVERGES | The normalizer consumes `RUN_*`, `TEXT_MESSAGE_*`, `TOOL_*`, custom engine events, and raw-observed frames (`nyx-chat/public/protocol.js:61-81`, `nyx-chat/public/protocol.js:84-125`, `nyx-chat/public/protocol.js:128-212`); the app callback ignores the SSE event/id metadata and has no cursor reducer (`nyx-chat/public/app.js:1649-1651`). |
| §3.8, A6, G4 | Raw workflow/engine telemetry never reaches the browser | DIVERGES | The BFF writes upstream chunks unchanged (`nyx-chat/server.mjs:914-924`); the frontend explicitly unwraps `aevatar.raw.observed`, retains `raw`, and renders a raw-event inspector after pattern redaction (`nyx-chat/public/protocol.js:178-210`, `nyx-chat/public/app.js:1881-1893`, `nyx-chat/public/app.js:2701-2718`). |
| §4.3, G3 | In-chat approval calls NyxID `/approvals/requests/{id}/decide` only | DIVERGES | FE calls `/api/demo/approve` (`nyx-chat/public/app.js:2332-2368`); BFF forwards to Aevatar `.../{actorId}:approve` (`nyx-chat/server.mjs:1065-1086`). |
| §4.3, F3 | OAuth placeholder key -> scoped initiate -> `card_action` | DIVERGES | OAuth is a generic NyxID `/keys?slug=...` deep link (`nyx-chat/public/app.js:619-643`, `nyx-chat/public/app.js:885-904`); there is no placeholder-key or scoped OAuth-initiate call. |
| §4.3, F4 | API-key secret goes to NyxID, then attach returned `key_id` | PARTIAL | Secret is posted to the BFF's NyxID key route (`nyx-chat/public/app.js:645-665`) and forwarded to NyxID `/api/v1/keys` (`nyx-chat/server.mjs:541-568`), but the returned key is ignored and no `connection_key_attached` action is sent (`nyx-chat/public/app.js:665-669`). |
| §4.3, F5 | Device-code initiate/poll, display code/URL, attach key | DIVERGES | Device code is treated like the same external `/keys?slug=...` flow (`nyx-chat/public/blocks.js:135-142`, `nyx-chat/public/app.js:885-904`); no initiate/poll endpoint or user code is used. |
| §3.6, §4.3 | Re-resolve catalog, validate scopes, extend agent-key allowlist | PARTIAL | Catalog data enriches a slug match (`nyx-chat/public/blocks.js:104-130`, `nyx-chat/server.mjs:498-510`), but available entries omit catalog scope/provider IDs, `requested_scopes` comes from the parsed LLM payload, and the key proxy validates only slug syntax/credential (`nyx-chat/public/blocks.js:85-96`, `nyx-chat/server.mjs:541-559`); no allowlist update exists in the card completion path (`nyx-chat/public/app.js:697-726`). |
| §3.4, A1, G2 | Detail GET alone renders final/current blocks | DIVERGES | Detail returns an array of string-content messages (`nyx-chat/server.mjs:779-795`); the frontend normalizer discards block envelopes and renders/re-parses `content` (`nyx-chat/public/protocol.js:321-332`, `nyx-chat/public/app.js:1514-1526`). |
| §3.4 | Poll detail every 1-3 s while `active_turn` exists | ABSENT | History refresh fetches the list immediately and once after 1.5 s (`nyx-chat/public/app.js:1563-1569`); conversation detail is fetched only when opening an uncached conversation (`nyx-chat/public/app.js:1421-1464`). |
| §3.4, A4, F11 | Stop posts `control:cancel` and terminalizes every block | DIVERGES | Stop only aborts local `AbortController`s (`nyx-chat/public/app.js:2766-2776`); UI states that upstream may still execute (`nyx-chat/public/app.js:1672-1680`); BFF merely aborts the open HTTP request on close (`nyx-chat/server.mjs:859-865`). |
| §3.0, A2-A3 | `schema_version`, `client_msg_id`, idempotent retry | ABSENT | Message POST body has none of these fields (`nyx-chat/public/app.js:1640-1646`), and history normalization emits only id/role/content/timestamp/status/error (`nyx-chat/public/protocol.js:321-332`). |
| §3.0, A2 | Unknown block/version renders neutral unsupported shell | ABSENT | Unknown frames normalize to `unknown` or an arbitrary lower-case type (`nyx-chat/public/protocol.js:61-81`, `nyx-chat/public/protocol.js:121-125`) and fall through without UI in the handler (`nyx-chat/public/app.js:1861-1867`); malformed connect fences become ordinary markdown (`nyx-chat/public/blocks.js:78-88`). |
| §3.0 | Binding error envelope, 32 KiB cap, pagination, client UUID lazy-create | DIVERGES | Errors are top-level `{code,message}` and the BFF cap is 10 MiB (`nyx-chat/server.mjs:13-14`, `nyx-chat/server.mjs:301-317`, `nyx-chat/server.mjs:1291-1304`); history URL has no `before/limit` (`nyx-chat/public/app.js:1299-1307`); BFF creates an actor via a separate POST and returns its upstream id (`nyx-chat/server.mjs:975-989`). |
| §3.6 | Markdown subset, no raw HTML, https/mailto links, no remote images | PARTIAL | It correctly uses DOMPurify and `rel="noopener noreferrer"` for HTTP(S), but enables the full HTML profile, accepts `http:`, and does not enforce an https/mailto allowlist (`nyx-chat/public/app.js:2282-2299`); CSP permits all HTTPS images and media rendering autoloads HTTPS images (`nyx-chat/server.mjs:1185-1193`, `nyx-chat/public/app.js:2399-2409`). |
| §3.6, F9 | Typed artifacts plus auth, MIME/size/header controls | ABSENT | The browser only handles `media` frames (`nyx-chat/public/app.js:1835-1838`, `nyx-chat/public/app.js:2399-2414`), and the BFF router has no artifact/download route among its chat routes (`nyx-chat/server.mjs:1262-1290`). |
| §7 A8, F2/F12 | All missing connections at once; provider readiness flow | DIVERGES | Missing connections are delegated to an LLM-authored fence; the injected catalog is best-effort and truncated to 60 (`nyx-chat/server.mjs:574-608`, `nyx-chat/server.mjs:1025-1033`). Health checks `/api/capabilities`, not NyxID `/api/v1/llm/status` (`nyx-chat/server.mjs:1119-1153`). |
| §3.3 | v1 rejects reserved image/file input blocks | DIVERGES | UI accepts arbitrary files up to 5 MiB (`nyx-chat/public/app.js:20`, `nyx-chat/public/app.js:2823-2842`); BFF maps them to image/audio/video/file input parts (`nyx-chat/server.mjs:946-972`). |
| §1, A6 | Connect-card secret never reaches Aevatar/chat context | CONFORMS | API-key value is sent only to `/api/nyxid/keys` (`nyx-chat/public/app.js:645-665`) and the BFF forwards it only to NyxID `/api/v1/keys` (`nyx-chat/server.mjs:541-559`); the chat retry stores prompt and attachment, not the credential (`nyx-chat/public/app.js:710-726`). |

## 3. Findings

### 1. BLOCKER - The C1 transport is a different API, not the three-endpoint v8 surface

The spec says, **“v1 surface = exactly three endpoints”** and **“no signals endpoint, no delete, no separate stream endpoint”** (`docs/nyx-chat-prd.md:66`). The implementation has separate create, stream, approval, workflow-resume, and delete paths. New conversation ids are upstream actor ids returned by a separate creation call, rather than client-minted UUIDs lazily materialized by the first message POST (`nyx-chat/server.mjs:975-989`, `nyx-chat/server.mjs:1025-1057`).

Complete browser API call map (excluding static/CDN asset loads):

| Browser call | What initiates it | BFF/upstream behavior |
|---|---|---|
| `GET /api/demo/config` | App initialization (`nyx-chat/public/app.js:367-387`) | Returns local config; if a credential exists, validates it through NyxID `/api/v1/users/me` (`nyx-chat/server.mjs:203-237`, `nyx-chat/server.mjs:1244-1259`). |
| `GET /api/auth/login?...` | Login navigation (`nyx-chat/public/app.js:494-498`) | Redirects the browser to NyxID `/cli-auth` on localhost or `/login` same-site (`nyx-chat/server.mjs:653-683`). |
| `GET /api/auth/session` | Auth refresh (`nyx-chat/public/app.js:519-539`) | Calls NyxID `/api/v1/users/me` (`nyx-chat/server.mjs:203-237`). |
| `GET /api/auth/services` | Service panel load (`nyx-chat/public/app.js:542-568`) | Calls NyxID `/api/v1/user-services` (`nyx-chat/server.mjs:410-429`). |
| `GET /api/nyxid/connectors[?fresh=1]` | Catalog/card refresh (`nyx-chat/public/app.js:571-591`) | Calls NyxID `/api/v1/keys` and `/api/v1/catalog`, with a 60 s cache (`nyx-chat/server.mjs:514-539`). |
| `POST /api/nyxid/keys` | API-key connect form (`nyx-chat/public/app.js:645-665`) | Calls NyxID `POST /api/v1/keys` (`nyx-chat/server.mjs:541-568`). |
| NyxID web `/keys` or `/keys?slug=...` | Service/OAuth/device navigation (`nyx-chat/public/app.js:500-512`, `nyx-chat/public/app.js:619-643`) | Browser navigation only; it does not call the v8 placeholder/OAuth/device APIs. |
| `POST /api/auth/logout` | Logout (`nyx-chat/public/app.js:1074-1089`) | Calls NyxID `POST /api/v1/auth/logout` (`nyx-chat/server.mjs:724-756`). |
| `POST /api/demo/health` | Connection check (`nyx-chat/public/app.js:1190-1209`) | Through NyxID proxies, calls Aevatar `/api/capabilities` and Ornn `/api/v1/skill-search?...` (`nyx-chat/server.mjs:1119-1153`). |
| `GET /api/demo/conversations` | History list (`nyx-chat/public/app.js:1314-1336`) | Calls Aevatar `/api/scopes/{scope}/chat-history` through the NyxID proxy (`nyx-chat/server.mjs:766-777`). |
| `GET /api/demo/conversations/{actorId}` | Open uncached history (`nyx-chat/public/app.js:1421-1464`) | Calls Aevatar `/api/scopes/{scope}/chat-history/conversations/{actorId}` (`nyx-chat/server.mjs:779-795`). |
| `DELETE /api/demo/conversations/{actorId}` | Trash button (`nyx-chat/public/app.js:1535-1556`) | Deletes both Aevatar actor and history resources using two upstream DELETEs (`nyx-chat/server.mjs:797-826`). |
| `POST /api/demo/chat` | Send prompt (`nyx-chat/public/app.js:1588-1651`) | Workflow mode calls `/api/chat`; NyxID-chat mode separately creates/polls a conversation and then calls `/api/scopes/{scope}/nyxid-chat/conversations/{actorId}:stream` (`nyx-chat/server.mjs:1002-1057`). |
| `POST /api/demo/approve` | Approval button (`nyx-chat/public/app.js:2332-2379`) | Calls Aevatar `.../{actorId}:approve`, or workflow `/api/scopes/{scope}/runs/{runId}:resume` (`nyx-chat/server.mjs:1060-1116`). |

All protected BFF routes first derive the session through NyxID `/api/v1/users/me`; runtime calls then use that verified user's id as `scopeId` (`nyx-chat/server.mjs:203-245`, `nyx-chat/server.mjs:276-289`). All Aevatar calls are made via `NYXID_AEVATAR_PROXY_URL`, because `fetchRequest` combines its path with `runtime.proxyBaseUrl`; the configured direct Aevatar URL is not used by that helper (`nyx-chat/server.mjs:276-295`, `nyx-chat/server.mjs:355-370`). Local token sessions may additionally call NyxID `/api/v1/auth/refresh` before any of the above (`nyx-chat/server.mjs:140-182`). These generic fetch sites plus the concrete paths in the table account for every server-to-upstream network call in `server.mjs` (`nyx-chat/server.mjs:141`, `nyx-chat/server.mjs:204`, `nyx-chat/server.mjs:364`, `nyx-chat/server.mjs:396`, `nyx-chat/server.mjs:735`). This call graph is coherent for the demo's architecture, but it cannot be used as a v8 request/response or network-conformance fixture.

### 2. BLOCKER - The load-bearing typed-block boundary is explicitly violated

The spec says, **“every message, in both directions, is a typed block array”** and **“Structure never rides inside markdown; the FE never parses text to derive an action, card, or identifier”** (`docs/nyx-chat-prd.md:76`). Here, the BFF appends instructions telling the LLM to output a fenced `nyxid:connect` JSON object (`nyx-chat/server.mjs:580-608`, `nyx-chat/server.mjs:1025-1031`). `blocks.js` then line-scans assistant markdown, JSON-parses the fence, extracts `catalog_slug`, `reason`, and `requested_scopes`, and creates a synthetic block id (`nyx-chat/public/blocks.js:20-28`, `nyx-chat/public/blocks.js:41-75`, `nyx-chat/public/blocks.js:78-97`).

The reverse direction is also untyped: the browser sends `{prompt, sessionId, actorId, attachment}` (`nyx-chat/public/app.js:1635-1647`) and the BFF sends `{prompt, sessionId, inputParts}` to the actor stream (`nyx-chat/server.mjs:1035-1049`). This matters because model prose now controls application structure and identifiers, history fidelity depends on re-parsing text, malformed output silently changes semantics, and there is no stable cross-boundary schema to validate.

### 3. BLOCKER - The stream vocabulary and reducer are AG-UI/workflow telemetry, with no cursor semantics

The spec requires `turn.status`, `message.started`, `block.started`, `block.delta`, `block.updated`, `block.completed`, `message.completed`, and `turn.completed`; **“every event has a per-turn monotonic `cursor`; delivery is at-least-once; the FE dedups by `cursor`”** (`docs/nyx-chat-prd.md:143-148`, `docs/nyx-chat-prd.md:243-256`).

The frontend instead normalizes protobuf-shaped and AG-UI frames such as `RUN_STARTED`, `TEXT_MESSAGE_CONTENT`, `TOOL_CALL_START`, `aevatar.step.request`, and `aevatar.raw.observed` (`nyx-chat/public/protocol.js:61-81`, `nyx-chat/public/protocol.js:84-125`, `nyx-chat/public/protocol.js:128-212`). `consumeSse` parses SSE `id`, but the app callback receives only `raw`, calls `handleFrame`, and never stores or deduplicates a v8 cursor (`nyx-chat/public/protocol.js:5-25`, `nyx-chat/public/app.js:1649-1651`). There is no block-id reducer or whole-top-level-field patch behavior; UI state is updated imperatively from run/tool/text events (`nyx-chat/public/app.js:1746-1867`). At-least-once delivery could therefore duplicate visible effects, and Appendix A cannot be replayed against this client.

### 4. BLOCKER - Raw engine telemetry reaches and is intentionally displayed in the browser

The spec says, **“None of this may appear on C1”** and **“C1 emits exclusively the §3.5/§3.7 presentation vocabulary”** (`docs/nyx-chat-prd.md:258-260`). The BFF inspects events only to reset an idle timer, then writes each upstream chunk to the response unchanged (`nyx-chat/server.mjs:847-856`, `nyx-chat/server.mjs:914-924`).

The browser explicitly recognizes `aevatar.raw.observed`, preserves its actor/correlation/state envelope and nested payload, and retains the original `raw` frame (`nyx-chat/public/protocol.js:178-210`). `recordEvent` applies pattern-based redaction, stores up to 120 raw frames, and the inspector renders them as JSON (`nyx-chat/public/app.js:1881-1893`, `nyx-chat/public/app.js:2701-2718`). The redactor protects some credential-shaped fields and `reasoningContent`, but recursively retains all other engine data (`nyx-chat/public/protocol.js:223-263`). Redaction is useful defense in depth; it is not equivalent to the prohibition. Workflow YAML, prompts, actor ids, downstream bodies, or new sensitive fields can still cross the browser boundary before or after incomplete redaction.

### 5. BLOCKER - Approval uses the forbidden Aevatar decision plane

The spec requires that the browser call **“`POST /api/v1/approvals/requests/{id}/decide`”** and Appendix C says an in-chat decision must produce **“exactly one NyxID decide call and no Aevatar approve call”** (`docs/nyx-chat-prd.md:296-301`, `docs/nyx-chat-prd.md:569`).

The card is constructed from a generic Aevatar approval event (`nyx-chat/public/app.js:2302-2329`). Clicking it posts run/actor/request/tool context to `/api/demo/approve` (`nyx-chat/public/app.js:2332-2368`), and the BFF forwards NyxID-chat approvals to `.../nyxid-chat/conversations/{actorId}:approve` (`nyx-chat/server.mjs:1060-1086`). No NyxID human-session decide endpoint or idempotency key is used. This creates the exact G3 split-brain risk: a decision made in chat is outside NyxID's web/Telegram/mobile convergence plane and is not guaranteed to match the proxy-created approval row.

### 6. MAJOR - The v8 block vocabulary is mostly absent, and the one block is client-invented

The spec's server union is **“`text` + `connect_card` + `run`”**, completed by `approval_card` and `artifact`, with shared `{block_id,type}` and `schema_version` on the message (`docs/nyx-chat-prd.md:150-154`).

The implementation does locally build a `connect_card` with many useful v8-like names, and it declares the correct six v8 connect states (`nyx-chat/public/blocks.js:11-18`, `nyx-chat/public/blocks.js:101-132`). However:

- Its `block_id` is synthesized from LLM-derived slug and ordinal, not supplied by Aevatar (`nyx-chat/public/blocks.js:89-97`).
- `requested_scopes` comes from the fenced model payload, while `granted_scopes` remains null in the builder (`nyx-chat/public/blocks.js:94-96`, `nyx-chat/public/blocks.js:119-123`).
- The active UI handles only `needs_connection`, `waiting_for_user`, `connected`, and `error` labels; it does not implement `waiting_for_provider` or `timed_out` behavior (`nyx-chat/public/app.js:749-757`).
- The run card is an activity/tool UI derived from engine events, not a typed `run` block with the specified `state`, complete `steps` array, and per-step service/artifact/approval references (`nyx-chat/public/app.js:1934-2055`).
- Approval is a generic event card with tool arguments and no v8 approval fields or terminal decision/channel mapping (`nyx-chat/public/app.js:2302-2329`).
- `artifact` does not exist; `media` is a different event and schema (`nyx-chat/public/app.js:1835-1838`, `nyx-chat/public/app.js:2399-2414`).

Unknown content also has no neutral shell: unknown frames fall through without rendering, while invalid connect JSON is reclassified as ordinary code-markdown (`nyx-chat/public/protocol.js:121-125`, `nyx-chat/public/app.js:1861-1867`, `nyx-chat/public/blocks.js:78-88`).

### 7. MAJOR - Connect choreography is not F3-F6 and trusts an incomplete action model

The spec requires catalog re-resolution, **“`requested_scopes ⊆` the catalog scope set”**, OAuth placeholder creation plus scoped initiate, a typed `connection_key_attached` action, device-code initiate/poll, and agent-key allowlist extension (`docs/nyx-chat-prd.md:296-310`).

The API-key path gets one important boundary right: the secret is submitted to NyxID rather than Aevatar (`nyx-chat/public/app.js:645-665`, `nyx-chat/server.mjs:541-559`). But the app never reads the returned key id; it marks the card connected locally and schedules a fresh submission of the original user prompt (`nyx-chat/public/app.js:665-669`, `nyx-chat/public/app.js:697-726`). It never posts `card_action:connection_key_attached`, so Aevatar cannot bind the created key to the waiting block, and replaying the original prompt can repeat already-executed work.

OAuth and device-code cards both open a generic `/keys?slug=...` page, set local `waiting_for_user`, and rely on a manual full catalog refresh (`nyx-chat/public/app.js:619-643`, `nyx-chat/public/app.js:678-695`, `nyx-chat/public/app.js:885-904`). There is no placeholder key, `scope_override`, OAuth JSON initiate, popup callback polling, RFC 8628 code/URL, fresh-key retry, or external-gate client block.

There is partial catalog re-resolution for presentation: the LLM slug must match the cached connected/available snapshot before an inline API-key form appears (`nyx-chat/public/blocks.js:104-130`, `nyx-chat/public/app.js:825-885`). But the BFF's available entry omits scopes and provider configuration, the LLM supplies `requested_scopes`, and the key endpoint merely regex-checks the submitted slug and credential (`nyx-chat/server.mjs:498-510`, `nyx-chat/public/blocks.js:85-96`, `nyx-chat/server.mjs:541-559`). No scope-subset check or `allowed_service_ids` update is present in completion (`nyx-chat/public/app.js:697-726`).

### 8. MAJOR - History cannot be the source of truth, and reconnect/polling is absent

The spec says, **“The page renders from this response alone”** and on stream drop/reload **“while `active_turn` is non-null it polls the detail GET (1-3 s) until null”** (`docs/nyx-chat-prd.md:101-117`, `docs/nyx-chat-prd.md:143-147`).

The BFF detail route returns a flat array of stored messages and only strips its injected prompt context from user `content` (`nyx-chat/server.mjs:779-795`). The frontend reduces each message to `id/role/content/timestamp/status/error`, discarding any typed envelope or current block state (`nyx-chat/public/protocol.js:321-332`), then reconstructs assistant connect cards by re-parsing text (`nyx-chat/public/app.js:1514-1526`). In-page mutable state lives in JS card objects and DOM; it is not sourced from detail (`nyx-chat/public/app.js:596-616`, `nyx-chat/public/app.js:697-707`). Run steps and approval decisions are not reconstructed from history at all.

There is no `active_turn` handling. `scheduleHistoryRefresh` refreshes only the list now and once 1.5 seconds later, and `loadConversation` fetches detail once only for an uncached conversation (`nyx-chat/public/app.js:1421-1464`, `nyx-chat/public/app.js:1563-1569`). Worse, the BFF converts 120 seconds of keepalive-only waiting into legacy `RUN_ERROR` and aborts upstream (`nyx-chat/server.mjs:895-910`), while the frontend has no detail-poll fallback. Long OAuth/admin/approval waits therefore cannot satisfy F10 or A1.

### 9. MAJOR - Stop cancels reception, not the turn, and cannot guarantee terminal blocks

The spec requires **“an action post with `{ "type": "control", "action": "cancel" }`”**, followed by terminal patches and `block.completed` for every open card/run before `turn.completed {cancelled}` (`docs/nyx-chat-prd.md:143-148`).

The stop button disables itself and aborts the active browser controllers (`nyx-chat/public/app.js:2766-2776`). The BFF responds to client close by aborting its upstream fetch (`nyx-chat/server.mjs:859-865`), but it sends no cancel command. The frontend explicitly tells the user that it only stopped receiving and the agent may continue (`nyx-chat/public/app.js:1672-1680`). Its local finalizer closes running tool/step rows, but it does not terminalize connect-card or approval state (`nyx-chat/public/app.js:2123-2149`). This fails A4 and can leave both UI and actual side effects in an indeterminate state.

### 10. MAJOR - Versioning, idempotency, errors, pagination, and conversation identity do not implement §3.0

The spec requires `schema_version` on every message, ≥24-hour `client_msg_id` idempotency, the nested `{error:{code,message}}` envelope and binding cases, stable `before/limit` pagination, and client-minted UUID conversations (`docs/nyx-chat-prd.md:74-83`).

The frontend message has no `blocks`, `schema_version`, or `client_msg_id` (`nyx-chat/public/app.js:1640-1646`). The stored-message normalizer likewise has no schema version or unknown-field preservation (`nyx-chat/public/protocol.js:321-332`). The BFF emits top-level `{code,message}`, not the contract envelope (`nyx-chat/server.mjs:1291-1304`), and enforces a generic 10 MiB request cap rather than the binding serialized-message limit/error case (`nyx-chat/server.mjs:13-14`, `nyx-chat/server.mjs:301-317`). The composer is separately capped at 12,000 characters (`nyx-chat/public/index.html:169-175`).

The history URL only sends `surface` and `workflow`, not `before/limit`, and server history requests do not forward pagination (`nyx-chat/public/app.js:1299-1307`, `nyx-chat/server.mjs:766-795`). New conversations are created by a separate upstream POST and assigned an actor id; DELETE is implemented in both browser and BFF despite being v1.1 (`nyx-chat/server.mjs:975-989`, `nyx-chat/public/app.js:1535-1547`, `nyx-chat/server.mjs:797-826`). Retries therefore have neither the contract's identity nor deduplication semantics.

### 11. MAJOR - Rendering is sanitized, but it is not the binding markdown/link/image subset

The spec requires **“no raw HTML; links limited to `https:`/`mailto:`”** and **“no autoloaded remote images”** except catalog `icon_url` (`docs/nyx-chat-prd.md:236-241`).

The implementation uses `marked` with GFM/breaks and then DOMPurify, which is a solid baseline (`nyx-chat/public/app.js:461-466`, `nyx-chat/public/app.js:2282-2292`). But `USE_PROFILES:{html:true}` sanitizes raw HTML rather than forbidding it, and the post-processing explicitly accepts both `http:` and `https:` links without enforcing an https/mailto allowlist (`nyx-chat/public/app.js:2288-2299`). Markdown-generated HTTPS images remain loadable because the CSP allows `img-src ... https:`, and separate media frames also autoload HTTPS images (`nyx-chat/server.mjs:1185-1193`, `nyx-chat/public/app.js:2399-2409`).

Display text is generally inserted with `textContent` through `el`, which is good (`nyx-chat/public/app.js:360-364`), and catalog icon URLs are restricted to HTTPS at rendering (`nyx-chat/public/app.js:760-775`). Those defenses should remain, but the renderer still needs a strict tag/attribute/protocol allowlist and remote-image suppression to conform.

### 12. MAJOR - Artifact blocks and artifact download security are absent

The spec requires an `artifact` block and downloads with owner authorization, attachment disposition, sanitized filename, `nosniff`, a four-MIME allowlist, and a 256 KiB cap (`docs/nyx-chat-prd.md:208-212`, `docs/nyx-chat-prd.md:236-241`).

The only output-file-adjacent event is `media`; images may be rendered inline from base64 or HTTPS, and other media becomes an informational message (`nyx-chat/public/app.js:1835-1838`, `nyx-chat/public/app.js:2399-2414`). The BFF routes chat, approval, history, and static files, but has no artifact download handler (`nyx-chat/server.mjs:1262-1290`). Consequently none of the artifact ownership, MIME, size, filename, or `Content-Disposition` guarantees can be verified or exercised.

### 13. MAJOR - A8/F2 and F12 are delegated to prompt compliance, not guaranteed

The spec guarantees **“Missing connections for a task are reported completely and at once”** and the zero-connector flow uses provider readiness to emit a provider connect card (`docs/nyx-chat-prd.md:405-415`, `docs/nyx-chat-prd.md:49-51`).

The BFF takes at most 60 available catalog entries and adds prose instructions asking the LLM to write a fence for a needed service (`nyx-chat/server.mjs:574-608`). Catalog acquisition is explicitly best effort; if it fails, chat proceeds without it (`nyx-chat/server.mjs:1025-1033`). There is no deterministic preflight that computes the full missing-service set, no enforcement that all cards are in one assistant message, and no `/api/v1/llm/status` readiness call. The health path checks Aevatar capabilities and Ornn only (`nyx-chat/server.mjs:1119-1153`). The LLM may happen to produce several fences, but that is not interface-observable A8 conformance.

### 14. MINOR - The client accepts v1-reserved multimodal input with incompatible limits and shapes

The spec says image/file client blocks are **“RESERVED for multimodal input in v1.1 - v1 rejects them 400”** (`docs/nyx-chat-prd.md:119-127`, `docs/nyx-chat-prd.md:230-234`).

The UI accepts an arbitrary file up to 5 MiB, base64-encodes it, and includes it beside the scalar prompt (`nyx-chat/public/app.js:20`, `nyx-chat/public/app.js:2823-2842`, `nyx-chat/public/app.js:1640-1646`). The BFF maps MIME prefixes to `image`, `audio`, `video`, or `file` input parts (`nyx-chat/server.mjs:946-972`). This may be a useful future feature, but on a v8/v1 endpoint it must be rejected or placed behind a negotiated v1.1 schema version; otherwise the reference implementation teaches clients a contract the PRD explicitly reserves.

## 4. Things the repo gets right / worth adopting

1. **Strong site-session BFF boundary.** Browser credentials are limited to an explicit Bearer token, a local HttpOnly session, or the single configured NyxID cookie; outgoing headers contain only `Authorization` or that cookie, and proxy-only identity/delegation headers are not accepted from clients (`nyx-chat/server.mjs:170-200`). User scope is derived from the server-verified `/users/me` profile, not browser authority (`nyx-chat/server.mjs:203-245`, `nyx-chat/server.mjs:276-289`). Preserve this topology when exposing the three C1 routes.

2. **Same-origin protection and hardening headers.** Non-safe methods are origin-checked (`nyx-chat/server.mjs:248-261`, `nyx-chat/server.mjs:1200-1207`), and static responses set CSP, `nosniff`, and `Referrer-Policy:no-referrer` (`nyx-chat/server.mjs:1181-1196`). The CSP needs the image restriction noted above, but the general approach is worth adopting.

3. **The API-key secret does not enter Aevatar chat content.** The form sends the credential to the NyxID key route (`nyx-chat/public/app.js:645-665`), which forwards only to NyxID `/api/v1/keys` (`nyx-chat/server.mjs:541-559`). The original prompt retry contains no credential (`nyx-chat/public/app.js:710-726`). Keep this broker property while replacing the completion step with typed `card_action`.

4. **Catalog enrichment is a useful renderer input.** The BFF joins connected keys with catalog metadata, derives display/auth information, and caches it briefly (`nyx-chat/server.mjs:455-532`). The frontend then uses catalog-derived service names, icons, instructions, and auth kind (`nyx-chat/public/blocks.js:104-132`). v8 should keep the catalog resolution idea, but include provider ids and scope sets and make it mandatory before actions.

5. **Defensive rendering and redaction exist.** Text helpers use `textContent` (`nyx-chat/public/app.js:360-364`), markdown passes through DOMPurify with a plaintext fallback (`nyx-chat/public/app.js:2282-2292`), external action links use `noopener noreferrer` (`nyx-chat/public/app.js:852-859`), and recursive redaction covers many credential-shaped keys/values plus reasoning (`nyx-chat/public/protocol.js:223-263`). Move telemetry redaction to server-side observability and retain client-side sanitization as defense in depth.

6. **The implementation is honest about its current stop behavior.** It tells users that stopping reception does not undo submitted production work (`nyx-chat/public/app.js:1672-1680`, `nyx-chat/public/app.js:2759-2763`). The v8 implementation must actually cancel and terminalize, but this explicit operational warning is preferable to falsely claiming cancellation while the backend may continue.

7. **Connect-card presentation already uses much of the target vocabulary.** The local builder includes `block_id`, `catalog_slug`, `service_name`, `auth_kind`, key/scope/code fields, state, error, steps, and footer, and declares all six v8 states (`nyx-chat/public/blocks.js:11-18`, `nyx-chat/public/blocks.js:111-132`). The renderer can be adapted to consume authoritative server blocks rather than discarded.

## 5. Proposed changes

1. **Replace the chat transport, tied to findings 1 and 10.** Expose exactly list/detail/message C1 routes under the chosen authenticated topology. Remove the frontend workflow surface selector and all C1-adjacent create, `:stream`, `:approve`, workflow-resume, and DELETE calls. Mint a UUID client-side and let the first typed message POST create it lazily.

2. **Introduce a validated typed message/block model, tied to findings 2 and 6.** Define schema validators for message envelopes, all five server blocks, and both client block types. Delete `nyxid:connect` prompt injection and `splitMessageSegments`; render blocks directly. Add a neutral unsupported-content shell that preserves unknown fields/version data.

3. **Build the v8 reducer and server-side telemetry adapter, tied to findings 3 and 4.** Map internal Aevatar/AG-UI events to only the eight presentation events inside Aevatar/BFF. Reject or drop every other frame before it reaches the browser. Implement monotonic-cursor dedup, `block_id` reconciliation, text append, whole-field shallow patch replacement, authoritative completion, and final detail reconciliation.

4. **Move approval to NyxID exclusively, tied to finding 5.** Hydrate the card from NyxID as needed and call `POST /api/v1/approvals/requests/{id}/decide` once with `idempotency-key`; optionally post `approval_decided` as a latency nudge. Delete `/api/demo/approve` and every Aevatar `:approve` call. Aevatar should only poll NyxID status and retry the operation-bound proxy request.

5. **Implement an explicit connect-card action controller, tied to finding 7.** Re-fetch catalog by `catalog_slug`, resolve provider/service/scope data, enforce `requested_scopes` subset, and refuse stale/unknown inputs. Implement OAuth placeholder + scoped initiate + popup/key polling; API-key creation; device-code initiate/poll; fresh-key retry; external-gate recheck; typed `connection_key_attached` actions; and agent-key allowlist extension using `user_service_id`.

6. **Make detail state authoritative and add polling reconnect, tied to finding 8.** Return final/current blocks, partial messages, `schema_version`, and `active_turn`. On load or stream failure, replace/reconcile UI from detail; poll every 1-3 seconds until `active_turn` is null. Do not treat idle keepalives as failed turns during legitimate waiting.

7. **Implement real cancellation, tied to finding 9.** Stop must POST a typed `control:cancel` action during the active turn. Aevatar must stop at a step boundary, terminalize connect/approval/run blocks, emit all `block.completed` events, then emit `turn.completed {cancelled}`. Reconcile detail afterward.

8. **Implement all §3.0 protocol guards, tied to finding 10.** Add `client_msg_id` retention/conflict behavior, message `schema_version`, 32 KiB serialized limit, nested binding error envelopes, active-turn conflict handling, action/content mixing validation, stable `before/limit` pagination, unknown-field round-trip, and ownership-not-found behavior.

9. **Lock down rendering and implement artifacts, tied to findings 11 and 12.** Configure a strict markdown token/tag subset that rejects raw HTML, accepts only `https:` and `mailto:`, and replaces remote images with inert text. Add owner-authorized artifact downloads with sanitized attachment filename, `nosniff`, exact MIME allowlist, and 256 KiB cap.

10. **Make connection discovery deterministic, tied to finding 13.** Have Aevatar compute the complete required service set against `/proxy/services` before emitting any connect card; emit all missing cards in one assistant message. Add the `/llm/status` zero-connector path and test both conditional-gated and ungated writes.

11. **Disable unnegotiated attachments in v1, tied to finding 14.** Reject attachment/image/file input on schema version 1. Reintroduce it only with the v1.1 block schemas and size/MIME/security rules.

12. **Use the PRD fixtures as network and reducer tests, tied to all findings.** Replay Appendix A exactly, including duplicate cursors and whole-array patches. Add Appendix C G1-G6 network assertions, reload-mid-turn convergence, all terminal states, scope-subset refusal, one NyxID approval decision, no raw telemetry, and no Aevatar routes outside the three C1 endpoints.

## 6. Open questions / things you could not verify

1. I did not contact deployed NyxID, Aevatar, Ornn, OAuth providers, or downstream services. Therefore I could not verify actual production payloads, whether Aevatar treats HTTP stream abortion as cancellation, proxy approval gating/operation binding, key-status behavior, notification-channel convergence, or deployed authorization policy. The audit reports what this checkout sends, accepts, and renders.

2. There is no artifact route or fixture in the checkout, so artifact authentication and headers could only be classified as absent; there was nothing live to inspect (`nyx-chat/server.mjs:1262-1290`).

3. The full test suite could not complete in this sandbox: `jsdom` is not installed, and server integration tests cannot bind `127.0.0.1` here (`EPERM`). The 14 dependency-free `blocks`/`protocol` tests passed, and `node --check` passed for `server.mjs`, `public/app.js`, and `public/protocol.js`. These checks validate that the audited divergent behavior is intentional and syntactically sound; they do not establish v8 conformance.

4. I did not treat `design/nyxid-assistant-shell.html` or visual resemblance as protocol evidence. A card can look correct while violating the authoritative transport, action, state, and security contract; this audit is scoped to the frontend chat contract and BFF behavior that shapes it.
