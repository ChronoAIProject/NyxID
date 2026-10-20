//! Operator-driven, bounded storage verification/rewrap. This runner never
//! performs provider effects or retires a key. A completed walk is observation
//! evidence, not proof that backups or rollback binaries have been retired.
use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use mongodb::{Database, options::ReturnDocument};
use serde::{Deserialize, Serialize};

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
};

const RUNS: &str = "encrypted_record_migration_runs";
const FINDINGS: &str = "encrypted_record_migration_findings";
const RUN_ID: &str = "contextual-records-v1";
const BATCH: i64 = 16;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    Verify,
    Migrate,
}

#[derive(Clone, Copy, Debug)]
enum Adapter {
    Trigger,
    Connection,
    Credential,
    PlatformCredential,
    Node,
    Mfa,
    SavedLogin,
    OAuthState,
    AssistantCredential,
    Attachment,
    Public,
    Header,
    Capability,
    CatalogCredential,
    CatalogCa,
    AsyncResult,
    ChannelReceiver,
    OracleLoginOuter,
    Broker,
    Setup,
    AgentLogin,
    // These are implementation blockers until their service-owned decoders and
    // actual writers are converted. They may not be waived by operator input.
    Pending,
}

struct Entry {
    collection: &'static str,
    adapter: Adapter,
    fields: &'static [&'static str],
}

// A collection can have multiple adapters (e.g. catalog credential + headers).
// Array/map/nested paths are explicit; recipient-sealed node deliveries are
// deliberately absent because they cannot be decrypted by the server KEK.
const REGISTRY: &[Entry] = &[
    Entry {
        collection: "user_provider_tokens",
        adapter: Adapter::Credential,
        fields: &[
            "access_token_encrypted",
            "refresh_token_encrypted",
            "api_key_encrypted",
        ],
    },
    Entry {
        collection: "user_api_keys",
        adapter: Adapter::Credential,
        fields: &[
            "credential_encrypted",
            "access_token_encrypted",
            "refresh_token_encrypted",
            "user_oauth_client_id_encrypted",
            "user_oauth_client_secret_encrypted",
        ],
    },
    Entry {
        collection: "user_provider_credentials",
        adapter: Adapter::Credential,
        fields: &["client_id_encrypted", "client_secret_encrypted"],
    },
    Entry {
        collection: "provider_configs",
        adapter: Adapter::Credential,
        fields: &["client_id_encrypted", "client_secret_encrypted"],
    },
    Entry {
        collection: "platform_credentials",
        adapter: Adapter::PlatformCredential,
        fields: &["secrets"],
    },
    Entry {
        collection: "nodes",
        adapter: Adapter::Node,
        fields: &["signing_secret_encrypted"],
    },
    Entry {
        collection: "mfa_factors",
        adapter: Adapter::Mfa,
        fields: &["secret_encrypted"],
    },
    Entry {
        collection: "saved_logins",
        adapter: Adapter::SavedLogin,
        fields: &[
            "username_encrypted",
            "password_encrypted",
            "totp_secret_encrypted",
        ],
    },
    Entry {
        collection: "oauth_states",
        adapter: Adapter::OAuthState,
        fields: &[
            "code_verifier",
            "device_code_encrypted",
            "user_code_encrypted",
        ],
    },
    Entry {
        collection: "assistant_agent_credentials",
        adapter: Adapter::AssistantCredential,
        fields: &["key_ciphertext"],
    },
    Entry {
        collection: "assistant_attachments",
        adapter: Adapter::Attachment,
        fields: &["text_encrypted", "data_encrypted"],
    },
    Entry {
        collection: "platform_public_sessions",
        adapter: Adapter::Public,
        fields: &["credential_ciphertext", "upstream_binding_ciphertext"],
    },
    Entry {
        collection: "platform_public_turns",
        adapter: Adapter::Public,
        fields: &["input_ciphertext", "reply_ciphertext"],
    },
    Entry {
        collection: "downstream_services",
        adapter: Adapter::Header,
        fields: &["default_request_headers"],
    },
    Entry {
        collection: "user_services",
        adapter: Adapter::Header,
        fields: &["default_request_headers"],
    },
    Entry {
        collection: "personal_erasure_effects",
        adapter: Adapter::Capability,
        fields: &["capability_encrypted"],
    },
    Entry {
        collection: "agent_key_login_requests",
        adapter: Adapter::AgentLogin,
        fields: &["delivery_credential_encrypted"],
    },
    Entry {
        collection: "assistant_agent_learning_proposals",
        adapter: Adapter::Pending,
        fields: &[
            "scope_encrypted",
            "body_encrypted",
            "publication.interface_encrypted",
        ],
    },
    Entry {
        collection: "assistant_async_operations",
        adapter: Adapter::AsyncResult,
        fields: &["result_encrypted"],
    },
    Entry {
        collection: "auth_device_codes",
        adapter: Adapter::Pending,
        fields: &[
            "delivery_access_token_encrypted",
            "delivery_refresh_token_encrypted",
        ],
    },
    Entry {
        collection: "auth_device_codes_v2",
        adapter: Adapter::Pending,
        fields: &["delivery_credential_encrypted"],
    },
    Entry {
        collection: "device_codes",
        adapter: Adapter::Pending,
        fields: &[
            "delivery_api_key_encrypted",
            "delivery_refresh_token_encrypted",
        ],
    },
    Entry {
        collection: "channel_bots",
        adapter: Adapter::Pending,
        fields: &[
            "registration_pin_encrypted",
            "webhook_secret_encrypted",
            "bot_token_encrypted",
            "app_secret_encrypted",
            "lark_verification_token_encrypted",
            "lark_encrypt_key_encrypted",
        ],
    },
    Entry {
        collection: "channel_connect_links",
        adapter: Adapter::ChannelReceiver,
        fields: &["receiver.secret_encrypted"],
    },
    Entry {
        collection: "downstream_services",
        adapter: Adapter::CatalogCredential,
        fields: &["credential_encrypted"],
    },
    Entry {
        collection: "downstream_services",
        adapter: Adapter::CatalogCa,
        fields: &["ssh_config.ca_private_key_encrypted"],
    },
    Entry {
        collection: "nyxbot_channels",
        adapter: Adapter::Pending,
        fields: &["agent_key_ciphertext"],
    },
    Entry {
        collection: "nyxbot_threads",
        adapter: Adapter::Pending,
        fields: &["event_ref_ciphertext"],
    },
    Entry {
        collection: "nyxbot_events",
        adapter: Adapter::Pending,
        fields: &["event_ref_ciphertext", "event_context_ciphertext"],
    },
    Entry {
        collection: "nyxbot_watches",
        adapter: Adapter::Pending,
        fields: &["target_ciphertext"],
    },
    Entry {
        collection: "oauth_broker_bindings",
        adapter: Adapter::Broker,
        fields: &["refresh_token_encrypted"],
    },
    Entry {
        collection: "oauth_clients",
        adapter: Adapter::Pending,
        fields: &[
            "revocation_webhook_secret_encrypted",
            "connection_webhook_secret_encrypted",
        ],
    },
    Entry {
        collection: "oracle_login_profiles",
        adapter: Adapter::OracleLoginOuter,
        fields: &["encrypted_envelope"],
    },
    Entry {
        collection: "oracle_login_snapshots",
        adapter: Adapter::OracleLoginOuter,
        fields: &["encrypted_envelope"],
    },
    Entry {
        collection: "platform_entry_gateway_transports",
        adapter: Adapter::Pending,
        fields: &["request_ciphertext"],
    },
    Entry {
        collection: "platform_native_reply_handles",
        adapter: Adapter::Pending,
        fields: &["ciphertext"],
    },
    Entry {
        collection: "platform_setups",
        adapter: Adapter::Setup,
        fields: &["facts_ciphertext"],
    },
    Entry {
        collection: "triggers",
        adapter: Adapter::Trigger,
        fields: &["verification_secret_encrypted", "delivery_secret_encrypted"],
    },
    Entry {
        collection: "trigger_deliveries",
        adapter: Adapter::Trigger,
        fields: &["envelope_encrypted"],
    },
    Entry {
        collection: "user_service_connections",
        adapter: Adapter::Connection,
        fields: &["credential_encrypted"],
    },
];

