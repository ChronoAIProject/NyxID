//! Only a versioned preparation is proof of no provider effect. The outer state
//! stays `sending`, so older workers conservatively leave these rows alone.
use crate::{
    errors::{AppError, AppResult},
    models::nyxbot_image_delivery::{COLLECTION_NAME, NyxbotImageDelivery},
};
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::{Database, options::Hint};

pub const PREPARATION_SECS: i64 = 180;
pub const SEND_SECS: i64 = 60;
pub const RETRY_SECS: i64 = 30;
pub const DUE_INDEX: &str = "image_preparation_due";

pub fn due() -> Document {
    doc! {"state":"sending", "preparation.version":1, "preparation.parked":false,
    "$expr":{"$and":[
        {"$eq":[{"$type":"$preparation.lease_until"},"date"]},
        {"$lte":["$preparation.lease_until","$$NOW"]}
    ]}}
}

pub async fn due_rows(db: &Database) -> AppResult<Vec<NyxbotImageDelivery>> {
    Ok(db
        .collection::<NyxbotImageDelivery>(COLLECTION_NAME)
        .find(due())
        .hint(Hint::Name(DUE_INDEX.into()))
        .sort(doc! {"preparation.lease_until":1,"_id":1})
        .limit(32)
        .await?
        .try_collect()
        .await?)
}

pub fn lost() -> AppError {
    AppError::Conflict("Attachment preparation changed".into())
}

pub async fn before_effect(db: &Database, id: &str, attempt: &str) -> AppResult<()> {
    // Repeated calls are uploads and the final message of this same bounded
    // attempt. Once any one may have happened, this row is never reclaimed.
    let changed = db
        .collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":id,"state":"sending","attempt":attempt,"preparation.version":1,
            "preparation.phase":{"$in":["preparing","sending"]},
            "$expr":{"$gt":["$preparation.lease_until","$$NOW"]}},
            vec![doc! {"$set":{
                "preparation.lease_until":{"$cond":[{"$eq":["$preparation.phase","preparing"]},
                    {"$add":["$$NOW",SEND_SECS*1000]},"$preparation.lease_until"]},
                "preparation.phase":"sending",
                "preparation.effects":{"$add":[{"$ifNull":["$preparation.effects",0]},1]},
            }}],
        )
        .await?;
    if changed.matched_count == 1 {
        Ok(())
    } else {
        Err(lost())
    }
}

tokio::task_local! { static EFFECT: super::channel_platform::PostEffectGuard; }
pub async fn with_effect<F: std::future::Future>(
    guard: super::channel_platform::PostEffectGuard,
    work: F,
) -> F::Output {
    EFFECT.scope(guard, work).await
}
pub async fn effect() -> AppResult<()> {
    if let Ok(guard) = EFFECT.try_with(Clone::clone) {
        #[cfg(test)]
        super::durable_wake_tests::after("image_prepared").await;
        guard().await?;
        #[cfg(test)]
        super::durable_wake_tests::after("image_dispatch_committed").await;
    }
    Ok(())
}

/// No provider effect occurred. Stop automatic retries after a permanent
/// source/authority refusal, while a later independently authorized request may
/// still claim this unsent source/audience slot with a fresh attempt.
pub async fn park(db: &Database, id: &str, attempt: &str) -> AppResult<()> {
    db.collection::<Document>(COLLECTION_NAME).update_one(
        doc! {"_id":id,"state":"sending","attempt":attempt,"preparation.version":1,
            "preparation.phase":{"$in":["preparing","released"]}},
        doc! {"$set":{"preparation.phase":"released","preparation.parked":true,"code":"recovery_parked"}},
    ).await?;
    Ok(())
}

