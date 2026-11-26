use std::collections::HashSet;

use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, Document, doc},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::{AnswerAudience, GuestMode, GuestTurn};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::AssistantAgent,
        catalog_skill_revision::SkillReference,
        nyxbot_channel::{NyxbotChannel, NyxbotThread, THREADS_COLLECTION_NAME},
        user::User,
    },
    services::{assistant_acknowledgement_service::ChatAuthority, audit_service::AuditActor},
};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Attachment {
    skill_id: String,
    audiences: Vec<AnswerAudience>,
    all_requests_approved: bool,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct Selection {
    revision: i64,
    skills_revision: i64,
    bot_owner: String,
    #[serde(default)]
    bot_ownership_version: i64,
    binding_id: String,
    agent_id: String,
    maintainer: String,
    attachments: Vec<Attachment>,
}

#[derive(Deserialize)]
struct Bot {
    #[serde(rename = "_id")]
    id: String,
    user_id: String,
    #[serde(default)]
    ownership_version: i64,
    is_active: bool,
    #[serde(default)]
    guest_skill_selection: Selection,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Update {
    #[serde(default)]
    agent_id: Option<String>,
    expected_revision: i64,
    skills_revision: i64,
    attachments: Vec<Attachment>,
}

#[derive(Serialize)]
pub(crate) struct Inventory {
    agent_id: String,
    revision: i64,
    skills_revision: i64,
    available: Vec<SkillReference>,
    standing_approval_skill_ids: Vec<String>,
    attachments: Vec<Attachment>,
}

fn denied() -> AppError {
    AppError::Forbidden("This attached skill is unavailable in the current channel turn".into())
}

fn stale() -> AppError {
    AppError::Conflict("Bot skill attachments changed. Review the current revision.".into())
}

pub(crate) fn exact_pin<'a>(
    agent: &'a AssistantAgent,
    skills_revision: i64,
    skill_id: &str,
) -> Option<&'a SkillReference> {
    if skills_revision != agent.skills_revision {
        return None;
    }
    agent
        .skills
        .iter()
        .find(|pin| !pin.disabled && pin.source == "ornn" && pin.skill_id == skill_id)
}

async fn hosted_binding(db: &Database, bot_id: &str) -> AppResult<NyxbotChannel> {
    let mut rows = db
        .collection::<NyxbotChannel>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .find(doc! {"channel_bot_id":bot_id,"status":"active"})
        .limit(2)
        .await?;
    let binding = rows.try_next().await?.ok_or_else(denied)?;
    if rows.try_next().await?.is_some() {
        return Err(stale());
    }
    Ok(binding)
}

async fn require_bot_maintainer(db: &Database, person: &str, bot: &Bot) -> AppResult<()> {
    if !bot.is_active {
        return Err(denied());
    }
    if bot.user_id != person
        && (db
            .collection::<User>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id": &bot.user_id, "user_type":"org", "is_active":true})
            .await?
            .is_none()
            || !crate::services::org_service::is_admin(db, person, &bot.user_id).await?)
    {
        return Err(denied());
    }
    let maintainer = db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":person,"is_active":true})
        .await?
        .filter(|user| user.user_type.is_person());
    if maintainer.is_none() {
        return Err(denied());
    }
    Ok(())
}

async fn binding_agent(
    db: &Database,
    binding: &NyxbotChannel,
    agent_id: Option<&str>,
) -> AppResult<AssistantAgent> {
    let agent_owner = binding
        .org_specialist
        .as_ref()
        .map(|org| org.org_id.as_str())
        .unwrap_or(&binding.user_id);
    let mut filter = doc! {"user_id":agent_owner,"destroyed_at":null};
    match agent_id {
        Some(id) => {
            filter.insert("_id", id);
        }
        None => {
            filter.insert("kind", "nyxbot");
        }
    }
    db.collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one(filter)
        .await?
        .ok_or_else(denied)
}

async fn owned_binding(
    db: &Database,
    person: &str,
    bot: &Bot,
) -> AppResult<(NyxbotChannel, AssistantAgent)> {
    require_bot_maintainer(db, person, bot).await?;
    let binding = hosted_binding(db, &bot.id).await?;
    if binding.bot_owner_id.as_deref().unwrap_or(&binding.user_id) != bot.user_id {
        return Err(denied());
    }
    let agent = binding_agent(db, &binding, binding.agent_id.as_deref()).await?;
    crate::services::org_agent_service::require_maintain(db, person, &agent).await?;
    Ok((binding, agent))
}

async fn load(db: &Database, bot_id: &str) -> AppResult<(Bot, Selection)> {
    let mut bot = db
        .collection::<Bot>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":bot_id})
        .projection(
            doc! {"user_id":1,"ownership_version":1,"is_active":1,"guest_skill_selection":1},
        )
        .await?
        .ok_or_else(|| AppError::ChannelBotNotFound(bot_id.to_owned()))?;
    let selection = std::mem::take(&mut bot.guest_skill_selection);
    Ok((bot, selection))
}

