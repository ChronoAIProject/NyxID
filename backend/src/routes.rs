use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::HeaderName,
    middleware,
    routing::{delete, get, patch, post, put},
};
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};
use tower_http::limit::RequestBodyLimitLayer;

use crate::AppState;
use crate::handlers;
use crate::mw::auth::{
    reject_api_key_tokens, reject_delegated_tokens, reject_relay_tokens,
    reject_service_account_tokens,
};

macro_rules! register_billing_routes {
    ($router:expr; $(($path:literal, $public_path:literal, $handler:literal, $method_router:expr, $policy:expr)),+ $(,)?) => {
        $router$(.route($path, $method_router.layer(axum::Extension($policy))))+
    };
}

#[cfg(test)]
macro_rules! collect_billing_route_specs {
    ($router:expr; $(($path:literal, $public_path:literal, $handler:literal, $method_router:expr, $policy:expr)),+ $(,)?) => {{
        let _ = $router;
        vec![$(
            crate::services::billing::route_inventory::BillingRouteSpec {
                handler: $handler,
                route: $public_path,
                policy: $policy,
            }
        ),+]
    }};
}

macro_rules! llm_billing_routes {
    ($apply:ident, $router:expr) => {
        $apply!($router;
            (
                "/gateway/v1/{*path}",
                "/api/v1/llm/gateway/v1/{*path}",
                "handlers::llm_gateway::gateway_request",
                axum::routing::any(handlers::llm_gateway::gateway_request),
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::LlmGateway
                )
            ),
            (
                "/{provider_slug}/v1/{*path}",
                "/api/v1/llm/{provider_slug}/v1/{*path}",
                "handlers::llm_gateway::llm_proxy_request",
                axum::routing::any(handlers::llm_gateway::llm_proxy_request),
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::LlmProvider
                )
            ),
            (
                "/status",
                "/api/v1/llm/status",
                "handlers::llm_gateway::llm_status",
                get(handlers::llm_gateway::llm_status),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "control-plane discovery; no downstream request"
                )
            ),
        )
    };
}

macro_rules! proxy_billing_routes {
    ($apply:ident, $router:expr) => {
        $apply!($router;
            (
                "/proxy/s/{slug}/{*path}",
                "/api/v1/proxy/s/{slug}/{*path}",
                "handlers::proxy::proxy_request_by_slug",
                axum::routing::any(handlers::proxy::proxy_request_by_slug),
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Proxy
                )
            ),
            (
                "/proxy/s/{slug}",
                "/api/v1/proxy/s/{slug}",
                "handlers::proxy::proxy_request_by_slug_root",
                axum::routing::any(handlers::proxy::proxy_request_by_slug_root),
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Proxy
                )
            ),
            (
                "/proxy/{service_id}/{*path}",
                "/api/v1/proxy/{service_id}/{*path}",
                "handlers::proxy::proxy_request",
                axum::routing::any(handlers::proxy::proxy_request),
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Proxy
                )
            ),
            (
                "/proxy/{service_id}",
                "/api/v1/proxy/{service_id}",
                "handlers::proxy::proxy_request_root",
                axum::routing::any(handlers::proxy::proxy_request_root),
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Proxy
                )
            ),
            (
                "/proxy/services",
                "/api/v1/proxy/services",
                "handlers::proxy::list_proxy_services",
                get(handlers::proxy::list_proxy_services),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "control-plane discovery; no downstream request"
                )
            ),
        )
    };
}

macro_rules! ssh_billing_routes {
    ($apply:ident, $router:expr) => {
        $apply!($router;
            (
                "/ssh/{service_id}/exec",
                "/api/v1/ssh/{service_id}/exec",
                "handlers::ssh_exec::ssh_exec",
                post(handlers::ssh_exec::ssh_exec),
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::SshExec
                )
            ),
            (
                "/ssh/{service_id}",
                "/api/v1/ssh/{service_id}",
                "handlers::ssh_tunnel::ssh_tunnel_ws",
                get(handlers::ssh_tunnel::ssh_tunnel_ws),
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::SshTunnel
                )
            ),
            (
                "/ssh/{service_id}/terminal",
                "/api/v1/ssh/{service_id}/terminal",
                "handlers::ssh_web_terminal::ssh_web_terminal",
                get(handlers::ssh_web_terminal::ssh_web_terminal),
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::SshWebTerminal
                )
            ),
            (
                "/ssh/{service_id}/certificate",
                "/api/v1/ssh/{service_id}/certificate",
                "handlers::ssh_tunnel::issue_ssh_certificate",
                post(handlers::ssh_tunnel::issue_ssh_certificate),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "credential control plane; no downstream SSH session"
                )
            ),
        )
    };
}

