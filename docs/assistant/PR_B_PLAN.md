# PR-B Plan — Operation Authorization + Resource Tokens

**Scope:** `TRAVEL_BOOKING.md` §A.2, on `travel-allowlist` off `main` (`0016d657`). This slice is **the blocker for creating a real Duffel row**: today the proxy forwards any path a caller supplies, so a Duffel row would expose `GET /air/orders` — every user's bookings on the shared platform credential — to any caller who can reach the row. Nothing travel-shaped ships in this PR; both mechanisms are generic platform primitives.

**Revised after adversarial review** (`PR_B_PLAN_REVIEW.md`): the token contract was under-specified — it did not bind the token to the *requested* resource (B1), had no MCP transport (B2), and its subject model broke org bookings (B5). This revision fixes the contract rather than adding mechanisms beside it. It also **corrects a decision the previous revision got wrong** (B4): `EffectiveActor::for_auth_user(&AuthUser)` would not have preserved the seal, because `AuthUser` is publicly constructible and is already fabricated in production code (`services/channel_event_service.rs:426-448`; fields public at `mw/auth.rs:47-87`). The honest resolution turns out to need no new constructor at all — see "The actor question, resolved by removal" below.

**Out of scope:** the Duffel row and overlay (PR-C/D), the `payment.complete` manifest entry, the payment card, any frontend, and — moved out by this revision — the **reissue route** (B6: it needs a per-provider descriptor that only exists once the Duffel policy and fixtures do; it lands in PR-C/D and **must land before the Duffel row is enabled**).

---

# Part I — Narrative

## What this slice delivers

1. **The proxy learns to say no.** A service declares its operations — method + path template — and the proxy enforces the list **in the data plane, at every entry point**. A path not on the list never leaves NyxID. Newly created platform-credential rows **fail closed**: a row with a usable master credential cannot be saved without a policy, and an empty rule list means deny-all (N3 — creating the future Duffel row without an allowlist must be impossible, not inadvisable).
2. **The proxy learns whose order is whose, without a database.** Creating a resource through a minting operation stamps the response with a **signed resource token**. Protected operations require presenting it, and — the B1 fix — the check compares the token's resource id against **the id in this request** (extracted from the path template's capture or a declared body path), failing closed when it cannot extract one. A token for your order is a token for *that order*, not a skeleton key for the service.

## The token, end to end, on both executors (B2 fix)

MCP tools exchange structured content, not HTTP headers — so a header-only token would simply not exist for the primary consumer. The contract is now executor-neutral:

- Both executors produce a shared `ProxyExecutionResult { status, headers, body, minted_token }`. REST surfaces the token as the `X-NyxID-Resource-Token` response header; **MCP appends a structured content block** after the provider body — `{"nyxid_resource_token": "...", "resource": "duffel:order:ord_X", "expires_at": "..."}` — leaving the provider JSON untouched (no envelope; existing consumers keep parsing what they parsed).
- Presentation: REST sends the request header; **MCP protected tools take a reserved `_nyxid_resource_token` argument**, injected into their schemas (typed and generic), consumed by NyxID before forwarding and never sent downstream. The existing block on `x-nyxid-*` header arguments (`mcp_service.rs:2399-2403`) stays — the argument is the sanctioned channel.
- The same order created via MCP and paid via the browser card works because the token is one artifact crossing both surfaces; the acceptance test drives exactly that flow (create via MCP → token from the content block → protected REST read → exchange-dispatch), on direct and node-routed responses, in both tool forms.

## Who the token names (B5 fix)

Two claims, because two identities exist: **`sub` — the authenticated person or the agent key's owning user** (who can later hold a browser session), and **`owner` — the proxy-resolution effective owner** (an org id for org-owned services, else equal to `sub`). Proxy-side protected operations check `owner` (custody per account, as designed — the agent books, the human pays, org-mates share); the human exchange route checks `sub == session user` and, when `owner != sub`, revalidates that `sub` is currently a member of `owner`. So an org booking mints a token its human booker can actually exchange — the previous single-subject design made that impossible, since org users cannot log in (`docs/site/shared/concepts/organizations.md:74-78`). Service-account callers get `sub` = the SA id, which no session can ever match: **SA-created orders cannot reach the human payment flow, by construction** — stated, not discovered later.

## The actor question, resolved by removal (B4 fix)

The previous revision proposed `pub fn EffectiveActor::for_auth_user(&AuthUser)` and claimed it preserved the type-level seal. **That claim was false** — anyone can build an `AuthUser` literal. Rather than publish a constructor under a false claim, the design removes the need: the exchange route verifies the session and the token's claims, which needs no `EffectiveActor`; and the future Duffel component-key exchange reaches the provider through the **server-chosen** path (`authorize_master_credential_server_chosen`, public-rows-only — and the Duffel row is public). Actor construction stays private to `proxy_service`, exactly as PR-A left it. The guarantee stated in `TRAVEL_BOOKING.md` remains the module-boundary one, which is true. If a *private*-row token kind ever needs exchange, that requires a real extractor-produced-principal design — deferred until a use case exists, and said so here rather than papered over.

