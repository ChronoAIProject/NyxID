# Server-encrypted field inventory and migration contract

This inventory mirrors the collection, field and adapter entries in the
[encrypted-record migration registry](../../backend/src/services/encrypted_record_migration.rs).
**Implemented** means the registry dispatches to a service-owned verifier and
migration adapter. It describes the source implementation; it does not establish
that a deployment has completed migration or is ready to retire a key.
**Pending** means the family still lacks a completed contextual adapter and
conversion of its actual writers and readers. Pending adapters block complete
migration readiness, including when their collections are empty.

A canonical context binds the collection, record ID, canonical owner and exact
field identity without ambiguous concatenation.
New envelope metadata and context are authenticated. Authentication failure in a
selected contextual format is terminal and never retries an unbound decoder.
Supported older formats use their explicit storage contract and obey the
legacy-reader policy. Existing additional authenticated data (AAD) contracts
must retain their established dimensions during rewrap.

## Registered fields

Each row corresponds to one registry entry. Multiple rows for a collection
represent distinct fields and adapter contracts. Pending field paths show the
current registry contract; completing a family also requires auditing nested
storage and every production writer and reader for fields missing from that
contract.

| Collection | Exact encrypted field paths | Service-owned adapter | Status |
|---|---|---|---|
| `user_provider_tokens` | `access_token_encrypted`, `refresh_token_encrypted`, `api_key_encrypted` | [credential_record_service.rs](../../backend/src/services/credential_record_service.rs) | Implemented |
| `user_api_keys` | `credential_encrypted`, `access_token_encrypted`, `refresh_token_encrypted`, `user_oauth_client_id_encrypted`, `user_oauth_client_secret_encrypted` | [credential_record_service.rs](../../backend/src/services/credential_record_service.rs) | Implemented |
| `user_provider_credentials` | `client_id_encrypted`, `client_secret_encrypted` | [credential_record_service.rs](../../backend/src/services/credential_record_service.rs) | Implemented |
| `provider_configs` | `client_id_encrypted`, `client_secret_encrypted` | [credential_record_service.rs](../../backend/src/services/credential_record_service.rs) | Implemented |
| `platform_credentials` | `secrets` | [platform_credential_service.rs](../../backend/src/services/platform_credential_service.rs) | Implemented |
| `nodes` | `signing_secret_encrypted` | [node_service.rs](../../backend/src/services/node_service.rs) | Implemented |
| `mfa_factors` | `secret_encrypted` | [mfa_service.rs](../../backend/src/services/mfa_service.rs) | Implemented |
| `saved_logins` | `username_encrypted`, `password_encrypted`, `totp_secret_encrypted` | [saved_login_service.rs](../../backend/src/services/saved_login_service.rs) | Implemented |
| `oauth_states` | `code_verifier`, `device_code_encrypted`, `user_code_encrypted` | [oauth_state_record_service.rs](../../backend/src/services/oauth_state_record_service.rs) | Implemented |
| `assistant_agent_credentials` | `key_ciphertext` | [assistant_record_service.rs](../../backend/src/services/assistant_record_service.rs) | Implemented |
| `assistant_attachments` | `text_encrypted`, `data_encrypted` | [assistant_record_service.rs](../../backend/src/services/assistant_record_service.rs) | Implemented |
| `platform_public_sessions` | `credential_ciphertext`, `upstream_binding_ciphertext` | [platform_public_record_service.rs](../../backend/src/services/platform_public_record_service.rs) | Implemented |
| `platform_public_turns` | `input_ciphertext`, `reply_ciphertext` | [platform_public_record_service.rs](../../backend/src/services/platform_public_record_service.rs) | Implemented |
| `downstream_services` | `default_request_headers` | [default_header_service.rs](../../backend/src/services/default_header_service.rs) | Implemented |
| `user_services` | `default_request_headers` | [default_header_service.rs](../../backend/src/services/default_header_service.rs) | Implemented |
| `personal_erasure_effects` | `capability_encrypted` | [personal_erasure_service/provider.rs](../../backend/src/services/personal_erasure_service/provider.rs) | Implemented |
| `agent_key_login_requests` | `delivery_credential_encrypted` | [agent_login_record_service.rs](../../backend/src/services/agent_login_record_service.rs) | Implemented |
| `assistant_agent_learning_proposals` | `scope_encrypted`, `body_encrypted`, `publication.interface_encrypted` | — | Pending |
| `assistant_async_operations` | `result_encrypted` | [async_service_operation.rs](../../backend/src/services/async_service_operation.rs) | Implemented |
| `auth_device_codes` | `delivery_access_token_encrypted`, `delivery_refresh_token_encrypted` | — | Pending |
| `auth_device_codes_v2` | `delivery_credential_encrypted` | — | Pending |
| `device_codes` | `delivery_api_key_encrypted`, `delivery_refresh_token_encrypted` | — | Pending |
| `channel_bots` | `registration_pin_encrypted`, `webhook_secret_encrypted`, `bot_token_encrypted`, `app_secret_encrypted`, `lark_verification_token_encrypted`, `lark_encrypt_key_encrypted` | — | Pending |
| `channel_connect_links` | `receiver.secret_encrypted` | [channel_connect_link_service.rs](../../backend/src/services/channel_connect_link_service.rs) | Implemented |
| `downstream_services` | `credential_encrypted` | [catalog_credential_record_service.rs](../../backend/src/services/catalog_credential_record_service.rs) | Implemented |
| `downstream_services` | `ssh_config.ca_private_key_encrypted` | [ssh_ca_record_service.rs](../../backend/src/services/ssh_ca_record_service.rs) | Implemented |
| `nyxbot_channels` | `agent_key_ciphertext` | — | Pending |
| `nyxbot_threads` | `event_ref_ciphertext` | — | Pending |
| `nyxbot_events` | `event_ref_ciphertext`, `event_context_ciphertext` | — | Pending |
| `nyxbot_watches` | `target_ciphertext` | — | Pending |
| `oauth_broker_bindings` | `refresh_token_encrypted` | [oauth_broker_record_service.rs](../../backend/src/services/oauth_broker_record_service.rs) | Implemented |
| `oauth_clients` | `revocation_webhook_secret_encrypted`, `connection_webhook_secret_encrypted` | — | Pending |
| `oracle_login_profiles` | `encrypted_envelope` | [oracle_login_record_service.rs](../../backend/src/services/oracle_login_record_service.rs) | Implemented |
| `oracle_login_snapshots` | `encrypted_envelope` | [oracle_login_record_service.rs](../../backend/src/services/oracle_login_record_service.rs) | Implemented |
| `platform_entry_gateway_transports` | `request_ciphertext` | — | Pending |
| `platform_native_reply_handles` | `ciphertext` | — | Pending |
| `platform_setups` | `facts_ciphertext` | [platform_setup_record_service.rs](../../backend/src/services/platform_setup_record_service.rs) | Implemented |
| `triggers` | `verification_secret_encrypted`, `delivery_secret_encrypted` | [trigger_record_service.rs](../../backend/src/services/trigger_record_service.rs) | Implemented |
| `trigger_deliveries` | `envelope_encrypted` | [trigger_record_service.rs](../../backend/src/services/trigger_record_service.rs) | Implemented |
| `user_service_connections` | `credential_encrypted` | [connection_record_service.rs](../../backend/src/services/connection_record_service.rs) | Implemented |

