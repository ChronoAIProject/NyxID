# Stage-4 adversarial review — `nyx-chat-use-cases.md` + `nyx-chat-wf/index.html`

- **Reviewer:** stage 4 (adversarial). Deliverables were **not** edited.
- **Reviewed artifacts:** `docs/nyx-chat-use-cases.md` (739 lines, SSOT) ·
  `docs/nyx-chat-wf/index.html` (1,628 lines / 77,792 bytes) ·
  `https://nyx-chat-wf.surge.sh/` (fetched 2026-08-06).
- **Contract:** `docs/nyx-chat-aevatar-support-spec.md` (Draft v3). `§` = contract;
  `PLAN-` = `docs/nyx-chat-use-cases-plan.md` v2.
- **Method:** all 11 PLAN-5 reviewer traps run against the actual transcript text; every
  PLAN-5 **(hard)** item checked independently of the implementer's self-assessment; HTML
  parity checked by executing the page's own renderer headlessly and diffing rendered output
  against the markdown SSOT.

**Note on the restyle.** I was told `index.html` had been restyled to NyxID prod styling since
my run started. It has not landed: local `index.html` and the surge-served body are
byte-identical to each other (77,792 bytes, `diff` clean) and to the copy I analysed
(mtime 2026-08-05 18:05). Findings 9 and 10 below are renderer-logic findings and survive a
restyle; finding 15 is presentational and is flagged for stage 5 rather than owned by me. If a
restyled file lands later, **only** findings 9, 10 and 15 need re-checking — the markdown SSOT
findings (1–8, 11–14, 16) are unaffected.

---

## P1 — must fix before PR

### 1. UC4 performs a downstream Lark data read before the communicate beat (fails a hard item)

> "I checked the relevant capability: Lark is connected and executable, **and the Candidate
> Tracker exposes the "2026 Pipeline" table.**" (`nyx-chat-use-cases.md:622-624`)
> "*(wire: disclosed Class-R **readiness/table discovery** precedes `nyxid.task.snapshot` …)*"
> (`:647`)

PLAN-1.6 and the PLAN-5 hard item allow exactly one pre-communicate exception: "§4.1 Phase-2
**capability-resolution reads** (readiness, connected-service inventory, tools/list)". Knowing
that a *specific Bitable* exposes a *specific named table* is neither readiness, nor
connected-service inventory, nor `tools/list` — Bitable table names are runtime downstream
data (generated tools are `{slug}__{operation}` from OpenAPI; a table name is a call result,
not a tool name). So UC4 fires a downstream Lark call before the approach is communicated,
which is the exact behaviour stage-2 finding #1 was raised to kill, and which the hard item
states as "No LLM operation, inventory read, or downstream call before the communicate beat".

It also buys nothing: the user *already named the table* in the gap answer
(`:616-618`, "Use the "2026 Pipeline" table").

**Required fix:** cut the table-discovery clause. Reduce the disclosed read to
readiness/inventory only — e.g. "I checked: Lark is connected and executable." Let the table
name come from the user's answer, and if existence must be proven, add an explicit `tool` step
*inside* revision 2 (after the gate) that reads the table list, with its own executor label.
Update the `:647` wire note to drop "table discovery".

### 2. UC3's retry re-executes an approval-gated, effect-capable tool with no second NyxID gate and no explanation

> " 4 ◐ running   Lark Approval - file the reimbursement · source: tool:
> lark-feishu__create_approval_instance … status: running" (`:556`)
> " 4 ● done      … status: done · externalEffect: confirmed" (`:566`)

Between the user's retry (`:546-549`) and success, no `CARD: NYXID APPROVAL` appears and
nothing explains why one is not needed. §7.5 is explicit and binding: "Per-step NyxID
authority is unchanged and non-bypassable: **approval-gated tool steps still raise
`nyxid.approval.request` at execution time**". The approval that *was* granted carried
`grantBoundary: nyxid_step_up` (`:476`) — a step-up is per-operation, not a standing grant, and
the retry is a new operation at a new generation. As written, the transcript teaches the
reader that a retry silently inherits a consumed step-up approval, which contradicts §1.2
("Aevatar … can never loosen or bypass a NyxID gate") and is precisely the lane-A/lane-B
conflation the whole document is built to avoid.

