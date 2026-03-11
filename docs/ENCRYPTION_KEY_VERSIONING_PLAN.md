# Encryption Key Versioning -- Phase 1 Plan

## Status: COMPLETED

## Problem Statement

NyxID currently uses a single static AES-256-GCM key (`ENCRYPTION_KEY` env var) to encrypt all sensitive credentials at rest. This means:

1. **No key rotation**: Changing `ENCRYPTION_KEY` breaks ALL existing encrypted data
2. **No version metadata**: Ciphertexts carry no indication of which key encrypted them
3. **No rollback path**: If a key rotation goes wrong, there is no fallback

## Current Architecture

```
ENCRYPTION_KEY (env var, 64 hex chars = 32 bytes)
        |
        v
   AES-256-GCM (random 12-byte nonce per operation)
        |
        v
   Storage format: nonce(12) || ciphertext || tag(16)
```

### What Is Encrypted (all fields across MongoDB)

| Data                              | Collection                 | Fields                                                     |
|-----------------------------------|----------------------------|------------------------------------------------------------|
| Service master credentials        | `downstream_services`      | `credential_encrypted`                                     |
| Per-user service credentials      | `user_service_connections` | `credential_encrypted`                                     |
| MFA TOTP secrets                  | `mfa_factors`              | `secret_encrypted`                                         |
| Provider OAuth client credentials | `provider_configs`         | `client_id_encrypted`, `client_secret_encrypted`           |
| User provider OAuth tokens        | `user_provider_tokens`     | `access_token_encrypted`, `refresh_token_encrypted`        |
| User provider API keys            | `user_provider_tokens`     | `api_key_encrypted`                                        |
| User provider credentials         | `user_provider_credentials`| `client_id_encrypted`, `client_secret_encrypted`           |
| OAuth state secrets               | `oauth_states`             | `code_verifier_encrypted`                                  |

### Files That Call encrypt/decrypt

**Services (business logic):**
- `services/provider_service.rs` -- provider OIDC credential encryption
- `services/connection_service.rs` -- user service connection credentials
- `services/user_credentials_service.rs` -- per-user provider credentials
- `services/user_token_service.rs` -- OAuth tokens, API keys, device codes
- `services/oauth_flow.rs` -- OAuth token refresh re-encryption
- `services/mfa_service.rs` -- TOTP secret encryption
- `services/proxy_service.rs` -- credential decryption for proxying
- `services/mcp_service.rs` -- MCP proxy credential resolution
- `services/delegation_service.rs` -- delegated access token decryption

**Handlers (HTTP layer -- parse hex key from config):**
- `handlers/providers.rs`
- `handlers/connections.rs`
- `handlers/services.rs`
- `handlers/user_tokens.rs`
- `handlers/user_credentials.rs`
- `handlers/mfa.rs`
- `handlers/proxy.rs`
- `handlers/llm_gateway.rs`
- `handlers/mcp_transport.rs`
- `handlers/admin_sa_providers.rs`
- `handlers/admin_sa_connections.rs`
- `handlers/auth.rs`

## Phase 1 Design: Key Versioning with Rotation Support

### Goals

1. Add a version prefix to all NEW ciphertexts
2. Transparently decrypt both old (unversioned) and new (versioned) ciphertexts
3. Support `ENCRYPTION_KEY` (current) + `ENCRYPTION_KEY_PREVIOUS` (old) env vars
4. Zero downtime -- no data migration required
5. Full rollback capability -- can revert code and data still works

### New Ciphertext Format

```
Version 0 (legacy, implicit): nonce(12) || ciphertext || tag(16)
Version 1 (new):              0x01 || key_id(1) || nonce(12) || ciphertext || tag(16)
```

- `0x01` = version byte
- `key_id`: stable 1-byte identifier derived from `SHA-256(key)[0]`
- Draft compatibility: the earlier uncommitted Phase 1 header (`0x00` / `0x01`) is still accepted during rollout

**Detection strategy**:
- Treat any ciphertext with length >= 30 and leading byte `0x01` as versioned
- Match `key_id` against the configured current or previous key
- If no configured key matches, fall back to v0 handling before returning an error

### New Abstraction: `EncryptionKeys`

```rust
pub struct EncryptionKeys {
    current: Vec<u8>,        // 32 bytes, from ENCRYPTION_KEY
    previous: Option<Vec<u8>>, // 32 bytes, from ENCRYPTION_KEY_PREVIOUS
}

impl EncryptionKeys {
    /// Encrypt with current key, v1 format
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError>;

    /// Decrypt: try v1 current, then v1 previous, then v0 current, then v0 previous
    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError>;
}
```