## Source and storage boundaries

- `managed_machine_bootstrap_deliveries` and `node_pending_credentials` contain
  recipient-sealed deliveries, outside server key-encryption-key migration.
  Their expiry and recipient lifecycle continue separately.
- `assistant_upload.rs` re-exports the `assistant_attachments` collection name;
  there is no separate `assistant_uploads` collection. Text and binary attachment
  fields have separate contexts.
- Shared provider and platform credentials belong to their explicit credential
  owner or the platform. Actor metadata such as `created_by` or `updated_by`
  must not substitute for a separately recorded owner.
- Catalog master credentials bind `owner_user_id`, falling back to `created_by`
  only for missing or null legacy ownership. Service transfer authenticates and
  rebinds the master credential and headers in the same exact-source owner
  mutation. SSH CA context remains platform/service/public-key bound across
  owner transfer.
- Fields bound to an owner must authenticate and re-encrypt for the new owner
  in the same fenced transfer transaction. This remains required follow-up work
  for the pending channel bot family. Oracle pool transfer must preserve the
  established pool-bound outer envelope contract.
- Oracle profile and snapshot adapters authenticate and rewrap only the outer
  server envelope, preserving their established pool/profile/generation/revision
  or pool/snapshot/version AAD. Inner browser payloads remain recipient-sealed.
