# NyxID API Reference

This document describes every HTTP endpoint exposed by the NyxID backend. All endpoints accept and return `application/json` unless otherwise noted.

---

## Table of Contents

- [Authentication](#authentication)
- [Error Format](#error-format)
- [Error Codes](#error-codes)
- [Endpoints](#endpoints)
  - [Health](#health)
  - [Auth](#auth)
  - [Social Auth](#social-auth)
  - [Users](#users)
  - [API Keys](#api-keys)
  - [Downstream Services](#downstream-services)
  - [Service Connections](#service-connections)
  - [Service Provider Requirements](#service-provider-requirements)
  - [Providers](#providers)
  - [User Provider Tokens](#user-provider-tokens)
  - [Sessions](#sessions)
  - [Service Endpoints](#service-endpoints)
  - [MCP Config](#mcp-config)
  - [Proxy](#proxy)
  - [Proxy Service Discovery](#proxy-service-discovery)
  - [LLM Gateway](#llm-gateway)
  - [MFA](#mfa-multi-factor-authentication)
  - [OAuth / OpenID Connect](#oauth--openid-connect)
  - [Token Exchange (Delegated Access)](#token-exchange-delegated-access)
  - [Token Introspection](#token-introspection)
  - [Token Revocation](#token-revocation)
  - [User Consents](#user-consents)
  - [OIDC Discovery](#oidc-discovery)
  - [Admin](#admin)
  - [Admin Roles](#admin-roles)
  - [Admin Groups](#admin-groups)
  - [Admin Service Accounts](#admin-service-accounts)
  - [Notification Settings](#notification-settings)
  - [Approval Management](#approval-management)
  - [Webhooks](#webhooks)

---

## Authentication

Most endpoints require authentication. NyxID supports four authentication methods, checked in the following order:

1. **Bearer Token** -- `Authorization: Bearer <access_token>` header
2. **Session Cookie** -- `nyx_session` HttpOnly cookie (set at login)
3. **Access Token Cookie** -- `nyx_access_token` HttpOnly cookie (set at login)
4. **API Key** -- `X-API-Key: <key>` header

Endpoints marked **Auth: None** do not require authentication.
Endpoints marked **Auth: Required** require any of the above.
Endpoints marked **Auth: Admin** require an authenticated user with `is_admin = true`.
Endpoints marked **Auth: Cookie** use a specific cookie (e.g., the refresh token cookie).

**Service accounts** authenticate via OAuth2 Client Credentials Grant at `POST /oauth/token` and receive a Bearer token. Service account tokens include an `sa: true` claim and are restricted to proxy, LLM gateway, connections, providers, and delegation endpoints.

---

## Error Format

All errors are returned as JSON with the following structure:

```json
{
  "error": "error_key",
  "error_code": 1000,
  "message": "Human-readable description"
}
```

The `session_token` field is only present when `error_code` is `2002` (MFA required):

```json
{
  "error": "mfa_required",
  "error_code": 2002,
  "message": "MFA verification required",
  "session_token": "temporary_mfa_session_token"
}
```

Internal errors never leak implementation details. The `message` for error codes `1006` and `1007` is always `"An internal error occurred"`.

---

## Error Codes

| Code | Key                        | HTTP Status | Description                              |
|------|----------------------------|-------------|------------------------------------------|
| 1000 | `bad_request`              | 400         | Malformed request                        |
| 1001 | `unauthorized`             | 401         | Missing or invalid credentials           |
| 1002 | `forbidden`                | 403         | Insufficient permissions                 |
| 1003 | `not_found`                | 404         | Resource does not exist                  |
| 1004 | `conflict`                 | 409         | Resource already exists                  |
| 1005 | `rate_limited`             | 429         | Rate limit exceeded                      |
| 1006 | `internal_error`           | 500         | Server error (details redacted)          |
| 1007 | `database_error`           | 500         | Database error (details redacted)        |
| 1008 | `validation_error`         | 400         | Input validation failed                  |
| 2000 | `authentication_failed`    | 401         | Wrong email/password or invalid MFA code |
| 2001 | `token_expired`            | 401         | JWT has expired                          |
| 2002 | `mfa_required`             | 403         | MFA verification needed to complete login|
| 3000 | `pkce_verification_failed` | 400         | PKCE code_verifier mismatch              |
| 3001 | `invalid_redirect_uri`     | 400         | Redirect URI not registered for client   |
| 3002 | `invalid_scope`            | 400         | Requested scope not allowed              |
| 5000 | `service_account_not_found`| 404         | Service account does not exist           |
| 5001 | `service_account_inactive` | 403         | Service account is deactivated           |
| 6000 | `social_auth_failed`       | 400         | Social authentication failed             |
| 6001 | `social_auth_conflict`     | 409         | Email already linked to another provider |
| 6002 | `social_auth_no_email`     | 400         | No verified email from provider          |
| 6003 | `social_auth_deactivated`  | 403         | Social login account is deactivated      |
| 7000 | `approval_required`        | 403         | User approval required (proxy/LLM requests block until decision; this code is used for async status polling) |

---

## Endpoints

### Health

#### GET /health

Returns service health status. No authentication required.

**Auth:** None

**Response:**

```json
{
  "status": "ok",
  "version": "0.1.0"
}
```

**Example:**

```bash
curl http://localhost:3001/health
```

---

### Auth

#### POST /api/v1/auth/register

Create a new user account.

**Auth:** None

**Request Body:**

| Field          | Type   | Required | Description                               |
|----------------|--------|----------|-------------------------------------------|
| `email`        | string | Yes      | Valid email address                       |
| `password`     | string | Yes      | 8-128 characters                          |
| `display_name` | string | No       | User display name                         |

```json
{
  "email": "user@example.com",
  "password": "securepassword123",
  "display_name": "Jane Doe"
}
```

**Response (200):**

```json
{
  "user_id": "550e8400-e29b-41d4-a716-446655440000",
  "message": "Registration successful. Please verify your email."
}
```

**Errors:**
- `1004 conflict` -- Email already registered
- `1008 validation_error` -- Invalid email format or password length

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/auth/register \
  -H "Content-Type: application/json" \
  -d '{
    "email": "user@example.com",
    "password": "securepassword123",
    "display_name": "Jane Doe"
  }'
```

---

#### POST /api/v1/auth/login

Authenticate with email and password. On success, sets three HttpOnly cookies (`nyx_session`, `nyx_access_token`, `nyx_refresh_token`) and returns the access token in the response body.

If the user has MFA enabled and no `mfa_code` is provided, returns a `403` with error code `2002` and a `session_token` for the MFA verification step.

**Auth:** None

**Request Body:**

| Field      | Type   | Required | Description                                    |
|------------|--------|----------|------------------------------------------------|
| `email`    | string | Yes      | User email address                             |
| `password` | string | Yes      | User password (max 128 chars)                  |
| `mfa_code` | string | No       | 6-digit TOTP code (required if MFA is enabled) |

```json
{
  "email": "user@example.com",
  "password": "securepassword123"
}
```

**Response (200):**

```json
{
  "user_id": "550e8400-e29b-41d4-a716-446655440000",
  "access_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...",
  "expires_in": 900
}
```

**Response Headers (Set-Cookie):**

```
Set-Cookie: nyx_session=<token>; HttpOnly; SameSite=Lax; Path=/; Max-Age=2592000
Set-Cookie: nyx_access_token=<jwt>; HttpOnly; SameSite=Lax; Path=/; Max-Age=900
Set-Cookie: nyx_refresh_token=<jwt>; HttpOnly; SameSite=Lax; Path=/api/v1/auth/refresh; Max-Age=604800
```

**MFA Challenge Response (403):**

```json
{
  "error": "mfa_required",
  "error_code": 2002,
  "message": "MFA verification required",
  "session_token": "temporary_session_token_here"
}
```

To complete login with MFA, re-send the login request with the `mfa_code` field included.

**Errors:**
- `2000 authentication_failed` -- Wrong email/password or invalid MFA code
- `2002 mfa_required` -- MFA code required (includes `session_token`)
- `1008 validation_error` -- Invalid email format or password too long

**Example:**

```bash
# Basic login
curl -X POST http://localhost:3001/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -c cookies.txt \
  -d '{
    "email": "user@example.com",
    "password": "securepassword123"
  }'

# Login with MFA
curl -X POST http://localhost:3001/api/v1/auth/login \
  -H "Content-Type: application/json" \
  -c cookies.txt \
  -d '{
    "email": "user@example.com",
    "password": "securepassword123",
    "mfa_code": "123456"
  }'
```

---

#### POST /api/v1/auth/logout

Revoke the current session and clear all authentication cookies.

**Auth:** Required

**Response (200):**

```json
{
  "message": "Logged out successfully"
}
```

**Response Headers:** Clears `nyx_session`, `nyx_access_token`, and `nyx_refresh_token` cookies.

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/auth/logout \
  -H "Authorization: Bearer <access_token>"
```

---

#### POST /api/v1/auth/refresh

Exchange a refresh token for a new access token. The refresh token is read from the `nyx_refresh_token` cookie. Implements token rotation: the old refresh token is invalidated and a new one is issued.

**Auth:** Cookie (`nyx_refresh_token`)

**Response (200):**

```json
{
  "access_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...",
  "expires_in": 900
}
```

**Response Headers:** Sets new `nyx_access_token` and `nyx_refresh_token` cookies.

**Errors:**
- `1001 unauthorized` -- No refresh token cookie present
- `2001 token_expired` -- Refresh token has expired

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/auth/refresh \
  -b cookies.txt \
  -c cookies.txt
```

---

#### POST /api/v1/auth/verify-email

Verify a user's email address using the token sent during registration.

**Auth:** None

**Request Body:**

| Field   | Type   | Required | Description                         |
|---------|--------|----------|-------------------------------------|
| `token` | string | Yes      | Email verification token            |

```json
{
  "token": "verification-token-here"
}
```

**Response (200):**

```json
{
  "message": "Email verified successfully"
}
```

**Errors:**
- `1000 bad_request` -- Missing or invalid token
- `1003 not_found` -- Token not found or already used

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/auth/verify-email \
  -H "Content-Type: application/json" \
  -d '{"token": "verification-token-here"}'
```

---

#### POST /api/v1/auth/forgot-password

Request a password reset. Always returns success to prevent email enumeration.

**Auth:** None

**Request Body:**

| Field   | Type   | Required | Description           |
|---------|--------|----------|-----------------------|
| `email` | string | Yes      | User email address    |

```json
{
  "email": "user@example.com"
}
```

**Response (200):**

```json
{
  "message": "If an account exists with that email, a password reset link has been sent."
}
```

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/auth/forgot-password \
  -H "Content-Type: application/json" \
  -d '{"email": "user@example.com"}'
```

---

#### POST /api/v1/auth/reset-password

Reset a user's password using a valid reset token.

**Auth:** None

**Request Body:**

| Field          | Type   | Required | Description                    |
|----------------|--------|----------|--------------------------------|
| `token`        | string | Yes      | Password reset token           |
| `new_password` | string | Yes      | New password (8-128 characters)|

```json
{
  "token": "reset-token-here",
  "new_password": "newsecurepassword123"
}
```

**Response (200):**

```json
{
  "message": "Password reset successfully"
}
```

**Errors:**
- `1000 bad_request` -- Missing token or password too short/long
- `1003 not_found` -- Token not found, expired, or already used

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/auth/reset-password \
  -H "Content-Type: application/json" \
  -d '{"token": "reset-token-here", "new_password": "newsecurepassword123"}'
```

---

#### POST /api/v1/auth/setup

One-time bootstrap endpoint to create the initial admin user. Only works when the users collection is completely empty. After the first user is created, this endpoint returns 403 Forbidden.

**Auth:** None

**Request Body:**

| Field          | Type   | Required | Description                               |
|----------------|--------|----------|-------------------------------------------|
| `email`        | string | Yes      | Valid email address                       |
| `password`     | string | Yes      | 8-128 characters                          |
| `display_name` | string | No       | Admin display name                        |

```json
{
  "email": "admin@example.com",
  "password": "secureadminpassword123",
  "display_name": "Admin"
}
```

**Response (200):**

```json
{
  "user_id": "550e8400-e29b-41d4-a716-446655440000",
  "message": "Admin account created successfully."
}
```

**Errors:**
- `1002 forbidden` -- Users already exist (setup already completed)
- `1008 validation_error` -- Invalid email format or password length

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/auth/setup \
  -H "Content-Type: application/json" \
  -d '{
    "email": "admin@example.com",
    "password": "secureadminpassword123",
    "display_name": "Admin"
  }'
```

---

### Social Auth

Social login allows users to authenticate via GitHub or Google OAuth 2.0. The flow is entirely browser-based: the frontend navigates to the authorize endpoint, the user authenticates with the provider, and the callback endpoint creates a session and redirects to the frontend dashboard.

**Flow:**

1. Frontend navigates to `GET /api/v1/auth/social/{provider}` (e.g., via a "Sign in with GitHub" button)
2. Backend generates a CSRF state token, stores it as a cookie, and redirects (302) to the provider's authorization page
3. User authorizes the application on the provider's site
4. Provider redirects back to `GET /api/v1/auth/social/{provider}/callback` with `code` and `state` query parameters
5. Backend validates the state token, exchanges the code for an access token, fetches the user's profile
6. Backend finds or creates a user (matching by email), creates a session, sets auth cookies, and redirects to the frontend dashboard

#### GET /api/v1/auth/social/{provider}

Initiate an OAuth 2.0 authorization flow with a social provider.

**Auth:** None

**Path Parameters:**

| Parameter  | Type   | Description                            |
|------------|--------|----------------------------------------|
| `provider` | string | Social provider: `"github"` or `"google"` |

**Response (302):** Redirects to the provider's authorization page. Sets a `social_auth_state` HttpOnly cookie containing the CSRF state token.

**Errors:**
- `6000 social_auth_failed` -- Provider not configured (missing client ID/secret) or unsupported provider

**Example:**

Navigate in browser:
```
http://localhost:3001/api/v1/auth/social/github
http://localhost:3001/api/v1/auth/social/google
```

---

#### GET /api/v1/auth/social/{provider}/callback

OAuth callback handler. Called by the provider after user authorization.

**Auth:** None (called by the OAuth provider redirect)

**Path Parameters:**

| Parameter  | Type   | Description                            |
|------------|--------|----------------------------------------|
| `provider` | string | Social provider: `"github"` or `"google"` |

**Query Parameters:**

| Parameter | Type   | Description                                         |
|-----------|--------|-----------------------------------------------------|
| `code`    | string | Authorization code from the provider                |
| `state`   | string | CSRF state token (must match the `social_auth_state` cookie) |
| `error`   | string | Error code from the provider (if authorization was denied) |

**Response (302 on success):** Redirects to the frontend dashboard (`{FRONTEND_URL}/dashboard`). Sets the same auth cookies as a regular login: `nyx_session`, `nyx_access_token`, `nyx_refresh_token`.

**Response (302 on error):** Redirects to `{FRONTEND_URL}/login?error={error_key}`.

Possible error keys in the redirect:

| Scenario                          | Error key in redirect         |
|-----------------------------------|-------------------------------|
| Provider returned an error        | `social_auth_denied`          |
| Missing code or state parameter   | `social_auth_invalid`         |
| State/CSRF token mismatch         | `social_auth_csrf`            |
| Code exchange failed              | `social_auth_exchange`        |
| Profile fetch failed              | `social_auth_profile`         |
| Email linked to another provider  | `social_auth_conflict`        |
| No verified email from provider   | `social_auth_no_email`        |
| Provider not configured           | `social_auth_unavailable`     |
| Unsupported provider name         | `social_auth_unsupported`     |
| Account is deactivated            | `social_auth_deactivated`     |

**Notes:**
- If a user with the same email already exists and has no social provider set, the account is linked to this provider
- If a user with the same email is already linked to a *different* social provider, the login fails with `social_auth_conflict`
- New users created via social login have `email_verified = true` (provider verified the email) and no password set
- Social login users cannot use the password reset or password login flows

---

### Users

#### GET /api/v1/users/me

Returns the profile of the currently authenticated user.

**Auth:** Required

**Response (200):**

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "email": "user@example.com",
  "display_name": "Jane Doe",
  "avatar_url": "https://example.com/avatar.jpg",
  "email_verified": true,
  "mfa_enabled": false,
  "created_at": "2025-01-15T10:30:00+00:00",
  "last_login_at": "2025-06-01T14:22:00+00:00"
}
```

**Example:**

```bash
curl http://localhost:3001/api/v1/users/me \
  -H "Authorization: Bearer <access_token>"
```

---

#### PUT /api/v1/users/me

Update the profile of the currently authenticated user.

**Auth:** Required

**Request Body:**

| Field          | Type   | Required | Description                                  |
|----------------|--------|----------|----------------------------------------------|
| `display_name` | string | No       | New display name (max 200 chars)             |
| `avatar_url`   | string | No       | New avatar URL (must use https:// or http://) |

```json
{
  "display_name": "Jane Smith",
  "avatar_url": "https://example.com/new-avatar.jpg"
}
```

**Response (200):**

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "email": "user@example.com",
  "display_name": "Jane Smith",
  "avatar_url": "https://example.com/new-avatar.jpg",
  "message": "Profile updated successfully"
}
```

**Errors:**
- `1008 validation_error` -- Display name too long, or avatar URL has invalid scheme

**Example:**

```bash
curl -X PUT http://localhost:3001/api/v1/users/me \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{"display_name": "Jane Smith"}'
```

---

### API Keys

#### GET /api/v1/api-keys

List all API keys for the authenticated user. The full key value is never returned after creation.

**Auth:** Required

**Response (200):**

```json
{
  "keys": [
    {
      "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "name": "Production API Key",
      "key_prefix": "nyx_k_a1b2c3d4",
      "scopes": "read write",
      "last_used_at": "2025-06-01T14:22:00+00:00",
      "expires_at": null,
      "is_active": true,
      "created_at": "2025-01-15T10:30:00+00:00"
    }
  ]
}
```

**Example:**

```bash
curl http://localhost:3001/api/v1/api-keys \
  -H "Authorization: Bearer <access_token>"
```

---

#### POST /api/v1/api-keys

Create a new API key. The full key is returned only in this response and cannot be retrieved again.

**Auth:** Required

**Request Body:**

| Field        | Type   | Required | Description                                  |
|--------------|--------|----------|----------------------------------------------|
| `name`       | string | Yes      | Human-readable name for the key              |
| `scopes`     | string | No       | Space-separated scopes (default: `"read"`)   |
| `expires_at` | string | No       | ISO 8601 expiration datetime                 |

```json
{
  "name": "Production API Key",
  "scopes": "read write",
  "expires_at": "2026-01-01T00:00:00Z"
}
```

**Response (200):**

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "name": "Production API Key",
  "key_prefix": "nyx_k_a1b2c3d4",
  "full_key": "nyx_k_a1b2c3d4e5f67890abcdef1234567890abcdef1234567890abcdef12345678",
  "scopes": "read write",
  "created_at": "2025-06-01T10:00:00+00:00"
}
```

**Errors:**
- `1008 validation_error` -- Empty name

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/api-keys \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{"name": "My Key", "scopes": "read"}'
```

---

#### DELETE /api/v1/api-keys/{key_id}

Deactivate an API key. The key can no longer be used for authentication after this operation.

**Auth:** Required

**Path Parameters:**

| Parameter | Type | Description      |
|-----------|------|------------------|
| `key_id`  | UUID | The API key ID   |

**Response (200):**

```json
{
  "message": "API key deleted"
}
```

**Errors:**
- `1003 not_found` -- Key does not exist or does not belong to the user

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/api-keys/a1b2c3d4-e5f6-7890-abcd-ef1234567890 \
  -H "Authorization: Bearer <access_token>"
```

---

#### POST /api/v1/api-keys/{key_id}/rotate

Rotate an API key: deactivate the existing key and create a new one with the same name and scopes. The new full key is returned in the response.

**Auth:** Required

**Path Parameters:**

| Parameter | Type | Description      |
|-----------|------|------------------|
| `key_id`  | UUID | The API key ID   |

**Response (200):**

```json
{
  "id": "new-uuid-here",
  "name": "Production API Key",
  "key_prefix": "nyx_k_b2c3d4e5",
  "full_key": "nyx_k_b2c3d4e5f67890abcdef1234567890abcdef1234567890abcdef12345678ab",
  "scopes": "read write",
  "created_at": "2025-06-02T10:00:00+00:00"
}
```

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/api-keys/a1b2c3d4-e5f6-7890-abcd-ef1234567890/rotate \
  -H "Authorization: Bearer <access_token>"
```

---

### Downstream Services

#### GET /api/v1/services

List all active downstream services. Supports optional filtering by service category.

**Auth:** Required

**Query Parameters:**

| Parameter  | Type   | Required | Description                                           |
|------------|--------|----------|-------------------------------------------------------|
| `category` | string | No       | Filter by service category: `provider`, `connection`, or `internal`. Omit for all. |

**Response (200):**

```json
{
  "services": [
    {
      "id": "d1e2f3a4-b5c6-7890-1234-567890abcdef",
      "name": "Stripe API",
      "slug": "stripe",
      "description": "Payment processing",
      "base_url": "https://api.stripe.com",
      "auth_method": "header",
      "auth_type": "api_key",
      "auth_key_name": "Authorization",
      "is_active": true,
      "oauth_client_id": null,
      "api_spec_url": null,
      "service_category": "connection",
      "requires_user_credential": true,
      "identity_propagation_mode": "none",
      "identity_include_user_id": false,
      "identity_include_email": false,
      "identity_include_name": false,
      "identity_jwt_audience": null,
      "created_by": "550e8400-e29b-41d4-a716-446655440000",
      "created_at": "2025-01-15T10:30:00+00:00",
      "updated_at": "2025-01-15T10:30:00+00:00"
    }
  ]
}
```

**Example:**

```bash
# List all services
curl http://localhost:3001/api/v1/services \
  -H "Authorization: Bearer <access_token>"

# List only connectable services
curl "http://localhost:3001/api/v1/services?category=connection" \
  -H "Authorization: Bearer <access_token>"
```

---

#### POST /api/v1/services

Register a new downstream service. The credential is encrypted with AES-256-GCM before storage.

When `auth_type` (or `auth_method`) is set to `"oidc"`, NyxID automatically provisions an OAuth client for the service, generates a client secret, and sets the default redirect URI to `{base_url}/callback`. No `credential` field is needed for OIDC services.

**Auth:** Admin

**Request Body:**

| Field              | Type   | Required | Description                                                                           |
|--------------------|--------|----------|---------------------------------------------------------------------------------------|
| `name`             | string | Yes      | Service display name (max 200 chars)                                                  |
| `slug`             | string | No       | URL-safe identifier (max 100 chars, unique). Auto-derived from `name` if omitted.     |
| `description`      | string | No       | Service description                                                                   |
| `base_url`         | string | Yes      | Downstream service base URL (max 2048 chars). Must not point to private/internal IPs. |
| `auth_type`        | string | No       | One of: `api_key`, `oauth2`/`bearer`, `basic`, `oidc`, `header`, `query`. Default: `header`. Alias: `auth_method`. |
| `auth_key_name`    | string | No       | Header or query param name. Defaults based on `auth_type`.                            |
| `credential`       | string | No       | API key, token, or `user:password` for basic. Not needed for OIDC services.           |
| `service_category` | string | No       | `"connection"` (default), `"internal"`, or `"provider"` (OIDC only). See below.       |

**Auth Type Mapping:**

| `auth_type` value  | Internal `auth_method` | Default `auth_key_name` | Behavior                                            |
|--------------------|------------------------|-------------------------|-----------------------------------------------------|
| `api_key` / `header` | `header`             | `X-API-Key`             | Adds `auth_key_name: credential` as a request header|
| `oauth2` / `bearer`  | `bearer`             | `Authorization`         | Adds `Authorization: Bearer credential` header      |
| `query`              | `query`              | `api_key`               | Appends `?auth_key_name=credential` to the URL      |
| `basic`              | `basic`              | `Authorization`         | Sends HTTP Basic Auth (credential = `user:password`) |
| `oidc`               | `oidc`               | `X-API-Key`             | Auto-provisions OAuth client; uses OIDC flow        |

**Service Category Rules:**

| `service_category` | When to use | `requires_user_credential` | User can connect? |
|--------------------|-------------|----------------------------|-------------------|
| `connection` (default) | External services users connect to with their own credentials | `true` | Yes (must supply credential) |
| `internal` | Services using a master credential managed by admin | `false` | Yes (enable only, no credential) |
| `provider` | OIDC services (auto-assigned when `auth_type` is `oidc`) | `false` | No (admin-managed) |

**Example (connection service with API key):**

```json
{
  "name": "Stripe API",
  "slug": "stripe",
  "description": "Payment processing",
  "base_url": "https://api.stripe.com",
  "auth_type": "api_key",
  "credential": "sk-master-key-here",
  "service_category": "connection"
}
```

**Example (internal service):**

```json
{
  "name": "Internal Analytics",
  "base_url": "https://analytics.internal.example.com",
  "auth_type": "bearer",
  "credential": "internal-master-token",
  "service_category": "internal"
}
```

**Example (OIDC service):**

```json
{
  "name": "Customer Portal",
  "base_url": "https://portal.example.com",
  "auth_type": "oidc"
}
```

**Response (200):**

```json
{
  "id": "d1e2f3a4-b5c6-7890-1234-567890abcdef",
  "name": "Stripe API",
  "slug": "stripe",
  "description": "Payment processing",
  "base_url": "https://api.stripe.com",
  "auth_method": "header",
  "auth_type": "api_key",
  "auth_key_name": "X-API-Key",
  "is_active": true,
  "oauth_client_id": null,
  "api_spec_url": null,
  "service_category": "connection",
  "requires_user_credential": true,
  "identity_propagation_mode": "none",
  "identity_include_user_id": false,
  "identity_include_email": false,
  "identity_include_name": false,
  "identity_jwt_audience": null,
  "created_by": "550e8400-e29b-41d4-a716-446655440000",
  "created_at": "2025-06-01T10:00:00+00:00",
  "updated_at": "2025-06-01T10:00:00+00:00"
}
```

For OIDC services, `oauth_client_id` will contain the auto-provisioned OAuth client ID and `service_category` will be `"provider"`.

**Errors:**
- `1002 forbidden` -- User is not an admin
- `1004 conflict` -- Slug already exists
- `1008 validation_error` -- Missing required fields, invalid auth_type, slug too long, or SSRF-blocked URL

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/services \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Analytics API",
    "slug": "analytics",
    "base_url": "https://analytics.example.com",
    "auth_type": "api_key",
    "credential": "secret-api-key"
  }'
```

---

#### GET /api/v1/services/{service_id}

Get a single downstream service by ID.

**Auth:** Required

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Response (200):**

```json
{
  "id": "d1e2f3a4-b5c6-7890-1234-567890abcdef",
  "name": "Internal Analytics API",
  "slug": "analytics",
  "description": "Company analytics service",
  "base_url": "https://analytics.example.com",
  "auth_method": "header",
  "auth_type": "api_key",
  "auth_key_name": "X-API-Key",
  "is_active": true,
  "oauth_client_id": null,
  "api_spec_url": null,
  "service_category": "connection",
  "requires_user_credential": true,
  "identity_propagation_mode": "none",
  "identity_include_user_id": false,
  "identity_include_email": false,
  "identity_include_name": false,
  "identity_jwt_audience": null,
  "created_by": "550e8400-e29b-41d4-a716-446655440000",
  "created_at": "2025-06-01T10:00:00+00:00",
  "updated_at": "2025-06-01T10:00:00+00:00"
}
```

**Errors:**
- `1003 not_found` -- Service does not exist

**Example:**

```bash
curl http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef \
  -H "Authorization: Bearer <access_token>"
```

---

#### PUT /api/v1/services/{service_id}

Update a downstream service. Only the provided fields are updated (partial update). If the service is an OIDC service and `base_url` is changed, the default redirect URI on the associated OAuth client is automatically updated.

**Auth:** Admin (or service creator)

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Request Body:**

| Field         | Type    | Required | Description                                     |
|---------------|---------|----------|-------------------------------------------------|
| `name`         | string  | No       | New display name (1-200 chars)                                          |
| `description`  | string  | No       | New description (max 500 chars)                                         |
| `base_url`     | string  | No       | New base URL (max 2048 chars, SSRF-validated)                           |
| `is_active`    | boolean | No       | Enable or disable the service                                           |
| `api_spec_url` | string  | No       | URL to an OpenAPI/Swagger spec for endpoint discovery (max 2048 chars)  |
| `identity_propagation_mode` | string | No | Identity propagation mode: `none` (default), `headers`, `jwt`, or `both` |
| `identity_include_user_id`  | boolean | No | Include `X-NyxID-User-Id` header when propagating identity |
| `identity_include_email`    | boolean | No | Include `X-NyxID-User-Email` header when propagating identity |
| `identity_include_name`     | boolean | No | Include `X-NyxID-User-Name` header when propagating identity |
| `identity_jwt_audience`     | string  | No | Custom JWT `aud` claim for identity assertions (defaults to service `base_url`) |

At least one field must be provided.

```json
{
  "name": "Updated Analytics API",
  "description": "Updated description",
  "base_url": "https://new-analytics.example.com",
  "api_spec_url": "https://analytics.example.com/openapi.json"
}
```

**Response (200):**

Returns the full updated service object (same shape as GET response).

**Errors:**
- `1002 forbidden` -- User is not admin and not the service creator
- `1003 not_found` -- Service does not exist
- `1008 validation_error` -- Name empty or too long, description too long, base_url too long or SSRF-blocked, or no fields provided

**Example:**

```bash
curl -X PUT http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{"name": "Updated Analytics API"}'
```

---

#### DELETE /api/v1/services/{service_id}

Deactivate a downstream service (soft delete). Only admins or the original service creator can perform this action.

**Auth:** Admin (or service creator)

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Response (200):**

```json
{
  "message": "Service deactivated"
}
```

**Errors:**
- `1002 forbidden` -- User is not admin and not the service creator
- `1003 not_found` -- Service does not exist

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### GET /api/v1/services/{service_id}/oidc-credentials

Retrieve the OIDC client credentials and discovery endpoints for a service configured with OIDC auth. The client secret is decrypted from storage and returned in plaintext.

**Auth:** Admin

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Response (200):**

```json
{
  "client_id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "client_secret": "nyx_secret_abc123...",
  "redirect_uris": ["https://portal.example.com/callback"],
  "allowed_scopes": "openid profile email",
  "issuer": "https://auth.example.com",
  "authorization_endpoint": "https://auth.example.com/oauth/authorize",
  "token_endpoint": "https://auth.example.com/oauth/token",
  "userinfo_endpoint": "https://auth.example.com/oauth/userinfo",
  "jwks_uri": "https://auth.example.com/.well-known/jwks.json"
}
```

**Errors:**
- `1000 bad_request` -- Service is not an OIDC service
- `1002 forbidden` -- User is not an admin
- `1003 not_found` -- Service does not exist

**Example:**

```bash
curl http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef/oidc-credentials \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### PUT /api/v1/services/{service_id}/redirect-uris

Update the redirect URIs for an OIDC service. Replaces the full set of redirect URIs on the associated OAuth client.

**Auth:** Admin

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Request Body:**

| Field           | Type     | Required | Description                                          |
|-----------------|----------|----------|------------------------------------------------------|
| `redirect_uris` | string[] | Yes      | Array of redirect URIs (1-10 items, max 2048 chars each, http/https only) |

```json
{
  "redirect_uris": [
    "https://portal.example.com/callback",
    "https://portal.example.com/auth/callback"
  ]
}
```

**Response (200):**

```json
{
  "redirect_uris": [
    "https://portal.example.com/callback",
    "https://portal.example.com/auth/callback"
  ]
}
```

**Errors:**
- `1000 bad_request` -- Service is not an OIDC service
- `1002 forbidden` -- User is not an admin
- `1003 not_found` -- Service does not exist
- `1008 validation_error` -- Empty array, more than 10 URIs, URI too long, or invalid URI scheme

**Example:**

```bash
curl -X PUT http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef/redirect-uris \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{"redirect_uris": ["https://portal.example.com/callback"]}'
```

---

#### POST /api/v1/services/{service_id}/regenerate-secret

Regenerate the OIDC client secret for a service. The previous secret is immediately invalidated. Store the new secret securely -- it cannot be retrieved again.

**Auth:** Admin

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Response (200):**

```json
{
  "client_secret": "nyx_secret_new_abc123...",
  "message": "Previous secret is now invalidated. Store this secret securely."
}
```

**Errors:**
- `1000 bad_request` -- Service is not an OIDC service
- `1002 forbidden` -- User is not an admin
- `1003 not_found` -- Service does not exist

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef/regenerate-secret \
  -H "Authorization: Bearer <admin_access_token>"
```

---

### Service Endpoints

Endpoints describe the individual API operations available on a downstream service. They are used by the MCP proxy to generate MCP tools, and can be created manually or auto-discovered from an OpenAPI spec.

Endpoint names must match `^[a-z][a-z0-9_]*$` (valid MCP tool names).

#### GET /api/v1/services/{service_id}/endpoints

List all active endpoints for a service.

**Auth:** Required

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Response (200):**

```json
{
  "endpoints": [
    {
      "id": "e1f2a3b4-c5d6-7890-abcd-ef1234567890",
      "service_id": "d1e2f3a4-b5c6-7890-1234-567890abcdef",
      "name": "list_customers",
      "description": "List all customers with pagination",
      "method": "GET",
      "path": "/v1/customers",
      "parameters": [
        {"name": "limit", "in": "query", "schema": {"type": "integer"}}
      ],
      "request_body_schema": null,
      "response_description": null,
      "is_active": true,
      "created_at": "2025-06-01T10:00:00+00:00",
      "updated_at": "2025-06-01T10:00:00+00:00"
    }
  ]
}
```

**Errors:**
- `1003 not_found` -- Service does not exist

**Example:**

```bash
curl http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef/endpoints \
  -H "Authorization: Bearer <access_token>"
```

---

#### POST /api/v1/services/{service_id}/endpoints

Create a new endpoint for a service.

**Auth:** Admin (or service creator)

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Request Body:**

| Field                  | Type   | Required | Description                                        |
|------------------------|--------|----------|----------------------------------------------------|
| `name`                 | string | Yes      | MCP tool name (1-100 chars, `^[a-z][a-z0-9_]*$`)  |
| `description`          | string | No       | Human-readable description                         |
| `method`               | string | Yes      | HTTP method: GET, POST, PUT, DELETE, PATCH         |
| `path`                 | string | Yes      | URL path starting with `/` (max 2048 chars)        |
| `parameters`           | JSON   | No       | OpenAPI-style parameter definitions                |
| `request_body_schema`  | JSON   | No       | JSON Schema for the request body                   |
| `response_description` | string | No       | Description of the expected response               |

```json
{
  "name": "list_customers",
  "description": "List all customers with pagination",
  "method": "GET",
  "path": "/v1/customers",
  "parameters": [
    {"name": "limit", "in": "query", "schema": {"type": "integer"}},
    {"name": "offset", "in": "query", "schema": {"type": "integer"}}
  ]
}
```

**Response (200):**

Returns the created endpoint object (same shape as list response items).

**Errors:**
- `1002 forbidden` -- User is not admin and not the service creator
- `1003 not_found` -- Service does not exist
- `1008 validation_error` -- Invalid name format, unsupported method, or path not starting with `/`
- `1007 database_error` -- Duplicate endpoint name for this service (unique constraint)

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef/endpoints \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "list_customers",
    "method": "GET",
    "path": "/v1/customers"
  }'
```

---

#### PUT /api/v1/services/{service_id}/endpoints/{endpoint_id}

Update an existing endpoint. Only the provided fields are updated (partial update).

**Auth:** Admin (or service creator)

**Path Parameters:**

| Parameter     | Type | Description      |
|---------------|------|------------------|
| `service_id`  | UUID | The service ID   |
| `endpoint_id` | UUID | The endpoint ID  |

**Request Body:**

| Field                  | Type    | Required | Description                                              |
|------------------------|---------|----------|----------------------------------------------------------|
| `name`                 | string  | No       | MCP tool name (1-100 chars, `^[a-z][a-z0-9_]*$`)        |
| `description`          | string? | No       | Human-readable description (null to clear)               |
| `method`               | string  | No       | HTTP method: GET, POST, PUT, DELETE, PATCH               |
| `path`                 | string  | No       | URL path starting with `/` (max 2048 chars)              |
| `parameters`           | JSON?   | No       | OpenAPI-style parameter definitions (null to clear)      |
| `request_body_schema`  | JSON?   | No       | JSON Schema for the request body (null to clear)         |
| `response_description` | string? | No       | Description of the expected response (null to clear)     |
| `is_active`            | boolean | No       | Enable or disable the endpoint                           |

**Response (200):**

```json
{
  "message": "Endpoint updated"
}
```

**Errors:**
- `1002 forbidden` -- User is not admin and not the service creator
- `1003 not_found` -- Service or endpoint does not exist
- `1008 validation_error` -- Invalid name, method, or path

**Example:**

```bash
curl -X PUT http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef/endpoints/e1f2a3b4-c5d6-7890-abcd-ef1234567890 \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{"description": "Updated description", "is_active": false}'
```

---

#### DELETE /api/v1/services/{service_id}/endpoints/{endpoint_id}

Permanently delete an endpoint.

**Auth:** Admin (or service creator)

**Path Parameters:**

| Parameter     | Type | Description      |
|---------------|------|------------------|
| `service_id`  | UUID | The service ID   |
| `endpoint_id` | UUID | The endpoint ID  |

**Response (200):**

```json
{
  "message": "Endpoint deleted"
}
```

**Errors:**
- `1002 forbidden` -- User is not admin and not the service creator
- `1003 not_found` -- Service or endpoint does not exist

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef/endpoints/e1f2a3b4-c5d6-7890-abcd-ef1234567890 \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### POST /api/v1/services/{service_id}/discover-endpoints

Fetch the service's `api_spec_url`, parse the OpenAPI/Swagger specification, and bulk upsert discovered endpoints. Existing endpoints matched by name are updated; new ones are created; endpoints not in the spec are soft-deleted (set `is_active = false`).

**Auth:** Admin (or service creator)

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Prerequisites:** The service must have `api_spec_url` set (via PUT /api/v1/services/{service_id}).

**Supported Specs:** OpenAPI 3.x and Swagger 2.0 in JSON format.

**Response (200):**

```json
{
  "message": "12 endpoints discovered and synced",
  "endpoints": [
    {
      "id": "e1f2a3b4-c5d6-7890-abcd-ef1234567890",
      "service_id": "d1e2f3a4-b5c6-7890-1234-567890abcdef",
      "name": "list_customers",
      "description": "List all customers",
      "method": "GET",
      "path": "/v1/customers",
      "parameters": [...],
      "request_body_schema": null,
      "response_description": null,
      "is_active": true,
      "created_at": "2025-06-01T10:00:00+00:00",
      "updated_at": "2025-06-01T12:00:00+00:00"
    }
  ]
}
```

**Errors:**
- `1000 bad_request` -- Service has no `api_spec_url`, spec fetch failed, invalid spec format, or spec is not JSON
- `1002 forbidden` -- User is not admin and not the service creator
- `1003 not_found` -- Service does not exist

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef/discover-endpoints \
  -H "Authorization: Bearer <admin_access_token>"
```

---

### MCP Config

#### GET /api/v1/mcp/config

Returns the MCP tool configuration for the authenticated user. Includes all services the user has valid connections to, along with their registered endpoints (tools) and the proxy base URL. Used by MCP clients to auto-configure available tools.

Services are only included if the user has a valid connection with satisfied credentials:
- For `connection` services: the user must have a stored encrypted credential.
- For `internal` services: an active connection record is sufficient.
- `provider` services are excluded (not proxyable).

**Auth:** Required

**Response (200):**

```json
{
  "user_id": "550e8400-e29b-41d4-a716-446655440000",
  "proxy_base_url": "https://auth.example.com/api/v1/proxy",
  "services": [
    {
      "service_id": "d1e2f3a4-b5c6-7890-1234-567890abcdef",
      "service_name": "Stripe API",
      "service_slug": "stripe",
      "description": "Payment processing",
      "base_url": "https://api.stripe.com",
      "service_category": "connection",
      "endpoints": [
        {
          "endpoint_id": "e1f2a3b4-c5d6-7890-abcd-ef1234567890",
          "name": "list_customers",
          "description": "List all customers with pagination",
          "method": "GET",
          "path": "/v1/customers",
          "parameters": [
            {"name": "limit", "in": "query", "schema": {"type": "integer"}}
          ],
          "request_body_schema": null,
          "response_description": null
        }
      ]
    }
  ],
  "total_services": 1,
  "total_endpoints": 1
}
```

If the user has no active connections or no valid credentials, `services` is an empty array and counts are `0`.

**Example:**

```bash
curl http://localhost:3001/api/v1/mcp/config \
  -H "Authorization: Bearer <access_token>"
```

---

### Service Connections

Connections allow individual users to associate themselves with downstream services. Services are divided into three categories:

- **provider** -- OIDC/SSO services where NyxID is the identity provider. Not user-connectable.
- **connection** -- External services that require per-user credentials (API keys, bearer tokens, basic auth).
- **internal** -- Services that use a master credential managed by the admin. Users just "enable" access.

When proxying requests, `connection` services use the per-user encrypted credential. `internal` services use the service-level master credential but require an active connection record.

#### GET /api/v1/connections

List all active service connections for the authenticated user.

**Auth:** Required

**Response (200):**

```json
{
  "connections": [
    {
      "service_id": "d1e2f3a4-b5c6-7890-1234-567890abcdef",
      "service_name": "Stripe API",
      "service_category": "connection",
      "auth_type": "api_key",
      "has_credential": true,
      "credential_label": "Production Key",
      "connected_at": "2025-06-01T10:00:00+00:00"
    },
    {
      "service_id": "a1b2c3d4-e5f6-7890-1234-567890abcdef",
      "service_name": "Internal Analytics",
      "service_category": "internal",
      "auth_type": "bearer",
      "has_credential": false,
      "credential_label": null,
      "connected_at": "2025-06-01T10:00:00+00:00"
    }
  ]
}
```

**Example:**

```bash
curl http://localhost:3001/api/v1/connections \
  -H "Authorization: Bearer <access_token>"
```

---

#### POST /api/v1/connections/{service_id}

Connect the authenticated user to a downstream service. For `connection` category services, a credential must be provided in the JSON body. For `internal` services, no credential is needed (omit `credential` or set to `null`). Provider services cannot be connected to.

**Auth:** Required

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Request Body:**

| Field              | Type   | Required | Description                                           |
|--------------------|--------|----------|-------------------------------------------------------|
| `credential`       | string | Depends  | Required for `connection` services. Must be absent/null for `internal` services. Max 8192 chars. |
| `credential_label` | string | No       | Optional label (e.g., "Production Key"). Max 200 chars. |

**Example (connection service):**

```json
{
  "credential": "sk-live-abc123...",
  "credential_label": "Production Key"
}
```

**Example (internal service):**

```json
{}
```

**Response (200):**

```json
{
  "service_id": "d1e2f3a4-b5c6-7890-1234-567890abcdef",
  "service_name": "Stripe API",
  "connected_at": "2025-06-01T10:00:00+00:00"
}
```

**Errors:**
- `1000 bad_request` -- Provider services are not connectable, or credential missing/unexpected for the service category
- `1003 not_found` -- Service does not exist or is inactive
- `1004 conflict` -- Already connected to this service
- `1008 validation_error` -- Credential empty, too long, or label too long

**Example:**

```bash
# Connect to a "connection" service with credentials
curl -X POST http://localhost:3001/api/v1/connections/d1e2f3a4-b5c6-7890-1234-567890abcdef \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{"credential": "sk-live-abc123", "credential_label": "Production Key"}'

# Connect to an "internal" service (no credential)
curl -X POST http://localhost:3001/api/v1/connections/a1b2c3d4-e5f6-7890-1234-567890abcdef \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{}'
```

---

#### PUT /api/v1/connections/{service_id}/credential

Update the credential on an existing connection. Only applicable to `connection` category services (those that require per-user credentials).

**Auth:** Required

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Request Body:**

| Field              | Type   | Required | Description                                  |
|--------------------|--------|----------|----------------------------------------------|
| `credential`       | string | Yes      | New credential value. Max 8192 chars.        |
| `credential_label` | string | No       | New label. When omitted, existing label is preserved. Max 200 chars. |

```json
{
  "credential": "sk-live-new-key-456...",
  "credential_label": "Rotated Production Key"
}
```

**Response (200):**

```json
{
  "message": "Credential updated"
}
```

**Errors:**
- `1000 bad_request` -- Service does not use per-user credentials
- `1003 not_found` -- No active connection found for this service
- `1008 validation_error` -- Credential empty, too long, or label too long

**Example:**

```bash
curl -X PUT http://localhost:3001/api/v1/connections/d1e2f3a4-b5c6-7890-1234-567890abcdef/credential \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{"credential": "sk-live-new-key-456"}'
```

---

#### DELETE /api/v1/connections/{service_id}

Disconnect the authenticated user from a downstream service. Securely clears all stored credential data (encrypted credential, credential type, credential label).

**Auth:** Required

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Response (200):**

```json
{
  "message": "Disconnected from service"
}
```

**Errors:**
- `1003 not_found` -- Connection does not exist

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/connections/d1e2f3a4-b5c6-7890-1234-567890abcdef \
  -H "Authorization: Bearer <access_token>"
```

---

### Service Provider Requirements

Service provider requirements define which external providers (e.g., OpenAI, Anthropic) a downstream service needs credentials from. When a user proxies a request to that service, NyxID resolves the user's provider tokens and injects them into the outbound request alongside the service credential.

#### GET /api/v1/services/{service_id}/requirements

List all provider requirements for a service.

**Auth:** Required

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Response (200):**

```json
{
  "requirements": [
    {
      "id": "r1a2b3c4-d5e6-7890-abcd-ef1234567890",
      "service_id": "d1e2f3a4-b5c6-7890-1234-567890abcdef",
      "provider_config_id": "p1a2b3c4-d5e6-7890-abcd-ef1234567890",
      "provider_name": "OpenAI",
      "provider_slug": "openai",
      "required": true,
      "scopes": null,
      "injection_method": "bearer",
      "injection_key": null,
      "created_at": "2025-06-01T10:00:00+00:00",
      "updated_at": "2025-06-01T10:00:00+00:00"
    }
  ]
}
```

**Errors:**
- `1003 not_found` -- Service does not exist

**Example:**

```bash
curl http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef/requirements \
  -H "Authorization: Bearer <access_token>"
```

---

#### POST /api/v1/services/{service_id}/requirements

Add a provider requirement to a service. The proxy will inject the user's token for this provider into outbound requests.

**Auth:** Admin

**Path Parameters:**

| Parameter    | Type | Description    |
|--------------|------|----------------|
| `service_id` | UUID | The service ID |

**Request Body:**

| Field                | Type     | Required | Description                                                              |
|----------------------|----------|----------|--------------------------------------------------------------------------|
| `provider_config_id` | string   | Yes      | ID of the provider configuration                                         |
| `required`           | boolean  | Yes      | If `true`, proxy fails when user has no token for this provider          |
| `scopes`             | string[] | No       | Specific OAuth scopes this service needs from the provider               |
| `injection_method`   | string   | Yes      | How to inject the token: `bearer`, `header`, or `query`                  |
| `injection_key`      | string   | No       | Header name or query param. Defaults: `Authorization` (bearer), `X-API-Key` (header), `api_key` (query) |

**Injection Method Defaults:**

| `injection_method` | Default `injection_key` | Behavior                              |
|---------------------|-------------------------|---------------------------------------|
| `bearer`            | `Authorization`         | Adds `Authorization: Bearer <token>`  |
| `header`            | `X-API-Key`             | Adds `<injection_key>: <token>`       |
| `query`             | `api_key`               | Appends `?<injection_key>=<token>`    |

**Blocked Injection Keys:** The following header names are blocked for security: `host`, `authorization`, `cookie`, `set-cookie`, `transfer-encoding`, `content-length`, `connection`, `x-forwarded-for`, `x-forwarded-host`, `x-real-ip`.

```json
{
  "provider_config_id": "p1a2b3c4-d5e6-7890-abcd-ef1234567890",
  "required": true,
  "injection_method": "bearer"
}
```

**Response (200):**

Returns the created requirement with provider details.

**Errors:**
- `1002 forbidden` -- User is not an admin
- `1003 not_found` -- Service or provider does not exist
- `1004 conflict` -- This provider requirement already exists for this service
- `1008 validation_error` -- Invalid injection_method or blocked injection_key

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef/requirements \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "provider_config_id": "p1a2b3c4-d5e6-7890-abcd-ef1234567890",
    "required": true,
    "injection_method": "bearer"
  }'
```

---

#### DELETE /api/v1/services/{service_id}/requirements/{requirement_id}

Remove a provider requirement from a service.

**Auth:** Admin

**Path Parameters:**

| Parameter        | Type | Description         |
|------------------|------|---------------------|
| `service_id`     | UUID | The service ID      |
| `requirement_id` | UUID | The requirement ID  |

**Response (200):**

```json
{
  "message": "Requirement removed"
}
```

**Errors:**
- `1002 forbidden` -- User is not an admin
- `1003 not_found` -- Requirement does not exist

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/services/d1e2f3a4-b5c6-7890-1234-567890abcdef/requirements/r1a2b3c4-d5e6-7890-abcd-ef1234567890 \
  -H "Authorization: Bearer <admin_access_token>"
```

---

### Providers

Providers represent external service providers (e.g., OpenAI, Anthropic, Google AI) that users can connect their credentials to. NyxID stores provider configurations centrally, and users connect by entering API keys or completing OAuth flows.

#### GET /api/v1/providers

List all active provider configurations.

**Auth:** Required

**Response (200):**

```json
{
  "providers": [
    {
      "id": "p1a2b3c4-d5e6-7890-abcd-ef1234567890",
      "slug": "openai",
      "name": "OpenAI",
      "description": "OpenAI API for GPT models",
      "provider_type": "api_key",
      "has_oauth_config": false,
      "default_scopes": null,
      "supports_pkce": false,
      "api_key_instructions": "Get your API key from https://platform.openai.com/api-keys",
      "api_key_url": "https://platform.openai.com/api-keys",
      "icon_url": "https://example.com/openai-icon.svg",
      "documentation_url": "https://platform.openai.com/docs",
      "is_active": true,
      "created_at": "2025-06-01T10:00:00+00:00",
      "updated_at": "2025-06-01T10:00:00+00:00"
    }
  ]
}
```

**Example:**

```bash
curl http://localhost:3001/api/v1/providers \
  -H "Authorization: Bearer <access_token>"
```

---

#### POST /api/v1/providers

Register a new provider configuration. OAuth2 providers require additional fields for the OAuth flow.

**Auth:** Admin

**Request Body:**

| Field               | Type     | Required | Description                                                          |
|---------------------|----------|----------|----------------------------------------------------------------------|
| `name`              | string   | Yes      | Display name (max 200 chars)                                         |
| `slug`              | string   | Yes      | URL-safe identifier (1-100 chars, lowercase alphanumeric + hyphens)  |
| `description`       | string   | No       | Provider description                                                 |
| `provider_type`     | string   | Yes      | `oauth2` or `api_key`                                                |
| `authorization_url` | string   | OAuth2   | OAuth2 authorization endpoint (required for `oauth2` type)           |
| `token_url`         | string   | OAuth2   | OAuth2 token endpoint (required for `oauth2` type)                   |
| `revocation_url`    | string   | No       | OAuth2 token revocation endpoint                                     |
| `default_scopes`    | string[] | No       | Default OAuth2 scopes to request                                     |
| `client_id`         | string   | OAuth2   | OAuth2 client ID (required for `oauth2` type, encrypted at rest)     |
| `client_secret`     | string   | OAuth2   | OAuth2 client secret (required for `oauth2` type, encrypted at rest) |
| `supports_pkce`     | boolean  | No       | Whether the provider supports PKCE (default: `false`)                |
| `api_key_instructions` | string | No      | Instructions for obtaining an API key (for `api_key` type)           |
| `api_key_url`       | string   | No       | URL where users can create API keys                                  |
| `icon_url`          | string   | No       | Provider icon/logo URL                                               |
| `documentation_url` | string   | No       | Provider documentation URL                                           |

**Slug Validation:** Must contain only lowercase letters, digits, and hyphens. No leading, trailing, or consecutive hyphens.

**Example (API key provider):**

```json
{
  "name": "OpenAI",
  "slug": "openai",
  "description": "OpenAI API for GPT models",
  "provider_type": "api_key",
  "api_key_instructions": "Get your API key from https://platform.openai.com/api-keys",
  "api_key_url": "https://platform.openai.com/api-keys",
  "icon_url": "https://example.com/openai-icon.svg",
  "documentation_url": "https://platform.openai.com/docs"
}
```

**Example (OAuth2 provider):**

```json
{
  "name": "Google AI",
  "slug": "google-ai",
  "provider_type": "oauth2",
  "authorization_url": "https://accounts.google.com/o/oauth2/v2/auth",
  "token_url": "https://oauth2.googleapis.com/token",
  "revocation_url": "https://oauth2.googleapis.com/revoke",
  "default_scopes": ["https://www.googleapis.com/auth/generative-language"],
  "client_id": "your-client-id.apps.googleusercontent.com",
  "client_secret": "your-client-secret",
  "supports_pkce": true
}
```

**Response (200):**

Returns the created provider (same shape as list response items, without encrypted fields).

**Errors:**
- `1002 forbidden` -- User is not an admin
- `1004 conflict` -- Slug already exists
- `1008 validation_error` -- Missing required fields, invalid provider_type, invalid slug, or SSRF-blocked URL

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/providers \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "OpenAI",
    "slug": "openai",
    "provider_type": "api_key",
    "api_key_url": "https://platform.openai.com/api-keys"
  }'
```

---

#### GET /api/v1/providers/{provider_id}

Get a single provider configuration by ID.

**Auth:** Required

**Path Parameters:**

| Parameter     | Type | Description     |
|---------------|------|-----------------|
| `provider_id` | UUID | The provider ID |

**Response (200):**

Returns a single provider object (same shape as list response items).

**Errors:**
- `1003 not_found` -- Provider does not exist

**Example:**

```bash
curl http://localhost:3001/api/v1/providers/p1a2b3c4-d5e6-7890-abcd-ef1234567890 \
  -H "Authorization: Bearer <access_token>"
```

---

#### PUT /api/v1/providers/{provider_id}

Update a provider configuration. Only the provided fields are updated (partial update).

**Auth:** Admin

**Path Parameters:**

| Parameter     | Type | Description     |
|---------------|------|-----------------|
| `provider_id` | UUID | The provider ID |

**Request Body:**

| Field               | Type     | Required | Description                                          |
|---------------------|----------|----------|------------------------------------------------------|
| `name`              | string   | No       | Display name                                         |
| `description`       | string   | No       | Provider description                                 |
| `is_active`         | boolean  | No       | Enable or disable the provider                       |
| `authorization_url` | string   | No       | OAuth2 authorization endpoint                        |
| `token_url`         | string   | No       | OAuth2 token endpoint                                |
| `revocation_url`    | string   | No       | OAuth2 revocation endpoint                           |
| `default_scopes`    | string[] | No       | Default OAuth2 scopes                                |
| `client_id`         | string   | No       | OAuth2 client ID (encrypted at rest)                 |
| `client_secret`     | string   | No       | OAuth2 client secret (encrypted at rest)             |
| `supports_pkce`     | boolean  | No       | PKCE support flag                                    |
| `api_key_instructions` | string | No      | Instructions for obtaining an API key                |
| `api_key_url`       | string   | No       | URL where users can create API keys                  |
| `icon_url`          | string   | No       | Provider icon/logo URL                               |
| `documentation_url` | string   | No       | Provider documentation URL                           |

**Response (200):**

Returns the updated provider object.

**Errors:**
- `1002 forbidden` -- User is not an admin
- `1003 not_found` -- Provider does not exist
- `1008 validation_error` -- SSRF-blocked URL

**Example:**

```bash
curl -X PUT http://localhost:3001/api/v1/providers/p1a2b3c4-d5e6-7890-abcd-ef1234567890 \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{"description": "Updated description", "is_active": true}'
```

---

#### DELETE /api/v1/providers/{provider_id}

Deactivate a provider and revoke all user tokens associated with it.

**Auth:** Admin

**Path Parameters:**

| Parameter     | Type | Description     |
|---------------|------|-----------------|
| `provider_id` | UUID | The provider ID |

**Response (200):**

```json
{
  "message": "Provider deactivated and user tokens revoked"
}
```

**Errors:**
- `1002 forbidden` -- User is not an admin
- `1003 not_found` -- Provider does not exist

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/providers/p1a2b3c4-d5e6-7890-abcd-ef1234567890 \
  -H "Authorization: Bearer <admin_access_token>"
```

---

### User Provider Tokens

Users connect to providers by submitting API keys or completing OAuth flows. These endpoints manage the user's provider token lifecycle.

#### GET /api/v1/providers/my-tokens

List all provider tokens for the authenticated user.

**Auth:** Required

**Response (200):**

```json
{
  "tokens": [
    {
      "provider_id": "p1a2b3c4-d5e6-7890-abcd-ef1234567890",
      "provider_name": "OpenAI",
      "provider_slug": "openai",
      "provider_type": "api_key",
      "status": "active",
      "label": "Production Key",
      "expires_at": null,
      "last_used_at": "2025-06-01T14:22:00+00:00",
      "connected_at": "2025-06-01T10:00:00+00:00"
    }
  ]
}
```

**Token Status Values:**

| Status           | Description                                              |
|------------------|----------------------------------------------------------|
| `active`         | Token is valid and ready for use                         |
| `expired`        | OAuth token has expired (will attempt lazy refresh)      |
| `revoked`        | User disconnected or admin deactivated the provider      |
| `refresh_failed` | OAuth token refresh failed (user must reconnect)         |

**Example:**

```bash
curl http://localhost:3001/api/v1/providers/my-tokens \
  -H "Authorization: Bearer <access_token>"
```

---

#### POST /api/v1/providers/{provider_id}/connect/api-key

Connect to an API key provider by submitting the key. The key is encrypted with AES-256-GCM before storage.

**Auth:** Required

**Path Parameters:**

| Parameter     | Type | Description     |
|---------------|------|-----------------|
| `provider_id` | UUID | The provider ID |

**Request Body:**

| Field     | Type   | Required | Description                            |
|-----------|--------|----------|----------------------------------------|
| `api_key` | string | Yes      | The API key (1-4096 characters)        |
| `label`   | string | No       | Human-readable label for the key       |

```json
{
  "api_key": "sk-proj-abc123...",
  "label": "Production Key"
}
```

**Response (200):**

```json
{
  "status": "connected",
  "message": "API key stored successfully"
}
```

**Errors:**
- `1003 not_found` -- Provider does not exist or is inactive
- `1008 validation_error` -- API key is empty or exceeds 4096 characters

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/providers/p1a2b3c4-d5e6-7890-abcd-ef1234567890/connect/api-key \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{"api_key": "sk-proj-abc123", "label": "My OpenAI Key"}'
```

---

#### GET /api/v1/providers/{provider_id}/connect/oauth

Initiate an OAuth2 connection flow with a provider. Returns the authorization URL that the user should be redirected to. Uses PKCE (S256) when the provider supports it.

**Auth:** Required

**Path Parameters:**

| Parameter     | Type | Description     |
|---------------|------|-----------------|
| `provider_id` | UUID | The provider ID |

**Response (200):**

```json
{
  "authorization_url": "https://accounts.google.com/o/oauth2/v2/auth?client_id=...&redirect_uri=...&code_challenge=...&state=..."
}
```

The frontend should redirect the user to this URL. After the user authorizes, the provider redirects back to NyxID's callback endpoint.

**Errors:**
- `1003 not_found` -- Provider does not exist, is inactive, or is not an OAuth2 provider

**Example:**

```bash
curl http://localhost:3001/api/v1/providers/p1a2b3c4-d5e6-7890-abcd-ef1234567890/connect/oauth \
  -H "Authorization: Bearer <access_token>"
```

---

#### GET /api/v1/providers/callback

Generic OAuth callback endpoint. Handles the redirect from OAuth providers after user authorization. Resolves the provider from the `state` parameter, verifies the session user matches, exchanges the code for tokens, and redirects to the frontend callback page.

**Auth:** Required (session cookie)

**Query Parameters:**

| Parameter           | Type   | Required | Description                                |
|---------------------|--------|----------|--------------------------------------------|
| `code`              | string | Yes      | Authorization code from the provider       |
| `state`             | string | Yes      | State parameter (maps to NyxID OAuth state)|
| `error`             | string | No       | Error code from the provider               |
| `error_description` | string | No       | Error description from the provider        |

**Response:** HTTP 302 redirect to `{FRONTEND_URL}/providers/callback?status=success` on success, or `?status=error&message=...` on failure.

This endpoint is not called directly by the frontend. It is the OAuth redirect URI registered with external providers.

---

#### DELETE /api/v1/providers/{provider_id}/disconnect

Disconnect from a provider. Sets the token status to "revoked" and clears encrypted credential data.

**Auth:** Required

**Path Parameters:**

| Parameter     | Type | Description     |
|---------------|------|-----------------|
| `provider_id` | UUID | The provider ID |

**Response (200):**

```json
{
  "status": "disconnected",
  "message": "Provider disconnected and credentials removed"
}
```

**Errors:**
- `1003 not_found` -- No token found for this provider

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/providers/p1a2b3c4-d5e6-7890-abcd-ef1234567890/disconnect \
  -H "Authorization: Bearer <access_token>"
```

---

#### POST /api/v1/providers/{provider_id}/refresh

Manually trigger a token refresh for an OAuth2 provider. For OAuth tokens, this triggers a lazy refresh if the token is within 5 minutes of expiry or already expired.

**Auth:** Required

**Path Parameters:**

| Parameter     | Type | Description     |
|---------------|------|-----------------|
| `provider_id` | UUID | The provider ID |

**Response (200):**

```json
{
  "status": "refreshed",
  "message": "Token refreshed successfully"
}
```

**Errors:**
- `1003 not_found` -- No active token for this provider

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/providers/p1a2b3c4-d5e6-7890-abcd-ef1234567890/refresh \
  -H "Authorization: Bearer <access_token>"
```

---

### Sessions

#### GET /api/v1/sessions

List all active (non-revoked, non-expired) sessions for the authenticated user. Sessions are returned in reverse chronological order.

**Auth:** Required

**Response (200):**

```json
[
  {
    "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
    "ip_address": "203.0.113.42",
    "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)...",
    "created_at": "2025-06-01T14:22:00+00:00",
    "expires_at": "2025-07-01T14:22:00+00:00"
  }
]
```

**Example:**

```bash
curl http://localhost:3001/api/v1/sessions \
  -H "Authorization: Bearer <access_token>"
```

---

### Proxy

#### ANY /api/v1/proxy/{service_id}/{*path}

Forward any HTTP request to a registered downstream service. NyxID resolves the service, verifies the user has an active connection, decrypts the appropriate credential, and injects it into the outbound request using the configured auth method.

**Connection enforcement:** An active `UserServiceConnection` is always required before proxying. For `connection` category services, the per-user encrypted credential is used. For `internal` category services, the service-level master credential is used. `provider` services are not proxyable.

**Path validation:** Paths containing `..` or `//` are rejected to prevent path traversal attacks.

**Auth:** Required

**Path Parameters:**

| Parameter    | Type   | Description                                    |
|--------------|--------|------------------------------------------------|
| `service_id` | UUID   | The downstream service ID                      |
| `*path`      | string | The path to forward (appended to service base URL) |

**Supported Methods:** GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS

**Request:** The request body, query parameters, and allowed headers are forwarded to the downstream service. Only safe headers are forwarded (content-type, accept, accept-language, accept-encoding, content-length, user-agent, x-request-id, x-correlation-id).

**Identity Propagation:** If the service has `identity_propagation_mode` set to `headers` or `both`, NyxID injects identity headers (`X-NyxID-User-Id`, `X-NyxID-User-Email`, `X-NyxID-User-Name`) based on the service configuration. If set to `jwt` or `both`, a short-lived RS256-signed identity assertion JWT is added as `X-NyxID-Identity-Token` (60-second lifetime).

**Credential Delegation:** If the service has provider requirements configured, NyxID resolves the user's provider tokens and injects them into the outbound request. Required provider tokens cause the request to fail if missing; optional tokens are silently skipped.

**Delegation Token Injection:** If the service has `inject_delegation_token: true`, NyxID generates a short-lived delegated access token (5-minute TTL) and injects it as the `X-NyxID-Delegation-Token` header. This allows the downstream service to call NyxID APIs (LLM gateway, proxy) on behalf of the user. The token can be refreshed via `POST /api/v1/delegation/refresh` for long-running workflows. See [Token Exchange (Delegated Access)](#token-exchange-delegated-access) for details.

**Response:** The downstream service's response status code, allowed headers, and body are returned directly. Only a safe allowlist of response headers is forwarded.

**Transaction Approval:** If the resource owner has `approval_required` enabled and the request uses a non-session auth method (API key, delegated token, service account, or access token), the proxy checks for an existing approval grant. If no grant exists, an approval request is created (with Telegram notification if configured) and the **HTTP connection is held open** until the user approves/rejects or the configured timeout expires. If approved, the request proceeds and the downstream response is returned. If rejected or timed out, a `403 Forbidden` is returned. Direct browser sessions (session cookie auth) bypass approval.

**Limits:** Request body is limited to 10 MB for proxy requests.

**Errors:**
- `1000 bad_request` -- Service is inactive, service is a provider, invalid proxy path, or connection missing credential
- `1002 forbidden` -- No active connection to this service, or approval was rejected/timed out
- `1003 not_found` -- Service does not exist

**Example:**

```bash
# GET request through proxy
curl http://localhost:3001/api/v1/proxy/d1e2f3a4-b5c6-7890-1234-567890abcdef/v1/reports \
  -H "Authorization: Bearer <access_token>"

# POST request through proxy
curl -X POST http://localhost:3001/api/v1/proxy/d1e2f3a4-b5c6-7890-1234-567890abcdef/v1/events \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{"event": "page_view", "page": "/home"}'
```

---

#### ANY /api/v1/proxy/s/{slug}/{*path}

Forward any HTTP request to a downstream service using the service's slug instead of its UUID. This endpoint is functionally identical to `ANY /api/v1/proxy/{service_id}/{*path}` but provides developer-friendly URLs.

The slug is resolved to the service UUID internally. All proxy behavior (credential injection, identity propagation, delegation token injection, transaction approval, path validation, header allowlists, body size limits) is the same as the UUID-based endpoint.

Only active services are resolved by slug. Inactive services return `1003 not_found`.

**Auth:** Required

**Path Parameters:**

| Parameter | Type   | Description                                        |
|-----------|--------|----------------------------------------------------|
| `slug`    | string | The service slug (e.g., `stripe`, `analytics`)     |
| `*path`   | string | The path to forward (appended to service base URL) |

**Supported Methods:** GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS

**Errors:**
- `1000 bad_request` -- Service is inactive, service is a provider, invalid proxy path, or connection missing credential
- `1002 forbidden` -- No active connection to this service
- `1003 not_found` -- No active service with this slug

**Example:**

```bash
# GET request via slug with API key
curl http://localhost:3001/api/v1/proxy/s/stripe/v1/charges \
  -H "X-API-Key: nyx_k_a1b2c3d4..."

# POST request via slug with Bearer token
curl -X POST http://localhost:3001/api/v1/proxy/s/analytics/v1/events \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{"event": "page_view", "page": "/home"}'
```

---

### Proxy Service Discovery

#### GET /api/v1/proxy/services

List downstream services available for proxying, with their connection status and proxy URLs. This endpoint is designed for developers integrating via API keys who need to discover available services and construct proxy URLs.

Services with `service_category = "provider"` are excluded (not proxyable). Only active services are returned.

**Auth:** Required

**Query Parameters:**

| Parameter  | Type   | Default | Description                                |
|------------|--------|---------|--------------------------------------------|
| `page`     | int    | 1       | Page number (minimum 1)                    |
| `per_page` | int    | 50      | Results per page (maximum 100)             |

**Response (200):**

```json
{
  "services": [
    {
      "id": "d1e2f3a4-b5c6-7890-1234-567890abcdef",
      "name": "Stripe API",
      "slug": "stripe",
      "description": "Payment processing",
      "service_category": "connection",
      "connected": true,
      "requires_connection": true,
      "proxy_url": "http://localhost:3001/api/v1/proxy/d1e2f3a4-b5c6-7890-1234-567890abcdef/{path}",
      "proxy_url_slug": "http://localhost:3001/api/v1/proxy/s/stripe/{path}"
    },
    {
      "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "name": "Internal Analytics",
      "slug": "analytics",
      "description": "Internal analytics service",
      "service_category": "internal",
      "connected": false,
      "requires_connection": false,
      "proxy_url": "http://localhost:3001/api/v1/proxy/a1b2c3d4-e5f6-7890-abcd-ef1234567890/{path}",
      "proxy_url_slug": "http://localhost:3001/api/v1/proxy/s/analytics/{path}"
    }
  ],
  "total": 2,
  "page": 1,
  "per_page": 50
}
```

**Response Fields:**

| Field                          | Type    | Description                                               |
|--------------------------------|---------|-----------------------------------------------------------|
| `services[].id`               | string  | Service UUID                                              |
| `services[].name`             | string  | Service display name                                      |
| `services[].slug`             | string  | Service slug (used in slug-based proxy URL)               |
| `services[].description`      | string? | Service description (nullable)                            |
| `services[].service_category` | string  | `"connection"` or `"internal"`                            |
| `services[].connected`        | bool    | Whether the authenticated user has an active connection   |
| `services[].requires_connection` | bool | Whether a connection is required before proxying          |
| `services[].proxy_url`        | string  | UUID-based proxy URL template (replace `{path}`)         |
| `services[].proxy_url_slug`   | string  | Slug-based proxy URL template (replace `{path}`)         |
| `total`                       | int     | Total number of matching services                         |
| `page`                        | int     | Current page number                                       |
| `per_page`                    | int     | Results per page                                          |

**Errors:**
- `1001 unauthorized` -- Missing or invalid credentials

**Example:**

```bash
# Discover services with API key
curl http://localhost:3001/api/v1/proxy/services \
  -H "X-API-Key: nyx_k_a1b2c3d4..."

# With pagination
curl "http://localhost:3001/api/v1/proxy/services?page=1&per_page=10" \
  -H "Authorization: Bearer <access_token>"
```

---

### LLM Gateway

The LLM Gateway provides unified access to multiple LLM providers through NyxID. Users connect their provider credentials (API keys or OAuth tokens) via the Providers endpoints, and the gateway handles routing, credential injection, and format translation.

Three access modes are available:
1. **Provider-specific proxy** -- Direct passthrough to a specific provider's API
2. **OpenAI-compatible gateway** -- Routes by model name and translates between API formats
3. **Status endpoint** -- Check which providers are ready for the current user

Streaming (`"stream": true`) is not yet supported. All requests must set `stream: false` or omit the field.

**Transaction Approval:** The same blocking approval flow applies to LLM gateway endpoints as to the proxy. If the resource owner has `approval_required` enabled and the request uses a non-session auth method (API key, delegated token, service account, or access token), the connection is held open until the user approves/rejects or the timeout expires. See the [Proxy](#proxy) section for details.

#### GET /api/v1/llm/status

Return which LLM providers the authenticated user can use, along with their proxy URLs.

**Auth:** Required

**Response (200):**

```json
{
  "providers": [
    {
      "provider_slug": "openai",
      "provider_name": "OpenAI",
      "status": "ready",
      "proxy_url": "http://localhost:3001/api/v1/llm/openai/v1"
    },
    {
      "provider_slug": "anthropic",
      "provider_name": "Anthropic",
      "status": "not_connected",
      "proxy_url": "http://localhost:3001/api/v1/llm/anthropic/v1"
    }
  ],
  "gateway_url": "http://localhost:3001/api/v1/llm/gateway/v1",
  "supported_models": [
    "gpt-*", "o1-*", "o3-*", "o4-*", "chatgpt-*",
    "claude-*", "gemini-*",
    "mistral-*", "codestral-*", "pixtral-*", "ministral-*", "open-mistral-*",
    "command-*", "embed-*", "rerank-*"
  ]
}
```

**Response Fields:**

| Field               | Type   | Description                                           |
|---------------------|--------|-------------------------------------------------------|
| `providers`         | array  | Per-provider status entries                           |
| `providers[].provider_slug` | string | Provider identifier (e.g., `openai`, `anthropic`) |
| `providers[].provider_name` | string | Display name                                    |
| `providers[].status` | string | `ready`, `not_connected`, or `expired`               |
| `providers[].proxy_url` | string | Direct proxy URL for this provider               |
| `gateway_url`       | string | OpenAI-compatible gateway URL                        |
| `supported_models`  | array  | Model name prefixes the gateway can route            |

**Example:**

```bash
curl http://localhost:3001/api/v1/llm/status \
  -H "Authorization: Bearer <access_token>"
```

---

#### ANY /api/v1/llm/{provider_slug}/v1/{*path}

Forward any HTTP request to a specific LLM provider's API. NyxID resolves the auto-seeded downstream service for the provider, injects the user's stored credential, and proxies the request. No request or response translation is applied -- the request is forwarded as-is.

**Auth:** Required

**Path Parameters:**

| Parameter       | Type   | Description                                         |
|-----------------|--------|-----------------------------------------------------|
| `provider_slug` | string | Provider identifier: `openai`, `openai-codex`, `anthropic`, `google-ai`, `mistral`, `cohere` |
| `*path`         | string | API path to forward (e.g., `chat/completions`)      |

**Supported Methods:** GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS

**Request:** The request body, query parameters, and allowed headers are forwarded to the provider's API. The user's stored credential is injected automatically using the provider's configured auth method.

**Response:** The provider's response status code, allowed headers, and body are returned directly.

**Limits:** Request body limited to 10 MB. Response body limited to 50 MB.

**Errors:**
- `1003 not_found` -- Provider slug not found
- `1000 bad_request` -- Provider credentials not available (user has not connected)
- `1000 bad_request` -- Streaming not yet supported
- `1006 internal_error` -- Auto-seeded LLM service not configured for provider

**Example (OpenAI):**

```bash
curl -X POST http://localhost:3001/api/v1/llm/openai/v1/chat/completions \
  -H "Authorization: Bearer <nyxid_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_tokens": 100
  }'
```

**Example (Anthropic native format):**

```bash
curl -X POST http://localhost:3001/api/v1/llm/anthropic/v1/messages \
  -H "Authorization: Bearer <nyxid_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-5-20250929",
    "messages": [{"role": "user", "content": "Hello"}],
    "max_tokens": 100
  }'
```

---

#### ANY /api/v1/llm/gateway/v1/{*path}

OpenAI-compatible gateway. Accepts requests in OpenAI chat completions format, determines the target provider from the `model` field, translates the request/response format if needed, and returns the result in OpenAI format.

**Auth:** Required

**Path Parameters:**

| Parameter | Type   | Description                                           |
|-----------|--------|-------------------------------------------------------|
| `*path`   | string | API path (typically `chat/completions`)               |

**Request Body:**

The request body must be valid JSON with a `model` field. The gateway uses the model name to determine which provider to route to.

| Field       | Type   | Required | Description                                        |
|-------------|--------|----------|----------------------------------------------------|
| `model`     | string | Yes      | Model name (determines routing)                    |
| `messages`  | array  | Yes      | Chat messages in OpenAI format                     |
| `max_tokens`| number | No       | Maximum tokens to generate (defaults to 4096 for Anthropic) |
| `temperature`| number| No       | Sampling temperature                               |
| `stream`    | boolean| No       | Must be `false` or omitted (streaming not yet supported) |

**Model-to-Provider Routing:**

| Model Prefix | Provider | Notes |
|-------------|----------|-------|
| `gpt-*`, `o1-*`, `o3-*`, `o4-*`, `chatgpt-*` | OpenAI | Falls back to OpenAI Codex if OpenAI not connected |
| `claude-*` | Anthropic | Request/response translated automatically |
| `gemini-*` | Google AI | Routed through Google's OpenAI-compatible endpoint |
| `mistral-*`, `codestral-*`, `pixtral-*`, `ministral-*`, `open-mistral-*` | Mistral | Native OpenAI-compatible format |
| `command-*`, `embed-*`, `rerank-*` | Cohere | Native format passthrough |

**Format Translation (Anthropic):**

When routing to Anthropic, the gateway automatically:
- Extracts `system` role messages into Anthropic's top-level `system` field
- Maps `stop` to `stop_sequences`
- Changes path from `chat/completions` to `messages`
- Adds `anthropic-version: 2023-06-01` header
- Translates the response back to OpenAI format (content, usage, finish_reason)

**Response (200):** OpenAI chat completion format regardless of the target provider.

```json
{
  "id": "chatcmpl-msg_01XFDUDYJgAACzvnptvVoYEL",
  "object": "chat.completion",
  "created": 1234567890,
  "model": "claude-sonnet-4-5-20250929",
  "choices": [{
    "index": 0,
    "message": {"role": "assistant", "content": "Hello! How can I help?"},
    "finish_reason": "stop"
  }],
  "usage": {
    "prompt_tokens": 25,
    "completion_tokens": 10,
    "total_tokens": 35
  }
}
```

**Error Response (gateway errors):**

When the upstream provider returns an error, the gateway wraps it in OpenAI error format:

```json
{
  "error": {
    "message": "Error message from upstream provider",
    "type": "gateway_error",
    "code": 400
  }
}
```

**Errors:**
- `1008 validation_error` -- Request body missing or `model` field not present
- `1000 bad_request` -- Unknown model (cannot determine provider), provider not connected, streaming not supported, or invalid JSON body

**Example (OpenAI model):**

```bash
curl -X POST http://localhost:3001/api/v1/llm/gateway/v1/chat/completions \
  -H "Authorization: Bearer <nyxid_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [
      {"role": "system", "content": "You are a helpful assistant."},
      {"role": "user", "content": "Hello"}
    ],
    "max_tokens": 100
  }'
```

**Example (Anthropic model via gateway):**

```bash
curl -X POST http://localhost:3001/api/v1/llm/gateway/v1/chat/completions \
  -H "Authorization: Bearer <nyxid_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "claude-sonnet-4-5-20250929",
    "messages": [
      {"role": "system", "content": "You are a helpful assistant."},
      {"role": "user", "content": "Hello"}
    ],
    "max_tokens": 1024
  }'
```

**Example (Google AI model via gateway):**

```bash
curl -X POST http://localhost:3001/api/v1/llm/gateway/v1/chat/completions \
  -H "Authorization: Bearer <nyxid_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gemini-2.0-flash",
    "messages": [
      {"role": "user", "content": "Hello"}
    ],
    "max_tokens": 100
  }'
```

---

### OAuth / OpenID Connect

NyxID implements the OpenID Connect Authorization Code flow with mandatory PKCE.

#### GET /oauth/authorize

Authorization endpoint. Validates the OAuth client and parameters, then issues an authorization code. Only `response_type=code` is supported. PKCE with `S256` method is required for all requests.

**Auth:** Required (the user must be logged in)

**Query Parameters:**

| Parameter               | Type   | Required | Description                              |
|-------------------------|--------|----------|------------------------------------------|
| `response_type`         | string | Yes      | Must be `code`                           |
| `client_id`             | string | Yes      | UUID of the registered OAuth client      |
| `redirect_uri`          | string | Yes      | Must match a registered redirect URI     |
| `scope`                 | string | No       | Space-separated scopes (default: `openid profile email`). Additional scopes: `roles` (include RBAC roles and permissions in tokens), `groups` (include group memberships in tokens) |
| `state`                 | string | No       | Opaque value for CSRF protection         |
| `code_challenge`        | string | Yes      | PKCE code challenge (base64url-encoded SHA-256) |
| `code_challenge_method` | string | No       | Must be `S256` if provided               |
| `nonce`                 | string | No       | Value included in ID token for replay protection |

**Response (200):**

```json
{
  "redirect_url": "https://app.example.com/callback?code=auth_code_here&state=xyz"
}
```

**Errors:**
- `1000 bad_request` -- Unsupported response_type, missing code_challenge, or unsupported method
- `3001 invalid_redirect_uri` -- Redirect URI not registered for this client
- `3002 invalid_scope` -- Requested scope not allowed for this client

**Example:**

```bash
curl -G http://localhost:3001/oauth/authorize \
  -H "Authorization: Bearer <access_token>" \
  --data-urlencode "response_type=code" \
  --data-urlencode "client_id=client-uuid-here" \
  --data-urlencode "redirect_uri=https://app.example.com/callback" \
  --data-urlencode "scope=openid profile email" \
  --data-urlencode "state=random-state-value" \
  --data-urlencode "code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM" \
  --data-urlencode "code_challenge_method=S256"
```

---

#### POST /oauth/token

Token endpoint. Exchanges an authorization code for access, refresh, and ID tokens. Also supports the `refresh_token` grant type and `urn:ietf:params:oauth:grant-type:token-exchange` for delegated access (see [Token Exchange](#token-exchange-delegated-access)).

**Auth:** None (client authenticates via `client_id` and optionally `client_secret`)

**Request Body (authorization_code grant):**

| Field           | Type   | Required | Description                              |
|-----------------|--------|----------|------------------------------------------|
| `grant_type`    | string | Yes      | `authorization_code`                     |
| `code`          | string | Yes      | The authorization code                   |
| `redirect_uri`  | string | Yes      | Must match the authorize request         |
| `client_id`     | string | Yes      | UUID of the OAuth client                 |
| `client_secret` | string | No       | Required for confidential clients        |
| `code_verifier` | string | No       | PKCE code verifier (required if PKCE used)|

**Request Body (refresh_token grant):**

| Field           | Type   | Required | Description                              |
|-----------------|--------|----------|------------------------------------------|
| `grant_type`    | string | Yes      | `refresh_token`                          |
| `refresh_token` | string | Yes      | A valid refresh token                    |

**Response (200):**

```json
{
  "access_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...",
  "token_type": "Bearer",
  "expires_in": 900,
  "refresh_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...",
  "id_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...",
  "scope": "openid profile email"
}
```

**ID Token Claims:**

| Claim            | Type    | Description                        |
|------------------|---------|------------------------------------|
| `sub`            | string  | User ID (UUID)                     |
| `iss`            | string  | Issuer (matches `JWT_ISSUER`)      |
| `aud`            | string  | Client ID                          |
| `exp`            | integer | Expiration (Unix timestamp)        |
| `iat`            | integer | Issued at (Unix timestamp)         |
| `email`          | string  | User email address                 |
| `email_verified` | boolean | Whether email is verified          |
| `name`           | string  | User display name                  |
| `picture`        | string  | User avatar URL                    |
| `nonce`          | string  | Echoed from authorize request      |

**Errors:**
- `1000 bad_request` -- Missing parameters, unsupported grant_type
- `3000 pkce_verification_failed` -- Code verifier does not match challenge

**Example:**

```bash
curl -X POST http://localhost:3001/oauth/token \
  -H "Content-Type: application/json" \
  -d '{
    "grant_type": "authorization_code",
    "code": "auth_code_here",
    "redirect_uri": "https://app.example.com/callback",
    "client_id": "client-uuid-here",
    "code_verifier": "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"
  }'
```

---

#### GET /oauth/userinfo

OpenID Connect UserInfo endpoint. Returns claims about the authenticated user. When the access token's scope includes `roles` or `groups`, the response includes RBAC claims.

**Auth:** Required (Bearer token issued by the `/oauth/token` endpoint)

**Response (200):**

```json
{
  "sub": "550e8400-e29b-41d4-a716-446655440000",
  "email": "user@example.com",
  "email_verified": true,
  "name": "Jane Doe",
  "picture": "https://example.com/avatar.jpg",
  "roles": ["admin", "editor"],
  "groups": ["engineering"],
  "permissions": ["users:read", "users:write", "content:read", "content:write"]
}
```

The `roles`, `groups`, and `permissions` fields are only present when the corresponding scopes (`roles`, `groups`) were requested during authorization.

**Example:**

```bash
curl http://localhost:3001/oauth/userinfo \
  -H "Authorization: Bearer <access_token>"
```

---

### Token Introspection

#### POST /oauth/introspect

Token introspection endpoint per [RFC 7662](https://tools.ietf.org/html/rfc7662). Validates a token and returns its active status with associated claims. The request body is `application/x-www-form-urlencoded`.

**Auth:** None (client authenticates via `client_id` and `client_secret` in the request body)

**Request Body (form-encoded):**

| Field             | Type   | Required | Description                              |
|-------------------|--------|----------|------------------------------------------|
| `token`           | string | Yes      | The token to introspect                  |
| `token_type_hint` | string | No       | `access_token` or `refresh_token`        |
| `client_id`       | string | Yes      | OAuth client ID                          |
| `client_secret`   | string | No       | OAuth client secret (required for confidential clients) |

**Response (200) -- Active token:**

```json
{
  "active": true,
  "scope": "openid profile email roles",
  "username": "user@example.com",
  "token_type": "access",
  "exp": 1717200000,
  "iat": 1717199100,
  "sub": "550e8400-e29b-41d4-a716-446655440000",
  "iss": "nyxid",
  "jti": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "roles": ["admin", "editor"],
  "groups": ["engineering"],
  "permissions": ["users:read", "users:write"]
}
```

**Response (200) -- Inactive/invalid token:**

```json
{
  "active": false
}
```

The endpoint always returns 200. Invalid tokens, expired tokens, revoked tokens, and requests with invalid client credentials all return `{"active": false}`.

**Example:**

```bash
curl -X POST http://localhost:3001/oauth/introspect \
  -d "token=eyJhbGciOiJSUzI1NiIs..." \
  -d "client_id=client-uuid-here" \
  -d "client_secret=client-secret-here"
```

---

### Token Revocation

#### POST /oauth/revoke

Token revocation endpoint per [RFC 7009](https://tools.ietf.org/html/rfc7009). Revokes a token so it can no longer be used. The request body is `application/x-www-form-urlencoded`.

**Auth:** None (client authenticates via `client_id` and `client_secret` in the request body)

**Request Body (form-encoded):**

| Field             | Type   | Required | Description                              |
|-------------------|--------|----------|------------------------------------------|
| `token`           | string | Yes      | The token to revoke                      |
| `token_type_hint` | string | No       | `access_token` or `refresh_token`        |
| `client_id`       | string | Yes      | OAuth client ID                          |
| `client_secret`   | string | No       | OAuth client secret (required for confidential clients) |

**Response:** Always returns `200 OK` with an empty body, per RFC 7009. This applies even if the token is invalid, already revoked, or client authentication fails.

Refresh tokens are revoked in the database (marked `revoked = true`). Access tokens are stateless JWTs and cannot be explicitly revoked; they expire naturally (default: 15 minutes).

**Example:**

```bash
curl -X POST http://localhost:3001/oauth/revoke \
  -d "token=eyJhbGciOiJSUzI1NiIs..." \
  -d "client_id=client-uuid-here" \
  -d "client_secret=client-secret-here"
```

---

### Token Exchange (Delegated Access)

NyxID supports [RFC 8693 OAuth 2.0 Token Exchange](https://tools.ietf.org/html/rfc8693) to enable downstream services to make API calls on behalf of users. This is used when a downstream service (registered as an OIDC client) needs to call NyxID's LLM gateway or proxy endpoints using the user's credentials.

#### POST /oauth/token (token-exchange grant)

Exchange a user's access token for a short-lived delegated access token. The delegated token can be used as a Bearer token at NyxID's proxy and LLM gateway endpoints.

**Auth:** None (client authenticates via `client_id` and `client_secret` in the request body)

**Preconditions:**
1. The downstream service must be registered as a **confidential** OAuth client with `delegation_scopes` configured
2. The user must have an existing consent record for the client (auto-created during OIDC login)
3. The subject token must be a valid, non-expired, non-delegated NyxID access token

**Request Body (form-encoded):**

| Field                | Type   | Required | Description                                                |
|----------------------|--------|----------|------------------------------------------------------------|
| `grant_type`         | string | Yes      | `urn:ietf:params:oauth:grant-type:token-exchange`          |
| `client_id`          | string | Yes      | The downstream service's OAuth client ID                   |
| `client_secret`      | string | Yes      | The downstream service's OAuth client secret               |
| `subject_token`      | string | Yes      | The user's NyxID access token                              |
| `subject_token_type` | string | Yes      | Must be `urn:ietf:params:oauth:token-type:access_token`    |
| `scope`              | string | No       | Requested delegation scopes (default: `llm:proxy`)         |

**Available Delegation Scopes:**

| Scope              | Access                                                          |
|--------------------|-----------------------------------------------------------------|
| `llm:proxy`        | LLM gateway and provider-specific proxy endpoints               |
| `proxy:*`          | All proxy endpoints (`/api/v1/proxy/{service_id}/{*path}`)      |
| `proxy:{service_id}` | A specific service's proxy endpoint                           |
| `llm:status`       | Read-only access to LLM status endpoint                        |

**Response (200):**

```json
{
  "access_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...",
  "token_type": "Bearer",
  "expires_in": 300,
  "scope": "llm:proxy"
}
```

**Delegated Token JWT Claims:**

| Claim       | Type    | Description                                        |
|-------------|---------|----------------------------------------------------|
| `sub`       | string  | User ID (the user being acted on behalf of)        |
| `iss`       | string  | NyxID issuer                                       |
| `aud`       | string  | NyxID base URL                                     |
| `exp`       | integer | Expiration (5 minutes from issuance)               |
| `iat`       | integer | Issued at                                          |
| `jti`       | string  | Unique token ID                                    |
| `scope`     | string  | Constrained delegation scopes                      |
| `token_type`| string  | `"access"`                                         |
| `act.sub`   | string  | OAuth client ID of the acting service (RFC 8693)   |
| `delegated` | boolean | `true` (distinguishes from direct user tokens)     |

**Errors:**
- `1000 bad_request` -- Missing required parameters, invalid `subject_token_type`, subject token is not an access token, or subject token is itself a delegated token (chained exchange rejected)
- `1001 unauthorized` -- Invalid client credentials or invalid/expired subject token
- `1002 forbidden` -- User has not consented to delegation for this client, or token exchange is not enabled for this client (empty `delegation_scopes`)
- `3002 invalid_scope` -- Requested scope is not in the client's `delegation_scopes`

**Example:**

```bash
curl -X POST http://localhost:3001/oauth/token \
  -d "grant_type=urn:ietf:params:oauth:grant-type:token-exchange" \
  -d "client_id=downstream-client-id" \
  -d "client_secret=downstream-client-secret" \
  -d "subject_token=eyJhbGciOiJSUzI1NiIs..." \
  -d "subject_token_type=urn:ietf:params:oauth:token-type:access_token" \
  -d "scope=llm:proxy"
```

**Using the Delegated Token:**

```bash
# Use the delegated token to call NyxID's LLM gateway on behalf of the user
curl -X POST http://localhost:3001/api/v1/llm/gateway/v1/chat/completions \
  -H "Authorization: Bearer <delegated_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "model": "gpt-4o",
    "messages": [{"role": "user", "content": "Hello"}]
  }'
```

**Endpoint Access Restrictions:**

Delegated tokens are restricted to proxy and LLM gateway endpoints only. All other endpoints (auth, users, API keys, services, admin, MFA, etc.) reject delegated tokens with `403 Forbidden`.

| Endpoint Group                   | Delegated Token Access |
|----------------------------------|------------------------|
| `/api/v1/llm/*`                  | Allowed                |
| `/api/v1/proxy/{id}/{*path}`     | Allowed                |
| `/api/v1/delegation/refresh`     | Allowed (required)     |
| All other `/api/v1/*`            | Blocked                |

---

#### POST /api/v1/delegation/refresh

Refresh a delegated access token. Issues a new delegation token with the same `act.sub` and scope but a fresh 5-minute TTL. Only accepts delegated tokens -- regular user tokens are rejected with 403.

This endpoint is critical for agentic/long-running workflows where a downstream service needs to make API calls over an extended period.

**Auth:** Required (delegated Bearer token only)

**Request:** No request body. The current delegation token is provided via the `Authorization: Bearer` header.

**Response (200):**

```json
{
  "access_token": "eyJhbGciOiJSUzI1NiIsInR5cCI6IkpXVCJ9...",
  "token_type": "Bearer",
  "expires_in": 300,
  "scope": "llm:proxy"
}
```

**Security:**
- Validates the user still exists and is active
- Validates the user still has active consent for the acting client (prevents indefinite refresh after consent revocation)

**Errors:**
- `1001 unauthorized` -- Invalid/expired token, or user account is inactive or not found
- `1002 forbidden` -- Token is not a delegated token (missing `act.sub`), or user consent has been revoked for the acting client

**Example:**

```bash
# Refresh a delegation token before it expires
curl -X POST http://localhost:3001/api/v1/delegation/refresh \
  -H "Authorization: Bearer <current_delegation_token>"
```

---

#### MCP Delegation Token Injection

When NyxID proxies an MCP tool call or REST proxy request to a downstream service that has `inject_delegation_token: true`, it automatically generates and injects a delegation token as the `X-NyxID-Delegation-Token` header.

This allows the downstream service to use the token as a Bearer token when calling back to NyxID's API endpoints (e.g., LLM gateway) on behalf of the user.

**Injected Header:**

| Header                          | Value                                    |
|---------------------------------|------------------------------------------|
| `X-NyxID-Delegation-Token`     | Short-lived delegated NyxID access JWT   |

**MCP-Injected Token Properties:**

| Property   | Value                                                         |
|------------|---------------------------------------------------------------|
| TTL        | 5 minutes (refreshable via `POST /api/v1/delegation/refresh`) |
| Scope      | Configurable per-service via `delegation_token_scope` (default: `llm:proxy`) |
| `act.sub`  | Service slug of the downstream service                        |
| `delegated`| `true`                                                        |

**Configuration (on the downstream service):**

| Field                      | Type    | Default      | Description                                  |
|----------------------------|---------|--------------|----------------------------------------------|
| `inject_delegation_token`  | boolean | `false`      | Whether to inject delegation tokens          |
| `delegation_token_scope`   | string  | `"llm:proxy"`| Space-separated scopes for the injected token|

**Downstream Service Usage Example (Python):**

```python
from flask import request
import requests

@app.route("/api/analyze", methods=["POST"])
def analyze():
    delegation_token = request.headers.get("X-NyxID-Delegation-Token")
    if not delegation_token:
        return {"error": "No delegation token"}, 400

    # Call NyxID's LLM gateway on behalf of the user
    response = requests.post(
        "https://nyx.example.com/api/v1/llm/gateway/v1/chat/completions",
        headers={
            "Authorization": f"Bearer {delegation_token}",
            "Content-Type": "application/json",
        },
        json={
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "Analyze this data..."}],
        },
    )
    return response.json()
```

---

### User Consents

#### GET /api/v1/users/me/consents

List all OAuth consents the current user has granted to third-party applications.

**Auth:** Required

**Response (200):**

```json
{
  "consents": [
    {
      "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "client_id": "client-uuid-here",
      "client_name": "My Web App",
      "scopes": "openid profile email",
      "granted_at": "2025-06-01T10:00:00+00:00",
      "expires_at": null
    }
  ]
}
```

**Example:**

```bash
curl http://localhost:3001/api/v1/users/me/consents \
  -H "Authorization: Bearer <access_token>"
```

---

#### DELETE /api/v1/users/me/consents/{client_id}

Revoke an OAuth consent for a specific client. The user will be prompted to re-authorize on the next OAuth flow with this client.

**Auth:** Required

**Path Parameters:**

| Parameter   | Type   | Description             |
|-------------|--------|-------------------------|
| `client_id` | string | The OAuth client ID     |

**Response (200):**

```json
{
  "message": "Consent revoked"
}
```

**Errors:**
- `1003 not_found` -- No consent found for this client

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/users/me/consents/client-uuid-here \
  -H "Authorization: Bearer <access_token>"
```

---

### OIDC Discovery

These endpoints are public and do not require authentication. They allow relying parties (downstream services using OIDC) to automatically discover NyxID's provider configuration and verify JWT signatures.

#### GET /.well-known/openid-configuration

Returns the OpenID Connect Provider metadata document. Relying parties use this to auto-configure authorization, token, and userinfo endpoint URLs.

**Auth:** None

**Response (200):**

```json
{
  "issuer": "nyxid",
  "authorization_endpoint": "https://auth.example.com/oauth/authorize",
  "token_endpoint": "https://auth.example.com/oauth/token",
  "userinfo_endpoint": "https://auth.example.com/oauth/userinfo",
  "jwks_uri": "https://auth.example.com/.well-known/jwks.json",
  "response_types_supported": ["code"],
  "grant_types_supported": ["authorization_code", "refresh_token"],
  "subject_types_supported": ["public"],
  "id_token_signing_alg_values_supported": ["RS256"],
  "scopes_supported": ["openid", "profile", "email"],
  "claims_supported": [
    "sub", "iss", "aud", "exp", "iat",
    "email", "email_verified", "name", "picture", "nonce"
  ],
  "code_challenge_methods_supported": ["S256"],
  "token_endpoint_auth_methods_supported": ["client_secret_post", "none"]
}
```

**Example:**

```bash
curl https://auth.example.com/.well-known/openid-configuration
```

---

#### GET /.well-known/jwks.json

Returns the JSON Web Key Set (JWKS) containing the public key(s) used to sign JWTs. Relying parties use this to verify token signatures without needing a shared secret.

**Auth:** None

**Response (200):**

```json
{
  "keys": [
    {
      "kty": "RSA",
      "use": "sig",
      "alg": "RS256",
      "n": "<base64url-encoded modulus>",
      "e": "AQAB",
      "kid": "<key-id>"
    }
  ]
}
```

**Example:**

```bash
curl https://auth.example.com/.well-known/jwks.json
```

---

### MFA (Multi-Factor Authentication)

#### POST /api/v1/mfa/setup

Begin TOTP MFA enrollment. Returns a TOTP secret and a QR code provisioning URL.

**Auth:** Required

**Response (200):**

```json
{
  "secret": "JBSWY3DPEHPK3PXP",
  "qr_url": "otpauth://totp/NyxID:user@example.com?secret=JBSWY3DPEHPK3PXP&issuer=NyxID"
}
```

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/mfa/setup \
  -H "Authorization: Bearer <access_token>"
```

---

#### POST /api/v1/mfa/verify-setup

Complete MFA enrollment by verifying a TOTP code. On success, MFA is enabled on the user account and recovery codes are returned.

**Auth:** Required

**Request Body:**

| Field  | Type   | Required | Description                           |
|--------|--------|----------|---------------------------------------|
| `code` | string | Yes      | 6-digit TOTP code from authenticator  |

```json
{
  "code": "123456"
}
```

**Response (200):**

```json
{
  "message": "MFA enabled successfully",
  "recovery_codes": [
    "ABCD-1234-EFGH",
    "IJKL-5678-MNOP",
    "QRST-9012-UVWX"
  ]
}
```

**Errors:**
- `2000 authentication_failed` -- Invalid TOTP code
- `1003 not_found` -- No pending MFA factor found

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/mfa/verify-setup \
  -H "Authorization: Bearer <access_token>" \
  -H "Content-Type: application/json" \
  -d '{"code": "123456"}'
```

---

### Admin

All admin endpoints require the authenticated user to have `is_admin = true`. Admin endpoints include self-protection: admins cannot change their own role, disable themselves, or delete themselves.

#### GET /api/v1/admin/users

List all users with pagination and optional email search.

**Auth:** Admin

**Query Parameters:**

| Parameter  | Type    | Default | Description                          |
|------------|---------|---------|--------------------------------------|
| `page`     | integer | `1`     | Page number (1-indexed)              |
| `per_page` | integer | `50`    | Items per page (max 100)             |
| `search`   | string  | --      | Case-insensitive email search filter |

**Response (200):**

```json
{
  "users": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "email": "user@example.com",
      "display_name": "Jane Doe",
      "avatar_url": null,
      "email_verified": true,
      "is_active": true,
      "is_admin": false,
      "mfa_enabled": true,
      "created_at": "2025-01-15T10:30:00+00:00",
      "last_login_at": "2025-06-01T14:22:00+00:00"
    }
  ],
  "total": 142,
  "page": 1,
  "per_page": 50
}
```

**Example:**

```bash
# List users
curl "http://localhost:3001/api/v1/admin/users?page=1&per_page=25" \
  -H "Authorization: Bearer <admin_access_token>"

# Search by email
curl "http://localhost:3001/api/v1/admin/users?search=jane" \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### POST /api/v1/admin/users

Create a new user. Admin-created accounts are pre-verified (`email_verified: true`) and active (`is_active: true`).

**Auth:** Admin

**Request Body:**

| Field          | Type   | Required | Description                                      |
|----------------|--------|----------|--------------------------------------------------|
| `email`        | string | Yes      | User email address                               |
| `password`     | string | Yes      | Password (8-128 characters)                      |
| `display_name` | string | No       | Display name (max 200 characters)                |
| `role`         | string | Yes      | `"admin"` or `"user"`                            |

**Response (200):**

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "email": "newuser@example.com",
  "display_name": "Jane Doe",
  "is_admin": false,
  "is_active": true,
  "email_verified": true,
  "created_at": "2025-06-15T10:30:00+00:00",
  "message": "User created successfully"
}
```

**Errors:**
- `1004 conflict` -- Email already in use
- `1008 validation_error` -- Invalid email, password too short/long, or invalid role

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/admin/users \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "email": "newuser@example.com",
    "password": "securepassword123",
    "display_name": "Jane Doe",
    "role": "user"
  }'
```

---

#### GET /api/v1/admin/users/{user_id}

Get detailed information about a specific user.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `user_id` | UUID | The user ID |

**Response (200):**

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "email": "user@example.com",
  "display_name": "Jane Doe",
  "avatar_url": null,
  "email_verified": true,
  "is_active": true,
  "is_admin": false,
  "mfa_enabled": true,
  "created_at": "2025-01-15T10:30:00+00:00",
  "last_login_at": "2025-06-01T14:22:00+00:00"
}
```

**Errors:**
- `1003 not_found` -- User does not exist

**Example:**

```bash
curl http://localhost:3001/api/v1/admin/users/550e8400-e29b-41d4-a716-446655440000 \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### PUT /api/v1/admin/users/{user_id}

Edit a user's profile fields. Only provided fields are updated.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `user_id` | UUID | The user ID |

**Request Body:**

| Field          | Type   | Required | Description                                |
|----------------|--------|----------|--------------------------------------------|
| `display_name` | string | No       | New display name (max 200 chars)           |
| `email`        | string | No       | New email (validated, unique check)        |
| `avatar_url`   | string | No       | New avatar URL (must use https://, max 2048 chars) |

```json
{
  "display_name": "Jane Smith",
  "email": "jane.smith@example.com"
}
```

**Response (200):**

Returns the updated user object (same shape as GET response).

**Errors:**
- `1003 not_found` -- User does not exist
- `1008 validation_error` -- Invalid email format, email already in use, display name too long, or invalid avatar URL

**Example:**

```bash
curl -X PUT http://localhost:3001/api/v1/admin/users/550e8400-e29b-41d4-a716-446655440000 \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{"display_name": "Jane Smith"}'
```

---

#### PATCH /api/v1/admin/users/{user_id}/role

Toggle admin role for a user. Self-protection: an admin cannot change their own role.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `user_id` | UUID | The user ID |

**Request Body:**

| Field      | Type    | Required | Description           |
|------------|---------|----------|-----------------------|
| `is_admin` | boolean | Yes      | New admin role status |

```json
{
  "is_admin": true
}
```

**Response (200):**

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "is_admin": true,
  "message": "User admin role updated"
}
```

**Errors:**
- `1003 not_found` -- User does not exist
- `1008 validation_error` -- Cannot change your own admin role

**Example:**

```bash
curl -X PATCH http://localhost:3001/api/v1/admin/users/550e8400-e29b-41d4-a716-446655440000/role \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{"is_admin": true}'
```

---

#### PATCH /api/v1/admin/users/{user_id}/status

Enable or disable a user account. Self-protection: an admin cannot change their own status. When disabling a user, all their sessions are revoked, all refresh tokens are invalidated, and all API keys are deactivated, effectively locking them out immediately (except for any in-flight JWT access tokens, which expire within 15 minutes).

**Auth:** Admin

**Path Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `user_id` | UUID | The user ID |

**Request Body:**

| Field       | Type    | Required | Description               |
|-------------|---------|----------|---------------------------|
| `is_active` | boolean | Yes      | New active status         |

```json
{
  "is_active": false
}
```

**Response (200):**

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "is_active": false,
  "message": "User status updated"
}
```

**Errors:**
- `1003 not_found` -- User does not exist
- `1008 validation_error` -- Cannot change your own active status

**Example:**

```bash
curl -X PATCH http://localhost:3001/api/v1/admin/users/550e8400-e29b-41d4-a716-446655440000/status \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{"is_active": false}'
```

---

#### POST /api/v1/admin/users/{user_id}/reset-password

Force a password reset for a user. Generates a reset token and revokes all existing sessions. Does not work for social login only accounts (no password set).

**Auth:** Admin

**Path Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `user_id` | UUID | The user ID |

**Response (200):**

```json
{
  "message": "Password reset initiated"
}
```

**Errors:**
- `1000 bad_request` -- User has no password (social login only)
- `1003 not_found` -- User does not exist

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/admin/users/550e8400-e29b-41d4-a716-446655440000/reset-password \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### DELETE /api/v1/admin/users/{user_id}

Delete a user and cascade-delete all related data. Self-protection: an admin cannot delete themselves. Audit log entries referencing the deleted user are preserved (orphaned reference).

**Cascade delete** removes documents from 8 collections:
- `sessions`
- `refresh_tokens`
- `api_keys`
- `user_service_connections`
- `user_provider_tokens`
- `mfa_factors`
- `authorization_codes`
- `oauth_states`

The deletion follows a two-phase approach: the user is first marked inactive (preventing authentication during cleanup), then related documents are deleted, and finally the user document itself is removed.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `user_id` | UUID | The user ID |

**Response (200):**

```json
{
  "message": "User deleted"
}
```

**Errors:**
- `1003 not_found` -- User does not exist
- `1008 validation_error` -- Cannot delete yourself

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/admin/users/550e8400-e29b-41d4-a716-446655440000 \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### PATCH /api/v1/admin/users/{user_id}/verify-email

Manually verify a user's email address. Clears any pending verification token.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `user_id` | UUID | The user ID |

**Response (200):**

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "email_verified": true,
  "message": "Email verified"
}
```

**Errors:**
- `1000 bad_request` -- Email already verified
- `1003 not_found` -- User does not exist

**Example:**

```bash
curl -X PATCH http://localhost:3001/api/v1/admin/users/550e8400-e29b-41d4-a716-446655440000/verify-email \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### GET /api/v1/admin/users/{user_id}/sessions

List all sessions for a user (including revoked and expired), sorted by creation time descending.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `user_id` | UUID | The user ID |

**Response (200):**

```json
{
  "sessions": [
    {
      "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "ip_address": "203.0.113.42",
      "user_agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7)...",
      "created_at": "2025-06-01T14:22:00+00:00",
      "expires_at": "2025-07-01T14:22:00+00:00",
      "last_active_at": "2025-06-01T15:00:00+00:00",
      "revoked": false
    }
  ],
  "total": 3
}
```

**Errors:**
- `1003 not_found` -- User does not exist

**Example:**

```bash
curl http://localhost:3001/api/v1/admin/users/550e8400-e29b-41d4-a716-446655440000/sessions \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### DELETE /api/v1/admin/users/{user_id}/sessions

Revoke all active sessions and refresh tokens for a user, effectively logging them out of all devices.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type | Description |
|-----------|------|-------------|
| `user_id` | UUID | The user ID |

**Response (200):**

```json
{
  "revoked_count": 3,
  "message": "All sessions revoked"
}
```

**Errors:**
- `1003 not_found` -- User does not exist

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/admin/users/550e8400-e29b-41d4-a716-446655440000/sessions \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### GET /api/v1/admin/audit-log

Query the audit log with pagination. Entries are returned in reverse chronological order. Supports filtering by user ID.

**Auth:** Admin

**Query Parameters:**

| Parameter  | Type    | Default | Description                              |
|------------|---------|---------|------------------------------------------|
| `page`     | integer | `1`     | Page number (1-indexed)                  |
| `per_page` | integer | `50`    | Items per page (max 100)                 |
| `user_id`  | string  | --      | Filter entries by acting user ID         |

**Response (200):**

```json
{
  "entries": [
    {
      "id": "entry-uuid-here",
      "user_id": "550e8400-e29b-41d4-a716-446655440000",
      "event_type": "admin.user.deleted",
      "event_data": {
        "target_user_id": "660e8400-e29b-41d4-a716-446655440000",
        "target_email": "deleted-user@example.com"
      },
      "ip_address": "203.0.113.42",
      "user_agent": "Mozilla/5.0...",
      "created_at": "2025-06-01T14:22:00+00:00"
    }
  ],
  "total": 1024,
  "page": 1,
  "per_page": 50
}
```

**Audit Event Types:**

| Event Type                     | Description                                  |
|--------------------------------|----------------------------------------------|
| `register`                     | New user registration                        |
| `login`                        | Successful login                             |
| `logout`                       | User logout                                  |
| `admin_setup`                  | Initial admin created via bootstrap endpoint |
| `admin_promoted`               | User promoted to admin via CLI               |
| `admin.user.updated`           | Admin edited a user's profile                |
| `admin.user.role_changed`      | Admin changed a user's admin role            |
| `admin.user.status_changed`    | Admin enabled/disabled a user account        |
| `admin.user.password_reset`    | Admin forced a password reset                |
| `admin.user.deleted`           | Admin deleted a user (cascade)               |
| `admin.user.email_verified`    | Admin manually verified a user's email       |
| `admin.user.sessions_revoked`  | Admin revoked all sessions for a user        |
| `service_created`              | Downstream service registered                |
| `service_updated`              | Downstream service updated                   |
| `service_deleted`              | Downstream service deactivated               |
| `connection_created`           | User connected to a service                  |
| `connection_credential_updated`| User updated their connection credential     |
| `connection_removed`           | User disconnected from a service             |
| `oidc_credentials_accessed`    | OIDC credentials retrieved                   |
| `oidc_secret_regenerated`      | OIDC client secret regenerated               |
| `redirect_uris_updated`       | OIDC redirect URIs updated                   |
| `proxy_request`                | Request forwarded through the proxy          |
| `proxy_request_denied`         | Proxy request denied (auth or config issue)  |
| `provider_created`             | Provider configuration created               |
| `provider_updated`             | Provider configuration updated               |
| `provider_deleted`             | Provider deactivated                         |
| `provider_token_connected`     | User connected a provider token              |
| `provider_token_disconnected`  | User disconnected a provider token           |
| `provider_token_refreshed`     | Provider token manually refreshed            |
| `provider_oauth_initiated`     | User started OAuth flow with a provider      |
| `provider_oauth_callback_failed` | Provider OAuth callback failed             |
| `service_requirement_added`    | Provider requirement added to a service      |
| `service_requirement_removed`  | Provider requirement removed from a service  |

**Example:**

```bash
# Query all audit entries
curl "http://localhost:3001/api/v1/admin/audit-log?page=1&per_page=25" \
  -H "Authorization: Bearer <admin_access_token>"

# Filter by acting user
curl "http://localhost:3001/api/v1/admin/audit-log?user_id=550e8400-e29b-41d4-a716-446655440000" \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### POST /api/v1/admin/oauth-clients

Create a new OAuth client. Returns the client secret only at creation time -- it cannot be retrieved again.

**Auth:** Admin

**Request Body:**

| Field           | Type     | Required | Description                                               |
|-----------------|----------|----------|-----------------------------------------------------------|
| `name`              | string   | Yes      | Client display name                                       |
| `redirect_uris`     | string[] | Yes     | At least one redirect URI                                 |
| `client_type`       | string   | No       | `"confidential"` (default) or `"public"`                  |
| `delegation_scopes` | string   | No       | Space-separated scopes for token exchange (empty = disabled). Values: `llm:proxy`, `proxy:*`, `proxy:{service_id}`, `llm:status` |

```json
{
  "name": "My Web App",
  "redirect_uris": ["https://app.example.com/callback"],
  "client_type": "confidential",
  "delegation_scopes": "llm:proxy"
}
```

**Response (200):**

```json
{
  "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
  "client_name": "My Web App",
  "client_type": "confidential",
  "redirect_uris": ["https://app.example.com/callback"],
  "allowed_scopes": "openid profile email",
  "is_active": true,
  "client_secret": "nyx_secret_abc123...",
  "created_at": "2025-06-01T10:00:00+00:00"
}
```

**Errors:**
- `1002 forbidden` -- User is not an admin
- `1008 validation_error` -- Empty name, no redirect URIs, or invalid client_type

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/admin/oauth-clients \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "My Web App",
    "redirect_uris": ["https://app.example.com/callback"],
    "client_type": "confidential"
  }'
```

---

#### GET /api/v1/admin/oauth-clients

List all registered OAuth clients. Client secrets are never included in the list response.

**Auth:** Admin

**Response (200):**

```json
{
  "clients": [
    {
      "id": "a1b2c3d4-e5f6-7890-abcd-ef1234567890",
      "client_name": "My Web App",
      "client_type": "confidential",
      "redirect_uris": ["https://app.example.com/callback"],
      "allowed_scopes": "openid profile email",
      "delegation_scopes": "",
      "is_active": true,
      "client_secret": null,
      "created_at": "2025-06-01T10:00:00+00:00"
    }
  ]
}
```

**Example:**

```bash
curl http://localhost:3001/api/v1/admin/oauth-clients \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### DELETE /api/v1/admin/oauth-clients/{client_id}

Deactivate an OAuth client. The client can no longer be used for authorization after this operation.

**Auth:** Admin

**Path Parameters:**

| Parameter   | Type | Description        |
|-------------|------|--------------------|
| `client_id` | UUID | The OAuth client ID |

**Response (200):**

```json
{
  "message": "OAuth client deactivated"
}
```

**Errors:**
- `1002 forbidden` -- User is not an admin
- `1003 not_found` -- Client does not exist

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/admin/oauth-clients/a1b2c3d4-e5f6-7890-abcd-ef1234567890 \
  -H "Authorization: Bearer <admin_access_token>"
```

---

### Admin Roles

All admin role endpoints require the authenticated user to have `is_admin = true`.

#### GET /api/v1/admin/roles

List all roles. Optionally filter by OAuth client scope.

**Auth:** Admin

**Query Parameters:**

| Parameter   | Type   | Default | Description                                 |
|-------------|--------|---------|---------------------------------------------|
| `client_id` | string | --      | Filter roles scoped to a specific client    |

**Response (200):**

```json
{
  "roles": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "name": "Admin",
      "slug": "admin",
      "description": "Full system administrator",
      "permissions": ["users:read", "users:write", "admin:*"],
      "is_default": false,
      "is_system": true,
      "client_id": null,
      "created_at": "2025-06-01T10:00:00+00:00",
      "updated_at": "2025-06-01T10:00:00+00:00"
    }
  ]
}
```

**Example:**

```bash
curl http://localhost:3001/api/v1/admin/roles \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### POST /api/v1/admin/roles

Create a new role.

**Auth:** Admin

**Request Body:**

| Field         | Type     | Required | Description                                       |
|---------------|----------|----------|---------------------------------------------------|
| `name`        | string   | Yes      | Human-readable name                               |
| `slug`        | string   | Yes      | URL-safe identifier (must be unique)              |
| `description` | string   | No       | Role description                                  |
| `permissions` | string[] | Yes      | Permission tags (e.g., `["users:read"]`)          |
| `is_default`  | boolean  | No       | Auto-assign to new users (default: false)         |
| `client_id`   | string   | No       | Scope to a specific OAuth client                  |

**Response (200):** Returns the created role object (same shape as list response items).

**Errors:**
- `1004 conflict` -- Slug already exists
- `1008 validation_error` -- Name or slug is empty

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/admin/roles \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Editor",
    "slug": "editor",
    "description": "Can edit content",
    "permissions": ["content:read", "content:write"]
  }'
```

---

#### GET /api/v1/admin/roles/{role_id}

Get a single role by ID.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type   | Description   |
|-----------|--------|---------------|
| `role_id` | string | The role ID   |

**Response (200):** Returns the role object.

**Errors:**
- `1003 not_found` -- Role does not exist

**Example:**

```bash
curl http://localhost:3001/api/v1/admin/roles/role-uuid-here \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### PUT /api/v1/admin/roles/{role_id}

Update a role. System roles (`admin`, `user`) cannot be renamed or have their slug changed.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type   | Description   |
|-----------|--------|---------------|
| `role_id` | string | The role ID   |

**Request Body (all fields optional):**

| Field         | Type     | Description                            |
|---------------|----------|----------------------------------------|
| `name`        | string   | New name                               |
| `slug`        | string   | New slug                               |
| `description` | string   | New description                        |
| `permissions` | string[] | New permissions list (replaces existing)|
| `is_default`  | boolean  | New default flag                       |

**Response (200):** Returns the updated role object.

**Errors:**
- `1002 forbidden` -- Cannot modify system role name/slug
- `1003 not_found` -- Role does not exist

**Example:**

```bash
curl -X PUT http://localhost:3001/api/v1/admin/roles/role-uuid-here \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "permissions": ["content:read", "content:write", "content:delete"]
  }'
```

---

#### DELETE /api/v1/admin/roles/{role_id}

Delete a role. System roles cannot be deleted.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type   | Description   |
|-----------|--------|---------------|
| `role_id` | string | The role ID   |

**Response (200):**

```json
{
  "message": "Role deleted"
}
```

**Errors:**
- `1002 forbidden` -- Cannot delete system role
- `1003 not_found` -- Role does not exist

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/admin/roles/role-uuid-here \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### GET /api/v1/admin/users/{user_id}/roles

Get a user's direct roles, inherited roles (via group membership), and effective permissions.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type   | Description   |
|-----------|--------|---------------|
| `user_id` | string | The user ID   |

**Response (200):**

```json
{
  "direct_roles": [
    {
      "id": "...",
      "name": "Editor",
      "slug": "editor",
      "permissions": ["content:read", "content:write"],
      "is_default": false,
      "is_system": false,
      "client_id": null,
      "created_at": "...",
      "updated_at": "..."
    }
  ],
  "inherited_roles": [
    {
      "id": "...",
      "name": "Viewer",
      "slug": "viewer",
      "permissions": ["content:read"],
      "is_default": true,
      "is_system": false,
      "client_id": null,
      "created_at": "...",
      "updated_at": "..."
    }
  ],
  "effective_permissions": ["content:read", "content:write"]
}
```

**Example:**

```bash
curl http://localhost:3001/api/v1/admin/users/user-uuid-here/roles \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### POST /api/v1/admin/users/{user_id}/roles/{role_id}

Assign a role directly to a user.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type   | Description   |
|-----------|--------|---------------|
| `user_id` | string | The user ID   |
| `role_id` | string | The role ID   |

**Response (200):**

```json
{
  "message": "Role assigned"
}
```

**Errors:**
- `1003 not_found` -- User or role does not exist
- `1004 conflict` -- Role already assigned

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/admin/users/user-uuid-here/roles/role-uuid-here \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### DELETE /api/v1/admin/users/{user_id}/roles/{role_id}

Revoke a directly-assigned role from a user.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type   | Description   |
|-----------|--------|---------------|
| `user_id` | string | The user ID   |
| `role_id` | string | The role ID   |

**Response (200):**

```json
{
  "message": "Role revoked"
}
```

**Errors:**
- `1003 not_found` -- User does not have this role

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/admin/users/user-uuid-here/roles/role-uuid-here \
  -H "Authorization: Bearer <admin_access_token>"
```

---

### Admin Groups

All admin group endpoints require the authenticated user to have `is_admin = true`.

#### GET /api/v1/admin/groups

List all groups with their associated roles and member counts.

**Auth:** Admin

**Response (200):**

```json
{
  "groups": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "name": "Engineering",
      "slug": "engineering",
      "description": "Engineering team",
      "roles": [
        {
          "id": "...",
          "name": "Developer",
          "slug": "developer",
          "permissions": ["code:read", "code:write"],
          "is_default": false,
          "is_system": false,
          "client_id": null,
          "created_at": "...",
          "updated_at": "..."
        }
      ],
      "parent_group_id": null,
      "member_count": 12,
      "created_at": "2025-06-01T10:00:00+00:00",
      "updated_at": "2025-06-01T10:00:00+00:00"
    }
  ]
}
```

**Example:**

```bash
curl http://localhost:3001/api/v1/admin/groups \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### POST /api/v1/admin/groups

Create a new group.

**Auth:** Admin

**Request Body:**

| Field             | Type     | Required | Description                                  |
|-------------------|----------|----------|----------------------------------------------|
| `name`            | string   | Yes      | Human-readable name                          |
| `slug`            | string   | Yes      | URL-safe identifier (must be unique)         |
| `description`     | string   | No       | Group description                            |
| `role_ids`        | string[] | No       | Role IDs to attach (members inherit these)   |
| `parent_group_id` | string   | No       | Parent group ID for hierarchy                |

**Response (200):** Returns the created group object (same shape as list response items).

**Errors:**
- `1004 conflict` -- Slug already exists
- `1008 validation_error` -- Name or slug is empty

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/admin/groups \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Engineering",
    "slug": "engineering",
    "description": "Engineering team",
    "role_ids": ["role-uuid-here"]
  }'
```

---

#### GET /api/v1/admin/groups/{group_id}

Get a single group by ID, including its roles and member count.

**Auth:** Admin

**Path Parameters:**

| Parameter  | Type   | Description    |
|------------|--------|----------------|
| `group_id` | string | The group ID   |

**Response (200):** Returns the group object.

**Errors:**
- `1003 not_found` -- Group does not exist

**Example:**

```bash
curl http://localhost:3001/api/v1/admin/groups/group-uuid-here \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### PUT /api/v1/admin/groups/{group_id}

Update a group.

**Auth:** Admin

**Path Parameters:**

| Parameter  | Type   | Description    |
|------------|--------|----------------|
| `group_id` | string | The group ID   |

**Request Body (all fields optional):**

| Field             | Type     | Description                                  |
|-------------------|----------|----------------------------------------------|
| `name`            | string   | New name                                     |
| `slug`            | string   | New slug                                     |
| `description`     | string   | New description                              |
| `role_ids`        | string[] | New role IDs (replaces existing)             |
| `parent_group_id` | string   | New parent group ID (empty string to unset)  |

**Response (200):** Returns the updated group object.

**Errors:**
- `1003 not_found` -- Group does not exist

**Example:**

```bash
curl -X PUT http://localhost:3001/api/v1/admin/groups/group-uuid-here \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "role_ids": ["role-uuid-1", "role-uuid-2"]
  }'
```

---

#### DELETE /api/v1/admin/groups/{group_id}

Delete a group. Members are not deleted, but lose the group's inherited roles.

**Auth:** Admin

**Path Parameters:**

| Parameter  | Type   | Description    |
|------------|--------|----------------|
| `group_id` | string | The group ID   |

**Response (200):**

```json
{
  "message": "Group deleted"
}
```

**Errors:**
- `1003 not_found` -- Group does not exist

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/admin/groups/group-uuid-here \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### GET /api/v1/admin/groups/{group_id}/members

List all members of a group.

**Auth:** Admin

**Path Parameters:**

| Parameter  | Type   | Description    |
|------------|--------|----------------|
| `group_id` | string | The group ID   |

**Response (200):**

```json
{
  "members": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "email": "user@example.com",
      "display_name": "Jane Doe"
    }
  ],
  "total": 1
}
```

**Example:**

```bash
curl http://localhost:3001/api/v1/admin/groups/group-uuid-here/members \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### POST /api/v1/admin/groups/{group_id}/members/{user_id}

Add a user to a group. The user inherits the group's roles.

**Auth:** Admin

**Path Parameters:**

| Parameter  | Type   | Description    |
|------------|--------|----------------|
| `group_id` | string | The group ID   |
| `user_id`  | string | The user ID    |

**Response (200):**

```json
{
  "message": "Member added"
}
```

**Errors:**
- `1003 not_found` -- Group or user does not exist
- `1004 conflict` -- User is already a member

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/admin/groups/group-uuid-here/members/user-uuid-here \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### DELETE /api/v1/admin/groups/{group_id}/members/{user_id}

Remove a user from a group. The user loses the group's inherited roles.

**Auth:** Admin

**Path Parameters:**

| Parameter  | Type   | Description    |
|------------|--------|----------------|
| `group_id` | string | The group ID   |
| `user_id`  | string | The user ID    |

**Response (200):**

```json
{
  "message": "Member removed"
}
```

**Errors:**
- `1003 not_found` -- User is not a member of this group

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/admin/groups/group-uuid-here/members/user-uuid-here \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### GET /api/v1/admin/users/{user_id}/groups

Get all groups a user belongs to.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type   | Description   |
|-----------|--------|---------------|
| `user_id` | string | The user ID   |

**Response (200):**

```json
{
  "groups": [
    {
      "id": "...",
      "name": "Engineering",
      "slug": "engineering",
      "description": "Engineering team",
      "roles": [...],
      "parent_group_id": null,
      "member_count": 12,
      "created_at": "...",
      "updated_at": "..."
    }
  ]
}
```

**Example:**

```bash
curl http://localhost:3001/api/v1/admin/users/user-uuid-here/groups \
  -H "Authorization: Bearer <admin_access_token>"
```

---

### Admin Service Accounts

Service accounts are non-human (machine-to-machine) identities that authenticate via OAuth2 Client Credentials Grant. All admin endpoints require `is_admin = true`.

#### POST /api/v1/admin/service-accounts

Create a new service account. The `client_secret` is returned once in the response and cannot be retrieved later.

**Auth:** Admin

**Request Body:**

| Field                 | Type     | Required | Description                                  |
|-----------------------|----------|----------|----------------------------------------------|
| `name`                | string   | Yes      | Human-readable name (1-100 chars)            |
| `description`         | string   | No       | Description (max 500 chars)                  |
| `allowed_scopes`      | string   | Yes      | Space-separated allowed scopes               |
| `role_ids`            | string[] | No       | Role IDs to assign                           |
| `rate_limit_override` | number   | No       | Per-account rate limit (requests/second)     |

```json
{
  "name": "CI/CD Pipeline",
  "description": "Automated deployment service",
  "allowed_scopes": "proxy:* llm:proxy",
  "role_ids": ["role-uuid-here"],
  "rate_limit_override": 50
}
```

**Response (200):**

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "name": "CI/CD Pipeline",
  "client_id": "sa_a1b2c3d4e5f6a1b2c3d4e5f6",
  "client_secret": "sas_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
  "allowed_scopes": "proxy:* llm:proxy",
  "role_ids": ["role-uuid-here"],
  "is_active": true,
  "created_at": "2025-06-01T10:00:00+00:00",
  "message": "Service account created. Save the client_secret now -- it cannot be retrieved later."
}
```

**Errors:**
- `1008 validation_error` -- Name is empty or too long

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/admin/service-accounts \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "CI/CD Pipeline",
    "allowed_scopes": "proxy:* llm:proxy"
  }'
```

---

#### GET /api/v1/admin/service-accounts

List all service accounts with pagination and optional search.

**Auth:** Admin

**Query Parameters:**

| Parameter  | Type   | Default | Description                       |
|------------|--------|---------|-----------------------------------|
| `page`     | number | `1`     | Page number                       |
| `per_page` | number | `20`    | Items per page                    |
| `search`   | string | --      | Filter by name (case-insensitive) |

**Response (200):**

```json
{
  "service_accounts": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "name": "CI/CD Pipeline",
      "description": "Automated deployment service",
      "client_id": "sa_a1b2c3d4e5f6a1b2c3d4e5f6",
      "secret_prefix": "sas_xxxx",
      "allowed_scopes": "proxy:* llm:proxy",
      "role_ids": ["role-uuid-here"],
      "is_active": true,
      "rate_limit_override": 50,
      "created_by": "admin-uuid-here",
      "created_at": "2025-06-01T10:00:00+00:00",
      "updated_at": "2025-06-01T10:00:00+00:00",
      "last_authenticated_at": "2025-06-15T08:30:00+00:00"
    }
  ],
  "total": 1,
  "page": 1,
  "per_page": 20
}
```

**Example:**

```bash
curl "http://localhost:3001/api/v1/admin/service-accounts?page=1&per_page=10&search=pipeline" \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### GET /api/v1/admin/service-accounts/{sa_id}

Get a single service account by ID.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type   | Description            |
|-----------|--------|------------------------|
| `sa_id`   | string | Service account UUID   |

**Response (200):** Same shape as a single item in the list response.

**Errors:**
- `5000 service_account_not_found` -- Service account does not exist

**Example:**

```bash
curl http://localhost:3001/api/v1/admin/service-accounts/sa-uuid-here \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### PUT /api/v1/admin/service-accounts/{sa_id}

Update a service account's mutable fields. All fields are optional; only provided fields are updated.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type   | Description            |
|-----------|--------|------------------------|
| `sa_id`   | string | Service account UUID   |

**Request Body:**

| Field                 | Type      | Required | Description                                |
|-----------------------|-----------|----------|--------------------------------------------|
| `name`                | string    | No       | New name (1-100 chars)                     |
| `description`         | string    | No       | New description (max 500 chars)            |
| `allowed_scopes`      | string    | No       | New allowed scopes                         |
| `role_ids`            | string[]  | No       | New role assignments                       |
| `rate_limit_override` | number?   | No       | New rate limit (null to remove override)   |
| `is_active`           | boolean   | No       | Enable or disable the account              |

**Response (200):** Returns the updated service account (same shape as list item).

**Errors:**
- `5000 service_account_not_found` -- Service account does not exist

**Example:**

```bash
curl -X PUT http://localhost:3001/api/v1/admin/service-accounts/sa-uuid-here \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{"name": "Updated Name", "is_active": false}'
```

---

#### DELETE /api/v1/admin/service-accounts/{sa_id}

Soft-delete (deactivate) a service account and revoke all its tokens.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type   | Description            |
|-----------|--------|------------------------|
| `sa_id`   | string | Service account UUID   |

**Response (200):**

```json
{
  "message": "Service account deleted"
}
```

**Errors:**
- `5000 service_account_not_found` -- Service account does not exist

**Example:**

```bash
curl -X DELETE http://localhost:3001/api/v1/admin/service-accounts/sa-uuid-here \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### POST /api/v1/admin/service-accounts/{sa_id}/rotate-secret

Generate a new client secret and revoke all existing tokens. The new secret is returned once.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type   | Description            |
|-----------|--------|------------------------|
| `sa_id`   | string | Service account UUID   |

**Response (200):**

```json
{
  "client_id": "sa_a1b2c3d4e5f6a1b2c3d4e5f6",
  "client_secret": "sas_yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy",
  "secret_prefix": "sas_yyyy",
  "message": "Secret rotated. All existing tokens have been revoked. Save the new secret now."
}
```

**Errors:**
- `5000 service_account_not_found` -- Service account does not exist

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/admin/service-accounts/sa-uuid-here/rotate-secret \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### POST /api/v1/admin/service-accounts/{sa_id}/revoke-tokens

Revoke all active tokens for a service account without rotating the secret.

**Auth:** Admin

**Path Parameters:**

| Parameter | Type   | Description            |
|-----------|--------|------------------------|
| `sa_id`   | string | Service account UUID   |

**Response (200):**

```json
{
  "revoked_count": 5,
  "message": "All tokens revoked"
}
```

**Errors:**
- `5000 service_account_not_found` -- Service account does not exist

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/admin/service-accounts/sa-uuid-here/revoke-tokens \
  -H "Authorization: Bearer <admin_access_token>"
```

---

#### Service Account Authentication (Client Credentials)

Service accounts authenticate at the existing `POST /oauth/token` endpoint using the `client_credentials` grant type.

**Request:**

```
POST /oauth/token
Content-Type: application/x-www-form-urlencoded

grant_type=client_credentials&client_id=sa_a1b2c3d4e5f6...&client_secret=sas_xxxxxxxx...&scope=proxy:* llm:proxy
```

Or with HTTP Basic Authentication:

```
POST /oauth/token
Content-Type: application/x-www-form-urlencoded
Authorization: Basic base64(client_id:client_secret)

grant_type=client_credentials&scope=proxy:* llm:proxy
```

**Response (200):**

```json
{
  "access_token": "eyJhbGci...",
  "token_type": "Bearer",
  "expires_in": 3600,
  "scope": "proxy:* llm:proxy"
}
```

The `scope` parameter is optional. If omitted, all of the service account's `allowed_scopes` are granted. If provided, the requested scopes must be a subset of `allowed_scopes`.

No refresh token is issued. When the access token expires, the service account must re-authenticate with client credentials.

**Errors:**
- `1001 unauthorized` -- Invalid client_id or client_secret
- `5001 service_account_inactive` -- Service account is deactivated
- `3002 invalid_scope` -- Requested scope not allowed

**Example:**

```bash
curl -X POST http://localhost:3001/oauth/token \
  -H "Content-Type: application/x-www-form-urlencoded" \
  -d 'grant_type=client_credentials&client_id=sa_a1b2c3d4e5f6...&client_secret=sas_xxxxxxxx...&scope=proxy:*'
```

---

## JWT Token Format

All JWTs are signed with RS256 (RSA SHA-256) using a 4096-bit key pair.

### Access Token Claims

| Claim        | Type   | Description                       |
|--------------|--------|-----------------------------------|
| Claim         | Type     | Description                                       |
|---------------|----------|---------------------------------------------------|
| `sub`         | string   | User ID (UUID)                                    |
| `iss`         | string   | Issuer (matches `JWT_ISSUER`)                     |
| `aud`         | string   | Audience (matches `BASE_URL`)                     |
| `exp`         | number   | Expiration (Unix timestamp)                       |
| `iat`         | number   | Issued at (Unix timestamp)                        |
| `jti`         | string   | Unique token ID (UUID)                            |
| `scope`       | string   | Space-separated scopes                            |
| `token_type`  | string   | `"access"`                                        |
| `roles`       | string[] | Role slugs (present when `roles` scope requested) |
| `groups`      | string[] | Group slugs (present when `groups` scope requested)|
| `permissions` | string[] | Effective permissions (present when `roles` scope requested) |
| `acr`         | string   | Authentication Context Class Reference            |
| `amr`         | string[] | Authentication Methods References                 |
| `auth_time`   | number   | Time of authentication (Unix timestamp)           |
| `sid`         | string   | Session ID                                        |

The `roles`, `groups`, and `permissions` claims are only included when the corresponding scopes (`roles`, `groups`) are requested in the OAuth authorization flow.

### Refresh Token Claims

Same structure as access tokens, but:
- `token_type` is `"refresh"`
- `scope` is empty
- `exp` uses `JWT_REFRESH_TTL_SECS` (default: 7 days)
- RBAC claims (`roles`, `groups`, `permissions`) are not included

### Service Account Token Claims

Service account access tokens include:

| Claim        | Type   | Description                                        |
|--------------|--------|----------------------------------------------------|
| `sub`        | string | Service account ID (UUID)                          |
| `iss`        | string | Issuer (matches `JWT_ISSUER`)                      |
| `aud`        | string | Audience (matches `BASE_URL`)                      |
| `exp`        | number | Expiration (Unix timestamp)                        |
| `iat`        | number | Issued at (Unix timestamp)                         |
| `jti`        | string | Unique token ID (UUID, used for revocation)        |
| `scope`      | string | Space-separated granted scopes                     |
| `token_type` | string | `"access"`                                         |
| `sa`         | boolean| Always `true` for service account tokens           |

Service account tokens do **not** include `sid`, `roles`, `groups`, `permissions`, `act`, or `delegated` claims. No refresh tokens are issued for service accounts.

---

## Notification Settings

Manage notification channels and approval preferences. All endpoints require authentication (human-only, no service accounts or delegated tokens).

### Get Notification Settings

```
GET /api/v1/notifications/settings
Authorization: Bearer <access_token>
```

**Response (200):**

```json
{
  "telegram_connected": true,
  "telegram_username": "johndoe",
  "telegram_enabled": true,
  "approval_required": true,
  "approval_timeout_secs": 30,
  "grant_expiry_days": 30
}
```

**curl:**

```bash
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:3001/api/v1/notifications/settings
```

### Update Notification Settings

```
PUT /api/v1/notifications/settings
Authorization: Bearer <access_token>
Content-Type: application/json
```

**Request:**

```json
{
  "telegram_enabled": true,
  "approval_required": true,
  "approval_timeout_secs": 60,
  "grant_expiry_days": 14
}
```

All fields are optional. Only provided fields are updated.

**Validation:**
- `approval_timeout_secs`: 10..=300
- `grant_expiry_days`: 1..=365
- `telegram_enabled: true` requires a linked Telegram account

**Response (200):** Same shape as GET response.

**curl:**

```bash
curl -X PUT -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"approval_required": true, "approval_timeout_secs": 60}' \
  http://localhost:3001/api/v1/notifications/settings
```

### Generate Telegram Link Code

```
POST /api/v1/notifications/telegram/link
Authorization: Bearer <access_token>
```

Generates a one-time code that the user sends to the NyxID Telegram bot via `/start <code>`.

**Response (200):**

```json
{
  "link_code": "NYXID-A1B2C3D4",
  "bot_username": "NyxIDBot",
  "expires_in_secs": 300,
  "instructions": "Send /start NYXID-A1B2C3D4 to @NyxIDBot on Telegram"
}
```

**curl:**

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" \
  http://localhost:3001/api/v1/notifications/telegram/link
```

### Disconnect Telegram

```
DELETE /api/v1/notifications/telegram
Authorization: Bearer <access_token>
```

Clears the linked Telegram account and disables Telegram notifications.

**Response (200):**

```json
{
  "message": "Telegram disconnected"
}
```

**curl:**

```bash
curl -X DELETE -H "Authorization: Bearer $TOKEN" \
  http://localhost:3001/api/v1/notifications/telegram
```

---

## Approval Management

View approval history, manage grants, and approve/reject requests via the web UI. All endpoints require authentication. The status polling endpoint is also accessible by delegated tokens and service accounts.

**Blocking vs. polling:** The primary approval flow is blocking -- proxy and LLM gateway requests hold the HTTP connection open until approval/rejection/timeout. The status polling endpoint below is a secondary mechanism for callers that use async workflows or need to monitor approval status from a separate connection.

### List Approval Requests (History)

```
GET /api/v1/approvals/requests?status=pending&page=1&per_page=20
Authorization: Bearer <access_token>
```

**Query parameters:**

| Parameter  | Type   | Default | Description                                |
|------------|--------|---------|--------------------------------------------|
| `status`   | string |         | Filter by status: `pending`, `approved`, `rejected`, `expired` |
| `page`     | number | `1`     | Page number (1-indexed)                    |
| `per_page` | number | `20`    | Results per page (max 100)                 |

**Response (200):**

```json
{
  "requests": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "service_name": "OpenAI API",
      "service_slug": "openai",
      "requester_type": "service_account",
      "requester_label": "CI Pipeline",
      "operation_summary": "proxy:POST /v1/chat/completions",
      "status": "approved",
      "created_at": "2026-03-03T00:00:00+00:00",
      "decided_at": "2026-03-03T00:00:05+00:00",
      "decision_channel": "telegram"
    }
  ],
  "total": 42,
  "page": 1,
  "per_page": 20
}
```

**curl:**

```bash
curl -H "Authorization: Bearer $TOKEN" \
  "http://localhost:3001/api/v1/approvals/requests?status=pending"
```

### Poll Approval Request Status

```
GET /api/v1/approvals/requests/{request_id}/status
Authorization: Bearer <access_token>
```

Status endpoint for monitoring approval requests. The primary flow is blocking (proxy/LLM connections wait for approval), but this endpoint is available for async callers that manage approval status separately. Also accessible by delegated tokens and service accounts.

The caller must authenticate and match the original approval request binding:
- resource owner (`approval_request.user_id`)
- `requester_type`
- `requester_id`

This prevents authenticated callers from polling approval requests that belong to a different caller context.

**Response (200):**

```json
{
  "status": "pending",
  "expires_at": "2026-03-03T00:00:30+00:00"
}
```

Status values: `pending`, `approved`, `rejected`, `expired`.

When status changes to `approved`, the caller can retry the original proxy request (the grant now exists).

**curl:**

```bash
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:3001/api/v1/approvals/requests/550e8400-e29b-41d4-a716-446655440000/status
```

### Approve/Reject via Web UI

```
POST /api/v1/approvals/requests/{request_id}/decide
Authorization: Bearer <access_token>
Content-Type: application/json
```

**Request:**

```json
{
  "approved": true
}
```

Only the resource owner (the user who must approve) can call this endpoint.

**Response (200):**

```json
{
  "id": "550e8400-e29b-41d4-a716-446655440000",
  "status": "approved",
  "decided_at": "2026-03-03T00:00:05+00:00"
}
```

**Error (404):** Request not found or already processed (replay-safe).

**curl:**

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"approved": true}' \
  http://localhost:3001/api/v1/approvals/requests/550e8400-e29b-41d4-a716-446655440000/decide
```

### List Active Grants

```
GET /api/v1/approvals/grants?page=1&per_page=20
Authorization: Bearer <access_token>
```

Returns active (non-expired, non-revoked) approval grants for the current user.

**Response (200):**

```json
{
  "grants": [
    {
      "id": "uuid",
      "service_id": "uuid",
      "service_name": "OpenAI API",
      "requester_type": "service_account",
      "requester_id": "uuid",
      "requester_label": "CI Pipeline",
      "granted_at": "2026-03-03T00:00:00+00:00",
      "expires_at": "2026-04-02T00:00:00+00:00"
    }
  ],
  "total": 5,
  "page": 1,
  "per_page": 20
}
```

**curl:**

```bash
curl -H "Authorization: Bearer $TOKEN" \
  http://localhost:3001/api/v1/approvals/grants
```

### Revoke a Grant

```
DELETE /api/v1/approvals/grants/{grant_id}
Authorization: Bearer <access_token>
```

Revokes a specific approval grant. The requester will need to re-approve on their next access attempt.

**Response (200):**

```json
{
  "message": "Grant revoked"
}
```

**curl:**

```bash
curl -X DELETE -H "Authorization: Bearer $TOKEN" \
  http://localhost:3001/api/v1/approvals/grants/550e8400-e29b-41d4-a716-446655440000
```

---

## Webhooks

### Telegram Webhook

```
POST /api/v1/webhooks/telegram
X-Telegram-Bot-Api-Secret-Token: <secret>
Content-Type: application/json
```

Receives Telegram updates. This endpoint is unauthenticated (no JWT/session required) but verified via the `X-Telegram-Bot-Api-Secret-Token` header using constant-time comparison.

Handles two types of updates:

1. **Callback queries** -- User pressed Approve/Reject on an inline keyboard. The handler verifies the chat ID matches the approval request, processes the decision, and edits the Telegram message to show the result.

2. **Messages** -- User sent `/start NYXID-XXXXXX` to link their Telegram account. The handler validates the link code, updates the notification channel, and sends a confirmation message.

**Response:** Always `200 OK` (empty body) to prevent Telegram retries.

This endpoint is not intended to be called directly by clients.

---

## Rate Limiting

All endpoints are subject to rate limiting. When the limit is exceeded, the server returns:

```
HTTP/1.1 429 Too Many Requests
Content-Type: application/json

{
  "error": "rate_limited",
  "error_code": 1005,
  "message": "Rate limited"
}
```

Default limits:
- **Per-IP:** 30 requests per 1-second window
- **Global:** 10 requests/second sustained with burst capacity of 30
