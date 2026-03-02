# NyxID Security Documentation

This document details NyxID's security architecture, threat mitigations, and operational security practices.

---

## Table of Contents

- [Encryption at Rest](#encryption-at-rest)
- [OAuth Security](#oauth-security)
- [Token Lifecycle Security](#token-lifecycle-security)
- [Identity Propagation Security](#identity-propagation-security)
- [Proxy Security](#proxy-security)
- [Credential Broker Security](#credential-broker-security)
- [Admin Access Control](#admin-access-control)
- [RBAC Security](#rbac-security)
- [Token Introspection and Revocation Security](#token-introspection-and-revocation-security)
- [Delegated Access Security](#delegated-access-security)
- [Consent Flow Security](#consent-flow-security)
- [Authentication Security](#authentication-security)
- [HTTP Security Headers](#http-security-headers)
- [Rate Limiting](#rate-limiting)
- [Threat Model](#threat-model)
- [Incident Response](#incident-response)
- [Security Controls Reference](#security-controls-reference)

---

## Encryption at Rest

### Algorithm

All sensitive data is encrypted using **AES-256-GCM** (Galois/Counter Mode):

- **Key size:** 256 bits (32 bytes), provided via `ENCRYPTION_KEY` environment variable (64 hex characters)
- **Nonce:** Random 96-bit (12-byte) nonce generated per encryption operation via `OsRng`
- **Storage format:** `nonce(12 bytes) || ciphertext || authentication_tag(16 bytes)`
- **Authentication:** GCM provides authenticated encryption -- tampering is detected on decryption

### What Is Encrypted

| Data                                | Collection               | Fields                                         |
|-------------------------------------|--------------------------|------------------------------------------------|
| Service master credentials          | `downstream_services`    | `credential_encrypted`                         |
| Per-user service credentials        | `user_service_connections` | `credential_encrypted`                       |
| MFA TOTP secrets                    | `mfa_factors`            | `secret_encrypted`                             |
| Provider OAuth client credentials   | `provider_configs`       | `client_id_encrypted`, `client_secret_encrypted` |
| User provider OAuth tokens          | `user_provider_tokens`   | `access_token_encrypted`, `refresh_token_encrypted` |
| User provider API keys              | `user_provider_tokens`   | `api_key_encrypted`                            |

### Key Management

- The encryption key must be generated using a cryptographically secure RNG: `openssl rand -hex 32`
- Never reuse the development key in production
- All instances in a horizontal scaling setup must share the same key
- Key rotation requires re-encrypting all stored data (not yet automated)
- The key is validated at startup -- all-zero keys are rejected

### Memory Protection

Decrypted secrets use the `zeroize` crate's `Zeroizing<Vec<u8>>` wrapper, which overwrites memory on drop. This prevents sensitive data from lingering in process memory after use.

---

## OAuth Security

### PKCE (Proof Key for Code Exchange)

NyxID enforces PKCE on two distinct OAuth flows:

**1. NyxID as OIDC Provider (downstream services authenticating via NyxID):**
- Method: S256 (SHA-256)
- `code_challenge` is mandatory on all authorization requests
- The code verifier is verified during token exchange

**2. NyxID as OAuth Client (connecting to external providers):**
- PKCE is used when `supports_pkce = true` on the provider configuration
- Code verifier: 32 random bytes, base64url-encoded (43 characters)
- Code challenge: SHA-256 of the verifier, base64url-encoded
- The code verifier is stored in the `oauth_states` collection (not encrypted, but short-lived)

### OAuth State / CSRF Protection

- Each OAuth initiation creates an `oauth_states` record with a unique UUID
- The state parameter maps to this record
- State records expire (short TTL) and are consumed atomically via `find_one_and_delete`
- The callback verifies the session user matches the state's `user_id` to prevent cross-user attacks

### Token Exchange Security

- **No-redirect HTTP client (SEC-H2):** Token exchange requests use a dedicated `reqwest::Client` configured with `redirect::Policy::none()`. This prevents SSRF attacks where a malicious provider could redirect the token exchange request to an internal service.
- **Error body truncation (SEC-M5):** Error responses from providers are truncated to 200 characters before storing in the database, preventing log injection or storage exhaustion.

---

## Token Lifecycle Security

### Access Tokens (NyxID-issued JWTs)

| Property         | Value                          |
|------------------|--------------------------------|
| Algorithm        | RS256 (RSA SHA-256)            |
| Key size         | 4096-bit RSA                   |
| Default TTL      | 15 minutes                     |
| Storage          | Client-side only (not in DB)   |
| Revocation       | Short TTL; no explicit revoke  |

### Refresh Tokens (NyxID-issued JWTs)

| Property         | Value                          |
|------------------|--------------------------------|
| Algorithm        | RS256                          |
| Default TTL      | 7 days                         |
| Storage          | JTI stored in DB               |
| Rotation         | New token on each refresh      |
| Replay detection | `replaced_by` chain tracking   |

### Provider OAuth Tokens (stored by credential broker)

| Property           | Value                                         |
|--------------------|-----------------------------------------------|
| Storage            | AES-256-GCM encrypted in `user_provider_tokens` |
| Refresh strategy   | Lazy refresh with 5-minute buffer before expiry |
| Refresh failure    | Status set to `refresh_failed` with error msg |
| Memory protection  | `Zeroizing` wrapper on decrypted tokens       |
| Disconnect cleanup | Status set to `revoked`, encrypted fields nulled |

### Session Tokens

| Property         | Value                          |
|------------------|--------------------------------|
| Generation       | 32 random bytes                |
| Storage          | SHA-256 hash in DB             |
| TTL              | 30 days                        |
| Revocation       | Marked `revoked` on logout     |

### API Keys

| Property         | Value                          |
|------------------|--------------------------------|
| Generation       | Random with `nyx_k_` prefix    |
| Storage          | SHA-256 hash in DB             |
| Full key         | Shown only at creation         |
| Revocation       | Soft deactivation              |

---

## Identity Propagation Security

### CRLF Injection Prevention (SEC-M1)

All identity header values pass through `sanitize_header_value()` which strips:
- Carriage return (`\r`)
- Line feed (`\n`)
- Null byte (`\0`)

This prevents CRLF injection attacks where a malicious display name like `"Alice\r\nX-Admin: true"` could inject additional headers.

### Short-Lived Identity JWTs

Identity assertion JWTs have a 60-second TTL to minimize the replay window. Claims include:
- `sub` -- User ID
- `iss` -- NyxID issuer
- `aud` -- Target service (scoped, preventing cross-service reuse)
- `jti` -- Unique token ID
- `nyx_service_id` -- Explicit service binding

### Per-Service Audience

The `aud` claim is set to the service's `identity_jwt_audience` (or `base_url` as fallback). This ensures an identity JWT captured from one service cannot be replayed against another.

### Claim Minimization

Each service explicitly opts into which claims to include:
- `identity_include_user_id` -- User ID
- `identity_include_email` -- Email address
- `identity_include_name` -- Display name

Services only receive the identity information they need.

---

## Proxy Security

### SSRF Protection

Base URLs for downstream services and OAuth provider endpoints are validated against:

- **Scheme check:** Must be `http://` or `https://`
- **Hostname blocklist:** `localhost`, `127.0.0.1`, `0.0.0.0`, `[::1]`, `metadata.google.internal`
- **Private IP ranges:** 10.0.0.0/8, 172.16.0.0/12, 192.168.0.0/16, 169.254.0.0/16, loopback

### Path Traversal Prevention

Proxy paths containing `..` or `//` are rejected to prevent path traversal attacks against downstream services.

### Header Allowlists

**Request headers forwarded to downstream:**
`content-type`, `accept`, `accept-language`, `accept-encoding`, `content-length`, `user-agent`, `x-request-id`, `x-correlation-id`

All other headers (including `Authorization`, `Cookie`) are stripped.

**Response headers returned to client:**
`content-type`, `content-length`, `content-encoding`, `content-language`, `content-disposition`, `cache-control`, `etag`, `last-modified`, `x-request-id`, `x-correlation-id`, `vary`, `access-control-*`

### Body Size Limit

Proxy requests are limited to 10 MB request body. The global body size limit for other endpoints is 1 MB.

### Slug-Based Proxy

The slug-based proxy route (`ANY /api/v1/proxy/s/{slug}/{*path}`) resolves the slug to a service UUID via a MongoDB query on the `slug` field, then delegates to the same `execute_proxy()` pipeline used by the UUID-based route. All security properties (SSRF protection, path traversal prevention, header allowlists, body size limits, credential injection, audit logging) are inherited identically.

Slug resolution only matches active services (`is_active: true`). Inactive services are not resolvable by slug.

---

## Credential Broker Security

### Injection Key Blocklist

To prevent security header injection via credential delegation, the following header names are blocked as `injection_key` values in service provider requirements:

`host`, `authorization`, `cookie`, `set-cookie`, `transfer-encoding`, `content-length`, `connection`, `x-forwarded-for`, `x-forwarded-host`, `x-real-ip`

### Provider Credential Isolation

- Provider OAuth client credentials (`client_id`, `client_secret`) are encrypted separately from user tokens
- User tokens are scoped to `(user_id, provider_config_id)` -- one user cannot access another user's tokens
- Provider deactivation atomically revokes all associated user tokens

### Credential Lifecycle

| Event                | Action                                                      |
|----------------------|-------------------------------------------------------------|
| API key connect      | Encrypted and stored; upserts existing token                |
| OAuth connect        | Code exchanged with no-redirect client; tokens encrypted    |
| Token use (proxy)    | Decrypted on-demand; lazy refresh if near expiry            |
| Disconnect           | Status set to `revoked`; encrypted fields set to null       |
| Provider deactivated | All user tokens for that provider are revoked               |

---

## Admin Access Control

### Admin Privilege Enforcement

All admin endpoints (`/api/v1/admin/*`) verify `is_admin = true` by querying the user record from the database on each request. This ensures admin status is checked against the current database state, not a cached JWT claim.

### Self-Protection Mechanisms

Admins are prevented from performing destructive actions on their own account:

| Action                | Self-protection                                    |
|-----------------------|----------------------------------------------------|
| Change admin role     | `admin_user_id != target_user_id` enforced         |
| Change active status  | `admin_user_id != target_user_id` enforced         |
| Delete user           | `admin_user_id != target_user_id` enforced         |

These checks prevent an admin from accidentally (or maliciously) locking themselves out or removing their own account.

### Session and Credential Revocation on User Disable

When an admin disables a user (`is_active = false`), the following are immediately revoked:

1. **All sessions** -- Marked as `revoked = true` in the `sessions` collection
2. **All refresh tokens** -- Marked as `revoked = true` in the `refresh_tokens` collection
3. **All API keys** -- Marked as `is_active = false` in the `api_keys` collection

Additionally, the `AuthUser` middleware extractor checks `is_active` on every request for session-based and API-key-based authentication. This means disabled users are locked out on their next request.

### Known Limitation: JWT Access Token Survival

JWT access tokens are stateless and not checked against the database. A disabled user's existing JWT access token remains valid until it expires (default: 15 minutes). This is a deliberate trade-off between performance (no DB lookup per request for JWT auth) and immediate revocation. For time-sensitive lockouts, admins should also revoke all sessions via the session revocation endpoint.

### Cascade User Deletion

When deleting a user, a two-phase approach ensures consistency:

1. **Phase 1:** Mark user as `is_active = false` (prevents authentication during cleanup)
2. **Phase 2:** Delete related documents from 8 collections: `sessions`, `refresh_tokens`, `api_keys`, `user_service_connections`, `user_provider_tokens`, `mfa_factors`, `authorization_codes`, `oauth_states`
3. **Phase 3:** Delete the user document itself

Audit log entries referencing the deleted user are preserved with an orphaned `user_id` reference for traceability.

### Admin Action Audit Logging

All admin user management actions are recorded in the audit log with:

- **Actor:** The admin performing the action (`user_id`)
- **Target:** The user being acted upon (`target_user_id`)
- **Action details:** Specific changes made (e.g., role change, status change)
- **Client metadata:** IP address and user-agent of the admin

Admin audit event types: `admin.user.updated`, `admin.user.role_changed`, `admin.user.status_changed`, `admin.user.password_reset`, `admin.user.deleted`, `admin.user.email_verified`, `admin.user.sessions_revoked`.

RBAC audit event types: `admin.role.created`, `admin.role.updated`, `admin.role.deleted`, `admin.role.assigned`, `admin.role.revoked`, `admin.group.created`, `admin.group.updated`, `admin.group.deleted`, `admin.group.member_added`, `admin.group.member_removed`.

---

## RBAC Security

### System Role Protection

System roles (`admin`, `user`) are seeded at startup with `is_system = true`. They cannot be:
- **Deleted** -- `delete_role()` rejects deletion of system roles
- **Renamed** -- `update_role()` prevents changing the `name` or `slug` of system roles
- **De-flagged** -- The `is_system` field cannot be modified via the API

### Admin-Only Endpoints

All RBAC management endpoints (`/api/v1/admin/roles/*`, `/api/v1/admin/groups/*`) require the `is_admin = true` check, which queries the user record from the database on each request (not cached from JWT claims).

### Permission Model

Permissions are opaque string tags (e.g., `users:read`, `content:write`). NyxID stores and propagates them but does not enforce them internally. Consuming applications are responsible for interpreting permission strings.

### Role Assignment Security

- Only admins can assign or revoke roles via the API
- Role assignment is additive: assigning a role that is already assigned returns a conflict error
- Revoking a role only removes the direct assignment; group-inherited roles are not affected

### Group Membership Security

- Group membership is stored as `group_ids` on the User document
- Adding a user to a group is an admin-only operation
- Removing a user from a group removes the group's inherited roles from the user's effective permissions

### Audit Trail

All RBAC operations (role CRUD, role assignment/revocation, group CRUD, member add/remove) are recorded in the audit log with the admin's user ID, the target resource, client IP, and user-agent.

---

## Token Introspection and Revocation Security

### Client Authentication Requirement

Both the introspection (`POST /oauth/introspect`) and revocation (`POST /oauth/revoke`) endpoints require client authentication via `client_id` and `client_secret` in the request body.

- **Introspection:** Returns `{"active": false}` if client authentication fails (never reveals token validity to unauthorized callers)
- **Revocation:** Returns `200 OK` regardless of authentication result (per RFC 7009, to prevent information leakage)

### Introspection Response Security

- The response includes RBAC claims (`roles`, `groups`, `permissions`) only when the token was issued with those scopes
- The `username` field (user email) is fetched from the database, not from the token, ensuring it reflects the current state
- Revoked refresh tokens return `{"active": false}` (checked against the database)

### Revocation Limitations

- **Refresh tokens:** Revoked immediately in the database (`revoked = true`)
- **Access tokens:** Cannot be explicitly revoked (stateless JWTs). They expire naturally within the configured TTL (default: 15 minutes). This is a deliberate trade-off consistent with the existing session revocation model.

---

## Delegated Access Security

### Security Properties

| Property | Mechanism |
|----------|-----------|
| **Service authentication** | Client credentials (`client_id` + hashed `client_secret`) required for token exchange |
| **User authentication** | Subject token must be a valid, non-expired NyxID access token |
| **Consent verification** | Consent record must exist for `(user_id, client_id)` before token exchange is allowed |
| **Scope limitation** | Delegated token scope is constrained to the client's configured `delegation_scopes` |
| **Time limitation** | Token exchange: 5-minute TTL; MCP injection: 5-minute TTL |
| **Renewable tokens** | Delegation tokens can be refreshed via `POST /api/v1/delegation/refresh`; each refresh issues a new token with fresh 5-minute TTL, same scope and acting client |
| **Consent-on-refresh** | Every refresh validates that the user still has active consent for the acting client; revoking consent immediately blocks future refreshes |
| **Endpoint restriction** | Delegated tokens are blocked from non-proxy/non-LLM endpoints via `reject_delegated_tokens` middleware |
| **No credential exposure** | Services never see user's provider credentials -- only NyxID resolves and injects them |
| **Chained exchange prevention** | Delegated tokens cannot be exchanged for new delegated tokens, preventing indefinite TTL extension |
| **Audit trail** | Token exchange and subsequent proxy requests are audit-logged with both `user_id` and `acting_client_id` |

### MCP Injection Security

When NyxID proxies MCP tool calls, delegation tokens are only injected for services with `inject_delegation_token: true` (default: `false`). Additional security controls:

| Property | Mechanism |
|----------|-----------|
| **Opt-in per service** | `inject_delegation_token` defaults to `false`; must be explicitly enabled by admin |
| **User intent** | User explicitly invoked the tool call; tokens are not pre-generated |
| **Scope control** | Token scope is fixed by admin config (`delegation_token_scope`); service cannot request broader access |
| **Single use window** | 5-minute TTL; token is generated per tool call; refreshable for long-running workflows |
| **Service identity** | `act.sub` claim identifies the downstream service for audit purposes |

### Threat Mitigations

| Threat | Mitigation |
|--------|------------|
| **Stolen service credentials** | Client secret is hashed (SHA-256) in storage. Constant-time comparison prevents timing attacks. |
| **Stolen subject token** | Subject tokens are short-lived (15 min). Attacker also needs client_secret for token exchange. |
| **Service acting without consent** | Consent check is mandatory at token exchange and on every refresh. Can be revoked by user at any time via consent management; revocation immediately blocks future refreshes. |
| **Scope escalation** | Delegated scope strictly limited by `delegation_scopes` on the client configuration. |
| **Chained token exchange** | Explicitly rejected: delegated tokens have `delegated: true` and are blocked from exchange. |
| **Token replay** | Delegated tokens have unique JTI and short TTL (5 min). Each refresh issues a new JTI. |
| **Downstream service replays MCP token** | 5-minute TTL. Refreshable via `/api/v1/delegation/refresh` for legitimate long-running workflows; user must remain active and consent must not be revoked for refresh to succeed. |
| **Delegated token used on restricted endpoints** | `reject_delegated_tokens` middleware blocks access to all non-proxy/non-LLM routes. |
| **User deactivation** | `AuthUser` extractor checks `is_active` for every request, including delegated tokens. |

---

## Consent Flow Security

### IDOR Prevention

Consent endpoints use the authenticated user's ID from the `AuthUser` middleware extractor, not from request parameters. This prevents insecure direct object reference (IDOR) attacks where a user could attempt to view or revoke another user's consents.

### Consent Scoping

- `GET /api/v1/users/me/consents` returns only consents belonging to the authenticated user
- `DELETE /api/v1/users/me/consents/{client_id}` deletes only the consent matching both the authenticated user's ID and the specified client ID

### Consent Lifecycle

- Consents are created or updated (upserted) during the OAuth authorization flow
- Revoking a consent does not invalidate existing tokens; it only prevents automatic re-authorization on the next OAuth flow
- Consent records include the granted scopes and optional expiration for audit purposes

---

## Authentication Security

### Password Hashing

- **Algorithm:** Argon2id (recommended by OWASP)
- **Parameters:** m=64MiB, t=3 iterations, p=4 parallelism
- **Salt:** Random per-hash via `SaltString::generate(OsRng)`
- **Max length:** 128 characters (prevents Argon2 DoS)

### Cookie Security

| Cookie              | Flags                                             | Path                      |
|---------------------|---------------------------------------------------|---------------------------|
| `nyx_session`       | HttpOnly, SameSite=Lax, Secure*                   | `/`                       |
| `nyx_access_token`  | HttpOnly, SameSite=Lax, Secure*                   | `/`                       |
| `nyx_refresh_token` | HttpOnly, SameSite=Lax, Secure*                   | `/api/v1/auth/refresh`    |

\* `Secure` flag is set when `BASE_URL` does not start with `http://localhost` or `http://127.0.0.1`.

### Authentication Chain

Requests are authenticated in this order:
1. `Authorization: Bearer <JWT>` header
2. `nyx_session` cookie (hashed, looked up in DB)
3. `nyx_access_token` cookie (JWT verified)
4. `X-API-Key` header (hashed, looked up in DB)

All methods verify the user is active before granting access.

### API Key Scope Population

When authenticating via API key, the key's configured scopes are populated into the `AuthUser.scope` field. This makes scopes available for downstream enforcement (e.g., future per-service scope requirements). Currently, no handler enforces API key scopes -- any valid API key can access all API-key-permitted endpoints. Scope enforcement is planned as a future enhancement.

---

## HTTP Security Headers

Every response includes:

| Header                       | Value                                              | Purpose                    |
|------------------------------|----------------------------------------------------|----------------------------|
| `Strict-Transport-Security`  | `max-age=31536000; includeSubDomains; preload`     | Enforce HTTPS              |
| `X-Content-Type-Options`     | `nosniff`                                          | Prevent MIME sniffing      |
| `X-Frame-Options`            | `DENY`                                             | Prevent clickjacking       |
| `Content-Security-Policy`    | `default-src 'none'; frame-ancestors 'none'`       | Restrict resource loading  |
| `Referrer-Policy`            | `strict-origin-when-cross-origin`                  | Control referrer leakage   |
| `Permissions-Policy`         | `camera=(), microphone=(), geolocation=(), interest-cohort=()` | Restrict browser APIs |
| `X-XSS-Protection`          | `1; mode=block`                                    | Legacy XSS protection      |

---

## Rate Limiting

Dual-layer rate limiting protects against abuse:

1. **Per-IP:** Sliding window counter per client IP
2. **Global:** Token-bucket algorithm as throughput safety net

| Setting                  | Default | Environment Variable        |
|--------------------------|---------|------------------------------|
| Per-IP limit             | 30/sec  | `RATE_LIMIT_BURST`           |
| Global sustained rate    | 10/sec  | `RATE_LIMIT_PER_SECOND`      |
| Global burst capacity    | 30      | `RATE_LIMIT_BURST`           |

Rate limit state is per-instance (in-memory). For distributed deployments, consider rate limiting at the reverse proxy level.

---

## Threat Model

### Assets

| Asset                          | Sensitivity | Protection                           |
|--------------------------------|-------------|--------------------------------------|
| User passwords                 | Critical    | Argon2id hashing (never stored plain)|
| Provider OAuth client secrets  | Critical    | AES-256-GCM encryption              |
| User provider tokens           | Critical    | AES-256-GCM encryption + zeroize    |
| Service master credentials     | Critical    | AES-256-GCM encryption              |
| Per-user service credentials   | Critical    | AES-256-GCM encryption              |
| MFA secrets                    | Critical    | AES-256-GCM encryption              |
| RSA private key                | Critical    | File permissions (0600)              |
| Encryption key                 | Critical    | Environment variable                 |
| Session tokens                 | High        | SHA-256 hashed in DB                 |
| API keys                       | High        | SHA-256 hashed, prefix-only display  |
| User PII (email, name)         | Medium      | Access control, audit logging        |
| RBAC assignments (roles/groups)| Medium      | Admin-only modification, audit logging |
| OAuth consent records          | Medium      | User-scoped access, IDOR prevention  |

### Threat Mitigations

| Threat                          | Mitigation                                          |
|---------------------------------|-----------------------------------------------------|
| Credential theft (DB breach)    | AES-256-GCM encryption at rest                      |
| Password brute force            | Argon2id cost parameters, rate limiting              |
| Session hijacking               | HttpOnly cookies, SameSite, short JWT TTL            |
| CSRF                            | SameSite cookies, OAuth state parameter              |
| SSRF via proxy                  | IP blocklist, hostname validation                    |
| SSRF via OAuth redirect         | No-redirect HTTP client for token exchange           |
| Path traversal via proxy        | Reject `..` and `//` in paths                        |
| Header injection                | Request/response header allowlists                   |
| CRLF injection via identity     | `sanitize_header_value()` strips CR/LF/NUL           |
| Injection key manipulation      | Blocked injection keys list for sensitive headers    |
| Token replay (identity JWT)     | 60-second TTL, per-service audience, unique JTI      |
| Token replay (refresh token)    | Single-use rotation with `replaced_by` chain         |
| Credential leakage in memory    | `zeroize` crate for secure memory cleanup            |
| Credential leakage in logs      | Custom `Debug` impls redact secrets                  |
| Error message info leakage      | Internal errors never expose details in responses    |
| Provider error leakage          | OAuth error bodies truncated to 200 chars            |
| Timing attacks on auth          | Constant-time comparison for token verification      |
| Admin privilege escalation      | Self-protection checks on role/status/delete         |
| Admin lockout                   | Cannot disable/delete own account                    |
| Orphaned data after user delete | Cascade delete across 8 collections; audit preserved |
| Disabled user continued access  | Session + refresh token + API key revocation on disable; JWT expires within 15 min |
| RBAC system role tampering      | System roles protected from deletion/rename; `is_system` not modifiable via API |
| Unauthorized RBAC modification  | All RBAC endpoints require `is_admin = true` (database check, not JWT claim) |
| Token introspection info leak   | Returns `{"active": false}` for unauthenticated callers |
| Consent IDOR                    | User ID taken from auth context, not request parameters |
| Delegated token scope escalation | Scope constrained to client's `delegation_scopes`; `reject_delegated_tokens` middleware blocks non-proxy endpoints |
| Chained token exchange          | Delegated tokens explicitly rejected as subject tokens for exchange |
| MCP token replay                | 5-minute TTL on MCP-injected tokens; per-call generation with unique JTI |

---

## Incident Response

### If ENCRYPTION_KEY Is Compromised

1. Generate a new key: `openssl rand -hex 32`
2. Stop all NyxID instances
3. Run a re-encryption migration (decrypt all data with old key, re-encrypt with new key)
4. Update the `ENCRYPTION_KEY` environment variable
5. Restart all instances
6. Audit all access logs for the compromise window

### If RSA Private Key Is Compromised

1. Generate a new key pair
2. Replace key files at the configured paths
3. Restart all NyxID instances
4. All existing JWTs are immediately invalidated
5. Users must re-authenticate

### If a Provider Token Is Compromised

1. User disconnects from the provider via `DELETE /api/v1/providers/{id}/disconnect`
2. User revokes the token directly with the provider (e.g., OpenAI dashboard)
3. User reconnects with a new token/key

### If Suspicious Activity Is Detected

1. Review the audit log: `GET /api/v1/admin/audit-log`
2. Filter by specific user: `GET /api/v1/admin/audit-log?user_id=<suspect_id>`
3. Check for unusual `proxy_request` patterns or `provider_token_connected` events
4. Check for admin action events: `admin.user.role_changed`, `admin.user.deleted`
5. Disable compromised user accounts: `PATCH /api/v1/admin/users/<user_id>/status` with `{"is_active": false}` (automatically revokes sessions, refresh tokens, and API keys)
6. If needed, force password reset: `POST /api/v1/admin/users/<user_id>/reset-password`
7. Revoke all sessions: `DELETE /api/v1/admin/users/<user_id>/sessions`
8. Rotate affected credentials

---

## Security Controls Reference

Summary of security controls by identifier (from the architecture plan):

| ID     | Control                                    | Implementation                              |
|--------|--------------------------------------------|---------------------------------------------|
| SEC-H2 | No-redirect HTTP client for token exchange | `reqwest::Client` with `redirect::Policy::none()` |
| SEC-M1 | CRLF injection prevention                 | `sanitize_header_value()` in identity_service |
| SEC-M3 | Zeroize decrypted credentials              | `Zeroizing<Vec<u8>>` wrapper in proxy_service |
| SEC-M5 | Error body truncation                      | 200-char limit on provider error messages   |
| CR-4/5/6 | N+1 query prevention                    | Batch `$in` queries for provider lookups    |
| CR-15  | Required provider enforcement              | 400 error for missing required provider tokens |
| DA-1   | Delegated token endpoint restriction       | `reject_delegated_tokens` middleware on non-proxy routes |
| DA-2   | Chained exchange prevention                | `delegated == Some(true)` check in `exchange_token()` |
| DA-3   | Consent-gated delegation                   | `consent_service::check_consent()` before token issuance |
| DA-4   | MCP injection opt-in                       | `inject_delegation_token` defaults to `false` per service |
