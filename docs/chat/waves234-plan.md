# Assistant Support Contract — Waves 2, 3, 4 master plan

**Status:** planning complete, nothing implemented. Written 2026-08-19 against NyxID
`origin/main` `21297220`, the Wave-1 PR branch `origin/feat/2026-08-17_wave1-reauthorize-impl`
(`4e68d699`), and Aevatar `origin/feature/integrate` `b64c96a45` (clone at
`~/Desktop/aelf-frontend-work/aevatar`; always read with
`git show origin/feature/integrate:<path>` after `git fetch` — the checkout is stale).

**Scope:** issues ChronoAIProject/NyxID#1403 (Wave 2), #1401 (Wave 3), #1402 (Wave 4) —
58 browser-action verbs total (15 + 8 + 35). Contract of record: support-contract gist
revision `f45febb0` §7.1 (local copy:
`~/Desktop/aelf-frontend-work/docs/nyx-chat-aevatar-support-spec.md`, §6.x class table).

**How to use this document.** This file is the master plan: shared substrate, revision
strategy, deploy gates, and the work-package map. Each work package has a self-contained
brief under `docs/chat/waves234/` written for a fresh agent with no memory of this
planning pass. Where any brief disagrees with this file, this file wins.

The Wave-1 reference set (read before implementing anything):

| Document (on `origin/feat/2026-08-17_wave1-reauthorize-impl`) | Role |
| --- | --- |
| `docs/chat/wave1-service-reauthorize-actions.md` | Consolidated defect list (A1–A8), Aevatar gate (B1–B4), what did NOT survive review |
| `docs/chat/wave1-service-reauthorize-plan.md` | §1 target contract shape, §5 rollout order — the per-verb pattern to copy |
| `…-review-sol.md`, `…-review-opus.md` | Evidence for the above |

---

## 1. Verified ground truth (2026-08-19)

Everything in this section was read from the named refs during this planning pass, not
inherited from briefings.

### 1.1 NyxID `origin/main`

- `backend/src/handlers/assistant_actions.rs` (538 lines): revision
  `nyxid-assistant-actions.v7`, 3 descriptors (`service.connect`, `key.create`,
  `key.rotate`), `ASSISTANT_ACTIONS_SCHEMA_VERSION = 4`. The Wave-1 PR branch bumps to
  **v8** and adds `service.reauthorize` at index 1 (4 descriptors). All planning below
  assumes v8 ships first; if #1462 dies, Wave 2 inherits its content.
- Test module `SUPPORTED_ACTIONS` is a **test-only const** listing 14 verbs (the "14-verb
  allowlist"): `service.connect`, `service.reauthorize`, `provider.set_app_credentials`,
  `key.create`, `key.rotate`, `node.register_token`, `node.rotate_token`,
  `node.inject_credential`, `service_account.create`, `service_account.rotate_secret`,
  `developer_app.create`, `developer_app.rotate_secret`, `account.mfa_setup`,
  `device.onboard`.
- Schema grammar enforced by `validate_schema_node` (NyxID test) and Aevatar's
  `ValidateSchemaNode` (runtime, fail-closed): **closed objects
  (`additionalProperties:false` + `properties`), arrays, and strings only. No booleans,
  no integers, no enums.** Property names are checked against the forbidden-secret list
  after ASCII-alphanumeric lowercasing. Every new verb's params must be designed inside
  this grammar (§5.3).
- Frontend: `frontend/src/lib/assistant/action-registry.ts` (280 lines on the PR branch),
  `frontend/src/schemas/assistant-actions.ts` (513), journeys implemented inline in
  `frontend/src/components/assistant/blocks/action-card.tsx`.
- Route surface for every Wave-2/3/4 verb family exists on `origin/main` (verified in
  `backend/src/routes.rs`; per-verb endpoint tables live in the briefs). Notables:
  - `service_account.*` routes are nested under `/api/v1/admin/service-accounts` —
    **admin surface**, but org admins can operate on org-owned SAs via
    `target_org_id`/`org_id` without global admin (`cli/src/commands/service_account.rs`
    module doc; see §7 Q-C for the acceptance consequence).
  - `developer_app.*` is user-facing: `/api/v1/developer/apps[...]` including
    `rotate-secret`.
  - `approval enable`/`disable` (CLI) map to `PUT /api/v1/notifications/settings`
    (global `approval_required` flag), not to the approvals router;
    `approval.configure` maps to `PUT /api/v1/approvals/service-configs/{service_id}`.
  - `external-key rotate` (CLI) is `PUT /api/v1/api-keys/external/{id}` with the new
    upstream credential in the body — a secret-bearing browser journey, never chat params.
  - `account.delete` is `DELETE /api/v1/users/me` (exists), `account.profile_update` is
    `PUT /api/v1/users/me`, consent revoke is `DELETE /api/v1/users/me/consents/{client_id}`,
    broker-binding revoke is `DELETE /api/v1/users/me/broker-bindings/{binding_hash}`.

### 1.2 Aevatar `origin/feature/integrate` (`b64c96a45`)

`agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistry.cs` and
`NyxIdActionPostconditionPort.cs`, plus `protos/nyxid_chat_task.proto`:

- **The Kind enum has exactly 15 values**: the 14 allowlist verbs plus
  `service.access_review`. **No proto kind exists for any other Wave-2/3/4 verb** —
  `key.update`, `service.delete`, `org.*`, `approval.*`, `notifications.*`,
  `external_key.*`, `endpoint.*`, `connection.revoke`, `pending_credential.*`,
  `node.delete`, `node.transfer`, `account.profile_update` / `.revoke_consent` /
  `.delete`, `service_account.update/delete/revoke_tokens`, `developer_app.update/delete`
  all require proto + parser + registry + mapper + producer + postcondition work on
  Aevatar before NyxID's descriptor is anything but dead weight.
- **Typed parsers exist for all 14 allowlist verbs** and define their wire param shapes
  (verbatim from the parsers — these are settled contracts, publish exactly these):
  - `provider.set_app_credentials` → `{providerSlug}` (max 128)
  - `node.register_token` → `{name}` (max 64; lowercase letters, digits, hyphens only)
  - `node.rotate_token` → `{nodeId}` (max 256)
  - `node.inject_credential` → `{nodeId, serviceSlug}`
  - `service_account.create` → `{name, allowedScopes[]}` (scopes ≤128 items × 256 chars)
  - `service_account.rotate_secret` → `{serviceAccountId}`
  - `developer_app.create` → `{name, redirectUris[]}` (≤32 URIs, each URL-safety-normalized)
  - `developer_app.rotate_secret` → `{clientId}`
  - `account.mfa_setup` → `{}` (paramless)
  - `device.onboard` → `{label}` (max 256; SSID/password stay browser-side)
- **Postcondition readers exist for only 5 kinds** (`service.connect`,
  `service.access_review`, `service.reauthorize`, `key.create`, `key.rotate`); every
  other kind falls to the default arm: `Unverified/UnsupportedCode` — "No typed read
  model is configured for this action postcondition."
- **`IsActionExecutable`'s wire-action switch maps only 3 kinds** (connect, key.create,
  key.rotate; the PR-branch v8 work adds reauthorize). This is Wave-1 finding B3 — it
  recurs for every wave.
