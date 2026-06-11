use axum::{
    Json,
    extract::{Path, State},
};
use chrono::Utc;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    AnonymousEndpointRule, COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::mw::auth::AuthUser;
use crate::services::{anonymous_endpoint_service, audit_service};

use super::services_helpers::{fetch_service, require_admin_or_creator};

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateAnonymousEndpointRequest {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    pub method: String,
    pub path_pattern: String,
    #[serde(default = "default_daily_quota")]
    pub daily_quota: u32,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateAnonymousEndpointRequest {
    pub enabled: Option<bool>,
    pub method: Option<String>,
    pub path_pattern: Option<String>,
    pub daily_quota: Option<u32>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AnonymousEndpointResponse {
    pub id: String,
    pub enabled: bool,
    pub method: String,
    pub path_pattern: String,
    pub daily_quota: u32,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct AnonymousEndpointListResponse {
    pub endpoints: Vec<AnonymousEndpointResponse>,
}

fn default_enabled() -> bool {
    true
}

fn default_daily_quota() -> u32 {
    1_000
}

fn response(rule: AnonymousEndpointRule) -> AnonymousEndpointResponse {
    AnonymousEndpointResponse {
        id: rule.id,
        enabled: rule.enabled,
        method: rule.method,
        path_pattern: rule.path_pattern,
        daily_quota: rule.daily_quota,
    }
}

pub async fn list_anonymous_endpoints(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
) -> AppResult<Json<AnonymousEndpointListResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;
    Ok(Json(AnonymousEndpointListResponse {
        endpoints: service
            .anonymous_endpoints
            .into_iter()
            .map(response)
            .collect(),
    }))
}

pub async fn create_anonymous_endpoint(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(service_id): Path<String>,
    Json(body): Json<CreateAnonymousEndpointRequest>,
) -> AppResult<Json<AnonymousEndpointResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    let rule =
        anonymous_endpoint_service::build_rule(anonymous_endpoint_service::AnonymousRuleInput {
            enabled: body.enabled,
            method: body.method,
            path_pattern: body.path_pattern,
            daily_quota: body.daily_quota,
        })?;
    let mut rules = service.anonymous_endpoints.clone();
    rules.push(rule.clone());
    anonymous_endpoint_service::validate_rules_for_service(&service, &rules)?;
    persist_rules(&state, &service_id, &rules).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "anonymous_endpoint_created",
        Some(serde_json::json!({
            "service_id": service_id,
            "rule_id": rule.id,
            "enabled": rule.enabled,
            "method": rule.method,
            "path_pattern": rule.path_pattern,
            "daily_quota": rule.daily_quota,
        })),
    );

    Ok(Json(response(rule)))
}

pub async fn update_anonymous_endpoint(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((service_id, rule_id)): Path<(String, String)>,
    Json(body): Json<UpdateAnonymousEndpointRequest>,
) -> AppResult<Json<AnonymousEndpointResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    let mut rules = service.anonymous_endpoints.clone();
    let index = rules
        .iter()
        .position(|rule| rule.id == rule_id)
        .ok_or_else(|| AppError::NotFound("Anonymous endpoint not found".to_string()))?;
    let updated = anonymous_endpoint_service::apply_rule_update(
        &rules[index],
        anonymous_endpoint_service::AnonymousRuleUpdate {
            enabled: body.enabled,
            method: body.method,
            path_pattern: body.path_pattern,
            daily_quota: body.daily_quota,
        },
    )?;
    rules[index] = updated.clone();
    anonymous_endpoint_service::validate_rules_for_service(&service, &rules)?;
    persist_rules(&state, &service_id, &rules).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "anonymous_endpoint_updated",
        Some(serde_json::json!({
            "service_id": service_id,
            "rule_id": updated.id,
            "enabled": updated.enabled,
            "method": updated.method,
            "path_pattern": updated.path_pattern,
            "daily_quota": updated.daily_quota,
        })),
    );

    Ok(Json(response(updated)))
}

pub async fn delete_anonymous_endpoint(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((service_id, rule_id)): Path<(String, String)>,
) -> AppResult<Json<AnonymousEndpointListResponse>> {
    let service = fetch_service(&state, &service_id).await?;
    require_admin_or_creator(&state, &auth_user, &service.created_by).await?;

    let mut rules = service.anonymous_endpoints.clone();
    let before = rules.len();
    rules.retain(|rule| rule.id != rule_id);
    if rules.len() == before {
        return Err(AppError::NotFound(
            "Anonymous endpoint not found".to_string(),
        ));
    }
    anonymous_endpoint_service::validate_rules_for_service(&service, &rules)?;
    persist_rules(&state, &service_id, &rules).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "anonymous_endpoint_deleted",
        Some(serde_json::json!({
            "service_id": service_id,
            "rule_id": rule_id,
        })),
    );

    Ok(Json(AnonymousEndpointListResponse {
        endpoints: rules.into_iter().map(response).collect(),
    }))
}

async fn persist_rules(
    state: &AppState,
    service_id: &str,
    rules: &[AnonymousEndpointRule],
) -> AppResult<()> {
    let rules_bson = bson::to_bson(rules)
        .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;
    state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! { "_id": service_id },
            doc! { "$set": {
                "anonymous_endpoints": rules_bson,
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            }},
        )
        .await?;
    Ok(())
}
