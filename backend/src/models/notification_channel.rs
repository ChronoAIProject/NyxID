use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "notification_channels";

/// A registered push notification device token (FCM or APNs).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeviceToken {
    /// Unique device ID (UUID v4 string, generated server-side)
    pub device_id: String,

    /// Platform: "fcm" or "apns"
    pub platform: String,

    /// The device registration token from FCM or APNs
    pub token: String,

    /// Human-readable device name (e.g. "iPhone 15", "Pixel 8")
    pub device_name: Option<String>,

    /// App bundle ID / package name (e.g. "dev.nyxid.app").
    /// Used for APNs apns-topic header.
    pub app_id: Option<String>,

    /// When the token was registered or last refreshed
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub registered_at: DateTime<Utc>,

    /// When a push was last successfully sent to this token
    #[serde(default, with = "bson_datetime::optional")]
    pub last_used_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NotificationChannel {
    /// UUID v4 string
    #[serde(rename = "_id")]
    pub id: String,

    /// Owner user ID (unique per user -- one notification config per user)
    pub user_id: String,

    // -- Telegram --
    /// Telegram chat ID for sending messages
    pub telegram_chat_id: Option<i64>,

    /// Telegram username (for display in settings UI)
    pub telegram_username: Option<String>,

    /// Whether Telegram notifications are enabled
    #[serde(default)]
    pub telegram_enabled: bool,

    /// One-time linking code for connecting Telegram account.
    pub telegram_link_code: Option<String>,

    /// Expiry for the link code (5 minutes)
    #[serde(default, with = "bson_datetime::optional")]
    pub telegram_link_code_expires_at: Option<DateTime<Utc>>,

    // -- User preferences --
    /// How long to wait for user response before auto-rejecting (seconds).
    /// Default: 30. Min: 10. Max: 300.
    #[serde(default = "default_approval_timeout")]
    pub approval_timeout_secs: u32,

    /// How many days an approval grant lasts before re-prompting.
    /// Default: 30. Min: 1. Max: 365.
    #[serde(default = "default_grant_expiry_days")]
    pub grant_expiry_days: u32,

    /// Whether approval is required for proxy/LLM requests using this user's
    /// credentials. When false, all requests are auto-approved (legacy behavior).
    #[serde(default)]
    pub approval_required: bool,

    // -- Push Notifications --
    /// Whether push notifications (FCM/APNs) are enabled
    #[serde(default)]
    pub push_enabled: bool,

    /// Registered device tokens for push notifications
    #[serde(default)]
    pub push_devices: Vec<DeviceToken>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

fn default_approval_timeout() -> u32 {
    30
}

fn default_grant_expiry_days() -> u32 {
    30
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "notification_channels");
    }

    fn make_notification_channel() -> NotificationChannel {
        NotificationChannel {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            telegram_chat_id: Some(12345),
            telegram_username: Some("testuser".to_string()),
            telegram_enabled: true,
            telegram_link_code: None,
            telegram_link_code_expires_at: None,
            approval_timeout_secs: 30,
            grant_expiry_days: 30,
            approval_required: true,
            push_enabled: false,
            push_devices: vec![],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let ch = make_notification_channel();
        let doc = bson::to_document(&ch).expect("serialize");
        assert!(doc.get_str("_id").is_ok());
        assert!(doc.get("id").is_none(), "raw 'id' should not exist in bson");
        let restored: NotificationChannel = bson::from_document(doc).expect("deserialize");
        assert_eq!(ch.id, restored.id);
        assert_eq!(ch.user_id, restored.user_id);
        assert_eq!(ch.telegram_enabled, restored.telegram_enabled);
        assert_eq!(ch.approval_required, restored.approval_required);
    }

    #[test]
    fn bson_roundtrip_with_optional_datetime() {
        let mut ch = make_notification_channel();
        ch.telegram_link_code = Some("NYXID-A1B2C3".to_string());
        ch.telegram_link_code_expires_at = Some(Utc::now());
        let doc = bson::to_document(&ch).expect("serialize");
        let restored: NotificationChannel = bson::from_document(doc).expect("deserialize");
        assert!(restored.telegram_link_code_expires_at.is_some());
    }

    #[test]
    fn bson_all_fields_serialized() {
        let ch = make_notification_channel();
        let doc = bson::to_document(&ch).expect("serialize");
        let keys: Vec<&str> = doc.keys().map(|k| k.as_str()).collect();
        assert!(keys.contains(&"_id"));
        assert!(keys.contains(&"user_id"));
        assert!(keys.contains(&"telegram_enabled"));
        assert!(keys.contains(&"approval_timeout_secs"));
        assert!(keys.contains(&"grant_expiry_days"));
        assert!(keys.contains(&"approval_required"));
        assert!(keys.contains(&"push_enabled"));
        assert!(keys.contains(&"push_devices"));
        assert!(keys.contains(&"created_at"));
        assert!(keys.contains(&"updated_at"));
    }

    #[test]
    fn bson_roundtrip_with_push_devices() {
        let mut ch = make_notification_channel();
        ch.push_enabled = true;
        ch.push_devices = vec![
            DeviceToken {
                device_id: uuid::Uuid::new_v4().to_string(),
                platform: "fcm".to_string(),
                token: "test-fcm-token-abc".to_string(),
                device_name: Some("Pixel 8".to_string()),
                app_id: Some("dev.nyxid.app".to_string()),
                registered_at: Utc::now(),
                last_used_at: None,
            },
            DeviceToken {
                device_id: uuid::Uuid::new_v4().to_string(),
                platform: "apns".to_string(),
                token: "test-apns-token-def".to_string(),
                device_name: Some("iPhone 15".to_string()),
                app_id: Some("dev.nyxid.app".to_string()),
                registered_at: Utc::now(),
                last_used_at: Some(Utc::now()),
            },
        ];
        let doc = bson::to_document(&ch).expect("serialize");
        let restored: NotificationChannel = bson::from_document(doc).expect("deserialize");
        assert!(restored.push_enabled);
        assert_eq!(restored.push_devices.len(), 2);
        assert_eq!(restored.push_devices[0].platform, "fcm");
        assert_eq!(restored.push_devices[1].platform, "apns");
        assert!(restored.push_devices[1].last_used_at.is_some());
    }

    #[test]
    fn defaults() {
        assert_eq!(default_approval_timeout(), 30);
        assert_eq!(default_grant_expiry_days(), 30);
    }
}