**Required fix:** either (a) render a second `CARD: NYXID APPROVAL`
(`nyxid-appr-uc3-2`) before the retry executes — this is the honest per-request default and
costs ~8 lines; or (b) state in assistant prose + a wire note that NyxID's per-service config
holds a time-boxed grant covering the retry window, naming it as NyxID's decision, not the
assistant's. Option (a) is preferred: it also demonstrates that a retry is a fresh
authorization, which no other transcript shows.

---

## P2 — should fix

### 3. UC3's `addedBy` flips from `replan` to `initial` on the same preserved step

> Revision 2: " 5 ○ planned   Verify - confirm the reimbursement exists · … · **addedBy:
> replan** …" (`:430`, `:456`, `:502`, `:525`)
> Revision 3: " 6 ○ planned   Verify - confirm the reimbursement exists · … · **addedBy:
> initial** …" (`:541`, `:558`, `:568`)

Same `stepId`, same title, same source — `addedBy` changes across a re-plan. §7.6 requires
completed/preserved steps be "preserved verbatim", and §7.3 defines `addedBy` as the provenance
of the step's *creation*. `initial` is additionally wrong on its face: this step was never in
revision 1 (revision 1 held only the gap input, per the deliverable's own lifecycle note at
`:43-45`). A renderer diffing revisions off `addedBy` (§7.6: "The renderer shows revision diffs
from `addedBy` + `cancelled` + `planRevision`") would mis-draw this as an original step.

**Required fix:** set `addedBy: replan` on the verify step in all three revision-3 cards
(`:541`, `:558`, `:568`).

### 4. UC3's generation ladder omits the approval re-entry bump, contradicting §4.3

> "*(wire: `step.retry` · … · **expectedOperationGeneration=1** · expectedStateVersion;
> **generation 2 is current generation + 1**.)*" (`:549`)

§4.3 OUTCOMES is explicit that this applies to **both** lanes: "Approved → the step proceeds
(**tool re-enters at generation N+1** under an exact grant)". So the filing step's ladder must
be: generation 1 = the pre-approval dispatch that raised `nyxid-appr-uc3-1`; generation 2 =
the post-approval re-entry, which is the run that took the Lark 502; retry therefore carries
`expectedOperationGeneration: 2` and lands at generation 3. The transcript instead has the
approved execution and the pre-approval dispatch silently share generation 1 — the exact
ambiguity stage-2 finding #8 was raised to remove, reintroduced from the other side.

**Required fix:** either renumber (`expectedOperationGeneration: 2`, re-entry at generation 3)
and add one wire note at the approval beat naming the N→N+1 approval re-entry, or state
explicitly that the pre-approval dispatch is not counted as a generation and reconcile that
with §4.3's "both lanes" wording.

### 5. UC2b's prose implies per-candidate external calls inside a single search step (trap 10)

> Plan card: " 1 ◐ running   Aevatar web search - check shortlisted candidates …"
> with substeps "Check private-room evidence **in one result set**" / "Check Friday 7 pm
> evidence **in the same result set**" (`:346-348`)
> Assistant, after that step: "I could not check Atlas Taverna's Friday hours right now because
> **its site was unreachable**" (`:361-362`)

A single search-result-set read cannot produce a per-restaurant "its site was unreachable"
signal — that outcome only exists if the step fetched Atlas Taverna's site. The card asserts
one operation; the prose reports evidence only N operations could produce. That is reviewer
trap 10 ("a step secretly looping") and, read literally, the §7.2 hard item ("N external calls
are N steps").

**Required fix:** make the unverifiable-candidate honesty come from the *search result* rather
than a site fetch — e.g. "the search results don't state Atlas Taverna's Friday hours, and I
haven't opened its site, so I can't confirm them" — or promote per-candidate verification to
its own steps. Either fix preserves the §1.2 cannot-check lesson, which is the beat's real
purpose.

### 6. UC3 marks an in-flight effect-capable call `externalEffect: not_applied`

> " 4 ◐ running   Lark Approval - file the reimbursement · … · status: running ·
> **externalEffect: not_applied** · availableActions: [stop]" (`:556`)

Every other `running` step in the deliverable carries `not_started` (8 of 9; verified by
sweep). More importantly this specific claim is the error UC3 spends two beats teaching
against: once a mutation is dispatched you cannot assert it did not apply. The prior generation
being proven `not_applied` (`:544`) says nothing about the generation now in flight.

**Required fix:** render the retry's running row as `externalEffect: not_started` (before
dispatch) — or, if it is mid-dispatch, `may_have_changed` — and keep `not_applied` only on the
terminalized failed generation.

