# Cloud KMS Migration Guide

Guide for migrating NyxID's encryption backend from the local `KeyProvider` to AWS KMS or GCP Cloud KMS. Complete [HMAC preparation](security/KEY_LIFECYCLE.md) before changing encryption sources. Follow the [reader-first encryption cutover and migration API](SECURITY.md#encrypted-record-migration) for contextual records; pending family adapters still block complete readiness.

A provider switch changes new writes. Deploy readers able to read both providers before any new-provider writes, or stop/drain all writers while preparing them. Do not perform a mixed rollout against local-only readers. The shipped KMS configuration provides KMS primary plus optional local fallback, not an arbitrary reversible pair of cloud providers.

---

## Table of Contents

- [Prerequisites](#prerequisites)
- [Migration Overview](#migration-overview)
- [Migrate to AWS KMS](#migrate-to-aws-kms)
- [Migrate to GCP Cloud KMS](#migrate-to-gcp-cloud-kms)
- [Verify Migration](#verify-migration)
- [Rollback Procedures](#rollback-procedures)
- [Cross-Provider KEK Rotation](#cross-provider-kek-rotation)
- [FAQ](#faq)

---

## Prerequisites

### AWS KMS

1. An AWS KMS symmetric key (AES-256) created in the target region
2. IAM credentials with `kms:Encrypt` and `kms:Decrypt` permissions on the key
3. Standard AWS credential chain configured (env vars, `~/.aws/credentials`, EC2 instance role, or ECS task role)
4. Build NyxID with the `aws-kms` feature flag:
   ```bash
   cargo build --release --features aws-kms
   ```

### GCP Cloud KMS

1. A GCP Cloud KMS key ring and symmetric encrypt/decrypt key
2. Service account or ADC credentials with `roles/cloudkms.cryptoKeyEncrypterDecrypter`
3. Application Default Credentials (ADC) configured
4. Build NyxID with the `gcp-kms` feature flag:
   ```bash
   cargo build --release --features gcp-kms
   ```

---

## Migration Overview

```mermaid
flowchart LR
    subgraph "Phase A: Dual-Provider (Migration Window)"
        A1["KEY_PROVIDER=aws-kms<br/>ENCRYPTION_KEY=&lt;old hex key&gt;<br/>AWS_KMS_KEY_ARN=arn:..."]
        A1 --> A2["New encryptions: KMS wraps DEK<br/>Old v2 reads: fallback to local<br/>v0/v1 reads: legacy fallback"]
    end

    subgraph "Phase B: Rewrap"
        B1["Operator-driven bounded migration<br/>of converted families"]
    end

    subgraph "Phase C: KMS-Only"
        C1["Remove ENCRYPTION_KEY<br/>All v2 DEKs wrapped by KMS<br/>No fallback needed"]
    end

    A1 --> B1 --> C1

    style A1 fill:#e67e22,color:#fff
    style B1 fill:#3498db,color:#fff
    style C1 fill:#27ae60,color:#fff
```

The migration uses a **fallback provider** to ensure zero downtime:

1. **Phase A**: Deploy with `KEY_PROVIDER=aws-kms` (or `gcp-kms`) while keeping `ENCRYPTION_KEY` set. New encryptions use KMS. Old v2 ciphertexts decrypt via the fallback local provider. Legacy v0/v1 decrypt via `LegacyKeys`.
2. **Phase B**: Run the explicit migration API for converted fields. v2 same-context rewrap can preserve the encrypted data; legacy-to-context conversion requires authenticated decrypt/re-encrypt. Resolve per-field findings and pending adapters.
3. **Phase C**: Remove the local fallback only after authenticated inventory, legacy-reader, rollback and retained-backup dependencies permit it. Zero `v2_fallback` traffic is insufficient evidence.

---

## Migrate to AWS KMS

### Step 1: Create the KMS Key

```bash
aws kms create-key \
  --key-spec SYMMETRIC_DEFAULT \
  --key-usage ENCRYPT_DECRYPT \
  --description "NyxID DEK wrapping key" \
  --region us-east-1

# Note the KeyId from the output, e.g.:
# arn:aws:kms:us-east-1:123456789012:key/mrk-abc123def456
```

### Step 2: Set IAM Permissions

The NyxID service needs `kms:Encrypt` and `kms:Decrypt` on the key. Example IAM policy:

```json
{
  "Version": "2012-10-17",
  "Statement": [
    {
      "Effect": "Allow",
      "Action": [
        "kms:Encrypt",
        "kms:Decrypt"
      ],
      "Resource": "arn:aws:kms:us-east-1:123456789012:key/mrk-abc123def456"
    }
  ]
}
```

### Step 3: Build with Feature Flag

```bash
cargo build --release --features aws-kms
```

### Step 4: Deploy with Dual-Provider Config

```bash
# Keep existing local key for fallback during migration
ENCRYPTION_KEY=<your existing 64 hex chars>

# Switch to AWS KMS for new encryptions
KEY_PROVIDER=aws-kms
AWS_KMS_KEY_ARN=arn:aws:kms:us-east-1:123456789012:key/mrk-abc123def456
```

At startup, NyxID will:
1. Initialize `AwsKmsProvider` as the primary provider
2. Initialize `LocalKeyProvider` as the fallback provider (because `ENCRYPTION_KEY` is set)
3. Check for key ID collisions between primary and fallback (panics if collision detected)
4. New `encrypt()` calls wrap DEKs with AWS KMS
5. `decrypt()` tries AWS KMS first, falls back to local for old v2 records

### Step 5: Run Rewrap Job

Use the [encrypted-record migration API](SECURITY.md#encrypted-record-migration). It holds a durable lease/cursor and uses exact-source CAS so a concurrent refresh, credential replacement or ownership transfer is not overwritten. It authenticates converted fields and records failures; pending adapters prevent complete readiness. Rewrap preserves payload bytes only when the envelope/context contract allows it. Traffic counters are diagnostics, not a complete inventory.

### Step 6: Remove Local Key (KMS-Only)

Only after authenticated field inventory, strict-reader readiness and retained-backup/rollback review permit local-key retirement:

```bash
# Remove local key -- KMS is now the sole provider
KEY_PROVIDER=aws-kms
AWS_KMS_KEY_ARN=arn:aws:kms:us-east-1:123456789012:key/mrk-abc123def456
# ENCRYPTION_KEY is no longer set
```

**Warning**: Only remove `ENCRYPTION_KEY` after confirming all v2 records have been rewrapped AND no v0/v1 records remain in the database. If v0/v1 records exist, keep `ENCRYPTION_KEY` set for legacy fallback.

---

## Migrate to GCP Cloud KMS

### Step 1: Create the Key Ring and Key

```bash
gcloud kms keyrings create nyxid-ring \
  --location us-east1

gcloud kms keys create nyxid-kek \
  --keyring nyxid-ring \
  --location us-east1 \
  --purpose encryption
```

The key resource name format:
```
projects/PROJECT_ID/locations/LOCATION/keyRings/KEYRING/cryptoKeys/KEY
```

### Step 2: Set IAM Permissions

```bash
gcloud kms keys add-iam-policy-binding nyxid-kek \
  --keyring nyxid-ring \
  --location us-east1 \
  --member serviceAccount:nyxid@PROJECT_ID.iam.gserviceaccount.com \
  --role roles/cloudkms.cryptoKeyEncrypterDecrypter
```

### Step 3: Build with Feature Flag

```bash
cargo build --release --features gcp-kms
```

### Step 4: Deploy with Dual-Provider Config

```bash
ENCRYPTION_KEY=<your existing 64 hex chars>
KEY_PROVIDER=gcp-kms
GCP_KMS_KEY_NAME=projects/my-project/locations/us-east1/keyRings/nyxid-ring/cryptoKeys/nyxid-kek
```

### Step 5: Run Rewrap Job

Use the same explicit bounded migration and authenticated readiness review as AWS. Keep the local fallback while any field, legacy reader, rollback binary or retained backup needs it.

### Step 6: Remove Local Key

```bash
KEY_PROVIDER=gcp-kms
GCP_KMS_KEY_NAME=projects/my-project/locations/us-east1/keyRings/nyxid-ring/cryptoKeys/nyxid-kek
# ENCRYPTION_KEY removed
```

---

## Verify Migration

### Check Decrypt Stats

The `EncryptionDecryptStats` struct (accessible via `decrypt_stats()`) provides counters for each decrypt path:

| Counter | Description | Target After Migration |
|---------|-------------|----------------------|
| `v2_current` | Decrypted with current KMS key | Increasing (normal traffic) |
| `v2_previous` | Decrypted with previous KMS key | 0 (after KMS rotation) |
| `v2_fallback` | Decrypted via fallback local provider | 0 expected after migration; not readiness evidence |
| `v1_current` | Legacy v1 with current local key | 0 (after re-encryption) |
| `v1_previous` | Legacy v1 with previous local key | 0 |
| `v0_current` | Legacy v0 with current local key | 0 (after re-encryption) |
| `v0_previous` | Legacy v0 with previous local key | 0 |

These counters observe recent decrypt traffic only. Complete migration requires authenticated field inventory and resolved findings, including untouched records. Pending adapters block readiness, and cloud-internal versions/backups require separate operator evidence.

### Startup Log Indicators

On startup with KMS provider:
```
INFO  AWS KMS provider initialized has_previous=false
```
or:
```
INFO  GCP Cloud KMS provider initialized has_previous=false
```

During decrypt of fallback records:
```
WARN  Decrypted v2 envelope via fallback provider; migration from previous provider is still in progress
```

---

## Rollback Procedures

### Rollback: KMS Back to Local

If issues arise after switching to KMS, preserve access to both local and KMS ciphertext before changing configuration. Keeping `ENCRYPTION_KEY` alone does not make KMS-written records readable by a local-only process.

Keep KMS primary plus the original local fallback while recovering. Switching directly to `KEY_PROVIDER=local` cannot decrypt KMS-written records, and the shipped configuration has no KMS-as-fallback setting. A return to local requires a separately reviewed migration that preserves access to both ciphertext sets; do not remove cloud access first.

Restore only a reader compatible with every stored contextual version and retain the matching HMAC keyset. Restoring an old database backup also requires its original provider/key dependencies. Preserve the original local key in the deployment's secret backup even after runtime retirement when backups still depend on it.

### Rollback: KMS Key Rotation

If a newly rotated KMS key causes issues:

```bash
# AWS: Set previous key ARN
AWS_KMS_KEY_ARN=arn:aws:kms:...:key/new-key
AWS_KMS_KEY_ARN_PREVIOUS=arn:aws:kms:...:key/old-key

# GCP: Set previous key name
GCP_KMS_KEY_NAME=projects/.../cryptoKeys/new-key
GCP_KMS_KEY_NAME_PREVIOUS=projects/.../cryptoKeys/old-key
```

The provider will try the current key first, then fall back to the previous key for unwrap operations.

### Rollback Decision Matrix

| Scenario | Action | Data Impact |
|----------|--------|-------------|
| KMS latency too high | Keep both readers; investigate or use a reviewed reverse migration | Local-only rollback cannot read KMS-written records |
| KMS auth failure | Fix IAM/credentials; keep KMS config | No data impact |
| Cross-provider key ID collision at startup | Use a different KMS key | No data impact (caught at startup) |
| KMS key accidentally disabled | Re-enable key in cloud console | Temporary 503 errors until re-enabled |
| KMS key deleted (in grace period) | Cancel deletion in cloud console | Temporary 503 errors until restored |
| KMS key permanently destroyed | **Unrecoverable** for KMS-wrapped records | Pre-migration local records still accessible if `ENCRYPTION_KEY` preserved |

---

## Cross-Provider KEK Rotation

### Rotating the KMS Key

KMS providers manage key rotation natively:

**AWS KMS:**
```bash
# Rotate the KMS key -- AWS automatically retains old key versions
AWS_KMS_KEY_ARN=arn:aws:kms:...:key/same-key-id
# AWS handles transparent decrypt with old versions
```

AWS KMS automatic rotation creates a new backing key but retains the same key ARN. Old ciphertexts are transparently decrypted using the old backing key.

**GCP Cloud KMS:**
```bash
# Create a new key version
gcloud kms keys versions create \
  --key nyxid-kek \
  --keyring nyxid-ring \
  --location us-east1

# Set new version as primary
gcloud kms keys update nyxid-kek \
  --keyring nyxid-ring \
  --location us-east1 \
  --primary-version <new-version-number>
```

GCP uses the primary version for encrypt and automatically selects the correct version for decrypt.

### Rotating Between Two KMS Keys (Same Provider)

To rotate from one KMS key to a completely different key:

```bash
# AWS
AWS_KMS_KEY_ARN=arn:aws:kms:...:key/new-key
AWS_KMS_KEY_ARN_PREVIOUS=arn:aws:kms:...:key/old-key

# GCP
GCP_KMS_KEY_NAME=projects/.../cryptoKeys/new-key
GCP_KMS_KEY_NAME_PREVIOUS=projects/.../cryptoKeys/old-key
```

First stage both-key readers on every replica, then switch the active key. Use the explicit migration/readiness procedure; remove `_PREVIOUS` only after authenticated inventory, legacy readers, rollback and backup dependencies allow it. A logical KMS key ID can still serve historical cloud-internal versions; successful decrypt does not prove those versions retired.

---

## FAQ

**Q: Can I use both AWS KMS and GCP KMS at the same time?**
A: No. Only one primary provider is active at a time. You can migrate between them using the fallback mechanism (e.g., AWS KMS primary with local fallback containing the local-encrypted records, then re-encrypt to GCP KMS).

**Q: What happens if the KMS is temporarily unavailable?**
A: Encrypt and decrypt operations return `AppError::Internal`. GCP KMS calls are retried (3 attempts, exponential backoff from 100ms). AWS SDK has built-in retry. If all retries fail, the request fails with an internal error.

**Q: How large are KMS-wrapped DEKs?**
A: LocalKeyProvider produces 60-byte wrapped DEKs. AWS KMS produces ~170-200 bytes. GCP Cloud KMS produces variable-size ciphertext. A `MAX_WRAPPED_DEK_SIZE` of 1024 bytes is enforced.

**Q: Does the v2 ciphertext format change with KMS?**
A: The v2 header format is unchanged. The `wrapped_dek_len` field (2-byte big-endian u16) accommodates variable wrapped DEK sizes. KMS-wrapped records are larger due to the bigger wrapped DEK blob.

**Q: Can I run the rewrap job while the application is serving traffic?**
A: The explicit runner uses exact-source CAS for converted families and can run with concurrent credential operations after reader/writer cutover. Keep fallback/previous keys and resolve findings; the raw primitive alone does not provide concurrency or inventory guarantees.

**Q: What if I get a key ID collision at startup?**
A: NyxID checks for collisions between the primary and fallback provider key IDs at startup and panics if one is detected (1-in-256 chance). Use a different KMS key or generate a new local key.
