# Cloud KMS Operations Guide

Operational guide for running NyxID with AWS KMS or GCP Cloud KMS as the encryption backend. Covers monitoring, key rotation, troubleshooting, and IAM setup.

---

## Table of Contents

- [Configuration Reference](#configuration-reference)
- [Feature Flags and Build](#feature-flags-and-build)
- [IAM Setup](#iam-setup)
- [Monitoring](#monitoring)
- [Key Rotation](#key-rotation)
- [Troubleshooting](#troubleshooting)
- [Security Considerations](#security-considerations)
- [Performance](#performance)

---

## Configuration Reference

### Environment Variables

| Variable | Required When | Description |
|----------|--------------|-------------|
| `KEY_PROVIDER` | Always | Provider backend: `local` (default), `aws-kms`, `gcp-kms` |
| `ENCRYPTION_KEY` | `KEY_PROVIDER=local` or migration fallback | 64 hex chars (32 bytes AES-256) |
| `ENCRYPTION_KEY_PREVIOUS` | Local key rotation | Previous local key (64 hex chars) |
| `AWS_KMS_KEY_ARN` | `KEY_PROVIDER=aws-kms` | Full ARN of the AWS KMS key |
| `AWS_KMS_KEY_ARN_PREVIOUS` | AWS KMS key rotation | Previous AWS KMS key ARN |
| `GCP_KMS_KEY_NAME` | `KEY_PROVIDER=gcp-kms` | Full GCP KMS key resource name |
| `GCP_KMS_KEY_NAME_PREVIOUS` | GCP KMS key rotation | Previous GCP KMS key name |

### AWS KMS Key ARN Format

```
arn:aws:kms:<region>:<account-id>:key/<key-id>
```

Example: `arn:aws:kms:us-east-1:123456789012:key/mrk-abc123def456`

### GCP KMS Key Resource Name Format

```
projects/<project-id>/locations/<location>/keyRings/<ring>/cryptoKeys/<key>
```

Example: `projects/my-project/locations/us-east1/keyRings/nyxid-ring/cryptoKeys/nyxid-kek`

### Example Configurations

**Local (default):**
```bash
KEY_PROVIDER=local
ENCRYPTION_KEY=abcdef0123456789...  # 64 hex chars
```

**AWS KMS (fresh deployment):**
```bash
KEY_PROVIDER=aws-kms
AWS_KMS_KEY_ARN=arn:aws:kms:us-east-1:123456789012:key/mrk-abc123
```

**AWS KMS (migration from local):**
```bash
KEY_PROVIDER=aws-kms
AWS_KMS_KEY_ARN=arn:aws:kms:us-east-1:123456789012:key/mrk-abc123
ENCRYPTION_KEY=abcdef0123456789...  # kept for fallback during migration
```

**GCP KMS (fresh deployment):**
```bash
KEY_PROVIDER=gcp-kms
GCP_KMS_KEY_NAME=projects/my-project/locations/us-east1/keyRings/nyxid-ring/cryptoKeys/nyxid-kek
```

---

## Feature Flags and Build

KMS provider dependencies are behind Cargo feature flags to keep the default binary lean.

| Feature | Dependencies Added | Binary Size Impact |
|---------|-------------------|-------------------|
| (default) | None | Baseline |
| `aws-kms` | `aws-config`, `aws-sdk-kms` | ~5-10 MB |
| `gcp-kms` | `google-cloud-kms` | ~5-10 MB |

```bash
# Build with AWS KMS only
cargo build --release --features aws-kms

# Build with GCP KMS only
cargo build --release --features gcp-kms

# Build with both
cargo build --release --features aws-kms,gcp-kms

# Default (local only)
cargo build --release
```

The `async-trait` crate is always included (required by the `KeyProvider` trait regardless of provider).

---

## IAM Setup

### AWS KMS

**Minimum permissions** (recommended for production):

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Sid": "NyxIDKMSAccess",
      "Effect": "Allow",
      "Action": [
        "kms:Encrypt",
        "kms:Decrypt"
      ],
      "Resource": "arn:aws:kms:us-east-1:123456789012:key/mrk-abc123"
    }
  ]
}
```

**Credential chain** (in order of precedence):
1. `AWS_ACCESS_KEY_ID` / `AWS_SECRET_ACCESS_KEY` environment variables
2. `~/.aws/credentials` profile
3. ECS task role (container deployments)
4. EC2 instance metadata (instance role)

**Do NOT grant**: `kms:CreateKey`, `kms:DeleteKey`, `kms:DisableKey`, `kms:ScheduleKeyDeletion`, `kms:PutKeyPolicy`. These are admin operations that should be restricted to infrastructure teams.

### GCP Cloud KMS

**Minimum role**: `roles/cloudkms.cryptoKeyEncrypterDecrypter`

```bash
gcloud kms keys add-iam-policy-binding KEY_NAME \
  --keyring KEYRING \
  --location LOCATION \
  --member serviceAccount:SERVICE_ACCOUNT_EMAIL \
  --role roles/cloudkms.cryptoKeyEncrypterDecrypter
```

**Credential chain** (Application Default Credentials):
1. `GOOGLE_APPLICATION_CREDENTIALS` environment variable (service account JSON)
2. gcloud CLI credentials (`gcloud auth application-default login`)
3. GCE metadata server (Compute Engine, GKE, Cloud Run)

**Do NOT grant**: `roles/cloudkms.admin`, `roles/cloudkms.cryptoKeyVersionManager`. Restrict key management to infrastructure teams.

---

## Monitoring

### Decrypt Stats

`EncryptionKeys::decrypt_stats()` returns an `EncryptionDecryptStats` struct with atomic counters for each decrypt path:

```rust
pub struct EncryptionDecryptStats {
    pub v2_current: u64,      // Normal v2 decrypt (current KMS key)
    pub v2_previous: u64,     // v2 decrypt with previous KMS key
    pub v2_fallback: u64,     // v2 decrypt via fallback provider (migration)
    pub v1_current: u64,      // Legacy v1 decrypt
    pub v1_previous: u64,     // Legacy v1 with previous key
    pub v0_current: u64,      // Legacy v0 decrypt
    pub v0_previous: u64,     // Legacy v0 with previous key
    pub unknown_key_id_failures: u64,  // Unknown key ID encountered
    pub decrypt_failures: u64,         // Total decrypt failures
}
```

### Key Metrics to Monitor

| Metric | Healthy Value | Alert Condition |
|--------|--------------|-----------------|
| `v2_current` | Increasing | Flat (no encrypt/decrypt traffic) |
| `v2_fallback` | 0 (post-migration) | > 0 after migration complete |
| `v0_current + v0_previous` | 0 | > 0 (legacy records not re-encrypted) |
| `v1_current + v1_previous` | 0 | > 0 (legacy records not re-encrypted) |
| `decrypt_failures` | 0 | > 0 (key mismatch or corruption) |
| `unknown_key_id_failures` | 0 | > 0 (misconfigured key) |

### Startup Log Messages

| Log | Level | Meaning |
|-----|-------|---------|
| `AWS KMS provider initialized` | INFO | AWS KMS provider is active |
| `GCP Cloud KMS provider initialized` | INFO | GCP KMS provider is active |
| `ENCRYPTION_KEY_PREVIOUS is configured...` | WARN | Key rotation in progress |
| `Decrypted v2 envelope via fallback provider...` | WARN | Migration still in progress (once per startup) |
| `Decrypted legacy v0 ciphertext...` | WARN | v0 records exist (re-encryption needed) |

### KMS-Specific Logging

Both KMS providers log operational events:

- **AWS KMS**: SDK errors logged at `ERROR` level; generic message returned in error chain
- **GCP KMS**: Transient failures logged at `WARN` per attempt; final failure at `ERROR`

All log messages redact key ARNs, key names, and key material. The `Debug` impls for both providers show `[REDACTED]` for all key identifiers.

---

## Key Rotation

### In-Provider Rotation (Recommended)

Cloud KMS providers handle key versioning internally:

**AWS KMS** -- Enable automatic annual rotation:
```bash
aws kms enable-key-rotation --key-id arn:aws:kms:...:key/mrk-abc123
```
AWS retains all previous key versions and automatically selects the correct version for decrypt. No NyxID config change needed.

**GCP Cloud KMS** -- Create a new key version and set as primary:
```bash
gcloud kms keys versions create \
  --key nyxid-kek \
  --keyring nyxid-ring \
  --location us-east1

gcloud kms keys update nyxid-kek \
  --keyring nyxid-ring \
  --location us-east1 \
  --primary-version <version>
```
GCP automatically selects the correct version for decrypt.

### Cross-Key Rotation

To rotate to a completely different KMS key:

1. Set the new key as primary and the old key as previous:
   ```bash
   AWS_KMS_KEY_ARN=arn:aws:kms:...:key/new-key
   AWS_KMS_KEY_ARN_PREVIOUS=arn:aws:kms:...:key/old-key
   ```

2. Deploy and verify startup (collision check passes)

3. Run `rewrap()` on all records

4. Monitor `v2_previous` until it reaches 0

5. Remove `_PREVIOUS` config

### Local Key Rotation

If using `KEY_PROVIDER=local`, rotate with:

```bash
ENCRYPTION_KEY=<new 64 hex chars>
ENCRYPTION_KEY_PREVIOUS=<old 64 hex chars>
```

Then run `rewrap()` on all v2 records. When `v2_previous == 0`, remove `ENCRYPTION_KEY_PREVIOUS`.

---

## Troubleshooting

### Startup Failures

**"Unsupported KEY_PROVIDER: aws-kms"**
- Cause: Binary was not built with the `aws-kms` feature flag
- Fix: `cargo build --release --features aws-kms`

**"AWS_KMS_KEY_ARN must be set when KEY_PROVIDER=aws-kms"**
- Cause: Missing `AWS_KMS_KEY_ARN` environment variable
- Fix: Set the env var to the full KMS key ARN

**"GCP_KMS_KEY_NAME must be set when KEY_PROVIDER=gcp-kms"**
- Cause: Missing `GCP_KMS_KEY_NAME` environment variable
- Fix: Set the env var to the full GCP key resource name

**"Primary and fallback providers have colliding key IDs (0xNN)"**
- Cause: The KMS key ARN/name and local `ENCRYPTION_KEY` produce the same SHA-256 first-byte hash (1-in-256 chance)
- Fix: Use a different KMS key, or generate a new local key (`openssl rand -hex 32`)

**"... produce the same key id (0xNN). This is a 1-in-256 hash collision."**
- Cause: Current and previous keys for the same provider produce the same key ID
- Fix: Use a different key for current or previous

### Runtime Failures

**"AWS KMS encrypt failed" / "AWS KMS decrypt failed"**
- Check: IAM permissions (`kms:Encrypt`, `kms:Decrypt`)
- Check: Key is enabled (not disabled or pending deletion)
- Check: AWS region matches the key's region
- Check: Network connectivity to KMS endpoint

**"GCP KMS encrypt failed after 3 attempts" / "GCP KMS decrypt failed after 3 attempts"**
- Check: IAM role (`roles/cloudkms.cryptoKeyEncrypterDecrypter`)
- Check: Key version is enabled
- Check: Application Default Credentials are configured
- Check: Network connectivity to GCP APIs

**"Wrapped DEK exceeds maximum size"**
- Cause: KMS returned a wrapped DEK larger than 1024 bytes
- This should not happen with standard KMS keys. Contact support if persistent.

**"No key could decrypt the data"**
- Cause: The ciphertext was encrypted with a key that is no longer configured
- Check: `unknown_key_id_failures` counter for specific key IDs
- Check: Was the `ENCRYPTION_KEY` removed before migration completed?
- Check: Was a KMS key disabled or deleted?

### GCP KMS Retry Behavior

GCP KMS calls include automatic retry with exponential backoff:
- Maximum attempts: 3
- Initial backoff: 100ms
- Backoff multiplier: 2x (100ms, 200ms, 400ms)
- Transient failures logged at `WARN` per attempt

AWS SDK includes built-in retry (3 attempts with exponential backoff via `BehaviorVersion::latest()`).

---

## Security Considerations

### Key Material in Memory

| Material | Protection | Notes |
|----------|-----------|-------|
| Plaintext DEK | `Zeroizing<[u8; 32]>` | Scrubbed on drop, all code paths |
| Wrapped DEK (in `WrappedKey`) | `Zeroizing<Vec<u8>>` | Defense-in-depth (data is encrypted) |
| KMS key ARN / key name | `[REDACTED]` in Debug | Never appears in logs or error messages |
| Local key (env var) | `Zeroizing<[u8; 32]>` in `LocalKeyProvider` | `[REDACTED]` in Debug |
| SDK-internal DEK copies | Not controlled by NyxID | Accepted limitation (documented in code) |

### Error Message Sanitization

KMS SDK errors are logged via `tracing::error!()` for diagnostics but the `AppError::Internal` returned to callers contains only a generic message (e.g., "AWS KMS encrypt failed"). SDK errors may contain request IDs, endpoint URLs, or key identifiers.

### Debug Output

All provider `Debug` impls redact sensitive identifiers:

```
AwsKmsProvider { current_key_arn: "[REDACTED]", current_key_id: "0xab", previous_key_arn: "None" }
GcpKmsProvider { current_key_name: "[REDACTED]", current_key_id: "0xcd", previous_key_name: "None" }
LocalKeyProvider { current: "[REDACTED]", previous: "None" }
```

### Network Security

Both AWS KMS and GCP Cloud KMS use TLS for all API calls (enforced by their respective SDKs). No additional TLS configuration is needed.

---

## Performance

### Latency

| Operation | Local Provider | AWS KMS | GCP Cloud KMS |
|-----------|---------------|---------|---------------|
| DEK wrap | ~1 us (in-process) | ~5-20 ms (network) | ~5-20 ms (network) |
| DEK unwrap | ~1 us (in-process) | ~5-20 ms (network) | ~5-20 ms (network) |
| Data encrypt (1 KB) | ~1 us | ~1 us (after DEK wrap) | ~1 us (after DEK wrap) |
| Rewrap per record | ~2 us | ~10-40 ms | ~10-40 ms |

KMS providers add network latency for DEK wrap/unwrap operations. Data encryption/decryption is always in-process using the unwrapped DEK.

### Wrapped DEK Sizes

| Provider | Wrapped DEK Size | v2 Overhead |
|----------|-----------------|-------------|
| Local | 60 bytes | 92 bytes |
| AWS KMS | ~170-200 bytes | ~202-232 bytes |
| GCP Cloud KMS | Variable | Variable |

Maximum wrapped DEK size is capped at 1024 bytes (`MAX_WRAPPED_DEK_SIZE`).

### Throughput Considerations

- KMS APIs have rate limits (AWS: ~10,000 req/s per key, GCP: varies by quota)
- For high-throughput workloads, consider local provider with periodic KMS-backed key rotation
- DEK caching is not implemented (each encrypt/decrypt calls the provider)
- Rewrap jobs should be throttled to stay within KMS rate limits
