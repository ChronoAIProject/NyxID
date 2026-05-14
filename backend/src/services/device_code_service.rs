#![allow(dead_code)]

use chrono::{DateTime, Duration, Utc};
use mongodb::{
    Collection, Database,
    bson::{self, doc},
    options::ReturnDocument,
};
use serde::Serialize;
use uuid::Uuid;

use crate::crypto::device_code::{generate_device_code, generate_user_code, verify_poll_signature};
use crate::crypto::token::hash_token;
use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::device_code::{
    COLLECTION_NAME as DEVICE_CODES, DeviceCode, DeviceCodeStatus, UserCodeGen,
};
use crate::models::node::{COLLECTION_NAME as NODES, Node};
use crate::services::node_service::DeviceNodeInput;
use crate::services::{key_service, node_service, org_service};

pub const DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD: u32 = 3;
pub const DEVICE_CODE_LOCKOUT_SECS: i64 = 60 * 60;
pub const DEVICE_CODE_EXPIRES_IN_SECS: i64 = 15 * 60;
pub const DEVICE_CODE_POLL_INTERVAL_SECS: u32 = 5;
pub const DEVICE_CODE_ROTATE_SECS: i64 = 30;
pub const DEVICE_CODE_TIMESTAMP_SKEW_SECS: i64 = 60;
pub const DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS: i64 = 24 * 60 * 60;
const DEVICE_CODE_API_KEY_SCOPES: &str = "read write proxy";

