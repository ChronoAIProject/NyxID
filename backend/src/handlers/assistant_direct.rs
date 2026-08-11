//! Flag-gated direct Chrono-LLM assistant endpoints.

use axum::{
    Json,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderValue, Request, header},
    response::Response,
};
use serde::Serialize;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::proxy::execute_admin_proxy;
use crate::mw::auth::AuthUser;
use crate::services::{assistant_direct, assistant_service, feature_flag_service};

#[derive(Serialize)]
pub struct DirectSkillResponse {
    slug: &'static str,
    label: &'static str,
}

#[derive(Serialize)]
pub struct DirectModelResponse {
    id: &'static str,
    label: &'static str,
    default: bool,
}

async fn require_direct_chat_enabled(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    let enabled =
        feature_flag_service::resolve_personal_features(&state.db, &auth_user.user_id.to_string())
            .await?
            .into_iter()
            .any(|key| key == feature_flag_service::DIRECT_CHAT_ENGINE_FLAG_KEY);
    if !enabled {
        return Err(AppError::NotFound("Assistant route not found.".to_string()));
    }
    Ok(())
}

pub async fn skills(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<Vec<DirectSkillResponse>>> {
    require_direct_chat_enabled(&state, &auth_user).await?;
    Ok(Json(
        assistant_direct::DIRECT_SKILLS
            .iter()
            .map(|skill| DirectSkillResponse {
                slug: skill.slug,
                label: skill.label,
            })
            .collect(),
    ))
}

pub async fn models(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<Vec<DirectModelResponse>>> {
    require_direct_chat_enabled(&state, &auth_user).await?;
    Ok(Json(
        assistant_direct::DIRECT_MODELS
            .iter()
            .map(|model| DirectModelResponse {
                id: model.id,
                label: model.label,
                default: model.default,
            })
            .collect(),
    ))
}

pub async fn completions(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
) -> AppResult<Response> {
    require_direct_chat_enabled(&state, &auth_user).await?;

    let (mut parts, body) = request.into_parts();
    let bytes = to_bytes(body, assistant_direct::MAX_DIRECT_REQUEST_BYTES)
        .await
        .map_err(|_| AppError::BadRequest("Direct chat request body is too large.".to_string()))?;
    let direct_request = assistant_direct::validate_direct_request(&bytes)?;
    let message_count = direct_request.messages.len();
    let content_bytes: usize = direct_request
        .messages
        .iter()
        .map(|message| message.content.len())
        .sum();
    let model = direct_request
        .model
        .clone()
        .unwrap_or_else(|| assistant_direct::DEFAULT_DIRECT_MODEL.to_string());
    let skill_slug = direct_request.skill_slug.clone();
    let upstream_body = assistant_direct::build_upstream_body(direct_request);
    let payload = serde_json::to_vec(&upstream_body).map_err(|_| {
        AppError::Internal("assistant: failed to encode direct chat request".to_string())
    })?;

    parts.headers.remove(header::CONTENT_LENGTH);
    parts.headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    parts.headers.insert(
        header::ACCEPT,
        HeaderValue::from_static("text/event-stream"),
    );
    let request = Request::from_parts(parts, Body::from(payload));
    let service = assistant_service::resolve_admin_service_by_slug(
        &state.db,
        assistant_direct::DIRECT_LLM_SLUG,
    )
    .await?;

    tracing::debug!(
        user_id = %auth_user.user_id,
        model = %model,
        skill_slug = skill_slug.as_deref().unwrap_or("none"),
        message_count,
        content_bytes,
        "assistant_direct_request"
    );

    let mut resolved_slug = String::new();
    let response = execute_admin_proxy(
        &state,
        &auth_user,
        &service.id,
        "chat/completions",
        request,
        Vec::new(),
        &mut resolved_slug,
    )
    .await?;

    tracing::debug!(
        user_id = %auth_user.user_id,
        model = %model,
        skill_slug = skill_slug.as_deref().unwrap_or("none"),
        message_count,
        content_bytes,
        status = response.status().as_u16(),
        "assistant_direct_response"
    );
    Ok(response)
}

#[cfg(test)]
mod tests {
    use super::*;

    const USER_ID: &str = "a026fd00-bd86-4284-9832-9e5e65fc8f50";

    #[tokio::test]
    async fn raw_body_overflow_maps_to_bad_request() {
        let result = to_bytes(
            Body::from(vec![b'x'; assistant_direct::MAX_DIRECT_REQUEST_BYTES + 1]),
            assistant_direct::MAX_DIRECT_REQUEST_BYTES,
        )
        .await
        .map_err(|_| AppError::BadRequest("Direct chat request body is too large.".to_string()));
        assert!(matches!(result, Err(AppError::BadRequest(_))));
    }

    #[tokio::test]
    async fn default_off_returns_not_found_for_all_direct_handlers() {
        let Some(db) = crate::test_utils::connect_test_database("assistant_direct_flag_off").await
        else {
            eprintln!("skipping direct assistant flag test: no local MongoDB available");
            return;
        };
        let state = crate::test_utils::test_app_state(db);
        let auth_user = crate::test_utils::test_auth_user(USER_ID);

        assert!(matches!(
            skills(State(state.clone()), auth_user.clone()).await,
            Err(AppError::NotFound(_))
        ));
        assert!(matches!(
            models(State(state.clone()), auth_user.clone()).await,
            Err(AppError::NotFound(_))
        ));
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/assistant/direct/completions")
            .body(Body::from(
                r#"{"messages":[{"role":"user","content":"hello"}]}"#,
            ))
            .unwrap();
        assert!(matches!(
            completions(State(state), auth_user, request).await,
            Err(AppError::NotFound(_))
        ));
    }

    #[tokio::test]
    async fn enabled_flag_serves_curated_skill_and_model_tables() {
        use crate::services::feature_flag_service::{FlagTarget, set_platform_override};

        let Some(db) = crate::test_utils::connect_test_database("assistant_direct_flag_on").await
        else {
            eprintln!("skipping direct assistant flag test: no local MongoDB available");
            return;
        };
        set_platform_override(
            &db,
            feature_flag_service::DIRECT_CHAT_ENGINE_FLAG_KEY,
            &FlagTarget::Global,
            true,
            "admin",
        )
        .await
        .unwrap();
        let state = crate::test_utils::test_app_state(db);
        let auth_user = crate::test_utils::test_auth_user(USER_ID);

        let skill_rows = skills(State(state.clone()), auth_user.clone())
            .await
            .unwrap()
            .0;
        let model_rows = models(State(state), auth_user).await.unwrap().0;
        assert_eq!(skill_rows.len(), assistant_direct::DIRECT_SKILLS.len());
        assert_eq!(model_rows.len(), assistant_direct::DIRECT_MODELS.len());
        assert_eq!(
            skill_rows.iter().map(|row| row.slug).collect::<Vec<_>>(),
            assistant_direct::DIRECT_SKILLS
                .iter()
                .map(|row| row.slug)
                .collect::<Vec<_>>()
        );
        assert_eq!(
            model_rows
                .iter()
                .filter(|row| row.default)
                .map(|row| (row.id, row.label))
                .collect::<Vec<_>>(),
            vec![(assistant_direct::DEFAULT_DIRECT_MODEL, "GPT-5.5")]
        );
    }
}
