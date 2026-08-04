# Assistant Chat Testing and Gaps

Last verified against `fix-new-chat-timing` (2026-08-04).

The assistant surface is covered at four levels: browser flow tests against a deterministic transport, React hook and component probes, transport/protocol unit tests, and Rust handler/service tests against reconstructed upstream calls. Live deployment capture remains a separate operational requirement.

## Playwright browser harness

`frontend/playwright.config.ts` runs the real Vite application in Desktop Chromium on strict port `4611`.

The harness uses:

- `fullyParallel: true`;
- three workers;
- no retries;
- 45-second test timeout;
- retained trace on failure; and
- a Vite server started with `--strictPort`.

Tests open `/assistant?mock=1`. In a development build, `frontend/src/lib/assistant/transport.ts` selects `MockAssistantTransport`; production builds ignore the query switch and use the Aevatar transport. The mock route seeds an authenticated user before load, so browser flows require neither the Rust backend nor an external identity provider.

Mock turns are scripted with a deterministic 100-millisecond event cadence. They exercise the same query, hook, page, thread, composer, sidebar, and block components as the real transport.

### Flow specifications

The browser suite is split by visible behavior:

| File | Coverage |
| --- | --- |
| `frontend/e2e/chatting.spec.ts` | optimistic send, thinking-to-stream transition, reply, tool activity, slow history continuity |
| `frontend/e2e/new-chat.spec.ts` | navigation-only New chat, lazy first allocation, alias adoption, draft migration, delete behavior |
| `frontend/e2e/history.spec.ts` | history loading, 404 materialization posture, non-404 failure notice, continued usability |
| `frontend/e2e/switching.spec.ts` | conversation switching, per-conversation state, alias continuity, cancellation and empty-error isolation |
| `frontend/e2e/nav.spec.ts` | assistant shell/sidebar navigation behavior |
| `frontend/e2e/defects.spec.ts` | regression cases for missing history, silent turns, slow projection, loader/empty-state gaps |

`frontend/e2e/helpers.ts` addresses the page through accessible roles, visible text, and the semantic markers `[data-assistant-halo]`, `[data-streaming-dots]`, and `[data-empty-turn-error]`. Selectors do not depend on Tailwind classes or visual implementation details.

### Continuity probe

Ordinary browser assertions can miss a one-render flash that repairs itself before Playwright polls. `observeTurnContinuity` installs a `MutationObserver` and records every committed reader-visible state during a turn.

It tracks whether:

- the optimistic user message appeared;
- a thinking/loading state appeared;
- the reply appeared;
- the screen bounced back to `Start a new conversation` after the user message;
- loading disappeared before the reply arrived;
- loading reappeared after the reply; or
- the empty state appeared after the reply.

The probe is read and disconnected at the end of the flow. It tests temporal continuity rather than only the final DOM.

## Mock fault injection

Before navigation, a test may set `window.__assistantMockFaults`. `AssistantMockFaults` exposes four controls:

| Control | Effect |
| --- | --- |
| `historyDelayMs` | delays transcript reads to expose projection and loading races |
| `historyErrorStatus` | makes transcript reads fail with a selected HTTP status |
| `sendSilent` | completes a turn without printable assistant output |
| `aliasOnFirstSend` | changes the first local conversation identity to exercise placeholder-to-canonical adoption |

The controls are deliberately behavioral, not DOM-specific. They let the same page reproduce history materialization, empty-terminal, and identity-alias boundaries deterministically.

Implementation: `frontend/src/lib/assistant/transport.ts:AssistantMockFaults`, `MockAssistantTransport`, and `frontend/src/lib/assistant/mock-data.ts`.

## Hook probes

The hook tests drive a transport double through TanStack Query and observe query cache, episode, mutation, and projection behavior.

### `use-assistant.test.tsx`

This suite covers:

- optimistic user messages and progressive reply projection;
- episode opening and terminal closure;
- restoration after a rejected concurrent send;
- automatic first-conversation allocation;
- single-flight allocation across racing sends and New chat;
- asynchronous stream-failure notification after mutation resolution;
- adaptive projection cadence and burst coalescing;
- cancellation and prevention of late writes;
- error copy for downstream auth, active turn, history, and generic failures;
- typed not-found no-retry behavior; and
- projection reconciliation, canonical-key invalidation, and waiter release;
- continued streaming when history is missing or fails.

### `use-assistant.audit.test.tsx`

