use axum::{
    extract::{Path, State},
    Json,
};
use mongodb::bson::doc;
use serde::Serialize;

use crate::errors::AppResult;
use crate::models::oauth_client::{OauthClient, COLLECTION_NAME as OAUTH_CLIENTS};
use crate::mw::auth::AuthUser;
use crate::services::consent_service;
use crate::AppState;

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
