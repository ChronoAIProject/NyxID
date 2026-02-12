# Delegated Access Plan

## 1. Executive Summary

NyxID currently requires every LLM gateway and proxy request to carry the end-user's own NyxID bearer token (`AuthUser` extractor). This works for direct user access but breaks two real-world flows:

1. **MCP-proxied services** (primary flow): A user calls an MCP tool that targets a downstream service, and that service needs to call back to NyxID's LLM gateway on behalf of the user. The service may not use NyxID as its OIDC provider.
2. **OIDC-linked services**: A downstream service that uses NyxID as its OIDC provider needs to call the LLM gateway on behalf of its authenticated users in server-to-server contexts outside of MCP.

This plan introduces a **delegated access pattern** with two complementary paths:

**Path A -- MCP Injection (Section 11):** When NyxID proxies an MCP tool call to a downstream service, it generates a short-lived (5-minute) delegation token and injects it as the `X-NyxID-Delegation-Token` header. The downstream service uses this token as a Bearer token when calling NyxID APIs. No OIDC relationship required.

**Path B -- Token Exchange (Sections 3-10):** Downstream services that are registered as OIDC clients can use **OAuth 2.0 Token Exchange (RFC 8693)** to exchange a user's access token for a short-lived (5min) delegated access token. This token can then be used at existing LLM gateway and proxy endpoints.

Both paths produce the same artifact: a standard NyxID JWT with `sub=user_id`, `act.sub=service_id`, `delegated=true`, and constrained scopes. All existing endpoints work unchanged with these tokens.

---

## 2. Current State Analysis

### 2.1 Authentication (`mw/auth.rs`)

The `AuthUser` extractor checks (in order):
1. `Authorization: Bearer <JWT>` header -- verifies a NyxID access token
2. `nyx_session` cookie -- validates a session hash
3. `nyx_access_token` cookie -- verifies a JWT from cookie
4. `X-API-Key` header -- validates a NyxID API key

All paths resolve to a `user_id: Uuid`. Every protected endpoint uses this extractor, so **there is no concept of "service acting on behalf of user"**.

### 2.2 LLM Gateway (`handlers/llm_gateway.rs`)

Three endpoints:
- `GET /api/v1/llm/status` -- returns which providers a user can use
- `ANY /api/v1/llm/{provider_slug}/v1/{*path}` -- direct provider proxy
- `ANY /api/v1/llm/gateway/v1/{*path}` -- unified OpenAI-compatible gateway

All three extract `auth_user.user_id` and pass it to `proxy_service::resolve_proxy_target()` and `delegation_service::resolve_delegated_credentials()`. The user_id is the key to look up which provider tokens the user has connected.

### 2.3 Proxy (`handlers/proxy.rs`)

`ANY /api/v1/proxy/{service_id}/{*path}` -- proxies to any downstream service. Same pattern: `auth_user.user_id` is used to resolve proxy targets, identity headers, and delegated credentials.

### 2.4 OAuth / OIDC (`handlers/oauth.rs`, `services/oauth_service.rs`)

NyxID is a full OIDC provider:
- `/oauth/authorize` -- authorization code flow with PKCE
- `/oauth/token` -- code exchange, token refresh
- `/oauth/userinfo` -- OIDC UserInfo endpoint
- `/oauth/introspect` -- RFC 7662 token introspection
- `/oauth/revoke` -- RFC 7009 token revocation
- `/oauth/register` -- RFC 7591 dynamic client registration

The `OauthClient` model supports `confidential` clients with secret hashing. The `oauth_service::authenticate_client()` function handles client authentication. The `Consent` model tracks user consent per `(user_id, client_id, scopes)`.

### 2.5 Provider Tokens (`services/delegation_service.rs`, `services/user_token_service.rs`)

Users connect to providers (OpenAI, Anthropic, etc.) and their tokens are stored encrypted in `user_provider_tokens`. The `delegation_service::resolve_delegated_credentials()` function loads these per-user tokens for injection into proxied requests.

### 2.6 The Gap

There is no mechanism for:
- A downstream service to authenticate as itself (not as a user)
- A service to specify "act on behalf of user X"
- NyxID to verify that user X consented to service Y acting on their behalf for a specific scope
- Issuing a token that represents "service Y acting as user X"

---

## 3. Proposed Architecture

### 3.1 Overview

```
Downstream Service                    NyxID                          LLM Provider
       |                               |                                |
       |  1. POST /oauth/token         |                                |
       |     grant_type=token_exchange  |                                |
       |     client_id + client_secret  |                                |
       |     subject_token=<user's AT>  |                                |
       |----------------------------->>|                                |
       |                               |  2. Validate client creds      |
       |                               |  3. Validate subject_token     |
       |                               |  4. Check consent              |
       |                               |  5. Issue delegated token      |
       |  <<----------------------------|                                |
       |  delegated_access_token        |                                |
       |                               |                                |
       |  6. POST /api/v1/llm/gateway/v1/chat/completions              |
       |     Authorization: Bearer <delegated_access_token>             |
       |----------------------------->>|                                |
       |                               |  7. Extract user from token    |
       |                               |  8. Resolve user's provider    |
       |                               |     credentials                |
       |                               |  9. Forward with credentials   |
       |                               |----------------------------->>|
       |                               |  <<----------------------------|
       |  <<----------------------------|                                |
       |  LLM response                 |                                |
```

### 3.2 Key Design Decisions

1. **RFC 8693 Token Exchange** as the standard mechanism, implemented on the existing `/oauth/token` endpoint via a new `grant_type=urn:ietf:params:oauth:grant-type:token-exchange`.

2. **Delegated access tokens** are standard NyxID JWTs with additional claims (`act.sub` for the acting service, constrained scopes). They work with the existing `AuthUser` extractor transparently.

3. **Consent is required** -- the user must have an active consent record for the downstream service's OAuth client. Consent is already auto-granted during OIDC login, so this is already satisfied in the normal flow.

