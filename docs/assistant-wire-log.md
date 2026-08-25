# Assistant Aevatar Wire Log

The assistant chat exposes a flag-gated diagnostic panel for inspecting the
HTTP exchanges NyxID makes through the managed Aevatar service. It is gated by
the `experimental:aevatar-chat-wire-log` runtime feature flag, which defaults
to off; where it is enabled, each authenticated assistant caller can opt into
capturing their own exchanges with the panel's **Capture** switch.

The wire log is diagnostic data, not an audit log. Backend echoes normally live
in a short-lived MongoDB record and are fetched only when a panel row expands.
Browser storage contains entry metadata and, only on the MongoDB-failure
fallback path, the bounded inline echo. Fetched echoes, delivered response
bodies, and SSE captures remain in browser memory only.

## Accuracy Contract

The panel keeps three sources of truth separate and labels each one honestly.

### Upstream request echo

This is the reconstructed request body NyxID assembled for Aevatar, captured
server-side before credential and identity injection. Its headers are only the
fixed allowlisted subset `content-type`, `idempotency-key`, and `accept`.

It is not a byte-for-byte network capture and does not claim to contain every
request header. Authorization, cookies, identity propagation, delegation
tokens, proxy defaults, and downstream credentials are intentionally absent.

### Upstream response metadata

This is metadata from Aevatar's response as observed by the NyxID backend:

- HTTP status
- whether the content type is RFC-compatible `text/event-stream`
- the first value of each allowlisted header: `content-type`, `x-request-id`,
  and `x-correlation-id`

Header values are capped at 256 UTF-8 bytes, with truncation reported per
value. NyxID never echoes `set-cookie`, authorization-shaped headers, identity
headers, delegation headers, or the wider proxy response-header set.

### Delivered response capture

This is the decoded response entity exposed by Fetch to the browser after
NyxID and intermediaries have handled it. It is labeled **as delivered by
NyxID**, never "as sent by Aevatar."

For SSE responses, the capture preserves every decoded line and its exact line
ending (`LF`, `CRLF`, bare `CR`, or an unterminated tail). Invalid UTF-8 becomes
U+FFFD. The byte counter records received bytes before decoding. This is not a
literal upstream-octet capture.

## Gates And Header Protocol

Both gates must pass:

1. The `experimental:aevatar-chat-wire-log` feature flag must be enabled for
   the calling user. It defaults to **off** in the code registry and is
   toggled at runtime — platform-wide, for a staff-selected org cohort, or per
   user — through the platform-admin feature-flag API
   (`PUT /api/v1/admin/feature-flags/{flag_key}`). No redeploy or restart is
   involved: the next request resolves the new value. Assistant chat is a
   personal surface, so resolution uses the same grant-union chain as
   `/users/me`: a personal user override takes precedence over the
   global/default value, while any organization grant enables the flag.
2. The authenticated caller must enable the per-browser **Capture** switch,
   which causes the frontend to send `X-NyxID-Debug-Upstream: 1`.

The backend evaluates these in the **opposite** order to the list above, and
that ordering is deliberate. The request-header check is a free in-memory
lookup; resolving the flag costs a MongoDB read. Normal chat traffic never
sends the header, so the header is checked first and that traffic performs
zero additional database work. A flag-resolution failure fails **closed**: no
echo is emitted.

The feature flag is the sole authorization gate for the diagnostic and does
not require a platform-admin role — a platform admin decides _who_ gets it,
but an enabled non-admin caller uses it exactly like anyone else. The backend
echo is constructed only from the assistant request currently being handled,
so a caller can capture their own exchange but can never observe another
caller's traffic.

Raw captures and replay placeholder JSON bypass the chat renderer's credential
redaction and may contain sensitive upstream payloads verbatim. Leave the flag
off for any user or environment where browser retention of unredacted raw
payloads is unacceptable.

Except for the conversation-list route, handler paths that return a final
`Response` through the assistant echo attachment path first store the selected
echo envelope in MongoDB. When that succeeds, NyxID sends its UUID in
`X-NyxID-Debug-Upstream-Id` and does not send an inline log header. The UUID
value is about 36 bytes regardless of the number or size of captured echoes.

Expanding an ID-backed panel row lazily calls:

```text
GET /api/v1/assistant/wire-logs/{id}
```

The response is:

```json
{
  "id": "8c38949c-69c4-4be2-92ab-b451e2bda321",
  "conversation_id": "nyxchat-example",
  "created_at": "2026-08-20T12:00:00Z",
  "payload": {
    "version": 2,
    "echoes": [
      {
        "degraded": true,
        "method": "POST",
        "path": "api/chat",
        "commandType": "text",
        "upstreamOutcome": "response",
        "status": 200
      }
    ],
    "droppedEchoCount": 0
  }
}
```

