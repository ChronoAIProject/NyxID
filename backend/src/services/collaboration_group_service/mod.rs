//! Multi-person groups with explicit markers and requester-bound authority.
//! All new writers require the shared-agents flag and full-fleet Phase 2 readiness.

pub mod cleanup;
pub mod identity;
pub mod membership;
pub mod requests;
pub mod runtime;
#[cfg(test)]
mod tests;
pub mod uploads;

use identity::AgentIdentity;

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};

use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_group::{AssistantGroup, COLLECTION_NAME as GROUPS},
        assistant_group_collaboration as model,
        user::{COLLECTION_NAME as USERS, User},
    },
};

pub const READINESS: &str = "assistant:collaboration-groups-v1";
pub const HISTORY_DISCLOSURE: &str = "New participants can read the full retained group history.";
pub const REFUSAL: &str = "Ask this agent's owner to share it with you.";

fn missing() -> AppError {
    AppError::NotFound("Group not found".into())
}
fn stale() -> AppError {
    AppError::Conflict("Group membership changed; reload and retry".into())
}
fn invalid(message: &str) -> AppError {
    AppError::ValidationError(message.into())
}

/// All new writers require a full-fleet Phase 2 capability, not merely the
/// already-enabled Phase 1 flag. Old replicas never admit a new marker.
async fn require_rollout(db: &Database) -> AppResult<()> {
    if !super::security_readiness_service::report(db, READINESS)
        .await?
        .ready
    {
        return Err(AppError::SecurityReadinessUnavailable);
    }
    Ok(())
}

fn marker(group: &AssistantGroup) -> AppResult<&model::Collaboration> {
    let marker = group.collaboration.as_ref().ok_or_else(missing)?;
    if marker.version != model::VERSION
        || marker.membership_revision < 1
        || marker.creator_user_id != group.user_id
        || group.created_by_user_id.is_some()
        || group.participant_user_ids.is_empty()
        || group.participant_user_ids.len() > 16
        || !group.participant_user_ids.contains(&marker.creator_user_id)
        || group.member_agent_ids.is_empty()
        || group.member_agent_ids.len() > 8
        || !group.member_agent_ids.contains(&group.lead_agent_id)
        || !unique(&group.participant_user_ids)
        || !unique(&group.member_agent_ids)
    {
        return Err(missing());
    }
    Ok(marker)
}

fn unique(ids: &[String]) -> bool {
    ids.iter().collect::<std::collections::HashSet<_>>().len() == ids.len()
}

async fn rows<T: serde::de::DeserializeOwned + Send + Sync + Unpin>(
    db: &Database,
    collection: &str,
    filter: bson::Document,
    session: &mut ClientSession,
) -> AppResult<Vec<T>> {
    let mut cursor = db
        .collection::<T>(collection)
        .find(filter)
        .limit(1025)
        .session(&mut *session)
        .await?;
    let rows: Vec<T> = cursor.stream(session).try_collect().await?;
    if rows.len() > 1024 {
        return Err(invalid("Too many authority records for one group request"));
    }
    Ok(rows)
}

async fn person(db: &Database, id: &str, session: &mut ClientSession) -> AppResult<User> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id":id,"is_active":true})
        .session(&mut *session)
        .await?
        .ok_or_else(missing)?;
    if !user.user_type.is_person() {
        return Err(missing());
    }
    Ok(user)
}

async fn fence_person(db: &Database, id: &str, session: &mut ClientSession) -> AppResult<()> {
    super::erasure_admission::fence_in_session(db, id, session).await?;
    person(db, id, session).await?;
    if db
        .collection::<User>(USERS)
        .update_one(
            doc! {"_id":id,"is_active":true},
            doc! {"$inc":{"collaboration_fence":1}},
        )
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(missing());
    }
    Ok(())
}

async fn read_group(
    db: &Database,
    actor: &str,
    id: &str,
    session: &mut ClientSession,
) -> AppResult<AssistantGroup> {
    let group = db
        .collection::<AssistantGroup>(GROUPS)
        .find_one(doc! {"_id":id,"participant_user_ids":actor,"collaboration.version":1})
        .session(&mut *session)
        .await?
        .ok_or_else(missing)?;
    marker(&group)?;
    person(db, actor, session).await?;
    Ok(group)
}

async fn fence_group(
    db: &Database,
    group: &AssistantGroup,
    session: &mut ClientSession,
) -> AppResult<()> {
    let revision = marker(group)?.membership_revision;
    if db.collection::<AssistantGroup>(GROUPS).update_one(
        doc! {"_id":&group.id,"collaboration.version":1,"collaboration.membership_revision":revision},
        doc! {"$inc":{"collaboration_fence":1}}).session(session).await?.matched_count != 1 {
        return Err(stale());
    }
    Ok(())
}

