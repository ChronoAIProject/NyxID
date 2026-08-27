use std::collections::HashSet;
use std::sync::Arc;

use chrono::{Days, NaiveDate, Utc};
use futures::{StreamExt, TryStreamExt};
use mongodb::bson::doc;
use mongodb::options::ReturnDocument;
use reqwest::header::{ACCEPT, ACCEPT_ENCODING, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, ProxyOperationPolicy,
};
use crate::models::platform_op_usage::{COLLECTION_NAME as PLATFORM_OP_USAGE, PlatformOpUsage};
use crate::models::platform_operation::{
    COLLECTION_NAME as PLATFORM_OPERATIONS, CallAndSayConfig, FlightSearchConfig,
    PlatformOperation, PlatformOperationConfig, PlatformOperationName, SpeakConfig, XSearchConfig,
    default_call_max_message_chars, default_call_max_per_user_per_day, default_call_voice,
    default_flight_search_max_offers_cap, default_flight_search_max_per_user_per_day,
    default_speak_max_chars, default_speak_model_id, default_x_search_max_results_cap,
};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::services::billing::route_inventory::BillingEgressPermit;
use crate::services::connection_expiry_service::ConnectionExpiryNotifier;
use crate::services::node_ws_manager::NodeWsManager;
use crate::services::{assistant_service, proxy_service, user_service_service};

pub const X_SEARCH_HARD_MAX_RESULTS: u32 = 25;
pub const SPEAK_HARD_MAX_CHARS: u32 = 5_000;
pub const CALL_AND_SAY_HARD_MAX_MESSAGE_CHARS: u32 = 1_000;
pub const FLIGHT_SEARCH_HARD_MAX_OFFERS: u32 = 50;
pub const MCP_SPEAK_HARD_MAX_AUDIO_BYTES: usize = 16 * 1024 * 1024;
const MAX_VENDOR_JSON_RESPONSE_BYTES: usize = 5 * 1024 * 1024;