### 7. HTML: the "verified against …" footer never renders for the three artifacts that have one

The renderer gates the footer on `/^(Verified against|Research artifact only):/`
(`index.html:1545`), which requires a colon immediately after "Verified against". The actual
lines are:

> " Verified against **#eng-updates**: the message exists…" (`:222`)
> " Verified against **Lark Approval**: the instance exists…" (`:584`)
> " Verified against **Lark Candidate Tracker**: the row exists…" (`:738`)

Executed headlessly: **1 of 4** artifacts renders `verified-footer` — and the one that does is
UC2, the *pure-read* artifact that has nothing to verify ("Research artifact only:"). The three
real verification statements fall through to ordinary key/value rows. PLAN-7.4 requires
"`ARTIFACT` — success-tinted card with the 'verified against …' footer", and verify-before-
success is a hard item; the HTML currently de-emphasises exactly the evidence the deliverable
exists to showcase. (Content is not lost — only its treatment.)

**Required fix:** change the test to `/^(Verified against\b|Research artifact only:)/` (or
`/^Verified against/`) so all four artifacts get their footer.

### 8. HTML: the transcript-convention and framing section is dropped entirely

`parseSections` keeps only `^## UC[1-4] - …` blocks (`index.html:1460`), so everything above
UC1 in the SSOT is never rendered. The standalone page therefore omits:

> "`CARD: ARTIFACT` is a documentation-only visual treatment … **It is not a wire frame,
> command, or contract object.**" (`:28-29`)
> "an Aevatar-scoped tool approval is intentionally out of scope, so `approval.resolve` does
> not appear in any of the four scenarios." (`:48-50`)
> plus the status-glyph legend, the ID scheme, and the §7.5 "first step" interpretation note
> (`:31-47`).

Stage-2 finding #14 was resolved specifically by requiring "the deliverable's convention
section must repeat this" — the HTML deliverable has no convention section, and its `Artifact`
card kicker is styled with the same weight as `Plan`, `Input`, `Connect` and `NyxID approval`,
all of which *are* contract objects. A reader who only opens the surge link is left to infer
that `ARTIFACT` is a fifth wire card and that `approval.resolve`'s absence is an oversight.

**Required fix:** render the pre-UC1 material once — either as a collapsed
`<details>` "Transcript conventions" block above the tabs, or as a fifth tab. At minimum, carry
the ARTIFACT disclaimer and the deliberate-`approval.resolve`-absence sentence into the page.

---

## P3 — nits

### 9. UC1's step 7 loses its `NyxID approval gate` marker in every later re-render

> Revision 2, first render: " 7 ○ planned   Lark - post to #eng-updates · source: tool:
> lark-feishu__send_message · **NyxID approval gate** · addedBy: replan …" (`:97`)
> All later renders of the same step: marker gone (`:151`, `:163`, `:205`).

Same step, same revision, unstable rendering. **Fix:** carry the marker on every render of step
7, or drop it from `:97` and rely on the approval card alone.

### 10. UC3 exceeds the PLAN-2.5 per-use-case length budget

