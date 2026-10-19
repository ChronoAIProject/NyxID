//! Exact-token write admission and durable revocation tombstones. Fingerprints
//! are keyed across provider/owner labels: relabelling an imported token cannot
//! create an independent upstream capability. No plaintext enters this index.
use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
};
use bson::{Document, doc};
use mongodb::{ClientSession, Database};
use std::collections::BTreeSet;

pub(crate) const HOLDERS: &str = "credential_token_holders";
const FENCES: &str = "credential_token_fences";
const RESULTS: &str = "credential_token_revocations";
const DISCOVERY: &str = "credential_token_discovery";
const CONTRACT: i32 = 1;
// The original preview marker covered only two collections without binding
// decoder/field coverage. Never accept that marker as a newer registry proof.
const COVERAGE_VERSION: i32 = 6;
const COLLECTIONS: [&str; 6] = [
    "user_api_keys",
    "user_provider_tokens",
    "downstream_services",
    "user_services",
    "user_service_connections",
    "platform_credentials",
];
const MATERIAL: [&str; 7] = [
    "credential_encrypted",
    "access_token_encrypted",
    "refresh_token_encrypted",
    "api_key_encrypted",
    "default_request_headers",
    "secrets",
    "fields",
];
const IDENTITY: [&str; 12] = [
    "user_id",
    "provider_config_id",
    "connection_id",
    "source_id",
    "secret_context_versions",
    "grant_lineage",
    "credential_type",
    "owner_user_id",
    "created_by",
    "service_id",
    "provider",
    "auth_method",
];

fn coverage_digest() -> String {
    use sha2::{Digest, Sha256};
    let mut hash = Sha256::new();
    hash.update(
        b"nyxid-token-holder-registry-v6:platform-token-fields-v2:oauth-ows-aliases-v1:credential-v3:catalog-v3:header-v4:connection-v3\0",
    );
    for field in COLLECTIONS
        .iter()
        .chain(MATERIAL.iter())
        .chain(IDENTITY.iter())
    {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field.as_bytes());
    }
    hex::encode(hash.finalize())
}

/// Additional execution stores and raw writers still need complete decoder,
/// discovery and atomic writer admission coverage. An acknowledgement cannot
/// waive missing implementation coverage.
pub(crate) fn pending_holder_families() -> &'static [&'static str] {
    &[
        "catalog_master_and_headers",
        "instance_headers_and_connection_credentials",
        "channel_bot_credentials",
        "platform_credentials_and_transports",
        "other_retained_execution_capabilities",
    ]
}

fn current_coverage(row: &Document) -> bool {
    row.get_i32("contract").ok() == Some(CONTRACT)
        && row.get_i32("coverage_version").ok() == Some(COVERAGE_VERSION)
        && row.get_str("coverage_digest").ok() == Some(coverage_digest().as_str())
}

fn invalid() -> AppError {
    AppError::Internal("Token copy admission requires authenticated repair".into())
}
pub(crate) fn fingerprint(token: &[u8]) -> AppResult<String> {
    super::audit_service::keyed_fingerprint(
        super::audit_service::audit_chain_hmac_key(),
        b"oauth-revocation-provenance-v1",
        token,
    )
    .ok_or_else(invalid)
}
fn holder_id(collection: &str, id: &str) -> AppResult<String> {
    if !COLLECTIONS.contains(&collection) || id.is_empty() {
        return Err(invalid());
    }
    Ok(format!("{collection}:{id}"))
}
pub(crate) fn material_changed(before: Option<&Document>, after: &Document) -> bool {
    before.is_none_or(|before| {
        MATERIAL
            .iter()
            .chain(IDENTITY.iter())
            .any(|field| before.get(*field) != after.get(*field))
    })
}
fn source_hash(row: &Document) -> AppResult<String> {
    use sha2::{Digest, Sha256};
    let fields: Document = MATERIAL
        .iter()
        .chain(IDENTITY.iter())
        .map(|field| {
            (
                (*field).into(),
                row.get(*field).cloned().unwrap_or(bson::Bson::Null),
            )
        })
        .collect();
    Ok(hex::encode(Sha256::digest(
        bson::to_vec(&fields).map_err(|_| invalid())?,
    )))
}
fn fence_owner(row: &Document) -> AppResult<Option<&str>> {
    if row.get_i32("contract").ok() != Some(CONTRACT) {
        return Err(invalid());
    }
    match row.get("closed_by") {
        None => Ok(None),
        Some(bson::Bson::String(owner)) if !owner.is_empty() => Ok(Some(owner)),
        Some(_) => Err(invalid()),
    }
}

pub(crate) async fn ensure_indexes(db: &Database) -> AppResult<()> {
    db.collection::<Document>(HOLDERS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"tokens":1,"_id":1})
                .build(),
        )
        .await?;
    Ok(())
}

/// Reader-first rollout: before explicit writer cutover, missing writer keys
/// leave discovery incomplete. After cutover every material writer must either
/// authenticate its installed snapshot or roll the whole transaction back.
pub(crate) fn write_in_session<'a>(
    db: &'a Database,
    keys: Option<&'a EncryptionKeys>,
    collection: &'a str,
    row: &'a Document,
    tx: &'a mut ClientSession,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        if db
            .collection::<Document>(DISCOVERY)
            .find_one(doc! {"_id":"v1"})
            .session(&mut *tx)
            .await?
            .is_none()
        {
            // Activation writes this same rollout fence transactionally with its
            // marker and scan ceilings. A held transaction which observed absence
            // must commit before activation or abort; it cannot insert below an
            // already-completed cursor without a holder. This shared write is only
            // needed during the pre-cutover deployment stage.
            super::erasure_admission::credential_inventory_in_session(db, tx, false).await?;
            return Ok(());
        }
        if let Some(keys) = keys {
            return replace_in_session(db, keys, collection, row, tx).await;
        }
        if !MATERIAL.iter().any(|field| {
            row.get(*field)
                .is_some_and(|value| *value != bson::Bson::Null)
        }) {
            return remove_in_session(
                db,
                collection,
                row.get_str("_id").map_err(|_| invalid())?,
                tx,
            )
            .await;
        }
        if db
            .collection::<Document>(DISCOVERY)
            .find_one(doc! {"_id":"v1"})
            .session(tx)
            .await?
            .is_some()
        {
            return Err(AppError::Conflict(
                "Credential writer must authenticate token admission after cutover".into(),
            ));
        }
        Ok(())
    })
}

/// OAuth access/refresh tokens can reach HTTP bearer authentication. Protect
/// both the exact input and HTTP OWS aliases without changing stored or wire
/// bytes. These aliases are negative safety evidence, never confirmation that
/// a distinct form/JSON request revoked a different byte sequence.
fn add_oauth_token(tokens: &mut BTreeSet<String>, token: &str) -> AppResult<()> {
    if token.is_empty() || token.trim_matches([' ', '\t']).is_empty() {
        return Err(invalid());
    }
    tokens.insert(fingerprint(token.as_bytes())?);
    tokens.insert(fingerprint(token.trim_matches([' ', '\t']).as_bytes())?);
    Ok(())
}