pub const X_SEARCH_VENDOR_SLUG: &str = "platform-x";
pub const SPEAK_VENDOR_SLUG: &str = "platform-elevenlabs";
pub const CALL_AND_SAY_VENDOR_SLUG: &str = "platform-twilio";
pub const FLIGHT_SEARCH_VENDOR_SLUG: &str = "platform-duffel";
pub const PLATFORM_OPERATION_NAMES: [PlatformOperationName; 4] = [
    PlatformOperationName::XSearch,
    PlatformOperationName::Speak,
    PlatformOperationName::CallAndSay,
    PlatformOperationName::FlightSearch,
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

pub const PLATFORM_OPERATION_VENDOR_CONTRACTS: [PlatformOperationVendorContract; 4] = [
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
    PlatformOperationVendorContract {
        operation: PlatformOperationName::FlightSearch,
        slug: FLIGHT_SEARCH_VENDOR_SLUG,
        base_url: "https://api.duffel.com",
        auth_method: "bearer",
        auth_key_name: None,
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlatformOperationCatalogContract {
    pub operation: PlatformOperationName,
    pub catalog_service_slug: &'static str,
    pub vendor: &'static str,
    pub display_name: &'static str,
    pub description: &'static str,
    pub mcp_tool: &'static str,
}

pub const PLATFORM_OPERATION_CATALOG_CONTRACTS: [PlatformOperationCatalogContract; 4] = [
    PlatformOperationCatalogContract {
        operation: PlatformOperationName::XSearch,
        catalog_service_slug: "api-twitter",
        vendor: "x",
        display_name: "X Search",
        description: "Search recent public posts on X.",
        mcp_tool: "nyx__x_search",
    },
    PlatformOperationCatalogContract {
        operation: PlatformOperationName::Speak,
        catalog_service_slug: "api-elevenlabs",
        vendor: "elevenlabs",
        display_name: "Speak",
        description: "Synthesize speech from bounded text input.",
        mcp_tool: "nyx__speak",
    },
    PlatformOperationCatalogContract {
        operation: PlatformOperationName::CallAndSay,
        catalog_service_slug: "api-twilio",
        vendor: "twilio",
        display_name: "Call and Say",
        description: "Place a voice call that speaks a bounded message.",
        mcp_tool: "nyx__call_and_say",
    },
    PlatformOperationCatalogContract {
        operation: PlatformOperationName::FlightSearch,
        catalog_service_slug: "duffel",
        vendor: "duffel",
        display_name: "Flight Search",
        description: "Search available flight offers without booking.",
        mcp_tool: "nyx__flight_search",
    },
];

/// Seed data for the admin-managed template collection.
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
        slug: FLIGHT_SEARCH_VENDOR_SLUG,
        base_url: "https://api.duffel.com",
        auth_method: "bearer",
        auth_key_name: None,
        credential_label: "Access token",
        credential_note: "Use a Duffel access token with permission to create offer requests.",
        operation: Some("flight_search"),
        capability_summary: "Searches flight offers through the bounded flight_search operation.",
        restriction_summary: "Does not create orders, payments, or cancellations and does not expose Duffel's general API.",
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

pub fn catalog_contract_for_operation(
    op: PlatformOperationName,
) -> &'static PlatformOperationCatalogContract {
    PLATFORM_OPERATION_CATALOG_CONTRACTS
        .iter()
        .find(|contract| contract.operation == op)
        .expect("every shipped platform operation must bind one user catalog contract")
}

pub fn is_platform_vendor_slug(slug: &str) -> bool {
    PLATFORM_OPERATION_VENDOR_CONTRACTS
        .iter()
        .any(|contract| contract.slug == slug)
}

pub fn platform_vendor_kill_policy() -> ProxyOperationPolicy {
    ProxyOperationPolicy { rules: Vec::new() }
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
    pub from: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FlightSearchRequest {
    pub origin: String,
    pub destination: String,
    pub departure_date: String,
    pub return_date: Option<String>,
    pub adults: Option<u32>,
    pub cabin_class: Option<String>,
    pub max_offers: Option<u32>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlightSearchUpstreamRequest {
    pub path: &'static str,
    pub query: &'static str,
    pub body: serde_json::Value,
    pub max_offers: u32,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct FlightSearchResponse {
    pub offer_request_id: String,
    pub offers: Vec<FlightOffer>,
    pub offer_count_returned: usize,
    pub offer_count_available: usize,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct FlightOffer {
    pub id: String,
    pub total_amount: String,
    pub total_currency: String,
    pub owner: FlightCarrier,
    pub expires_at: Option<String>,
    pub slices: Vec<FlightSlice>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct FlightCarrier {
    #[serde(default)]
    pub iata_code: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct FlightSlice {
    pub origin: Option<String>,
    pub destination: Option<String>,
    pub duration: Option<String>,
    pub segments: Vec<FlightSegment>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct FlightSegment {
    pub carrier: FlightCarrier,
    pub flight_number: Option<String>,
    pub origin: Option<String>,
    pub destination: Option<String>,
    pub departing_at: Option<String>,
    pub arriving_at: Option<String>,
    pub aircraft: Option<String>,
}

#[derive(Debug)]
pub struct SpeakVendorResponse {
    pub response: reqwest::Response,
}

pub struct VendorTarget {
    pub target: proxy_service::ProxyTarget,
}

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlatformCredentialSource {
    Platform,
    OwnConnection,
}

impl PlatformCredentialSource {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Platform => "platform",
            Self::OwnConnection => "own_connection",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OwnConnectionMetadata {
    pub user_service_id: String,
    pub slug: String,
    pub label: String,
    pub is_active: bool,
}

pub enum PlatformCredentialResolution {
    Platform {
        vendor: Box<DownstreamService>,
        disabled_connection: Option<OwnConnectionMetadata>,
    },
    OwnConnection {
        resolution: Box<proxy_service::UserServiceResolution>,
        connection: OwnConnectionMetadata,
    },
    NodeRouted {
        connection: OwnConnectionMetadata,
    },
    Unusable {
        connection: OwnConnectionMetadata,
        error: Option<AppError>,
    },
}

#[derive(Clone, Copy)]
pub enum CredentialResolutionMode<'a> {
    Execute {
        connection_expiry_notifier: Option<&'a ConnectionExpiryNotifier>,
    },
    Discover,
}

pub fn operation_name(op: PlatformOperationName) -> &'static str {
    match op {
        PlatformOperationName::XSearch => "x_search",
        PlatformOperationName::Speak => "speak",
        PlatformOperationName::CallAndSay => "call_and_say",
        PlatformOperationName::FlightSearch => "flight_search",
    }
}

pub fn parse_operation_name(value: &str) -> AppResult<PlatformOperationName> {
    match value {
        "x_search" => Ok(PlatformOperationName::XSearch),
        "speak" => Ok(PlatformOperationName::Speak),
        "call_and_say" => Ok(PlatformOperationName::CallAndSay),
        "flight_search" => Ok(PlatformOperationName::FlightSearch),
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
        PlatformOperationName::FlightSearch => {
            PlatformOperationConfig::FlightSearch(FlightSearchConfig {
                max_offers_cap: default_flight_search_max_offers_cap(),
                max_searches_per_user_per_day: default_flight_search_max_per_user_per_day(),
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
        (PlatformOperationName::FlightSearch, PlatformOperationConfig::FlightSearch(config)) => {
            validate_flight_search_config(config)?
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

fn validate_flight_search_config(config: &FlightSearchConfig) -> AppResult<()> {
    if !(1..=FLIGHT_SEARCH_HARD_MAX_OFFERS).contains(&config.max_offers_cap) {
        return Err(AppError::BadRequest(format!(
            "max_offers_cap must be between 1 and {FLIGHT_SEARCH_HARD_MAX_OFFERS}."
        )));
    }
    if config.max_searches_per_user_per_day == 0 {
        return Err(AppError::BadRequest(
            "max_searches_per_user_per_day must be at least 1.".to_string(),
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

pub fn is_twilio_account_sid(value: &str) -> bool {
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

pub fn x_search_path_for_base_url(base_url: &str) -> AppResult<&'static str> {
    let url = url::Url::parse(base_url)
        .map_err(|_| AppError::BadRequest("X connection has an invalid base URL.".to_string()))?;
    let has_version_segment = url
        .path_segments()
        .and_then(|mut segments| segments.rfind(|segment| !segment.is_empty()))
        == Some("2");
    Ok(if has_version_segment {
        "tweets/search/recent"
    } else {
        "2/tweets/search/recent"
    })
}

#[cfg(test)]
pub fn build_speak_request(
    config: &SpeakConfig,
    request: &SpeakRequest,
) -> AppResult<SpeakUpstreamRequest> {
    build_speak_request_for_source(config, request, true)
}

pub fn build_speak_request_for_source(
    config: &SpeakConfig,
    request: &SpeakRequest,
    enforce_platform_allowlist: bool,
) -> AppResult<SpeakUpstreamRequest> {
    if !is_safe_identifier(&request.voice_id, 128) {
        return Err(AppError::BadRequest(
            "voice_id must use only letters, digits, periods, hyphens, and underscores."
                .to_string(),
        ));
    }
    if enforce_platform_allowlist
        && !config
            .allowed_voice_ids
            .iter()
            .any(|voice_id| voice_id == &request.voice_id)
    {
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

#[cfg(test)]
pub fn build_call_and_say_request(
    config: &CallAndSayConfig,
    request: &CallAndSayRequest,
) -> AppResult<CallAndSayUpstreamRequest> {
    build_call_and_say_request_for_source(config, request, CallCredentialIdentity::Platform)
}

pub enum CallCredentialIdentity<'a> {
    Platform,
    OwnConnection { credential: &'a str },
}

pub fn build_call_and_say_request_for_source(
    config: &CallAndSayConfig,
    request: &CallAndSayRequest,
    identity: CallCredentialIdentity<'_>,
) -> AppResult<CallAndSayUpstreamRequest> {
    if !is_e164_number(&request.to) {
        return Err(AppError::BadRequest(
            "to must be a valid E.164 phone number.".to_string(),
        ));
    }

    let (account_sid, call_from) = match identity {
        CallCredentialIdentity::Platform => {
            if request.from.is_some() {
                return Err(AppError::BadRequest(
                    "`from` is not accepted when using the platform credential".to_string(),
                ));
            }
            if !destination_matches_prefixes(&request.to, &config.allowed_destination_prefixes) {
                return Err(AppError::BadRequest(
                    "Destination is not allowed for this platform operation.".to_string(),
                ));
            }
            (config.account_sid.clone(), config.call_from.clone())
        }
        CallCredentialIdentity::OwnConnection { credential } => {
            let from = request.from.as_deref().ok_or_else(|| {
                AppError::BadRequest(
                    "from is required when using your own Twilio connection.".to_string(),
                )
            })?;
            if !is_e164_number(from) {
                return Err(AppError::BadRequest(
                    "from must be a valid E.164 phone number.".to_string(),
                ));
            }
            let account_sid = credential.split_once(':').map(|(sid, _)| sid).unwrap_or("");
            if !is_twilio_account_sid(account_sid) {
                return Err(AppError::BadRequest(
                    "The stored Twilio credential must use AccountSID:AuthToken format."
                        .to_string(),
                ));
            }
            (account_sid.to_string(), from.to_string())
        }
    };

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
        path: format!("2010-04-01/Accounts/{account_sid}/Calls.json"),
        form: vec![
            ("To", request.to.clone()),
            ("From", call_from),
            ("Twiml", twiml),
        ],
    })
}

pub fn build_flight_search_request(
    config: &FlightSearchConfig,
    request: &FlightSearchRequest,
) -> AppResult<FlightSearchUpstreamRequest> {
    build_flight_search_request_at(config, request, Utc::now().date_naive())
}

fn build_flight_search_request_at(
    config: &FlightSearchConfig,
    request: &FlightSearchRequest,
    today: NaiveDate,
) -> AppResult<FlightSearchUpstreamRequest> {
    let origin = normalize_iata_code("origin", &request.origin)?;
    let destination = normalize_iata_code("destination", &request.destination)?;
    if origin == destination {
        return Err(AppError::BadRequest(
            "destination must differ from origin.".to_string(),
        ));
    }

    let departure_date = parse_flight_date("departure_date", &request.departure_date)?;
    let latest = today.checked_add_days(Days::new(365)).ok_or_else(|| {
        AppError::Internal("Unable to calculate the flight search date window".to_string())
    })?;
    if departure_date < today {
        return Err(AppError::BadRequest(
            "departure_date must not be in the past.".to_string(),
        ));
    }
    if departure_date > latest {
        return Err(AppError::BadRequest(
            "departure_date must be within 365 days.".to_string(),
        ));
    }

    let return_date = request
        .return_date
        .as_deref()
        .map(|value| parse_flight_date("return_date", value))
        .transpose()?;
    if let Some(return_date) = return_date {
        if return_date < departure_date {
            return Err(AppError::BadRequest(
                "return_date must be on or after departure_date.".to_string(),
            ));
        }
        if return_date > latest {
            return Err(AppError::BadRequest(
                "return_date must be within 365 days.".to_string(),
            ));
        }
    }

    let adults = request.adults.unwrap_or(1);
    if !(1..=9).contains(&adults) {
        return Err(AppError::BadRequest(
            "adults must be between 1 and 9.".to_string(),
        ));
    }

    let cabin_class = request
        .cabin_class
        .as_deref()
        .map(|value| value.trim().to_ascii_lowercase());
    if let Some(value) = cabin_class.as_deref()
        && !matches!(value, "economy" | "premium_economy" | "business" | "first")
    {
        return Err(AppError::BadRequest(
            "cabin_class must be economy, premium_economy, business, or first.".to_string(),
        ));
    }

    let configured_cap = config.max_offers_cap.min(FLIGHT_SEARCH_HARD_MAX_OFFERS);
    if configured_cap == 0 {
        return Err(AppError::PlatformOperationUnavailable);
    }
    let requested_offers = request.max_offers.unwrap_or(configured_cap);
    if requested_offers == 0 {
        return Err(AppError::BadRequest(
            "max_offers must be at least 1.".to_string(),
        ));
    }
    let max_offers = requested_offers.min(configured_cap);

    let mut slices = vec![serde_json::json!({
        "origin": origin,
        "destination": destination,
        "departure_date": departure_date.format("%Y-%m-%d").to_string(),
    })];
    if let Some(return_date) = return_date {
        slices.push(serde_json::json!({
            "origin": destination,
            "destination": origin,
            "departure_date": return_date.format("%Y-%m-%d").to_string(),
        }));
    }
    let passengers = (0..adults)
        .map(|_| serde_json::json!({ "type": "adult" }))
        .collect::<Vec<_>>();
    let mut data = serde_json::Map::from_iter([
        ("slices".to_string(), serde_json::Value::Array(slices)),
        (
            "passengers".to_string(),
            serde_json::Value::Array(passengers),
        ),
    ]);
    if let Some(cabin_class) = cabin_class {
        data.insert(
            "cabin_class".to_string(),
            serde_json::Value::String(cabin_class),
        );
    }

    Ok(FlightSearchUpstreamRequest {
        path: "air/offer_requests",
        query: "return_offers=true",
        body: serde_json::json!({ "data": data }),
        max_offers,
    })
}

fn normalize_iata_code(field: &str, value: &str) -> AppResult<String> {
    let normalized = value.trim().to_ascii_uppercase();
    if normalized.len() != 3 || !normalized.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(AppError::BadRequest(format!(
            "{field} must be a three-letter IATA code."
        )));
    }
    Ok(normalized)
}

fn parse_flight_date(field: &str, value: &str) -> AppResult<NaiveDate> {
    NaiveDate::parse_from_str(value.trim(), "%Y-%m-%d")
        .map_err(|_| AppError::BadRequest(format!("{field} must use YYYY-MM-DD format.")))
}

#[derive(Default, Deserialize)]
struct DuffelOfferRequestEnvelope {
    #[serde(default)]
    data: DuffelOfferRequestData,
}

#[derive(Default, Deserialize)]
struct DuffelOfferRequestData {
    #[serde(default)]
    id: String,
    #[serde(default)]
    offers: Vec<DuffelOffer>,
}

#[derive(Default, Deserialize)]
struct DuffelOffer {
    #[serde(default)]
    id: String,
    #[serde(default)]
    total_amount: String,
    #[serde(default)]
    total_currency: String,
    #[serde(default)]
    owner: FlightCarrier,
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    slices: Vec<DuffelSlice>,
}

#[derive(Default, Deserialize)]
struct DuffelSlice {
    #[serde(default)]
    origin: DuffelPlace,
    #[serde(default)]
    destination: DuffelPlace,
    #[serde(default)]
    duration: Option<String>,
    #[serde(default)]
    segments: Vec<DuffelSegment>,
}

#[derive(Default, Deserialize)]
struct DuffelSegment {
    #[serde(default)]
    marketing_carrier: FlightCarrier,
    #[serde(default)]
    flight_number: Option<String>,
    #[serde(default)]
    origin: DuffelPlace,
    #[serde(default)]
    destination: DuffelPlace,
    #[serde(default)]
    departing_at: Option<String>,
    #[serde(default)]
    arriving_at: Option<String>,
    #[serde(default)]
    aircraft: Option<DuffelAircraft>,
}

#[derive(Default, Deserialize)]
struct DuffelPlace {
    #[serde(default)]
    iata_code: Option<String>,
    #[serde(default)]
    name: Option<String>,
}

#[derive(Default, Deserialize)]
struct DuffelAircraft {
    #[serde(default)]
    name: Option<String>,
}

impl DuffelPlace {
    fn display_code(self) -> Option<String> {
        self.iata_code.or(self.name)
    }
}

pub fn project_flight_search_response(
    value: serde_json::Value,
    max_offers: u32,
) -> AppResult<FlightSearchResponse> {
    let payload: DuffelOfferRequestEnvelope = serde_json::from_value(value).map_err(|error| {
        tracing::error!(
            op = "flight_search",
            error = %error,
            "Duffel returned an invalid offer request response"
        );
        AppError::PlatformOperationUnavailable
    })?;
    let offer_count_available = payload.data.offers.len();
    let offers = payload
        .data
        .offers
        .into_iter()
        .take(max_offers.min(FLIGHT_SEARCH_HARD_MAX_OFFERS) as usize)
        .map(|offer| FlightOffer {
            id: offer.id,
            total_amount: offer.total_amount,
            total_currency: offer.total_currency,
            owner: offer.owner,
            expires_at: offer.expires_at,
            slices: offer
                .slices
                .into_iter()
                .map(|slice| FlightSlice {
                    origin: slice.origin.display_code(),
                    destination: slice.destination.display_code(),
                    duration: slice.duration,
                    segments: slice
                        .segments
                        .into_iter()
                        .map(|segment| FlightSegment {
                            carrier: segment.marketing_carrier,
                            flight_number: segment.flight_number,
                            origin: segment.origin.display_code(),
                            destination: segment.destination.display_code(),
                            departing_at: segment.departing_at,
                            arriving_at: segment.arriving_at,
                            aircraft: segment.aircraft.and_then(|aircraft| aircraft.name),
                        })
                        .collect(),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    Ok(FlightSearchResponse {
        offer_request_id: payload.data.id,
        offer_count_returned: offers.len(),
        offer_count_available,
        offers,
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

/// Resolve whether an operation should use a user-owned server credential or
/// the platform vendor credential. Both execution and discovery enter through
/// this function; discovery selects the read-only proxy resolver mode.
pub async fn resolve_operation_credential_source(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    node_ws_manager: &Arc<NodeWsManager>,
    resolution_user_id: &str,
    operation: &PlatformOperation,
    mode: CredentialResolutionMode<'_>,
) -> AppResult<PlatformCredentialResolution> {
    let contract = catalog_contract_for_operation(operation.op);
    let catalog_service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": contract.catalog_service_slug, "is_active": true })
        .await?;

    let Some(catalog_service) = catalog_service else {
        let vendor =
            resolve_platform_vendor_service(db, operation.op, &operation.vendor_service_slug)
                .await?;
        return Ok(PlatformCredentialResolution::Platform {
            vendor: Box::new(vendor),
            disabled_connection: None,
        });
    };

    let visible = user_service_service::list_user_services_with_sources_including_disabled(
        db,
        resolution_user_id,
    )
    .await?;
    let matching = visible
        .into_iter()
        .filter(|entry| {
            entry.service.catalog_service_id.as_deref() == Some(catalog_service.id.as_str())
        })
        .collect::<Vec<_>>();
    let active_candidate = matching
        .iter()
        .find(|entry| entry.service.is_active)
        .map(|entry| entry.service.clone());
    let disabled_connection = if active_candidate.is_none() {
        match matching.iter().find(|entry| !entry.service.is_active) {
            Some(entry) => Some(connection_metadata(db, &entry.service).await?),
            None => None,
        }
    } else {
        None
    };

    let Some(active_candidate) = active_candidate else {
        let vendor =
            resolve_platform_vendor_service(db, operation.op, &operation.vendor_service_slug)
                .await?;
        return Ok(PlatformCredentialResolution::Platform {
            vendor: Box::new(vendor),
            disabled_connection,
        });
    };
    let connection = connection_metadata(db, &active_candidate).await?;

    let resolved = match mode {
        CredentialResolutionMode::Execute {
            connection_expiry_notifier,
        } => {
            proxy_service::resolve_proxy_target_from_user_service(
                db,
                encryption_keys,
                node_ws_manager,
                resolution_user_id,
                None,
                Some(&catalog_service.id),
                connection_expiry_notifier,
            )
            .await
        }
        CredentialResolutionMode::Discover => {
            proxy_service::read_proxy_authority_snapshot_from_user_service(
                db,
                encryption_keys,
                resolution_user_id,
                None,
                Some(&catalog_service.id),
            )
            .await
        }
    };

    let resolution = match resolved {
        Ok(Some(resolution)) => resolution,
        Ok(None) => {
            return Ok(PlatformCredentialResolution::Unusable {
                connection,
                error: execution_mode_error(
                    mode,
                    AppError::Conflict(format!(
                        "Your {} connection could not be resolved.",
                        contract.vendor
                    )),
                ),
            });
        }
        Err(error) => {
            let error = normalize_own_connection_error(contract.vendor, error);
            return Ok(PlatformCredentialResolution::Unusable {
                connection,
                error: execution_mode_error(mode, error),
            });
        }
    };

    if resolution.master_credential || resolution.api_key_id.is_none() {
        let vendor =
            resolve_platform_vendor_service(db, operation.op, &operation.vendor_service_slug)
                .await?;
        return Ok(PlatformCredentialResolution::Platform {
            vendor: Box::new(vendor),
            disabled_connection: None,
        });
    }
    if resolution
        .node_id
        .as_deref()
        .is_some_and(|id| !id.is_empty())
        || !resolution.has_server_credential
    {
        return Ok(PlatformCredentialResolution::NodeRouted { connection });
    }

    Ok(PlatformCredentialResolution::OwnConnection {
        resolution: Box::new(resolution),
        connection,
    })
}

fn execution_mode_error(mode: CredentialResolutionMode<'_>, error: AppError) -> Option<AppError> {
    matches!(mode, CredentialResolutionMode::Execute { .. }).then_some(error)
}

fn normalize_own_connection_error(vendor: &str, error: AppError) -> AppError {
    let contextualize =
        |message: String| format!("Your {vendor} connection is unusable. {message}");
    match error {
        AppError::Internal(_) => AppError::Conflict(format!(
            "Your {vendor} connection is unusable. Update or disable it before retrying."
        )),
        AppError::BadRequest(message) => AppError::BadRequest(contextualize(message)),
        AppError::ValidationError(message) => AppError::ValidationError(contextualize(message)),
        AppError::Unauthorized(message) => AppError::Unauthorized(contextualize(message)),
        AppError::Forbidden(message) => AppError::Forbidden(contextualize(message)),
        AppError::NotFound(message) => AppError::NotFound(contextualize(message)),
        AppError::Conflict(message) => AppError::Conflict(contextualize(message)),
        AppError::AuthenticationFailed(message) => {
            AppError::AuthenticationFailed(contextualize(message))
        }
        other => other,
    }
}

async fn connection_metadata(
    db: &mongodb::Database,
    service: &crate::models::user_service::UserService,
) -> AppResult<OwnConnectionMetadata> {
    let label = db
        .collection::<UserEndpoint>(USER_ENDPOINTS)
        .find_one(doc! { "_id": &service.endpoint_id })
        .await?
        .map(|endpoint| endpoint.label)
        .unwrap_or_else(|| service.slug.clone());
    Ok(OwnConnectionMetadata {
        user_service_id: service.id.clone(),
        slug: service.slug.clone(),
        label,
        is_active: service.is_active,
    })
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
    backfill_platform_vendor_kill_policy_for_slug(db, &vendor_service_slug).await?;
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

pub async fn backfill_platform_vendor_kill_policies(db: &mongodb::Database) -> AppResult<u64> {
    let slugs = PLATFORM_OPERATION_VENDOR_CONTRACTS
        .iter()
        .map(|contract| contract.slug)
        .collect::<Vec<_>>();
    backfill_platform_vendor_kill_policy_for_slugs(db, &slugs).await
}

async fn backfill_platform_vendor_kill_policy_for_slug(
    db: &mongodb::Database,
    slug: &str,
) -> AppResult<u64> {
    if !is_platform_vendor_slug(slug) {
        return Ok(0);
    }
    backfill_platform_vendor_kill_policy_for_slugs(db, &[slug]).await
}

async fn backfill_platform_vendor_kill_policy_for_slugs(
    db: &mongodb::Database,
    slugs: &[&str],
) -> AppResult<u64> {
    let collection = db.collection::<mongodb::bson::Document>(DOWNSTREAM_SERVICES);
    let rows: Vec<mongodb::bson::Document> = collection
        .find(doc! {
            "slug": { "$in": slugs },
            "$or": [
                { "proxy_operation_policy": { "$exists": false } },
                { "proxy_operation_policy": bson::Bson::Null },
            ],
        })
        .await?
        .try_collect()
        .await?;
    let policy = bson::to_bson(&platform_vendor_kill_policy()).map_err(|error| {
        AppError::Internal(format!(
            "Failed to serialize platform vendor kill policy: {error}"
        ))
    })?;
    let mut modified = 0_u64;
    for row in rows {
        let Some(id) = row.get_str("_id").ok() else {
            continue;
        };
        let slug = row.get_str("slug").unwrap_or("unknown");
        let result = collection
            .update_one(
                doc! {
                    "_id": id,
                    "$or": [
                        { "proxy_operation_policy": { "$exists": false } },
                        { "proxy_operation_policy": bson::Bson::Null },
                    ],
                },
                doc! { "$set": { "proxy_operation_policy": policy.clone() } },
            )
            .await?;
        if result.modified_count == 1 {
            modified += 1;
            tracing::warn!(
                service_id = id,
                service_slug = slug,
                "Backfilled deny-all policy on platform vendor row"
            );
        }
    }
    Ok(modified)
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

#[allow(clippy::too_many_arguments)]
pub async fn forward_operation_request(
    http_client: &reqwest::Client,
    target: &proxy_service::ProxyTarget,
    op: PlatformOperationName,
    method: reqwest::Method,
    path: &str,
    query: Option<&str>,
    headers: HeaderMap,
    body: Option<bytes::Bytes>,
    token_exchange_cache: &crate::services::provider_token_exchange_service::TokenExchangeCache,
    cloud_response_cache: &crate::services::cloud_response_cache::CloudResponseCache,
    billing_egress_permit: BillingEgressPermit,
) -> AppResult<reqwest::Response> {
    proxy_service::forward_request(
        http_client,
        target,
        method,
        path,
        query,
        headers,
        proxy_service::ProxyBody::Buffered(body),
        Vec::new(),
        Vec::new(),
        None,
        token_exchange_cache,
        cloud_response_cache,
        billing_egress_permit,
    )
    .await
    .map_err(|error| {
        tracing::error!(
            op = operation_name(op),
            error = %error,
            "Platform operation vendor request failed"
        );
        AppError::PlatformOperationUnavailable
    })
}

pub fn json_request_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));
    headers
}

pub fn form_request_headers() -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("application/x-www-form-urlencoded"),
    );
    headers
}

pub fn duffel_request_headers() -> HeaderMap {
    let mut headers = json_request_headers();
    headers.insert("duffel-version", HeaderValue::from_static("v2"));
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("gzip"));
    headers
}

pub async fn resolve_platform_vendor_service(
    db: &mongodb::Database,
    op: PlatformOperationName,
    slug: &str,
) -> AppResult<DownstreamService> {
    let requirement = vendor_requirement_for_operation(op);
    let service = assistant_service::resolve_admin_service_by_slug(db, slug)
        .await
        .map_err(|error| vendor_configuration_failed(op, error))?;
    validate_vendor_binding_shape(requirement, slug, Some(&service), false)
        .map_err(|error| vendor_configuration_failed(op, error))?;
    Ok(service)
}

pub async fn materialize_platform_vendor_target(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    op: PlatformOperationName,
    service: DownstreamService,
) -> AppResult<VendorTarget> {
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

    let catalog_default_headers = service.default_request_headers.clone().unwrap_or_default();
    Ok(VendorTarget {
        target: proxy_service::ProxyTarget {
            base_url: service.base_url.clone(),
            auth_method: service.auth_method.clone(),
            auth_key_name: service.auth_key_name.clone(),
            credential: credential.to_string(),
            service,
            catalog_default_headers,
            user_service_default_headers: Vec::new(),
            ws_frame_injections: Vec::new(),
            connection_id: None,
        },
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
    match service.proxy_operation_policy.as_ref() {
        None => {
            return Err(AppError::PlatformVendorProvisioningInvalid(format!(
                "{op} requires the actor-addressed deny-all kill policy; row '{}' has no proxy_operation_policy",
                service.slug
            )));
        }
        Some(policy) if !policy.rules.is_empty() => {
            return Err(AppError::PlatformVendorProvisioningInvalid(format!(
                "{op} requires an empty actor-addressed deny-all kill policy; row '{}' has {} rule(s)",
                service.slug,
                policy.rules.len()
            )));
        }
        Some(_) => {}
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

pub fn ensure_vendor_success(
    op: PlatformOperationName,
    response: &reqwest::Response,
) -> AppResult<()> {
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

pub async fn read_vendor_json(
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

pub async fn reserve_daily_operation(
    db: &mongodb::Database,
    op: PlatformOperationName,
    user_id: &str,
    yyyymmdd: &str,
    cap: u32,
) -> AppResult<()> {
    let collection = db.collection::<PlatformOpUsage>(PLATFORM_OP_USAGE);
    let filter = doc! {
        "op": operation_name(op),
        "user_id": user_id,
        "yyyymmdd": yyyymmdd,
        "count": { "$lt": i64::from(cap) },
    };
    let update = doc! {
        "$setOnInsert": {
            "_id": uuid::Uuid::new_v4().to_string(),
            "op": operation_name(op),
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

pub async fn release_daily_operation(
    db: &mongodb::Database,
    op: PlatformOperationName,
    user_id: &str,
    yyyymmdd: &str,
) -> AppResult<()> {
    let collection = db.collection::<PlatformOpUsage>(PLATFORM_OP_USAGE);
    collection
        .update_one(
            doc! {
                "op": operation_name(op),
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
    collection
        .delete_one(doc! {
            "op": operation_name(op),
            "user_id": user_id,
            "yyyymmdd": yyyymmdd,
            "count": { "$lte": 0_i64 },
        })
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
    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};

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
        service.proxy_operation_policy = Some(platform_vendor_kill_policy());
        service
    }

    fn provisioning_message(error: AppError) -> String {
        let AppError::PlatformVendorProvisioningInvalid(message) = error else {
            panic!("expected a platform vendor provisioning error, got {error:?}");
        };
        message
    }

    fn test_user_key(id: &str, user_id: &str, credential_encrypted: Option<Vec<u8>>) -> UserApiKey {
        UserApiKey {
            id: id.to_string(),
            user_id: user_id.to_string(),
            label: "Own ElevenLabs key".to_string(),
            credential_type: "api_key".to_string(),
            credential_encrypted,
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: None,
            connection_id: None,
            oauth_attempt_nonce: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            credential_source: None,
            status: "active".to_string(),
            last_used_at: None,
            last_authorized_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            credential_epoch: 1,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn operation_contracts_and_seeded_templates_are_consistent() {
        assert_eq!(PLATFORM_OPERATION_VENDOR_CONTRACTS.len(), 4);
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
            let operation = template
                .operation
                .expect("every platform vendor template binds a shipped operation");
            let operation = parse_operation_name(operation).expect("seeded operation name");
            let contract = vendor_requirement_for_operation(operation);
            assert_eq!(template.auth_method, contract.auth_method);
            assert_eq!(template.auth_key_name, contract.auth_key_name);
        }
    }

    #[tokio::test]
    async fn kill_policy_backfill_changes_only_canonical_vendor_policy_fields() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_vendor_policy_backfill").await
        else {
            eprintln!("skipping platform vendor backfill test: no local MongoDB available");
            return;
        };
        let mut platform = valid_speak_vendor_service();
        platform.id = uuid::Uuid::new_v4().to_string();
        platform.proxy_operation_policy = None;
        platform.credential_encrypted = vec![9, 8, 7, 6];
        let mut unrelated = platform.clone();
        unrelated.id = uuid::Uuid::new_v4().to_string();
        unrelated.slug = "internal-unrelated".to_string();
        unrelated.credential_encrypted = vec![1, 2, 3, 4];
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_many([platform.clone(), unrelated.clone()])
            .await
            .expect("insert backfill fixtures");

        assert_eq!(
            backfill_platform_vendor_kill_policies(&db)
                .await
                .expect("backfill platform policies"),
            1
        );

        let platform_after = db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "_id": &platform.id })
            .await
            .expect("read platform vendor")
            .expect("platform vendor exists");
        assert!(
            platform_after
                .proxy_operation_policy
                .as_ref()
                .is_some_and(|policy| policy.rules.is_empty())
        );
        assert_eq!(
            platform_after.credential_encrypted,
            platform.credential_encrypted
        );

        let unrelated_after = db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find_one(doc! { "_id": &unrelated.id })
            .await
            .expect("read unrelated row")
            .expect("unrelated row exists");
        assert!(unrelated_after.proxy_operation_policy.is_none());
        assert_eq!(
            unrelated_after.credential_encrypted,
            unrelated.credential_encrypted
        );
    }

    #[tokio::test]
    async fn credential_source_resolution_covers_every_connection_state() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_credential_resolution").await
        else {
            eprintln!("skipping platform credential resolution test: no local MongoDB available");
            return;
        };
        let encryption_keys = crate::test_utils::test_encryption_keys();
        let user_id = uuid::Uuid::new_v4().to_string();
        let catalog_id = uuid::Uuid::new_v4().to_string();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let user_service_id = uuid::Uuid::new_v4().to_string();
        let api_key_id = uuid::Uuid::new_v4().to_string();
        let node_ws_manager = Arc::new(NodeWsManager::new(30, 100));

        let mut vendor = valid_speak_vendor_service();
        vendor.id = uuid::Uuid::new_v4().to_string();
        vendor.credential_encrypted = encryption_keys
            .encrypt(b"platform-elevenlabs-secret")
            .await
            .expect("encrypt platform credential");
        let mut catalog = dummy_service();
        catalog.id = catalog_id.clone();
        catalog.slug = "api-elevenlabs".to_string();
        catalog.name = "ElevenLabs".to_string();
        catalog.base_url = "https://api.elevenlabs.io".to_string();
        catalog.service_category = "connection".to_string();
        catalog.auth_method = "header".to_string();
        catalog.auth_key_name = "xi-api-key".to_string();
        catalog.requires_user_credential = true;
        catalog.credential_encrypted = Vec::new();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_many([vendor, catalog])
            .await
            .expect("insert vendor catalogs");

        let operation = PlatformOperation {
            id: uuid::Uuid::new_v4().to_string(),
            op: PlatformOperationName::Speak,
            enabled: true,
            vendor_service_slug: SPEAK_VENDOR_SLUG.to_string(),
            config: PlatformOperationConfig::Speak(speak_config()),
            updated_at: Utc::now(),
            updated_by: "admin".to_string(),
        };
        let resolve = |mode| {
            resolve_operation_credential_source(
                &db,
                &encryption_keys,
                &node_ws_manager,
                &user_id,
                &operation,
                mode,
            )
        };

        assert!(matches!(
            resolve(CredentialResolutionMode::Discover)
                .await
                .expect("resolve no connection"),
            PlatformCredentialResolution::Platform {
                disabled_connection: None,
                ..
            }
        ));

        let endpoint = crate::test_utils::test_user_endpoint(
            &endpoint_id,
            &user_id,
            "My ElevenLabs",
            "https://api.elevenlabs.io",
            None,
            Some(&catalog_id),
        );
        let mut user_service = crate::test_utils::test_user_service(
            &user_service_id,
            &user_id,
            "my-elevenlabs",
            &endpoint_id,
            Some(&catalog_id),
            None,
        );
        user_service.is_active = false;
        user_service.auth_method = "header".to_string();
        user_service.auth_key_name = "xi-api-key".to_string();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .expect("insert user endpoint");
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&user_service)
            .await
            .expect("insert disabled user service");

        match resolve(CredentialResolutionMode::Discover)
            .await
            .expect("resolve disabled connection")
        {
            PlatformCredentialResolution::Platform {
                disabled_connection: Some(connection),
                ..
            } => {
                assert_eq!(connection.user_service_id, user_service_id);
                assert!(!connection.is_active);
            }
            _ => panic!("disabled connection must select platform credentials"),
        }

        let encrypted = encryption_keys
            .encrypt(b"own-elevenlabs-secret")
            .await
            .expect("encrypt own credential");
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(test_user_key(&api_key_id, &user_id, Some(encrypted)))
            .await
            .expect("insert own key");
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &user_service_id },
                doc! {
                    "$set": {
                        "is_active": true,
                        "api_key_id": &api_key_id,
                        "auth_method": "header",
                        "auth_key_name": "xi-api-key",
                    }
                },
            )
            .await
            .expect("activate own connection");

        match resolve(CredentialResolutionMode::Execute {
            connection_expiry_notifier: None,
        })
        .await
        .expect("resolve own connection")
        {
            PlatformCredentialResolution::OwnConnection { resolution, .. } => {
                assert_eq!(resolution.api_key_id.as_deref(), Some(api_key_id.as_str()));
                assert_eq!(resolution.target.credential, "own-elevenlabs-secret");
                assert!(resolution.has_server_credential);
            }
            _ => panic!("active server-held key must select the own connection"),
        }

        db.collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                doc! { "_id": &api_key_id },
                doc! { "$set": { "status": "revoked" } },
            )
            .await
            .expect("revoke own key");
        match resolve(CredentialResolutionMode::Execute {
            connection_expiry_notifier: None,
        })
        .await
        .expect("classify unusable connection")
        {
            PlatformCredentialResolution::Unusable {
                error: Some(AppError::BadRequest(message)),
                ..
            } => {
                assert!(message.starts_with("Your elevenlabs connection is unusable."));
                assert!(message.contains("revoked"));
            }
            _ => panic!("revoked active key must be unusable without platform fallback"),
        }

        db.collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                doc! { "_id": &api_key_id },
                doc! {
                    "$set": { "status": "active", "credential_type": "node_managed" },
                    "$unset": { "credential_encrypted": "" },
                },
            )
            .await
            .expect("make key node-managed");
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &user_service_id },
                doc! { "$set": { "node_id": "node-1" } },
            )
            .await
            .expect("route connection through node");
        assert!(matches!(
            resolve(CredentialResolutionMode::Execute {
                connection_expiry_notifier: None,
            })
            .await
            .expect("resolve node connection"),
            PlatformCredentialResolution::NodeRouted { .. }
        ));

        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &user_service_id },
                doc! {
                    "$set": { "auth_method": "none" },
                    "$unset": { "api_key_id": "", "node_id": "" },
                },
            )
            .await
            .expect("make active row credentialless");
        assert!(matches!(
            resolve(CredentialResolutionMode::Discover)
                .await
                .expect("resolve credentialless connection"),
            PlatformCredentialResolution::Platform {
                disabled_connection: None,
                ..
            }
        ));
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
            (
                "speak requires the actor-addressed deny-all kill policy; row 'platform-elevenlabs' has no proxy_operation_policy",
                SPEAK_VENDOR_SLUG,
                Some(crate::models::downstream_service::DownstreamService {
                    proxy_operation_policy: None,
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

        let flight = PlatformOperationConfig::FlightSearch(FlightSearchConfig {
            max_offers_cap: FLIGHT_SEARCH_HARD_MAX_OFFERS + 1,
            max_searches_per_user_per_day: 20,
        });
        assert!(
            validate_operation_config(
                PlatformOperationName::FlightSearch,
                FLIGHT_SEARCH_VENDOR_SLUG,
                &flight
            )
            .is_err()
        );
    }

    fn flight_request() -> FlightSearchRequest {
        FlightSearchRequest {
            origin: " sin ".to_string(),
            destination: "lhr".to_string(),
            departure_date: "2026-09-01".to_string(),
            return_date: Some("2026-09-08".to_string()),
            adults: Some(2),
            cabin_class: Some("business".to_string()),
            max_offers: Some(40),
        }
    }

    #[test]
    fn flight_search_builder_normalizes_and_bounds_server_composed_request() {
        let upstream = build_flight_search_request_at(
            &FlightSearchConfig {
                max_offers_cap: 10,
                max_searches_per_user_per_day: 20,
            },
            &flight_request(),
            NaiveDate::from_ymd_opt(2026, 8, 27).expect("valid test date"),
        )
        .expect("valid flight search");

        assert_eq!(upstream.path, "air/offer_requests");
        assert_eq!(upstream.query, "return_offers=true");
        assert_eq!(upstream.max_offers, 10);
        assert_eq!(
            upstream.body,
            serde_json::json!({
                "data": {
                    "slices": [
                        { "origin": "SIN", "destination": "LHR", "departure_date": "2026-09-01" },
                        { "origin": "LHR", "destination": "SIN", "departure_date": "2026-09-08" }
                    ],
                    "passengers": [{ "type": "adult" }, { "type": "adult" }],
                    "cabin_class": "business"
                }
            })
        );
    }

    #[test]
    fn flight_search_builder_enforces_dates_codes_and_bounds() {
        let config = FlightSearchConfig {
            max_offers_cap: 10,
            max_searches_per_user_per_day: 20,
        };
        let today = NaiveDate::from_ymd_opt(2026, 8, 27).expect("valid test date");
        let mut request = flight_request();

        request.origin = "S1N".to_string();
        assert!(build_flight_search_request_at(&config, &request, today).is_err());
        request = flight_request();
        request.destination = "sin".to_string();
        assert!(build_flight_search_request_at(&config, &request, today).is_err());
        request = flight_request();
        request.departure_date = "2026-08-26".to_string();
        assert!(build_flight_search_request_at(&config, &request, today).is_err());
        request = flight_request();
        request.departure_date = "2027-08-28".to_string();
        request.return_date = None;
        assert!(build_flight_search_request_at(&config, &request, today).is_err());
        request = flight_request();
        request.return_date = Some("2026-08-31".to_string());
        assert!(build_flight_search_request_at(&config, &request, today).is_err());
        request = flight_request();
        request.adults = Some(10);
        assert!(build_flight_search_request_at(&config, &request, today).is_err());
        request = flight_request();
        request.cabin_class = Some("private_jet".to_string());
        assert!(build_flight_search_request_at(&config, &request, today).is_err());
        request = flight_request();
        request.max_offers = Some(0);
        assert!(build_flight_search_request_at(&config, &request, today).is_err());
    }

    #[test]
    fn flight_search_projection_is_bounded_and_tolerates_missing_optional_fields() {
        let projected = project_flight_search_response(
            serde_json::json!({
                "data": {
                    "id": "orq_123",
                    "offers": [
                        {
                            "id": "off_1",
                            "total_amount": "123.45",
                            "total_currency": "SGD",
                            "owner": { "iata_code": "SQ", "name": "Singapore Airlines" },
                            "expires_at": "2026-08-27T12:00:00Z",
                            "slices": [{
                                "origin": { "iata_code": "SIN" },
                                "destination": { "iata_code": "LHR" },
                                "duration": "PT13H30M",
                                "segments": [{
                                    "marketing_carrier": { "iata_code": "SQ", "name": "Singapore Airlines" },
                                    "flight_number": "322",
                                    "origin": { "iata_code": "SIN" },
                                    "destination": { "iata_code": "LHR" },
                                    "departing_at": "2026-09-01T23:30:00",
                                    "arriving_at": "2026-09-02T05:55:00",
                                    "aircraft": { "name": "Airbus A380" },
                                    "ignored_vendor_field": { "large": true }
                                }],
                                "ignored_vendor_field": true
                            }]
                        },
                        { "id": "off_2" },
                        { "id": "off_3" }
                    ],
                    "ignored_vendor_field": [1, 2, 3]
                }
            }),
            2,
        )
        .expect("valid Duffel projection");

        assert_eq!(projected.offer_request_id, "orq_123");
        assert_eq!(projected.offer_count_available, 3);
        assert_eq!(projected.offer_count_returned, 2);
        assert_eq!(projected.offers[0].owner.iata_code.as_deref(), Some("SQ"));
        assert_eq!(projected.offers[0].slices[0].origin.as_deref(), Some("SIN"));
        assert_eq!(
            projected.offers[0].slices[0].segments[0]
                .aircraft
                .as_deref(),
            Some("Airbus A380")
        );
        assert_eq!(projected.offers[1].id, "off_2");
        assert!(projected.offers[1].expires_at.is_none());
    }

    #[test]
    fn empty_destination_prefixes_deny_every_destination() {
        let config = call_config(Vec::new());
        let request = CallAndSayRequest {
            to: "+16505550199".to_string(),
            message: "Hello".to_string(),
            from: None,
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
    fn x_search_path_respects_catalog_base_versioning() {
        assert_eq!(
            x_search_path_for_base_url("https://api.x.com").expect("platform X path"),
            "2/tweets/search/recent"
        );
        assert_eq!(
            x_search_path_for_base_url("https://api.x.com/2").expect("own X path"),
            "tweets/search/recent"
        );
        assert_eq!(
            x_search_path_for_base_url("https://api.x.com/2/").expect("own X trailing slash path"),
            "tweets/search/recent"
        );
    }

    #[test]
    fn call_source_controls_require_exactly_the_applicable_caller_identity() {
        let config = call_config(vec!["+1".to_string()]);
        let platform_with_from = build_call_and_say_request_for_source(
            &config,
            &CallAndSayRequest {
                to: "+14155550100".to_string(),
                message: "Hello".to_string(),
                from: Some("+14155550101".to_string()),
            },
            CallCredentialIdentity::Platform,
        )
        .expect_err("platform calls must reject caller-provided from");
        assert!(matches!(
            platform_with_from,
            AppError::BadRequest(message)
                if message == "`from` is not accepted when using the platform credential"
        ));

        let own_without_from = build_call_and_say_request_for_source(
            &config,
            &CallAndSayRequest {
                to: "+6512345678".to_string(),
                message: "Hello".to_string(),
                from: None,
            },
            CallCredentialIdentity::OwnConnection {
                credential: "ACaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:token",
            },
        )
        .expect_err("own calls must require from");
        assert!(matches!(
            own_without_from,
            AppError::BadRequest(message) if message.contains("from is required")
        ));

        let own = build_call_and_say_request_for_source(
            &config,
            &CallAndSayRequest {
                to: "+6512345678".to_string(),
                message: "Hello".to_string(),
                from: Some("+14155550101".to_string()),
            },
            CallCredentialIdentity::OwnConnection {
                credential: "ACaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa:token:with:colons",
            },
        )
        .expect("own calls use the SID before the first colon");
        assert_eq!(
            own.path,
            "2010-04-01/Accounts/ACaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa/Calls.json"
        );
        assert!(own.form.contains(&("From", "+14155550101".to_string())));
    }

    #[test]
    fn twiml_composition_escapes_xml_and_cdata_terminators() {
        let config = call_config(vec!["+65".to_string()]);
        let request = CallAndSayRequest {
            to: "+6512345678".to_string(),
            message: "A & B <tag> \"quoted\" 'single' ]]> done".to_string(),
            from: None,
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
            from: None,
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
        assert!(
            serde_json::from_value::<FlightSearchRequest>(serde_json::json!({
                "origin": "SIN",
                "destination": "LHR",
                "departure_date": "2026-09-01",
                "create_order": true,
            }))
            .is_err()
        );
    }
}
