//! Public native identities for first-party verified app links.

use super::AppConfig;

/// Keep link availability aligned with the identity published by AASA. These
/// public identifiers do not require an APNs signing key.
pub fn apple_app_id(config: &AppConfig) -> Option<String> {
    let team = config.apns_team_id.as_deref()?.trim();
    let bundle = config.apns_topic.as_deref()?.trim();
    if team.is_empty() || bundle.is_empty() {
        return None;
    }
    Some(format!("{team}.{bundle}"))
}

/// Reuse the identity served by the existing Digital Asset Links endpoint.
pub fn has_android_identity(config: &AppConfig) -> bool {
    config
        .android_app_package
        .as_deref()
        .is_some_and(|value| !value.trim().is_empty())
        && !config.android_app_sha256_cert_fingerprints.is_empty()
}
