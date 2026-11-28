//! Server-authored admission and rollback fences for channel guest work.
pub(crate) mod assignment;
pub(crate) mod google_sheets;
pub(crate) mod guest_tools;
pub(crate) mod personal_records;
pub(crate) mod policy;
pub(crate) mod relay;
pub(crate) mod setup;
pub(crate) mod skills;
pub(crate) mod transport;

#[cfg(test)]
mod discovery_tests;
#[cfg(test)]
mod sheets_runtime_tests;
#[cfg(test)]
mod test_support;

use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::feature_flag_service as flags;
use crate::errors::{AppError, AppResult};

pub const FLAG: &str = "nyxbot:guest-audience-v1";
const ACTIVATION: &str = "channel_guest_activation";
const ACTIVATION_FENCES: &str = "channel_guest_activation_fences";
const FENCE_COUNT: u8 = 16;
const WORK: &str = "channel_guest_work_v1";
const OBSERVATIONS: &str = "channel_guest_verified_observations";
const CONVERSATION_PREFIX: &str = "nyxag-";

pub use crate::models::channel_guest_access::{
    AnswerAudience, GuestActor, GuestAssurance, GuestTurn,
};

/// Constructed only from a verified ingress and the locally resolved binding.
pub(crate) struct VerifiedGuestInput<'a> {
    pub bot_id: &'a str,
    pub binding_id: &'a str,
    pub platform: &'a str,
    pub conversation_id: &'a str,
    pub source_event_id: &'a str,
    pub sender_id: Option<&'a str>,
    pub audience: AnswerAudience,
    pub shared_room_verified: bool,
    pub meta_grant_id: Option<&'a str>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum GuestMode {
    Legacy,
    AddOn {
        activation_epoch: i64,
        bot_revision: i64,
    },
}

/// Stored on the authenticated source before owner or guest admission.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub(crate) enum HostedAdmission {
    Legacy,
    AddOn {
        activation_epoch: i64,
        bot_revision: i64,
    },
    Unavailable,
}

impl HostedAdmission {
    pub(crate) fn from_selection(selection: AppResult<GuestMode>) -> Self {
        match selection {
            Ok(GuestMode::Legacy) => Self::Legacy,
            Ok(GuestMode::AddOn {
                activation_epoch,
                bot_revision,
            }) => Self::AddOn {
                activation_epoch,
                bot_revision,
            },
            Err(error) => {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Channel guest selection unavailable"
                );
                Self::Unavailable
            }
        }
    }

    pub(crate) fn guest_mode(self) -> Option<GuestMode> {
        match self {
            Self::Legacy => Some(GuestMode::Legacy),
            Self::AddOn {
                activation_epoch,
                bot_revision,
            } => Some(GuestMode::AddOn {
                activation_epoch,
                bot_revision,
            }),
            Self::Unavailable => None,
        }
    }
}

pub(crate) enum GuestAdmission {
    Legacy,
    AddOn(Box<GuestTurn>),
}

#[derive(Deserialize)]
struct Activation {
    epoch: i64,
    enabled: bool,
    pilot_bot_ids: Option<Vec<String>>,
    #[serde(default)]
    fleet: bool,
}

fn unavailable() -> AppError {
    AppError::SecurityReadinessUnavailable
}

pub(crate) fn obsolete() -> AppError {
    AppError::Forbidden("Channel guest admission is no longer valid".into())
}

fn activation_collection(db: &Database) -> mongodb::Collection<Activation> {
    db.collection(ACTIVATION)
}

pub(crate) async fn update_activation_in_session(
    db: &Database,
    session: &mut ClientSession,
    enabled: bool,
) -> AppResult<()> {
    use mongodb::options::ReturnDocument;
    let state = activation_collection(db)
        .find_one_and_update(
            doc! {"_id": FLAG},
            doc! {"$inc": {"epoch": 1_i64}, "$set": {"enabled": enabled}},
        )
        .return_document(ReturnDocument::After)
        .upsert(true)
        .session(&mut *session)
        .await?
        .ok_or_else(unavailable)?;
    let fences = db.collection::<Document>(ACTIVATION_FENCES);
    if state.epoch == 1 {
        let rows = (0..FENCE_COUNT).map(|stripe| {
            doc! {"_id": i32::from(stripe), "epoch": state.epoch, "enabled": enabled}
        });
        fences.insert_many(rows).session(session).await?;
    } else {
        let updated = fences
            .update_many(
                doc! {"_id": {"$gte": 0, "$lt": i32::from(FENCE_COUNT)}},
                doc! {"$set": {"epoch": state.epoch, "enabled": enabled}},
            )
            .session(session)
            .await?;
        if enabled && updated.matched_count != u64::from(FENCE_COUNT) {
            return Err(unavailable());
        }
    }
    Ok(())
}

pub(crate) async fn select_mode(db: &Database, bot_id: &str) -> AppResult<GuestMode> {
    let bot = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(active_bot(bot_id))
        .projection(
            doc! {"is_active": 1, "status": 1, "guest_access_v1": 1, "guest_access_v1_revision": 1},
        )
        .await?;
    select_mode_for_snapshot(db, bot_id, bot.as_ref()).await
}

pub(super) async fn select_mode_for_snapshot(
    db: &Database,
    bot_id: &str,
    bot: Option<&Document>,
) -> AppResult<GuestMode> {
    let Some(bot) = bot.filter(|bot| {
        bot.get_bool("is_active") == Ok(true) && bot.get_str("status") != Ok("suspended")
    }) else {
        return Ok(GuestMode::Legacy);
    };
    let enabled = match bot.get("guest_access_v1") {
        None | Some(bson::Bson::Boolean(false)) => false,
        Some(bson::Bson::Boolean(true)) => true,
        _ => return Err(unavailable()),
    };
    if !enabled {
        return Ok(GuestMode::Legacy);
    }
    let revision = bot
        .get_i64("guest_access_v1_revision")
        .or_else(|_| bot.get_i32("guest_access_v1_revision").map(i64::from))
        .map_err(|_| unavailable())?;
    activate_selected(db, bot_id, select_rollout(db, bot_id, revision).await?).await
}

