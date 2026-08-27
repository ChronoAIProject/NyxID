use std::collections::HashSet;

use chrono::Utc;
use futures::{StreamExt, TryStreamExt};
use mongodb::bson::doc;
use mongodb::options::ReturnDocument;
use reqwest::header::CONTENT_TYPE;
use serde::Deserialize;
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::platform_op_usage::{COLLECTION_NAME as PLATFORM_OP_USAGE, PlatformOpUsage};
use crate::models::platform_operation::{
    COLLECTION_NAME as PLATFORM_OPERATIONS, CallAndSayConfig, PlatformOperation,
    PlatformOperationConfig, PlatformOperationName, SpeakConfig, XSearchConfig,
    default_call_max_message_chars, default_call_max_per_user_per_day, default_call_voice,
    default_speak_max_chars, default_speak_model_id, default_x_search_max_results_cap,
};
use crate::services::{assistant_service, proxy_service};

pub const X_SEARCH_HARD_MAX_RESULTS: u32 = 25;
pub const SPEAK_HARD_MAX_CHARS: u32 = 5_000;
pub const CALL_AND_SAY_HARD_MAX_MESSAGE_CHARS: u32 = 1_000;
pub const MCP_SPEAK_HARD_MAX_AUDIO_BYTES: usize = 16 * 1024 * 1024;
const MAX_VENDOR_JSON_RESPONSE_BYTES: usize = 5 * 1024 * 1024;

pub const X_SEARCH_VENDOR_SLUG: &str = "platform-x";
pub const SPEAK_VENDOR_SLUG: &str = "platform-elevenlabs";
pub const CALL_AND_SAY_VENDOR_SLUG: &str = "platform-twilio";
pub const PLATFORM_OPERATION_NAMES: [PlatformOperationName; 3] = [
    PlatformOperationName::XSearch,
    PlatformOperationName::Speak,
    PlatformOperationName::CallAndSay,
];

/// Runtime-enforced operation contract. Keep this registry code-owned: admin
/// templates are provisioning metadata and must never be able to weaken these
/// checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlatformOperationVendorContract {
    pub operation: PlatformOperationName,
    pub slug: &'static str,
    pub base_url: &'static str,
    pub auth_method: &'static str,
    pub auth_key_name: Option<&'static str>,
}

pub const PLATFORM_OPERATION_VENDOR_CONTRACTS: [PlatformOperationVendorContract; 3] = [
    PlatformOperationVendorContract {
        operation: PlatformOperationName::CallAndSay,
        slug: CALL_AND_SAY_VENDOR_SLUG,
        base_url: "https://api.twilio.com",
        auth_method: "basic",
        auth_key_name: None,
    },
    PlatformOperationVendorContract {
        operation: PlatformOperationName::Speak,
        slug: SPEAK_VENDOR_SLUG,
        base_url: "https://api.elevenlabs.io",
        auth_method: "header",
        auth_key_name: Some("xi-api-key"),
    },
    PlatformOperationVendorContract {
        operation: PlatformOperationName::XSearch,
        slug: X_SEARCH_VENDOR_SLUG,
        base_url: "https://api.x.com",
        auth_method: "bearer",
        auth_key_name: None,
    },
];

/// Seed data for the admin-managed template collection. Duffel deliberately
/// has no operation binding yet; a future operation adds a code contract and
/// sets this row's `operation` to that operation name before it can be bound.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeededPlatformVendorTemplate {
    pub vendor: &'static str,
    pub display_name: &'static str,
    pub slug: &'static str,
    pub base_url: &'static str,
    pub auth_method: &'static str,
    pub auth_key_name: Option<&'static str>,
    pub credential_label: &'static str,
    pub credential_note: &'static str,
    pub operation: Option<&'static str>,
    pub capability_summary: &'static str,
    pub restriction_summary: &'static str,
}

pub const DEFAULT_PLATFORM_VENDOR_TEMPLATES: [SeededPlatformVendorTemplate; 4] = [
    SeededPlatformVendorTemplate {
        vendor: "twilio",
        display_name: "Twilio",
        slug: CALL_AND_SAY_VENDOR_SLUG,
        base_url: "https://api.twilio.com",
        auth_method: "basic",
        auth_key_name: None,
        credential_label: "Auth token",
        credential_note: "Use the Auth Token paired with the Account SID configured on Call and Say.",
        operation: Some("call_and_say"),
        capability_summary: "Places server-constructed voice calls through call_and_say.",
        restriction_summary: "Does not expose Twilio's general API or import its OpenAPI operations.",
    },
    SeededPlatformVendorTemplate {
        vendor: "elevenlabs",
        display_name: "ElevenLabs",
        slug: SPEAK_VENDOR_SLUG,
        base_url: "https://api.elevenlabs.io",
        auth_method: "header",
        auth_key_name: Some("xi-api-key"),
        credential_label: "API key",
        credential_note: "Use a restricted ElevenLabs API key with text-to-speech access.",
        operation: Some("speak"),
        capability_summary: "Synthesizes speech through the server-constructed speak operation.",
        restriction_summary: "Does not expose voice cloning or import ElevenLabs vendor tools.",
    },
    SeededPlatformVendorTemplate {
        vendor: "x",
        display_name: "X",
        slug: X_SEARCH_VENDOR_SLUG,
        base_url: "https://api.x.com",
        auth_method: "bearer",
        auth_key_name: None,
        credential_label: "Bearer token",
        credential_note: "Use an app bearer token with read access to recent search.",
        operation: Some("x_search"),
        capability_summary: "Searches recent posts through the bounded x_search operation.",
        restriction_summary: "Does not publish, modify accounts, or expose X's general API.",
    },
    SeededPlatformVendorTemplate {
        vendor: "duffel",
        display_name: "Duffel",
        slug: "platform-duffel",
        base_url: "https://api.duffel.com",
        auth_method: "bearer",
        auth_key_name: None,
        credential_label: "Access token",
        credential_note: "Store a server-side Duffel access token for a future platform operation.",
        operation: None,
        capability_summary: "Pre-provisions the credential row; no Duffel platform operation is shipped yet.",
        restriction_summary: "Does not expose Duffel endpoints or publish any executable vendor tools.",
    },
];

