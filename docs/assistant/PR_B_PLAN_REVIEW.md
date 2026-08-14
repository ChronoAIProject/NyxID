# PR-B Plan Adversarial Review

## Verdict: REWORK

PR-B is not ready to implement. The operation allowlist can be placed before side effects in both executors, and the measured token-lifetime numbers are reasonable, but the resource-token contract does not yet provide cross-user isolation. As written, a token for one order can authorize a request for another order; MCP cannot return or present the token at all; organization-owned services mint tokens that no human session can exchange; and the reissue route lacks enough policy to perform its promised server-side proof. The canonical-path and actor-construction contracts also claim guarantees the current interfaces cannot supply. These are architecture defects, not mechanical omissions, so implementation must stop until the plan defines the missing security boundaries.

## Blocking findings

### B1. A token for one order is not bound to the order being requested

**Consequence:** A customer who possesses a valid token for their own order can use it to read or mutate another customer's order if they learn that order ID.

**Classification:** **WRONG.** The plan claims that protected operations prove "whose order is whose," but the authorization interface never receives the requested resource identity.

**Claim attacked:** The token's `res` claim and the protected-operation check together enforce actor-to-resource attribution (`docs/assistant/PR_B_PLAN.md:14`, `:67-73`, `:84`).

**Concrete failure scenario:**

1. User A creates `ord_A` and receives a correctly signed token with `res = "duffel:order:ord_A"`.
2. User A requests `GET /air/orders/ord_B` and supplies the token for `ord_A`.
3. The GET path matches the allowlist rule, the signature and subject are valid, and `requires_resource_token` is satisfied.
4. `authorize_proxy_operation` receives the method, canonical path, actor, and token, but no request body or extracted path resource. Nothing in its contract compares `ord_B` with `ord_A`, so the request can be forwarded.

**Evidence:** `ProxyOperationRule` has only a body-oriented `resource_token_id_path`, with no path-template capture contract (`docs/assistant/PR_B_PLAN.md:52-62`). The shared authorization function has no body or requested-resource parameter (`docs/assistant/PR_B_PLAN.md:67-73`). `path_pattern` is specified as a `globset` pattern, which matches but does not define a named resource-ID capture (`docs/assistant/PR_B_PLAN.md:54-55`).

**Proposed fix:** Require every protected rule to declare exactly one validated resource extractor: either a named path-template capture or a JSON body path. Convert that to a `RequestedResource { kind, id }` before authorization and compare both fields exactly with the verified token's resource claim. Reject admin writes where `requires_resource_token` is true but no unambiguous extractor exists. Add tests proving that a token for A cannot authorize path- or body-addressed resource B in REST, typed MCP, and generic MCP.

### B2. MCP has no transport for returning or presenting resource tokens

**Consequence:** An agent that creates an order through MCP cannot receive the token needed for the next operation, and even an agent given a token out of band cannot present it to a protected MCP tool.

**Classification:** **WRONG.** The plan promises token enforcement in both MCP forms, but the current MCP boundary transports only a status and body while the proposed token exists only in an HTTP response header.

**Claim attacked:** Both typed and generic MCP operations mint and enforce the same resource token as REST (`docs/assistant/PR_B_PLAN.md:76`, `:84`, `:98-99`).

**Concrete failure scenario:**

1. An agent invokes the typed or generic MCP create-order tool.
2. Duffel returns a successful JSON order, and NyxID mints `X-NyxID-Resource-Token` as specified.
3. `execute_tool` consumes the downstream response and returns only `(status, body)`, so the MCP caller never receives the header.
4. On a later protected read or payment, `execute_tool` receives only tool arguments. The schema currently blocks all `X-NyxID-*` header arguments, and there is no reserved token argument. Authorization must therefore reject every legitimate call or silently omit enforcement.

**Evidence:** `execute_tool` returns `AppResult<(u16, String)>` (`backend/src/services/mcp_service.rs:3037-3063`). Its direct path reads the body and discards headers (`backend/src/services/mcp_service.rs:3575-3589`); its node path similarly reduces a response to status/body even though the node response contains headers (`backend/src/services/mcp_service.rs:3497-3507`; `backend/src/services/node_ws_manager.rs:42-47`). Both MCP call sites destructure only `(status, body)` (`backend/src/handlers/mcp_transport.rs:1331-1353`, `:1680-1702`). MCP parameter generation blocks every `x-nyxid-*` header (`backend/src/services/mcp_service.rs:2399-2403`).

**Proposed fix:** Define an executor-neutral result containing status, headers, and body. Define a reserved MCP argument such as `_nyxid_resource_token`, consume it inside NyxID, and never forward it downstream. Specify how protected typed and generic tool schemas expose that argument and how minting tools return the token as structured MCP output. Audit every `execute_tool` caller, including approval and direct-agent paths, for preservation or explicit rejection of token-bearing operations. Test a complete MCP create -> receive token -> protected read flow for direct and node responses in both tool forms.

### B3. One `CanonicalPath::parse(&str)` cannot safely serve both REST and MCP inputs

**Consequence:** The same encoded path can be interpreted differently depending on entry point, leaving an allowlisted operation reachable through an alternate encoding even though the policy appears identical.

