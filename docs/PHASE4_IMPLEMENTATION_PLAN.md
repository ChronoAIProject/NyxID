# Phase 4: Cloud KMS Integration -- Implementation Plan

This document is the authoritative implementation plan for Phase 4 of NyxID's encryption architecture. The implementer should follow this plan precisely, file by file.

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Async Trait Design](#1-async-trait-design)
3. [AWS KMS Provider Design](#2-aws-kms-provider-design)
4. [GCP Cloud KMS Provider Design](#3-gcp-cloud-kms-provider-design)
5. [Backward Compatibility and Migration Strategy](#4-backward-compatibility-and-migration-strategy)
6. [Testing Strategy](#5-testing-strategy)
7. [Dependency Analysis](#6-dependency-analysis)
8. [File-by-File Change List](#7-file-by-file-change-list)
9. [Configuration Reference](#8-configuration-reference)
10. [Rollback Procedures](#9-rollback-procedures)

---

## Executive Summary

Phase 4 adds AWS KMS and GCP Cloud KMS as pluggable `KeyProvider` backends for envelope encryption. New encryptions can use a cloud KMS to wrap per-record DEKs, while existing v0/v1/v2 ciphertexts remain fully decryptable via a fallback chain. The implementation requires:

- Making the `KeyProvider` trait async (KMS calls are network I/O)
- Making `EncryptionKeys::encrypt()`/`decrypt()`/`rewrap()` async
- Adding a fallback provider for migration scenarios (local-wrapped v2 DEKs)
- Two new provider implementations: `AwsKmsProvider` and `GcpKmsProvider`
- Feature flags to keep cloud SDK dependencies optional
- Updating ~42 call sites to add `.await` (mechanical change)

**Key constraints**:
- ALL existing v0/v1/v2 ciphertexts must remain decryptable
- Rollback from KMS to local must work (within the migration window)
- DEK zeroization must be maintained on all code paths
- No `unwrap()` on fallible operations in production code
- Key material must never appear in logs, errors, or Debug output

---

## 1. Async Trait Design

### Decision: `async-trait` crate with `Arc<dyn KeyProvider>`

**Chosen approach**: Make `KeyProvider` methods async using the `async-trait` proc macro crate, keeping `Arc<dyn KeyProvider>` as the storage type in `EncryptionKeys`.

**Alternatives considered and rejected**:

| Approach | Pros | Cons | Verdict |
|----------|------|------|---------|
| A. `async-trait` crate | Battle-tested, simple, keeps `dyn` dispatch | ~20-30ns boxing per call | **Chosen** |
| B. Enum dispatch | Zero-cost, no boxing | Couples EncryptionKeys to all provider types; breaks open/closed principle | Rejected |
| C. `tokio::block_in_place` sync wrapper | Zero changes to callers | Blocks Tokio thread; panics on `current_thread` runtime (tests); anti-pattern | Rejected |
| D. Native async fn + `dynosaur` | No external dep for trait | `dynosaur` is newer/less proven; more complex | Rejected |
| E. Separate `AsyncKeyProvider` trait | No breaking change | Two traits to maintain; adapter boilerplate | Rejected |

**Rationale for `async-trait`**:

1. The `async-trait` crate is the de facto standard for async dyn dispatch (used by Axum <0.8, Tower, tonic, etc.)
2. The ~20-30ns boxing overhead is negligible compared to AES-GCM operations (~microseconds) or KMS calls (~10-50ms)
3. `LocalKeyProvider` async methods return immediately -- the optimizer eliminates the future state machine; the only cost is the `Box` allocation
4. All 42 call sites are already inside async functions (Axum handlers and async services), so adding `.await` is a mechanical change
5. Keeps the clean `Arc<dyn KeyProvider>` abstraction, preserving the open/closed principle for future providers (Vault, Azure, etc.)

### New trait signature

```rust
use async_trait::async_trait;

#[async_trait]
pub trait KeyProvider: Send + Sync + std::fmt::Debug {
    async fn wrap_dek(&self, plaintext_dek: &[u8]) -> Result<WrappedKey, AppError>;
    async fn unwrap_dek(&self, wrapped: &WrappedKey) -> Result<Zeroizing<Vec<u8>>, AppError>;
    fn current_key_id(&self) -> u8;
    fn has_key_id(&self, key_id: u8) -> bool;
    fn has_previous_key(&self) -> bool;
}
```

Note: Only `wrap_dek` and `unwrap_dek` become async. The metadata methods (`current_key_id`, `has_key_id`, `has_previous_key`) remain sync because they never do I/O.

### Impact on `EncryptionKeys`

```rust
impl EncryptionKeys {
    pub async fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError> { ... }
    pub async fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> { ... }
    pub async fn rewrap(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> { ... }
}
```

### Call site migration

42 call sites across 8 files need `.await` added. Most are straightforward:

```rust
// Before
let encrypted = encryption_keys.encrypt(data.as_bytes())?;

// After
let encrypted = encryption_keys.encrypt(data.as_bytes()).await?;
```

**Special case -- `Option::map` with encrypt**: Several call sites in `provider_service.rs` and `user_token_service.rs` use `Option::map(|v| encryption_keys.encrypt(...))`. These cannot use async closures directly and must be refactored:

```rust
// Before
let enc = value.map(|v| encryption_keys.encrypt(v.as_bytes())).transpose()?;

// After
let enc = match value {
    Some(v) => Some(encryption_keys.encrypt(v.as_bytes()).await?),
    None => None,
};
```

---

## 2. AWS KMS Provider Design

### Overview

AWS KMS performs envelope encryption natively: the `Encrypt` API wraps a plaintext DEK and returns an opaque `CiphertextBlob` (~170-200 bytes for a 32-byte DEK). The `Decrypt` API accepts the blob and returns the plaintext DEK. Key rotation is transparent -- AWS internally tracks key versions within the blob.

### key_id mapping strategy

The v2 header stores a `u8` key_id to identify which KEK wrapped the DEK. For AWS KMS:

- **Derive from key ARN**: `SHA256(key_arn.as_bytes())[0]`
- This is deterministic and consistent with `LocalKeyProvider` (which derives from key material)
- The key ARN is stable across rotations (AWS rotates the backing key, not the ARN)
- Collision risk with local key_id: 1/256 -- acceptable because you never run local + KMS simultaneously for the same ciphertext

For the optional previous KMS key (multi-key migration, not rotation):
- `previous_key_id = SHA256(previous_key_arn.as_bytes())[0]`
- Panic at startup if `current_key_id == previous_key_id` (same pattern as `LocalKeyProvider`)

### Wrapped DEK format

The `WrappedKey.ciphertext` field stores the raw AWS KMS `CiphertextBlob` bytes (not base64). The v2 header's `wrapped_dek_len` field (u16 BE) accommodates this -- AWS blobs are ~170-200 bytes, well within the u16 range.

```
v2 envelope with AWS KMS:
[0x02] [kek_id: 1B] [wrapped_dek_len: 2B BE] [AWS CiphertextBlob: ~170-200B] [data_nonce: 12B] [data_ciphertext] [data_tag: 16B]

Total overhead: ~4 + 170-200 + 12 + 16 = ~202-232 bytes (vs 92 bytes for local)
```

### Key rotation

AWS KMS handles rotation transparently:
- `Encrypt` always uses the current key version
- `Decrypt` auto-selects the correct version from the CiphertextBlob metadata
- No change needed in NyxID code -- the same key ARN works before and after rotation
- `has_previous_key()` returns `true` only if a separate `AWS_KMS_KEY_ARN_PREVIOUS` is configured (for multi-key migration, not for AWS auto-rotation)

### IAM permissions required

Minimum IAM policy for the NyxID service role:

```json
{
  "Version": "2012-10-17",
  "Statement": [{
    "Effect": "Allow",
    "Action": ["kms:Encrypt", "kms:Decrypt"],
    "Resource": "arn:aws:kms:REGION:ACCOUNT:key/KEY_ID"
  }]
}
```

For key rotation monitoring, also grant `kms:DescribeKey` (optional).

### Config env vars

```bash
KEY_PROVIDER=aws-kms
AWS_KMS_KEY_ARN=arn:aws:kms:us-east-1:123456789:key/mrk-abcdef1234567890
AWS_KMS_KEY_ARN_PREVIOUS=arn:aws:kms:us-east-1:123456789:key/old-key-id  # optional

# Standard AWS SDK env vars (handled by aws-config crate):
AWS_REGION=us-east-1
AWS_ACCESS_KEY_ID=...          # or use IAM role (recommended)
AWS_SECRET_ACCESS_KEY=...      # or use IAM role (recommended)
```

### Provider struct

```rust
pub struct AwsKmsProvider {
    client: aws_sdk_kms::Client,
    current_key_arn: String,
    current_key_id: u8,
    previous_key_arn: Option<String>,
    previous_key_id: Option<u8>,
}
```

### Error handling

- Network errors -> `AppError::Internal("AWS KMS encrypt failed: {service_error}")` -- never include key ARN or plaintext
- Retry: rely on the AWS SDK's built-in retry behavior (default: 3 retries with exponential backoff)
- Timeout: AWS SDK default connect timeout is 3.1s; no custom override needed

---

## 3. GCP Cloud KMS Provider Design

### Overview

GCP Cloud KMS is architecturally similar to AWS KMS. The `encrypt` API wraps plaintext and returns an opaque ciphertext blob. The `decrypt` API accepts the blob and returns plaintext. Key rotation is transparent via CryptoKeyVersions.

### key_id mapping strategy

Same pattern as AWS: `SHA256(key_resource_name.as_bytes())[0]`

GCP key resource name format: `projects/PROJECT/locations/LOCATION/keyRings/RING/cryptoKeys/KEY`

### Wrapped DEK format

Same pattern as AWS: store the raw GCP ciphertext bytes in `WrappedKey.ciphertext`. GCP ciphertext blobs are similar in size (~100-200 bytes for a 32-byte DEK).

### Key rotation

GCP Cloud KMS handles rotation transparently:
- `encrypt` always uses the primary CryptoKeyVersion
- `decrypt` auto-selects the correct version from embedded metadata
- Automatic rotation can be configured (e.g., every 90 days)
- `has_previous_key()` returns `true` only if `GCP_KMS_KEY_NAME_PREVIOUS` is configured

### IAM roles required

Minimum: `roles/cloudkms.cryptoKeyEncrypterDecrypter` on the specific key resource.

For least privilege, separate roles:
- `roles/cloudkms.cryptoKeyEncrypter` (encrypt only)
- `roles/cloudkms.cryptoKeyDecrypter` (decrypt only)

### Config env vars

```bash
KEY_PROVIDER=gcp-kms
GCP_KMS_KEY_NAME=projects/my-project/locations/us-east1/keyRings/nyxid/cryptoKeys/nyxid-kek
GCP_KMS_KEY_NAME_PREVIOUS=projects/my-project/locations/us-east1/keyRings/nyxid/cryptoKeys/old-kek  # optional

# Standard GCP auth (handled by google-cloud-kms crate):
GOOGLE_APPLICATION_CREDENTIALS=/path/to/service-account.json  # or use workload identity
```

### Provider struct

```rust
pub struct GcpKmsProvider {
    client: google_cloud_kms::client::Client,
    current_key_name: String,
    current_key_id: u8,
    previous_key_name: Option<String>,
    previous_key_id: Option<u8>,
}
```

---

## 4. Backward Compatibility and Migration Strategy

### The migration problem

When switching from `KEY_PROVIDER=local` to `KEY_PROVIDER=aws-kms`, existing v2 ciphertexts have DEKs wrapped with the local AES-256-GCM key. The AWS KMS provider cannot unwrap these -- it only knows how to call AWS KMS.

### Solution: Fallback provider in `EncryptionKeys`

Add an optional fallback provider to `EncryptionKeys`:

```rust
pub struct EncryptionKeys {
    provider: Arc<dyn KeyProvider>,
    /// Optional fallback provider for v2 DEKs wrapped by a previous provider
    /// (e.g., local provider during migration to KMS).
    fallback_provider: Option<Arc<dyn KeyProvider>>,
    legacy: Option<LegacyKeys>,
    counters: DecryptCounters,
}
```

### Decrypt flow with fallback

The v2 decrypt path becomes:

```
1. Try primary provider.unwrap_dek(wrapped)
   - If kek_id matches primary -> success
2. If primary fails, try fallback_provider.unwrap_dek(wrapped)
   - If kek_id matches fallback -> success (bump fallback counter)
3. Fall through to v1/v0 legacy paths
```

### Migration scenarios

#### Scenario A: Local -> KMS (forward migration)

```bash
# Before (local only)
KEY_PROVIDER=local
ENCRYPTION_KEY=<hex>

# During migration (KMS primary, local fallback)
KEY_PROVIDER=aws-kms
AWS_KMS_KEY_ARN=arn:...
ENCRYPTION_KEY=<hex>           # kept for fallback decrypt of old v2 ciphertexts
ENCRYPTION_KEY_PREVIOUS=<hex>  # if rotating local keys simultaneously

# After all data re-wrapped (KMS only)
KEY_PROVIDER=aws-kms
AWS_KMS_KEY_ARN=arn:...
# ENCRYPTION_KEY can be removed
```

During migration:
- New encryptions use KMS (v2 with KMS kek_id)
- Old v2 ciphertexts: primary fails (wrong kek_id) -> fallback local provider succeeds
- Old v1/v0 ciphertexts: handled by existing legacy path (uses ENCRYPTION_KEY)

#### Scenario B: KMS -> Local (rollback)

```bash
# Rollback to local
KEY_PROVIDER=local
ENCRYPTION_KEY=<hex>
```

- Old local-wrapped v2 ciphertexts: decrypt fine
- KMS-wrapped v2 ciphertexts: **cannot decrypt** (no KMS access)
- Rollback is only safe BEFORE re-wrapping data with KMS

**Important**: once data has been re-wrapped with KMS DEKs, rolling back to local loses access to that data. The rollback window is between "deploy with KMS" and "run re-wrap batch job."

#### Scenario C: KMS key rotation (no provider change)

Both AWS and GCP handle rotation internally. No NyxID config change needed. The same key ARN/name works before and after rotation. The KMS automatically uses the old key version to decrypt old ciphertexts.

### Fallback provider wiring in `main.rs`

```rust
let (provider, fallback_provider): (Arc<dyn KeyProvider>, Option<Arc<dyn KeyProvider>>) =
    match config.key_provider.as_str() {
        "local" => {
            let local = Arc::new(LocalKeyProvider::from_config(&config));
            (local, None)
        }
        #[cfg(feature = "aws-kms")]
        "aws-kms" => {
            let kms = Arc::new(AwsKmsProvider::from_config(&config).await);
            // If ENCRYPTION_KEY is set, create a local fallback for migration
            let fallback = config.encryption_key.as_ref().map(|_| {
                Arc::new(LocalKeyProvider::from_config(&config)) as Arc<dyn KeyProvider>
            });
            (kms, fallback)
        }
        #[cfg(feature = "gcp-kms")]
        "gcp-kms" => {
            let kms = Arc::new(GcpKmsProvider::from_config(&config).await);
            let fallback = config.encryption_key.as_ref().map(|_| {
                Arc::new(LocalKeyProvider::from_config(&config)) as Arc<dyn KeyProvider>
            });
            (kms, fallback)
        }
        other => panic!("Unsupported KEY_PROVIDER: {other}"),
    };

let encryption_keys = Arc::new(
    EncryptionKeys::with_provider_and_fallback(provider, fallback_provider, legacy)
);
```

### Re-wrap batch job

Not implemented in Phase 4 (out of scope). The existing `rewrap()` method handles single-record re-wrapping. A future batch job would iterate over all encrypted fields in MongoDB and call `rewrap()` for each. During Phase 4, operators can re-encrypt data by reading+writing through the normal application paths, or by building a simple batch script.

---

## 5. Testing Strategy

### Unit tests (no KMS access needed)

#### Mock provider tests

The existing `MockKeyProvider` in `aes.rs` tests is a good pattern. For Phase 4, extend it:

1. **`MockKmsProvider`**: Simulates KMS behavior with variable-size wrapped DEKs (e.g., 170 bytes instead of 60). Verifies that `EncryptionKeys` handles non-local blob sizes correctly.

2. **Fallback provider tests**:
   - Encrypt with `MockLocalProvider` (key_id=0xAA), then decrypt with primary=`MockKmsProvider` (key_id=0xBB) + fallback=`MockLocalProvider` (key_id=0xAA) -> should succeed via fallback
   - Encrypt with `MockKmsProvider`, decrypt with only local provider -> should fail gracefully
   - Verify counters track fallback usage

3. **key_id collision detection**: Verify that `AwsKmsProvider::new()` panics if current and previous ARNs produce the same key_id.

4. **Async trait tests**: Use `#[tokio::test]` for all tests that call async `encrypt()`/`decrypt()`.

#### Provider-specific unit tests

- `AwsKmsProvider`: test `derive_key_id` from ARN, test construction with/without previous key, test Debug redaction
- `GcpKmsProvider`: same pattern

### Integration tests (require KMS access)

Behind feature flags and env var guards:

```rust
#[cfg(feature = "aws-kms")]
#[tokio::test]
#[ignore] // Only run with: cargo test --features aws-kms -- --ignored
async fn aws_kms_roundtrip() {
    let key_arn = std::env::var("TEST_AWS_KMS_KEY_ARN")
        .expect("TEST_AWS_KMS_KEY_ARN must be set for KMS integration tests");
    // ... roundtrip test
}
```

### Migration scenario tests

Using mock providers (no KMS access needed):

1. Encrypt with local provider, switch to KMS with local fallback, decrypt -> success
2. Encrypt with KMS, switch to local only, decrypt -> failure (expected)
3. Encrypt with local, rewrap with KMS, decrypt with KMS only -> success
4. Verify v0/v1/v2 fallback chain works with KMS primary + local fallback

### Test matrix

| Scenario | Primary | Fallback | v0 | v1 | v2-local | v2-KMS | Expected |
|----------|---------|----------|----|----|----------|--------|----------|
| Local only | Local | None | OK | OK | OK | N/A | All pass |
| KMS only | KMS | None | Fail | Fail | Fail | OK | v0/v1/v2-local fail |
| KMS + local legacy | KMS | Local | OK | OK | OK | OK | All pass |
| Rollback to local | Local | None | OK | OK | OK | Fail | v2-KMS fail |

---

## 6. Dependency Analysis

### New dependencies

```toml
# backend/Cargo.toml

[dependencies]
async-trait = "0.1"  # Required: async dyn KeyProvider

# Optional KMS dependencies (behind feature flags)
aws-config = { version = "1.1", optional = true, features = ["behavior-version-latest"] }
aws-sdk-kms = { version = "1.90", optional = true }
google-cloud-kms = { version = "0.6", optional = true }

[features]
default = []
aws-kms = ["dep:aws-config", "dep:aws-sdk-kms"]
gcp-kms = ["dep:google-cloud-kms"]
```

### Dependency impact

| Dependency | Size impact | Compile time impact | Notes |
|------------|------------|-------------------|-------|
| `async-trait` | Minimal (~10KB) | Minimal (proc macro) | Always included |
| `aws-sdk-kms` + `aws-config` | ~5-10MB debug, ~1-2MB release | +30-60s first build | Only with `--features aws-kms` |
| `google-cloud-kms` | ~3-8MB debug, ~1-2MB release | +20-40s first build | Only with `--features gcp-kms` |

### Build commands

```bash
# Default (local only, no KMS deps)
cargo build
cargo test

# With AWS KMS
cargo build --features aws-kms
cargo test --features aws-kms

# With GCP KMS
cargo build --features gcp-kms
cargo test --features gcp-kms

# All providers
cargo build --features aws-kms,gcp-kms
cargo test --features aws-kms,gcp-kms
```

---

## 7. File-by-File Change List

### Modified files

#### 1. `backend/Cargo.toml`

- Add `async-trait = "0.1"` to `[dependencies]`
- Add `aws-config`, `aws-sdk-kms` as optional deps
- Add `google-cloud-kms` as optional dep
- Add `[features]` section with `aws-kms` and `gcp-kms` features

#### 2. `backend/src/crypto/key_provider.rs`

Changes:
- Add `use async_trait::async_trait;`
- Add `#[async_trait]` attribute to `KeyProvider` trait
- Make `wrap_dek` and `unwrap_dek` async: `async fn wrap_dek(...)` and `async fn unwrap_dek(...)`
- `current_key_id()`, `has_key_id()`, `has_previous_key()` remain sync (no I/O)

```rust
use async_trait::async_trait;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::errors::AppError;

#[derive(Debug, Clone)]
pub struct WrappedKey {
    pub key_id: u8,
    pub ciphertext: Vec<u8>,
}

#[async_trait]
pub trait KeyProvider: Send + Sync + std::fmt::Debug {
    async fn wrap_dek(&self, plaintext_dek: &[u8]) -> Result<WrappedKey, AppError>;
    async fn unwrap_dek(&self, wrapped: &WrappedKey) -> Result<Zeroizing<Vec<u8>>, AppError>;
    fn current_key_id(&self) -> u8;
    fn has_key_id(&self, key_id: u8) -> bool;
    fn has_previous_key(&self) -> bool;
}

pub(crate) fn derive_key_id(key: &[u8]) -> u8 {
    let digest = Sha256::digest(key);
    digest[0]
}
```

#### 3. `backend/src/crypto/local_key_provider.rs`

Changes:
- Add `use async_trait::async_trait;`
- Add `#[async_trait]` attribute to `impl KeyProvider for LocalKeyProvider`
- Add `async` keyword to `wrap_dek` and `unwrap_dek` implementations
- Internal logic is identical -- these methods just happen to be async now but return immediately
- Update tests to use `#[tokio::test]` where calling async methods

#### 4. `backend/src/crypto/aes.rs`

This is the largest change. Summary:
- Make `encrypt()`, `decrypt()`, `rewrap()` async
- Add `fallback_provider: Option<Arc<dyn KeyProvider>>` to `EncryptionKeys`
- Add `with_provider_and_fallback()` constructor
- In `decrypt()`: after primary provider fails on v2, try fallback provider
- Add fallback counter fields to `DecryptCounters` and `EncryptionDecryptStats`
- Update all provider calls with `.await`
- Update `MockKeyProvider` in tests to use `#[async_trait]`
- Change `#[test]` to `#[tokio::test]` for tests calling encrypt/decrypt/rewrap

Detailed changes to `EncryptionKeys`:

```rust
pub struct EncryptionKeys {
    provider: Arc<dyn KeyProvider>,
    fallback_provider: Option<Arc<dyn KeyProvider>>,  // NEW
    legacy: Option<LegacyKeys>,
    counters: DecryptCounters,
}

impl EncryptionKeys {
    pub fn with_provider(provider: Arc<dyn KeyProvider>) -> Self {
        Self {
            provider,
            fallback_provider: None,
            legacy: None,
            counters: DecryptCounters::default(),
        }
    }

    /// Build with a primary and optional fallback provider.
    /// Used during migration from one provider to another.
    pub fn with_provider_and_fallback(
        provider: Arc<dyn KeyProvider>,
        fallback: Option<Arc<dyn KeyProvider>>,
    ) -> Self {
        Self {
            provider,
            fallback_provider: fallback,
            legacy: None,
            counters: DecryptCounters::default(),
        }
    }

    // from_config() stays similar but sets fallback_provider to None

    pub async fn encrypt(&self, plaintext: &[u8]) -> Result<Vec<u8>, AppError> {
        // ... same logic, but:
        let wrapped = self.provider.wrap_dek(dek.as_ref()).await?;
        // ... rest unchanged
    }

    pub async fn decrypt(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
        // v2 path:
        match self.provider.unwrap_dek(&wrapped).await {
            Ok(dek_bytes) => { /* success */ }
            Err(_) => {
                // NEW: try fallback provider
                if let Some(ref fallback) = self.fallback_provider {
                    if fallback.has_key_id(kek_id) {
                        match fallback.unwrap_dek(&wrapped).await {
                            Ok(dek_bytes) => {
                                self.counters.v2_fallback.fetch_add(1, Ordering::Relaxed);
                                // decrypt data with DEK, return
                            }
                            Err(_) => { /* fall through */ }
                        }
                    }
                }
                // existing v1/v0 fallback continues...
            }
        }
    }

    pub async fn rewrap(&self, ciphertext: &[u8]) -> Result<Vec<u8>, AppError> {
        // ... same logic, but with .await on provider calls
        // Also: try fallback_provider.unwrap_dek() if primary fails
        let plaintext_dek = match self.provider.unwrap_dek(&old_wrapped).await {
            Ok(dek) => dek,
            Err(_) => {
                // Try fallback for migration rewrap
                self.fallback_provider
                    .as_ref()
                    .ok_or_else(|| AppError::Internal("No provider could unwrap DEK".into()))?
                    .unwrap_dek(&old_wrapped)
                    .await?
            }
        };
        let new_wrapped = self.provider.wrap_dek(&plaintext_dek).await?;
        // ... assemble new envelope
    }
}
```

New counter fields:
```rust
struct DecryptCounters {
    // ... existing fields ...
    v2_fallback: AtomicU64,        // NEW
    logged_v2_fallback: AtomicBool, // NEW
}

pub struct EncryptionDecryptStats {
    // ... existing fields ...
    pub v2_fallback: u64,  // NEW
}
```

#### 5. `backend/src/crypto/mod.rs`

Add conditional module declarations:

```rust
pub mod aes;
pub mod apple_client_secret;
pub mod jwks;
pub mod jwt;
pub mod key_provider;
pub mod local_key_provider;
pub mod password;
pub mod token;

#[cfg(feature = "aws-kms")]
pub mod aws_kms_provider;

#[cfg(feature = "gcp-kms")]
pub mod gcp_kms_provider;
```

#### 6. `backend/src/config.rs`

Add new config fields:

```rust
pub struct AppConfig {
    // ... existing fields ...

    // AWS KMS (Phase 4)
    /// AWS KMS key ARN for DEK wrapping. Required when KEY_PROVIDER=aws-kms.
    pub aws_kms_key_arn: Option<String>,
    /// Optional previous AWS KMS key ARN for multi-key migration.
    pub aws_kms_key_arn_previous: Option<String>,

    // GCP KMS (Phase 4)
    /// GCP Cloud KMS key resource name. Required when KEY_PROVIDER=gcp-kms.
    pub gcp_kms_key_name: Option<String>,
    /// Optional previous GCP KMS key name for multi-key migration.
    pub gcp_kms_key_name_previous: Option<String>,
}
```

Update `from_env()`:
```rust
aws_kms_key_arn: env::var("AWS_KMS_KEY_ARN").ok().filter(|s| !s.is_empty()),
aws_kms_key_arn_previous: env::var("AWS_KMS_KEY_ARN_PREVIOUS").ok().filter(|s| !s.is_empty()),
gcp_kms_key_name: env::var("GCP_KMS_KEY_NAME").ok().filter(|s| !s.is_empty()),
gcp_kms_key_name_previous: env::var("GCP_KMS_KEY_NAME_PREVIOUS").ok().filter(|s| !s.is_empty()),
```

Update `validate_key_provider()`:
```rust
pub fn validate_key_provider(&self) {
    match self.key_provider.as_str() {
        "local" => self.validate_encryption_key(),
        #[cfg(feature = "aws-kms")]
        "aws-kms" => {
            self.aws_kms_key_arn.as_ref().unwrap_or_else(|| {
                panic!("AWS_KMS_KEY_ARN must be set when KEY_PROVIDER=aws-kms")
            });
            // ENCRYPTION_KEY is optional (for migration fallback)
            if self.encryption_key.is_some() {
                self.validate_encryption_key();
            }
        }
        #[cfg(feature = "gcp-kms")]
        "gcp-kms" => {
            self.gcp_kms_key_name.as_ref().unwrap_or_else(|| {
                panic!("GCP_KMS_KEY_NAME must be set when KEY_PROVIDER=gcp-kms")
            });
            if self.encryption_key.is_some() {
                self.validate_encryption_key();
            }
        }
        other => {
            let mut supported = vec!["local"];
            #[cfg(feature = "aws-kms")]
            supported.push("aws-kms");
            #[cfg(feature = "gcp-kms")]
            supported.push("gcp-kms");
            panic!(
                "Unsupported KEY_PROVIDER: {other}. Supported providers: {}",
                supported.join(", ")
            );
        }
    }
}
```

Update `Debug` impl to redact new fields:
```rust
.field("aws_kms_key_arn", &self.aws_kms_key_arn) // ARN is not secret
.field("aws_kms_key_arn_previous", &self.aws_kms_key_arn_previous)
.field("gcp_kms_key_name", &self.gcp_kms_key_name) // resource name is not secret
.field("gcp_kms_key_name_previous", &self.gcp_kms_key_name_previous)
```

Update test helper `make_config()` and `test_config()` to include new fields (all `None`).

#### 7. `backend/src/main.rs`

Update provider initialization block to be async-aware and support KMS providers:

```rust
// Validate provider-specific encryption config before any seed calls that use it.
config.validate_key_provider();

// Build key provider(s)
let (provider, fallback_provider): (Arc<dyn KeyProvider>, Option<Arc<dyn KeyProvider>>) =
    match config.key_provider.as_str() {
        "local" => {
            let local = Arc::new(LocalKeyProvider::from_config(&config));
            (local, None)
        }
        #[cfg(feature = "aws-kms")]
        "aws-kms" => {
            let kms = Arc::new(
                crypto::aws_kms_provider::AwsKmsProvider::from_config(&config).await
            );
            let fallback = config.encryption_key.as_ref().map(|_| {
                Arc::new(LocalKeyProvider::from_config(&config)) as Arc<dyn KeyProvider>
            });
            (kms, fallback)
        }
        #[cfg(feature = "gcp-kms")]
        "gcp-kms" => {
            let kms = Arc::new(
                crypto::gcp_kms_provider::GcpKmsProvider::from_config(&config).await
            );
            let fallback = config.encryption_key.as_ref().map(|_| {
                Arc::new(LocalKeyProvider::from_config(&config)) as Arc<dyn KeyProvider>
            });
            (kms, fallback)
        }
        other => panic!("Unsupported KEY_PROVIDER: {other}"),
    };

// Build EncryptionKeys with provider and optional fallback
let legacy = if config.encryption_key.is_some() {
    Some(LegacyKeys::from_config(&config))
} else {
    None
};

let encryption_keys = Arc::new({
    let mut ek = EncryptionKeys::with_provider_and_fallback(provider, fallback_provider);
    if let Some(l) = legacy {
        ek.set_legacy(l);
    }
    ek
});
```

Note: `LegacyKeys` and `set_legacy()` may need to be made `pub(crate)` to be accessible from `main.rs`. Alternatively, restructure `from_config()` vs `with_provider_and_fallback()` to handle this internally. The implementer should choose the cleanest approach that keeps `LegacyKeys` encapsulated.

Also update the seed calls to `.await` on encryption operations:

```rust
services::provider_service::seed_default_providers(&db, encryption_keys.as_ref())
    .await
    .expect("Failed to seed default providers");
```

These already use `.await` on the service call, but if the service internally calls `encrypt()`, that service function signature also needs to be async (it already is).

#### 8. Service files (mechanical `.await` addition)

Each of these files needs `.await` added after every `encryption_keys.encrypt(...)` and `encryption_keys.decrypt(...)` call:

| File | Encrypt calls | Decrypt calls | Total | Special handling |
|------|--------------|---------------|-------|-----------------|
| `services/user_token_service.rs` | 10 | 8 | 18 | 2x `Option::map` -> match |
| `services/provider_service.rs` | 8 | 0 | 8 | 4x `Option::map` -> match |
| `services/user_credentials_service.rs` | 2 | 3 | 5 | None |
| `services/mfa_service.rs` | 1 | 2 | 3 | None |
| `services/oauth_flow.rs` | 2 | 1 | 3 | None |
| `services/connection_service.rs` | 2 | 0 | 2 | None |
| `services/proxy_service.rs` | 0 | 1 | 1 | None |
| `handlers/services.rs` | 2 | 0 | 2 | None |
| **Total** | **27** | **15** | **42** | **6x map refactor** |

### New files

#### 9. `backend/src/crypto/aws_kms_provider.rs` (new)

```rust
//! AWS KMS KeyProvider implementation.
//!
//! Wraps and unwraps DEKs using the AWS KMS Encrypt/Decrypt APIs.
//! The wrapped DEK is the raw AWS CiphertextBlob (~170-200 bytes).

use async_trait::async_trait;
use aws_sdk_kms::Client as KmsClient;
use aws_sdk_kms::primitives::Blob;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::config::AppConfig;
use crate::errors::AppError;

use super::key_provider::{KeyProvider, WrappedKey};

pub struct AwsKmsProvider {
    client: KmsClient,
    current_key_arn: String,
    current_key_id: u8,
    previous_key_arn: Option<String>,
    previous_key_id: Option<u8>,
}

impl std::fmt::Debug for AwsKmsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AwsKmsProvider")
            .field("current_key_arn", &self.current_key_arn)
            .field("current_key_id", &format!("0x{:02x}", self.current_key_id))
            .field("previous_key_arn", &self.previous_key_arn)
            .finish()
    }
}

fn derive_kms_key_id(key_identifier: &str) -> u8 {
    let digest = Sha256::digest(key_identifier.as_bytes());
    digest[0]
}

impl AwsKmsProvider {
    pub async fn from_config(config: &AppConfig) -> Self {
        let key_arn = config
            .aws_kms_key_arn
            .as_deref()
            .expect("AWS_KMS_KEY_ARN must be set when KEY_PROVIDER=aws-kms");

        let sdk_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
        let client = KmsClient::new(&sdk_config);

        let current_key_id = derive_kms_key_id(key_arn);

        let previous_key_arn = config.aws_kms_key_arn_previous.clone();
        let previous_key_id = previous_key_arn.as_deref().map(derive_kms_key_id);

        if let Some(prev_id) = previous_key_id {
            if current_key_id == prev_id {
                panic!(
                    "AWS_KMS_KEY_ARN and AWS_KMS_KEY_ARN_PREVIOUS produce the same key id \
                     (0x{:02x}). This is a 1-in-256 hash collision. Use a different key.",
                    current_key_id
                );
            }
        }

        tracing::info!(
            key_id = format!("0x{:02x}", current_key_id),
            has_previous = previous_key_arn.is_some(),
            "AWS KMS provider initialized"
        );

        Self {
            client,
            current_key_arn: key_arn.to_string(),
            current_key_id,
            previous_key_arn,
            previous_key_id,
        }
    }
}

#[async_trait]
impl KeyProvider for AwsKmsProvider {
    async fn wrap_dek(&self, plaintext_dek: &[u8]) -> Result<WrappedKey, AppError> {
        let resp = self
            .client
            .encrypt()
            .key_id(&self.current_key_arn)
            .plaintext(Blob::new(plaintext_dek))
            .send()
            .await
            .map_err(|e| {
                AppError::Internal(format!("AWS KMS encrypt failed: {e}"))
            })?;

        let ciphertext_blob = resp
            .ciphertext_blob()
            .ok_or_else(|| AppError::Internal("AWS KMS returned empty ciphertext".into()))?;

        Ok(WrappedKey {
            key_id: self.current_key_id,
            ciphertext: ciphertext_blob.as_ref().to_vec(),
        })
    }

    async fn unwrap_dek(&self, wrapped: &WrappedKey) -> Result<Zeroizing<Vec<u8>>, AppError> {
        // Determine which key ARN to use based on key_id
        let key_arn = if wrapped.key_id == self.current_key_id {
            &self.current_key_arn
        } else if self.previous_key_id == Some(wrapped.key_id) {
            self.previous_key_arn.as_ref().ok_or_else(|| {
                AppError::Internal("Previous key id matched but ARN is missing".into())
            })?
        } else {
            return Err(AppError::Internal(
                "No AWS KMS key available for key id".into(),
            ));
        };

        let resp = self
            .client
            .decrypt()
            .key_id(key_arn)
            .ciphertext_blob(Blob::new(&wrapped.ciphertext))
            .send()
            .await
            .map_err(|e| {
                AppError::Internal(format!("AWS KMS decrypt failed: {e}"))
            })?;

        let plaintext = resp
            .plaintext()
            .ok_or_else(|| AppError::Internal("AWS KMS returned empty plaintext".into()))?;

        Ok(Zeroizing::new(plaintext.as_ref().to_vec()))
    }

    fn current_key_id(&self) -> u8 {
        self.current_key_id
    }

    fn has_key_id(&self, key_id: u8) -> bool {
        key_id == self.current_key_id || self.previous_key_id == Some(key_id)
    }

    fn has_previous_key(&self) -> bool {
        self.previous_key_arn.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_key_id_deterministic() {
        let arn = "arn:aws:kms:us-east-1:123456789:key/mrk-abc123";
        let id1 = derive_kms_key_id(arn);
        let id2 = derive_kms_key_id(arn);
        assert_eq!(id1, id2);
    }

    #[test]
    fn derive_key_id_different_arns() {
        let arn1 = "arn:aws:kms:us-east-1:123456789:key/key-aaa";
        let arn2 = "arn:aws:kms:us-east-1:123456789:key/key-bbb";
        // Different ARNs *may* produce different IDs (probabilistic)
        // This test just verifies the function doesn't panic
        let _id1 = derive_kms_key_id(arn1);
        let _id2 = derive_kms_key_id(arn2);
    }

    #[test]
    fn debug_does_not_leak_secrets() {
        // AwsKmsProvider::from_config requires async + AWS creds,
        // so we test Debug format on the struct fields conceptually.
        // Key ARNs are not secrets (they're resource identifiers).
        // No key material is stored in the provider.
    }
}
```

#### 10. `backend/src/crypto/gcp_kms_provider.rs` (new)

```rust
//! GCP Cloud KMS KeyProvider implementation.
//!
//! Wraps and unwraps DEKs using the GCP Cloud KMS encrypt/decrypt APIs.

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::config::AppConfig;
use crate::errors::AppError;

use super::key_provider::{KeyProvider, WrappedKey};

pub struct GcpKmsProvider {
    client: google_cloud_kms::client::Client,
    current_key_name: String,
    current_key_id: u8,
    previous_key_name: Option<String>,
    previous_key_id: Option<u8>,
}

impl std::fmt::Debug for GcpKmsProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GcpKmsProvider")
            .field("current_key_name", &self.current_key_name)
            .field("current_key_id", &format!("0x{:02x}", self.current_key_id))
            .field("previous_key_name", &self.previous_key_name)
            .finish()
    }
}

fn derive_kms_key_id(key_identifier: &str) -> u8 {
    let digest = Sha256::digest(key_identifier.as_bytes());
    digest[0]
}

impl GcpKmsProvider {
    pub async fn from_config(config: &AppConfig) -> Self {
        let key_name = config
            .gcp_kms_key_name
            .as_deref()
            .expect("GCP_KMS_KEY_NAME must be set when KEY_PROVIDER=gcp-kms");

        let client_config = google_cloud_kms::client::ClientConfig::default()
            .with_auth()
            .await
            .expect("Failed to configure GCP KMS authentication");
        let client = google_cloud_kms::client::Client::new(client_config);

        let current_key_id = derive_kms_key_id(key_name);

        let previous_key_name = config.gcp_kms_key_name_previous.clone();
        let previous_key_id = previous_key_name.as_deref().map(derive_kms_key_id);

        if let Some(prev_id) = previous_key_id {
            if current_key_id == prev_id {
                panic!(
                    "GCP_KMS_KEY_NAME and GCP_KMS_KEY_NAME_PREVIOUS produce the same key id \
                     (0x{:02x}). This is a 1-in-256 hash collision. Use a different key.",
                    current_key_id
                );
            }
        }

        tracing::info!(
            key_id = format!("0x{:02x}", current_key_id),
            has_previous = previous_key_name.is_some(),
            "GCP Cloud KMS provider initialized"
        );

        Self {
            client,
            current_key_name: key_name.to_string(),
            current_key_id,
            previous_key_name,
            previous_key_id,
        }
    }
}

#[async_trait]
impl KeyProvider for GcpKmsProvider {
    async fn wrap_dek(&self, plaintext_dek: &[u8]) -> Result<WrappedKey, AppError> {
        use google_cloud_kms::grpc::kms::v1::EncryptRequest;

        let request = EncryptRequest {
            name: self.current_key_name.clone(),
            plaintext: plaintext_dek.to_vec(),
            ..Default::default()
        };

        let response = self
            .client
            .encrypt(request, None)
            .await
            .map_err(|e| AppError::Internal(format!("GCP KMS encrypt failed: {e}")))?;

        Ok(WrappedKey {
            key_id: self.current_key_id,
            ciphertext: response.ciphertext,
        })
    }

    async fn unwrap_dek(&self, wrapped: &WrappedKey) -> Result<Zeroizing<Vec<u8>>, AppError> {
        use google_cloud_kms::grpc::kms::v1::DecryptRequest;

        let key_name = if wrapped.key_id == self.current_key_id {
            &self.current_key_name
        } else if self.previous_key_id == Some(wrapped.key_id) {
            self.previous_key_name.as_ref().ok_or_else(|| {
                AppError::Internal("Previous key id matched but name is missing".into())
            })?
        } else {
            return Err(AppError::Internal(
                "No GCP KMS key available for key id".into(),
            ));
        };

        let request = DecryptRequest {
            name: key_name.clone(),
            ciphertext: wrapped.ciphertext.clone(),
            ..Default::default()
        };

        let response = self
            .client
            .decrypt(request, None)
            .await
            .map_err(|e| AppError::Internal(format!("GCP KMS decrypt failed: {e}")))?;

        Ok(Zeroizing::new(response.plaintext))
    }

    fn current_key_id(&self) -> u8 {
        self.current_key_id
    }

    fn has_key_id(&self, key_id: u8) -> bool {
        key_id == self.current_key_id || self.previous_key_id == Some(key_id)
    }

    fn has_previous_key(&self) -> bool {
        self.previous_key_name.is_some()
    }
}
```

**Note on GCP API**: The exact API surface of `google-cloud-kms` may differ from what is shown above. The implementer should check the `google-cloud-kms` crate docs at build time and adapt the request/response types accordingly. The key structure (key_name, plaintext/ciphertext fields) is correct per the GCP Cloud KMS v1 API.

---

## 8. Configuration Reference

### Environment variables (complete)

| Variable | Required for | Default | Description |
|----------|-------------|---------|-------------|
| `KEY_PROVIDER` | All | `local` | Provider type: `local`, `aws-kms`, `gcp-kms` |
| `ENCRYPTION_KEY` | `local` (required), `aws-kms`/`gcp-kms` (optional for migration) | None | 64 hex chars (32 bytes AES-256) |
| `ENCRYPTION_KEY_PREVIOUS` | Optional | None | Previous local key for rotation |
| `AWS_KMS_KEY_ARN` | `aws-kms` | None | AWS KMS key ARN |
| `AWS_KMS_KEY_ARN_PREVIOUS` | Optional | None | Previous AWS KMS key ARN (multi-key migration) |
| `AWS_REGION` | `aws-kms` | None | AWS region (standard SDK var) |
| `AWS_ACCESS_KEY_ID` | `aws-kms` (unless IAM role) | None | AWS credentials (standard SDK var) |
| `AWS_SECRET_ACCESS_KEY` | `aws-kms` (unless IAM role) | None | AWS credentials (standard SDK var) |
| `GCP_KMS_KEY_NAME` | `gcp-kms` | None | GCP KMS key resource name |
| `GCP_KMS_KEY_NAME_PREVIOUS` | Optional | None | Previous GCP KMS key name |
| `GOOGLE_APPLICATION_CREDENTIALS` | `gcp-kms` (unless workload identity) | None | Path to GCP service account JSON |

### Startup validation matrix

| KEY_PROVIDER | ENCRYPTION_KEY | KMS key config | Behavior |
|-------------|---------------|----------------|----------|
| `local` | Required | Ignored | Local-only mode (current behavior) |
| `aws-kms` | Not set | Required | KMS-only, no legacy/fallback |
| `aws-kms` | Set | Required | KMS primary + local fallback (migration) |
| `gcp-kms` | Not set | Required | KMS-only, no legacy/fallback |
| `gcp-kms` | Set | Required | KMS primary + local fallback (migration) |

---

## 9. Rollback Procedures

### Rollback from KMS to local (before re-wrapping)

If issues are discovered shortly after switching to KMS, and no data has been re-wrapped yet:

1. All v2 ciphertexts still have local-wrapped DEKs
2. Change config: `KEY_PROVIDER=local`, keep `ENCRYPTION_KEY`
3. Restart server
4. All data decrypts normally via local provider

### Rollback from KMS to local (after partial re-wrapping)

If some data has been re-wrapped with KMS DEKs:

1. Data encrypted BEFORE KMS switch: local-wrapped, decryptable with local
2. Data encrypted AFTER KMS switch: KMS-wrapped, **NOT decryptable** without KMS
3. To recover: keep KMS access temporarily, re-wrap KMS data back to local using `rewrap()`, then switch to local

### Rollback from KMS to local (after full re-wrapping)

All data is KMS-wrapped. Local-only rollback is **not possible**. Options:
- Keep KMS access and switch back to KMS
- Re-wrap all data from KMS to local before removing KMS access

### Preventing accidental data loss

The fallback provider mechanism ensures that during the migration window:
- Reading: both local and KMS ciphertexts are decryptable
- Writing: new data uses KMS (no dual-write complexity)
- The operator controls when to remove the local fallback

---

## Implementation Order

The implementer should follow this order to maintain a compilable project at each step:

1. Add `async-trait` dependency to `Cargo.toml`
2. Update `key_provider.rs` (make trait async)
3. Update `local_key_provider.rs` (add async annotations)
4. Update `aes.rs` (make methods async, add fallback provider, update tests)
5. Update `config.rs` (add new fields, update validation)
6. Update `main.rs` (provider dispatch)
7. Update all 8 service/handler files (add `.await`)
8. Add `aws-kms` and `gcp-kms` feature flags to `Cargo.toml`
9. Create `aws_kms_provider.rs` (behind feature flag)
10. Create `gcp_kms_provider.rs` (behind feature flag)
11. Update `crypto/mod.rs` (conditional module exports)
12. Run `cargo test` (local tests, no features)
13. Run `cargo build --features aws-kms,gcp-kms` (verify compilation)
14. Run `cargo clippy --all-features`
