# Adversarial review: Wave 1 `service.reauthorize` (PR #1462)

> **Evidence document, not an action list.** The live action list is
> [`wave1-service-reauthorize-actions.md`](./wave1-service-reauthorize-actions.md),
> where every finding below appears as a tracked item (A1–A8, B1–B4, C1–C2).
> This document holds the full reproductions, code citations, and the record of
> which earlier concerns did **not** survive scrutiny.

Reviewer: Opus (third pass, adversarial). Reviewed commit `5b1189ee` on
`feat/2026-08-17_wave1-reauthorize-impl`, diffed against `origin/main`.
Aevatar consumer read from `aevatarAI/aevatar` `origin/feature/integrate`
(fetched 2026-08-17).

Prior artifacts: `docs/chat/wave1-service-reauthorize-plan.md` (Fable),
`docs/chat/wave1-service-reauthorize-review-sol.md` (Sol).

---

## Overall verdict

**BLOCKED** — on the consumer prerequisite the PR already declares, plus two
MAJOR defects that should be fixed before the Aevatar-first rollout completes.

| Severity | Count |
| --- | --- |
| BLOCKER | 1 |
| MAJOR | 3 |
| MINOR | 5 |

The single most dangerous finding is **M1**: an ordinary, NyxID-validated,
user-configurable WebSocket auth template of the shape
`{"headers":{"Authorization":"Bearer ${credential}"}}` makes `GET
/api/v1/keys/{id}` permanently unparseable by *both* Aevatar's evidence parser
and this PR's own browser preflight. The affected service can never be
re-authorized through the assistant, and the only user-facing signal is the
generic "NyxID could not verify this service for re-authorization."

The contract work itself is correct. I re-derived the published descriptor
against Aevatar's `ValidatePinnedContract` and all six evidence conditions in
`VerifyServiceReauthorizeAsync` and found no near-miss. Several concerns raised
in the review brief turned out to be unfounded; those are recorded in
§"Concerns that did not survive" so they are not re-litigated.

---

## BLOCKER

### B1. Publishing v8 before Aevatar ships a v8 consumer disables the entire assistant-action registry

**Verified.** `agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistry.cs`
`PinnedActionsByRevision` (~line 207) contains only v4, v5, v6, v7. `Load()`
(~line 322) does:

```csharp
if (!PinnedActionsByRevision.TryGetValue(revision, out var pinnedActions) ||
    !ExecutableActionsByRevision.TryGetValue(revision, out var executableActions))
    throw Error(RevisionUnsupported, "The NyxID action registry revision is not supported.");
```

`nyxid-assistant-actions.v8` (`backend/src/handlers/assistant_actions.rs:9`) hits
that throw. Every Aevatar process started after a NyxID v8 deploy loses **all**
NyxID action cards, not just `service.reauthorize`.

This is the PR's own declared merge gate and it is correctly and prominently
stated. I confirm the gate is real (see §"Merge gate" below). It stays a BLOCKER
because it is not yet retired.

**Two prerequisite details the PR body under-specifies.** Adding v8 to the two
revision maps is *not* sufficient. Both of these are in
`NyxIdAssistantActionRegistry.cs` and both fail closed in ways that look like the
same catastrophic symptom:

1. `IsActionExecutable` (~line 293) maps action kinds to wire actions through a
   hardcoded switch:

   ```csharp
   var wireAction = action switch
   {
       NyxIdAssistantActionKind.ServiceConnect => "service.connect",
       NyxIdAssistantActionKind.KeyCreate => "key.create",
       NyxIdAssistantActionKind.KeyRotate => "key.rotate",
       _ => null,
   };
   ```

   `ServiceReauthorize` falls through to `null`, so the verb stays
   non-executable no matter what the revision maps say.