**Classification:** **UNPROVEN.** Rejecting encoded separators and dot segments is the right rule, but the plan does not define a common raw input on which that rule operates.

**Claim attacked:** One constructor decodes exactly once and supplies the path used for both matching and forwarding in both executors (`docs/assistant/PR_B_PLAN.md:20`, `:78-80`).

**Concrete failure scenario:**

1. A REST caller sends an encoded separator such as `%2F`; Axum decodes the wildcard before passing the `path` argument to the handler.
2. An MCP caller supplies the same characters as a raw argument string.
3. Calling one percent-decoding constructor on both values decodes REST twice relative to the wire but MCP once. Avoiding the decode has the opposite problem.
4. Matching or forwarding can therefore differ across executors, especially for double-encoded input, even though both claim to use the same canonical path.

**Evidence:** REST handlers receive `Path((service_id, path))` or `Path((slug, path))` (`backend/src/handlers/proxy.rs:571-576`, `:794-799`). The repository has an explicit test proving Axum turns `%2F` into `/` in that wildcard (`backend/src/handlers/proxy.rs:5651-5677`), and REST separately validates `OriginalUri` before using the extracted path (`backend/src/handlers/proxy.rs:619-628`, `:841-850`). MCP receives and validates a raw `path` argument directly (`backend/src/services/mcp_service.rs:3592-3645`).

**Proposed fix:** Define the authorization input at the wire boundary. Either use separate `CanonicalPath::from_raw_uri_path` and `CanonicalPath::from_decoded_segments` constructors with one shared invariant, or derive REST's downstream tail from `OriginalUri` and use raw-path input in both pipelines. State whether percent escapes are retained or decoded for forwarding, not merely matching. Add parity tests that send every adversarial spelling through REST and MCP and assert the same allow/deny decision and identical downstream request target.

### B4. `EffectiveActor::for_auth_user` does not preserve the claimed type-level seal

**Consequence:** Internal code can fabricate an `AuthUser`, obtain an `EffectiveActor`, and mint or verify tokens as a synthetic user without passing through authentication middleware.

**Classification:** **WRONG.** Taking `&AuthUser` instead of `&str` narrows the API cosmetically, but `AuthUser` is not an extractor-only capability in this repository.

**Claim attacked:** A public `for_auth_user(&AuthUser)` constructor preserves the guarantee that only a real authenticated principal can create an effective actor (`docs/assistant/PR_B_PLAN.md:30-32`).

**Concrete failure scenario:**

1. Any backend module constructs `AuthUser { user_id: victim_id, ... }`; all fields needed to do so are public.
2. It calls the proposed public `EffectiveActor::for_auth_user(&synthetic)`.
3. The returned actor is indistinguishable from one created from the Axum extractor and can enter the token or credential authorization path.

**Evidence:** Every `AuthUser` field is public (`backend/src/mw/auth.rs:47-87`). Production code already constructs one outside the extractor in `channel_event_service` (`backend/src/services/channel_event_service.rs:426-448`), and additional direct constructions exist in handlers and tests. The currently sealed `EffectiveActor` has a private field and private constructor (`backend/src/services/proxy_service.rs:82-96`); the proposed public constructor would expand that boundary based on a false premise.

**Proposed fix:** Introduce an extractor-produced principal/newtype with private fields and no general production constructor, then accept that capability at the resource-token service boundary. Alternatively, explicitly downgrade the type-level claim and keep actor construction private, with resource-token handlers calling a narrow proxy-service operation that receives verified handler context and performs the conversion internally. Do not publish `for_auth_user` under the current claim.

### B5. Organization-owned services mint tokens that no human session can exchange

**Consequence:** A person booking through an organization-owned service receives a valid token but can never open the human payment flow with it.

**Classification:** **WRONG.** Mint and exchange use incompatible identities for organization access.

**Claim attacked:** `sub = effective_owner_id`, `sub == session user` at exchange, and the org-member mint-to-verify test all describe one coherent subject (`docs/assistant/PR_B_PLAN.md:84`, `:92-94`, `:99`).

**Concrete failure scenario:**

1. Person U invokes an organization-owned Duffel service; proxy resolution's effective owner is organization O.
2. NyxID mints the resource token with `sub = O`.
3. U opens the payment component and calls the human-session exchange route.
4. The route compares `sub` with U's session user ID, so O != U and rejects.
5. No caller can succeed because organization O cannot have a login session.

**Evidence:** The plan binds `sub` to the effective owner and later requires equality with the session user (`docs/assistant/PR_B_PLAN.md:84`, `:93`). NyxID's organization contract states that organization users cannot authenticate and all access goes through person members (`docs/site/shared/concepts/organizations.md:74-78`).

**Proposed fix:** Carry both identities explicitly. Use the authenticated person/agent identity as the token subject and a separate effective-owner claim for the service/account boundary, then define whether and when current organization membership is revalidated. Specify API-key, service-account, and delegated mint semantics separately from human exchange. If organization-backed travel is intentionally unsupported, reject it before the provider write and remove the org-member success test.

### B6. The reissue route cannot derive a provider read from the declared policy