- OAuth broker bindings preserve the established binding-hash AAD for supported
  older storage and bind the owner, client and exact field in contextual
  storage. A broker binding is a NyxID protocol credential, distinct from an
  external provider token.
- Agent Key login delivery binds the request, approving person, selected parent
  and issued child. Platform setup facts bind their row and owner; anonymous
  native setup also binds its server-keyed sender scope, and adoption must
  re-encrypt for the new owner in the same fenced transaction.
- The pending platform entry transport and native reply families must preserve
  their established digest AAD when their envelope implementations are
  completed.

## Header authority compatibility boundary

The header adapter uses storage metadata version **4** with the contextual v3
crypto envelope and encrypts every configured value. Its AAD binds collection,
record, effective owner, canonical case-insensitive header name and management
visibility. `sensitive` controls management masking, rather than whether a value
is encrypted. Explicit version 3 sensitive-only records retain their earlier
AAD. Unmarked plaintext is readable only under the legacy-reader policy.
Authentication failure in a selected encrypted format never retries another
format. Writing configured header values requires contextual writer activation;
there is no new plaintext-storage fallback.

The existing execution-authority v1/v2 `HeaderProjection` includes exact
plaintext header values. Storage encryption must preserve those serialized
values and digests; randomized ciphertext or replacement hashes cannot stand in
for the values.

Policy and ownership/ACL admission precede header authority decoding. The
bounded local decoder authenticates values using the trusted containing record
and canonical header identity solely to compute the existing read-only
authority projection. Owned plaintext buffers zeroize on drop. This path does
not issue upstream requests, refresh OAuth, exchange tokens or invoke generic
credential materialization. Normal credential materialization follows the
policy and approval gates, and the resolved execution authority is compared
again after materialization and before any provider effect.

Masked management edits preserve the live value under an exact-array or
raw-document compare-and-swap guard. A masked value cannot be renamed or made
public; those edits require an explicit replacement. Management input/output
types remain separate from server-owned encrypted storage metadata. Direct
HTTP, direct WebSocket, node HTTP and node WebSocket use the same authenticated
resolved header semantics. Header decoding or credential lookup failure stops
execution. Storage changes must retain both existing authority digests, and
policy-denial tests must prove that neither header decoding nor provider
materialization ran.

## Migration status and retirement contract