4. **Scope restriction** -- delegated tokens are constrained to specific scopes (e.g., `llm:proxy`, `proxy:*`, `proxy:{service_id}`). The requesting service can only get scopes that are a subset of its `allowed_scopes`.

5. **Backwards compatible** -- existing direct user access continues to work unchanged. The delegated access pattern is additive.

---

## 4. Authentication Flow

### 4.1 Preconditions

Before delegated access works:
1. Downstream service is registered as a **confidential** OAuth client in NyxID (`client_id` + `client_secret`)
2. User has logged into the downstream service via NyxID OIDC (creating a consent record)
3. User has connected their LLM provider credentials in NyxID (e.g., OpenAI API key)

### 4.2 Token Exchange Flow (RFC 8693)

**Request:**
```http
POST /oauth/token HTTP/1.1
Content-Type: application/x-www-form-urlencoded

grant_type=urn:ietf:params:oauth:grant-type:token-exchange
&client_id=<downstream_service_client_id>
&client_secret=<downstream_service_client_secret>
&subject_token=<user's_nyxid_access_token>
&subject_token_type=urn:ietf:params:oauth:token-type:access_token
&scope=llm:proxy
```

- `subject_token`: The user's NyxID access token that the downstream service obtained during the OIDC login flow (or from a prior token refresh).
- `scope`: The requested scope for the delegated token. Must be a subset of the client's `allowed_scopes`.

**Response (success):**
```json
{
  "access_token": "<delegated_jwt>",
  "token_type": "Bearer",
  "expires_in": 300,
  "issued_token_type": "urn:ietf:params:oauth:token-type:access_token",
  "scope": "llm:proxy"
}
```

**Response (error):**
```json
{
  "error": "invalid_grant",
  "error_description": "User has not consented to delegation for this client"
}
```

### 4.3 Delegated Token Structure

The delegated access token is a standard NyxID JWT with extra claims:

```json
{
  "sub": "<user_id>",
  "iss": "nyxid",
  "aud": "http://localhost:3001",
  "exp": 1700000300,
  "iat": 1700000000,
  "jti": "<unique_id>",
  "scope": "llm:proxy",
  "token_type": "access",
  "act": {
    "sub": "<downstream_service_client_id>"
  },
  "delegated": true
}
```

- `sub`: The **user** being acted on behalf of (same as standard tokens)
- `act.sub`: The **service** (OAuth client_id) performing the action (RFC 8693 Section 4.1)
- `delegated`: Boolean flag to distinguish delegated from direct tokens
- `scope`: Constrained to only the requested delegation scopes

### 4.4 Using the Delegated Token

Once obtained, the delegated token is used exactly like a regular bearer token:

```http
POST /api/v1/llm/gateway/v1/chat/completions HTTP/1.1
Authorization: Bearer <delegated_access_token>
Content-Type: application/json

{
  "model": "gpt-4o",
  "messages": [{"role": "user", "content": "Hello"}]
}
```

The existing `AuthUser` extractor will decode the JWT, extract `sub` as the user_id, and everything downstream (credential resolution, proxy, etc.) works unchanged.

---

## 5. New/Modified Endpoints

### 5.1 Modified: `POST /oauth/token`

Add support for `grant_type=urn:ietf:params:oauth:grant-type:token-exchange`.

**New parameters (form-encoded):**
| Parameter | Required | Description |
|-----------|----------|-------------|
| `grant_type` | Yes | `urn:ietf:params:oauth:grant-type:token-exchange` |
| `client_id` | Yes | The downstream service's OAuth client ID |
| `client_secret` | Yes | The downstream service's OAuth client secret |
| `subject_token` | Yes | The user's NyxID access token |
| `subject_token_type` | Yes | `urn:ietf:params:oauth:token-type:access_token` |
| `scope` | No | Requested scopes (defaults to `llm:proxy`) |
| `audience` | No | Target audience (for future multi-tenant support) |

**Response:** Same `TokenResponse` struct with `issued_token_type` field added.

### 5.2 New: `GET /api/v1/llm/delegated/status`

Allows a downstream service to check which LLM providers a user has connected, using a delegated token.

Identical to `GET /api/v1/llm/status` but accessible with delegated tokens that have `llm:proxy` scope.

**Note:** This endpoint is optional for the initial implementation. The existing `/api/v1/llm/status` endpoint already works with the delegated token since it goes through `AuthUser`.

### 5.3 Unchanged Endpoints

All existing LLM gateway and proxy endpoints work unchanged with delegated tokens:
- `ANY /api/v1/llm/{provider_slug}/v1/{*path}`
- `ANY /api/v1/llm/gateway/v1/{*path}`
- `ANY /api/v1/proxy/{service_id}/{*path}`

The delegated JWT is a valid NyxID access token, so `AuthUser` extracts the user_id normally.

---

## 6. New/Modified Models

### 6.1 Modified: `OauthClient` Model

Add a field to track which scopes a client is allowed to request via token exchange:

```rust
// models/oauth_client.rs

pub struct OauthClient {
    // ... existing fields ...

    /// Space-separated scopes the client can request via token exchange.
    /// Empty string means token exchange is not allowed.
    /// Example: "llm:proxy proxy:*"
    #[serde(default)]
    pub delegation_scopes: String,
}
```

### 6.2 Modified: `Claims` (JWT)

Add optional delegation claims:

```rust
// crypto/jwt.rs

pub struct Claims {
    // ... existing fields ...

    /// RFC 8693 actor claim -- identifies the service acting on behalf of the user.
    /// Present only in delegated tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub act: Option<ActorClaim>,

    /// Flag indicating this is a delegated access token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegated: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ActorClaim {
    pub sub: String,
}
```

### 6.3 Modified: `AuthUser`

Add optional delegation context:

```rust
// mw/auth.rs

pub struct AuthUser {
    pub user_id: Uuid,
    pub session_id: Option<Uuid>,
    pub scope: String,
    /// If this is a delegated request, the OAuth client_id of the acting service.
    pub acting_client_id: Option<String>,
}
```

### 6.4 Modified: `DownstreamService` Model

Add fields to control delegation token injection via MCP:

```rust
// models/downstream_service.rs

pub struct DownstreamService {
    // ... existing fields ...

    /// Whether to inject a delegation token (X-NyxID-Delegation-Token)
    /// when proxying requests to this service via MCP or REST proxy.
    /// The token allows the service to call NyxID APIs on behalf of the user.
    #[serde(default)]
    pub inject_delegation_token: bool,

    /// Space-separated scopes for the injected delegation token.
    /// Default: "llm:proxy"
    #[serde(default = "default_delegation_scope")]
    pub delegation_token_scope: String,
}

fn default_delegation_scope() -> String {
    "llm:proxy".to_string()
}
```

### 6.5 No New Collections

No new MongoDB collections are needed. The existing `consents`, `oauth_clients`, `downstream_services`, and `user_provider_tokens` collections are sufficient.

---

## 7. New/Modified Services

### 7.1 New: `services/token_exchange_service.rs`

Core service for RFC 8693 Token Exchange.

```rust
/// Perform an OAuth 2.0 Token Exchange (RFC 8693).
///
/// 1. Authenticate the requesting client (client_id + client_secret)
/// 2. Validate the subject_token (user's access token)
/// 3. Verify the user has consented to this client
/// 4. Issue a constrained delegated access token
pub async fn exchange_token(
    db: &mongodb::Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    client_id: &str,
    client_secret: &str,
    subject_token: &str,
    subject_token_type: &str,
    requested_scope: Option<&str>,
) -> AppResult<TokenExchangeResponse> {
    // Step 1: Authenticate the requesting client
    let client = oauth_service::authenticate_client(db, client_id, Some(client_secret)).await?;

    // Step 2: Validate subject_token_type
    if subject_token_type != "urn:ietf:params:oauth:token-type:access_token" {
        return Err(AppError::BadRequest(
            "Only access_token subject_token_type is supported".to_string(),
        ));
    }

    // Step 3: Validate the subject token (user's access token)
    let subject_claims = jwt::verify_token(jwt_keys, config, subject_token)?;
    if subject_claims.token_type != "access" {
        return Err(AppError::BadRequest(
            "subject_token must be an access token".to_string(),
        ));
    }
    let user_id_str = &subject_claims.sub;

    // Step 4: Verify user has consented to this client
    // The consent was created when the user logged into the downstream service
    // via OIDC. We check that a consent record exists.
    let consent = consent_service::check_consent(
        db,
        user_id_str,
        client_id,
        "openid", // Minimal scope check -- user consented to OIDC login
    ).await?;

    if consent.is_none() {
        return Err(AppError::Forbidden(
            "User has not consented to delegation for this client".to_string(),
        ));
    }

    // Step 5: Validate requested scope against client's delegation_scopes
    let scope = validate_delegation_scope(
        requested_scope.unwrap_or("llm:proxy"),
        &client.delegation_scopes,
    )?;

    // Step 6: Issue delegated access token (short-lived: 5 minutes)
    let user_uuid = Uuid::parse_str(user_id_str)?;
    let delegated_token = generate_delegated_access_token(
        jwt_keys, config, &user_uuid, &scope, client_id,
        DELEGATED_TOKEN_TTL_SECS as i64,
    )?;

    Ok(TokenExchangeResponse {
        access_token: delegated_token,
        token_type: "Bearer".to_string(),
        expires_in: DELEGATED_TOKEN_TTL_SECS,
        issued_token_type: "urn:ietf:params:oauth:token-type:access_token".to_string(),
        scope,
    })
}
```

**Constants:**
- `DELEGATED_TOKEN_TTL_SECS = 300` (5 minutes) -- short-lived to limit blast radius

### 7.2 Modified: `services/oauth_service.rs`

No changes needed. The existing `authenticate_client()` and `validate_scopes()` functions are reused.

### 7.3 Modified: `services/consent_service.rs`

No changes needed. The existing `check_consent()` function is reused to verify the user's consent.

### 7.4 Modified: `services/mcp_service.rs`

The `execute_tool()` function needs to generate and inject a delegation token when the downstream service has `inject_delegation_token: true`:

```rust
// In execute_tool(), after building identity_headers and before forward_request():

if target.service.inject_delegation_token {
    let user_uuid = uuid::Uuid::parse_str(user_id)
        .map_err(|_| AppError::Internal("Invalid user_id".to_string()))?;

    match crate::crypto::jwt::generate_delegated_access_token(
        jwt_keys,
        config,
        &user_uuid,
        &target.service.delegation_token_scope,
        &service.service_slug,
        60, // 60-second TTL
    ) {
        Ok(delegation_token) => {
            identity_headers.push((
                "X-NyxID-Delegation-Token".to_string(),
                delegation_token,
            ));
        }
        Err(e) => {
            tracing::warn!(
                service_id = %service.service_id,
                error = %e,
                "Failed to generate delegation token for MCP tool"
            );
        }
    }
}
```

The same injection should also happen in `handlers/proxy.rs` for REST proxy requests, not just MCP tool calls.

### 7.5 New: `crypto/jwt.rs` additions

Add a function to generate delegated access tokens:

```rust
/// Generate a delegated access token (RFC 8693).
///
/// Like a regular access token, but with:
/// - `act.sub` claim identifying the acting service
/// - `delegated: true` flag
/// - Constrained scope (only delegation-specific scopes)
/// - Configurable short TTL
///
/// Use `ttl_secs = 300` for Token Exchange (OIDC path).
/// Use `ttl_secs = 60` for MCP injection (matches identity assertion TTL).
pub fn generate_delegated_access_token(
    keys: &JwtKeys,
    config: &AppConfig,
    user_id: &Uuid,
    scope: &str,
    acting_client_id: &str,
    ttl_secs: i64,
) -> Result<String, AppError> {
    let now = Utc::now().timestamp();

    let claims = Claims {
        sub: user_id.to_string(),
        iss: config.jwt_issuer.clone(),
        aud: config.base_url.clone(),
        exp: now + ttl_secs,
        iat: now,
        jti: Uuid::new_v4().to_string(),
        scope: scope.to_string(),
        token_type: "access".to_string(),
        roles: None,
        groups: None,
        permissions: None,
        sid: None,
        act: Some(ActorClaim {
            sub: acting_client_id.to_string(),
        }),
        delegated: Some(true),
    };

    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(keys.kid.clone());

    encode(&header, &claims, &keys.encoding)
        .map_err(|e| AppError::Internal(format!("Failed to encode delegated token: {e}")))
}
```