**Consequence:** If minting fails after Duffel creates an order, or a multi-day token expires, the user is told to use a recovery route that cannot determine what to fetch or how to prove ownership.

**Classification:** **WRONG.** The route is specified, but the configuration it needs is absent and internally contradictory.

**Claim attacked:** `POST /resource-tokens/reissue {service_slug, resource_id}` can perform a credentialed server read and mint after matching a configured owner email (`docs/assistant/PR_B_PLAN.md:90`, `:94`).

**Concrete failure scenario:**

1. Duffel creates `ord_X`, but the create response is malformed, compressed, too large, or missing the configured ID, so NyxID returns the controlled error.
2. The user calls reissue with only the service slug and `ord_X`.
3. The server has no configured resource kind, provider read method/path template, or declared owner-email extractor with which to construct and validate the read.
4. With multiple mint rules or resource kinds, it also has no deterministic rule to select. Recovery cannot execute safely.

**Evidence:** The declared policy includes create-response ID and expiry paths plus an optional protected-operation body path (`docs/assistant/PR_B_PLAN.md:48-62`). It contains no reissue read template or resource-kind dispatch key. The route text refers to `owner_email_path` "configured on the mint rule" (`docs/assistant/PR_B_PLAN.md:94`), but `ResourceTokenMint` as shown has no such field (`docs/assistant/PR_B_PLAN.md:57`).

**Proposed fix:** Add a complete, uniquely keyed reissue descriptor per resource kind: validated ID format, canonical server-read method/path template, owner-evidence extractor, and expiry extractor. Validate uniqueness and safe path substitution at admin write. If reissue is provider-specific, move the route and its dispatch contract to PR-C/D when the Duffel policy and fixtures exist; PR-B can still ship token verification without claiming generic recovery.

## Non-blocking findings

### N1. The proposed "human-only" router still accepts ordinary OAuth access tokens

**Consequence:** A non-browser OAuth client may reach exchange or reissue even though the plan describes those endpoints as browser-session-only.

**Classification:** **UNPROVEN.** The existing router rejects several machine credential classes, but it does not require session authentication.

**Failure scenario:** An OAuth client presents an ordinary bearer access token. It is neither delegated, an API key, a service account, nor a relay token, so all four router middleware checks pass and the handler runs.

**Evidence:** `AuthMethod::AccessToken` is distinct from `Session` (`backend/src/mw/auth.rs:27-45`). The human-only router layers only the delegated, API-key, service-account, and relay rejection middleware (`backend/src/routes.rs:1501-1564`).

**Proposed fix:** Add and test a session-only guard for these routes, explicitly requiring `auth_method == Session` and a valid session identifier. Keep this separate from the broader existing human-only router unless changing all of its routes is intended.

### N2. Key overlap requires a real verification model, not only a runbook

**Consequence:** A routine signing-key rotation can invalidate every outstanding multi-day order token before payment completes.

**Classification:** **UNPROVEN.** The 14-day overlap is sufficient arithmetically, but current key loading and publication support only one key.

**Failure scenario:** Deployment switches from key A to key B. Existing tokens name A's `kid`, but the process holds only B's decoding key and the JWKS endpoint publishes only B. A token with two days remaining immediately fails.

**Evidence:** `JwtKeys` has one encoding key, one decoding key, and one `kid` (`backend/src/crypto/jwt.rs:16-23`, `:207-255`). Startup reads and publishes one public key (`backend/src/main.rs:351-361`). The plan adds a previous-key path and runbook but does not state whether overlap is resource-token-only, how `kid` selects a verifier, or whether JWKS publishes both (`docs/assistant/PR_B_PLAN.md:88`).

**Proposed fix:** Specify the key-ring data structure, `kid` lookup, startup validation, JWKS behavior, and deployment order. Decide whether previous-key verification applies only to resource tokens or all JWT classes. Refuse startup or emit a hard operational error when a configured previous-key overlap is invalid; a prose minimum cannot enforce itself.

### N3. New platform-credential services can accidentally remain default-open

**Consequence:** An administrator can create the future shared Duffel row without a policy and recreate the cross-user list exposure this PR is intended to prevent.

**Classification:** **TRADEOFF with an unsafe creation default.** `None = passthrough` is necessary for legacy compatibility, but it is not a safe default for newly stored platform credentials.

**Failure scenario:** An admin creates a public internal service with a master credential but omits `proxy_operation_policy`. The credential gate admits the row, and the authorization layer deliberately passes every operation.

**Evidence:** The plan defines `None = passthrough` (`docs/assistant/PR_B_PLAN.md:48-50`). Service creation is admin-only (`backend/src/handlers/services.rs:686-707`), while updates are available to an admin or original creator (`backend/src/handlers/services.rs:1281-1307`), so both write paths need an explicit invariant.

**Proposed fix:** Preserve `None` for existing rows but require a non-empty, validated policy whenever an admin create/update would leave a catalog row with a usable platform credential. At minimum, make the future Duffel seed fail startup/backfill validation without policy. Ensure an empty `rules` list means deny-all, never allow-all.

### N4. Post-write mint failure is not a recoverable 502 until reissue is defined

**Consequence:** The provider may hold a real order while NyxID reports failure, inviting an agent to create a duplicate order.

