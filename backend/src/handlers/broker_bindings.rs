use axum::{
    Json,
    extract::{Path, State},
};
use futures::TryStreamExt;
use mongodb::bson::{self, Bson, doc};
use serde::Serialize;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::authorization_code::ExternalSubjectRef;
use crate::models::oauth_broker_binding::{
    COLLECTION_NAME as OAUTH_BROKER_BINDINGS, OauthBrokerBinding,
};
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, oauth_broker_service};

#[derive(Serialize)]
pub struct BrokerBindingListItem {
    pub binding_hash: String,
    pub client_id: String,
    pub client_name: Option<String>,
    pub external_subject: Option<ExternalSubjectRef>,
    pub scopes: Vec<String>,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Serialize)]
pub struct BrokerBindingListResponse {
    pub bindings: Vec<BrokerBindingListItem>,
}

/// GET /api/v1/users/me/broker-bindings
pub async fn list_my_broker_bindings(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<BrokerBindingListResponse>> {
    let user_id = auth_user.user_id.to_string();
    let raw = oauth_broker_service::list_user_bindings(&state.db, &user_id).await?;

    // Resolve client_name for each unique client_id in one batch.
    let unique_client_ids: std::collections::HashSet<String> = raw
        .iter()
        .map(|binding| binding.client_id.clone())
        .collect();
    let mut name_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if !unique_client_ids.is_empty() {
        let ids: Vec<Bson> = unique_client_ids
            .iter()
            .map(|id| Bson::String(id.clone()))
            .collect();
        let cursor = state
            .db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find(bson::doc! { "_id": { "$in": ids } })
            .await?;
        let clients: Vec<OauthClient> = cursor.try_collect().await?;
        for client in clients {
            name_map.insert(client.id.clone(), client.client_name);
        }
    }

    let bindings = raw
        .into_iter()
        .map(|binding| BrokerBindingListItem {
            binding_hash: binding.binding_hash.clone(),
            client_name: name_map.get(&binding.client_id).cloned(),
            client_id: binding.client_id,
            external_subject: binding.external_subject,
            scopes: binding.scopes,
            created_at: binding.created_at.to_rfc3339(),
            last_used_at: binding.last_used_at.map(|time| time.to_rfc3339()),
        })
        .collect();

    Ok(Json(BrokerBindingListResponse { bindings }))
}

/// DELETE /api/v1/users/me/broker-bindings/{binding_hash}
pub async fn revoke_my_broker_binding(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(binding_hash): Path<String>,
) -> AppResult<axum::http::StatusCode> {
    let user_id = auth_user.user_id.to_string();
    let binding = state
        .db
        .collection::<OauthBrokerBinding>(OAUTH_BROKER_BINDINGS)
        .find_one(doc! {
            "_id": &binding_hash,
            "user_id": &user_id,
            "revoked": false,
        })
        .await?
        .ok_or_else(|| AppError::NotFound("binding not found".to_string()))?;

    oauth_broker_service::revoke_binding_by_user(
        &state.db,
        state.encryption_keys.clone(),
        &state.http_client,
        &user_id,
        &binding_hash,
        "user_revoked",
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "oauth_broker_binding_revoked",
        Some(serde_json::json!({
            "revoke_source": "user",
            "client_id": binding.client_id,
            "binding_hash": oauth_broker_service::binding_hash_prefix(&binding_hash),
            "reason": "user_revoked",
        })),
    );

    Ok(axum::http::StatusCode::NO_CONTENT)
}
