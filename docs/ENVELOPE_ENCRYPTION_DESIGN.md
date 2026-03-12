# Envelope Encryption Technical Design (Phase 2)

## Status: APPROVED FOR IMPLEMENTATION

This document provides the detailed technical specification for implementing envelope
encryption in `backend/src/crypto/aes.rs`. The change is entirely internal to this
module -- the public API of `EncryptionKeys` (`encrypt`, `decrypt`, `decrypt_stats`,
`has_previous`, `from_config`) does NOT change its signatures.

---

## 1. v2 Ciphertext Format

### Byte Layout

```
Offset  Size     Field                Description
------  ------   -------------------  ------------------------------------------------
0       1        version              0x02 (VERSION_V2)
1       1        kek_id               SHA-256(kek_material)[0] -- identifies which KEK wrapped the DEK
2       2        wrapped_dek_len      Big-endian u16, length of wrapped_dek blob (currently always 60)
4       12       dek_nonce            Random nonce for AES-256-GCM DEK wrapping
16      48       encrypted_dek+tag    AES-256-GCM(KEK, dek_nonce, DEK) = encrypted_dek(32) || dek_tag(16)
64      12       data_nonce           Random nonce for AES-256-GCM data encryption
76      N        data_ciphertext      AES-256-GCM(DEK, data_nonce, plaintext) ciphertext
76+N    16       data_tag             AES-256-GCM authentication tag for data
```

### Wrapped DEK Breakdown (60 bytes)

```
Offset  Size  Field           Description
------  ----  --------------- ----------------------------------------
0       12    dek_nonce       Random 96-bit nonce for DEK wrapping
12      32    encrypted_dek   AES-256-GCM ciphertext of the 32-byte DEK
44      16    dek_tag         Authentication tag for DEK wrapping
```

Note: `encrypted_dek + dek_tag` (48 bytes) is the raw output of `Aes256Gcm::encrypt()`.
The aes-gcm crate concatenates `ciphertext || tag` in its output, so
`Aes256Gcm::encrypt(dek_nonce, dek_plaintext)` returns exactly 48 bytes for a 32-byte
input.

### Constants

```rust
const VERSION_V2: u8 = 0x02;

/// v2 header: version(1) + kek_id(1) + wrapped_dek_len(2) = 4 bytes
const V2_HEADER_SIZE: usize = 4;

/// Wrapped DEK: dek_nonce(12) + encrypted_dek(32) + dek_tag(16) = 60 bytes
const WRAPPED_DEK_SIZE: usize = NONCE_SIZE + 32 + 16; // = 60

/// Minimum v2 ciphertext: header(4) + wrapped_dek(60) + data_nonce(12) + data_tag(16) = 92 bytes
/// (this represents an empty plaintext)
const V2_MIN_SIZE: usize = V2_HEADER_SIZE + WRAPPED_DEK_SIZE + NONCE_SIZE + 16; // = 92
```

### Size Overhead Analysis

| Format | Fixed Overhead | Total Size | Example: 100B plaintext |
|--------|---------------|------------|------------------------|
| v0     | 28 bytes      | 28 + N     | 128 bytes              |
| v1     | 30 bytes      | 30 + N     | 130 bytes              |
| v2     | 92 bytes      | 92 + N     | 192 bytes              |

v2 adds **+62 bytes** over v1 per record. For typical credentials:
- OAuth access token (~100 bytes): 192 bytes total, +48% vs v1
- OAuth refresh token (~60 bytes): 152 bytes total, +69% vs v1
- API key (~40 bytes): 132 bytes total, +89% vs v1
- MFA secret (~32 bytes): 124 bytes total, +100% vs v1
- Large credential (~1000 bytes): 1092 bytes total, +6% vs v1

The per-record overhead is constant at 62 bytes regardless of plaintext size. For a
database with 1M encrypted fields, total additional storage is ~59 MB -- negligible
for MongoDB.

---

## 2. Encrypt Flow (v2)

The `encrypt()` method changes from v1 to v2. All new encryptions produce v2 envelopes.

### Pseudocode

