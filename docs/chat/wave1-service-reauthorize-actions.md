# Wave 1 `service.reauthorize` — consolidated action list

**This is the working document for the wave-1 `service.reauthorize` work**,
originally PR #1462 (now merged — see the note in §1). It is the single place
that says what is done, what is outstanding, and who owns each item. The four
documents it consolidates are evidence, not instructions:

| Document | Author | Role now |
| --- | --- | --- |
| `wave1-service-reauthorize-plan.md` | planner | Contract reference (§1 target contract, §5 rollout). Task breakdown is **superseded by this file**. |
| `wave1-service-reauthorize-review-sol.md` | implementer's pre-build review | Evidence for the plan corrections that shaped the build. |
| `wave1-service-reauthorize-review-opus.md` | independent adversarial review | Evidence for every A-item below; full reproductions live there. |
| `wave1-relook-fable.md` (branch `review/2026-08-19_wave1-relook`, `d6c57a00`) | fourth-pass relook | Evidence for F1-F4. Found no errors in the other three documents. |

Where any source document disagrees with this file, this file wins. Where a
finding was overturned by a later pass, it is recorded in §5 so it is not
re-litigated.

Scope: ChronoAIProject/NyxID#1400 **item 1** only. Items 2 and 3 of that issue are
out of scope (item 3 shipped separately in #1404).

---

## 1. Status at a glance

| | Count | Status |
| --- | --- | --- |
| **A. NyxID actions** | 3 major, 5 minor | A2-A8 closed; A1 partly closed (see below) |
| **B. Aevatar prerequisites** | 4 code sites + 2 evidence reads | **outstanding — hard gate** |
| **C. Verification gaps** | 2 | C1 closed; C2 open |
| **D. Coordination (Calvin)** | 2 posts | not posted |
| **F. Fourth-pass findings** | 1 major, 3 minor | F1 partly closed, F2 closed, F3/F4 recorded |

> **PR #1462 was merged into `main` at 2026-08-19T04:08:46Z** (merge commit
> `67c9bc1f`, by `ctkm-aelf`), against the DO-NOT-MERGE instruction that was in
> force. `main` now publishes `nyxid-assistant-actions.v8` while the Aevatar
> consumer is still pinned at v7 with no `registry-v8.json`. If `main` is
> deployed, every NyxID action card goes dark on Aevatar processes started
> afterwards — chat itself survives. Whether it has been deployed is
> **unverified**; it could not be checked from the implementation environment.
> The rollout order in §3 is now out of sequence and needs Calvin's decision.

The fixes below therefore live on `feat/2026-08-17_wave1-reauthorize-impl` past
the merge point, with no PR tracking them yet.

---

## 2. Section A — NyxID actions

Severity, owner, and status per item. File references are on this branch unless
marked otherwise.

### A1 · MAJOR · partly closed — user-controlled `Bearer …` strings break the evidence read

**Status.** The NyxID side that can be closed unilaterally is closed. The half
that requires Aevatar is not, and cannot be.

**What was wrong.** Two recursive scanners run over the whole `/keys/{id}`
body — Aevatar's `RejectSecretBearingRead` and the browser's
`assertSecretFreeReadBack` — both matching
`(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})`. `KeyResponse` carries at
least three user-controlled free-text carriers that reach them:
`ws_frame_injections[].template` (NyxID's own validator explicitly permits
`{"headers":{"Authorization":"Bearer ${credential}"}}`),
`default_request_headers[].value`, and `label` / `name`. NyxID stores a value
it then refuses to read back.

**Why the two fixes the reviews proposed are both unavailable as written.**

- *Drop the carriers from `/keys/{id}`* — not possible. They have live
  consumers: `frontend/src/pages/key-detail.tsx`, `schemas/keys.ts`,
  `pages/service-detail.tsx`, `service-edit.tsx`, `cli/src/commands/service.rs`,
  and the route is a documented public API (`backend/src/api_docs.rs`).
  Dropping them is a breaking change to a published contract.
- *Sanitize the values in place* — not possible either, and worse than it
  looks. `key-detail.tsx:2635-2641` seeds the WS-template editor directly from
  this response and `:2081` PUTs the edited value back, so a redacted template
  would be written over the user's real configuration the next time they touch
  that section. Same round trip for `default_request_headers`. This destroys
  data rather than protecting it.