**Classification:** **UNPROVEN.** Failing closed protects confidentiality, but the caller needs an explicit unknown-outcome contract.

**Failure scenario:** The create request succeeds at Duffel, but token extraction misses. NyxID returns 502. An agent interprets that as no order and retries create because it never received the resource ID or token.

**Evidence:** The plan acknowledges that the provider write may exist and says the error names reissue (`docs/assistant/PR_B_PLAN.md:90`), while B6 shows that reissue is not implementable from the current policy.

**Proposed fix:** After fixing B6, define a distinct provider-outcome-unknown error with correlation metadata and agent guidance that forbids blind recreation. Test that no success is returned without a token and that the recovery response can identify the existing resource without leaking it cross-user.

### N5. Matcher validation must reject rules broader than the administrator can see

**Consequence:** A syntactically valid glob can unintentionally authorize descendants or sibling-looking paths beyond the intended provider operation.

**Classification:** **UNPROVEN.** The plan says globs are validated but does not define the accepted grammar or dangerous constructs.

**Failure scenario:** An administrator intends one path and writes a broad pattern such as `/air/orders/**` or a character class with unexpected scope. It compiles, passes validation, and exposes more provider operations than the overlay publishes.

**Evidence:** `path_pattern` is only described as `globset; compiled + validated at admin write` (`docs/assistant/PR_B_PLAN.md:54-55`); no grammar, anchoring rule, or overlap diagnostic is specified.

**Proposed fix:** Define a deliberately small path-template grammar rather than exposing general glob semantics, or document exact anchoring and reject recursive wildcards, alternation, and patterns that do not begin at the canonical root. Return the normalized operation set in admin responses so review can see what will execute.

## Claims I verified and confirmed correct

- **There are two independent executors.** REST/WS execution is in `handlers/proxy.rs`, while MCP builds and forwards its own request in `mcp_service::execute_tool` (`backend/src/services/mcp_service.rs:3037-3075`, `:3412-3419`, `:3568-3589`). A shared primitive must be called explicitly by both.
- **A shared pre-side-effect authorization point is feasible.** REST has the final buffered body before approval at `backend/src/handlers/proxy.rs:1817-1859`, and billing opens later at `:2131`. MCP has final method/path/body at `backend/src/services/mcp_service.rs:3069-3075`, while billing opens at `:3398-3410` and forwarding begins at `:3412-3419`.
- **The existing REST code already recognizes Axum's path-decoding hazard.** It validates `OriginalUri` before proceeding (`backend/src/handlers/proxy.rs:619-628`, `:841-850`) and tests percent-encoded separator decoding (`:5651-5677`). The plan should build on that control rather than replace it with an ambiguous parser input.
- **Node responses retain the information needed for response inspection.** `NodeProxyResponse` contains status, headers, and raw body (`backend/src/services/node_ws_manager.rs:42-47`). The loss occurs later in MCP and can be corrected once its result contract is redesigned.
- **Legacy compatibility can be preserved.** A `None` policy can remain passthrough for existing services; the defect is allowing newly credentialed shared rows to omit policy, not the compatibility behavior itself.
- **The primary service create surface is admin-only.** `create_service` calls `require_admin` (`backend/src/handlers/services.rs:686-707`). The update surface is admin-or-creator and must receive the same policy validation (`:1281-1307`).
- **The measured lifetime correction is supported by the committed fixture.** The sample has `payment_required_by` three days after capture and a price guarantee ending one day earlier (`docs/assistant/fixtures/duffel-offer-request-sample.json:1-15`). A 96-hour maximum token lifetime and 14-day old-key overlap cover this measured case; the open issue is enforcing rotation correctly.
- **Fail-closed response inspection constraints are directionally sound.** Requiring 2xx JSON, identity encoding, a bounded body, and a validated ID before minting prevents a downstream response from choosing arbitrary token claims (`docs/assistant/PR_B_PLAN.md:90`), provided the unknown-outcome recovery contract is completed.

## Claims I could not verify

- **Duffel component-key exchange and ownership evidence:** The exact request/response schema and the stable owner-email field are intentionally deferred to PR-C/D. Verification needs captured Duffel fixtures or current official API documentation for the component client-key endpoint and order representation.
- **Aevatar workflow behavior:** The wait-loop cap, checkpoint behavior, and `self_reschedule` fallback live outside this repository. Verification needs the deployed Aevatar workflow engine source/version and an integration run across multiple wait legs.
- **Production key rotation:** Repository code currently loads one key. Verification of a 14-day overlap needs the deployment manifests, secret-mount process, and rollout order in addition to the proposed code.
- **Organization-backed travel intent:** The plan asks for org-member agreement, but it does not state whether organization-owned Duffel services are a supported product mode. A product decision is needed before choosing dual identity claims versus an explicit pre-write rejection.
- **Provider response encoding behavior at scale:** The plan requires identity encoding and a 1 MiB body, but no live evidence shows that every eligible create-order response satisfies those constraints. Verification needs representative live/test Duffel responses with large passenger and segment counts.

# Round 2

## Verdict: REWORK

