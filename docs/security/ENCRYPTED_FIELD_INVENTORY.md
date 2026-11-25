# Server-encrypted field inventory and migration contract

This inventory mirrors the collection, field and adapter entries in the
[encrypted-record migration registry](../../backend/src/services/encrypted_record_migration.rs).
**Implemented** means the registry dispatches to a service-owned verifier and
migration adapter. It describes the source implementation; it does not establish
that a deployment has completed migration or is ready to retire a key.
**Pending** means the family lacks a complete registry verifier and migration
adapter. Some Pending families already have contextual writers and readers.
Pending adapters block complete migration readiness, including when their
collections are empty.

A canonical context binds the collection, record ID, canonical owner and exact
field identity without ambiguous concatenation.
New envelope metadata and context are authenticated. Authentication failure in a
selected contextual format is terminal and never retries an unbound decoder.
Supported older formats use their explicit storage contract and obey the
legacy-reader policy. Existing additional authenticated data (AAD) contracts
must retain their established dimensions during rewrap.

## Registered fields

The registry has 50 entries for 48 collections, with 42 implemented adapters
and 8 Pending entries. Each row lists the exact BSON paths for one adapter.
A collection may use separate adapters for different fields.

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
| `assistant_tool_results` | `data_encrypted` | [assistant_tool_result_service/storage.rs](../../backend/src/services/assistant_tool_result_service/storage.rs) | Implemented |
| `platform_public_sessions` | `credential_ciphertext`, `upstream_binding_ciphertext` | [platform_public_record_service.rs](../../backend/src/services/platform_public_record_service.rs) | Implemented |
| `platform_public_turns` | `input_ciphertext`, `reply_ciphertext` | [platform_public_record_service.rs](../../backend/src/services/platform_public_record_service.rs) | Implemented |
| `downstream_services` | `default_request_headers` | [default_header_service.rs](../../backend/src/services/default_header_service.rs) | Implemented |
| `user_services` | `default_request_headers` | [default_header_service.rs](../../backend/src/services/default_header_service.rs) | Implemented |
| `personal_erasure_effects` | `capability_encrypted` | [personal_erasure_service/provider.rs](../../backend/src/services/personal_erasure_service/provider.rs) | Implemented |
| `agent_key_login_requests` | `delivery_credential_encrypted` | [agent_login_record_service.rs](../../backend/src/services/agent_login_record_service.rs) | Implemented |
| `assistant_agent_learning_proposals` | `scope_encrypted`, `body_encrypted`, `publication.interface_encrypted` | [assistant_learning_record_storage.rs](../../backend/src/services/assistant_learning_record_storage.rs) | Implemented |
| `assistant_async_operations` | `result_encrypted` | [async_service_operation.rs](../../backend/src/services/async_service_operation.rs) | Implemented |
| `auth_device_codes` | `delivery_access_token_encrypted`, `delivery_refresh_token_encrypted` | — | Pending |
| `auth_device_codes_v2` | `agent_key_grant.delivery_credential_encrypted`, `delivery_access_token_encrypted`, `delivery_refresh_token_encrypted` | — | Pending |
| `device_codes` | `delivery_api_key_encrypted`, `delivery_refresh_token_encrypted` | — | Pending |
| `channel_bots` | `registration_pin_encrypted`, `webhook_secret_encrypted`, `bot_token_encrypted`, `app_secret_encrypted`, `lark_verification_token_encrypted`, `lark_encrypt_key_encrypted` | [channel_bot_record.rs](../../backend/src/services/channel_bot_record.rs) | Implemented |
| `channel_connect_links` | `receiver.secret_encrypted` | [channel_connect_link_service.rs](../../backend/src/services/channel_connect_link_service.rs) | Implemented |
| `downstream_services` | `credential_encrypted` | [catalog_credential_record_service.rs](../../backend/src/services/catalog_credential_record_service.rs) | Implemented |
| `downstream_services` | `ssh_config.ca_private_key_encrypted` | [ssh_ca_record_service.rs](../../backend/src/services/ssh_ca_record_service.rs) | Implemented |
| `nyxbot_channels` | `agent_key_ciphertext` | [nyxbot_channel_key.rs](../../backend/src/services/nyxbot_channel_key.rs) | Implemented |
| `nyxbot_threads` | `event_ref_ciphertext` | [Thread reference storage](../../backend/src/services/nyxbot_thread_reference.rs) | Implemented |
| `nyxbot_events` | `delivery.target_ciphertext`, `event_context_ciphertext`, `guest_setup_payload` | — | Pending |
| `mobile_social_deliveries` | `ciphertext` | — | Pending |
| `oauth_broker_bindings` | `refresh_token_encrypted` | [oauth_broker_record_service.rs](../../backend/src/services/oauth_broker_record_service.rs) | Implemented |
| `oauth_clients` | `revocation_webhook_secret_encrypted`, `connection_webhook_secret_encrypted` | [OAuth client webhooks](../../backend/src/services/oauth_client_webhook_record.rs) | Implemented |
| `oracle_login_profiles` | `encrypted_envelope` | [oracle_login_record_service.rs](../../backend/src/services/oracle_login_record_service.rs) | Implemented |
| `oracle_login_snapshots` | `encrypted_envelope` | [oracle_login_record_service.rs](../../backend/src/services/oracle_login_record_service.rs) | Implemented |
| `platform_entry_gateway_transports` | `request_ciphertext` | [platform_entry_gateway_service.rs](../../backend/src/services/platform_entry_gateway_service.rs) | Implemented |
| `platform_native_reply_handles` | `ciphertext` | — | Pending |
| `platform_setups` | `facts_ciphertext` | [platform_setup_record_service.rs](../../backend/src/services/platform_setup_record_service.rs) | Implemented |
| `triggers` | `verification_secret_encrypted`, `delivery_secret_encrypted` | [trigger_record_service.rs](../../backend/src/services/trigger_record_service.rs) | Implemented |
| `trigger_deliveries` | `envelope_encrypted` | [trigger_record_service.rs](../../backend/src/services/trigger_record_service.rs) | Implemented |
| `user_service_connections` | `credential_encrypted` | [connection_record_service.rs](../../backend/src/services/connection_record_service.rs) | Implemented |
| `async_service_operation_artifacts` | `data_encrypted` | [generated_artifacts/storage.rs](../../backend/src/services/generated_artifacts/storage.rs) | Implemented |
| `assistant_cma_sandboxes` | `request_ciphertext` | [assistant_cma_request_record.rs](../../backend/src/services/assistant_cma_request_record.rs) | Implemented |
| `assistant_cma_creations` | `request_ciphertext` | [assistant_cma_request_record.rs](../../backend/src/services/assistant_cma_request_record.rs) | Implemented |
| `nyxbot_event_bindings` | `provider_key_ciphertext`, `producer_secret_ciphertext` | [storage.rs](../../backend/src/services/nyxbot_event_binding_service/storage.rs) | Implemented |
| `nyxbot_gateway_responses` | `payload_ciphertext` | — | Pending |
| `channel_whatsapp_native` | `native_verify_token_encrypted` | [token_record.rs](../../backend/src/services/whatsapp_native/token_record.rs) | Implemented |
| `assistant_conversations` | `active_turn.restart_admission.window.input` | [Reattach window storage](../../backend/src/services/assistant_reattach_window_record.rs) | Implemented |
| `approval_requests` | `execution_proposal.v1.payload`, `execution_proposal.v1.result`, `execution_proposal.v2.payload`, `execution_proposal.v2.result`, `execution_proposal.v3.payload`, `execution_proposal.v3.result`, `execution_proposal.v4.payload`, `execution_proposal.v4.result` | [Approval proposal storage](../../backend/src/services/execution_permission_dispatch/proposals/record.rs) | Implemented |
| `channel_meta_oauth_sessions` | `assets_encrypted` | — | Pending |

