//! Fixed, caller-scoped chrono-probe transport. Monitoring state stays upstream.
use std::time::Duration;

use chrono::{DateTime, Utc};
use futures::StreamExt;
use mongodb::{Database, bson::doc};
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use utoipa::ToSchema;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    config::AppConfig,
    crypto::jwt::{self, JwtKeys, TokenRestrictionClaims},
    errors::{AppError, AppResult},
    models::user_service::{COLLECTION_NAME, UserService},
    mw::auth::{AuthMethod, AuthUser},
    services::{
        org_service::{self, OwnerAccess},
        user_service_service,
    },
};

const MAX_RESPONSE_BYTES: usize = 64 * 1024;
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct Check {
    pub path: String,
    #[serde(default = "default_statuses")]
    pub expect_status: Vec<u16>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub body_contains: String,
}

fn default_statuses() -> Vec<u16> {
    vec![200]
}

#[derive(Clone, Default, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct OwnerConfig {
    pub health: Option<Check>,
    pub readiness: Option<Check>,
}

#[derive(Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct WriteConfig {
    pub revision: i64,
    pub health: Option<Check>,
    pub readiness: Option<Check>,
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct Configuration {
    pub user_service_id: String,
    pub revision: i64,
    pub owner_config: OwnerConfig,
}

#[derive(Clone, Default, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckStatus {
    Healthy,
    Unhealthy,
    Ready,
    NotReady,
    NotConfigured,
    #[default]
    Unknown,
}

#[derive(Clone, Default, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Owner,
    Builtin,
    Discovered,
    #[default]
    None,
}

#[derive(Clone, Default, Deserialize, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Freshness {
    Fresh,
    Stale,
    #[default]
    Unknown,
}