/// Safe control-plane report: no database cursors, record IDs, ciphertext,
/// provider selectors, plaintext or raw decrypt/SDK errors are serialized.
#[derive(Serialize)]
pub struct Report {
    pub generation: String,
    pub mode: Mode,
    pub phase: String,
    pub collection: Option<&'static str>,
    pub scanned: i64,
    /// Durably recorded successful rewrite observations. A crash after the
    /// source CAS and before its checkpoint can omit that rewrite on replay.
    #[serde(rename = "rewrite_operations_observed")]
    pub rewritten: i64,
    pub unresolved: i64,
    pub blockers: Vec<&'static str>,
    pub field_outcomes: Vec<FieldOutcome>,
}

#[derive(Serialize)]
pub struct FieldOutcome {
    pub collection: &'static str,
    pub field: &'static str,
    pub authenticated_current: i64,
    pub no_material_observed: i64,
    pub source_removed_observed: i64,
    pub verification_blocked: i64,
}

fn field_outcomes(row: &Document) -> Vec<FieldOutcome> {
    let summaries = row.get_document("field_outcomes").ok();
    REGISTRY
        .iter()
        .enumerate()
        .flat_map(|(index, entry)| {
            entry
                .fields
                .iter()
                .enumerate()
                .map(move |(field_index, field)| {
                    let summary = summaries
                        .and_then(|values| values.get_document(index.to_string()).ok())
                        .and_then(|values| values.get_document(field_index.to_string()).ok());
                    let count = |kind| {
                        summary
                            .and_then(|value| value.get_i64(kind).ok())
                            .unwrap_or(0)
                    };
                    FieldOutcome {
                        collection: entry.collection,
                        field,
                        authenticated_current: count("authenticated_current"),
                        no_material_observed: count("no_material_observed"),
                        source_removed_observed: count("source_removed_observed"),
                        verification_blocked: count("verification_blocked"),
                    }
                })
        })
        .collect()
}

fn invalid() -> AppError {
    AppError::Internal("Invalid encrypted migration checkpoint".into())
}
fn lost() -> AppError {
    AppError::Conflict("Encrypted migration lease changed; resume the retained generation".into())
}

fn registry_digest() -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    for entry in REGISTRY {
        hash.update(entry.collection.as_bytes());
        hash.update([0]);
        hash.update(entry_contract(entry).as_bytes());
        hash.update([0]);
        for field in entry.fields {
            hash.update(field.as_bytes());
            hash.update([0]);
        }
    }
    hex::encode(hash.finalize())
}

/// Domain identifiers are exposed only for reviewed collections. Protocol state
/// handles remain opaque even when their storage representation is a UUID.
fn exposes_record_id(entry: &Entry) -> bool {
    matches!(
        entry.collection,
        "user_provider_tokens"
            | "user_api_keys"
            | "user_provider_credentials"
            | "provider_configs"
            | "platform_credentials"
            | "nodes"
            | "mfa_factors"
            | "saved_logins"
            | "assistant_agent_credentials"
            | "assistant_attachments"
            | "downstream_services"
            | "user_services"
    )
}

fn entry_contract(entry: &Entry) -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(b"encrypted-migration-entry-v2\0");
    hash.update(entry.collection.as_bytes());
    hash.update([0]);
    hash.update(format!("{:?}", entry.adapter).as_bytes());
    hash.update([u8::from(exposes_record_id(entry))]);
    for field in entry.fields {
        hash.update([0]);
        hash.update(field.as_bytes());
    }
    hex::encode(hash.finalize())
}

fn finding_entry(finding: &Document) -> AppResult<(usize, &'static Entry)> {
    let index =
        usize::try_from(finding.get_i32("entry").map_err(|_| invalid())?).map_err(|_| invalid())?;
    let entry = REGISTRY.get(index).ok_or_else(invalid)?;
    if finding.get_str("registry_digest").ok() != Some(registry_digest().as_str())
        || finding.get_str("entry_contract").ok() != Some(entry_contract(entry).as_str())
        || finding.get_str("collection").ok() != Some(entry.collection)
        || finding.get_array("fields").ok()
            != Some(
                &entry
                    .fields
                    .iter()
                    .map(|field| Bson::String((*field).into()))
                    .collect(),
            )
    {
        return Err(AppError::Conflict("Encrypted field registry changed; run a new verification walk before resolving findings".into()));
    }
    Ok((index, entry))
}

async fn verify_generation(keys: &EncryptionKeys, row: &Document) -> AppResult<()> {
    if row.get_str("registry_digest").ok() != Some(registry_digest().as_str()) {
        return Err(AppError::Conflict(
            "Encrypted field registry changed; start a new verification generation".into(),
        ));
    }
    let generation = row.get_str("generation").map_err(|_| invalid())?;
    keys.authenticate_current_record(
        row.get_binary_generic("provider_proof")
            .map_err(|_| invalid())?,
        &crate::crypto::aes::RecordContext::new(RUNS, generation, "platform", &["provider_proof"])?,
    )
    .await
}

pub async fn status(db: &Database, keys: &EncryptionKeys) -> AppResult<Option<Report>> {
    let db = super::transaction_service::durable_database(db);
    let Some(row) = db
        .collection::<Document>(RUNS)
        .find_one(doc! {"_id":RUN_ID})
        .await?
    else {
        return Ok(None);
    };
    let mut blockers = vec!["backup_and_rollback_reader_evidence_required"];
    if REGISTRY
        .iter()
        .any(|entry| matches!(entry.adapter, Adapter::Pending))
    {
        blockers.push("implementation_registry_incomplete");
    }
    if verify_generation(keys, &row).await.is_err() {
        blockers.push("registry_or_primary_provider_changed");
    }
    if keys.key_version_retirement_evidence() == crate::crypto::key_provider::KeyVersionRetirementEvidence::OperatorUnverifiedInternalVersions {
        blockers.push("cloud_internal_key_versions_operator_unverified");
    }
    if !keys.contextual_record_writes() {
        blockers.push("contextual_writers_not_activated");
    }
    if keys.allow_legacy_record_reads() {
        blockers.push("legacy_reader_dependency_enabled");
    }
    let phase = row.get_str("phase").map_err(|_| invalid())?.to_owned();
    if phase != "observed" {
        blockers.push("walk_incomplete");
    }
    let unresolved = row.get_i64("unresolved").map_err(|_| invalid())?;
    if unresolved > 0 {
        blockers.push("unresolved_storage_findings");
    }
    let mode: Mode =
        bson::from_bson(row.get("mode").cloned().ok_or_else(invalid)?).map_err(|_| invalid())?;
    let index = row.get_i32("entry").map_err(|_| invalid())?;
    Ok(Some(Report {
        generation: row.get_str("generation").map_err(|_| invalid())?.into(),
        mode,
        phase,
        collection: usize::try_from(index)
            .ok()
            .and_then(|index| REGISTRY.get(index))
            .map(|entry| entry.collection),
        scanned: row.get_i64("scanned").map_err(|_| invalid())?,
        rewritten: row.get_i64("rewritten").map_err(|_| invalid())?,
        unresolved,
        blockers,
        field_outcomes: field_outcomes(&row),
    }))
}