Revision 2 closes three important defects from Round 1: it gives REST and MCP distinct canonical-path constructors, removes the unsafe `EffectiveActor` constructor, and gives MCP an explicit token presentation channel. It still cannot be implemented as a security boundary. MCP currently creates and persists approval data before `execute_tool`, so the proposed check is too late and the reserved token can be interpreted as provider input before it is stripped. Protected rules also lack the resource kind required for an exact token comparison; organization-owned agent keys still mint an organization subject that no person can exchange; and credentialed rows remain default-open at runtime despite the new admin validation. The proposed MCP result object is not a valid, provenance-distinct MCP content block, and the requirement that reissue precede production minting is still only prose. These defects affect the authorization order, token identity, and fail-closed behavior, so they are architecture-blocking rather than inline implementation choices.

## Blocking findings

### R2-B1. MCP approval runs before the proposed authorization and token-consumption point

**Consequence:** A protected MCP request can create an approval request before its resource token is checked, and the token itself can be folded into the provider body and persisted in approval text before the plan says it is removed.

**Classification:** **WRONG.** The plan says the shared check runs before approval and places the MCP call in `execute_tool`; current control flow performs approval before calling `execute_tool`.

**Claim attacked:** The operation check is called after final request construction but before approval, and `_nyxid_resource_token` is consumed by NyxID and never treated as downstream input (`docs/assistant/PR_B_PLAN.md:20-24`, `:91-105`).

**Concrete failure scenario:**

1. An MCP caller invokes a protected typed tool with `_nyxid_resource_token = <secret>`.
2. The handler calls `build_mcp_operation_descriptor` on the unmodified arguments.
3. For a flattened JSON request body, `build_proxy_args` classifies any unknown top-level argument as a body field. The reserved token therefore enters the synthesized provider body.
4. `authorize_mcp_tool_operation` derives an action description from that body and may persist it in an approval request.
5. Only after approval completes does the handler call `execute_tool`, the location where the plan currently places the shared check and token consumption.

The same sequence exists in both direct tool dispatch and `nyx__call_tool`.

**Evidence:** Direct dispatch builds the operation and runs approval at `backend/src/handlers/mcp_transport.rs:1314-1329`, then calls `execute_tool` at `:1331-1353`. Meta-tool dispatch repeats that order at `:1652-1663` and `:1680-1702`. `build_mcp_operation_descriptor` calls the same argument builders used for forwarding (`backend/src/services/mcp_service.rs:2731-2747`), and unknown typed-tool arguments become JSON body fields (`:2638-2667`). The descriptor includes a body-derived action description (`backend/src/services/operation_descriptor.rs:99-108`), which is copied into the persisted approval row (`backend/src/services/approval_service.rs:405-415`, `:501-518`).

**Proposed fix:** Define one MCP preparation primitive called before both approval and execution, for example `prepare_proxy_tool_call(service, endpoint, arguments) -> PreparedProxyCall`. It must extract and validate the reserved token, remove it from a cloned argument object, construct the final method/canonical path/query/headers/body exactly once, run operation authorization, and derive the approval descriptor only from the sanitized provider request. Both direct and meta-tool dispatch must pass that prepared value into execution rather than rebuilding from raw arguments. Tests must assert that the token is absent from the provider body, node request, approval action description, audit data, and tracing output, including when approval denies or times out.

### R2-B2. Protected rules still do not declare the resource kind being authorized

**Consequence:** The server cannot construct the exact requested resource identity promised by the plan without trusting the token itself to supply part of that identity.

**Classification:** **WRONG.** Revision 2 adds exact ID extraction but omits the corresponding kind from a protected rule.

**Claim attacked:** A protected rule extracts `RequestedResource { kind, id }` and compares the token's complete `res = "{kind}:{id}"` value exactly (`docs/assistant/PR_B_PLAN.md:71-75`, `:93-103`).

**Concrete failure scenario:**

1. A protected `GET /air/orders/{id}` rule matches `ord_X`.
2. The rule has `requires_resource_token = true` and captures `id`, but `mints_resource_token` is absent because this is a read, not a create.
3. No field on the rule says that the capture represents `duffel:order`.
4. The implementation must either reject every call, infer the kind from the presented token, or compare only the ID. The latter two allow a token of another configured resource kind with the same identifier to satisfy a rule that was supposed to require an order token.

**Evidence:** `ResourceTokenMint` carries `resource_kind`, but only minting rules contain that structure (`docs/assistant/PR_B_PLAN.md:67-83`). A protected non-minting rule contains only `requires_resource_token` and `resource_token_id_path` (`:69-75`). Nevertheless, the authorization contract says it derives both `kind` and `id` from the rule (`:99-102`).

**Proposed fix:** Replace the two loose protected fields with a required descriptor such as `requires_resource_token: Option<ResourceTokenRequirement>`, containing `resource_kind` and exactly one ID source. Validate the kind and ID pattern at admin write. Authorization must derive both expected fields exclusively from the matched policy and request, then compare them with separately parsed token claims; it must never take the expected kind from the token being checked.

### R2-B3. Organization-owned agent keys still cannot produce a human-exchangeable token

**Consequence:** An organization can use its supported shared agent key to create an order, but no administrator or member can open the human payment flow for that order.

