//! Admin-only authoring for the inactive v1 usage tariff family.
use super::super::admin_helpers::{require_admin, require_admin_or_operator};
use crate::{
    AppState,
    errors::AppResult,
    models::sandbox_usage_rates::{
        ChargeUnit, DimensionPolicy, LegacyCoverage, Selection, UsagePolicy, UsageScope,
        UsageTariff,
    },
    mw::auth::AuthUser,
    services::{
        audit_service::AuditActor,
        billing::sandbox_usage::{pricing_schema, tariff_store},
    },
};
use axum::{
    Json,
    extract::{Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TariffQuery {
    producer_namespace: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeQuery {
    producer_namespace: String,
    owner_id: String,
    sandbox_id: String,
    source_epoch: String,
}
impl From<ScopeQuery> for UsageScope {
    fn from(s: ScopeQuery) -> Self {
        Self {
            producer_namespace: s.producer_namespace,
            owner_id: s.owner_id,
            sandbox_id: s.sandbox_id,
            source_epoch: s.source_epoch,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TariffRequest {
    producer_namespace: String,
    expected_version: i64,
    effective_from: DateTime<Utc>,
    rates: BTreeMap<String, String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LegacyRequest {
    covered_until: DateTime<Utc>,
    manifest_digest: String,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DimensionsDto {
    cpu: Selection,
    memory: Selection,
    disk: Selection,
}
impl From<DimensionsDto> for DimensionPolicy {
    fn from(d: DimensionsDto) -> Self {
        Self {
            cpu: d.cpu,
            memory: d.memory,
            disk: d.disk,
        }
    }
}
impl From<DimensionPolicy> for DimensionsDto {
    fn from(d: DimensionPolicy) -> Self {
        Self {
            cpu: d.cpu,
            memory: d.memory,
            disk: d.disk,
        }
    }
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyRequest {
    scope: ScopeQuery,
    expected_version: i64,
    effective_from: DateTime<Utc>,
    dimensions: DimensionsDto,
    legacy_coverage: Option<LegacyRequest>,
}

#[derive(Serialize)]
pub struct RateResponse {
    metric_code: String,
    definition_version: String,
    charge_unit: ChargeUnit,
    credits_per_unit: String,
    provider_proof: Option<String>,
}
#[derive(Serialize)]
pub struct TariffResponse {
    id: String,
    schema_version: u32,
    producer_namespace: String,
    version: i64,
    effective_from: DateTime<Utc>,
    rates: Vec<RateResponse>,
    shadow_only: bool,
}
impl From<UsageTariff> for TariffResponse {
    fn from(t: UsageTariff) -> Self {
        Self {
            id: t.id,
            schema_version: t.schema_version,
            producer_namespace: t.producer_namespace,
            version: t.version,
            effective_from: t.effective_from,
            rates: t
                .rates
                .into_iter()
                .map(|r| RateResponse {
                    metric_code: r.metric_code,
                    definition_version: r.definition_version,
                    charge_unit: r.charge_unit,
                    credits_per_unit: r.credits_per_unit.to_string(),
                    provider_proof: r.provider_proof,
                })
                .collect(),
            shadow_only: true,
        }
    }
}
#[derive(Serialize)]
pub struct PolicyResponse {
    id: String,
    schema_version: u32,
    version: i64,
    effective_from: DateTime<Utc>,
    dimensions: DimensionsDto,
    legacy_covered_until: Option<DateTime<Utc>>,
    legacy_manifest_digest: Option<String>,
    shadow_only: bool,
}
impl From<UsagePolicy> for PolicyResponse {
    fn from(p: UsagePolicy) -> Self {
        Self {
            id: p.id,
            schema_version: p.schema_version,
            version: p.version,
            effective_from: p.effective_from,
            dimensions: p.dimensions.into(),
            legacy_covered_until: p.legacy_coverage.as_ref().map(|l| l.covered_until),
            legacy_manifest_digest: p.legacy_coverage.map(|l| l.manifest_digest),
            shadow_only: true,
        }
    }
}
#[derive(Serialize)]
pub struct PreparationResponse {
    schema_version: u32,
    shadow_only: bool,
}

pub async fn prepare(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<PreparationResponse>> {
    require_admin(&state, &auth).await?;
    pricing_schema::prepare(&state.db).await?;
    Ok(Json(PreparationResponse {
        schema_version: 1,
        shadow_only: true,
    }))
}
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<TariffQuery>,
) -> AppResult<Json<Vec<TariffResponse>>> {
    require_admin_or_operator(&state, &auth, "admin.billing.cma_usage_rates").await?;
    Ok(Json(
        tariff_store::list_tariffs(&state.db, &query.producer_namespace)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
pub async fn publish(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<TariffRequest>,
) -> AppResult<Json<TariffResponse>> {
    require_admin(&state, &auth).await?;
    let card = tariff_store::publish_tariff(
        &state.db,
        tariff_store::PublishTariff {
            producer_namespace: input.producer_namespace,
            expected_version: input.expected_version,
            effective_from: input.effective_from,
            rates: input.rates,
        },
        &AuditActor::from_auth_user(&auth),
        state.audit_chain_hmac_key.as_slice(),
    )
    .await?;
    Ok(Json(card.into()))
}
pub async fn policies(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ScopeQuery>,
) -> AppResult<Json<Vec<PolicyResponse>>> {
    require_admin_or_operator(&state, &auth, "admin.billing.cma_usage_rates").await?;
    Ok(Json(
        tariff_store::list_policies(&state.db, &query.into())
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    ))
}
pub async fn publish_policy(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<PolicyRequest>,
) -> AppResult<Json<PolicyResponse>> {
    require_admin(&state, &auth).await?;
    let policy = tariff_store::publish_policy(
        &state.db,
        tariff_store::PublishPolicy {
            scope: input.scope.into(),
            expected_version: input.expected_version,
            effective_from: input.effective_from,
            dimensions: input.dimensions.into(),
            legacy_coverage: input.legacy_coverage.map(|l| LegacyCoverage {
                covered_until: l.covered_until,
                manifest_digest: l.manifest_digest,
            }),
        },
        &AuditActor::from_auth_user(&auth),
        state.audit_chain_hmac_key.as_slice(),
    )
    .await?;
    Ok(Json(policy.into()))
}

#[cfg(test)]
#[path = "usage_tests.rs"]
mod tests;
