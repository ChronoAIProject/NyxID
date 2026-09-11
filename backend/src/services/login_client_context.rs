use crate::models::auth_device_code::{
    AuthDeviceClientIpAttribution, AuthDeviceCodeStatus, AuthDeviceInitiatingOriginStatus,
};
use crate::models::login_client_context::LoginClientContext;
use chrono::{DateTime, Utc};
use std::net::IpAddr;

const CLIENT_UA_PARSE_MAX_LEN: usize = 512;
const CLIENT_VERSION_MAX_LEN: usize = 32;
pub(super) const CLIENT_DISPLAY_MAX_LEN: usize = 96;
const CLIENT_TIMEZONE_MAX_LEN: usize = 64;
const CLIENT_LOCALE_MAX_LEN: usize = 35;
const CLIENT_SCREEN_DIMENSION_MAX: u32 = 32_768;
const CLIENT_HARDWARE_CONCURRENCY_MAX: u16 = 1_024;
const CLIENT_DEVICE_PIXEL_RATIO_MAX: f64 = 16.0;
const CLIENT_DEVICE_MEMORY_MAX: f64 = 1_024.0;
pub(crate) const INITIATING_ORIGIN_MAX_LEN: usize = 256;

#[derive(Clone, PartialEq)]
pub struct PreviewOutput {
    pub requested_profile: Option<String>,
    pub client_label: Option<String>,
    pub client_user_agent: Option<String>,
    pub client_ip: Option<String>,
    pub client_ip_attribution: String,
    pub client_country: Option<String>,
    pub client_city: Option<String>,
    pub client_region: Option<String>,
    pub client_continent: Option<String>,
    pub client_ip_timezone: Option<String>,
    pub initiating_origin: Option<String>,
    pub initiating_origin_status: String,
    pub client_kind: String,
    pub client_app: Option<String>,
    pub client_platform: Option<String>,
    pub client_model: Option<String>,
    pub client_form_factor: Option<String>,
    pub client_timezone: Option<String>,
    pub client_timezone_matches_ip: Option<bool>,
    pub client_locale: Option<String>,
    pub client_screen_width: Option<u32>,
    pub client_screen_height: Option<u32>,
    pub client_device_pixel_ratio: Option<f64>,
    pub client_hardware_concurrency: Option<u16>,
    pub client_device_memory: Option<f64>,
    pub same_ip_as_viewer: Option<bool>,
    pub network_relation: Option<String>,
    pub seconds_remaining: i64,
    pub initiated_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub status: AuthDeviceCodeStatus,
}

pub(crate) fn sanitize_context(input: LoginClientContext) -> LoginClientContext {
    LoginClientContext {
        requested_profile: sanitize_optional(input.requested_profile, 64),
        client_label: sanitize_optional(input.client_label, 64),
        client_user_agent: sanitize_optional(input.client_user_agent, 256),
        client_ip: input.client_ip,
        client_ip_attribution: input.client_ip_attribution,
        client_country: normalize_client_country(input.client_country),
        client_city: normalize_geo_label(input.client_city),
        client_region: normalize_geo_label(input.client_region),
        client_continent: normalize_client_continent(input.client_continent),
        client_ip_timezone: normalize_client_timezone(input.client_ip_timezone),
        initiating_origin: sanitize_optional(input.initiating_origin, INITIATING_ORIGIN_MAX_LEN),
        initiating_origin_status: input.initiating_origin_status,
        client_app: sanitize_optional(input.client_app, CLIENT_DISPLAY_MAX_LEN),
        client_platform: sanitize_optional(input.client_platform, CLIENT_DISPLAY_MAX_LEN),
        client_model: sanitize_optional(input.client_model, CLIENT_DISPLAY_MAX_LEN),
        client_form_factor: normalize_client_form_factor(input.client_form_factor),
        client_timezone: normalize_client_timezone(input.client_timezone),
        client_locale: normalize_client_locale(input.client_locale),
        client_screen_width: normalize_screen_dimension(input.client_screen_width),
        client_screen_height: normalize_screen_dimension(input.client_screen_height),
        client_device_pixel_ratio: normalize_device_pixel_ratio(input.client_device_pixel_ratio),
        client_hardware_concurrency: normalize_hardware_concurrency(
            input.client_hardware_concurrency,
        ),
        client_device_memory: normalize_device_memory(input.client_device_memory),
    }
}