```rust
pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
    // 1. Generate random 32-byte Data Encryption Key (DEK)
    let mut dek = Zeroizing::new([0u8; 32]);
    rand::thread_rng().fill_bytes(dek.as_mut());

    // 2. Encrypt plaintext with DEK using AES-256-GCM
    let data_cipher = Aes256Gcm::new_from_slice(dek.as_ref())?;
    let mut data_nonce_bytes = [0u8; NONCE_SIZE];
    rand::thread_rng().fill_bytes(&mut data_nonce_bytes);
    let data_nonce = Nonce::from_slice(&data_nonce_bytes);
    let encrypted_data = data_cipher.encrypt(data_nonce, plaintext)?;
    // encrypted_data = data_ciphertext || data_tag
    // len(encrypted_data) = len(plaintext) + 16

    // 3. Wrap DEK with current KEK using AES-256-GCM (separate nonce)
    let kek_cipher = Aes256Gcm::new_from_slice(self.current.as_ref())?;
    let mut dek_nonce_bytes = [0u8; NONCE_SIZE];
    rand::thread_rng().fill_bytes(&mut dek_nonce_bytes);
    let dek_nonce = Nonce::from_slice(&dek_nonce_bytes);
    let wrapped_dek_payload = kek_cipher.encrypt(dek_nonce, dek.as_ref().as_ref())?;
    // wrapped_dek_payload = encrypted_dek(32) || dek_tag(16) = 48 bytes

    // 4. Assemble v2 envelope
    let wrapped_dek_len = (NONCE_SIZE + wrapped_dek_payload.len()) as u16; // = 60
    let total_size = V2_HEADER_SIZE
        + wrapped_dek_len as usize
        + NONCE_SIZE
        + encrypted_data.len();

    let mut result = Vec::with_capacity(total_size);
    result.push(VERSION_V2);                              // byte 0
    result.push(self.current_id);                         // byte 1
    result.extend_from_slice(&wrapped_dek_len.to_be_bytes()); // bytes 2-3
    result.extend_from_slice(&dek_nonce_bytes);           // bytes 4-15
    result.extend_from_slice(&wrapped_dek_payload);       // bytes 16-63
    result.extend_from_slice(&data_nonce_bytes);          // bytes 64-75
    result.extend_from_slice(&encrypted_data);            // bytes 76..end

    // 5. DEK is automatically zeroized when `dek` drops (Zeroizing)
    Ok(result)
}
```

### Security Properties

- **DEK uniqueness**: Each `encrypt()` call generates a fresh random DEK. Even encrypting
  the same plaintext twice produces completely different ciphertext (different DEK,
  different nonces).
- **Nonce separation**: The DEK-wrapping nonce and the data-encryption nonce are
  independently random. They serve different AES-256-GCM instances with different keys,
  so there is zero nonce-reuse risk.
- **DEK zeroization**: The DEK is held in `Zeroizing<[u8; 32]>`, which overwrites memory
  with zeroes on drop. The DEK never escapes the `encrypt()` function scope.

---

## 3. Decrypt Flow (v2)

### Version Detection

```rust
fn looks_like_v2(data: &[u8]) -> bool {
    data.len() >= V2_MIN_SIZE && data[0] == VERSION_V2
}
```

The `V2_MIN_SIZE` of 92 bytes is large enough that accidental false positives from v0
ciphertext (where the first nonce byte happens to be 0x02) are extremely unlikely to
also satisfy the length check AND produce a valid AEAD unwrap. The fallback chain
handles these edge cases gracefully (see Section 4).

### Pseudocode: decrypt_v2 (private helper)

