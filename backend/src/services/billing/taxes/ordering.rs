//! Publication fences for Lago tax reads; value readers keep their existing contract.
use super::{BillingTaxProfile, BillingWallet, COLLECTION_NAME};
use crate::{
    errors::{AppError, AppResult},
    models::{
        billing_tax_plan_epoch::{BillingTaxPlanEpoch, COLLECTION_NAME as EPOCHS, EPOCH_ID},
        billing_wallet::COLLECTION_NAME as WALLETS,
    },
};
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
    options::ReturnDocument,
};

pub(super) async fn capture_plan_epoch(db: &Database) -> AppResult<i64> {
    let epochs = db.collection::<BillingTaxPlanEpoch>(EPOCHS);
    if let Some(epoch) = epochs.find_one(doc! {"_id": EPOCH_ID}).await? {
        return Ok(epoch.generation);
    }
    let initialized = epochs
        .find_one_and_update(
            doc! {"_id": EPOCH_ID},
            doc! {"$setOnInsert": {
                "generation": 0_i64,
                "guard_token": uuid::Uuid::new_v4().to_string(),
            }},
        )
        .upsert(true)
        .return_document(ReturnDocument::After)
        .await;
    let epoch = match initialized {
        Err(error) if super::super::ledger::is_duplicate_key_error(&error) => {
            epochs.find_one(doc! {"_id":EPOCH_ID}).await?
        }
        result => result?,
    }
    .ok_or_else(|| AppError::Internal("Tax plan epoch disappeared during capture".into()))?;
    Ok(epoch.generation)
}

fn generation_eq(field: &str, captured: i64) -> Document {
    doc! {"$eq": [{"$ifNull": [format!("${field}"), 0_i64]}, captured]}
}

fn profile_filter(profile: &BillingTaxProfile) -> Document {
    // Fingerprint binds the resolved tax values. The other fields also detect
    // old binaries' ordinary value/timestamp writes without a revision bump.
    doc! {
        "_id": &profile.id, "owner_id": &profile.owner_id,
        "$expr": generation_eq("revision", profile.revision),
        "subscription_id": &profile.subscription_id, "plan_code": &profile.plan_code,
        "billing_entity_code": &profile.billing_entity_code, "fingerprint": &profile.fingerprint,
        "fetched_at": bson::DateTime::from_chrono(profile.fetched_at),
        "refresh_failed_at": profile.refresh_failed_at.map(bson::DateTime::from_chrono),
    }
}
fn wallet_filter(wallet: &BillingWallet) -> Document {
    doc! {
        "_id": &wallet.id, "owner_id": &wallet.owner_id,
        "lago_customer_id": &wallet.lago_customer_id,
        "lago_subscription_id": &wallet.lago_subscription_id,
        "lago_wallet_id": &wallet.lago_wallet_id,
        "$expr": generation_eq("tax_generation", wallet.tax_generation),
    }
}

#[derive(Debug)]
struct PublicationLost;

pub(super) async fn publish(
    db: &Database,
    wallet: &BillingWallet,
    captured: Option<&BillingTaxProfile>,
    epoch: i64,
    mut value: BillingTaxProfile,
) -> AppResult<Option<BillingTaxProfile>> {
    value.revision = captured
        .map_or(0, |p| p.revision)
        .checked_add(1)
        .ok_or_else(|| AppError::Internal("Tax profile revision exhausted".into()))?;
    if let Some(captured) = captured {
        value.id.clone_from(&captured.id);
    }
    let mut fields = bson::to_document(&value).map_err(|e| AppError::Internal(e.to_string()))?;
    fields.remove("_id");
    // Missing/repaired subscription identities need the wallet guard too.
    if let Some(captured) = captured.filter(|p| {
        !p.fingerprint.is_empty()
            && !p.plan_code.is_empty()
            && p.plan_code == value.plan_code
            && !p.subscription_id.is_empty()
            && p.subscription_id == value.subscription_id
    }) {
        let update = db
            .collection::<Document>(COLLECTION_NAME)
            .update_one(profile_filter(captured), doc! {"$set":fields})
            .await?;
        return Ok((update.matched_count == 1).then_some(value));
    }

    let mut session = db.client().start_session().await?;
    let database = db.clone();
    let wallet_match = wallet_filter(wallet);
    let profile_match = captured.map(profile_filter);
    let inserted = value.clone();
    // The original capture and payload remain fixed across transaction retries.
    let outcome = session
        .start_transaction()
        .and_run2(async move |session| {
            publish_in_session(
                &database,
                session,
                &wallet_match,
                epoch,
                profile_match.as_ref(),
                &fields,
                &inserted,
            )
            .await
        })
        .await;
    match outcome {
        Ok(()) => Ok(Some(value)),
        Err(error)
            if error.get_custom::<PublicationLost>().is_some()
                || super::super::ledger::is_duplicate_key_error(&error) =>
        {
            Ok(None)
        }
        Err(error) => Err(error.into()),
    }
}

