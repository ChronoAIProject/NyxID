//! Explicit catalog-declared credential checks. Ordinary traffic never records evidence.
use chrono::Utc;
use futures::{StreamExt, TryStreamExt};
use mongodb::{
    Database,
    bson::{self, doc},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::sync::{Arc, Mutex};

use crate::{
    errors::{AppError, AppResult},
    models::{
        credential_verification::{CredentialVerificationCheck, CredentialVerificationObservation},
        downstream_service::DownstreamService,
        service_endpoint::{EndpointRisk, PublicationState, ServiceEndpoint},
        user_api_key::UserApiKey,
        user_endpoint::UserEndpoint,
        user_service::UserService,
    },
};

pub const MAX_RESPONSE_BYTES: usize = 64 * 1024;
pub const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

#[derive(Clone, Debug, Serialize, Deserialize, utoipa::ToSchema)]
pub struct VerificationView {
    pub state: String,
    pub reason_code: String,
    pub observed_at: Option<String>,
    /// `request` is reserved; this release only produces `check`.
    pub source: Option<String>,
    pub can_check: bool,
}
impl VerificationView {
    pub fn unverified(reason: &str, can_check: bool) -> Self {
        Self {
            state: "not_verified".into(),
            reason_code: reason.into(),
            observed_at: None,
            source: None,
            can_check,
        }
    }
}

pub fn eligible(service: &UserService, key: &UserApiKey) -> bool {
    matches!(key.credential_type.as_str(), "api_key" | "bearer")
        && service.service_type == "http"
        && service.node_id.is_none()
        && super::platform_key_service::binding(service) != "platform"
}

/// Nonsecret configuration only: never hash stored credential/ciphertext or a response.
pub fn binding(
    service: &UserService,
    endpoint: &UserEndpoint,
    key: &UserApiKey,
    catalog: Option<&DownstreamService>,
    operation: Option<&ServiceEndpoint>,
) -> String {
    let projection = serde_json::json!({
        "service": service.id, "owner": service.user_id, "version": service.state_version,
        "service_updated": service.updated_at, "endpoint": endpoint.id, "url": endpoint.url,
        "endpoint_updated": endpoint.updated_at, "key": key.id, "epoch": key.credential_epoch,
        "key_status": key.status, "key_updated": key.updated_at,
        "catalog": catalog.map(|c| (&c.id, c.updated_at, &c.credential_verification)),
        "operation": operation.map(|op| (&op.id, op.operation_generation, op.updated_at, &op.path, &op.method, &op.parameters, &op.risk, &op.publication)),
    });
    format!("{:x}", Sha256::digest(projection.to_string().as_bytes()))
}

pub fn view(
    service: &UserService,
    endpoint: &UserEndpoint,
    key: &UserApiKey,
    catalog: Option<&DownstreamService>,
    operation: Option<&ServiceEndpoint>,
) -> Option<VerificationView> {
    if !eligible(service, key) {
        return None;
    }
    let reason = if catalog.is_some_and(|c| c.requires_gateway_url) {
        "gateway_check_unsupported"
    } else if catalog
        .and_then(|c| c.credential_verification.as_ref())
        .is_none()
    {
        "check_not_declared"
    } else if operation.is_none() || !service.is_active || key.status != "active" {
        "check_unavailable"
    } else {
        "not_checked"
    };
    let can_check = reason == "not_checked";
    let Some(observation) = service
        .credential_verification
        .as_ref()
        .filter(|v| v.binding == binding(service, endpoint, key, catalog, operation))
    else {
        return Some(VerificationView::unverified(reason, can_check));
    };
    Some(VerificationView {
        state: observation.state.clone(),
        reason_code: observation.reason_code.clone(),
        observed_at: observation.observed_at.map(|d| d.to_rfc3339()),
        source: observation.observed_at.map(|_| "check".into()),
        can_check,
    })
}

pub fn validate_declaration(check: &CredentialVerificationCheck) -> AppResult<()> {
    let safe = !check.operation.is_empty()
        && check.operation.len() <= 100
        && check
            .operation
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_')
        && check.query.len() <= 4
        && check.query.iter().all(|(name, value)| {
            matches!(name.as_str(), "limit" | "page" | "page_size" | "per_page" | "offset")
                // Fixed pagination bounds only; credentials/URLs/resource IDs are not arguments.
                && value.parse::<u16>().is_ok_and(|n| n <= 1000)
        });
    if !safe {
        return Err(AppError::ValidationError(
            "Verification requires an operation name and bounded numeric query arguments".into(),
        ));
    }
    Ok(())
}

pub async fn seed_declaration(db: &Database, service: &DownstreamService) -> AppResult<()> {
    let Some(spec) = super::catalog_spec_registry::spec_for_slug(&service.slug) else {
        return Ok(());
    };
    let Some(value) = spec.get("x-nyxid-credential-verification") else {
        return Ok(());
    };
    let declaration: CredentialVerificationCheck = serde_json::from_value(value.clone())
        .map_err(|_| AppError::Internal("Invalid trusted verification declaration".into()))?;
    validate_declaration(&declaration)?;
    db.collection::<bson::Document>("downstream_services").update_one(doc! {
        "_id": &service.id, "credential_verification_admin_modified": {"$ne":true},
        "credential_verification": {"$exists":false},
    }, doc! {"$set": {"credential_verification":bson::to_bson(&declaration).map_err(|_|AppError::Internal("Verification encoding failed".into()))?}}).await?;
    Ok(())
}

pub struct Snapshot {
    pub service: UserService,
    pub key: UserApiKey,
    pub endpoint: UserEndpoint,
    pub catalog: Option<DownstreamService>,
    pub operation: Option<ServiceEndpoint>,
}
impl Snapshot {
    pub fn binding(&self) -> String {
        binding(
            &self.service,
            &self.endpoint,
            &self.key,
            self.catalog.as_ref(),
            self.operation.as_ref(),
        )
    }
    pub fn view(&self) -> Option<VerificationView> {
        view(
            &self.service,
            &self.endpoint,
            &self.key,
            self.catalog.as_ref(),
            self.operation.as_ref(),
        )
    }
}
pub async fn snapshot(db: &Database, service_id: &str) -> AppResult<Option<Snapshot>> {
    let Some(service) = db
        .collection::<UserService>("user_services")
        .find_one(doc! {"_id":service_id,"deleted_at":bson::Bson::Null})
        .await?
    else {
        return Ok(None);
    };
    let Some(key_id) = &service.api_key_id else {
        return Ok(None);
    };
    let Some(key) = db
        .collection::<UserApiKey>("user_api_keys")
        .find_one(doc! {"_id":key_id,"user_id":&service.user_id})
        .await?
    else {
        return Ok(None);
    };
    let Some(endpoint) = db
        .collection::<UserEndpoint>("user_endpoints")
        .find_one(doc! {"_id":&service.endpoint_id,"user_id":&service.user_id})
        .await?
    else {
        return Ok(None);
    };
    let catalog = match &service.catalog_service_id {
        Some(id) => {
            db.collection::<DownstreamService>("downstream_services")
                .find_one(doc! {"_id":id})
                .await?
        }
        None => None,
    };
    let mut result = Snapshot {
        service,
        key,
        endpoint,
        catalog,
        operation: None,
    };
    result.operation = operation(db, &result).await?;
    Ok(Some(result))
}

pub async fn operation(db: &Database, snapshot: &Snapshot) -> AppResult<Option<ServiceEndpoint>> {
    let Some(catalog) = snapshot
        .catalog
        .as_ref()
        .filter(|c| c.is_active && !c.requires_gateway_url)
    else {
        return Ok(None);
    };
    let Some(check) = &catalog.credential_verification else {
        return Ok(None);
    };
    validate_declaration(check)?;
    let operations: Vec<ServiceEndpoint> = db
        .collection::<ServiceEndpoint>("service_endpoints")
        .find(doc! {"service_id":&catalog.id,"name":&check.operation,"is_active":true})
        .limit(2)
        .await?
        .try_collect()
        .await?;
    Ok(select_operation(&snapshot.service, catalog, &operations).cloned())
}

/// Inventory and execution must agree when catalog operation names are ambiguous.
pub fn select_operation<'a>(
    service: &UserService,
    catalog: &DownstreamService,
    operations: &'a [ServiceEndpoint],
) -> Option<&'a ServiceEndpoint> {
    let check = catalog.credential_verification.as_ref()?;
    let mut matching = operations
        .iter()
        .filter(|op| op.service_id == catalog.id && op.name == check.operation && op.is_active);
    let operation = matching.next()?;
    (matching.next().is_none() && valid_operation(service, catalog, operation)).then_some(operation)
}