pub(crate) async fn select_mode_for_bot(
    db: &Database,
    bot: &crate::models::channel_bot::ChannelBot,
) -> AppResult<GuestMode> {
    activate_selected(db, &bot.id, select_rollout_for_bot(db, bot).await?).await
}

pub(crate) async fn select_rollout_for_bot(
    db: &Database,
    bot: &crate::models::channel_bot::ChannelBot,
) -> AppResult<GuestMode> {
    if !bot.is_active || bot.status == "suspended" || !bot.guest_access_v1 {
        return Ok(GuestMode::Legacy);
    }
    select_rollout(db, &bot.id, bot.guest_access_v1_revision).await
}

async fn select_rollout(db: &Database, bot_id: &str, bot_revision: i64) -> AppResult<GuestMode> {
    if bot_revision < 1 {
        return Err(unavailable());
    }
    if !global_rollout_enabled(db).await? {
        return Ok(GuestMode::Legacy);
    }
    let activation = activation_collection(db)
        .find_one(doc! {"_id": FLAG})
        .await
        .map_err(|_| unavailable())?
        .filter(|record| record.enabled && record.epoch > 0);
    let Some(activation) = activation else {
        // The admin commits both documents together. The two reads may have
        // straddled a disable. Require a fresh successful read of global off.
        return if global_rollout_enabled(db).await? {
            Err(unavailable())
        } else {
            Ok(GuestMode::Legacy)
        };
    };
    if !activation.fleet
        && !activation
            .pilot_bot_ids
            .as_ref()
            .is_some_and(|bots| bots.iter().any(|id| id == bot_id))
    {
        return Ok(GuestMode::Legacy);
    }
    super::security_readiness_service::require_serving(db, FLAG).await?;
    Ok(GuestMode::AddOn {
        activation_epoch: activation.epoch,
        bot_revision,
    })
}

async fn global_rollout_enabled(db: &Database) -> AppResult<bool> {
    flags::channel_guest_access_enabled(db)
        .await
        .map_err(|_| unavailable())
}

async fn activate_selected(db: &Database, bot_id: &str, mode: GuestMode) -> AppResult<GuestMode> {
    if let GuestMode::AddOn {
        activation_epoch,
        bot_revision,
    } = mode
        && !Box::pin(setup::activate_prepared_for_rollout(
            db,
            bot_id,
            activation_epoch,
            bot_revision,
        ))
        .await?
    {
        return Ok(GuestMode::Legacy);
    }
    Ok(mode)
}

/// Write a unique value to force a MongoDB write conflict with a concurrent
/// activation or pilot change in the enclosing transaction.
pub(crate) async fn fence_selection_in_session(
    db: &Database,
    session: &mut ClientSession,
    bot_id: &str,
    activation_epoch: i64,
    bot_revision: i64,
) -> AppResult<()> {
    if activation_epoch < 1 || bot_revision < 1 {
        return Err(obsolete());
    }
    let selected = activation_collection(db)
        .find_one(
            doc! {"_id": FLAG, "enabled": true, "epoch": activation_epoch,
            "$or": [{"fleet": true}, {"pilot_bot_ids": bot_id}]},
        )
        .session(&mut *session)
        .await?;
    if selected.is_none() {
        return Err(obsolete());
    }
    // Independent bots spread their writes across the fence rows. An epoch
    // transition updates every row inside the flag transaction.
    let stripe = Sha256::digest(bot_id.as_bytes())[0] % FENCE_COUNT;
    let matched = db
        .collection::<Document>(ACTIVATION_FENCES)
        .update_one(
            doc! {"_id": i32::from(stripe), "enabled": true, "epoch": activation_epoch},
            doc! {"$set": {"fence": uuid::Uuid::new_v4().to_string()}},
        )
        .session(&mut *session)
        .await?;
    if matched.matched_count != 1 {
        return Err(obsolete());
    }
    let mut filter = active_bot(bot_id);
    filter.insert("guest_access_v1", true);
    filter.insert("guest_access_v1_revision", bot_revision);
    let guarded = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            filter,
            doc! {"$set": {"channel_guest_work_fence": uuid::Uuid::new_v4().to_string()}},
        )
        .session(&mut *session)
        .await?;
    if guarded.matched_count != 1 {
        return Err(obsolete());
    }
    Ok(())
}

pub(crate) async fn fence_in_session(
    db: &Database,
    session: &mut ClientSession,
    turn: &GuestTurn,
) -> AppResult<()> {
    fence_selection_in_session(
        db,
        session,
        &turn.bot_id,
        turn.activation_epoch,
        turn.bot_revision,
    )
    .await?;
    assignment::fence(db, session, turn).await
}

fn digest(parts: &[&str]) -> String {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part.as_bytes());
    }
    hex::encode(hash.finalize())
}

fn event_identity(bot_id: &str, binding_id: &str, platform: &str, event_id: &str) -> String {
    digest(&[
        "channel-guest-event-v1",
        bot_id,
        binding_id,
        platform,
        event_id,
    ])
}

impl GuestTurn {
    // Unverified actors are event-scoped; the follow validator independently
    // binds the retained source sender and exact event delivery.
    pub(crate) fn matches_sender(&self, sender: &str) -> bool {
        let (scope, subject) = match self.actor.assurance {
            GuestAssurance::ProviderVerified => ("provider", sender),
            GuestAssurance::Unverified => ("event", self.source_event_id.as_str()),
        };
        let mut parts = vec![
            "channel-guest-actor-v1",
            self.bot_id.as_str(),
            self.binding_id.as_str(),
            self.platform.as_str(),
            scope,
            subject,
        ];
        parts.extend(self.meta_grant_id.as_deref());
        self.actor.id == digest(&parts)
    }

    pub(crate) fn allows_shared_sheets_write(&self) -> bool {
        self.shared_room_verified
            && self.audience == AnswerAudience::Shared
            && self.actor.assurance == GuestAssurance::ProviderVerified
    }

