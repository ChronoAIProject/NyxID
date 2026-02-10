# Credential Broker & Identity Propagation Research

Research document for NyxID's credential broker, user OAuth connections, and identity
propagation features. Covers LLM provider authentication, token broker patterns in Rust,
identity propagation standards, and encrypted token storage.

---

## 1. LLM Provider Authentication Landscape

### 1.1 OpenAI / Codex

**Authentication methods:**

| Method | Description | Use Case |
|--------|-------------|----------|
| API Key | Static key via `Authorization: Bearer sk-...` | Standard API usage |
| ChatGPT OAuth | Browser-based OAuth 2.0 + PKCE | Codex CLI subscription access |
| Device Code | `codex login --device-auth`, user enters code on any browser | Headless/remote environments |
| Ephemeral Token | Short-lived token (1-10 min) minted by backend | Browser/mobile Realtime API |

**Key observations:**
- OpenAI's primary API auth is simple Bearer token with API keys
- The Codex CLI device code flow is a separate system for subscription-based access
- For a broker, the relevant pattern is API key management (users provide their `sk-...` key)
- Ephemeral token pattern is interesting for credential delegation: backend holds permanent
  key, issues short-lived tokens to downstream consumers
- Credential storage: Codex stores in `~/.codex/auth.json` or OS keyring

**Broker integration recommendation:** Store user's API key encrypted in MongoDB.
When proxying, decrypt and inject as `Authorization: Bearer {key}`. No OAuth flow
needed -- OpenAI API uses static API keys.

**Source:** https://developers.openai.com/codex/auth/

### 1.2 Anthropic (Claude API)

**Authentication:** API key only (Bearer token: `x-api-key: {key}`)

- No OAuth support, no service accounts, no token exchange
- Keys are generated in the Anthropic Console (console.anthropic.com)
- Keys shown only once at creation time
- GitHub secret scanning integration: exposed keys auto-deactivated
- Best practice: rotate every 60-90 days, environment-specific keys

**Broker integration recommendation:** Same as OpenAI -- encrypted API key storage.
Inject as `x-api-key` header when proxying. Consider adding key rotation reminders
in the UI.

**Source:** https://support.claude.com/en/articles/9767949-api-key-best-practices-keeping-your-keys-safe-and-secure

### 1.3 Google AI (Gemini / Vertex AI)

**Authentication methods:**

| Method | Description | Use Case |
|--------|-------------|----------|
| API Key | Simple key for Google AI Studio | Development, direct API |
| OAuth 2.0 | User-consented access tokens | Apps accessing user data |
| Service Account | JSON key file, no user interaction | CI/CD, server-to-server |
| ADC | Application Default Credentials | GCP-hosted environments |

**Key differences:**
- Google AI Studio (ai.google.dev): supports API keys
- Vertex AI: does NOT support API keys, requires OAuth2 or service account tokens
- OAuth 2.0 tokens expire (typically 1 hour), require refresh flow
- Service account keys are long-lived but carry security risk

**Broker integration recommendation:**
- For Google AI Studio: API key storage (same as OpenAI/Anthropic)
- For Vertex AI: OAuth 2.0 flow with refresh token storage. This requires:
  1. NyxID registers as an OAuth client with Google
  2. User completes consent flow, NyxID stores refresh token
  3. Token refresh on each request or via background task
  4. This is the only LLM provider requiring actual OAuth flows

**Source:** https://ai.google.dev/gemini-api/docs/oauth

### 1.4 Azure OpenAI

**Authentication methods:**

| Method | Description | Use Case |
|--------|-------------|----------|
| API Key | `api-key: {key}` header | Simple deployments |
| Entra ID (OAuth) | Bearer token via client credentials or auth code | Enterprise, RBAC |
| Managed Identity | Automatic token for Azure-hosted workloads | Azure VMs, Functions |

