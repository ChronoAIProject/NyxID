//! Human decisions bound to one conversation and one credential generation.
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::{
        api_key::{ApiKey, COLLECTION_NAME as KEYS},
        assistant_acknowledgement::{AssistantAcknowledgement, COLLECTION_NAME as ACKS},
        assistant_agent::GuestAccess,
        assistant_agent_credential::COLLECTION_NAME as CREDENTIALS,
        assistant_conversation::{
            AgentRole, AssistantConversation, COLLECTION_NAME as CONVERSATIONS,
        },
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
    },
    mw::auth::ASSISTANT_ACCOUNT_SCOPE,
    services::{api_key_mutation_service as mutations, audit_service, key_service},
};

/// The request-time authority snapshot for a per-conversation assistant key.
/// It is populated while the conversation is loaded during authentication and
/// never contains key material.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssistantTurnFence {
    pub conversation_id: String,
    pub turn_id: Option<String>,
    pub turn_live: bool,
    pub turn_stopped: bool,
}

impl AssistantTurnFence {
    pub fn require_live(&self) -> AppResult<()> {
        if self.turn_live && !self.turn_stopped {
            return Ok(());
        }
        Err(AppError::AssistantTurnRequired)
    }

    pub fn refusal_reason(&self) -> &'static str {
        if self.turn_id.is_none() {
            "idle"
        } else if self.turn_stopped {
            "stopped"
        } else {
            "expired"
        }
    }
}

/// Apply the rollout-controlled request fence and emit a bounded, sampled
/// metadata-only audit record for refusals. The fence itself is always
/// computed by authentication; this function only controls enforcement.
#[allow(clippy::too_many_arguments)]
pub async fn enforce_turn_gate(
    db: &Database,
    fence: &AssistantTurnFence,
    user_id: &str,
    api_key_id: Option<&str>,
    api_key_name: Option<&str>,
    ip_address: Option<&str>,
    user_agent: Option<&str>,
    route: &str,
) -> AppResult<()> {
    if !crate::services::feature_flag_service::personal_flag_enabled(
        db,
        user_id,
        crate::services::feature_flag_service::ASSISTANT_LIVE_TURN_GATE_FLAG_KEY,
    )
    .await?
    {
        return Ok(());
    }
    if let Err(error) = fence.require_live() {
        let sample_id = format!("{user_id}:{}", fence.conversation_id);
        if crate::telemetry::should_sample_event("assistant_turn_gate_refused", &sample_id, 10) {
            let _ = audit_service::log_actor_event(
                db.clone(),
                &audit_service::AuditActor {
                    user_id: user_id.to_owned(),
                    ip_address: ip_address.map(str::to_owned),
                    user_agent: user_agent.map(str::to_owned),
                    api_key_id: api_key_id.map(str::to_owned),
                    api_key_name: api_key_name.map(str::to_owned),
                },
                "assistant_turn_gate_refused",
                Some(json!({
                    "route": route,
                    "conversation_id": fence.conversation_id,
                    "turn_id": fence.turn_id,
                    "reason": fence.refusal_reason(),
                    "sample_percent": 10,
                })),
            )
            .await;
        }
        return Err(error);
    }
    Ok(())
}

pub const PENDING_SECONDS: i64 = 15 * 60;
pub const ACTION_SECONDS: i64 = 10 * 60;
pub const DECISION_NARRATION: &str =
    "Then tell the user in one or two plain sentences what you did and the result.";

/// How risky an assistant action is. The owner's access mode decides which
/// classes need a single-use action card; policy floors (webhook automations,
/// machine and saved-login confirmation) apply in Auto and Ask every time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OwnerActionClass {
    Read,
    Change,
    Sensitive,
    Destructive,
}

impl OwnerActionClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Change => "change",
            Self::Sensitive => "sensitive",
            Self::Destructive => "destructive",
        }
    }
}

/// Whether the owner's access mode asks before an action of `class`.
/// Full access never asks; Auto asks for sensitive and destructive actions;
/// Ask every time asks for every action except reads.
pub fn owner_confirmation_required(
    mode: crate::models::assistant_settings::AccessMode,
    class: OwnerActionClass,
) -> bool {
    use crate::models::assistant_settings::AccessMode;
    match mode {
        AccessMode::AskEveryTime => class != OwnerActionClass::Read,
        AccessMode::Nyxmode => matches!(
            class,
            OwnerActionClass::Sensitive | OwnerActionClass::Destructive
        ),
        AccessMode::Autopilot => false,
    }
}

/// The access mode that governs `chat`'s agent: its owner-set override, else
/// the acting person's mode. `chat.agent_id` is resolved by the server from
/// the thread's credential binding, never from tool input, so the agent is
/// read by its ID; this also resolves organization-owned agents, whose rows
/// belong to the organization rather than the acting person.
pub async fn effective_access_mode(
    db: &Database,
    chat: &ChatAuthority,
) -> AppResult<crate::models::assistant_settings::AccessMode> {
    effective_mode_for_agent(db, &chat.user_id, &chat.agent_id).await
}

async fn effective_mode_for_agent(
    db: &Database,
    user: &str,
    agent: &str,
) -> AppResult<crate::models::assistant_settings::AccessMode> {
    let override_mode = if agent.is_empty() {
        None
    } else {
        db.collection::<crate::models::assistant_agent::AssistantAgent>(
            crate::models::assistant_agent::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": agent})
        .await?
        .and_then(|agent| agent.access_mode_override)
    };
    match override_mode {
        Some(mode) => Ok(mode),
        None => Ok(super::assistant_settings_service::get(db, user)
            .await?
            .access_mode),
    }
}

pub(super) type AccessModeCache =
    std::collections::HashMap<(String, String), crate::models::assistant_settings::AccessMode>;

pub(super) async fn cached_full_access(
    db: &Database,
    modes: &mut AccessModeCache,
    user: &str,
    agent: &str,
) -> AppResult<bool> {
    let identity = (user.to_owned(), agent.to_owned());
    let mode = match modes.get(&identity) {
        Some(mode) => *mode,
        None => {
            let mode = effective_mode_for_agent(db, user, agent).await?;
            modes.insert(identity, mode);
            mode
        }
    };
    Ok(mode == crate::models::assistant_settings::AccessMode::Autopilot)
}

/// Retire owner asks on policy writes and once at startup. Walk only pending
/// owner-action rows, in bounded batches, and resolve each governing mode once.
/// Retirement is expiry; the action has not run and has no success receipt yet.
pub async fn retire_full_access_confirmations(
    db: &Database,
    user: Option<&str>,
    agent: Option<&str>,
) -> AppResult<()> {
    let sweep_time = bson::DateTime::now();
    let mut filter = doc! {
        "kind": "action", "decider": "user", "status": "pending",
        "created_at": {"$lte": sweep_time},
    };
    if let Some(user) = user {
        filter.insert("user_id", user);
    }
    if let Some(agent) = agent {
        // Legacy asks without a requester ID are resolved from their one thread.
        filter.insert(
            "requester_agent_id",
            doc! {"$in": [agent, bson::Bson::Null]},
        );
    }
    let collection = db.collection::<AssistantAcknowledgement>(ACKS);
    let mut modes = AccessModeCache::new();
    loop {
        let pending: Vec<_> = collection
            .find(filter.clone())
            .sort(doc! {"_id": 1})
            .limit(100)
            .await?
            .try_collect()
            .await?;
        let Some(last) = pending.last() else { break };
        filter.insert("_id", doc! {"$gt": &last.id});
        let mut ids = Vec::new();
        for ask in pending {
            let requester = match ask.requester_agent_id {
                Some(id) => id,
                None => {
                    let thread = db
                        .collection::<AssistantConversation>(CONVERSATIONS)
                        .find_one(doc! {"_id": &ask.conversation_id, "user_id": &ask.user_id})
                        .await?;
                    let Some(thread) = thread else { continue };
                    thread.agent_id.unwrap_or_default()
                }
            };
            if agent.is_some_and(|target| target != requester) {
                continue;
            }
            if cached_full_access(db, &mut modes, &ask.user_id, &requester).await? {
                ids.push(ask.id);
            }
        }
        if !ids.is_empty() {
            collection
                .update_many(
                    doc! {"_id": {"$in": ids}, "status": "pending"},
                    doc! {"$set": {"status": "expired", "expires_at": sweep_time}},
                )
                .await?;
        }
    }
    // Proxy approvals can exist without an acknowledgement; their owning service
    // resolves the actual chat credential and applies its normal expiry lifecycle.
    super::approval_service::retire_full_access_chat_requests(
        db, user, agent, &mut modes, sweep_time,
    )
    .await?;
    Ok(())
}

/// The closed risk class of a native (`nyxid__`), meta (`nyx__`) or machine
/// tool, or `None` for a name outside those inventories. Service operations
/// are classified from their catalog/HTTP effects by [`service_action_class`].
pub fn known_action_class(tool: &str, args: &Value) -> Option<OwnerActionClass> {
    use OwnerActionClass::*;
    if let Some(operation) = crate::services::machine_tools::operation(tool) {
        // Work on a machine the owner granted is the agent's ordinary job;
        // each machine's own confirmation setting (`machine_confirm`) is the
        // owner's policy for its riskier changes. Ask every time still asks
        // for every change. Webhook policy keeps its destructive contract in
        // the machine adapter.
        return Some(
            if crate::services::machine_service::changing(operation, args) {
                Change
            } else {
                Read
            },
        );
    }
    if let Some(name) = tool.strip_prefix("nyx__") {
        return Some(match name {
            "search_tools"
            | "discover_services"
            | "list_connected_services"
            | "wait_for_connection"
            | "ssh_list_services"
            | "oracle_pools"
            | "oracle_result"
            | "oracle_session"
            | "attachment_read"
            | "machine_list"
            | "saved_logins" => Read,
            "connect_service" | "oracle_ask" | "oracle_attach" | "oracle_extract" => Change,
            // A wrapped service call is classified from the operation it reaches.
            "call_tool" => Read,
            "ssh_exec" => Destructive,
            _ => return None,
        });
    }
    let name = tool.strip_prefix("nyxid__")?;
    use super::{assistant_account_tools as account, assistant_team_tools as team};
    if account::TOOL_NAMES.contains(&name) {
        return Some(if account::read_only(name) {
            Read
        } else if account::destructive(name) {
            Destructive
        } else if name == "set_service_enabled" {
            Change
        } else {
            // Agent keys, credential bindings, channel bots, routes and
            // approval policy carry authority.
            Sensitive
        });
    }
    if !team::TOOL_NAMES.contains(&name) && !team::AGENT_TOOL_NAMES.contains(&name) {
        return None;
    }
    let grants = |fields: &[&str]| {
        fields.iter().any(|field| match &args[*field] {
            Value::Array(items) => !items.is_empty(),
            Value::Bool(value) => *value,
            Value::Null => false,
            _ => true,
        })
    };
    Some(if team::read_only(name) {
        Read
    } else if team::destructive(name)
        || (name == "update_group"
            && (args["leave"].as_bool() == Some(true)
                || args.get("participant_user_ids").is_some()))
    {
        Destructive
    } else if matches!(
        name,
        "grant_subagent"
            | "set_guest_access"
            | "set_agent_operations"
            | "set_agent_skills"
            | "connect_channel_bot"
            | "link_channel_bot"
            | "update_channel_access"
            | "update_channel_chat"
            | "create_schedule"
            | "update_schedule"
            | "machine_pair"
    ) || (name == "spawn_subagent"
        && grants(&["services", "account_read", "machines", "logins"]))
        || (name == "update_subagent" && grants(&["model"]))
        || (name == "decide_permission" && args["decision"].as_str() == Some("allow"))
    {
        // Each widens what an agent, chat or automation may do.
        Sensitive
    } else {
        Change
    })
}

/// The risk class of a named tool; unknown names are treated as changes.
pub fn classify_owner_action(tool: &str, args: &Value) -> OwnerActionClass {
    known_action_class(tool, args).unwrap_or(OwnerActionClass::Change)
}

/// A service operation's risk class from its effects: reads, ordinary
/// changes, or destructive operations (DELETE, catalog-destructive, or any
/// call with a method override, whose real effect is unknown).
pub fn service_action_class(read_only: bool, destructive: bool) -> OwnerActionClass {
    if destructive {
        OwnerActionClass::Destructive
    } else if read_only {
        OwnerActionClass::Read
    } else {
        OwnerActionClass::Change
    }
}

/// Why an action needs the owner's card. A single card satisfies every
/// reason at once, and is consumed exactly once.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ConfirmationNeed {
    pub class: OwnerActionClass,
    /// The owner's access mode asks for this class.
    pub mode: Option<crate::models::assistant_settings::AccessMode>,
    /// The webhook automation's confirmation policy asks.
    pub webhook: bool,
    /// Another policy floor (machine or saved-login confirmation) asks.
    pub policy: bool,
}

