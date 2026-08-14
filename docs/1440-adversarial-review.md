# Adversarial Review: #1440 Plan

## Verdict

The plan is not safe to implement as written. The dedicated route is the right
shape, and the six historical claims are mostly verified correctly, but the
proposed response digest is knowingly not a digest of the response, the stated
discovery-to-approval test cannot work with the proposed catalog-only token,
and the `/mcp` fix preserves a claim-less allow-all path without specifying the
live-grant behavior for delegated catalog tokens. Rework those contracts and
tests before implementation. After those changes, the route-first approach is
reasonable; widening `/mcp` is not required.

## Findings

### 1. BLOCKING - The proposed `catalog_digest` is not the digest of the caller-visible view

The plan explicitly chooses to hash the unprojected `catalog.services` while
returning only generic-free `UserManaged` services (plan lines 223-252 and
294-340). That is already a direct deviation from #1440's response contract,
but the mismatch is larger than the plan acknowledges. The canonical helper at
[`mcp_service.rs`](/Users/chronoai/Library/Application%20Support/heca/worktrees/3571ce16/nyxid-1440-impl/backend/src/services/mcp_service.rs:284)
hashes every service in the input, including platform services and generic
services, and includes `recommended_skills` at lines 313-325. Each endpoint
also contributes `name`, `description`, and `response_description` (lines
292-304), while the proposed DTO in plan lines 362-373 does not return those
description fields or `recommended_skills`. Thus even a catalog containing no
generic service can change its digest when an omitted platform row, skill, or
description changes.

Concrete failure: Aevatar fetches a catalog containing one typed UserService,
then an unrelated platform fallback or generic service is added, or an
operation description is edited. The response's `services` and operations are
unchanged, but `catalog_digest` changes. Aevatar retries an otherwise identical
exact approval and receives `catalog_drift`; conversely, a consumer cannot
recompute the advertised digest from the bytes it was given. The planned test
at lines 426 and 450-455 only compares the response to the same full internal
`catalog.services`, so it proves the deviation, not the stated contract.

There is a third option beyond accepting the contradiction or silently
changing one field: define an explicit admission projection (generic-free,
UserManaged-only, with exactly the DTO's contract fields), give that projection
a versioned canonical digest, and make approval create/redeem use that same
projection. Migrate the existing `/mcp/config` producer in the same compatibility
decision, or expose an explicitly versioned legacy digest for old in-flight
approvals. Do not call the full legacy catalog digest a caller-visible digest.

### 2. BLOCKING - The planned discovery-to-approval flow uses a token that cannot create the approval

The plan's happy-path fixture says the delegated token is minted with
`mcp:catalog:read` (plan line 426), and its drift proof says to use that facade
caller to create an exact-service approval (lines 449-455). The exact approval
handler calls `auth_user.ensure_rest_proxy_access()` before entering the service
at [`exact_service_approvals.rs`](/Users/chronoai/Library/Application%20Support/heca/worktrees/3571ce16/nyxid-1440-impl/backend/src/handlers/exact_service_approvals.rs:17)
and does the same for redeem at line 43. `mcp:catalog:read` deliberately does
not satisfy the proxy scope, as the existing assertion shows at
[`auth.rs`](/Users/chronoai/Library/Application%20Support/heca/worktrees/3571ce16/nyxid-1440-impl/backend/src/mw/auth.rs:1494).

Concrete failure: the planned integration test obtains a 403 before
`resolve_exact_catalog`; it cannot demonstrate digest compatibility or drift
revalidation. More importantly, a real Aevatar client following the documented
catalog-only authority cannot submit or redeem an approval. This is not fixed
by widening catalog-read: #1439 explicitly says that scope must not imply
effect authority.

The plan must specify the intended two-capability flow. Either the caller uses
a separate proxy/effect-authorized delegated token for approval operations, or
the token carries both independently granted `mcp:catalog:read` and proxy
scopes. The tests must exercise that exact flow and assert that catalog-only
tokens remain unable to create/redeem. If one token is intended, the plan needs
an admission-only approval scope and a separate security review; silently
assuming catalog-read is enough would violate the prerequisite contract.

### 3. SHOULD-FIX - The `/mcp` bounds patch keeps a future catalog-authority fail-open

Step 2 copies the middleware's `unwrap_or(true)` fallback verbatim (plan lines
280-287). The fallback is compatible with old ordinary MCP access tokens, but
the manual transport still does not call `validate_live_grant`, which the
normal `AuthUser` path invokes for every delegated token carrying
`mcp:catalog:read` at [`auth.rs`](/Users/chronoai/Library/Application%20Support/heca/worktrees/3571ce16/nyxid-1440-impl/backend/src/mw/auth.rs:763).
The transport's only pre-existing JWT gate is the proxy gate at
[`mcp_transport.rs`](/Users/chronoai/Library/Application%20Support/heca/worktrees/3571ce16/nyxid-1440-impl/backend/src/handlers/mcp_transport.rs:416), so a token containing both `proxy:*` and `mcp:catalog:read` can enter `/mcp`.

Concrete failure: during a rolling upgrade, or after a minting regression, a
delegated token has the catalog scope but missing restriction claims or no live
grant. The new code maps missing claims to `allow_all_* = true`, and the manual
transport never performs the grant check. `tools/list` and `tools/call` then
run with unrestricted service/node authority, despite #1439 requiring missing
catalog authority to fail closed. The current source may not mint that exact
malformed token, but preserving this behavior makes the security fix depend on
an unstated deployment invariant.

