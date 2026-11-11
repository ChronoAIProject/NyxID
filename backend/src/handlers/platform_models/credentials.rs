//! First-party administration. Responses allowlist metadata, never secret material.
use super::super::{
    admin_helpers::{require_admin, require_admin_or_operator},
    login_client_context::require_first_party_human,
    services,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::platform_model_credentials as credentials,
    telemetry::TelemetryContext,
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use credentials::Kind;
use serde::{Deserialize, Serialize};

#[derive(Serialize)]
pub struct Credential {
    id: String,
    name: String,
    kind: Option<Kind>,
    enabled: bool,
    created_at: String,
    updated_at: String,
    references: Vec<Reference>,
}
#[derive(Serialize, Deserialize)]
struct Reference {
    model_id: String,
    public_id: String,
    display_name: String,
    route_id: Option<String>,
    enabled: bool,
}
fn view(row: credentials::Record, refs: &[bson::Document]) -> AppResult<Credential> {
    let references = refs
        .iter()
        .filter(|r| r.get_str("credential_id") == Ok(row.id.as_str()))
        .map(|r| {
            bson::from_document(r.clone())
                .map_err(|_| AppError::Internal("Invalid model reference".into()))
        })
        .collect::<AppResult<_>>()?;
    Ok(Credential {
        id: row.id,
        name: row.name,
        kind: Kind::from_url(&row.base_url),
        enabled: row.is_active && row.platform_key.enabled,
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
        references,
    })
}
#[derive(Deserialize, Default)]
pub struct ListQuery {
    pub after: Option<String>,
}
#[derive(Serialize)]
pub struct Inventory {
    records: Vec<Credential>,
    next_after: Option<String>,
}
async fn inventory(
    state: AppState,
    auth: AuthUser,
    query: ListQuery,
    offering: bool,
) -> AppResult<Json<Inventory>> {
    require_first_party_human(&auth)?;
    require_admin_or_operator(&state, &auth, "admin.platform_models.credentials.list").await?;
    let rows = credentials::list(&state.db, offering, query.after.as_deref()).await?;
    let next_after = (rows.len() == 100).then(|| rows.last().unwrap().id.clone());
    let refs = credentials::references(
        &state.db,
        &rows.iter().map(|r| r.id.clone()).collect::<Vec<_>>(),
        offering,
    )
    .await?;
    Ok(Json(Inventory {
        records: rows
            .into_iter()
            .map(|r| view(r, &refs))
            .collect::<AppResult<_>>()?,
        next_after,
    }))
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<Inventory>> {
    inventory(state, auth, query, false).await
}
pub async fn offerings(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<Inventory>> {
    inventory(state, auth, query, true).await
}
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Credential>> {
    require_first_party_human(&auth)?;
    require_admin_or_operator(&state, &auth, "admin.platform_models.credentials.get").await?;
    let row = credentials::get(&state.db, &id).await?;
    let refs = credentials::references(&state.db, &[id], false).await?;
    Ok(Json(view(row, &refs)?))
}
async fn write_access(state: &AppState, auth: &AuthUser) -> AppResult<()> {
    require_first_party_human(auth)?;
    require_admin(state, auth).await?;
    if !crate::services::feature_flag_service::personal_flag_enabled(
        &state.db,
        &auth.user_id.to_string(),
        super::registry::FLAG,
    )
    .await?
    {
        return Err(AppError::Forbidden(
            "Platform Models is read-only while the feature is disabled".into(),
        ));
    }
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateCredential {
    pub name: String,
    pub kind: Kind,
    pub credential: zeroize::Zeroizing<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateOffering {
    pub name: String,
}
async fn create_record(
    state: AppState,
    auth: AuthUser,
    tele: TelemetryContext,
    name: String,
    kind: Option<Kind>,
    credential: Option<zeroize::Zeroizing<String>>,
) -> AppResult<Json<Credential>> {
    // Internal category skips all spec discovery and provider calls. Endpoint is
    // non-secret display metadata only; execution uses the closed route target.
    let body = serde_json::json!({"name":name,"slug":format!("model-{}-{}",if kind.is_some() {"credential"} else {"offering"},uuid::Uuid::new_v4()),"base_url":kind.map_or("https://models.nyxid.invalid",Kind::base_url),"service_type":"http","service_category":"internal","visibility":"private","auth_method":if kind.is_some() {"bearer"} else {"none"},"platform_key":{"enabled":true,"audience":"restricted","allowed_owner_ids":[],"purpose":if kind.is_some() {"model_route"} else {"model_offering"}}});
    let mut body: services::CreateServiceRequest = serde_json::from_value(body)
        .map_err(|_| AppError::Internal("Invalid model catalog template".into()))?;
    body.credential = credential;
    let Json(created) =
        services::create_service(State(state.clone()), auth, tele, Json(body)).await?;
    Ok(Json(Credential {
        id: created.id,
        name: created.name,
        kind,
        enabled: true,
        created_at: created.created_at,
        updated_at: created.updated_at,
        references: vec![],
    }))
}
fn decode_secret<T: serde::de::DeserializeOwned>(
    body: Result<
        Json<crate::crypto::secret_json::SecretJson>,
        axum::extract::rejection::JsonRejection,
    >,
) -> AppResult<T> {
    let Json(body) =
        body.map_err(|_| AppError::ValidationError("Invalid credential request JSON".into()))?;
    T::deserialize(&body.0)
        .map_err(|_| AppError::ValidationError("Invalid credential request fields".into()))
}
/// Sanitize serde failures too: rejected field values may contain key material.
pub async fn create_request(
    state: State<AppState>,
    auth: AuthUser,
    tele: TelemetryContext,
    body: Result<
        Json<crate::crypto::secret_json::SecretJson>,
        axum::extract::rejection::JsonRejection,
    >,
) -> AppResult<Json<Credential>> {
    create(state, auth, tele, Json(decode_secret(body)?)).await
}
pub async fn update_request(
    state: State<AppState>,
    auth: AuthUser,
    tele: TelemetryContext,
    id: Path<String>,
    body: Result<
        Json<crate::crypto::secret_json::SecretJson>,
        axum::extract::rejection::JsonRejection,
    >,
) -> AppResult<Json<Credential>> {
    update(state, auth, tele, id, Json(decode_secret(body)?)).await
}
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<CreateCredential>,
) -> AppResult<Json<Credential>> {
    write_access(&state, &auth).await?;
    body.kind.validate(&body.credential)?;
    create_record(
        state,
        auth,
        tele,
        body.name,
        Some(body.kind),
        Some(body.credential),
    )
    .await
}
pub async fn create_offering(
    State(state): State<AppState>,
    auth: AuthUser,
    tele: TelemetryContext,
    Json(body): Json<CreateOffering>,
) -> AppResult<Json<Credential>> {
    write_access(&state, &auth).await?;
    create_record(state, auth, tele, body.name, None, None).await
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateCredential {
    pub expected_updated_at: chrono::DateTime<chrono::Utc>,
    pub credential: Option<zeroize::Zeroizing<String>>,
    pub enabled: Option<bool>,
}
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    tele: TelemetryContext,
    Path(id): Path<String>,
    Json(body): Json<UpdateCredential>,
) -> AppResult<Json<Credential>> {
    write_access(&state, &auth).await?;
    let row = credentials::get(&state.db, &id).await?;
    if body.enabled == Some(true) {
        return Err(AppError::ValidationError(
            "Use the catalog to re-enable a disabled credential after review".into(),
        ));
    }
    if let Some(raw) = &body.credential {
        if raw.len() > 64 * 1024 {
            return Err(AppError::ValidationError(
                "Credential exceeds 64 KiB".into(),
            ));
        }
        match Kind::from_url(&row.base_url) {
            Some(kind) => kind.validate(raw)?,
            None => crate::services::platform_model_execution::token::validate_saved_credential(
                raw,
                raw.trim_start().starts_with('{'),
            )?,
        }
    }
    let refs = credentials::references(&state.db, std::slice::from_ref(&id), false).await?;
    if body.enabled == Some(false) && refs.iter().any(|r| r.get_bool("enabled") == Ok(true)) {
        return Err(AppError::Conflict(
            "Disable the enabled routes that reference this credential first".into(),
        ));
    }
    let mut patch: services::UpdateServiceRequest =
        serde_json::from_value(serde_json::json!({}))
            .map_err(|_| AppError::Internal("Invalid catalog template".into()))?;
    patch.credential = body.credential;
    if body.enabled == Some(false) {
        let mut config = row.platform_key;
        config.enabled = false;
        patch.platform_key = Some(config);
    }
    let _updated = services::update_service_at_revision(
        state.clone(),
        auth.clone(),
        tele,
        id.clone(),
        patch,
        Some(body.expected_updated_at),
    )
    .await?;
    get(State(state), auth, Path(id)).await
}

#[cfg(test)]
mod tests;
