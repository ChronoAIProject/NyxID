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

    warn_if_bridge_disarmed(&service);

    Ok(service)
}

/// Surface the one row misconfiguration that silently 401s the entire chat
/// surface, instead of waiting for a user to report it.
///
/// Deployed Aevatar authenticates only `Authorization: Bearer <NyxID JWT>`.
/// The bridge that supplies it (`handlers/assistant.rs::needs_forward_token_bridge`)
/// is gated on `Session && forward_access_token`, so clearing that flag both
/// stops the bearer forward and disarms the mint: NyxID then sends no
/// `Authorization` at all and every Aevatar surface answers 401. This exact
/// flip took production chat down on 2026-08-12, and nothing in the system
/// said so — the only signal was a user seeing a failed chat.
///
/// Deliberately a warning, not a hard failure: `false` becomes the CORRECT
/// value once Aevatar validates `X-NyxID-Identity-Token` (the TD-3 cutover),
/// and failing closed here would turn that rollout into an outage of its own.
/// Fires once per process — the condition is process-lifetime config, so
/// per-request logging would only add noise.
fn warn_if_bridge_disarmed(service: &DownstreamService) {
    if service.forward_access_token {
        return;
    }
    static WARNED: std::sync::Once = std::sync::Once::new();
    WARNED.call_once(|| {
        tracing::error!(
            service_slug = %AEVATAR_SLUG,
            "assistant: '{AEVATAR_SLUG}' has forward_access_token=false, which disarms the \
             session bearer bridge. Unless Aevatar now validates X-NyxID-Identity-Token, every \
             assistant request will fail upstream with 401. Set forward_access_token=true to \
             restore chat."
        );
    });
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

/// Validate the exact actor-id shape generated by Aevatar's typed chat
/// controller (`nyxid-chat-{guid:N}`).
pub fn validate_typed_conversation_id(conversation_id: &str) -> AppResult<()> {
    let suffix = conversation_id.strip_prefix(NYXID_CHAT_ACTOR_PREFIX);
    let valid = suffix.is_some_and(|suffix| {
        suffix.len() == 32
            && suffix
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    });
    if !valid {
        return Err(AppError::BadRequest(
            "Invalid typed conversation id.".to_string(),
        ));
    }
    Ok(())
}

/// Legacy scoped Chat History index retained for `chatc-*` list/read/delete.
pub fn history_index_path(user_id: &str) -> String {
    format!("api/scopes/{user_id}/chat-history")
}

/// Canonical typed NyxIdChat conversation index.
pub fn canonical_conversations_path() -> String {
    "api/chat/conversations".to_string()
}

/// Actor-id prefix of a `nyxid-chat` conversation (upstream
/// `NyxIdChatServiceDefaults.ActorIdPrefix`; ids are `nyxid-chat-{guid:N}`).
const NYXID_CHAT_ACTOR_PREFIX: &str = "nyxid-chat-";

/// Conversation-id prefix retained for historical list/read/delete support
/// (upstream `ChatHistoryActorIds.CreateConversationId`; ids are
/// `chatc-{hash[..32]}`).
const LEGACY_CONVERSATION_PREFIX: &str = "chatc-";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConversationResourceFamily {
    Typed,
    Legacy,
}

pub fn conversation_resource_family(
    conversation_id: &str,
) -> AppResult<ConversationResourceFamily> {
    validate_conversation_id(conversation_id)?;
    if conversation_id.starts_with(NYXID_CHAT_ACTOR_PREFIX) {
        validate_typed_conversation_id(conversation_id)?;
        return Ok(ConversationResourceFamily::Typed);
    }
    if conversation_id.starts_with(LEGACY_CONVERSATION_PREFIX) {
        return Ok(ConversationResourceFamily::Legacy);
    }
    Err(AppError::NotFound("Conversation not found".to_string()))
}

/// Add one index page to a drained result, accepting only the source's
/// authoritative conversation family and keeping the first occurrence of
/// each id across both sources.
pub fn append_conversation_family_page(
    index: &serde_json::Value,
    family: ConversationResourceFamily,
    conversations: &mut Vec<serde_json::Value>,
    seen: &mut HashSet<String>,
) -> AppResult<Option<String>> {
    let Some(rows) = index
        .get("conversations")
        .and_then(serde_json::Value::as_array)
    else {
        // A mixed-version upstream can briefly return a different index shape.
        // Preserve the rows already drained instead of blanking the sidebar.
        return Ok(None);
    };

    for row in rows {
        let Some(id) = row.get("id").and_then(serde_json::Value::as_str) else {
            continue;
        };
        if conversation_resource_family(id).ok() == Some(family) && seen.insert(id.to_string()) {
            conversations.push(row.clone());
        }
    }

    match index.get("nextCursor") {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(cursor)) => {
            let cursor = cursor.trim();
            Ok((!cursor.is_empty()).then(|| cursor.to_string()))
        }
        Some(_) => Err(AppError::Internal(
            "assistant: chat history index returned an invalid nextCursor".to_string(),
        )),
    }
}

pub fn sort_conversation_rows_newest_first(conversations: &mut [serde_json::Value]) {
    conversations.sort_by(|left, right| {
        let left_key = conversation_updated_at_key(left);
        let right_key = conversation_updated_at_key(right);
        match (left_key.parsed.as_ref(), right_key.parsed.as_ref()) {
            (Some(left_time), Some(right_time)) => right_time.cmp(left_time),
            _ => right_key.raw.cmp(&left_key.raw),
        }
    });
}