**Key observations:**
- Microsoft recommends Entra ID (token-based) over API keys for production
- Entra ID supports RBAC, conditional access, audit logging
- Starting August 2025: automatic token refresh without separate Azure client dependency
- For non-Azure-hosted brokers, client credentials flow is most practical

**Broker integration recommendation:**
- API key path: same as OpenAI/Anthropic
- Entra ID path: store client_id + client_secret or user's refresh token
  Requires OAuth 2.0 client credentials or authorization code flow

**Source:** https://learn.microsoft.com/en-us/azure/api-management/api-management-authenticate-authorize-ai-apis

### 1.5 Mistral AI

**Authentication:** API key only (`Authorization: Bearer {key}`)

- Keys generated at console.mistral.ai
- Shown only once at creation
- Standard Bearer token auth on all endpoints
- Base URL: https://api.mistral.ai/v1/

**Broker integration:** Same as OpenAI -- encrypted API key storage.

**Source:** https://docs.mistral.ai/api

### 1.6 Cohere

**Authentication:** API key only (`Authorization: Bearer {key}`)

- Keys generated at dashboard.cohere.com
- Trial key auto-created on signup
- Standard Bearer token auth

**Broker integration:** Same as OpenAI -- encrypted API key storage.

**Source:** https://docs.cohere.com/reference/about

### 1.7 Provider Summary

| Provider | Auth Type | Token Expiry | OAuth Required | Broker Complexity |
|----------|-----------|-------------|---------------|-------------------|
| OpenAI | API Key | Never | No | Low |
| Anthropic | API Key | Never | No | Low |
| Google AI Studio | API Key | Never | No | Low |
| Vertex AI | OAuth 2.0 | ~1 hour | Yes | High |
| Azure OpenAI (key) | API Key | Never | No | Low |
| Azure OpenAI (Entra) | OAuth 2.0 | ~1 hour | Yes | High |
| Mistral | API Key | Never | No | Low |
| Cohere | API Key | Never | No | Low |

**Recommendation:** Start with API key support (covers 6/8 scenarios). Add OAuth
flows later for Vertex AI and Azure Entra ID. The existing `connection_service.rs`
already handles API key storage with AES-256-GCM encryption, so the foundation is
solid.

---

## 2. Token Broker Patterns in Rust

### 2.1 Rust OAuth2 Crate (`oauth2` 4.x)

The `oauth2` crate is the standard Rust OAuth2 client library.

**Supported grant types:**
- Authorization Code + PKCE (primary)
- Client Credentials
- Device Authorization Flow (RFC 8628)
- Resource Owner Password Credentials
- Implicit Grant
- Token Introspection (RFC 7662)
- Token Revocation (RFC 7009)

**Extension points for RFC 8693:**
- Custom grant types via `TokenRequest` trait
- Custom token fields via `ExtraTokenFields` trait
- Custom error responses via `ErrorResponse` trait
- Pluggable HTTP clients (reqwest, curl, ureq)

**Key limitation:** No built-in RFC 8693 (Token Exchange) support. Must implement
custom grant type on top of the extensible framework.

**Source:** https://docs.rs/oauth2/latest/oauth2/

### 2.2 Axum Integration Libraries

| Crate | Description | Production Ready |
|-------|-------------|-----------------|
| `oauth-axum` | Wrapper around `oauth2` with preconfigured providers | Simple, in-memory state |
| `oxide-auth-axum` | Full OAuth2 server library for Axum | Yes, configurable backends |
| `oauth2-passkey-axum` | Handlers + middleware + UI for OAuth2+passkey | Opinionated |
| `axum-token-auth` | Token validation middleware | Minimal |

**Recommendation:** Use `oauth2` crate directly (not `oauth-axum`) for maximum
control. NyxID already has its own OAuth server implementation. For new OAuth
*client* flows (Vertex AI, Azure Entra), use `oauth2` crate's `BasicClient` with
custom token storage in MongoDB.

