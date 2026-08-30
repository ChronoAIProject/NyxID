use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::AppResult;
use crate::handlers::admin_helpers::require_admin;
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::platform_operation::{
    ConstrainedConfig, OperationBilling, OperationBillingComponent, OperationLimits,
    PerRequestCaps, PlatformOperationKind, PlatformOperationRow,
};
use crate::models::service_billing::{BillingMetric, PricingSyncStatus};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, platform_operation_service};

#[derive(Debug, Serialize)]
pub struct AdminPlatformOperationListResponse {
    pub operations: Vec<AdminPlatformOperationResponse>,
}

#[derive(Debug, Serialize)]
pub struct AdminPlatformOperationResponse {
    pub operation_id: String,
    pub catalog_service_id: String,
    pub provider_slug: Option<String>,
    pub provider_name: Option<String>,
    pub operation_name: String,
    pub enabled: bool,
    pub kind: AdminPlatformOperationKindResponse,
    pub limits: AdminOperationLimitsResponse,
    pub pricing: AdminPlatformOperationPricingResponse,
    pub created_at: String,
    pub created_by: String,
    pub updated_at: String,
    pub updated_by: String,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminPlatformOperationKindResponse {
    Endpoint {
        method: String,
        path_template: String,
        name: String,
        description: Option<String>,
    },
    Constrained {
        op: &'static str,
        config: AdminConstrainedConfigResponse,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminConstrainedConfigResponse {
    Speak {
        allowed_voice_ids: Vec<String>,
        model_id: String,
        max_calls_per_user_per_day: u32,
    },
    CallAndSay {
        allowed_destination_prefixes: Vec<String>,
        voice: String,
        account_sid: String,
        call_from: String,
    },
    FlightSearch,
}

#[derive(Debug, Serialize)]
pub struct AdminOperationLimitsResponse {
    pub per_request: AdminPerRequestCapsResponse,
    pub per_user_per_day: Option<u32>,
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminPerRequestCapsResponse {
    Endpoint,
    Speak {
        max_chars: u32,
    },
    CallAndSay {
        max_message_chars: u32,
        max_duration_seconds: u32,
    },
    FlightSearch {
        max_offers: u32,
    },
}

#[derive(Debug, Serialize)]
pub struct AdminPlatformOperationPricingResponse {
    pub billable: bool,
    pub metric: &'static str,
    pub price_per_unit: String,
    pub secondary: Option<AdminPlatformOperationPricingComponentResponse>,
    pub base_fee_per_call: Option<String>,
    pub display: String,
    pub lago_metric_code: String,
    pub sync_status: PricingSyncStatus,
    pub sync_error: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdminPlatformOperationPricingComponentResponse {
    pub metric: &'static str,
    pub price_per_unit: String,
    pub lago_metric_code: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreatePlatformOperationRequest {
    pub catalog_service_id: String,
    pub enabled: bool,
    pub kind: PlatformOperationKind,
    pub limits: OperationLimits,
    pub billing: UpdatePlatformOperationBillingRequest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlatformOperationRequest {
    pub enabled: bool,
    pub kind: PlatformOperationKind,
    pub limits: OperationLimits,
    pub billing: UpdatePlatformOperationBillingRequest,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlatformOperationBillingRequest {
    pub metric: BillingMetric,
    pub price_per_unit: String,
    #[serde(default)]
    pub secondary: Option<UpdatePlatformOperationBillingComponentRequest>,
    #[serde(default)]
    pub base_fee_per_call: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlatformOperationBillingComponentRequest {
    pub metric: BillingMetric,
    pub price_per_unit: String,
}

impl From<UpdatePlatformOperationBillingRequest> for OperationBilling {
    fn from(value: UpdatePlatformOperationBillingRequest) -> Self {
        Self {
            metric: value.metric,
            price_per_unit: value.price_per_unit,
            secondary: value.secondary.map(|component| OperationBillingComponent {
                metric: component.metric,
                price_per_unit: component.price_per_unit,
                lago_metric_code: String::new(),
            }),
            base_fee_per_call: value.base_fee_per_call,
            lago_metric_code: String::new(),
            sync_status: PricingSyncStatus::Pending,
            sync_error: None,
        }
    }
}

#[derive(Debug, Serialize)]
pub struct DeletePlatformOperationResponse {
    pub deleted_operation_id: String,
}

/// GET /api/v1/admin/platform-ops
pub async fn list_platform_operations(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<AdminPlatformOperationListResponse>> {
    require_admin(&state, &auth_user).await?;
    let rows = platform_operation_service::list_operation_rows(&state.db).await?;
    let service_ids = rows
        .iter()
        .map(|row| row.catalog_service_id.as_str())
        .collect::<Vec<_>>();
    let services: Vec<DownstreamService> = state
        .db
        .collection(DOWNSTREAM_SERVICES)
        .find(doc! { "_id": { "$in": service_ids } })
        .await?
        .try_collect()
        .await?;
    let services = services
        .iter()
        .map(|service| (service.id.as_str(), service))
        .collect::<HashMap<_, _>>();
    let operations = rows
        .into_iter()
        .map(|row| {
            let service = services.get(row.catalog_service_id.as_str()).copied();
            platform_operation_response(row, service)
        })
        .collect();
    Ok(Json(AdminPlatformOperationListResponse { operations }))
}

/// POST /api/v1/admin/platform-ops
pub async fn create_platform_operation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreatePlatformOperationRequest>,
) -> AppResult<Json<AdminPlatformOperationResponse>> {
    require_admin(&state, &auth_user).await?;
    let actor_id = auth_user.user_id.to_string();
    let row = platform_operation_service::create_operation_row(
        &state.db,
        &body.catalog_service_id,
        body.enabled,
        body.kind,
        body.limits,
        body.billing.into(),
        &actor_id,
    )
    .await?;
    state.billing.sync_operation_price(&row).await?;
    let row = platform_operation_service::load_operation_row(&state.db, &row.id).await?;
    audit_operation_change(&state, &auth_user, "admin_platform_operation_created", &row);
    operation_json(&state, row).await
}

/// PUT /api/v1/admin/platform-ops/{operation_id}
pub async fn update_platform_operation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(operation_id): Path<String>,
    Json(body): Json<UpdatePlatformOperationRequest>,
) -> AppResult<Json<AdminPlatformOperationResponse>> {
    require_admin(&state, &auth_user).await?;
    let row = platform_operation_service::update_operation_row(
        &state.db,
        &operation_id,
        body.enabled,
        body.kind,
        body.limits,
        body.billing.into(),
        &auth_user.user_id.to_string(),
    )
    .await?;
    state.billing.sync_operation_price(&row).await?;
    let row = platform_operation_service::load_operation_row(&state.db, &row.id).await?;
    audit_operation_change(&state, &auth_user, "admin_platform_operation_updated", &row);
    operation_json(&state, row).await
}

/// DELETE /api/v1/admin/platform-ops/{operation_id}
pub async fn delete_platform_operation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(operation_id): Path<String>,
) -> AppResult<Json<DeletePlatformOperationResponse>> {
    require_admin(&state, &auth_user).await?;
    let row = platform_operation_service::prepare_operation_deletion(
        &state.db,
        &operation_id,
        &auth_user.user_id.to_string(),
    )
    .await?;
    state.billing.remove_operation_price(&row).await?;
    let deleted = platform_operation_service::delete_operation_row(&state.db, &row.id).await?;
    audit_operation_change(
        &state,
        &auth_user,
        "admin_platform_operation_deleted",
        &deleted,
    );
    Ok(Json(DeletePlatformOperationResponse {
        deleted_operation_id: deleted.id,
    }))
}

async fn operation_json(
    state: &AppState,
    row: PlatformOperationRow,
) -> AppResult<Json<AdminPlatformOperationResponse>> {
    let service = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": &row.catalog_service_id })
        .await?;
    Ok(Json(platform_operation_response(row, service.as_ref())))
}

fn audit_operation_change(
    state: &AppState,
    auth_user: &AuthUser,
    action: &str,
    row: &PlatformOperationRow,
) {
    audit_service::log_for_user(
        state.db.clone(),
        auth_user,
        action,
        Some(serde_json::json!({
            "operation_id": row.id,
            "catalog_service_id": row.catalog_service_id,
            "kind_key": row.kind_key,
            "enabled": row.enabled,
        })),
    );
}

pub(crate) fn platform_operation_response(
    row: PlatformOperationRow,
    vendor: Option<&DownstreamService>,
) -> AdminPlatformOperationResponse {
    let billable = row.billing.price_per_unit != "0"
        || row.billing.secondary.is_some()
        || row.billing.base_fee_per_call.is_some();
    let pricing = AdminPlatformOperationPricingResponse {
        billable,
        metric: row.billing.metric.as_str(),
        price_per_unit: row.billing.price_per_unit.clone(),
        secondary: row.billing.secondary.as_ref().map(|component| {
            AdminPlatformOperationPricingComponentResponse {
                metric: component.metric.as_str(),
                price_per_unit: component.price_per_unit.clone(),
                lago_metric_code: component.lago_metric_code.clone(),
            }
        }),
        base_fee_per_call: row.billing.base_fee_per_call.clone(),
        display: crate::services::billing::pricing::format_operation_price(&row.billing, billable),
        lago_metric_code: row.billing.lago_metric_code.clone(),
        sync_status: row.billing.sync_status,
        sync_error: row.billing.sync_error.clone(),
    };
    let operation_name = match &row.kind {
        PlatformOperationKind::Endpoint { name, .. } => name.clone(),
        PlatformOperationKind::Constrained { op, .. } => {
            platform_operation_service::catalog_contract_for_operation(*op)
                .display_name
                .to_string()
        }
    };
    AdminPlatformOperationResponse {
        operation_id: row.id,
        catalog_service_id: row.catalog_service_id,
        provider_slug: vendor.map(|service| service.slug.clone()),
        provider_name: vendor.map(|service| service.name.clone()),
        operation_name,
        enabled: row.enabled,
        kind: row.kind.into(),
        limits: row.limits.into(),
        pricing,
        created_at: row.created_at.to_rfc3339(),
        created_by: row.created_by,
        updated_at: row.updated_at.to_rfc3339(),
        updated_by: if row.updated_by.is_empty() {
            "legacy".to_string()
        } else {
            row.updated_by
        },
    }
}

impl From<PlatformOperationKind> for AdminPlatformOperationKindResponse {
    fn from(kind: PlatformOperationKind) -> Self {
        match kind {
            PlatformOperationKind::Endpoint {
                method,
                path_template,
                name,
                description,
            } => Self::Endpoint {
                method,
                path_template,
                name,
                description,
            },
            PlatformOperationKind::Constrained { op, config } => Self::Constrained {
                op: platform_operation_service::operation_name(op),
                config: config.into(),
            },
        }
    }
}

impl From<ConstrainedConfig> for AdminConstrainedConfigResponse {
    fn from(config: ConstrainedConfig) -> Self {
        match config {
            ConstrainedConfig::Speak(config) => Self::Speak {
                allowed_voice_ids: config.allowed_voice_ids,
                model_id: config.model_id,
                max_calls_per_user_per_day: config.max_calls_per_user_per_day,
            },
            ConstrainedConfig::CallAndSay(config) => Self::CallAndSay {
                allowed_destination_prefixes: config.allowed_destination_prefixes,
                voice: config.voice,
                account_sid: config.account_sid,
                call_from: config.call_from,
            },
            ConstrainedConfig::FlightSearch(_) => Self::FlightSearch,
        }
    }
}

impl From<OperationLimits> for AdminOperationLimitsResponse {
    fn from(limits: OperationLimits) -> Self {
        Self {
            per_request: limits.per_request.into(),
            per_user_per_day: limits.per_user_per_day,
        }
    }
}

impl From<PerRequestCaps> for AdminPerRequestCapsResponse {
    fn from(caps: PerRequestCaps) -> Self {
        match caps {
            PerRequestCaps::Endpoint => Self::Endpoint,
            PerRequestCaps::Speak { max_chars } => Self::Speak { max_chars },
            PerRequestCaps::CallAndSay {
                max_message_chars,
                max_duration_seconds,
            } => Self::CallAndSay {
                max_message_chars,
                max_duration_seconds,
            },
            PerRequestCaps::FlightSearch { max_offers } => Self::FlightSearch { max_offers },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::platform_operation::{
        OperationBilling, OperationLimits, PerRequestCaps, PlatformOperationRow,
    };

    #[test]
    fn write_request_rejects_unknown_fields() {
        let value = serde_json::json!({
            "enabled": false,
            "kind": {
                "kind": "endpoint",
                "method": "GET",
                "path_template": "/v1/items/{id}",
                "name": "Get item"
            },
            "limits": {
                "per_request": { "type": "endpoint" },
                "per_user_per_day": 10
            },
            "billing": {
                "metric": "requests",
                "price_per_unit": "1"
            },
            "vendor_body": { "text": "must never be accepted" }
        });
        assert!(serde_json::from_value::<UpdatePlatformOperationRequest>(value).is_err());
    }

    #[test]
    fn endpoint_response_exposes_uuid_identity_and_response_byte_price() {
        let mut row = PlatformOperationRow::new_endpoint(
            "catalog-duffel".to_string(),
            "GET".to_string(),
            "/air/offers/{id}".to_string(),
            "Get offer".to_string(),
            None,
            OperationLimits {
                per_request: PerRequestCaps::Endpoint,
                per_user_per_day: Some(20),
            },
            OperationBilling::free(BillingMetric::Bytes),
            "admin-user".to_string(),
        );
        row.billing.lago_metric_code = "platform_op_duffel_get_offer_hash".to_string();
        let response = platform_operation_response(row.clone(), None);

        assert_eq!(response.operation_id, row.id);
        assert_eq!(response.operation_name, "Get offer");
        assert_eq!(response.pricing.metric, "bytes");
        assert!(matches!(
            response.kind,
            AdminPlatformOperationKindResponse::Endpoint { .. }
        ));
    }
}