pub(crate) async fn maintainer_agent(
    db: &Database,
    person: &str,
    bot_id: &str,
) -> AppResult<AssistantAgent> {
    Ok(maintainer_binding(db, person, bot_id).await?.1)
}

pub(crate) async fn maintainer_binding(
    db: &Database,
    person: &str,
    bot_id: &str,
) -> AppResult<(NyxbotChannel, AssistantAgent)> {
    let (bot, _) = load(db, bot_id).await?;
    owned_binding(db, person, &bot).await
}

pub(crate) async fn standing_approved(
    db: &Database,
    actor: &str,
    bot_id: &str,
    agent: &AssistantAgent,
    skill_id: &str,
) -> AppResult<bool> {
    let (bot, selection) = load(db, bot_id).await?;
    let (binding, live) = owned_binding(db, actor, &bot).await?;
    Ok(live.id == agent.id
        && live.skills_revision == agent.skills_revision
        && selection.bot_owner == bot.user_id
        && selection.bot_ownership_version == bot.ownership_version
        && selection.binding_id == binding.id
        && selection.agent_id == live.id
        && selection.skills_revision == live.skills_revision
        && selection.maintainer == actor
        && selection
            .attachments
            .iter()
            .any(|pin| pin.skill_id == skill_id && pin.all_requests_approved))
}

fn attachment_scope_matches(selection: &Selection, bot: &Bot, binding: &NyxbotChannel) -> bool {
    selection.binding_id == binding.id
        && selection.bot_owner == bot.user_id
        && selection.bot_ownership_version == bot.ownership_version
}

fn override_threads(binding: &NyxbotChannel) -> Document {
    doc! {
        "channel_id": &binding.id, "user_id": &binding.user_id,
        "record_scope": {"$ne": crate::services::channel_thread_follow_service::SCOPE},
    }
}

#[derive(Default)]
struct OverrideAgents {
    maintainable: Vec<AssistantAgent>,
    restricted_skill_ids: HashSet<String>,
}

async fn override_agents(
    db: &Database,
    actor: &str,
    binding: &NyxbotChannel,
    default_agent: &AssistantAgent,
) -> AppResult<OverrideAgents> {
    use crate::services::org_agent_service;
    let mut filter = override_threads(binding);
    filter.insert(
        "agent_id",
        doc! {"$type": "string", "$ne": &default_agent.id},
    );
    let ids = db
        .collection::<NyxbotThread>(THREADS_COLLECTION_NAME)
        .distinct("agent_id", filter)
        .await?;
    if ids.is_empty() {
        return Ok(OverrideAgents::default());
    }
    let access = org_agent_service::access(db, actor, &default_agent.user_id).await?;
    let agents: Vec<AssistantAgent> = db
        .collection(crate::models::assistant_agent::COLLECTION_NAME)
        .find(doc! {
            "_id": {"$in": ids}, "user_id": &default_agent.user_id, "destroyed_at": null,
            "skills": {"$elemMatch": {"source": "ornn", "disabled": {"$ne": true}}},
        })
        .sort(doc! {"_id": 1})
        .await?
        .try_collect()
        .await?;
    let mut overrides = OverrideAgents::default();
    for agent in agents {
        if org_agent_service::specialist_rights(actor, &agent, &access).can_maintain {
            overrides.maintainable.push(agent);
        } else {
            overrides.restricted_skill_ids.extend(
                agent
                    .skills
                    .into_iter()
                    .filter(|pin| pin.source == "ornn")
                    .map(|pin| pin.skill_id),
            );
        }
    }
    Ok(overrides)
}

fn inventory(
    agent: AssistantAgent,
    overrides: Vec<AssistantAgent>,
    selection: Selection,
) -> Inventory {
    let standing_approval_skill_ids: Vec<_> = agent
        .skills
        .iter()
        .filter(|pin| !pin.disabled && pin.source == "ornn")
        .map(|pin| pin.skill_id.clone())
        .collect();
    let mut seen = HashSet::new();
    let available: Vec<_> = agent
        .skills
        .into_iter()
        .chain(overrides.into_iter().flat_map(|agent| agent.skills))
        .filter(|pin| !pin.disabled && pin.source == "ornn" && seen.insert(pin.skill_id.clone()))
        .collect();
    let attachments = selection
        .attachments
        .into_iter()
        .filter(|entry| seen.contains(&entry.skill_id))
        .map(|mut entry| {
            entry.all_requests_approved &= selection.agent_id == agent.id
                && selection.skills_revision == agent.skills_revision
                && standing_approval_skill_ids.contains(&entry.skill_id);
            entry
        })
        .collect();
    Inventory {
        agent_id: agent.id,
        revision: selection.revision,
        skills_revision: agent.skills_revision,
        available,
        standing_approval_skill_ids,
        attachments,
    }
}

