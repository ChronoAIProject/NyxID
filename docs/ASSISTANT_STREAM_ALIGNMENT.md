# Assistant Chat — Reference Alignment & Open Gaps

Status of NyxID's assistant chat surface against the reference client
[`eanz17/nyxid-chat`](https://github.com/eanz17/nyxid-chat) (`main`, commit
`819dc0d`), which is the working implementation of the Aevatar chat contract.

Scope: the live AG-UI stream (frames → rendered blocks), the NyxID proxy's
streaming behaviour, and the recovery flows around them. The PRD
(`nyx-chat-prd.md`, Draft v8) remains the contract SSOT; this file records
where our implementation stands against the reference *implementation*.

Last verified: 2026-07-29 against Aevatar `origin/dev` at `020a9acd2`.

---

## 1. Frame taxonomy

The reference vocabulary lives in `public/protocol.js` (`normalizeFrame`).
Every frame family is accepted in both `type`-tagged and body-keyed shapes.

| Frame | Reference | NyxID | Rendered as |
|---|---|---|---|
| `RUN_STARTED` | ✅ | ✅ | context only |
| `RUN_FINISHED` | ✅ | ✅ | terminal `completed` |
| `RUN_ERROR` | ✅ | ✅ | terminal `failed` (message redacted) |
| `RUN_STOPPED` | ✅ | ✅ | terminal `cancelled` |
| `STEP_STARTED` / `STEP_FINISHED` | ✅ | ✅ | run-ledger step |
| `TEXT_MESSAGE_START/CONTENT/END` | ✅ | ✅ | streamed `text` block |
| `TOOL_CALL_START` / `TOOL_CALL_END` | ✅ | ✅ | transient run-ledger step |
| `TOOL_APPROVAL_REQUEST` | ✅ | ✅ | `approval_card` |
| `AUTHORIZATION_REQUIRED` | ✅ | ✅ | `connect_card` |
| `USAGE` | ✅ | ✅ | conversation model metadata |
| `MEDIA_CONTENT` | ✅ | ✅ | `artifact` block |
| `stateSnapshot` | normalized, unrendered | ignored | — |
| `CUSTOM aevatar.run.context` | ✅ | ✅ | context only |
| `CUSTOM demo.conversation.context` | ✅ | ✅ | context only |
| `CUSTOM aevatar.step.request/completed` | ✅ | ✅ | run-ledger step |
| `CUSTOM aevatar.tool_approval.pending` | ✅ | ✅ | `approval_card` + step parked |
| `CUSTOM aevatar.human_input.request` | ✅ | ✅ | `approval_card` |
| `CUSTOM aevatar.authorization.required` | ✅ | ✅ | `connect_card` |
| `CUSTOM nyxid.authorization.required` | ✅ | ✅ | `connect_card` |
| `CUSTOM nyxid.action.request` (schema v4) | ✅ | ✅ | persistent `action_card` + connect journey |
| `CUSTOM aevatar.workflow.waiting_signal` | ✅ | ✅ | turn status `waiting` |
| `CUSTOM aevatar.llm.reasoning` | ✅ (never displayed) | ✅ (never displayed) | — (PRD §3.8) |
| `CUSTOM aevatar.nyxid_chat.keepalive` | ✅ | ✅ | liveness only, not progress |
| `CUSTOM aevatar.raw.observed` → `RoleChatSessionCompletedEvent` | ✅ | ✅ | replayed into ledger + fallback text |

**Frame coverage is complete.** Unknown/newer frames are skipped without
dropping the turn (PRD §3.0 forward-compat posture).

### Intentional divergences

1. **Malformed frame handling.** The reference synthesises a
   `DEMO_PROTOCOL_ERROR` frame on JSON parse failure, which fails the turn.
   We skip the unparseable frame and continue. One corrupt frame killing an
   otherwise healthy run contradicts PRD §3.0 ("never drop, never crash"),
   so this divergence is deliberate. Trade-off: a persistently malformed
   stream surfaces as a truncation at EOF rather than an immediate error.
2. **Progress watchdog placement.** The reference enforces the 120 s
   no-progress timeout in its BFF (`DEMO_STREAM_PROGRESS_TIMEOUT_MS` →
   `UPSTREAM_PROGRESS_TIMEOUT`); NyxID has no BFF, so the same 120 s budget
   is enforced client-side in the transport. Same guarantee, different layer.
   Keepalives explicitly do not count as progress in either.
3. **Redaction shape.** The reference walks parsed objects recursively and
   replaces secret-keyed *fields*; ours redacts the serialised display
   string. Ours additionally covers bare provider token shapes (AWS
   `AKIA…`, `sk-…`, `ghp_…`, `AIza…`, `xox…`) that appear in prose error
   messages with no key to match on.

---

## 2. Streaming transport

| Concern | Reference | NyxID |
|---|---|---|
| SSE framing | incremental `data:` parse | same (shared `sse.ts`, handles `\r\n`/`\r`, split frames) |
| Final frame without trailing blank line | flushed | flushed |
| `X-Accel-Buffering: no` | set by BFF (`server.mjs:335`) | set by response-header middleware for **every** SSE surface |
| Reverse-proxy buffering | avoided | avoided (verified locally end-to-end) |
| Session id | one per conversation | one per conversation, survives history/list reprojection |
| Approval decision | POST → SSE continuation | POST → SSE continuation, cursors continue past prior turn |
| Text request body | `{type:"text", prompt, clientRequestId}` | same, with an exact allowlisted object |
| Browser action continuation | `{type:"action.continue", clientRequestId, originTurnId, actions}` | same, grouped by origin turn and streamed through the normal turn path |
| Idle stop at approval gate | pause, card stays actionable | same |

### Browser action cards

