use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::AppResult;
use crate::handlers::admin_helpers::require_admin;
use crate::models::platform_operation::{
    CallAndSayConfig, PlatformOperation, PlatformOperationConfig, PlatformOperationName,
    SpeakConfig, XSearchConfig,
};
use crate::models::platform_vendor_template::PlatformVendorTemplate;
use crate::mw::auth::AuthUser;
use crate::services::{
    audit_service, platform_operation_service, platform_vendor_template_service,
};

#[derive(Debug, Serialize)]
pub struct AdminPlatformOperationListResponse {
    pub operations: Vec<AdminPlatformOperationResponse>,
}

#[derive(Debug, Serialize)]
pub struct AdminPlatformVendorRequirementListResponse {
    pub vendors: Vec<AdminPlatformVendorRequirementResponse>,
}

#[derive(Debug, Serialize)]
pub struct AdminPlatformVendorRequirementResponse {
    pub id: String,
    pub vendor: String,
    pub display_name: String,
    pub operation: Option<String>,
    pub slug: String,
    pub base_url: String,
    pub auth_method: String,
    pub auth_key_name: Option<String>,
    pub service_category: String,
    pub visibility: String,
    pub credential_label: String,
    pub credential_note: String,
    pub capability_summary: String,
    pub restriction_summary: String,
    pub is_active: bool,
    pub is_seeded: bool,
    pub existing_service: Option<AdminPlatformVendorServiceResponse>,
}

