# Aevatar access-review authority decision

**Status.** Use consent-derived delegated restrictions as the AC-3 authority
model. Service access review is not reachable from the current assistant
bridge, so preserve unrestricted forwarding until AC-3 passes its security and
deployment gates.

This record is the AC-2 probe and authority decision. It changes no product
behavior. NyxID evidence was collected from local SHA
`3fbae40a7b9526afdc97b3fdc005c1a543dabaaf`. Aevatar source evidence is pinned
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
| Bridge Bearer reads REST MCP config | Local NyxID `3fbae40a` | 403 | none exposed | delegated; actor present; `allow_all_services=true`; 0 allowed services and 0 resources; `proxy:*`; 300 seconds | Route denied before catalog filtering |
| Restricted Bearer uses LLM gateway | Local NyxID `3fbae40a` | 200 | none | delegated; actor present; `allow_all_services=false`; 1 allowed service; 0 resources; `proxy`; 300 seconds | Pinned gateway callback is reachable |
| Restricted Bearer uses platform proxy slug | Local NyxID `3fbae40a` | 403 | none exposed | same restricted summary | Legacy platform-row scope gate denies the callback |

The machine receipt is `/tmp/ac2-evidence/seven-row-receipt.json`. Local setup
statuses are in `/tmp/ac2-evidence/local-runtime-setup.json`.

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
does not call this transport for either access check.

## Authority candidates

| Candidate | Threat model | Revocation and retry | Deployment | Rollback |
| --- | --- | --- | --- | --- |
| Consent-derived delegated restrictions | Least authority can exclude unapproved user services. It is safe only with a live grant bound to `jti`, subject, actor client, receiving client, service IDs, resources, and expiry. A bare scope claim is insufficient. | Consent revocation must invalidate the online grant and the next minted capability. Confirmation retry must atomically merge the same service without replacing existing scopes or IDs. | Requires the live-grant-backed REST read and the validated Aevatar client link. The pinned gateway survives restriction. The platform proxy slug does not, but no receipt shows that Aevatar calls it. | Disable the access-review action and effect first. Then stop restricted minting, restore unrestricted bridge minting, verify both auth inputs, and remove the REST exception and client link. Keep old grants deny-by-default until expiry. |
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
   proxy and management route remains denied.
2. **Restricted platform callback.** The configured gateway already suffices.
   `llm_gateway::gateway_request` starts with only
   `ensure_llm_proxy_access` (`backend/src/handlers/llm_gateway.rs:557-575`),
   and the local restricted receipt is 200. The ordinary platform proxy fails.
   `execute_proxy_inner` returns `ApiKeyScopeForbidden` on any legacy
   `DownstreamService` when `allow_all_services=false`
   (`backend/src/handlers/proxy.rs:2011-2033`). If
   `/proxy/s/chrono-llm-public` remains a supported callback, add an exemption
   bound to the exact platform row, verified Aevatar actor and receiving
   client, and live catalog grant. Do not broadly bypass the gate. The
   server-chosen assistant ingress already uses `execute_admin_proxy`, which
   intentionally skips this caller-addressed gate
   (`backend/src/handlers/proxy.rs:1439-1481`). This exemption is not required
   for AC-3 unless an Aevatar receipt shows a call to the proxy-slug path.
3. **Durable Aevatar client link.** Add an admin-managed
   `delegated_authority_client_id` field to the `aevatar`
   `DownstreamService`, and validate that it references an active OAuth client
   on create and update. Do not reuse `oauth_client_id`. That field means the OIDC
   client used when `auth_method="oidc"`
   (`backend/src/models/downstream_service.rs:220-222`). A row field is
   preferable to environment configuration because the row already owns the
   integration's base URL, identity audience, token forwarding, and delegation
   policy. It can be inspected, validated, audited, and rolled back with the
   integration. A process setting would permit row/config drift and make
   multiple Aevatar rows ambiguous. The pinned upstream client is
   `a6ff2946-f02f-4c35-8203-1ec46132b660`
   (`src/Aevatar.Mainnet.Host.Api/appsettings.json:41-44`), but the current
   NyxID row has no field that resolves that identity.

## Decision predicate

| Condition | Current system | Candidate design | Evidence and AC-3 proof |
| --- | --- | --- | --- |
| Restricted chat bootstrap still works | **Pass on the configured path.** The restricted local request to `/api/v1/llm/gateway/v1` returned 200. The proxy-slug request returned 403, but the pinned Aevatar configuration names the gateway path. | **Pass.** Keep `handlers/llm_gateway.rs:gateway_request` as the callback. Do not add a proxy-slug exemption unless an Aevatar receipt establishes that dependency. | Seven-row restricted callback rows; pinned `GatewayEndpoint` at `appsettings.json:16-20` |
| Exact Aevatar OAuth client resolves without guessing | **Not linked.** Aevatar and the registered NyxID client agree on `a6ff2946-f02f-4c35-8203-1ec46132b660`, but `DownstreamService` cannot express that relationship today. | **Pass.** Add the validated admin-managed `delegated_authority_client_id` field to the `aevatar` row. The field must resolve the registered active client before minting. | Pinned `BackendConsole.OidcClientId`; production client observation; `DownstreamService` field audit; AC-3 validation tests |
| Approving a service changes the next minted capability | **Not implemented.** Revoking consent changed the consent read but left the next current-bridge summary unrestricted. | **Pass.** Mint each assistant capability from current consent for the linked client. An atomic approval merge must therefore change the next minted service list, while revocation must remove that authority. | Negative-case consent receipt; AC-3 before, apply, retry, next-mint, and revoke tests |

