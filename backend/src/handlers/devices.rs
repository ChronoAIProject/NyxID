use axum::{Json, extract::State};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use serde::Deserialize;
use serde_json::json;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::audit_service;
use crate::services::device_code_service::{
    DeviceCodeApprove, DeviceCodeApproveInput, DeviceCodeInitiate, DeviceCodeInitiateInput,
    DeviceCodePoll, DeviceCodePollInput, approve, initiate, poll,
};

#[derive(Debug, Deserialize)]
pub struct RequestDeviceCodeRequest {
    pub device_pubkey: String,
    pub hw_id: String,
    #[serde(default)]
    pub suggested_label: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PollDeviceCodeRequest {
    pub device_code: String,
    pub timestamp: i64,
    pub signature: String,
}

#[derive(Debug, Deserialize)]
pub struct ApproveDeviceCodeRequest {
    pub user_code: String,
    #[serde(default)]
    pub org_id: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

pub async fn request_device_code(
    State(state): State<AppState>,
    Json(req): Json<RequestDeviceCodeRequest>,
) -> AppResult<Json<DeviceCodeInitiate>> {
    let device_pubkey = decode_device_pubkey(&req.device_pubkey)?;
    let hw_id = normalize_hw_id(&req.hw_id)?;
    let suggested_label = normalize_suggested_label(req.suggested_label)?;

    let response = initiate(
        &state.db,
        DeviceCodeInitiateInput {
            device_pubkey,
            hw_id,
            suggested_label,
            frontend_url: state.config.frontend_url.clone(),
        },
    )
    .await?;

    Ok(Json(response))
}

pub async fn poll_device_code(
    State(state): State<AppState>,
    Json(req): Json<PollDeviceCodeRequest>,
) -> AppResult<Json<DeviceCodePoll>> {
    let device_code = normalize_device_code(&req.device_code)?;
    let signature = decode_poll_signature(&req.signature)?;
    let response = poll(
        &state.db,
        DeviceCodePollInput {
            device_code,
            timestamp: req.timestamp,
            signature,
        },
    )
    .await?;

    Ok(Json(response))
}

pub async fn approve_device_code(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(req): Json<ApproveDeviceCodeRequest>,
) -> AppResult<Json<DeviceCodeApprove>> {
    let user_code = normalize_user_code(&req.user_code)?;
    let label = normalize_label(req.label)?;
    let org_id = normalize_org_id(req.org_id)?;

    let response = approve(
        &state.db,
        &auth_user.user_id.to_string(),
        DeviceCodeApproveInput {
            user_code,
            org_id,
            label,
        },
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "device_code_approved",
        Some(json!({
            "api_key_id": response.api_key_id,
            "node_id": response.node_id,
            "owner_user_id": response.owner_user_id,
            "org_id": response.org_id,
            "hw_id": response.hw_id,
            "device_label": response.device_label,
        })),
    );

    Ok(Json(response))
}

fn normalize_device_code(value: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.len() != 64 || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::BadRequest(
            "device_code must be 64 hex characters".to_string(),
        ));
    }
    Ok(trimmed.to_ascii_lowercase())
}

fn normalize_user_code(value: &str) -> AppResult<String> {
    let compact = value
        .chars()
        .filter(|c| !c.is_ascii_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase();

    if compact.len() != 12 || !compact.bytes().all(is_user_code_byte) {
        return Err(AppError::DeviceUserCodeInvalid);
    }

    Ok(format!(
        "{}-{}-{}",
        &compact[0..4],
        &compact[4..8],
        &compact[8..12]
    ))
}

fn is_user_code_byte(byte: u8) -> bool {
    matches!(
        byte,
        b'A' | b'B'
            | b'C'
            | b'D'
            | b'E'
            | b'F'
            | b'G'
            | b'H'
            | b'J'
            | b'K'
            | b'L'
            | b'M'
            | b'N'
            | b'P'
            | b'Q'
            | b'R'
            | b'S'
            | b'T'
            | b'U'
            | b'V'
            | b'W'
            | b'X'
            | b'Y'
            | b'Z'
            | b'2'
            | b'3'
            | b'4'
            | b'5'
            | b'6'
            | b'7'
            | b'8'
            | b'9'
    )
}

fn decode_poll_signature(value: &str) -> AppResult<[u8; 64]> {
    let decoded = BASE64_STANDARD
        .decode(value)
        .map_err(|_| AppError::BadRequest("signature must be valid base64".to_string()))?;
    decoded
        .try_into()
        .map_err(|_| AppError::BadRequest("signature must decode to 64 bytes".to_string()))
}

fn decode_device_pubkey(value: &str) -> AppResult<[u8; 32]> {
    let decoded = BASE64_STANDARD
        .decode(value)
        .map_err(|_| AppError::BadRequest("device_pubkey must be valid base64".to_string()))?;
    decoded
        .try_into()
        .map_err(|_| AppError::BadRequest("device_pubkey must decode to 32 bytes".to_string()))
}

fn normalize_hw_id(value: &str) -> AppResult<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.len() > 256 {
        return Err(AppError::BadRequest(
            "hw_id must be between 1 and 256 characters".to_string(),
        ));
    }
    Ok(trimmed.to_string())
}

