use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::header,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::models::oauth_client::OauthClient;
use crate::services::{audit_service, oauth_branding_service as branding};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
};

#[derive(Debug, Default, Serialize)]
pub struct BrandingResponse {
    pub logo_asset_id: Option<String>,
    pub logo_url: Option<String>,
    pub homepage_url: Option<String>,
    pub branding_revision: u32,
    pub branding_verified_revision: Option<u32>,
    pub verified: bool,
}

impl From<&OauthClient> for BrandingResponse {
    fn from(client: &OauthClient) -> Self {
        Self {
            logo_asset_id: client.logo_asset_id.clone(),
            logo_url: branding::logo_url(client),
            homepage_url: client.homepage_url.clone(),
            branding_revision: client.branding_revision,
            branding_verified_revision: client.branding_verified_revision,
            verified: branding::is_verified(client),
        }
    }
}

pub async fn upload_logo(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    mut multipart: Multipart,
) -> AppResult<Json<BrandingResponse>> {
    let owner = super::developer_apps::resolve_developer_app_write_owner(
        &state,
        &auth.user_id.to_string(),
        &id,
    )
    .await?;
    crate::services::app_connect_link_service::enabled_client(&state, &id).await?;
    let mut file = None;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| AppError::ValidationError("Invalid logo upload".into()))?
    {
        if field.name() != Some("logo") || file.is_some() {
            return Err(AppError::ValidationError(
                "Upload exactly one logo field".into(),
            ));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = field
            .chunk()
            .await
            .map_err(|_| AppError::ValidationError("Invalid logo upload".into()))?
        {
            if bytes.len() + chunk.len() > branding::MAX_LOGO_INPUT {
                return Err(AppError::RequestBodyTooLarge {
                    max_bytes: branding::MAX_LOGO_INPUT,
                    context: "Logo".into(),
                });
            }
            bytes.extend_from_slice(&chunk);
        }
        file = Some(bytes);
    }
    let client = branding::upload_logo(
        &state.db,
        &id,
        &owner,
        file.ok_or_else(|| AppError::ValidationError("A logo is required".into()))?,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "app_branding_updated",
        Some(serde_json::json!({ "client_id": id, "branding_revision": client.branding_revision })),
    );
    Ok(Json(BrandingResponse::from(&client)))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerifyRequest {
    pub branding_revision: u32,
    pub verified: bool,
}

pub async fn verify(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<VerifyRequest>,
) -> AppResult<Json<BrandingResponse>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    let client = branding::verify(&state.db, &id, body.branding_revision, body.verified).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "app_branding_verification_changed",
        Some(
            serde_json::json!({ "client_id": id, "branding_revision": body.branding_revision, "verified": body.verified }),
        ),
    );
    Ok(Json(BrandingResponse::from(&client)))
}

pub async fn asset(State(state): State<AppState>, Path(id): Path<String>) -> AppResult<Response> {
    let bytes = branding::read_logo(&state.db, &id).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "image/png"),
            (header::CACHE_CONTROL, "public, max-age=31536000, immutable"),
            (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            (
                header::CONTENT_SECURITY_POLICY,
                "default-src 'none'; sandbox",
            ),
        ],
        bytes,
    )
        .into_response())
}
