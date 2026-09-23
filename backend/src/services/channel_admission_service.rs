//! Inbound admission shares the coordination store and commits with metadata.

use super::coordination_service::{EventDedupClaim, EventDedupClaimResult, EventDedupStore};
use crate::errors::{AppError, AppResult};
use crate::models::channel_message::{COLLECTION_NAME, ChannelMessage};
use crate::models::coordination::EVENT_DEDUP_COLLECTION_NAME;
use bson::{Document, doc};

pub(crate) async fn claim(
    db: &mongodb::Database,
    message: &ChannelMessage,
) -> AppResult<EventDedupClaimResult> {
    let event = message.platform_message_id.as_deref().unwrap_or_default();
    let namespace = match message.platform.as_str() {
        "whatsapp" if super::channel_delivery_service::valid_message_id(event) => {
            "whatsapp-inbound"
        }
        "x" if !event.is_empty()
            && event.len() <= 32
            && event.bytes().all(|b| b.is_ascii_digit()) =>
        {
            "x-inbound"
        }
        _ => {
            return Err(AppError::ValidationError(
                "Invalid channel message identity".into(),
            ));
        }
    };
    let scope = serde_json::to_string(&(&message.channel_bot_id, &message.user_id))
        .map_err(|_| AppError::Internal("Unable to encode message admission".into()))?;
    EventDedupStore::claim(
        db,
        namespace,
        &scope,
        event,
        std::time::Duration::from_secs(30),
    )
    .await
}

pub(crate) async fn admit(
    db: &mongodb::Database,
    claim: &EventDedupClaim,
    message: &ChannelMessage,
) -> AppResult<bool> {
    let result: Result<bool, mongodb::error::Error> = async {
        let mut session = db.client().start_session().await?;
        session.start_transaction().and_run(
            (db.clone(), claim.clone(), message.clone()),
            |session, (db, claim, message)| {
                Box::pin(async move {
                    let fenced = db.collection::<Document>(EVENT_DEDUP_COLLECTION_NAME)
                        .update_one(
                            doc! {"_id":&claim.id, "claim_id":&claim.claim_id, "state":"claimed", "$expr":{"$gt":["$expires_at", "$$NOW"]}},
                            vec![doc! {"$set":{"state":"committed", "updated_at":"$$NOW", "expires_at":{"$dateAdd":{"startDate":"$$NOW", "unit":"day", "amount":30}}}}],
                        ).session(&mut *session).await?;
                    if fenced.matched_count != 1 { return Ok(false); }
                    let existing = db.collection::<ChannelMessage>(COLLECTION_NAME)
                        .find_one(doc! {
                            "channel_bot_id":&message.channel_bot_id, "user_id":&message.user_id,
                            "platform":&message.platform, "direction":"inbound", "platform_message_id":&message.platform_message_id,
                        }).session(&mut *session).await?;
                    if existing.is_some() { return Ok(false); }
                    db.collection::<ChannelMessage>(COLLECTION_NAME)
                        .insert_one(&*message).session(&mut *session).await?;
                    Ok(true)
                })
            },
        ).await
    }.await;
    if result.is_err() {
        // This predicate can only delete an uncommitted claim. Metadata and the
        // committed state are atomic, including an uncertain transaction result.
        let _ = EventDedupStore::release(db, claim).await;
    }
    Ok(result?)
}

pub(crate) async fn store(
    db: &mongodb::Database,
    message: ChannelMessage,
) -> AppResult<Option<ChannelMessage>> {
    match claim(db, &message).await? {
        EventDedupClaimResult::Duplicate => Ok(None),
        EventDedupClaimResult::Claimed(claim) => {
            Ok(admit(db, &claim, &message).await?.then_some(message))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(event: &str) -> ChannelMessage {
        super::super::channel_relay_service::inbound_metadata(
            "bot",
            "conversation",
            "owner",
            "whatsapp",
            &super::super::channel_platform::InboundMessage {
                platform_message_id: event.into(),
                conversation_id: "789".into(),
                conversation_type: "private".into(),
                sender_platform_id: "789".into(),
                sender_display_name: None,
                content_type: "text".into(),
                text: Some("private callback content".into()),
                attachments: vec![],
                reply_to_platform_message_id: None,
                thread_id: None,
                raw_data: serde_json::json!({"text":"private raw content"}),
            },
            "agent",
            &uuid::Uuid::new_v4().to_string(),
        )
    }
    async fn acquire(db: &mongodb::Database, message: &ChannelMessage) -> EventDedupClaim {
        match claim(db, message).await.unwrap() {
            EventDedupClaimResult::Claimed(claim) => claim,
            EventDedupClaimResult::Duplicate => panic!("expected new claim"),
        }
    }

    #[tokio::test]
    async fn expired_and_replaced_claim_holders_cannot_admit() {
        let db =
            crate::test_utils::connect_transaction_test_database("channel_admission_fence").await;
        let message = message("wamid.same");
        let old = acquire(&db, &message).await;
        db.collection::<Document>(EVENT_DEDUP_COLLECTION_NAME)
            .update_one(
                doc! {"_id":&old.id},
                doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        assert!(!admit(&db, &old, &message).await.unwrap());
        let new = acquire(&db, &message).await;
        assert!(!admit(&db, &old, &message).await.unwrap());
        assert!(!EventDedupStore::release(&db, &old).await.unwrap());
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert!(admit(&db, &new, &message).await.unwrap());
        assert!(!EventDedupStore::release(&db, &new).await.unwrap());
        assert!(store(&db, message.clone()).await.unwrap().is_none());
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn pre_admission_insert_failure_releases_only_uncommitted_claim() {
        let db =
            crate::test_utils::connect_transaction_test_database("channel_admission_failure").await;
        let collision = message("wamid.existing");
        db.collection::<ChannelMessage>(COLLECTION_NAME)
            .insert_one(&collision)
            .await
            .unwrap();
        let mut incoming = message("wamid.new");
        incoming.id = collision.id.clone();
        assert!(store(&db, incoming.clone()).await.is_err());
        assert_eq!(
            db.collection::<Document>(EVENT_DEDUP_COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        incoming.id = uuid::Uuid::new_v4().to_string();
        assert!(store(&db, incoming).await.unwrap().is_some());
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            2
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn legacy_metadata_is_rechecked_under_claim_and_tenant_scoped() {
        let db =
            crate::test_utils::connect_transaction_test_database("channel_admission_legacy").await;
        let incoming = message("wamid.legacy");
        let claim = acquire(&db, &incoming).await;
        db.collection::<ChannelMessage>(COLLECTION_NAME)
            .insert_one(&incoming)
            .await
            .unwrap();
        let mut retry = incoming.clone();
        retry.id = uuid::Uuid::new_v4().to_string();
        assert!(!admit(&db, &claim, &retry).await.unwrap());
        assert!(!EventDedupStore::release(&db, &claim).await.unwrap());
        retry.user_id = "another-owner".into();
        assert!(store(&db, retry.clone()).await.unwrap().is_some());
        retry.id = uuid::Uuid::new_v4().to_string();
        retry.channel_bot_id = Some("another-bot".into());
        assert!(store(&db, retry).await.unwrap().is_some());
        assert_eq!(
            db.collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            3
        );
        let raw = db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":&incoming.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!raw.to_string().contains("private"));
        db.drop().await.unwrap();
    }
}
