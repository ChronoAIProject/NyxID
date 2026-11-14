use axum::{Json, extract::State};
use serde::Deserialize;

use crate::{
    AppState,
    errors::AppResult,
    mw::auth::AuthUser,
    services::personal_erasure_orphan_sweep::{self, OrphanSweepReport},
};

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OrphanSweepRequest {
    /// Dry-run is the default. Applying requires the digest returned by a
    /// preceding dry-run so an operator reviews the exact candidate set.
    #[serde(default)]
    pub apply: bool,
    pub digest: Option<String>,
}

/// POST /api/v1/admin/personal-erasure/orphans
pub async fn orphan_sweep(
    State(state): State<AppState>,
    auth: AuthUser,
    request: Option<Json<OrphanSweepRequest>>,
) -> AppResult<Json<OrphanSweepReport>> {
    crate::handlers::admin_helpers::require_admin(&state, &auth).await?;
    crate::handlers::login_client_context::require_first_party_human(&auth)?;
    let request = request.map(|Json(value)| value).unwrap_or_default();
    Ok(Json(
        personal_erasure_orphan_sweep::run(
            &state,
            &auth.user_id.to_string(),
            request.apply,
            request.digest.as_deref(),
        )
        .await?,
    ))
}
