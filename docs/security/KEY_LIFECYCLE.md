# JWT overlap and independent HMAC keyset adoption

This runbook describes independent HMAC keyset adoption and staged JWT signer
rotation. Preserve the existing HMAC domain keys before changing JWT or
encryption sources, then stage JWT verifier overlap before activating a new
signer. Deployment operators must perform and verify each rollout step.

## Prepare stable HMAC keys before changing JWT or encryption sources

Six HMAC domains historically derive from the active encryption key or JWT
private PEM: CLI pairing, auth-device login, audit, assistant action receipts,
billing ledger and internal dispatch. Existing environment overrides affect
their respective domains. Audit-derived material also keys instruction context,
call/channel identities and callbacks, voice credential revisions, Machine and
Oracle lifecycle, service history and other persisted operation fingerprints.

Durable provider-revocation obligations retain these audit-derived digests on
`personal_erasure_effects`. Delivery receipts use the separate, owner-scoped
`personal_erasure_revocation_receipts` collection:

| Digest domain | Key-dependent evidence | Recovery requirement |
|---|---|---|
| `oauth-revocation-authority-v1` | Captured provider endpoint, style, scope and resolved OAuth app credentials; per-owner token SHA-256 dedup and receipt matching | Restore the same stable audit-domain key so a receipt matches only its captured revocation authority. |
| `provider-issuance-response-v1` | Per-effect evidence of a late provider issuance response | Preserve the key with unresolved issuance evidence; this is not a token index. |
| `provider-install-token-v1` | Per-effect evidence that the issued credential was installed | Preserve the key until the issuance/install obligation and retained evidence no longer need it; this is not a token index. |