### Config Changes

```bash
# Required (existing)
ENCRYPTION_KEY=<64 hex chars>

# Optional (new -- set during rotation)
ENCRYPTION_KEY_PREVIOUS=<64 hex chars of the OLD key>
```

### Rotation Procedure

1. Generate new key: `openssl rand -hex 32`
2. Set `ENCRYPTION_KEY_PREVIOUS` to the current `ENCRYPTION_KEY` value
3. Set `ENCRYPTION_KEY` to the new key
4. Restart the service
5. All new encryptions use the new key (v1 format)
6. All old data decrypts via fallback chain (v0 with old key, or v1 with old key)
7. Run a background re-encryption job to migrate all data that still depends on the old key, including legacy v0 ciphertexts and any v1 ciphertexts written before the rotation
8. Use `/health` decrypt counters to confirm traffic is no longer using `v1_previous`, `v0_current`, or `v0_previous`
9. After all old-key data is re-encrypted, remove `ENCRYPTION_KEY_PREVIOUS`

### Rollback Procedure

If something goes wrong after rotation:
1. Set `ENCRYPTION_KEY` back to the old key
2. Set `ENCRYPTION_KEY_PREVIOUS` to the new key (so v1 data encrypted with new key still decrypts)
3. Restart -- everything works

If reverting the code entirely (back to pre-versioning):
- v0 (legacy) ciphertexts still work with the original key
- v1 ciphertexts would fail to decrypt -- but only data written AFTER the upgrade
- Mitigation: deploy Phase 1 code first, let it run, only then rotate keys

### Migration Strategy (code changes)

1. **`crypto/aes.rs`**: Add `EncryptionKeys` struct with versioned encrypt/decrypt
2. **`config.rs`**: Add `encryption_key_previous: Option<String>` field
3. **All handlers**: Replace `aes::parse_hex_key(&state.config.encryption_key)` with `EncryptionKeys::from_config(&state.config)`
4. **All services**: Change `encryption_key: &[u8]` params to `encryption_keys: &EncryptionKeys`
5. **Existing `encrypt`/`decrypt` functions**: Keep as-is for backward compatibility, add new versioned functions

### Testing Strategy

- Unit tests: roundtrip v0 and v1, cross-version decrypt, wrong key rejection
- Integration: encrypt with v0, upgrade to v1, verify old data decrypts
- Rotation: encrypt with key A, rotate to key B, verify old data decrypts
- Rollback: encrypt with key B (v1), rollback to key A, verify via previous key fallback

## Completion Notes

Phase 1 has been implemented with two important clarifications:

- The v1 `key_id` is now a stable key fingerprint byte instead of a logical "current/previous" flag
- Phase 1 supports exactly one previous key at a time, so repeated rotations require re-encryption of all old-key data before the next rotation

### What was implemented

- `EncryptionKeys` struct in `crypto/aes.rs` with stable key IDs, versioned encrypt (always v1), decrypt counters, and decrypt fallback support for current/previous/draft v1 headers plus legacy v0
- `encryption_key_previous: Option<String>` field in `AppConfig` with validation (hex format, length, all-zeros rejection)
- `EncryptionKeys` wired into `AppState` in `main.rs`, constructed at startup
- All 9 service files migrated from `encryption_key: &[u8]` to `encryption_keys: &EncryptionKeys`
- All 12 handler files migrated from `aes::parse_hex_key(&state.config.encryption_key)` to `&state.encryption_keys`
- All test `AppConfig` constructors updated with `encryption_key_previous: None`
- Comprehensive unit tests: v0 roundtrip, v1 roundtrip, draft-header compatibility, cross-version decrypt, key rotation, rollback, second-rotation limitation, tamper detection, empty/large plaintext, Debug redaction, decrypt stats
- Manual `Debug` impl on `EncryptionKeys` that redacts key material

### Design decisions

- Legacy `encrypt()`/`decrypt()` free functions retained for backward compatibility (used in tests)
- `parse_hex_key()` kept as a public function (may still be referenced externally)
- Error messages from the fallback chain are generic ("no key could decrypt the data") to avoid leaking key/version information

---

## Future Phases (out of scope for Phase 1)

- **Phase 2**: Envelope encryption (per-record DEKs wrapped by KEK)
- **Phase 3**: `KeyProvider` trait (pluggable KMS backends)
- **Phase 4**: AWS KMS / GCP Cloud KMS integration
- **Phase 5**: Background re-encryption job
- **Phase 6**: Per-tenant key isolation / BYOK
