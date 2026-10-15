use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Bson, Document, doc},
    results::{DeleteResult, InsertOneResult, UpdateResult},
};

use crate::{
    errors::{AppError, AppResult},
    models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS},
};

const AUTHORITY_FIELDS: [&str; 6] = [
    "rotation_predecessor_id",
    "state_version",
    "updated_at",
    "purpose",
    "machine_external_fence",
    "machine_external_grants_present",
];

// These are the only mutations that cannot change the source authorization.
// An empty mutation is deliberately security-sensitive: binding services use
// it to fence a credential-binding change on its unchanged parent key.
fn changes_source_authority(update: &Document) -> bool {
    const BENIGN: [&str; 4] = ["name", "description", "platform", "last_used_at"];
    update.is_empty()
        || update.iter().any(|(operator, value)| {
            let Bson::Document(fields) = value else {
                return true;
            };
            fields.keys().any(|field| !BENIGN.contains(&field.as_str()))
                || (operator == "$rename"
                    && fields
                        .values()
                        .any(|value| !value.as_str().is_some_and(|field| BENIGN.contains(&field))))
        })
}

fn authority_field(field: &str) -> bool {
    AUTHORITY_FIELDS.contains(&field.split('.').next().unwrap_or(field))
}

#[derive(Debug)]
struct TransactionAppError(std::sync::Mutex<Option<AppError>>);

#[cfg(test)]
#[derive(Clone)]
pub struct TransactionCollisionHook {
    barrier: std::sync::Arc<tokio::sync::Barrier>,
    attempts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    waited: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
impl TransactionCollisionHook {
    pub fn new(barrier: std::sync::Arc<tokio::sync::Barrier>) -> Self {
        Self {
            barrier,
            attempts: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            waited: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        }
    }

    pub fn begin_attempt(&self) {
        self.attempts
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    }

    pub async fn after_reads(&self) {
        if !self.waited.swap(true, std::sync::atomic::Ordering::SeqCst) {
            self.barrier.wait().await;
        }
    }

    pub fn attempts(&self) -> usize {
        self.attempts.load(std::sync::atomic::Ordering::SeqCst)
    }
}

pub fn abort_transaction(error: AppError) -> mongodb::error::Error {
    mongodb::error::Error::custom(TransactionAppError(std::sync::Mutex::new(Some(error))))
}

pub fn map_transaction_error(error: mongodb::error::Error) -> AppError {
    let app_error = error
        .get_custom::<TransactionAppError>()
        .and_then(|custom| custom.0.lock().ok()?.take());
    app_error.unwrap_or(AppError::DatabaseError(error))
}

pub fn transaction_result<T>(result: AppResult<T>) -> mongodb::error::Result<T> {
    result.map_err(|error| match error {
        AppError::DatabaseError(error) => error,
        error => abort_transaction(error),
    })
}

fn operator_document<'a>(update: &'a mut Document, operator: &str) -> AppResult<&'a mut Document> {
    if !update.contains_key(operator) {
        update.insert(operator, Document::new());
    }
    update
        .get_document_mut(operator)
        .map_err(|_| AppError::Internal(format!("API key {operator} must be a document")))
}

fn reject_authority_override(update: &Document) -> AppResult<()> {
    for (operator, value) in update {
        if !operator.starts_with('$') {
            return Err(AppError::Internal(
                "API key mutations must use update operators".to_string(),
            ));
        }
        let Bson::Document(fields) = value else {
            return Err(AppError::Internal(format!(
                "API key {operator} must be a document"
            )));
        };
        if fields.keys().any(|field| authority_field(field)) {
            return Err(AppError::Internal(
                "API key authority fields are owned by the mutation service".to_string(),
            ));
        }
        if operator == "$rename"
            && fields
                .values()
                .any(|value| value.as_str().is_some_and(authority_field))
        {
            return Err(AppError::Internal(
                "API key authority fields cannot be rename targets".to_string(),
            ));
        }
    }
    Ok(())
}