/// Effects of one action as seen by the webhook confirmation policy, which
/// predates access modes and keeps its own contract.
#[derive(Clone, Copy, Debug)]
pub struct ActionFacts {
    pub class: OwnerActionClass,
    pub read_only: bool,
    pub destructive: bool,
}

impl ActionFacts {
    pub fn of(class: OwnerActionClass) -> Self {
        Self {
            class,
            read_only: class == OwnerActionClass::Read,
            destructive: class == OwnerActionClass::Destructive,
        }
    }
}

/// Decide whether an action needs a card: the webhook floor, a caller policy
/// floor, or the governing access mode. Guests never confirm cards and are
/// refused by their own gates, so only floors apply to their turns. Reads
/// are never asked by the access mode and cost no settings lookup.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AutoAllowed {
    pub class: OwnerActionClass,
    pub mode: crate::models::assistant_settings::AccessMode,
    pub reason: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gate {
    Proceed,
    AutoAllowed(AutoAllowed),
    Ask(ConfirmationNeed),
}

impl Gate {
    pub fn ask(self) -> Option<ConfirmationNeed> {
        match self {
            Self::Ask(need) => Some(need),
            _ => None,
        }
    }
    pub fn auto_allowed(self) -> Option<AutoAllowed> {
        match self {
            Self::AutoAllowed(auto) => Some(auto),
            _ => None,
        }
    }
}

pub async fn confirmation_needed(
    db: &Database,
    chat: &ChatAuthority,
    facts: ActionFacts,
    policy_floor: bool,
) -> AppResult<Gate> {
    let webhook = webhook_confirmation_required(chat, facts.read_only, facts.destructive);
    let effective =
        if chat.guest || (facts.class == OwnerActionClass::Read && !webhook && !policy_floor) {
            None
        } else {
            Some(effective_access_mode(db, chat).await?)
        };
    if effective == Some(crate::models::assistant_settings::AccessMode::Autopilot) {
        let reason = if webhook {
            "webhook"
        } else if policy_floor {
            "policy"
        } else {
            "access_mode"
        };
        if webhook
            || policy_floor
            || matches!(
                facts.class,
                OwnerActionClass::Sensitive | OwnerActionClass::Destructive
            )
        {
            return Ok(Gate::AutoAllowed(AutoAllowed {
                class: facts.class,
                mode: effective.unwrap(),
                reason,
            }));
        }
        return Ok(Gate::Proceed);
    }
    let mode = effective.filter(|mode| owner_confirmation_required(*mode, facts.class));
    if webhook || policy_floor || mode.is_some() {
        return Ok(Gate::Ask(ConfirmationNeed {
            class: facts.class,
            mode,
            webhook,
            policy: policy_floor,
        }));
    }
    Ok(Gate::Proceed)
}

/// Best-effort transcript receipt after a successful action. This record never
/// grants execution authority and must never be consumed as an owner card.
pub async fn record_auto_allowed(
    db: &Database,
    chat: &ChatAuthority,
    tool: &str,
    args: &Value,
    auto: &AutoAllowed,
    summary: &str,
) {
    record_auto_allowed_with_link(db, chat, tool, args, auto, summary, None).await;
}

#[allow(clippy::too_many_arguments)]
pub async fn record_auto_allowed_with_link(
    db: &Database,
    chat: &ChatAuthority,
    tool: &str,
    args: &Value,
    auto: &AutoAllowed,
    summary: &str,
    link: Option<&str>,
) {
    if chat.guest {
        return;
    }
    let now = Utc::now();
    let row = AssistantAcknowledgement {
        id: Uuid::new_v4().to_string(),
        conversation_id: chat.conversation_id.clone(),
        user_id: chat.user_id.clone(),
        api_key_id: chat.api_key_id.clone(),
        requester_agent_id: Some(chat.agent_id.clone()),
        kind: "action".into(),
        status: "allowed".into(),
        decider: "user".into(),
        decided_by: Some("access_mode".into()),
        confirmation: Some(
            crate::models::assistant_acknowledgement::ActionConfirmation {
                risk: auto.class.as_str().into(),
                access_mode: Some(auto.mode),
                reason: auto.reason.into(),
            },
        ),
        summary: summary.into(),
        link: link.map(str::to_owned),
        tool_name: Some(tool.into()),
        arguments_digest: Some(arguments_digest(args)),
        created_at: now,
        decided_at: Some(now),
        expires_at: now,
        authored_skill: None,
        service_id: None,
        service_slug: None,
        service_name: None,
        platform: false,
        operation_selection: None,
        operation_contract_digest: None,
        skill_selection: None,
        requested_turn_id: chat.turn_id.clone(),
        voice_request_id: None,
        continuation_receipt_id: None,
        trigger_run_id: None,
        request_excerpt: None,
        reason: None,
        machine_context: None,
    };
    if db
        .collection::<AssistantAcknowledgement>(ACKS)
        .insert_one(&row)
        .await
        .is_err()
    {
        tracing::warn!(conversation_id = %chat.conversation_id, tool, code = "receipt_insert_failed", "Automatic action receipt write failed");
        return;
    }
    audit_service::log_async(
        db.clone(),
        Some(chat.user_id.clone()),
        "assistant_action_auto_allowed".into(),
        Some(
            json!({"conversation_id": chat.conversation_id, "agent_id": chat.agent_id,
            "tool": tool, "risk": auto.class.as_str(), "access_mode": auto.mode, "reason": auto.reason}),
        ),
        None,
        None,
        Some(chat.api_key_id.clone()),
        None,
    );
}

/// Require the owner's single-use card for an action that needs one.
/// Retrying with an approved `acknowledgement_id` consumes it (once) and
/// returns `None`; otherwise the refusal to return to the model.
pub async fn confirm_action(
    db: &Database,
    chat: &ChatAuthority,
    tool: &str,
    args: &Value,
    need: &ConfirmationNeed,
    summary: &str,
) -> AppResult<Option<Value>> {
    confirm_action_with(db, chat, tool, args, need, summary, InvalidCard::Refuse).await
}

/// What a call presenting an unusable card id gets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InvalidCard {
    /// `acknowledgement_invalid`: the agent must ask before a new card.
    Refuse,
    /// A fresh card for these exact arguments (machine confirmation's
    /// contract outside webhook automations).
    Reissue,
}

pub async fn confirm_action_with(
    db: &Database,
    chat: &ChatAuthority,
    tool: &str,
    args: &Value,
    need: &ConfirmationNeed,
    summary: &str,
    on_invalid: InvalidCard,
) -> AppResult<Option<Value>> {
    if let Some(id) = args["acknowledgement_id"].as_str() {
        if consume_action(db, chat, id, tool, args).await? {
            return Ok(None);
        }
        if on_invalid == InvalidCard::Refuse {
            return Ok(Some(json!({
                "error": "acknowledgement_invalid", "kind": "action",
                "instructions": "This action card is missing, expired, used or does not match \
                    the call. Ask the user before requesting a new action card.",
            })));
        }
    }
    let summary = if need.webhook {
        format!("Webhook automation requests: {summary}. Review this action before allowing it.")
    } else {
        summary.to_owned()
    };
    let (card, _) = request_tracked_with_machine_context(
        db,
        chat,
        Request {
            kind: "action",
            service: None,
            tool: Some(tool),
            arguments: Some(args),
            summary: &summary,
            platform: false,
        },
        None,
        Some(
            crate::models::assistant_acknowledgement::ActionConfirmation {
                risk: need.class.as_str().into(),
                access_mode: need.mode,
                reason: if need.webhook {
                    "webhook"
                } else if need.mode.is_some() {
                    "access_mode"
                } else {
                    "policy"
                }
                .into(),
            },
        ),
    )
    .await?;
    Ok(Some(refusal(&card)))
}

#[cfg(test)]
mod access_mode_tests {
    use super::*;
    use crate::models::assistant_settings::AccessMode::*;

    #[test]
    fn mode_class_matrix() {
        use OwnerActionClass::*;
        for (mode, class, expected) in [
            (Autopilot, Read, false),
            (Autopilot, Change, false),
            (Autopilot, Sensitive, false),
            (Autopilot, Destructive, false),
            (Nyxmode, Read, false),
            (Nyxmode, Change, false),
            (Nyxmode, Sensitive, true),
            (Nyxmode, Destructive, true),
            (AskEveryTime, Read, false),
            (AskEveryTime, Change, true),
            (AskEveryTime, Sensitive, true),
            (AskEveryTime, Destructive, true),
        ] {
            assert_eq!(
                owner_confirmation_required(mode, class),
                expected,
                "{mode:?} {class:?}"
            );
        }
    }

