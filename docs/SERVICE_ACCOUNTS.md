# Service Accounts

Service accounts provide machine-to-machine authentication for automated systems, CI/CD pipelines, backend services, and other non-human clients that need to interact with NyxID APIs. They authenticate using the OAuth 2.0 Client Credentials grant and receive short-lived JWT tokens.

---

## Table of Contents

- [Overview](#overview)
- [Lifecycle](#lifecycle)
  - [Creating a Service Account](#creating-a-service-account)
  - [Storing the Secret](#storing-the-secret)
  - [Updating a Service Account](#updating-a-service-account)
  - [Rotating the Secret](#rotating-the-secret)
  - [Deactivating and Deleting](#deactivating-and-deleting)
- [Authentication](#authentication)
  - [Client Credentials Grant](#client-credentials-grant)
  - [Token Format](#token-format)
- [Using Tokens](#using-tokens)
  - [LLM Gateway](#llm-gateway)
  - [Proxy to Downstream Services](#proxy-to-downstream-services)
  - [Provider Management](#provider-management)
- [Token Expiry and Re-Authentication](#token-expiry-and-re-authentication)
- [Scopes and Access Control](#scopes-and-access-control)
- [Token Revocation](#token-revocation)
- [Security](#security)
- [Configuration Reference](#configuration-reference)
- [API Reference](#api-reference)

---

## Overview

Service accounts differ from user accounts in several key ways:

| Aspect | User Account | Service Account |
|--------|-------------|----------------|
| Authentication | Email/password, OAuth, MFA | Client ID + Client Secret |
| Token grant | Authorization Code, Refresh Token | Client Credentials only |
| Token TTL | 15 min access + 7 day refresh | 1 hour access, no refresh token |
| Token renewal | Refresh token grant | Re-authenticate with credentials |
| Identity | Human user with profile | Machine identity with name/description |
| MFA | Supported | Not applicable |
| Sessions | Session tracking | No sessions |
| Admin UI | User management panel | Dedicated service account management |

---

## Lifecycle

### Creating a Service Account

Admins create service accounts via the admin API:

```http
POST /api/v1/admin/service-accounts HTTP/1.1
Authorization: Bearer <admin_access_token>
Content-Type: application/json

{
  "name": "CI Pipeline Bot",
  "description": "Automated CI/CD pipeline that runs LLM evaluations",
  "allowed_scopes": "llm:proxy llm:status proxy:*",
  "role_ids": ["role-uuid-1"]
}
```

Response:

```json
{
  "id": "sa-uuid",
  "client_id": "sa_a1b2c3d4e5f6a1b2c3d4e5f6",
  "client_secret": "sas_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx",
  "name": "CI Pipeline Bot",
  "description": "Automated CI/CD pipeline that runs LLM evaluations",
  "allowed_scopes": "llm:proxy llm:status proxy:*",
  "created_at": "2025-01-15T10:00:00Z"
}
```

**The `client_secret` is returned only once.** It is hashed (SHA-256) before storage and cannot be retrieved again.

### Storing the Secret

Store the `client_id` and `client_secret` securely:

- Use a secrets manager (AWS Secrets Manager, HashiCorp Vault, etc.)
- Never commit credentials to source control
- Never log the client secret
- Use environment variables in deployment configurations

```bash
# Example: Environment variables
export NYXID_CLIENT_ID="sa_a1b2c3d4e5f6a1b2c3d4e5f6"
export NYXID_CLIENT_SECRET="sas_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx"
```

### Updating a Service Account

Admins can update mutable fields (name, description, scopes, roles, active status):

```http
PUT /api/v1/admin/service-accounts/:sa_id HTTP/1.1
Authorization: Bearer <admin_access_token>
Content-Type: application/json

{
  "name": "CI Pipeline Bot (Production)",
  "allowed_scopes": "llm:proxy llm:status"
}
```

Scope changes take effect on the next token issuance. Existing tokens retain their original scopes until they expire or are revoked.

### Rotating the Secret

If a secret is compromised or as part of regular rotation:

```http
POST /api/v1/admin/service-accounts/:sa_id/rotate-secret HTTP/1.1
Authorization: Bearer <admin_access_token>
```

Response:

```json
{
  "client_id": "sa_a1b2c3d4e5f6a1b2c3d4e5f6",
  "client_secret": "sas_yyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyyy",
  "secret_prefix": "sas_yyyy"
}
```

This immediately:
1. Generates a new client secret
2. **Revokes all existing tokens** for this service account
3. The old secret can no longer authenticate

### Deactivating and Deleting

**Deactivate** (reversible):

```http
PUT /api/v1/admin/service-accounts/:sa_id HTTP/1.1
Authorization: Bearer <admin_access_token>
Content-Type: application/json

{"is_active": false}
```

**Delete** (soft delete -- deactivates and revokes all tokens):

```http
DELETE /api/v1/admin/service-accounts/:sa_id HTTP/1.1
Authorization: Bearer <admin_access_token>
```

Both operations immediately block future authentication and revoke all outstanding tokens.

---

## Authentication

### Client Credentials Grant

Service accounts authenticate at the OAuth token endpoint:

```http
POST /oauth/token HTTP/1.1
Content-Type: application/x-www-form-urlencoded

grant_type=client_credentials
&client_id=sa_a1b2c3d4e5f6a1b2c3d4e5f6
&client_secret=sas_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
&scope=llm:proxy
```

Or using HTTP Basic authentication:

```http
POST /oauth/token HTTP/1.1
Authorization: Basic base64(client_id:client_secret)
Content-Type: application/x-www-form-urlencoded

grant_type=client_credentials
&scope=llm:proxy
```

**Scope parameter:**
- Optional. If omitted, the token includes all of the service account's `allowed_scopes`.
- If provided, must be a space-separated subset of `allowed_scopes`.
- Requesting a scope not in `allowed_scopes` returns an `invalid_scope` error.

**Response (success):**

```json
{
  "access_token": "eyJhbGciOiJSUzI1NiIs...",
  "token_type": "Bearer",
  "expires_in": 3600,
  "scope": "llm:proxy"
}
```

**Response (failure):**

```json
{
  "error": "invalid_client",
  "error_description": "Invalid client credentials"
}
```

Authentication failures return a generic error message regardless of whether the `client_id` exists, preventing enumeration attacks.

### Token Format

Service account tokens are RS256-signed JWTs:

```json
{
  "sub": "<service_account_id>",
  "iss": "nyxid",
  "aud": "https://your-nyxid-instance.com",
  "exp": 1700003600,
  "iat": 1700000000,
  "jti": "<unique_token_id>",
  "scope": "llm:proxy",
  "token_type": "access",
  "sa": true
}
```

Key differences from user tokens:

| Claim | User Token | Service Account Token |
|-------|-----------|----------------------|
| `sub` | User UUID | Service account UUID |
| `sa` | Absent | `true` |
| `sid` | Session UUID | Absent |
| `act` | Present on delegation tokens | Absent |
| `delegated` | Present on delegation tokens | Absent |

The `sa: true` claim identifies the token as a service account token throughout the system.

---

## Using Tokens

Service account tokens are used as Bearer tokens in the `Authorization` header.

### LLM Gateway

The primary use case for service accounts is calling LLM providers through the gateway.

**Prerequisites:** The service account must have provider credentials connected before it can use the LLM gateway.

**Option A: Admin connects providers (recommended)**

Admins can connect providers on behalf of a service account directly from the admin API, without needing the SA's credentials. Three connection methods are available depending on the provider type:

**API Key Connection**

For providers that use API keys (OpenAI, Anthropic, etc.):

1. Admin creates the service account and saves the `client_id` + `client_secret`
2. Admin connects the provider with an API key:
   ```http
   POST /api/v1/admin/service-accounts/:sa_id/providers/:provider_id/connect/api-key HTTP/1.1
   Authorization: Bearer <admin_access_token>
   Content-Type: application/json

   {"api_key": "sk-...", "label": "Production OpenAI key"}
   ```
3. Admin verifies connected providers:
   ```http
   GET /api/v1/admin/service-accounts/:sa_id/providers HTTP/1.1
   Authorization: Bearer <admin_access_token>
   ```
4. The SA authenticates and uses the LLM gateway -- no provider setup needed on the SA side

**OAuth Redirect Connection**

For providers that support standard OAuth 2.0 authorization code flows:

1. Admin initiates the OAuth flow on behalf of the SA:
   ```http
   GET /api/v1/admin/service-accounts/:sa_id/providers/:provider_id/connect/oauth HTTP/1.1
   Authorization: Bearer <admin_access_token>
   ```
2. The admin is redirected to the provider's authorization page to grant access
3. After authorization, the callback stores the OAuth tokens under the SA's identity
4. The SA can use the provider via the LLM gateway without any further setup

**Device Code Connection**

For providers that use the device authorization grant (e.g., OpenAI Codex with ChatGPT subscription):

1. Admin initiates the device code flow:
   ```http
   POST /api/v1/admin/service-accounts/:sa_id/providers/:provider_id/connect/device-code/initiate HTTP/1.1
   Authorization: Bearer <admin_access_token>
   ```
   Response includes `user_code` and `verification_uri` for the admin to complete authorization in a browser.

2. Admin opens the `verification_uri` in a browser and enters the `user_code` to authorize

3. Admin (or frontend) polls for completion:
   ```http
   POST /api/v1/admin/service-accounts/:sa_id/providers/:provider_id/connect/device-code/poll HTTP/1.1
   Authorization: Bearer <admin_access_token>
   Content-Type: application/json

   {"device_auth_id": "...", "user_code": "..."}
   ```
4. Once authorized, the tokens are stored under the SA's identity

All three approaches keep provider credentials centrally managed by admins and avoid distributing them to SA operators.

**Option B: SA connects providers itself**

Alternatively, the SA can connect providers using its own token:

1. Admin creates the service account and saves the `client_id` + `client_secret`
2. Authenticate as the SA to get a token:
   ```
   POST /oauth/token
   grant_type=client_credentials&client_id=sa_...&client_secret=sas_...&scope=providers:write llm:proxy
   ```
3. Use the SA token to connect providers:
   - **API key:** `POST /api/v1/providers/{provider_id}/connect/api-key`
   - **OAuth:** `GET /api/v1/providers/{provider_id}/connect/oauth`
   - **List connected:** `GET /api/v1/providers/my-tokens`

The platform treats service account IDs identically to user IDs for credential storage and retrieval. Once connected (via either option), the LLM gateway resolves the SA's provider credentials automatically.

#### Provider-Specific Proxy

```http
POST /api/v1/llm/openai/v1/chat/completions HTTP/1.1
Authorization: Bearer <sa_access_token>
Content-Type: application/json

{
  "model": "gpt-4o",
  "messages": [
    {"role": "system", "content": "You are a helpful assistant."},
    {"role": "user", "content": "Summarize the latest test results."}
  ]
}
```

NyxID resolves the service account's OpenAI API key and proxies the request to OpenAI.

#### OpenAI-Compatible Gateway

Route to any provider using OpenAI-compatible format:

```http
POST /api/v1/llm/gateway/v1/chat/completions HTTP/1.1
Authorization: Bearer <sa_access_token>
Content-Type: application/json

{
  "model": "claude-sonnet-4-5-20250929",
  "messages": [
    {"role": "user", "content": "Analyze this log file..."}
  ]
}
```

NyxID detects the provider from the model prefix, translates the request format if needed, and proxies to the correct provider.

#### Check Available Providers

```http
GET /api/v1/llm/status HTTP/1.1
Authorization: Bearer <sa_access_token>
```

Returns which providers the service account has connected and their status.

### Proxy to Downstream Services

Service accounts can proxy requests to configured downstream services:

```http
GET /api/v1/proxy/<service_id>/items?query=test HTTP/1.1
Authorization: Bearer <sa_access_token>
```

Requires `proxy:*` or `proxy:<service_id>` scope.

### Provider Management

Service accounts can manage their own provider connections:

```http
# List connected providers
GET /api/v1/providers HTTP/1.1
Authorization: Bearer <sa_access_token>

# Connect a provider (e.g., store an API key)
POST /api/v1/providers/<provider_id>/connect HTTP/1.1
Authorization: Bearer <sa_access_token>
Content-Type: application/json

{
  "api_key": "sk-..."
}
```

Requires `providers:read` and `providers:write` scopes respectively.

---

## Token Expiry and Re-Authentication

**Service account tokens do not have refresh tokens.** When a token expires, the service must re-authenticate with its credentials.

| Aspect | Detail |
|--------|--------|
| Default TTL | 1 hour (3600 seconds) |
| Configurable | Via `SA_TOKEN_TTL_SECS` environment variable |
| Refresh token | Not issued |
| Renewal method | Re-authenticate with `client_credentials` grant |

### Recommended Token Management Pattern

```python
import time
import requests

class NyxIDClient:
    def __init__(self, client_id, client_secret, base_url):
        self.client_id = client_id
        self.client_secret = client_secret
        self.base_url = base_url
        self.token = None
        self.token_expiry = 0

    def get_token(self):
        # Re-authenticate if token expires within 60 seconds
        if self.token and time.time() < self.token_expiry - 60:
            return self.token

        resp = requests.post(
            f"{self.base_url}/oauth/token",
            data={
                "grant_type": "client_credentials",
                "client_id": self.client_id,
                "client_secret": self.client_secret,
                "scope": "llm:proxy",
            },
        )
        resp.raise_for_status()
        data = resp.json()
        self.token = data["access_token"]
        self.token_expiry = time.time() + data["expires_in"]
        return self.token

    def chat(self, model, messages):
        token = self.get_token()
        resp = requests.post(
            f"{self.base_url}/api/v1/llm/gateway/v1/chat/completions",
            headers={"Authorization": f"Bearer {token}"},
            json={"model": model, "messages": messages},
        )
        resp.raise_for_status()
        return resp.json()
```

### Handling Token Errors

| HTTP Status | Meaning | Action |
|------------|---------|--------|
| `401 Unauthorized` | Token expired or revoked | Re-authenticate |
| `403 Forbidden` | Insufficient scope or SA deactivated | Check scopes; contact admin |
| `429 Too Many Requests` | Rate limit exceeded | Back off and retry |

---

## Scopes and Access Control

### Available Scopes

| Scope | Access |
|-------|--------|
| `proxy:*` | All proxy endpoints |
| `proxy:<service_id>` | Specific service proxy only |
| `llm:proxy` | LLM gateway proxy requests |
| `llm:status` | LLM status endpoint |
| `connections:read` | List service connections |
| `connections:write` | Connect/disconnect services |
| `providers:read` | List providers and tokens |
| `providers:write` | Connect to providers, store API keys |

### Routes Accessible to Service Accounts

| Endpoint | Required Scope |
|----------|---------------|
| `ANY /api/v1/llm/{provider}/v1/*` | `llm:proxy` |
| `ANY /api/v1/llm/gateway/v1/*` | `llm:proxy` |
| `GET /api/v1/llm/status` | `llm:status` |
| `ANY /api/v1/proxy/{service_id}/*` | `proxy:*` or `proxy:{service_id}` |
| `GET /api/v1/connections` | `connections:read` |
| `POST /api/v1/connections` | `connections:write` |
| `GET /api/v1/providers` | `providers:read` |
| `POST /api/v1/providers/*/connect` | `providers:write` |

### Routes Blocked for Service Accounts

Service accounts cannot access human-only endpoints:

- `/api/v1/auth/*` (login, register, MFA, password reset)
- `/api/v1/users/*` (user profile)
- `/api/v1/sessions/*` (session management)
- `/api/v1/api-keys/*` (API key management)
- `/api/v1/admin/*` (admin panel)
- `/api/v1/services/*` (service definition management)
- `/api/v1/mcp/*` (MCP configuration)

---

## Token Revocation

### Revoking a Specific Token

Use the OAuth revocation endpoint:

```http
POST /oauth/revoke HTTP/1.1
Content-Type: application/x-www-form-urlencoded

token=<access_token>
&client_id=sa_a1b2c3d4e5f6a1b2c3d4e5f6
&client_secret=sas_xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx
```

### Bulk Revocation (Admin)

Revoke all active tokens for a service account:

```http
POST /api/v1/admin/service-accounts/:sa_id/revoke-tokens HTTP/1.1
Authorization: Bearer <admin_access_token>
```

### Automatic Revocation Triggers

Tokens are automatically revoked when:

- The service account's secret is rotated
- The service account is deactivated (`is_active: false`)
- The service account is deleted

Expired tokens are automatically cleaned up by a MongoDB TTL index on the `service_account_tokens` collection.

---

## Security

### Credential Security

- **Client secrets are hashed** (SHA-256) before storage -- never stored in plaintext
- **Shown once** at creation -- admin must save immediately
- **Constant-time comparison** prevents timing attacks during authentication
- **Generic error messages** prevent `client_id` enumeration

### Token Security

- **RS256 signed** JWTs verified on every request
- **Per-token revocation** via `jti` claim and `service_account_tokens` collection
- **Active check** on every request -- deactivating a service account immediately blocks all requests
- **Scope enforcement** -- tokens can only access resources within their granted scope

### Rate Limiting

- Global rate limiter applies by default
- Optional per-account `rate_limit_override` for fine-grained control

### Audit Logging

All service account operations are logged:

| Event | Description |
|-------|-------------|
| `admin.sa.created` | Service account created |
| `admin.sa.updated` | Service account settings changed |
| `admin.sa.deleted` | Service account deactivated |
| `admin.sa.secret_rotated` | Client secret rotated |
| `admin.sa.tokens_revoked` | Bulk token revocation |
| `sa.token_issued` | Successful authentication |
| `sa.auth_failed` | Failed authentication attempt |

---

## Configuration Reference

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `SA_TOKEN_TTL_SECS` | `3600` | Service account token TTL (seconds) |
| `JWT_PRIVATE_KEY_PATH` | `keys/private.pem` | RSA private key for signing |
| `JWT_PUBLIC_KEY_PATH` | `keys/public.pem` | RSA public key for verification |
| `JWT_ISSUER` | `nyxid` | `iss` claim in tokens |
| `BASE_URL` | `http://localhost:3001` | `aud` claim in tokens |
| `RATE_LIMIT_PER_SECOND` | `10` | Default rate limit |
| `RATE_LIMIT_BURST` | `30` | Rate limit burst |

---

## API Reference

### Admin Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/api/v1/admin/service-accounts` | Create service account |
| `GET` | `/api/v1/admin/service-accounts` | List service accounts (paginated) |
| `GET` | `/api/v1/admin/service-accounts/:id` | Get service account details |
| `PUT` | `/api/v1/admin/service-accounts/:id` | Update service account |
| `DELETE` | `/api/v1/admin/service-accounts/:id` | Delete (deactivate) service account |
| `POST` | `/api/v1/admin/service-accounts/:id/rotate-secret` | Rotate client secret |
| `POST` | `/api/v1/admin/service-accounts/:id/revoke-tokens` | Revoke all tokens |
| `GET` | `/api/v1/admin/service-accounts/:id/providers` | List SA's connected providers |
| `POST` | `/api/v1/admin/service-accounts/:id/providers/:pid/connect/api-key` | Connect API key provider to SA |
| `GET` | `/api/v1/admin/service-accounts/:id/providers/:pid/connect/oauth` | Initiate OAuth redirect flow for SA |
| `POST` | `/api/v1/admin/service-accounts/:id/providers/:pid/connect/device-code/initiate` | Initiate device code flow for SA |
| `POST` | `/api/v1/admin/service-accounts/:id/providers/:pid/connect/device-code/poll` | Poll device code authorization status |
| `DELETE` | `/api/v1/admin/service-accounts/:id/providers/:pid/disconnect` | Disconnect provider from SA |

### OAuth Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/oauth/token` | Authenticate (`grant_type=client_credentials`) |
| `POST` | `/oauth/revoke` | Revoke a specific token |
