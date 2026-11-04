//! Per-owner catalog read fences. Steady-state admission never writes a shared
//! catalog row. A catalog mutation invalidates registered readers in its own
//! transaction; first registration writes the catalog to close the phantom gap.
use crate::errors::{AppError, AppResult};
use mongodb::{
    ClientSession, Database,
    bson::{Document, doc},
};
use sha2::{Digest, Sha256};

pub(crate) const COLLECTION: &str = "catalog_reader_fences";

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<Document>(COLLECTION)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"catalog_id":1})
                .build(),
        )
        .await?;
    Ok(())
}

pub async fn admit(
    db: &Database,
    catalog: &str,
    owner: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let id = hex::encode(Sha256::digest(format!(
        "{}:{catalog}{owner}",
        catalog.len()
    )));
    let rows = db.collection::<Document>(COLLECTION);
    if rows
        .update_one(doc! {"_id": &id}, doc! {"$inc": {"fence": 1_i64}})
        .session(&mut *session)
        .await?
        .matched_count
        == 1
    {
        return Ok(());
    }
    // A concurrent mutation must retry its reader scan if this owner registers
    // after that scan's snapshot. Registration and the first effect commit as one.
    if db
        .collection::<Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(
            doc! {"_id": catalog, "is_active": true},
            doc! {"$inc": {"reader_registration_fence": 1_i64}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(AppError::ApiKeyScopeForbidden(
            "Catalog service is unavailable".into(),
        ));
    }
    rows.insert_one(doc! {"_id": id, "catalog_id": catalog, "owner_id": owner, "fence": 1_i64})
        .session(session)
        .await?;
    Ok(())
}

/// Every supported catalog authority mutation enters catalog_mutation_service.
/// Keep registrations after disconnect: deleting one would reopen the race.
/// The personal-erasure tombstone is the sole owner-cleanup exception.
pub async fn invalidate(
    db: &Database,
    catalog: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<Document>(COLLECTION)
        .update_many(
            doc! {"catalog_id": catalog},
            doc! {"$inc": {"fence": 1_i64}},
        )
        .session(session)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::{
        COLLECTION_NAME as CATALOG, test_helpers::dummy_service,
    };
    async fn begin(db: &Database) -> ClientSession {
        let mut tx = db.client().start_session().await.unwrap();
        tx.start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .write_concern(mongodb::options::WriteConcern::majority())
            .await
            .unwrap();
        tx
    }
    async fn register(db: &Database, id: &str, owner: &str) {
        let mut tx = begin(db).await;
        admit(db, id, owner, &mut tx).await.unwrap();
        tx.commit_transaction().await.unwrap();
    }

    #[tokio::test]
    async fn independent_owners_do_not_contend_and_catalog_mutation_conflicts_with_admission() {
        let db = crate::test_utils::connect_transaction_test_database(
            "catalog_reader_fence_concurrency",
        )
        .await;
        ensure_indexes(&db).await.unwrap();
        let catalog = dummy_service();
        db.collection::<crate::models::downstream_service::DownstreamService>(CATALOG)
            .insert_one(&catalog)
            .await
            .unwrap();
        register(&db, &catalog.id, "alice").await;
        register(&db, &catalog.id, "bob").await;
        let mut alice = begin(&db).await;
        admit(&db, &catalog.id, "alice", &mut alice).await.unwrap();
        let mut bob = begin(&db).await;
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            admit(&db, &catalog.id, "bob", &mut bob),
        )
        .await
        .unwrap()
        .unwrap();
        bob.commit_transaction().await.unwrap();
        let mut mutation = begin(&db).await;
        assert!(
            crate::services::catalog_mutation_service::fence(&db, &mut mutation, &catalog, None)
                .await
                .is_err(),
            "Catalog mutation must conflict with the still-open Alice admission"
        );
        mutation.abort_transaction().await.unwrap();
        alice.commit_transaction().await.unwrap();
        let stored = db
            .collection::<Document>(CATALOG)
            .find_one(doc! {"_id": &catalog.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.get_i64("reader_registration_fence"),
            Ok(2),
            "Steady-state readers must never write the catalog"
        );
        let mut mutation = begin(&db).await;
        crate::services::catalog_mutation_service::fence(&db, &mut mutation, &catalog, None)
            .await
            .unwrap();
        let mut blocked = begin(&db).await;
        assert!(admit(&db, &catalog.id, "bob", &mut blocked).await.is_err());
        blocked.abort_transaction().await.unwrap();
        mutation.commit_transaction().await.unwrap();
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn first_registration_and_catalog_mutation_cannot_miss_each_other() {
        let db = crate::test_utils::connect_transaction_test_database(
            "catalog_reader_fence_registration",
        )
        .await;
        let catalog = dummy_service();
        db.collection::<crate::models::downstream_service::DownstreamService>(CATALOG)
            .insert_one(&catalog)
            .await
            .unwrap();
        let mut mutation = begin(&db).await;
        crate::services::catalog_mutation_service::fence(&db, &mut mutation, &catalog, None)
            .await
            .unwrap();
        let mut reader = begin(&db).await;
        assert!(
            admit(&db, &catalog.id, "new-owner", &mut reader)
                .await
                .is_err()
        );
        reader.abort_transaction().await.unwrap();
        mutation.commit_transaction().await.unwrap();
        let mut reader = begin(&db).await;
        admit(&db, &catalog.id, "new-owner", &mut reader)
            .await
            .unwrap();
        let mut mutation = begin(&db).await;
        assert!(
            crate::services::catalog_mutation_service::fence(&db, &mut mutation, &catalog, None)
                .await
                .is_err()
        );
        mutation.abort_transaction().await.unwrap();
        reader.commit_transaction().await.unwrap();
        db.drop().await.unwrap();
    }
}
