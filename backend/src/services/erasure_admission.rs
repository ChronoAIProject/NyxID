//! Persistent personal-owner write fencing. This is a deletion barrier, not an
//! authentication decision: each caller retains its existing ACL/owner checks.
//! The negative fence outlives source removal, so a held writer cannot recreate
//! owned data after the account document has gone away.
use crate::errors::{AppError, AppResult};
use bson::{Document, doc};
use mongodb::{ClientSession, Database};

#[derive(Debug)]
pub(crate) struct OwnerFenceCreationRace;

pub(crate) const COLLECTION: &str = "personal_owner_write_fences";

fn accepted_filter(owner: &str) -> Document {
    doc! {"_id":owner,"erasure_accepted":true}
}

/// Observe accepted erasure without taking a write fence or changing revisions.
/// Like write admission, check both the target and its canonical person owner.
/// This is a read-time eligibility check, not a replacement for transactional
/// fencing when a caller writes owned state.
pub(crate) async fn is_erasure_accepted(db: &Database, owner: &str) -> AppResult<bool> {
    let fences = db.collection::<Document>(COLLECTION);
    if fences.find_one(accepted_filter(owner)).await?.is_some() {
        return Ok(true);
    }
    let canonical = credential_owner(db, owner).await?;
    Ok(canonical != owner
        && fences
            .find_one(accepted_filter(&canonical))
            .await?
            .is_some())
}

pub(crate) async fn fence_in_session(
    db: &Database,
    owner: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    fence_one_in_session(db, owner, tx).await?;
    if let Some(account) = db
        .collection::<crate::models::service_account::ServiceAccount>(
            crate::models::service_account::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":owner})
        .session(&mut *tx)
        .await?
    {
        let canonical = account
            .owner_user_id
            .as_deref()
            .unwrap_or(&account.created_by);
        if canonical != owner {
            fence_one_in_session(db, canonical, tx).await?;
            db.collection::<Document>(crate::models::service_account::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":owner},
                    doc! {"$inc":{"owner_lifecycle_fence":1_i64}},
                )
                .session(tx)
                .await?;
        }
    }
    Ok(())
}

async fn fence_one_in_session(db: &Database, owner: &str, tx: &mut ClientSession) -> AppResult<()> {
    if db
        .collection::<Document>(COLLECTION)
        .find_one(accepted_filter(owner))
        .session(&mut *tx)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Account erasure has closed this owner's write admission".into(),
        ));
    }
    // Both first admission and accepted deletion write the same unique owner
    // record. Snapshot isolation alone would not fence a concurrent insertion.
    db.collection::<Document>(COLLECTION)
        .update_one(
            doc! {"_id":owner},
            doc! {"$inc":{"revision":1_i64},"$setOnInsert":{"erasure_accepted":false}},
        )
        .upsert(true)
        .session(&mut *tx)
        .await.map_err(|error| {
            let duplicate = matches!(error.kind.as_ref(),mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write)) if write.code==11000)
                || matches!(error.kind.as_ref(),mongodb::error::ErrorKind::Command(command) if command.code==11000);
            if duplicate { AppError::DatabaseError(mongodb::error::Error::custom(OwnerFenceCreationRace)) }
            else { AppError::DatabaseError(error) }
        })?;
    Ok(())
}

pub(crate) async fn close_in_session(
    db: &Database,
    owner: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(COLLECTION)
        .update_one(
            doc! {"_id":owner},
            vec![doc! {"$set":{"erasure_accepted":true,"accepted_at":"$$NOW",
            "revision":{"$add":[{"$ifNull":["$revision",0_i64]},1_i64]}}}],
        )
        .upsert(true)
        .session(tx)
        .await?;
    Ok(())
}

/// Source and destination dependency writers use the same owner fence and
/// revision as preparation/acceptance. Storage-only rewrap may invalidate
/// preparation but never changes credential epochs or OAuth grant authority.
pub(crate) async fn credential_change_in_session(
    db: &Database,
    owner: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    fence_in_session(db, owner, tx).await?;
    db.collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":owner},
            doc! {"$inc":{"credential_dependency_revision":1_i64}},
        )
        .session(&mut *tx)
        .await?;
    if let Some(account) = db
        .collection::<crate::models::service_account::ServiceAccount>(
            crate::models::service_account::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":owner})
        .session(&mut *tx)
        .await?
    {
        let canonical = account
            .owner_user_id
            .as_deref()
            .unwrap_or(&account.created_by);
        if canonical != owner {
            db.collection::<Document>(crate::models::user::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":canonical},
                    doc! {"$inc":{"credential_dependency_revision":1_i64}},
                )
                .session(tx)
                .await?;
        }
    }
    Ok(())
}

