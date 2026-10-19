# Token-copy admission and remote revocation

This is an implementation and rollout draft. Checkpoint16 compiled a holder
index for `user_api_keys` and `user_provider_tokens`; independent backend runtime
finished with 2,788 passed and 22 failed across 2,810 unique attempts. New control-plane and coverage-version changes postdate16 and passed the
shared backend preflight; their harness/runtime verification remains pending. **Production activation is blocked until the remaining holder
adapters and atomic writers are implemented.** No production activation has run.

## What the current index proves

User-key and provider-token writers authenticate their installed ciphertext in
the same transaction as their material write, update keyed token holders and
contend with dispatch on the exact token's durable fence. Before activation,
new writers touch a rollout fence even if the discovery marker is absent. An
activation transaction writes that fence too, so a held pre-activation writer
must commit before the captured scan or abort and re-enter admitted writing.

Discovery captures finite collection ceilings and advances one exact raw source
per transaction. New inserts below an already visited cursor install their own
holders. Completion is bound to the registry coverage version and digest;
adding collections, fields or decoder contracts requires a new authenticated
backfill. A historical two-collection complete marker cannot authorize a newer
registry. The current draft refuses that mismatch; an explicit reviewed upgrade
control remains required before upgrading such preview data. Do not edit a
complete marker or delete old fences to bypass it.

Post16 source adds decoders/backfill for catalog master credentials, configured
catalog and instance headers, and legacy service connection credentials. It
indexes exact scalar values and supported HTTP authentication scheme payloads,
including multiple spaces in bearer headers. OAuth access and refresh token
admission and dispatch protect both exact bytes and HTTP SP/HTAB OWS aliases,
so reversing which copy is disconnected does not escape the holder fence.
Opaque API-key fields retain exact-byte identity. Stored and wire material are
not rewritten. Authority receipts remain exact-request-byte evidence: an alias
is sufficient to block dispatch or import, never sufficient to infer that a
different form/JSON token request was revoked. The registry coverage version
and digest change with this identity contract, requiring a new backfill. Public and masked headers both
retain execution capability. Arbitrary encoded blobs are not claimed exhaustively
token-parsed. Writer integration and runtime verification remain incomplete.
Post17 source adds platform credential-map discovery and atomic normal writes/deletes/rewraps: X app bearer and Telegram manager tokens are independent retained capabilities; Meta/Aurinko client and signing secrets are separate protocols. Coverage version5 invalidates earlier coverage proofs. Unknown platform descriptors block discovery. This slice is pending compilation/runtime. Channel bots, platform transports and other server-readable execution capabilities still need explicit source accounting. Oracle login
profiles/snapshots contain recipient-sealed browser envelopes: the server
authenticates/decrypts only their outer layer using the established AAD. Their
inner material is unobservable and recipient controlled, like node E2E storage;
it is outside server-readable token-copy detection, while outer key migration
and backup/retirement obligations remain in W4. The other Oracle credential
fields are SHA-256 pool/worker verifiers, not retained raw provider tokens.
References which resolve back to an indexed row must be distinguished from
independent retained material; for example, an X connection reader uses its
UserApiKey, while a bot's own token field is a separate holder. OAuth broker
bindings retain NyxID-issued refresh tokens under their existing binding AAD,
and require explicit issuer/protocol review rather than classification as an
unrelated provider token by field name. Remaining source/caller review is not
complete. The new status control lists these implementation blockers and
activation refuses them; an operator acknowledgement cannot waive coverage.

## Reader-first cutover and controls

1. Preserve the exact stable HMAC key and its backup as described in
   [key lifecycle](KEY_LIFECYCLE.md). Upgrade readers and every credential writer,
   including startup copies, transfers, refreshes, manual imports and late
   issuance. Finish the source coverage/admission audit before activation.
2. Drain old binaries and in-flight old-writer transactions. Rollback binaries
   must enforce the same holder contract; starting an old writer after cutover
   invalidates the proof. Do not infer drain from a scan finding no rows.
3. A first-party human admin may request `POST /api/v1/admin/token-admission`
   with `{ "old_writers_drained": true }`. Activation and its chained actor audit
   commit atomically with snapshot/majority concerns. Activation remains blocked
   by missing holder coverage in the current source. No startup task performs
   this acknowledgement automatically.
4. Admin/operator reads use `GET /api/v1/admin/token-admission`. After activation,
   first-party human admins request
   `POST /api/v1/admin/token-admission/{generation}/advance`. One request attempts
   at most 32 rows. The generation is public operation metadata, never a token or
   authority digest. Concurrent steps serialize through exact progress/source
   predicates. There is no skip, force-complete or unbounded request scan.
5. An unreadable row retains a blocked cursor and a fixed diagnostic. The original five
   domain-ID collections have explicitly reviewed IDs; platform credential IDs remain opaque in repair responses. Only the
   per-family exposure allowlist can identify a row to repair through normal credential management. Restore
   matching keys or replace/remove the source through its normal authenticated
   path, then retry the same generation. The cursor and observed-row count
   advance transactionally only after authenticated holder installation.

The POST controls require ordinary human/admin checks; delegated credentials
cannot activate or step. Metadata reads contain no token fingerprints, client
secret digests, ciphertext or protocol delivery handles. A failed or uncertain
transaction does not create a completion claim.

## Dispatch, duplicates and preparation

Closing a token is global across owners and provider labels. The fence is never
released after a timeout: the provider may already have acted. A receipt is
specific to the captured revocation authority, including endpoint, style,
reviewed effect semantics and resolved client identity/credentials. Equivalent
effects can wait durably and reuse confirmed results; a providerB unknown-token
HTTP200 cannot settle providerA's obligation. Partial success persists each
token receipt before attempting the next token. Pending/unknown results retain
the encrypted material and operator evidence.

Direct disconnect commits source removal and the encrypted remote intent
atomically. Failure to persist intent rolls back removal. Broad or unreviewed
grant effects require external reconciliation, including a delayed first call:
an old token alone cannot prove that a grant-wide endpoint will spare a newly
authorized successor grant. An accepted local cascade is not remote completion.

Once a complete matching index exists, preparation uses finite scan ceilings and
the affected owner's dependency revision without a global write on unrelated
owners. Preparation is an observation of owner material, not proof that no
future cross-owner import exists. The actual dispatch transaction rereads live
holders and closes the token before HTTP. A matching import which wins first
leaves an unresolved durable revocation obligation; erasure must not report
that obligation complete. An import which loses to closure rolls back its
material write. External copies unknown to NyxID cannot be proven absent; this
contract addresses supported in-platform holders and does not assert universal
upstream isolation.