#[derive(Clone, Default, Deserialize, Serialize, ToSchema)]
pub struct CheckResult {
    pub status: CheckStatus,
    pub configured: Option<bool>,
    #[serde(default)]
    pub source: Source,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub reason: String,
    pub checked_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<f64>,
    pub status_code: Option<u16>,
    #[serde(default)]
    pub freshness: Freshness,
    pub interval_ms: Option<f64>,
    pub stale_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct ServiceIdentity {
    pub id: String,
}

#[derive(Deserialize, Serialize, ToSchema)]
pub struct ProbeResult {
    pub service: ServiceIdentity,
    #[serde(default)]
    pub health: CheckResult,
    #[serde(default)]
    pub snapshot_present: bool,
    pub readiness: Option<CheckResult>,
}

/// Only server-authorized capabilities are returned to the UI.
pub struct Access {
    pub service_id: String,
    pub can_manage: bool,
    pub can_probe: bool,
}

pub async fn authorize(db: &Database, auth: &AuthUser, id: &str) -> AppResult<Access> {
    ensure_human(auth)?;
    let canonical = Uuid::parse_str(id)
        .map_err(|_| AppError::BadRequest("Monitoring requires an exact UserService UUID".into()))?
        .to_string();
    let service = crate::services::service_history::collection::<UserService>(db, COLLECTION_NAME)
        .find_one(doc! { "_id": &canonical })
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".into()))?;
    let access =
        org_service::resolve_owner_access(db, &auth.user_id.to_string(), &service.user_id).await?;
    authorize_access(auth, &service, &access)
}

fn authorize_access(
    auth: &AuthUser,
    service: &UserService,
    access: &OwnerAccess,
) -> AppResult<Access> {
    if !access.can_read()
        || !access.allows_resource(&service.id)
        || (!auth.allow_all_services && !auth.allowed_service_ids.contains(&service.id))
    {
        return Err(AppError::NotFound("Service not found".into()));
    }
    let usable = match access {
        OwnerAccess::Direct | OwnerAccess::AsOrgAdmin { .. } => true,
        OwnerAccess::AsOrgMember { role, .. } => {
            user_service_service::role_can_proxy_service(*role, service)
        }
        OwnerAccess::Forbidden => false,
    };
    if !usable {
        return Err(AppError::Forbidden(
            "Service use permission is required for monitoring".into(),
        ));
    }
    Ok(Access {
        service_id: service.id.clone(),
        can_manage: access.can_write(),
        can_probe: service.is_active && service.service_type == "http" && auth.can_use_rest_proxy(),
    })
}

fn ensure_human(auth: &AuthUser) -> AppResult<()> {
    // Agent API keys cannot be converted into a user token: doing so would lose
    // per-agent credential bindings, operation scopes and live-turn authority.
    if !matches!(
        auth.auth_method,
        AuthMethod::Session | AuthMethod::AccessToken
    ) || auth.oauth_client_id.is_some()
        || auth.api_key_id.is_some()
        || auth.acting_client_id.is_some()
    {
        return Err(AppError::Forbidden(
            "Monitoring requires a first-party human account session".into(),
        ));
    }
    Ok(())
}

/// This address is operator configuration, never request input. Redirects and
/// ambient HTTP proxies are disabled before any caller token is transmitted.
pub fn validate_base_url(value: &str) -> AppResult<url::Url> {
    let url = url::Url::parse(value).map_err(|_| AppError::MonitoringUnavailable)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || url.path() != "/"
    {
        return Err(AppError::MonitoringUnavailable);
    }
    Ok(url)
}

pub struct Client {
    base: url::Url,
    http: reqwest::Client,
    token: Zeroizing<String>,
    id: String,
}

impl Client {
    pub fn new(
        config: &AppConfig,
        keys: &JwtKeys,
        auth: &AuthUser,
        access: &Access,
        execute: bool,
    ) -> AppResult<Self> {
        ensure_human(auth)?;
        let base = validate_base_url(
            config
                .chrono_probe_url
                .as_deref()
                .ok_or(AppError::MonitoringUnavailable)?,
        )?;
        if execute {
            auth.ensure_rest_proxy_access()?;
        }
        let mut restrictions = TokenRestrictionClaims::from_auth_user(auth);
        if restrictions.oauth_lineage.is_none() {
            restrictions.oauth_lineage = auth.session_id.map(|session_id| {
                crate::models::oauth_delegation_authority::OauthAuthorityLineage {
                    source_session_id: Some(session_id.to_string()),
                    delegation_jti: None,
                    consents: Vec::new(),
                }
            });
        }
        restrictions.allowed_service_ids = Some(vec![access.service_id.clone()]);
        restrictions.allow_all_services = Some(false);
        let scope = if execute {
            "account:read proxy"
        } else {
            "account:read"
        };
        let token = jwt::generate_delegated_access_token(
            keys,
            config,
            &auth.user_id,
            scope,
            "chrono-probe",
            30,
            Some(&restrictions),
        )?;
        let http = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .connect_timeout(Duration::from_secs(3))
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| AppError::MonitoringUnavailable)?;
        Ok(Self {
            base,
            http,
            token: Zeroizing::new(token),
            id: access.service_id.clone(),
        })
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        suffix: &str,
        body: Option<&impl Serialize>,
        revision: Option<i64>,
    ) -> AppResult<T> {
        let url = self
            .base
            .join(&format!(
                "api/v1/monitoring/services/{}/{}",
                self.id, suffix
            ))
            .map_err(|_| AppError::MonitoringUnavailable)?;
        let mut request = self
            .http
            .request(method, url)
            .bearer_auth(self.token.as_str());
        if let Some(revision) = revision {
            request = request.query(&[("revision", revision)]);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = request
            .send()
            .await
            .map_err(|_| AppError::MonitoringUnavailable)?;
        if !response.status().is_success() {
            return Err(upstream_error(response.status()));
        }
        let mut bytes = Vec::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| AppError::MonitoringUnavailable)?;
            if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(AppError::MonitoringUnavailable);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| AppError::MonitoringUnavailable)
    }

    pub async fn status(&self) -> AppResult<ProbeResult> {
        let mut result: ProbeResult = self
            .request(Method::GET, "status", None::<&()>, None)
            .await?;
        if result.snapshot_present {
            if let Some(readiness) = result
                .readiness
                .as_mut()
                .filter(|check| check.configured == Some(false))
            {
                readiness.status = CheckStatus::NotConfigured;
            }
        }
        self.checked_result(result)
    }

    pub async fn configuration(&self) -> AppResult<Configuration> {
        let result = self
            .request(Method::GET, "config", None::<&()>, None)
            .await?;
        self.checked_config(result)
    }

    pub async fn save(&self, body: &WriteConfig) -> AppResult<Configuration> {
        if body.revision < 0 {
            return Err(AppError::BadRequest("Revision must not be negative".into()));
        }
        let result = self
            .request(Method::PUT, "config", Some(body), None)
            .await?;
        self.checked_config(result)
    }

    pub async fn clear(&self, revision: i64) -> AppResult<Configuration> {
        if revision < 0 {
            return Err(AppError::BadRequest("Revision must not be negative".into()));
        }
        let result = self
            .request(Method::DELETE, "config", None::<&()>, Some(revision))
            .await?;
        self.checked_config(result)
    }

    pub async fn probe(&self, draft: Option<&OwnerConfig>) -> AppResult<ProbeResult> {
        let suffix = if draft.is_some() {
            "config/validate"
        } else {
            "probes"
        };
        let mut result: ProbeResult = self.request(Method::POST, suffix, draft, None).await?;
        for check in std::iter::once(&mut result.health).chain(result.readiness.iter_mut()) {
            // Manual responses have no scheduler interval; they are an observation
            // from this explicit operation, not a new scheduler snapshot.
            let now = Utc::now();
            if let Some(at) = check.checked_at.filter(|at| {
                *at <= now + chrono::Duration::seconds(5)
                    && *at >= now - chrono::Duration::seconds(60)
            }) {
                check.freshness = Freshness::Fresh;
                check.stale_at = Some(at + chrono::Duration::seconds(60));
            } else {
                check.freshness = Freshness::Unknown;
                check.stale_at = None;
            }
        }
        self.checked_result(result)
    }

    fn checked_result(&self, result: ProbeResult) -> AppResult<ProbeResult> {
        if result.service.id != self.id {
            return Err(AppError::MonitoringUnavailable);
        }
        for check in std::iter::once(&result.health).chain(result.readiness.iter()) {
            if check.path.len() > 512
                || check.reason.len() > 64
                || check
                    .reason
                    .chars()
                    .any(|c| !(c.is_ascii_lowercase() || c == '_'))
            {
                return Err(AppError::MonitoringUnavailable);
            }
        }
        Ok(result)
    }

    fn checked_config(&self, result: Configuration) -> AppResult<Configuration> {
        if result.user_service_id != self.id || result.revision < 0 {
            return Err(AppError::MonitoringUnavailable);
        }
        Ok(result)
    }
}

