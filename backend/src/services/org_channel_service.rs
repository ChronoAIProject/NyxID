//! Organization channel authority. The linking admin is a transport principal;
//! execution is bound to one verified member or a restricted org-owned guest.
use bson::{Document, doc};
use mongodb::{ClientSession, Database};
use std::sync::Arc;

use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_agent::AssistantAgent,
        assistant_conversation::{AssistantConversation, ChannelOrigin},
        channel_bot::ChannelBot,
        nyxbot_channel::{NyxbotChannel, NyxbotThread},
        org_channel::{Binding, Execution, GuestPolicy, LINKS, SenderLink},
        user_service::UserService,
    },
};

pub(crate) mod adoption;
pub(crate) mod budget;
pub(crate) mod lifecycle;
pub(crate) mod links;
pub(crate) mod management;
#[cfg(test)]
pub(crate) mod tests;
pub(crate) mod transcript;

pub const GUEST_GRANTS_READINESS: &str = "nyxbot:org-guest-grants-v1";

pub const FLAG: &str = "nyxbot:org-specialist-channels";
pub const UNAVAILABLE: &str = "This assistant is unavailable right now. Please try again later.";

pub(crate) fn denied() -> AppError {
    AppError::Forbidden("Organization channel authority is no longer available".into())
}

pub(crate) async fn enabled(db: &Database, admin: &str) -> AppResult<()> {
    if !super::feature_flag_service::personal_flag_enabled(db, admin, FLAG).await? {
        return Err(denied());
    }
    super::security_readiness_service::require_serving(db, FLAG).await
}

/// Readiness is checked only on organization guest paths, never ordinary billing.
pub(crate) async fn guest_grants_ready(db: &Database) -> AppResult<bool> {
    match super::security_readiness_service::serving_report(db, GUEST_GRANTS_READINESS).await {
        Ok(report) => Ok(report.serving_ready),
        Err(AppError::SecurityReadinessUnavailable) => Ok(false),
        Err(error) => Err(error),
    }
}

/// Binding selection only: operation/guest classification remains in the shared
/// operation-scope evaluators. A legacy digest mismatch never widens a key.
pub(crate) fn guest_service_selected(
    agent: &AssistantAgent,
    policy: &GuestPolicy,
    ready: bool,
    id: &str,
) -> bool {
    agent.grants.service_ids.iter().any(|granted| granted == id)
        && ((ready && policy.follows_agent_grants)
            || (policy.service_ids.iter().any(|selected| selected == id)
                && policy
                    .service_authority
                    .get(id)
                    .is_some_and(|digest| *digest == management::service_digest(agent, id))))
}

/// Only this module can construct the billing exception. A public actor or an
/// arbitrary organization-owned resource never creates this execution lane.
#[derive(Clone, Debug)]
pub struct GuestPayer {
    org_id: String,
    channel_id: String,
    revision: String,
    sender_hash: String,
    agent_id: String,
    origin: ChannelOrigin,
}
impl GuestPayer {
    pub(crate) fn usage(&self) -> crate::models::org_channel::GuestUsage {
        crate::models::org_channel::GuestUsage {
            channel_id: self.channel_id.clone(),
            sender_hash: self.sender_hash.clone(),
        }
    }
    pub(crate) fn org_id(&self) -> &str {
        &self.org_id
    }
}

#[derive(Clone)]
pub struct Access {
    channel: NyxbotChannel,
    execution: Execution,
    origin: ChannelOrigin,
    agent: AssistantAgent,
    member_private_source: bool,
    guest_grants_ready: bool,
}
impl std::fmt::Debug for Access {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OrgChannelAccess { verified }")
    }
}
impl Access {
    pub(crate) fn channel(&self) -> &NyxbotChannel {
        &self.channel
    }
    pub(crate) fn execution(&self) -> &Execution {
        &self.execution
    }
    pub(crate) fn guest_service_selected(&self, id: &str) -> bool {
        self.channel.org_specialist.as_ref().is_some_and(|b| {
            guest_service_selected(&self.agent, &b.guests, self.guest_grants_ready, id)
        })
    }
    pub(crate) fn agent(&self) -> &AssistantAgent {
        &self.agent
    }
    pub(crate) fn origin(&self) -> &ChannelOrigin {
        &self.origin
    }
    pub(crate) fn guest_payer(&self) -> Option<GuestPayer> {
        self.execution.guest.then(|| GuestPayer {
            org_id: self.execution.org_id.clone(),
            channel_id: self.channel.id.clone(),
            revision: self.execution.revision.clone(),
            sender_hash: self.execution.sender_hash.clone(),
            agent_id: self.execution.agent_id.clone(),
            origin: self.origin.clone(),
        })
    }
    pub(crate) fn matches(&self, actor: &str, owner: &str) -> bool {
        self.execution.actor_id == actor && self.execution.org_id == owner
    }
    pub(crate) fn allows_service(&self, service: &UserService) -> bool {
        service.user_id == self.execution.org_id
            && service.is_active
            && (!self.execution.guest
                || (service.node_id.is_none()
                    && service.service_type == "http"
                    && self.guest_service_selected(&service.id)))
    }

