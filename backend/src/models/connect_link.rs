use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;
use crate::redaction::RedactedLen;

pub const COLLECTION_NAME: &str = "connect_links";

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectLinkStatus {
    Pending,
    Completed,
    Expired,
    Cancelled,
}

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectLinkWebhookStatus {
    Pending,
    Delivered,
    Abandoned,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ConnectLink {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub service_slug: String,
    pub service_id: String,
    pub label: Option<String>,
    pub requested_by: Option<String>,
    #[serde(default)]
    pub requesting_app_id: Option<String>,
    #[serde(default)]
    pub requesting_app_name: Option<String>,
    pub token_hash: String,
    pub status: ConnectLinkStatus,
    pub callback_url: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    pub completed_user_service_id: Option<String>,
    /// Short-lived claim that serializes provisioning attempts. It is cleared
    /// after a normal failure or after the provisioned service id is stored.
    pub completion_claim_id: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub completion_claim_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_error_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub webhook_event_reserved_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub webhook_event_id: Option<String>,
    #[serde(default)]
    pub webhook_event_status: Option<ConnectLinkWebhookStatus>,
    #[serde(default)]
    pub webhook_event_attempts: u32,
    #[serde(default, with = "bson_datetime::optional")]
    pub webhook_event_delivered_at: Option<DateTime<Utc>>,
}

impl fmt::Debug for ConnectLinkStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pending => "Pending",
            Self::Completed => "Completed",
            Self::Expired => "Expired",
            Self::Cancelled => "Cancelled",
        })
    }
}

impl fmt::Debug for ConnectLinkWebhookStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Pending => "Pending",
            Self::Delivered => "Delivered",
            Self::Abandoned => "Abandoned",
        })
    }
}

impl fmt::Debug for ConnectLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ConnectLink")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("service_slug", &self.service_slug)
            .field("service_id", &self.service_id)
            .field("label", &self.label)
            .field("requested_by", &self.requested_by)
            .field("requesting_app_id", &self.requesting_app_id)
            .field("requesting_app_name", &self.requesting_app_name)
            .field("token_hash", &RedactedLen(self.token_hash.len()))
            .field("status", &self.status)
            .field(
                "callback_url",
                &self
                    .callback_url
                    .as_ref()
                    .map(|callback_url| RedactedLen(callback_url.len())),
            )
            .field("created_at", &self.created_at)
            .field("completed_at", &self.completed_at)
            .field("expires_at", &self.expires_at)
            .field("completed_user_service_id", &self.completed_user_service_id)
            .field(
                "completion_claim_id",
                &self
                    .completion_claim_id
                    .as_ref()
                    .map(|claim_id| RedactedLen(claim_id.len())),
            )
            .field("completion_claim_at", &self.completion_claim_at)
            .field("last_error", &self.last_error)
            .field("last_error_at", &self.last_error_at)
            .field("webhook_event_reserved_at", &self.webhook_event_reserved_at)
            .field("webhook_event_id", &self.webhook_event_id)
            .field("webhook_event_status", &self.webhook_event_status)
            .field("webhook_event_attempts", &self.webhook_event_attempts)
            .field(
                "webhook_event_delivered_at",
                &self.webhook_event_delivered_at,
            )
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> ConnectLink {
        let now = Utc::now();
        ConnectLink {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            service_slug: "api-github-pat".to_string(),
            service_id: uuid::Uuid::new_v4().to_string(),
            label: Some("Release automation".to_string()),
            requested_by: Some("codex-release".to_string()),
            requesting_app_id: Some("desktop-client".to_string()),
            requesting_app_name: Some("Desktop Client".to_string()),
            token_hash: "ab".repeat(32),
            status: ConnectLinkStatus::Pending,
            callback_url: Some("https://app.example.test/connected".to_string()),
            created_at: now,
            completed_at: None,
            expires_at: now + chrono::Duration::minutes(15),
            completed_user_service_id: None,
            completion_claim_id: Some("completion-claim-secret".to_string()),
            completion_claim_at: Some(now),
            last_error: Some("provider_access_denied".to_string()),
            last_error_at: Some(now),
            webhook_event_reserved_at: None,
            webhook_event_id: None,
            webhook_event_status: None,
            webhook_event_attempts: 0,
            webhook_event_delivered_at: None,
        }
    }

    #[test]
    fn collection_name_is_connect_links() {
        assert_eq!(COLLECTION_NAME, "connect_links");
    }

    #[test]
    fn bson_roundtrip_preserves_dates_and_status() {
        let link = fixture();
        let document = bson::to_document(&link).expect("serialize connect link");
        let restored: ConnectLink =
            bson::from_document(document).expect("deserialize connect link");

        assert_eq!(restored.id, link.id);
        assert_eq!(restored.status, ConnectLinkStatus::Pending);
        assert_eq!(
            restored.created_at.timestamp_millis(),
            link.created_at.timestamp_millis()
        );
        assert_eq!(
            restored.expires_at.timestamp_millis(),
            link.expires_at.timestamp_millis()
        );
        assert_eq!(
            restored
                .completion_claim_at
                .expect("completion claim timestamp")
                .timestamp_millis(),
            link.completion_claim_at
                .expect("fixture completion claim timestamp")
                .timestamp_millis()
        );
    }

    #[test]
    fn debug_redacts_token_hash() {
        let link = fixture();
        let debug = format!("{link:?}");

        assert!(!debug.contains(&link.token_hash));
        assert!(!debug.contains("completion-claim-secret"));
        assert!(!debug.contains("app.example.test"));
        assert!(debug.contains("redacted"));
        assert!(debug.contains("api-github-pat"));
    }
}
