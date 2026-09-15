//! Proactive-send coordination. Stores delivery identity and receipts, never content.

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use sha2::{Digest, Sha256};

use crate::errors::{AppError, AppResult};
use crate::models::channel_conversation::{COLLECTION_NAME as CONVERSATIONS, ChannelConversation};
use crate::models::channel_send_claim::{COLLECTION_NAME, ChannelSendClaim};

pub enum ClaimResult {
    Claimed(ChannelSendClaim),
    Sent {
        message_id: String,
        platform_message_id: Option<String>,
    },
}

/// Canonicalize objects recursively so key order does not alter delivery identity.
pub fn delivery_fingerprint(text: Option<&str>, metadata: Option<&serde_json::Value>) -> String {
    fn canonical(value: serde_json::Value) -> serde_json::Value {
        match value {
            serde_json::Value::Object(object) => {
                let sorted: std::collections::BTreeMap<_, _> = object.into_iter().collect();
                serde_json::Value::Object(
                    sorted.into_iter().map(|(k, v)| (k, canonical(v))).collect(),
                )
            }
            serde_json::Value::Array(values) => {
                serde_json::Value::Array(values.into_iter().map(canonical).collect())
            }
            other => other,
        }
    }
    let payload = canonical(serde_json::json!({ "text": text, "metadata": metadata }));
    hex::encode(Sha256::digest(payload.to_string().as_bytes()))
}

pub async fn claim_send(
    db: &mongodb::Database,
    conversation_id: &str,
    idempotency_key: &str,
    fingerprint: &str,
) -> AppResult<ClaimResult> {
    let claim = ChannelSendClaim {
        id: format!("{conversation_id}:{idempotency_key}"),
        attempt_id: uuid::Uuid::new_v4().to_string(),
        fingerprint: fingerprint.to_string(),
        status: "pending".to_string(),
        message_id: None,
        platform_message_id: None,
        expires_at: Utc::now() + chrono::Duration::hours(24),
    };
    let collection = db.collection::<ChannelSendClaim>(COLLECTION_NAME);
    match collection.insert_one(&claim).await {
        Ok(_) => Ok(ClaimResult::Claimed(claim)),
        Err(error) => {
            if !matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(
                mongodb::error::WriteFailure::WriteError(e)) if e.code == 11000)
            {
                return Err(error.into());
            }
            let existing = collection
                .find_one(doc! { "_id": &claim.id })
                .await?
                .ok_or_else(|| {
                    AppError::Conflict("Send claim changed; retry the request".to_string())
                })?;
            if existing.fingerprint != fingerprint {
                return Err(AppError::Conflict(
                    "Idempotency key was used for a different message".to_string(),
                ));
            }
            match (existing.status.as_str(), existing.message_id) {
                ("sent", Some(message_id)) => Ok(ClaimResult::Sent {
                    message_id,
                    platform_message_id: existing.platform_message_id,
                }),
                _ => Err(AppError::Conflict(
                    "Send in progress or delivery outcome uncertain".to_string(),
                )),
            }
        }
    }
}

pub async fn release_send(db: &mongodb::Database, claim: &ChannelSendClaim) -> AppResult<()> {
    db.collection::<ChannelSendClaim>(COLLECTION_NAME)
        .delete_one(doc! { "_id": &claim.id, "attempt_id": &claim.attempt_id, "status": "pending" })
        .await?;
    Ok(())
}

pub async fn complete_send(
    db: &mongodb::Database,
    claim: &ChannelSendClaim,
    message_id: &str,
    platform_message_id: Option<&str>,
) -> AppResult<()> {
    let result = db.collection::<ChannelSendClaim>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": &claim.id, "attempt_id": &claim.attempt_id, "status": "pending" },
            doc! { "$set": { "status": "sent", "message_id": message_id, "platform_message_id": platform_message_id } },
        ).await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict(
            "Send claim expired before receipt was recorded".to_string(),
        ));
    }
    Ok(())
}

pub fn is_addressable(conversation: &ChannelConversation) -> bool {
    conversation.platform != "device"
        && !conversation.platform_conversation_id.trim().is_empty()
        && conversation.platform_conversation_id != "*"
}

pub async fn list_agent_conversations(
    db: &mongodb::Database,
    api_key_id: &str,
    page: u64,
    per_page: u64,
) -> AppResult<(Vec<ChannelConversation>, u64)> {
    let collection = db.collection::<ChannelConversation>(CONVERSATIONS);
    let filter = doc! { "agent_api_key_id": api_key_id, "is_active": true };
    let total = collection.count_documents(filter.clone()).await?;
    let rows = collection
        .find(filter)
        .sort(doc! { "created_at": -1, "_id": 1 })
        .skip(page.saturating_sub(1).saturating_mul(per_page))
        .limit(per_page as i64)
        .await?
        .try_collect()
        .await?;
    Ok((rows, total))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn channel_send_indexes_and_attempt_fences() {
        let Some(db) = crate::test_utils::connect_test_database("channel_send_claim_indexes").await
        else {
            eprintln!("no local MongoDB available");
            return;
        };
        crate::db::ensure_indexes(&db).await.unwrap();
        let collection = db.collection::<ChannelSendClaim>(COLLECTION_NAME);
        let indexes: Vec<_> = collection
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(indexes.iter().any(|index| index.keys == doc! { "_id": 1 }));
        assert!(indexes.iter().any(|index| {
            index.keys == doc! { "expires_at": 1 }
                && index
                    .options
                    .as_ref()
                    .and_then(|options| options.expire_after)
                    == Some(std::time::Duration::ZERO)
        }));
        let ClaimResult::Claimed(first) = claim_send(&db, "conversation", "key", "fingerprint")
            .await
            .unwrap()
        else {
            panic!("expected new claim");
        };
        assert!((first.expires_at - Utc::now()).num_seconds() > 86_390);
        release_send(&db, &first).await.unwrap();
        let ClaimResult::Claimed(replacement) =
            claim_send(&db, "conversation", "key", "fingerprint")
                .await
                .unwrap()
        else {
            panic!("expected replacement claim");
        };
        // An old attempt cannot delete or complete a later attempt with the same key.
        release_send(&db, &first).await.unwrap();
        assert!(matches!(
            complete_send(&db, &first, "stale", None).await,
            Err(AppError::Conflict(_))
        ));
        complete_send(&db, &replacement, "accepted", None)
            .await
            .unwrap();
        assert!(
            matches!(claim_send(&db, "conversation", "key", "fingerprint").await.unwrap(),
            ClaimResult::Sent { message_id, platform_message_id: None } if message_id == "accepted")
        );
        db.drop().await.unwrap();
    }

    #[test]
    fn channel_delivery_fingerprint_is_canonical_and_payload_bound() {
        let first = serde_json::from_str(r#"{"z":[{"b":2,"a":1}],"a":true}"#).unwrap();
        let reordered = serde_json::from_str(r#"{"a":true,"z":[{"a":1,"b":2}]}"#).unwrap();
        assert_eq!(
            delivery_fingerprint(Some("hello"), Some(&first)),
            delivery_fingerprint(Some("hello"), Some(&reordered))
        );
        assert_ne!(
            delivery_fingerprint(Some("hello"), Some(&first)),
            delivery_fingerprint(Some("changed"), Some(&first))
        );
        assert_ne!(
            delivery_fingerprint(Some("hello"), Some(&first)),
            delivery_fingerprint(Some("hello"), None)
        );
    }
}
