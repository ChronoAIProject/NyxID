//! Assistant chat pass-through: resolution and upstream path construction.
//!
//! The browser talks to NyxID's own `/api/v1/assistant/*` routes (PRD
//! decision 4) rather than reaching Aevatar through the generic proxy. Two
//! invariants live here:
//!
//! 1. **Admin-managed resolution.** Aevatar is resolved as the admin-seeded
//!    `DownstreamService` (`service_category = "internal"`,
//!    `requires_user_credential = false`), never through the caller's
//!    personal `UserService`. This matters because `lookup_user_service`
//!    matches on slug *and* `catalog_service_id`, so a user who happens to
//!    have connected Aevatar personally would otherwise silently route
//!    through their own row while everyone else routed through the platform
//!    one -- the assistant would work for exactly the people who tested it
//!    and no one else. Behavior must not depend on caller-owned rows.
//! 2. **Server-owned scope.** The Aevatar scope segment is derived from the
//!    verified `AuthUser.user_id`, never from a browser-supplied path or
//!    body param, so a caller cannot address another user's scope.

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use chrono::{DateTime, FixedOffset};
use mongodb::bson::doc;
use regex::Regex;
use std::collections::HashSet;
use std::sync::LazyLock;

/// Catalog slug of the admin-seeded Aevatar service.
pub const AEVATAR_SLUG: &str = "aevatar";

/// Resolve the admin-managed Aevatar service.
///
/// Deliberately reads `downstream_services` (the admin catalog) directly:
/// this is the "admin service" path, and it must resolve identically for
/// every caller. See the module invariants.
pub async fn resolve_admin_service(db: &mongodb::Database) -> AppResult<DownstreamService> {
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": AEVATAR_SLUG, "is_active": true })
        .await?
        // Absent/inactive is a platform provisioning fault, not a caller
        // error: `Internal` keeps the detail server-side.
        .ok_or_else(|| {
            AppError::Internal(format!(
                "assistant: no active downstream service with slug '{AEVATAR_SLUG}'"
            ))
        })?;

    // Guard the provisioning contract: a service that demands a per-user
    // credential cannot back a platform surface, and misconfiguring it that
    // way would degrade into per-user connections without any visible error.
    if service.requires_user_credential {
        return Err(AppError::Internal(format!(
            "assistant: service '{AEVATAR_SLUG}' requires a user credential and cannot back the platform assistant surface"
        )));
    }

    Ok(service)
}

/// Reject ids that could escape the upstream path segment they are
/// interpolated into. Aevatar ids are opaque `nyxid-chat-<hex>` handles;
/// anything carrying a separator is a caller trying to reshape the URL.
fn validate_conversation_id(conversation_id: &str) -> AppResult<()> {
    let valid = !conversation_id.is_empty()
        && conversation_id.len() <= 128
        && conversation_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !valid {
        return Err(AppError::BadRequest("Invalid conversation id.".to_string()));
    }
    Ok(())
}

/// `chat-history` -- legacy workflow (`chatc-…`) index used only when the
/// canonical `/api/chat/conversations` list does not yet include those rows.
pub fn history_index_path(user_id: &str) -> String {
    format!("api/scopes/{user_id}/chat-history")
}

/// Actor-id prefix of a `nyxid-chat` conversation (upstream
/// `NyxIdChatServiceDefaults.ActorIdPrefix`; ids are `nyxid-chat-{guid:N}`).
const NYXID_CHAT_ACTOR_PREFIX: &str = "nyxid-chat-";

/// Conversation-id prefix of a workflow-chat conversation (upstream
/// `ChatHistoryActorIds.CreateConversationId`; ids are `chatc-{hash[..32]}`).
const WORKFLOW_CHAT_CONVERSATION_PREFIX: &str = "chatc-";

/// Drop conversation rows this surface cannot address.
///
/// The canonical `/api/chat/conversations` list and the legacy workflow
/// `chat-history` index are both shared read models. Only the two assistant
/// families stay visible here: typed `nyxid-chat-…` rows and workflow
/// `chatc-…` rows.
///
/// Shape-tolerant by design — an index that is not `{"conversations": [...]}`
/// (or a row without a string `id`) is returned untouched, matching the
/// deploy-independence posture of the transcript route.
pub fn filter_addressable_conversation_index(index: &mut serde_json::Value) -> bool {
    let Some(rows) = index
        .get_mut("conversations")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return false;
    };
    let before = rows.len();
    rows.retain(|row| {
        row.get("id")
            .and_then(serde_json::Value::as_str)
            // Keep unknown-shaped rows: a row we cannot classify is not
            // evidence that it belongs to another surface.
            .is_none_or(|id| {
                id.starts_with(NYXID_CHAT_ACTOR_PREFIX)
                    || id.starts_with(WORKFLOW_CHAT_CONVERSATION_PREFIX)
            })
    });
    rows.len() != before
}

/// Whether the canonical index already includes workflow rows.
pub fn conversation_index_includes_workflow(index: &serde_json::Value) -> bool {
    index
        .get("conversations")
        .and_then(serde_json::Value::as_array)
        .is_some_and(|rows| {
            rows.iter().any(|row| {
                row.get("id")
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|id| id.starts_with(WORKFLOW_CHAT_CONVERSATION_PREFIX))
            })
        })
}

/// Merge workflow-only `chat-history` rows into the canonical index, newest
/// first, deduping by id and keeping the canonical row when both exist.
pub fn merge_workflow_history_rows(
    canonical: &mut serde_json::Value,
    workflow_history: &serde_json::Value,
) -> bool {
    let Some(canonical_rows) = canonical
        .get_mut("conversations")
        .and_then(serde_json::Value::as_array_mut)
    else {
        return false;
    };

    let Some(history_rows) = workflow_history
        .get("conversations")
        .and_then(serde_json::Value::as_array)
    else {
        return false;
    };

    let before = canonical_rows.len();
    let mut merged = Vec::with_capacity(canonical_rows.len() + history_rows.len());
    let mut seen = HashSet::new();

    for row in canonical_rows.iter() {
        if let Some(id) = row.get("id").and_then(serde_json::Value::as_str) {
            seen.insert(id.to_string());
        }
        merged.push(row.clone());
    }

    for row in history_rows {
        let Some(id) = row.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if !id.starts_with(WORKFLOW_CHAT_CONVERSATION_PREFIX) || !seen.insert(id.to_string()) {
            continue;
        }
        merged.push(row.clone());
    }

    merged.sort_by(|left, right| {
        let left_key = conversation_updated_at_key(left);
        let right_key = conversation_updated_at_key(right);
        match (left_key.parsed.as_ref(), right_key.parsed.as_ref()) {
            (Some(left_time), Some(right_time)) => right_time.cmp(left_time),
            _ => right_key.raw.cmp(&left_key.raw),
        }
    });
    *canonical_rows = merged;
    canonical_rows.len() != before
}

/// `api/chat/conversations` -- canonical conversation list.
pub fn canonical_conversations_path() -> String {
    "api/chat/conversations".to_string()
}

/// `api/chat/conversations/{id}` -- canonical conversation detail/delete.
pub fn canonical_conversation_path(conversation_id: &str) -> AppResult<String> {
    validate_conversation_id(conversation_id)?;
    Ok(format!("api/chat/conversations/{conversation_id}"))
}