Records expire 15 minutes after creation. The fetch filters on both `_id` and
the authenticated owner and also checks `expires_at` directly instead of
waiting for MongoDB's TTL sweep. A malformed or unknown ID, another user's ID,
an expired record, a disabled feature flag, or a flag-resolution failure all
return the same not-found response. The route is outside assistant billing and
the frontend suppresses the debug request header on this endpoint, so fetching
a wire log cannot recursively create another wire log.

If envelope selection, serialization, or MongoDB insertion fails, NyxID
attempts a bounded inline fallback in the legacy
`X-NyxID-Debug-Upstream-Log` response header. It contains a Base64-encoded
UTF-8 JSON envelope capped at 4 KiB. A response carries either the ID header or
the inline fallback header, never both. If no envelope fits the inline budget,
NyxID emits neither debug header. The fallback path never fails the assistant
request.

`GET /api/v1/assistant/conversations` is deliberately silent even when both
capture gates pass: it sends neither debug response header and does not create
a stored record. This suppression also covers the frontend's background list
membership reads and removes wire-log header growth from the list path
entirely.

The stored and inline payload envelope remains version 2:

```json
{
  "version": 2,
  "echoes": [
    {
      "degraded": true,
      "method": "POST",
      "path": "api/chat",
      "commandType": "text",
      "upstreamOutcome": "response",
      "status": 200
    }
  ],
  "droppedEchoCount": 0
}
```

The frontend also accepts the legacy bare-array payload. It does not infer an
upstream outcome for legacy echoes because an older list fallback could record
an attempted request whose proxy call subsequently failed.

### Version skew

Compatibility is one-directional by design:

- **New frontend + backend that does not know the flag key** — the key is
  absent from `capabilities.enabled_features` on `/users/me`, so the panel
  treats it as disabled and stays hidden. This is fail-closed and self-heals
  once the browser reaches a backend whose registry carries the flag. The
  frontend re-reads `/users/me` on its existing one-minute interval, so a
  runtime toggle reaches the panel without a hard reload.
- **New frontend + flag-aware backend with the old echo payload** — supported.
  The decoder accepts the legacy bare array and normalises it, leaving
  `upstreamOutcome` and `response` absent.
- **Old frontend + new backend** — a pre-ID frontend ignores the healthy-path
  ID header and therefore shows no entry. A version-2-capable frontend can
  still decode the inline header if the backend takes the MongoDB-failure
  fallback path.

The panel-hidden and header-format cases are transient, authenticated-caller,
capture-only diagnostic gaps during a rolling deploy. They clear once the
browser reaches the matching backend and frontend bundle. They do not affect
chat delivery, and no other surface reads either debug response header. The
version 2 payload contract remains the same across stored and fallback
transport so degradation does not introduce a second envelope format.

## Echo Shapes

Every version 2 echo is one member of a discriminated union.

### Full echo

```json
{
  "degraded": false,
  "method": "POST",
  "path": "api/chat",
  "commandType": "text",
  "body": {},
  "headers": {},
  "identity": {
    "mode": "jwt",
    "forward_access_token": false,
    "inject_delegation_token": true,
    "bridge_minted": false
  },
  "truncated": false,
  "upstreamOutcome": "response",
  "response": {
    "status": 200,
    "headers": {},
    "sse": true
  }
}
```

`upstreamOutcome` is optional and may be `response`, `no_response`, or
`unknown`. `no_response` means the backend did not obtain a concrete upstream
`Response`; it does not expose an internal error. An absent or `unknown` value
means the backend did not report the outcome, and the frontend must not guess.

### Minimal echo

```json
{
  "degraded": true,
  "method": "POST",
  "path": "api/chat",
  "commandType": "text",
  "upstreamOutcome": "response",
  "status": 200
}
```

## Payload Size Degradation

NyxID applies the same deterministic ladder to two explicit budgets. The
normal stored envelope is capped at 1 MiB of plain JSON. If storage fails, the
inline fallback repeats selection against a 4 KiB cap measured on the final
Base64 header value:

1. Binary-search truncate the largest request bodies.
2. Replace request bodies with `null` and mark them truncated.
3. Remove request-header maps and response-header maps, retaining identity,
   response status, and SSE classification.
4. Convert all echoes to the minimal union member.
5. Retain the first eight echoes in chronological order and report the number
   removed in `droppedEchoCount`.

Retained paths are capped at 256 bytes, command types at 64 bytes, and header
values at 256 bytes on UTF-8 boundaries. The backend test suite proves the
inline header never exceeds 4 KiB. The true worst-case eight-minimal-echo
wrapper does not fit that budget, so it is rejected and no debug header is
emitted. The healthy path emits only the UUID header, and a stored document
that exceeds the 1 MiB service limit is rejected and attempts the same bounded
inline fallback.