/// Starting a new generation is explicit. Old unresolved findings are retained;
/// a subsequent complete pass rechecks their collections without starving rows
/// behind a corrupt first record. No automatic activation or infinite retry.
pub async fn start(
    db: &Database,
    keys: &EncryptionKeys,
    mode: Mode,
    restart_generation: Option<&str>,
) -> AppResult<String> {
    if !keys.contextual_record_writes() {
        return Err(AppError::Conflict(
            "Contextual writer cutover is required before migration verification".into(),
        ));
    }
    let db = super::transaction_service::durable_database(db);
    db.collection::<Document>(FINDINGS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"generation":1,"_id":1})
                .build(),
        )
        .await?;
    let previous = db
        .collection::<Document>(RUNS)
        .find_one(doc! {"_id":RUN_ID})
        .await?;
    if previous.as_ref().is_some_and(|row| {
        row.get_str("phase").ok() == Some("running")
            && row.get_str("generation").ok() != restart_generation
    }) {
        return Err(AppError::Conflict(
            "Resume the existing encrypted migration generation first".into(),
        ));
    }
    let generation = uuid::Uuid::new_v4().to_string();
    let proof = keys
        .encrypt_record(
            b"nyxid encrypted inventory provider proof",
            &crate::crypto::aes::RecordContext::new(
                RUNS,
                &generation,
                "platform",
                &["provider_proof"],
            )?,
        )
        .await?;
    let row = doc! {"_id":RUN_ID,"generation":&generation,"mode":bson::to_bson(&mode).map_err(|_|invalid())?,
    "phase":"running","registry_digest":registry_digest(),"provider_proof":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:proof},
        "entry":0_i32,"cursor":Bson::Null,"scanned":0_i64,"rewritten":0_i64,"unresolved":0_i64,"field_outcomes":doc!{},
    "lease_token":Bson::Null,"lease_until":bson::DateTime::from_millis(0),"started_at":bson::DateTime::now()};
    match previous {
        Some(previous) => {
            let predicate = doc! {"$and":[super::record_storage::exact_document_predicate(&previous),doc!{"$expr":{"$lte":["$lease_until","$$NOW"]}}]};
            if db
                .collection::<Document>(RUNS)
                .replace_one(predicate, row)
                .await?
                .matched_count
                != 1
            {
                return Err(lost());
            }
        }
        None => {
            db.collection::<Document>(RUNS).insert_one(row).await?;
        }
    }
    Ok(generation)
}

async fn verify(
    entry: &Entry,
    db: &Database,
    keys: &EncryptionKeys,
    row: &Document,
) -> AppResult<()> {
    match entry.adapter {
        Adapter::Trigger => {
            super::trigger_record_service::verify_current_snapshot(keys, entry.collection, row)
                .await
        }
        Adapter::Connection => {
            super::connection_record_service::verify_current_snapshot(keys, row).await
        }

        Adapter::Credential => {
            super::credential_record_service::verify_current_snapshot(keys, entry.collection, row)
                .await
        }
        Adapter::PlatformCredential => {
            super::platform_credential_service::verify_current_storage_snapshot(keys, row).await
        }
        Adapter::Node => {
            let node: crate::models::node::Node =
                bson::from_document(row.clone()).map_err(|_| invalid())?;
            if node.signing_secret_encrypted.is_none()
                && node.signing_secret_context_version.is_none()
            {
                return Ok(());
            }
            super::node_service::verify_current_signing_secret(keys, &node).await
        }
        Adapter::Mfa => {
            let factor: crate::models::mfa_factor::MfaFactor =
                bson::from_document(row.clone()).map_err(|_| invalid())?;
            if factor.factor_type == "recovery_codes"
                && factor.secret_encrypted.is_none()
                && factor.secret_context_version.is_none()
            {
                return Ok(());
            }
            super::mfa_service::verify_current_secret(keys, &factor).await
        }
        Adapter::SavedLogin => {
            super::saved_login_service::verify_current_secrets(
                keys,
                &bson::from_document(row.clone()).map_err(|_| invalid())?,
            )
            .await
        }
        Adapter::OAuthState => {
            super::oauth_state_record_service::verify_current_snapshot(keys, row).await
        }
        Adapter::AssistantCredential => {
            super::assistant_record_service::verify_current_credential(db, keys, row).await
        }
        Adapter::Attachment => {
            super::assistant_record_service::verify_current_attachment(db, keys, row).await
        }
        Adapter::Public => {
            super::platform_public_record_service::verify_current_snapshot(
                db,
                keys,
                entry.collection,
                row,
            )
            .await
        }
        Adapter::Header => {
            super::default_header_service::verify_current_snapshot(
                keys,
                entry.collection,
                row,
                header_owner(entry, row)?,
            )
            .await
        }
        Adapter::Capability => {
            super::personal_erasure_service::provider::verify_current_capability_snapshot(keys, row)
                .await
        }
        Adapter::CatalogCredential => {
            super::catalog_credential_record_service::verify_current_snapshot(keys, row).await
        }
        Adapter::CatalogCa => {
            super::ssh_ca_record_service::verify_current_snapshot(keys, row).await
        }
        Adapter::AsyncResult => {
            super::async_service_operation::verify_current_result_snapshot(keys, row).await
        }
        Adapter::ChannelReceiver => {
            super::channel_connect_link_service::verify_current_receiver_snapshot(keys, row).await
        }
        Adapter::OracleLoginOuter => {
            super::oracle_login_record_service::verify_current_snapshot(keys, entry.collection, row)
                .await
        }
        Adapter::Broker => {
            super::oauth_broker_record_service::verify_current_snapshot(keys, row).await
        }
        Adapter::Setup => {
            super::platform_setup_record_service::verify_current_snapshot(keys, row).await
        }
        Adapter::AgentLogin => {
            super::agent_login_record_service::verify_current_snapshot(keys, row).await
        }
        Adapter::Pending => Err(AppError::Conflict(
            "Encrypted field adapter implementation is pending".into(),
        )),
    }
}

