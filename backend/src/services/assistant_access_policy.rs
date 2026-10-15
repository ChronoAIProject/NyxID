//! Issue #1883. The rule is the owner's: talk to an agent and that agent
//! handles everything; NyxBot may take over only when the owner talks to
//! NyxBot (it assigned the work) or in a group chat that NyxBot is in.
//! Ambiguity (no live turn, missing fields, legacy rows) never creates NyxBot
//! involvement.
use std::sync::LazyLock;

use chrono::{DateTime, Utc};
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use serde_json::{Value, json};

use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_acknowledgement::{
            AssistantAcknowledgement, COLLECTION_NAME as ACKS, CardRouting,
        },
        assistant_agent::COLLECTION_NAME as AGENTS,
        assistant_conversation::{AgentEvent, AssistantConversation},
        assistant_group::COLLECTION_NAME as GROUPS,
    },
    services::{
        assistant_acknowledgement_service::{self as acks, ChatAuthority},
        assistant_links::{AGENT_DETAILS_GRANTS, AGENT_DETAILS_SKILLS, AssistantPage},
    },
};

// ---------------------------------------------------------------------------
// Involvement
// ---------------------------------------------------------------------------

/// The group ID comes from the fenced conversation row, never from tool input.
pub fn routing_for(chat: &ChatAuthority, conversation: &AssistantConversation) -> CardRouting {
    if !chat.involved() || !same_live_turn(chat, conversation, Utc::now()) {
        return CardRouting::NotInvolved;
    }
    if let Some(report_to) = chat.report_to.as_deref() {
        return CardRouting::Assigned {
            report_to: report_to.to_owned(),
        };
    }
    if chat.with_nyxbot
        && let Some(group_id) = conversation.group_id.as_deref()
    {
        return CardRouting::GroupWithNyxbot {
            group_id: group_id.to_owned(),
        };
    }
    CardRouting::NotInvolved
}

pub fn same_live_turn(
    chat: &ChatAuthority,
    conversation: &AssistantConversation,
    now: DateTime<Utc>,
) -> bool {
    let Some(turn_id) = chat.turn_id.as_deref() else {
        return false;
    };
    super::assistant_nyxagent::live_turn(conversation, now).is_some_and(|turn| {
        turn.turn_id == turn_id
            && !turn.stop_requested
            && turn.report_to.as_deref() == chat.report_to.as_deref()
            && turn.with_nyxbot == chat.with_nyxbot
    })
}

pub fn require_same_live_turn(
    chat: &ChatAuthority,
    conversation: &AssistantConversation,
) -> AppResult<()> {
    if same_live_turn(chat, conversation, Utc::now()) {
        return Ok(());
    }
    Err(AppError::Conflict(
        "The turn that asked for this has ended or changed; ask again from a live turn".into(),
    ))
}

async fn nyxbot_agent_id(
    db: &Database,
    owner: &str,
    session: Option<&mut ClientSession>,
) -> AppResult<Option<String>> {
    let collection = db.collection::<bson::Document>(AGENTS);
    let filter = doc! {"user_id": owner, "kind": "nyxbot", "destroyed_at": bson::Bson::Null};
    let row = match session {
        Some(session) => {
            collection
                .find_one(filter)
                .projection(doc! {"_id": 1, "home_conversation_id": 1})
                .session(session)
                .await?
        }
        None => {
            collection
                .find_one(filter)
                .projection(doc! {"_id": 1, "home_conversation_id": 1})
                .await?
        }
    };
    Ok(row.and_then(|row| row.get_str("_id").ok().map(str::to_owned)))
}

