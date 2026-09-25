use chrono::Utc;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};

use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_acknowledgement::COLLECTION_NAME as ACKS,
        assistant_agent_credential::COLLECTION_NAME as CREDENTIALS,
        assistant_conversation::{
            AccessMode, AssistantConversation, COLLECTION_NAME as CONVERSATIONS,
        },
    },
    mw::auth::ASSISTANT_ACCOUNT_SCOPE,
    services::api_key_mutation_service as mutations,
};

pub async fn apply_key_mode(
    db: &Database,
    user: &str,
    key: &str,
    mode: AccessMode,
    session: &mut ClientSession,
) -> AppResult<()> {
    let full = mode == AccessMode::Full;
    let scopes = if full {
        format!("proxy {ASSISTANT_ACCOUNT_SCOPE}")
    } else {
        "proxy".into()
    };
    let result = mutations::update_one(
        db,
        doc! {"_id": key, "user_id": user, "is_active": true},
        doc! {"$set": {"allow_all_services": full, "allow_all_nodes": true,
        "allow_auto_connected_services": true, "scopes": scopes}},
        Some(session),
    )
    .await?;
    if result.matched_count != 1 {
        return Err(AppError::NotFound("Conversation key not found".into()));
    }
    Ok(())
}

pub async fn change(
    db: &Database,
    user: &str,
    id: &str,
    mode: AccessMode,
) -> AppResult<(AccessMode, AssistantConversation)> {
    super::assistant_nyxagent::get(db, user, id).await?;
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let user = user.to_owned();
    let id = id.to_owned();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let operation = async {
                let collection = db.collection::<AssistantConversation>(CONVERSATIONS);
                let filter = doc! {"_id": &id, "user_id": &user};
                let mut row = collection
                    .find_one(filter.clone())
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| AppError::NotFound("Conversation not found".into()))?;
                if super::assistant_nyxagent::live_turn(&row, Utc::now()).is_some() {
                    return Err(AppError::AssistantTurnActive);
                }
                let credential = db
                    .collection::<bson::Document>(CREDENTIALS)
                    .find_one(doc! {"conversation_id": &id, "user_id": &user})
                    .projection(doc! {"api_key_id": 1})
                    .session(&mut *session)
                    .await?;
                if let Some(credential) = credential {
                    let key = credential.get_str("api_key_id").map_err(|_| {
                        AppError::Internal("Assistant credential unavailable".into())
                    })?;
                    apply_key_mode(&db, &user, key, mode, session).await?;
                }
                // Replaced/revoked keys have no credential row; the next begin_turn
                // provisions with this mode in its transaction.
                let old = row.access_mode;
                row.access_mode = mode;
                collection
                    .replace_one(filter, &row)
                    .session(&mut *session)
                    .await?;
                db.collection::<bson::Document>(ACKS)
                    .update_many(
                        doc! {"conversation_id": &id, "user_id": &user, "$or": [
                            {"status": "pending"}, {"status": "allowed", "kind": "action"},
                        ]},
                        doc! {"$set": {"status": "expired"}},
                    )
                    .session(&mut *session)
                    .await?;
                Ok((old, row))
            }
            .await;
            mutations::transaction_result(operation)
        })
        .await
        .map_err(mutations::map_transaction_error)
}

pub async fn audit_change(db: &Database, user: &str, id: &str, old: AccessMode, new: AccessMode) {
    if old == new {
        return;
    }
    let _ = super::audit_service::log_actor_event(
        db.clone(),
        &super::audit_service::AuditActor {
            user_id: user.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        "assistant_access_mode_changed",
        Some(serde_json::json!({"conversation_id": id, "old_mode": old, "new_mode": new})),
    )
    .await;
}
