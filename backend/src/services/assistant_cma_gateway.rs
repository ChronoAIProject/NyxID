//! A bounded, owner-authenticated boundary to the single catalog CMA deployment.
//! Remote payloads are projected here; catalog credentials and arbitrary upstream
//! error text never enter the management API.

use std::{
    sync::{Arc, OnceLock},
    time::Duration,
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use url::Url;

use crate::{
    AppState,
    errors::AppError,
    models::{
        assistant_cma_sandbox::{SandboxCapacitySource, SandboxObservationState},
        downstream_service::DownstreamService,
        user::{COLLECTION_NAME as USERS, User},
    },
};

pub const CMA_SLUG: &str = "cma";
pub const MAX_PAGE_SIZE: usize = 100;
pub const MAX_RESPONSE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_CURSOR_BYTES: usize = 1024;
pub const MAX_STREAM_SECONDS: u64 = 30 * 60;
pub const MAX_DESKTOP_FRAME_BYTES: usize = 8 * 1024 * 1024;
const MAX_SANDBOX_CAPACITY_READS: usize = 8;
const SANDBOX_CAPACITY_READ_BUDGET: Duration = Duration::from_secs(2);
pub type CmaDesktopSocket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

pub type CmaResult<T> = Result<T, CmaGatewayError>;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CmaGatewayError {
    #[error("CMA credential setup is required")]
    CredentialSetupRequired { login_url: String },
    #[error("CMA resource not found")]
    NotFound,
    #[error("CMA access denied")]
    Forbidden,
    #[error("CMA authorization observation is unavailable")]
    UpstreamAuthorizationUnavailable,
    #[error("CMA resource state conflicts with this request")]
    Conflict,
    #[error("CMA request timed out")]
    Timeout,
    #[error("CMA is temporarily unavailable")]
    Unavailable,
    #[error("CMA returned an invalid response")]
    InvalidResponse,
    #[error("CMA configuration is unavailable")]
    ConfigurationRequired,
    #[error("Invalid CMA request")]
    InvalidRequest,
    #[error("CMA resource inventory is incomplete")]
    InventoryIncomplete,
    #[error("CMA desktop is unavailable")]
    DesktopUnavailable { reason: &'static str },
}

impl From<CmaGatewayError> for AppError {
    fn from(error: CmaGatewayError) -> Self {
        match error {
            CmaGatewayError::CredentialSetupRequired { login_url } => Self::Conflict(format!(
                "sandbox_credential_setup_required: Sign in once at {login_url}; keep the original creation request for recovery"
            )),
            CmaGatewayError::NotFound => Self::NotFound("CMA resource not found".into()),
            CmaGatewayError::Forbidden => Self::Forbidden("CMA resource access denied".into()),
            CmaGatewayError::InvalidRequest => Self::ValidationError("Invalid CMA request".into()),
            CmaGatewayError::Conflict => {
                Self::Conflict("CMA resource state changed; refresh before retrying".into())
            }
            CmaGatewayError::ConfigurationRequired => {
                Self::Conflict("cma_configuration_required".into())
            }
            CmaGatewayError::InventoryIncomplete => {
                Self::Conflict("cma_inventory_incomplete".into())
            }
            CmaGatewayError::DesktopUnavailable { reason } => Self::Conflict(reason.into()),
            CmaGatewayError::Timeout
            | CmaGatewayError::Unavailable
            | CmaGatewayError::UpstreamAuthorizationUnavailable
            | CmaGatewayError::InvalidResponse => {
                Self::Conflict("cma_observation_unavailable".into())
            }
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmaSandbox {
    pub id: String,
    pub owner_id: Option<String>,
    pub state: SandboxObservationState,
    pub resource_state: Option<String>,
    pub name: Option<String>,
    pub machine_included: Option<bool>,
    pub cpu_millis: Option<u32>,
    pub memory_mib: Option<u32>,
    #[serde(default)]
    pub capacity_source: Option<SandboxCapacitySource>,
    pub last_activity_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmaPage<T> {
    pub items: Vec<T>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmaLifecycleResult {
    pub pending: bool,
    pub sandbox: Option<CmaSandbox>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmaAgent {
    pub id: String,
    pub owner_id: Option<String>,
    pub sandbox_id: Option<String>,
    pub workspace_id: Option<String>,
    pub worktree_id: Option<String>,
    pub name: Option<String>,
    pub status: String,
    pub phase: Option<String>,
    pub revision: Option<i64>,
    pub profile: Option<Value>,
    pub current_response_id: Option<String>,
    pub desktop: Option<bool>,
    pub sandbox_status: Option<String>,
    /// Missing preserves the previous observation; null or invalid clears it.
    #[serde(
        default,
        deserialize_with = "deserialize_agent_callback_url",
        skip_serializing_if = "Option::is_none"
    )]
    pub callback_url: Option<Option<String>>,
}

fn deserialize_agent_callback_url<'de, D>(
    deserializer: D,
) -> Result<Option<Option<String>>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    Ok(Some(super::cma_operation::validated_callback_url(
        value.as_str(),
    )))
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmaTranscriptMessage {
    pub id: Option<String>,
    pub sequence: Option<i64>,
    pub role: String,
    pub content: Value,
    pub created_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmaTranscriptTurn {
    pub turn_sequence: i64,
    pub started_at: Option<DateTime<Utc>>,
    pub status: String,
    pub messages: Vec<CmaTranscriptMessage>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmaTranscript {
    pub messages: Vec<CmaTranscriptMessage>,
    #[serde(default)]
    pub turns: Vec<CmaTranscriptTurn>,
    pub next_cursor: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmaEvent {
    pub id: Option<String>,
    pub sequence: Option<i64>,
    pub message_sequence: Option<i64>,
    #[serde(rename = "type")]
    pub kind: String,
    pub status: Option<String>,
    pub message: Option<String>,
    pub created_at: Option<DateTime<Utc>>,
    pub cursor: Option<String>,
    pub response_id: Option<String>,
    pub turn_sequence: Option<i64>,
    pub approval_id: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmaEventSnapshot {
    pub events: Vec<CmaEvent>,
    pub next_cursor: Option<String>,
    pub through_cursor: Option<String>,
    pub incarnation: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmaResponse {
    pub id: String,
    pub agent_id: String,
    pub status: String,
    pub output: Option<Value>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmaDesktopDescriptor {
    pub available: bool,
    pub reason: Option<String>,
    pub shared: bool,
    pub width: Option<u32>,
    pub height: Option<u32>,
    /// Server-constructed relative path, never an upstream-selected destination.
    pub websocket_path: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CmaFileWrite {
    pub path: String,
    pub size_bytes: u64,
    pub sha256: String,
}

#[async_trait]
pub trait CmaGateway: Send + Sync {
    async fn service_id(&self, owner_id: &str) -> CmaResult<String>;
    /// Recovery must address the same deployment that accepted the original
    /// request, even when a catalog row keeps its ID while its target changes.
    async fn verify_creation_deployment(
        &self,
        owner_id: &str,
        service_id: &str,
        _fingerprint: &str,
    ) -> CmaResult<()> {
        if self.service_id(owner_id).await? != service_id {
            return Err(CmaGatewayError::ConfigurationRequired);
        }
        Ok(())
    }
    async fn get_controls(
        &self,
        _owner_id: &str,
        _agent_id: &str,
        _response_id: &str,
    ) -> CmaResult<Value> {
        Err(CmaGatewayError::Unavailable)
    }
    async fn get_lifecycle(&self, _owner_id: &str, _operation_id: &str) -> CmaResult<Value> {
        Err(CmaGatewayError::Unavailable)
    }
    async fn list_agents(
        &self,
        _owner_id: &str,
        _cursor: Option<&str>,
    ) -> CmaResult<CmaPage<CmaAgent>> {
        Err(CmaGatewayError::Unavailable)
    }
    async fn get_workspace_sandbox(
        &self,
        _owner_id: &str,
        _workspace_id: &str,
    ) -> CmaResult<String> {
        Err(CmaGatewayError::Unavailable)
    }
    async fn write_agent_file(
        &self,
        _owner_id: &str,
        _agent_id: &str,
        _path: &str,
        _bytes: &[u8],
    ) -> CmaResult<CmaFileWrite> {
        Err(CmaGatewayError::Unavailable)
    }
    async fn get_sandbox(&self, owner_id: &str, sandbox_id: &str) -> CmaResult<CmaSandbox>;
    /// Exact owner/resource observation for authority checks. Display-only
    /// capacity/profile enrichment must not consume a live stream's budget.
    async fn get_sandbox_authority(
        &self,
        owner_id: &str,
        sandbox_id: &str,
    ) -> CmaResult<CmaSandbox> {
        self.get_sandbox(owner_id, sandbox_id).await
    }
    async fn pause_sandbox(
        &self,
        owner_id: &str,
        sandbox_id: &str,
        idempotency_key: &str,
    ) -> CmaResult<CmaLifecycleResult>;
    async fn revive_sandbox(
        &self,
        owner_id: &str,
        sandbox_id: &str,
        idempotency_key: &str,
    ) -> CmaResult<CmaLifecycleResult>;
    async fn retire_sandbox(
        &self,
        owner_id: &str,
        sandbox_id: &str,
        idempotency_key: &str,
    ) -> CmaResult<CmaLifecycleResult>;
    async fn list_sandbox_events(
        &self,
        owner_id: &str,
        sandbox_id: &str,
        cursor: Option<&str>,
    ) -> CmaResult<CmaEventSnapshot>;
    async fn get_agent(&self, owner_id: &str, agent_id: &str) -> CmaResult<CmaAgent>;
    async fn get_transcript(
        &self,
        owner_id: &str,
        agent_id: &str,
        cursor: Option<&str>,
    ) -> CmaResult<CmaTranscript>;
    async fn get_event_snapshot(
        &self,
        owner_id: &str,
        agent_id: &str,
        cursor: Option<&str>,
    ) -> CmaResult<CmaEventSnapshot>;
    async fn get_response(
        &self,
        owner_id: &str,
        agent_id: &str,
        response_id: &str,
    ) -> CmaResult<CmaResponse>;
    async fn desktop_descriptor(
        &self,
        owner_id: &str,
        agent_id: &str,
    ) -> CmaResult<CmaDesktopDescriptor>;
}

#[derive(Clone)]
pub struct CatalogCmaGateway {
    db: mongodb::Database,
    jwt_keys: crate::crypto::jwt::JwtKeys,
    config: crate::config::AppConfig,
    // One boundary instance cannot switch deployments during an operation.
    service_id: Arc<OnceLock<String>>,
    execution_fingerprint: Arc<OnceLock<String>>,
    native_authority: Option<(
        super::assistant_acknowledgement_service::ChatAuthority,
        crate::models::assistant_settings::AccessMode,
    )>,
    watch_authority: Option<(
        crate::models::async_service_operation::AsyncServiceOperation,
        Arc<super::node_ws_manager::NodeWsManager>,
    )>,
}

pub(crate) fn catalog_execution_fingerprint(service: &DownstreamService) -> String {
    // Bind every catalog input used by this boundary. Publication metadata and
    // timestamps do not participate; changing destination or signed identity
    // policy during one operation must stop before the next outbound request.
    let projection = json!({
        "service_id":service.id,"base_url":service.base_url,
        "service_type":service.service_type,"service_category":service.service_category,
        "auth_method":service.auth_method,"requires_user_credential":service.requires_user_credential,
        "identity_propagation_mode":service.identity_propagation_mode,
        "identity_include_user_id":service.identity_include_user_id,
        "identity_include_email":service.identity_include_email,
        "identity_include_name":service.identity_include_name,
        "identity_jwt_audience":service.identity_jwt_audience,
        "forward_access_token":service.forward_access_token,
        "inject_delegation_token":service.inject_delegation_token,
        "delegation_token_scope":service.delegation_token_scope,
        "proxy_operation_policy":service.proxy_operation_policy,
    });
    let mut digest = Sha256::new();
    digest.update(b"nyxid:cma-management-execution:v1\0");
    digest.update(projection.to_string());
    hex::encode(digest.finalize())
}

fn require_same_deployment_target(
    catalog: &DownstreamService,
    service_id: &str,
    base_url: &str,
) -> CmaResult<()> {
    if catalog.id != service_id || catalog.base_url != base_url {
        return Err(CmaGatewayError::ConfigurationRequired);
    }
    Ok(())
}

impl CatalogCmaGateway {
    pub fn new(state: &AppState) -> Self {
        Self {
            db: state.db.clone(),
            jwt_keys: state.jwt_keys.clone(),
            config: state.config.clone(),
            service_id: Arc::new(OnceLock::new()),
            execution_fingerprint: Arc::new(OnceLock::new()),
            native_authority: None,
            watch_authority: None,
        }
    }

    pub fn for_service(state: &AppState, service_id: &str) -> Self {
        Self::from_parts(&state.db, &state.jwt_keys, &state.config, service_id)
    }

    pub fn from_parts(
        db: &mongodb::Database,
        jwt_keys: &crate::crypto::jwt::JwtKeys,
        config: &crate::config::AppConfig,
        service_id: &str,
    ) -> Self {
        let pinned = OnceLock::new();
        let _ = pinned.set(service_id.to_owned());
        Self {
            db: db.clone(),
            jwt_keys: jwt_keys.clone(),
            config: config.clone(),
            service_id: Arc::new(pinned),
            execution_fingerprint: Arc::new(OnceLock::new()),
            native_authority: None,
            watch_authority: None,
        }
    }

    pub(crate) fn with_native_authority(
        mut self,
        chat: &super::assistant_acknowledgement_service::ChatAuthority,
        mode: crate::models::assistant_settings::AccessMode,
    ) -> Self {
        self.native_authority = Some((chat.clone(), mode));
        self
    }

    pub(crate) fn with_watch_authority(
        mut self,
        watch: &crate::models::async_service_operation::AsyncServiceOperation,
        nodes: Arc<super::node_ws_manager::NodeWsManager>,
    ) -> Self {
        self.watch_authority = Some((watch.clone(), nodes));
        self
    }

    /// Compare catalog snapshots for native operations. Ordinary proxy
    /// connections may have their own identity policy and must instead bind
    /// their actual deployment through `deployment_fingerprint_for_target`.
    pub async fn verify_catalog(
        &self,
        owner_id: &str,
        expected: &DownstreamService,
    ) -> CmaResult<()> {
        let (current, _, _) = self.resolve(owner_id).await?;
        if catalog_execution_fingerprint(&current) != catalog_execution_fingerprint(expected) {
            return Err(CmaGatewayError::ConfigurationRequired);
        }
        Ok(())
    }

    pub(crate) async fn deployment_fingerprint_for_target(
        &self,
        owner_id: &str,
        service_id: &str,
        base_url: &str,
    ) -> CmaResult<String> {
        let (catalog, _, _) = self.resolve(owner_id).await?;
        require_same_deployment_target(&catalog, service_id, base_url)?;
        Ok(catalog_execution_fingerprint(&catalog))
    }

    /// A fresh human recovery uses the saved request through the same catalog
    /// admission and final dispatch hooks as ordinary CMA execution.
    pub(super) async fn replay_creation(
        &self,
        keys: &crate::crypto::aes::EncryptionKeys,
        creation: &crate::models::assistant_cma_creation::AssistantCmaCreation,
        body: &[u8],
    ) -> crate::errors::AppResult<()> {
        use super::{proxy_service, service_dispatch_registry as registry};
        let owner_id = &creation.user_id;
        let (service, owner, base) = self.resolve(owner_id).await?;
        if service.id != creation.cma_service_id
            || catalog_execution_fingerprint(&service) != creation.deployment_fingerprint
        {
            return Err(CmaGatewayError::ConfigurationRequired.into());
        }
        authorize_native_route(&service, &creation.method, &creation.path)?;
        let target = proxy_service::resolve_proxy_target(
            &self.db,
            keys,
            owner_id,
            &service.id,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::new(
                self.config.platform_service_rate_limit_per_second,
                self.config.platform_service_rate_limit_burst,
            ),
        )
        .await?;
        let caller = registry::DispatchCaller {
            actor_user_id: owner_id,
            resource_owner_id: owner_id,
            user_service_id: None,
        };
        let admission = registry::admit(
            &self.db,
            keys,
            &self.jwt_keys,
            &self.config,
            &caller,
            true,
            &target,
            &creation.method,
            &creation.path,
            None,
            body,
            || {
                Ok(vec![
                    ("Idempotency-Key".into(), creation.idempotency_key.clone()),
                    (
                        super::assistant_cma_admission::RECOVERY_SUBMISSION_HEADER.into(),
                        super::assistant_cma_admission::recovery_submission(creation),
                    ),
                ]
                .into())
            },
            false,
        )
        .await?;
        let identity = crate::models::default_request_header::SecretHeaders::from(
            self.identity_headers(&service, &owner)
                .await?
                .iter()
                .map(|(name, value)| {
                    Ok((
                        name.as_str().to_owned(),
                        value
                            .to_str()
                            .map_err(|_| CmaGatewayError::InvalidRequest)?
                            .to_owned(),
                    ))
                })
                .collect::<CmaResult<Vec<_>>>()?,
        );
        let headers = proxy_service::build_effective_outbound_headers(
            &target,
            vec![
                ("Idempotency-Key".into(), creation.idempotency_key.clone()),
                ("Accept".into(), "application/json".into()),
                ("Content-Type".into(), "application/json".into()),
            ],
            &identity,
            &[],
            &[],
        );
        // Do not use the inventory URL builder: its pagination parameters are
        // not part of the original creation request.
        let url = format!(
            "{}/{}",
            base.as_str().trim_end_matches('/'),
            creation.path.trim_start_matches('/')
        );
        let mut request = super::nyxbot_gateway_service::GATEWAY_HTTP_CLIENT
            .request(
                reqwest::Method::from_bytes(creation.method.as_bytes())
                    .map_err(|_| CmaGatewayError::InvalidRequest)?,
                url,
            )
            .timeout(Duration::from_secs(20))
            .body(body.to_vec());
        for (name, value) in headers.iter() {
            let mut value = reqwest::header::HeaderValue::from_str(value)
                .map_err(|_| CmaGatewayError::InvalidRequest)?;
            value.set_sensitive(true);
            request = request.header(name.as_str(), value);
        }
        let request = request
            .build()
            .map_err(|_| CmaGatewayError::InvalidRequest)?;
        super::assistant_cma_agent_service::require_enabled(&self.db, owner_id).await?;
        super::assistant_cma_admission::require_recovery_current(&self.db, creation).await?;
        let guard = registry::verify(Some(&admission), &target, Some(&caller))?
            .ok_or(CmaGatewayError::ConfigurationRequired)?;
        guard.before_send(&request).await?;
        let response = super::nyxbot_gateway_service::GATEWAY_HTTP_CLIENT
            .execute(request)
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    CmaGatewayError::Timeout
                } else {
                    CmaGatewayError::Unavailable
                }
            })?;
        let response = guard.observe_response(response).await?;
        let status = response.status().as_u16();
        if !(200..300).contains(&status) {
            let body = bounded_body(response).await;
            return Err(map_status(status, body.as_ref().ok(), &base).into());
        }
        Ok(())
    }

    async fn identity_headers(
        &self,
        service: &DownstreamService,
        owner: &User,
    ) -> CmaResult<reqwest::header::HeaderMap> {
        let principal = super::identity_service::Principal::from(owner);
        let assertion = zeroize::Zeroizing::new(
            super::identity_service::generate_identity_assertion(
                &self.jwt_keys,
                &self.config,
                &principal,
                service,
                &self.db,
            )
            .await
            .map_err(|_| CmaGatewayError::Unavailable)?,
        );
        let mut headers = reqwest::header::HeaderMap::new();
        let mut value = reqwest::header::HeaderValue::from_str(assertion.as_str())
            .map_err(|_| CmaGatewayError::ConfigurationRequired)?;
        value.set_sensitive(true);
        headers.insert("x-nyxid-identity-token", value);
        if service.identity_propagation_mode == "both" {
            for (name, value) in
                super::identity_service::build_identity_headers(&principal, service)
            {
                let name = reqwest::header::HeaderName::from_bytes(name.as_bytes())
                    .map_err(|_| CmaGatewayError::ConfigurationRequired)?;
                let mut value = reqwest::header::HeaderValue::from_str(&value)
                    .map_err(|_| CmaGatewayError::ConfigurationRequired)?;
                value.set_sensitive(true);
                headers.insert(name, value);
            }
        }
        Ok(headers)
    }

    pub async fn open_event_stream(
        &self,
        owner_id: &str,
        agent_id: &str,
        response_id: Option<&str>,
        cursor: Option<&str>,
    ) -> CmaResult<reqwest::Response> {
        validate_remote_id(agent_id)?;
        let (service, owner, base) = self.resolve(owner_id).await?;
        let mut url = match response_id {
            Some(response) => {
                endpoint_url(&base, &["api", "v2", "responses", response, "events"], None)?
            }
            None => endpoint_url(&base, &["api", "v2", "agents", agent_id, "events"], None)?,
        };
        // Stream APIs have no limit parameter; their cursors are different
        // from snapshot pagination and are never interchangeable.
        url.set_query(None);
        if let Some(cursor) = cursor {
            if cursor.is_empty()
                || cursor.len() > MAX_CURSOR_BYTES
                || cursor.chars().any(char::is_control)
                || (response_id.is_some() && cursor.parse::<u64>().is_err())
            {
                return Err(CmaGatewayError::InvalidRequest);
            }
            url.query_pairs_mut().append_pair(
                if response_id.is_some() {
                    "starting_after"
                } else {
                    "after_cursor"
                },
                cursor,
            );
        }
        let response = tokio::time::timeout(
            Duration::from_secs(20),
            super::nyxbot_gateway_service::GATEWAY_HTTP_CLIENT
                .get(url)
                .headers(self.identity_headers(&service, &owner).await?)
                .header(reqwest::header::ACCEPT, "text/event-stream")
                .timeout(Duration::from_secs(MAX_STREAM_SECONDS))
                .send(),
        )
        .await
        .map_err(|_| CmaGatewayError::Timeout)?
        .map_err(|_| CmaGatewayError::Unavailable)?;
        let status = response.status().as_u16();
        if status != 200 {
            let body = bounded_body(response).await;
            return Err(map_status(status, body.as_ref().ok(), &base));
        }
        if !response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value
                    .split(';')
                    .next()
                    .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
            })
        {
            return Err(CmaGatewayError::InvalidResponse);
        }
        if let Some(expected) = response_id
            && response
                .headers()
                .get("x-cma-response-id")
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value != expected)
        {
            return Err(CmaGatewayError::InvalidResponse);
        }
        Ok(response)
    }

    pub async fn open_desktop(
        &self,
        owner_id: &str,
        agent_id: &str,
    ) -> CmaResult<CmaDesktopSocket> {
        use tokio_tungstenite::tungstenite::{
            client::IntoClientRequest, protocol::WebSocketConfig,
        };
        let (service, owner, base) = self.resolve(owner_id).await?;
        let mut url = endpoint_url(
            &base,
            &["api", "v1", "agents", agent_id, "desktop", "ws"],
            None,
        )?;
        url.set_scheme(if base.scheme() == "https" {
            "wss"
        } else {
            "ws"
        })
        .map_err(|_| CmaGatewayError::ConfigurationRequired)?;
        let mut request = url
            .as_str()
            .into_client_request()
            .map_err(|_| CmaGatewayError::InvalidRequest)?;
        request
            .headers_mut()
            .extend(self.identity_headers(&service, &owner).await?);
        // No browser cookies or caller-selected Origin reach CMA. Its own
        // owner-authenticated open is the shared Sandbox single-viewer fence.
        let mut limits = WebSocketConfig::default();
        limits.max_message_size = Some(MAX_DESKTOP_FRAME_BYTES);
        limits.max_frame_size = Some(MAX_DESKTOP_FRAME_BYTES);
        let result = tokio::time::timeout(
            Duration::from_secs(20),
            tokio_tungstenite::connect_async_with_config(request, Some(limits), true),
        )
        .await
        .map_err(|_| CmaGatewayError::Timeout)?;
        match result {
            Ok((socket, _)) => Ok(socket),
            Err(tokio_tungstenite::tungstenite::Error::Http(response)) => {
                let body = response
                    .body()
                    .as_ref()
                    .filter(|body| body.len() <= MAX_RESPONSE_BYTES)
                    .and_then(|body| serde_json::from_slice::<Value>(body).ok());
                Err(map_status(response.status().as_u16(), body.as_ref(), &base))
            }
            Err(_) => Err(CmaGatewayError::Unavailable),
        }
    }

    async fn resolve(&self, owner_id: &str) -> CmaResult<(DownstreamService, User, Url)> {
        let owner = self
            .db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": owner_id, "is_active": true})
            .await
            .map_err(|_| CmaGatewayError::Unavailable)?
            .filter(|user| user.user_type.is_person())
            .ok_or(CmaGatewayError::Forbidden)?;
        let service =
            super::catalog_service::get_downstream_service_by_slug(&self.db, CMA_SLUG, owner_id)
                .await
                .map_err(|error| {
                    if matches!(error, AppError::NotFound(_)) {
                        CmaGatewayError::ConfigurationRequired
                    } else {
                        CmaGatewayError::Unavailable
                    }
                })?;
        if service.service_type != "http"
            || service.requires_user_credential
            || service.auth_method != "none"
            || !matches!(service.identity_propagation_mode.as_str(), "jwt" | "both")
        {
            return Err(CmaGatewayError::ConfigurationRequired);
        }
        if self.service_id.get_or_init(|| service.id.clone()) != &service.id {
            return Err(CmaGatewayError::ConfigurationRequired);
        }
        let fingerprint = catalog_execution_fingerprint(&service);
        if self
            .execution_fingerprint
            .get_or_init(|| fingerprint.clone())
            != &fingerprint
        {
            return Err(CmaGatewayError::ConfigurationRequired);
        }
        let base = validated_base(&service.base_url)?;
        Ok((service, owner, base))
    }

    async fn request(
        &self,
        owner_id: &str,
        method: reqwest::Method,
        segments: &[&str],
        cursor: Option<&str>,
        idempotency_key: Option<&str>,
    ) -> CmaResult<(u16, Value)> {
        self.request_body(owner_id, method, segments, cursor, idempotency_key, None)
            .await
    }

    async fn pinned_sandbox_capacity(
        &self,
        owner_id: &str,
        profile: PinnedSandboxProfile,
    ) -> CmaResult<SandboxCapacity> {
        let revision = profile.revision.to_string();
        // Use the ordinary request boundary so native/watch input scopes and
        // catalog policy still authorize this exact additional GET.
        let (_, body) = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &[
                    "api",
                    "v1",
                    "sandbox-profiles",
                    &profile.id,
                    "revisions",
                    &revision,
                ],
                None,
                None,
            )
            .await?;
        parse_sandbox_capacity(&profile, &body)
    }

    async fn runtime_sandbox_capacity(
        &self,
        owner_id: &str,
        sandbox: CmaSandbox,
    ) -> CmaResult<SandboxCapacity> {
        // This optional read uses the same catalog, native and watch scopes as
        // every other owner request. A missing grant only leaves capacity unknown.
        let (_, body) = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &["api", "v1", "usage", "sandboxes", &sandbox.id],
                None,
                None,
            )
            .await?;
        parse_sandbox_allocation(&sandbox, &body)
    }

    async fn request_body(
        &self,
        owner_id: &str,
        method: reqwest::Method,
        segments: &[&str],
        cursor: Option<&str>,
        idempotency_key: Option<&str>,
        body: Option<Vec<u8>>,
    ) -> CmaResult<(u16, Value)> {
        let (service, owner, base) = self.resolve(owner_id).await?;
        let url = endpoint_url(&base, segments, cursor)?;
        let principal = super::identity_service::Principal::from(&owner);
        let assertion = zeroize::Zeroizing::new(
            super::identity_service::generate_identity_assertion(
                &self.jwt_keys,
                &self.config,
                &principal,
                &service,
                &self.db,
            )
            .await
            .map_err(|_| CmaGatewayError::Unavailable)?,
        );
        let mut request = super::nyxbot_gateway_service::GATEWAY_HTTP_CLIENT
            .request(method, url)
            .timeout(Duration::from_secs(20))
            .header(reqwest::header::ACCEPT, "application/json")
            .header("X-NyxID-Identity-Token", assertion.as_str());
        if service.identity_propagation_mode == "both" {
            for (name, value) in
                super::identity_service::build_identity_headers(&principal, &service)
            {
                request = request.header(name, value);
            }
        }
        if let Some(key) = idempotency_key {
            validate_idempotency_key(key)?;
            request = request.header("Idempotency-Key", key);
        }
        if let Some(body) = body {
            request = request
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body);
        }
        let request = request
            .build()
            .map_err(|_| CmaGatewayError::InvalidRequest)?;
        if let Some((chat, mode)) = &self.native_authority {
            if chat.user_id != owner_id {
                return Err(CmaGatewayError::Forbidden);
            }
            authorize_native_route(
                &service,
                request.method().as_str(),
                &format!("/{}", segments.join("/")),
            )?;
            super::assistant_cma_tools::scope(
                &self.db,
                chat,
                &service.id,
                request.method().as_str(),
                &format!("/{}", segments.join("/")),
                request.url().query(),
                request
                    .body()
                    .and_then(reqwest::Body::as_bytes)
                    .unwrap_or_default(),
                *mode,
            )
            .await
            .map_err(|_| CmaGatewayError::Forbidden)?;
        }
        if let Some((watch, nodes)) = &self.watch_authority {
            if watch.user_id != owner_id {
                return Err(CmaGatewayError::Forbidden);
            }
            let path = format!("/{}", segments.join("/"));
            authorize_native_route(&service, request.method().as_str(), &path)?;
            super::assistant_cma_tools::watch_scope(
                &self.db,
                nodes,
                watch,
                &service.id,
                request.method().as_str(),
                &path,
                request.url().query(),
                request
                    .body()
                    .and_then(reqwest::Body::as_bytes)
                    .unwrap_or_default(),
            )
            .await
            .map_err(|_| CmaGatewayError::Forbidden)?;
        }
        let response = super::nyxbot_gateway_service::GATEWAY_HTTP_CLIENT
            .execute(request)
            .await
            .map_err(|error| {
                if error.is_timeout() {
                    CmaGatewayError::Timeout
                } else {
                    CmaGatewayError::Unavailable
                }
            })?;
        let status = response.status().as_u16();
        let body = bounded_body(response).await;
        if !(200..300).contains(&status) {
            return Err(map_status(status, body.as_ref().ok(), &base));
        }
        Ok((status, body?))
    }

    async fn lifecycle(
        &self,
        owner_id: &str,
        sandbox_id: &str,
        action: Option<&str>,
        key: &str,
    ) -> CmaResult<CmaLifecycleResult> {
        validate_remote_id(sandbox_id)?;
        let mut path = vec!["api", "v1", "sandboxes", sandbox_id];
        if let Some(action) = action {
            path.push(action);
        }
        let (status, body) = self
            .request(
                owner_id,
                if action.is_some() {
                    reqwest::Method::POST
                } else {
                    reqwest::Method::DELETE
                },
                &path,
                None,
                Some(key),
            )
            .await?;
        let item = body.get("sandbox").unwrap_or(&body);
        let sandbox = if item.get("id").is_some() || item.get("sandbox_id").is_some() {
            Some(parse_sandbox(item, owner_id)?)
        } else {
            None
        };
        if sandbox.as_ref().is_some_and(|item| item.id != sandbox_id) {
            return Err(CmaGatewayError::InvalidResponse);
        }
        Ok(CmaLifecycleResult {
            pending: status == 202
                || sandbox.as_ref().is_none_or(|item| {
                    matches!(
                        item.state,
                        SandboxObservationState::Unknown
                            | SandboxObservationState::Provisioning
                            | SandboxObservationState::Retiring
                    )
                }),
            sandbox,
        })
    }
}

pub fn validate_remote_id(value: &str) -> CmaResult<()> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':'))
        || matches!(value, "." | "..")
    {
        return Err(CmaGatewayError::InvalidRequest);
    }
    Ok(())
}