pub(crate) fn context_preview(
    row: LoginClientContext,
    created_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    status: AuthDeviceCodeStatus,
    viewer_ip: Option<&str>,
    viewer_ip_attribution: AuthDeviceClientIpAttribution,
) -> PreviewOutput {
    let parsed_client = parse_client_user_agent(row.client_user_agent.as_deref());
    let client_ip_attribution =
        effective_client_ip_attribution(row.client_ip.as_deref(), row.client_ip_attribution);
    let network_relation = network_relation(
        row.client_ip.as_deref(),
        client_ip_attribution,
        viewer_ip,
        viewer_ip_attribution,
    );
    let same_ip_as_viewer = network_relation.map(|relation| relation == "same_ip");
    let seconds_remaining = seconds_remaining_at(expires_at, Utc::now());
    let verified_ip = client_ip_attribution == AuthDeviceClientIpAttribution::Verified;
    let client_ip_timezone = verified_ip.then_some(row.client_ip_timezone).flatten();
    let client_timezone_matches_ip = verified_ip
        .then(|| {
            timezones_match(
                client_ip_timezone.as_deref(),
                row.client_timezone.as_deref(),
            )
        })
        .flatten();

    PreviewOutput {
        requested_profile: row.requested_profile,
        client_label: row.client_label,
        client_user_agent: row.client_user_agent,
        client_ip: row.client_ip,
        client_ip_attribution: client_ip_attribution.as_str().to_string(),
        client_country: verified_ip.then_some(row.client_country).flatten(),
        client_city: verified_ip.then_some(row.client_city).flatten(),
        client_region: verified_ip.then_some(row.client_region).flatten(),
        client_continent: verified_ip.then_some(row.client_continent).flatten(),
        client_ip_timezone,
        initiating_origin: row.initiating_origin,
        initiating_origin_status: row.initiating_origin_status.as_str().to_string(),
        client_kind: parsed_client.kind.to_string(),
        client_app: row.client_app.or(parsed_client.app),
        client_platform: row.client_platform.or(parsed_client.platform),
        client_model: row.client_model,
        client_form_factor: row.client_form_factor,
        client_timezone: row.client_timezone,
        client_timezone_matches_ip,
        client_locale: row.client_locale,
        client_screen_width: row.client_screen_width,
        client_screen_height: row.client_screen_height,
        client_device_pixel_ratio: row.client_device_pixel_ratio,
        client_hardware_concurrency: row.client_hardware_concurrency,
        client_device_memory: row.client_device_memory,
        same_ip_as_viewer,
        network_relation: network_relation.map(str::to_string),
        seconds_remaining,
        initiated_at: created_at,
        expires_at,
        status,
    }
}

pub(crate) fn sanitize_optional(value: Option<String>, max_len: usize) -> Option<String> {
    let value = value?;
    let sanitized: String = value
        .trim()
        .chars()
        .filter(|ch| !ch.is_control())
        .take(max_len)
        .collect();
    let sanitized = sanitized.trim().to_string();
    if sanitized.is_empty() {
        None
    } else {
        Some(sanitized)
    }
}