This suite is the focused ownership and timing probe. It covers:

- active episode ownership across rejected concurrency;
- placeholder history continuity and canonical list identity;
- stopping through placeholder and canonical aliases;
- distinct durable IDs for concurrent pending drafts;
- typed tombstone, unknown, and unrecoverable-placeholder reads;
- the 30-second first-event deadline;
- the same start deadline for approval continuation;
- the 5-second projection deadline;
- approval episode disowning and rollback; and
- thinking/empty-state semantics for continuation gaps.

### `use-assistant.aevatar.test.tsx`

This suite uses the real Aevatar transport with controlled history/stream responses. It verifies that text-only server history does not erase local structured cards during materialization. Cases include:

- local transcript longer than server text;
- card-only blocked turns anchored after their own server rows;
- multiple structured turns retaining order;
- later history materialization moving activity to the correct turn anchor;
- terminal 404 followed by materialization; and
- preservation of an upstream empty text shell around an action terminal.

## Transport and protocol tests

`frontend/src/lib/assistant/aevatar-transport.test.ts` is the broad executable wire contract. It verifies, among other behavior:

- exact browser URLs, JSON bodies, headers, and typed discriminators;
- no browser-supplied scope on the wire;
- list/index mapping without per-row fan-out;
- legacy-array and wrapped transcript decoding;
- strict malformed-history failure;
- typed turn identity and replay idempotency;
- stop, pre-start cancel fences, approval ordering, and delete reservation;
- SSE terminals, truncated EOF, duplicate terminals, body-keyed frames, and final unterminated frame flush;
- tool, step, approval, authorization, media, usage, and observed completion projection;
- keepalive-insensitive watchdog behavior;
- error and tool-result redaction;
- v4 action-card validation, duplicate/conflict behavior, blocked re-arm, report batching, post-report copy, and delivery requeue;
- Studio create body, continuation body, stable session ID, and positive fence;
- reservation-specific 503 refresh and retry;
- no Studio continuation replay after accepted stream truncation;
- create recovery entry points, poll validation, history reconciliation, scope guard, alias adoption, terminal-kind preservation, and fail-closed exhaustion;
- workflow client-only cancellation and workflow delete;
- account-scope reset and abandoned-retry settlement, receipt-backed cold reads, raw-index evidence, active/pending mirror suppression, projection materialization, and pre-alias deletion intents;
- new-chat keep-max convergence and retention at current, missing-turn, below-fence, legacy, and live-turn boundaries;
- receiptless placeholder-alias DELETE, delete-before-context canonical cleanup, and persisted-intent reload sweeping; and
- refusal to send typed actor approval/wake controls to a `chatc-...` identity.

Captured fixtures in `frontend/src/lib/assistant/__fixtures__/aevatar-nyxid-chat-stream.sse` and `aevatar-chat-history.json` are replayed through awkward byte chunks to exercise incremental parsing and history mapping.

Supporting unit suites isolate the components:

- `sse.test.ts` and `chat-stream-parser.test.ts`: newline normalization, multi-data joining, malformed payloads, chunk boundaries, and final flush;
- `chat-stream.worker.test.ts`, `chat-stream-worker-client.test.ts`: worker batching, cancellation, errors, and inline fallback behavior;
- `stream.test.ts`: turn-event reducer invariants;
- `assistant-actions.test.ts`: strict action/request/report schemas and secret rejection;
- `canonical-command-guard.test.ts`: canonical typed command route/body use;
- `transport.test.ts` and `mock-data.test.ts`: transport selection and deterministic mock behavior;
- component tests for thread, composer, sidebar, text, run, approval, connect, action, and artifact rendering;
- draft, context, and wire-log store tests;
- `assistant-receipts.test.ts` and `assistant-receipt-store.test.ts`: corrupt entry rejection, positive fences, timestamp skew, per-account namespaces, independent caps, storage fallback, cross-tab rehydration, and materialization-timer deduplication;
- `backoff.test.ts`: nonzero jitter floor and capped deadline-spanning delays; and
- `frontend/src/pages/assistant.test.tsx`: page-level selection, send, and state composition.

## Aevatar wire-capture runbook

Use this runbook for a turn that stops rendering, closes without printable
content, or appears only after a browser refresh. A Network-panel request list
is not sufficient: HTTP 200 proves that response headers arrived, not that the
event-stream body completed or contained a printable assistant event.

### Enable and capture

