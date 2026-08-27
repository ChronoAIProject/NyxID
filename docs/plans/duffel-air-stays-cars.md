# Duffel Air, Stays, and Cars Platform Integration

Status: reviewed and revised after adversarial review

Repository: `ChronoAIProject/NyxID`

Base: `origin/main` at `ed372d8c` or newer

Existing enforcement foundation: PR #1448, merged as `578bb8dc`

Catalog precedent: PR #1454, merged as `b0c7b880`

## Objective

Publish one public, platform-credentialed Duffel catalog service that lets any
authenticated NyxID caller search, quote, book, pay where the product exposes a
separate payment operation, and cancel across Duffel Air, Stays, and Cars. The
shared Duffel credential must never permit booking/order list or read
operations. NyxID stores no booking data and no order-to-user mapping.

This is a multi-release implementation. Endpoint upload is not the first
production action. The credential is the final go-live switch and must not be
provisioned until the catalog row, fixed version header, operation policy,
runtime request-body validation, endpoint publication, idempotency behavior,
payment-token handoff, audit behavior, and smoke tests are deployed together.

## Settled Product Decisions

- One catalog service, slug `duffel`, base URL `https://api.duffel.com`.
- `visibility: public`, meaning available to authenticated NyxID callers. This
  does not enable anonymous/public-proxy execution.
- `service_category: internal`, `requires_user_credential: false`, and no
  `ProviderConfig` association.
- One shared platform credential, encrypted at runtime and never committed.
- The initial seeded row is inactive and has literal empty credential
  ciphertext. It must never contain ciphertext produced by encrypting an empty
  plaintext because that ciphertext is non-empty and passes the current master
  credential presence check.
- No resource tokens, no general fail-closed change for unrelated catalog rows,
  and no booking ownership database.
- A row without a policy preserves legacy behavior; the Duffel row always has
  an explicit policy and is deny-by-default.
- Air order reads, Stays booking reads, and Cars booking reads are not exposed.
- Human confirmation belongs to the product workflow unless a separate
  service-owned mandatory-approval feature is deliberately built. OpenAPI
  `x-aevatar-tool.requiresApproval` is metadata, not a runtime guarantee.
- Cancellation remains available wherever Duffel exposes it. Shipping writes
  without the remediation operation is not acceptable.

## Curated V1 Operation Set

The exact method/path set below is both the discovery contract and the runtime
authorization target. The policy remains separately declared from the overlay;
a test compares the two sets. Do not generate authorization automatically from
OpenAPI, because adding a tool must never silently expand credential authority.

### Air

```text
POST /air/offer_requests
GET  /air/offers
GET  /air/offers/{id}
POST /air/orders
POST /air/payments
POST /air/order_cancellations
POST /air/order_cancellations/{id}/actions/confirm
```

Explicitly absent:

```text
GET /air/orders
GET /air/orders/{id}
```

### Stays

```text
POST /stays/search
POST /stays/search_results/{id}/actions/fetch_all_rates
POST /stays/accommodation/suggestions
GET  /stays/accommodation
GET  /stays/accommodation/{id}
GET  /stays/accommodation/{id}/reviews
POST /stays/quotes
GET  /stays/quotes/{id}
POST /stays/bookings
POST /stays/bookings/{id}/actions/cancel
```

Explicitly absent:

```text
GET /stays/bookings
GET /stays/bookings/{id}
```

### Cars

```text
POST /cars/search
POST /cars/quotes
POST /cars/bookings
POST /cars/bookings/{id}/actions/cancel
```

Explicitly absent:

```text
GET /cars/bookings/{id}
```

The Stays and Cars method/path contracts must be rechecked against the pinned
official Duffel SDK/spec revision during implementation. The current official
JavaScript SDK exposes these resources, but SDK presence does not prove that
the production Duffel account is entitled to use them.

## Invariants

Every code PR and deployment step must preserve these invariants:

1. A missing Duffel credential makes the service inert and must not fall back
   to a user credential, provider credential, node credential, or anonymous
   route.
2. `Duffel-Version: v2` reaches every direct/node HTTP request and cannot be
   replaced by caller or `UserService` headers.