pub(crate) async fn get(db: &Database, person: &str, bot_id: &str) -> AppResult<Inventory> {
    let (bot, mut selection) = load(db, bot_id).await?;
    let (binding, agent) = owned_binding(db, person, &bot).await?;
    if !attachment_scope_matches(&selection, &bot, &binding) {
        selection.attachments.clear();
    }
    let overrides = override_agents(db, person, &binding, &agent).await?;
    Ok(inventory(agent, overrides.maintainable, selection))
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PreparedOverride {
    agent_id: String,
    skills_revision: i64,
    maintain_revision: i64,
    chat_id: String,
    binding_generation: i64,
}

async fn validate_update(
    db: &Database,
    binding: &NyxbotChannel,
    agent: &AssistantAgent,
    overrides: &[AssistantAgent],
    input: &Update,
    previous: Option<&Selection>,
) -> AppResult<Vec<PreparedOverride>> {
    if input.skills_revision != agent.skills_revision
        || input.agent_id.as_ref().is_some_and(|id| id != &agent.id)
    {
        return Err(stale());
    }
    let mut roots = HashSet::new();
    let override_for = |skill_id: &str| {
        overrides
            .iter()
            .find(|agent| exact_pin(agent, agent.skills_revision, skill_id).is_some())
    };
    let mut touched = Vec::new();
    let mut agents = HashSet::new();
    for attachment in &input.attachments {
        let default_pin = exact_pin(agent, agent.skills_revision, &attachment.skill_id).is_some();
        if !roots.insert(&attachment.skill_id)
            || (attachment.all_requests_approved && !default_pin)
            || attachment.audiences.len() > 2
            || matches!(attachment.audiences.as_slice(), [first, second] if first == second)
        {
            return Err(AppError::ValidationError(
                "Choose current enabled skill pins and unique channel audiences".into(),
            ));
        }
        if default_pin {
            continue;
        }
        let override_agent = override_for(&attachment.skill_id).ok_or_else(stale)?;
        if agents.insert(&override_agent.id) {
            touched.push(override_agent);
        }
    }
    for attachment in previous
        .into_iter()
        .flat_map(|selection| &selection.attachments)
    {
        if !roots.contains(&attachment.skill_id)
            && exact_pin(agent, agent.skills_revision, &attachment.skill_id).is_none()
            && let Some(override_agent) = override_for(&attachment.skill_id)
            && agents.insert(&override_agent.id)
        {
            touched.push(override_agent);
        }
    }
    let mut prepared = Vec::new();
    for override_agent in touched {
        let mut filter = override_threads(binding);
        filter.insert("agent_id", &override_agent.id);
        let chat = db
            .collection::<NyxbotThread>(THREADS_COLLECTION_NAME)
            .find_one(filter)
            .await?
            .ok_or_else(stale)?;
        prepared.push(PreparedOverride {
            agent_id: override_agent.id.clone(),
            skills_revision: override_agent.skills_revision,
            maintain_revision: override_agent.org_maintain_revision,
            chat_id: chat.id,
            binding_generation: chat.follow.binding_generation,
        });
    }
    Ok(prepared)
}

/// Server-observed binding and rights attached to a maintainer's staged choice.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PreparedSelection {
    bot_id: String,
    expected_revision: i64,
    agent_owner: String,
    maintain_revision: i64,
    binding_agent: Option<String>,
    binding_owner: String,
    binding_org_revision: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    overrides: Vec<PreparedOverride>,
    selection: Selection,
}

pub(crate) async fn prepare(
    db: &Database,
    actor: &str,
    bot_id: &str,
    input: &Update,
) -> AppResult<PreparedSelection> {
    let (bot, previous) = load(db, bot_id).await?;
    let (binding, agent) = owned_binding(db, actor, &bot).await?;
    if previous.revision != input.expected_revision {
        return Err(stale());
    }
    let overrides = override_agents(db, actor, &binding, &agent).await?;
    let same_scope = attachment_scope_matches(&previous, &bot, &binding);
    if same_scope
        && previous.agent_id != agent.id
        && input.agent_id.is_none()
        && input
            .attachments
            .iter()
            .any(|entry| entry.all_requests_approved)
    {
        return Err(stale());
    }
    let prepared_overrides = validate_update(
        db,
        &binding,
        &agent,
        &overrides.maintainable,
        input,
        same_scope.then_some(&previous),
    )
    .await?;
    let mut attachments = input.attachments.clone();
    if same_scope {
        // Preserve restrictions on live overrides outside the editor's maintenance scope.
        attachments.extend(previous.attachments.into_iter().filter_map(|mut entry| {
            if !overrides.restricted_skill_ids.contains(&entry.skill_id)
                || exact_pin(&agent, agent.skills_revision, &entry.skill_id).is_some()
                || overrides.maintainable.iter().any(|override_agent| {
                    exact_pin(
                        override_agent,
                        override_agent.skills_revision,
                        &entry.skill_id,
                    )
                    .is_some()
                })
            {
                return None;
            }
            entry.all_requests_approved = false;
            Some(entry)
        }));
    }
    Ok(PreparedSelection {
        bot_id: bot.id,
        expected_revision: previous.revision,
        agent_owner: agent.user_id,
        maintain_revision: agent.org_maintain_revision,
        binding_agent: binding.agent_id,
        binding_owner: binding.user_id,
        binding_org_revision: binding.org_specialist.map(|binding| binding.revision),
        overrides: prepared_overrides,
        selection: Selection {
            revision: previous.revision.checked_add(1).ok_or_else(stale)?,
            skills_revision: agent.skills_revision,
            bot_owner: bot.user_id,
            bot_ownership_version: bot.ownership_version,
            binding_id: binding.id,
            agent_id: agent.id,
            maintainer: actor.to_owned(),
            attachments,
        },
    })
}

