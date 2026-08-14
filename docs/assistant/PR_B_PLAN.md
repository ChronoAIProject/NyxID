# PR-B Plan — The Operation Allowlist

**Scope (owner decision, 2026-08-14):** the allowlist, and nothing else. The entire resource-token design — signed tokens, key rotation, MCP token transport, exchange and reissue routes — is **deleted from this plan**. It existed solely to stop user A reading user B's order through the shared platform credential; with order reads simply not allowlisted, that capability does not exist through NyxID at all. Removing the problem is strictly stronger than defending against it, which is why two review rounds could not close the token contract and this closes it in one line. NyxID stores nothing, restricts search and buying, and does not offer re-reading.

**Branch:** `travel-allowlist`, rebased onto current `origin/main`. Nothing travel-shaped ships in this PR — the primitive is generic; the Duffel row and its policy land in PR-C/D.

---

# Part I — Narrative

## What this slice delivers

A service can declare its callable operations — method + path template — and the proxy enforces the list **in the data plane, at every entry point, before any side effect**. Default deny: nothing is reachable unless a rule allows it, an empty rule list allows nothing, and — the runtime fail-closed rule — **a platform-credentialed row with no policy is unusable, not open**.

The Duffel policy this enables (PR-C/D):

- **Allowed:** `POST /air/offer_requests`, `GET /air/offers`, `GET /air/offers/{id}` (search), `POST /air/orders` (create), `POST /air/payments` (pay).
- **Blocked:** `GET /air/orders` (the cross-user list exposure), `GET /air/orders/{id}` (reads), cancellations, `/identity/component_client_keys`, and everything else — by absence, which is the whole mechanism.

## The two placement facts that make this correct

1. **Both executors, before side effects — and MCP's side effects start earlier than its executor.** The REST path has its final buffered body before approval (`handlers/proxy.rs:1817-1859`) and opens billing later (`:2131`), so the check slots after resolution, before approval. But MCP builds its operation descriptor and **creates/persists approval data before `execute_tool` is ever called** (direct dispatch: descriptor + approval at `mcp_transport.rs:1314-1329`, execution at `:1331-1353`; meta-tool repeats the order at `:1652-1663`, `:1680-1702`). A check inside `execute_tool` runs too late — a denied operation would already have minted an approval row. So MCP gets **one preparation primitive**, `prepare_proxy_tool_call(service, endpoint, arguments) -> PreparedProxyCall`, which constructs the final method/canonical path/query/body **exactly once**, runs the authorization check, and only then lets the caller derive the approval descriptor from the prepared request; both dispatch paths pass the same `PreparedProxyCall` into execution rather than rebuilding from raw arguments. Deny means: no approval row, no billing open, no node frame, no forward.
2. **REST and MCP inputs cannot share one parser.** Axum pre-decodes the REST wildcard (in-tree proof: `handlers/proxy.rs:5651-5677` shows `%2F` arriving as `/`), while an MCP path argument is a literal string. Two constructors, one invariant: `CanonicalPath::from_raw_uri_path(&OriginalUri)` for REST (wire form; reject encoded separators and dot-segments, then decode exactly once — building on the existing `OriginalUri` validation at `:619-628`, `:841-850`) and `CanonicalPath::from_literal(&str)` for MCP (**any `%` or backslash rejected outright** — no legitimate provider path contains one). Shared invariant: no dot segments, duplicate or trailing slashes, control/space characters; query stripped from matching; case-sensitive; forwarding re-encodes deterministically from canonical segments so match and forward cannot diverge.

With no tokens there is no reserved argument, so Round 2's collision finding (R2-N1) is moot by construction — nothing NyxID-shaped enters tool arguments or provider bodies. Confirmed: the check consumes only method + path; request bodies are not inspected.

## Fail closed at runtime, not just at write time (R2-B4)

Admin-side validation is kept but is not the control — seeds and migrations write `DownstreamService` documents directly (`provider_service.rs:3630-3685`), and ciphertext length cannot distinguish a secret from an encrypted empty string (`handlers/services.rs:828, 889-917`). The data-plane rule, keyed on what actually matters: at the enforcement point, if `master_credential_required(service)` (`proxy_service.rs:143-147` — `auth_method != "none"`, the same predicate the credential gate uses) and the row has **no policy and no explicit legacy marker**, deny everything. A one-time migration stamps `policy_mode: "legacy_passthrough"` on the credentialed rows that exist at deploy (enumerable, audited, logged); new rows can never acquire the marker. The ~30 seeded rows are `auth_method: "none"` and never inject a credential, so they pass through unaffected without markers. Result: forgetting the policy on a future credentialed row — the exact Duffel-row mistake this PR exists to prevent — yields a dead row, not an open one.

## The consequence to state plainly: no read-back through NyxID

Blocking order reads means **the agent cannot verify a booking succeeded through NyxID** — the paid-proof rule (`awaiting_payment == false` **and** documents issued) requires a read that is no longer callable. Fine for this slice, which ships the primitive and unblocks search; a real gap for the PR that builds payment. Recorded as an open item there, with the options named, not designed: (a) a server-side verification path inside NyxID that performs the order read itself and returns only a verdict — not an allowlisted caller operation; or (b) a narrowly-scoped read (e.g. allowlisted `GET /air/orders/{id}` with some ownership control, which would reopen the attribution question the owner closed). The payment PR must pick one before the paid state can be trusted.

## Measured facts (unchanged, `fixtures/duffel-offer-request-sample.json`, 2026-08-14)

