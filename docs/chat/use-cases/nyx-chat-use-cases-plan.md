# Plan v2 — `nyx-chat-use-cases.md` (4 worked chat transcripts + HTML mockup)

- **Status:** Planning artifact, revision 2 (post stage-2 adversarial review; all P1/P2
  findings resolved in the body below — resolution notes appended under each finding at the
  bottom). Pipeline: plan → adversarial check → implement → review/PR. The implementer writes
  `docs/nyx-chat-use-cases.md` **and** a single-file `index.html` mockup (PLAN-7) from this
  plan; reviewers hold the deliverables to the PLAN-5 acceptance checklist.
- **Contract:** `docs/nyx-chat-aevatar-support-spec.md` (Draft v3, self-contained). All `§`
  references below are to that spec unless prefixed `PLAN-`.
- **Deliverable:** (a) a companion doc containing **4 fully-worked chat transcripts** — user ↔
  assistant messages with cards rendered inline as labeled blocks — showing exactly how a user
  interacts with NyxID chat; (b) a self-contained chat-UI mockup of the same 4 transcripts,
  deployed to https://nyx-chat-wf.surge.sh (PLAN-7).

---

## PLAN-1 · Ground rules for the deliverable

1. **Target-state framing, stated up front.** The transcripts depict the §4 target-state
   contract flows (spec's own words: "what the finished product does"). The deliverable MUST
   open with a short framing note: gaps G1–G9 are assumed closed *except* where the contract
   itself reserves capability or leaves a bridge unspecified — the web executor (§8.2) stays
   reserved, and the lane-A approval bridge (G8) is rendered as observable behavior only
   (PLAN-2.2), never as an invented command. Without this note, reviewers will correctly
   object that today's chat executes one action and mounts zero tools (§0.3).
2. **Wire fidelity over invention.** Every card, block, and annotation must map to a real
   contract object: the 8 command discriminators, the 10 committed frame names, the closed
   status sets, the v4 identity model (§2.1), the registry verbs (§2.3/§7.1). Field scoping:
   PLAN-card/task annotations use only §7.3 `TaskPlan`/`TaskStep` fields; every other block
   (input, approval, action, control, continuation, terminal) uses only the fields of its
   corresponding §2.1 object or command. Nothing may be invented — no frame names, no
   statuses, no verbs outside the 14-verb allowlist + `service.connect`, no compound statuses.
3. **Product realism.** Messages read like a real product, not an annotated protocol dump.
   Wire mechanics live in small italic annotations (PLAN-2.4), never in the assistant's
   voice. The assistant's prose must obey §1.2 (honesty, no drip-feed, no invented
   capability) and §8.1 (every step names its executor).
