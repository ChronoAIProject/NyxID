//! Restricted synthetic ingress for production channel inspections.
//!
//! The caller selects only a server-configured case. Destination and routing
//! identifiers are loaded from the allowlist and never accepted in the body or
//! returned by the API. The synthetic inbound message grants reply authority
//! only; it never mints owner access authority.

use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::Deserialize;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::api_key::ApiKey;
use crate::models::channel_bot::{COLLECTION_NAME as CHANNEL_BOTS, ChannelBot};
use crate::models::channel_conversation::{
    COLLECTION_NAME as CHANNEL_CONVERSATIONS, ChannelConversation,
};
use crate::models::channel_inspection_dispatch::{COLLECTION_NAME, ChannelInspectionDispatch};
use crate::models::channel_message::{
    COLLECTION_NAME as CHANNEL_MESSAGES, CallbackDisposition, ChannelMessage,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};

use super::channel_inspection_auth::InspectionAuth;
use super::channel_platform::InboundMessage;
use super::{audit_service, channel_relay_service, channel_routing_service};

pub const PROTOCOL_VERSION: u8 = 1;
pub const REQUIRED_CASE_COUNT: usize = 10;
pub const REQUEST_BODY_LIMIT: usize = 4096;
pub const REPLY_DEADLINE_SECS: i64 = 180;
const RETENTION_DAYS: i64 = 7;
const MAX_CASES_JSON_BYTES: usize = 32 * 1024;
const FIXED_MESSAGE: &str = "NyxID production inspection. Reply with exactly: inspection ok";
const SYNTHETIC_SENDER: &str = "nyxid-production-inspection";

#[derive(Clone, Copy)]
struct CaseContract {
    platform: &'static str,
    conversation_type: &'static str,
    organization_owned: bool,
}

fn case_contract(case_id: &str) -> Option<CaseContract> {
    let contract = match case_id {
        "TG-P-DM-O-01" => CaseContract {
            platform: "telegram",
            conversation_type: "private",
            organization_owned: false,
        },
        "TG-P-GRP-O-01" => CaseContract {
            platform: "telegram",
            conversation_type: "group",
            organization_owned: false,
        },
        "TG-O-DM-O-01" => CaseContract {
            platform: "telegram",
            conversation_type: "private",
            organization_owned: true,
        },
        "TG-O-GRP-O-01" => CaseContract {
            platform: "telegram",
            conversation_type: "group",
            organization_owned: true,
        },
        "LK-O-DM-O-01" => CaseContract {
            platform: "lark",
            conversation_type: "private",
            organization_owned: true,
        },
        "LK-O-GRP-O-01" => CaseContract {
            platform: "lark",
            conversation_type: "group",
            organization_owned: true,
        },
        "DC-P-DM-O-01" => CaseContract {
            platform: "discord",
            conversation_type: "private",
            organization_owned: false,
        },
        "DC-P-GRP-U-01" => CaseContract {
            platform: "discord",
            conversation_type: "group",
            organization_owned: false,
        },
        "DC-O-DM-O-01" => CaseContract {
            platform: "discord",
            conversation_type: "private",
            organization_owned: true,
        },
        "DC-O-GRP-O-01" => CaseContract {
            platform: "discord",
            conversation_type: "group",
            organization_owned: true,
        },
        _ => return None,
    };
    Some(contract)
}

const STATE_PREPARING: &str = "preparing";
const STATE_CALLBACK_DELIVERED: &str = "callback_delivered";
const STATE_CALLBACK_FAILED: &str = "callback_failed";
const STATE_PREPARATION_FAILED: &str = "preparation_failed";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InspectionFixture {
    pub case_id: String,
    pub platform: String,
    pub conversation_id: String,
    pub bot_id: String,
}

impl std::fmt::Debug for InspectionFixture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InspectionFixture")
            .field("case_id", &self.case_id)
            .field("platform", &self.platform)
            .field("routing", &"[REDACTED]")
            .finish()
    }
}

#[derive(Clone)]
pub struct Runtime {
    auth: InspectionAuth,
    fixtures: BTreeMap<String, InspectionFixture>,
}

impl std::fmt::Debug for Runtime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelInspectionRuntime")
            .field("auth", &"[REDACTED]")
            .field("fixture_count", &self.fixtures.len())
            .finish()
    }
}