- *Reject at write time* — does not repair rows already stored, and would
  reject a feature CLAUDE.md §6 documents as supported.

**What landed.** `GET /api/v1/keys/{id}/authorization`: same resolution, ACL and
lazy `pending_auth` reconciliation as the detail read, projected to exactly the
seven properties the evidence reader consumes (`id`, `api_key_id`, `is_active`,
`status`, `connection_status`, `granted_scopes`, `last_authorized_at`). The
detail response is untouched. Tests assert both directions — that a fully
populated `KeyResponse` carrying a `Bearer ${credential}` WS template, a
`Bearer …` header value and a `Bearer Bot` label *does* trip the scan, and that
the projection does not — so the projection cannot quietly stop being
load-bearing.

The browser half is fully closed: the eligibility preflight no longer scans the
detail response (it reports nothing to the assistant), the assertion moved to
the evidence read, and a scan hit there now produces a specific note instead of
the generic "could not verify this service".

**What remains open, and why it cannot be closed here.** Aevatar hardcodes
`GET /api/v1/keys/{Uri.EscapeDataString(id)}` in `NyxIdApiClient.GetServiceAsync`
and sends no header or query parameter NyxID could content-negotiate on. The
same client method also serves `NyxIdServicesTool`, `NyxIdSshCommandExecutor`
and `NyxIdAssistantToolSource`, which legitimately need the full document — so
NyxID cannot narrow that response by caller either. Until Aevatar points its
evidence read at the projection (**B5**), a service configured with any of the
three carriers stays unverifiable on the Aevatar side.

### A2 · MAJOR · closed — `completed` was reported without checking scopes

Both completion paths now read `/keys/{id}/authorization` and require
`requested_scopes` to be an ordinal subset of `granted_scopes` before
`report("completed", …)`, matching Aevatar's `StringComparer.Ordinal`. On a
shortfall the card calls `onBlock` naming the missing scopes; an unreadable or
secret-bearing evidence response blocks with its own distinct note rather than
settling. Covered by three tests in `action-card.test.tsx` (shortfall, read
failure, and the unchanged happy path).

### A3 · MAJOR · partly closed — the freshness gate proved "some authorization advanced"

**Decision taken: the fresh-baseline option, not full server-side attempt
correlation.** Stated plainly because the brief asked for that choice to be
explicit.

**What landed.** `ensureAuthKey` re-reads the row when Connect is clicked
instead of returning the `reconnectKey` prop, so the baseline is current as of
the authorization it is about to start rather than as of when the card was
clicked — a window that is minutes wide in the assistant flow. The read is
deliberately not caught: falling back to a stale baseline can settle the card
on someone else's authorization, so failing the click is the safer outcome.
Two tests in `add-key-dialog.test.tsx` cover both directions.

**What remains open.** The gate still proves "an authorization landed on this
row after Connect was pressed", not "*this* attempt landed". A concurrent
authorization of the same service from another surface — the AI Services page,
a second tab, `nyxid service scopes`, a second card — inside the window between
Connect and settlement still satisfies it.