/// Inventory raw scalar credentials and the supported HTTP scheme form. Header
/// masking has no bearing on execution: both public and masked defaults retain
/// their values. Fingerprint the whole value and an explicit scheme's payload;
/// no plaintext is retained in the holder document.
fn add_text(tokens: &mut BTreeSet<String>, text: &str) -> AppResult<()> {
    if text.is_empty() {
        return Ok(());
    }
    tokens.insert(fingerprint(text.as_bytes())?);
    // HTTP field OWS is SP/HTAB, not Unicode whitespace. Preserve the exact
    // stored material and conservatively also protect its wire-normalized form.
    let normalized = text.trim_matches([' ', '\t']);
    if !normalized.is_empty() {
        tokens.insert(fingerprint(normalized.as_bytes())?);
    }
    if let Some((scheme, value)) = normalized.split_once(' ')
        && scheme.eq_ignore_ascii_case("bearer")
        && !value.trim_start_matches(' ').is_empty()
        && !value
            .trim_start_matches(' ')
            .bytes()
            .any(|byte| byte.is_ascii_whitespace())
    {
        tokens.insert(fingerprint(value.trim_start_matches(' ').as_bytes())?);
    }
    Ok(())
}

async fn retained_tokens(
    keys: &EncryptionKeys,
    collection: &'static str,
    row: &Document,
) -> AppResult<(String, BTreeSet<String>)> {
    let mut tokens = BTreeSet::new();
    let id = row.get_str("_id").map_err(|_| invalid())?;
    let owner = match collection {
        "platform_credentials" => {
            let platform: crate::models::platform_credential::PlatformCredential =
                bson::from_document(row.clone()).map_err(|_| invalid())?;
            let values = super::platform_credential_service::decrypt_platform_snapshot(
                keys,
                Some(&platform),
            )
            .await?;
            // These are actual independently retained bearer capabilities.
            // Client secrets and webhook verifiers are distinct protocols.
            // Unknown descriptors require a reviewed coverage upgrade.
            let fields: &[&str] = match platform.provider.as_str() {
                "x" => &["app_bearer_token"],
                "telegram-new" => &["manager_bot_token"],
                "meta" | "aurinko" => &[],
                _ => return Err(invalid()),
            };
            for field in fields {
                if let Some(value) = values.get(field) {
                    add_oauth_token(&mut tokens, value)?;
                }
            }
            super::credential_record_service::PLATFORM_OWNER.to_owned()
        }
        "downstream_services" => {
            let service: crate::models::downstream_service::DownstreamService =
                bson::from_document(row.clone()).map_err(|_| invalid())?;
            super::catalog_credential_record_service::validate(&service)?;
            let owner = super::catalog_credential_record_service::owner(&service)?.to_owned();
            if !service.credential_encrypted.is_empty() {
                let value = super::catalog_credential_record_service::text(keys, &service).await?;
                if service.auth_method == "bearer" {
                    add_oauth_token(&mut tokens, &value)?;
                } else {
                    tokens.insert(fingerprint(value.as_bytes())?);
                }
            }
            for header in super::default_header_service::decode(
                Some(keys),
                collection,
                id,
                &owner,
                service.default_request_headers.as_deref(),
            )
            .await?
            {
                add_text(&mut tokens, &header.value)?;
            }
            owner
        }
        "user_services" => {
            let service: crate::models::user_service::UserService =
                bson::from_document(row.clone()).map_err(|_| invalid())?;
            for header in super::default_header_service::decode(
                Some(keys),
                collection,
                id,
                &service.user_id,
                service.default_request_headers.as_deref(),
            )
            .await?
            {
                add_text(&mut tokens, &header.value)?;
            }
            service.user_id
        }
        "user_service_connections" => {
            let connection: crate::models::user_service_connection::UserServiceConnection =
                bson::from_document(row.clone()).map_err(|_| invalid())?;
            // Validate absence too: a dangling context marker is never an empty
            // holder. Lifecycle validation is shared with the execution reader.
            super::connection_record_service::validate(&connection)?;
            if connection.credential_encrypted.is_some() {
                let value = super::connection_record_service::text(keys, &connection).await?;
                if matches!(
                    connection.credential_type.as_deref(),
                    Some("bearer" | "oauth2")
                ) {
                    add_oauth_token(&mut tokens, &value)?;
                } else if !value.is_empty() {
                    tokens.insert(fingerprint(value.as_bytes())?);
                }
            }
            connection.user_id
        }
        "user_api_keys" | "user_provider_tokens" => {
            for field in MATERIAL
                .iter()
                .filter(|field| **field != "default_request_headers")
            {
                if let Some(value) = row.get(*field).filter(|value| **value != bson::Bson::Null) {
                    let bytes = super::record_field_service::stored_bytes(value)?;
                    let value = super::credential_record_service::decrypt_stored_field(
                        keys, collection, row, field, &bytes,
                    )
                    .await?;
                    if !value.is_empty()
                        && (matches!(*field, "access_token_encrypted" | "refresh_token_encrypted")
                            || (*field == "credential_encrypted"
                                && matches!(
                                    row.get_str("credential_type").ok(),
                                    Some("bearer" | "oauth2")
                                )))
                    {
                        add_oauth_token(
                            &mut tokens,
                            std::str::from_utf8(&value).map_err(|_| invalid())?,
                        )?;
                    } else if !value.is_empty() {
                        tokens.insert(fingerprint(&value)?);
                    }
                }
            }
            row.get_str("user_id").map_err(|_| invalid())?.to_owned()
        }
        _ => return Err(invalid()),
    };
    if owner.is_empty() || id.is_empty() {
        return Err(invalid());
    }
    Ok((owner, tokens))
}

/// Authenticate the exact installed row, then serialize its holder replacement
/// with dispatch on each token. Call inside the writer's majority transaction;
/// a failure rolls back both the material write and the index replacement.
pub(crate) fn replace_in_session<'a>(
    db: &'a Database,
    keys: &'a EncryptionKeys,
    collection: &'a str,
    row: &'a Document,
    tx: &'a mut ClientSession,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        let collection = COLLECTIONS
            .into_iter()
            .find(|candidate| *candidate == collection)
            .ok_or_else(invalid)?;
        let id = row.get_str("_id").map_err(|_| invalid())?;
        let holder = holder_id(collection, id)?;
        let (owner, tokens) = retained_tokens(keys, collection, row).await?;
        let previous = db
            .collection::<Document>(HOLDERS)
            .find_one(doc! {"_id":&holder})
            .session(&mut *tx)
            .await?;
        let mut affected = tokens.clone();
        if let Some(previous) = previous.as_ref() {
            for value in previous.get_array("tokens").map_err(|_| invalid())? {
                affected.insert(value.as_str().ok_or_else(invalid)?.to_owned());
            }
        }
        // Stable order bounds contention to actual equal-token writers. A closed
        // token remains closed across failed/uncertain upstream calls and restarts.
        for token in affected {
            let fence = db
                .collection::<Document>(FENCES)
                .find_one_and_update(
                    doc! {"_id":&token},
                    doc! {"$inc":{"revision":1_i64},"$setOnInsert":{"contract":CONTRACT}},
                )
                .upsert(true)
                .return_document(mongodb::options::ReturnDocument::After)
                .session(&mut *tx)
                .await?
                .ok_or_else(invalid)?;
            if fence_owner(&fence)?.is_some() && tokens.contains(&token) {
                return Err(AppError::Conflict(
                    "This token is reserved for revocation; reconnect for a new authorization"
                        .into(),
                ));
            }
        }
        db.collection::<Document>(HOLDERS).replace_one(doc! {"_id":&holder}, doc! {"_id":holder,"collection":collection,"record_id":id,"resource_owner_id":owner,"tokens":tokens.into_iter().collect::<Vec<_>>(),"source_hash":source_hash(row)?,"contract":CONTRACT}).upsert(true).session(&mut *tx).await?;
        Ok(())
    })
}