/// A receipt/refusal recorded after the sweep read must win over that stale read.
pub async fn retire_dispatch(db: &Database, id: &str, attempt: &str) -> AppResult<()> {
    let mut filter = due();
    filter.extend(doc! {"_id":id,"attempt":attempt,"preparation.phase":"sending"});
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(filter, doc! {"$set":{"state":"unknown"}})
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        models::{image_preparation::Route, nyxbot_image_delivery::ImageDeliveryState},
        services::channel_image_delivery as claims,
    };

    async fn fixture() -> Database {
        let db =
            crate::test_utils::connect_transaction_test_database("attachment_preparation").await;
        claims::ensure_indexes(&db).await.unwrap();
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                "owner",
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .insert_one(doc! {"_id":"conversation","user_id":"owner"})
            .await
            .unwrap();
        db
    }
    async fn claim(db: &Database, attachment: &str, route: &Route) -> claims::ImageClaim {
        claims::prepare_for_turn(
            db,
            "owner",
            "conversation",
            "turn",
            "channel",
            attachment,
            "audience",
            route,
        )
        .await
        .unwrap()
    }
    fn dispatch(result: claims::ImageClaim) -> (String, String) {
        match result {
            claims::ImageClaim::Dispatch { id, attempt } => (id, attempt),
            _ => panic!("expected preparation"),
        }
    }
    fn route() -> Route {
        Route::Manager {
            delivery_id: "delivery".into(),
        }
    }
    async fn expire(db: &Database, id: &str) {
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":id},
                doc! {"$set":{"preparation.lease_until":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn attachment_preparation_two_survivors_fence_the_slow_worker_and_never_reclaim_dispatch()
    {
        let db = fixture().await;
        let route = route();
        let (id, old) = dispatch(claim(&db, "image", &route).await);
        expire(&db, &id).await;
        let (a, b) = tokio::join!(claim(&db, "image", &route), claim(&db, "image", &route));
        let next = match (a, b) {
            (claims::ImageClaim::Dispatch { attempt, .. }, claims::ImageClaim::Existing(_))
            | (claims::ImageClaim::Existing(_), claims::ImageClaim::Dispatch { attempt, .. }) => {
                attempt
            }
            _ => panic!("exactly one survivor must own preparation"),
        };
        assert!(before_effect(&db, &id, &old).await.is_err());
        assert!(matches!(
            claims::finish(&db, &id, &old, ImageDeliveryState::Sent, None)
                .await
                .unwrap(),
            claims::ImageCompletion::Existing(_)
        ));
        before_effect(&db, &id, &next).await.unwrap();
        // A lost response to the boundary write cannot release a possible effect.
        assert!(matches!(
            claims::release(&db, &id, &next).await.unwrap(),
            claims::ImageCompletion::Existing(_)
        ));
        expire(&db, &id).await;
        assert!(matches!(
            claim(&db, "image", &route).await,
            claims::ImageClaim::Existing(_)
        ));
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn attachment_preparation_release_preserves_unsent_proof_and_refusals_after_upload_do_not_retry()
     {
        let db = fixture().await;
        let (id, attempt) = dispatch(claim(&db, "image", &route()).await);
        before_effect(&db, &id, &attempt).await.unwrap();
        assert!(matches!(
            claims::release_refused(&db, &id, &attempt).await.unwrap(),
            claims::ImageCompletion::Recorded
        ));
        let (_, next) = dispatch(
            claim(
                &db,
                "image",
                &Route::Manager {
                    delivery_id: "other-delivery".into(),
                },
            )
            .await,
        );
        assert!(before_effect(&db, &id, &attempt).await.is_err());
        before_effect(&db, &id, &next).await.unwrap();
        before_effect(&db, &id, &next).await.unwrap();
        assert!(matches!(
            claims::release_refused(&db, &id, &next).await.unwrap(),
            claims::ImageCompletion::Existing(_)
        ));
        let (parked, attempt) = dispatch(claim(&db, "parked", &route()).await);
        park(&db, &parked, &attempt).await.unwrap();
        expire(&db, &parked).await;
        assert!(due_rows(&db).await.unwrap().is_empty());
        assert!(matches!(
            claim(&db, "parked", &route()).await,
            claims::ImageClaim::Dispatch { .. }
        ));
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn attachment_preparation_idle_is_one_indexed_read_and_legacy_states_are_never_recovered()
    {
        let db = fixture().await;
        for state in ["sending", "sent", "unknown", "failed", "skipped"] {
            db.collection::<Document>(COLLECTION_NAME)
                .insert_one(doc! {"_id":state,"state":state})
                .await
                .unwrap();
        }
        let (monitored, counts) = crate::services::durable_wake_tests::monitored(&db).await;
        assert!(due_rows(&monitored).await.unwrap().is_empty());
        let counts = crate::services::durable_wake_tests::take(&counts);
        assert_eq!(counts.values().sum::<usize>(), 1, "{counts:?}");
        println!("ATTACHMENT_IDLE_COMMANDS {counts:?}");
        let explain=db.run_command(doc! {"explain":{"find":COLLECTION_NAME,"filter":due(),"hint":DUE_INDEX},"verbosity":"queryPlanner"}).await.unwrap();
        let plan = serde_json::to_string(&explain).unwrap();
        assert!(plan.contains("IXSCAN"), "{plan}");
        assert!(!plan.contains("COLLSCAN"), "{plan}");
        println!("ATTACHMENT_DUE_PLAN {plan}");
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn attachment_preparation_lease_is_database_timed_and_old_reader_sees_known_sending_state()
     {
        let db = fixture().await;
        let before = db
            .run_command(doc! {"hello":1})
            .await
            .unwrap()
            .get_datetime("localTime")
            .unwrap()
            .timestamp_millis();
        let (id, _) = dispatch(claim(&db, "image", &route()).await);
        let after = db
            .run_command(doc! {"hello":1})
            .await
            .unwrap()
            .get_datetime("localTime")
            .unwrap()
            .timestamp_millis();
        let mut raw = db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
        let lease = raw
            .get_document("preparation")
            .unwrap()
            .get_datetime("lease_until")
            .unwrap()
            .timestamp_millis();
        assert!(
            lease >= before + PREPARATION_SECS * 1000 && lease <= after + PREPARATION_SECS * 1000
        );
        #[derive(serde::Deserialize)]
        struct OldReader {
            state: ImageDeliveryState,
            attempt: Option<String>,
        }
        let old: OldReader = bson::from_document(raw.clone()).unwrap();
        assert_eq!(old.state, ImageDeliveryState::Sending);
        assert!(old.attempt.is_some());
        raw.remove("preparation");
        assert!(
            bson::from_document::<NyxbotImageDelivery>(raw)
                .unwrap()
                .preparation
                .is_none()
        );
        db.drop().await.unwrap();
    }
}
