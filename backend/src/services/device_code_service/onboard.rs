use chrono::{Duration, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
    options::ReturnDocument,
};
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::crypto::token::hash_token;
use crate::errors::{AppError, AppResult};
use crate::models::device_onboard_credential::{
    COLLECTION_NAME as DEVICE_ONBOARD_CREDENTIALS, DeviceOnboardCredential,
};
use crate::services::node_service::{DEVICE_ONBOARD_PROVISIONING_SOURCE, DeviceNodeInput};
use crate::services::{key_service, node_service, org_service};

use super::approve::{
    cleanup_partial_approval, resolve_default_service_ids, scope_api_key_to_node,
};
use super::{
    DEVICE_CODE_API_KEY_SCOPES, DeviceOnboard, DeviceOnboardInput, DeviceOnboardRedeem,
    DeviceOnboardRedeemInput,
};

const DEVICE_ONBOARD_API_KEY_PLATFORM: &str = "device-onboard";
const DEVICE_ONBOARD_HW_ID: &str = "qr-onboard";
const DEVICE_ONBOARD_TOKEN_PREFIX: &str = "nyx_obt_";
pub(super) const DEVICE_ONBOARD_EXPIRES_IN_SECS: i64 = 15 * 60;

/// Create a short-lived, single-use bootstrap token for QR provisioning.
///
/// The returned QR payload contains only the bootstrap token and non-secret
/// routing metadata. WiFi credentials are combined into the QR by the client so
/// they are not sent to NyxID, and durable device credentials are minted only
/// after the physical device redeems the bootstrap token.
pub async fn onboard(
    db: &Database,
    actor_user_id: &str,
    input: DeviceOnboardInput,
) -> AppResult<DeviceOnboard> {
    let input = validate_onboard_input(input)?;
    let owner_user_id = input
        .org_id
        .clone()
        .unwrap_or_else(|| actor_user_id.to_string());
    let owner_access = org_service::resolve_owner_access(db, actor_user_id, &owner_user_id).await?;
    if !owner_access.can_write() {
        return Err(AppError::Forbidden(
            "You must be the owner or an org admin to onboard this device".to_string(),
        ));
    }

    let default_service_ids =
        resolve_default_service_ids(db, &owner_user_id, input.default_services.as_deref()).await?;
    let bootstrap_token = Zeroizing::new(generate_bootstrap_token());
    let now = Utc::now();
    let expires_at = now + Duration::seconds(DEVICE_ONBOARD_EXPIRES_IN_SECS);
    let credential = DeviceOnboardCredential {
        id: uuid::Uuid::new_v4().to_string(),
        owner_user_id,
        bootstrap_token_hash: hash_token(bootstrap_token.as_str()),
        label: input.label.clone(),
        default_service_ids,
        used: false,
        redeemed_api_key_id: None,
        redeemed_node_id: None,
        redeemed_refresh_token_hash: None,
        created_at: now,
        expires_at,
    };

    db.collection::<DeviceOnboardCredential>(DEVICE_ONBOARD_CREDENTIALS)
        .insert_one(&credential)
        .await?;

    let qr_payload = build_qr_payload(
        bootstrap_token.as_str(),
        &credential.id,
        &input.base_url,
        DEVICE_ONBOARD_EXPIRES_IN_SECS,
    );

    Ok(DeviceOnboard {
        qr_payload,
        bootstrap_id: credential.id,
        label: input.label,
        expires_in: DEVICE_ONBOARD_EXPIRES_IN_SECS,
        expires_at,
    })
}

