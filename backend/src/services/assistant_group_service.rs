//! Group chats (Grok-Bot style): the owner plus several of their agents in
//! one transcript. A user message goes to its picked recipients, else the
//! members it @mentions, else the sender's sticky last recipients, else the
//! lead; members hand work to each other by @mentioning them in replies,
//! bounded per user message. Every member speaks through its own hidden
//! member thread, so it keeps its own key, grants and memory. Starting turns
//! lives in `handlers::assistant_group`; this module owns the durable state.
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
    options::ReturnDocument,
};
use uuid::Uuid;

/// The owner's sticky recipient selection for later unaddressed messages.
pub async fn persist_last_recipients(
    db: &Database,
    owner: &str,
    group_id: &str,
    ids: &[String],
) -> AppResult<()> {
    db.collection::<AssistantGroup>(GROUPS)
        .update_one(
            doc! {"_id": group_id, "user_id": owner},
            doc! {"$set": {"last_recipient_ids": ids}},
        )
        .await?;
    Ok(())
}

/// An explicit recipient selection from the composer pills: deduplicated in
/// order, 1 to `MAX_MEMBERS` ids, each a live (not destroyed) member.
pub fn explicit_recipients(ids: &[String], live: &[AssistantAgent]) -> AppResult<Vec<String>> {
    let mut selected: Vec<String> = Vec::new();
    for id in ids {
        if !selected.contains(id) {
            selected.push(id.clone());
        }
    }
    if selected.is_empty()
        || selected.len() > MAX_MEMBERS
        || selected
            .iter()
            .any(|id| !live.iter().any(|member| member.id == *id))
    {
        return Err(AppError::ValidationError(format!(
            "Choose 1 to {MAX_MEMBERS} current group members as recipients"
        )));
    }
    Ok(selected)
}

/// The recipients an unaddressed message goes to, as the composer previews
/// them: the sticky selection's live members, else the lead while it is live.
pub fn default_recipients(sticky: &[String], live_ids: &[String], lead: &str) -> Vec<String> {
    let mut ids: Vec<String> = Vec::new();
    for id in sticky {
        if live_ids.contains(id) && !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    if ids.is_empty() && live_ids.iter().any(|id| id == lead) {
        ids.push(lead.to_owned());
    }
    ids
}

pub async fn pending_thread_actions(
    db: &Database,
    thread_ids: &[String],
) -> AppResult<Vec<crate::models::assistant_acknowledgement::AssistantAcknowledgement>> {
    use crate::models::assistant_acknowledgement::{AssistantAcknowledgement, COLLECTION_NAME};
    Ok(db.collection::<AssistantAcknowledgement>(COLLECTION_NAME).find(doc! {"conversation_id":{"$in":thread_ids},"kind":"action","status":"pending","decider":"user","expires_at":{"$gt":bson::DateTime::now()}}).await?.try_collect().await?)
}

pub async fn pending_group_approvals(
    db: &Database,
    group_id: &str,
) -> AppResult<Vec<crate::models::approval_request::ApprovalRequest>> {
    Ok(db.collection::<crate::models::approval_request::ApprovalRequest>(crate::models::approval_request::COLLECTION_NAME).find(doc! {"assistant_group.group_id":group_id,"status":"pending","expires_at":{"$gt":bson::DateTime::now()}}).limit(100).await?.try_collect().await?)
}

use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::AssistantAgent,
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        assistant_group::{
            AssistantGroup, COLLECTION_NAME as GROUPS, FOLLOWER_TTL_HOURS, GroupFollower,
            GroupMessage, MAX_FOLLOWERS, MAX_MEMBERS, MAX_NAME_CHARS,
            MESSAGES_COLLECTION_NAME as MESSAGES,
        },
    },
    services::{
        api_key_mutation_service as transactions,
        assistant_nyxagent::{self as engine, excerpt, identifier, live_turn},
        assistant_team_service as team,
    },
};
use mongodb::ClientSession;

/// Transcript budget handed to a member per turn.
const TRANSCRIPT_CHARS: usize = 12_000;
const TRANSCRIPT_MESSAGES: i64 = 60;

fn not_found() -> AppError {
    AppError::NotFound("Group not found".into())
}

pub fn valid_id(id: &str) -> bool {
    id.strip_prefix("nyxg-")
        .is_some_and(|rest| rest.len() == 32 && rest.bytes().all(|b| b.is_ascii_hexdigit()))
}

