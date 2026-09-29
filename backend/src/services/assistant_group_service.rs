//! Group chats (Grok-Bot style): the owner plus several of their agents in
//! one transcript. A user message goes to the members it @mentions, else to
//! the lead; members hand work to each other by @mentioning them in replies,
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

use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::AssistantAgent,
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        assistant_group::{
            AssistantGroup, COLLECTION_NAME as GROUPS, GroupFollower, GroupMessage, MAX_FOLLOWERS,
            MAX_MEMBERS, MAX_NAME_CHARS, MESSAGES_COLLECTION_NAME as MESSAGES,
        },
    },
    services::{
        assistant_nyxagent::{self as engine, excerpt, identifier, live_turn},
        assistant_team_service as team,
    },
};

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
        name,
        member_agent_ids: members.iter().map(|member| member.id.clone()).collect(),
        lead_agent_id: lead_of(&members),
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
    db.collection::<AssistantGroup>(GROUPS)
        .insert_one(&group)
        .await?;
    append(
        db,
        owner,
        &group.id,
        "notice",
        None,
        &format!("Group created with {}.", names(&members)),
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
                doc! {"$pull": {"pending_agent_ids": {"$nin": &ids}}},
            )
            .await?;
    }
    db.collection::<AssistantGroup>(GROUPS)
        .update_one(owner_filter(owner, id)?, doc! {"$set": set})
        .await?;
    if let Some(notice) = notice {
        append(db, owner, id, "notice", None, &notice).await?;
    }
    get(db, owner, id).await
}

/// Delete a group, its transcript and its members' hidden threads (and their
/// keys). Refused while a member is still answering.
pub async fn delete(db: &Database, owner: &str, id: &str) -> AppResult<()> {
    get(db, owner, id).await?;
    let threads = member_threads(db, owner, id).await?;
    let now = Utc::now();
    if threads.iter().any(|row| live_turn(row, now).is_some()) {
        return Err(AppError::AssistantTurnActive);
    }
    for row in threads {
        match engine::delete(db, owner, &row.id).await {
            Ok(_) | Err(AppError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
    }
    db.collection::<GroupMessage>(MESSAGES)
        .delete_many(doc! {"group_id": id, "user_id": owner})
        .await?;
    db.collection::<AssistantGroup>(GROUPS)
        .delete_one(owner_filter(owner, id)?)
        .await?;
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
) -> AppResult<GroupMessage> {
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
        .await?
        .ok_or_else(not_found)?;
    let message = GroupMessage {
        id: Uuid::new_v4().to_string(),
        group_id: group_id.into(),
        user_id: owner.into(),
        seq: group.message_count,
        role: role.into(),
        agent_id: agent.map(|agent| agent.id.clone()),
        agent_name: agent.map(|agent| agent.name.clone()),
        text: text.into(),
        created_at: now,
    };
    db.collection::<GroupMessage>(MESSAGES)
        .insert_one(&message)
        .await?;
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

/// A NyxBot thread posted work into the group: it is woken with a summary of
/// what follows once the group goes quiet. Its newest post replaces an older
/// one; bounded.
pub async fn follow(
    db: &Database,
    owner: &str,
    group_id: &str,
    conversation_id: &str,
    since_seq: i64,
) -> AppResult<()> {
    let filter = owner_filter(owner, group_id)?;
    let groups = db.collection::<AssistantGroup>(GROUPS);
    groups
        .update_one(
            filter.clone(),
            doc! {"$pull": {"followers": {"conversation_id": conversation_id}}},
        )
        .await?;
    let follower = bson::to_bson(&GroupFollower {
        conversation_id: conversation_id.into(),
        since_seq,
        created_at: Utc::now(),
    })
    .map_err(|_| AppError::Internal("Follower encoding failed".into()))?;
    groups
        .update_one(
            filter,
            doc! {"$push": {"followers": {"$each": [follower],
            "$slice": -(MAX_FOLLOWERS as i64)}}},
        )
        .await?;
    Ok(())
}

/// Take the group's followers once it is quiet (nobody working or waiting to
/// answer). Atomic: only the caller that clears them notifies.
pub async fn take_followers_if_quiet(
    db: &Database,
    owner: &str,
    group_id: &str,
) -> AppResult<Vec<GroupFollower>> {
    let group = get(db, owner, group_id).await?;
    if group.followers.is_empty()
        || !group.pending_agent_ids.is_empty()
        || !working(db, owner, group_id).await?.is_empty()
    {
        return Ok(Vec::new());
    }
    let mut filter = owner_filter(owner, group_id)?;
    filter.insert(
        "followers",
        bson::to_bson(&group.followers)
            .map_err(|_| AppError::Internal("Follower encoding failed".into()))?,
    );
    filter.insert("pending_agent_ids", doc! {"$size": 0});
    let taken = db
        .collection::<AssistantGroup>(GROUPS)
        .update_one(filter, doc! {"$set": {"followers": []}})
        .await?
        .modified_count
        == 1;
    Ok(if taken { group.followers } else { Vec::new() })
}

/// What the members said after `since_seq`, for a follower's summary.
pub async fn replies_since(
    db: &Database,
    owner: &str,
    group_id: &str,
    since_seq: i64,
) -> AppResult<String> {
    let rows: Vec<GroupMessage> = db
        .collection::<GroupMessage>(MESSAGES)
        .find(
            doc! {"group_id": group_id, "user_id": owner, "seq": {"$gt": since_seq},
            "role": {"$ne": "user"}},
        )
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

/// Groups with members waiting to answer, for the retry sweep.
pub async fn with_pending(db: &Database) -> AppResult<Vec<AssistantGroup>> {
    let rows: Vec<AssistantGroup> = db
        .collection::<AssistantGroup>(GROUPS)
        .find(doc! {"pending_agent_ids.0": {"$exists": true}})
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
            delete(db, owner, &group.id).await?;
            continue;
        }
        db.collection::<AssistantGroup>(GROUPS)
            .update_one(
                doc! {"_id": &group.id, "user_id": owner},
                doc! {"$set": {"member_agent_ids": &remaining,
                "lead_agent_id": lead_of(&members)},
                "$pull": {"pending_agent_ids": agent_id}},
            )
            .await?;
    }
    Ok(())
}
