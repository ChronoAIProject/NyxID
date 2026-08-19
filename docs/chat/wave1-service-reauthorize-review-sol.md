# Adversarial review: Wave 1 `service.reauthorize`

> **Evidence document, not an action list.** The live action list is
> [`wave1-service-reauthorize-actions.md`](./wave1-service-reauthorize-actions.md).
> This is the implementer's pre-build review of the plan; its findings were
> dispositioned during implementation. Note it was written against the
> **pre-rebase** branch point, which makes its §3 freshness finding accurate for
> that branch but not for what shipped — see action list A4.

**Reviewed:** 2026-08-17

**NyxID plan source:** `51cab391` (`docs/chat/wave1-service-reauthorize-plan.md`)

**NyxID source checked:** `origin/main` at `ed372d8c`, plus the required implementation
branch starting point at `51cab391`

**Aevatar source checked:** `origin/feature/integrate` at `05db6b4b0` in
`~/Desktop/aelf-frontend-work/aevatar` (all citations below are from `git show`, not the
stale checkout)

## Overall verdict

**BUILD-WITH-FIXES.** There is **1 BLOCKER**, **4 MAJOR** findings, and **2 MINOR**
findings. The blocker is a merge/deploy gate, not a local implementation blocker: build
against the working v8 assumption, but the NyxID PR must remain draft and
**DO-NOT-MERGE** until Aevatar accepts and deploys that exact revision.

The plan's central endpoint and Aevatar-contract corrections are independently verified.
Its frontend completion path and provider-scope normalization are not sufficient as
written, and the required implementation branch is missing a backend prerequisite that
the plan assumed was already present.

## BLOCKER

### 1. Aevatar has no v8 consumer; publishing v8 first disables every NyxID assistant action in newly started processes

**Evidence.** Aevatar defines accepted revisions only through v7 at
`agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistry.cs:30-36`. Its pinned map
contains `service.reauthorize` only for v5
(`NyxIdAssistantActionRegistry.cs:207-232`), while its executable maps expose only
`service.connect` in v4/v5, add `key.create` in v6, and add `key.rotate` in v7
(`NyxIdAssistantActionRegistry.cs:234-248`). `IsActionExecutable` does not even map the
`ServiceReauthorize` enum to a wire action (`NyxIdAssistantActionRegistry.cs:295-316`).
Registry loading rejects any revision absent from both maps
(`NyxIdAssistantActionRegistry.cs:319-334`). The conformance tree contains
`registry-v4.json` through `registry-v7.json` and no v8 fixture; a branch-wide grep found
no v8 revision constant.

The failure mode is exactly the plan's corrected wording. Startup fetches and loads once;
any exception logs that assistant actions are disabled and initializes an empty disabled
registry (`agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistryStartup.cs:119-140`).
Chat startup is not aborted, but `TryGetDefinition` can no longer return any registry
action (`NyxIdAssistantActionRegistry.cs:421-443`). Thus chat survives while all NyxID
action cards are unavailable on Aevatar processes started after an early NyxID deploy.

**Concrete fix.** Implement NyxID against the explicitly provisional
`nyxid-assistant-actions.v8`, but keep the PR draft and prominently marked
**DO-NOT-MERGE**. Aevatar must first add v8 to both maps, make all four actions executable,
pin the exact schemas/metadata, add `registry-v8.json`, merge, deploy, and prove a restart
still loads the live v7 manifest. Only then may NyxID merge/deploy the v8 manifest.

## MAJOR

### 2. The implementation branch does not contain the `connection_status` baseline assumed by B2

**Evidence.** The plan was reviewed against `origin/main` at `ed372d8c`, where
`KeyResponse.connection_status` exists at `backend/src/handlers/keys.rs:437`, maps from
`KeyView` at `backend/src/handlers/keys.rs:2311`, and is derived by
`oauth_connection_status` at `backend/src/services/unified_key_service.rs:3803-3821`.
Those changes came from `206720f5` (`feat(keys): expose OAuth connection health`). The
required reset, however, leaves this branch at `51cab391`, whose source does not contain
`KeyResponse.connection_status`, `KeyView.connection_status`, or
`oauth_connection_status`. Therefore B2 cannot be implemented here merely by removing
three serde attributes and adjusting an existing helper.

