//! Read-only authoring metadata, independent of flag/readiness and provider calls.
use super::*;
use crate::{
    models::platform_model_tariff::{CacheTtl, Modality, UsageContract, UsageFamily},
    services::platform_model_execution::certification::contracts::{self, Flags, Transport},
};

#[derive(Serialize)]
pub struct CertifiedContract {
    version: &'static str,
    family: UsageFamily,
    transports: &'static [Transport],
    fixture_sha256: String,
    flags: Flags,
    cache_write_ttls: &'static [CacheTtl],
    input_modalities: &'static [Modality],
    output_modalities: &'static [Modality],
    default_contract: UsageContract,
}
pub async fn certified_contracts(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<CertifiedContract>>> {
    super::super::login_client_context::require_first_party_human(&auth)?;
    require_admin_or_operator(&state, &auth, "admin.platform_models.contracts").await?;
    Ok(Json(
        contracts::DEFINITIONS
            .iter()
            .map(|d| CertifiedContract {
                version: d.version,
                family: d.family,
                transports: d.transports,
                fixture_sha256: d.digest(),
                flags: d.flags,
                cache_write_ttls: d.cache_write_ttls,
                input_modalities: d.input_modalities,
                output_modalities: d.output_modalities,
                default_contract: d.default_contract(),
            })
            .collect(),
    ))
}