async fn publish_in_session(
    db: &Database,
    session: &mut ClientSession,
    wallet_match: &Document,
    epoch: i64,
    profile_match: Option<&Document>,
    fields: &Document,
    inserted: &BillingTaxProfile,
) -> mongodb::error::Result<()> {
    let guarded = db
        .collection::<Document>(WALLETS)
        .update_one(
            wallet_match.clone(),
            doc! {"$set":{"tax_guard_token":uuid::Uuid::new_v4().to_string()}},
        )
        .session(&mut *session)
        .await?;
    if guarded.matched_count != 1 {
        return Err(mongodb::error::Error::custom(PublicationLost));
    }
    let guarded = db
        .collection::<Document>(EPOCHS)
        .update_one(
            doc! {"_id":EPOCH_ID, "$expr":generation_eq("generation", epoch)},
            doc! {"$set":{"guard_token":uuid::Uuid::new_v4().to_string()}},
        )
        .session(&mut *session)
        .await?;
    if guarded.matched_count != 1 {
        return Err(mongodb::error::Error::custom(PublicationLost));
    }
    #[cfg(test)]
    super::ordering_tests::point(super::ordering_tests::Phase::GuardsWritten).await;
    if let Some(filter) = profile_match {
        let changed = db
            .collection::<Document>(COLLECTION_NAME)
            .update_one(filter.clone(), doc! {"$set":fields})
            .session(&mut *session)
            .await?;
        if changed.matched_count != 1 {
            return Err(mongodb::error::Error::custom(PublicationLost));
        }
    } else {
        db.collection::<BillingTaxProfile>(COLLECTION_NAME)
            .insert_one(inserted)
            .session(&mut *session)
            .await?;
    }
    Ok(())
}

fn invalidated_fields(clear_failure: bool) -> Document {
    let mut update =
        doc! {"$set":{"fetched_at":bson::DateTime::from_millis(0)}, "$inc":{"revision":1_i64}};
    if clear_failure {
        update.insert("$unset", doc! {"refresh_failed_at":""});
    }
    update
}

pub(super) async fn invalidate_owner(
    db: &Database,
    owner: &str,
) -> AppResult<Option<BillingWallet>> {
    let wallet = db
        .collection::<BillingWallet>(WALLETS)
        .find_one_and_update(
            doc! {"owner_id":owner},
            doc! {"$inc":{"tax_generation":1_i64}},
        )
        .return_document(ReturnDocument::After)
        .await?;
    #[cfg(test)]
    super::ordering_tests::point(super::ordering_tests::Phase::InvalidationAdvanced).await;
    // Completion is this row update, not the preceding counter increment.
    db.collection::<Document>(COLLECTION_NAME)
        .update_many(doc! {"owner_id":owner}, invalidated_fields(false))
        .await?;
    Ok(wallet)
}

pub(super) async fn invalidate_plan(db: &Database, plan: &str) -> AppResult<()> {
    let epochs = db.collection::<Document>(EPOCHS);
    let filter = doc! {"_id":EPOCH_ID};
    let update = doc! {
        "$inc": {"generation": 1_i64},
        "$setOnInsert": {"guard_token": uuid::Uuid::new_v4().to_string()},
    };
    match epochs
        .update_one(filter.clone(), update.clone())
        .upsert(true)
        .await
    {
        Err(error) if super::super::ledger::is_duplicate_key_error(&error) => {
            epochs.update_one(filter, update).await?;
        }
        result => {
            result?;
        }
    }
    #[cfg(test)]
    super::ordering_tests::point(super::ordering_tests::Phase::InvalidationAdvanced).await;
    db.collection::<Document>(COLLECTION_NAME)
        .update_many(doc! {"plan_code":plan}, invalidated_fields(true))
        .await?;
    Ok(())
}

pub(super) async fn repair_subscription(
    db: &Database,
    wallet: &BillingWallet,
    subscription: &str,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let database = db.clone();
    let wallet = wallet.clone();
    let subscription = subscription.to_owned();
    session
        .start_transaction()
        .and_run2(async move |session| {
            repair_in_session(&database, session, &wallet, &subscription).await
        })
        .await?;
    Ok(())
}
async fn repair_in_session(
    db: &Database,
    session: &mut ClientSession,
    wallet: &BillingWallet,
    subscription: &str,
) -> mongodb::error::Result<()> {
    let changed = db
        .collection::<Document>(WALLETS)
        .update_one(
            doc! {
                "_id": &wallet.id,
                "owner_id": &wallet.owner_id,
                "lago_customer_id": &wallet.lago_customer_id,
                "$or": [
                    {"lago_subscription_id": bson::Bson::Null},
                    {"lago_subscription_id": ""},
                ],
            },
            doc! {
                "$set": {
                    "lago_subscription_id": subscription,
                    "updated_at": bson::DateTime::now(),
                },
                "$inc": {"tax_generation": 1_i64},
            },
        )
        .session(&mut *session)
        .await?;
    if changed.matched_count == 1 {
        db.collection::<Document>(COLLECTION_NAME)
            .update_many(
                doc! {"owner_id":&wallet.owner_id},
                invalidated_fields(false),
            )
            .session(&mut *session)
            .await?;
        #[cfg(test)]
        super::ordering_tests::point(super::ordering_tests::Phase::RepairBeforeCommit).await;
    }
    Ok(())
}