/// Add the authoritative version/timestamp transition to an ApiKey update.
///
/// Missing legacy `state_version` fields are initialized to one by MongoDB's
/// `$inc` semantics. Callers cannot supply either authority field themselves.
pub fn authoritative_update_at(
    mut update: Document,
    updated_at: DateTime<Utc>,
) -> AppResult<Document> {
    reject_authority_override(&update)?;
    let security_change = changes_source_authority(&update);
    operator_document(&mut update, "$set")?.insert(
        "updated_at",
        Bson::DateTime(bson::DateTime::from_chrono(updated_at)),
    );
    operator_document(&mut update, "$inc")?.insert("state_version", 1_i64);
    if security_change {
        operator_document(&mut update, "$inc")?.insert("machine_external_fence", 1_i64);
    }
    Ok(update)
}

pub fn authoritative_update(update: Document) -> AppResult<Document> {
    authoritative_update_at(update, Utc::now())
}

/// Serialize issuance against the exact parent state reviewed for child login.
/// This consumes the consent snapshot without changing the parent's permission
/// contract. Binding changes must continue using the security-sensitive update
/// path, including its empty-update fence and transactional grant invalidation.
pub async fn fence_child_consent_in_session(
    db: &Database,
    parent: &ApiKey,
    session: &mut ClientSession,
) -> AppResult<()> {
    if parent.state_version < 0 {
        return Err(AppError::Conflict("API key approval source changed".into()));
    }
    let mut filter = doc! {
        "_id": &parent.id,
        "user_id": &parent.user_id,
        "is_active": true,
        "$and": [{"$or": [
            {"expires_at": Bson::Null},
            {"expires_at": {"$gt": bson::DateTime::now()}},
        ]}],
    };
    if parent.state_version == 0 {
        filter.insert(
            "$or",
            vec![
                doc! {"state_version": 0_i64},
                doc! {"state_version": {"$exists": false}},
            ],
        );
    } else {
        filter.insert("state_version", parent.state_version);
    }
    let result = db
        .collection::<ApiKey>(API_KEYS)
        .update_one(
            filter,
            doc! {
                "$set": {"updated_at": bson::DateTime::now()},
                "$inc": {"state_version": 1_i64},
            },
        )
        .session(session)
        .await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict("API key approval source changed".into()));
    }
    Ok(())
}

pub async fn update_one(
    db: &Database,
    filter: Document,
    update: Document,
    session: Option<&mut ClientSession>,
) -> AppResult<UpdateResult> {
    reject_authority_override(&update)?;
    if changes_source_authority(&update) {
        return security_update(db, filter, update, false, session).await;
    }
    let collection = db.collection::<ApiKey>(API_KEYS);
    let action = collection.update_one(filter, authoritative_update(update)?);
    Ok(match session {
        Some(session) => action.session(session).await?,
        None => action.await?,
    })
}

pub async fn update_many(
    db: &Database,
    filter: Document,
    update: Document,
    session: Option<&mut ClientSession>,
) -> AppResult<UpdateResult> {
    reject_authority_override(&update)?;
    if changes_source_authority(&update) {
        return security_update(db, filter, update, true, session).await;
    }
    let collection = db.collection::<ApiKey>(API_KEYS);
    let action = collection.update_many(filter, authoritative_update(update)?);
    Ok(match session {
        Some(session) => action.session(session).await?,
        None => action.await?,
    })
}

async fn security_update(
    db: &Database,
    filter: Document,
    update: Document,
    many: bool,
    session: Option<&mut ClientSession>,
) -> AppResult<UpdateResult> {
    if let Some(session) = session {
        return security_update_in_session(db, filter, update, many, session).await;
    }
    let db = db.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            transaction_result(
                security_update_in_session(&db, filter.clone(), update.clone(), many, session)
                    .await,
            )
        })
        .await
        .map_err(map_transaction_error)
}

