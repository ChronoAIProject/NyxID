mod contracts;
pub use contracts::certified_contracts;
pub mod credentials;
// Admin-only private configuration and an allowlisted public metadata view.
use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};

use super::admin_helpers::{require_admin, require_admin_or_operator};
use crate::{
    AppState,
    errors::AppResult,
    models::platform_model::*,
    mw::auth::{AuthMethod, AuthUser},
    services::{audit_service::AuditActor, platform_model_service as registry},
};

#[derive(Serialize)]
pub struct PublicModel {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub capabilities: Capabilities,
    pub availability: &'static str,
    pub retail_prices: Vec<RetailPrice>,
}
impl From<&PlatformModel> for PublicModel {
    fn from(row: &PlatformModel) -> Self {
        Self {
            id: row.public_id.clone(),
            display_name: row.display_name.clone(),
            description: row.description.clone(),
            capabilities: row.capabilities.clone(),
            availability: "unavailable",
            retail_prices: vec![],
        }
    }
}
/// Explicit public economics, never the private tariff, billing binding or route.
#[derive(Serialize)]
pub struct RetailPrice {
    pub variant: std::collections::BTreeMap<String, String>,
    pub available: bool,
    pub components: Vec<RetailComponent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_input: Option<crate::services::platform_model_execution::images::PublicLimits>,
}
#[derive(Serialize)]
pub struct RetailComponent {
    pub metric: crate::models::service_billing::BillingMetric,
    pub selector: crate::models::platform_model_tariff::Selector,
    pub credits_per_unit: String,
    pub explicit_free: bool,
}
impl From<crate::services::billing::model_tariffs::Explanation> for RetailPrice {
    fn from(price: crate::services::billing::model_tariffs::Explanation) -> Self {
        Self {
            variant: price.variant,
            available: price.available,
            image_input: price.image_input,
            components: if price.available {
                price
                    .components
                    .into_iter()
                    .map(|c| RetailComponent {
                        metric: c.metric,
                        selector: c.selector,
                        credits_per_unit: c.credits_per_unit,
                        explicit_free: c.explicit_free,
                    })
                    .collect()
            } else {
                vec![]
            },
        }
    }
}
#[derive(Serialize)]
pub struct AdminModel {
    pub id: String,
    pub public_id: String,
    pub display_name: String,
    pub description: String,
    pub family: ModelFamily,
    pub capabilities: Capabilities,
    pub lifecycle: Lifecycle,
    pub enabled: bool,
    pub revision: i64,
    pub authorization_revision: i64,
    pub billing_service_id: Option<String>,
    pub entitlements: Entitlements,
    pub routing_revision_id: Option<String>,
    pub routes: Vec<ModelRoute>,
    pub execution_ready: bool,
    pub blocking_reason: &'static str,
    pub required_readiness: Vec<&'static str>,
    pub created_at: String,
    pub updated_at: String,
}
async fn admin_view(state: &AppState, row: PlatformModel) -> AppResult<AdminModel> {
    let routing = registry::routing(&state.db, &row).await?;
    let execution_ready = match registry::require_execution_ready(&state.db).await {
        Ok(()) => true,
        Err(crate::errors::AppError::SecurityReadinessUnavailable) => false,
        Err(error) => return Err(error),
    };
    let mut required_readiness = vec![
        registry::REGISTRY_READINESS,
        registry::EXECUTION_READINESS,
        registry::BILLING_READINESS,
    ];
    let contracts = crate::services::billing::model_tariffs::list(&state.db, &row.id)
        .await?
        .iter()
        .any(|t| {
            t.status != crate::models::platform_model_tariff::TariffStatus::Retired
                && crate::services::platform_model_execution::certification::family_contract(
                    &t.contract,
                )
        });
    let execution_ready = if contracts {
        let capability = crate::services::platform_model_execution::certification::READINESS;
        required_readiness.push(capability);
        execution_ready
            && match crate::services::security_readiness_service::report(&state.db, capability)
                .await
            {
                Ok(report) => report.ready,
                Err(crate::errors::AppError::SecurityReadinessUnavailable) => false,
                Err(error) => return Err(error),
            }
    } else {
        execution_ready
    };
    Ok(AdminModel {
        id: row.id,
        public_id: row.public_id,
        display_name: row.display_name,
        description: row.description,
        family: row.family,
        capabilities: row.capabilities,
        lifecycle: row.lifecycle,
        enabled: row.enabled,
        revision: row.revision,
        authorization_revision: row.authorization_revision,
        billing_service_id: row.billing_service_id,
        entitlements: row.entitlements,
        routing_revision_id: row.routing_revision_id,
        routes: routing.map_or_else(Vec::new, |r| r.routes),
        execution_ready,
        blocking_reason: if execution_ready {
            ""
        } else {
            "execution_fleet_not_ready"
        },
        required_readiness,
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    })
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub after: Option<String>,
    pub limit: Option<u32>,
}
#[derive(Serialize)]
pub struct PublicList {
    pub models: Vec<PublicModel>,
    pub next_after: Option<String>,
}
#[derive(Serialize)]
pub struct AdminList {
    pub models: Vec<AdminModel>,
    pub next_after: Option<String>,
}

