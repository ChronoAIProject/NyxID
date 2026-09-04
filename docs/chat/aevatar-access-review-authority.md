# Aevatar access-review authority decision

**Status.** Use consent-derived delegated restrictions as the AC-3 authority
model, with the live-grant-backed REST read, the exact platform-callback
exemption, and the validated Aevatar client link. Service access review remains
unreachable today because the current bridge is both route-denied and
unrestricted. Keep `forward_access_token=true` and unrestricted forwarding in
production until AC-3 lands; AC-3 proceeds under its own security review gate.

This record is the AC-2 probe and authority decision. It changes no product
behavior. NyxID evidence was collected from local SHA
`28f26d85b00ca00e23dfeb50c38251e184e95d06`. Aevatar source evidence is pinned
to `e5bba2e9719ad5132004b882744caa3875db1123`. Runtime Aevatar rows are marked
source-proven because this environment has no .NET runtime and Aevatar accepts
only production-JWKS tokens.

## Receipt contract

Every fixed probe row is reconstructed as this closed record:

```text
ProbeReceiptRow
  row_id
  surface
  deployment_version
  request_shape
  http_status
  upstream_code
  claim_summary
    delegated
    actor_present
    client_id_present
    allow_all_services
    allowed_service_ids_count
    resource_count
    scope_names
    expiry_window_seconds
  verdict
  observation_kind
```

The probe never emits subjects, actor values, client values, service IDs,
resource URIs, `jti` values, tokens, cookies, authorization values, refresh
tokens, API keys, or response prose. The source rows deliberately use `null`
where pinned source does not establish an HTTP observation.

## Seven-row receipt

| Row | Surface and version | HTTP | Safe code | Claim summary | Verdict |
| --- | --- | ---: | --- | --- | --- |
| Identity only | Pinned Aevatar source `e5bba2e` | n/a | n/a | non-delegated; no actor, client, service, or resource authority; 60-second lifetime | Execution capability is still required; source-proven, not runtime-observed |
| Identity plus capability Bearer | Pinned Aevatar source `e5bba2e` | n/a | n/a | delegated; actor present; `allow_all_services=false`; 1 allowed service; 0 resources; `proxy`; 300 seconds | Authorization Bearer is selected as `ProxyDelegation`; source-proven, not runtime-observed |
| Identity plus delegation header | Pinned Aevatar source `e5bba2e` | n/a | n/a | delegated; actor present; `allow_all_services=true`; 0 allowed services and 0 resources; `proxy:*`; 300 seconds | Delegation header supplies `ProxyDelegation` when Bearer is absent; source-proven, not runtime-observed |
| Replayed identity `jti` | Pinned Aevatar source `e5bba2e` | 401 | `identity_assertion_replayed` | non-delegated identity assertion; 60 seconds | Replay rejected; source-proven, not runtime-observed |
| Bridge Bearer reads REST MCP config | Local NyxID `28f26d85` | 403 | `1002` | delegated; actor present; `allow_all_services=true`; 0 allowed services and 0 resources; `proxy:*`; 300 seconds | Route denied before catalog filtering; runtime-observed |
| Restricted Bearer uses LLM gateway | Local NyxID `28f26d85` | 200 | none | delegated; actor present; `allow_all_services=false`; 1 allowed service; 0 resources; `proxy`; 300 seconds | Gateway control reaches its upstream; runtime-observed |
| Restricted Bearer uses platform proxy slug | Local NyxID `28f26d85` | 403 | `9000` | same restricted summary | Legacy platform-row scope gate denies Aevatar's default callback; runtime-observed |

The machine receipt is `/tmp/ac2-evidence/seven-row-receipt.json`. Local setup
statuses are in `/tmp/ac2-evidence/local-runtime-setup.json`. The three local
rows were rerun through a real `cargo run` NyxID backend at the recorded SHA.
The only stub was a loopback OpenAI-compatible downstream behind NyxID. It
returned one fixed, successful chat-completion response so the gateway row
could prove that NyxID forwarded the request; it did not replace NyxID or
Aevatar. The stub log contains four identical POST shapes from the repaired
receipt run, the raw-response capture, the error-code-parser rerun, and the
final rerun after correcting the local identity-token fixture to a 60-second
lifetime. The final machine receipt comes from the fourth run; the raw gateway
response comes from the second. The gateway row is a control, not Aevatar's
default route. Raw response logs are under `/tmp/ac2-evidence/runtime/`.