**Classification:** **WRONG.** The two-claim model fixes a personal member's key, but the statement that an agent key's owner can later hold a browser session is false for organization-owned keys.

**Claim attacked:** `sub` is the authenticated person or agent key owner "who can later hold a browser session," while `owner` is the effective organization; an organization booking can therefore be exchanged by its human booker (`docs/assistant/PR_B_PLAN.md:26-28`, `:118`, `:127`, `:132`).

**Concrete failure scenario:**

1. Organization admin U creates a supported organization-owned API key for organization O.
2. The key authenticates as O, not U, and an agent creates an order with it.
3. The revised rule mints `sub = O` and `owner = O`.
4. U later calls the session-only exchange route. `sub == session user` fails because O != U.
5. No human can satisfy the check because organization users cannot authenticate. Unlike the service-account case, the plan neither rejects this flow nor identifies it as non-exchangeable.

**Evidence:** The API explicitly supports `target_org_id`; the resulting key's `user_id` is the organization and callers authenticate as that organization (`backend/src/handlers/api_keys.rs:125-130`). API-key authentication copies the key owner into `AuthUser.user_id` (`backend/src/mw/auth.rs:585-623`) and MCP does the same (`backend/src/handlers/mcp_transport.rs:373-401`). Organization users have no session (`docs/site/shared/concepts/organizations.md:74-78`).

**Proposed fix:** Make an explicit product/security choice. Either prohibit minting operations for organization-owned API keys before the provider write, as already proposed for service accounts, or introduce a separately authenticated human sponsor bound when the organization key is issued/invoked. Do not infer the creating admin from an organization-owned bearer key: that attribution is not present in the request and all organization admins can possess the same key. Add the organization-owned-key case to the subject matrix and acceptance tests.

### R2-B4. Credentialed services are still default-open at runtime

**Consequence:** A new catalog writer or seed that forgets the policy can expose every provider operation under a shared master credential even though the PR claims such a row is unusable.

**Classification:** **WRONG.** Admin create/update validation narrows two write paths, but the data-plane rule remains `None = passthrough`, and a startup warning is not fail-closed enforcement.

**Claim attacked:** A newly created platform-credential row cannot be saved without a policy and creating the future Duffel row without an allowlist is impossible (`docs/assistant/PR_B_PLAN.md:13-16`, `:53-58`, `:131`).

**Concrete failure scenario:**

1. A seed, migration, maintenance script, or future backend writer inserts a public internal service containing a usable master credential but leaves `proxy_operation_policy = None`.
2. The admin-handler validation never runs. The existing catalog seeder demonstrates that backend code writes `DownstreamService` documents directly.
3. Startup emits only the planned warning.
4. Runtime sees `None` and deliberately passes every method/path, recreating the cross-user list exposure that PR-B is meant to eliminate.

There is also no reliable model-level `credential non-empty` test: admin create encrypts even an empty plaintext credential, so ciphertext length does not distinguish a stored secret from an empty template.

**Evidence:** The plan explicitly retains runtime passthrough for `None` and specifies only an audit warning for grandfathered rows (`docs/assistant/PR_B_PLAN.md:53-58`). The catalog seeder constructs and directly inserts `DownstreamService` (`backend/src/services/provider_service.rs:3630-3685`). Admin create uses `unwrap_or_default()` for the plaintext credential and encrypts it even when empty (`backend/src/handlers/services.rs:828`, `:889-917`), so `credential_encrypted.is_empty()` cannot implement the proposed condition safely.

**Proposed fix:** Make the exception explicit and data-plane enforceable. At migration, mark the exact legacy rows permitted to retain passthrough with a persisted policy mode/version; at runtime, deny every master-credential injection whose row has neither a policy nor that explicit legacy marker. Preferably inspect an authoritative `has_master_credential` field maintained with credential writes, rather than ciphertext length. All application writers, including seeds, must call shared model/service validation, but runtime denial remains the final control. An empty policy must remain deny-all.

### R2-B5. The proposed MCP token block is not a valid provenance-distinct result shape

**Consequence:** Depending on how the shorthand is implemented, conforming MCP clients either reject the tool result, treat the token as ordinary provider text, or cannot reliably distinguish NyxID metadata from JSON returned by the provider.

**Classification:** **UNPROVEN.** A second MCP `content` entry must be a defined MCP content-block variant; the object shown in the plan has no `type`, and converting it to a text block loses the provenance boundary the attack target requires.

**Claim attacked:** MCP can append `{"nyxid_resource_token", "resource", "expires_at"}` as a structured content block while leaving existing consumers and provider JSON untouched (`docs/assistant/PR_B_PLAN.md:18-24`).

**Concrete failure scenario:**

1. Duffel or another downstream returns JSON containing the same three fields with an invalid or attacker-selected token.
2. NyxID returns the provider body as its existing text block.
3. If the new metadata is also serialized as a text block, an agent that searches content for those keys cannot distinguish the provider's object from NyxID's object. If the raw object shown in the plan is appended directly, it has no MCP content-block `type` and violates the advertised protocol shape.
4. A client that currently requires one text block can also reject or mishandle the newly appended second block despite the claim that existing parsing is unchanged.