3. The effective operation policy is present before a credential can be set.
4. Endpoint publication never grants authority; runtime authorization remains
   the control.
5. Published Duffel MCP operations are a subset of the policy, and the checked
   v1 contract requires exact equality.
6. No booking/order list or read operation is published or authorized.
7. A policy denial happens before approval persistence, billing, node
   transport, credential decryption, or downstream forwarding.
8. Policy denials generate a metadata-only audit event and never contain
   credentials, passenger/guest/driver data, request bodies, or responses.
9. A release older than `578bb8dc` must never run in an environment containing
   the Duffel credential.
10. No card number, CVC, or raw payment credential passes through MCP, the
    generic proxy, audit, tracing, or NyxID persistence. Stays/Cars may receive
    only provider-issued opaque payment artifacts after the payment rail is
    separately verified.
11. Every allowed write body is validated at runtime against the pinned,
    operation-specific Duffel wire schema before credential decryption,
    approval persistence, billing, node transport, or forwarding. Validation
    is not limited to MCP-generated calls; raw authenticated REST proxy calls
    pass through the same check.
12. Every retriable write has a stable idempotency key generated or supplied by
    the calling workflow and preserved unchanged through direct and node
    forwarding. A timeout must not silently create a second order, payment,
    booking, or cancellation.
13. Duffel Cards is a separate security boundary at
    `https://api.duffel.cards`; `POST /payments/cards` is not part of the
    `duffel` service, overlay, or operation policy.

## Dependency Graph

```text
Step 1A: platform-credential/header/audit/publication/body-validation substrate
    | parallel
Step 1B: pinned Duffel contracts, entitlement, payment tokenization, idempotency
    +----------------------------+
                                 v
Step 2: verified overlay + inactive, literal-credential-empty Duffel seed
    -> Step 3: staging activation and full proof
        -> Step 4: production activation and monitoring
```

Steps 1A and 1B may proceed in parallel. Step 2 cannot be authored from guessed
SDK convenience arguments: it consumes the pinned paths, exact wire envelopes,
schemas, entitlement decisions, and idempotency rules produced by Step 1B.

## Branch and PR Execution

The planning worktree is on `travel-allowlist` at `15b215ed`, which is behind
and already contained by `origin/main` at plan time (`ed372d8c`). Do not create
implementation branches from this worktree branch. Fetch and branch from the
then-current `origin/main` for every code/evidence PR.

| Work | Branch | Base and dependency | Deliverable |
|---|---|---|---|
| Step 1A | `feat/catalog-platform-credentials` | Current `origin/main`; parallel with 1B | Substrate code, CLI, migration/reconciliation, and tests |
| Step 1B | `research/duffel-contract-fixtures` | Current `origin/main`; parallel with 1A | Pinned vendor evidence, redacted fixtures, entitlement/payment/idempotency decisions |
| Step 2 | `feat/catalog-duffel-air-stays-cars` | Fresh `origin/main` after 1A and 1B are merged | Inactive seed, verified overlay, runtime schema bindings, parity and mock tests |
| Step 3 | No source branch | Exact staging image built after Step 2 | Signed staging evidence and tested rollback |
| Step 4 | No source branch | Exact staging-approved image digest | Production activation record and monitoring evidence |

Cold-start commands for Step 1A or 1B:

```bash
git fetch origin
git status --short --branch
git switch -c <step-branch> origin/main
git rev-parse HEAD
gh auth status
```

Cold-start commands for Step 2, after verifying both prerequisite PRs are
merged:

```bash
git fetch origin
git log --oneline --decorate -20 origin/main
git merge-base --is-ancestor <step-1a-merge-sha> origin/main
git merge-base --is-ancestor <step-1b-merge-sha> origin/main
git switch -c feat/catalog-duffel-air-stays-cars origin/main
git rev-parse HEAD
```

Each code/evidence PR must include its verification output and must merge before
the dependent operator advances. Do not stack Step 2 on an unmerged Step 1
branch because that makes review and rollback boundaries ambiguous.

## Step 1A - Harden the Platform-Credential Service Substrate

Suggested PR: `feat/catalog-platform-credentials`

### Context Brief

