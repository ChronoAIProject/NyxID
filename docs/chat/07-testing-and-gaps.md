# Assistant Chat Testing and Gaps

Last verified against Aevatar console and SDK commit `e7ba2e6eb` (2026-08-25).

The assistant is covered at the HTTP fixture, protocol/reducer, React hook and
component, Playwright, backend contract, and live producer-contract levels.
The deterministic frontend suites require neither the Rust backend nor an
external identity provider.

## HTTP fixture world

Development `?mock=1` and Vitest mode install
`globalThis.__nyxidAssistantHttpMock` from
`frontend/src/lib/assistant/assistant-http-fixtures.ts`. The fixture is mounted
inside `assistant-http.ts`, so the production page, chat API, SSE decoder,
runtime accumulator, actor reducer, history decoders, session hook, and message
components all remain in the path under test.

The fixture serves the real browser endpoints:

- JSON list, wrapped transcript, state, and delete responses;
- chunked `text/event-stream` responses for `POST /assistant/chat`;
- authoritative actor/turn identities, reasoning and text;
- step/tool lifecycles and all supported `nyxid.*` actor facts;
- approvals, inputs, plans, controls, v4 actions, authorization blockers, and
  media;
- `RUN_FINISHED`, `RUN_ERROR`, `RUN_STOPPED`, malformed frames, `[DONE]`, and
  mixed line endings.

The app-wide `frontend/src/lib/mock-data.ts` remains separate and unchanged.
The developer mock-scenarios action now selects HTTP fixture responses and
fixture-world state; it no longer intercepts a transport or emits synthetic
turn events.

The fixture fault object is `window.__nyxidAssistantHttpFaults`:

| Control | Effect |
| --- | --- |
| `historyDelayMs` | delays transcript reads |
| `historyErrorStatus` | returns a selected transcript HTTP failure |
| `sendSilent` | emits a content-free terminal |
| `aliasOnFirstSend` | assigns a new canonical actor on first send |
| `firstEventSilenceMs` | delays the first stream event |
| `progressStallMs` | stalls between meaningful frames |
| `stateEnvelopeSequence` | returns a deterministic sequence of state envelopes |
| `unauthorized` | chooses coded dead-session or uncoded upstream 401 attribution |

## Unit and component suites

The principal executable contracts are:

| Suite | Coverage |
| --- | --- |
| `sse-frame-normalizer.test.ts` | UTF-8 chunks, CR/LF variants, oneof normalization, malformed frames, `[DONE]`, media |
| `runtime-event-semantics.test.ts` | text/reasoning, steps, tools, terminals, redaction, authorization and media accumulation |
| `chat-actor-state.test.ts` | live/current-state parity, state/version monotonicity, plans, inputs, approvals, actions and controls |
| `chat-task-plan.test.ts` | strict TaskPlan and step decoding |
| `chat-history-decoders.test.ts` | strict drained index, cursor contract, wrapped transcript and extensible stored statuses |
| `chat-api.test.ts` | canonical command bodies, idempotency headers and pre-reader HTTP errors |
| `assistant-http.test.ts` | cookie auth, telemetry header, attributed 401s, fixture boundary and wire-log capture |
| `use-assistant-chat.test.tsx` | restore, identity adoption, per-conversation streaming, deadlines, watchdog, quiet Stop, 404/pending history, controls, actions and delete |
| `chat-message.test.tsx` | halo/dots lifecycle, activity, text, errors and tail following |
| `chat-message-cards.test.tsx` | artifact, connect and persistent approval composition |
| `assistant-wire-replay-view.test.tsx` | canonical normalizer/accumulator replay and partial capture handling |
| `assistant-http-fixtures.test.ts` | JSON routes, chunked streams, fixture controls and mock scenarios |
| `direct-transport.test.ts` | Direct request bounds, SSE, timeouts, cancellation, settings and identity isolation |
| `assistant-actions.test.ts` | strict `service.access_review` v4 shape, malformed resources, and exact continuation report |
| `action-registry.test.ts` | manifest closure and executable dialog binding |
| `assistant-service-access-review-dialog.test.tsx` | exact effect POST, response identity, cancel, retry, and lost-response replay |
| `action-card.wiring.test.tsx` | approve and decline wiring with safe `UserService` reports |

Component suites continue to cover the composer, sidebar, task plan, input,
approval, connect, action, artifact, Direct controls, and wire-log panel.

`frontend/src/lib/assistant/__fixtures__/aevatar-chat-history.json` uses the
canonical wrapped `{messages, stateVersion, projectionStatus}` shape. Stored
`status: "completed"` remains accepted as a settled message. The captured
`aevatar-nyxid-chat-stream.sse` fixture exercises the normalizer and projection
path.

## Playwright

`npm run test:e2e` owns the strict port-4611 Vite run. Tests open
`/assistant?mock=1`, which lazy-loads the HTTP fixture chunk and the new page.
Production builds cannot enable the fixture from the query string, and
`npm run build` runs `scripts/assert-mock-footprint.mjs` to reject the forbidden
fixture/interceptor symbols in production output.

The browser specifications are:

| File | Coverage |
| --- | --- |
| `chatting.spec.ts` | optimistic send, thinking/streaming markers, canonical reply, activity and slow-history continuity |
| `history.spec.ts` | loading, known 404 placeholder, failure notice and usable composer |
| `switching.spec.ts` | background streaming, live restoration, optimistic isolation and alias race protection |
| `new-chat.spec.ts` | navigation-only drafts, lazy identity adoption, draft migration and delete |
| `defects.spec.ts` | start/silent-turn escape, missing deep links, projection gaps, approval persistence and continuity |
| `nav.spec.ts` | shared shell and sidebar navigation |
| `wave2-service-actions.spec.ts` | full v4 service-action UI flows |

The helpers use accessible names plus `[data-assistant-halo]`,
`[data-streaming-dots]`, and `[data-empty-turn-error]`. Mutation-observer
continuity probes catch one-render gaps that final-state assertions would miss.

## Producer contract

`npm run test:producer-contract` executes
`frontend/scripts/verify-aevatar-action-wake.mjs` against a live NyxID/Aevatar
deployment. It verifies that an empty `actions: []` continuation starts a new
turn on the same typed actor.

The command is intentionally not a local stub. It requires all four live values:

```text
NYXID_URL
NYXID_ACCESS_TOKEN
NYXID_AEVATAR_ACTOR_ID
NYXID_AEVATAR_ORIGIN_TURN_ID
```

Absence of a valid access token and real actor/origin-turn IDs is an unmet
environment prerequisite, not a producer pass.

## Direct seam

The default-off Direct engine retains its existing memory-only transport and
OpenAI-compatible completion stream. `conversation-ids.ts` routes only
flag-enabled drafts and `direct-*` IDs to `use-assistant-direct.ts`; typed and
legacy IDs remain on the canonical page. Direct reuses the canonical message
list, composer, shell, tail following, and quiet local Stop, but exposes no
actor plan/input/approval/action controls. Direct conversations disappear on
reload or identity change by design.

## Backend coverage

Rust assistant service and handler tests remain authoritative for platform
service resolution, human/cookie authentication, prefix-family path selection,
strict command reconstruction, action manifest output, history multiplexing,
state resources, response headers, and upstream request bodies. Frontend tests
do not replace those server-side security boundaries.

AC-3 adds database and route coverage for:

- exact `GET /api/v1/mcp/config` admission with a matching live grant, and denial for a bare, expired, revoked, or mismatched grant;
- unchanged successful catalogs for session and ordinary access-token authentication, plus unchanged `api_v1_human_only` denial for API keys, service accounts, and relay tokens;
- identical normalized REST and JSON-RPC catalogs for one delegated token, including both MCP tool execution paths using the same proof-derived scope;
- restricted consent with empty and populated service sets, unrestricted consent, and removal of every platform row from a restricted catalog;
- the exact active `chrono-llm-public` callback and denial for another row, inactive duplicate, subject, actor, receiver, scope, or grant;
- link validation at write, startup, and mint, including removal of each required client scope in turn;
- first consent merge, exact replay, changed-request conflict, E11000 retry, simultaneous same-service and different-service merges, and preservation of unrelated consent fields;
- owner-scoped revocation for actor and receiver client roles, revoke-then-regrant within the token TTL, and denial of an old wider token after the OAuth consent page narrows consent; and
- the full pre-mutation adversarial matrix: another user's service ID, wrong slug, HTTP URI, userinfo, query, fragment, encoded path confusion, and inactive service, with database before-and-after snapshots and metadata-only audit assertions.

The assistant mint has two explicit regression states. With no Aevatar client link, chat still uses the existing unrestricted session-derived path and startup continues. With a valid link, restricted and unrestricted capabilities match current consent exactly. An unrestricted-consent user has chosen full service access, so access review is intentionally unreachable for that user.

The local runtime check uses a real NyxID process, MongoDB `rs0`, a cookie session, a locally registered linked OAuth client, and a recording upstream stub. It proves the restricted catalog omission that triggers Aevatar's `USER_SERVICE_ACCESS_REQUIRED`, the default callback, approval changing the next mint, exact retry, revocation, and cross-user pre-mutation denial. Aevatar cannot validate a token minted by local NyxID, and this repository cannot run Aevatar without .NET. Upstream action-envelope, parameter, postcondition, and continuation behavior is therefore source-proven against pinned Aevatar `e5bba2e9719ad5132004b882744caa3875db1123`, not represented as a live Aevatar result.

## Remaining operational gaps

- Live producer verification needs the credentialed environment above and is a
  manual contract check, not a credentialed CI job.
- A linked restricted rollout requires the active OAuth client's `delegation_scopes` to contain both `proxy:*` and `mcp:catalog:read`. The no-link state remains compatible, but it is unrestricted by design.
- Chat history does not serialize `MEDIA_CONTENT`, so live artifacts are not
  restored after reload.
- Terminal settlement does not automatically reconcile the local transcript
  against a fresh server history read.
- `nyxid.authorization.required` connect cards are live-only and are not
  reconstructed from transcript history or typed actor state.
- Wire-log replay is diagnostic and inert; it does not mount approval,
  connection, action, or actor-control side effects.
- The Direct engine is stateless and memory-only. Its seam deliberately carries
  a separate text-only turn reducer rather than using the typed actor reducer;
  it is not a durable-history alternative.
