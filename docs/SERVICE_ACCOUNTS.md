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
- [Platform catalog editors](#platform-catalog-editors)
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

Global admins create personal service accounts via the admin API. Organization admins can create accounts for their organization with `target_org_id`.

The scope field offers suggestions from [the options API](OPTIONS_API.md): supported code-defined scopes and scopes already configured on service accounts belonging to the authorized owner. Suggestions help prefill the field; they are not a complete permission vocabulary or an authorization grant. The field shows every selected scope as a full, wrapping pill with Edit and Remove controls. Click or keyboard-activate a pill to edit it in place; Enter finishes the replacement and Escape cancels. The dropdown hides selected values and loads all suggestion pages automatically. For colon-delimited configured values, choose prefixes to navigate to a complete scope; for example, previously configured `reports:finance:read` can be reached through `reports:` and `reports:finance:`. Prefix navigation does not grant or save an intermediate scope. Type a full custom scope and press Enter, or paste space-separated scopes. Typed custom values reach the form immediately, so Save includes an unfinished draft without changing the field layout during the click. Arrow keys explicitly select a suggestion; Enter without an active suggestion adds exactly what you typed. Custom entry remains available when suggestions cannot load. All admin, organization, shared edit, assistant, and CLI wizard forms use this picker.

Scope strings remain free-form. Existing values stay editable, and custom scopes can be created or updated without appearing in suggestions. Adding an unknown name does not create a new permission check. The picker deduplicates tokens when you edit the selection; it preserves the original stored string until an edit.

```http
POST /api/v1/admin/service-accounts HTTP/1.1
Authorization: Bearer <admin_access_token>
Content-Type: application/json

{
  "name": "CI Pipeline Bot",
  "description": "Automated CI/CD pipeline that runs LLM evaluations",
  "allowed_scopes": "llm:proxy proxy:*",
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
  "allowed_scopes": "llm:proxy proxy:*",
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
  "allowed_scopes": "llm:proxy"
}
```

Scope changes affect future token issuance. CatalogEditor also checks live account scopes on every request; removing a scope removes that authority from existing tokens. Legacy editors additionally check their assigned global role permissions. Legacy Curation checks live scopes and its grant.

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
2. Atomically advances `credential_generation` with the new secret hash, then **revokes all existing token rows** for this service account. A token issued late from an older validated secret is rejected even if its row was inserted after the revocation sweep.
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
   grant_type=client_credentials&client_id=sa_...&client_secret=sas_...&scope=llm:proxy
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

Requires `proxy` or its `proxy:*` alias. General service accounts resolve the effective owner's service connections; scope strings do not implement per-service grants. Curation accounts instead enforce the exact live Ornn target and dedicated credential boundary described below.

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

These routes use their existing authentication and ownership checks. The strings `providers:read` and `providers:write` are accepted as custom scope values but are not enforced permission gates for these operations.

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

### Scope Suggestions and Existing Checks

| Value | Existing behavior |
|-------|-------------------|
| `proxy` | Passes the proxy scope check and the LLM gateway scope check; resource and owner checks still apply |
| `proxy:*` | Existing alias of `proxy`; grants the same access, not additional access |
| `llm:proxy` | Passes the LLM gateway scope check, including status |
| `roles` | Includes assigned roles and permissions in OAuth userinfo |
| `catalog:skills:read` | Catalog metadata, key metadata, skills, and history after a platform admin saves the scope; legacy Curation uses its grant |
| `catalog:skills:write` | Catalog recommendation changes and restore after a platform admin saves the scope; legacy Curation uses its grant |
| `user-services:read` | Nonsecret private connection metadata with General/Curation connection grants; also required by legacy role-based editors for key GETs |
| `groups` | Includes groups in OAuth userinfo; service accounts have no group memberships, so the list is empty |

The default suggestion menu includes all eight values in the table, even before an account exists. The `proxy:*` alias and empty `groups` behavior are labeled explicitly. New service-account scope checks must be added to the suggestions registry. A platform administrator saving catalog scopes grants CatalogEditor access. Legacy editors retain their matching global role requirement until an explicit scope save. Legacy Curation instead requires a platform-admin-issued grant for selected catalog services. Additional values found on the owner's service accounts are labeled as custom/configured suggestions. This does not reinterpret their meaning. `llm:status`, `connections:read/write`, and `providers:read/write` do not establish separate permission checks in the current implementation. Per-service scope strings such as `proxy:<service_id>` are not supported as service restrictions.

General accounts support free-form scope strings; an explicit platform-admin save containing catalog scopes grants CatalogEditor authority. CatalogEditor accounts accept only `catalog:skills:read`, `catalog:skills:write`, `user-services:read`, and `proxy`. Legacy Curation accounts restrict scopes to their grant contract below. A requested token scope must be an exact whitespace-separated subset of the stored values; for example, configuring only `proxy:*` does not allow requesting the different string `proxy`. Changing an account's configured scopes affects subsequent token issuance. Existing tokens retain their issued scopes until expiry or explicit revocation. Curation and key metadata reads additionally check the live configured scope on every request.

### Routes Accessible to Service Accounts

The general route table below does not widen CatalogEditor or Curation access. CatalogEditor permits catalog metadata GETs, catalog-curation routes, and the Ornn HTTP target described below. Legacy Curation remains confined to its grants.

| Endpoint | Existing scope check |
|----------|----------------------|
| `ANY /api/v1/llm/{provider}/v1/*` | `proxy`, `proxy:*`, or `llm:proxy` |
| `ANY /api/v1/llm/gateway/v1/*` | `proxy`, `proxy:*`, or `llm:proxy` |
| `GET /api/v1/llm/status` | `proxy`, `proxy:*`, or `llm:proxy` |
| `ANY /api/v1/proxy/{service_id}/*` | `proxy` or `proxy:*`; Curation additionally requires the exact live Ornn target |
| `GET /api/v1/keys` | CatalogEditor: catalog read scope (legacy role-based editors also need `user-services:read` and a catalog read role); General/Curation: live connection grant and owner access |
| `GET /api/v1/keys/{uuid}` | Same authority as list; catalog UUID for CatalogEditor, connection UUID for General/Curation |
| `GET /api/v1/mcp/config` | `proxy` or `proxy:*`; General SAs only, using their own discovery identity |
| Connection/provider management | Existing route authentication and ownership checks; no separate connections/providers scope enforcement |

### Routes Blocked for Service Accounts

Service accounts cannot access human-only endpoints:

- `/api/v1/auth/*` (login, register, MFA, password reset)
- `/api/v1/users/*` (user profile)
- `/api/v1/sessions/*` (session management)
- `/api/v1/api-keys/*` (API key management)
- `/api/v1/admin/*` (admin panel)
- `/api/v1/services/*` (service definition management)
- `/api/v1/keys` without catalog editor authority or a live connection grant; slug reads, writes, and `/keys/{id}/authorization` are unavailable to SAs

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
- **Scope checks** -- proxy and LLM routes check recognized scope values alongside existing resource authorization; custom scope names do not add enforcement to other routes

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


## Platform catalog editors

A platform administrator grants catalog access by saving `catalog:skills:read` and/or `catalog:skills:write` in the ordinary service account create/edit form. These scopes cover all existing and future catalog services. There is no separate catalog role, activation action, connection grant, or service UUID allowlist.

For Aevatar, save `catalog:skills:read catalog:skills:write proxy` in **Allowed Scopes**. Keep its existing Ornn role assigned for package content operations. Then request a client-credentials token with these scopes. The existing account and secret can be used.

| Operation | Required scope on token and live account |
| --- | --- |
| `GET /api/v1/keys` | `catalog:skills:read` |
| `GET /api/v1/keys/{catalog_uuid}` | `catalog:skills:read` |
| Catalog discovery, skills, history, OpenAPI contract | `catalog:skills:read` |
| Catalog recommendation PUT or restore | `catalog:skills:write` |

Read and write are independent. Read alone authorizes both key GETs; `user-services:read` is unnecessary for this catalog metadata path. Write alone does not grant reads. The scopes authorize catalog metadata and recommendations, not execution against the services or access to private connections.

The same grant can be saved with the CLI using a platform administrator's identity:

```sh
nyxid service-account update "$SA_ID" \
  --scopes 'catalog:skills:read catalog:skills:write proxy' \
  --output json
```

This replaces the allowed scopes and preserves assigned roles. The backend records `catalog_scope_authorized: true`, `purpose: "catalog_editor"`, and `platform_protected: true` atomically with the scopes. These are server-controlled fields; ordinary request bodies cannot set them. Organization administrators cannot grant platform catalog authority. Accounts remain under platform-admin management after scope removal, so owners cannot restore revoked access or obtain unrestricted account routes.

The ordinary edit form also supports saving catalog scopes already present on an older account. Its usual change review includes the resulting platform-wide catalog grant. Access updates include the account's original roles, scopes, purpose, protection, authorization marker, and enabled state as an atomic precondition. A concurrent change returns HTTP 409; reload before saving. The CLI and existing API clients can omit this optional precondition and should inspect current account settings before replacing scopes.

Existing CatalogEditor accounts keep their previous role-based authority until an administrator explicitly saves catalog scopes. In that legacy mode, live global `nyxid:catalog:skills:read/write` permissions remain required, and key GETs additionally require `user-services:read`. Removing a legacy authorizing role still revokes access. A scope save deliberately converts the account to scope authority; subsequent catalog revocation uses the allowed scopes. Existing General or Curation accounts are not upgraded merely because their stored scopes contain these strings. Role-only and metadata-only API updates do not convert them.

For CatalogEditor, `/keys` returns `{"keys":[...]}` with `resource_type: "catalog_service"` on every row. Both `id` and `catalog_service_id` are **catalog UUIDs**. Use an `id` from this list for detail and `/catalog-curation/services/{id}/skills`; personal/org connection UUIDs are a separate namespace. Existing and disabled catalog entries are included; inspect `is_active`. Responses contain names, identifiers, type, active status, recommendations, references, revision, and manifest digest. Queries project these metadata fields without loading credentials or private connection settings. GET responses use `Cache-Control: private, no-store`.

Writes use the [machine recommendation API](#machine-recommendation-api) and retain revision conflicts, request replay protection, history, token revocation, and credential-generation checks. Scope, account, and token changes conflict with in-flight mutation transactions and force revalidation; legacy role authority is also fenced. A persisted per-account write window applies across replicas: 60 changed writes per second by default, or `rate_limit_override`. Unchanged writes and committed retries do not consume additional budget. Removing a scope blocks subsequent requests with existing tokens; a newly added scope must also be present on the token.

Ornn HTTP requests require `proxy`. Keep the assigned Ornn role permissions `ornn:skill:read`, `ornn:skill:create`, and `ornn:skill:update` for package content operations. A converted legacy Curation account retains its administrator-selected Ornn target; a fresh editor uses the active HTTP catalog service with slug `ornn-api`. That target must have the explicit operation policy described below. Downstream authentication uses the SA's own credential, unless the target is configured for identity-only/no-auth access. Configure JWT or Both identity propagation so Ornn receives the SA UUID and live Ornn permissions. Ornn still enforces ownership/sharing for package edits. Revoking catalog scopes does not revoke the separate `proxy` capability; remove `proxy`, revoke tokens, or disable the account to stop Ornn access.

The account is confined to these metadata/skill routes and its Ornn HTTP target. It cannot manage service definitions, use other proxy targets, select instances, upgrade to WebSocket, or use MCP/LLM/general account administration. Legacy exact grants cannot change a CatalogEditor's purpose or grant fallback authority.

**Deployment order:** upgrade every serving backend replica before using scope grants and deploy the frontend after the backend. Older catalog-editor binaries ignore the new marker and still require roles, so newly scope-authorized requests can fail during a mixed deployment. Existing legacy editors retain their old behavior. Binaries predating CatalogEditor cannot deserialize its purpose. Before rollback, disable affected accounts and revoke tokens; retain binaries that understand their purpose until accounts are migrated. After deployment, verify both GETs with the intended SA and a catalog ID returned by its list.

## Catalog skill curation

The following exact-grant setup remains available for deliberately limited legacy Curation workloads. Platform-wide Aevatar editors use the scope setup above; both modes share the recommendation API and Ornn content semantics below.

A dedicated service account can autonomously assign, replace, remove, clear, and restore recommended skills for exact permitted catalog services. The account has one embedded live grant and a persisted write budget shared by all backend replicas. There is no per-change approval or semantic review gate.

NyxID stores recommendation names and optional immutable references. **Ornn owns package content, visibility, ownership, and retained immutable versions.** Its existing `ornn:skill:read`, `ornn:skill:create`, and `ornn:skill:update` permissions support a service account reading public skills and creating, reading, updating, and changing visibility on its own skills. Creation records the authenticated SA UUID as `createdBy`; the author has implicit object read/write/manage authority even with an empty sharing ACL. Updating someone else's skill requires an explicit object `write` grant. Skill and version deletion require the separate `ornn:skill:delete` permission, which this role must omit. The NyxID operation policy below independently excludes deletion and unrelated management routes.

This contract was checked against the connected Ornn API's OpenAPI 0.18.0 document and [Ornn source revision e7e21e9](https://github.com/ChronoAIProject/Ornn/tree/e7e21e9b7279ccc28ef7d4a40523da40b8cabc25). Source and local tests establish the authorization rules; deployment configuration still needs verification with the intended service-account identity. The proposed content-only permission in [Ornn #1247](https://github.com/ChronoAIProject/Ornn/issues/1247) addresses a different requirement: it would deliberately deny existing-skill visibility/settings changes. It is not required for this creator-managed workflow.

### Upgrade order and rollback

Upgrade **all backend replicas that authenticate tokens, proxy requests, or serve MCP** before issuing, enabling, or delivering any Curation credentials. Older replicas ignore purpose and credential generation: a proxy-scoped token can regain General account behavior on an old replica, and the rotation fence does not protect requests authenticated there. The additive recommendation fields and separate digest support consumer compatibility; they do not make mixed-version authorization safe.

Before rolling back to a backend without these checks, disable the Curation accounts and revoke their tokens. Do not deliver or re-enable their credentials while any old replica remains. No live account provisioning or deployment is performed by the implementation itself.

### Legacy Curation platform administration

Existing legacy Curation accounts retain their exact grants and token/live scope requirements. Manage their grants from **Catalog skill curation** on the account detail page or through the routes below. Saving catalog scopes through platform-admin create/update now grants platform-wide CatalogEditor access instead; it is the supported setup for new catalog editors. Do not use an ordinary scope save to maintain exact Curation authority. Existing grant revocation, metadata updates, disable, and token revocation remain available to platform admins.

Use the existing Admin → Service Accounts detail page, or:

```sh
nyxid service-account curation-grant issue "$SA_ID" \
  --service-id "$CATALOG_SERVICE_ID" \
  --service-id "$SECOND_CATALOG_SERVICE_ID" \
  --ornn-proxy-service-id "$ORNN_CATALOG_ID" \
  --max-writes 100 --window-seconds 3600 \
  --expires-at 2026-12-31T23:59:59Z
nyxid service-account curation-grant show "$SA_ID"
nyxid service-account curation-grant revoke "$SA_ID"
```

The dedicated routes are `POST` and `DELETE /api/v1/admin/service-accounts/{id}/curation-grant`; inspection uses the normal account detail `GET`. Issuance accepts `service_ids` (1–100 distinct existing catalog UUIDs), optional `ornn_proxy_service_id`, optional future `expires_at`, `max_writes` (1–10000), and `window_seconds` (60–86400). Issuing/replacing a grant starts a fresh budget window. Ordinary account create/update requests reject grant, purpose, protection, and generation fields.

Issuance permanently sets `purpose=curation` and `platform_protected=true`. Revocation removes the live grant while both fields remain. Expiry and revocation fail closed for curation reads, writes, and Ornn proxy requests. Protected metadata, scope/role changes, disable, rotation, and provider/connection management require platform admin; direct owner or org-admin status is insufficient. Service-account role IDs stay under platform-admin control for downstream assertions and do not become NyxID platform-admin privileges.

Grant and history responses contain no credentials or secret hashes. The original issuer's later demotion does not revoke a standing workload grant. During offboarding, rotate any client secret or Ornn credential a former administrator could retain, and revoke the grant or disable the account when the workload itself should stop.

### Runtime confinement and token revocation

A Curation bearer may use `/api/v1/catalog-curation/...`, ordinary HTTP `/api/v1/proxy/{ornn_proxy_service_id}/...`, and grant-filtered `GET /api/v1/keys` or exact `GET /api/v1/keys/{uuid}` with the separate key read scope and grant below. The grant and token/live scopes must authorize the request. The curation operation-contract route (`GET /api/v1/catalog-curation/services/{catalog_service_id}/openapi.json`) is the supported way to read a grant-listed admin service's OpenAPI document. It returns the source contract, preserving upstream server declarations for authoring; it does not rewrite them into executable proxy URLs or grant execution access. Authored skills must use the consumer's authorized NyxID service connection for execution. It is read-only, bounded, and does not expose a user-managed `/keys` row. Generic catalog/services listing, other proxy IDs, slug routing, `_nyxid_via` instance selection, WebSocket upgrades, MCP, LLM/OpenAI routes, provider/connection self-management, nodes, oracle, and triggers are unavailable.

The Ornn proxy uses the granted catalog URL and the service account's own connection or delegated provider credential. It never inherits its creator's UserService, endpoint override, gateway URL, node, or broad credential, and does not fall back to a catalog master credential. Explicitly disconnecting the SA connection blocks execution. A platform admin attaches the separately scoped Ornn credential using the existing SA provider/connection management surface. General-purpose account routing keeps its existing behavior.

Every SA access token must have a live token row matching its account, JWT ID, exact scope, expiry, revocation state, and current credential generation. SA JWTs carry `sgen`; missing legacy generations count as zero only while the current account is generation zero. Rotation advances generation atomically with replacing the secret, including when old-secret issuance finishes after rotation. MCP bearer authentication and OAuth introspection use the same validation. Curation cannot use a General-era MCP session as a fallback.

### Separate Ornn identity and endpoint

Assign the SA a NyxID role whose `permissions` contain exactly
`ornn:skill:read`, `ornn:skill:create`, and `ornn:skill:update`. These are role
permissions, separate from the SA OAuth scopes. Configure JWT or Both identity
propagation on the dedicated Ornn catalog service. NyxID mints the downstream
identity assertion with `sub` equal to the SA UUID and `permissions` resolved
from that SA's roles. Ornn reads those claims as `auth.userId` and
`auth.permissions`; it does not use the administrator who created the SA.

Ornn applies two checks: the route checks the required permission in the minted
identity, and the object check uses the stored creator or sharing grants:

| Operation | Request permission | Object rule |
|---|---|---|
| Validate a package or read package JSON | `ornn:skill:read` | Package reads respect public visibility, creator ownership, or a read/write grant |
| Create a skill | `ornn:skill:create` | Records the caller's UUID as `createdBy`; starts private |
| Upload a new immutable version | `ornn:skill:update` | Creator or explicit `write` grantee |
| Change private/public visibility | `ornn:skill:update` | Creator; a `write` grant alone is insufficient |
| Delete a skill or version | `ornn:skill:delete` | Creator/object-manage authority is also required; denied because the role omits delete |

Platform administrators also pass the object checks, but this SA must not hold
`ornn:admin:skill`. Object-manage authority on a skill the SA created is an
ownership rule, not a platform-admin role and not an ACL grant that needs to be
assigned. The typed sharing ACL accepts `read` or `write`; an empty ACL means no
collaborators, not an ownerless object. For a skill created by another principal,
an owner/admin can grant the SA content write once with
`{"type":"user","id":"<SA UUID>","level":"write"}`. Public visibility alone
never grants content write access.

The existing API supports the complete creator workflow without a new scope:

1. Send a raw ZIP to `POST /api/v1/skills`. The result is private and owned by the SA.
2. Read it through `GET /api/v1/skills/{id}` or `/json` as the same SA.
3. Make it public with JSON `{"isPrivate":false}` to `PUT /api/v1/skills/{id}`.
4. Upload a newer ZIP to the same PUT route. Existing versions remain immutable.
5. Make it private again with JSON `{"isPrivate":true}` to the same PUT route.

A skill tied through Ornn's own `nyxid-service` binding to an admin service is
forced public and cannot become private until untied. That Ornn binding is
separate from assigning recommendations through NyxID's Curation API. An SA
creating a new identity after deletion/recreation gets a new UUID and does not
inherit the old identity's skills; rotating credentials on the existing SA
preserves ownership.

`ornn:skill:update` also authorizes creator-managed sharing permissions,
source configuration and refresh, version deprecation, dist-tag assignment,
ownership transfer, and service binding changes. The policy below includes
those exact routes. Ornn still checks creator/manage authority and validates
the target of each operation. Removing a dist-tag uses that update permission
despite being an HTTP DELETE, so omitting `ornn:skill:delete` alone does not
prevent tag removal. The policy deliberately excludes all DELETE routes,
including skill, version, and tag deletion.

Use direct user-type grants for an SA. Ornn's organization-grant resolution
requires a NyxID organization lookup that Curation tokens cannot perform.
Ownership transfer requires a known Ornn recipient and leaves the former owner
with read access; transferring away a skill ends the SA's creator edit rights.
The service-binding route allows an owner to untie a skill with
`{"nyxidServiceId":null}`. Creating a new Ornn binding additionally requires a
target resolvable by Ornn's caller-visible service lookup; Curation tokens do
not gain general catalog access. Assign/remove admin catalog recommendations
through the NyxID Curation API instead of relying on that separate binding.

Before enabling the workload, verify this sequence with the intended SA:
create private, read private, upload a newer version, set public, read publicly,
set private again, and confirm that another principal's private skills and
ungranted updates are denied. Confirm skill/version deletion is denied even for
the creator, and that unlisted management paths never reach Ornn. Use the
Curation API separately to assign/remove recommendations on grant-listed admin
catalog services. Private skill recommendations do not grant readers access to
private content; shared recommendations need public content or suitable Ornn
read grants.

The relevant Ornn implementation is
[`nyxidAuth.ts`](https://github.com/ChronoAIProject/Ornn/blob/e7e21e9b7279ccc28ef7d4a40523da40b8cabc25/ornn-api/src/middleware/nyxidAuth.ts),
[`authorize.ts`](https://github.com/ChronoAIProject/Ornn/blob/e7e21e9b7279ccc28ef7d4a40523da40b8cabc25/ornn-api/src/domains/skills/crud/authorize.ts),
and the CRUD
[`routes.ts`](https://github.com/ChronoAIProject/Ornn/blob/e7e21e9b7279ccc28ef7d4a40523da40b8cabc25/ornn-api/src/domains/skills/crud/routes.ts)
and
[`service.ts`](https://github.com/ChronoAIProject/Ornn/blob/e7e21e9b7279ccc28ef7d4a40523da40b8cabc25/ornn-api/src/domains/skills/crud/service.ts).

The Ornn role controls downstream authorization. It does not add OAuth scopes, attach a credential, or configure NyxID operation policy. The following checklist is for legacy Curation; CatalogEditor uses the scope setup above:

1. The NyxID account must be Curation-protected with `catalog:skills:read`,
   `catalog:skills:write`, and `proxy` in its configured scopes. The client-
   credentials request must also include `proxy` when it will call Ornn.
2. The live grant must list every catalog service whose recommendations may be
   assigned and must name the one exact Ornn catalog service as
   `ornn_proxy_service_id`.
3. A platform admin must attach a dedicated Ornn connection to the service
   account. Curation proxy resolution never falls back to the creator's
   credential or a catalog master credential.
4. The Ornn service row's `proxy_operation_policy` must explicitly allow every
   required read/write operation, including `POST /api/v1/skill-format/validate`,
   `POST /api/v1/skills`, and `PUT /api/v1/skills/{id}`. The policy is the NyxID
   route allowlist; the Ornn role is not a substitute for it.
5. Ornn must receive the SA UUID and the three role permissions in the NyxID
   identity assertion. The stored `createdBy` must match that UUID for creator
   access; only skills created by another identity need explicit write grants.
   Omit `ornn:skill:delete` and `ornn:admin:skill` from every role assigned to the SA.
   Role permissions are additive, so another assigned role must not reintroduce them.

Use a **dedicated Ornn catalog service** so other clients retain their existing
endpoint configuration. With `base_url=https://ornn.example` (origin only), configure
this existing `proxy_operation_policy` on that catalog row:

```json
{"rules":[
  {"method":"GET","path_template":"/api/v1/skill-search"},
  {"method":"GET","path_template":"/api/v1/skill-format/rules"},
  {"method":"POST","path_template":"/api/v1/skill-format/validate"},
  {"method":"GET","path_template":"/api/v1/skills/{id}"},
  {"method":"GET","path_template":"/api/v1/skills/{id}/json"},
  {"method":"GET","path_template":"/api/v1/skills/{id}/versions"},
  {"method":"GET","path_template":"/api/v1/skills/{id}/versions/{version}/download"},
  {"method":"GET","path_template":"/api/v1/skills/{id}/closure"},
  {"method":"POST","path_template":"/api/v1/skills"},
  {"method":"POST","path_template":"/api/v1/skills/pull"},
  {"method":"PUT","path_template":"/api/v1/skills/{id}"},
  {"method":"PUT","path_template":"/api/v1/skills/{id}/permissions"},
  {"method":"PUT","path_template":"/api/v1/skills/{id}/source"},
  {"method":"POST","path_template":"/api/v1/skills/{id}/refresh"},
  {"method":"PATCH","path_template":"/api/v1/skills/{id}/versions/{version}"},
  {"method":"GET","path_template":"/api/v1/skills/{id}/dist-tags"},
  {"method":"PUT","path_template":"/api/v1/skills/{id}/dist-tags/{tag}"},
  {"method":"POST","path_template":"/api/v1/skills/{id}/transfer-ownership"},
  {"method":"PUT","path_template":"/api/v1/skills/{id}/nyxid-service"}
]}
```

Paths are relative to the configured base URL. The existing POST creates privately;
use the JSON visibility update on the allowlisted PUT route to make it public.
Do not rely on an unimplemented `public=true` creation parameter. This policy denies unlisted methods/paths with
`404 Service operation not found` before execution,
including Ornn's auth-only assistant/audit/account routes. It is required both when
a grant selects the target and on every Curation resolution; removing it fails
closed. An explicit empty policy is valid and denies all operations. General
accounts keep the existing optional-policy behavior. The Ornn request role by
itself is not an execution-route allowlist. Do not grant `ornn:skill:delete`,
`ornn:admin:skill`, build or playground permissions, and do not configure wildcard route rules.

### Machine recommendation API

Acquire a token through the existing client-credentials flow. For example, with the client secret already supplied by your secret store:

```sh
curl --fail-with-body "$NYXID_URL/oauth/token" \
  --data-urlencode grant_type=client_credentials \
  --data-urlencode "client_id=$NYXID_CLIENT_ID" \
  --data-urlencode "client_secret=$NYXID_CLIENT_SECRET" \
  --data-urlencode 'scope=catalog:skills:read catalog:skills:write proxy'
```

Use the returned access token as `$CURATION_TOKEN`. The `proxy` scope is required for Ornn content operations; a token containing only the two `catalog:skills:*` scopes can read/assign recommendations but receives `403` from the Ornn proxy.
After the scope and connection checks pass, a missing operation-policy rule fails
closed as `404 Service operation not found`; that response means the exact method
and path still need to be added to the Ornn catalog row's policy.

```sh
curl --fail-with-body -H "Authorization: Bearer $CURATION_TOKEN" \
  "$NYXID_URL/api/v1/catalog-curation/services"
curl --fail-with-body -H "Authorization: Bearer $CURATION_TOKEN" \
  "$NYXID_URL/api/v1/catalog-curation/services/$CATALOG_SERVICE_ID/skills"
curl --fail-with-body -H "Authorization: Bearer $CURATION_TOKEN" \
  "$NYXID_URL/api/v1/catalog-curation/services/$CATALOG_SERVICE_ID/openapi.json"
```

CatalogEditor sees every catalog service; legacy Curation sees only grant-listed services. These are catalog `DownstreamService` UUIDs, also returned by CatalogEditor `/keys`. General/Curation connection reads use separate `UserService` UUIDs and connection grants. Catalog IDs never grant private instance access. Each skill response contains `service_id`, `recommended_skills`, optional `recommended_skill_refs`, `skills_revision`, and the separately versioned `skills_manifest_digest`. Disallowed service IDs return 404 without disclosing their content or history. API keys, delegated/relay tokens, and human session/access tokens cannot use this router.

The operation-contract route can read a hosted overlay or a custom `openapi_spec_url`
only when NyxID can fetch that URL without downstream credentials. It uses the
existing SSRF-checked, redirect-free, five-megabyte bounded fetch path and never
injects a service-account or catalog credential. An authenticated custom spec is
therefore an explicit unsupported case until a separate credentialed, read-only
contract capability is designed and deployed; metadata access through `/keys/{uuid}` does not grant credentialed fetching, MCP discovery, or general proxy access.

Replace the entire list with an observed revision and a fresh UUID `request_id`:

```http
PUT /api/v1/catalog-curation/services/{catalog_uuid}/skills
Authorization: Bearer <curation_token>
Content-Type: application/json

{
  "base_revision": 0,
  "request_id": "f53bba8b-c55f-4b73-9eaf-e4aec599f6d7",
  "recommended_skills": ["other-publisher/setup", "operations/manual"]
}
```

The same operation assigns new defaults or reassigns existing ones. To unassign one recommendation, submit the remaining names; to clear all, submit `recommended_skills: []`. If refs are present (including an empty refs array), a name change requires replacement refs or `clear_refs: true`. Explicit clearing drops refs and keeps the supplied advisory names. Advisory names are not an assertion of immutable package contents.

An immutable reference has this shape:

```json
{
  "source": "ornn",
  "skill_id": "immutable-skill-id",
  "name": "operations/manual",
  "version": "1.5",
  "sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
  "dependencies": []
}
```

`recommended_skill_refs` derives the legacy names in order; if names are also supplied, they must match. Exact versions use numeric release versions (two or three components, optional prerelease/build suffix), never `latest` or mutable tags. SHA-256 is 64 lowercase hex characters. Lists allow at most 50 distinct names/refs and 50 dependency pins per ref; source/ID/name fields are 1–256 bytes and version is at most 128 bytes. The skill input is capped at 64 KiB. Dependencies use the same pin fields. NyxID validates this shape without fetching package bytes or proving the claimed hash.

A changed request atomically validates live authority, reserves persisted budget, changes the service, increments revision, and records history plus an actor/request receipt in one MongoDB transaction. Single-token revocation and live account/grant changes conflict with an in-flight writer. Transaction abort rolls back all effects. The history has no deletion route or short TTL.

On a lost response, resend the identical entire request with the same ID. Committed replays resolve before budget checks, but always require current authorization. Reusing a committed ID with another target or different request returns 409. A stale base revision returns 409: fetch the current state and decide a new complete list, then submit it with a new ID and that revision. Do not automatically retry stale editor values. Exhausted budget returns 429 until the persisted window resets.

A pure no-op at the current revision does not consume budget, increment revision, write history or a receipt, or trigger post-commit side effects. Its ID therefore makes no durable replay claim. Human mixed metadata/skills changes share this transaction path: changed metadata with identical skills receives a durable receipt but no skill revision/history. Full request semantics, including omitted versus explicitly cleared headers, determine retry identity. Legacy human skill updates that omit `skills_revision` mean expected revision zero. The admin editor sends its observed revision only when skills change.

### History and recovery

```sh
curl --fail-with-body -H "Authorization: Bearer $CURATION_TOKEN" \
  "$NYXID_URL/api/v1/catalog-curation/services/$CATALOG_SERVICE_ID/skills/history?limit=20"
```

History is newest first. Follow `next_before_revision` using `before_revision`; `limit` is bounded to 1–100. Entries include before/after states, actor/grant/request attribution, revision, and timestamp. Restore with:

```http
POST /api/v1/catalog-curation/services/{catalog_uuid}/skills/restore
Authorization: Bearer <curation_token>
Content-Type: application/json

{
  "revision": 0,
  "base_revision": 3,
  "request_id": "9c5d004c-4c59-42b4-958f-016cb4b1f9a3"
}
```

Revision zero restores the exact legacy baseline captured before the first edit, including absent refs. Restore appends a new revision under the same authority, budget, and compare-and-swap rules; it deletes neither history nor packages. Restoring instructions cannot undo external actions a consumer already executed.

Catalog/MCP/key read surfaces expose optional refs and revision. An instance name override suppresses catalog refs, including when that override is an empty list. The existing name-based `catalog_digest` algorithm is unchanged; ref-only changes affect a separate `skills_manifest_digest` with a `v1:` prefix. Consumers see updates on their next fetch. Locally installed/copied skills do not update automatically.

Human metadata side effects remain after commit. Idempotent retries complete OIDC redirect and billing work from the current committed desired state and re-dispatch eligible endpoint discovery, without reapplying older request values over later edits. Identity propagation uses its durable reconciliation marker; unresolved reconciliation returns a conflict directing a platform admin to identity resync.


## Connection metadata reads

This section describes private connection metadata for General and legacy Curation accounts. CatalogEditor uses the catalog projection and scope grant described above; it does not need these grants.

General and Curation service accounts may call `GET /api/v1/keys` and `GET /api/v1/keys/{uuid}` with `user-services:read` in both the issued token and the live SA scopes, plus a platform-admin-issued **key read grant**. The list contains only currently readable connections named in that grant; detail requires the exact UserService UUID. Ornn role permissions, `proxy`, and catalog grants do not authorize these reads. General accounts retain their purpose; they do not need a Curation grant. A Curation account still needs its live Curation grant in addition to the key read grant.

In **Admin → Service Accounts**, add `user-services:read` to **Allowed Scopes**, then use **Connection metadata access → Grant read access** to enter the permitted connection UUIDs. Obtain a fresh client-credentials token including the new scope. No secret rotation is required.

Grant administration uses:

```http
PUT /api/v1/admin/service-accounts/{sa_id}/key-read-grant
Authorization: Bearer <platform-admin-token>
Content-Type: application/json

{
  "user_service_ids": ["<UserService UUID>"],
  "expires_at": "2026-12-31T23:59:59Z"
}
```

`expires_at` is optional. Issuance/replacement accepts 1–100 distinct existing connection UUIDs, replaces the entire grant, and validates the SA owner's access to every target and its backing endpoint before saving. Invalid replacements leave the prior grant intact. The grant binds the effective SA owner and each target's owner. `GET` on this admin route returns the grant or `null`; platform admins and operators may inspect it. `PUT` and `DELETE` require platform admin. `DELETE` revokes the grant. Issuance and revocation produce SA grant audit events; a metadata read creates no service history mutation.

The SA detail response contains only `id`, `slug`, `name`, `label`, `service_type`, `is_active`, catalog ID/slug/name, `recommended_skills`, `recommended_skill_refs`, `skills_revision`, and `skills_manifest_digest`. The list wraps the same projected entries in a `keys` array. Both omit credential material, headers, frame injections, raw endpoint/spec URLs, node configuration, OAuth configuration, and credential IDs. No credential decryption, provisioning, or OAuth reconciliation occurs. Responses use `Cache-Control: private, no-store`.

Recommendation inheritance matches existing key reads: an endpoint name override (including an explicitly stored empty list) suppresses catalog refs and revision. Otherwise the catalog's names, refs and revision are returned. Custom connections without recommendations return null names/refs/revision and the digest of the empty state. This endpoint does not persist per-instance refs or permit any writes.

Only the list and exact UUID detail GETs are allowed for SAs. Slugs, HEAD, upgrades, authorization evidence, history, and POST/PUT/DELETE remain unavailable. Disabled connections remain in the list and readable by UUID; deleted connections and missing/cross-owner backing endpoints disappear from the list and return 404 by UUID. Ungranted targets return 404 by UUID. Grant revocation/expiry, current scope removal, and owner changes deny both reads; inactive owners and lost org membership/access remove affected entries from the list and deny their detail reads. Existing token revocation, expiry and credential-generation checks still apply. Existing human, API-key and delegated key reads retain their response and authorization behavior.

Deploy all serving replicas before enabling clients; old replicas still reject the list endpoint for SAs. Existing accounts gain no access until both the scope and exact grant are configured.
