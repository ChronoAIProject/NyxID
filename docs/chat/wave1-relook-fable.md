# Wave 1 relook (Fable, 2026-08-19) — is #1400 item 1 actually complete?

Fourth pass over ChronoAIProject/NyxID#1400 item 1. Reviewed
`origin/feat/2026-08-17_wave1-reauthorize-impl` at `4e68d699`, against
`origin/main` and Aevatar `origin/feature/integrate` at `b64c96a45`
(all Aevatar citations from `git show`, not the stale checkout).

Scope discipline: A1–A8 are known and being fixed by Opus — **nothing below
re-raises them**. This document contains only what the plan, Sol's review, and
Opus's review did not cover, plus an independent re-verification of their
claims.

---

## Verdict

**No — item 1 is not complete, and the missing piece is not in the A/B lists.**

The three prior passes audited `service.reauthorize` to a high standard and
their findings hold. What none of them did is apply that same standard to the
other two verbs item 1 covers. `key.create` and `key.rotate` shipped earlier
(v6/v7) with **the same class of defect the action list itself rates MAJOR for
reauthorize (A1)** — on a different endpoint that A1's fix does not touch.
Issue #1400 item 1 is a three-verb requirement; two of the three verbs have an
unaudited evidence surface with reachable permanent-failure paths.

Everything else I hunted for came back clean: the §7 alignment table is
accurate row by row, the B1–B4 Aevatar site list is complete (one rollout
caveat, F3), the facade G6(c) claim is verified in code, #1405/#1406 are
genuinely closed and conformant, and the edge paths I chased
(slug-form ids, deleted services, inactive services, missing credentials,
org non-admins, device-code providers) all fail closed.

---

## New findings

### F1 · MAJOR — the A1 tripwire class applies, unaudited, to `key.create` and `key.rotate` via `GET /api-keys/{id}`

**Verified in code.** Aevatar's evidence parser for both key verbs,
`ParseAgentApiKeyDocument`, runs the same recursive secret scan A1 documents —
`RejectSecretBearingRead` over the **entire** `/api-keys/{id}` body
(`src/Aevatar.AI.ToolProviders.NyxId/NyxIdApiAccessContracts.cs:445-447`;
value pattern `(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})` at
`:294-296`; endpoint `GET /api/v1/api-keys/{id}` via `GetAgentApiKeyAsync`,
`NyxIdApiClient.cs:1322-1323`; consumed by `VerifyKeyCreateAsync` /
`VerifyKeyRotateAsync`, `NyxIdActionPostconditionPort.cs:356-360, 420-424`).
A scan hit → `NyxIdContractException` → `MalformedResponse` →
`ProviderReadFailure` → the action can never verify.

NyxID's `ApiKeyResponse` (built by `enrich_api_keys_batch`, returned by
`get_key`, `backend/src/handlers/api_keys.rs:1113-1123`) carries at least four
user-controlled free-text carriers that reach that scan:

| Field | Source | `api_keys.rs` |
| --- | --- | --- |
| `name` | key name, CLI/UI free text | `:311` |
| `description` | key description, free text | model `api_key.rs:52-53`, always echoed |
| `allowed_services[].label` | **`UserEndpoint.label`** — the same user-controlled service label A1 flags on `/keys/{id}` | `:585-589` |
| `allowed_nodes[].name` | node display name | `:609-611` |

**Reachable paths, verified:**

1. **`key.rotate` inherits the tripwire.** The rotation successor clones
   `name`, `description`, `scopes`, and `platform` from the old key
   (`backend/src/services/key_service.rs`, successor construction inside the
   rotate transaction — `name: old_key.name.clone()`,
   `description: old_key.description.clone()`). A key created via CLI or UI
   with a name like `Bearer Bot`, or a description mentioning `Bearer xyz` or
   a `nyxid_…`-style example token, rotates successfully in the browser — and
   then **every** postcondition read of the successor is malformed. The card
   shows the rotation as done; Aevatar can never verify it.