The other five facts do exist: `id`, `status`, and `is_active` are stable response fields;
`granted_scopes` and `last_authorized_at` are on `KeyResponse` and mapped from `KeyView`;
the six credential statuses are documented on `UserApiKey.status` at
`backend/src/models/user_api_key.rs:68-80`. The plan is correct about `origin/main`, but
not about the actual build starting point mandated for this task.

**Concrete fix.** After this review commit, port only `206720f5` before applying B2. Do
not merge all of `origin/main`; it contains unrelated churn. Preserve the later branch's
current response fields and tests while resolving the two-file port.

### 3. F2 can report `completed` before any reauthorization occurs

**Evidence.** The baseline-aware dialog hook is real:
`useKeyAuthorizationStatus` accepts `previousAuthorizationAt` and requires
`last_authorized_at` to change before an active row is terminal
(`frontend/src/hooks/use-keys.ts:55-107`). But the background card hook cited by F2 is
not baseline-aware. `useKeyAuthorizationWatch` accepts only `enabled` and `deadlineAt`,
stops as soon as `status` is `active`, and returns that raw status
(`frontend/src/hooks/use-keys.ts:133-203`). `ActionCard` stores only `keyId` and
`startedAt`, invokes that hook without an authorization baseline, and reports completed
on the first active read (`frontend/src/components/assistant/blocks/action-card.tsx:203-206`,
`:222-251`). That behavior is correct for `service.connect`, whose new placeholder begins
as `pending_auth`, but it is wrong for reauthorization, whose existing key is already
`active`.

This is a real false-positive path: click Re-authorize, close or even fail to complete the
provider flow, and the first cached/network read of the existing active key can settle the
action. Aevatar's postcondition will later reject the stale `last_authorized_at`, but the
NyxID browser has still emitted a false completed journey and forced an avoidable
postcondition failure.

**Concrete fix.** Carry `previousAuthorizationAt` in the card's pending-authorization
state and extend `useKeyAuthorizationWatch` with an optional baseline. An active row is
terminal only when the baseline is absent (the existing connect behavior) or when
`last_authorized_at` is non-null and differs from the baseline. Add regression tests in
the hook/action-card suites proving an already-active unchanged row does not complete and
an advanced timestamp does.

### 4. Whitespace-only scope parsing makes GitHub-style token responses structurally unverifiable

**Evidence.** All catalog providers share one callback path; there is no per-provider
scope normalizer. The callback reads `token_payload["scope"]` as a raw string
(`backend/src/services/user_token_service.rs:1762-1789`) and passes it unchanged to both
chat and ordinary multi-connection writers (`user_token_service.rs:1809-1839`). The
writer stores that raw string verbatim when present
(`backend/src/services/user_api_key_service.rs:444-489`). The legacy path likewise stores
the raw string, preserving the previous value only when a provider omits `scope`
(`user_token_service.rs:1883-1927`). Device-code storage follows the same rule
(`user_token_service.rs:1435-1460`).

By contrast, `build_key_view` uses only `split_whitespace`
(`backend/src/services/unified_key_service.rs:3773-3786` on `origin/main`). GitHub is a
seeded OAuth provider (`backend/src/services/provider_service.rs:655-688`) and GitHub-style
comma-separated echoes such as `repo,read:user` therefore become one array item. Aevatar
requires Ordinal per-scope containment and rejects duplicate arrays, so the requested
`repo` and `read:user` can never match that single item. NyxID already treats commas and
whitespace as separators for user-supplied OAuth scopes
(`backend/src/services/user_token_service.rs:90-142`), so the read behavior is internally
inconsistent too.

Deduplication itself is safe if it is case-sensitive and first-occurrence preserving.
OAuth scopes are compared as an unordered set by Aevatar
(`agents/Aevatar.GAgents.NyxidChat/NyxIdActionPostconditionPort.cs:291-303`, `:624-637`),
and no inspected NyxID consumer assigns semantic meaning to duplicate occurrences.
Preserving first occurrence keeps display and request order stable.

**Concrete fix.** At the `KeyView` read boundary, split on comma **or** Unicode
whitespace, trim empty items, and deduplicate with Ordinal/case-sensitive first-occurrence
order. Add whitespace, duplicate, and GitHub comma-style tests. Keep the stored provider
echo unchanged to avoid silently rewriting persistence used by other paths. Document the
remaining fail-closed behavior: when a provider omits `scope`, NyxID preserves the last
known set; Aevatar still requires both a fresh authorization timestamp and the requested
scope superset.

