use axum::{
    Json,
    extract::{Path, State},
    http::header,
    response::{Html, IntoResponse, Response},
};
use utoipa::OpenApi;

use crate::AppState;
use crate::errors::AppError;
use crate::errors::AppResult;
use crate::handlers::services_helpers::{ResolvedService, resolve_service_or_user_service};
use crate::mw::auth::AuthUser;
use crate::services::api_docs_service;

#[utoipa::path(
    get,
    path = "/api/v1/docs",
    responses(
        (status = 200, description = "Scalar API reference for NyxID", content_type = "text/html")
    ),
    tag = "Docs"
)]
pub async fn docs_ui(State(state): State<AppState>, _auth_user: AuthUser) -> impl IntoResponse {
    let spec_url = format!(
        "{}/api/v1/docs/openapi.json",
        state.config.base_url.trim_end_matches('/')
    );
    html_response_with_csp(
        api_docs_service::render_scalar_html("NyxID API Docs", &spec_url),
        &api_docs_service::scalar_docs_csp(),
    )
}

#[utoipa::path(
    get,
    path = "/api/v1/docs/catalog",
    responses(
        (status = 200, description = "NyxID unified API catalog", content_type = "text/html")
    ),
    tag = "Docs"
)]
pub async fn catalog_ui(_state: State<AppState>, _auth_user: AuthUser) -> impl IntoResponse {
    html_response_with_csp(
        api_docs_service::render_catalog_html().to_string(),
        api_docs_service::catalog_csp(),
    )
}

#[utoipa::path(
    get,
    path = "/api/v1/docs/openapi.json",
    responses(
        (status = 200, description = "NyxID OpenAPI 3.1 specification", content_type = "application/json")
    ),
    tag = "Docs"
)]
pub async fn openapi_json(
    State(state): State<AppState>,
    _auth_user: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    let mut openapi = crate::api_docs::ApiDoc::openapi();
    openapi.openapi = utoipa::openapi::OpenApiVersion::Version31;
    openapi.servers = Some(vec![utoipa::openapi::Server::new(
        state.config.base_url.trim_end_matches('/'),
    )]);

    let value = serde_json::to_value(openapi).map_err(|e| {
        crate::errors::AppError::Internal(format!("Failed to serialize OpenAPI document: {e}"))
    })?;
    Ok(Json(value))
}

#[utoipa::path(
    get,
    path = "/api/v1/docs/asyncapi.json",
    responses(
        (status = 200, description = "NyxID AsyncAPI 3.0 specification", content_type = "application/json")
    ),
    tag = "Docs"
)]
pub async fn asyncapi_json(
    State(state): State<AppState>,
    _auth_user: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(api_docs_service::build_asyncapi_document(
        &state.config.base_url,
    )))
}