1. Add `experimental:aevatar-chat-wire-log` to the authenticated account's
   `capabilities.enabled_features`. The capability is server-driven and may be
   enabled for an ordinary user; a role or local-storage override does not
   enable the panel.
2. Reload the Assistant page and open the `Aevatar wire log` action.
3. Enable capture and Responses before sending. Capture exactly one failing
   turn so the exchange, terminal, and first History observation remain easy
   to correlate.
4. Leave the conversation open until projection either materializes the reply
   or reaches its bounded stalled state. Then export the wire log. If a refresh
   is needed to reveal the reply, export before and after the refresh and note
   the approximate refresh time.
5. Send the export to the Aevatar team with the matching NyxID request time,
   conversation id, and turn id when one was announced. Never paste prompts,
   replies, credentials, authorization headers, or raw payloads into tickets.

### Required evidence

The evidence package must include:

- response `Content-Type` and the raw SSE record capture in the protected
  export;
- captured body byte count and truncation state;
- `wireOutcome` (`complete`, `cancelled`, `network_error`, `worker_error`, or
  `protocol_cancel`) separately from `transportOutcome` (`completed`,
  `stream_closed`, `network_error`, a sanitized RUN_ERROR code, or
  `cancelled`);
- `framesSeen`, `printableFramesSeen`, `printableTurnEvents`, `wireBytes`,
  `terminalReceived`, `firstFrameMs`, and `lastFrameMs`;
- whether `aevatar.chat.context`, `TEXT_MESSAGE_*`,
  `RUN_FINISHED.result.output`, `RUN_ERROR`, or no terminal arrived;
- the sanitized client `turn.completed` status and error code; and
- the first fence-current History response for the same turn, including
  whether and when its assistant row appeared.

The numeric transport telemetry is metadata only. It never contains frame
bodies, prompt text, assistant output, or credentials. Raw SSE/body capture is
already a separate, explicitly enabled, session-only diagnostic surface and
must be handled as sensitive data.

### Classification

- Printable, contract-valid wire content that produced no corresponding turn
  event is a NyxID client/parser regression.
- Clean EOF with no AG-UI terminal is `wireOutcome=complete` plus
  `transportOutcome=stream_closed`; it does not mean the server-side run
  failed. If History later contains the exact turn, delivery was interrupted
  while Aevatar still completed and materialized the run.
- A dying response body is `network_error` at both layers. A later exact-turn
  History row still proves server-side completion despite browser delivery
  failure.
- A valid content-free `RUN_FINISHED` is an authoritative empty terminal. A
  `RUN_ERROR` uses its sanitized error code. No assistant row before the
  projection deadline remains a bounded, inconclusive materialization
  failure rather than proof that no row will ever appear.

The incident that motivated this runbook now has one decisive observation:
refreshing reveals the assistant message. The run therefore completed and
materialized upstream; the unresolved issue is why browser stream delivery
stopped. That cause belongs with the Aevatar team. It is unverified and needs a
live stack plus the capture above; Aevatar services are currently unavailable,
so local stubbed SSE/History tests are the only pre-ship verification.

`transcriptSettling` remains a separate follow-up. Its projection counter
currently treats fulfilled, rejected, aborted, and deadline-elapsed reads as
equivalent, and it does not observe the background reconciler. Do not wire the
reconciler into that flag without making the accounting outcome-aware.

## Rust service tests

`backend/src/services/assistant_service.rs` tests pure path, family, index, body, and validation logic. Its coverage includes:

- platform `aevatar` resolution guards;
- conversation path-segment safety and prefix family selection;
- shared-index filtering, dedupe, cursor handling, and newest-first sorting;
- typed command discrimination, unknown-field rejection, control identities, secret rejection, bounds, exact reconstruction, response kind, and action resources;
- create-only `commandId`, continuation-only positive `minimumStateVersion`, prompt trimming, exact Studio body order, and optional `sessionId`; and
- all canonical upstream path builders.

The test `migration_guard_keeps_scoped_typed_commands_and_per_conversation_commands_out` prevents removed routing shapes from returning. Typed commands stay on `POST /api/chat`; command routes are not reintroduced beneath scoped or per-conversation paths. This guard protects the upstream dispatch contract from an accidental return to older endpoint families.

## Rust handler/upstream-stub tests

The integration-style assistant tests live with proxy handler tests in `backend/src/handlers/proxy.rs` because the assistant handlers reuse the administrative proxy data plane.