pub(crate) fn normalize_client_country(value: Option<String>) -> Option<String> {
    let normalized = value?.trim().to_ascii_uppercase();
    if normalized.len() != 2
        || !normalized.bytes().all(|byte| byte.is_ascii_alphabetic())
        || normalized == "XX"
    {
        return None;
    }
    Some(normalized)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct TrustedClientLocation {
    pub country: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub continent: Option<String>,
    pub timezone: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InitiatingOriginClassification {
    pub origin: Option<String>,
    pub status: AuthDeviceInitiatingOriginStatus,
}

pub(crate) fn classify_initiating_origin(
    raw_origin: Option<&str>,
    frontend_url: &str,
) -> InitiatingOriginClassification {
    let Some(raw_origin) = raw_origin else {
        return InitiatingOriginClassification {
            origin: None,
            status: AuthDeviceInitiatingOriginStatus::Absent,
        };
    };

    let trimmed = raw_origin.trim();
    let stored = sanitize_optional(Some(trimmed.to_string()), INITIATING_ORIGIN_MAX_LEN);
    if trimmed.is_empty()
        || trimmed.chars().count() > INITIATING_ORIGIN_MAX_LEN
        || trimmed.chars().any(char::is_control)
    {
        return InitiatingOriginClassification {
            origin: stored,
            status: AuthDeviceInitiatingOriginStatus::Malformed,
        };
    }

    let Ok(parsed) = url::Url::parse(trimmed) else {
        return InitiatingOriginClassification {
            origin: stored,
            status: AuthDeviceInitiatingOriginStatus::Malformed,
        };
    };
    if !matches!(parsed.scheme(), "http" | "https") {
        return InitiatingOriginClassification {
            origin: stored,
            status: AuthDeviceInitiatingOriginStatus::NonHttp,
        };
    }
    if parsed.host().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.path() != "/"
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return InitiatingOriginClassification {
            origin: stored,
            status: AuthDeviceInitiatingOriginStatus::Malformed,
        };
    }

    let origin = parsed.origin().ascii_serialization();
    let frontend_origin = url::Url::parse(frontend_url.trim())
        .ok()
        .filter(|url| matches!(url.scheme(), "http" | "https"))
        .map(|url| url.origin().ascii_serialization());
    let status = if frontend_origin.as_deref() == Some(origin.as_str()) {
        AuthDeviceInitiatingOriginStatus::Matched
    } else {
        AuthDeviceInitiatingOriginStatus::Mismatched
    };

    InitiatingOriginClassification {
        origin: Some(origin),
        status,
    }
}

pub(crate) fn normalize_geo_label(value: Option<String>) -> Option<String> {
    sanitize_optional(value, CLIENT_DISPLAY_MAX_LEN)
}

pub(crate) fn normalize_client_continent(value: Option<String>) -> Option<String> {
    let normalized = value?.trim().to_ascii_uppercase();
    if normalized.len() != 2
        || !normalized.bytes().all(|byte| byte.is_ascii_alphabetic())
        || normalized == "XX"
    {
        return None;
    }
    Some(normalized)
}

pub(crate) fn normalize_client_timezone(value: Option<String>) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > CLIENT_TIMEZONE_MAX_LEN
        || trimmed.chars().any(char::is_control)
        || !trimmed.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '/' | '_' | '-' | '+')
        })
    {
        return None;
    }
    Some(trimmed.to_string())
}

pub(crate) fn normalize_client_locale(value: Option<String>) -> Option<String> {
    let value = value?;
    let trimmed = value.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > CLIENT_LOCALE_MAX_LEN
        || trimmed.chars().any(char::is_control)
        || !trimmed
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '-')
    {
        return None;
    }
    Some(trimmed.to_string())
}

pub(crate) fn normalize_client_form_factor(value: Option<String>) -> Option<String> {
    match value?.trim().to_ascii_lowercase().as_str() {
        "desktop" => Some("desktop".to_string()),
        "mobile" => Some("mobile".to_string()),
        "tablet" => Some("tablet".to_string()),
        "unknown" => Some("unknown".to_string()),
        _ => None,
    }
}

pub(crate) fn normalize_screen_dimension(value: Option<u32>) -> Option<u32> {
    value.filter(|value| (1..=CLIENT_SCREEN_DIMENSION_MAX).contains(value))
}

pub(crate) fn normalize_device_pixel_ratio(value: Option<f64>) -> Option<f64> {
    value.filter(|value| {
        value.is_finite() && *value > 0.0 && *value <= CLIENT_DEVICE_PIXEL_RATIO_MAX
    })
}

pub(crate) fn normalize_hardware_concurrency(value: Option<u16>) -> Option<u16> {
    value.filter(|value| (1..=CLIENT_HARDWARE_CONCURRENCY_MAX).contains(value))
}

pub(crate) fn normalize_device_memory(value: Option<f64>) -> Option<f64> {
    value.filter(|value| value.is_finite() && *value > 0.0 && *value <= CLIENT_DEVICE_MEMORY_MAX)
}