2. `ValidatePinnedContract` (~line 886) selects the pinned `key.create` schema
   by revision:

   ```csharp
   var pinnedParamsSchema = revision is LeastScopeRegistryRevision or SupportedRegistryRevision &&
                            contract.Action == NyxIdAssistantActionKind.KeyCreate
       ? LeastScopeKeyCreateParamsSchema
       : contract.PinnedParamsSchema;
   ```

   v8 is in neither branch, so `key.create` would be `DeepEquals`-compared
   against the *unconstrained* `KeyCreateParamsSchema`. NyxID publishes the
   least-scope variant with `minItems: 1` / `maxItems: 64` / `uniqueItems: true`
   (asserted at `backend/src/handlers/assistant_actions.rs:527-536`), so the
   comparison fails and `Load()` throws `RegistryInvalid` — again killing the
   whole registry.

   The PR body's prerequisite 3 does say "Preserve the least-scope `key.create`
   schema pin for v8 as well", which is the right instruction; it does not name
   the code site, and a reader who only edits the revision maps will ship a
   broken consumer that passes its own revision-map tests.

**Fix:** keep the merge gate. When drafting the Aevatar issue, name all four
edit sites explicitly: `PinnedActionsByRevision`, `ExecutableActionsByRevision`,
the `IsActionExecutable` wire-action switch, and the `ValidatePinnedContract`
revision ternary.

---

## MAJOR

### M1. User-controlled `Bearer …` strings in `KeyResponse` permanently break both the journey and the verify

**Verified, with a reproduction.** Two independent recursive scanners run over
the *entire* `/keys/{id}` body:

- Aevatar: `NyxIdApiAccessContracts.cs` `RejectSecretBearingRead`, called first
  in `ParseUserServiceAuthorizationDocument`. Value pattern:
  `(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})`, `IgnoreCase`. A hit
  throws `NyxIdContractException` → `MalformedResponse` → `ProviderReadFailure`
  → the action is never verified.
- This PR: `frontend/src/components/assistant/blocks/action-card.tsx:110-128`
  (`assertSecretFreeReadBack`), same pattern, run on the raw response in
  `readReauthorizationKey` (`:135-137`) *before* the journey opens.

`KeyResponse` carries at least three user-controlled free-text carriers that
reach both scanners:

| Field | `keys.rs` | Serialization |
| --- | --- | --- |
| `ws_frame_injections[].template` | `:509-511` | always (bare `Vec`) |
| `default_request_headers[].value` | `:504-506` | present whenever the user set any; only `sensitive: true` entries are redacted (`models/default_request_header.rs:344-364`) |
| `label` / `name` | `:417-418` | always |

The WS template case is the sharp one, because **NyxID's own validator
explicitly permits exactly the string Aevatar rejects**.
`services/ws_frame_injector.rs:189-197`:

```rust
fn validate_template_does_not_embed_credentials(idx: usize, template: &str) -> AppResult<()> {
    let stripped = template.replace("${credential}", "");
    if stripped.contains("nyxid_") || contains_jwt_like_literal(&stripped) { ... }
```

`{"headers":{"Authorization":"Bearer ${credential}"}}` strips to
`{"headers":{"Authorization":"Bearer "}}` — no `nyxid_`, no `eyJ` segment —
so NyxID stores it, and then refuses to read it back. Verified against the
literal regex:

```
TRIPS  ws_frame_injections[].template   ->   {"headers":{"Authorization":"Bearer ${credential}"}}
ok     ws_frame_injections[].template   ->   {"type":"auth","access_token":"${credential}"}   (the HA example in NyxID's own tests)
TRIPS  default_request_headers[].value  ->   Bearer sk-live-abc123
TRIPS  UserService.label                ->   Bearer Bot
```

Sol's mitigation (`backend/src/handlers/keys.rs:2664-2749`) asserts the scan
over `key_response_from_result` built from a `CreateKeyResult` with
`api_key: None` — a near-empty response with no `ws_frame_injections` entries,
no `default_request_headers`, and a fixed test label. It cannot catch any of the
three carriers above. Sol's own review flagged this as a residual hardening gap;
it is not hypothetical, it is reachable through a documented, admin- and
user-supported feature (CLAUDE.md §6 describes `ws_frame_injections` as the
supported WS auth mechanism).

