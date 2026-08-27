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
    CallAndSayConfig, FlightSearchConfig, PlatformOperation, PlatformOperationConfig,
    PlatformOperationName, SpeakConfig,
};
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
    pub credits_per_call: Option<String>,
    pub metric: &'static str,
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
}

#[derive(Debug, Serialize)]
pub struct AdminCallAndSayConfigResponse {
    pub allowed_destination_prefixes: Vec<String>,
    pub max_message_chars: u32,
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
    let operation = platform_operation_service::upsert_operation(
        &state.db,
        &state.encryption_keys,
        op,
        body.enabled,
        body.vendor_service_slug,
        body.config,
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

fn platform_operation_response(
    op: PlatformOperationName,
    operation: Option<PlatformOperation>,
    vendor: Option<&crate::models::downstream_service::DownstreamService>,
) -> AdminPlatformOperationResponse {
    let pricing = AdminPlatformOperationPricingResponse {
        billable: vendor
            .and_then(|service| service.billing.as_ref())
            .is_some_and(|billing| billing.platform_billable),
        credits_per_call: vendor
            .and_then(|service| service.billing.as_ref())
            .and_then(|billing| billing.platform_pricing.as_ref())
            .map(|pricing| pricing.credits_per_unit.clone()),
        metric: "requests",
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
            }) => Self::Speak(AdminSpeakConfigResponse {
                allowed_voice_ids,
                max_chars,
                model_id,
            }),
            PlatformOperationConfig::CallAndSay(CallAndSayConfig {
                allowed_destination_prefixes,
                max_message_chars,
                voice,
                max_calls_per_user_per_day,
                account_sid,
                call_from,
            }) => Self::CallAndSay(AdminCallAndSayConfigResponse {
                allowed_destination_prefixes,
                max_message_chars,
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
    use crate::models::platform_operation::COLLECTION_NAME as PLATFORM_OPERATIONS;

    #[test]
    fn missing_rows_render_as_disabled_defaults() {
        let response = platform_operation_response(PlatformOperationName::Speak, None, None);
        assert_eq!(response.op, "speak");
        assert!(!response.enabled);
        assert_eq!(response.vendor_service_slug, "platform-elevenlabs");
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
            "vendor_service_slug": "platform-elevenlabs",
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
        vendor.slug = "platform-elevenlabs".to_string();
        vendor.base_url = "https://api.elevenlabs.io".to_string();
        vendor.auth_method = "header".to_string();
        vendor.auth_key_name = "xi-api-key".to_string();
        vendor.service_category = "internal".to_string();
        vendor.visibility = "public".to_string();
        vendor.requires_user_credential = false;
        vendor.credential_encrypted = encryption_keys
            .encrypt(b"elevenlabs-key")
            .await
            .expect("encrypt vendor credential");
        db.collection::<crate::models::downstream_service::DownstreamService>(
            crate::models::downstream_service::COLLECTION_NAME,
        )
        .insert_one(vendor)
        .await
        .expect("insert platform vendor row");

        let operation = platform_operation_service::upsert_operation(
            &db,
            &encryption_keys,
            PlatformOperationName::Speak,
            true,
            "platform-elevenlabs".to_string(),
            PlatformOperationConfig::Speak(SpeakConfig {
                allowed_voice_ids: vec!["voice-a".to_string()],
                max_chars: 1_200,
                model_id: "eleven_multilingual_v2".to_string(),
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
            })
        );
        assert_eq!(
            db.collection::<PlatformOperation>(PLATFORM_OPERATIONS)
                .count_documents(mongodb::bson::doc! { "op": "speak" })
                .await
                .expect("count operation rows"),
            1
        );
    }
}