The explicit [migration API](../SECURITY.md#encrypted-record-migration) uses a
durable generation, lease and cursor, processes bounded batches, and records
per-field outcomes and durable findings. Rewrites compare the exact observed
source and the adapter's ownership/version fences, so a concurrent refresh,
replacement or transfer wins rather than being overwritten. Current-format
markers alone do not establish readiness: the winning material must
authenticate under the expected binding and selected primary provider.
Malformed, missing-key, binding and lifecycle failures remain unresolved until
repaired and rechecked. Removed-source and no-material observations are
reported separately from authenticated ciphertext.

The current status response exposes phase, current collection, scanned rows,
`rewrite_operations_observed`, unresolved findings, per-field outcomes and
blockers. Rewrite observations are not an exact total: a crash between a
successful source update and its progress record can omit that rewrite on replay.
`implementation_registry_incomplete` remains a blocker while any Pending
adapter exists. Backup/rollback-reader evidence, enabled legacy readers,
provider/registry changes, unfinished walks and unresolved findings also block
readiness. Completing a walk does not authorize key retirement. The complete
per-family format/key inventory and retirement report remain follow-up work.

Retirement readiness must account for every retained row, supported old
provider/key ID and legacy format, enabled legacy reader, backup dependency and
rollback binary. Short-lived families may use drain reporting only after all
writers/readers are converted, every writer's maximum lifetime is proved, and
contextual writes have been enabled for at least that lifetime. Drain checks
must include expired rows and rows awaiting purge; an expiry timestamp does not
remove a retained old-key dependency. The pending families do not currently
provide that completed drain-reporting contract.

AWS and GCP KMS may decrypt several internal key versions behind one configured
logical key ID. Successful authentication with the current logical ID does not
prove old internal versions have been retired. Operators need provider-specific
key-version and backup evidence; the runner cannot infer it from a successful
decrypt. Rewrap also cannot erase an attacker's earlier database copy. Audit and
billing history must not be mutated or re-signed as part of ciphertext migration.

Follow the [reader-first cutover procedure](../ENV.md#contextual-encryption-writer-cutover):
deploy compatible readers with contextual writes off, drain old writers, and
activate writes only with the explicit drained-writer acknowledgement. The
legacy-reader switch applies to converted adapters and does not retire
unconverted raw crypto call sites or backups. Turning it off requires a
completed authenticated inventory and the separate readiness evidence. This
inventory does not certify a production cutover or retirement.

## Erasure retry capabilities

`personal_erasure_effects.capability_encrypted` is a server-encrypted migration
target, including retained capabilities for pending or operator-action effects.
Its trusted context is the collection, exact effect/attempt `_id`, canonical
personal `user_id`, and `capability_encrypted` field. `target_owner_id` identifies
a destination, which may be a service account; it does not replace the personal
owner in the context.

The supported pre-cutover contract uses encrypted v2 storage with
length-delimited `nyxid:erasure-capability:v1` AAD. Activated writers use
contextual v3. Neither writes plaintext. Old deletion and credential writers
must be drained before activating the new erasure contract.

The adapter authenticates and validates retained material, re-encrypts under the
selected current primary provider and compares the exact source
ciphertext/version/owner and effect lease. It preserves effect identity, app
lineage, status, retry/reconciliation state and timestamps. An active lease or
concurrent source change causes a retry instead of an overwrite. Resolving an
effect removes its retained capability in the same terminal update. The
migration registry includes this adapter; its presence does not establish that
migration or retirement has completed.

## Remaining work

[Encryption migration and retirement work (#1934)](https://github.com/ChronoAIProject/NyxID/issues/1934)
includes completing these twelve Pending families: `assistant_agent_learning_proposals`,
`auth_device_codes`, `auth_device_codes_v2`, `device_codes`, `channel_bots`,
`nyxbot_channels`, `nyxbot_threads`, `nyxbot_events`, `nyxbot_watches`,
`oauth_clients`, `platform_entry_gateway_transports` and
`platform_native_reply_handles`. Each needs complete writer/reader conversion,
context and lifecycle validation, owner rebind where applicable, exact-source
migration or a proved drain contract, and swap/concurrency/round-trip regression
evidence. The same follow-up must deliver per-family readiness and
key-retirement reporting, including drain status and retained old-key, legacy
reader, rollback and backup dependencies.

Closing the unbound encryption API remains follow-up work for
[secret handling (#1933)](https://github.com/ChronoAIProject/NyxID/issues/1933) and
[encryption lifecycle (#1934)](https://github.com/ChronoAIProject/NyxID/issues/1934):
move the record-field API into the crypto module, make raw unbound
`EncryptionKeys` methods private to that module, and convert remaining callers
to the record-field or dedicated recipient-sealed API. Enforce the boundary with
compilation and a source scan. The existing rollout switches alone do not close
this API or prove every encrypted field is context-bound.
