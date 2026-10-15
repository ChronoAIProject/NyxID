//! Durable arbitration between the provider stream and asynchronous channel replies.
//! Never reclaim a dispatch barrier: an interrupted external send is ambiguous.
use bson::doc;
use chrono::Utc;
use mongodb::{ClientSession, Database};
use mongodb::{IndexModel, options::IndexOptions};
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::nyxbot_channel::EVENTS_COLLECTION_NAME as EVENTS,
    models::nyxbot_image_delivery::{
        COLLECTION_NAME as IMAGE_DELIVERIES, ImageDeliveryItem, NyxbotImageDelivery,
    },
};

// Fixed namespace for deterministic per-turn, per-destination image claims.
const IMAGE_DELIVERY_NAMESPACE: Uuid = Uuid::from_u128(0x12d62f3e_f5bb_4e22_a218_9104c4662329);

pub fn image_delivery_id(conversation_id: &str, turn_id: &str, destination_key: &str) -> String {
    let key = format!("v1\0{conversation_id}\0{turn_id}\0{destination_key}");
    Uuid::new_v5(&IMAGE_DELIVERY_NAMESPACE, key.as_bytes()).to_string()
}

pub async fn ensure_image_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<bson::Document>(IMAGE_DELIVERIES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

/// Claim an image batch after its text delivery barrier. A sending or done
/// row is deliberately never reclaimed: an interrupted provider request is
/// ambiguous and retrying could duplicate an image.
pub async fn claim_images(
    db: &Database,
    user_id: &str,
    conversation_id: &str,
    turn_id: &str,
    channel_id: &str,
    destination_key: &str,
    image_ids: &[String],
) -> AppResult<Option<NyxbotImageDelivery>> {
    let id = image_delivery_id(conversation_id, turn_id, destination_key);
    let collection = db.collection::<NyxbotImageDelivery>(IMAGE_DELIVERIES);
    let now = Utc::now();
    let row = NyxbotImageDelivery {
        id: id.clone(),
        user_id: user_id.to_owned(),
        conversation_id: conversation_id.to_owned(),
        turn_id: turn_id.to_owned(),
        channel_id: channel_id.to_owned(),
        destination_key: destination_key.to_owned(),
        state: "pending".into(),
        claim_id: String::new(),
        items: image_ids
            .iter()
            .map(|id| ImageDeliveryItem {
                id: id.clone(),
                state: "pending".into(),
                code: None,
            })
            .collect(),
        created_at: now,
        expires_at: now + chrono::Duration::days(7),
    };
    match collection.insert_one(&row).await {
        Ok(_) => {}
        Err(error) if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(w)) if w.code == 11000) =>
        {
            let Some(existing) = collection.find_one(doc! {"_id": &id}).await? else {
                return Ok(None);
            };
            if existing.state != "pending" {
                return Ok(None);
            }
        }
        Err(error) => return Err(error.into()),
    }
    let claim_id = Uuid::new_v4().to_string();
    let updated = collection
        .update_one(
            doc! {"_id": &id, "state": "pending"},
            doc! {"$set": {"state": "sending", "claim_id": &claim_id}},
        )
        .await?;
    if updated.modified_count != 1 {
        return Ok(None);
    }
    Ok(collection.find_one(doc! {"_id": id}).await?)
}

pub async fn read_images(
    db: &Database,
    conversation_id: &str,
    turn_id: &str,
    destination_key: &str,
) -> AppResult<Option<NyxbotImageDelivery>> {
    Ok(db
        .collection::<NyxbotImageDelivery>(IMAGE_DELIVERIES)
        .find_one(doc! {"_id": image_delivery_id(conversation_id, turn_id, destination_key)})
        .await?)
}

pub async fn claim_image_item(
    db: &Database,
    delivery_id: &str,
    claim_id: &str,
    image_id: &str,
) -> AppResult<bool> {
    Ok(db.collection::<bson::Document>(IMAGE_DELIVERIES).update_one(
        doc! {"_id": delivery_id, "claim_id": claim_id, "state": "sending", "items": {"$elemMatch": {"id": image_id, "state": "pending"}}},
        doc! {"$set": {"items.$.state": "sending"}},
    ).await?.modified_count == 1)
}

pub async fn finish_image_item(
    db: &Database,
    delivery_id: &str,
    claim_id: &str,
    image_id: &str,
    state: &str,
    code: Option<&str>,
) -> AppResult<()> {
    let mut set = doc! {"items.$.state": state};
    if let Some(code) = code {
        set.insert("items.$.code", code);
    }
    db.collection::<bson::Document>(IMAGE_DELIVERIES).update_one(
        doc! {"_id": delivery_id, "claim_id": claim_id, "state": "sending", "items": {"$elemMatch": {"id": image_id, "state": "sending"}}},
        doc! {"$set": set},
    ).await?;
    Ok(())
}

