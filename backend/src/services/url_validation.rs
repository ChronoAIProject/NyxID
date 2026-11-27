//! Shared URL-validation helpers used by both the HTTP layer and the
//! service layer.
//!
//! These live under `services/` (the middle layer of the
//! `handlers/ -> services/ -> models/` stack) so that services like
//! `api_docs_service` and `user_endpoint_service` can reuse them without
//! pulling the handler layer into the service layer -- the project's stated
//! layering rule (`CLAUDE.md#2 Layer Architecture`). Handlers import from
//! here just like any other service-layer helper.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::errors::{AppError, AppResult};

pub const MAX_URL_LEN: usize = 2048;

pub(crate) fn url_host_for_logs(value: &str) -> String {
    url::Url::parse(value)
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .and_then(|url| Some(format!("{}://{}", url.scheme(), url.host_str()?)))
        .unwrap_or_else(|| "<invalid-url>".to_owned())
}

/// Validate a user-supplied URL that will be stored and later shown to a
/// remote operator. Unlike `validate_base_url`, this rejects private,
/// loopback, link-local, CGNAT, unspecified, and metadata targets because
/// the server should not persist internal routing hints supplied from a
/// different trust boundary.
pub async fn validate_public_http_url(url: &str, field_name: &str) -> AppResult<()> {
    validate_public_http_url_with_limit(url, field_name, MAX_URL_LEN).await
}