    #[test]
    fn classifier_covers_tool_families() {
        use OwnerActionClass::*;
        let none = json!({});
        for (tool, args, class) in [
            ("nyx__machine_exec", json!({"command": "ls"}), Change),
            ("nyx__machine_read_file", json!({"path": "a"}), Read),
            ("nyx__machine_fill_login", json!({"login": "l"}), Change),
            ("nyx__ssh_exec", none.clone(), Destructive),
            ("nyx__oracle_ask", none.clone(), Change),
            ("nyx__connect_service", none.clone(), Change),
            ("nyx__search_tools", none.clone(), Read),
            ("nyxid__delete_agent_key", none.clone(), Destructive),
            ("nyxid__set_service_enabled", none.clone(), Change),
            ("nyxid__set_approval_mode", none.clone(), Sensitive),
            ("nyxid__list_agent_keys", none.clone(), Read),
            ("nyxid__grant_subagent", none.clone(), Sensitive),
            ("nyxid__spawn_subagent", json!({"name": "a"}), Change),
            (
                "nyxid__spawn_subagent",
                json!({"services": ["github"]}),
                Sensitive,
            ),
            ("nyxid__update_subagent", json!({"persona": "calm"}), Change),
            (
                "nyxid__update_subagent",
                json!({"model": "nyxagent/fast"}),
                Sensitive,
            ),
            ("nyxid__update_group", json!({"name": "x"}), Change),
            ("nyxid__update_group", json!({"leave": true}), Destructive),
            ("nyxid__delete_schedule", none.clone(), Destructive),
            ("nyxid__message_subagent", none.clone(), Change),
            ("nyxid__list_schedules", none.clone(), Read),
        ] {
            assert_eq!(
                known_action_class(tool, &args),
                Some(class),
                "{tool} {args}"
            );
        }
        assert_eq!(known_action_class("github__list_repos", &none), None);
        assert_eq!(known_action_class("nyxid__not_a_tool", &none), None);
    }

    #[test]
    fn every_registered_tool_has_an_explicit_class() {
        let none = json!({});
        let native = crate::services::assistant_account_tools::TOOL_NAMES
            .iter()
            .chain(crate::services::assistant_team_tools::TOOL_NAMES)
            .chain(crate::services::assistant_team_tools::AGENT_TOOL_NAMES)
            .map(|name| format!("nyxid__{name}"));
        let meta = crate::services::mcp_service::generate_tool_definitions(&[], None)
            .into_iter()
            .map(|tool| tool.name)
            .chain(
                crate::services::machine_tools::definitions()
                    .into_iter()
                    .map(|tool| tool.name),
            );
        for name in native.chain(meta) {
            assert!(
                known_action_class(&name, &none).is_some(),
                "unclassified tool: {name}"
            );
        }
    }

    #[test]
    fn service_effects_map_to_classes() {
        assert_eq!(service_action_class(true, false), OwnerActionClass::Read);
        assert_eq!(service_action_class(false, false), OwnerActionClass::Change);
        assert_eq!(
            service_action_class(true, true),
            OwnerActionClass::Destructive
        );
    }

    #[test]
    fn webhook_policy_floor_remains_independent_of_owner_mode() {
        let mut chat = ChatAuthority {
            org_agent_access: None,
            turn_id: None,
            turn_stopped: false,
            turn_live: true,
            machine_node_ids: vec![],
            saved_login_ids: vec![],
            conversation_id: "c".into(),
            user_id: "u".into(),
            api_key_id: "k".into(),
            role: AgentRole::Orchestrator,
            agent_id: "a".into(),
            agent_name: "NyxBot".into(),
            guest: false,
            confirmation_policy: Some(crate::models::trigger_schedule::ConfirmationPolicy::Changes),
        };
        assert!(webhook_confirmation_required(&chat, false, false));
        chat.confirmation_policy =
            Some(crate::models::trigger_schedule::ConfirmationPolicy::Destructive);
        assert!(!webhook_confirmation_required(&chat, false, false));
        assert!(webhook_confirmation_required(&chat, false, true));
    }
}

#[derive(Clone)]
pub struct ChatAuthority {
    pub org_agent_access: Option<std::sync::Arc<super::org_agent_service::RequestAccess>>,
    pub turn_id: Option<String>,
    pub turn_stopped: bool,
    pub turn_live: bool,
    pub machine_node_ids: Vec<String>,
    pub saved_login_ids: Vec<String>,
    pub conversation_id: String,
    pub user_id: String,
    pub api_key_id: String,
    pub role: AgentRole,
    /// The agent this thread belongs to (NyxBot or a specialist).
    pub agent_id: String,
    pub agent_name: String,
    /// The thread's newest turn acts for a channel chat guest (not the
    /// owner): service calls only as far as the owner lets guests use each
    /// service (`AssistantAgent::guest_access`); see `guest_refusal`.
    pub guest: bool,
    pub confirmation_policy: Option<crate::models::trigger_schedule::ConfirmationPolicy>,
}
impl ChatAuthority {
    /// NyxBot threads run with Full access; specialists only with their grants.
    pub fn is_orchestrator(&self) -> bool {
        self.role == AgentRole::Orchestrator
    }

    /// Only the owner's NyxBot may widen access or create an access request.
    /// Keeping this predicate here makes every acknowledgement-producing path
    /// share the same authority decision.
    pub fn access_request_allowed(&self) -> bool {
        self.is_orchestrator() && !self.guest
    }

    /// Compatibility policy used by owner-controlled settings. Strict mode
    /// denies specialist requests; routed mode permits the existing
    /// orchestrator-card path while never granting specialist authority.
    pub fn access_request_allowed_with_mode(
        &self,
        mode: crate::models::assistant_settings::SpecialistAccessRequests,
    ) -> bool {
        self.access_request_allowed()
            || (matches!(
                mode,
                crate::models::assistant_settings::SpecialistAccessRequests::RoutedToNyxbot
            ) && !self.guest)
    }

    pub fn turn_fence(&self) -> AssistantTurnFence {
        AssistantTurnFence {
            conversation_id: self.conversation_id.clone(),
            turn_id: self.turn_id.clone(),
            turn_live: self.turn_live,
            turn_stopped: self.turn_stopped,
        }
    }

    pub async fn ensure_live_turn(
        &self,
        db: &Database,
        ip_address: Option<&str>,
        user_agent: Option<&str>,
        route: &str,
    ) -> AppResult<()> {
        enforce_turn_gate(
            db,
            &self.turn_fence(),
            &self.user_id,
            Some(&self.api_key_id),
            None,
            ip_address,
            user_agent,
            route,
        )
        .await
    }
}
impl std::fmt::Debug for ChatAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChatAuthority { [REDACTED] }")
    }
}

/// What NyxBot is told when a guest turn calls a tool: NyxBot holds all of
/// the owner's services, so it uses none for other people.
pub fn orchestrator_guest_refusal() -> Value {
    json!({"error": "owner_only", "instructions": "You are answering someone other than the \
        owner, so you use no tools here: answer from the conversation. If people in this chat \
        should use certain services, the owner can give the chat a specialist with just those \
        services (ask NyxBot in NyxID)."})
}

/// What a guest turn (someone other than the owner) is told when it asks
/// for something only the owner can ask for.
pub fn guest_refusal() -> Value {
    json!({"error": "owner_only", "instructions": "You are answering someone other than the \
        owner. Only the owner can ask for account actions, new connections, more access, \
        anything that needs their approval, or more than the owner lets guests do with a \
        service. Help with your services otherwise, and say that only the bot's owner can \
        ask for that."})
}

pub fn access_request_denied() -> Value {
    json!({
        "error": "access_request_denied",
        "instructions": "Only NyxBot can request or grant access. Report the need to NyxBot in your reply."
    })
}

/// Record a metadata-only denial. Callers use this before returning the stable
/// refusal so denied attempts remain visible without storing arguments or
/// credentials.
pub async fn audit_access_request_denied(
    db: &Database,
    chat: &ChatAuthority,
    target: Option<&str>,
    scope: &str,
) {
    let _ = audit_service::log_actor_event(
        db.clone(),
        &audit_service::AuditActor {
            user_id: chat.user_id.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: Some(chat.api_key_id.clone()),
            api_key_name: None,
        },
        "assistant_access_request_denied",
        Some(
            json!({"requester_agent": chat.agent_id, "thread": chat.conversation_id,
            "key": chat.api_key_id, "target": target, "scope": scope,
            "decision": "denied"}),
        ),
    )
    .await;
    if let Ok(agents) = super::assistant_team_service::agents(db, &chat.user_id, false).await
        && let Some(home) = agents
            .into_iter()
            .find(|a| a.is_nyxbot())
            .and_then(|a| a.home_conversation_id)
    {
        // One notice per specialist per hour. The claim and the message
        // sequence allocation are a single atomic update of NyxBot's home
        // thread, so concurrent denials cannot both notify.
        let now = Utc::now();
        let cutoff = bson::DateTime::from_chrono(now - chrono::Duration::hours(1));
        let marker = format!("specialist_denial_notices.{}", chat.agent_id);
        let claimed = db
            .collection::<crate::models::assistant_conversation::AssistantConversation>(
                crate::models::assistant_conversation::COLLECTION_NAME,
            )
            .find_one_and_update(
                doc! {"_id": &home, "user_id": &chat.user_id, "$or": [
                    {marker.as_str(): {"$exists": false}},
                    {marker.as_str(): {"$lt": cutoff}},
                ]},
                doc! {"$set": {marker.as_str(): bson::DateTime::from_chrono(now)},
                "$inc": {"message_count": 1}},
            )
            .await;
        let Ok(Some(row)) = claimed else {
            return;
        };
        let text = format!(
            "specialist_access_denied: {} needs {} for {}",
            chat.agent_name,
            scope,
            target.unwrap_or("account")
        );
        let _ = super::assistant_nyxagent::push_events(
            db,
            &chat.user_id,
            &home,
            vec![crate::models::assistant_conversation::AgentEvent {
                id: Uuid::new_v4().to_string(),
                kind: "permission_requested".into(),
                text: text.clone(),
                agent_id: Some(chat.agent_id.clone()),
                question_key: None,
                reply_to: Vec::new(),
                created_at: now,
            }],
        )
        .await;
        let message = crate::models::assistant_message::AssistantMessage {
            author: None,
            activity_total: 0,
            continuation_of: None,
            retry_of: None,
            specialist_id: Some(chat.agent_id.clone()),
            steering: None,
            error_detail: None,
            upstream_error_code: None,
            voice: None,
            execution_pending: false,
            id: Uuid::new_v4().to_string(),
            conversation_id: home,
            user_id: chat.user_id.clone(),
            seq: row.message_count + 1,
            turn_id: format!("access-denied-{}", Uuid::new_v4()),
            role: "event".into(),
            text,
            status: "completed".into(),
            error_code: Some("specialist_access_denied".into()),
            created_at: now,
            activities: Vec::new(),
            attachments: Vec::new(),
            origin: None,
            via: None,
        };
        let _ = db
            .collection::<crate::models::assistant_message::AssistantMessage>(
                crate::models::assistant_message::COLLECTION_NAME,
            )
            .insert_one(message)
            .await;
    }
}

/// An org permission request may use the person's NyxBot only while that
/// person maintains the agent. Otherwise a maintainer must make the change.
const ORG_MAINTAINER_REQUIRED: &str = "This organization agent needs a permission or skill change that you cannot maintain. Ask an organization maintainer to update the agent; your personal NyxBot cannot approve it on your behalf.";

pub fn organization_agent_refusal() -> Value {
    json!({
        "error": "organization_grant_required",
        "instructions": ORG_MAINTAINER_REQUIRED
    })
}