/// Capture canonical ownership before an upstream exchange can be held across
/// source cleanup. Service-account creator fallback is its documented legacy
/// owner contract, never attribution on unrelated platform/org resources.
pub(crate) async fn credential_owner(db: &Database, target: &str) -> AppResult<String> {
    let account = db
        .collection::<crate::models::service_account::ServiceAccount>(
            crate::models::service_account::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":target})
        .await?;
    Ok(account
        .map(|row| row.owner_user_id.unwrap_or(row.created_by))
        .unwrap_or_else(|| target.to_owned()))
}

pub(crate) async fn fence_credential_target_in_session(
    db: &Database,
    target: &str,
    expected_owner: &str,
    tx: &mut ClientSession,
) -> AppResult<()> {
    if target != expected_owner {
        let Some(account) = db
            .collection::<crate::models::service_account::ServiceAccount>(
                crate::models::service_account::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":target,"is_active":true})
            .session(&mut *tx)
            .await?
        else {
            return Err(AppError::Conflict(
                "Credential target is unavailable or changed".into(),
            ));
        };
        if account
            .owner_user_id
            .as_deref()
            .unwrap_or(&account.created_by)
            != expected_owner
        {
            return Err(AppError::Conflict("Credential target owner changed".into()));
        }
        db.collection::<Document>(crate::models::service_account::COLLECTION_NAME)
            .update_one(
                doc! {"_id":target,"is_active":true},
                doc! {"$inc":{"credential_admission_revision":1_i64}},
            )
            .session(&mut *tx)
            .await?;
    }
    credential_change_in_session(db, expected_owner, tx).await
}

/// Start a bounded metadata write under the persistent owner barrier. Callers
/// retain their existing ACL and exact-row predicates and finish this same tx.
pub(crate) async fn begin_write(db: &Database, owner: &str) -> AppResult<ClientSession> {
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    if let Err(error) = fence_in_session(db, owner, &mut tx).await {
        let _ = tx.abort_transaction().await;
        return Err(error);
    }
    Ok(tx)
}

pub(crate) async fn insert_owned<T: serde::Serialize + Sync>(
    db: &Database,
    collection: &str,
    owner: &str,
    row: &T,
) -> AppResult<mongodb::results::InsertOneResult> {
    let document = bson::to_document(row)
        .map_err(|_| AppError::Internal("Invalid owned metadata serialization".into()))?;
    let context = (
        db.clone(),
        collection.to_owned(),
        owner.to_owned(),
        document,
    );
    super::transaction_service::run_db_only(
        db,
        &context,
        |tx, (db, collection, owner, document)| {
            Box::pin(async move {
                fence_in_session(db, owner, tx).await?;
                Ok(db
                    .collection::<Document>(collection)
                    .insert_one(document.clone())
                    .session(tx)
                    .await?)
            })
        },
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn read_only_erasure_check_preserves_missing_open_and_accepted_fences() {
        let db = crate::test_utils::connect_transaction_test_database("erasure_read_only").await;
        let fences = db.collection::<Document>(COLLECTION);
        assert!(!is_erasure_accepted(&db, "person").await.unwrap());
        assert_eq!(fences.count_documents(doc! {}).await.unwrap(), 0);
        for accepted in [false, true] {
            let row = doc! {"_id":"person","erasure_accepted":accepted,"revision":7_i64};
            fences
                .replace_one(doc! {"_id":"person"}, &row)
                .upsert(true)
                .await
                .unwrap();
            assert_eq!(is_erasure_accepted(&db, "person").await.unwrap(), accepted);
            assert_eq!(
                fences
                    .find_one(doc! {"_id":"person"})
                    .await
                    .unwrap()
                    .unwrap(),
                row
            );
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn read_only_erasure_check_follows_canonical_and_legacy_service_account_owners() {
        let db =
            crate::test_utils::connect_transaction_test_database("erasure_read_canonical").await;
        let accounts = db.collection::<Document>(crate::models::service_account::COLLECTION_NAME);
        let fences = db.collection::<Document>(COLLECTION);
        let now = bson::DateTime::now();
        let mut account = doc! {
            "_id":"service-account","name":"Read fence fixture","client_id":"sa_fixture",
            "client_secret_hash":"fixture","secret_prefix":"fixture","allowed_scopes":"proxy",
            "is_active":true,"created_by":"creator","owner_user_id":"person",
            "created_at":now,"updated_at":now,
        };
        accounts.insert_one(&account).await.unwrap();
        // A distinct administrator/creator does not override the canonical owner.
        fences
            .insert_one(doc! {"_id":"creator","erasure_accepted":true})
            .await
            .unwrap();
        assert!(!is_erasure_accepted(&db, "service-account").await.unwrap());
        fences
            .insert_one(doc! {"_id":"person","erasure_accepted":true})
            .await
            .unwrap();
        assert!(is_erasure_accepted(&db, "service-account").await.unwrap());
        assert_eq!(
            accounts
                .find_one(doc! {"_id":"service-account"})
                .await
                .unwrap()
                .unwrap(),
            account
        );
        assert!(
            fences
                .find_one(doc! {"_id":"service-account"})
                .await
                .unwrap()
                .is_none()
        );
        // Legacy accounts without owner_user_id use the creator, just like writers.
        fences.delete_one(doc! {"_id":"person"}).await.unwrap();
        account.remove("owner_user_id");
        accounts
            .replace_one(doc! {"_id":"service-account"}, &account)
            .await
            .unwrap();
        assert!(is_erasure_accepted(&db, "service-account").await.unwrap());
        assert_eq!(
            accounts
                .find_one(doc! {"_id":"service-account"})
                .await
                .unwrap()
                .unwrap(),
            account
        );
        assert_eq!(fences.count_documents(doc! {}).await.unwrap(), 1);
        // The account's own negative fence remains authoritative independently.
        fences.delete_one(doc! {"_id":"creator"}).await.unwrap();
        let own_fence = doc! {"_id":"service-account","erasure_accepted":true,"revision":3_i64};
        fences.insert_one(&own_fence).await.unwrap();
        assert!(is_erasure_accepted(&db, "service-account").await.unwrap());
        assert_eq!(
            fences
                .find_one(doc! {"_id":"service-account"})
                .await
                .unwrap()
                .unwrap(),
            own_fence
        );
        assert_eq!(
            accounts
                .find_one(doc! {"_id":"service-account"})
                .await
                .unwrap()
                .unwrap(),
            account
        );
        db.drop().await.unwrap();
    }
}