---

## 8. Handler Changes

### 8.1 Modified: `handlers/oauth.rs` -- `token()` handler

Add a match arm for the token exchange grant type:

```rust
pub async fn token(
    State(state): State<AppState>,
    Form(body): Form<TokenRequest>,
) -> AppResult<Json<TokenResponse>> {
    match body.grant_type.as_str() {
        "authorization_code" => { /* ... existing ... */ }
        "refresh_token" => { /* ... existing ... */ }

        // RFC 8693 Token Exchange
        "urn:ietf:params:oauth:grant-type:token-exchange" => {
            let client_id = body.client_id.as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing client_id".to_string()))?;
            let client_secret = body.client_secret.as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing client_secret".to_string()))?;
            let subject_token = body.subject_token.as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing subject_token".to_string()))?;
            let subject_token_type = body.subject_token_type.as_deref()
                .ok_or_else(|| AppError::BadRequest("Missing subject_token_type".to_string()))?;

            let result = token_exchange_service::exchange_token(
                &state.db,
                &state.config,
                &state.jwt_keys,
                client_id,
                client_secret,
                subject_token,
                subject_token_type,
                body.scope.as_deref(),
            ).await?;

            audit_service::log_async(
                state.db.clone(),
                Some(result.user_id.clone()),
                "token_exchange".to_string(),
                Some(serde_json::json!({
                    "client_id": client_id,
                    "scope": &result.scope,
                })),
                None,
                None,
            );

            Ok(Json(TokenResponse {
                access_token: result.access_token,
                token_type: "Bearer".to_string(),
                expires_in: result.expires_in,
                refresh_token: None, // No refresh token for delegated tokens
                id_token: None,
                scope: Some(result.scope),
            }))
        }

        other => Err(AppError::BadRequest(format!(
            "Unsupported grant_type: {other}"
        ))),
    }
}
```

### 8.2 Modified: `TokenRequest` struct

Add token exchange fields:

```rust
#[derive(Debug, Deserialize)]
pub struct TokenRequest {
    pub grant_type: String,
    pub code: Option<String>,
    pub redirect_uri: Option<String>,
    pub client_id: Option<String>,
    pub client_secret: Option<String>,
    pub code_verifier: Option<String>,
    pub refresh_token: Option<String>,
    // --- Token Exchange (RFC 8693) fields ---
    pub subject_token: Option<String>,
    pub subject_token_type: Option<String>,
    pub scope: Option<String>,
}
```

### 8.3 No Changes to LLM Gateway/Proxy Handlers

The LLM gateway and proxy handlers do not need any changes. The delegated JWT is a valid NyxID access token, so the `AuthUser` extractor handles it transparently:

1. `AuthUser` extracts `user_id` from the JWT's `sub` claim (the end user)
2. The handler passes `user_id` to proxy/delegation services
3. Credentials are resolved for the user as normal

The `act` and `delegated` claims are informational for audit logging.

---

## 9. Middleware Changes

### 9.1 Modified: `AuthUser` Extractor (`mw/auth.rs`)

The `AuthUser` extractor needs minimal changes. When decoding a Bearer JWT, populate the optional `acting_client_id` field:

```rust
if let Some(token) = auth_str.strip_prefix("Bearer ") {
    let claims = jwt::verify_token(&state.jwt_keys, &state.config, token)?;

    if claims.token_type != "access" {
        return Err(AppError::Unauthorized("Expected access token".to_string()));
    }

    let user_id = Uuid::parse_str(&claims.sub).map_err(|_| {
        AppError::Unauthorized("Invalid token subject".to_string())
    })?;

    // ... existing user active check ...

    return Ok(AuthUser {
        user_id,
        session_id: None,
        scope: claims.scope,
        acting_client_id: claims.act.map(|a| a.sub),
    });
}
```

### 9.2 Scope Enforcement (Optional, Phase 2)

For enhanced security, add optional scope checking to specific route groups:

```rust
/// Middleware that requires a specific scope in the token.
/// Used to restrict delegated tokens to only the endpoints they need.
pub async fn require_scope(
    auth_user: AuthUser,
    required_scope: &str,
    // ... axum middleware pattern ...
) -> Result<(), AppError> {
    if auth_user.acting_client_id.is_some() {
        // Delegated token -- enforce scope
        let scopes: Vec<&str> = auth_user.scope.split_whitespace().collect();
        if !scopes.contains(&required_scope) {
            return Err(AppError::Forbidden(
                "Insufficient scope for this operation".to_string(),
            ));
        }
    }
    Ok(())
}
```

This is optional for Phase 1 since the delegated token already has a 5-minute TTL and restricted scope.

---

## 10. Security Model

### 10.1 Trust Chain

```
User consents to Service (via OIDC login)
  --> Consent record: (user_id, client_id, scopes)

Service holds user's access token (from OIDC code exchange)
  --> Service can prove it has the user's token

Service exchanges token at NyxID
  --> NyxID verifies: service identity + user token + consent
  --> NyxID issues constrained delegated token

Service uses delegated token
  --> NyxID sees user_id in sub, resolves user's credentials
```

### 10.2 Security Properties

| Property | Mechanism |
|----------|-----------|
| **Service authentication** | Client credentials (client_id + hashed client_secret) |
| **User authentication** | Subject token is a valid, non-expired NyxID access token |
| **Consent verification** | Consent record must exist for (user_id, client_id) |
| **Scope limitation** | Delegated token scope is intersection of: requested scope, client's `delegation_scopes`, and client's `allowed_scopes` |
| **Time limitation** | Delegated tokens have 5-minute TTL (vs 15-minute for direct tokens) |
| **No credential exposure** | Service never sees user's provider credentials (API keys, OAuth tokens). Only NyxID resolves and injects them. |
| **Audit trail** | Token exchange and subsequent proxy requests are audit-logged with both user_id and acting_client_id |
| **Revocation** | Revoking user consent immediately prevents new token exchanges. Existing delegated tokens expire in 5 minutes max. |

