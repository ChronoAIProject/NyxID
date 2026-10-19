use super::auth_device_code::{AuthDeviceClientIpAttribution, AuthDeviceInitiatingOriginStatus};
use serde::{Deserialize, Serialize};

#[derive(Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LoginClientContext {
    pub requested_profile: Option<String>,
    pub client_label: Option<String>,
    pub client_user_agent: Option<String>,
    pub client_ip: Option<String>,
    pub client_ip_attribution: AuthDeviceClientIpAttribution,
    pub client_country: Option<String>,
    pub client_city: Option<String>,
    pub client_region: Option<String>,
    pub client_continent: Option<String>,
    pub client_ip_timezone: Option<String>,
    pub initiating_origin: Option<String>,
    pub initiating_origin_status: AuthDeviceInitiatingOriginStatus,
    pub client_app: Option<String>,
    pub client_platform: Option<String>,
    pub client_model: Option<String>,
    pub client_form_factor: Option<String>,
    pub client_timezone: Option<String>,
    pub client_locale: Option<String>,
    pub client_screen_width: Option<u32>,
    pub client_screen_height: Option<u32>,
    pub client_device_pixel_ratio: Option<f64>,
    pub client_hardware_concurrency: Option<u16>,
    pub client_device_memory: Option<f64>,
}

impl std::fmt::Debug for LoginClientContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginClientContext").finish_non_exhaustive()
    }
}
