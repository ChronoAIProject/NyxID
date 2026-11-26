use axum::Json;
use axum::extract::State;
use axum::http::{StatusCode, header::CACHE_CONTROL};
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::AppState;
use crate::crypto::aes::EncryptionDecryptStats;
use crate::errors::AppResult;
use crate::services::feature_flag_service;

/// GET /api/v1/admin/security-readiness. Observed support and durable activation,
/// including the optional legacy roster/drain diagnostics.
pub async fn security_readiness(
    State(state): State<AppState>,
    auth: crate::mw::auth::AuthUser,
) -> AppResult<
    Json<
        std::collections::BTreeMap<String, crate::services::security_readiness_service::Readiness>,
    >,
> {
    crate::handlers::admin_helpers::require_admin_or_operator(
        &state,
        &auth,
        "admin.security_readiness.get",
    )
    .await?;
    let mut reports = std::collections::BTreeMap::new();
    for capability in crate::services::security_readiness_service::CAPABILITIES
        .iter()
        .chain(std::iter::once(
            &crate::services::security_readiness_service::AUTOMATION_COMPLETION,
        ))
    {
        reports.insert(
            (*capability).to_owned(),
            crate::services::security_readiness_service::report(&state.db, capability).await?,
        );
    }
    Ok(Json(reports))
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub commit: String,
    pub encryption: EncryptionHealthResponse,
}

#[derive(Serialize)]
pub struct EncryptionHealthResponse {
    pub previous_key_configured: bool,
    pub decrypt_stats: EncryptionDecryptStats,
}

pub(crate) fn worker_health_router(
    jobs: std::sync::Arc<crate::services::job_health::Jobs>,
    shutdown: crate::services::shutdown::Runtime,
) -> axum::Router {
    #[derive(Serialize)]
    struct WorkerHealth {
        draining: bool,
        jobs: std::collections::BTreeMap<&'static str, crate::services::job_health::JobStatus>,
    }
    axum::Router::new().route(
        "/internal/v1/health/jobs",
        axum::routing::get(move || async move {
            (
                [(CACHE_CONTROL, "no-store")],
                Json(WorkerHealth {
                    draining: shutdown.is_draining(),
                    jobs: jobs.snapshot(),
                }),
            )
        }),
    )
}

/// GET /ready: preserve the drain-only, empty-body orchestration contract.
pub async fn ready(State(state): State<AppState>) -> Response {
    let status = if state.shutdown.is_draining() {
        StatusCode::SERVICE_UNAVAILABLE
    } else {
        StatusCode::OK
    };
    (status, [(CACHE_CONTROL, "no-store")]).into_response()
}

/// GET /health
///
/// Returns service health status. Used by load balancers and monitoring.
pub async fn health_check(State(state): State<AppState>) -> impl IntoResponse {
    (
        [(CACHE_CONTROL, "no-store")],
        Json(HealthResponse {
            status: "ok".to_string(),
            version: env!("CARGO_PKG_VERSION").to_string(),
            commit: reported_commit(std::env::var("NYXID_GIT_HASH").ok()),
            encryption: EncryptionHealthResponse {
                previous_key_configured: state.encryption_keys.has_previous(),
                decrypt_stats: state.encryption_keys.decrypt_stats(),
            },
        }),
    )
}

/// Container releases stamp this after compilation. Standalone binaries keep
/// their build.rs identity when there is no nonempty runtime override.
fn reported_commit(runtime: Option<String>) -> String {
    runtime
        .filter(|commit| !commit.is_empty())
        .unwrap_or_else(|| env!("NYXID_GIT_HASH").to_owned())
}