/// What a guest turn is told when a service call goes beyond what the owner
/// lets guests do with that service.
pub fn guest_service_refusal(service: &str, access: GuestAccess) -> Value {
    let allowed = match access {
        GuestAccess::Read => "only look things up with",
        _ => "look things up, create and act with, but not change or delete anything in,",
    };
    json!({"error": "owner_only", "service": service, "guest_access": access.as_str(),
        "instructions": format!("You are answering someone other than the owner, who may \
        {allowed} {service}. Help within that, and say that only the bot's owner can ask for \
        more; the owner can change it by asking NyxBot.")})
}

/// What a guest turn is told when a service call carries a method override.
pub fn guest_method_override_refusal(service: &str) -> Value {
    json!({"error": "owner_only", "service": service,
        "instructions": format!("You are answering someone other than the owner: calls to \
        {service} for them use the operation's own HTTP method, never a method override \
        (an X-HTTP-Method-Override header, a _method field, or a method field naming another \
        verb), and a request body sent as JSON must be valid JSON. Call it that way, or say \
        that only the bot's owner can ask for that.")})
}

pub async fn for_key(
    db: &Database,
    user: &str,
    key: Option<&str>,
) -> AppResult<Option<ChatAuthority>> {
    Box::pin(for_key_with_access(db, user, key, None)).await
}

pub async fn for_key_with_access(
    db: &Database,
    user: &str,
    key: Option<&str>,
    access: Option<&std::sync::Arc<super::org_agent_service::RequestAccess>>,
) -> AppResult<Option<ChatAuthority>> {
    let Some(key) = key else { return Ok(None) };
    let Some(row) = db
        .collection::<bson::Document>(CREDENTIALS)
        .find_one(doc! {"user_id": user, "api_key_id": key})
        .projection(doc! {"conversation_id": 1})
        .await?
    else {
        return Ok(None);
    };
    let conversation_id = row.get_str("conversation_id").map_err(|_| not_found())?;
    let (conversation, agent) = if let Some(access) = access {
        // Authentication already resolved live membership. Retain the private
        // thread/owner/agent fences while reusing that request's snapshot.
        let conversation = access.conversation(db, user, conversation_id).await?;
        let agent = access
            .agent(
                db,
                user,
                conversation.agent_id.as_deref().ok_or_else(not_found)?,
            )
            .await?;
        (conversation, agent)
    } else {
        let conversation = super::assistant_nyxagent::get(db, user, conversation_id).await?;
        let agent =
            super::assistant_team_service::agent_for_conversation(db, &conversation).await?;
        (conversation, agent)
    };
    if agent.destroyed_at.is_some() {
        return Err(not_found());
    }
    let confirmation_policy = if let Some(run_id) = conversation
        .active_turn
        .as_ref()
        .and_then(|turn| turn.trigger_run_id.as_deref())
    {
        db.collection::<crate::models::trigger_run::TriggerRun>(
            crate::models::trigger_run::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": run_id, "user_id": user})
        .await?
        .and_then(|run| run.confirmation_policy)
    } else {
        None
    };
    let turn_live = super::assistant_nyxagent::live_turn(&conversation, Utc::now()).is_some();
    Ok(Some(ChatAuthority {
        org_agent_access: access.cloned(),
        turn_id: conversation.active_turn.as_ref().map(|t| t.turn_id.clone()),
        turn_stopped: conversation
            .active_turn
            .as_ref()
            .is_none_or(|t| t.stop_requested),
        turn_live,
        machine_node_ids: agent.machine_node_ids.clone(),
        saved_login_ids: agent.saved_login_ids.clone(),
        user_id: user.into(),
        api_key_id: key.into(),
        conversation_id: conversation_id.into(),
        role: if agent.is_nyxbot() {
            AgentRole::Orchestrator
        } else {
            AgentRole::Subagent
        },
        agent_id: agent.id,
        agent_name: agent.name,
        guest: conversation.guest_turn,
        confirmation_policy,
    }))
}

fn not_found() -> AppError {
    AppError::NotFound("Acknowledgement not found".into())
}

pub fn arguments_digest(arguments: &Value) -> String {
    fn canonical(value: &Value) -> Value {
        match value {
            Value::Object(map) => {
                let sorted: std::collections::BTreeMap<_, _> = map
                    .iter()
                    .map(|(key, value)| (key.clone(), canonical(value)))
                    .collect();
                Value::Object(sorted.into_iter().collect())
            }
            Value::Array(values) => Value::Array(values.iter().map(canonical).collect()),
            _ => value.clone(),
        }
    }
    let mut value = arguments.clone();
    if let Some(map) = value.as_object_mut() {
        map.remove("acknowledgement_id");
    }
    hex::encode(Sha256::digest(canonical(&value).to_string().as_bytes()))
}

pub(crate) async fn fence(
    db: &Database,
    chat: &ChatAuthority,
    session: &mut ClientSession,
) -> AppResult<(
    AssistantConversation,
    ApiKey,
    Option<std::sync::Arc<super::org_agent_service::RequestAccess>>,
)> {
    let credential = db
        .collection::<bson::Document>(CREDENTIALS)
        .update_one(
            doc! {"user_id": &chat.user_id, "conversation_id": &chat.conversation_id,
            "api_key_id": &chat.api_key_id},
            doc! {"$inc": {"authority_fence": 1}},
        )
        .session(&mut *session)
        .await?;
    if credential.matched_count != 1 {
        return Err(not_found());
    }
    let row = db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one_and_update(
            doc! {"_id": &chat.conversation_id, "user_id": &chat.user_id},
            doc! {"$inc": {"authority_fence": 1}},
        )
        .session(&mut *session)
        .await?
        .ok_or_else(not_found)?;
    let key = db
        .collection::<ApiKey>(KEYS)
        .find_one(doc! {"_id": &chat.api_key_id, "user_id": &chat.user_id, "is_active": true})
        .session(&mut *session)
        .await?
        .ok_or_else(not_found)?;
    if key.expires_at.is_some_and(|expiry| expiry <= Utc::now()) {
        return Err(not_found());
    }
    // Cards are execution authority. Human decisions and internal NyxBot paths
    // must enforce the same live org/group ACL as a new tool request.
    let access = if let Some(owner) = row.agent_owner_id.as_deref() {
        let access = super::org_agent_service::key_access_with_snapshot(
            db,
            &row.user_id,
            Some(owner),
            chat.org_agent_access.as_ref(),
        )
        .await?;
        if let Some(access) = access.as_ref() {
            super::org_group_service::check_thread_participation(db, &row, access).await?;
            if row.group_request_id.is_some()
                && let Some(id) = row.group_id.as_deref()
            {
                let group =
                    super::org_group_service::get(db, &row.user_id, id, Some(access)).await?;
                super::org_group_service::fence(db, &group, session).await?;
            }
        }
        access
    } else {
        None
    };
    Ok((row, key, access))
}

pub async fn expire(db: &Database, user: &str, conversation: &str) -> AppResult<()> {
    db.collection::<AssistantAcknowledgement>(ACKS)
        .update_many(
            doc! {"user_id": user, "conversation_id": conversation,
            "decided_by": {"$ne": "access_mode"},
            "expires_at": {"$lte": bson::DateTime::now()}, "$or": [
                {"status": "pending"}, {"status": "allowed", "kind": "action"},
            ]},
            doc! {"$set": {"status": "expired"}},
        )
        .await?;
    Ok(())
}

pub async fn history(
    db: &Database,
    user: &str,
    conversation: &str,
) -> AppResult<Vec<AssistantAcknowledgement>> {
    history_for_messages(db, user, conversation, None).await
}

/// Keep decisions beside the messages being displayed, including older pages.
/// The latest twenty remain visible even on an empty or narrowly loaded page.
pub async fn history_for_messages(
    db: &Database,
    user: &str,
    conversation: &str,
    oldest_message: Option<DateTime<Utc>>,
) -> AppResult<Vec<AssistantAcknowledgement>> {
    super::assistant_nyxagent::get(db, user, conversation).await?;
    expire(db, user, conversation).await?;
    let collection = db.collection::<AssistantAcknowledgement>(ACKS);
    let mut rows: Vec<_> = collection
        .find(doc! {
            "user_id": user, "conversation_id": conversation, "status": "pending",
        })
        .await?
        .try_collect()
        .await?;
    let decided: Vec<_> = collection
        .find(doc! {
            "user_id": user, "conversation_id": conversation, "status": {"$ne": "pending"},
        })
        .sort(doc! {"created_at": -1, "_id": -1})
        .limit(20)
        .await?
        .try_collect()
        .await?;
    rows.extend(decided);
    if let Some(oldest) = oldest_message {
        let visible: Vec<_> = collection
            .find(doc! {
                "user_id": user, "conversation_id": conversation,
                "status": {"$ne": "pending"},
                "created_at": {"$gte": bson::DateTime::from_chrono(oldest)},
            })
            .await?
            .try_collect()
            .await?;
        let mut ids: std::collections::HashSet<_> = rows.iter().map(|r| r.id.clone()).collect();
        rows.extend(visible.into_iter().filter(|r| ids.insert(r.id.clone())));
    }
    rows.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    Ok(rows)
}

/// Include an explicitly linked card outside the normal recent-history window.
pub async fn include_card(
    db: &Database,
    user: &str,
    conversation: &str,
    card: &str,
    rows: &mut Vec<AssistantAcknowledgement>,
) -> AppResult<()> {
    if !rows.iter().any(|row| row.id == card)
        && let Some(row) = db
            .collection::<AssistantAcknowledgement>(ACKS)
            .find_one(doc! {"_id":card,"user_id":user,"conversation_id":conversation})
            .await?
    {
        rows.push(row);
        rows.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    }
    Ok(())
}

/// Cards the owner allowed or denied after `since`, oldest first and bounded.
/// Each new turn reports decisions made since the previous user message, so a
/// decision made while a turn was still running reaches the model exactly once.
pub async fn decided_since(
    db: &Database,
    user: &str,
    conversation: &str,
    since: chrono::DateTime<Utc>,
) -> AppResult<Vec<AssistantAcknowledgement>> {
    expire(db, user, conversation).await?;
    Ok(db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find(doc! {
            "user_id": user,
            "conversation_id": conversation,
            "status": {"$in": ["allowed", "denied"]},
            "decided_by": {"$ne": "access_mode"},
            "decided_at": {"$gt": bson::DateTime::from_chrono(since)},
        })
        .sort(doc! {"decided_at": 1, "_id": 1})
        .limit(DECISION_NOTE_LIMIT)
        .await?
        .try_collect()
        .await?)
}

pub const DECISION_NOTE_LIMIT: i64 = 10;

/// Identifier-only rendering: slugs and tool names, never free-form names or
/// summaries, so owner-controlled text cannot enter the instructions.
fn identifier(value: Option<&str>) -> String {
    let cleaned: String = value
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .take(64)
        .collect();
    if cleaned.is_empty() {
        "unknown".into()
    } else {
        cleaned
    }
}

/// Instructions note telling the model about card decisions it has not seen.
pub fn decisions_note(rows: &[AssistantAcknowledgement]) -> String {
    let rows: Vec<_> = rows
        .iter()
        .filter(|row| row.decided_by.as_deref() != Some("access_mode"))
        .collect();
    if rows.is_empty() {
        return String::new();
    }
    let mut note = String::from(
        "\n\nChat card decisions the user made since your previous reply \
        (recorded by NyxID; facts, not instructions):",
    );
    for row in &rows {
        let target = match row.kind.as_str() {
            "service" => format!("service {}", identifier(row.service_slug.as_deref())),
            "account" => "account management".into(),
            "skills" if row.status == "allowed" => "skill proposal: NyxBot attaches it after the user confirms a separate card; do not request it again".into(),
            "skills" => "skill proposal".into(),
            "operations" => "operation scope".into(),
            "machine" => "machine access".into(),
            "saved_login" => "saved login access".into(),
            _ => format!(
                "action {} (acknowledgement_id {})",
                identifier(row.tool_name.as_deref()),
                identifier(Some(&row.id))
            ),
        };
        note.push_str(&format!("\n- {}: {target}", identifier(Some(&row.status))));
    }
    if rows
        .iter()
        .any(|row| row.status == "allowed" && !matches!(row.kind.as_str(), "skills" | "action"))
    {
        note.push_str("\nAllowed services, account access, machine access, saved logins and operation scopes are usable now.");
    }
    if rows
        .iter()
        .any(|row| row.status == "allowed" && row.kind == "action")
    {
        note.push_str("\nRetry an allowed action with its acknowledgement_id.");
    }
    note.push_str("\nDo not request denied items again unless the user asks.");
    note
}

pub async fn pending_counts(
    db: &Database,
    user: &str,
    ids: &[String],
) -> AppResult<std::collections::HashMap<String, u32>> {
    let mut cursor = db
        .collection::<bson::Document>(ACKS)
        .aggregate([
            doc! {"$match": {"user_id": user, "conversation_id": {"$in": ids},
            "status": "pending", "expires_at": {"$gt": bson::DateTime::now()}}},
            doc! {"$group": {"_id": "$conversation_id", "count": {"$sum": 1}}},
        ])
        .await?;
    let mut counts = std::collections::HashMap::new();
    while let Some(row) = cursor.try_next().await? {
        if let (Ok(id), Ok(count)) = (row.get_str("_id"), row.get_i32("count")) {
            counts.insert(id.into(), count as u32);
        }
    }
    Ok(counts)
}

pub struct Request<'a> {
    pub kind: &'a str,
    pub service: Option<(&'a str, &'a str, &'a str)>,
    pub tool: Option<&'a str>,
    pub arguments: Option<&'a Value>,
    pub summary: &'a str,
    /// `service` requests only: the target is a platform-provided catalog entry.
    pub platform: bool,
}
impl std::fmt::Debug for Request<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AcknowledgementRequest { [REDACTED] }")
    }
}