impl Runtime {
    pub fn from_env(db: mongodb::Database) -> Result<Option<Self>, String> {
        let key = std::env::var("CHANNEL_INSPECTION_HMAC_KEY")
            .ok()
            .filter(|value| !value.trim().is_empty());
        let cases = std::env::var("CHANNEL_INSPECTION_CASES_JSON")
            .ok()
            .filter(|value| !value.trim().is_empty());
        match (key, cases) {
            (None, None) => Ok(None),
            (Some(_), None) | (None, Some(_)) => Err(
                "CHANNEL_INSPECTION_HMAC_KEY and CHANNEL_INSPECTION_CASES_JSON must be configured together"
                    .into(),
            ),
            (Some(key), Some(cases)) => Self::new(db, &key, &cases, true).map(Some),
        }
    }

    fn new(
        db: mongodb::Database,
        key: &str,
        cases_json: &str,
        require_full_inventory: bool,
    ) -> Result<Self, String> {
        if cases_json.len() > MAX_CASES_JSON_BYTES {
            return Err("CHANNEL_INSPECTION_CASES_JSON exceeds 32768 bytes".into());
        }
        let auth = InspectionAuth::from_hex(db, key)
            .map_err(|reason| format!("CHANNEL_INSPECTION_HMAC_KEY {reason}"))?;
        let parsed: Vec<InspectionFixture> = serde_json::from_str(cases_json).map_err(|_| {
            "CHANNEL_INSPECTION_CASES_JSON must be a valid fixture array".to_string()
        })?;
        if require_full_inventory && parsed.len() != REQUIRED_CASE_COUNT {
            return Err(format!(
                "CHANNEL_INSPECTION_CASES_JSON must contain exactly {REQUIRED_CASE_COUNT} cases"
            ));
        }
        if parsed.is_empty() || parsed.len() > REQUIRED_CASE_COUNT {
            return Err(format!(
                "CHANNEL_INSPECTION_CASES_JSON must contain 1-{REQUIRED_CASE_COUNT} cases"
            ));
        }
        let mut fixtures = BTreeMap::new();
        for fixture in parsed {
            validate_fixture(&fixture)?;
            if fixtures.insert(fixture.case_id.clone(), fixture).is_some() {
                return Err("CHANNEL_INSPECTION_CASES_JSON case_id values must be unique".into());
            }
        }
        Ok(Self { auth, fixtures })
    }

    pub async fn authenticate(
        &self,
        headers: &axum::http::HeaderMap,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> bool {
        self.auth.authenticate(headers, method, path, body).await
    }

    fn fixture(&self, case_id: &str) -> Option<&InspectionFixture> {
        self.fixtures.get(case_id)
    }
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DispatchRequest {
    pub protocol_version: u8,
    pub run_id: String,
    pub event_id: String,
    pub timestamp: DateTime<Utc>,
}

impl std::fmt::Debug for DispatchRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchRequest")
            .field("protocol_version", &self.protocol_version)
            .field("run_id", &self.run_id)
            .field("event_id", &"[REDACTED]")
            .field("timestamp", &self.timestamp)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct DispatchAccepted {
    pub dispatch_id: String,
    pub case_id: String,
    pub run_id: String,
    pub idempotent_replay: bool,
}

#[derive(Clone, Debug)]
pub struct InspectionStatus {
    pub dispatch_id: String,
    pub case_id: String,
    pub run_id: String,
    pub phase: &'static str,
    pub terminal: bool,
    pub evidence_complete: bool,
    pub outcome: Option<&'static str>,
    pub callback_admitted: Option<bool>,
    pub correlated_outbound_count: u8,
    pub provider_accepted: bool,
    pub latency_ms: Option<u64>,
    pub deadline_at: DateTime<Utc>,
}

fn validate_fixture(fixture: &InspectionFixture) -> Result<(), String> {
    if !valid_label(&fixture.case_id, 64) {
        return Err(
            "inspection case_id must contain only ASCII letters, digits, underscore, dot, or hyphen"
                .into(),
        );
    }
    let contract = case_contract(&fixture.case_id)
        .ok_or_else(|| "inspection case_id is not in the exact active matrix".to_string())?;
    if super::channel_platform::canonical_platform(&fixture.platform) != contract.platform {
        return Err("inspection fixture platform does not match its case_id".into());
    }
    if uuid::Uuid::parse_str(&fixture.conversation_id).is_err()
        || uuid::Uuid::parse_str(&fixture.bot_id).is_err()
    {
        return Err("inspection conversation_id and bot_id must be UUIDs".into());
    }
    Ok(())
}

fn validate_request(request: &DispatchRequest) -> AppResult<()> {
    if request.protocol_version != PROTOCOL_VERSION {
        return Err(AppError::ValidationError(
            "unsupported inspection protocol_version".into(),
        ));
    }
    if !valid_label(&request.run_id, 96) {
        return Err(AppError::ValidationError(
            "run_id must be 1-96 safe ASCII characters".into(),
        ));
    }
    if uuid::Uuid::parse_str(&request.event_id).is_err() {
        return Err(AppError::ValidationError("event_id must be a UUID".into()));
    }
    if Utc::now()
        .signed_duration_since(request.timestamp)
        .num_seconds()
        .unsigned_abs()
        > 300
    {
        return Err(AppError::ValidationError(
            "inspection request timestamp is outside the accepted window".into(),
        ));
    }
    Ok(())
}

fn valid_label(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':' | b'+')
        })
}

