use axum::Json;
use serde::Serialize;

use crate::mw::auth::AuthUser;
use crate::services::oracle_worker_bundle_service;

#[derive(Serialize)]
pub struct OracleWorkerBundleResponse {
    pub version: String,
    pub sha256: String,
    pub bundle: String,
    pub playwright_core_version: String,
}

pub fn bundle_response() -> OracleWorkerBundleResponse {
    let bundle = oracle_worker_bundle_service::current_bundle();
    OracleWorkerBundleResponse {
        version: bundle.version.to_string(),
        sha256: bundle.sha256.to_string(),
        bundle: bundle.source.to_string(),
        playwright_core_version: bundle.playwright_core_version.to_string(),
    }
}

pub async fn get_bundle(_auth_user: AuthUser) -> Json<OracleWorkerBundleResponse> {
    Json(bundle_response())
}
