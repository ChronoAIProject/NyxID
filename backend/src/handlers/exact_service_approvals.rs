use axum::{
    Extension, Json,
    extract::{Path, State},
};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::billing::route_inventory::{
    BillingIngress, BillingRoutePolicy, enforce_billing_egress_classification,
};
use crate::services::exact_service_approval_service::{
    self, ExactServiceApprovalCaller, ExactServiceApprovalCreate, ExactServiceApprovalFence,
    ExactServiceApprovalResult,
};

pub async fn create_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(input): Json<ExactServiceApprovalCreate>,
) -> AppResult<Json<ExactServiceApprovalResult>> {
    auth_user.ensure_rest_proxy_access()?;
    enforce_rate_limit(&state, &auth_user)?;
    let caller = caller(&auth_user)?;
    Ok(Json(
        exact_service_approval_service::create_request(&state, &caller, input).await?,
    ))
}

pub async fn observe_request(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(request_id): Path<String>,
) -> AppResult<Json<ExactServiceApprovalResult>> {
    auth_user.ensure_rest_proxy_access()?;
    enforce_rate_limit(&state, &auth_user)?;
    let caller = caller(&auth_user)?;
    Ok(Json(
        exact_service_approval_service::observe_request(&state, &caller, &request_id).await?,
    ))
}

pub async fn redeem_request(
    State(state): State<AppState>,
    Extension(billing_route_policy): Extension<BillingRoutePolicy>,
    auth_user: AuthUser,
    Path(request_id): Path<String>,
    Json(fence): Json<ExactServiceApprovalFence>,
) -> AppResult<Json<ExactServiceApprovalResult>> {
    auth_user.ensure_rest_proxy_access()?;
    enforce_rate_limit(&state, &auth_user)?;
    let caller = caller(&auth_user)?;
    let permit =
        enforce_billing_egress_classification(Some(billing_route_policy), BillingIngress::Mcp)?;
    Ok(Json(
        exact_service_approval_service::redeem_request(&state, &caller, &request_id, fence, permit)
            .await?,
    ))
}

fn caller(auth_user: &AuthUser) -> AppResult<ExactServiceApprovalCaller> {
    let actor_user_id = auth_user.user_id.to_string();
    let (requester_type, requester_id) =
        match auth_user.auth_method {
            AuthMethod::ApiKey => (
                "api_key",
                auth_user.api_key_id.clone().ok_or_else(|| {
                    AppError::Unauthorized("api_key identity missing".to_string())
                })?,
            ),
            AuthMethod::Relay => (
                "relay",
                auth_user.api_key_id.clone().ok_or_else(|| {
                    AppError::Unauthorized("relay key identity missing".to_string())
                })?,
            ),
            AuthMethod::Delegated => (
                "delegated",
                auth_user.acting_client_id.clone().ok_or_else(|| {
                    AppError::Unauthorized("delegated client identity missing".to_string())
                })?,
            ),
            AuthMethod::ServiceAccount => ("service_account", actor_user_id.clone()),
            AuthMethod::AccessToken => (
                "access_token",
                auth_user
                    .oauth_client_id
                    .clone()
                    .unwrap_or_else(|| actor_user_id.clone()),
            ),
            AuthMethod::Session => ("session", actor_user_id.clone()),
        };
    Ok(ExactServiceApprovalCaller {
        actor_user_id,
        proxy_resolution_user_id: auth_user.proxy_resolution_user_id(),
        approval_owner_user_id: auth_user.effective_approval_owner_user_id(),
        requester_type: requester_type.to_string(),
        requester_id,
        requester_label: auth_user.api_key_name.clone(),
        api_key_id: auth_user.api_key_id.clone(),
        allow_all_services: auth_user.allow_all_services,
        allowed_service_ids: auth_user.allowed_service_ids.clone(),
        allow_all_nodes: auth_user.allow_all_nodes,
        allowed_node_ids: auth_user.allowed_node_ids.clone(),
    })
}

fn enforce_rate_limit(state: &AppState, auth_user: &AuthUser) -> AppResult<()> {
    crate::mw::rate_limit::check_agent_rate_limit_raw(
        &state.per_agent_limiter,
        auth_user.api_key_id.as_deref(),
        auth_user.rate_limit_per_second,
        auth_user.rate_limit_burst,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::api_key::ApiKeyPurpose;

    fn auth(method: AuthMethod) -> AuthUser {
        AuthUser {
            user_id: uuid::Uuid::new_v4(),
            session_id: None,
            scope: "proxy".to_string(),
            acting_client_id: (method == AuthMethod::Delegated).then(|| "client-alpha".to_string()),
            oauth_client_id: None,
            approval_owner_user_id: None,
            auth_method: method,
            allow_all_services: true,
            allow_all_nodes: true,
            allowed_service_ids: vec![],
            resource_uris: None,
            allowed_node_ids: vec![],
            api_key_id: Some("key-alpha".to_string()),
            api_key_name: Some("Agent".to_string()),
            api_key_purpose: ApiKeyPurpose::General,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            ip_address: None,
            user_agent: None,
        }
    }

    #[test]
    fn api_key_requester_is_bound_to_key_not_user() {
        let auth = auth(AuthMethod::ApiKey);
        let caller = caller(&auth).unwrap();
        assert_eq!(caller.requester_id, "key-alpha");
        assert_ne!(caller.requester_id, auth.user_id.to_string());
    }

    #[test]
    fn delegated_requester_is_bound_to_oauth_client() {
        let auth = auth(AuthMethod::Delegated);
        assert_eq!(caller(&auth).unwrap().requester_id, "client-alpha");
    }
}
