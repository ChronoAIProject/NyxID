# Server-encrypted field inventory and migration contract

This is the W4 implementation inventory, **not evidence that contextual migration has shipped or run**. The seed scan was expanded with nested structs/maps, encoded values and shared collections. Every row still requires a concrete context adapter, writer/reader conversion, bounded CAS migration and regression evidence before W4 completion. Source: [assessed plan](https://gist.github.com/ctkm-aelf/49087f48b677a312fa308e686503e2f5); current checkpoint in [W1–W4 tracker](../SECURITY_REMEDIATION_W1_W4.md).

A canonical context must encode collection, record ID, real polymorphic owner and exact field identity without ambiguous concatenation. New envelope metadata and context are authenticated. New-format authentication failure is terminal; it must never retry as legacy unbound data. Existing unbound v1/v2 formats require their exact legacy decoder followed by authenticated contextual write. Explicitly bound Oracle outer envelopes retain their established AAD through authenticated current-provider rewrap; OAuth/native adapters must preserve their own reviewed binding contracts. Supported legacy decoding is a controlled rollout dependency, not fallback after a new-format failure.

The table mirrors the actual migration registry. **Frozen16 has 19 Pending entries; frozen17 has 15 after async-result, channel-receiver and two Oracle outer-envelope implementations. Live source has 14 after the subsequent broker adapter; its compilation/runtime are pending.** An implemented adapter is not a production migration or retirement result. The [current tracker](../SECURITY_REMEDIATION_W1_W4.md) records independent runtime failures and source repairs separately from frozen evidence. Multiple entries for one collection deliberately use different adapters and contracts.

| Collection | Exact encrypted field paths | Service-owned adapter | Current source status |
|---|---|---|---|
| `user_provider_tokens` | `access_token_encrypted`, `refresh_token_encrypted`, `api_key_encrypted` | `credential_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `user_api_keys` | `credential_encrypted`, `access_token_encrypted`, `refresh_token_encrypted`, `user_oauth_client_id_encrypted`, `user_oauth_client_secret_encrypted` | `credential_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `user_provider_credentials` | `client_id_encrypted`, `client_secret_encrypted` | `credential_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `provider_configs` | `client_id_encrypted`, `client_secret_encrypted` | `credential_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `platform_credentials` | `secrets` | `platform_credential_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `nodes` | `signing_secret_encrypted` | `node_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `mfa_factors` | `secret_encrypted` | `mfa_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `saved_logins` | `username_encrypted`, `password_encrypted`, `totp_secret_encrypted` | `saved_login_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `oauth_states` | `code_verifier`, `device_code_encrypted`, `user_code_encrypted` | `oauth_state_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `assistant_agent_credentials` | `key_ciphertext` | `assistant_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `assistant_attachments` | `text_encrypted`, `data_encrypted` | `assistant_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `platform_public_sessions` | `credential_ciphertext`, `upstream_binding_ciphertext` | `platform_public_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `platform_public_turns` | `input_ciphertext`, `reply_ciphertext` | `platform_public_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `downstream_services` | `default_request_headers` | `default_header_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `user_services` | `default_request_headers` | `default_header_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `personal_erasure_effects` | `capability_encrypted` | `personal_erasure_service/provider.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `agent_key_login_requests` | `delivery_credential_encrypted` | — | Pending service-owned adapter/caller implementation. |
| `assistant_agent_learning_proposals` | `scope_encrypted`, `body_encrypted`, `publication.interface_encrypted` | — | Pending service-owned adapter/caller implementation. |
| `assistant_async_operations` | `result_encrypted` | `async_service_operation.rs` | Post16 source adapter; compilation and runtime pending. |
| `auth_device_codes` | `delivery_access_token_encrypted`, `delivery_refresh_token_encrypted` | — | Pending service-owned adapter/caller implementation. |
| `auth_device_codes_v2` | `delivery_credential_encrypted` | — | Pending service-owned adapter/caller implementation. |
| `device_codes` | `delivery_api_key_encrypted`, `delivery_refresh_token_encrypted` | — | Pending service-owned adapter/caller implementation. |
| `channel_bots` | `registration_pin_encrypted`, `webhook_secret_encrypted`, `bot_token_encrypted`, `app_secret_encrypted`, `lark_verification_token_encrypted`, `lark_encrypt_key_encrypted` | — | Pending service-owned adapter/caller implementation. |
| `channel_connect_links` | `receiver.secret_encrypted` | `channel_connect_link_service.rs` | Post16 source adapter; compilation and runtime pending. |
| `downstream_services` | `credential_encrypted` | `catalog_credential_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `downstream_services` | `ssh_config.ca_private_key_encrypted` | `ssh_ca_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `nyxbot_channels` | `agent_key_ciphertext` | — | Pending service-owned adapter/caller implementation. |
| `nyxbot_threads` | `event_ref_ciphertext` | — | Pending service-owned adapter/caller implementation. |
| `nyxbot_events` | `event_ref_ciphertext`, `event_context_ciphertext` | — | Pending service-owned adapter/caller implementation. |
| `nyxbot_watches` | `target_ciphertext` | — | Pending service-owned adapter/caller implementation. |
| `oauth_broker_bindings` | `refresh_token_encrypted` | `oauth_broker_record_service.rs` | Post17 source: binding-hash AAD compatibility while staged, contextual owner/client/field writes and actual exchange-reader conversion, exact raw CAS. Not external provider-token inventory; pending compilation/runtime. |
| `oauth_clients` | `revocation_webhook_secret_encrypted`, `connection_webhook_secret_encrypted` | — | Pending service-owned adapter/caller implementation. |
| `oracle_login_profiles` | `encrypted_envelope` | `OracleLoginOuter` | Existing pool/profile/generation/revision AAD retained; authenticates outer only, exact-CAS rewrap and expired-profile cleared state; actual reader converted. Source only, pending verification. |
| `oracle_login_snapshots` | `encrypted_envelope` | `OracleLoginOuter` | Existing pool/snapshot/version AAD retained; exact-CAS outer rewrap and actual worker-fetch validation. Inner recipient-sealed payload excluded from token-copy parsing. Source only, pending verification. |
| `platform_entry_gateway_transports` | `request_ciphertext` | — | Pending service-owned adapter/caller implementation. |
| `platform_native_reply_handles` | `ciphertext` | — | Pending service-owned adapter/caller implementation. |
| `platform_setups` | `facts_ciphertext` | — | Pending service-owned adapter/caller implementation. |
| `triggers` | `verification_secret_encrypted`, `delivery_secret_encrypted` | `trigger_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `trigger_deliveries` | `envelope_encrypted` | `trigger_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |
| `user_service_connections` | `credential_encrypted` | `connection_record_service.rs` | Implemented in frozen16; consult current verdict for runtime evidence and reported repairs. |

## Additional source and storage boundaries

- `managed_machine_bootstrap_deliveries` and `node_pending_credentials` contain recipient-sealed/E2E deliveries. They are not server-KEK migration targets. Expiry and recipient lifecycle continue separately.
- `assistant_upload.rs` re-exports `assistant_attachment::COLLECTION_NAME`: uploads are in `assistant_attachments`, never a fictitious `assistant_uploads` collection. Distinct upload/attachment variants need exact per-field handling.
- Service-layer BSON literals, embedded encrypted transport credentials, optional enum variants and encoded strings must be reconciled against all production encrypt/decrypt call sites before inventory is declared complete. A top-level field-name scan alone is insufficient.
- Shared provider/platform credentials belong to platform or explicit credential owner; `created_by`/`updated_by` identify an actor. Deleting that actor must not purge shared configuration.
- Context owner changes require authenticated rebind inside the same fenced transfer: nodes, downstream services, ChannelBot and X provider-token/key transfers. Oracle pool transfer must preserve pool-bound snapshot/profile formats and lifecycle.
- Header masked edits preserve the live secret with a version/CAS guard. Rename or sensitive-to-public changes cannot reveal an existing secret through a flag flip; require explicit replacement or preserve sensitive semantics. Direct HTTP, direct WebSocket and node HTTP/WebSocket consume the same resolved values. Credential decode/lookup failure must stop execution.

## Header authority compatibility boundary

The frozen execution-authority v1/v2 `HeaderProjection` includes the exact
plaintext header value. Storage encryption must preserve those serialized bytes
and their digests: randomized ciphertext and replacement hashes cannot stand in
for the value. Policy and ownership/ACL rejection must happen before header
decryption or provider work.

The draft compatibility adapter has one narrow pre-approval responsibility:
authenticate and decode at most the configured header limit locally, using the
trusted collection, record, owner and canonical case-insensitive header name,
solely to compute the existing read-only authority projection. Owned plaintext
buffers must zeroize on drop. This path must not issue an upstream request,
refresh OAuth, exchange tokens or invoke generic credential materialization.
Normal credential materialization remains after the policy and approval gates;
the existing comparison against resolved execution authority must run again
after materialization and before any provider effect. This distinction is an
implementation requirement, not a claim that the header adapter is complete.

Acceptance requires storage-only encryption to retain both frozen digests, and
a denied-policy regression to prove that neither the header decoder nor the
provider materializer ran. Management input/output types must remain separate
from server-owned encrypted storage metadata.

## Completion and retirement evidence

The runner must keep a durable lease, cursor and per-field outcome, process bounded batches, and compare-and-set the exact source blob plus ownership/version so it cannot overwrite concurrent replacement/refresh/transfer. A current-format prefix is not proof of readiness: authenticate/decrypt it and record malformed, missing-key and binding failures. Rerun after interruption and after concurrent writes; do not advance past unresolved failures without a durable retry path.

Retirement readiness must enumerate every supported old provider/key ID and legacy unbound format, plus backups and rollback readers. AWS/GCP KMS may decrypt multiple internal key versions behind one configured logical key ID; current logical-ID authentication does not prove old cloud internal versions retired. Operators need provider-specific key-version and backup dependency evidence; the runner cannot infer it from successful decrypts. Stage compatible readers on every replica before new-format writers or drain old writers. Rewrap cannot erase an attacker's old database copy. No audit/billing historic chain mutation or re-signing is authorized; W8 retention integrity remains separate.

The current header adapter is **v4** and encrypts every configured value. Its AAD binds collection, record, effective owner, canonical header name and management visibility. `sensitive` controls management masking, including after explicit public-value decoding; it does not select encryption. Explicit v3 sensitive-only records retain their exact earlier AAD, and unmarked plaintext is available only under the legacy reader rollout policy. New-format authentication failure never retries another format. Exact-array/raw-document CAS protects masked preservation and owner rebind; frozen authority digests continue to use authenticated plaintext semantics. Core transport and digest tests exist, while each checkpoint's execution evidence remains in the tracker.

Catalog master credentials bind effective owner (`owner_user_id`, falling back to `created_by` only for missing/null legacy ownership). Service transfer authenticates and rebinds the master and headers with the same exact-source owner mutation. SSH CA context deliberately remains platform/service/public-key bound across owner transfer. Empty lifecycle values must have no dangling context markers. Migration readiness authenticates actual winning material and reports removed/no-material observations separately from authenticated ciphertext.

### Erasure preparation source snapshots (draft adapter)

Dependency preparation stores keyed plaintext-token fingerprints and a digest of exact owner/app/source ciphertext metadata. It does **not** persist duplicate access/refresh/BYO ciphertext under `revocation_provenance.source`; that earlier unshipped draft has been removed. Writers must maintain or invalidate the derived source digest/dependency registry transactionally with credential replacement, refresh, ownership rebind and migration. Rewrap preserves authenticated plaintext fingerprints but changes the storage digest; recompute it in the same source CAS. The live token-holder backfill and writer integration remain incomplete and production activation is blocked. Preparation uses the legacy global fence before a complete matching index and a bounded owner walk afterward, with live token closure still required at dispatch. No key retirement is authorized.

### Remediation-owned retry capabilities

`personal_erasure_effects.capability_encrypted` is a server-encrypted migration
target, including pending revocations, uncertain issuance responses and effects
that require operator action. Its trusted context is the collection, exact
retained effect/attempt `_id`, canonical personal `user_id`, and
`capability_encrypted` field. `target_owner_id` records the destination (which
may be a service account); it must not replace canonical ownership in the
context. The pre-cutover storage contract is explicitly bound v2 with a
length-delimited `nyxid:erasure-capability:v1` AAD. Activated writers use v3.
Neither path writes plaintext. Old binaries ignore this dedicated collection;
they must be drained from deletion/credential writers for the erasure contract.

The field adapter authenticates and validates the preserved material, reencrypts
under the selected primary current provider, verifies primary-only current
record authentication, and CASes the exact source ciphertext/version/owner and
lease. It preserves effect ID, response fingerprint, app lineage, status,
retry/reconciliation identity and timestamps. An active effect lease or a
concurrent receipt/configuration replacement causes a retry, never an overwrite.
The global runner must visit this adapter before reporting retirement ready;
its existence alone does not mean that runner or retirement has completed.

Preparation stores fingerprints and source digests only; no retained preparation
source ciphertext is currently introduced. Any future retained encrypted
snapshot must be added here and to the runner before authoring it. Readonly
recipient E2E pending node deliveries remain excluded.