/// The caller fences live enrollment or prepared activation in the same
/// transaction. This function fences the resources and authority of the choice.
pub(crate) async fn apply_prepared_in_session(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    bot_id: &str,
    prepared: &PreparedSelection,
) -> AppResult<()> {
    use crate::services::org_membership_authority;
    if prepared.bot_id != bot_id {
        return Err(stale());
    }
    let selection = &prepared.selection;
    let maintainer = &selection.maintainer;
    org_membership_authority::fence_live_member(db, maintainer, tx).await?;
    if selection.bot_owner != *maintainer {
        org_membership_authority::fence_org(db, &selection.bot_owner, tx).await?;
        let admin = db
            .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
            .find_one(doc! {
                "org_user_id": &selection.bot_owner, "member_user_id": maintainer,
                "role": "admin", "revoked_at": null,
            })
            .session(&mut *tx)
            .await?;
        if admin.is_none() {
            return Err(denied());
        }
    }
    fence_maintained_agent(
        db,
        tx,
        maintainer,
        &prepared.agent_owner,
        &selection.agent_id,
        selection.skills_revision,
        prepared.maintain_revision,
    )
    .await?;
    for override_agent in &prepared.overrides {
        fence_maintained_agent(
            db,
            tx,
            maintainer,
            &prepared.agent_owner,
            &override_agent.agent_id,
            override_agent.skills_revision,
            override_agent.maintain_revision,
        )
        .await?;
        let matched = db.collection::<Document>(THREADS_COLLECTION_NAME)
            .update_one(doc! {
                "_id": &override_agent.chat_id, "channel_id": &selection.binding_id,
                "user_id": &prepared.binding_owner, "agent_id": &override_agent.agent_id,
                "record_scope": {"$ne":crate::services::channel_thread_follow_service::SCOPE},
                "$expr": {"$eq": [{"$ifNull": ["$binding_generation", 0_i64]}, override_agent.binding_generation]},
            }, doc! {"$inc": {"guest_skill_selection_fence":1_i64}})
            .session(&mut *tx).await?;
        if matched.matched_count != 1 {
            return Err(stale());
        }
    }
    let binding = db.collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .update_one(doc! {
            "_id": &selection.binding_id, "channel_bot_id": bot_id, "status": "active",
            "agent_id": &prepared.binding_agent, "user_id": &prepared.binding_owner,
            "org_specialist.revision": &prepared.binding_org_revision,
            "$expr": {"$eq": [{"$ifNull": ["$bot_owner_id", "$user_id"]}, &selection.bot_owner]},
        }, doc! {"$inc":{"guest_skill_selection_fence":1_i64}})
        .session(&mut *tx).await?;
    if binding.matched_count != 1 {
        return Err(stale());
    }
    let mut filter = doc! {
        "_id": bot_id, "user_id": &selection.bot_owner, "is_active": true,
        "$expr": {"$eq": [{"$ifNull": ["$ownership_version", 0_i64]}, selection.bot_ownership_version]},
    };
    if prepared.expected_revision == 0 {
        filter.insert("guest_skill_selection", doc! {"$exists": false});
    } else {
        filter.insert("guest_skill_selection.revision", prepared.expected_revision);
    }
    let value = bson::to_document(selection)
        .map_err(|_| AppError::Internal("Could not store attached channel skills".into()))?;
    let changed = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            filter,
            doc! {"$set":{
                "guest_skill_selection": value, "channel_guest_data_present": true,
            }},
        )
        .session(&mut *tx)
        .await?;
    if changed.matched_count != 1 {
        return Err(stale());
    }
    Ok(())
}

