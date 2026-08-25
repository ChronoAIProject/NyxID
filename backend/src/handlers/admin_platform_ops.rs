use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::AppResult;
use crate::handlers::admin_helpers::require_admin;
use crate::models::platform_operation::{
    CallAndSayConfig, PlatformOperation, PlatformOperationConfig, PlatformOperationName,
    SpeakConfig, XSearchConfig,
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
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AdminPlatformOperationConfigResponse {
    XSearch(AdminXSearchConfigResponse),
    Speak(AdminSpeakConfigResponse),
    CallAndSay(AdminCallAndSayConfigResponse),
}

#[derive(Debug, Serialize)]
pub struct AdminXSearchConfigResponse {
    pub max_results_cap: u32,
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
    let operations = platform_operation_service::PLATFORM_OPERATION_NAMES
        .into_iter()
        .map(|op| {
            let row = configured.iter().find(|row| row.op == op).cloned();
            platform_operation_response(op, row)
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

    Ok(Json(platform_operation_response(op, Some(operation))))
}

fn platform_operation_response(
    op: PlatformOperationName,
    operation: Option<PlatformOperation>,
) -> AdminPlatformOperationResponse {
    match operation {
        Some(operation) => AdminPlatformOperationResponse {
            op: platform_operation_service::operation_name(op),
            enabled: operation.enabled,
            vendor_service_slug: operation.vendor_service_slug,
            config: operation.config.into(),
            updated_at: Some(operation.updated_at.to_rfc3339()),
            updated_by: Some(operation.updated_by),
        },
        None => AdminPlatformOperationResponse {
            op: platform_operation_service::operation_name(op),
            enabled: false,
            vendor_service_slug: platform_operation_service::default_vendor_service_slug(op)
                .to_string(),
            config: platform_operation_service::default_operation_config(op).into(),
            updated_at: None,
            updated_by: None,
        },
    }
}

impl From<PlatformOperationConfig> for AdminPlatformOperationConfigResponse {
    fn from(config: PlatformOperationConfig) -> Self {
        match config {
            PlatformOperationConfig::XSearch(XSearchConfig { max_results_cap }) => {
                Self::XSearch(AdminXSearchConfigResponse { max_results_cap })
            }
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
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::platform_operation::COLLECTION_NAME as PLATFORM_OPERATIONS;

    #[test]
    fn missing_rows_render_as_disabled_defaults() {
        let response = platform_operation_response(PlatformOperationName::XSearch, None);
        assert_eq!(response.op, "x_search");
        assert!(!response.enabled);
        assert_eq!(response.vendor_service_slug, "platform-x");
        assert!(response.updated_at.is_none());
        assert!(matches!(
            response.config,
            AdminPlatformOperationConfigResponse::XSearch(AdminXSearchConfigResponse {
                max_results_cap: 10
            })
        ));
    }

    #[test]
    fn update_request_rejects_unknown_fields() {
        let value = serde_json::json!({
            "enabled": false,
            "vendor_service_slug": "platform-x",
            "config": { "type": "x_search", "max_results_cap": 10 },
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

        let operation = platform_operation_service::upsert_operation(
            &db,
            PlatformOperationName::XSearch,
            true,
            "platform-x".to_string(),
            PlatformOperationConfig::XSearch(XSearchConfig {
                max_results_cap: 12,
            }),
            "admin-user",
        )
        .await
        .expect("upsert platform operation");

        assert!(operation.enabled);
        assert_eq!(operation.updated_by, "admin-user");
        assert_eq!(
            operation.config,
            PlatformOperationConfig::XSearch(XSearchConfig {
                max_results_cap: 12,
            })
        );
        assert_eq!(
            db.collection::<PlatformOperation>(PLATFORM_OPERATIONS)
                .count_documents(mongodb::bson::doc! { "op": "x_search" })
                .await
                .expect("count operation rows"),
            1
        );
    }
}
