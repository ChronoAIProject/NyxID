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
  - [Service Connections](#service-connections)
  - [Service Provider Requirements](#service-provider-requirements)
  - [Providers](#providers)
  - [User Provider Tokens](#user-provider-tokens)
  - [Sessions](#sessions)
  - [Service Endpoints](#service-endpoints)
  - [MCP Config](#mcp-config)
  - [Proxy](#proxy)
  - [MFA](#mfa-multi-factor-authentication)
  - [OAuth / OpenID Connect](#oauth--openid-connect)
  - [OIDC Discovery](#oidc-discovery)
  - [Admin](#admin)

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

**Response:** The downstream service's response status code, allowed headers, and body are returned directly. Only a safe allowlist of response headers is forwarded.

**Limits:** Request body is limited to 10 MB for proxy requests.

**Errors:**
- `1000 bad_request` -- Service is inactive, service is a provider, invalid proxy path, or connection missing credential
- `1002 forbidden` -- No active connection to this service
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
| `name`          | string   | Yes      | Client display name                                       |
| `redirect_uris` | string[] | Yes     | At least one redirect URI                                 |
| `client_type`   | string   | No       | `"confidential"` (default) or `"public"`                  |

```json
{
  "name": "My Web App",
  "redirect_uris": ["https://app.example.com/callback"],
  "client_type": "confidential"
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
