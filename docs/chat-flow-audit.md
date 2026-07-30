# Assistant Chat Flow Audit

**Date:** 2026-07-30 · **Branch:** `halo-sprite-loader` (PR #1276) · **Role:** audit/PM — no production fixes were made in this round.

Every defect previously found in this work came from static review. This audit adds the missing empirical layer: a Playwright e2e harness that drives the real UI against the deterministic mock transport, plus hook-level vitest probes for the mechanisms e2e cannot reach. Each hypothesis from the two Codex reviews is confirmed or refuted below **with the test that proves it**, and the ones that could not be tested are listed with the reason.

---

## 1. The harness

### One command

```bash
cd frontend && npm run test:e2e        # boots vite dev on :4611 (strict) and runs headless chromium
```

- **Config:** `frontend/playwright.config.ts`. The webServer boots `npm run dev -- --port 4611 --strictPort`, so a parallel worktree racing for :3000 can never serve a different checkout to these tests.
- **Specs:** `frontend/e2e/*.spec.ts` + shared driver `frontend/e2e/helpers.ts`. All assertions are user-observable: roles, visible text, and the three state markers the thread already exposes (`data-assistant-halo`, `data-streaming-dots`, `data-empty-turn-error`).
- **No backend, no auth:** `/assistant?mock` selects `MockAssistantTransport` in dev and the route's beforeLoad seeds a mock user.
- **Hook probes:** `frontend/src/hooks/use-assistant.audit.test.tsx` (runs inside the normal `npx vitest run` suite) drives the real hooks + real mock transport for race mechanisms the UI cannot reliably reproduce.

### Spec conventions

- Plain specs assert the intended behavior of the four flows and **pass** — they are the regression suite the implementer must keep green.
- `test.fail(...)`-annotated specs (in `e2e/defects.spec.ts`) assert the **desired** behavior for an open defect and are expected to fail. Fixing the defect makes Playwright report "passed unexpectedly"; remove the annotation and the spec becomes the regression test.
- "current behavior" specs and the vitest `AUDIT` probes are **characterization** tests: they pass by pinning down today's defective behavior so every defect is reproducible on demand. Flip their marked assertions when fixing.

### Fault injection

The deterministic mock streams a full turn in ~1.2 s, which cannot reach latency- and failure-dependent states — and the loading states under audit *are* latency states. `window.__assistantMockFaults` (set via Playwright `addInitScript`) adds three knobs to the mock transport only:

| Knob | Real condition it reproduces |
|---|---|
| `historyDelayMs` | slow transcript reads (`chat-history` round-trip) |
| `historyErrorStatus` | transcript 404 (no history row yet) / 5xx |
| `sendSilent` | a stream POST hung before response headers — user message landed, no event will ever arrive |

### Changes to `src/` (complete list, per the audit rules)

1. `frontend/src/lib/assistant/transport.ts` — the `AssistantMockFaults` interface + the three fault checks inside `MockAssistantTransport.getHistory`/`sendMessage`. Mock-only code paths; inert in production (the mock transport is only selected in dev/test sessions).
2. `frontend/src/main.tsx` — dev-only `window.__nyxQueryClient` handle so defect specs can assert on cache slots nothing renders yet (the episode leak, NYX-5). Stripped from production builds.
3. `frontend/src/hooks/use-assistant.audit.test.tsx` — new test file (probes only, no production code).

No production logic was changed. No `data-testid`s were needed — the thread already exposes semantic markers.

---

## 2. Defects

Severity: **P1** = a user sees a wrong/dead chat in a realistic path; **P2** = wrong state that needs a second condition to become visible; **P3** = polish/latent.

---

### NYX-1 (P1) — A turn that never starts has no deadline: dots forever, no Stop, no error

- **Repro:** `e2e/defects.spec.ts` › "NYX-1 … current behavior: thinking dots forever over an idle-looking composer" (`sendSilent` fault = live transport hanging before response headers).
- **Expected:** within a bounded time the reader gets either a Stop button (turn acknowledged live) or an error (turn acknowledged dead).
- **Actual:** the sent message sits there with bouncing dots forever. No watchdog fires (the transport arms it only once a response body exists, `aevatar-transport.ts:1683`; the send fetch itself has no header deadline, `aevatar-transport.ts:1601`). `turn.status` never arrives, so `active` stays false: the composer is **enabled** and shows Send, not Stop — the chat looks idle while claiming to think.
- **Verdict on P1-b: CONFIRMED** (UI layers empirically via the silent-turn analog; the transport-internal claim — watchdog arms only post-body, no header deadline on send or approve fetches — confirmed by code reading at the cited lines).

### NYX-2 (P1) — Episode slot has no owner: a rejected retry erases the evidence that anything is running

- **Repro (UI):** `e2e/defects.spec.ts` › "NYX-1 … retrying into the hang erases even the thinking dots". Because NYX-1's composer looks idle, the reader naturally re-sends; the retry is rejected by the active-turn guard and its cleanup **nulls the live episode** (`use-assistant.ts` `useSendMessage` catch). The dots vanish too: sent message, no activity, no error, no Stop, rejected text restored into the composer. The chat now looks fully dead.
- **Repro (mechanism):** `use-assistant.audit.test.tsx` › "AUDIT: episode-slot ownership". Also observed live during probe development: with events flowing, the null is re-overwritten within ~100 ms by the winner pump's next event **or its async projection finalizer** — proof that any pump, superseded or not, writes `assistantKeys.episode(conversationId)` unconditionally.
- **Verdict on P1-a: CONFIRMED** (both halves: the unconditional null on a rejected send, and unguarded writes from superseded pumps/finalizers). The approval-flow variant — `pauseForApproval` settling the old run *after* the continuation's pump opened, so the old pump's `turn.completed` overwrites the new episode's `open:true` (`use-assistant.ts:539` evaluation order + `aevatar-transport.ts:1214`) — is confirmed by code reading; it is live-transport-only (see §4).

### NYX-3 (P1) — Approval continuations show no loading state at all

- **Repro:** `use-assistant.audit.test.tsx` › "an approval continuation's pre-status gap shows NO loading indicator at all". The probed page inputs for the gap between clicking Approve and the continuation's first `turn.status` are: `active` false (turn cache still holds the prior terminal), tail = the assistant-owned approval card. `thinking` requires a non-assistant tail and `streaming` requires an active turn (`pages/assistant.tsx:397-400`), so ChatThread renders **neither dots nor halo** — verified in DOM.
- **Expected:** deciding an approval is a send-equivalent; the thinking state must hold the floor until the continuation speaks.
- **Actual:** the card flips and then nothing moves until (on the live contract) the continuation's first status frame — for the whole approve POST round-trip the chat looks finished.
- **Verdict on P1-d: CONFIRMED** (hook-level cache states probed empirically; ChatThread DOM rendered for exactly those states; the page expressions mapping one to the other read directly at the cited lines).

### NYX-4 (P1) — Successful approval reads as "printed nothing" → false red error

- **Repro:** `use-assistant.audit.test.tsx` › "AUDIT: approval decision events do not count as printed" + "turnEnded + printed:false over an approved card shows the false error". The probe scripts exactly what the live transport emits on the JSON-ack approve path (`aevatar-transport.ts:1285-1349`): `block.updated` (decision patch), `block.updated` (parked ledger), `turn.completed(completed)`. `eventPrintsContent` (`use-assistant.ts:348`) has no case for `block.updated`, so the episode closes `{open:false, printed:false}`; rendering ChatThread with the resulting `turnEnded=true, turnPrinted=false` shows **"Sorry, there seems to be an error with the request for now."** over a successfully approved card after the 700 ms grace.
- **Verdict on P1-c: CONFIRMED** (empirically at both the pump layer and the DOM layer; the event vocabulary was taken verbatim from the live transport code).

### NYX-5 (P2) — Deciding an approval opens an episode nothing ever closes

- **Repro:** `e2e/defects.spec.ts` › "NYX-5 … current behavior" (state read via the dev query-client handle) and `use-assistant.audit.test.tsx` › "AUDIT: approval leaves the episode open forever". The decision's pump is constructed (episode `{open:true}`) before the transport answers; the mock `decideApproval` settles the card and returns `null` without emitting any event (`transport.ts` mock class), and error paths after pump construction don't clean up either.
- **Visible impact today:** none with the current seeded fixtures — the tail is an assistant message, which suppresses the thinking row; the next send replaces the pump. The e2e spec proves the chat still works after approving. But the leaked `open:true` permanently suppresses `turnEnded`, so a later real empty-turn failure in that conversation can no longer be reported, and any future UI reading `episode.open` inherits a lie.
- **Verdict on P2-f: CONFIRMED** (empirically, state-level). Note the prompt's warning that this would bite the harness directly did **not** materialize — the flow specs pass because the leak is invisible under the current fixtures.

### NYX-6 (P2) — Re-sending text used earlier suppresses the new message's echo

- **Repro:** `e2e/defects.spec.ts` › "NYX-6". Control (passes): with a 1.5 s transcript delay, **novel** text echoes instantly. Defect (`test.fail`): re-sending the exact text of an earlier user message shows **nothing** until the transcript round-trip lands — the whole-transcript dedup (`pages/assistant.tsx:318`) mistakes the old message for the new one's projection. The textarea has already cleared, so the send looks swallowed.
- **Verdict on P2-h: CONFIRMED** (empirically, user-visible under injected latency; on the live transport the window is every send's real round-trip).

### NYX-7 (P2) — Transcript-hang states have no timeout anywhere above the transport

- What the harness showed: with `historyDelayMs` injected, every "settling" state recovers once the read lands — good. But nothing bounds it: `projectTransportState` awaits `getHistory`/`listConversations`/workspace invalidation with no deadline, and `projecting` only decrements in the projection's `finally` (`use-assistant.ts:420-425`). A read that never settles keeps `transcriptSettling` true forever, suppressing the empty-turn error path — and the streamed content itself never reaches the screen (content renders only via the projected history cache).
- **Verdict on P2-g: CONFIRMED for the mechanism, by code reading + the delay-injection behavior**; the stuck-forever endpoint itself was not held open in a spec (a permanently hanging read adds a multi-minute test for no additional information).

### NYX-8 (P3) — 404/no-transcript notice is reachable and correct (no defect found)

- `e2e/history.spec.ts` proves: 404 → "This conversation has no saved transcript yet. You can keep chatting…" over a usable composer; 5xx → "Could not load earlier messages…"; slow read → "Loading conversation…" then content. One papercut worth noting: with the default query retry policy (3 retries, exponential backoff) the notice takes **~7 s** to appear after a 404 — a 404 is a "not really a fault" state per the code's own comment and probably shouldn't be retried at all.

---

## 3. Hypothesis scorecard

| Hypothesis | Verdict | Evidence |
|---|---|---|
| P1-a episode slot unguarded / rejected send nulls live episode | **CONFIRMED** | vitest probe (NYX-2) + e2e "retrying into the hang"; approval-variant overwrite by code reading |
| P1-b pre-header hang: no deadline, no Stop | **CONFIRMED** | e2e silent-turn specs (NYX-1) for the UI layers; transport internals by code reading (`armWatchdog` only in `consumeTurnStream`; send/approve fetches have no header timeout) |
| P1-c `block.updated` not "printed" → false error on approval | **CONFIRMED** | vitest probes (NYX-4), pump layer + DOM layer, events scripted verbatim from the live approve path |
| P1-d no dots on approval continuations | **CONFIRMED** | vitest probe (NYX-3): probed cache states + ChatThread DOM |
| P2-e raw-id episode key vs `workflow-pending-*` re-key | **PLAUSIBLE / NOT TESTED** (see §4) | code reading only: `aevatar-transport.ts:2246-2250` re-keys mid-stream; pump keeps publishing under the placeholder; page follows the server id |
| P2-f approval leaves episode open (mock) | **CONFIRMED** | e2e state probe + vitest probe (NYX-5); currently invisible with seeded fixtures |
| P2-g projecting can stick forever | **CONFIRMED (mechanism)** | code reading + delay-injection recovery behavior (NYX-7); stuck-forever endpoint not held open in a spec |
| P2-h whole-transcript echo dedup | **CONFIRMED** | e2e control + expected-fail pair (NYX-6) |

## 4. What this harness could NOT test, and why

- **P2-e (workflow re-key mid-stream):** requires the live `AevatarAssistantTransport` workflow protocol (`workflow-pending-*` → `chatc-…` aliasing on the first context frame). The mock transport has no workflow surface and no re-keying. Testing it empirically needs either a fake-fetch vitest harness around `AevatarAssistantTransport` (the existing `aevatar-transport.test.ts` fixtures are the right starting point) or a live Aevatar stack. Code reading says the defect is real: the pump and the page disagree about the conversation's address mid-stream, so the page reads an empty episode slot and loses halo/Stop/dots for the rest of the first workflow turn.
- **P1-b's exact transport internals** (watchdog arming point, retry loop) and **P1-a's approval-variant overwrite** (`pauseForApproval` settling the old run after the new pump opened): the e2e layer proves the user-visible consequences via analogs; the internal sequencing is asserted by code reading, not by a test that drives the real SSE path. Same remedy: a fake-fetch transport harness.
- **"Switch while a send is in flight" against real create latency:** covered with `historyDelayMs` injection (the pending window is stretched artificially). The real first-send-create race (`createConversationOnce` + navigate-on-return) has the same shape but was not driven against a real network.
- **Screen-reader semantics** are asserted only at the role/name level (`role=status`, `role=alert`), not with an actual AT.
- **The halo's visual correctness** (sprite animation, fade timing) — the specs assert presence/absence of `data-assistant-halo`, not pixels.

## 5. Prioritized backlog (work top to bottom)

Ordered by user impact — what makes the chat *look dead or lie* in realistic paths first. NYX-1/2/3/4 compound in the single most important real-world flow on the live contract (send → wait → approve → wait).

1. **NYX-1 — Add a start deadline for every stream episode** (send AND approve). Arm a client-side header/first-event watchdog when the pump is constructed, not when a response body exists; on expiry settle the episode as failed (red error + toast) and free the composer. This single fix turns "chat died silently" into "chat told me and let me retry" for every hang class. *(Spec to flip: `defects.spec.ts` NYX-1 desired.)*
2. **NYX-2 — Give the episode slot an owner.** Tag each pump with a generation/id for its conversation; only the current owner may write or null `assistantKeys.episode(...)` (including from projection finalizers and the `useSendMessage` catch). Fixes the rejected-retry wipe and the approval-flow overwrite in one move. *(Probe to flip: AUDIT NYX-2; e2e "retrying into the hang" then shows dots surviving the rejected retry.)*
3. **NYX-4 — Count approval-decision patches as printed** (or as their own "settled" signal): `eventPrintsContent` must handle `block.updated` with a decision/ledger patch, so a successful JSON-ack approval never renders the red error. *(Probes to flip: AUDIT NYX-4 pair.)*
4. **NYX-3 — Show the thinking state for approval continuations.** The thread needs to treat "episode open + nothing printed this episode" as thinking even when the tail is an assistant-owned approval card (e.g. drive `thinking` from the episode rather than tail role). *(Probe to flip: AUDIT NYX-3.)*
5. **NYX-6 — Replace whole-transcript echo dedup with identity-based reconciliation** (client message id / count-aware tail check), so re-sent text echoes instantly. *(Spec to flip: `defects.spec.ts` NYX-6 desired.)*
6. **NYX-5 — Close or disown the episode on every decideApproval exit path** (null-handle return, throw after pump construction). Cheap once NYX-2's ownership exists. *(Specs to flip: `defects.spec.ts` NYX-5 desired + AUDIT NYX-5.)*
7. **NYX-7 — Bound the projection wait** (timeout on `projectTransportState`, or decrement `projecting` on a deadline) so a hung transcript read cannot suppress error reporting forever. Pairs with NYX-1.
8. **P2-e — Re-key the episode/turn caches when the transport re-keys the conversation** (or publish under the canonical id from the start). Needs the fake-fetch transport harness first (see §4) — build that harness as part of this item; it also converts the §4 code-reading verdicts into regression tests.
9. **NYX-8 papercut — Don't retry transcript 404s** (retry: false for 404 on the history query) so the "no transcript yet" notice appears in <1 s instead of ~7 s.

**Definition of done for the implementer:** all `test.fail` annotations in `e2e/defects.spec.ts` removed with their specs passing, the AUDIT characterization tests flipped to desired behavior (or replaced by regression tests), and the four flow spec files still green: `npm run test:e2e` + `npx vitest run`.