/// `api/chat/conversations/{id}` -- canonical conversation detail/delete.
pub fn canonical_conversation_path(conversation_id: &str) -> AppResult<String> {
    validate_typed_conversation_id(conversation_id)?;
    Ok(format!("api/chat/conversations/{conversation_id}"))
}

/// Scoped workflow transcript/delete resource.
pub fn history_conversation_path(user_id: &str, conversation_id: &str) -> AppResult<String> {
    validate_conversation_id(conversation_id)?;
    Ok(format!(
        "{}/conversations/{conversation_id}",
        history_index_path(user_id)
    ))
}

/// `api/chat/conversations/{id}/state` -- canonical reconnect surface.
pub fn canonical_state_path(conversation_id: &str) -> AppResult<String> {
    Ok(format!(
        "{}/state",
        canonical_conversation_path(conversation_id)?
    ))
}

/// `v1/chat/completions` -- OpenAI-compatible surface. Scope-free: the
/// endpoint is stateless and carries its history in the request body.
pub fn completions_path() -> String {
    "v1/chat/completions".to_string()
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
const INPUT_ANSWER_MAX_CHARS: usize = 32_768;
const INPUT_SELECTION_MAX_OPTIONS: usize = 6;

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
    InputResolve(InputResolveCommand),
    ActionContinue(ActionContinueCommand),
    ApprovalResolve(ApprovalResolveCommand),
    PlanResolve(PlanResolveCommand),
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
pub struct InputResolveCommand {
    pub conversation_id: String,
    pub client_request_id: String,
    pub request_id: String,
    pub answer: InputAnswer,
    pub expected_state_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InputAnswer {
    FreeText { free_text: String },
    SelectedOptions { selected_option_ids: Vec<String> },
}

impl InputAnswer {
    fn to_json(&self) -> serde_json::Value {
        match self {
            Self::FreeText { free_text } => serde_json::json!({
                "freeText": free_text
            }),
            Self::SelectedOptions {
                selected_option_ids,
            } => serde_json::json!({
                "selectedOptionIds": selected_option_ids
            }),
        }
    }
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
    pub expected_state_version: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlanResolveCommand {
    pub conversation_id: String,
    pub task_id: String,
    pub plan_id: String,
    pub request_id: String,
    pub client_request_id: String,
    pub plan_revision: i64,
    pub confirmed: bool,
    pub expected_state_version: i64,
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
struct RawInputResolveCommand {
    #[serde(rename = "type")]
    _command_type: String,
    conversation_id: String,
    client_request_id: String,
    request_id: String,
    answer: RawInputAnswer,
    expected_state_version: i64,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawInputAnswer {
    #[serde(default)]
    free_text: Option<String>,
    #[serde(default)]
    selected_option_ids: Option<Vec<String>>,
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
    expected_state_version: i64,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct RawPlanResolveCommand {
    #[serde(rename = "type")]
    _command_type: String,
    conversation_id: String,
    task_id: String,
    plan_id: String,
    request_id: String,
    client_request_id: String,
    plan_revision: i64,
    confirmed: bool,
    expected_state_version: i64,
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
        && value
            .chars()
            .all(|c| !c.is_whitespace() && !c.is_control() && !matches!(c, '/' | '\\' | '?' | '#'));
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

fn parse_input_answer(raw: RawInputAnswer) -> AppResult<InputAnswer> {
    match (raw.free_text, raw.selected_option_ids) {
        (Some(free_text), None) => {
            let trimmed = free_text.trim();
            if trimmed.is_empty() || free_text.chars().count() > INPUT_ANSWER_MAX_CHARS {
                return Err(AppError::BadRequest("Invalid input answer.".to_string()));
            }
            Ok(InputAnswer::FreeText {
                free_text: trimmed.to_string(),
            })
        }
        (None, Some(selected_option_ids)) => {
            if selected_option_ids.is_empty()
                || selected_option_ids.len() > INPUT_SELECTION_MAX_OPTIONS
            {
                return Err(AppError::BadRequest("Invalid input answer.".to_string()));
            }
            let mut seen = HashSet::new();
            let mut normalized = Vec::with_capacity(selected_option_ids.len());
            for option_id in selected_option_ids {
                let option_id = option_id.trim();
                validate_control_identity(option_id, "selectedOptionId")?;
                if !seen.insert(option_id.to_string()) {
                    return Err(AppError::BadRequest("Invalid input answer.".to_string()));
                }
                normalized.push(option_id.to_string());
            }
            Ok(InputAnswer::SelectedOptions {
                selected_option_ids: normalized,
            })
        }
        _ => Err(AppError::BadRequest("Invalid input answer.".to_string())),
    }
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
    let parse_identity = |payload: &serde_json::Map<String, serde_json::Value>,
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
            service_account_id: parse_identity(payload, "serviceAccountId", "serviceAccountId")?,
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
        .ok_or_else(|| {
            AppError::BadRequest("Assistant chat commands require a type.".to_string())
        })?;
    reject_secret_shaped_command(&value)?;

    match command_type {
        "text" => {
            let raw: RawTextChatCommand = serde_json::from_value(value)
                .map_err(|e| AppError::BadRequest(format!("Invalid text chat request: {e}")))?;
            validate_prompt(&raw.prompt, TYPED_CHAT_PROMPT_MAX_CHARS)?;
            validate_control_identity(&raw.client_request_id, "clientRequestId")?;
            if let Some(conversation_id) = raw.conversation_id.as_deref() {
                validate_typed_conversation_id(conversation_id)?;
            }
            Ok(AssistantChatCommand::Text(TextChatCommand {
                prompt: raw.prompt,
                client_request_id: raw.client_request_id,
                conversation_id: raw.conversation_id,
            }))
        }
        "plan.resolve" => {
            let raw: RawPlanResolveCommand = serde_json::from_value(value).map_err(|e| {
                AppError::BadRequest(format!("Invalid plan resolution request: {e}"))
            })?;
            validate_typed_conversation_id(&raw.conversation_id)?;
            validate_control_identity(&raw.task_id, "taskId")?;
            validate_control_identity(&raw.plan_id, "planId")?;
            validate_control_identity(&raw.request_id, "requestId")?;
            validate_control_identity(&raw.client_request_id, "clientRequestId")?;
            validate_positive(raw.plan_revision, "planRevision")?;
            validate_positive(raw.expected_state_version, "expectedStateVersion")?;
            Ok(AssistantChatCommand::PlanResolve(PlanResolveCommand {
                conversation_id: raw.conversation_id,
                task_id: raw.task_id,
                plan_id: raw.plan_id,
                request_id: raw.request_id,
                client_request_id: raw.client_request_id,
                plan_revision: raw.plan_revision,
                confirmed: raw.confirmed,
                expected_state_version: raw.expected_state_version,
            }))
        }
        "input.resolve" => {
            let raw: RawInputResolveCommand = serde_json::from_value(value).map_err(|e| {
                AppError::BadRequest(format!("Invalid input resolution request: {e}"))
            })?;
            validate_typed_conversation_id(&raw.conversation_id)?;
            validate_control_identity(&raw.client_request_id, "clientRequestId")?;
            validate_control_identity(&raw.request_id, "requestId")?;
            validate_positive(raw.expected_state_version, "expectedStateVersion")?;
            let answer = parse_input_answer(raw.answer)?;
            Ok(AssistantChatCommand::InputResolve(InputResolveCommand {
                conversation_id: raw.conversation_id,
                client_request_id: raw.client_request_id,
                request_id: raw.request_id,
                answer,
                expected_state_version: raw.expected_state_version,
            }))
        }
        "action.continue" => {
            let raw: RawActionContinueCommand = serde_json::from_value(value).map_err(|e| {
                AppError::BadRequest(format!("Invalid action continuation request: {e}"))
            })?;
            validate_typed_conversation_id(&raw.conversation_id)?;
            validate_control_identity(&raw.client_request_id, "clientRequestId")?;
            if let Some(origin_turn_id) = raw.origin_turn_id.as_deref() {
                validate_control_identity(origin_turn_id, "originTurnId")?;
            }
            if raw.actions.len() > ACTION_CONTINUATION_MAX_REPORTS {
                return Err(AppError::BadRequest("Too many action reports.".to_string()));
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
                if let Some(origin_turn_id) = raw.origin_turn_id.as_deref()
                    && report.origin_turn_id != origin_turn_id
                {
                    return Err(AppError::BadRequest(
                        "Action report originTurnId must match the continuation origin."
                            .to_string(),
                    ));
                }
                if !seen.insert(report.action_request_id.clone()) {
                    return Err(AppError::BadRequest(
                        "Duplicate actionRequestId in action continuation.".to_string(),
                    ));
                }
                let disposition = ActionDisposition::parse(&report.disposition)?;
                let resource = parse_action_resource(report.resource)?;
                if disposition == ActionDisposition::Completed && resource.is_none() {
                    return Err(AppError::BadRequest(
                        "Completed action reports require a resource reference.".to_string(),
                    ));
                }
                actions.push(ActionReport {
                    action_request_id: report.action_request_id,
                    origin_turn_id: report.origin_turn_id,
                    disposition,
                    resource,
                });
            }
            Ok(AssistantChatCommand::ActionContinue(
                ActionContinueCommand {
                    conversation_id: raw.conversation_id,
                    client_request_id: raw.client_request_id,
                    origin_turn_id: raw.origin_turn_id,
                    actions,
                },
            ))
        }
        "approval.resolve" => {
            let raw: RawApprovalResolveCommand = serde_json::from_value(value).map_err(|e| {
                AppError::BadRequest(format!("Invalid approval resolution request: {e}"))
            })?;
            validate_typed_conversation_id(&raw.conversation_id)?;
            validate_control_identity(&raw.client_request_id, "clientRequestId")?;
            validate_control_identity(&raw.request_id, "requestId")?;
            validate_positive(raw.expected_state_version, "expectedStateVersion")?;
            let reason = normalize_reason(raw.reason)?;
            Ok(AssistantChatCommand::ApprovalResolve(
                ApprovalResolveCommand {
                    conversation_id: raw.conversation_id,
                    client_request_id: raw.client_request_id,
                    request_id: raw.request_id,
                    approved: raw.approved,
                    reason,
                    expected_state_version: raw.expected_state_version,
                },
            ))
        }
        "task.stop" => {
            let raw: RawTaskStopCommand = serde_json::from_value(value)
                .map_err(|e| AppError::BadRequest(format!("Invalid stop request: {e}")))?;
            validate_typed_conversation_id(&raw.conversation_id)?;
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
            let raw: RawTaskSteerCommand = serde_json::from_value(value)
                .map_err(|e| AppError::BadRequest(format!("Invalid steering request: {e}")))?;
            validate_typed_conversation_id(&raw.conversation_id)?;
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
            let raw: RawStepRetryCommand = serde_json::from_value(value)
                .map_err(|e| AppError::BadRequest(format!("Invalid retry request: {e}")))?;
            validate_typed_conversation_id(&raw.conversation_id)?;
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
            let raw: RawStepSkipCommand = serde_json::from_value(value)
                .map_err(|e| AppError::BadRequest(format!("Invalid skip request: {e}")))?;
            validate_typed_conversation_id(&raw.conversation_id)?;
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
                validate_typed_conversation_id(conversation_id)?;
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
        AssistantChatCommand::InputResolve(command) => Ok(PreparedAssistantChatCommand {
            body: serde_json::json!({
                "type": "input.resolve",
                "conversationId": command.conversation_id,
                "clientRequestId": command.client_request_id,
                "requestId": command.request_id,
                "answer": command.answer.to_json(),
                "expectedStateVersion": command.expected_state_version,
            }),
            client_request_id: command.client_request_id.clone(),
            response_kind: AssistantChatResponseKind::Json,
        }),
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
            body.insert(
                "approved".to_string(),
                serde_json::Value::Bool(command.approved),
            );
            if let Some(reason) = &command.reason {
                body.insert(
                    "reason".to_string(),
                    serde_json::Value::String(reason.clone()),
                );
            }
            body.insert(
                "expectedStateVersion".to_string(),
                serde_json::Value::Number(command.expected_state_version.into()),
            );
            Ok(PreparedAssistantChatCommand {
                body: serde_json::Value::Object(body),
                client_request_id: command.client_request_id.clone(),
                response_kind: AssistantChatResponseKind::Json,
            })
        }
        AssistantChatCommand::PlanResolve(command) => Ok(PreparedAssistantChatCommand {
            body: serde_json::json!({
                "type": "plan.resolve",
                "conversationId": command.conversation_id,
                "taskId": command.task_id,
                "planId": command.plan_id,
                "requestId": command.request_id,
                "clientRequestId": command.client_request_id,
                "planRevision": command.plan_revision,
                "confirmed": command.confirmed,
                "expectedStateVersion": command.expected_state_version,
            }),
            client_request_id: command.client_request_id.clone(),
            response_kind: AssistantChatResponseKind::Json,
        }),
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

    macro_rules! command_contract_test {
        ($name:ident, $variant:ident, $body:expr, $response_kind:expr) => {
            #[test]
            fn $name() {
                let body = $body;
                let command = parse_command(body.clone());
                assert!(matches!(&command, AssistantChatCommand::$variant(_)));
                let prepared = prepare_assistant_chat_command(&command).unwrap();
                assert_eq!(prepared.body, body);
                assert_eq!(prepared.response_kind, $response_kind);
            }
        };
    }

    command_contract_test!(
        text_command_matches_the_typed_contract,
        Text,
        json!({
            "type": "text",
            "prompt": "show github repos",
            "clientRequestId": "client-text-1"
        }),
        AssistantChatResponseKind::Stream
    );

    command_contract_test!(
        plan_resolve_command_matches_the_typed_contract,
        PlanResolve,
        json!({
            "type": "plan.resolve",
            "conversationId": CONV,
            "taskId": "task-alpha",
            "planId": "plan-alpha",
            "requestId": "plan-gate-alpha",
            "clientRequestId": "client-plan-alpha",
            "planRevision": 3,
            "confirmed": true,
            "expectedStateVersion": 23
        }),
        AssistantChatResponseKind::Json
    );

    command_contract_test!(
        input_resolve_command_matches_the_typed_contract,
        InputResolve,
        json!({
            "type": "input.resolve",
            "conversationId": CONV,
            "clientRequestId": "client-input-1",
            "requestId": "input-1",
            "answer": { "freeText": "Singapore" },
            "expectedStateVersion": 19
        }),
        AssistantChatResponseKind::Json
    );

    command_contract_test!(
        action_continue_command_matches_the_typed_contract,
        ActionContinue,
        json!({
            "type": "action.continue",
            "conversationId": CONV,
            "clientRequestId": "client-action-1",
            "actions": []
        }),
        AssistantChatResponseKind::Stream
    );

    command_contract_test!(
        approval_resolve_command_matches_the_typed_contract,
        ApprovalResolve,
        json!({
            "type": "approval.resolve",
            "conversationId": CONV,
            "clientRequestId": "client-approval-1",
            "requestId": "approval-1",
            "approved": false,
            "expectedStateVersion": 21
        }),
        AssistantChatResponseKind::Json
    );

    command_contract_test!(
        task_stop_command_matches_the_typed_contract,
        TaskStop,
        json!({
            "type": "task.stop",
            "conversationId": CONV,
            "turnId": "turn-1",
            "stopRequestId": "stop-1",
            "clientRequestId": "client-stop-1",
            "expectedStateVersion": 0
        }),
        AssistantChatResponseKind::Json
    );

    command_contract_test!(
        task_steer_command_matches_the_typed_contract,
        TaskSteer,
        json!({
            "type": "task.steer",
            "conversationId": CONV,
            "turnId": "turn-1",
            "steeringId": "steer-1",
            "clientRequestId": "client-steer-1",
            "instruction": "Try again",
            "expectedStateVersion": 2
        }),
        AssistantChatResponseKind::Json
    );

    command_contract_test!(
        step_retry_command_matches_the_typed_contract,
        StepRetry,
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
        }),
        AssistantChatResponseKind::Json
    );

    command_contract_test!(
        step_skip_command_matches_the_typed_contract,
        StepSkip,
        json!({
            "type": "step.skip",
            "conversationId": CONV,
            "turnId": "turn-1",
            "taskId": "task-1",
            "stepId": "step-2",
            "skipRequestId": "skip-1",
            "clientRequestId": "client-skip-1",
            "expectedOperationGeneration": 4,
            "expectedStateVersion": 5
        }),
        AssistantChatResponseKind::Json
    );

    #[test]
    fn drains_each_index_to_its_authoritative_family_only() {
        let index = json!({
            "conversations": [
                { "id": WORKFLOW_CONV, "updatedAt": "2026-07-29T12:00:00.000Z" },
                { "id": CONV, "updatedAt": "2026-07-29T13:00:00.000Z" },
                { "id": "voicec-1", "updatedAt": "2026-07-29T14:00:00.000Z" },
                { "title": "unknown-shaped row" }
            ],
            "nextCursor": "page-2"
        });
        let mut conversations = Vec::new();
        let mut seen = HashSet::new();

        assert_eq!(
            append_conversation_family_page(
                &index,
                ConversationResourceFamily::Typed,
                &mut conversations,
                &mut seen,
            )
            .unwrap(),
            Some("page-2".to_string())
        );
        assert_eq!(
            conversations
                .iter()
                .map(|row| row.get("id").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>(),
            vec![Some(CONV)]
        );

        append_conversation_family_page(
            &index,
            ConversationResourceFamily::Legacy,
            &mut conversations,
            &mut seen,
        )
        .unwrap();
        assert_eq!(
            conversations
                .iter()
                .map(|row| row.get("id").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>(),
            vec![Some(CONV), Some(WORKFLOW_CONV)]
        );
    }

    #[test]
    fn drained_index_dedupes_and_sorts_newest_first() {
        let page = json!({
            "conversations": [
                {
                    "id": WORKFLOW_CONV,
                    "title": "workflow",
                    "updatedAt": "2026-07-29T14:00:00.000Z"
                },
                {
                    "id": CONV,
                    "title": "typed",
                    "updatedAt": "2026-07-29T13:00:00.000Z"
                },
                {
                    "id": CONV,
                    "title": "duplicate",
                    "updatedAt": "2026-07-29T15:00:00.000Z"
                }
            ]
        });
        let mut conversations = Vec::new();
        let mut seen = HashSet::new();

        append_conversation_family_page(
            &page,
            ConversationResourceFamily::Typed,
            &mut conversations,
            &mut seen,
        )
        .unwrap();
        append_conversation_family_page(
            &page,
            ConversationResourceFamily::Legacy,
            &mut conversations,
            &mut seen,
        )
        .unwrap();
        sort_conversation_rows_newest_first(&mut conversations);
        assert_eq!(
            conversations
                .iter()
                .map(|row| row["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![WORKFLOW_CONV, CONV]
        );
        assert_eq!(
            conversations[1]["title"].as_str(),
            Some("typed"),
            "the first occurrence must win the dedupe"
        );
    }

    #[test]
    fn merge_sorts_rfc3339_offsets_across_updated_and_created_keys() {
        let mut conversations = vec![
            json!({
                "id": CONV,
                "title": "typed",
                "createdAt": "2026-07-29T05:30:00.000Z"
            }),
            json!({
                "id": WORKFLOW_CONV,
                "title": "workflow",
                "updatedAt": "2026-07-29T13:00:00.000+08:00"
            }),
        ];

        sort_conversation_rows_newest_first(&mut conversations);
        assert_eq!(
            conversations
                .iter()
                .map(|row| row["id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec![CONV, WORKFLOW_CONV],
            "RFC3339 offsets and mixed timestamp keys must sort chronologically"
        );
    }

    #[test]
    fn builds_family_aware_resource_paths() {
        assert_eq!(
            history_index_path(USER),
            format!("api/scopes/{USER}/chat-history")
        );
        assert_eq!(
            canonical_conversation_path(CONV).unwrap(),
            format!("api/chat/conversations/{CONV}")
        );
        assert_eq!(
            canonical_state_path(CONV).unwrap(),
            format!("api/chat/conversations/{CONV}/state")
        );
        assert_eq!(
            history_conversation_path(USER, WORKFLOW_CONV).unwrap(),
            format!("api/scopes/{USER}/chat-history/conversations/{WORKFLOW_CONV}")
        );
        assert_eq!(
            conversation_resource_family(CONV).unwrap(),
            ConversationResourceFamily::Typed
        );
        assert_eq!(
            conversation_resource_family(WORKFLOW_CONV).unwrap(),
            ConversationResourceFamily::Legacy
        );
        assert_eq!(typed_chat_path(), "api/chat");
        assert_eq!(completions_path(), "v1/chat/completions");
    }

    #[test]
    fn unknown_conversation_families_are_not_found_shaped() {
        assert!(matches!(
            conversation_resource_family("workflow-pending-123"),
            Err(AppError::NotFound(_))
        ));
    }

    #[test]
    fn typed_conversation_ids_require_the_exact_upstream_actor_shape() {
        for malformed in [
            "nyxid-chat-",
            "nyxid-chat-short",
            "nyxid-chat-4a1e60ebd1fd44f192bf4bb90e1812ag",
            "nyxid-chat-4A1E60EBD1FD44F192BF4BB90E1812AE",
            "nyxid-chat-4a1e60eb-d1fd-44f1-92bf-4bb90e1812ae",
        ] {
            assert!(matches!(
                validate_typed_conversation_id(malformed),
                Err(AppError::BadRequest(_))
            ));
            assert!(matches!(
                conversation_resource_family(malformed),
                Err(AppError::BadRequest(_))
            ));
            assert!(canonical_conversation_path(malformed).is_err());
            assert!(canonical_state_path(malformed).is_err());
        }

        assert_eq!(
            conversation_resource_family(CONV).unwrap(),
            ConversationResourceFamily::Typed
        );
        assert_eq!(
            conversation_resource_family(WORKFLOW_CONV).unwrap(),
            ConversationResourceFamily::Legacy
        );
        assert!(history_conversation_path(USER, WORKFLOW_CONV).is_ok());
    }

    #[test]
    fn plan_resolve_requires_an_explicit_confirmed_decision() {
        let error = parse_assistant_chat_command(
            &serde_json::to_vec(&json!({
                "type": "plan.resolve",
                "conversationId": CONV,
                "taskId": "task-alpha",
                "planId": "plan-alpha",
                "requestId": "plan-gate-alpha",
                "clientRequestId": "client-plan-alpha",
                "planRevision": 3,
                "expectedStateVersion": 23
            }))
            .unwrap(),
        )
        .unwrap_err();

        assert_eq!(
            axum::response::IntoResponse::into_response(error).status(),
            axum::http::StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn legacy_conversation_ids_are_rejected_by_every_typed_verb() {
        let mut commands = vec![
            json!({
                "type": "text",
                "prompt": "continue",
                "clientRequestId": "client-text-1",
                "conversationId": WORKFLOW_CONV
            }),
            json!({
                "type": "plan.resolve",
                "conversationId": WORKFLOW_CONV,
                "taskId": "task-1",
                "planId": "plan-1",
                "requestId": "gate-1",
                "clientRequestId": "client-plan-1",
                "planRevision": 1,
                "confirmed": true,
                "expectedStateVersion": 1
            }),
            json!({
                "type": "input.resolve",
                "conversationId": WORKFLOW_CONV,
                "clientRequestId": "client-input-1",
                "requestId": "input-1",
                "answer": { "freeText": "answer" },
                "expectedStateVersion": 1
            }),
            json!({
                "type": "action.continue",
                "conversationId": WORKFLOW_CONV,
                "clientRequestId": "client-action-1",
                "actions": []
            }),
            json!({
                "type": "approval.resolve",
                "conversationId": WORKFLOW_CONV,
                "clientRequestId": "client-approval-1",
                "requestId": "approval-1",
                "approved": true,
                "expectedStateVersion": 1
            }),
            json!({
                "type": "task.stop",
                "conversationId": WORKFLOW_CONV,
                "turnId": "turn-1",
                "stopRequestId": "stop-1",
                "clientRequestId": "client-stop-1",
                "expectedStateVersion": 0
            }),
            json!({
                "type": "task.steer",
                "conversationId": WORKFLOW_CONV,
                "turnId": "turn-1",
                "steeringId": "steer-1",
                "clientRequestId": "client-steer-1",
                "instruction": "Try again",
                "expectedStateVersion": 0
            }),
            json!({
                "type": "step.retry",
                "conversationId": WORKFLOW_CONV,
                "turnId": "turn-1",
                "taskId": "task-1",
                "stepId": "step-1",
                "retryRequestId": "retry-1",
                "clientRequestId": "client-retry-1",
                "expectedOperationGeneration": 1,
                "expectedStateVersion": 0
            }),
            json!({
                "type": "step.skip",
                "conversationId": WORKFLOW_CONV,
                "turnId": "turn-1",
                "taskId": "task-1",
                "stepId": "step-1",
                "skipRequestId": "skip-1",
                "clientRequestId": "client-skip-1",
                "expectedOperationGeneration": 1,
                "expectedStateVersion": 0
            }),
        ];

        for command in commands.drain(..) {
            let command_type = command["type"].as_str().unwrap().to_string();
            let error = parse_assistant_chat_command(&serde_json::to_vec(&command).unwrap())
                .expect_err(&format!(
                    "{command_type} must reject a legacy conversation id"
                ));
            assert_eq!(
                axum::response::IntoResponse::into_response(error).status(),
                axum::http::StatusCode::BAD_REQUEST,
                "{command_type}"
            );
        }
    }

    #[test]
    fn unknown_typed_command_is_bad_request() {
        let error = parse_assistant_chat_command(
            &serde_json::to_vec(&json!({
                "type": "workflow.studio",
                "prompt": "do not fall through"
            }))
            .unwrap(),
        )
        .unwrap_err();

        assert_eq!(
            axum::response::IntoResponse::into_response(error).status(),
            axum::http::StatusCode::BAD_REQUEST
        );
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
            assert!(history_conversation_path(USER, bad).is_err());
        }
    }

    #[test]
    fn migration_guard_keeps_scoped_typed_commands_and_per_conversation_commands_out() {
        const SOURCE: &str = include_str!("assistant_service.rs");
        const TEST_MODULE_MARKER: &str = "#[cfg(test)]\nmod tests";
        let production_source = SOURCE
            .split_once(TEST_MODULE_MARKER)
            .map_or(SOURCE, |(production, _)| production);
        let scoped_prefix = ["nyxid", "-chat/"].concat();
        assert!(
            !production_source.contains(&scoped_prefix),
            "scoped typed command paths must not return"
        );
        let typed_detail = match conversation_resource_family(CONV).unwrap() {
            ConversationResourceFamily::Typed => canonical_conversation_path(CONV).unwrap(),
            ConversationResourceFamily::Legacy => history_conversation_path(USER, CONV).unwrap(),
        };
        assert!(
            !typed_detail.contains("/chat-history/conversations"),
            "typed conversations must remain on the canonical resource family"
        );
        for suffix in [":stream", "/approve", "/stop", "/steer", "/retry", "/skip"] {
            let has_forbidden_interpolation = production_source
                .match_indices("conversations/{")
                .any(|(start, fragment)| {
                    let interpolation_tail = &production_source[start + fragment.len()..];
                    interpolation_tail
                        .find('}')
                        .is_some_and(|end| interpolation_tail[end + 1..].starts_with(suffix))
                });
            assert!(
                !has_forbidden_interpolation,
                "per-conversation command route {suffix} must not return"
            );
        }
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
    fn parses_and_rebuilds_both_closed_input_answer_variants() {
        let selected = prepare_assistant_chat_command(&parse_command(json!({
            "type": "input.resolve",
            "conversationId": CONV,
            "clientRequestId": "client-input-1",
            "requestId": "input-1",
            "answer": {
                "selectedOptionIds": [" option-a ", "option-b"]
            },
            "expectedStateVersion": 19
        })))
        .unwrap();
        let free_text = prepare_assistant_chat_command(&parse_command(json!({
            "type": "input.resolve",
            "conversationId": CONV,
            "clientRequestId": "client-input-2",
            "requestId": "input-2",
            "answer": {
                "freeText": "  Singapore  "
            },
            "expectedStateVersion": 20
        })))
        .unwrap();

        assert_eq!(
            selected,
            PreparedAssistantChatCommand {
                body: json!({
                    "type": "input.resolve",
                    "conversationId": CONV,
                    "clientRequestId": "client-input-1",
                    "requestId": "input-1",
                    "answer": {
                        "selectedOptionIds": ["option-a", "option-b"]
                    },
                    "expectedStateVersion": 19
                }),
                client_request_id: "client-input-1".to_string(),
                response_kind: AssistantChatResponseKind::Json,
            }
        );
        assert_eq!(
            free_text.body,
            json!({
                "type": "input.resolve",
                "conversationId": CONV,
                "clientRequestId": "client-input-2",
                "requestId": "input-2",
                "answer": {
                    "freeText": "Singapore"
                },
                "expectedStateVersion": 20
            })
        );
        assert_eq!(
            free_text.response_kind.accept_header_value(),
            "application/json"
        );
    }

    #[test]
    fn rejects_invalid_input_resolutions_fail_closed() {
        let command = |answer: serde_json::Value, expected_state_version: i64| {
            json!({
                "type": "input.resolve",
                "conversationId": CONV,
                "clientRequestId": "client-input-1",
                "requestId": "input-1",
                "answer": answer,
                "expectedStateVersion": expected_state_version
            })
        };
        let mut unknown_root = command(json!({ "freeText": "answer" }), 19);
        unknown_root
            .as_object_mut()
            .unwrap()
            .insert("scopeId".to_string(), json!("someone-else"));
        let values = vec![
            json!({
                "type": "input.resolve",
                "conversationId": CONV,
                "clientRequestId": "client-input-1",
                "requestId": "input-1",
                "answer": { "freeText": "answer" }
            }),
            command(json!({ "freeText": "answer" }), 0),
            command(json!({}), 19),
            command(
                json!({
                    "freeText": "answer",
                    "selectedOptionIds": ["option-a"]
                }),
                19,
            ),
            command(json!({ "freeText": "   " }), 19),
            command(
                json!({ "freeText": "x".repeat(INPUT_ANSWER_MAX_CHARS + 1) }),
                19,
            ),
            command(json!({ "selectedOptionIds": [] }), 19),
            command(
                json!({
                    "selectedOptionIds": [
                        "option-a", "option-b", "option-c", "option-d",
                        "option-e", "option-f", "option-g"
                    ]
                }),
                19,
            ),
            command(
                json!({ "selectedOptionIds": ["option-a", " option-a "] }),
                19,
            ),
            command(json!({ "selectedOptionIds": ["bad/id"] }), 19),
            command(json!({ "freeText": "answer", "label": "extra" }), 19),
            unknown_root,
        ];
        for value in values {
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
    fn completed_action_reports_round_trip_each_safe_resource_variant() {
        for (index, resource) in [
            json!({ "userService": { "userServiceId": "service-1" } }),
            json!({ "key": { "keyId": "key-1" } }),
            json!({ "node": { "nodeId": "node-1" } }),
            json!({ "serviceAccount": { "serviceAccountId": "sa-1" } }),
            json!({ "developerApp": { "clientId": "app-1" } }),
            json!({ "device": { "deviceId": "device-1" } }),
        ]
        .into_iter()
        .enumerate()
        {
            let action_request_id = format!("act-{index}");
            let command = parse_command(json!({
                "type": "action.continue",
                "conversationId": CONV,
                "clientRequestId": format!("client-{index}"),
                "originTurnId": "turn-1",
                "actions": [{
                    "actionRequestId": action_request_id,
                    "originTurnId": "turn-1",
                    "disposition": "completed",
                    "resource": resource.clone()
                }]
            }));

            assert_eq!(
                prepare_assistant_chat_command(&command).unwrap().body["actions"][0]["resource"],
                resource
            );
        }
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
            json!({
                "type": "action.continue",
                "conversationId": CONV,
                "clientRequestId": "request-1",
                "originTurnId": "turn-1",
                "actions": [{
                    "actionRequestId": "act-1",
                    "originTurnId": "turn-1",
                    "disposition": "completed",
                    "resource": {
                        "key": { "keyId": "key-1" },
                        "node": { "nodeId": "node-1" }
                    }
                }]
            }),
            json!({
                "type": "action.continue",
                "conversationId": CONV,
                "clientRequestId": "request-1",
                "originTurnId": "turn-1",
                "actions": [{
                    "actionRequestId": "act-1",
                    "originTurnId": "turn-1",
                    "disposition": "completed",
                    "resource": {
                        "key": { "keyId": "key-1", "label": "extra" }
                    }
                }]
            }),
            json!({
                "type": "action.continue",
                "conversationId": CONV,
                "clientRequestId": "request-1",
                "originTurnId": "turn-1",
                "actions": [{
                    "actionRequestId": "act-1",
                    "originTurnId": "turn-1",
                    "disposition": "completed",
                    "resource": {
                        "workspace": { "workspaceId": "workspace-1" }
                    }
                }]
            }),
            json!({
                "type": "action.continue",
                "conversationId": CONV,
                "clientRequestId": "request-1",
                "originTurnId": "turn-1",
                "actions": [{
                    "actionRequestId": "act-1",
                    "originTurnId": "turn-1",
                    "disposition": "completed",
                    "resource": {
                        "key": { "nodeId": "node-1" }
                    }
                }]
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
            "reason": "Bearer eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9",
            "expectedStateVersion": 21
        });

        assert!(parse_assistant_chat_command(&serde_json::to_vec(&value).unwrap()).is_err());
    }

    #[test]
    fn approval_resolution_requires_a_positive_observed_state_version() {
        for value in [
            json!({
                "type": "approval.resolve",
                "conversationId": CONV,
                "clientRequestId": "request-approval-1",
                "requestId": "approval-1",
                "approved": true
            }),
            json!({
                "type": "approval.resolve",
                "conversationId": CONV,
                "clientRequestId": "request-approval-1",
                "requestId": "approval-1",
                "approved": true,
                "expectedStateVersion": 0
            }),
            json!({
                "type": "approval.resolve",
                "conversationId": CONV,
                "clientRequestId": "request-approval-1",
                "requestId": "approval-1",
                "approved": true,
                "expectedStateVersion": 21,
                "stateVersion": 21
            }),
        ] {
            assert!(parse_assistant_chat_command(&serde_json::to_vec(&value).unwrap()).is_err());
        }
    }

    #[test]
    fn parses_and_rebuilds_exact_plan_resolution() {
        let prepared = prepare_assistant_chat_command(&parse_command(json!({
            "type": "plan.resolve",
            "conversationId": CONV,
            "taskId": "task-alpha",
            "planId": "plan-alpha",
            "requestId": "plan-gate-alpha",
            "clientRequestId": "client-plan-alpha",
            "planRevision": 3,
            "confirmed": true,
            "expectedStateVersion": 23
        })))
        .unwrap();

        assert_eq!(prepared.response_kind, AssistantChatResponseKind::Json);
        assert_eq!(prepared.client_request_id, "client-plan-alpha");
        assert_eq!(
            prepared.body,
            json!({
                "type": "plan.resolve",
                "conversationId": CONV,
                "taskId": "task-alpha",
                "planId": "plan-alpha",
                "requestId": "plan-gate-alpha",
                "clientRequestId": "client-plan-alpha",
                "planRevision": 3,
                "confirmed": true,
                "expectedStateVersion": 23
            })
        );
    }

    #[test]
    fn plan_resolution_requires_exact_positive_identities_and_versions() {
        for value in [
            json!({
                "type": "plan.resolve",
                "conversationId": CONV,
                "taskId": "task-alpha",
                "planId": "plan-alpha",
                "requestId": "plan-gate-alpha",
                "clientRequestId": "client-plan-alpha",
                "planRevision": 0,
                "confirmed": true,
                "expectedStateVersion": 23
            }),
            json!({
                "type": "plan.resolve",
                "conversationId": CONV,
                "taskId": "task-alpha",
                "planId": "bad/plan",
                "requestId": "plan-gate-alpha",
                "clientRequestId": "client-plan-alpha",
                "planRevision": 3,
                "confirmed": false,
                "expectedStateVersion": 23
            }),
            json!({
                "type": "plan.resolve",
                "conversationId": CONV,
                "taskId": "task-alpha",
                "planId": "plan-alpha",
                "requestId": "plan-gate-alpha",
                "clientRequestId": "client-plan-alpha",
                "planRevision": 3,
                "confirmed": true,
                "expectedStateVersion": 0
            }),
            json!({
                "type": "plan.resolve",
                "conversationId": CONV,
                "taskId": "task-alpha",
                "planId": "plan-alpha",
                "requestId": "plan-gate-alpha",
                "clientRequestId": "client-plan-alpha",
                "planRevision": 3,
                "confirmed": true,
                "expectedStateVersion": 23,
                "stateVersion": 23
            }),
        ] {
            assert!(parse_assistant_chat_command(&serde_json::to_vec(&value).unwrap()).is_err());
        }
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
                "type": "input.resolve",
                "conversationId": CONV,
                "clientRequestId": "client-input-1",
                "requestId": "input-1",
                "answer": { "freeText": "nyxid_secret_abcdefgh" },
                "expectedStateVersion": 19
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
            "reason": "Approved by user",
            "expectedStateVersion": 21
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

        assert_eq!(approval.response_kind, AssistantChatResponseKind::Json);
        assert_eq!(
            approval.response_kind.accept_header_value(),
            "application/json"
        );
        assert_eq!(
            approval.body,
            json!({
                "type": "approval.resolve",
                "conversationId": CONV,
                "clientRequestId": "request-approval-1",
                "requestId": "approval-1",
                "approved": true,
                "reason": "Approved by user",
                "expectedStateVersion": 21
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
        assert_eq!(
            retry.response_kind.accept_header_value(),
            "application/json"
        );
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
}
