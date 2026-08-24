# Platform Operations V1 — Spec

**Decision (owner):** platform capabilities are forced into a single NyxID-owned surface of
named, server-constructed operations — the chrono-llm pattern (`services/assistant_direct.rs`:
`deny_unknown_fields` request structs, allowlisted values, server-composed upstream body) —
rather than config-row proxying. Rationale: platform services must be scope-limited and
non-destructive; the generic allowlist cannot see request bodies, and for Twilio/ElevenLabs the
danger lives in the body (`To`, `Body`, `Twiml`, `voice_id`). V1 operations: **Twilio,
ElevenLabs, X (Twitter)**. Admin-configurable per operation.

## Architecture

- **Vendor credentials** stay on admin-created internal `DownstreamService` rows (one per
  vendor: `platform-twilio`, `platform-elevenlabs`, `platform-x`), created via the existing
  admin API with `service_category: "internal"`, `visibility: "public"`, a master credential,
  and **no** `proxy_operation_policy` (these rows are never caller-addressed; the flag-off
  server-chosen path resolves them, same as `chrono-llm-public`). Handlers resolve them with
  `assistant_service::resolve_admin_service_by_slug` + the server-chosen credential gate —
  callers can never name a vendor path.
- **New model `PlatformOperation`** (collection `platform_operations`, UUID-string `_id`,
  `COLLECTION_NAME`): `op` (unique key: `"x_search" | "speak" | "call_and_say"`), `enabled:
  bool` (default false), `vendor_service_slug: String`, `config: <typed per-op struct>`,
  `updated_at`, `updated_by`. Serde-tagged per-op config enum, `deny_unknown_fields`:
  - `x_search`: `{ max_results_cap: u32 (default 10, hard cap 25) }`
  - `speak`: `{ allowed_voice_ids: Vec<String> (non-empty required), max_chars: u32 (default
    1000, hard cap 5000), model_id: String (default "eleven_multilingual_v2") }`
  - `call_and_say`: `{ allowed_destination_prefixes: Vec<String> (E.164 prefixes, e.g. "+65";
    empty = deny all), max_message_chars: u32 (default 500, hard cap 1000), voice: String
    (default "alice"), max_calls_per_user_per_day: u32 (default 3) }`
- **Fail closed everywhere**: op missing or `enabled: false` → 404 not-found-shaped; vendor row
  missing/invalid → 502-shaped internal error, never a fallback; `call_and_say` with empty
  prefixes → every call denied.

## Operations (HTTP first; MCP publication in the same PR)

Routes under `/api/v1/platform-ops`, JWT session users and `nyxid_ag_` API keys; delegated,
relay, and service-account tokens rejected before the handler (existing `reject_relay_tokens`
pattern). Request structs `deny_unknown_fields`. Per-op audit events are **metadata-only**
(op, user_id, api_key_id, sizes/durations, outcome — never message text, never audio, never
destination beyond a redacted suffix).

1. **`POST /platform-ops/x-search`** `{ query: String (1..=512), max_results: Option<u32> }` →
   forwards `GET {base}/2/tweets/search/recent?query=...&max_results=min(req, cap)` with the
   vendor bearer. Response: passthrough JSON body.
2. **`POST /platform-ops/speak`** `{ text: String (1..=max_chars), voice_id: String }` →
   `voice_id` must be in `allowed_voice_ids` (400 otherwise, naming allowed values, matching
   assistant_direct's error style). Forwards `POST {base}/v1/text-to-speech/{voice_id}` with
   server-built body `{ text, model_id }`. Response: `audio/mpeg` streamed through.
3. **`POST /platform-ops/call-and-say`** `{ to: String (E.164), message: String
   (1..=max_message_chars) }` → `to` must match an allowed prefix; per-user daily counter
   (Mongo, `platform_op_usage` collection, `{op, user_id, yyyymmdd}` unique, `$inc` guarded by
   the cap) enforced **before** the vendor call. NyxID composes inline TwiML
   `<Response><Say voice="...">{xml-escaped message}</Say></Response>` and posts
   `To`, `From` (from config? No — **from the vendor row**: store the platform caller-ID as
   `call_from: String` in the op config, required non-empty), and `Twiml` to
   `POST {base}/2010-04-01/Accounts/{AccountSid}/Calls.json`. `AccountSid` comes from op config
   (`account_sid: String`, required), never from the caller. No `Url`, no `StatusCallback`, no
   recording. XML-escape the message; reject control characters.

MCP: register three first-party tools (`nyx__x_search`, `nyx__speak`, `nyx__call_and_say`) in
the existing `nyx__` registry, delegating to the same service-layer functions (not HTTP
round-trips). Disabled ops are absent from `tools/list`.

## Admin surface

- **Backend:** `GET /api/v1/admin/platform-ops` (list, full config), `PUT
  /api/v1/admin/platform-ops/{op}` (upsert config; validates per-op struct; hard caps enforced
  server-side; audit event on change). Admin role required. No delete — `enabled: false` is off.
- **Frontend:** one admin page (`/admin/platform-ops`) listing the three ops with an
  enable/disable switch and a per-op form (typed fields, not raw JSON): X (max results), speech
  (voice ids as chips, max chars), call (prefixes as chips, caller id, account sid, message cap,
  daily cap). Follow existing admin page + `useAppForm` patterns; schema in
  `frontend/src/schemas/`; hook in `frontend/src/hooks/`.

## Non-goals (named so they are not rediscovered)

Billing/metering (deferred by owner). Phone-ownership verification (destination control is the
prefix allowlist in v1). SMS (excluded by owner decision). ElevenLabs realtime/convai (out of
scope; frame protocols cannot be policed). Chat-assistant visibility (the tool-calling POC was
removed in `3afe8141`; MCP is the v1 agent surface).

## Testing

Unit: per-op config validation (hard caps, empty-prefix deny, unknown fields rejected), TwiML
composition (XML escaping incl. `]]>`, control chars), voice-id rejection message, destination
prefix matching (exact-prefix, not substring). Handler (Mongo-gated, CI runs them): disabled op
→ 404; enabled op with missing vendor row → error, no panic; daily cap: N allowed, N+1 refused,
counter not incremented on vendor failure; delegated/relay tokens rejected. `cargo check`,
`clippy`, `fmt` clean locally; Docker is unavailable locally so the suite runs in CI only.

## Sequencing for the PR

1. Model + config validation + service layer (request construction, pure functions where
   possible so tests need no Mongo).
2. HTTP handlers + routes + auth posture + audit.
3. Admin endpoints.
4. MCP tool registration.
5. Frontend admin page.
6. Tests throughout; PR against `main` stating CI is the first suite execution.
