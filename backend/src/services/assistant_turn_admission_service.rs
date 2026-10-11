use chrono::Utc;
use mongodb::{ClientSession, Database, bson::doc};

use crate::{
    errors::{AppError, AppResult},
    models::assistant_turn_admission::{AssistantTurnAdmission, COLLECTION_NAME},
};

use super::assistant_nyxagent::TurnRequest;

#[derive(Clone, Debug)]
pub struct AdmissionRequest {
    pub client_request_id: String,
    pub payload_fingerprint: String,
    #[cfg(test)]
    pub test_hook: Option<AdmissionTestHook>,
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub struct AdmissionTestHook {
    pub after_missing: std::sync::Arc<tokio::sync::Notify>,
    pub resume: std::sync::Arc<tokio::sync::Notify>,
    pub armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

#[cfg(test)]
pub async fn pause_after_missing(request: &AdmissionRequest) {
    use std::sync::atomic::Ordering;

    let Some(hook) = request.test_hook.as_ref() else {
        return;
    };
    if hook.armed.swap(false, Ordering::SeqCst) {
        hook.after_missing.notify_one();
        hook.resume.notified().await;
    }
}

pub enum ReplayLookup {
    Missing,
    Exact(AssistantTurnAdmission),
}

pub enum RecoveryLookup {
    Admitted(AssistantTurnAdmission),
    ExplicitlyAbsent,
}

pub fn parse_client_request_id(value: &str) -> AppResult<String> {
    let parsed = uuid::Uuid::parse_str(value)
        .map_err(|_| AppError::BadRequest("Idempotency-Key must be a UUID v4".into()))?;
    if parsed.get_version() != Some(uuid::Version::Random)
        || parsed.hyphenated().to_string() != value
    {
        return Err(AppError::BadRequest(
            "Idempotency-Key must be a canonical UUID v4".into(),
        ));
    }
    Ok(value.to_owned())
}

pub fn payload_fingerprint(request: &TurnRequest) -> String {
    super::mcp_service::canonical_sha256(serde_json::json!({
        "attachment_ids": request.attachment_ids,
        "conversation_id": request.conversation_id,
        "agent_id": request.agent_id,
        "text": request.text,
        "model": request.model,
    }))
}

fn id(user_id: &str, client_request_id: &str) -> String {
    format!("{user_id}:{client_request_id}")
}

fn conflict() -> AppError {
    AppError::Conflict("Idempotency-Key was already used for another assistant turn".into())
}

pub fn verify(
    row: AssistantTurnAdmission,
    payload_fingerprint: &str,
) -> AppResult<AssistantTurnAdmission> {
    if !row.not_admitted && row.payload_fingerprint == payload_fingerprint {
        Ok(row)
    } else {
        Err(conflict())
    }
}

pub async fn lookup(
    db: &Database,
    user_id: &str,
    request: &AdmissionRequest,
) -> AppResult<ReplayLookup> {
    match get(db, user_id, &request.client_request_id).await? {
        Some(row) => Ok(ReplayLookup::Exact(verify(
            row,
            &request.payload_fingerprint,
        )?)),
        None => Ok(ReplayLookup::Missing),
    }
}

pub async fn get(
    db: &Database,
    user_id: &str,
    client_request_id: &str,
) -> AppResult<Option<AssistantTurnAdmission>> {
    Ok(db
        .collection::<AssistantTurnAdmission>(COLLECTION_NAME)
        .find_one(doc! {
            "_id": id(user_id, client_request_id),
            "user_id": user_id,
            "client_request_id": client_request_id,
        })
        .await?)
}

pub async fn get_in_session(
    db: &Database,
    user_id: &str,
    client_request_id: &str,
    session: &mut ClientSession,
) -> AppResult<Option<AssistantTurnAdmission>> {
    Ok(db
        .collection::<AssistantTurnAdmission>(COLLECTION_NAME)
        .find_one(doc! {
            "_id": id(user_id, client_request_id),
            "user_id": user_id,
            "client_request_id": client_request_id,
        })
        .session(session)
        .await?)
}

/// Resolve recovery exactly once. An absent key is durably fenced in the same
/// unique namespace as positive admissions, so a concurrent or delayed POST
/// must either have committed first or roll its whole transaction back.
pub async fn get_or_fence_absent(
    db: &Database,
    user_id: &str,
    client_request_id: &str,
) -> AppResult<RecoveryLookup> {
    let fence = AssistantTurnAdmission {
        id: id(user_id, client_request_id),
        user_id: user_id.to_owned(),
        client_request_id: client_request_id.to_owned(),
        not_admitted: true,
        payload_fingerprint: String::new(),
        conversation_id: String::new(),
        turn_id: String::new(),
        conversation_deleted: false,
        conversation_deleted_at: None,
        created_at: Utc::now(),
    };
    match db
        .collection::<AssistantTurnAdmission>(COLLECTION_NAME)
        .insert_one(&fence)
        .await
    {
        Ok(_) => Ok(RecoveryLookup::ExplicitlyAbsent),
        Err(error) if is_duplicate_key(&error) => {
            let existing = get(db, user_id, client_request_id).await?.ok_or_else(|| {
                AppError::Internal("Assistant turn admission race could not be resolved".into())
            })?;
            if existing.not_admitted {
                Ok(RecoveryLookup::ExplicitlyAbsent)
            } else {
                Ok(RecoveryLookup::Admitted(existing))
            }
        }
        Err(error) => Err(error.into()),
    }
}

pub async fn insert_in_session(
    db: &Database,
    user_id: &str,
    request: &AdmissionRequest,
    conversation_id: &str,
    turn_id: &str,
    session: &mut ClientSession,
) -> AppResult<AssistantTurnAdmission> {
    let row = AssistantTurnAdmission {
        id: id(user_id, &request.client_request_id),
        user_id: user_id.to_owned(),
        client_request_id: request.client_request_id.clone(),
        not_admitted: false,
        payload_fingerprint: request.payload_fingerprint.clone(),
        conversation_id: conversation_id.to_owned(),
        turn_id: turn_id.to_owned(),
        conversation_deleted: false,
        conversation_deleted_at: None,
        created_at: Utc::now(),
    };
    db.collection::<AssistantTurnAdmission>(COLLECTION_NAME)
        .insert_one(&row)
        .session(session)
        .await?;
    Ok(row)
}

pub fn is_duplicate_key(error: &mongodb::error::Error) -> bool {
    match error.kind.as_ref() {
        mongodb::error::ErrorKind::Command(command) => command.code == 11000,
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error)) => {
            write_error.code == 11000
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::assistant_conversation::AccessMode;

    #[test]
    fn client_request_id_requires_canonical_uuid_v4() {
        let valid = uuid::Uuid::new_v4().to_string();
        assert_eq!(parse_client_request_id(&valid).unwrap(), valid);
        assert!(parse_client_request_id(&valid.to_uppercase()).is_err());
        assert!(parse_client_request_id("00000000-0000-1000-8000-000000000000").is_err());
        assert!(parse_client_request_id("not-a-uuid").is_err());
    }

    #[test]
    fn fingerprint_ignores_retired_access_mode_and_binds_execution_fields() {
        let base = TurnRequest {
            attachment_ids: vec!["attachment".into()],
            conversation_id: Some("nyxa-0123456789abcdef0123456789abcdef".into()),
            agent_id: None,
            text: "hello".into(),
            model: Some("nyxagent/chat".into()),
            access_mode: Some(AccessMode::Ask),
        };
        let mut retired_changed = base.clone();
        retired_changed.access_mode = Some(AccessMode::Full);
        assert_eq!(
            payload_fingerprint(&base),
            payload_fingerprint(&retired_changed)
        );
        let mut text_changed = base.clone();
        text_changed.text = "different".into();
        assert_ne!(
            payload_fingerprint(&base),
            payload_fingerprint(&text_changed)
        );
    }

    #[test]
    fn negative_recovery_fence_can_never_replay_as_an_admitted_turn() {
        let row = AssistantTurnAdmission {
            id: "owner:key".into(),
            user_id: "owner".into(),
            client_request_id: uuid::Uuid::new_v4().to_string(),
            not_admitted: true,
            payload_fingerprint: "fingerprint".into(),
            conversation_id: String::new(),
            turn_id: String::new(),
            conversation_deleted: false,
            conversation_deleted_at: None,
            created_at: Utc::now(),
        };
        assert!(matches!(
            verify(row, "fingerprint"),
            Err(AppError::Conflict(_))
        ));
    }

    #[test]
    fn duplicate_key_detection_accepts_write_and_command_errors() {
        let write: mongodb::error::WriteError = mongodb::bson::from_document(doc! {
            "code": 11000,
            "codeName": "DuplicateKey",
            "errmsg": "duplicate",
        })
        .unwrap();
        let write = mongodb::error::Error::from(mongodb::error::ErrorKind::Write(
            mongodb::error::WriteFailure::WriteError(write),
        ));
        assert!(is_duplicate_key(&write));

        let command: mongodb::error::CommandError = mongodb::bson::from_document(doc! {
            "code": 11000,
            "codeName": "DuplicateKey",
            "errmsg": "duplicate",
        })
        .unwrap();
        let command = mongodb::error::Error::from(mongodb::error::ErrorKind::Command(command));
        assert!(is_duplicate_key(&command));

        let other: mongodb::error::CommandError = mongodb::bson::from_document(doc! {
            "code": 42,
            "codeName": "Other",
            "errmsg": "other",
        })
        .unwrap();
        let other = mongodb::error::Error::from(mongodb::error::ErrorKind::Command(other));
        assert!(!is_duplicate_key(&other));
    }
}
