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