macro_rules! mcp_billing_routes {
    ($apply:ident, $router:expr) => {
        $apply!($router;
            (
                "/mcp",
                "/mcp (POST)",
                "handlers::mcp_transport::mcp_post",
                post(handlers::mcp_transport::mcp_post),
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Mcp
                )
            ),
            (
                "/mcp",
                "/mcp (GET)",
                "handlers::mcp_transport::mcp_get",
                get(handlers::mcp_transport::mcp_get),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "session transport control; tool dispatch meters POST"
                )
            ),
            (
                "/mcp",
                "/mcp (DELETE)",
                "handlers::mcp_transport::mcp_delete",
                delete(handlers::mcp_transport::mcp_delete),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "session teardown; no downstream request"
                )
            ),
        )
    };
}

macro_rules! public_proxy_billing_routes {
    ($apply:ident, $router:expr) => {
        $apply!($router;
            (
                "/public/s/{slug}/{*path}",
                "/public/s/{slug}/{*path}",
                "handlers::public_proxy::public_proxy_request",
                axum::routing::any(handlers::public_proxy::public_proxy_request),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "block-not-meter; billable anonymous services are invalid"
                )
            ),
            (
                "/public/s/{slug}",
                "/public/s/{slug}",
                "handlers::public_proxy::public_proxy_request_root",
                axum::routing::any(handlers::public_proxy::public_proxy_request_root),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "block-not-meter; billable anonymous services are invalid"
                )
            ),
        )
    };
}

macro_rules! public_mcp_billing_routes {
    ($apply:ident, $router:expr) => {
        $apply!($router;
            (
                "/public/mcp",
                "/public/mcp",
                "handlers::public_mcp::public_mcp_post",
                post(handlers::public_mcp::public_mcp_post),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "discovery-only; public tools/call never forwards"
                )
            ),
        )
    };
}

macro_rules! oracle_billing_routes {
    ($apply:ident, $router:expr) => {
        $apply!($router;
            (
                "/pools/{id_or_slug}/tasks",
                "/api/v1/oracle/pools/{id_or_slug}/tasks",
                "handlers::oracle_tasks::submit_task",
                post(handlers::oracle_tasks::submit_task),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "user-supplied browser worker capacity"
                )
            ),
            (
                "/pools/{id_or_slug}/attach",
                "/api/v1/oracle/pools/{id_or_slug}/attach",
                "handlers::oracle_tasks::attach_conversation",
                post(handlers::oracle_tasks::attach_conversation),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "user-supplied browser worker capacity"
                )
            ),
            (
                "/pools/{id_or_slug}/extract",
                "/api/v1/oracle/pools/{id_or_slug}/extract",
                "handlers::oracle_tasks::extract_url",
                post(handlers::oracle_tasks::extract_url),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "user-supplied browser worker capacity"
                )
            ),
            (
                "/pools/{id_or_slug}/status",
                "/api/v1/oracle/pools/{id_or_slug}/status",
                "handlers::oracle_tasks::pool_status",
                get(handlers::oracle_tasks::pool_status),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "relay control plane; no downstream dispatch"
                )
            ),
            (
                "/tasks/{task_id}",
                "/api/v1/oracle/tasks/{task_id}",
                "handlers::oracle_tasks::get_task",
                get(handlers::oracle_tasks::get_task),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "relay control plane; no downstream dispatch"
                )
            ),
            (
                "/tasks/{task_id}/cancel",
                "/api/v1/oracle/tasks/{task_id}/cancel",
                "handlers::oracle_tasks::cancel_task",
                post(handlers::oracle_tasks::cancel_task),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "relay control plane; no downstream dispatch"
                )
            ),
            (
                "/sessions",
                "/api/v1/oracle/sessions",
                "handlers::oracle_tasks::list_sessions",
                get(handlers::oracle_tasks::list_sessions),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "relay control plane; no downstream dispatch"
                )
            ),
            (
                "/sessions/{conversation_id}",
                "/api/v1/oracle/sessions/{conversation_id}",
                "handlers::oracle_tasks::get_session",
                get(handlers::oracle_tasks::get_session),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "relay control plane; no downstream dispatch"
                )
            ),
            (
                "/sessions/{conversation_id}/close",
                "/api/v1/oracle/sessions/{conversation_id}/close",
                "handlers::oracle_tasks::close_session",
                post(handlers::oracle_tasks::close_session),
                crate::services::billing::route_inventory::BillingRoutePolicy::Exempt(
                    "relay control plane; no downstream dispatch"
                )
            ),
        )
    };
}