fn upstream_error(status: StatusCode) -> AppError {
    match status {
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => AppError::BadRequest("Invalid monitoring configuration; use a safe relative GET path and bounded expectations".into()),
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => AppError::Forbidden("Monitoring authorization was denied".into()),
        StatusCode::CONFLICT => AppError::Conflict("Monitoring configuration changed or checking is unavailable; reload before retrying".into()),
        StatusCode::TOO_MANY_REQUESTS => AppError::RateLimitedWithRetry { retry_after: 10 },
        _ => AppError::MonitoringUnavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{
        cached_test_jwt_keys, test_app_config, test_auth_user, test_user_service,
    };
    use serde_json::{Value, json};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path, query_param},
    };

    pub(super) fn result(id: &str) -> Value {
        json!({"service":{"id":id,"secret":"never forward"},"snapshot_present":true,
            "health":{"status":"healthy","source":"owner","path":"healthz","reason":"ok",
            "checked_at":Utc::now(),"freshness":"fresh","interval_ms":60000,
            "stale_at":Utc::now()+chrono::Duration::seconds(60),"duration_ms":3.5,"status_code":200},
            "readiness":{"status":"unknown","configured":false,"source":"none","freshness":"unknown"}})
    }

    fn client(server: &MockServer, id: &str, execute: bool) -> Client {
        let mut config = test_app_config();
        config.chrono_probe_url = Some(server.uri());
        Client::new(
            &config,
            &cached_test_jwt_keys(),
            &test_auth_user(&Uuid::new_v4().to_string()),
            &Access {
                service_id: id.into(),
                can_manage: true,
                can_probe: true,
            },
            execute,
        )
        .unwrap()
    }

    #[test]
    fn operator_origin_is_not_an_open_proxy() {
        for origin in ["https://probe.internal", "http://127.0.0.1:8080/"] {
            assert!(validate_base_url(origin).is_ok());
        }
        for origin in [
            "file:///tmp/status",
            "https://user:pass@probe/",
            "https://probe/api",
            "https://probe/?token=x",
            "https://probe/#x",
        ] {
            assert!(validate_base_url(origin).is_err());
        }
    }

    #[test]
    fn owner_member_viewer_and_service_restrictions_are_distinct() {
        use crate::models::org_membership::OrgRole;
        let mut auth = test_auth_user(&Uuid::new_v4().to_string());
        let mut service =
            test_user_service("exact-id", "owner", "same-slug", "endpoint", None, None);
        let member = |role, ids| OwnerAccess::AsOrgMember {
            org_user_id: "org".into(),
            membership_id: "membership".into(),
            role,
            allowed_service_ids: ids,
        };
        assert!(
            authorize_access(&auth, &service, &OwnerAccess::Direct)
                .unwrap()
                .can_manage
        );
        let access = authorize_access(&auth, &service, &member(OrgRole::Member, None)).unwrap();
        assert!(!access.can_manage && access.can_probe);
        assert!(authorize_access(&auth, &service, &member(OrgRole::Viewer, None)).is_err());
        assert!(
            authorize_access(
                &auth,
                &service,
                &member(OrgRole::Member, Some(vec!["sibling".into()]))
            )
            .is_err()
        );
        assert!(authorize_access(&auth, &service, &OwnerAccess::Forbidden).is_err());
        service.admin_only = true;
        assert!(authorize_access(&auth, &service, &member(OrgRole::Member, None)).is_err());
        service.admin_only = false;
        auth.allow_all_services = false;
        auth.allowed_service_ids = vec!["sibling".into()];
        assert!(authorize_access(&auth, &service, &OwnerAccess::Direct).is_err());
        auth.allowed_service_ids = vec![service.id.clone()];
        auth.auth_method = AuthMethod::AccessToken;
        auth.scope = "account:read".into();
        assert!(
            !authorize_access(&auth, &service, &OwnerAccess::Direct)
                .unwrap()
                .can_probe
        );
    }

    #[test]
    fn callback_derivation_rejects_agents_delegates_and_oauth_clients() {
        for method in [
            AuthMethod::ApiKey,
            AuthMethod::Delegated,
            AuthMethod::Relay,
            AuthMethod::ServiceAccount,
        ] {
            let mut auth = test_auth_user(&Uuid::new_v4().to_string());
            auth.auth_method = method;
            assert!(ensure_human(&auth).is_err());
        }
        let mut auth = test_auth_user(&Uuid::new_v4().to_string());
        auth.auth_method = AuthMethod::AccessToken;
        auth.scope = "openid".into();
        auth.oauth_client_id = Some("third-party-client".into());
        assert!(ensure_human(&auth).is_err());
    }

    #[tokio::test]
    async fn status_is_one_exact_read_with_narrow_caller_token_and_projection() {
        let server = MockServer::start().await;
        let id = Uuid::new_v4().to_string();
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/monitoring/services/{id}/status")))
            .respond_with(ResponseTemplate::new(200).set_body_json(result(&id)))
            .expect(1)
            .mount(&server)
            .await;
        let mut config = test_app_config();
        config.chrono_probe_url = Some(server.uri());
        let keys = cached_test_jwt_keys();
        let mut auth = test_auth_user(&Uuid::new_v4().to_string());
        auth.allow_all_nodes = false;
        auth.allowed_node_ids = vec!["allowed-node".into()];
        auth.session_id = Some(Uuid::new_v4());
        auth.resource_uris = Some(vec![format!("{}/proxy/same-slug", config.base_url)]);
        let client = Client::new(
            &config,
            &keys,
            &auth,
            &Access {
                service_id: id.clone(),
                can_manage: false,
                can_probe: true,
            },
            false,
        )
        .unwrap();
        let output = serde_json::to_value(client.status().await.unwrap()).unwrap();
        assert_eq!(output["readiness"]["status"], "not_configured");
        assert!(output["service"].get("secret").is_none());
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        let token = requests[0].headers["authorization"]
            .to_str()
            .unwrap()
            .strip_prefix("Bearer ")
            .unwrap();
        let claims = jwt::verify_token(&keys, &config, token).unwrap();
        assert_eq!(claims.sub, auth.user_id.to_string());
        assert_eq!(claims.scope, "account:read");
        assert_eq!(claims.allowed_service_ids, Some(vec![id]));
        assert_eq!(claims.allow_all_services, Some(false));
        assert_eq!(claims.allowed_node_ids, Some(vec!["allowed-node".into()]));
        assert_eq!(claims.allow_all_nodes, Some(false));
        assert_eq!(claims.resources, auth.resource_uris);
        assert_eq!(
            claims.oauth_lineage.unwrap().source_session_id,
            auth.session_id.map(|id| id.to_string())
        );
        assert!(claims.exp - claims.iat <= 30);
    }

    #[tokio::test]
    async fn revisions_and_draft_methods_are_forwarded_without_writes() {
        let server = MockServer::start().await;
        let id = Uuid::new_v4().to_string();
        let client = client(&server, &id, true);
        let root = format!("/api/v1/monitoring/services/{id}");
        let config = json!({"user_service_id":id,"revision":8,"owner_config":{}});
        let body = json!({"revision":7,"health":{"path":"healthz","expect_status":[200]},"readiness":null});
        Mock::given(method("PUT"))
            .and(path(format!("{root}/config")))
            .and(body_json(&body))
            .respond_with(ResponseTemplate::new(200).set_body_json(&config))
            .expect(1)
            .mount(&server)
            .await;
        assert_eq!(
            client
                .save(&serde_json::from_value(body).unwrap())
                .await
                .unwrap()
                .revision,
            8
        );
        Mock::given(method("DELETE"))
            .and(path(format!("{root}/config")))
            .and(query_param("revision", "8"))
            .respond_with(ResponseTemplate::new(409).set_body_string("secret downstream response"))
            .expect(1)
            .mount(&server)
            .await;
        assert!(matches!(client.clear(8).await, Err(AppError::Conflict(_))));
        let draft = json!({"health":null,"readiness":{"path":"readyz","expect_status":[200]}});
        let mut output = result(&id);
        output.as_object_mut().unwrap().remove("health");
        output["readiness"] = json!({"status":"ready","source":"owner","path":"readyz","reason":"ok","checked_at":Utc::now()});
        Mock::given(method("POST"))
            .and(path(format!("{root}/config/validate")))
            .and(body_json(&draft))
            .respond_with(ResponseTemplate::new(200).set_body_json(output))
            .expect(1)
            .mount(&server)
            .await;
        let output = client
            .probe(Some(&serde_json::from_value(draft).unwrap()))
            .await
            .unwrap();
        assert!(matches!(output.health.status, CheckStatus::Unknown));
        assert!(matches!(
            output.readiness.unwrap().freshness,
            Freshness::Fresh
        ));
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
    }

    #[tokio::test]
    async fn missing_stale_mismatched_and_unsafe_upstreams_fail_closed() {
        let server = MockServer::start().await;
        let id = Uuid::new_v4().to_string();
        let client = client(&server, &id, false);
        let endpoint = format!("/api/v1/monitoring/services/{id}/status");
        for status in [301, 401, 403, 409, 429, 500, 503] {
            Mock::given(path(&endpoint))
                .respond_with(
                    ResponseTemplate::new(status)
                        .insert_header("Location", "http://127.0.0.1:1/secret")
                        .set_body_string("never expose this"),
                )
                .mount(&server)
                .await;
            let error = client.status().await.err().unwrap();
            assert!(!error.to_string().contains("never expose"));
            assert!(!matches!(error, AppError::Unauthorized(_)));
            server.reset().await;
        }
        for body in [
            "invalid json".to_string(),
            "x".repeat(MAX_RESPONSE_BYTES + 1),
            result("another-instance").to_string(),
        ] {
            Mock::given(path(&endpoint))
                .respond_with(ResponseTemplate::new(200).set_body_string(body))
                .mount(&server)
                .await;
            assert!(matches!(
                client.status().await,
                Err(AppError::MonitoringUnavailable)
            ));
            server.reset().await;
        }
        let mut value = result(&id);
        value["health"]["freshness"] = json!("stale");
        Mock::given(path(&endpoint))
            .respond_with(ResponseTemplate::new(200).set_body_json(value))
            .mount(&server)
            .await;
        assert!(matches!(
            client.status().await.unwrap().health.freshness,
            Freshness::Stale
        ));
        server.reset().await;
        Mock::given(path(&endpoint)).respond_with(ResponseTemplate::new(200).set_body_json(json!({"service":{"id":id},"health":{"status":"unknown","freshness":"unknown"},"readiness":{"status":"unknown","freshness":"unknown"}}))).mount(&server).await;
        let value = client.status().await.unwrap();
        assert!(matches!(value.health.status, CheckStatus::Unknown));
        assert!(matches!(
            value.readiness.unwrap().status,
            CheckStatus::Unknown
        ));
    }
}