The forged token will fail cryptographic verification, so this does not directly authorize another order; it can still misdirect the agent and make the payment artifact unusable.

**Evidence:** NyxID advertises MCP protocol version `2025-11-25` (`backend/src/handlers/mcp_transport.rs:32`). Its current result shape is one typed text content block (`:90-99`), including the SSE variant (`:115-143`). The plan's example is an untyped object (`docs/assistant/PR_B_PLAN.md:22`). The protocol's tool result shape provides top-level `structuredContent` for structured output; individual `content` elements remain typed content blocks (Model Context Protocol 2025-11-25, "Tool Result": `https://modelcontextprotocol.io/specification/2025-11-25/server/tools`).

**Proposed fix:** Keep the existing single provider text block unchanged and place NyxID-owned metadata in the protocol's top-level `structuredContent`, under a namespaced schema whose token is accepted only after signature verification. Return the same structure from normal and SSE-wrapped tool results. Document that consumers must trust only the top-level NyxID structure, never token-looking fields inside provider text. Add a malicious-provider-body test proving it cannot create or replace that top-level field, plus compatibility tests for a no-token response.

### R2-B6. Reissue-before-enablement remains an unenforced sequencing note

**Consequence:** An administrator can enable a minting rule before recovery exists, leaving a real provider order stranded whenever post-write token extraction fails.

**Classification:** **WRONG.** Revision 2 honestly defers reissue, but its claim that minting is unreachable until PR-C/D is not enforced by the proposed model or write validation.

**Claim attacked:** The provider-outcome-unknown branch is unreachable in production because no minting rule exists, and reissue must land before the Duffel row is enabled (`docs/assistant/PR_B_PLAN.md:7`, `:40`, `:124-127`).

**Concrete failure scenario:**

1. PR-B deploys the generic `mints_resource_token` policy field and admin create/update support.
2. An administrator configures a minting rule on any platform-credential service, or PR-C adds the Duffel row but misses the prose sequencing requirement.
3. The provider commits a create while response extraction fails.
4. NyxID returns guidance to use a route that is not mounted. The user cannot recover the signed capability and may retry the provider write.

**Evidence:** `ResourceTokenMint` is part of the PR-B policy accepted at admin write (`docs/assistant/PR_B_PLAN.md:67-83`, `:131`), while reissue is explicitly absent (`:7`, `:126`). No registry dependency, feature fence, startup assertion, or write-time requirement links a minting resource kind to a recovery implementation.

**Proposed fix:** Make enablement depend on code, not rollout memory. Admin validation must reject any production mint rule whose resource kind lacks a registered exchange and reissue/recovery handler; the test-only kind must be unavailable outside tests. PR-C/D can register `duffel:order` atomically with its recovery descriptor before its seed passes validation. Alternatively, omit minting support from PR-B and ship only allowlisting until recovery exists.

## Non-blocking findings

### R2-N1. The reserved MCP argument can collide with a real provider parameter or flattened body field

**Consequence:** A provider operation that legitimately defines `_nyxid_resource_token` can either lose its input or receive NyxID's bearer token.

**Classification:** **UNPROVEN.** The name is described as reserved, but collision validation is not specified.

**Failure scenario:** An OpenAPI operation has a path/query parameter or flattened JSON property named `_nyxid_resource_token`. Schema injection overwrites it, or pre-forward stripping removes the provider value.

**Evidence:** Typed MCP schemas merge request parameters and flattened JSON properties into one top-level object (`backend/src/services/mcp_service.rs:2154-2259`), and the argument builder assigns unknown top-level keys to the body (`:2638-2667`). The plan specifies injection and stripping but no collision rule (`docs/assistant/PR_B_PLAN.md:22-24`).

**Proposed fix:** Reserve the name at policy/schema construction. Reject protected endpoint definitions with a path/query/header/cookie parameter of that name, and force a wrapped provider body when the JSON schema contains the same property so the top-level transport argument and nested provider field remain distinct. Test typed, generic, and `nyx__call_tool` forms.

### R2-N2. `ProxyExecutionResult.body` does not define streaming or WebSocket behavior

**Consequence:** A literal buffered-body refactor can silently remove streaming or force unbounded buffering on existing services.

**Classification:** **UNPROVEN.** Minting rules correctly reject streaming, but the shared result type is said to cover both executors generally, including allowlisted non-minting streaming and WS operations.

**Failure scenario:** REST executes a permitted SSE or WebSocket operation. A concrete `body: Bytes` result cannot represent the existing stream/upgrade, while buffering it changes latency and memory behavior.

**Evidence:** The plan gives `ProxyExecutionResult { status, headers, body, minted_token }` without a body type or variant contract (`docs/assistant/PR_B_PLAN.md:20-24`). Current REST node handling has separate complete and streaming response branches (`backend/src/handlers/proxy.rs:2465-2505`), and the policy test matrix includes WS and node entry shapes (`docs/assistant/PR_B_PLAN.md:131`).

**Proposed fix:** Define the shared type as an enum that preserves buffered, streaming, and upgrade responses, or narrow it explicitly to buffered mint-capable responses while leaving existing streaming transport intact. Only the buffered variant may mint; all variants still pass the pre-forward operation check.