`assistant_chat_handlers_rebuild_bodies_for_the_admin_service` provisions a controlled platform service and observes upstream requests. It verifies:

- requests resolve the admin-managed row;
- typed and workflow bodies are rebuilt rather than passed through;
- `Accept`, content type, and idempotency headers match the command;
- the upstream path is canonical;
- caller-supplied fields do not survive reconstruction;
- identity mode metadata is correct;
- `X-NyxID-Identity-Token` is injected;
- `X-NyxID-Delegation-Token` is injected; and
- caller Authorization is not leaked on the steady-state configuration.

The test-only upstream echo facility in `backend/src/handlers/assistant.rs` captures method, path, safe body, selected headers, and identity-mode metadata. It redacts credential material and is enabled only through the controlled debug/test path.

`assistant_list_drains_mixed_history_pages_and_captures_every_upstream_call` verifies cursor draining across typed, workflow, and unrelated history rows, including every upstream page request.

`assistant_deleted_scoped_command_routes_are_unroutable` verifies obsolete scoped typed-command routes are not mounted.

Additional proxy tests cover identity assertion generation, delegation-token injection, service-row modes, Authorization precedence, direct/node consistency, and debug capture redaction. Route tests in `backend/src/routes.rs` cover the human-only mount and public manifest placement.

## Actions manifest tests

`backend/src/handlers/assistant_actions.rs` parses the serialized static body and verifies:

- schema version 4 and exact revision;
- descriptor uniqueness;
- the golden `service.connect` parameter schema;
- supported action vocabulary;
- v1 risk/tier/remember fields;
- manifest size below Aevatar's 1 MiB cap; and
- absence of forbidden secret-shaped property names.

The pinned Aevatar repository has corresponding registry-loader and composition tests for version/revision validation, size, required action presence, typed parameter parsing, disabled composition, and fail-closed unsupported requests.

## Commands

The relevant repository gates are:

```bash
cargo test --workspace assistant

cd frontend
npm run build
npx vitest run
npm run lint

cargo test -p nyxid-cli --test wizard_bundle_freshness
```

Playwright is available separately through the frontend's `test:e2e` script. The deterministic E2E harness does not require a backend, but the full Vitest run and Rust assistant-filtered tests remain required because most wire and identity invariants are below the browser flow layer.

## Known gaps

- Live production request/response capture is outstanding. The available capture tokens are rotated or invalid, so no current capture proves the deployed end-to-end request, identity headers, stream context, and terminal sequence.
- The deployed Aevatar composition is unproven. Source and checked-in Mainnet configuration enable the action registry and define the identity assertion, but a live host has not demonstrated that the expected composition and service registration are active.
- The workflow `scopeId` guard is enforce-when-known. When the frontend auth store has not hydrated, context adoption validates conversation, state, and turn but cannot require an exact local user match. Requiring scope unconditionally waits on the live capture to prove deployed frame ordering and field presence.
- `nyxid.action.request` and `service.connect` are emitted only by the typed NyxIdChat producer. Normal new browser conversations use the Studio workflow engine, so action/connect cards are not produced on that path.
- Workflow approval decisions are unsupported in this assistant mount. The client refuses to send the typed actor approval route for a `chatc-...` conversation.
- Workflow runs have no server-side stop route in this mount. Stop aborts the browser stream; first-turn cancellation also performs background create recovery so an upstream-created conversation is not lost.
- Aevatar Chat History is text-oriented. Structured action, approval, run, connect, and artifact blocks are retained in the browser mirror and merged around materialized text, but they are not yet a complete durable server transcript.
- Pending action cards do not survive a full page reload. Durable history is text-only, the history merge can preserve only cards still held in memory, and no state-based card rehydration exists. A reload before the actor re-emits or verifies the action loses the pending card.
- Create recovery does not inspect the recovered `status`. A terminal-failure status such as `failed`, `abandoned`, or `append_rejected` is adoptable whenever the identity fields validate and transcript reconciliation passes. See [Wire contract](02-wire-contract.md).
- The frontend accepts the legacy flat transcript array and the wrapped `{messages, stateVersion}` form. This compatibility form remains until every supported Aevatar environment is confirmed on the wrapped contract.
- The steady-state identity-token service-row cutover is code- and configuration-verified but not live-verified. In particular, no current production capture proves `forward_access_token=false` with both identity assertion and delegated capability present at Aevatar.