pub fn validate_idempotency_key(value: &str) -> CmaResult<()> {
    if value.is_empty()
        || value.len() > 200
        || !value
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'"' && b != b'\\')
    {
        return Err(CmaGatewayError::InvalidRequest);
    }
    Ok(())
}

fn validated_base(raw: &str) -> CmaResult<Url> {
    if raw.len() > 2048 {
        return Err(CmaGatewayError::ConfigurationRequired);
    }
    let url = Url::parse(raw).map_err(|_| CmaGatewayError::ConfigurationRequired)?;
    let loopback = matches!(url.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(CmaGatewayError::ConfigurationRequired);
    }
    Ok(url)
}

fn endpoint_url(base: &Url, segments: &[&str], cursor: Option<&str>) -> CmaResult<Url> {
    for segment in segments {
        validate_remote_id(segment)?;
    }
    let mut url = base.clone();
    url.path_segments_mut()
        .map_err(|_| CmaGatewayError::ConfigurationRequired)?
        .pop_if_empty()
        .extend(segments);
    if let Some(cursor) = cursor {
        if cursor.is_empty()
            || cursor.len() > MAX_CURSOR_BYTES
            || cursor.chars().any(char::is_control)
        {
            return Err(CmaGatewayError::InvalidRequest);
        }
        let query_key = if segments.ends_with(&["events", "snapshot"]) {
            "page_cursor"
        } else if segments.get(2) == Some(&"sandboxes") && segments.last() == Some(&"events") {
            if cursor.parse::<u64>().is_err() {
                return Err(CmaGatewayError::InvalidRequest);
            }
            "after"
        } else {
            "cursor"
        };
        url.query_pairs_mut().append_pair(query_key, cursor);
    }
    if segments.last() == Some(&"events") || segments.ends_with(&["events", "snapshot"]) {
        url.query_pairs_mut().append_pair("limit", "100");
    }
    if segments == ["api", "v1", "agents"] {
        url.query_pairs_mut().append_pair("limit", "8");
    }
    if url.as_str().len() > 4096 {
        return Err(CmaGatewayError::InvalidRequest);
    }
    Ok(url)
}