OAuth client webhook signing keys use the registration ID, the persisted creator
and a separate field path for each webhook. System, dynamic and ownerless
registrations have distinct scopes. Compatible writers preserve legacy binary
storage. Contextual writers set the replaced field’s version atomically, and
disabling the connection webhook clears its material and marker. The migration
compares the exact observed client document before replacing stored material.

Channel bot writers bind each local credential to its bot ID, owner and field when
contextual writes are enabled. The adapter verifies the six fields and preserves
verification, native-entry, Discord resume and Aurinko pins across storage rewrites.
Telegram manager, connection-backed and exactly revoked/removed Meta bots can
retain an unmarked empty local token. The adapter preserves that retired sentinel;
strict readers never interpret it as a contextual envelope. Shared signing
producers prepare their dependent bots before rewrap. A real replacement starts a
new field generation. Owner transfer rebinds local credentials and retires the old
generations in the transaction that moves the bot and its routes.
Meta manual credential PATCH, OAuth installation/reconnect and confirmed
Instagram refresh use this same record writer. Disconnect and signed
deauthorization retire only the captured current grant's token, clear its
decoder marker and advance its credential generation. Meta's grant/owner
transaction also owns the raw BSON credential write and any newly captured
shared signer fence. Storage rewrap preserves the retained logical pins;
transfer authenticates and rebinds the current transaction's source row.
[Bot record adapter](../../backend/src/services/channel_bot_record.rs) · [Fingerprint selection](../../backend/src/services/channel_credential_continuity.rs)