Fail mode is closed, not leaky — no secret escapes. But the feature is silently
dead for the affected service, and the user sees only
`REAUTHORIZE_UNAVAILABLE_NOTE` ("NyxID could not verify this service for
re-authorization"), which points them at AI Services where nothing is wrong.

**Fix (pick one):**

1. Preferred — serve a minimal evidence projection. Either a dedicated
   representation for the evidence read, or drop `ws_frame_injections`,
   `default_request_headers`, and any other free-text carrier from it. Aevatar
   only reads `id`, `api_key_id`, `is_active`, `status`, `connection_status`,
   `granted_scopes`, `last_authorized_at`; everything else on that response is
   pure attack surface for the tripwire.
2. Cheaper stopgap — reject `Bearer\s` (and the rest of the Aevatar value
   pattern) in `validate_template_does_not_embed_credentials`, in
   `default_request_header` value validation, and in label validation, so NyxID
   never stores a value it cannot read back. This is a behaviour change for
   existing rows and does not repair data already stored, so it is strictly
   worse than (1).
3. At minimum — make the browser distinguish this case from a transport
   failure, and add a backend test that runs `assert_aevatar_secret_free` over a
   *fully populated* `KeyResponse` including a `Bearer ${credential}` WS
   template. Today that test would fail, which is the point.

### M2. `completed` is reported without checking that the requested scopes were actually granted

**Verified.** Both completion paths report `completed` on identity match plus
`status == active` plus `last_authorized_at` advancement. Neither looks at
`granted_scopes`:

- watch path — `action-card.tsx:513-541`: guards `keyId !== user_service_id`,
  then `onResolve({disposition: "completed", resource: {userService: {userServiceId: keyId}}})`.
- dialog path — `action-card.tsx:965-973`: `onSuccess` guards
  `userServiceId !== params.user_service_id`, then `report("completed", ...)`.

Aevatar's condition 5 (`NyxIdActionPostconditionPort.cs:294-296`) *does* check
it:

```csharp
!input.Params.ServiceReauthorize.RequestedScopes.All(scope =>
    evidence.GrantedScopes.Contains(scope, StringComparer.Ordinal))
```

so the model is not misled — it gets `MismatchCode`. The **user** is: the card
renders the `Re-authorized` badge and the footer "The assistant received only
the verified service reference", and the conversation then behaves as if nothing
happened.

Three reachable ways to land here:

1. The user deselects a requested scope in `UpstreamScopePicker` before
   clicking Connect. `prefillScopes` only seeds the `useState` initializer
   (`add-key-dialog.tsx:1530-1536`); nothing pins it afterwards.
2. The provider grants a subset (GitHub org access not approved, Google consent
   screen with scopes unchecked, Lark app permission still pending review).
3. The provider omits `scope` from the token response. The backend then
   **preserves the previous `token_scopes`**
   (`services/user_api_key_service.rs:568-580`, deliberately, per the NyxID#917
   comment) while still stamping `last_authorized_at`. So the freshness gate
   passes with a completely unchanged grant set.

**Fix:** in both completion paths, re-read `/keys/{id}` and require
`requested_scopes ⊆ granted_scopes` (case-sensitive, matching Aevatar's
`StringComparer.Ordinal`) before `report("completed", …)`. On a shortfall, call
`onBlock` with a note naming the missing scopes — that is a far better user
outcome than a green badge followed by an assistant that says the permission
isn't there.

### M3. The freshness gate proves "some authorization advanced", not "this attempt's authorization completed"

**Verified.** The predicate is a timestamp inequality with no server-side
correlation to the attempt:

```ts
// frontend/src/hooks/use-keys.ts:213-216
const authorizationAdvanced =
  previousAuthorizationAt === undefined ||
  (query.data?.last_authorized_at != null &&
    query.data.last_authorized_at !== previousAuthorizationAt);
```

`attemptId` is a client-side `crypto.randomUUID()` used only as a TanStack Query
cache generation (`use-keys.ts:154`); it never reaches the server and is never
matched against anything.

The baseline is captured from a **stale snapshot**. `handleConnect` does
`key = await ensureKey()` (`add-key-dialog.tsx:1764`), and for reconnect mode
`ensureAuthKey` returns the `reconnectKey` prop verbatim
(`add-key-dialog.tsx:3191-3203`) — the object `readReauthorizationKey` fetched at
*card-click* time, which may be minutes old by the time the user presses
Connect. The baseline is then `key.last_authorized_at`
(`add-key-dialog.tsx:1792-1794`).

Consequences:

- Any unrelated fresh authorization of the same key inside that window (the AI
  Services page, a second browser tab, `nyxid service scopes`, the CLI wizard, a
  second assistant card for the same service with different scopes) satisfies
  the gate and settles the card as `completed`.
- Two concurrent reauthorize cards for one service share the same baseline;
  completing either settles both.

Combined with M2 this is the real "false `completed`" surface: not a background
refresh (that path is genuinely closed — see §"Concerns that did not survive"),
but *someone else's* authorization being claimed as this attempt's.

**Fix:** correlate to the server-side attempt rather than to a timestamp delta.
`initiateOAuthAsync` already returns an `attempt_nonce` and the flow already
threads a `connection_id`; settle on evidence that *this* attempt landed. If
that is too large for this wave, at minimum re-read `/keys/{id}` inside
`handleConnect` and derive the baseline from that fresh read, which closes the
card-click→Connect-click window (it does not close the concurrent-flow case).

---

## MINOR

### m4. The PR body misattributes the M3-class fix; no freshness code is in this diff

The disposition table says:

> MAJOR: a currently-active key could make F2 report completed without a new
> authorization — **Fixed.** Pending attempts carry `previousAuthorizationAt`;
> the watcher requires `last_authorized_at` to advance for reconnects.

All three parts of that mechanism already exist on `origin/main`:

- `git show origin/main:frontend/src/stores/pending-connect-store.ts` → line 12,
  `readonly previousAuthorizationAt: string | null | undefined;`
- `git show origin/main:frontend/src/components/assistant/blocks/action-card.tsx`
  → line 285, `previousAuthorizationAt: pendingAuth?.previousAuthorizationAt,`
- `git show origin/main:frontend/src/hooks/use-keys.ts` → 11 occurrences

`frontend/src/hooks/use-keys.ts` and
`frontend/src/stores/pending-connect-store.ts` are not in this PR's diffstat at
all. What this PR actually adds on that axis is the **identity** guard
(`action-card.tsx:513-525`), which is real and correct but is a different
control.

This matters because it is the line a reader trusts to conclude the risk was
retired *and re-verified here*. It was inherited, and it happens to be reachable
only because `reconnectMode` is true for this journey
(`add-key-dialog.tsx:2911, 3499`) — a coupling nothing in this PR tests or
documents. Sol's review §3 was written against the pre-rebase branch point and
is accurate about that branch; the PR body then carried the "Fixed" verdict
forward without re-checking what the rebase brought in.

**Fix:** correct the table entry to "already present on `main`; this PR adds the
exact-service identity guard and relies on the inherited baseline gate", and add
the negative test in m5.

### m5. The freshness test does not test freshness

`frontend/src/components/assistant/blocks/action-card.test.tsx` — "waits for a
fresh authorization timestamp before auto-completing reauthorization". The mock
returns the original `last_authorized_at` on read 1 and an advanced one on read
2, then asserts `onResolve` was called once with `completed`.

Delete the entire freshness gate from `use-keys.ts` and this test still passes.
The assertion that carries the name — an unchanged `last_authorized_at` must
*not* settle the card — is absent.

**Fix:** add a case whose key read always returns the original
`last_authorized_at`, and assert `onResolve` is never called (and that the card
eventually reports the timeout note). The rest of the new tests in this PR are
genuine behavioural tests, not padding; this one overclaims in its name.

### m6. Browser scope grammar is stricter than both the published schema and RFC 6749

`frontend/src/schemas/assistant-actions.ts:49-56`:

```ts
.regex(/^[A-Za-z0-9._:/~+*=-]+$/, "Scope contains characters NyxID cannot request")
```

The published `params_schema` says `{"type": "string"}`, and Aevatar's
`ValidateRequest` enforces only that. RFC 6749 §3.3 `scope-token` permits every
printable ASCII except space, `"` and `\`. A provider scope containing
`! # $ % & ' ( ) , ; < > ? @ [ ] ^ { | }` parses fine at Aevatar, then fails
`serviceReauthorizeActionParamsSchema` → `{variant: "unknown"}` →
`resolveAssistantAction` returns `unsupportedDescriptor` → the user sees
"Unsupported action request" with no indication which scope was rejected.

I could not find a real catalog provider that trips this (Google, GitHub, Slack,
Lark, Microsoft, Zoom, HubSpot, Atlassian scopes all pass), so the practical
likelihood is low. It is worth documenting the divergence in
`docs/chat/06-actions-registry.md`, since the published schema is the contract
and this restriction is invisible from it.

Same class, same file: `requestedScopes` rejects an empty array (`.min(1)`)
while the published schema permits one. Aevatar's `VerifyServiceReauthorizeAsync`
also requires non-empty (`requireAny: true`), but its *production* path
(`ValidateRequest`) does not — so an LLM emitting `requestedScopes: []` produces
a dead card rather than a clear error.

### m7. The org-admin preflight guard is optional-and-fails-open

`action-card.tsx:79-88, 158-163`:

```ts
credential_source: reauthorizationCredentialSourceSchema.optional(),
...
if (snapshot.credential_source?.type === "org" && snapshot.credential_source.role !== "admin")
```

`KeyResponse.credential_source` is mandatory (`keys.rs:544-546`, no
`skip_serializing_if`), so this is currently unreachable. But an `.optional()`
schema means the guard silently vanishes if the response shape ever changes,
and the whole point of this preflight is defence in depth. Make it required so a
shape change fails loudly instead of quietly widening who can drive a re-auth on
a shared org credential. (The backend remains the real gate.)

### m8. Preflight fetches the whole catalog to read one entry

`readReauthorizationKey` does `api.get("/catalog?include_all=true")`
(`action-card.tsx:167-169`) and then `.find(entry => entry.slug === catalogSlug)`.
`GET /api/v1/catalog/{slug}` exists and returns the same
`provider_type` / `provider_config_id` / `device_code_format` fields. This is one
full-catalog fetch per card click.

---

## Concerns from the review brief that did not survive

Recorded so they are not re-raised. Each was checked against code, not against
the prior reviews.

**Evidence endpoint — Sol is right, the original briefing was wrong.**
`NyxIdActionEvidenceReadPort.GetUserServiceAuthorizationAsync` calls
`client.GetServiceAsync`, which is `GET /api/v1/keys/{id}`
(`NyxIdApiClient.cs:279`). Not `/api/v1/user-services`.

**Params-schema `DeepEquals` — exact.** Aevatar already carries
`ServiceReauthorizeParamsSchema` (`NyxIdAssistantActionRegistry.cs:90-101`) and
it is structurally identical to what NyxID publishes
(`assistant_actions.rs:91-108`). `JsonNode.DeepEquals` is whitespace- and
property-order-insensitive. `risk=grant`, `tier=v1`,
`remember_eligible=false`, `schema_version=4` all line up with
`SupportedActions["service.reauthorize"]` and `SupportedSchemaVersion`.

**All six evidence conditions are satisfiable.**

| Aevatar requirement | NyxID source | Status |
| --- | --- | --- |
| `RequireNormalizedString(root, "id")` | `KeyResponse.id` | ok |
| `RequireBoolean(root, "is_active")` | `KeyResponse.is_active` (`keys.rs:469`) | ok |
| `status` ∈ {active, expired, revoked, failed, refresh_failed, pending_auth} | `UserApiKey.status`, documented domain at `models/user_api_key.rs:68` | exact match |
| `connection_status` **property present**; null or active/expired | now always serialized (`keys.rs:435`) | fixed by this PR |
| `granted_scopes` **property present**; null or **distinct** normalized strings | now always serialized (`keys.rs:498`); `parse_granted_scopes` dedups Ordinal | fixed by this PR — the dedup is load-bearing, `ReadNormalizedStringArray` throws on duplicates |
| `last_authorized_at` **property present**; null or RFC3339 | now always serialized (`keys.rs:503`); `chrono::to_rfc3339` emits `…+00:00`, which matches Aevatar's `Rfc3339Pattern` | fixed by this PR |

Removing `skip_serializing_if` on those three was necessary and correct:
Aevatar uses `RequireProperty` (not `TryGetProperty`) for all three, so an
omitted property is a hard `MalformedResponse`, not a soft miss.

**Blast radius of the serializer change — both of Sol's claims re-derived and
they hold.**

- No Rust or TypeScript consumer of `granted_scopes`, `connection_status`, or
  `last_authorized_at` exists in `cli/src`, `sdk/`, or `mobile/src` (grepped;
  zero hits). The classic `#[serde(default)]`-on-a-non-`Option`-field-breaks-on-
  explicit-null hazard has no target here.
- No frontend Zod schema parses the key response;
  `frontend/src/types/keys.ts:24` types `connection_status` as
  `?: "active" | "expired" | null`, which already accepts an explicit null.
- Aevatar's `ParseUserServiceKeysDocument` (the `/keys` list parser) reads only
  named properties, ignores additive fields, and — importantly — does **not**
  call `RejectSecretBearingRead`. The list path is unaffected.
- `frontend/src/components/dashboard/add-key-dialog.tsx` is **not** in
  `cli/src/wizard/bundle-meta/index.manifest`, so no wizard-bundle rebuild is
  required and the freshness test is legitimately green.

**`oauth_connection_status` override — Sol overrode the plan and was right.**
The only consumer outside the evidence path is
`frontend/src/pages/keys.tsx:100,152,338`, and it falls back to
`connection_status ?? status`. Walking every credential shape:

- expiry-less oauth2, `status == active`, access token stored → was `None`,
  now `Some("active")`; `effectiveStatus` is `"active"` either way. No change.
- `status == pending_auth` with a stale `expires_at` → was `Some("active")`,
  now `None` → `effectiveStatus` becomes `"pending_auth"`, which *adds* the
  Reconnect affordance (`RECONNECTABLE_STATUSES`, `keys.tsx:84-89`). An
  improvement.
- terminal states still short-circuit to `"expired"` before the new guards.
- the ordinary expired case (`status == active`, past `expires_at`, no refresh
  token, access token present) still yields `"expired"`.

The one shape that would regress — `status == "active"` with
`access_token_encrypted == None` and a past `expires_at` — I could not
construct: `sync_provider_token_to_api_keys_impl`
(`user_api_key_service.rs:397-407`) writes the access token whenever it writes
`active`, and `write_oauth_tokens_to_key` always writes both. *Inferred, not
proven by test* — flagging the reasoning so it can be challenged.

**Comma-scope normalization — correct, and strictly better than what it
replaced.** `parse_granted_scopes` (`unified_key_service.rs:3819-3828`) splits on
comma-or-Unicode-whitespace, drops empties, and dedups on first occurrence with
`HashSet<&str>` (case-sensitive). That matches Aevatar's `Ordinal` `Distinct`
requirement and its `Ordinal` `Contains` comparison. Order is preserved and no
consumer depends on order (the frontend diffs granted-vs-selected as sets;
Aevatar uses `Contains`). The old `split_whitespace` turned a GitHub echo of
`repo,read:user` into the single garbage token `repo,read:user`, which was then
re-submitted verbatim as a scope on the next reconnect — the new behaviour fixes
that too.

**No scope is silently dropped in the picker.** I specifically looked for this.
`UpstreamScopePicker.buildPills` takes `value` as an input, so an
assistant-requested scope outside the catalog still becomes a pill, and
`toggle` re-emits from `pills.map(...).filter(...)`
(`upstream-scope-picker.tsx:165-176`) rather than from the catalog — so touching
any pill does not drop an unknown prefill scope. `platformScopeAllowlist`, the
other place that filters `submittedScopes`, is gated on `!isReconnect`
(`add-key-dialog.tsx:3512`) and is therefore inactive for this journey.

**A background token refresh cannot falsely satisfy the freshness gate.** This
was the worst hypothetical and it is closed by pre-existing NyxID#917
discipline: `last_authorized_at` is written only by
`sync_provider_token_to_api_keys_after_authorization`
(`user_api_key_service.rs:338-345`) and `write_oauth_tokens_to_key` /
`store_device_code_tokens` — never by refresh paths, which call the
non-stamping wrapper. The `OAUTH_REFRESH_SWEEP_INTERVAL_SECS` sweep cannot
advance it.

**Malformed params degrade gracefully, they do not strand the card.**
`resolveAssistantAction` (`action-registry.ts:141-165`) returns
`unsupportedDescriptor` whenever `journey === null`, which is what
`{variant: "unknown"}` produces, so `unsupported` is true and no CTA renders.
`beginJourney` is unreachable in that state.

**Merge gate is real.** `gh pr view 1462`: `isDraft: true`, `reviews: []`, the
only comment is the `github-actions` coverage bot. `gh issue view 1400`: newest
comments are 2026-08-10 from eanz17 on unrelated work — the drafted #1400
comment was not posted. No `aevatarAI/aevatar` issue mentioning
`assistant-actions v8` exists. Nothing was deployed. The drafted coordination
text lives only in the PR body, as instructed.

---

## Test results (re-run by me on this branch)

| Command | Sol's claim | My result |
| --- | --- | --- |
| `npm --prefix frontend run test` | 246 files / 2,847 tests, 0 failed | **246 files / 2,847 tests passed, 0 failed** — matches exactly |
| `npm --prefix frontend run lint` | 0 errors, 23 warnings | **0 errors, 23 warnings** — matches exactly |
| `npm --prefix frontend run build` (CI parity: `tsc -b`, `noUncheckedIndexedAccess`) | passed | **passed** — main build 3,470 modules, credential-accept 107 modules, mock-footprint assertion passed |
| `cargo test assistant_actions` | 5 passed | **5 passed, 0 failed** (5,330 filtered out) |
| `cargo test unified_key_service` | 163 passed | **163 passed, 0 failed** (5,172 filtered out) |
| `cargo fmt --check` | passed | **passed** |
| `cargo clippy --workspace --all-targets -- -D warnings` | passed | **passed**, exit 0, no warnings emitted |
| `cargo test -p nyxid-cli` | 1,100 + 1 wizard freshness | **1,100 passed + 1 wizard-freshness passed, 0 failed** — matches exactly |
| full `cargo test` | 5,189 passed / 129 failed, all attributed to a missing MongoDB replica set | **not re-run** — see below |

Notes:

- The machine's disk was at 100% (0 bytes free) when I started, which corrupted
  a cached `utoipa-swagger-ui` archive mid-build. I reclaimed one month-stale
  cargo build cache (`worktrees/3571ce16/we-are-plannign-to-pivot/target`, 27G,
  last touched 2026-07-16 — build output only, no source) and re-seeded the
  archive from a sibling worktree. All numbers above are from clean runs after
  that.
- **I did not attempt the full backend suite.** No MongoDB replica set is
  available here and `NYXID_TEST_DATABASE_URL` is unset, so I would reproduce
  exactly the connection-failure signature Sol describes and learn nothing. I
  can neither confirm nor refute the 5,189/129 split or the attribution of all
  129 failures to fixtures. Sol was explicit that this is not a green-suite
  claim, which is the right posture; it also means **nobody has run the full
  backend suite against this change**. Someone with a replica set should, before
  the eventual merge.
- Coverage bot on the PR reports frontend line coverage 65.28% (+0.06 vs base),
  above the 15% gate.

### Are the new tests meaningful?

Mostly yes. Concretely:

- `frontend/src/schemas/assistant-actions.test.ts` — the seven-case rejection
  table (empty, duplicate, untrimmed, space-packed, comma-packed, widened
  object, invalid identity) is real negative testing, and the URL-shaped-scope
  case pins the grammar against a realistic Google scope. Keep.
- `add-key-dialog.test.tsx` — the three scope-merge tests assert the exact
  `scopeOverride` array reaching `initiateOAuth`/`initiateDeviceCode`, including
  dedup order and the RFC 8628 path. Real. Keep.
- `action-card.test.tsx` — the non-OAuth block, the OpenAI-device-code block,
  and the wiring test (`data-reconnect-key`, `data-prefill-scopes`,
  `data-launch`, `data-flow`) are real behavioural tests. Keep.
- `action-card.test.tsx` freshness test — see **m5**. It would pass with the
  feature deleted.
- `backend/src/handlers/keys.rs::key_response_always_serializes_authorization_evidence_properties`
  — the three `Some(Value::Null)` assertions are exactly right and would catch a
  regression on the `skip_serializing_if` removal. The
  `assert_aevatar_secret_free` half is near-worthless as written: it scans a
  response with no api_key, no WS injections, no headers, and a fixed test
  label, so it can never fail. See **M1** for what it would need to cover to
  earn its place.
- `backend/src/handlers/assistant_actions.rs` golden-manifest changes — index
  shifts plus one added descriptor, compared against a hand-written expected
  value. Not a tautology (the expected literal is independent of the
  production constant except for the shared `SERVICE_REAUTHORIZE_DESCRIPTION`
  reference). Fine.

---

## Claims I could not confirm

Every numeric test claim in the PR body that I could execute reproduced
exactly. Two things remain open:

1. **The full-suite 5,189 / 129 split and the fixture attribution.** Not
   reproducible here — no MongoDB replica set, `NYXID_TEST_DATABASE_URL` unset.
   Unverified either way. Sol did not claim it green, which is correct, but it
   also means no full backend run exists for this change.
2. **That the `status == "active"` + missing-access-token + past-`expires_at`
   shape is unreachable.** Argued from the write paths
   (`sync_provider_token_to_api_keys_impl`, `write_oauth_tokens_to_key`), not
   proven by test. If someone can construct it, `oauth_connection_status`
   regresses that row from `"expired"` to `null` and hides its Reconnect button.

Separately, the PR body's **disposition-table claim that the freshness fix was
delivered here is wrong** (see **m4**) — the mechanism was inherited from
`main`. That is an attribution error, not a test-number error.

---

## Verdict

**BLOCKED.**

The PR is honest about its own gate and the contract work is exact — I went
looking for a near-miss against `DeepEquals` and the six evidence conditions and
did not find one. Before this lands:

- keep B1's Aevatar-first gate, and add the two under-specified Aevatar code
  sites to the coordination text;
- fix **M1** (minimal evidence projection) — this is the one that silently
  bricks real user services;
- fix **M2** (require `requested_scopes ⊆ granted_scopes` before reporting
  `completed`);
- fix or explicitly accept **M3** with a written rationale;
- correct **m4** in the PR body and add the negative freshness test from **m5**.

m6–m8 are safe to defer.