### 5. The proposed expiry-less derivation incorrectly labels `pending_auth` as an active OAuth connection

**Evidence.** `UserApiKey.status` includes `pending_auth`
(`backend/src/models/user_api_key.rs:68-80`). The helper first maps only terminal states
to `expired`, then derives from `expires_at`
(`backend/src/services/unified_key_service.rs:3803-3821` on `origin/main`). B2 says every
non-terminal expiry-less OAuth row should become `active`; that includes a placeholder
with no completed callback and normally no access token. `connection_status` is a global
`KeyResponse` field used by list/create/update/detail responses, not an assistant-only
projection (`backend/src/handlers/keys.rs:414-541`, `:2280-2360`). Aevatar would still
reject the row because credential `status != active`, but other callers would receive a
misleading health signal.

**Concrete fix.** Return expiry-less `active` only when the credential status is exactly
`active` and OAuth token material exists. Keep terminal states mapped to `expired`, and
return `None`/JSON `null` for `pending_auth`, non-OAuth rows, and inconsistent active rows
without token material. Cover each branch in unit tests. This deliberately narrows the
plan's proposed non-terminal rule to avoid changing non-assistant behavior incorrectly.

## MINOR

### 6. The plan overstates the secret-scan audit by checking names but not user-controlled values

**Evidence.** The Aevatar parser recursively visits every object and array and rejects
both forbidden normalized field names and secret-shaped strings
(`src/Aevatar.AI.ToolProviders.NyxId/NyxIdApiAccessContracts.cs:262-296`, `:564-591`).
`KeyResponse` contains user-controlled display strings, URLs, custom User-Agent, default
headers, and WebSocket injection configuration, not just the six evidence fields
(`backend/src/handlers/keys.rs:414-531`). Sensitive default-header values are redacted at
the response boundary (`backend/src/models/default_request_header.rs:333-364`), but a
non-sensitive value or label that happens to match `Bearer ...` or the NyxID-key regex
still causes a malformed evidence read.

The three serializer changes themselves are safe: they add only `null`, scope arrays,
and RFC3339 timestamps under non-forbidden names. This is therefore not a blocker to the
current build, but the plan's statement that field names are clear is not a complete
tripwire analysis.

**Concrete fix.** Add the requested `KeyResponse` serialization test and include an
Aevatar-compatible recursive tripwire assertion over the representative evidence
response. Do not add or rename unrelated response fields in this PR. Track a future
minimal evidence projection or a less false-positive-prone Aevatar scan if arbitrary
user-controlled display/configuration values become an operational problem.

### 7. The response-shape compatibility conclusion is safe, but the plan cites frontend helpers that do not exist on this branch

**Evidence.** No `assertSecretFreeReadBack`, `verifyAllowedServices`, or strict assistant
key read-back schema exists in this checkout. `useKey` and `useKeys` use TypeScript
generic response typing rather than runtime Zod parsing
(`frontend/src/hooks/use-keys.ts:17-34`). The only key-domain Zod response schema is for
`/user-services`, explicitly permits unknown fields, and does not declare these evidence
fields (`frontend/src/schemas/keys.ts:27-49`). `KeyInfo.granted_scopes` and
`last_authorized_at` already accept absent or null values
(`frontend/src/types/keys.ts:55-105`); `connection_status` is ignored. CLI `/keys`
consumers use `serde_json::Value`, for example
`cli/src/commands/node.rs:692-708` and `cli/src/commands/service.rs:911-920`. Whole-tree
searches found no mobile or SDK parser for these fields.

Aevatar's `/keys` list parser reads selected properties and ignores extras
(`src/Aevatar.AI.ToolProviders.NyxId/NyxIdApiAccessContracts.cs:397-428`). Therefore
always serializing the three fields does not break the audited frontend, CLI, mobile,
SDK, or Aevatar list consumer. Rust/serde `Option<T>` also accepts explicit null for any
future typed CLI consumer.

**Concrete fix.** Do not invent schema changes or nonexistent helper calls. Keep the
three fields null-compatible in `KeyInfo`, add the narrow runtime secret-free assertion
inside the new reauthorize read path if one is needed, and rely on frontend build/tests
plus CLI tests as the compatibility gate.

## Re-derived contract facts

### Evidence endpoint: confirmed

