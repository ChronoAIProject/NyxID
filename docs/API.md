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
  - [Users](#users)
  - [API Keys](#api-keys)
  - [Downstream Services](#downstream-services)
  - [Proxy](#proxy)
  - [OAuth / OpenID Connect](#oauth--openid-connect)
  - [Admin](#admin)

---

## Authentication

Most endpoints require authentication. NyxID supports three authentication methods, checked in the following order:

1. **Bearer Token** -- `Authorization: Bearer <access_token>` header
2. **Session Cookie** -- `nyx_session` HttpOnly cookie (set at login)
3. **Access Token Cookie** -- `nyx_access_token` HttpOnly cookie (set at login)

Endpoints marked **Auth: None** do not require authentication.
Endpoints marked **Auth: Required** require any of the above.
Endpoints marked **Auth: Admin** require an authenticated user with `is_admin = true`.
Endpoints marked **Auth: Cookie** use a specific cookie (e.g., the refresh token cookie).

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

List all active downstream services.

**Auth:** Required

**Response (200):**

```json
{
  "services": [
    {
      "id": "d1e2f3a4-b5c6-7890-1234-567890abcdef",
      "name": "Internal Analytics API",
      "slug": "analytics",
      "description": "Company analytics service",
      "base_url": "https://analytics.internal.example.com",
      "auth_method": "bearer",
      "auth_key_name": "Authorization",
      "is_active": true,
      "created_at": "2025-01-15T10:30:00+00:00"
    }
  ]
}
```

**Example:**

```bash
curl http://localhost:3001/api/v1/services \
  -H "Authorization: Bearer <access_token>"
```

---

#### POST /api/v1/services

Register a new downstream service. The credential is encrypted with AES-256-GCM before storage.

**Auth:** Admin

**Request Body:**

| Field           | Type   | Required | Description                                     |
|-----------------|--------|----------|-------------------------------------------------|
| `name`          | string | Yes      | Service display name (max 200 chars)            |
| `slug`          | string | Yes      | URL-safe identifier (max 100 chars, unique)     |
| `description`   | string | No       | Service description                             |
| `base_url`      | string | Yes      | Downstream service base URL (max 2048 chars)    |
| `auth_method`   | string | Yes      | One of: `header`, `bearer`, `query`, `basic`    |
| `auth_key_name` | string | Yes      | Header name, query param name, etc.             |
| `credential`    | string | Yes      | API key, token, or `username:password` for basic|

**Auth Methods:**

| Method   | Behavior                                            |
|----------|-----------------------------------------------------|
| `header` | Adds `auth_key_name: credential` as a request header|
| `bearer` | Adds `Authorization: Bearer credential` header      |
| `query`  | Appends `?auth_key_name=credential` to the URL      |
| `basic`  | Sends HTTP Basic Auth (credential = `user:password`) |

```json
{
  "name": "Internal Analytics API",
  "slug": "analytics",
  "description": "Company analytics service",
  "base_url": "https://analytics.internal.example.com",
  "auth_method": "bearer",
  "auth_key_name": "Authorization",
  "credential": "sk-analytics-secret-key-here"
}
```

**Response (200):**

```json
{
  "id": "d1e2f3a4-b5c6-7890-1234-567890abcdef",
  "name": "Internal Analytics API",
  "slug": "analytics",
  "description": "Company analytics service",
  "base_url": "https://analytics.internal.example.com",
  "auth_method": "bearer",
  "auth_key_name": "Authorization",
  "is_active": true,
  "created_at": "2025-06-01T10:00:00+00:00"
}
```

**Errors:**
- `1002 forbidden` -- User is not an admin
- `1004 conflict` -- Slug already exists
- `1008 validation_error` -- Missing required fields, invalid auth_method, or SSRF-blocked URL

**Example:**

```bash
curl -X POST http://localhost:3001/api/v1/services \
  -H "Authorization: Bearer <admin_access_token>" \
  -H "Content-Type: application/json" \
  -d '{
    "name": "Analytics API",
    "slug": "analytics",
    "base_url": "https://analytics.example.com",
    "auth_method": "header",
    "auth_key_name": "X-API-Key",
    "credential": "secret-api-key"
  }'
```

---

#### DELETE /api/v1/services/{service_id}

Deactivate a downstream service. Only admins or the original service creator can perform this action.

**Auth:** Admin (or service creator)

**Path Parameters:**

| Parameter    | Type | Description          |
|--------------|------|----------------------|
| `service_id` | UUID | The service ID       |

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

### Proxy

#### ANY /api/v1/proxy/{service_id}/{*path}

Forward any HTTP request to a registered downstream service. NyxID resolves the service, decrypts the stored credential, and injects it into the outbound request using the configured auth method.

If the authenticated user has a per-user credential override for this service (via `user_service_connections`), that credential is used instead of the service-level default.

**Auth:** Required

**Path Parameters:**

| Parameter    | Type   | Description                                    |
|--------------|--------|------------------------------------------------|
| `service_id` | UUID   | The downstream service ID                      |
| `*path`      | string | The path to forward (appended to service base URL) |

**Supported Methods:** GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS

**Request:** The request body, query parameters, and allowed headers are forwarded to the downstream service. Only safe headers are forwarded (content-type, accept, accept-language, accept-encoding, content-length, user-agent, x-request-id, x-correlation-id).

**Response:** The downstream service's response status code, headers (minus hop-by-hop headers), and body are returned directly.

**Limits:** Request body is limited to 10 MB for proxy requests.

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
| `scope`                 | string | No       | Space-separated scopes (default: `openid profile email`) |
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

Token endpoint. Exchanges an authorization code for access, refresh, and ID tokens. Also supports the `refresh_token` grant type.

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

OpenID Connect UserInfo endpoint. Returns claims about the authenticated user.

**Auth:** Required (Bearer token issued by the `/oauth/token` endpoint)

**Response (200):**

```json
{
  "sub": "550e8400-e29b-41d4-a716-446655440000",
  "email": "user@example.com",
  "email_verified": true,
  "name": "Jane Doe",
  "picture": "https://example.com/avatar.jpg"
}
```

**Example:**

```bash
curl http://localhost:3001/oauth/userinfo \
  -H "Authorization: Bearer <access_token>"
```

---

### Admin

All admin endpoints require the authenticated user to have `is_admin = true`.

#### GET /api/v1/admin/users

List all users with pagination.

**Auth:** Admin

**Query Parameters:**

| Parameter  | Type    | Default | Description                     |
|------------|---------|---------|---------------------------------|
| `page`     | integer | `1`     | Page number (1-indexed)         |
| `per_page` | integer | `50`    | Items per page (max 100)        |

**Response (200):**

```json
{
  "users": [
    {
      "id": "550e8400-e29b-41d4-a716-446655440000",
      "email": "user@example.com",
      "display_name": "Jane Doe",
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
curl "http://localhost:3001/api/v1/admin/users?page=1&per_page=25" \
  -H "Authorization: Bearer <admin_access_token>"
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

#### GET /api/v1/admin/audit-log

Query the audit log with pagination. Entries are returned in reverse chronological order.

**Auth:** Admin

**Query Parameters:**

| Parameter  | Type    | Default | Description                     |
|------------|---------|---------|---------------------------------|
| `page`     | integer | `1`     | Page number (1-indexed)         |
| `per_page` | integer | `50`    | Items per page (max 100)        |

**Response (200):**

```json
{
  "entries": [
    {
      "id": "entry-uuid-here",
      "user_id": "550e8400-e29b-41d4-a716-446655440000",
      "action": "login",
      "resource_type": "session",
      "resource_id": "session-uuid-here",
      "ip_address": "203.0.113.42",
      "created_at": "2025-06-01T14:22:00+00:00"
    }
  ],
  "total": 1024,
  "page": 1,
  "per_page": 50
}
```

**Audit Actions:**

| Action       | Resource Type | Description                        |
|--------------|---------------|------------------------------------|
| `register`   | `user`        | New user registration              |
| `login`      | `session`     | Successful login                   |
| `logout`     | `session`     | User logout                        |

**Example:**

```bash
curl "http://localhost:3001/api/v1/admin/audit-log?page=1&per_page=25" \
  -H "Authorization: Bearer <admin_access_token>"
```

---

## JWT Token Format

All JWTs are signed with RS256 (RSA SHA-256) using a 4096-bit key pair.

### Access Token Claims

| Claim        | Type   | Description                       |
|--------------|--------|-----------------------------------|
| `sub`        | string | User ID (UUID)                    |
| `iss`        | string | Issuer (matches `JWT_ISSUER`)     |
| `aud`        | string | Audience (matches `BASE_URL`)     |
| `exp`        | number | Expiration (Unix timestamp)       |
| `iat`        | number | Issued at (Unix timestamp)        |
| `jti`        | string | Unique token ID (UUID)            |
| `scope`      | string | Space-separated scopes            |
| `token_type` | string | `"access"`                        |

### Refresh Token Claims

Same structure as access tokens, but:
- `token_type` is `"refresh"`
- `scope` is empty
- `exp` uses `JWT_REFRESH_TTL_SECS` (default: 7 days)

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
