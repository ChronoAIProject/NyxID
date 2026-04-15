//! Shared URL-validation helpers used by both the HTTP layer and the
//! service layer.
//!
//! These live under `services/` (the middle layer of the
//! `handlers/ -> services/ -> models/` stack) so that services like
//! `api_docs_service` and `user_endpoint_service` can reuse them without
//! pulling the handler layer into the service layer -- the project's stated
//! layering rule (`CLAUDE.md#2 Layer Architecture`). Handlers import from
//! here just like any other service-layer helper.

use crate::errors::{AppError, AppResult};

/// Validate that a URL has a valid scheme and hostname.
///
/// Cloud metadata endpoints (169.254.169.254, metadata.google.internal)
/// are blocked in every environment.  Private IPs and localhost are
/// allowed so that self-hosted nodes and services remain reachable.
pub fn validate_base_url(url: &str) -> AppResult<()> {
    // Must start with https:// or http://
    if !url.starts_with("https://") && !url.starts_with("http://") {
        return Err(AppError::ValidationError(
            "base_url must start with https:// or http://".to_string(),
        ));
    }

    // Parse the URL to extract the hostname
    let parsed = url::Url::parse(url)
        .map_err(|_| AppError::ValidationError("Invalid base_url format".to_string()))?;

    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::ValidationError("base_url must contain a hostname".to_string()))?;

    // Block cloud metadata endpoints -- dangerous in any environment
    if is_cloud_metadata_host(host) {
        return Err(AppError::ValidationError(
            "URL must not point to a cloud metadata endpoint".to_string(),
        ));
    }

    Ok(())
}

/// Returns true if the hostname is a known cloud metadata endpoint.
fn is_cloud_metadata_host(host: &str) -> bool {
    let normalized = host.trim_end_matches('.').to_ascii_lowercase();
    normalized == "metadata.google.internal"
        || normalized == "169.254.169.254"
        || normalized == "[fd00:ec2::254]"
}

/// Validate an optional documentation spec URL.
///
/// Spec URLs are fetched server-side and returned in API responses / UI, so
/// we enforce the userinfo ban at storage-time -- otherwise
/// `https://user:pass@host/spec.json` would land in MongoDB, leak into
/// responses, and only trip the fetch-time guard later. Defense in depth is
/// kept: `api_docs_service::validate_spec_fetch_target` re-checks on read.
pub fn validate_optional_spec_url(url: &str) -> AppResult<()> {
    if url.len() > 2048 {
        return Err(AppError::ValidationError(
            "Spec URL must not exceed 2048 characters".to_string(),
        ));
    }

    validate_base_url(url)?;

    // Re-parse here instead of threading a `url::Url` through `validate_base_url`'s
    // public signature -- the parse is cheap and keeps the existing helper
    // callers (endpoint base URLs, provider URLs) untouched.
    let parsed = url::Url::parse(url)
        .map_err(|_| AppError::ValidationError("Invalid spec URL format".to_string()))?;
    reject_url_userinfo(&parsed)?;
    Ok(())
}

/// Reject URLs with embedded credentials (`https://user:pass@host/...`).
/// Shared between storage-time validation (`validate_optional_spec_url`) and
/// fetch-time validation (`api_docs_service::validate_spec_fetch_target`) so
/// the two can't drift.
pub fn reject_url_userinfo(parsed: &url::Url) -> AppResult<()> {
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::ValidationError(
            "URL must not contain userinfo (user:pass@)".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{reject_url_userinfo, validate_base_url, validate_optional_spec_url};

    #[test]
    fn validate_base_url_accepts_public_url() {
        assert!(validate_base_url("https://api.example.com").is_ok());
        assert!(validate_base_url("http://api.example.com").is_ok());
    }

    #[test]
    fn validate_base_url_accepts_private_ips() {
        assert!(validate_base_url("http://localhost:3000").is_ok());
        assert!(validate_base_url("http://127.0.0.1:8080").is_ok());
        assert!(validate_base_url("http://192.168.1.50:3000").is_ok());
        assert!(validate_base_url("http://10.0.0.5:8080").is_ok());
        assert!(validate_base_url("http://100.64.0.10:3000").is_ok());
        assert!(validate_base_url("http://172.16.0.1:3000").is_ok());
    }

    #[test]
    fn validate_base_url_rejects_cloud_metadata() {
        assert!(validate_base_url("http://metadata.google.internal").is_err());
        assert!(validate_base_url("http://169.254.169.254").is_err());
    }

    #[test]
    fn validate_base_url_rejects_invalid_scheme() {
        assert!(validate_base_url("ftp://example.com").is_err());
        assert!(validate_base_url("javascript:alert(1)").is_err());
    }

    #[test]
    fn validate_optional_spec_url_accepts_public_https_url() {
        assert!(validate_optional_spec_url("https://example.com/openapi.json").is_ok());
    }

    #[test]
    fn validate_optional_spec_url_rejects_metadata() {
        assert!(validate_optional_spec_url("http://169.254.169.254/latest").is_err());
    }

    #[test]
    fn validate_optional_spec_url_rejects_embedded_credentials() {
        // Regression: P2 finding -- storage-time validation must reject URLs
        // that carry credentials in the userinfo component, otherwise
        // `POST /keys` / admin catalog writes could persist a secret into
        // `openapi_spec_url` that later leaks into API responses and logs.
        assert!(validate_optional_spec_url("https://user:pass@example.com/openapi.json").is_err());
        assert!(validate_optional_spec_url("https://user@example.com/openapi.json").is_err());
        // Sanity: the happy path still accepts a credential-free URL.
        assert!(validate_optional_spec_url("https://example.com/openapi.json").is_ok());
    }

    #[test]
    fn reject_url_userinfo_accepts_credential_free_urls() {
        let parsed = url::Url::parse("https://example.com/openapi.json").unwrap();
        assert!(reject_url_userinfo(&parsed).is_ok());
    }

    #[test]
    fn reject_url_userinfo_blocks_username_or_password() {
        let with_both = url::Url::parse("https://user:pass@example.com/").unwrap();
        assert!(reject_url_userinfo(&with_both).is_err());
        let with_username_only = url::Url::parse("https://user@example.com/").unwrap();
        assert!(reject_url_userinfo(&with_username_only).is_err());
    }
}
