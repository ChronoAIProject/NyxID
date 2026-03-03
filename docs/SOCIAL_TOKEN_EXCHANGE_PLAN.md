# Implementation Plan: Social Token Exchange (RFC 8693 Extension)

## Overview

Extend NyxID's existing RFC 8693 Token Exchange endpoint (`POST /oauth/token`) to accept
external provider ID tokens (Google, GitHub) as `subject_token`. This enables mobile apps
using native SDKs (Google Sign-In, GitHub OAuth) to exchange provider-issued tokens for
full NyxID token sets (access + refresh + ID token) without browser redirects.

The existing delegated token exchange flow (NyxID access token -> delegated token) remains
completely unchanged. The new flow is distinguished by `provider` plus provider-specific
`subject_token_type`:
- `provider=google` -> `urn:ietf:params:oauth:token-type:id_token`
- `provider=github` -> `urn:ietf:params:oauth:token-type:access_token`

## Requirements

- Accept Google ID tokens (JWT, RS256, JWKS-verified) as subject_token
- Accept GitHub access tokens (opaque, verified via GitHub API) as subject_token
- Issue full NyxID token sets (access_token, refresh_token, id_token) after user match/creation
- Reuse existing `find_or_create_user` logic from `social_auth_service`
- No new API routes -- extend existing `POST /oauth/token` handler
- No new env vars required -- reuses existing `GOOGLE_CLIENT_ID` / `GITHUB_CLIENT_ID`
- Backward compatible -- existing delegated token exchange is unaffected

## API Contract

### Request

```
POST /oauth/token
Content-Type: application/x-www-form-urlencoded

grant_type=urn:ietf:params:oauth:grant-type:token-exchange
&subject_token={google_id_token_or_github_access_token}
&subject_token_type={urn:ietf:params:oauth:token-type:id_token | urn:ietf:params:oauth:token-type:access_token}
&client_id={nyxid_oauth_client_id}
&client_secret={nyxid_oauth_client_secret}  (required for confidential clients)
&provider=google                             (required: "google" or "github")
```

### Response (Success)

```json
{
  "access_token": "eyJ...",
  "token_type": "Bearer",
  "expires_in": 900,
  "refresh_token": "eyJ...",
  "id_token": "eyJ...",
  "scope": "openid profile email",
  "issued_token_type": "urn:ietf:params:oauth:token-type:access_token"
}
```

### Response (Error)

Standard RFC 6749 Section 5.2 error response (same format as existing token endpoint errors):

```json
{
  "error": "invalid_grant",
  "error_description": "External ID token validation failed: ..."
}
```

### Parameter Details

| Parameter | Required | Description |
|---|---|---|
| `grant_type` | Yes | Must be `urn:ietf:params:oauth:grant-type:token-exchange` |
| `subject_token` | Yes | The external provider token (Google JWT or GitHub access token) |
| `subject_token_type` | Yes | Google: `urn:ietf:params:oauth:token-type:id_token`; GitHub: `urn:ietf:params:oauth:token-type:access_token` |
| `client_id` | Yes | NyxID OAuth client ID |
| `client_secret` | Conditional | Required for confidential clients |
| `provider` | Yes | Provider hint: `"google"` or `"github"` |

### Why `provider` is Required

While Google ID tokens contain an `iss` claim that could be auto-detected, GitHub tokens
are opaque (no parseable issuer). Requiring `provider` keeps the logic simple, avoids
security risks from issuer guessing, and is consistent across providers.

## Data Flow

```
Mobile App                           NyxID                          Google/GitHub
    |                                  |                                |
    |  1. Native SDK login             |                                |
    |  (Google Sign-In / GitHub)       |                                |
    |--------------------------------->|                                |
    |  id_token / access_token         |                                |
    |                                  |                                |
    |  2. POST /oauth/token            |                                |
    |  grant_type=token-exchange       |                                |
    |  subject_token=<provider_token>  |                                |
    |  subject_token_type=id_token     | (Google)                       |
    |  subject_token_type=access_token | (GitHub)                       |
    |  provider=google                 |                                |
    |  client_id=<nyxid_client>        |                                |
    |--------------------------------->|                                |
    |                                  |  3. Verify token               |
    |                                  |  Google: JWKS signature check  |
    |                                  |  GitHub: app-bound token check +
    |                                  |  GET /user + /user/emails
    |                                  |-------------------------------->|
    |                                  |  4. Validated claims / profile  |
    |                                  |<-------------------------------|
    |                                  |                                |
    |                                  |  5. find_or_create_user()      |
    |                                  |  (reuse social_auth_service)   |
    |                                  |                                |
    |                                  |  6. Issue NyxID tokens         |
    |                                  |  (access + refresh + id_token) |
    |                                  |                                |
    |  7. Token response               |                                |
    |<---------------------------------|                                |
```