## Browser Exchange Model

One browser HTTP exchange creates one panel entry only after a valid backend
wire-log ID or inline echo is received. Client captures can attach to that
backend-created exchange; they can never create an entry independently.

All assistant requests now pass through
`frontend/src/lib/assistant/assistant-http.ts`. When both gates are active it
adds the debug header, records the backend ID/inline envelope, and reads a
`Response.clone()` into `WireBodyCapture`. The original response remains owned
by `chat-api.ts` and the canonical SSE normalizer. Capture cannot reorder,
delay, or consume live frames.

An exchange records:

- a client-derived label and transport kind
- the NyxID-to-browser status
- the owning conversation ID, or `null` for unattributed routes such as
  completions
- the server wire-log ID, or a bounded inline fallback envelope
- an optional session-only delivered capture

New HTTP captures settle as `complete` or `network_error`. The schema retains
the older `cancelled`, `worker_error`, and `protocol_cancel` values only for
already-materialized diagnostic entries. Truncation is independent from the
outcome. `WireBodyCapture` counts all received bytes, decodes UTF-8 with
replacement for malformed sequences, and retains at most 4 MiB. Reaching the
cap cancels only the cloned reader.

`POST /assistant/chat` is initially unattributed because its authoritative ID
is inside `RUN_STARTED`. The orchestrator assigns the captured exchange to the
adopted conversation as soon as that frame passes identity validation.

The Zustand persistence schema is version 3 under the existing
`nyxid.assistant.wirelog.v1` local-storage key. Pre-v3 entries are discarded.
The 100-entry, 2 MiB persistence budget covers metadata and inline fallback
envelopes only; payloads fetched by ID are never written to local storage. The
separate session capture budget is 4 MiB; oldest captures become `evicted`
stubs without removing persisted request history. The immutable TanStack Query
cache is keyed by wire-log ID, so collapse and re-expansion reuse one lazy fetch
for the life of that browser query cache. Logout and authenticated-user changes
reset the browser wire-log store so metadata or inline fallback payloads cannot
cross owners.

## Panel Views

The panel receives the active conversation ID from the assistant page and, by
default, shows only entries attributed to that conversation. The **All
conversations** switch reveals every retained entry, including entries whose
conversation ID is `null`. Rows render immediately from metadata: timestamp,
transport kind, NyxID status, and the client-derived route label.

Expanding an ID-backed row starts its lazy fetch and renders loading, loaded,
expired-or-unavailable, or generic error state. Inline fallback rows render
their envelope directly without a fetch. Loaded echoes use the same request,
response-metadata, raw SSE, delivered-response, and replay views as before.

The **Responses** switch at the top of the panel hides or shows every
response-derived surface, including Aevatar status badges, backend-observed
response metadata, delivered response bodies, raw SSE, and the Rendered view.

Raw SSE is derived from the retained body and windowed in 200-line increments.

Rendered replay passes the captured body through the same
`SsePayloadDecoder`, `normalizeBackendSseFrame`, runtime accumulator, and actor
reducer as live chat. It carries capture outcome and truncation into partial
replay handling. Text, reasoning, step/tool activity, errors, and media use the
production message renderer. Actor facts are shown separately as diagnostic
JSON. They never mount task, input, approval, connection, action, navigation,
query, or store side effects; connect cards are disabled in replay.

Raw captures, fetched request echoes, and placeholder JSON may contain
sensitive upstream payloads verbatim. They bypass the chat renderer's
credential redaction. Echo construction still occurs before credential and
identity injection and uses fixed request/response header allowlists, so
stored payloads do not contain injected authorization values, cookies,
delegation tokens, or downstream credentials.

Where the feature flag is enabled, any authenticated assistant caller — admin
or not — can capture and fetch only their own exchanges; no caller can observe
another user's traffic. Stored echoes expire after 15 minutes. Fetched echoes
and raw delivered captures are excluded from local-storage persistence,
telemetry, audit logs, and crash reports. Backend warnings and tracing around
storage and fetch contain metadata such as IDs, byte counts, and outcomes, not
payload bodies. Fetched echoes enter the browser only through the owner-scoped
endpoint; browser-held data leaves only through an explicit copy action. Leave
the flag off for any user or environment where short-lived retention of
unredacted request payloads is unacceptable.

## Deliberate Non-Goals

- The WebSocket workflow channel is not captured.
- The conversation-list route never captures or emits a wire log, even when
  the feature and browser capture gates are enabled.
- A final `AppError` has no assistant debug echo because no final handler
  `Response` passes through the attachment path.
- Production interactive actor cards are not replayed. Actor facts remain
  diagnostic JSON.
- Literal upstream response octets are not captured or persisted server-side.
  The delivered browser entity is the privacy-preserving diagnostic boundary.
