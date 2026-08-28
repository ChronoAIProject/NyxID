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
use crate::models::platform_operation::{
    CallAndSayConfig, FlightSearchConfig, OperationBilling, OperationBillingComponent,
    PlatformOperation, PlatformOperationConfig, PlatformOperationName, SpeakConfig,
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
    pub op: &'static str,
    pub enabled: bool,
    pub vendor_service_slug: String,
    pub config: AdminPlatformOperationConfigResponse,
    pub updated_at: Option<String>,
    pub updated_by: Option<String>,
    pub vendor_service_id: Option<String>,
    pub pricing: AdminPlatformOperationPricingResponse,
}

#[derive(Debug, Serialize)]
pub struct AdminPlatformOperationPricingResponse {
    pub billable: bool,
    pub metric: &'static str,
    pub price_per_unit: String,
    pub secondary: Option<AdminPlatformOperationPricingComponentResponse>,
    pub base_fee_per_call: Option<String>,
    /// Server-rendered price sentence. The admin table shows this verbatim so
    /// every metric formats identically across surfaces.
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

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminPlatformOperationConfigResponse {
    Speak(AdminSpeakConfigResponse),
    CallAndSay(AdminCallAndSayConfigResponse),
    FlightSearch(AdminFlightSearchConfigResponse),
}

#[derive(Debug, Serialize)]
pub struct AdminSpeakConfigResponse {
    pub allowed_voice_ids: Vec<String>,
    pub max_chars: u32,
    pub model_id: String,
    pub max_calls_per_user_per_day: u32,
}

#[derive(Debug, Serialize)]
pub struct AdminCallAndSayConfigResponse {
    pub allowed_destination_prefixes: Vec<String>,
    pub max_message_chars: u32,
    pub max_duration_seconds: u32,
    pub voice: String,
    pub max_calls_per_user_per_day: u32,
    pub account_sid: String,
    pub call_from: String,
}

#[derive(Debug, Serialize)]
pub struct AdminFlightSearchConfigResponse {
    pub max_offers_cap: u32,
    pub max_searches_per_user_per_day: u32,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlatformOperationRequest {
    pub enabled: bool,
    pub vendor_service_slug: String,
    pub config: PlatformOperationConfig,
    #[serde(default)]
    pub billing: Option<UpdatePlatformOperationBillingRequest>,
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

/// GET /api/v1/admin/platform-ops
pub async fn list_platform_operations(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<AdminPlatformOperationListResponse>> {
    // Deliberately ungated: administrators can stage per-operation configuration
    // while the caller-facing platform-services feature flag remains disabled.
    require_admin(&state, &auth_user).await?;
    let configured = platform_operation_service::list_configured_operations(&state.db).await?;
    let vendor_slugs = platform_operation_service::PLATFORM_OPERATION_NAMES
        .iter()
        .map(|op| platform_operation_service::default_vendor_service_slug(*op))
        .collect::<Vec<_>>();
    let vendor_services: Vec<crate::models::downstream_service::DownstreamService> = state
        .db
        .collection(crate::models::downstream_service::COLLECTION_NAME)
        .find(doc! { "slug": { "$in": vendor_slugs }, "is_active": true })
        .await?
        .try_collect()
        .await?;
    let operations = platform_operation_service::PLATFORM_OPERATION_NAMES
        .into_iter()
        .map(|op| {
            let row = configured.iter().find(|row| row.op == op).cloned();
            let vendor_slug = row
                .as_ref()
                .map(|operation| operation.vendor_service_slug.as_str())
                .unwrap_or_else(|| platform_operation_service::default_vendor_service_slug(op));
            let vendor = vendor_services
                .iter()
                .find(|service| service.slug == vendor_slug);
            platform_operation_response(op, row, vendor)
        })
        .collect();

    Ok(Json(AdminPlatformOperationListResponse { operations }))
}

/// PUT /api/v1/admin/platform-ops/{op}
pub async fn update_platform_operation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(op): Path<String>,
    Json(body): Json<UpdatePlatformOperationRequest>,
) -> AppResult<Json<AdminPlatformOperationResponse>> {
    // Deliberately ungated: administrators can stage per-operation configuration
    // while the caller-facing platform-services feature flag remains disabled.
    require_admin(&state, &auth_user).await?;
    let op = platform_operation_service::parse_operation_name(&op)?;
    let operation = platform_operation_service::upsert_operation_with_billing(
        &state.db,
        op,
        body.enabled,
        body.vendor_service_slug,
        body.config,
        body.billing.map(Into::into),
        &auth_user.user_id.to_string(),
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_platform_operation_updated",
        Some(serde_json::json!({
            "op": platform_operation_service::operation_name(op),
            "enabled": operation.enabled,
            "vendor_service_slug": operation.vendor_service_slug,
        })),
    );

    let operation_row =
        platform_operation_service::load_operation_row(&state.db, &operation.id).await?;
    state.billing.sync_operation_price(&operation_row).await?;
    let operation = platform_operation_service::list_configured_operations(&state.db)
        .await?
        .into_iter()
        .find(|row| row.op == op)
        .ok_or_else(|| {
            crate::errors::AppError::Internal(
                "Platform operation disappeared after price synchronization".to_string(),
            )
        })?;
    let vendor = state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .find_one(doc! { "slug": &operation.vendor_service_slug, "is_active": true })
        .await?;
    Ok(Json(platform_operation_response(
        op,
        Some(operation),
        vendor.as_ref(),
    )))
}

pub(crate) fn platform_operation_response(
    op: PlatformOperationName,
    operation: Option<PlatformOperation>,
    vendor: Option<&crate::models::downstream_service::DownstreamService>,
) -> AdminPlatformOperationResponse {
    let billing = operation
        .as_ref()
        .map(|operation| operation.billing.clone())
        .unwrap_or_else(|| platform_operation_service::default_operation_billing(op));
    let billable = billing.price_per_unit != "0"
        || billing.secondary.is_some()
        || billing.base_fee_per_call.is_some();
    let secondary = billing.secondary.as_ref().map(|component| {
        AdminPlatformOperationPricingComponentResponse {
            metric: component.metric.as_str(),
            price_per_unit: component.price_per_unit.clone(),
            lago_metric_code: component.lago_metric_code.clone(),
        }
    });
    let pricing = AdminPlatformOperationPricingResponse {
        billable,
        metric: billing.metric.as_str(),
        display: crate::services::billing::pricing::format_operation_price(&billing, billable),
        price_per_unit: billing.price_per_unit,
        secondary,
        base_fee_per_call: billing.base_fee_per_call,
        lago_metric_code: billing.lago_metric_code,
        sync_status: billing.sync_status,
        sync_error: billing.sync_error,
    };
    let vendor_service_id = vendor.map(|service| service.id.clone());
    match operation {
        Some(operation) => AdminPlatformOperationResponse {
            op: platform_operation_service::operation_name(op),
            enabled: operation.enabled,
            vendor_service_slug: operation.vendor_service_slug,
            config: operation.config.into(),
            updated_at: Some(operation.updated_at.to_rfc3339()),
            updated_by: Some(operation.updated_by),
            vendor_service_id,
            pricing,
        },
        None => AdminPlatformOperationResponse {
            op: platform_operation_service::operation_name(op),
            enabled: false,
            vendor_service_slug: platform_operation_service::default_vendor_service_slug(op)
                .to_string(),
            config: platform_operation_service::default_operation_config(op).into(),
            updated_at: None,
            updated_by: None,
            vendor_service_id,
            pricing,
        },
    }
}

impl From<PlatformOperationConfig> for AdminPlatformOperationConfigResponse {
    fn from(config: PlatformOperationConfig) -> Self {
        match config {
            PlatformOperationConfig::Speak(SpeakConfig {
                allowed_voice_ids,
                max_chars,
                model_id,
                max_calls_per_user_per_day,
            }) => Self::Speak(AdminSpeakConfigResponse {
                allowed_voice_ids,
                max_chars,
                model_id,
                max_calls_per_user_per_day,
            }),
            PlatformOperationConfig::CallAndSay(CallAndSayConfig {
                allowed_destination_prefixes,
                max_message_chars,
                max_duration_seconds,
                voice,
                max_calls_per_user_per_day,
                account_sid,
                call_from,
            }) => Self::CallAndSay(AdminCallAndSayConfigResponse {
                allowed_destination_prefixes,
                max_message_chars,
                max_duration_seconds,
                voice,
                max_calls_per_user_per_day,
                account_sid,
                call_from,
            }),
            PlatformOperationConfig::FlightSearch(FlightSearchConfig {
                max_offers_cap,
                max_searches_per_user_per_day,
            }) => Self::FlightSearch(AdminFlightSearchConfigResponse {
                max_offers_cap,
                max_searches_per_user_per_day,
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::platform_operation::{
        COLLECTION_NAME as PLATFORM_OPERATIONS, PlatformOperationRow,
    };

    #[test]
    fn missing_rows_render_as_disabled_defaults() {
        let response = platform_operation_response(PlatformOperationName::Speak, None, None);
        assert_eq!(response.op, "speak");
        assert!(!response.enabled);
        assert_eq!(response.vendor_service_slug, "api-elevenlabs");
        assert!(response.updated_at.is_none());
        assert!(matches!(
            response.config,
            AdminPlatformOperationConfigResponse::Speak(AdminSpeakConfigResponse {
                allowed_voice_ids,
                max_chars: 1_000,
                ..
            }) if allowed_voice_ids.is_empty()
        ));
    }

    #[test]
    fn update_request_rejects_unknown_fields() {
        let value = serde_json::json!({
            "enabled": false,
            "vendor_service_slug": "api-elevenlabs",
            "config": {
                "type": "speak",
                "allowed_voice_ids": ["voice-a"],
                "max_chars": 100,
                "model_id": "eleven_multilingual_v2"
            },
            "vendor_body": { "query": "caller controlled" },
        });
        assert!(serde_json::from_value::<UpdatePlatformOperationRequest>(value).is_err());
    }

    #[tokio::test]
    async fn upsert_persists_validated_typed_config() {
        let Some(db) = crate::test_utils::connect_test_database("admin_platform_ops_upsert").await
        else {
            eprintln!("skipping admin platform operation test: no local MongoDB available");
            return;
        };

        let encryption_keys = crate::test_utils::test_encryption_keys();
        let mut vendor = crate::models::downstream_service::test_helpers::dummy_service();
        vendor.id = uuid::Uuid::new_v4().to_string();
        vendor.slug = "api-elevenlabs".to_string();
        vendor.base_url = "https://api.elevenlabs.io".to_string();
        vendor.auth_method = "header".to_string();
        vendor.auth_key_name = "xi-api-key".to_string();
        vendor.credential_encrypted = Vec::new();
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(&vendor)
        .await
        .expect("insert catalog provider row");

        let operation = platform_operation_service::upsert_operation(
            &db,
            &encryption_keys,
            PlatformOperationName::Speak,
            true,
            "api-elevenlabs".to_string(),
            PlatformOperationConfig::Speak(SpeakConfig {
                allowed_voice_ids: vec!["voice-a".to_string()],
                max_chars: 1_200,
                model_id: "eleven_multilingual_v2".to_string(),
                max_calls_per_user_per_day: 50,
            }),
            "admin-user",
        )
        .await
        .expect("upsert platform operation");

        assert!(operation.enabled);
        assert_eq!(operation.updated_by, "admin-user");
        assert_eq!(
            operation.config,
            PlatformOperationConfig::Speak(SpeakConfig {
                allowed_voice_ids: vec!["voice-a".to_string()],
                max_chars: 1_200,
                model_id: "eleven_multilingual_v2".to_string(),
                max_calls_per_user_per_day: 50,
            })
        );
        assert_eq!(
            db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
                .count_documents(mongodb::bson::doc! {
                    "catalog_service_id": &vendor.id,
                    "kind_key": "constrained:speak",
                })
                .await
                .expect("count operation rows"),
            1
        );
    }
}