pub async fn admin_list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<AdminList>> {
    super::login_client_context::require_first_party_human(&auth)?;
    require_admin_or_operator(&state, &auth, "admin.platform_models.list").await?;
    let limit = query.limit.unwrap_or(50);
    let rows = registry::list(&state.db, query.after.as_deref(), limit).await?;
    let next_after = (rows.len() == limit as usize).then(|| rows.last().unwrap().public_id.clone());
    let mut models = Vec::with_capacity(rows.len());
    for row in rows {
        models.push(admin_view(&state, row).await?);
    }
    Ok(Json(AdminList { models, next_after }))
}
pub async fn admin_get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<AdminModel>> {
    super::login_client_context::require_first_party_human(&auth)?;
    require_admin_or_operator(&state, &auth, "admin.platform_models.get").await?;
    Ok(Json(
        admin_view(&state, registry::get(&state.db, &id).await?).await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub public_id: String,
    pub display_name: String,
    #[serde(default)]
    pub description: String,
    pub family: ModelFamily,
    #[serde(default)]
    pub capabilities: Capabilities,
}
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateRequest>,
) -> AppResult<Json<AdminModel>> {
    super::login_client_context::require_first_party_human(&auth)?;
    require_admin(&state, &auth).await?;
    let model = registry::create(
        &state.db,
        &AuditActor::from_auth_user(&auth),
        registry::NewModel {
            public_id: body.public_id,
            display_name: body.display_name,
            description: body.description,
            family: body.family,
            capabilities: body.capabilities,
        },
    )
    .await?;
    Ok(Json(admin_view(&state, model).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdateRequest {
    pub expected_revision: i64,
    pub display_name: String,
    pub description: String,
    pub capabilities: Capabilities,
    pub lifecycle: Lifecycle,
    pub enabled: bool,
    pub billing_service_id: Option<String>,
}
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateRequest>,
) -> AppResult<Json<AdminModel>> {
    super::login_client_context::require_first_party_human(&auth)?;
    require_admin(&state, &auth).await?;
    let model = registry::update(
        &state.db,
        &AuditActor::from_auth_user(&auth),
        &id,
        registry::ModelUpdate {
            expected_revision: body.expected_revision,
            display_name: body.display_name,
            description: body.description,
            capabilities: body.capabilities,
            lifecycle: body.lifecycle,
            enabled: body.enabled,
            billing_service_id: body.billing_service_id,
        },
    )
    .await?;
    Ok(Json(admin_view(&state, model).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntitlementRequest {
    pub expected_revision: i64,
    pub entitlements: Entitlements,
}
pub async fn entitlements(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<EntitlementRequest>,
) -> AppResult<Json<AdminModel>> {
    super::login_client_context::require_first_party_human(&auth)?;
    require_admin(&state, &auth).await?;
    let model = registry::set_entitlements(
        &state.db,
        &AuditActor::from_auth_user(&auth),
        &id,
        body.expected_revision,
        body.entitlements,
    )
    .await?;
    Ok(Json(admin_view(&state, model).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoutesRequest {
    pub expected_revision: i64,
    pub routes: Vec<ModelRoute>,
}
pub async fn routes(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<RoutesRequest>,
) -> AppResult<Json<AdminModel>> {
    super::login_client_context::require_first_party_human(&auth)?;
    require_admin(&state, &auth).await?;
    let model = registry::set_routes(
        &state.db,
        &AuditActor::from_auth_user(&auth),
        &id,
        body.expected_revision,
        body.routes,
    )
    .await?;
    Ok(Json(admin_view(&state, model).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExplainQuery {
    pub user_id: String,
}
#[derive(Serialize)]
pub struct AccessExplanation {
    pub model_id: String,
    pub user_id: String,
    pub revision: i64,
    pub policy_reason: &'static str,
    pub model_reason: &'static str,
    pub flag_enabled: bool,
    pub executable: bool,
    pub route_count: Option<usize>,
    pub route_revision_checked: Option<i64>,
}
pub async fn explain(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<ExplainQuery>,
) -> AppResult<Json<AccessExplanation>> {
    super::login_client_context::require_first_party_human(&auth)?;
    require_admin_or_operator(&state, &auth, "admin.platform_models.explain").await?;
    let model = registry::get(&state.db, &id).await?;
    let snapshot = registry::EntitlementSnapshot::load(&state.db, &query.user_id).await?;
    let flag_enabled = crate::services::feature_flag_service::personal_flag_enabled(
        &state.db,
        &query.user_id,
        registry::FLAG,
    )
    .await?;
    let plan = if flag_enabled && snapshot.reason(&model) == "allowed" {
        match registry::resolve_route_metadata(&state.db, &query.user_id, &model.public_id, None)
            .await
        {
            Ok(plan) => Some(plan),
            Err(crate::errors::AppError::NotFound(_)) => None,
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    Ok(Json(AccessExplanation {
        route_count: plan.as_ref().map(|p| p.route_count()),
        route_revision_checked: plan.as_ref().map(|p| p.model_revision()),
        model_id: model.id.clone(),
        user_id: query.user_id,
        revision: model.revision,
        policy_reason: snapshot.policy_reason(&model.entitlements),
        model_reason: snapshot.reason(&model),
        flag_enabled,
        executable: false,
    }))
}

pub async fn public_list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<PublicList>> {
    auth.ensure_llm_proxy_access()?;
    // PR 5 supplies the paired actor-bound contract for internal consumers;
    // a synthetic service-account/relay identity is never a person entitlement.
    if !matches!(
        auth.auth_method,
        AuthMethod::Session | AuthMethod::AccessToken | AuthMethod::ApiKey
    ) {
        return Err(registry::unavailable());
    }
    let person = auth.user_id.to_string();
    if !crate::services::feature_flag_service::personal_flag_enabled(
        &state.db,
        &person,
        registry::FLAG,
    )
    .await?
    {
        return Ok(Json(PublicList {
            models: vec![],
            next_after: None,
        }));
    }
    let snapshot = registry::EntitlementSnapshot::load(&state.db, &person).await?;
    let limit = query.limit.unwrap_or(50);
    let catalog_scope = registry::authorized_catalog_scope(
        &state.db,
        &snapshot,
        auth.allow_all_services,
        &auth.allowed_service_ids,
        &auth.assistant_operation_scopes,
    )
    .await?;
    let mut rows = registry::list_authorized(
        &state.db,
        &snapshot,
        catalog_scope.as_ref(),
        query.after.as_deref(),
        limit,
    )
    .await?;
    let more = rows.len() > limit as usize;
    rows.truncate(limit as usize);
    let next_after = more.then(|| rows.last().unwrap().public_id.clone());
    let mut prices = registry::retail_explanations(
        &state.db,
        &rows,
        state.config.billing_rate_cache_ttl_secs,
        state.billing.billing_enabled(),
    )
    .await?;
    let ready = match registry::require_execution_ready(&state.db).await {
        Ok(()) => true,
        Err(crate::errors::AppError::SecurityReadinessUnavailable) => false,
        Err(error) => return Err(error),
    };
    Ok(Json(PublicList {
        models: rows
            .iter()
            .map(|row| {
                let mut model = PublicModel::from(row);
                model.retail_prices = prices
                    .remove(&row.id)
                    .unwrap_or_default()
                    .into_iter()
                    .map(|mut price| {
                        if !ready
                            || auth.api_key_purpose
                                != crate::models::api_key::ApiKeyPurpose::General
                            || auth.assistant_chat.is_some()
                            || auth.org_agent_access.is_some()
                            || auth.execution_permission.is_some()
                        {
                            price.image_input = None;
                        }
                        if let Some(limits) = &mut price.image_input {
                            limits.max_request_bytes =
                                limits.max_request_bytes.min(state.config.llm_max_body_size);
                        }
                        price.into()
                    })
                    .collect();
                if ready {
                    model.availability = "request_dependent";
                }
                model
            })
            .collect(),
        next_after,
    }))
}

/// Private admin projection; customer readers never receive verification sources.
#[derive(Serialize)]
pub struct TariffResponse {
    pub id: String,
    pub revision: i64,
    pub service_id: String,
    pub model_id: String,
    pub public_model: String,
    pub lane: crate::models::platform_model_tariff::TariffLane,
    pub variant: std::collections::BTreeMap<String, String>,
    pub contract: crate::models::platform_model_tariff::UsageContract,
    pub bounds: crate::models::platform_model_tariff::RequestBounds,
    pub components: Vec<crate::models::platform_model_tariff::TariffComponent>,
    pub source: String,
    pub status: crate::models::platform_model_tariff::TariffStatus,
    pub sync_error: Option<String>,
    pub created_at: String,
}
impl From<crate::models::platform_model_tariff::ModelTariff> for TariffResponse {
    fn from(t: crate::models::platform_model_tariff::ModelTariff) -> Self {
        Self {
            id: t.id,
            revision: t.revision,
            service_id: t.service_id,
            model_id: t.model_id,
            public_model: t.public_model,
            lane: t.lane,
            variant: t.variant,
            contract: t.contract,
            bounds: t.bounds,
            components: t.components,
            source: t.source,
            status: t.status,
            sync_error: t.sync_error,
            created_at: t.created_at.to_rfc3339(),
        }
    }
}
pub async fn tariffs(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Vec<TariffResponse>>> {
    super::login_client_context::require_first_party_human(&auth)?;
    require_admin_or_operator(&state, &auth, "admin.platform_models.tariffs").await?;
    registry::get(&state.db, &id).await?;
    Ok(Json(
        crate::services::billing::model_tariffs::list(&state.db, &id)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
pub async fn create_tariff(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<crate::services::billing::model_tariffs::NewTariff>,
) -> AppResult<Json<TariffResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    require_admin(&state, &auth).await?;
    let t = crate::services::billing::model_tariffs::create(
        &state.db,
        &AuditActor::from_auth_user(&auth),
        &id,
        body,
    )
    .await?;
    // Durable pending state is returned even if Lago is down. Reconcile, or
    // the existing admin billing resync route, publishes the complete revision.
    Ok(Json(t.into()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetireTariffRequest {
    pub revision: i64,
}
pub async fn retire_tariff(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, tariff_id)): Path<(String, String)>,
    Json(body): Json<RetireTariffRequest>,
) -> AppResult<axum::http::StatusCode> {
    super::login_client_context::require_first_party_human(&auth)?;
    require_admin(&state, &auth).await?;
    crate::services::billing::model_tariffs::retire(
        &state.db,
        &AuditActor::from_auth_user(&auth),
        &id,
        &tariff_id,
        body.revision,
    )
    .await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests;