/// Media URLs can contain long provider signatures while retaining the same host boundary.
pub(crate) async fn validate_public_http_url_with_limit(
    url: &str,
    field_name: &str,
    max_len: usize,
) -> AppResult<()> {
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
    reject_url_userinfo(&parsed)?;

    let host = parsed.host_str().ok_or_else(|| {
        AppError::ValidationError(format!("{field_name} must include a hostname"))
    })?;
    let normalized_host = normalize_host(host);
    // Fast-path obvious local/internal names here. This list is not intended
    // to be exhaustive; the DNS resolution check below is the actual safety
    // guard and catches private, loopback, link-local, and metadata targets
    // regardless of which hostname resolved to them.
    if normalized_host == "localhost" || is_cloud_metadata_host(&normalized_host) {
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
/// Deployments can enable loopback for development. A strictly public call
/// declines that opt-in. Catalog exceptions require a server-resolved ID.
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

    let access = super::proxy_destination::Access::for_request(None, url, &parsed, hosted_mode)?;
    super::proxy_destination::Destination::resolve(&parsed, access).await?;

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
pub(super) fn is_cloud_metadata_host(host: &str) -> bool {
    let host = normalize_host(host);
    matches!(
        host.as_str(),
        "metadata"
            | "metadata.google.internal"
            | "metadata.goog"
            | "instance-data"
            | "instance-data.ec2.internal"
    ) || host
        .parse::<IpAddr>()
        .ok()
        .and_then(canonical_ip)
        .is_some_and(is_metadata_ip)
}

fn canonical_ip(ip: IpAddr) -> Option<IpAddr> {
    let IpAddr::V6(v6) = ip else {
        return Some(ip);
    };
    let bytes = v6.octets();
    let v4 = match v6.segments() {
        [0x64, 0xff9b, 0, 0, 0, 0, ..] => Ipv4Addr::new(bytes[12], bytes[13], bytes[14], bytes[15]),
        [0x64, 0xff9b, 1, ..] => {
            // RFC 6052 splits IPv4 around the reserved octet at bit 64 for a /48.
            if bytes[8] != 0 || bytes[11..] != [0; 5] {
                return None;
            }
            Ipv4Addr::new(bytes[6], bytes[7], bytes[9], bytes[10])
        }
        _ if !v6.is_loopback() && !v6.is_unspecified() => {
            return Some(v6.to_ipv4().map_or(ip, IpAddr::V4));
        }
        _ => return Some(ip),
    };
    Some(IpAddr::V4(v4))
}

fn is_metadata_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => matches!(
            v4.octets(),
            [169, 254, 169, 254] | [169, 254, 170, 2] | [100, 100, 100, 200] | [168, 63, 129, 16]
        ),
        IpAddr::V6(v6) => v6 == Ipv6Addr::new(0xfd00, 0xec2, 0, 0, 0, 0, 0, 0x254),
    }
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

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AddressClass {
    Public,
    Private,
    Loopback,
    Reserved,
}

pub(super) fn address_class(ip: IpAddr) -> AddressClass {
    let Some(ip) = canonical_ip(ip) else {
        return AddressClass::Reserved;
    };
    if ip.is_unspecified() || ip.is_multicast() || is_metadata_ip(ip) {
        return AddressClass::Reserved;
    }
    if ip.is_loopback() {
        return AddressClass::Loopback;
    }
    match ip {
        IpAddr::V4(v4)
            if v4.is_link_local()
                || matches!(v4.octets(), [0, ..] | [192, 0, 0, _] | [240..=255, ..]) =>
        {
            AddressClass::Reserved
        }
        IpAddr::V4(v4)
            if v4.is_private()
                || is_rfc6598_cgnat(v4)
                || matches!(v4.octets(), [198, 18..=19, ..]) =>
        {
            AddressClass::Private
        }
        IpAddr::V6(v6) if v6.is_unicast_link_local() => AddressClass::Reserved,
        IpAddr::V6(v6)
            if v6.is_unique_local() || matches!(v6.segments(), [0xfec0..=0xfeff, ..]) =>
        {
            AddressClass::Private
        }
        _ => AddressClass::Public,
    }
}

pub(crate) fn is_private_or_internal_ip(ip: IpAddr) -> bool {
    address_class(ip) != AddressClass::Public
}

fn is_rfc6598_cgnat(ip: Ipv4Addr) -> bool {
    ip.octets()[0] == 100 && (64..=127).contains(&ip.octets()[1])
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_URL_LEN, reject_url_userinfo, validate_advisory_http_url, validate_base_url,
        validate_optional_spec_url, validate_public_http_url, validate_user_endpoint_url,
    };

    #[test]
    fn diagnostics_format_scheme_and_host() {
        for (input, expected) in [
            (
                "https://user:pass@example.com:8443/path?query=value#fragment",
                "https://example.com",
            ),
            ("http://[::1]:8080/test", "http://[::1]"),
            ("not a url", "<invalid-url>"),
            ("data:text/plain,fixture", "<invalid-url>"),
        ] {
            assert_eq!(super::url_host_for_logs(input), expected);
        }
    }

    #[test]
    fn proxy_destination_public_address_classes() {
        use super::AddressClass::{Loopback, Private, Public, Reserved};

        let mismatches: Vec<_> = [
            ("64:ff9b::808:808", Public),
            ("64:ff9b::a00:1", Private),
            ("64:ff9b::7f00:1", Loopback),
            ("64:ff9b::a9fe:a9fe", Reserved),
            ("64:ff9b:1:808:8:800::", Public),
            ("64:ff9b:1:a00:0:100::", Private),
            ("64:ff9b:1:7f00:0:100::", Loopback),
            ("64:ff9b:1:a9fe:a9:fe00::", Reserved),
            ("64:ff9b:1:808:8:800::1", Reserved),
            ("64:ff9b:1:808:108:800::", Reserved),
            ("64:ff9b:2:a9fe:a9:fe00::", Public),
            ("fec0::", Private),
            ("feff:ffff:ffff:ffff:ffff:ffff:ffff:ffff", Private),
            ("198.17.255.255", Public),
            ("198.18.0.0", Private),
            ("198.19.255.255", Private),
            ("198.20.0.0", Public),
            ("192.0.0.0", Reserved),
            ("192.0.0.255", Reserved),
            ("192.0.1.0", Public),
            ("240.0.0.0", Reserved),
            ("255.255.255.254", Reserved),
        ]
        .into_iter()
        .filter_map(|(literal, expected)| {
            (super::address_class(literal.parse().unwrap()) != expected).then_some(literal)
        })
        .collect();
        assert!(
            mismatches.is_empty(),
            "incorrect address classes {mismatches:?}"
        );

        let nonpublic = [
            "0.0.0.0",
            "127.0.0.1",
            "10.0.0.1",
            "172.31.0.1",
            "192.168.1.1",
            "169.254.170.2",
            "100.64.0.1",
            "224.0.0.1",
            "255.255.255.255",
            "::",
            "::1",
            "fd12::1",
            "fe80::1",
            "ff02::1",
            "::ffff:169.254.169.254",
            "::192.168.1.1",
            "168.63.129.16",
        ];
        let allowed: Vec<_> = nonpublic
            .into_iter()
            .filter(|ip| !super::is_private_or_internal_ip(ip.parse().unwrap()))
            .collect();
        assert!(
            allowed.is_empty(),
            "unexpected public classification {allowed:?}"
        );
        for ip in ["93.184.216.34", "2606:4700:4700::1111"] {
            assert!(!super::is_private_or_internal_ip(ip.parse().unwrap()));
        }
    }

    #[tokio::test]
    async fn proxy_destination_private_network_requires_explicit_authority() {
        for hosted in [false, true] {
            assert!(
                validate_user_endpoint_url("http://192.168.1.1", hosted, "endpoint_url")
                    .await
                    .is_err()
            );
        }
    }

    #[test]
    fn proxy_destination_metadata_literals_are_normalized() {
        for base in [
            "http://[::ffff:a9fe:a9fe]",
            "http://169.254.170.2",
            "http://[::a9fe:a9fe]",
            "http://Metadata.Google.Internal.",
            "http://MeTaDaTa.GoOg.",
            "http://[64:ff9b::a9fe:a9fe]",
            "http://[64:ff9b:1:a9fe:a9:fe00::]",
        ] {
            assert!(validate_base_url(base).is_err(), "{base}");
        }
    }

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
            validate_user_endpoint_url("https://93.184.216.34/v1", false, "endpoint_url")
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