fn owner_filter(owner: &str, id: &str) -> AppResult<bson::Document> {
    if !valid_id(id) {
        return Err(not_found());
    }
    Ok(doc! {"_id": id, "user_id": owner})
}

/// Resolve and check members: live agents of this owner, unique, bounded.
async fn resolve_members(
    db: &Database,
    owner: &str,
    ids: &[String],
) -> AppResult<Vec<AssistantAgent>> {
    let mut members: Vec<AssistantAgent> = Vec::new();
    for id in ids {
        let id = id.trim();
        if members.iter().any(|member| member.id == id) {
            continue;
        }
        let agent = team::agent(db, owner, id)
            .await
            .map_err(|_| AppError::ValidationError("Unknown agent in member_agent_ids".into()))?;
        super::org_agent_service::require_use(db, owner, &agent).await?;
        if agent.destroyed_at.is_some() {
            return Err(AppError::Conflict(format!(
                "Agent {} was destroyed",
                identifier(&agent.name)
            )));
        }
        members.push(agent);
    }
    if members.is_empty() || members.len() > MAX_MEMBERS {
        return Err(AppError::ValidationError(format!(
            "A group has 1 to {MAX_MEMBERS} agents"
        )));
    }
    Ok(members)
}

fn lead_of(members: &[AssistantAgent]) -> String {
    members
        .iter()
        .find(|member| member.is_nyxbot())
        .unwrap_or(&members[0])
        .id
        .clone()
}

fn valid_name(name: &str) -> AppResult<String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(AppError::ValidationError(format!(
            "A group name has 1 to {MAX_NAME_CHARS} characters"
        )));
    }
    Ok(name.to_owned())
}

fn names(members: &[AssistantAgent]) -> String {
    members
        .iter()
        .map(|member| identifier(&member.name))
        .collect::<Vec<_>>()
        .join(", ")
}

pub async fn create(
    db: &Database,
    owner: &str,
    name: &str,
    member_ids: &[String],
    created_by: &'static str,
) -> AppResult<AssistantGroup> {
    let name = valid_name(name)?;
    let members = resolve_members(db, owner, member_ids).await?;
    let now = Utc::now();
    let group = AssistantGroup {
        id: format!("nyxg-{}", Uuid::new_v4().simple()),
        user_id: owner.into(),
        participant_user_ids: Vec::new(),
        created_by_user_id: None,
        name,
        member_agent_ids: members.iter().map(|member| member.id.clone()).collect(),
        lead_agent_id: lead_of(&members),
        last_recipient_ids: Vec::new(),
        last_recipient_ids_by_actor: std::collections::HashMap::new(),
        created_by: created_by.into(),
        message_count: 0,
        pending_agent_ids: Vec::new(),
        hops_remaining: handoff_budget(db, owner).await?,
        followers: Vec::new(),
        pending_checked_at: None,
        last_message_at: None,
        created_at: now,
        updated_at: now,
    };
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .read_concern(mongodb::options::ReadConcern::snapshot())
        .write_concern(mongodb::options::WriteConcern::majority())
        .await?;
    let result: AppResult<()> = async {
        super::erasure_admission::fence_in_session(db, owner, &mut session).await?;
        db.collection::<AssistantGroup>(GROUPS)
            .insert_one(&group)
            .session(&mut session)
            .await?;
        Ok(())
    }
    .await;
    super::org_membership_authority::finish(session, result).await?;
    append(
        db,
        owner,
        &group.id,
        "notice",
        None,
        &format!("Group created with {}.", names(&members)),
        Vec::new(),
    )
    .await?;
    get(db, owner, &group.id).await
}

pub async fn get(db: &Database, owner: &str, id: &str) -> AppResult<AssistantGroup> {
    db.collection::<AssistantGroup>(GROUPS)
        .find_one(owner_filter(owner, id)?)
        .await?
        .ok_or_else(not_found)
}

/// Find a group by ID or (case-insensitive) name, for agent tools.
pub async fn find(db: &Database, owner: &str, reference: &str) -> AppResult<AssistantGroup> {
    let reference = reference.trim();
    if valid_id(reference) {
        return get(db, owner, reference).await;
    }
    list(db, owner)
        .await?
        .into_iter()
        .find(|group| group.name.eq_ignore_ascii_case(reference))
        .ok_or_else(not_found)
}