/// Source mutation, monotonic source fence and grant revocation commit together.
/// Admission/renewal writes this same source document in its own transaction;
/// `state_version` is intentionally not a persistent Machine grant predicate.
async fn security_update_in_session(
    db: &Database,
    filter: Document,
    update: Document,
    many: bool,
    session: &mut ClientSession,
) -> AppResult<UpdateResult> {
    let collection = db.collection::<Document>(API_KEYS);
    let mut query = collection.find(filter.clone()).projection(doc! {
        "_id": 1, "user_id": 1, "machine_external_fence": 1,
        "machine_external_grants_present": 1,
    });
    if !many {
        query = query.limit(1);
    }
    let mut cursor = query.session(&mut *session).await?;
    let rows: Vec<Document> = cursor.stream(&mut *session).try_collect().await?;
    let update = authoritative_update(update)?;
    let result = if many {
        collection
            .update_many(filter, update)
            .session(&mut *session)
            .await?
    } else {
        // Bind the update to the source row that was read, even if a caller's
        // selector is broader than an ID. The impossible filter preserves the
        // ordinary zero-match UpdateResult without manufacturing a source.
        let id_filter = rows.first().map_or_else(
            || doc! {"_id": {"$exists": false}},
            |row| doc! {"_id": row.get("_id").cloned().unwrap_or(Bson::Null)},
        );
        collection
            .update_one(doc! {"$and": [filter, id_filter]}, update)
            .session(&mut *session)
            .await?
    };
    if result.matched_count != rows.len() as u64 {
        return Err(AppError::Conflict(
            "API key changed during source fencing".into(),
        ));
    }
    for row in rows {
        super::machine_external_source_service::api_key_security_changed_in_session(
            db,
            &row,
            &mut *session,
        )
        .await?;
    }
    Ok(result)
}

/// Physical removal for abandoned, never-delivered key provisioning. Normal
/// user deletion remains a soft revoke in `key_service`. Even an abandoned row
/// may have been visible to its owner, so source fencing and grant revocation
/// must precede its removal in the same transaction.
pub async fn delete_one(
    db: &Database,
    user_id: &str,
    key_id: &str,
    session: Option<&mut ClientSession>,
) -> AppResult<DeleteResult> {
    if let Some(session) = session {
        return delete_in_session(db, user_id, key_id, session).await;
    }
    let db = db.clone();
    let user_id = user_id.to_owned();
    let key_id = key_id.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            transaction_result(delete_in_session(&db, &user_id, &key_id, session).await)
        })
        .await
        .map_err(map_transaction_error)
}

async fn delete_in_session(
    db: &Database,
    user_id: &str,
    key_id: &str,
    session: &mut ClientSession,
) -> AppResult<DeleteResult> {
    let filter = doc! {"_id": key_id, "user_id": user_id};
    security_update_in_session(db, filter.clone(), doc! {}, false, &mut *session).await?;
    Ok(db
        .collection::<ApiKey>(API_KEYS)
        .delete_one(filter)
        .session(session)
        .await?)
}

/// Consume a human Machine approval's source CAS exactly once. The caller must
/// validate live key eligibility and serialize current grant handling in this
/// same transaction before inserting the new grant. This is not a security
/// mutation of an existing grant: it records the reviewed source version, sets
/// the permanent presence hint and prevents a delayed approval replay after a
/// later grant revoke. Runtime admission never compares `state_version`.
pub async fn approve_machine_grant_source_in_session(
    db: &Database,
    user_id: &str,
    key_id: &str,
    expected_state_version: i64,
    session: &mut ClientSession,
) -> AppResult<()> {
    if expected_state_version < 0 {
        return Err(AppError::Conflict("API key approval source changed".into()));
    }
    let mut filter = doc! {
        "_id": key_id, "user_id": user_id, "is_active": true,
        "$and": [{"$or": [
            {"expires_at": Bson::Null},
            {"expires_at": {"$gt": bson::DateTime::now()}},
        ]}],
    };
    if expected_state_version == 0 {
        filter.insert(
            "$or",
            vec![
                doc! {"state_version": 0_i64},
                doc! {"state_version": {"$exists": false}},
            ],
        );
    } else {
        filter.insert("state_version", expected_state_version);
    }
    let updated = db
        .collection::<ApiKey>(API_KEYS)
        .update_one(
            filter,
            doc! {
                "$set": {
                    "machine_external_grants_present": true,
                    "updated_at": bson::DateTime::now(),
                },
                "$inc": {"state_version": 1_i64, "machine_external_fence": 1_i64},
            },
        )
        .session(session)
        .await?;
    if updated.matched_count != 1 {
        return Err(AppError::Conflict("API key approval source changed".into()));
    }
    Ok(())
}