`DEFAULT_SERVICE_SEEDS` in `backend/src/services/provider_service.rs` is built
for user-connected provider services. It always attaches a provider and writes
`proxy_operation_policy: None`; copying it would make the master-credential
branch unreachable. The generic admin service update request cannot set or
clear a non-OIDC catalog credential. The existing `service rotate-credential`
CLI command updates a user's `UserService`, not a platform catalog row.

Default header merge order is caller -> catalog -> user service. Because later
non-overridable entries replace earlier entries, a user-level default can
currently replace a catalog-level `Duffel-Version`. That contradicts the
catalog authority required here.

The existing seed precedent encrypts `b""`, which creates non-empty ciphertext,
while master-credential resolution currently treats any non-empty ciphertext
as present. A platform seed therefore needs a distinct literal missing-secret
representation. REST execution also resolves the target before its operation
policy check, so the substrate must split non-secret service/policy resolution
from credential resolution if denial is to precede decryption in reality.

### Tasks

- Introduce a provider-less system-service seed shape, separate from
  `DefaultServiceSeed`. A conservative name is `PlatformServiceSeed`.
- Support explicit seed fields for visibility, category, auth method/key,
  capabilities, hosted spec slug, default headers, and
  `ProxyOperationPolicy`. Seed missing credentials as literal `Vec::new()`
  ciphertext only. Never call `encrypt(b"")` for the missing-secret sentinel.
- Add a regression test that stores both literal empty ciphertext and encrypted
  empty plaintext. The former must be treated as missing; the latter must not
  be accepted as a valid provisioned credential and should fail closed as an
  invalid credential state.
- Require provider-less platform seeds to have a policy. `Some([])` remains a
  valid intentional kill policy; `None` is invalid for this seed class.
- Keep the seed idempotent and `created_by: "system"` so hosted overlay sync can
  find it. Never overwrite `credential_encrypted` during seed reconciliation.
- Preserve `is_active: false` during initial Duffel seeding and reconciliation;
  neither startup nor overlay sync may reactivate a disabled platform row.
- Define security-owned reconciliation rules. The Duffel policy, base URL,
  auth shape, and non-overridable version header should be code-authoritative;
  use a seed version or exact guarded migration rather than an untracked broad
  overwrite. Record every material reconciliation in logs/audit metadata.
- Add admin-only catalog credential subresources, distinct from user
  connections:

  ```text
  PUT    /api/v1/services/{service_id}/credential
  DELETE /api/v1/services/{service_id}/credential
  ```

- `PUT` accepts the secret in the request body, encrypts it, never returns it,
  and rejects rows that are not provider-less platform/master-credential
  services or that lack an explicit operation policy.
- `DELETE` clears the encrypted credential and is the operational kill switch.
  Both operations append metadata-only audit events.
- Add CLI commands under the existing `nyxid admin` namespace. Require env/file
  or interactive secret input; discourage and hide raw command-line values so
  secrets do not enter shell history. Target UX:

  ```text
  nyxid admin service credential set duffel --credential-env DUFFEL_API_TOKEN
  nyxid admin service credential clear duffel
  nyxid admin service enable duffel
  nyxid admin service disable duffel
  ```

  `enable` must reject a policy-bearing platform row with literal empty
  credential ciphertext, an invalid encrypted-empty credential, no operation
  policy, or unresolved locked headers/body schemas. `disable` is the first
  action in the emergency rollback path.

- Correct default-header semantics across direct HTTP, node HTTP, direct WS,
  and node WS: a catalog header with `overridable: false` is locked against
  both caller and user-service layers. A catalog header with `overridable:
  true` may be replaced. Reject locked-name collisions on user-service writes
  as defense in depth, while retaining runtime enforcement for legacy rows.
- Split policy-bearing service resolution into a metadata/policy phase and a
  secret-resolution phase. Canonicalize and authorize the operation after the
  target service identity is known but before decrypting any catalog, user,
  provider, agent-bound, or node credential. Keep the existing final
  resolved-target authorization check as defense in depth against target drift.
- Add an operation-specific runtime JSON body-schema hook for policy-bearing
  platform services. Validate the exact downstream wire body, with
  `additionalProperties: false` at relevant object boundaries, on REST, MCP,
  delegated-app, public authenticated, and node-routed entry paths before any
  side effect. Productionize or extract the existing JSON-schema subset
  validator in `backend/src/services/assistant_direct_agent_poc/tools.rs`
  rather than maintaining a second ad hoc validator.
