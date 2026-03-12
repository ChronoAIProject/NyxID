# Envelope Encryption Code Review & Security Review

## Review Summary

**File reviewed:** `backend/src/crypto/aes.rs` (1383 lines)
**Design doc:** `docs/ENVELOPE_ENCRYPTION_DESIGN.md`
**Branch:** `feature/envelope-encryption`
**Reviewer:** code-reviewer + security-reviewer agent

### Verdict: APPROVE WITH WARNINGS

The implementation is well-structured, follows the design doc closely, and maintains
full backward compatibility. No CRITICAL or HIGH issues were found. Several MEDIUM and
LOW findings are documented below for the fixer agent.

### Test Results

- All 49 tests pass (13 legacy v0, 18 v1, 18 v2)
- No clippy warnings in `crypto/aes.rs`
- Pre-existing clippy warnings in other files (approvals, developer_apps) are unrelated

---

## Code Review Findings

### [MEDIUM] DEK raw bytes not zeroized before Zeroizing wrapper in decrypt_v2_payload
**File:** backend/src/crypto/aes.rs:480-486
**Description:** In `decrypt_v2_payload()`, the DEK is first decrypted into `dek_bytes`
(a plain `Vec<u8>`) on line 480, then copied into a `Zeroizing<[u8; 32]>` on lines 483-486.
The original `dek_bytes` Vec is not explicitly zeroized -- it will be freed by the allocator
but its contents may linger in heap memory. The same pattern appears in `rewrap()` at
lines 403-409.
**Recommendation:** Use `zeroize::Zeroizing` to wrap the `Vec<u8>` returned from decrypt,
or zeroize `dek_bytes` explicitly before it drops:
```rust
let mut dek_bytes = kek_cipher
    .decrypt(dek_nonce, dek_ciphertext)
    .map_err(|e| AppError::Internal(format!("DEK unwrap failed: {e}")))?;
let dek = Zeroizing::new(
    <[u8; 32]>::try_from(dek_bytes.as_slice())
        .map_err(|_| AppError::Internal("Unwrapped DEK is not 32 bytes".to_string()))?,
);
dek_bytes.zeroize(); // Zeroize the intermediate Vec
```
Note: Import `zeroize::Zeroize` trait (not just `Zeroizing`) for the `.zeroize()` method.

### [MEDIUM] File exceeds 800-line guideline
**File:** backend/src/crypto/aes.rs (1383 lines)
**Description:** Per project coding standards, files should be 200-400 lines typical with
800 max. The file is 1383 lines, though ~780 of those are tests. The production code is
~510 lines, which is reasonable but still over the 400 typical target.
**Recommendation:** This is acceptable for a self-contained crypto module where keeping
everything in one file aids auditability. No action required unless the team prefers to
split tests into a separate file (e.g., `aes_tests.rs`). Mark as acknowledged.

### [LOW] Inconsistent test naming convention
**File:** backend/src/crypto/aes.rs:762-776
**Description:** The existing v1 tests (e.g., `v1_roundtrip`, `v1_empty_plaintext`) were
updated to verify v2 output but their names still start with `v1_`. This is technically
correct (they test the v1 backward-compat path for decrypt, and the encrypt path now
produces v2), but the comment on line 770 "Verify v2 header (encrypt now produces v2
format)" shows the naming is slightly misleading.
**Recommendation:** Consider adding a comment block above these tests clarifying they
test the `EncryptionKeys` public API (which now produces v2) rather than v1-specific
behavior. The v1-specific decrypt tests at lines 1019-1043 are correctly named.

### [LOW] encrypt_v1 test helper could be marked #[cfg(test)]
**File:** backend/src/crypto/aes.rs:575-596
**Description:** The `encrypt_v1` function is already gated behind `#[cfg(test)]`, which
is correct. No issue -- this is informational.
**Recommendation:** No action needed.

### [INFO] Constants are well-defined and match design doc
**File:** backend/src/crypto/aes.rs:38-45
**Description:** `V2_HEADER_SIZE=4`, `WRAPPED_DEK_SIZE=60`, `V2_MIN_SIZE=92` all match the
design doc exactly. The `TAG_SIZE=16` constant was added (previously inline magic number)
which improves readability.
**Recommendation:** No action needed. Good improvement.

### [INFO] Public API unchanged as specified
**File:** backend/src/crypto/aes.rs:181,232,361
**Description:** `encrypt()` and `decrypt()` signatures are unchanged. `rewrap()` is a new
public method. `EncryptionDecryptStats` gained `v2_current` and `v2_previous` fields. All
per design doc.
**Recommendation:** No action needed.

---

## Security Review Findings

