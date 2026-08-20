# Assistant Support Contract — Wave Programme Plan

- **Date:** 2026-08-20 · **Author:** PM/planner session · **Branch:** `feat/assistant-waves` @ origin/main `c9776b49`
- **Contract of record:** gist `ctkm-aelf/b4dd5182…`, revision `f45febb0` (verified current head, untouched since 2026-08-06)
- **Issues:** #1400 (Wave 1 / NyxID deliverables), #1403 (Wave 2), #1401 (Wave 3), #1402 (Wave 4)

**Bottom line.** NyxID's Wave-1 side is code-complete; the deploy lock is smaller than believed (Aevatar already validates an accepted-revision *set*, and the v8 pin is already merged to `feature/integrate`); the loader provably skips unknown manifest actions, so wave descriptors can merge to main continuously without touching the revision string. The four teams should ship **the revision-negotiation endpoint plus a trimmed Wave 2 (12 of 15 verbs)** — not Waves 2+3+4. Measured cost of the last two single-verb PRs is ~4.1–4.4k added LOC each; three full waves ≈ 53 verbs is not a four-team increment.

---

## 1. Verified state table

Every claim re-verified against origin/main `c9776b49` (and Aevatar `origin/feature/integrate` / `origin/dev`, fetched 2026-08-20). Corrections are called out in bold.