#[derive(Debug, Serialize)]
pub struct AdminPlatformVendorTemplateListResponse {
    pub vendors: Vec<AdminPlatformVendorRequirementResponse>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformVendorTemplateRequest {
    pub vendor: String,
    pub display_name: String,
    pub slug: String,
    pub base_url: String,
    pub auth_method: String,
    #[serde(default)]
    pub auth_key_name: Option<String>,
    pub credential_label: String,
    pub credential_note: String,
    #[serde(default)]
    pub operation: Option<String>,
    pub capability_summary: String,
    pub restriction_summary: String,
    #[serde(default = "default_template_active")]
    pub is_active: bool,
}

fn default_template_active() -> bool {
    true
}

#[derive(Debug, Serialize)]
pub struct AdminPlatformVendorServiceResponse {
    pub id: String,
    pub name: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub service_category: String,
    pub visibility: String,
    pub is_active: bool,
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

/// GET /api/v1/admin/platform-ops/vendor-requirements
pub async fn get_vendor_requirements(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<AdminPlatformVendorRequirementListResponse>> {
    require_admin(&state, &auth_user).await?;
    let templates = platform_vendor_template_service::list_templates(&state.db, false).await?;
    let services = list_active_vendor_services(&state.db, &templates).await?;
    Ok(Json(vendor_requirements_response(&templates, &services)))
}

/// GET /api/v1/admin/platform-ops/vendor-templates
pub async fn list_vendor_templates(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<AdminPlatformVendorTemplateListResponse>> {
    require_admin(&state, &auth_user).await?;
    let templates = platform_vendor_template_service::list_templates(&state.db, true).await?;
    let services = list_active_vendor_services(&state.db, &templates).await?;
    Ok(Json(AdminPlatformVendorTemplateListResponse {
        vendors: vendor_requirements_response(&templates, &services).vendors,
    }))
}

/// POST /api/v1/admin/platform-ops/vendor-templates
pub async fn create_vendor_template(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<PlatformVendorTemplateRequest>,
) -> AppResult<Json<AdminPlatformVendorRequirementResponse>> {
    require_admin(&state, &auth_user).await?;
    let template = platform_vendor_template_service::create_template(
        &state.db,
        body.into(),
        &auth_user.user_id.to_string(),
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_platform_vendor_template_created",
        Some(serde_json::json!({ "vendor": template.vendor, "slug": template.slug })),
    );
    Ok(Json(vendor_template_response(&template, None)))
}

/// PUT /api/v1/admin/platform-ops/vendor-templates/{template_id}
pub async fn update_vendor_template(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(template_id): Path<String>,
    Json(body): Json<PlatformVendorTemplateRequest>,
) -> AppResult<Json<AdminPlatformVendorRequirementResponse>> {
    require_admin(&state, &auth_user).await?;
    let template = platform_vendor_template_service::update_template(
        &state.db,
        &template_id,
        body.into(),
        &auth_user.user_id.to_string(),
    )
    .await?;
    let services = list_active_vendor_services(&state.db, std::slice::from_ref(&template)).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_platform_vendor_template_updated",
        Some(serde_json::json!({ "vendor": template.vendor, "slug": template.slug })),
    );
    Ok(Json(vendor_template_response(
        &template,
        services
            .iter()
            .find(|service| service.slug == template.slug),
    )))
}

/// DELETE /api/v1/admin/platform-ops/vendor-templates/{template_id}
pub async fn disable_vendor_template(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(template_id): Path<String>,
) -> AppResult<StatusCode> {
    require_admin(&state, &auth_user).await?;
    platform_vendor_template_service::disable_template(
        &state.db,
        &template_id,
        &auth_user.user_id.to_string(),
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_platform_vendor_template_disabled",
        Some(serde_json::json!({ "template_id": template_id })),
    );
    Ok(StatusCode::NO_CONTENT)
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

    Ok(Json(platform_operation_response(op, Some(operation))))
}

async fn list_active_vendor_services(
    db: &mongodb::Database,
    templates: &[PlatformVendorTemplate],
) -> AppResult<Vec<crate::models::downstream_service::DownstreamService>> {
    let slugs = templates
        .iter()
        .map(|template| template.slug.as_str())
        .collect::<Vec<_>>();
    if slugs.is_empty() {
        return Ok(Vec::new());
    }
    db.collection::<crate::models::downstream_service::DownstreamService>(
        crate::models::downstream_service::COLLECTION_NAME,
    )
    .find(doc! { "slug": { "$in": slugs }, "is_active": true })
    .await?
    .try_collect()
    .await
    .map_err(crate::errors::AppError::DatabaseError)
}

fn vendor_requirements_response(
    templates: &[PlatformVendorTemplate],
    services: &[crate::models::downstream_service::DownstreamService],
) -> AdminPlatformVendorRequirementListResponse {
    let vendors = templates
        .iter()
        .map(|template| {
            vendor_template_response(
                template,
                services
                    .iter()
                    .find(|service| service.slug == template.slug),
            )
        })
        .collect();
    AdminPlatformVendorRequirementListResponse { vendors }
}

fn vendor_template_response(
    template: &PlatformVendorTemplate,
    service: Option<&crate::models::downstream_service::DownstreamService>,
) -> AdminPlatformVendorRequirementResponse {
    AdminPlatformVendorRequirementResponse {
        id: template.id.clone(),
        vendor: template.vendor.clone(),
        display_name: template.display_name.clone(),
        operation: template.operation.clone(),
        slug: template.slug.clone(),
        base_url: template.base_url.clone(),
        auth_method: template.auth_method.clone(),
        auth_key_name: template.auth_key_name.clone(),
        service_category: "internal".to_string(),
        visibility: "public".to_string(),
        credential_label: template.credential_label.clone(),
        credential_note: template.credential_note.clone(),
        capability_summary: template.capability_summary.clone(),
        restriction_summary: template.restriction_summary.clone(),
        is_active: template.is_active,
        is_seeded: template.is_seeded,
        existing_service: service.map(|service| AdminPlatformVendorServiceResponse {
            id: service.id.clone(),
            name: service.name.clone(),
            auth_method: service.auth_method.clone(),
            auth_key_name: service.auth_key_name.clone(),
            service_category: service.service_category.clone(),
            visibility: service.visibility.clone(),
            is_active: service.is_active,
        }),
    }
}

impl From<PlatformVendorTemplateRequest>
    for platform_vendor_template_service::PlatformVendorTemplateInput
{
    fn from(request: PlatformVendorTemplateRequest) -> Self {
        Self {
            vendor: request.vendor,
            display_name: request.display_name,
            slug: request.slug,
            base_url: request.base_url,
            auth_method: request.auth_method,
            auth_key_name: request.auth_key_name,
            credential_label: request.credential_label,
            credential_note: request.credential_note,
            operation: request.operation,
            capability_summary: request.capability_summary,
            restriction_summary: request.restriction_summary,
            is_active: request.is_active,
        }
    }
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
    use crate::services::platform_operation_service::DEFAULT_PLATFORM_VENDOR_TEMPLATES;

    fn seeded_templates() -> Vec<PlatformVendorTemplate> {
        let now = chrono::Utc::now();
        DEFAULT_PLATFORM_VENDOR_TEMPLATES
            .iter()
            .enumerate()
            .map(|(index, seed)| PlatformVendorTemplate {
                id: format!("template-{index}"),
                vendor: seed.vendor.to_string(),
                display_name: seed.display_name.to_string(),
                slug: seed.slug.to_string(),
                base_url: seed.base_url.to_string(),
                auth_method: seed.auth_method.to_string(),
                auth_key_name: seed.auth_key_name.map(str::to_string),
                credential_label: seed.credential_label.to_string(),
                credential_note: seed.credential_note.to_string(),
                operation: seed.operation.map(str::to_string),
                capability_summary: seed.capability_summary.to_string(),
                restriction_summary: seed.restriction_summary.to_string(),
                is_active: true,
                is_seeded: true,
                created_at: now,
                updated_at: now,
                updated_by: "system".to_string(),
            })
            .collect()
    }

    #[test]
    fn vendor_requirements_response_exposes_the_provisioning_contract() {
        let response = vendor_requirements_response(&seeded_templates(), &[]);
        assert_eq!(response.vendors.len(), 4);

        let elevenlabs = response
            .vendors
            .iter()
            .find(|vendor| vendor.vendor == "elevenlabs")
            .expect("ElevenLabs requirement");
        assert_eq!(elevenlabs.operation.as_deref(), Some("speak"));
        assert_eq!(elevenlabs.slug, "platform-elevenlabs");
        assert_eq!(elevenlabs.base_url, "https://api.elevenlabs.io");
        assert_eq!(elevenlabs.auth_method, "header");
        assert_eq!(elevenlabs.auth_key_name.as_deref(), Some("xi-api-key"));
        assert_eq!(elevenlabs.service_category, "internal");
        assert_eq!(elevenlabs.visibility, "public");

        let duffel = response
            .vendors
            .iter()
            .find(|vendor| vendor.vendor == "duffel")
            .expect("Duffel requirement");
        assert_eq!(duffel.operation, None);
        assert_eq!(duffel.slug, "platform-duffel");
        assert_eq!(duffel.auth_method, "bearer");
    }

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

        let encryption_keys = crate::test_utils::test_encryption_keys();
        let mut vendor = crate::models::downstream_service::test_helpers::dummy_service();
        vendor.id = uuid::Uuid::new_v4().to_string();
        vendor.slug = "platform-x".to_string();
        vendor.base_url = "https://api.x.com".to_string();
        vendor.auth_method = "bearer".to_string();
        vendor.auth_key_name = "Authorization".to_string();
        vendor.service_category = "internal".to_string();
        vendor.visibility = "public".to_string();
        vendor.requires_user_credential = false;
        vendor.credential_encrypted = encryption_keys
            .encrypt(b"x-bearer-token")
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
