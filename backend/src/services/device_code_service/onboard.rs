use chrono::Utc;
use mongodb::{Database, bson::doc};
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
use super::{DEVICE_CODE_API_KEY_SCOPES, DeviceOnboard, DeviceOnboardInput};

const DEVICE_ONBOARD_API_KEY_PLATFORM: &str = "device-onboard";
const DEVICE_ONBOARD_HW_ID: &str = "qr-onboard";

/// Provision a headless device by returning a one-time QR payload.
///
/// The WiFi password is only held in memory while this function builds the
/// `nyxprov://` payload. It is never written to MongoDB, audit logs, or tracing
/// fields.
pub async fn onboard(
    db: &Database,
    encryption_keys: &EncryptionKeys,
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

    let allowed_service_ids =
        resolve_default_service_ids(db, &owner_user_id, input.default_services.as_deref()).await?;
    let empty_node_ids: Vec<String> = Vec::new();
    let created_key = key_service::create_api_key(
        db,
        &owner_user_id,
        &input.label,
        DEVICE_CODE_API_KEY_SCOPES,
        None,
        Some("QR-onboarded device"),
        Some(&allowed_service_ids),
        Some(&empty_node_ids),
        Some(false),
        Some(false),
        None,
        None,
        Some(DEVICE_ONBOARD_API_KEY_PLATFORM),
        None,
    )
    .await?;

    let node = match node_service::create_for_device(
        db,
        encryption_keys,
        DeviceNodeInput {
            user_id: &owner_user_id,
            api_key_id: &created_key.id,
            hw_id: DEVICE_ONBOARD_HW_ID,
            label: &input.label,
            device_pubkey: None,
            provisioning_source: DEVICE_ONBOARD_PROVISIONING_SOURCE,
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
    let credential = DeviceOnboardCredential {
        id: uuid::Uuid::new_v4().to_string(),
        owner_user_id: owner_user_id.clone(),
        api_key_id: created_key.id.clone(),
        node_id: node.id.clone(),
        refresh_token_hash: hash_token(refresh_token.as_str()),
        created_at: Utc::now(),
    };
    if let Err(error) = db
        .collection::<DeviceOnboardCredential>(DEVICE_ONBOARD_CREDENTIALS)
        .insert_one(&credential)
        .await
    {
        cleanup_partial_onboard(db, &owner_user_id, Some(&created_key.id), Some(&node.id)).await;
        return Err(error.into());
    }

    let qr_payload = build_qr_payload(
        &input.wifi_ssid,
        &input.wifi_password,
        &created_key.full_key,
        &node.id,
        refresh_token.as_str(),
        &input.base_url,
    );

    Ok(DeviceOnboard {
        qr_payload,
        node_id: node.id,
        api_key_id: created_key.id,
        label: input.label,
    })
}

fn validate_onboard_input(mut input: DeviceOnboardInput) -> AppResult<DeviceOnboardInput> {
    input.label = input.label.trim().to_string();
    input.wifi_ssid = input.wifi_ssid.trim().to_string();
    input.base_url = input.base_url.trim().trim_end_matches('/').to_string();

    if input.label.is_empty() || input.label.len() > 128 {
        return Err(AppError::ValidationError(
            "label must be between 1 and 128 characters".to_string(),
        ));
    }
    if input.wifi_ssid.is_empty() || input.wifi_ssid.len() > 32 {
        return Err(AppError::ValidationError(
            "wifi_ssid must be between 1 and 32 characters".to_string(),
        ));
    }
    if input.wifi_password.len() < 8 || input.wifi_password.len() > 63 {
        return Err(AppError::ValidationError(
            "wifi_password must be between 8 and 63 characters".to_string(),
        ));
    }
    if input.base_url.is_empty() {
        return Err(AppError::ValidationError(
            "base_url must not be empty".to_string(),
        ));
    }

    Ok(input)
}

fn build_qr_payload(
    ssid: &str,
    password: &str,
    api_key: &str,
    node_id: &str,
    refresh_token: &str,
    base_url: &str,
) -> String {
    format!(
        "nyxprov://full?ssid={}&psw={}&key={}&node={}&refresh={}&url={}",
        urlencoding::encode(ssid),
        urlencoding::encode(password),
        urlencoding::encode(api_key),
        urlencoding::encode(node_id),
        urlencoding::encode(refresh_token),
        urlencoding::encode(base_url),
    )
}

async fn cleanup_partial_onboard(
    db: &Database,
    owner_user_id: &str,
    api_key_id: Option<&str>,
    node_id: Option<&str>,
) {
    if let Some(node_id) = node_id
        && let Err(error) = db
            .collection::<DeviceOnboardCredential>(DEVICE_ONBOARD_CREDENTIALS)
            .delete_one(doc! { "node_id": node_id, "owner_user_id": owner_user_id })
            .await
    {
        tracing::warn!(
            node_id = %node_id,
            user_id = %owner_user_id,
            error = %error,
            "Failed to clean up partial device onboard credential"
        );
    }

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
            wifi_ssid: " Home ".to_string(),
            wifi_password: "hunter22".to_string(),
            default_services: None,
            base_url: " https://api.example.com/ ".to_string(),
        })
        .expect("valid input");

        assert_eq!(input.label, "Kitchen");
        assert_eq!(input.wifi_ssid, "Home");
        assert_eq!(input.base_url, "https://api.example.com");

        assert!(validate_onboard_input(test_input_with_password("short")).is_err());
        assert!(validate_onboard_input(test_input_with_ssid(&"x".repeat(33))).is_err());
        assert!(validate_onboard_input(test_input_with_label(&"x".repeat(129))).is_err());
    }

    #[test]
    fn build_qr_payload_percent_encodes_sensitive_fields() {
        let payload = build_qr_payload(
            "Home & Lab",
            "p@ss word/1",
            "nyxid_ag_secret",
            "node-1",
            "refresh/1",
            "https://api.example.com",
        );

        assert_eq!(
            payload,
            "nyxprov://full?ssid=Home%20%26%20Lab&psw=p%40ss%20word%2F1&key=nyxid_ag_secret&node=node-1&refresh=refresh%2F1&url=https%3A%2F%2Fapi.example.com"
        );
    }

    #[tokio::test]
    async fn onboard_without_default_services_issues_empty_service_allowlist() {
        let Some(db) = connect_test_database("device_onboard_empty_services").await else {
            return;
        };
        crate::db::ensure_indexes(&db).await.expect("indexes");
        let actor_user_id = Uuid::new_v4().to_string();

        let response = onboard(
            &db,
            &test_encryption_keys(),
            &actor_user_id,
            DeviceOnboardInput {
                org_id: None,
                label: "Kitchen Camera".to_string(),
                wifi_ssid: "MyHomeNetwork".to_string(),
                wifi_password: "hunter22".to_string(),
                default_services: None,
                base_url: "https://api.example.com".to_string(),
            },
        )
        .await
        .expect("onboard");

        assert_eq!(response.label, "Kitchen Camera");
        let parsed = parsed_qr_query(&response.qr_payload);
        assert_eq!(
            parsed.get("ssid").map(String::as_str),
            Some("MyHomeNetwork")
        );
        assert_eq!(parsed.get("psw").map(String::as_str), Some("hunter22"));
        assert_eq!(
            parsed.get("node").map(String::as_str),
            Some(response.node_id.as_str())
        );
        assert_eq!(
            parsed.get("url").map(String::as_str),
            Some("https://api.example.com")
        );
        let api_key = parsed.get("key").expect("raw api key");
        let refresh_token = parsed.get("refresh").expect("refresh token");
        assert!(api_key.starts_with("nyxid_ag_"));
        assert_eq!(refresh_token.len(), 64);

        let stored_key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &response.api_key_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored_key.platform.as_deref(),
            Some(DEVICE_ONBOARD_API_KEY_PLATFORM)
        );
        assert_eq!(stored_key.scopes, DEVICE_CODE_API_KEY_SCOPES);
        assert!(!stored_key.allow_all_services);
        assert!(stored_key.allowed_service_ids.is_empty());
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
            .find_one(doc! { "node_id": &response.node_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(credential.api_key_id, response.api_key_id);
        assert_eq!(credential.refresh_token_hash, hash_token(refresh_token));
    }

    #[tokio::test]
    async fn onboard_allows_default_services_by_uuid_and_slug() {
        let Some(db) = connect_test_database("device_onboard_default_services").await else {
            return;
        };
        crate::db::ensure_indexes(&db).await.expect("indexes");
        let actor_user_id = Uuid::new_v4().to_string();
        let service_by_id = insert_user_service(&db, &actor_user_id, "svc-by-id").await;
        let service_by_slug = insert_user_service(&db, &actor_user_id, "svc-by-slug").await;

        let response = onboard(
            &db,
            &test_encryption_keys(),
            &actor_user_id,
            DeviceOnboardInput {
                org_id: None,
                label: "Lab Camera".to_string(),
                wifi_ssid: "Lab".to_string(),
                wifi_password: "hunter22".to_string(),
                default_services: Some(vec![
                    service_by_id.id.clone(),
                    service_by_slug.slug.clone(),
                ]),
                base_url: "https://api.example.com".to_string(),
            },
        )
        .await
        .expect("onboard");

        let api_key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": &response.api_key_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            api_key.allowed_service_ids,
            vec![service_by_id.id, service_by_slug.id]
        );
    }

    #[tokio::test]
    async fn onboard_unknown_default_service_returns_not_found_without_partials() {
        let Some(db) = connect_test_database("device_onboard_unknown_service").await else {
            return;
        };
        let actor_user_id = Uuid::new_v4().to_string();

        let error = onboard(
            &db,
            &test_encryption_keys(),
            &actor_user_id,
            DeviceOnboardInput {
                org_id: None,
                label: "Lab Camera".to_string(),
                wifi_ssid: "Lab".to_string(),
                wifi_password: "hunter22".to_string(),
                default_services: Some(vec!["missing-svc".to_string()]),
                base_url: "https://api.example.com".to_string(),
            },
        )
        .await
        .expect_err("unknown service should fail");

        assert!(matches!(error, AppError::NotFound(_)));
        assert_no_partial_onboard(&db).await;
    }

    fn test_input_with_password(password: &str) -> DeviceOnboardInput {
        DeviceOnboardInput {
            org_id: None,
            label: "Kitchen".to_string(),
            wifi_ssid: "Home".to_string(),
            wifi_password: password.to_string(),
            default_services: None,
            base_url: "https://api.example.com".to_string(),
        }
    }

    fn test_input_with_ssid(ssid: &str) -> DeviceOnboardInput {
        DeviceOnboardInput {
            wifi_ssid: ssid.to_string(),
            ..test_input_with_password("hunter22")
        }
    }

    fn test_input_with_label(label: &str) -> DeviceOnboardInput {
        DeviceOnboardInput {
            label: label.to_string(),
            ..test_input_with_password("hunter22")
        }
    }

    fn parsed_qr_query(payload: &str) -> std::collections::BTreeMap<String, String> {
        let query = payload
            .strip_prefix("nyxprov://full?")
            .expect("nyxprov full payload");
        url::form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect()
    }

    async fn assert_no_partial_onboard(db: &Database) {
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
        assert_eq!(
            db.collection::<DeviceOnboardCredential>(DEVICE_ONBOARD_CREDENTIALS)
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
