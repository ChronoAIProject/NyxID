# JWT overlap and independent HMAC keyset adoption

This operator runbook describes the implementation draft under local review.
Runtime acceptance for this batch is tracked in
[the W1–W4 checklist](../SECURITY_REMEDIATION_W1_W4.md). No deployment or
production key migration has been performed by this remediation work.

## Prepare stable HMAC keys before changing JWT or encryption sources

Six HMAC domains historically derive from the active encryption key or JWT
private PEM: CLI pairing, auth-device login, audit, assistant action receipts,
billing ledger and internal dispatch. Existing environment overrides affect
their respective domains. Audit-derived material also keys instruction context,
call/channel identities and callbacks, voice credential revisions, Machine and
Oracle lifecycle, service history and other persisted operation fingerprints.

The token-copy admission ledger adds durable audit-derived dependencies:

| Collection | Key-dependent evidence | Recovery requirement |
|---|---|---|
| `credential_token_holders` | Exact-token fingerprints and explicit bearer OWS safety aliases under `oauth-revocation-provenance-v1`; authenticated source-to-token associations | Restore the same stable audit-domain key with the database, then reconcile the matching holder coverage/backfill contract before dispatch. |
| `credential_token_fences` | The same global fingerprints identify permanently closed tokens across provider and owner labels | Preserve tombstones and their original ownership/uncertainty. A new key cannot safely rediscover these identities or permit an old token to return. |
| `credential_token_revocations` | Token fingerprints plus `oauth-revocation-authority-v1` digests of captured provider endpoint/style/effect scope and resolved client identity/credentials | Confirmations apply only to equivalent captured authorities. Keep the original key to match delayed effects and partial-success receipts; never treat another authority's HTTP200 as confirmation. |
| `credential_revocation_dependencies` | Keyed token fingerprints and grant-origin provenance used by erasure preparation | Preserve the key and source revision semantics; an empty/mismatching index is not evidence of isolation. |

These are stable-key **backup and retirement dependencies**, even after local
source ciphertext has been removed. Replacing the audit key would make old
closures and receipts unmatchable. The current implementation has no complete
versioned re-key protocol for these ledgers, and the encryption migration
runner does not rotate their HMAC identities. Independent keyset adoption keeps
the exact existing bytes; arbitrary HMAC-key rotation remains unsupported.
Token-holder implementation coverage and deployment activation are tracked
separately in [the token admission runbook](TOKEN_COPY_ADMISSION.md).

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
random environment override invalidates historical verification. W8 retention
and external witnessing remain separate dependencies; no unsafe TTL, historic
mutation or re-signing is authorized here.

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
[coordinated OAuth authority cutover](OAUTH_AUTHORITY_ROLLOUT.md); JWT verifier
overlap does not make mixed old/new authority readers safe or preserve legacy
callbacks. Affected clients must reauthorize.