    pub(crate) fn check_conversation(&self, row: &AssistantConversation) -> AppResult<()> {
        let origin = row
            .channel
            .as_ref()
            .or(row.reply_channel.as_ref())
            .ok_or_else(denied)?;
        if !self.matches(
            &row.user_id,
            row.agent_owner_id.as_deref().unwrap_or(&row.user_id),
        ) || row.agent_id.as_deref() != Some(self.agent.id.as_str())
            || origin.org_execution.as_deref() != Some(&self.execution)
            || origin.nyxbot_channel_id != self.channel.id
            || origin.partition != self.origin.partition
            || row.collaboration.is_some()
            || row.group_id.is_some()
            || row.platform_thread.is_some()
            || (row.guest_turn != self.execution.guest && !row.guest_model_setup)
            || (row.guest_model_setup && (!self.execution.guest || row.guest_turn))
        {
            return Err(denied());
        }
        Ok(())
    }

    /// A member's private agent home can accept sources from multiple bound
    /// bots. Its latest reply preference is not the authority of queued work.
    /// Only a live private member source may replace that preference for this
    /// check; group/guest/follow lanes retain their exact persisted binding.
    pub(crate) fn check_source_conversation(&self, row: &AssistantConversation) -> AppResult<()> {
        if self.member_private_source && row.channel.is_none() {
            let mut scoped = row.clone();
            scoped.reply_channel = Some(self.origin.clone());
            self.check_conversation(&scoped)
        } else {
            self.check_conversation(row)
        }
    }
}

pub(crate) async fn require_admin(db: &Database, actor: &str, org: &str) -> AppResult<()> {
    if !matches!(
        super::org_agent_service::access(db, actor, org).await?,
        super::org_service::OwnerAccess::AsOrgAdmin { .. }
    ) {
        return Err(denied());
    }
    Ok(())
}

pub(crate) async fn link_target(
    db: &Database,
    actor: &str,
    bot: &ChannelBot,
    agent: &AssistantAgent,
) -> AppResult<Option<Binding>> {
    if agent.user_id == actor {
        return Ok(None);
    }
    require_target(db, actor, agent).await?;
    if bot.user_id != agent.user_id {
        return Err(AppError::ValidationError(
            "The bot and specialist must be owned by the same organization".into(),
        ));
    }
    if !bot.is_active {
        return Err(AppError::ValidationError(
            "The organization bot is inactive".into(),
        ));
    }
    Ok(Some(Binding {
        version: 1,
        org_id: agent.user_id.clone(),
        revision: uuid::Uuid::new_v4().to_string(),
        guests: GuestPolicy {
            follows_agent_grants: guest_grants_ready(db).await?,
            // Conservative compatibility fields for an old reader.
            daily_credits: Some(management::default_daily_credits()),
            ..Default::default()
        },
    }))
}

/// Preserve the existing closed/typed refusal before resolving any bot on
/// disabled replicas. Human routes and NyxBot preflight share this same gate.
pub(crate) async fn require_target(
    db: &Database,
    actor: &str,
    agent: &AssistantAgent,
) -> AppResult<()> {
    if agent.user_id == actor {
        return Ok(());
    }
    if !super::feature_flag_service::personal_flag_enabled(db, actor, FLAG).await? {
        return Err(AppError::organization_agent_channels_disabled());
    }
    enabled(db, actor).await?;
    if !matches!(
        super::org_agent_service::access(db, actor, &agent.user_id).await?,
        super::org_service::OwnerAccess::AsOrgAdmin { .. }
    ) {
        return Err(AppError::Forbidden(
            "Only an organization admin can connect this specialist to a channel bot".into(),
        ));
    }
    if agent.is_nyxbot() || agent.destroyed_at.is_some() {
        return Err(AppError::ValidationError(
            "Choose an active organization specialist".into(),
        ));
    }
    Ok(())
}