pub(crate) fn timezones_match(
    verified_timezone: Option<&str>,
    reported_timezone: Option<&str>,
) -> Option<bool> {
    Some(verified_timezone?.eq_ignore_ascii_case(reported_timezone?))
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ParsedClientUserAgent {
    pub(crate) kind: &'static str,
    pub(crate) app: Option<String>,
    pub(crate) platform: Option<String>,
}

pub(crate) fn parse_client_user_agent(value: Option<&str>) -> ParsedClientUserAgent {
    let Some(user_agent) = bounded_clean_text(value.unwrap_or_default(), CLIENT_UA_PARSE_MAX_LEN)
    else {
        return unknown_client_user_agent();
    };

    if let Some(version) = native_client_version(&user_agent, "nyxid-cli/") {
        return ParsedClientUserAgent {
            kind: "cli",
            app: bounded_display(format!("NyxID CLI {version}")),
            platform: native_client_platform(&user_agent),
        };
    }

    if let Some(version) = native_client_version(&user_agent, "nyxid-mobile/") {
        return ParsedClientUserAgent {
            kind: "mobile",
            app: bounded_display(format!("NyxID Mobile {version}")),
            platform: native_client_platform(&user_agent),
        };
    }

    let browser = [
        ("EdgiOS/", "Edge"),
        ("EdgA/", "Edge"),
        ("Edg/", "Edge"),
        ("CriOS/", "Chrome"),
        ("Chrome/", "Chrome"),
        ("FxiOS/", "Firefox"),
        ("Firefox/", "Firefox"),
    ]
    .into_iter()
    .find_map(|(marker, name)| {
        browser_major_version(&user_agent, marker).map(|version| (name, version))
    })
    .or_else(|| {
        if user_agent.contains("Safari/") {
            browser_major_version(&user_agent, "Version/").map(|version| ("Safari", version))
        } else {
            None
        }
    });

    let Some((browser_name, version)) = browser else {
        return unknown_client_user_agent();
    };

    ParsedClientUserAgent {
        kind: "browser",
        app: bounded_display(format!("{browser_name} {version}")),
        platform: browser_platform(&user_agent),
    }
}

pub(crate) fn unknown_client_user_agent() -> ParsedClientUserAgent {
    ParsedClientUserAgent {
        kind: "unknown",
        app: None,
        platform: None,
    }
}

pub(crate) fn bounded_clean_text(value: &str, max_len: usize) -> Option<String> {
    let cleaned = value
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(max_len)
        .collect::<String>();
    let cleaned = cleaned.trim();
    (!cleaned.is_empty()).then(|| cleaned.to_string())
}

pub(crate) fn bounded_display(value: String) -> Option<String> {
    bounded_clean_text(&value, CLIENT_DISPLAY_MAX_LEN)
}

pub(crate) fn native_client_version(user_agent: &str, prefix: &str) -> Option<String> {
    let rest = user_agent.strip_prefix(prefix)?;
    let version = rest
        .chars()
        .take_while(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_' | '+')
        })
        .take(CLIENT_VERSION_MAX_LEN)
        .collect::<String>();
    (!version.is_empty()).then_some(version)
}

pub(crate) fn browser_major_version(user_agent: &str, marker: &str) -> Option<String> {
    let start = user_agent.find(marker)? + marker.len();
    let version = user_agent[start..]
        .chars()
        .take_while(char::is_ascii_digit)
        .take(CLIENT_VERSION_MAX_LEN)
        .collect::<String>();
    (!version.is_empty()).then_some(version)
}

pub(crate) fn native_client_platform(user_agent: &str) -> Option<String> {
    let start = user_agent.find('(')? + 1;
    let end = user_agent[start..].find(')')? + start;
    let mut segments = user_agent[start..end].split(';').map(str::trim);
    let platform = canonical_platform(segments.next()?)?;
    let architecture = segments.next().and_then(canonical_architecture);
    platform_with_architecture(platform, architecture)
}

pub(crate) fn canonical_platform(value: &str) -> Option<&'static str> {
    match value.to_ascii_lowercase().as_str() {
        "macos" | "mac os" | "darwin" => Some("macOS"),
        "windows" | "win32" => Some("Windows"),
        "linux" => Some("Linux"),
        "ios" | "iphone" | "ipad" => Some("iOS"),
        "android" => Some("Android"),
        _ => None,
    }
}

pub(crate) fn canonical_architecture(value: &str) -> Option<&'static str> {
    match value.trim().to_ascii_lowercase().as_str() {
        "aarch64" => Some("aarch64"),
        "arm64" => Some("arm64"),
        "arm" | "armv7" | "armv7l" => Some("arm"),
        "x86_64" | "x64" | "amd64" => Some("x86_64"),
        "x86" | "i386" | "i686" => Some("x86"),
        _ => None,
    }
}