- **No v8/v9/v10/v11 anywhere.** `PinnedActionsByRevision` / `ExecutableActionsByRevision`
  end at v7.
- **Two load-time properties that change deploy-risk calculus (both verified in
  `Load()`):**
  1. **Descriptors for actions that are unknown or unpinned are silently skipped**
     (`Load()` `continue`s when the wire action is missing from `SupportedActions` or
     from the revision's pinned set). Only three things hard-fail the load: an unknown
     **revision string**, a pinned action **missing** from the manifest, and a pinned
     schema/risk mismatch (`ValidatePinnedContract`).
  2. **`ValidatePinnedContract` is a no-op for contracts without a pinned schema** —
     currently the 10 non-Wave-1 allowlist verbs have `PinnedParamsSchema = null`, so
     risk/remember pinning is skipped too. Until eanz17 pins them, NyxID's published
     schema is grammar-checked (fail-closed) but not byte-compared. NyxID still owns
     risk/tier/remember policy per the issues — the pin, when added, must match what
     NyxID publishes, so the schemas in the briefs are written to be pin-ready.

### 1.3 What Wave 1 actually cost, and why waves 2–4 must not repeat it

One verb (`service.reauthorize`) consumed: a 594-line plan, an implementation, two
adversarial reviews, a consolidation doc — and still carries 3 MAJOR open defects (A1
evidence-read tripwire, A2 unverified `completed`, A3 uncorrelated freshness) plus 5
minors. All three majors are **systemic**: any per-verb reimplementation inherits them.
Phase 0 (§3) turns them into shared infrastructure with single points of truth, so a
work-package brief can say "use the substrate" instead of re-deriving the discipline
60 times.

---

## 2. Verb inventory (normative for this plan)

Issue scope lists, expanded against spec §6/§7.1. Names marked ⚠ have an unresolved
definition question (§7).

**Wave 2 — #1403, 15 verbs, registry revision v9** (all *allowlist ext.* except
`provider.set_app_credentials`):
`key.update`, `key.delete`, `key.extend_scope`, `key.bind_credential`,
`external_key.rotate`, `external_key.delete`, `service.update`, `service.delete`,
`service.route`, `service.rotate_credential`, `connection.revoke`,
`provider.set_app_credentials`, `provider.disconnect`, `endpoint.update`,
`endpoint.delete`.

**Wave 3 — #1401, 8 verbs, revision v10**:
`node.register_token`, `node.rotate_token`, `node.delete` (*ext.*, destructive),
`node.transfer` (*ext.*, destructive), `node.inject_credential`,
`pending_credential.push` (*ext.* — see below), `pending_credential.cancel` (*ext.*),
`device.onboard`. `device.approve` stays excluded by contract.