### Step-by-step Internal Flow

1. **Handler receives request** (`handlers/oauth.rs` -- `token_handler`)
   - Matches `grant_type = "urn:ietf:params:oauth:grant-type:token-exchange"`
   - Checks `provider` + `subject_token_type`:
     - `provider=google` requires `"urn:ietf:params:oauth:token-type:id_token"`
     - `provider=github` requires `"urn:ietf:params:oauth:token-type:access_token"`
     - `provider` omitted + `"urn:ietf:params:oauth:token-type:access_token"` -> existing delegated flow (unchanged)
   - Extracts new `provider` field from `TokenRequest`

2. **Client authentication** (`oauth_service::authenticate_client`)
   - Validates `client_id` exists, is active
   - For confidential clients, verifies `client_secret`
   - For public clients (mobile apps), allows no secret (already supported)

3. **Provider token verification** (`services/social_token_exchange_service.rs`)
   - **Google**: Fetch JWKS from `https://www.googleapis.com/oauth2/v3/certs`, verify
     RS256 signature, validate `iss`, `aud` (matches `GOOGLE_CLIENT_ID`), `exp`, `email_verified`
   - **GitHub**: Verify app binding via
     `POST https://api.github.com/applications/{client_id}/token`, then call
     `GET https://api.github.com/user` + `GET https://api.github.com/user/emails`
     with the access token (reuse `fetch_user_profile` from `social_auth_service`)

4. **Build SocialProfile** -- normalize claims into the existing `SocialProfile` struct

5. **User resolution** (`social_auth_service::find_or_create_user`)
   - Same 3-case logic: returning social user, email linking, new user creation

6. **Token issuance** -- generate full NyxID token set via existing `token_service` /
   `jwt` functions (access token, refresh token, optional ID token)

7. **Audit logging** -- log `social_token_exchange` event with provider and client_id

## Architecture Changes

### New Files

| File | Responsibility |
|---|---|
| `backend/src/services/social_token_exchange_service.rs` | Core orchestration: verify external token, build SocialProfile, delegate to find_or_create_user, issue NyxID tokens |
| `backend/src/crypto/jwks.rs` | JWKS fetcher + cache: fetch remote JWKS endpoints, parse JWK sets, build `DecodingKey`, cache with TTL |

### Modified Files

| File | Changes |
|---|---|
| `backend/src/handlers/oauth.rs` | Add `provider` field to `TokenRequest`; route social exchange by provider + subject_token_type while keeping delegated flow unchanged |
| `backend/src/services/mod.rs` | Add `pub mod social_token_exchange_service;` |
| `backend/src/crypto/mod.rs` | Add `pub mod jwks;` |
| `backend/src/errors/mod.rs` | Add new error variants for external token verification failures |
| `backend/src/main.rs` | Add `JwksCache` to `AppState` (initialized at startup, shared via `Arc`) |

### Unchanged Files

| File | Why Unchanged |
|---|---|
| `backend/src/services/token_exchange_service.rs` | Existing delegated flow is completely separate; no modifications needed |
| `backend/src/services/social_auth_service.rs` | Reused as-is (`find_or_create_user`, `fetch_user_profile`, `SocialProfile`, `SocialProvider`) |
| `backend/src/crypto/jwt.rs` | All needed JWT generation functions already exist |
| `backend/src/services/token_service.rs` | `create_session_and_issue_tokens` reused as-is for issuing the full token set |
| `backend/src/config.rs` | No new env vars needed (reuses existing Google/GitHub client IDs) |
| `backend/src/routes.rs` | No new routes needed |

## Implementation Steps

### Phase 1: JWKS Infrastructure (`crypto/jwks.rs`)

**1.1. Create JWKS cache struct** (File: `backend/src/crypto/jwks.rs`)
- Action: Create `JwksCache` struct with `tokio::sync::RwLock<HashMap<String, CachedJwks>>`
- `CachedJwks` holds: `Vec<DecodingKey>` + `fetched_at: Instant` + `max_age: Duration`
- Default cache TTL: 1 hour (Google rotates keys roughly every 6 hours)
- The cache is keyed by JWKS URI string
- Dependencies: None
- Risk: Low
- Complexity: Medium

