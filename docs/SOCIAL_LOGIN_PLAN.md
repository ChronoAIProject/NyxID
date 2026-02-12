# Social Login Technical Spec: GitHub + Google OAuth

## Overview

Implement OAuth2 Authorization Code flow for GitHub and Google social login.
The frontend already has social login buttons that navigate to `/api/v1/auth/social/{provider}`,
which currently 404s. This spec details the full backend implementation and
the frontend cleanup (remove Apple, fix grid).

---

## 1. OAuth Flow Sequence

```
Browser                     NyxID Backend                 Provider (GitHub/Google)
  |                              |                              |
  |-- GET /auth/social/github -->|                              |
  |                              |-- generate state token       |
  |                              |-- set state cookie           |
  |<-- 302 Redirect ------------|                              |
  |                              |                              |
  |-- GET github.com/login/oauth/authorize ------------------>|
  |                              |                              |
  |<-- 302 callback?code=X&state=Y ----------------------------|
  |                              |                              |
  |-- GET /auth/social/github/callback?code=X&state=Y ------->|
  |                              |-- validate state vs cookie   |
  |                              |-- POST token URL (exchange)  |
  |                              |-- GET user profile API       |
  |                              |-- find_or_create_user        |
  |                              |-- create_session_and_issue_tokens
  |                              |-- set auth cookies           |
  |                              |-- clear state cookie         |
  |<-- 302 Redirect to frontend/dashboard --------------------|
```

---

## 2. User Model Changes

**File:** `backend/src/models/user.rs`

Add two optional fields to the `User` struct:

```rust
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    // ... existing fields ...

    /// Social login provider name: "github" | "google"
    #[serde(default)]
    pub social_provider: Option<String>,

    /// Provider's unique user ID (e.g. GitHub numeric ID, Google sub claim)
    #[serde(default)]
    pub social_provider_id: Option<String>,
}
```

**Rationale:**
- `#[serde(default)]` ensures backward compatibility when reading existing users that lack these fields.
- `password_hash` is already `Option<String>`, so social-only users naturally have `password_hash: None`.
- No `#[serde(skip_serializing)]` -- critical for MongoDB `insert_one` (per project conventions).

**Update `make_user()` in tests** to include:
```rust
social_provider: None,
social_provider_id: None,
```

**Update `auth_service::register_user`** constructor to include:
```rust
social_provider: None,
social_provider_id: None,
```

---

## 3. Database Index

**File:** `backend/src/db.rs`

Add a unique sparse compound index on the `users` collection after the existing user indexes:

```rust
// Social login lookup: find user by (provider, provider_id)
users
    .create_index(
        IndexModel::builder()
            .keys(doc! { "social_provider": 1, "social_provider_id": 1 })
            .options(
                IndexOptions::builder()
                    .unique(true)
                    .sparse(true) // Only index documents where both fields exist
                    .build(),
            )
            .build(),
    )
    .await?;
```

**Why sparse:** Most existing users won't have these fields. A sparse unique index
only includes documents where the indexed fields are non-null, preventing false
uniqueness conflicts from `null, null` pairs.

---

## 4. New Error Variant

**File:** `backend/src/errors/mod.rs`

Add a new variant for social auth failures:

```rust
#[error("Social authentication failed: {0}")]
SocialAuthFailed(String),
```

Mappings:
- `status_code`: `StatusCode::BAD_REQUEST` (400)
- `error_code`: `6000`
- `error_key`: `"social_auth_failed"`

This variant covers: unsupported provider, state mismatch, code exchange failure,
profile fetch failure, and provider not configured.

---

## 5. Social Auth Service

**File (new):** `backend/src/services/social_auth_service.rs`

### 5.1 Provider Enum

```rust
use serde::Deserialize;

/// Supported social login providers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SocialProvider {
    GitHub,
    Google,
}

impl SocialProvider {
    /// Parse from URL path segment. Returns None for unsupported providers.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "github" => Some(Self::GitHub),
            "google" => Some(Self::Google),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::GitHub => "github",
            Self::Google => "google",
        }
    }
}
```

### 5.2 Provider Profile

```rust
/// Normalized user profile from a social provider.
pub struct SocialProfile {
    pub provider: SocialProvider,
    pub provider_id: String,
    pub email: String,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}
```

### 5.3 build_authorization_url

```rust
pub fn build_authorization_url(
    provider: SocialProvider,
    state: &str,
    config: &AppConfig,
) -> AppResult<String>
```