pub fn vendor_requirement_for_operation(
    op: PlatformOperationName,
) -> &'static PlatformOperationVendorContract {
    PLATFORM_OPERATION_VENDOR_CONTRACTS
        .iter()
        .find(|contract| contract.operation == op)
        .expect("every shipped platform operation must bind one vendor contract")
}

pub fn vendor_contract_for_operation_name(
    name: &str,
) -> Option<&'static PlatformOperationVendorContract> {
    PLATFORM_OPERATION_VENDOR_CONTRACTS
        .iter()
        .find(|contract| operation_name(contract.operation) == name)
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct XSearchRequest {
    pub query: String,
    pub max_results: Option<u32>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SpeakRequest {
    pub text: String,
    pub voice_id: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CallAndSayRequest {
    pub to: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XSearchUpstreamRequest {
    pub path: &'static str,
    pub query: String,
    pub max_results: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SpeakUpstreamRequest {
    pub path: String,
    pub body: serde_json::Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallAndSayUpstreamRequest {
    pub path: String,
    pub form: Vec<(&'static str, String)>,
}

#[derive(Debug)]
pub struct SpeakVendorResponse {
    pub response: reqwest::Response,
}

struct VendorTarget {
    base_url: String,
    auth_key_name: String,
    credential: Zeroizing<String>,
}

pub fn operation_name(op: PlatformOperationName) -> &'static str {
    match op {
        PlatformOperationName::XSearch => "x_search",
        PlatformOperationName::Speak => "speak",
        PlatformOperationName::CallAndSay => "call_and_say",
    }
}

pub fn parse_operation_name(value: &str) -> AppResult<PlatformOperationName> {
    match value {
        "x_search" => Ok(PlatformOperationName::XSearch),
        "speak" => Ok(PlatformOperationName::Speak),
        "call_and_say" => Ok(PlatformOperationName::CallAndSay),
        _ => Err(AppError::NotFound(
            "Platform operation not found".to_string(),
        )),
    }
}

pub fn default_operation_config(op: PlatformOperationName) -> PlatformOperationConfig {
    match op {
        PlatformOperationName::XSearch => PlatformOperationConfig::XSearch(XSearchConfig {
            max_results_cap: default_x_search_max_results_cap(),
        }),
        PlatformOperationName::Speak => PlatformOperationConfig::Speak(SpeakConfig {
            allowed_voice_ids: Vec::new(),
            max_chars: default_speak_max_chars(),
            model_id: default_speak_model_id(),
        }),
        PlatformOperationName::CallAndSay => {
            PlatformOperationConfig::CallAndSay(CallAndSayConfig {
                allowed_destination_prefixes: Vec::new(),
                max_message_chars: default_call_max_message_chars(),
                voice: default_call_voice(),
                max_calls_per_user_per_day: default_call_max_per_user_per_day(),
                account_sid: String::new(),
                call_from: String::new(),
            })
        }
    }
}

pub fn default_vendor_service_slug(op: PlatformOperationName) -> &'static str {
    vendor_requirement_for_operation(op).slug
}

pub fn validate_operation_config(
    op: PlatformOperationName,
    vendor_service_slug: &str,
    config: &PlatformOperationConfig,
) -> AppResult<()> {
    validate_vendor_service_slug(vendor_service_slug)?;

    match (op, config) {
        (PlatformOperationName::XSearch, PlatformOperationConfig::XSearch(config)) => {
            if !(1..=X_SEARCH_HARD_MAX_RESULTS).contains(&config.max_results_cap) {
                return Err(AppError::BadRequest(format!(
                    "max_results_cap must be between 1 and {X_SEARCH_HARD_MAX_RESULTS}."
                )));
            }
        }
        (PlatformOperationName::Speak, PlatformOperationConfig::Speak(config)) => {
            validate_speak_config(config)?;
        }
        (PlatformOperationName::CallAndSay, PlatformOperationConfig::CallAndSay(config)) => {
            validate_call_and_say_config(config)?
        }
        _ => {
            return Err(AppError::BadRequest(
                "config type must match the platform operation.".to_string(),
            ));
        }
    }

    Ok(())
}

fn validate_vendor_service_slug(slug: &str) -> AppResult<()> {
    let valid = !slug.is_empty()
        && slug.len() <= 128
        && slug
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-');
    if !valid {
        return Err(AppError::BadRequest(
            "vendor_service_slug must contain only lowercase letters, digits, and hyphens."
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_speak_config(config: &SpeakConfig) -> AppResult<()> {
    if config.allowed_voice_ids.is_empty() {
        return Err(AppError::BadRequest(
            "allowed_voice_ids must contain at least one voice id.".to_string(),
        ));
    }
    if config.allowed_voice_ids.len() > 100 {
        return Err(AppError::BadRequest(
            "allowed_voice_ids must contain at most 100 voice ids.".to_string(),
        ));
    }
    let mut unique = HashSet::new();
    for voice_id in &config.allowed_voice_ids {
        if !is_safe_identifier(voice_id, 128) {
            return Err(AppError::BadRequest(
                "Each allowed voice id must use only letters, digits, hyphens, and underscores."
                    .to_string(),
            ));
        }
        if !unique.insert(voice_id) {
            return Err(AppError::BadRequest(
                "allowed_voice_ids must not contain duplicates.".to_string(),
            ));
        }
    }
    if !(1..=SPEAK_HARD_MAX_CHARS).contains(&config.max_chars) {
        return Err(AppError::BadRequest(format!(
            "max_chars must be between 1 and {SPEAK_HARD_MAX_CHARS}."
        )));
    }
    if !is_safe_identifier(&config.model_id, 128) {
        return Err(AppError::BadRequest(
            "model_id must use only letters, digits, periods, hyphens, and underscores."
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_call_and_say_config(config: &CallAndSayConfig) -> AppResult<()> {
    if config.allowed_destination_prefixes.len() > 100 {
        return Err(AppError::BadRequest(
            "allowed_destination_prefixes must contain at most 100 prefixes.".to_string(),
        ));
    }
    let mut unique = HashSet::new();
    for prefix in &config.allowed_destination_prefixes {
        if !is_e164_prefix(prefix) {
            return Err(AppError::BadRequest(format!(
                "Invalid E.164 destination prefix: {prefix}."
            )));
        }
        if !unique.insert(prefix) {
            return Err(AppError::BadRequest(
                "allowed_destination_prefixes must not contain duplicates.".to_string(),
            ));
        }
    }
    if !(1..=CALL_AND_SAY_HARD_MAX_MESSAGE_CHARS).contains(&config.max_message_chars) {
        return Err(AppError::BadRequest(format!(
            "max_message_chars must be between 1 and {CALL_AND_SAY_HARD_MAX_MESSAGE_CHARS}."
        )));
    }
    if !is_safe_identifier(&config.voice, 128) {
        return Err(AppError::BadRequest(
            "voice must use only letters, digits, periods, hyphens, and underscores.".to_string(),
        ));
    }
    if config.max_calls_per_user_per_day == 0 {
        return Err(AppError::BadRequest(
            "max_calls_per_user_per_day must be at least 1.".to_string(),
        ));
    }
    if !is_twilio_account_sid(&config.account_sid) {
        return Err(AppError::BadRequest(
            "account_sid must be a Twilio Account SID (AC followed by 32 hexadecimal characters)."
                .to_string(),
        ));
    }
    if !is_e164_number(&config.call_from) {
        return Err(AppError::BadRequest(
            "call_from must be a valid E.164 phone number.".to_string(),
        ));
    }
    Ok(())
}

fn is_safe_identifier(value: &str, max_len: usize) -> bool {
    !value.is_empty()
        && value.len() <= max_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
}

fn is_twilio_account_sid(value: &str) -> bool {
    value.len() == 34
        && value.starts_with("AC")
        && value[2..].bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn is_e164_prefix(value: &str) -> bool {
    let Some(digits) = value.strip_prefix('+') else {
        return false;
    };
    !digits.is_empty()
        && digits.len() <= 15
        && !digits.starts_with('0')
        && digits.bytes().all(|byte| byte.is_ascii_digit())
}

pub fn is_e164_number(value: &str) -> bool {
    is_e164_prefix(value)
}

pub fn destination_matches_prefixes(to: &str, prefixes: &[String]) -> bool {
    prefixes.iter().any(|prefix| to.starts_with(prefix))
}

pub fn build_x_search_request(
    config: &XSearchConfig,
    request: &XSearchRequest,
) -> AppResult<XSearchUpstreamRequest> {
    let query_chars = request.query.chars().count();
    if !(1..=512).contains(&query_chars) {
        return Err(AppError::BadRequest(
            "query must contain between 1 and 512 characters.".to_string(),
        ));
    }
    let configured_cap = config.max_results_cap.min(X_SEARCH_HARD_MAX_RESULTS);
    if configured_cap == 0 {
        return Err(AppError::PlatformOperationUnavailable);
    }
    let requested = request.max_results.unwrap_or(configured_cap);
    if requested == 0 {
        return Err(AppError::BadRequest(
            "max_results must be at least 1.".to_string(),
        ));
    }
    let max_results = requested.min(configured_cap);
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("query", &request.query)
        .append_pair("max_results", &max_results.to_string())
        .finish();

    Ok(XSearchUpstreamRequest {
        path: "2/tweets/search/recent",
        query,
        max_results,
    })
}

pub fn build_speak_request(
    config: &SpeakConfig,
    request: &SpeakRequest,
) -> AppResult<SpeakUpstreamRequest> {
    let allowed = config
        .allowed_voice_ids
        .iter()
        .any(|voice_id| voice_id == &request.voice_id);
    if !allowed {
        return Err(AppError::BadRequest(format!(
            "voice_id must be one of the allowed values: {}.",
            config.allowed_voice_ids.join(", ")
        )));
    }

    let text_chars = request.text.chars().count();
    let max_chars = config.max_chars.min(SPEAK_HARD_MAX_CHARS) as usize;
    if text_chars == 0 || text_chars > max_chars {
        return Err(AppError::BadRequest(format!(
            "text must contain between 1 and {max_chars} characters."
        )));
    }

    Ok(SpeakUpstreamRequest {
        path: format!("v1/text-to-speech/{}", request.voice_id),
        body: serde_json::json!({
            "text": request.text,
            "model_id": config.model_id,
        }),
    })
}

pub fn build_call_and_say_request(
    config: &CallAndSayConfig,
    request: &CallAndSayRequest,
) -> AppResult<CallAndSayUpstreamRequest> {
    if !is_e164_number(&request.to) {
        return Err(AppError::BadRequest(
            "to must be a valid E.164 phone number.".to_string(),
        ));
    }
    if !destination_matches_prefixes(&request.to, &config.allowed_destination_prefixes) {
        return Err(AppError::BadRequest(
            "Destination is not allowed for this platform operation.".to_string(),
        ));
    }

    let message_chars = request.message.chars().count();
    let max_chars = config
        .max_message_chars
        .min(CALL_AND_SAY_HARD_MAX_MESSAGE_CHARS) as usize;
    if message_chars == 0 || message_chars > max_chars {
        return Err(AppError::BadRequest(format!(
            "message must contain between 1 and {max_chars} characters."
        )));
    }
    if request.message.chars().any(char::is_control) {
        return Err(AppError::BadRequest(
            "message must not contain control characters.".to_string(),
        ));
    }

    let twiml = format!(
        "<Response><Say voice=\"{}\">{}</Say></Response>",
        xml_escape(&config.voice),
        xml_escape(&request.message)
    );

    Ok(CallAndSayUpstreamRequest {
        path: format!("2010-04-01/Accounts/{}/Calls.json", config.account_sid),
        form: vec![
            ("To", request.to.clone()),
            ("From", config.call_from.clone()),
            ("Twiml", twiml),
        ],
    })
}

pub fn xml_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        match character {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&apos;"),
            _ => escaped.push(character),
        }
    }
    escaped
}

pub fn redacted_destination_suffix(to: &str) -> String {
    let suffix: String = to
        .chars()
        .rev()
        .take(4)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    format!("***{suffix}")
}

pub async fn load_enabled_operation(
    db: &mongodb::Database,
    op: PlatformOperationName,
) -> AppResult<PlatformOperation> {
    let operation = db
        .collection::<PlatformOperation>(PLATFORM_OPERATIONS)
        .find_one(doc! { "op": operation_name(op), "enabled": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Platform operation not found".to_string()))?;

    validate_operation_config(
        operation.op,
        &operation.vendor_service_slug,
        &operation.config,
    )
    .map_err(|error| {
        tracing::error!(
            op = operation_name(op),
            error = %error,
            "Stored platform operation config is invalid"
        );
        AppError::PlatformOperationUnavailable
    })?;
    if operation.op != op {
        return Err(AppError::PlatformOperationUnavailable);
    }
    Ok(operation)
}

pub async fn list_configured_operations(
    db: &mongodb::Database,
) -> AppResult<Vec<PlatformOperation>> {
    let operation_names = PLATFORM_OPERATION_NAMES
        .iter()
        .map(|op| operation_name(*op))
        .collect::<Vec<_>>();
    db.collection::<PlatformOperation>(PLATFORM_OPERATIONS)
        .find(doc! { "op": { "$in": operation_names } })
        .await?
        .try_collect()
        .await
        .map_err(AppError::DatabaseError)
}

pub async fn list_enabled_operations(db: &mongodb::Database) -> AppResult<Vec<PlatformOperation>> {
    let configured = list_configured_operations(db).await?;
    Ok(configured
        .into_iter()
        .filter(|operation| {
            if !operation.enabled {
                return false;
            }
            if let Err(error) = validate_operation_config(
                operation.op,
                &operation.vendor_service_slug,
                &operation.config,
            ) {
                tracing::error!(
                    op = operation_name(operation.op),
                    error = %error,
                    "Omitting invalid enabled platform operation from discovery"
                );
                return false;
            }
            true
        })
        .collect())
}

pub async fn upsert_operation(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    op: PlatformOperationName,
    enabled: bool,
    vendor_service_slug: String,
    config: PlatformOperationConfig,
    updated_by: &str,
) -> AppResult<PlatformOperation> {
    validate_operation_config(op, &vendor_service_slug, &config)?;
    validate_vendor_binding(db, encryption_keys, op, &vendor_service_slug).await?;

    let config = bson::to_bson(&config).map_err(|error| {
        AppError::Internal(format!(
            "Failed to serialize platform operation config: {error}"
        ))
    })?;
    let updated_at = Utc::now();
    db.collection::<PlatformOperation>(PLATFORM_OPERATIONS)
        .find_one_and_update(
            doc! { "op": operation_name(op) },
            doc! {
                "$set": {
                    "enabled": enabled,
                    "vendor_service_slug": vendor_service_slug,
                    "config": config,
                    "updated_at": bson::DateTime::from_chrono(updated_at),
                    "updated_by": updated_by,
                },
                "$setOnInsert": {
                    "_id": uuid::Uuid::new_v4().to_string(),
                    "op": operation_name(op),
                },
            },
        )
        .upsert(true)
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| {
            AppError::Internal("Platform operation upsert returned no document".to_string())
        })
}

pub async fn execute_x_search(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    http_client: &reqwest::Client,
    request: XSearchRequest,
) -> AppResult<serde_json::Value> {
    let operation = load_enabled_operation(db, PlatformOperationName::XSearch).await?;
    let PlatformOperationConfig::XSearch(config) = &operation.config else {
        return Err(AppError::PlatformOperationUnavailable);
    };
    let upstream = build_x_search_request(config, &request)?;
    let target = resolve_vendor_target(
        db,
        encryption_keys,
        PlatformOperationName::XSearch,
        &operation.vendor_service_slug,
    )
    .await?;
    let url = format!(
        "{}/{}?{}",
        target.base_url.trim_end_matches('/'),
        upstream.path,
        upstream.query
    );
    let response = http_client
        .get(url)
        .bearer_auth(target.credential.as_str())
        .send()
        .await
        .map_err(|error| vendor_request_failed(PlatformOperationName::XSearch, error))?;
    read_vendor_json(PlatformOperationName::XSearch, response).await
}

pub async fn execute_speak(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    http_client: &reqwest::Client,
    request: SpeakRequest,
) -> AppResult<SpeakVendorResponse> {
    let operation = load_enabled_operation(db, PlatformOperationName::Speak).await?;
    let PlatformOperationConfig::Speak(config) = &operation.config else {
        return Err(AppError::PlatformOperationUnavailable);
    };
    let upstream = build_speak_request(config, &request)?;
    let target = resolve_vendor_target(
        db,
        encryption_keys,
        PlatformOperationName::Speak,
        &operation.vendor_service_slug,
    )
    .await?;
    let url = format!(
        "{}/{}",
        target.base_url.trim_end_matches('/'),
        upstream.path
    );
    let response = http_client
        .post(url)
        .header(&target.auth_key_name, target.credential.as_str())
        .header(CONTENT_TYPE, "application/json")
        .json(&upstream.body)
        .send()
        .await
        .map_err(|error| vendor_request_failed(PlatformOperationName::Speak, error))?;
    ensure_vendor_success(PlatformOperationName::Speak, &response)?;

    Ok(SpeakVendorResponse { response })
}

pub async fn collect_speak_audio(vendor: SpeakVendorResponse) -> AppResult<Vec<u8>> {
    if vendor
        .response
        .content_length()
        .is_some_and(|length| length > MCP_SPEAK_HARD_MAX_AUDIO_BYTES as u64)
    {
        tracing::error!("Platform speak response exceeded the MCP audio response limit");
        return Err(AppError::PlatformOperationUnavailable);
    }

    let mut audio = Vec::new();
    let mut stream = vendor.response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk =
            chunk.map_err(|error| vendor_request_failed(PlatformOperationName::Speak, error))?;
        if audio.len().saturating_add(chunk.len()) > MCP_SPEAK_HARD_MAX_AUDIO_BYTES {
            tracing::error!("Platform speak response exceeded the MCP audio response limit");
            return Err(AppError::PlatformOperationUnavailable);
        }
        audio.extend_from_slice(&chunk);
    }
    Ok(audio)
}

pub async fn execute_call_and_say(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    http_client: &reqwest::Client,
    user_id: &str,
    yyyymmdd: &str,
    request: CallAndSayRequest,
) -> AppResult<serde_json::Value> {
    validate_usage_date(yyyymmdd)?;
    let operation = load_enabled_operation(db, PlatformOperationName::CallAndSay).await?;
    let PlatformOperationConfig::CallAndSay(config) = &operation.config else {
        return Err(AppError::PlatformOperationUnavailable);
    };
    let upstream = build_call_and_say_request(config, &request)?;
    let target = resolve_vendor_target(
        db,
        encryption_keys,
        PlatformOperationName::CallAndSay,
        &operation.vendor_service_slug,
    )
    .await?;

    reserve_daily_call(db, user_id, yyyymmdd, config.max_calls_per_user_per_day).await?;
    let response = send_call_and_say(http_client, config, &target, &upstream).await;
    let response = match response {
        Ok(response) => response,
        Err(error) => {
            release_daily_call(db, user_id, yyyymmdd).await?;
            return Err(error);
        }
    };

    Ok(response)
}

async fn send_call_and_say(
    http_client: &reqwest::Client,
    config: &CallAndSayConfig,
    target: &VendorTarget,
    upstream: &CallAndSayUpstreamRequest,
) -> AppResult<serde_json::Value> {
    let url = format!(
        "{}/{}",
        target.base_url.trim_end_matches('/'),
        upstream.path
    );
    let response = http_client
        .post(url)
        .basic_auth(&config.account_sid, Some(target.credential.as_str()))
        .form(&upstream.form)
        .send()
        .await
        .map_err(|error| vendor_request_failed(PlatformOperationName::CallAndSay, error))?;
    read_vendor_json(PlatformOperationName::CallAndSay, response).await
}

async fn resolve_vendor_target(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    op: PlatformOperationName,
    slug: &str,
) -> AppResult<VendorTarget> {
    let requirement = vendor_requirement_for_operation(op);
    let service = assistant_service::resolve_admin_service_by_slug(db, slug)
        .await
        .map_err(|error| vendor_configuration_failed(op, error))?;
    validate_vendor_binding_shape(requirement, slug, Some(&service), false)
        .map_err(|error| vendor_configuration_failed(op, error))?;
    let authorized = proxy_service::authorize_master_credential_server_chosen(db, &service)
        .await
        .map_err(|error| vendor_configuration_failed(op, error))?;
    let decrypted = Zeroizing::new(
        authorized
            .decrypt(encryption_keys)
            .await
            .map_err(|error| vendor_configuration_failed(op, error))?,
    );
    let credential = Zeroizing::new(String::from_utf8((*decrypted).clone()).map_err(|error| {
        tracing::error!(
            op = operation_name(op),
            error = %error,
            "Platform operation vendor credential is not UTF-8"
        );
        AppError::PlatformOperationUnavailable
    })?);
    if credential.is_empty() {
        return Err(AppError::PlatformOperationUnavailable);
    }

    Ok(VendorTarget {
        base_url: service.base_url,
        auth_key_name: service.auth_key_name,
        credential,
    })
}

async fn validate_vendor_binding(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    op: PlatformOperationName,
    slug: &str,
) -> AppResult<()> {
    let collection = db.collection::<DownstreamService>(DOWNSTREAM_SERVICES);
    let service = match collection
        .find_one(doc! { "slug": slug, "is_active": true })
        .await?
    {
        Some(service) => Some(service),
        None => collection.find_one(doc! { "slug": slug }).await?,
    };
    let requirement = vendor_requirement_for_operation(op);
    validate_vendor_binding_shape(requirement, slug, service.as_ref(), true)?;
    let service = service.expect("validated vendor binding must have a service row");

    if service.credential_encrypted.is_empty() {
        return validate_vendor_credential(requirement, &service, b"");
    }
    let credential = Zeroizing::new(
        encryption_keys
            .decrypt(&service.credential_encrypted)
            .await
            .map_err(|_| {
                AppError::PlatformVendorProvisioningInvalid(format!(
                    "{} requires a readable credential; row '{}' cannot be decrypted and must be replaced",
                    operation_name(op), service.slug
                ))
            })?,
    );
    validate_vendor_credential(requirement, &service, credential.as_slice())
}

/// `enforce_base_url` is true only at provisioning time. The canonical base URL is a
/// template default and a bind-time guard against typos -- it is deliberately NOT a
/// runtime gate, so an operator may legitimately point a vendor row at a regional
/// endpoint, an egress proxy, or a test double. The security-bearing checks (auth
/// shape, category, visibility, credential) are enforced on every path.
fn validate_vendor_binding_shape(
    requirement: &PlatformOperationVendorContract,
    slug: &str,
    service: Option<&DownstreamService>,
    enforce_base_url: bool,
) -> AppResult<()> {
    let op = operation_name(requirement.operation);
    let Some(service) = service else {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "{op} requires vendor row '{slug}'; no row with that slug exists"
        )));
    };
    if slug != requirement.slug {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "{op} requires canonical vendor_service_slug '{}'; requested row slug is '{slug}'",
            requirement.slug
        )));
    }
    if enforce_base_url && service.base_url.trim_end_matches('/') != requirement.base_url {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "{op} requires base_url '{}'; row '{}' has '{}'",
            requirement.base_url, service.slug, service.base_url
        )));
    }
    if !service.is_active {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "{op} requires an active vendor row; row '{}' is inactive",
            service.slug
        )));
    }
    if service.auth_method != requirement.auth_method {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "{op} requires auth_method '{}'; row '{}' has '{}'",
            requirement.auth_method, service.slug, service.auth_method
        )));
    }
    if let Some(expected) = requirement.auth_key_name
        && !service.auth_key_name.eq_ignore_ascii_case(expected)
    {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "{op} requires auth_key_name '{expected}'; row '{}' has '{}'",
            service.slug, service.auth_key_name
        )));
    }
    if service.service_category != "internal" {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "{op} requires service_category 'internal'; row '{}' has '{}'",
            service.slug, service.service_category
        )));
    }
    if service.visibility != "public" {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "{op} requires visibility 'public'; row '{}' has '{}'",
            service.slug, service.visibility
        )));
    }
    if service.provider_config_id.is_some() {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "{op} requires provider_config_id to be absent; row '{}' has '{}'",
            service.slug,
            service.provider_config_id.as_deref().unwrap_or_default()
        )));
    }
    if service.service_type != "http" {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "{op} requires service_type 'http'; row '{}' has '{}'",
            service.slug, service.service_type
        )));
    }
    if service.requires_user_credential {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "{op} requires requires_user_credential false; row '{}' has true",
            service.slug
        )));
    }
    Ok(())
}

