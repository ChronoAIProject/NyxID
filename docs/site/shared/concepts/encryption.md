---
title: Encryption & key management
description: How NyxID encrypts the third-party credentials it holds — AES-256 envelope encryption with pluggable local or cloud-KMS key providers.
---

NyxID is a credential broker: it holds your third-party API keys and OAuth tokens so your agents and apps never see them. That only works if those secrets are protected at rest. Dedicated credential fields use **AES-256 envelope encryption**. Converted default-header writers encrypt every configured value, including unfamiliar header names and values whose `sensitive` flag is omitted. That flag controls management masking, not encryption. Reader rollout still permits legacy plaintext until migration and strict cutover; conversion of other field families remains incomplete under #1934.

## Envelope encryption

Each stored secret is encrypted with a unique **data encryption key (DEK)** using AES-256-GCM. The DEK itself is then encrypted ("wrapped") by a **key encryption key (KEK)** that lives in a key provider, not in MongoDB. Only the wrapped DEK is persisted alongside the ciphertext.

To decrypt a credential at proxy time, NyxID asks the key provider to unwrap the DEK, decrypts the secret in memory, injects it into the outbound request, and discards it. Cloud KMS KEKs never leave KMS; a local provider holds its KEK in the application process.

:::note
The size of a wrapped DEK is bounded (`MAX_WRAPPED_DEK_SIZE`) on both the encrypt and decrypt paths, so a malformed or hostile blob can't force unbounded work.
:::

## Key providers

The KEK is supplied by a pluggable provider, selected with the `KEY_PROVIDER` environment variable:

- **`local`** (default) — the KEK is a 32-byte AES-256 key supplied as `ENCRYPTION_KEY` (64 hex chars). Simplest to run; you own the key material.
- **`aws-kms`** — the KEK lives in AWS KMS and never leaves it; NyxID calls KMS to wrap/unwrap. Enabled by the `aws-kms` build feature and `AWS_KMS_KEY_ARN`.
- **`gcp-kms`** — the same model backed by Google Cloud KMS, behind the `gcp-kms` feature and `GCP_KMS_KEY_NAME`.

Cloud KMS keeps the wrapping KEK out of the application process. NyxID still holds other long-lived secrets, including its local JWT signing PEM and configured HMAC material. Database access does not itself grant KMS access; application/KMS permissions remain a separate trust boundary.

## Rotation and migration

Keys can be rotated without downtime:

- **Same backend:** prepare every replica to read both keys before enabling new-key writes, then retain the old key as previous (`ENCRYPTION_KEY_PREVIOUS`, or the provider-specific previous-key setting). Follow the ordered operator runbook in `docs/SECURITY.md` → Key Rotation; changing a writer first is unsafe. NyxID can decrypt with either key and writes new ciphertext with the active key. A bounded, leased migration and verification registry is implemented for converted families. It still reports unfinished adapters, so it is not yet evidence for complete production migration or key retirement. Repeating a pass authenticates current records without rewriting them.
- **From local to KMS:** the shipped configuration supports a KMS primary with an optional local fallback. Stage readers that can decrypt both ciphertext sets before changing writers. There is no arbitrary cloud-to-cloud or KMS-as-fallback configuration; those transitions require a separately reviewed migration. See the repository runbook `docs/KMS_MIGRATION_GUIDE.md`.

Before changing AES or JWT sources, preserve the existing final HMAC domain keys using `HMAC_KEYSET_PATH`. This adoption keeps historical verification stable; it is not arbitrary HMAC-key rotation. Contextual writes default off; activating them requires compatible readers plus the explicit `CONTEXTUAL_RECORD_WRITERS_DRAINED` acknowledgement. Disable legacy reads only after authenticated migration readiness.

## Defense in depth

- Key-provider DEKs and converted record-field reads use **`Zeroizing`** wrappers. Raw decrypt entry points still return ordinary buffers and remain callable outside crypto; restricting that interface and propagating guarded buffers remain unfinished under #1933 and #1934. Zeroization does not erase library copies or protect a live process from memory access.
- Identified AWS credential, signed-header and exchange-cache Debug implementations redact sensitive values. This is not proof that every secret-bearing type or error in the repository is safe to log.
- Encryption coverage must be checked by field and transport. Dedicated credential fields and new configured default-header values are encrypted. Historical plaintext headers require the staged migration path, and remaining families must be converted before strict global readiness.
- Keep previous providers/keys until a complete ciphertext inventory and retained backups permit retirement. Zero old-key decrypt traffic does not prove migration completed.

## Related

- [Manage keys & credentials](/docs/web/guides/manage-keys) — adding, rotating, and scoping the credentials NyxID encrypts.
- [The broker model](/docs/shared/concepts/broker-model) — why NyxID holds credentials in the first place.