pub fn valid_operation(
    service: &UserService,
    catalog: &DownstreamService,
    op: &ServiceEndpoint,
) -> bool {
    let Some(check) = catalog.credential_verification.as_ref() else {
        return false;
    };
    catalog.is_active
        && !catalog.requires_gateway_url
        && op.service_id == catalog.id
        && op.name == check.operation
        && op.is_active
        && op.method == "GET"
        && op.risk == Some(EndpointRisk::Read)
        && op.publication == PublicationState::Published
        && op.target_id.is_none()
        && op.operation_generation > 0
        && !op.has_request_body()
        && op.async_operation.is_none()
        && op.path.starts_with('/')
        && !op.path.starts_with("//")
        && !op.path.contains(['{', '}', '?', '#', '%', '\\'])
        && !op.path.split('/').any(|s| s == ".." || s == ".")
        && op
            .parameters
            .as_ref()
            .and_then(|v| v.as_array())
            .is_none_or(|params| {
                params.iter().all(|p| {
                    p["required"] != true
                        || (p["in"] == "query"
                            && p["name"]
                                .as_str()
                                .is_some_and(|n| check.query.contains_key(n)))
                })
            })
        && matches!(
            service.auth_method.as_str(),
            "bearer" | "header" | "basic" | "query"
        )
        && !service.forward_access_token
        && !service.inject_delegation_token
        && service.identity_propagation_mode == "none"
}