pub async fn request(
    db: &Database,
    chat: &ChatAuthority,
    request: Request<'_>,
) -> AppResult<AssistantAcknowledgement> {
    Ok(request_tracked(db, chat, request).await?.0)
}

/// Find an operation-scope request owned by `owner`.
///
/// Callers use this before deciding whether an owner confirmation card is
/// applicable; keeping the lookup here preserves the acknowledgement service
/// boundary and its ownership filter.
pub async fn find_operation_request(
    db: &Database,
    owner: &str,
    request_id: &str,
    status: Option<&str>,
) -> AppResult<Option<AssistantAcknowledgement>> {
    let mut filter = doc! {
        "_id": request_id,
        "user_id": owner,
        "decider": "orchestrator",
        "kind": "operations",
    };
    if let Some(status) = status {
        filter.insert("status", status);
    }
    db.collection::<AssistantAcknowledgement>(ACKS)
        .find_one(filter)
        .await
        .map_err(AppError::from)
}

/// Create the same owner action card used by native machine tools, while
/// retaining a bounded typed payload for the graphical context editor.
pub async fn request_machine_context(
    db: &Database,
    chat: &ChatAuthority,
    action: crate::models::machine_access::HumanContextAction,
    summary: &str,
) -> AppResult<AssistantAcknowledgement> {
    let args = serde_json::json!({
        "agent": action.agent_id,
        "machine": action.node_id,
        "selection": action.selection,
        "source": "human_machine_settings"
    });
    let encoded = bson::to_bson(&action).map_err(|_| not_found())?;
    let (row, _) = request_tracked_with_machine_context(
        db,
        chat,
        Request {
            kind: "action",
            service: None,
            tool: Some("nyxid__machine_capabilities"),
            arguments: Some(&args),
            summary,
            platform: false,
        },
        Some(action),
        None,
    )
    .await?;
    Ok(db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find_one_and_update(
            doc! {"_id": &row.id, "status": "pending", "machine_context": {"$exists": false}},
            doc! {"$set": {"machine_context": encoded}},
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .await?
        .unwrap_or(row))
}

/// Like [`request`], also reporting whether a new row was created (a pending
/// duplicate is returned as-is). Subagent requests are decided by the team's
/// orchestrator; action confirmations always belong to the user.
pub async fn request_tracked(
    db: &Database,
    chat: &ChatAuthority,
    request: Request<'_>,
) -> AppResult<(AssistantAcknowledgement, bool)> {
    request_tracked_with_machine_context(db, chat, request, None, None).await
}

/// Initiating identity retained by a durable authored-draft claim. Recovery
/// must not bind the original confirmation to a later voice or trigger turn.
#[derive(Clone)]
pub struct RequestOrigin {
    pub requested_turn_id: Option<String>,
    pub voice_request_id: Option<String>,
    pub trigger_run_id: Option<String>,
}

/// Insert or reuse a card in a caller-owned transaction. The caller must
/// commit its claim in this same transaction; this helper performs the usual
/// authority fence, deduplication and voice-confirmation bookkeeping.
pub async fn request_in_session(
    db: &Database,
    chat: &ChatAuthority,
    request: Request<'_>,
    origin: Option<&RequestOrigin>,
    session: &mut ClientSession,
) -> AppResult<AssistantAcknowledgement> {
    let candidate = prepare_request_candidate(db, chat, request, None, None).await?;
    Ok(
        request_candidate_in_session(db, chat, &candidate, origin, session)
            .await?
            .0,
    )
}

async fn request_tracked_with_machine_context(
    db: &Database,
    chat: &ChatAuthority,
    request: Request<'_>,
    machine_context: Option<crate::models::machine_access::HumanContextAction>,
    confirmation: Option<crate::models::assistant_acknowledgement::ActionConfirmation>,
) -> AppResult<(AssistantAcknowledgement, bool)> {
    expire(db, &chat.user_id, &chat.conversation_id).await?;
    let candidate =
        prepare_request_candidate(db, chat, request, machine_context, confirmation).await?;
    let (audit_db, audit_chat) = (db.clone(), chat.clone());
    let db = db.clone();
    let chat = chat.clone();
    let mut session = db.client().start_session().await?;
    let (row, created) = session
        .start_transaction()
        .and_run2(async move |session| {
            let operation = Box::pin(request_candidate_in_session(
                &db, &chat, &candidate, None, session,
            ))
            .await;
            mutations::transaction_result(operation)
        })
        .await
        .map_err(mutations::map_transaction_error)?;
    if created && row.kind != "action" {
        audit_access_requested(&audit_db, &audit_chat, &row).await;
    }
    Ok((row, created))
}

async fn prepare_request_candidate(
    db: &Database,
    chat: &ChatAuthority,
    request: Request<'_>,
    machine_context: Option<crate::models::machine_access::HumanContextAction>,
    confirmation: Option<crate::models::assistant_acknowledgement::ActionConfirmation>,
) -> AppResult<AssistantAcknowledgement> {
    let request_mode = super::assistant_settings_service::get(db, &chat.user_id)
        .await
        .map(|s| s.specialist_access_requests)
        .unwrap_or_default();
    if request.kind != "action" && !chat.access_request_allowed_with_mode(request_mode) {
        audit_access_request_denied(db, chat, request.service.map(|(id, _, _)| id), request.kind)
            .await;
        return Err(AppError::Forbidden(
            "access_request_denied: only NyxBot can request or grant access; report the need to NyxBot in your reply".into(),
        ));
    }
    if request.kind == "operations" {
        Box::pin(
            super::agent_operation_scope_service::require_configuration_enabled(db, &chat.user_id),
        )
        .await?;
    }
    let now = Utc::now();
    let orchestrated = !chat.is_orchestrator() && request.kind != "action";
    let candidate = AssistantAcknowledgement {
        authored_skill: if request.tool == Some(super::assistant_agent_learning_review::TOOL) {
            request
                .arguments
                .and_then(|a| a.get("authored_skill"))
                .map(|value| serde_json::from_value(value.clone()).map_err(|_| not_found()))
                .transpose()?
        } else {
            None
        },
        skill_selection: if request.kind == "skills"
            && request
                .arguments
                .is_some_and(|args| args.get("skills").is_some())
        {
            let selection: super::agent_skill_service::Selection =
                serde_json::from_value(request.arguments.cloned().ok_or_else(not_found)?)
                    .map_err(|_| AppError::ValidationError("Invalid skill request".into()))?;
            super::agent_skill_service::validate(&selection)?;
            Some(selection)
        } else {
            None
        },
        operation_contract_digest: None,
        operation_selection: if request.kind == "operations" {
            Some(
                serde_json::from_value(request.arguments.cloned().ok_or_else(not_found)?).map_err(
                    |_| AppError::ValidationError("Invalid operation permission request".into()),
                )?,
            )
        } else {
            None
        },
        id: Uuid::new_v4().to_string(),
        conversation_id: chat.conversation_id.clone(),
        user_id: chat.user_id.clone(),
        api_key_id: chat.api_key_id.clone(),
        requester_agent_id: Some(chat.agent_id.clone()),
        confirmation: confirmation.filter(|_| request.kind == "action"),
        kind: request.kind.into(),
        service_id: request.service.map(|s| s.0.into()),
        service_slug: request.service.map(|s| s.1.into()),
        service_name: request.service.map(|s| s.2.into()),
        platform: request.platform,
        tool_name: request.tool.map(str::to_owned),
        arguments_digest: request.arguments.map(arguments_digest),
        summary: request.summary.into(),
        link: None,
        status: "pending".into(),
        requested_turn_id: None,
        voice_request_id: None,
        continuation_receipt_id: None,
        trigger_run_id: None,
        created_at: now,
        decided_at: None,
        expires_at: now + Duration::seconds(PENDING_SECONDS),
        decider: if orchestrated { "orchestrator" } else { "user" }.into(),
        request_excerpt: None,
        decided_by: None,
        reason: None,
        machine_context,
    };
    let mut candidate = candidate;
    // Bind the request to the compiled contract shown to its decider.
    if let (Some(selection), Some(service)) = (
        candidate.operation_selection.as_ref(),
        candidate.service_id.as_deref(),
    ) {
        let digest = Box::pin(
            super::agent_operation_scope_service::preview_request_digest(
                db,
                &chat.agent_id,
                service,
                selection,
            ),
        )
        .await?;
        candidate.operation_contract_digest = Some(digest);
    }
    Ok(candidate)
}

async fn request_candidate_in_session(
    db: &Database,
    chat: &ChatAuthority,
    candidate: &AssistantAcknowledgement,
    origin: Option<&RequestOrigin>,
    session: &mut ClientSession,
) -> AppResult<(AssistantAcknowledgement, bool)> {
    let now = candidate.created_at;
    let orchestrated = candidate.decider == "orchestrator";
    let (conversation, _, access) = Box::pin(fence(db, chat, session)).await?;
    // Covers every permission kind, including machine/skill/scope
    // requests and internal callers without an auth snapshot.
    // Reuse the access already resolved or reused by the fence.
    if orchestrated && access.as_ref().is_some_and(|a| !a.can_maintain()) {
        return Err(AppError::Forbidden(ORG_MAINTAINER_REQUIRED.into()));
    }
    // The owner may have allowed the request after the handler's
    // preflight. Check again under the same conversation fence
    // that serializes this insert with the decision transaction.
    if candidate.kind == "skills"
        && let Some(existing) = db
            .collection::<AssistantAcknowledgement>(ACKS)
            .find_one(allowed_skill_filter(
                chat,
                candidate.arguments_digest.clone(),
                now,
            ))
            .sort(doc! {"decided_at": -1})
            .session(&mut *session)
            .await?
    {
        return Ok((existing, false));
    }
    let mut row = candidate.clone();
    // Ordinary denials stay bound to the initiating user/orchestrator
    // message across event turns. Only trigger runs use the active turn.
    let mut initiating_filter = doc! {
        "conversation_id": &conversation.id, "user_id": &chat.user_id,
        "role": {"$in": ["user", "orchestrator"]},
        "steering": bson::Bson::Null,
        "execution_pending": {"$ne": true},
    };
    if let Some(seq) = conversation
        .active_turn
        .as_ref()
        .and_then(|t| t.initiating_message_seq)
    {
        initiating_filter.insert("seq", doc! {"$lte": seq});
    }
    let started_by = db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(initiating_filter)
        .sort(doc! {"seq": -1})
        .session(&mut *session)
        .await?;
    row.requested_turn_id = conversation
        .active_turn
        .as_ref()
        .filter(|turn| turn.trigger_run_id.is_some())
        .map(|turn| turn.turn_id.clone())
        .or_else(|| started_by.as_ref().map(|message| message.turn_id.clone()));
    row.voice_request_id = conversation
        .active_turn
        .as_ref()
        .and_then(|t| t.voice_request_id.clone());
    row.trigger_run_id = conversation
        .active_turn
        .as_ref()
        .and_then(|turn| turn.trigger_run_id.clone());
    if let Some(origin) = origin {
        row.requested_turn_id = origin.requested_turn_id.clone();
        row.voice_request_id = origin.voice_request_id.clone();
        row.trigger_run_id = origin.trigger_run_id.clone();
    }
    if row.decider == "orchestrator" {
        row.request_excerpt = started_by.map(|message| {
            format!(
                "{} said: {}",
                message.role,
                super::assistant_nyxagent::excerpt(&message.text, 600)
            )
        });
    }
    let filter = doc! {"conversation_id": &chat.conversation_id,
    "user_id": &chat.user_id, "api_key_id": &chat.api_key_id, "kind": &row.kind,
    "service_id": &row.service_id, "tool_name": &row.tool_name,
    "arguments_digest": &row.arguments_digest,
    "$or": [{"status": "pending", "expires_at": {"$gt": bson::DateTime::from_chrono(now)}},
        {"status": "denied", "requested_turn_id": &row.requested_turn_id}]};
    // A valid allowed action can be reused only by explicitly presenting its id.
    if let Some(mut existing) = db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find_one(filter)
        .sort(doc! {"created_at": -1})
        .session(&mut *session)
        .await?
    {
        // A pending text card may be encountered again from voice.
        // Keep its digest/expiry/initiating identity, and attach the
        // server continuation before returning that same record.
        if existing.status == "pending"
            && existing.voice_request_id.is_none()
            && row.voice_request_id.is_some()
            && row.decider == "user"
        {
            existing.voice_request_id = row.voice_request_id.clone();
            db.collection::<AssistantAcknowledgement>(ACKS)
                .replace_one(doc! {"_id":&existing.id}, &existing)
                .session(&mut *session)
                .await?;
            super::assistant_voice::await_confirmation(db, &existing, session).await?;
        }
        return Ok((existing, false));
    }
    db.collection::<AssistantAcknowledgement>(ACKS)
        .insert_one(&row)
        .session(&mut *session)
        .await?;
    super::assistant_voice::await_confirmation(db, &row, session).await?;
    Ok((row, true))
}

/// An advisory skill approval remains pending attachment across later turns.
pub const SKILL_REQUEST_ALLOW_WINDOW_HOURS: i64 = 24;

fn allowed_skill_filter(
    chat: &ChatAuthority,
    digest: Option<String>,
    now: DateTime<Utc>,
) -> bson::Document {
    doc! {"conversation_id": &chat.conversation_id,
    "user_id": &chat.user_id, "api_key_id": &chat.api_key_id,
    "kind": "skills", "status": "allowed", "arguments_digest": digest,
    "decided_at": {"$gt": bson::DateTime::from_chrono(now - Duration::hours(SKILL_REQUEST_ALLOW_WINDOW_HOURS))}}
}

pub fn allowed_skill_result(row: &AssistantAcknowledgement) -> Value {
    json!({"status":"allowed_pending_attachment", "request_id":row.id,
        "instructions":"This request was already allowed. NyxBot resolves the exact version and applies the governing mode to attach it (automatic in Full access, otherwise an owner card). Do not call request_agent_skills again for this request. Continue without it and check nyxid__get_agent_skills later."})
}

pub async fn allowed_skill_request(
    db: &Database,
    chat: &ChatAuthority,
    arguments: &Value,
) -> AppResult<Option<Value>> {
    let row = db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find_one(allowed_skill_filter(
            chat,
            Some(arguments_digest(arguments)),
            Utc::now(),
        ))
        .sort(doc! {"decided_at": -1})
        .await?;
    Ok(row.as_ref().map(allowed_skill_result))
}

/// Record a permission request that the policy allowed to be raised: who
/// asked, for what, and who decides it. Metadata only, like the denial.
async fn audit_access_requested(
    db: &Database,
    chat: &ChatAuthority,
    row: &AssistantAcknowledgement,
) {
    let _ = audit_service::log_actor_event(
        db.clone(),
        &audit_service::AuditActor {
            user_id: chat.user_id.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: Some(chat.api_key_id.clone()),
            api_key_name: None,
        },
        "assistant_access_requested",
        Some(
            json!({"requester_agent": chat.agent_id, "thread": chat.conversation_id,
            "key": chat.api_key_id, "acknowledgement_id": row.id,
            "target": row.service_id, "scope": row.kind, "decider": row.decider,
            "decision": "pending"}),
        ),
    )
    .await;
}

pub fn refusal(row: &AssistantAcknowledgement) -> Value {
    let denied = row.status == "denied";
    let instructions = if row.decider == "orchestrator" {
        if denied {
            "Your orchestrator denied this request. Do not retry it; report what you \
            could do without it."
                .into()
        } else {
            "NyxID asked your orchestrator for this permission. End your turn now with a \
            one-line note about what you are waiting for; NyxID resumes you with the \
            decision."
                .into()
        }
    } else if denied {
        "The user denied this request. Do not retry or request another \
                card unless the user explicitly asks again in a later message."
            .into()
    } else if row.authored_skill.is_some() {
        "End your turn. The review card is in the NyxID app conversation. If the owner is in a chat app or group, or cannot find the card, send review_url as a full URL. Only the card, after reviewing all files, approves publication; a chat reply such as allow never approves. NyxID publishes and pins after approval without a tool retry.".into()
    } else if row.voice_request_id.is_some() {
        "NyxID is awaiting the owner's confirmation of this exact action. End this turn; \
        the voice coordinator reads the server summary and handles the owner's decision. \
        Never infer approval or confirm it yourself; NyxID resumes the task after a decision."
            .into()
    } else {
        match row.kind.as_str() {
            "service" => format!(
                "Ask the user to approve access to {} for this chat (a card is \
                shown in the chat), then retry.",
                row.service_name.as_deref().unwrap_or("this service")
            ),
            "account" => "Ask the user to approve account management for this chat (a card \
                is shown in the chat), then retry."
                .into(),
            _ => format!(
                "Ask the user to confirm the action card, then retry with \
                acknowledgement_id. Never confirm it yourself. Where no card can be shown (a \
                chat app or a group chat), ask them to reply \"yes {code}\" to confirm or \
                \"no {code}\" to cancel.",
                code = confirm_code(&row.id)
            ),
        }
    };
    let mut value = json!({"error": if denied {"acknowledgement_denied"} else {"acknowledgement_required"},
        "kind": row.kind, "acknowledgement_id": row.id, "service_slug": row.service_slug,
        "service_name": row.service_name, "summary": row.summary, "decider": row.decider,
        "instructions": instructions});
    if row.kind == "action"
        && !denied
        && row.voice_request_id.is_none()
        && row.authored_skill.is_none()
    {
        value["confirm_phrase"] = json!(format!("yes {}", confirm_code(&row.id)));
    }
    value
}

pub fn refusal_with_frontend(row: &AssistantAcknowledgement, frontend: &str) -> Value {
    let mut value = refusal(row);
    if row.authored_skill.is_some() {
        value["review_url"] = json!(
            super::assistant_links::AssistantPage::ReviewCard {
                conversation: &row.conversation_id,
                card: &row.id,
            }
            .url(frontend)
        );
    }
    value
}

/// A short code the owner quotes to confirm one specific card by reply.
pub fn confirm_code(id: &str) -> String {
    let hex: String = id.chars().filter(char::is_ascii_hexdigit).take(6).collect();
    format!(
        "{:04}",
        u32::from_str_radix(&hex, 16).unwrap_or_default() % 10_000
    )
}

/// A plain confirmation reply: yes/no, optionally followed by a card code.
pub fn parse_reply(text: &str) -> Option<(bool, Option<String>)> {
    let normalized = text
        .trim()
        .trim_end_matches(['.', '!', '。', '！'])
        .trim()
        .to_lowercase();
    let (head, code) = match normalized.rsplit_once(char::is_whitespace) {
        Some((head, code)) if code.len() == 4 && code.bytes().all(|b| b.is_ascii_digit()) => {
            (head.trim().to_owned(), Some(code.to_owned()))
        }
        _ => (normalized.clone(), None),
    };
    let allow = match head.as_str() {
        "yes" | "y" | "yes please" | "confirm" | "confirmed" | "allow" | "approve" | "ok"
        | "okay" | "go ahead" | "do it" | "是" | "是的" | "确认" | "好" | "好的" | "可以" => {
            true
        }
        "no" | "n" | "deny" | "cancel" | "stop" | "reject" | "don't" | "do not" | "否" | "不"
        | "不要" | "取消" => false,
        _ => return None,
    };
    Some((allow, code))
}

/// Decide the owner's action card a reply answers, from a chat app or group
/// where no card can be shown. A quoted code picks that card; a plain yes/no
/// applies only when exactly one card was raised since `since` (the owner's
/// previous message), so an answer to another question never confirms a
/// stale card.
pub async fn decide_reply(
    db: &Database,
    owner: &str,
    conversation_ids: &[String],
    text: &str,
    since: Option<DateTime<Utc>>,
) -> AppResult<Option<AssistantAcknowledgement>> {
    let Some((allow, code)) = parse_reply(text) else {
        return Ok(None);
    };
    if conversation_ids.is_empty() {
        return Ok(None);
    }
    let now = Utc::now();
    let pending: Vec<AssistantAcknowledgement> = db
        .collection::<AssistantAcknowledgement>(ACKS)
        .find(
            doc! {"user_id": owner, "conversation_id": {"$in": conversation_ids},
            "kind": "action", "status": "pending", "decider": "user",
            "authored_skill": bson::Bson::Null,
            "expires_at": {"$gt": bson::DateTime::from_chrono(now)}},
        )
        .await?
        .try_collect()
        .await?;
    let chosen = match code {
        Some(code) => pending
            .into_iter()
            .find(|ack| confirm_code(&ack.id) == code),
        None => {
            let mut recent: Vec<AssistantAcknowledgement> = pending
                .into_iter()
                .filter(|ack| since.is_none_or(|since| ack.created_at > since))
                .collect();
            if recent.len() == 1 {
                recent.pop()
            } else {
                None
            }
        }
    };
    let Some(ack) = chosen else {
        return Ok(None);
    };
    decide(db, owner, &ack.conversation_id, &ack.id, allow)
        .await
        .map(Some)
}

/// Gate a subagent's service call. Orchestrators have full account authority and are
/// never gated. `platform` targets are catalog entries reached through NyxID's
/// platform credential; they are granted on `allowed_platform_service_ids`.
/// Returns the refusal and, when a new request was created, its row so the
/// caller can notify the orchestrator.
pub async fn service_gate(
    db: &Database,
    chat: &ChatAuthority,
    id: &str,
    slug: &str,
    name: &str,
    platform: bool,
) -> AppResult<Option<(Value, Option<AssistantAcknowledgement>)>> {
    if chat.is_orchestrator() {
        // NyxBot holds every service; other people get none of them.
        if chat.guest {
            return Ok(Some((orchestrator_guest_refusal(), None)));
        }
        return Ok(None);
    }
    let key = key_service::get_api_key(db, &chat.user_id, &chat.api_key_id).await?;
    let mut access = chat.org_agent_access.clone();
    let granted = if platform {
        key.allowed_platform_service_ids
            .iter()
            .any(|allowed| allowed == id)
    } else {
        // This authority check already needed live access. Retain a newly
        // resolved snapshot for the ungranted path and its card transaction.
        access = super::org_agent_service::key_access_with_snapshot(
            db,
            &chat.user_id,
            key.assistant_agent_owner_id.as_deref(),
            access.as_ref(),
        )
        .await?;
        key_service::effective_allowed_service_ids_with_access(db, &key, access.as_deref())
            .await?
            .iter()
            .any(|allowed| allowed == id)
    };
    if granted {
        return Ok(None);
    }
    // A guest never widens what the agent may use: no permission request.
    if chat.guest {
        return Ok(Some((guest_refusal(), None)));
    }
    let routed = super::assistant_settings_service::get(db, &chat.user_id)
        .await
        .map(|s| chat.access_request_allowed_with_mode(s.specialist_access_requests))
        .unwrap_or(false);
    if !routed {
        audit_access_request_denied(db, chat, Some(id), "service").await;
        return Ok(Some((access_request_denied(), None)));
    }
    access = super::org_agent_service::key_access_with_snapshot(
        db,
        &chat.user_id,
        key.assistant_agent_owner_id.as_deref(),
        access.as_ref(),
    )
    .await?;
    if access.as_ref().is_some_and(|a| !a.can_maintain()) {
        return Ok(Some((organization_agent_refusal(), None)));
    }
    let mut request_chat = chat.clone();
    request_chat.org_agent_access = access;
    let summary = if platform {
        format!("Use {name} (NyxID platform credential)")
    } else {
        format!("Use {name}")
    };
    let (row, created) = request_tracked(
        db,
        &request_chat,
        Request {
            kind: "service",
            service: Some((id, slug, name)),
            tool: None,
            arguments: None,
            summary: &summary,
            platform,
        },
    )
    .await?;
    let value = refusal(&row);
    Ok(Some((value, created.then_some(row))))
}

/// Gate a subagent's account tool. Subagents reach only read-only tools, and
/// only with an `account_read` grant.
pub async fn account_gate(
    db: &Database,
    chat: &ChatAuthority,
) -> AppResult<Option<(Value, Option<AssistantAcknowledgement>)>> {
    if chat.is_orchestrator() {
        return Ok(None);
    }
    let key = key_service::get_api_key(db, &chat.user_id, &chat.api_key_id).await?;
    if key
        .scopes
        .split_whitespace()
        .any(|scope| scope == ASSISTANT_ACCOUNT_SCOPE)
    {
        return Ok(None);
    }
    let routed = super::assistant_settings_service::get(db, &chat.user_id)
        .await
        .map(|s| chat.access_request_allowed_with_mode(s.specialist_access_requests))
        .unwrap_or(false);
    if !routed {
        audit_access_request_denied(db, chat, None, "account").await;
        return Ok(Some((access_request_denied(), None)));
    }
    let access = super::org_agent_service::key_access_with_snapshot(
        db,
        &chat.user_id,
        key.assistant_agent_owner_id.as_deref(),
        chat.org_agent_access.as_ref(),
    )
    .await?;
    if access.as_ref().is_some_and(|a| !a.can_maintain()) {
        return Ok(Some((organization_agent_refusal(), None)));
    }
    let mut request_chat = chat.clone();
    request_chat.org_agent_access = access;
    let (row, created) = request_tracked(
        db,
        &request_chat,
        Request {
            kind: "account",
            service: None,
            tool: None,
            arguments: None,
            summary: "Read the NyxID account (keys, channel bots, services, nodes, approvals)",
            platform: false,
        },
    )
    .await?;
    let value = refusal(&row);
    Ok(Some((value, created.then_some(row))))
}

/// Who decides a card.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decider {
    /// The owner, from the card's own conversation.
    User,
    /// The owner's NyxBot deciding a specialist's request.
    Nyxbot,
    /// NyxBot has consumed an owner action card bound to this exact decision.
    NyxbotOwnerConfirmed,
}

