# Phase 2: Envelope Encryption Plan

## Status: COMPLETED

## Problem Statement

Phase 1 (key versioning) solved key rotation, but the KEK still directly encrypts all data. This means:

1. **Single key touches all data**: The KEK is used in every encrypt/decrypt operation
2. **No per-record isolation**: Compromising the KEK exposes all records
3. **Full re-encryption on rotation**: Rotating the KEK requires re-encrypting all data (not just re-wrapping small DEK blobs)

## Phase 1 Architecture (Current)

```
KEK (from ENCRYPTION_KEY) ---> directly encrypts ---> data
```

Each encrypted field stores: `v1_header(2) || nonce(12) || ciphertext || tag(16)`

## Phase 2 Design: Envelope Encryption

### Key Hierarchy

```
KEK (Key Encryption Key, from ENCRYPTION_KEY, 32 bytes)
  |
  +-- wraps --> DEK #1 (random AES-256, per-record) --> encrypts credential #1
  +-- wraps --> DEK #2 (random AES-256, per-record) --> encrypts credential #2
  +-- wraps --> DEK #N (random AES-256, per-record) --> encrypts credential #N
```

### Design Principle: Self-Contained Envelope

The envelope format is fully self-contained in the existing `Vec<u8>` blob. This means:

- **Zero changes to MongoDB models** (fields stay `Vec<u8>`)
- **Zero changes to services** (they call `encryption_keys.encrypt()`/`decrypt()` unchanged)
- **Zero changes to handlers** (they pass `&state.encryption_keys` unchanged)
- **Zero changes to config** (no new env vars)

The entire change is internal to `crypto/aes.rs`.

### v2 Ciphertext Format

```
0x02 || kek_id(1) || wrapped_dek_len(2, BE) || wrapped_dek(60) || data_nonce(12) || data_ciphertext || data_tag(16)
```

Where:
- `0x02` = version byte for envelope encryption
- `kek_id(1)` = stable key ID of the KEK that wrapped the DEK (same derivation as v1)
- `wrapped_dek_len(2)` = length of wrapped DEK blob, big-endian u16
- `wrapped_dek(60)` = DEK encrypted with KEK: `dek_nonce(12) || encrypted_dek(32) || dek_tag(16)`
- `data_nonce(12)` = nonce for AES-256-GCM data encryption
- `data_ciphertext || data_tag(16)` = actual credential encrypted with DEK

Total overhead vs v1: +62 bytes per record (very acceptable).

### Encrypt Flow (v2)

1. Generate random 32-byte DEK
2. Encrypt plaintext with DEK using AES-256-GCM (random nonce)
3. Wrap DEK with KEK using AES-256-GCM (separate random nonce)
4. Assemble v2 envelope: header + wrapped DEK + encrypted data
5. Zeroize DEK from memory

### Decrypt Flow (v2)

1. Parse v2 header, extract kek_id
2. Select KEK (current or previous) based on kek_id
3. Unwrap DEK from wrapped_dek blob using selected KEK
4. Decrypt data using unwrapped DEK
5. Zeroize DEK from memory
6. Return plaintext

### Decrypt Fallback Chain (Updated)

```
1. If looks_like_v2: try v2 with current KEK, then previous KEK
2. If looks_like_v1: try v1 with current key, then previous key
3. Try v0 with current key
4. Try v0 with previous key
5. Return error
```

### Re-wrap Optimization for KEK Rotation

```rust
/// Re-wrap a v2 ciphertext's DEK from the previous KEK to the current KEK.
/// Only the wrapped_dek portion changes; encrypted data is untouched.
pub fn rewrap(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError>;
```

During KEK rotation, a background job can call `rewrap()` on each record. This:
- Unwraps the DEK with the old KEK
- Re-wraps the DEK with the new KEK
- Replaces only the header + wrapped_dek portion
- Does NOT decrypt or re-encrypt the actual data

### Backward Compatibility

