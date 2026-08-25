use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use super::bson_datetime;
use crate::redaction::RedactedLen;

pub const COLLECTION_NAME: &str = "auth_device_codes";

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthDeviceClientIpAttribution {
    Verified,
    #[default]
    Unverified,
    Unavailable,
}

impl AuthDeviceClientIpAttribution {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Unverified => "unverified",
            Self::Unavailable => "unavailable",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthDeviceInitiatingOriginStatus {
    #[default]
    Absent,
    Matched,
    Mismatched,
    Malformed,
    NonHttp,
}

impl AuthDeviceInitiatingOriginStatus {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Absent => "absent",
            Self::Matched => "matched",
            Self::Mismatched => "mismatched",
            Self::Malformed => "malformed",
            Self::NonHttp => "non_http",
        }
    }
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AuthDeviceCodeStatus {
    Pending,
    Approved,
    Denied,
    Expired,
    Delivered,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AuthDeviceCode {
    #[serde(rename = "_id")]
    pub id: String,
    pub device_code_hmac: String,
    pub user_code_hmac: String,
    pub status: AuthDeviceCodeStatus,
    pub poll_interval_secs: u32,
    pub slow_down_increments: u32,
    pub client_label: Option<String>,
    pub client_user_agent: Option<String>,
    #[serde(default)]
    pub client_ip: Option<String>,
    #[serde(default)]
    pub client_ip_attribution: AuthDeviceClientIpAttribution,
    #[serde(default)]
    pub client_country: Option<String>,
    #[serde(default)]
    pub client_city: Option<String>,
    #[serde(default)]
    pub client_region: Option<String>,
    #[serde(default)]
    pub client_continent: Option<String>,
    #[serde(default)]
    pub client_ip_timezone: Option<String>,
    #[serde(default)]
    pub initiating_origin: Option<String>,
    #[serde(default)]
    pub initiating_origin_status: AuthDeviceInitiatingOriginStatus,
    #[serde(default)]
    pub client_app: Option<String>,
    #[serde(default)]
    pub client_platform: Option<String>,
    #[serde(default)]
    pub client_model: Option<String>,
    #[serde(default)]
    pub client_form_factor: Option<String>,
    #[serde(default)]
    pub client_timezone: Option<String>,
    #[serde(default)]
    pub client_locale: Option<String>,
    #[serde(default)]
    pub client_screen_width: Option<u32>,
    #[serde(default)]
    pub client_screen_height: Option<u32>,
    #[serde(default)]
    pub client_device_pixel_ratio: Option<f64>,
    #[serde(default)]
    pub client_hardware_concurrency: Option<u16>,
    #[serde(default)]
    pub client_device_memory: Option<f64>,
    pub client_ip_hmac: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_polled_at: Option<DateTime<Utc>>,
    pub approved_user_id: Option<String>,
    pub approved_session_id: Option<String>,
    pub approver_ip_hmac: Option<String>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub delivery_access_token_encrypted: Option<Vec<u8>>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub delivery_refresh_token_encrypted: Option<Vec<u8>>,
    pub delivery_access_token_expires_in: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub approved_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub delivered_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub denied_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub denied_by_user_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl fmt::Debug for AuthDeviceCodeStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pending => f.write_str("Pending"),
            Self::Approved => f.write_str("Approved"),
            Self::Denied => f.write_str("Denied"),
            Self::Expired => f.write_str("Expired"),
            Self::Delivered => f.write_str("Delivered"),
        }
    }
}