**Why full correlation was not attempted here.** The server does hold a
correlation primitive, `UserApiKey.oauth_attempt_nonce` ("current chat-popup
OAuth attempt allowed to mutate this connection"), but it is *consumed*:
`user_api_key_service.rs` unsets it on completion, so it cannot witness after
the fact which attempt won. Making it witness would mean a new stored field
written on the OAuth token-write path, surfaced through the evidence contract,
and required by the browser before settling. If any fresh-authorization path
failed to stamp it — device-code reauthorization is the obvious candidate —
cards would hang to the timeout instead of completing, which is a worse failure
than the one being fixed. That is a change worth making deliberately, with its
own coverage of every authorization path, not as a rider here.

Note also that the residual is bounded by what the assistant is actually told:
with A2 in place the report is only issued when the service genuinely holds the
requested scopes and was genuinely authorized within the window. The remaining
inaccuracy is about *causation*, not about state — and Aevatar re-verifies
independently against its own recorded baseline.

### A4 · MINOR · closed here, still wrong on the PR

The disposition table in PR #1462's body claims the freshness mechanism was
delivered in that PR. It was not: all three parts pre-exist on `origin/main`
(`stores/pending-connect-store.ts:12`, `action-card.tsx:285`, `hooks/use-keys.ts`
×11), and neither `use-keys.ts` nor `pending-connect-store.ts` appears in its
diffstat. What that PR added on this axis is the exact-service **identity**
guard — real and correct, but a different control.

Corrected here, which is the authoritative record. The PR body itself still
carries the error; #1462 is now merged, so editing it is Calvin's call rather
than something to do unprompted.

### A5 · MINOR · closed — the freshness test now tests freshness

Two cases added to `action-card.test.tsx`: one where the key read always
returns the original `last_authorized_at`, asserting `onResolve` is never
called across repeated polls; and one that drives the watch past its deadline
and asserts the card reports the timeout note. Deleting the freshness gate from
`use-keys.ts` now fails the first.

### A6 · MINOR · closed — scope-grammar divergence documented

`docs/chat/06-actions-registry.md` gains a "Where the browser is stricter than
the published schema" section: the character class, the length and count
bounds, the RFC 6749 §3.3 comparison, the degradation path
(`{variant: "unknown"}` → "Unsupported action request"), and the instruction to
widen the browser regex rather than the manifest if a provider ever needs it.
The `params_schema` is unchanged — it is pinned by `JsonNode.DeepEquals`.

### A7 · MINOR · closed — `credential_source` is required

The reauthorization key schema now requires `credential_source` instead of
declaring it `.optional()`, so the org-admin guard fails loudly rather than
silently vanishing if the response shape changes. The backend remains the real
gate.

### A8 · MINOR · closed — one catalog entry, not the whole catalog

The preflight fetches `GET /api/v1/catalog/{slug}` and maps a 404 to the
existing catalog-unresolvable note, replacing a full `?include_all=true` fetch
per card click.

## 3. Section B — Aevatar prerequisites (hard merge gate)

Owner: **eanz17** / Aevatar, on `aevatarAI/aevatar` `feature/integrate`.
All four sites are in `agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistry.cs`
and **all four fail closed with the same catastrophic symptom** — `Load()` throws,
`CreateDisabled()` initializes, and every NyxID action card goes dark on Aevatar
processes started afterward (chat itself survives).

Editing only the first two is the likely mistake: it ships a consumer that passes
its own revision-map tests and still kills the registry.

| # | Site | Required change |
| --- | --- | --- |
| B1 | `PinnedActionsByRevision` (~line 207) | add `v8` → all four actions |
| B2 | `ExecutableActionsByRevision` (~line 234) | add `v8` → all four actions |
| B3 | `IsActionExecutable` wire-action switch (~line 293) | **`ServiceReauthorize` currently falls through to `null`** — map it to `"service.reauthorize"` or the verb stays non-executable regardless of B1/B2 |
| B4 | `ValidatePinnedContract` revision ternary (~line 886) | v8 is in neither branch, so `key.create` would be compared against the *unconstrained* schema while NyxID publishes the least-scope variant → `DeepEquals` fails → `RegistryInvalid` |
| B5 | `NyxIdActionEvidenceReadPort.GetUserServiceAuthorizationAsync` | point the evidence read at `GET /api/v1/keys/{id}/authorization` instead of `GET /api/v1/keys/{id}`. Without this, A1 stays live: the shared `GetServiceAsync` also serves the services tool and the SSH executor, which need the full document, so NyxID cannot narrow the detail response by caller. |
| B6 | `NyxIdActionEvidenceReadPort.GetAgentApiKeyAsync` | same, for `GET /api/v1/api-keys/{id}/authorization`. Closes F1's non-`name` carriers and F2. |

Plus: `docs/contracts/nyxid-assistant-conformance/v1/registry-v8.json` matching the
published manifest, and registry/conformance test updates.

**B4 rollout trap (F3).** B4's ternary is written over named constants
(`revision is LeastScopeRegistryRevision or SupportedRegistryRevision`). The
natural v8 implementation — repointing `SupportedRegistryRevision` from v7 to
v8 — silently drops v7 from the least-scope branch, so the new consumer rejects
the **live v7 manifest** during exactly the rollout step (2, below) that exists
to catch this. Add a v8 constant and extend the pattern list; do not repoint
`SupportedRegistryRevision`.

**Rollout order (was binding; step 3 has already happened).**
1. Aevatar merges + deploys the additive v8 consumer.
2. Restart an Aevatar process while NyxID still serves **v7**; prove the registry
   still loads with all current actions.
3. Only then NyxID merges + deploys v8.
4. Restart another Aevatar process, prove it loads v8, run one end-to-end
   `service.reauthorize`.

⚠ **Step 3 has been performed out of order**: PR #1462 merged to `main` on
2026-08-19, so NyxID's `main` publishes v8 before step 1. Step 2 can no longer
be run against `main`. Either hold the NyxID deploy until the Aevatar consumer
ships, or run step 2 against a NyxID build pinned to v7.

---

## 4. Section C — verification gaps

### C1 · closed — the full backend suite has now been run

Docker is unavailable in the implementation environment, so the replica set was
stood up directly: `mongod` (Homebrew `mongodb-community@7.0`) as a single-node
`rs0` on `127.0.0.1:27019`, with
`NYXID_TEST_DATABASE_URL=mongodb://127.0.0.1:27019/?replicaSet=rs0`.

Numbers are in §4a below. The earlier 5,189/129 split was never reproducible in
either direction and is superseded rather than confirmed.

### C1a · wire-visible changes on the existing detail routes

Asked for explicitly. "Merged" below means already on `main` via #1462;
"pending" means on the fix branch.

**`GET /api/v1/keys/{id}` and `GET /api/v1/keys` — three changes, all merged.**

| # | Change | Shape |
| --- | --- | --- |
| 1 | `connection_status`, `granted_scopes` and `last_authorized_at` lost their `skip_serializing_if`. They were omitted when null; they are now always present, as explicit `null`. | Additive for a tolerant parser. **Breaking** for any consumer that treats *absence* as meaningful — `"granted_scopes" in response` flips from `false` to `true` for every non-OAuth key. |
| 2 | `granted_scopes` values changed. The old code split `token_scopes` on whitespace only; it now splits on commas **and** whitespace and de-duplicates, preserving first-occurrence order. | **Value-breaking, and a fix.** A provider echoing `repo,read:user` previously produced the single garbage token `"repo,read:user"` (which was then re-submitted verbatim as a scope); it now produces `["repo", "read:user"]`. Repeated scopes now collapse to one entry. |
| 3 | `connection_status` derivation changed in both directions. | **Value-breaking.** It now returns `null` when the credential's `status != "active"` or it has no stored access token — so a `pending_auth` row that used to report `"expired"`/`"active"` now reports `null`. Conversely an active row with no `expires_at` used to report `null` and now reports `"active"`. |

Known consumers were checked: no Rust or TypeScript consumer of these three
fields exists in `cli/src`, `sdk/` or `mobile/src`, no frontend Zod schema
parses the key response, and `frontend/src/pages/keys.tsx` reads
`connection_status ?? status`, so change 3 gives a `pending_auth` row a
Reconnect affordance it did not have. External consumers were not enumerable.

**`GET /api/v1/api-keys/{id}` — no change.** F2's lineage-trio fix lives only on
the new projection; the detail response still emits
`state_version: 0` with two nulls for legacy rows, as it always has.

**Pending changes are additive only.** Two new routes,
`GET /api/v1/keys/{id}/authorization` and
`GET /api/v1/api-keys/{id}/authorization`. No existing response body is
modified by the fix branch.

### C2 · open · low — one unreachability claim is inferred, not proven

`oauth_connection_status` regressing a row from `"expired"` to `null` requires
`status == "active"` + `access_token_encrypted == None` + past `expires_at`. That
shape was argued unreachable from the write paths
(`user_api_key_service.rs:397-407`, `write_oauth_tokens_to_key`), not proven by
test. If constructible, the affected row loses its Reconnect button.

---

## 5. Findings that did not survive — do not re-raise

Each was checked against code by the independent pass.

- **Evidence endpoint** is `GET /api/v1/keys/{id}` via `NyxIdApiClient.GetServiceAsync`
  (`NyxIdApiClient.cs:279`), **not** `/api/v1/user-services`. The original briefing
  was wrong; the correction is what made this a serializer-sized change.
- **`params_schema` `DeepEquals`** — exact. Verified independently again for this
  document: `backend/src/handlers/assistant_actions.rs:92-110` is structurally
  identical to Aevatar's `ServiceReauthorizeParamsSchema`, with `risk=grant`,
  `tier=v1`, `remember_eligible=false`, `schema_version=4`.
- **All six evidence conditions** are satisfiable; removing `skip_serializing_if`
  on the three fields was necessary (Aevatar uses `RequireProperty`, so an omitted
  property is a hard `MalformedResponse`). Re-verified for this document:
  `keys.rs:436` (`connection_status`), `:500` (`granted_scopes`), `:504`
  (`last_authorized_at`) all serialize unconditionally, each with a comment
  recording why.
- **Serializer blast radius** — no Rust or TypeScript consumer of the three fields
  exists in `cli/src`, `sdk/`, or `mobile/src`; no frontend Zod schema parses the
  key response; Aevatar's `/keys` list parser reads only named properties and does
  not run the secret scan.
- **`oauth_connection_status` override was right.** The one non-evidence consumer
  (`frontend/src/pages/keys.tsx`) falls back to `connection_status ?? status`; the
  `pending_auth` case now *adds* a Reconnect affordance.
- **Comma-scope normalization is correct** and strictly better — the old
  `split_whitespace` turned a GitHub echo of `repo,read:user` into one garbage
  token that was then re-submitted verbatim as a scope.
- **No scope is silently dropped in the picker** — `buildPills` takes `value` as
  input, and `platformScopeAllowlist` is gated on `!isReconnect`.
- **A background token refresh cannot falsely satisfy the freshness gate** —
  `last_authorized_at` is written only on fresh-authorization paths, never by
  refresh (pre-existing NyxID#917 discipline).
- **Malformed params degrade gracefully** — `{variant: "unknown"}` yields
  `unsupportedDescriptor`, no CTA renders, `beginJourney` is unreachable.
- **Merge gate is real** — PR is draft, no reviews, nothing posted to #1400, no
  Aevatar issue filed, nothing deployed.

---

## 5a. Section F — fourth-pass findings (Fable, 2026-08-19)

Evidence: `wave1-relook-fable.md` on `review/2026-08-19_wave1-relook`
(`d6c57a00`). That pass found **no errors** in the other three documents.

### F1 · MAJOR · partly closed — the A1 tripwire class also hits `key.create` and `key.rotate`

#1400 item 1 is a three-verb requirement, and the two key verbs were never
audited to the standard applied to `service.reauthorize`. Aevatar verifies both
through `GET /api/v1/api-keys/{id}` (`GetAgentApiKeyAsync`), and
`ParseAgentApiKeyDocument` runs the same `RejectSecretBearingRead` scan.
`ApiKeyResponse` reaches it with four user-controlled carriers: `name`,
`description`, `allowed_services[].label` (the same `UserEndpoint.label` A1
flags), and `allowed_nodes[].name`. `key.rotate` inherits the problem because
the successor clones `name` and `description` from the predecessor.

**Closed:** `GET /api/v1/api-keys/{id}/authorization` projects to the thirteen
properties the reader consumes, removing `description`, `allowed_services`,
`allowed_nodes` and everything else. Symmetric tests to A1's, both directions.

**Irreducible remainder:** `name` is itself required evidence — the reader both
demands it (`RequireNormalizedString(root, "name")`) and scans it, and the
postcondition pins `evidence.Name == params.name`. A key named `Bearer Bot`
stays unverifiable no matter what NyxID serves. No projection can fix that; it
needs either a write-time restriction on key names (which would also have to
handle rotation inheritance and existing rows) or an Aevatar-side change to
exclude its own required evidence properties from the scan. Recommended: the
latter — the scan exists to catch *unexpected* secret-bearing fields, not to
reject the fields it deliberately reads.

Also depends on **B6** to take effect at all.

### F2 · MINOR · closed — legacy pre-lineage key rows were structurally unparseable

`ApiKeyResponse` always serializes the lineage trio, so a pre-#1406 row emits
`rotation_predecessor_id: null`, `state_version: 0`, `updated_at: null`.
Aevatar's `ParseOptionalVersionEvidence` sees all three present, skips its
absent-trio exit, then throws on `state_version <= 0` — the whole document is
malformed. Zero impact today because create/rotate only read freshly written
rows, but every legacy key violates the published evidence contract.

The projection emits the trio as a trio or not at all: `state_version >= 1` or
all three omitted. The detail response is unchanged, so this is not a wire
change to `/api-keys/{id}`. Two tests pin both shapes.

### F3 · MINOR · recorded — B4 rollout trap

Folded into §3 above, and it belongs in the Aevatar issue text (D1).

### F4 · MINOR · recorded — no clock-skew allowance

All three verbs reject evidence timestamps a single tick ahead of Aevatar's
clock (`LastAuthorizedAtUtc > now`, `NyxIdActionPostconditionPort.cs:306-308`;
create `:381-382`; rotate `:460-464`). NyxID stamps, Aevatar compares, tolerance
is zero, and verification runs immediately after the journey — the worst case
for small positive NyxID skew. The failure is a retryable `StaleCode`, so this
is a robustness note, not a bug. Deliberately **not** fixed here: any tolerance
is Aevatar-side and should not be introduced unilaterally.

---

## 6. Section D — coordination, owner: Calvin

| # | Action | Status |
| --- | --- | --- |
| D1 | Post the drafted Aevatar issue (text in PR #1462 body) — **add the B3/B4 code sites, which the draft under-specifies** | not posted |
| D2 | Post the drafted #1400 comment (text in PR #1462 body) | not posted |

Both were deliberately left unposted.

---

## 7. Issue #1400 item 1 — alignment check

Verified against the issue text and this branch on 2026-08-18.

| Issue item-1 requirement | State |
| --- | --- |
| Registry **descriptor** in the compile-time manifest for `service.reauthorize`, `key.create`, `key.rotate` — grammar-conformant params, model-facing description, `risk`/`tier`/`remember_eligible`; new revision pin | **Met.** `assistant_actions.rs:9` pins `nyxid-assistant-actions.v8`; four descriptors in order `[service.connect, service.reauthorize, key.create, key.rotate]`. `key.create`/`key.rotate` shipped earlier (v6/v7); `service.reauthorize` added here. |
| **Card + journey** — browser executes the verb with the user's own session and reports exactly one typed safe resource (`userService` for reauthorize; `key` for key.create/rotate); key material shown once, never in a report | **Met structurally, defective in completion logic.** Journey reports only `{userService:{userServiceId}}`. A2/A3 mean it can report `completed` when the grant did not actually change — the *shape* is right, the *truth* is not yet. |
| **Facade acceptance of non-`userService` completed resources** (gap G6(c)) | **Met — no code change needed.** `ActionResource` (`assistant_service.rs:472-479`) carries all six variants including `Key`; verified directly. Closed by #1404. |
| Param-shape confirmation: `{keyId, requestedScopes}` vs `{userServiceId, requestedScopes[]}` (aevatar#3315 item 6) | **Settled: `{userServiceId, requestedScopes[]}`.** Aevatar already implemented this shape on `feature/integrate`; NyxID now publishes it byte-identically. No decision outstanding. |
| Wave 1 hardening follow-ups #1405, #1406 | **Both closed.** |

**Conclusion.** Item 1 is fully addressed in scope and contract **for
`service.reauthorize`**. It is not fully addressed for the three verbs
together: `key.create` and `key.rotate` shipped earlier (v6/v7) and were never
audited to the same standard — see F1, which found the identical evidence-read
tripwire on `GET /api/v1/api-keys/{id}` and one carrier (`name`) that no NyxID
change can close.

It is not yet *complete* either — completion requires A1's Aevatar half plus
§3 B1–B6 shipped and deployed by Aevatar. The issue's framing that this "extends verb coverage beyond
`service.connect`" is now accurate for all three Wave-1 verbs.

Items 2 (exact-service non-blocking approval) and 3 (facade v4 conformance) of
#1400 are untouched by this work; item 3 shipped in #1404.
