//! One-time relay Verify Token reveal during native WhatsApp rollback.
use super::*;
use axum::{http::header, response::IntoResponse};
use zeroize::Zeroizing;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReplaceRequest {
    pub revision: i64,
}

#[derive(Serialize)]
struct ReplaceResponse {
    verify_token: Zeroizing<String>,
    relay_webhook_url: String,
    revision: i64,
    previous_token_invalidated_immediately: bool,
}

pub async fn replace(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<ReplaceRequest>,
) -> AppResult<axum::response::Response> {
    super::super::login_client_context::require_first_party_human(&auth)?;
    let (_, bot) = super::super::channel_bots::resolve_bot_owner_for_write(
        &state,
        &auth.user_id.to_string(),
        &id,
    )
    .await?;
    let issued = Box::pin(native::rollback::issue(
        &state.db,
        &bot,
        body.revision,
        &audit_service::AuditActor::from_auth_user(&auth),
    ))
    .await?;
    Ok((
        [(header::CACHE_CONTROL, "private, no-store")],
        Json(ReplaceResponse {
            verify_token: issued.verify_token,
            relay_webhook_url: crate::services::channel_bot_service::webhook_url(
                &state.config.base_url,
                &bot,
            ),
            revision: issued.revision,
            previous_token_invalidated_immediately: true,
        }),
    )
        .into_response())
}

#[cfg(test)]
#[path = "channel_whatsapp_native_rollback_tests.rs"]
mod tests;