/// Storage adapters replace ciphertext/markers and the holder's source digest
/// in one transaction. Never index a model roundtrip in place of the actual
/// installed BSON winner.
pub(crate) async fn migrate_storage(
    db: &Database,
    keys: &EncryptionKeys,
    collection: &str,
    source: &Document,
    set: &Document,
) -> AppResult<bool> {
    let id = source.get_str("_id").map_err(|_| invalid())?;
    let db = super::transaction_service::durable_database(db);
    super::transaction_service::run_db_only(
        &db,
        &(&db, keys, collection, source, set, id),
        |tx, context| {
            let &(db, keys, collection, source, set, id) = context;
            Box::pin(async move {
                let changed = db
                    .collection::<Document>(collection)
                    .update_one(
                        super::record_storage::exact_document_predicate(source),
                        doc! {"$set":set},
                    )
                    .session(&mut *tx)
                    .await?;
                if changed.matched_count == 0 {
                    return Ok(false);
                }
                installed_in_session(db, keys, collection, id, tx).await?;
                Ok(true)
            })
        },
    )
    .await
}

/// Raw provider-token writers reread their installed winner inside the same
/// transaction. Model reserialization must not replace raw context evidence.
pub(crate) async fn installed_in_session(
    db: &Database,
    keys: &EncryptionKeys,
    collection: &str,
    id: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let row = db
        .collection::<Document>(collection)
        .find_one(doc! {"_id":id})
        .session(&mut *tx)
        .await?
        .ok_or_else(invalid)?;
    write_in_session(db, Some(keys), collection, &row, tx).await
}

pub(crate) async fn remove_in_session(
    db: &Database,
    collection: &str,
    id: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    let holder = holder_id(collection, id)?;
    if let Some(previous) = db
        .collection::<Document>(HOLDERS)
        .find_one(doc! {"_id":&holder})
        .session(&mut *tx)
        .await?
    {
        let mut tokens = previous
            .get_array("tokens")
            .map_err(|_| invalid())?
            .iter()
            .map(|value| value.as_str().map(str::to_owned).ok_or_else(invalid))
            .collect::<AppResult<Vec<_>>>()?;
        tokens.sort();
        for token in tokens {
            let fence = db
                .collection::<Document>(FENCES)
                .find_one_and_update(doc! {"_id":token}, doc! {"$inc":{"revision":1_i64}})
                .return_document(mongodb::options::ReturnDocument::After)
                .session(&mut *tx)
                .await?
                .ok_or_else(invalid)?;
            fence_owner(&fence)?;
        }
        db.collection::<Document>(HOLDERS)
            .delete_one(doc! {"_id":holder})
            .session(tx)
            .await?;
    }
    Ok(())
}

/// Begin discovery only after the operator has drained binaries which do not
/// enforce this write contract. No scan is proof that old writers have stopped.
#[cfg(test)]
pub(crate) async fn begin_discovery(db: &Database, old_writers_drained: bool) -> AppResult<()> {
    begin_discovery_with_audit(db, old_writers_drained, None).await
}

/// Completion is immutable through this API. Material writers still acquire
/// every affected token fence in their own transaction after this point.
pub(crate) async fn discovery_complete_in_session(
    db: &Database,
    tx: &mut ClientSession,
) -> AppResult<bool> {
    let Some(row) = db
        .collection::<Document>(DISCOVERY)
        .find_one(doc! {"_id":"v1"})
        .session(tx)
        .await?
    else {
        return Ok(false);
    };
    if !current_coverage(&row) {
        return Ok(false);
    }
    match row.get_str("status").map_err(|_| invalid())? {
        "complete" => Ok(true),
        "building" | "blocked" => Ok(false),
        _ => Err(invalid()),
    }
}

pub(crate) async fn activate_discovery(
    db: &Database,
    old_writers_drained: bool,
    actor: &super::audit_service::AuditActor,
) -> AppResult<()> {
    if !pending_holder_families().is_empty() {
        return Err(AppError::Conflict("Token holder coverage is incomplete; deployment acknowledgement cannot activate automatic dispatch".into()));
    }
    begin_discovery_with_audit(db, old_writers_drained, Some(actor)).await
}

async fn begin_discovery_with_audit(
    db: &Database,
    old_writers_drained: bool,
    actor: Option<&super::audit_service::AuditActor>,
) -> AppResult<()> {
    if !old_writers_drained {
        return Err(AppError::Conflict(
            "Token admission writer cutover acknowledgement is required".into(),
        ));
    }
    ensure_indexes(db).await?;
    let db = super::transaction_service::durable_database(db);
    super::transaction_service::run_db_only(&db, &(&db, actor), |tx, (db, actor)| {
        Box::pin(async move {
            super::erasure_admission::credential_inventory_in_session(db, tx, false).await?;
            if let Some(existing) = db
                .collection::<Document>(DISCOVERY)
                .find_one(doc! {"_id":"v1"})
                .session(&mut *tx)
                .await?
            {
                if !current_coverage(&existing) {
                    return Err(AppError::Conflict(
                        "Token discovery registry changed; authenticated backfill is required"
                            .into(),
                    ));
                }
                return Ok(());
            }
            let mut ceilings = Document::new();
            for collection in COLLECTIONS {
                let rows = db.collection::<Document>(collection);
                if rows
                    .find_one(doc! {"_id":{"$not":{"$type":"string"}}})
                    .session(&mut *tx)
                    .await?
                    .is_some()
                {
                    return Err(invalid());
                }
                let last = rows
                    .find_one(doc! {})
                    .sort(doc! {"_id":-1})
                    .projection(doc! {"_id":1})
                    .session(&mut *tx)
                    .await?;
                ceilings.insert(
                    collection,
                    last.and_then(|row| row.get("_id").cloned())
                        .unwrap_or(bson::Bson::Null),
                );
            }
            let generation = uuid::Uuid::new_v4().to_string();
            db.collection::<Document>(DISCOVERY)
                .insert_one(doc! {
                    "_id":"v1","status":"building","contract":CONTRACT,
                    "coverage_version":COVERAGE_VERSION,"coverage_digest":coverage_digest(),
                    "generation":&generation,"observed_rows":0_i64,
                    "collection_index":0_i32,"cursor":bson::Bson::Null,"ceilings":ceilings,
                })
                .session(&mut *tx)
                .await?;
            if let Some(actor) = actor {
                super::audit_service::log_actor_event_in_session(
                    db,
                    tx,
                    super::audit_service::audit_chain_hmac_key().ok_or_else(invalid)?,
                    actor,
                    "token_admission.activated",
                    serde_json::json!({"generation":generation,"old_writers_drained":true}),
                )
                .await?;
            }
            Ok(())
        })
    })
    .await
}