async fn load_live_fixture(
    state: &AppState,
    fixture: &InspectionFixture,
) -> AppResult<(ChannelConversation, ChannelBot, ApiKey, String)> {
    let contract = case_contract(&fixture.case_id).ok_or_else(fixture_unavailable)?;
    let conversation = state
        .db
        .collection::<ChannelConversation>(CHANNEL_CONVERSATIONS)
        .find_one(doc! {
            "_id": &fixture.conversation_id,
            "channel_bot_id": &fixture.bot_id,
            "platform": &fixture.platform,
            "platform_conversation_type": contract.conversation_type,
            "is_active": true,
            "retired_by_transfer": {"$ne": true},
        })
        .await?
        .filter(|conversation| {
            conversation.platform_sender_id.is_none()
                && super::channel_send_service::is_concrete_platform_address(
                    &conversation.platform_conversation_id,
                )
        })
        .ok_or_else(fixture_unavailable)?;
    let owner = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! {
            "_id": &conversation.user_id,
            "is_active": true,
        })
        .await?
        .ok_or_else(fixture_unavailable)?;
    if owner.user_type.is_org() != contract.organization_owned {
        return Err(fixture_unavailable());
    }
    let bot = state
        .db
        .collection::<ChannelBot>(CHANNEL_BOTS)
        .find_one(doc! {
            "_id": &fixture.bot_id,
            "user_id": &conversation.user_id,
            "platform": &fixture.platform,
            "is_active": true,
            "status": "active",
        })
        .await?
        .ok_or_else(fixture_unavailable)?;
    let route = channel_routing_service::AgentRoute {
        callback_url: String::new(),
        api_key_id: conversation.agent_api_key_id.clone(),
        conversation: conversation.clone(),
    };
    let key = state
        .db
        .collection::<ApiKey>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {
            "_id": &conversation.agent_api_key_id,
            "user_id": &conversation.user_id,
            "is_active": true,
        })
        .await?
        .ok_or_else(fixture_unavailable)?;
    channel_routing_service::ensure_route_agent(&key, &conversation.user_id)
        .map_err(|_| fixture_unavailable())?;
    let callback_url = key
        .callback_url
        .clone()
        .filter(|url| !url.trim().is_empty())
        .ok_or_else(fixture_unavailable)?;
    let route = channel_routing_service::AgentRoute {
        callback_url: callback_url.clone(),
        ..route
    };
    channel_routing_service::load_callback_key(&state.db, &route, &conversation.user_id)
        .await
        .map_err(|_| fixture_unavailable())?;
    super::ownership_transfer_service::require_current_bot(&state.db, &bot)
        .await
        .map_err(|_| fixture_unavailable())?;
    Ok((conversation, bot, key, callback_url))
}

fn fixture_unavailable() -> AppError {
    AppError::NotFound("inspection case is unavailable".into())
}

