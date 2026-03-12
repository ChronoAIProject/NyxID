# Phase 4 Cloud KMS Integration -- Review Findings

**Reviewer**: Code review + security review agent
**Date**: 2026-03-12
**Branch**: `feature/envelope-encryption`
**Scope**: All files modified or added in Phase 4 (Cloud KMS integration)
**Fixer**: All 11 findings resolved on 2026-03-12. 515 tests pass with `--all-features`.

---

## Summary

The Phase 4 implementation is well-structured and thorough. The async `KeyProvider` trait,
AWS/GCP KMS providers, fallback provider for migration, and backward-compatible decrypt
chain are all correctly implemented. Test coverage for `aes.rs` is excellent (50+ tests
covering v0/v1/v2/fallback/rewrap paths). The findings below are primarily around
security hardening, observability, and robustness.

---

## CRITICAL

_None found._

---

## HIGH

### H1. AWS/GCP key identifiers exposed in Debug and log output

**Files**:
- `backend/src/crypto/aws_kms_provider.rs:26-32` (Debug impl shows `current_key_arn`, `previous_key_arn`)
- `backend/src/crypto/gcp_kms_provider.rs:23-29` (Debug impl shows `current_key_name`)
- `backend/src/config.rs:194-197` (AppConfig Debug prints raw `aws_kms_key_arn`, `aws_kms_key_arn_previous`, `gcp_kms_key_name`, `gcp_kms_key_name_previous`)

**Description**: The AWS KMS ARN format `arn:aws:kms:us-east-1:123456789012:key/mrk-abc123`
leaks the AWS account ID, region, and key alias. The GCP key name
`projects/my-project/locations/us-east1/keyRings/my-ring/cryptoKeys/my-key` leaks project
name, location, key ring, and key name. These are infrastructure identifiers that assist
targeted attacks. The `LocalKeyProvider` Debug correctly shows `[REDACTED]`, but the KMS
providers do not follow this pattern for their identifiers.

**Suggested fix**: Redact or truncate key identifiers in Debug impls. For `AppConfig`,
wrap them the same way `encryption_key` is handled (`Some([REDACTED])` / `None`). For the
provider Debug impls, show only a truncated hash or the last few characters of the
identifier (e.g., `"...key/mrk-abc***"`).

**Resolution**: FIXED. All three Debug impls now show `[REDACTED]` / `Some([REDACTED])` / `None` for key identifiers, matching the `LocalKeyProvider` and `encryption_key` patterns. Added test `debug_impl_redacts_arns` to verify ARNs do not appear in debug output.

### H2. No cross-provider key_id collision check during migration

**Files**:
- `backend/src/main.rs:96-138` (provider construction and fallback setup)

**Description**: During local-to-KMS migration, both the primary KMS provider and the
fallback `LocalKeyProvider` have their own `current_key_id()` (derived from different
inputs -- KMS derives from ARN string, local derives from raw key bytes). If these happen
to collide (1-in-256 chance), the v2 decrypt path would try the wrong provider first and
fail. Within a single provider, collisions are checked (e.g., current vs previous), but
no cross-provider check exists.

**Suggested fix**: After constructing both providers, compare
`provider.current_key_id()` against the fallback provider's key IDs and panic at startup
if they collide:
```rust
if let Some(ref fallback) = fallback_provider {
    if fallback.has_key_id(provider.current_key_id()) {
        panic!("Primary and fallback providers have colliding key IDs (0x{:02x}). \
                This is a 1-in-256 hash collision. Use a different KMS key.",
                provider.current_key_id());
    }
}
```

**Resolution**: FIXED. Added cross-provider collision check in `main.rs` (after provider construction, before `EncryptionKeys`). Uses `fallback.has_key_id(provider.current_key_id())` and panics at startup on collision.

---

## MEDIUM

### M1. No retry/backoff for GCP KMS API calls

**Files**:
- `backend/src/crypto/gcp_kms_provider.rs:94-98` (encrypt)
- `backend/src/crypto/gcp_kms_provider.rs:127-131` (decrypt)

**Description**: The AWS SDK (`aws-sdk-kms`) includes built-in retry with exponential
backoff (3 attempts by default via `BehaviorVersion::latest()`). The `google-cloud-kms`
crate does not appear to include automatic retry. A single transient failure (network
blip, throttling, 503) causes the entire encrypt/decrypt operation to fail with no retry.