```rust
/// Attempt to decrypt a v2 envelope. Returns Ok(plaintext) or Err.
fn decrypt_v2(&self, ciphertext: &[u8]) -> Result<Vec<u8>, DecryptV2Result> {
    // 1. Parse header
    let kek_id = ciphertext[1];
    let wrapped_dek_len = u16::from_be_bytes([ciphertext[2], ciphertext[3]]) as usize;

    // 2. Bounds check: wrapped_dek_len must fit within the ciphertext
    let wrapped_dek_end = V2_HEADER_SIZE + wrapped_dek_len;
    let remaining_after_dek = ciphertext.len().checked_sub(wrapped_dek_end);
    if remaining_after_dek.is_none() || remaining_after_dek.unwrap() < NONCE_SIZE + 16 {
        return Err(DecryptV2Result::FormatError);
    }

    // 3. Select KEK by kek_id
    let (kek, is_previous) = if kek_id == self.current_id {
        (self.current.as_ref(), false)
    } else if self.previous_id == Some(kek_id) {
        match self.previous.as_ref() {
            Some(prev) => (prev.as_ref(), true),
            None => return Err(DecryptV2Result::UnknownKeyId(kek_id)),
        }
    } else {
        return Err(DecryptV2Result::UnknownKeyId(kek_id));
    };

    // 4. Extract wrapped DEK
    let wrapped_dek = &ciphertext[V2_HEADER_SIZE..wrapped_dek_end];
    let dek_nonce_bytes = &wrapped_dek[..NONCE_SIZE];
    let encrypted_dek_with_tag = &wrapped_dek[NONCE_SIZE..];

    // 5. Unwrap DEK
    let kek_cipher = Aes256Gcm::new_from_slice(kek)?;
    let dek_nonce = Nonce::from_slice(dek_nonce_bytes);
    let dek_raw = kek_cipher.decrypt(dek_nonce, encrypted_dek_with_tag)
        .map_err(|_| DecryptV2Result::DekUnwrapFailed)?;

    // Validate DEK length (must be exactly 32 bytes)
    if dek_raw.len() != 32 {
        return Err(DecryptV2Result::DekUnwrapFailed);
    }

    let mut dek_array = [0u8; 32];
    dek_array.copy_from_slice(&dek_raw);
    let dek = Zeroizing::new(dek_array);

    // 6. Decrypt data with DEK
    let data_nonce_start = wrapped_dek_end;
    let data_nonce_bytes = &ciphertext[data_nonce_start..data_nonce_start + NONCE_SIZE];
    let encrypted_data = &ciphertext[data_nonce_start + NONCE_SIZE..];

    let data_cipher = Aes256Gcm::new_from_slice(dek.as_ref())?;
    let data_nonce = Nonce::from_slice(data_nonce_bytes);
    let plaintext = data_cipher.decrypt(data_nonce, encrypted_data)
        .map_err(|_| DecryptV2Result::DataDecryptFailed)?;

    // 7. Update counters
    if is_previous {
        // bump v2_previous counter + log once
    } else {
        // bump v2_current counter
    }

    // 8. DEK zeroized on drop
    Ok(plaintext)
}
```

### Internal Result Type for decrypt_v2

To communicate failure reasons back to the decrypt fallback chain without exposing
them in the public API:

```rust
enum DecryptV2Result {
    FormatError,                // Not valid v2 structure -> fall through
    UnknownKeyId(u8),           // kek_id doesn't match any configured key -> record + fall through
    DekUnwrapFailed,            // KEK matched but DEK unwrap failed -> fall through (might be v0 misidentified)
    DataDecryptFailed,          // DEK unwrapped but data decrypt failed -> fall through (might be v0 misidentified)
    CipherError(String),        // Cipher initialization error -> propagate as AppError
}
```

Design decision: ALL v2 decrypt failures fall through to v1/v0. This handles the edge
case where a v0 ciphertext happens to start with byte 0x02. The v0 AEAD will either
succeed (correct) or fail (genuine error), and no valid v2 envelope will be silently
dropped.

---

## 4. Updated Decrypt Fallback Chain

### Flow

```
1. If looks_like_v2(ciphertext):
   a. Parse kek_id from byte 1
   b. If kek_id == current_id:
      - Unwrap DEK with current KEK
      - Decrypt data with DEK
      - If success: bump v2_current, return plaintext
   c. If kek_id == previous_id:
      - Unwrap DEK with previous KEK
      - Decrypt data with DEK
      - If success: bump v2_previous, log once, return plaintext
   d. If kek_id unknown:
      - Record unknown_key_id for later reporting
      - Fall through

2. If looks_like_v1(ciphertext):
   a. (Existing v1 logic -- unchanged)
   b. Try current key, then previous key, then draft key compat
   c. If success: bump appropriate v1 counter, return plaintext

3. Try v0 with current key:
   - If success: bump v0_current, log once, return plaintext

4. Try v0 with previous key:
   - If success: bump v0_previous, log once, return plaintext

5. If unknown kek_id was recorded:
   - Bump unknown_key_id_failures, log once

6. Bump decrypt_failures
7. Return Err("no key could decrypt the data")
```

### Pseudocode