fn header_owner<'a>(entry: &Entry, row: &'a Document) -> AppResult<&'a str> {
    let owner = if entry.collection == "downstream_services" {
        match row.get("owner_user_id") {
            None | Some(Bson::Null) => row.get_str("created_by").map_err(|_| invalid())?,
            Some(Bson::String(owner)) => owner.as_str(),
            Some(_) => return Err(invalid()),
        }
    } else {
        row.get_str("user_id").map_err(|_| invalid())?
    };
    if owner.is_empty() {
        return Err(invalid());
    }
    Ok(owner)
}

async fn migrate(
    entry: &Entry,
    db: &Database,
    keys: &EncryptionKeys,
    row: &Document,
) -> AppResult<bool> {
    match entry.adapter {
        Adapter::Trigger => {
            super::trigger_record_service::migrate_snapshot(db, keys, entry.collection, row).await
        }
        Adapter::Connection => {
            super::connection_record_service::migrate_snapshot(db, keys, row).await
        }

        Adapter::Credential => {
            super::credential_record_service::migrate_snapshot(db, keys, entry.collection, row)
                .await
        }
        Adapter::PlatformCredential => {
            super::platform_credential_service::migrate_storage_snapshot(db, keys, row).await
        }
        Adapter::Node => Ok(matches!(
            super::node_service::migrate_signing_secret_snapshot(db, keys, row).await?,
            super::node_service::SecretMigrationOutcome::Rewritten
        )),
        Adapter::Mfa => super::mfa_service::migrate_secret_snapshot(db, keys, row).await,
        Adapter::SavedLogin => {
            super::saved_login_service::migrate_secret_snapshot(db, keys, row).await
        }
        Adapter::OAuthState => {
            super::oauth_state_record_service::migrate_snapshot(db, keys, row).await
        }
        Adapter::AssistantCredential => {
            super::assistant_record_service::migrate_credential_snapshot(db, keys, row).await
        }
        Adapter::Attachment => {
            super::assistant_record_service::migrate_attachment_snapshot(db, keys, row).await
        }
        Adapter::Public => {
            super::platform_public_record_service::migrate_snapshot(db, keys, entry.collection, row)
                .await
        }
        Adapter::Header => {
            super::default_header_service::migrate_snapshot(
                db,
                keys,
                entry.collection,
                row,
                header_owner(entry, row)?,
            )
            .await
        }
        Adapter::Capability => {
            super::personal_erasure_service::provider::rewrap_capability_snapshot(db, keys, row)
                .await
        }
        Adapter::CatalogCredential => {
            super::catalog_credential_record_service::migrate_snapshot(db, keys, row).await
        }
        Adapter::CatalogCa => super::ssh_ca_record_service::migrate_snapshot(db, keys, row).await,
        Adapter::AsyncResult => {
            super::async_service_operation::migrate_result_snapshot(db, keys, row).await
        }
        Adapter::ChannelReceiver => {
            super::channel_connect_link_service::migrate_receiver_snapshot(db, keys, row).await
        }
        Adapter::OracleLoginOuter => {
            super::oracle_login_record_service::migrate_snapshot(db, keys, entry.collection, row)
                .await
        }
        Adapter::Broker => {
            super::oauth_broker_record_service::migrate_snapshot(db, keys, row).await
        }
        Adapter::Setup => {
            super::platform_setup_record_service::migrate_snapshot(db, keys, row).await
        }
        Adapter::AgentLogin => {
            super::agent_login_record_service::migrate_snapshot(db, keys, row).await
        }
        Adapter::Pending => Err(AppError::Conflict(
            "Encrypted field adapter implementation is pending".into(),
        )),
    }
}

fn path<'a>(row: &'a Document, field: &str) -> Option<&'a Bson> {
    let mut components = field.split('.');
    let mut value = row.get(components.next()?)?;
    for component in components {
        value = value.as_document()?.get(component)?;
    }
    Some(value)
}
fn field_has_material(row: &Document, field: &str) -> bool {
    path(row, field).is_some_and(|value| match value {
        Bson::Null => false,
        Bson::Document(map) => !map.is_empty(),
        Bson::Array(array) => !array.is_empty(),
        Bson::Binary(bytes) => !bytes.bytes.is_empty(),
        Bson::String(value) => !value.is_empty(),
        _ => true,
    })
}
fn has_material(entry: &Entry, row: &Document) -> bool {
    entry
        .fields
        .iter()
        .any(|field| field_has_material(row, field))
}

fn diagnostic_id(index: usize, id: &Bson) -> AppResult<String> {
    #[derive(Serialize)]
    struct Identity<'a> {
        entry: i32,
        id: &'a Bson,
    }
    let encoded = zeroize::Zeroizing::new(
        bson::to_vec(&Identity {
            entry: index as i32,
            id,
        })
        .map_err(|_| invalid())?,
    );
    super::audit_service::keyed_fingerprint(
        super::audit_service::audit_chain_hmac_key(),
        b"encrypted-migration-row-v1",
        &encoded,
    )
    .ok_or_else(|| AppError::Internal("Migration evidence key is unavailable".into()))
}

#[derive(Serialize)]
pub struct FindingLookup {
    pub finding_id: String,
    pub collection: &'static str,
    pub fields: &'static [&'static str],
    pub disposition: &'static str,
    /// Reviewed non-capability domain identifiers only. UUID syntax alone
    /// cannot authorize disclosure of a protocol state/delivery handle.
    pub record_uuid: Option<uuid::Uuid>,
    pub owner_uuid: Option<uuid::Uuid>,
    pub next: Option<String>,
}

#[derive(Serialize)]
pub struct FindingSummary {
    pub id: String,
    pub generation: uuid::Uuid,
    pub collection: &'static str,
    pub fields: &'static [&'static str],
    pub category: &'static str,
}
#[derive(Serialize)]
pub struct FindingPage {
    pub findings: Vec<FindingSummary>,
    pub next: Option<String>,
}

pub async fn list_findings(db: &Database, after: Option<&str>) -> AppResult<FindingPage> {
    let mut filter = doc! {};
    if let Some(after) = after {
        if after.len() != 64 || !after.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(AppError::ValidationError(
                "Invalid migration finding cursor".into(),
            ));
        }
        filter.insert("_id", doc! {"$gt":after});
    }
    let db = super::transaction_service::durable_database(db);
    let rows = db
        .collection::<Document>(FINDINGS)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(BATCH)
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    let mut findings = Vec::with_capacity(rows.len());
    for row in rows {
        let (_, entry) = finding_entry(&row)?;
        let id = row.get_str("_id").map_err(|_| invalid())?;
        if id.len() != 64 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid());
        }
        let category = match row.get_str("category").ok() {
            Some("adapter_not_implemented") => "adapter_not_implemented",
            Some("authentication_binding_or_lifecycle_unresolved") => {
                "authentication_binding_or_lifecycle_unresolved"
            }
            _ => "finding_metadata_invalid",
        };
        findings.push(FindingSummary {
            id: id.into(),
            generation: uuid::Uuid::parse_str(row.get_str("generation").map_err(|_| invalid())?)
                .map_err(|_| invalid())?,
            collection: entry.collection,
            fields: entry.fields,
            category,
        });
    }
    let next = (findings.len() == BATCH as usize).then(|| findings.last().unwrap().id.clone());
    Ok(FindingPage { findings, next })
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LookupCursor {
    after: zeroize::Zeroizing<String>,
    expires_at_ms: i64,
}