### [MEDIUM] Intermediate DEK Vec not zeroized (same as code review finding)
**File:** backend/src/crypto/aes.rs:480-486, 403-409
**Description:** (See code review finding above.) The raw DEK bytes from AES-GCM decrypt
are stored in a heap-allocated `Vec<u8>` before being copied into `Zeroizing<[u8; 32]>`.
The Vec's memory is not guaranteed to be zeroed by the allocator after deallocation.
While this is a narrow window and requires memory access to exploit (physical access or
a separate vulnerability), it violates defense-in-depth for key material handling.
**Recommendation:** Zeroize the intermediate Vec explicitly in both `decrypt_v2_payload()`
and `rewrap()`. See code example in the code review section.

### [LOW] derive_key_id uses only 1 byte of SHA-256 (1/256 collision probability)
**File:** backend/src/crypto/aes.rs:455-458
**Description:** The key ID is derived as the first byte of SHA-256(key), giving only 256
possible values. With 2 keys (current + previous), a collision probability of ~1/256 is
checked at startup via `assert_ne!`. However, if a user happens to generate two keys with
the same first SHA-256 byte, they'll get a startup panic. This is a pre-existing design
from Phase 1, not introduced by this PR.
**Recommendation:** No change needed for this PR. For a future improvement, consider using
2 bytes (u16) for the key ID to reduce collision probability to ~1/65536. Document the
1/256 collision risk in the architecture docs.

### [LOW] Error messages from rewrap contain operational details
**File:** backend/src/crypto/aes.rs:363-391
**Description:** Error messages like "rewrap() only supports v2 envelope format" and
"Ciphertext kek_id does not match current or previous key" are returned via `AppError::Internal`.
These are internal errors that should not reach end users. The `AppError::Internal` variant
maps to HTTP 500 and the error message is typically not exposed in the API response (verified
by checking the error handling pattern in `errors/mod.rs`).
**Recommendation:** Confirm that `AppError::Internal` messages are not leaked to API
responses. If they are, sanitize these messages. Based on the existing error handling
pattern, this appears safe -- `Internal` errors return a generic "Internal server error"
to clients. No action needed.

### [INFO] DEK generated with cryptographically secure RNG
**File:** backend/src/crypto/aes.rs:183-184
**Description:** DEK is generated via `rand::thread_rng().fill_bytes()`, which uses the
OS CSPRNG (via `ThreadRng` -> `OsRng` internally in the `rand` crate). This is
cryptographically appropriate.
**Recommendation:** No action needed.

### [INFO] DEK wrapped in Zeroizing for automatic cleanup
**File:** backend/src/crypto/aes.rs:183
**Description:** The DEK in `encrypt()` is held in `Zeroizing<[u8; 32]>`, ensuring it is
zeroed on drop. The DEK never escapes the function scope. This is correct.
**Recommendation:** No action needed.

### [INFO] Separate nonces for DEK wrapping and data encryption
**File:** backend/src/crypto/aes.rs:189-204
**Description:** Two independent random nonces are generated: `data_nonce_bytes` for data
encryption and `dek_nonce_bytes` for DEK wrapping. Since they use different keys (DEK vs
KEK), even nonce collision would not be exploitable, but the nonces are independently random
anyway. This is correct.
**Recommendation:** No action needed.

### [INFO] Fallback chain does not enable downgrade attacks
**File:** backend/src/crypto/aes.rs:232-354
**Description:** The decrypt fallback chain tries v2 -> v1 -> v0. A v2 ciphertext that fails
v2 parsing falls through to v1/v0, which will also fail (v2 data cannot accidentally decrypt
as v0 because the AEAD tag check will fail with ~2^-128 probability). An attacker cannot
force a downgrade because:
1. They cannot forge a valid v0/v1 ciphertext from a v2 ciphertext without the key
2. Stripping the v2 header would result in invalid data for v0/v1 AEAD
**Recommendation:** No action needed. The design is sound.

### [INFO] rewrap() validates input format correctly
**File:** backend/src/crypto/aes.rs:361-436
**Description:** `rewrap()` checks: (1) v2 format via `looks_like_v2()`, (2) ciphertext length
vs declared `wrapped_dek_len`, (3) kek_id matches current or previous, (4) DEK unwrap
succeeds, (5) DEK is exactly 32 bytes. This is comprehensive input validation.
**Recommendation:** No action needed.

### [INFO] No integer overflow risk in wrapped_dek_len parsing
**File:** backend/src/crypto/aes.rs:238-240
**Description:** `wrapped_dek_len` is parsed as `u16` (max 65535) and then added to
`V2_HEADER_SIZE` (4) + `NONCE_SIZE` (12) + `TAG_SIZE` (16) = 32. Max `required_len` =
65535 + 32 = 65567, well within `usize` range. No overflow possible.
**Recommendation:** No action needed.

### [INFO] Debug impl properly redacts all key material
**File:** backend/src/crypto/aes.rs:104-118
**Description:** The custom `Debug` impl shows `[REDACTED]` for both current and previous
keys. The test `v1_debug_redacts_keys` confirms no key bytes leak. This is unchanged from
Phase 1 and remains correct.
**Recommendation:** No action needed.