    pub(crate) fn hosted_conversation_id(&self) -> String {
        let audience = match self.audience {
            AnswerAudience::Private => "private",
            AnswerAudience::Shared => "shared",
        };
        let epoch = self.activation_epoch.to_string();
        let revision = self.bot_revision.to_string();
        let policy = self.policy_revision.to_string();
        let mut scope = vec![
            "channel-guest-session-v1",
            &self.bot_id,
            &self.binding_id,
            &self.platform,
            &self.conversation_id,
            &self.actor.id,
            audience,
            &epoch,
            &revision,
            &policy,
        ];
        if self.allows_shared_sheets_write() {
            scope.push("verified-shared-room-v1");
        }
        let settings_generation;
        let channel_generation;
        if let Some(assignment) = &self.hosted_assignment {
            settings_generation = assignment.settings_generation.to_string();
            channel_generation = assignment.channel_generation.to_string();
            scope.extend([
                "hosted-assignment-v1",
                assignment.agent_id.as_str(),
                assignment.settings_chat_id.as_str(),
                &settings_generation,
                &channel_generation,
            ]);
            scope.extend(assignment.org_binding_revision.as_deref());
            if let Some(child) = assignment.followed_child_id.as_deref() {
                scope.extend(["followed-child", child]);
            }
        }
        let hash = digest(&scope);
        format!("{CONVERSATION_PREFIX}{}", &hash[..32])
    }

    fn work_id(&self) -> String {
        event_identity(
            &self.bot_id,
            &self.binding_id,
            &self.platform,
            &self.source_event_id,
        )
    }
}

pub(crate) fn audience_guidance(audience: AnswerAudience) -> &'static str {
    match audience {
        AnswerAudience::Shared => {
            "\nIf a guest asks to view or change their personal data, ask them to send a direct message to this bot and continue there. Keep personal data, record identifiers, and account status out of this reply.\n"
        }
        AnswerAudience::Private => {
            "\nThis is a verified private chat. Use personal data only through tools authorized for this guest and the requested operation.\n"
        }
    }
}