Erasure attestations are ordinary chained audit rows. The digests and receipts
are stable-key backup and retirement dependencies even after source ciphertext
has been removed. Receipts remain until final owner cleanup, after every
provider obligation settles. See the [personal-erasure runbook](PERSONAL_ERASURE_INVENTORY.md#durable-phases-and-completion)
for dispatch limits, operator resolutions and completion rules. Independent
keyset adoption keeps the existing digest bytes; the encryption migration
runner does not rotate these identities, and arbitrary HMAC-key rotation
remains unsupported.

Independent keyset adoption preserves the **exact existing final domain keys**.
It does not rotate those HMAC keys. Keeping their bytes unchanged preserves
historical chain verification and delayed-operation matching without rewriting
any rows or trying a new key against old fingerprints. The legacy internal
dispatch and assistant-action derivations consume encryption hex **string
bytes**; the other four decode the hex. Export retains that distinction.

1. Keep the deployed JWT PEM, encryption source and existing domain overrides
   unchanged. Back up the original sources securely. Upgrade compatible readers
   before starting any source rotation.
2. Run the server binary's local command
   `nyxid-server export-hmac-keyset --path /restricted/new-hmac-keyset.json`
   with the existing deployment configuration. The new file must not exist.
   The command exports keys to an exclusively created file with mode 0600 on
   Unix; it never prints them. A partial export must be restored/recreated
   before use. Do not check this file into source control.
3. Preparation compares all six values to unchanged legacy derivations and
   checks the MAC of the latest chained audit row and latest billing row before
   creating a majority-durable consistency pin in `security_hmac_keysets`.
   This is bounded recent evidence, **not a full history audit**. Existing full
   Integrity verification remains available and should be reviewed separately.
   No historical row is edited or re-signed.
4. Provision the same exported file through the deployment's secret delivery
   mechanism to every replica and set `HMAC_KEYSET_PATH` to its local path.
   Confirm every replica starts successfully with the file before changing
   JWT/encryption sources. A configured but unreadable/malformed file fails
   closed. A file differing from the durable pin also fails closed.
5. Keep the keyset and its recovery backup for as long as retained chains or
   persisted fingerprints depend on it. Then use the staged reader rollout
   appropriate to the independent JWT/encryption change below.

Ordinary unchanged-source startup can create the same pin using exact legacy
derivations and the two recent MAC checks. A pin records commitments, never raw
keys. First adoption must precede source rotation: if sources were already
changed, restore the originals and prepare there. Do not remove or replace the
pin to bypass a mismatch. Restoring a database backup also requires its matching
keyset. Existing replicas using legacy derivation must keep the old sources
throughout the adoption stage.

Arbitrary HMAC-key rotation, versioned chain signing and re-keying all persisted
fingerprints are outside this adoption contract. Replacing a chain key with a
random environment override invalidates historical verification. Retention
policy and external witnessing remain separate concerns; this adoption changes
neither retention nor historical signatures.

## Stage JWT verifier overlap before activating the new signer

The active private/public pair must correspond and contain RSA keys of at least
3072 bits. Private imports accept PKCS#1/PKCS#8; public imports accept
PKCS#1/SPKI. A partial pair fails without replacing the surviving file. Generated
private files are restrictive from their first creation and existing files are
never truncated.

A previously accepted imported RSA key smaller than 3072 bits cannot be active
or retained in this strict ring. The export command deliberately runs before
signer import and can prepare the exact legacy HMAC bytes from that old PEM.
For such a deployment, prepare independent HMAC material while the original
sources are available, then plan a coordinated signer cutover with token
invalidation and user/client reauthorization. Ordinary retained-key overlap is
unavailable for the rejected weak key; do not silently lower validation or
claim an uninterrupted token chain. Keep the original material securely for
independent HMAC recovery rather than configuring it as a JWT verifier.

`JWT_RETAINED_PUBLIC_KEY_PATHS` is a comma-separated list of at most sixteen
retained public PEM paths. Duplicate key identifiers, including duplicates of
the active key, fail configuration. Tokens require RS256 and a known `kid`;
unknown/missing identifiers never trigger key probing or a fallback.

1. Complete independent HMAC adoption above. Record token lifetime and refresh
   policy, consumer JWKS caching and all replicas that must read the tokens.
2. Keep the old active signer and add the new public PEM to every replica's
   retained list. Deploy and confirm all readers have both keys before any new
   signing. The JWKS endpoint publishes both validated public keys.
3. Switch the active pair to the new signer and retain the old public PEM.
   Prepared replicas read both generations throughout this rolling change.
   OAuth consent requests, public assertions and channel reply/callback tokens
   use the same strict selection, alongside access/refresh/ID tokens.
4. Retain the old verifier until all tokens it signed that must remain valid
   have expired or been deliberately revoked, and dependent JWKS caches have
   converged. Refresh-token lifetimes can exceed access-token lifetimes. Remove
   the old public PEM only after that review. Retirement deliberately rejects
   the retired generation.

For rollback during overlap, restore the old active pair while retaining the
new public PEM, so tokens written by either signer still verify. Keep the same
HMAC keyset. External remote-machine bootstrap/provider issuer and public-key
configuration remains a separate trust root; it does not import this ring.

Encryption rotation additionally requires the ordered dual-reader staging and
full ciphertext migration/retirement readiness in
[the security operator runbook](../SECURITY.md#key-rotation). JWT overlap and a
stable HMAC keyset alone do not prove ciphertext/backups ready for key retirement.

OAuth consent revisions and signed delegation origins introduce an independent
authority compatibility boundary. Follow the
[coordinated OAuth authority cutover](OAUTH_AUTHORITY_ROLLOUT.md#coordinated-drained-deployment);
JWT verifier overlap does not make mixed old/new authority readers safe. Under
the runbook's [refresh-record upgrade rules](OAUTH_AUTHORITY_ROLLOUT.md#compatibility-boundary),
eligible ordinary refresh records upgrade once without reauthorization;
ineligible records require authorization. Legacy delegation and callback work
must drain before the cutover.

## Enrolling the encryption ring and recording dependencies

A full administrator can enroll the configuration already in use. Enrollment assigns a deployment UUID, a restore epoch and an opaque UUID to each configured key. A ring with a previous or fallback key starts with an open rotation. Each slot must authenticate its own contextual challenge, including when contextual *application* writes are disabled. The AWS enrollment identity needs `kms:DescribeKey` to resolve the configured resource plus encrypt/decrypt access to each enrolled key. GCP enrollment uses the configured CryptoKey resource. Cloud proof covers the logical resource. A claim about a historical internal version needs separately recorded provider evidence.

The following paths are relative to `/api/v1/admin/key-lifecycle`. Use a first-party person session or access token. The lifecycle observer needs MongoDB catalog access and permission to read the replica-set configuration. Admins and operators can read. Writes require a full admin. The operator read audit identifies the endpoint. Each mutation, including the enrollment, commits a chained audit with the lifecycle revision in the same majority transaction.

| Method and path | Operation |
|---|---|
| `GET /` (base path without trailing slash) | Read the enrolled scope, opaque keys, revision, restore blocker and current evidence pointers. A JSON null means enrollment has not occurred. The HMAC file check compares all six configured-file commitments with the durable pin. Fleet activation is a separate check. |
| `POST /enroll` | Supply `deployment_revision` and `recording` with `expected_revision: 0`. Record the owner of the initial inventory. |
| `GET /dependencies` | Read dependency records in opening-revision order. Each record retains its parents, key and format scope, owner, last receipt and resolution. |
| `GET /evidence` | Read immutable entries in revision order. Each includes authenticated producer/issuer, named owner, evidence reference, before/after scope and database-recorded time. |
| `POST /evidence` | Append a typed assertion, inventory change, ring verification or epoch change. The receipt names the committed revision. |

For either list, `after` is an exclusive revision and `limit` defaults to 50, up to 100. Continue from the last opening revision for dependencies or the last receipt revision for evidence. Use the singleton revision to detect changes between pages.

Every mutation carries the following recording fields.

```json
{
  "recording": {
    "request_id": "0c0845a9-70f1-4129-b4c6-f73b81c200e5",
    "expected_revision": 0,
    "owner": "identity-operations",
    "evidence_ref": "fc9cb60a-b1fc-4249-8220-ba6eeb5011d8"
  },
  "deployment_revision": "release-2026-11-26"
}
```

Generate fresh v4 UUIDs for request IDs and immutable evidence references. Keep the referenced documents in the owner's controlled inventory. Owner and deployment revision are bounded directory or release labels. Repeating the same request as its original actor yields the original dated receipt. After a revision conflict, reload the scope and review the intervening entries before submitting a new request.

For `POST /evidence`, replace the enrollment's `deployment_revision` field with `scope` from the status response and a `change` object. Scope includes deployment ID, release revision, restore epoch and the affected key references. A dependency with unknown key scope covers every enrolled key. The recording API assigns the issuer and producer. Version-one assertion payloads and immutable scope are independent of how the issuer authenticated, so a future automated attester can use the same stored format.

1. **Inventory.** Submit `open_dependency` with `inventory`, `parents` (IDs of prerequisite dependencies) and `formats`. Its entry ID becomes the dependency ID. Record reader and writer cohorts, independent HMAC adoption, migration canaries, database credentials, full and incremental backups, PITR, clones, archives, restore drills, rollback images, cloud versions and unknown dependencies. Record the parents of each incremental or PITR chain and include their keys and formats. Register a copy before making it restorable. Include a newly discovered restorable copy even if a previous review already closed the inventory.
2. **Closure.** Submit `close_dependency` with `dependency_id` and a typed `resolution`. Close restorable children before their parents. Backup and PITR evidence must establish verified disposal or a tested survivor restore. A rollback image requires withdrawal or a tested survivor rollback. Record revocation or rotation for credentials and provider-specific verification for a cloud version. An expiry date alone cannot close a dependency. Submit `reopen_dependency` with the same dependency ID to supersede a closure. Use `review_inventory` with `inventory` and disposition `reviewed`, `reviewed_empty` or `withdrawn`. Reader, HMAC and migration-canary dependencies accept the corresponding tested survivor-reader, verified independent-HMAC and tested survivor-canary evidence. A missing review leaves coverage unknown.
3. **Evidence after the baseline.** Use `attest` with `assertion`, `affirmed` and optional `valid_until_ms` (Unix milliseconds). Record `old_writers_withdrawn`, `database_access_revoked`, `restore_continuity` and `recovery_dependencies_closed`. Include old images, deployments and jobs, obsolete credentials and live sessions, and all restorable backup, PITR and rollback dependencies. A changed dependency clears its prior inventory review and aggregate assertion for the affected keys. A withdrawal or replacement appends a new revision. The decision reader must check those exact entries, their expiry and the singleton at decision commit. To establish strict post-baseline ordering, read the singleton revision at snapshot S and require each relied-on evidence revision to be greater. Wall-clock ties are possible.
4. **Restore or deployment change.** Submit `advance_restore_epoch` for the complete ring after an announced restore/import, or `change_deployment_revision` with the new release revision when that scope changes. Reread status, start a fresh census and record fresh scoped reviews and attestations. A detected database or lifecycle collection identity change opens a durable restore blocker. Only an explicit epoch advance clears it. NyxID cannot independently prove that an unannounced restore or import did not occur. A restore can roll back the database, journal and evidence together.
5. **Verification and rollback.** Submit `verify_ring` for the complete scope to challenge the enrolled layout again. The previous release can continue serving under the enrolled configuration and ignores the new collections. Enrollment authorizes neither a key removal nor a second rotation. The census runner and release rehearsal must repeat the fixed-load throughput and tail-latency gate on an isolated rig before enabling certification.