## What the transport authenticates

The identity assertion and execution capability have separate jobs. Aevatar
selects `X-NyxID-Identity-Token` as the authentication scheme whenever it is
present and treats its signed subject as the tenant identity, while leaving the
Authorization header available to workflow execution
(`NyxIdIdentityAssertionAuthentication.cs:31-46,68-98`). Its validator checks
signature, issuer, audience, lifetime, `sub`, `jti`, and `iat`, then atomically
consumes replay state (`NyxIdIdentityAssertionValidator.cs:120-183`).

Identity alone is not an execution capability. Streaming extracts a NyxID
credential and returns 401 when none is available
(`NyxIdChatEndpoints.Streaming.cs:91-101`). Credential extraction gives a
Bearer header precedence, classifies a delegated Bearer as `ProxyDelegation`,
and falls back to `X-NyxID-Delegation-Token` only when Bearer is absent
(`NyxIdChatEndpoints.cs:317-379`). The pinned header-matrix test independently
fixes identity-only as credential-less and both delegation forms as proxy
delegation (`NyxIdChatBrowserCredentialTests.cs:31-33` and
`ChatEndpointsInternalTests.cs:1859-1899`).

NyxID's session bridge currently supplies both sides. Session authentication
constructs `AuthUser` with `allow_all_services=true`, an empty service list, and
no resource list (`backend/src/mw/auth.rs:920-941`).
`TokenRestrictionClaims::from_auth_user` copies those values
(`backend/src/crypto/jwt.rs:341-353`), and
`build_forward_authorization` signs that projection into the five-minute
delegated Bearer (`backend/src/handlers/assistant.rs:722-741`). Standard
`X-NyxID-Delegation-Token` injection uses the same projection
(`backend/src/handlers/proxy.rs:2493-2507`).

## Why access review is unreachable

`USER_SERVICE_ACCESS_REQUIRED` requires a successful current-bearer catalog
read followed by zero exact matches. Aevatar calls `GET /api/v1/mcp/config`
with the execution Bearer (`NyxIdRequireServiceTool.cs:311-344` and
`NyxIdApiClient.cs:1339-1340`). Access denial becomes source unavailable, which
produces `SourceStale` or `INVENTORY_INVALID`, not service access denied
(`NyxIdRequireServiceTool.cs:217-280`). The access-review postcondition follows
the same path through `NyxIdMcpOperationCatalogReader.ReadAsync`
(`NyxIdActionEvidenceReadPort.cs:75-145` and
`NyxIdMcpOperationCatalogReader.cs:48-95`).

Two independent NyxID authorities prevent the required zero-match result:

1. **The read cannot run.** `delegated_read_denied_path` rejects the entire
   `mcp` class (`backend/src/mw/auth.rs:394-420`).
   `delegated_request_allowed` requires both exact `account:read` and a path
   outside that class (`backend/src/mw/auth.rs:468-482`), and
   `reject_delegated_tokens` enforces it (`backend/src/mw/auth.rs:1159-1181`).
   `catalog_read_scope_alone_grants_no_proxy_or_management_route` pins
   `/api/v1/mcp/config` as denied (`backend/src/mw/auth.rs:1508-1518`). The
   local bridge receipt confirms HTTP 403.
2. **The current bridge cannot omit one connected service.** If the route were
   reachable, `get_mcp_config` would select `ServiceScope::Unrestricted` for
   this token (`backend/src/handlers/mcp.rs:83-111`). The service filter admits
   every row in that mode (`backend/src/services/mcp_service.rs:676-709`). The
   bridge receipt records `allow_all_services=true`, and the consent-revocation
   probe records the same unrestricted summary before and after revocation.