## What the live API changed (measured 2026-08-14, `fixtures/duffel-offer-request-sample.json`)

Unchanged from the previous revision, restated for self-containment: the transport already works end to end (84-offer search through `/api/v1/proxy/s/duffel/air/offer_requests`, zero new proxy code; omitting `Duffel-Version: v2` yields Duffel 400, confirming the non-overridable default header). **78% of offers are holdable** (495/635). **Hold windows run 1–3 days**, breaking any single-`wait_signal` design (24 h cap) — the workflow uses a deadline-driven re-entrant loop (≤ 3–4 legs), `self_reschedule` as fallback. **`price_guarantee_expires_at` precedes `payment_required_by`** on the measured holdable offer, so re-price inside the hold is a live branch: the payment card re-reads totals on open, `price_changed` on submit is a normal refresh-and-reconfirm path, and token TTL derives from `payment_required_by`.

## What is deliberately not built

No per-user provider identity, no order mapping, no token revocation store (expiry bounds it; `jti` enables tracing later), no allowlist UI, no reissue in this PR (PR-C/D, before row enablement), no generic glob grammar (a deliberately small template grammar instead — N5).

---

# Appendix A — Implementation contract

Citations verified on `travel-allowlist` at `65fb546c`; **ASSUMPTION** marks what is not.

## A.1 Policy model

On `DownstreamService`, in the idiom of `AnonymousEndpointRule` (`models/downstream_service.rs:115-123`, stored list at `:334`):

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub proxy_operation_policy: Option<ProxyOperationPolicy>,
// None = passthrough for EXISTING rows only (grandfathered; startup audit warns on
// credentialed passthrough rows). Admin create/update REJECTS a row where
// master_credential_required && credential non-empty && policy is None (N3).
// Some(policy) with empty rules = deny-all. Never allow-all.