pub async fn get(db: &Database, actor: &str, id: &str) -> AppResult<AssistantGroup> {
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .await?;
    let group = read_group(db, actor, id, &mut session).await?;
    session.commit_transaction().await?;
    Ok(group)
}

pub async fn messages(
    db: &Database,
    actor: &str,
    id: &str,
    before: Option<i64>,
) -> AppResult<Vec<crate::models::assistant_group::GroupMessage>> {
    // Same snapshot for participation and transcript. No requester/owner filter:
    // participation deliberately grants all retained group messages.
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .await?;
    read_group(db, actor, id, &mut session).await?;
    let mut filter = doc! {"group_id":id,"collaboration.version":1};
    if let Some(before) = before {
        filter.insert("seq", doc! {"$lt":before});
    }
    let mut cursor = db
        .collection::<crate::models::assistant_group::GroupMessage>(
            crate::models::assistant_group::MESSAGES_COLLECTION_NAME,
        )
        .find(filter)
        .sort(doc! {"seq":-1})
        .limit(100)
        .session(&mut session)
        .await?;
    let mut rows: Vec<_> = cursor.stream(&mut session).try_collect().await?;
    rows.reverse();
    session.commit_transaction().await?;
    Ok(rows)
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    for (collection, keys, unique) in [
        (model::INVITES, doc! {"token_hash":1}, true),
        (
            model::INVITES,
            doc! {"group_id":1,"cancelled_at":1,"accepted_at":1},
            false,
        ),
        (model::AGENT_ACCESS, doc! {"group_id":1,"agent_id":1}, true),
        (
            model::PROPOSALS,
            doc! {"group_id":1,"membership_revision":1},
            false,
        ),
        (
            crate::models::assistant_group::REQUESTS_COLLECTION_NAME,
            doc! {"collaboration.version":1,"actor_user_id":1,"created_at":1},
            false,
        ),
        (
            crate::models::assistant_conversation::COLLECTION_NAME,
            doc! {"collaboration.version":1,"collaboration_checked_at":1},
            false,
        ),
        (
            crate::models::assistant_conversation::COLLECTION_NAME,
            doc! {"collaboration.group_id":1},
            false,
        ),
        (
            model::SLOTS,
            doc! {"group_id":1,"agent_id":1,"original_requester_id":1,"lane":1},
            true,
        ),
        (
            model::SLOTS,
            doc! {"original_requester_id":1,"revoked":1},
            false,
        ),
        (
            GROUPS,
            doc! {"participant_user_ids":1,"collaboration.version":1,"updated_at":-1},
            false,
        ),
    ] {
        db.collection::<bson::Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(IndexOptions::builder().unique(unique).build())
                    .build(),
            )
            .await?;
    }
    for collection in [model::INVITES, model::TRANSFERS, model::PROPOSALS] {
        db.collection::<bson::Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"expires_at":1})
                    .options(
                        IndexOptions::builder()
                            .expire_after(std::time::Duration::ZERO)
                            .build(),
                    )
                    .build(),
            )
            .await?;
    }
    Ok(())
}

