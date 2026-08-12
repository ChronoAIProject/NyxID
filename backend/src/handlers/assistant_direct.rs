//! Flag-gated direct Chrono-LLM assistant endpoints.

use axum::{
    Json,
    body::{Body, to_bytes},
    extract::State,
    http::{HeaderValue, Request, header},
    response::Response,
};
use futures::StreamExt;
use serde::Serialize;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::proxy::execute_admin_proxy;
use crate::mw::auth::AuthUser;
use crate::mw::rate_limit::DirectChatPermit;
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
    let permit = state
        .direct_chat_limiter
        .try_acquire(&auth_user.user_id.to_string())?;

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
    // The direct route owns the complete upstream contract. Caller query
    // parameters from the NyxID endpoint must not cross this platform-
    // credentialed boundary; only the fixed `chat/completions` path below is
    // forwarded. Retained Aevatar handlers continue to preserve their query
    // strings through the shared proxy path.
    parts.uri = parts.uri.path().parse().map_err(|_| {
        AppError::Internal("assistant: failed to rebuild direct chat URI".to_string())
    })?;
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
    Ok(attach_in_flight_permit(response, permit))
}

fn attach_in_flight_permit(response: Response, permit: DirectChatPermit) -> Response {
    let (parts, body) = response.into_parts();
    let stream = async_stream::stream! {
        let _permit = permit;
        let mut body = body.into_data_stream();
        while let Some(chunk) = body.next().await {
            yield chunk;
        }
    };
    Response::from_parts(parts, Body::from_stream(stream))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::post};
    use bytes::Bytes;
    use std::convert::Infallible;
    use tokio::net::TcpListener;

    const USER_ID: &str = "a026fd00-bd86-4284-9832-9e5e65fc8f50";

    #[tokio::test]
    async fn raw_body_overflow_reaches_completions_handler() {
        use crate::services::feature_flag_service::{FlagTarget, set_platform_override};

        let Some(db) =
            crate::test_utils::connect_test_database("assistant_direct_body_overflow").await
        else {
            eprintln!("skipping direct assistant body overflow test: no local MongoDB available");
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
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/assistant/direct/completions")
            .body(Body::from(vec![b'x'; 300 * 1024]))
            .unwrap();

        assert!(matches!(
            completions(State(state), auth_user, request).await,
            Err(AppError::BadRequest(_))
        ));
    }

    #[tokio::test]
    async fn direct_completion_does_not_forward_caller_query_string() {
        use crate::models::downstream_service::{
            COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
        };
        use crate::models::user::{COLLECTION_NAME as USERS, UserType};
        use crate::services::billing::{BillingIngress, route_inventory::BillingRoutePolicy};
        use crate::services::feature_flag_service::{FlagTarget, set_platform_override};

        let Some(db) =
            crate::test_utils::connect_test_database("assistant_direct_query_strip").await
        else {
            eprintln!("skipping direct assistant query test: no local MongoDB available");
            return;
        };

        let (uri_tx, uri_rx) = tokio::sync::oneshot::channel();
        let uri_tx = std::sync::Arc::new(std::sync::Mutex::new(Some(uri_tx)));
        let sink = uri_tx.clone();
        let downstream = Router::new().route(
            "/{*path}",
            post(move |uri: axum::http::Uri| {
                let sink = sink.clone();
                async move {
                    if let Some(tx) = sink.lock().unwrap().take() {
                        let _ = tx.send(uri);
                    }
                    (
                        [(header::CONTENT_TYPE, "text/event-stream")],
                        "data: [DONE]\n\n",
                    )
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind direct assistant downstream");
        let addr = listener
            .local_addr()
            .expect("direct assistant downstream addr");
        let server = tokio::spawn(async move {
            axum::serve(listener, downstream)
                .await
                .expect("serve direct assistant downstream");
        });

        set_platform_override(
            &db,
            feature_flag_service::DIRECT_CHAT_ENGINE_FLAG_KEY,
            &FlagTarget::Global,
            true,
            "admin",
        )
        .await
        .unwrap();
        db.collection(USERS)
            .insert_one(crate::test_utils::test_user(USER_ID, UserType::Person))
            .await
            .expect("insert direct assistant user");
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = assistant_direct::DIRECT_LLM_SLUG.to_string();
        service.name = "Direct Chrono LLM".to_string();
        service.base_url = format!("http://{addr}/v1");
        service.service_category = "internal".to_string();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(service)
            .await
            .expect("insert direct assistant platform service");

        let state = crate::test_utils::test_app_state(db);
        let auth_user = crate::test_utils::test_auth_user(USER_ID);
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/assistant/direct/completions?api_key=caller-secret&stream=false")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(
                r#"{"messages":[{"role":"user","content":"hello"}]}"#,
            ))
            .unwrap();
        request
            .extensions_mut()
            .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));

        let response = completions(State(state), auth_user, request)
            .await
            .expect("direct completion should reach downstream");
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let forwarded_uri = tokio::time::timeout(std::time::Duration::from_secs(10), uri_rx)
            .await
            .expect("timed out after 10s waiting for downstream direct request")
            .expect("downstream URI sender should stay open");
        assert_eq!(forwarded_uri.path(), "/v1/chat/completions");
        assert_eq!(forwarded_uri.query(), None);

        drop(response);
        server.abort();
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

    #[test]
    fn response_body_holds_in_flight_slot_until_drop_and_isolates_users() {
        let limiter =
            std::sync::Arc::new(crate::mw::rate_limit::DirectChatRateLimiter::new(10, 60, 2));
        let response = || {
            Response::new(Body::from_stream(futures::stream::pending::<
                Result<Bytes, Infallible>,
            >()))
        };

        let first = attach_in_flight_permit(response(), limiter.try_acquire("user-a").unwrap());
        let second = attach_in_flight_permit(response(), limiter.try_acquire("user-a").unwrap());
        assert!(matches!(
            limiter.try_acquire("user-a"),
            Err(AppError::RateLimited)
        ));
        assert!(limiter.try_acquire("user-b").is_ok());

        drop(first);
        assert!(limiter.try_acquire("user-a").is_ok());
        drop(second);
    }
}