- Reject unsupported content types and bodies for schema-bound operations.
  Explicitly reject raw payment field names such as PAN/card number, CVC/CVV,
  expiry, and magnetic-stripe data even if a future schema is accidentally
  loosened. Only documented opaque identifiers such as `card_id` or a verified
  3DS session identifier may cross the generic proxy boundary.
- Define a write-idempotency contract shared by REST, MCP, delegated, direct,
  and node-routed calls. Preserve the caller/workflow's stable idempotency
  header byte-for-byte on retries; do not generate a new value after an
  ambiguous timeout. Reject write operations that require idempotency when the
  workflow cannot supply or recover a stable key.
- Make policy denials auditable on REST and MCP paths. Use the existing
  `proxy_request_denied` convention with reason `operation_not_allowlisted`,
  method, canonical path, service id/slug, and actor/API-key attribution. Do
  not move auditing after credential resolution.
- Prevent misleading MCP publication for policy-bearing services: apply policy
  filtering after the complete operation catalog has been assembled, not only
  while reading persisted `ServiceEndpoint` rows. Cover seeded endpoints,
  admin-added rows, instance-mounted OpenAPI overrides, template fallback rows,
  and both platform and catalog-backed `UserService` publication. Preserve
  passthrough behavior for services with no policy. This also hides stale
  additive-sync endpoint rows after a policy removes an operation.
- Guard master-credential `base_url` changes. At minimum require platform admin
  authority and emit a dedicated audit event with old/new origins; preferably
  require clearing the credential before changing origins so a stored secret
  cannot be redirected to an attacker-controlled host.
- Add a startup warning for a seeded public platform service that has a policy
  but no credential. This is a safe state, not a startup failure.
- Update stale planning documentation that claims general fail-closed behavior
  for policy-less rows.

### Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p nyxid proxy_authorization
cargo test -p nyxid default_request_header
cargo test -p nyxid provider_service
cargo test -p nyxid mcp_service
cargo test -p nyxid-cli admin
```

Add focused integration tests proving:

- Provider-less seed rows reach the master-credential path.
- Missing credential returns a not-found-shaped denial without decryption,
  billing, node transport, or forwarding.
- Literal empty ciphertext and encrypted empty plaintext cannot be confused;
  neither produces an executable platform credential.
- Credential set/replace/clear is admin-only, audited, redacted, and preserves
  the service UUID and policy.
- Catalog locked headers beat user locked headers on all transports.
- Policy denials write one denial audit and no success audit.
- MCP does not publish an endpoint excluded by a present policy.
- Every MCP operation source is filtered after assembly, including an
  instance-mounted OpenAPI override that attempts to add a forbidden read.
- Raw REST and MCP requests with extra/unknown fields or raw card fields are
  rejected before credential decryption and forwarding.
- Direct and node-routed retries preserve one stable idempotency key; a mocked
  upstream timeout followed by retry produces one logical write.
- Services without policies retain existing publication and execution behavior.

### Exit Criteria

- The substrate can represent an inert provider-less, public, policy-bearing
  service without a credential.
- Operators can set, rotate, and clear its credential without recreating it.
- A catalog non-overridable header is genuinely authoritative.
- Denied and stale operations are neither forwarded nor advertised.

### Rollback

This PR contains no Duffel row and no production credential. Rollback is an
ordinary binary rollback. Do not use the new credential endpoint until Step 2
is deployed.

## Step 1B - Pin Duffel Contracts, Entitlements, Payments, and Idempotency

Suggested evidence branch: `research/duffel-contract-fixtures`

This work may run in parallel with Step 1A, but its reviewed outputs are hard
inputs to Step 2. It must finish before anyone authors or uploads the overlay.

### Context Brief

The official `@duffel/api` SDK is a convenience layer, not the downstream wire
contract. In the inspected 4.28.0 source, write payloads are wrapped as
`{"data": ...}` by `src/Client.ts`, so copying SDK method arguments into an
OpenAPI request body would publish the wrong API. SDK resource presence also
does not prove that the production Duffel account is entitled to Air, Stays,
or Cars.

Duffel Cards uses the separate `https://api.duffel.cards` origin and accepts
card data at `POST /payments/cards`. It is explicitly outside the generic
NyxID proxy. A separate browser/payment-owned component must exchange raw card
data directly with the tokenization provider and return only an opaque artifact
to the application workflow.