pub(crate) fn platform_with_architecture(
    platform: &'static str,
    architecture: Option<&'static str>,
) -> Option<String> {
    bounded_display(match architecture {
        Some(architecture) => format!("{platform} ({architecture})"),
        None => platform.to_string(),
    })
}

pub(crate) fn browser_platform(user_agent: &str) -> Option<String> {
    let lower = user_agent.to_ascii_lowercase();
    if lower.contains("iphone") || lower.contains("ipad") || lower.contains("ipod") {
        return platform_with_architecture("iOS", None);
    }
    if lower.contains("android") {
        return platform_with_architecture("Android", reported_architecture(&lower));
    }
    if lower.contains("windows") {
        return platform_with_architecture("Windows", reported_architecture(&lower));
    }
    if lower.contains("macintosh") || lower.contains("mac os x") {
        let architecture = lower
            .contains("intel mac")
            .then_some("x86_64")
            .or_else(|| reported_architecture(&lower));
        return platform_with_architecture("macOS", architecture);
    }
    if lower.contains("linux") || lower.contains("x11") {
        return platform_with_architecture("Linux", reported_architecture(&lower));
    }
    None
}

pub(crate) fn reported_architecture(lower_user_agent: &str) -> Option<&'static str> {
    if lower_user_agent.contains("aarch64") {
        Some("aarch64")
    } else if lower_user_agent.contains("arm64") {
        Some("arm64")
    } else if lower_user_agent.contains("x86_64")
        || lower_user_agent.contains("win64")
        || lower_user_agent.contains("x64")
        || lower_user_agent.contains("amd64")
    {
        Some("x86_64")
    } else if lower_user_agent.contains("i686") || lower_user_agent.contains("i386") {
        Some("x86")
    } else {
        None
    }
}

pub(crate) fn effective_client_ip_attribution(
    requester_ip: Option<&str>,
    stored_attribution: AuthDeviceClientIpAttribution,
) -> AuthDeviceClientIpAttribution {
    match requester_ip
        .and_then(|value| value.parse::<IpAddr>().ok())
        .map(crate::config::normalize_ip_address)
    {
        Some(ip) if crate::mw::rate_limit::is_global_unicast(ip) => stored_attribution,
        _ => AuthDeviceClientIpAttribution::Unavailable,
    }
}

#[cfg(test)]
pub(crate) fn same_ip_as_viewer(
    requester_ip: Option<&str>,
    requester_attribution: AuthDeviceClientIpAttribution,
    viewer_ip: Option<&str>,
    viewer_attribution: AuthDeviceClientIpAttribution,
) -> Option<bool> {
    network_relation(
        requester_ip,
        requester_attribution,
        viewer_ip,
        viewer_attribution,
    )
    .map(|relation| relation == "same_ip")
}

pub(crate) fn network_relation(
    requester_ip: Option<&str>,
    requester_attribution: AuthDeviceClientIpAttribution,
    viewer_ip: Option<&str>,
    viewer_attribution: AuthDeviceClientIpAttribution,
) -> Option<&'static str> {
    if requester_attribution != AuthDeviceClientIpAttribution::Verified
        || viewer_attribution != AuthDeviceClientIpAttribution::Verified
    {
        return None;
    }
    let requester_ip = crate::config::normalize_ip_address(requester_ip?.parse::<IpAddr>().ok()?);
    let viewer_ip = crate::config::normalize_ip_address(viewer_ip?.parse::<IpAddr>().ok()?);
    if requester_ip == viewer_ip {
        return Some("same_ip");
    }

    let same_network = match (requester_ip, viewer_ip) {
        (IpAddr::V4(requester), IpAddr::V4(viewer)) => {
            requester.octets()[..3] == viewer.octets()[..3]
        }
        (IpAddr::V6(requester), IpAddr::V6(viewer)) => {
            requester.segments()[..3] == viewer.segments()[..3]
        }
        _ => false,
    };
    Some(if same_network {
        "same_network"
    } else {
        "different_network"
    })
}

pub(crate) fn seconds_remaining_at(expires_at: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    let remaining_millis = (expires_at - now).num_milliseconds();
    if remaining_millis <= 0 {
        0
    } else {
        remaining_millis.saturating_add(999) / 1_000
    }
}
