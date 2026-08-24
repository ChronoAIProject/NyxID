use chrono::{Duration, Utc};
use mongodb::Database;
use mongodb::bson::{DateTime as BsonDateTime, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::assistant_wire_log::AssistantWireLog;

pub const WIRE_LOG_TTL_SECS: i64 = 15 * 60;
pub const WIRE_LOG_MAX_PAYLOAD_BYTES: usize = 1024 * 1024;

pub async fn store(
    db: &Database,
    user_id: &str,
    conversation_id: Option<&str>,
    payload_json: String,
) -> AppResult<String> {
    if payload_json.len() > WIRE_LOG_MAX_PAYLOAD_BYTES {
        return Err(AppError::Internal(
            "assistant wire-log payload exceeds the storage limit".to_string(),
        ));
    }

    let created_at = Utc::now();
    let wire_log = AssistantWireLog {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        conversation_id: conversation_id.map(str::to_string),
        payload: payload_json,
        created_at,
        expires_at: created_at + Duration::seconds(WIRE_LOG_TTL_SECS),
    };
    let id = wire_log.id.clone();

    db.collection::<AssistantWireLog>(AssistantWireLog::COLLECTION_NAME)
        .insert_one(&wire_log)
        .await
        .map_err(AppError::from)?;

    Ok(id)
}

pub async fn fetch_for_user(
    db: &Database,
    user_id: &str,
    id: &str,
) -> AppResult<Option<AssistantWireLog>> {
    let now = BsonDateTime::from_millis(Utc::now().timestamp_millis());
    db.collection::<AssistantWireLog>(AssistantWireLog::COLLECTION_NAME)
        .find_one(doc! {
            "_id": id,
            "user_id": user_id,
            "expires_at": { "$gt": now },
        })
        .await
        .map_err(AppError::from)
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    use super::*;

    #[tokio::test]
    async fn stored_wire_log_is_owner_scoped_and_expiry_filtered() {
        let Some(db) = crate::test_utils::connect_test_database("assistant_wire_log").await else {
            eprintln!("skipping assistant wire-log service test: no local MongoDB available");
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        let other_user_id = Uuid::new_v4().to_string();
        let conversation_id = "nyxchat-test";
        let payload = r#"{"version":2,"echoes":[],"droppedEchoCount":0}"#.to_string();

        let id = store(&db, &owner_id, Some(conversation_id), payload.clone())
            .await
            .expect("store wire log");

        let stored = fetch_for_user(&db, &owner_id, &id)
            .await
            .expect("fetch owner wire log")
            .expect("owner wire log exists");
        assert_eq!(stored.user_id, owner_id);
        assert_eq!(stored.conversation_id.as_deref(), Some(conversation_id));
        assert_eq!(stored.payload, payload);
        assert!(
            fetch_for_user(&db, &other_user_id, &id)
                .await
                .expect("fetch other user's wire log")
                .is_none()
        );

        let expired = AssistantWireLog {
            id: Uuid::new_v4().to_string(),
            user_id: owner_id.clone(),
            conversation_id: None,
            payload: r#"{"version":2,"echoes":[],"droppedEchoCount":0}"#.to_string(),
            created_at: Utc::now() - Duration::seconds(2),
            expires_at: Utc::now() - Duration::seconds(1),
        };
        db.collection::<AssistantWireLog>(AssistantWireLog::COLLECTION_NAME)
            .insert_one(&expired)
            .await
            .expect("insert expired wire log");
        assert!(
            fetch_for_user(&db, &owner_id, &expired.id)
                .await
                .expect("fetch expired wire log")
                .is_none()
        );

        let oversized = store(
            &db,
            &owner_id,
            None,
            "x".repeat(WIRE_LOG_MAX_PAYLOAD_BYTES + 1),
        )
        .await;
        assert!(matches!(oversized, Err(AppError::Internal(_))));
    }
}