fn validate_vendor_credential(
    requirement: &PlatformOperationVendorContract,
    service: &DownstreamService,
    credential: &[u8],
) -> AppResult<()> {
    if credential.is_empty() {
        let op = operation_name(requirement.operation);
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "{op} requires a non-empty credential; row '{}' has an empty credential",
            service.slug
        )));
    }
    Ok(())
}

fn vendor_configuration_failed(op: PlatformOperationName, error: AppError) -> AppError {
    tracing::error!(
        op = operation_name(op),
        error = %error,
        "Platform operation vendor configuration is unavailable"
    );
    AppError::PlatformOperationUnavailable
}

fn vendor_request_failed(op: PlatformOperationName, error: reqwest::Error) -> AppError {
    tracing::error!(
        op = operation_name(op),
        error = %error,
        "Platform operation vendor request failed"
    );
    AppError::PlatformOperationUnavailable
}

fn ensure_vendor_success(op: PlatformOperationName, response: &reqwest::Response) -> AppResult<()> {
    if response.status().is_success() {
        return Ok(());
    }
    tracing::error!(
        op = operation_name(op),
        upstream_status = response.status().as_u16(),
        "Platform operation vendor returned an unsuccessful status"
    );
    Err(AppError::PlatformOperationUnavailable)
}

