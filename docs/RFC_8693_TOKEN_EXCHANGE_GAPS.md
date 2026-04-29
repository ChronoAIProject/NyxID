# RFC 8693 Token Exchange — Known Gaps (Server Side)

> **Status**: discussion / known-gap log. Not a decision record. Created
> 2026-04-29. Companion document on the aevatar side at
> `<aevatar repo>/docs/history/2026-04/2026-04-29-rfc-8693-token-exchange-gaps.md`.

## TL;DR

NyxID currently advertises
**[RFC 8693](https://datatracker.ietf.org/doc/html/rfc8693) Token Exchange
(delegated access)** in [`docs/OIDC.md`](OIDC.md), [`docs/API.md`](API.md), and
[`docs/MCP_DELEGATION_FLOW.md`](MCP_DELEGATION_FLOW.md). What we actually
ship is a **purpose-built subset** of the spec — three concrete grant
shapes covering self-delegation, OAuth-broker binding exchange, and
social-IdP token exchange. It is **not a complete RFC 8693 server**.
This document records what is and isn't implemented so future readers
don't mistake "supported in OIDC.md" for "spec-complete". If we need
fuller RFC 8693 semantics (real impersonation chains, audience-bound
tokens, multi-token-type conversion), the checklist at the end is the
backlog.

## Context

- Role: **RFC 8693 authorization server** (token issuer for the
  `urn:ietf:params:oauth:grant-type:token-exchange` grant).
- Endpoint: `POST /oauth/token` (shared with all grants).
- Key code:
  - `backend/src/handlers/oauth.rs:1322-1562` — grant dispatch and three
    `subject_token_type` branches (social / `access_token` / broker
    `binding-id`)
  - `backend/src/services/token_exchange_service.rs` — the
    `access_token → delegated access_token` branch (5-minute TTL,
    consent-checked, scope-validated, chained-delegation rejected)
  - `backend/src/services/oauth_broker_service.rs` — broker binding
    exchange branch
  - `backend/src/services/social_token_exchange_service.rs` — provider
    (Google / GitHub) token → NyxID token branch
  - `backend/src/handlers/oidc_discovery.rs:30,51` — discovery metadata
- Discovery currently advertises only
  `urn:ietf:params:oauth:token-type:access_token` as a
  `token_endpoint_auth_methods_supported`-adjacent token type
  (`oidc_discovery.rs:30`).

## What is actually implemented

### Branches recognised at the token endpoint

| `subject_token_type` | Branch | What it does |
|---|---|---|
| `urn:ietf:params:oauth:token-type:access_token` (no `provider`) | `token_exchange_service::exchange_token` | Self-delegation: user's access token → 5-minute delegated access token. `act.sub = client_id`. Rejects chained delegation (`token_exchange_service.rs:59-70`). |
| `urn:ietf:params:oauth:token-type:access_token` + `provider` set | `social_token_exchange_service::exchange_social_token` | Provider (Google ID token / GitHub token) → full NyxID token set. Used by mobile native SDKs (`SOCIAL_TOKEN_EXCHANGE_MOBILE_INTEGRATION.md`). |
| `urn:nyxid:...:token-type:binding-id` (private URI) | `oauth_broker_service::exchange_via_binding` | Broker binding (opaque pointer) → short-lived access token. DPoP / mTLS sender-constraint enforced (`oauth.rs:1463-1511`). |
| anything else | rejected with `BadRequest("Unsupported subject_token_type: ...")` | |

### [RFC 8693 §2.1](https://datatracker.ietf.org/doc/html/rfc8693#section-2.1) request parameters

| Parameter | Status | Notes |
|---|---|---|
| `grant_type` | ✅ Required, validated | `oauth.rs:1322` |
| `subject_token` | ✅ Required, validated | `oauth.rs:1346-1349` |
| `subject_token_type` | ✅ Required, validated | `oauth.rs:1350-1353` |
| `scope` | ✅ Honored | `token_exchange_service.rs:85-88` validates against client's `delegation_scopes` |
| `actor_token` | ❌ Not parsed | |
| `actor_token_type` | ❌ Not parsed | |
| `requested_token_type` | ❌ Not parsed | Always issues `urn:ietf:params:oauth:token-type:access_token` |
| `audience` | ❌ Not parsed | |
| `resource` | ❌ Not parsed | |

### [RFC 8693 §2.2.1](https://datatracker.ietf.org/doc/html/rfc8693#section-2.2.1) response

| Field | Status | Notes |
|---|---|---|
| `access_token` | ✅ |  |
| `issued_token_type` | ✅ | Set on all branches (`oauth.rs:1442,1556,1385`) |
| `token_type` | ✅ | `Bearer` |
| `expires_in` | ✅ |  |
| `scope` | ✅ |  |
| `refresh_token` | ⚠️ Intentionally `None` for the `access_token` and broker branches (`oauth.rs:1438,1552`); set only on the social branch. [RFC 8693 §2.2.1](https://datatracker.ietf.org/doc/html/rfc8693#section-2.2.1) allows omission, but downstream readers may expect it. |

### Error handling ([RFC 8693 §2.2.2](https://datatracker.ietf.org/doc/html/rfc8693#section-2.2.2) / [RFC 6749 §5.2](https://datatracker.ietf.org/doc/html/rfc6749#section-5.2))

Standard OAuth error codes are emitted by `errors/mod.rs` (`invalid_request`
/ `invalid_client` / `invalid_grant` / `invalid_scope`). Format conforms
to [RFC 6749](https://datatracker.ietf.org/doc/html/rfc6749) (`oauth.rs:250-268`).

### `act` claim ([RFC 8693 §4.1](https://datatracker.ietf.org/doc/html/rfc8693#section-4.1))

We **emit** an `act.sub = client_id` on tokens minted by the self-delegation
branch (`jwt.rs:149-153`, the `ActorClaim` struct). We do **not** consume
or chain `act` on input — see Gaps §A.1 below.

### Sender-constrained tokens

DPoP and mTLS are validated **only on the broker branch** (`oauth.rs:1463-1511`).
The `access_token → delegated` branch does not bind a confirmation key,
so a leaked delegated token is bearer-only.

## Gaps (vs the full RFC)

### A. Impersonation / delegation modelling

1. **`actor_token` is not accepted.**
   [RFC 8693 §1.3](https://datatracker.ietf.org/doc/html/rfc8693#section-1.3)
   defines two distinct semantics — *impersonation* (token requester gets a
   token that hides the chain) and *delegation* (token has nested `act`
   claims that name every actor). Today we cannot accept "actor B asks to
   act on behalf of subject A, here is B's own token in `actor_token`".
   Adding this is a prerequisite for *any* multi-hop delegation chain
   across services.
2. **`act` chain is single-level.** `jwt::generate_delegated_access_token`
   sets `act.sub = client_id` once.
   [RFC 8693 §4.1](https://datatracker.ietf.org/doc/html/rfc8693#section-4.1)
   says nested `act` must accumulate when one delegated token is exchanged
   for another — but we reject chained delegation outright
   (`token_exchange_service.rs:59-70`) to prevent indefinite TTL extension.
   This is correct *given the absence of audience/scope narrowing*; once
   §B is in, the right answer is "allow chaining but force monotonic
   narrowing", not blanket reject.

### B. Audience / resource scoping

3. **`audience` not implemented.** Tokens issued today carry only
   `aud=client_id` (or NyxID's own audience), not
   [RFC 8707](https://datatracker.ietf.org/doc/html/rfc8707) resource
   indicators. A delegated token is therefore implicitly usable against
   any NyxID-protected resource the scope permits — there is no
   per-request audience narrowing.
4. **`resource` not implemented.** Same shape as `audience`. Should be
   handled together.

### C. Token-type coverage

5. **Only `access_token` (and the private broker URI) accepted as
   `subject_token_type`.**
   [RFC 8693 §3](https://datatracker.ietf.org/doc/html/rfc8693#section-3)
   lists six standard URIs (`access_token`, `refresh_token`, `id_token`,
   `saml1`, `saml2`, `jwt`). We are not interoperable as a *target* for
   clients that hold any other token type.
6. **Only `access_token` issued.** No `requested_token_type` honoring,
   so we cannot mint `id_token` or `jwt` on demand.
7. **Discovery does not advertise the broker URI.**
   `oidc_discovery.rs:30` lists only the standard `access_token` URI;
   the private `urn:nyxid:...:binding-id` is not in
   `subject_token_types_supported` / `issued_token_types_supported`,
   making the broker branch invisible to spec-driven clients.

### D. Sender-constraint asymmetry

8. **Self-delegation branch has no DPoP / mTLS binding.** The broker
   branch enforces sender-constraint (`oauth.rs:1463-1511`); the
   higher-trust self-delegation branch does not. A leaked 5-minute
   delegated bearer token is fully usable from anywhere. This is
   tolerable for short TTL but a real RFC 8693 deployment with longer
   TTLs would need confirmation-key binding here.

### E. Client-side gaps that show up here

9. The aevatar client (`NyxIdRemoteCapabilityBroker`) does not parse
   `issued_token_type` from our response. We emit the field correctly,
   but no current consumer reads it. Tracked in
   `<aevatar repo>/docs/history/2026-04/2026-04-29-rfc-8693-token-exchange-gaps.md`
   §"Gaps" #4.

### F. Test coverage

10. `token_exchange_service.rs:266-305` only unit-tests
    `validate_delegation_scope` and a placeholder for the chained-delegation
    guard. There is **no end-to-end test** that posts to `/oauth/token`
    with a `urn:ietf:params:oauth:grant-type:token-exchange` body and
    asserts on the full response shape, error code mapping, or DPoP
    binding round-trip.

## Why we are where we are

The current shape was driven by three concrete callers:

- OIDC-linked downstream services that need server-to-server calls on
  behalf of a logged-in user (`MCP_DELEGATION_FLOW.md` Flow B).
- aevatar's per-user NyxID broker binding model
  (`<aevatar repo>/docs/adr/0018-per-user-nyxid-binding-via-oauth-broker.md`),
  where aevatar must never hold a refresh token but needs to exchange
  an opaque binding pointer for a 5-minute access token per turn.
- Mobile apps using native social SDKs
  (`SOCIAL_TOKEN_EXCHANGE_MOBILE_INTEGRATION.md`), exchanging Google /
  GitHub tokens for NyxID tokens.

For all three, `actor_token` and `audience` were out of scope; refresh
tokens are deliberately omitted on the delegated branches to bound TTL
extension. The implementation matches the use cases. It does **not**
match the full spec.

## Future work — if and when we want a complete RFC 8693 server

Roughly ordered by ratio of payoff to cost:

1. **Advertise honestly in discovery.** Add
   `subject_token_types_supported`, `issued_token_types_supported`, and
   the broker URI (or hide it) in
   `/.well-known/openid-configuration`. Cheap, prevents clients from
   making wrong assumptions.
2. **End-to-end tests on `/oauth/token` with the token-exchange grant.**
   Cover happy path + every error-code branch + DPoP round-trip + the
   chained-delegation guard. Cheap, prevents silent regressions.
3. **`audience` / `resource` parameters.** Validate against
   per-resource-server registry; bake `aud` into the issued token. This
   is what unlocks a meaningful audit story ("this delegated token can
   only call resource X").
4. **`actor_token` + nested `act` chaining**, paired with **monotonic
   scope/audience narrowing** so the existing chained-delegation rejection
   becomes redundant. Significant work — needs an ADR on how multi-hop
   actor identity is modelled and how `delegated` flag interacts with a
   non-empty `act` chain.
5. **`requested_token_type=jwt` / `id_token`.** Add a self-contained
   JWT issuance path so token-exchange can hand out audience-bound,
   self-describing tokens for non-NyxID-protected downstream services.
6. **DPoP / mTLS confirmation on the self-delegation branch.** Mirror
   the broker branch's sender-constraint code.
7. **Standardise the broker `subject_token_type`.** Replace
   `urn:nyxid:...:binding-id` with a JWT-wrapped binding so clients can
   use `urn:ietf:params:oauth:token-type:jwt` and remain spec-portable.
   Requires changing the wire contract — coordinate with aevatar.

## References

- [RFC 8693 — OAuth 2.0 Token Exchange](https://datatracker.ietf.org/doc/html/rfc8693)
- [RFC 8707 — Resource Indicators for OAuth 2.0](https://datatracker.ietf.org/doc/html/rfc8707)
- [RFC 6749 — The OAuth 2.0 Authorization Framework](https://datatracker.ietf.org/doc/html/rfc6749)
- [`docs/OIDC.md`](OIDC.md) — current spec-support table (line 33)
- [`docs/MCP_DELEGATION_FLOW.md`](MCP_DELEGATION_FLOW.md) — Flow B usage
- [`docs/SOCIAL_TOKEN_EXCHANGE_MOBILE_INTEGRATION.md`](SOCIAL_TOKEN_EXCHANGE_MOBILE_INTEGRATION.md) — social branch
- [`docs/ARCHITECTURE.md`](ARCHITECTURE.md) — references at `:214`, `:1493`, `:1583`
- aevatar-side companion:
  `<aevatar repo>/docs/history/2026-04/2026-04-29-rfc-8693-token-exchange-gaps.md`
- aevatar broker design: `<aevatar repo>/docs/adr/0018-per-user-nyxid-binding-via-oauth-broker.md`