/// Bounded privileged resolution of a keyed diagnostic. The client receives
/// safe domain identifiers or an encrypted short-lived continuation, never a
/// nonce-bearing database selector. An explicit recheck can invoke the same
/// storage-only adapter; it cannot clear the full-pass readiness requirement.
pub async fn lookup_finding(
    db: &Database,
    keys: &EncryptionKeys,
    finding_id: &str,
    after: Option<&str>,
    rewrap: bool,
) -> AppResult<FindingLookup> {
    if finding_id.len() != 64 || !finding_id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(AppError::ValidationError(
            "Invalid migration finding".into(),
        ));
    }
    let db = super::transaction_service::durable_database(db);
    let finding = db
        .collection::<Document>(FINDINGS)
        .find_one(doc! {"_id":finding_id})
        .await?
        .ok_or_else(|| AppError::NotFound("Migration finding not found".into()))?;
    let (index, entry) = finding_entry(&finding)?;
    let generation = finding.get_str("generation").map_err(|_| invalid())?;
    let components = ["lookup_cursor", generation];
    let context =
        crate::crypto::aes::RecordContext::new(FINDINGS, finding_id, "platform", &components)?;
    let mut filter = doc! {};
    if let Some(after) = after {
        if after.len() > 4096 {
            return Err(invalid());
        }
        let ciphertext = hex::decode(after).map_err(|_| invalid())?;
        let bytes = keys.decrypt_record(&ciphertext, &context).await?;
        let cursor: LookupCursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if cursor.expires_at_ms < chrono::Utc::now().timestamp_millis() {
            return Err(AppError::Conflict("Migration lookup cursor expired".into()));
        }
        filter.insert("_id", doc! {"$gt":cursor.after.as_str()});
    }
    let rows = db
        .collection::<Document>(entry.collection)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(BATCH)
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    let mut result = FindingLookup {
        finding_id: finding_id.into(),
        collection: entry.collection,
        fields: entry.fields,
        disposition: "lookup_incomplete",
        record_uuid: None,
        owner_uuid: None,
        next: None,
    };
    for row in &rows {
        let id = row.get("_id").ok_or_else(invalid)?;
        let candidate = diagnostic_id(index, id)?;
        if candidate != finding_id {
            continue;
        }
        if exposes_record_id(entry) {
            result.record_uuid = id
                .as_str()
                .and_then(|value| uuid::Uuid::parse_str(value).ok());
        }
        result.owner_uuid = ["user_id", "owner_id", "sponsor_owner_user_id"]
            .iter()
            .find_map(|field| {
                row.get_str(*field)
                    .ok()
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
            });
        result.disposition = if verify(entry, &db, keys, row).await.is_ok() {
            "authenticated_current_new_pass_required"
        } else if rewrap && !matches!(entry.adapter, Adapter::Pending) {
            match migrate(entry, &db, keys, row).await {
                Ok(_) => match db
                    .collection::<Document>(entry.collection)
                    .find_one(doc! {"_id":id})
                    .await?
                {
                    Some(current) if verify(entry, &db, keys, &current).await.is_ok() => {
                        "authenticated_current_new_pass_required"
                    }
                    None => "source_removed_new_pass_required",
                    _ => "storage_or_lifecycle_repair_required",
                },
                Err(AppError::DatabaseError(error)) => return Err(AppError::DatabaseError(error)),
                Err(_) => "storage_or_lifecycle_repair_required",
            }
        } else if matches!(entry.adapter, Adapter::Pending) {
            "adapter_implementation_required"
        } else {
            "storage_or_lifecycle_repair_required"
        };
        return Ok(result);
    }
    if rows.len() == BATCH as usize {
        let after = rows
            .last()
            .and_then(|row| row.get_str("_id").ok())
            .ok_or_else(invalid)?;
        let cursor = LookupCursor {
            after: after.to_owned().into(),
            expires_at_ms: chrono::Utc::now().timestamp_millis() + 900_000,
        };
        let plain = crate::crypto::secret_json::to_secret_string(&cursor).map_err(|_| invalid())?;
        result.next = Some(hex::encode(
            keys.encrypt_record(plain.as_bytes(), &context).await?,
        ));
    } else {
        result.disposition = "source_absent_new_pass_required";
    }
    Ok(result)
}