async fn fence_maintained_agent(
    db: &Database,
    tx: &mut mongodb::ClientSession,
    maintainer: &str,
    agent_owner: &str,
    agent_id: &str,
    skills_revision: i64,
    maintain_revision: i64,
) -> AppResult<()> {
    use crate::services::org_agent_service;

    let agent = db
        .collection::<AssistantAgent>(crate::models::assistant_agent::COLLECTION_NAME)
        .find_one_and_update(
            doc! {
                "_id": agent_id, "user_id": agent_owner, "destroyed_at": null,
                "$expr": {"$and": [
                    {"$eq": [{"$ifNull": ["$skills_revision", 0_i64]}, skills_revision]},
                    {"$eq": [{"$ifNull": ["$org_maintain_revision", 0_i64]}, maintain_revision]},
                ]},
            },
            doc! {"$inc":{"guest_skill_selection_fence":1_i64}},
        )
        .session(&mut *tx)
        .await?
        .ok_or_else(stale)?;
    if agent.user_id != maintainer
        && !org_agent_service::member_rights_in_session(
            db,
            tx,
            maintainer,
            &agent,
            org_agent_service::MemberFence::MemberAndOrg,
        )
        .await?
        .can_maintain
    {
        return Err(denied());
    }
    Ok(())
}

pub(crate) async fn set(
    state: &crate::AppState,
    actor: &AuditActor,
    bot_id: &str,
    input: Update,
) -> AppResult<Inventory> {
    let db = &state.db;
    let GuestMode::AddOn {
        activation_epoch,
        bot_revision,
    } = super::select_mode(db, bot_id).await?
    else {
        return Err(denied());
    };
    let prepared = prepare(db, &actor.user_id, bot_id, &input).await?;
    let context = (
        db,
        actor,
        bot_id,
        activation_epoch,
        bot_revision,
        &prepared,
        &state.audit_chain_hmac_key,
    );
    crate::services::transaction_service::run_db_only(db, &context, |tx, context| {
        let (db, actor, bot_id, epoch, bot_revision, prepared, audit_key) = *context;
        Box::pin(async move {
            super::fence_selection_in_session(db, tx, bot_id, epoch, bot_revision).await?;
            apply_prepared_in_session(db, tx, bot_id, prepared).await?;
            let selection = &prepared.selection;
            crate::services::audit_service::log_actor_event_in_session(
                db, tx, audit_key.as_ref().as_ref(), actor, "channel_guest_skills_updated",
                serde_json::json!({
                    "bot_id": bot_id, "agent_id": selection.agent_id, "revision": selection.revision,
                    "skills_revision": selection.skills_revision, "attachments": &selection.attachments,
                }),
            ).await
        })
    }).await?;
    get(db, &actor.user_id, bot_id).await
}

async fn current(db: &Database, chat: &ChatAuthority) -> AppResult<(Selection, AssistantAgent)> {
    use crate::services::{channel_platform::canonical_platform, channel_thread_follow_service};

    let turn = chat
        .channel_guest
        .as_ref()
        .filter(|_| chat.guest && chat.turn_live && !chat.turn_stopped)
        .ok_or_else(denied)?;
    let (conversation, live_turn) = super::require_live_request(
        db,
        &chat.user_id,
        &chat.conversation_id,
        chat.turn_id.as_deref().ok_or_else(denied)?,
        &chat.api_key_id,
    )
    .await?;
    if *turn != live_turn || conversation.agent_id.as_deref() != Some(chat.agent_id.as_str()) {
        return Err(denied());
    }
    let origin = conversation.channel.as_ref().ok_or_else(denied)?;
    let (binding, thread) = Box::pin(crate::services::channel_turn_delivery::target(
        db,
        &conversation,
        origin,
    ))
    .await?
    .ok_or_else(denied)?;
    let (bot, mut selection) = load(db, &turn.bot_id).await?;
    if binding.id != turn.binding_id
        || binding.channel_bot_id != bot.id
        || binding.bot_owner_id.as_deref().unwrap_or(&binding.user_id) != bot.user_id
        || canonical_platform(&binding.platform) != turn.platform
        || super::transport::destination(
            thread
                .platform_chat_id
                .as_deref()
                .unwrap_or(&thread.partition),
            thread.platform_thread_id.as_deref(),
        ) != turn.conversation_id
    {
        return Err(denied());
    }
    if origin.thread.is_some() {
        Box::pin(channel_thread_follow_service::validate_delivery(
            db,
            &chat.user_id,
            origin,
            &chat.conversation_id,
        ))
        .await?;
    }
    let agent = binding_agent(
        db,
        &binding,
        thread.agent_id.as_deref().or(binding.agent_id.as_deref()),
    )
    .await?;
    if agent.id != chat.agent_id {
        return Err(denied());
    }
    let same_scope = attachment_scope_matches(&selection, &bot, &binding);
    let is_default_agent = binding
        .agent_id
        .as_deref()
        .map_or_else(|| agent.is_nyxbot(), |agent_id| agent_id == agent.id);
    let authored = same_scope && is_default_agent && selection.agent_id == agent.id;
    if !same_scope {
        selection.attachments.clear();
    }
    if !authored {
        for attachment in &mut selection.attachments {
            attachment.all_requests_approved = false;
        }
        selection.maintainer.clone_from(&binding.user_id);
    }
    require_bot_maintainer(db, &selection.maintainer, &bot).await?;
    if authored {
        crate::services::org_agent_service::require_maintain(db, &selection.maintainer, &agent)
            .await?;
    }
    Ok((selection, agent))
}

