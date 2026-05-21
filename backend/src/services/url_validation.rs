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

pub const MAX_URL_LEN: usize = 2048;

/// Validate a user-supplied URL that will be stored and later shown to a
/// remote operator. Unlike `validate_base_url`, this rejects private,
/// loopback, link-local, CGNAT, unspecified, and metadata targets because
/// the server should not persist internal routing hints supplied from a
/// different trust boundary.
pub async fn validate_public_http_url(url: &str, field_name: &str) -> AppResult<()> {
    if url.len() > 2048 {
        return Err(AppError::ValidationError(format!(
            "{field_name} must not exceed 2048 characters"
        )));
    }

    let parsed = url::Url::parse(url)
        .map_err(|_| AppError::ValidationError(format!("{field_name} must be a valid URL")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::ValidationError(format!(
            "{field_name} must use http or https"
        )));
    }
    reject_url_userinfo(&parsed)?;

    let host = parsed.host_str().ok_or_else(|| {
        AppError::ValidationError(format!("{field_name} must include a hostname"))
    })?;
    let normalized_host = normalize_host(host);
    // Fast-path obvious local/internal names here. This list is not intended
    // to be exhaustive; the DNS resolution check below is the actual safety
    // guard and catches private, loopback, link-local, and metadata targets
    // regardless of which hostname resolved to them.
    if matches!(
        normalized_host.as_str(),
        "localhost" | "metadata.google.internal"
    ) {
        return Err(AppError::ValidationError(format!(
            "{field_name} must not target a private or internal hostname"
        )));
    }

    let port = parsed.port_or_known_default().ok_or_else(|| {
        AppError::ValidationError(format!("{field_name} must include a valid port"))
    })?;
    let resolved_addrs = if let Ok(ip) = normalized_host.parse::<std::net::IpAddr>() {
        vec![std::net::SocketAddr::new(ip, port)]
    } else {
        tokio::net::lookup_host((normalized_host.as_str(), port))
            .await
            .map_err(|e| {
                AppError::ValidationError(format!("Failed to resolve {field_name} host: {e}"))
            })?
            .collect()
    };

    if resolved_addrs.is_empty() {
        return Err(AppError::ValidationError(format!(
            "{field_name} host did not resolve to any IP addresses"
        )));
    }
    if resolved_addrs
        .iter()
        .map(std::net::SocketAddr::ip)
        .any(is_private_or_internal_ip)
    {
        return Err(AppError::ValidationError(format!(
            "{field_name} must not resolve to private or internal IP addresses"
        )));
    }

    Ok(())
}

/// Validate a user-supplied advisory URL that is stored as display metadata.
///
/// This intentionally does not perform DNS resolution or IP range checks. Use
/// it only for URLs that NyxID will not fetch server-side. Cloud metadata
/// hostnames are still rejected because the node agent forwards proxy
/// requests to this URL and reaching the local metadata endpoint would leak
/// IAM credentials.
pub fn validate_advisory_http_url(url: &str, field_name: &str, max_len: usize) -> AppResult<()> {
    if url.len() > max_len {
        return Err(AppError::ValidationError(format!(
            "{field_name} must not exceed {max_len} characters"
        )));
    }

    let parsed = url::Url::parse(url)
        .map_err(|_| AppError::ValidationError(format!("{field_name} must be a valid URL")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::ValidationError(format!(
            "{field_name} must use http or https"
        )));
    }
    let Some(host) = parsed.host_str() else {
        return Err(AppError::ValidationError(format!(
            "{field_name} must include a hostname"
        )));
    };
    if is_cloud_metadata_host(host) {
        return Err(AppError::ValidationError(format!(
            "{field_name} must not point to a cloud metadata endpoint"
        )));
    }
    reject_url_userinfo(&parsed)?;

    Ok(())
}