### 10.3 Threat Mitigations

| Threat | Mitigation |
|--------|------------|
| **Stolen service credentials** | Client secret is hashed (SHA-256) in storage. Constant-time comparison prevents timing attacks. Service should rotate secrets periodically. |
| **Stolen subject token** | Subject tokens are short-lived (15 min). Even if stolen, the attacker also needs the client_secret to perform token exchange. |
| **Service acting without consent** | Consent check is mandatory. Auto-granted during OIDC login but can be revoked by the user at any time via `/users/me/consents/{client_id}`. |
| **Scope escalation** | Delegated scope is strictly limited by `delegation_scopes` on the client. Cannot exceed what the client is configured for. |
| **Token replay** | Delegated tokens have a JTI and 5-minute TTL. No refresh tokens are issued for delegated tokens. |
| **User deactivation** | `AuthUser` extractor already checks `user.is_active` for every request, including delegated tokens. |

### 10.4 Delegation Scopes

New scopes specific to delegated access:

| Scope | Access |
|-------|--------|
| `llm:proxy` | Access to LLM gateway and provider-specific proxy endpoints |
| `proxy:*` | Access to all proxy endpoints (`/api/v1/proxy/{service_id}/{*path}`) |
| `proxy:{service_id}` | Access to a specific service's proxy endpoint |
| `llm:status` | Read-only access to LLM status endpoint |

These scopes are only meaningful for delegated tokens. Direct user tokens have full access (backwards compatible).

---

## 11. MCP Identity Injection (Primary Flow)

### 11.1 Problem Statement

The **primary flow** for delegated access is MCP-centric, not OIDC-centric:

```
User -> MCP -> NyxID MCP Proxy -> Downstream Service -> NyxID LLM Gateway
```

A user connects to NyxID via MCP (e.g., from Claude Code or Cursor), calls an MCP tool that targets a downstream service, and that downstream service needs to call back to NyxID's LLM gateway on behalf of the same user. The downstream service may NOT use Nyx as its OIDC provider -- it could be any service registered in NyxID.

