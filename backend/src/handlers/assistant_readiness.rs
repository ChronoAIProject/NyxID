use axum::{Json, extract::State};
use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::AppState;
use crate::mw::auth::AuthUser;
use crate::services::assistant_readiness_service::{self, CapabilityReadiness};

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
    let capabilities = snapshot
        .capabilities
        .into_iter()
        .map(|capability| capability_response(&state.config.frontend_url, capability))
        .collect();

    Json(AssistantReadinessResponse {
        revision: snapshot.revision,
        evaluated_at: snapshot.evaluated_at,
        capabilities,
    })
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
        management_url: build_management_url(frontend_url, capability.management_path),
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

    const FIXTURE: &str = include_str!("../../../tests/fixtures/assistant/readiness-v1.json");

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
    fn versioned_fixture_matches_the_published_contract_and_contains_no_secret_shape() {
        let fixture: Value = serde_json::from_str(FIXTURE).expect("fixture must be valid JSON");
        assert_eq!(
            fixture["revision"],
            assistant_readiness_service::ASSISTANT_READINESS_REVISION
        );
        assert!(fixture["evaluatedAt"].as_str().is_some());
        let capabilities = fixture["capabilities"]
            .as_array()
            .expect("capabilities array");
        assert_eq!(capabilities.len(), 1);
        assert_eq!(capabilities[0]["capabilityId"], "api-github");
        assert_eq!(
            capabilities[0]["requestedScopes"],
            serde_json::json!(["repo"])
        );
        assert_eq!(
            capabilities[0]["managementUrl"],
            "https://id.nyx.example/keys"
        );

        assert_no_secret_shape(&fixture);
        let management_url = capabilities[0]["managementUrl"]
            .as_str()
            .and_then(|value| url::Url::parse(value).ok())
            .expect("safe fixture management URL");
        assert_eq!(management_url.scheme(), "https");
        assert!(management_url.username().is_empty());
        assert!(management_url.password().is_none());
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

        assert_eq!(json["revision"], "nyxid-assistant-readiness.v1");
        assert_eq!(json["capabilities"][0]["status"], "missing");
        assert_eq!(json["capabilities"][0]["connectionState"], "not_connected");
        assert_eq!(json["capabilities"][0]["grantState"], "missing");
        assert_eq!(
            json["capabilities"][0]["reasonCode"],
            "service_not_connected"
        );
        assert!(json.to_string().find(&actor_id).is_none());
        assert!(json.to_string().find(&other_user_id).is_none());
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