The JSON-RPC `/mcp` transport is not a substitute for Aevatar's REST call. Its
catalog authority is online and revocable: a signed `mcp:catalog:read` token
must match an unexpired, unrevoked `CatalogDelegationGrant`, active clients,
current consent, service ownership, and canonical resources
(`backend/src/services/catalog_delegation_service.rs:38-42,81-172`). Aevatar
does not call this transport for either access check. The current bridge Bearer
can already read the same operation catalog through `POST /mcp` `tools/list`
(`backend/src/handlers/mcp_transport.rs:455-470,709-739,1248-1266`). The REST
allowance therefore exposes no new catalog data class to that bearer; its new
security surface is the live-grant check on the additional route.

Restricted catalog projection has a deliberate limitation. `ServiceScope::Allowed`
keeps user-service rows only and drops every platform row
(`backend/src/services/mcp_service.rs:676-709`); JSON-RPC applies the same rule
(`backend/src/handlers/mcp_transport.rs:709-739`). Consent contains user-service
IDs, so it cannot restore a platform row to a restricted catalog.

## Authority candidates

| Candidate | Threat model | Revocation and retry | Deployment | Rollback |
| --- | --- | --- | --- | --- |
| Consent-derived delegated restrictions | Least authority can exclude unapproved user services. It is safe only with a live grant bound to `jti`, subject, actor client, receiving client, service IDs, resources, and expiry. A bare scope claim is insufficient. | Consent revocation must invalidate the online grant and the next minted capability. Confirmation retry must atomically merge the same service without replacing existing scopes or IDs. | Requires the live-grant-backed REST read, an exact platform-callback exemption, and the validated Aevatar client link. The gateway control survives restriction, but Aevatar's default slug callback does not without the exemption. | Disable the access-review action and effect first. Revoke grants, restore unrestricted bridge minting, verify both auth inputs and default chat, then remove the slug and REST exceptions before clearing the client link. |
| Complete OAuth browser round trip with NyxID return channel | OAuth authorization code, PKCE, `state`, redirect ownership, tab and session binding, and return-channel replay become part of chat's security boundary. It duplicates an identity and capability transport that already exists. | Standard OAuth revocation is clear, but assistant retry must resume across redirects and distinguish user cancellation, callback replay, stale conversations, and completed-but-unobserved consent. | Requires new browser navigation, a callback and return protocol, Aevatar continuation state, and two-system rollout. No such return channel exists in the pinned contract. | Disable the new browser entry and return to the bridge. In-flight OAuth state must expire without resuming a chat action. |
| Preserve unrestricted forwarding with an upstream reachability explanation | Aevatar retains broad service authority for the session, so it cannot represent per-service approval. The UI must not claim that it can. Existing route denials still protect management surfaces. | Five-minute capabilities expire normally. Consent and service review cannot revoke a subset because no subset is minted. Ordinary chat retry behavior is unchanged. | No product change. Keep the live row's `forward_access_token=true` as the deployed posture until AC-3 is proven. | This is the rollback posture. A restricted rollout must disable its access-review action before restoring this bridge, then verify identity plus capability delivery. |

### Cost of the consent-derived candidate

1. **Reachable, live-grant-backed REST catalog read.** Add an exact GET-only
   exception for `/api/v1/mcp/config` in
   `backend/src/mw/auth.rs:delegated_request_allowed` without opening the rest
   of the `mcp` class. The verified request must also pass
   `catalog_delegation_service::validate_live_grant`; accepting
   `mcp:catalog:read` from the middleware's unverified payload peek is not
   sufficient. `handlers/mcp.rs:get_mcp_config` must receive the verified grant
   identity before it calls `mcp_service::load_operation_catalog`.
   `catalog_read_scope_alone_grants_no_proxy_or_management_route` must change
   from unconditional denial to two cases: no live grant remains denied, while
   the exact route plus a matching live grant reaches the handler. Every other
   proxy and management route remains denied. The handler itself calls
   `ensure_rest_proxy_access` (`backend/src/handlers/mcp.rs:88-92`), so the
   assistant capability's exact scope string must be
   `proxy:* mcp:catalog:read`. Do not add `mcp:catalog:read` to
   `SERVICE_DELEGATION_SCOPES`: that list feeds ordinary
   `inject_delegation_token` minting, which has no matching online-grant
   persistence (`backend/src/mw/auth.rs:271-279` and
   `backend/src/handlers/proxy.rs:2493-2507`).
