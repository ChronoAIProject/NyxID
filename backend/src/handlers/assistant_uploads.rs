//! First-party owner upload ingress; separate, strictly thread-key image egress.
use super::login_client_context::require_first_party_human;
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{
        assistant_nyxagent as engine, assistant_team_service as team,
        assistant_upload_service as uploads,
    },
};
use axum::{
    Json,
    body::Body,
    extract::{Path, State},
    http::{Request, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};
use std::sync::LazyLock;

static UPLOADS: LazyLock<tokio::sync::Semaphore> = LazyLock::new(|| tokio::sync::Semaphore::new(4));

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Draft {
    pub agent_id: Option<String>,
}

pub async fn draft(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<Draft>,
) -> AppResult<Json<Value>> {
    require_first_party_human(&auth)?;
    let user = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user).await?;
    let agent = match body.agent_id {
        Some(id) => team::agent(&state.db, &user, &id).await?,
        None => team::ensure_nyxbot(&state.db, &user).await?,
    };
    if agent.destroyed_at.is_some() {
        return Err(AppError::Forbidden("Agent was destroyed".into()));
    }
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().await?;
    let row = team::create_thread(
        &state.db,
        &state.encryption_keys,
        &agent,
        "New conversation",
        &mut session,
    )
    .await?;
    session.commit_transaction().await?;
    Ok(Json(json!({"id":row.id})))
}

pub async fn upload(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(scope): Path<String>,
    request: Request<Body>,
) -> AppResult<(
    StatusCode,
    Json<super::assistant_nyxagent::AttachmentResponse>,
)> {
    require_first_party_human(&auth)?;
    let user = auth.user_id.to_string();
    engine::require_enabled(&state.db, &user).await?;
    uploads::owner_scope(&state.db, &user, &scope).await?;
    uploads::admit(&state.db, &user).await?;
    let _permit = UPLOADS.try_acquire().map_err(|_| {
        AppError::BadRequest("Upload processing is busy. Try again shortly.".into())
    })?;
    let encoded = request
        .headers()
        .get("x-attachment-name")
        .and_then(|v| v.to_str().ok())
        .filter(|v| v.len() <= 1024)
        .unwrap_or("attachment");
    let name = urlencoding::decode(encoded)
        .map_err(|_| AppError::BadRequest("Invalid display filename".into()))?
        .into_owned();
    let bytes = axum::body::to_bytes(
        request.into_body(),
        crate::services::attachment_extraction::MAX_BYTES,
    )
    .await
    .map_err(|_| AppError::BadRequest("Attachment exceeds 20 MiB.".into()))?;
    let item = uploads::upload(
        &state.db,
        &state.encryption_keys,
        &user,
        &scope,
        &name,
        bytes.to_vec(),
    )
    .await?;
    Ok((StatusCode::CREATED, Json(item.into())))
}

pub async fn remove(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((scope, id)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    require_first_party_human(&auth)?;
    uploads::pending_delete(&state.db, &auth.user_id.to_string(), &scope, &id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn group_content(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((scope, id)): Path<(String, String)>,
) -> AppResult<Response> {
    require_first_party_human(&auth)?;
    let (mime, bytes) = uploads::owner_read(
        &state.db,
        &state.encryption_keys,
        &auth.user_id.to_string(),
        &scope,
        &id,
    )
    .await?;
    content(mime, bytes)
}

pub async fn thread_image(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Response> {
    if !matches!(auth.auth_method, crate::mw::auth::AuthMethod::ApiKey) {
        return Err(AppError::Forbidden("A thread Agent Key is required".into()));
    }
    let chat = crate::services::assistant_acknowledgement_service::for_key(
        &state.db,
        &auth.user_id.to_string(),
        auth.api_key_id.as_deref(),
    )
    .await?
    .ok_or_else(|| AppError::Forbidden("A thread Agent Key is required".into()))?;
    let (mime, bytes) =
        uploads::chat_bytes(&state.db, &state.encryption_keys, &chat, &id, true).await?;
    content(mime, bytes)
}

fn content(mime: String, bytes: Vec<u8>) -> AppResult<Response> {
    let mut response = bytes.into_response();
    for (key, value) in [
        ("content-type", mime.as_str()),
        ("cache-control", "private, no-store"),
        ("x-content-type-options", "nosniff"),
        ("content-disposition", "attachment"),
        ("content-security-policy", "default-src 'none'; sandbox"),
    ] {
        response.headers_mut().insert(
            key,
            value
                .parse()
                .map_err(|_| AppError::Internal("Attachment header".into()))?,
        );
    }
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn assistant_upload_routes_refuse_non_human_credentials_before_reading_body() {
        let state = crate::test_utils::test_app_state_no_db().await;
        let owner = "12345678-1234-4123-8123-123456789012";
        for kind in ["api_key", "oauth", "delegated"] {
            let mut auth = crate::test_utils::test_auth_user(owner);
            match kind {
                "api_key" => auth.api_key_id = Some("key".into()),
                "oauth" => auth.oauth_client_id = Some("client".into()),
                _ => auth.acting_client_id = Some("client".into()),
            }
            let result = upload(
                State(state.clone()),
                auth,
                Path("nyxa-00000000000000000000000000000000".into()),
                Request::new(Body::empty()),
            )
            .await;
            assert!(matches!(result, Err(AppError::Forbidden(_))));
        }
    }
}