**Source:** https://crates.io/crates/oauth-axum, https://crates.io/crates/oxide-auth-axum

### 2.3 Token Refresh Strategy: Lazy vs Background

#### Option A: Lazy Refresh (Recommended for NyxID)

```
Request arrives -> Check token expiry -> If expired, refresh inline -> Use token
```

**Pros:**
- Simple implementation
- No background tasks or timers
- Only refreshes tokens that are actually used
- Lower resource consumption
- Works naturally with Axum request lifecycle

**Cons:**
- First request after expiry pays refresh latency (~200-500ms)
- Risk of thundering herd if many requests hit expired token simultaneously

**Implementation pattern:**
```rust
async fn get_valid_token(db: &Database, conn_id: &str, key: &[u8]) -> Result<String> {
    let conn = load_connection(db, conn_id).await?;
    if let Some(expires_at) = conn.token_expires_at {
        if expires_at <= Utc::now() + Duration::seconds(30) {
            // Refresh token and update DB
            let new_token = refresh_oauth_token(&conn).await?;
            update_stored_token(db, conn_id, &new_token, key).await?;
            return Ok(new_token.access_token);
        }
    }
    decrypt_token(&conn.credential_encrypted, key)
}
```

#### Option B: Background Refresh

```
Tokio task runs periodically -> Scans expiring tokens -> Refreshes proactively
```

**Pros:**
- Zero latency on request path
- Can handle rate limits gracefully with backoff

**Cons:**
- Complex: needs `tokio::spawn` + interval + graceful shutdown
- Refreshes unused tokens (wasted API calls)
- Distributed systems complexity (needs leader election or per-token locks)
- Must handle DB contention from concurrent refreshes

**Implementation pattern:**
```rust
tokio::spawn(async move {
    let mut interval = tokio::time::interval(Duration::from_secs(300));
    loop {
        interval.tick().await;
        let expiring = find_expiring_tokens(&db, Duration::minutes(10)).await;
        for conn in expiring {
            if let Err(e) = refresh_and_store(&db, &conn, &key).await {
                tracing::warn!("Token refresh failed for {}: {e}", conn.id);
            }
        }
    }
});
```

**Recommendation:** Use lazy refresh for NyxID. Most LLM providers use static API
keys (no expiry), so OAuth refresh only applies to Vertex AI and Azure Entra.
Lazy refresh is simpler and only pays the cost when needed. Add a 30-second buffer
before expiry to avoid edge cases. If specific providers need it, add background
refresh as an opt-in enhancement later.

---

## 3. Identity Propagation Standards

### 3.1 RFC 8693 - OAuth 2.0 Token Exchange

**Purpose:** Exchange one security token for another, supporting delegation and
impersonation across service boundaries.

**Grant type:** `urn:ietf:params:oauth:grant-type:token-exchange`

**Key concepts:**

| Concept | Description |
|---------|-------------|
| Subject Token | Represents the identity being acted upon (the user) |
| Actor Token | Represents the acting party (the service/client) |
| Impersonation | Output token has only subject identity; actor is invisible |
| Delegation | Output token has both subject and actor identity (`act` claim) |

**Request parameters:**
```
POST /token HTTP/1.1
Content-Type: application/x-www-form-urlencoded

grant_type=urn:ietf:params:oauth:grant-type:token-exchange
&subject_token={user_jwt}
&subject_token_type=urn:ietf:params:oauth:token-type:jwt
&actor_token={service_jwt}
&actor_token_type=urn:ietf:params:oauth:token-type:jwt
&scope=openai:chat
&audience=https://api.openai.com
```

**Response:**
```json
{
  "access_token": "...",
  "issued_token_type": "urn:ietf:params:oauth:token-type:access_token",
  "token_type": "Bearer",
  "expires_in": 3600,
  "scope": "openai:chat"
}
```