/// Authentication uses opaque platform IDs; no display name can link a person.
pub(crate) fn sender_hash(channel: &str, sender: &str) -> String {
    crate::crypto::token::hash_token(&format!("org-channel-sender:{channel}:{sender}"))
}

pub(crate) fn sender_identity(channel: &NyxbotChannel, sender: &str) -> String {
    let namespace = channel
        .org_specialist
        .as_ref()
        .and_then(|binding| binding.guests.continuity_channel_id.as_deref())
        .unwrap_or(&channel.id);
    sender_hash(namespace, sender)
}

pub(crate) async fn sender(
    db: &Database,
    row: &NyxbotChannel,
    sender: &str,
    eligible: bool,
    agent_id: &str,
) -> AppResult<(Execution, bool)> {
    let binding = row.org_specialist.as_ref().ok_or_else(denied)?;
    let hash = sender_identity(row, sender);
    let link = db
        .collection::<SenderLink>(LINKS)
        .find_one(doc! {"channel_id":&row.id,"sender_hash":&hash})
        .await?;
    let (actor, link_id, link_revision, admin, guest) = match link {
        Some(link) => {
            // A removed member is never silently downgraded to an org-paid guest.
            if !eligible || !link.active || link.binding_revision != binding.revision {
                return Err(denied());
            }
            let acl = super::org_agent_service::access(db, &link.user_id, &binding.org_id).await?;
            if !super::org_agent_service::can_use(&acl) {
                return Err(denied());
            }
            let admin = matches!(acl, super::org_service::OwnerAccess::AsOrgAdmin { .. });
            (
                link.user_id,
                Some(link.id),
                Some(link.revision),
                admin,
                false,
            )
        }
        None if binding.guests.enabled => {
            // Rebuilds require fresh verification, but a previously claimed or
            // erased member identity must not become an organization-paid guest.
            if db
                .collection::<Document>(LINKS)
                .find_one(doc! {"org_id":&binding.org_id,"sender_hash":&hash})
                .await?
                .is_some()
            {
                return Err(denied());
            }
            (binding.org_id.clone(), None, None, false, true)
        }
        None => return Err(denied()),
    };
    Ok((
        Execution {
            version: 1,
            org_id: binding.org_id.clone(),
            revision: binding.revision.clone(),
            actor_id: actor,
            agent_id: agent_id.into(),
            guest,
            guest_revision: guest.then(|| binding.guests.revision.clone()),
            sender_hash: hash,
            link_id,
            link_revision,
        },
        admin,
    ))
}

pub(crate) async fn live_doc(
    db: &Database,
    session: &mut ClientSession,
    collection: &str,
    filter: Document,
    fence: bool,
) -> AppResult<Document> {
    let rows = db.collection::<Document>(collection);
    let found = if fence {
        rows.find_one_and_update(filter, doc! {"$inc":{"org_channel_fence":1_i64}})
            .session(session)
            .await?
    } else {
        rows.find_one(filter).session(session).await?
    };
    found.ok_or_else(denied)
}

pub(crate) async fn validate(
    db: &Database,
    origin: &ChannelOrigin,
    actor: &str,
    agent: &str,
) -> AppResult<Arc<super::org_agent_service::RequestAccess>> {
    let mut session = db.client().start_session().await?;
    let access = validate_in_session(db, &mut session, origin, actor, agent, false).await?;
    super::org_agent_service::RequestAccess::from_channel(db, access).await
}