| # | Claim | Verdict | Evidence |
|---|---|---|---|
| 1 | Registry = v8, 4 descriptors | **Confirmed** | `backend/src/handlers/assistant_actions.rs:9` (`nyxid-assistant-actions.v8`), descriptors at `:38-148` (`service.connect`, `service.reauthorize`, `key.create`, `key.rotate`) |
| 2 | Exact-service approvals shipped, per-request only | **Confirmed** | Routes macro `backend/src/routes.rs:229-248`, mounted `:399`/`:1406`; `backend/src/services/exact_service_approval_service.rs` = exactly 1,852 lines; grant mode → 409 `exact_service_requires_per_request_mode` at `:194` |
| 3 | Facade v4 complete (G6 closed) | **Confirmed** | `input.resolve` parsed at `backend/src/services/assistant_service.rs:848`; `approval.resolve` + `expectedStateVersion` at `:923-940`; six `ActionResource` variants at `:776-795`. The gist's G6 / obligation 3 ("not started") is stale. |
| 4 | Evidence projections + delegated admission | **Confirmed, one citation corrected** | `GET /keys/{id}/authorization` at `routes.rs:1083-1086`; `GET /api-keys/{id}/authorization` at `routes.rs:498-502`. **Correction:** `mw/auth.rs:1371-1380` is the *test fixture* (`DELEGATED_ALLOWED_MANAGEMENT_PATHS`); enforcement is deny-based — `delegated_read_denied_path` at `mw/auth.rs:393` + `delegated_request_allowed` at `:466`. New safe GET projections are admitted *by default*; only secret-delivering GETs need a deny entry (CLAUDE.md §5). |
| 5 | #1405 / #1406 closed | **Confirmed** | Both CLOSED on GitHub. |
| 6 | Per-verb cost: reauthorize ≈ 2,100 LOC / 20 files | **Corrected — undercounted ~2×** | #1462 = +2,906/−87 (16 files) + #1464 = +1,446/−299 (12 files) → **+4,352/−386 across 28 files**. #1423 (`key.rotate`) = +4,101/−494 / 34 files (confirmed). Planning rate: **~4k added LOC per hardened verb**, before reuse discounts. |
| 7 | Issue scope = §7.1 verbatim; acceptance is issue-authored | **Confirmed** | #1403's 15 verbs, #1401's 8 verbs, #1402's families match §7.1 exactly. "canary"/"canaries": **0 occurrences** in the 1,530-line gist. Intake declines on #1401/#1403 both cite exactly the production-canary + human-authorization lines. Nuance: per-verb replay/idempotency and negative fixtures *are* issue-authored but match shipped Wave-1 practice (receipt idempotency, golden-manifest rails) — keep them, re-homed to local integration tests (§2). |
| 8 | Issues drop Wave 0; G1 undecided, G2 absent, G7 not started | **Confirmed, with G1 nuance** | No issue mentions Wave 0/G1/G2/G7. `nyx__list_api_keys` / `nyx__readiness`: zero hits in the tree. G7's trigger persists: `nyx__connect_service` still takes a raw `credential` string (`backend/src/services/mcp_service.rs:1869-1891`). G1 is **de facto decided as fork (b)** — see #9. |
| 9 | Fork (b) chosen by construction, never written down | **Confirmed** | Wave 1 postconditions shipped as hardened REST projections + delegated `account:read` GET admission, consumed by Aevatar's REST reader; the MCP `nyx__*` G2 built-ins were never built. `docs/chat/06-actions-registry.md:188-216` documents the REST evidence reads as the operative contract. Recording this is a docs deliverable (§3). |
| 10 | #1464 finding: detail responses unsafe as evidence | **Confirmed from code** | Reader runs a recursive secret-shape scan (`Bearer\s+\S+`, `nyxid_` prefix) over the whole document: doc comments at `handlers/keys.rs:567-587` and `handlers/api_keys.rs:351-368`, incl. the `name` irreducible remainder. Whether *every* family needs a projection is now answered per family in §6: **yes for every Wave 2/3/4 family except (marginally) notifications.** |
| 11 | Deploy lock: Aevatar pins one string by exact equality; #3496 (v8 pin) open and blocking | **Materially corrected** | (a) Aevatar validates **set membership**, not a single string: `PinnedActionsByRevision.TryGetValue(revision, …)` with per-revision pinned + executable sets (`NyxIdAssistantActionRegistry.cs`, `feature/integrate`). `origin/dev` accepts {v4…v7}; `origin/feature/integrate` accepts {v4…v8}. (b) The v8 pin is **already merged to `feature/integrate`** as commit `418cab838` "Pin NyxID assistant registry v8 and keep service.reauthorize closed"; open PR #3496's head is *not* an ancestor of `feature/integrate` (overlapping/likely superseded content — needs an upstream disposition ask). #3497 (draft, stacked on #3496) is the step that makes reauthorize *executable*; until it lands, v8 pins reauthorize dormant. (c) "Main cannot be deployed today" is true only relative to the deployed dev-lineage composition (accepts ≤v7); the unblock is the routine `feature/integrate`→`dev` merge (last one, #3467 on 08-15, predates `418cab838`) + deploy — not #3496 per se. |
| 12 | Manifest content additive/tolerant; string is the tripwire | **Confirmed at code level — and stronger than stated** | Loader: unknown wire action, or action not in the served revision's pinned set → `continue` (skipped, not fatal); `tier`/`risk`/schema validation and `DeepEquals` run **only for pinned actions**. Consequence (load-bearing for §5/§7): **NyxID can append fully-formed new descriptors to the live manifest under the unchanged v8 string; every current Aevatar composition skips them.** Only NyxID's own test rail (`SUPPORTED_ACTIONS`, `assistant_actions.rs:181-196`) must be extended first. |
| 13 | Never modify a shipped verb's `params_schema`; browser may be stricter than manifest | **Confirmed** | `JsonNode.DeepEquals` pin + browser-only rules at `docs/chat/06-actions-registry.md:163-186`. History note: `key.create`'s schema did change v5→v6 (least-scope `minItems`), coordinated while the verb was not yet executable in a deployed composition — the rule binds for shipped-and-deployed verbs. |
| 14 | (New) Allowlist arithmetic for the waves | **New finding** | Wave 2: only 1 of 15 verbs (`provider.set_app_credentials`) is in the 14-name parser allowlist; the other 14 need both the NyxID rail and Aevatar's `SupportedActions` extended. Wave 3: 4 of 8 allowlisted (`node.register_token`, `node.rotate_token`, `node.inject_credential`, `device.onboard`); `node.delete`, `node.transfer`, `pending_credential.push/.cancel` are not — the gist's "Waves 1 and 3 draw only from it" contradicts its own Wave-3 list. Allowlist extensions are upstream-parser work and must ride the consumer wave issues. |

---

## 2. Issue hygiene — exact strikes and replacements

The intake bot declined #1401/#1403 specifically on production canaries + deployment-as-closure. Those lines are issue-authored; the gist's §7.1 DoD + §10 conformance ask for descriptors/journeys/resource refs/postconditions + utterance/drift/honesty fixtures, nothing about production. Proposed edits (paste-ready):

### #1403 (Wave 2) — replace the whole **Acceptance** section with:

> ## Acceptance
>
> NyxID-side closure requires, per verb: grammar-safe descriptor merged **dormant** under the current registry revision (no revision bump in wave PRs); browser card + human-session journey; exactly one typed safe resource reference; an authoritative postcondition read on the family's hardened evidence projection (never the full detail response); and local integration tests proving typed read-back, exact-retry receipt replay, and the negative set (secret-shaped params, stale identity, conflicting-content replay, unauthorized scope) against a local replica-set backend. Utterance→route and honesty fixtures per contract §10 land in the matching Aevatar wave issue.
>
> The registry revision bump for this wave is a separate, single-line PR gated on the cross-repo handshake (Aevatar ships acceptance of the new revision first, or the revision-negotiation endpoint is live and Aevatar fetches its pinned revision explicitly). Production rollout and verification are tracked in the handshake issue, not here; a descriptor or card mock alone is still not shipped.