**GitHub:**
```
https://github.com/login/oauth/authorize
  ?client_id={GITHUB_CLIENT_ID}
  &redirect_uri={BASE_URL}/api/v1/auth/social/github/callback
  &scope=user:email
  &state={state}
```

**Google:**
```
https://accounts.google.com/o/oauth2/v2/auth
  ?client_id={GOOGLE_CLIENT_ID}
  &redirect_uri={BASE_URL}/api/v1/auth/social/google/callback
  &scope=openid+email+profile
  &state={state}
  &response_type=code
  &access_type=online
```

**Error cases:**
- Provider not configured (missing client_id or client_secret) -> `AppError::SocialAuthFailed("Provider not configured")`

### 5.4 exchange_code

```rust
pub async fn exchange_code(
    provider: SocialProvider,
    code: &str,
    config: &AppConfig,
    http_client: &reqwest::Client,
) -> AppResult<String>
```

Returns the access token string.

**GitHub token exchange:**
- `POST https://github.com/login/oauth/access_token`
- Headers: `Accept: application/json`
- Body (form-encoded): `client_id`, `client_secret`, `code`, `redirect_uri`
- Response: `{ "access_token": "gho_...", "token_type": "bearer", "scope": "user:email" }`

```rust
#[derive(Deserialize)]
struct GitHubTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}
```

**Google token exchange:**
- `POST https://oauth2.googleapis.com/token`
- Body (form-encoded): `client_id`, `client_secret`, `code`, `redirect_uri`, `grant_type=authorization_code`
- Response: `{ "access_token": "ya29...", "token_type": "Bearer", "expires_in": 3599, "id_token": "..." }`

```rust
#[derive(Deserialize)]
struct GoogleTokenResponse {
    access_token: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}
```

**Error cases:**
- HTTP request failure -> `AppError::SocialAuthFailed("Failed to exchange code with {provider}")`
- Response contains `error` field -> `AppError::SocialAuthFailed(error_description)`
- Missing `access_token` -> `AppError::SocialAuthFailed("No access token in response")`

### 5.5 fetch_user_profile

```rust
pub async fn fetch_user_profile(
    provider: SocialProvider,
    access_token: &str,
    http_client: &reqwest::Client,
) -> AppResult<SocialProfile>
```

**GitHub profile fetch (2 requests):**

1. `GET https://api.github.com/user` with `Authorization: Bearer {token}`, `User-Agent: NyxID`
   ```rust
   #[derive(Deserialize)]
   struct GitHubUser {
       id: u64,
       login: String,
       name: Option<String>,
       avatar_url: Option<String>,
       email: Option<String>, // May be null if email is private
   }
   ```

2. If `email` is `None`, fetch `GET https://api.github.com/user/emails`
   ```rust
   #[derive(Deserialize)]
   struct GitHubEmail {
       email: String,
       primary: bool,
       verified: bool,
   }
   ```
   Select the first email where `primary == true && verified == true`.
   If none found, try first `verified == true`.
   If still none, return `AppError::SocialAuthFailed("No verified email found on GitHub account")`.

**Google profile fetch (1 request):**
- `GET https://www.googleapis.com/oauth2/v3/userinfo` with `Authorization: Bearer {token}`
  ```rust
  #[derive(Deserialize)]
  struct GoogleUserInfo {
      sub: String,        // Unique Google user ID
      email: String,
      email_verified: Option<bool>,
      name: Option<String>,
      picture: Option<String>,
  }
  ```
- If `email_verified` is `Some(false)`, return `AppError::SocialAuthFailed("Google email not verified")`

**Error cases:**
- HTTP request failure -> `AppError::SocialAuthFailed("Failed to fetch profile from {provider}")`
- Deserialization failure -> `AppError::SocialAuthFailed("Invalid profile response from {provider}")`

### 5.6 find_or_create_user

```rust
pub async fn find_or_create_user(
    db: &mongodb::Database,
    profile: &SocialProfile,
) -> AppResult<User>
```

**Logic (3 cases, in order):**

1. **Returning social user:** Query by `social_provider` + `social_provider_id`.
   ```rust
   db.collection::<User>(USERS)
       .find_one(doc! {
           "social_provider": profile.provider.as_str(),
           "social_provider_id": &profile.provider_id,
       })
       .await?
   ```
   If found: update `last_login_at`, `updated_at`, optionally update `avatar_url` and
   `display_name` if provider has newer values. Return the user.