```rust
pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
    let mut unknown_key_id = None;

    // --- v2 envelope ---
    if looks_like_v2(ciphertext) {
        match self.try_decrypt_v2(ciphertext) {
            Ok(plaintext) => return Ok(plaintext),
            Err(DecryptV2Result::UnknownKeyId(kid)) => {
                unknown_key_id = Some(kid);
            }
            Err(DecryptV2Result::CipherError(msg)) => {
                return Err(AppError::Internal(msg));
            }
            Err(_) => {
                // FormatError, DekUnwrapFailed, DataDecryptFailed -> fall through
            }
        }
    }

    // --- v1 envelope (existing logic, unchanged) ---
    if looks_like_v1(ciphertext) {
        let key_id = ciphertext[1];
        let payload = &ciphertext[V1_HEADER_SIZE..];

        // ... (existing v1 logic exactly as-is) ...
    }

    // --- v0 with current key ---
    if let Ok(plain) = decrypt_raw(ciphertext, self.current.as_ref()) {
        self.counters.v0_current.fetch_add(1, Ordering::Relaxed);
        self.log_once(/* ... */);
        return Ok(plain);
    }

    // --- v0 with previous key ---
    if let Some(ref prev) = self.previous {
        if let Ok(plain) = decrypt_raw(ciphertext, prev.as_ref()) {
            self.counters.v0_previous.fetch_add(1, Ordering::Relaxed);
            self.log_once(/* ... */);
            return Ok(plain);
        }
    }

    // --- unknown key id reporting ---
    if let Some(key_id) = unknown_key_id {
        self.counters.unknown_key_id_failures.fetch_add(1, Ordering::Relaxed);
        self.log_once(/* ... */);
    }

    self.counters.decrypt_failures.fetch_add(1, Ordering::Relaxed);
    Err(AppError::Internal("AES decryption failed: no key could decrypt the data".to_string()))
}
```

### Important: v1 Logic Unchanged

The existing v1 decrypt path (including draft key ID compatibility for `0x00`/`0x01`)
remains exactly as implemented. It is NOT modified. The only change to `decrypt()` is
prepending the v2 attempt at the top.

---

## 5. Rewrap Method

### Purpose

During KEK rotation, a background job calls `rewrap()` on each v2-encrypted record.
This unwraps the DEK with the old KEK and re-wraps it with the new KEK, without
touching the encrypted data. This is dramatically faster than full re-encryption
because only the 60-byte wrapped DEK changes.

### Pseudocode