2. **`key.create` is only partially protected by Aevatar's params policy.**
   `NyxIdActionSecretPolicy.ValidateStringValue` rejects only strings that
   **start with** `Bearer ` / `Basic ` (`NyxIdActionSecretPolicy.cs:134-135`),
   while the evidence regex matches `Bearer\s+\S+` **anywhere**. A name like
   `My Bearer abc key` passes action validation, the key is created, and the
   evidence read is permanently malformed.
3. **Service labels trip it regardless of the key's own fields.** A key
   scoped to any service whose endpoint label matches the pattern (the exact
   `Bearer Bot` example from A1's reproduction table) makes both `key.create`
   and `key.rotate` evidence reads malformed — even with pristine key names.

Fail mode is closed, not leaky — same as A1. But the user-facing symptom is
the same silent lie: the browser card settles (`Created` / `Rotated` badge,
the dialog paths report on factual dialog success), while Aevatar's verify
fails with a provider-read failure and the conversation treats the action as
unproven.

**Why this is new:** A1/M1 and its fix discussion are scoped entirely to
`KeyResponse` / `GET /api/v1/keys/{id}`. Sol's §6 and Opus's M1 analyze only
that response type; the Opus "did not survive" list even notes the `/keys`
*list* parser doesn't run the scan — but nobody looked at
`ParseAgentApiKeyDocument`, which does. The `assert_aevatar_secret_free`
backend test extension demanded by A1 also only covers `KeyResponse`.

**Fix.** Extend whatever A1 resolves into to this endpoint. Aevatar reads
exactly: `id`, `name`, `scopes`, `platform`, `is_active`,
`allowed_service_ids`, `allow_all_services`, `allowed_node_ids`,
`allow_all_nodes`, `created_at`, `rotation_predecessor_id`, `state_version`,
`updated_at` (`NyxIdApiAccessContracts.cs:445-465`). `description`,
`allowed_services`, `allowed_nodes`, `callback_url`, `key_prefix`, rate
limits, `credential_source` are pure tripwire surface for this read — a
minimal evidence projection (or dropping the enriched arrays from the single
`get_key`) removes the whole class except `name` itself, which is pinned by
the postcondition (`evidence.Name == params.name`) and therefore must stay;
`name` remains reachable only via `key.rotate` inheritance and the
mid-string-`Bearer` create case, which a write-time reject (or a documented
Aevatar-side policy alignment) would close. At minimum, add the backend test:
run the Aevatar-equivalent scan over a **fully populated** `ApiKeyResponse`
(description set, allowed_services with a `Bearer `-shaped label). Written
correctly it fails today — which is the point.

(Checked and safe, for the record: `key_prefix` is `nyxid_ag_` + 8 hex chars —
11 chars after `nyxid_`, below the regex's 16 minimum
(`key_service.rs:184-193`); UUID ids, timestamps and enums are clean; the
assistant key-create dialog pins `scopes` to the literal `"proxy"` the
postcondition requires, `assistant-key-create-dialog.tsx:55` +
`NyxIdActionPostconditionPort.cs:369`.)

### F2 · MINOR — legacy (pre-lineage) key rows are structurally unparseable by Aevatar's evidence parser

**Verified in code.** `ApiKeyResponse` always serializes the lineage trio
(no `skip_serializing_if`): a legacy row deserializes to
`rotation_predecessor_id: None`, `state_version: 0`, `updated_at: None`
(model defaults asserted by NyxID's own test,
`backend/src/models/api_key.rs:201-203`) and therefore serializes as
`null / 0 / null`. Aevatar's `ParseOptionalVersionEvidence` sees all three
properties **present**, so it does not take the absent-trio exit, then throws
on `state_version <= 0` (`NyxIdApiAccessContracts.cs:467-479`) — making the
whole document malformed, not merely lineage-free.

Impact today is nil: create/rotate postconditions only ever read
freshly-written rows (`state_version: 1`, `updated_at` set). But it means
`GET /api-keys/{id}` for **every key created before the #1406 lineage work**
violates the published evidence contract, and the first future consumer that
reads an existing key (a `key.update` verb, a key-state read, a re-verify)
breaks mysteriously. Fix cheaply while the file is open: serialize the trio
only when `state_version >= 1`, or emit `state_version: 1` +
`updated_at: created_at` for legacy rows at the response boundary, or fold
into the F1 projection.

### F3 · MINOR — B4 has a rollout trap the action list doesn't name: do not repoint `SupportedRegistryRevision`

The B1–B4 site list is otherwise **complete** — I read the registry load path
end to end (`NyxIdAssistantActionRegistry.cs`, whole file, plus
`NyxIdAssistantActionRegistryStartup.cs`) and found no fifth gate:
`IsSupportedRegistryRevision` derives from `PinnedActionsByRevision`
(`:288-293`, covered by B1), `TryGetDefinition` and `ValidateRequest` gate on
the executable set (covered by B2), and the producer/wire/audit/projection
sites (`NyxIdChatBrowserActions.cs:1348`, `NyxIdChatTaskPlanWireMapper.cs:527`,
`NyxIdChatConversationAguiFrameBuilder.cs:354-390`,
`NyxIdChatActionAuditTranslators.cs:344`,
`NyxIdChatConversationCurrentStateProjector.cs:1012`) all already handle
`ServiceReauthorize`.

The trap: the B4 ternary is written over **named constants** —
`revision is LeastScopeRegistryRevision or SupportedRegistryRevision`
(`:886-889`, constants `:33-34`). The natural-looking implementation of v8 —
repoint `SupportedRegistryRevision` from `"…v7"` to `"…v8"` and add a
constant for v7 — silently **removes v7 from the least-scope pin branch**
unless the old constant's value is re-added to the pattern. Result: the new
consumer rejects the **live v7 manifest** (`key.create` DeepEquals fails →
`RegistryInvalid` → registry disabled) during exactly the rollout window
step 2 exists to protect. Step 2 of the binding rollout catches this if it is
actually run against live v7; the D1 Aevatar issue text should still say
explicitly: *add a v8 constant and extend the pattern list; do not repoint
`SupportedRegistryRevision`* (also used cosmetically by `CreateDisabled()`,
`:281-286`).

### F4 · MINOR — no clock-skew allowance in any freshness window

All three verbs reject evidence timestamps even one tick ahead of Aevatar's
clock: reauthorize `LastAuthorizedAtUtc > now → Stale`
(`NyxIdActionPostconditionPort.cs:306-308`), create `CreatedAtUtc > now`
(`:381-382`), rotate additionally `UpdatedAtUtc > now` (`:460-464`). The
timestamps are stamped by NyxID's clock and compared against Aevatar's with
zero tolerance, and the verify runs immediately after the journey completes —
the worst case for a small positive NyxID skew. NTP makes this rare, and the
failure is a retryable `StaleCode`, so this is a robustness note, not a bug:
put one line in the D1 coordination text so a prod verify failure right after
a successful journey is recognized as possible skew before anyone starts
debugging the OAuth path. Tolerance, if wanted, is an Aevatar-side change.

---

## Errors in the existing four documents

I went looking for a surviving wrong claim and, for once, did not find one.
Every load-bearing claim I re-verified independently held:

- Evidence endpoint `GET /api/v1/keys/{id}` via `GetServiceAsync` —
  confirmed at `NyxIdApiClient.cs:279-280`.
- Facade G6(c) "no per-action resource gating; all six variants parse" —
  confirmed by reading `parse_action_resource` on this branch (all six
  variants, `userService` and `key` included). #1404 did modify
  `assistant_service.rs`, so the "closed by #1404" attribution is plausible
  as written.
- §7 row "Registry descriptor — Met": confirmed,
  `ASSISTANT_ACTIONS_REVISION = "nyxid-assistant-actions.v8"`
  (`assistant_actions.rs:9`), four descriptors, reauthorize schema
  structurally identical to Aevatar's `ServiceReauthorizeParamsSchema`
  (`NyxIdAssistantActionRegistry.cs:93-103`), `risk=grant`,
  `remember_eligible=false`.
- §7 row "#1405 / #1406 both closed": confirmed via `gh` (both CLOSED), and
  the #1406 lineage contract is genuinely conformant for fresh rows —
  successor writes `state_version: 1`, `updated_at == created_at` (satisfies
  Aevatar's `UpdatedAtUtc >= CreatedAtUtc` check), `rotation_predecessor_id`
  set; lineage fields present on `origin/main`.
- Param shape settled as `{userServiceId, requestedScopes[]}`: confirmed in
  Aevatar's pinned schema.
- B1–B4 line references (~207/~234/~293/~886): all match
  `origin/feature/integrate` at `b64c96a45`.
- Merge gate: PR #1462 still draft, still open.
- The one §7 statement I would amend is the conclusion sentence — "Item 1 is
  fully addressed in scope and contract." That is true of
  `service.reauthorize` only. Item 1's own text holds all three verbs to the
  same standard ("per verb"), and F1 shows the two shipped verbs do not meet
  the standard this PR is being held to. "Addressed" should read: reauthorize
  fully, key verbs pending an F1-class audit fix.

## Edge paths checked and clean (so nobody re-chases them)

- **Slug-form `userServiceId`** (model emits a slug instead of a UUID — GET
  `/keys/{slug}` resolves fine server-side): fails closed at preflight,
  `snapshot.id !== userServiceId` → typed identity block
  (`action-card.tsx`, `readReauthorizationKey`). Aevatar would also
  Ordinal-mismatch it. No false completion.
- **Service deleted before CTA**: 404 → `REAUTHORIZE_NOT_FOUND_NOTE`.
  Deleted mid-watch: the poll never reaches `active`+fresh → deadline →
  timeout note. Fail-closed, mildly generic copy, acceptable.
- **Inactive service / missing credential row / auto-connected platform
  service**: each has an explicit typed block in the preflight.
- **Org non-admin**: blocked at preflight (re-read happens at CTA click, so
  role loss between card render and CTA is caught). Role loss between CTA and
  popup completion relies on backend ACL on the initiate/callback path —
  see "could not verify".
- **Device-code providers (non-OpenAI)**: deliberately supported, and the
  path is coherent — device-code tokens store `credential_type: "oauth2"`,
  `store_device_code_tokens` stamps `last_authorized_at`, so
  `connection_status` derives `active` and freshness can pass. OpenAI-format
  device code is blocked with a scope-safety note.
- **`ParseUserServiceKeysDocument`** (the `/keys` list parser) does not run
  the secret scan — the action list's claim is accurate; the list path stays
  safe.

## Could not verify

1. **That the Aevatar human-session credential can actually read
   `/api/v1/keys/{id}` in production.** Plan Q2 resolves this by code-reading
   the delegated-GET allowlist and asserting the posture is "proven
   compatible in production" because `key.create`/`key.rotate` evidence reads
   against `/api-keys/{id}` "already work". I found no recorded end-to-end
   production run of either key verb's postcondition (the #1408 prod
   verification exercised tool parity, not action postconditions). If none
   exists, rollout step 4's single end-to-end reauthorize is the **first
   live exercise of the entire evidence-read path** — worth knowing before
   treating step 4 as a formality. A one-off prod `key.create` through chat
   before the v8 flip would retire this cheaply.
2. **Backend ACL on org-key reconnect initiate/callback** (the gate behind
   A7's "the backend remains the real gate"): `user_tokens.rs` verifiably
   enforces org-admin for `target_org_id` flows (`:299-315`); the
   `key_id`-resolution path for an org-owned service is owner-scoped but I
   did not trace it to a `can_write` check. Pre-existing manage-scopes
   surface, not introduced by this PR — but the A7 rationale leans on it, so
   someone should confirm it once.
3. **Full backend suite** — unchanged from C1; still nobody has run
   `cargo test` against a replica set for this change. (I did not attempt it
   here for the same environmental reasons as the prior two passes.)

---

## Bottom line for Calvin

`service.reauthorize` itself is in good shape modulo A1–A3, and the Aevatar
gate list is right (add the F3 sentence to the D1 issue text). But item 1 as
written is a three-verb contract, and the two verbs everyone treated as
"done, shipped earlier" have the same silent-brick class A1 was rated MAJOR
for — on `/api-keys/{id}`, which no in-flight fix touches. If A1's resolution
is a minimal evidence projection, extend the same decision to
`/api-keys/{id}` in the same PR (F1) and take F2 while the file is open;
otherwise item 1 ships with a standard applied to one verb out of three.
