# New-chat follow-up diagnosis — UI state derivation and UX

**VERDICT: DEFECT.** Both messages are wrong for the same underlying reason: neither of them is derived from what actually happened to the turn. The banner is set on *every* terminal of a `chatc-` workflow turn regardless of outcome, so it claims a transcript is on its way even when the turn produced nothing to project. The error is derived from *absence of content* rather than from a declared failure, and its only suppressor (`transcriptSettling`) is blind to the reconciler that is actually waiting on the transcript. The two are not contradictory by accident — they are two independent guesses about the same unobserved fact.

Scope note: *why* the turn printed nothing is a sibling agent's question. Everything below holds whether the stream failed upstream or the client dropped it.

---

## 1. The exact flag sequence

Reported state: brand-new chat, id `chatc-a100…`, user sent "hi", `POST /workflow-chat` 200 after ~4.0–4.6 s, no assistant text, then `GET /api/v1/assistant/conversations/chatc-a100…` **(canceled)**.

| # | Where | What happens |
|---|-------|--------------|
| 1 | `use-assistant.ts:719` | `useSendMessage` constructs the pump. `open = true`, `printed = false`, `projections = 0` (`use-assistant.ts:531-533`), published to the episode cache slot (`use-assistant.ts:547-553, 597`). |
| 2 | `aevatar-transport.ts:2339` | `void this.streamTurn(...)` — fire-and-forget. `sendMessage()` itself is synchronous, so the mutation resolves within a tick. **`sendMessage.isPending` is false for essentially the whole 4.6 s POST**, i.e. it contributes nothing to `transcriptSettling` (`assistant.tsx:559`). |
| 3 | stream runs | No `block.delta` / `block.started` with printable content ⇒ `eventPrintsContent()` never returns true (`use-assistant.ts:491-507`) ⇒ `printed` stays `false`. |
| 4 | `aevatar-transport.ts:5563-5577` | `finishTurn(..., "completed" \| "failed", …)` emits `turn.completed`. |
| 5 | `aevatar-transport.ts:3329-3344` | In `emit()`: on `turn.completed`, for a workflow run on a `chatc-` conversation, sets `stored.projectionPending = true`, `stored.requiredTurnId = run.turnId`, `stored.projectionStalledAt = undefined`. **There is no `event.status` check here.** |
| 6 | `use-assistant.ts:631, 657` | Pump sets `open = false` and calls `scheduleProjection(immediate)`. |
| 7 | `use-assistant.ts:561-576` | `project()` → `projections = 1` → publish (`projecting: true`) → `projectTransportState(...)`. |
| 8 | `aevatar-transport.ts:1704-1715` | Inside that read, `getHistory` short-circuits to the local mirror because `projectionPending && messages.length > 0`, and `historyFromStored` (`:1823-1840`) stamps **`awaitingProjection: true`**. No network hop. |
| 9 | `use-assistant.ts:571-575` | `projectTransportState` settles ⇒ `projections = 0` ⇒ publish `projecting: false`. |
| 10 | `assistant.tsx:520-527` | `history.data.awaitingProjection === true` ⇒ **"Syncing conversation history…"** renders. |
| 11 | `assistant.tsx:452-455, 556-560` | `turnEnded = true` (episode closed, status not `cancelled`), `turnPrinted = false`, `transcriptSettling = false` (step 9 + step 2). |
| 12 | `chat-thread.tsx:532-545` | `tailAnswered = turnPrinted = false`; the `useSettled` condition is all-true; after `EMPTY_TURN_GRACE_MS = 700` (`:387`) ⇒ **`EmptyTurnError`** renders (`:347-358`, `:691` / `:701-708`). |
| 13 | `use-assistant.ts:337-361` | Separately, `awaitingProjection === true` starts `reconcileProjection`, which polls `GET /conversations/{id}` (`aevatar-transport.ts:2076`) under a scope `AbortController` (`:2049`). That is the request seen **canceled** in the network panel. |

Both messages are on screen from ~700 ms after the turn ends, and stay there.

### Verified by running it