#[derive(Clone, Debug)]
pub struct DeviceCodeInitiateInput {
    pub device_pubkey: [u8; 32],
    pub hw_id: String,
    pub suggested_label: Option<String>,
    pub frontend_url: String,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct DeviceCodeInitiate {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: i64,
    pub poll_interval: u32,
}

#[derive(Clone, Debug)]
pub struct DeviceCodePollInput {
    pub device_code: String,
    pub timestamp: i64,
    pub signature: [u8; 64],
}

#[derive(Clone, Debug, Serialize, PartialEq)]
#[serde(tag = "status", rename_all = "lowercase")]
pub enum DeviceCodePoll {
    Pending {
        current_user_code: String,
        interval: u32,
    },
    Approved {
        api_key: String,
        node_id: String,
        refresh_token: String,
        expires_in: i64,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceCodeApproveInput {
    pub user_code: String,
    pub org_id: Option<String>,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct DeviceCodeApprove {
    pub device_label: String,
    pub hw_id: String,
    pub api_key_id: String,
    pub node_id: String,
    pub owner_user_id: String,
    pub org_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DeviceCodeLockoutNotification {
    pub recipients: Vec<String>,
    pub device_label: String,
    pub hw_id: String,
    pub node_id: Option<String>,
    pub failed_poll_count: u32,
    pub locked_until: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SignatureFailureLockout {
    pub failed_poll_count: u32,
    pub locked_until: Option<DateTime<Utc>>,
}

pub fn apply_signature_failure_lockout(
    current_failed_poll_count: u32,
    now: DateTime<Utc>,
) -> SignatureFailureLockout {
    let failed_poll_count = current_failed_poll_count.saturating_add(1);
    let locked_until = (failed_poll_count >= DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD)
        .then_some(now + Duration::seconds(DEVICE_CODE_LOCKOUT_SECS));

    SignatureFailureLockout {
        failed_poll_count,
        locked_until,
    }
}

pub fn is_locked(locked_until: Option<DateTime<Utc>>, now: DateTime<Utc>) -> bool {
    locked_until.is_some_and(|until| until > now)
}

pub async fn initiate(
    db: &Database,
    input: DeviceCodeInitiateInput,
) -> AppResult<DeviceCodeInitiate> {
    let now = Utc::now();
    let (device_code, device_code_hash) = generate_device_code();
    let user_code = generate_user_code();
    let (verification_uri, verification_uri_complete) =
        build_verification_uris(&input.frontend_url, &user_code)?;

    let row = DeviceCode {
        id: Uuid::new_v4().to_string(),
        device_code_hash,
        device_pubkey: input.device_pubkey.to_vec(),
        hw_id: input.hw_id,
        suggested_label: input.suggested_label,
        user_code_history: vec![UserCodeGen {
            code: user_code.clone(),
            generated_at: now,
        }],
        status: DeviceCodeStatus::Pending,
        approved_by_user_id: None,
        approved_org_id: None,
        issued_api_key_id: None,
        issued_node_id: None,
        delivery_api_key: None,
        delivery_refresh_token: None,
        refresh_token_hash: None,
        failed_poll_count: 0,
        locked_until: None,
        lock_alert_sent_at: None,
        expires_at: now + Duration::seconds(DEVICE_CODE_EXPIRES_IN_SECS),
        created_at: now,
        last_polled_at: None,
        last_poll_timestamp: None,
        last_rotated_at: now,
    };

    db.collection::<DeviceCode>(DEVICE_CODES)
        .insert_one(&row)
        .await?;

    Ok(DeviceCodeInitiate {
        device_code,
        user_code,
        verification_uri,
        verification_uri_complete,
        expires_in: DEVICE_CODE_EXPIRES_IN_SECS,
        poll_interval: DEVICE_CODE_POLL_INTERVAL_SECS,
    })
}

pub async fn approve(
    db: &Database,
    actor_user_id: &str,
    input: DeviceCodeApproveInput,
) -> AppResult<DeviceCodeApprove> {
    let now = Utc::now();
    let collection = db.collection::<DeviceCode>(DEVICE_CODES);
    let row = collection
        .find_one(doc! { "user_code_history.code": &input.user_code })
        .sort(doc! { "created_at": -1 })
        .await?
        .ok_or(AppError::DeviceUserCodeInvalid)?;

    ensure_row_approvable(&collection, &row, now).await?;

    let owner_user_id = input
        .org_id
        .clone()
        .unwrap_or_else(|| actor_user_id.to_string());
    let owner_access = org_service::resolve_owner_access(db, actor_user_id, &owner_user_id).await?;
    if !owner_access.can_write() {
        return Err(AppError::Forbidden(
            "You must be the owner or an org admin to approve this device".to_string(),
        ));
    }

    let label = choose_device_label(&row, input.label.as_deref())?;
    let empty_node_ids: Vec<String> = Vec::new();
    let created_key = key_service::create_api_key(
        db,
        &owner_user_id,
        &label,
        DEVICE_CODE_API_KEY_SCOPES,
        None,
        Some("Device-code provisioned device"),
        None,
        Some(&empty_node_ids),
        Some(true),
        Some(false),
        None,
        None,
        Some("device-code"),
        None,
    )
    .await?;

    let pubkey: [u8; 32] = row
        .device_pubkey
        .clone()
        .try_into()
        .map_err(|_| AppError::Internal("stored device_pubkey is not 32 bytes".to_string()))?;

    let node = match node_service::create_for_device(
        db,
        DeviceNodeInput {
            user_id: &owner_user_id,
            api_key_id: &created_key.id,
            hw_id: &row.hw_id,
            label: &label,
            device_pubkey: &pubkey,
        },
    )
    .await
    {
        Ok(node) => node,
        Err(error) => {
            cleanup_partial_approval(db, &owner_user_id, Some(&created_key.id), None).await;
            return Err(error);
        }
    };

    if let Err(error) = scope_api_key_to_node(db, &owner_user_id, &created_key.id, &node.id).await {
        cleanup_partial_approval(db, &owner_user_id, Some(&created_key.id), Some(&node.id)).await;
        return Err(error);
    }

    let refresh_token = hex::encode(rand::random::<[u8; 32]>());
    let refresh_token_hash = hash_token(&refresh_token);
    let approved_status = bson::to_bson(&DeviceCodeStatus::Approved)
        .map_err(|e| AppError::Internal(format!("serialize device code status: {e}")))?;
    let now = Utc::now();
    let update_result = collection
        .update_one(
            doc! {
                "_id": &row.id,
                "status": "pending",
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            },
            doc! {
                "$set": {
                    "status": approved_status,
                    "approved_by_user_id": actor_user_id,
                    "approved_org_id": input.org_id.clone(),
                    "issued_api_key_id": &created_key.id,
                    "issued_node_id": &node.id,
                    "delivery_api_key": &created_key.full_key,
                    "delivery_refresh_token": &refresh_token,
                    "refresh_token_hash": &refresh_token_hash,
                }
            },
        )
        .await;

    let update_result = match update_result {
        Ok(update_result) => update_result,
        Err(error) => {
            cleanup_partial_approval(db, &owner_user_id, Some(&created_key.id), Some(&node.id))
                .await;
            return Err(error.into());
        }
    };

    if update_result.matched_count == 0 {
        cleanup_partial_approval(db, &owner_user_id, Some(&created_key.id), Some(&node.id)).await;
        if row.expires_at <= now {
            return Err(AppError::DeviceCodeExpired);
        }
        return Err(AppError::DeviceCodeAlreadyDelivered);
    }

    Ok(DeviceCodeApprove {
        device_label: label,
        hw_id: row.hw_id,
        api_key_id: created_key.id,
        node_id: node.id,
        owner_user_id,
        org_id: input.org_id,
    })
}

pub async fn claim_lockout_notification(
    db: &Database,
    device_code_raw: &str,
) -> AppResult<Option<DeviceCodeLockoutNotification>> {
    let now = Utc::now();
    let row = db
        .collection::<DeviceCode>(DEVICE_CODES)
        .find_one_and_update(
            doc! {
                "device_code_hash": hash_token(device_code_raw),
                "failed_poll_count": { "$gte": i64::from(DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD) },
                "locked_until": { "$gt": bson::DateTime::from_chrono(now) },
                "lock_alert_sent_at": bson::Bson::Null,
            },
            doc! { "$set": { "lock_alert_sent_at": bson::DateTime::from_chrono(now) } },
        )
        .return_document(ReturnDocument::After)
        .await?;

    let Some(row) = row else {
        return Ok(None);
    };
    let Some(locked_until) = row.locked_until else {
        return Ok(None);
    };

    let recipients = lockout_notification_recipients(db, &row).await?;
    let device_label = choose_device_label(&row, None)?;
    Ok(Some(DeviceCodeLockoutNotification {
        recipients,
        device_label,
        hw_id: row.hw_id,
        node_id: row.issued_node_id,
        failed_poll_count: row.failed_poll_count,
        locked_until,
    }))
}

pub async fn poll(db: &Database, input: DeviceCodePollInput) -> AppResult<DeviceCodePoll> {
    let now = Utc::now();
    let collection = db.collection::<DeviceCode>(DEVICE_CODES);
    let mut row = collection
        .find_one(doc! {
            "device_code_hash": crate::crypto::token::hash_token(&input.device_code),
        })
        .await?
        .ok_or(AppError::DeviceCodeNotFound)?;

    if row.status == DeviceCodeStatus::Delivered {
        return Err(AppError::DeviceCodeAlreadyDelivered);
    }

    if row.expires_at <= now || row.status == DeviceCodeStatus::Expired {
        collection
            .update_one(
                doc! { "_id": &row.id },
                doc! { "$set": { "status": "expired" } },
            )
            .await?;
        return Err(AppError::DeviceCodeExpired);
    }

    if is_locked(row.locked_until, now) {
        return Err(AppError::DeviceCodeLocked);
    }

    verify_poll_timestamp(&row, input.timestamp, now)?;

    let pubkey: [u8; 32] = row
        .device_pubkey
        .clone()
        .try_into()
        .map_err(|_| AppError::Internal("stored device_pubkey is not 32 bytes".to_string()))?;
    if let Err(error) = verify_poll_signature(
        &pubkey,
        &input.device_code,
        input.timestamp,
        &input.signature,
    ) {
        let transition = apply_signature_failure_lockout(row.failed_poll_count, now);
        let mut set_doc = doc! {
            "failed_poll_count": i64::from(transition.failed_poll_count),
            "last_polled_at": bson::DateTime::from_chrono(now),
        };
        if let Some(locked_until) = transition.locked_until {
            set_doc.insert("locked_until", bson::DateTime::from_chrono(locked_until));
        }
        collection
            .update_one(doc! { "_id": &row.id }, doc! { "$set": set_doc })
            .await?;

        if transition.locked_until.is_some() {
            return Err(AppError::DeviceCodeLocked);
        }
        return Err(error);
    }

    row.failed_poll_count = 0;
    row.last_polled_at = Some(now);
    row.last_poll_timestamp = Some(input.timestamp);

    match row.status {
        DeviceCodeStatus::Pending => {
            let current_user_code = rotate_user_code_if_needed(&mut row, now)?;
            persist_successful_poll(db, &row, None).await?;
            Ok(DeviceCodePoll::Pending {
                current_user_code,
                interval: DEVICE_CODE_POLL_INTERVAL_SECS,
            })
        }
        DeviceCodeStatus::Approved => {
            let api_key = row.delivery_api_key.clone().ok_or_else(|| {
                AppError::Internal("approved device code missing delivery api key".to_string())
            })?;
            let refresh_token = row.delivery_refresh_token.clone().ok_or_else(|| {
                AppError::Internal(
                    "approved device code missing delivery refresh token".to_string(),
                )
            })?;
            let node_id = row.issued_node_id.clone().ok_or_else(|| {
                AppError::Internal("approved device code missing issued node id".to_string())
            })?;

            persist_successful_poll(db, &row, Some(DeviceCodeStatus::Delivered)).await?;
            Ok(DeviceCodePoll::Approved {
                api_key,
                node_id,
                refresh_token,
                expires_in: DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS,
            })
        }
        DeviceCodeStatus::Denied => Err(AppError::Forbidden("Device code denied".to_string())),
        DeviceCodeStatus::Expired => Err(AppError::DeviceCodeExpired),
        DeviceCodeStatus::Delivered => Err(AppError::DeviceCodeAlreadyDelivered),
    }
}

async fn ensure_row_approvable(
    collection: &Collection<DeviceCode>,
    row: &DeviceCode,
    now: DateTime<Utc>,
) -> AppResult<()> {
    if row.expires_at <= now || row.status == DeviceCodeStatus::Expired {
        collection
            .update_one(
                doc! { "_id": &row.id },
                doc! { "$set": { "status": "expired" } },
            )
            .await?;
        return Err(AppError::DeviceCodeExpired);
    }

    if is_locked(row.locked_until, now) {
        return Err(AppError::DeviceCodeLocked);
    }

    match row.status {
        DeviceCodeStatus::Pending => Ok(()),
        DeviceCodeStatus::Denied => Err(AppError::Forbidden("Device code denied".to_string())),
        DeviceCodeStatus::Expired => Err(AppError::DeviceCodeExpired),
        DeviceCodeStatus::Approved | DeviceCodeStatus::Delivered => {
            Err(AppError::DeviceCodeAlreadyDelivered)
        }
    }
}

async fn scope_api_key_to_node(
    db: &Database,
    owner_user_id: &str,
    api_key_id: &str,
    node_id: &str,
) -> AppResult<()> {
    let result = db
        .collection::<ApiKey>(API_KEYS)
        .update_one(
            doc! { "_id": api_key_id, "user_id": owner_user_id, "is_active": true },
            doc! {
                "$set": {
                    "allow_all_nodes": false,
                    "allowed_node_ids": vec![node_id.to_string()],
                }
            },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::Internal(
            "created device API key disappeared before node scoping".to_string(),
        ));
    }

    Ok(())
}

async fn cleanup_partial_approval(
    db: &Database,
    owner_user_id: &str,
    api_key_id: Option<&str>,
    node_id: Option<&str>,
) {
    if let Some(node_id) = node_id
        && let Err(error) = db
            .collection::<Node>(NODES)
            .delete_one(doc! { "_id": node_id, "user_id": owner_user_id })
            .await
    {
        tracing::warn!(
            node_id = %node_id,
            user_id = %owner_user_id,
            error = %error,
            "Failed to clean up partial device-code node"
        );
    }

    if let Some(api_key_id) = api_key_id
        && let Err(error) = db
            .collection::<ApiKey>(API_KEYS)
            .delete_one(doc! { "_id": api_key_id, "user_id": owner_user_id })
            .await
    {
        tracing::warn!(
            api_key_id = %api_key_id,
            user_id = %owner_user_id,
            error = %error,
            "Failed to clean up partial device-code API key"
        );
    }
}

async fn lockout_notification_recipients(
    db: &Database,
    row: &DeviceCode,
) -> AppResult<Vec<String>> {
    let mut recipients = if let Some(org_id) = row.approved_org_id.as_deref() {
        org_service::list_admin_user_ids(db, org_id).await?
    } else {
        Vec::new()
    };

    if recipients.is_empty()
        && let Some(user_id) = row.approved_by_user_id.as_ref()
    {
        recipients.push(user_id.clone());
    }

    recipients.sort();
    recipients.dedup();
    Ok(recipients)
}

fn choose_device_label(row: &DeviceCode, requested_label: Option<&str>) -> AppResult<String> {
    let candidate = requested_label
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .or_else(|| {
            row.suggested_label
                .as_deref()
                .map(str::trim)
                .filter(|label| !label.is_empty())
        })
        .map(str::to_string)
        .unwrap_or_else(|| format!("device-{}", row.hw_id));

    let label = truncate_to_max_bytes(candidate.trim(), 200);
    if label.is_empty() {
        return Err(AppError::ValidationError(
            "Device label must be between 1 and 200 characters".to_string(),
        ));
    }

    Ok(label)
}

fn truncate_to_max_bytes(value: &str, max_bytes: usize) -> String {
    let mut output = String::new();
    for c in value.chars() {
        if output.len() + c.len_utf8() > max_bytes {
            break;
        }
        output.push(c);
    }
    output
}

fn verify_poll_timestamp(row: &DeviceCode, timestamp: i64, now: DateTime<Utc>) -> AppResult<()> {
    let skew = (timestamp - now.timestamp()).abs();
    if skew > DEVICE_CODE_TIMESTAMP_SKEW_SECS {
        return Err(AppError::DevicePollSignatureInvalid(
            "poll timestamp outside allowed skew".to_string(),
        ));
    }
    if let Some(last) = row.last_poll_timestamp
        && timestamp <= last
    {
        return Err(AppError::DevicePollSignatureInvalid(
            "poll timestamp replay detected".to_string(),
        ));
    }
    Ok(())
}

fn rotate_user_code_if_needed(row: &mut DeviceCode, now: DateTime<Utc>) -> AppResult<String> {
    if now.signed_duration_since(row.last_rotated_at) > Duration::seconds(DEVICE_CODE_ROTATE_SECS) {
        let new_code = generate_user_code();
        row.user_code_history.insert(
            0,
            UserCodeGen {
                code: new_code.clone(),
                generated_at: now,
            },
        );
        row.user_code_history.truncate(4);
        row.last_rotated_at = now;
        return Ok(new_code);
    }

    row.user_code_history
        .first()
        .map(|generation| generation.code.clone())
        .ok_or_else(|| AppError::Internal("device code has no user code history".to_string()))
}

async fn persist_successful_poll(
    db: &Database,
    row: &DeviceCode,
    status: Option<DeviceCodeStatus>,
) -> AppResult<()> {
    let mut set_doc = doc! {
        "failed_poll_count": 0_i64,
        "last_polled_at": bson::DateTime::from_chrono(row.last_polled_at.expect("set before persist")),
        "last_poll_timestamp": row.last_poll_timestamp.expect("set before persist"),
        "user_code_history": bson::to_bson(&row.user_code_history)
            .map_err(|e| AppError::Internal(format!("serialize user_code_history: {e}")))?,
        "last_rotated_at": bson::DateTime::from_chrono(row.last_rotated_at),
    };
    if let Some(ref status) = status {
        set_doc.insert(
            "status",
            bson::to_bson(&status)
                .map_err(|e| AppError::Internal(format!("serialize status: {e}")))?,
        );
    }

    let update = if matches!(status, Some(DeviceCodeStatus::Delivered)) {
        doc! {
            "$set": set_doc,
            "$unset": {
                "delivery_api_key": "",
                "delivery_refresh_token": "",
            },
        }
    } else {
        doc! { "$set": set_doc }
    };

    db.collection::<DeviceCode>(DEVICE_CODES)
        .update_one(doc! { "_id": &row.id }, update)
        .await?;
    Ok(())
}

fn build_verification_uris(frontend_url: &str, user_code: &str) -> AppResult<(String, String)> {
    let frontend = frontend_url.trim_end_matches('/');
    let verification_uri = format!("{frontend}/settings/devices/bind");
    let mut parsed = url::Url::parse(&verification_uri)
        .map_err(|_| AppError::Internal("FRONTEND_URL is not a valid URL".to_string()))?;
    parsed.query_pairs_mut().append_pair("user_code", user_code);
    Ok((verification_uri, parsed.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::crypto::device_code::decode_device_code;
    use crate::crypto::token::hash_token;
    use crate::models::api_key::ApiKey;
    use crate::models::node::{Node, NodeStatus};
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::test_utils::{connect_test_database, test_user};
    use ed25519_dalek::{Signer, SigningKey};

    fn signing_key() -> SigningKey {
        SigningKey::from_bytes(&[77u8; 32])
    }

    fn sign_poll(device_code: &str, timestamp: i64, key: &SigningKey) -> [u8; 64] {
        let decoded = decode_device_code(device_code).expect("valid device code");
        let mut message = Vec::with_capacity(decoded.len() + std::mem::size_of::<i64>());
        message.extend_from_slice(&decoded);
        message.extend_from_slice(&timestamp.to_be_bytes());
        key.sign(&message).to_bytes()
    }

    async fn setup_pending_row(prefix: &str) -> Option<(Database, DeviceCodeInitiate, SigningKey)> {
        let db = connect_test_database(prefix).await?;
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let key = signing_key();
        let response = initiate(
            &db,
            DeviceCodeInitiateInput {
                device_pubkey: key.verifying_key().to_bytes(),
                hw_id: "esp32-p4-cam-1".to_string(),
                suggested_label: Some("Kitchen cam".to_string()),
                frontend_url: "https://app.example.com".to_string(),
            },
        )
        .await
        .expect("initiate");
        Some((db, response, key))
    }

    #[test]
    fn signature_failures_below_threshold_do_not_lock() {
        let now = Utc::now();
        let transition = apply_signature_failure_lockout(1, now);

        assert_eq!(transition.failed_poll_count, 2);
        assert_eq!(transition.locked_until, None);
    }

    #[test]
    fn signature_failure_at_threshold_locks_for_one_hour() {
        let now = Utc::now();
        let transition = apply_signature_failure_lockout(2, now);

        assert_eq!(transition.failed_poll_count, 3);
        assert_eq!(
            transition.locked_until.expect("locked").timestamp(),
            (now + Duration::hours(1)).timestamp()
        );
    }

    #[test]
    fn signature_failure_after_threshold_keeps_locking() {
        let now = Utc::now();
        let transition = apply_signature_failure_lockout(3, now);

        assert_eq!(transition.failed_poll_count, 4);
        assert!(transition.locked_until.is_some());
    }

    #[test]
    fn is_locked_only_when_until_is_in_future() {
        let now = Utc::now();

        assert!(is_locked(Some(now + Duration::seconds(1)), now));
        assert!(!is_locked(Some(now), now));
        assert!(!is_locked(Some(now - Duration::seconds(1)), now));
        assert!(!is_locked(None, now));
    }

    #[test]
    fn verification_uris_point_to_bind_page_and_include_user_code() {
        let (uri, complete) =
            build_verification_uris("https://app.example.com/", "ABCD-EFGH-JKLM").unwrap();

        assert_eq!(uri, "https://app.example.com/settings/devices/bind");
        assert_eq!(
            complete,
            "https://app.example.com/settings/devices/bind?user_code=ABCD-EFGH-JKLM"
        );
    }

    #[test]
    fn choose_device_label_prefers_request_then_suggested_then_hw_id() {
        let row = DeviceCode {
            id: Uuid::new_v4().to_string(),
            device_code_hash: "deadbeef".repeat(8),
            device_pubkey: vec![1u8; 32],
            hw_id: "esp32-cam".to_string(),
            suggested_label: Some("Kitchen".to_string()),
            user_code_history: vec![],
            status: DeviceCodeStatus::Pending,
            approved_by_user_id: None,
            approved_org_id: None,
            issued_api_key_id: None,
            issued_node_id: None,
            delivery_api_key: None,
            delivery_refresh_token: None,
            refresh_token_hash: None,
            failed_poll_count: 0,
            locked_until: None,
            lock_alert_sent_at: None,
            expires_at: Utc::now() + Duration::minutes(15),
            created_at: Utc::now(),
            last_polled_at: None,
            last_poll_timestamp: None,
            last_rotated_at: Utc::now(),
        };

        assert_eq!(
            choose_device_label(&row, Some(" Hallway ")).unwrap(),
            "Hallway"
        );
        assert_eq!(choose_device_label(&row, None).unwrap(), "Kitchen");

        let mut row_without_suggestion = row;
        row_without_suggestion.suggested_label = None;
        assert_eq!(
            choose_device_label(&row_without_suggestion, None).unwrap(),
            "device-esp32-cam"
        );
    }

    #[tokio::test]
    async fn initiate_persists_pending_device_code_row() {
        let Some(db) = connect_test_database("device_code_initiate").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");

        let response = initiate(
            &db,
            DeviceCodeInitiateInput {
                device_pubkey: [11u8; 32],
                hw_id: "esp32-p4-cam-1".to_string(),
                suggested_label: Some("Kitchen cam".to_string()),
                frontend_url: "https://app.example.com".to_string(),
            },
        )
        .await
        .expect("initiate device code");

        assert_eq!(response.expires_in, DEVICE_CODE_EXPIRES_IN_SECS);
        assert_eq!(response.poll_interval, DEVICE_CODE_POLL_INTERVAL_SECS);
        assert!(response.verification_uri_complete.contains("user_code="));

        let row = db
            .collection::<DeviceCode>(DEVICE_CODES)
            .find_one(doc! { "device_code_hash": hash_token(&response.device_code) })
            .await
            .expect("query")
            .expect("row exists");

        assert_eq!(row.status, DeviceCodeStatus::Pending);
        assert_eq!(row.device_pubkey, vec![11u8; 32]);
        assert_eq!(row.hw_id, "esp32-p4-cam-1");
        assert_eq!(row.suggested_label.as_deref(), Some("Kitchen cam"));
        assert_eq!(row.user_code_history.len(), 1);
        assert_eq!(row.user_code_history[0].code, response.user_code);
        assert_eq!(row.failed_poll_count, 0);
        assert!(row.locked_until.is_none());
    }

    #[tokio::test]
    async fn poll_pending_returns_current_user_code() {
        let Some((db, response, key)) = setup_pending_row("device_code_poll_pending").await else {
            return;
        };
        let timestamp = Utc::now().timestamp();

        let poll_response = poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature: sign_poll(&response.device_code, timestamp, &key),
            },
        )
        .await
        .expect("poll");

        assert_eq!(
            poll_response,
            DeviceCodePoll::Pending {
                current_user_code: response.user_code,
                interval: DEVICE_CODE_POLL_INTERVAL_SECS,
            }
        );
    }

    #[tokio::test]
    async fn poll_wrong_signature_increments_and_locks_at_threshold() {
        let Some((db, response, _key)) = setup_pending_row("device_code_poll_wrong_sig").await
        else {
            return;
        };
        let wrong_key = SigningKey::from_bytes(&[88u8; 32]);

        for attempt in 0..3 {
            let timestamp = Utc::now().timestamp() + attempt;
            let error = poll(
                &db,
                DeviceCodePollInput {
                    device_code: response.device_code.clone(),
                    timestamp,
                    signature: sign_poll(&response.device_code, timestamp, &wrong_key),
                },
            )
            .await
            .expect_err("wrong signature should fail");

            if attempt < 2 {
                assert!(matches!(error, AppError::DevicePollSignatureInvalid(_)));
            } else {
                assert!(matches!(error, AppError::DeviceCodeLocked));
            }
        }

        let row = db
            .collection::<DeviceCode>(DEVICE_CODES)
            .find_one(doc! { "device_code_hash": hash_token(&response.device_code) })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.failed_poll_count, 3);
        assert!(row.locked_until.is_some());
    }

    #[tokio::test]
    async fn poll_rejects_replayed_timestamp() {
        let Some((db, response, key)) = setup_pending_row("device_code_poll_replay").await else {
            return;
        };
        let timestamp = Utc::now().timestamp();
        let signature = sign_poll(&response.device_code, timestamp, &key);

        poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature,
            },
        )
        .await
        .expect("first poll");

        let error = poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature,
            },
        )
        .await
        .expect_err("replay should fail");
        assert!(matches!(error, AppError::DevicePollSignatureInvalid(_)));
    }

