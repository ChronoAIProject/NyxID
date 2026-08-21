# Assistant Waves — Completion Plan

- **Date:** 2026-08-21 · **Author:** PM session · **Branch:** `feat/assistant-waves` @ `9b247448` (= origin, PR #1471, tree clean)
- **Inputs:** `docs/chat/assistant-waves-plan.md` §8/§9 · `/tmp/fable-audit.md` (F1–F9) · PR #1471 body · direct re-verification of HEAD
- Companion conventions: `docs/chat/evidence-projection-conventions.md` (reference this in agent briefs; never enumerate credential/token/secret terms inline — that is what killed two agents)

---

## 0. Ground-truth corrections (verified at HEAD before planning)

Three facts in the brief needed correction; they change the shape of the work.

1. **The "9 missing verbs" are not missing — committed wrappers exist for all 30 Wave-4 verbs.** `assistant_action_effects_org.rs` declares, implements, and routes every verb including `org.member_update_role` (`:1595`), all five `service_account.*` (`:1741–2015`), `developer_app.rotate_secret` (`:2175`, and it does return the rotated secret — I verified, `:2220`), `notifications.telegram_disconnect` (`:2323`), `external_key.add_gcp_service_account` (`:2370`). These landed in `eb43dbfa`/`4a00d021`/`87a0dea7` — the killed agents' residue, committed by the sibling. So the task is **not greenfield implementation; it is adversarial review + hardening + tests of untrusted residue** (FI-001). Concretely visible already: every org handler answers `InProgress` with an unconditional 409 (the F8 wedge, module-wide); `service_account.rotate_secret`'s fingerprint is `{id}` only (no fence, no semantic content); `member_update_role` binds the target role but not the prior state; the module has 4 tests for 30 verbs.
2. **Wave 3 has zero dialogs.** The 28 dialogs on disk cover Wave 1 (2), Wave 2 (12), Wave 4 (14, several parameterised across verbs). No node/device/pending-credential dialog exists. "28 built, wiring is the gap" is true for W2/W4; for W3 the journeys themselves are unbuilt. This feeds the cut list (§7).
3. **Smaller updates:** `assistant_action_effects_org` now has 4 tests (not 0); all family `/authorization` projections are mounted on the production router (users/consents/api-keys/bindings/keys/endpoints/external-keys/orgs/members/service-accounts/notifications/grants/service-configs/developer-apps — verified in `routes.rs`), confirming F1 and the F5 mount-half are closed; F7 confirmed still open (`users.rs:81` byte `len` vs dialog UTF-16 `.length`); PR #1471's body is stale on Wave-4 counts ("8 of 30", "22 unimplemented") and on `account.delete` ("not yet fixed" — correct today, wrong after C1 lands). C9 refreshes it.

---

## 1. Ordered task list

| # | Task | Touches | Unblocks | Owner |
|---|---|---|---|---|
| C0 | Pre-land shared contracts: keyed-fingerprint helper (HMAC, domain-derived key, same pattern as CLI pairing); named pointer to the keys-module `InProgress`-resume exemplar; wiring conventions note; file-ownership map (below) | `assistant_action_receipts.rs` (or sibling util), this doc | C3, C4, C5, C6 | PM |
| C1 | **F2**: `account.delete` confirmation server-side — email in request body, compared against the account, bound into the fingerprint | `assistant_action_effects_org.rs`, `assistant-account-delete-dialog.tsx` + test | C6 (vacates org file only after C5) | PM |
| C2 | **Harness refactor**: table-driven `action-card.tsx` dispatch; migrate the 4 Wave-1 verbs into it; zero new verbs (§4) | `action-card.tsx`, `action-registry.ts`, `schemas/assistant-actions.ts` | C7, C8 | PM |
| C3 | **Stream A** — services + endpoints: F3 (cascade port), F6 (sibling-set binding), F8 for both modules, F9b (keyed rotate fingerprints), replace the 3 vacuous tests named in §5, module test gaps | `assistant_action_effects_services.rs`, `_endpoints.rs`, `assistant-service-*` / `-endpoint-*` / `-external-key-*` dialogs + tests | C7 | Dispatch |
| C4 | **Stream B phase 1** — nodes: F5 (state fencing on `node.delete`/`node.transfer`), F9d (stale cancel comment), nodes test campaign (falsifiable, ~8–10 tests incl. replacing the 2 near-vacuous ones) | `assistant_action_effects_nodes.rs`, `node_pending_credential_service.rs` (comment only) | — (backend-complete W3; journeys deferred §7) | Dispatch |
| C5 | **Sensitive Wave-4 residue**: harden + test `org.member_update_role`, `service_account.create`, `service_account.rotate_secret`, `developer_app.rotate_secret`, `external_key.add_gcp_service_account`; F9c (MFA resume proves the pinned factor); F8 for these arms (§6 argues ownership) | `assistant_action_effects_org.rs` + the sa / dev-app / org dialogs' secret-display paths | C6, C8 | PM |
| C6 | **Stream B phase 2** — org remainder: F8 for remaining arms, harden + test the 4 mechanical residue verbs (`service_account.{update,delete,revoke_tokens}`, `notifications.telegram_disconnect`), F7, org test campaign, replace `assistant-wave4-account-approval.test.tsx` (all five tests are vacuous) | `assistant_action_effects_org.rs`, `users.rs` (F7), wave-4 dialogs + tests | C8 | Dispatch |
| C7 | **Wiring tranche 1** — Wave 2, 12 verbs: param variants, registry rows, adapters, journey tests | the 3 registry files + per-dialog adapters | v9 | Dispatch (single writer) |
| C8 | **Wiring tranche 2** — Wave 4 curated (27 verbs: 30 − `account.delete` − `account.mfa_setup` − `openclaw.connect`, §7) | same 3 files + adapters | v10 candidate | Dispatch (single writer) |
| C9 | PM verification sweep (consolidated module test run on a fresh dbpath), PR #1471 body refresh, docs cross-refs | PR body, `docs/chat/*` | honest merge | PM |
| C10 | **v9 bump — Wave 2 only** (§8): revision-map entry, fixture, single-line revision change; file the four upstream asks | `assistant_actions.rs`, fixture | production reachability | PM |

Not listed as tasks because they are already done and only need to stay done: F1, F4, the mount smoke tests, revision negotiation.

---

## 2. Dependency graph and the first move

```
C0 ──► C3 (Stream A: services+endpoints) ──────────► review A ──► C7 (wire W2) ─┐
   ──► C4 (Stream B ph.1: nodes)        ─► review B1                            │
C1 ──► review (small, early — the route is live today)                          ├─► C9 ─► C10 (v9 = W2 only)
C2 (harness) ────────────────────────────► review W0 ─────────────► C7, C8      │
C5 (PM, org file) ─► C6 (Stream B ph.2, org file) ─► review B2 ─► C8 (wire W4) ─┘
```

Hard orderings:
- **C5 before C6** — same file (`assistant_action_effects_org.rs`), one `.git/index`; the PM vacates the org module before Stream B enters it. Stream B is not idle meanwhile: phase 1 (nodes) is disjoint and provides the runway.
- **C2 before C7/C8** — no wiring tranche starts until the table-driven harness is reviewed; otherwise we hand-write JSX branches we will immediately rewrite.
- **C7 and C8 are mutually serial** — both write the same 3 registry files. One writer at a time, ever.
- **Review-close gates wiring**: a family is wired only after its backend batch has passed Calvin's review (§4 Q3).
- C10 additionally gates on upstream (the `?revision=` fetch deployed, or v9 acceptance merged) — outside our control; everything else in this plan is doable without it.

**Single highest-leverage first move: C2, the table-driven harness — started the same hour C3/C4 are dispatched (they don't conflict).** Reasons: it is single-writer, so it can never be parallelised later and every day it isn't done is a day the wiring critical path hasn't started; it converts ~39 future JSX branches into data rows, which shrinks per-verb review cost — and review, not agent capacity, is the programme bottleneck; and it is provable against known-good Wave-1 behavior now, while the surface is small. (C1 is the highest-*urgency* item — a live mounted route with a client-only gate on an irreversible delete — but it is half a day; do it first in the PM queue, then C2.)

---

## 3. Two-stream design

### File ownership (exhaustive; anything not listed is PM-only)

| Writer | Backend | Frontend |
|---|---|---|
| **Stream A** | `assistant_action_effects_services.rs`, `assistant_action_effects_endpoints.rs` | `assistant-service-{update,delete,route,rotate-credential}-dialog.tsx`, `assistant-endpoint-*`, `assistant-external-key-*` + their `.test.tsx`, `frontend/e2e/wave2-service-actions.spec.ts` |
| **Stream B ph.1** | `assistant_action_effects_nodes.rs`, `node_pending_credential_service.rs` (one comment) | — |
| **Stream B ph.2** | `assistant_action_effects_org.rs` (after PM handoff), `handlers/users.rs` (F7 only) | `assistant-account-*`, `assistant-approval-*`, `assistant-org-*`, `assistant-service-account-*`, `assistant-developer-app-*`, `assistant-notifications-*` dialogs + tests, `assistant-wave4-account-approval.test.tsx` |
| **PM only, always** | `routes.rs`, `assistant_actions.rs`, `assistant_action_receipts.rs`, `db.rs` | `action-card.tsx`, `action-registry.ts`, `schemas/assistant-actions.ts` |

### Shared contracts the PM pre-lands (C0) before any dispatch

1. **Keyed-fingerprint helper** in the receipts module: `fingerprint_credential_material(...)` HMAC-keyed via the existing domain-derivation pattern. Needed by Stream A (F9b: rotate fingerprints in both modules) and C5 (`add_gcp_service_account`). Without pre-landing, two streams invent two helpers in two files and the PM merges them by hand — the exact Wave-2 failure.
2. **Resume-pattern pointer**: one paragraph in this doc naming `commit_key_update` / the keys `InProgress` arms as the exemplar every F8 fix must copy (verify-effect-landed → `mark_completed` → return `replayed: true`), plus the falsifiability recipe (`key_update_interrupted_retry_does_not_apply_twice` forcibly reopens the receipt). Streams copy a pattern; they do not design one.
3. **Ownership map above**, pasted into both briefs verbatim.
4. Registry files are already stable — no Zod/descriptor pre-land is needed this round (descriptors are all published; C2 owns the card-params side).

### Ground rules for both streams (measured constraints, not style)

- Stage only owned files; never `git add -A`; never `git add` a registry file.
- Targeted module tests only: `cargo test -p nyxid assistant_action_effects_<module>`. Never the full suite (5,400 tests exhausts mongod). Never run tests while the other stream is running theirs — coordinate through the PM; PM runs the consolidated sweep at C9 on a wiped dbpath.
- Briefs reference `docs/chat/evidence-projection-conventions.md` instead of naming credential/token/secret terms inline (content-filter attrition is real and killed the last two agents on exactly the C5-class verbs).
- No PR touches `ASSISTANT_ACTIONS_REVISION`. Frontend gate is `npm run build`, not `tsc --noEmit`. No dep/lockfile changes (wizard freshness).
- Every new test lands with a one-line note in the commit/PR naming the mutation or revert that makes it fail. Tests without a named falsifier are rejected in review — this is the recurring defect (8 named in the audit).

---

## 4. Journey wiring — the three questions

**Q1: table-driven, yes — and the table already half-exists.** `action-registry.ts` is already a per-verb descriptor map (`title/body/cta/risk/journey`); the only JSX-branch part is `action-card.tsx` translating `journey` strings into `<Dialog>` mounts. Extend `ActionDescriptor` with a dialog binding — `{ component, mapParams(params, ctx) }` — and have the card render `registry[action].dialog` through one generic mount point. Adding a verb becomes: one `ActionCardParams` variant (per *dialog shape*, not per verb — the parameterised W4 dialogs mean ~20–24 variants cover 39 verbs), one registry row, zero card edits. Type safety via a mapped type keyed on the variant so a row cannot pair a dialog with the wrong params. The `unknown` fallback variant already exists and stays the safety net. At 1,135 lines for 4 verbs, 50 JSX branches is not survivable and would make every future wiring PR a merge conflict in the same switch; a data row is also a materially cheaper review unit, which is the bottleneck.

**Q2: split — one harness PR, then per-family data PRs.** PR-W0 (C2) is the refactor alone: introduce the binding, migrate `key_create` and `key_rotate` into it, leave the `service.connect`/`reauthorize` special paths as-is (they route through other components; migrating them buys nothing and risks shipped behavior). Falsifiable gate: the existing assistant vitest surface passes **unmodified** — the tests are the spec; if a test needs editing, the harness changed behavior. Then C7/C8 are data-plus-adapters PRs that review in minutes per verb. One mega-PR would put a structural refactor and 39 verbs of copy in front of one reviewer in a single sitting, and any harness defect found late invalidates all of it.

**Q3: wire only reviewed backends, by family tranche — but for the right reason.** The security argument is weaker than it looks: every effect route is already mounted on the production router, so direct-POST reachability exists today regardless of wiring; and Aevatar's pin-set means no card for these verbs can even arrive until a revision is pinned upstream. What wiring actually controls is **what becomes real the instant v9 lands** — and v9 is one switch per revision. Wiring an unhardened verb creates a "wired but known-defective" ledger someone must remember at bump time; that bookkeeping has already failed once on this programme (S2 was reintroduced twice in one PR). Discipline: a verb is wired when its backend batch is review-closed, wiring lands per family, and by construction everything wired is v9-eligible. The tranches also match the serial-review reality — there is no wall-clock cost to gating.

---

## 5. Definitions of done (the test, and what it must fail against)

Every task below names its falsifier. "Passes if the feature is deleted" is an automatic review reject.

- **C1 (F2):** `account_delete_rejects_missing_or_mismatched_confirmation` — POST the effect with no/wrong confirmation → 4xx and the user still exists. Fails against HEAD (currently 200 + cascade delete). Plus: same `actionRequestId`, different confirmation → 409 (fingerprint binds the content — fails if the fingerprint stays `{action, user_id}`).
- **C2:** existing assistant vitest suite green **without edits**; plus `action_registry_covers_every_manifest_verb` — iterates a checked-in fixture of the 54 names, asserts each resolves to a dialog binding, an explicit `deferred` marker, or a legacy path. Fails when a verb is added without a row (and when a row is deleted).
- **C3 (F3):** `service_delete_cascade_grant_retry_completes` — first call refuses 11500-shaped *without* wedging the receipt; retry with same id + `cascadeGrant: true` → 200 and the service is gone. Fails against HEAD (409 fingerprint mismatch, always). The dialog test re-written against the real contract (the current one mocks the impossible 200 — delete it, don't patch it).
- **C3 (F6):** `service_delete_rejects_changed_sibling_set` — mutate dependents between confirmation and commit → sibling-changed error. Fails against HEAD (delete proceeds).
- **C3/C6 (F8, per module):** `<verb>_interrupted_retry_resumes_and_completes` — keys-pattern: force the receipt back to pending after commit, retry → 200 `replayed: true`, receipt Completed, effect applied exactly once. Fails against HEAD (409 forever).
- **C4 (F5):** `node_delete_stale_state_version_rejected` / same for transfer — bump `state_version` after fingerprinting → conflict, node survives. Fails against HEAD (delete applies). Replace the two near-vacuous tests: the fingerprint test must vary **every** semantic field per verb (table-driven), the replay-material test must go through the handlers, not the response builders.
- **C5:** per sensitive verb — `service_account_rotate_binds_fence_and_replays_without_material` (fence in fingerprint: fails against `{id}`-only HEAD; replay arm returns no material: tripwire-scan the replay body); `org_member_update_role_rejects_concurrent_role_change` (prior role bound → concurrent change conflicts; fails against HEAD) and a not-found-shaped denial test for a non-admin actor; `add_gcp_service_account_fingerprint_is_keyed` (offline-oracle test: fingerprint of low-entropy material not reproducible without the key; fails against unkeyed SHA-256); F9c: MFA resume must prove the pinned factor, not the global flag.
- **C6:** `assistant-wave4-account-approval.test.tsx` replaced with tests that assert **outgoing request bodies** and stub the now-mounted evidence routes with real shapes (the current five hardcode the unmounted-route 404 as success — delete them); F7: `profile_update_verification_accepts_multibyte_display_name` — "café"/emoji round-trip; fails against byte-`len`.
- **C7/C8:** per wired verb, one journey test: synthesize the action envelope → card renders the right dialog → dialog POSTs the effect with `actionRequestId` and the semantic fields asserted **on the request body**. Fails if the registry row, adapter, or dialog contract drifts.
- **C9:** consolidated module sweep green on a fresh dbpath, numbers recorded in §9-style; PR body statements each verifiable against HEAD.
- **C10:** the revision-map/fixture tests from the negotiation suite extended for v9; dormancy test flips to assert the 12 W2 names are in v9's set and everything else is not.

---

## 6. The 9 residue verbs — who does what

Reframed by §0: all nine exist; the work is hardening untrusted residue. Split **5 PM / 4 dispatched**, and not evenly, on purpose:

**PM (C5): `org.member_update_role`, `service_account.create`, `service_account.rotate_secret`, `developer_app.rotate_secret`, `external_key.add_gcp_service_account`.**
- The four secret journeys are exactly the class that killed both previous agents: their briefs cannot avoid credential-term density (the handlers, fields, and assertions are *about* one-time material), so re-dispatching them predictably repeats the filter attrition. Third data point available if we want it, but I don't.
- For secret-display and privilege-escalation code, PM review of an agent's diff costs roughly what writing it costs — dispatching saves nothing on the real bottleneck and adds an attrition lottery.
- `member_update_role` is the one verb where a subtle miss (last-admin demotion, org-scope confusion, missing fence) becomes privilege escalation. The backing handler (`orgs::update_member`) carries the ACL, but the wrapper must be verified not to widen it, and the fence must be added. That is judgment work, not typing work.

**Dispatch (C6): `service_account.{update,delete,revoke_tokens}`, `notifications.telegram_disconnect`.**
- No one-time material in any response (verified: `client_secret: None` on every arm), destructive-but-recoverable or plainly mechanical, backing handlers own the ACL (`require_admin_or_owning_org_admin`), and the fixes needed are the same F8/fingerprint/test patterns Stream B will have just applied to nodes. Briefs can be written entirely in terms of the conventions doc.

---

## 7. Cut or defer — recommended, with reasons

1. **Wave-3 browser journeys: defer entirely (biggest cut).** Zero dialogs exist (§0.2), so this is 8 verbs of *new* journey construction, two of which (`node.register_token`, `node.rotate_token`) are one-time-token displays with real hardening cost. Node/device operations are CLI-native — the personas driving them live in `nyxid node ...`, not the assistant. The backend effects are built and (after C4) hardened; the descriptors stay dormant at zero cost. Revisit only on demand evidence.
2. **`openclaw.connect`: defer from wiring and any pin.** Wrapper exists; niche integration; its journey overlaps the node-family problem. Dormant descriptor costs nothing.
3. **`account.delete` and `account.mfa_setup`: fix, but do not wire, and argue against ever pinning `account.delete`.** F2 gets fixed regardless (the route is live). But an irreversible cascade delete — which the audit notes orphans encrypted third-party credentials, nodes, and sole-admin orgs — has no business being an assistant chat journey; the settings page with full friction is the right surface. MFA setup's recovery-codes display belongs in settings for the same one-time-material reason. Wire neither in C8; exclude both from any revision set until someone makes the product case.
4. **v9 scope: Wave 2 only** (see §8). Do not hold the 12 hardened W2 verbs hostage to Wave-4 curation.
5. **No new `?mock=1` e2e specs.** The two existing wave2 specs are vacuous (audit 3c-3: they intercept everything including the state they later assert). Fix them to assert request bodies or delete them (FI-006/FI-007); do not multiply the pattern to W4.
6. **No exhaustive per-verb test matrices.** Right-size to: one falsifiable test per defect class per module + table-driven fingerprint coverage across verbs. Coverage padding is a rejected pattern on this programme; the DoD list in §5 is the bar, not a floor to inflate.
7. **No org-module file split.** Tempting (2,596 lines, 30 verbs), but sequencing (C5 → C6) solves the single-writer collision for free, and a mechanical split churns the PR right when review bandwidth matters most. Revisit post-merge.

---

## 8. v9 and upstream

**Recommendation: v9 = the 12 Wave-2 verbs only.** They will be the first family fully hardened + wired (C3 + C7); Aevatar needs `SupportedActions` parser work for 11 of the 12 names regardless of what we bundle, so a bigger v9 buys nothing upstream and puts unreviewed families behind one switch. Wave-4 curated becomes v10 when C8 closes.

File these upstream (PM, text drafted at C10; none block C0–C9):
1. **Adopt the `/authorization` evidence reads** — affects *shipped Wave-1 verbs in production today* (`NyxIdApiClient` reads poison-prone detail routes; a service labelled `Bearer Bot` can never confirm). Highest-value ask; independent of the waves.
2. **`?revision=` startup fetch** (one URL-builder change + 404 fallback) — makes v9 deployable without deploy choreography.
3. **`SupportedActions` extension** for the 12 W2 names + v9 pin-set entry.
4. **aevatar#3496 disposition** (superseded by `418cab838`?) — bookkeeping from the previous plan, still open.

---

## 9. Effort (calibrated: ~4k LOC/greenfield hardened verb; Sol ≈ 13 verbs/90 min *with defects*; review is serial)

| Task | Build | Calvin review |
|---|---|---|
| C0 contracts | 0.5 d PM | 0.5 h |
| C1 F2 | 0.5 d PM | 0.5 h |
| C2 harness | 1 d PM | 1.5 h |
| C3 Stream A | 2.5–3 agent-d | 2.5 h |
| C4 Stream B ph.1 | 1.5–2 agent-d | 1.5 h |
| C5 sensitive residue | 2 d PM | 1.5 h (it's PM-written; still reviewed) |
| C6 Stream B ph.2 | 2.5–3 agent-d | 2.5 h |
| C7 wire W2 | 1.5 agent-d | 1.5 h |
| C8 wire W4 | 2 agent-d | 2 h |
| C9 sweep + PR body | 0.5 d PM | 0.5 h |
| C10 v9 + asks | 0.5 d PM | 0.5 h |

Totals: **PM ≈ 5 days; agents ≈ 10–11.5 days across two capped streams ≈ 5–6 calendar days clean — call it 7–8 with the measured attrition rate; Calvin ≈ 15 hours serial review.** Honest calendar to v9-ready: **~2.5–3 weeks**, review-bound not build-bound. Upstream deploy latency for actual reachability is additive and not ours to schedule. If the §7 cuts are rejected (W3 journeys + full W4 wiring + bigger v9), add ~1.5–2 weeks.

---

## 10. What this plan deliberately does not do

No re-litigation of closed findings (F1/F4/S1/T1/T2 verified closed or fixed-since); no new e2e infrastructure; no revision bump before its family clears; no third concurrent agent, ever; no full-suite runs outside C9.