#[utoipa::path(
    get,
    path = "/api/v1/proxy/services/{service_id}/docs",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 200, description = "Scalar API reference for the downstream service", content_type = "text/html"),
        (status = 404, description = "Service or documentation not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Proxy Docs"
)]
pub async fn service_docs_ui(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Response> {
    auth_user.ensure_rest_proxy_access()?;

    let base = state.config.base_url.trim_end_matches('/');
    let caller_user_id = auth_user.user_id.to_string();

    match resolve_service_or_user_service(&state, &service_id, &caller_user_id).await? {
        ResolvedService::Catalog(service) => {
            let spec_url = if service.openapi_spec_url.is_some() {
                format!("{base}/api/v1/proxy/services/{service_id}/openapi.json")
            } else if service.asyncapi_spec_url.is_some() {
                format!("{base}/api/v1/proxy/services/{service_id}/asyncapi.json")
            } else {
                return Err(AppError::NotFound(
                    "Service has no documentation spec configured".to_string(),
                ));
            };

            Ok(html_response_with_csp(
                api_docs_service::render_scalar_html(
                    &format!("{} API Docs", service.name),
                    &spec_url,
                ),
                &api_docs_service::scalar_docs_csp(),
            ))
        }
        ResolvedService::Owned {
            user_service,
            user_endpoint,
            ..
        } => {
            if user_endpoint.openapi_spec_url.is_none() {
                return Err(AppError::NotFound(
                    "Service has no documentation spec configured".to_string(),
                ));
            }

            let title = if user_endpoint.label.trim().is_empty() {
                user_service.slug.as_str()
            } else {
                user_endpoint.label.as_str()
            };
            let spec_url = format!("{base}/api/v1/proxy/services/{service_id}/openapi.json");
            Ok(html_response_with_csp(
                api_docs_service::render_scalar_html(&format!("{title} API Docs"), &spec_url),
                &api_docs_service::scalar_docs_csp(),
            ))
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/proxy/services/{service_id}/openapi.json",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 200, description = "Proxied OpenAPI document for a downstream service", content_type = "application/json"),
        (status = 404, description = "Service or spec not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Proxy Docs"
)]
pub async fn service_openapi_json(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Response> {
    auth_user.ensure_rest_proxy_access()?;

    let caller_user_id = auth_user.user_id.to_string();

    match resolve_service_or_user_service(&state, &service_id, &caller_user_id).await? {
        ResolvedService::Catalog(service) => {
            let spec =
                api_docs_service::fetch_downstream_openapi_spec(&service, &state.config.base_url)
                    .await?;
            Ok(Json(spec).into_response())
        }
        ResolvedService::Owned {
            user_endpoint,
            owner_id,
            ..
        } => {
            let Some(spec_url) = user_endpoint.openapi_spec_url.as_deref() else {
                return Err(AppError::NotFound(
                    "Service has no documentation spec configured".to_string(),
                ));
            };

            // Serialize the shared `Arc<Value>` directly into bytes so we
            // don't deep-clone the parsed spec tree on every cache hit.
            let spec = api_docs_service::fetch_spec_json_scoped(spec_url, &owner_id).await?;
            let body = serde_json::to_vec(spec.as_ref())
                .map_err(|err| AppError::Internal(format!("serialize openapi spec: {err}")))?;
            Ok(([(header::CONTENT_TYPE, "application/json")], body).into_response())
        }
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/proxy/services/{service_id}/asyncapi.json",
    params(
        ("service_id" = String, Path, description = "Downstream service ID")
    ),
    responses(
        (status = 200, description = "Proxied AsyncAPI document for a downstream service", content_type = "application/json"),
        (status = 404, description = "Service or spec not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Proxy Docs"
)]
pub async fn service_asyncapi_json(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    auth_user.ensure_rest_proxy_access()?;

    let caller_user_id = auth_user.user_id.to_string();

    match resolve_service_or_user_service(&state, &service_id, &caller_user_id).await? {
        ResolvedService::Catalog(service) => Ok(Json(
            api_docs_service::fetch_downstream_asyncapi_spec(&service, &state.config.base_url)
                .await?,
        )),
        ResolvedService::Owned { .. } => Err(AppError::NotFound(
            "Service has no AsyncAPI spec configured".to_string(),
        )),
    }
}

fn html_response_with_csp(html: String, csp: &str) -> Response {
    let mut response = Html(html).into_response();
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        csp.parse().expect("valid docs CSP header"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::service_openapi_json;
    use crate::errors::AppError;
    use crate::models::user::COLLECTION_NAME as USERS;
    use crate::models::user::UserType;
    use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::services::api_docs_service::{cache_test_spec, clear_test_spec_cache};
    use crate::test_utils::{
        connect_test_database, test_app_state, test_auth_user, test_user, test_user_endpoint,
        test_user_service,
    };
    use axum::{
        body::to_bytes,
        extract::{Path, State},
    };
    use uuid::Uuid;

    const SPEC_URL: &str = "https://example.com/service-openapi.json";

    fn openapi_spec() -> serde_json::Value {
        serde_json::json!({
            "openapi": "3.1.0",
            "info": { "title": "Custom API", "version": "1.0.0" },
            "paths": {
                "/ping": {
                    "get": {
                        "responses": {
                            "200": { "description": "ok" }
                        }
                    }
                }
            }
        })
    }

    #[tokio::test]
    async fn service_openapi_json_returns_owned_user_service_spec() {
        let Some(db) = connect_test_database("docs_openapi_owned").await else {
            eprintln!("skipping docs integration test: no local MongoDB available");
            return;
        };
        clear_test_spec_cache();

        let caller_id = Uuid::new_v4().to_string();
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "Custom API",
            "https://custom.example.com",
            Some(SPEC_URL),
            None,
        );
        let user_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "custom-api",
            &endpoint.id,
            None,
            None,
        );

        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&caller_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(user_service.clone())
            .await
            .unwrap();
        cache_test_spec(SPEC_URL, Some(&caller_id), openapi_spec());

        let state = test_app_state(db);
        let response = service_openapi_json(
            State(state),
            test_auth_user(&caller_id),
            Path(user_service.id),
        )
        .await
        .unwrap();

        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CONTENT_TYPE)
                .map(|v| v.to_str().unwrap()),
            Some("application/json"),
        );
        let body_bytes = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let spec: serde_json::Value = serde_json::from_slice(&body_bytes).unwrap();

        assert_eq!(spec["openapi"], "3.1.0");
        assert!(spec["paths"]["/ping"]["get"].is_object());
    }

    #[tokio::test]
    async fn service_openapi_json_returns_not_found_when_custom_service_has_no_spec() {
        let Some(db) = connect_test_database("docs_openapi_no_spec").await else {
            eprintln!("skipping docs integration test: no local MongoDB available");
            return;
        };

        let caller_id = Uuid::new_v4().to_string();
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "Custom API",
            "https://custom.example.com",
            None,
            None,
        );
        let user_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &caller_id,
            "custom-api",
            &endpoint.id,
            None,
            None,
        );

        db.collection::<crate::models::user::User>(USERS)
            .insert_one(test_user(&caller_id, UserType::Person))
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(user_service.clone())
            .await
            .unwrap();

        let state = test_app_state(db);
        let err = service_openapi_json(
            State(state),
            test_auth_user(&caller_id),
            Path(user_service.id),
        )
        .await
        .expect_err("missing custom spec should 404");

        assert!(matches!(
            err,
            AppError::NotFound(message)
                if message == "Service has no documentation spec configured"
        ));
    }

    #[tokio::test]
    async fn service_openapi_json_hides_cross_user_custom_services() {
        let Some(db) = connect_test_database("docs_openapi_cross_user").await else {
            eprintln!("skipping docs integration test: no local MongoDB available");
            return;
        };

        let caller_id = Uuid::new_v4().to_string();
        let owner_id = Uuid::new_v4().to_string();
        let endpoint = test_user_endpoint(
            &Uuid::new_v4().to_string(),
            &owner_id,
            "Other API",
            "https://other.example.com",
            Some(SPEC_URL),
            None,
        );
        let user_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &owner_id,
            "other-api",
            &endpoint.id,
            None,
            None,
        );

        db.collection::<crate::models::user::User>(USERS)
            .insert_many([
                test_user(&caller_id, UserType::Person),
                test_user(&owner_id, UserType::Person),
            ])
            .await
            .unwrap();
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_one(endpoint)
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(user_service.clone())
            .await
            .unwrap();

        let state = test_app_state(db);
        let err = service_openapi_json(
            State(state),
            test_auth_user(&caller_id),
            Path(user_service.id),
        )
        .await
        .expect_err("cross-user service should be hidden");

        assert!(matches!(err, AppError::NotFound(message) if message == "Service not found"));
    }
}
