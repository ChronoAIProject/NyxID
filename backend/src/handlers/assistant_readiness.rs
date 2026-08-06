use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::AppState;
use crate::mw::auth::AuthUser;
use crate::services::assistant_readiness_service::{self, CapabilityReadiness, ReadinessSnapshot};

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantReadinessResponse {
    revision: &'static str,
    evaluated_at: DateTime<Utc>,
    capabilities: Vec<AssistantCapabilityResponse>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct AssistantCapabilityResponse {
    capability_id: &'static str,
    label: &'static str,
    required: bool,
    status: &'static str,
    connection_state: &'static str,
    grant_state: &'static str,
    requested_scopes: &'static [&'static str],
    management_url: Option<String>,
    reason_code: Option<&'static str>,
}

/// GET /api/v1/assistant/readiness
pub async fn get_readiness(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> Json<AssistantReadinessResponse> {
    let evaluated_at = Utc::now();
    let snapshot = assistant_readiness_service::evaluate_readiness(
        &state.db,
        state.node_ws_manager.as_ref(),
        &auth_user.user_id.to_string(),
        evaluated_at,
    )
    .await;
    Json(readiness_response(&state.config.frontend_url, snapshot))
}

fn readiness_response(
    frontend_url: &str,
    snapshot: ReadinessSnapshot,
) -> AssistantReadinessResponse {
    let capabilities = snapshot
        .capabilities
        .into_iter()
        .map(|capability| capability_response(frontend_url, capability))
        .collect();

    AssistantReadinessResponse {
        revision: snapshot.revision,
        evaluated_at: snapshot.evaluated_at,
        capabilities,
    }
}

fn capability_response(
    frontend_url: &str,
    capability: CapabilityReadiness,
) -> AssistantCapabilityResponse {
    AssistantCapabilityResponse {
        capability_id: capability.capability_id,
        label: capability.label,
        required: capability.required,
        status: capability.status.as_str(),
        connection_state: capability.connection_state.as_str(),
        grant_state: capability.grant_state.as_str(),
        requested_scopes: capability.requested_scopes,
        management_url: capability
            .management_path
            .and_then(|path| build_management_url(frontend_url, path)),
        reason_code: capability.reason_code.map(|reason| reason.as_str()),
    }
}

/// Construct a management URL only from the configured NyxID frontend origin.
/// Userinfo, query, fragment, non-HTTPS schemes, and hostless URLs fail closed.
fn build_management_url(frontend_url: &str, path: &str) -> Option<String> {
    let (raw_scheme, authority_and_path) = frontend_url.split_once("://")?;
    if !raw_scheme.eq_ignore_ascii_case("https")
        || authority_and_path.starts_with('/')
        || authority_and_path.contains('\\')
    {
        return None;
    }
    let mut url = url::Url::parse(frontend_url).ok()?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || !path.starts_with('/')
        || path.starts_with("//")
    {
        return None;
    }
    url.set_path(path);
    url.set_query(None);
    url.set_fragment(None);
    Some(url.into())
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeSet, HashSet};

    use axum::extract::State;
    use serde_json::Value;
    use uuid::Uuid;

    use super::*;
    use crate::models::downstream_service::{
        COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
    };
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::test_utils::{
        connect_test_database, test_app_config, test_app_state_with_config, test_auth_user,
        test_user_service,
    };

    use crate::services::assistant_readiness_service::{
        CapabilityStatus, ConnectionState, FixtureEvidence, GrantState,
    };

    const FIXTURE: &str = include_str!("../../../tests/fixtures/assistant/readiness-v2.json");
    const MATRIX_FIXTURE: &str =
        include_str!("../../../tests/fixtures/assistant/readiness-v2-matrix.json");
    const EVALUATED_AT: &str = "2026-08-01T00:00:00Z";

    fn fixture_evidence(
        connection_state: ConnectionState,
        grant_state: GrantState,
    ) -> FixtureEvidence {
        FixtureEvidence {
            catalog_available: true,
            access_allowed: true,
            connection_state,
            grant_state,
            executable: Some(connection_state != ConnectionState::NotConnected),
        }
    }

    fn unavailable_evidence() -> FixtureEvidence {
        FixtureEvidence {
            catalog_available: false,
            access_allowed: true,
            connection_state: ConnectionState::Unknown,
            grant_state: GrantState::Unknown,
            executable: None,
        }
    }

    fn platform_fixture_evidence(
        connection_state: ConnectionState,
        executable: bool,
    ) -> FixtureEvidence {
        FixtureEvidence {
            catalog_available: true,
            access_allowed: true,
            connection_state,
            grant_state: GrantState::NotRequired,
            executable: Some(executable),
        }
    }

    fn serialized_fixture_response(
        github: FixtureEvidence,
        model: FixtureEvidence,
        runtime: FixtureEvidence,
    ) -> Value {
        let evaluated_at = DateTime::parse_from_rfc3339(EVALUATED_AT)
            .expect("fixture timestamp")
            .with_timezone(&Utc);
        let snapshot = ReadinessSnapshot {
            revision: assistant_readiness_service::ASSISTANT_READINESS_REVISION,
            evaluated_at,
            capabilities: vec![
                assistant_readiness_service::evaluate_fixture_capability("api-github", github),
                assistant_readiness_service::evaluate_fixture_capability("model", model),
                assistant_readiness_service::evaluate_fixture_capability("runtime", runtime),
            ],
        };
        serde_json::to_value(readiness_response("https://id.nyx.example", snapshot))
            .expect("serialize fixture response through handler")
    }

    fn expected_scenario(name: &str) -> Value {
        let mut github = fixture_evidence(ConnectionState::Connected, GrantState::Granted);
        let mut model = platform_fixture_evidence(ConnectionState::Connected, true);
        let mut runtime = platform_fixture_evidence(ConnectionState::Connected, true);
        match name {
            "all_ready" | "model_backstop_available" => {}
            "not_connected" => {
                github = fixture_evidence(ConnectionState::NotConnected, GrantState::Missing)
            }
            "connecting" => {
                github = fixture_evidence(ConnectionState::Connecting, GrantState::Unknown)
            }
            "verifying" => {
                github = fixture_evidence(ConnectionState::Verifying, GrantState::Unknown)
            }
            "expired" => github = fixture_evidence(ConnectionState::Expired, GrantState::Expired),
            "revoked" => github = fixture_evidence(ConnectionState::Revoked, GrantState::Revoked),
            "unknown" => github = fixture_evidence(ConnectionState::Unknown, GrantState::Unknown),
            "partial_grant" => {
                github = fixture_evidence(ConnectionState::Connected, GrantState::Partial)
            }
            "missing_grant" => {
                github = fixture_evidence(ConnectionState::Connected, GrantState::Missing)
            }
            "expired_grant" => {
                github = fixture_evidence(ConnectionState::Connected, GrantState::Expired)
            }
            "revoked_grant" => {
                github = fixture_evidence(ConnectionState::Connected, GrantState::Revoked)
            }
            "unknown_grant" => {
                github = fixture_evidence(ConnectionState::Connected, GrantState::Unknown)
            }
            "model_disconnected" => {
                model = fixture_evidence(ConnectionState::NotConnected, GrantState::Missing)
            }
            "model_credential_unprovisioned" => {
                model = platform_fixture_evidence(ConnectionState::Verifying, false)
            }
            "model_byok_expired" => {
                model = platform_fixture_evidence(ConnectionState::Expired, true)
            }
            "model_org_presence_unverifiable" => model = unavailable_evidence(),
            "model_autoprovision_drift" => {
                model = platform_fixture_evidence(ConnectionState::Connected, false)
            }
            "runtime_unprovisioned" => runtime = unavailable_evidence(),
            "runtime_credential_unprovisioned" => {
                runtime = platform_fixture_evidence(ConnectionState::Verifying, false)
            }
            "runtime_misconfigured" | "runtime_auth_chain_unconfigured" => {
                runtime = platform_fixture_evidence(ConnectionState::Connected, false)
            }
            _ => panic!("unknown fixture scenario '{name}'"),
        }
        serialized_fixture_response(github, model, runtime)
    }

    #[test]
    fn management_url_is_https_configuration_owned_and_normalized() {
        assert_eq!(
            build_management_url("https://id.nyx.example/old?token=bad#fragment", "/keys")
                .as_deref(),
            Some("https://id.nyx.example/keys")
        );
        assert_eq!(
            build_management_url("https://id.nyx.example:8443", "/keys").as_deref(),
            Some("https://id.nyx.example:8443/keys")
        );
    }

    #[test]
    fn unsafe_management_urls_fail_closed() {
        for unsafe_url in [
            "http://id.nyx.example",
            "javascript:alert(1)",
            "https://user:password@id.nyx.example",
            "https:///missing-host",
            "not a url",
        ] {
            assert_eq!(build_management_url(unsafe_url, "/keys"), None);
        }
        assert_eq!(
            build_management_url("https://id.nyx.example", "//evil.example/keys"),
            None
        );
    }

    #[test]
    fn canonical_fixture_matches_actual_handler_serialization() {
        let fixture: Value = serde_json::from_str(FIXTURE).expect("fixture must be valid JSON");
        assert_eq!(fixture, expected_scenario("all_ready"));
        assert_eq!(
            fixture["revision"],
            assistant_readiness_service::ASSISTANT_READINESS_REVISION
        );
        let capabilities = fixture["capabilities"]
            .as_array()
            .expect("capabilities array");
        assert_eq!(capabilities.len(), 3);
        assert_eq!(capabilities[0]["capabilityId"], "api-github");
        assert_eq!(
            capabilities[0]["requestedScopes"],
            serde_json::json!(["repo"])
        );
        assert_eq!(capabilities[1]["capabilityId"], "model");
        assert_eq!(capabilities[1]["required"], true);
        assert_eq!(capabilities[1]["requestedScopes"], serde_json::json!([]));
        assert_eq!(capabilities[2]["capabilityId"], "runtime");
        assert_eq!(capabilities[2]["required"], true);
        assert!(capabilities[2]["managementUrl"].is_null());
    }

    #[test]
    fn matrix_rows_are_profile_valid_and_match_actual_handler_serialization() {
        let matrix: Value =
            serde_json::from_str(MATRIX_FIXTURE).expect("matrix fixture must be valid JSON");
        assert_eq!(
            matrix["revision"],
            assistant_readiness_service::ASSISTANT_READINESS_REVISION
        );
        let scenarios = matrix["scenarios"].as_array().expect("scenarios array");
        let mut names = HashSet::new();
        for scenario in scenarios {
            let name = scenario["name"].as_str().expect("scenario name");
            assert!(names.insert(name), "duplicate scenario '{name}'");
            assert_eq!(scenario["response"], expected_scenario(name), "{name}");
        }
        assert_eq!(names.len(), 22);
    }

    #[test]
    fn matrix_covers_every_closed_status_connection_and_grant_value() {
        let matrix: Value = serde_json::from_str(MATRIX_FIXTURE).expect("valid matrix fixture");
        let capabilities = matrix["scenarios"]
            .as_array()
            .expect("scenarios array")
            .iter()
            .flat_map(|scenario| {
                scenario["response"]["capabilities"]
                    .as_array()
                    .expect("scenario capabilities")
            });
        let mut statuses = BTreeSet::new();
        let mut connections = BTreeSet::new();
        let mut grants = BTreeSet::new();
        for capability in capabilities {
            statuses.insert(capability["status"].as_str().expect("status"));
            connections.insert(
                capability["connectionState"]
                    .as_str()
                    .expect("connectionState"),
            );
            grants.insert(capability["grantState"].as_str().expect("grantState"));
        }

        assert_eq!(
            statuses,
            CapabilityStatus::ALL
                .map(CapabilityStatus::as_str)
                .into_iter()
                .collect()
        );
        assert_eq!(
            connections,
            ConnectionState::ALL
                .map(ConnectionState::as_str)
                .into_iter()
                .collect()
        );
        assert_eq!(
            grants,
            GrantState::ALL
                .map(GrantState::as_str)
                .into_iter()
                .collect()
        );
    }

    #[test]
    fn fixtures_preserve_reason_and_safety_contracts() {
        let fixture: Value = serde_json::from_str(FIXTURE).expect("valid canonical fixture");
        let matrix: Value = serde_json::from_str(MATRIX_FIXTURE).expect("valid matrix fixture");

        assert_no_secret_shape(&fixture);
        assert_no_secret_shape(&matrix);
        assert_response_contract(&fixture);
        for scenario in matrix["scenarios"].as_array().expect("scenarios array") {
            assert_response_contract(&scenario["response"]);
        }
    }

    #[tokio::test]
    async fn handler_uses_verified_auth_user_and_does_not_cross_user_boundaries() {
        let Some(db) = connect_test_database("assistant_readiness_handler").await else {
            return;
        };
        let catalog_id = Uuid::new_v4().to_string();
        let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
        catalog.id = catalog_id.clone();
        catalog.slug = "api-github".to_string();
        catalog.name = "GitHub".to_string();
        catalog.service_type = "http".to_string();
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(catalog)
            .await
            .expect("insert catalog service");

        let actor_id = Uuid::new_v4().to_string();
        let other_user_id = Uuid::new_v4().to_string();
        let other_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &other_user_id,
            "api-github",
            &Uuid::new_v4().to_string(),
            Some(&catalog_id),
            None,
        );
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(other_service)
            .await
            .expect("insert another user's service");

        let mut config = test_app_config();
        config.frontend_url = "https://id.nyx.example".to_string();
        let state = test_app_state_with_config(db, config);
        let Json(response) = get_readiness(State(state), test_auth_user(&actor_id)).await;
        let json = serde_json::to_value(response).expect("serialize response");

        assert_eq!(json["revision"], "nyxid-assistant-readiness.v2");
        assert_eq!(json["capabilities"][0]["capabilityId"], "api-github");
        assert_eq!(json["capabilities"][0]["status"], "missing");
        assert_eq!(json["capabilities"][0]["connectionState"], "not_connected");
        assert_eq!(json["capabilities"][0]["grantState"], "missing");
        assert_eq!(
            json["capabilities"][0]["reasonCode"],
            "service_not_connected"
        );
        for capability_id in ["model", "runtime"] {
            let capability = json["capabilities"]
                .as_array()
                .expect("capabilities")
                .iter()
                .find(|capability| capability["capabilityId"] == capability_id)
                .expect("platform capability");
            assert_eq!(capability["status"], "cannot_check");
            assert_eq!(capability["connectionState"], "unknown");
            assert_eq!(capability["grantState"], "unknown");
        }
        assert!(json.to_string().find(&actor_id).is_none());
        assert!(json.to_string().find(&other_user_id).is_none());
    }

    fn assert_response_contract(response: &Value) {
        assert_eq!(
            response["revision"],
            assistant_readiness_service::ASSISTANT_READINESS_REVISION
        );
        assert!(response["evaluatedAt"].as_str().is_some());
        let capabilities = response["capabilities"]
            .as_array()
            .expect("capabilities array");
        assert_eq!(capabilities.len(), 3);
        for capability in capabilities {
            assert_eq!(
                capability["reasonCode"].is_null(),
                capability["status"] == "available"
            );
            if capability["capabilityId"] == "runtime" {
                assert!(capability["managementUrl"].is_null());
            } else {
                let management_url = capability["managementUrl"]
                    .as_str()
                    .and_then(|value| url::Url::parse(value).ok())
                    .expect("safe fixture management URL");
                assert_eq!(management_url.scheme(), "https");
                assert!(management_url.host_str().is_some());
                assert!(management_url.username().is_empty());
                assert!(management_url.password().is_none());
            }
        }
    }

    fn assert_no_secret_shape(value: &Value) {
        const FORBIDDEN_KEYS: &[&str] = &[
            "authorization",
            "cookie",
            "credential",
            "password",
            "secret",
            "token",
        ];
        match value {
            Value::Object(object) => {
                for (key, child) in object {
                    let normalized: String = key
                        .chars()
                        .filter(char::is_ascii_alphanumeric)
                        .map(|character| character.to_ascii_lowercase())
                        .collect();
                    assert!(
                        !FORBIDDEN_KEYS.contains(&normalized.as_str()),
                        "fixture contains forbidden key {key}"
                    );
                    assert_no_secret_shape(child);
                }
            }
            Value::Array(array) => {
                for child in array {
                    assert_no_secret_shape(child);
                }
            }
            Value::String(string) => {
                let lower = string.to_ascii_lowercase();
                assert!(!lower.contains("bearer "));
                assert!(!lower.contains("token="));
                assert!(!lower.contains("nyx_"));
                assert!(!lower.contains("sk-"));
            }
            _ => {}
        }
    }
}
