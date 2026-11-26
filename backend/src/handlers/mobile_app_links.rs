//! First-party app links use the API origin already trusted by released clients.
//! Opening a link only identifies a request; approval and delivery stay in the
//! existing authenticated device-login protocol.

use axum::{
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;

use crate::{
    AppState,
    config::{
        AppConfig,
        mobile_app_links::{apple_app_id, has_android_identity},
    },
    errors::{AppError, AppResult},
    services::auth_device_service,
};

/// Configured identity permits advertising a link; it does not establish that
/// an external OS association is operational. Never derive authority from Host.
pub(super) fn app_verification_uri(config: &AppConfig, user_code: &str) -> Option<String> {
    if apple_app_id(config).is_none() && !has_android_identity(config) {
        return None;
    }
    let mut url = url::Url::parse(config.base_url.trim()).ok()?;
    let frontend = url::Url::parse(config.frontend_url.trim()).ok()?;
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
        || !matches!(frontend.scheme(), "http" | "https")
        || url.origin() == frontend.origin()
    {
        return None;
    }
    url.set_path("/login/device");
    url.query_pairs_mut().append_pair("user_code", user_code);
    Some(url.to_string())
}

#[derive(Deserialize)]
pub struct LoginLinkQuery {
    user_code: String,
}

/// Browser fallback when the OS does not open an installed app. The target is
/// server-configured, and only the validated public code crosses the redirect.
pub async fn device_login(
    State(state): State<AppState>,
    Query(query): Query<LoginLinkQuery>,
) -> AppResult<Response> {
    let code = auth_device_service::normalize_user_code(&query.user_code)?;
    let mut target = url::Url::parse(state.config.frontend_url.trim())
        .map_err(|_| AppError::Internal("login frontend is not configured".into()))?;
    if !matches!(target.scheme(), "http" | "https") {
        return Err(AppError::Internal("login frontend is invalid".into()));
    }
    if url::Url::parse(state.config.base_url.trim())
        .is_ok_and(|base| base.origin() == target.origin())
    {
        return Ok(StatusCode::NOT_FOUND.into_response());
    }
    target.set_path("/login/device");
    target.set_query(None);
    target.set_fragment(None);
    target
        .query_pairs_mut()
        .append_pair("user_code", &auth_device_service::format_user_code(&code));
    Ok((
        [
            (header::CACHE_CONTROL, "no-store"),
            (header::REFERRER_POLICY, "no-referrer"),
        ],
        Redirect::to(target.as_str()),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Router, body::Body, http::Request, routing::get};
    use tower::ServiceExt;

    #[test]
    fn app_link_uses_only_the_configured_https_api_origin_and_public_code() {
        let mut config = crate::test_utils::test_app_config();
        config.base_url = "https://api.example.com".into();
        config.apns_team_id = Some("TEAM123".into());
        config.apns_topic = Some("com.example.nyxid".into());
        assert_eq!(
            app_verification_uri(&config, "ABCD-EFGH").unwrap(),
            "https://api.example.com/login/device?user_code=ABCD-EFGH"
        );
        for invalid in [
            "http://localhost:3001",
            "nyxid://login",
            "https://user@api.example.com",
            "https://api.example.com/api/v1",
            "https://api.example.com?redirect=evil",
            "https://api.example.com/#fragment",
        ] {
            config.base_url = invalid.into();
            assert!(app_verification_uri(&config, "ABCD-EFGH").is_none());
        }
    }

    #[test]
    fn app_link_requires_configured_apple_or_android_identity() {
        let mut config = crate::test_utils::test_app_config();
        config.base_url = "https://api.example.com".into();
        assert!(app_verification_uri(&config, "ABCD-EFGH").is_none());
        for (team, bundle) in [("TEAM123", " "), ("\t", "com.example.nyxid"), ("", "")] {
            config.apns_team_id = Some(team.into());
            config.apns_topic = Some(bundle.into());
            assert!(app_verification_uri(&config, "ABCD-EFGH").is_none());
        }
        config.android_app_package = Some("com.example.nyxid".into());
        assert!(app_verification_uri(&config, "ABCD-EFGH").is_none());
        config.android_app_sha256_cert_fingerprints = vec![vec!["AB"; 32].join(":")];
        assert!(app_verification_uri(&config, "ABCD-EFGH").is_some());
    }

    #[test]
    fn app_link_compares_canonical_origins_including_scheme_and_port() {
        let mut config = crate::test_utils::test_app_config();
        config.base_url = "https://API.example.com:443/".into();
        config.apns_team_id = Some(" TEAM123 ".into());
        config.apns_topic = Some(" com.example.nyxid ".into());
        for frontend in [
            "https://api.example.com",
            "https://api.example.com:443/some/path",
        ] {
            config.frontend_url = frontend.into();
            assert!(app_verification_uri(&config, "ABCD-EFGH").is_none());
        }
        for frontend in [
            "http://api.example.com",
            "https://api.example.com:444",
            "https://app.example.com",
        ] {
            config.frontend_url = frontend.into();
            assert!(app_verification_uri(&config, "ABCD-EFGH").is_some());
        }
    }

    #[tokio::test]
    async fn app_link_fallback_refuses_same_origin_redirect_loop() {
        let mut state = crate::test_utils::test_app_state_no_db().await;
        state.config.base_url = "https://API.example.com:443/".into();
        state.config.frontend_url = "https://api.example.com".into();
        let app = Router::new()
            .route("/login/device", get(device_login))
            .with_state(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/login/device?user_code=ABCD-EFGH")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        assert!(response.headers().get(header::LOCATION).is_none());
    }

    #[tokio::test]
    async fn app_link_fallback_formats_numeric_code_as_a_string_for_frontend_search() {
        let mut state = crate::test_utils::test_app_state_no_db().await;
        state.config.frontend_url = "https://nyxid.example".into();
        let app = Router::new()
            .route("/login/device", get(device_login))
            .with_state(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/login/device?user_code=23456789")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers()[header::LOCATION],
            "https://nyxid.example/login/device?user_code=2345-6789"
        );
    }

    #[tokio::test]
    async fn app_link_falls_back_to_fixed_frontend_without_forwarding_extra_parameters() {
        let mut state = crate::test_utils::test_app_state_no_db().await;
        state.config.frontend_url = "https://nyxid.example".into();
        let app = Router::new()
            .route("/login/device", get(device_login))
            .with_state(state);
        let response = app.clone().oneshot(Request::builder()
            .uri("/login/device?user_code=abcd-efgh&redirect=https://evil.example&device_code=secret")
            .header(header::HOST, "evil.example")
            .body(Body::empty()).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response.headers()[header::LOCATION],
            "https://nyxid.example/login/device?user_code=ABCD-EFGH"
        );
        assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
        assert_eq!(response.headers()[header::REFERRER_POLICY], "no-referrer");
        for query in [
            "",
            "?user_code=bad",
            "?user_code=ABCDEFGH&user_code=JKMNPQRS",
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/login/device{query}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
            assert!(response.headers().get(header::LOCATION).is_none());
        }
    }
}