2. **Existing email user (account linking):** Query by `email` (lowercased).
   ```rust
   db.collection::<User>(USERS)
       .find_one(doc! { "email": profile.email.to_lowercase() })
       .await?
   ```
   If found AND `social_provider` is `None` (user registered via email/password):
   - Link the social identity by setting `social_provider` and `social_provider_id`
   - Update `avatar_url` if currently `None` and provider has one
   - Update `last_login_at` and `updated_at`
   - If email not yet verified, mark `email_verified: true` (provider verified it)
   - Return the user.

   If found AND `social_provider` is `Some` but different provider:
   - Return `AppError::SocialAuthFailed("Email already linked to a different social provider")`
   - This prevents one email from being linked to multiple social providers.

3. **New social user:** Create a new user document.
   ```rust
   let now = Utc::now();
   let user_id = Uuid::new_v4().to_string();

   let new_user = User {
       id: user_id.clone(),
       email: profile.email.to_lowercase(),
       password_hash: None,
       display_name: profile.display_name.clone(),
       avatar_url: profile.avatar_url.clone(),
       email_verified: true, // Provider verified the email
       email_verification_token: None,
       password_reset_token: None,
       password_reset_expires_at: None,
       is_active: true,
       is_admin: false,
       role_ids: vec![],
       group_ids: vec![],
       mfa_enabled: false,
       social_provider: Some(profile.provider.as_str().to_string()),
       social_provider_id: Some(profile.provider_id.clone()),
       created_at: now,
       updated_at: now,
       last_login_at: Some(now),
   };

   db.collection::<User>(USERS).insert_one(&new_user).await?;
   ```

**Important:** The `authenticate_user` function in `auth_service.rs` already handles
social users correctly -- it returns `"This account uses social login"` when
`password_hash` is `None`.

---

## 6. Social Auth Handler

**File (new):** `backend/src/handlers/social_auth.rs`

### 6.1 State/CSRF Cookie

Cookie name: `nyx_social_state`

The state token provides CSRF protection:
- Generate a random token via `crypto::token::generate_random_token()`
- Store the SHA-256 hash of the token in an HttpOnly cookie
- Pass the raw token as the `state` parameter to the provider
- On callback, hash the incoming `state` and compare with the cookie value

This approach avoids database storage for short-lived state.

Cookie settings:
```rust
const SOCIAL_STATE_COOKIE: &str = "nyx_social_state";
const SOCIAL_STATE_MAX_AGE: i64 = 600; // 10 minutes
```

### 6.2 GET /api/v1/auth/social/{provider} (authorize)

```rust
pub async fn authorize(
    State(state): State<AppState>,
    Path(provider_name): Path<String>,
) -> AppResult<(StatusCode, HeaderMap, ())>
```

**Steps:**
1. Parse `provider_name` via `SocialProvider::from_str()`.
   - If `None`: return `AppError::SocialAuthFailed("Unsupported provider: {provider_name}")`
2. Generate random state token: `let state_token = generate_random_token();`
3. Compute hash: `let state_hash = hash_token(&state_token);`
4. Build authorization URL: `social_auth_service::build_authorization_url(provider, &state_token, &state.config)?`
5. Build response:
   - Set cookie: `build_cookie(SOCIAL_STATE_COOKIE, &state_hash, SOCIAL_STATE_MAX_AGE, "/api/v1/auth/social", secure)`
   - Set `Location` header to authorization URL
   - Return `StatusCode::FOUND` (302)

### 6.3 GET /api/v1/auth/social/{provider}/callback

```rust
#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

pub async fn callback(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(provider_name): Path<String>,
    Query(params): Query<CallbackQuery>,
    headers: HeaderMap,
) -> AppResult<(StatusCode, HeaderMap, ())>
```

**Steps:**

1. **Parse provider:**
   - `SocialProvider::from_str(&provider_name)` -> if None, redirect to frontend with error

2. **Check for provider error:**
   - If `params.error` is Some, redirect to `{FRONTEND_URL}/login?error=social_auth_denied`

3. **Extract and validate code + state:**
   - If `params.code` is None or `params.state` is None, redirect with error
   - Hash the incoming state: `hash_token(state_param)`
   - Extract cookie: parse `nyx_social_state` from the request cookie header
   - Compare: `cookie_state_hash == computed_state_hash`
   - If mismatch: redirect to `{FRONTEND_URL}/login?error=social_auth_csrf`

4. **Exchange code for access token:**
   ```rust
   let access_token = social_auth_service::exchange_code(
       provider, &code, &state.config, &state.http_client
   ).await?;
   ```