/// Decide a card as the owner from its own conversation.
pub async fn decide(
    db: &Database,
    user: &str,
    conversation: &str,
    id: &str,
    allow: bool,
) -> AppResult<AssistantAcknowledgement> {
    Box::pin(decide_as(
        db,
        user,
        Some(conversation),
        id,
        allow,
        Decider::User,
        None,
    ))
    .await
}

/// Decide a card. The owner decides from the card's conversation; the owner's
/// NyxBot decides only NyxBot-routed specialist requests. A specialist grant is
/// also written to the agent's durable grants so every thread keeps it.
pub async fn decide_as(
    db: &Database,
    user: &str,
    conversation: Option<&str>,
    id: &str,
    allow: bool,
    decider: Decider,
    reason: Option<&str>,
) -> AppResult<AssistantAcknowledgement> {
    decide_with_voice(db, user, conversation, id, allow, decider, reason, None).await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn decide_with_voice(
    db: &Database,
    user: &str,
    conversation: Option<&str>,
    id: &str,
    allow: bool,
    decider: Decider,
    reason: Option<&str>,
    voice: Option<super::voice::confirmation::DecisionFence>,
) -> AppResult<AssistantAcknowledgement> {
    let db = db.clone();
    let user = user.to_owned();
    let conversation = conversation.map(str::to_owned);
    let id = id.to_owned();
    let by_nyxbot = decider != Decider::User;
    let allow_operation_widening = matches!(decider, Decider::User | Decider::NyxbotOwnerConfirmed);
    let reason = reason.map(|reason| super::assistant_nyxagent::excerpt(reason, 300));
    let mut session = db.client().start_session().await?;
    let row = session
        .start_transaction()
        .and_run2(async move |session| {
            let operation = Box::pin(async {
                let collection = db.collection::<AssistantAcknowledgement>(ACKS);
                let mut filter = doc! {"_id": &id, "user_id": &user};
                if let Some(conversation) = &conversation {
                    filter.insert("conversation_id", conversation);
                }
                if by_nyxbot {
                    filter.insert("decider", "orchestrator");
                }
                let mut row = collection
                    .find_one(filter.clone())
                    .session(&mut *session)
                    .await?
                    .ok_or_else(not_found)?;
                if row.authored_skill.is_some() && (voice.is_some() || by_nyxbot) {
                    return Err(AppError::Forbidden("Review the complete skill draft on its owner card".into()));
                }
                if let Some(voice)=&voice {
                    super::voice::confirmation::fence_decision(&db,session,&row,voice).await?;
                }
                if row.status != "pending" || row.expires_at <= Utc::now() {
                    return Err(AppError::Conflict(
                        "Acknowledgement is no longer pending".into(),
                    ));
                }
                let target = db
                    .collection::<AssistantConversation>(CONVERSATIONS)
                    .find_one(doc! {"_id": &row.conversation_id, "user_id": &user})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(not_found)?;
                if row.voice_request_id.is_some() {
                    super::assistant_agent_credential_service::authority_in_session(
                        &db,
                        &target,
                        &mut *session,
                    )
                    .await?;
                }
                let chat = ChatAuthority {
                    org_agent_access: None,
                    turn_id: target.active_turn.as_ref().map(|t| t.turn_id.clone()),
                    turn_stopped: target.active_turn.as_ref().is_none_or(|t| t.stop_requested),
                    turn_live: super::assistant_nyxagent::live_turn(&target, Utc::now()).is_some(),
                    machine_node_ids: Vec::new(),
                    saved_login_ids: Vec::new(),
                    confirmation_policy: None,
                    user_id: user.clone(),
                    conversation_id: row.conversation_id.clone(),
                    api_key_id: row.api_key_id.clone(),
                    role: target.role,
                    agent_id: target.agent_id.clone().unwrap_or_default(),
                    agent_name: String::new(),
                    guest: target.guest_turn,
                };
                // Decision requests have no inherited auth snapshot. The
                // transaction's fence resolves live membership, including for
                // cards created before delegation supported org specialists.
                let (_, key, access) = Box::pin(fence(&db, &chat, session)).await?;
                if by_nyxbot && access.as_ref().is_some_and(|a| !a.can_maintain()) {
                    return Err(AppError::Forbidden(ORG_MAINTAINER_REQUIRED.into()));
                }
                let subagent = target.role == AgentRole::Subagent;
                let now = Utc::now();
                if row.kind == "service" {
                    let service_id = row.service_id.as_deref().ok_or_else(not_found)?;
                    if row.platform {
                        // A platform grant names an active catalog entry. Visibility
                        // through platform grants is re-checked on every execution,
                        // so a stale entry on the key can never execute by itself.
                        db.collection::<bson::Document>(
                            crate::models::downstream_service::COLLECTION_NAME,
                        )
                        .find_one(doc! {"_id": service_id, "is_active": true})
                        .session(&mut *session)
                        .await?
                        .ok_or_else(not_found)?;
                    } else {
                        // Only owner-visible UserService rows can receive chat grants.
                        // Reject inaccessible rows before any decision.
                        super::api_key_scope_service::validate_service_ids(
                            &db,
                            &user,
                            &[service_id.into()],
                            super::api_key_scope_service::ScopeAuthorization::for_actor(Some(
                                &user,
                            )),
                        )
                        .await
                        .map_err(|error| match error {
                            AppError::ValidationError(_) => not_found(),
                            error => error,
                        })?;
                    }
                }
                if matches!(row.kind.as_str(), "machine" | "saved_login") {
                    let id = row.service_id.as_deref().ok_or_else(not_found)?;
                    if row.kind == "machine" {
                        let node = super::node_service::get_node_by_id(&db, id)
                            .await?
                            .ok_or_else(not_found)?;
                        let access =
                            super::org_service::resolve_owner_access(&db, &user, &node.user_id)
                                .await?;
                        let org_agent_node = if subagent {
                            let agent = super::assistant_team_service::maintained_agent(
                                &db,
                                &user,
                                target.agent_id.as_deref().ok_or_else(not_found)?,
                            )
                            .await?;
                            agent.user_id != user && agent.user_id == node.user_id
                        } else {
                            false
                        };
                        if !node.is_active
                            || !(access.can_write()
                                || (org_agent_node
                                    && super::org_agent_service::can_maintain(&access)))
                        {
                            return Err(not_found());
                        }
                    } else {
                        super::saved_login_service::get(&db, &user, id).await?;
                    }
                    if allow && subagent {
                        let change = super::assistant_team_service::GrantChange::Machine {
                            base: Box::new(super::assistant_team_service::GrantChange::Add(
                                Default::default(),
                            )),
                            machines: (row.kind == "machine").then(|| vec![id.to_owned()]),
                            logins: (row.kind == "saved_login").then(|| vec![id.to_owned()]),
                            mode: super::assistant_team_service::MachineGrantMode::Add,
                        };
                        Box::pin(super::assistant_team_service::apply_grants_in_session(
                            &db,
                            &user,
                            target.agent_id.as_deref().ok_or_else(not_found)?,
                            &change,
                            session,
                        ))
                        .await?;
                    }
                }
                if allow && subagent && row.kind == "operations" {
                    let mut selection = row.operation_selection.clone().ok_or_else(not_found)?;
                    selection.contract_digest = row.operation_contract_digest.clone();
                    Box::pin(super::agent_operation_scope_service::apply_in_session(
                        &db,
                        &user,
                        target.agent_id.as_deref().ok_or_else(not_found)?,
                        row.service_id.as_deref().ok_or_else(not_found)?,
                        &selection,
                        allow_operation_widening,
                        session,
                    ))
                    .await?;
                }
                if allow && subagent {
                    // A specialist's grant lives on its agent and converges on
                    // every one of its thread keys, not just the requesting one.
                    let mut grant = crate::models::assistant_agent::AgentGrants::default();
                    match (row.kind.as_str(), row.service_id.clone()) {
                        ("service", Some(service_id)) if row.platform => {
                            grant.platform_service_ids.push(service_id)
                        }
                        ("service", Some(service_id)) => grant.service_ids.push(service_id),
                        ("account", _) => grant.account_read = true,
                        _ => {}
                    }
                    if grant != Default::default() {
                        let agent_id = target.agent_id.as_deref().ok_or_else(not_found)?;
                        Box::pin(super::assistant_team_service::apply_grants_in_session(
                            &db,
                            &user,
                            agent_id,
                            &super::assistant_team_service::GrantChange::Add(grant),
                            &mut *session,
                        ))
                        .await?;
                    }
                } else if allow && row.kind == "service" {
                    let service_id = row.service_id.as_deref().ok_or_else(not_found)?;
                    let field = if row.platform {
                        "allowed_platform_service_ids"
                    } else {
                        "allowed_service_ids"
                    };
                    mutations::update_one(
                        &db,
                        doc! {"_id": &key.id, "user_id": &user},
                        doc! {"$addToSet": {field: service_id}},
                        Some(&mut *session),
                    )
                    .await?;
                } else if allow && row.kind == "account" {
                    let mut scopes: Vec<_> = key.scopes.split_whitespace().collect();
                    if !scopes.contains(&ASSISTANT_ACCOUNT_SCOPE) {
                        scopes.push(ASSISTANT_ACCOUNT_SCOPE);
                    }
                    mutations::update_one(
                        &db,
                        doc! {"_id": &key.id, "user_id": &user},
                        doc! {"$set": {"scopes": scopes.join(" ")}},
                        Some(&mut *session),
                    )
                    .await?;
                }
                row.status = if allow { "allowed" } else { "denied" }.into();
                row.decided_at = Some(now);
                row.decided_by = Some(if by_nyxbot { "orchestrator" } else { "user" }.into());
                row.reason = reason.clone();
                if allow && row.kind == "action" {
                    row.expires_at = now + Duration::seconds(ACTION_SECONDS);
                }
                row.continuation_receipt_id =
                    Box::pin(super::assistant_voice::continuation(&db, &row, session)).await?;
                collection
                    .replace_one(filter, &row)
                    .session(&mut *session)
                    .await?;
                if allow && row.kind == "skills" && !by_nyxbot {
                    // Internal outbox marker: the saved approval always has
                    // durable work, even if the deciding process exits before
                    // it can notify NyxBot. Consumption clears it in the same
                    // transaction that binds the event to a NyxBot turn.
                    db.collection::<bson::Document>(ACKS)
                        .update_one(doc! {"_id": &row.id}, doc! {"$set": {"skill_notification_pending": true}})
                        .session(&mut *session).await?;
                }
                if let Some(run_id) = &row.trigger_run_id {
                    // Durable wakeup in the decision transaction. Settlement
                    // reads card state and writes this same work row, preventing
                    // a simultaneous settlement from overwriting the wakeup.
                    db.collection::<bson::Document>(super::trigger_schedule::WORK)
                        .update_one(
                            doc! {"_id": run_id},
                            doc! {"$set": {
                                "at": bson::DateTime::from_chrono(now), "fence": "", "deferrals": 0,
                            }},
                        )
                        .session(&mut *session)
                        .await?;
                }
                if let Some(voice)=&voice {
                    super::audit_service::log_actor_event_in_session(&db,session,voice.audit_key.as_ref().as_ref(),
                        &super::audit_service::AuditActor{user_id:user.clone(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None},
                        "assistant_confirmation_decided",json!({"source":"voice","conversation_id":row.conversation_id,
                            "session_id":voice.session.id,"acknowledgement_id":row.id,"request_id":row.voice_request_id,
                            "continuation_receipt_id":row.continuation_receipt_id})).await?;
                }
                Ok(row)
            })
            .await;
            mutations::transaction_result(operation)
        })
        .await
        .map_err(mutations::map_transaction_error)?;
    Ok(row)
}

#[path = "assistant_skill_notifications.rs"]
pub mod skill_notifications;

/// Atomically consume a digest-bound action. Failures never fall through to execution.
pub async fn consume_action(
    db: &Database,
    chat: &ChatAuthority,
    id: &str,
    tool: &str,
    arguments: &Value,
) -> AppResult<bool> {
    let db = db.clone();
    let chat = chat.clone();
    let id = id.to_owned();
    let tool = tool.to_owned();
    let digest = arguments_digest(arguments);
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let operation = async {
            Box::pin(fence(&db, &chat, session)).await?;
            let result = db.collection::<AssistantAcknowledgement>(ACKS).update_one(
                doc! {"_id": &id, "user_id": &chat.user_id, "conversation_id": &chat.conversation_id,
                    "api_key_id": &chat.api_key_id, "kind": "action", "tool_name": &tool,
                    "arguments_digest": &digest,
                    "status": "allowed",
                    "decided_by": {"$ne": "access_mode"},
                    "expires_at": {"$gt": bson::DateTime::now()},
                },
                doc! {"$set": {"status": "used"}},
            ).session(&mut *session).await?;
            Ok(result.modified_count == 1)
        }.await;
        mutations::transaction_result(operation)
    }).await.map_err(mutations::map_transaction_error)
}