**Delegation vs Impersonation:**
- Impersonation: only `subject_token`, no `actor_token` -> output token is
  indistinguishable from a regular token for that user
- Delegation: both `subject_token` and `actor_token` -> output token has `act`
  claim showing the chain: `{ "sub": "user123", "act": { "sub": "service456" } }`

**Implementation in NyxID:**
Since NyxID is both the authorization server and the resource broker, a simplified
version of RFC 8693 makes sense:

1. Client authenticates to NyxID (existing JWT auth)
2. Client requests token exchange: "give me a token for OpenAI on behalf of user X"
3. NyxID validates: does user X have an active connection to OpenAI?
4. NyxID returns a short-lived proxy token or directly proxies the request
5. Audit log records the delegation chain

**Recommendation:** Implement a simplified token exchange endpoint that:
- Accepts the user's NyxID JWT as `subject_token`
- Validates connection existence and permissions
- Returns a short-lived (5-15 min) scoped token for the specific provider
- Includes `act` claim for audit trail
- OR: skip token exchange entirely and use the existing proxy pattern with
  identity headers (simpler, already partially implemented)

**Sources:**
- https://datatracker.ietf.org/doc/html/rfc8693
- https://www.scottbrady.io/oauth/delegation-patterns-for-oauth-20
- https://zitadel.com/docs/guides/integrate/token-exchange

### 3.2 Alternative: Identity Headers (Simpler Approach)

For NyxID's proxy model, full RFC 8693 may be overkill. A simpler approach:

```
Client -> NyxID Proxy -> Downstream Service
         [validates JWT]   [injects credential + identity headers]
```

**Identity headers to inject:**
```
X-NyxID-User-Id: {user_uuid}
X-NyxID-User-Email: {user_email}
X-NyxID-Request-Id: {correlation_id}
X-NyxID-Delegation: true
```

This is what the existing `proxy_service.rs` already does conceptually -- it
validates the user's NyxID token, resolves their credential, and forwards the
request. Adding identity headers to the forwarded request completes the identity
propagation story.

**Recommendation:** Start with identity headers (already nearly implemented).
Add RFC 8693 token exchange as a future enhancement for MCP clients that need
to obtain bearer tokens directly.

### 3.3 Signed JWT Assertions (RFC 7523)

For services that trust NyxID as an identity provider, NyxID can issue signed
JWTs asserting user identity. The downstream service validates the JWT signature
against NyxID's JWKS endpoint (already implemented via OIDC discovery).

```json
{
  "iss": "https://nyx.example.com",
  "sub": "user-uuid",
  "aud": "downstream-service-id",
  "exp": 1234567890,
  "act": { "sub": "nyxid-proxy" }
}
```

This works well for provider-category services that already trust NyxID as their
IdP.

---

## 4. MCP Integration Considerations

### 4.1 MCP Authorization Spec (Draft, Nov 2025)

The MCP authorization specification has evolved significantly:

**Key requirements:**
- OAuth 2.1 compliance with PKCE (mandatory)
- MCP servers act as OAuth Resource Servers
- Resource Indicators (RFC 8707) mandatory for token audience binding
- Protected Resource Metadata (RFC 9728) for authorization server discovery
- Client ID Metadata Documents for clientless registration

**Three client registration approaches:**
1. Client ID Metadata Documents (most common for MCP)
2. Pre-registration (existing relationship)
3. Dynamic Client Registration (RFC 7591, backwards compat)

**Scope management:**
- Servers provide `scopes_supported` in protected resource metadata
- Step-up authorization for incremental scope requests
- `WWW-Authenticate` header with scope hints on 401/403

**Source:** https://modelcontextprotocol.io/specification/draft/basic/authorization

### 4.2 NyxID as MCP Authorization Server

NyxID already has OIDC/OAuth infrastructure. To serve as an MCP authorization
server, it needs:

1. **Protected Resource Metadata endpoint** (`/.well-known/oauth-protected-resource`)
   Returns `authorization_servers` pointing to NyxID's own OAuth endpoints

2. **Resource Indicators support** in token issuance
   Include `aud` claim matching the MCP server's canonical URI

3. **Client ID Metadata Document support**
   Fetch and validate HTTPS-hosted client metadata documents

4. **Scope-based tool access control**
   Map NyxID scopes to provider capabilities (e.g., `openai:chat`, `anthropic:complete`)

### 4.3 MCP Token Flow for Provider Access

```
MCP Client -> NyxID (Auth Server) -> Token with provider scopes
MCP Client -> NyxID (MCP Server) -> Uses token to proxy to LLM provider
```

1. MCP client discovers NyxID via Protected Resource Metadata
2. Client authenticates via OAuth 2.1 + PKCE
3. NyxID issues token with scopes like `provider:openai:chat`
4. Client calls NyxID's MCP tools (e.g., `chat_completion`)
5. NyxID resolves the user's stored credential for OpenAI
6. NyxID proxies the request, injecting the credential

**Tool discovery with provider capabilities:**
MCP `tools/list` response should include provider-specific tools only for
providers the user has connected to. Example:

```json
{
  "tools": [
    {
      "name": "openai_chat",
      "description": "Chat completion via OpenAI",
      "inputSchema": { ... }
    }
  ]
}
```

### 4.4 Enterprise-Managed Authorization

The November 2025 MCP spec update introduced "Cross App Access" -- enterprise
IdPs can issue tokens for MCP servers without OAuth redirects. NyxID could
support this by acting as the enterprise IdP, allowing admin-configured access
to MCP servers.

---

## 5. Encrypted Token Storage

### 5.1 Current Implementation (Already Solid)

NyxID already implements application-level field encryption using AES-256-GCM
in `crypto/aes.rs`:

- 256-bit key from hex-encoded environment variable
- Random 96-bit nonce per encryption (prevents IV reuse)
- Nonce prepended to ciphertext for self-contained decryption
- `aes-gcm` 0.10.3 crate (NCC Group audited, constant-time)
- Used by `connection_service.rs` for per-user credentials
- Used by `downstream_service` for master credentials

### 5.2 Recommendations for Enhancement

**Current gaps to address:**

1. **Key rotation support:** Currently a single `ENCRYPTION_KEY` env var. Should
   support multiple keys with a key ID prefix on ciphertexts. When rotating:
   - New encryptions use the new key
   - Old ciphertexts decrypted with old key (identified by prefix)
   - Background migration re-encrypts old ciphertexts

   ```rust
   // Proposed format: key_version (1 byte) || nonce (12 bytes) || ciphertext
   pub fn encrypt_v2(plaintext: &[u8], key: &[u8], key_version: u8) -> Vec<u8> {
       let mut result = Vec::with_capacity(1 + NONCE_SIZE + ciphertext.len());
       result.push(key_version);
       result.extend_from_slice(&nonce_bytes);
       result.extend_from_slice(&ciphertext);
       result
   }
   ```

2. **OAuth token fields:** For OAuth-connected providers (Vertex AI, Azure Entra),
   need to store additional fields beyond the credential:

   ```rust
   pub struct OAuthTokenData {
       pub access_token: String,
       pub refresh_token: Option<String>,
       pub token_type: String,
       pub expires_at: Option<DateTime<Utc>>,
       pub scope: Option<String>,
   }
   ```

   Serialize to JSON, encrypt the entire blob, store in `credential_encrypted`.

3. **Separate refresh token storage:** Refresh tokens are more sensitive than
   access tokens (longer-lived, can generate new access tokens). Consider:
   - Storing refresh tokens in a separate field with an additional encryption layer
   - OR: encrypting with a different key derivation (HKDF to derive per-purpose keys)