2. **Restricted platform callback.** Aevatar's default chat path is
   `/api/v1/proxy/s/chrono-llm-public`. `LlmDefaults.NyxIdRoute` supplies the
   slug when `Aevatar:NyxId:DefaultRoute` is unset, as it is in the pinned
   mainnet settings; `NyxIdLLMProvider` then normalizes it to the proxy path
   (`LlmDefaults.cs:21`, `ServiceCollectionExtensions.cs:1167-1170`,
   `NyxIdLLMProvider.cs:365-379`, and `NyxIdChatEndpoints.cs:272`). NyxID's own
   bridge documentation names the same route
   (`backend/src/handlers/assistant.rs:691-699,811-820`). The gateway control
   reaches the stub, but it is not the default callback.

   The default path currently fails because `execute_proxy_inner` returns
   `ApiKeyScopeForbidden` for every legacy `DownstreamService` when
   `allow_all_services=false` (`backend/src/handlers/proxy.rs:2011-2033`). Add a
   narrow exemption for the exact active `chrono-llm-public` platform row. It
   must require verified delegated assistant authority, actor equal to the
   Aevatar row's linked client, receiving client equal to that same link, and a
   valid live catalog grant. Do not bypass the gate for any other platform row
   or delegated caller. The server-chosen assistant ingress is different: it
   uses `execute_admin_proxy`, which already skips this caller-addressed gate
   (`backend/src/handlers/proxy.rs:1439-1481`).
3. **Durable Aevatar client link.** Add an admin-managed
   `delegated_authority_client_id` field to the `aevatar`
   `DownstreamService`. On create or change, resolve an active OAuth client and
   call `catalog_delegation_service::ensure_client_can_delegate_catalog`; emit a
   metadata-only audit event for the link change. Resolve and validate the row
   and client again for every mint. Do not reuse `oauth_client_id`. That field
   means the OIDC client used when `auth_method="oidc"`
   (`backend/src/models/downstream_service.rs:220-222`). A row field is
   preferable to environment configuration because the row already owns the
   integration's base URL, identity audience, token forwarding, and delegation
   policy. It can be inspected, validated, audited, and rolled back with the
   integration. A process setting would permit row/config drift and split one
   integration's policy across deployment state. NyxID already prevents active
   row ambiguity with a partial unique index on `slug` for `is_active=true`
   (`backend/src/db.rs:335-352`). The pinned upstream client is
   `a6ff2946-f02f-4c35-8203-1ec46132b660`
   (`src/Aevatar.Mainnet.Host.Api/appsettings.json:41-44`), but the current
   NyxID row has no field that resolves that identity.

   The linked client's `delegation_scopes` must contain both `proxy:*` and
   `mcp:catalog:read`. `validate_live_grant` checks every minted scope against
   both the actor and receiving client, even when the link supplies both roles
   (`backend/src/services/catalog_delegation_service.rs:120-132` and
   `backend/src/services/token_exchange_service.rs:487-499`).
   `OAUTH_CLIENT_DELEGATION_SCOPES` admits `proxy:*` and
   `mcp:catalog:read`, but not bare `proxy`. This allowed list is why the mint
   uses `proxy:* mcp:catalog:read`
   (`backend/src/services/oauth_client_service.rs:41-49`). Validate the complete
   scope at link time and again at mint time. A missing scope must fail closed.

## Decision predicate

