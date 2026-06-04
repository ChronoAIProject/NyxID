use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{extract_ip, extract_user_agent};
use crate::models::node::Node;
use crate::models::node_pending_credential::NodePendingCredential;
use crate::services::{
    audit_service, node_pending_credential_service, node_service,
    rci_audit_service::{self, RciAuditEventKind, RciAuditSubject},
};

#[derive(Debug, Deserialize)]
pub struct DeclinePendingCredentialRequest {
    #[serde(default)]
    pub reason: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NodeAgentPendingCredentialInfo {
    pub id: String,
    pub service_slug: String,
    pub injection_method: String,
    pub field_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub crypto: Option<NodeAgentPendingCredentialCryptoInfo>,
}

#[derive(Debug, Serialize)]
pub struct NodeAgentPendingCredentialCryptoInfo {
    pub version: String,
}

#[derive(Debug, Serialize)]
pub struct NodeAgentPendingCredentialListResponse {
    pub pending_credentials: Vec<NodeAgentPendingCredentialInfo>,
}

async fn authenticate_node(state: &AppState, headers: &HeaderMap) -> AppResult<Node> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or_else(|| AppError::Unauthorized("Missing node bearer token".to_string()))?;

    node_service::validate_auth_token(&state.db, token).await
}

fn pending_info(
    pending: NodePendingCredential,
    include_remote_crypto: bool,
) -> NodeAgentPendingCredentialInfo {
    let crypto = if include_remote_crypto {
        pending
            .crypto
            .as_ref()
            .filter(|crypto| crypto.version == "v1")
            .map(|crypto| NodeAgentPendingCredentialCryptoInfo {
                version: crypto.version.clone(),
            })
    } else {
        None
    };

    NodeAgentPendingCredentialInfo {
        id: pending.id,
        service_slug: pending.service_slug,
        injection_method: pending.injection_method.as_str().to_string(),
        field_name: pending.field_name,
        target_url: pending.target_url,
        label: pending.label,
        created_at: pending.created_at.to_rfc3339(),
        expires_at: pending.expires_at.to_rfc3339(),
        crypto,
    }
}

fn pending_completion_audit_event_type(
    pending: &NodePendingCredential,
    legacy_event_type: &'static str,
    rci_kind: RciAuditEventKind,
) -> &'static str {
    if RciAuditSubject::pending_is_rci(pending) {
        rci_kind.event_type()
    } else {
        legacy_event_type
    }
}

fn log_pending_completion_audit(
    state: &AppState,
    headers: &HeaderMap,
    pending: &NodePendingCredential,
    legacy_event_type: &'static str,
    rci_kind: RciAuditEventKind,
) {
    if pending_completion_audit_event_type(pending, legacy_event_type, rci_kind)
        == rci_kind.event_type()
    {
        let subject = RciAuditSubject::from_pending(pending);
        rci_audit_service::log_rci_for_node(
            state.db.clone(),
            &pending.owner_user_id,
            extract_ip(headers),
            extract_user_agent(headers),
            &subject,
            rci_kind,
        );
    } else {
        audit_service::log_async(
            state.db.clone(),
            Some(pending.owner_user_id.clone()),
            legacy_event_type.to_string(),
            Some(serde_json::json!({
                "node_id": &pending.node_id,
                "pending_credential_id": &pending.id,
                "service_slug": &pending.service_slug,
                "owner_user_id": &pending.owner_user_id,
            })),
            extract_ip(headers),
            extract_user_agent(headers),
            None,
            None,
        );
    }
}