```rust
pub struct JwksCache {
    inner: tokio::sync::RwLock<HashMap<String, CachedJwks>>,
    http_client: reqwest::Client,
}

struct CachedJwks {
    keys: Vec<CachedKey>,
    fetched_at: std::time::Instant,
    max_age: std::time::Duration,
}

struct CachedKey {
    kid: Option<String>,
    decoding_key: DecodingKey,
    algorithm: Algorithm,
}
```

**1.2. Implement JWKS fetching** (File: `backend/src/crypto/jwks.rs`)
- Action: Implement `JwksCache::get_keys(jwks_uri)` method
  - Check cache first (RwLock read)
  - If stale or missing, fetch JWKS JSON via HTTP, parse JWK set, build `DecodingKey` for each key
  - Store in cache (RwLock write)
  - Parse `Cache-Control: max-age=N` header to set per-entry TTL (capped at 24h, floor at 5min)
- Security: Only fetch from hardcoded allowlisted URIs (Google JWKS URL)
- Dependencies: 1.1
- Risk: Medium (network dependency, parsing external JSON)
- Complexity: Medium

**1.3. Implement ID token verification** (File: `backend/src/crypto/jwks.rs`)
- Action: Implement `JwksCache::verify_google_id_token(token, expected_audience)` method
  - Decode JWT header to get `kid`
  - Fetch JWKS for Google endpoint
  - Find matching key by `kid`
  - Verify signature (RS256), `iss` (must be `https://accounts.google.com`), `aud` (must match
    `GOOGLE_CLIENT_ID`), `exp` (not expired)
  - Return parsed claims (`sub`, `email`, `email_verified`, `name`, `picture`)
- Dependencies: 1.2
- Risk: Medium (security-critical path)
- Complexity: Medium

**1.4. Register module** (File: `backend/src/crypto/mod.rs`)
- Action: Add `pub mod jwks;`
- Dependencies: 1.1
- Risk: Low

**1.5. Add JwksCache to AppState** (File: `backend/src/main.rs`)
- Action: Add `pub jwks_cache: Arc<JwksCache>` field to `AppState`
- Initialize in `main()` with `JwksCache::new(http_client.clone())`
- Dependencies: 1.1
- Risk: Low

### Phase 2: Social Token Exchange Service (`services/social_token_exchange_service.rs`)

**2.1. Define response struct** (File: `backend/src/services/social_token_exchange_service.rs`)
- Action: Create `SocialTokenExchangeResponse` struct containing: `access_token`, `refresh_token`,
  `id_token` (optional), `expires_in`, `scope`, `user_id`
- Dependencies: None
- Risk: Low

**2.2. Implement `exchange_social_token` function** (File: `backend/src/services/social_token_exchange_service.rs`)
- Action: Create the main orchestration function:
  ```rust
  pub async fn exchange_social_token(
      db: &mongodb::Database,
      config: &AppConfig,
      jwt_keys: &JwtKeys,
      jwks_cache: &JwksCache,
      http_client: &reqwest::Client,
      client_id: &str,
      client_secret: Option<&str>,
      subject_token: &str,
      provider: &str,
  ) -> AppResult<SocialTokenExchangeResponse>
  ```
- Flow:
  1. Authenticate client via `oauth_service::authenticate_client(db, client_id, client_secret)`
  2. Parse provider via `SocialProvider::parse(provider)` -- error if unsupported
  3. Verify token based on provider:
     - **Google**: `jwks_cache.verify_google_id_token(subject_token, config.google_client_id)`
       -> Build `SocialProfile` from JWT claims
     - **GitHub**: `social_auth_service::fetch_user_profile(SocialProvider::GitHub, subject_token, http_client)`
       -> Already returns `SocialProfile`
  4. Call `social_auth_service::find_or_create_user(db, &profile)`
  5. Call `token_service::create_session_and_issue_tokens(db, config, jwt_keys, &user.id, None, None)`
  6. Generate ID token via `jwt::generate_id_token(...)` with user data and `client_id` as audience
  7. Audit log: fire-and-forget `audit_service::log_async` with event `social_token_exchange`
  8. Return response struct
- Dependencies: Phase 1 (JWKS cache for Google), social_auth_service (for GitHub + user matching)
- Risk: Medium
- Complexity: Medium

