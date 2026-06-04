use chrono::{DateTime, Duration, Utc};
use mongodb::{
    Collection, Database,
    bson::{self, doc},
};
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::crypto::token::hash_token;
use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::device_code::{COLLECTION_NAME as DEVICE_CODES, DeviceCode, DeviceCodeStatus};
use crate::models::node::{COLLECTION_NAME as NODES, Node};
use crate::services::node_service::{DEVICE_CODE_PROVISIONING_SOURCE, DeviceNodeInput};
use crate::services::{key_service, node_service, org_service, user_service_service};

use super::{
    DEVICE_CODE_API_KEY_SCOPES, DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS, DeviceCodeApprove,
    DeviceCodeApproveInput, choose_device_label,
};

/// Approve a device using only the current displayed user-code generation.
///
/// Older retained generations are kept so polling devices can continue to show
/// recent codes, but approval intentionally requires `user_code_history[0]` to
/// preserve the anti-shoulder-surfing value of 30-second rotation.
pub async fn approve(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    actor_user_id: &str,
    input: DeviceCodeApproveInput,
) -> AppResult<DeviceCodeApprove> {
    let now = Utc::now();
    let collection = db.collection::<DeviceCode>(DEVICE_CODES);
    let row = collection
        .find_one(doc! { "user_code_history.0.code": &input.user_code })
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
    let allowed_service_ids =
        resolve_default_service_ids(db, &owner_user_id, input.default_services.as_deref()).await?;
    let empty_node_ids: Vec<String> = Vec::new();
    let created_key = key_service::create_api_key(
        db,
        &owner_user_id,
        &label,
        DEVICE_CODE_API_KEY_SCOPES,
        None,
        Some("Device-code provisioned device"),
        Some(&allowed_service_ids),
        Some(&empty_node_ids),
        Some(false),
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
        encryption_keys,
        DeviceNodeInput {
            user_id: &owner_user_id,
            api_key_id: &created_key.id,
            hw_id: &row.hw_id,
            label: &label,
            device_pubkey: Some(&pubkey),
            provisioning_source: DEVICE_CODE_PROVISIONING_SOURCE,
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

    let refresh_token = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
    let refresh_token_hash = hash_token(refresh_token.as_str());
    let delivery_api_key_encrypted = match encryption_keys
        .encrypt(created_key.full_key.as_bytes())
        .await
    {
        Ok(encrypted) => encrypted,
        Err(error) => {
            cleanup_partial_approval(db, &owner_user_id, Some(&created_key.id), Some(&node.id))
                .await;
            return Err(error);
        }
    };
    let delivery_refresh_token_encrypted =
        match encryption_keys.encrypt(refresh_token.as_bytes()).await {
            Ok(encrypted) => encrypted,
            Err(error) => {
                cleanup_partial_approval(db, &owner_user_id, Some(&created_key.id), Some(&node.id))
                    .await;
                return Err(error);
            }
        };
    let approved_status = bson::to_bson(&DeviceCodeStatus::Approved)
        .map_err(|e| AppError::Internal(format!("serialize device code status: {e}")))?;
    let now = Utc::now();
    let delivery_expires_at = now + Duration::seconds(DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS);
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
                    "delivery_api_key_encrypted": bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: delivery_api_key_encrypted,
                    },
                    "delivery_refresh_token_encrypted": bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: delivery_refresh_token_encrypted,
                    },
                    "refresh_token_hash": &refresh_token_hash,
                    "expires_at": bson::DateTime::from_chrono(delivery_expires_at),
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

    match row.status {
        DeviceCodeStatus::Pending => Ok(()),
        DeviceCodeStatus::Denied => Err(AppError::Forbidden("Device code denied".to_string())),
        DeviceCodeStatus::Expired => Err(AppError::DeviceCodeExpired),
        DeviceCodeStatus::Approved | DeviceCodeStatus::Delivered => {
            Err(AppError::DeviceCodeAlreadyDelivered)
        }
    }
}

pub(super) async fn resolve_default_service_ids(
    db: &Database,
    owner_user_id: &str,
    default_services: Option<&[String]>,
) -> AppResult<Vec<String>> {
    let Some(default_services) = default_services else {
        return Ok(Vec::new());
    };

    let mut resolved = Vec::new();
    for raw in default_services {
        let service_id = user_service_service::resolve_service_id(db, owner_user_id, raw).await?;
        if !resolved.contains(&service_id) {
            resolved.push(service_id);
        }
    }

    Ok(resolved)
}

