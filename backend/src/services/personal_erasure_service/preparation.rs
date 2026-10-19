//! Bounded provenance preparation leaves account authority unchanged. Plaintext
//! token fingerprints survive ciphertext rewrap/rebind and never expose tokens.
use super::*;
use crate::crypto::aes::EncryptionKeys;
use sha2::{Digest, Sha256};
pub(super) const PREPARATIONS: &str = "personal_erasure_preparations";
const VERSION: i32 = 5;
const DEPENDENCIES: &str = "credential_revocation_dependencies";

pub(super) async fn ensure_indexes(db: &Database) -> AppResult<()> {
    for keys in [
        doc! {"owner_user_id":1,"_id":1},
        doc! {"token_fingerprints":1,"owner_user_id":1},
        doc! {"grant_origins":1,"owner_user_id":1},
        doc! {"connection_id":1,"owner_user_id":1},
        doc! {"source_id":1,"owner_user_id":1},
    ] {
        db.collection::<Document>(DEPENDENCIES)
            .create_index(mongodb::IndexModel::builder().keys(keys).build())
            .await?;
    }
    for collection in COLLECTIONS {
        db.collection::<Document>(collection)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! {"provider_config_id":1,"_id":1})
                    .build(),
            )
            .await?;
        db.collection::<Document>(collection)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! {"user_id":1,"_id":1})
                    .build(),
            )
            .await?;
    }
    Ok(())
}

pub(super) async fn require_ready_in_session(
    db: &Database,
    user: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    // Acceptance performs point reads only. Every credential/copy/transfer writer
    // must fence this person and bump this revision with its dependency change.
    let owner = db
        .collection::<Document>(USERS)
        .find_one(doc! {"_id":user})
        .session(&mut *tx)
        .await?
        .ok_or_else(|| AppError::Conflict("Credential owner changed".into()))?;
    let revision = owner
        .get("credential_dependency_revision")
        .cloned()
        .unwrap_or(bson::Bson::Int64(0));
    let inventory_revision =
        super::super::erasure_admission::credential_inventory_in_session(db, tx, false).await?;
    let prepared = db
        .collection::<Document>(PREPARATIONS)
        .find_one(doc! {"_id":user,"status":"ready","source_revision":revision,"inventory_revision":inventory_revision,"version":VERSION})
        .session(&mut *tx)
        .await?;
    if let Some(prepared) = prepared {
        return if prepared.get_bool("shared_dependency").unwrap_or(true) {
            Err(AppError::Conflict("Another owner retains a related OAuth connection or token copy; separate or reconnect that shared grant before deleting this account".into()))
        } else {
            Ok(())
        };
    }
    if !provider_ids_in_session(db, user, tx).await?.is_empty() {
        return Err(AppError::Conflict("Credential dependency preparation is required; retry deletion while the account remains usable".into()));
    }
    Ok(())
}
const FIELDS: [&str; 4] = [
    "access_token_encrypted",
    "refresh_token_encrypted",
    "credential_encrypted",
    "api_key_encrypted",
];
// Bind exact authenticated source and app/owner lineage with a storage digest.
// Persisting a second ciphertext snapshot would retain hidden key dependencies.
// Real credential writers must invalidate/bump dependency authority atomically.
const SOURCE_FIELDS: &[&str] = &[
    "user_id",
    "provider_config_id",
    "credential_user_id",
    "connection_id",
    "source_id",
    "credential_source",
    "oauth_app_observation",
    "credential_epoch",
    "secret_context_versions",
    "grant_lineage",
    "access_token_encrypted",
    "refresh_token_encrypted",
    "credential_encrypted",
    "api_key_encrypted",
    "user_oauth_client_id_encrypted",
    "user_oauth_client_secret_encrypted",
];

fn source_snapshot(row: &Document) -> Document {
    SOURCE_FIELDS
        .iter()
        .map(|field| {
            (
                (*field).to_owned(),
                row.get(*field).cloned().unwrap_or(bson::Bson::Null),
            )
        })
        .collect()
}