pub async fn list(db: &Database, owner: &str) -> AppResult<Vec<AssistantGroup>> {
    Ok(db
        .collection::<AssistantGroup>(GROUPS)
        .find(doc! {"user_id": owner})
        .sort(doc! {"updated_at": -1})
        .limit(100)
        .await?
        .try_collect()
        .await?)
}

/// Members in group order, including destroyed agents (shown as such).
pub async fn members(
    db: &Database,
    owner: &str,
    group: &AssistantGroup,
) -> AppResult<Vec<AssistantAgent>> {
    let agents = team::agents(db, owner, true).await?;
    Ok(group
        .member_agent_ids
        .iter()
        .filter_map(|id| agents.iter().find(|agent| &agent.id == id).cloned())
        .collect())
}

pub async fn update(
    db: &Database,
    owner: &str,
    id: &str,
    name: Option<&str>,
    member_ids: Option<&[String]>,
) -> AppResult<AssistantGroup> {
    let group = get(db, owner, id).await?;
    let mut set = doc! {"updated_at": bson::DateTime::now()};
    if let Some(name) = name {
        set.insert("name", valid_name(name)?);
    }
    let mut notice = None;
    if let Some(member_ids) = member_ids {
        let members = resolve_members(db, owner, member_ids).await?;
        let ids: Vec<String> = members.iter().map(|member| member.id.clone()).collect();
        let before = self::members(db, owner, &group).await?;
        let joined: Vec<AssistantAgent> = members
            .iter()
            .filter(|member| !group.member_agent_ids.contains(&member.id))
            .cloned()
            .collect();
        let left: Vec<AssistantAgent> = before
            .into_iter()
            .filter(|member| !ids.contains(&member.id))
            .collect();
        let mut parts = Vec::new();
        if !joined.is_empty() {
            parts.push(format!("{} joined.", names(&joined)));
        }
        if !left.is_empty() {
            parts.push(format!("{} left.", names(&left)));
        }
        if !parts.is_empty() {
            notice = Some(parts.join(" "));
        }
        set.insert("lead_agent_id", lead_of(&members));
        set.insert("member_agent_ids", ids.clone());
        db.collection::<AssistantGroup>(GROUPS)
            .update_one(
                owner_filter(owner, id)?,
                doc! {"$pull": {"pending_agent_ids": {"$nin": &ids}, "last_recipient_ids": {"$nin": &ids}}},
            )
            .await?;
    }
    db.collection::<AssistantGroup>(GROUPS)
        .update_one(owner_filter(owner, id)?, doc! {"$set": set})
        .await?;
    if let Some(notice) = notice {
        append(db, owner, id, "notice", None, &notice, Vec::new()).await?;
    }
    get(db, owner, id).await
}

