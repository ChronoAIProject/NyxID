use axum::{
    routing::{delete, get, patch, post, put},
    Router,
};

use crate::handlers;
use crate::AppState;

/// Build the complete application router with all route groups.
pub fn build_router() -> Router<AppState> {
    let mfa_routes = Router::new()
        .route("/setup", post(handlers::mfa::setup))
        .route("/confirm", post(handlers::mfa::confirm))
        .route("/verify", post(handlers::mfa::verify))
        .route("/disable", post(handlers::mfa::disable));

    let auth_routes = Router::new()
        .route("/register", post(handlers::auth::register))
        .route("/login", post(handlers::auth::login))
        .route("/logout", post(handlers::auth::logout))
        .route("/refresh", post(handlers::auth::refresh))
        .route("/verify-email", post(handlers::auth::verify_email))
        .route("/forgot-password", post(handlers::auth::forgot_password))
        .route("/reset-password", post(handlers::auth::reset_password))
        .route("/setup", post(handlers::auth::setup))
        .nest("/mfa", mfa_routes);

    let user_routes = Router::new()
        .route("/me", get(handlers::users::get_me))
        .route("/me", put(handlers::users::update_me));

    let api_key_routes = Router::new()
        .route("/", get(handlers::api_keys::list_keys))
        .route("/", post(handlers::api_keys::create_key))
        .route("/{key_id}", delete(handlers::api_keys::delete_key))
        .route("/{key_id}/rotate", post(handlers::api_keys::rotate_key));

    let service_routes = Router::new()
        .route("/", get(handlers::services::list_services))
        .route("/", post(handlers::services::create_service))
        .route("/{service_id}", get(handlers::services::get_service))
        .route("/{service_id}", put(handlers::services::update_service))
        .route("/{service_id}", delete(handlers::services::delete_service))
        .route("/{service_id}/oidc-credentials", get(handlers::services::get_oidc_credentials))
        .route("/{service_id}/redirect-uris", put(handlers::services::update_redirect_uris))
        .route("/{service_id}/regenerate-secret", post(handlers::services::regenerate_oidc_secret))
        .route("/{service_id}/endpoints", get(handlers::endpoints::list_endpoints))
        .route("/{service_id}/endpoints", post(handlers::endpoints::create_endpoint))
        .route("/{service_id}/endpoints/{endpoint_id}", put(handlers::endpoints::update_endpoint))
        .route("/{service_id}/endpoints/{endpoint_id}", delete(handlers::endpoints::delete_endpoint))
        .route("/{service_id}/discover-endpoints", post(handlers::endpoints::discover_endpoints))
        .route("/{service_id}/requirements", get(handlers::service_requirements::list_requirements))
        .route("/{service_id}/requirements", post(handlers::service_requirements::add_requirement))
        .route("/{service_id}/requirements/{requirement_id}", delete(handlers::service_requirements::remove_requirement));

    let session_routes = Router::new()
        .route("/", get(handlers::sessions::list_sessions));

    let mcp_routes = Router::new()
        .route("/config", get(handlers::mcp::get_mcp_config));

    let connection_routes = Router::new()
        .route("/", get(handlers::connections::list_connections))
        .route("/{service_id}", post(handlers::connections::connect_service))
        .route(
            "/{service_id}",
            delete(handlers::connections::disconnect_service),
        )
        .route(
            "/{service_id}/credential",
            put(handlers::connections::update_connection_credential),
        );

    let provider_routes = Router::new()
        .route("/", get(handlers::providers::list_providers))
        .route("/", post(handlers::providers::create_provider))
        .route("/my-tokens", get(handlers::user_tokens::list_my_tokens))
        .route(
            "/callback",
            get(handlers::user_tokens::generic_oauth_callback),
        )
        .route("/{provider_id}", get(handlers::providers::get_provider))
        .route("/{provider_id}", put(handlers::providers::update_provider))
        .route("/{provider_id}", delete(handlers::providers::delete_provider))
        .route(
            "/{provider_id}/connect/api-key",
            post(handlers::user_tokens::connect_api_key),
        )
        .route(
            "/{provider_id}/connect/oauth",
            get(handlers::user_tokens::initiate_oauth_connect),
        )
        .route(
            "/{provider_id}/callback",
            get(handlers::user_tokens::oauth_callback),
        )
        .route(
            "/{provider_id}/connect/device-code/initiate",
            post(handlers::user_tokens::request_device_code),
        )
        .route(
            "/{provider_id}/connect/device-code/poll",
            post(handlers::user_tokens::poll_device_code),
        )
        .route(
            "/{provider_id}/disconnect",
            delete(handlers::user_tokens::disconnect_provider),
        )
        .route(
            "/{provider_id}/refresh",
            post(handlers::user_tokens::manual_refresh),
        );

    let admin_routes = Router::new()
        .route("/users", get(handlers::admin::list_users)
            .post(handlers::admin::create_user))
        .route("/users/{user_id}", get(handlers::admin::get_user)
            .put(handlers::admin::update_user)
            .delete(handlers::admin::delete_user))
        .route("/users/{user_id}/role", patch(handlers::admin::set_user_role))
        .route("/users/{user_id}/status", patch(handlers::admin::set_user_status))
        .route("/users/{user_id}/reset-password", post(handlers::admin::force_password_reset))
        .route("/users/{user_id}/verify-email", patch(handlers::admin::verify_user_email))
        .route("/users/{user_id}/sessions", get(handlers::admin::list_user_sessions)
            .delete(handlers::admin::revoke_user_sessions))
        .route("/audit-log", get(handlers::admin::list_audit_log))
        .route("/oauth-clients", get(handlers::admin::list_oauth_clients)
            .post(handlers::admin::create_oauth_client))
        .route("/oauth-clients/{client_id}", delete(handlers::admin::delete_oauth_client));

    let oauth_routes = Router::new()
        .route("/authorize", get(handlers::oauth::authorize))
        .route("/token", post(handlers::oauth::token))
        .route("/userinfo", get(handlers::oauth::userinfo))
        .route("/register", post(handlers::oauth::register_client));

    let api_v1 = Router::new()
        .nest("/auth", auth_routes)
        .nest("/users", user_routes)
        .nest("/api-keys", api_key_routes)
        .nest("/services", service_routes)
        .nest("/sessions", session_routes)
        .nest("/connections", connection_routes)
        .nest("/providers", provider_routes)
        .nest("/mcp", mcp_routes)
        .nest("/admin", admin_routes)
        .route("/public/config", get(handlers::health::public_config))
        .route("/proxy/{service_id}/{*path}", axum::routing::any(handlers::proxy::proxy_request));

    let well_known_routes = Router::new()
        .route("/openid-configuration", get(handlers::oidc_discovery::openid_configuration))
        .route("/oauth-authorization-server", get(handlers::oidc_discovery::oauth_authorization_server_metadata))
        .route("/jwks.json", get(handlers::oidc_discovery::jwks))
        .route("/oauth-protected-resource", get(handlers::oidc_discovery::oauth_protected_resource));

    Router::new()
        .route("/health", get(handlers::health::health_check))
        .nest("/.well-known", well_known_routes)
        .nest("/oauth", oauth_routes)
        .nest("/api/v1", api_v1)
        // MCP StreamableHTTP endpoint (root level, not under /api/v1)
        .route(
            "/mcp",
            post(handlers::mcp_transport::mcp_post)
                .get(handlers::mcp_transport::mcp_get)
                .delete(handlers::mcp_transport::mcp_delete),
        )
}