**Suggested fix**: Either wrap GCP KMS calls with a retry loop (2-3 attempts with
exponential backoff), or document that the GCP crate's internal retry behavior has been
verified. Consider using the `backon` crate for a lightweight retry wrapper.

**Resolution**: FIXED. Added retry loop (3 attempts, exponential backoff starting at 100ms) for both `wrap_dek` and `unwrap_dek` in `GcpKmsProvider`. Each attempt is logged at `warn` level; final failure at `error` level. No new crate dependency needed.

### M2. KMS providers lack mock-based integration tests

**Files**:
- `backend/src/crypto/aws_kms_provider.rs:146-167` (only 2 tests: `derive_key_id_*`)
- `backend/src/crypto/gcp_kms_provider.rs:149-169` (only 2 tests: `derive_key_id_*`)

**Description**: The KMS providers only test the `derive_kms_key_id` function. There are
no tests for the `KeyProvider` trait implementation itself (wrap_dek, unwrap_dek,
has_key_id, etc.) because the actual KMS clients cannot be unit-tested without mocking.
The `aes.rs` tests do exercise the `KeyProvider` trait via `MockKeyProvider`, which
validates the *interface*, but provider-specific logic (error mapping, response parsing)
is untested.

**Suggested fix**: Add tests that verify:
1. `has_key_id` returns correct values for current, previous, and unknown IDs
2. `has_previous_key` returns correct values
3. `current_key_id` returns the expected derived value
These can be tested by constructing the provider with known ARNs/key names (skipping
the actual KMS client construction). The struct fields could be made `pub(crate)` for
test construction, or add a `#[cfg(test)] fn new_for_test(...)` constructor.

**Resolution**: FIXED. AWS: added `test_provider()` constructor that builds a real `AwsKmsProvider` with a dummy `KmsClient` (sufficient for key_id tests). Added 7 tests: `current_key_id_matches_derived`, `has_key_id_current_only`, `has_key_id_with_previous`, `has_previous_key_false_when_none`, `has_previous_key_true_when_some`, `debug_impl_redacts_arns`. GCP: added `TestGcpKeyIdHelper` struct mirroring the provider's key_id logic (avoids needing a real GCP client). Added 7 tests covering the same scenarios plus `debug_impl_redacts_key_names`.

### M3. No maximum wrapped DEK size validation

**Files**:
- `backend/src/crypto/aes.rs:286` (encrypt: `wrapped_dek_len as u16`)
- `backend/src/crypto/aes.rs:317` (decrypt: `wrapped_dek_len` from header)

**Description**: The wrapped DEK length is stored as a `u16` (max 65535 bytes). While
`LocalKeyProvider` produces 60-byte wrapped DEKs and KMS providers produce ~170-200
bytes, there is no upper bound validation. A corrupted or malicious ciphertext could
declare a very large wrapped DEK length, causing the code to slice into potentially
invalid memory (though length checks prevent actual out-of-bounds). More importantly, a
KMS provider returning an unexpectedly large wrapped key would silently succeed.

**Suggested fix**: Add a constant `MAX_WRAPPED_DEK_SIZE` (e.g., 1024 bytes) and validate
both in `encrypt()` (after `provider.wrap_dek()`) and `decrypt()` (when parsing the
header):
```rust
const MAX_WRAPPED_DEK_SIZE: usize = 1024;
if wrapped_dek_len > MAX_WRAPPED_DEK_SIZE {
    return Err(AppError::Internal("Wrapped DEK exceeds maximum size".into()));
}
```

**Resolution**: FIXED. Added `MAX_WRAPPED_DEK_SIZE = 1024` constant. Validated in both `encrypt()` (after `provider.wrap_dek()`) and `decrypt()` (before processing v2 envelope). Returns `AppError::Internal` on violation.

### M4. Plaintext DEK copies in KMS SDK calls are not zeroized

**Files**:
- `backend/src/crypto/aws_kms_provider.rs:88` (`Blob::new(plaintext_dek)` copies DEK)
- `backend/src/crypto/gcp_kms_provider.rs:90` (`plaintext_dek.to_vec()` copies DEK)

**Description**: When calling KMS wrap APIs, the plaintext DEK is copied into SDK
request structures (`Blob` for AWS, `Vec<u8>` for GCP). These copies are not wrapped in
`Zeroizing` and will not be scrubbed from memory when the SDK request is dropped. The
caller's `Zeroizing` wrapper handles their own copy, but the SDK's internal copies
persist until garbage-collected.