### Tasks

- Pin an exact official Duffel SDK tag or commit and, where available, an exact
  official OpenAPI/spec revision. Record source URLs, content hashes, retrieval
  date, and the Duffel API version header in a repository evidence document.
- Verify every proposed Air, Stays, and Cars method/path against the pinned
  source. In particular, verify that accommodation suggestions is
  `POST /stays/accommodation/suggestions`, not GET.
- Capture redacted request/response fixtures from Duffel test mode for every
  proposed operation. Fixtures must show the exact HTTP method, path, query,
  headers, content type, `{"data": ...}` request envelope, status, and response
  envelope. Remove passenger, guest, driver, credential, payment, and live
  booking data before committing.
- Derive strict request schemas from the verified wire fixtures, not SDK-level
  arguments. Use `additionalProperties: false` wherever Duffel's contract is
  closed and make every write envelope require the top-level `data` member.
- Verify the platform account's Air, Stays, and Cars entitlements in test mode
  and obtain an explicit production entitlement statement. Mark beta,
  region-limited, or access-gated operations as excluded until proven.
- Produce a payer/payment-liability matrix for each product and rate type:
  Duffel Balance, postpaid, opaque `card_id`, and any 3DS flow. Do not infer
  that Air `/air/payments` is the payment path for Stays or Cars.
- Define the separate payment-tokenization workstream: owning repository/team,
  browser or hosted component, provider endpoint, CSP/origin controls, API
  handoff, lifecycle/expiry of opaque artifacts, error recovery, and tests.
  The handoff into NyxID may contain only an opaque `card_id` or verified 3DS
  session identifier. `api.duffel.cards` is never added to the `duffel` row.
- Verify Duffel's official idempotency mechanism for Air orders/payments,
  Stays bookings, Cars bookings, and cancellation actions. Record header name,
  value constraints, retention window, response behavior, and which operations
  support it. For any write without provider idempotency, define a product-level
  no-automatic-retry rule and ambiguity recovery behavior.
- Define application response retention and user messaging. Since booking/order
  reads are blocked and NyxID stores no booking data, the application must
  retain or deliver the create response required by the user; NyxID cannot
  later reconstruct "my trip" from Duffel.

### Verification

- A reviewer can reproduce each method/path/schema from the pinned vendor
  revision without relying on prose or SDK names.
- Contract fixtures prove the exact `{"data": ...}` wire envelope for every
  POST operation.
- Test-account flows cover search -> offer/quote -> booking -> payment where
  applicable -> cancellation for every product proposed for activation.
- The tokenization integration has an independently reviewed threat model and
  tests proving PAN/CVC never reaches NyxID, MCP, the generic proxy, logs, or
  traces.
- Timeout/retry tests prove one stable idempotency key is reused and document
  the behavior for writes that Duffel cannot make idempotent.

### Exit Criteria

- The exact v1 method/path/body/response set is pinned and reviewed.
- Every enabled product is entitled, has an understood payer-of-record, and has
  a verified cancellation path.
- Payment tokenization has a named owner and tested opaque-artifact handoff.
- Every write has either verified provider idempotency or an explicit no-retry
  workflow that handles ambiguous outcomes.

### Rollback

This is evidence and test-mode work only. Remove unverified operations from the
proposed v1 set; do not weaken schemas or add speculative endpoints to keep a
product family nominally present.

## Step 2 - Add the Inert Duffel Catalog Integration

Suggested PR: `feat/catalog-duffel-air-stays-cars`

### Context Brief

PR #1454 established the repository-backed overlay pattern for Twilio and
ElevenLabs: curated OpenAPI under `backend/specs/catalog`, registry mapping,
startup sync to `ServiceEndpoint`, transport-specific tests, and drift checks.
Duffel copies that pipeline but not their provider row, connection UX, or
per-user credential model. The overlay in this step is a transcription of the
reviewed Step 1B wire contract; it is not the place to discover or guess Duffel
operations.