`NyxIdApiClient.GetServiceAsync` calls
`GET /api/v1/keys/{Uri.EscapeDataString(id)}`
(`src/Aevatar.AI.ToolProviders.NyxId/NyxIdApiClient.cs:266-275`). The
`/api/v1/user-services` method is a separate list method at
`NyxIdApiClient.cs:1022-1033`. The reauthorize evidence port calls the former with the
exact requested user-service id
(`agents/Aevatar.GAgents.NyxidChat/NyxIdActionPostconditionPort.cs:277-289`). Section
0.1 is correct.

### Evidence parser: confirmed

`ParseUserServiceAuthorizationDocument` runs the recursive secret scan, then requires
`id`, `is_active`, `status`, and the **presence** of `connection_status`,
`granted_scopes`, and `last_authorized_at`; only `api_key_id` is optional
(`src/Aevatar.AI.ToolProviders.NyxId/NyxIdApiAccessContracts.cs:431-442`). Credential
status accepts exactly `active`, `expired`, `revoked`, `failed`, `refresh_failed`, and
`pending_auth` (`NyxIdApiAccessContracts.cs:593-602`). Connection status accepts only
`active`, `expired`, or explicit null as `Unspecified`
(`NyxIdApiAccessContracts.cs:604-617`). Scope arrays reject non-normalized values and
Ordinal duplicates (`NyxIdApiAccessContracts.cs:517-542`); timestamps must match the
RFC3339 regex and parse successfully (`NyxIdApiAccessContracts.cs:286-288`, `:545-557`,
`:843-855`).

Malformed JSON or any contract exception becomes a failed provider read
(`NyxIdApiAccessContracts.cs:325-350`), which the postcondition maps to provider-read
unavailable/not-found rather than an evidence mismatch
(`NyxIdActionPostconditionPort.cs:283-313`, `:596-611`). A syntactically valid document
that parses but fails identity, activation, status, scope-superset, or freshness checks is
merely unverified. Section 1.4 is correct.

### Exact params-schema pin: confirmed

`ValidatePinnedContract` parses the pinned and published schemas into `JsonNode` values
and calls `JsonNode.DeepEquals`, then checks exact risk and remember eligibility
(`agents/Aevatar.GAgents.NyxidChat/NyxIdAssistantActionRegistry.cs:879-903`). NyxID must
publish the schema exactly as written, with no `minItems`, `uniqueItems`, length bounds,
or other refinements. Section 0.6 is correct.

### Facade/B3: confirmed no code change

The backend report does not carry an action id. `RawActionReport` contains request id,
origin turn, disposition, and optional resource only
(`backend/src/services/assistant_service.rs:550-558`). `parse_action_resource` already
accepts the exact `userService.userServiceId` shape with identity validation
(`assistant_service.rs:748-798`), and the only completed-report rule is that some resource
must be present (`assistant_service.rs:885-912`). The existing test round-trips all six
safe resource variants, including `userService`
(`assistant_service.rs:2124-2156`). B3 needs no implementation change.

## Explicit TODO resolutions

### Frontend strict/null schemas

Resolved: no frontend Zod schema strictly parses `GET /keys/{id}` or `/keys` into a
closed object. The key fields are TypeScript-only and already null-compatible; no schema
change is required. The assistant action schemas are strict request/report schemas, not
key-response parsers.

### Unknown/free-form OAuth scopes

Resolved: `UpstreamScopePicker` explicitly supports free-form scopes. It unions catalog,
defaults, locked scopes, and unknown selected values into pills
(`frontend/src/components/shared/upstream-scope-picker.tsx:80-128`), and `addCustom`
parses/deduplicates unknown values (`upstream-scope-picker.tsx:179-202`). A shared
platform OAuth allowlist can intentionally reject an unknown scope
(`upstream-scope-picker.tsx:147-152`, `:181-194`). The reauthorize journey must preserve
unknown requested scopes and surface a typed block when the selected managed OAuth path
forbids one; it must never silently drop them.

### Q3: what callbacks store in `token_scopes`

Resolved from source: there is no catalog-provider-specific normalization. OAuth and
device-code callbacks store the provider's raw string-valued `scope` exactly as returned;
when `scope` is absent, multi-connection and legacy flows preserve the previous stored
value. Authorization initiation normalizes comma/whitespace user input and sends a
space-joined request, but the callback does not normalize the provider echo. Consequently
the response read boundary must handle both comma- and whitespace-delimited echoes, and
provider omission remains a last-known-scope behavior guarded by Aevatar's fresh timestamp
and scope-superset verification.