/// Live recheck for `GroupWithNyxbot`: the owner's NyxBot is a current
/// member of this *personal* group (org groups never qualify). A group that
/// gained or lost NyxBot after a card was raised changes this answer, which
/// is the point: decisions and delayed routing re-evaluate it. Inside a
/// transaction the recheck is a fenced write (`authority_fence`) on the
/// group document, so a concurrent membership change conflicts with the
/// decision instead of racing past it (plan §2.2).
pub async fn nyxbot_in_personal_group(
    db: &Database,
    owner: &str,
    group_id: &str,
    mut session: Option<&mut ClientSession>,
) -> AppResult<bool> {
    let Some(nyxbot) = nyxbot_agent_id(db, owner, session.as_deref_mut()).await? else {
        return Ok(false);
    };
    let filter = doc! {
        "_id": group_id, "user_id": owner,
        "created_by_user_id": bson::Bson::Null,
        "member_agent_ids": &nyxbot,
    };
    let collection = db.collection::<bson::Document>(GROUPS);
    let found = match session {
        Some(session) => {
            collection
                .find_one_and_update(filter, doc! {"$inc": {"authority_fence": 1_i64}})
                .projection(doc! {"_id": 1})
                .session(session)
                .await?
        }
        None => {
            collection
                .find_one(filter)
                .projection(doc! {"_id": 1})
                .await?
        }
    };
    Ok(found.is_some())
}

pub async fn nyxbot_destination(
    db: &Database,
    owner: &str,
    routing: Option<&CardRouting>,
    mut session: Option<&mut ClientSession>,
) -> AppResult<Option<String>> {
    let conversations =
        db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME);
    let orchestrator_thread = |id: &str| doc! {"_id": id, "user_id": owner, "role": "orchestrator"};
    match routing {
        Some(CardRouting::Assigned { report_to }) => {
            let filter = orchestrator_thread(report_to);
            let exists = match session.as_deref_mut() {
                Some(session) => {
                    conversations
                        .find_one(filter)
                        .projection(doc! {"_id": 1})
                        .session(session)
                        .await?
                }
                None => {
                    conversations
                        .find_one(filter)
                        .projection(doc! {"_id": 1})
                        .await?
                }
            };
            Ok(exists.map(|_| report_to.clone()))
        }
        Some(CardRouting::GroupWithNyxbot { group_id }) => {
            if !nyxbot_in_personal_group(db, owner, group_id, session.as_deref_mut()).await? {
                return Ok(None);
            }
            let collection = db.collection::<bson::Document>(AGENTS);
            let filter =
                doc! {"user_id": owner, "kind": "nyxbot", "destroyed_at": bson::Bson::Null};
            let agent = match session {
                Some(session) => {
                    collection
                        .find_one(filter)
                        .projection(doc! {"home_conversation_id": 1})
                        .session(session)
                        .await?
                }
                None => {
                    collection
                        .find_one(filter)
                        .projection(doc! {"home_conversation_id": 1})
                        .await?
                }
            };
            Ok(agent.and_then(|row| row.get_str("home_conversation_id").ok().map(str::to_owned)))
        }
        Some(CardRouting::NotInvolved) | None => Ok(None),
    }
}

// ---------------------------------------------------------------------------
// Specialist-facing guidance
// ---------------------------------------------------------------------------

/// What a specialist lacked; non-involved wording never mentions NyxBot.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AccessArea {
    Service,
    Account,
    Operations,
    Skills,
    Machine,
    SavedLogin,
    MachineCapabilities,
    Connect,
    Schedules,
}