pub(crate) fn is_hosted_conversation(id: &str) -> bool {
    id.strip_prefix(CONVERSATION_PREFIX).is_some_and(|suffix| {
        suffix.len() == 32
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

/// The namespace survives settlement and rollback, including rows without an active turn.
pub(crate) fn is_hosted_conversation_namespace(id: &str) -> bool {
    id.starts_with(CONVERSATION_PREFIX)
}

pub(crate) fn exclude_hosted_conversations() -> Document {
    doc! {"$not": {"$regex": format!("^{CONVERSATION_PREFIX}")}}
}

fn check_turn(turn: &GuestTurn) -> AppResult<()> {
    if [
        &turn.bot_id,
        &turn.binding_id,
        &turn.platform,
        &turn.conversation_id,
        &turn.source_event_id,
        &turn.actor.id,
    ]
    .iter()
    .any(|value| value.is_empty())
        || turn.activation_epoch < 1
        || turn.bot_revision < 1
        || turn.policy_revision < 0
        || (turn.audience == AnswerAudience::Private
            && turn.actor.assurance != GuestAssurance::ProviderVerified)
    {
        return Err(obsolete());
    }
    Ok(())
}

fn active_bot(bot_id: &str) -> Document {
    doc! {"_id": bot_id, "is_active": true, "status": {"$ne": "suspended"}}
}

async fn bot_ownership_version(db: &Database, bot_id: &str) -> AppResult<i64> {
    let row = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(active_bot(bot_id))
        .projection(doc! {"ownership_version": 1})
        .await?
        .ok_or_else(obsolete)?;
    Ok(ownership_version(&row))
}

fn ownership_version(row: &Document) -> i64 {
    row.get_i64("ownership_version")
        .or_else(|_| row.get_i32("ownership_version").map(i64::from))
        .unwrap_or_default()
}

pub(crate) async fn revalidate(db: &Database, turn: &GuestTurn) -> AppResult<()> {
    revalidate_turn(db, turn, false).await
}

// Queue setup may be checked before a turn is admitted. Execution always
// requires the durable work row that was written with the turn.
async fn revalidate_turn(db: &Database, turn: &GuestTurn, admitted: bool) -> AppResult<()> {
    check_turn(turn)?;
    if select_mode(db, &turn.bot_id).await?
        != (GuestMode::AddOn {
            activation_epoch: turn.activation_epoch,
            bot_revision: turn.bot_revision,
        })
    {
        return Err(obsolete());
    }
    let version = bot_ownership_version(db, &turn.bot_id).await?;
    let work = db
        .collection::<Work>(WORK)
        .find_one(doc! {"_id": turn.work_id()})
        .await?;
    match work {
        Some(work)
            if work.ownership_version == version
                && work.turn == *turn
                && work.expires_at > bson::DateTime::now() => {}
        None if !admitted => {}
        _ => return Err(obsolete()),
    }
    assignment::revalidate(db, turn).await?;
    let verified = (turn.actor.assurance == GuestAssurance::ProviderVerified)
        .then_some(turn.actor.id.as_str());
    if !setup::allows_guest(db, &turn.bot_id, verified).await? {
        return Err(obsolete());
    }
    if policy::load_policy(db, &turn.bot_id)
        .await?
        .map_or(0, |policy| policy.revision)
        != turn.policy_revision
    {
        return Err(obsolete());
    }
    Ok(())
}

/// Hosted ingress already owns a durable provider event. Bind the add-on
/// work in the same transaction that claims its assistant turn.
#[cfg(test)]
pub(crate) async fn classify(
    db: &Database,
    mode: GuestMode,
    input: VerifiedGuestInput<'_>,
) -> AppResult<GuestAdmission> {
    classify_with_meta(db, mode, input, None).await
}

pub(crate) async fn classify_with_meta(
    db: &Database,
    mode: GuestMode,
    input: VerifiedGuestInput<'_>,
    meta_grant: Option<&crate::models::channel_meta::GrantRef>,
) -> AppResult<GuestAdmission> {
    let GuestMode::AddOn {
        activation_epoch,
        bot_revision,
    } = mode
    else {
        return Ok(GuestAdmission::Legacy);
    };
    if meta_grant.map(|grant| grant.grant_id.as_str()) != input.meta_grant_id
        || meta_grant.is_some_and(|grant| grant.bot_id != input.bot_id)
    {
        return Err(obsolete());
    }
    let sender = input.sender_id.filter(|sender| !sender.is_empty());
    let (assurance, actor_subject, scope) = match sender {
        Some(sender) => (GuestAssurance::ProviderVerified, sender, "provider"),
        None => (GuestAssurance::Unverified, input.source_event_id, "event"),
    };
    let mut actor_scope = vec![
        "channel-guest-actor-v1",
        input.bot_id,
        input.binding_id,
        input.platform,
        scope,
        actor_subject,
    ];
    actor_scope.extend(input.meta_grant_id);
    let actor = GuestActor {
        id: digest(&actor_scope),
        assurance,
    };
    let verified = sender.map(|_| actor.id.as_str());
    if !setup::allows_guest(db, input.bot_id, verified).await? {
        if let Some(actor_id) = verified {
            observe_verified_guest(
                db,
                &input,
                actor_id,
                activation_epoch,
                bot_revision,
                meta_grant,
            )
            .await?;
        }
        return Err(obsolete());
    }
    let policy_revision = policy::load_policy(db, input.bot_id)
        .await?
        .map_or(0, |policy| policy.revision);
    let turn = GuestTurn {
        bot_id: input.bot_id.into(),
        binding_id: input.binding_id.into(),
        platform: input.platform.into(),
        conversation_id: input.conversation_id.into(),
        source_event_id: input.source_event_id.into(),
        actor,
        audience: if sender.is_some() {
            input.audience
        } else {
            AnswerAudience::Shared
        },
        shared_room_verified: sender.is_some()
            && input.audience == AnswerAudience::Shared
            && input.shared_room_verified,
        policy_revision,
        activation_epoch,
        bot_revision,
        meta_grant_id: input.meta_grant_id.map(str::to_owned),
        hosted_assignment: None,
    };
    check_turn(&turn)?;
    Ok(GuestAdmission::AddOn(Box::new(turn)))
}

async fn observe_verified_guest(
    db: &Database,
    input: &VerifiedGuestInput<'_>,
    actor_id: &str,
    activation_epoch: i64,
    bot_revision: i64,
    meta_grant: Option<&crate::models::channel_meta::GrantRef>,
) -> AppResult<()> {
    let room_id = (input.audience == AnswerAudience::Shared && !input.conversation_id.is_empty())
        .then(|| {
            verified_room(
                input.bot_id,
                input.binding_id,
                input.platform,
                input.conversation_id,
                input.meta_grant_id,
            )
        });
    let context = (
        db.clone(),
        input.bot_id.to_owned(),
        actor_id.to_owned(),
        room_id,
        input.platform.to_owned(),
        meta_grant.cloned(),
        activation_epoch,
        bot_revision,
    );
    super::transaction_service::run_db_only(db, &context, |session, (db, bot_id, actor_id, room_id, platform, grant, epoch, revision)| {
        Box::pin(async move {
            fence_selection_in_session(db, session, bot_id, *epoch, *revision).await?;
            if let Some(grant) = grant {
                super::channel_meta_service::scopes::fence_reference(db, session, grant, true)
                    .await?;
            }
            let bot = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                .find_one_and_update(active_bot(bot_id), doc! {"$set": {"channel_guest_data_present": true}})
                .projection(doc! {"ownership_version": 1}).session(&mut *session).await?
                .ok_or_else(obsolete)?;
            let version = ownership_version(&bot);
            let id = digest(&["channel-guest-observation-v1", actor_id, room_id.as_deref().unwrap_or(""), &version.to_string()]);
            db.collection::<Document>(OBSERVATIONS)
                .update_one(doc! {"_id": id}, doc! {"$set": {
                    "bot_id": bot_id, "actor_id": actor_id, "room_id": room_id, "platform": platform,
                    "meta_grant_id": grant.as_ref().map(|grant| grant.grant_id.as_str()),
                    "ownership_version": version, "expires_at": retention_deadline(),
                }})
                .upsert(true).session(session).await?;
            Ok(())
        })
    }).await
}

fn retention_deadline() -> bson::DateTime {
    bson::DateTime::from_chrono(chrono::Utc::now() + chrono::Duration::days(30))
}

#[derive(Serialize, Deserialize)]
struct Work {
    #[serde(rename = "_id")]
    id: String,
    turn: GuestTurn,
    #[serde(default)]
    ownership_version: i64,
    expires_at: bson::DateTime,
}

pub(crate) async fn bind_work_in_session(
    db: &Database,
    session: &mut ClientSession,
    turn: &GuestTurn,
) -> AppResult<()> {
    check_turn(turn)?;
    fence_in_session(db, session, turn).await?;
    let bot = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one_and_update(
            active_bot(&turn.bot_id),
            doc! {"$set": {"channel_guest_data_present": true}},
        )
        .projection(doc! {"ownership_version": 1})
        .session(&mut *session)
        .await?
        .ok_or_else(obsolete)?;
    // Setup writes use the same activation fence. The snapshot therefore
    // cannot commit an admission against a concurrently replaced policy.
    let revision = db
        .collection::<crate::models::channel_guest_policy::GuestDataPolicy>(
            crate::models::channel_guest_policy::COLLECTION_NAME,
        )
        .find_one(doc! {"bot_id": &turn.bot_id})
        .session(&mut *session)
        .await?
        .map_or(0, |policy| policy.revision);
    if revision != turn.policy_revision {
        return Err(obsolete());
    }
    let work = Work {
        id: turn.work_id(),
        turn: turn.clone(),
        ownership_version: ownership_version(&bot),
        expires_at: retention_deadline(),
    };
    db.collection::<Work>(WORK)
        .insert_one(work)
        .session(session)
        .await?;
    Ok(())
}

pub(crate) async fn admitted_turn(
    db: &Database,
    bot_id: &str,
    binding_id: &str,
    platform: &str,
    event_id: &str,
) -> AppResult<Option<GuestTurn>> {
    Ok(db
        .collection::<Work>(WORK)
        .find_one(doc! {
            "_id": event_identity(bot_id, binding_id, platform, event_id),
            "expires_at": {"$gt": bson::DateTime::now()},
        })
        .await?
        .map(|work| work.turn))
}

async fn hosted_delivery_turn(
    db: &Database,
    event: &crate::models::nyxbot_channel::NyxbotEvent,
) -> AppResult<Option<GuestTurn>> {
    let Some(conversation_id) = event
        .conversation_id
        .as_deref()
        .filter(|id| is_hosted_conversation(id))
    else {
        return Ok(None);
    };
    let Some(delivery) = &event.delivery else {
        return Ok(None);
    };
    let work = db
        .collection::<Work>(WORK)
        .find_one(doc! {
            "turn.source_event_id": &event.id, "turn.binding_id": &event.channel_id,
            "turn.platform": super::channel_platform::canonical_platform(&delivery.origin.platform),
            "expires_at": {"$gt": bson::DateTime::now()},
        })
        .await?;
    Ok(work
        .map(|work| work.turn)
        .filter(|turn| turn.hosted_conversation_id() == conversation_id))
}

pub(crate) async fn claim_hosted_delivery(
    db: &Database,
    event: &crate::models::nyxbot_channel::NyxbotEvent,
    filter: Document,
    update: impl Into<mongodb::options::UpdateModifications>,
) -> AppResult<bool> {
    use crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME;
    let Some(turn) = hosted_delivery_turn(db, event).await? else {
        return Ok(false);
    };
    if let Err(error) = policy::load_bound_policy(db, &turn).await {
        return match error {
            AppError::Forbidden(_) => Ok(false),
            error => Err(error),
        };
    }
    let grant = if let Some(grant_id) = &turn.meta_grant_id {
        let grant = super::channel_meta_service::scopes::event_reference(db, &event.id).await?;
        let Some(grant) =
            grant.filter(|grant| grant.bot_id == turn.bot_id && grant.grant_id == *grant_id)
        else {
            return Ok(false);
        };
        Some(grant)
    } else {
        None
    };
    let context = (db.clone(), turn, grant, filter, update.into());
    match super::transaction_service::run_db_only(
        db,
        &context,
        |session, (db, turn, grant, filter, update)| {
            Box::pin(async move {
                fence_in_session(db, session, turn).await?;
                if let Some(grant) = grant {
                    super::channel_meta_service::scopes::fence_reference(db, session, grant, true)
                        .await?;
                }
                if turn.policy_revision > 0 {
                    policy::fence_policy_in_session(db, session, turn).await?;
                } else if db
                    .collection::<Document>(crate::models::channel_guest_policy::COLLECTION_NAME)
                    .find_one(doc! {"bot_id": &turn.bot_id})
                    .projection(doc! {"_id": 1})
                    .session(&mut *session)
                    .await?
                    .is_some()
                {
                    return Err(obsolete());
                }
                Ok(db
                    .collection::<Document>(EVENTS_COLLECTION_NAME)
                    .update_one(filter.clone(), update.clone())
                    .session(session)
                    .await?
                    .matched_count
                    == 1)
            })
        },
    )
    .await
    {
        Err(AppError::Forbidden(_)) => Ok(false),
        result => result,
    }
}

/// Store the selected path on the existing provider event before asynchronous
/// execution. Older receipts and duplicates retain the ordinary path.
pub(crate) async fn stamp_hosted_admission(
    db: &Database,
    event_id: &str,
    binding_id: &str,
    admission: HostedAdmission,
) -> AppResult<()> {
    if admission == HostedAdmission::Legacy {
        return Ok(());
    }
    let mode = bson::to_bson(&admission).map_err(|_| unavailable())?;
    let stored = db.collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
        .update_one(
            doc! {"_id": event_id, "channel_id": binding_id, "channel_guest_admission": {"$exists": false}},
            doc! {"$set": {"channel_guest_admission": mode}},
        ).await?;
    if stored.matched_count != 1 {
        return Err(obsolete());
    }
    Ok(())
}

pub(crate) async fn hosted_admission(
    db: &Database,
    event_id: &str,
    binding_id: &str,
) -> AppResult<HostedAdmission> {
    let row = db
        .collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
        .find_one(doc! {"_id": event_id, "channel_id": binding_id})
        .projection(doc! {"channel_guest_admission": 1})
        .await?
        .ok_or_else(obsolete)?;
    row.get("channel_guest_admission")
        .map(|mode| bson::from_bson(mode.clone()).map_err(|_| unavailable()))
        .transpose()
        .map(|mode| mode.unwrap_or(HostedAdmission::Legacy))
}

pub(crate) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    personal_records::ensure_indexes(db).await?;
    db.collection::<Work>(WORK)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"expires_at": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<Work>(WORK)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"turn.binding_id": 1, "turn.source_event_id": 1, "turn.platform": 1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"turn.bot_id": 1, "turn.actor.assurance": 1, "expires_at": -1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"turn.bot_id": 1, "turn.meta_grant_id": 1, "_id": 1})
                .build(),
        ])
        .await?;
    db.collection::<Document>(OBSERVATIONS)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"expires_at": 1})
                .options(
                    IndexOptions::builder()
                        .expire_after(std::time::Duration::ZERO)
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! {"bot_id": 1, "expires_at": -1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"bot_id": 1, "meta_grant_id": 1, "_id": 1})
                .build(),
        ])
        .await?;
    Ok(())
}