```rust
/// Re-wrap a v2 ciphertext's DEK from the previous KEK to the current KEK.
///
/// - If the ciphertext is already wrapped with the current KEK, returns it unchanged
///   (idempotent).
/// - If the ciphertext is not v2, returns an error.
/// - Only the header + wrapped_dek portion changes; encrypted data is untouched.
pub fn rewrap(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
    // 1. Validate v2 format
    if !looks_like_v2(ciphertext) {
        return Err(AppError::Internal(
            "rewrap: ciphertext is not v2 format".to_string()
        ));
    }

    // 2. Parse header
    let kek_id = ciphertext[1];
    let wrapped_dek_len = u16::from_be_bytes([ciphertext[2], ciphertext[3]]) as usize;
    let wrapped_dek_end = V2_HEADER_SIZE + wrapped_dek_len;

    // 3. Bounds check
    if ciphertext.len() < wrapped_dek_end + NONCE_SIZE + 16 {
        return Err(AppError::Internal(
            "rewrap: ciphertext too short for declared wrapped_dek_len".to_string()
        ));
    }

    // 4. If already wrapped with current KEK, return unchanged (idempotent)
    if kek_id == self.current_id {
        return Ok(ciphertext.to_vec());
    }

    // 5. Require previous key that matches kek_id
    let prev_kek = match (self.previous_id, &self.previous) {
        (Some(pid), Some(prev)) if pid == kek_id => prev.as_ref(),
        _ => {
            return Err(AppError::Internal(format!(
                "rewrap: kek_id 0x{kek_id:02x} does not match current or previous key"
            )));
        }
    };

    // 6. Extract and unwrap DEK with previous KEK
    let wrapped_dek = &ciphertext[V2_HEADER_SIZE..wrapped_dek_end];
    let dek_nonce_bytes = &wrapped_dek[..NONCE_SIZE];
    let encrypted_dek_with_tag = &wrapped_dek[NONCE_SIZE..];

    let prev_cipher = Aes256Gcm::new_from_slice(prev_kek)
        .map_err(|e| AppError::Internal(format!("Failed to create AES cipher: {e}")))?;
    let dek_raw = prev_cipher
        .decrypt(Nonce::from_slice(dek_nonce_bytes), encrypted_dek_with_tag)
        .map_err(|e| AppError::Internal(format!("rewrap: DEK unwrap failed: {e}")))?;

    let mut dek_array = [0u8; 32];
    dek_array.copy_from_slice(&dek_raw);
    let dek = Zeroizing::new(dek_array);

    // 7. Re-wrap DEK with current KEK (fresh nonce)
    let cur_cipher = Aes256Gcm::new_from_slice(self.current.as_ref())
        .map_err(|e| AppError::Internal(format!("Failed to create AES cipher: {e}")))?;
    let mut new_dek_nonce_bytes = [0u8; NONCE_SIZE];
    rand::thread_rng().fill_bytes(&mut new_dek_nonce_bytes);
    let new_wrapped_dek_payload = cur_cipher
        .encrypt(
            Nonce::from_slice(&new_dek_nonce_bytes),
            dek.as_ref().as_ref(),
        )
        .map_err(|e| AppError::Internal(format!("rewrap: DEK re-wrap failed: {e}")))?;

    // 8. Reassemble envelope: new header + new wrapped DEK + original data portion
    let new_wrapped_dek_len = (NONCE_SIZE + new_wrapped_dek_payload.len()) as u16;
    let data_portion = &ciphertext[wrapped_dek_end..]; // data_nonce + encrypted_data

    let mut result = Vec::with_capacity(
        V2_HEADER_SIZE + new_wrapped_dek_len as usize + data_portion.len()
    );
    result.push(VERSION_V2);
    result.push(self.current_id);
    result.extend_from_slice(&new_wrapped_dek_len.to_be_bytes());
    result.extend_from_slice(&new_dek_nonce_bytes);
    result.extend_from_slice(&new_wrapped_dek_payload);
    result.extend_from_slice(data_portion);

    // 9. DEK zeroized on drop
    Ok(result)
}
```

### Rewrap Properties

- **Idempotent**: If the ciphertext is already wrapped with the current KEK, the
  original ciphertext is returned as-is (cloned).
- **Data untouched**: Only the first 64 bytes (header + wrapped DEK) change. The data
  nonce, ciphertext, and tag are byte-for-byte identical.
- **v2 only**: `rewrap()` only operates on v2 envelopes. v0 and v1 data must be fully
  re-encrypted via `decrypt()` + `encrypt()`.
- **Requires previous key**: The previous KEK must be configured to rewrap. Without it,
  there is no key to unwrap the old DEK.

### Rewrap Performance

Per-record cost:
- 1 AES-256-GCM decrypt (unwrap DEK, 32 bytes) + 1 AES-256-GCM encrypt (re-wrap DEK, 32 bytes)
- No data decryption/re-encryption
- Expected: ~1 microsecond per record on modern hardware

For 1M records: ~1 second total (vs minutes/hours for full re-encryption).

---

## 6. Updated DecryptCounters

### Struct Changes

```rust
#[derive(Default)]
struct DecryptCounters {
    // v2 counters (new)
    v2_current: AtomicU64,
    v2_previous: AtomicU64,

    // v1 counters (unchanged)
    v1_current: AtomicU64,
    v1_previous: AtomicU64,

    // v0 counters (unchanged)
    v0_current: AtomicU64,
    v0_previous: AtomicU64,

    // Error counters (unchanged)
    unknown_key_id_failures: AtomicU64,
    decrypt_failures: AtomicU64,

    // Log-once flags (v2 additions)
    logged_v2_previous: AtomicBool,

    // Log-once flags (existing, unchanged)
    logged_v1_previous: AtomicBool,
    logged_v0_current: AtomicBool,
    logged_v0_previous: AtomicBool,
    logged_unknown_key_id: AtomicBool,
}
```

### Updated EncryptionDecryptStats

```rust
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct EncryptionDecryptStats {
    pub v2_current: u64,       // new
    pub v2_previous: u64,      // new
    pub v1_current: u64,
    pub v1_previous: u64,
    pub v0_current: u64,
    pub v0_previous: u64,
    pub unknown_key_id_failures: u64,
    pub decrypt_failures: u64,
}
```