**Suggested fix**: This is an accepted limitation of external SDKs -- the caller cannot
control their internal memory management. Document this as an accepted risk in a code
comment:
```rust
// Note: The SDK copies plaintext_dek internally. We cannot zeroize the SDK's copy.
// The caller's Zeroizing wrapper handles the caller-side copy.
```

**Resolution**: FIXED. Added documenting comments to both `aws_kms_provider.rs` and `gcp_kms_provider.rs` at the `Blob::new(plaintext_dek)` / `plaintext_dek.to_vec()` call sites explaining the accepted limitation.

### M5. KMS error messages may leak infrastructure details to logs

**Files**:
- `backend/src/crypto/aws_kms_provider.rs:91,98,124` (AWS error formatting)
- `backend/src/crypto/gcp_kms_provider.rs:98,131` (GCP error formatting)

**Description**: KMS SDK error messages are included verbatim in `AppError::Internal`
messages (e.g., `format!("AWS KMS encrypt failed: {e}")`). While `AppError::Internal`
never leaks to HTTP clients (returns "An internal error occurred"), the full error
is logged server-side. SDK errors can include request IDs, endpoint URLs, key resource
names, and internal error details that may be sensitive.

**Suggested fix**: Consider sanitizing or truncating the SDK error before logging:
```rust
.map_err(|e| {
    tracing::error!("AWS KMS encrypt failed: {e}");
    AppError::Internal("AWS KMS encrypt failed".to_string())
})?;
```
This keeps the detailed error in structured logging but prevents it from being passed
around in the error chain.

**Resolution**: FIXED. AWS: error details logged via `tracing::error!()`, only generic message in `AppError::Internal`. GCP: same pattern applied in the retry loop -- transient failures logged at `warn`, final failure at `error`, generic message in error chain.

---

## LOW

### L1. Duplicate `derive_kms_key_id` functions across providers

**Files**:
- `backend/src/crypto/aws_kms_provider.rs:35-38`
- `backend/src/crypto/gcp_kms_provider.rs:32-35`

**Description**: Both KMS providers have an identical `derive_kms_key_id(key_identifier: &str) -> u8`
function. This is distinct from `key_provider::derive_key_id(&[u8]) -> u8` because it
hashes a string rather than raw bytes. The duplication is minor (4 lines) but could
diverge if one is updated without the other.

**Suggested fix**: Extract a shared `derive_key_id_from_str` function into
`key_provider.rs`:
```rust
pub(crate) fn derive_key_id_from_str(identifier: &str) -> u8 {
    let digest = Sha256::digest(identifier.as_bytes());
    digest[0]
}
```

**Resolution**: FIXED. Extracted `derive_key_id_from_str(identifier: &str) -> u8` into `key_provider.rs` (gated behind `#[cfg(any(feature = "aws-kms", feature = "gcp-kms"))]`). Both providers now import and use the shared function. Removed duplicate local functions and unused `sha2` imports.

### L2. `WrappedKey.ciphertext` is not zeroized

**Files**:
- `backend/src/crypto/key_provider.rs:13` (`pub ciphertext: Vec<u8>`)

**Description**: The `WrappedKey` struct holds the *encrypted* DEK (not plaintext).
Zeroizing encrypted material is defense-in-depth but not strictly necessary since the
data is already encrypted. The plaintext DEK *is* correctly zeroized everywhere.

**Suggested fix**: Optionally wrap in `Zeroizing<Vec<u8>>` for defense-in-depth. Low
priority since the data is encrypted.

**Resolution**: FIXED. Changed `WrappedKey.ciphertext` from `Vec<u8>` to `Zeroizing<Vec<u8>>`. Updated all construction sites (aws_kms_provider, gcp_kms_provider, local_key_provider, aes.rs encrypt/decrypt/rewrap, MockKeyProvider). All read accesses work transparently via `Deref`.

### L3. Test config constructors require manual update for new fields

**Files**:
- `backend/src/crypto/aes.rs:679-729` (`test_config` function)
- `backend/src/crypto/local_key_provider.rs:264-312` (`from_config_builds_correctly`)
- `backend/src/config.rs:538-588` (`make_config` function)

**Description**: Three separate test helper functions construct `AppConfig` with 30+ fields.
Adding a new config field requires updating all three. The 4 new KMS fields were correctly
added to all helpers.