/// Check-only receipt from the final normal-proxy transport, not a local HTTP refusal.
#[derive(Clone)]
pub struct CheckContext {
    pub db: Database,
    pub actor_id: String,
    pub service_id: String,
    pub key_id: String,
    pub epoch: i64,
    pub binding: String,
    pub outcome: Arc<Mutex<Option<(&'static str, &'static str)>>>,
}
tokio::task_local! { pub static CHECK: CheckContext; }
pub fn checking() -> bool {
    CHECK.try_with(|_| ()).is_ok()
}
pub fn record(state: &'static str, code: &'static str) {
    let _ = CHECK
        .try_with(|c| *c.outcome.lock().unwrap_or_else(|e| e.into_inner()) = Some((state, code)));
}

pub fn require_resolution(
    resolution: &super::proxy_service::UserServiceResolution,
) -> AppResult<()> {
    let wrong = CHECK
        .try_with(|c| {
            resolution.user_service_id != c.service_id
                || resolution.api_key_id.as_deref() != Some(c.key_id.as_str())
                || resolution.credential_epoch != c.epoch
                || resolution.node_id.is_some()
                || resolution.master_credential
                || resolution.pool_selection.is_some()
        })
        .unwrap_or(false);
    if wrong {
        return Err(AppError::Conflict("Verification binding changed".into()));
    }
    Ok(())
}