### Monitoring Implications

During rotation, operators can watch `decrypt_stats()` to track migration progress:

| Counter         | Meaning                                | Action Required |
|-----------------|----------------------------------------|-----------------|
| v2_current > 0  | Normal v2 decrypts with current KEK    | None            |
| v2_previous > 0 | v2 decrypts needing old KEK            | Run rewrap job  |
| v1_current > 0  | v1 data still in use                   | Re-encrypt      |
| v1_previous > 0 | v1 data on old key                     | Re-encrypt      |
| v0_current > 0  | Legacy v0 data still in use            | Re-encrypt      |
| v0_previous > 0 | Legacy v0 data on old key              | Re-encrypt      |

Target state after full migration: only `v2_current > 0`, all other counters at 0.

---

## 7. Backward Compatibility Analysis

### Forward Compatibility (new code reads old data)

| Data Format | Encrypted By   | Decryptable By Phase 2 Code? | Mechanism        |
|-------------|----------------|------------------------------|------------------|
| v0          | Phase 0 code   | Yes                          | v0 fallback      |
| v0          | Phase 0 + prev | Yes (if prev key configured) | v0 prev fallback |
| v1 current  | Phase 1 code   | Yes                          | v1 fallback      |
| v1 previous | Phase 1 + prev | Yes (if prev key configured) | v1 prev fallback |
| v1 draft    | Draft Phase 1  | Yes                          | Draft compat     |

All existing data remains readable. No migration is required at deploy time.

### Backward Compatibility (old code reads new data)

| Data Format | Encrypted By   | Decryptable By Phase 1 Code? | Notes              |
|-------------|----------------|------------------------------|--------------------|
| v2          | Phase 2 code   | **NO**                       | v2 format unknown  |

Phase 1 code cannot read v2 data. This is acceptable because:

1. Only data written AFTER Phase 2 deployment uses v2 format
2. v0 and v1 data (written before Phase 2) remains readable by Phase 1 code
3. Rollback procedure (Section 7.1) handles this

### 7.1 Rollback Procedure

If Phase 2 code must be reverted to Phase 1:

1. **Immediate impact**: v2 ciphertexts (written after Phase 2 deploy) become unreadable
2. **Unaffected**: All v0 and v1 ciphertexts continue to work
3. **Mitigation**: Before rollback, run a migration script that:
   - Reads each v2-encrypted field with Phase 2 code
   - Re-encrypts it as v1 using Phase 1 format
   - Writes back to MongoDB
4. **Risk window**: The window is only the time between Phase 2 deploy and rollback
5. **Recommendation**: Deploy Phase 2, let it stabilize for 24-48 hours, then proceed
   with any key rotation. This limits the v2 data set if rollback is needed.

### Version Byte Collision Analysis

Could a v0 ciphertext accidentally trigger v2 detection?

- v0 starts with a random nonce byte. P(byte == 0x02) = 1/256
- v2 requires len >= 92 bytes. Most v0 ciphertexts exceed this for payloads > 64 bytes
- Even if both match, `looks_like_v2` returns true, but v2 decryption will fail
  (wrapped DEK unwrap fails on random bytes with 2^-128 probability)
- The fallback chain then tries v1, then v0, which succeeds

No behavioral change for v0 data. The only cost is one extra failed AEAD attempt (~1us).

---

## 8. Testing Plan

### 8.1 v2 Core Tests

| Test Name                    | Description                                              |
|------------------------------|----------------------------------------------------------|
| `v2_roundtrip`               | Encrypt + decrypt with same EncryptionKeys               |
| `v2_different_nonces`        | Same plaintext twice produces different ciphertext        |
| `v2_empty_plaintext`         | Encrypt/decrypt empty byte slice                         |
| `v2_large_plaintext`         | Encrypt/decrypt 10,000 bytes                             |
| `v2_header_format`           | Verify byte 0 is 0x02, byte 1 is current kek_id         |
| `v2_wrapped_dek_len_field`   | Verify bytes 2-3 encode 60 as big-endian u16             |

### 8.2 v2 Security Tests

