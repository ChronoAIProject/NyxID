# Chat Action Envelope — v4 Alignment Audit & Implementation Brief

Owner: Calvin · PM/final review: Fable · Implementer: Codex · Adversary: Opus
Date: 2026-07-30 · Status: Phase 1 implemented (frontend only); Phase 2 pending review

Audit of our assistant rich-card **action envelope** (shipped in PR #1273) against the
reference client [`eanz17/nyxid-chat`](https://github.com/eanz17/nyxid-chat) at
`f1807da` (2026-07-30) and the canonical Aevatar contract
`docs/canon/nyxid-chat-api.md` on `aevatarAI/aevatar` `origin/feature/integrate`
(read 2026-07-30). The reference moved 9 commits since our last check
(`docs/ASSISTANT_STREAM_ALIGNMENT.md` was verified against `819dc0d`): it now
implements the full actor-owned schema-v4 protocol and, on 2026-07-29, migrated to
the canonical `POST /api/chat` facade.

Local reference checkouts (read-only, never modify):
`~/Desktop/aelf-frontend-work/nyx-chat` (eanz17/nyxid-chat, `main` = `f1807da`) and
`~/Desktop/aelf-frontend-work/aevatar` (`origin/feature/integrate`).

Phase 1 landed on 2026-07-30 in NyxID's frontend scope: assistant-prose
` ```nyxid:connect ` fences are now inert Markdown in live streams and history,
matching the reference client's v4 behavior.

---

## 1. The reference approach (what they do, and why it's right)

The reference's rule, stated in `public/blocks.js` and enforced by tests: **assistant
prose is always Markdown text; executable cards are built only from actor-authored,
schema-v4 action requests.** Concretely:

1. **Single card source.** Cards come only from `CUSTOM` SSE frames named
   `nyxid.action.request`, validated fail-closed (`public/protocol.js`
   `validateActionRequest`): `schemaVersion === 4`, `action === "service.connect"`,
   all five identities (`actorId`, `originTurnId`, `taskId`, `stepId`,
   `actionRequestId`) present + charset/length-validated, exactly one of
   `params.catalogService | customService`, key allowlists (undeclared field →
   reject), catalog `serviceSlug` charset `/^[A-Za-z0-9._-]+$/`, custom
   `endpointUrl` absolute HTTPS with no userinfo/query/fragment, and a recursive
   secret scan (forbidden key names + `Bearer …`/`nyx_…` value shapes → reject).
   Rejected input renders a safe protocol-error item that can never become a button.
2. **Prose is inert.** Their old fence parser (` ```nyxid:connect ` blocks inside
   assistant Markdown → connect cards) was **deleted** in the v4 migration; a fence
   in prose or history now renders as an ordinary code block. Their old BFF-injected
   `[[NYXID_CONTEXT]]` catalog prose + fence instructions were removed with it.
3. **Idempotency vs conflict.** Same `actionRequestId` re-emitted byte-equal → no-op.
   Same ID with *different* actor/origin/task/step/action/params → explicit
   `conflicted` state: the first request is kept and the journey is disabled. Two
   different IDs with the same slug are independent cards.
4. **Typed continuation.** Journey outcomes go back as one frozen
   `{type:"action.continue", clientRequestId, originTurnId, actions:[{actionRequestId,
   originTurnId, disposition, resource?}]}` body — five closed dispositions
   (`completed|declined|failed|cancelled|expired`), at most one of six safe resource
   variants, and for `service.connect` + `completed` the resource **must** be
   `userService.userServiceId` (the real `UserService.id`; never `api_key_id`, a
   catalog id, or a slug). No `userServiceId` → no completed report; the card stays
   blocked with an explanation.
5. **Completed ≠ success.** Browser `completed` means "journey reported; actor
   postcondition pending". The card shows *awaiting actor verification* after a 2xx
   and flips to *verified* only on actor-authored evidence: the matching action's
   `postconditionResult.verified === true` (disposition/resource matching), or the
   matching postcondition step terminal with `externalEffect: "confirmed"`. A 2xx,
   a green connector tile, or prose never mark success. No original-prompt replay
   exists anywhere (their old `scheduleConnectCardRetry → sendPrompt(originalPrompt)`
   path was deleted).
6. **Reload restores blocked cards.** On reopening a conversation they call the
   current-state query; pending actions restore durable cards. Full params are
   cached per session under `nyxid-chat:v4-action:{actorId}:{actionRequestId}` and
   may restore an *executable* card only when all five identities match exactly;
   a summary-only restore renders a non-executable card that links to NyxID
   management and explains why it cannot infer a target. Params are never
   reconstructed from transcript prose, slug guesses, or catalog state.
7. **Canonical endpoints.** As of 2026-07-29 the reference speaks only
   `POST /api/chat` (typed commands `text`, `action.continue`, `approval.resolve`,
   `task.stop`, `task.steer`, `step.retry`, `step.skip`) and
   `/api/chat/conversations/**`; conversation identity comes solely from
   `RUN_STARTED` (no pre-create, no index polling, single DELETE). The old
   `/api/scopes/{scopeId}/nyxid-chat/**` + `chat-history` family is a deprecated
   compatibility adapter.

Reference anti-patterns we should **not** copy while integrating:

- One malformed SSE frame fails their whole turn (`DEMO_PROTOCOL_ERROR`). Our
  skip-and-continue posture is a documented deliberate divergence — keep it.
- Their pre-v4 history (fence parsing, BFF context injection, prompt replay,
  slug-keyed card identity) is exactly what they deleted; treat those as the
  cautionary list, not as options.

## 2. Conformance matrix (ours = PR #1273, worktree `f0691e23`)

| Contract point | Reference/canon | Ours | Verdict |
|---|---|---|---|
| Cards only from typed `nyxid.action.request` | yes | action cards: yes (`aevatar-transport.ts:2686`) | ✅ |
| `type:"text"` discriminated stream body | required | sent (`aevatar-transport.ts:2006-2016`) | ✅ |
| `action.continue` body shape | typed, frozen, batched per origin turn | matches incl. per-report originTurnId equality + dup rejection (`schemas/assistant-actions.ts:206-288`) | ✅ |
| Six safe resource variants, ids only | yes | matches (`assistant-actions.ts:157-195`) | ✅ |
| `UserService.id` vs `api_key_id` | must be real UserService id | `AddKeyDialog` reports `createdKey.id` (`add-key-dialog.tsx:3134`) | ✅ |
| Stable `clientRequestId` across retries; requeue on error terminal | yes | matches (`aevatar-transport.ts:1691-1696`, `:3383-3396`) | ✅ |
| Unknown verb / wrong schemaVersion → fail-closed card | yes | unsupported card, decline-only (`action-registry.ts:146-170`) | ✅ |
| Approval `requestId` ≠ browser `actionRequestId`, separate routes | yes | separate (`aevatar-transport.ts:1218-1305`) | ✅ |
| **Prose never mints executable UI** | fence parser deleted; fences inert | **` ```nyxid:connect ` fences still parsed into connect cards** (`connect-fence.ts`, `textToBlocks` at `aevatar-transport.ts:572-611`, live + history) | ❌ AP-1 |
| **Same-ID re-emission conflict** | byte-equal → no-op; different → conflicted, disabled | **any re-emission silently rewrites params/origin/actor on the existing card** (`aevatar-transport.ts:3107-3119`) | ❌ AP-2 |
| **completed requires userService resource** | enforced | `completed` without resource is sent when the dialog yields no id (`action-card.tsx:186-210`) | ❌ G-1 |
| **Completed ≠ success presentation** | awaiting-verification → verified on actor proof | card flips straight to a `completed` success receipt on journey 2xx | ❌ G-2 |
| Five identities stored on the card | all five | `taskId`/`stepId` parsed then **dropped** (`assistant-actions.ts:70-81` → `types/assistant.ts:115-126`) | ❌ G-3 |
| Custom `endpointUrl` https-only, no query/fragment | reject → fail closed | http allowed, query/fragment allowed; userinfo → **blank the field but keep the journey** (`action-registry.ts:105-118`) | ❌ G-4 |
| Catalog `serviceSlug` charset | `/^[A-Za-z0-9._-]+$/` | no charset check on the action-params slug | ❌ G-4 |
| Secret-shaped key/value rejection on action input | recursive reject | structural `.strict()` only; no value scan | ❌ G-4 |
| Reload restores blocked cards | state query + session params cache | none — cards lost on reload; next text turn does not re-emit | ❌ G-5 (Phase 2) |
| Actor task/control/continuation frames | projected | all five `nyxid.*` actor frames silently ignored | ❌ G-6 (partial in Phase 2) |
| Canonical `/api/chat` facade | migrated 07-29 | backend pass-through still on deprecated scoped family + `chat-history` dual-delete + index polling (`backend/src/services/assistant_service.rs`) | ⚠ follow-up, out of scope |
| Authoritative controls (stop/steer/retry/skip gated by `availableActions`, exact `expectedStateVersion`) | yes | FE stop sends hardcoded `expectedStateVersion: 0` (`aevatar-transport.ts:3756-3765`); steer/retry/skip FE-absent (backend routes exist) | ⚠ follow-up |
| Empty-actions wake-up (`actions: []`) | supported by canon for out-of-band journeys | never sent (was forbidden by the pre-canon server) | ⚠ follow-up |

Doc rot found while auditing (fix in Phase 1): `docs/ASSISTANT_STREAM_ALIGNMENT.md`
(a) is pinned to reference `819dc0d`; (b) claims `CUSTOM aevatar.authorization.required`
is handled — code handles only `nyxid.authorization.required`
(`aevatar-transport.ts:2681`); (c) gap **G1** recommends holding the original prompt
and offering re-send after connect — obsolete: v4 deleted prompt replay entirely;
the `action.continue` continuation *is* the resume signal and our shipped code
already does this.

## 3. Anti-patterns flagged (the "please flag" list)

- **AP-1 — executable UI minted from LLM prose.** `connect-fence.ts` +
  `textToBlocks` still turn a model-authored ` ```nyxid:connect ` fence into a
  connect card with a live CTA (assistant-role only, but tool results and
  retrieved content routinely steer assistant prose). The reference deleted this
  class entirely: prompt-injected text must never become a click-to-connect
  affordance. Our mitigations (slug regex, catalog re-resolution, non-actionable
  prefill) reduce but don't remove the class. Removal costs nothing extra at
  deploy time: this branch is already hard-gated on new-contract Aevatar by
  `type:"text"`, and the typed `nyxid.authorization.required` + `nyxid.action.request`
  paths cover the real journeys.
- **AP-2 — trusting re-emission over first commit.** Any later frame with a known
  `actionRequestId` overwrites the rendered card's params in place while it is
  pending — the user can read "Connect GitHub", have the card silently repointed,
  and click. The reference keeps the first request and disables the card on
  mismatch (`NYXID_ACTION_ID_CONFLICT`).
- **AP-3 — success theater.** Presenting the journey 2xx as a green "completed"
  receipt claims a success the actor has not verified (canon: "browser completed
  cannot make a step done"). Minimum honest copy: "reported — awaiting assistant
  verification".

## 4. Implementation plan

Scope discipline: Phase 1 is frontend-only (`frontend/src`) + the two doc files;
zero backend changes, no new deps. Phase 2 lands only if Phase 1 is green and
reviewed. Follow-ups are documented, not attempted.

### Phase 1 — envelope correctness (the loop's exit criteria)

1. **Retire fence→card minting (AP-1).** `textToBlocks` returns plain text blocks;
   ` ```nyxid:connect ` renders as an inert Markdown code block in live streams and
   history. Remove the connect-marker special-casing from the live/history paths
   (`closeOpenMessage`, `historyEntryToMessage`) and the mid-stream partial-marker
   withholding (`renderableText` diffing at `aevatar-transport.ts:2500-2503`) —
   with fences inert there is nothing to withhold. Delete `connect-fence.ts` and its
   tests, or reduce it to whatever trivial helper remains genuinely used; sweep
   `mock-data` and fixtures for fence samples. The typed connect-card path
   (`nyxid.authorization.required` → `parseAuthorizationBlocker` → `addConnectCard`)
   and approval/action cards are untouched. Update tests: a fence in assistant prose
   must render as text/code and must NOT create a `connect_card`.
2. **Same-ID conflict semantics (AP-2).** In `addActionCard`: if the incoming
   request is deep-equal (action, originTurnId, actorId, taskId, stepId, params) to
   the stored card's committed request → idempotent no-op (keep status). If it
   differs in any of those → set status `conflicted` (new `ActionCardStatus`
   member): CTA and Decline disabled, note explaining the id conflict; keep the
   first request's params on display; never patch params from the newcomer. A
   terminal card (completed/declined/failed reported) keeps its receipt. The
   existing supported→unsupported downgrade only applies to byte-equal re-emission
   the client can no longer service. Store the committed request on the block (or in
   the run map) so equality is checked against the *first* commit, not the latest
   patch. `conflicted` must also be excluded from continuation sending.
3. **`completed` ⇒ `userService` resource (G-1).** If the journey cannot produce a
   `userServiceId`, do not send a `completed` report: the card stays in a blocked
   "connected, but not verifiable — manage in NyxID" state (no fabricated ids, no
   slug guesses, no `api_key_id`). Enforce at both layers: `ActionCard.report()`
   refuses `completed` without id, and `buildActionContinueBody`/`continueActions`
   rejects a `service.connect` completed report lacking the `userService` variant
   (the call site knows the verb from the card; thread it through).
4. **Honest post-report state (AP-3, minimum slice of G-2).** After a `completed`
   report is accepted (batch settled), the card shows status copy "reported —
   awaiting assistant verification", not a success receipt. `declined/failed`
   receipts stay as-is. Keep wire dispositions unchanged. (Full verified-state
   consumption is Phase 2; do not fake it.)
5. **Store all five identities (G-3).** `ActionCardContentBlock` gains readonly
   `task_id` / `step_id` (may be `""` when the frame omitted them); schema stops
   discarding them. Needed by Phase 2 and by any future step-scoped control.
6. **Fail-closed param hardening (G-4).** In `schemas/assistant-actions.ts` /
   `action-registry.ts`:
   - catalog `serviceSlug` must match `/^[A-Za-z0-9._-]{1,128}$/` → else the card
     resolves unsupported;
   - custom `endpointUrl` must be absolute `https:` with no userinfo, no query, no
     fragment → else **unsupported** (replace the current blank-and-continue);
   - recursive secret scan over the parsed request (port the reference's
     `FORBIDDEN_ACTION_KEY` key regex and `SECRET_VALUE` value regex from
     `nyx-chat/public/protocol.js:1-4`) → match = treat like a failed parse: the
     2-field `recoverUnsupportedAssistantActionRequest` fallback (decline-only
     card), never rendered params.
   Keep the existing tolerance for *absent* `actorId`/`taskId`/`stepId` (protobuf
   default-omission); batches without an actor id already stay unsent.
7. **Docs.** Update `docs/ASSISTANT_STREAM_ALIGNMENT.md`: re-pin to reference
   `f1807da`, fix the `aevatar.authorization.required` taxonomy row, replace G1
   with the v4 continuation reality, and add the follow-up gaps table from §2 of
   this audit (canonical facade migration, authoritative controls, wake-up form).
   Note the fence retirement in both docs.

### Phase 2 — verification + rehydration (only after Phase 1 review)

8. **Actor verification consumption (G-2).** On continuation (and any live) streams,
   parse `nyxid.task.snapshot` / `nyxid.task.step.changed` minimally to extract, for
   actions matching our stored `actionRequestId`, `postconditionResult.verified`
   and postcondition-step `status`/`externalEffect`. Flip "awaiting verification" →
   "verified" only on exact identity + disposition/resource match; anything else
   stays awaiting. **Verify the DTO field names against the Aevatar repo before
   coding** (`NyxIdChatConversationAguiFrameBuilder.cs`, `nyxid_chat_task.proto`,
   read-only) — do not guess from the reference JS alone.
9. **Reload rehydration (G-5).** Cache validated request params in `sessionStorage`
   under `nyxid:v4-action:{actorId}:{actionRequestId}` at frame receipt (no
   credentials, no user input ever). On conversation open, call the existing
   pass-through `GET /api/v1/assistant/conversations/{id}/state`; restore pending
   actions as cards: executable only when the cache hit matches all five identities
   exactly; otherwise a non-executable explanatory card linking to `/keys`.
   Handle the four canon statuses (`current`/`not_modified`/`reload_required` once,
   bounded/`not_found`) without loops.

### Follow-ups (separate efforts, do not start)

- Canonical `/api/chat` facade migration of the backend pass-through (removes
  pre-create, index polling, dual-delete; conversation identity from `RUN_STARTED`).
- Authoritative controls: state-version tracking, `availableActions` gating,
  steer/retry/skip UI, real `expectedStateVersion` on stop.
- Empty-actions wake-up for journeys finished outside the page.
- Display-redaction pass over rendered action params.

## 5. Hard rules (unchanged from the #1273 brief; still binding)

1. Never put a secret/token/credential in cards, continue bodies, receipts, logs,
   or fixtures. 2. Outbound bodies are explicit literals — Aevatar rejects unknown
   members. 3. Don't break approvals, typed connect cards, keepalive, or
   unknown-frame tolerance. 4. No `console.log`; kebab-case files; readonly types;
   zod in `schemas/`. 5. Gates: `npm run build`, `npm run test`, `npm run lint`
   from `frontend/`, all green (build is the CI gate: `tsc -b` with
   `noUncheckedIndexedAccess`). 6. Frontend + docs only; no backend/cli/sdk/mobile
   changes; no new deps. 7. Conventional commits on this branch; commit locally,
   do not push.

## 6. Acceptance (Phase 1)

1. A streamed assistant message containing a well-formed ` ```nyxid:connect ` fence
   renders it as an inert code block — no card, live or from history.
2. `nyxid.action.request` byte-equal re-emission → one card, unchanged. Re-emission
   with different params → card shows conflicted, both buttons disabled, original
   params still displayed, nothing sent on the wire.
3. Completing the AddKeyDialog journey with a created key → one `action.continue`
   with `completed` + `userService.userServiceId`; card then reads "reported —
   awaiting assistant verification". A journey that yields no id → no completed
   report, card explains why.
4. An action request with `http://`, a query string, a fragment, a bad slug, or a
   `Bearer`-shaped value anywhere → unsupported/decline-only card, no params
   rendered from the offending request.
5. All existing suites green; new tests cover each bullet above.