Aevatar may end a turn as blocked after emitting a `CUSTOM` frame named
`nyxid.action.request`. NyxID accepts schema version `4` and currently maps
`service.connect` to either the catalog-service or custom-service path in the
existing Add Service dialog. The card uses NyxID-owned consent copy, shows safe
request parameters, and remains interactive after the origin stream terminates.

Completing or explicitly declining a card creates a strict continuation turn:

```json
{
  "type": "action.continue",
  "clientRequestId": "<stable retry id>",
  "originTurnId": "turn-...",
  "actions": [
    {
      "actionRequestId": "act-...",
      "originTurnId": "turn-...",
      "disposition": "completed",
      "resource": { "userService": { "userServiceId": "<id>" } }
    }
  ]
}
```

Reports resolving during another local turn stay queued. Reports from one
origin turn are batched together, never mixed with another origin, and rejected
continuations retain their original `clientRequestId` for a later idle retry.
Unknown custom frames remain ignored. Unknown action verbs and non-v4 requests
render a decline-only unsupported card.

Action cards are not rehydrated after a page reload because conversation history
is currently text-only. A subsequent text turn lets Aevatar re-emit the pending
action idempotently.

### Deployment gate

The discriminated body (`type:"text"`) is **required** by Aevatar ≥
`feature/integrate` (PR aevatarAI/aevatar#2911) and **rejected** (unknown member)
by the currently deployed prod Aevatar. This branch must deploy **after** the
Aevatar dev contract reaches prod. Same for the whole action-card feature.

---

## 3. Open gaps

Ordered by user-visible impact. None are believed to block the current
surface; each states what "closed" looks like.

### G1 — No explicit retry after connecting a service (UX)

The reference preserves the original request when `AUTHORIZATION_REQUIRED`
arrives and offers a "重试请求" button once the service reports connected —
deliberately never auto-retrying, since the run may have partially executed.

NyxID renders the connect card and opens the connect wizard in place, but
the user must retype/re-send. Their message is still in the transcript, so
recovery is one action away, but it is not one click.

*Closed when:* the connect card holds the originating prompt and offers an
explicit re-send once `useKeys()` reports the service active. Must stay
explicit — never auto-retry.

### G2 — No production-assembly test for the SSE header (test coverage)

`with_response_headers()` is exercised by unit tests, and `main.rs` calls it
on the fully merged router. Nothing fails if that call is removed, moved to
one router branch, or a router is merged after it.

*Closed when:* a DB-backed integration harness boots the real app and
asserts `X-Accel-Buffering: no` on a representative SSE route. The repo has
no such harness today (`backend/tests/` does not exist).

*Interim mitigation:* the wiring is one named call instead of an inline
`.layer()`, and the local verification recipe in §4 reproduces it manually.

### G3 — Live-Aevatar frame confirmation is unverified (contract risk)

Our handling is verified against the reference contract and captured wire
fixtures (`frontend/src/lib/assistant/__fixtures__/`, captured 2026-07-16 —
a text-only run). No authed capture yet shows prod Aevatar emitting
`TOOL_CALL_*`, `TOOL_APPROVAL_REQUEST`, or `AUTHORIZATION_REQUIRED` on the
deployed `:stream` path.

*Closed when:* an authed `curl -N` against a tool-using prompt is captured
and added as a fixture. If those frames never appear, the gap is upstream
(Aevatar), not in this code.

### G4 — No attachment support in the composer (feature parity)

The reference composer accepts a file attachment and renders it on the user
message; PRD §3.5 reserves `image`/`file` client blocks for v1.1 and has v1
reject them. NyxID has no attachment affordance.

*Closed when:* v1.1 multimodal input is scheduled. Not a v1 gap.

### G5 — No health/status probe surface (diagnostics)

The reference exposes `POST /api/demo/health`, probing Aevatar capabilities
and the Ornn skill route, and renders per-component route state. NyxID has
no equivalent, so "is chat down, or is my request just slow?" is not
answerable in-product.

*Closed when:* a lightweight assistant health check surfaces upstream
reachability in the Plugins view.

### G6 — Approval decisions carry no reason (contract parity)

The reference sends `{actorId, requestId, approved, reason, sessionId}`;
we omit `reason` (the UI never collects one). Harmless while the field is
optional upstream; would matter if Aevatar starts requiring or surfacing it.

*Closed when:* the approval card collects an optional deny reason and
forwards it.

---

## 4. Verifying the streaming path locally

Reproduces the end-to-end check used for the anti-buffering fix. Requires
the dev MongoDB (`docker compose up -d mongodb`).

```bash
# 1. A timed SSE upstream: emit N frames one second apart.
# 2. Run the backend against it (PORT=3011), register a user, then:
POST /api/v1/keys  {"label":"…","endpoint_url":"http://127.0.0.1:3001",
                    "slug":"sse-verify","credential":"…",
                    "auth_method":"bearer","auth_key_name":"Authorization"}

# 3. Headers — expect `x-accel-buffering: no` + `transfer-encoding: chunked`:
curl -sS -D - -o /dev/null -N \
  "http://localhost:3011/api/v1/proxy/s/sse-verify/sse-demo" \
  -H "Authorization: Bearer $TOKEN"

# 4. Timing — expect one line per second, not five at once:
curl -sN "http://localhost:3011/api/v1/proxy/s/sse-verify/sse-demo" \
  -H "Authorization: Bearer $TOKEN" \
  | while IFS= read -r l; do [ -n "$l" ] && echo "$(date +%H:%M:%S) $l"; done
```

Against production, the same `curl -N` on the assistant stream endpoint
distinguishes the two remaining suspects: incremental arrival means the
path is healthy end to end; a single end-of-run burst with
`x-accel-buffering: no` present means the batching is upstream in Aevatar.