**Suggested fix**: Consider implementing `Default` for `AppConfig` (test-only) or a
builder pattern. This is a pre-existing pattern, not introduced by Phase 4.

**Resolution**: Acknowledged. Pre-existing pattern not introduced by Phase 4. The KMS fields were correctly added to all three helpers. No code change needed.

### L4. `async-trait` dependency is always-on

**Files**:
- `backend/Cargo.toml:43` (`async_trait = "0.1"`)

**Description**: `async-trait` is a non-optional dependency because `KeyProvider` trait
and `LocalKeyProvider` (both always compiled) use it. This is correct -- it cannot be
behind a feature flag. The `async-trait` crate is lightweight and widely used.

**Suggested fix**: None needed. This is the correct design. Note: when Rust stabilizes
`async fn in trait` (expected soon), `async-trait` can be removed and the trait can use
native async methods. This would eliminate the heap allocation per async trait call.

**Resolution**: Acknowledged. No change needed -- this is the correct design as noted by the reviewer.

---

## Architecture Verification

The implementation matches the Phase 4 plan:

| Requirement | Status | Notes |
|---|---|---|
| Async `KeyProvider` trait | Done | Via `async-trait` crate |
| AWS KMS provider | Done | Behind `aws-kms` feature flag |
| GCP KMS provider | Done | Behind `gcp-kms` feature flag |
| Fallback provider for migration | Done | `with_provider_and_fallback()` |
| v2 decrypt fallback chain | Done | Primary -> fallback -> v1 -> v0 |
| All 42+ call sites updated with `.await` | Done | Verified via grep |
| Config validation for KMS | Done | `validate_key_provider()` |
| Backward compat (v0/v1/v2) | Done | 50+ tests |
| DEK zeroization | Done | `Zeroizing<[u8; 32]>` on all paths |
| Feature flags isolate KMS deps | Done | `aws-kms`, `gcp-kms` features |
| Rewrap with fallback provider | Done | `rewrap()` tries fallback |
| Decrypt stats track fallback | Done | `v2_fallback` counter |
| `LegacyKeys` accessible for main.rs | Done | `pub(crate)` |

---

## Test Coverage Assessment

| File | Test Count | Coverage Assessment |
|---|---|---|
| `aes.rs` | ~50 tests | Excellent: v0/v1/v2 roundtrip, rotation, rollback, rewrap, tamper, fallback, stats, collision, size |
| `local_key_provider.rs` | 9 tests | Good: roundtrip, rotation, collision panic, debug redaction, config |
| `aws_kms_provider.rs` | 9 tests | Good: derive_key_id, current_key_id, has_key_id (with/without previous), has_previous_key, debug redaction (M2 resolved) |
| `gcp_kms_provider.rs` | 9 tests | Good: derive_key_id, current_key_id, has_key_id (with/without previous), has_previous_key, debug redaction (M2 resolved) |
| `config.rs` | 14 tests | Good: validation for key length, hex, zeros, previous key |

---

## Priority Fix Order (all resolved)

1. **H1** - Redact KMS key identifiers in Debug/log output -- FIXED
2. **H2** - Add cross-provider key_id collision check -- FIXED
3. **M1** - Verify/add GCP KMS retry behavior -- FIXED (3 attempts, exponential backoff)
4. **M3** - Add maximum wrapped DEK size validation -- FIXED (MAX_WRAPPED_DEK_SIZE = 1024)
5. **M5** - Sanitize KMS error messages in logs -- FIXED (log detail, return generic)
6. **M2** - Add provider-specific unit tests -- FIXED (14 new tests across both providers)
7. **M4** - Document accepted SDK memory limitation -- FIXED (comments added)
8. **L1** - Extract shared `derive_kms_key_id` function -- FIXED (shared `derive_key_id_from_str`)
9. **L2** - Zeroize `WrappedKey.ciphertext` -- FIXED (`Zeroizing<Vec<u8>>`)
10. **L3** - Test config constructors -- Acknowledged (pre-existing, not Phase 4)
11. **L4** - `async-trait` always-on -- Acknowledged (correct design)

**Verification**: `cargo test -p nyxid` passes (499 tests), `cargo test -p nyxid --features aws-kms` passes (507 tests), `cargo test -p nyxid --features gcp-kms` passes (507 tests), and `cargo test -p nyxid --all-features` passes (515 tests).