/// Delete a group, its transcript and its members' hidden threads (and their
/// keys). Active member turns are stopped before their rows are removed; a
/// worker that races deletion is fenced by the missing conversation row.
pub async fn delete(db: &Database, owner: &str, id: &str) -> AppResult<()> {
    get(db, owner, id).await?;
    let threads = member_threads(db, owner, id).await?;
    for row in &threads {
        if live_turn(row, Utc::now()).is_some() {
            engine::request_stop(db, owner, &row.id).await?;
        }
    }
    for row in threads {
        match engine::delete(db, owner, &row.id).await {
            Ok(_) | Err(AppError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
    }
    // Commit the parent deletion and upload cleanup together. Upload admission
    // writes the same parent, so an in-flight parser cannot leave orphan files.
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    db.collection::<bson::Document>(crate::models::assistant_message_feedback::COLLECTION_NAME)
        .delete_many(doc! {"group_id": id})
        .session(&mut session)
        .await?;
    db.collection::<AssistantGroup>(GROUPS)
        .delete_one(owner_filter(owner, id)?)
        .session(&mut session)
        .await?;
    let mut attachment_cursor = db
        .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
        .find(doc! {"group_id": id, "user_id": owner})
        .projection(doc! {"_id": 1})
        .session(&mut session)
        .await?;
    let attachment_rows: Vec<bson::Document> =
        attachment_cursor.stream(&mut session).try_collect().await?;
    let attachment_ids: Vec<String> = attachment_rows
        .iter()
        .filter_map(|row| row.get_str("_id").ok().map(str::to_owned))
        .collect();
    if !attachment_ids.is_empty() {
        db.collection::<bson::Document>(crate::models::nyxbot_image_delivery::COLLECTION_NAME)
            .delete_many(doc! {"attachment_id": {"$in": &attachment_ids}, "user_id": owner})
            .session(&mut session)
            .await?;
    }
    db.collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
        .delete_many(doc! {"group_id": id, "user_id": owner})
        .session(&mut session)
        .await?;
    db.collection::<bson::Document>(crate::models::assistant_upload_retention::TOMBSTONES)
        .delete_many(doc! {"group_id": id, "user_id": owner})
        .session(&mut session)
        .await?;
    db.collection::<GroupMessage>(MESSAGES)
        .delete_many(doc! {"group_id": id, "user_id": owner})
        .session(&mut session)
        .await?;
    session.commit_transaction().await?;
    Ok(())
}

/// Append a message; its sequence number is allocated atomically.
pub async fn append(
    db: &Database,
    owner: &str,
    group_id: &str,
    role: &str,
    agent: Option<&AssistantAgent>,
    text: &str,
    extra_attachments: Vec<crate::models::assistant_conversation::TurnAttachment>,
) -> AppResult<GroupMessage> {
    append_addressed(
        db,
        owner,
        group_id,
        role,
        agent,
        text,
        &[],
        extra_attachments,
    )
    .await
}

/// Append a message that records the members it was addressed to.
#[allow(clippy::too_many_arguments)]
async fn append_addressed(
    db: &Database,
    owner: &str,
    group_id: &str,
    role: &str,
    agent: Option<&AssistantAgent>,
    text: &str,
    recipients: &[String],
    extra_attachments: Vec<crate::models::assistant_conversation::TurnAttachment>,
) -> AppResult<GroupMessage> {
    append_following(
        db,
        owner,
        group_id,
        role,
        agent,
        text,
        recipients,
        extra_attachments,
        None,
    )
    .await
}

/// Sequence allocation, message insert and follower registration commit
/// together, so every sequence number at or below `message_count` is visible
/// once a follower bound is taken.
#[allow(clippy::too_many_arguments)]
pub async fn append_following(
    db: &Database,
    owner: &str,
    group_id: &str,
    role: &str,
    agent: Option<&AssistantAgent>,
    text: &str,
    recipients: &[String],
    extra_attachments: Vec<crate::models::assistant_conversation::TurnAttachment>,
    follower: Option<&str>,
) -> AppResult<GroupMessage> {
    let filter = owner_filter(owner, group_id)?;
    let db = db.clone();
    let owner = owner.to_owned();
    let group_id = group_id.to_owned();
    let role = role.to_owned();
    let agent_id = agent.map(|agent| agent.id.clone());
    let agent_name = agent.map(|agent| agent.name.clone());
    let text = text.to_owned();
    let recipients = recipients.to_vec();
    let follower = follower.map(str::to_owned);
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<GroupMessage> = Box::pin(async {
                super::erasure_admission::fence_in_session(&db, &owner, session).await?;
                let now = Utc::now();
                let group = db
                    .collection::<AssistantGroup>(GROUPS)
                    .find_one_and_update(
                        filter.clone(),
                        doc! {"$inc": {"message_count": 1},
                        "$set": {"last_message_at": bson::DateTime::from_chrono(now),
                        "updated_at": bson::DateTime::from_chrono(now)}},
                    )
                    .return_document(ReturnDocument::After)
                    .session(&mut *session)
                    .await?
                    .ok_or_else(not_found)?;
                let message = GroupMessage {
                    activities: Vec::new(),
                    org_group: false,
                    author_user_id: None,
                    author_display_name: None,
                    request_id: None,
                    attachments: extra_attachments.clone(),
                    id: Uuid::new_v4().to_string(),
                    group_id: group_id.clone(),
                    user_id: owner.clone(),
                    seq: group.message_count,
                    role: role.clone(),
                    agent_id: agent_id.clone(),
                    agent_name: agent_name.clone(),
                    text: text.clone(),
                    recipient_agent_ids: recipients.clone(),
                    created_at: now,
                };
                db.collection::<GroupMessage>(MESSAGES)
                    .insert_one(&message)
                    .session(&mut *session)
                    .await?;
                if let Some(conversation_id) = follower.as_deref() {
                    follow_in_session(
                        &db,
                        &owner,
                        &group_id,
                        conversation_id,
                        message.seq,
                        now,
                        session,
                    )
                    .await?;
                }
                Ok(message)
            })
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

/// Test helper: an unaddressed message with uploads.
#[cfg(test)]
pub async fn append_with_uploads(
    db: &Database,
    owner: &str,
    group_id: &str,
    role: &str,
    agent: Option<&AssistantAgent>,
    text: &str,
    ids: &[String],
) -> AppResult<GroupMessage> {
    append_with_recipients(db, owner, group_id, role, agent, text, ids, &[]).await
}

/// Append a message with its uploads and the members it was addressed to,
/// all in the one insert.
#[allow(clippy::too_many_arguments)]
pub async fn append_with_recipients(
    db: &Database,
    owner: &str,
    group_id: &str,
    role: &str,
    agent: Option<&AssistantAgent>,
    text: &str,
    ids: &[String],
    recipients: &[String],
) -> AppResult<GroupMessage> {
    if ids.is_empty() {
        return append_addressed(
            db,
            owner,
            group_id,
            role,
            agent,
            text,
            recipients,
            Vec::new(),
        )
        .await;
    }
    if role != "user" {
        return Err(AppError::Forbidden("Only the owner uploads files".into()));
    }
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    let now = Utc::now();
    let group = db
        .collection::<AssistantGroup>(GROUPS)
        .find_one_and_update(
            owner_filter(owner, group_id)?,
            doc! {"$inc": {"message_count": 1},
            "$set": {"last_message_at": bson::DateTime::from_chrono(now),
            "updated_at": bson::DateTime::from_chrono(now)}},
        )
        .return_document(ReturnDocument::After)
        .session(&mut session)
        .await?
        .ok_or_else(not_found)?;
    let message_id = Uuid::new_v4().to_string();
    let attachments = Box::pin(super::assistant_upload_service::bind(
        db,
        owner,
        group_id,
        &message_id,
        ids,
        &mut session,
    ))
    .await?;
    let message = GroupMessage {
        activities: Vec::new(),
        org_group: false,
        author_user_id: None,
        author_display_name: None,
        request_id: None,
        attachments,
        id: message_id,
        group_id: group_id.into(),
        user_id: owner.into(),
        seq: group.message_count,
        role: role.into(),
        agent_id: agent.map(|agent| agent.id.clone()),
        agent_name: agent.map(|agent| agent.name.clone()),
        text: text.into(),
        recipient_agent_ids: recipients.to_vec(),
        created_at: now,
    };
    db.collection::<GroupMessage>(MESSAGES)
        .insert_one(&message)
        .session(&mut session)
        .await?;
    session.commit_transaction().await?;
    Ok(message)
}

/// A page of the transcript in ascending order, newest page first.
pub async fn messages(
    db: &Database,
    owner: &str,
    group_id: &str,
    limit: i64,
    before_seq: Option<i64>,
) -> AppResult<Vec<GroupMessage>> {
    let mut filter = doc! {"group_id": group_id, "user_id": owner};
    if let Some(before) = before_seq {
        filter.insert("seq", doc! {"$lt": before});
    }
    let mut rows: Vec<GroupMessage> = db
        .collection::<GroupMessage>(MESSAGES)
        .find(filter)
        .sort(doc! {"seq": -1})
        .limit(limit)
        .await?
        .try_collect()
        .await?;
    rows.reverse();
    Ok(rows)
}

/// The members a text @mentions (by agent name, case-insensitive).
pub fn mentions(text: &str, members: &[AssistantAgent]) -> Vec<String> {
    let mut found = Vec::new();
    for (index, _) in text.match_indices('@') {
        // `mail@researcher` is not a mention: only a word start counts.
        if text[..index]
            .chars()
            .next_back()
            .is_some_and(|c| c.is_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            continue;
        }
        let name: String = text[index + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'))
            .collect();
        if name.is_empty() {
            continue;
        }
        if let Some(member) = members
            .iter()
            .find(|member| member.name.eq_ignore_ascii_case(&name))
            && !found.contains(&member.id)
        {
            found.push(member.id.clone());
        }
    }
    found
}

/// Address members: they answer once they are free.
pub async fn address(db: &Database, owner: &str, group_id: &str, ids: &[String]) -> AppResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    db.collection::<AssistantGroup>(GROUPS)
        .update_one(
            owner_filter(owner, group_id)?,
            doc! {"$addToSet": {"pending_agent_ids": {"$each": ids}}},
        )
        .await?;
    Ok(())
}

/// Claim an addressed member once across replicas.
pub async fn take_pending(
    db: &Database,
    owner: &str,
    group_id: &str,
    agent_id: &str,
) -> AppResult<bool> {
    let mut filter = owner_filter(owner, group_id)?;
    filter.insert("pending_agent_ids", agent_id);
    Ok(db
        .collection::<AssistantGroup>(GROUPS)
        .update_one(filter, doc! {"$pull": {"pending_agent_ids": agent_id}})
        .await?
        .modified_count
        == 1)
}

/// The owner's hand-offs per message (their NyxBot setting).
async fn handoff_budget(db: &Database, owner: &str) -> AppResult<i32> {
    Ok(super::assistant_settings_service::get(db, owner)
        .await?
        .max_group_handoffs)
}

/// A user message restores the hand-off budget.
pub async fn reset_hops(db: &Database, owner: &str, group_id: &str) -> AppResult<()> {
    let budget = handoff_budget(db, owner).await?;
    db.collection::<AssistantGroup>(GROUPS)
        .update_one(
            owner_filter(owner, group_id)?,
            doc! {"$set": {"hops_remaining": budget}},
        )
        .await?;
    Ok(())
}

/// Test helper for follower semantics on their own: a newer post replaces the
/// thread's follower; replaying the same post (an equal or older sequence
/// number) never renews it. Production registers followers with the post
/// (`append_following`).
#[cfg(test)]
pub async fn follow(
    db: &Database,
    owner: &str,
    group_id: &str,
    conversation_id: &str,
    since_seq: i64,
) -> AppResult<()> {
    let db = db.clone();
    let owner = owner.to_owned();
    let group_id = group_id.to_owned();
    let conversation_id = conversation_id.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> = Box::pin(follow_in_session(
                &db,
                &owner,
                &group_id,
                &conversation_id,
                since_seq,
                Utc::now(),
                session,
            ))
            .await;
            transactions::transaction_result(result)
        })
        .await
        .map_err(transactions::map_transaction_error)
}

async fn follow_in_session(
    db: &Database,
    owner: &str,
    group_id: &str,
    conversation_id: &str,
    since_seq: i64,
    now: chrono::DateTime<Utc>,
    session: &mut ClientSession,
) -> AppResult<()> {
    let mut filter = owner_filter(owner, group_id)?;
    filter.insert("created_by_user_id", bson::Bson::Null);
    let groups = db.collection::<AssistantGroup>(GROUPS);
    // Only an older follower of this thread is replaced: a replay of the same
    // (or an older) post keeps the existing window and deadline.
    groups
        .update_one(
            filter.clone(),
            doc! {"$pull": {"followers": {"conversation_id": conversation_id,
            "since_seq": {"$lt": since_seq}}}},
        )
        .session(&mut *session)
        .await?;
    let follower = bson::to_bson(&GroupFollower {
        conversation_id: conversation_id.into(),
        since_seq,
        created_at: now,
        expires_at: Some(now + chrono::Duration::hours(FOLLOWER_TTL_HOURS)),
    })
    .map_err(|_| AppError::Internal("Follower encoding failed".into()))?;
    filter.insert(
        "followers",
        doc! {"$not": {"$elemMatch": {"conversation_id": conversation_id}}},
    );
    groups
        .update_one(
            filter,
            doc! {"$push": {"followers": {"$each": [follower],
            "$slice": -(MAX_FOLLOWERS as i64)}}},
        )
        .session(&mut *session)
        .await?;
    Ok(())
}

/// Expiry is classified at the acknowledged take, not at a clock sampled
/// before the quiet checks' awaits; the result is frozen afterwards.
pub(crate) fn followers_live_at(
    followers: Vec<GroupFollower>,
    at: chrono::DateTime<Utc>,
) -> Vec<GroupFollower> {
    followers
        .into_iter()
        .filter(|follower| follower.expiry() > at)
        .collect()
}

/// The upper bound comes from the same update as the take, so later messages
/// never enter a summary.
pub async fn take_followers_if_quiet(
    db: &Database,
    owner: &str,
    group_id: &str,
) -> AppResult<(Vec<GroupFollower>, i64)> {
    let now = Utc::now();
    let groups = db.collection::<AssistantGroup>(GROUPS);
    // `{"expires_at": null}` matches rows storing null and legacy rows that
    // have no such field.
    let legacy_cutoff = now - chrono::Duration::hours(FOLLOWER_TTL_HOURS);
    for condition in [
        doc! {"expires_at": {"$lte": bson::DateTime::from_chrono(now)}},
        doc! {"expires_at": null,
        "created_at": {"$lte": bson::DateTime::from_chrono(legacy_cutoff)}},
    ] {
        groups
            .update_one(
                owner_filter(owner, group_id)?,
                doc! {"$pull": {"followers": condition}},
            )
            .await?;
    }
    let group = get(db, owner, group_id).await?;
    let live: Vec<GroupFollower> = group
        .followers
        .into_iter()
        .filter(|follower| follower.expiry() > now)
        .collect();
    if live.is_empty()
        || !group.pending_agent_ids.is_empty()
        || !working(db, owner, group_id).await?.is_empty()
    {
        return Ok((Vec::new(), 0));
    }
    // Identity and length, not whole-array equality: legacy rows have no
    // `expires_at` and would never compare equal to a re-serialized document.
    let identities: Vec<bson::Document> = live
        .iter()
        .map(|follower| {
            doc! {"$elemMatch": {"conversation_id": &follower.conversation_id,
            "since_seq": follower.since_seq}}
        })
        .collect();
    let mut filter = owner_filter(owner, group_id)?;
    filter.insert("pending_agent_ids", doc! {"$size": 0});
    filter.insert(
        "$and",
        vec![
            doc! {"followers": {"$size": live.len() as i64}},
            doc! {"followers": {"$all": identities}},
        ],
    );
    let before = groups
        .find_one_and_update(filter, doc! {"$set": {"followers": []}})
        .return_document(ReturnDocument::Before)
        .await?;
    // Sampled after the acknowledgement: a follower that expired during the
    // quiet checks is dropped without a summary.
    let taken_at = Utc::now();
    Ok(match before {
        Some(before) => (
            followers_live_at(before.followers, taken_at),
            before.message_count,
        ),
        None => (Vec::new(), 0),
    })
}

/// What the members said in `(since_seq, upper_seq]`, for a follower's summary.
pub async fn replies_since(
    db: &Database,
    owner: &str,
    group_id: &str,
    since_seq: i64,
    upper_seq: i64,
) -> AppResult<String> {
    let rows: Vec<GroupMessage> = db
        .collection::<GroupMessage>(MESSAGES)
        .find(doc! {"group_id": group_id, "user_id": owner,
        "seq": {"$gt": since_seq, "$lte": upper_seq},
        "role": {"$ne": "user"}})
        .sort(doc! {"seq": -1})
        .limit(12)
        .await?
        .try_collect()
        .await?;
    let mut lines: Vec<String> = rows
        .iter()
        .map(|row| {
            let speaker = row
                .agent_name
                .as_deref()
                .map(identifier)
                .unwrap_or_else(|| "NyxID".into());
            format!(
                "[{speaker}]: {}",
                excerpt(&row.text, 600).replace('\n', "\n    ")
            )
        })
        .collect();
    lines.reverse();
    Ok(lines.join("\n"))
}

/// Spend one agent hand-off; `false` once the budget is used up.
pub async fn spend_hop(db: &Database, owner: &str, group_id: &str) -> AppResult<bool> {
    let mut filter = owner_filter(owner, group_id)?;
    filter.insert("hops_remaining", doc! {"$gt": 0});
    Ok(db
        .collection::<AssistantGroup>(GROUPS)
        .update_one(filter, doc! {"$inc": {"hops_remaining": -1}})
        .await?
        .modified_count
        == 1)
}

pub async fn member_threads(
    db: &Database,
    owner: &str,
    group_id: &str,
) -> AppResult<Vec<AssistantConversation>> {
    Ok(db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find(doc! {"user_id": owner, "group_id": group_id})
        .await?
        .try_collect()
        .await?)
}

pub async fn member_thread(
    db: &Database,
    owner: &str,
    group_id: &str,
    agent_id: &str,
) -> AppResult<Option<AssistantConversation>> {
    Ok(db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"user_id": owner, "group_id": group_id, "agent_id": agent_id})
        .await?)
}

