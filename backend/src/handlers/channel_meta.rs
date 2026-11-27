use crate::{
    AppState,
    errors::AppResult,
    mw::auth::AuthUser,
    services::channel_meta_service::{self, MetaService},
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    pub label: String,
    pub target_org_id: Option<String>,
    pub reconnect_bot_id: Option<String>,
    pub setup_token: Option<zeroize::Zeroizing<String>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exchange {
    pub state: zeroize::Zeroizing<String>,
    pub code: Option<zeroize::Zeroizing<String>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finish {
    pub asset_id: String,
    pub exclusive_routing_verified: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    pub enabled: bool,
}

#[derive(Serialize)]
pub struct PlatformDeliveryResponse {
    pub platform: String,
    pub enabled: bool,
    pub resume_requires_bot_verification: bool,
}

#[derive(Serialize)]
pub struct BotDeliveryResponse {
    pub id: String,
    pub enabled: bool,
}

#[derive(Serialize, Deserialize)]
pub struct ProviderRequestResponse {
    pub confirmation_code: String,
    pub url: String,
}

#[derive(Serialize, Deserialize)]
pub struct DeletionStatusResponse {
    pub status: String,
}

fn provider_response<T: serde::de::DeserializeOwned>(value: serde_json::Value) -> AppResult<T> {
    serde_json::from_value(value).map_err(|_| {
        crate::errors::AppError::Internal("Unable to report provider cleanup status".into())
    })
}

pub async fn platform_delivery(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(platform): Path<String>,
    Json(input): Json<Delivery>,
) -> AppResult<Json<PlatformDeliveryResponse>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    crate::services::channel_meta_management_service::platform_delivery(
        &state.db,
        &platform,
        input.enabled,
    )
    .await?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_platform_delivery_changed",
        Some(serde_json::json!({"platform":platform,"enabled":input.enabled})),
    );
    Ok(Json(PlatformDeliveryResponse {
        platform,
        enabled: input.enabled,
        resume_requires_bot_verification: true,
    }))
}
pub async fn bot_delivery(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<Delivery>,
) -> AppResult<Json<BotDeliveryResponse>> {
    crate::services::channel_meta_management_service::bot_delivery(
        &state,
        &auth.user_id.to_string(),
        &id,
        input.enabled,
    )
    .await?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_bot_delivery_changed",
        Some(serde_json::json!({"bot_id":id,"enabled":input.enabled})),
    );
    Ok(Json(BotDeliveryResponse {
        id,
        enabled: input.enabled,
    }))
}

#[derive(Deserialize)]
pub struct SignedRequest {
    pub signed_request: zeroize::Zeroizing<String>,
}
pub async fn deauthorize(
    State(state): State<AppState>,
    Path(platform): Path<String>,
    axum::Form(input): axum::Form<SignedRequest>,
) -> AppResult<Json<ProviderRequestResponse>> {
    Ok(Json(provider_response(
        channel_meta_service::accept_provider_request(
            &MetaService::from(&state),
            &platform,
            &input.signed_request,
            false,
        )
        .await?,
    )?))
}
pub async fn data_deletion(
    State(state): State<AppState>,
    Path(platform): Path<String>,
    axum::Form(input): axum::Form<SignedRequest>,
) -> AppResult<Json<ProviderRequestResponse>> {
    Ok(Json(provider_response(
        channel_meta_service::accept_provider_request(
            &MetaService::from(&state),
            &platform,
            &input.signed_request,
            true,
        )
        .await?,
    )?))
}
pub async fn deletion_status(
    State(state): State<AppState>,
    Path((platform, id)): Path<(String, String)>,
) -> AppResult<Json<DeletionStatusResponse>> {
    Ok(Json(provider_response(
        channel_meta_service::request_status(&state.db, &platform, &id).await?,
    )?))
}

pub use crate::services::channel_meta_management_service::CustomerAction;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomerSetup {
    pub agent_id: String,
    pub action: CustomerAction,
}

#[derive(Serialize)]
pub struct CustomerSetupResponse {
    pub channel_id: String,
    pub agent_id: Option<String>,
    pub guests_enabled: bool,
    pub readiness: serde_json::Value,
    pub owner_link_code: Option<String>,
    pub owner_link_expires_at: Option<String>,
}

pub async fn customer_setup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<CustomerSetup>,
) -> AppResult<Json<CustomerSetupResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let result = crate::services::channel_meta_management_service::customer_setup(
        &state,
        &auth.user_id.to_string(),
        &id,
        &input.agent_id,
        input.action,
    )
    .await?;
    Ok(Json(CustomerSetupResponse {
        channel_id: result.channel_id,
        agent_id: result.agent_id,
        guests_enabled: result.guests_enabled,
        readiness: result.readiness,
        owner_link_code: result.owner_link_code.map(|code| code.to_string()),
        owner_link_expires_at: result.owner_link_expires_at,
    }))
}

pub async fn start(
    State(s): State<AppState>,
    auth: AuthUser,
    Path(platform): Path<String>,
    Json(input): Json<Start>,
) -> AppResult<Json<channel_meta_service::Started>> {
    super::login_client_context::require_first_party_human(&auth)?;
    crate::services::channel_meta_management_service::oauth_limit(
        &s.db,
        &auth.user_id.to_string(),
        &platform,
    )
    .await?;
    let actor = auth.user_id.to_string();
    Ok(Json(
        crate::services::channel_meta_management_service::start(
            &s,
            &actor,
            &platform,
            &input.label,
            input.target_org_id.as_deref(),
            input.reconnect_bot_id.as_deref(),
            input.setup_token.as_deref().map(|s| s.as_str()),
        )
        .await?,
    ))
}

pub async fn exchange(
    State(s): State<AppState>,
    auth: AuthUser,
    Path(platform): Path<String>,
    Json(input): Json<Exchange>,
) -> AppResult<Json<channel_meta_service::Progress>> {
    super::login_client_context::require_first_party_human(&auth)?;
    crate::services::channel_meta_management_service::oauth_limit(
        &s.db,
        &auth.user_id.to_string(),
        &platform,
    )
    .await?;
    Ok(Json(
        MetaService::from(&s)
            .exchange(
                &auth.user_id.to_string(),
                &platform,
                &input.state,
                input.code.as_ref().map(|s| s.as_str()),
            )
            .await?,
    ))
}
pub async fn progress(
    State(s): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<channel_meta_service::Progress>> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(
        MetaService::from(&s)
            .progress(&auth.user_id.to_string(), &id)
            .await?,
    ))
}
#[derive(Serialize)]
pub struct Connected {
    pub id: String,
    pub platform: String,
    pub label: String,
    pub platform_bot_username: String,
    pub status: String,
    pub credential_source: String,
    pub webhook_ingestion: bool,
    pub webhook_url: String,
}
pub async fn finish(
    State(s): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<Finish>,
) -> AppResult<Json<Connected>> {
    super::login_client_context::require_first_party_human(&auth)?;
    crate::services::channel_meta_management_service::oauth_limit(
        &s.db,
        &auth.user_id.to_string(),
        "finish",
    )
    .await?;
    let bot = MetaService::from(&s)
        .finish(
            &auth.user_id.to_string(),
            &id,
            &input.asset_id,
            input.exclusive_routing_verified,
        )
        .await?;
    Ok(Json(Connected {
        webhook_url: crate::services::channel_bot_service::webhook_url(&s.config.base_url, &bot),
        id: bot.id,
        platform: bot.platform,
        label: bot.label,
        platform_bot_username: bot.platform_bot_username,
        status: bot.status,
        credential_source: bot.credential_source,
        webhook_ingestion: true,
    }))
}