/// One bounded keyset page with a server-clock lease. A storage CAS can finish
/// after lease loss, but cannot overwrite a successor. A stale worker cannot
/// advance progress or report success. Database failure retains the cursor;
/// malformed material records a finite finding and does not starve later rows.
pub async fn advance(db: &Database, keys: &EncryptionKeys, generation: &str) -> AppResult<()> {
    let db = super::transaction_service::durable_database(db);
    let token = uuid::Uuid::new_v4().to_string();
    let run=db.collection::<Document>(RUNS).find_one_and_update(
        doc!{"_id":RUN_ID,"generation":generation,"phase":"running","$expr":{"$lte":["$lease_until","$$NOW"]}},
        vec![doc!{"$set":{"lease_token":&token,"lease_until":{"$add":["$$NOW",120_000]}}}])
        .return_document(ReturnDocument::After).await?.ok_or_else(lost)?;
    verify_generation(keys, &run).await?;
    let index =
        usize::try_from(run.get_i32("entry").map_err(|_| invalid())?).map_err(|_| invalid())?;
    if index == REGISTRY.len() {
        // Retain every unresolved row encountered by this pass. Findings from
        // previous passes which were not reencountered now have no source;
        // remove only a bounded page, fenced with this exact generation.
        let obsolete = db
            .collection::<Document>(FINDINGS)
            .find(doc! {"generation":{"$ne":generation}})
            .sort(doc! {"_id":1})
            .limit(BATCH)
            .projection(doc! {"_id":1})
            .await?
            .try_collect::<Vec<_>>()
            .await?;
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .write_concern(mongodb::options::WriteConcern::majority())
            .await?;
        let fence = doc! {"_id":RUN_ID,"generation":generation,"lease_token":&token,"$expr":{"$gt":["$lease_until","$$NOW"]}};
        let changed=db.collection::<Document>(RUNS).update_one(fence,doc!{"$set":{
            "phase":if obsolete.len()<BATCH as usize{"observed"}else{"running"},
            "cursor":Bson::Null,"lease_token":Bson::Null,"lease_until":bson::DateTime::from_millis(0),"updated_at":bson::DateTime::now()
        }}).session(&mut session).await?;
        if changed.matched_count != 1 {
            session.abort_transaction().await?;
            return Err(lost());
        }
        for finding in obsolete {
            db.collection::<Document>(FINDINGS)
                .delete_one(
                    doc! {"_id":finding.get("_id").cloned(),"generation":{"$ne":generation}},
                )
                .session(&mut session)
                .await?;
        }
        super::transaction_service::commit(&mut session).await?;
        return Ok(());
    }
    let entry = REGISTRY.get(index).ok_or_else(invalid)?;
    let mode: Mode =
        bson::from_bson(run.get("mode").cloned().ok_or_else(invalid)?).map_err(|_| invalid())?;
    let mut filter = doc! {};
    if let Some(cursor) = run.get("cursor").filter(|cursor| **cursor != Bson::Null) {
        filter.insert("_id", doc! {"$gt":cursor});
    }
    let rows = db
        .collection::<Document>(entry.collection)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(BATCH)
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    let mut cursor = run.get("cursor").cloned().unwrap_or(Bson::Null);
    for row in &rows {
        let fence = doc! {"_id":RUN_ID,"generation":generation,"lease_token":&token,"$expr":{"$gt":["$lease_until","$$NOW"]}};
        if db
            .collection::<Document>(RUNS)
            .update_one(
                fence.clone(),
                vec![doc! {"$set":{"lease_until":{"$add":["$$NOW",120_000]}}}],
            )
            .await?
            .matched_count
            != 1
        {
            return Err(lost());
        }
        let id = row.get("_id").ok_or_else(invalid)?;
        // Keyed opaque identity; diagnostics retain no token, owner, BSON row,
        // plaintext header or low-entropy unkeyed secret digest.
        let diagnostic = diagnostic_id(index, id)?;
        let no_material = !has_material(entry, row);
        // Only optional families can lack material. Public-session tombstones
        // still require lifecycle checks; an empty required ciphertext is an
        // error, never an exemption based solely on its byte length.
        let exempt = no_material
            && match entry.adapter {
                Adapter::Pending => true,
                Adapter::Capability => {
                    row.get("capability_encrypted")
                        .is_none_or(|v| v == &Bson::Null)
                        && row.get_str("status") == Ok("complete")
                }
                Adapter::Attachment => {
                    row.get_str("origin") == Ok("user_upload")
                        && row.get("text_encrypted").is_none_or(|v| v == &Bson::Null)
                        && !row.contains_key("parent_attachment_id")
                        && super::record_field_service::stored_versions(row)?.is_empty()
                }
                _ => false,
            };
        let mut verified_winner = Some(row.clone());
        let result = if exempt {
            Ok(false)
        } else {
            match verify(entry, &db, keys, row).await {
                Ok(()) => Ok(false),
                Err(_) if mode == Mode::Migrate && !matches!(entry.adapter, Adapter::Pending) => {
                    match migrate(entry, &db, keys, row).await {
                        Ok(changed) => {
                            // Reread the actual winner after CAS/no-op. A false
                            // result is never itself proof of successful rewrap.
                            match db
                                .collection::<Document>(entry.collection)
                                .find_one(doc! {"_id":id})
                                .await?
                            {
                                Some(current) => {
                                    let checked = verify(entry, &db, keys, &current).await;
                                    if checked.is_ok() {
                                        verified_winner = Some(current);
                                    }
                                    checked.map(|()| changed)
                                }
                                None => {
                                    verified_winner = None;
                                    Ok(changed)
                                }
                            }
                        }
                        Err(error) => Err(error),
                    }
                }
                Err(error) => Err(error),
            }
        };
        let mut rewritten = 0_i64;
        let mut unresolved = 0_i64;
        let category = match result {
            Ok(changed) => {
                rewritten += i64::from(changed);
                None
            }
            Err(AppError::DatabaseError(error)) => return Err(AppError::DatabaseError(error)),
            Err(_) => {
                unresolved += 1;
                Some(if matches!(entry.adapter, Adapter::Pending) {
                    "adapter_not_implemented"
                } else {
                    "authentication_binding_or_lifecycle_unresolved"
                })
            }
        };
        // Checkpoint and finding commit together; an expired worker cannot
        // erase a later worker's evidence or skip this page.
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .write_concern(mongodb::options::WriteConcern::majority())
            .await?;
        let mut increments = doc! {"scanned":1_i64,"rewritten":rewritten,"unresolved":unresolved};
        for (field_index, field) in entry.fields.iter().enumerate() {
            let kind = if category.is_some() {
                "verification_blocked"
            } else if verified_winner.is_none() {
                "source_removed_observed"
            } else if !field_has_material(verified_winner.as_ref().unwrap(), field) {
                "no_material_observed"
            } else {
                "authenticated_current"
            };
            increments.insert(
                format!("field_outcomes.{index}.{field_index}.{kind}"),
                1_i64,
            );
        }
        let changed = db
            .collection::<Document>(RUNS)
            .update_one(
                fence,
                doc! {"$set":{"last_row_checked_at":bson::DateTime::now(),"cursor":id},"$inc":increments},
            )
            .session(&mut session)
            .await?;
        if changed.matched_count != 1 {
            session.abort_transaction().await?;
            return Err(lost());
        }
        if let Some(category) = category {
            db.collection::<Document>(FINDINGS).update_one(doc!{"_id":&diagnostic},doc!{"$set":{"generation":generation,"entry":index as i32,"collection":entry.collection,"registry_digest":registry_digest(),"entry_contract":entry_contract(entry),"fields":entry.fields,"category":category,"observed_at":bson::DateTime::now()}}).upsert(true).session(&mut session).await?;
        } else {
            db.collection::<Document>(FINDINGS)
                .delete_one(doc! {"_id":&diagnostic})
                .session(&mut session)
                .await?;
        }
        super::transaction_service::commit(&mut session).await?;
        cursor = id.clone();
    }
    let finished = rows.len() < BATCH as usize;
    let next = if finished { index + 1 } else { index };
    // One final bounded cleanup phase follows the collection walk.
    let phase = "running";
    let changed=db.collection::<Document>(RUNS).update_one(doc!{"_id":RUN_ID,"generation":generation,"lease_token":&token,"$expr":{"$gt":["$lease_until","$$NOW"]}},
        doc!{"$set":{"entry":next as i32,"phase":phase,"cursor":if finished{Bson::Null}else{cursor},"lease_token":Bson::Null,"lease_until":bson::DateTime::from_millis(0),"updated_at":bson::DateTime::now()}}).await?;
    if changed.matched_count != 1 {
        return Err(lost());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::{
        key_provider::{KeyProvider, WrappedKey},
        local_key_provider::LocalKeyProvider,
    };
    use std::sync::Arc;
    use zeroize::Zeroizing;

    #[test]
    fn catalog_header_owner_only_falls_back_for_legacy_absence_or_null() {
        let entry = REGISTRY
            .iter()
            .find(|entry| {
                entry.collection == "downstream_services"
                    && matches!(entry.adapter, Adapter::Header)
            })
            .unwrap();
        assert_eq!(
            header_owner(entry, &doc! {"created_by":"creator"}).unwrap(),
            "creator"
        );
        assert_eq!(
            header_owner(
                entry,
                &doc! {"created_by":"creator","owner_user_id":Bson::Null}
            )
            .unwrap(),
            "creator"
        );
        assert_eq!(
            header_owner(
                entry,
                &doc! {"created_by":"creator","owner_user_id":"owner"}
            )
            .unwrap(),
            "owner"
        );
        for owner in [
            Bson::Int32(4),
            Bson::Boolean(false),
            Bson::String(String::new()),
            Bson::Document(doc! {}),
        ] {
            assert!(
                header_owner(entry, &doc! {"created_by":"creator","owner_user_id":owner}).is_err()
            );
        }
    }

    #[test]
    fn finding_contract_rejects_a_different_adapter_for_the_same_collection() {
        let (index, entry) = REGISTRY
            .iter()
            .enumerate()
            .find(|(_, entry)| {
                entry.collection == "downstream_services"
                    && matches!(entry.adapter, Adapter::Header)
            })
            .unwrap();
        let mut finding = doc! {"entry":index as i32,"collection":entry.collection,"fields":entry.fields,"registry_digest":registry_digest(),"entry_contract":entry_contract(entry)};
        assert!(finding_entry(&finding).is_ok());
        let other = REGISTRY
            .iter()
            .position(|entry| {
                entry.collection == "downstream_services"
                    && matches!(entry.adapter, Adapter::CatalogCredential)
            })
            .unwrap();
        finding.insert("entry", other as i32);
        assert!(finding_entry(&finding).is_err());
    }

    #[tokio::test]
    async fn ordinary_uuid_oauth_state_finding_never_discloses_the_protocol_handle() {
        crate::test_utils::init_test_audit_chain_hmac_key();
        let Some(db) =
            crate::test_utils::connect_test_database("encrypted_migration_opaque_uuid").await
        else {
            return;
        };
        let keys = crate::test_utils::test_encryption_keys();
        let id = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>("oauth_states")
            .insert_one(doc! {"_id":&id,"code_verifier":"invalid-legacy-value"})
            .await
            .unwrap();
        let generation = start(&db, &keys, Mode::Verify, None).await.unwrap();
        finish(&db, &keys, &generation).await;
        let finding = db
            .collection::<Document>(FINDINGS)
            .find_one(doc! {"collection":"oauth_states"})
            .await
            .unwrap()
            .unwrap();
        let result = lookup_finding(&db, &keys, finding.get_str("_id").unwrap(), None, false)
            .await
            .unwrap();
        assert!(result.record_uuid.is_none());
        assert!(!serde_json::to_string(&result).unwrap().contains(&id));
        db.drop().await.unwrap();
    }

    #[derive(Debug)]
    struct HeldWrap {
        inner: crate::crypto::local_key_provider::LocalKeyProvider,
        armed: std::sync::atomic::AtomicBool,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    #[async_trait::async_trait]
    impl crate::crypto::key_provider::KeyProvider for HeldWrap {
        async fn wrap_dek(&self, key: &[u8]) -> AppResult<crate::crypto::key_provider::WrappedKey> {
            if self.armed.swap(false, std::sync::atomic::Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            self.inner.wrap_dek(key).await
        }
        async fn unwrap_dek(
            &self,
            wrapped: &crate::crypto::key_provider::WrappedKey,
        ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
            self.inner.unwrap_dek(wrapped).await
        }
        fn current_key_id(&self) -> u8 {
            self.inner.current_key_id()
        }
        fn has_key_id(&self, id: u8) -> bool {
            self.inner.has_key_id(id)
        }
        fn has_previous_key(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn concurrent_source_delete_and_optional_clear_are_reported_from_the_verified_winner() {
        crate::test_utils::init_test_audit_chain_hmac_key();
        for remove in [true, false] {
            let Some(db) =
                crate::test_utils::connect_test_database("migration_winner_reporting").await
            else {
                return;
            };
            let standard = crate::test_utils::test_encryption_keys();
            let rows = db.collection::<Document>("provider_configs");
            rows.insert_one(doc! {"_id":"held-provider","client_id_encrypted":bson::Binary {
                subtype:bson::spec::BinarySubtype::Generic,bytes:standard.encrypt(b"held-client").await.unwrap()
            }}).await.unwrap();
            let held = std::sync::Arc::new(HeldWrap {
                inner: crate::crypto::local_key_provider::LocalKeyProvider::new([0x11; 32], None),
                armed: Default::default(),
                entered: Default::default(),
                release: Default::default(),
            });
            let keys = EncryptionKeys::with_provider(held.clone());
            let generation = start(&db, &keys, Mode::Migrate, None).await.unwrap();
            for _ in 0..3 {
                advance(&db, &keys, &generation).await.unwrap();
            }
            held.armed.store(true, std::sync::atomic::Ordering::SeqCst);
            let writer = async {
                held.entered.notified().await;
                if remove {
                    rows.delete_one(doc! {"_id":"held-provider"}).await.unwrap();
                } else {
                    rows.update_one(doc! {"_id":"held-provider"},doc! {"$set":{"client_id_encrypted":Bson::Null},"$unset":{"secret_context_versions":""}}).await.unwrap();
                }
                held.release.notify_one();
            };
            let (advanced, ()) = tokio::time::timeout(std::time::Duration::from_secs(20), async {
                tokio::join!(advance(&db, &keys, &generation), writer)
            })
            .await
            .expect("held migration completes");
            advanced.unwrap();
            let report = status(&db, &keys).await.unwrap().unwrap();
            let outcome = report
                .field_outcomes
                .iter()
                .find(|field| {
                    field.collection == "provider_configs" && field.field == "client_id_encrypted"
                })
                .unwrap();
            assert_eq!(outcome.authenticated_current, 0);
            assert_eq!(outcome.source_removed_observed, i64::from(remove));
            assert_eq!(outcome.no_material_observed, i64::from(!remove));
            assert_eq!(outcome.verification_blocked, 0);
            db.drop().await.unwrap();
        }
    }

    async fn finish(db: &Database, keys: &EncryptionKeys, generation: &str) -> Report {
        for _ in 0..REGISTRY.len() + 16 {
            let report = status(db, keys).await.unwrap().unwrap();
            if report.phase == "observed" {
                return report;
            }
            advance(db, keys, generation).await.unwrap();
        }
        panic!("bounded fixture walk did not finish")
    }

    #[tokio::test]
    async fn resumable_walk_migrates_later_rows_preserves_findings_and_does_not_rewrap_current_rows()
     {
        crate::test_utils::init_test_audit_chain_hmac_key();
        let Some(db) = crate::test_utils::connect_test_database("encrypted_migration_walk").await
        else {
            return;
        };
        let keys = crate::test_utils::test_encryption_keys();
        let rows = db.collection::<Document>("provider_configs");
        let legacy = keys.encrypt(b"fixture-client").await.unwrap();
        for index in 0..35 {
            rows.insert_one(doc!{"_id":format!("provider-{index:03}"),"client_id_encrypted":bson::Binary{
                subtype:bson::spec::BinarySubtype::Generic,bytes:if index==0{vec![1,2,3]}else{legacy.clone()}
            }}).await.unwrap();
        }
        let generation = start(&db, &keys, Mode::Migrate, None).await.unwrap();
        // Retain the exact cursor/generation between separate runner calls.
        for _ in 0..4 {
            advance(&db, &keys, &generation).await.unwrap();
        }
        let interrupted = status(&db, &keys).await.unwrap().unwrap();
        assert_eq!(interrupted.generation, generation);
        assert_eq!(interrupted.phase, "running");
        let report = finish(&db, &keys, &generation).await;
        assert_eq!(report.unresolved, 1);
        assert_eq!(report.rewritten, 34);
        assert!(report.blockers.contains(&"unresolved_storage_findings"));
        let last = rows
            .find_one(doc! {"_id":"provider-034"})
            .await
            .unwrap()
            .unwrap();
        super::super::credential_record_service::verify_current_snapshot(
            &keys,
            "provider_configs",
            &last,
        )
        .await
        .unwrap();
        let finding = db
            .collection::<Document>(FINDINGS)
            .find_one(doc! {"generation":&generation})
            .await
            .unwrap()
            .unwrap();
        let serialized = serde_json::to_string(&finding).unwrap();
        assert!(!serialized.contains("fixture-client"));
        assert!(!serialized.contains("provider-000"));
        assert_eq!(
            finding.get_str("category").unwrap(),
            "authentication_binding_or_lifecycle_unresolved"
        );
        // Repair is followed by another explicit complete pass. Previous
        // findings stay durable until the repaired row is authenticated.
        let page = list_findings(&db, None).await.unwrap();
        assert_eq!(page.findings.len(), 1);
        let reference = page.findings[0].id.clone();
        let lookup = lookup_finding(&db, &keys, &reference, None, false)
            .await
            .unwrap();
        assert_eq!(lookup.disposition, "storage_or_lifecycle_repair_required");
        assert!(
            lookup.record_uuid.is_none(),
            "arbitrary selectors must not be exposed"
        );
        rows.update_one(
            doc! {"_id":"provider-000"},
            doc! {"$set":{"client_id_encrypted":bson::Binary{
            subtype:bson::spec::BinarySubtype::Generic,bytes:legacy}}},
        )
        .await
        .unwrap();
        let recheck = lookup_finding(&db, &keys, &reference, None, true)
            .await
            .unwrap();
        assert_eq!(
            recheck.disposition,
            "authenticated_current_new_pass_required"
        );
        assert_eq!(
            status(&db, &keys).await.unwrap().unwrap().unresolved,
            1,
            "targeted repair does not manufacture a full-pass checkpoint"
        );
        let generation2 = start(&db, &keys, Mode::Migrate, None).await.unwrap();
        let repaired = finish(&db, &keys, &generation2).await;
        assert_eq!(repaired.unresolved, 0);
        assert_eq!(
            repaired.rewritten, 0,
            "targeted recheck already rewrote the repaired row"
        );
        assert_eq!(
            rows.find_one(doc! {"_id":"provider-034"})
                .await
                .unwrap()
                .unwrap(),
            last
        );
        assert_eq!(
            db.collection::<Document>(FINDINGS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let generation3 = start(&db, &keys, Mode::Migrate, None).await.unwrap();
        let verified = finish(&db, &keys, &generation3).await;
        assert_eq!(
            verified.rewritten, 0,
            "a current row must not churn ciphertext or dependency revisions"
        );
        assert_eq!(
            rows.find_one(doc! {"_id":"provider-034"})
                .await
                .unwrap()
                .unwrap(),
            last
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn lease_and_explicit_restart_preserve_generation_fences() {
        crate::test_utils::init_test_audit_chain_hmac_key();
        let Some(db) = crate::test_utils::connect_test_database("encrypted_migration_lease").await
        else {
            return;
        };
        let keys = crate::test_utils::test_encryption_keys();
        let generation = start(&db, &keys, Mode::Verify, None).await.unwrap();
        let runs = db.collection::<Document>(RUNS);
        runs.update_one(doc!{"_id":RUN_ID},vec![doc!{"$set":{"lease_token":"other-replica","lease_until":{"$add":["$$NOW",120_000]}}}]).await.unwrap();
        let held = runs.find_one(doc! {"_id":RUN_ID}).await.unwrap().unwrap();
        assert!(matches!(
            advance(&db, &keys, &generation).await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            start(&db, &keys, Mode::Verify, Some(&generation)).await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            runs.find_one(doc! {"_id":RUN_ID}).await.unwrap().unwrap(),
            held
        );
        runs.update_one(
            doc! {"_id":RUN_ID},
            doc! {"$set":{"lease_until":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
        let successor = start(&db, &keys, Mode::Verify, Some(&generation))
            .await
            .unwrap();
        assert_ne!(successor, generation);
        assert!(matches!(
            advance(&db, &keys, &generation).await,
            Err(AppError::Conflict(_))
        ));
        advance(&db, &keys, &successor).await.unwrap();
        db.drop().await.unwrap();
    }

    /// A single logical resource accepts two enabled internal versions. This
    /// deliberately models the cloud behavior which a u8 selector cannot prove.
    #[derive(Debug)]
    struct LogicalCloud {
        versions: [LocalKeyProvider; 2],
        current: usize,
    }
    #[async_trait::async_trait]
    impl KeyProvider for LogicalCloud {
        async fn wrap_dek(&self, plain: &[u8]) -> AppResult<WrappedKey> {
            let wrapped = self.versions[self.current].wrap_dek(plain).await?;
            let mut bytes = Zeroizing::new(vec![self.current as u8]);
            bytes.extend_from_slice(&wrapped.ciphertext);
            Ok(WrappedKey {
                key_id: self.current_key_id(),
                ciphertext: bytes,
            })
        }
        async fn unwrap_dek(&self, wrapped: &WrappedKey) -> AppResult<Zeroizing<Vec<u8>>> {
            let (version, bytes) = wrapped.ciphertext.split_first().ok_or_else(invalid)?;
            let provider = self.versions.get(*version as usize).ok_or_else(invalid)?;
            provider
                .unwrap_dek(&WrappedKey {
                    key_id: provider.current_key_id(),
                    ciphertext: bytes.to_vec().into(),
                })
                .await
        }
        fn current_key_id(&self) -> u8 {
            0x55
        }
        fn has_key_id(&self, id: u8) -> bool {
            id == 0x55
        }
        fn has_previous_key(&self) -> bool {
            false
        }
    }

    #[tokio::test]
    async fn same_logical_cloud_key_decrypting_old_internal_version_is_not_retirement_proof() {
        let Some(db) =
            crate::test_utils::connect_test_database("encrypted_migration_cloud_versions").await
        else {
            return;
        };
        let make_keys = |current| {
            let mut keys = EncryptionKeys::with_provider(Arc::new(LogicalCloud {
                versions: [
                    LocalKeyProvider::new([31; 32], None),
                    LocalKeyProvider::new([32; 32], None),
                ],
                current,
            }));
            keys.set_contextual_record_writes(true);
            keys
        };
        let original = make_keys(0);
        let generation = start(&db, &original, Mode::Verify, None).await.unwrap();
        let rotated = make_keys(1);
        let row = db
            .collection::<Document>(RUNS)
            .find_one(doc! {"_id":RUN_ID})
            .await
            .unwrap()
            .unwrap();
        // This authentication succeeds under the new logical provider while
        // its wrapped DEK still depends on internal version zero.
        verify_generation(&rotated, &row).await.unwrap();
        let report = status(&db, &rotated).await.unwrap().unwrap();
        assert_eq!(report.generation, generation);
        assert!(
            report
                .blockers
                .contains(&"cloud_internal_key_versions_operator_unverified")
        );
        assert!(
            !report
                .blockers
                .contains(&"registry_or_primary_provider_changed")
        );
        db.drop().await.unwrap();
    }
}