/// Validate a user-supplied endpoint URL for a custom (non-catalog) service.
///
/// Always:
///   - Must parse as http(s).
///   - Must not contain query (`?...`) or fragment (`#...`) components.
///     A base URL like `https://api.example.com/v1` is allowed (the proxy
///     treats the path as a prefix); query and fragment are not.
///   - Must not contain userinfo (`user:pass@`).
///   - Must be <= 2048 chars (matches `validate_public_http_url`).
///
/// When `hosted_mode` is true, this also rejects loopback, private,
/// link-local, CGNAT, unspecified, broadcast, and cloud-metadata targets via
/// DNS resolution. Self-hosted/development deployments skip that DNS/IP-class
/// guard so they can target local services while still enforcing the URL
/// shape above.
pub async fn validate_user_endpoint_url(
    url: &str,
    hosted_mode: bool,
    field_name: &str,
) -> AppResult<()> {
    if url.len() > 2048 {
        return Err(AppError::ValidationError(format!(
            "{field_name} must not exceed 2048 characters"
        )));
    }

    let parsed = url::Url::parse(url)
        .map_err(|_| AppError::ValidationError(format!("{field_name} must be a valid URL")))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::ValidationError(format!(
            "{field_name} must use http or https"
        )));
    }
    if parsed.host_str().is_none() {
        return Err(AppError::ValidationError(format!(
            "{field_name} must include a hostname"
        )));
    }
    if parsed.query().is_some() {
        return Err(AppError::ValidationError(format!(
            "{field_name} must not contain a query string"
        )));
    }
    if parsed.fragment().is_some() {
        return Err(AppError::ValidationError(format!(
            "{field_name} must not contain a fragment"
        )));
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return Err(AppError::ValidationError(format!(
            "{field_name} must not contain userinfo (user:pass@)"
        )));
    }

    if hosted_mode {
        validate_public_http_url(url, field_name).await?;
    }

    Ok(())
}

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
    let normalized = host
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase();
    normalized == "metadata.google.internal"
        || normalized == "169.254.169.254"
        || normalized == "fd00:ec2::254"
        || normalized == "100.100.100.200"
        || normalized == "168.63.129.16"
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

fn normalize_host(host: &str) -> String {
    host.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn is_private_or_internal_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            ipv4.is_loopback()
                || ipv4.is_private()
                || ipv4.is_link_local()
                || ipv4.is_unspecified()
                || ipv4.is_broadcast()
                || is_rfc6598_cgnat(ipv4)
        }
        std::net::IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                || ipv6.is_unspecified()
                || (ipv6.segments()[0] & 0xfe00) == 0xfc00
                || (ipv6.segments()[0] & 0xffc0) == 0xfe80
                || ipv6
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_private_or_internal_ip(mapped.into()))
        }
    }
}