> Spec inconsistency, resolved here: §7.1 says Wave 3 is "parser-allowlisted except
> where noted" but `pending_credential.push`/`.cancel` are **not** in the 14-verb
> allowlist (neither NyxID's test const nor Aevatar's `SupportedActions`). They are
> allowlist extensions with full Aevatar-side proto/parser cost. Planned as such.

**Wave 4 — #1402, 35 verbs, revision v11** (parser-allowlisted: `account.mfa_setup`,
`service_account.create`, `service_account.rotate_secret`, `developer_app.create`,
`developer_app.rotate_secret`; everything else *ext.*):

- `org.create`, `org.update`, `org.delete` (destructive), `org.join`, `org.set_primary`,
  `org.member_add`, `org.member_update`, `org.member_remove`, `org.invite_create`,
  `org.invite_cancel`, `org.role_scope_set`, `org.role_scope_clear` (12)
- `account.profile_update`, `account.revoke_consent`, `account.delete` (destructive),
  `account.mfa_setup` (4)
- `approval.configure`, `approval.enable`, `approval.disable`, `approval.revoke_grant` (4)
- `notifications.update`, `notifications.telegram_link`,
  `notifications.telegram_disconnect` (3)
- `service_account.create`, `service_account.rotate_secret`, `service_account.update`,
  `service_account.delete` (destructive), `service_account.revoke_tokens` (5)
- `developer_app.create`, `developer_app.rotate_secret`, `developer_app.update`,
  `developer_app.delete` (destructive) (4)
- `external_key.add_gcp_service_account` (1)
- `openclaw.connect` ⚠ (1) — §7 Q-D
- `broker_binding.revoke` ⚠ (1) — §6.9 of the spec assigns `oauth bindings revoke` to
  W4 but issue #1402 does not name it; carried here as a wildcard pending Calvin's call
  (§7 Q-E)

Total: 15 + 8 + 35 = 58.

---

## 3. Phase 0 — shared substrate (mandatory, blocks all waves)

Phase 0 generalizes Wave 1's three MAJOR defects (and the generalizable minors) into
infrastructure that every subsequent verb consumes. Two packages, independently
executable: **WP-0A** (backend) and **WP-0B** (frontend). Briefs:
`docs/chat/waves234/wp0a-evidence-substrate.md`, `wp0b-journey-substrate.md`.

### 3.1 S1 — Evidence projection endpoints (generalizes A1)

**Problem.** Aevatar's postcondition reads and the browser's own `assertSecretFreeReadBack`
recursively scan the entire read-back body for secret-shaped strings
(`Bearer\s+\S+|nyxid_(?:ag_)?…`) and forbidden field names. Full CRUD responses
(`KeyResponse` today; node, org, SA responses tomorrow) are tripwire surface: any
user-controlled `Bearer …` inside `ws_frame_injections[].template`,
`default_request_headers[].value`, or a label permanently bricks the verb for that row.

**Fix pattern (Phase 0 lands it once).** A dedicated assistant-evidence read surface:

```
GET /api/v1/assistant/evidence/{kind}/{id}
```

- One new handler module `backend/src/handlers/assistant_evidence.rs` with **one file
  per resource family** under `backend/src/handlers/assistant_evidence/` (`keys.rs`,
  `services.rs`, `nodes.rs`, `orgs.rs`, `account.rs`, `approvals.rs`,
  `service_accounts.rs`, `developer_apps.rs`, …). Phase 0 lands the `mod.rs` dispatcher,
  the route (one `routes.rs` edit, ever), the shared response envelope, and the `keys`
  + `services` projections as the reference implementation; wave teams add their own
  family file — **no two packages ever edit the same evidence file**.
- Each projection returns **only** the fields the verb's postcondition needs: stable
  identity, status/state facts, counts, and RFC3339 timestamps. No labels, no templates,
  no headers, no URLs with userinfo, no anything user-authored — by construction, not
  by scanning.
- ACL identical to the underlying resource read (personal/org via
  `org_service::resolve_owner_access`); unauthorized → NotFound-shaped, no metadata leak.
  Auth posture: human-session JWT and delegated `account:read` GETs (same as `/keys` —
  do **not** add these routes to `delegated_read_denied_path`; they are secret-free by
  design, which is the point).
- **Golden secret-scan test helper** (`backend/src/test_utils` addition): serialize a
  *fully populated* projection — every Option `Some`, every list non-empty, every
  user-controlled upstream field set to `Bearer ${credential}` / `nyxid_ag_x…` bait —
  and run the Aevatar tripwire regex + forbidden-name normalization over the whole JSON.
  Wave 1's A1-adjacent test failed because it scanned a near-empty response; the helper
  makes the bait mandatory.
- Aevatar consumes these endpoints for all new-verb postconditions (§5 checklist item
  4). For the three shipped Wave-1 verbs nothing changes (`GET /keys/{id}` stays their
  evidence read; migrating them is optional follow-up, not in these waves).