pub(super) async fn scope_api_key_to_node(
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

pub(super) async fn cleanup_partial_approval(
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::device_code::UserCodeGen;
    use crate::models::device_pubkey_lockout::{
        COLLECTION_NAME as DEVICE_PUBKEY_LOCKOUTS, DevicePubkeyLockout,
    };
    use crate::models::node::NodeStatus;
    use crate::models::ssh_auth_mode::SshAuthMode;
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::services::device_code_service::DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD;
    use crate::services::device_code_service::tests_support::{setup_pending_row, sign_poll};
    use crate::services::device_code_service::{DeviceCodePoll, DeviceCodePollInput, poll};
    use crate::test_utils::{connect_test_database, test_encryption_keys, test_user};
    use chrono::Duration;
    use sha2::{Digest, Sha256};
    use uuid::Uuid;

    #[tokio::test]
    async fn approve_without_default_services_issues_empty_service_allowlist_and_poll_secret() {
        let Some((db, response, key)) = setup_pending_row("device_code_approve_happy").await else {
            return;
        };
        let actor_user_id = Uuid::new_v4().to_string();

        let approval = approve_for_test(
            &db,
            &actor_user_id,
            DeviceCodeApproveInput {
                user_code: response.user_code.clone(),
                org_id: None,
                label: Some("Garage Camera".to_string()),
                default_services: None,
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
        assert!(!api_key.allow_all_services);
        assert!(api_key.allowed_service_ids.is_empty());
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
        assert_eq!(node.auth_token_hash.len(), 64);
        assert_eq!(node.signing_secret_hash.len(), 64);
        assert!(node.signing_secret_encrypted.is_some());

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
        assert!(row.delivery_api_key_encrypted.is_some());
        assert!(row.delivery_refresh_token_encrypted.is_some());
        assert!(row.refresh_token_hash.is_some());
        let encryption_keys = test_encryption_keys();
        let decrypted_api_key = String::from_utf8(
            encryption_keys
                .decrypt(
                    row.delivery_api_key_encrypted
                        .as_deref()
                        .expect("encrypted api key"),
                )
                .await
                .expect("decrypt api key"),
        )
        .expect("api key utf8");
        let decrypted_refresh_token = String::from_utf8(
            encryption_keys
                .decrypt(
                    row.delivery_refresh_token_encrypted
                        .as_deref()
                        .expect("encrypted refresh token"),
                )
                .await
                .expect("decrypt refresh token"),
        )
        .expect("refresh token utf8");
        assert!(decrypted_api_key.starts_with("nyxid_ag_"));
        assert_eq!(
            hash_token(&decrypted_refresh_token),
            row.refresh_token_hash.unwrap()
        );

        let timestamp = Utc::now().timestamp();
        let delivery = poll_for_test(
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
        assert_eq!(api_key, decrypted_api_key);
        assert_eq!(node_id, approval.node_id);
        assert_eq!(refresh_token, decrypted_refresh_token);
        assert_eq!(expires_in, DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS);
    }

    #[tokio::test]
    async fn approve_allows_default_services_by_uuid_and_slug() {
        let Some((db, response, _key)) =
            setup_pending_row("device_code_approve_default_services").await
        else {
            return;
        };
        let actor_user_id = Uuid::new_v4().to_string();
        let service_by_id = insert_user_service(&db, &actor_user_id, "svc-by-id").await;
        let service_by_slug = insert_user_service(&db, &actor_user_id, "svc-by-slug").await;

        let approval = approve_for_test(
            &db,
            &actor_user_id,
            DeviceCodeApproveInput {
                user_code: response.user_code,
                org_id: None,
                label: None,
                default_services: Some(vec![
                    service_by_id.id.clone(),
                    service_by_slug.slug.clone(),
                ]),
            },
        )
        .await
        .expect("approve with default services");

        let api_key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &approval.api_key_id })
            .await
            .unwrap()
            .unwrap();
        assert!(!api_key.allow_all_services);
        assert_eq!(
            api_key.allowed_service_ids,
            vec![service_by_id.id, service_by_slug.id]
        );
    }

    #[tokio::test]
    async fn approve_unknown_default_service_returns_not_found_without_partials() {
        let Some((db, response, _key)) =
            setup_pending_row("device_code_approve_default_unknown").await
        else {
            return;
        };
        let actor_user_id = Uuid::new_v4().to_string();

        let error = approve_for_test(
            &db,
            &actor_user_id,
            DeviceCodeApproveInput {
                user_code: response.user_code,
                org_id: None,
                label: None,
                default_services: Some(vec!["missing-svc".to_string()]),
            },
        )
        .await
        .expect_err("unknown service should fail");

        assert!(matches!(error, AppError::NotFound(_)));
        assert_no_partial_approval(&db).await;
    }

    #[tokio::test]
    async fn approve_cross_owner_default_service_returns_forbidden_without_partials() {
        let Some((db, response, _key)) =
            setup_pending_row("device_code_approve_default_cross_owner").await
        else {
            return;
        };
        let actor_user_id = Uuid::new_v4().to_string();
        let other_user_id = Uuid::new_v4().to_string();
        let other_service = insert_user_service(&db, &other_user_id, "other-svc").await;

        let error = approve_for_test(
            &db,
            &actor_user_id,
            DeviceCodeApproveInput {
                user_code: response.user_code,
                org_id: None,
                label: None,
                default_services: Some(vec![other_service.id]),
            },
        )
        .await
        .expect_err("cross-owner service should fail");

        assert!(matches!(
            error,
            AppError::Forbidden(message) if message.contains("not owned")
        ));
        assert_no_partial_approval(&db).await;
    }

    #[tokio::test]
    async fn approve_mixed_valid_and_invalid_default_services_is_atomic() {
        let Some((db, response, _key)) =
            setup_pending_row("device_code_approve_default_mixed_invalid").await
        else {
            return;
        };
        let actor_user_id = Uuid::new_v4().to_string();
        let valid_service = insert_user_service(&db, &actor_user_id, "valid-svc").await;

        let error = approve_for_test(
            &db,
            &actor_user_id,
            DeviceCodeApproveInput {
                user_code: response.user_code,
                org_id: None,
                label: None,
                default_services: Some(vec![valid_service.id, "missing-svc".to_string()]),
            },
        )
        .await
        .expect_err("mixed valid and invalid services should fail");

        assert!(matches!(error, AppError::NotFound(_)));
        assert_no_partial_approval(&db).await;
    }

    #[tokio::test]
    async fn approve_extends_near_ttl_expiry_for_delivery_window() {
        let Some((db, response, _key)) = setup_pending_row("device_code_approve_ttl_bump").await
        else {
            return;
        };
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! {
                    "$set": {
                        "expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::seconds(1)),
                    }
                },
            )
            .await
            .expect("move row close to TTL expiry");

        let before_approve = Utc::now();
        approve_for_test(
            &db,
            &Uuid::new_v4().to_string(),
            DeviceCodeApproveInput {
                user_code: response.user_code.clone(),
                org_id: None,
                label: None,
                default_services: None,
            },
        )
        .await
        .expect("approve near-expiry row");

        let row = db
            .collection::<DeviceCode>(DEVICE_CODES)
            .find_one(doc! { "device_code_hash": hash_token(&response.device_code) })
            .await
            .unwrap()
            .unwrap();

        assert_eq!(row.status, DeviceCodeStatus::Approved);
        assert!(
            row.expires_at
                >= before_approve + Duration::seconds(DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS - 5),
            "approved row should survive the delivery window before TTL can purge it"
        );
    }

    #[tokio::test]
    async fn approve_rejects_double_approve_before_delivery() {
        let Some((db, response, _key)) = setup_pending_row("device_code_approve_double").await
        else {
            return;
        };
        let actor_user_id = Uuid::new_v4().to_string();

        approve_for_test(
            &db,
            &actor_user_id,
            DeviceCodeApproveInput {
                user_code: response.user_code.clone(),
                org_id: None,
                label: None,
                default_services: None,
            },
        )
        .await
        .expect("first approval");

        let error = approve_for_test(
            &db,
            &actor_user_id,
            DeviceCodeApproveInput {
                user_code: response.user_code,
                org_id: None,
                label: None,
                default_services: None,
            },
        )
        .await
        .expect_err("second approval should fail");

        assert!(matches!(error, AppError::DeviceCodeAlreadyDelivered));
    }

    #[tokio::test]
    async fn approve_allows_pubkey_locked_for_polling() {
        let Some((db, response, key)) =
            setup_pending_row("device_code_approve_pubkey_locked").await
        else {
            return;
        };
        db.collection::<DevicePubkeyLockout>(DEVICE_PUBKEY_LOCKOUTS)
            .insert_one(DevicePubkeyLockout {
                id: test_pubkey_hash(&key.verifying_key().to_bytes()),
                failed_poll_count: DEVICE_CODE_SIGNATURE_FAILURE_LOCK_THRESHOLD,
                locked_until: Some(Utc::now() + Duration::hours(1)),
                last_failure_at: Utc::now(),
                last_lockout_audited_at: None,
            })
            .await
            .expect("seed pubkey lockout");

        let approval = approve_for_test(
            &db,
            &Uuid::new_v4().to_string(),
            DeviceCodeApproveInput {
                user_code: response.user_code,
                org_id: None,
                label: None,
                default_services: None,
            },
        )
        .await
        .expect("approve should not be blocked by poll/request lockout");

        assert_eq!(approval.hw_id, "esp32-p4-cam-1");
    }

    #[tokio::test]
    async fn approve_rejects_stale_retained_user_code_generation() {
        let Some((db, response, _key)) = setup_pending_row("device_code_approve_stale_gen").await
        else {
            return;
        };
        let now = Utc::now();
        let current_code = "AAAA-BBBB-CCCC".to_string();
        let history = vec![
            UserCodeGen {
                code: current_code,
                generated_at: now,
            },
            UserCodeGen {
                code: response.user_code.clone(),
                generated_at: now - Duration::seconds(31),
            },
        ];
        db.collection::<DeviceCode>(DEVICE_CODES)
            .update_one(
                doc! { "device_code_hash": hash_token(&response.device_code) },
                doc! {
                    "$set": {
                        "user_code_history": bson::to_bson(&history).expect("serialize history"),
                    }
                },
            )
            .await
            .expect("set stale history");

        let error = approve_for_test(
            &db,
            &Uuid::new_v4().to_string(),
            DeviceCodeApproveInput {
                user_code: response.user_code,
                org_id: None,
                label: None,
                default_services: None,
            },
        )
        .await
        .expect_err("stale generation must not approve");

        assert!(matches!(error, AppError::DeviceUserCodeInvalid));
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

        let error = approve_for_test(
            &db,
            &Uuid::new_v4().to_string(),
            DeviceCodeApproveInput {
                user_code: response.user_code.clone(),
                org_id: None,
                label: None,
                default_services: None,
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

        let error = approve_for_test(
            &db,
            &actor_user_id,
            DeviceCodeApproveInput {
                user_code: response.user_code,
                org_id: Some(org_user_id),
                label: None,
                default_services: None,
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
        let empty_service_ids: Vec<String> = Vec::new();
        let empty_node_ids: Vec<String> = Vec::new();
        let created_key = key_service::create_api_key(
            &db,
            &owner_user_id,
            "Cleanup Device",
            DEVICE_CODE_API_KEY_SCOPES,
            None,
            Some("Device-code provisioned device"),
            Some(&empty_service_ids),
            Some(&empty_node_ids),
            Some(false),
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
            &test_encryption_keys(),
            DeviceNodeInput {
                user_id: &owner_user_id,
                api_key_id: &created_key.id,
                hw_id: "esp32-cleanup",
                label: "Cleanup Device",
                device_pubkey: Some(&pubkey),
                provisioning_source: DEVICE_CODE_PROVISIONING_SOURCE,
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

    async fn assert_no_partial_approval(db: &Database) {
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

    async fn approve_for_test(
        db: &Database,
        actor_user_id: &str,
        input: DeviceCodeApproveInput,
    ) -> AppResult<DeviceCodeApprove> {
        let encryption_keys = test_encryption_keys();
        approve(db, &encryption_keys, actor_user_id, input).await
    }

    async fn poll_for_test(db: &Database, input: DeviceCodePollInput) -> AppResult<DeviceCodePoll> {
        let encryption_keys = test_encryption_keys();
        poll(db, &encryption_keys, input).await
    }

    async fn insert_user_service(db: &Database, user_id: &str, slug: &str) -> UserService {
        let now = Utc::now();
        let service = UserService {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            slug: slug.to_string(),
            endpoint_id: Uuid::new_v4().to_string(),
            api_key_id: None,
            auth_method: "bearer".to_string(),
            auth_key_name: "Authorization".to_string(),
            catalog_service_id: None,
            node_id: None,
            node_priority: 0,
            service_type: "http".to_string(),
            ssh_auth_mode: SshAuthMode::ProxyOnly,
            ssh_node_keys_stale: false,
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            is_active: true,
            source: None,
            source_id: None,
            source_app_id: None,
            created_at: now,
            updated_at: now,
        };
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert user service");
        service
    }

    fn test_pubkey_hash(pubkey: &[u8]) -> String {
        let mut hasher = Sha256::new();
        hasher.update(pubkey);
        hex::encode(hasher.finalize())
    }
}