Scratch harness (`/tmp`, since deleted; mirrors `frontend/src/pages/assistant.test.tsx`'s mock shape). 4/4 passed:

```
✓ A : awaitingProjection=true + episode{open:false,printed:false,projecting:false} + turnStatus "completed"
      → "Syncing conversation history..." AND "Sorry, there seems to be an error with the request for now."
        rendered simultaneously
✓ A2: same with turnStatus "failed"
✓ B : projecting=true suppresses the error for >1.2 s; flipping projecting→false surfaces it within the
      grace window while the banner is still up
✓ C : turnStatus "cancelled" correctly shows neither
```

---

## 2. Does a canceled read un-suppress the error?

**Disproven as stated — and the truth is worse.**

The canceled `GET /conversations/{id}` is the **reconciler's** poll (`aevatar-transport.ts:2049, 2076`), reached via `useConversation`'s effect (`use-assistant.ts:337-361`). That request feeds `projectionPending` / `projectionStalledAt`. It **never touches `transcriptSettling`**, because `transcriptSettling` is only `episodeState.projecting || sendMessage.isPending` (`assistant.tsx:558-560`), and `projecting` counts only in-flight `projectTransportState` calls issued by the pump (`use-assistant.ts:566-575`). Nothing about the reconcile poll — in flight, completed, or aborted — can change it. So the canceled read did not un-suppress anything; the error was already un-suppressed at step 9, before that GET was even issued.

The *related* real defect, which would bite the moment anyone wired the reconciler into `transcriptSettling`, is that `projections` is decremented **outcome-blind**:

- `projectTransportState` wraps both reads in `Promise.allSettled` (`use-assistant.ts:127-130`), so a rejected/aborted transcript read is indistinguishable from a fulfilled one at the `.finally` that decrements (`:571-575`).
- `waitWithDeadline` (`:91-103`) resolves the whole thing at `PROJECTION_DEADLINE_MS = 5_000` (`:56`) even if the read is still in flight, decrementing `projections` for a read that has not returned.

So "a read is settling" currently means "a read was started less than 5 s ago and has since either succeeded, failed, been aborted, or timed out." That is not a usable suppressor for anything.

`chat-thread.tsx:428-430` documents `transcriptSettling` as "A transcript read is in flight… what keeps a slow-projecting answer from being reported as a turn that printed nothing." **It cannot do that job**, because the read that is actually waiting for the slow projection is the reconciler's, and the reconciler is invisible to it.

---

## 3. Is `projectionPending` set on failed turns?

**Proven: yes — on failed, on empty-but-"completed", and on successful turns alike.**

`aevatar-transport.ts:3329-3344` gates only on `event.event === "turn.completed"`, `nextTurnState !== previousTurnState`, `run.streamDispatched`, `run.protocol === "workflow"`, and the `chatc-` prefix. `finishTurn` (`:5563-5577`) emits `turn.completed` for every terminal including `"failed"` (`:3823`, `:3920`, `:4008`, `:4217`, `:5403`, …).

Scratch transport test (real `AevatarAssistantTransport`, stubbed fetch/SSE, `chatc-` conversation seeded as materialized, then one send). 3/3 passed, with the observed values:

```
terminal: {"event":"turn.completed","turn_id":"turn-empty-1","status":"completed","error":null}
          events: turn.status,turn.completed
after.awaitingProjection = true                       ← empty turn

failed terminal: {"event":"turn.completed","status":"failed","error":{"code":"run_error",…}}
failed after.awaitingProjection = true                ← failed turn

good terminal: {"event":"turn.completed","status":"completed","error":null}
               events: turn.status,message.started,block.started,block.delta,block.completed,
                       message.completed,turn.completed
good after.awaitingProjection = true                  ← successful turn
```

Two consequences:

1. **The banner is not an exception state.** It fires after *every* turn in a `chatc-` conversation and stays up until the transcript materializes. On a healthy chat it is a flicker; on a new chat with projection lag (the exact condition #1329 was built for) it is a persistent claim.
2. **On a failed turn the claim can never come true.** `applyMaterializationObservation` (`:3247-3268`) only clears `projectionPending` when `historyIncludesAssistantTurn(entries, stored.requiredTurnId)` — an *assistant* row carrying that exact turn id (`:820-829`). A turn that produced no assistant message upstream will never satisfy it. The reconciler then burns its full `PROJECTION_BACKOFF_POLICY.deadlineMs = 90_000` (`backoff.ts:8-13`), settles `timed_out`, sets `projectionStalledAt` (`:2239-2241`), and the UI swaps to **"History is taking longer than expected. / Retry"** (`assistant.tsx:504-519`) — a Retry that re-arms `projectionPending` (`:1947-1950`) and repeats the same 90 s wait, forever. The user is offered a retry for something that structurally cannot succeed.

---

## 4. Correct state machine and precedence

### The category error

`awaitingProjection` and "the turn failed" are answers to different questions:

- **Turn outcome** — a fact about *this* turn: did it print, did it fail, did the reader stop it. Known live and exactly, from the episode (`printed`, `open`) and `turn.completed.status`/`.error`.
- **Transcript state** — a fact about the *server-side read model*: has a durable row for this conversation materialized. Known only from the reconciler.

They should never both speak. The transcript matters to the user in exactly one situation: **the transcript is the only source of the messages on screen.** During and just after a live turn it is not — the stream already put the content there. So:

> **Precedence rule:** if this session has a live episode for the conversation, the turn's own outcome is the only thing the user is told. Projection state is background reconciliation and renders nothing. Projection state may speak only when there is no live episode and the thread is empty — i.e. the transcript really is the only source.

Mechanically, that means the banner condition becomes roughly `awaitingProjection && episodeState === undefined && messages.length === 0`, instead of today's bare `history.data?.awaitingProjection` (`assistant.tsx:520`).

### Precedence table

| Case | Turn outcome | Transcript | What the user sees | Owner |
|---|---|---|---|---|
| **(a) Turn failed / closed empty, nothing projected** | terminal, `printed === false` | irrelevant — nothing to project | **Only** an inline turn failure in the assistant column, with a retry affordance. **No** syncing banner. | `chat-thread.tsx` renders it; `assistant.tsx` suppresses the banner; `aevatar-transport.ts:3335-3343` must stop setting `projectionPending` for a terminal that produced no assistant turn |
| **(b) Turn succeeded, transcript lags** | terminal, `printed === true` | pending | **Nothing.** The answer is on screen from the stream; the reconciler is housekeeping. | `assistant.tsx` (banner gate); reconciler stays silent |
| **(c) Cold reload during projection lag** | no episode this session (`episodeState === undefined`) | pending, thread empty | A quiet inline "catching up" state **inside the thread**, where the missing messages belong. Escalates to the stalled + Retry treatment on `projectionStalled`. | `assistant.tsx` decides; `chat-thread.tsx` renders it in the transcript area |
| **(d) Conversation genuinely absent/deleted** | n/a | 404 confirmed absent | "This conversation no longer exists." plus a route back to a new chat. Never "syncing" — the transport already distinguishes this (`AssistantConversationNotFoundError`, tombstoning at `:2125-2138`, `:1735-1737`). | `aevatar-transport.ts` classifies; `assistant.tsx` routes |

### Module ownership

- **`lib/assistant/aevatar-transport.ts`** owns provenance (`identityPending`, `projectionPending`, `projectionStalledAt`, `requiredTurnId`) and must be status-aware at `:3335-3343`: no `projectionPending` for a terminal with no assistant turn to wait for. It should also carry the turn's terminal `status` + `error` forward as a first-class fact, not only as a toast (`use-assistant.ts:647-652`).
- **`hooks/use-assistant.ts`** owns turning provenance into query state. `projections` accounting needs to distinguish *settled* from *aborted/deadline-elapsed* (`:127-130`, `:91-103`, `:571-575`) before `transcriptSettling` can be trusted by anyone.
- **`pages/assistant.tsx`** owns precedence between the banners and the thread — it is the only module that sees both `history.data` and `episodeState`, and it is where the case (a)/(b)/(c) gate belongs.
- **`components/assistant/chat-thread.tsx`** should render a turn outcome it is *told*, not infer failure from absence. Its current job — "guess whether this turn is empty" — is the wrong contract, and no amount of grace tuning fixes it.

---

## 5. UX judgment

`DESIGN.md` read first, per project rule. Relevant anchors: *"Color is earned. Semantic colors do the heavy lifting for status"* (l.13); the error-state pattern is `ErrorBanner message onRetry?` (l.361); banners are `rounded-xl border border-{color}/15 bg-{color}/[0.04] px-4 py-3` with a 36×36 icon tile and `text-[12px]` (l.375); status labels are title-case (l.441); density is compact and *"if in doubt, go smaller"* (l.14); motion is minimal-functional (l.276). The 2026-07-31 decision (l.474) is the governing precedent here: *"a loader is furniture in front of an answer"* — a status surface must not out-shout the content it is standing in for.

### What's wrong today, as UX

1. **Two voices for one event.** A page-wide chrome strip and a red in-thread alert both describe the same thing. Nothing tells the reader which to believe, and they disagree.
2. **The banner is a page-level claim about a turn-level fact.** A full-bleed `border-b … px-6 py-2` strip across the top of the chat (`assistant.tsx:521-526`) reads as "the app is in a mode." The thing it describes is one conversation's read model. It is also not any banner shape in `DESIGN.md` — the sanctioned inline notice is a `rounded-xl` card, and page-wide status is the 2px `AmbientStatusLine` (l.409), which explicitly says pages should not draw their own equivalent.
3. **The error copy is filler.** "Sorry, there seems to be an error with the request for now." — apologises (nobody wants the apology), hedges twice ("seems to be", "for now"), names nothing ("the request"), and offers no action. Meanwhile the actual error message *is* in hand: `turn.completed.error.message`, already surfaced as a toast (`use-assistant.ts:647-652`) — a transient popup that outlives nothing, while the permanent surface says nothing useful.
4. **A Retry that cannot succeed.** Case (a) ends at "History is taking longer than expected. / Retry", which re-arms a 90 s wait for a row that will never exist.

### Recommendation

**Case (a) — turn failed or closed empty. One message, in the thread, where the answer would have been.**

Keep the placement (`chat-thread.tsx:691` / `:701-708`: inside the assistant content column, under the identity mark — that is exactly where the reader is looking). Replace the copy and add an action:

```
⚠  The assistant didn’t reply.
   {error.message when known, else: “The reply ended before anything was sent.”}
   [Try again]
```

- Headline: `text-[13px] text-destructive` (unchanged token; DESIGN.md l.42 `#f87171`). Icon `h-3.5 w-3.5` as today.
- Detail line: `text-[12px] text-muted-foreground` — the cause, not a second apology. Body copy is 12px per the scale (l.65).
- `Try again`: `variant="ghost"` `size="sm"`, `text-[12px]`, resends the last user message. This is the `ErrorBanner … onRetry` contract (l.361) expressed inline. Without it, the reader's only recovery is to retype.
- Suppress the syncing banner entirely in this case.
- **No grace period when the failure is declared.** `turn.completed` with `status: "failed"` and an `error` is not an out-of-order-arrival risk — render immediately. Keep the grace only for the inferred case (closed, no error, printed nothing).

**Case (b) — turn succeeded, transcript lagging. Say nothing.** The answer is on screen. A banner here is the app narrating its own plumbing; per l.474, furniture must not ask to be watched. If a later action genuinely needs the transcript, that action reports its own state.

**Case (c) — cold reload while the transcript is still materializing. One quiet inline state, in the thread body.** This is the only honest home for "Syncing conversation history…", because here the thread really is empty *because* the transcript hasn't landed. Render it where the messages belong, not as page chrome — same shape as the empty state (l.350-356): centred column, `text-[12px] text-muted-foreground`, with the existing three-dot loader (`StreamingDots`) rather than a spinner, so the assistant surface has one loading vocabulary. Copy: **"Catching up on this conversation…"** — describes the app's behaviour without asserting an outcome; "Syncing conversation history" is a system noun the reader has no model for.

Escalation on `projectionStalled` keeps a Retry, but as the documented banner shape (l.375) inside the thread column rather than a chrome strip: `rounded-xl border border-warning/15 bg-warning/[0.04] px-4 py-3`, 36×36 warning tile, `text-[12px]`. Copy: **"This conversation is taking a while to load."** + `Retry`. Amber, not neutral-muted: it is an attention item, and `DESIGN.md` reserves amber for exactly that (l.41, l.376).

**Case (d) — absent.** "This conversation no longer exists." + `Start a new chat`, using the empty-state shape with an error-flavoured headline (l.362).

**Timing.** Don't show anything for the first ~400 ms of case (c) — a materialization that lands fast should never have flashed a notice. Above that, show it and leave it (no re-entry animation on refetch). Motion budget per l.278: short/200 ms fade, `ease-out`.

---

## 6. Grace and settling windows

`EMPTY_TURN_GRACE_MS = 700` (`chat-thread.tsx:387`) was sized against a specific hazard: `turn.completed` and the transcript projection landing in either order within one macrotask (`:181-188`, `:524-528`), and the 500 ms thinking-row exit fade (`:448`). For **that** hazard it is correctly sized and should not change.

It is the wrong tool for projection lag, and lengthening it would be a mistake:

- The reconciler's timescale is 250 ms floor → 30 s cap → **90 s deadline** (`backoff.ts:8-13`). Covering it would mean holding a genuine failure silent for up to 90 s, during which the thread shows a dead gutter with nothing spinning — precisely the "chat looks dead" state the empty-turn error was added to eliminate (`chat-thread.tsx:342-345`).
- It would also hide the failure behind a *lie*, since in case (a) the banner claims a sync that will never complete.
- And it would not even work: `transcriptSettling` cannot observe the reconciler at all (§2), so a longer grace would just delay the same wrong verdict.

The fix is a correct signal, not a longer timer. Concretely:

1. Stop setting `projectionPending` on terminals with no assistant turn (`aevatar-transport.ts:3335-3343`) — removes the false banner at the source.
2. Gate the banner on `episodeState === undefined && messages.length === 0` (`assistant.tsx:520`) — removes it for cases (a) and (b) even if 1 regresses.
3. Render the failure from the declared terminal (`status`/`error`), with grace only for the inferred-empty case.
4. If `transcriptSettling` is kept at all, make it outcome-aware: don't decrement `projections` for an aborted read, and don't let `waitWithDeadline`'s 5 s timeout (`use-assistant.ts:91-103, 56`) pass as a settle.

---

## 7. Not determined

- **Why the turn printed nothing.** Out of scope by instruction (sibling agent).
- **Which abort produced the canceled `GET`.** Candidates, all reachable: `releaseProjectionWaiter` on the `useConversation` effect cleanup (`use-assistant.ts:357-360` → `aevatar-transport.ts:1996-2000`), `settleReconcileEntry`'s `entry.controller?.abort()` (`:2237`), `resetScope` aborting every scope controller on an auth-scope change (`:1307-1320`), or the browser cancelling on navigation. It does not change the verdict — §2 shows the abort is causally irrelevant to the error appearing. Distinguishing them needs a live capture with the wire-log panel on.
- **Real-world duration of the `projecting` window in prod.** It is bounded above by `PROJECTION_DEADLINE_MS = 5_000` and below by the `listConversations` round trip inside the same `allSettled`; I measured it only in the scratch harness (sub-millisecond, no network), not against production latency.
- **Whether `requiredTurnId` is ever null on a failed prod turn.** If the run never announced a turn id, `historyIncludesAssistantTurn(entries, null)` returns `true` (`:824`) and materialization *can* clear on the state-version fence alone. The reported turn did reach `POST /workflow-chat` 200, which normally carries the chat-context frame with a `turnId`, so the never-clears path is the likely one — but I did not confirm it against the production wire log.