### Tasks

- Add `backend/specs/catalog/duffel.openapi.json` as an OpenAPI 3.1 overlay
  containing exactly the v1 operation set above.
- Give every operation a stable `operationId`, accurate path/query/body schema,
  response content types, and `x-aevatar-tool` metadata. Mark booking, payment,
  and cancellation operations destructive as appropriate. Do not claim that
  `requiresApproval` enforces runtime approval.
- Describe the actual Duffel HTTP wire shape. Every POST schema must require the
  top-level `data` envelope verified in Step 1B and must not substitute the
  SDK's unwrapped convenience arguments.
- Attach each allowed write operation to the runtime schema-validation hook
  introduced in Step 1A. The overlay schema and runtime schema may share a
  reviewed source artifact, but the independently declared method/path policy
  remains the authority boundary.
- Group operations with `Air`, `Stays`, and `Cars` tags so CLI/MCP discovery is
  legible while retaining one credential/service row.
- Register the overlay in `HOSTED_SPEC_SOURCES` and map `duffel` in
  `SLUG_TO_SPEC_KEY`.
- Add a `PlatformServiceSeed` for `duffel` with:

  ```text
  name: Duffel
  slug: duffel
  base_url: https://api.duffel.com
  service_type: http
  visibility: public
  service_category: internal
  is_active: false
  requires_user_credential: false
  provider_config_id: null
  auth_method: bearer
  auth_key_name: Authorization
  credential_encrypted: literal Vec::new(), never encrypt(b"")
  Duffel-Version: v2, overridable=false
  proxy_operation_policy: explicit v1 set
  ```

- Define the overlay operation set and policy operation set independently, then
  add a parity test that fails with a human-readable set diff if they differ.
- Add explicit negative assertions for every forbidden booking/order read and
  representative undeclared paths from each product family.
- Assert that no server URL, operation, or credential path references
  `api.duffel.cards` or `POST /payments/cards`.
- Add overlay registry/schema tests, endpoint materialization tests, MCP tool
  publication tests, and idempotent second-startup tests.
- Add a drift-guard entry against the best authoritative Duffel source
  available. A network/fetch failure in the drift job must fail or clearly mark
  the check inconclusive; it must not silently pass.
- Document the public authenticated exposure, missing order/history behavior,
  credential activation process, rollback floor, and the fact that NyxID has
  no "my trips" data.

