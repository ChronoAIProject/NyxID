# Wave 1 — `service.reauthorize` implementation plan (issue #1400, item 1)

> **Superseded as an instruction set.** The live action list is
> [`wave1-service-reauthorize-actions.md`](./wave1-service-reauthorize-actions.md).
> This document is retained as the **contract reference** — §1 (target contract)
> and §5 (rollout order) remain accurate and are cited from the action list. Its
> §3 task breakdown was executed and partly overridden during implementation; do
> not work from it. Several §0 claims were amended by later passes — see the
> action list §5 for what did and did not survive.

**Status:** plan only, nothing implemented. Written 2026-08-17 against NyxID `origin/main`
`ed372d8c` and Aevatar `origin/feature/integrate` `05db6b4b0` (clone at
`~/Desktop/aelf-frontend-work/aevatar`; read files with
`git show origin/feature/integrate:<path>`, do not trust the checkout).

**Scope:** issue #1400 item 1 reduces to exactly one verb. `key.create` and `key.rotate`
already shipped in NyxID revision `nyxid-assistant-actions.v7`
(`backend/src/handlers/assistant_actions.rs` on `origin/main`:
`ASSISTANT_ACTIONS_SCHEMA_VERSION = 4`, `ASSISTANT_ACTIONS_REVISION = "nyxid-assistant-actions.v7"`,
descriptors for `service.connect`, `key.create`, `key.rotate`; `service.reauthorize` appears only
in the test module's 14-verb `SUPPORTED_ACTIONS` allowlist). What remains is publishing the
`service.reauthorize` descriptor, closing three small evidence-serialization gaps on
`GET /api/v1/keys/{id}`, the facade acceptance check, and the browser journey.

---

## 0. Corrections to the issue text and the briefing (verified 2026-08-17)

Read these before anything else; they change the shape of the work.

1. **The evidence endpoint is `GET /api/v1/keys/{id}`, not `/api/v1/user-services`.**
   Aevatar's `NyxIdActionEvidenceReadPort.GetUserServiceAuthorizationAsync` calls
   `client.GetServiceAsync(bearerToken, userServiceId)`, and in
   `src/Aevatar.AI.ToolProviders.NyxId/NyxIdApiClient.cs`:

   ```csharp
   public Task<string> GetServiceAsync(string token, string id, CancellationToken ct) =>
       GetAsync(token, $"/api/v1/keys/{Uri.EscapeDataString(id)}", ct);
   ```

   (`/api/v1/user-services` at `NyxIdApiClient.cs:1027` backs a *different* parser,
   `ParseUserServicesDocument`, which is not used for reauthorize evidence.)

2. **All six evidence facts already exist on the `KeyResponse`** returned by
   `GET /api/v1/keys/{id}` (shipped with the NyxID#917 manage-scopes work). This is a
   serializer-level change, not a model or write-path change. Details in §2.

3. **Aevatar does not "pin v7" as a single value.** `NyxIdAssistantActionRegistry.cs`
   keeps a *map* of accepted revisions — `IsSupportedRegistryRevision` accepts
   `nyxid-assistant-actions.v4/v5/v6/v7` plus the Aevatar-owned
   `aevatar-nyxid-actions.v1`. Two separate maps matter:
   - `PinnedActionsByRevision`: `service.reauthorize` appears only in **v5**'s pinned set.
   - `ExecutableActionsByRevision`: `service.reauthorize` appears in **no revision's**
     executable set (v7 = `{service.connect, key.create, key.rotate}`).

   So even though Aevatar's parser, schema constant, postcondition port, browser-action
   producer, wire mapper, audit translators, and tests for `service.reauthorize` are all
   built, **no revision makes the verb executable today**.

4. **No v8 exists anywhere in Aevatar.** `docs/contracts/nyxid-assistant-conformance/v1/`
   holds fixtures `registry-v4.json` … `registry-v7.json` only; a grep for `v8` across
   `agents/Aevatar.GAgents.NyxidChat`, the registry tests, and the conformance directory
   returns nothing. The Aevatar-side consumer for the new revision **must be filed and
   shipped as a prerequisite** — see §5.

5. **Blast radius of publishing early, precisely:** Aevatar's startup gate
   (`NyxIdAssistantActionRegistryStartup.cs`) fetches `GET /api/v1/assistant/actions`
   once at process start; on any load failure (including an unknown revision) it logs
   `"NyxID Assistant action registry startup failed … Assistant actions are disabled for
   this process"` and initializes `NyxIdAssistantActionRegistry.CreateDisabled()`.
   Chat itself survives, but **every assistant action card (connect, key.create,
   key.rotate) stops working** on Aevatar processes started after the NyxID deploy.
   Deployment order is therefore binding (§5) even though "chat breaks" overstates it.

6. **Param shape is settled and must be byte-exact.** Aevatar's
   `ValidatePinnedContract` parses NyxID's published `params_schema` and requires
   `JsonNode.DeepEquals` with its pinned constant, plus exact `risk` and
   `remember_eligible` matches. Do not add `minItems`/`uniqueItems`/`maxLength` or any
   other refinement to the published schema — it would fail the deep-equality pin.

---

## 1. Target contract (what Aevatar already enforces — do not re-open)

### 1.1 Params schema (publish exactly this)

`agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistry.cs`,
`ServiceReauthorizeParamsSchema`:

```json
{
  "type": "object",
  "additionalProperties": false,
  "required": ["userServiceId", "requestedScopes"],
  "properties": {
    "userServiceId": {"type": "string"},
    "requestedScopes": {"type": "array", "items": {"type": "string"}}
  }
}
```

Pinned metadata: `risk = grant`, `remember_eligible = false` (registry entry
`["service.reauthorize"] = new(…, NyxIdAssistantActionRisk.Grant, false)`), tier `v1`.

At postcondition time Aevatar additionally requires `requestedScopes` to be a
**non-empty set of unique, normalized strings** (`ValidNormalizedSet(…, requireAny: true)`,
`Ordinal` dedup). The NyxID journey should refuse to launch with an empty or duplicated
scope list even though the published schema cannot say so.

### 1.2 Descriptor text (already authored — reuse verbatim)

Aevatar's `registry-v5.json` fixture carries the full descriptor eanz17 authored on the
NyxID draft branch `feat/2026-08-10_wave1-action-registry` (still on origin at
`ee3b8f81e36647c67a06505b10ec0444ab064000`):

```json
{
  "action": "service.reauthorize",
  "description": "Ask the user's browser to re-authorize an existing connected service and review its requested scopes. Use when a task needs permissions that the referenced user service does not currently grant. NyxID owns the authorization journey and credential storage, and reports only a safe user-service reference. Never ask the user for keys, tokens, passwords, or authorization codes in chat.",
  "params_schema": { …exactly §1.1… },
  "risk": "grant",
  "tier": "v1",
  "remember_eligible": false
}
```

Ordering precedent (v5 fixture): `["service.connect", "service.reauthorize", "key.create", "key.rotate"]`
— insert the new descriptor at **index 1**, after `service.connect`.

### 1.3 Postcondition verify (what "success" means to Aevatar)

`agents/Aevatar.GAgents.NyxidChat/NyxIdActionPostconditionPort.cs`,
`VerifyServiceReauthorizeAsync`, after reading `GET /api/v1/keys/{userServiceId}` with the
human session's NyxID bearer (`AgentToolHumanSessionNyxIdCredential.ResolveBearerToken` —
the same credential already used for the shipped `key.create`/`key.rotate` evidence reads
against `GET /api-keys/{id}`):

- `evidence.UserServiceId == params.userServiceId` (Ordinal)
- `evidence.IsActive == true`
- `evidence.CredentialStatus == Active`
- `evidence.OAuthConnectionStatus == Active`
- `evidence.GrantedScopes != null` and contains **every** requested scope (Ordinal)
- `evidence.LastAuthorizedAtUtc != null`, `>= requestedAt` (the action's request time)
  and `<= now`

Any resource hint on the completion report must be `userService` (or absent) and, when
present, must equal `params.userServiceId`.

### 1.4 Evidence parser field contract (`NyxIdApiAccessContracts.cs`,
`ParseUserServiceAuthorizationDocument`)

On the JSON **root** of the `GET /api/v1/keys/{id}` response:

| JSON field | Requirement |
|---|---|
| `id` | required, non-empty normalized string |
| `api_key_id` | optional normalized string |
| `is_active` | required boolean |
| `status` | required string, exactly one of `active` / `expired` / `revoked` / `failed` / `refresh_failed` / `pending_auth` |
| `connection_status` | **property must be present**; `null` → `Unspecified` (fails verify), else exactly `"active"` or `"expired"` |
| `granted_scopes` | **property must be present**; `null` allowed, else array of unique non-empty normalized strings (duplicates → malformed) |
| `last_authorized_at` | **property must be present**; `null` allowed, else RFC3339 (`Z` or `±HH:MM` offset, ≤9 fractional digits — chrono's `to_rfc3339()` conforms) |

An absent required property, an out-of-domain `status`, or duplicate scopes makes the
whole read **malformed** (`ProviderReadFailure`), not merely unverified.

**Secret-scan tripwire:** the parser recursively walks the entire response and throws if
any field's lowercased-alphanumeric name is in
`{apikey, fullkey, keyhash, credential, credentials, accesstoken, refreshtoken,
authorization, cookie, cookies, secret, secrets, clientsecret, password, token,
passphrase, usercode, devicecode, rawbody, rawupstreambody}`, or any string value matches
`(?:Bearer\s+\S+|nyxid_(?:ag_)?[A-Za-z0-9_-]{16,})` (case-insensitive). Current
`KeyResponse` field names all normalize clear of the set (`api_key_id` → `apikeyid`,
`credential_type` → `credentialtype`, etc.). **Never add a field whose normalized name
lands in that set to `KeyResponse`.**

---

## 2. Evidence-field answer: current state of `GET /api/v1/keys/{id}` on `origin/main`

Handler: `backend/src/handlers/keys.rs` `get_key` (lines 1090–1161, route
`GET /api/v1/keys/{key_id}`), which returns `KeyResponse` at the JSON root (no envelope;
`Ok(Json(response))`) via `unified_key_service::get_key` → `build_key_view`
(`backend/src/services/unified_key_service.rs`, `KeyView` struct at line 392, response
mapping `key_response_from_view` in `keys.rs` lines 2280–2360). Accepts UUID **or slug**
(`find_user_service_for_actor`, keys.rs:115–160) and enforces personal/org ACL through
`resolve_key_read_owner` → `org_service::resolve_owner_access` (keys.rs:214–228;
unauthorized → NotFound, no metadata leak). Aevatar passes the `userServiceId` (a UUID),
which hits the `_id` branch.

Freshness stamping is covered on **all three** fresh-authorization write paths
(`user_api_key_service.rs`): multi-connection `write_oauth_tokens_to_key` (line 484),
the chat connect-card popup's `write_chat_oauth_tokens_to_key` (line 569), and the
legacy fan-out `sync_provider_token_to_api_keys_after_authorization` (line 337, invoked
with `fresh_authorization = true` only from the OAuth callback,
`user_tokens.rs:1467-1505`). Token refresh, disconnect, and manual sync deliberately do
NOT stamp — so the `last_authorized_at`-advance completion signal cannot false-fire.

Field-by-field against §1.3/§1.4 — **all six facts exist; the underlying Mongo data
exists for all of them; the gaps are serialization/derivation only:**

| Verify condition | Field on `KeyResponse` | State | Gap |
|---|---|---|---|
| identity match | `id: String` | ✅ always serialized | none |
| `IsActive` | `is_active: bool` | ✅ always serialized | none |
| `CredentialStatus == Active` | `status: String` | ✅ always serialized; value domain on `models/user_api_key.rs:68` is exactly the six values Aevatar's parser accepts | none |
| `OAuthConnectionStatus == Active` | `connection_status: Option<String>` | ⚠️ exists (`unified_key_service::oauth_connection_status`, line 3803) | **(a)** `skip_serializing_if = "Option::is_none"` → property absent when `None`, parser requires presence. **(b)** returns `None` for an `oauth2` key whose `expires_at` is `None` — i.e. expiry-less providers (GitHub-class OAuth apps) can **never** verify. Both fixable in the serializer/derivation; no new data needed (`credential_type`, `status`, `expires_at`, `refresh_token_encrypted` are all on the Mongo row). |
| `GrantedScopes ⊇ requested` | `granted_scopes: Option<Vec<String>>` | ⚠️ exists — `build_key_view` splits `UserApiKey.token_scopes` on whitespace (line ~3777); `token_scopes` is written by the OAuth callback | **(a)** same absent-when-`None` issue. **(b)** no dedup — Aevatar rejects duplicate entries as malformed. **(c)** caveat: `write_oauth_tokens_to_key` deliberately *preserves* the old `token_scopes` when the provider's token response omits `scope`; for such providers a scope-adding reauth may verify against stale scopes or fail the superset check (see Open Question Q3). |
| `LastAuthorizedAt` fresh | `last_authorized_at: Option<String>` | ⚠️ exists — `UserApiKey.last_authorized_at` (model doc `models/user_api_key.rs:72-80`) is stamped **only** by `user_api_key_service::write_oauth_tokens_to_key` (line 457; `"last_authorized_at": &now` in the `$set`) on a fresh OAuth/device-code callback, never by token refresh — exactly the freshness semantics Aevatar checks. `to_rfc3339()` output is parser-compatible. | same absent-when-`None` issue only |
| (optional) | `api_key_id: Option<String>` | ✅ present when the credential row resolves | none (parser treats as optional) |

**Bottom line:** no model change, no write-path change, no migration. The work is:
always-serialize three fields (emit `null` instead of omitting), fix the
`oauth_connection_status` derivation for expiry-less tokens, and dedup the scope split.

---

## 3. NyxID task breakdown

Execute in this order. Backend first (tasks B1–B3 are independent of the frontend and of
Aevatar timing — only the **merge/deploy** of B1 is gated, see §5).

### B1. Publish the `service.reauthorize` descriptor + revision bump

File: `backend/src/handlers/assistant_actions.rs`.

1. Insert a descriptor struct literal at **index 1** (between `service.connect` and
   `key.create`) with:
   - `action: "service.reauthorize"`
   - description: the exact §1.2 text
   - params schema: the exact §1.1 JSON
   - `risk = grant`, `tier = v1`, `remember_eligible = false`
   Match the existing descriptor struct/format in the file — the shipped `key.create`
   entry is the pattern (grant / v1 / false). Consult the draft branch
   `feat/2026-08-10_wave1-action-registry` @ `ee3b8f81` for eanz17's original authoring
   (it was written against v5; **port the one descriptor, do not merge the branch** —
   the manifest has since moved to v7 with the least-scope `key.create` schema).
2. Bump `ASSISTANT_ACTIONS_REVISION` from `"nyxid-assistant-actions.v7"` to the revision
   string agreed with Aevatar (expected `"nyxid-assistant-actions.v8"` — Open Question
   Q1). `ASSISTANT_ACTIONS_SCHEMA_VERSION` stays `4` (the frontend gate
   `resolveAssistantAction` requires `schemaVersion === 4`, and Aevatar's fixtures pin
   `schema_version: 4`).
3. Descriptor mechanics (verified): descriptors are `AssistantActionDescriptor` struct
   literals (lines 22–30: `action`, `description`, `params_schema: Value` via `json!`,
   `risk`, `tier`, `remember_eligible`) inside a `static MANIFEST_BODY: LazyLock<String>`
   (line 33); descriptions are `const` strings (`SERVICE_CONNECT_DESCRIPTION` line 11).
   Add a `SERVICE_REAUTHORIZE_DESCRIPTION` const with the §1.2 text and a `json!` schema
   matching §1.1 exactly.
4. Update the test module (lines 147–541; all four fns enumerated and verified on
   `origin/main`):
   - `assistant_actions_manifest_matches_golden_payload` (459–491): pins the revision
     string (line 465), `actions.len() == 3` (line 466), per-index
     id/risk/tier/remember_eligible/params_schema assertions in order, and a full
     `assert_eq!(manifest, golden_manifest())` (line 490). Update: revision, count → 4,
     shifted index assertions (`actions[1]` becomes `service.reauthorize`), a new
     `service_reauthorize_params_schema()` test helper, and a new entry in
     `golden_manifest()` (lines 276–307).
   - `assistant_actions_manifest_conforms_to_aevatar_parser_contract` (493–496) →
     `assert_manifest_conforms` (387–457): the new descriptor passes automatically if
     schema-compliant — every action must be in `SUPPORTED_ACTIONS` (line 418, already
     contains `service.reauthorize` at line 163), objects need
     `additionalProperties: false`, property names are checked against
     `FORBIDDEN_SECRET_NAMES` (lines 177–196), risk ∈ {low, grant, destructive}, tier
     must be `"v1"`. `userServiceId`/`requestedScopes` are clean of the forbidden-name
     set. No parser/grammar change.
   - `secret_name_normalization_is_ascii_alphanumeric_only` (498–503): untouched.
   - `assistant_actions_route_is_public_json_and_matches_static_body` (505–541):
     inherits the golden update automatically.
5. Bump every other consumer of the revision string (whole-tree grep verified — this is
   the complete list): `docs/chat/06-actions-registry.md` lines 16 (sample payload),
   119 (composition-snapshot field table), 190 (validation rule
   `revision == "nyxid-assistant-actions.v7"`). (`docs/assistant/TRAVEL_BOOKING.md:36`
   references v4 and is already stale — optional.) No frontend/CLI consumer pins the
   revision string.

**Acceptance:** `cargo test assistant_actions` green; manifest JSON for the new revision,
filtered to `service.reauthorize`, deep-equals the §1.2 object; action order is
`[service.connect, service.reauthorize, key.create, key.rotate]`.

### B2. Evidence serialization fixes on `KeyResponse`

Files: `backend/src/handlers/keys.rs`, `backend/src/services/unified_key_service.rs`.

1. Remove `#[serde(skip_serializing_if = "Option::is_none")]` from exactly these three
   `KeyResponse` fields so they serialize as `null` when `None`:
   `connection_status`, `granted_scopes`, `last_authorized_at`.
   Leave every other optional field as-is. Note `KeyResponse` is also used by
   `list_keys`, `create_key`, `update_key`, `delete_key` responses — the change is
   intentionally global to the type (Aevatar's `/keys` list parser
   (`ParseUserServiceKeysDocument`) does not read these three fields, so list responses
   are unaffected on the Aevatar side).
2. `oauth_connection_status` (unified_key_service.rs:3803): for a `credential_type ==
   "oauth2"` key in a non-terminal status with `expires_at == None`, return
   `Some("active")` instead of `None` (an OAuth credential without a recorded expiry is
   live, not unknown). Terminal statuses keep returning `"expired"`; non-OAuth keys keep
   returning `None` (serialized as `null` after step 1 — which Aevatar maps to
   `Unspecified`, correctly failing reauthorize verify for non-OAuth services).
3. `build_key_view` granted-scopes split (~line 3777): dedup preserving first-occurrence
   order after `split_whitespace` (Aevatar hard-rejects duplicates).
4. Unit tests: extend the existing `oauth_connection_status` tests (~line 3988) with the
   no-expiry case; extend `build_key_view_parses_granted_scopes_from_token_scopes`
   (~5841) with a duplicate-scope input; add a serialization test asserting the three
   fields are present-as-null on a minimal `KeyResponse`.

**Frontend/CLI fallout check (required in the same PR):**
- `frontend/src/schemas/` key schemas: any `granted_scopes` / `last_authorized_at` /
  `connection_status` declared `.optional()` must become `.nullish()` (or equivalent) —
  `null` now arrives where the property used to be absent. Also check the strict
  read-back snapshot schemas used by the assistant key dialogs
  (`assistant-key-create-dialog.tsx` `verifyAllowedServices` and the rotate dialog's
  read-back) — they parse `GET /keys/{id}` / `GET /api-keys/{id}` responses strictly and
  may reject unexpected `null`s.
- CLI: serde `Option<T>` handles `null` natively; no change expected, but run
  `cargo test -p nyxid-cli`.
- > TODO — not investigated: whether any frontend zod schema for keys uses `.strict()`
  > with `.optional()` on these fields. Grep `granted_scopes` and `last_authorized_at`
  > under `frontend/src/schemas/` and `frontend/src/components/` before assuming.

**Acceptance:** backend suite green (`cargo test` — see §6 test-environment note);
frontend `npm --prefix frontend run build && npm --prefix frontend run test` green; a
manual `curl` of `GET /api/v1/keys/{id}` for (a) an OAuth key, (b) a plain API key shows
all three properties present (value or `null`), and an expiry-less OAuth key shows
`"connection_status": "active"`.

### B3. Facade: completed `userService` report for `service.reauthorize` — **no code change needed (verified)**

Parsing lives in `backend/src/services/assistant_service.rs` (NOT `handlers/assistant.rs`
— the handler at `assistant.rs:986` just calls
`assistant_service::parse_assistant_chat_command(&bytes)?`). Verified on `origin/main`:

- `ActionResource` enum (lines 471–479) already has all six variants;
  `parse_action_resource` (lines 748–798) accepts `{"userService":{"userServiceId": …}}`
  with `validate_control_identity` hardening.
- **There is no action-id → resource-kind gating map.** `RawActionReport` (lines
  550–558) carries only `action_request_id`, `origin_turn_id`, `disposition`,
  `resource` — the action id never appears in the report. The only completed-report rule
  (lines 900–906) is that a `completed` disposition must carry *some* resource. A
  completed `service.reauthorize` report with a `userService` resource **parses and
  forwards today**. Test `completed_action_reports_round_trip_each_safe_resource_variant`
  (lines 2124–2156) already round-trips all six variants.

Remaining (optional, cheap): no new backend test is strictly required; if adding one,
extend the existing round-trip test's comment to note `service.reauthorize` relies on
the `userService` variant. Do not invent per-action gating that doesn't exist.

**Acceptance:** none beyond the existing suite — `cargo test assistant_service` green.

### F1. Frontend: registry descriptor + params schema

Files: `frontend/src/schemas/assistant-actions.ts`,
`frontend/src/lib/assistant/action-registry.ts`.

1. `assistant-actions.ts`: add `serviceReauthorizeActionParamsSchema` — strict object
   `{ userServiceId: requiredActionIdentitySchema, requestedScopes: <array of
   actionControlIdentitySchema, min 1, max ~64, unique> }` (mirror
   `keyCreateActionParamsSchema`'s allowedServiceIds constraints; the *frontend* may be
   stricter than the published schema — Aevatar's postcondition requires non-empty
   unique anyway). Add the params union member and a new `ActionCardParams` variant
   (suggested `variant: "service_reauthorize"`, snake_case fields per existing pattern).
   No `ActionResource` change needed (`userService` already exists).
2. `action-registry.ts`: add `serviceReauthorizeDescriptor` following the existing
   pattern —

   ```ts
   const serviceReauthorizeDescriptor: ActionDescriptor = {
     title: () => "Re-authorize service",
     body: (params) =>
       params.variant === "service_reauthorize"
         ? `NyxID will re-authorize this connected service with the requested permissions. Your credential stays in NyxID and is never shared with the model.`
         : "NyxID will re-authorize one exact connected service.",
     cta: () => "Re-authorize",
     risk: "credential_access",
     journey: (params) =>
       params.variant === "service_reauthorize" ? "service_reauthorize" : null,
   };
   ```

   Register under `"service.reauthorize"` in `ACTION_REGISTRY`, add the
   `normalizeParams` branch (schema `safeParse` → variant, `{variant:"unknown"}` on
   failure), and extend the `ActionJourney` union with `"service_reauthorize"`.
   Copy tone/length from the existing descriptors; final copy per DESIGN.md voice.

### F2. Frontend: the journey (reuse the reconnect flow — do not invent an OAuth path)

The existing re-authorization UX is `AddKeyDialog`'s **reconnect mode**
(`frontend/src/components/dashboard/add-key-dialog.tsx`, `reconnectKey` prop): it reuses
the key's `connection_id`, seeds/locks already-granted scopes via `OAuthStep`
(`grantedScopes` / `lockedScopes`, scope UI in
`frontend/src/components/shared/upstream-scope-picker.tsx`), starts the flow with
`useInitiateOAuth()` (`frontend/src/hooks/use-providers.ts:111` —
`GET /providers/{providerId}/connect/oauth?scope_override=…&key_id=…&flow=cc`), runs the
managed popup from `frontend/src/lib/oauth-popup.ts` (#1349), and completion is detected
by polling `GET /keys/{id}` until `status === "active"` **and** `last_authorized_at`
advances past the pre-flow baseline (`useKeyAuthorizationWatch` /
`useKeyAuthorizationStatus`, `frontend/src/hooks/use-keys.ts:56-240`, with
`previousAuthorizationAt` captured before launch). The chat `connect-card.tsx` already
drives exactly this for its `NYXID_UNAUTHORIZED` reconnect banner, and
`action-card.tsx`'s `service.connect` journey already handles the out-of-band completion
via `pending-connect-store` + auto-resolve.

Implementation in `frontend/src/components/assistant/blocks/action-card.tsx` (plus a new
`assistant-service-reauthorize-*` wrapper if the logic warrants its own file):

1. On CTA for `variant === "service_reauthorize"`:
   a. Resolve the key: `GET /keys/{userServiceId}` with the same secret-free read-back
      assertion pattern the key dialogs use (`assertSecretFreeReadBack` + a minimal
      strict snapshot: `is_active`, OAuth-manageable — `credential_type === "oauth2"` or
      `auth_method` `oauth2|oidc`, mirroring `connect-card.tsx`'s `reconnectKey`
      predicate). Not found / not OAuth / org-managed-not-manageable → block the card
      with a typed note (never report completed).
   b. Capture `previousAuthorizationAt = key.last_authorized_at` and open `AddKeyDialog`
      with `reconnectKey={key}`, `launch="popup"`, `flow="cc"`, and the **requested
      scopes pre-selected**.
2. **`AddKeyDialog`/`OAuthStep` extension (the one new capability):** a prop
   (suggested `prefillScopes?: string[]`) that unions the action's `requestedScopes`
   into the initial scope-picker selection on top of `grantedScopes`. Unknown scope
   strings (not in the catalog's scope list) must still be sent — pass through to
   `scope_override` — or, if the picker cannot represent them, block the journey with a
   clear note rather than silently dropping a requested scope (a dropped scope means
   Aevatar's superset check fails and the action reads as a broken promise).
   > TODO — not investigated: whether `upstream-scope-picker.tsx` supports free-form /
   > unknown scope entries today. Check before choosing between "pass through" and
   > "block".
3. Completion: reuse the `service.connect` out-of-band pattern — write the attempt into
   `pending-connect-store`, run `useKeyAuthorizationWatch` with the
   `previousAuthorizationAt` baseline; on `authorized` report
   `report("completed", { userService: { userServiceId } })`; on failure → `onBlock`
   with the backend reason; on `connectWatchDeadline` timeout → timeout note. Dedupe /
   origin-turn rules come free from the existing `actionContinueBodySchema` path.
4. Do NOT gate completion on the granted scopes matching client-side — Aevatar's
   postcondition is the authority (§1.3). The card reports the journey outcome; the
   verify happens server-side on Aevatar with fresh evidence.

**UI constraints (DESIGN.md, read it before building):** compact dark UI; purple
(`nyx-secondary-400` / `variant="primary"`) only on the primary CTA; dialogs `p-5 gap-4
rounded-xl`, title `text-[15px] font-semibold`; buttons `h-8 text-[12px] rounded-lg`
with `isLoading`; 12/11/10px type scale; badges `text-[10px] … rounded-md` tint style;
scope lists in the existing picker component; ids in `font-mono text-[12px]`; no colored
top accent rail (the action-card tests assert this). Caveat: the live `frontend/src/app.css`
(+ Mona Sans) is the source of truth for concrete hex/font values; DESIGN.md is stale on
those — follow DESIGN.md for structure/density, verify hex/fonts against `app.css`.

**Tests (mirror the existing suites):**
- `assistant-actions.test.ts`: params parsing (valid, empty scopes rejected, dup scopes
  rejected, control-identity rules), unknown-variant fallback, resource-variant pairing
  for the new verb.
- `action-card.test.tsx` additions: journey opens `AddKeyDialog` in reconnect mode with
  requested scopes pre-selected; popup handoff; auto-completion when the watch sees
  `last_authorized_at` advance; failure/timeout notes; reports only
  `{userService:{userServiceId}}`; non-OAuth key blocks the card.
- `add-key-dialog.test.tsx`: `prefillScopes` union behavior (incl. unknown-scope
  handling per the decision above).

**Acceptance:** `npm --prefix frontend run test`, `run lint`, `run build` (note: CI runs
`build` = `tsc -b` with `noUncheckedIndexedAccess`; `tsc --noEmit` passing is NOT
sufficient).

### C1. Coordination artifacts (part of this work, not optional)

1. Comment on ChronoAIProject/NyxID#1400 stating: item-1 residual scope =
   `service.reauthorize` only; NyxID-side branch/PR link; the deployment-order
   constraint (§5) restated; the evidence-endpoint correction (§0.1).
2. File (or ask eanz17 to file) the **Aevatar prerequisite issue** (consumer for the new
   revision — §5 list), referencing aevatar#3312 (the Wave-1 consumer issue named in
   #1400) and aevatar#3315 item 6 (param-shape confirmation — answer: settled as
   `{userServiceId, requestedScopes[]}`, already implemented on `feature/integrate`).
3. Mark the NyxID PR **DO-NOT-MERGE** until the Aevatar consumer is deployed and
   verified against the live v7 manifest.

---

## 4. What does NOT need building (verified already shipped / already built)

- Aevatar `service.reauthorize` machinery: `ServiceReauthorizeParamsSchema`,
  `ParseServiceReauthorize`, `VerifyServiceReauthorizeAsync`, browser-action producer,
  AGUI frame builder, TaskPlan wire mapper, audit translators, state projector, tests —
  all present on `origin/feature/integrate`. Blocked only by the revision gate.
- NyxID `last_authorized_at` freshness semantics (`write_oauth_tokens_to_key` stamps on
  fresh authorization only, not refresh) — exactly what the verify needs; no change.
- The manage-scopes reconnect UX, managed OAuth popup (#1349), key-authorization
  watch/polling, pending-connect store, six-variant safe-resource union, and the
  `service.connect` completed-`userService` report path — all shipped on `origin/main`.
- The 14-verb `SUPPORTED_ACTIONS` parser allowlist already includes
  `service.reauthorize` (backend test module) — no parser change.

---

## 5. Deployment order (binding) and the Aevatar prerequisite

**Order: Aevatar consumer first, NyxID manifest second.** Rationale in §0.5 — a NyxID
deploy publishing an unknown revision flips every Aevatar process (started thereafter)
to a disabled action registry: all assistant actions dark, silently.

The Aevatar-side prerequisite (does not exist today; §0.4) is, on
`aevatarAI/aevatar` `feature/integrate`:

1. New revision constant (expected `nyxid-assistant-actions.v8`) accepted by
   `IsSupportedRegistryRevision`.
2. `PinnedActionsByRevision[v8] = {service.connect, service.reauthorize, key.create, key.rotate}`.
3. `ExecutableActionsByRevision[v8]` = same four (this is what finally makes the verb
   executable).
4. Pinned-schema wiring in `ValidatePinnedContract` so v8's `key.create` pins
   `LeastScopeKeyCreateParamsSchema` (the existing `revision is LeastScopeRegistryRevision
   or SupportedRegistryRevision && action == KeyCreate` special-case must include v8) and
   `service.reauthorize` pins its schema + grant/false metadata.
5. `docs/contracts/nyxid-assistant-conformance/v1/registry-v8.json` fixture =
   the exact manifest NyxID will publish (v7 fixture + §1.2 descriptor at index 1,
   revision bumped).
6. Registry test updates (`NyxIdAssistantActionRegistryTests.cs`,
   `NyxIdActionPostconditionPortTests.cs` already covers the verify path).

Sequence:
1. Aevatar merges + deploys the v8-accepting consumer; verify its startup against the
   **live v7** manifest (must stay fully functional — v8 acceptance is additive).
2. NyxID merges + deploys B1 (manifest bump) — B2/B3/F1/F2 can merge earlier; only the
   revision bump is order-sensitive. (If B1 is split so the descriptor+bump is its own
   final PR, everything else is unblocked immediately.)
3. Verify an Aevatar process restarted after the NyxID deploy loads v8 and advertises
   `service.reauthorize` as executable; run one end-to-end reauthorize through chat.

---

## 6. Test commands

```bash
# Backend (repo root). Backend tests need a MongoDB replica set +
# NYXID_TEST_DATABASE_URL (thousands of tests failing within seconds = connection
# failure, not code regressions — fix the test DB first).
source "$HOME/.cargo/env" 2>/dev/null
cargo test assistant_actions          # B1 focused
cargo test unified_key_service        # B2 focused
cargo test                            # full suite
cargo fmt --check

# CLI
cargo test -p nyxid-cli

# Frontend (CI parity: build, not just typecheck)
npm --prefix frontend run test
npm --prefix frontend run lint
npm --prefix frontend run build
```

Do not blanket-rebuild the CLI wizard bundle: the freshness CI check is source-only over
`index.manifest`; rebuild (`npm --prefix frontend run build:wizard` + commit
`cli/src/wizard/`) **only if** frontend deps/lockfile changed, and if the branch is
rebased after a rebuild, rebuild again.

---

## 7. Open questions for Calvin / eanz17

- **Q1 (blocking B1's final revision string):** confirm the next revision is
  `nyxid-assistant-actions.v8` and that Aevatar (eanz17) will author the consumer
  (§5 items 1–6). No v8 exists in Aevatar today; NyxID must not invent the pin
  unilaterally since Aevatar's map is the acceptance gate.
- **Q2 (auth posture of the evidence read) — RESOLVED, no work needed.** Verified on
  `origin/main`: `/keys` routes are nested inside `api_v1_human_only` (`routes.rs`
  lines 1507–1569). `reject_delegated_tokens` permits delegated GETs via
  `delegated_request_allowed` (`mw/auth.rs:466-479`: GET + `account:read` scope +
  path not in `delegated_read_denied_path` + not a WS upgrade), and `keys` is **not** in
  the deny list — confirmed by the test constant `DELEGATED_ALLOWED_MANAGEMENT_PATHS`
  (`mw/auth.rs:1371`, includes `"/api/v1/keys"` at line 1374) and test
  `delegated_account_read_allows_expected_management_families` (line 1469). So a
  session JWT or a delegated `account:read` token reads the evidence fine. Relay tokens
  (`reject_relay_tokens`, unconditional 403) and agent API keys / SA tokens are blocked
  — same posture as the already-working `GET /api-keys/{id}` reads for
  `key.create`/`key.rotate`, so Aevatar's human-session credential is proven compatible
  in production.
- **Q3 (provider scope-echo caveat):** `write_oauth_tokens_to_key` preserves old
  `token_scopes` when the provider token response omits `scope`, and `build_key_view`
  splits on whitespace. Two per-provider risks: a provider that never echoes scopes
  leaves `granted_scopes` stale (superset check may pass/fail incorrectly), and a
  provider that returns comma-separated scopes (GitHub-style `repo,read:user`) would
  yield a single comma-joined entry that can never match Aevatar's per-scope Ordinal
  contains. `TODO — not investigated`: what NyxID's callback actually stores per catalog
  provider (check `handle_oauth_callback` normalization). Decide whether to normalize
  commas→spaces at the callback write for affected providers as part of B2.
- **Q4 (product):** for `scope_removal === "unsupported"` catalogs (GitHub), granted
  scopes are locked in the picker; requested-scope pre-selection is purely additive
  there — confirm that is acceptable UX (it matches the existing manage-scopes flow).

---

## 8. Execution notes for the implementing agent

- Verify every "current state" quote in this plan against `origin/main` before editing —
  worktrees here are known to go stale.
- Branch: work on `feat/2026-08-17_wave1-service-reauthorize`; PR targets `main`;
  conventional commits; push with the `ctkm-aelf` gh account.
- Never place key material, tokens, or scope-bearing secrets in chat transcripts, audit
  events, or test fixtures (§1.4 tripwire list is a useful blocklist to test against).
- Rollup/branch CI caveat: `ci.yml` gates `main`/`dev` only — a green-looking PR into
  any other branch has NOT run the suite; run §6 locally.
