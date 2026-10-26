use super::*;
use crate::auth::refresh_tests::{Home, args, runtime};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
#[test]
fn all_request_helpers_propagate_refresh_rejection_and_proxy_never_replays() {
    let _home = Home::new();
    runtime().block_on(async {
        let server = MockServer::start().await;
        let auth = args(&server.uri());
        let profile = auth.profile.as_deref();
        crate::auth::refresh_tests::save_destination(profile, &server.uri()).unwrap();
        crate::auth::save_tokens_for(profile, "stale-access", Some("original-refresh")).unwrap();
        for helper in 0..13 {
            server.reset().await;
            Mock::given(path("/api/v1/auth/refresh"))
                .respond_with(ResponseTemplate::new(401).set_body_json(
                    serde_json::json!({"message":"Unauthorized: Session has expired"}),
                ))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(path(if helper == 12 {
                "/mcp"
            } else {
                "/api/v1/check"
            }))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
            let mut api = ApiClient::from_auth(&auth).unwrap();
            let body = serde_json::json!({});
            let result = match helper {
                0 => api.get::<serde_json::Value>("/check").await.map(|_| ()),
                1 => api
                    .get_optional::<serde_json::Value>("/check")
                    .await
                    .map(|_| ()),
                2 => api
                    .post::<serde_json::Value, _>("/check", &body)
                    .await
                    .map(|_| ()),
                3 => api
                    .put::<serde_json::Value, _>("/check", &body)
                    .await
                    .map(|_| ()),
                4 => api
                    .patch::<serde_json::Value, _>("/check", &body)
                    .await
                    .map(|_| ()),
                5 => api.delete_empty("/check").await,
                6 => api.delete::<serde_json::Value>("/check").await.map(|_| ()),
                7 => api
                    .delete_with_body::<serde_json::Value, _>("/check", &body)
                    .await
                    .map(|_| ()),
                8 => api
                    .delete_optional::<serde_json::Value>("/check")
                    .await
                    .map(|_| ()),
                9 => api.post_empty("/check", &body).await,
                10 => api.get_value("/check").await.map(|_| ()),
                11 => api
                    .proxy_request("POST", "/check", &[], None)
                    .await
                    .map(|_| ()),
                _ => api
                    .mcp_request(None, "2025-03-26", &body, true)
                    .await
                    .map(|_| ()),
            };
            let error = result.unwrap_err();
            let rendered: serde_json::Value =
                serde_json::from_str(&crate::error_format::render_error(&error, true)).unwrap();
            assert_eq!(
                rendered["error"], "session_expired",
                "helper {helper}: {error}"
            );
            assert_eq!(rendered["exit_code"], 3);
            server.verify().await;
        }
        server.reset().await;
        Mock::given(path("/api/v1/check"))
            .respond_with(ResponseTemplate::new(401))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/auth/refresh"))
            .respond_with(
                ResponseTemplate::new(503)
                    .insert_header("Retry-After", "9999")
                    .set_body_json(serde_json::json!({})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut api = ApiClient::from_auth(&auth).unwrap();
        let error = api
            .proxy_request("GET", "/check", &[], None)
            .await
            .unwrap_err();
        let rendered: serde_json::Value =
            serde_json::from_str(&crate::error_format::render_error(&error, true)).unwrap();
        assert_eq!(rendered["error"], "refresh_unavailable");
        assert_eq!(rendered["exit_code"], 4);
        server.verify().await;
        for disabled in [true, false] {
            server.reset().await;
            Mock::given(path("/api/v1/check"))
                .respond_with(ResponseTemplate::new(401))
                .expect(1)
                .mount(&server)
                .await;
            Mock::given(path("/api/v1/auth/refresh"))
                .respond_with(ResponseTemplate::new(500))
                .expect(0)
                .mount(&server)
                .await;
            let mut api = if disabled {
                ApiClient::from_auth(&auth).unwrap().without_write_refresh()
            } else {
                let mut explicit = auth.clone();
                explicit.access_token = Some("explicit".into());
                ApiClient::from_auth(&explicit).unwrap()
            };
            let error = if disabled {
                let response = api
                    .proxy_request("POST", "/check", &[], Some(b"raw-body"))
                    .await
                    .unwrap();
                assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
                ApiError::new("/check", response.status(), String::new()).into()
            } else {
                api.post::<serde_json::Value, _>("/check", &serde_json::json!({}))
                    .await
                    .unwrap_err()
            };
            assert!(error.is::<ApiError>());
            server.verify().await;
        }
    });
}
