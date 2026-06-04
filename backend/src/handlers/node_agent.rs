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
use crate::services::{audit_service, node_pending_credential_service, node_service};

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

    audit_service::log_async(
        state.db.clone(),
        Some(pending.owner_user_id.clone()),
        "node_credential_push_consumed".to_string(),
        Some(serde_json::json!({
            "node_id": &node.id,
            "pending_credential_id": &pending.id,
            "service_slug": &pending.service_slug,
            "owner_user_id": &pending.owner_user_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
        None,
        None,
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

    audit_service::log_async(
        state.db.clone(),
        Some(pending.owner_user_id.clone()),
        "node_credential_push_declined".to_string(),
        Some(serde_json::json!({
            "node_id": &node.id,
            "pending_credential_id": &pending.id,
            "service_slug": &pending.service_slug,
            "owner_user_id": &pending.owner_user_id,
            "reason_present": body
                .as_ref()
                .and_then(|body| body.reason.as_deref())
                .is_some_and(|reason| !reason.trim().is_empty()),
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
        None,
        None,
    );

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::{DeclinePendingCredentialRequest, pending_info};
    use crate::models::node_pending_credential::{
        CryptoBundle, InjectionMethod, NodePendingCredential,
    };
    use chrono::{Duration, Utc};

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
}