fn authorize_native_route(service: &DownstreamService, method: &str, path: &str) -> CmaResult<()> {
    let path = super::proxy_authorization::CanonicalPath::from_mcp_literal(path)
        .map_err(|_| CmaGatewayError::InvalidRequest)?;
    super::proxy_authorization::authorize_proxy_operation(service, method, &path)
        .map_err(|_| CmaGatewayError::Forbidden)?;
    Ok(())
}

#[cfg(test)]
mod native_policy_tests {
    use super::*;

    #[tokio::test]
    async fn cma_watch_enrichment_requires_leased_original_legacy_authority() {
        use crate::models::async_service_operation::{
            AsyncServiceOperation, COLLECTION_NAME as WATCHES,
        };
        use crate::services::{
            assistant_authority_tests::orchestrator_fixture,
            assistant_service_execution as execution, cma_session_origin, mcp_service,
        };
        use crate::test_utils::{
            test_auto_connected_catalog_service, test_service_endpoint, test_user_endpoint,
            test_user_service,
        };
        use axum::{Json, Router, routing::get};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let f = orchestrator_fixture("cma_watch_enrichment_authority").await;
        let reads = Arc::new(AtomicUsize::new(0));
        let observed = reads.clone();
        let app = Router::new().route(
            "/api/v1/agents/agt_observed",
            get(move || {
                observed.fetch_add(1, Ordering::SeqCst);
                async {
                    Json(json!({
                        "id":"agt_observed","status":"working","revision":1
                    }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let db = &f.state.db;
        let mut catalog = test_auto_connected_catalog_service();
        catalog.slug = "cma".into();
        catalog.base_url = base.clone();
        catalog.identity_propagation_mode = "jwt".into();
        db.collection::<DownstreamService>("downstream_services")
            .insert_one(&catalog)
            .await
            .unwrap();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let connection_id = uuid::Uuid::new_v4().to_string();
        db.collection("user_endpoints")
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &f.owner,
                "CMA",
                &base,
                None,
                Some(&catalog.id),
            ))
            .await
            .unwrap();
        let mut connection = test_user_service(
            &connection_id,
            &f.owner,
            "sandbox",
            &endpoint_id,
            Some(&catalog.id),
            None,
        );
        connection.identity_propagation_mode = "jwt".into();
        connection.identity_jwt_audience = catalog.identity_jwt_audience.clone();
        db.collection("user_services")
            .insert_one(connection)
            .await
            .unwrap();
        db.collection::<bson::Document>("api_keys")
            .update_one(
                doc! {"_id":&f.row.credential_api_key_id},
                doc! {"$set":{"allow_all_services":true}},
            )
            .await
            .unwrap();
        let mut submit = test_service_endpoint(
            &catalog.id,
            "submit_response",
            "POST",
            "/api/v2/agents/{agent_id}/responses",
        );
        submit.parameters = Some(json!([{
            "name":"agent_id","in":"path","required":true,"schema":{"type":"string"}
        }]));
        let mut status = test_service_endpoint(
            &catalog.id,
            "response_status",
            "GET",
            "/api/v2/responses/{response_id}",
        );
        status.parameters = Some(json!([{
            "name":"response_id","in":"path","required":true,"schema":{"type":"string"}
        }]));
        db.collection::<crate::models::service_endpoint::ServiceEndpoint>("service_endpoints")
            .insert_many([&submit, &status])
            .await
            .unwrap();
        let published = mcp_service::load_operation_catalog(
            db,
            &f.state.node_ws_manager,
            &f.owner,
            mcp_service::NodeScope::Unrestricted,
            mcp_service::ServiceScope::Unrestricted,
        )
        .await
        .unwrap();
        let service = published
            .services
            .iter()
            .find(|service| service.service_id == connection_id)
            .unwrap();
        let submit = service
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == submit.id)
            .unwrap();
        let now = Utc::now();
        let mut watch: AsyncServiceOperation = bson::from_document(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"user_id":&f.owner,
            "conversation_id":&f.row.id,"agent_id":&f.row.agent_id,
            "turn_id":&f.row.active_turn.as_ref().unwrap().turn_id,
            "api_key_id":&f.row.credential_api_key_id,"service_id":&connection_id,
            "submit_endpoint_id":&submit.endpoint_id,"contract":bson::to_bson(submit.async_operation.as_ref().unwrap()).unwrap(),
            "contract_binding":execution::async_contract_binding(service,submit).unwrap(),
            "execution_binding":execution::connection_binding(db,&connection_id,&f.row.credential_api_key_id).await.unwrap(),
            "authority_version":1_i32,"operation_id":"resp_observed",
            "cma":{"kind":"cma_response","agent_id":"agt_observed",
                "observer_binding":cma_session_origin::personal_binding(db,&f.owner,&connection_id,Some(&f.row.credential_api_key_id)).await.unwrap()},
            "delivery":{"origin":"user"},"state":"waiting","attempts":0,
            "lease_id":null,"lease_until":null,"created_at":bson::DateTime::from_chrono(now),
            "deadline":bson::DateTime::from_chrono(now+chrono::Duration::hours(1)),
            "poll_at":bson::DateTime::from_chrono(now),
            "expires_at":bson::DateTime::from_chrono(now+chrono::Duration::hours(2)),
        })
        .unwrap();
        let watches = db.collection::<bson::Document>(WATCHES);
        watches
            .insert_one(bson::to_document(&watch).unwrap())
            .await
            .unwrap();
        let read = |watch: &AsyncServiceOperation| {
            CatalogCmaGateway::for_service(&f.state, &catalog.id)
                .with_watch_authority(watch, f.state.node_ws_manager.clone())
        };
        assert!(
            read(&watch)
                .get_agent(&f.owner, "agt_observed")
                .await
                .is_err(),
            "submission receipts have no polling lease"
        );
        watch.lease_id = Some("original-lease".into());
        watch.lease_until = Some(now + chrono::Duration::minutes(1));
        watches
            .replace_one(doc! {"_id":&watch.id}, bson::to_document(&watch).unwrap())
            .await
            .unwrap();
        read(&watch)
            .get_agent(&f.owner, "agt_observed")
            .await
            .unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 1);

        // A same-ID lease expiry must stop the next supplemental provider read.
        watches
            .update_one(
                doc! {"_id":&watch.id},
                doc! {"$set":{"lease_until":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        assert!(
            read(&watch)
                .get_agent(&f.owner, "agt_observed")
                .await
                .is_err()
        );
        watches
            .replace_one(doc! {"_id":&watch.id}, bson::to_document(&watch).unwrap())
            .await
            .unwrap();

        // The gateway's canonical catalog route cannot bless a changed original connection.
        db.collection::<bson::Document>("user_endpoints")
            .update_one(
                doc! {"_id":&endpoint_id},
                doc! {"$set":{"url":format!("{base}/changed")}},
            )
            .await
            .unwrap();
        assert!(
            read(&watch)
                .get_agent(&f.owner, "agt_observed")
                .await
                .is_err()
        );
        db.collection::<bson::Document>("user_endpoints")
            .update_one(doc! {"_id":&endpoint_id}, doc! {"$set":{"url":&base}})
            .await
            .unwrap();

        // Endpoint-only drift is outside the connection digest and needs the live contract gate.
        db.collection::<bson::Document>("service_endpoints")
            .update_one(
                doc! {"_id":&status.id},
                doc! {"$set":{"path":"/api/v2/responses/{response_id}/changed"}},
            )
            .await
            .unwrap();
        assert!(
            read(&watch)
                .get_agent(&f.owner, "agt_observed")
                .await
                .is_err()
        );
        db.collection::<bson::Document>("service_endpoints")
            .update_one(doc! {"_id":&status.id}, doc! {"$set":{"path":&status.path}})
            .await
            .unwrap();

        // Optional reads have no independent execution-task route admission yet.
        // Refuse both a marked snapshot and an older snapshot of the marked row.
        let legacy = watch.clone();
        let mut task =
            crate::services::execution_permission_dispatch::phase_b_tests::task(db, &f.row).await;
        task.parent_turn_id = Some(watch.turn_id.clone());
        watch.execution_task = Some(task.clone());
        watches
            .replace_one(doc! {"_id":&watch.id}, bson::to_document(&watch).unwrap())
            .await
            .unwrap();
        for snapshot in [&watch, &legacy] {
            assert!(
                read(snapshot)
                    .get_agent(&f.owner, "agt_observed")
                    .await
                    .is_err()
            );
        }
        db.collection::<bson::Document>("assistant_messages")
            .update_one(doc! {"_id":&task.task.root_id},doc! {"$set":{"execution_task.cancelled":true},"$inc":{"execution_task.epoch":1_i64}})
            .await.unwrap();
        assert!(
            read(&watch)
                .get_agent(&f.owner, "agt_observed")
                .await
                .is_err()
        );
        assert_eq!(
            reads.load(Ordering::SeqCst),
            1,
            "denied enrichments never reach the provider"
        );

        // Removing only this fixture's marked descendant restores the legacy path.
        watches
            .replace_one(doc! {"_id":&legacy.id}, bson::to_document(&legacy).unwrap())
            .await
            .unwrap();
        read(&legacy)
            .get_agent(&f.owner, "agt_observed")
            .await
            .unwrap();
        assert_eq!(reads.load(Ordering::SeqCst), 2);
        server.abort();
    }

    #[test]
    fn native_catalog_policy_denies_unpublished_file_changes() {
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        catalog.proxy_operation_policy =
            Some(crate::models::downstream_service::ProxyOperationPolicy {
                rules: vec![crate::models::downstream_service::ProxyOperationRule {
                    method: "GET".into(),
                    path_template: "/api/v1/agents".into(),
                    ..Default::default()
                }],
            });
        assert!(authorize_native_route(&catalog, "GET", "/api/v1/agents").is_ok());
        assert_eq!(
            authorize_native_route(&catalog, "POST", "/api/v1/agents/agt_test/files"),
            Err(CmaGatewayError::Forbidden)
        );
        catalog
            .proxy_operation_policy
            .as_mut()
            .unwrap()
            .rules
            .clear();
        assert_eq!(
            authorize_native_route(&catalog, "GET", "/api/v1/agents"),
            Err(CmaGatewayError::Forbidden)
        );
    }

    #[test]
    fn optional_capacity_reads_do_not_bypass_catalog_operation_policy() {
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        catalog.proxy_operation_policy =
            Some(crate::models::downstream_service::ProxyOperationPolicy {
                rules: vec![crate::models::downstream_service::ProxyOperationRule {
                    method: "GET".into(),
                    path_template: "/api/v1/sandboxes/{id}".into(),
                    ..Default::default()
                }],
            });
        assert!(authorize_native_route(&catalog, "GET", "/api/v1/sandboxes/sbx_1").is_ok());
        for path in [
            "/api/v1/sandbox-profiles/sbp_1/revisions/7",
            "/api/v1/usage/sandboxes/sbx_1",
        ] {
            assert_eq!(
                authorize_native_route(&catalog, "GET", path),
                Err(CmaGatewayError::Forbidden)
            );
        }
        catalog.proxy_operation_policy.as_mut().unwrap().rules.push(
            crate::models::downstream_service::ProxyOperationRule {
                method: "GET".into(),
                path_template: "/api/v1/usage/sandboxes/{id}".into(),
                ..Default::default()
            },
        );
        assert!(authorize_native_route(&catalog, "GET", "/api/v1/usage/sandboxes/sbx_1").is_ok());
        assert_eq!(
            authorize_native_route(&catalog, "GET", "/api/v1/usage/sandboxes"),
            Err(CmaGatewayError::Forbidden)
        );
    }
}

fn map_status(status: u16, body: Option<&Value>, base: &Url) -> CmaGatewayError {
    let code = body
        .and_then(|body| body.get("code").or_else(|| body.get("error")))
        .and_then(|value| {
            value
                .as_str()
                .or_else(|| value.get("code").and_then(Value::as_str))
        });
    if status == 409 && code == Some("sandbox_credential_setup_required") {
        let mut login = base.clone();
        if let Ok(mut segments) = login.path_segments_mut() {
            segments.pop_if_empty().extend(["auth", "login"]);
        }
        return CmaGatewayError::CredentialSetupRequired {
            login_url: login.to_string(),
        };
    }
    match (status, code) {
        (404, Some("desktop_unavailable")) => {
            return CmaGatewayError::DesktopUnavailable {
                reason: "desktop_unavailable",
            };
        }
        (409, Some("desktop_busy")) => {
            return CmaGatewayError::DesktopUnavailable {
                reason: "desktop_busy",
            };
        }
        (503, Some("desktop_runtime_unavailable")) => {
            return CmaGatewayError::DesktopUnavailable {
                reason: "desktop_runtime_unavailable",
            };
        }
        _ => {}
    }
    match status {
        401 | 403 => CmaGatewayError::UpstreamAuthorizationUnavailable,
        404 => CmaGatewayError::NotFound,
        409 => CmaGatewayError::Conflict,
        _ => CmaGatewayError::Unavailable,
    }
}

async fn bounded_body(mut response: reqwest::Response) -> CmaResult<Value> {
    if response
        .content_length()
        .is_some_and(|len| len > MAX_RESPONSE_BYTES as u64)
    {
        return Err(CmaGatewayError::InvalidResponse);
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| CmaGatewayError::InvalidResponse)?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
            return Err(CmaGatewayError::InvalidResponse);
        }
        bytes.extend_from_slice(&chunk);
    }
    if bytes.is_empty() {
        return Ok(Value::Null);
    }
    serde_json::from_slice(&bytes).map_err(|_| CmaGatewayError::InvalidResponse)
}

fn text(value: &Value, key: &str, max: usize) -> Option<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .filter(|v| !v.is_empty() && v.len() <= max && !v.contains('\0'))
        .map(str::to_owned)
}
fn date(value: &Value, key: &str) -> Option<DateTime<Utc>> {
    value
        .get(key)
        .and_then(Value::as_str)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}
fn number(value: &Value, key: &str) -> Option<u32> {
    value
        .get(key)
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
}
fn id(value: &Value, key: &str) -> CmaResult<String> {
    let id = text(value, key, 256).ok_or(CmaGatewayError::InvalidResponse)?;
    validate_remote_id(&id).map_err(|_| CmaGatewayError::InvalidResponse)?;
    Ok(id)
}

fn owner(value: &Value, expected: &str) -> CmaResult<Option<String>> {
    let owner = text(value, "owner", 256)
        .or_else(|| text(value, "owner_id", 256))
        .or_else(|| text(value, "user_id", 256));
    if owner.as_deref().is_some_and(|owner| owner != expected) {
        return Err(CmaGatewayError::Forbidden);
    }
    Ok(owner)
}

fn observation_state(raw: &str) -> SandboxObservationState {
    match raw {
        "creating"
        | "provisioning"
        | "installing"
        | "starting"
        | "resuming"
        | "reviving"
        | "rotating"
        | "credential_rotating" => SandboxObservationState::Provisioning,
        "ready" | "running" | "active" => SandboxObservationState::Ready,
        "paused" | "suspended" => SandboxObservationState::Paused,
        "retiring" | "deleting" => SandboxObservationState::Retiring,
        "retired" => SandboxObservationState::Retired,
        "failed" | "error" => SandboxObservationState::Failed,
        _ => SandboxObservationState::Unknown,
    }
}

fn parse_sandbox(value: &Value, expected_owner: &str) -> CmaResult<CmaSandbox> {
    let owner_id = owner(value, expected_owner)?.ok_or(CmaGatewayError::Forbidden)?;
    Ok(CmaSandbox {
        id: id(
            value,
            if value.get("id").is_some() {
                "id"
            } else {
                "sandbox_id"
            },
        )?,
        owner_id: Some(owner_id),
        state: observation_state(
            value
                .get("status")
                .or_else(|| value.get("state"))
                .and_then(Value::as_str)
                .unwrap_or("unknown"),
        ),
        resource_state: text(value, "status", 120).or_else(|| text(value, "state", 120)),
        name: text(value, "name", 120),
        machine_included: value.get("machine_included").and_then(Value::as_bool),
        // SandboxView has no capacity projection. Only the exact profile or
        // owner allocation read below can establish these optional facts.
        cpu_millis: None,
        memory_mib: None,
        capacity_source: None,
        last_activity_at: date(value, "last_activity_at"),
    })
}

fn parse_exact_sandbox(
    value: &Value,
    expected_owner: &str,
    expected_id: &str,
) -> CmaResult<CmaSandbox> {
    let sandbox = parse_sandbox(value, expected_owner)?;
    if sandbox.id != expected_id {
        return Err(CmaGatewayError::InvalidResponse);
    }
    Ok(sandbox)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct PinnedSandboxProfile {
    id: String,
    revision: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SandboxCapacity {
    cpu_millis: u32,
    memory_mib: u32,
    source: SandboxCapacitySource,
}

fn pinned_sandbox_profile(value: &Value) -> Option<PinnedSandboxProfile> {
    let profile = value.get("profile")?;
    Some(PinnedSandboxProfile {
        id: id(profile, "id").ok()?,
        revision: profile.get("revision")?.as_u64()?.try_into().ok()?,
    })
}

fn parse_sandbox_capacity(
    expected: &PinnedSandboxProfile,
    value: &Value,
) -> CmaResult<SandboxCapacity> {
    if value.get("profile_id").and_then(Value::as_str) != Some(expected.id.as_str())
        || value.get("revision").and_then(Value::as_u64) != Some(u64::from(expected.revision))
    {
        return Err(CmaGatewayError::InvalidResponse);
    }
    let content = value
        .get("content")
        .ok_or(CmaGatewayError::InvalidResponse)?;
    Ok(SandboxCapacity {
        cpu_millis: number(content, "cpu_millis")
            .filter(|value| *value > 0)
            .ok_or(CmaGatewayError::InvalidResponse)?,
        memory_mib: number(content, "memory_mib")
            .filter(|value| *value > 0)
            .ok_or(CmaGatewayError::InvalidResponse)?,
        source: SandboxCapacitySource::ProfileRevision,
    })
}

fn parse_sandbox_allocation(expected: &CmaSandbox, value: &Value) -> CmaResult<SandboxCapacity> {
    let expected_owner = expected
        .owner_id
        .as_deref()
        .ok_or(CmaGatewayError::InvalidResponse)?;
    if value.get("owner").and_then(Value::as_str) != Some(expected_owner) {
        return Err(CmaGatewayError::Forbidden);
    }
    // Allocation is a separate current-usage snapshot, not the immutable
    // profile. Refuse lifecycle drift and require the exact public DTO identity.
    if value.get("sandbox_id").and_then(Value::as_str) != Some(expected.id.as_str())
        || !active_sandbox_allocation(expected)
        || value.get("status").and_then(Value::as_str) != expected.resource_state.as_deref()
    {
        return Err(CmaGatewayError::InvalidResponse);
    }
    let allocation = value
        .get("allocation")
        .ok_or(CmaGatewayError::InvalidResponse)?;
    if allocation.get("running").and_then(Value::as_bool) != Some(true) {
        return Err(CmaGatewayError::InvalidResponse);
    }
    Ok(SandboxCapacity {
        cpu_millis: number(allocation, "cpu_millis")
            .filter(|value| *value > 0)
            .ok_or(CmaGatewayError::InvalidResponse)?,
        memory_mib: number(allocation, "memory_mib")
            .filter(|value| *value > 0)
            .ok_or(CmaGatewayError::InvalidResponse)?,
        source: SandboxCapacitySource::RuntimeAllocation,
    })
}

fn active_sandbox_allocation(sandbox: &CmaSandbox) -> bool {
    sandbox.state == SandboxObservationState::Ready
        && matches!(
            sandbox.resource_state.as_deref(),
            Some("active" | "ready" | "running")
        )
}

/// Optional display enrichment with one total read count and deadline. Prefer
/// immutable exact pins, then current owner allocation for active Sandboxes.
/// Cache exact profile results only within this call. Failures never invalidate
/// the lifecycle observation or authorize any account-slot transition.
async fn hydrate_sandbox_capacity<F, Fut, A, AFut>(
    items: &mut [(CmaSandbox, Option<PinnedSandboxProfile>)],
    budget: Duration,
    mut read_profile: F,
    mut read_allocation: A,
) where
    F: FnMut(PinnedSandboxProfile) -> Fut,
    Fut: std::future::Future<Output = CmaResult<SandboxCapacity>>,
    A: FnMut(CmaSandbox) -> AFut,
    AFut: std::future::Future<Output = CmaResult<SandboxCapacity>>,
{
    let deadline = tokio::time::Instant::now() + budget;
    let mut cache = std::collections::HashMap::new();
    let mut reads = 0;
    for (sandbox, profile) in items {
        sandbox.cpu_millis = None;
        sandbox.memory_mib = None;
        sandbox.capacity_source = None;
        let mut capacity = if let Some(profile) = profile {
            if let Some(cached) = cache.get(profile) {
                *cached
            } else if reads < MAX_SANDBOX_CAPACITY_READS && tokio::time::Instant::now() < deadline {
                reads += 1;
                let observed = tokio::time::timeout_at(deadline, read_profile(profile.clone()))
                    .await
                    .ok()
                    .and_then(Result::ok);
                cache.insert(profile.clone(), observed);
                observed
            } else {
                None
            }
        } else {
            None
        };
        if capacity.is_none()
            && active_sandbox_allocation(sandbox)
            && reads < MAX_SANDBOX_CAPACITY_READS
            && tokio::time::Instant::now() < deadline
        {
            reads += 1;
            capacity = tokio::time::timeout_at(deadline, read_allocation(sandbox.clone()))
                .await
                .ok()
                .and_then(Result::ok);
        }
        if let Some(capacity) = capacity {
            sandbox.cpu_millis = Some(capacity.cpu_millis);
            sandbox.memory_mib = Some(capacity.memory_mib);
            sandbox.capacity_source = Some(capacity.source);
        }
    }
}

fn page_items<'a>(body: &'a Value, key: &str) -> CmaResult<&'a Vec<Value>> {
    let items = body
        .as_array()
        .or_else(|| body.get(key).and_then(Value::as_array))
        .or_else(|| body.get("items").and_then(Value::as_array))
        .ok_or(CmaGatewayError::InvalidResponse)?;
    if items.len() > MAX_PAGE_SIZE {
        return Err(CmaGatewayError::InventoryIncomplete);
    }
    Ok(items)
}

fn next_cursor(body: &Value) -> CmaResult<Option<String>> {
    let value = body
        .get("next_page_cursor")
        .or_else(|| body.get("next"))
        .or_else(|| body.get("next_cursor"))
        .or_else(|| body.get("nextCursor"));
    let cursor = match value {
        None | Some(Value::Null) => None,
        Some(Value::String(cursor))
            if !cursor.is_empty()
                && cursor.len() <= MAX_CURSOR_BYTES
                && !cursor.chars().any(char::is_control) =>
        {
            Some(cursor.clone())
        }
        _ => return Err(CmaGatewayError::InvalidResponse),
    };
    if body.get("has_more").and_then(Value::as_bool) == Some(true) && cursor.is_none() {
        return Err(CmaGatewayError::InventoryIncomplete);
    }
    Ok(cursor)
}

fn events(body: &Value) -> CmaResult<CmaEventSnapshot> {
    let events = page_items(body, "events")?
        .iter()
        .map(|value| {
            let kind = value.get("kind").unwrap_or(value);
            let binding = value.get("response_binding").unwrap_or(&Value::Null);
            Ok(CmaEvent {
                id: text(value, "id", 256),
                sequence: value
                    .get("sequence")
                    .or_else(|| value.get("seq"))
                    .and_then(Value::as_i64),
                message_sequence: (kind.get("type").and_then(Value::as_str) == Some("message"))
                    .then(|| {
                        value
                            .get("sequence")
                            .or_else(|| value.get("seq"))
                            .and_then(Value::as_i64)
                    })
                    .flatten(),
                kind: text(kind, "type", 120).unwrap_or_else(|| "unknown".into()),
                status: text(kind, "status", 120).or_else(|| text(kind, "to", 120)),
                message: text(kind, "content", 16 * 1024)
                    .or_else(|| text(kind, "title", 16 * 1024))
                    .or_else(|| text(kind, "message", 16 * 1024)),
                created_at: date(value, "appended_at").or_else(|| date(value, "created_at")),
                cursor: text(value, "cursor", MAX_CURSOR_BYTES),
                response_id: text(binding, "response_id", 256),
                turn_sequence: binding
                    .get("turn_sequence")
                    .or_else(|| kind.get("turn_sequence"))
                    .and_then(Value::as_i64),
                approval_id: matches!(
                    kind.get("type").and_then(Value::as_str),
                    Some("approval_requested" | "approval_resolved")
                )
                .then(|| id(kind, "approval_id"))
                .transpose()?,
            })
        })
        .collect::<CmaResult<Vec<_>>>()?;
    Ok(CmaEventSnapshot {
        events,
        next_cursor: next_cursor(body)?,
        through_cursor: text(body, "through_cursor", MAX_CURSOR_BYTES),
        incarnation: text(body, "incarnation", 256),
    })
}

fn parse_transcript(body: &Value, agent_id: &str) -> CmaResult<CmaTranscript> {
    if body.get("agent_id").and_then(Value::as_str) != Some(agent_id) {
        return Err(CmaGatewayError::InvalidResponse);
    }
    let turns = body
        .get("turns")
        .and_then(Value::as_array)
        .ok_or(CmaGatewayError::InvalidResponse)?;
    let mut messages = Vec::new();
    let mut projected_turns = Vec::new();
    let mut seen = std::collections::HashMap::new();
    let mut last_sequence = None;
    let mut last_turn = None;
    let mut message_count = 0usize;
    if turns.len() > 10_000 {
        return Err(CmaGatewayError::InvalidResponse);
    }
    for turn in turns {
        let turn_sequence = turn
            .get("turn_sequence")
            .and_then(Value::as_i64)
            .filter(|sequence| *sequence > 0)
            .ok_or(CmaGatewayError::InvalidResponse)?;
        if last_turn.is_some_and(|previous| turn_sequence <= previous) {
            return Err(CmaGatewayError::InvalidResponse);
        }
        last_turn = Some(turn_sequence);
        let mut turn_messages = Vec::new();
        let mut turn_seen = std::collections::HashSet::new();
        let items = turn
            .get("messages")
            .and_then(Value::as_array)
            .ok_or(CmaGatewayError::InvalidResponse)?;
        for item in items {
            message_count += 1;
            if message_count > 10_000 {
                return Err(CmaGatewayError::InvalidResponse);
            }
            let sequence = item
                .get("sequence")
                .and_then(Value::as_i64)
                .filter(|sequence| *sequence >= 0)
                .ok_or(CmaGatewayError::InvalidResponse)?;
            let role = text(item, "role", 32)
                .filter(|role| matches!(role.as_str(), "user" | "assistant"))
                .ok_or(CmaGatewayError::InvalidResponse)?;
            let content = item
                .get("content")
                .filter(|content| content.is_string())
                .cloned()
                .ok_or(CmaGatewayError::InvalidResponse)?;
            let message = CmaTranscriptMessage {
                id: Some(format!("event:{sequence}")),
                sequence: Some(sequence),
                role,
                content,
                created_at: date(item, "appended_at"),
            };
            // CMA retries can reference the same admitted owner message in
            // both the failed and retried turns. Keep its first durable
            // occurrence, but never accept two contents for one sequence.
            if let Some(previous) = seen.get(&sequence) {
                if previous != &message {
                    return Err(CmaGatewayError::InvalidResponse);
                }
                if turn_seen.insert(sequence) {
                    turn_messages.push(message);
                }
                continue;
            }
            if last_sequence.is_some_and(|previous| sequence <= previous) {
                return Err(CmaGatewayError::InvalidResponse);
            }
            last_sequence = Some(sequence);
            seen.insert(sequence, message.clone());
            turn_seen.insert(sequence);
            turn_messages.push(message.clone());
            messages.push(message);
        }
        let status = turn
            .pointer("/outcome/type")
            .and_then(Value::as_str)
            .filter(|status| matches!(*status, "active" | "completed" | "failed" | "interrupted"))
            .unwrap_or("unknown");
        projected_turns.push(CmaTranscriptTurn {
            turn_sequence,
            started_at: date(turn, "started_at"),
            status: status.to_owned(),
            messages: turn_messages,
        });
    }
    Ok(CmaTranscript {
        messages,
        turns: projected_turns,
        next_cursor: None,
    })
}

fn parse_agent(body: &Value, owner_id: &str, agent_id: &str) -> CmaResult<CmaAgent> {
    let value = body.get("agent").unwrap_or(body);
    if id(value, "id")? != agent_id {
        return Err(CmaGatewayError::InvalidResponse);
    }
    let profile = value.get("profile").filter(|p| p.is_object()).map(|p| {
        json!({
            "model":text(p,"model",256),
            "permission_mode":text(p,"permission_level",120),
            "thinking_effort":text(p,"thinking_effort",120),
        })
    });
    let parent = |field: &str| -> CmaResult<Option<String>> {
        if let Some(value) = value.get(field).filter(|value| !value.is_null()) {
            return id(value, "id").map(Some);
        }
        let key = format!("{field}_id");
        value
            .get(&key)
            .filter(|value| !value.is_null())
            .map(|_| id(value, &key))
            .transpose()
    };
    Ok(CmaAgent {
        id: agent_id.to_owned(),
        owner_id: owner(value, owner_id)?,
        sandbox_id: parent("sandbox")?,
        workspace_id: parent("workspace")?,
        worktree_id: parent("worktree")?,
        name: text(value, "owner_title", 120).or_else(|| text(value, "title", 120)),
        status: text(value, "status", 120).unwrap_or_else(|| "unknown".into()),
        phase: text(value, "phase", 120),
        revision: value.get("revision").and_then(Value::as_i64),
        profile,
        current_response_id: text(value, "current_response_id", 256),
        desktop: value.get("desktop").and_then(Value::as_bool),
        sandbox_status: value
            .get("sandbox")
            .and_then(|sandbox| text(sandbox, "status", 120)),
        callback_url: value
            .get("callback_url")
            .map(|callback| super::cma_operation::validated_callback_url(callback.as_str())),
    })
}

fn parse_response(body: &Value, agent_id: &str, response_id: &str) -> CmaResult<CmaResponse> {
    let value = body.get("response").unwrap_or(body);
    let bound_agent = text(value, "agent_id", 256)
        .or_else(|| {
            value
                .get("conversation")
                .and_then(|conversation| text(conversation, "id", 256))
        })
        .or_else(|| value.get("agent").and_then(|value| text(value, "id", 256)))
        .ok_or(CmaGatewayError::InvalidResponse)?;
    if id(value, "id")? != response_id || bound_agent != agent_id {
        return Err(CmaGatewayError::Forbidden);
    }
    Ok(CmaResponse {
        id: response_id.to_owned(),
        agent_id: bound_agent,
        status: text(value, "status", 120).unwrap_or_else(|| "unknown".into()),
        output: value.get("output").cloned(),
    })
}

fn parse_desktop(body: &Value, agent_id: &str) -> CmaResult<CmaDesktopDescriptor> {
    validate_remote_id(agent_id)?;
    let expected = format!("/api/v1/agents/{agent_id}/desktop/ws");
    if body.get("websocket_url").and_then(Value::as_str) != Some(expected.as_str()) {
        return Err(CmaGatewayError::InvalidResponse);
    }
    Ok(CmaDesktopDescriptor {
        available: true,
        reason: None,
        shared: true,
        width: None,
        height: None,
        websocket_path: Some(expected),
    })
}

#[async_trait]
impl CmaGateway for CatalogCmaGateway {
    async fn service_id(&self, owner_id: &str) -> CmaResult<String> {
        Ok(self.resolve(owner_id).await?.0.id)
    }
    async fn verify_creation_deployment(
        &self,
        owner_id: &str,
        service_id: &str,
        fingerprint: &str,
    ) -> CmaResult<()> {
        let (service, _, _) = self.resolve(owner_id).await?;
        if service.id != service_id || catalog_execution_fingerprint(&service) != fingerprint {
            return Err(CmaGatewayError::ConfigurationRequired);
        }
        Ok(())
    }
    async fn get_controls(
        &self,
        owner_id: &str,
        agent_id: &str,
        response_id: &str,
    ) -> CmaResult<Value> {
        let (_, body) = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &[
                    "api",
                    "v2",
                    "agents",
                    agent_id,
                    "responses",
                    response_id,
                    "controls",
                ],
                None,
                None,
            )
            .await?;
        if body
            .pointer("/response/receipt/agent_id")
            .and_then(Value::as_str)
            != Some(agent_id)
            || body
                .pointer("/response/receipt/response_id")
                .and_then(Value::as_str)
                != Some(response_id)
            || body
                .pointer("/target/response_id")
                .is_some_and(|value| value.as_str() != Some(response_id))
        {
            return Err(CmaGatewayError::InvalidResponse);
        }
        Ok(body)
    }
    async fn get_lifecycle(&self, owner_id: &str, operation_id: &str) -> CmaResult<Value> {
        let (_, body) = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &["api", "v1", "agent-lifecycle", operation_id],
                None,
                None,
            )
            .await?;
        if body.get("id").and_then(Value::as_str) != Some(operation_id) {
            return Err(CmaGatewayError::InvalidResponse);
        }
        Ok(body)
    }
    async fn list_agents(
        &self,
        owner_id: &str,
        cursor: Option<&str>,
    ) -> CmaResult<CmaPage<CmaAgent>> {
        let (_, body) = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &["api", "v1", "agents"],
                cursor,
                None,
            )
            .await?;
        let items = page_items(&body, "agents")?;
        if items.len() > 8 {
            return Err(CmaGatewayError::InvalidResponse);
        }
        Ok(CmaPage {
            items: items
                .iter()
                .map(|value| parse_agent(value, owner_id, &id(value, "id")?))
                .collect::<CmaResult<Vec<_>>>()?,
            next_cursor: next_cursor(&body)?,
        })
    }
    async fn get_workspace_sandbox(&self, owner_id: &str, workspace_id: &str) -> CmaResult<String> {
        let (_, body) = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &["api", "v1", "workspaces", workspace_id],
                None,
                None,
            )
            .await?;
        let workspace = body.get("workspace").unwrap_or(&body);
        if id(workspace, "id")? != workspace_id {
            return Err(CmaGatewayError::InvalidResponse);
        }
        let sandbox_id = id(workspace, "sandbox_id")?;
        self.get_sandbox_authority(owner_id, &sandbox_id).await?;
        Ok(sandbox_id)
    }
    async fn write_agent_file(
        &self,
        owner_id: &str,
        agent_id: &str,
        path: &str,
        bytes: &[u8],
    ) -> CmaResult<CmaFileWrite> {
        use base64::Engine;
        if bytes.len() > 32 * 1024 * 1024
            || path.is_empty()
            || path.len() > 1024
            || path.starts_with('/')
            || path.contains('\\')
            || path.chars().any(char::is_control)
            || path.split('/').any(|part| matches!(part, "" | "." | ".."))
        {
            return Err(CmaGatewayError::InvalidRequest);
        }
        let expected_sha = hex::encode(Sha256::digest(bytes));
        let body=serde_json::to_vec(&json!({"path":path,"content_base64":base64::engine::general_purpose::STANDARD.encode(bytes),"directory":false}))
            .map_err(|_|CmaGatewayError::InvalidRequest)?;
        let (_, result) = self
            .request_body(
                owner_id,
                reqwest::Method::POST,
                &["api", "v1", "agents", agent_id, "files"],
                None,
                None,
                Some(body),
            )
            .await?;
        if result.get("path").and_then(Value::as_str) != Some(path)
            || result.get("size_bytes").and_then(Value::as_u64) != Some(bytes.len() as u64)
            || result.get("sha256").and_then(Value::as_str) != Some(expected_sha.as_str())
        {
            return Err(CmaGatewayError::InvalidResponse);
        }
        Ok(CmaFileWrite {
            path: path.into(),
            size_bytes: bytes.len() as u64,
            sha256: expected_sha,
        })
    }
    async fn get_sandbox(&self, owner_id: &str, sandbox_id: &str) -> CmaResult<CmaSandbox> {
        let (_, body) = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &["api", "v1", "sandboxes", sandbox_id],
                None,
                None,
            )
            .await?;
        let value = body.get("sandbox").unwrap_or(&body);
        let item = parse_exact_sandbox(value, owner_id, sandbox_id)?;
        let mut items = [(item, pinned_sandbox_profile(value))];
        hydrate_sandbox_capacity(
            &mut items,
            SANDBOX_CAPACITY_READ_BUDGET,
            |profile| self.pinned_sandbox_capacity(owner_id, profile),
            |sandbox| self.runtime_sandbox_capacity(owner_id, sandbox),
        )
        .await;
        let [(item, _)] = items;
        Ok(item)
    }
    async fn get_sandbox_authority(
        &self,
        owner_id: &str,
        sandbox_id: &str,
    ) -> CmaResult<CmaSandbox> {
        let (_, body) = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &["api", "v1", "sandboxes", sandbox_id],
                None,
                None,
            )
            .await?;
        parse_exact_sandbox(body.get("sandbox").unwrap_or(&body), owner_id, sandbox_id)
    }
    async fn pause_sandbox(
        &self,
        owner_id: &str,
        sandbox_id: &str,
        idempotency_key: &str,
    ) -> CmaResult<CmaLifecycleResult> {
        self.lifecycle(owner_id, sandbox_id, Some("pause"), idempotency_key)
            .await
    }
    async fn revive_sandbox(
        &self,
        owner_id: &str,
        sandbox_id: &str,
        idempotency_key: &str,
    ) -> CmaResult<CmaLifecycleResult> {
        self.lifecycle(owner_id, sandbox_id, Some("revive"), idempotency_key)
            .await
    }
    async fn retire_sandbox(
        &self,
        owner_id: &str,
        sandbox_id: &str,
        idempotency_key: &str,
    ) -> CmaResult<CmaLifecycleResult> {
        self.lifecycle(owner_id, sandbox_id, None, idempotency_key)
            .await
    }
    async fn list_sandbox_events(
        &self,
        owner_id: &str,
        sandbox_id: &str,
        cursor: Option<&str>,
    ) -> CmaResult<CmaEventSnapshot> {
        let (_, body) = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &["api", "v1", "sandboxes", sandbox_id, "events"],
                cursor,
                None,
            )
            .await?;
        let mut page = events(&body)?;
        if page.events.len() == MAX_PAGE_SIZE {
            page.next_cursor = page
                .events
                .last()
                .and_then(|event| event.sequence)
                .map(|seq| seq.to_string());
        }
        if body.as_array().is_some_and(|items| {
            items
                .iter()
                .any(|item| item.get("sandbox_id").and_then(Value::as_str) != Some(sandbox_id))
        }) {
            return Err(CmaGatewayError::InvalidResponse);
        }
        Ok(page)
    }
    async fn get_agent(&self, owner_id: &str, agent_id: &str) -> CmaResult<CmaAgent> {
        let (_, body) = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &["api", "v1", "agents", agent_id],
                None,
                None,
            )
            .await?;
        parse_agent(&body, owner_id, agent_id)
    }
    async fn get_transcript(
        &self,
        owner_id: &str,
        agent_id: &str,
        cursor: Option<&str>,
    ) -> CmaResult<CmaTranscript> {
        // CMA transcript is a bounded full projection, not a paginated API.
        if cursor.is_some() {
            return Err(CmaGatewayError::InvalidRequest);
        }
        let (_, body) = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &["api", "v1", "agents", agent_id, "transcript"],
                cursor,
                None,
            )
            .await?;
        parse_transcript(&body, agent_id)
    }
    async fn get_event_snapshot(
        &self,
        owner_id: &str,
        agent_id: &str,
        cursor: Option<&str>,
    ) -> CmaResult<CmaEventSnapshot> {
        let (_, body) = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &["api", "v2", "agents", agent_id, "events", "snapshot"],
                cursor,
                None,
            )
            .await?;
        if body.get("agent_id").and_then(Value::as_str) != Some(agent_id) {
            return Err(CmaGatewayError::InvalidResponse);
        }
        events(&body)
    }
    async fn get_response(
        &self,
        owner_id: &str,
        agent_id: &str,
        response_id: &str,
    ) -> CmaResult<CmaResponse> {
        validate_remote_id(agent_id)?;
        let (_, body) = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &["api", "v2", "responses", response_id],
                None,
                None,
            )
            .await?;
        parse_response(&body, agent_id, response_id)
    }
    async fn desktop_descriptor(
        &self,
        owner_id: &str,
        agent_id: &str,
    ) -> CmaResult<CmaDesktopDescriptor> {
        let response = self
            .request(
                owner_id,
                reqwest::Method::GET,
                &["api", "v1", "agents", agent_id, "desktop"],
                None,
                None,
            )
            .await;
        let body = match response {
            Ok((_, body)) => body,
            Err(CmaGatewayError::DesktopUnavailable { reason }) => {
                return Ok(CmaDesktopDescriptor {
                    available: false,
                    reason: Some(reason.into()),
                    shared: true,
                    width: None,
                    height: None,
                    websocket_path: None,
                });
            }
            Err(error) => return Err(error),
        };
        parse_desktop(&body, agent_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_identity_flags_do_not_define_cma_deployment() {
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        catalog.identity_propagation_mode = "jwt".into();
        catalog.identity_include_user_id = false;
        catalog.identity_include_email = false;
        catalog.identity_include_name = false;
        let mut connection = catalog.clone();
        connection.identity_include_user_id = true;
        connection.identity_include_email = true;
        connection.identity_include_name = true;
        assert_ne!(
            catalog_execution_fingerprint(&catalog),
            catalog_execution_fingerprint(&connection)
        );
        assert!(
            require_same_deployment_target(&catalog, &connection.id, &connection.base_url).is_ok()
        );
        assert!(
            require_same_deployment_target(&catalog, "another-service", &connection.base_url)
                .is_err()
        );
        assert!(
            require_same_deployment_target(
                &catalog,
                &connection.id,
                "https://another-deployment.example"
            )
            .is_err()
        );
    }
    #[test]
    fn sandbox_authority_projection_keeps_exact_identity_without_profile_enrichment() {
        let body = json!({
            "id":"sandbox", "owner":"owner", "status":"active",
            "profile":{"profile_id":"internal-profile", "revision":7},
        });
        let observed = parse_exact_sandbox(&body, "owner", "sandbox").unwrap();
        assert_eq!(observed.id, "sandbox");
        assert_eq!(observed.owner_id.as_deref(), Some("owner"));
        assert_eq!(observed.state, SandboxObservationState::Ready);
        assert_eq!((observed.cpu_millis, observed.memory_mib), (None, None));
        assert!(parse_exact_sandbox(&body, "other", "sandbox").is_err());
        assert!(parse_exact_sandbox(&body, "owner", "other").is_err());
        let mut missing_owner = body;
        missing_owner.as_object_mut().unwrap().remove("owner");
        assert!(parse_exact_sandbox(&missing_owner, "owner", "sandbox").is_err());
    }

    #[test]
    fn bounded_paths_reject_destination_and_encoded_traversal() {
        let base = validated_base("https://cma.example/prefix/").unwrap();
        assert_eq!(
            endpoint_url(
                &base,
                &["api", "v1", "sandboxes", "sb_1"],
                Some("opaque&cursor")
            )
            .unwrap()
            .as_str(),
            "https://cma.example/prefix/api/v1/sandboxes/sb_1?cursor=opaque%26cursor"
        );
        for id in [
            "..",
            "x/y",
            "%2e%2e",
            "x?y",
            "x#y",
            "https://other.example",
            "x\\y",
        ] {
            assert!(endpoint_url(&base, &["api", id], None).is_err());
        }
        for raw in [
            "https://user:secret@cma.example",
            "file:///tmp/cma",
            "http://cma.example",
            "https://cma.example?secret=x",
        ] {
            assert!(validated_base(raw).is_err());
        }
    }
    #[test]
    fn credential_setup_link_comes_only_from_the_catalog_base() {
        let base = validated_base("https://cma.example/base").unwrap();
        let error = map_status(
            409,
            Some(
                &json!({"error":"sandbox_credential_setup_required","login_url":"https://evil.example","message":"secret"}),
            ),
            &base,
        );
        assert_eq!(
            error,
            CmaGatewayError::CredentialSetupRequired {
                login_url: "https://cma.example/base/auth/login".into()
            }
        );
        assert!(!AppError::from(error).to_string().contains("secret"));
    }
    #[test]
    fn unknown_and_failed_states_do_not_imply_retirement() {
        for state in ["deleted", "not_found", "failed_cleanup", "error", "unknown"] {
            assert_ne!(observation_state(state), SandboxObservationState::Retired);
        }
        assert_eq!(
            observation_state("retired"),
            SandboxObservationState::Retired
        );
        assert!(parse_sandbox(&json!({"id":"sandbox","owner_id":"other"}), "owner").is_err());
        assert!(next_cursor(&json!({"has_more":true})).is_err());
    }

    #[test]
    fn cma_sandbox_contract_preserves_owner_pagination_and_transient_status() {
        let page = json!({"items":[{"id":"sb_one","owner":"owner","status":"pausing"}],"next":"opaque-next"});
        let sandbox = parse_sandbox(&page_items(&page, "sandboxes").unwrap()[0], "owner").unwrap();
        assert_eq!(sandbox.owner_id.as_deref(), Some("owner"));
        assert_eq!(sandbox.resource_state.as_deref(), Some("pausing"));
        assert_eq!(sandbox.state, SandboxObservationState::Unknown);
        assert_eq!(sandbox.machine_included, None);
        assert_eq!(sandbox.cpu_millis, None);
        assert_eq!(next_cursor(&page).unwrap().as_deref(), Some("opaque-next"));
        assert!(parse_sandbox(&json!({"id":"sb_one","status":"ready"}), "owner").is_err());
        assert!(parse_sandbox(&page["items"][0], "other").is_err());
    }

    #[test]
    fn cma_agent_contract_reads_nested_parents_and_public_profile() {
        let body = json!({
            "id":"agent_1", "status":"ready", "desktop":true,
            "owner_title":"My execution", "title":"generated title",
            "sandbox":{"id":"sandbox_1","status":"paused"},
            "workspace":{"id":"workspace_1"}, "worktree":{"id":"worktree_1"},
            "profile":{"revision":12,"permission_level":"guarded","model":"model_1","thinking_effort":"high","secret":"not projected"}
        });
        let agent = parse_agent(&body, "owner", "agent_1").unwrap();
        assert_eq!(agent.sandbox_id.as_deref(), Some("sandbox_1"));
        assert_eq!(agent.workspace_id.as_deref(), Some("workspace_1"));
        assert_eq!(agent.worktree_id.as_deref(), Some("worktree_1"));
        assert_eq!(agent.name.as_deref(), Some("My execution"));
        assert_eq!(agent.desktop, Some(true));
        assert_eq!(agent.sandbox_status.as_deref(), Some("paused"));
        assert_eq!(agent.revision, None);
        assert_eq!(
            agent.profile.as_ref().unwrap()["permission_mode"],
            "guarded"
        );
        assert!(agent.profile.as_ref().unwrap().get("secret").is_none());
        assert!(parse_agent(&body, "owner", "agent_2").is_err());
    }

    #[test]
    fn cma_agent_callback_preserves_the_observed_safe_url_and_identity() {
        for callback in [
            "https://CMA.example:8443/agents/agent_1",
            "http://localhost:8080/agents/agent_1",
        ] {
            let body = json!({"agent":{
                "id":"agent_1","owner":"owner","status":"working","revision":7,
                "sandbox":{"id":"sandbox_1"},"workspace":{"id":"workspace_1"},
                "worktree":{"id":"worktree_1"},"callback_url":callback
            }});
            let agent = parse_agent(&body, "owner", "agent_1").unwrap();
            let projected = serde_json::to_value(&agent).unwrap();
            assert_eq!(projected.get("callback_url"), Some(&json!(callback)));
            assert_eq!(agent.status, "working");
            assert_eq!(agent.revision, Some(7));
            assert_eq!(agent.owner_id.as_deref(), Some("owner"));
            assert_eq!(agent.sandbox_id.as_deref(), Some("sandbox_1"));
            assert_eq!(agent.workspace_id.as_deref(), Some("workspace_1"));
            assert_eq!(agent.worktree_id.as_deref(), Some("worktree_1"));
            assert!(parse_agent(&body, "other-owner", "agent_1").is_err());
            assert!(parse_agent(&body, "owner", "different-agent").is_err());
        }
    }

    #[test]
    fn cma_agent_callback_omission_does_not_fabricate_a_destination() {
        let body = json!({
            "callback_url":"https://unrelated.example/agents/other",
            "agent":{"id":"agent_1","owner":"owner","status":"ready"}
        });
        let agent = parse_agent(&body, "owner", "agent_1").unwrap();
        let projected = serde_json::to_value(agent).unwrap();
        assert!(projected.get("callback_url").is_none());
        assert_eq!(projected["status"], "ready");
    }

    #[test]
    fn cma_agent_callback_null_and_invalid_values_clear_without_losing_status() {
        for callback in [
            json!(null),
            json!(123),
            json!(true),
            json!({"url":"https://cma.example/agents/agent_1"}),
            json!(["https://cma.example/agents/agent_1"]),
            json!(""),
            json!("/agents/agent_1"),
            json!("javascript:alert(1)"),
            json!("data:text/html,agent"),
            json!("https://user:secret@cma.example/agents/agent_1"),
            json!("https://cma.example/agents/agent_1?token=secret"),
            json!("https://cma.example/agents/agent_1#token"),
            json!("http://169.254.169.254/latest/meta-data"),
            json!("http://metadata.google.internal/agents/agent_1"),
            json!("https://cma.example/agents/agent_1\n"),
            json!("https://cma.example/agents/agent_1\u{0000}"),
            json!("https://cma.example/agents/agent 1"),
            json!("https://cma.example\\agents\\agent_1"),
            json!(format!("https://cma.example/{}", "a".repeat(2048))),
        ] {
            let body = json!({
                "id":"agent_1","owner":"owner","status":"failed",
                "callback_url":callback
            });
            let agent = parse_agent(&body, "owner", "agent_1").unwrap();
            let projected = serde_json::to_value(agent).unwrap();
            assert_eq!(
                projected.get("callback_url"),
                Some(&Value::Null),
                "invalid or withdrawn callback must remain an explicit clear: {callback}"
            );
            assert_eq!(projected["status"], "failed");
            assert_eq!(projected["id"], "agent_1");
        }
    }

    fn sandbox_with_profile(id: &str, profile: &str, revision: Value) -> Value {
        json!({
            "id":id,"owner":"owner","name":"Configured environment","status":"active",
            "profile":{"id":profile,"revision":revision},"nyxid_services":[],
            "limits":{"max_workspaces":4,"max_agents_per_workspace":8},
            "lifecycle":{},"last_activity_at":"2026-11-04T00:00:00Z","revision":19
        })
    }

    #[tokio::test]
    async fn real_sandbox_capacity_uses_its_exact_pinned_profile_and_reuses_page_reads() {
        let sandbox = sandbox_with_profile("sbx_1", "sbp_1", json!(7));
        let second = sandbox_with_profile("sbx_2", "sbp_1", json!(7));
        let revision = json!({
            "profile_id":"sbp_1","revision":7,"digest":"pinned-digest","published_at":"2026-11-01T00:00:00Z",
            "content":{"cpu_millis":2000,"memory_mib":4096,"agent_provider":"codex","model_access":"nyxid",
                "max_workspaces":4,"max_agents_per_workspace":8,"nyxid_services":[],"workspace_profiles":[],
                "environment_variables":[{"name":"INTERNAL_ONLY","value":"not-projected"}],
                "software_install_commands":["not-projected"]}
        });
        let mut items = [sandbox, second]
            .iter()
            .map(|value| {
                (
                    parse_sandbox(value, "owner").unwrap(),
                    pinned_sandbox_profile(value),
                )
            })
            .collect::<Vec<_>>();
        assert!(
            items
                .iter()
                .all(|(sandbox, _)| sandbox.cpu_millis.is_none() && sandbox.memory_mib.is_none())
        );
        let mut requests = Vec::new();
        hydrate_sandbox_capacity(
            &mut items,
            Duration::from_secs(1),
            |profile| {
                requests.push(profile.clone());
                std::future::ready(parse_sandbox_capacity(&profile, &revision))
            },
            |_| std::future::ready(Err(CmaGatewayError::Unavailable)),
        )
        .await;
        assert_eq!(
            requests,
            vec![PinnedSandboxProfile {
                id: "sbp_1".into(),
                revision: 7
            }]
        );
        for (sandbox, _) in &items {
            assert_eq!(sandbox.cpu_millis, Some(2000));
            assert_eq!(sandbox.memory_mib, Some(4096));
            let safe = serde_json::to_string(sandbox).unwrap();
            assert!(!safe.contains("INTERNAL_ONLY"));
            assert!(!safe.contains("not-projected"));
        }
        let pin = requests.remove(0);
        let mut latest = revision.clone();
        latest["revision"] = json!(8);
        assert!(parse_sandbox_capacity(&pin, &latest).is_err());
        latest["revision"] = json!(7);
        latest["profile_id"] = json!("sbp_other");
        assert!(parse_sandbox_capacity(&pin, &latest).is_err());
    }

    #[test]
    fn malformed_profile_pins_never_select_latest_or_inline_capacity() {
        for revision in [
            Value::Null,
            json!("latest"),
            json!("7"),
            json!(-1),
            json!(u64::from(u32::MAX) + 1),
        ] {
            let mut sandbox = sandbox_with_profile("sbx_1", "sbp_1", revision);
            sandbox["resources"] = json!({"cpu_millis":9999,"memory_mib":9999});
            assert!(pinned_sandbox_profile(&sandbox).is_none());
            let parsed = parse_sandbox(&sandbox, "owner").unwrap();
            assert!(parsed.cpu_millis.is_none() && parsed.memory_mib.is_none());
        }
        let mut sandbox = sandbox_with_profile("sbx_1", "sbp_1", json!(7));
        sandbox["profile"]["id"] = json!("../latest");
        assert!(pinned_sandbox_profile(&sandbox).is_none());
    }

    #[test]
    fn unproven_inline_capacity_remains_unknown() {
        for capacity in [
            json!({"cpu_millis":2000,"memory_mib":8192}),
            json!({"resources":{"cpu_millis":2000,"memory_mib":8192}}),
        ] {
            let mut body = json!({"id":"sbx_1","owner":"owner","status":"active"});
            body.as_object_mut()
                .unwrap()
                .extend(capacity.as_object().unwrap().clone());
            let parsed = parse_sandbox(&body, "owner").unwrap();
            assert_eq!((parsed.cpu_millis, parsed.memory_mib), (None, None));
        }
    }

    fn allocation_body(id: &str) -> Value {
        json!({
            "sandbox_id":id,"owner":"owner","status":"active","as_of":1793791925u64,
            "allocation":{"running":true,"cpu_millis":2000,"memory_mib":8192,"disk_gib":64}
        })
    }

    #[test]
    fn sandbox_allocation_parser_requires_exact_identity_and_active_state() {
        let expected =
            parse_sandbox(&sandbox_with_profile("sbx_1", "sbp_1", json!(7)), "owner").unwrap();
        let body = allocation_body("sbx_1");
        let capacity = parse_sandbox_allocation(&expected, &body).unwrap();
        assert_eq!((capacity.cpu_millis, capacity.memory_mib), (2000, 8192));
        assert_eq!(capacity.source, SandboxCapacitySource::RuntimeAllocation);
        for (path, value) in [
            ("/sandbox_id", json!("sbx_other")),
            ("/sandbox_id", Value::Null),
            ("/owner", json!("other")),
            ("/owner", Value::Null),
            ("/status", json!("ready")),
            ("/status", Value::Null),
        ] {
            let mut substituted = body.clone();
            *substituted.pointer_mut(path).unwrap() = value;
            assert!(
                parse_sandbox_allocation(&expected, &substituted).is_err(),
                "accepted {path}"
            );
        }
        let mut owner_alias = body;
        owner_alias.as_object_mut().unwrap().remove("owner");
        owner_alias["owner_id"] = json!("owner");
        assert!(parse_sandbox_allocation(&expected, &owner_alias).is_err());
    }

    #[test]
    fn sandbox_allocation_parser_rejects_paused_zero_and_malformed_values() {
        let mut expected =
            parse_sandbox(&sandbox_with_profile("sbx_1", "sbp_1", json!(7)), "owner").unwrap();
        let body = allocation_body("sbx_1");
        for (path, value) in [
            ("/status", json!("paused")),
            ("/status", json!("retired")),
            ("/allocation/running", json!(false)),
            ("/allocation/running", Value::Null),
            ("/allocation/cpu_millis", json!(0)),
            ("/allocation/memory_mib", json!(0)),
            ("/allocation/cpu_millis", json!(-1)),
            ("/allocation/memory_mib", json!("8192")),
            ("/allocation/cpu_millis", json!(u64::from(u32::MAX) + 1)),
            ("/allocation/memory_mib", Value::Null),
            ("/allocation", Value::Null),
        ] {
            let mut malformed = body.clone();
            *malformed.pointer_mut(path).unwrap() = value;
            assert!(
                parse_sandbox_allocation(&expected, &malformed).is_err(),
                "accepted {path}"
            );
        }
        expected.state = SandboxObservationState::Paused;
        assert!(parse_sandbox_allocation(&expected, &body).is_err());
        expected.state = SandboxObservationState::Ready;
        expected.resource_state = Some("pausing".into());
        assert!(parse_sandbox_allocation(&expected, &body).is_err());
        expected.resource_state = Some("active".into());
        expected.owner_id = None;
        assert!(parse_sandbox_allocation(&expected, &body).is_err());
    }

    #[tokio::test]
    async fn sandbox_allocation_fallback_preserves_profile_priority_and_exact_owner() {
        let mut items = [
            ("sbx_profile", "sbp_profile"),
            ("sbx_fallback", "sbp_internal"),
            ("sbx_other", "sbp_internal"),
        ]
        .into_iter()
        .map(|(id, pin)| {
            let body = sandbox_with_profile(id, pin, json!(7));
            (
                parse_sandbox(&body, "owner").unwrap(),
                pinned_sandbox_profile(&body),
            )
        })
        .collect::<Vec<_>>();
        // A verified immutable profile remains useful while the runtime is paused.
        items[0].0.state = SandboxObservationState::Paused;
        items[0].0.resource_state = Some("paused".into());
        let mut profiles = Vec::new();
        let mut allocations = Vec::new();
        hydrate_sandbox_capacity(&mut items, Duration::from_secs(1), |pin| {
            profiles.push(pin.id.clone());
            std::future::ready(if pin.id == "sbp_profile" {
                parse_sandbox_capacity(&pin, &json!({"profile_id":pin.id,"revision":7,"content":{"cpu_millis":1000,"memory_mib":4096}}))
            } else { Err(CmaGatewayError::NotFound) })
        }, |sandbox| {
            allocations.push((sandbox.id.clone(), sandbox.owner_id.clone()));
            std::future::ready(parse_sandbox_allocation(&sandbox, &allocation_body(&sandbox.id)))
        }).await;
        assert_eq!(profiles, ["sbp_profile", "sbp_internal"]);
        assert_eq!(
            allocations,
            [
                ("sbx_fallback".into(), Some("owner".into())),
                ("sbx_other".into(), Some("owner".into()))
            ]
        );
        assert_eq!(
            items[0].0.capacity_source,
            Some(SandboxCapacitySource::ProfileRevision)
        );
        assert_eq!(
            (items[0].0.cpu_millis, items[0].0.memory_mib),
            (Some(1000), Some(4096))
        );
        for (sandbox, _) in &items[1..] {
            assert_eq!(
                sandbox.capacity_source,
                Some(SandboxCapacitySource::RuntimeAllocation)
            );
            assert_eq!(
                (sandbox.cpu_millis, sandbox.memory_mib),
                (Some(2000), Some(8192))
            );
        }
    }

    #[tokio::test]
    async fn sandbox_allocation_failures_keep_inventory_usable_and_inactive_capacity_unknown() {
        for error in [
            CmaGatewayError::Forbidden,
            CmaGatewayError::UpstreamAuthorizationUnavailable,
            CmaGatewayError::NotFound,
            CmaGatewayError::Timeout,
            CmaGatewayError::InvalidResponse,
        ] {
            let mut items = ["active", "paused", "retired", "pausing"]
                .into_iter()
                .map(|status| {
                    let mut body =
                        sandbox_with_profile(&format!("sbx_{status}"), "sbp_internal", json!(7));
                    body["status"] = json!(status);
                    (
                        parse_sandbox(&body, "owner").unwrap(),
                        pinned_sandbox_profile(&body),
                    )
                })
                .collect::<Vec<_>>();
            let mut reads = 0;
            hydrate_sandbox_capacity(
                &mut items,
                Duration::from_secs(1),
                |_| std::future::ready(Err(CmaGatewayError::NotFound)),
                |_| {
                    reads += 1;
                    std::future::ready(Err(error.clone()))
                },
            )
            .await;
            assert_eq!(reads, 1);
            assert!(items.iter().all(|(sandbox, _)| sandbox.cpu_millis.is_none()
                && sandbox.memory_mib.is_none()
                && sandbox.capacity_source.is_none()));
            assert_eq!(items[0].0.state, SandboxObservationState::Ready);
            assert_eq!(items[1].0.state, SandboxObservationState::Paused);
            assert_eq!(items[2].0.state, SandboxObservationState::Retired);
        }
    }

    #[tokio::test]
    async fn sandbox_allocation_fallback_shares_the_total_read_and_time_budget() {
        let mut items = (0..MAX_PAGE_SIZE)
            .map(|index| {
                let body = sandbox_with_profile(&format!("sbx_{index}"), "sbp_internal", json!(7));
                (
                    parse_sandbox(&body, "owner").unwrap(),
                    pinned_sandbox_profile(&body),
                )
            })
            .collect::<Vec<_>>();
        let reads = std::cell::Cell::new(0);
        hydrate_sandbox_capacity(
            &mut items,
            Duration::from_secs(1),
            |_| {
                reads.set(reads.get() + 1);
                std::future::ready(Err(CmaGatewayError::NotFound))
            },
            |sandbox| {
                reads.set(reads.get() + 1);
                std::future::ready(parse_sandbox_allocation(
                    &sandbox,
                    &allocation_body(&sandbox.id),
                ))
            },
        )
        .await;
        assert_eq!(reads.get(), MAX_SANDBOX_CAPACITY_READS);
        assert_eq!(
            items
                .iter()
                .filter(|(sandbox, _)| sandbox.capacity_source
                    == Some(SandboxCapacitySource::RuntimeAllocation))
                .count(),
            MAX_SANDBOX_CAPACITY_READS - 1
        );
        let calls = std::cell::Cell::new(0);
        tokio::time::timeout(
            Duration::from_secs(1),
            hydrate_sandbox_capacity(
                &mut items,
                Duration::from_millis(10),
                |_| {
                    calls.set(calls.get() + 1);
                    std::future::ready(Err(CmaGatewayError::NotFound))
                },
                |_| {
                    calls.set(calls.get() + 1);
                    std::future::pending::<CmaResult<SandboxCapacity>>()
                },
            ),
        )
        .await
        .unwrap();
        assert_eq!(calls.get(), 2);
        assert!(items.iter().all(|(sandbox, _)| sandbox.cpu_millis.is_none()
            && sandbox.memory_mib.is_none()
            && sandbox.capacity_source.is_none()));
    }

    #[tokio::test]
    async fn optional_profile_failures_keep_inventory_usable_and_capacity_unknown() {
        for error in [
            CmaGatewayError::Forbidden,
            CmaGatewayError::UpstreamAuthorizationUnavailable,
            CmaGatewayError::NotFound,
            CmaGatewayError::Timeout,
            CmaGatewayError::InvalidResponse,
        ] {
            let mut items = (0..2)
                .map(|index| {
                    let value = sandbox_with_profile(&format!("sbx_{index}"), "sbp_1", json!(7));
                    (
                        parse_sandbox(&value, "owner").unwrap(),
                        pinned_sandbox_profile(&value),
                    )
                })
                .collect::<Vec<_>>();
            let mut calls = 0;
            hydrate_sandbox_capacity(
                &mut items,
                Duration::from_secs(1),
                |_| {
                    calls += 1;
                    std::future::ready(Err(error.clone()))
                },
                |_| std::future::ready(Err(CmaGatewayError::Unavailable)),
            )
            .await;
            assert_eq!(calls, 1);
            assert!(items.iter().all(|(sandbox, _)| sandbox.cpu_millis.is_none()
                && sandbox.memory_mib.is_none()
                && sandbox.state == SandboxObservationState::Ready));
        }
    }

    #[tokio::test]
    async fn optional_profile_reads_have_a_page_count_and_total_time_bound() {
        let mut items = (0..MAX_PAGE_SIZE)
            .map(|index| {
                let value = sandbox_with_profile(
                    &format!("sbx_{index}"),
                    &format!("sbp_{index}"),
                    json!(7),
                );
                (
                    parse_sandbox(&value, "owner").unwrap(),
                    pinned_sandbox_profile(&value),
                )
            })
            .collect::<Vec<_>>();
        let mut calls = 0;
        hydrate_sandbox_capacity(
            &mut items,
            Duration::from_secs(1),
            |_| {
                calls += 1;
                std::future::ready(Ok(SandboxCapacity {
                    cpu_millis: 1000,
                    memory_mib: 1024,
                    source: SandboxCapacitySource::ProfileRevision,
                }))
            },
            |_| std::future::ready(Err(CmaGatewayError::Unavailable)),
        )
        .await;
        assert_eq!(calls, MAX_SANDBOX_CAPACITY_READS);
        assert_eq!(
            items
                .iter()
                .filter(|(sandbox, _)| sandbox.cpu_millis.is_some())
                .count(),
            MAX_SANDBOX_CAPACITY_READS
        );
        let mut calls = 0;
        tokio::time::timeout(
            Duration::from_secs(1),
            hydrate_sandbox_capacity(
                &mut items,
                Duration::from_millis(10),
                |_| {
                    calls += 1;
                    std::future::pending::<CmaResult<SandboxCapacity>>()
                },
                |_| std::future::ready(Err(CmaGatewayError::Unavailable)),
            ),
        )
        .await
        .unwrap();
        assert_eq!(calls, 1);
        assert!(
            items
                .iter()
                .all(|(sandbox, _)| sandbox.cpu_millis.is_none() && sandbox.memory_mib.is_none())
        );
    }

    #[test]
    fn cma_transcript_contract_flattens_turns_without_changing_message_order() {
        let mut body = json!({"agent_id":"agent_1","through_seq":7,"turns":[
            {"turn_sequence":1,"messages":[
                {"sequence":2,"role":"user","content":"hello","appended_at":"2026-11-04T00:00:00Z"},
                {"sequence":5,"role":"assistant","content":"first"}
            ]},
            {"turn_sequence":2,"messages":[{"sequence":7,"role":"assistant","content":"second"}]}
        ]});
        let transcript = parse_transcript(&body, "agent_1").unwrap();
        assert_eq!(
            transcript
                .messages
                .iter()
                .map(|item| item.sequence.unwrap())
                .collect::<Vec<_>>(),
            vec![2, 5, 7]
        );
        assert_eq!(transcript.messages[2].id.as_deref(), Some("event:7"));
        assert!(transcript.next_cursor.is_none());
        assert!(parse_transcript(&body, "other").is_err());
        body["turns"][1]["messages"][0]["sequence"] = json!(5);
        assert!(parse_transcript(&body, "agent_1").is_err());
    }

    #[test]
    fn cma_transcript_preserves_authoritative_turns_and_outcomes() {
        let body = json!({"agent_id":"agent_1","turns":[
            {"turn_sequence":10,"started_at":"2026-11-05T09:00:00Z","outcome":{"type":"completed"},"messages":[
                {"sequence":11,"role":"user","content":"first request"},
                {"sequence":12,"role":"assistant","content":"first result"}
            ]},
            {"turn_sequence":20,"outcome":{"type":"active"},"messages":[
                {"sequence":21,"role":"user","content":"second request"}
            ]}
        ]});
        let value = serde_json::to_value(parse_transcript(&body, "agent_1").unwrap()).unwrap();
        assert_eq!(value["turns"][0]["turn_sequence"], 10);
        assert_eq!(value["turns"][0]["status"], "completed");
        assert_eq!(value["turns"][0]["started_at"], "2026-11-05T09:00:00Z");
        assert_eq!(value["turns"][1]["status"], "active");
        assert_eq!(
            value["turns"][1]["messages"][0]["content"],
            "second request"
        );
    }

    #[test]
    fn cma_transcript_reused_request_remains_in_each_turn() {
        let message = json!({"sequence":11,"role":"user","content":"same admitted input"});
        let body = json!({"agent_id":"agent_1","turns":[
            {"turn_sequence":10,"outcome":{"type":"failed"},"messages":[message.clone()]},
            {"turn_sequence":20,"outcome":{"type":"completed"},"messages":[message]}
        ]});
        let value = serde_json::to_value(parse_transcript(&body, "agent_1").unwrap()).unwrap();
        assert_eq!(value["messages"].as_array().unwrap().len(), 1);
        assert_eq!(value["turns"][1]["messages"][0]["sequence"], 11);
    }

    #[test]
    fn cma_transcript_rejects_ambiguous_turn_identities() {
        for sequence in [0, 10, 9] {
            let body = json!({"agent_id":"agent_1","turns":[
                {"turn_sequence":10,"messages":[]},
                {"turn_sequence":sequence,"messages":[]}
            ]});
            assert!(parse_transcript(&body, "agent_1").is_err());
        }
    }

    #[test]
    fn cma_transcript_does_not_guess_unrecognized_outcomes() {
        let body = json!({"agent_id":"agent_1","turns":[
            {"turn_sequence":10,"outcome":{"type":"future_status"},"messages":[]}
        ]});
        let value = serde_json::to_value(parse_transcript(&body, "agent_1").unwrap()).unwrap();
        assert_eq!(value["turns"][0]["status"], "unknown");
    }

    #[test]
    fn cma_retried_turn_can_reuse_its_original_admitted_owner_message() {
        let message = json!({"sequence":3,"role":"user","content":"queued input","appended_at":"2026-11-04T00:00:00Z"});
        let mut body = json!({"agent_id":"agent_1","through_seq":13,"turns":[
            {"turn_sequence":1,"messages":[message.clone()]},
            {"turn_sequence":2,"messages":[message,{"sequence":13,"role":"assistant","content":"done"}]}
        ]});
        let transcript = parse_transcript(&body, "agent_1").unwrap();
        assert_eq!(
            transcript
                .messages
                .iter()
                .map(|message| message.sequence)
                .collect::<Vec<_>>(),
            vec![Some(3), Some(13)]
        );
        body["turns"][1]["messages"][0]["content"] = json!("substituted input");
        assert!(parse_transcript(&body, "agent_1").is_err());
    }

    #[test]
    fn cma_agent_inventory_uses_flat_parent_identifiers() {
        let agent = parse_agent(&json!({"id":"agent_1","status":"ready","sandbox_id":"sandbox_1","workspace_id":"workspace_1","worktree_id":"worktree_1"}), "owner", "agent_1").unwrap();
        assert_eq!(agent.sandbox_id.as_deref(), Some("sandbox_1"));
        assert_eq!(agent.workspace_id.as_deref(), Some("workspace_1"));
        assert_eq!(agent.worktree_id.as_deref(), Some("worktree_1"));
    }

    #[test]
    fn cma_event_contract_keeps_snapshot_page_and_stream_cursors_distinct() {
        let body = json!({
            "agent_id":"agent_1", "incarnation":"inc_1", "through_cursor":"head_20",
            "next_page_cursor":"page_10_of_20", "items":[{
                "cursor":"event_10","sequence":10,"appended_at":"2026-11-04T00:00:00Z",
                "kind":{"type":"message","role":"assistant","content":"hello"},
                "response_binding":{"response_id":"response_1","turn_sequence":3,"command_id":"command_1"}
            }]
        });
        let snapshot = events(&body).unwrap();
        assert_eq!(snapshot.next_cursor.as_deref(), Some("page_10_of_20"));
        assert_eq!(snapshot.through_cursor.as_deref(), Some("head_20"));
        assert_eq!(snapshot.events[0].cursor.as_deref(), Some("event_10"));
        assert_eq!(snapshot.events[0].message_sequence, Some(10));
        assert_eq!(
            snapshot.events[0].response_id.as_deref(),
            Some("response_1")
        );
        assert_eq!(snapshot.events[0].turn_sequence, Some(3));
        let base = validated_base("https://cma.example").unwrap();
        let snapshot_url = endpoint_url(
            &base,
            &["api", "v2", "agents", "agent_1", "events", "snapshot"],
            Some("page_10_of_20"),
        )
        .unwrap();
        assert_eq!(
            snapshot_url.query(),
            Some("page_cursor=page_10_of_20&limit=100")
        );
        let sandbox_url = endpoint_url(
            &base,
            &["api", "v1", "sandboxes", "sb_one", "events"],
            Some("12"),
        )
        .unwrap();
        assert_eq!(sandbox_url.query(), Some("after=12&limit=100"));
        assert!(
            endpoint_url(
                &base,
                &["api", "v1", "sandboxes", "sb_one", "events"],
                Some("opaque")
            )
            .is_err()
        );
    }

    #[test]
    fn cma_response_and_desktop_contracts_bind_exact_agent() {
        let response = json!({"id":"response_1","conversation":{"id":"agent_1"},"status":"completed","output":[]});
        assert_eq!(
            parse_response(&response, "agent_1", "response_1")
                .unwrap()
                .agent_id,
            "agent_1"
        );
        assert!(parse_response(&response, "agent_2", "response_1").is_err());
        assert!(parse_response(&response, "agent_1", "response_2").is_err());
        let descriptor = parse_desktop(
            &json!({"websocket_url":"/api/v1/agents/agent_1/desktop/ws"}),
            "agent_1",
        )
        .unwrap();
        assert!(descriptor.available && descriptor.shared);
        assert_eq!(
            descriptor.websocket_path.as_deref(),
            Some("/api/v1/agents/agent_1/desktop/ws")
        );
        for url in [
            "wss://other.example/ws",
            "/api/v1/agents/agent_2/desktop/ws",
            "/api/v1/agents/agent_1/desktop/ws?token=secret",
        ] {
            assert!(parse_desktop(&json!({"websocket_url":url}), "agent_1").is_err());
        }
    }

    #[test]
    fn execution_binding_detects_same_catalog_row_retargeting() {
        let original = crate::test_utils::test_auto_connected_catalog_service();
        let digest = catalog_execution_fingerprint(&original);
        let mut changed = original.clone();
        changed.base_url = "https://another-deployment.example".into();
        assert_ne!(digest, catalog_execution_fingerprint(&changed));
        changed = original.clone();
        changed.identity_jwt_audience = Some("other-audience".into());
        assert_ne!(digest, catalog_execution_fingerprint(&changed));
        changed = original.clone();
        changed.updated_at = Utc::now();
        assert_eq!(digest, catalog_execution_fingerprint(&changed));
    }

    #[test]
    fn upstream_authorization_failure_is_unknown_without_weakening_local_owner_checks() {
        let base = validated_base("https://cma.example").unwrap();
        for status in [401, 403] {
            let error = AppError::from(map_status(status, None, &base));
            assert!(
                matches!(error,AppError::Conflict(ref message) if message == "cma_observation_unavailable")
            );
        }
        assert!(matches!(
            AppError::from(CmaGatewayError::Forbidden),
            AppError::Forbidden(_)
        ));
    }
}