Each CMA request copy uses its own stored collection, row ID, user ID and field.
The adapter checks the hash of the exact HTTP body and compares the full source
document before updating the envelope and its marker. An uncertain creation
retains its request and idempotency key through rewrap and recovery.
[CMA request storage](../../backend/src/services/assistant_cma_request_record.rs)

Learning proposal writers use the record-write switch for the body, scope and
publication interface. Each field binds the proposal, owner and agent. The
interface also binds its publication operation. Migration compares the exact
source document and preserves terminal body sentinels and publication leases.
[Proposal storage](../../backend/src/services/assistant_learning_record_storage.rs)

Approval payloads and results bind the approval ID, user and selected versioned
field path. Migration compares the complete source approval before replacing
retained fields. Dispatch claims and per-effect send receipts keep their own fences.

Reattach input binds the conversation, user, admitted credential and window key.
Recovery and continuation writers store the marker with the input. Rewrap
compares the full source conversation so worker takeover, window replacement,
credential rotation, stop and settlement retain their own fences.
[Window storage](../../backend/src/services/assistant_reattach_window_record.rs)

Gateway thread references bind the thread ID, user and channel. Rewrap preserves
the reference deadline and relay message. A matching relay binding is recomputed
with the new ciphertext in the same exact-source update.
[Thread reference storage](../../backend/src/services/nyxbot_thread_reference.rs)

### Production storage for Pending fields

- Mobile social sign-in stores and redeems its encrypted PKCE-bound response.
  [Mobile delivery](../../backend/src/services/mobile_social_service.rs)
- V2 device approval stores the credential inside the Agent Key grant. A session
  grant writes the access and refresh token fields at the top level.
  [Agent Key grants](../../backend/src/services/auth_device_service/grants.rs) · [Device sessions](../../backend/src/services/auth_device_service.rs)
- The NyxBot event holds its context, the late-delivery target and the queued guest setup payload.
  Gateway responses store an encrypted replay payload.
  [Event context](../../backend/src/handlers/nyxbot_native_responses.rs) · [Late delivery](../../backend/src/handlers/nyxbot_late_delivery.rs) · [Guest queue](../../backend/src/handlers/nyxbot_guest_setup_queue.rs) · [Replay](../../backend/src/services/nyxbot_response_recovery.rs)

NyxBot custom-event bindings retain a provider API key and a producer HMAC
secret. Each field binds to the binding ID, linking person, target agent and
its recorded owner, field name and corresponding key ID. Storage verification
checks the provider key against its independent API-key digest. For a legacy
producer secret, NyxID can check the original 32-byte base64 encoding. The
gateway receives the credentials during setup, and the owner receives the
producer secret once. Subsequent serving paths use binding metadata and live
authority. [Binding setup and cleanup](../../backend/src/services/nyxbot_event_binding_service.rs)

The NyxBot gateway agent key binds to the channel ID, linking person and
installed agent key ID. On an org route, the org owns the separate route key.
Direct-to-gateway preparation stores pending IDs. Adoption publishes the
ciphertext with its installed ID. Storage verification checks the matching
API key’s retained owner and SHA-256 digest, including for revoked keys.
[Channel key storage](../../backend/src/services/nyxbot_channel_key.rs)