fn verified_room(
    bot: &str,
    binding: &str,
    platform: &str,
    conversation: &str,
    grant: Option<&str>,
) -> String {
    let mut room_scope = vec![
        "channel-guest-room-v1",
        bot,
        binding,
        platform,
        conversation,
    ];
    room_scope.extend(grant);
    digest(&room_scope)
}

impl GuestTurn {
    pub(crate) fn verified_room_id(&self) -> Option<String> {
        (self.actor.assurance == GuestAssurance::ProviderVerified
            && !self.conversation_id.is_empty())
        .then(|| {
            verified_room(
                &self.bot_id,
                &self.binding_id,
                &self.platform,
                &self.conversation_id,
                self.meta_grant_id.as_deref(),
            )
        })
    }
}

pub(crate) async fn live_turn(
    db: &Database,
    row: &crate::models::assistant_conversation::AssistantConversation,
    active: Option<&crate::models::assistant_conversation::ActiveTurn>,
) -> AppResult<Option<GuestTurn>> {
    let guest = active.and_then(|turn| turn.channel_guest.as_ref());
    if !is_hosted_conversation(&row.id) && guest.is_none() {
        return Ok(None);
    }
    let active = active.ok_or(AppError::AssistantTurnRequired)?;
    let guest = guest.ok_or(AppError::AssistantTurnRequired)?;
    if !row.guest_turn
        || active.stop_requested
        || guest.hosted_conversation_id() != row.id
        || active.channel_event_id.as_deref() != Some(guest.source_event_id.as_str())
        || row
            .channel
            .as_ref()
            .is_none_or(|channel| channel.nyxbot_channel_id != guest.binding_id)
    {
        return Err(AppError::AssistantTurnRequired);
    }
    revalidate_turn(db, guest, true).await?;
    if let Some(grant_id) = &guest.meta_grant_id {
        let grant = active
            .channel_context
            .as_ref()
            .filter(|context| context.source_event_id == guest.source_event_id)
            .and_then(|context| context.meta_grant.as_ref())
            .filter(|grant| grant.bot_id == guest.bot_id && grant.grant_id == *grant_id)
            .ok_or_else(obsolete)?;
        let context = (db, guest, grant);
        super::transaction_service::run_db_only(db, &context, |session, &(db, guest, grant)| {
            Box::pin(async move {
                fence_in_session(db, session, guest).await?;
                super::channel_meta_service::scopes::fence_reference(db, session, grant, true).await
            })
        })
        .await?;
    }
    Ok(Some(guest.clone()))
}