pub async fn insert_one(
    db: &Database,
    key: &ApiKey,
    session: Option<&mut ClientSession>,
) -> AppResult<InsertOneResult> {
    if key.state_version <= 0 || key.updated_at.is_none() {
        return Err(AppError::Internal(
            "new API keys require positive authority evidence".to_string(),
        ));
    }
    let collection = db.collection::<ApiKey>(API_KEYS);
    let action = collection.insert_one(key);
    Ok(match session {
        Some(session) => action.session(session).await?,
        None => action.await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::doc;

    #[test]
    fn authoritative_update_preserves_business_operators_and_adds_authority() {
        let now = Utc::now();
        let update = authoritative_update_at(
            doc! {
                "$set": { "is_active": false },
                "$pull": { "allowed_service_ids": "svc-1" },
            },
            now,
        )
        .unwrap();

        assert_eq!(update["$set"]["is_active"], Bson::Boolean(false));
        assert_eq!(
            update["$pull"]["allowed_service_ids"],
            Bson::String("svc-1".to_string())
        );
        assert_eq!(update["$inc"]["state_version"], Bson::Int64(1));
        assert_eq!(
            update["$set"]["updated_at"],
            Bson::DateTime(bson::DateTime::from_chrono(now))
        );
    }

    #[test]
    fn authoritative_update_rejects_authority_overrides() {
        for update in [
            doc! { "$set": { "updated_at": bson::DateTime::now() } },
            doc! { "$set": { "purpose": "general" } },
            doc! { "$set": { "purpose.kind": "general" } },
            doc! { "$unset": { "purpose": "" } },
            doc! { "$rename": { "legacy_purpose": "purpose" } },
            doc! { "$rename": { "legacy_purpose": "purpose.kind" } },
            doc! { "$inc": { "state_version": 10 } },
            doc! { "$unset": { "rotation_predecessor_id": "" } },
            doc! { "$rename": { "legacy_version": "state_version" } },
            doc! { "$rename": { "legacy_parent": "rotation_predecessor_id" } },
        ] {
            assert!(authoritative_update(update).is_err());
        }
    }

    #[test]
    fn authoritative_update_rejects_replacement_documents() {
        assert!(authoritative_update(doc! { "is_active": false }).is_err());
    }

    #[test]
    fn machine_fence_changes_for_security_but_not_usage_or_labels() {
        for update in [
            doc! {"$set": {"last_used_at": bson::DateTime::now()}},
            doc! {"$set": {"name": "new name", "description": "new label", "platform": "codex"}},
        ] {
            let update = authoritative_update(update).unwrap();
            assert_eq!(update["$inc"]["state_version"], Bson::Int64(1));
            assert!(
                !update
                    .get_document("$inc")
                    .unwrap()
                    .contains_key("machine_external_fence")
            );
        }
        for update in [
            doc! {"$set": {"is_active": false}},
            doc! {"$set": {"scopes": "read"}},
            doc! {"$pull": {"allowed_node_ids": "node"}},
            doc! {"$set": {"expires_at": bson::DateTime::now()}},
            doc! {"$set": {"rate_limit_per_second": 1}},
            doc! {"$rename": {"description": "scopes"}},
            // Credential binding mutations deliberately touch an otherwise
            // unchanged parent row through this existing service.
            doc! {},
        ] {
            let update = authoritative_update(update).unwrap();
            assert_eq!(update["$inc"]["machine_external_fence"], Bson::Int64(1));
        }
    }

    #[test]
    fn machine_fence_and_presence_hint_are_owned_by_source_authority() {
        for update in [
            doc! {"$set": {"machine_external_fence": 0}},
            doc! {"$unset": {"machine_external_grants_present": ""}},
            doc! {"$rename": {"description": "machine_external_fence"}},
            doc! {"$set": {"state_version.nested": 1}},
        ] {
            assert!(authoritative_update(update).is_err());
        }
    }
}