5. **Fetch user profile:**
   ```rust
   let profile = social_auth_service::fetch_user_profile(
       provider, &access_token, &state.http_client
   ).await?;
   ```

6. **Find or create user:**
   ```rust
   let user = social_auth_service::find_or_create_user(&state.db, &profile).await?;
   ```

7. **Issue session and tokens** (reuse existing token_service):
   ```rust
   let ip = extract_ip(&headers, Some(peer));
   let ua = extract_user_agent(&headers);

   let tokens = token_service::create_session_and_issue_tokens(
       &state.db,
       &state.config,
       &state.jwt_keys,
       &user.id,
       ip.as_deref(),
       ua.as_deref(),
   ).await?;
   ```

8. **Audit log:**
   ```rust
   audit_service::log_async(
       state.db.clone(),
       Some(user.id.clone()),
       "social_login".to_string(),
       Some(serde_json::json!({
           "provider": provider.as_str(),
           "session_id": tokens.session_id,
       })),
       ip,
       ua,
   );
   ```

9. **Build response headers** (same cookie pattern as `login` handler):
   ```rust
   let secure = state.config.use_secure_cookies();
   let mut response_headers = HeaderMap::new();

   // Session cookie (30 days)
   response_headers.insert(
       header::SET_COOKIE,
       build_cookie(SESSION_COOKIE_NAME, &tokens.session_token, 30 * 24 * 3600, "/", secure)
           .parse().map_err(|_| AppError::Internal("Cookie error".to_string()))?,
   );

   // Access token cookie
   response_headers.append(
       header::SET_COOKIE,
       build_cookie(ACCESS_TOKEN_COOKIE_NAME, &tokens.access_token, tokens.access_expires_in, "/", secure)
           .parse().map_err(|_| AppError::Internal("Cookie error".to_string()))?,
   );

   // Refresh token cookie
   response_headers.append(
       header::SET_COOKIE,
       build_cookie("nyx_refresh_token", &tokens.refresh_token, state.config.jwt_refresh_ttl_secs, "/api/v1/auth/refresh", secure)
           .parse().map_err(|_| AppError::Internal("Cookie error".to_string()))?,
   );

   // Clear state cookie
   response_headers.append(
       header::SET_COOKIE,
       clear_cookie(SOCIAL_STATE_COOKIE, "/api/v1/auth/social", secure)
           .parse().map_err(|_| AppError::Internal("Cookie error".to_string()))?,
   );

   // Redirect to frontend dashboard
   response_headers.insert(
       header::LOCATION,
       format!("{}/dashboard", state.config.frontend_url)
           .parse().map_err(|_| AppError::Internal("Redirect error".to_string()))?,
   );
   ```

10. Return `(StatusCode::FOUND, response_headers, ())`

### 6.4 Error Handling in Callback

For errors that happen during the callback flow (code exchange failure, profile
fetch failure, etc.), redirect to the frontend with a query parameter instead
of returning a JSON error (since this is a browser redirect flow):

```rust
fn redirect_with_error(frontend_url: &str, error: &str, secure: bool) -> (StatusCode, HeaderMap, ()) {
    let mut headers = HeaderMap::new();
    let url = format!("{}/login?error={}", frontend_url, error);
    headers.insert(header::LOCATION, url.parse().unwrap());
    // Clear the state cookie on error too
    headers.append(
        header::SET_COOKIE,
        clear_cookie(SOCIAL_STATE_COOKIE, "/api/v1/auth/social", secure)
            .parse().unwrap(),
    );
    (StatusCode::FOUND, headers, ())
}
```

Error mapping for redirects:
| Failure                   | Error query param          |
|---------------------------|---------------------------|
| Provider error response   | `social_auth_denied`      |
| Missing code/state        | `social_auth_invalid`     |
| State/CSRF mismatch       | `social_auth_csrf`        |
| Code exchange failure     | `social_auth_exchange`    |
| Profile fetch failure     | `social_auth_profile`     |
| Email already linked      | `social_auth_conflict`    |
| No verified email         | `social_auth_no_email`    |
| Provider not configured   | `social_auth_unavailable` |
| Unsupported provider      | `social_auth_unsupported` |

---

## 7. Route Registration

**File:** `backend/src/routes.rs`

Add the two social auth routes to `auth_routes`:

```rust
let auth_routes = Router::new()
    .route("/register", post(handlers::auth::register))
    .route("/login", post(handlers::auth::login))
    .route("/logout", post(handlers::auth::logout))
    .route("/refresh", post(handlers::auth::refresh))
    .route("/verify-email", post(handlers::auth::verify_email))
    .route("/forgot-password", post(handlers::auth::forgot_password))
    .route("/reset-password", post(handlers::auth::reset_password))
    .route("/setup", post(handlers::auth::setup))
    // Social login (no auth required -- these are browser redirects)
    .route("/social/{provider}", get(handlers::social_auth::authorize))
    .route("/social/{provider}/callback", get(handlers::social_auth::callback))
    .nest("/mfa", mfa_routes);
```

**Note:** These routes sit inside `api_v1_human_only` which has
`reject_delegated_tokens` and `reject_service_account_tokens` middleware.
However, these middleware layers check for existing auth tokens -- the social
login endpoints are public (no `AuthUser` extractor), so unauthenticated
browser requests will pass through the middleware unaffected.

**File:** `backend/src/handlers/mod.rs`

Add:
```rust
pub mod social_auth;
```

**File:** `backend/src/services/mod.rs`

Add:
```rust
pub mod social_auth_service;
```

---

## 8. Public Config: Expose Enabled Providers

**File:** `backend/src/handlers/health.rs`

Update `PublicConfigResponse` to include social providers:

```rust
#[derive(Serialize)]
pub struct PublicConfigResponse {
    pub mcp_url: String,
    pub version: String,
    pub social_providers: Vec<String>,
}
```

Update `public_config` handler:

```rust
pub async fn public_config(State(state): State<AppState>) -> Json<PublicConfigResponse> {
    let base = state.config.base_url.trim_end_matches('/');

    let mut social_providers = Vec::new();
    if state.config.github_client_id.is_some() && state.config.github_client_secret.is_some() {
        social_providers.push("github".to_string());
    }
    if state.config.google_client_id.is_some() && state.config.google_client_secret.is_some() {
        social_providers.push("google".to_string());
    }

    Json(PublicConfigResponse {
        mcp_url: format!("{base}/mcp"),
        version: env!("CARGO_PKG_VERSION").to_string(),
        social_providers,
    })
}
```

---

## 9. Frontend Changes

### 9.1 Remove Apple from social-login-buttons.tsx

**File:** `frontend/src/components/auth/social-login-buttons.tsx`

- Remove the Apple entry from `SOCIAL_PROVIDERS` array
- Change `grid-cols-3` to `grid-cols-2`
- Keep Google and GitHub entries unchanged

### 9.2 Update PublicConfig type

**File:** `frontend/src/types/api.ts`

```typescript
export interface PublicConfig {
  readonly mcp_url: string;
  readonly version: string;
  readonly social_providers: readonly string[];
}
```

### 9.3 Optional: Conditional rendering based on config

The `SocialLoginButtons` component could optionally consume `usePublicConfig()`
to hide buttons for unconfigured providers. This is a nice-to-have for the
initial implementation -- the backend will return a clear error redirect if a
user clicks a button for an unconfigured provider.

---

## 10. Provider-Specific Reference

### GitHub

| Item | Value |
|------|-------|
| Authorization URL | `https://github.com/login/oauth/authorize` |
| Token URL | `https://github.com/login/oauth/access_token` |
| User API | `https://api.github.com/user` |
| Emails API | `https://api.github.com/user/emails` |
| Scope | `user:email` |
| Client ID env | `GITHUB_CLIENT_ID` |
| Client Secret env | `GITHUB_CLIENT_SECRET` |
| Provider ID field | `id` (u64, convert to string) |
| Required headers | `User-Agent: NyxID`, `Accept: application/json` |

### Google

| Item | Value |
|------|-------|
| Authorization URL | `https://accounts.google.com/o/oauth2/v2/auth` |
| Token URL | `https://oauth2.googleapis.com/token` |
| UserInfo URL | `https://www.googleapis.com/oauth2/v3/userinfo` |
| Scope | `openid email profile` |
| Client ID env | `GOOGLE_CLIENT_ID` |
| Client Secret env | `GOOGLE_CLIENT_SECRET` |
| Provider ID field | `sub` (string) |
| Extra auth params | `response_type=code`, `access_type=online` |

---

## 11. Cookie Summary

| Cookie Name | Purpose | Max-Age | Path | HttpOnly | SameSite |
|-------------|---------|---------|------|----------|----------|
| `nyx_social_state` | CSRF state hash | 600s (10 min) | `/api/v1/auth/social` | Yes | Lax |
| `nyx_session` | Session token | 30 days | `/` | Yes | Lax |
| `nyx_access_token` | JWT access token | 900s (15 min) | `/` | Yes | Lax |
| `nyx_refresh_token` | JWT refresh token | 7 days | `/api/v1/auth/refresh` | Yes | Lax |