Retain legacy allow-all only when the token is not exercising catalog authority.
For a delegated token with `mcp:catalog:read`, require complete claims and a
shared live-grant validator (or explicitly reject that scope on `/mcp`). Add a
test for a catalog-scoped token with absent claims, revoked grant, disabled
client, and stale grant. Tightening all claim-less ordinary MCP tokens would be
breaking; conditional tightening preserves compatibility without importing the
fail-open into the new authority domain.

### 4. SHOULD-FIX - The plan overstates the case against extending `/mcp`

The route-first conclusion is defensible, but the plan calls all three reasons
against `/mcp` independently sufficient (plan lines 174-193), and that is too
strong. The dispatcher parses the JSON-RPC method before dispatch at
[`mcp_transport.rs`](/Users/chronoai/Library/Application%20Support/heca/worktrees/3571ce16/nyxid-1440-impl/backend/src/handlers/mcp_transport.rs:665), with separate `tools/list` and `tools/call` arms at lines 736-758. A method-level gate could admit a catalog-only token for
`initialize`/`tools/list`/`ping` while rejecting `tools/call`, generic proxy,
and mutating meta-tools. The live-grant check could be extracted into a shared
service helper rather than duplicated in two handlers.

That alternative is still more invasive and has MCP session semantics and
protocol-projection costs, so it does not overturn the dedicated-route choice.
It does mean the plan's argument is a tradeoff, not a proof that extension is
infeasible. The review should state the required method matrix and shared
validator if `/mcp` is considered, otherwise the cheaper existing-client path
has not received a real hearing.

### 5. SHOULD-FIX - "Pure read" is underspecified because the canonical loader can fetch user URLs

Step 3 says the facade performs only `find` reads and no provider invocation
(plan lines 376-382), but `load_operation_catalog` invokes the user-spec path
in [`mcp_service.rs`](/Users/chronoai/Library/Application%20Support/heca/worktrees/3571ce16/nyxid-1440-impl/backend/src/services/mcp_service.rs:974) and
[`mcp_service.rs`](/Users/chronoai/Library/Application%20Support/heca/worktrees/3571ce16/nyxid-1440-impl/backend/src/services/mcp_service.rs:1019). Those paths call
`fetch_spec_json_scoped` in [`api_docs_service.rs`](/Users/chronoai/Library/Application%20Support/heca/worktrees/3571ce16/nyxid-1440-impl/backend/src/services/api_docs_service.rs:113), which performs an outbound HTTP fetch (with a cache, DNS pinning, and size limits).

Concrete failure: a catalog request blocks on or times out against a
user-configured/private OpenAPI URL and causes an external network request,
despite the route's "pure read/no provider invocation" promise. The hardened
fetch path reduces SSRF risk but does not make the operation local or
side-effect-free. Decide explicitly whether documentation fetches are in scope;
if not, use persisted/template endpoint rows for this contract and make the
approval resolver use the same rule, or document and test the outbound fetch as
an allowed read dependency. The mutation-spy plan currently observes Mongo
collections only and would miss this behavior.

### 6. NOTE - Empty bounds and stale identifiers need an explicit semantic contract

Returning 200 with an empty catalog for a valid `allow_all_services=false` plus
empty list is a sound deny-all interpretation (plan lines 319-326): it does not
leak whether any service exists. The plan should nevertheless distinguish this
from unknown or deleted IDs in the contract. Its test only expects unknown IDs
to be rejected earlier by mint/live-grant validation (lines 431-437); it does
not test a grant whose previously valid service/node is deleted between token
validation and catalog resolution. Keep the response not-found-shaped or
empty, but state which behavior is intentional and ensure it never turns a
stale restricted grant into unrestricted scope.

## Verification of the six claims

I spot-checked all six claim classifications against this worktree. Claims 1-4
are supported: the manual `/mcp` authentication and proxy scope gate are at
`mcp_transport.rs:230,365,416,447-476`; the management deny family is
`mw/auth.rs:390-446` and is applied in the `AuthUser` branch at
`mw/auth.rs:763-787`; and the transport context still defaults to allow-all at
`mcp_transport.rs:262-275`. Claims 5 and 6 are also correctly refuted: the
approval resolver calls all three digest helpers at
`exact_service_approval_service.rs:482-528`, and `handlers/mcp.rs:125,193`
already uses the canonical helper. The cited `mcp.rs:212-214` text is indeed a
dangling comment, not a competing implementation.

The requested GitHub lookup for issue #1424 returns "Could not resolve to an
Issue"; commit `d1a042c2` in this worktree is the concrete #1424 implementation
and supports the plan's provenance claims. That repository metadata gap is not
a refutation, but release notes should link the commit/PR rather than an
unresolvable issue URL.

## Scope omissions and additions

The plan is honest that it omits the Aevatar canary and new numeric error codes,
and those omissions are consistent with #1440's delivery boundary. The
`mcp_transport.rs` bounds patch is outside the narrow route addition, but it is
justified by the live fail-open and should either be retained with the stricter
catalog-scope behavior above or split into a separately reviewed security fix.
The missing items are the token-scope flow for approval create/redeem, a
contract-level decision for the digest migration, and an outbound-spec-fetch
test/decision; those are not listed as assumptions and must be added before
implementation.