pub async fn dispatch(
    state: &AppState,
    runtime: &Runtime,
    case_id: &str,
    request: &DispatchRequest,
) -> AppResult<DispatchAccepted> {
    validate_request(request)?;
    let fixture = runtime.fixture(case_id).ok_or_else(fixture_unavailable)?;
    let (conversation, bot, api_key, callback_url) = load_live_fixture(state, fixture).await?;
    let now = Utc::now();
    let dispatch = ChannelInspectionDispatch {
        id: uuid::Uuid::new_v4().to_string(),
        protocol_version: request.protocol_version,
        case_id: case_id.to_string(),
        run_id: request.run_id.clone(),
        event_id: request.event_id.clone(),
        platform: fixture.platform.clone(),
        channel_bot_id: bot.id.clone(),
        conversation_id: conversation.id.clone(),
        agent_api_key_id: api_key.id.clone(),
        inbound_message_id: uuid::Uuid::new_v4().to_string(),
        state: STATE_PREPARING.into(),
        failure_code: None,
        callback_http_status: None,
        created_at: now,
        updated_at: now,
        deadline_at: now + chrono::Duration::seconds(REPLY_DEADLINE_SECS),
        purge_at: now + chrono::Duration::days(RETENTION_DAYS),
    };
    let collection = state
        .db
        .collection::<ChannelInspectionDispatch>(COLLECTION_NAME);
    if let Err(error) = collection.insert_one(&dispatch).await {
        if duplicate_key(&error) {
            let existing = collection
                .find_one(doc! {"run_id": &request.run_id, "case_id": case_id})
                .await?
                .ok_or_else(|| AppError::Conflict("inspection dispatch is being created".into()))?;
            return Ok(DispatchAccepted {
                dispatch_id: existing.id,
                case_id: existing.case_id,
                run_id: existing.run_id,
                idempotent_replay: true,
            });
        }
        return Err(error.into());
    }

    let inbound = InboundMessage {
        platform_message_id: request.event_id.clone(),
        conversation_id: conversation.platform_conversation_id.clone(),
        conversation_type: conversation.platform_conversation_type.clone(),
        sender_platform_id: SYNTHETIC_SENDER.into(),
        sender_display_name: None,
        content_type: "text".into(),
        text: Some(FIXED_MESSAGE.into()),
        attachments: Vec::new(),
        embedded_attachments: Vec::new(),
        embedded_attachments_omitted: 0,
        reply_to_platform_message_id: None,
        thread_id: None,
        raw_data: serde_json::json!({
            "nyxid_inspection": {
                "synthetic": true,
                "protocol_version": PROTOCOL_VERSION,
                "case_id": case_id,
                "run_id": request.run_id,
            }
        }),
    };
    let stored = match channel_relay_service::store_inbound_message_with_id(
        &state.db,
        &bot.id,
        &conversation.id,
        &bot.user_id,
        &bot.platform,
        &inbound,
        &api_key.id,
        &dispatch.inbound_message_id,
    )
    .await
    {
        Ok(message) => message,
        Err(error) => {
            mark_dispatch(
                &state.db,
                &dispatch.id,
                STATE_PREPARATION_FAILED,
                Some("inbound_persistence_failed"),
                None,
            )
            .await;
            return Err(error);
        }
    };

    let reply_token = match crate::crypto::jwt::generate_relay_reply_token(
        &state.jwt_keys,
        &state.config,
        &api_key.id,
        &conversation.id,
        &stored.id,
        &bot.platform,
    ) {
        Ok(token) => token,
        Err(error) => {
            mark_dispatch(
                &state.db,
                &dispatch.id,
                STATE_PREPARATION_FAILED,
                Some("reply_token_unavailable"),
                None,
            )
            .await;
            return Err(error);
        }
    };
    let payload = channel_relay_service::build_callback_payload(
        &stored,
        &conversation,
        &api_key.id,
        &api_key.name,
        &inbound,
        Some(reply_token),
        &state.config.base_url,
    );

    // The inspection is synthetic and never proves a human actor. Deliberately
    // omit X-NyxID-User-Token while retaining the one-use anchored reply token.
    let delivery = channel_relay_service::forward_to_agent(
        &state.http_client,
        &state.config,
        &state.jwt_keys,
        &callback_url,
        payload,
        &api_key.id,
        &api_key.key_hash,
        None,
    )
    .await;
    if let Some(disposition) = &delivery.disposition {
        let _ =
            channel_relay_service::record_callback_disposition(&state.db, &stored.id, disposition)
                .await;
    }
    let callback_ok = delivery.result.is_ok();
    let callback_state = if callback_ok {
        STATE_CALLBACK_DELIVERED
    } else {
        STATE_CALLBACK_FAILED
    };
    let failure_code = (!callback_ok).then_some("callback_failed");
    channel_relay_service::update_callback_status(
        &state.db,
        &stored.id,
        if callback_ok { "delivered" } else { "failed" },
    )
    .await?;
    mark_dispatch(
        &state.db,
        &dispatch.id,
        callback_state,
        failure_code,
        delivery.http_status,
    )
    .await;
    if callback_ok {
        let _ = channel_routing_service::touch_conversation(&state.db, &conversation.id).await;
    }
    audit_service::log_async(
        state.db.clone(),
        None,
        "channel_inspection.dispatched".into(),
        Some(serde_json::json!({
            "dispatch_id": dispatch.id,
            "case_id": case_id,
            "run_id": request.run_id,
            "callback": if callback_ok { "delivered" } else { "failed" },
        })),
        None,
        None,
        None,
        None,
    );

    Ok(DispatchAccepted {
        dispatch_id: dispatch.id,
        case_id: case_id.to_string(),
        run_id: request.run_id.clone(),
        idempotent_replay: false,
    })
}

