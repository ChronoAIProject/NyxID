use chrono::{DateTime, Utc};
use mongodb::{
    Collection, Database,
    bson::{self, doc},
};

use crate::crypto::token::hash_token;
use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::device_code::{COLLECTION_NAME as DEVICE_CODES, DeviceCode, DeviceCodeStatus};
use crate::models::node::{COLLECTION_NAME as NODES, Node};
use crate::services::node_service::DeviceNodeInput;
use crate::services::{key_service, node_service, org_service};

use super::{
    DEVICE_CODE_API_KEY_SCOPES, DeviceCodeApprove, DeviceCodeApproveInput, choose_device_label,
    is_locked,
};

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::node::NodeStatus;
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::services::device_code_service::tests_support::{setup_pending_row, sign_poll};
    use crate::services::device_code_service::{
        DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS, DeviceCodePoll, DeviceCodePollInput, poll,
    };
    use crate::test_utils::{connect_test_database, test_user};
    use chrono::Duration;
    use uuid::Uuid;

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