#[derive(Serialize)]
pub struct PublicConfigResponse {
    pub frontend_url: String,
    pub mcp_url: String,
    pub node_ws_url: String,
    pub version: String,
    pub social_providers: Vec<String>,
    pub invite_code_required: bool,
    pub email_auth_enabled: bool,
    /// Global `mobile:new-domains` admin flag. Mobile builds that embed both
    /// production domain sets read it from both API hosts at launch.
    pub mobile_new_domains: bool,
    /// Process-local rollout discovery hint, independent of account permission.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub crash_diagnostics_offered: bool,
    /// Public PostHog ingest key for the frontend. Non-secret by design
    /// (PostHog ingest keys are write-only and project-scoped). Empty
    /// when telemetry is off. See docs/TELEMETRY.md §3.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_dsn: Option<String>,
    /// Host for the telemetry vendor, e.g. `https://us.i.posthog.com`.
    /// Empty when the frontend should use its compiled-in default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub telemetry_host: Option<String>,
    /// Whether community share-back is opted in on this deployment.
    /// Frontend uses this to decide whether to fall back to the
    /// compiled-in public DSN when `telemetry_dsn` is empty. Omitted
    /// from the response when false so the default-off deploy's
    /// `/public/config` shape remains byte-identical to pre-telemetry.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub telemetry_share_analytics: bool,
}

/// GET /api/v1/public/config
///
/// Returns public configuration needed by the frontend (no auth required).
pub async fn public_config(State(state): State<AppState>) -> AppResult<Json<PublicConfigResponse>> {
    let invite_code_required = feature_flag_service::invitation_code_required(&state.db).await?;
    let mobile_new_domains = feature_flag_service::mobile_new_domains_enabled(&state.db).await?;
    let base = state.config.base_url.trim_end_matches('/');

    let mut social_providers = Vec::new();
    if state.config.github_client_id.is_some() && state.config.github_client_secret.is_some() {
        social_providers.push("github".to_string());
    }
    if state.config.google_client_id.is_some() && state.config.google_client_secret.is_some() {
        social_providers.push("google".to_string());
    }
    if state.config.apple_configured() {
        social_providers.push("apple".to_string());
    }

    let ws_base = base
        .replace("https://", "wss://")
        .replace("http://", "ws://");

    // Telemetry fields are only populated when the deployment has
    // opted in — either by setting an explicit DSN or by enabling
    // community share-back. This keeps the default-off `/public/config`
    // response byte-identical to a pre-telemetry deploy (no new JSON
    // keys leak). A deploy that sets only HOST but no DSN / no
    // share-back is treated as off.
    let telemetry_enabled = state
        .config
        .telemetry_dsn
        .as_ref()
        .is_some_and(|s| !s.is_empty())
        || state.config.share_analytics;

    let (telemetry_dsn, telemetry_host, telemetry_share_analytics) = if telemetry_enabled {
        (
            state
                .config
                .telemetry_dsn
                .as_ref()
                .filter(|s| !s.is_empty())
                .cloned(),
            state
                .config
                .telemetry_host
                .as_ref()
                .filter(|s| !s.is_empty())
                .cloned(),
            state.config.share_analytics,
        )
    } else {
        (None, None, false)
    };

    Ok(Json(PublicConfigResponse {
        frontend_url: state.config.frontend_url.trim_end_matches('/').to_string(),
        mcp_url: format!("{base}/mcp"),
        node_ws_url: format!("{ws_base}/api/v1/nodes/ws"),
        version: env!("CARGO_PKG_VERSION").to_string(),
        social_providers,
        invite_code_required,
        email_auth_enabled: state.config.email_auth_enabled,
        mobile_new_domains,
        crash_diagnostics_offered: crate::services::crash_report_service::public_config::offered(
            &state.db,
        ),
        telemetry_dsn,
        telemetry_host,
        telemetry_share_analytics,
    }))
}