### 3.2 S2 — Postcondition-verified completion (generalizes A2)

**Problem.** Wave-1 journeys report `completed` on weak signals (identity + status +
timestamp) without checking the actual effect (granted scopes). Aevatar catches the lie,
the user does not — the card renders success while the conversation knows better.

**Fix pattern.** New `frontend/src/lib/assistant/journey-postcondition.ts`:

```ts
reportCompletedAfterVerify({
  read: () => fetchEvidence(kind, id),      // S1 endpoint
  predicate: (evidence) => boolean | { blocked: string },  // per-verb typed check
  resource: { … exactly one safe-resource variant … },
  report, onBlock,
})
```

- A journey **may not** call `report("completed", …)` directly; lint-enforced via
  `no-restricted-syntax` on the journey directory (same mechanism as the `useAppForm`
  rule) so the discipline survives 60 verbs and N teams.
- Test-harness rule (generalizes A5): every journey's test suite must include the
  **predicate-false case** — evidence read returns a shape whose predicate fails; assert
  the card never resolves `completed` and surfaces the block note. A journey PR without
  that case fails review; the shared harness exports a `expectNeverCompletes` helper to
  make it one-liner cheap.

### 3.3 S3 — Attempt-correlated completion (generalizes A3)

Two journey shapes exist, with different correlation strength:

- **Direct-mutation journeys** (the overwhelming majority of waves 2–4: update/delete/
  rotate/create via authenticated fetch from the card dialog). Correlation is intrinsic:
  *this* HTTP response is *this* attempt's outcome. The substrate rule: completion
  evidence = the mutation response's identity fields + one S1 evidence re-read; **no
  timestamp-advancement inference anywhere**. Rotation verbs correlate on the returned
  new identity (`rotated_to_id` / new `token_prefix` / `secret_version`), not on
  `rotated_at` advancing.