4. **Consistent world.** One fictional user ("Calvin", per §0.2's own example artifact), one
   org, consistent service slugs: `api-github` (connected in UC1), `lark-feishu` (catalog-only
   at UC1 start; connected from UC1 onward, reused in UC3/UC4). Contents fictional but
   concrete (real-looking repo names, amounts, dates in the week of 2026-08-03).
5. **Secrets never appear.** No token, key, code, or credential value anywhere — including
   inside connect-card renderings (the card shows the journey; the OAuth happens "inside
   NyxID's browser surface" as a narrator line, §4.2 step 3).
6. **No work before the communicated approach.** No LLM operation, inventory read, or
   downstream call may appear in any transcript before the communicate beat — with exactly
   one sanctioned exception: §4.1 Phase-2 **capability-resolution reads** (readiness,
   connected-service inventory, tools/list). These are Class R, never gated, run after the
   gap answers and before the plan, and MUST be disclosed in the communicate prose ("I
   checked your connections: GitHub is connected; Lark isn't yet"). Gap cards are authored
   from the user's message and standing context only — never from a fresh tool read.

---

## PLAN-2 · Transcript formatting convention (normative for the deliverable)

Defined once at the top of the deliverable, used identically in all four transcripts.

### 2.1 Speakers and narration

```markdown
**User**
> Summarise this week's merged PRs and post to #eng-updates.

**Assistant**
> Here's how I'll approach it — …

*[narrator: the user completes the Lark OAuth journey inside the NyxID popup; the secret
lands only in NyxID.]*
```

- `**User**` / `**Assistant**` headers; message body as blockquote.
- Narrator lines — italic, square-bracketed — carry out-of-chat facts (time passing, page
  reload, OAuth journey, a decision arriving from mobile or recorded on a NyxID surface).
  Anything the chat surface cannot itself display is a narrator line, never assistant prose.

### 2.2 Card blocks

Cards are fenced code blocks whose first line is a card header. Closed card-type set (do not
invent more):

| Card type | Renders | Contract mapping |
|---|---|---|
| `CARD: INPUT` | gap questions / plan-gate affordance | pending input, `nyxid.input.request` → `input.resolve` (§2.1, §7.5). Note: `grantBoundary` is a pending-**approval** presentation field and never appears on inputs. |
| `CARD: PLAN` | the run card — full plan, re-rendered on revision/status milestones | `nyxid.task.snapshot` / `TaskPlan` (§7.3) |
| `CARD: CONNECT` | connect a service mid-flow | `nyxid.action.request` → `action.continue` (§2.1, §4.2) |
| `CARD: NYXID APPROVAL` | lane-A service approval with countdown | Rendered from the actor's pending-approval fact (`nyxid.approval.request` presentation: `grantBoundary: nyxid_step_up`, optional `nyxidRequestId` correlation, §4.3 lane A). **The decision is NyxID's**: the card's Approve/Deny buttons are a NyxID-owned surface embedded in chat (like Telegram/mobile/dashboard); the decision travels NyxID's own approval channel, **never** `approval.resolve`. The actor only *observes* the outcome — rendered as a wake/recheck of the gated step. The G8 bridge is unspecified: transcripts show only the observable outcome and annotate that the resume mechanism is honest wake/recheck, not an invented command. |
| `CARD: ARTIFACT` | the delivered result | **Documentation-only visual convention** — this is the assistant's final delivery message styled as a card for readability. It is NOT a wire frame, command, or contract object; the deliverable's convention section must say so. |

Card header line: `CARD: <TYPE> · <key facts>` — e.g.
`CARD: PLAN · task task-uc1 · revision 2 · gate: confirm — "posting publishes outside NyxID"`.

Plan-card step lines carry: index, status glyph, **executor label — plain-language title**,
source detail, and (when relevant) effect/availableActions. Example (UC1, revision 2):

```
CARD: PLAN · task task-uc1 · revision 2 · gate: confirm — "posting publishes outside NyxID"
 1 ● done      You — answer scoping questions        (input · asked once, revision 1)
 2 ⏸ waiting   You — approve this plan               (input · plan gate)
 3 ○ planned   NyxID — connect Lark                  (action: service.connect)
 4 ○ planned   GitHub — read merged PRs: nyxid-backend   (tool: api-github__list_pull_requests)
 5 ○ planned   GitHub — read merged PRs: nyxid-frontend  (tool: api-github__list_pull_requests)
 6 ○ planned   Assistant — draft the update          (llm)
 7 ○ planned   Lark — post to #eng-updates           (tool: lark-feishu__send_message · NyxID approval gate)
 8 ○ planned   Verify — confirm the post exists      (postcondition: lark-feishu__get_message)
```

**Status glyphs — one glyph per §2.1 status, no sharing:** `○` planned · `⏸` waiting · `◐`
running · `●` done · `✕` failed · `⊘` skipped · `⊗` cancelled · `?` uncertain. Every step
line ALSO carries the literal status word next to the glyph (as in the example above), so no
reading depends on glyph recognition; `skipped`, `cancelled`, and `uncertain` must always be
spelled out wherever they appear. Step status and effect evidence are separate fields and are
always rendered separately: `status: done · externalEffect: confirmed` — never a compound
like "done/confirmed".

**Substeps** (§7.4) indent under their step with `·` bullets, statuses `running|done|failed`
only. Substeps are presentation-only phase markers **within one single operation** (one LLM
round or one tool call). A substep may never represent its own external call: work needing N
external calls is N steps (§7.2).

### 2.3 Identity scheme and task lifecycle

Readable, internally consistent IDs: `task-uc1`, `turn-uc1-1` (origin), `turn-uc1-2`
(continuation — always a NEW id, §2.1), `act-lark-1`, `input-uc1-gaps`, `input-uc1-gate`,
`stop-uc2-1`, `step-read-prs-backend`… Continuations reference `originTurnId` correctly.
Approval identities: the NyxID approval object has its own NyxID-side id (e.g.
`nyxid-appr-uc1-1`, surfaced as `nyxidRequestId` on the actor's pending-approval fact); it is
never an `actionRequestId` and never a pending-input `requestId`.

**Canonical task lifecycle (used in all four transcripts, annotated once per transcript):**

- One goal = one task (`task-ucN`), stable across its turns; the goal `text` creates
  `turn-ucN-1`.
- **planRevision 1 — understanding phase:** the plan's only step is the gap-collection
  pending input (`input-ucN-gaps`, authored by `ask_user`, carrying `turnId/taskId/stepId`
  per §2.1). Nothing else is planned yet; no tool has run.
- On the gap answer (`input.resolve`), the actor performs its disclosed Phase-2
  capability-resolution reads (PLAN-1.6), then re-plans: **planRevision 2 — execution phase**
  (`addedBy: "replan"` on the new steps), emitted together with the communicate message. For
  `gate: confirm` plans, the first *pending* step of revision 2 is the plan-gate input
  (`input-ucN-gate`) — a distinct actor-owned `requestId`, never reusing `input-ucN-gaps`.
- **Interpretation note (stated openly in the deliverable):** §7.5 says the confirm gate is
  the task's first step; with a preceding gap-input step this is satisfied as "first step of
  the execution-phase revision" — the gap step (revision 1, already `done`) precedes it in
  the rendered plan. The deliverable flags this as an interpretation, not silently.
- The gap answer resumes `turn-ucN-1`; execution proceeds on it until a blocking action or a
  control creates a continuation turn. No task or step is silently reused or orphaned;
  a stopped task is never resumed — post-stop work is an explicitly new task (UC2).

### 2.4 Wire annotations

One italic line under the block or beat it explains, prefixed `(wire:`. Abbreviated but
accurate — exact frame/command names and the load-bearing fields only:

```markdown
*(wire: `nyxid.action.request` · action=service.connect · actionRequestId=act-lark-1 ·
originTurnId=turn-uc1-1 → task/turn committed `blocked`; stream ends RUN_FINISHED(blocked))*
```

Rules: ≤2 lines each; only where the mechanics are the point (blocked handoff, continuation,
postcondition, fence, gate derivation, lifecycle note, idempotency, generation renewal);
never inside user/assistant prose. Annotations on non-task blocks use that block's §2.1
object fields (PLAN-1.2).

### 2.5 Length budget

90–180 transcript lines per use case; deliverable total ≤ ~1100 lines including the framing
note and convention section.

---

## PLAN-3 · Per-use-case outlines

Each outline lists beats; per beat: what's on screen, cards shown, and the spec sections it
demonstrably exercises. The implementer may tighten dialogue but may not drop a beat or its
spec coverage.

### UC1 — GitHub: "summarise this week's merged PRs and post to #eng-updates"

**Preconditions (one-line setup note):** `api-github` connected & executable; `lark-feishu`
in catalog, NOT connected. Flagship transcript — the full §4.1 loop with the §4.2 connect
cycle (up front, before execution) and the lane-A approval gate.

| # | Beat | Cards | Exercises |
|---|---|---|---|
| 1 | User states the goal. `turn-uc1-1`, `task-uc1` revision 1. | — | §4.1 phase 1 entry; PLAN-2.3 lifecycle |
| 2 | Assistant asks ALL gaps once, one message, one card — authored from the user's message only, **no tool read**: which repos, which week window (suggests Mon 08-03 → today), plain summary or grouped by area. `allowFreeText: true`; answered as ONE composite free-text message (PLAN-2.2/F11). Wire note: revision-1 plan = this input step only; nothing has executed. | INPUT (`input-uc1-gaps`) | §4.1 phase 1 ask-ONCE; §2.1 pending input; PLAN-1.6 |
| 3 | User answers in one message, closing every gap explicitly: "nyxid-backend and nyxid-frontend, Monday through today, grouped by area." | — | `input.resolve` (single freeText answer, closed union) |
| 4 | **Decide + communicate.** Disclosed Phase-2 reads (Class R, reads-only, never gated): connections + readiness — prose states "I checked your connections: GitHub is connected; Lark isn't yet." Then the approach in prose (each step, its tool, why) + PLAN revision 2 (`addedBy:"replan"`, exactly the PLAN-2.2 example): gate input → **connect Lark FIRST** (missing capability surfaces up front, before any execution) → one read step per repo → draft → post (NyxID-gated) → verify. Gate stated as **derived**: `confirm`, because step 7 publishes outside NyxID (§7.5 formula quoted). Wire notes: lifecycle (rev 1 → rev 2); G9 — one connect card per blocked turn; sequential connects is the honest v1 promise. | PLAN + INPUT (`input-uc1-gate`) | §4.1 phase 2: tiers 1+2, communicate-first, derived gate, disclosed reads; §7.5; §7.6 revision semantics; §7.3 fields |
| 5 | User approves the plan — lane-B decision via `input.resolve` on `input-uc1-gate` (options `[{proceed}]`, free text = objections folded into re-plan). No `grantBoundary` — that field belongs to pending approvals, not inputs. | — | §4.3 lane B (plan confirmation); §7.5 confirm |
| 6 | Step 3: connect Lark → CONNECT card; turn blocks. Narrator: user completes OAuth **inside NyxID**; secret never visible. | CONNECT | §4.2 steps 1–3; §2.1 atomic blocked commit; secret boundary |
| 7 | Continuation: `action.continue` (disposition `completed` + typed `userService` ref) → NEW turn `turn-uc1-2`; assistant runs the **postcondition** (`nyx__list_connected_services` → slug present ∧ executable) and only then sets `status: done · externalEffect: confirmed` (separate fields). Prose: "Lark is connected and verified — proceeding." User never re-asks. | PLAN (re-render) | §4.2 steps 4–6: completed = signal not proof; typed postcondition; resume (§1.2); F18 separation |
| 8 | Steps 4–5: GitHub reads — **one step per repo, one tool call each** (no multi-call substeps). PLAN re-render mid-run (`◐ running`). | PLAN (re-render) | §7.2 granularity (N calls = N steps); §7.8 progress; §8.1 labels; Class P |
| 9 | Step 6: LLM draft; draft shown in chat for transparency. | — | LLM step |
| 10 | Step 7: effect-capable Lark post → **NYXID APPROVAL card** (lane A): NyxID-owned presentation (action/target/reversibility), countdown, `grantBoundary: nyxid_step_up`, `nyxidRequestId=nyxid-appr-uc1-1`, note that Telegram/mobile/dashboard can also decide. Assistant prose: "this is NyxID's approval gate, not mine — I can't decide, loosen, or bypass it." *[narrator: Calvin approves on the card — a NyxID surface; NyxID records the decision.]* The actor **observes** the gate outcome and the gated tool call proceeds. Wire note: the lane-A resume is honest wake/recheck — the G8 bridge is unspecified and the transcript invents no command; `approval.resolve` is NOT used (it is Aevatar's lane-B tool-approval command). | NYXID APPROVAL | §4.3 lane A end-to-end, observe-only + lane-separation prose; §2.1 identity separation; G8 honesty |
| 11 | Step 8: verify — postcondition reads the posted message back; `status: done · externalEffect: confirmed`. | PLAN (final re-render, all `● done`) | §7.7 mandatory verify |
| 12 | ARTIFACT (documentation-only card): posted summary text, channel + message link, PR count, "verified against #eng-updates — the message exists". RUN_FINISHED. | ARTIFACT | §4.1 phase 4: proven, never claimed |

### UC2 — Dinner reservation (the §0.2 example)

**Preconditions:** no catalog or connected service can place a restaurant booking; Aevatar's
ecosystem includes a mounted web-search skill. This transcript owns **gate `auto`**, tiers
**3 and 4**, the **steer** moment, the **stop** sequence, **cannot-check honesty**, and the
**pure-read verify exemption**.

| # | Beat | Cards | Exercises |
|---|---|---|---|
| 1 | User: "Book a dinner reservation for the team on Friday — Greek food, somewhere in the north of Singapore, 6–7 pm." `task-uc2` revision 1. | — | §0.2 verbatim scenario |
| 2 | Gaps once, one card, one message — including the **scope decision** (F16): party size; dietary restrictions (assistant states honestly none are on file — it doesn't invent); budget; AND the honest capability statement: "I can't place restaurant bookings yet — web-driven booking is reserved for a future release (§8.2). Shall I research and prepare a ready-to-book shortlist instead?" One free-text answer closes everything. | INPUT (`input-uc2-gaps`) | §4.1 phase 1; §1.2 honesty; tier-4 honest can't asked up front, not sprung later |
| 3 | User: "Party of 6, one vegetarian, no budget cap — yes, do the shortlist." Scope explicitly agreed; the task's goal is now the research artifact, never a booking. | — | `input.resolve`; F16 scope honesty |
| 4 | Communicate + PLAN revision 2: search candidates (**Aevatar — web search skill**, tier 3 — a concrete mounted ecosystem tool, annotated `source.kind:"tool"` with the skill identity, explicitly NOT the reserved §8.2 `web` executor, which covers browser-driving actions) → shortlist & fit check (Assistant — llm). Gate derived **auto**: every step reads or drafts; nothing books, spends, or publishes — card renders already-running; prose: "using existing capability never costs a fresh permission click" AND "no reservation will be made by this task." | PLAN | §4.1 tiers 3+4; §3 fallback; §8.2 reserved (no `web` step exists); §7.5 auto; F15 |
| 5 | Search step runs; substeps = phases of the **single** search operation (query · parse · filter by area/hours). | PLAN (re-render) | §7.4 presentation-only; §7.8 |
| 6 | **STEER:** user types mid-run: "Actually 7 pm sharp, and we need a private room." Wire note: plain `text` rejected (`ACTIVE_TURN_REQUIRES_STEERING`) → front end sends `task.steer` (`steeringId`, `expectedStateVersion`); fence commits; continuation turn `turn-uc2-2`; **planRevision 3**, new/changed steps `addedBy:"steering"`; the completed search step and its results preserved, never re-run. PLAN shows the diff. | PLAN (revision 3) | §4.4 STEER; §7.6; §7.3 `addedBy`/`planRevision` |
| 7 | **STOP:** while the refine step is `◐ running`, user: "Hold off — we might do lunch instead. Stop." → `task.stop` (`stopRequestId=stop-uc2-1`, `clientRequestId`, `expectedStateVersion`). Wire note: fence commits before any successor decision; the in-flight LLM operation is cancelled (step `⊗ cancelled`); effect disclosure: every step's `externalEffect: not_started`/`not_applied` — nothing changed outside NyxID (pure-read task). Task terminalizes `stopped`. Assistant delivers a **partial-work receipt** clearly labeled "stopped — not a completed result": search results so far. Late evidence note: the stopped plan can never advance (§4.4). | PLAN (stopped) | §4.4 STOP: fence, best-effort cancellation, effect disclosure, stopped ≠ success; F9 |
| 8 | *[narrator: 20 minutes later.]* User: "Dinner's back on — finish the shortlist." Wire note: a stopped task is never resumed — this is an explicitly **new task** `task-uc2b` (new plan, gate `auto` again; prior results carried as conversational context, steps re-planned, completed evidence not re-executed but also not silently grafted). | PLAN (task-uc2b) | §4.4 stop semantics; PLAN-2.3 lifecycle honesty |
| 9 | task-uc2b completes. One candidate's hours could not be verified (site unreachable): reported as **"couldn't check right now"** — explicitly NOT "closed"/"unavailable". | PLAN (all terminal) | §1.2 cannot-check ≠ negative fact |
| 10 | ARTIFACT: **research artifact — "no reservation was made"** stated in the card; shortlist of 3 with recommendation, private-room + vegetarian fit, booking links/phone; the unverifiable candidate flagged. Wire note: **no verify step — pure-read task, §7.7 exemption**. Assistant closes: "Want me to draft the reservation request, or post this shortlist to the team on Lark?" | ARTIFACT | §7.7 exemption stated; §1.2 honesty; §4.1 phase 4; F16 terminal copy |

### UC3 — Finance: file a reimbursement from pasted invoices

**Preconditions:** `lark-feishu` connected (Lark Approval reachable as generated operation
tools). This transcript owns **ambiguous failure → reconciliation → retry**, **dedupe
honesty**, **no-duplicates idempotency**, and the **approval-expiry ALT branch**.

| # | Beat | Cards | Exercises |
|---|---|---|---|
| 1 | User pastes 3 invoice texts: "file a reimbursement for these." (Invoices 2 and 3 are the same invoice pasted twice — same vendor/number/amount.) `task-uc3` revision 1. | — | §0.2 live-Finance-flow scenario |
| 2 | Gaps once, one card: expense category, cost center, currency for the USD invoice. One free-text answer. **No extraction, LLM pass, or table before the communicate beat** — the gap questions come from skimming the user's own pasted text, which is conversation content, not a tool run. | INPUT (`input-uc3-gaps`) | §4.1 phase 1; PLAN-1.6 |
| 3 | User answers in one message. | — | `input.resolve` |
| 4 | **Communicate + PLAN revision 2** (before any LLM/tool work): extract & dedupe line items (Assistant — llm; substeps = per-invoice phases of the one extraction operation) → file Lark Approval instance (Lark — tool, tier 1, NyxID-gated) → verify read-back (postcondition). Gate derived **confirm** (creates an approval instance outside NyxID). | PLAN + INPUT (`input-uc3-gate`) | §4.1 phase 2 communicate-first (F1); §7.5 confirm; §7.4 |
| 5 | User approves the plan (`input.resolve` on `input-uc3-gate`). | — | §4.3 lane B |
| 6 | Extraction step runs → extracted table shown. **Dedupe:** assistant flags invoices 2≡3 visibly — "these are the same invoice; I'm filing 2 items, not 3" — dropped item named, never silent. | PLAN (re-render) + inline table | §1.2 never-silent; data honesty |
| 7 | Filing step → **NYXID APPROVAL card** (lane A: NyxID-owned decision surface, countdown, `nyxid_step_up`, `nyxidRequestId=nyxid-appr-uc3-1`). *[narrator: the user double-clicks Approve on the NyxID card.]* Wire note: one decision commits on NyxID's side; the duplicate click is idempotent — no second decision, no duplicate instance. The actor observes the outcome (wake/recheck; G8 honest, no `approval.resolve`). **ALT branch (clearly marked alternate outcome, does not replace the primary path):** if the countdown expires → expiry = denial (§1.2); the filing step terminalizes with a typed denied receipt, the dependent verify step becomes `⊗ cancelled`, and the assistant explains how to ask again. | NYXID APPROVAL (+ ALT box) | §4.3 lane A observe-only; §1.2 no-duplicates; §1.2 expiry-as-denial (F19) |
| 8 | Gated tool call runs (operation generation 1) and gets a **Lark 502**. Honest state: a gateway error cannot prove the mutation didn't land — operation phase `uncertain`, step `? uncertain` (word spelled out), `externalEffect: may_have_changed`. **Retry is NOT offered yet** — effect-capable tools are never replayed on ambiguous evidence; `availableActions: [stop]`. Assistant reports per §1.2: what completed, what may have changed, what happens next. | PLAN (re-render, `? uncertain`) | §1.2 failure rule; §2.1 uncertain ≠ retry invitation (F7) |
| 9 | **Reconciliation:** assistant runs a named read (`lark-feishu__list_approval_instances`, filtered to this submitter/day) → no matching instance exists → effect truth resolved to `externalEffect: not_applied`; only now does the actor compute `availableActions: [retry, skip, stop]`. Wire note: retry requires rebuildable typed input + proof replay is safe — both now hold. | PLAN (re-render, `✕ failed · not_applied`) | F7: evidence before retry; §7.3 actor-computed availableActions |
| 10 | User clicks Retry → `step.retry` with `expectedOperationGeneration: 1`; the operation re-enters at **generation 2** ("current generation + 1") → succeeds. | PLAN (re-render) | §2.1 generation fencing (F8) |
| 11 | Verify step: read the instance back → exists, status "pending manager approval"; `status: done · externalEffect: confirmed`. | PLAN (all terminal) | §7.7 |
| 12 | ARTIFACT: instance code + link, 2 line items + total, approver, "verified against Lark Approval — the instance exists and is pending". | ARTIFACT | §4.1 phase 4 |

### UC4 — HR: screen a pasted résumé (score gate → Lark Bitable write)

**Preconditions:** `lark-feishu` connected; a "Candidate Tracker" Bitable exists. This
transcript owns **reload/rehydration**, **cross-channel first-decision-wins**, the
**rubric-sourced score**, and the **conditional-write (`skipped`) communication**.

| # | Beat | Cards | Exercises |
|---|---|---|---|
| 1 | User pastes a résumé: "screen this for the Senior Backend Engineer role — if they clear our bar, add them to the candidate tracker." `task-uc4` revision 1. | — | scenario entry |
| 2 | Gaps once, one card — no tool read behind it: **what to score against** ("paste the job description or the criteria — I won't invent a rubric"), the bar (suggest 70/100?), which tracker table (by name), stage tag. One free-text answer. | INPUT (`input-uc4-gaps`) | §4.1 phase 1; §1.2 honesty (no fabricated rubric — F17); PLAN-1.6 |
| 3 | User pastes the JD's 5 criteria, sets bar 75, names "2026 Pipeline", stage "screen". | — | `input.resolve` |
| 4 | Communicate + PLAN revision 2, with disclosed Phase-2 read: "I checked — Lark is connected and I can see the 2026 Pipeline table." Steps: score against the supplied JD criteria (Assistant — llm) → **conditional communicated**: "if the score is below 75 I'll skip the write — the step will show `skipped` (literal status), not silently vanish" → Bitable write (Lark — tool, NyxID-gated) → verify read-back. Gate **confirm** (external write). User approves plan (`input-uc4-gate`). | PLAN + INPUT | §7.5; disclosed reads (PLAN-1.6); honest conditional; §4.3 lane B |
| 5 | Scoring step (one LLM operation; substeps = phases: parse · score vs criteria · summary) → **82/100 against the user-supplied JD criteria**, clears bar 75; 3-line rationale citing the rubric source. | PLAN (re-render) | §7.4; F17 rubric attribution |
| 6 | Write step → **NYXID APPROVAL card**, countdown running (`nyxidRequestId=nyxid-appr-uc4-1`). *[narrator: the user reloads the page.]* The identical card rebuilds — same pending approval, countdown live, nothing duplicated. Wire note: rehydration = the state query returning the same `TaskPlan` shape + pending facts — no frame replay (§7.9, §4.4 RELOAD). | NYXID APPROVAL (re-render post-reload) | §4.4 RELOAD; §7.9; §1.2 no-duplicates |
| 7 | *[narrator: the user approves from the NyxID mobile app instead of the chat card.]* The chat card flips to display-only "Approved on mobile" — first decision wins; every other surface only displays the outcome. The actor observes the gate outcome; the write proceeds. (Same G8-honest wake/recheck note as UC1; no `approval.resolve`.) | NYXID APPROVAL (outcome) | §1.2/§4.3 one-decision-wins, cross-channel; lane-A observe-only |
| 8 | Verify: read the Bitable row back → exists with written fields; `status: done · externalEffect: confirmed`. | PLAN (all `● done`) | §7.7 |
| 9 | ARTIFACT: candidate name, 82/100 with breakdown "scored against the JD you provided", row link in "2026 Pipeline", "verified: the row exists". | ARTIFACT | §4.1 phase 4 |

---

## PLAN-4 · Coverage map (spec-section × use case)

`✔` = demonstrably exercised (visible in transcript + annotated) · `–` = not this transcript.
Every row must have ≥1 `✔`; rows marked **(hard)** are deliverable requirements.

| Spec obligation | UC1 | UC2 | UC3 | UC4 |
|---|---|---|---|---|
| §4.1 P1 understand + gaps asked ONCE, one message, no tool behind the gap card **(hard)** | ✔ | ✔ | ✔ | ✔ |
| §4.1 P2 disclosed capability-resolution reads (reads-only, named in prose) | ✔ | – | – | ✔ |
| §4.1 P2 tier 1 — NyxID connected | ✔ | – | ✔ | ✔ |
| §4.1 P2 tier 2 — NyxID connect card, surfaced up front before execution **(hard)** | ✔ | – | – | – |
| §4.1 P2 tier 3 — concrete Aevatar-ecosystem tool, labeled, non-`web` encoding | – | ✔ | – | – |
| §4.1 P2 tier 4 — honest can't / web-reserved, scope agreed up front **(hard)** | – | ✔ | – | – |
| §4.1 P2 COMMUNICATE approach before any LLM/tool work **(hard)** | ✔ | ✔ | ✔ | ✔ |
| §7.5 gate derived — `confirm` via plan-gate pending input (distinct requestId) | ✔ | – | ✔ | ✔ |
| §7.5 gate derived — `auto`, starts immediately | – | ✔ | – | – |
| PLAN-2.3 task lifecycle (rev 1 gap input → rev 2 execution; §7.5 interpretation noted) | ✔ | ✔ | ✔ | ✔ |
| §4.2 connect card: blocked turn → action.continue → postcondition → resume **(hard)** | ✔ | – | – | – |
| §4.3 lane A: NyxID-surface decision, actor observes only, G8-honest wake/recheck **(hard)** | ✔ | – | ✔ | ✔ |
| §4.3 lane B: gap/plan-gate confirmations via `input.resolve` **(hard)** | ✔ | ✔ | ✔ | ✔ |
| §4.3 lane separation stated, never conflated; `approval.resolve` never used for lane A **(hard)** | ✔ | – | ✔ | ✔ |
| §4.3 cross-channel first-decision-wins | – | – | – | ✔ |
| §4.4 STEER (fence, addedBy:"steering", preserved steps) **(counts as steer moment)** | – | ✔ | – | – |
| §4.4 STOP (fence, cancellation, effect disclosure, stopped ≠ success, new task after) **(hard)** | – | ✔ | – | – |
| §4.4 RELOAD / §7.9 rehydration | – | – | – | ✔ |
| §1.2 failure: ambiguous → `uncertain`/`may_have_changed`, reconciliation before retry **(counts as failure moment)** | – | – | ✔ | – |
| §2.1 generation renewal: retry at current+1 with `expectedOperationGeneration` | – | – | ✔ | – |
| §1.2 approval expiry = denial (ALT branch: terminalized step, cancelled dependents) | – | – | ✔ | – |
| §1.2 no-duplicates (double-click / reload idempotency) | – | – | ✔ | ✔ |
| §1.2 honesty: cannot-check ≠ not-connected/negative | – | ✔ | – | – |
| §1.2 honesty: dedupe/dropped items named; rubric never invented | – | – | ✔ | ✔ |
| §7.7 verify before success claim **(hard)** | ✔ | – | ✔ | ✔ |
| §7.7 pure-read exemption, stated | – | ✔ | – | – |
| §7.2 granularity: N external calls = N steps (no hidden work in substeps) **(hard)** | ✔ | ✔ | ✔ | ✔ |
| §7.4 substeps = phases of one operation only | – | ✔ | ✔ | ✔ |
| §7.3 fields visible: planRevision / addedBy / source kinds / availableActions; status and effect always separate | ✔ | ✔ | ✔ | ✔ |
| §7.6 re-planning (revision diff rendered) | ✔ | ✔ | – | – |
| §8.1 executor labeling on every step **(hard)** | ✔ | ✔ | ✔ | ✔ |
| Artifact result (documentation-only card; "verified against …" where effects occurred) **(hard)** | ✔ | ✔ | ✔ | ✔ |
| §2.1 completed-disposition = signal, postcondition = proof | ✔ | – | – | – |

Set-level requirements satisfied: connect-card mid-flow in ≥1 (UC1); both approval lanes
correctly separated (UC1/UC3/UC4); failure (UC3), steer (UC2), and stop (UC2) all present.

---

## PLAN-5 · Acceptance checklist

The implementer self-checks against this; the adversarial checker and reviewers grade against
it item by item. **Fail any (hard) item ⇒ revise before PR.**

### Global (apply to every transcript)

- [ ] **(hard)** Framing note present (PLAN-1.1): target-state per §4; gaps assumed closed;
      web executor stays reserved; the G8 lane-A bridge rendered as observable wake/recheck
      only.
- [ ] **(hard)** Formatting convention defined once and used identically (PLAN-2): closed
      card-type set; one glyph per status with `skipped`/`cancelled`/`uncertain` always
      spelled out; ID scheme + task lifecycle (PLAN-2.3) consistent.
- [ ] **(hard)** Exactly ONE gap-collection message and ONE gap-input request per transcript
      (zero drip-feed); confirm-gated transcripts additionally render exactly one distinct
      plan-gate input; the two never share a `requestId` and are never collapsed into one
      decision.
- [ ] **(hard)** Gap cards are authored without tool reads; each is answered by ONE user
      message (single freeText or single flat option selection) that visibly closes every
      asked gap.
- [ ] **(hard)** No LLM operation, inventory read, or downstream call before the communicate
      beat, except disclosed Phase-2 capability-resolution reads (reads-only, named in the
      communicate prose). No tool call the user never saw coming.
- [ ] **(hard)** Gate always stated as *derived* with its §7.5 reason — never a choice. A
      known-missing capability surfaces up front: connect steps precede execution steps.
- [ ] **(hard)** Lane A vs lane B never conflated: lane-A cards are NyxID-owned decision
      surfaces (countdown, cross-channel note, `nyxid_step_up`, `nyxidRequestId`), decided on
      NyxID's side, with the actor only observing (wake/recheck; no invented bridge command).
      `approval.resolve` NEVER appears in a lane-A beat. Lane B = gap/plan-gate confirmations
      via `input.resolve`. `grantBoundary` appears only on pending-approval presentation,
      never on inputs. Approval identities, `actionRequestId`s, and input `requestId`s are
      three disjoint identity families.
- [ ] **(hard)** Every transcript with ≥1 effect-capable step ends with a verify step
      (`postcondition` source, named check tool) BEFORE any success claim; artifacts use
      "verified against …" language. UC2 states the pure-read exemption explicitly.
- [ ] **(hard)** Every plan-card step line carries an executor label (§8.1); `source.kind`
      only from the §7.3 closed union; NO step ever uses `kind: "web"`. N external calls are
      N steps; substeps only mark phases within one operation.
- [ ] **(hard)** Step status and effect evidence always rendered as separate fields
      (`status: done · externalEffect: confirmed`) — no compound statuses anywhere.
- [ ] **(hard)** No secrets, tokens, codes, or credential values anywhere; OAuth/key journeys
      happen behind narrator lines inside NyxID surfaces.
- [ ] **(hard)** Honesty: no invented capability, tool, verb, or rubric; unknowns are "can't
      check right now", never a negative claim; failures never silent; scope reductions
      agreed with the user up front, and terminal copy never implies the unsupported original
      goal succeeded.
- [ ] Wire annotations use only real frame/command names; field scoping per PLAN-1.2 (§7.3
      for task payloads, §2.1 objects elsewhere); every continuation is a NEW turn id with
      correct `originTurnId`; `taskId` stable within a goal; `planRevision` monotonic.
- [ ] Tool names plausible per §2.2 (`{slug}__{operation}` or `nyx__*` built-ins); G2-style
      reads flagged as such if used; UC2's ecosystem tool named concretely and annotated as
      non-`web`.
- [ ] Status transitions legal in the closed sets: ambiguous failures pass through
      `uncertain`/`may_have_changed` and reach retryability only via named reconciliation
      evidence (`not_applied`); retries carry `expectedOperationGeneration` and re-enter at
      current generation + 1; `availableActions` framed as actor-computed.
- [ ] A stopped task is never marked successful and never resumed; post-stop delivery is a
      partial-work receipt; further work is an explicitly new task.
- [ ] Length within PLAN-2.5 budget; prose reads as product copy, not protocol narration.

### Per use case

- [ ] **UC1 (hard):** connect step ordered BEFORE all execution steps and after the accepted
      gate; full §4.2 cycle (atomic blocked commit, connect card, narrator OAuth,
      `action.continue` with typed `userService` resource, named postcondition, "signal not
      proof" annotated, resume without re-asking); one read step per repo; lane-A approval
      decided on the NyxID card with observe-only annotation; tiers 1+2 in the decide beat;
      G9 sequential-connect note; lifecycle note (rev 1 → rev 2).
- [ ] **UC2 (hard):** scope decision (research-only, no booking) asked inside the single gap
      message and reflected in plan + terminal copy ("no reservation was made"); concrete
      ecosystem tool, non-`web` annotation; gate `auto` with the no-fresh-permission line;
      steer beat (text rejected → `task.steer`, fence, revision bump, `addedBy:"steering"`,
      preserved step); **stop beat** (`stopRequestId`, fence, `⊗ cancelled` step, effect
      disclosure, `stopped` task, partial receipt ≠ success, explicitly new task afterward);
      cannot-check candidate flagged; pure-read exemption stated.
- [ ] **UC3 (hard):** no extraction before the communicate beat; dedupe visible and named;
      double-click idempotency annotated; ALT expiry branch (denial, terminalized step,
      cancelled dependents, how-to-re-ask) clearly marked alternate; failure beat shows
      `uncertain` + `may_have_changed` with retry withheld, then reconciliation read
      resolving `not_applied`, then retry with `expectedOperationGeneration: 1` at
      generation 2; verify reads the instance; artifact carries instance code + status.
- [ ] **UC4 (hard):** rubric sourced from the user (asked in the one gap message; score and
      artifact cite it); conditional write communicated with the literal `skipped` promise;
      disclosed Phase-2 read named; reload beat rebuilds the identical approval card via the
      state query (§7.9); mobile decision flips the card to display-only outcome
      (first-decision-wins), actor observe-only; verify reads the row; artifact links it.

### Reviewer traps (adversarial checker: probe these specifically)

1. Any question asked in a second message that was knowable up front (drip-feed), or a gap
   card whose options required a tool read.
2. `approval.resolve` appearing anywhere in a lane-A beat, or any invented G8 bridge
   command; a plan gate resolved by anything but `input.resolve`.
3. An `action.continue: completed` treated as completion without a postcondition read.
4. A success claim ("posted!", "filed!", "booked!") before the verify step; an artifact
   without "verified against" grounding; a stopped or scope-reduced run implying the original
   goal succeeded.
5. An invented frame name, status value, compound status ("done/confirmed"), verb, or a
   `source.kind` outside §7.3; `grantBoundary` on an input.
6. The dinner transcript booking anything, emitting a `web` step, or auto-running the
   substitute scope without the user's up-front agreement.
7. A continuation reusing the origin `turnId`; `taskId` changing mid-goal; a stopped task
   resumed; gap and gate inputs sharing a `requestId`.
8. Steering rendered as a queued new turn instead of `task.steer` on the active task.
9. A retry offered while effect evidence is `uncertain`/`may_have_changed` (no
   reconciliation), or a retry without `expectedOperationGeneration` / at the wrong
   generation.
10. Hidden work: a substep that is really its own external call; a step secretly looping.
11. Secrets, key values, or user codes anywhere — including "the card shows your new key".

---

## PLAN-6 · Out of scope for the deliverable

- Class L (CLI handoff) and Class X declines — no natural fit in these 4 scenarios; do not
  force one in.
- A lane-B **tool** approval (`approval.resolve` with `grantBoundary: within_grant`) — none
  of these four scenarios naturally raises an Aevatar-scoped tool approval; lane B is
  demonstrated by the gap and plan-gate inputs. State this explicitly in the deliverable so
  `approval.resolve`'s absence reads as deliberate, not overlooked.
- Declined-connect variant (§4.2) — covered by the spec's own flow text.
- Key-mint / one-time-reveal scenario (§0.2's fifth example) — separate use case, not in
  this set of 4.
- Any claim about *today's* shipped behavior (that is the contract's §5 gap register's job).

---

## PLAN-7 · HTML rendering conventions (`index.html` → https://nyx-chat-wf.surge.sh)

Stage 3 also produces a **single-file** `index.html` — a clean chat-UI mockup rendering all
4 transcripts — deployed to `https://nyx-chat-wf.surge.sh` (surge deploy is a stage-3
mechanical step; the file itself must not depend on the host).

1. **Single file, zero network dependencies.** All CSS inline in `<style>`, any JS inline in
   `<script>` (vanilla only, small); system font stack (`-apple-system, "Segoe UI", Roboto,
   "Helvetica Neue", sans-serif`; monospace stack for wire annotations and tool names). No
   external fonts, icons, images, CDNs, or fetches — the page must render fully offline.
2. **Layout.** One page: compact sticky header (title + one-line target-state framing note
   linking nowhere external), then a 4-tab switcher — one conversation per tab (UC1–UC4,
   short labels: "GitHub → #eng-updates", "Dinner reservation", "Reimbursement", "Résumé
   screen"). Tabs via minimal JS (or CSS-only); each UC also gets an `id` anchor so
   `#uc3` deep-links work. Conversation column max-width ~760 px, centered.
3. **Message rendering.** User bubbles right-aligned (accent background), assistant bubbles
   left-aligned (neutral background); narrator lines centered, italic, muted; wire
   annotations as small muted monospace footnote lines attached beneath the element they
   annotate (visible by default — they are part of the deliverable, not debug output).
4. **Card visual mapping** (1:1 with the PLAN-2.2 card types; each card carries its header
   line text): `INPUT` — bordered card with the question list and, once answered, the user's
   answer echoed as a filled state; `PLAN` — bordered card with header chips (task id,
   revision, gate + reason), one row per step: glyph + literal status word chip + executor
   badge + title + monospace source; substeps indented; `CONNECT` — card with service name,
   NyxID-branded "Connect via NyxID" button state, and completed state; `NYXID APPROVAL` —
   visually distinct from every other card (NyxID-branded border/badge "Decided on NyxID —
   not by the assistant", countdown pill, cross-channel note, and an outcome state incl.
   "Approved on mobile" for UC4); `ARTIFACT` — success-tinted card with the "verified
   against …" footer (UC2's labeled "research artifact — no reservation was made"). ALT
   branches (UC3 expiry) render as a collapsed `<details>` block labeled "Alternate outcome".
5. **Status rendering.** Same glyph set as PLAN-2.2 plus a color-coded literal-word chip per
   status (done green, running blue, waiting amber, failed red, uncertain amber, skipped
   gray-dashed, cancelled gray, planned neutral). Never color-only: the word chip is always
   present (accessibility + the F5 rule).
6. **Mobile-readable.** Single column collapses cleanly ≤ 400 px wide; base font ≥ 14 px;
   long monospace lines wrap or scroll within their card, never the page; tabs remain
   reachable (wrap to two rows if needed).
7. **Content parity.** The HTML renders the markdown transcripts 1:1 — same beats, same card
   contents, same wire annotations, no extra or missing beats. The markdown doc is the SSOT;
   any divergence is a review failure.

---

## Stage-2 adversarial findings (binding on the implementer)

1. **P1 must-fix — Three outlines perform task work before the approach is communicated.**
   Exact plan text at fault: UC1 beat 2 says `options from real GitHub inventory`; UC3 beat 4
   says `Extraction step (LLM, substep per invoice) → extracted table shown`, while its
   communicate beat is beat 5; UC4 beat 2 says `options listed from real Bitable inventory
   read`. These contradict §4.1 phase 1 (`no tool runs yet`) and the plan's own hard rule that
   the approach precedes the first tool call. Required correction: do not query GitHub or
   Bitable to build the initial gap card; either ask for repo/table names in the single
   free-text gap answer or make an already-authoritative snapshot part of the stated incoming
   context. Move UC3 extraction behind a communicate + PLAN beat that names extraction,
   dedupe, filing, and verification, with their executors and reasons. No LLM, inventory, or
   downstream operation may appear before that beat.
   **Resolved by:** PLAN-1.6 (gap cards from user message only; Phase-2 reads disclosed-only);
   UC1/UC4 gap cards now free-text asks; UC3 extraction moved behind the beat-4 communicate +
   PLAN (beats 4–6 reordered).

2. **P1 must-fix — UC1 knowingly postpones a required connect until after execution has
   started.** Exact plan text at fault: beat 4 says `Lark → in catalog, not connected →
   connect card before that step runs`, followed by beat 6 GitHub execution, beat 7 drafting,
   and only beat 8 `Step 4 reached → CONNECT card`. §4 and §4.1 say a missing capability known
   at planning time surfaces up front and the connect card occurs before execution starts.
   Required correction: after the derived plan gate is accepted, make the Lark connect the
   first executable step; block, continue on a new turn, prove the connection, and only then
   run the GitHub reads and draft. This still exercises the full §4.2 blocked-turn cycle.
   **Resolved by:** UC1 plan reordered — connect Lark is step 3 (first executable after the
   gate), reads/draft/post follow (beats 6–9); PLAN-2.2 example card updated to match.

3. **P1 must-fix — The plan conflates lane-A NyxID decisions with Aevatar's
   `approval.resolve`.** Exact plan text at fault: PLAN-2.2 maps `CARD: NYXID APPROVAL` to
   ``nyxid.approval.request` → `approval.resolve` (§4.3 lane A)`; UC1 beat 10 says the user
   approves that lane-A card with ``approval.resolve`, `approved:true``; the global checklist
   repeats that lane A is carried this way. Under §4.3, lane A is a NyxID approval object
   decided on NyxID surfaces and Aevatar only observes it. `approval.resolve` is the
   Aevatar-owned tool-approval command in lane B, and G8 explicitly says the lane-A wake/resume
   bridge is unspecified. Required correction: render the lane-A card and decision as a
   NyxID-owned interaction, use a distinct NyxID approval identity (optionally correlated by
   `nyxidRequestId`), and describe the resulting Aevatar wake/recheck as the assumed G8 bridge
   without naming `approval.resolve`. Reserve `approval.resolve` for an actual lane-B tool
   approval with `grantBoundary: within_grant`, or demonstrate lane B only through the plan
   gate's `input.resolve`. Apply this correction to UC1, UC3, UC4, PLAN-2.2, and PLAN-5.
   **Resolved by:** PLAN-2.2 NYXID APPROVAL row rewritten (NyxID-owned surface, `nyxidRequestId`,
   observe-only wake/recheck, G8-honest); UC1 beat 10, UC3 beat 7, UC4 beat 7 reworked; lane B
   demonstrated via inputs only, `approval.resolve` explicitly out of scope (PLAN-6).

4. **P1 must-fix — `grantBoundary` is assigned to pending input even though that field does
   not exist there.** Exact plan text at fault: UC1 beat 5 says `lane B decision,
   input.resolve, grantBoundary within_grant`, and PLAN-5 says `plan/input confirmations
   resolved by input.resolve/approval.resolve with within_grant`. In §2.1,
   `grantBoundary` belongs to pending-approval `presentation`, not `PENDING_INPUT` or
   `input.resolve`. Required correction: omit `grantBoundary` from every gap or plan-gate
   input and its annotations. Use `within_grant` only on a lane-B pending tool approval.
   **Resolved by:** `grantBoundary` removed from all input beats; PLAN-2.2 INPUT row and the
   PLAN-5 lane rule now state it is a pending-approval-only field.

5. **P1 must-fix — The status-glyph claim is false and permits loss of state.** Exact plan
   text at fault: `Status glyphs (map 1:1 to the §2.1 closed step set)` followed by `⊘
   skipped/cancelled`. Two distinct closed statuses share one glyph, so the mapping is not
   1:1 and a reader cannot tell a conditional skip from cancellation by steering, stop, or
   re-plan. Required correction: give `skipped` and `cancelled` distinct glyphs and labels,
   and require the literal status in any ambiguous/plain-text rendering.
   **Resolved by:** PLAN-2.2 — `⊘` skipped vs `⊗` cancelled, plus the literal status word on
   every step line (also mirrored in the PLAN-7 HTML chips).

6. **P1 must-fix — UC1 hides multiple external operations as presentation-only substeps.**
   Exact plan text at fault: `Step 2 runs: GitHub read with substeps (one per repo)` after the
   user selects two repositories. §7.2 requires N external calls to be N steps, while §7.4
   forbids substeps from carrying operation identity, effect evidence, or controls. Required
   correction: either name one real generated operation whose schema reads both repositories
   in a single call, or create one plan step per repository, each with its own operation and
   retry/failure state. Substeps may describe phases within one such call only.
   **Resolved by:** UC1 now has one read step per repo (steps 4–5); PLAN-2.2 substep rule
   tightened ("phases within one single operation; never its own external call") and made a
   hard checklist item + reviewer trap 10.

7. **P1 must-fix — A bare Lark 502 cannot prove `not_applied`.** Exact plan text at fault:
   `Tool runs and fails (Lark 502). Step ✕ failed, externalEffect: not_applied (typed evidence
   — nothing was created)`. A gateway error can occur after the downstream accepted the
   mutation; absent stronger evidence, the honest state is `may_have_changed`/`uncertain`,
   which is not retryable automatically. Required correction: make the failure demonstrably
   pre-dispatch, or add a named idempotency/status/postcondition read proving that the exact
   reimbursement instance does not exist before setting `not_applied` and offering retry.
   Otherwise show reconciliation and withhold retry until effect truth is resolved.
   **Resolved by:** UC3 beats 8–9 — 502 lands as `uncertain` + `may_have_changed` with retry
   withheld (`availableActions: [stop]`); a named reconciliation read resolves `not_applied`
   before retry becomes available.

8. **P1 must-fix — UC3 reuses the wrong operation generation.** Exact plan text at fault:
   UC1 establishes that approval re-entry runs at `generation N+1`, while UC3 beat 8 says
   `User clicks Retry → step.retry, generation N+1 → succeeds` after the approved execution
   has already run and failed. Required correction: show the initial approval-waiting
   operation at N, approval re-entry at N+1, and the later retry at N+2, or use the unambiguous
   wording `current generation + 1` with `expectedOperationGeneration` set to the failed
   generation.
   **Resolved by:** UC3 beat 10 — explicit generations: failed run = generation 1, retry sends
   `expectedOperationGeneration: 1` and re-enters at generation 2 ("current generation + 1");
   UC1's lane-A rework removed the ambiguous N+1 re-entry claim.

9. **P1 must-fix — The plan explicitly omits part of §4.4 while claiming to demonstrate §4
   user flows.** Exact plan text at fault: PLAN-6 says ``task.stop` ... omit unless a beat
   needs [it]`, and the coverage map contains STEER and RELOAD but no STOP row. §4.4 specifies
   stop fencing, best-effort cancellation, effect-truth disclosure, and the rule that late
   evidence cannot advance the stopped plan. Required correction: add a bounded stop sequence
   to one transcript (including `stopRequestId`, fence, cancellation result, and any
   `confirmed`/`may_have_changed` disclosure) and add it to PLAN-4/PLAN-5. Do not mark a stopped
   run successful; if the use case still needs a final artifact, distinguish a partial-work
   receipt from a success artifact or start an explicitly new task afterward.
   **Resolved by:** UC2 beats 7–8 — full stop sequence (`stop-uc2-1`, fence, `⊗ cancelled`
   step, effect disclosure, `stopped` task, partial-work receipt) followed by an explicitly
   new task `task-uc2b`; STOP is now a hard coverage row and checklist item; removed from
   PLAN-6.

10. **P1 must-fix — The initial gap input and the plan-gate input have an unresolved step/task
    lifecycle.** Exact plan text at fault: every use case first emits a gap `INPUT`, while
    UC1 beat 4 says `First step = plan-approval pending input`; §7.5 likewise makes the plan
    gate the task's first step, but §2.1 requires every earlier pending input to already carry
    `turnId`, `taskId`, and `stepId`. Required correction: define, using only canonical IDs,
    whether gap collection is a distinct preliminary task/turn or how it precedes the
    execution TaskPlan without making the gate cease to be its first step. Give the gap and
    gate distinct actor-owned `requestId`s and do not silently reuse or orphan a task/step.
    **Resolved by:** PLAN-2.3 canonical lifecycle — one task per goal; revision 1 = gap-input
    step; revision 2 (`addedBy:"replan"`) = execution plan whose first pending step is the
    gate input; distinct `input-ucN-gaps` / `input-ucN-gate` requestIds; the §7.5 "first
    step" interpretation is stated openly in the deliverable.

11. **P2 should-fix — The gap cards are underspecified relative to the closed answer union.**
    Exact plan text at fault: UC1 combines repo multi-selection, week window, and grouping in
    one input; UC3 combines category, cost center, and currency; UC4 combines numeric bar,
    tracker table, and stage. A single `input.resolve` answer is exactly one `freeText` value
    or one flat `selectedOptionIds` list; the contract has no grouped fields or mixed
    free-text-plus-options payload. Required correction: specify a single composite free-text
    answer for each card, or define a flat, opaque option-id scheme that can represent every
    required answer without inventing form fields. Show the user's answer closing every gap;
    UC1's current `User answers (2 repos, grouped)` also needs to accept or replace the week
    window explicitly.
    **Resolved by:** all gap cards are `allowFreeText: true` composite questions answered by
    ONE free-text message shown verbatim closing every gap (UC1 beat 3 now includes the
    window); plan-gate inputs are single `[{proceed}]` selections; codified in PLAN-5.

12. **P2 should-fix — The global “one input card” rule contradicts confirm-gated plans.**
    Exact plan text at fault: `ONE input card per transcript`, while UC1, UC3, and UC4 each
    require both a gap input and a separate first-step plan-approval input. Required
    correction: change the binding interpretation to “exactly one gap-collection message and
    one gap-input request”; confirm-gated transcripts additionally render one distinct
    plan-gate input. They must not share `requestId` or be collapsed into one decision.
    **Resolved by:** PLAN-5 global item rewritten exactly as required (one gap message/input +
    one distinct gate input for confirm plans; disjoint requestIds).

13. **P2 should-fix — PLAN-1.2 scopes the field allowlist to the wrong schema.** Exact plan
    text at fault: `no fields outside §7.3`. §7.3 defines only `TaskPlan`/`TaskStep`; input,
    approval, action, control, continuation, and terminal annotations require fields from
    §2.1. Required correction: restrict the §7.3 field rule to PLAN/task payloads and state
    that every other block uses only the fields of its corresponding §2.1 object or command.
    **Resolved by:** PLAN-1.2 rewritten with the split field-scoping rule; echoed in PLAN-2.4
    and PLAN-5.

14. **P2 should-fix — `CARD: ARTIFACT` is presented as a contract card although no such frame
    or object is defined.** Exact plan text at fault: `Every card, block, and annotation must
    map to a real contract object`, followed by the card-table row `CARD: ARTIFACT ... §4.1
    phase 4`. The contract promises an artifact in the conversation but defines no
    `nyxid.artifact.*` frame. Required correction: render the artifact as an ordinary
    assistant result block, or explicitly label `CARD: ARTIFACT` as a documentation-only
    visual convention that is not a frame, command, or new wire object.
    **Resolved by:** PLAN-2.2 ARTIFACT row relabeled "documentation-only visual convention —
    NOT a wire frame"; the deliverable's convention section must repeat this.

15. **P2 should-fix — UC2's executor label can be mistaken for the reserved `web` source and
    names no concrete ecosystem tool.** Exact plan text at fault: `Aevatar — web search,
    labeled as ecosystem tool, tier 3`, while PLAN-5 prohibits `kind: "web"`. Required
    correction: name the actual mounted Aevatar ecosystem tool/skill and encode it using a
    valid non-web source representation; explicitly annotate that this is `source.kind:
    "tool"` (with the required source identity), not the reserved §8.2 web executor. If no
    such concrete tool is part of the assumed target state, use the tier-4 honest fallback
    instead of inventing one.
    **Resolved by:** UC2 beat 4 — concrete "Aevatar web search skill" with a `source.kind:
    "tool"` annotation carrying the skill identity and an explicit not-the-§8.2-web-executor
    note; mirrored in PLAN-5.

16. **P2 should-fix — UC2 silently changes “book” into a read-only shortlist and auto-runs a
    different outcome.** Exact plan text at fault: `I'll deliver a ready-to-book shortlist
    instead of pretending` followed by `Gate derived auto` and later an `ARTIFACT: shortlist`.
    Honesty requires more than admitting the missing capability: the run must not imply that
    the requested reservation was completed. Required correction: make the communicated plan
    and terminal copy explicitly state that the booking goal is unsupported and that only a
    research artifact will be produced; obtain an in-chat scope decision if the product does
    not permit automatic nearest-alternative reads, and ensure task/artifact status never
    says booked, reserved, or otherwise succeeded against the original effectful goal.
    **Resolved by:** the scope decision is asked inside UC2's single gap card (beat 2) and
    agreed in beat 3; plan prose and the artifact both state "no reservation was made"
    (research artifact); PLAN-5 UC2 item + reviewer trap 6 updated.

17. **P2 should-fix — UC4 invents a defensible employment score without a rubric source.**
    Exact plan text at fault: the only setup is `a "Candidate Tracker" Bitable exists`, the
    gap asks merely for `scoring bar`, and beat 5 produces `82/100`. A threshold is not a
    scoring rubric, so the score and write can look fabricated even in the contract's stated
    résumé scenario. Required correction: add an authoritative role rubric/job description
    to the incoming context, or ask for it in the one gap message; identify it in the
    communicated LLM step and artifact. Do not claim objective fit beyond that supplied
    rubric.
    **Resolved by:** UC4 gap card asks for the JD/criteria ("I won't invent a rubric"); the
    user pastes 5 criteria; the score step and artifact cite "scored against the JD you
    provided".

18. **P2 should-fix — Several status phrases merge a step state with effect evidence.** Exact
    plan text at fault: UC1 beat 9 says `marks the step done / confirmed`. In the closed
    models, `done` is `TaskStep.status` and `confirmed` is `externalEffect`; the slash form can
    be implemented as an invented compound status. Required correction: write and render
    them as separate fields: `status: done; externalEffect: confirmed`. Apply the same
    explicit separation anywhere a status and effect-evidence value appear together.
    **Resolved by:** PLAN-2.2 separation rule ("never a compound"); all beat texts rewritten
    to `status: done · externalEffect: confirmed`; hard checklist item + reviewer trap 5.

19. **P2 should-fix — Approval expiry is excluded even though §1.2 and §4.3 give it binding
    behavior.** Exact plan text at fault: PLAN-6 says `approval-expiry-as-denial ... omit`, and
    no transcript shows expiry or denial. Required correction: add a short denial/expiry
    branch to one approval beat showing that the required step terminalizes, dependent steps
    do not run, and the assistant explains how to ask again. A branch may be clearly marked as
    an alternate outcome so it does not replace the primary successful transcript.
    **Resolved by:** UC3 beat 7 ALT branch (expiry = denial → terminalized filing step,
    `⊗ cancelled` verify, how-to-re-ask), clearly marked alternate; coverage row + PLAN-5
    item added; removed from PLAN-6.

20. **P3 nit — The opening cross-reference points at the contract's gap register, not the
    plan checklist.** Exact plan text at fault: `reviewers hold both docs to the §5 acceptance
    checklist below`. Required correction: call it `PLAN-5` so readers do not resolve `§5` to
    the contract's gap register under the plan's own reference convention.
    **Resolved by:** header now says "PLAN-5 acceptance checklist".