/// One captured row per transaction. Concurrent replacements either participate
/// in admission or lose this exact source CAS; unrelated writes never reset the
/// cursor. New IDs below the cursor are indexed by their own writer transaction.
pub(crate) async fn discover_one(db: &Database, keys: &EncryptionKeys) -> AppResult<bool> {
    let durable = super::transaction_service::durable_database(db);
    let db = &durable;
    let Some(progress) = db
        .collection::<Document>(DISCOVERY)
        .find_one(doc! {"_id":"v1","status":{"$in":["building","blocked"]},"contract":CONTRACT})
        .await?
    else {
        return Ok(false);
    };
    if !current_coverage(&progress) {
        return Err(invalid());
    }
    let index = progress
        .get_i32("collection_index")
        .map_err(|_| invalid())? as usize;
    let Some(collection) = COLLECTIONS.get(index) else {
        return Err(invalid());
    };
    let ceiling = progress
        .get_document("ceilings")
        .map_err(|_| invalid())?
        .get(*collection)
        .cloned()
        .ok_or_else(invalid)?;
    let mut range = doc! {"$lte":ceiling};
    if let Ok(cursor) = progress.get_str("cursor") {
        range.insert("$gt", cursor);
    }
    let filter = doc! {"_id":range};
    let row = db
        .collection::<Document>(collection)
        .find_one(filter)
        .sort(doc! {"_id":1})
        .await?;
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let mut source_unreadable = false;
    let result: AppResult<()> = async {
        let next = if let Some(row) = row.as_ref() {
            // Write the source before installing its index. A held legacy
            // snapshot cannot overwrite the successor writer's holder entry.
            let changed = db
                .collection::<Document>(collection)
                .update_one(
                    super::record_storage::exact_document_predicate(row),
                    doc! {"$inc":{"token_discovery_fence":1_i64}},
                )
                .session(&mut tx)
                .await?;
            if changed.matched_count != 1 {
                return Err(AppError::Conflict(
                    "Token discovery source changed; retry this cursor".into(),
                ));
            }
            if let Err(error) = replace_in_session(db, keys, collection, row, &mut tx).await {
                source_unreadable = matches!(error, AppError::Internal(_));
                return Err(error);
            }
            doc! {"cursor":row.get_str("_id").map_err(|_|invalid())?,"status":"building"}
        } else if index + 1 < COLLECTIONS.len() {
            doc! {"collection_index":(index+1) as i32,"cursor":bson::Bson::Null,"status":"building"}
        } else {
            doc! {"status":"complete"}
        };
        let changed = db
            .collection::<Document>(DISCOVERY)
            .update_one(
                super::record_storage::exact_document_predicate(&progress),
                doc! {"$set":next,"$inc":{"observed_rows":i64::from(row.is_some())},"$unset":{"blocked_record_id":""}},
            )
            .session(&mut tx)
            .await?;
        if changed.matched_count != 1 {
            return Err(AppError::Conflict("Token discovery cursor changed".into()));
        }
        Ok(())
    }
    .await;
    let result = super::org_membership_authority::finish(tx, result).await;
    if source_unreadable {
        // The row transaction aborted. Persist a finite repair diagnostic only
        // if the same cursor is still current. Never skip unreadable material
        // or convert an unknown commit result into a repaired/complete claim.
        if let Some(row) = row.as_ref() {
            db.collection::<Document>(DISCOVERY).update_one(
                super::record_storage::exact_document_predicate(&progress),
                doc! {"$set":{"status":"blocked","blocked_record_id":row.get_str("_id").map_err(|_|invalid())?}},
            ).await?;
        }
    }
    result?;
    Ok(true)
}

/// Deliberately excludes token fingerprints, authority digests, cursor values
/// and ciphertext. Only explicitly reviewed non-capability domain row IDs
/// are exposed for operator repair; UUID shape alone never grants exposure.
#[derive(serde::Serialize)]
pub(crate) struct DiscoveryReport {
    pub generation: String,
    pub status: &'static str,
    pub collection: Option<&'static str>,
    pub observed_rows: i64,
    pub repair_record_uuid: Option<uuid::Uuid>,
    pub repair_reason: Option<&'static str>,
}

fn repair_record_uuid(collection: &str, id: &str) -> Option<uuid::Uuid> {
    match collection {
        "user_api_keys"
        | "user_provider_tokens"
        | "downstream_services"
        | "user_services"
        | "user_service_connections" => uuid::Uuid::parse_str(id).ok(),
        _ => None,
    }
}

pub(crate) async fn discovery_status(db: &Database) -> AppResult<Option<DiscoveryReport>> {
    let db = super::transaction_service::durable_database(db);
    let Some(row) = db
        .collection::<Document>(DISCOVERY)
        .find_one(doc! {"_id":"v1"})
        .await?
    else {
        return Ok(None);
    };
    if !current_coverage(&row) {
        return Err(invalid());
    }
    let status = match row.get_str("status").map_err(|_| invalid())? {
        "building" => "building",
        "blocked" => "blocked",
        "complete" => "complete",
        _ => return Err(invalid()),
    };
    let generation = row.get_str("generation").map_err(|_| invalid())?;
    uuid::Uuid::parse_str(generation).map_err(|_| invalid())?;
    let index = usize::try_from(row.get_i32("collection_index").map_err(|_| invalid())?)
        .map_err(|_| invalid())?;
    let collection = *COLLECTIONS.get(index).ok_or_else(invalid)?;
    let observed_rows = row.get_i64("observed_rows").map_err(|_| invalid())?;
    if observed_rows < 0 {
        return Err(invalid());
    }
    Ok(Some(DiscoveryReport {
        generation: generation.to_owned(),
        status,
        collection: (status != "complete").then_some(collection),
        observed_rows,
        repair_record_uuid: if status == "blocked" {
            row.get_str("blocked_record_id")
                .ok()
                .and_then(|id| repair_record_uuid(collection, id))
        } else {
            None
        },
        repair_reason: (status == "blocked")
            .then_some("source_authentication_or_key_configuration"),
    }))
}

/// The caller supplies the visible generation. Concurrent steps contend on
/// exact progress/source predicates; there is no skip, force-complete or reset
/// API. Repair a blocked source through its normal credential management path,
/// or restore the matching keys, then advance the same cursor again.
pub(crate) async fn advance_discovery(
    db: &Database,
    keys: &EncryptionKeys,
    generation: &str,
) -> AppResult<Option<DiscoveryReport>> {
    let Some(status) = discovery_status(db).await? else {
        return Err(AppError::Conflict(
            "Token discovery has not been activated".into(),
        ));
    };
    if status.generation != generation {
        return Err(AppError::Conflict(
            "Token discovery generation changed".into(),
        ));
    }
    for _ in 0..32 {
        match discover_one(db, keys).await {
            Ok(true) => {}
            Ok(false) => break,
            Err(error) => {
                let current = discovery_status(db).await?;
                if current
                    .as_ref()
                    .is_some_and(|report| report.status == "blocked")
                {
                    return Ok(current);
                }
                return Err(error);
            }
        }
    }
    discovery_status(db).await
}

