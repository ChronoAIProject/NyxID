use axum::{
    Router, middleware,
    routing::{delete, get, patch, post, put},
};
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};

use crate::AppState;
use crate::handlers;
use crate::mw::auth::{reject_delegated_tokens, reject_service_account_tokens};

/// Per RFC 9207 / OAuth 2.0 for Browser-Based Apps, the token endpoint,
/// userinfo endpoint, and discovery documents MUST be accessible from any
/// origin so that public SPA clients can call them directly.
fn oauth_public_cors() -> CorsLayer {
    CorsLayer::new()
        .allow_origin(AllowOrigin::any())
        .allow_methods(AllowMethods::list([
            axum::http::Method::GET,
            axum::http::Method::POST,
            axum::http::Method::OPTIONS,
        ]))
        .allow_headers(AllowHeaders::list([
            axum::http::header::CONTENT_TYPE,
            axum::http::header::AUTHORIZATION,
            axum::http::header::ACCEPT,
        ]))
}

/// Build two routers: (public OAuth / .well-known, private API).
///
/// The caller must attach separate CORS layers to each before merging.
/// Public OAuth endpoints allow any origin (per RFC 9207) while private
/// API endpoints restrict origin to FRONTEND_URL.
pub fn build_router() -> (Router<AppState>, Router<AppState>) {
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
        .route("/social/{provider}", get(handlers::social_auth::authorize))
        .route(
            "/social/apple/callback",
            post(handlers::social_auth::apple_callback),
        )
        .route(
            "/social/{provider}/callback",
            get(handlers::social_auth::callback),
        )
        .nest("/mfa", mfa_routes);

    let user_routes = Router::new()
        .route("/me", get(handlers::users::get_me))
        .route("/me", put(handlers::users::update_me))
        .route("/me", delete(handlers::users::delete_me))
        .route("/me/consents", get(handlers::consent::list_my_consents))
        .route(
            "/me/consents/{client_id}",
            delete(handlers::consent::revoke_my_consent),
        );

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
        .route(
            "/{service_id}/oidc-credentials",
            get(handlers::services::get_oidc_credentials),
        )
        .route(
            "/{service_id}/redirect-uris",
            put(handlers::services::update_redirect_uris),
        )
        .route(
            "/{service_id}/regenerate-secret",
            post(handlers::services::regenerate_oidc_secret),
        )
        .route(
            "/{service_id}/endpoints",
            get(handlers::endpoints::list_endpoints),
        )
        .route(
            "/{service_id}/endpoints",
            post(handlers::endpoints::create_endpoint),
        )
        .route(
            "/{service_id}/endpoints/{endpoint_id}",
            put(handlers::endpoints::update_endpoint),
        )
        .route(
            "/{service_id}/endpoints/{endpoint_id}",
            delete(handlers::endpoints::delete_endpoint),
        )
        .route(
            "/{service_id}/discover-endpoints",
            post(handlers::endpoints::discover_endpoints),
        )
        .route(
            "/{service_id}/requirements",
            get(handlers::service_requirements::list_requirements),
        )
        .route(
            "/{service_id}/requirements",
            post(handlers::service_requirements::add_requirement),
        )
        .route(
            "/{service_id}/requirements/{requirement_id}",
            delete(handlers::service_requirements::remove_requirement),
        );

    let session_routes = Router::new().route("/", get(handlers::sessions::list_sessions));

    let mcp_routes = Router::new().route("/config", get(handlers::mcp::get_mcp_config));

    let connection_routes = Router::new()
        .route("/", get(handlers::connections::list_connections))
        .route(
            "/{service_id}",
            post(handlers::connections::connect_service),
        )
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
        .route(
            "/{provider_id}",
            delete(handlers::providers::delete_provider),
        )
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
        )
        .route(
            "/{provider_id}/credentials",
            get(handlers::user_credentials::get_my_credentials)
                .put(handlers::user_credentials::set_my_credentials)
                .delete(handlers::user_credentials::delete_my_credentials),
        );

    // TODO(M-7): LLM endpoints share the global rate limiter. Consider adding a
    // dedicated, more restrictive per-user rate limiter for LLM routes (e.g., 5
    // req/s per user) to prevent API quota burn and separate LLM traffic from
    // lightweight auth requests.
    let llm_routes = Router::new()
        .route("/status", get(handlers::llm_gateway::llm_status))
        .route(
            "/gateway/v1/{*path}",
            axum::routing::any(handlers::llm_gateway::gateway_request),
        )
        .route(
            "/{provider_slug}/v1/{*path}",
            axum::routing::any(handlers::llm_gateway::llm_proxy_request),
        );

    let sa_admin_routes = Router::new()
        .route(
            "/",
            get(handlers::admin_service_accounts::list_service_accounts)
                .post(handlers::admin_service_accounts::create_service_account),
        )
        .route(
            "/{sa_id}",
            get(handlers::admin_service_accounts::get_service_account)
                .put(handlers::admin_service_accounts::update_service_account)
                .delete(handlers::admin_service_accounts::delete_service_account),
        )
        .route(
            "/{sa_id}/rotate-secret",
            post(handlers::admin_service_accounts::rotate_secret),
        )
        .route(
            "/{sa_id}/revoke-tokens",
            post(handlers::admin_service_accounts::revoke_tokens),
        )
        .route(
            "/{sa_id}/providers",
            get(handlers::admin_sa_providers::list_sa_providers),
        )
        .route(
            "/{sa_id}/providers/{provider_id}/connect/api-key",
            post(handlers::admin_sa_providers::connect_api_key_for_sa),
        )
        .route(
            "/{sa_id}/providers/{provider_id}/connect/oauth",
            get(handlers::admin_sa_providers::initiate_oauth_for_sa),
        )
        .route(
            "/{sa_id}/providers/{provider_id}/connect/device-code/initiate",
            post(handlers::admin_sa_providers::initiate_device_code_for_sa),
        )
        .route(
            "/{sa_id}/providers/{provider_id}/connect/device-code/poll",
            post(handlers::admin_sa_providers::poll_device_code_for_sa),
        )
        .route(
            "/{sa_id}/providers/{provider_id}/disconnect",
            delete(handlers::admin_sa_providers::disconnect_sa_provider),
        )
        .route(
            "/{sa_id}/connections",
            get(handlers::admin_sa_connections::list_sa_connections),
        )
        .route(
            "/{sa_id}/connections/{service_id}",
            post(handlers::admin_sa_connections::connect_sa_service)
                .delete(handlers::admin_sa_connections::disconnect_sa_service),
        )
        .route(
            "/{sa_id}/connections/{service_id}/credential",
            put(handlers::admin_sa_connections::update_sa_connection_credential),
        );

    let admin_routes = Router::new()
        .route(
            "/users",
            get(handlers::admin::list_users).post(handlers::admin::create_user),
        )
        .route(
            "/users/{user_id}",
            get(handlers::admin::get_user)
                .put(handlers::admin::update_user)
                .delete(handlers::admin::delete_user),
        )
        .route(
            "/users/{user_id}/role",
            patch(handlers::admin::set_user_role),
        )
        .route(
            "/users/{user_id}/status",
            patch(handlers::admin::set_user_status),
        )
        .route(
            "/users/{user_id}/reset-password",
            post(handlers::admin::force_password_reset),
        )
        .route(
            "/users/{user_id}/verify-email",
            patch(handlers::admin::verify_user_email),
        )
        .route(
            "/users/{user_id}/sessions",
            get(handlers::admin::list_user_sessions).delete(handlers::admin::revoke_user_sessions),
        )
        .route(
            "/users/{user_id}/roles",
            get(handlers::admin_roles::get_user_roles),
        )
        .route(
            "/users/{user_id}/roles/{role_id}",
            post(handlers::admin_roles::assign_role).delete(handlers::admin_roles::revoke_role),
        )
        .route(
            "/users/{user_id}/groups",
            get(handlers::admin_groups::get_user_groups),
        )
        .route(
            "/roles",
            get(handlers::admin_roles::list_roles).post(handlers::admin_roles::create_role),
        )
        .route(
            "/roles/{role_id}",
            get(handlers::admin_roles::get_role)
                .put(handlers::admin_roles::update_role)
                .delete(handlers::admin_roles::delete_role),
        )
        .route(
            "/groups",
            get(handlers::admin_groups::list_groups).post(handlers::admin_groups::create_group),
        )
        .route(
            "/groups/{group_id}",
            get(handlers::admin_groups::get_group)
                .put(handlers::admin_groups::update_group)
                .delete(handlers::admin_groups::delete_group),
        )
        .route(
            "/groups/{group_id}/members",
            get(handlers::admin_groups::get_members),
        )
        .route(
            "/groups/{group_id}/members/{user_id}",
            post(handlers::admin_groups::add_member).delete(handlers::admin_groups::remove_member),
        )
        .route("/audit-log", get(handlers::admin::list_audit_log))
        .route(
            "/oauth-clients",
            get(handlers::admin::list_oauth_clients).post(handlers::admin::create_oauth_client),
        )
        .route(
            "/oauth-clients/{client_id}",
            delete(handlers::admin::delete_oauth_client),
        )
        .route(
            "/oauth-clients/{client_id}/consents",
            get(handlers::admin::list_client_consents),
        )
        .nest("/service-accounts", sa_admin_routes);

    let oauth_routes = Router::new()
        .route("/authorize", get(handlers::oauth::authorize))
        .route(
            "/authorize/decision",
            post(handlers::oauth::authorize_decision),
        )
        .route("/token", post(handlers::oauth::token))
        .route(
            "/userinfo",
            get(handlers::oauth::userinfo).post(handlers::oauth::userinfo),
        )
        .route("/register", post(handlers::oauth::register_client))
        .route("/introspect", post(handlers::oauth::introspect))
        .route("/revoke", post(handlers::oauth::revoke));

    let delegation_routes = Router::new().route(
        "/refresh",
        post(handlers::delegation::refresh_delegation_token),
    );

    // Notification settings (human-only)
    let notification_routes = Router::new()
        .route(
            "/settings",
            get(handlers::notifications::get_settings)
                .put(handlers::notifications::update_settings),
        )
        .route(
            "/telegram/link",
            post(handlers::notifications::telegram_link),
        )
        .route(
            "/telegram",
            delete(handlers::notifications::telegram_disconnect),
        )
        // Device token management for push notifications
        .route(
            "/devices",
            get(handlers::device_tokens::list_devices)
                .post(handlers::device_tokens::register_device),
        )
        .route(
            "/devices/current",
            delete(handlers::device_tokens::remove_current_device),
        )
        .route(
            "/devices/{device_id}",
            delete(handlers::device_tokens::remove_device),
        );

    // Approval management (human-only; status polling is in api_v1_delegated)
    let approval_routes = Router::new()
        .route("/requests", get(handlers::approvals::list_requests))
        .route(
            "/requests/{request_id}",
            get(handlers::approvals::get_request_by_id),
        )
        .route(
            "/requests/{request_id}/decide",
            post(handlers::approvals::decide_request),
        )
        .route("/grants", get(handlers::approvals::list_grants))
        .route(
            "/grants/{grant_id}",
            delete(handlers::approvals::revoke_grant),
        )
        .route(
            "/service-configs",
            get(handlers::approvals::list_service_configs),
        )
        .route(
            "/service-configs/{service_id}",
            put(handlers::approvals::set_service_config)
                .delete(handlers::approvals::delete_service_config),
        );

    let developer_routes = Router::new()
        .route(
            "/oauth-clients",
            get(handlers::developer_apps::list_my_oauth_clients)
                .post(handlers::developer_apps::create_my_oauth_client),
        )
        .route(
            "/oauth-clients/{client_id}",
            get(handlers::developer_apps::get_my_oauth_client)
                .patch(handlers::developer_apps::update_my_oauth_client)
                .delete(handlers::developer_apps::delete_my_oauth_client),
        )
        .route(
            "/oauth-clients/{client_id}/rotate-secret",
            post(handlers::developer_apps::rotate_my_oauth_client_secret),
        );

    // Routes that ALLOW delegated tokens (proxy, LLM gateway, delegation refresh)
    // Also accessible by service accounts.
    let api_v1_delegated = Router::new()
        .nest("/llm", llm_routes)
        .nest("/delegation", delegation_routes)
        .route(
            "/approvals/requests/{request_id}/status",
            get(handlers::approvals::get_request_status),
        )
        .route("/proxy/services", get(handlers::proxy::list_proxy_services))
        .route(
            "/proxy/s/{slug}/{*path}",
            axum::routing::any(handlers::proxy::proxy_request_by_slug),
        )
        .route(
            "/proxy/{service_id}/{*path}",
            axum::routing::any(handlers::proxy::proxy_request),
        );

    // Routes accessible by both users and service accounts (block delegated tokens)
    let api_v1_shared = Router::new()
        .nest("/connections", connection_routes)
        .nest("/providers", provider_routes)
        .layer(middleware::from_fn(reject_delegated_tokens));

    // Routes that BLOCK service account tokens (human-only endpoints)
    let api_v1_human_only = Router::new()
        .nest("/auth", auth_routes)
        .nest("/users", user_routes)
        .nest("/api-keys", api_key_routes)
        .nest("/services", service_routes)
        .nest("/sessions", session_routes)
        .nest("/mcp", mcp_routes)
        .nest("/developer", developer_routes)
        .nest("/admin", admin_routes)
        .nest("/notifications", notification_routes)
        .nest("/approvals", approval_routes)
        .route("/public/config", get(handlers::health::public_config))
        .layer(middleware::from_fn(reject_delegated_tokens))
        .layer(middleware::from_fn(reject_service_account_tokens));

    let api_v1 = api_v1_delegated
        .merge(api_v1_shared)
        .merge(api_v1_human_only);

    let well_known_routes = Router::new()
        .route(
            "/openid-configuration",
            get(handlers::oidc_discovery::openid_configuration),
        )
        .route(
            "/oauth-authorization-server",
            get(handlers::oidc_discovery::oauth_authorization_server_metadata),
        )
        .route("/jwks.json", get(handlers::oidc_discovery::jwks))
        .route(
            "/oauth-protected-resource",
            get(handlers::oidc_discovery::oauth_protected_resource),
        );

    let public_oauth = Router::new()
        .nest("/.well-known", well_known_routes)
        .nest("/oauth", oauth_routes)
        .layer(oauth_public_cors());

    // Webhook routes -- unauthenticated (verified by secret token)
    let webhook_routes =
        Router::new().route("/telegram", post(handlers::webhooks::telegram_webhook));

    let private = Router::new()
        .route("/health", get(handlers::health::health_check))
        .nest("/api/v1/webhooks", webhook_routes)
        .nest("/api/v1", api_v1)
        .route(
            "/mcp",
            post(handlers::mcp_transport::mcp_post)
                .get(handlers::mcp_transport::mcp_get)
                .delete(handlers::mcp_transport::mcp_delete),
        );

    (public_oauth, private)
}