| Condition | Current system | Candidate design | Evidence and AC-3 proof |
| --- | --- | --- | --- |
| Restricted chat bootstrap still works | **Fail.** The restricted gateway control reaches its upstream, but the default `/api/v1/proxy/s/chrono-llm-public` callback returns 403 at the legacy platform-row gate. | **Pass only with change 2.** The exact live-grant-bound `chrono-llm-public` exemption admits the default callback without admitting another platform row. | Seven-row restricted callback rows; pinned default-route selection in `LlmDefaults.cs:21`, `ServiceCollectionExtensions.cs:1167-1170`, `NyxIdLLMProvider.cs:365-379`, and `NyxIdChatEndpoints.cs:272` |
| Exact Aevatar OAuth client resolves without guessing | **Not linked.** Aevatar and the registered NyxID client agree on `a6ff2946-f02f-4c35-8203-1ec46132b660`, but `DownstreamService` cannot express that relationship today. | **Pass.** Add the admin-managed `delegated_authority_client_id` field, validate catalog delegation at link time, and re-resolve it before every mint. | Pinned `BackendConsole.OidcClientId`; production client observation; `DownstreamService` field and audit tests |
| Approving a service changes the next minted capability | **Not implemented.** Revoking consent changed the consent read but left the next current-bridge summary unrestricted. | **Pass.** Mint each assistant capability from current consent for the linked client. An atomic approval merge must therefore change the next minted service list, while revocation must remove that authority. | Negative-case consent receipt; AC-3 before, apply, retry, next-mint, and revoke tests |

The current system fails all three conditions: restriction breaks the default
callback, the bridge has no client link, and consent cannot change its minted
authority. Those results explain why access review is unreachable today. They
do not falsify the candidate, whose three enabling changes supply the missing
authorities.

The consent-derived candidate satisfies the predicate and is accepted for
AC-3. A complete OAuth round trip is rejected because it adds a browser return
protocol without improving the chosen authority model. Unrestricted forwarding
remains the deployed and rollback posture until AC-3 proves the replacement.

### Accepted operational design

AC-3 may implement consent-derived delegated restrictions under its own
security review. It must add the exact GET-only `/api/v1/mcp/config` allowance
backed by a verified live `CatalogDelegationGrant`, while every other route in
the `mcp` class remains denied. It must add the exact live-grant-bound
`chrono-llm-public` exemption required by Aevatar's default chat route, plus the
validated `delegated_authority_client_id` link, and mint each assistant
capability from the linked client's current consent with scope
`proxy:* mcp:catalog:read`. AC-3 must split the existing route-policy test into
no-grant denial and matching-live-grant success cases, and it remains subject to
an independent security review before implementation begins.

Before restricted minting is enabled, provision or verify both `proxy:*` and
`mcp:catalog:read` in the linked Aevatar OAuth client's `delegation_scopes`.
Verify the same complete scope for both actor and receiving client if AC-3 uses
separate clients. Deployment must stop if either client is missing either
scope.

Until AC-3 lands and proves that Aevatar receives both caller identity and the
restricted capability, keep `forward_access_token=true` and preserve the
current unrestricted forwarding posture. Access review remains unreachable in
that interim state for the independent route-denial and allow-all reasons above.

## AC-3 change map

This map defines the symbols for the authorized consent-derived implementation.
AC-3 remains subject to its own security review gate.

