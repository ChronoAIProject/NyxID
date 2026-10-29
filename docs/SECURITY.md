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
- [Transaction Approval Security](#transaction-approval-security)
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

Server-held credential fields use **AES-256-GCM** (Galois/Counter Mode). Passwords and session tokens use hashes; account/profile metadata is not universally encrypted at the application layer:

- **Key size:** 256 bits (32 bytes); the local KEK comes from `ENCRYPTION_KEY` (64 hex characters), while AWS/GCP KMS providers wrap DEKs through KMS
- **Nonce:** Random 96-bit (12-byte) nonce generated per encryption operation via `OsRng`
- **Authentication:** GCM provides authenticated encryption -- tampering is detected on decryption

### Ciphertext Formats

NyxID retains legacy v0/v1/v2 decoders and adds contextual v3 storage for converted field families. `CONTEXTUAL_RECORD_WRITES=false` preserves compatible legacy writes during reader rollout. After compatible readers and writers are deployed or drained, the explicit writer acknowledgement permits contextual writes. Binary adapters dispatch using stored field-version metadata: a current-format authentication failure is terminal, and a byte prefix alone cannot distinguish historical random nonces. Legacy reads remain gated by `ALLOW_LEGACY_RECORD_READS`. Conversion is still in progress; the registry and [field inventory](security/ENCRYPTED_FIELD_INVENTORY.md) identify unfinished adapters, so strict global cutover and key retirement are not yet approved.

**v0 (legacy):**

```
nonce(12) || ciphertext || tag(16)
```

**v1 (Phase 1):**

```
0x01 || key_id(1) || nonce(12) || ciphertext || tag(16)
```

- `0x01` -- version byte
- `key_id` -- a stable 1-byte identifier derived from `SHA-256(key)[0]`
- NyxID also accepts the earlier draft Phase 1 header (`0x00` / `0x01`) for local backward compatibility during rollout

**v2 (current -- envelope encryption):**

```
0x02 || kek_id(1) || wrapped_dek_len(2 BE) || wrapped_dek(N) || data_nonce(12) || data_ciphertext || data_tag(16)
```

- `0x02` -- version byte for envelope encryption
- `kek_id` -- stable 1-byte identifier of the KEK that wrapped the DEK, derived from `SHA-256(kek)[0]`
- `wrapped_dek_len` -- big-endian u16, length of the provider-defined wrapped DEK blob (60 bytes for the local provider; bounded at 1024 bytes)
- `wrapped_dek` -- provider-defined wrapped key; the local provider uses `dek_nonce(12) || encrypted_dek(32) || dek_tag(16)`
- `data_nonce` -- random 12-byte nonce for data encryption (separate from the DEK-wrapping nonce)
- `data_ciphertext || data_tag` -- the actual credential encrypted with the per-record DEK

Each v2 encryption generates a fresh random 32-byte DEK that is used once and immediately zeroized. The KEK never directly touches plaintext data.

### Decrypt Fallback Chain

When decrypting, the system tries formats and keys in this order:

1. If data looks like v2 (len >= 33 and byte 0 == `0x02`): unwrap DEK with current KEK, decrypt data with DEK
2. If v2 and `kek_id` matches the previous KEK: unwrap DEK with previous KEK, decrypt data with DEK
3. If data looks like v1 (len >= 30 and byte 0 == `0x01`) and `key_id` matches current key: try v1 payload with current key
4. If v1 and `key_id` matches previous key: try v1 payload with previous key
5. If v1 uses the draft Phase 1 header (`0x00` / `0x01`): try current key, then previous key for compatibility
6. Try v0 (full data as `nonce || ciphertext || tag`) with current key
7. Try v0 with previous key
8. Return error if all attempts fail

Error messages do not reveal which key was tried or which format was detected.

### Rewrap Capability

The `rewrap()` method enables efficient KEK rotation for v2 ciphertexts without full re-encryption:

1. Unwraps the per-record DEK with the previous KEK
2. Re-wraps the DEK with the current KEK using a fresh random nonce
3. Replaces the header and provider-defined wrapped DEK portion (64 bytes for the local provider)
4. The data nonce, ciphertext, and authentication tag are byte-for-byte unchanged

`rewrap()` is idempotent: if the ciphertext is already wrapped with the current KEK, it returns the input unchanged. It only operates on v2 envelopes; v0 and v1 data must be fully re-encrypted via `decrypt()` + `encrypt()`.

The generic v2 parser minimum is 33 bytes; local-provider envelopes have 92 bytes of overhead. Wrapped-DEK sizes vary by provider. A bounded, operator-driven migration runner now ships for converted families, with durable leases/cursors, exact-source compare-and-swap (CAS) and findings. It does not provide universal lazy writeback or authorize key retirement. See [the migration procedure](#encrypted-record-migration).

### What Is Encrypted

| Data                                | Collection               | Fields                                         |
|-------------------------------------|--------------------------|------------------------------------------------|
| Service master credentials          | `downstream_services`    | `credential_encrypted`                         |
| Per-user service credentials        | `user_service_connections` | `credential_encrypted`                       |
| MFA TOTP secrets                    | `mfa_factors`            | `secret_encrypted`                             |
| Provider OAuth client credentials   | `provider_configs`       | `client_id_encrypted`, `client_secret_encrypted` |
| User provider OAuth tokens          | `user_provider_tokens`   | `access_token_encrypted`, `refresh_token_encrypted` |
| User provider API keys              | `user_provider_tokens`   | `api_key_encrypted`                            |
| User provider OAuth app credentials | `user_provider_credentials` | `client_id_encrypted`, `client_secret_encrypted` |
| Unified user credentials and OAuth tokens | `user_api_keys` | `credential_encrypted`, `access_token_encrypted`, `refresh_token_encrypted`, `user_oauth_client_id_encrypted`, `user_oauth_client_secret_encrypted` |
| Node request-signing secret | `nodes` | `signing_secret_encrypted` |
| Assistant conversation Agent Key | `assistant_agent_credentials` | `key_ciphertext` |
| Assistant attachments and uploads | `assistant_attachments` (both attachment and upload records) | `data_encrypted`, `text_encrypted` |
| Saved-login credentials | `saved_logins` | `username_encrypted`, `password_encrypted`, `totp_secret_encrypted` |
| Oracle login envelopes | `oracle_login_profiles`, `oracle_login_snapshots` | `encrypted_envelope` (existing context-bound format) |
| Channel bot credentials | `channel_bots` | Bot tokens, webhook secrets, app secrets, registration PIN and Lark verification/encryption material |
| OAuth broker refresh tokens | `oauth_broker_bindings` | `refresh_token_encrypted` (existing context-bound format) |
| OAuth state secrets                 | `oauth_states`           | `code_verifier`, `device_code_encrypted`, `user_code_encrypted` |

This table is a subsystem overview. The [encrypted-field inventory](security/ENCRYPTED_FIELD_INVENTORY.md) and `encrypted_record_migration::REGISTRY` include platform credentials, delivery payloads, transport credentials, trigger secrets, assistant learning fields and SSH material. Pending adapters remain implementation blockers. Configured catalog/instance default-header values use encrypted v4 adapter storage over contextual v3 envelopes; `sensitive` controls management masking. Header writes require contextual writer activation; legacy plaintext headers are accepted only by staged legacy readers until migrated. Masked edits preserve the live value, and changing a sensitive header to public requires an explicit replacement.

### Key Management

- The encryption key must be generated using a cryptographically secure RNG: `openssl rand -hex 32`
- Never reuse the development key in production
- All instances in a horizontal scaling setup must share the same `ENCRYPTION_KEY` and `ENCRYPTION_KEY_PREVIOUS`
- Only one previous key is supported at a time
- The key is validated at startup -- all-zero keys are rejected
- The `EncryptionKeys` struct redacts key material in `Debug` output to prevent accidental logging
- `/health` exposes decrypt counters so operators can confirm whether traffic still depends on `v2_previous`, `v1_current`, `v1_previous`, `v0_current`, or `v0_previous`
- v2 envelope encryption ensures the KEK never directly touches plaintext data -- only per-record DEKs encrypt data

### Key Rotation

NyxID supports zero-downtime key rotation via the `ENCRYPTION_KEY_PREVIOUS` environment variable. Existing ciphertexts continue to decrypt during the rotation window, and all new writes use the new key immediately.

**Rotation procedure:**

1. Generate a new key:
   ```bash
   openssl rand -hex 32
   ```
2. Prepare dual-key readers: keep `ENCRYPTION_KEY` on the old key and set `ENCRYPTION_KEY_PREVIOUS` to the new key. Deploy that configuration to every replica, without enabling new-key writes.
3. Confirm every reader has both keys and that the selected provider supports both IDs. Alternatively drain all writers while deploying the complete reader set. Do not proceed with old-only readers.
4. Activate new writes: set `ENCRYPTION_KEY` to the new key and `ENCRYPTION_KEY_PREVIOUS` to the old key, then roll that configuration across the prepared replicas. Both configurations can read both keys. A one-by-one new-active rollout against old-only readers is unsafe.
5. Verify the service is healthy: `curl /health`
6. New writes use the new key and the configured compatible/contextual format. Existing data requires its explicit legacy or current-field decoder and retained provider/key. A failed contextual authentication never retries an unbound legacy decoder.
7. Retain the previous key and follow [Encrypted Record Migration](#encrypted-record-migration). The runner migrates converted families explicitly; pending adapters block complete readiness.
8. Review authenticated per-field findings and the complete encrypted-field inventory. Exact-source CAS protects concurrent replacements; unresolved findings require repair and another pass. Zero old-key traffic counters do not prove dormant records migrated.
9. Remove the previous key only after all inventory/readiness blockers and retained-backup dependencies have an explicit disposition. The runner never destroys keys. Until then, do not start a second rotation.

**Important:** Only one previous key is supported, so you must finish re-wrapping/re-encrypting old-key data before rotating again.

### Encrypted Record Migration

Prepare the [independent HMAC keyset](security/KEY_LIFECYCLE.md) before changing encryption/JWT sources. Contextual format activation is a separate reader-first cutover:

1. Deploy compatible readers with `CONTEXTUAL_RECORD_WRITES=false` and `ALLOW_LEGACY_RECORD_READS=true`. Stop/drain all old readers and writers, including background producers and restartable old images.
2. Set `CONTEXTUAL_RECORD_WRITERS_DRAINED=true` and `CONTEXTUAL_RECORD_WRITES=true` on the prepared fleet. The acknowledgement is an operator assertion; startup does not discover or stop other binaries. New contextual records cannot be read by older releases.
3. Use `POST /api/v1/admin/encrypted-records` with `{"mode":"verify"}` or `{"mode":"migrate"}`. Both require contextual writes to be active. Save the returned generation; advance bounded batches with `POST /api/v1/admin/encrypted-records/{generation}/advance`. No migration runs automatically at startup.
4. Inspect `GET /api/v1/admin/encrypted-records` and `/api/v1/admin/encrypted-records/findings`. Admins start/advance walks; admins/operators read status/findings. Admin-only `POST /api/v1/admin/encrypted-records/findings/{finding_id}/lookup` provides bounded record diagnostics and optional storage rewrap. Leases, cursors and exact-source CAS allow resume without overwriting a concurrent refresh, replacement or transfer. Resolve malformed, missing-key and binding failures rather than skipping them.
5. Pending registry families prevent complete readiness. After every family has an authenticated disposition and old writers are drained, set `ALLOW_LEGACY_RECORD_READS=false`. Startup rejects strict readers with compatible writers. Retain old keys for backups and rollback readers; cloud-internal key versions require separate operator evidence even if the configured KMS resource still decrypts them.

Readiness is observation evidence about the registered fields, not proof that every retained backup is independent of an old key. There is no automatic key-destruction action or universal lazy rewrap. See [ENV.md](ENV.md#contextual-encryption-writer-cutover) for the mode matrix and [the field inventory](security/ENCRYPTED_FIELD_INVENTORY.md) for coverage. Remaining field-family conversion and restriction of raw crypto entry points are tracked in #1933 and #1934.

### Key Rotation Rollback

If a key rotation causes issues, you can roll back:

1. Set `ENCRYPTION_KEY` back to the old key value
2. Set `ENCRYPTION_KEY_PREVIOUS` to the new key (so v2 writes made during the rotation window still decrypt)
3. Restart all NyxID instances
4. Verify the service is healthy: `curl /health`

After rollback, the old key is active and the new key remains previous: both supported legacy records under the old key and v2 records written under the new key remain readable. Keep both across every replica and retained-backup dependency; rollback does not migrate either set.

### Memory Protection

Key-provider DEKs and several caller buffers use `Zeroizing`, which overwrites their owned memory on drop. `EncryptionKeys::decrypt` currently returns an ordinary `Vec<u8>`; some callers copy plaintext into ordinary strings, including proxy credentials. End-to-end plaintext lifetime hardening is tracked in #1933. Zeroizing an owned buffer does not erase copies made by transport libraries or protect a live process from memory access.

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
- The code verifier is stored encrypted in the `oauth_states` collection

### OAuth State / CSRF Protection

- Each OAuth initiation creates an `oauth_states` record with a unique UUID
- The state parameter maps to this record
- State records expire (short TTL) and are consumed atomically via `find_one_and_delete`
- The callback verifies the session user matches the state's `user_id` to prevent cross-user attacks

### Token Exchange Security

- **No-redirect HTTP client (SEC-H2):** Token exchange requests use a dedicated `reqwest::Client` configured with `redirect::Policy::none()`. This prevents SSRF attacks where a malicious provider could redirect the token exchange request to an internal service.
- **Provider error handling:** Provider paths apply their own bounded/sanitized error handling. Proxy diagnostics exclude upstream body content. A historical 200-character truncation claim is not a general guarantee that provider text is safe to store or log.

---

## Token Lifecycle Security

### Access Tokens (NyxID-issued JWTs)

| Property         | Value                          |
|------------------|--------------------------------|
| Algorithm        | RS256 (RSA SHA-256)            |
| Key size         | Generated RSA: 4096 bits; imported signing pairs and retained verifiers: at least 3072 bits, with matching signing pair enforced |
| Default TTL      | 15 minutes                     |
| Storage          | Client-side only (not in DB)   |
| Revocation       | Live active-user/session and token-class checks; ordinary OAuth tokens require the original protected consent revision and an active client |

### Signing and HMAC Key Lifecycle

One active RSA pair signs tokens; `JWT_RETAINED_PUBLIC_KEY_PATHS` supplies up to sixteen verification-only public PEMs. Verification requires RS256 and a known `kid`; unknown/missing identifiers do not trigger key probing. JWKS publishes the active and retained public keys. Imported pairs must match and all imported/retained RSA keys must be at least 3072 bits; generated keys are 4096 bits.

Before changing JWT or AES source material, export and adopt the unchanged independent HMAC keyset with `HMAC_KEYSET_PATH`. It preserves six existing domain keys and pins commitments durably; malformed, unreadable or mismatching configured files fail closed. This does not rotate HMAC keys or re-sign history. Follow [the key lifecycle runbook](security/KEY_LIFECYCLE.md), including weak-key cutover and backup recovery requirements. Do not replace audit/billing keys with new random overrides against existing chains.

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
| Memory protection  | Guarded converted reads; ordinary copies remain (see Memory Protection) |
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
| Generation       | Random with `nyx_` (general) or `nyxid_ag_` (agent) prefix    |
| Storage          | SHA-256 hash in DB             |
| Full key         | Shown only at creation         |
| Revocation       | Soft deactivation              |

API-key scopes are enforced by authentication middleware: REST proxy access requires the applicable proxy scope, and management operations require their mapped scopes. Scope checks supplement owner/service restrictions.

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
- **Cloud metadata blocklist:** `metadata.google.internal`, `169.254.169.254`, `[fd00:ec2::254]`

Private IPs (`10.x`, `172.16-31.x`, `192.168.x`, `127.x`) and `localhost` are allowed. NyxID is a self-hosted platform where services commonly run on private infrastructure, especially when accessed via node agents that inject credentials on-premise.

### Path Traversal Prevention

Proxy paths containing `..` or `//` are rejected to prevent path traversal attacks against downstream services.

### Header Allowlists

**Request headers forwarded to downstream:**
`git-protocol`, `content-type`, `accept`, `accept-language`, `accept-encoding`, `user-agent`, `x-request-id`, `x-correlation-id`, `idempotency-key`, `traceparent`, `tracestate`, `last-event-id`, `prefer`, `x-trace-id`, `range`, `accept-profile`, `content-profile`, `range-unit`, `if-range`, `if-none-match`, `if-modified-since`, `http-referer`, `x-title`, `linkedin-version`, `x-restli-protocol-version`; plus `x-openclaw-*`, `x-amz-*`, `x-goog-*`, `x-openrouter-*`. Trace headers are validated. Caller `content-length` is dropped so transport can recalculate it after transforms.

Caller `Authorization` and `Cookie` are stripped; credential injection builds downstream authentication separately. See `ALLOWED_FORWARD_HEADERS` and `ALLOWED_FORWARD_HEADER_PREFIXES` in `proxy_service.rs` for the authoritative request list and signer-managed exclusions.

**Response headers returned to client:**
`ALLOWED_RESPONSE_HEADERS` and `forwardable_response_header` in `backend/src/services/proxy_http_response.rs` define response filtering. The list includes `content-type`, `content-length`, `content-encoding`, `content-language`, `content-disposition`, `cache-control`, `etag`, `last-modified`, `x-request-id`, `x-correlation-id`, `accept-ranges`, `content-range`, `content-profile`, `range-unit`, `retry-after`, `preference-applied`, `location`, `operation-location`, and `content-location`. Location values have additional async/URL handling. Downstream CORS headers are not forwarded; NyxID owns CORS. See the implementation for transport-specific filtering.


### Body Size Limit

Authenticated proxy and MCP raw bodies default to 100 MiB (`PROXY_MAX_BODY_SIZE=104857600`), LLM routes to 10 MiB, and public proxy/public MCP to 1 MiB. Other routes have their own bounded limits; these are not one global proxy limit.

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

### Online JWT Authority and Offline Limits

Access JWT authentication checks the live active user. First-party access tokens require a live session; ordinary OAuth client tokens require the original protected consent revision, matching scope/service authority and an active OAuth client. Consent revocation or replacement invalidates the old revision; revoke then regrant cannot resurrect it. Machine, relay, delegated and service-account tokens retain their additional live checks. NyxID online verification and an external relying party verifying a signature offline have different revocation visibility: offline verifiers retain an expiry/cache window.

Legacy ordinary OAuth access tokens without protected authority fail closed. Eligible legacy refresh records can upgrade once under the [OAuth authority rollout](security/OAUTH_AUTHORITY_ROLLOUT.md) compatibility rules. Temporary authority-store failures return retryable HTTP 503 responses; they do not require reauthorization. Delegation/callback tokens still require a coordinated drain; JWT key overlap alone does not bridge that boundary.

### Cascade User Deletion

Personal deletion uses a durable accepted intent and account fence, followed by a bounded, leased worker with domain progress. Preparation may reject or defer acceptance while the account remains usable. Accepted requests prevent further authority creation and remove personally owned data through the implemented domain inventory; organization-owned resources remain separate.

Acceptance is not completion. The global token-copy admission subsystem is removed; per-owner planning and durable provider-revocation obligations replace it. Provider effects can remain pending or require operator reconciliation, and complete collection coverage, exhaustive interruption/race tests and a retained-evidence status list remain unfinished under #1930. Do not infer complete erasure from source-account absence. The [personal-erasure inventory](security/PERSONAL_ERASURE_INVENTORY.md) describes the lifecycle; the [implementation limits](SECURITY_REMEDIATION_W1_W4.md#limits) identify remaining work.

Audit/billing evidence and required Machine lifecycle/fencing records are retained unmodified. No automatic audit purge ships today; deleting or re-signing chain rows is not part of personal erasure.

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
- **Access tokens:** NyxID online admission checks first-party sessions or the applicable live OAuth consent revision/client, plus stronger Machine/relay/delegated fences. External relying parties validating JWTs offline cannot observe immediate database revocation. A refresh-token-only revocation and consent revocation are distinct operations.

---

## Delegated Access Security

### Security Properties

| Property | Mechanism |
|----------|-----------|
| **Service authentication** | Client credentials (`client_id` + hashed `client_secret`) required for token exchange |
| **User authentication** | Subject token must be a valid, non-expired NyxID access token |
| **Consent verification** | Consent record must exist for `(user_id, client_id)` before token exchange is allowed |
| **Scope limitation** | OAuth-client token exchange is constrained to the client's configured `delegation_scopes`; service-issued tokens are constrained to the admin-configured service scope |
| **Time limitation** | Token exchange: 5-minute TTL; MCP injection: 5-minute TTL |
| **Renewable tokens** | Delegation tokens can be refreshed via `POST /api/v1/delegation/refresh`; each refresh issues a new token with fresh 5-minute TTL, same scope and acting client |
| **Consent-on-refresh** | Every refresh validates that the user still has active consent for the acting client; revoking consent immediately blocks future refreshes |
| **Endpoint restriction** | Native delegated routes remain available; management access requires the exact `account:read` scope and the read-parity controls below |
| **No credential exposure** | Services never see user's provider credentials -- only NyxID resolves and injects them |
| **Chained exchange prevention** | Delegated tokens cannot be exchanged for new delegated tokens, preventing indefinite TTL extension |
| **Connected-service catalog authority** | The dedicated `mcp:catalog:read` scope is carried only by short-lived tokens with explicit service/node/resource bounds. RFC 8693 exchange intersects source claims, both client consents, receiver delegation scope, requested service/node bounds, and RFC 8707 resources; `All` cannot widen a restricted source and an empty restricted set remains deny-all. |
| **Catalog token revocation** | Catalog grants are anchored by JWT `jti` in `catalog_delegation_grants`; every authenticated request checks expiry, revocation, actor/receiver identity, active clients, current delegation scopes, and current consent bounds. |
| **Exact catalog representation binding** | Delegated discovery remains `nyxid-delegated-operation-catalog.v2`. It publishes a sorted generic-free service/operation projection with additive `risk`, `supports_idempotency_key`, and nullable `operation_generation` fields. Create, observe, and redeem accept both the current projection digest and the bounded pre-additive v2 digest. New rows retain the pre-additive digest for old replicas and separately bind the full digest for new replicas, so old-writer policy changes remain detectable during an ordinary rolling deploy. Durable endpoints publish a positive producer generation; instance-spec operations remain visible with `null` and rely on their endpoint-contract digest. |
| **Delegated exact-approval authority** | Delegated exact-service requests must carry `mcp:catalog:read` on create, status and redeem (`403 delegated_catalog_scope_required` otherwise), closing the path where a proxy-only delegated token reached these routes without live catalog-grant revalidation. Delegated callers must supply `exact_view_digest` at create/redeem. `operation_generation` is advisory at create: NyxID always persists the live producer generation when one exists and logs a caller mismatch only at debug level. Observe/redeem compare that stored value; generationless instance-spec operations skip only this comparison. |
| **Exact-service execution-authority binding** | Create persists a real v1 digest plus an explicit `{ projection_version, digest }` for `nyxid-exact-execution-authority.v2` over the resolved execution inputs: destination URL, auth method, credential id+epoch, identity injection, default headers, proxy operation policy, configured node bindings, `service_category`, `requires_user_credential`, and an execution-only digest of `token_exchange_config`. Old replicas validate the v1 slot and new replicas validate v2. New replicas also validate main-era v1-only rows against the live v1 projection; pre-digest rows skip this one expiry-bounded gate, while genuinely unknown future versions fail closed. Observe and redeem share the ordered live evaluator `catalog/exact generation -> policy -> read-only execution authority`. Observe is transient and read-only; redeem claims first and durably records terminal revalidation results. Credential materialization runs only after those gates and is followed by another authority comparison before effect. Mismatch is HTTP 200 `state: drifted` / `failure_code: execution_authority_drift` with zero provider effect. `credential_epoch` bumps only on user-initiated credential replacement, not background token refresh. An `executing` claim left stale for 15 minutes becomes durable `failed/provider_outcome_unknown` and is never automatically replayed. Direct DNS/connect/TLS-handshake and request-build failures terminate as `failed/provider_unreachable`; all other in-flight transport failures, timeouts, and response-body failures remain outcome-unknown. |
| **Audit trail** | Token exchange and subsequent proxy requests are audit-logged with both `user_id` and `acting_client_id` |

### Account Read Parity

A service-issued delegated token with the exact, case-sensitive `account:read` scope can read the user's existing management resources through their normal endpoints. The request must use `GET`, must not be a WebSocket upgrade, and remains subject to each handler's existing personal and organization ACLs. Writes are never enabled by this scope.

The read path denies `admin/**`, `ssh/**`, `assistant/**`, `auth/**`, `devices/**`, `cli-pairings/**`, `services/**`, `mcp/**`, `webhooks/**`, `integrations/**`, `billing/**`, `oracle/**`, `channel-bots/**`, and `channel-conversations/**`. It also denies `nodes/ws`, node pending-credential protocol routes, provider OAuth initiation/callback routes, and `orgs/{org_id}/invites` because org invite responses carry redeemable bearer nonces. Ordinary organization list, detail, member, role-scope, and feature-flag reads remain available. Native delegated routes under `llm`, `delegation`, `proxy`, `demo`, `channel-relay`, and `channel-events`, plus exact approval-status polling, retain their existing behavior. Proxy handlers validate the original encoded URI before Axum wildcard decoding, so percent-encoded path separators still return `400 Invalid proxy path` on native proxy routes.

The `AuthUser` extractor applies the authoritative decision only after verifying the JWT signature and requiring `delegated: true` and `act` to be present together. `reject_delegated_tokens` performs the same decision using an unverified claim peek only as a fail-fast optimization. Accepted management reads emit metadata-only `delegated_account_read` tracing events; they do not create tamper-evident audit-chain rows.

Delegated JWTs also fail closed on routes outside `/api/v1` whenever those routes use `AuthUser` or `OptionalAuthUser`; `account:read` does not make top-level OAuth routes such as `/oauth/userinfo` delegated-native. Required-auth extractors return the delegated-policy `403`, while `OptionalAuthUser` treats the client rejection as absent optional credentials and logs it at debug level; genuine internal extractor failures remain error-level events. The top-level pending-credential accept pages take no authentication extractor and remain public HTML delivery, but their credential API calls stay inside the explicitly denied pending-credential route class.

OAuth clients cannot configure or exchange for `account:read`, and delegation refresh cannot re-add it. The scope is confined to admin-configured downstream and user-service rows; a service must receive a newly minted token from an eligible invocation. Its bearer-token replay window is the existing five-minute delegated-token TTL and must not be increased.

The separate `mcp:catalog:read` capability is intentionally not an account-read, proxy, approval, management, or secret-delivery grant. Its delegated JWT uses `sub` for the owner, `act.sub` for the source/acting OAuth client, and `client_id` for the receiving OAuth client. Catalog authority claims are explicit (`allow_all_services`/`allowed_service_ids`, `allow_all_nodes`/`allowed_node_ids`, and `resources`); missing or contradictory claims fail closed. Refresh preserves both client identities and rechecks both clients and both consent records.

Channel media downloads (`GET /api/v1/channel-relay/messages/{message_id}/attachments/{index}`) are explicitly denied by `delegated_read_denied_path`, even though other native channel-relay routes retain their own authorization. `GET /api/v1/channel-platforms` exposes metadata only and is allowed for delegated `account:read`; it is not a denied route.

New management `GET` routes default to readable for `account:read`. Route authors must add any new secret-delivery, execution, streaming, WebSocket-upgrade, or authentication/provisioning protocol route to `delegated_read_denied_path` in `mw/auth.rs` before mounting it.

### MCP Injection Security

When NyxID proxies MCP tool calls, delegation tokens are only injected for services with `inject_delegation_token: true` (default: `false`). Additional security controls:

| Property | Mechanism |
|----------|-----------|
| **Opt-in per service** | `inject_delegation_token` defaults to `false`; must be explicitly enabled by admin |
| **User intent** | User explicitly invoked the tool call; tokens are not pre-generated |
| **Scope control** | Token scope is fixed by admin config (`delegation_token_scope`); service cannot request broader access |
| **Single use window** | 5-minute TTL; token is generated per tool call; refreshable for long-running workflows |
| **Service identity** | `act.sub` claim identifies the downstream service for audit purposes |

The service-issued `sandbox:execute` scope is limited to configured downstream
and user-service rows. It is not available through OAuth-client token exchange.

### Threat Mitigations

| Threat | Mitigation |
|--------|------------|
| **Stolen service credentials** | Client secret is hashed (SHA-256) in storage. Constant-time comparison prevents timing attacks. |
| **Stolen subject token** | Subject tokens are short-lived (15 min). Attacker also needs client_secret for token exchange. |
| **Service acting without consent** | Consent check is mandatory at token exchange and on every refresh. Can be revoked by user at any time via consent management; revocation immediately blocks future refreshes. |
| **Scope escalation** | OAuth-client scopes are strictly limited by `delegation_scopes`; `account:read` is rejected during client validation, token exchange, and refresh. Service-issued scopes are admin-configured. |
| **Chained token exchange** | Explicitly rejected: delegated tokens have `delegated: true` and are blocked from exchange. |
| **Token replay** | Delegated tokens have unique JTI and short TTL (5 min). Each refresh issues a new JTI. |
| **Downstream service replays MCP token** | 5-minute TTL. Refreshable via `/api/v1/delegation/refresh` for legitimate long-running workflows; user must remain active and consent must not be revoked for refresh to succeed. |
| **Delegated token used on restricted endpoints** | The verified `AuthUser` extractor enforces exact scope, GET-only access, the deny classes, and the no-WebSocket-upgrade rule; middleware rejection is fail-fast only. |
| **User deactivation** | `AuthUser` extractor checks `is_active` for every request, including delegated tokens. |

---

## Transaction Approval Security

### Webhook Verification

Telegram webhooks are verified using constant-time comparison of the `X-Telegram-Bot-Api-Secret-Token` header. To prevent timing side-channels from length differences, both the received and expected values are pre-hashed with SHA-256 before the constant-time comparison:

```rust
let h1 = Sha256::digest(received.as_bytes());
let h2 = Sha256::digest(expected.as_bytes());
h1.ct_eq(&h2).into()
```

The `subtle` crate provides the `ConstantTimeEq` trait.

### Replay Prevention

Approval decisions are processed atomically using MongoDB `findOneAndUpdate` with filter `{ status: "pending" }`. Once status changes from `pending`, subsequent callbacks for the same request are no-ops. The webhook handler always returns 200 OK regardless of outcome to prevent Telegram retries.

### Chat ID Binding

When processing a Telegram callback, the handler verifies that `callback_query.message.chat.id` matches the `telegram_chat_id` stored on the approval request. This prevents a user from approving another user's requests by spoofing callback data.

### Link Code Security

| Property       | Value                                              |
|----------------|----------------------------------------------------|
| Format         | `NYXID-` prefix + 8 alphanumeric characters        |
| Entropy        | ~41 bits (36^8 combinations)                       |
| Expiry         | 5 minutes                                          |
| Usage          | Single-use (cleared after successful linking)      |
| Generation     | `rand::thread_rng()` with `gen_range(0..36)`       |

### Idempotency

Approval requests use a SHA-256 idempotency key computed from `(user_id, service_id, requester_type, requester_id)`. The `idempotency_key` field has a unique index in MongoDB. If a pending request with the same key exists, it is returned instead of creating a duplicate. Duplicate key errors from race conditions are handled gracefully.

### Status Polling Endpoint Security

The `GET /api/v1/approvals/requests/{request_id}/status` endpoint requires authentication and verifies the original caller binding:
- `approval_request.user_id` matches the authenticated resource owner
- `approval_request.requester_type` matches the caller auth method
- `approval_request.requester_id` matches the caller identity (user ID, service account ID, or delegated client ID)

This prevents cross-caller polling even if another authenticated principal obtains a `request_id`.

### Approval Decision Authorization

The `POST /api/v1/approvals/requests/{request_id}/decide` endpoint verifies that the authenticated user's ID matches the `user_id` on the approval request. Only the resource owner can approve or reject their own requests.

### Error Message Safety

- Telegram messages show service name and requester label but never show credentials or internal IDs
- The `ApprovalRequired` error response (code 7000) includes the `request_id` but no credential information
- Internal errors during webhook processing are logged but never returned to Telegram

### Audit Trail

All approval decisions are recorded in the audit log with:
- `request_id` and `service_id`
- Whether the request was approved or rejected
- The decision channel (`telegram` or `web`)
- Telegram account linking and disconnection events

### Threat Mitigations

| Threat                                | Mitigation                                                    |
|---------------------------------------|---------------------------------------------------------------|
| Webhook spoofing                      | Constant-time secret verification (pre-hashed SHA-256)        |
| Replay of approval callbacks          | Atomic `findOneAndUpdate` with `status: "pending"` filter     |
| Cross-user approval via spoofed chat  | Chat ID binding verification on callback processing           |
| Duplicate approval requests           | SHA-256 idempotency key with unique index                     |
| Link code brute force                 | 5-minute expiry, ~41 bits of entropy                          |
| Approval request enumeration          | UUID v4 provides ~122 bits of unguessability                  |
| Stale pending requests                | Background task auto-expires after configured timeout         |
| Credential leakage in Telegram messages | Only service name and requester label shown; no secrets     |

---

## Consent Flow Security

### IDOR Prevention

Consent endpoints use the authenticated user's ID from the `AuthUser` middleware extractor, not from request parameters. This prevents insecure direct object reference (IDOR) attacks where a user could attempt to view or revoke another user's consents.

### Consent Scoping

- `GET /api/v1/users/me/consents` returns only consents belonging to the authenticated user
- `DELETE /api/v1/users/me/consents/{client_id}` deletes only the consent matching both the authenticated user's ID and the specified client ID

### Consent Lifecycle

- Consents are created or updated (upserted) during the OAuth authorization flow
- Revoking consent removes its live authority and revokes refresh records. NyxID online admission rejects ordinary OAuth tokens bound to that revision; later regrant cannot resurrect them. Offline signature verification retains its expiry/cache limitation
- Consent records include the granted scopes and optional expiration for audit purposes

---

## Authentication Security

### Password Hashing

- **Algorithm:** Argon2id (recommended by OWASP)
- **Parameters:** m=64MiB, t=3 iterations, p=4 parallelism
- **Salt:** Random per-hash via `SaltString::generate(OsRng)`
- **Max length:** 128 UTF-8 bytes (prevents Argon2 DoS)

### MFA enforcement and recovery

Login MFA accepts a six-digit TOTP, a current 32-hex recovery code, or a legacy eight-character recovery code through the shared login-factor verifier. Recovery hashes use bounded Argon2 verification on a blocking pool, and redemption atomically removes the exact matching hash from the live verified factor. Only one concurrent redemption succeeds. Challenge expiry, attempt limits, owner/factor checks and account fences still apply. Enrollment verification remains TOTP-only. Recovery use emits metadata-only audit and an owner notification; codes are never included. Web, login-approval and mobile inputs support both login factors.

MFA enrollment and management are independent of `EMAIL_AUTH_ENABLED` and require first-party human authority. Disable requires a live TOTP or recovery code and, for password accounts, the current password. Password proof, factor redemption and the existing `mfa_state_version` generation fence commit together. Recovery is consumed once. Enable/disable/cancellation and rejected explicit unverified authority produce metadata-only audits; disable sends an owner notification through the existing recovery notification delivery mechanism.

Google, GitHub and Apple web callbacks always require NyxID MFA after the provider first factor. Mobile redirects (including Apple form_post) and native social id_token exchange resolve the code-declared `auth:mobile-mfa` flag for the signing-in person, including live org overrides. The flag defaults **off**: an MFA account receives a session explicitly marked `mfa_assured: false, mfa_exemption: "mobile_rollout"`. While off, anyone holding the account's social first factor can sign in through mobile without a code. Turning it on restores the mobile fragment challenge / OAuth `mfa_required` response; existing exempt sessions expire normally. Email/password mobile login always requires MFA. Flag resolution occurs only at sign-in, never per authenticated request. Skipped mobile MFA writes metadata-only `mfa: skipped_mobile_rollout` audit metadata: `social_login` for redirect sign-in and `social_token_exchange` for native exchange. Both paths also emit the typed `auth.mobile_mfa_exempted` telemetry event through the existing consent gates.

Social challenge creation retains the live active-person/MFA account fence and dedicated five-minute challenge in one transaction. Web challenges travel only in an HttpOnly, Secure (production), SameSite=Lax cookie scoped to `/api/v1/auth/mfa`; mobile uses the callback URI fragment. Verify clears terminal cookies, retains live challenges on a wrong code, and cancellation deletes the proof. Challenge values never enter logs, audit, telemetry or web query parameters.

Completed sessions distinguish verified, explicit unverified, legacy (absent/null assurance), and mobile-exempt authority through shared session policy. Cookie/bearer auth, first-party refresh and MCP fallback accept legacy and mobile-exempt devices until expiry/revocation. Only explicit `mfa_assured: false` without an exemption is rejected for an MFA account; rejection atomically revokes that session, refreshes and MCP fallback and audits once. Every new session serializes explicit assurance. Legacy sessions retain their fixed 30-day expiry. Sid-less first-party JWTs were issued only by legacy code and remain usable for their token lifetime. On first refresh of a sid-less nil-client token, a transaction binds the active token to one explicitly classified `legacy_migration` session with a fixed 30-day lifetime; rotation and retries reuse it. This ends the rolling grandfathered chain and brings revocation/sign-out-others under ordinary session controls. Parent-less legacy MCP fallback for MFA accounts expires at `created_at + SESSION_TTL_SECS` (30 days), even when its idle expiry has renewed; rejection removes the row, slots and notifications with one audit. Non-MFA MCP fallback retains its historical lifetime.

Third-party OAuth authorize/consent is SSO from existing IdP authority and accepts legacy/mobile-exempt sessions. Account management, such as API-key management, is also not a NyxID sign-in derivation. A new NyxID sign-in derived from an existing device requires verified authority for an MFA account unless the scoped app-approved sign-in exception below is on.

Existing-device exemptions by themselves never authorize a new device for MFA accounts. The app-approved sign-in preference below can permit explicit QR/manual-code approvals and their bound deliveries. Ordinary login-code minting, account code redemption and CLI token derivation still require verified authority. Authentication carries the already-loaded live account MFA policy and exact session assurance into a shared pure derivation guard, preserving the user-read count (including MCP). Current approvers receive HTTP 403 / `2003 mfa_step_up_required`, preserving their session. `POST /api/v1/auth/mfa/step-up` accepts `{ "code": "..." }` from a first-party human cookie or bearer with `sid`; API keys, service accounts, delegated/relay and OAuth-client tokens receive the standard denial. The login verifier supplies its shared per-account 10-attempt/300-second rate budget, TOTP replay protection, recovery consumption and account-generation fence. Session upgrade, exemption removal and metadata-only `mfa_step_up` audit commit in that transaction. Wrong/replayed codes return 400; accounts without MFA return 400. Web prompts retry the original action after success. Mobile shows that prompt only when `/users/me` capabilities include the flag; otherwise it directs the user to the NyxID website to verify and approve or create a login code. Request/account changes discard pending retries. No prompt auto-approves a sign-in.

Rollout requires no environment variables or startup cutoff. The flag defaults off in production and tests. Existing devices keep working until normal expiry; enrollment still verifies the current identified session and transactionally signs out every other session/refresh/MCP fallback. #1970 is unreleased and is not part of v0.73.0. Production replicas at v0.73.0 or earlier never enforced session assurance, so no production replica rejects legacy sessions during this upgrade. Sessions issued by v0.73.0 replicas during a mixed rollout, including MFA-less web social sign-ins, lack assurance and upgraded replicas treat them as existing devices. Rolling back to v0.73.0 removes social MFA enforcement. Only environments that ran unreleased #1970 builds have rejecting replicas to drain. Deploy all replicas before enabling mobile MFA or deploying the step-up web UI. In-flight device codes, login codes and login approvals written by old replicas without assurance continue to fail closed; retry those sign-ins. Device delivery and browser handoff creation revalidate the persisted approval proof independently of whether its prepared session is legacy. This rollout does not backfill assurance or upgrade a legacy record into verified authority.

### App-approved sign-in MFA preference

The per-person **Skip MFA for NyxID app sign-ins** setting defaults on, including
accounts without a stored preference. Only an explicit off disables it. Turning
it back on with MFA enrolled requires a verified, identified first-party session;
without MFA either value may be saved. A
signed-in human can then explicitly approve QR or typed sign-in requests without
additional TOTP from a legacy/mobile-exempt parent. Review only codes shown on
your own screen: possession of that phone/browser session is sufficient to
approve another login while the setting is on. API keys, service accounts,
delegated clients and sid-less bearers cannot invoke the exception.

The exception is scoped to device/Agent Key sign-in approval and app identity
verification. Ordinary password/social MFA and standalone login-code issuance
remain strict. Sessions preserve real MFA evidence: policy-exempt sessions have
`mfa_assured: false` and an explicit `approved_sign_in_policy` exemption. Every
exempt grant/proof binds the policy revision and approval/final delivery write
fences commit with claims. Disabling blocks pending exempt grants even after
re-enabling; completed sessions keep their fixed expiry and can be revoked.
MFA disable preserves the choice and rotates the revision to invalidate pending
proofs. Changes, policy cleanup and default-driven bypass use are audited.

With default-off `auth:mobile-mfa`, a social first factor can establish a
mobile-exempt session and then approve a web QR sign-in without NyxID TOTP by
default. Turning this setting off closes that derivation path; password/social
web login and unrelated credential issuance retain their MFA checks.

Use a coordinated traffic cutover to a fully upgraded backend fleet. Default-on
approvals write this exemption immediately; old replicas cannot deserialize it
and may return 500 during a mixed-fleet rollout. Before
rolling back to an older backend, revoke sessions with
`mfa_exemption: "approved_sign_in_policy"` and drain pending exempt grants/proofs.
See [device login](DEVICE_LOGIN_PROTOCOL.md#mfa-skip-for-app-approved-sign-ins).


### Cookie Security

| Cookie              | Flags                                             | Path                      |
|---------------------|---------------------------------------------------|---------------------------|
| `nyx_session`       | HttpOnly, SameSite=Lax, Secure*                   | `/`                       |

\* `Secure` flag is set when `BASE_URL` does not start with `http://localhost` or `http://127.0.0.1`.

NyxID's first-party web app uses only the `nyx_session` cookie for browser authentication. Mobile apps and OAuth clients use bearer access tokens and explicit refresh-token request bodies instead of browser auth cookies.

The first-party browser device-code login uses the same cookie contract: a successful public `POST /api/v1/auth/device/poll-web` atomically claims delivery, revokes the approval-created JWT session, creates a fresh browser session bound to the polling browser's IP and user agent, and returns only `{ "ok": true }` with `nyx_session` set. The route never returns access or refresh tokens and shares the JSON poll rate limiter; integrators must use `/poll` for token delivery.

### Authentication Chain

Requests are authenticated in this order:
1. `Authorization: Bearer <JWT>` header
2. `nyx_session` cookie (hashed, looked up in DB)
3. `X-API-Key` header (hashed, looked up in DB)

All methods verify the user is active before granting access. The shared session
authority predicate accepts only authenticated-purpose rows or completed legacy
rows. Dedicated MFA challenges, explicit pending purposes and legacy
`user_agent = "mfa_pending"` proofs cannot authenticate as `nyx_session` cookies;
only the dedicated MFA completion flow can redeem a live challenge.

### Password changes

`POST /api/v1/auth/password/change` requires verified first-party human authority
and the current stored password. It accepts new passwords of 8–128 UTF-8 bytes,
including the current password, and cannot create a passwordless credential.
A transaction compares the verified hash and live active-person/not-deleting
state, replaces the credential, clears reset tokens, and cancels pending MFA
challenges and login approval proofs through the shared session-authority helper.
Password-proof creation and completion fence the same user document, so a stale
first factor cannot acquire authority after the change. Completed sessions and
refresh authority remain valid. Reset completion instead checks the live token
and expiry against MongoDB server time and revokes all session authority in its
transaction. See [API.md](API.md#post-apiv1authpasswordchange)
for errors and the [account security guide](site/web/guides/account-security.md)
for session revocation.

### Browser CSRF Protection

Unsafe private API requests that look browser-originated, or that carry browser auth cookies, must present an `Origin` or `Referer` matching either `FRONTEND_URL` or `BASE_URL`. This protects first-party cookie-authenticated browser sessions from cross-site request forgery while leaving bearer-token, API-key, and OAuth token endpoint traffic unaffected.

### API Key Scope Population

When authenticating via API key, configured scopes populate `AuthUser.scope` and are enforced by middleware and management helpers such as `ensure_management_write_scope`, together with service/node restrictions. Scope presence alone does not grant every endpoint: actor class, delegated-read restrictions, ownership and live execution authority remain applicable.

---

## HTTP Security Headers

Backend security-header middleware sets the following headers on its responses. The separately served SPA and deployment edge need their own configuration; this table does not establish their headers:

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

Password changes use
dedicated `password_change_ip` and `password_change_user` namespaces with limits
of 20 attempts/IP/minute and 5 attempts/account/minute. The IP comes from the
trusted-proxy resolver. Successful changes consume attempts and do not reset
counters; both limits run before password verification.

Production global, per-IP and per-agent enforcement includes MongoDB-backed shared state. Some local guards and in-memory mechanisms remain; inspect each limiter rather than assuming all state is per-instance. Reverse-proxy limits can add another layer.

---

## Threat Model

### Assets

| Asset                          | Sensitivity | Protection                           |
|--------------------------------|-------------|--------------------------------------|
| User passwords                 | Critical    | Argon2id hashing (never stored plain)|
| Provider OAuth client secrets  | Critical    | AES-256-GCM encryption              |
| User provider tokens           | Critical    | AES-256-GCM encryption; partial plaintext zeroization    |
| Service master credentials     | Critical    | AES-256-GCM encryption              |
| Per-user service credentials   | Critical    | AES-256-GCM encryption              |
| MFA secrets                    | Critical    | AES-256-GCM encryption              |
| RSA private key                | Critical    | File permissions (0600)              |
| Encryption key (KEK)           | Critical    | Environment variable, wraps per-record DEKs |
| Session tokens                 | High        | SHA-256 hashed in DB                 |
| API keys                       | High        | SHA-256 hashed, prefix-only display  |
| User PII (email, name)         | Medium      | Access control, audit logging        |
| RBAC assignments (roles/groups)| Medium      | Admin-only modification, audit logging |
| OAuth consent records          | Medium      | User-scoped access, IDOR prevention  |

### Threat Mitigations

| Threat                          | Mitigation                                          |
|---------------------------------|-----------------------------------------------------|
| Credential theft (DB breach)    | AES-256-GCM envelope encryption (per-record DEKs)   |
| Password brute force            | Argon2id cost parameters, rate limiting              |
| Session hijacking               | HttpOnly cookies, SameSite, short JWT TTL            |
| CSRF                            | SameSite cookies, OAuth state parameter              |
| SSRF via proxy                  | Cloud metadata blocklist, scheme validation           |
| SSRF via OAuth redirect         | No-redirect HTTP client for token exchange           |
| Path traversal via proxy        | Reject `..` and `//` in paths                        |
| Header injection                | Request/response header allowlists                   |
| CRLF injection via identity     | `sanitize_header_value()` strips CR/LF/NUL           |
| Injection key manipulation      | Blocked injection keys list for sensitive headers    |
| Token replay (identity JWT)     | 60-second TTL, per-service audience, unique JTI      |
| Token replay (refresh token)    | Single-use rotation with `replaced_by` chain         |
| Credential leakage in memory | Owned key buffers use zeroization; ordinary plaintext copies remain (see Memory Protection) |
| Credential leakage in logs | Identified secret-bearing types have redacted Debug; this is not a repository-wide logging guarantee |
| Error message info leakage      | Internal errors never expose details in responses    |
| Provider error leakage | Provider paths use sanitization/metadata rules; proxy diagnostics exclude upstream bodies. Inspect each path; there is no universal 200-character guarantee |
| Timing attacks on auth          | Constant-time comparison for token verification      |
| Admin privilege escalation      | Self-protection checks on role/status/delete         |
| Admin lockout                   | Cannot disable/delete own account                    |
| Orphaned data after user delete | Durable accepted intent, account fence and resumable domain worker; complete inventory and remote-obligation semantics remain unfinished under #1930. Audit/billing and required Machine evidence retained |
| Disabled user continued access | Live active-user/session checks; ordinary OAuth tokens also check their original consent revision and active client |
| RBAC system role tampering      | System roles protected from deletion/rename; `is_system` not modifiable via API |
| Unauthorized RBAC modification  | All RBAC endpoints require `is_admin = true` (database check, not JWT claim) |
| Token introspection info leak   | Returns `{"active": false}` for unauthenticated callers |
| Consent IDOR                    | User ID taken from auth context, not request parameters |
| Delegated token scope escalation | OAuth-client scopes exclude `account:read`; verified claims gate GET-only management parity and denied route classes |
| Chained token exchange          | Delegated tokens explicitly rejected as subject tokens for exchange |
| MCP token replay                | 5-minute TTL on MCP-injected tokens; per-call generation with unique JTI |
| Approval webhook spoofing       | Constant-time secret verification with pre-hashed SHA-256 comparison |
| Approval replay attack          | Atomic `findOneAndUpdate` with `status: "pending"` filter; no-op after first decision |
| Cross-user approval             | Chat ID binding verification; web UI verifies ownership via `user_id` match |

---

## Incident Response

### If ENCRYPTION_KEY Is Compromised

1. Contain the compromise and revoke/replace exposed upstream credentials. Rewrapping cannot protect a database copy already held by an attacker.
2. Follow the ordered [key rotation procedure](#key-rotation), staging dual-key readers on every replica before activating new-key writes, or draining writers during reader preparation.
3. Retain required old-key access and use the explicit [encrypted-record migration procedure](#encrypted-record-migration). The runner provides durable progress and exact-source CAS for converted families; unfinished adapters and unresolved findings block complete readiness.
4. Retire old keys only after every required encrypted field and retained backup has an explicit disposition. Decrypt traffic counters alone are insufficient.
5. Audit access during the compromise window and verify the recovery on the actual deployment.

### If RSA Private Key Is Compromised

1. Contain signing access and prepare independent HMAC material before changing any source from which it was derived. Preserve the matching keyset and database evidence.
2. Generate a valid replacement pair and coordinate every verifier. For a compromise, deliberately retire the compromised public key instead of retaining normal overlap; any token signed by a retained verifier remains cryptographically valid subject to its live authority checks.
3. Replace the active pair, remove the compromised verifier from `JWT_RETAINED_PUBLIC_KEY_PATHS`, and restart the fleet. External relying parties may retain cached JWKS until their own refresh.
4. Revoke affected sessions/grants as appropriate and require reauthentication/reauthorization. Follow [key lifecycle preparation](security/KEY_LIFECYCLE.md) for normal overlap; an emergency retirement has different availability consequences.

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
| SEC-M3 | Partial plaintext zeroization | Some proxy buffers use Zeroizing; later ordinary copies remain and are tracked in #1933 |
| SEC-M5 | Provider error containment | Path-specific sanitization; proxy diagnostics use metadata only |
| CR-4/5/6 | N+1 query prevention                    | Batch `$in` queries for provider lookups    |
| CR-15  | Required provider enforcement              | 400 error for missing required provider tokens |
| DA-1   | Delegated token endpoint restriction       | Verified `AuthUser` policy plus fail-fast `reject_delegated_tokens` middleware |
| DA-2   | Chained exchange prevention                | `delegated == Some(true)` check in `exchange_token()` |
| DA-3   | Consent-gated delegation                   | `consent_service::check_consent()` before token issuance |
| DA-4   | MCP injection opt-in                       | `inject_delegation_token` defaults to `false` per service |