**2.3. Handle Google-specific verification** (File: `backend/src/services/social_token_exchange_service.rs`)
- Action: Create `verify_google_token` helper that:
  1. Calls `jwks_cache.verify_google_id_token()`
  2. Validates `email_verified == true` (reject unverified emails)
  3. Builds and returns `SocialProfile` from the claims
- Dependencies: 2.2
- Risk: Medium (security-critical)

**2.4. Register module** (File: `backend/src/services/mod.rs`)
- Action: Add `pub mod social_token_exchange_service;`
- Dependencies: 2.1
- Risk: Low

### Phase 3: Handler Integration

**3.1. Add `provider` field to `TokenRequest`** (File: `backend/src/handlers/oauth.rs`)
- Action: Add `pub provider: Option<String>` field to `TokenRequest` struct
- This is backward-compatible (existing requests don't send `provider`)
- Dependencies: None
- Risk: Low

**3.2. Branch token exchange by `provider` + `subject_token_type`** (File: `backend/src/handlers/oauth.rs`)
- Action: In the `"urn:ietf:params:oauth:grant-type:token-exchange"` match arm of `token_handler`:
  - If `provider=google` and `subject_token_type == "urn:ietf:params:oauth:token-type:id_token"`:
    - Call `social_token_exchange_service::exchange_social_token(...)`
    - Return `TokenResponse` with full token set (access, refresh, id_token)
  - If `provider=github` and `subject_token_type == "urn:ietf:params:oauth:token-type:access_token"`:
    - Call `social_token_exchange_service::exchange_social_token(...)`
    - Return `TokenResponse` with full token set (access, refresh, id_token)
  - If `provider` is omitted and `subject_token_type == "urn:ietf:params:oauth:token-type:access_token"`:
    - Existing delegated flow (unchanged)
- Changes are additive; existing code path is untouched
- Dependencies: Phase 2
- Risk: Low

### Phase 4: Error Handling

**4.1. Add error variants** (File: `backend/src/errors/mod.rs`)
- Action: Add two new `AppError` variants:

```rust
#[error("External token verification failed: {0}")]
ExternalTokenInvalid(String),

#[error("External provider not configured: {0}")]
ExternalProviderNotConfigured(String),
```

- `ExternalTokenInvalid` -> HTTP 400, error_code 6004, oauth_error_code "invalid_grant"
- `ExternalProviderNotConfigured` -> HTTP 400, error_code 6005, oauth_error_code "invalid_request"
- Update all match arms: `status_code()`, `error_code()`, `oauth_error_code()`, `oauth_status()`,
  `error_key()`, and the `IntoResponse` impl
- Dependencies: None
- Risk: Low
- Complexity: Low

### Phase 5: Testing

**5.1. Unit tests for JWKS cache** (File: `backend/src/crypto/jwks.rs`)
- Test JWK JSON parsing (valid RS256 key)
- Test cache hit / cache miss behavior
- Test stale cache refresh
- Test rejection of unsupported key types
- Dependencies: Phase 1

**5.2. Unit tests for social token exchange** (File: `backend/src/services/social_token_exchange_service.rs`)
- Test provider parsing validation
- Test missing provider error
- Test that Google path requires GOOGLE_CLIENT_ID
- Test that GitHub path requires valid access token
- Dependencies: Phase 2

**5.3. Unit tests for handler routing** (File: `backend/src/handlers/oauth.rs`)
- Verify `TokenRequest` deserializes `provider` field correctly
- Test that `provider` field is optional (backward compatible)
- Dependencies: Phase 3

**5.4. Integration tests** (if test infrastructure exists)
- Full flow: Google ID token -> NyxID token set
- Full flow: GitHub access token -> NyxID token set
- Error case: invalid/expired provider token
- Error case: missing provider parameter
- Error case: unconfigured provider
- Backward compat: existing delegated flow still works

## Security Model

### JWKS Caching Strategy

| Concern | Mitigation |
|---|---|
| Cache poisoning | JWKS URIs are hardcoded constants, never derived from user input |
| Stale keys | TTL-based expiry (default 1h); respects `Cache-Control: max-age` from provider |
| Key rotation | On verification failure with cached keys, force one re-fetch before failing |
| Memory exhaustion | Cache is keyed by provider (max 2 entries: Google); cap at reasonable size |
| HTTPS only | JWKS fetch uses HTTPS (reqwest with rustls-tls) |

### Issuer Trust Model

Hardcoded allowlist -- no dynamic issuer discovery:

```rust
const GOOGLE_JWKS_URI: &str = "https://www.googleapis.com/oauth2/v3/certs";
const GOOGLE_ISSUER: &str = "https://accounts.google.com";
```

GitHub does not issue JWTs, so there is no JWKS URI for GitHub. GitHub tokens are
verified as app-bound to NyxID's configured OAuth app, then profile data is fetched
via GitHub APIs (same core profile logic as existing social auth).

### Token Verification Checklist

**Google ID Token:**
- [ ] Signature verified against Google JWKS (RS256)
- [ ] `iss` == `"https://accounts.google.com"` (exact match)
- [ ] `aud` == NyxID's configured `GOOGLE_CLIENT_ID` (exact match)
- [ ] `exp` > current time (not expired)
- [ ] `email_verified` == `true` (reject unverified)
- [ ] `sub` is present and non-empty
- [ ] `alg` header == RS256 (reject `none`, HS256, etc.)

**GitHub Access Token:**
- [ ] Token is verified against NyxID's configured OAuth app via
      `POST /applications/{client_id}/token`
- [ ] `GET /user` returns 200 (token is valid)
- [ ] Primary verified email found via `GET /user/emails`
- [ ] `provider_id` (GitHub user ID) is present

### Anti-Replay

- Google ID tokens have `exp` claim; the `jsonwebtoken` crate validates this automatically
- Google tokens also have `iat` claim; we reject tokens older than 10 minutes (`iat` check)
- GitHub tokens are app-bound checked in real-time via API calls, so replay is limited by token validity
- NyxID refresh tokens use rotation with reuse detection (existing infrastructure)

### Rate Limiting

- Existing rate limiter (`mw/rate_limit.rs`) applies to `POST /oauth/token`
- No additional rate limiting needed beyond what exists
- Consider monitoring for high volume of failed external token validations (abuse indicator)

### Audit Logging

Every social token exchange (success or failure) is logged via `audit_service::log_async`:

```json
{
  "event_type": "social_token_exchange",
  "event_data": {
    "provider": "google",
    "client_id": "...",
    "result": "success"
  }
}
```

Failed attempts also logged:

```json
{
  "event_type": "social_token_exchange_failed",
  "event_data": {
    "provider": "google",
    "client_id": "...",
    "reason": "token_expired"
  }
}
```

## Configuration

### No New Environment Variables Required

The feature reuses existing configuration:

| Config Field | Used For |
|---|---|
| `google_client_id` | Audience validation for Google ID tokens |
| `google_client_secret` | Not used (ID token verification is public-key based) |
| `github_client_id` | Used for app-bound GitHub token verification in social token exchange |
| `github_client_secret` | Used for app-bound GitHub token verification in social token exchange |

### JWKS Cache Configuration (Hardcoded Defaults)

These are hardcoded constants, not env vars (to minimize attack surface):

```rust
const JWKS_DEFAULT_TTL_SECS: u64 = 3600;   // 1 hour
const JWKS_MIN_TTL_SECS: u64 = 300;        // 5 minutes (floor)
const JWKS_MAX_TTL_SECS: u64 = 86400;      // 24 hours (cap)
const GOOGLE_ID_TOKEN_MAX_AGE_SECS: i64 = 600; // 10 minutes (iat check)
```

## Error Handling

### New Error Variants

| Variant | HTTP Status | Error Code | OAuth Error | When |
|---|---|---|---|---|
| `ExternalTokenInvalid(String)` | 400 | 6004 | `invalid_grant` | External token signature, expiry, audience, or claims validation failed |
| `ExternalProviderNotConfigured(String)` | 400 | 6005 | `invalid_request` | Provider hint missing or provider not configured (e.g., no `GOOGLE_CLIENT_ID`) |

### Existing Error Variants Reused

| Variant | When |
|---|---|
| `BadRequest` | Missing required parameters (subject_token, subject_token_type, provider) |
| `Unauthorized` | Invalid client credentials |
| `SocialAuthFailed` | GitHub API call failure |
| `SocialAuthNoEmail` | No verified email from provider |
| `SocialAuthDeactivated` | User account is deactivated |
| `SocialAuthConflict` | Email already linked to another provider |

### Error Response Example

```json
{
  "error": "invalid_grant",
  "error_description": "External token verification failed: token has expired"
}
```

## Backward Compatibility

### Routing Logic

The branching point uses `provider` plus `subject_token_type` within the existing token exchange grant:

```
grant_type = "urn:ietf:params:oauth:grant-type:token-exchange"
  |
  |-- provider omitted + subject_token_type = "urn:ietf:params:oauth:token-type:access_token"
  |   -> Existing delegated flow (token_exchange_service::exchange_token)
  |   -> UNCHANGED
  |
  |-- provider = "google" + subject_token_type = "urn:ietf:params:oauth:token-type:id_token"
  |   -> New social exchange flow (social_token_exchange_service::exchange_social_token)
  |   -> NEW
  |
  |-- provider = "github" + subject_token_type = "urn:ietf:params:oauth:token-type:access_token"
  |   -> New social exchange flow (social_token_exchange_service::exchange_social_token)
  |   -> NEW
  |
  |-- anything else
      -> BadRequest("Unsupported subject_token_type for provider")
      -> UNCHANGED (already returns error)
```

### What Does NOT Change

- Existing delegated token exchange (NyxID access token -> delegated token)
- OAuth authorization code flow
- Client credentials flow
- Refresh token flow
- All existing request/response shapes
- All existing error codes and status codes
- `TokenRequest` struct gains optional `provider` field (additive, backward-compatible)

### What Changes

- `TokenRequest` struct: new optional `provider: Option<String>` field
- `token_handler` in `oauth.rs`: new provider-aware branch for social token exchange
- `AppState`: new `jwks_cache` field
- `AppError`: two new variants (additive to existing error code space)

## User Matching Strategy

Delegates entirely to `social_auth_service::find_or_create_user`, which implements:

### Case 1: Returning Social User
- Query: `social_provider == provider AND social_provider_id == sub`
- If found and active: update `last_login_at`, return user
- If found and inactive: return `SocialAuthDeactivated` error

### Case 2: Email Linking (Account Linking)
- Query: `email == provider_email` (case-insensitive)
- If found with no social provider: link social identity, mark email verified
- If found with different social provider: return `SocialAuthConflict` error
- If found and inactive: return `SocialAuthDeactivated` error

### Case 3: New User Creation
- Create new user with UUID v4 ID
- Set `email_verified = true` (trusted from provider)
- Set `social_provider` and `social_provider_id`
- Return newly created user

This reuse is intentional -- it ensures web-based social login and mobile token exchange
produce identical user records and follow the same linking rules.

## Risks and Mitigations

### Risk 1: JWKS Fetch Failure (Google Down)
- **Impact**: Google token exchange unavailable
- **Mitigation**: Cache serves stale keys for up to 24h; on verification failure with
  cached keys, force one re-fetch before returning error; log clearly
- **Severity**: Medium (temporary, self-healing)

### Risk 2: Provider Token Replay
- **Impact**: Attacker replays a captured provider token
- **Mitigation**: `exp` validation + `iat` freshness check (max 10 min for Google);
  GitHub tokens verified in real-time; NyxID issues new session each time
- **Severity**: Low (tokens are short-lived)

### Risk 3: Email-Based Account Takeover
- **Impact**: Attacker with unverified email gains access to existing account
- **Mitigation**: Google `email_verified == true` required; GitHub only uses verified
  emails from `/user/emails`; same policy as existing social auth
- **Severity**: Low (already mitigated in existing code)

### Risk 4: Public Client Abuse
- **Impact**: Attacker sends stolen provider tokens with a valid public client_id
- **Mitigation**: Rate limiting on token endpoint; provider tokens are short-lived;
  audit logging enables detection; this is an accepted trade-off for public mobile clients
  (same model as Firebase Auth, Auth0)
- **Severity**: Low

### Risk 5: `alg: none` or HMAC Substitution Attack
- **Impact**: Attacker crafts unsigned JWT accepted as valid Google token
- **Mitigation**: `jsonwebtoken` crate's `Validation` struct requires explicit algorithm
  (RS256); `none` and HMAC algorithms are rejected by default
- **Severity**: Low (framework-level protection)

## Success Criteria

- [ ] Google ID tokens are verified via JWKS and produce NyxID token sets
- [ ] GitHub access tokens are app-bound verified and produce NyxID token sets
- [ ] Existing delegated token exchange continues to work unchanged
- [ ] JWKS keys are cached with appropriate TTL
- [ ] Invalid/expired tokens return clear error messages
- [ ] User matching follows same rules as web social auth
- [ ] All new code paths have unit tests
- [ ] No new security vulnerabilities introduced
- [ ] No new env vars required (zero-config for existing deployments)