| Responsibility | Symbols |
| --- | --- |
| Atomic consent merge | Add `consent_service::merge_consent_services_atomic` in `backend/src/services/consent_service.rs`. Use one atomic aggregation-pipeline `find_one_and_update` upsert keyed by `(user_id, client_id)`. Set-union the requested service ID and required scopes while preserving all existing scopes, IDs, `allow_all_services`, grant identity, and unrelated fields. On a concurrent first-approval E11000, retry as a non-upserting merge against the winning row. Never call `grant_consent_with_services`. `grant_consent_internal` replaces the full row at lines 71-100, and the OAuth consent page in `backend/src/handlers/oauth.rs:807-836` still calls that replacement path; treat it as a known concurrent-writer hazard. The unique `(user_id, client_id)` index is at `backend/src/db.rs:911-920`. |
| HTTP effect | Add the confirmed `service.access_review` effect in `backend/src/handlers/assistant_action_effects_services.rs` and mount only its typed route in `backend/src/routes.rs`. Call `assistant_action_receipts::reserve_or_replay` before mutation. Resolve actor, client link, and user-owned service server-side; reject cross-user IDs before the merge. |
| Authority mint | Replace the session projection in `backend/src/handlers/assistant.rs:build_forward_authorization`. Re-resolve the active `aevatar` row and `delegated_authority_client_id`, read current consent, derive `CatalogAuthority`, and call `generate_delegated_access_token_for_client` with exact scope `proxy:* mcp:catalog:read` so `client_id` is present. Use the linked client as both actor and receiver unless AC-3 introduces and validates a separate actor client. Before minting, require every scope in that exact string to appear in both clients' `delegation_scopes`; fail closed on a missing scope. `OAUTH_CLIENT_DELEGATION_SCOPES` permits this pair and excludes bare `proxy` (`backend/src/services/oauth_client_service.rs:41-49`). Persist the matching online grant with `catalog_delegation_service::persist_grant` before forwarding. Do not derive service restrictions from session `AuthUser`. |
| REST catalog guard | Change `mw/auth.rs:delegated_request_allowed`, `reject_delegated_tokens`, and their route tests; add verified live-grant enforcement before `handlers/mcp.rs:get_mcp_config` publishes `mcp_service::ServiceScope`. |
| Platform callback | Narrow the legacy `DownstreamService` gate in `handlers/proxy.rs:execute_proxy_inner` for the exact active `chrono-llm-public` row. Require delegated assistant authority, actor and receiving client equal to the current Aevatar row link, and a valid live grant. Keep `handlers/llm_gateway.rs:gateway_request` as a control path; it is not Aevatar's default route. |
| Client link | Extend `models/downstream_service.rs`, the admin service request and response DTOs in `handlers/services.rs`, and admin validation. At link create or change, require an active OAuth client, call `catalog_delegation_service::ensure_client_can_delegate_catalog`, and validate complete delegation scope `proxy:* mcp:catalog:read`. Append a metadata-only audit event. Re-resolve the link and repeat both scope checks at mint time. No environment variable is added. |
| OAuth client prerequisite | Before deployment, provision or verify `proxy:*` and `mcp:catalog:read` in the linked Aevatar client's `delegation_scopes`. Use `token_exchange_service::validate_delegation_scope` for the complete string and keep `catalog_delegation_service::validate_live_grant` authoritative at request time. Apply the same check to both actor and receiver if AC-3 uses separate clients. `OAUTH_CLIENT_DELEGATION_SCOPES` in `services/oauth_client_service.rs` permits this pair and excludes bare `proxy`. Fail closed if either scope is missing. |
| Consent revocation | Extend `consent_service::revoke_consent` and `handlers/consent.rs::revoke_my_consent` to mark all outstanding catalog grants for the user and linked client revoked before consent deletion completes. Regranting consent within the old token's TTL must not reactivate that token. |
| Audit | Append metadata-only events `assistant_delegated_authority_client_link_changed`, `assistant_service_access_review_requested`, `assistant_service_access_grant_merged`, `assistant_service_access_grant_replayed`, `assistant_service_access_grant_denied`, and `assistant_delegated_authority_minted`. Include actor ID, owner ID, service ID, client ID, action request ID, outcome, and online-grant ID where applicable; never include a token, credential, cookie, or authorization value. Preserve the existing `oauth_consent_revoked` event and make grant invalidation observable. |

Required tests include database cases for the state before the merge, the first
apply, an idempotent retry, revocation, and a cross-user request. They also
include concurrent merges of different service IDs, live-grant expiry and
revocation, revoke then regrant within one token TTL with the old token still
denied, a missing/inactive/mismatched client link, exact-route denial without a
grant, and audit metadata redaction. Catalog tests must prove that a restricted
mint drops every platform row and that consent cannot restore one. Bootstrap
tests must cover both the gateway control and the default proxy-slug callback,
including denial for the wrong row, actor, receiver, or grant. Client tests must
remove `proxy:*` and `mcp:catalog:read` one at a time from both the actor and
receiving client, and prove that link validation and minting fail closed.