### #1401 (Wave 3) — same Acceptance replacement as #1403, plus strike from **Required artifacts**:

- Strike: "atomic replay/idempotency behavior and destructive-action confirmation" → replace with: "exact-retry idempotent receipts (same identity + same content replays the committed receipt; identity reuse with different content fails closed) and destructive-action confirmation every time, proven by local integration tests".
- Strike: "authenticated production canaries must prove register/rotate/transfer/inject/onboard outcomes through typed read-back, with every disposable node, token, pending credential, and device artifact exactly cleaned up."
- Keep verbatim: "No token or code may appear in evidence." (This matches the gist secret boundary and §6's finding that `RotateTokenResponse` is secret-bearing by design — `node_admin.rs:189`.)

### #1402 (Wave 4) — same Acceptance replacement, plus:

- Strike: "authenticated production canaries per resource family" and "exact cleanup or an explicitly sanctioned irreversible test account".
- Keep: "UI prose is never completion evidence."

### #1400 — no acceptance edits; add one clarifying comment:

> NyxID side verified code-complete on main `c9776b49` (items 1–3 incl. the 2026-08-07 revision; #1405/#1406 closed). Remaining: the deploy handshake. Note the v8 pin is already on aevatar `feature/integrate` (`418cab838`); please confirm whether aevatar#3496 is superseded by it, and land the `feature/integrate`→`dev` merge + deploy before NyxID main deploys. aevatar#3497 is still required before `service.reauthorize` becomes executable (v8 pins it dormant).

Additionally, all three wave issues should gain one scope line: "Wave 0 is superseded for this wave: postconditions ride the fork-(b) REST evidence-projection pattern established by Wave 1 (see docs/chat/assistant-waves-plan.md §3); no MCP-client, `nyx__*` built-in, or planner-allowlist work is a prerequisite."

---

## 3. The Wave-0 question, answered

**Claim to prove:** nothing beyond per-family evidence projections (plus recording the fork) is required before Wave 2 can produce provable postconditions.

**Proof by the shipped Wave-1 mechanism.** A Wave-2 postcondition is: Aevatar's reader turns a browser `completed` disposition into `done/confirmed` by an exact match on an authoritative read. Wave 1 shipped four verbs whose postconditions run entirely on: (i) hardened REST projections (`/keys/{id}/authorization`, `/api-keys/{id}/authorization`); (ii) delegated `account:read` GET admission (deny-based, so safe new GETs are admitted with zero middleware change — `mw/auth.rs:393,466`); (iii) Aevatar's REST reader (`NyxIdAssistantToolSource` / `NyxIdApiClient`). None of the Wave-0 artifacts exist (G1 fork never formally decided, zero `nyx__*` G2 read built-ins, no G7 allowlist), yet Wave 1's postconditions are live and #1405/#1406 hardened them. Therefore the Wave-0 dependency in §7.1 ("every later wave's postconditions depend on this one") is empirically false under fork (b): the per-family projection *is* the Wave-0 obligation, amortized into each wave.

**What is genuinely required before Wave 2:**
1. **Per-family evidence projections** — required, per family, per the §6 audit (every Wave-2 family carries free-text tripwires). This is wave work, not a prerequisite wave.
2. **Record fork (b) as decided** — a docs/contract deliverable, zero code: one ADR paragraph in `docs/chat/06-actions-registry.md` (or a gist revision) stating: G1 resolved as (b) extended REST; G2's MCP half is dead; §6 matrix "mechanism" column re-targets to the registered REST reads + projections. Without this, Aevatar wave issues may still target `nyx__list_api_keys`-style tools that will never exist.
3. **Nothing else.** G7 (planner tool-exposure allowlist) remains a real Aevatar-side safety item for *chat-mounted read tools* and the MCP surface (`nyx__connect_service` raw-credential arg persists, `mcp_service.rs:1869-1891`), but it gates chat tool exposure, not browser-action waves: actions execute in NyxID's browser journey and verify through the projections. File it upstream as a standing safety issue; do not block Wave 2 on it.

**Caveat kept honest:** fork (b) leaves Class-P (proxy execution in chat) and the uncovered Class-R parity reads exactly as gapped as today. That is Wave-0 *product* debt, owned by Aevatar's mechanism decision — but it does not gate Wave 2/3/4 *postconditions*, which is the question asked.

---

## 4. Revision-negotiation design

**What already exists (correcting the premise).** Aevatar (`feature/integrate`) validates the fetched revision by set membership over `PinnedActionsByRevision` {v4…v8}, validates schemas only for pinned actions, skips unknown actions, and keeps a per-revision executable subset (so v8 carries `service.reauthorize` dormant). Aevatar has even minted its own composition namespace (`aevatar-nyxid-actions.v1`). **The Aevatar half of negotiation is built.** What is missing:

- **NyxID side:** the endpoint serves only the latest composition. A deployed Aevatar that does not yet know revision N+1 hard-fails startup the moment NyxID deploys N+1. This forces "Aevatar always deploys first" as fragile choreography (exactly what broke: NyxID main hit v8 on 08-19 while the deployed lineage accepts ≤v7).
- **Aevatar side (small consumer ask):** the startup fetch requests the bare path; it cannot ask for the revision it actually supports.

### Design: historical compositions on the same route

`GET /api/v1/assistant/actions?revision=<r>`:

- No param → latest body, byte-identical to today (zero risk to current consumers).
- Known `<r>` → a composition with `revision: <r>` and **that revision's action-name set, serialized with the *current* schemas/descriptions**. Not historical bytes: schemas changed v5→v6, and Aevatar's `DeepEquals` compares against its single compiled constant per action — current bytes are the only ones that can pass. Note v7's set is {connect, key.create, key.rotate}: **not a prefix** of the current array (reauthorize sits at index 1), so the implementation is a static `revision → action-name set` map mirroring Aevatar's `PinnedActionsByRevision`, with per-revision bodies pre-serialized in the existing `LazyLock` pattern.
- Unknown `<r>` → 404 with a stable error body; malformed/oversized (>128 chars, control chars) → 400.
- Rate-limit exemption is exact-path (`mw/rate_limit.rs`); query strings don't change the path — verify with a test, no change expected.
- Contract addition to `docs/chat/06-actions-registry.md`: published revisions are immutable (action set never edited), sets are monotone (each ⊆ the next), waves only append.

**Upstream consumer ask (file on aevatarAI/aevatar):** startup source appends `?revision={SupportedRegistryRevision}`; on 404 (older NyxID without the feature), fall back to the bare fetch + existing set-membership validation. One URL-builder change + fallback.

**Resulting protocol — two independent deploys, permanently:** NyxID may deploy any time (history keeps serving every pinned composition); Aevatar may deploy any time (it fetches the composition it supports). Wave cadence becomes: descriptors merge dormant continuously under the current string (safe per §1 row 12); one single-line revision-bump PR per wave, mergeable the moment either the upstream fetch change is deployed or upstream ships acceptance of the new revision.

**Immediate v8 unblock (process, not code, and not gated on this feature):** confirm the deployed Aevatar composition includes `418cab838` (needs a `feature/integrate`→`dev` merge later than #3467 + deploy), then NyxID main is deployable. `service.reauthorize` stays dormant until aevatar#3497.

### Test cases (NyxID, concrete)

| # | Name | Kind | Asserts |
|---|---|---|---|
| 1 | `assistant_actions_default_body_is_byte_identical_to_golden` | regression | bare GET == existing golden manifest, byte-for-byte |
| 2 | `assistant_actions_revision_v7_serves_exact_action_set_with_current_schemas` | unit | `?revision=…v7` → revision field `v7`, actions == {connect, key.create, key.rotate}, each `params_schema` deep-equal to the current constants |
| 3 | `assistant_actions_revision_v4_serves_service_connect_only` | unit | singleton set, current schema |
| 4 | `assistant_actions_unknown_revision_returns_404_stable_error` | negative | `?revision=…v3` → 404, error body, no manifest content |
| 5 | `assistant_actions_malformed_revision_param_returns_400` | negative | 129-char value; value with control chars |
| 6 | `assistant_actions_every_published_revision_passes_parser_contract` | fixture | run `assert_manifest_conforms` over every historical body |
| 7 | `assistant_actions_revision_sets_are_monotone_append_only` | invariant | for consecutive revisions rᵢ ⊆ rᵢ₊₁ |
| 8 | `assistant_actions_revision_sets_match_aevatar_pinned_fixture` | drift | checked-in fixture mirroring `PinnedActionsByRevision`; diff fails the build |
| 9 | `assistant_actions_route_with_query_remains_rate_limit_exempt` | integration | exemption unaffected by query string |
| 10 | `assistant_actions_dormant_descriptor_is_served_but_absent_from_all_pinned_sets` | invariant | any action not in the latest revision's set is by definition dormant; guards the wave-merge protocol |

---

## 5. Four-workstream split

### Scope decision (the numbers)

Measured: ~4.1–4.4k added LOC per hardened verb (#1423; #1462+#1464). Assume 40–60% marginal savings from established receipt/evidence/journey patterns → **1.8–2.6k LOC/verb realistic**. Wave 2 (15) ≈ 27–39k; Wave 3 (8) ≈ 14–21k; Wave 4 (~30, incl. org ACLs and destructive `account.delete`) ≈ 55k+. Four teams cannot honestly ship 53 verbs in one increment. **Recommendation: this increment = negotiation + Wave 2 trimmed to 12 verbs.** Deferred from Wave 2 with reasons: `connection.revoke` and `provider.disconnect` (they mutate the *legacy* collections — `connections.rs`, `providers.rs`; re-scope after the unified-migration decision rather than build cards over surfaces slated for deletion, FI-007), and `provider.set_app_credentials` (the one already-allowlisted verb, but a secret-bearing journey of a different shape — batch it with Wave 4's `external_key.add_gcp_service_account`). Waves 3/4 are re-planned after re-measuring Wave-2 marginal cost.

### PM pre-landed shared contracts (WS-0 — merged before any team dispatches)

1. **Registry descriptor block:** all 12 Wave-2 descriptors appended dormant to `assistant_actions.rs` under unchanged v8; `SUPPORTED_ACTIONS` rail extended with the 12 names; golden-manifest test updated. (Safe per §1 row 12; single hottest shared file leaves team ownership entirely.)
2. **Zod envelope:** `frontend/src/schemas/assistant-actions.ts` extended with all 12 param/report shapes (schema only, no journeys).
3. **Route nests:** `routes.rs` gains three one-line nests → `handlers/assistant_action_effects_keys.rs`, `_services.rs`, `_endpoints.rs`, each exporting a `router()` the owning team fills (routes.rs never touched again this increment). Confirm billing route-inventory coverage for nested routers at land time.
4. **Receipt helper:** extract/bless the durable secret-free receipt pattern from the existing key-create/rotate effects (`handlers/assistant_action_effects.rs`) into a shared service if not already one.
5. **Projection conventions doc:** one page — no free text, no `skip_serializing_if` on consumed fields, lineage-trio rule, absence-evidence (404) rule.

Ground rules for every team: never `git add -A`; stage only owned files, commit atomically; no team edits `assistant_actions.rs`, `routes.rs`, or `schemas/assistant-actions.ts` (Team 1 excepted for the first); no PR bumps `ASSISTANT_ACTIONS_REVISION`; frontend gate is `npm run build` (not `tsc --noEmit`); backend tests need replica-set Mongo + `NYXID_TEST_DATABASE_URL` (~5000 failures in ~13s = connection failure); no frontend dep/lockfile changes (wizard freshness) — none of the owned files are in the wizard graph (`wizard-entry.tsx` does not import `components/assistant/`).

### Team 1 — Revision negotiation (backend-only; no e2e)

- **Scope:** §4 in full: revision map, `?revision=` serving, tests 1–10, `docs/chat/06-actions-registry.md` negotiation + fork-(b) ADR section, drafted upstream consumer-ask issue text (PM files it).
- **Files owned:** `backend/src/handlers/assistant_actions.rs`, `docs/chat/06-actions-registry.md`, new fixture file for the Aevatar pinned-set mirror.
- **Interface contracts:** revision-set map is append-only and mirrors Aevatar; Teams 2–4 never depend on it (their verbs are dormant regardless).
- **Acceptance:** tests 1–10 green; default body byte-identical; docs updated; consumer-ask text delivered.

### Team 2 — Wave 2 keys family (4 verbs: `key.update`, `key.delete`, `key.extend_scope`, `key.bind_credential`)

- **Files owned:** `backend/src/handlers/assistant_action_effects_keys.rs` (new), `backend/src/handlers/api_keys.rs`, `backend/src/handlers/agent_bindings.rs`, `frontend/src/components/assistant/assistant-key-update-dialog.tsx` / `-key-delete-` / `-key-scope-` / `-key-bind-` (+ tests), matching hooks file.
- **Evidence:** update/extend_scope ride the existing `ApiKeyAuthorizationEvidenceResponse` (`allowed_service_ids`, lineage trio — `api_keys.rs:369-392`) — no new projection; delete = 404 on the projection route (body-free); **bind_credential needs a new binding evidence projection** (`BindingResponse` carries `service_label`/`credential_label` free text, `agent_bindings.rs:77-92`) — ids + timestamps only.
- **Interface contracts:** `extend_scope`/`bind_credential` are never `remember_eligible` (§7.1, binding); delete confirms every time; receipts via the WS-0 helper.
- **Test matrix (named):** unit `key_update_effect_reserves_receipt_and_replays_exact_retry`, `key_extend_scope_widens_allowed_service_ids_only_with_valid_ids`, `key_bind_credential_rejects_cross_owner_credential`; integration `key_update_evidence_reflects_state_version_advance`, `key_delete_evidence_read_returns_404_after_delete`, `binding_evidence_projection_contains_no_label_properties`; negative `key_update_rejects_secret_shaped_name`, `key_extend_scope_conflicting_content_replay_fails_closed`, `key_delete_stale_state_version_rejected`; fixture `wave2_key_descriptors_present_and_dormant_in_manifest`.

### Team 3 — Wave 2 services family (4 verbs: `service.update`, `service.delete`, `service.route`, `service.rotate_credential`) — **owns e2e (port 4611)**

- **Files owned:** `backend/src/handlers/assistant_action_effects_services.rs` (new), `backend/src/handlers/keys.rs`, `backend/src/handlers/user_services_handler.rs`, `frontend/src/components/assistant/assistant-service-*-dialog.tsx` (+ tests), the e2e spec directory.
- **Evidence:** `KeyAuthorizationEvidenceResponse` (`keys.rs:588-597`) lacks update/route/rotate fields; `UserService` has `updated_at` but **no `state_version`** (`models/user_service.rs:109`). Plan: additive fields (`updated_at`, `node_id`, credential-lineage trio) on the existing projection, **gated on an upstream reader-tolerance confirmation** (the reader distinguishes null from missing; additive non-free-text fields should pass its scan, but confirm before merge — fallback is a sibling projection route). Adding `state_version` to `UserService` is in scope for this team (mirrors `ApiKey`, `models/api_key.rs:43`).
- **Test matrix:** unit `service_update_effect_normalizes_and_receipts`, `service_route_sets_and_clears_node_id_atomically`, `service_rotate_credential_never_returns_credential_material`; integration `service_evidence_projection_gains_no_free_text_fields`, `service_delete_cascade_prompts_grant_confirmation` (existing `GrantCascadeConfirmationRequired` 11500 path), `service_route_evidence_shows_node_id_after_route`; negative `service_update_rejects_ws_template_in_evidence_path`, `service_rotate_stale_identity_replay_rejected`; e2e `wave2_service_update_journey_end_to_end`, `wave2_service_delete_confirm_every_time`; fixture `wave2_service_descriptors_dormant`.

### Team 4 — Wave 2 endpoints + external keys (4 verbs: `endpoint.update`, `endpoint.delete`, `external_key.rotate`, `external_key.delete`)

- **Files owned:** `backend/src/handlers/assistant_action_effects_endpoints.rs` (new), `backend/src/handlers/user_endpoints.rs`, `backend/src/handlers/user_api_keys_external.rs`, `frontend/src/components/assistant/assistant-endpoint-*-dialog.tsx`, `assistant-external-key-*-dialog.tsx` (+ tests).
- **Evidence:** both families need new projections (§6): endpoints (`label` free text, `user_endpoints.rs:106-119`) → {id, auto_connected, catalog_service_id, updated_at}; external keys (`label`, upstream `error_message`, `user_api_keys_external.rs:79-94`) → {id, credential_type, status, expires_at, last_used_at, updated_at}. Deletes = 404 evidence. `external_key.delete` must respect the existing `cascade_grant` contract (`DeleteExternalApiKeyQuery`).
- **Test matrix:** unit `endpoint_update_rejects_url_with_userinfo_or_fragment`, `external_key_rotate_reserves_successor_receipt`; integration `endpoint_evidence_projection_has_no_label`, `external_key_evidence_projection_has_no_error_message`, `external_key_delete_with_cascade_grant_confirms`; negative `endpoint_update_secret_shaped_label_rejected_as_evidence_never_served`, `external_key_rotate_conflicting_replay_fails_closed`, `endpoint_delete_stale_id_404_not_500`; fixture `wave2_endpoint_external_descriptors_dormant`.

---

## 6. Per-family evidence-read audit (from code, not assumption)

Criterion: the postcondition reader secret-scans the entire document (`Bearer\s+\S+`, `nyxid_` prefix); any user-controlled or secret-shaped string is a tripwire that can permanently block a legitimate user's verification.

| Family (wave) | Detail read struct | User-controlled / secret-shaped carriers | Verdict |
|---|---|---|---|
| API keys (W2) | `ApiKeyResponse` | `name`, `description`, `allowed_services[].label`, `allowed_nodes[].name` (doc: `api_keys.rs:351-368`) | Unsafe; **projection exists** and already covers update/extend_scope/delete evidence |
| Agent bindings (W2) | `BindingResponse` (`agent_bindings.rs:77-92`) | `service_label`, `credential_label` | Unsafe; **new projection needed** (Team 2) |
| User services (W2) | `KeyResponse` | `label`/`name`, `default_request_headers[].value`, `ws_frame_injections[].template` (doc: `keys.rs:567-587`) | Unsafe; projection exists but **needs additive fields** for update/route/rotate (Team 3) |
| External keys (W2) | `ExternalApiKeyResponse` (`user_api_keys_external.rs:79-94`) | `label`, `error_message` (echoes upstream text) | Unsafe; **new projection** (Team 4) |
| User endpoints (W2) | `EndpointResponse` (`user_endpoints.rs:106-119`) | `label`; `url` user-controlled | Unsafe; **new projection** (Team 4) |
| Connections (W2, deferred) | `ConnectionItem` (`connections.rs:62-70`) | `service_name`, `credential_label` | Unsafe; legacy surface — defer verb |
| Providers (W2, deferred) | `ProviderResponse` (`providers.rs:160-187`) | `description`, `api_key_instructions` (legitimately contains "Bearer YOUR_KEY"-style examples), `extra_auth_params` values | Unsafe; defer verb |
| Nodes (W3) | `NodeInfo` (`node_admin.rs:167-186`) | `name` (user-chosen), `NodeMetadata` (device-supplied), owner info | Unsafe; projection needed. **`RotateTokenResponse` (`:189-193`) carries raw `auth_token` + `signing_secret` by design — never evidence** |
| Pending credentials (W3) | `PendingCredentialInfo` (`node_admin.rs:234`) | `label`, `field_name` (users legitimately set it to `Authorization`), `target_url` | Unsafe; projection needed |
| Orgs (W4) | `OrgResponse` (`orgs.rs:132`) | `display_name`, `avatar_url`, `contact_email` | Unsafe; projection needed |
| Account (W4) | `UserProfileResponse` (`users.rs:40-57`) | `display_name`, `avatar_url` | Unsafe; projection needed |
| Approvals (W4) | `ApprovalGrantItem` (`approvals.rs:77-101`); `ServiceApprovalConfigItem` (`:1048-1071`) | `service_name`, `requester_label`, `org_name`; `rules` | Unsafe; projection needed |
| Notifications (W4) | `NotificationSettingsResponse` (`notifications.rs:17-26`) | mostly booleans/counters; `telegram_username` external text (could start `nyxid_`) | **Marginal** — the only near-safe family; still trim `telegram_username` |
| Service accounts (W4) | `ServiceAccountItem` (`admin_service_accounts.rs:132-146`) | `name`, `description`, **`secret_prefix` (credential-shaped by construction)** | Unsafe; projection needed |
| Developer apps (W4) | `DeveloperOAuthClientResponse` (`developer_apps.rs:128-144`) | `client_name`, `redirect_uris`, **`client_secret` field exists on the struct** | Categorically unsafe; projection needed |
| Broker bindings (W4) | `BrokerBindingListItem` | resolved `client_name` | Unsafe; projection needed |

**Conclusion:** the #1464 finding generalizes to *every* family (notifications marginal). "Per-family evidence projection" moves from open question to standing per-verb DoD line — reflected in §2's rewritten acceptance and §5's team scopes.

---

## 7. Sequencing

```
[now]      #1400 handshake (ops, no NyxID code):
           confirm deployed Aevatar ≥ 418cab838 (feature/integrate→dev merge later
           than #3467, + deploy)  ──►  NyxID main deployable (v8)
           upstream disposition ask: is #3496 superseded? #3497 still required
           before service.reauthorize is executable.

[gate 0]   WS-0 (PM): dormant descriptors + rails + zod envelope + route nests +
           receipt helper. Merges FIRST. Teams dispatch only after.

[parallel] Team 1 (negotiation)  ─ merge anytime after WS-0; MUST precede any
                                   future revision bump; file upstream fetch ask.
           Teams 2/3/4           ─ per-verb PRs merge continuously (dormant verbs,
                                   revision untouched — safe per §1 row 12).

[gate 1]   Wave-2 revision bump (v9): one single-line PR + revision-map entry.
           Merge ONLY after (a) upstream merges v9 acceptance for the 12 verbs
           (incl. SupportedActions parser extension — 11 of 12 names are outside
           the current 14), OR (b) upstream ?revision= fetch change is deployed.
           Until then v9 must not exist on main.

[always]   Do-not-merge-before rules:
           - no wave PR touches ASSISTANT_ACTIONS_REVISION, assistant_actions.rs,
             routes.rs, or schemas/assistant-actions.ts (Team 1 exception: first);
           - Team 3's projection field additions gated on upstream reader-tolerance
             confirmation (fallback: sibling route);
           - issue-hygiene edits (§2) land before any wave PR references its issue,
             so intake/labels reflect the real acceptance.
```

**Explicitly out of this increment:** Waves 3 and 4 (re-plan after Wave-2 cost re-measurement), `connection.revoke`, `provider.disconnect`, `provider.set_app_credentials` (deferral reasons in §5), G7 planner allowlist (upstream safety issue), G8 approval bridge, G9 batching.

---

## 8. Adversarial review findings (2026-08-20)

All four streams were reviewed by an independent adversarial reviewer; **all four
returned REWORK**. Every verb is dormant, so none of this is reachable in
production. Recorded here so the backlog survives the session that found it.

### Cross-repo (blocks the whole evidence design — NOT NyxID-fixable)

**Aevatar never calls the `/authorization` projections.** `NyxIdApiClient` builds
no such path: user-service evidence reads `/api/v1/keys/{id}` (`:280`) and agent-key
evidence reads `/api/v1/api-keys/{id}` (`:1323`) — the full detail routes. Confirmed
independently by two reviewers plus a direct grep of `origin/feature/integrate`.

Consequences: the #1464 hardening shipped on our side and was never adopted;
**shipped Wave-1 verbs** (`key.create`, `key.rotate`, `service.reauthorize`) verify
against poison-prone detail responses today, so a user with a service labelled
`Bearer Bot` can never confirm those actions; `docs/chat/06-actions-registry.md:188`
documents intent, not reality; and all 12 Wave-2 projections are unread until
upstream changes. Requires an upstream ask.

### Pre-existing production bugs (found reviewing dormant code)

| # | Bug | Status |
|---|---|---|
| P1 | `validate_endpoint_url` accepted userinfo + fragments on **every** scheme (`ssh://` skipped validation entirely; `validate_base_url` never checked userinfo), so `ssh://user:pass@host` and `https://user:pass@host` persisted into `UserEndpoint.url` and were served in list/detail responses | **FIXED** `6c814f5e` |
| P2 | `update_api_key` special-cases only `oauth2`; other types leave `access_token_encrypted` untouched, so rotating a GCP service-account key keeps injecting the old token for up to 5 min while evidence reports `active` | open (T4) |

### Systemic (hit all three effect modules)

| # | Finding | Status |
|---|---|---|
| S1 | `reserve_or_replay` returned `Replay` for **pending** receipts; handlers guarded only on `Completed` and fell through to mutation, so an effect that commits then fails to mark complete is applied **twice** on retry | fixed via `ReceiptOutcome::InProgress` (`5c8b08e1`/`95e2c104`), needs verification |
| S2 | Evidence routes mounted only under the nested `/assistant/actions/*` router, not the canonical production paths the dialogs call — so verification 404s after a successful mutation, and delete dialogs read that 404 as success. Tests masked it by mounting the missing route in their own test app | T2 partially, T4 open |
| S3 | Receipt fingerprints omit semantic request content, so identity-reuse-with-different-content replays instead of failing closed (rotate discards the requested credential; delete collapses cascade options) | T2 done, T3/T4 open |
| S4 | Tests do not prove their names: backend integration tests early-return without Mongo; `state_version` tests seed 1 and assert `>= 1`; e2e intercepts and synthesises every response | all streams open |

### Per-stream

- **T1 revision negotiation — FIXED (`eb3ff7a2`).** Served v5 composition used the
  *current* `key.create` schema, but Aevatar does per-revision schema selection
  (`ValidatePinnedContract`): v4/v5 pin the old schema, v6/v7/v8 the least-scope one.
  A 200 that then fails `DeepEquals` disables the whole registry at startup.
- **T2 keys** — state-version fence non-atomic (lost update); `expected_state_version`
  absent from fingerprint; binding evidence unmounted and unaddressable from the
  report; `bind_credential` accepted revoked credentials then confirmed the binding.
- **T3 services** — non-atomic multi-write (`service.update` commits the endpoint name
  then fails validation; `rotate_credential` orphans a successor and can wedge on the
  unique `(source, source_id)` index); dialogs accept stale evidence; `service.delete`
  never reads absence evidence.
- **T4 endpoints/external keys** — delete-as-absence **falsely confirms** deletion of a
  nonexistent or foreign id (retry marks the receipt completed and returns 200);
  cascade confirmation not bound to the sibling set the user saw (TOCTOU);
  update/rotate evidence proves no mutation; migrated `telegram_identity` rows 500 the
  projection; `ssh_certificate` rotation strands the replacement.

### Settled contract decisions

- `external_key.*` must not reuse the `key` resource variant (`api_keys` vs
  `user_api_keys` are different collections; the consumer resolves `key` against
  `/api/v1/api-keys/{id}`). An `endpoint` cannot be proven through an owning
  `userService` either — it may be orphaned, shared, or have no active service.
  New variants required: `endpoint.endpointId`, `externalKey.externalKeyId`.
- Upstream enum parsers are closed (`ParseCredentialStatus: _ => throw`), so any
  served enum value set is pinned contract; adding a value is breaking, not additive.