async fn mark_dispatch(
    db: &mongodb::Database,
    dispatch_id: &str,
    state: &str,
    failure_code: Option<&str>,
    callback_http_status: Option<u16>,
) {
    let mut set = doc! {
        "state": state,
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };
    set.insert(
        "failure_code",
        failure_code.map_or(bson::Bson::Null, |value| value.into()),
    );
    set.insert(
        "callback_http_status",
        callback_http_status.map_or(bson::Bson::Null, |value| i32::from(value).into()),
    );
    if let Err(_error) = db
        .collection::<ChannelInspectionDispatch>(COLLECTION_NAME)
        .update_one(doc! {"_id": dispatch_id}, doc! {"$set": set})
        .await
    {
        tracing::error!("Inspection state update failed");
    }
}

fn duplicate_key(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write))
            if write.code == 11000
    )
}

pub async fn status(
    db: &mongodb::Database,
    runtime: &Runtime,
    dispatch_id: &str,
) -> AppResult<InspectionStatus> {
    if uuid::Uuid::parse_str(dispatch_id).is_err() {
        return Err(AppError::NotFound("inspection dispatch not found".into()));
    }
    let dispatch = db
        .collection::<ChannelInspectionDispatch>(COLLECTION_NAME)
        .find_one(doc! {"_id": dispatch_id})
        .await?
        .ok_or_else(|| AppError::NotFound("inspection dispatch not found".into()))?;
    let fixture_matches = runtime.fixture(&dispatch.case_id).is_some_and(|fixture| {
        fixture.platform == dispatch.platform
            && fixture.bot_id == dispatch.channel_bot_id
            && fixture.conversation_id == dispatch.conversation_id
    });
    if !fixture_matches {
        return Ok(incomplete_status(&dispatch));
    }
    let inbound = db
        .collection::<ChannelMessage>(CHANNEL_MESSAGES)
        .find_one(doc! {
            "_id": &dispatch.inbound_message_id,
            "direction": "inbound",
            "channel_bot_id": &dispatch.channel_bot_id,
            "conversation_id": &dispatch.conversation_id,
            "agent_api_key_id": &dispatch.agent_api_key_id,
            "platform": &dispatch.platform,
        })
        .await?;
    let Some(inbound) = inbound else {
        return Ok(incomplete_status(&dispatch));
    };
    let outbound: Vec<ChannelMessage> = db
        .collection::<ChannelMessage>(CHANNEL_MESSAGES)
        .find(doc! {
            "reply_to_message_id": &dispatch.inbound_message_id,
            "direction": "outbound",
            "channel_bot_id": &dispatch.channel_bot_id,
            "conversation_id": &dispatch.conversation_id,
            "agent_api_key_id": &dispatch.agent_api_key_id,
            "platform": &dispatch.platform,
        })
        .sort(doc! {"created_at": 1})
        .limit(2)
        .await?
        .try_collect()
        .await?;
    let callback_admitted = match inbound.callback_disposition {
        Some(CallbackDisposition::Accepted) => Some(true),
        Some(CallbackDisposition::Refused { .. }) => Some(false),
        Some(CallbackDisposition::Unknown) | None => None,
    };
    let base = |phase: &'static str,
                terminal,
                evidence_complete,
                outcome: Option<&'static str>,
                provider_accepted,
                latency_ms| {
        InspectionStatus {
            dispatch_id: dispatch.id.clone(),
            case_id: dispatch.case_id.clone(),
            run_id: dispatch.run_id.clone(),
            phase,
            terminal,
            evidence_complete,
            outcome,
            callback_admitted,
            correlated_outbound_count: outbound.len().min(2) as u8,
            provider_accepted,
            latency_ms,
            deadline_at: dispatch.deadline_at,
        }
    };
    if dispatch.state == STATE_CALLBACK_FAILED
        || inbound.callback_status.as_deref() == Some("failed")
    {
        return Ok(base(
            "terminal",
            true,
            true,
            Some("callback_failed"),
            false,
            None,
        ));
    }
    if matches!(
        inbound.callback_disposition,
        Some(CallbackDisposition::Refused { .. })
    ) {
        return Ok(base(
            "terminal",
            true,
            true,
            Some("callback_rejected"),
            false,
            None,
        ));
    }
    if outbound.len() > 1 {
        return Ok(base(
            "terminal",
            true,
            true,
            Some("duplicate_reply"),
            outbound.iter().all(provider_receipt),
            first_latency_ms(&dispatch, &outbound),
        ));
    }
    if let Some(message) = outbound.first() {
        let accepted = provider_receipt(message);
        return Ok(base(
            "terminal",
            true,
            true,
            Some(if accepted {
                "ok"
            } else {
                "provider_receipt_missing"
            }),
            accepted,
            first_latency_ms(&dispatch, &outbound),
        ));
    }
    if Utc::now() >= dispatch.deadline_at {
        return Ok(base(
            "terminal",
            true,
            true,
            Some("reply_timeout"),
            false,
            None,
        ));
    }
    Ok(base(
        if dispatch.state == STATE_CALLBACK_DELIVERED {
            "reply_pending"
        } else {
            "callback_pending"
        },
        false,
        false,
        None,
        false,
        None,
    ))
}