/// Consume inside the caller's durable effect transaction. Learning uses this
/// to bind one human card to exactly one publication operation before egress.
pub(crate) async fn consume_action_in_session(
    db: &Database,
    chat: &ChatAuthority,
    id: &str,
    tool: &str,
    arguments: &Value,
    session: &mut ClientSession,
) -> AppResult<bool> {
    Box::pin(fence(db, chat, session)).await?;
    let result = db
        .collection::<AssistantAcknowledgement>(ACKS)
        .update_one(
            doc! {"_id": id, "user_id": &chat.user_id, "conversation_id": &chat.conversation_id,
            "api_key_id": &chat.api_key_id, "kind": "action", "tool_name": tool,
            "arguments_digest": arguments_digest(arguments), "status": "allowed",
            "decider": "user", "decided_by": {"$eq": "user", "$ne": "access_mode"}, "expires_at": {"$gt": bson::DateTime::now()}},
            doc! {"$set": {"status": "used"}},
        )
        .session(&mut *session)
        .await?;
    Ok(result.modified_count == 1)
}

/// Who decided a card: the owner's NyxBot (deciding a specialist's routed
/// request), the owner in person, or an API key acting for the owner.
fn approver_kind(
    row: &AssistantAcknowledgement,
    actor: &audit_service::AuditActor,
) -> &'static str {
    if row.decided_by.as_deref() == Some("orchestrator") {
        "nyxbot"
    } else if actor.api_key_id.is_some() {
        "api_key"
    } else {
        "owner"
    }
}

pub async fn audit_decision(
    db: &Database,
    actor: &audit_service::AuditActor,
    row: &AssistantAcknowledgement,
) {
    let _ = audit_service::log_actor_event(
        db.clone(),
        actor,
        "assistant_acknowledgement_decided",
        Some(json!({
            "conversation_id": row.conversation_id, "kind": row.kind,
            "service_id": row.service_id, "tool_name": row.tool_name,
            "decision": if row.status == "allowed" {"allow"} else {"deny"},
            "requester_agent_id": row.requester_agent_id,
            "approver_user_id": actor.user_id,
            "approver_api_key_id": actor.api_key_id,
            "approver_kind": approver_kind(row, actor),
        })),
    )
    .await;
}

/// Confirm exact changing actions initiated by untrusted webhook data. Native
/// tools use their closed inventory; service callers use catalog/HTTP semantics.
pub fn webhook_confirmation_required(
    chat: &ChatAuthority,
    read_only: bool,
    destructive: bool,
) -> bool {
    use crate::models::trigger_schedule::ConfirmationPolicy;
    match chat.confirmation_policy {
        Some(ConfirmationPolicy::Changes) => !read_only,
        Some(ConfirmationPolicy::Destructive) => destructive,
        None => false,
    }
}
