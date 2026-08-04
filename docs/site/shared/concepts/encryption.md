---
title: Encryption & key management
description: How NyxID encrypts the third-party credentials it holds — AES-256 envelope encryption with pluggable local or cloud-KMS key providers.
---

NyxID is a credential broker: it holds your third-party API keys and OAuth tokens so your agents and apps never see them. That only works if those secrets are protected at rest. Every credential NyxID stores is encrypted with **AES-256 envelope encryption**, and the key that protects them never lives in the database.

## Envelope encryption

Each stored secret is encrypted with a unique **data encryption key (DEK)** using AES-256-GCM. The DEK itself is then encrypted ("wrapped") by a **key encryption key (KEK)** that lives in a key provider, not in MongoDB. Only the wrapped DEK is persisted alongside the ciphertext.

To decrypt a credential at proxy time, NyxID asks the key provider to unwrap the DEK, decrypts the secret in memory, injects it into the outbound request, and discards it. The KEK never leaves the provider.

:::note
The size of a wrapped DEK is bounded (`MAX_WRAPPED_DEK_SIZE`) on both the encrypt and decrypt paths, so a malformed or hostile blob can't force unbounded work.
:::

## Key providers

The KEK is supplied by a pluggable provider, selected with the `KEY_PROVIDER` environment variable:

- **`local`** (default) — the KEK is a 32-byte AES-256 key supplied as `ENCRYPTION_KEY` (64 hex chars). Simplest to run; you own the key material.
- **`aws-kms`** — the KEK lives in AWS KMS and never leaves it; NyxID calls KMS to wrap/unwrap. Enabled by the `aws-kms` build feature and `AWS_KMS_KEY_ARN`.
- **`gcp-kms`** — the same model backed by Google Cloud KMS, behind the `gcp-kms` feature and `GCP_KMS_KEY_NAME`.

With a cloud KMS provider, NyxID holds no long-lived key material at all — compromising the database yields only ciphertext and wrapped DEKs that are useless without KMS access.

## Rotation and migration

Keys can be rotated without downtime:

- **Same backend:** set `ENCRYPTION_KEY_PREVIOUS` (or `..._KEY_ARN_PREVIOUS` / `..._KEY_NAME_PREVIOUS`) to the old key. NyxID decrypts with either key and re-wraps with the new one as records are touched.
- **Across backends:** a **fallback provider** lets NyxID read secrets wrapped by the old provider while writing new ones with the new provider, so you can migrate from `local` to a cloud KMS (or between clouds) with zero downtime.

## Defense in depth

- All key material is held in **`Zeroizing`** wrappers, so plaintext keys are scrubbed from memory as soon as they go out of scope.
- Every `Debug` implementation **redacts** secrets and key identifiers — keys can't leak into logs or panics.
- Encryption is enforced at the storage boundary, so no service path can persist a credential in plaintext.

## Related

- [Manage keys & credentials](/docs/web/guides/manage-keys) — adding, rotating, and scoping the credentials NyxID encrypts.
- [The broker model](/docs/shared/concepts/broker-model) — why NyxID holds credentials in the first place.
