//! Explicit preparation only. No startup or reconciliation hook calls this module.
use super::policy::unavailable;
use crate::{
    errors::AppResult,
    models::sandbox_usage_rates::{BUCKET_REVISIONS, COLLECTION_NAME, PINS, POLICIES, SCHEMA},
    services::feature_flag_service as flags,
};
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{Document, doc},
    options::{Acknowledgment, IndexOptions, ReadConcern, WriteConcern},
};

pub async fn require_enabled(db: &Database) -> AppResult<()> {
    if !flags::cma_sandbox_usage_charge_enabled(db).await? {
        return Err(unavailable("Sandbox usage shadow pricing is disabled"));
    }
    Ok(())
}

pub async fn prepare(db: &Database) -> AppResult<()> {
    require_enabled(db).await?;
    for (name, keys, unique) in [
        (SCHEMA, doc! {"schema_version":1}, true),
        (
            COLLECTION_NAME,
            doc! {"producer_namespace":1,"version":1},
            true,
        ),
        (
            COLLECTION_NAME,
            doc! {"producer_namespace":1,"effective_from":1},
            true,
        ),
        (POLICIES, doc! {"scope_key":1,"version":1}, true),
        (POLICIES, doc! {"scope_key":1,"effective_from":1}, true),
        (POLICIES, doc! {"scope.owner_id":1}, false),
        (PINS, doc! {"scope_key":1,"minute":1}, true),
        (PINS, doc! {"scope.owner_id":1}, false),
        (BUCKET_REVISIONS, doc! {"bucket_id":1,"revision":1}, true),
        (BUCKET_REVISIONS, doc! {"owner_subject":1}, false),
    ] {
        db.collection::<Document>(name)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(IndexOptions::builder().unique(unique).build())
                    .build(),
            )
            .await?;
    }
    db.collection::<Document>(SCHEMA).update_one(doc! {"schema_version":1},
        doc! {"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),"schema_version":1,"revision":0_i64},"$set":{"ready":true}})
        .upsert(true).await?;
    Ok(())
}

pub(super) async fn begin(db: &Database) -> AppResult<ClientSession> {
    require_enabled(db).await?;
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(ReadConcern::snapshot())
        .write_concern(WriteConcern::builder().w(Acknowledgment::Majority).build())
        .await?;
    // Contend with the ordinary flag writer: revocation cannot race a new pin.
    let enabled = db
        .collection::<Document>(crate::models::feature_flag_override::COLLECTION_NAME)
        .update_one(
            doc! {"flag_key":flags::CMA_SANDBOX_USAGE_CHARGE_FLAG_KEY,
            "org_user_id":bson::Bson::Null,"definition_id":bson::Bson::Null,
            "target_kind":"global","target_key":bson::Bson::Null,"enabled":true},
            doc! {"$inc":{"usage_pricing_revision":1_i64}},
        )
        .session(&mut session)
        .await?;
    if enabled.matched_count != 1 {
        return Err(unavailable("Sandbox usage shadow pricing is disabled"));
    }
    let ready = db
        .collection::<Document>(SCHEMA)
        .update_one(
            doc! {"schema_version":1,"ready":true},
            doc! {"$inc":{"revision":1_i64}},
        )
        .session(&mut session)
        .await?;
    if ready.matched_count != 1 {
        return Err(unavailable(
            "Prepare Sandbox usage pricing schema v1 explicitly",
        ));
    }
    Ok(session)
}

pub(super) async fn commit(session: &mut ClientSession) -> AppResult<()> {
    for _ in 0..8 {
        match session.commit_transaction().await {
            Ok(()) => return Ok(()),
            Err(e) if e.contains_label("UnknownTransactionCommitResult") => continue,
            Err(e) => return Err(e.into()),
        }
    }
    Err(unavailable(
        "Usage pricing commit is uncertain; inspect the original version before retrying",
    ))
}