pub(crate) async fn validate_in_session(
    db: &Database,
    session: &mut ClientSession,
    origin: &ChannelOrigin,
    actor: &str,
    agent_id: &str,
    fence: bool,
) -> AppResult<Access> {
    let execution = origin.org_execution.as_deref().ok_or_else(denied)?;
    if execution.version != 1 || execution.actor_id != actor || execution.agent_id != agent_id {
        return Err(denied());
    }
    let row = live_doc(
        db,
        session,
        "nyxbot_channels",
        doc! {"_id":&origin.nyxbot_channel_id,
        "status":"active","org_specialist.version":1,
        "org_specialist.org_id":&execution.org_id,"org_specialist.revision":&execution.revision,
        "bot_owner_id":&execution.org_id},
        fence,
    )
    .await?;
    let channel: NyxbotChannel = bson::from_document(row).map_err(|_| denied())?;
    enabled(db, &channel.user_id).await?;
    live_doc(
        db,
        session,
        "users",
        doc! {"_id":&execution.org_id,"is_active":true,"user_type":"org"},
        fence,
    )
    .await?;
    live_doc(
        db,
        session,
        "users",
        doc! {"_id":&channel.user_id,"is_active":true},
        fence,
    )
    .await?;
    live_doc(
        db,
        session,
        "org_memberships",
        doc! {"org_user_id":&execution.org_id,"member_user_id":&channel.user_id,
        "revoked_at":bson::Bson::Null,"role":"admin"},
        fence,
    )
    .await?;
    live_doc(
        db,
        session,
        "channel_bots",
        doc! {"_id":&channel.channel_bot_id,"user_id":&execution.org_id,
        "is_active":true,"status":{"$ne":"suspended"}},
        fence,
    )
    .await?;
    let agent: AssistantAgent = bson::from_document(
        live_doc(
            db,
            session,
            "assistant_agents",
            doc! {"_id":agent_id,
            "user_id":&execution.org_id,"kind":"specialist","destroyed_at":bson::Bson::Null},
            fence,
        )
        .await?,
    )
    .map_err(|_| denied())?;
    let mut thread: NyxbotThread = bson::from_document(
        live_doc(
            db,
            session,
            "nyxbot_threads",
            doc! {
            "channel_id":&channel.id,"partition":&origin.partition,"user_id":&channel.user_id},
            fence,
        )
        .await?,
    )
    .map_err(|_| denied())?;
    if origin.thread.is_some() {
        thread = bson::from_document(
            live_doc(
                db,
                session,
                "nyxbot_threads",
                doc! {"_id":&thread.follow.settings_chat_id,
                "channel_id":&channel.id,"user_id":&channel.user_id},
                fence,
            )
            .await?,
        )
        .map_err(|_| denied())?;
    }
    if thread
        .agent_id
        .as_ref()
        .or(channel.agent_id.as_ref())
        .map(String::as_str)
        != Some(agent_id)
    {
        return Err(denied());
    }
    let guest_grants_ready = execution.guest && guest_grants_ready(db).await?;
    if execution.guest {
        let policy = &channel.org_specialist.as_ref().ok_or_else(denied)?.guests;
        if actor != execution.org_id
            || !policy.enabled
            || execution.guest_revision.as_deref() != Some(policy.revision.as_str())
            || policy.revision.is_empty()
            || (!guest_grants_ready
                && policy
                    .daily_credits
                    .is_none_or(|c| c <= crate::models::credits::Credits::ZERO))
            || execution.link_id.is_some()
            || execution.link_revision.is_some()
            || (thread.kind.as_deref() == Some("private")
                && channel.private_chats.as_deref() != Some("everyone"))
            || (thread.kind.as_deref() != Some("private")
                && !matches!(thread.members.as_deref(), Some("everyone"))
                && !(thread.members.is_none() && thread.owner_seen))
        {
            return Err(denied());
        }
        // A claimed member identity must never be converted into guest authority.
        if db
            .collection::<Document>(LINKS)
            .find_one(doc! {"org_id":&execution.org_id,"sender_hash":&execution.sender_hash})
            .session(&mut *session)
            .await?
            .is_some()
        {
            return Err(denied());
        }
        if !(guest_grants_ready && policy.follows_agent_grants)
            && policy.service_authority.iter().any(|(id, digest)| {
                !agent.grants.service_ids.contains(id)
                    || management::service_digest(&agent, id) != *digest
            })
        {
            return Err(denied());
        }
        budget::require_wallet(db, &execution.org_id).await?;
    } else {
        live_doc(
            db,
            session,
            "users",
            doc! {"_id":actor,"is_active":true,"user_type":{"$ne":"org"}},
            fence,
        )
        .await?;
        live_doc(
            db,
            session,
            "org_memberships",
            doc! {"org_user_id":&execution.org_id,"member_user_id":actor,
            "revoked_at":bson::Bson::Null,"role":{"$in":["admin","member"]}},
            fence,
        )
        .await?;
        live_doc(db, session, LINKS, doc!{"_id":&execution.link_id,"channel_id":&channel.id,"user_id":actor,
            "active":true,"revision":&execution.link_revision,"binding_revision":&execution.revision,"sender_hash":&execution.sender_hash}, fence).await?;
    }
    // Routes and keys are transport authority even for native sources. Fence
    // their revocation in the same transaction as a turn or billed admission.
    if fence {
        super::platform_bot_authority::fence_personal_routes(
            db,
            &channel.channel_bot_id,
            &execution.org_id,
            session,
        )
        .await?;
    } else if !super::platform_bot_authority::permits_personal_routes(db, &channel.channel_bot_id)
        .await?
    {
        return Err(denied());
    }
    live_doc(
        db,
        session,
        "api_keys",
        doc! {"_id":&channel.route_api_key_id,"user_id":&execution.org_id,"is_active":true,
        "$or":[{"expires_at":null},{"expires_at":{"$gt":bson::DateTime::now()}}]},
        fence,
    )
    .await?;
    live_doc(db,session,crate::models::channel_conversation::COLLECTION_NAME,doc!{"_id":&channel.route_id,"user_id":&execution.org_id,
        "channel_bot_id":&channel.channel_bot_id,"agent_api_key_id":&channel.route_api_key_id,"is_active":true,"retired_by_transfer":{"$ne":true}},fence).await?;
    Ok(Access {
        guest_grants_ready,
        member_private_source: !execution.guest
            && origin.thread.is_none()
            && thread.kind.as_deref() == Some("private"),
        channel,
        execution: execution.clone(),
        origin: origin.clone(),
        agent,
    })
}

