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
pub fn delivery_fingerprint(
    text: Option<&str>,
    metadata: Option<&serde_json::Value>,
    attachments: &[super::channel_platform::OutboundAttachment],
    max_bytes: u64,
) -> AppResult<String> {
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
    let media = attachments.iter().map(|a| {
        use super::channel_platform::OutboundMediaSource;
        let source = match &a.source {
            OutboundMediaSource::Url { url } => serde_json::json!({"url": url}),
            OutboundMediaSource::Base64 { data } => serde_json::json!({"sha256": hex::encode(Sha256::digest(super::channel_media_service::decode_base64(data, max_bytes)?))}),
        };
        Ok(serde_json::json!({"kind": a.kind, "source": source, "filename": a.filename, "mime_type": a.mime_type, "caption": a.caption}))
    }).collect::<AppResult<Vec<_>>>()?;
    // Preserve fingerprints of pre-media, text-only claims during rolling upgrades.
    let mut payload = serde_json::json!({ "text": text, "metadata": metadata });
    if !media.is_empty() {
        payload["attachments"] = serde_json::json!(media);
    }
    let payload = canonical(payload);
    Ok(hex::encode(Sha256::digest(payload.to_string().as_bytes())))
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

pub fn is_concrete_platform_address(id: &str) -> bool {
    let id = id.trim();
    !id.is_empty() && id != "*"
}

pub fn is_addressable(conversation: &ChannelConversation) -> bool {
    conversation.platform != "device"
        && !(conversation.platform == "x"
            && super::channel_adapters::x::is_public_conversation(
                &conversation.platform_conversation_id,
            ))
        && is_concrete_platform_address(&conversation.platform_conversation_id)
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

    #[test]
    fn channel_media_fingerprint_binds_each_attachment_field_and_decoded_content() {
        use crate::services::channel_platform::{
            MediaKind, OutboundAttachment, OutboundMediaSource,
        };
        let attachment = OutboundAttachment {
            kind: MediaKind::File,
            source: OutboundMediaSource::Base64 {
                data: "aGVsbG8=".into(),
            },
            filename: Some("a.txt".into()),
            mime_type: Some("text/plain".into()),
            caption: Some("caption".into()),
        };
        let fingerprint = |a: &OutboundAttachment| {
            delivery_fingerprint(None, None, std::slice::from_ref(a), 100).unwrap()
        };
        let original = fingerprint(&attachment);
        for field in 0..6 {
            let mut changed = attachment.clone();
            match field {
                0 => changed.kind = MediaKind::Image,
                1 => {
                    changed.source = OutboundMediaSource::Base64 {
                        data: "d29ybGQ=".into(),
                    }
                }
                2 => {
                    changed.source = OutboundMediaSource::Url {
                        url: "https://public.example/a".into(),
                    }
                }
                3 => changed.filename = Some("b.txt".into()),
                4 => changed.caption = None,
                _ => changed.mime_type = None,
            }
            assert_ne!(original, fingerprint(&changed));
        }
        assert_eq!(original, fingerprint(&attachment));
        assert!(!original.contains("caption"));
    }

    #[test]
    fn concrete_platform_address_rejects_empty_and_wildcard_routes() {
        for id in ["", " \t\n", "*", "  *  "] {
            assert!(!is_concrete_platform_address(id), "{id:?}");
        }
        for id in ["chat_123", "-100123", "C123", "  chat_123  "] {
            assert!(is_concrete_platform_address(id), "{id:?}");
        }
    }

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
            delivery_fingerprint(Some("hello"), Some(&first), &[], 1024).unwrap(),
            delivery_fingerprint(Some("hello"), Some(&reordered), &[], 1024).unwrap()
        );
        assert_ne!(
            delivery_fingerprint(Some("hello"), Some(&first), &[], 1024).unwrap(),
            delivery_fingerprint(Some("changed"), Some(&first), &[], 1024).unwrap()
        );
        assert_ne!(
            delivery_fingerprint(Some("hello"), Some(&first), &[], 1024).unwrap(),
            delivery_fingerprint(Some("hello"), None, &[], 1024).unwrap()
        );
    }
}
