use axum::{
    Json,
    extract::{Path, State},
};
use mongodb::bson::doc;
use serde::Serialize;

use crate::AppState;
use crate::errors::AppResult;
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::mw::auth::AuthUser;
use crate::services::consent_service;

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct ConsentItem {
    pub id: String,
    pub client_id: String,
    pub client_name: String,
    pub scopes: String,
    pub granted_at: String,
    pub expires_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct ConsentListResponse {
    pub consents: Vec<ConsentItem>,
}

#[derive(Debug, Serialize)]
pub struct ConsentRevokeResponse {
    pub message: String,
}

// --- Handlers ---

/// GET /api/v1/users/me/consents
pub async fn list_my_consents(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<ConsentListResponse>> {
    let user_id = auth_user.user_id.to_string();
    let consents = consent_service::list_user_consents(&state.db, &user_id).await?;

    let mut items = Vec::with_capacity(consents.len());
    for c in consents {
        // Look up client name
        let client_name = state
            .db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find_one(doc! { "_id": &c.client_id })
            .await?
            .map(|cl| cl.client_name)
            .unwrap_or_else(|| c.client_id.clone());

        items.push(ConsentItem {
            id: c.id,
            client_id: c.client_id,
            client_name,
            scopes: c.scopes,
            granted_at: c.granted_at.to_rfc3339(),
            expires_at: c.expires_at.map(|t| t.to_rfc3339()),
        });
    }

    Ok(Json(ConsentListResponse { consents: items }))
}

/// DELETE /api/v1/users/me/consents/:client_id
pub async fn revoke_my_consent(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(client_id): Path<String>,
) -> AppResult<Json<ConsentRevokeResponse>> {
    let user_id = auth_user.user_id.to_string();
    consent_service::revoke_consent(&state.db, &user_id, &client_id).await?;

    Ok(Json(ConsentRevokeResponse {
        message: "Consent revoked".to_string(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- ConsentItem serialization ----

    #[test]
    fn consent_item_serializes_all_fields() {
        let item = ConsentItem {
            id: "consent-1".to_string(),
            client_id: "client-abc".to_string(),
            client_name: "My App".to_string(),
            scopes: "openid profile email".to_string(),
            granted_at: "2026-01-01T00:00:00+00:00".to_string(),
            expires_at: Some("2027-01-01T00:00:00+00:00".to_string()),
        };
        let json = serde_json::to_value(&item).unwrap();
        assert_eq!(json["id"], "consent-1");
        assert_eq!(json["client_id"], "client-abc");
        assert_eq!(json["client_name"], "My App");
        assert_eq!(json["scopes"], "openid profile email");
        assert_eq!(json["granted_at"], "2026-01-01T00:00:00+00:00");
        assert_eq!(json["expires_at"], "2027-01-01T00:00:00+00:00");
    }

    #[test]
    fn consent_item_with_no_expiry() {
        let item = ConsentItem {
            id: "consent-2".to_string(),
            client_id: "client-xyz".to_string(),
            client_name: "Other App".to_string(),
            scopes: "openid".to_string(),
            granted_at: "2026-01-01T00:00:00+00:00".to_string(),
            expires_at: None,
        };
        let json = serde_json::to_value(&item).unwrap();
        assert!(json["expires_at"].is_null());
    }

    // ---- ConsentListResponse serialization ----

    #[test]
    fn consent_list_response_empty() {
        let resp = ConsentListResponse { consents: vec![] };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["consents"], serde_json::json!([]));
    }

    // ---- ConsentRevokeResponse serialization ----

    #[test]
    fn consent_revoke_response_message() {
        let resp = ConsentRevokeResponse {
            message: "Consent revoked".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["message"], "Consent revoked");
    }
}