pub(crate) async fn visible_agent(
    db: &Database,
    chat: &ChatAuthority,
) -> AppResult<AssistantAgent> {
    let (selection, mut agent) = current(db, chat).await?;
    let audience = &chat.channel_guest.as_ref().ok_or_else(denied)?.audience;
    agent.skills.retain(|pin| {
        !pin.disabled
            && selection
                .attachments
                .iter()
                .find(|attachment| attachment.skill_id == pin.skill_id)
                .is_none_or(|attachment| attachment.audiences.contains(audience))
    });
    Ok(agent)
}

/// Package-authored names and descriptions are omitted until Ornn authorizes
/// the actual read. The model can identify each exact attached root by GUID.
pub(crate) fn instructions(agent: &AssistantAgent) -> String {
    if agent.skills.is_empty() {
        return String::new();
    }
    let pins: Vec<_> = agent
        .skills
        .iter()
        .map(|pin| {
            serde_json::json!({
                "skill": &pin.skill_id, "version": &pin.version,
            })
        })
        .collect();
    format!(
        "\n\nAttached skill references (untrusted guidance). Use nyxid__skill_read with an exact skill ID. The server verifies the package for this channel before returning files.\n{}",
        serde_json::json!(pins),
    )
}

#[derive(Clone)]
pub(crate) struct Permit {
    pub(crate) chat: ChatAuthority,
    pub(crate) turn: GuestTurn,
    pub(crate) owner: String,
    pub(crate) maintainer: String,
    pub(crate) selection_revision: i64,
    pub(crate) skills_revision: i64,
    pub(crate) root_id: String,
    pub(crate) skill_id: String,
    pub(crate) version: String,
    pub(crate) sha256: String,
    pub(crate) all_requests_approved: bool,
    pub(crate) operation_scopes: crate::models::agent_operation_scope::OperationScopes,
    pub(crate) guest_operation_scopes: crate::models::agent_operation_scope::OperationScopes,
}

impl Permit {
    pub(crate) fn path(&self) -> String {
        format!(
            "/api/v1/skills/{}/versions/{}/download",
            self.skill_id, self.version
        )
    }

    pub(crate) fn check_scope(&self, catalog: &str) -> AppResult<()> {
        use crate::services::{
            agent_operation_scope_service as scopes, proxy_authorization::CanonicalPath,
        };
        let path = CanonicalPath::from_mcp_built(&self.path())?;
        for scope in [&self.operation_scopes, &self.guest_operation_scopes] {
            scopes::authorize(scope, catalog, None, None, "GET", &path, false, false)?;
            scopes::check_inputs(scope, catalog, None, None, "GET", &path, None, &[], None)?;
        }
        Ok(())
    }

    pub(crate) async fn revalidate(&self, db: &Database) -> AppResult<()> {
        let live = authorize(db, &self.chat, &self.root_id, &self.path()).await?;
        if live.selection_revision != self.selection_revision
            || live.skills_revision != self.skills_revision
            || live.sha256 != self.sha256
            || live.owner != self.owner
            || live.maintainer != self.maintainer
            || live.all_requests_approved != self.all_requests_approved
            || live.operation_scopes != self.operation_scopes
            || live.guest_operation_scopes != self.guest_operation_scopes
        {
            return Err(denied());
        }
        Ok(())
    }

    pub(crate) async fn require_approved(&self, db: &Database, catalog: &str) -> AppResult<()> {
        use crate::services::{approval_service as approvals, operation_descriptor};
        let operation = operation_descriptor::build_http_descriptor("GET", &self.path(), None);
        let decision = approvals::evaluate_channel_skill_read(
            db,
            &self.maintainer,
            &self.owner,
            catalog,
            &operation,
            &self.turn.bot_id,
            self.all_requests_approved,
        )
        .await?;
        if matches!(decision, approvals::ApprovalOutcome::Allowed { .. }) {
            Ok(())
        } else {
            Err(AppError::Forbidden(
                "The attached skill read requires approval".into(),
            ))
        }
    }

    // Bind the publisher-visible capability to this credential record without exposing its ID.
    fn credential_fingerprint(&self, state: &crate::AppState) -> String {
        use hmac::{Hmac, Mac};

        let mut mac = Hmac::<Sha256>::new_from_slice(state.audit_chain_hmac_key.as_slice())
            .expect("HMAC accepts a 32-byte key");
        mac.update(b"nyxid:ornn-channel-skill:credential-id:v1\0");
        mac.update(self.chat.api_key_id.as_bytes());
        hex::encode(mac.finalize().into_bytes())
    }