impl AccessArea {
    pub fn scope(self) -> &'static str {
        match self {
            Self::Service => "service",
            Self::Account => "account",
            Self::Operations => "operations",
            Self::Skills => "skills",
            Self::Machine => "machine",
            Self::SavedLogin => "saved_login",
            Self::MachineCapabilities => "machine_capabilities",
            Self::Connect => "connect_service",
            Self::Schedules => "schedules",
        }
    }

    pub fn from_scope(scope: &str) -> Option<Self> {
        Some(match scope {
            "service" => Self::Service,
            "account" => Self::Account,
            "operations" => Self::Operations,
            "skills" => Self::Skills,
            "machine" => Self::Machine,
            "saved_login" => Self::SavedLogin,
            "machine_capabilities" => Self::MachineCapabilities,
            "connect_service" => Self::Connect,
            "schedules" => Self::Schedules,
            _ => return None,
        })
    }

    fn page<'a>(self, agent_id: &'a str) -> AssistantPage<'a> {
        match self {
            Self::Skills => AssistantPage::AgentDetails {
                agent: agent_id,
                tab: AGENT_DETAILS_SKILLS,
            },
            Self::Schedules => AssistantPage::Automations {
                setup: None,
                agent: Some(agent_id),
            },
            _ => AssistantPage::AgentDetails {
                agent: agent_id,
                tab: AGENT_DETAILS_GRANTS,
            },
        }
    }

    fn what(self, target: &str) -> String {
        match self {
            Self::Service => format!("the service {target}"),
            Self::Account => "read access to the NyxID account".into(),
            Self::Operations => format!("more operations of the service {target}"),
            Self::Skills => format!("the skill {target}"),
            Self::Machine => format!("the machine {target}"),
            Self::SavedLogin => format!("the saved login {target}"),
            Self::MachineCapabilities => format!("a capability change on the machine {target}"),
            Self::Connect => "connecting a new service".into(),
            Self::Schedules => "creating or changing schedules".into(),
        }
    }
}

fn safe(value: Option<&str>) -> String {
    let cleaned: String = value
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ' '))
        .take(80)
        .collect();
    if cleaned.trim().is_empty() {
        "this resource".into()
    } else {
        cleaned.trim().to_owned()
    }
}

pub const MISSING_ACCESS: &str = "missing_access";

pub fn missing_access(
    area: AccessArea,
    target: Option<&str>,
    agent_id: &str,
    status: Option<&str>,
) -> Value {
    let what = area.what(&safe(target));
    let path = area.page(agent_id).path();
    let instructions = match area {
        AccessArea::Skills => format!(
            "This agent does not have {what} attached. No request was created and nothing \
            will be attached by itself. Skills already attached to you are usable with \
            nyxid__skill_read. Tell the person you are working for what you are missing and \
            continue with what you can do; they can attach skills on this agent's Skills page \
            in NyxID ({path})."
        ),
        AccessArea::Schedules => format!(
            "You cannot do {what}. No request was created. Tell the person you are working \
            for what you need and continue with what you can do; they can set it up on the \
            Automations page in NyxID ({path})."
        ),
        AccessArea::Connect => format!(
            "You cannot do {what}. No request was created. Tell the person you are working \
            for which service you need and continue with what you can do; they can grant \
            services on this agent's Grants page in NyxID ({path})."
        ),
        _ => format!(
            "You do not have access to {what}. No request was created. Tell the person you are \
            working for what you are missing and continue with what you can do; they can grant \
            it on this agent's Grants page in NyxID ({path})."
        ),
    };
    let mut value = json!({
        "error": MISSING_ACCESS,
        "scope": area.scope(),
        "target": target,
        "request_created": false,
        "agent_id": agent_id,
        "grants_path": path,
        "instructions": instructions,
    });
    if let Some(status) = status {
        value["status"] = json!(status);
    }
    value
}

pub fn with_frontend(mut value: Value, frontend: &str) -> Value {
    if value["error"] == MISSING_ACCESS
        && let Some(path) = value["grants_path"].as_str()
    {
        let url = format!("{}{}", frontend.trim_end_matches('/'), path);
        let previous = value["grants_url"].as_str().unwrap_or(path);
        if let Some(instructions) = value["instructions"].as_str() {
            value["instructions"] =
                json!(instructions.replace(&format!("({previous})"), &format!("({url})"),));
        }
        value["grants_url"] = json!(url);
    }
    value
}

pub fn missing_access_error(value: &Value) -> AppError {
    AppError::AssistantMissingAccess(Box::new(value.clone()))
}