/// Close exact tokens before any network dispatch. Every writer contends on
/// these same records. Never release a tombstone after an HTTP timeout: the
/// provider might already have acted. Confirmed results can be reused by later
/// durable effects; uncertain results remain owned by the original effect.
pub(crate) struct TokenClosure {
    confirmed: BTreeSet<String>,
    pub(crate) waiting_on: BTreeSet<String>,
}
impl TokenClosure {
    pub(crate) fn confirmed(&self, token: &str) -> AppResult<bool> {
        Ok(self.confirmed.contains(&fingerprint(token.as_bytes())?))
    }
}

pub(crate) async fn close_for_dispatch(
    db: &Database,
    effect_id: &str,
    authority: &str,
    tokens: &[&str],
) -> AppResult<TokenClosure> {
    if effect_id.is_empty()
        || authority.len() != 64
        || !authority.bytes().all(|b| b.is_ascii_hexdigit())
        || tokens.is_empty()
        || tokens.iter().any(|token| token.is_empty())
    {
        return Err(invalid());
    }
    let mut aliases = BTreeSet::new();
    for token in tokens {
        add_oauth_token(&mut aliases, token)?;
    }
    // Receipts remain exact-byte and authority bound. Aliases only broaden
    // refusal/closure, never positive upstream confirmation.
    let tokens = tokens
        .iter()
        .map(|token| fingerprint(token.as_bytes()))
        .collect::<AppResult<BTreeSet<_>>>()?;
    let context = (db, effect_id, authority, &tokens, &aliases);
    super::transaction_service::run_db_only(db, &context, |tx, context| {
        let &(db, effect_id, authority, tokens, aliases) = context;
        Box::pin(async move {
            if !discovery_complete_in_session(db, tx).await? {
                return Err(AppError::Conflict(
                    "Token copy discovery is incomplete".into(),
                ));
            }
            // Serialize every accepted bearer identity with writers, including
            // reverse-direction OWS variants imported as OAuth token material.
            for alias in aliases {
                let fence = db
                    .collection::<Document>(FENCES)
                    .find_one_and_update(
                        doc! {"_id":alias},
                        doc! {"$inc":{"revision":1_i64},"$setOnInsert":{"contract":CONTRACT}},
                    )
                    .upsert(true)
                    .return_document(mongodb::options::ReturnDocument::After)
                    .session(&mut *tx)
                    .await?
                    .ok_or_else(invalid)?;
                let owner = fence_owner(&fence)?;
                if db
                    .collection::<Document>(HOLDERS)
                    .find_one(doc! {"tokens":alias})
                    .session(&mut *tx)
                    .await?
                    .is_some()
                {
                    return Err(AppError::Conflict(
                        "Another credential retains this token; revocation requires reconciliation"
                            .into(),
                    ));
                }
                if owner.is_none() {
                    db.collection::<Document>(FENCES)
                        .update_one(doc! {"_id":alias}, doc! {"$set":{"closed_by":effect_id}})
                        .session(&mut *tx)
                        .await?;
                }
            }
            let mut confirmed = BTreeSet::new();
            let mut waiting_on = BTreeSet::new();
            for token in tokens {
                let result = db
                    .collection::<Document>(RESULTS)
                    .find_one_and_update(
                        doc! {"_id":format!("{token}:{authority}")},
                        doc! {"$inc":{"revision":1_i64},"$setOnInsert":{"contract":CONTRACT}},
                    )
                    .upsert(true)
                    .return_document(mongodb::options::ReturnDocument::After)
                    .session(&mut *tx)
                    .await?
                    .ok_or_else(invalid)?;
                let owner = fence_owner(&result)?;
                let is_confirmed = match result.get("confirmed") {
                    None | Some(bson::Bson::Boolean(false)) => false,
                    Some(bson::Bson::Boolean(true)) if owner.is_some() => true,
                    Some(_) => return Err(invalid()),
                };
                if is_confirmed {
                    confirmed.insert(token.clone());
                    continue;
                }
                if let Some(owner) = owner.filter(|closed| *closed != effect_id) {
                    waiting_on.insert(owner.to_owned());
                    continue;
                }
                db.collection::<Document>(RESULTS)
                    .update_one(
                        doc! {"_id":format!("{token}:{authority}")},
                        doc! {"$set":{"closed_by":effect_id}},
                    )
                    .session(&mut *tx)
                    .await?;
            }
            Ok(TokenClosure {
                confirmed,
                waiting_on,
            })
        })
    })
    .await
}