/// Read-only metadata; no transcript, instruction, credentials or result bodies.
pub async fn automation_continuations(
    State(state): State<AppState>,
    auth: crate::mw::auth::AuthUser,
) -> AppResult<Json<crate::services::trigger_schedule::continuation::Diagnostic>> {
    crate::handlers::admin_helpers::require_admin_or_operator(
        &state,
        &auth,
        "admin.automation_continuations.get",
    )
    .await?;
    Ok(Json(
        crate::services::trigger_schedule::continuation::diagnostic(
            &state.db,
            state.config.automation_continuation_v2_enabled,
        )
        .await?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn assert_probe(
        state: AppState,
        path: &str,
        expected_status: axum::http::StatusCode,
    ) -> serde_json::Value {
        assert_probe_headers(state, path, expected_status, &[]).await
    }

    async fn assert_probe_headers(
        state: AppState,
        path: &str,
        expected_status: axum::http::StatusCode,
        headers: &[(&str, &str)],
    ) -> serde_json::Value {
        use tower::ServiceExt;
        let (_, router) = crate::routes::build_router_with_state(state.clone());
        let app = router
            .layer(axum::middleware::from_fn_with_state(
                state.clone(),
                crate::mw::csrf::browser_csrf_middleware,
            ))
            .layer(axum::middleware::from_fn_with_state(
                state.shutdown.clone(),
                crate::mw::shutdown::track,
            ))
            .with_state(state);
        let mut request = axum::http::Request::builder().uri(path);
        for (name, value) in headers {
            request = request.header(*name, *value);
        }
        let response = app
            .oneshot(request.body(axum::body::Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected_status);
        assert_eq!(response.headers()["cache-control"], "no-store");
        assert!(!response.headers().contains_key("location"));
        assert!(!response.headers().contains_key("www-authenticate"));
        if path != "/ready" {
            assert_eq!(response.headers()["content-type"], "application/json");
        }
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        if path == "/ready" {
            assert!(bytes.is_empty());
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        }
    }

    #[tokio::test]
    async fn readiness_http_stays_database_independent_and_preserves_drain_contract() {
        use axum::http::StatusCode;
        let state = crate::test_utils::test_app_state_no_db().await;
        assert_eq!(
            assert_probe(state.clone(), "/health", StatusCode::OK).await["status"],
            "ok"
        );
        assert_eq!(
            assert_probe(state.clone(), "/ready", StatusCode::OK).await,
            serde_json::Value::Null
        );
        state.shutdown.begin_drain();
        // Probe routes keep their contracts even after ordinary work is stopped.
        state.shutdown.stop();
        assert_eq!(
            assert_probe(state.clone(), "/health", StatusCode::OK).await["status"],
            "ok"
        );
        assert_eq!(
            assert_probe(state.clone(), "/ready", StatusCode::SERVICE_UNAVAILABLE).await,
            serde_json::Value::Null
        );
    }

    #[tokio::test]
    async fn readiness_production_routes_ignore_missing_and_invalid_login_credentials() {
        use axum::http::StatusCode;
        let state = crate::test_utils::test_app_state_no_db().await;
        for headers in [
            vec![],
            vec![("authorization", "Bearer invalid.expired.token")],
            vec![("cookie", "nyx_session=expired; nyx_access_token=expired")],
            vec![
                ("authorization", "Bearer nyx_invalid_agent_key"),
                ("x-api-key", "invalid"),
            ],
            vec![
                ("origin", "https://untrusted.example"),
                ("cookie", "nyx_session=expired"),
            ],
        ] {
            assert_eq!(
                assert_probe_headers(state.clone(), "/health", StatusCode::OK, &headers).await["status"],
                "ok"
            );
            assert_eq!(
                assert_probe_headers(state.clone(), "/ready", StatusCode::OK, &headers).await,
                serde_json::Value::Null
            );
        }
    }

    #[test]
    fn health_commit_prefers_runtime_stamp_and_preserves_standalone_fallback() {
        assert_eq!(
            reported_commit(Some("runtime-release".into())),
            "runtime-release"
        );
        assert_eq!(reported_commit(None), env!("NYXID_GIT_HASH"));
        assert_eq!(reported_commit(Some(String::new())), env!("NYXID_GIT_HASH"));
    }

    // ---- HealthResponse serialization ----

    fn default_decrypt_stats() -> EncryptionDecryptStats {
        EncryptionDecryptStats {
            v2_current: 0,
            v2_previous: 0,
            v2_fallback: 0,
            v1_current: 0,
            v1_previous: 0,
            v0_current: 0,
            v0_previous: 0,
            unknown_key_id_failures: 0,
            decrypt_failures: 0,
        }
    }

    #[test]
    fn health_response_serializes_all_fields() {
        let resp = HealthResponse {
            status: "ok".to_string(),
            version: "1.0.0".to_string(),
            commit: "abc1234".to_string(),
            encryption: EncryptionHealthResponse {
                previous_key_configured: false,
                decrypt_stats: default_decrypt_stats(),
            },
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["status"], "ok");
        assert_eq!(json["version"], "1.0.0");
        assert_eq!(json["commit"], "abc1234");
        assert_eq!(json["encryption"]["previous_key_configured"], false);
        assert!(json["encryption"]["decrypt_stats"].is_object());
    }

    #[test]
    fn health_response_with_previous_key() {
        let mut stats = default_decrypt_stats();
        stats.v2_current = 100;
        stats.v2_previous = 5;
        let resp = HealthResponse {
            status: "ok".to_string(),
            version: "2.0.0".to_string(),
            commit: "def5678".to_string(),
            encryption: EncryptionHealthResponse {
                previous_key_configured: true,
                decrypt_stats: stats,
            },
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["encryption"]["previous_key_configured"], true);
        assert_eq!(json["encryption"]["decrypt_stats"]["v2_current"], 100);
        assert_eq!(json["encryption"]["decrypt_stats"]["v2_previous"], 5);
    }

    // ---- PublicConfigResponse serialization ----

    #[test]
    fn public_config_response_serializes_required_fields() {
        let resp = PublicConfigResponse {
            frontend_url: "https://app.nyxid.dev".to_string(),
            mcp_url: "https://auth.nyxid.dev/mcp".to_string(),
            node_ws_url: "wss://auth.nyxid.dev/api/v1/nodes/ws".to_string(),
            version: "1.0.0".to_string(),
            social_providers: vec!["github".to_string()],
            invite_code_required: true,
            email_auth_enabled: true,
            mobile_new_domains: true,
            crash_diagnostics_offered: false,
            telemetry_dsn: None,
            telemetry_host: None,
            telemetry_share_analytics: false,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["frontend_url"], "https://app.nyxid.dev");
        assert_eq!(json["mcp_url"], "https://auth.nyxid.dev/mcp");
        assert_eq!(json["node_ws_url"], "wss://auth.nyxid.dev/api/v1/nodes/ws");
        assert_eq!(json["social_providers"], serde_json::json!(["github"]));
        assert_eq!(json["invite_code_required"], true);
        assert_eq!(json["email_auth_enabled"], true);
        assert_eq!(json["mobile_new_domains"], true);
    }

    #[test]
    fn public_config_response_omits_none_telemetry_fields() {
        let resp = PublicConfigResponse {
            frontend_url: "https://app.nyxid.dev".to_string(),
            mcp_url: "https://auth.nyxid.dev/mcp".to_string(),
            node_ws_url: "wss://auth.nyxid.dev/api/v1/nodes/ws".to_string(),
            version: "1.0.0".to_string(),
            social_providers: vec![],
            invite_code_required: false,
            email_auth_enabled: false,
            mobile_new_domains: false,
            crash_diagnostics_offered: false,
            telemetry_dsn: None,
            telemetry_host: None,
            telemetry_share_analytics: false,
        };
        let json = serde_json::to_value(&resp).unwrap();
        // skip_serializing_if means these should be absent
        assert!(json.get("telemetry_dsn").is_none());
        assert!(json.get("telemetry_host").is_none());
        // telemetry_share_analytics false => skipped (Not::not)
        assert!(json.get("telemetry_share_analytics").is_none());
        assert!(json.get("crash_diagnostics_offered").is_none());
        // Mobile launches must distinguish an explicit off from an older backend.
        assert_eq!(json["mobile_new_domains"], false);
    }

    #[test]
    fn public_config_response_includes_telemetry_when_set() {
        let resp = PublicConfigResponse {
            frontend_url: "https://app.nyxid.dev".to_string(),
            mcp_url: "https://auth.nyxid.dev/mcp".to_string(),
            node_ws_url: "wss://auth.nyxid.dev/api/v1/nodes/ws".to_string(),
            version: "1.0.0".to_string(),
            social_providers: vec![],
            invite_code_required: false,
            email_auth_enabled: false,
            mobile_new_domains: false,
            crash_diagnostics_offered: false,
            telemetry_dsn: Some("phc_test123".to_string()),
            telemetry_host: Some("https://us.i.posthog.com".to_string()),
            telemetry_share_analytics: true,
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert_eq!(json["telemetry_dsn"], "phc_test123");
        assert_eq!(json["telemetry_host"], "https://us.i.posthog.com");
        assert_eq!(json["telemetry_share_analytics"], true);
    }

    #[tokio::test]
    async fn public_config_crash_hint_omits_off_and_killed_includes_global_and_cohort_without_reads()
     {
        use crate::{
            models::user::{COLLECTION_NAME as USERS, User, UserType},
            services::{
                crash_report_service::{FLAG, admin, public_config as hint},
                feature_flag_service::{self as flags, FlagTarget},
                security_readiness_service as readiness,
            },
            test_utils::*,
        };
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{Arc, Mutex};

        let commands = Arc::new(Mutex::new(Vec::new()));
        let seen = commands.clone();
        let db = connect_test_database_with_command_handler(
            "crash_public_config_reads",
            EventHandler::callback(move |event| {
                if let CommandEvent::Started(event) = event {
                    seen.lock().unwrap().push(event.command);
                }
            }),
        )
        .await
        .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&id, UserType::Person))
            .await
            .unwrap();
        readiness::ready_test_replica(&db).await;
        let state = test_app_state(db.clone());
        for expected in [
            admin::Rollout::Off,
            admin::Rollout::Global,
            admin::Rollout::Killed,
            admin::Rollout::Cohort,
        ] {
            match expected {
                admin::Rollout::Global | admin::Rollout::Killed => {
                    flags::set_platform_override(
                        &db,
                        FLAG,
                        &FlagTarget::Global,
                        expected == admin::Rollout::Global,
                        &id,
                    )
                    .await
                    .unwrap();
                }
                admin::Rollout::Cohort => {
                    flags::set_platform_override(
                        &db,
                        FLAG,
                        &FlagTarget::User(id.clone()),
                        true,
                        &id,
                    )
                    .await
                    .unwrap();
                    flags::clear_platform_override(&db, FLAG, &FlagTarget::Global)
                        .await
                        .unwrap();
                }
                admin::Rollout::Off => {}
            }
            assert_eq!(admin::rollout(&db).await.unwrap(), expected);
            commands.lock().unwrap().clear();
            let value =
                serde_json::to_value(public_config(State(state.clone())).await.unwrap().0).unwrap();
            let offered = matches!(expected, admin::Rollout::Global | admin::Rollout::Cohort);
            if offered {
                assert_eq!(value["crash_diagnostics_offered"], true);
            } else {
                assert!(value.get("crash_diagnostics_offered").is_none());
            }
            assert_eq!(hint::offered(&db), offered);
            let reads = commands.lock().unwrap().clone();
            assert_eq!(
                reads.len(),
                2,
                "only the existing public flag reads: {reads:?}"
            );
            for (command, key) in reads
                .iter()
                .zip(["auth:invitation-code", "mobile:new-domains"])
            {
                assert_eq!(
                    command.get_str("find").unwrap(),
                    crate::models::feature_flag_override::COLLECTION_NAME
                );
                assert_eq!(
                    command
                        .get_document("filter")
                        .unwrap()
                        .get_str("flag_key")
                        .unwrap(),
                    key
                );
            }
        }
    }
}
