use super::auth_device::AuthDeviceRequestBody;
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::auth_device_code::AuthDeviceClientIpAttribution,
    mw::rate_limit::{ClientIpAttribution, ResolvedClientIp},
    services::auth_device_service::{self, InitiateInput},
};
use axum::http::{HeaderMap, header};
use std::net::{IpAddr, SocketAddr};

pub(super) fn require_first_party_human(user: &crate::mw::auth::AuthUser) -> AppResult<()> {
    use crate::mw::auth::AuthMethod;
    if !matches!(
        user.auth_method,
        AuthMethod::Session | AuthMethod::AccessToken
    ) || user.oauth_client_id.is_some()
    {
        return Err(AppError::Forbidden(
            "A first-party human account session is required".into(),
        ));
    }
    Ok(())
}

pub(super) fn resolve_client_ip(
    headers: &HeaderMap,
    addr: SocketAddr,
    state: &AppState,
) -> AppResult<IpAddr> {
    resolve_client_context(headers, addr, state).map(|resolved| resolved.ip)
}

pub(super) fn resolve_client_context(
    headers: &HeaderMap,
    addr: SocketAddr,
    state: &AppState,
) -> AppResult<ResolvedClientIp> {
    crate::mw::rate_limit::resolve_client_ip(headers, Some(addr), &state.config.trusted_proxy_ips)
        .ok_or_else(|| AppError::Internal("unable to resolve client IP".to_string()))
}

pub(super) fn auth_device_attribution(value: ClientIpAttribution) -> AuthDeviceClientIpAttribution {
    match value {
        ClientIpAttribution::Verified => AuthDeviceClientIpAttribution::Verified,
        ClientIpAttribution::Unverified => AuthDeviceClientIpAttribution::Unverified,
        ClientIpAttribution::Unavailable => AuthDeviceClientIpAttribution::Unavailable,
    }
}

pub(super) fn trusted_client_location(
    headers: &HeaderMap,
    peer: SocketAddr,
    trusted_proxies: &[crate::config::TrustedProxyRange],
) -> auth_device_service::TrustedClientLocation {
    if !crate::mw::rate_limit::is_trusted_proxy(peer.ip(), trusted_proxies) {
        return Default::default();
    }

    // Cloudflare also offers coordinates, postal codes, and metro codes. Those
    // are intentionally not collected: city-level recognition is sufficient
    // for this approval check and avoids retaining unnecessary precise location.
    auth_device_service::TrustedClientLocation {
        country: auth_device_service::normalize_client_country(header_text(
            headers,
            "cf-ipcountry",
        )),
        city: auth_device_service::normalize_geo_label(header_text(headers, "cf-ipcity")),
        region: auth_device_service::normalize_geo_label(header_text(headers, "cf-region")),
        continent: auth_device_service::normalize_client_continent(header_text(
            headers,
            "cf-ipcontinent",
        )),
        timezone: auth_device_service::normalize_client_timezone(header_text(
            headers,
            "cf-timezone",
        )),
    }
}

pub(super) fn header_text(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|header| header.to_str().ok())
        .map(String::from)
}

pub(super) fn capture_client_context(
    headers: &HeaderMap,
    addr: SocketAddr,
    state: &AppState,
    body: AuthDeviceRequestBody,
) -> AppResult<InitiateInput> {
    let resolved_client = resolve_client_context(headers, addr, state)?;
    let client_ip = resolved_client.ip;
    let location = trusted_client_location(headers, addr, &state.config.trusted_proxy_ips);
    let origin = auth_device_service::classify_initiating_origin(
        header_text(headers, header::ORIGIN.as_str()).as_deref(),
        &state.config.frontend_url,
    );

    Ok(InitiateInput {
        requested_profile: body.requested_profile,
        client_label: body.client_label,
        client_user_agent: body.client_user_agent,
        client_ip: Some(client_ip.to_string()),
        client_ip_attribution: auth_device_attribution(resolved_client.attribution),
        client_country: location.country,
        client_city: location.city,
        client_region: location.region,
        client_continent: location.continent,
        client_ip_timezone: location.timezone,
        initiating_origin: origin.origin,
        initiating_origin_status: origin.status,
        client_app: body.client_app,
        client_platform: body.client_platform,
        client_model: body.client_model,
        client_form_factor: body.client_form_factor,
        client_timezone: body.client_timezone,
        client_locale: body.client_locale,
        client_screen_width: body.client_screen_width,
        client_screen_height: body.client_screen_height,
        client_device_pixel_ratio: body.client_device_pixel_ratio,
        client_hardware_concurrency: body.client_hardware_concurrency,
        client_device_memory: body.client_device_memory,
    })
}