#[cfg(test)]
pub(crate) fn mounted_billing_route_inventory()
-> Vec<crate::services::billing::route_inventory::BillingRouteSpec> {
    let mut routes = llm_billing_routes!(collect_billing_route_specs, ());
    routes.extend(proxy_billing_routes!(collect_billing_route_specs, ()));
    routes.extend(ssh_billing_routes!(collect_billing_route_specs, ()));
    routes.extend(mcp_billing_routes!(collect_billing_route_specs, ()));
    routes.extend(public_proxy_billing_routes!(
        collect_billing_route_specs,
        ()
    ));
    routes.extend(public_mcp_billing_routes!(collect_billing_route_specs, ()));
    routes.extend(oracle_billing_routes!(collect_billing_route_specs, ()));
    routes
}

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
            HeaderName::from_static("dpop"),
        ]))
}

/// Build two routers: (public OAuth / .well-known, private API).
///
/// The caller must attach separate CORS layers to each before merging.
/// Public OAuth endpoints allow any origin (per RFC 9207) while private
/// API endpoints restrict origin to FRONTEND_URL.
pub fn build_router(
    proxy_max_body_size: usize,
    public_proxy_max_body_size: usize,
) -> (Router<AppState>, Router<AppState>) {
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
        .route(
            "/me/onboarding/complete",
            post(handlers::users::complete_onboarding),
        )
        .route(
            "/me/broker-bindings",
            get(handlers::broker_bindings::list_my_broker_bindings),
        )
        .route(
            "/me/broker-bindings/{binding_hash}",
            delete(handlers::broker_bindings::revoke_my_broker_binding),
        )
        .route("/me/consents", get(handlers::consent::list_my_consents))
        .route(
            "/me/consents/{client_id}",
            delete(handlers::consent::revoke_my_consent),
        );

    let api_key_routes = Router::new()
        .route("/", get(handlers::api_keys::list_keys))
        .route("/", post(handlers::api_keys::create_key))
        .route("/scope-plan", post(handlers::api_keys::plan_key_scope))
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
            "/{key_id}/durable-grants",
            get(handlers::api_keys::list_durable_grants),
        )
        .route(
            "/{key_id}/durable-grants/reauthorize",
            post(handlers::api_keys::reauthorize_durable_grants),
        )
        .route(
            "/{key_id}/durable-grants/{grant_id}/revoke",
            post(handlers::api_keys::revoke_durable_grant),
        )
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
            "/{service_id}/resync-identity",
            post(handlers::services::resync_service_identity),
        )
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
            "/{service_id}/anonymous-endpoints",
            get(handlers::admin_anonymous_endpoints::list_anonymous_endpoints)
                .post(handlers::admin_anonymous_endpoints::create_anonymous_endpoint),
        )
        .route(
            "/{service_id}/anonymous-endpoints/{rule_id}",
            put(handlers::admin_anonymous_endpoints::update_anonymous_endpoint)
                .delete(handlers::admin_anonymous_endpoints::delete_anonymous_endpoint),
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

    let session_routes = Router::new()
        .route("/", get(handlers::sessions::list_sessions))
        .route("/{id}", delete(handlers::sessions::revoke_own_session));

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
    let llm_routes = llm_billing_routes!(register_billing_routes, Router::new());

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
        // OAuth-connect on behalf of a service account is a state-mutating
        // action (creates an OAuth state row, emits an audit entry). It is
        // mounted as POST under the new route. The legacy GET form is kept
        // for one release of back-compat with older frontends/CLIs and will
        // be removed in a future release. Both methods route to the same
        // handler, which keeps `require_admin` (NOT `require_admin_or_operator`)
        // because operator is a read-only role.
        .route(
            "/{sa_id}/providers/{provider_id}/connect/oauth",
            post(handlers::admin_sa_providers::initiate_oauth_for_sa)
                .get(handlers::admin_sa_providers::initiate_oauth_for_sa),
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
            "/feature-flags",
            get(handlers::admin_feature_flags::list_feature_flags),
        )
        .route(
            "/feature-flags/{flag_key}",
            put(handlers::admin_feature_flags::set_feature_flag)
                .delete(handlers::admin_feature_flags::clear_feature_flag),
        )
        .route(
            "/feature-flags/{flag_key}/metadata",
            put(handlers::admin_feature_flags::update_feature_flag_metadata),
        )
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
        .route("/audit-log/verify", get(handlers::admin::verify_audit_log))
        .route(
            "/billing-ledger/verify",
            get(handlers::admin::verify_billing_ledger),
        )
        .route(
            "/chain-verification",
            get(handlers::admin::get_chain_verification),
        )
        .route(
            "/chain-verification/run",
            post(handlers::admin::run_chain_verification),
        )
        .route(
            "/settings/broker",
            get(handlers::admin::get_broker_settings)
                .patch(handlers::admin::update_broker_settings),
        )
        .route(
            "/oauth-clients",
            get(handlers::admin::list_oauth_clients).post(handlers::admin::create_oauth_client),
        )
        .route(
            "/oauth-clients/{client_id}",
            patch(handlers::admin::update_oauth_client)
                .delete(handlers::admin::delete_oauth_client),
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
        .route("/par", post(handlers::oauth::pushed_authorization_request))
        .route("/token", post(handlers::oauth::token))
        .route(
            "/bindings",
            get(handlers::oauth::list_bindings_by_external_subject),
        )
        .route(
            "/bindings/{binding_id}",
            get(handlers::oauth::get_binding).delete(handlers::oauth::delete_binding),
        )
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

    let node_registration_routes = Router::new().route(
        "/register-token",
        post(handlers::node_admin::create_registration_token),
    );

    let node_routes = Router::new()
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
            "/{node_id}/transfer",
            post(handlers::node_admin::transfer_node),
        )
        .route(
            "/{node_id}/credentials/push",
            post(handlers::node_admin::push_pending_credential),
        )
        .route(
            "/credentials/push/fan-out",
            post(handlers::node_admin::push_pending_credential_fan_out),
        )
        .route(
            "/credentials/pending/{fanout_id}/fan-out",
            get(handlers::node_admin::get_fan_out_pending_credential),
        )
        .route(
            "/credentials/pending/{fanout_id}/fan-out/pubkeys",
            get(handlers::node_admin::get_fan_out_pending_credential_pubkeys),
        )
        .route(
            "/credentials/pending/{fanout_id}/fan-out/ciphertexts",
            post(handlers::node_admin::post_fan_out_pending_credential_ciphertexts).layer(
                DefaultBodyLimit::max(
                    crate::services::node_pending_credential_service::MAX_FAN_OUT_HTTP_BODY_BYTES,
                ),
            ),
        )
        .route(
            "/credentials/pending/{fanout_id}/fan-out/retry-failed",
            post(handlers::node_admin::retry_failed_fan_out_pending_credential),
        )
        .route(
            "/{node_id}/credentials/pending",
            get(handlers::node_admin::list_pending_credentials),
        )
        .route(
            "/{node_id}/credentials/pending/{pending_id}",
            get(handlers::node_admin::get_pending_credential_pubkey)
                .delete(handlers::node_admin::cancel_pending_credential),
        )
        .route(
            "/{node_id}/credentials/pending/{pending_id}/remote-crypto",
            post(handlers::node_admin::init_pending_credential_remote_crypto),
        )
        .route(
            "/{node_id}/credentials/pending/{pending_id}/ciphertext",
            post(handlers::node_admin::post_pending_credential_ciphertext),
        )
        .route("/{node_id}/admins", get(handlers::node_admin::list_admins))
        .route(
            "/{node_id}/bindings",
            get(handlers::node_admin::list_bindings).post(handlers::node_admin::create_binding),
        )
        .route(
            "/{node_id}/bindings/{binding_id}",
            patch(handlers::node_admin::update_binding)
                .delete(handlers::node_admin::delete_binding),
        );

    let node_agent_routes = Router::new()
        .route(
            "/pending-credentials",
            get(handlers::node_agent::list_pending_credentials),
        )
        .route(
            "/pending-credentials/{pending_id}/consume",
            post(handlers::node_agent::consume_pending_credential),
        )
        .route(
            "/pending-credentials/{pending_id}/decline",
            post(handlers::node_agent::decline_pending_credential),
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

    let connect_link_routes = Router::new()
        .route("/", post(handlers::connect_links::create_connect_link))
        .route("/{id}", get(handlers::connect_links::get_connect_link))
        .route(
            "/{id}/cancel",
            post(handlers::connect_links::cancel_connect_link),
        )
        .layer(middleware::from_fn(reject_delegated_tokens))
        .layer(middleware::from_fn(reject_service_account_tokens))
        .layer(middleware::from_fn(reject_relay_tokens));

    let trigger_routes = Router::new()
        .route(
            "/",
            get(handlers::triggers::list_triggers).post(handlers::triggers::create_trigger),
        )
        .route(
            "/{id}",
            get(handlers::triggers::get_trigger)
                .patch(handlers::triggers::update_trigger)
                .delete(handlers::triggers::delete_trigger),
        )
        .route(
            "/{id}/rotate-secret",
            post(handlers::triggers::rotate_trigger_secret),
        )
        .route(
            "/{id}/rotate-delivery-secret",
            post(handlers::triggers::rotate_trigger_delivery_secret),
        )
        .route(
            "/{id}/deliveries",
            get(handlers::triggers::list_trigger_deliveries),
        )
        .route(
            "/{id}/deliveries/{event_id}/redeliver",
            post(handlers::triggers::redeliver_trigger_delivery),
        )
        .layer(middleware::from_fn(reject_delegated_tokens))
        .layer(middleware::from_fn(reject_service_account_tokens))
        .layer(middleware::from_fn(reject_relay_tokens));

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
            "/gcp-service-account",
            post(handlers::user_api_keys_external::create_gcp_service_account_key),
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
            "/{service_id}/ssh-auth-mode",
            patch(handlers::user_services_handler::patch_user_service_ssh_auth_mode),
        )
        .route(
            "/{service_id}",
            put(handlers::user_services_handler::update_user_service)
                .delete(handlers::user_services_handler::delete_user_service),
        );

    let service_pool_routes = Router::new()
        .route(
            "/",
            get(handlers::service_pools_handler::list_pools)
                .post(handlers::service_pools_handler::create_pool),
        )
        .route(
            "/{pool_id}",
            get(handlers::service_pools_handler::get_pool)
                .put(handlers::service_pools_handler::update_pool)
                .delete(handlers::service_pools_handler::delete_pool),
        )
        .route(
            "/{pool_id}/members",
            put(handlers::service_pools_handler::set_members)
                .post(handlers::service_pools_handler::add_member),
        )
        .route(
            "/{pool_id}/members/{user_service_id}",
            delete(handlers::service_pools_handler::remove_member),
        );

    let billing_routes = Router::new()
        .route("/usage", get(handlers::billing::get_usage))
        .route(
            "/wallet",
            get(handlers::billing::get_wallet).post(handlers::billing::provision_wallet),
        )
        .route("/topup", post(handlers::billing::create_topup))
        .route("/topups", get(handlers::billing::list_topups))
        .route(
            "/invoices/{invoice_id}/download",
            get(handlers::billing::download_invoice),
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
        )
        .route(
            "/oauth-clients/{client_id}/connection-webhook",
            put(handlers::developer_apps::configure_connection_webhook)
                .delete(handlers::developer_apps::disable_connection_webhook),
        )
        .route(
            "/oauth-clients/{client_id}/connection-webhook/rotate-secret",
            post(handlers::developer_apps::rotate_connection_webhook_secret),
        );

    // Proxy pass-through routes allow larger request bodies than the rest of the API.
    // Use RequestBodyLimitLayer so manual Request<Body> handlers are also protected.
    let proxy_passthrough_routes = proxy_billing_routes!(register_billing_routes, Router::new())
        .layer(RequestBodyLimitLayer::new(proxy_max_body_size));

    let public_passthrough_routes =
        public_proxy_billing_routes!(register_billing_routes, Router::new())
            .layer(RequestBodyLimitLayer::new(public_proxy_max_body_size));

    // LLM gateway routes get a moderate limit (10 MB for LLM payloads).
    let llm_routes = llm_routes.layer(RequestBodyLimitLayer::new(10 * 1024 * 1024));

    // Public API routes that expose non-sensitive runtime metadata.
    let device_code_public_routes = Router::new()
        .route("/request", post(handlers::devices::request_device_code))
        .route("/poll", post(handlers::devices::poll_device_code));
    let auth_device_public_routes = Router::new()
        .route("/request", post(handlers::auth_device::request_auth_device))
        .route("/poll", post(handlers::auth_device::poll_auth_device))
        .route("/preview", post(handlers::auth_device::preview_auth_device));
    let device_onboard_public_routes =
        Router::new().route("/redeem", post(handlers::devices::redeem_onboard_device));

    let api_v1_public = Router::new()
        .route(
            "/assistant/actions",
            get(handlers::assistant_actions::get_assistant_actions),
        )
        .route(
            "/runtime-config",
            get(handlers::runtime_config::get_runtime_config),
        )
        .route(
            "/catalog-specs/{spec_key}/openapi.json",
            get(handlers::docs::catalog_spec_json),
        )
        .nest("/auth/device", auth_device_public_routes)
        .route(
            "/connect-links/preview",
            post(handlers::connect_links::preview_connect_link),
        )
        .nest("/devices/code", device_code_public_routes)
        .nest("/devices/onboard", device_onboard_public_routes);

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
        .route("/demo", get(handlers::demo::get_demo))
        .nest("/channel-relay", channel_relay_routes)
        .nest("/channel-events", channel_event_routes)
        .merge(proxy_passthrough_routes);

    // Oracle relay consumer routes: submit tasks to browser worker pools
    // and poll results. Open to users AND agent API keys (blocks delegated
    // tokens via the shared layer below). Submits can carry a PDF
    // attachment as base64, so this router gets a 16 MiB body cap instead
    // of the app-wide 1 MiB.
    let oracle_consumer_routes = oracle_billing_routes!(
        register_billing_routes,
        Router::new()
            .route(
                "/pools",
                get(handlers::oracle_pools::list_pools).post(handlers::oracle_pools::create_pool),
            )
            .route(
                "/pools/{id_or_slug}",
                get(handlers::oracle_pools::get_pool).patch(handlers::oracle_pools::update_pool),
            )
            .route(
                "/pools/{id_or_slug}/rotate-token",
                post(handlers::oracle_pools::rotate_token),
            )
    )
    .layer(DefaultBodyLimit::max(16 * 1024 * 1024));

    // Routes accessible by both users and service accounts (block delegated tokens)
    let api_v1_shared = Router::new()
        .nest("/connections", connection_routes)
        .nest("/providers", provider_routes)
        .nest("/nodes", node_registration_routes)
        .nest("/oracle", oracle_consumer_routes)
        .nest("/connect-links", connect_link_routes)
        .nest("/triggers", trigger_routes)
        .layer(middleware::from_fn(reject_delegated_tokens))
        .layer(middleware::from_fn(reject_relay_tokens));

    // Assistant chat pass-through (PRD decision 4): the browser calls these
    // NyxID routes with its session cookie and NyxID forwards to the
    // admin-managed Aevatar service with a server-derived scope. Mounted in
    // the human-only router below: the chat surface -- approvals above all --
    // is a human session surface, so API-key, service-account, delegated, and
    // relay callers are rejected by that router's layers.
    let assistant_proxy_routes = Router::new()
        .route(
            "/conversations",
            get(handlers::assistant::list_conversations),
        )
        .route(
            "/conversations/{conversation_id}",
            get(handlers::assistant::get_history).delete(handlers::assistant::delete_conversation),
        )
        .route(
            "/conversations/{conversation_id}/state",
            get(handlers::assistant::get_state),
        )
        .route("/completions", post(handlers::assistant::completions))
        .route("/chat", post(handlers::assistant::typed_chat))
        .route_layer(axum::Extension(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        ));
    let assistant_routes = Router::new()
        .route(
            "/readiness",
            get(handlers::assistant_readiness::get_readiness),
        )
        .merge(assistant_proxy_routes);

    let ssh_billing_routes = ssh_billing_routes!(register_billing_routes, Router::new());

    // Routes that BLOCK service account tokens (human-only endpoints)
    let api_v1_human_only = Router::new()
        .nest("/assistant", assistant_routes)
        .nest("/auth", auth_routes)
        .route(
            "/devices/code/approve",
            post(handlers::devices::approve_device_code),
        )
        .route(
            "/auth/device/approve",
            post(handlers::auth_device::approve_auth_device),
        )
        .route(
            "/connect-links/complete",
            post(handlers::connect_links::complete_connect_link),
        )
        .route(
            "/connect-links/cancel",
            post(handlers::connect_links::cancel_hosted_connect_link),
        )
        .route("/devices/onboard", post(handlers::devices::onboard_device))
        .route(
            "/devices/onboard/{bootstrap_id}",
            delete(handlers::devices::revoke_onboard_device),
        )
        .nest("/users", user_routes)
        .nest("/api-keys", api_key_routes)
        .nest("/services", service_routes)
        .route("/docs", get(handlers::docs::docs_ui))
        .route("/docs/catalog", get(handlers::docs::catalog_ui))
        .route("/docs/openapi.json", get(handlers::docs::openapi_json))
        .route("/docs/asyncapi.json", get(handlers::docs::asyncapi_json))
        .merge(ssh_billing_routes)
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
        .nest("/service-pools", service_pool_routes)
        .nest("/billing", billing_routes)
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
        .layer(middleware::from_fn(reject_api_key_tokens))
        .layer(middleware::from_fn(reject_service_account_tokens))
        .layer(middleware::from_fn(reject_relay_tokens));

    let api_v1 = api_v1_public
        .merge(api_v1_delegated)
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
    let webhook_routes = Router::new()
        .route("/telegram", post(handlers::webhooks::telegram_webhook))
        .route("/lago", post(handlers::billing::lago_webhook));
    let trigger_webhook_routes = Router::new()
        .route(
            "/{trigger_id}",
            post(handlers::trigger_webhooks::receive_trigger),
        )
        .layer(DefaultBodyLimit::disable());

    // Integration webhook routes -- unauthenticated (verified by HMAC signature)
    let integration_routes = Router::new().route(
        "/openclaw/channel",
        post(handlers::openclaw_channel::handle_channel_message),
    );

    let private = Router::new()
        .route("/health", get(handlers::health::health_check))
        .route("/llms.txt", get(handlers::llms_txt::llms_txt))
        .route("/llms-full.txt", get(handlers::llms_txt::llms_full_txt))
        .route(
            "/nodes/{node_id}/credentials/pending/{pending_id}/accept",
            get(handlers::credential_accept::accept_page),
        )
        .route(
            "/nodes/credentials/pending/{pending_id}/fan-out/accept",
            get(handlers::credential_accept::fan_out_accept_page),
        )
        .route(
            "/credential-accept/assets/{*file}",
            get(handlers::credential_accept::asset),
        )
        .nest("/api/v1/webhooks", webhook_routes)
        .nest("/api/v1/webhooks/triggers", trigger_webhook_routes)
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
        .nest("/api/v1/node-agent", node_agent_routes)
        // Oracle worker routes -- authenticated by the pool worker token
        // (`nyx_owk_...`) inside each handler, NOT by the JWT middleware,
        // so they mount alongside node-agent rather than inside api_v1.
        // Results can carry multi-MB extracted answers: 16 MiB body cap.
        .nest(
            "/api/v1/oracle/worker",
            Router::new()
                .route("/task", get(handlers::oracle_worker::poll_task))
                .route("/ack", post(handlers::oracle_worker::ack))
                .route("/result", post(handlers::oracle_worker::submit_result))
                .route(
                    "/transcript",
                    post(handlers::oracle_worker::submit_transcript),
                )
                .route("/pin-conv-url", post(handlers::oracle_worker::pin_conv_url))
                .layer(DefaultBodyLimit::max(16 * 1024 * 1024)),
        )
        .nest("/api/v1", api_v1)
        .merge(public_passthrough_routes)
        // WebSocket endpoint for node agents. Auth happens in-message (not middleware).
        // Rate limiting: global per-IP rate limiter covers HTTP upgrade requests.
        // Connection limiting: NodeWsManager enforces max concurrent connections.
        .route("/api/v1/nodes/ws", get(handlers::node_ws::ws_handler));

    let private = mcp_billing_routes!(register_billing_routes, private);
    let private = public_mcp_billing_routes!(register_billing_routes, private);

    (public_oauth, private)
}
