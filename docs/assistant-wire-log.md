# Assistant Aevatar Wire Log

The assistant chat exposes an operator-gated diagnostic panel for inspecting
the HTTP exchanges NyxID makes through the managed Aevatar service. The
fleet-wide feature is disabled by default; when enabled, each authenticated
assistant caller can opt into capturing their own exchanges with the panel's
**Capture** switch.

The wire log is diagnostic data, not an audit log. Backend echoes may persist
in browser storage, while delivered response bodies and SSE captures remain in
session memory only.

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

Both gates must pass, in this precedence order:

1. The operator must set `AEVATAR_CHAT_WIRE_LOG_ENABLED=true`. It defaults to
   `false` and is evaluated before request-header inspection or any database
   lookup.
2. The authenticated caller must enable the per-browser **Capture** switch,
   which causes the frontend to send `X-NyxID-Debug-Upstream: 1`.

The feature flag is the sole authorization gate for the diagnostic and does
not require a platform-admin role. The backend echo is constructed only from
the assistant request currently being handled, so a caller can capture their
own exchange but can never observe another caller's traffic.

On handler paths that return a final `Response` through the assistant echo
attachment path, NyxID sends a Base64-encoded UTF-8 JSON value in
`X-NyxID-Debug-Upstream-Log`.

The current payload is version 2:

```json
{
  "version": 2,
  "echoes": [],
  "droppedEchoCount": 0
}
```

The frontend also accepts the legacy bare-array payload. It does not infer an
upstream outcome for legacy echoes because an older list fallback could record
an attempted request whose proxy call subsequently failed.

### Version skew

Compatibility is one-directional by design:

- **New frontend + backend without the public feature-flag field** — the field
  is treated as disabled and the panel stays hidden. This is fail-closed and
  self-heals once the browser reaches a backend that advertises the enabled
  flag in `/public/config`.
- **New frontend + flag-aware backend with the old echo payload** — supported.
  The decoder accepts the legacy bare array and normalises it, leaving
  `upstreamOutcome` and `response` absent.
- **Old frontend + new backend** — the panel shows no entries. A pre-version-2
  frontend expects a top-level JSON array and rejects the version 2 wrapper, so
  `captureAssistantWireLogHeader` discards the header.

The panel-hidden and header-format cases are transient,
authenticated-caller, capture-only diagnostic gaps during a rolling deploy.
They clear once the browser reaches the matching backend and frontend bundle.
They do not affect chat delivery, and no other surface reads the debug header.
The wrapper is deliberately *not* emitted conditionally to preserve
old-frontend parsing: a dual wire format would leave the degraded path
exercised only in rare production cases, which is worse than a self-healing
empty panel.

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

## Header Size Degradation

The encoded header value is capped at 12 KiB. NyxID applies this deterministic
ladder until the value fits:

1. Binary-search truncate the largest request bodies.
2. Replace request bodies with `null` and mark them truncated.
3. Remove request-header maps and response-header maps, retaining identity,
   response status, and SSE classification.
4. Convert all echoes to the minimal union member.
5. Retain the first eight echoes in chronological order and report the number
   removed in `droppedEchoCount`.

Retained paths are capped at 256 bytes, command types at 64 bytes, and header
values at 256 bytes on UTF-8 boundaries. The backend test suite proves the
worst-case eight-minimal-echo wrapper remains below the 12 KiB value cap with
aggregate response-header headroom.

## Browser Exchange Model

One browser HTTP exchange creates one panel entry only after a valid backend
echo is decoded. Client captures can attach to that backend-created exchange;
they can never create an entry independently.

An exchange records:

- the NyxID-to-browser status
- one or more chronological upstream request echoes
- any count removed by degradation
- an optional session-only delivered capture

Capture terminal outcomes are `complete`, `cancelled`, `network_error`,
`worker_error`, or `protocol_cancel`. Truncation is independent of the terminal
outcome: a stream may complete normally after exceeding the capture limit.

The worker and inline fallback use the same independent tee. The tee does not
change the bytes passed to the live AG-UI parser, frame ordering, or frame
callback behavior. With capture disabled, cancellation still settles locally
without a worker acknowledgement, HTTP error text still uses the original
character-bounded `Response.text()` path, and a successful response without a
body still reports `stream_closed` to live chat. With capture enabled, request
retirement waits for the diagnostic wire flush acknowledgement so retained
data is not lost; the live cancellation and bodyless-response results remain
the same. Wire callbacks are isolated from frame callbacks.

The per-response limits are:

- 512 KiB retained decoded SSE line data
- 64 KiB retained non-SSE or error response data
- 32 KiB maximum serialized worker wire message, using line fragments when a
  single logical line exceeds the message limit

The persisted request-echo envelope budget is 2 MiB. The separate session
capture budget is 4 MiB; oldest captures become `evicted` stubs without
removing persisted request history.

## Panel Views

The **Responses** switch at the top of the panel hides or shows every
response-derived surface, including Aevatar status badges, backend-observed
response metadata, delivered response bodies, raw SSE, and the Rendered view.

Raw SSE renders as individual decoded line items. The DOM is windowed in
200-line increments so a capture containing many short or blank lines does not
mount every retained line at once.

Rendered replay uses the production SSE parser, turn-event reducer, and text
Markdown renderer. It derives actor or workflow ordering rules from the
exchange and carries capture outcome and truncation into EOF handling. The
replay projector is diagnostic-only; parity tests compare its event sequence
and final reduced messages against the real transport for actor and workflow
fixtures.

Only text uses the production chat renderer. Run ledgers, connection cards,
action cards, approval cards, and media are inert placeholders. Each
placeholder shows the original source frame JSON from the replay sidecar and
is explicitly marked "Not replayed." No live controls, queries, navigation, or
assistant-store writes are mounted by replay.

Raw captures and placeholder JSON may contain sensitive upstream payloads
verbatim. They bypass the chat renderer's credential redaction. When the
operator flag is enabled, any authenticated assistant caller can capture their
own exchanges; no caller can observe another user's traffic. Raw captures are
session-only, excluded from persistence, telemetry, audit logs, and crash
reports, and leave the browser only through an explicit copy action. Keep the
operator flag off in environments where browser retention of unredacted raw
payloads is unacceptable.

## Deliberate Non-Goals

- The WebSocket workflow channel is not captured.
- A final `AppError` has no assistant debug echo because no final handler
  `Response` passes through the attachment path. This includes post-upstream
  failures such as a conversation-list response exceeding its buffer cap.
- Production interactive cards are not replayed. Read-only card variants are
  a possible follow-up.
- Literal upstream response octets are not captured or persisted server-side.
  The delivered browser entity is the privacy-preserving diagnostic boundary.