fn incomplete_status(dispatch: &ChannelInspectionDispatch) -> InspectionStatus {
    let terminal = dispatch.state == STATE_PREPARATION_FAILED || Utc::now() >= dispatch.deadline_at;
    InspectionStatus {
        dispatch_id: dispatch.id.clone(),
        case_id: dispatch.case_id.clone(),
        run_id: dispatch.run_id.clone(),
        phase: if terminal {
            "inspection_incomplete"
        } else {
            "dispatch_pending"
        },
        terminal,
        evidence_complete: false,
        outcome: terminal.then_some("inspection_incomplete"),
        callback_admitted: None,
        correlated_outbound_count: 0,
        provider_accepted: false,
        latency_ms: None,
        deadline_at: dispatch.deadline_at,
    }
}

fn provider_receipt(message: &ChannelMessage) -> bool {
    message
        .platform_message_id
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
}

fn first_latency_ms(
    dispatch: &ChannelInspectionDispatch,
    outbound: &[ChannelMessage],
) -> Option<u64> {
    let duration = outbound.first()?.created_at - dispatch.created_at;
    u64::try_from(duration.num_milliseconds()).ok()
}

pub async fn ensure_indexes(db: &mongodb::Database) -> Result<(), mongodb::error::Error> {
    use mongodb::IndexModel;
    use mongodb::options::IndexOptions;

    let collection = db.collection::<ChannelInspectionDispatch>(COLLECTION_NAME);
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"run_id": 1, "case_id": 1})
                .options(
                    IndexOptions::builder()
                        .name("channel_inspection_run_case_uniq".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await?;
    collection
        .create_index(
            IndexModel::builder()
                .keys(doc! {"purge_at": 1})
                .options(
                    IndexOptions::builder()
                        .name("channel_inspection_purge_ttl".to_string())
                        .expire_after(Duration::ZERO)
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(case_id: &str) -> InspectionFixture {
        InspectionFixture {
            case_id: case_id.into(),
            platform: "telegram".into(),
            conversation_id: uuid::Uuid::new_v4().to_string(),
            bot_id: uuid::Uuid::new_v4().to_string(),
        }
    }

    #[test]
    fn fixture_debug_and_runtime_debug_do_not_expose_destinations() {
        let fixture = fixture("telegram_private");
        let rendered = format!("{fixture:?}");
        assert!(!rendered.contains(&fixture.conversation_id));
        assert!(!rendered.contains(&fixture.bot_id));
    }

    #[test]
    fn request_contract_rejects_destinations_and_message_bodies() {
        let request = serde_json::json!({
            "protocol_version": 1,
            "run_id": "20261118T160000+0800",
            "event_id": uuid::Uuid::new_v4().to_string(),
            "timestamp": Utc::now(),
            "conversation_id": uuid::Uuid::new_v4().to_string(),
            "message": "arbitrary",
        });
        assert!(serde_json::from_value::<DispatchRequest>(request).is_err());
    }

    #[tokio::test]
    async fn full_runtime_requires_exactly_ten_unique_cases() {
        let fixtures: Vec<_> = (0..9)
            .map(|index| {
                let fixture = fixture(&format!("case_{index}"));
                serde_json::json!({
                    "case_id": fixture.case_id,
                    "platform": fixture.platform,
                    "conversation_id": fixture.conversation_id,
                    "bot_id": fixture.bot_id,
                })
            })
            .collect();
        let json = serde_json::to_string(&fixtures).unwrap();
        let client = mongodb::Client::with_uri_str("mongodb://127.0.0.1:27017")
            .await
            .expect("client");
        let error = Runtime::new(
            client.database("inspection_config_test"),
            &hex::encode([7; 32]),
            &json,
            true,
        )
        .unwrap_err();
        assert!(error.contains("exactly 10"));
    }

    #[test]
    fn allow_agent_initiated_is_not_part_of_fixture_or_request_contract() {
        let fixture = fixture("telegram_private");
        let fixture_value = serde_json::json!({
            "case_id": fixture.case_id,
            "platform": fixture.platform,
            "conversation_id": fixture.conversation_id,
            "bot_id": fixture.bot_id,
        });
        assert!(fixture_value.get("allow_agent_initiated").is_none());
        assert!(serde_json::from_value::<InspectionFixture>(fixture_value).is_ok());
    }

    #[test]
    fn fixture_contract_rejects_extra_authority_and_wildcard_routes() {
        let fixture_row = fixture("telegram_private");
        let with_authority = serde_json::json!({
            "case_id": fixture_row.case_id,
            "platform": fixture_row.platform,
            "conversation_id": fixture_row.conversation_id,
            "bot_id": fixture_row.bot_id,
            "allow_agent_initiated": true,
        });
        assert!(serde_json::from_value::<InspectionFixture>(with_authority).is_err());

        let mut wildcard = fixture("telegram_wildcard");
        wildcard.conversation_id = "*".into();
        assert!(validate_fixture(&wildcard).is_err());
    }

    #[test]
    fn fixture_contract_locks_the_exact_case_platform_matrix() {
        let mut row = fixture("TG-P-DM-O-01");
        assert!(validate_fixture(&row).is_ok());
        row.platform = "discord".into();
        assert!(validate_fixture(&row).is_err());

        row = fixture("arbitrary_case");
        assert!(validate_fixture(&row).is_err());
        assert_eq!(
            case_contract("LK-O-GRP-O-01").unwrap().conversation_type,
            "group"
        );
        assert!(case_contract("LK-O-GRP-O-01").unwrap().organization_owned);
    }

    #[test]
    fn request_validation_requires_current_protocol_and_event_uuid() {
        let mut request = DispatchRequest {
            protocol_version: PROTOCOL_VERSION,
            run_id: "20261118T160000+0800".into(),
            event_id: uuid::Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
        };
        assert!(validate_request(&request).is_ok());
        request.protocol_version += 1;
        assert!(validate_request(&request).is_err());
        request.protocol_version = PROTOCOL_VERSION;
        request.event_id = "platform-message-id".into();
        assert!(validate_request(&request).is_err());
    }
}