## Negative cases

| Case | Evidence | Result | Audit |
| --- | --- | --- | --- |
| Expired delegated capability | The local gateway received an expired delegated capability | 401; failed closed | The receipt records no audit event ID |
| Identity `jti` replay | Pinned validator consumes replay state only after signed-claim validation; its authentication test sends the same token twice (`NyxIdIdentityAssertionAuthenticationTests.cs:87-96`) | Second request is 401 with `identity_assertion_replayed`; source-proven because Aevatar cannot run locally | Replay guard is upstream; no local NyxID audit ID |
| Service removal | A user service deletion returned 200. The next request for the removed service returned 404. | Failed closed | The receipt records audit event `a62fe61d-cd77-461f-b942-7fbbf50e935f` without an event name |
| Consent revocation | The revoked user's consent read became 404. The other user's consent read stayed 200. The next bridge summary did not change. | Revocation is owner-scoped, but the consent-derived bridge candidate fails because bridge authority did not change. | The receipt records audit event `f37bd037-41a0-4359-851b-3e0d98232c69` without an event name |
| Cross-user resource substitution | The substitution case returned 404. The other user's consent read stayed 200. | Failed closed | The receipt records no audit event ID |

The machine matrix is `/tmp/ac2-evidence/negative-case-receipt.json`.

## Deployment and rollback

`forward_access_token` must remain `true` on the live `aevatar` row until a
replacement is proven to deliver both the caller identity assertion and an
execution capability to Aevatar. The identity assertion cannot replace the
capability: Aevatar's stream rejects identity-only requests.

For a future restricted rollout, use this rollback order:

1. Disable the access-review action and effect so no new confirmation can
   promise a per-service grant.
2. Stop issuing restricted assistant grants, mark their outstanding
   `CatalogDelegationGrant` rows revoked, and restore the current unrestricted
   session bridge.
3. Verify typed chat, identity assertion receipt, and capability receipt using
   Aevatar's default proxy-slug route; keep the gateway as a control.
4. Remove the proxy-slug exemption, then the REST MCP exception.
5. Remove or clear the authority-client link only after no minted grant depends
   on it; leave online grants revoked or let them expire.

This order restores execution before removing read paths and metadata. It does
not create an interval where a UI claims per-service authority while Aevatar is
running unrestricted.

## Production attempt

The permitted production read probe ran against
`https://nyx-api.chrono-ai.fun`. The supplied `~/.nyxid/access_token` expired
during the probe window. Authenticated reads returned 401; the public assistant
action manifest returned 200 with 57 items. No production chat POST or DELETE
was attempted. The redacted receipt is
`/tmp/ac2-evidence/production-read.json`. This is an explicit missing deployed
observation, not a runtime claim about Aevatar.

## Method decisions

- **Model the Domain.** The receipt is a discriminated, fixed seven-row record,
  not generic request and response logging.
- **Laziness Protocol.** The authority link lives on the existing Aevatar row;
  AC-3 adds no process configuration or second integration registry.
- **Boundary Discipline.** Tokens stay inside the probe process. The probe
  reconstructs output field by field and applies a final secret-shape barrier.
- **Make Operations Idempotent.** The access-review effect reserves or replays
  its request before one atomic set-union merge, including an E11000 retry for
  simultaneous first approvals.
- **Fix Root Causes.** The accepted design addresses the route denial and the
  unrestricted bridge claims as separate authorities, and binds the required
  proxy-slug exception to the same live grant instead of opening platform rows.
- **Prove It Works.** The record labels source-only Aevatar rows separately from
  runtime NyxID rows. It reports the missing .NET runtime and the expired
  production token instead of inventing observations.
- **Sequence Work into Verifiable Units.** The probe tests route reachability,
  claim authority, callback bootstrap, consent revocation, and cross-user
  isolation independently. AC-3 owns the missing authorities and must pass a
  separate security review before rollout.
- **Build the Lever.** The dependency-free probe is rerunnable and mode-gated.
  It keeps production reads separate from the one-write chat transaction.