    #[tokio::test]
    async fn poll_rotates_pending_user_code_after_window() {
        let Some((db, response, key)) = setup_pending_row("device_code_poll_rotate").await else {
            return;
        };
        let old_rotated_at = Utc::now() - Duration::seconds(DEVICE_CODE_ROTATE_SECS + 1);
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! { "$set": { "last_rotated_at": bson::DateTime::from_chrono(old_rotated_at) } },
            )
            .await
            .expect("age row");

        let timestamp = Utc::now().timestamp();
        let poll_response = poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature: sign_poll(&response.device_code, timestamp, &key),
            },
        )
        .await
        .expect("poll");

        let DeviceCodePoll::Pending {
            current_user_code, ..
        } = poll_response
        else {
            panic!("expected pending");
        };
        assert_ne!(current_user_code, response.user_code);

        let row = db
            .collection::<DeviceCode>(DEVICE_CODES)
            .find_one(doc! { "device_code_hash": hash_token(&response.device_code) })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.user_code_history.len(), 2);
        assert_eq!(row.user_code_history[0].code, current_user_code);
        assert_eq!(row.user_code_history[1].code, response.user_code);
    }

    #[tokio::test]
    async fn poll_expired_code_returns_expired() {
        let Some((db, response, key)) = setup_pending_row("device_code_poll_expired").await else {
            return;
        };
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! { "$set": { "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)) } },
            )
            .await
            .expect("expire row");
        let timestamp = Utc::now().timestamp();

        let error = poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature: sign_poll(&response.device_code, timestamp, &key),
            },
        )
        .await
        .expect_err("expired");
        assert!(matches!(error, AppError::DeviceCodeExpired));
    }

    #[tokio::test]
    async fn poll_approved_delivers_once_and_clears_delivery_secrets() {
        let Some((db, response, key)) = setup_pending_row("device_code_poll_approved").await else {
            return;
        };
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! {
                    "$set": {
                        "status": "approved",
                        "issued_node_id": "node-1",
                        "delivery_api_key": "nyx_secret",
                        "delivery_refresh_token": "refresh_secret",
                        "refresh_token_hash": hash_token("refresh_secret"),
                    }
                },
            )
            .await
            .expect("approve row");
        let timestamp = Utc::now().timestamp();

        let poll_response = poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature: sign_poll(&response.device_code, timestamp, &key),
            },
        )
        .await
        .expect("approved poll");

        assert_eq!(
            poll_response,
            DeviceCodePoll::Approved {
                api_key: "nyx_secret".to_string(),
                node_id: "node-1".to_string(),
                refresh_token: "refresh_secret".to_string(),
                expires_in: DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS,
            }
        );

        let row = db
            .collection::<DeviceCode>(DEVICE_CODES)
            .find_one(doc! { "device_code_hash": hash_token(&response.device_code) })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, DeviceCodeStatus::Delivered);
        assert!(row.delivery_api_key.is_none());
        assert!(row.delivery_refresh_token.is_none());

        let next_timestamp = timestamp + 1;
        let error = poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp: next_timestamp,
                signature: sign_poll(&response.device_code, next_timestamp, &key),
            },
        )
        .await
        .expect_err("delivered");
        assert!(matches!(error, AppError::DeviceCodeAlreadyDelivered));
    }

    #[tokio::test]
    async fn claim_lockout_notification_claims_once_and_returns_recipients() {
        let Some((db, response, _key)) = setup_pending_row("device_code_lockout_claim").await
        else {
            return;
        };
        let approved_by = Uuid::new_v4().to_string();
        let locked_until = Utc::now() + Duration::hours(1);
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! {
                    "$set": {
                        "failed_poll_count": i64::from(DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD),
                        "locked_until": bson::DateTime::from_chrono(locked_until),
                        "approved_by_user_id": &approved_by,
                    }
                },
            )
            .await
            .expect("lock row");

        let claim = claim_lockout_notification(&db, &response.device_code)
            .await
            .expect("claim")
            .expect("claimed");

        assert_eq!(claim.recipients, vec![approved_by]);
        assert_eq!(claim.device_label, "Kitchen cam");
        assert_eq!(claim.hw_id, "esp32-p4-cam-1");
        assert_eq!(
            claim.failed_poll_count,
            DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD
        );
        assert_eq!(claim.locked_until.timestamp(), locked_until.timestamp());

        let second = claim_lockout_notification(&db, &response.device_code)
            .await
            .expect("second claim");
        assert!(second.is_none());
    }

    #[tokio::test]
    async fn approve_issues_scoped_api_key_node_and_poll_delivery_secret() {
        let Some((db, response, key)) = setup_pending_row("device_code_approve_happy").await else {
            return;
        };
        let actor_user_id = Uuid::new_v4().to_string();

        let approval = approve(
            &db,
            &actor_user_id,
            DeviceCodeApproveInput {
                user_code: response.user_code.clone(),
                org_id: None,
                label: Some("Garage Camera".to_string()),
            },
        )
        .await
        .expect("approve");

        assert_eq!(approval.device_label, "Garage Camera");
        assert_eq!(approval.hw_id, "esp32-p4-cam-1");
        assert_eq!(approval.owner_user_id, actor_user_id);
        assert!(approval.org_id.is_none());

        let api_key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &approval.api_key_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(api_key.platform.as_deref(), Some("device-code"));
        assert_eq!(api_key.scopes, DEVICE_CODE_API_KEY_SCOPES);
        assert!(api_key.allow_all_services);
        assert!(!api_key.allow_all_nodes);
        assert_eq!(api_key.allowed_node_ids, vec![approval.node_id.clone()]);

        let node = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": &approval.node_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(node.user_id, approval.owner_user_id);
        assert_eq!(node.status, NodeStatus::Offline);
        assert!(node.is_active);

        let row = db
            .collection::<DeviceCode>(DEVICE_CODES)
            .find_one(doc! { "device_code_hash": hash_token(&response.device_code) })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, DeviceCodeStatus::Approved);
        assert_eq!(
            row.issued_api_key_id.as_deref(),
            Some(approval.api_key_id.as_str())
        );
        assert_eq!(
            row.issued_node_id.as_deref(),
            Some(approval.node_id.as_str())
        );
        assert!(row.delivery_api_key.is_some());
        assert!(row.delivery_refresh_token.is_some());
        assert!(row.refresh_token_hash.is_some());

        let timestamp = Utc::now().timestamp();
        let delivery = poll(
            &db,
            DeviceCodePollInput {
                device_code: response.device_code.clone(),
                timestamp,
                signature: sign_poll(&response.device_code, timestamp, &key),
            },
        )
        .await
        .expect("poll approved");

        let DeviceCodePoll::Approved {
            api_key,
            node_id,
            refresh_token,
            expires_in,
        } = delivery
        else {
            panic!("expected approved");
        };
        assert!(api_key.starts_with("nyxid_ag_"));
        assert_eq!(node_id, approval.node_id);
        assert_eq!(refresh_token.len(), 64);
        assert_eq!(expires_in, DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS);
    }

    #[tokio::test]
    async fn approve_rejects_double_approve_before_delivery() {
        let Some((db, response, _key)) = setup_pending_row("device_code_approve_double").await
        else {
            return;
        };
        let actor_user_id = Uuid::new_v4().to_string();

        approve(
            &db,
            &actor_user_id,
            DeviceCodeApproveInput {
                user_code: response.user_code.clone(),
                org_id: None,
                label: None,
            },
        )
        .await
        .expect("first approval");

        let error = approve(
            &db,
            &actor_user_id,
            DeviceCodeApproveInput {
                user_code: response.user_code,
                org_id: None,
                label: None,
            },
        )
        .await
        .expect_err("second approval should fail");

        assert!(matches!(error, AppError::DeviceCodeAlreadyDelivered));
    }

    #[tokio::test]
    async fn approve_expired_code_marks_expired() {
        let Some((db, response, _key)) = setup_pending_row("device_code_approve_expired").await
        else {
            return;
        };
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! { "$set": { "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)) } },
            )
            .await
            .expect("expire row");

        let error = approve(
            &db,
            &Uuid::new_v4().to_string(),
            DeviceCodeApproveInput {
                user_code: response.user_code.clone(),
                org_id: None,
                label: None,
            },
        )
        .await
        .expect_err("expired");

        assert!(matches!(error, AppError::DeviceCodeExpired));
        let row = db
            .collection::<DeviceCode>(DEVICE_CODES)
            .find_one(doc! { "device_code_hash": hash_token(&response.device_code) })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.status, DeviceCodeStatus::Expired);
    }

    #[tokio::test]
    async fn approve_rejects_org_without_admin_access() {
        let Some((db, response, _key)) = setup_pending_row("device_code_approve_wrong_org").await
        else {
            return;
        };
        let actor_user_id = Uuid::new_v4().to_string();
        let org_user_id = Uuid::new_v4().to_string();
        let org_user: User = test_user(&org_user_id, UserType::Org);
        db.collection::<User>(USERS)
            .insert_one(&org_user)
            .await
            .expect("insert org");

        let error = approve(
            &db,
            &actor_user_id,
            DeviceCodeApproveInput {
                user_code: response.user_code,
                org_id: Some(org_user_id),
                label: None,
            },
        )
        .await
        .expect_err("forbidden");

        assert!(matches!(error, AppError::Forbidden(_)));
        assert_eq!(
            db.collection::<ApiKey>(API_KEYS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<Node>(NODES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn cleanup_partial_approval_deletes_key_and_node() {
        let Some(db) = connect_test_database("device_code_approve_cleanup").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");
        let owner_user_id = Uuid::new_v4().to_string();
        let empty_node_ids: Vec<String> = Vec::new();
        let created_key = key_service::create_api_key(
            &db,
            &owner_user_id,
            "Cleanup Device",
            DEVICE_CODE_API_KEY_SCOPES,
            None,
            Some("Device-code provisioned device"),
            None,
            Some(&empty_node_ids),
            Some(true),
            Some(false),
            None,
            None,
            Some("device-code"),
            None,
        )
        .await
        .expect("create key");
        let pubkey = [9u8; 32];
        let node = node_service::create_for_device(
            &db,
            DeviceNodeInput {
                user_id: &owner_user_id,
                api_key_id: &created_key.id,
                hw_id: "esp32-cleanup",
                label: "Cleanup Device",
                device_pubkey: &pubkey,
            },
        )
        .await
        .expect("create node");

        cleanup_partial_approval(&db, &owner_user_id, Some(&created_key.id), Some(&node.id)).await;

        assert!(
            db.collection::<ApiKey>(API_KEYS)
                .find_one(doc! { "_id": &created_key.id })
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            db.collection::<Node>(NODES)
                .find_one(doc! { "_id": &node.id })
                .await
                .unwrap()
                .is_none()
        );
    }
}
