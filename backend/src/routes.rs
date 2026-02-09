use axum::{
    routing::{delete, get, post, put},
    Router,
};

use crate::handlers;
use crate::AppState;

/// Build the complete application router with all route groups.
pub fn build_router() -> Router<AppState> {
    let auth_routes = Router::new()
        .route("/register", post(handlers::auth::register))
        .route("/login", post(handlers::auth::login))
        .route("/logout", post(handlers::auth::logout))
        .route("/refresh", post(handlers::auth::refresh))
        .route("/verify-email", post(handlers::auth::verify_email))
        .route("/forgot-password", post(handlers::auth::forgot_password))
        .route("/reset-password", post(handlers::auth::reset_password));

    let mfa_routes = Router::new()
        .route("/setup", post(handlers::mfa::setup))
        .route("/verify-setup", post(handlers::mfa::verify_setup));

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
        .route("/{service_id}", delete(handlers::services::delete_service));

    let admin_routes = Router::new()
        .route("/users", get(handlers::admin::list_users))
        .route("/users/{user_id}", get(handlers::admin::get_user))
        .route("/audit-log", get(handlers::admin::list_audit_log));

    let oauth_routes = Router::new()
        .route("/authorize", get(handlers::oauth::authorize))
        .route("/token", post(handlers::oauth::token))
        .route("/userinfo", get(handlers::oauth::userinfo));

    let api_v1 = Router::new()
        .nest("/auth", auth_routes)
        .nest("/users", user_routes)
        .nest("/mfa", mfa_routes)
        .nest("/api-keys", api_key_routes)
        .nest("/services", service_routes)
        .nest("/admin", admin_routes)
        .route("/proxy/{service_id}/{*path}", axum::routing::any(handlers::proxy::proxy_request));

    Router::new()
        .route("/health", get(handlers::health::health_check))
        .nest("/oauth", oauth_routes)
        .nest("/api/v1", api_v1)
}
