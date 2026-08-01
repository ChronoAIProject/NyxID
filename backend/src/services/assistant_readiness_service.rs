use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::errors::{AppError, AppResult};

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ReadinessStatus {
    Available,
    Missing,
    CannotUse,
    CannotCheck,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConnectionState {
    NotConnected,
    Connecting,
    Verifying,
    Connected,
    Expired,
    Revoked,
    Unknown,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantState {
    NotRequired,
    Granted,
    Partial,
    Missing,
    Expired,
    Revoked,
    Unknown,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessCapability {
    pub capability_id: String,
    pub label: String,
    pub required: bool,
    pub status: ReadinessStatus,
    pub connection_state: ConnectionState,
    pub grant_state: GrantState,
    pub requested_scopes: Vec<String>,
    pub management_url: Option<String>,
    pub reason_code: Option<String>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ReadinessSnapshot {
    pub revision: String,
    pub evaluated_at: DateTime<Utc>,
    pub capabilities: Vec<ReadinessCapability>,
}

fn derive_status(
    connection_state: ConnectionState,
    grant_state: GrantState,
    cannot_use: bool,
) -> ReadinessStatus {
    if cannot_use {
        return ReadinessStatus::CannotUse;
    }
    if connection_state == ConnectionState::Unknown || grant_state == GrantState::Unknown {
        return ReadinessStatus::CannotCheck;
    }
    if connection_state == ConnectionState::Connected
        && matches!(grant_state, GrantState::Granted | GrantState::NotRequired)
    {
        return ReadinessStatus::Available;
    }
    ReadinessStatus::Missing
}

fn build_management_url(frontend_url: &str) -> Option<String> {
    let mut url = Url::parse(frontend_url).ok()?;
    if url.scheme() != "https"
        || url.cannot_be_a_base()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.host_str().is_none()
    {
        return None;
    }
    url.set_path("/keys");
    url.set_query(None);
    url.set_fragment(None);
    Some(url.to_string())
}

fn merge_capability(existing: &mut ReadinessCapability, incoming: ReadinessCapability) {
    let conflicting_evidence = existing.connection_state != incoming.connection_state
        || existing.grant_state != incoming.grant_state;
    let cannot_use = existing.status == ReadinessStatus::CannotUse
        || incoming.status == ReadinessStatus::CannotUse;

    if incoming.label < existing.label {
        existing.label = incoming.label;
    }
    existing.required |= incoming.required;
    if existing.connection_state != incoming.connection_state {
        existing.connection_state = ConnectionState::Unknown;
    }
    if existing.grant_state != incoming.grant_state {
        existing.grant_state = GrantState::Unknown;
    }
    if existing.management_url != incoming.management_url {
        existing.management_url = None;
    }
    existing.requested_scopes.clear();
    existing.status = derive_status(existing.connection_state, existing.grant_state, cannot_use);
    if conflicting_evidence {
        existing.reason_code = Some("conflicting_evidence".to_string());
    } else if incoming.reason_code < existing.reason_code {
        existing.reason_code = incoming.reason_code;
    }
}

pub fn build_snapshot(
    capabilities: Vec<ReadinessCapability>,
    evaluated_at: DateTime<Utc>,
) -> AppResult<ReadinessSnapshot> {
    let mut by_id = BTreeMap::<String, ReadinessCapability>::new();
    for mut capability in capabilities {
        capability.requested_scopes.clear();
        capability.status = derive_status(
            capability.connection_state,
            capability.grant_state,
            capability.status == ReadinessStatus::CannotUse,
        );
        match by_id.entry(capability.capability_id.clone()) {
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(capability);
            }
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                merge_capability(entry.get_mut(), capability);
            }
        }
    }

    let mut capabilities: Vec<_> = by_id.into_values().collect();
    capabilities.sort_by(|left, right| {
        right
            .required
            .cmp(&left.required)
            .then_with(|| left.capability_id.cmp(&right.capability_id))
    });
    let bytes = serde_json::to_vec(&capabilities).map_err(|_| {
        AppError::Internal("assistant: failed to encode readiness revision".to_string())
    })?;
    let revision = hex::encode(Sha256::digest(bytes));

    Ok(ReadinessSnapshot {
        revision,
        evaluated_at,
        capabilities,
    })
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use serde_json::json;

    use super::*;

    fn capability(
        capability_id: &str,
        required: bool,
        connection_state: ConnectionState,
        grant_state: GrantState,
    ) -> ReadinessCapability {
        ReadinessCapability {
            capability_id: capability_id.to_string(),
            label: capability_id.to_string(),
            required,
            status: derive_status(connection_state, grant_state, false),
            connection_state,
            grant_state,
            requested_scopes: Vec::new(),
            management_url: Some("https://nyx.example/keys".to_string()),
            reason_code: None,
        }
    }

    #[test]
    fn closed_enums_serialize_to_the_consumer_contract() {
        assert_eq!(
            serde_json::to_value([
                ReadinessStatus::Available,
                ReadinessStatus::Missing,
                ReadinessStatus::CannotUse,
                ReadinessStatus::CannotCheck,
            ])
            .unwrap(),
            json!(["available", "missing", "cannot_use", "cannot_check"])
        );
        assert_eq!(
            serde_json::to_value([
                ConnectionState::NotConnected,
                ConnectionState::Connecting,
                ConnectionState::Verifying,
                ConnectionState::Connected,
                ConnectionState::Expired,
                ConnectionState::Revoked,
                ConnectionState::Unknown,
            ])
            .unwrap(),
            json!([
                "not_connected",
                "connecting",
                "verifying",
                "connected",
                "expired",
                "revoked",
                "unknown"
            ])
        );
        assert_eq!(
            serde_json::to_value([
                GrantState::NotRequired,
                GrantState::Granted,
                GrantState::Partial,
                GrantState::Missing,
                GrantState::Expired,
                GrantState::Revoked,
                GrantState::Unknown,
            ])
            .unwrap(),
            json!([
                "not_required",
                "granted",
                "partial",
                "missing",
                "expired",
                "revoked",
                "unknown"
            ])
        );
    }

    #[test]
    fn status_is_available_only_for_proven_connection_and_grant() {
        assert_eq!(
            derive_status(ConnectionState::Connected, GrantState::Granted, false),
            ReadinessStatus::Available
        );
        assert_eq!(
            derive_status(ConnectionState::Connected, GrantState::NotRequired, false),
            ReadinessStatus::Available
        );

        for grant in [
            GrantState::Partial,
            GrantState::Missing,
            GrantState::Expired,
            GrantState::Revoked,
        ] {
            assert_eq!(
                derive_status(ConnectionState::Connected, grant, false),
                ReadinessStatus::Missing
            );
        }
        assert_eq!(
            derive_status(ConnectionState::Unknown, GrantState::Granted, false),
            ReadinessStatus::CannotCheck
        );
        assert_eq!(
            derive_status(ConnectionState::Connected, GrantState::Unknown, false),
            ReadinessStatus::CannotCheck
        );
        assert_eq!(
            derive_status(ConnectionState::Connected, GrantState::Granted, true),
            ReadinessStatus::CannotUse
        );
    }

    #[test]
    fn duplicate_conflicting_evidence_fails_closed_and_required_items_sort_first() {
        let evaluated_at = Utc.with_ymd_and_hms(2026, 8, 1, 1, 2, 3).unwrap();
        let snapshot = build_snapshot(
            vec![
                capability(
                    "api-github",
                    false,
                    ConnectionState::Connected,
                    GrantState::Granted,
                ),
                capability(
                    "runtime",
                    true,
                    ConnectionState::Connected,
                    GrantState::NotRequired,
                ),
                capability(
                    "api-github",
                    false,
                    ConnectionState::Expired,
                    GrantState::Expired,
                ),
                capability(
                    "model",
                    true,
                    ConnectionState::Connected,
                    GrantState::NotRequired,
                ),
            ],
            evaluated_at,
        )
        .unwrap();

        assert_eq!(
            snapshot
                .capabilities
                .iter()
                .map(|item| item.capability_id.as_str())
                .collect::<Vec<_>>(),
            ["model", "runtime", "api-github"]
        );
        let github = snapshot.capabilities.last().unwrap();
        assert_eq!(github.connection_state, ConnectionState::Unknown);
        assert_eq!(github.grant_state, GrantState::Unknown);
        assert_eq!(github.status, ReadinessStatus::CannotCheck);
        assert_eq!(github.reason_code.as_deref(), Some("conflicting_evidence"));
    }

    #[test]
    fn revision_is_stable_for_identical_evidence_and_json_has_only_safe_contract_fields() {
        let capability = capability(
            "runtime",
            true,
            ConnectionState::Connected,
            GrantState::NotRequired,
        );
        let first = build_snapshot(
            vec![capability.clone()],
            Utc.with_ymd_and_hms(2026, 8, 1, 1, 2, 3).unwrap(),
        )
        .unwrap();
        let second = build_snapshot(
            vec![capability],
            Utc.with_ymd_and_hms(2026, 8, 1, 2, 3, 4).unwrap(),
        )
        .unwrap();

        assert_eq!(first.revision, second.revision);
        assert_eq!(first.revision.len(), 64);
        let value = serde_json::to_value(first).unwrap();
        let mut snapshot_keys = value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        snapshot_keys.sort_unstable();
        assert_eq!(snapshot_keys, ["capabilities", "evaluatedAt", "revision"]);
        let mut capability_keys = value["capabilities"][0]
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>();
        capability_keys.sort_unstable();
        assert_eq!(
            capability_keys,
            [
                "capabilityId",
                "connectionState",
                "grantState",
                "label",
                "managementUrl",
                "reasonCode",
                "requestedScopes",
                "required",
                "status",
            ]
        );
        assert_eq!(value["capabilities"][0]["requestedScopes"], json!([]));
    }

    #[test]
    fn management_url_uses_only_a_configured_https_origin() {
        assert_eq!(
            build_management_url("https://nyx.example/base?query=ignored#fragment"),
            Some("https://nyx.example/keys".to_string())
        );
        assert_eq!(build_management_url("http://nyx.example"), None);
        assert_eq!(build_management_url("https://user:pass@nyx.example"), None);
        assert_eq!(build_management_url("not a url"), None);
    }
}