UC3 runs 204 transcript lines (`:383`–`:586`) against the stated budget of "90–180 transcript
lines per use case". UC1 = 173, UC2 = 158, UC4 = 153; deliverable total 739 ≤ ~1100, so only
UC3 is over. Cause is six near-identical full plan re-renders. **Fix:** drop one or two
intermediate plan cards (the pre-retry `running` card at `:552-559` adds nothing the post-retry
card doesn't), or note the budget overrun as accepted.

### 11. `source: tool: <mcp-tool-name>` is not a §7.3 `kind:"tool"` field

§7.3 defines `{ kind: "tool"; serviceId?; serviceSlug; readinessCapabilityId? }` — the MCP tool
name is a §7.3 field only on `postcondition` (`check`). The transcripts render
`source: tool: api-github__list_pull_requests` throughout. This follows PLAN-2.2's own example
so it is not an implementer deviation, but it is a field-scoping stretch under PLAN-1.2.
Sharper case: `source: tool: aevatar-web-search-skill` (`:269` and 5 more) has **no**
`serviceSlug` at all, because an Aevatar ecosystem skill has no NyxID service. **Fix:** either
add one line to the convention section stating that `source: tool: X` renders the step's tool
identity (slug-derived where NyxID-brokered, skill identity otherwise), or render
`serviceSlug` + tool name explicitly.

### 12. The framing note claims G1–G9 closed, but UC1 depends on G9 being open

> "gaps G1-G9 are assumed closed except where the contract itself **reserves a capability or
> leaves a bridge unspecified**" (`:3-5`)
> vs. "Sequential connect cards are the honest v1 G9 behavior." (`:108`)

G9 (connect-card batching) is neither a reserved capability nor an unspecified bridge — §5
calls it "undecided". **Fix:** widen the carve-out to "…reserves a capability, leaves a bridge
unspecified, or leaves a v1 scope decision open (G9)".

### 13. UC3 renders the retry as a plain text turn; UC2 teaches that this is rejected

> UC2: "*(wire: plain `text` would be rejected with `ACTIVE_TURN_REQUIRES_STEERING` …)*" (`:294`)
> UC3: "**User** > Retry the filing step." followed by "*(wire: `step.retry` …)*" (`:546-549`)

PLAN-3 UC3 beat 10 specified "User **clicks** Retry". Rendering it as a chat message invites
the reader to ask why the §2.1 active-turn rule didn't fire here. **Fix:** replace the user
message with a narrator line — *[narrator: Calvin clicks Retry on the failed step.]* — or add
half a wire clause noting the turn had already terminalized.

### 14. UC3's approval-denial ALT branch is visually indistinguishable from an execution failure

> " 4 ✕ failed    Lark Approval - file the reimbursement · … status: failed · externalEffect:
> not_applied · availableActions: []" (`:501`)

§4.3 says denial produces a "typed denied receipt"; the row reads identically to a 502-style
failure, and only the prose (`:505-508`) distinguishes them. **Fix:** add `· outcome: denied
(approval expired)` to the step row, or mention the typed denied receipt in the ALT wire note.

### 15. HTML: markdown soft-wraps render as hard `<br>` breaks (flagged for stage 5)

`renderTextBlock` joins source lines with `<br>` (`index.html:1361`), producing 70 hard breaks.
Most are invisible at 760 px, but at least one breaks mid-sentence:

> "… **Console**<br>- Shipped the integrity status view …" (from `:174-175`)

and at ≤400 px every wrapped source line double-breaks. **Fix (stage 5's call):** join
paragraph lines with a space and let CSS wrap; use `<br>` only for genuinely blank-line-
separated blocks.

### 16. Consecutive full plan re-renders with no intervening message (realism)

UC1 emits two full plan cards back to back (`:144-165`) — step 4 running, then step 5 running —
with nothing between them. UC3 emits six. A shipped product mutates one live run card in place;
a stack of near-identical cards is a documentation convention, not product behaviour, and the
HTML reproduces it faithfully. **Fix:** acknowledge in the convention section that repeated
PLAN cards represent one card re-rendering in place, so the HTML isn't read as the product
spamming the thread.

---

## Verified clean (attacked, found nothing)

Recorded so the PR shows these were tested, not skipped.

- **Trap 1 (drip-feed / tool-backed gap card):** exactly one gap message and one
  `input-ucN-gaps` per transcript; all four gap cards are `allowFreeText` composites answered
  by one user message that closes every gap; UC1/UC2/UC3 gap cards are authored from the user's
  message only. (UC4's *post-answer* read is finding 1; its gap card itself is clean.)
- **Trap 2:** `approval.resolve` appears exactly once in the whole document — in the convention
  section, declaring its deliberate absence (`:49`). Zero lane-A occurrences. No invented G8
  bridge command anywhere; every lane-A resume is "wake/recheck".
- **Trap 3:** UC1's `action.continue: completed` is explicitly a signal, with
  `nyx__list_connected_services` run as postcondition before `done · confirmed` (`:135-141`).
- **Trap 4:** no success claim precedes its verify step in any transcript; all three effectful
  artifacts carry "Verified against …"; UC2's stopped run delivers a labelled partial-work
  receipt and its artifact says "no reservation was made" three times.
- **Trap 5:** zero compound statuses (swept for `done/confirmed` and siblings); every frame
  name used (`nyxid.action.request`, `nyxid.input.request`, `nyxid.task.snapshot`) is on the
  §2.1 committed list; every status word is in the closed set and spelled out on every row;
  `grantBoundary` appears only on the three approval cards, never on an input.
- **Trap 6:** UC2 books nothing, emits no `web` step (zero occurrences of `kind: "web"` /
  `source: web`), and the research-only scope is agreed in the gap answer before any work.
- **Trap 7:** turn/task identities are clean — `turn-uc1-2`, `turn-uc2-2`, `turn-uc2b-1` are all
  new ids; `taskId` is stable within each goal; the stopped `task-uc2` is never resumed
  (`task-uc2b` is explicitly new); gap and gate `requestId`s are disjoint in all four.
- **Trap 8:** UC2's steer is `task.steer` with `steeringId`, a fence, `planRevision` 2→3, the
  replacement step `addedBy: steering`, and the completed search preserved and not re-run.
- **Trap 9:** retry is withheld while `? uncertain · may_have_changed`
  (`availableActions: [stop]`, `:524`), and only becomes available after the named
  reconciliation read resolves `not_applied` (`:539-544`). (The generation *number* is finding 4.)
- **Trap 11:** no token, key, code, or credential value anywhere; the Lark OAuth journey happens
  behind a narrator line inside NyxID; the connect card carries scopes only.
- **Identity families:** `nyxid-appr-*`, `act-*`, `input-*` are three disjoint schemes,
  consistently used.
- **HTML integrity:** single file, zero external requests (no `src=`, `href=`, `@import`,
  `url()`, or `fetch`); inline JS parses cleanly (`new Function` check); 4 tabs render with
  correct `role="tablist"`/`aria-selected` plus arrow-key and `#ucN` deep-link handling; mobile
  CSS present at 620 px and 400 px breakpoints with a `prefers-reduced-motion` block.
- **HTML content parity:** the embedded `<script type="text/plain">` source is **byte-identical**
  to the markdown SSOT (46,278 chars, `difflib` diff = 0 lines). Executing the page's renderer
  headlessly reproduces 39/39 messages, 39/39 wire annotations, 8/8 narrator lines, 39/39 cards
  (20 plan · 7 input · 2 connect · 6 approval · 4 artifact), 103/103 plan step rows, 5 substep
  groups, the 1 inline table, and the 1 `<details>` ALT branch. No beat is dropped. Served copy
  at surge.sh is byte-identical to local.

---

## Counts

| Severity | Count |
|---|---|
| **P1** | 2 |
| **P2** | 6 |
| **P3** | 8 |
| **Total** | 16 |

## Verdict

**SHIP-WITH-FIXES.**

The deliverable is substantially contract-faithful: all 11 reviewer traps were probed, 9 come
back clean, and HTML↔markdown parity is exact rather than approximate (the page renders the
SSOT itself, byte-for-byte, and drops nothing). The two P1s are narrow and cheap — one clause
to cut in UC4, one approval card or one sentence to add in UC3 — but both must land before PR:
finding 1 fails a PLAN-5 **(hard)** item outright, and finding 2 currently teaches that a retry
bypasses a NyxID step-up gate, which inverts the single most load-bearing invariant the
document exists to demonstrate. Fix P1s and P2s 3, 4, 6, 7 (all mechanical); P2s 5 and 8 are
one paragraph each; P3s are optional polish.