fn normalize_suggested_label(value: Option<String>) -> AppResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 256 {
        return Err(AppError::BadRequest(
            "suggested_label must be at most 256 characters".to_string(),
        ));
    }
    Ok(Some(trimmed.to_string()))
}

fn normalize_label(value: Option<String>) -> AppResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.len() > 200 {
        return Err(AppError::BadRequest(
            "label must be at most 200 characters".to_string(),
        ));
    }
    Ok(Some(trimmed.to_string()))
}

fn normalize_org_id(value: Option<String>) -> AppResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    uuid::Uuid::parse_str(trimmed)
        .map_err(|_| AppError::BadRequest("org_id must be a UUID".to_string()))?;
    Ok(Some(trimmed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_device_pubkey_accepts_exactly_32_base64_bytes() {
        let encoded = BASE64_STANDARD.encode([5u8; 32]);

        assert_eq!(decode_device_pubkey(&encoded).unwrap(), [5u8; 32]);
    }

    #[test]
    fn decode_device_pubkey_rejects_invalid_base64() {
        let error = decode_device_pubkey("not base64").expect_err("invalid");

        assert!(matches!(error, AppError::BadRequest(_)));
    }

    #[test]
    fn decode_device_pubkey_rejects_wrong_length() {
        let encoded = BASE64_STANDARD.encode([5u8; 31]);
        let error = decode_device_pubkey(&encoded).expect_err("wrong length");

        assert!(matches!(error, AppError::BadRequest(_)));
    }

    #[test]
    fn normalize_device_code_accepts_64_hex_chars_and_lowercases() {
        let raw = "AABBCCDD".repeat(8);

        assert_eq!(
            normalize_device_code(&raw).unwrap(),
            raw.to_ascii_lowercase()
        );
    }

    #[test]
    fn normalize_device_code_rejects_wrong_shape() {
        assert!(normalize_device_code("abc").is_err());
        assert!(normalize_device_code(&"z".repeat(64)).is_err());
    }

    #[test]
    fn normalize_user_code_accepts_spaces_dashes_and_lowercase() {
        assert_eq!(
            normalize_user_code("abcd efgh jklm").unwrap(),
            "ABCD-EFGH-JKLM"
        );
        assert_eq!(
            normalize_user_code("abcd-efgh-jklm").unwrap(),
            "ABCD-EFGH-JKLM"
        );
    }

    #[test]
    fn normalize_user_code_rejects_ambiguous_or_wrong_length_input() {
        assert!(matches!(
            normalize_user_code("ABCD-EFGH-JKL").expect_err("short"),
            AppError::DeviceUserCodeInvalid
        ));
        assert!(matches!(
            normalize_user_code("ABCD-EFGH-IJKL").expect_err("ambiguous"),
            AppError::DeviceUserCodeInvalid
        ));
        assert!(matches!(
            normalize_user_code("ABCD-EFGH-OJKL").expect_err("ambiguous"),
            AppError::DeviceUserCodeInvalid
        ));
    }

    #[test]
    fn decode_poll_signature_accepts_exactly_64_base64_bytes() {
        let encoded = BASE64_STANDARD.encode([8u8; 64]);

        assert_eq!(decode_poll_signature(&encoded).unwrap(), [8u8; 64]);
    }

    #[test]
    fn decode_poll_signature_rejects_wrong_length() {
        let encoded = BASE64_STANDARD.encode([8u8; 63]);

        assert!(decode_poll_signature(&encoded).is_err());
    }

    #[test]
    fn normalize_hw_id_trims_and_bounds_length() {
        assert_eq!(normalize_hw_id(" esp32 ").unwrap(), "esp32");
        assert!(normalize_hw_id("").is_err());
        assert!(normalize_hw_id(&"x".repeat(257)).is_err());
    }

    #[test]
    fn normalize_suggested_label_trims_empty_to_none_and_caps_length() {
        assert_eq!(
            normalize_suggested_label(Some(" Lab ".to_string())).unwrap(),
            Some("Lab".to_string())
        );
        assert_eq!(
            normalize_suggested_label(Some("   ".to_string())).unwrap(),
            None
        );
        assert!(normalize_suggested_label(Some("x".repeat(257))).is_err());
    }

    #[test]
    fn normalize_label_trims_empty_to_none_and_caps_length() {
        assert_eq!(
            normalize_label(Some(" Hallway ".to_string())).unwrap(),
            Some("Hallway".to_string())
        );
        assert_eq!(normalize_label(Some("   ".to_string())).unwrap(), None);
        assert!(normalize_label(Some("x".repeat(201))).is_err());
    }

    #[test]
    fn normalize_org_id_accepts_uuid_and_rejects_names() {
        let id = uuid::Uuid::new_v4().to_string();
        assert_eq!(normalize_org_id(Some(id.clone())).unwrap(), Some(id));
        assert!(normalize_org_id(Some("my-org".to_string())).is_err());
    }
}