4. **Memory protection:** Credential strings in memory should be zeroized after use.
   Consider using the `zeroize` crate for sensitive data:
   ```rust
   use zeroize::Zeroize;
   let mut credential = decrypt_credential(...);
   // ... use credential ...
   credential.zeroize();
   ```

### 5.3 MongoDB-Specific Considerations

- **MongoDB Enterprise encryption at rest:** Complementary to app-level encryption,
  uses AES256-GCM at the storage engine level. NyxID's app-level encryption is
  the right approach since it protects data independent of deployment (works with
  MongoDB Community Edition and Atlas)

- **BSON Binary storage:** The existing pattern of storing `Vec<u8>` as BSON Binary
  with `BinarySubtype::Generic` is correct

- **Index considerations:** Encrypted fields cannot be indexed for queries. The
  existing design correctly uses `user_id` + `service_id` (unencrypted) as query
  fields, with `credential_encrypted` as a data-only field

---

## 6. Architectural Recommendations Summary

### 6.1 Phase 1: API Key Broker (Low Effort, High Value)

Extend the existing `connection_service.rs` + `proxy_service.rs`:

1. Add provider-specific configuration (auth header names, base URLs)
2. Add identity propagation headers to proxied requests
3. Add credential validation on connect (test API key against provider)
4. Add key rotation reminders (last_rotated_at timestamp)

**Covers:** OpenAI, Anthropic, Google AI Studio, Mistral, Cohere, Azure (key mode)

### 6.2 Phase 2: OAuth Provider Connections (Medium Effort)

Add OAuth 2.0 client flows for providers that require them:

1. Implement OAuth authorization code + PKCE flow using `oauth2` crate
2. Store access + refresh tokens in encrypted format
3. Lazy token refresh on proxy requests
4. Provider-specific OAuth client registration (Google, Azure)

**Covers:** Vertex AI, Azure Entra ID

### 6.3 Phase 3: MCP Authorization (Medium-High Effort)

Enhance NyxID's OAuth server to comply with MCP authorization spec:

1. Add Protected Resource Metadata endpoint
2. Add Resource Indicators (RFC 8707) to token issuance
3. Add Client ID Metadata Document support
4. Add scope-based tool access control
5. Dynamic tool listing based on user's connected providers

### 6.4 Phase 4: Token Exchange (Future)

Implement RFC 8693 for advanced delegation scenarios:

1. Token exchange endpoint with subject/actor token support
2. Short-lived scoped tokens for specific providers
3. Delegation chain in `act` claims for audit

### 6.5 Crate Dependencies

| Crate | Version | Purpose |
|-------|---------|---------|
| `oauth2` | 4.x | OAuth 2.0 client flows (Phase 2) |
| `aes-gcm` | 0.10.x | Already in use for encryption |
| `zeroize` | 1.x | Memory protection for credentials |
| `reqwest` | 0.12.x | Already in use for HTTP client |
| `jsonwebtoken` | 9.x | Already in use for JWT |

No new major dependencies needed for Phase 1. Phase 2 adds `oauth2`.

---

## 7. Security Considerations

1. **Credential isolation:** Each user's credentials are encrypted with a shared
   application key. Consider per-user key derivation (HKDF with user_id as info)
   for defense-in-depth.

2. **Audit trail:** All credential access (decrypt, proxy, refresh) should be logged
   via `audit_service`. The existing audit infrastructure supports this.

3. **Rate limiting:** Proxy requests should be rate-limited per-user per-service
   to prevent abuse. The existing `governor` middleware can be extended.

4. **Credential validation:** On connect, validate the credential against the
   provider's API (e.g., call a lightweight endpoint like OpenAI's `/models`).

5. **Token theft mitigation:** Short-lived proxy tokens, credential never exposed
   in API responses, encrypted at rest, zeroized in memory after use.

6. **SSRF prevention:** The existing `proxy_service.rs` has a TODO for DNS
   rebinding prevention. This should be addressed before adding more proxy targets.
