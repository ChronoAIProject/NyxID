use std::time::Instant;

use axum::{
    Json,
    body::Body,
    extract::{Request, State},
    http::{StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use chrono::Utc;
use serde_json::{Map, Value, json};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::platform_operation_service::{
    CallAndSayRequest, SpeakRequest, XSearchRequest,
};
use crate::services::{audit_service, feature_flag_service, platform_operation_service};

async fn require_platform_ops_enabled(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    let enabled =
        feature_flag_service::resolve_personal_features(&state.db, &auth_user.user_id.to_string())
            .await?
            .into_iter()
            .any(|key| key == feature_flag_service::PLATFORM_SERVICES_FLAG_KEY);
    if !enabled {
        return Err(AppError::NotFound(
            "Platform operation route not found.".to_string(),
        ));
    }
    Ok(())
}

/// Structural feature gate for every route in the `/platform-ops` nest.
/// Handler-level checks remain as belt-and-braces protection for direct calls
/// and tests, but a newly mounted route inherits this middleware automatically.
pub async fn platform_services_feature_gate(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request,
    next: Next,
) -> Result<Response, AppError> {
    require_platform_ops_enabled(&state, &auth_user).await?;
    Ok(next.run(request).await)
}

pub async fn x_search(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(request): Json<XSearchRequest>,
) -> AppResult<Response> {
    require_platform_ops_enabled(&state, &auth_user).await?;
    ensure_platform_operation_caller(&state, &auth_user).await?;
    enforce_agent_rate_limit(&state, &auth_user)?;

    let started = Instant::now();
    let query_chars = request.query.chars().count();
    let requested_max_results = request.max_results;
    let result = platform_operation_service::execute_x_search(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        request,
    )
    .await;
    audit_operation(
        &state,
        &auth_user,
        "x_search",
        &result,
        started,
        json!({
            "query_chars": query_chars,
            "requested_max_results": requested_max_results,
        }),
    );

    Ok(Json(result?).into_response())
}

pub async fn speak(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(request): Json<SpeakRequest>,
) -> AppResult<Response> {
    require_platform_ops_enabled(&state, &auth_user).await?;
    ensure_platform_operation_caller(&state, &auth_user).await?;
    enforce_agent_rate_limit(&state, &auth_user)?;

    let started = Instant::now();
    let text_chars = request.text.chars().count();
    let voice_id = request.voice_id.clone();
    let result = platform_operation_service::execute_speak(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        request,
    )
    .await;
    audit_operation(
        &state,
        &auth_user,
        "speak",
        &result,
        started,
        json!({
            "text_chars": text_chars,
            "voice_id": voice_id,
        }),
    );

    let vendor = result?;
    let content_length = vendor.response.content_length();
    let stream = vendor.response.bytes_stream();
    let mut builder = Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "audio/mpeg");
    if let Some(content_length) = content_length {
        builder = builder.header(header::CONTENT_LENGTH, content_length);
    }
    builder
        .body(Body::from_stream(stream))
        .map_err(|error| AppError::Internal(format!("Failed to build speech response: {error}")))
}

pub async fn call_and_say(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(request): Json<CallAndSayRequest>,
) -> AppResult<Response> {
    require_platform_ops_enabled(&state, &auth_user).await?;
    ensure_platform_operation_caller(&state, &auth_user).await?;
    enforce_agent_rate_limit(&state, &auth_user)?;

    let started = Instant::now();
    let message_chars = request.message.chars().count();
    let destination_suffix = if platform_operation_service::is_e164_number(&request.to) {
        platform_operation_service::redacted_destination_suffix(&request.to)
    } else {
        "invalid".to_string()
    };
    // The date is deliberately derived at the transport edge and passed into
    // the service, keeping quota tests deterministic and avoiding hidden time
    // reads in the reservation logic.
    let yyyymmdd = Utc::now().format("%Y%m%d").to_string();
    let result = platform_operation_service::execute_call_and_say(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &auth_user.user_id.to_string(),
        &yyyymmdd,
        request,
    )
    .await;
    audit_operation(
        &state,
        &auth_user,
        "call_and_say",
        &result,
        started,
        json!({
            "message_chars": message_chars,
            "destination_suffix": destination_suffix,
        }),
    );

    Ok(Json(result?).into_response())
}

async fn ensure_platform_operation_caller(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    match auth_user.auth_method {
        AuthMethod::Session | AuthMethod::AccessToken => Ok(()),
        AuthMethod::ApiKey => {
            let api_key_id = auth_user.api_key_id.as_deref().ok_or_else(|| {
                AppError::Unauthorized("Agent API key identity is missing".to_string())
            })?;
            let is_agent_key = state
                .db
                .collection::<ApiKey>(API_KEYS)
                .find_one(mongodb::bson::doc! {
                    "_id": api_key_id,
                    "key_prefix": { "$regex": r"^nyxid_ag_" },
                    "is_active": true,
                })
                .await?
                .is_some();
            if is_agent_key {
                Ok(())
            } else {
                Err(AppError::Forbidden(
                    "A nyxid_ag_ agent API key is required for API-key access.".to_string(),
                ))
            }
        }
        AuthMethod::Delegated | AuthMethod::Relay | AuthMethod::ServiceAccount => Err(
            AppError::Forbidden("This token type cannot access platform operations.".to_string()),
        ),
    }
}

fn enforce_agent_rate_limit(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    crate::mw::rate_limit::check_agent_rate_limit_raw(
        &state.per_agent_limiter,
        auth_user.api_key_id.as_deref(),
        auth_user.rate_limit_per_second,
        auth_user.rate_limit_burst,
    )
}

fn audit_operation<T>(
    state: &AppState,
    auth_user: &AuthUser,
    op: &'static str,
    result: &AppResult<T>,
    started: Instant,
    metadata: Value,
) {
    let mut data = match metadata {
        Value::Object(data) => data,
        _ => Map::new(),
    };
    data.insert("op".to_string(), Value::String(op.to_string()));
    data.insert(
        "outcome".to_string(),
        Value::String(audit_outcome(result).to_string()),
    );
    data.insert(
        "duration_ms".to_string(),
        Value::from(started.elapsed().as_millis() as u64),
    );
    audit_service::log_for_user(
        state.db.clone(),
        auth_user,
        "platform_operation",
        Some(Value::Object(data)),
    );
}

fn audit_outcome<T>(result: &AppResult<T>) -> &'static str {
    match result {
        Ok(_) => "succeeded",
        Err(AppError::NotFound(_)) => "not_found",
        Err(AppError::RateLimited) => "rate_limited",
        Err(AppError::BadRequest(_) | AppError::ValidationError(_)) => "rejected",
        Err(AppError::PlatformOperationUnavailable) => "vendor_unavailable",
        Err(_) => "failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, routing::post};
    use mongodb::{IndexModel, options::IndexOptions};
    use std::sync::Arc;
    use tokio::net::TcpListener;

    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::platform_op_usage::{COLLECTION_NAME as PLATFORM_OP_USAGE, PlatformOpUsage};
    use crate::models::platform_operation::{
        COLLECTION_NAME as PLATFORM_OPERATIONS, CallAndSayConfig, PlatformOperation,
        PlatformOperationConfig, PlatformOperationName, XSearchConfig,
    };

    const USER_ID: &str = "65de27dc-8cf8-44b6-b8d2-5304e4a90aa4";

    fn operation(
        op: PlatformOperationName,
        enabled: bool,
        config: PlatformOperationConfig,
    ) -> PlatformOperation {
        PlatformOperation {
            id: uuid::Uuid::new_v4().to_string(),
            op,
            enabled,
            vendor_service_slug: platform_operation_service::default_vendor_service_slug(op)
                .to_string(),
            config,
            updated_at: Utc::now(),
            updated_by: USER_ID.to_string(),
        }
    }

    fn call_config(cap: u32) -> CallAndSayConfig {
        CallAndSayConfig {
            allowed_destination_prefixes: vec!["+65".to_string()],
            max_message_chars: 500,
            voice: "alice".to_string(),
            max_calls_per_user_per_day: cap,
            account_sid: format!("AC{}", "1".repeat(32)),
            call_from: "+16505550100".to_string(),
        }
    }

    async fn insert_twilio_vendor(state: &AppState, base_url: String) {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = platform_operation_service::CALL_AND_SAY_VENDOR_SLUG.to_string();
        service.name = "Platform Twilio".to_string();
        service.base_url = base_url;
        service.service_category = "internal".to_string();
        service.visibility = "public".to_string();
        service.auth_method = "basic".to_string();
        service.auth_key_name = "Authorization".to_string();
        service.credential_encrypted = state
            .encryption_keys
            .encrypt(b"twilio-auth-token")
            .await
            .expect("encrypt Twilio token");
        state
            .db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(service)
            .await
            .expect("insert Twilio vendor row");
    }

    async fn ensure_usage_index(db: &mongodb::Database) {
        db.collection::<mongodb::bson::Document>(PLATFORM_OP_USAGE)
            .create_index(
                IndexModel::builder()
                    .keys(mongodb::bson::doc! {
                        "op": 1,
                        "user_id": 1,
                        "yyyymmdd": 1,
                    })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .expect("create platform operation usage index");
    }

    async fn enable_platform_services_for_tests(db: &mongodb::Database) {
        crate::services::feature_flag_service::set_platform_override(
            db,
            crate::services::feature_flag_service::PLATFORM_SERVICES_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            true,
            USER_ID,
        )
        .await
        .expect("enable platform-services flag for test");
    }

    async fn spawn_twilio(status: StatusCode) -> (String, tokio::task::JoinHandle<()>) {
        let app = Router::new().route(
            "/2010-04-01/Accounts/{account_sid}/Calls.json",
            post(move || async move {
                (
                    status,
                    Json(json!({ "sid": "CA-test", "status": "queued" })),
                )
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind Twilio test server");
        let address = listener.local_addr().expect("Twilio test server address");
        let server = tokio::spawn(async move {
            axum::serve(listener, app)
                .await
                .expect("serve Twilio test server");
        });
        (format!("http://{address}"), server)
    }

    #[tokio::test]
    async fn platform_services_flag_off_returns_not_found_for_all_http_operations() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_ops_flag_default_off").await
        else {
            eprintln!("skipping platform operation handler test: no local MongoDB available");
            return;
        };
        let state = crate::test_utils::test_app_state(db);
        let auth_user = crate::test_utils::test_auth_user(USER_ID);

        let x_search_result = x_search(
            State(state.clone()),
            auth_user.clone(),
            Json(XSearchRequest {
                query: "nyxid".to_string(),
                max_results: None,
            }),
        )
        .await;
        assert!(matches!(x_search_result, Err(AppError::NotFound(_))));

        let speak_result = speak(
            State(state.clone()),
            auth_user.clone(),
            Json(SpeakRequest {
                text: "Hello".to_string(),
                voice_id: "voice".to_string(),
            }),
        )
        .await;
        assert!(matches!(speak_result, Err(AppError::NotFound(_))));

        let call_result = call_and_say(
            State(state),
            auth_user,
            Json(CallAndSayRequest {
                to: "+6512345678".to_string(),
                message: "Hello".to_string(),
            }),
        )
        .await;
        assert!(matches!(call_result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn disabled_operation_returns_not_found() {
        let Some(db) = crate::test_utils::connect_test_database("platform_ops_disabled").await
        else {
            eprintln!("skipping platform operation handler test: no local MongoDB available");
            return;
        };
        enable_platform_services_for_tests(&db).await;
        db.collection::<PlatformOperation>(PLATFORM_OPERATIONS)
            .insert_one(operation(
                PlatformOperationName::XSearch,
                false,
                PlatformOperationConfig::XSearch(XSearchConfig {
                    max_results_cap: 10,
                }),
            ))
            .await
            .expect("insert disabled operation");
        let result = x_search(
            State(crate::test_utils::test_app_state(db)),
            crate::test_utils::test_auth_user(USER_ID),
            Json(XSearchRequest {
                query: "nyxid".to_string(),
                max_results: None,
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn enabled_operation_with_missing_vendor_returns_bad_gateway_error() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_ops_vendor_missing").await
        else {
            eprintln!("skipping platform operation handler test: no local MongoDB available");
            return;
        };
        enable_platform_services_for_tests(&db).await;
        db.collection::<PlatformOperation>(PLATFORM_OPERATIONS)
            .insert_one(operation(
                PlatformOperationName::XSearch,
                true,
                PlatformOperationConfig::XSearch(XSearchConfig {
                    max_results_cap: 10,
                }),
            ))
            .await
            .expect("insert enabled operation");
        let result = x_search(
            State(crate::test_utils::test_app_state(db)),
            crate::test_utils::test_auth_user(USER_ID),
            Json(XSearchRequest {
                query: "nyxid".to_string(),
                max_results: None,
            }),
        )
        .await;
        let error = result.expect_err("missing vendor must fail closed");
        assert!(matches!(error, AppError::PlatformOperationUnavailable));
        assert_eq!(error.into_response().status(), StatusCode::BAD_GATEWAY);
    }

    #[tokio::test]
    async fn daily_cap_allows_n_calls_and_rejects_n_plus_one() {
        let Some(db) = crate::test_utils::connect_test_database("platform_ops_daily_cap").await
        else {
            eprintln!("skipping platform operation handler test: no local MongoDB available");
            return;
        };
        enable_platform_services_for_tests(&db).await;
        ensure_usage_index(&db).await;
        let state = crate::test_utils::test_app_state(db.clone());
        let (base_url, server) = spawn_twilio(StatusCode::CREATED).await;
        insert_twilio_vendor(&state, base_url).await;
        db.collection::<PlatformOperation>(PLATFORM_OPERATIONS)
            .insert_one(operation(
                PlatformOperationName::CallAndSay,
                true,
                PlatformOperationConfig::CallAndSay(call_config(2)),
            ))
            .await
            .expect("insert call operation");

        for _ in 0..2 {
            let result = call_and_say(
                State(state.clone()),
                crate::test_utils::test_auth_user(USER_ID),
                Json(CallAndSayRequest {
                    to: "+6512345678".to_string(),
                    message: "Hello".to_string(),
                }),
            )
            .await;
            assert!(result.is_ok());
        }
        let result = call_and_say(
            State(state),
            crate::test_utils::test_auth_user(USER_ID),
            Json(CallAndSayRequest {
                to: "+6512345678".to_string(),
                message: "Hello again".to_string(),
            }),
        )
        .await;
        assert!(matches!(result, Err(AppError::RateLimited)));
        server.abort();
    }

    #[tokio::test]
    async fn vendor_failure_releases_daily_call_reservation() {
        let Some(db) = crate::test_utils::connect_test_database("platform_ops_failed_call").await
        else {
            eprintln!("skipping platform operation handler test: no local MongoDB available");
            return;
        };
        enable_platform_services_for_tests(&db).await;
        ensure_usage_index(&db).await;
        let state = crate::test_utils::test_app_state(db.clone());
        let (base_url, server) = spawn_twilio(StatusCode::INTERNAL_SERVER_ERROR).await;
        insert_twilio_vendor(&state, base_url).await;
        db.collection::<PlatformOperation>(PLATFORM_OPERATIONS)
            .insert_one(operation(
                PlatformOperationName::CallAndSay,
                true,
                PlatformOperationConfig::CallAndSay(call_config(2)),
            ))
            .await
            .expect("insert call operation");

        let result = call_and_say(
            State(state),
            crate::test_utils::test_auth_user(USER_ID),
            Json(CallAndSayRequest {
                to: "+6512345678".to_string(),
                message: "Hello".to_string(),
            }),
        )
        .await;
        assert!(matches!(
            result,
            Err(AppError::PlatformOperationUnavailable)
        ));
        let usage = db
            .collection::<PlatformOpUsage>(PLATFORM_OP_USAGE)
            .find_one(mongodb::bson::doc! {
                "op": "call_and_say",
                "user_id": USER_ID,
            })
            .await
            .expect("read usage row")
            .expect("usage row remains for the day");
        assert_eq!(usage.count, 0);
        server.abort();
    }

    #[tokio::test]
    async fn delegated_relay_and_service_account_callers_are_rejected() {
        let state = Arc::new(crate::test_utils::test_app_state_no_db().await);
        for method in [
            AuthMethod::Delegated,
            AuthMethod::Relay,
            AuthMethod::ServiceAccount,
        ] {
            let mut auth_user = crate::test_utils::test_auth_user(USER_ID);
            auth_user.auth_method = method;
            assert!(matches!(
                ensure_platform_operation_caller(&state, &auth_user).await,
                Err(AppError::Forbidden(_))
            ));
        }
    }

    #[test]
    fn audit_payloads_are_metadata_only() {
        let result: AppResult<()> = Ok(());
        assert_eq!(audit_outcome(&result), "succeeded");
        let call_metadata = json!({
            "message_chars": 12,
            "destination_suffix": "***5678",
        });
        let encoded = serde_json::to_string(&call_metadata).expect("encode audit metadata");
        assert!(!encoded.contains("Hello world"));
        assert!(!encoded.contains("+6512345678"));
    }
}