Today, `mcp_service::execute_tool()` already injects:
- **Identity headers** (`X-NyxID-User-Id`, `X-NyxID-User-Email`, `X-NyxID-User-Name`) via `identity_service::build_identity_headers()`
- **Identity assertion JWT** (`X-NyxID-Identity-Token`) via `identity_service::generate_identity_assertion()`
- **Delegated provider credentials** (e.g., user's OpenAI API key) via `delegation_service::resolve_delegated_credentials()`

**The gap:** None of these provide the downstream service with a token it can use to call NyxID's authenticated API endpoints (like the LLM gateway). The identity assertion JWT uses `IdentityAssertionClaims` (not `Claims`), so `AuthUser` cannot validate it. The downstream service has no way to make delegated requests back to NyxID.

### 11.2 Solution: Delegation Token Injection

When NyxID proxies an MCP tool call to a downstream service, it generates a **short-lived delegation token** and injects it as an additional header. The downstream service can then use this token as a Bearer token when calling NyxID's API endpoints.

```
User (MCP Client)              NyxID                          Downstream Service              NyxID LLM Gateway
       |                        |                                    |                              |
       | 1. tools/call          |                                    |                              |
       |   "acme__analyze"      |                                    |                              |
       |----------------------->|                                    |                              |
       |                        | 2. Generate delegation token       |                              |
       |                        |    sub=user_id                     |                              |
       |                        |    act.sub=service_slug            |                              |
       |                        |    scope=llm:proxy                 |                              |
       |                        |    ttl=60s                         |                              |
       |                        |                                    |                              |
       |                        | 3. Proxy tool call to downstream   |                              |
       |                        |    + X-NyxID-User-Id: <user_id>    |                              |
       |                        |    + X-NyxID-Identity-Token: <jwt> |                              |
       |                        |    + X-NyxID-Delegation-Token: <delegation_jwt>                   |
       |                        |    + Service credentials           |                              |
       |                        |----------------------------------->|                              |
       |                        |                                    |                              |
       |                        |                                    | 4. Process tool request       |
       |                        |                                    | 5. Need LLM completion        |
       |                        |                                    |                              |
       |                        |                                    | 6. POST /api/v1/llm/gateway   |
       |                        |                                    |    Authorization: Bearer      |
       |                        |                                    |    <delegation_token>         |
       |                        |                                    |----------------------------->|
       |                        |                                    |                              |
       |                        |                                    |  7. AuthUser validates token  |
       |                        |                                    |     sub=user_id               |
       |                        |                                    |  8. Resolve user's provider   |
       |                        |                                    |     credentials               |
       |                        |                                    |  9. Forward to LLM provider   |
       |                        |                                    |                              |
       |                        |                                    | <--- 10. LLM response -------|
       |                        | <--- 11. Tool result --------------|                              |
       | <--- 12. MCP result ---|                                    |                              |
```

### 11.3 Delegation Token Properties

The delegation token injected via MCP is a standard NyxID access JWT:

```json
{
  "sub": "<user_id>",
  "iss": "nyxid",
  "aud": "http://localhost:3001",
  "exp": 1700000060,
  "iat": 1700000000,
  "jti": "<unique_id>",
  "scope": "llm:proxy",
  "token_type": "access",
  "act": {
    "sub": "<service_slug>"
  },
  "delegated": true
}
```

Key properties:
- **TTL: 60 seconds** -- matches the identity assertion TTL. The downstream service must use it within the scope of a single tool call.
- **scope: `llm:proxy`** -- restricts what the downstream service can do. Can be extended to `proxy:*` if the downstream service needs to call other proxy endpoints.
- **act.sub: service slug** -- identifies which service is using the token for audit purposes.
- **Standard NyxID JWT** -- `AuthUser` extractor validates it like any other access token. No changes to the auth middleware needed beyond what Section 9 already covers.

### 11.4 Implementation: `mcp_service::execute_tool()` Changes

The `execute_tool()` function in `services/mcp_service.rs` needs to generate and inject the delegation token alongside the existing identity headers:

```rust
// In execute_tool(), after building identity_headers:

// Generate delegation token for callbacks to NyxID (scope: llm:proxy)
// This allows the downstream service to call NyxID's LLM gateway
// on behalf of the authenticated user during this tool call.
if target.service.identity_propagation_mode != "none" {
    match generate_delegated_access_token(
        jwt_keys,
        config,
        &user_uuid,
        "llm:proxy",          // scope
        &service.service_slug, // acting service identifier
        60,                    // 60-second TTL (matches identity assertion)
    ) {
        Ok(delegation_token) => {
            identity_headers.push((
                "X-NyxID-Delegation-Token".to_string(),
                delegation_token,
            ));
        }
        Err(e) => {
            tracing::warn!(
                service_id = %service.service_id,
                error = %e,
                "Failed to generate delegation token for MCP tool"
            );
        }
    }
}
```

The delegation token is injected as the `X-NyxID-Delegation-Token` header alongside the existing identity headers. The downstream service reads this header and uses it as a Bearer token for NyxID API calls.

**Note: `identity_service.rs` does NOT need changes.** The delegation token is a NyxID access JWT (generated by `crypto/jwt.rs::generate_delegated_access_token()`), not an identity assertion. It is generated directly in `mcp_service::execute_tool()` and injected into the `identity_headers` vector, which `proxy_service::forward_request()` already handles. The existing identity headers and JWT assertions continue to serve their original purpose (telling the downstream service *who* the user is), while the delegation token serves a new purpose (letting the downstream service *act as* the user when calling back to NyxID).

### 11.5 Downstream Service Usage

A downstream service that receives a NyxID-proxied MCP tool call can use the delegation token like this:

```python
# Example: Python downstream service
from flask import request
import requests

@app.route("/api/analyze", methods=["POST"])
def analyze():
    # Read the delegation token from NyxID's injection
    delegation_token = request.headers.get("X-NyxID-Delegation-Token")
    user_id = request.headers.get("X-NyxID-User-Id")

    if not delegation_token:
        return {"error": "No delegation token provided"}, 400

    # Use the delegation token to call NyxID's LLM gateway
    llm_response = requests.post(
        "https://nyx.example.com/api/v1/llm/gateway/v1/chat/completions",
        headers={
            "Authorization": f"Bearer {delegation_token}",
            "Content-Type": "application/json",
        },
        json={
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "Analyze this data..."}
            ],
        },
    )

    # Process LLM response and return tool result
    analysis = llm_response.json()
    return {"result": analysis["choices"][0]["message"]["content"]}
```

### 11.6 Configuring Delegation Token Injection

Not all downstream services need a delegation token. Add a new field to `DownstreamService` to control this:

```rust
// models/downstream_service.rs

pub struct DownstreamService {
    // ... existing fields ...

    /// Whether to inject a delegation token (X-NyxID-Delegation-Token)
    /// when proxying requests to this service.
    /// The token allows the service to call NyxID APIs on behalf of the user.
    #[serde(default)]
    pub inject_delegation_token: bool,

    /// Space-separated scopes for the delegation token.
    /// Default: "llm:proxy"
    #[serde(default = "default_delegation_scope")]
    pub delegation_token_scope: String,
}

fn default_delegation_scope() -> String {
    "llm:proxy".to_string()
}
```

The `inject_delegation_token` flag (default `false`) controls whether the delegation token is generated and injected. The `delegation_token_scope` field allows admins to restrict what the delegation token can access.

### 11.7 Two Paths to Delegated Access

The plan now covers two complementary paths for downstream services to make delegated requests to NyxID:

| Path | When to Use | How It Works |
|------|-------------|--------------|
| **MCP Injection** (Section 11) | Downstream services called via NyxID's MCP proxy. Service does NOT need to be an OIDC client. | NyxID generates a delegation token and injects it as a header when proxying the MCP tool call. |
| **Token Exchange** (Sections 3-10) | Downstream services that use NyxID as their OIDC provider and need to make server-to-server calls outside of MCP. | Service exchanges the user's access token for a delegation token via RFC 8693 at `/oauth/token`. |

Both paths produce the same artifact: a delegated NyxID JWT with `sub=user_id`, `act.sub=service_id`, and constrained scopes. The LLM gateway and other NyxID endpoints handle both identically.

### 11.8 Security Model for MCP Injection

| Property | Mechanism |
|----------|-----------|
| **User authentication** | User authenticated to MCP via JWT/session before any tool call |
| **User intent** | User explicitly invoked the tool call; NyxID does not pre-generate tokens |
| **Service identity** | `act.sub` claim identifies the downstream service receiving the token |
| **Time limitation** | 60-second TTL -- token expires before the MCP tool call timeout |
| **Scope limitation** | Token scope restricted to `delegation_token_scope` configured on the service |
| **No credential exposure** | Downstream service sees the delegation token but never the user's provider credentials |
| **Single use window** | Token is generated per tool call; each tool execution gets a fresh token |
| **Audit trail** | Token `jti` is unique; LLM gateway logs include `acting_client_id` from `act.sub` |
| **Revocation** | Deactivating the service or user immediately prevents new token generation |

**Threat mitigations specific to MCP injection:**

| Threat | Mitigation |
|--------|------------|
| **Downstream service replays token** | 60-second TTL. Service cannot accumulate tokens for later use. |
| **Downstream service escalates scope** | Token scope is fixed by admin config (`delegation_token_scope`). Cannot request broader access. |
| **Token intercepted in transit** | Same TLS requirement as all proxy traffic. Token travels over HTTPS to the downstream service. |
| **Downstream service shares token** | Token is scoped to `llm:proxy` and has 60s TTL. Blast radius is limited even if shared. |
| **Unintended delegation** | `inject_delegation_token` defaults to `false`. Must be explicitly enabled per service by admin. |

---

## 12. Frontend Changes

### 12.1 Consent Management (Existing)

Users can already view and revoke consents:
- `GET /api/v1/users/me/consents` -- list all consents
- `DELETE /api/v1/users/me/consents/{client_id}` -- revoke consent

The frontend already has pages/components for this. No new UI is needed for basic delegated access.

### 12.2 Admin: OAuth Client Configuration (Minor)

The admin panel for managing OAuth clients should show the new `delegation_scopes` field:

**Admin OAuth Client Edit Form:**
- Add a "Delegation Scopes" input field (space-separated scopes)
- Show which delegation scopes are available (dropdown/chips)
- Default: empty (no delegation allowed)

**Files to modify:**
- `frontend/src/pages/admin-oauth-clients.tsx` -- add delegation_scopes field to the form
- `frontend/src/schemas/` -- add validation for delegation_scopes
- `frontend/src/types/` -- update OauthClient type

### 12.3 User Dashboard: Delegated Access Info (Phase 2)

In a later phase, show users which services have used delegated access on their behalf:
- "Service X made Y requests on your behalf in the last 24 hours"
- Based on audit log entries with `acting_client_id`

This is purely informational and not needed for the initial implementation.

---

## 13. Migration Strategy

### 13.1 Database Migration

**Collection: `oauth_clients`**

Add `delegation_scopes` field with empty string default:

```javascript
// docs/migrations/002-delegation-scopes.js
db.oauth_clients.updateMany(
  { delegation_scopes: { $exists: false } },
  { $set: { delegation_scopes: "" } }
);
```

This is non-breaking: existing clients get empty `delegation_scopes`, which means token exchange is not allowed until explicitly enabled.

**Collection: `downstream_services`**

Add `inject_delegation_token` and `delegation_token_scope` fields:

```javascript
// docs/migrations/003-delegation-token-injection.js
db.downstream_services.updateMany(
  { inject_delegation_token: { $exists: false } },
  { $set: { inject_delegation_token: false, delegation_token_scope: "llm:proxy" } }
);
```

This is non-breaking: existing services get `inject_delegation_token: false`, which means no delegation tokens are injected until explicitly enabled by an admin.

### 13.2 Backwards Compatibility

| Component | Impact |
|-----------|--------|
| **Existing user tokens** | Unchanged. Direct access tokens continue to work exactly as before. |
| **Existing OAuth flows** | Unchanged. Authorization code and refresh token grants are unaffected. |
| **Existing proxy/LLM endpoints** | Unchanged. They accept any valid `AuthUser` token. |
| **AuthUser extractor** | `acting_client_id` defaults to `None`, matching existing behavior. |
| **JWT Claims** | `act` and `delegated` fields use `skip_serializing_if = None`, so existing tokens (without these fields) deserialize fine. |
| **Token introspection** | Works with delegated tokens (they are standard JWTs). |
| **MCP tool calls** | Unchanged. `inject_delegation_token` defaults to `false`. Existing services see no new headers. |
| **REST proxy** | Unchanged. Same default applies for REST proxy requests. |

### 13.3 Rollout Plan

1. **Phase 1: Core** (this plan)
   - Add `delegation_scopes` to `OauthClient`
   - Add `act` + `delegated` to JWT `Claims`
   - Add `acting_client_id` to `AuthUser`
   - Implement `token_exchange_service.rs`
   - Add `token-exchange` grant type to `/oauth/token`
   - Add audit logging for delegated requests

2. **Phase 2: Scope Enforcement**
   - Add scope-checking middleware for LLM/proxy routes
   - Restrict delegated tokens to only their declared scopes
   - Add delegation scope management in admin UI

3. **Phase 3: Observability**
   - Dashboard showing delegated access usage per service
   - Rate limiting per (client_id, user_id) pair for delegated tokens
   - User-facing activity log showing delegated access

---

## 14. Implementation Order

### Step 1: Model Changes (< 1 hour)
1. Add `delegation_scopes: String` to `OauthClient` in `models/oauth_client.rs`
2. Add `ActorClaim` struct and `act: Option<ActorClaim>`, `delegated: Option<bool>` to `Claims` in `crypto/jwt.rs`
3. Add `acting_client_id: Option<String>` to `AuthUser` in `mw/auth.rs`
4. Update all `AuthUser` construction sites to include `acting_client_id: None`
5. Add `subject_token`, `subject_token_type`, `scope` to `TokenRequest` in `handlers/oauth.rs`
6. Add `inject_delegation_token: bool` and `delegation_token_scope: String` to `DownstreamService` in `models/downstream_service.rs`
7. Write migration scripts for `oauth_clients` and `downstream_services`

### Step 2: JWT Generation (< 1 hour)
1. Add `generate_delegated_access_token()` to `crypto/jwt.rs` with configurable TTL parameter
2. Add unit tests for delegated token generation and verification
3. Verify existing `verify_token()` works with the new claims (it should, since unknown fields are ignored by serde)

### Step 3: Token Exchange Service (~ 2 hours)
1. Create `services/token_exchange_service.rs`
2. Implement `exchange_token()` with:
   - Client authentication
   - Subject token validation
   - Consent verification
   - Delegation scope validation
   - Delegated token generation
3. Add `validate_delegation_scope()` helper
4. Write unit tests

### Step 4: Handler Integration (~ 1 hour)
1. Add `token-exchange` match arm in `handlers/oauth.rs :: token()`
2. Update `TokenResponse` to support `issued_token_type` (or use `scope` field)
3. Wire up audit logging for token exchange events

### Step 5: MCP Delegation Token Injection (~ 1.5 hours)
1. Update `mcp_service::execute_tool()` to generate and inject delegation tokens
2. Only inject when `target.service.inject_delegation_token` is true
3. Use `target.service.delegation_token_scope` for the token scope
4. Token TTL: 60 seconds (matching identity assertion TTL)
5. Inject as `X-NyxID-Delegation-Token` header via `identity_headers`
6. Add audit logging for delegation token generation in MCP context

### Step 6: Auth Middleware Update (< 30 minutes)
1. Update `AuthUser::from_request_parts()` to extract `act.sub` from JWT claims
2. Set `acting_client_id` on `AuthUser` when present
3. Update all other `AuthUser` construction sites (session, cookie, API key) to set `acting_client_id: None`

### Step 7: Audit Logging Enhancement (< 30 minutes)
1. Include `acting_client_id` in audit log entries for proxy and LLM gateway requests
2. Add `acting_client_id` field to the `audit_service::log_async()` event data when present

### Step 8: Admin API (~ 1 hour)
1. Update admin OAuth client create/update handlers to accept `delegation_scopes`
2. Update admin OAuth client list response to include `delegation_scopes`
3. Update admin downstream service create/update handlers to accept `inject_delegation_token` and `delegation_token_scope`

### Step 9: Tests (~ 3 hours)
1. Unit tests for `token_exchange_service`
2. Unit tests for delegated token JWT generation/verification
3. Unit tests for scope validation
4. Integration test: full token exchange flow (OIDC path)
5. Integration test: MCP tool call with delegation token injection
6. Integration test: delegated token used at LLM gateway
7. Negative tests: expired subject token, missing consent, wrong client_secret, invalid scope
8. MCP injection tests: `inject_delegation_token=false` does not inject, correct scope in token

### Step 10: Documentation (< 1 hour)
1. Update `docs/API.md` with token exchange endpoint docs
2. Add delegation flow to `docs/ARCHITECTURE.md` (both OIDC and MCP paths)
3. Add migration instructions to `docs/migrations/`
4. Document the `X-NyxID-Delegation-Token` header for downstream service developers

### Step 11: Frontend (~ 1 hour)
1. Add `delegation_scopes` field to admin OAuth client forms
2. Add `inject_delegation_token` toggle and `delegation_token_scope` field to admin downstream service forms
3. Update TypeScript types for OauthClient and DownstreamService

**Total estimated effort: ~12 hours**

---

## 15. Delegation Token Refresh (Implemented)

### 15.1 Problem

The original 60-second TTL for MCP-injected delegation tokens was too short for agentic/long-running LLM workflows. Downstream services performing multi-step operations (e.g., chain-of-thought reasoning with multiple LLM calls) could see their delegation tokens expire mid-execution.

### 15.2 Solution

Two changes address this:

1. **Increased MCP injection TTL to 5 minutes** (`MCP_DELEGATION_TOKEN_TTL_SECS` changed from 60 to 300). Both MCP-injected and token-exchange tokens now have the same 5-minute TTL.

2. **New refresh endpoint: `POST /api/v1/delegation/refresh`**. Downstream services can renew their delegation token before it expires, receiving a new token with fresh 5-minute TTL.

### 15.3 Refresh Endpoint

```
POST /api/v1/delegation/refresh
Authorization: Bearer <current_delegation_token>

Response:
{
  "access_token": "<new_delegation_token>",
  "token_type": "Bearer",
  "expires_in": 300,
  "scope": "llm:proxy"
}
```

**Security properties:**
- Only delegated tokens accepted (regular user tokens return 403)
- User must still be active for refresh to succeed
- New token inherits `act.sub` and scope from the original
- Each refresh generates a new JTI
- Audit-logged as `delegation_token_refreshed`

### 15.4 Route Placement

The refresh endpoint is in the delegated-allowed route group (alongside `/api/v1/llm/*` and `/api/v1/proxy/*`), so it is NOT blocked by `reject_delegated_tokens` middleware.

### 15.5 Implementation Files

| File | Change |
|------|--------|
| `crypto/jwt.rs` | `MCP_DELEGATION_TOKEN_TTL_SECS` changed from 60 to 300 |
| `services/token_exchange_service.rs` | Added `refresh_delegation_token()` and `DelegationRefreshResponse` |
| `handlers/delegation.rs` | New handler module with `refresh_delegation_token()` endpoint |
| `handlers/mod.rs` | Registered `delegation` module |
| `routes.rs` | Added `POST /api/v1/delegation/refresh` to delegated-allowed routes |

---

## 16. Open Questions / Future Considerations

1. **Should we support `client_credentials` grant type?** This would let a service get a service-level token (no user context). Not needed for the current use case but could be useful for service-to-service health checks.

2. **Token exchange for refresh tokens?** Currently only access tokens can be exchanged. Supporting refresh token exchange would allow longer-lived delegated sessions, but increases the security surface.

3. **Per-user rate limiting for delegated tokens?** A service could burn through a user's LLM quota by making many delegated requests. Consider adding rate limits per (client_id, user_id) pair for both MCP-injected and token-exchanged tokens.

4. **Delegation consent UI?** For the OIDC path, consent is auto-granted during OIDC login. For the MCP path, the user implicitly consents by calling the tool. Should we add a separate delegation consent step for either path? ("Service X wants to make LLM requests on your behalf -- Allow?")

5. **Token caching?** For the Token Exchange path, downstream services should cache delegated tokens for their 5-minute TTL to avoid repeated exchanges. For the MCP injection path, each tool call gets a fresh 5-minute token that can be refreshed via `POST /api/v1/delegation/refresh`.

6. **Should MCP-injected delegation tokens also be available via REST proxy?** Currently the plan includes injection in both `mcp_service::execute_tool()` and `handlers/proxy.rs`. This means any service with `inject_delegation_token: true` gets the token whether accessed via MCP or direct REST proxy. Is this desirable, or should it be MCP-only?

7. **Multiple concurrent LLM calls?** Downstream services can now refresh their delegation token via `POST /api/v1/delegation/refresh` for long-running workflows. The 5-minute TTL plus refresh support covers multi-step agentic operations.

8. **Delegation token scope per tool call?** Currently the scope is fixed per-service (`delegation_token_scope`). Should we allow per-endpoint scope overrides, so that different MCP tools within the same service get different delegation scopes?