pub async fn finish_images(db: &Database, delivery_id: &str, claim_id: &str) -> AppResult<()> {
    db.collection::<bson::Document>(IMAGE_DELIVERIES)
        .update_one(
            doc! {"_id": delivery_id, "claim_id": claim_id, "state": "sending"},
            doc! {"$set": {"state": "done"}},
        )
        .await?;
    Ok(())
}

pub async fn bind_in_session(
    db: &Database,
    session: &mut ClientSession,
    event: &str,
    owner: &str,
    conversation: &str,
    turn: &str,
) -> AppResult<()> {
    let bound = db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "user_id": owner, "status": "running",
            "turn_id": bson::Bson::Null, "delivery.version": 1},
            doc! {"$set": {"conversation_id": conversation, "turn_id": turn}},
        )
        .session(session)
        .await?;
    if bound.matched_count != 1 {
        return Err(AppError::Conflict("Channel event already admitted".into()));
    }
    Ok(())
}

pub async fn detach(db: &Database, event: &str) -> AppResult<()> {
    db.collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "delivery.version": 1, "delivery.state": "waiting"},
            doc! {"$set": {"delivery.state": "pending"}},
        )
        .await?;
    Ok(())
}

/// The write precedes the first committed message frame. Once claimed, a lost
/// stream cannot safely be retried without a gateway delivery acknowledgement.
pub async fn claim_stream(db: &Database, event: &str, status: &str) -> AppResult<bool> {
    Ok(db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "delivery.version": 1, "delivery.state": "waiting",
            "delivery.stream_deadline": {"$gt": bson::DateTime::from_chrono(Utc::now())}},
            doc! {"$set": {"delivery.state": "streamed", "status": status}},
        )
        .await?
        .modified_count
        == 1)
}

/// One irreversible outbound attempt, shared by the callback and all sweepers.
pub async fn claim_send(db: &Database, event: &str, claim: &str) -> AppResult<bool> {
    Ok(db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "delivery.version": 1, "delivery.state": "pending"},
            doc! {"$set": {"delivery.state": "sending", "delivery.claim_id": claim}},
        )
        .await?
        .modified_count
        == 1)
}

pub async fn finish(db: &Database, event: &str, claim: &str, status: &str) -> AppResult<()> {
    db.collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id": event, "delivery.state": "sending", "delivery.claim_id": claim},
            doc! {"$set": {"delivery.state": status}},
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod image_claim_tests {
    use super::*;
    #[tokio::test]
    async fn image_claims_are_concurrent_single_attempt_and_never_reclaim_sending_items() {
        let db = crate::test_utils::connect_transaction_test_database("image_claims").await;
        ensure_image_indexes(&db).await.unwrap();
        let ids = vec!["image-1".into(), "image-2".into()];
        let claim = || {
            claim_images(
                &db,
                "owner",
                "conversation",
                "turn",
                "channel",
                "inbound:source",
                &ids,
            )
        };
        let (first, second) = tokio::join!(claim(), claim());
        let first = first.unwrap();
        let second = second.unwrap();
        assert_eq!(
            usize::from(first.is_some()) + usize::from(second.is_some()),
            1
        );
        let row = first.or(second).unwrap();
        assert_eq!(
            row.id,
            image_delivery_id("conversation", "turn", "inbound:source")
        );
        let parsed = Uuid::parse_str(&row.id).unwrap();
        assert_eq!(parsed.get_version(), Some(uuid::Version::Sha1));
        assert_ne!(
            row.id,
            image_delivery_id("conversation", "turn", "inbound:other")
        );
        assert_ne!(
            row.id,
            image_delivery_id("conversation", "other", "inbound:source")
        );
        assert!(
            claim_image_item(&db, &row.id, &row.claim_id, &ids[0])
                .await
                .unwrap()
        );
        assert!(
            !claim_image_item(&db, &row.id, &row.claim_id, &ids[0])
                .await
                .unwrap()
        );
        assert!(claim().await.unwrap().is_none());
        finish_images(&db, &row.id, &row.claim_id).await.unwrap();
        finish_images(&db, &row.id, &row.claim_id).await.unwrap();
        assert!(claim().await.unwrap().is_none());
        let row = read_images(&db, "conversation", "turn", "inbound:source")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.state, "done");
        assert_eq!(row.items[0].state, "sending");
        db.drop().await.unwrap();
    }
}