- **Out-of-band journeys** (OAuth popup, Telegram deep-link, MFA setup, device QR,
  pending-credential consumption — anything where the effect lands on a callback, not
  the card's own request): completion must correlate to a server-side attempt artifact.
  `initiateOAuthAsync` already returns an attempt-scoped `connection_id`/nonce; the
  substrate exposes `awaitOutOfBandCompletion({attemptRef, read, predicate, deadline})`
  which (a) captures a **fresh** baseline read at launch time (inside the click handler,
  closing Wave 1's stale-snapshot window), (b) polls the S1 evidence read, (c) requires
  the per-verb predicate plus attempt correlation where the server exposes it, and
  (d) reports the timeout note otherwise.
  `TODO — not investigated:` which callback paths persist the attempt nonce onto the
  written row today. WP-0B includes a half-day spike; where the server genuinely cannot
  correlate, the brief's fallback (fresh-baseline + predicate) is the documented floor,
  and the gap is listed per-verb in the brief rather than silently absorbed.

### 3.4 S4 — Journey dispatch refactor (the parallelism enabler)

`action-card.tsx` implements journeys inline today; it is the third guaranteed-conflict
file. Phase 0 refactors it into a dispatch table:

- New directory `frontend/src/components/assistant/journeys/`, one module per journey
  exporting a standard interface (`{ preflight?, Component | begin, … }` — WP-0B fixes
  the exact shape against the existing `service.connect` + reauthorize flows).
- `action-card.tsx` resolves `journey → module` through a registry map
  (`journeys/index.ts`). A journey id with no registered module renders the existing
  "unsupported" card — so the PM can land registry stubs before team journeys exist.
- After Phase 0, `action-card.tsx` and `journeys/index.ts` are **PM-owned**; teams only
  add new files under `journeys/` and their own dialogs/tests.

### 3.5 S5/S6 — folded minors

- **A6:** the divergence rule is documented once in `docs/chat/06-actions-registry.md`:
  published `params_schema` is the wire contract; the browser's Zod layer may be
  stricter only where Aevatar's *postcondition* is stricter too, and every such
  tightening is listed in that doc per verb.
- **A7:** evidence Zod schemas declare required fields **required** (`.nullable()` where
  the wire says nullable, never `.optional()` for present-by-contract properties) so
  shape drift fails loud. The S1 envelope makes this cheap: projections are small.
- **A8:** journey preflights fetch single entries (`GET /catalog/{slug}`,
  `GET /keys/{id}`, …), never whole collections. Stated once in the substrate contract;
  briefs inherit it.

---

## 4. The parallelism model — who owns which files

The three all-verbs-touch-them files, plus the files Phase 0 adds, get a **single
writer** (the PM — the orchestrating agent/person for a wave). Teams own disjoint
per-family files. Two teams never write the same path.

### 4.1 PM-owned (single writer, serialized PRs)

| File | Why |
| --- | --- |
| `backend/src/handlers/assistant_actions.rs` | descriptors, revision bump, golden tests, `SUPPORTED_ACTIONS` |
| `frontend/src/schemas/assistant-actions.ts` | Zod param schemas + `ActionCardParams` variants |
| `frontend/src/lib/assistant/action-registry.ts` | descriptors, `normalizeParams`, journey ids |
| `frontend/src/components/assistant/blocks/action-card.tsx` | post-refactor dispatch only |
| `frontend/src/components/assistant/journeys/index.ts` | journey registry map |
| `backend/src/routes.rs` | evidence route lands once in Phase 0; nothing else needed |
| `backend/src/handlers/assistant_evidence.rs` (`mod.rs` only) | family dispatch lines |
| `docs/chat/06-actions-registry.md` | revision + divergence tables |

**Per wave, the PM lands one scaffolding PR first** (see the `wpXpm-registry.md`
briefs): all the wave's descriptors + schemas + registry entries + journey-id stubs +
revision bump *kept out* (revision bump is the final, deploy-gated commit — §6). Teams
then run fully parallel.

### 4.2 Team-owned (parallel, disjoint)

Each work package owns: its `journeys/*.tsx` modules + tests, its dashboard dialog
extensions (existing per-family dialog/page files — disjoint across packages by
construction), its `assistant_evidence/<family>.rs` + tests, and its family's negative
fixtures. Exact file lists are in each brief.

### 4.3 Work packages

| WP | Wave | Verbs (count) | Owned backend files | Owned frontend files |
| --- | --- | --- | --- | --- |
| **0A** | pre | substrate (0) | `handlers/assistant_evidence/{mod,keys,services}.rs`, test-utils helper, `routes.rs` (once) | — |
| **0B** | pre | substrate (0) | — | `journeys/` refactor, `journey-postcondition.ts`, out-of-band helper, test harness; last non-PM edit of `action-card.tsx` |
| **2PM** | 2 | scaffolding v9 (15) | `assistant_actions.rs` | `assistant-actions.ts`, `action-registry.ts`, `journeys/index.ts` |
| **2A** | 2 | `key.update/.delete/.extend_scope/.bind_credential`, `external_key.rotate/.delete`, `endpoint.update/.delete` (8) | `assistant_evidence/api_keys.rs`, `assistant_evidence/external_keys.rs`, `assistant_evidence/endpoints.rs` | `journeys/key-*.tsx`, `journeys/external-key-*.tsx`, `journeys/endpoint-*.tsx` + dialogs/tests |
| **2B** | 2 | `service.update/.delete/.route/.rotate_credential`, `connection.revoke`, `provider.set_app_credentials/.disconnect` (7) | `assistant_evidence/services.rs` (extends 0A ref impl), `assistant_evidence/providers.rs` | `journeys/service-*.tsx`, `journeys/provider-*.tsx`, `journeys/connection-revoke.tsx` |
| **3PM** | 3 | scaffolding v10 (8) | same as 2PM | same as 2PM |
| **3A** | 3 | all 8 node/device verbs | `assistant_evidence/nodes.rs`, `assistant_evidence/devices.rs` | `journeys/node-*.tsx`, `journeys/pending-credential-*.tsx`, `journeys/device-onboard.tsx` |
| **4PM** | 4 | scaffolding v11 (35) | same as 2PM | same as 2PM |
| **4A** | 4 | `org.*` (12) | `assistant_evidence/orgs.rs` | `journeys/org-*.tsx` |
| **4B** | 4 | `account.*`, `approval.*`, `notifications.*` (11) | `assistant_evidence/{account,approvals,notifications}.rs` | `journeys/account-*.tsx`, `journeys/approval-*.tsx`, `journeys/notifications-*.tsx` |
| **4C** | 4 | `service_account.*`, `developer_app.*`, `external_key.add_gcp_service_account`, `openclaw.connect`⚠, `broker_binding.revoke`⚠ (12) | `assistant_evidence/{service_accounts,developer_apps,broker_bindings}.rs` | `journeys/service-account-*.tsx`, `journeys/developer-app-*.tsx`, `journeys/external-key-add-gcp.tsx`, … |

Dependency order: 0A + 0B (parallel) → 2PM → {2A ∥ 2B} → wave-2 gate; 3PM → 3A;
4PM → {4A ∥ 4B ∥ 4C}. Waves 3/4 scaffolding can start once Phase 0 merges — waves are
sequential only at the **deploy gate**, not in development (§6).

Aevatar-side prerequisites are one consumer issue per wave (drafts in each wave's PM
brief), owner eanz17 — NyxID teams do not edit the Aevatar repo.

---

## 5. The four cross-repo questions, answered

### 5.1 Q1 — Registry revision strategy: one per wave (v9, v10, v11), and why

Verified mechanics that frame the decision (§1.2): the **revision string is the only
catastrophic gate** — an unknown revision throws at `Load()` and flips the whole action
registry to `CreateDisabled()` for Aevatar processes started after the NyxID deploy.
Unknown/unpinned **actions are silently skipped**; extra descriptors are harmless.

Decision: **one revision per wave, exactly as the issues demand.** Rationale:

1. **A shared revision couples wave closure.** Each issue closes only on deployed
   revision + Aevatar consumption + canaries. One revision for 58 verbs means nothing
   closes until everything closes — the opposite of "verbs 5–64 cost less."
2. **`ExecutableActionsByRevision` timing.** Aevatar must not mark a verb executable
   before its postcondition reader exists (§1.2 — default arm = permanently
   `uncertain` tasks). Executable sets ship per Aevatar release; per-wave revisions keep
   "revision accepted" and "verbs executable" in the same reviewable unit.
3. **The bump risk is order-managed, not size-managed.** The registry-dark blast radius
   is identical for a 15-verb bump and a 58-verb bump; what removes it is the deploy
   order (§6), which we pay per wave regardless.

Cost-reducers that make three bumps cheap:

- **Aevatar may pre-land future revision map entries additively** — an unused
  `PinnedActionsByRevision["…v10"]` entry is dead config until NyxID publishes v10
  (verified: the gate only evaluates the fetched manifest's revision). If eanz17 batches
  consumer work, one Aevatar release can accept v9+v10 simultaneously; NyxID still bumps
  per wave.
- **NyxID merges are never deploy-blocked except the bump commit.** Because unpinned
  descriptors are skipped, NyxID *could* even ship descriptors early under the current
  revision without breaking Aevatar. Default posture stays the Wave-1 pattern — keep
  each wave's descriptor-set + revision bump in one final PR, everything else merges
  freely — but the skip property means an accidental early publish is a non-event, not
  an incident.
- The golden-test restructuring in 2PM (table-driven descriptors) makes each subsequent
  bump a ~50-line diff instead of a rewrite.

### 5.2 Q2 — The reusable Aevatar prerequisite checklist, and what is already built

Per wave, on `aevatarAI/aevatar` `feature/integrate`, **all seven or the registry goes
dark / the verb stays dead** (generalizing Wave-1 B1–B4):

1. `protos/nyxid_chat_task.proto`: `NyxIdAssistantActionKind` values for every verb that
   lacks one (all *allowlist ext.* verbs).
2. `NyxIdAssistantActionRegistry.cs`:
   a. `SupportedActions` entry + `ParseX` parser (secret-policy-conformant) per new verb;
   b. `PinnedActionsByRevision[vN]` — the wave's full action list **including all
      previous waves' verbs** (a pinned action absent from the manifest is
      `RegistryInvalid` → dark, so pin sets are cumulative);
   c. `ExecutableActionsByRevision[vN]` — same list, gated on (5) existing;
   d. **`IsActionExecutable` wire-action switch** — one arm per new Kind (Wave-1 B3;
      falls through to `null` = verb dead regardless of b/c);
   e. **`ValidatePinnedContract` revision handling** (Wave-1 B4) — if any pinned-schema
      special-case keys off revision constants (today: the `key.create` least-scope
      ternary), every new revision constant must be added to it; adding pinned schemas
      for new verbs must byte-match NyxID's published `params_schema` + risk +
      remember_eligible.
3. Wire mapper / TaskPlan producer / browser-action producer / audit translators /
   state projector for each new Kind (for Wave 1 these existed; for waves 2–4 they
   mostly do not — see the matrix below).
4. Postcondition reader per verb: `NyxIdActionPostconditionPort.VerifyAsync` switch arm
   + evidence parser (`NyxIdApiAccessContracts.cs`) + `NyxIdApiClient` read against the
   **S1 evidence endpoint** (§3.1) — this is the NyxID↔Aevatar coordination point each
   wave's PM brief pins down per verb.
5. `docs/contracts/nyxid-assistant-conformance/v1/registry-vN.json` fixture = the exact
   manifest NyxID will publish.
6. Registry + postcondition + producer test updates, incl. failed/uncertain fixtures.
7. Deploy, then restart-verify against the **live previous revision** before NyxID bumps.

**What Aevatar has already built (verified per §1.2):**

| Verb set | Kind + parser | Executable mapping | Postcondition | Producer |
| --- | --- | --- | --- | --- |
| Wave-1 four | ✅ | ✅ (v8 pending merge) | ✅ | ✅ |
| 10 remaining allowlist verbs (1 in W2, 3+`device.onboard` in W3, 5 in W4) | ✅ built and idle | ❌ | ❌ (default-arm unverified) | ❌ `TODO — not investigated:` producer/wire-mapper coverage for these 10 was not individually audited; treat as absent until eanz17 confirms |
| The other 44 verbs | ❌ nothing | ❌ | ❌ | ❌ |

So: **Wave 3 is the cheapest Aevatar wave** (5 of 8 verbs have parsers already), Wave 2
is 14/15 new on Aevatar, Wave 4 is 30/35 new. This asymmetry is an argument for
running Wave 3 immediately after Wave 2's NyxID work starts, if eanz17 bandwidth — not
NyxID readiness — is the binding constraint. Flagged in §7 Q-B.

### 5.3 Q3 — What extending the 14-verb allowlist actually entails

- **NyxID side: trivial, per wave.** `SUPPORTED_ACTIONS` is a test-module const in
  `assistant_actions.rs`; the extension is adding the wave's verb strings in the same PR
  that adds descriptors (PM scaffolding). The real NyxID-side constraint is the
  **schema grammar** the conformance test enforces: closed objects / arrays / strings
  only, no booleans/integers/enums, no property name whose ASCII-lowercased form hits
  the forbidden-secret list. Every brief's param design already conforms (e.g.
  `notifications.update` models toggles as `"on"/"off"` strings; `key.bind_credential`
  uses `credentialLabel` — normalizes to `credentiallabel`, which is not in the
  forbidden set, verified against both graders' exact-match logic).
- **Aevatar side: this is where "allowlist ext." is real money.** For a non-allowlisted
  verb, "extending the allowlist" means the full §5.2 checklist items 1–4 (proto kind,
  parser, registry entry, mapper/producer, postcondition) — per verb, not per wave.
  For the 10 pre-built verbs, items 1–2a exist; 2b–7 remain — per wave.
- **Cost class summary:** per-wave fixed cost (revision maps, fixture, deploy gate)
  ≈ small; per-verb Aevatar cost ≈ the dominant term for waves 2 and 4. Calvin's
  "just verb additions" framing holds **on the NyxID side** given Phase 0; it does not
  hold on the Aevatar side for the 44 unbuilt verbs, and no NyxID-side engineering can
  change that (§7 Q-B).

### 5.4 Q4 — Wave 4's acceptance bar: what is and is not satisfiable

Issue #1402 demands authenticated production canaries per resource family with exact
cleanup. Assessment per family:

| Family | Prod canary feasible? | Notes |
| --- | --- | --- |
| `org.*` | ✅ self-cleaning | create canary org → run member/invite/role-scope verbs inside it → `org.delete` last (which is itself the destructive-verb canary). Requires a second sanctioned account for member/invite verbs (invite-gated registration: mint an invite code as platform admin). |
| `approval.*`, `notifications.update` | ✅ | settings are reversible; snapshot-and-restore in the canary script. |
| `notifications.telegram_link` | ⚠ partial | requires a real Telegram account interaction mid-journey. Propose: prod canary covers `notifications.update` + `telegram_disconnect` (idempotent on unlinked accounts returns clean error fixture); `telegram_link` is proven by one **manual** authenticated prod pass, recorded with evidence, per deploy — not per-CI-run. |
| `account.profile_update`, `account.revoke_consent` | ✅ | reversible on a canary account (consent canary needs one OAuth client consent seeded first — the SDK demo app or a canary developer app works). |
| `account.mfa_setup` | ✅ on canary account | setup is idempotent against unverified factors (NyxID#506); cleanup = disable MFA with the canary's TOTP. Never on a human account. |
| `account.delete` | ⚠ only with a sanctioned disposable account | the issue itself allows "an explicitly sanctioned irreversible test account". One per canary run (invite-code registration + email auto-verify is dev-only, so prod canary needs a real mailbox or a plus-address on a controlled domain). **Additional design problem:** deletion terminates the session that must report completion, and Aevatar's postcondition read with that user's credential can only observe 401/absence. The brief specifies: journey reports `completed` on the DELETE 200 **before** session teardown; the postcondition contract for this one verb is inverted (evidence read must *fail* with auth-gone/not-found). Needs eanz17 sign-off — §7 Q-F. |
| `service_account.*` | ⚠ org-admin only | routes live under `/admin/service-accounts` but org admins can manage org-owned SAs via `target_org_id`. Canary = org-admin session in the canary org, creating/rotating/deleting an org-owned SA. A *personal* (non-org) SA journey is platform-admin-only — the descriptor description + preflight must say so, and the canary uses the org path. |
| `developer_app.*`, `external_key.*`, `endpoint.*`, `key.*`, `service.*`, `connection.revoke`, `provider.*` | ✅ | all user-facing CRUD with exact-cleanup deletes. |
| `openclaw.connect` | ❌ as currently defined | undefined verb (§7 Q-D); if it means the node-local OpenClaw flow, a prod canary requires a live node + local OpenClaw instance — propose staging-node canary or descope. |
| Wave 3 nodes/devices | ⚠ | `node.register_token`/`rotate_token`/`delete`/`transfer`/`pending_credential.*` canaries need a disposable live node agent; feasible with a CI-managed `nyxid node` profile against prod, but it is real infrastructure — the Wave-3 brief budgets it explicitly rather than pretending a fixture is a canary. `device.onboard` canary can assert QR issuance + registered stub + exact cleanup without a physical device. |

**Bottom line:** the bar is satisfiable for ~90% of Wave 4 with (a) one standing canary
org, (b) one sanctioned disposable-account procedure, (c) two explicitly-manual
exceptions (`telegram_link`, and `account.delete`'s inverted postcondition). The plan
does not quietly assume the impossible: the three ⚠/❌ rows above are surfaced as
decisions (§7), not buried in briefs.

---

## 6. Deploy gates and rollout order (per wave, binding)

Copied from the Wave-1 binding order, generalized — this is the same gate every wave:

1. **NyxID Phase 0 + wave team PRs merge freely** (no revision bump inside them).
2. **Aevatar consumer for revision vN merges + deploys** (checklist §5.2 items 1–6).
3. Restart an Aevatar process against the **live previous revision**; prove the registry
   loads and all previously-shipped actions still work (additive-acceptance proof).
4. **NyxID merges + deploys the vN bump PR** (descriptors + `SUPPORTED_ACTIONS` +
   golden updates in one commit).
5. Restart an Aevatar process; prove vN loads and the wave's verbs are executable.
6. Run the wave's canary protocol (§5.4); post evidence on the wave issue; close.

Rollup/branch CI caveat (standing): `ci.yml` gates `main`/`dev` only — run the full
local suite on every PR regardless of green checks. Test commands are in every brief;
backend tests need the MongoDB replica set + `NYXID_TEST_DATABASE_URL` (thousands of
failures in seconds = connection failure, not regressions).

---

## 7. Open questions for Calvin (decisions needed, none block Phase 0)

- **Q-A (revision numbering):** plan assumes Wave-1 v8 (#1462) ships before Wave 2. If
  #1462 stalls past Wave-2 readiness, does Wave 2 absorb `service.reauthorize` into v9?
  (Mechanically trivial; contractually it re-opens #1400's closure story.)
- **Q-B (Aevatar capacity + wave order):** waves 2 and 4 are dominated by Aevatar-side
  per-verb work (44 verbs with zero existing machinery — §5.2 matrix). Confirm eanz17
  owns that on the Aevatar side and whether Wave 3 (5/8 verbs pre-built there) should
  jump the queue. NyxID-side plans are order-independent after Phase 0.
- **Q-C (service accounts):** accept the org-admin-scoped journey as the Wave-4 shape
  (personal SAs stay platform-admin/dashboard-only)? Alternative — building a user-facing
  SA surface — is a product change this plan does not smuggle in.
- **Q-D (`openclaw.connect`):** the catalog `llm-openclaw` connect already ships via
  `service.connect`, and `nyxid node openclaw connect` is node-local (Class L). What is
  this verb supposed to do? Options: (a) descope from Wave 4; (b) define as the
  `/integrations/openclaw/mappings` channel-mapping journey; (c) navigation-only card.
  Recommendation: (a) descope until a concrete user story exists.
- **Q-E (`broker_binding.revoke`):** spec §6.9 says W4, issue #1402 omits it. Include
  (12th 4C verb, cheap — routes exist) or defer? Recommendation: include; flag in the
  issue comment when the wave opens.
- **Q-F (`account.delete` postcondition inversion):** sign off (with eanz17) on the
  §5.4 design — completion on DELETE 200 + evidence-read-must-fail postcondition — or
  downgrade `account.delete` to a navigation-only card. Recommendation: keep the verb,
  it is the honest test of the destructive-confirm rail, but it needs the inverted
  contract agreed cross-repo before 4B builds it.
- **Q-G (`key.extend_scope` vs `key.update`):** `PUT /api-keys/{key_id}` handles scope
  changes; `extend_scope` as a *separate verb* exists because widening is
  never-remember-eligible while rename/rate-limit edits are benign. Confirm the split:
  `key.update` = non-authority fields only (name, rate limits, platform);
  `key.extend_scope` = `allowed_service_ids`/`allow_all_*` widening only, with the
  journey refusing mixed edits. (Briefs are written to this split.)

---

## 8. Work-package brief index

| Brief | Verbs |
| --- | --- |
| `waves234/wp0a-evidence-substrate.md` | — (backend substrate) |
| `waves234/wp0b-journey-substrate.md` | — (frontend substrate) |
| `waves234/wp2pm-registry.md` | Wave-2 scaffolding + v9 bump + Aevatar issue draft |
| `waves234/wp2a-keys-endpoints.md` | 8 |
| `waves234/wp2b-services-providers.md` | 7 |
| `waves234/wp3pm-registry.md` | Wave-3 scaffolding + v10 bump + Aevatar issue draft |
| `waves234/wp3a-nodes-devices.md` | 8 |
| `waves234/wp4pm-registry.md` | Wave-4 scaffolding + v11 bump + Aevatar issue draft |
| `waves234/wp4a-orgs.md` | 12 |
| `waves234/wp4b-account-approvals-notifications.md` | 11 |
| `waves234/wp4c-service-accounts-developer-apps.md` | 12 |

Every brief is self-contained: exact verbs, owned files, endpoint tables, param schemas
(grammar-conformant, pin-ready), journey pattern, postcondition predicate, negative
fixtures, acceptance criteria, and test commands. Briefs assume Phase 0 and the wave's
PM scaffolding are merged; each brief opens with its own "verify before building" list
because worktrees here go stale.