async fn read_vendor_json(
    op: PlatformOperationName,
    response: reqwest::Response,
) -> AppResult<serde_json::Value> {
    ensure_vendor_success(op, &response)?;
    if response
        .content_length()
        .is_some_and(|length| length > MAX_VENDOR_JSON_RESPONSE_BYTES as u64)
    {
        return Err(AppError::PlatformOperationUnavailable);
    }

    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| vendor_request_failed(op, error))?;
        if body.len().saturating_add(chunk.len()) > MAX_VENDOR_JSON_RESPONSE_BYTES {
            tracing::error!(
                op = operation_name(op),
                "Platform operation vendor response exceeded the JSON response limit"
            );
            return Err(AppError::PlatformOperationUnavailable);
        }
        body.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&body).map_err(|error| {
        tracing::error!(
            op = operation_name(op),
            error = %error,
            "Platform operation vendor returned invalid JSON"
        );
        AppError::PlatformOperationUnavailable
    })
}

fn validate_usage_date(yyyymmdd: &str) -> AppResult<()> {
    if yyyymmdd.len() == 8 && yyyymmdd.bytes().all(|byte| byte.is_ascii_digit()) {
        return Ok(());
    }
    Err(AppError::Internal(
        "Platform operation usage date must use YYYYMMDD format".to_string(),
    ))
}

