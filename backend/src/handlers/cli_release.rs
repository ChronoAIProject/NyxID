use axum::Json;
use axum::http::header;
use axum::response::IntoResponse;
use serde::Serialize;

const CACHE_CONTROL_VALUE: &str = "public, max-age=300";
const GITHUB_RELEASE_BASE: &str = "https://github.com/ChronoAIProject/NyxID/releases";
const COSIGN_ISSUER: &str = "https://token.actions.githubusercontent.com";

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct CliReleaseManifest {
    pub version: String,
    pub commit: String,
    pub release_tag: String,
    pub release_url: String,
    pub asset_base_url: String,
    pub checksums_url: String,
    pub checksums_signature_url: String,
    pub checksums_cert_url: String,
    pub asset_name_template: String,
    pub cosign_identity: String,
    pub cosign_issuer: String,
}

fn latest_manifest() -> CliReleaseManifest {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let release_tag = format!("v{version}");
    let asset_base_url = format!("{GITHUB_RELEASE_BASE}/download/{release_tag}/");

    CliReleaseManifest {
        version: version.clone(),
        commit: env!("NYXID_GIT_HASH").to_string(),
        release_tag: release_tag.clone(),
        release_url: format!("{GITHUB_RELEASE_BASE}/tag/{release_tag}"),
        asset_base_url: asset_base_url.clone(),
        checksums_url: format!("{asset_base_url}SHA256SUMS"),
        checksums_signature_url: format!("{asset_base_url}SHA256SUMS.sig"),
        checksums_cert_url: format!("{asset_base_url}SHA256SUMS.pem"),
        asset_name_template: "nyxid-{version}-{target}.{ext}".to_string(),
        cosign_identity: format!(
            "https://github.com/ChronoAIProject/NyxID/.github/workflows/release.yml@refs/tags/v{version}"
        ),
        cosign_issuer: COSIGN_ISSUER.to_string(),
    }
}

/// GET /cli/latest
///
/// Returns the current NyxID CLI release manifest for self-update clients.
pub async fn latest_cli_release() -> impl IntoResponse {
    (
        [(header::CACHE_CONTROL, CACHE_CONTROL_VALUE)],
        Json(latest_manifest()),
    )
}

#[cfg(test)]
mod tests {
    use super::latest_manifest;

    #[test]
    fn manifest_version_matches_package_version() {
        assert_eq!(latest_manifest().version, env!("CARGO_PKG_VERSION"));
    }
}