/// `api/chat/conversations/{id}/state` -- canonical reconnect surface.
pub fn canonical_state_path(conversation_id: &str) -> AppResult<String> {
    Ok(format!("{}/state", canonical_conversation_path(conversation_id)?))
}

/// `v1/chat/completions` -- OpenAI-compatible surface. Scope-free: the
/// endpoint is stateless and carries its history in the request body.
pub fn completions_path() -> String {
    "v1/chat/completions".to_string()
}

/// `api/chat` -- ad-hoc workflow chat (`StartWorkflowChat`). Scope-free on
/// the wire: Aevatar derives the scope from the propagated identity JWT, so
/// the server-owned-scope invariant holds with nothing in the path.
pub fn workflow_chat_path() -> String {
    "api/chat".to_string()
}

/// `api/chat` -- typed NyxIdChat create-and-first-turn stream.
///
/// Aevatar selects the typed NyxIdChat producer from the discriminated body
/// when no Workflow Studio fields are present. Scope remains absent from the
/// wire body: the propagated, verified NyxID identity is authoritative.
pub fn typed_chat_path() -> String {
    "api/chat".to_string()
}

/// Matches the client cap in `aevatar-transport.ts` (`MAX_MESSAGE_CHARS`).
const TYPED_CHAT_PROMPT_MAX_CHARS: usize = 32_768;
const ACTION_CONTINUATION_MAX_REPORTS: usize = 64;
const APPROVAL_REASON_MAX_CHARS: usize = 2_048;

static FORBIDDEN_COMMAND_KEY: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:^|[_-])(authorization|api[-_]?key|token|secret|password|credential|cookie|user[-_]?code|device[-_]?code)(?:$|[_-])",
    )
    .expect("FORBIDDEN_COMMAND_KEY regex")
});
static FORBIDDEN_COMMAND_VALUE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)Bearer\s+[A-Za-z0-9._~+/-]+|nyx(?:id)?_[A-Za-z0-9_-]{8,}")
        .expect("FORBIDDEN_COMMAND_VALUE regex")
});

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AssistantChatResponseKind {
    Stream,
    Json,
}