pub(crate) async fn require_live_request(
    db: &Database,
    user_id: &str,
    conversation_id: &str,
    turn_id: &str,
    credential_id: &str,
) -> AppResult<(
    crate::models::assistant_conversation::AssistantConversation,
    GuestTurn,
)> {
    use crate::models::assistant_conversation::{AssistantConversation, COLLECTION_NAME};
    let row = db
        .collection::<AssistantConversation>(COLLECTION_NAME)
        .find_one(doc! {
            "_id": conversation_id, "user_id": user_id, "credential_api_key_id": credential_id,
            "active_turn.turn_id": turn_id, "active_turn.stop_requested": {"$ne": true},
        })
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
    let active = super::assistant_nyxagent::live_turn(&row, chrono::Utc::now());
    let guest = live_turn(db, &row, active)
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
    let key_live = db
        .collection::<Document>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {
            "_id": credential_id, "user_id": user_id, "is_active": true,
            "$or": [{"expires_at": null}, {"expires_at": {"$gt": bson::DateTime::now()}}],
        })
        .projection(doc! {"_id": 1})
        .await?
        .is_some();
    if !key_live {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok((row, guest))
}

pub(crate) async fn list_verified_actors(db: &Database, bot_id: &str) -> AppResult<Vec<String>> {
    Ok(list_verified_actor_hints(db, bot_id)
        .await?
        .into_iter()
        .map(|hint| hint.id)
        .collect())
}

pub(crate) async fn list_verified_rooms(db: &Database, bot_id: &str) -> AppResult<Vec<String>> {
    Ok(list_verified_room_hints(db, bot_id)
        .await?
        .into_iter()
        .map(|hint| hint.id)
        .collect())
}

#[derive(Serialize)]
pub(crate) struct ObservationHint {
    pub id: String,
    pub platform: String,
    pub last_seen_at: String,
}

pub(crate) async fn list_verified_actor_hints(
    db: &Database,
    bot_id: &str,
) -> AppResult<Vec<ObservationHint>> {
    observation_hints(db, bot_id, false).await
}

pub(crate) async fn list_verified_room_hints(
    db: &Database,
    bot_id: &str,
) -> AppResult<Vec<ObservationHint>> {
    observation_hints(db, bot_id, true).await
}

async fn observation_hints(
    db: &Database,
    bot_id: &str,
    rooms: bool,
) -> AppResult<Vec<ObservationHint>> {
    use futures::TryStreamExt;
    let version = bot_ownership_version(db, bot_id).await?;
    let version_filter = if version == 0 {
        doc! {"$in": [bson::Bson::Null, bson::Bson::Int64(0)]}
    } else {
        doc! {"$eq": version}
    };
    let now = bson::DateTime::now();
    let mut filter = doc! {
        "turn.bot_id": bot_id, "turn.actor.assurance": "provider_verified",
        "ownership_version": &version_filter, "expires_at": {"$gt": now},
    };
    if rooms {
        filter.insert("turn.audience", "shared");
    }
    let work = db
        .collection::<Work>(WORK)
        .find(filter)
        .sort(doc! {"expires_at": -1})
        .limit(500)
        .await?;
    let work: Vec<Work> = work.try_collect().await?;
    let field = if rooms { "room_id" } else { "actor_id" };
    let observed = db
        .collection::<Document>(OBSERVATIONS)
        .find(doc! {"bot_id": bot_id, "ownership_version": version_filter,
        field: {"$type": "string"}, "expires_at": {"$gt": now}})
        .sort(doc! {"expires_at": -1})
        .limit(500)
        .await?;
    let observed: Vec<Document> = observed.try_collect().await?;
    let admitted = work.into_iter().filter_map(|work| {
        let id = if rooms {
            work.turn.verified_room_id()?
        } else {
            work.turn.actor.id.clone()
        };
        Some((id, work.turn.platform, work.expires_at))
    });
    let declined = observed.into_iter().filter_map(|row| {
        Some((
            row.get_str(field).ok()?.to_owned(),
            row.get_str("platform").ok()?.to_owned(),
            *row.get_datetime("expires_at").ok()?,
        ))
    });
    let mut hints: Vec<_> = admitted.chain(declined).collect();
    hints.sort_by_key(|(_, _, expires)| std::cmp::Reverse(*expires));
    let mut seen = std::collections::HashSet::new();
    Ok(hints
        .into_iter()
        .filter(|(id, _, _)| seen.insert(id.clone()))
        .take(100)
        .map(|(id, platform, expires)| ObservationHint {
            id,
            platform,
            last_seen_at: (expires.to_chrono() - chrono::Duration::days(30)).to_rfc3339(),
        })
        .collect())
}

pub(crate) async fn purge_observations_before_ownership(
    db: &Database,
    bot_id: &str,
    ownership_version: i64,
) -> AppResult<()> {
    db.collection::<Document>(OBSERVATIONS)
        .delete_many(doc! {"bot_id": bot_id, "ownership_version": {"$lt": ownership_version}})
        .await?;
    Ok(())
}

