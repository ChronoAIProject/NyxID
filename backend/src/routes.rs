use axum::{
    Router,
    extract::DefaultBodyLimit,
    middleware,
    routing::{delete, get, patch, post, put},
};
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;

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
pub fn build_router(proxy_max_body_size: usize) -> (Router<AppState>, Router<AppState>) {
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
        .route("/cli-token", post(handlers::auth::cli_token))
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
        .route("/usage", get(handlers::api_keys::list_key_usage))
        .route(
            "/{key_id}",
            get(handlers::api_keys::get_key)
                .put(handlers::api_keys::update_key)
                .delete(handlers::api_keys::delete_key),
        )
        .route("/{key_id}/usage", get(handlers::api_keys::get_key_usage))
        .route("/{key_id}/rotate", post(handlers::api_keys::rotate_key))
        .route(
            "/{key_id}/bindings",
            get(handlers::agent_bindings::list_bindings)
                .post(handlers::agent_bindings::create_binding),
        )
        .route(
            "/{key_id}/bindings/{binding_id}",
            delete(handlers::agent_bindings::delete_binding),
        );

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
            get(handlers::user_tokens::generic_oauth_callback)
                .post(handlers::user_tokens::generic_oauth_callback_post),
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
            "/{provider_id}/connect/telegram",
            get(handlers::user_tokens::get_telegram_connect_config),
        )
        .route(
            "/{provider_id}/connect/telegram/callback",
            post(handlers::user_tokens::telegram_callback),
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
            "/roles/{role_id}/assign-bulk",
            post(handlers::admin_roles::bulk_assign_role),
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
        .route("/nodes", get(handlers::admin_nodes::admin_list_nodes))
        .route(
            "/nodes/{node_id}",
            get(handlers::admin_nodes::admin_get_node)
                .delete(handlers::admin_nodes::admin_delete_node),
        )
        .route(
            "/nodes/{node_id}/disconnect",
            post(handlers::admin_nodes::admin_disconnect_node),
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
        .nest("/service-accounts", sa_admin_routes)
        .nest("/invite-codes", {
            Router::new()
                .route("/", get(handlers::invite_codes::list_invite_codes))
                .route("/", post(handlers::invite_codes::create_invite_code))
                .route("/{id}", patch(handlers::invite_codes::update_invite_code))
                .route(
                    "/{id}",
                    delete(handlers::invite_codes::deactivate_invite_code),
                )
        });

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
        .route(
            "/requests",
            get(handlers::approvals::list_requests).post(handlers::approvals::create_request),
        )
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

    let node_routes = Router::new()
        .route(
            "/register-token",
            post(handlers::node_admin::create_registration_token),
        )
        .route("/", get(handlers::node_admin::list_nodes))
        .route(
            "/my-bindings",
            get(handlers::node_admin::list_my_bound_services),
        )
        .route("/{node_id}", get(handlers::node_admin::get_node))
        .route("/{node_id}", delete(handlers::node_admin::delete_node))
        .route(
            "/{node_id}/rotate-token",
            post(handlers::node_admin::rotate_token),
        )
        .route(
            "/{node_id}/bindings",
            get(handlers::node_admin::list_bindings).post(handlers::node_admin::create_binding),
        )
        .route(
            "/{node_id}/bindings/{binding_id}",
            patch(handlers::node_admin::update_binding)
                .delete(handlers::node_admin::delete_binding),
        );

    let unified_key_routes = Router::new()
        .route(
            "/",
            get(handlers::keys::list_keys).post(handlers::keys::create_key),
        )
        .route(
            "/{key_id}",
            get(handlers::keys::get_key)
                .put(handlers::keys::update_key)
                .delete(handlers::keys::delete_key),
        );

    let user_endpoint_routes = Router::new()
        .route("/", get(handlers::user_endpoints::list_endpoints))
        .route(
            "/{endpoint_id}",
            put(handlers::user_endpoints::update_endpoint)
                .delete(handlers::user_endpoints::delete_endpoint),
        )
        .route(
            "/{endpoint_id}/openapi-endpoints",
            get(handlers::user_endpoints::list_openapi_endpoints),
        );

    let external_api_key_routes = Router::new()
        .route(
            "/",
            get(handlers::user_api_keys_external::list_external_api_keys),
        )
        .route(
            "/{key_id}",
            put(handlers::user_api_keys_external::update_external_api_key)
                .delete(handlers::user_api_keys_external::delete_external_api_key),
        );

    let user_service_routes = Router::new()
        .route(
            "/",
            get(handlers::user_services_handler::list_user_services),
        )
        .route(
            "/{service_id}",
            put(handlers::user_services_handler::update_user_service)
                .delete(handlers::user_services_handler::delete_user_service),
        );

    // Org management routes (creation, members, invites). All routes
    // authenticate as a regular session/user; admin-vs-member checks happen
    // inside the handlers based on org_memberships rather than a global flag.
    let org_routes = Router::new()
        .route(
            "/",
            get(handlers::orgs::list_orgs).post(handlers::orgs::create_org),
        )
        .route("/join/{nonce}", post(handlers::orgs::redeem_invite))
        .route(
            "/{org_id}",
            get(handlers::orgs::get_org)
                .patch(handlers::orgs::update_org)
                .delete(handlers::orgs::delete_org),
        )
        .route(
            "/{org_id}/members",
            get(handlers::orgs::list_members).post(handlers::orgs::add_member),
        )
        .route(
            "/{org_id}/members/{member_id}",
            patch(handlers::orgs::update_member).delete(handlers::orgs::remove_member),
        )
        .route(
            "/{org_id}/role-scopes",
            get(handlers::org_role_scopes::list_role_scopes),
        )
        .route(
            "/{org_id}/role-scopes/{role}",
            put(handlers::org_role_scopes::set_role_scope)
                .delete(handlers::org_role_scopes::clear_role_scope),
        )
        .route(
            "/{org_id}/invites",
            get(handlers::orgs::list_invites).post(handlers::orgs::create_invite),
        )
        .route(
            "/{org_id}/invites/{invite_id}",
            delete(handlers::orgs::cancel_invite),
        );

    let catalog_routes = Router::new()
        .route("/", get(handlers::catalog::list_catalog))
        .route("/{slug}", get(handlers::catalog::get_catalog_entry))
        .route(
            "/{slug}/endpoints",
            get(handlers::catalog::list_catalog_endpoints),
        );

    let cli_pairing_routes = Router::new()
        .route("/", post(handlers::cli_pairings::create_pairing))
        .route("/claim", post(handlers::cli_pairings::claim_pairing))
        .route("/{id}/poll", get(handlers::cli_pairings::poll_pairing))
        .route(
            "/{id}/reserve-action",
            post(handlers::cli_pairings::reserve_action),
        )
        .route(
            "/{id}/rewind-action",
            post(handlers::cli_pairings::rewind_action),
        )
        .route(
            "/{id}/complete",
            post(handlers::cli_pairings::complete_pairing),
        )
        .route("/{id}/cancel", post(handlers::cli_pairings::cancel_pairing));

    let channel_bot_routes = Router::new()
        .route(
            "/",
            get(handlers::channel_bots::list_bots).post(handlers::channel_bots::create_bot),
        )
        .route(
            "/{id}",
            get(handlers::channel_bots::get_bot)
                .patch(handlers::channel_bots::update_bot)
                .delete(handlers::channel_bots::delete_bot),
        )
        .route("/{id}/verify", post(handlers::channel_bots::verify_bot));

    let channel_conversation_routes = Router::new()
        .route(
            "/",
            get(handlers::channel_conversations::list_conversations)
                .post(handlers::channel_conversations::create_conversation),
        )
        .route(
            "/{id}",
            get(handlers::channel_conversations::get_conversation)
                .put(handlers::channel_conversations::update_conversation)
                .delete(handlers::channel_conversations::delete_conversation),
        )
        .route(
            "/{id}/messages",
            get(handlers::channel_conversations::list_conversation_messages),
        );

    let channel_relay_routes = Router::new()
        .route("/reply", post(handlers::channel_relay::async_reply))
        .route("/reply/update", post(handlers::channel_relay::update_reply))
        .route(
            "/messages/{conversation_id}",
            get(handlers::channel_relay::list_messages),
        )
        .route(
            "/resolve-sender",
            get(handlers::channel_relay::resolve_sender),
        );

    // HTTP Event Gateway — device event ingress (NyxID#221, ADR-013).
    // Authenticated via API key; rate-limited per conversation.
    //
    // `DefaultBodyLimit::disable()` opts this router out of the app-wide
    // 1 MiB body cap set in `main.rs`. Per the plan §8.1 and the gateway
    // design doc §NOT in Scope, NyxID deliberately does not enforce an
    // application-level payload size limit on device events — analyzers
    // that ship larger JSON blobs or embedded metadata must be accepted.
    let channel_event_routes = Router::new()
        .route(
            "/{conversation_id}",
            post(handlers::channel_events::post_event),
        )
        .layer(DefaultBodyLimit::disable());

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

    // Proxy pass-through routes allow larger request bodies than the rest of the API.
    // Use RequestBodyLimitLayer so manual Request<Body> handlers are also protected.
    let proxy_passthrough_routes = Router::new()
        .route(
            "/proxy/s/{slug}/{*path}",
            axum::routing::any(handlers::proxy::proxy_request_by_slug),
        )
        .route(
            "/proxy/s/{slug}",
            axum::routing::any(handlers::proxy::proxy_request_by_slug_root),
        )
        .route(
            "/proxy/{service_id}/{*path}",
            axum::routing::any(handlers::proxy::proxy_request),
        )
        .route(
            "/proxy/{service_id}",
            axum::routing::any(handlers::proxy::proxy_request_root),
        )
        .layer(RequestBodyLimitLayer::new(proxy_max_body_size));

    // LLM gateway routes get a moderate limit (10 MB for LLM payloads).
    let llm_routes = llm_routes.layer(RequestBodyLimitLayer::new(10 * 1024 * 1024));

    // Routes that ALLOW delegated tokens (proxy, LLM gateway, delegation refresh)
    // Also accessible by service accounts.
    let api_v1_delegated = Router::new()
        .nest("/llm", llm_routes)
        .nest("/delegation", delegation_routes)
        .route(
            "/approvals/requests/{request_id}/status",
            get(handlers::approvals::get_request_status),
        )
        .route(
            "/proxy/services/{service_id}/docs",
            get(handlers::docs::service_docs_ui),
        )
        .route(
            "/proxy/services/{service_id}/openapi.json",
            get(handlers::docs::service_openapi_json),
        )
        .route(
            "/proxy/services/{service_id}/asyncapi.json",
            get(handlers::docs::service_asyncapi_json),
        )
        .route("/proxy/services", get(handlers::proxy::list_proxy_services))
        .nest("/channel-relay", channel_relay_routes)
        .nest("/channel-events", channel_event_routes)
        .merge(proxy_passthrough_routes);

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
        .route("/docs", get(handlers::docs::docs_ui))
        .route("/docs/catalog", get(handlers::docs::catalog_ui))
        .route("/docs/openapi.json", get(handlers::docs::openapi_json))
        .route("/docs/asyncapi.json", get(handlers::docs::asyncapi_json))
        .route(
            "/ssh/{service_id}/certificate",
            post(handlers::ssh_tunnel::issue_ssh_certificate),
        )
        .route(
            "/ssh/{service_id}",
            get(handlers::ssh_tunnel::ssh_tunnel_ws),
        )
        .route("/ssh/{service_id}/exec", post(handlers::ssh_exec::ssh_exec))
        .route(
            "/ssh/{service_id}/terminal",
            get(handlers::ssh_web_terminal::ssh_web_terminal),
        )
        .nest("/sessions", session_routes)
        .nest("/mcp", mcp_routes)
        .nest("/developer", developer_routes)
        .nest("/admin", admin_routes)
        .nest("/notifications", notification_routes)
        .nest("/approvals", approval_routes)
        .nest("/nodes", node_routes)
        .nest("/keys", unified_key_routes)
        .nest("/endpoints", user_endpoint_routes)
        .nest("/api-keys/external", external_api_key_routes)
        .nest("/user-services", user_service_routes)
        .nest("/orgs", org_routes)
        .route(
            "/users/me/primary-org",
            patch(handlers::orgs::set_primary_org),
        )
        .nest("/catalog", catalog_routes)
        .nest("/cli-pairings", cli_pairing_routes)
        .nest("/channel-bots", channel_bot_routes)
        .nest("/channel-conversations", channel_conversation_routes)
        .route(
            "/integrations/openclaw/mappings",
            post(handlers::openclaw_channel::create_mapping),
        )
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

    // Integration webhook routes -- unauthenticated (verified by HMAC signature)
    let integration_routes = Router::new().route(
        "/openclaw/channel",
        post(handlers::openclaw_channel::handle_channel_message),
    );

    let private = Router::new()
        .route("/health", get(handlers::health::health_check))
        .route("/llms.txt", get(handlers::llms_txt::llms_txt))
        .route("/llms-full.txt", get(handlers::llms_txt::llms_full_txt))
        .nest("/api/v1/webhooks", webhook_routes)
        // Channel bot webhook routes -- unauthenticated (per-bot signature verified)
        .route(
            "/api/v1/webhooks/channel/telegram/{bot_id}",
            post(handlers::channel_webhooks::telegram_webhook),
        )
        .route(
            "/api/v1/webhooks/channel/discord/{bot_id}",
            post(handlers::channel_webhooks::discord_webhook),
        )
        .route(
            "/api/v1/webhooks/channel/lark/{bot_id}",
            post(handlers::channel_webhooks::lark_webhook),
        )
        .route(
            "/api/v1/webhooks/channel/feishu/{bot_id}",
            post(handlers::channel_webhooks::feishu_webhook),
        )
        .route(
            "/api/v1/webhooks/channel/slack/{bot_id}",
            post(handlers::channel_webhooks::slack_webhook),
        )
        .nest("/api/v1/integrations", integration_routes)
        .nest("/api/v1", api_v1)
        // WebSocket endpoint for node agents. Auth happens in-message (not middleware).
        // Rate limiting: global per-IP rate limiter covers HTTP upgrade requests.
        // Connection limiting: NodeWsManager enforces max concurrent connections.
        .route("/api/v1/nodes/ws", get(handlers::node_ws::ws_handler))
        .route(
            "/mcp",
            post(handlers::mcp_transport::mcp_post)
                .get(handlers::mcp_transport::mcp_get)
                .delete(handlers::mcp_transport::mcp_delete),
        );

    (public_oauth, private)
}