### R2-N3. The canonical-path parity claim disagrees on ordinary percent encodings

**Consequence:** Tests written literally from the plan can demand incompatible behavior even though the security-critical separator and dot-segment cases are fixed.

**Classification:** **UNPROVEN, local.** REST permits decoding some ordinary percent escapes while MCP rejects every percent character.

**Failure scenario:** REST receives `/air/orders/%6Frd_X`, decodes it once to `ord_X`, and can safely match it; MCP receives the literal same spelling and rejects it because any `%` is forbidden. A test requiring the same allow/deny result for every spelling cannot pass.

**Evidence:** REST rejects selected encoded separators/dots and otherwise decodes once (`docs/assistant/PR_B_PLAN.md:109-113`); MCP rejects any `%` (`:112`); the next line requires parity for every adversarial spelling (`:114`).

**Proposed fix:** Define parity over equivalent semantic paths after each entry point's documented input interpretation, or reject all percent escapes in REST as well. Keep the byte-identical forwarded-target assertion for requests accepted by both.

### R2-N4. The JWT contract lists `kid` as both a claim and a header convention

**Consequence:** Two copies of the key identifier can disagree and create ambiguous verifier behavior.

**Classification:** **UNPROVEN, local.** The existing house convention places `kid` in the JWT header.

**Failure scenario:** A malformed token has header `kid = A` and claim `kid = B`; one verifier selects A while telemetry or later code trusts B.

**Evidence:** Existing `JwtKeys.kid` is documented for JWT headers (`backend/src/crypto/jwt.rs:18-23`), while the revised claim list includes `kid` inside the claims object (`docs/assistant/PR_B_PLAN.md:118`) and rotation selects by the token's `kid` (`:122`).

**Proposed fix:** Keep `kid` only in the protected JWT header, select the decoding key from that header, and omit it from resource-token claims. Test unknown, missing, and mismatched-header cases.

## Claims I verified and confirmed correct in Round 2

- **B1's ID failure mode is now fail-closed in the stated interface.** The shared check receives the final body, requires exactly one path-or-body extractor, denies extraction failure, and compares the extracted ID exactly (`docs/assistant/PR_B_PLAN.md:71-75`, `:93-103`). The remaining blocker is the absent expected resource kind.
- **B3's raw-path ambiguity is structurally addressed.** REST now starts from `OriginalUri`, MCP starts from a literal tool argument, and both produce a canonical value that is used for matching and deterministic forwarding (`docs/assistant/PR_B_PLAN.md:107-114`).
- **B4 is closed by removal.** Revision 2 adds no public `EffectiveActor` constructor and uses the server-chosen credential path only for the future public-row exchange (`docs/assistant/PR_B_PLAN.md:30-32`). The PR-A seal can remain unchanged.
- **Personal agent keys and direct member sessions can use the two-identity model.** For a personal key, `sub` remains a person and an organization-owned service can set `owner` to the organization; membership revalidation prevents that person from exchanging after removal (`docs/assistant/PR_B_PLAN.md:26-28`, `:127`). The unresolved case is an organization-owned key, not this personal-key flow.
- **The existing `x-nyxid-*` header parameter block can remain intact.** It already rejects those header arguments (`backend/src/services/mcp_service.rs:2399-2403`); a separately reserved, consumed MCP argument is the correct channel once preparation occurs before approval.
- **Node responses retain status, headers, and body.** The executor can inspect complete node responses without changing the node wire protocol (`backend/src/services/node_ws_manager.rs:42-47`).
- **The rotation arithmetic and key-selection model are now coherent.** A 96-hour maximum lifetime fits inside a 14-day previous-key overlap, and selecting the current/previous decoding key by header `kid` is implementable with the existing RSA machinery (`docs/assistant/PR_B_PLAN.md:118-122`).
- **Session-only exchange is specified correctly.** Revision 2 explicitly requires `AuthMethod::Session`, avoiding the existing broader human-only router's OAuth access-token gap (`docs/assistant/PR_B_PLAN.md:126-127`).
- **The small path-template grammar removes the general-glob ambiguity.** Literal segments plus single-segment captures, root anchoring, and rejection of recursive/alternate/class syntax are precise enough to validate (`docs/assistant/PR_B_PLAN.md:60-75`).

## Claims I could not verify in Round 2

- **MCP client compatibility with multiple content blocks:** Verification would require the exact Aevatar/Ornn MCP client parser and representative external MCP clients. The repository only proves NyxID currently emits one text block.
- **Production resource-kind registration and recovery sequencing:** No registry or PR-C/D code exists yet, so the claimed ordering cannot be tested until the plan makes it an enforceable dependency.
- **Duffel reissue evidence:** The provider read path and owner-evidence extractor remain intentionally deferred. Verification still needs the PR-C/D fixture and implementation.
- **Organization-owned key product intent:** The repository proves those keys exist and authenticate as the organization, but the plan does not decide whether travel minting through them should be rejected or sponsored by a separately authenticated person.
- **Streaming mint detection before response commitment:** The plan says runtime streaming fails closed, but the concrete shared-result/body abstraction is undefined. Verification requires the revised response-type contract.