### Verification

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p nyxid catalog_spec_registry
cargo test -p nyxid catalog_spec_sync
cargo test -p nyxid provider_service
cargo test -p nyxid proxy_authorization
python3 scripts/check-catalog-spec-drift.py
```

Also run an integration test against a local mock Duffel server for every
allowed operation. Assert the exact method, path, query, content type,
serialized `{"data": ...}` body, stable idempotency header where applicable,
injected `Authorization` header, and `Duffel-Version: v2`. Assert forbidden
reads and schema-invalid bodies never reach the mock. Cover direct and node
HTTP routing; WebSocket is not a Duffel v1 transport requirement.

### Exit Criteria

- Fresh and existing databases converge on one system-owned Duffel row and the
  exact typed endpoint set.
- The row is visible in `catalog --all` but inactive and inert because its
  credential ciphertext is literally empty.
- `nyxid catalog show duffel`, `nyxid catalog endpoints duffel`, and
  admin inspection show the curated operations; executable MCP tools do not
  appear until the service is explicitly activated through the reviewed
  operational path.
- Every execution attempt returns a safe inactive/missing-service denial until
  activation, and startup never converts the row into an active state.

### Rollback

Because no credential has been installed, rollback is safe. The seeded row may
remain in MongoDB but is inert. If necessary, deploy a kill policy `Some([])`
before removing catalog code.

## Step 3 - Activate and Prove in Staging

Operational change, not a source-code PR.

### Context Brief

The seeded service is intentionally inactive and credential-empty. Credential
installation and activation are two separate audited actions. The running
image must contain PR #1448 and the reviewed outputs of Steps 1A, 1B, and 2.

### Tasks

- Record the deployed image digest and source commit. Reject any image older
  than merge `578bb8dc`.
- Confirm the seeded row is inactive, has literal empty credential ciphertext,
  and has the expected policy, runtime body-schema bindings, and locked version
  header before setting the credential.
- Set the Duffel test credential with the new admin CLI using an environment
  variable or secure interactive input.
- While the row remains inactive, prove that no execution path or executable
  MCP tool becomes available merely because the credential was installed.
- Explicitly enable the row with the audited admin CLI command.
- Positive-test every published operation through REST, MCP, delegated app,
  public authenticated proxy, and node-routed HTTP where supported.
- For every POST, assert the exact `{"data": ...}` body and that retries retain
  the same verified Duffel idempotency key.
- Negative-test all forbidden reads, trailing/duplicate slash variants,
  encoded separators, wrong case, method mismatches, query smuggling, and
  method-override headers.
- Confirm denials produce one metadata-only audit event and no billing,
  approval persistence, node request, credential decrypt, or upstream request.
- Send schema-invalid bodies, unknown extra properties, and representative raw
  card fields through both REST and MCP; confirm rejection before decryption,
  billing, node transport, or upstream.
- Confirm catalog, MCP config, and CLI endpoint listings agree.
- Exercise the kill switch in operational order: disable the row, clear the
  credential, verify literal empty ciphertext and inert execution, reinstall
  the test credential while inactive, then explicitly re-enable it.

### Exit Criteria

- Staging evidence covers every direct and node-routed execution surface in
  use by the product.
- No forbidden operation reaches Duffel.
- Disable, credential clear/restore, and explicit re-enable work without
  changing the service UUID.
- Operations and audit dashboards make missing credential versus policy denial
  distinguishable without leaking secrets.

### Rollback

Disable the row, clear the credential, verify it is inert, then roll back.
Never roll the image below the allowlist enforcement floor while a credential
exists.

## Step 4 - Production Activation and Monitoring

Operational change with a reviewed runbook.

### Tasks

- Deploy the exact staging-approved image digest.
- Verify policy, overlay parity, locked header, public authenticated exposure,
  and absence of anonymous endpoint rules.
- Provision the production credential through the admin CLI. Do not use curl
  with a literal secret, shell history, source control, CI variables printed in
  logs, or MongoDB writes.
- Verify the row remains inactive after provisioning, then explicitly enable it
  only after the preflight assertions pass.
- Run low-value/product-approved smoke flows for Air, Stays, and Cars. If a
  product is not entitled or its payment rail is not verified, remove its
  operations before activation rather than accepting predictable runtime
  failures.
- Monitor policy-denial audit counts, upstream 4xx/5xx, booking/payment errors,
  billing side effects, and credential-health signals.
- Anchor the rollback runbook: disable -> clear credential -> verify literal
  empty ciphertext and inert execution -> roll back.
- Schedule credential rotation through the same audited endpoint and verify the
  old credential no longer works.

### Exit Criteria

- Production exposes only the reviewed operation set.
- The first real flows confirm user-funded booking/payment behavior and
  cancellation for each enabled product.
- On-call has tested credential clear, rotate, and rollback procedures.

## Parallel Work

The following Step 1A and Step 1B tasks can proceed in parallel:

- Backend provider-less seed and credential subresource.
- CLI admin credential commands.
- Header precedence, pre-decryption policy ordering, post-assembly MCP
  filtering, runtime body validation, and idempotency forwarding.
- Pinned vendor-contract/fixture research.
- Duffel test-account entitlement, payer-liability, payment tokenization, and
  idempotency research.

Overlay/schema authoring starts only after Step 1B freezes its reviewed wire
contract. Step 1A and Step 1B must converge before Step 2 merges because the
parity and integration tests depend on both outputs.

## Anti-Patterns to Reject

- Importing Duffel's full upstream API specification.
- Treating OpenAPI publication as authorization.
- Generating the allowlist from the overlay automatically.
- Copying Twilio/ElevenLabs as a provider-backed connection service.
- Committing or seeding the Duffel credential.
- Creating three service rows that duplicate the same secret unless product-
  level service scoping becomes a requirement.
- Enabling anonymous proxy rules for Duffel.
- Claiming `requiresApproval` guarantees runtime approval.
- Allowing booking/order list or read operations because IDs appear hard to
  guess.
- Sending raw payment-card fields through MCP or the generic proxy.
- Adding `api.duffel.cards` or `/payments/cards` to the generic Duffel row.
- Assuming method/path allowlisting makes arbitrary bodies safe.
- Authoring the overlay from SDK method arguments before pinning the wire
  contract and exact `data` envelope.
- Automatically retrying a booking/payment/cancellation with a new idempotency
  key after an ambiguous timeout.
- Installing the credential before policy/header/publication validation.
- Rolling back below `578bb8dc` while the credential remains installed.
- Deleting and recreating the service to rotate its credential.
- Letting an admin base-URL edit redirect a live platform credential without an
  explicit clear-and-reprovision sequence.

## Plan Mutation Protocol

- Add an operation only by changing the overlay, explicit policy, parity test,
  negative test inventory, product contract fixtures, and rollout checklist in
  the same reviewed change.
- Remove an operation from policy first or atomically with publication; never
  leave it authorized but undiscoverable by accident.
- If Duffel changes a method/path, land the new path deny-by-default, verify it
  in test mode, then replace the old operation in overlay and policy together.
- If separate product scoping becomes necessary, stop and design credential
  references/rotation ownership before splitting the service row. Do not copy
  the ciphertext into three independently managed rows.
- Any request to enable booking reads changes a settled security boundary and
  requires a new owner decision; it is not a routine overlay expansion.

## Adversarial Review Response

The first review verdict was `REWORK`. This revision resolves each blocking
point as follows; implementation review must verify the code rather than accept
this table as evidence.

| Review finding | Resolution in this revision |
|---|---|
| Encrypted empty plaintext is not a missing-credential sentinel | Step 1A requires literal `Vec::new()` ciphertext, rejects encrypted-empty state, and adds a distinguishing regression test; Step 2 seeds inactive with literal empty ciphertext. |
| Overlay was scheduled before authoritative contract verification | Contract pinning, redacted wire fixtures, entitlement, and payment research moved to Step 1B; Step 2 is blocked on its reviewed output. |
| Method/path policy cannot prevent unsafe card fields | Step 1A adds strict runtime per-operation body validation on every execution path plus explicit raw-payment-field rejection; tokenization is outside the generic proxy. |
| POST wire envelope was not proven | Step 1B captures exact vendor fixtures, and Step 2 mock tests assert exact serialized `{"data": ...}` bodies, headers, methods, and paths. |
| MCP filtering covered too few operation sources | Step 1A filters after complete operation assembly and tests seeded, admin, instance-spec, fallback, platform, and catalog-backed sources. |
| Payment tokenization lacked an owner and prerequisite gate | Step 1B defines a separate owned workstream and independently reviewed opaque-artifact handoff; `api.duffel.cards` is excluded from the service. |
| Write retries had no idempotency contract | Steps 1A and 1B require verified Duffel idempotency rules, stable forwarded keys, retry tests, and explicit no-retry handling where unsupported. |
| REST denial could occur after credential resolution/decryption | Step 1A splits metadata/policy lookup from secret resolution and preserves a final resolved-target check as defense in depth. |

## Final Acceptance Matrix

| Area | Required evidence |
|---|---|
| Seed | One inactive provider-less Duffel row; literal empty ciphertext; never encrypted empty plaintext |
| Auth | Shared credential encrypted, rotatable, clearable, never returned |
| Header | `Duffel-Version: v2` wins on every transport |
| Discovery | Only curated Air/Stays/Cars operations are published |
| Policy | Exact parity with overlay; all other operations denied |
| Privacy | No Air order, Stays booking, or Cars booking reads |
| Ordering | Denial before approval, billing, node, decrypt, forwarding |
| Audit | Metadata-only denial and credential-management events |
| Bodies | Exact `data` envelopes; strict runtime schemas on REST and MCP |
| Payment | Separate tokenization; no Cards origin or raw card data in NyxID |
| Retry | Stable Duffel idempotency key or explicit no-retry behavior |
| Operations | Inactive/missing credential inert; disable/clear/rotate tested |