/// Members whose group turn is running.
pub async fn working(db: &Database, owner: &str, group_id: &str) -> AppResult<Vec<String>> {
    let now = Utc::now();
    Ok(member_threads(db, owner, group_id)
        .await?
        .into_iter()
        .filter(|row| live_turn(row, now).is_some())
        .filter_map(|row| row.agent_id)
        .collect())
}

/// New transcript lines for a member since `since_seq`, bounded, and the
/// newest sequence number they cover.
pub async fn transcript_since(
    db: &Database,
    owner: &str,
    group_id: &str,
    since_seq: i64,
) -> AppResult<(String, i64)> {
    let mut rows: Vec<GroupMessage> = db
        .collection::<GroupMessage>(MESSAGES)
        .find(doc! {"group_id": group_id, "user_id": owner, "seq": {"$gt": since_seq}})
        .sort(doc! {"seq": -1})
        .limit(TRANSCRIPT_MESSAGES)
        .await?
        .try_collect()
        .await?;
    let newest = rows.first().map_or(since_seq, |row| row.seq);
    let mut lines = Vec::new();
    let mut used = 0;
    for row in &rows {
        let speaker = match row.role.as_str() {
            "user" => "user".to_owned(),
            "agent" => identifier(row.agent_name.as_deref().unwrap_or("agent")),
            _ => "NyxID".to_owned(),
        };
        // Continuation lines are indented, so only a real user message can
        // start a line with "[user]:".
        let line = format!(
            "[{speaker}]: {}",
            excerpt(&row.text, 3000).replace('\n', "\n    ")
        );
        used += line.len();
        if used > TRANSCRIPT_CHARS && !lines.is_empty() {
            break;
        }
        lines.push(line);
    }
    rows.clear();
    lines.reverse();
    Ok((lines.join("\n"), newest))
}