async fn reserve_daily_call(
    db: &mongodb::Database,
    user_id: &str,
    yyyymmdd: &str,
    cap: u32,
) -> AppResult<()> {
    let collection = db.collection::<PlatformOpUsage>(PLATFORM_OP_USAGE);
    let filter = doc! {
        "op": operation_name(PlatformOperationName::CallAndSay),
        "user_id": user_id,
        "yyyymmdd": yyyymmdd,
        "count": { "$lt": i64::from(cap) },
    };
    let update = doc! {
        "$setOnInsert": {
            "_id": uuid::Uuid::new_v4().to_string(),
            "op": operation_name(PlatformOperationName::CallAndSay),
            "user_id": user_id,
            "yyyymmdd": yyyymmdd,
        },
        "$inc": { "count": 1_i64 },
        "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
    };
    match collection
        .find_one_and_update(filter, update)
        .upsert(true)
        .return_document(ReturnDocument::After)
        .await
    {
        Ok(Some(_)) => Ok(()),
        Ok(None) => Err(AppError::RateLimited),
        Err(error) if is_duplicate_key_error(&error) => Err(AppError::RateLimited),
        Err(error) => Err(AppError::DatabaseError(error)),
    }
}

async fn release_daily_call(
    db: &mongodb::Database,
    user_id: &str,
    yyyymmdd: &str,
) -> AppResult<()> {
    db.collection::<PlatformOpUsage>(PLATFORM_OP_USAGE)
        .update_one(
            doc! {
                "op": operation_name(PlatformOperationName::CallAndSay),
                "user_id": user_id,
                "yyyymmdd": yyyymmdd,
                "count": { "$gt": 0_i64 },
            },
            doc! {
                "$inc": { "count": -1_i64 },
                "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
            },
        )
        .await?;
    Ok(())
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Command(command) if command.code == 11000
    ) || matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::test_helpers::dummy_service;

    fn speak_config() -> SpeakConfig {
        SpeakConfig {
            allowed_voice_ids: vec!["voice-a".to_string(), "voice-b".to_string()],
            max_chars: 100,
            model_id: "eleven_multilingual_v2".to_string(),
        }
    }

    fn call_config(prefixes: Vec<String>) -> CallAndSayConfig {
        CallAndSayConfig {
            allowed_destination_prefixes: prefixes,
            max_message_chars: 500,
            voice: "alice".to_string(),
            max_calls_per_user_per_day: 3,
            account_sid: format!("AC{}", "1".repeat(32)),
            call_from: "+16505550100".to_string(),
        }
    }

    fn valid_speak_vendor_service() -> crate::models::downstream_service::DownstreamService {
        let mut service = dummy_service();
        service.slug = SPEAK_VENDOR_SLUG.to_string();
        service.base_url = "https://api.elevenlabs.io".to_string();
        service.auth_method = "header".to_string();
        service.auth_key_name = "xi-api-key".to_string();
        service.service_category = "internal".to_string();
        service.visibility = "public".to_string();
        service.requires_user_credential = false;
        service.credential_encrypted = vec![1];
        service
    }

    fn provisioning_message(error: AppError) -> String {
        let AppError::PlatformVendorProvisioningInvalid(message) = error else {
            panic!("expected a platform vendor provisioning error, got {error:?}");
        };
        message
    }

    #[test]
    fn operation_contracts_and_seeded_templates_are_consistent() {
        assert_eq!(PLATFORM_OPERATION_VENDOR_CONTRACTS.len(), 3);
        for contract in PLATFORM_OPERATION_VENDOR_CONTRACTS {
            assert_eq!(
                vendor_requirement_for_operation(contract.operation),
                &contract
            );
            assert_eq!(
                default_vendor_service_slug(contract.operation),
                contract.slug
            );
        }

        for template in DEFAULT_PLATFORM_VENDOR_TEMPLATES {
            let Some(operation) = template.operation else {
                // Duffel is intentionally a credential template until a code
                // operation is shipped; there is no contract to cross-check.
                continue;
            };
            let operation = parse_operation_name(operation).expect("seeded operation name");
            let contract = vendor_requirement_for_operation(operation);
            assert_eq!(template.auth_method, contract.auth_method);
            assert_eq!(template.auth_key_name, contract.auth_key_name);
        }
    }

    #[test]
    fn bind_validation_names_every_vendor_row_mismatch() {
        let requirement = vendor_requirement_for_operation(PlatformOperationName::Speak);
        let valid = valid_speak_vendor_service();

        let cases: Vec<(
            &str,
            &str,
            Option<crate::models::downstream_service::DownstreamService>,
        )> = vec![
            (
                "speak requires canonical vendor_service_slug 'platform-elevenlabs'; requested row slug is 'platform-other'",
                "platform-other",
                Some(crate::models::downstream_service::DownstreamService {
                    slug: "platform-other".to_string(),
                    ..valid.clone()
                }),
            ),
            (
                "speak requires base_url 'https://api.elevenlabs.io'; row 'platform-elevenlabs' has 'https://wrong.example'",
                SPEAK_VENDOR_SLUG,
                Some(crate::models::downstream_service::DownstreamService {
                    base_url: "https://wrong.example".to_string(),
                    ..valid.clone()
                }),
            ),
            (
                "speak requires vendor row 'platform-elevenlabs'; no row with that slug exists",
                SPEAK_VENDOR_SLUG,
                None,
            ),
            (
                "speak requires an active vendor row; row 'platform-elevenlabs' is inactive",
                SPEAK_VENDOR_SLUG,
                Some(crate::models::downstream_service::DownstreamService {
                    is_active: false,
                    ..valid.clone()
                }),
            ),
            (
                "speak requires auth_method 'header'; row 'platform-elevenlabs' has 'bearer'",
                SPEAK_VENDOR_SLUG,
                Some(crate::models::downstream_service::DownstreamService {
                    auth_method: "bearer".to_string(),
                    ..valid.clone()
                }),
            ),
            (
                "speak requires auth_key_name 'xi-api-key'; row 'platform-elevenlabs' has 'X-API-Key'",
                SPEAK_VENDOR_SLUG,
                Some(crate::models::downstream_service::DownstreamService {
                    auth_key_name: "X-API-Key".to_string(),
                    ..valid.clone()
                }),
            ),
            (
                "speak requires service_category 'internal'; row 'platform-elevenlabs' has 'connection'",
                SPEAK_VENDOR_SLUG,
                Some(crate::models::downstream_service::DownstreamService {
                    service_category: "connection".to_string(),
                    ..valid.clone()
                }),
            ),
            (
                "speak requires visibility 'public'; row 'platform-elevenlabs' has 'private'",
                SPEAK_VENDOR_SLUG,
                Some(crate::models::downstream_service::DownstreamService {
                    visibility: "private".to_string(),
                    ..valid.clone()
                }),
            ),
            (
                "speak requires provider_config_id to be absent; row 'platform-elevenlabs' has 'provider-1'",
                SPEAK_VENDOR_SLUG,
                Some(crate::models::downstream_service::DownstreamService {
                    provider_config_id: Some("provider-1".to_string()),
                    ..valid.clone()
                }),
            ),
        ];

        for (expected, slug, service) in cases {
            let error = validate_vendor_binding_shape(requirement, slug, service.as_ref(), true)
                .expect_err("mismatched vendor row must be rejected");
            assert_eq!(provisioning_message(error), expected);
        }

        validate_vendor_binding_shape(requirement, SPEAK_VENDOR_SLUG, Some(&valid), true)
            .expect("valid vendor row shape");
        let error = validate_vendor_credential(requirement, &valid, b"")
            .expect_err("empty credential must be rejected");
        assert_eq!(
            provisioning_message(error),
            "speak requires a non-empty credential; row 'platform-elevenlabs' has an empty credential"
        );
    }

    #[tokio::test]
    async fn enabled_operation_listing_omits_disabled_and_invalid_rows() {
        let Some(db) = crate::test_utils::connect_test_database("platform_ops_enabled_list").await
        else {
            eprintln!("skipping platform operation listing test: no local MongoDB available");
            return;
        };
        let now = Utc::now();
        let rows = [
            PlatformOperation {
                id: uuid::Uuid::new_v4().to_string(),
                op: PlatformOperationName::XSearch,
                enabled: true,
                vendor_service_slug: X_SEARCH_VENDOR_SLUG.to_string(),
                config: PlatformOperationConfig::XSearch(XSearchConfig {
                    max_results_cap: 10,
                }),
                updated_at: now,
                updated_by: "admin-user".to_string(),
            },
            PlatformOperation {
                id: uuid::Uuid::new_v4().to_string(),
                op: PlatformOperationName::Speak,
                enabled: false,
                vendor_service_slug: SPEAK_VENDOR_SLUG.to_string(),
                config: PlatformOperationConfig::Speak(speak_config()),
                updated_at: now,
                updated_by: "admin-user".to_string(),
            },
            PlatformOperation {
                id: uuid::Uuid::new_v4().to_string(),
                op: PlatformOperationName::CallAndSay,
                enabled: true,
                vendor_service_slug: CALL_AND_SAY_VENDOR_SLUG.to_string(),
                config: PlatformOperationConfig::CallAndSay(CallAndSayConfig {
                    max_message_chars: CALL_AND_SAY_HARD_MAX_MESSAGE_CHARS + 1,
                    ..call_config(vec!["+65".to_string()])
                }),
                updated_at: now,
                updated_by: "admin-user".to_string(),
            },
        ];
        db.collection::<PlatformOperation>(PLATFORM_OPERATIONS)
            .insert_many(rows)
            .await
            .expect("insert platform operation rows");

        let enabled = list_enabled_operations(&db)
            .await
            .expect("list enabled operations");
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].op, PlatformOperationName::XSearch);
    }

    #[test]
    fn config_validation_enforces_hard_caps() {
        let x = PlatformOperationConfig::XSearch(XSearchConfig {
            max_results_cap: X_SEARCH_HARD_MAX_RESULTS + 1,
        });
        assert!(
            validate_operation_config(PlatformOperationName::XSearch, X_SEARCH_VENDOR_SLUG, &x)
                .is_err()
        );

        let speak = PlatformOperationConfig::Speak(SpeakConfig {
            max_chars: SPEAK_HARD_MAX_CHARS + 1,
            ..speak_config()
        });
        assert!(
            validate_operation_config(PlatformOperationName::Speak, SPEAK_VENDOR_SLUG, &speak)
                .is_err()
        );

        let call = PlatformOperationConfig::CallAndSay(CallAndSayConfig {
            max_message_chars: CALL_AND_SAY_HARD_MAX_MESSAGE_CHARS + 1,
            ..call_config(vec!["+65".to_string()])
        });
        assert!(
            validate_operation_config(
                PlatformOperationName::CallAndSay,
                CALL_AND_SAY_VENDOR_SLUG,
                &call
            )
            .is_err()
        );
    }

    #[test]
    fn empty_destination_prefixes_deny_every_destination() {
        let config = call_config(Vec::new());
        let request = CallAndSayRequest {
            to: "+16505550199".to_string(),
            message: "Hello".to_string(),
        };

        assert!(build_call_and_say_request(&config, &request).is_err());
    }

    #[test]
    fn destination_prefix_matching_is_anchored_at_the_start() {
        let prefixes = vec!["+65".to_string()];
        assert!(destination_matches_prefixes("+6512345678", &prefixes));
        assert!(!destination_matches_prefixes("+16512345678", &prefixes));
        assert!(!destination_matches_prefixes("+1650000065", &prefixes));
    }

    #[test]
    fn twiml_composition_escapes_xml_and_cdata_terminators() {
        let config = call_config(vec!["+65".to_string()]);
        let request = CallAndSayRequest {
            to: "+6512345678".to_string(),
            message: "A & B <tag> \"quoted\" 'single' ]]> done".to_string(),
        };

        let upstream = build_call_and_say_request(&config, &request).expect("valid request");
        let twiml = upstream
            .form
            .iter()
            .find(|(name, _)| *name == "Twiml")
            .map(|(_, value)| value)
            .expect("TwiML field");
        assert_eq!(
            twiml,
            "<Response><Say voice=\"alice\">A &amp; B &lt;tag&gt; &quot;quoted&quot; &apos;single&apos; ]]&gt; done</Say></Response>"
        );
        assert!(!twiml.contains("<tag>"));
        assert!(!twiml.contains("]]>"));
    }

    #[test]
    fn twiml_composition_rejects_control_characters() {
        let config = call_config(vec!["+65".to_string()]);
        let request = CallAndSayRequest {
            to: "+6512345678".to_string(),
            message: "hello\nworld".to_string(),
        };

        assert!(build_call_and_say_request(&config, &request).is_err());
    }

    #[test]
    fn voice_id_rejection_names_the_allowed_list() {
        let error = build_speak_request(
            &speak_config(),
            &SpeakRequest {
                text: "hello".to_string(),
                voice_id: "voice-c".to_string(),
            },
        )
        .expect_err("voice must be rejected");

        let AppError::BadRequest(message) = error else {
            panic!("expected a bad request");
        };
        assert!(message.contains("voice-a, voice-b"));
    }

    #[test]
    fn speak_body_is_server_constructed() {
        let upstream = build_speak_request(
            &speak_config(),
            &SpeakRequest {
                text: "hello".to_string(),
                voice_id: "voice-a".to_string(),
            },
        )
        .expect("valid request");

        assert_eq!(
            upstream.body,
            serde_json::json!({
                "text": "hello",
                "model_id": "eleven_multilingual_v2",
            })
        );
    }

    #[test]
    fn request_structs_reject_vendor_shape_fields() {
        assert!(
            serde_json::from_value::<CallAndSayRequest>(serde_json::json!({
                "to": "+6512345678",
                "message": "hello",
                "from": "+16505550100",
                "url": "https://attacker.invalid/twiml",
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<SpeakRequest>(serde_json::json!({
                "text": "hello",
                "voice_id": "voice-a",
                "model_id": "caller-model",
            }))
            .is_err()
        );
    }
}