- v0 (legacy) ciphertexts: still decryptable via fallback chain
- v1 (Phase 1) ciphertexts: still decryptable via fallback chain
- v2 (Phase 2) ciphertexts: new default for all encryptions
- Rollback to Phase 1 code: v2 ciphertexts would fail (only data written after Phase 2 deploy)
- Mitigation: deploy Phase 2 first, let it run, only then do any key rotation

### Rollback Procedure

If Phase 2 code needs to be reverted:
1. Any v0/v1 ciphertexts still work with Phase 1 code
2. v2 ciphertexts (written after Phase 2 deploy) would NOT work with Phase 1 code
3. A migration script can be provided to re-encrypt v2 data back to v1 format before rollback

### Testing Strategy

- Unit tests: v2 roundtrip, v2 + v1 + v0 cross-version decrypt
- DEK uniqueness: same plaintext encrypted twice produces different DEKs
- KEK rotation with v2: encrypt with KEK-A, rotate to KEK-B, decrypt still works
- Rewrap: encrypt with KEK-A, rewrap to KEK-B, decrypt with KEK-B only
- Rollback: encrypt with KEK-B (v2), rollback to KEK-A with previous=B
- Tamper detection: modify wrapped_dek, modify data, modify header
- Empty/large plaintext with v2
- Decrypt stats: v2_current, v2_previous counters
- Memory safety: DEK zeroized after use

### Files to Change

1. `backend/src/crypto/aes.rs` - Core envelope encryption implementation
2. `docs/ENCRYPTION_ARCHITECTURE.md` - Update architecture diagrams
3. `docs/ENCRYPTION_KEY_VERSIONING_PLAN.md` - Update status and notes
4. `docs/SECURITY.md` - Update encryption format docs

---

## Completion Notes

Phase 2 has been implemented entirely within `backend/src/crypto/aes.rs`. The public API (`encrypt()`, `decrypt()`) is unchanged; only the internal ciphertext format changed from v1 (direct KEK encryption) to v2 (envelope encryption with per-record DEKs).

### What was implemented

- v2 envelope encryption: each `encrypt()` call generates a random 32-byte DEK, encrypts data with the DEK, wraps the DEK with the KEK, and assembles a self-contained v2 envelope
- v2 ciphertext format: `0x02 || kek_id(1) || wrapped_dek_len(2 BE) || wrapped_dek(60) || data_nonce(12) || data_ciphertext || data_tag(16)`
- `rewrap()` method for efficient KEK rotation: unwraps DEK with old KEK, re-wraps with new KEK, leaves encrypted data untouched
- DEK memory safety: `Zeroizing<[u8; 32]>` ensures DEK is wiped from memory after use
- Decrypt fallback chain updated: v2 current -> v2 previous -> v1 current -> v1 previous -> draft v1 -> v0 current -> v0 previous -> error
- `DecryptCounters` extended with `v2_current` and `v2_previous` fields for health monitoring
- Full backward compatibility: v0 and v1 ciphertexts still decrypt via fallback chain
- Comprehensive unit tests: v2 roundtrip, cross-version decrypt, DEK uniqueness, KEK rotation with v2, rewrap, tamper detection, empty/large plaintext, decrypt stats, memory safety

### Design decisions

- **Self-contained envelope**: the v2 format fits entirely within the existing `Vec<u8>` blob, requiring zero changes to MongoDB models, services, handlers, or config
- **Per-record DEK isolation**: compromising one record's DEK does not expose other records
- **Rewrap optimization**: KEK rotation only re-wraps the small DEK blob (~60 bytes) per record, not the full data ciphertext -- enabling rotation of 1M+ records in seconds
- **Separate nonces**: DEK wrapping and data encryption each use independent random 12-byte nonces
- **Constants**: `VERSION_V2 = 0x02`, `V2_HEADER_SIZE = 4`, `WRAPPED_DEK_SIZE = 60`, `V2_MIN_SIZE = 92`