| Test Name                          | Description                                         |
|------------------------------------|-----------------------------------------------------|
| `v2_tamper_wrapped_dek`            | Flip byte in wrapped DEK region, decrypt fails      |
| `v2_tamper_data_ciphertext`        | Flip byte in data ciphertext, decrypt fails         |
| `v2_tamper_data_nonce`             | Flip byte in data nonce, decrypt fails              |
| `v2_tamper_version_byte`           | Change version 0x02 to 0x03, v2 detection fails     |
| `v2_tamper_kek_id`                 | Change kek_id, unwrap fails (wrong key selected)    |
| `v2_wrong_key_fails`               | Decrypt with unrelated key, all fallbacks fail      |
| `v2_dek_uniqueness`                | Two encryptions of same plaintext have different wrapped DEKs |

### 8.3 v2 Key Rotation Tests

| Test Name                                | Description                                         |
|------------------------------------------|-----------------------------------------------------|
| `v2_rotation_decrypt_with_previous`      | Encrypt with KEK-A, rotate to B (prev=A), decrypt OK |
| `v2_rollback_decrypt_with_previous`      | Encrypt with KEK-B, rollback to A (prev=B), decrypt OK |
| `v2_second_rotation_without_rewrap_fails`| Encrypt with A, rotate to B, rotate to C (prev=B), decrypt fails (A gone) |

### 8.4 v2 Rewrap Tests

| Test Name                                | Description                                         |
|------------------------------------------|-----------------------------------------------------|
| `v2_rewrap_basic`                        | Encrypt with A, rewrap to B, decrypt with B-only keys |
| `v2_rewrap_idempotent`                   | Rewrap ciphertext already on current KEK returns same |
| `v2_rewrap_non_v2_returns_error`         | Rewrap a v1 ciphertext returns error                 |
| `v2_rewrap_then_decrypt`                 | Full cycle: encrypt, rotate, rewrap, decrypt          |
| `v2_rewrap_preserves_data`               | After rewrap, data_nonce + encrypted_data bytes identical |
| `v2_rewrap_unknown_kek_id_fails`         | Rewrap with kek_id not matching any configured key    |

### 8.5 Cross-Version Compatibility Tests

| Test Name                              | Description                                           |
|----------------------------------------|-------------------------------------------------------|
| `v2_decrypt_v0_data`                   | v0 ciphertext still decryptable after v2 upgrade      |
| `v2_decrypt_v1_data`                   | v1 ciphertext still decryptable after v2 upgrade      |
| `v2_mixed_versions_all_decrypt`        | Create v0, v1, v2 ciphertexts; all decrypt with same keys |
| `v2_v0_collision_fallback`             | v0 ciphertext starting with 0x02 still decrypts via fallback |

### 8.6 Counter and Stats Tests

| Test Name                              | Description                                           |
|----------------------------------------|-------------------------------------------------------|
| `v2_decrypt_stats_v2_current`          | v2 decrypt with current key bumps v2_current          |
| `v2_decrypt_stats_v2_previous`         | v2 decrypt with previous key bumps v2_previous        |
| `v2_decrypt_stats_mixed`               | Mix of v0, v1, v2 decrypts, verify all counters       |

### 8.7 Edge Case Tests

| Test Name                              | Description                                           |
|----------------------------------------|-------------------------------------------------------|
| `v2_ciphertext_too_short`              | Ciphertext < 92 bytes with version 0x02, falls through |
| `v2_invalid_wrapped_dek_len`           | wrapped_dek_len exceeds ciphertext length, falls through |
| `v2_debug_still_redacts`               | Debug format still shows [REDACTED] for keys           |

### Test Execution

All tests are in-module (`#[cfg(test)] mod tests`). They use the existing `test_config`
helper. No external dependencies, no I/O, no MongoDB -- pure unit tests.

Expected: ~35-40 new test functions, all existing tests unchanged and passing.

---

## 9. Implementation Notes

### 9.1 No New Dependencies

The implementation uses only crates already in `Cargo.toml`:
- `aes-gcm` (AES-256-GCM)
- `rand` (random nonce/DEK generation)
- `zeroize` (DEK zeroization)
- `sha2` (key ID derivation)

### 9.2 No Public API Changes

The following public API is unchanged:

```rust
impl EncryptionKeys {
    pub fn from_config(config: &AppConfig) -> Self;
    pub fn has_previous(&self) -> bool;
    pub fn decrypt_stats(&self) -> EncryptionDecryptStats;
    pub fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError>;
    pub fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError>;
}
```

The only addition is:

```rust
impl EncryptionKeys {
    pub fn rewrap(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError>;  // NEW
}
```

And `EncryptionDecryptStats` gains two new fields (`v2_current`, `v2_previous`).

### 9.3 No Changes Outside crypto/aes.rs

- **models/**: No changes (encrypted fields stay `Vec<u8>`)
- **services/**: No changes (they call `encryption_keys.encrypt()`/`decrypt()`)
- **handlers/**: No changes (they pass `&state.encryption_keys`)
- **config.rs**: No changes (no new env vars)
- **Cargo.toml**: No changes (no new dependencies)

### 9.4 Thread Safety

All new code maintains the existing thread-safety guarantees:
- `EncryptionKeys` fields are immutable after construction
- `DecryptCounters` uses `AtomicU64`/`AtomicBool` with `Ordering::Relaxed`
- `Zeroizing<[u8; 32]>` is stack-allocated, scoped to single function calls
- No `Mutex`, `RwLock`, or shared mutable state

### 9.5 Error Messages

Error messages from v2 decrypt failures should NOT leak implementation details:
- Public: `"AES decryption failed: no key could decrypt the data"` (same as today)
- Internal log (tracing::warn): May mention v2/DEK for debugging, but only at warn level
  and only once per counter path

---

## 10. Risks and Mitigations

### Risk 1: v0 Ciphertext Misidentified as v2

- **Likelihood**: Low (requires byte 0 == 0x02 AND len >= 92)
- **Impact**: None (v2 decrypt fails, falls through to v0 which succeeds)
- **Mitigation**: Fallback chain handles this transparently
- **Cost**: ~1us extra per misidentified record

### Risk 2: Performance Regression

- **Likelihood**: Very low
- **Impact**: ~1us extra per encrypt (DEK generation + wrapping), negligible per decrypt
- **Mitigation**: AES-256-GCM on 32-byte DEK is sub-microsecond; the data encryption
  dominates total time
- **Measurement**: Add timing logs in development mode if needed

### Risk 3: DEK Not Zeroized on Panic

- **Likelihood**: Very low (AES-256-GCM operations don't panic)
- **Impact**: DEK may persist in memory if thread panics mid-operation
- **Mitigation**: `Zeroizing` uses `Drop` which is called during stack unwinding.
  Rust's default panic behavior (unwind) does call destructors. Only `panic=abort`
  would skip zeroization, which is acceptable (process is terminating).

### Risk 4: Rollback After Phase 2 Deploy

- **Likelihood**: Low (Phase 2 is a contained change)
- **Impact**: v2 ciphertexts become unreadable with Phase 1 code
- **Mitigation**: See Section 7.1 rollback procedure. Deploy Phase 2, stabilize before
  any key rotation. Re-encrypt v2 -> v1 before rollback if needed.

---

## 11. Implementation Checklist

- [ ] Add v2 constants (VERSION_V2, V2_HEADER_SIZE, WRAPPED_DEK_SIZE, V2_MIN_SIZE)
- [ ] Add `looks_like_v2()` function
- [ ] Update `encrypt()` to produce v2 envelopes
- [ ] Add private `try_decrypt_v2()` helper
- [ ] Update `decrypt()` to try v2 before v1
- [ ] Add `rewrap()` public method
- [ ] Update `DecryptCounters` with v2_current, v2_previous, logged_v2_previous
- [ ] Update `EncryptionDecryptStats` with v2_current, v2_previous
- [ ] Update `DecryptCounters::snapshot()` to include v2 fields
- [ ] Add all v2 core tests (Section 8.1)
- [ ] Add all v2 security tests (Section 8.2)
- [ ] Add all v2 rotation tests (Section 8.3)
- [ ] Add all v2 rewrap tests (Section 8.4)
- [ ] Add all cross-version tests (Section 8.5)
- [ ] Add all counter tests (Section 8.6)
- [ ] Add all edge case tests (Section 8.7)
- [ ] Update existing test assertions for EncryptionDecryptStats (add v2 fields)
- [ ] Verify `cargo test` passes (all existing + new tests)
- [ ] Verify `cargo clippy` has no warnings