/// The error itself stays neutral (code 12420); the context decides whom the
/// agent turns to.
pub fn machine_permission_guidance(chat: &ChatAuthority, frontend: &str) -> String {
    if chat.is_orchestrator() {
        "Read the machine's current capability revision with nyxid__machine_capabilities and \
        apply the capabilities the owner asked for under the governing permission mode."
            .into()
    } else if chat.guest {
        "Only the bot's owner can change what this agent may do on a machine.".into()
    } else if chat.involved() {
        "Request the needed capabilities with nyxid__machine_capabilities: read its current \
        revision first and propose only what the user's request needs; the request is \
        decided by NyxBot for work it assigned or a group it is in, or by the owner."
            .into()
    } else {
        format!(
            "You cannot request capability changes from this chat and no request was created. \
            Tell the person you are working for which capability you are missing and continue \
            with what you can do; they can change it on this agent's Grants page in NyxID ({}).",
            AssistantPage::AgentDetails {
                agent: &chat.agent_id,
                tab: AGENT_DETAILS_GRANTS,
            }
            .url(frontend)
        )
    }
}

pub fn connect_refusal(chat: &ChatAuthority) -> Value {
    if chat.involved() {
        acks::access_request_denied()
    } else {
        missing_access(AccessArea::Connect, None, &chat.agent_id, None)
    }
}

// ---------------------------------------------------------------------------
// Transitional cutover and skill-notification eligibility (plan §4)
// ---------------------------------------------------------------------------

pub const MIGRATIONS_COLLECTION_NAME: &str = "assistant_migrations";
pub const CUTOVER_MARKER_ID: &str = "issue-1883";

static CUTOVER: LazyLock<dashmap::DashMap<String, DateTime<Utc>>> =
    LazyLock::new(dashmap::DashMap::new);

/// Write the immutable cutover marker (`$setOnInsert`: the first writer's
/// value is kept forever) and cache the value by database name for this process. Every
/// replica calls this at startup before serving assistant traffic; an error
/// means the marker is unreadable and skill routing must stay refused (fail
/// closed) until it can be read. The upsert itself is idempotent and always
/// runs, so a process never serves a database whose marker it has not seen.
pub async fn ensure_cutover(db: &Database) -> AppResult<DateTime<Utc>> {
    let row = db
        .collection::<bson::Document>(MIGRATIONS_COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id": CUTOVER_MARKER_ID},
            doc! {"$setOnInsert": {"cutover_at": bson::DateTime::now()}},
        )
        .upsert(true)
        .return_document(mongodb::options::ReturnDocument::After)
        .await?
        .ok_or_else(unreadable)?;
    let at = parse_cutover(&row)?;
    Ok(*CUTOVER.entry(db.name().to_owned()).or_insert(at))
}

fn unreadable() -> AppError {
    AppError::Internal(
        "The issue-1883 cutover marker is unreadable; skill routing is refused".into(),
    )
}

pub fn parse_cutover(row: &bson::Document) -> AppResult<DateTime<Utc>> {
    Ok(row
        .get_datetime("cutover_at")
        .map_err(|_| unreadable())?
        .to_chrono())
}

pub fn cutover_at(db: &Database) -> Option<DateTime<Utc>> {
    CUTOVER.get(db.name()).map(|at| *at)
}