pub struct ProxyOperationPolicy { pub rules: Vec<ProxyOperationRule> }
pub struct ProxyOperationRule {
    pub method: String,                     // uppercase, validated at admin write
    /// Path TEMPLATE, not a glob (N5): literal segments and single-segment
    /// {name} captures only. No "**", no alternation, no character classes,
    /// anchored at root. Admin responses echo the normalized operation set.
    pub path_template: String,              // e.g. "/air/orders/{id}"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mints_resource_token: Option<ResourceTokenMint>,
    #[serde(default)]
    pub requires_resource_token: bool,
    /// B1: exactly one extractor must exist when requires_resource_token —
    /// either a capture named "id" in path_template, XOR this body dot-path.
    /// Validated at admin write; ambiguous or absent extractor = rejected.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resource_token_id_path: Option<String>,
}
pub struct ResourceTokenMint {
    pub resource_kind: String,              // "duffel:order"
    pub id_path: String,                    // "data.id"
    pub id_pattern: String,                 // "^ord_[A-Za-z0-9]+$"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expiry_path: Option<String>,        // "data.payment_required_by"
}
```

## A.2 The shared check (B1 fix in the signature)

`services/proxy_authorization.rs`:

```rust
/// Side-effect-free. Called from BOTH executors after the final method/path/body
/// exist and before approval, billing, node transport, or forwarding.
pub fn authorize_proxy_operation(
    policy: Option<&ProxyOperationPolicy>, method: &str, path: &CanonicalPath,
    body: Option<&[u8]>,                    // for body-path resource extraction
    token: Option<&VerifiedResourceToken>,  // pre-verified upstream (signature, exp, kid)
) -> AppResult<OperationDecision>;
// Allowed { mint: Option<&ResourceTokenMint> } | NotFound-shaped denial.
// For requires_resource_token rules: extract RequestedResource { kind, id }
// from the rule's declared extractor (path capture "id" or body path);
// extraction failure = deny; then require token.res == "{kind}:{id}" exactly
// AND token.owner == the current proxy-resolution effective owner.
```

**Call sites:** REST/WS/node executor in `handlers/proxy.rs` — the buffered body exists before approval (`handlers/proxy.rs:1817-1859`) and billing opens later (`:2131`), so the check slots between resolution and approval; and `mcp_service::execute_tool` — final method/path/body at `mcp_service.rs:3069-3075`, before its billing open (`:3398-3410`) and forward (`:3412-3419`). The policy travels on the resolved target so neither executor re-reads Mongo. Denials: `NotFound` + `warn` with slug/method/path/reason (PR-A observability convention).

## A.3 Canonical path (B3 fix: per-entry constructors, one invariant)

Two constructors, one shared invariant, matching **and** forwarding from the same value:

- `CanonicalPath::from_raw_uri_path(&OriginalUri)` — REST: derive the downstream tail from the *wire-form* URI (the existing `OriginalUri` validation at `handlers/proxy.rs:619-628, 841-850` is the base — Axum's wildcard pre-decodes, proven by the in-tree test at `:5651-5677`, so the wire form is the only trustworthy input). Reject encoded separators (`%2F`, `%5C`), encoded dots in dot-segment position, fragments; then decode exactly once.
- `CanonicalPath::from_literal(&str)` — MCP: tool-argument paths are literal, already-decoded strings; **any `%` is rejected outright** (no legitimate provider path contains one), as are backslashes.
- Shared invariant, both constructors: no dot segments, no duplicate or trailing slashes, no control/space characters; query stripped before matching (forwarded separately); case-sensitive. **Forwarding re-encodes deterministically from the canonical segments**, so the matched path and the forwarded path cannot diverge.
- Parity tests: every adversarial spelling through REST and MCP → same allow/deny **and** byte-identical downstream request target.

## A.4 Resource tokens

**Format** (house conventions: `kid` `crypto/jwt.rs:22, 243`; `jti` `:39,139,158`): RS256, claims `{iss, aud: "nyxid:resource-token", token_type: "resource_token", sub: <authenticated person / agent-key owner / SA id>, owner: <effective owner id>, res: "<kind>:<id>", iat, exp, jti, kid}`. `sub`/`owner` semantics per Part I (B5). One shared function computes the effective owner at mint and at verify.

**TTL:** `expiry_path` extract (+1 h, cap 96 h); absent/failed → 72 h. Grounded in measured 1–3-day holds.

**Rotation (N2 — a verification model, not a runbook):** `JwtKeys` grows a key ring: `previous: Option<{kid, decoding_key}>` loaded from `JWT_PUBLIC_KEY_PREVIOUS_PATH`; **verification selects the key by the token's `kid`** (unknown kid = reject); JWKS publishes both keys; startup **fails hard** if the previous key is configured but unparseable; previous-key verification applies to **resource tokens only** in this PR (other token classes keep current behavior — their lifetimes are minutes, not days). Retirement floor ≥ 14 days (> max TTL + margin), enforced by the runbook *and* verified by test: token minted under key A verifies after rotation to B-current/A-previous; unknown-kid token rejects. Deployment order documented in `ENV.md`.

**Minting, fail-closed at runtime:** mint only on 2xx + `application/json` + identity encoding + body ≤ 1 MiB + `id_path` matching `id_pattern`. Otherwise the response is **not forwarded as success**: a distinct **provider-outcome-unknown** error (N4) carrying the correlation id and explicit guidance — *"the provider write may have succeeded; do not re-create; recover via reissue"* — with the honest note that reissue lands in PR-C/D, **before** the Duffel row is enabled (until then no minting rule exists in production, so the branch is unreachable outside tests). Admin write rejects minting rules on streaming-marked operations; runtime streaming on a minting match takes the same fail-closed branch.

**Routes** (this PR: exchange only; reissue → PR-C/D per B6):
- `POST /api/v1/resource-tokens/exchange` `{resource_token}` — **session-only** (N1: an explicit `AuthMethod::Session` guard on these routes — the existing human-only router does not reject ordinary OAuth access tokens, `mw/auth.rs:27-45`, `routes.rs:1501-1564`); verify signature/`kid`/expiry; require `sub == session user`; when `owner != sub`, revalidate current membership of `sub` in `owner`; dispatch on `res` kind. **v1 ships dispatch with a test-only kind registered** — the `duffel:order` → component-key exchange lands with the Duffel row (PR-C/D) via the server-chosen credential path (public rows only; no actor construction — Part I B4).

## A.5 Tests and gates

**Allowlist:** policy service × every entry shape (REST slug/UUID/`_nyxid_via`/WS/node, typed MCP, generic MCP) × allow/deny, denials stub-asserting no downstream request; `None`-policy passthrough regression; **empty-rules deny-all**; N3 creation invariant (credentialed row without policy rejected at create and update — `handlers/services.rs:686-707`, `:1281-1307`; Duffel-shaped seed fails validation); template grammar rejects `**`/alternation/classes/unanchored; canonical-path parity suite (B3).
**Tokens:** mint + REST header + **MCP content block**; `_nyxid_resource_token` argument consumed, never forwarded; **B1 acceptance: token for `ord_A` denied against `ord_B`** by path capture and by body path, on REST, typed MCP, generic MCP; extraction-failure deny; `owner` mismatch deny; org flow: member mints (sub=member, owner=org) → member exchanges ✓, non-member session ✗, membership-revoked ✗; SA-minted token cannot exchange; TTL from `expiry_path` vs default; rotation kid-ring tests; exchange session-only matrix (session ✓; access-token, API key, delegated, SA, relay ✗); full **MCP create → token → protected read → exchange-dispatch** flow, direct and node, both tool forms; mutation-grade check during review (flip `requires_resource_token` off → token-required test must fail), PR-A style.
**Gate:** standard CI set; Mongo tests `panic!` when DB absent; no frontend files.

## A.6 Local-dev notes

`EMAIL_AUTH_ENABLED` defaults **off** — set it to register locally. Admin resolves through `role_ids` → the Admin role document, not the legacy `is_admin` boolean. `Duffel-Version: v2` as a non-overridable default header is confirmed necessary (live 400 without).