/// GET /api/v1/node-agent/pending-credentials
pub async fn list_pending_credentials(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<NodeAgentPendingCredentialListResponse>> {
    let node = authenticate_node(&state, &headers).await?;
    let pending =
        node_pending_credential_service::list_pending_credentials_for_node(&state.db, &node.id)
            .await?;
    let include_remote_crypto = state
        .node_ws_manager
        .supports_remote_credential_crypto(&node.id);

    Ok(Json(NodeAgentPendingCredentialListResponse {
        pending_credentials: pending
            .into_iter()
            .map(|pending| pending_info(pending, include_remote_crypto))
            .collect(),
    }))
}

/// POST /api/v1/node-agent/pending-credentials/{pending_id}/consume
pub async fn consume_pending_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pending_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let node = authenticate_node(&state, &headers).await?;
    let pending = node_pending_credential_service::consume_pending_credential_for_node(
        &state.db,
        &node.id,
        &pending_id,
    )
    .await?;

    log_pending_completion_audit(
        &state,
        &headers,
        &pending,
        "node_credential_push_consumed",
        RciAuditEventKind::Consumed,
    );

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/node-agent/pending-credentials/{pending_id}/decline
pub async fn decline_pending_credential(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(pending_id): Path<String>,
    Json(body): Json<Option<DeclinePendingCredentialRequest>>,
) -> AppResult<impl IntoResponse> {
    let node = authenticate_node(&state, &headers).await?;
    let pending = node_pending_credential_service::decline_pending_credential_for_node(
        &state.db,
        &node.id,
        &pending_id,
    )
    .await?;

    let reason_present = body
        .as_ref()
        .and_then(|body| body.reason.as_deref())
        .is_some_and(|reason| !reason.trim().is_empty());
    if RciAuditSubject::pending_is_rci(&pending) {
        log_pending_completion_audit(
            &state,
            &headers,
            &pending,
            "node_credential_push_declined",
            RciAuditEventKind::Declined { reason_present },
        );
    } else {
        audit_service::log_async(
            state.db.clone(),
            Some(pending.owner_user_id.clone()),
            "node_credential_push_declined".to_string(),
            Some(serde_json::json!({
                "node_id": &node.id,
                "pending_credential_id": &pending.id,
                "service_slug": &pending.service_slug,
                "owner_user_id": &pending.owner_user_id,
                "reason_present": reason_present,
            })),
            extract_ip(&headers),
            extract_user_agent(&headers),
            None,
            None,
        );
    }

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::{
        DeclinePendingCredentialRequest, consume_pending_credential, decline_pending_credential,
        pending_completion_audit_event_type, pending_info,
    };
    use crate::crypto::token::hash_token;
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::node_pending_credential::{
        COLLECTION_NAME as NODE_PENDING_CREDENTIALS, CryptoBundle, InjectionMethod,
        NodePendingCredential, RemoteCryptoState,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::services::{
        audit_service, node_pending_credential_service, rci_audit_service::RciAuditEventKind,
    };
    use crate::test_utils::{connect_test_database, test_app_state, test_user};
    use axum::{
        Json,
        extract::{Path, State},
        http::{HeaderMap, StatusCode, header},
        response::IntoResponse,
    };
    use chrono::{Duration, Utc};
    use mongodb::bson::doc;
    use uuid::Uuid;

    #[test]
    fn decline_request_accepts_empty_json_object() {
        let parsed: Option<DeclinePendingCredentialRequest> =
            serde_json::from_str("{}").expect("empty object parses");
        assert!(parsed.expect("request body").reason.is_none());
    }

    fn test_pending(crypto: Option<CryptoBundle>) -> NodePendingCredential {
        let now = Utc::now();
        NodePendingCredential {
            id: "pending-1".to_string(),
            node_id: "node-1".to_string(),
            service_slug: "openclaw".to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "X-API-Key".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: "user-1".to_string(),
            owner_user_id: "user-1".to_string(),
            created_at: now,
            expires_at: now + Duration::minutes(5),
            consumed_at: None,
            declined_at: None,
            crypto,
            remote_state: None,
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: true,
        }
    }

    async fn test_db(prefix: &str) -> mongodb::Database {
        connect_test_database(prefix)
            .await
            .expect("local MongoDB required for node-agent audit tests")
    }

    fn test_node(owner_id: &str, raw_auth_token: &str) -> Node {
        let now = Utc::now();
        Node {
            id: Uuid::new_v4().to_string(),
            user_id: owner_id.to_string(),
            name: "node-agent-audit".to_string(),
            status: NodeStatus::Offline,
            auth_token_hash: hash_token(raw_auth_token),
            signing_secret_encrypted: None,
            signing_secret_hash: "signing-hash".to_string(),
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: NodeMetrics::default(),
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn node_headers(raw_auth_token: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::AUTHORIZATION,
            format!("Bearer {raw_auth_token}").parse().unwrap(),
        );
        headers.insert(header::USER_AGENT, "nyxid-node-test".parse().unwrap());
        headers.insert("x-forwarded-for", "198.51.100.20".parse().unwrap());
        headers
    }

    async fn create_remote_pending(
        db: &mongodb::Database,
        actor_id: &str,
        node_id: &str,
        service_slug: &str,
    ) -> NodePendingCredential {
        node_pending_credential_service::create_pending_credential(
            db,
            actor_id,
            node_id,
            node_pending_credential_service::CreatePendingCredentialInput {
                service_slug: service_slug.to_string(),
                injection_method: InjectionMethod::Header,
                field_name: "X-API-Key".to_string(),
                target_url: None,
                label: Some("Production".to_string()),
                ttl_secs: 86_400,
                remote_crypto: true,
            },
        )
        .await
        .expect("create remote pending credential")
    }

    async fn load_pending(db: &mongodb::Database, pending_id: &str) -> NodePendingCredential {
        db.collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
            .find_one(doc! { "_id": pending_id })
            .await
            .expect("query pending credential")
            .expect("pending credential exists")
    }

    async fn load_audit_entry(
        db: &mongodb::Database,
        receiver: tokio::sync::oneshot::Receiver<String>,
    ) -> AuditLog {
        let audit_id = receiver.await.expect("audit write notification");
        db.collection::<AuditLog>(AUDIT_LOG)
            .find_one(doc! { "_id": audit_id })
            .await
            .expect("query audit log")
            .expect("audit log exists")
    }

    fn sorted_strings(values: &[&str]) -> Vec<String> {
        let mut values: Vec<String> = values.iter().map(|value| value.to_string()).collect();
        values.sort();
        values
    }

    fn assert_rci_audit_row(
        entry: &AuditLog,
        expected_event_type: &str,
        pending: &NodePendingCredential,
        expected_remote_state: &str,
        extra_keys: &[&str],
    ) {
        assert_eq!(entry.event_type, expected_event_type);
        assert_eq!(
            entry.user_id.as_deref(),
            Some(pending.owner_user_id.as_str())
        );
        assert_eq!(entry.ip_address.as_deref(), Some("198.51.100.20"));
        assert_eq!(entry.user_agent.as_deref(), Some("nyxid-node-test"));
        let event_data = entry.event_data.as_ref().expect("audit event data");
        let object = event_data.as_object().expect("audit event data object");
        let mut expected_keys = vec![
            "event_at",
            "flow",
            "node_id",
            "owner_user_id",
            "pending_created_at",
            "pending_credential_id",
            "pending_expires_at",
            "remote_state",
            "routed_via",
            "service_slug",
        ];
        expected_keys.extend(extra_keys.iter().copied());
        let mut actual_keys: Vec<String> = object.keys().cloned().collect();
        actual_keys.sort();
        assert_eq!(actual_keys, sorted_strings(&expected_keys));
        assert_eq!(object["flow"], "remote_credential_injection");
        assert_eq!(object["routed_via"], "node");
        assert_eq!(object["node_id"], pending.node_id);
        assert_eq!(object["pending_credential_id"], pending.id);
        assert_eq!(object["service_slug"], pending.service_slug);
        assert_eq!(object["owner_user_id"], pending.owner_user_id);
        assert_eq!(object["remote_state"], expected_remote_state);

        for forbidden_key in [
            "plaintext",
            "secret",
            "ciphertext",
            "nonce",
            "node_pubkey",
            "admin_pubkey",
            "hash",
            "fingerprint",
            "length",
            "bytes",
            "target_url",
            "field_name",
            "injection_method",
            "raw_version",
            "raw_status",
            "raw_node_error",
            "raw_decline_reason",
            "queue_count",
            "queued_pending_ids",
        ] {
            assert!(
                !object.contains_key(forbidden_key),
                "{expected_event_type}: {forbidden_key}"
            );
        }
        let event_json = event_data.to_string();
        for forbidden_value in [
            "super-secret-plaintext-fixture",
            "secret-value-fixture",
            "raw-node-error-fixture",
            "decline-reason-fixture",
        ] {
            assert!(!event_json.contains(forbidden_value), "{forbidden_value}");
        }
    }

    #[test]
    fn pending_info_omits_crypto_without_capability() {
        let info = pending_info(
            test_pending(Some(CryptoBundle {
                version: "v1".to_string(),
                node_pubkey: String::new(),
                admin_pubkey: None,
                nonce: None,
                ciphertext: None,
            })),
            false,
        );

        let json = serde_json::to_value(&info).expect("serialize");
        assert!(json.get("crypto").is_none());
    }

    #[test]
    fn pending_info_includes_crypto_version_with_capability() {
        let info = pending_info(
            test_pending(Some(CryptoBundle {
                version: "v1".to_string(),
                node_pubkey: String::new(),
                admin_pubkey: None,
                nonce: None,
                ciphertext: None,
            })),
            true,
        );

        let json = serde_json::to_value(&info).expect("serialize");
        assert_eq!(json["crypto"]["version"], "v1");
        assert!(json["crypto"].get("node_pubkey").is_none());
    }

    #[tokio::test]
    async fn node_agent_consume_and_decline_write_rci_audit_rows() {
        let db = test_db("node_agent_rci_completion_audit").await;
        let owner_id = Uuid::new_v4().to_string();
        let raw_auth_token = "nyx_nauth_node_agent_audit";
        let node = test_node(&owner_id, raw_auth_token);
        db.collection(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .expect("insert user");
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .expect("insert node");
        let consume_pending = create_remote_pending(&db, &owner_id, &node.id, "consume-rci").await;
        let decline_pending = create_remote_pending(&db, &owner_id, &node.id, "decline-rci").await;
        let state = test_app_state(db.clone());
        let consume_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_consumed",
            Some(consume_pending.id.clone()),
        );
        let decline_audit = audit_service::notify_on_audit_write(
            "node_credential_rci_declined",
            Some(decline_pending.id.clone()),
        );

        let consume_response = consume_pending_credential(
            State(state.clone()),
            node_headers(raw_auth_token),
            Path(consume_pending.id.clone()),
        )
        .await
        .expect("consume pending credential")
        .into_response();
        assert_eq!(consume_response.status(), StatusCode::NO_CONTENT);

        let decline_response = decline_pending_credential(
            State(state),
            node_headers(raw_auth_token),
            Path(decline_pending.id.clone()),
            Json(Some(DeclinePendingCredentialRequest {
                reason: Some("decline-reason-fixture".to_string()),
            })),
        )
        .await
        .expect("decline pending credential")
        .into_response();
        assert_eq!(decline_response.status(), StatusCode::NO_CONTENT);

        let consumed = load_pending(&db, &consume_pending.id).await;
        let declined = load_pending(&db, &decline_pending.id).await;
        let consume_entry = load_audit_entry(&db, consume_audit).await;
        assert_rci_audit_row(
            &consume_entry,
            "node_credential_rci_consumed",
            &consumed,
            "consumed",
            &[],
        );
        let decline_entry = load_audit_entry(&db, decline_audit).await;
        assert_rci_audit_row(
            &decline_entry,
            "node_credential_rci_declined",
            &declined,
            "declined",
            &["reason_present"],
        );
        assert_eq!(
            decline_entry.event_data.as_ref().unwrap()["reason_present"],
            true
        );
    }

    #[test]
    fn regression_legacy_cli_flow_unchanged() {
        let legacy = test_pending(None);
        assert_eq!(
            pending_completion_audit_event_type(
                &legacy,
                "node_credential_push_consumed",
                RciAuditEventKind::Consumed,
            ),
            "node_credential_push_consumed"
        );
        assert_eq!(
            pending_completion_audit_event_type(
                &legacy,
                "node_credential_push_declined",
                RciAuditEventKind::Declined {
                    reason_present: true,
                },
            ),
            "node_credential_push_declined"
        );

        let rci_crypto = test_pending(Some(CryptoBundle {
            version: "v1".to_string(),
            node_pubkey: String::new(),
            admin_pubkey: None,
            nonce: None,
            ciphertext: None,
        }));
        assert_eq!(
            pending_completion_audit_event_type(
                &rci_crypto,
                "node_credential_push_consumed",
                RciAuditEventKind::Consumed,
            ),
            "node_credential_rci_consumed"
        );

        let mut rci_remote_state = test_pending(None);
        rci_remote_state.remote_state = Some(RemoteCryptoState::PubkeyPosted);
        assert_eq!(
            pending_completion_audit_event_type(
                &rci_remote_state,
                "node_credential_push_declined",
                RciAuditEventKind::Declined {
                    reason_present: false,
                },
            ),
            "node_credential_rci_declined"
        );
    }
}