fn is_rfc6598_cgnat(ipv4: std::net::Ipv4Addr) -> bool {
    ipv4.octets()[0] == 100 && (64..=127).contains(&ipv4.octets()[1])
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_URL_LEN, reject_url_userinfo, validate_advisory_http_url, validate_base_url,
        validate_optional_spec_url, validate_public_http_url, validate_user_endpoint_url,
    };

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
        assert!(validate_base_url("http://[fd00:ec2::254]").is_err());
        assert!(validate_base_url("http://100.100.100.200").is_err());
        assert!(validate_base_url("http://168.63.129.16").is_err());
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

    #[tokio::test]
    async fn validate_public_http_url_rejects_internal_targets() {
        assert!(
            validate_public_http_url("http://127.0.0.1:3000", "target_url")
                .await
                .is_err()
        );
        assert!(
            validate_public_http_url("http://localhost:3000", "target_url")
                .await
                .is_err()
        );
        assert!(
            validate_public_http_url("http://10.0.0.5:3000", "target_url")
                .await
                .is_err()
        );
    }

    #[test]
    fn validate_advisory_http_url_accepts_node_local_targets() {
        assert!(
            validate_advisory_http_url("http://192.168.1.1/", "target_url", MAX_URL_LEN).is_ok()
        );
        assert!(
            validate_advisory_http_url(
                "https://homeassistant.local.hass.io:8123/",
                "target_url",
                MAX_URL_LEN
            )
            .is_ok()
        );
        assert!(
            validate_advisory_http_url("http://127.0.0.1:1883/", "target_url", MAX_URL_LEN).is_ok()
        );
        assert!(validate_advisory_http_url("http://[::1]/", "target_url", MAX_URL_LEN).is_ok());
    }

    #[test]
    fn validate_advisory_http_url_rejects_invalid_shapes() {
        assert!(validate_advisory_http_url("not-a-url", "target_url", MAX_URL_LEN).is_err());
        assert!(
            validate_advisory_http_url("ftp://example.com/", "target_url", MAX_URL_LEN).is_err()
        );

        let overlong = format!("http://example.com/{}", "a".repeat(MAX_URL_LEN));
        assert!(validate_advisory_http_url(&overlong, "target_url", MAX_URL_LEN).is_err());
    }

    #[test]
    fn validate_advisory_http_url_rejects_cloud_metadata_endpoints() {
        // Node agents fetch target_url at proxy time, so even though the
        // server only stores this value, a malicious metadata endpoint
        // would leak IAM credentials from the node host.
        assert!(
            validate_advisory_http_url(
                "http://169.254.169.254/latest/meta-data/",
                "target_url",
                MAX_URL_LEN
            )
            .is_err()
        );
        assert!(
            validate_advisory_http_url(
                "http://metadata.google.internal/computeMetadata/v1/",
                "target_url",
                MAX_URL_LEN
            )
            .is_err()
        );
        assert!(
            validate_advisory_http_url(
                "https://METADATA.GOOGLE.INTERNAL./",
                "target_url",
                MAX_URL_LEN
            )
            .is_err()
        );
        assert!(
            validate_advisory_http_url("http://[fd00:ec2::254]/", "target_url", MAX_URL_LEN)
                .is_err()
        );
        assert!(
            validate_advisory_http_url("http://100.100.100.200/", "target_url", MAX_URL_LEN)
                .is_err()
        );
        assert!(
            validate_advisory_http_url("http://168.63.129.16/", "target_url", MAX_URL_LEN).is_err()
        );
    }

    #[test]
    fn validate_advisory_http_url_rejects_userinfo() {
        assert!(
            validate_advisory_http_url("http://user:pass@example.com/", "target_url", MAX_URL_LEN)
                .is_err()
        );
    }

    #[tokio::test]
    async fn validate_user_endpoint_url_rejects_query_and_fragment() {
        assert!(
            validate_user_endpoint_url("https://api.example.com?x=1", false, "endpoint_url")
                .await
                .is_err()
        );
        assert!(
            validate_user_endpoint_url("https://api.example.com#frag", false, "endpoint_url")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn validate_user_endpoint_url_allows_path() {
        // Base path `/v1` is a legitimate prefix.
        assert!(
            validate_user_endpoint_url("https://api.example.com/v1", false, "endpoint_url")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn validate_user_endpoint_url_rejects_loopback_in_hosted_mode() {
        assert!(
            validate_user_endpoint_url("http://localhost:9999", true, "endpoint_url")
                .await
                .is_err()
        );
        assert!(
            validate_user_endpoint_url("http://127.0.0.1:9999", true, "endpoint_url")
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn validate_user_endpoint_url_allows_loopback_in_self_hosted_mode() {
        assert!(
            validate_user_endpoint_url("http://localhost:9999", false, "endpoint_url")
                .await
                .is_ok()
        );
        assert!(
            validate_user_endpoint_url("http://127.0.0.1:9999", false, "endpoint_url")
                .await
                .is_ok()
        );
    }

    #[tokio::test]
    async fn validate_user_endpoint_url_rejects_userinfo() {
        assert!(
            validate_user_endpoint_url("https://user:pass@api.example.com", false, "endpoint_url",)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn validate_user_endpoint_url_rejects_invalid_scheme() {
        assert!(
            validate_user_endpoint_url("ftp://example.com", false, "endpoint_url")
                .await
                .is_err()
        );
    }
}