78% of offers holdable (495/635); hold windows 1–3 days; `price_guarantee_expires_at` precedes `payment_required_by`, so re-price inside a hold is a live branch. Transport confirmed end to end through `/api/v1/proxy/s/duffel/air/offer_requests` with zero new proxy code; `Duffel-Version: v2` as a non-overridable default header is mandatory (live 400 without).

---

# Appendix A — Implementation contract

Citations verified on `travel-allowlist` (post-rebase); **ASSUMPTION** marks what is not.

## A.1 Policy model

On `DownstreamService`, in the idiom of `AnonymousEndpointRule` (`models/downstream_service.rs:115-123`, stored list at `:334`):

```rust
#[serde(default, skip_serializing_if = "Option::is_none")]
pub proxy_operation_policy: Option<ProxyOperationPolicy>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub policy_mode: Option<String>,   // "legacy_passthrough" — set ONLY by the one-time migration

pub struct ProxyOperationPolicy { pub rules: Vec<ProxyOperationRule> }   // empty = deny-all
pub struct ProxyOperationRule {
    pub method: String,        // uppercase, validated at admin write
    /// Path TEMPLATE, not a glob: literal segments and single-segment {name}
    /// captures only. No "**", no alternation, no character classes, anchored
    /// at root. Admin responses echo the normalized operation set.
    pub path_template: String, // e.g. "/air/offers/{id}"
}
```

Admin create/update (`handlers/services.rs:686-707`, `:1281-1307`): reject a write that leaves `master_credential_required(service)` true with `proxy_operation_policy: None` (unless the row already carries the legacy marker); validate method + template grammar; echo the normalized rule set.

## A.2 The check and its call sites

`services/proxy_authorization.rs`:

```rust
/// Side-effect-free. Consumes only method + canonical path.
pub fn authorize_proxy_operation(
    policy: Option<&ProxyOperationPolicy>, policy_mode: Option<&str>,
    master_credential_required: bool, method: &str, path: &CanonicalPath,
) -> AppResult<()>;
// Some(policy): allow iff a rule matches (empty rules = deny-all).
// None + legacy marker, or None + no master credential: passthrough (today's behavior).
// None + master credential + no marker: DENY ALL (R2-B4 runtime fail-closed).
// Denial: NotFound-shaped + warn {slug, method, path, reason} (PR-A convention).
```

**REST/WS/node:** called in `handlers/proxy.rs` immediately after target resolution, before the approval evaluation (`:1817-1859`) — which also puts it before billing (`:2131`), node transport, and forwarding. The policy fields travel on the resolved target; no extra Mongo read.

**MCP (R2-B1):** new `prepare_proxy_tool_call(...) -> PreparedProxyCall` in `mcp_service`, called by **both** dispatch paths **before** approval descriptor construction (`mcp_transport.rs:1314-1329` and `:1652-1663`): builds method/path/query/body exactly once (reusing the existing arg builders, `mcp_service.rs:2638-2667, 2731-2747`), runs `authorize_proxy_operation`, and returns the prepared request. Approval descriptors derive from the prepared value; `execute_tool` receives it and does not rebuild from raw arguments. The generic path tool gets the same treatment via `CanonicalPath::from_literal`.

## A.3 Canonical path

Per Part I: `from_raw_uri_path` (REST, wire-form, reject-then-decode-once) and `from_literal` (MCP, `%`/`\` rejected), one shared invariant, deterministic re-encode for forwarding. Existing validators (`proxy_service.rs:471, 491`) are narrower inputs and unchanged.

## A.4 Migration

One-time startup migration (house pattern: `migrate_legacy_api_spec_url`): stamp `policy_mode: "legacy_passthrough"` on every existing row where `master_credential_required` holds; log the slugs at `warn`; count recorded in the migration log. New writes never set it; admin API rejects attempts to set it.

## A.5 Tests

**Enforcement:** policy service × every entry shape — REST slug, REST UUID, `_nyxid_via`, WS upgrade, node-routed, typed MCP, generic MCP — allowed and denied, every denial stub-asserting **no downstream request**; **MCP deny leaves no approval row and no billing open** (the R2-B1 acceptance: invoke a denied protected tool, assert `approval_requests` unchanged and no meter row); `None`-policy + legacy-marker passthrough regression; `None`-policy + credentialed + no marker → deny-all (R2-B4 acceptance: insert such a row directly into Mongo, bypassing admin validation, assert every operation 404s); empty-rules deny-all; seeded `auth_method: "none"` rows unaffected.
**Grammar/path:** template validation rejects `**`, alternation, classes, unanchored; canonical-path parity suite — every adversarial spelling through REST and MCP asserts the same allow/deny **and** byte-identical forwarded target; `%2F`-in-wildcard REST case pinned against the existing decode test (`handlers/proxy.rs:5651-5677`).
**Admin:** credentialed write without policy rejected on create and update; legacy marker not settable via API; normalized rule echo.
**Gate:** standard CI set (fmt, clippy `-D warnings`, boundary script, billing smoke, build, nextest, CLI build+test); Mongo tests `panic!` when the DB is absent (PR-A pattern); no frontend files; mutation-grade check during review, PR-A style: flip the R2-B4 deny branch to passthrough → the smuggled-row test must fail.

## A.6 Local-dev notes

`EMAIL_AUTH_ENABLED` defaults **off** — set it to register locally. Admin resolves through `role_ids` → the Admin role document, not the legacy `is_admin` boolean. `Duffel-Version: v2` as a non-overridable default header is confirmed necessary (live 400 without).