impl AssistantChatResponseKind {
    pub fn accept_header_value(self) -> &'static str {
        match self {
            Self::Stream => "text/event-stream",
            Self::Json => "application/json",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAssistantChatCommand {
    pub body: serde_json::Value,
    pub client_request_id: String,
    pub response_kind: AssistantChatResponseKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AssistantChatCommand {
    Text(TextChatCommand),
    ActionContinue(ActionContinueCommand),
    ApprovalResolve(ApprovalResolveCommand),
    TaskStop(TaskStopCommand),
    TaskSteer(TaskSteerCommand),
    StepRetry(StepRetryCommand),
    StepSkip(StepSkipCommand),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TextChatCommand {
    pub prompt: String,
    pub client_request_id: String,
    pub conversation_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionContinueCommand {
    pub conversation_id: String,
    pub client_request_id: String,
    pub origin_turn_id: Option<String>,
    pub actions: Vec<ActionReport>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovalResolveCommand {
    pub conversation_id: String,
    pub client_request_id: String,
    pub request_id: String,
    pub approved: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStopCommand {
    pub conversation_id: String,
    pub turn_id: String,
    pub stop_request_id: String,
    pub client_request_id: String,
    pub expected_state_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSteerCommand {
    pub conversation_id: String,
    pub turn_id: String,
    pub steering_id: String,
    pub client_request_id: String,
    pub instruction: String,
    pub expected_state_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRetryCommand {
    pub conversation_id: String,
    pub turn_id: String,
    pub task_id: String,
    pub step_id: String,
    pub retry_request_id: String,
    pub client_request_id: String,
    pub expected_operation_generation: i64,
    pub expected_state_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepSkipCommand {
    pub conversation_id: String,
    pub turn_id: String,
    pub task_id: String,
    pub step_id: String,
    pub skip_request_id: String,
    pub client_request_id: String,
    pub expected_operation_generation: i64,
    pub expected_state_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActionReport {
    pub action_request_id: String,
    pub origin_turn_id: String,
    pub disposition: ActionDisposition,
    pub resource: Option<ActionResource>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ActionDisposition {
    Completed,
    Declined,
    Failed,
    Cancelled,
    Expired,
}

impl ActionDisposition {
    fn parse(value: &str) -> AppResult<Self> {
        match value {
            "completed" => Ok(Self::Completed),
            "declined" => Ok(Self::Declined),
            "failed" => Ok(Self::Failed),
            "cancelled" => Ok(Self::Cancelled),
            "expired" => Ok(Self::Expired),
            _ => Err(AppError::BadRequest(
                "Invalid action report disposition.".to_string(),
            )),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Declined => "declined",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Expired => "expired",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionResource {
    UserService { user_service_id: String },
    Key { key_id: String },
    Node { node_id: String },
    ServiceAccount { service_account_id: String },
    DeveloperApp { client_id: String },
    Device { device_id: String },
}

impl ActionResource {
    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::UserService { user_service_id } => serde_json::json!({
                "userService": { "userServiceId": user_service_id }
            }),
            Self::Key { key_id } => serde_json::json!({
                "key": { "keyId": key_id }
            }),
            Self::Node { node_id } => serde_json::json!({
                "node": { "nodeId": node_id }
            }),
            Self::ServiceAccount { service_account_id } => serde_json::json!({
                "serviceAccount": { "serviceAccountId": service_account_id }
            }),
            Self::DeveloperApp { client_id } => serde_json::json!({
                "developerApp": { "clientId": client_id }
            }),
            Self::Device { device_id } => serde_json::json!({
                "device": { "deviceId": device_id }
            }),
        }
    }

    fn is_user_service(&self) -> bool {
        matches!(self, Self::UserService { .. })
    }
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawTextChatCommand {
    #[serde(rename = "type")]
    _command_type: String,
    prompt: String,
    client_request_id: String,
    #[serde(default)]
    conversation_id: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawActionContinueCommand {
    #[serde(rename = "type")]
    _command_type: String,
    conversation_id: String,
    client_request_id: String,
    #[serde(default)]
    origin_turn_id: Option<String>,
    actions: Vec<RawActionReport>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawActionReport {
    action_request_id: String,
    origin_turn_id: String,
    disposition: String,
    #[serde(default)]
    resource: Option<serde_json::Value>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawApprovalResolveCommand {
    #[serde(rename = "type")]
    _command_type: String,
    conversation_id: String,
    client_request_id: String,
    request_id: String,
    approved: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawTaskStopCommand {
    #[serde(rename = "type")]
    _command_type: String,
    conversation_id: String,
    turn_id: String,
    stop_request_id: String,
    client_request_id: String,
    expected_state_version: i64,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawTaskSteerCommand {
    #[serde(rename = "type")]
    _command_type: String,
    conversation_id: String,
    turn_id: String,
    steering_id: String,
    client_request_id: String,
    instruction: String,
    expected_state_version: i64,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawStepRetryCommand {
    #[serde(rename = "type")]
    _command_type: String,
    conversation_id: String,
    turn_id: String,
    task_id: String,
    step_id: String,
    retry_request_id: String,
    client_request_id: String,
    expected_operation_generation: i64,
    expected_state_version: i64,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawStepSkipCommand {
    #[serde(rename = "type")]
    _command_type: String,
    conversation_id: String,
    turn_id: String,
    task_id: String,
    step_id: String,
    skip_request_id: String,
    client_request_id: String,
    expected_operation_generation: i64,
    expected_state_version: i64,
}

fn validate_control_identity(value: &str, label: &str) -> AppResult<()> {
    let valid = !value.is_empty()
        && value.encode_utf16().count() <= 256
        && value.chars().all(|c| {
            !c.is_whitespace() && !c.is_control() && !matches!(c, '/' | '\\' | '?' | '#')
        });
    if !valid {
        return Err(AppError::BadRequest(format!("Invalid {label}.")));
    }
    Ok(())
}

fn validate_prompt(prompt: &str, max_chars: usize) -> AppResult<()> {
    if prompt.trim().is_empty() || prompt.chars().count() > max_chars {
        return Err(AppError::BadRequest(format!(
            "Prompt must contain between 1 and {max_chars} characters."
        )));
    }
    Ok(())
}

fn validate_nonnegative(value: i64, label: &str) -> AppResult<()> {
    if value < 0 {
        return Err(AppError::BadRequest(format!("Invalid {label}.")));
    }
    Ok(())
}

fn validate_positive(value: i64, label: &str) -> AppResult<()> {
    if value <= 0 {
        return Err(AppError::BadRequest(format!("Invalid {label}.")));
    }
    Ok(())
}

fn normalize_reason(reason: Option<String>) -> AppResult<Option<String>> {
    let Some(reason) = reason else {
        return Ok(None);
    };
    let trimmed = reason.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().count() > APPROVAL_REASON_MAX_CHARS {
        return Err(AppError::BadRequest("Invalid approval reason.".to_string()));
    }
    Ok(Some(trimmed.to_string()))
}

fn reject_secret_shaped_command(value: &serde_json::Value) -> AppResult<()> {
    if contains_secret_shaped_value(value) {
        return Err(AppError::BadRequest(
            "Assistant chat command contained secret material.".to_string(),
        ));
    }
    Ok(())
}

fn contains_secret_shaped_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(text) => FORBIDDEN_COMMAND_VALUE.is_match(text),
        serde_json::Value::Array(entries) => entries.iter().any(contains_secret_shaped_value),
        serde_json::Value::Object(entries) => entries.iter().any(|(key, entry)| {
            FORBIDDEN_COMMAND_KEY.is_match(key) || contains_secret_shaped_value(entry)
        }),
        _ => false,
    }
}

fn parse_action_resource(value: Option<serde_json::Value>) -> AppResult<Option<ActionResource>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let Some(resource) = value.as_object() else {
        return Err(AppError::BadRequest("Invalid action resource.".to_string()));
    };
    if resource.len() != 1 {
        return Err(AppError::BadRequest("Invalid action resource.".to_string()));
    }
    let (variant, payload) = resource.iter().next().expect("resource has one entry");
    let Some(payload) = payload.as_object() else {
        return Err(AppError::BadRequest("Invalid action resource.".to_string()));
    };
    if payload.len() != 1 {
        return Err(AppError::BadRequest("Invalid action resource.".to_string()));
    }
    let parse_identity =
        |payload: &serde_json::Map<String, serde_json::Value>,
         key: &str,
         label: &str|
         -> AppResult<String> {
            let value = payload
                .get(key)
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| AppError::BadRequest("Invalid action resource.".to_string()))?;
            validate_control_identity(value, label)?;
            Ok(value.to_string())
        };
    let resource = match variant.as_str() {
        "userService" => ActionResource::UserService {
            user_service_id: parse_identity(payload, "userServiceId", "userServiceId")?,
        },
        "key" => ActionResource::Key {
            key_id: parse_identity(payload, "keyId", "keyId")?,
        },
        "node" => ActionResource::Node {
            node_id: parse_identity(payload, "nodeId", "nodeId")?,
        },
        "serviceAccount" => ActionResource::ServiceAccount {
            service_account_id: parse_identity(
                payload,
                "serviceAccountId",
                "serviceAccountId",
            )?,
        },
        "developerApp" => ActionResource::DeveloperApp {
            client_id: parse_identity(payload, "clientId", "clientId")?,
        },
        "device" => ActionResource::Device {
            device_id: parse_identity(payload, "deviceId", "deviceId")?,
        },
        _ => return Err(AppError::BadRequest("Invalid action resource.".to_string())),
    };
    Ok(Some(resource))
}

pub fn parse_assistant_chat_command(bytes: &[u8]) -> AppResult<AssistantChatCommand> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|e| AppError::BadRequest(format!("Invalid assistant chat request: {e}")))?;
    let command_type = value
        .get("type")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| AppError::BadRequest("Assistant chat commands require a type.".to_string()))?;
    reject_secret_shaped_command(&value)?;

    match command_type {
        "text" => {
            let raw: RawTextChatCommand = serde_json::from_value(value).map_err(|e| {
                AppError::BadRequest(format!("Invalid text chat request: {e}"))
            })?;
            validate_prompt(&raw.prompt, TYPED_CHAT_PROMPT_MAX_CHARS)?;
            validate_control_identity(&raw.client_request_id, "clientRequestId")?;
            if let Some(conversation_id) = raw.conversation_id.as_deref() {
                validate_conversation_id(conversation_id)?;
            }
            Ok(AssistantChatCommand::Text(TextChatCommand {
                prompt: raw.prompt,
                client_request_id: raw.client_request_id,
                conversation_id: raw.conversation_id,
            }))
        }
        "action.continue" => {
            let raw: RawActionContinueCommand =
                serde_json::from_value(value).map_err(|e| {
                    AppError::BadRequest(format!("Invalid action continuation request: {e}"))
                })?;
            validate_conversation_id(&raw.conversation_id)?;
            validate_control_identity(&raw.client_request_id, "clientRequestId")?;
            if let Some(origin_turn_id) = raw.origin_turn_id.as_deref() {
                validate_control_identity(origin_turn_id, "originTurnId")?;
            }
            if raw.actions.len() > ACTION_CONTINUATION_MAX_REPORTS {
                return Err(AppError::BadRequest(
                    "Too many action reports.".to_string(),
                ));
            }
            if !raw.actions.is_empty() && raw.origin_turn_id.is_none() {
                return Err(AppError::BadRequest(
                    "Action continuations with reports require originTurnId.".to_string(),
                ));
            }
            let mut seen = HashSet::new();
            let mut actions = Vec::with_capacity(raw.actions.len());
            for report in raw.actions {
                validate_control_identity(&report.action_request_id, "actionRequestId")?;
                validate_control_identity(&report.origin_turn_id, "originTurnId")?;
                if let Some(origin_turn_id) = raw.origin_turn_id.as_deref() {
                    if report.origin_turn_id != origin_turn_id {
                        return Err(AppError::BadRequest(
                            "Action report originTurnId must match the continuation origin."
                                .to_string(),
                        ));
                    }
                }
                if !seen.insert(report.action_request_id.clone()) {
                    return Err(AppError::BadRequest(
                        "Duplicate actionRequestId in action continuation.".to_string(),
                    ));
                }
                let disposition = ActionDisposition::parse(&report.disposition)?;
                let resource = parse_action_resource(report.resource)?;
                if disposition == ActionDisposition::Completed
                    && !resource.as_ref().is_some_and(ActionResource::is_user_service)
                {
                    return Err(AppError::BadRequest(
                        "Completed action reports require resource.userService.userServiceId."
                            .to_string(),
                    ));
                }
                actions.push(ActionReport {
                    action_request_id: report.action_request_id,
                    origin_turn_id: report.origin_turn_id,
                    disposition,
                    resource,
                });
            }
            Ok(AssistantChatCommand::ActionContinue(ActionContinueCommand {
                conversation_id: raw.conversation_id,
                client_request_id: raw.client_request_id,
                origin_turn_id: raw.origin_turn_id,
                actions,
            }))
        }
        "approval.resolve" => {
            let raw: RawApprovalResolveCommand = serde_json::from_value(value).map_err(|e| {
                AppError::BadRequest(format!("Invalid approval resolution request: {e}"))
            })?;
            validate_conversation_id(&raw.conversation_id)?;
            validate_control_identity(&raw.client_request_id, "clientRequestId")?;
            validate_control_identity(&raw.request_id, "requestId")?;
            let reason = normalize_reason(raw.reason)?;
            Ok(AssistantChatCommand::ApprovalResolve(ApprovalResolveCommand {
                conversation_id: raw.conversation_id,
                client_request_id: raw.client_request_id,
                request_id: raw.request_id,
                approved: raw.approved,
                reason,
            }))
        }
        "task.stop" => {
            let raw: RawTaskStopCommand = serde_json::from_value(value).map_err(|e| {
                AppError::BadRequest(format!("Invalid stop request: {e}"))
            })?;
            validate_conversation_id(&raw.conversation_id)?;
            validate_control_identity(&raw.turn_id, "turnId")?;
            validate_control_identity(&raw.stop_request_id, "stopRequestId")?;
            validate_control_identity(&raw.client_request_id, "clientRequestId")?;
            validate_nonnegative(raw.expected_state_version, "expectedStateVersion")?;
            Ok(AssistantChatCommand::TaskStop(TaskStopCommand {
                conversation_id: raw.conversation_id,
                turn_id: raw.turn_id,
                stop_request_id: raw.stop_request_id,
                client_request_id: raw.client_request_id,
                expected_state_version: raw.expected_state_version,
            }))
        }
        "task.steer" => {
            let raw: RawTaskSteerCommand = serde_json::from_value(value).map_err(|e| {
                AppError::BadRequest(format!("Invalid steering request: {e}"))
            })?;
            validate_conversation_id(&raw.conversation_id)?;
            validate_control_identity(&raw.turn_id, "turnId")?;
            validate_control_identity(&raw.steering_id, "steeringId")?;
            validate_control_identity(&raw.client_request_id, "clientRequestId")?;
            validate_nonnegative(raw.expected_state_version, "expectedStateVersion")?;
            if raw.instruction.trim().is_empty() {
                return Err(AppError::BadRequest("Invalid instruction.".to_string()));
            }
            Ok(AssistantChatCommand::TaskSteer(TaskSteerCommand {
                conversation_id: raw.conversation_id,
                turn_id: raw.turn_id,
                steering_id: raw.steering_id,
                client_request_id: raw.client_request_id,
                instruction: raw.instruction,
                expected_state_version: raw.expected_state_version,
            }))
        }
        "step.retry" => {
            let raw: RawStepRetryCommand = serde_json::from_value(value).map_err(|e| {
                AppError::BadRequest(format!("Invalid retry request: {e}"))
            })?;
            validate_conversation_id(&raw.conversation_id)?;
            validate_control_identity(&raw.turn_id, "turnId")?;
            validate_control_identity(&raw.task_id, "taskId")?;
            validate_control_identity(&raw.step_id, "stepId")?;
            validate_control_identity(&raw.retry_request_id, "retryRequestId")?;
            validate_control_identity(&raw.client_request_id, "clientRequestId")?;
            validate_positive(
                raw.expected_operation_generation,
                "expectedOperationGeneration",
            )?;
            validate_nonnegative(raw.expected_state_version, "expectedStateVersion")?;
            Ok(AssistantChatCommand::StepRetry(StepRetryCommand {
                conversation_id: raw.conversation_id,
                turn_id: raw.turn_id,
                task_id: raw.task_id,
                step_id: raw.step_id,
                retry_request_id: raw.retry_request_id,
                client_request_id: raw.client_request_id,
                expected_operation_generation: raw.expected_operation_generation,
                expected_state_version: raw.expected_state_version,
            }))
        }
        "step.skip" => {
            let raw: RawStepSkipCommand = serde_json::from_value(value).map_err(|e| {
                AppError::BadRequest(format!("Invalid skip request: {e}"))
            })?;
            validate_conversation_id(&raw.conversation_id)?;
            validate_control_identity(&raw.turn_id, "turnId")?;
            validate_control_identity(&raw.task_id, "taskId")?;
            validate_control_identity(&raw.step_id, "stepId")?;
            validate_control_identity(&raw.skip_request_id, "skipRequestId")?;
            validate_control_identity(&raw.client_request_id, "clientRequestId")?;
            validate_positive(
                raw.expected_operation_generation,
                "expectedOperationGeneration",
            )?;
            validate_nonnegative(raw.expected_state_version, "expectedStateVersion")?;
            Ok(AssistantChatCommand::StepSkip(StepSkipCommand {
                conversation_id: raw.conversation_id,
                turn_id: raw.turn_id,
                task_id: raw.task_id,
                step_id: raw.step_id,
                skip_request_id: raw.skip_request_id,
                client_request_id: raw.client_request_id,
                expected_operation_generation: raw.expected_operation_generation,
                expected_state_version: raw.expected_state_version,
            }))
        }
        _ => Err(AppError::BadRequest(
            "Unsupported assistant chat command.".to_string(),
        )),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ConversationUpdatedAtSortKey {
    parsed: Option<DateTime<FixedOffset>>,
    raw: String,
}

fn conversation_updated_at_key(row: &serde_json::Value) -> ConversationUpdatedAtSortKey {
    for key in [
        "updatedAt",
        "updated_at",
        "lastMessageAt",
        "last_message_at",
        "createdAt",
        "created_at",
    ] {
        if let Some(value) = row.get(key).and_then(serde_json::Value::as_str) {
            return ConversationUpdatedAtSortKey {
                parsed: DateTime::parse_from_rfc3339(value).ok(),
                raw: value.to_string(),
            };
        }
    }
    ConversationUpdatedAtSortKey {
        parsed: None,
        raw: String::new(),
    }
}

pub fn prepare_assistant_chat_command(
    command: &AssistantChatCommand,
) -> AppResult<PreparedAssistantChatCommand> {
    match command {
        AssistantChatCommand::Text(command) => {
            validate_prompt(&command.prompt, TYPED_CHAT_PROMPT_MAX_CHARS)?;
            validate_control_identity(&command.client_request_id, "clientRequestId")?;
            if let Some(conversation_id) = command.conversation_id.as_deref() {
                validate_conversation_id(conversation_id)?;
            }
            Ok(PreparedAssistantChatCommand {
                body: serde_json::json!({
                    "type": "text",
                    "prompt": command.prompt,
                    "clientRequestId": command.client_request_id,
                    "conversationId": command.conversation_id,
                })
                .as_object()
                .map(|body| {
                    let mut body = body.clone();
                    if command.conversation_id.is_none() {
                        body.remove("conversationId");
                    }
                    serde_json::Value::Object(body)
                })
                .expect("text command body is an object"),
                client_request_id: command.client_request_id.clone(),
                response_kind: AssistantChatResponseKind::Stream,
            })
        }
        AssistantChatCommand::ActionContinue(command) => {
            let mut actions = Vec::with_capacity(command.actions.len());
            for report in &command.actions {
                let mut body = serde_json::Map::new();
                body.insert(
                    "actionRequestId".to_string(),
                    serde_json::Value::String(report.action_request_id.clone()),
                );
                body.insert(
                    "originTurnId".to_string(),
                    serde_json::Value::String(report.origin_turn_id.clone()),
                );
                body.insert(
                    "disposition".to_string(),
                    serde_json::Value::String(report.disposition.as_str().to_string()),
                );
                if let Some(resource) = &report.resource {
                    body.insert("resource".to_string(), resource.to_json());
                }
                actions.push(serde_json::Value::Object(body));
            }
            let mut body = serde_json::Map::new();
            body.insert(
                "type".to_string(),
                serde_json::Value::String("action.continue".to_string()),
            );
            body.insert(
                "conversationId".to_string(),
                serde_json::Value::String(command.conversation_id.clone()),
            );
            body.insert(
                "clientRequestId".to_string(),
                serde_json::Value::String(command.client_request_id.clone()),
            );
            if let Some(origin_turn_id) = &command.origin_turn_id {
                body.insert(
                    "originTurnId".to_string(),
                    serde_json::Value::String(origin_turn_id.clone()),
                );
            }
            body.insert("actions".to_string(), serde_json::Value::Array(actions));
            Ok(PreparedAssistantChatCommand {
                body: serde_json::Value::Object(body),
                client_request_id: command.client_request_id.clone(),
                response_kind: AssistantChatResponseKind::Stream,
            })
        }
        AssistantChatCommand::ApprovalResolve(command) => {
            let mut body = serde_json::Map::new();
            body.insert(
                "type".to_string(),
                serde_json::Value::String("approval.resolve".to_string()),
            );
            body.insert(
                "conversationId".to_string(),
                serde_json::Value::String(command.conversation_id.clone()),
            );
            body.insert(
                "clientRequestId".to_string(),
                serde_json::Value::String(command.client_request_id.clone()),
            );
            body.insert(
                "requestId".to_string(),
                serde_json::Value::String(command.request_id.clone()),
            );
            body.insert("approved".to_string(), serde_json::Value::Bool(command.approved));
            if let Some(reason) = &command.reason {
                body.insert(
                    "reason".to_string(),
                    serde_json::Value::String(reason.clone()),
                );
            }
            Ok(PreparedAssistantChatCommand {
                body: serde_json::Value::Object(body),
                client_request_id: command.client_request_id.clone(),
                response_kind: AssistantChatResponseKind::Stream,
            })
        }
        AssistantChatCommand::TaskStop(command) => Ok(PreparedAssistantChatCommand {
            body: serde_json::json!({
                "type": "task.stop",
                "conversationId": command.conversation_id,
                "turnId": command.turn_id,
                "stopRequestId": command.stop_request_id,
                "clientRequestId": command.client_request_id,
                "expectedStateVersion": command.expected_state_version,
            }),
            client_request_id: command.client_request_id.clone(),
            response_kind: AssistantChatResponseKind::Json,
        }),
        AssistantChatCommand::TaskSteer(command) => Ok(PreparedAssistantChatCommand {
            body: serde_json::json!({
                "type": "task.steer",
                "conversationId": command.conversation_id,
                "turnId": command.turn_id,
                "steeringId": command.steering_id,
                "clientRequestId": command.client_request_id,
                "instruction": command.instruction,
                "expectedStateVersion": command.expected_state_version,
            }),
            client_request_id: command.client_request_id.clone(),
            response_kind: AssistantChatResponseKind::Json,
        }),
        AssistantChatCommand::StepRetry(command) => Ok(PreparedAssistantChatCommand {
            body: serde_json::json!({
                "type": "step.retry",
                "conversationId": command.conversation_id,
                "turnId": command.turn_id,
                "taskId": command.task_id,
                "stepId": command.step_id,
                "retryRequestId": command.retry_request_id,
                "clientRequestId": command.client_request_id,
                "expectedOperationGeneration": command.expected_operation_generation,
                "expectedStateVersion": command.expected_state_version,
            }),
            client_request_id: command.client_request_id.clone(),
            response_kind: AssistantChatResponseKind::Json,
        }),
        AssistantChatCommand::StepSkip(command) => Ok(PreparedAssistantChatCommand {
            body: serde_json::json!({
                "type": "step.skip",
                "conversationId": command.conversation_id,
                "turnId": command.turn_id,
                "taskId": command.task_id,
                "stepId": command.step_id,
                "skipRequestId": command.skip_request_id,
                "clientRequestId": command.client_request_id,
                "expectedOperationGeneration": command.expected_operation_generation,
                "expectedStateVersion": command.expected_state_version,
            }),
            client_request_id: command.client_request_id.clone(),
            response_kind: AssistantChatResponseKind::Json,
        }),
    }
}

/// The one workflow the assistant surface may start. Pinned server-side:
/// Aevatar's `/api/chat` runs whatever catalog workflow the body names
/// (`direct`, `auto`, `auto_review`, file-loaded definitions, …), and which
/// engine backs the platform chat is a platform decision, not a caller input.
pub const WORKFLOW_CHAT_WORKFLOW: &str = "studio";

/// Matches the client cap in `aevatar-transport.ts` (`MAX_MESSAGE_CHARS`).
const WORKFLOW_CHAT_PROMPT_MAX_CHARS: usize = 32_768;

/// The caller half of the workflow-chat turn contract. Everything else in
/// Aevatar's `HttpChatInput` (workflow selection, inline YAML, llmControl,
/// toolContext, metadata, headers) is deliberately not expressible here.
///
/// `deny_unknown_fields` keeps client drift loud: Aevatar rejects unknown
/// body members (`JsonUnmappedMemberHandling.Disallow`), and a field that
/// silently vanished here would surface as a confusing upstream 400 instead.
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkflowChatTurnRequest {
    pub prompt: String,
    /// `chatc-…` conversation to continue; absent starts a new conversation.
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// Chat-history read fence for continuations: the `stateVersion` the
    /// client last observed. Aevatar requires `> 0` alongside a conversation
    /// id (`CHAT_HISTORY_RESERVATION_UNAVAILABLE` otherwise).
    #[serde(default)]
    pub minimum_state_version: Option<i64>,
    /// Client idempotency identity for the create/turn (Aevatar replays the
    /// same conversation/turn for a repeated id, 409s on payload mismatch).
    #[serde(default)]
    pub command_id: Option<String>,
}

/// Opaque client token (command ids): UUID-shaped material only, so nothing
/// structural or attacker-shaped rides through to the upstream body.
fn validate_client_token(value: &str, label: &str) -> AppResult<()> {
    let valid = !value.is_empty()
        && value.len() <= 64
        && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        return Err(AppError::BadRequest(format!("Invalid {label}.")));
    }
    Ok(())
}

/// Build the upstream `/api/chat` body for a caller turn request.
///
/// The `conversation` object is always present: without it Aevatar runs the
/// turn ephemerally and **persists nothing** to chat history, which would
/// silently drop the conversation from the sidebar contract. `workflow` is
/// pinned to [`WORKFLOW_CHAT_WORKFLOW`]. A body `scopeId` is ignored by
/// Aevatar (trusted scope wins), so none is sent.
pub fn workflow_chat_body(request: &WorkflowChatTurnRequest) -> AppResult<serde_json::Value> {
    let prompt = request.prompt.trim();
    if prompt.is_empty() || request.prompt.chars().count() > WORKFLOW_CHAT_PROMPT_MAX_CHARS {
        return Err(AppError::BadRequest(format!(
            "Prompt must contain between 1 and {WORKFLOW_CHAT_PROMPT_MAX_CHARS} characters."
        )));
    }

    let conversation = match (&request.conversation_id, request.minimum_state_version) {
        (None, None) => serde_json::json!({ "conversationId": null }),
        (None, Some(_)) => {
            return Err(AppError::BadRequest(
                "minimumStateVersion requires a conversationId.".to_string(),
            ));
        }
        (Some(id), version) => {
            validate_conversation_id(id)?;
            if !id.starts_with(WORKFLOW_CHAT_CONVERSATION_PREFIX) {
                // A `nyxid-chat-…` actor id is a different surface; failing
                // fast beats an upstream CONVERSATION_NOT_FOUND after a run
                // was already admitted.
                return Err(AppError::BadRequest(
                    "Only workflow conversations can be continued here.".to_string(),
                ));
            }
            let Some(version) = version.filter(|v| *v > 0) else {
                return Err(AppError::BadRequest(
                    "Continuing a conversation requires the last observed minimumStateVersion."
                        .to_string(),
                ));
            };
            serde_json::json!({ "conversationId": id, "minimumStateVersion": version })
        }
    };

    let command_id = match &request.command_id {
        Some(id) => {
            validate_client_token(id, "commandId")?;
            id.clone()
        }
        None => uuid::Uuid::new_v4().to_string(),
    };

    Ok(serde_json::json!({
        "commandId": command_id,
        "conversation": conversation,
        "prompt": request.prompt,
        "workflow": WORKFLOW_CHAT_WORKFLOW,
    }))
}

/// `api/ws/chat` -- WebSocket twin of the workflow chat
/// (`StartWorkflowChatWebSocket`).
pub fn workflow_chat_ws_path() -> String {
    "api/ws/chat".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const USER: &str = "add69059-bece-4f0e-9559-99cfd10b47eb";
    const CONV: &str = "nyxid-chat-f8369965a444433f92ec50e67ad8ee52";

    const WORKFLOW_CONV: &str = "chatc-650906f30cc985fa341477281303b6de";

    fn parse_command(value: serde_json::Value) -> AssistantChatCommand {
        parse_assistant_chat_command(&serde_json::to_vec(&value).unwrap()).unwrap()
    }

    fn workflow_turn_request(value: serde_json::Value) -> WorkflowChatTurnRequest {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn filters_indexes_to_the_addressable_families_only() {
        let mut index = json!({
            "conversations": [
                { "id": WORKFLOW_CONV, "updatedAt": "2026-07-29T12:00:00.000Z" },
                { "id": CONV, "updatedAt": "2026-07-29T13:00:00.000Z" },
                { "id": "voicec-1", "updatedAt": "2026-07-29T14:00:00.000Z" },
                { "title": "unknown-shaped row" }
            ]
        });

        assert!(filter_addressable_conversation_index(&mut index));
        assert_eq!(
            index["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row.get("id").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>(),
            vec![Some(WORKFLOW_CONV), Some(CONV), None]
        );
    }

    #[test]
    fn merge_prefers_canonical_rows_and_appends_workflow_history_newest_first() {
        let mut canonical = json!({
            "conversations": [
                {
                    "id": CONV,
                    "title": "typed",
                    "updatedAt": "2026-07-29T13:00:00.000Z"
                }
            ]
        });
        let legacy = json!({
            "conversations": [
                {
                    "id": WORKFLOW_CONV,
                    "title": "workflow",
                    "updatedAt": "2026-07-29T14:00:00.000Z"
                },
                {
                    "id": CONV,
                    "title": "stale duplicate",
                    "updatedAt": "2026-07-29T12:00:00.000Z"
                }
            ]
        });

        assert!(merge_workflow_history_rows(&mut canonical, &legacy));
        assert_eq!(
            canonical["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![WORKFLOW_CONV, CONV]
        );
        assert_eq!(
            canonical["conversations"][1]["title"].as_str(),
            Some("typed"),
            "canonical rows must win the dedupe"
        );
    }

    #[test]
    fn merge_sorts_rfc3339_offsets_across_updated_and_created_keys() {
        let mut canonical = json!({
            "conversations": [
                {
                    "id": CONV,
                    "title": "typed",
                    "createdAt": "2026-07-29T05:30:00.000Z"
                }
            ]
        });
        let legacy = json!({
            "conversations": [
                {
                    "id": WORKFLOW_CONV,
                    "title": "workflow",
                    "updatedAt": "2026-07-29T13:00:00.000+08:00"
                }
            ]
        });

        assert!(merge_workflow_history_rows(&mut canonical, &legacy));
        assert_eq!(
            canonical["conversations"]
                .as_array()
                .unwrap()
                .iter()
                .map(|row| row["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![CONV, WORKFLOW_CONV],
            "RFC3339 offsets and mixed timestamp keys must sort chronologically"
        );
    }

    #[test]
    fn canonical_index_workflow_detection_is_prefix_based() {
        assert!(conversation_index_includes_workflow(&json!({
            "conversations": [{ "id": WORKFLOW_CONV }]
        })));
        assert!(!conversation_index_includes_workflow(&json!({
            "conversations": [{ "id": CONV }]
        })));
    }

    #[test]
    fn builds_canonical_paths_only() {
        assert_eq!(
            history_index_path(USER),
            format!("api/scopes/{USER}/chat-history")
        );
        assert_eq!(canonical_conversations_path(), "api/chat/conversations");
        assert_eq!(
            canonical_conversation_path(CONV).unwrap(),
            format!("api/chat/conversations/{CONV}")
        );
        assert_eq!(
            canonical_state_path(CONV).unwrap(),
            format!("api/chat/conversations/{CONV}/state")
        );
        assert_eq!(typed_chat_path(), "api/chat");
        assert_eq!(workflow_chat_path(), "api/chat");
        assert_eq!(workflow_chat_ws_path(), "api/ws/chat");
        assert_eq!(completions_path(), "v1/chat/completions");
    }

    #[test]
    fn rejects_conversation_ids_that_would_escape_the_canonical_path_segment() {
        for bad in [
            "",
            "../../admin",
            "abc/def",
            "abc:stream",
            "abc?x=1",
            "abc#frag",
            "abc def",
            "%2e%2e",
        ] {
            assert!(
                canonical_conversation_path(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
            assert!(canonical_state_path(bad).is_err());
        }
    }

    #[test]
    fn migration_guard_keeps_scoped_typed_paths_out_of_runtime_source() {
        const SOURCE: &str = include_str!("assistant_service.rs");
        let scoped_prefix = ["nyxid", "-chat/"].concat();
        let legacy_history_path = ["/chat-history", "/conversations"].concat();
        assert!(
            !SOURCE.contains(&scoped_prefix),
            "scoped typed command/resource paths must not return"
        );
        assert!(
            !SOURCE.contains(&legacy_history_path),
            "legacy transcript/delete paths must not return to assistant_service.rs"
        );
    }

    #[test]
    fn parses_text_commands_for_first_turn_and_continuation() {
        let create = parse_command(json!({
            "type": "text",
            "prompt": "show github repos",
            "clientRequestId": "00000000-0000-4000-8000-000000000001"
        }));
        let continue_turn = parse_command(json!({
            "type": "text",
            "prompt": "and then",
            "clientRequestId": "00000000-0000-4000-8000-000000000002",
            "conversationId": CONV
        }));

        assert_eq!(
            prepare_assistant_chat_command(&create).unwrap(),
            PreparedAssistantChatCommand {
                body: json!({
                    "type": "text",
                    "prompt": "show github repos",
                    "clientRequestId": "00000000-0000-4000-8000-000000000001"
                }),
                client_request_id: "00000000-0000-4000-8000-000000000001".to_string(),
                response_kind: AssistantChatResponseKind::Stream,
            }
        );
        assert_eq!(
            prepare_assistant_chat_command(&create)
                .unwrap()
                .response_kind
                .accept_header_value(),
            "text/event-stream"
        );
        assert_eq!(
            prepare_assistant_chat_command(&continue_turn).unwrap(),
            PreparedAssistantChatCommand {
                body: json!({
                    "type": "text",
                    "prompt": "and then",
                    "clientRequestId": "00000000-0000-4000-8000-000000000002",
                    "conversationId": CONV,
                }),
                client_request_id: "00000000-0000-4000-8000-000000000002".to_string(),
                response_kind: AssistantChatResponseKind::Stream,
            }
        );
        assert_eq!(
            prepare_assistant_chat_command(&continue_turn)
                .unwrap()
                .response_kind
                .accept_header_value(),
            "text/event-stream"
        );
    }

    #[test]
    fn rejects_invalid_text_commands_and_unknown_fields() {
        for value in [
            json!({
                "type": "text",
                "prompt": " ",
                "clientRequestId": "request-1"
            }),
            json!({
                "type": "text",
                "prompt": "hi",
                "clientRequestId": "bad/request"
            }),
            json!({
                "type": "text",
                "prompt": "hi",
                "clientRequestId": "request-1",
                "scopeId": "someone-else"
            }),
        ] {
            assert!(parse_assistant_chat_command(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }

    #[test]
    fn parses_action_continue_for_reports_and_resource_free_wakes() {
        let continuation = parse_command(json!({
            "type": "action.continue",
            "conversationId": CONV,
            "clientRequestId": "request-1",
            "originTurnId": "turn-1",
            "actions": [
                {
                    "actionRequestId": "act-1",
                    "originTurnId": "turn-1",
                    "disposition": "completed",
                    "resource": {
                        "userService": {
                            "userServiceId": "00000000-0000-4000-8000-000000000123"
                        }
                    }
                }
            ]
        }));
        let wake = parse_command(json!({
            "type": "action.continue",
            "conversationId": CONV,
            "clientRequestId": "request-2",
            "actions": []
        }));

        assert_eq!(
            prepare_assistant_chat_command(&continuation).unwrap().body,
            json!({
                "type": "action.continue",
                "conversationId": CONV,
                "clientRequestId": "request-1",
                "originTurnId": "turn-1",
                "actions": [
                    {
                        "actionRequestId": "act-1",
                        "originTurnId": "turn-1",
                        "disposition": "completed",
                        "resource": {
                            "userService": {
                                "userServiceId": "00000000-0000-4000-8000-000000000123"
                            }
                        }
                    }
                ]
            })
        );
        assert_eq!(
            prepare_assistant_chat_command(&wake).unwrap().body,
            json!({
                "type": "action.continue",
                "conversationId": CONV,
                "clientRequestId": "request-2",
                "actions": []
            })
        );
        assert_eq!(
            prepare_assistant_chat_command(&continuation)
                .unwrap()
                .response_kind
                .accept_header_value(),
            "text/event-stream"
        );
    }

    #[test]
    fn rejects_invalid_action_continuations_fail_closed() {
        for value in [
            json!({
                "type": "action.continue",
                "conversationId": CONV,
                "clientRequestId": "request-1",
                "actions": [
                    {
                        "actionRequestId": "act-1",
                        "originTurnId": "turn-1",
                        "disposition": "declined"
                    }
                ]
            }),
            json!({
                "type": "action.continue",
                "conversationId": CONV,
                "clientRequestId": "request-1",
                "originTurnId": "turn-1",
                "actions": [
                    {
                        "actionRequestId": "act-1",
                        "originTurnId": "turn-2",
                        "disposition": "declined"
                    }
                ]
            }),
            json!({
                "type": "action.continue",
                "conversationId": CONV,
                "clientRequestId": "request-1",
                "originTurnId": "turn-1",
                "actions": [
                    {
                        "actionRequestId": "act-1",
                        "originTurnId": "turn-1",
                        "disposition": "completed"
                    }
                ]
            }),
            json!({
                "type": "action.continue",
                "conversationId": CONV,
                "clientRequestId": "request-1",
                "originTurnId": "turn-1",
                "actions": [
                    {
                        "actionRequestId": "act-1",
                        "originTurnId": "turn-1",
                        "disposition": "declined"
                    },
                    {
                        "actionRequestId": "act-1",
                        "originTurnId": "turn-1",
                        "disposition": "failed"
                    }
                ]
            }),
            json!({
                "type": "action.continue",
                "conversationId": CONV,
                "clientRequestId": "request-1",
                "originTurnId": "turn-1",
                "actions": [
                    {
                        "actionRequestId": "act-1",
                        "originTurnId": "turn-1",
                        "disposition": "declined",
                        "resource": {
                            "device": {
                                "deviceId": "Bearer definitely-not-allowed"
                            }
                        }
                    }
                ]
            }),
        ] {
            assert!(parse_assistant_chat_command(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }

    #[test]
    fn rejects_secret_shaped_approval_resolution_reason_before_deserialization() {
        let value = json!({
            "type": "approval.resolve",
            "conversationId": CONV,
            "clientRequestId": "request-approval-1",
            "requestId": "approval-1",
            "approved": true,
            "reason": "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9"
        });

        assert!(parse_assistant_chat_command(&serde_json::to_vec(&value).unwrap()).is_err());
    }

    #[test]
    fn rejects_secret_shaped_task_steer_instruction_before_deserialization() {
        let value = json!({
            "type": "task.steer",
            "conversationId": CONV,
            "turnId": "turn-1",
            "steeringId": "steer-1",
            "clientRequestId": "request-steer-1",
            "instruction": "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9",
            "expectedStateVersion": 2
        });

        assert!(parse_assistant_chat_command(&serde_json::to_vec(&value).unwrap()).is_err());
    }

    #[test]
    fn rejects_secret_shaped_values_across_command_variants() {
        for value in [
            json!({
                "type": "text",
                "prompt": "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9",
                "clientRequestId": "request-text-1"
            }),
            json!({
                "type": "action.continue",
                "conversationId": CONV,
                "clientRequestId": "request-action-1",
                "originTurnId": "turn-1",
                "actions": [
                    {
                        "actionRequestId": "act-1",
                        "originTurnId": "turn-1",
                        "disposition": "declined",
                        "resource": {
                            "device": {
                                "deviceId": "nyxid_secret_abcdefgh"
                            }
                        }
                    }
                ]
            }),
            json!({
                "type": "task.stop",
                "conversationId": CONV,
                "turnId": "nyxid_secret_abcdefgh",
                "stopRequestId": "stop-1",
                "clientRequestId": "request-stop-1",
                "expectedStateVersion": 0
            }),
            json!({
                "type": "step.retry",
                "conversationId": CONV,
                "turnId": "turn-1",
                "taskId": "task-1",
                "stepId": "nyxid_secret_abcdefgh",
                "retryRequestId": "retry-1",
                "clientRequestId": "request-retry-1",
                "expectedOperationGeneration": 2,
                "expectedStateVersion": 3
            }),
            json!({
                "type": "step.skip",
                "conversationId": CONV,
                "turnId": "turn-1",
                "taskId": "task-1",
                "stepId": "nyxid_secret_abcdefgh",
                "skipRequestId": "skip-1",
                "clientRequestId": "request-skip-1",
                "expectedOperationGeneration": 4,
                "expectedStateVersion": 5
            }),
        ] {
            assert!(parse_assistant_chat_command(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }

    #[test]
    fn builds_exact_approval_and_control_bodies() {
        let approval = prepare_assistant_chat_command(&parse_command(json!({
            "type": "approval.resolve",
            "conversationId": CONV,
            "clientRequestId": "request-approval-1",
            "requestId": "approval-1",
            "approved": true,
            "reason": "Approved by user"
        })))
        .unwrap();
        let stop = prepare_assistant_chat_command(&parse_command(json!({
            "type": "task.stop",
            "conversationId": CONV,
            "turnId": "turn-1",
            "stopRequestId": "stop-1",
            "clientRequestId": "client-stop-1",
            "expectedStateVersion": 0
        })))
        .unwrap();
        let retry = prepare_assistant_chat_command(&parse_command(json!({
            "type": "step.retry",
            "conversationId": CONV,
            "turnId": "turn-1",
            "taskId": "task-1",
            "stepId": "step-1",
            "retryRequestId": "retry-1",
            "clientRequestId": "client-retry-1",
            "expectedOperationGeneration": 2,
            "expectedStateVersion": 3
        })))
        .unwrap();

        assert_eq!(approval.response_kind, AssistantChatResponseKind::Stream);
        assert_eq!(approval.response_kind.accept_header_value(), "text/event-stream");
        assert_eq!(
            approval.body,
            json!({
                "type": "approval.resolve",
                "conversationId": CONV,
                "clientRequestId": "request-approval-1",
                "requestId": "approval-1",
                "approved": true,
                "reason": "Approved by user"
            })
        );
        assert_eq!(stop.response_kind, AssistantChatResponseKind::Json);
        assert_eq!(stop.response_kind.accept_header_value(), "application/json");
        assert_eq!(
            stop.body,
            json!({
                "type": "task.stop",
                "conversationId": CONV,
                "turnId": "turn-1",
                "stopRequestId": "stop-1",
                "clientRequestId": "client-stop-1",
                "expectedStateVersion": 0
            })
        );
        assert_eq!(retry.response_kind, AssistantChatResponseKind::Json);
        assert_eq!(retry.response_kind.accept_header_value(), "application/json");
        assert_eq!(
            retry.body,
            json!({
                "type": "step.retry",
                "conversationId": CONV,
                "turnId": "turn-1",
                "taskId": "task-1",
                "stepId": "step-1",
                "retryRequestId": "retry-1",
                "clientRequestId": "client-retry-1",
                "expectedOperationGeneration": 2,
                "expectedStateVersion": 3
            })
        );
    }

    #[test]
    fn workflow_body_creates_a_conversation_with_the_pinned_workflow() {
        let body = workflow_chat_body(&workflow_turn_request(json!({
            "prompt": "hi",
            "commandId": "0d4b0a52-3d5f-4d2e-9f10-8f6f9b1c2d3e",
        })))
        .unwrap();
        assert_eq!(
            body,
            json!({
                "commandId": "0d4b0a52-3d5f-4d2e-9f10-8f6f9b1c2d3e",
                "conversation": { "conversationId": null },
                "prompt": "hi",
                "workflow": "studio",
            })
        );
    }

    #[test]
    fn workflow_body_continues_with_the_observed_state_version() {
        let body = workflow_chat_body(&workflow_turn_request(json!({
            "prompt": "and then?",
            "conversationId": WORKFLOW_CONV,
            "minimumStateVersion": 18,
        })))
        .unwrap();
        assert_eq!(body["conversation"]["conversationId"], WORKFLOW_CONV);
        assert_eq!(body["conversation"]["minimumStateVersion"], 18);
        assert_eq!(body["workflow"], "studio");
        assert!(body["commandId"].as_str().is_some_and(|id| !id.is_empty()));
    }

    #[test]
    fn workflow_body_rejects_out_of_contract_turns() {
        assert!(workflow_chat_body(&workflow_turn_request(json!({ "prompt": "  " }))).is_err());
        assert!(
            workflow_chat_body(&workflow_turn_request(json!({
                "prompt": "a".repeat(32_769)
            })))
            .is_err()
        );
        assert!(
            workflow_chat_body(&workflow_turn_request(json!({
                "prompt": "hi",
                "conversationId": WORKFLOW_CONV
            })))
            .is_err()
        );
        assert!(
            workflow_chat_body(&workflow_turn_request(json!({
                "prompt": "hi",
                "conversationId": WORKFLOW_CONV,
                "minimumStateVersion": 0
            })))
            .is_err()
        );
        assert!(
            workflow_chat_body(&workflow_turn_request(json!({
                "prompt": "hi",
                "minimumStateVersion": 3
            })))
            .is_err()
        );
        assert!(
            workflow_chat_body(&workflow_turn_request(json!({
                "prompt": "hi",
                "conversationId": CONV,
                "minimumStateVersion": 3
            })))
            .is_err()
        );
        assert!(
            workflow_chat_body(&workflow_turn_request(json!({
                "prompt": "hi",
                "conversationId": "chatc-a/b",
                "minimumStateVersion": 3
            })))
            .is_err()
        );
        assert!(
            workflow_chat_body(&workflow_turn_request(json!({
                "prompt": "hi",
                "commandId": "not a token!"
            })))
            .is_err()
        );
        assert!(
            serde_json::from_value::<WorkflowChatTurnRequest>(json!({
                "prompt": "hi",
                "workflow": "direct"
            }))
            .is_err()
        );
    }
}