impl fmt::Debug for AuthDeviceCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AuthDeviceCode")
            .field("id", &self.id)
            .field(
                "device_code_hmac",
                &RedactedLen(self.device_code_hmac.len()),
            )
            .field("user_code_hmac", &RedactedLen(self.user_code_hmac.len()))
            .field("status", &self.status)
            .field("poll_interval_secs", &self.poll_interval_secs)
            .field("slow_down_increments", &self.slow_down_increments)
            .field(
                "client_label",
                &self
                    .client_label
                    .as_ref()
                    .map(|value| RedactedLen(value.len())),
            )
            .field(
                "client_user_agent",
                &self
                    .client_user_agent
                    .as_ref()
                    .map(|value| RedactedLen(value.len())),
            )
            .field(
                "client_ip",
                &self.client_ip.as_ref().map(|ip| RedactedLen(ip.len())),
            )
            .field("client_ip_attribution", &self.client_ip_attribution)
            .field(
                "client_ip_hmac",
                &self
                    .client_ip_hmac
                    .as_ref()
                    .map(|hash| RedactedLen(hash.len())),
            )
            .field("last_polled_at", &self.last_polled_at)
            .field("approved_user_id", &self.approved_user_id)
            .field("approved_session_id", &self.approved_session_id)
            .field(
                "approver_ip_hmac",
                &self
                    .approver_ip_hmac
                    .as_ref()
                    .map(|hash| RedactedLen(hash.len())),
            )
            .field(
                "delivery_access_token_encrypted",
                &self
                    .delivery_access_token_encrypted
                    .as_ref()
                    .map(|bytes| RedactedLen(bytes.len())),
            )
            .field(
                "delivery_refresh_token_encrypted",
                &self
                    .delivery_refresh_token_encrypted
                    .as_ref()
                    .map(|bytes| RedactedLen(bytes.len())),
            )
            .field(
                "delivery_access_token_expires_in",
                &self.delivery_access_token_expires_in,
            )
            .field("created_at", &self.created_at)
            .field("approved_at", &self.approved_at)
            .field("delivered_at", &self.delivered_at)
            .field("denied_at", &self.denied_at)
            .field("denied_by_user_id", &self.denied_by_user_id)
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_auth_device_code() -> AuthDeviceCode {
        let now = Utc::now();
        AuthDeviceCode {
            id: uuid::Uuid::new_v4().to_string(),
            device_code_hmac: "abc123ff".repeat(8),
            user_code_hmac: "def456aa".repeat(8),
            status: AuthDeviceCodeStatus::Pending,
            poll_interval_secs: 5,
            slow_down_increments: 0,
            client_label: Some("wsl-calvin".to_string()),
            client_user_agent: Some("nyxid-cli/0.8.0".to_string()),
            client_ip: Some("203.0.113.10".to_string()),
            client_ip_attribution: AuthDeviceClientIpAttribution::Verified,
            client_country: Some("SG".to_string()),
            client_city: Some("Singapore".to_string()),
            client_region: Some("Singapore".to_string()),
            client_continent: Some("AS".to_string()),
            client_ip_timezone: Some("Asia/Singapore".to_string()),
            initiating_origin: Some("https://nyxid.dev".to_string()),
            initiating_origin_status: AuthDeviceInitiatingOriginStatus::Matched,
            client_app: Some("Chrome 131".to_string()),
            client_platform: Some("macOS 15.2 (arm64)".to_string()),
            client_model: None,
            client_form_factor: Some("desktop".to_string()),
            client_timezone: Some("Asia/Singapore".to_string()),
            client_locale: Some("en-SG".to_string()),
            client_screen_width: Some(1512),
            client_screen_height: Some(982),
            client_device_pixel_ratio: Some(2.0),
            client_hardware_concurrency: Some(12),
            client_device_memory: Some(16.0),
            client_ip_hmac: Some("11112222".repeat(8)),
            last_polled_at: Some(now + chrono::Duration::seconds(5)),
            approved_user_id: Some(uuid::Uuid::new_v4().to_string()),
            approved_session_id: Some(uuid::Uuid::new_v4().to_string()),
            approver_ip_hmac: Some("33334444".repeat(8)),
            delivery_access_token_encrypted: Some(vec![0xab, 0xcd, 0xef]),
            delivery_refresh_token_encrypted: Some(vec![0x12, 0x34, 0x56]),
            delivery_access_token_expires_in: Some(900),
            created_at: now,
            approved_at: Some(now + chrono::Duration::seconds(10)),
            delivered_at: Some(now + chrono::Duration::seconds(20)),
            denied_at: None,
            denied_by_user_id: None,
            expires_at: now + chrono::Duration::minutes(10),
        }
    }

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "auth_device_codes");
    }

    #[test]
    fn status_serializes_as_lowercase() {
        let value = serde_json::to_value(AuthDeviceCodeStatus::Pending).expect("serialize status");
        assert_eq!(value, serde_json::json!("pending"));
    }

    #[test]
    fn bson_roundtrip_preserves_struct_identity() {
        let row = make_auth_device_code();
        let doc = bson::to_document(&row).expect("serialize");
        let restored: AuthDeviceCode = bson::from_document(doc).expect("deserialize");

        assert_eq!(row.id, restored.id);
        assert_eq!(row.device_code_hmac, restored.device_code_hmac);
        assert_eq!(row.user_code_hmac, restored.user_code_hmac);
        assert_eq!(row.status, restored.status);
        assert_eq!(row.poll_interval_secs, restored.poll_interval_secs);
        assert_eq!(row.slow_down_increments, restored.slow_down_increments);
        assert_eq!(row.client_label, restored.client_label);
        assert_eq!(row.client_user_agent, restored.client_user_agent);
        assert_eq!(row.client_ip, restored.client_ip);
        assert_eq!(row.client_ip_attribution, restored.client_ip_attribution);
        assert_eq!(row.client_country, restored.client_country);
        assert_eq!(row.client_city, restored.client_city);
        assert_eq!(row.client_region, restored.client_region);
        assert_eq!(row.client_continent, restored.client_continent);
        assert_eq!(row.client_ip_timezone, restored.client_ip_timezone);
        assert_eq!(row.initiating_origin, restored.initiating_origin);
        assert_eq!(
            row.initiating_origin_status,
            restored.initiating_origin_status
        );
        assert_eq!(row.client_app, restored.client_app);
        assert_eq!(row.client_platform, restored.client_platform);
        assert_eq!(row.client_model, restored.client_model);
        assert_eq!(row.client_form_factor, restored.client_form_factor);
        assert_eq!(row.client_timezone, restored.client_timezone);
        assert_eq!(row.client_locale, restored.client_locale);
        assert_eq!(row.client_screen_width, restored.client_screen_width);
        assert_eq!(row.client_screen_height, restored.client_screen_height);
        assert_eq!(
            row.client_device_pixel_ratio,
            restored.client_device_pixel_ratio
        );
        assert_eq!(
            row.client_hardware_concurrency,
            restored.client_hardware_concurrency
        );
        assert_eq!(row.client_device_memory, restored.client_device_memory);
        assert_eq!(row.client_ip_hmac, restored.client_ip_hmac);
        assert_eq!(row.approved_user_id, restored.approved_user_id);
        assert_eq!(row.approved_session_id, restored.approved_session_id);
        assert_eq!(row.approver_ip_hmac, restored.approver_ip_hmac);
        assert_eq!(
            row.delivery_access_token_encrypted,
            restored.delivery_access_token_encrypted
        );
        assert_eq!(
            row.delivery_refresh_token_encrypted,
            restored.delivery_refresh_token_encrypted
        );
        assert_eq!(
            row.delivery_access_token_expires_in,
            restored.delivery_access_token_expires_in
        );
        assert_eq!(
            row.created_at.timestamp_millis(),
            restored.created_at.timestamp_millis()
        );
        assert_eq!(
            row.last_polled_at.unwrap().timestamp_millis(),
            restored.last_polled_at.unwrap().timestamp_millis()
        );
        assert_eq!(
            row.approved_at.unwrap().timestamp_millis(),
            restored.approved_at.unwrap().timestamp_millis()
        );
        assert_eq!(
            row.delivered_at.unwrap().timestamp_millis(),
            restored.delivered_at.unwrap().timestamp_millis()
        );
        assert_eq!(row.denied_at, restored.denied_at);
        assert_eq!(row.denied_by_user_id, restored.denied_by_user_id);
        assert_eq!(
            row.expires_at.timestamp_millis(),
            restored.expires_at.timestamp_millis()
        );
    }

    #[test]
    fn debug_redacts_hashes_and_ciphertext_but_prints_safe_fields() {
        let row = make_auth_device_code();
        let debug = format!("{row:?}");

        for secret in [
            row.device_code_hmac.as_str(),
            row.user_code_hmac.as_str(),
            row.client_ip_hmac.as_deref().unwrap(),
            row.client_ip.as_deref().unwrap(),
            row.client_country.as_deref().unwrap(),
            row.approver_ip_hmac.as_deref().unwrap(),
            "abcdef",
            "123456",
        ] {
            assert!(!debug.contains(secret), "{secret} leaked in {debug}");
        }

        assert!(debug.contains("Pending"));
        assert!(debug.contains("created_at"));
        assert!(debug.contains("expires_at"));
        assert!(!debug.contains("wsl-calvin"));
        assert!(!debug.contains("nyxid-cli/0.8.0"));
    }

    #[test]
    fn legacy_bson_without_new_optional_fields_deserializes() {
        let row = make_auth_device_code();
        let mut doc = bson::to_document(&row).expect("serialize");
        doc.remove("client_ip");
        doc.remove("denied_by_user_id");

        let restored: AuthDeviceCode = bson::from_document(doc).expect("deserialize legacy row");

        assert!(restored.client_ip.is_none());
        assert!(restored.denied_by_user_id.is_none());
    }

    #[test]
    fn legacy_row_without_client_country_deserializes_to_none() {
        let mut doc = bson::to_document(&make_auth_device_code()).expect("serialize");
        doc.remove("client_country");

        let restored: AuthDeviceCode = bson::from_document(doc).expect("deserialize legacy row");

        assert!(restored.client_country.is_none());
    }

    #[test]
    fn legacy_row_without_rich_browser_context_uses_neutral_defaults() {
        let mut doc = bson::to_document(&make_auth_device_code()).expect("serialize");
        for field in [
            "client_city",
            "client_region",
            "client_continent",
            "client_ip_timezone",
            "initiating_origin",
            "initiating_origin_status",
            "client_app",
            "client_platform",
            "client_model",
            "client_form_factor",
            "client_timezone",
            "client_locale",
            "client_screen_width",
            "client_screen_height",
            "client_device_pixel_ratio",
            "client_hardware_concurrency",
            "client_device_memory",
        ] {
            doc.remove(field);
        }

        let restored: AuthDeviceCode = bson::from_document(doc).expect("deserialize legacy row");
        assert_eq!(
            restored.initiating_origin_status,
            AuthDeviceInitiatingOriginStatus::Absent
        );
        assert!(restored.initiating_origin.is_none());
        assert!(restored.client_city.is_none());
        assert!(restored.client_ip_timezone.is_none());
        assert!(restored.client_timezone.is_none());
        assert!(restored.client_device_memory.is_none());
    }

    #[test]
    fn legacy_row_without_client_ip_attribution_is_unverified() {
        let mut doc = bson::to_document(&make_auth_device_code()).expect("serialize");
        doc.remove("client_ip_attribution");

        let restored: AuthDeviceCode = bson::from_document(doc).expect("deserialize legacy row");

        assert_eq!(
            restored.client_ip_attribution,
            AuthDeviceClientIpAttribution::Unverified
        );
    }
}
