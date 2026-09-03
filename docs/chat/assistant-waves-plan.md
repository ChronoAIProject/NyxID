# Assistant Support Contract — Wave Programme Plan

- **Date:** 2026-08-20 · **Author:** PM/planner session · **Branch:** `feat/assistant-waves` @ origin/main `c9776b49`
- **Contract of record:** gist `ctkm-aelf/b4dd5182…`, revision `f45febb0` (verified current head, untouched since 2026-08-06)
- **Issues:** #1400 (Wave 1 / NyxID deliverables), #1403 (Wave 2), #1401 (Wave 3), #1402 (Wave 4)

**Bottom line.** NyxID's Wave-1 side is code-complete. Aevatar's current loader
treats `schema_version` as the only registry-wide gate, treats `revision` as an
observability label, and degrades unknown or divergent descriptors per action.
Startup retries three times, then pins a disabled fallback and recovers in the
background. Wave descriptors can merge to the default manifest without a
revision bump. `plan.resolve` and `POST /api/v1/assistant/completions` are
retired NyxID surfaces, not current contract. The four teams should ship a
trimmed Wave 2 (12 of 15 verbs), not Waves 2+3+4. Measured cost of the last two
single-verb PRs is about 4.1k to 4.4k added LOC each. Three full waves total
about 53 verbs, which is not a four-team increment.

---

## 1. Verified state table

The original state table records sources fetched on 2026-08-20. AC-1 registry
corrections were re-verified against Aevatar `feature/integrate` at
`e5bba2e9719ad5132004b882744caa3875db1123` on 2026-09-03. Corrections are
called out in bold.

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
| 11 | Deploy lock: Aevatar pins one string by exact equality; #3496 (v8 pin) open and blocking | **Superseded by AC-0 and AC-1** | Current Aevatar `Load` does not fail the registry on revision mismatch. `schema_version` is the only registry-wide gate. `revision` is an observability label. Startup retries, then installs a disabled fallback and recovers later. |
| 12 | Manifest content additive/tolerant; string is the tripwire | **Confirmed, and stronger** | Unknown wire actions are skipped. Known actions whose descriptors diverge are skipped per action and recorded. NyxID can append fully-formed new descriptors to the live manifest under the unchanged v8 string. |
| 13 | Never modify a shipped verb's `params_schema`; browser may be stricter than manifest | **Confirmed** | `JsonNode.DeepEquals` pin + browser-only rules at `docs/chat/06-actions-registry.md:163-186`. History note: `key.create`'s schema did change v5→v6 (least-scope `minItems`), coordinated while the verb was not yet executable in a deployed composition — the rule binds for shipped-and-deployed verbs. |
| 14 | (New) Allowlist arithmetic for the waves | **New finding** | Wave 2: only 1 of 15 verbs (`provider.set_app_credentials`) is in the 14-name parser allowlist; the other 14 need both the NyxID rail and Aevatar's `SupportedActions` extended. Wave 3: 4 of 8 allowlisted (`node.register_token`, `node.rotate_token`, `node.inject_credential`, `device.onboard`); `node.delete`, `node.transfer`, `pending_credential.push/.cancel` are not — the gist's "Waves 1 and 3 draw only from it" contradicts its own Wave-3 list. Allowlist extensions are upstream-parser work and must ride the consumer wave issues. |

---

## 2. Issue hygiene — exact strikes and replacements

The intake bot declined #1401/#1403 specifically on production canaries + deployment-as-closure. Those lines are issue-authored; the gist's §7.1 DoD + §10 conformance ask for descriptors/journeys/resource refs/postconditions + utterance/drift/honesty fixtures, nothing about production. Proposed edits (paste-ready):

### #1403 (Wave 2) — replace the whole **Acceptance** section with:

> ## Acceptance
>
> Each verb needs a grammar-safe descriptor published additively under the current registry revision, a browser card and human-session journey, and one typed safe resource reference. Each verb also needs an authoritative postcondition read on the family's hardened evidence projection, never the full detail response. Local replica-set integration tests must prove typed read-back, exact-retry receipt replay, and the negative cases. Those cases cover secret-shaped params, stale identity, conflicting-content replay, and unauthorized scope. Utterance-to-route and honesty fixtures per contract §10 land in the matching Aevatar wave issue.
>
> Aevatar skips unknown descriptors and degrades divergent known descriptors per action. No registry revision bump or cross-repository revision handshake gates an additive descriptor. Production rollout and verification remain separate work. A descriptor or card mock alone is still not shipped.

### #1401 (Wave 3) — same Acceptance replacement as #1403, plus strike from **Required artifacts**:

- Strike: "atomic replay/idempotency behavior and destructive-action confirmation" → replace with: "exact-retry idempotent receipts (same identity + same content replays the committed receipt; identity reuse with different content fails closed) and destructive-action confirmation every time, proven by local integration tests".
- Strike: "authenticated production canaries must prove register/rotate/transfer/inject/onboard outcomes through typed read-back, with every disposable node, token, pending credential, and device artifact exactly cleaned up."
- Keep verbatim: "No token or code may appear in evidence." (This matches the gist secret boundary and §6's finding that `RotateTokenResponse` is secret-bearing by design — `node_admin.rs:189`.)

### #1402 (Wave 4) — same Acceptance replacement, plus:

- Strike: "authenticated production canaries per resource family" and "exact cleanup or an explicitly sanctioned irreversible test account".
- Keep: "UI prose is never completion evidence."

### #1400 — no acceptance edits; add one clarifying comment:

> NyxID side verified code-complete on main `c9776b49` for items 1 through 3, including the 2026-08-07 revision. #1405 and #1406 are closed. The current Aevatar loader does not gate on the revision label. Track deployment proof separately. `service.reauthorize` remains non-executable until its typed producer and postcondition path land upstream.

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

## 4. Registry compatibility contract

Aevatar fetches the bare `GET /api/v1/assistant/actions` path.
`schema_version` is the only registry-wide compatibility gate. `revision` is
an observability label, so a future or missing value does not reject the
registry. The loader ignores unknown action names. It records and skips a known
descriptor when its shape, policy, or pinned schema diverges. Every other valid
descriptor remains loaded.

The startup service tries the fetch three times. If all attempts fail, it pins
a disabled fallback registry and retries in the background with capped
exponential delay. The first valid response may replace only that fallback. A
served registry remains pinned for the process lifetime.

NyxID still supports `GET /api/v1/assistant/actions?revision=<r>` for its own
historical compositions. Aevatar does not use that query as a negotiation or
load gate. New descriptors may ship additively under the current revision.
They become executable only after Aevatar has the matching contract, producer,
wire mapper, and postcondition reader.

### Test cases

| Name | Asserts |
|---|---|
| `default_manifest_action_names_match_consumer_fixture_golden` | The additive default manifest keeps every descriptor name. |
| `default_manifest_matches_aevatar_chat_contract_pin` | The manifest uses the pinned schema version, revision semantics, and per-action degrade behavior. |
| `known_descriptors_are_accepted_by_the_per_action_consumer` | Every descriptor name supported at the pinned Aevatar head loads from the default manifest. |
| `unknown_descriptors_are_skipped_without_disabling_known_actions` | An additive unknown name does not change the accepted known set. |
| `divergent_descriptors_degrade_per_action` | A divergent known descriptor is skipped while sibling actions remain loaded. |
| `schema_version_gates_the_registry` | A schema mismatch rejects the registry, while a future revision label does not. |

Earlier revisions of this plan proposed a synchronized revision map and a
consumer `?revision=` request. AC-1 removed those assumptions because a checked-in
copy could not detect upstream drift and the pinned consumer no longer uses
revision equality.

---

## 5. Four-workstream split

### Scope decision (the numbers)

The last two hardened verbs added about 4.1k to 4.4k lines each in #1423,
#1462, and #1464. A 40% to 60% marginal saving from the established receipt,
evidence, and journey patterns puts each later verb at about 1.8k to 2.6k
lines. Wave 2 has 15 verbs, Wave 3 has 8, and Wave 4 has about 30. Four teams
cannot ship all 53 verbs in one increment. This increment should contain the
registry contract correction and 12 Wave-2 verbs. Defer `connection.revoke`
and `provider.disconnect` because they mutate the legacy collections in
`connections.rs` and `providers.rs`. Revisit them after the unified migration
decision. Defer the secret-bearing `provider.set_app_credentials` journey with
Wave 4's `external_key.add_gcp_service_account`. Re-plan Waves 3 and 4 after
measuring the Wave-2 cost.

### PM pre-landed shared contracts (WS-0 — merged before any team dispatches)

1. **Registry descriptor block:** all 12 Wave-2 descriptors appended additively to `assistant_actions.rs` under unchanged v8; `SUPPORTED_ACTIONS` rail extended with the 12 names; golden-manifest test updated. Current Aevatar builds skip unsupported names per action. The single shared file stays under one team.
2. **Zod envelope:** `frontend/src/schemas/assistant-actions.ts` extended with all 12 param/report shapes (schema only, no journeys).
3. **Route nests:** `routes.rs` gains three one-line nests → `handlers/assistant_action_effects_keys.rs`, `_services.rs`, `_endpoints.rs`, each exporting a `router()` the owning team fills (routes.rs never touched again this increment). Confirm billing route-inventory coverage for nested routers at land time.
4. **Receipt helper:** extract/bless the durable secret-free receipt pattern from the existing key-create/rotate effects (`handlers/assistant_action_effects.rs`) into a shared service if not already one.
5. **Projection conventions doc:** one page — no free text, no `skip_serializing_if` on consumed fields, lineage-trio rule, absence-evidence (404) rule.

Ground rules for every team: never `git add -A`; stage only owned files, commit atomically; no team edits `assistant_actions.rs`, `routes.rs`, or `schemas/assistant-actions.ts` (Team 1 excepted for the first); no PR bumps `ASSISTANT_ACTIONS_REVISION`; frontend gate is `npm run build` (not `tsc --noEmit`); backend tests need replica-set Mongo + `NYXID_TEST_DATABASE_URL` (~5000 failures in ~13s = connection failure); no frontend dep/lockfile changes (wizard freshness) — none of the owned files are in the wizard graph (`wizard-entry.tsx` does not import `components/assistant/`).

### Team 1. Registry contract correction

- **Scope.** Preserve NyxID's historical composition query, delete synchronized revision-map assertions, add a default-manifest golden, and test known, unknown, and divergent descriptors independently.
- **Files owned.** `backend/src/handlers/assistant_actions.rs`, `docs/chat/06-actions-registry.md`, and `tests/fixtures/assistant/aevatar-pinned-actions-by-revision.json`.
- **Interface contracts.** `schema_version` gates the registry. `revision` is an observability label. Unknown or divergent descriptors degrade per action.
- **Acceptance.** The default manifest keeps every additive descriptor, per-action tests pass, and the docs match the pinned Aevatar source.

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
[now]      AC-0 pins the Aevatar source head, public commands, and registry
           semantics used by every later phase.

[gate 0]   AC-1 removes obsolete commands and revision-map assumptions. The
           default-manifest golden and per-action tests preserve additive load.

[gate 1]   WS-0 (PM): additive descriptors + rails + zod envelope + route nests +
           receipt helper. Merges FIRST. Teams dispatch only after.

[parallel] Teams 2/3/4           per-verb PRs merge continuously. Aevatar skips
                                  unknown names until its typed support lands.

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

- **T1 revision negotiation.** The earlier exact-revision design was superseded
  by AC-1. Aevatar now uses `DeepEquals` only to decide whether one known
  descriptor loads. A mismatch skips that action and leaves sibling actions
  enabled.
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

---

## 9. Verification record (2026-08-20, PM-executed)

Every agent on this programme reported green gates it had not run. These results
were executed directly and are the only trustworthy numbers in this document.

### Why the full suite could not produce a number

Three full-suite attempts all failed for environmental reasons, not regressions:

1. First run (5301 passed / 127 failed, 697s) — output piped through `tail`, so the
   failure list was discarded. Partially contended.
2. Second run (4110 / 1318, 92s) — a concurrent agent and the PM both ran the suite
   against the same replica set; mongod was restarted mid-run
   (`InterruptedAtShutdown`). Both results void.
3. Third and fourth runs (4115 / 1317, 699s; 4760 / 672, 810s) — **mongod crashed**
   (`terminate() called` in the ftdc thread). Root cause: every test creates its own
   `nyxid_test_*` database with index builds; the dbpath had accumulated **33,177
   files** across the day's runs. 666-1305 of the "failures" were a single message:
   `NYXID_TEST_DATABASE_URL is configured but MongoDB is not reachable and writable;
   refusing to fall back to a different test database` — the harness correctly
   refusing to silently pass against the wrong database.

**Operational note for anyone repeating this:** wipe the dbpath and restart the
replica set before a full run, and do not run the suite concurrently with an agent
that also runs tests. Full-suite parallelism (5,432 tests) exhausts a local mongod.

### What was verified, per module, on a fresh replica set

| Group | Result |
|---|---|
| `assistant_action_effects_{keys,services,endpoints}`, `assistant_action_receipts`, `assistant_actions`, `assistant_service`, `agent_binding*` | **121 passed, 0 failed** |
| `key_service`, `user_api_key_service`, `user_endpoint_service`, `user_service_service`, `unified_key_service` | **697 passed, 0 failed** |
| `user_api_keys_external`, `user_endpoints`, `user_services_handler`, `handlers::keys`, `handlers::api_keys` | **160 passed, 0 failed** |
| `state_version` ripple: `auth_device`, `catalog_service`, `connection_expiry`, `credential_push`, `device_code_service`, `proxy_service`, `handlers::oauth` | **326 passed, 0 failed** |
| **Backend total across every module this branch touches** | **1,304 passed, 0 failed** |

| Other gate | Result |
|---|---|
| `cargo fmt --all -- --check` | PASS |
| `cargo check -p nyxid` | PASS, **0 warnings** (CI uses `-Dwarnings`) |
| `cargo check -p nyxid --tests` | PASS |
| Frontend `npm run build` (the real CI gate) | **exit 0** |
| Frontend vitest, full assistant surface | **782 passed**, 62 files |
| Schema vitest incl. 3 new resource-variant tests | **25 passed** |

### Still not verified

- **T2's six findings.** Its verification agent died twice before reporting. The code
  changes are present and its module tests pass, but no one has confirmed each
  finding is closed by a test that would fail if it were not.
- **A single clean full-suite run.** Not achieved; the per-module sweep above is the
  substitute. Modules outside this branch's surface were not re-run.