### [INFO] Tamper detection verified by tests
**File:** backend/src/crypto/aes.rs:1104-1139
**Description:** Tests cover tampering with: wrapped DEK bytes, data ciphertext/tag, and
kek_id byte. All correctly detect tampering via AES-GCM tag verification failure.
**Recommendation:** No action needed.

---

## Test Coverage Assessment

### Tests Added (18 v2 tests)
| Test | Covers |
|------|--------|
| `v2_roundtrip` | Basic encrypt/decrypt, header format |
| `v2_different_deks` | DEK uniqueness per operation |
| `v2_empty_plaintext` | Edge case: empty input |
| `v2_large_plaintext` | 100KB payload |
| `v2_tamper_wrapped_dek` | Tamper detection: DEK region |
| `v2_tamper_data` | Tamper detection: data region |
| `v2_tamper_kek_id` | Tamper detection: kek_id byte |
| `v2_kek_rotation` | Decrypt with previous KEK after rotation |
| `v2_rewrap_roundtrip` | Full rewrap cycle |
| `v2_rewrap_already_current` | Idempotent rewrap |
| `v2_rewrap_non_v2_fails` | Rewrap rejects v0/v1 |
| `v2_rewrap_preserves_data` | Data portion unchanged after rewrap |
| `v2_rollback` | Decrypt after KEK rollback |
| `v2_decrypt_stats` | v2_current counter |
| `v2_decrypt_stats_previous` | v2_previous counter |
| `v2_cross_version_all_formats` | v0 + v1 + v2 all decrypt with same keys |
| `v2_size_overhead` | Verify expected output size |
| `v2_rewrap_unknown_kek_id_fails` | Rewrap with unrecognized kek_id |
| `v2_second_rotation_without_rewrap_fails` | Double rotation without rewrap |
| `v2_rewrap_then_drop_old_key` | Rewrap enables safe key removal |

### Existing Tests Updated (2)
| Test | Change |
|------|--------|
| `v1_roundtrip` | Updated to verify v2 header (encrypt now produces v2) |
| `v1_cross_version_roundtrip` | Updated to verify v2 format |
| `v1_decrypt_stats_track_fallback_paths` | Updated stats struct with v2 fields |

### Missing Test Coverage (suggestions)
| Test | Description | Priority |
|------|-------------|----------|
| `v2_v0_collision_fallback` | v0 ciphertext starting with 0x02 still decrypts via fallback | LOW |
| `v2_tamper_version_byte` | Change version 0x02 to 0x03, verify fallback works | LOW |
| `v2_invalid_wrapped_dek_len` | wrapped_dek_len exceeds ciphertext length | LOW |

These missing tests are LOW priority -- the behavior is implicitly covered by the fallback
chain logic and existing tamper tests.

---

## Compliance with Design Doc

| Design Requirement | Status | Notes |
|---|---|---|
| v2 byte layout matches spec | PASS | Verified in `v2_roundtrip` and `v2_size_overhead` |
| Encrypt produces v2 format | PASS | All `encrypt()` calls produce v2 |
| Decrypt fallback: v2 -> v1 -> v0 | PASS | `v2_cross_version_all_formats` verifies |
| rewrap() method | PASS | Full test coverage |
| rewrap idempotent | PASS | `v2_rewrap_already_current` |
| DecryptCounters updated | PASS | v2_current, v2_previous fields + tests |
| No public API signature changes | PASS | Only `rewrap()` added |
| No changes outside aes.rs | PASS | Verified via git diff |
| DEK in Zeroizing | PASS | Lines 183, 406, 483 |
| Separate nonces | PASS | Lines 189-204 |
| No new dependencies | PASS | Only existing crates used |

---

## Summary of Actionable Items

| # | Severity | Finding | Action | Status |
|---|----------|---------|--------|--------|
| 1 | MEDIUM | Intermediate DEK Vec not zeroized in decrypt_v2_payload and rewrap | Add explicit `.zeroize()` call on `dek_bytes` Vec | RESOLVED: Added `dek_bytes.zeroize()` in both `decrypt_v2_payload()` and `rewrap()`. Imported `zeroize::Zeroize` trait. |
| 2 | MEDIUM | File exceeds 800-line guideline | Acknowledge -- acceptable for crypto module | ACKNOWLEDGED: Crypto module benefits from single-file auditability. |
| 3 | LOW | v1 test names slightly misleading post-v2 upgrade | Consider adding clarifying comment block | RESOLVED: Added comment block above v1 tests explaining they test the EncryptionKeys public API. |
| 4 | LOW | Missing 3 edge case tests from design doc | Add if time permits | RESOLVED: Added `v2_v0_collision_fallback`, `v2_tamper_version_byte`, `v2_invalid_wrapped_dek_len` tests. Total: 52 tests. |
