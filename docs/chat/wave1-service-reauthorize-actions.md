# Wave 1 `service.reauthorize` — consolidated action list

**This is the working document for PR #1462.** It is the single place that says what
is done, what is outstanding, and who owns each item. The three documents it
consolidates are evidence, not instructions:

| Document | Author | Role now |
| --- | --- | --- |
| `wave1-service-reauthorize-plan.md` | planner | Contract reference (§1 target contract, §5 rollout). Task breakdown is **superseded by this file**. |
| `wave1-service-reauthorize-review-sol.md` | implementer's pre-build review | Evidence for the plan corrections that shaped the build. |
| `wave1-service-reauthorize-review-opus.md` | independent adversarial review | Evidence for every A-item below; full reproductions live there. |

Where any source document disagrees with this file, this file wins. Where a
finding was overturned by a later pass, it is recorded in §5 so it is not
re-litigated.

Scope: ChronoAIProject/NyxID#1400 **item 1** only. Items 2 and 3 of that issue are
out of scope (item 3 shipped separately in #1404).

---

## 1. Status at a glance

| | Count | Blocking merge? |
| --- | --- | --- |
| Shipped and verified | 4 workstreams | — |
| **A. NyxID actions outstanding** | 3 major, 5 minor | 3 major: yes |
| **B. Aevatar prerequisites** | 4 code sites | **yes — hard gate** |
| **C. Verification gaps** | 2 | 1 should clear before merge |
| **D. Coordination (Calvin)** | 2 posts | not code-blocking |

**Overall: BLOCKED.** PR #1462 stays draft. Two independent gates — the Aevatar
consumer (§3) and the three MAJOR defects (§2) — must both clear.

---

## 2. Section A — NyxID actions outstanding

Severity, owner, and status per item. File references are on this branch unless
marked otherwise.

### A1 · MAJOR · open — user-controlled `Bearer …` strings permanently break the journey

**Where** (line numbers re-verified on `5b1189ee` for this document).
`backend/src/handlers/keys.rs:514` (`ws_frame_injections: Vec<WsFrameInjection>` —
a bare `Vec`, always serialized), `:509` (`default_request_headers`), `:417-418`
(`name` / `label`); `services/ws_frame_injector.rs:189-197` (the validator that
permits the offending template);
`frontend/src/components/assistant/blocks/action-card.tsx:110-128`
(`assertSecretFreeReadBack`).

**What.** Two recursive scanners run over the whole `/keys/{id}` body — Aevatar's
`RejectSecretBearingRead` and this PR's `assertSecretFreeReadBack` — both matching
`(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})`. NyxID's own WS-template
validator explicitly permits `{"headers":{"Authorization":"Bearer ${credential}"}}`
(it strips `${credential}` before checking, leaving `Bearer ` — no `nyxid_`, no JWT
segment). So NyxID stores a value it then refuses to read back. The service can
never be re-authorized through the assistant; the user sees only the generic
"NyxID could not verify this service for re-authorization" and is pointed at AI
Services, where nothing is visibly wrong.

Fail mode is closed, not leaky — no secret escapes. But this is reachable through a
documented, supported feature (CLAUDE.md §6), not a hypothetical.

**Fix — preferred.** Serve a minimal evidence projection for this read. Aevatar
consumes only `id`, `api_key_id`, `is_active`, `status`, `connection_status`,
`granted_scopes`, `last_authorized_at`; every other field on that response is pure
tripwire surface. Alternatives (reject `Bearer\s` at write time; browser-side error
differentiation) are documented in the Opus review §M1 and are strictly worse — the
write-time option does not repair rows already stored.

**Also required.** Extend `assert_aevatar_secret_free`
(`backend/src/handlers/keys.rs:2692`, called from
`key_response_always_serializes_authorization_evidence_properties` at `:2665`) to
run over a *fully populated* `KeyResponse` including a `Bearer ${credential}` WS
template. Today it runs over a near-empty response — no `ws_frame_injections`, no
`default_request_headers`, fixed test label — so it cannot fail. Written correctly
it would fail today, which is the point.

### A2 · MAJOR · open — `completed` is reported without checking scopes were granted

**Where.** `frontend/src/components/assistant/blocks/action-card.tsx:513-541`
(watch path), `:965-973` (dialog path).

**What.** Both completion paths report `completed` on identity match + `status ==
active` + `last_authorized_at` advancement. Neither reads `granted_scopes`. Aevatar
*does* check (`NyxIdActionPostconditionPort.cs:294-296`), so the model is not
misled — it receives `MismatchCode`. **The user is**: the card renders the
`Re-authorized` badge while the conversation proceeds as though nothing happened.

Three reachable paths: the user deselects a prefilled scope before clicking Connect
(`prefillScopes` only seeds the `useState` initializer,
`add-key-dialog.tsx:1530-1536`, nothing pins it after); the provider grants a subset
(unapproved GitHub org access, unchecked Google consent scopes, pending Lark review);
or the provider omits `scope` from the token response, in which case the backend
deliberately preserves the previous `token_scopes`
(`services/user_api_key_service.rs:568-580`, per NyxID#917) while still stamping
`last_authorized_at` — so the freshness gate passes on a completely unchanged grant.

**Fix.** In both completion paths, re-read `/keys/{id}` and require
`requested_scopes ⊆ granted_scopes` (case-sensitive, matching Aevatar's
`StringComparer.Ordinal`) before `report("completed", …)`. On shortfall call
`onBlock` naming the missing scopes.

### A3 · MAJOR · open — the freshness gate proves "some authorization advanced", not "this one"

**Where.** `frontend/src/hooks/use-keys.ts:213-216`;
`frontend/src/components/dashboard/add-key-dialog.tsx:1764, 1792-1794, 3191-3203`.

**What.** The predicate is a bare timestamp inequality with no server-side
correlation to the attempt. `attemptId` is a client-side `crypto.randomUUID()` used
only as a TanStack Query cache generation (`use-keys.ts:154`); it never reaches the
server. The baseline comes from a **stale snapshot** — `ensureAuthKey` returns the
`reconnectKey` prop verbatim, i.e. the object fetched at *card-click* time, which
may be minutes old by the time the user presses Connect.

Consequence: any unrelated fresh authorization of the same key inside that window —
the AI Services page, a second tab, `nyxid service scopes`, the CLI wizard, a second
assistant card for the same service — satisfies the gate and settles the card as
`completed`. Two concurrent cards for one service share a baseline; completing
either settles both.

Combined with A2 this is the real false-`completed` surface: not a background
refresh (that path is genuinely closed — see §5), but *someone else's*
authorization being claimed as this attempt's.

**Fix.** Correlate to the server-side attempt. `initiateOAuthAsync` already returns
an `attempt_nonce` and the flow threads a `connection_id`; settle on evidence that
*this* attempt landed. If that is too large for this wave, at minimum re-read
`/keys/{id}` inside `handleConnect` and derive the baseline from that fresh read —
closes the card-click→Connect-click window, does not close the concurrent-flow case.

### A4 · MINOR · open — PR body misattributes the freshness fix

The disposition table claims the freshness mechanism was delivered here. All three
parts pre-exist on `origin/main` (`stores/pending-connect-store.ts:12`,
`action-card.tsx:285`, `hooks/use-keys.ts` ×11); neither `use-keys.ts` nor
`pending-connect-store.ts` is in this PR's diffstat. What this PR actually adds on
that axis is the exact-service **identity** guard (`action-card.tsx:513-525`) —
real and correct, but a different control.

Sol's review was accurate about its pre-rebase branch point; the PR body carried
"Fixed" forward without re-checking what the rebase brought in.

**Fix.** Correct the table entry to "already present on `main`; this PR adds the
exact-service identity guard and relies on the inherited baseline gate."

### A5 · MINOR · open — the freshness test does not test freshness

`action-card.test.tsx`, "waits for a fresh authorization timestamp before
auto-completing reauthorization". Delete the entire freshness gate from
`use-keys.ts` and this test still passes — the assertion its name promises (an
unchanged `last_authorized_at` must **not** settle the card) is absent.

**Fix.** Add a case whose key read always returns the original
`last_authorized_at`; assert `onResolve` is never called and the card reports the
timeout note.

### A6 · MINOR · open — browser scope grammar is stricter than the published contract

`frontend/src/schemas/assistant-actions.ts:49-56` enforces
`/^[A-Za-z0-9._:/~+*=-]+$/` and `.min(1)`. The published `params_schema` says
`{"type": "string"}` and permits an empty array; RFC 6749 §3.3 allows every
printable ASCII except space, `"` and `\`. A conforming scope outside that regex
parses at Aevatar, then degrades to `{variant: "unknown"}` → "Unsupported action
request" with no indication which scope was rejected.

No real catalog provider trips it (Google, GitHub, Slack, Lark, Microsoft, Zoom,
HubSpot, Atlassian all pass), so likelihood is low.

**Fix.** Document the divergence in `docs/chat/06-actions-registry.md` — the
published schema is the contract and this restriction is invisible from it.

### A7 · MINOR · open — org-admin preflight guard fails open

`action-card.tsx:79-88, 158-163` declares `credential_source` `.optional()`, but
`KeyResponse.credential_source` is mandatory (`keys.rs:544-546`), so the guard is
currently unreachable. An `.optional()` schema means it silently vanishes if the
response shape changes — the opposite of defence in depth.

**Fix.** Make it required so a shape change fails loudly. (The backend remains the
real gate.)

### A8 · MINOR · open — preflight fetches the whole catalog for one entry

`action-card.tsx:167-169` does `GET /catalog?include_all=true` then finds one slug.
`GET /api/v1/catalog/{slug}` returns the same `provider_type` /
`provider_config_id` / `device_code_format`. One full-catalog fetch per card click.

---

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

Plus: `docs/contracts/nyxid-assistant-conformance/v1/registry-v8.json` matching the
published manifest, and registry/conformance test updates.

**Rollout order (binding).**
1. Aevatar merges + deploys the additive v8 consumer.
2. Restart an Aevatar process while NyxID still serves **v7**; prove the registry
   still loads with all current actions.
3. Only then NyxID merges + deploys v8.
4. Restart another Aevatar process, prove it loads v8, run one end-to-end
   `service.reauthorize`.

---

## 4. Section C — verification gaps

### C1 · open · should clear before merge — nobody has run the full backend suite

The implementer reported 5,189 passed / 129 failed with all failures attributed to
a missing MongoDB replica set (`NYXID_TEST_DATABASE_URL` unset, Docker unavailable),
and explicitly did **not** claim green — the correct posture. The reviewer could not
reproduce it for the same reason. So the split and its attribution are unverified in
both directions, and **no full backend run exists for this change**.

**Action.** Someone with a replica set runs `cargo test` before merge.

Everything else reproduced exactly on an independent re-run:

| Command | Result |
| --- | --- |
| `npm --prefix frontend run test` | 246 files / 2,847 passed, 0 failed |
| `npm --prefix frontend run lint` | 0 errors, 23 pre-existing warnings |
| `npm --prefix frontend run build` (CI parity: `tsc -b`) | passed — 3,470 + 107 modules |
| `cargo test assistant_actions` | 5 passed, 0 failed |
| `cargo test unified_key_service` | 163 passed, 0 failed |
| `cargo fmt --check` / `clippy -D warnings` | passed |
| `cargo test -p nyxid-cli` | 1,100 + 1 wizard-freshness passed |

No wizard-bundle rebuild is required: `add-key-dialog.tsx` is not in
`cli/src/wizard/bundle-meta/index.manifest`.

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

**Conclusion.** Item 1 is fully addressed in scope and contract. It is not yet
*complete* — completion requires §2 A1–A3 fixed and §3 B1–B4 shipped and deployed
by Aevatar. The issue's framing that this "extends verb coverage beyond
`service.connect`" is now accurate for all three Wave-1 verbs.

Items 2 (exact-service non-blocking approval) and 3 (facade v4 conformance) of
#1400 are untouched by this work; item 3 shipped in #1404.
