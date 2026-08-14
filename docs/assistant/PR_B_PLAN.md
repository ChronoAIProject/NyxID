# PR-B Plan — Operation Authorization + Resource Tokens

**Scope:** `TRAVEL_BOOKING.md` §A.2, on `travel-allowlist` off `main` (`0016d657` — PR-A's credential gate and log redaction are merged and live). This slice is **the blocker for creating a real Duffel row**: today the proxy forwards any path a caller supplies, so a Duffel row would expose `GET /air/orders` — every user's bookings on the shared platform credential — to any caller who can reach the row. Nothing travel-shaped ships in this PR; both mechanisms are generic platform primitives.

**Out of scope:** the Duffel row and overlay (PR-C/D), the `payment.complete` manifest entry, the payment card, any frontend. Sequence after this: Sol adversarial-reviews this plan → Sol implements → Opus reviews → design-author verification → PR.

---

# Part I — Narrative

## What this slice delivers

1. **The proxy learns to say no.** A service can declare which operations exist — method + path pattern — and the proxy enforces that list **in the data plane, at every entry point**. Declaring operations to MCP is discovery; this is authorization. A path not on the list never leaves NyxID, no matter how the request arrived.
2. **The proxy learns whose order is whose, without a database.** When a caller creates a resource through an allowlisted operation, NyxID stamps the response with a **signed resource token** — short-lived, bound to the effective account, stateless. Operations marked as protected (order reads, payment, cancellation, the card-form key exchange) require presenting it. Attribution lives in the signed artifact; NyxID still stores nothing.

## Why the enforcement lives where it does

NyxID has **two independent request executors**. The REST/WebSocket path resolves and forwards through `handlers/proxy.rs`; the MCP tool layer (`mcp_service::execute_tool`, `mcp_service.rs:3044`) resolves credentials via the same *resolvers* (`mcp_service.rs:3084, 3226, 3234`) but builds and forwards requests **on its own** — it never enters the REST executor. A check present in only one executor is not a control. So the primitive is one shared, side-effect-free function called from **both**, after the final method/path/body exist and before approval, billing, node transport, or forwarding. PR-A proved this census discipline works (every credential decrypt was routed through one gate); PR-B applies it one layer up.

Paths are compared in **one canonical form** — decoded exactly once, encoded separators and dot-segments and duplicate/trailing slashes rejected outright, query strings excluded — used for matching *and* forwarding, so a path that looks different to NyxID but identical to Duffel cannot slip past.

## What the live API changed (measured 2026-08-14, `fixtures/duffel-offer-request-sample.json`)

The transport already works: a real 84-offer search returned through `/api/v1/proxy/s/duffel/air/offer_requests` with **zero new proxy code**, and omitting `Duffel-Version: v2` returns HTTP 400 from Duffel — confirming the non-overridable default-header design. Three measured facts supersede assumptions:

- **78% of offers are holdable** (495 of 635 on a real SIN→NRT return). The v1 holdable-only restriction is mild, and is now recorded as measured.
- **Hold windows run 1–3 days, not ~24 h** (`payment_required_by` of Aug 15 and Aug 17 from an Aug 14 search). This breaks the single-wait assumption: Aevatar's `wait_signal` caps at 86,400,000 ms. The workflow's wait is therefore specified as a **deadline-driven re-entrant loop** (Appendix A.5): each leg waits `min(payment_required_by − now, 24 h)`, wakes on signal or timeout, re-reads truth, checkpoints, re-enters — at most 3–4 legs for measured windows — with `self_reschedule` as the fallback if looped waits prove awkward in the engine.
- **The price guarantee expires *inside* the hold** (`price_guarantee_expires_at` Aug 16 vs `payment_required_by` Aug 17 on the same offer). A re-price between those instants is a **live case, not an edge**: the seat is held but the amount can move. Consequences, now stated as behavior: the payment card must re-read the order's current totals when opened (the amount in the action params is display-only); a `price_changed` rejection on payment submit is a normal branch that refreshes the display for re-confirmation (3DS shows the true amount regardless); and the **resource token's lifetime is derived from `payment_required_by`**, not from a fixed day.

## The decision made now, not mid-implementation

PR-A sealed `EffectiveActor` — private field, private `from_user_id` (`proxy_service.rs:86-91`) — which means the planned `/api/v1/resource-tokens/*` handlers **cannot reach the credential gate from a new module**. Decision: **expose `pub fn EffectiveActor::for_auth_user(&AuthUser) -> EffectiveActor`, defined beside the gate in `proxy_service`, field stays private.** Justification: the alternative (hosting handler logic inside `proxy_service`) breaks the handlers→services layering (CLAUDE.md Rule 2) and grows the module that PR-A deliberately minimized; the narrow constructor preserves what the sealing actually bought — an actor can only be built from a real authenticated principal (`AuthUser` is produced only by the auth middleware), so a synthetic "system" actor still cannot be constructed anywhere. The constructor takes `&AuthUser`, not a string — the string-typed hole is what the sealing closed, and it stays closed.

## What is deliberately not built

No per-user Duffel identity, no order mapping, no token revocation store (expiry is the bound — stateless by owner decree), no allowlist UI (policy ships in seed/admin JSON like `anonymous_endpoints` does today). The token primitive carries `jti` so per-mint tracing is possible later without new state.

---

# Appendix A — Implementation contract

Citations verified on `travel-allowlist` at `65fb546c`; **ASSUMPTION** marks what is not.

## A.1 Policy model and the shared check

On `DownstreamService`, in the idiom of `AnonymousEndpointRule` (`models/downstream_service.rs:115-123`, stored list at `:334`):

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub proxy_operation_policy: Option<ProxyOperationPolicy>,   // None = passthrough (today's behavior)

pub struct ProxyOperationPolicy { pub rules: Vec<ProxyOperationRule> }
pub struct ProxyOperationRule {
    pub method: String,                       // uppercase at admin write
    pub path_pattern: String,                 // globset; compiled + validated at admin write
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mints_resource_token: Option<ResourceTokenMint>,   // { resource_kind: "duffel:order", id_path: "data.id", id_pattern: "^ord_[A-Za-z0-9]+$", expiry_path: Option<"data.payment_required_by"> }
    #[serde(default)]
    pub requires_resource_token: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_token_id_path: Option<String>,            // body dot-path when the id is not in the URL path
}
```

`services/proxy_authorization.rs`:

```rust
/// Side-effect-free. Called from BOTH executors after the final method/path/body
/// exist and before approval, billing, node transport, or forwarding.
pub fn authorize_proxy_operation(
    policy: Option<&ProxyOperationPolicy>, actor: &EffectiveActor,
    method: &str, path: &CanonicalPath, token: Option<&VerifiedResourceToken>,
) -> AppResult<OperationDecision>;   // Allowed { mint: Option<&ResourceTokenMint> } | NotFound-shaped denial
```

**Call sites (both executors):** the REST/WS/node executor in `handlers/proxy.rs` immediately after target resolution and before its forward/node/WS branches; and `mcp_service::execute_tool` (`mcp_service.rs:3044`) before its own node forward and its direct `proxy_service::forward_request` call (`proxy_service.rs:2882` is the shared forward; the MCP generic-path tool gets the same check). The policy travels on the resolved `ProxyTarget`/service so both executors receive it without re-reading Mongo. Denial: `NotFound` (existence non-leak), with a `warn` carrying service slug, method, canonical path, and a reason discriminator — same observability convention PR-A established for the credential gate.

## A.2 Canonical path contract

One constructor, `CanonicalPath::parse(&str) -> AppResult<CanonicalPath>`, used by both executors for matching **and** forwarding: method uppercased separately; percent-decode exactly once; **reject** (never normalize) encoded separators (`%2F`, `%5C`), dot segments, fragments, duplicate slashes, trailing slashes, and control/space characters; strip the query string before matching (it forwards, but never participates in a match); case-sensitive. The existing path-injection validators (`proxy_service.rs:471, 491`) validate narrower inputs and stay as they are; `CanonicalPath` is the request-path contract. Bypass tests per rejected variant.

## A.3 Resource tokens

**Format** — house JWT conventions (`kid` at `crypto/jwt.rs:22` and derivation at `:243`, `jti` at `:39,139,158`): RS256, claims `{iss, aud: "nyxid:resource-token", token_type: "resource_token", sub: <effective_owner_id>, res: "<resource_kind>:<id>", iat, exp, jti, kid}`. Response header `X-NyxID-Resource-Token` on the minting operation's response. `sub` is the proxy-resolution effective owner, computed by one shared function at mint and verify so org-member semantics cannot diverge.

**TTL (measured-fact update):** when the minting rule's `expiry_path` extracts a timestamp (Duffel: `payment_required_by`), `exp = that + 1 h` capped at **96 h**; extraction absent/failed → default **72 h**. Grounded in the measured 1–3-day hold windows, not the old ~24 h guess.

**Rotation:** verify against the current **and** previous public key (new `JWT_PUBLIC_KEY_PREVIOUS_PATH`, JWKS conventions reused); previous-key retirement **≥ 14 days** (> max TTL with margin); rotation runbook documented in `ENV.md`; test: token minted under key A verifies after rotation to key B while A remains as previous.

**Minting, fail-closed at runtime:** mint only on 2xx + `application/json` + identity encoding + body ≤ 1 MiB + `id_path` matching `id_pattern`. Anything else — including a runtime streaming response — is **not forwarded as success**: controlled 502-class error naming the reissue route (the provider write may exist). Admin write rejects minting rules on streaming-marked operations.

**Routes** (new handler module, human-session-only router, using `EffectiveActor::for_auth_user`):
- `POST /api/v1/resource-tokens/exchange` `{resource_token}` — verify signature/expiry/`sub == session user`; dispatch on `res` kind. **v1 ships the dispatch with no kinds registered** (the `duffel:order` → component-key exchange lands with the Duffel row in PR-C/D); this PR delivers verification, dispatch, and tests against a test-only kind.
- `POST /api/v1/resource-tokens/reissue` `{service_slug, resource_id}` — resolve the service, fetch the resource server-side via the credential gate, mint iff the resource's owner email equals the session user's verified email (**ASSUMPTION:** per-provider email field path — configured on the mint rule as `owner_email_path`, exercised by the test-only kind now, Duffel's real path fixture-verified in PR-C/D). Agent, delegated, and service-account callers rejected on both routes.

## A.4 Tests and gates

**Allowlist:** policy-bearing service × every entry shape — REST slug, REST UUID, `_nyxid_via`, WS upgrade, node-routed, typed MCP, generic MCP — for allowed and denied operations, each denial asserting **no downstream request was made** (stub-asserted); `None`-policy passthrough regression on an existing service; canonical-path bypass suite (each rejected variant); policy validation at admin write (bad glob, lowercase method, minting rule on streaming op).
**Tokens:** mint on create + header present; expiry derived from `expiry_path` vs default; `requires_resource_token` denial without/with-wrong/with-expired token on both executors; `sub` mismatch denial; org-member mint→verify agreement; rotation overlap; mutation-grade check: the token-required test must fail when `requires_resource_token` is flipped off (run once during review, PR-A style); exchange/reissue: session-only, wrong-user, forged, expired, email match/mismatch.
**Gate:** the standard CI set (fmt, clippy `-D warnings`, boundary script, billing smoke, build, nextest, CLI build+test); Mongo-backed tests `panic!` when the DB is absent (PR-A pattern); no frontend files.

## A.5 The workflow wait (Aevatar-side, specified here; measured-fact update)

```
after hold: deadline = payment_required_by; guarantee = price_guarantee_expires_at
loop:
  leg = min(deadline − now, 86_400_000 ms)          // wait_signal cap, verified
  wait_signal("payment_completed:<order_id>", leg)
  on wake (signal OR timeout):
    token-bearing GET order → paid-proof (awaiting_payment == false AND documents issued) → done
    now ≥ deadline → lapsed: inform + offer rebook; end
    guarantee passed & first time → notify user the price is no longer locked
    checkpoint; continue                              // ≤ 3–4 legs for measured 1–3 day holds
fallback: self_reschedule if looped waits prove awkward in the engine
```

## A.6 Local-dev notes (from the live end-to-end run)

`EMAIL_AUTH_ENABLED` defaults **off** — set it to register a local user. Admin is resolved through `role_ids` → the Admin role document, not the legacy `is_admin` boolean — assign the role, don't flip the flag. `Duffel-Version: v2` as a non-overridable `default_request_headers` entry is confirmed necessary (Duffel returns 400 without it).