/// Revision changes revoke request bindings immediately. Runtime cancellation,
/// card invalidation and key revocation commit before background artifact cleanup.
async fn invalidate(db: &Database, group: &str, session: &mut ClientSession) -> AppResult<()> {
    use crate::models::{
        assistant_acknowledgement as acks, assistant_conversation as chats,
        assistant_sharing as shares,
    };
    let mut cursor = db
        .collection::<model::ExecutionSlot>(model::SLOTS)
        .find(doc! {"group_id":group})
        .session(&mut *session)
        .await?;
    let slots: Vec<_> = cursor.stream(&mut *session).try_collect().await?;
    let ids: Vec<_> = slots
        .iter()
        .filter_map(|slot| slot.conversation_id.as_deref())
        .collect();
    db.collection::<bson::Document>(model::SLOTS)
        .update_many(doc! {"group_id":group}, doc! {"$set":{"revoked":true}})
        .session(&mut *session)
        .await?;
    db.collection::<bson::Document>(crate::models::assistant_group::REQUESTS_COLLECTION_NAME)
        .update_many(
            doc! {"group_id":group,"collaboration":{"$ne":null}},
            doc! {"$set":{"pending_agent_ids":[],"dropped":true}},
        )
        .session(&mut *session)
        .await?;
    if ids.is_empty() {
        return Ok(());
    }
    let mut cursor = db
        .collection::<bson::Document>(chats::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":&ids}})
        .projection(doc! {"credential_api_key_id":1})
        .session(&mut *session)
        .await?;
    let chats: Vec<_> = cursor.stream(&mut *session).try_collect().await?;
    let keys: Vec<_> = chats
        .iter()
        .filter_map(|chat| chat.get_str("credential_api_key_id").ok())
        .collect();
    super::api_key_mutation_service::update_many(
        db,
        doc! {"_id":{"$in":keys}},
        doc! {"$set":{"is_active":false}},
        Some(&mut *session),
    )
    .await?;
    db.collection::<bson::Document>(chats::COLLECTION_NAME)
        .update_many(
            doc! {"_id":{"$in":&ids}},
            doc! {"$set":{"pending_events":[],"nyxagent_session_id":null}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<bson::Document>(chats::COLLECTION_NAME)
        .update_many(
            doc! {"_id":{"$in":&ids},"active_turn":{"$ne":null}},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<bson::Document>(acks::COLLECTION_NAME)
        .update_many(
            doc! {"conversation_id":{"$in":&ids},"status":{"$in":["pending","allowed"]}},
            doc! {"$set":{"status":"expired"}},
        )
        .session(&mut *session)
        .await?;
    db.collection::<bson::Document>(crate::models::approval_request::COLLECTION_NAME)
        .update_many(doc! {"assistant_group.conversation_id":{"$in":&ids},"status":{"$in":["pending","approved"]}},doc! {"$set":{"status":"expired"}}).session(&mut *session).await?;
    db.collection::<bson::Document>(shares::CONSENTS_COLLECTION_NAME)
        .update_many(
            doc! {"conversation_id":{"$in":&ids}},
            doc! {"$set":{"decision":"revoked","updated_at":bson::DateTime::now()}},
        )
        .session(&mut *session)
        .await?;
    Ok(())
}

async fn audit(db: &Database, actor: &str, group: &str, change: &str) {
    let _ = super::audit_service::log_actor_event(
        db.clone(),
        &super::audit_service::AuditActor {
            user_id: actor.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        "assistant_collaboration_group_changed",
        Some(serde_json::json!({"actor_id":actor,"group_id":group,"change":change})),
    )
    .await;
}

/// Select only marked rows. No access or private metadata is returned by the
/// dispatch hint; the selected service must still authorize the participant.
pub async fn marked(db: &Database, id: &str) -> AppResult<bool> {
    Ok(db
        .collection::<bson::Document>(GROUPS)
        .find_one(doc! {"_id":id,"collaboration":{"$ne":null}})
        .projection(doc! {"_id":1})
        .await?
        .is_some())
}

pub async fn require_available(db: &Database, actor: &str) -> AppResult<()> {
    if !super::feature_flag_service::personal_flag_enabled(
        db,
        actor,
        super::shared_agent_service::FLAG,
    )
    .await?
    {
        return Err(missing());
    }
    require_rollout(db).await
}

pub async fn list(db: &Database, actor: &str) -> AppResult<Vec<AssistantGroup>> {
    // Retained groups remain discoverable for history, leave and deletion
    // during a flag/readiness rollback. New writers gate independently.
    Ok(db
        .collection::<AssistantGroup>(GROUPS)
        .find(doc! {"participant_user_ids":actor,"collaboration.version":1})
        .sort(doc! {"updated_at":-1})
        .limit(100)
        .await?
        .try_collect()
        .await?)
}

/// Safe roster projection is performed by the handler. Never serialize these
/// agent models: they include private fields even for participant-owned agents.
pub async fn roster(
    db: &Database,
    actor: &str,
    id: &str,
) -> AppResult<Vec<crate::models::assistant_agent::AssistantAgent>> {
    let group = get(db, actor, id).await?;
    Ok(db
        .collection(crate::models::assistant_agent::COLLECTION_NAME)
        .find(doc! {"_id":{"$in":group.member_agent_ids}})
        .limit(8)
        .await?
        .try_collect()
        .await?)
}

pub async fn participants(
    db: &Database,
    group: &AssistantGroup,
) -> AppResult<Vec<super::org_group_service::Participant>> {
    let users: Vec<User> = db
        .collection(USERS)
        .find(doc! {"_id":{"$in":&group.participant_user_ids}})
        .limit(16)
        .await?
        .try_collect()
        .await?;
    Ok(group
        .participant_user_ids
        .iter()
        .map(|id| super::org_group_service::Participant {
            id: id.clone(),
            display_name: users
                .iter()
                .find(|u| u.id == *id)
                .and_then(|u| u.display_name.clone())
                .unwrap_or_else(|| "Member".into()),
        })
        .collect())
}

pub async fn threads(
    db: &Database,
    group: &str,
) -> AppResult<Vec<crate::models::assistant_conversation::AssistantConversation>> {
    Ok(db
        .collection(crate::models::assistant_conversation::COLLECTION_NAME)
        .find(doc! {"collaboration.group_id":group,"collaboration.version":1})
        .limit(1024)
        .await?
        .try_collect()
        .await?)
}