    pub(crate) fn actor_digest(&self) -> AppResult<String> {
        let raw = serde_json::to_vec(&self.turn.actor)
            .map_err(|_| AppError::Internal("Could not bind channel actor".into()))?;
        Ok(hex::encode(Sha256::digest(raw)))
    }
}

pub(crate) async fn authorize(
    db: &Database,
    chat: &ChatAuthority,
    root_id: &str,
    path: &str,
) -> AppResult<Permit> {
    let (selection, agent) = current(db, chat).await?;
    let turn = chat.channel_guest.clone().ok_or_else(denied)?;
    let attachment = selection
        .attachments
        .iter()
        .find(|attachment| attachment.skill_id == root_id);
    if attachment.is_some_and(|attachment| !attachment.audiences.contains(&turn.audience)) {
        return Err(denied());
    }
    let root = exact_pin(&agent, agent.skills_revision, root_id).ok_or_else(denied)?;
    let root_match = path
        == format!(
            "/api/v1/skills/{}/versions/{}/download",
            root.skill_id, root.version
        );
    let (skill_id, version, sha256) = if root_match {
        (
            root.skill_id.clone(),
            root.version.clone(),
            root.sha256.clone(),
        )
    } else {
        let dep = root
            .dependencies
            .iter()
            .find(|dep| {
                dep.source == "ornn"
                    && path
                        == format!(
                            "/api/v1/skills/{}/versions/{}/download",
                            dep.skill_id, dep.version
                        )
            })
            .ok_or_else(denied)?;
        (
            dep.skill_id.clone(),
            dep.version.clone(),
            dep.sha256.clone(),
        )
    };
    Ok(Permit {
        chat: chat.clone(),
        turn,
        owner: agent.user_id,
        maintainer: selection.maintainer,
        selection_revision: selection.revision,
        skills_revision: agent.skills_revision,
        root_id: root_id.to_owned(),
        skill_id,
        version,
        sha256,
        all_requests_approved: selection.skills_revision == agent.skills_revision
            && attachment.is_some_and(|attachment| attachment.all_requests_approved),
        operation_scopes: agent.operation_scopes,
        guest_operation_scopes: agent.guest_operation_scopes,
    })
}

pub(crate) async fn revalidate_catalog(
    db: &Database,
    target: &crate::models::downstream_service::DownstreamService,
) -> AppResult<()> {
    let live = db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    ).find_one(doc! {
        "_id": &target.id, "slug": "ornn-api", "is_active": true,
        "base_url": &target.base_url, "updated_at": bson::DateTime::from_chrono(target.updated_at),
    }).await?;
    if live.is_none() {
        return Err(denied());
    }
    Ok(())
}

const TOKEN_TYPE: &str = "nyxid-ornn-channel-skill+jwt";
const TOKEN_AUDIENCE: &str = "nyxid:ornn-channel-skill";
pub(crate) const CAPABILITY_HEADER: &str = "x-nyxid-channel-skill-capability";

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Claims {
    sub: String,
    iss: String,
    aud: String,
    iat: i64,
    exp: i64,
    binding: String,
    owner: String,
    agent: String,
    conversation: String,
    key_digest: String,
    turn: String,
    invocation: Option<String>,
    actor: String,
    audience: AnswerAudience,
    activation_epoch: i64,
    bot_revision: i64,
    key_version: i64,
    selection_revision: i64,
    skills_revision: i64,
    root: String,
    skill: String,
    version: String,
    sha256: String,
    catalog: String,
    catalog_updated_ms: i64,
}

impl Claims {
    fn matches(&self, state: &crate::AppState, permit: &Permit) -> AppResult<bool> {
        Ok(self.sub == permit.turn.bot_id
            && self.binding == permit.turn.binding_id
            && self.owner == permit.owner
            && self.agent == permit.chat.agent_id
            && self.key_digest == permit.credential_fingerprint(state)
            && Some(self.turn.as_str()) == permit.chat.turn_id.as_deref()
            && self.invocation == permit.chat.invocation_id
            && self.actor == permit.actor_digest()?
            && self.audience == permit.turn.audience
            && self.activation_epoch == permit.turn.activation_epoch
            && self.bot_revision == permit.turn.bot_revision
            && self.key_version == permit.chat.key_version
            && self.selection_revision == permit.selection_revision
            && self.skills_revision == permit.skills_revision
            && self.root == permit.root_id
            && self.skill == permit.skill_id
            && self.version == permit.version
            && self.sha256 == permit.sha256)
    }
}