The native WhatsApp Verify Token binds to the settings row’s bot ID, owner and
ownership generation. Callback transitions preserve it. Relay-phase settings
replacement clears it. Enrollment requires a matching live bot. Migration can
authenticate retained settings under the original owner after a transfer.
[WhatsApp enrollment and storage](../../backend/src/services/whatsapp_native/token_record.rs)

## Source and storage boundaries

Services resolve the trusted collection, row, owner and field components before
calling `crypto::record_field`. For binary fields with historically unbound
encryption, the writer uses the rollout setting and the reader dispatches on
the stored marker. Marker 3 requires authenticated v3. An absent marker permits
the historical decoder only while legacy reads are enabled. BSON decoding,
version-map validation and exact-source migration stay in services.
[Crypto API](../../backend/src/crypto/record_field.rs) · [Storage orchestration](../../backend/src/services/record_field_service.rs)

- The migration generation’s `provider_proof` authenticates its selected primary
  provider. The runner verifies it separately from the application field registry.
- Managed machine bootstrap seals for the recipient public key. The server
  stores that envelope in `managed_machine_bootstrap_deliveries` and the node
  opens it with its private key. For `node_pending_credentials` the server
  queues and forwards a sender-encrypted delivery. Recipient key lifecycle
  and expiry govern both protocols independently of server KEK rewrap.
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
  in the same fenced transfer transaction. Oracle pool transfer must preserve the
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
- Platform gateway setup binds the serialized request to the transport ID,
  credential owner and field. Replay and migration check the exact bytes
  against the retained SHA-256, including for historical unbound envelopes.
  Retirement retains the request and its hash. Native
  replies authenticate the binding digest bytes as AES AAD.
  [Gateway transport](../../backend/src/services/platform_entry_gateway_service.rs) · [Native reply reader](../../backend/src/services/platform_native_reply_service.rs)
- Mobile sign-in delivery authenticates the code hash, PKCE challenge and user.
  Execution proposals v1, v3 and v4 use `execution-proposal:v1:{id}`. V2 uses
  `execution-proposal:v2:{id}`. Each result appends `:result`.
  The assistant reattach window authenticates the serialized tuple of domain,
  user, conversation, credential ID and window key. These services own the
  encoding of their historical AAD.

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

After all serving replicas upgrade, start a fresh verification generation.
Status uses the stored registry digest to select field and collection labels.
For recognized historical registries it reports their original spellings and positions.
Digests with identical field names and positions share the same label mapping.
An unknown or missing digest returns aggregate totals, the mismatch blocker,
a null current collection and an empty field list. Resume, rewrap and finding
resolution require the live registry contract.

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
includes completing the Pending families in the registry table above. Each needs
complete writer/reader coverage, context and lifecycle validation, owner rebind
where applicable, exact-source
migration or a proved drain contract, and swap/concurrency/round-trip regression
evidence. The same follow-up must deliver per-family readiness and
key-retirement reporting, including drain status and retained old-key, legacy
reader, rollback and backup dependencies.

Closing the unbound encryption API remains follow-up work for
[secret handling (#1933)](https://github.com/ChronoAIProject/NyxID/issues/1933) and
[encryption lifecycle (#1934)](https://github.com/ChronoAIProject/NyxID/issues/1934):
make raw unbound
`EncryptionKeys` methods private to that module, and convert remaining callers
to the record-field or dedicated recipient-sealed API. Enforce the boundary with
compilation and a source scan. The existing rollout switches alone do not close
this API or prove every encrypted field is context-bound.


### Generated Sandbox files (#2037)

`async_service_operation_artifacts.data_encrypted` uses contextual v3 envelope AAD:
collection, pending record ID and owner, then `data_encrypted`, conversation,
originating turn, watch, operation and source-result identity. Only pending rows
retain ciphertext; materialized/expired/discarded/missing rows clear it and their
context-version map. `assistant_attachments.data_encrypted` with generated origin
uses the existing attachment/owner/conversation/completion-turn AAD plus originating
turn, watch, operation and source-result identity. Legacy attachment AAD is unchanged.
The selected metadata summary reuses `assistant_async_operations.result_encrypted`
and its existing adapter. Delivery/content-series metadata contains no ciphertext;
full source paths are never persisted.

The generated-artifact adapter handles pending files. Walks and privileged
lookups for these files and mixed attachments reserve bytes before fetching BSON and
process one large record per batch, retaining the budget through decrypt/rewrap/CAS.