/// Drain one page from each collection after the exact Meta grant is fenced
/// against new work. The erasure journal retains the phase until both are empty.
pub(crate) async fn purge_meta_grant_batch(
    db: &Database,
    bot_id: &str,
    grant_id: &str,
) -> AppResult<bool> {
    use futures::TryStreamExt;

    const BATCH: i64 = 100;
    let mut empty = true;
    for (name, filter) in [
        (
            WORK,
            doc! {"turn.bot_id": bot_id, "turn.meta_grant_id": grant_id},
        ),
        (
            OBSERVATIONS,
            doc! {"bot_id": bot_id, "meta_grant_id": grant_id},
        ),
    ] {
        let rows = db.collection::<Document>(name);
        let ids: Vec<_> = rows
            .find(filter.clone())
            .projection(doc! {"_id": 1})
            .sort(doc! {"_id": 1})
            .limit(BATCH)
            .await?
            .map_ok(|mut row| row.remove("_id").expect("projected MongoDB ID"))
            .try_collect()
            .await?;
        if !ids.is_empty() {
            rows.delete_many(doc! {"$and": [filter, {"_id": {"$in": ids}}]})
                .await?;
            empty = false;
        }
    }
    Ok(empty)
}

pub(crate) async fn purge_bot(db: &Database, bot_id: &str) -> AppResult<()> {
    db.collection::<Work>(WORK)
        .delete_many(doc! {"turn.bot_id": bot_id})
        .await?;
    db.collection::<Document>(OBSERVATIONS)
        .delete_many(doc! {"bot_id": bot_id})
        .await?;
    Ok(())
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PilotSelection {
    #[serde(deserialize_with = "Deserialize::deserialize")]
    pub bot_ids: Option<Vec<String>>,
    #[serde(default)]
    pub fleet: bool,
}

pub(crate) async fn pilot(db: &Database) -> AppResult<PilotSelection> {
    let state = activation_collection(db)
        .find_one(doc! {"_id": FLAG})
        .await?;
    Ok(PilotSelection {
        fleet: state.as_ref().is_some_and(|state| state.fleet),
        bot_ids: state.and_then(|state| state.pilot_bot_ids),
    })
}

pub(crate) async fn set_pilot(
    db: &Database,
    selection: PilotSelection,
    actor: &super::audit_service::AuditActor,
) -> AppResult<PilotSelection> {
    let fleet = selection.fleet;
    let bot_ids = selection
        .bot_ids
        .map(|mut ids| {
            if ids.len() > 100 || ids.iter().any(|id| uuid::Uuid::parse_str(id).is_err()) {
                return Err(AppError::ValidationError(
                    "Provide up to 100 channel bot IDs".into(),
                ));
            }
            ids.sort_unstable();
            ids.dedup();
            Ok(ids)
        })
        .transpose()?;
    if fleet && bot_ids.as_ref().is_some_and(|ids| !ids.is_empty()) {
        return Err(AppError::ValidationError(
            "Choose either a bot list or fleet rollout".into(),
        ));
    }
    let context = (db.clone(), bot_ids.clone(), fleet, actor.clone());
    super::transaction_service::run_db_only(
        db,
        &context,
        |session, (db, bot_ids, fleet, actor)| Box::pin(async move {
            let before = activation_collection(db)
                .find_one(doc! {"_id": FLAG})
                .session(&mut *session)
                .await?;
            if before.as_ref().and_then(|state| state.pilot_bot_ids.as_ref()) == bot_ids.as_ref()
                && before.as_ref().is_some_and(|state| state.fleet) == *fleet {
                return Ok(());
            }
            if before.as_ref().is_some_and(|state| state.enabled) {
                return Err(AppError::Conflict(
                    "Turn off guest audience access before editing the pilot selection".into(),
                ));
            }
            if let Some(ids) = bot_ids {
                let count = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                    .count_documents(doc! {"_id": {"$in": ids}, "is_active": true})
                    .session(&mut *session)
                    .await?;
                if count != ids.len() as u64 {
                    return Err(AppError::ValidationError("Every pilot bot must be active".into()));
                }
            }
            let pilot = match bot_ids {
                Some(ids) => bson::Bson::from(ids.clone()),
                None => bson::Bson::Null,
            };
            db.collection::<Document>(ACTIVATION)
                .update_one(
                    doc! {"_id": FLAG},
                    doc! {"$set": {"pilot_bot_ids": pilot, "fleet": fleet}, "$setOnInsert": {"epoch": 0_i64, "enabled": false}},
                )
                .upsert(true)
                .session(&mut *session)
                .await?;
            super::audit_service::log_actor_event_in_transaction(
                db,
                session,
                actor,
                "admin_channel_guest_pilot_updated",
                serde_json::json!({"before": before.map(|state| serde_json::json!({"bot_ids": state.pilot_bot_ids, "fleet": state.fleet})), "after": {"bot_ids": bot_ids, "fleet": fleet}}),
            )
            .await?;
            Ok(())
        }),
    ).await?;
    Ok(PilotSelection { bot_ids, fleet })
}

#[cfg(test)]
pub(crate) mod test_hooks {
    use std::{
        collections::HashMap,
        sync::{Arc, LazyLock, Mutex},
    };
    use tokio::sync::Semaphore;

    struct PausedWorker {
        provider_entered: Semaphore,
        settlement_release: Semaphore,
    }

    static PAUSED_WORKERS: LazyLock<Mutex<HashMap<String, Arc<PausedWorker>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    pub(crate) struct WorkerPause {
        db_name: String,
        worker: Arc<PausedWorker>,
    }

    pub(crate) fn pause_worker(db: &mongodb::Database) -> WorkerPause {
        let worker = Arc::new(PausedWorker {
            provider_entered: Semaphore::new(0),
            settlement_release: Semaphore::new(0),
        });
        PAUSED_WORKERS
            .lock()
            .unwrap()
            .insert(db.name().into(), worker.clone());
        WorkerPause {
            db_name: db.name().into(),
            worker,
        }
    }

    impl WorkerPause {
        pub(crate) async fn wait_for_provider(&self) {
            self.worker
                .provider_entered
                .acquire()
                .await
                .unwrap()
                .forget();
        }
    }

    impl Drop for WorkerPause {
        fn drop(&mut self) {
            PAUSED_WORKERS.lock().unwrap().remove(&self.db_name);
            self.worker.settlement_release.close();
        }
    }

    pub(crate) fn provider_entered(db: &mongodb::Database, conversation_id: &str) {
        if super::is_hosted_conversation(conversation_id)
            && let Some(worker) = PAUSED_WORKERS.lock().unwrap().get(db.name())
        {
            worker.provider_entered.add_permits(1);
        }
    }

    pub(crate) async fn wait_before_settlement(db: &mongodb::Database, conversation_id: &str) {
        if super::is_hosted_conversation(conversation_id) {
            let worker = PAUSED_WORKERS.lock().unwrap().get(db.name()).cloned();
            if let Some(worker) = worker {
                let _ = worker.settlement_release.acquire().await;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_guest_audience_guidance_requests_a_dm_for_shared_personal_data() {
        let shared = audience_guidance(AnswerAudience::Shared);
        let private = audience_guidance(AnswerAudience::Private);
        assert!(shared.contains("send a direct message to this bot"));
        assert!(shared.contains("Keep personal data, record identifiers, and account status out"));
        assert!(!private.contains("send a direct message"));
        assert!(private.contains("tools authorized for this guest"));
    }

    #[tokio::test]
    async fn channel_guest_activation_fences_pilot_and_keeps_distinct_bot_transactions_independent()
    {
        use crate::models::feature_flag_override::FlagTargetKind;
        use crate::services::{audit_service::AuditActor, feature_flag_admin_service as admin};
        crate::test_utils::init_test_audit_chain_hmac_key();
        let db =
            crate::test_utils::connect_transaction_test_database("channel_guest_activation").await;
        let actor = AuditActor {
            user_id: uuid::Uuid::new_v4().to_string(),
            api_key_id: None,
            api_key_name: None,
            ip_address: None,
            user_agent: None,
        };
        let first = uuid::Uuid::new_v4().to_string();
        let mut second = uuid::Uuid::new_v4().to_string();
        while Sha256::digest(first.as_bytes())[0] % FENCE_COUNT
            == Sha256::digest(second.as_bytes())[0] % FENCE_COUNT
        {
            second = uuid::Uuid::new_v4().to_string();
        }
        db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_many([
                doc! {"_id": &first, "is_active": true, "guest_access_v1": true, "guest_access_v1_revision": 1_i64},
                doc! {"_id": &second, "is_active": true, "guest_access_v1": true, "guest_access_v1_revision": 1_i64},
                doc! {"_id": "outside-pilot", "is_active": true, "guest_access_v1": true, "guest_access_v1_revision": 1_i64},
            ])
            .await
            .unwrap();
        set_pilot(
            &db,
            PilotSelection {
                bot_ids: Some(vec![first.clone(), second.clone()]),
                fleet: false,
            },
            &actor,
        )
        .await
        .unwrap();
        assert_eq!(select_mode(&db, &first).await.unwrap(), GuestMode::Legacy);
        crate::services::security_readiness_service::ready_test_replica(&db).await;
        assert!(
            admin::mutate(
                &db,
                FLAG,
                admin::Mutation::Override {
                    kind: FlagTargetKind::User,
                    target_id: Some(actor.user_id.clone()),
                    enabled: Some(true),
                },
                &actor
            )
            .await
            .is_err()
        );
        admin::mutate(
            &db,
            FLAG,
            admin::Mutation::Override {
                kind: FlagTargetKind::Global,
                target_id: None,
                enabled: Some(true),
            },
            &actor,
        )
        .await
        .unwrap();
        let GuestMode::AddOn {
            activation_epoch,
            bot_revision,
        } = select_mode(&db, &first).await.unwrap()
        else {
            panic!("pilot bot must use the add-on");
        };
        assert_eq!(
            select_mode(&db, "outside-pilot").await.unwrap(),
            GuestMode::Legacy
        );

        let bots = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME);
        for change in [
            doc! {"is_active": true, "status": "suspended"},
            doc! {"is_active": false, "status": "active"},
        ] {
            bots.update_one(doc! {"_id": &first}, doc! {"$set": change})
                .await
                .unwrap();
            assert_eq!(select_mode(&db, &first).await.unwrap(), GuestMode::Legacy);
            let observed = bots.find_one(doc! {"_id": &first}).await.unwrap();
            assert_eq!(
                select_mode_for_snapshot(&db, &first, observed.as_ref())
                    .await
                    .unwrap(),
                GuestMode::Legacy,
            );
        }
        bots.update_one(doc! {"_id": &first}, doc! {"$set": {"is_active": true}})
            .await
            .unwrap();

        let mut txn1 = db.client().start_session().await.unwrap();
        txn1.start_transaction().await.unwrap();
        fence_selection_in_session(&db, &mut txn1, &first, activation_epoch, bot_revision)
            .await
            .unwrap();
        let mut txn2 = db.client().start_session().await.unwrap();
        txn2.start_transaction().await.unwrap();
        tokio::time::timeout(
            std::time::Duration::from_secs(3),
            fence_selection_in_session(&db, &mut txn2, &second, activation_epoch, bot_revision),
        )
        .await
        .expect("another bot should not depend on the first bot's open transaction")
        .unwrap();
        txn2.commit_transaction().await.unwrap();
        txn1.commit_transaction().await.unwrap();

        let started = std::time::Instant::now();
        admin::mutate(
            &db,
            FLAG,
            admin::Mutation::Override {
                kind: FlagTargetKind::Global,
                target_id: None,
                enabled: None,
            },
            &actor,
        )
        .await
        .unwrap();
        tracing::info!(
            elapsed_ms = started.elapsed().as_millis(),
            "channel guest test switch-off"
        );
        assert_eq!(select_mode(&db, &first).await.unwrap(), GuestMode::Legacy);
        let mut stale = db.client().start_session().await.unwrap();
        stale.start_transaction().await.unwrap();
        assert!(matches!(
            fence_selection_in_session(&db, &mut stale, &first, activation_epoch, bot_revision)
                .await,
            Err(AppError::Forbidden(_))
        ));
        stale.abort_transaction().await.unwrap();
        assert_eq!(pilot(&db).await.unwrap().bot_ids.unwrap().len(), 2);
        db.drop().await.unwrap();
    }
}