All cookies get `Secure` flag when `config.use_secure_cookies()` returns true
(i.e., not localhost/127.0.0.1).

---

## 12. Files Changed Summary

| File | Action | Description |
|------|--------|-------------|
| `backend/src/models/user.rs` | Modify | Add `social_provider`, `social_provider_id` fields |
| `backend/src/db.rs` | Modify | Add sparse unique compound index on `(social_provider, social_provider_id)` |
| `backend/src/errors/mod.rs` | Modify | Add `SocialAuthFailed` variant (error code 6000) |
| `backend/src/services/social_auth_service.rs` | **New** | OAuth flow: build URL, exchange code, fetch profile, find/create user |
| `backend/src/handlers/social_auth.rs` | **New** | `authorize` and `callback` HTTP handlers |
| `backend/src/services/mod.rs` | Modify | Add `pub mod social_auth_service;` |
| `backend/src/handlers/mod.rs` | Modify | Add `pub mod social_auth;` |
| `backend/src/routes.rs` | Modify | Add `/social/{provider}` and `/social/{provider}/callback` routes |
| `backend/src/handlers/health.rs` | Modify | Add `social_providers` to `PublicConfigResponse` |
| `backend/src/services/auth_service.rs` | Modify | Add `social_provider: None, social_provider_id: None` to `register_user` |
| `frontend/src/components/auth/social-login-buttons.tsx` | Modify | Remove Apple, change grid to 2 columns |
| `frontend/src/types/api.ts` | Modify | Add `social_providers` to `PublicConfig` |

---

## 13. Testing Strategy

### Unit Tests

- `SocialProvider::from_str` -- valid/invalid provider names
- `build_authorization_url` -- correct URL construction for both providers
- `build_authorization_url` -- error when provider not configured
- User model BSON roundtrip with new fields (extend existing test)
- Error variant mappings for `SocialAuthFailed`

### Integration Tests (require MongoDB)

- `find_or_create_user` -- new user creation
- `find_or_create_user` -- returning social user
- `find_or_create_user` -- account linking (email match)
- `find_or_create_user` -- conflict (email linked to different provider)
- Social provider compound index prevents duplicate (provider, provider_id) pairs

### Manual Test Plan

1. Click "Sign in with GitHub" -> redirects to GitHub OAuth page
2. Authorize on GitHub -> callback creates user, sets cookies, redirects to dashboard
3. Log out and click "Sign in with GitHub" again -> recognizes existing user
4. Register with email, then click "Sign in with GitHub" using same email -> links accounts
5. Click "Sign in with Google" -> redirects to Google OAuth page
6. Complete Google flow -> creates user, redirects to dashboard
7. Verify only GitHub and Google buttons visible (no Apple)
8. Verify `GET /api/v1/public/config` returns `social_providers` array
9. Verify unconfigured provider returns redirect with error query param

---

## 14. Security Considerations

1. **CSRF via state parameter:** The state token is generated server-side, its hash
   stored in an HttpOnly cookie. The callback verifies the hash matches. An
   attacker cannot forge this without access to the cookie.

2. **No open redirects:** The callback only redirects to `FRONTEND_URL` (from config).
   The authorization URL only redirects to provider URLs built server-side.

3. **Token exchange server-side:** Client secrets never leave the backend.
   The code-for-token exchange happens server-to-server.

4. **Email verification trust:** We trust GitHub's and Google's email verification.
   For GitHub, we only use emails marked `verified: true`.

5. **Account linking guard:** We prevent linking one email to multiple social providers,
   reducing the risk of account takeover via a compromised provider.

6. **State cookie scoped:** The `nyx_social_state` cookie is scoped to
   `/api/v1/auth/social` and expires in 10 minutes, minimizing exposure.

7. **Provider ID uniqueness:** The sparse unique compound index ensures a provider user
   ID can only be linked to one NyxID account.

---

## 15. Dependencies

No new crate dependencies required. The implementation uses:
- `reqwest` (already in Cargo.toml with `json` feature)
- `serde` / `serde_json` (already present)
- `sha2` via `crypto::token::hash_token` (already present)
- `rand` via `crypto::token::generate_random_token` (already present)
- `uuid` (already present)
- `chrono` (already present)