pub async fn require_target(
    target: &super::proxy_service::ProxyTarget,
    method: &reqwest::Method,
    path: &str,
) -> AppResult<()> {
    let Ok(context) = CHECK.try_with(Clone::clone) else {
        return Ok(());
    };
    let current = snapshot(&context.db, &context.service_id).await?;
    if let Some(s) = &current {
        let access = super::org_service::resolve_owner_access(
            &context.db,
            &context.actor_id,
            &s.service.user_id,
        )
        .await?;
        if !access.can_write() || !access.allows_resource(&context.service_id) {
            return Err(AppError::Forbidden("Connection authority changed".into()));
        }
    }
    if !current.is_some_and(|s| {
        s.binding() == context.binding
            && s.view().is_some_and(|v| v.can_check)
            && s.catalog
                .as_ref()
                .is_some_and(|c| c.id == target.service.id)
            && s.endpoint.url == target.base_url
            && s.service.auth_method == target.auth_method
            && s.service.auth_key_name == target.auth_key_name
            && target.connection_id.is_none()
            && *method == reqwest::Method::GET
            && s.operation
                .as_ref()
                .is_some_and(|op| op.path.trim_start_matches('/') == path.trim_start_matches('/'))
    }) {
        return Err(AppError::Conflict("Verification binding changed".into()));
    }
    Ok(())
}
/// Drain at most 64 KiB then discard. Do not let proxy buffering retain a check body.
pub async fn observe(response: reqwest::Response) -> AppResult<reqwest::Response> {
    let status = response.status();
    let headers = response.headers().clone();
    let mut bytes = 0usize;
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            record(
                "not_verified",
                if error.is_timeout() {
                    "check_timeout"
                } else {
                    "response_incomplete"
                },
            );
            AppError::Internal("Verification response incomplete".into())
        })?;
        bytes = bytes.saturating_add(chunk.len());
        if bytes > MAX_RESPONSE_BYTES {
            record("not_verified", "response_too_large");
            return Err(AppError::BadRequest(
                "Verification response exceeds limit".into(),
            ));
        }
    }
    let (state, code) = match status.as_u16() {
        200..=299 => ("verified", "provider_accepted"),
        401 => ("rejected", "provider_unauthorized"),
        403 => ("rejected", "provider_forbidden"),
        300..=399 => ("not_verified", "redirect_refused"),
        500..=599 => ("not_verified", "provider_unavailable"),
        _ => ("not_verified", "check_unavailable"),
    };
    record(state, code);
    let mut response = http::Response::builder()
        .status(status)
        .body(reqwest::Body::from(Vec::<u8>::new()))
        .map_err(|_| AppError::Internal("Verification response construction failed".into()))?;
    for (name, value) in headers.iter().filter(|(name, _)| {
        !matches!(
            name.as_str(),
            "content-length" | "transfer-encoding" | "content-encoding"
        )
    }) {
        response.headers_mut().insert(name, value.clone());
    }
    Ok(response.into())
}

pub async fn claim(
    db: &Database,
    snapshot: &Snapshot,
    started_at: chrono::DateTime<Utc>,
) -> AppResult<bool> {
    let observation = CredentialVerificationObservation {
        binding: snapshot.binding(),
        state: "not_verified".into(),
        reason_code: "check_in_progress".into(),
        started_at,
        observed_at: None,
    };
    let result=db.collection::<bson::Document>("user_services").update_one(doc!{
        "_id":&snapshot.service.id,"user_id":&snapshot.service.user_id,"updated_at":bson::DateTime::from_chrono(snapshot.service.updated_at),
        "$or":[{"credential_verification.started_at":{"$exists":false}},{"credential_verification.started_at":{"$lt":bson::DateTime::from_chrono(started_at-chrono::Duration::seconds(10))}}],
    },doc!{"$set":{"credential_verification":bson::to_bson(&observation).map_err(|_|AppError::Internal("Verification encoding failed".into()))?}}).await?;
    Ok(result.modified_count == 1)
}