fn source_hash(row: &Document) -> AppResult<String> {
    let bytes = bson::to_vec(&source_snapshot(row))
        .map_err(|_| AppError::Internal("Credential source encoding failed".into()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}
const COLLECTIONS: [&str; 2] = [
    crate::models::user_api_key::COLLECTION_NAME,
    crate::models::user_provider_token::COLLECTION_NAME,
];

fn secret_rows() -> Document {
    // GCP service-account cached access has no per-token revocation API or
    // provider-config lineage. Its explicit adapter preserves an unavailable
    // revocation disposition; it must not enter OAuth grant attribution.
    doc! {"credential_type":{"$ne":"gcp_service_account"},"$or":[{"access_token_encrypted":{"$ne":bson::Bson::Null}},{"refresh_token_encrypted":{"$ne":bson::Bson::Null}}]}
}

/// Discover personal and canonically owned SA providers under the same
/// snapshot/person revision. Each child lookup uses the credential owner
/// index; creator attribution on an explicitly other-owned SA is excluded.
// Empty is an internal inventory bucket for absent/null attribution. It is
// persisted as BSON null and never used as a fabricated provider identity.
fn provider_bucket(group: &Document) -> AppResult<String> {
    match group.get("_id") {
        None | Some(bson::Bson::Null) => Ok(String::new()),
        Some(bson::Bson::String(provider)) if !provider.is_empty() => Ok(provider.clone()),
        _ => Err(AppError::Conflict(
            "OAuth provider attribution has an invalid shape".into(),
        )),
    }
}

async fn provider_ids_in_session(
    db: &Database,
    user: &str,
    tx: &mut ClientSession,
) -> AppResult<std::collections::BTreeSet<String>> {
    use futures::TryStreamExt;
    let mut providers = std::collections::BTreeSet::new();
    for collection in COLLECTIONS {
        let mut filter = secret_rows();
        filter.insert("user_id", user);
        let mut cursor = db
            .collection::<Document>(collection)
            .aggregate([
                doc! {"$match":filter},
                doc! {"$group":{"_id":"$provider_config_id"}},
            ])
            .session(&mut *tx)
            .await?;
        while let Some(row) = cursor.stream(&mut *tx).try_next().await? {
            providers.insert(provider_bucket(&row)?);
        }
        let mut cursor = db.collection::<Document>(crate::models::service_account::COLLECTION_NAME).aggregate([
            doc! {"$match":{"$or":[{"owner_user_id":user},{"owner_user_id":bson::Bson::Null,"created_by":user}]}},
            doc! {"$lookup":{"from":collection,"localField":"_id","foreignField":"user_id",
                "pipeline":[{"$match":secret_rows()},{"$group":{"_id":"$provider_config_id"}}],"as":"providers"}},
            doc! {"$unwind":"$providers"},doc! {"$group":{"_id":"$providers._id"}}
        ]).session(&mut *tx).await?;
        while let Some(row) = cursor.stream(&mut *tx).try_next().await? {
            providers.insert(provider_bucket(&row)?);
        }
    }
    Ok(providers)
}

/// The API can report preparation in progress and retain sign-in capability.
/// The worker does real bounded work; there is no permanent cardinality limit.
pub(crate) async fn ready_or_enqueue(db: &Database, user: &str) -> AppResult<bool> {
    let db = super::super::transaction_service::durable_database(db);
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result:AppResult<bool>=async {
        // Discover the inventory and its generation under the SAME source
        // writer fence/snapshot. A newly introduced provider cannot be omitted
        // while its successor owner revision is accepted as prepared.
        let owner=db.collection::<Document>(USERS).find_one_and_update(doc! {"_id":user,"is_active":true,"platform_personal_deleting":{"$ne":true}},doc! {"$inc":{"credential_preparation_fence":1_i64}}).session(&mut tx).await?
            .ok_or_else(||AppError::Conflict("Credential owner changed".into()))?;
        let revision=owner.get("credential_dependency_revision").cloned().unwrap_or(bson::Bson::Int64(0));
        let inventory_revision=super::super::erasure_admission::credential_inventory_in_session(&db,&mut tx,false).await?;
        if let Some(job)=db.collection::<Document>(PREPARATIONS).find_one(doc! {"_id":user,"source_revision":&revision,"inventory_revision":inventory_revision,"version":VERSION}).session(&mut tx).await? {
            if job.get_str("status")==Ok("ready") {return Ok(true);}
            if job.get_str("status")==Ok("preparing") {return Ok(false);}
        }
        let providers=provider_ids_in_session(&db,user,&mut tx).await?;
        if providers.is_empty() {return Ok(true);}
        let mut ceilings = Document::new();
        for collection in COLLECTIONS {
            let last = db.collection::<Document>(collection).find_one(doc! {}).sort(doc! {"_id":-1})
                .projection(doc! {"_id":1}).session(&mut tx).await?;
            ceilings.insert(collection, last.and_then(|row|row.get("_id").cloned()).unwrap_or(bson::Bson::Null));
        }
        db.collection::<Document>(PREPARATIONS).update_one(doc! {"_id":user},doc! {"$set":{"version":VERSION,"status":"preparing","generation":uuid::Uuid::new_v4().to_string(),"phase":"scan","collection_index":0_i32,
            "cursor":bson::Bson::Null,"ceilings":ceilings,"source_revision":revision,"inventory_revision":inventory_revision,"provider_ids":providers.into_iter().map(|provider| if provider.is_empty() {bson::Bson::Null} else {bson::Bson::String(provider)}).collect::<Vec<_>>(),"shared_dependency":false,"lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null},"$setOnInsert":{"created_at":bson::DateTime::now()},"$currentDate":{"updated_at":true}}).upsert(true).session(&mut tx).await?;
        Ok(false)
    }.await;
    super::super::org_membership_authority::finish(tx, result).await
}

fn progress_filter(job: &Document) -> AppResult<Document> {
    let mut filter = doc! {"_id":job.get_str("_id").map_err(|_|AppError::Internal("Invalid preparation identity".into()))?,"status":"preparing",
    "generation":job.get("generation").cloned().unwrap_or(bson::Bson::Null),
    "lease_token":job.get("lease_token").cloned().unwrap_or(bson::Bson::Null),
    "$expr":{"$gt":["$lease_expires_at","$$NOW"]}};
    for field in ["source_revision", "phase", "cursor", "collection_index"] {
        filter.insert(field, job.get(field).cloned().unwrap_or(bson::Bson::Null));
    }
    Ok(filter)
}
async fn advance(db: &Database, job: &Document, set: Document) -> AppResult<()> {
    let result=db.collection::<Document>(PREPARATIONS).update_one(progress_filter(job)?,doc! {"$set":set,"$unset":{"lease_token":"","lease_expires_at":""},"$currentDate":{"updated_at":true}}).await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict(
            "Preparation lease/generation changed".into(),
        ));
    }
    Ok(())
}

pub(super) async fn process_one(db: &Database, keys: &EncryptionKeys) -> AppResult<bool> {
    use futures::TryStreamExt;
    let durable = super::super::transaction_service::durable_database(db);
    let db = &durable;
    let Some(job)=db.collection::<Document>(PREPARATIONS).find_one_and_update(
        doc! {"status":"preparing","version":VERSION,"$expr":{"$lte":[{"$ifNull":["$lease_expires_at",bson::DateTime::from_millis(0)]},"$$NOW"]}},
        vec![doc! {"$set":{"lease_token":uuid::Uuid::new_v4().to_string(),"lease_expires_at":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":120}},"updated_at":"$$NOW"}}],
    ).sort(doc! {"updated_at":1,"_id":1}).return_document(mongodb::options::ReturnDocument::After).await? else {return Ok(false);};
    let _user = job
        .get_str("_id")
        .map_err(|_| AppError::Internal("Invalid preparation identity".into()))?;

    if job.get_str("phase") == Ok("verify") {
        return verify_page(db, &job).await;
    }
    let collection_index = job.get_i32("collection_index").unwrap_or(0) as usize;
    let Some(collection) = COLLECTIONS.get(collection_index) else {
        advance(db, &job, doc! {"phase":"verify","cursor":bson::Bson::Null}).await?;
        return Ok(true);
    };
    // Provider labels and local app handles cannot bound token-copy identity.
    // Walk every credential material row in bounded pages, including generic
    // bearer/API-key imports which can contain the same opaque upstream token.
    let mut filter = doc! {"$or":FIELDS.iter().map(|field|doc! {*field:{"$ne":bson::Bson::Null}}).collect::<Vec<_>>()};
    let ceiling = job
        .get_document("ceilings")
        .ok()
        .and_then(|ceilings| ceilings.get(*collection))
        .cloned()
        .ok_or_else(|| AppError::Internal("Preparation scan ceiling is invalid".into()))?;
    let mut range = doc! {"$lte":ceiling};
    if let Ok(cursor) = job.get_str("cursor") {
        range.insert("$gt", cursor);
    }
    filter.insert("_id", range);
    let rows: Vec<Document> = db
        .collection::<Document>(collection)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(32)
        .await?
        .try_collect()
        .await?;
    for row in &rows {
        prepare_row(db, keys, collection, row).await?;
    }
    let set = if let Some(last) = rows.last() {
        doc! {"cursor":last.get_str("_id").map_err(|_|AppError::Internal("Credential identity is invalid".into()))?}
    } else {
        doc! {"collection_index":(collection_index+1) as i32,"cursor":bson::Bson::Null}
    };
    advance(db, &job, set).await?;
    Ok(true)
}

async fn prepare_row(
    db: &Database,
    keys: &EncryptionKeys,
    collection: &str,
    row: &Document,
) -> AppResult<()> {
    let id = row
        .get_str("_id")
        .map_err(|_| AppError::Internal("Invalid credential identity".into()))?;
    let owner = row
        .get_str("user_id")
        .map_err(|_| AppError::Internal("Invalid credential ownership".into()))?;
    let canonical_owner = super::super::erasure_admission::credential_owner(db, owner).await?;
    let mut predicate = doc! {"_id":id,"user_id":owner};
    let mut tokens = Vec::new();
    let mut unreadable = false;
    for field in FIELDS {
        let value = row.get(field).cloned().unwrap_or(bson::Bson::Null);
        predicate.insert(field, value.clone());
        if matches!(value, bson::Bson::Null) {
            continue;
        }
        #[derive(serde::Deserialize)]
        struct StoredBytes {
            #[serde(with = "crate::models::bson_bytes::required")]
            data: Vec<u8>,
        }
        let bytes = match bson::from_document::<StoredBytes>(doc! {"data":value}) {
            Ok(value) => value.data,
            Err(_) => {
                unreadable = true;
                continue;
            }
        };
        let storage_collection = match collection {
            crate::models::user_api_key::COLLECTION_NAME => {
                crate::models::user_api_key::COLLECTION_NAME
            }
            crate::models::user_provider_token::COLLECTION_NAME => {
                crate::models::user_provider_token::COLLECTION_NAME
            }
            _ => {
                return Err(AppError::Internal(
                    "Unsupported credential preparation source".into(),
                ));
            }
        };
        let decoded = super::super::credential_record_service::decrypt_stored_field(
            keys,
            storage_collection,
            row,
            field,
            &bytes,
        )
        .await;
        match decoded {
            Ok(token) if !token.is_empty() => tokens.push(
                super::super::audit_service::keyed_fingerprint(
                    super::super::audit_service::audit_chain_hmac_key(),
                    b"oauth-revocation-provenance-v1",
                    &token,
                )
                .ok_or_else(|| {
                    AppError::Internal("Credential provenance key is unavailable".into())
                })?,
            ),
            Ok(_) => {}
            Err(_) => unreadable = true,
        }
    }
    let grant_lineage =
        match super::super::oauth_grant_lineage_service::from_snapshot(keys, collection, row).await
        {
            Ok(lineage) => lineage,
            Err(_) => {
                unreadable = true;
                None
            }
        };
    let grant_origins = grant_lineage
        .map(|lineage| lineage.origins)
        .unwrap_or_default();
    let snapshot = source_snapshot(row);
    predicate.extend(snapshot.clone());
    let source_id = format!("{collection}:{id}");
    let source_hash = source_hash(row)?;
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result:AppResult<()> = async {
        // Contend on service-account ownership without treating preparation
        // itself as credential replacement or changing its dependency epoch.
        if owner != canonical_owner {
            let changed=db.collection::<Document>(crate::models::service_account::COLLECTION_NAME)
                .update_one(doc! {"_id":owner,"$or":[{"owner_user_id":&canonical_owner},{"owner_user_id":bson::Bson::Null,"created_by":&canonical_owner}]},doc! {"$inc":{"owner_lifecycle_fence":1_i64}}).session(&mut tx).await?;
            if changed.matched_count!=1 {return Err(AppError::Conflict("Credential target ownership changed".into()));}
        }
        let changed=db.collection::<Document>(collection).update_one(predicate,doc! {"$set":{"revocation_provenance":{"version":VERSION,
            "state":if unreadable {"unreadable"} else {"ready"},"token_fingerprints":&tokens,"source_hash":&source_hash}}}).session(&mut tx).await?;
        if changed.matched_count!=1 {return Ok(());}
        let dependency=doc! {"_id":&source_id,"source_collection":collection,"source_record_id":id,"owner_user_id":&canonical_owner,"resource_owner_id":owner,
            "provider_config_id":row.get("provider_config_id").cloned().unwrap_or(bson::Bson::Null),
            "connection_id":row.get("connection_id").cloned().unwrap_or(bson::Bson::Null),
            "source_id":row.get("source_id").cloned().unwrap_or(bson::Bson::Null),
            "grant_origins":grant_origins,"token_fingerprints":tokens,"state":if unreadable {"unreadable"} else {"ready"},"source_hash":source_hash};
        db.collection::<Document>(DEPENDENCIES).replace_one(doc! {"_id":source_id},dependency).upsert(true).session(&mut tx).await?;
        Ok(())
    }.await;
    super::super::org_membership_authority::finish(tx, result).await?;
    Ok(())
}

/// Each page makes at most32 indexed source queries and bounded indexed
/// equality lookups. Independent target owners are never scanned for no-match.
async fn verify_page(db: &Database, job: &Document) -> AppResult<bool> {
    use futures::TryStreamExt;
    let user = job
        .get_str("_id")
        .map_err(|_| AppError::Internal("Preparation identity is invalid".into()))?;
    let mut filter = doc! {"owner_user_id":user};
    if let Ok(cursor) = job.get_str("cursor") {
        filter.insert("_id", doc! {"$gt":cursor});
    }
    let rows: Vec<Document> = db
        .collection::<Document>(DEPENDENCIES)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(32)
        .await?
        .try_collect()
        .await?;
    let mut shared = job.get_bool("shared_dependency").unwrap_or(false);
    for row in &rows {
        let collection = row
            .get_str("source_collection")
            .map_err(|_| AppError::Internal("Invalid dependency source".into()))?;
        if !COLLECTIONS.contains(&collection) {
            return Err(AppError::Internal("Invalid dependency collection".into()));
        }
        let id = row
            .get_str("source_record_id")
            .map_err(|_| AppError::Internal("Invalid dependency identity".into()))?;
        let live = db
            .collection::<Document>(collection)
            .find_one(doc! {"_id":id})
            .await?;
        if live.as_ref().is_none_or(|source| {
            source.get_str("user_id").ok()
                != row
                    .get_str("resource_owner_id")
                    .or_else(|_| row.get_str("owner_user_id"))
                    .ok()
                || source_hash(source).ok().as_deref() != row.get_str("source_hash").ok()
        }) {
            db.collection::<Document>(DEPENDENCIES).delete_one(doc! {"_id":row.get("_id").cloned().unwrap(),"source_hash":row.get("source_hash").cloned().unwrap_or(bson::Bson::Null)}).await?;
            if live.is_some() {
                return Err(AppError::Conflict(
                    "Credential dependency changed; restart preparation".into(),
                ));
            }
            continue;
        }
        if row.get_str("state") != Ok("ready") {
            return Err(AppError::Conflict(
                "A credential requires authenticated repair before erasure".into(),
            ));
        }
        let mut alternatives = Vec::new();
        if let Ok(tokens) = row.get_array("token_fingerprints")
            && (!tokens.is_empty())
        {
            alternatives.push(doc! {"token_fingerprints":{"$in":tokens}});
        }
        if let Ok(origins) = row.get_array("grant_origins")
            && !origins.is_empty()
        {
            alternatives.push(doc! {"grant_origins":{"$in":origins}});
        }
        if let Ok(connection) = row.get_str("connection_id") {
            alternatives.push(doc! {"connection_id":connection});
            alternatives.push(doc! {"source_id":connection});
        }
        if let Ok(source) = row.get_str("source_record_id") {
            alternatives.push(doc! {"source_id":source});
        }
        if !alternatives.is_empty() {
            // Scan a bounded dependency page; every stale candidate is
            // removed using its exact source digest before retry. A no-match
            // uses the token/connection/source indexes, not a raw source scan.
            let candidates: Vec<Document> = db
                .collection::<Document>(DEPENDENCIES)
                .find(doc! {"owner_user_id":{"$ne":user},"$or":alternatives})
                .limit(32)
                .await?
                .try_collect()
                .await?;
            for candidate in &candidates {
                let collection = candidate
                    .get_str("source_collection")
                    .map_err(|_| AppError::Internal("Invalid dependency collection".into()))?;
                if !COLLECTIONS.contains(&collection) {
                    return Err(AppError::Internal("Invalid dependency source".into()));
                }
                let live=db.collection::<Document>(collection).find_one(doc! {"_id":candidate.get_str("source_record_id").map_err(|_|AppError::Internal("Invalid dependency identity".into()))?}).await?;
                if live.as_ref().is_some_and(|source| {
                    source.get_str("user_id").ok()
                        == candidate
                            .get_str("resource_owner_id")
                            .or_else(|_| candidate.get_str("owner_user_id"))
                            .ok()
                        && source_hash(source).ok().as_deref()
                            == candidate.get_str("source_hash").ok()
                }) {
                    shared = true;
                    break;
                }
                db.collection::<Document>(DEPENDENCIES).delete_one(doc! {"_id":candidate.get("_id").cloned().unwrap(),"source_hash":candidate.get("source_hash").cloned().unwrap_or(bson::Bson::Null)}).await?;
            }
            if candidates.len() == 32 && !shared {
                return Err(AppError::Conflict(
                    "Stale dependency page removed; resume preparation".into(),
                ));
            }
        }
    }
    if let Some(last) = rows.last() {
        advance(db,job,doc! {"cursor":last.get_str("_id").map_err(|_|AppError::Internal("Preparation cursor is invalid".into()))?,"shared_dependency":shared}).await?;
        return Ok(true);
    }
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::majority())
        .await?;
    let result:AppResult<()> = async {
        let revision=job.get("source_revision").cloned().unwrap_or(bson::Bson::Int64(0));
        let observed_inventory=job.get_i64("inventory_revision").map_err(|_|AppError::Internal("Missing credential inventory revision".into()))?;
        let current_inventory=super::super::erasure_admission::credential_inventory_in_session(db,&mut tx,false).await?;
        if current_inventory!=observed_inventory {
            db.collection::<Document>(PREPARATIONS).update_one(progress_filter(job)?,doc! {"$set":{"status":"stale"},"$unset":{"lease_token":"","lease_expires_at":""}}).session(&mut tx).await?;
            return Ok(());
        }
        let changed=db.collection::<Document>(USERS).update_one(doc! {"_id":user,"is_active":true,
            "$expr":{"$eq":[{"$ifNull":["$credential_dependency_revision",0_i64]},{"$literal":&revision}]}},
            doc! {"$inc":{"credential_preparation_fence":1_i64}}).session(&mut tx).await?;
        if changed.matched_count!=1 {
            db.collection::<Document>(PREPARATIONS).update_one(progress_filter(job)?,doc! {"$set":{"status":"stale"},"$unset":{"lease_token":"","lease_expires_at":""}}).session(&mut tx).await?;
        } else {
            let changed=db.collection::<Document>(PREPARATIONS).update_one(progress_filter(job)?,
                vec![doc! {"$set":{"status":"ready","shared_dependency":shared,"lease_token":bson::Bson::Null,"lease_expires_at":bson::Bson::Null,"updated_at":"$$NOW"}}]).session(&mut tx).await?;
            if changed.matched_count!=1 {return Err(AppError::Conflict("Preparation lease changed".into()));}
        }
        Ok(())
    }.await;
    super::super::org_membership_authority::finish(tx, result).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary(bytes: Vec<u8>) -> bson::Bson {
        bson::Bson::Binary(bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes,
        })
    }

    async fn replace(db: &Database, owner: &str, keys: &EncryptionKeys) -> AppResult<()> {
        let encrypted = keys.encrypt(b"successor-token").await?;
        let mut tx = db.client().start_session().await?;
        tx.start_transaction()
            .write_concern(WriteConcern::majority())
            .await?;
        super::super::super::erasure_admission::credential_change_in_session(db, owner, &mut tx)
            .await?;
        db.collection::<Document>(COLLECTIONS[1]).update_one(doc! {"_id":"token-0000"},doc! {"$set":{"access_token_encrypted":binary(encrypted)},"$unset":{"revocation_provenance":""}}).session(&mut tx).await?;
        super::super::super::transaction_service::commit(&mut tx).await
    }
    async fn finish_preparation(db: &Database, keys: &EncryptionKeys, user: &str) {
        for _ in 0..48 {
            if ready_or_enqueue(db, user).await.unwrap() {
                return;
            }
            assert!(process_one(db, keys).await.unwrap());
        }
        panic!("bounded preparation did not finish");
    }
    #[derive(Debug)]
    struct HeldImport {
        inner: crate::crypto::local_key_provider::LocalKeyProvider,
        first: std::sync::atomic::AtomicBool,
        entered: tokio::sync::Notify,
        release: tokio::sync::Notify,
    }
    #[async_trait::async_trait]
    impl crate::crypto::key_provider::KeyProvider for HeldImport {
        async fn wrap_dek(&self, key: &[u8]) -> AppResult<crate::crypto::key_provider::WrappedKey> {
            if !self.first.swap(true, std::sync::atomic::Ordering::SeqCst) {
                self.entered.notify_one();
                self.release.notified().await;
            }
            self.inner.wrap_dek(key).await
        }
        async fn unwrap_dek(
            &self,
            key: &crate::crypto::key_provider::WrappedKey,
        ) -> AppResult<zeroize::Zeroizing<Vec<u8>>> {
            self.inner.unwrap_dek(key).await
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
    async fn import_material(
        db: &Database,
        keys: &EncryptionKeys,
        owner: &str,
        provider: Option<&str>,
        credential_type: &str,
    ) -> crate::models::user_api_key::UserApiKey {
        crate::services::user_api_key_service::create_api_key(
            db,
            keys,
            owner,
            crate::services::user_api_key_service::CreateApiKeyParams {
                label: "Known opaque copy",
                credential_type,
                credential: "same-opaque-upstream-token",
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
                provider_config_id: provider,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn indexed_preparation_progresses_under_continuous_unrelated_credential_writes() {
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        import_material(&db, &keys, &user, Some("personal-provider"), "oauth2").await;
        crate::services::token_copy_admission::begin_discovery(&db, true)
            .await
            .unwrap();
        for _ in 0..16 {
            if !crate::services::token_copy_admission::discover_one(&db, &keys)
                .await
                .unwrap()
            {
                break;
            }
        }
        assert_eq!(
            crate::services::token_copy_admission::discovery_status(&db)
                .await
                .unwrap()
                .unwrap()
                .status,
            "complete"
        );
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        let initial = db
            .collection::<Document>(PREPARATIONS)
            .find_one(doc! {"_id":&user})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(initial.get_i64("inventory_revision"), Ok(-1));
        let global_before = db
            .collection::<Document>(crate::services::erasure_admission::CREDENTIAL_INVENTORY)
            .find_one(doc! {"_id":"global-v1"})
            .await
            .unwrap();
        let mut ready = false;
        for iteration in 0..32 {
            crate::services::user_api_key_service::create_api_key(
                &db,
                &keys,
                &uuid::Uuid::new_v4().to_string(),
                crate::services::user_api_key_service::CreateApiKeyParams {
                    label: "Unrelated live writer",
                    credential_type: "bearer",
                    credential: &format!("unrelated-token-{iteration}"),
                    access_token: None,
                    refresh_token: None,
                    token_scopes: None,
                    expires_at: None,
                    provider_config_id: None,
                    connection_id: None,
                    oauth_client_id: None,
                    oauth_client_secret: None,
                    status: "active",
                    source: None,
                    source_id: None,
                },
            )
            .await
            .unwrap();
            assert!(process_one(&db, &keys).await.unwrap());
            if ready_or_enqueue(&db, &user).await.unwrap() {
                ready = true;
                break;
            }
        }
        assert!(ready, "unrelated writers starved finite preparation");
        let current = db
            .collection::<Document>(PREPARATIONS)
            .find_one(doc! {"_id":&user})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(current.get("generation"), initial.get("generation"));
        assert!(!current.get_bool("shared_dependency").unwrap());
        assert_eq!(
            db.collection::<Document>(crate::services::erasure_admission::CREDENTIAL_INVENTORY)
                .find_one(doc! {"_id":"global-v1"})
                .await
                .unwrap(),
            global_before
        );
        // Readiness is an owner observation, not a promise that no other owner
        // can import this token later. Actual dispatch still checks holders.
        let copied = import_material(
            &db,
            &keys,
            &uuid::Uuid::new_v4().to_string(),
            None,
            "bearer",
        )
        .await;
        assert!(ready_or_enqueue(&db, &user).await.unwrap());
        assert!(
            crate::services::token_copy_admission::close_for_dispatch(
                &db,
                "effect",
                &"a".repeat(64),
                &["same-opaque-upstream-token"]
            )
            .await
            .is_err()
        );
        assert!(
            db.collection::<Document>(crate::models::user_api_key::COLLECTION_NAME)
                .find_one(doc! {"_id":copied.id})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn cross_provider_and_unattributed_manual_copies_are_discovered_without_a_source_link() {
        for provider in [Some("different-provider"), None] {
            let (db, user) = super::super::tests::fixture().await;
            let keys = crate::test_utils::test_encryption_keys();
            let org = uuid::Uuid::new_v4().to_string();
            db.collection::<crate::models::user::User>(USERS)
                .insert_one(crate::test_utils::test_user(
                    &org,
                    crate::models::user::UserType::Org,
                ))
                .await
                .unwrap();
            import_material(&db, &keys, &user, Some("personal-provider"), "oauth2").await;
            let copy = import_material(&db, &keys, &org, provider, "oauth2").await;
            assert!(copy.source_id.is_none());
            finish_preparation(&db, &keys, &user).await;
            assert!(matches!(
                super::super::accept(&db, &user, &user, None).await,
                Err(AppError::Conflict(_))
            ));
            assert!(
                db.collection::<crate::models::user::User>(USERS)
                    .find_one(doc! {"_id":&user,"is_active":true})
                    .await
                    .unwrap()
                    .is_some()
            );
            assert_eq!(
                db.collection::<Document>(super::super::provider::EFFECTS)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn held_manual_bearer_import_after_ready_invalidates_the_prepared_person() {
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let org = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(USERS)
            .insert_one(crate::test_utils::test_user(
                &org,
                crate::models::user::UserType::Org,
            ))
            .await
            .unwrap();
        import_material(&db, &keys, &user, Some("personal-provider"), "oauth2").await;
        let held = std::sync::Arc::new(HeldImport {
            inner: crate::crypto::local_key_provider::LocalKeyProvider::new([0x11; 32], None),
            first: Default::default(),
            entered: Default::default(),
            release: Default::default(),
        });
        let held_keys = EncryptionKeys::with_provider(held.clone());
        let import = import_material(&db, &held_keys, &org, None, "bearer");
        let prepare = async {
            held.entered.notified().await;
            finish_preparation(&db, &keys, &user).await;
            let observed = db
                .collection::<Document>(PREPARATIONS)
                .find_one(doc! {"_id":&user})
                .await
                .unwrap()
                .unwrap();
            assert!(!observed.get_bool("shared_dependency").unwrap());
            held.release.notify_one();
            observed
        };
        let (copy, observed) = tokio::join!(import, prepare);
        assert!(copy.source_id.is_none());
        let unchanged_person = db
            .collection::<Document>(USERS)
            .find_one(doc! {"_id":&user})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            unchanged_person.get("credential_dependency_revision"),
            observed.get("source_revision")
        );
        // Destination-only personal revisions would incorrectly accept here.
        assert!(matches!(
            super::super::accept(&db, &user, &user, None).await,
            Err(AppError::Conflict(_))
        ));
        finish_preparation(&db, &keys, &user).await;
        assert!(matches!(
            super::super::accept(&db, &user, &user, None).await,
            Err(AppError::Conflict(_))
        ));
        let preparation = db
            .collection::<Document>(PREPARATIONS)
            .find_one(doc! {"_id":&user})
            .await
            .unwrap()
            .unwrap();
        assert!(preparation.get_bool("shared_dependency").unwrap());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn providerless_oauth_writer_stays_usable_then_retains_an_external_erasure_obligation() {
        use crate::services::user_api_key_service::{self, CreateApiKeyParams};
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let key = user_api_key_service::create_api_key(
            &db,
            &keys,
            &user,
            CreateApiKeyParams {
                label: "Custom OAuth",
                credential_type: "oauth2",
                credential: "imported-access",
                access_token: None,
                refresh_token: Some("imported-refresh"),
                token_scopes: None,
                expires_at: None,
                provider_config_id: None,
                connection_id: None,
                oauth_client_id: None,
                oauth_client_secret: None,
                status: "active",
                source: None,
                source_id: None,
            },
        )
        .await
        .unwrap();
        assert!(key.grant_lineage.as_ref().unwrap().provider_unattributed);
        assert!(key.provider_config_id.is_none());
        assert_eq!(
            crate::services::credential_record_service::CredentialRecord::key(&key)
                .text(
                    &keys,
                    "access_token_encrypted",
                    key.access_token_encrypted.as_deref().unwrap()
                )
                .await
                .unwrap()
                .as_str(),
            "imported-access"
        );
        user_api_key_service::update_api_key(
            &db,
            &keys,
            &user,
            &key.id,
            None,
            Some("replaced-custom-access"),
        )
        .await
        .unwrap();
        let key = db
            .collection::<crate::models::user_api_key::UserApiKey>(
                crate::models::user_api_key::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&key.id,"user_id":&user})
            .await
            .unwrap()
            .unwrap();
        assert!(key.grant_lineage.as_ref().unwrap().provider_unattributed);
        finish_preparation(&db, &keys, &user).await;
        let job = super::super::accept(&db, &user, &user, None).await.unwrap();
        let mut actionable = false;
        for _ in 0..6 {
            if matches!(
                super::super::provider::pass(&db, &keys, &job)
                    .await
                    .unwrap(),
                super::super::provider::Pass::Action(_)
            ) {
                actionable = true;
                break;
            }
        }
        assert!(actionable);
        let effect = db
            .collection::<Document>(super::super::provider::EFFECTS)
            .find_one(doc! {"user_id":&user,"source_id":&key.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(effect.get_str("status"), Ok("operator_action"));
        assert_eq!(
            effect.get_str("reason"),
            Ok("oauth_provider_unknown_requires_external_reconciliation")
        );
        assert!(effect.contains_key("capability_encrypted"));
        let source = db
            .collection::<Document>(COLLECTIONS[0])
            .find_one(doc! {"_id":&key.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!source.contains_key("access_token_encrypted"));
        assert!(!source.contains_key("refresh_token_encrypted"));
        // Unknown attribution cannot be attested as an unsupported provider.
        let request = super::super::provider::EffectReconciliationRequest {
            effect_id: effect.get_str("_id").unwrap().to_owned(),
            action: "record_remote_revocation_unavailable".into(),
            evidence_reference: "external-ticket".into(),
            expected_retry_generation: 0,
            idempotency_key: uuid::Uuid::new_v4().to_string(),
        };
        assert!(matches!(
            super::super::provider::reconcile_effect(&db, &keys, &user, &user, &request).await,
            Err(AppError::Conflict(_))
        ));
        let confirmed = super::super::provider::EffectReconciliationRequest {
            action: "record_external_revocation_confirmed".into(),
            ..request
        };
        super::super::provider::reconcile_effect(&db, &keys, &user, &user, &confirmed)
            .await
            .unwrap();
        let completed = db
            .collection::<Document>(super::super::provider::EFFECTS)
            .find_one(doc! {"_id":&confirmed.effect_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(completed.get_str("status"), Ok("complete"));
        assert_eq!(completed.get_bool("remote_revocation_confirmed"), Ok(true));
        assert!(!completed.contains_key("capability_encrypted"));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn actual_byo_replacement_rewrap_and_deletion_invalidate_prepared_dependency_revision() {
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let mut legacy = crate::test_utils::test_encryption_keys();
        legacy.set_contextual_record_writes(false);
        db.collection::<Document>(COLLECTIONS[1]).insert_one(doc! {"_id":"token","user_id":&user,"provider_config_id":"provider",
            "credential_user_id":&user,"status":"active","access_token_encrypted":binary(keys.encrypt(b"existing-token").await.unwrap())}).await.unwrap();
        let credentials = crate::services::user_credentials_service::upsert_user_credentials(
            &db,
            &legacy,
            &user,
            "provider",
            "original-client",
            Some("original-secret"),
            None,
        )
        .await
        .unwrap();
        finish_preparation(&db, &keys, &user).await;
        assert!(ready_or_enqueue(&db, &user).await.unwrap());
        crate::services::user_credentials_service::upsert_user_credentials_with_expected_state_version(&db, &legacy, &user, "provider", "replacement-client", Some("replacement-secret"), None, Some(credentials.state_version)).await.unwrap();
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        finish_preparation(&db, &keys, &user).await;
        // Storage-only rewrap must invalidate the same prepared dependency
        // authority as live BYO replacement, even with unchanged plaintext.
        let source = db
            .collection::<Document>(crate::models::user_provider_credentials::COLLECTION_NAME)
            .find_one(doc! {"_id":&credentials.id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            crate::services::credential_record_service::migrate_snapshot(
                &db,
                &keys,
                crate::models::user_provider_credentials::COLLECTION_NAME,
                &source,
            )
            .await
            .unwrap()
        );
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        // The actual token is migrated through its live field adapter too;
        // strict preparation must use that adapter instead of raw decrypt.
        let token = db
            .collection::<Document>(COLLECTIONS[1])
            .find_one(doc! {"_id":"token"})
            .await
            .unwrap()
            .unwrap();
        assert!(
            crate::services::credential_record_service::migrate_snapshot(
                &db,
                &keys,
                crate::models::user_provider_token::COLLECTION_NAME,
                &token,
            )
            .await
            .unwrap()
        );
        let mut strict = crate::test_utils::test_encryption_keys();
        strict.set_allow_legacy_record_reads(false);
        finish_preparation(&db, &strict, &user).await;
        assert!(ready_or_enqueue(&db, &user).await.unwrap());
        assert_eq!(
            db.collection::<Document>(COLLECTIONS[1])
                .find_one(doc! {"_id":"token"})
                .await
                .unwrap()
                .unwrap()
                .get_document("revocation_provenance")
                .unwrap()
                .get_str("state"),
            Ok("ready")
        );
        crate::services::user_credentials_service::delete_user_credentials(&db, &user, "provider")
            .await
            .unwrap();
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        assert!(
            db.collection::<Document>(USERS)
                .find_one(doc! {"_id":&user,"is_active":true})
                .await
                .unwrap()
                .is_some()
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn preparation_pages_more_than_256_rows_and_refresh_invalidates_exact_source() {
        let (db, user) = super::super::tests::fixture().await;
        let keys = crate::test_utils::test_encryption_keys();
        let tokens = db.collection::<Document>(COLLECTIONS[1]);
        let mut rows = Vec::new();
        for index in 0..270 {
            rows.push(doc! {"_id":format!("token-{index:04}"),"user_id":&user,"provider_config_id":"provider",
                "status":"active","access_token_encrypted":binary(keys.encrypt(format!("token-{index}").as_bytes()).await.unwrap())});
        }
        tokens.insert_many(rows).await.unwrap();
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        for _ in 0..40 {
            if ready_or_enqueue(&db, &user).await.unwrap() {
                break;
            }
            assert!(process_one(&db, &keys).await.unwrap());
        }
        assert!(ready_or_enqueue(&db, &user).await.unwrap());
        assert_eq!(
            tokens
                .count_documents(doc! {"revocation_provenance.state":"ready"})
                .await
                .unwrap(),
            270
        );
        let original = tokens
            .find_one(doc! {"_id":"token-0000"})
            .await
            .unwrap()
            .unwrap();
        // An old snapshot cannot stamp readiness after a refresh wins its CAS.
        replace(&db, &user, &keys).await.unwrap();
        prepare_row(&db, &keys, COLLECTIONS[1], &original)
            .await
            .unwrap();
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        finish_preparation(&db, &keys, &user).await;
        assert!(ready_or_enqueue(&db, &user).await.unwrap());
        let refreshed = tokens
            .find_one(doc! {"_id":"token-0000"})
            .await
            .unwrap()
            .unwrap();
        assert_ne!(
            original
                .get_document("revocation_provenance")
                .unwrap()
                .get_array("token_fingerprints")
                .unwrap(),
            refreshed
                .get_document("revocation_provenance")
                .unwrap()
                .get_array("token_fingerprints")
                .unwrap()
        );
        // Replacing storage with the same authenticated token preserves its
        // fingerprint; readiness requires the full scan and verification phases.
        let fingerprint = refreshed
            .get_document("revocation_provenance")
            .unwrap()
            .get_array("token_fingerprints")
            .unwrap()
            .clone();
        replace(&db, &user, &keys).await.unwrap();
        assert!(!ready_or_enqueue(&db, &user).await.unwrap());
        finish_preparation(&db, &keys, &user).await;
        assert_eq!(
            tokens
                .find_one(doc! {"_id":"token-0000"})
                .await
                .unwrap()
                .unwrap()
                .get_document("revocation_provenance")
                .unwrap()
                .get_array("token_fingerprints")
                .unwrap(),
            &fingerprint
        );
        assert!(
            db.collection::<User>(USERS)
                .find_one(doc! {"_id":&user,"is_active":true})
                .await
                .unwrap()
                .is_some()
        );
        assert!(super::super::status(&db, &user).await.unwrap().is_none());
        db.drop().await.unwrap();
    }
}
