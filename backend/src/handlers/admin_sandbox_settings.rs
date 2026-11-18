//! Service-owned Sandbox producer settings; prices remain on the rate-card API.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    handlers::admin_helpers::{require_admin, require_admin_or_operator},
    mw::auth::AuthUser,
    services::{audit_service::AuditActor, billing::sandbox_settings},
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct SettingsResponse {
    service_id: String,
    authorized_producer_service_account_id: Option<String>,
    revision: i64,
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<SettingsResponse>> {
    require_admin_or_operator(&state, &auth, "admin.billing.sandbox_settings").await?;
    let service = sandbox_settings::resolve_service(&state.db).await?;
    if service.id != id {
        return Err(AppError::NotFound("CMA catalog service not found".into()));
    }
    let settings = sandbox_settings::load(&state.db, &id).await?;
    Ok(Json(SettingsResponse {
        service_id: id,
        revision: settings.as_ref().map_or(0, |s| s.revision),
        authorized_producer_service_account_id: settings
            .and_then(|s| s.authorized_producer_service_account_id),
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRequest {
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    authorized_producer_service_account_id: Option<Option<String>>,
    expected_revision: i64,
}

pub async fn put(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<UpdateRequest>,
) -> AppResult<Json<SettingsResponse>> {
    require_admin(&state, &auth).await?;
    let producer = input
        .authorized_producer_service_account_id
        .ok_or_else(|| {
            AppError::ValidationError(
                "authorized_producer_service_account_id is required (null clears it)".into(),
            )
        })?;
    let saved = sandbox_settings::update(
        &state.db,
        &id,
        producer.as_deref(),
        input.expected_revision,
        &AuditActor::from_auth_user(&auth),
        state.audit_chain_hmac_key.as_slice(),
    )
    .await?;
    Ok(Json(SettingsResponse {
        service_id: saved.service_id,
        authorized_producer_service_account_id: saved.authorized_producer_service_account_id,
        revision: saved.revision,
    }))
}

#[cfg(test)]
mod tests {
    use super::super::curation_tests::{fixture, request};
    use axum::http::StatusCode;
    use mongodb::bson::{Document, doc};
    use serde_json::json;

    #[tokio::test]
    async fn sandbox_settings_routes_require_admin_writes_and_preserve_revision() {
        let f = fixture("sandbox_settings_routes", false).await;
        f.state
            .db
            .collection::<Document>("downstream_services")
            .update_one(doc! {"_id":&f.service.id}, doc! {"$set":{"slug":"cma"}})
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>("service_accounts")
            .update_one(
                doc! {"_id":&f.sa.id},
                doc! {"$set":{"allowed_scopes":"billing:sandbox:write"}},
            )
            .await
            .unwrap();
        let path = format!("/api/v1/admin/billing/services/{}/sandbox", f.service.id);
        let (status, initial) = request(&f.state, "GET", &path, &f.human_token, None).await;
        assert_eq!(status, StatusCode::OK, "{initial}");
        assert_eq!(initial["revision"], 0);
        assert!(initial["authorized_producer_service_account_id"].is_null());
        let update =
            json!({"authorized_producer_service_account_id": f.sa.id, "expected_revision":0});
        let (status, saved) =
            request(&f.state, "PUT", &path, &f.human_token, Some(update.clone())).await;
        assert_eq!(status, StatusCode::OK, "{saved}");
        assert_eq!(saved["revision"], 1);
        assert_eq!(saved["service_id"], f.service.id);
        assert_eq!(
            request(&f.state, "PUT", &path, &f.human_token, Some(update))
                .await
                .0,
            StatusCode::CONFLICT
        );
        assert_eq!(
            request(
                &f.state,
                "PUT",
                &path,
                &f.human_token,
                Some(json!({"expected_revision":1}))
            )
            .await
            .0,
            StatusCode::BAD_REQUEST
        );
        let roles = crate::services::role_service::get_platform_role_ids(&f.state.db)
            .await
            .unwrap();
        f.state
            .db
            .collection::<Document>("users")
            .update_one(
                doc! {"_id":&f.owner},
                doc! {"$set":{"is_admin":false,"role_ids":[roles.operator]}},
            )
            .await
            .unwrap();
        assert_eq!(
            request(&f.state, "GET", &path, &f.human_token, None)
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            request(
                &f.state,
                "PUT",
                &path,
                &f.human_token,
                Some(json!({"authorized_producer_service_account_id":null,"expected_revision":1}))
            )
            .await
            .0,
            StatusCode::FORBIDDEN
        );
        f.state
            .db
            .collection::<Document>("users")
            .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"role_ids":[]}})
            .await
            .unwrap();
        assert_eq!(
            request(&f.state, "GET", &path, &f.human_token, None)
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        let stored = crate::services::billing::sandbox_settings::load(&f.state.db, &f.service.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.authorized_producer_service_account_id.as_deref(),
            Some(f.sa.id.as_str())
        );
        assert_eq!(stored.revision, 1);
    }
}