pub async fn set_seen(db: &Database, owner: &str, thread_id: &str, seq: i64) -> AppResult<()> {
    db.collection::<AssistantConversation>(CONVERSATIONS)
        .update_one(
            doc! {"_id": thread_id, "user_id": owner, "group_seen_seq": {"$lt": seq}},
            doc! {"$set": {"group_seen_seq": seq}},
        )
        .await?;
    Ok(())
}

/// Groups with members waiting to answer, or with followers still waiting
/// for a quiet group (so their summaries and expiry never depend on another
/// member turn), for the retry sweep.
pub async fn with_pending(db: &Database) -> AppResult<Vec<AssistantGroup>> {
    let rows: Vec<AssistantGroup> = db
        .collection::<AssistantGroup>(GROUPS)
        .find(doc! {"$or": [{"pending_agent_ids.0": {"$exists": true}}, {"followers.0": {"$exists": true}}, {"created_by_user_id": {"$type": "string"}}]})
        .sort(doc! {"pending_checked_at": 1})
        .limit(50)
        .await?
        .try_collect()
        .await?;
    let ids: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    db.collection::<AssistantGroup>(GROUPS)
        .update_many(
            doc! {"_id": {"$in": ids}},
            doc! {"$set": {"pending_checked_at": bson::DateTime::now()}},
        )
        .await?;
    Ok(rows)
}