#[cfg(test)]
pub(crate) async fn initialize_test_cutover(
    db: &Database,
    at: DateTime<Utc>,
) -> AppResult<DateTime<Utc>> {
    db.collection::<bson::Document>(MIGRATIONS_COLLECTION_NAME)
        .update_one(
            doc! {"_id": CUTOVER_MARKER_ID},
            doc! {"$set": {"cutover_at": bson::DateTime::from_chrono(at)}},
        )
        .upsert(true)
        .await?;
    CUTOVER.remove(db.name());
    ensure_cutover(db).await
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillDestination {
    /// The assigning NyxBot thread.
    Assigned(String),
    /// NyxBot's home thread, after a live membership recheck of this group.
    GroupNyxbotHome { group_id: String },
    /// A card saved before the cutover without a routing: the pre-#1883
    /// destination rules, until sunset.
    Legacy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SkillEligibility {
    Eligible(SkillDestination),
    Ineligible(&'static str),
}

/// `cutover` is the value read at startup; `None` fails closed with an error,
/// never with eligibility.
pub fn skill_notification_eligible(
    card: &AssistantAcknowledgement,
    cutover: Option<DateTime<Utc>>,
) -> AppResult<SkillEligibility> {
    Ok(match card.routing.as_ref() {
        Some(CardRouting::Assigned { report_to }) => {
            SkillEligibility::Eligible(SkillDestination::Assigned(report_to.clone()))
        }
        Some(CardRouting::GroupWithNyxbot { group_id }) => {
            SkillEligibility::Eligible(SkillDestination::GroupNyxbotHome {
                group_id: group_id.clone(),
            })
        }
        Some(CardRouting::NotInvolved) => SkillEligibility::Ineligible("not_involved"),
        None => {
            let cutover = cutover.ok_or_else(unreadable)?;
            if card.created_at < cutover {
                SkillEligibility::Eligible(SkillDestination::Legacy)
            } else {
                SkillEligibility::Ineligible("unbound_after_cutover")
            }
        }
    })
}

pub async fn terminalize_skill_outbox_in_session(
    db: &Database,
    owner: &str,
    card_id: &str,
    reason: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    db.collection::<bson::Document>(ACKS)
        .update_one(
            doc! {"_id": card_id, "user_id": owner, "kind": "skills"},
            doc! {
                "$unset": {"skill_notification_pending": "",
                    "skill_notification_authority_reason": "",
                    "skill_notification_authority_failures": "",
                    "skill_notification_retry_after": ""},
                "$set": {"skill_notification_terminal": {
                    "reason": reason, "at": bson::DateTime::now()}},
            },
        )
        .session(session)
        .await?;
    Ok(())
}

#[derive(Debug, Default)]
pub struct AdmittedEvents {
    pub eligible: Vec<AgentEvent>,
    pub dropped: Vec<String>,
}

/// Storage errors propagate and the admission retries; nothing is treated as
/// ineligible because of them (R5-4).
pub async fn admit_skill_events_in_session(
    db: &Database,
    owner: &str,
    events: Vec<AgentEvent>,
    session: &mut ClientSession,
) -> AppResult<AdmittedEvents> {
    let mut admitted = AdmittedEvents::default();
    for event in events {
        if event.kind != "skill_request_allowed" {
            admitted.eligible.push(event);
            continue;
        }
        let card = db
            .collection::<AssistantAcknowledgement>(ACKS)
            .find_one(doc! {"_id": &event.id, "user_id": owner, "kind": "skills"})
            .session(&mut *session)
            .await?;
        let Some(card) = card else {
            admitted.dropped.push(event.id);
            continue;
        };
        let verdict = match skill_notification_eligible(&card, cutover_at(db))? {
            SkillEligibility::Eligible(SkillDestination::GroupNyxbotHome { group_id }) => {
                if nyxbot_in_personal_group(db, owner, &group_id, Some(&mut *session)).await? {
                    Ok(())
                } else {
                    Err("group_membership_changed")
                }
            }
            SkillEligibility::Eligible(_) => Ok(()),
            SkillEligibility::Ineligible(reason) => Err(reason),
        };
        match verdict {
            Ok(()) => admitted.eligible.push(event),
            Err(reason) => {
                terminalize_skill_outbox_in_session(db, owner, &card.id, reason, session).await?;
                admitted.dropped.push(event.id);
            }
        }
    }
    acks::skill_notifications::consume_in_session(db, owner, &admitted.eligible, session).await?;
    Ok(admitted)
}

#[cfg(test)]
#[path = "assistant_access_policy_tests.rs"]
mod tests;