/// Consume a QR bootstrap token exactly once and mint durable device
/// credentials for the scanning device.
pub async fn redeem_onboard(
    db: &Database,
    encryption_keys: &EncryptionKeys,
    input: DeviceOnboardRedeemInput,
) -> AppResult<DeviceOnboardRedeem> {
    let bootstrap_token = validate_bootstrap_token(&input.bootstrap_token)?;
    let token_hash = hash_token(&bootstrap_token);
    let now = Utc::now();
    let collection = db.collection::<DeviceOnboardCredential>(DEVICE_ONBOARD_CREDENTIALS);
    let credential = collection
        .find_one_and_update(
            doc! {
                "bootstrap_token_hash": &token_hash,
                "used": false,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            },
            doc! {
                "$set": {
                    "used": true,
                }
            },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or(AppError::DeviceCodeExpired)?;

    let empty_node_ids: Vec<String> = Vec::new();
    let created_key = match key_service::create_api_key(
        db,
        &credential.owner_user_id,
        &credential.label,
        DEVICE_CODE_API_KEY_SCOPES,
        None,
        Some("QR-onboarded device"),
        Some(&credential.default_service_ids),
        Some(&empty_node_ids),
        Some(false),
        Some(false),
        None,
        None,
        Some(DEVICE_ONBOARD_API_KEY_PLATFORM),
        None,
    )
    .await
    {
        Ok(created_key) => created_key,
        Err(error) => {
            mark_bootstrap_unconsumed(db, &credential.id).await;
            return Err(error);
        }
    };

    let node = match node_service::create_for_device(
        db,
        encryption_keys,
        DeviceNodeInput {
            user_id: &credential.owner_user_id,
            api_key_id: &created_key.id,
            hw_id: DEVICE_ONBOARD_HW_ID,
            label: &credential.label,
            device_pubkey: None,
            provisioning_source: DEVICE_ONBOARD_PROVISIONING_SOURCE,
        },
    )
    .await
    {
        Ok(node) => node,
        Err(error) => {
            cleanup_partial_redeem(db, &credential.owner_user_id, Some(&created_key.id), None)
                .await;
            mark_bootstrap_unconsumed(db, &credential.id).await;
            return Err(error);
        }
    };

    if let Err(error) =
        scope_api_key_to_node(db, &credential.owner_user_id, &created_key.id, &node.id).await
    {
        cleanup_partial_redeem(
            db,
            &credential.owner_user_id,
            Some(&created_key.id),
            Some(&node.id),
        )
        .await;
        mark_bootstrap_unconsumed(db, &credential.id).await;
        return Err(error);
    }

    let refresh_token = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
    let refresh_token_hash = hash_token(refresh_token.as_str());
    if let Err(error) = collection
        .update_one(
            doc! { "_id": &credential.id, "used": true },
            doc! {
                "$set": {
                    "redeemed_api_key_id": &created_key.id,
                    "redeemed_node_id": &node.id,
                    "redeemed_refresh_token_hash": &refresh_token_hash,
                }
            },
        )
        .await
    {
        cleanup_partial_redeem(
            db,
            &credential.owner_user_id,
            Some(&created_key.id),
            Some(&node.id),
        )
        .await;
        mark_bootstrap_unconsumed(db, &credential.id).await;
        return Err(error.into());
    }

    Ok(DeviceOnboardRedeem {
        api_key: created_key.full_key,
        node_id: node.id,
        refresh_token: refresh_token.to_string(),
        expires_in: super::DEVICE_CODE_DELIVERY_EXPIRES_IN_SECS,
    })
}

/// Revoke an unredeemed QR bootstrap token owned by the actor or an org they
/// can administer.
pub async fn revoke_onboard(
    db: &Database,
    actor_user_id: &str,
    bootstrap_id: &str,
) -> AppResult<()> {
    let collection = db.collection::<DeviceOnboardCredential>(DEVICE_ONBOARD_CREDENTIALS);
    let credential = collection
        .find_one(doc! { "_id": bootstrap_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Device onboard bootstrap not found".to_string()))?;
    let owner_access =
        org_service::resolve_owner_access(db, actor_user_id, &credential.owner_user_id).await?;
    if !owner_access.can_write() {
        return Err(AppError::NotFound(
            "Device onboard bootstrap not found".to_string(),
        ));
    }

    let result = collection
        .delete_one(doc! {
            "_id": bootstrap_id,
            "used": false,
            "expires_at": { "$gt": bson::DateTime::from_chrono(Utc::now()) },
        })
        .await?;
    if result.deleted_count == 0 {
        return Err(AppError::DeviceCodeExpired);
    }

    Ok(())
}

fn validate_onboard_input(mut input: DeviceOnboardInput) -> AppResult<DeviceOnboardInput> {
    input.label = input.label.trim().to_string();
    input.base_url = input.base_url.trim().trim_end_matches('/').to_string();

    if input.label.is_empty() || input.label.len() > 128 {
        return Err(AppError::ValidationError(
            "label must be between 1 and 128 characters".to_string(),
        ));
    }
    if input.base_url.is_empty() {
        return Err(AppError::ValidationError(
            "base_url must not be empty".to_string(),
        ));
    }

    Ok(input)
}

fn validate_bootstrap_token(token: &str) -> AppResult<String> {
    let token = token.trim();
    let Some(hex_part) = token.strip_prefix(DEVICE_ONBOARD_TOKEN_PREFIX) else {
        return Err(AppError::DeviceCodeNotFound);
    };
    if hex_part.len() != 64 || !hex_part.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::DeviceCodeNotFound);
    }
    Ok(token.to_string())
}

fn generate_bootstrap_token() -> String {
    format!(
        "{}{}",
        DEVICE_ONBOARD_TOKEN_PREFIX,
        hex::encode(rand::random::<[u8; 32]>())
    )
}

fn build_qr_payload(token: &str, bootstrap_id: &str, base_url: &str, expires_in: i64) -> String {
    format!(
        "nyxprov://bootstrap?token={}&id={}&url={}&exp={}",
        urlencoding::encode(token),
        urlencoding::encode(bootstrap_id),
        urlencoding::encode(base_url),
        expires_in,
    )
}

async fn mark_bootstrap_unconsumed(db: &Database, bootstrap_id: &str) {
    if let Err(error) = db
        .collection::<DeviceOnboardCredential>(DEVICE_ONBOARD_CREDENTIALS)
        .update_one(
            doc! { "_id": bootstrap_id, "used": true },
            doc! { "$set": { "used": false } },
        )
        .await
    {
        tracing::warn!(
            bootstrap_id = %bootstrap_id,
            error = %error,
            "Failed to release device onboard bootstrap after redemption failure"
        );
    }
}

async fn cleanup_partial_redeem(
    db: &Database,
    owner_user_id: &str,
    api_key_id: Option<&str>,
    node_id: Option<&str>,
) {
    cleanup_partial_approval(db, owner_user_id, api_key_id, node_id).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
    use crate::models::node::{COLLECTION_NAME as NODES, Node};
    use crate::models::ssh_auth_mode::SshAuthMode;
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::test_utils::{connect_test_database, test_encryption_keys};
    use uuid::Uuid;

    #[test]
    fn validate_onboard_input_trims_and_enforces_bounds() {
        let input = validate_onboard_input(DeviceOnboardInput {
            org_id: None,
            label: " Kitchen ".to_string(),
            default_services: None,
            base_url: " https://api.example.com/ ".to_string(),
        })
        .expect("valid input");

        assert_eq!(input.label, "Kitchen");
        assert_eq!(input.base_url, "https://api.example.com");

        assert!(validate_onboard_input(test_input_with_label("")).is_err());
        assert!(validate_onboard_input(test_input_with_label(&"x".repeat(129))).is_err());
    }

    #[test]
    fn build_qr_payload_percent_encodes_bootstrap_fields() {
        let payload = build_qr_payload(
            "nyx_obt_secret",
            "bootstrap-1",
            "https://api.example.com",
            900,
        );

        assert_eq!(
            payload,
            "nyxprov://bootstrap?token=nyx_obt_secret&id=bootstrap-1&url=https%3A%2F%2Fapi.example.com&exp=900"
        );
    }

    #[test]
    fn validate_bootstrap_token_requires_expected_prefix_and_entropy() {
        let valid = format!("{DEVICE_ONBOARD_TOKEN_PREFIX}{}", "a".repeat(64));
        assert_eq!(validate_bootstrap_token(&valid).unwrap(), valid);
        assert!(validate_bootstrap_token("nyxid_ag_secret").is_err());
        assert!(validate_bootstrap_token(&format!("{DEVICE_ONBOARD_TOKEN_PREFIX}short")).is_err());
        assert!(
            validate_bootstrap_token(&format!("{DEVICE_ONBOARD_TOKEN_PREFIX}{}", "z".repeat(64)))
                .is_err()
        );
    }

    #[tokio::test]
    async fn onboard_creates_short_lived_bootstrap_without_durable_credentials() {
        let Some(db) = connect_test_database("device_onboard_bootstrap").await else {
            return;
        };
        crate::db::ensure_indexes(&db).await.expect("indexes");
        let actor_user_id = Uuid::new_v4().to_string();

        let response = onboard(
            &db,
            &actor_user_id,
            DeviceOnboardInput {
                org_id: None,
                label: "Kitchen Camera".to_string(),
                default_services: None,
                base_url: "https://api.example.com".to_string(),
            },
        )
        .await
        .expect("onboard");

        assert_eq!(response.label, "Kitchen Camera");
        assert_eq!(response.expires_in, DEVICE_ONBOARD_EXPIRES_IN_SECS);
        let parsed = parsed_qr_query(&response.qr_payload);
        let token = parsed.get("token").expect("bootstrap token");
        assert!(token.starts_with(DEVICE_ONBOARD_TOKEN_PREFIX));
        assert!(!parsed.contains_key("key"));
        assert!(!parsed.contains_key("refresh"));
        assert!(!parsed.contains_key("psw"));
        assert_eq!(
            parsed.get("url").map(String::as_str),
            Some("https://api.example.com")
        );

        let stored = db
            .collection::<DeviceOnboardCredential>(DEVICE_ONBOARD_CREDENTIALS)
            .find_one(doc! { "_id": &response.bootstrap_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.owner_user_id, actor_user_id);
        assert_eq!(stored.bootstrap_token_hash, hash_token(token));
        assert_eq!(stored.label, "Kitchen Camera");
        assert!(stored.default_service_ids.is_empty());
        assert!(!stored.used);
        assert!(stored.expires_at > Utc::now());
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
    async fn redeem_onboard_consumes_bootstrap_and_issues_scoped_credentials() {
        let Some(db) = connect_test_database("device_onboard_redeem").await else {
            return;
        };
        crate::db::ensure_indexes(&db).await.expect("indexes");
        let actor_user_id = Uuid::new_v4().to_string();
        let service = insert_user_service(&db, &actor_user_id, "svc-by-slug").await;
        let bootstrap = onboard(
            &db,
            &actor_user_id,
            DeviceOnboardInput {
                org_id: None,
                label: "Lab Camera".to_string(),
                default_services: Some(vec![service.slug.clone()]),
                base_url: "https://api.example.com".to_string(),
            },
        )
        .await
        .expect("onboard");
        let token = parsed_qr_query(&bootstrap.qr_payload)
            .remove("token")
            .expect("token");

        let response = redeem_onboard(
            &db,
            &test_encryption_keys(),
            DeviceOnboardRedeemInput {
                bootstrap_token: token.clone(),
            },
        )
        .await
        .expect("redeem");

        assert!(response.api_key.starts_with("nyxid_ag_"));
        assert_eq!(response.refresh_token.len(), 64);

        let stored_key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "allowed_node_ids": &response.node_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored_key.platform.as_deref(),
            Some(DEVICE_ONBOARD_API_KEY_PLATFORM)
        );
        assert_eq!(stored_key.scopes, DEVICE_CODE_API_KEY_SCOPES);
        assert!(!stored_key.allow_all_services);
        assert_eq!(stored_key.allowed_service_ids, vec![service.id]);
        assert!(!stored_key.allow_all_nodes);
        assert_eq!(stored_key.allowed_node_ids, vec![response.node_id.clone()]);

        let node = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": &response.node_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(node.auth_token_hash.len(), 64);
        assert_eq!(node.signing_secret_hash.len(), 64);
        assert!(node.signing_secret_encrypted.is_some());
        assert_eq!(
            node.metadata
                .as_ref()
                .and_then(|metadata| metadata.provisioning_source.as_deref()),
            Some(DEVICE_ONBOARD_PROVISIONING_SOURCE)
        );

        let credential = db
            .collection::<DeviceOnboardCredential>(DEVICE_ONBOARD_CREDENTIALS)
            .find_one(doc! { "_id": &bootstrap.bootstrap_id })
            .await
            .unwrap()
            .unwrap();
        assert!(credential.used);
        assert_eq!(credential.redeemed_api_key_id, Some(stored_key.id));
        assert_eq!(credential.redeemed_node_id, Some(response.node_id));
        assert_eq!(
            credential.redeemed_refresh_token_hash,
            Some(hash_token(&response.refresh_token))
        );

        let replay = redeem_onboard(
            &db,
            &test_encryption_keys(),
            DeviceOnboardRedeemInput {
                bootstrap_token: token,
            },
        )
        .await
        .expect_err("bootstrap is single-use");
        assert!(matches!(replay, AppError::DeviceCodeExpired));
    }

    #[tokio::test]
    async fn revoke_onboard_deletes_unredeemed_bootstrap_and_blocks_redeem() {
        let Some(db) = connect_test_database("device_onboard_revoke").await else {
            return;
        };
        crate::db::ensure_indexes(&db).await.expect("indexes");
        let actor_user_id = Uuid::new_v4().to_string();
        let bootstrap = onboard(
            &db,
            &actor_user_id,
            DeviceOnboardInput {
                org_id: None,
                label: "Lab Camera".to_string(),
                default_services: None,
                base_url: "https://api.example.com".to_string(),
            },
        )
        .await
        .expect("onboard");
        let token = parsed_qr_query(&bootstrap.qr_payload)
            .remove("token")
            .expect("token");

        revoke_onboard(&db, &actor_user_id, &bootstrap.bootstrap_id)
            .await
            .expect("revoke");

        let stored = db
            .collection::<DeviceOnboardCredential>(DEVICE_ONBOARD_CREDENTIALS)
            .find_one(doc! { "_id": &bootstrap.bootstrap_id })
            .await
            .unwrap();
        assert!(stored.is_none());
        let redeem = redeem_onboard(
            &db,
            &test_encryption_keys(),
            DeviceOnboardRedeemInput {
                bootstrap_token: token,
            },
        )
        .await
        .expect_err("revoked bootstrap must not redeem");
        assert!(matches!(redeem, AppError::DeviceCodeExpired));
        assert_no_durable_credentials(&db).await;
    }

    #[tokio::test]
    async fn revoke_onboard_requires_owner_write_access() {
        let Some(db) = connect_test_database("device_onboard_revoke_acl").await else {
            return;
        };
        crate::db::ensure_indexes(&db).await.expect("indexes");
        let actor_user_id = Uuid::new_v4().to_string();
        let other_user_id = Uuid::new_v4().to_string();
        let bootstrap = onboard(
            &db,
            &actor_user_id,
            DeviceOnboardInput {
                org_id: None,
                label: "Lab Camera".to_string(),
                default_services: None,
                base_url: "https://api.example.com".to_string(),
            },
        )
        .await
        .expect("onboard");

        let error = revoke_onboard(&db, &other_user_id, &bootstrap.bootstrap_id)
            .await
            .expect_err("other user cannot revoke");

        assert!(matches!(error, AppError::NotFound(_)));
        let stored = db
            .collection::<DeviceOnboardCredential>(DEVICE_ONBOARD_CREDENTIALS)
            .find_one(doc! { "_id": &bootstrap.bootstrap_id })
            .await
            .unwrap();
        assert!(stored.is_some());
    }

    #[tokio::test]
    async fn onboard_unknown_default_service_returns_not_found_without_partials() {
        let Some(db) = connect_test_database("device_onboard_unknown_service").await else {
            return;
        };
        let actor_user_id = Uuid::new_v4().to_string();

        let error = onboard(
            &db,
            &actor_user_id,
            DeviceOnboardInput {
                org_id: None,
                label: "Lab Camera".to_string(),
                default_services: Some(vec!["missing-svc".to_string()]),
                base_url: "https://api.example.com".to_string(),
            },
        )
        .await
        .expect_err("unknown service should fail");

        assert!(matches!(error, AppError::NotFound(_)));
        assert_no_partial_onboard(&db).await;
    }

    #[tokio::test]
    async fn redeem_expired_bootstrap_does_not_issue_partials() {
        let Some(db) = connect_test_database("device_onboard_expired_redeem").await else {
            return;
        };
        let token = generate_bootstrap_token();
        let now = Utc::now();
        db.collection::<DeviceOnboardCredential>(DEVICE_ONBOARD_CREDENTIALS)
            .insert_one(DeviceOnboardCredential {
                id: Uuid::new_v4().to_string(),
                owner_user_id: Uuid::new_v4().to_string(),
                bootstrap_token_hash: hash_token(&token),
                label: "Expired Camera".to_string(),
                default_service_ids: Vec::new(),
                used: false,
                redeemed_api_key_id: None,
                redeemed_node_id: None,
                redeemed_refresh_token_hash: None,
                created_at: now - Duration::minutes(20),
                expires_at: now - Duration::minutes(1),
            })
            .await
            .unwrap();

        let error = redeem_onboard(
            &db,
            &test_encryption_keys(),
            DeviceOnboardRedeemInput {
                bootstrap_token: token,
            },
        )
        .await
        .expect_err("expired token");

        assert!(matches!(error, AppError::DeviceCodeExpired));
        assert_no_durable_credentials(&db).await;
    }

    fn test_input_with_label(label: &str) -> DeviceOnboardInput {
        DeviceOnboardInput {
            org_id: None,
            label: label.to_string(),
            default_services: None,
            base_url: "https://api.example.com".to_string(),
        }
    }

    fn parsed_qr_query(payload: &str) -> std::collections::BTreeMap<String, String> {
        let query = payload
            .strip_prefix("nyxprov://bootstrap?")
            .expect("nyxprov bootstrap payload");
        url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect()
    }

    async fn assert_no_partial_onboard(db: &Database) {
        assert_no_durable_credentials(db).await;
        assert_eq!(
            db.collection::<DeviceOnboardCredential>(DEVICE_ONBOARD_CREDENTIALS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }

    async fn assert_no_durable_credentials(db: &Database) {
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
            admin_only: false,
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
}