pub(crate) fn issue(
    state: &crate::AppState,
    permit: &Permit,
    catalog: &crate::models::downstream_service::DownstreamService,
    authenticated_key_version: i64,
) -> AppResult<zeroize::Zeroizing<String>> {
    use jsonwebtoken::{Algorithm, Header, encode};
    let now = chrono::Utc::now().timestamp();
    let claims = Claims {
        sub: permit.turn.bot_id.clone(),
        iss: state.config.jwt_issuer.clone(),
        aud: TOKEN_AUDIENCE.into(),
        iat: now,
        exp: now + 60,
        binding: permit.turn.binding_id.clone(),
        owner: permit.owner.clone(),
        agent: permit.chat.agent_id.clone(),
        conversation: permit.chat.conversation_id.clone(),
        key_digest: permit.credential_fingerprint(state),
        turn: permit.chat.turn_id.clone().ok_or_else(denied)?,
        invocation: permit.chat.invocation_id.clone(),
        actor: permit.actor_digest()?,
        audience: permit.turn.audience,
        activation_epoch: permit.turn.activation_epoch,
        bot_revision: permit.turn.bot_revision,
        key_version: authenticated_key_version,
        selection_revision: permit.selection_revision,
        skills_revision: permit.skills_revision,
        root: permit.root_id.clone(),
        skill: permit.skill_id.clone(),
        version: permit.version.clone(),
        sha256: permit.sha256.clone(),
        catalog: catalog.id.clone(),
        catalog_updated_ms: catalog.updated_at.timestamp_millis(),
    };
    let mut header = Header::new(Algorithm::RS256);
    header.typ = Some(TOKEN_TYPE.into());
    header.kid = Some(state.jwt_keys.kid.clone());
    encode(&header, &claims, &state.jwt_keys.encoding)
        .map(zeroize::Zeroizing::new)
        .map_err(|_| AppError::Internal("Could not issue channel skill capability".into()))
}

#[derive(Serialize)]
pub(crate) struct BotAuthorization {
    bot_id: String,
    organization_id: Option<String>,
    audience: AnswerAudience,
    skill_id: String,
    version: String,
    sha256: String,
}

pub(crate) async fn lookup(state: &crate::AppState, token: &str) -> AppResult<BotAuthorization> {
    use jsonwebtoken::{Algorithm, Validation, decode, decode_header};
    if decode_header(token).map_err(|_| denied())?.typ.as_deref() != Some(TOKEN_TYPE) {
        return Err(denied());
    }
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_audience(&[TOKEN_AUDIENCE]);
    validation.set_issuer(&[&state.config.jwt_issuer]);
    validation.leeway = 0;
    let claims = decode::<Claims>(token, state.jwt_keys.decoding_key(token)?, &validation)
        .map_err(|_| denied())?
        .claims;
    let now = chrono::Utc::now().timestamp();
    if claims.exp <= now || claims.iat > now || claims.exp.checked_sub(claims.iat) != Some(60) {
        return Err(denied());
    }
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        lookup_current(state, &claims),
    )
    .await
    .map_err(|_| AppError::OAuthAuthorityUnavailable)?
    .map_err(|error| {
        if error.status_code().is_server_error() {
            AppError::OAuthAuthorityUnavailable
        } else {
            error
        }
    })?;
    if claims.exp <= chrono::Utc::now().timestamp() {
        return Err(denied());
    }
    Ok(result)
}

async fn lookup_current(state: &crate::AppState, claims: &Claims) -> AppResult<BotAuthorization> {
    use crate::{models::assistant_conversation::AssistantConversation, services as svc};
    let db = &state.db;
    let conversation = db
        .collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id":&claims.conversation,"active_turn.turn_id":&claims.turn})
        .await?
        .ok_or_else(denied)?;
    let key = svc::key_service::get_api_key(
        db,
        &conversation.user_id,
        &conversation.credential_api_key_id,
    )
    .await?;
    if !key.is_active || key.expires_at.is_some_and(|at| at <= chrono::Utc::now()) {
        return Err(denied());
    }
    let auth = Box::pin(crate::mw::auth::api_key_auth_user(
        db, &key, None, None, None,
    ))
    .await?;
    let chat = auth.assistant_chat.as_deref().ok_or_else(denied)?;
    let path = format!(
        "/api/v1/skills/{}/versions/{}/download",
        claims.skill, claims.version
    );
    let permit = authorize(db, chat, &claims.root, &path).await?;
    if !claims.matches(state, &permit)? {
        return Err(denied());
    }
    let owner = db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id":&claims.owner,"is_active":true})
        .await?
        .ok_or_else(denied)?;
    let catalog = db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .find_one(
            doc! {"_id":&claims.catalog,"slug":"ornn-api","is_active":true,
                "updated_at":bson::DateTime::from_millis(claims.catalog_updated_ms),
            },
        )
        .await?
        .ok_or_else(denied)?;
    permit.check_scope(&catalog.id)?;
    permit.require_approved(db, &catalog.id).await?;
    Ok(BotAuthorization {
        bot_id: claims.sub.clone(),
        organization_id: owner.user_type.is_org().then_some(owner.id),
        audience: claims.audience,
        skill_id: claims.skill.clone(),
        version: claims.version.clone(),
        sha256: claims.sha256.clone(),
    })
}