/// A positive upstream result is monotonic. A stale/different effect cannot
/// manufacture it, and a failed/unknown result never releases the tombstone.
pub(crate) async fn confirm_result(
    db: &Database,
    effect_id: &str,
    authority: &str,
    token: &str,
) -> AppResult<()> {
    if effect_id.is_empty() || token.is_empty() {
        return Err(invalid());
    }
    let durable = super::transaction_service::durable_database(db);
    let changed = durable.collection::<Document>(RESULTS).update_one(
        doc! {"_id":format!("{}:{authority}",fingerprint(token.as_bytes())?),"contract":CONTRACT,"closed_by":effect_id},
        doc! {"$set":{"confirmed":true,"confirmed_at":bson::DateTime::now()}},
    ).await?;
    if changed.matched_count != 1 {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) async fn results_confirmed(
    db: &Database,
    authority: &str,
    tokens: &[&str],
) -> AppResult<bool> {
    if tokens.is_empty() || tokens.iter().any(|token| token.is_empty()) {
        return Err(invalid());
    }
    for token in tokens {
        let id = format!("{}:{authority}", fingerprint(token.as_bytes())?);
        let Some(row) = db
            .collection::<Document>(RESULTS)
            .find_one(doc! {"_id":id})
            .await?
        else {
            return Ok(false);
        };
        if fence_owner(&row)?.is_none() || row.get_bool("confirmed").ok() != Some(true) {
            return Ok(false);
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::super::user_api_key_service::{self as keys_service, CreateApiKeyParams};
    use super::*;
    const AUTHORITY: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn params<'a>(token: &'a str, provider: &'a str) -> CreateApiKeyParams<'a> {
        CreateApiKeyParams {
            label: "Imported authorization",
            credential_type: "oauth2",
            credential: "",
            access_token: Some(token),
            refresh_token: None,
            token_scopes: Some("read"),
            expires_at: None,
            provider_config_id: Some(provider),
            connection_id: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            status: "active",
            source: None,
            source_id: None,
        }
    }
    async fn fixture(name: &str) -> (Database, EncryptionKeys) {
        crate::test_utils::init_test_audit_chain_hmac_key();
        let db = crate::test_utils::connect_transaction_test_database(name).await;
        (db, crate::test_utils::test_encryption_keys())
    }
    async fn finish_discovery(db: &Database, keys: &EncryptionKeys) {
        for _ in 0..32 {
            if !discover_one(db, keys).await.unwrap() {
                return;
            }
        }
        panic!("bounded token discovery did not finish");
    }

    #[tokio::test]
    async fn actual_platform_writers_retain_copies_and_rollback_closed_token_imports() {
        use super::super::{
            platform_credential_service as platform,
            provider_token_exchange_service::TokenExchangeCache,
        };
        let (db, keys) = fixture("platform_token_admission").await;
        let actor = uuid::Uuid::new_v4().to_string();
        let provider_id = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>("provider_configs").insert_one(doc! {
            "_id":&provider_id,"slug":"twitter","name":"Twitter","provider_type":"oauth2",
            "is_active":true,"created_by":&actor,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
        }).await.unwrap();
        let cache = std::sync::Arc::new(TokenExchangeCache::new());
        begin_discovery(&db, true).await.unwrap();
        finish_discovery(&db, &keys).await;
        for (provider, field) in [
            ("telegram-new", "manager_bot_token"),
            ("x", "app_bearer_token"),
        ] {
            let descriptor = platform::descriptor(&cache, provider).unwrap().1;
            let token = format!("retained-{provider}-token");
            let fields = [(
                field.to_owned(),
                Some(zeroize::Zeroizing::new(token.clone())),
            )]
            .into_iter()
            .collect();
            platform::update(&db, &keys, &descriptor, &actor, &fields, false)
                .await
                .unwrap();
            let stored = platform::load_decrypted(&db, &keys, &descriptor)
                .await
                .unwrap();
            assert_eq!(stored.get(field), Some(token.as_str()));
            assert!(matches!(
                close_for_dispatch(&db, "platform-effect", AUTHORITY, &[&token]).await,
                Err(AppError::Conflict(_))
            ));
            platform::delete(&db, &descriptor).await.unwrap();
            assert!(
                close_for_dispatch(&db, "platform-effect", AUTHORITY, &[&token])
                    .await
                    .is_ok()
            );
            // Admission failure must roll back the real writer, not merely its
            // derived holder. Composite backing also preserves the provider.
            assert!(
                platform::update(&db, &keys, &descriptor, &actor, &fields, false)
                    .await
                    .is_err()
            );
            assert!(
                db.collection::<Document>("platform_credentials")
                    .find_one(doc! {"provider":provider})
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(
                db.collection::<Document>("provider_configs")
                    .find_one(doc! {"_id":&provider_id})
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn oauth_ows_identity_is_symmetric_without_reusing_distinct_request_receipts() {
        for (index, (removed_token, retained_token)) in [
            (" \treverse-copy-token \t", "reverse-copy-token"),
            ("reverse-copy-token", " \treverse-copy-token \t"),
        ]
        .into_iter()
        .enumerate()
        {
            let (db, keys) = fixture(&format!("token_reverse_ows_{index}")).await;
            let person = uuid::Uuid::new_v4().to_string();
            let org = uuid::Uuid::new_v4().to_string();
            let removed = keys_service::create_api_key(
                &db,
                &keys,
                &person,
                params(removed_token, "provider-a"),
            )
            .await
            .unwrap();
            let retained = keys_service::create_api_key(
                &db,
                &keys,
                &org,
                params(retained_token, "provider-b"),
            )
            .await
            .unwrap();
            // Admission preserves the exact imported material.
            let stored = db
                .collection::<Document>("user_api_keys")
                .find_one(doc! {"_id":&retained.id})
                .await
                .unwrap()
                .unwrap();
            let bytes = super::super::record_field_service::stored_bytes(
                stored.get("access_token_encrypted").unwrap(),
            )
            .unwrap();
            assert_eq!(
                super::super::credential_record_service::decrypt_stored_field(
                    &keys,
                    "user_api_keys",
                    &stored,
                    "access_token_encrypted",
                    &bytes,
                )
                .await
                .unwrap()
                .as_slice(),
                retained_token.as_bytes()
            );
            begin_discovery(&db, true).await.unwrap();
            finish_discovery(&db, &keys).await;
            keys_service::delete_api_key(&db, &person, &removed.id)
                .await
                .unwrap();
            assert!(matches!(
                close_for_dispatch(&db, "removed-effect", AUTHORITY, &[removed_token],).await,
                Err(AppError::Conflict(_))
            ));
            keys_service::delete_api_key(&db, &org, &retained.id)
                .await
                .unwrap();
            close_for_dispatch(&db, "removed-effect", AUTHORITY, &[removed_token])
                .await
                .unwrap();
            confirm_result(&db, "removed-effect", AUTHORITY, removed_token)
                .await
                .unwrap();
            assert!(
                results_confirmed(&db, AUTHORITY, &[removed_token])
                    .await
                    .unwrap()
            );
            assert!(
                !results_confirmed(&db, AUTHORITY, &[retained_token])
                    .await
                    .unwrap()
            );
            assert!(
                !close_for_dispatch(&db, "distinct-request", AUTHORITY, &[retained_token])
                    .await
                    .unwrap()
                    .confirmed(retained_token)
                    .unwrap()
            );
            // Both representations are permanently closed for late imports.
            for token in [removed_token, retained_token] {
                assert!(
                    keys_service::create_api_key(&db, &keys, &org, params(token, "provider-c"),)
                        .await
                        .is_err()
                );
            }
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn catalog_ows_and_raw_or_multispace_header_copies_block_dispatch_and_late_writes() {
        use crate::models::{
            default_request_header::DefaultRequestHeader, downstream_service::DownstreamService,
        };
        let (db, keys) = fixture("token_catalog_header_copies").await;
        let owner = uuid::Uuid::new_v4().to_string();
        let org = uuid::Uuid::new_v4().to_string();
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = uuid::Uuid::new_v4().to_string();
        catalog.owner_user_id = Some(org.clone());
        catalog.auth_method = "bearer".into();
        catalog.credential_encrypted = super::super::catalog_credential_record_service::encrypt(
            &keys,
            &catalog.id,
            &org,
            b" \tcatalog-copy-token \t",
        )
        .await
        .unwrap();
        catalog.secret_context_versions =
            super::super::catalog_credential_record_service::initial_versions(&keys, true);
        db.collection::<DownstreamService>("downstream_services")
            .insert_one(&catalog)
            .await
            .unwrap();
        let key = keys_service::create_api_key(
            &db,
            &keys,
            &owner,
            params("catalog-copy-token", "another-provider"),
        )
        .await
        .unwrap();
        begin_discovery(&db, true).await.unwrap();
        finish_discovery(&db, &keys).await;
        keys_service::delete_api_key(&db, &owner, &key.id)
            .await
            .unwrap();
        assert!(matches!(
            close_for_dispatch(&db, "catalog-effect", AUTHORITY, &["catalog-copy-token"]).await,
            Err(AppError::Conflict(_))
        ));
        // The stored bytes are unchanged: normalization is comparison evidence,
        // never a rewrite of the credential supplied to the upstream client.
        assert_eq!(
            super::super::catalog_credential_record_service::text(&keys, &catalog)
                .await
                .unwrap()
                .as_str(),
            " \tcatalog-copy-token \t"
        );
        for (index, value) in ["raw-header-token", "Bearer  header-copy-token"]
            .into_iter()
            .enumerate()
        {
            let token = if index == 0 {
                "raw-header-token"
            } else {
                "header-copy-token"
            };
            let source = db
                .collection::<Document>("downstream_services")
                .find_one(doc! {"_id":&catalog.id})
                .await
                .unwrap()
                .unwrap();
            let headers = super::super::default_header_service::encode(
                Some(&keys),
                "downstream_services",
                &catalog.id,
                &org,
                vec![DefaultRequestHeader {
                    name: "X-Copied-Token".into(),
                    value: value.to_owned().into(),
                    sensitive: index == 0,
                    overridable: false,
                }],
            )
            .await
            .unwrap();
            let set = doc! {"default_request_headers":bson::to_bson(&headers).unwrap()};
            assert!(
                migrate_storage(&db, &keys, "downstream_services", &source, &set)
                    .await
                    .unwrap()
            );
            assert!(matches!(
                close_for_dispatch(&db, "header-effect", AUTHORITY, &[token]).await,
                Err(AppError::Conflict(_))
            ));
        }
        // Both outcomes are safe under a real concurrent writer/closure race:
        // one transaction wins, and the loser cannot install unindexed material
        // or dispatch while its matching holder exists.
        for index in 0..2 {
            let token = format!("concurrent-header-token-{index}");
            let source = db
                .collection::<Document>("downstream_services")
                .find_one(doc! {"_id":&catalog.id})
                .await
                .unwrap()
                .unwrap();
            let value = if index == 0 {
                token.clone()
            } else {
                format!("Bearer  {token}")
            };
            let headers = super::super::default_header_service::encode(
                Some(&keys),
                "downstream_services",
                &catalog.id,
                &org,
                vec![DefaultRequestHeader {
                    name: "X-Copied-Token".into(),
                    value: value.into(),
                    sensitive: false,
                    overridable: false,
                }],
            )
            .await
            .unwrap();
            let set = doc! {"default_request_headers":bson::to_bson(&headers).unwrap()};
            let token_refs = [token.as_str()];
            let (installed, closed) = tokio::join!(
                migrate_storage(&db, &keys, "downstream_services", &source, &set),
                close_for_dispatch(&db, "concurrent-effect", AUTHORITY, &token_refs)
            );
            assert_ne!(
                installed.is_ok_and(|changed| changed),
                closed.is_ok(),
                "writer and dispatch both admitted equal token"
            );
        }
        let protocol_id = uuid::Uuid::new_v4().to_string();
        assert!(repair_record_uuid("oauth_states", &protocol_id).is_none());
        assert!(repair_record_uuid("user_api_keys", &protocol_id).is_some());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn old_complete_marker_and_retained_catalog_copy_cannot_activate_dispatch() {
        let (db, keys) = fixture("token_admission_coverage_upgrade").await;
        let personal = uuid::Uuid::new_v4().to_string();
        let retained_owner = uuid::Uuid::new_v4().to_string();
        let key = keys_service::create_api_key(
            &db,
            &keys,
            &personal,
            params("same-catalog-token", "provider-a"),
        )
        .await
        .unwrap();
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = uuid::Uuid::new_v4().to_string();
        catalog.owner_user_id = Some(retained_owner.clone());
        catalog.auth_method = "bearer".into();
        catalog.credential_encrypted = super::super::catalog_credential_record_service::encrypt(
            &keys,
            &catalog.id,
            &retained_owner,
            b"same-catalog-token",
        )
        .await
        .unwrap();
        catalog.secret_context_versions =
            super::super::catalog_credential_record_service::initial_versions(&keys, true);
        db.collection::<crate::models::downstream_service::DownstreamService>(
            "downstream_services",
        )
        .insert_one(&catalog)
        .await
        .unwrap();
        db.collection::<Document>(DISCOVERY)
            .insert_one(doc! {"_id":"v1","status":"complete","contract":CONTRACT})
            .await
            .unwrap();
        assert_eq!(
            super::super::catalog_credential_record_service::text(&keys, &catalog)
                .await
                .unwrap()
                .as_str(),
            "same-catalog-token"
        );
        assert!(
            begin_discovery(&db, true).await.is_err(),
            "legacy complete marker silently reused"
        );
        let actor = super::super::audit_service::AuditActor {
            user_id: personal.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        };
        assert!(
            activate_discovery(&db, true, &actor).await.is_err(),
            "incomplete catalog coverage was waived"
        );
        keys_service::delete_api_key(&db, &personal, &key.id)
            .await
            .unwrap();
        assert!(
            close_for_dispatch(&db, "effect", AUTHORITY, &["same-catalog-token"])
                .await
                .is_err()
        );
        assert!(
            db.collection::<Document>("downstream_services")
                .find_one(doc! {"_id":&catalog.id})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn audited_discovery_retains_blocked_cursor_and_resumes_after_real_credential_repair() {
        let (db, keys) = fixture("token_admission_block_repair").await;
        let owner = uuid::Uuid::new_v4().to_string();
        let mut input = params("original", "provider");
        input.credential_type = "bearer";
        input.credential = "original";
        input.access_token = None;
        let key = keys_service::create_api_key(&db, &keys, &owner, input)
            .await
            .unwrap();
        // A legacy source which cannot authenticate remains a blocker; the
        // transaction must not count it or advance beyond its row.
        db.collection::<Document>("user_api_keys").update_one(doc! {"_id":&key.id},doc! {"$set":{"credential_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:vec![9,8,7]}}}).await.unwrap();
        let actor = super::super::audit_service::AuditActor {
            user_id: owner.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        };
        // Exercise the same atomic activation transaction independently of
        // the control-plane coverage gate, which remains closed above.
        begin_discovery_with_audit(&db, true, Some(&actor))
            .await
            .unwrap();
        let generation = discovery_status(&db).await.unwrap().unwrap().generation;
        let blocked = advance_discovery(&db, &keys, &generation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(blocked.status, "blocked");
        assert_eq!(blocked.observed_rows, 0);
        assert_eq!(
            blocked.repair_record_uuid,
            Some(uuid::Uuid::parse_str(&key.id).unwrap())
        );
        assert_eq!(
            db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(doc! {"event_type":"token_admission.activated"})
                .await
                .unwrap(),
            1
        );
        keys_service::update_api_key(&db, &keys, &owner, &key.id, None, Some("repaired"))
            .await
            .unwrap();
        let complete = advance_discovery(&db, &keys, &generation)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(complete.status, "complete");
        assert_eq!(complete.observed_rows, 1);
        assert!(complete.repair_reason.is_none());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn activation_serializes_held_absent_marker_writer_and_indexes_below_cursor_insert() {
        let (db, keys) = fixture("token_admission_activation").await;
        assert!(begin_discovery(&db, false).await.is_err());
        let owner = uuid::Uuid::new_v4().to_string();
        let provider = uuid::Uuid::new_v4().to_string();
        let held_id = "00000000-0000-4000-8000-000000000001";
        let held = keys_service::build_api_key_with_id(
            &keys,
            &owner,
            held_id,
            params("held-import", &provider),
        )
        .await
        .unwrap();
        let mut tx = db.client().start_session().await.unwrap();
        tx.start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .write_concern(mongodb::options::WriteConcern::majority())
            .await
            .unwrap();
        db.collection::<Document>("user_api_keys")
            .insert_one(bson::to_document(&held).unwrap())
            .session(&mut tx)
            .await
            .unwrap();
        write_in_session(
            &db,
            Some(&keys),
            "user_api_keys",
            &bson::to_document(&held).unwrap(),
            &mut tx,
        )
        .await
        .unwrap();
        let activation = begin_discovery(&db, true);
        tokio::pin!(activation);
        // MongoDB may wait for the conflicting writer or return a definite
        // lock/write conflict immediately. Neither permits an activation marker.
        // Retain the actual result so a successful activation can never pass.
        let early =
            tokio::time::timeout(std::time::Duration::from_millis(100), &mut activation).await;
        match &early {
            Ok(Ok(())) => panic!("activation crossed a held absent-marker writer"),
            Ok(Err(error)) => assert!(
                matches!(error, AppError::Conflict(_) | AppError::DatabaseError(_)),
                "unexpected activation failure: {error:?}"
            ),
            Err(_) => {}
        }
        assert!(
            db.collection::<Document>(DISCOVERY)
                .find_one(doc! {"_id":"v1"})
                .await
                .unwrap()
                .is_none()
        );
        super::super::transaction_service::commit(&mut tx)
            .await
            .unwrap();
        if early.is_err() {
            tokio::time::timeout(std::time::Duration::from_secs(10), activation)
                .await
                .unwrap()
                .unwrap();
        } else {
            // The previous attempt definitely aborted before creating a marker.
            begin_discovery(&db, true).await.unwrap();
        }
        assert!(discover_one(&db, &keys).await.unwrap());
        // The discovery cursor has passed the held ID. This new, lower ID
        // participates in admission itself and cannot hide below that cursor.
        let late_id = "00000000-0000-4000-8000-000000000000";
        crate::test_utils::create_api_key_with_id(
            &db,
            &keys,
            &owner,
            late_id,
            params("below-cursor", &provider),
        )
        .await
        .unwrap();
        finish_discovery(&db, &keys).await;
        for (id, token) in [(held_id, "held-import"), (late_id, "below-cursor")] {
            assert!(db.collection::<Document>(HOLDERS).find_one(
                doc! {"_id":holder_id("user_api_keys", id).unwrap(),"tokens":fingerprint(token.as_bytes()).unwrap()},
            ).await.unwrap().is_some());
            assert!(
                close_for_dispatch(&db, "effect-held", AUTHORITY, &[token])
                    .await
                    .is_err()
            );
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn cross_provider_copy_and_post_ready_import_block_dispatch_then_closed_token_cannot_return()
     {
        let (db, keys) = fixture("token_admission_copies").await;
        let person = uuid::Uuid::new_v4().to_string();
        let org = uuid::Uuid::new_v4().to_string();
        let provider_a = uuid::Uuid::new_v4().to_string();
        let provider_b = uuid::Uuid::new_v4().to_string();
        let personal = keys_service::create_api_key(
            &db,
            &keys,
            &person,
            params("same-raw-token", &provider_a),
        )
        .await
        .unwrap();
        begin_discovery(&db, true).await.unwrap();
        finish_discovery(&db, &keys).await;
        let copied =
            keys_service::create_api_key(&db, &keys, &org, params("same-raw-token", &provider_b))
                .await
                .unwrap();
        keys_service::delete_api_key(&db, &person, &personal.id)
            .await
            .unwrap();
        assert!(
            close_for_dispatch(&db, "delete-person", AUTHORITY, &["same-raw-token"])
                .await
                .is_err()
        );
        keys_service::delete_api_key(&db, &org, &copied.id)
            .await
            .unwrap();
        close_for_dispatch(&db, "last-copy", AUTHORITY, &["same-raw-token"])
            .await
            .unwrap();
        // Idempotent same-effect retry survives a timeout, but the tombstone
        // rejects another effect and a provider returning the same token late.
        close_for_dispatch(&db, "last-copy", AUTHORITY, &["same-raw-token"])
            .await
            .unwrap();
        assert!(
            !close_for_dispatch(&db, "new-effect", AUTHORITY, &["same-raw-token"])
                .await
                .unwrap()
                .waiting_on
                .is_empty()
        );
        assert!(
            keys_service::create_api_key(&db, &keys, &org, params("same-raw-token", &provider_b))
                .await
                .is_err()
        );
        assert_eq!(
            db.collection::<Document>("user_api_keys")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn unrelated_writers_do_not_restart_or_extend_the_discovery_walk() {
        let (db, keys) = fixture("token_admission_progress").await;
        let owner = uuid::Uuid::new_v4().to_string();
        let provider = uuid::Uuid::new_v4().to_string();
        keys_service::create_api_key(&db, &keys, &owner, params("legacy-token", &provider))
            .await
            .unwrap();
        begin_discovery(&db, true).await.unwrap();
        let mut complete = false;
        for index in 0..8 {
            keys_service::create_api_key(
                &db,
                &keys,
                &uuid::Uuid::new_v4().to_string(),
                params(&format!("unrelated-{index}"), &provider),
            )
            .await
            .unwrap();
            if !discover_one(&db, &keys).await.unwrap() {
                complete = true;
                break;
            }
        }
        assert!(
            complete,
            "unrelated credential inserts starved a finite discovery pass"
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn malformed_tombstone_and_empty_dispatch_material_fail_closed_without_repairing_storage()
    {
        let (db, keys) = fixture("token_admission_malformed").await;
        begin_discovery(&db, true).await.unwrap();
        finish_discovery(&db, &keys).await;
        assert!(
            close_for_dispatch(&db, "effect", AUTHORITY, &[])
                .await
                .is_err()
        );
        assert!(
            close_for_dispatch(&db, "effect", AUTHORITY, &[""])
                .await
                .is_err()
        );
        for (index, mut row) in [
            doc! {"contract":CONTRACT,"closed_by":bson::Bson::Null},
            doc! {"contract":CONTRACT,"closed_by":17_i32},
            doc! {"contract":CONTRACT,"closed_by":""},
            doc! {"contract":CONTRACT+1,"closed_by":"original-effect"},
            doc! {"closed_by":"original-effect"},
        ]
        .into_iter()
        .enumerate()
        {
            let token = format!("malformed-{index}");
            let fingerprint = fingerprint(token.as_bytes()).unwrap();
            row.insert("_id", &fingerprint);
            row.insert("revision", 1_i64);
            db.collection::<Document>(FENCES)
                .insert_one(&row)
                .await
                .unwrap();
            assert!(
                close_for_dispatch(&db, "replacement-effect", AUTHORITY, &[&token])
                    .await
                    .is_err()
            );
            assert_eq!(
                db.collection::<Document>(FENCES)
                    .find_one(doc! {"_id":&fingerprint})
                    .await
                    .unwrap()
                    .unwrap(),
                row
            );
        }
        db.drop().await.unwrap();
    }
}

#[cfg(test)]
mod platform_digest_tests {
    use super::*;
    #[test]
    fn platform_plain_metadata_is_part_of_the_holder_source_identity() {
        let before =
            doc! {"_id":"platform", "provider":"telegram-new", "fields":{"public_label":"one"}};
        let after =
            doc! {"_id":"platform", "provider":"telegram-new", "fields":{"public_label":"two"}};
        assert!(material_changed(Some(&before), &after));
        assert_ne!(source_hash(&before).unwrap(), source_hash(&after).unwrap());
    }
}
