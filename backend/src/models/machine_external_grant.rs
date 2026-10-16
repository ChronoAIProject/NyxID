//! Machine resource authorization over existing OAuth clients and parent keys.
//! A child credential is attribution, never a grant identity.
use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{bson_datetime, machine_access::Policy};

pub const COLLECTION_NAME: &str = "machine_external_grants";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExternalGrantBinding {
    #[serde(rename = "oauth")]
    OAuth {
        oauth_client_id: String,
        consent_id: String,
        consent_revision: String,
    },
    ApiKey {
        api_key_id: String,
        /// Evidence of the human's approval CAS, never a runtime equality gate.
        api_key_state_version_at_approval: i64,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantStatus {
    Active,
    Revoked,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MachineRestrictions {
    pub file_roots: Vec<String>,
    /// Explicit UserService IDs. Empty means no gateway services.
    pub service_ids: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MachineExternalGrant {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub binding: ExternalGrantBinding,
    pub policy: Policy,
    pub restrictions: BTreeMap<String, MachineRestrictions>,
    /// Whole-grant execution fence, independent of Policy/Assignment revisions.
    pub revision: i64,
    pub status: GrantStatus,
    /// Writes by admission/renewal conflict with grant edits and revocation.
    pub admission_fence: i64,
    /// Source disable/re-enable fences; zero is the pre-Machine source epoch.
    pub user_security_revision: i64,
    pub client_security_revision: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub revoked_at: Option<DateTime<Utc>>,
    pub revoked_reason: Option<String>,
}

/// Durable cancellation selectors for the next protocol revision. They are not
/// serialized into the legacy Assistant-only machine_revocations collection.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ExternalRevocationSelector {
    #[serde(rename = "oauth_principal")]
    OAuthPrincipal {
        user_id: String,
        oauth_client_id: String,
    },
    ApiKeyPrincipal {
        user_id: String,
        api_key_id: String,
    },
    Child {
        user_id: String,
        api_key_id: String,
        api_key_credential_id: String,
    },
    Grant {
        user_id: String,
        grant_id: String,
        through_revision: i64,
    },
}

/// Existing verified identity reference for persistent job attribution. The
/// child is deliberately absent from ExternalGrantBinding (the lookup key).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ExternalPrincipalRef {
    #[serde(rename = "oauth")]
    OAuth {
        user_id: String,
        oauth_client_id: String,
    },
    ApiKey {
        user_id: String,
        api_key_id: String,
        api_key_credential_id: Option<String>,
    },
}

impl std::fmt::Debug for ExternalGrantBinding {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ExternalGrantBinding { [REDACTED] }")
    }
}

impl std::fmt::Debug for ExternalRevocationSelector {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ExternalRevocationSelector { [REDACTED] }")
    }
}

impl std::fmt::Debug for ExternalPrincipalRef {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ExternalPrincipalRef { [REDACTED] }")
    }
}

#[cfg(test)]
mod debug_tests {
    use super::*;

    #[test]
    fn external_source_identifiers_are_redacted_without_changing_serialization() {
        let key = "sentinel-parent-key-id";
        let child = "sentinel-child-key-id";
        let binding = ExternalGrantBinding::ApiKey {
            api_key_id: key.into(),
            api_key_state_version_at_approval: 7,
        };
        let principal = ExternalPrincipalRef::ApiKey {
            user_id: "sentinel-owner".into(),
            api_key_id: key.into(),
            api_key_credential_id: Some(child.into()),
        };
        let selector = ExternalRevocationSelector::Child {
            user_id: "sentinel-owner".into(),
            api_key_id: key.into(),
            api_key_credential_id: child.into(),
        };
        for debug in [
            format!("{binding:?}"),
            format!("{binding:#?}"),
            format!("{principal:?}"),
            format!("{principal:#?}"),
            format!("{selector:?}"),
            format!("{selector:#?}"),
        ] {
            for value in [key, child, "sentinel-owner"] {
                assert!(!debug.contains(value));
            }
            assert!(debug.contains("REDACTED"));
        }
        assert_eq!(serde_json::to_value(&principal).unwrap()["api_key_id"], key);
        assert_eq!(
            serde_json::to_value(&principal).unwrap()["api_key_credential_id"],
            child
        );
        assert_eq!(
            serde_json::to_value(&selector).unwrap()["api_key_credential_id"],
            child
        );
        assert_eq!(
            bson::from_document::<ExternalGrantBinding>(bson::to_document(&binding).unwrap())
                .unwrap(),
            binding
        );
    }
}