/// Drop a departed agent from the pending queue and every sticky selection
/// (the owner's and, in organization groups, each participant's).
fn pull_recipient(group: &AssistantGroup, agent_id: &str) -> bson::Document {
    let mut pull = doc! {"pending_agent_ids": agent_id, "last_recipient_ids": agent_id};
    for actor in group.last_recipient_ids_by_actor.keys() {
        pull.insert(format!("last_recipient_ids_by_actor.{actor}"), agent_id);
    }
    pull
}

/// A purged agent leaves every group it was in.
pub async fn remove_agent(db: &Database, owner: &str, agent_id: &str) -> AppResult<()> {
    let groups: Vec<AssistantGroup> = db
        .collection::<AssistantGroup>(GROUPS)
        .find(doc! {"user_id": owner, "member_agent_ids": agent_id})
        .await?
        .try_collect()
        .await?;
    for group in groups {
        let remaining: Vec<String> = group
            .member_agent_ids
            .iter()
            .filter(|id| id.as_str() != agent_id)
            .cloned()
            .collect();
        let agents = team::agents(db, owner, true).await?;
        let members: Vec<AssistantAgent> = remaining
            .iter()
            .filter_map(|id| agents.iter().find(|agent| &agent.id == id).cloned())
            .collect();
        if members.is_empty() {
            if super::org_group_service::is_org(&group) {
                super::org_group_service::delete_contents(db, &group, None).await?;
            } else {
                delete(db, owner, &group.id).await?;
            }
            continue;
        }
        db.collection::<AssistantGroup>(GROUPS)
            .update_one(
                doc! {"_id": &group.id, "user_id": owner},
                doc! {"$set": {"member_agent_ids": &remaining,
                "lead_agent_id": lead_of(&members)},
                "$pull": pull_recipient(&group, agent_id)},
            )
            .await?;
    }
    Ok(())
}