pub(crate) fn origin(row: &AssistantConversation) -> Option<&ChannelOrigin> {
    row.channel
        .as_ref()
        .filter(|o| o.org_execution.is_some())
        .or_else(|| {
            row.reply_channel
                .as_ref()
                .filter(|o| o.org_execution.is_some())
        })
}

pub(crate) async fn conversation_access(
    db: &Database,
    row: &AssistantConversation,
) -> AppResult<Option<Arc<super::org_agent_service::RequestAccess>>> {
    match origin(row) {
        Some(origin) => {
            let access = validate(
                db,
                origin,
                &row.user_id,
                row.agent_id.as_deref().ok_or_else(denied)?,
            )
            .await?;
            access.check_conversation(row)?;
            if origin.thread.is_some() {
                let conversation = follow_conversation(db, row, origin, None).await?;
                Box::pin(super::channel_thread_follow_service::validate_delivery(
                    db,
                    &row.user_id,
                    origin,
                    &conversation,
                ))
                .await?;
            }
            Ok(Some(access))
        }
        None => Ok(None),
    }
}

/// Hidden setup owns its own credential and settlement. Only its follow check
/// uses the originating guest's admitted lane, bound to the exact sender/policy.
pub(crate) async fn follow_conversation(
    db: &Database,
    row: &AssistantConversation,
    origin: &ChannelOrigin,
    session: Option<&mut ClientSession>,
) -> AppResult<String> {
    let Some(e) = origin
        .org_execution
        .as_ref()
        .filter(|_| row.guest_model_setup && origin.thread.is_some())
    else {
        return Ok(row.id.clone());
    };
    if !e.guest
        || row.guest_turn
        || e.actor_id != row.user_id
        || row.agent_id.as_deref() != Some(&e.agent_id)
    {
        return Err(denied());
    }
    let lanes = db.collection::<Document>(crate::models::org_channel::LANES);
    let lookup = lanes.find_one(doc! {
        "channel_id":&origin.nyxbot_channel_id,"partition":&origin.partition,"actor_id":&row.user_id,
        "agent_id":&e.agent_id,"binding_revision":&e.revision,"guest_revision":&e.guest_revision,
        "sender_hash":&e.sender_hash
    });
    let lane = match session {
        Some(session) => lookup.session(session).await?,
        None => lookup.await?,
    }
    .ok_or_else(denied)?;
    Ok(lane
        .get_str("conversation_id")
        .map_err(|_| denied())?
        .to_owned())
}

pub(crate) async fn guest_rate(
    db: &Database,
    execution: &Execution,
    channel: &str,
) -> AppResult<bool> {
    if !execution.guest {
        return Ok(true);
    }
    Ok(super::coordination_service::RateWindowStore::admit(
        db,
        "org-channel-guest",
        &format!("{channel}:{}", execution.sender_hash),
        6,
        std::time::Duration::from_secs(60),
    )
    .await?
    .allowed)
}

/// Provider ownership stays on the linking admin; conversation ownership is
/// an independently verified member or org guest actor.
pub(crate) fn event_actor(event: &crate::models::nyxbot_channel::NyxbotEvent) -> &str {
    event
        .delivery
        .as_ref()
        .and_then(|d| d.origin.org_execution.as_ref())
        .map_or(event.user_id.as_str(), |e| e.actor_id.as_str())
}