The current bridge fails the last two conditions by construction. Those results
explain why access review is unreachable today. They do not falsify the
candidate that adds the missing client link and derives every new capability
from current consent.

The consent-derived candidate satisfies the predicate and is accepted for
AC-3. A complete OAuth round trip is rejected because it adds a browser return
protocol without improving the chosen authority model. Unrestricted forwarding
remains the deployed and rollback posture until AC-3 proves the replacement.

### Accepted operational design

AC-3 may implement consent-derived delegated restrictions under its own
security review. It must add the exact GET-only `/api/v1/mcp/config` allowance
backed by a verified live `CatalogDelegationGrant`, while every other route in
the `mcp` class remains denied. It must also add the validated
`delegated_authority_client_id` link and mint each assistant capability from the
linked client's current consent. AC-3 must split the existing route-policy test
into no-grant denial and matching-live-grant success cases.

Do not add a `/proxy/s/chrono-llm-public` exemption unless a new receipt proves
that Aevatar uses that callback. The pinned Aevatar deployment configures the
gateway path, and that path passed the restricted-token probe.

Until AC-3 lands and proves that Aevatar receives both caller identity and the
restricted capability, keep `forward_access_token=true` and preserve the
current unrestricted forwarding posture. Access review remains unreachable in
that interim state for the independent route-denial and allow-all reasons above.

## AC-3 change map

This map defines the symbols for the authorized consent-derived implementation.
AC-3 remains subject to its own security review gate.

| Responsibility | Symbols |
| --- | --- |
| Atomic consent merge | Add `consent_service::merge_consent_services_atomic` in `backend/src/services/consent_service.rs`. Use one atomic aggregation-pipeline `find_one_and_update` keyed by `(user_id, client_id)`. Union the requested service ID and required scopes while preserving all existing scopes, IDs, `allow_all_services`, grant identity, and unrelated fields. Never call `grant_consent_with_services`. `grant_consent_internal` replaces the full row at lines 71-100. The unique `(user_id, client_id)` index is at `backend/src/db.rs:911-920`. |
| HTTP effect | Add the confirmed `service.access_review` effect in `backend/src/handlers/assistant_action_effects_services.rs` and mount only its typed route in `backend/src/routes.rs`. Resolve actor, client link, and user-owned service server-side; reject cross-user IDs before the merge. |
| Authority mint | Replace the session projection in `backend/src/handlers/assistant.rs:build_forward_authorization`. Resolve `delegated_authority_client_id`, read current consent, derive `CatalogAuthority`, call `generate_delegated_access_token_for_client` so `client_id` is present, then persist the matching online grant with `catalog_delegation_service::persist_grant`. Do not derive service restrictions from session `AuthUser`. |
| REST catalog guard | Change `mw/auth.rs:delegated_request_allowed`, `reject_delegated_tokens`, and their route tests; add verified live-grant enforcement before `handlers/mcp.rs:get_mcp_config` publishes `mcp_service::ServiceScope`. |
| Platform callback | Keep `handlers/llm_gateway.rs:gateway_request` as the proven path. No proxy-slug exemption is required. If a later Aevatar receipt shows that callback, narrow the legacy gate in `handlers/proxy.rs:execute_proxy_inner` to the exact live-grant-bound Aevatar platform callback. |
| Client link | Extend `models/downstream_service.rs`, the admin service request and response DTOs in `handlers/services.rs`, and admin validation so `delegated_authority_client_id` always names an active OAuth client. No environment variable is added. |
| Audit | Append metadata-only events `assistant_service_access_review_requested`, `assistant_service_access_grant_merged`, `assistant_service_access_grant_replayed`, `assistant_service_access_grant_denied`, and `assistant_delegated_authority_minted`. Include actor ID, owner ID, service ID, client ID, action request ID, outcome, and online-grant ID where applicable; never include a token, credential, cookie, or authorization value. Preserve the existing `oauth_consent_revoked` event and make grant invalidation observable. |

Required tests include database cases for the state before the merge, the first
apply, an idempotent retry, revocation, and a cross-user request. They also
include concurrent merges of different service IDs, live-grant expiry and
revocation, a missing/inactive/mismatched client link, exact-route denial
without a grant, catalog filtering, gateway bootstrap, continued proxy-slug
denial unless a dependency is observed, and audit metadata redaction.

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
2. Stop issuing restricted assistant grants and restore the current
   unrestricted session bridge.
3. Verify typed chat, identity assertion receipt, and capability receipt using
   the pinned gateway.
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
- **Boundary Discipline.** Tokens stay inside the probe process. The probe
  reconstructs output field by field and applies a final secret-shape barrier.
- **Fix Root Causes.** The accepted design addresses the route denial and the
  unrestricted bridge claims as separate authorities. It does not add a
  proxy-slug exemption without evidence that Aevatar uses that path.
- **Prove It Works.** The record labels source-only Aevatar rows separately from
  runtime NyxID rows. It reports the missing .NET runtime and the expired
  production token instead of inventing observations.
- **Sequence Work into Verifiable Units.** The probe tests route reachability,
  claim authority, callback bootstrap, consent revocation, and cross-user
  isolation independently. AC-3 owns the missing authorities and must pass a
  separate security review before rollout.
- **Build the Lever.** The dependency-free probe is rerunnable and mode-gated.
  It keeps production reads separate from the one-write chat transaction.
