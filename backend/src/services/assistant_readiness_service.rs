use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::services::approval_service::ApprovalReadiness;
use crate::services::user_service_service::CredentialSource;

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
    if conflicting_evidence {
        existing.status = ReadinessStatus::CannotCheck;
        existing.reason_code = Some("conflicting_evidence".to_string());
    } else {
        existing.status =
            derive_status(existing.connection_state, existing.grant_state, cannot_use);
        if incoming.reason_code < existing.reason_code {
            existing.reason_code = incoming.reason_code;
        }
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

fn core_capability(
    capability_id: &str,
    label: &str,
    service: Option<&DownstreamService>,
    usable: bool,
    management_url: &Option<String>,
) -> ReadinessCapability {
    let (connection_state, status, reason_code) = match service {
        None
        | Some(DownstreamService {
            is_active: false, ..
        }) => (
            ConnectionState::NotConnected,
            ReadinessStatus::Missing,
            Some("service_missing".to_string()),
        ),
        Some(_) if !usable => (
            ConnectionState::Connected,
            ReadinessStatus::CannotUse,
            Some("service_misconfigured".to_string()),
        ),
        Some(_) => (ConnectionState::Connected, ReadinessStatus::Available, None),
    };
    ReadinessCapability {
        capability_id: capability_id.to_string(),
        label: label.to_string(),
        required: true,
        status,
        connection_state,
        grant_state: GrantState::NotRequired,
        requested_scopes: Vec::new(),
        management_url: management_url.clone(),
        reason_code,
    }
}

fn runtime_is_usable(service: &DownstreamService) -> bool {
    service.is_active
        && service.service_type == "http"
        && service.service_category != "provider"
        && !service.requires_user_credential
        && (service.auth_method == "none" || !service.credential_encrypted.is_empty())
}

fn model_is_usable(service: &DownstreamService, has_provider_requirement: bool) -> bool {
    let no_auth = service.auth_method == "none"
        && matches!(service.service_category.as_str(), "connection" | "internal")
        && !has_provider_requirement;
    let master_credential = service.visibility == "public"
        && service.service_category == "internal"
        && !matches!(service.auth_method.as_str(), "none" | "token_exchange")
        && !service.credential_encrypted.is_empty()
        && service.provider_config_id.is_none()
        && !has_provider_requirement;
    service.is_active
        && service.service_type == "http"
        && !service.requires_user_credential
        && (no_auth || master_credential)
}

fn connection_state(key: &crate::services::unified_key_service::KeyView) -> ConnectionState {
    if !key.is_active {
        return ConnectionState::NotConnected;
    }
    match key.status.as_str() {
        "active" => ConnectionState::Connected,
        "pending_auth" => ConnectionState::Connecting,
        "expired" => ConnectionState::Expired,
        "revoked" => ConnectionState::Revoked,
        _ => ConnectionState::Unknown,
    }
}

fn grant_state(readiness: ApprovalReadiness) -> GrantState {
    match readiness {
        ApprovalReadiness::NotRequired => GrantState::NotRequired,
        ApprovalReadiness::Granted => GrantState::Granted,
        ApprovalReadiness::Partial => GrantState::Partial,
        ApprovalReadiness::Missing | ApprovalReadiness::Denied => GrantState::Missing,
        ApprovalReadiness::Expired => GrantState::Expired,
        ApprovalReadiness::Revoked => GrantState::Revoked,
        ApprovalReadiness::Unknown => GrantState::Unknown,
    }
}

fn connector_reason(
    connection_state: ConnectionState,
    approval_readiness: ApprovalReadiness,
    cannot_use: bool,
) -> Option<String> {
    let code = if cannot_use {
        "access_denied"
    } else {
        match connection_state {
            ConnectionState::Connecting | ConnectionState::Verifying => "connection_pending",
            ConnectionState::Expired => "connection_expired",
            ConnectionState::Revoked => "connection_revoked",
            ConnectionState::Unknown => "connection_unknown",
            ConnectionState::NotConnected => "connection_missing",
            ConnectionState::Connected => match approval_readiness {
                ApprovalReadiness::Partial => "approval_partial",
                ApprovalReadiness::Missing => "approval_missing",
                ApprovalReadiness::Expired => "approval_expired",
                ApprovalReadiness::Revoked => "approval_revoked",
                ApprovalReadiness::Unknown => "approval_unknown",
                ApprovalReadiness::Denied => "approval_denied",
                ApprovalReadiness::NotRequired | ApprovalReadiness::Granted => return None,
            },
        }
    };
    Some(code.to_string())
}

pub async fn evaluate_readiness(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    frontend_url: &str,
    evaluated_at: DateTime<Utc>,
) -> AppResult<ReadinessSnapshot> {
    use crate::models::service_provider_requirement::{
        COLLECTION_NAME as SERVICE_PROVIDER_REQUIREMENTS, ServiceProviderRequirement,
    };

    let management_url = build_management_url(frontend_url);
    let runtime = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": crate::services::assistant_service::AEVATAR_SLUG })
        .await?;
    let model = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": "chrono-llm-public" })
        .await?;
    let model_has_provider_requirement = if let Some(model) = model.as_ref() {
        db.collection::<ServiceProviderRequirement>(SERVICE_PROVIDER_REQUIREMENTS)
            .count_documents(doc! { "service_id": &model.id })
            .await?
            > 0
    } else {
        false
    };
    let mut capabilities = vec![
        core_capability(
            "model",
            "Model",
            model.as_ref(),
            model
                .as_ref()
                .is_some_and(|service| model_is_usable(service, model_has_provider_requirement)),
            &management_url,
        ),
        core_capability(
            "runtime",
            "Chat runtime",
            runtime.as_ref(),
            runtime.as_ref().is_some_and(runtime_is_usable),
            &management_url,
        ),
    ];

    // ponytail: one approval lookup per visible connector; batch when measured latency warrants it.
    for key in crate::services::unified_key_service::list_keys(db, encryption_keys, user_id).await?
    {
        let capability_id = key
            .catalog_service_slug
            .clone()
            .unwrap_or_else(|| key.slug.clone());
        if matches!(capability_id.as_str(), "aevatar" | "chrono-llm-public") {
            continue;
        }
        let (service_owner_user_id, role_denied) = match &key.credential_source {
            CredentialSource::Personal => (user_id, false),
            CredentialSource::Org {
                org_user_id,
                allowed,
                ..
            } => (org_user_id.as_str(), !allowed),
        };
        let service_id = key.catalog_service_id.as_deref().unwrap_or(&key.id);
        let approval_readiness = crate::services::approval_service::summarize_approval_readiness(
            db,
            user_id,
            service_owner_user_id,
            service_id,
            "delegated",
            "aevatar",
            evaluated_at,
        )
        .await?;
        let connection_state = connection_state(&key);
        let grant_state = grant_state(approval_readiness);
        let policy_denied = approval_readiness == ApprovalReadiness::Denied;
        let cannot_use = role_denied || policy_denied;
        capabilities.push(ReadinessCapability {
            capability_id,
            label: key.catalog_service_name.unwrap_or(key.label),
            required: false,
            status: derive_status(connection_state, grant_state, cannot_use),
            connection_state,
            grant_state,
            requested_scopes: Vec::new(),
            management_url: management_url.clone(),
            reason_code: connector_reason(connection_state, approval_readiness, cannot_use),
        });
    }

    build_snapshot(capabilities, evaluated_at)
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
    fn duplicate_conflicts_are_unknown_even_when_one_entry_is_denied() {
        let evaluated_at = Utc.with_ymd_and_hms(2026, 8, 1, 1, 2, 3).unwrap();
        let available = capability(
            "api-github",
            false,
            ConnectionState::Connected,
            GrantState::Granted,
        );
        let mut denied = capability(
            "api-github",
            false,
            ConnectionState::Connected,
            GrantState::Missing,
        );
        denied.status = ReadinessStatus::CannotUse;

        let snapshot = build_snapshot(vec![available, denied], evaluated_at).unwrap();
        let github = &snapshot.capabilities[0];

        assert_eq!(github.connection_state, ConnectionState::Connected);
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

    #[test]
    fn versioned_fixture_is_parseable_complete_and_revision_consistent() {
        use std::collections::BTreeSet;

        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("fixtures/assistant-readiness/v1.json");
        let body = std::fs::read_to_string(&path)
            .unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
        let snapshot: ReadinessSnapshot = serde_json::from_str(&body).unwrap();
        let rebuilt = build_snapshot(snapshot.capabilities.clone(), snapshot.evaluated_at).unwrap();
        assert_eq!(snapshot.revision, rebuilt.revision);
        assert_eq!(snapshot.capabilities, rebuilt.capabilities);

        let value = serde_json::to_value(&snapshot).unwrap();
        let statuses = value["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|capability| capability["status"].as_str())
            .collect::<BTreeSet<_>>();
        let connections = value["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|capability| capability["connectionState"].as_str())
            .collect::<BTreeSet<_>>();
        let grants = value["capabilities"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|capability| capability["grantState"].as_str())
            .collect::<BTreeSet<_>>();
        assert_eq!(
            statuses,
            BTreeSet::from(["available", "cannot_check", "cannot_use", "missing"])
        );
        assert_eq!(
            connections,
            BTreeSet::from([
                "connected",
                "connecting",
                "expired",
                "not_connected",
                "revoked",
                "unknown",
                "verifying",
            ])
        );
        assert_eq!(
            grants,
            BTreeSet::from([
                "expired",
                "granted",
                "missing",
                "not_required",
                "partial",
                "revoked",
                "unknown",
            ])
        );

        let lower = body.to_ascii_lowercase();
        for forbidden in [
            "credential",
            "authorization",
            "cookie",
            "access_token",
            "refresh_token",
            "client_secret",
            "private_key",
        ] {
            assert!(!lower.contains(forbidden), "fixture contains {forbidden}");
        }
    }

    #[tokio::test]
    async fn evaluate_readiness_is_user_scoped_and_preserves_connector_grant_evidence() {
        use crate::models::approval_grant::{ApprovalGrant, COLLECTION_NAME as GRANTS};
        use crate::models::downstream_service::{
            COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
        };
        use crate::models::service_approval_config::{
            ApprovalMode, COLLECTION_NAME as APPROVAL_CONFIGS, ServiceApprovalConfig,
        };
        use crate::models::ssh_auth_mode::SshAuthMode;
        use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
        use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
        use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
        use crate::test_utils::{connect_test_database, test_encryption_keys};

        let Some(db) = connect_test_database("assistant_readiness_user_scope").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let other_user_id = uuid::Uuid::new_v4().to_string();
        let now = Utc.with_ymd_and_hms(2026, 8, 1, 1, 2, 3).unwrap();

        let mut runtime = crate::models::downstream_service::test_helpers::dummy_service();
        runtime.id = "core-runtime".to_string();
        runtime.slug = "aevatar".to_string();
        runtime.name = "Aevatar Runtime".to_string();
        runtime.service_category = "internal".to_string();
        let mut model = crate::models::downstream_service::test_helpers::dummy_service();
        model.id = "core-model".to_string();
        model.slug = "chrono-llm-public".to_string();
        model.name = "Chrono Model".to_string();
        model.service_category = "internal".to_string();
        let mut github = crate::models::downstream_service::test_helpers::dummy_service();
        github.id = "catalog-github".to_string();
        github.slug = "api-github".to_string();
        github.name = "GitHub".to_string();
        github.requires_user_credential = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_many([runtime, model, github])
            .await
            .unwrap();

        let make_connector = |owner: &str, suffix: &str| {
            let endpoint_id = format!("endpoint-{suffix}");
            let key_id = format!("key-{suffix}");
            let service_id = format!("service-{suffix}");
            (
                UserEndpoint {
                    id: endpoint_id.clone(),
                    user_id: owner.to_string(),
                    label: "GitHub".to_string(),
                    url: "https://api.github.example".to_string(),
                    catalog_service_id: Some("catalog-github".to_string()),
                    openapi_spec_url: None,
                    created_at: now,
                    updated_at: now,
                },
                UserApiKey {
                    id: key_id.clone(),
                    user_id: owner.to_string(),
                    label: "GitHub".to_string(),
                    credential_type: "oauth2".to_string(),
                    credential_encrypted: None,
                    access_token_encrypted: Some(vec![1]),
                    refresh_token_encrypted: None,
                    token_scopes: Some("repo".to_string()),
                    expires_at: None,
                    provider_config_id: None,
                    connection_id: None,
                    user_oauth_client_id_encrypted: None,
                    user_oauth_client_secret_encrypted: None,
                    credential_source: None,
                    status: "active".to_string(),
                    last_used_at: None,
                    last_authorized_at: None,
                    error_message: None,
                    source: None,
                    source_id: None,
                    created_at: now,
                    updated_at: now,
                },
                UserService {
                    id: service_id,
                    user_id: owner.to_string(),
                    slug: format!("github-{suffix}"),
                    endpoint_id,
                    api_key_id: Some(key_id),
                    auth_method: "bearer".to_string(),
                    auth_key_name: "Authorization".to_string(),
                    catalog_service_id: Some("catalog-github".to_string()),
                    node_id: None,
                    node_priority: 0,
                    service_type: "http".to_string(),
                    ssh_auth_mode: SshAuthMode::ProxyOnly,
                    admin_only: false,
                    ssh_node_keys_stale: false,
                    identity_propagation_mode: "none".to_string(),
                    identity_include_user_id: false,
                    identity_include_email: false,
                    identity_include_name: false,
                    identity_jwt_audience: None,
                    forward_access_token: false,
                    inject_delegation_token: false,
                    delegation_token_scope: "proxy".to_string(),
                    custom_user_agent: None,
                    default_request_headers: None,
                    ws_frame_injections: Vec::new(),
                    is_active: true,
                    source: None,
                    source_id: None,
                    source_app_id: None,
                    created_at: now,
                    updated_at: now,
                },
            )
        };
        let (endpoint, key, service) = make_connector(&user_id, "mine");
        let (other_endpoint, other_key, other_service) = make_connector(&other_user_id, "other");
        db.collection::<UserEndpoint>(USER_ENDPOINTS)
            .insert_many([endpoint, other_endpoint])
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_many([key, other_key])
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([service, other_service])
            .await
            .unwrap();

        db.collection::<ServiceApprovalConfig>(APPROVAL_CONFIGS)
            .insert_one(ServiceApprovalConfig {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: user_id.clone(),
                service_id: "catalog-github".to_string(),
                service_name: "GitHub".to_string(),
                approval_required: true,
                approval_mode: ApprovalMode::Grant,
                rules: Vec::new(),
                default_effect: None,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        db.collection::<ApprovalGrant>(GRANTS)
            .insert_one(ApprovalGrant {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: user_id.clone(),
                service_id: "catalog-github".to_string(),
                service_name: "GitHub".to_string(),
                requester_type: "delegated".to_string(),
                requester_id: "aevatar".to_string(),
                requester_label: None,
                approval_request_id: uuid::Uuid::new_v4().to_string(),
                scope: None,
                granted_at: now,
                expires_at: now + chrono::Duration::days(30),
                revoked: false,
                org_scoped: false,
            })
            .await
            .unwrap();

        let snapshot = evaluate_readiness(
            &db,
            &test_encryption_keys(),
            &user_id,
            "https://nyx.example",
            now,
        )
        .await
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
        assert_eq!(github.connection_state, ConnectionState::Connected);
        assert_eq!(github.grant_state, GrantState::Granted);
        assert_eq!(github.status, ReadinessStatus::Available);
        assert!(
            snapshot
                .capabilities
                .iter()
                .all(|item| item.requested_scopes.is_empty())
        );
    }

    #[tokio::test]
    async fn evaluate_readiness_reports_missing_and_misconfigured_core_truthfully() {
        use crate::models::downstream_service::{
            COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
        };
        use crate::test_utils::{connect_test_database, test_encryption_keys};

        let Some(db) = connect_test_database("assistant_readiness_core_state").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let now = Utc.with_ymd_and_hms(2026, 8, 1, 1, 2, 3).unwrap();
        let mut runtime = crate::models::downstream_service::test_helpers::dummy_service();
        runtime.id = "core-runtime".to_string();
        runtime.slug = "aevatar".to_string();
        runtime.name = "Aevatar Runtime".to_string();
        runtime.requires_user_credential = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(runtime)
            .await
            .unwrap();

        let snapshot = evaluate_readiness(
            &db,
            &test_encryption_keys(),
            &uuid::Uuid::new_v4().to_string(),
            "http://unsafe.example",
            now,
        )
        .await
        .unwrap();
        let model = &snapshot.capabilities[0];
        let runtime = &snapshot.capabilities[1];
        assert_eq!(model.capability_id, "model");
        assert_eq!(model.status, ReadinessStatus::Missing);
        assert_eq!(model.reason_code.as_deref(), Some("service_missing"));
        assert_eq!(runtime.capability_id, "runtime");
        assert_eq!(runtime.status, ReadinessStatus::CannotUse);
        assert_eq!(
            runtime.reason_code.as_deref(),
            Some("service_misconfigured")
        );
        assert!(
            snapshot
                .capabilities
                .iter()
                .all(|item| item.management_url.is_none())
        );
    }
}