pub(crate) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use crate::models::org_channel::*;
    use mongodb::{IndexModel, options::IndexOptions};
    for (collection, keys) in [
        (LINKS, doc! {"channel_id":1,"sender_hash":1}),
        (
            LANES,
            doc! {"channel_id":1,"partition":1,"actor_id":1,"agent_id":1,"binding_revision":1,"guest_revision":1,"sender_hash":1},
        ),
    ] {
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
    }
    db.collection::<Document>(CHALLENGES)
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
    for collection in [LINKS, LANES, MESSAGES, BUDGETS, HOLDS, CHALLENGES] {
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"org_id":1,"channel_id":1})
                    .build(),
            )
            .await?;
    }
    db.collection::<Document>(LINKS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"org_id":1,"sender_hash":1})
                .build(),
        )
        .await?;
    db.collection::<Document>(MESSAGES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"user_id":1,"group_id":1,"seq":1})
                .build(),
        )
        .await?;
    Ok(())
}

pub(crate) async fn project_sender(
    db: &Database,
    row: &NyxbotChannel,
    sender_id: &str,
    human: bool,
    agent: &str,
) -> AppResult<(NyxbotChannel, bool)> {
    if row.org_specialist.is_none() {
        return Ok((
            row.clone(),
            row.owner_sender_ids.iter().any(|id| id == sender_id),
        ));
    }
    let (identity, admin) = sender(db, row, sender_id, human, agent).await?;
    let mut projected = row.clone();
    projected.owner_sender_ids = if identity.guest {
        Vec::new()
    } else {
        vec![sender_id.into()]
    };
    Ok((projected, admin))
}

/// Only confirmed authority loss permits retiring a persisted channel. Rollout
/// controls suspend work; dependency errors propagate without changing authority.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OrgAccess {
    Holds,
    Lost,
    Unavailable,
}

pub(crate) async fn transport_access(db: &Database, row: &NyxbotChannel) -> AppResult<OrgAccess> {
    let Some(b) = &row.org_specialist else {
        return Ok(OrgAccess::Holds);
    };
    if b.version != 1 || row.bot_owner_id.as_ref() != Some(&b.org_id) {
        return Ok(OrgAccess::Lost);
    }
    if db
        .collection::<Document>("assistant_agents")
        .find_one(
            doc! {"_id":&row.agent_id,"user_id":&b.org_id,"kind":"specialist","destroyed_at":null},
        )
        .await?
        .is_none()
        || db
            .collection::<Document>("channel_bots")
            .find_one(doc! {"_id":&row.channel_bot_id,"user_id":&b.org_id,"is_active":true})
            .await?
            .is_none()
    {
        return Ok(OrgAccess::Lost);
    }
    match enabled(db, &row.user_id).await {
        Ok(()) => Ok(OrgAccess::Holds),
        // Flag-off refuses management with 403, but only suspends a saved transport.
        Err(AppError::Forbidden(_) | AppError::SecurityReadinessUnavailable) => {
            Ok(OrgAccess::Unavailable)
        }
        Err(error) => Err(error),
    }
}

/// Management uses the acting admin as well as the persisted transport admin.
/// Both memberships must serialize against demotion in the mutation transaction.
pub(crate) async fn fence_management(
    db: &Database,
    tx: &mut ClientSession,
    row: &NyxbotChannel,
    actor: &str,
) -> AppResult<()> {
    let b = row.org_specialist.as_ref().ok_or_else(denied)?;
    enabled(db, &row.user_id).await?;
    live_doc(
        db,
        tx,
        "users",
        doc! {"_id":&b.org_id,"is_active":true,"user_type":"org"},
        true,
    )
    .await?;
    for person in [actor, row.user_id.as_str()] {
        live_doc(db, tx, "users", doc! {"_id":person,"is_active":true}, true).await?;
        live_doc(
            db,
            tx,
            "org_memberships",
            doc! {"org_user_id":&b.org_id,"member_user_id":person,"role":"admin","revoked_at":null},
            true,
        )
        .await?;
    }
    live_doc(
        db,
        tx,
        "channel_bots",
        doc! {"_id":&row.channel_bot_id,"user_id":&b.org_id,"is_active":true},
        true,
    )
    .await?;
    live_doc(
        db,
        tx,
        "assistant_agents",
        doc! {"_id":&row.agent_id,"user_id":&b.org_id,"kind":"specialist","destroyed_at":null},
        true,
    )
    .await?;
    live_doc(db,tx,"nyxbot_channels",doc!{"_id":&row.id,"org_specialist.revision":&b.revision,"status":{"$in":["active","pending"]}},true).await?;
    Ok(())
}
