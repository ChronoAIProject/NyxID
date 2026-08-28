use futures::TryStreamExt;
use mongodb::bson::{Binary, Bson, doc, spec::BinarySubtype};
use mongodb::options::ReturnDocument;
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService, ProxyOperationPolicy,
    ProxyOperationRule,
};
use crate::models::platform_credential::{
    COLLECTION_NAME as PLATFORM_CREDENTIALS, PlatformCredential,
};
use crate::models::platform_operation::{
    COLLECTION_NAME as PLATFORM_OPERATIONS, ConstrainedConfig, ConstrainedOp, PerRequestCaps,
    PlatformOperationKind, PlatformOperationRow, constrained_kind_key, endpoint_kind_key,
};
use crate::models::platform_provider_promotion::{
    COLLECTION_NAME as PLATFORM_PROVIDER_PROMOTIONS, PlatformProviderPromotion,
};
use crate::services::{proxy_authorization, proxy_service};

pub const MAX_PLATFORM_CREDENTIAL_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PlatformProviderContract {
    pub catalog_slug: &'static str,
    pub auth_method: &'static str,
    pub auth_key_name: &'static str,
}

pub const REGISTERED_PLATFORM_PROVIDERS: [PlatformProviderContract; 4] = [
    PlatformProviderContract {
        catalog_slug: "api-elevenlabs",
        auth_method: "header",
        auth_key_name: "xi-api-key",
    },
    PlatformProviderContract {
        catalog_slug: "api-twilio",
        auth_method: "basic",
        auth_key_name: "Authorization",
    },
    PlatformProviderContract {
        catalog_slug: "duffel",
        auth_method: "bearer",
        auth_key_name: "Authorization",
    },
    PlatformProviderContract {
        catalog_slug: "api-twitter",
        auth_method: "bearer",
        auth_key_name: "Authorization",
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformCredentialStatus {
    pub id: String,
    pub catalog_service_id: String,
    pub configured: bool,
    pub auth_method: String,
    pub auth_key_name: String,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug)]
pub struct PlatformProviderLifecycleStatus {
    pub catalog_service: DownstreamService,
    pub eligible: bool,
    pub eligibility_reason: Option<String>,
    pub promotion: Option<PlatformProviderPromotion>,
    pub credential: Option<PlatformCredentialStatus>,
    pub enabled_operation_count: u64,
}

impl From<PlatformCredential> for PlatformCredentialStatus {
    fn from(credential: PlatformCredential) -> Self {
        Self {
            id: credential.id,
            catalog_service_id: credential.catalog_service_id,
            configured: true,
            auth_method: credential.auth_method,
            auth_key_name: credential.auth_key_name,
            created_at: credential.created_at,
            updated_at: credential.updated_at,
        }
    }
}

pub struct AuthorizedPlatformCredential {
    catalog_service: DownstreamService,
    operation: PlatformOperationRow,
    credential: Zeroizing<String>,
}

pub struct AuthorizedPlatformOperation {
    catalog_service: DownstreamService,
    operation: PlatformOperationRow,
}

impl std::fmt::Debug for AuthorizedPlatformOperation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthorizedPlatformOperation")
            .field("catalog_service_id", &self.catalog_service.id)
            .field("operation_id", &self.operation.id)
            .finish()
    }
}

impl AuthorizedPlatformOperation {
    pub fn catalog_service(&self) -> &DownstreamService {
        &self.catalog_service
    }

    pub fn operation(&self) -> &PlatformOperationRow {
        &self.operation
    }
}

impl std::fmt::Debug for AuthorizedPlatformCredential {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AuthorizedPlatformCredential")
            .field("catalog_service_id", &self.catalog_service.id)
            .field("operation_id", &self.operation.id)
            .field("credential", &"[REDACTED]")
            .finish()
    }
}

impl AuthorizedPlatformCredential {
    #[cfg(test)]
    pub fn catalog_service_id(&self) -> &str {
        &self.catalog_service.id
    }

    #[cfg(test)]
    pub fn operation(&self) -> &PlatformOperationRow {
        &self.operation
    }

    pub fn into_proxy_target(self) -> proxy_service::ProxyTarget {
        let catalog_default_headers = self
            .catalog_service
            .default_request_headers
            .clone()
            .unwrap_or_default();
        proxy_service::ProxyTarget {
            base_url: self.catalog_service.base_url.clone(),
            auth_method: self.catalog_service.auth_method.clone(),
            auth_key_name: self.catalog_service.auth_key_name.clone(),
            credential: self.credential.to_string(),
            service: self.catalog_service,
            catalog_default_headers,
            user_service_default_headers: Vec::new(),
            ws_frame_injections: Vec::new(),
            connection_id: None,
        }
    }
}

pub fn provider_contract_for_slug(slug: &str) -> Option<&'static PlatformProviderContract> {
    REGISTERED_PLATFORM_PROVIDERS
        .iter()
        .find(|contract| contract.catalog_slug == slug)
}

pub fn catalog_slug_for_constrained(op: ConstrainedOp) -> &'static str {
    match op {
        ConstrainedOp::Speak => "api-elevenlabs",
        ConstrainedOp::CallAndSay => "api-twilio",
        ConstrainedOp::FlightSearch => "duffel",
    }
}

pub async fn validate_catalog_provider(
    db: &mongodb::Database,
    catalog_service_id: &str,
) -> AppResult<DownstreamService> {
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": catalog_service_id, "is_active": true })
        .await?
        .ok_or_else(|| {
            AppError::PlatformVendorProvisioningInvalid(
                "The catalog service is missing or inactive.".to_string(),
            )
        })?;
    validate_catalog_service_shape(&service)?;
    Ok(service)
}

fn catalog_service_shape_error(service: &DownstreamService) -> Option<String> {
    let Some(contract) = provider_contract_for_slug(&service.slug) else {
        return Some(format!(
            "Catalog service '{}' is not a registered platform provider.",
            service.slug
        ));
    };
    if !service.is_active {
        return Some("The catalog service is inactive.".to_string());
    }
    if service.service_type != "http" {
        return Some(format!(
            "Catalog service '{}' must use service_type 'http'.",
            service.slug
        ));
    }
    if service.auth_method != contract.auth_method
        || !service
            .auth_key_name
            .eq_ignore_ascii_case(contract.auth_key_name)
    {
        return Some(format!(
            "Catalog service '{}' does not match its registered authentication shape.",
            service.slug
        ));
    }
    if service.token_exchange_config.is_some() {
        return Some(format!(
            "Catalog service '{}' uses token exchange and is not eligible for a platform credential.",
            service.slug
        ));
    }
    if service.auth_type.as_deref().is_some_and(|auth_type| {
        matches!(
            auth_type.trim().to_ascii_lowercase().as_str(),
            "oauth" | "oauth2" | "oidc"
        )
    }) {
        return Some(format!(
            "Catalog service '{}' is OAuth-backed and is not eligible for a platform credential.",
            service.slug
        ));
    }
    if !service.credential_encrypted.is_empty() {
        return Some(format!(
            "Catalog service '{}' still carries a master credential and is not eligible for promotion.",
            service.slug
        ));
    }
    None
}

fn validate_catalog_service_shape(
    service: &DownstreamService,
) -> AppResult<&'static PlatformProviderContract> {
    if let Some(reason) = catalog_service_shape_error(service) {
        return Err(AppError::PlatformVendorProvisioningInvalid(reason));
    }
    Ok(provider_contract_for_slug(&service.slug)
        .expect("eligible provider must have a registered contract"))
}

pub async fn list_provider_lifecycle_statuses(
    db: &mongodb::Database,
) -> AppResult<Vec<PlatformProviderLifecycleStatus>> {
    let registered_slugs = REGISTERED_PLATFORM_PROVIDERS
        .iter()
        .map(|contract| contract.catalog_slug)
        .collect::<Vec<_>>();
    let services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(doc! { "slug": { "$in": registered_slugs } })
        .sort(doc! { "name": 1, "slug": 1, "_id": 1 })
        .await?
        .try_collect()
        .await?;
    let mut statuses = Vec::with_capacity(services.len());
    for service in services {
        statuses.push(provider_lifecycle_status_for_service(db, service).await?);
    }
    Ok(statuses)
}

pub async fn provider_lifecycle_status(
    db: &mongodb::Database,
    catalog_service_id: &str,
) -> AppResult<PlatformProviderLifecycleStatus> {
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": catalog_service_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Catalog service not found".to_string()))?;
    provider_lifecycle_status_for_service(db, service).await
}

async fn provider_lifecycle_status_for_service(
    db: &mongodb::Database,
    service: DownstreamService,
) -> AppResult<PlatformProviderLifecycleStatus> {
    let eligibility_reason = catalog_service_shape_error(&service);
    let promotion = db
        .collection::<PlatformProviderPromotion>(PLATFORM_PROVIDER_PROMOTIONS)
        .find_one(doc! { "catalog_service_id": &service.id })
        .await?;
    let credential = credential_status(db, &service.id).await?;
    let enabled_operation_count = db
        .collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
        .count_documents(doc! {
            "catalog_service_id": &service.id,
            "enabled": true,
        })
        .await?;
    Ok(PlatformProviderLifecycleStatus {
        catalog_service: service,
        eligible: eligibility_reason.is_none(),
        eligibility_reason,
        promotion,
        credential,
        enabled_operation_count,
    })
}

pub async fn promote_provider(
    db: &mongodb::Database,
    catalog_service_id: &str,
    vendor_terms_accepted: bool,
    actor_id: &str,
) -> AppResult<PlatformProviderLifecycleStatus> {
    if !vendor_terms_accepted {
        return Err(AppError::PlatformVendorProvisioningInvalid(
            "Vendor terms must be accepted before promoting a platform provider.".to_string(),
        ));
    }
    validate_catalog_provider(db, catalog_service_id).await?;
    let now = chrono::Utc::now();
    db.collection::<PlatformProviderPromotion>(PLATFORM_PROVIDER_PROMOTIONS)
        .find_one_and_update(
            doc! { "catalog_service_id": catalog_service_id },
            doc! {
                "$set": {
                    "updated_by": actor_id,
                    "updated_at": bson::DateTime::from_chrono(now),
                },
                "$setOnInsert": {
                    "_id": uuid::Uuid::new_v4().to_string(),
                    "catalog_service_id": catalog_service_id,
                    "vendor_terms_accepted_by": actor_id,
                    "vendor_terms_accepted_at": bson::DateTime::from_chrono(now),
                    "promoted_by": actor_id,
                    "promoted_at": bson::DateTime::from_chrono(now),
                },
            },
        )
        .upsert(true)
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| {
            AppError::Internal("Platform provider promotion returned no document".to_string())
        })?;
    provider_lifecycle_status(db, catalog_service_id).await
}

pub async fn ensure_provider_promoted(
    db: &mongodb::Database,
    catalog_service_id: &str,
) -> AppResult<PlatformProviderPromotion> {
    db.collection::<PlatformProviderPromotion>(PLATFORM_PROVIDER_PROMOTIONS)
        .find_one(doc! { "catalog_service_id": catalog_service_id })
        .await?
        .ok_or_else(|| {
            AppError::PlatformVendorProvisioningInvalid(
                "The provider has not been promoted with accepted vendor terms.".to_string(),
            )
        })
}

pub async fn demote_provider(
    db: &mongodb::Database,
    catalog_service_id: &str,
) -> AppResult<PlatformProviderLifecycleStatus> {
    provider_lifecycle_status(db, catalog_service_id).await?;
    db.collection::<PlatformProviderPromotion>(PLATFORM_PROVIDER_PROMOTIONS)
        .delete_one(doc! { "catalog_service_id": catalog_service_id })
        .await?;
    disable_provider_operations(db, catalog_service_id).await?;
    db.collection::<PlatformCredential>(PLATFORM_CREDENTIALS)
        .delete_one(doc! { "catalog_service_id": catalog_service_id })
        .await?;
    provider_lifecycle_status(db, catalog_service_id).await
}

pub async fn set_credential(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    catalog_service_id: &str,
    credential: &str,
    created_by: &str,
) -> AppResult<PlatformCredentialStatus> {
    if credential.is_empty() || credential.len() > MAX_PLATFORM_CREDENTIAL_BYTES {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "The platform credential must contain between 1 and {MAX_PLATFORM_CREDENTIAL_BYTES} bytes."
        )));
    }
    let service = validate_catalog_provider(db, catalog_service_id).await?;
    ensure_provider_promoted(db, catalog_service_id).await?;
    let contract = validate_catalog_service_shape(&service)?;
    let encrypted = encryption_keys.encrypt(credential.as_bytes()).await?;
    let encrypted = Bson::Binary(Binary {
        subtype: BinarySubtype::Generic,
        bytes: encrypted,
    });
    let now = chrono::Utc::now();
    let row = db
        .collection::<PlatformCredential>(PLATFORM_CREDENTIALS)
        .find_one_and_update(
            doc! { "catalog_service_id": catalog_service_id },
            doc! {
                "$set": {
                    "credential_encrypted": encrypted,
                    "auth_method": contract.auth_method,
                    "auth_key_name": contract.auth_key_name,
                    "updated_at": bson::DateTime::from_chrono(now),
                },
                "$setOnInsert": {
                    "_id": uuid::Uuid::new_v4().to_string(),
                    "catalog_service_id": catalog_service_id,
                    "created_by": created_by,
                    "created_at": bson::DateTime::from_chrono(now),
                },
            },
        )
        .upsert(true)
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| {
            AppError::Internal("Platform credential upsert returned no document".to_string())
        })?;
    Ok(row.into())
}

#[cfg(test)]
pub async fn set_credential_for_test(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    catalog_service_id: &str,
    credential: &str,
    created_by: &str,
) -> AppResult<PlatformCredentialStatus> {
    promote_provider(db, catalog_service_id, true, created_by).await?;
    set_credential(
        db,
        encryption_keys,
        catalog_service_id,
        credential,
        created_by,
    )
    .await
}

pub async fn credential_status(
    db: &mongodb::Database,
    catalog_service_id: &str,
) -> AppResult<Option<PlatformCredentialStatus>> {
    Ok(db
        .collection::<PlatformCredential>(PLATFORM_CREDENTIALS)
        .find_one(doc! { "catalog_service_id": catalog_service_id })
        .await?
        .map(Into::into))
}

pub async fn credential_is_configured(
    db: &mongodb::Database,
    catalog_service_id: &str,
) -> AppResult<bool> {
    if db
        .collection::<mongodb::bson::Document>(PLATFORM_PROVIDER_PROMOTIONS)
        .find_one(doc! { "catalog_service_id": catalog_service_id })
        .projection(doc! { "_id": 1 })
        .await?
        .is_none()
    {
        return Ok(false);
    }
    Ok(db
        .collection::<mongodb::bson::Document>(PLATFORM_CREDENTIALS)
        .find_one(doc! { "catalog_service_id": catalog_service_id })
        .projection(doc! { "_id": 1 })
        .await?
        .is_some())
}

pub async fn delete_credential(
    db: &mongodb::Database,
    catalog_service_id: &str,
) -> AppResult<PlatformProviderLifecycleStatus> {
    provider_lifecycle_status(db, catalog_service_id).await?;
    disable_provider_operations(db, catalog_service_id).await?;
    db.collection::<PlatformCredential>(PLATFORM_CREDENTIALS)
        .delete_one(doc! { "catalog_service_id": catalog_service_id })
        .await?;
    provider_lifecycle_status(db, catalog_service_id).await
}

async fn disable_provider_operations(
    db: &mongodb::Database,
    catalog_service_id: &str,
) -> AppResult<()> {
    let now = chrono::Utc::now();
    db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
        .update_many(
            doc! { "catalog_service_id": catalog_service_id, "enabled": true },
            doc! {
                "$set": {
                    "enabled": false,
                    "updated_at": bson::DateTime::from_chrono(now),
                },
            },
        )
        .await?;
    Ok(())
}

pub fn normalize_endpoint_definition(
    catalog_slug: &str,
    method: &str,
    path_template: &str,
) -> AppResult<(String, String)> {
    if provider_contract_for_slug(catalog_slug).is_none() {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "Catalog service '{catalog_slug}' is not a registered platform provider."
        )));
    }
    let normalized = proxy_authorization::normalize_policy(ProxyOperationPolicy {
        rules: vec![ProxyOperationRule {
            method: method.to_string(),
            path_template: path_template.to_string(),
        }],
    })?;
    let rule = normalized
        .rules
        .into_iter()
        .next()
        .expect("one input operation must yield one normalized rule");
    let safe = matches!(rule.method.as_str(), "GET" | "HEAD")
        || (catalog_slug == "duffel"
            && rule.method == "POST"
            && rule.path_template == "/air/offer_requests");
    if !safe {
        return Err(AppError::PlatformVendorProvisioningInvalid(format!(
            "{} {} is not registered as a safe platform endpoint.",
            rule.method, rule.path_template
        )));
    }
    Ok((rule.method, rule.path_template))
}

pub async fn authorize_endpoint(
    db: &mongodb::Database,
    catalog_service_id: &str,
    method: &str,
    canonical_path: &proxy_authorization::CanonicalPath,
) -> AppResult<AuthorizedPlatformOperation> {
    let method = method.trim().to_ascii_uppercase();
    let rows: Vec<PlatformOperationRow> = db
        .collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
        .find(doc! {
            "catalog_service_id": catalog_service_id,
            "enabled": true,
            "kind.kind": "endpoint",
            "kind.method": &method,
        })
        .await?
        .try_collect()
        .await?;

    let service = validate_catalog_provider_for_authorization(db, catalog_service_id).await?;
    let mut matches = Vec::new();
    for row in rows {
        let PlatformOperationKind::Endpoint {
            method: row_method,
            path_template,
            ..
        } = &row.kind
        else {
            continue;
        };
        if !matches!(row.limits.per_request, PerRequestCaps::Endpoint) {
            continue;
        }
        let Ok((normalized_method, normalized_path)) =
            normalize_endpoint_definition(&service.slug, row_method, path_template)
        else {
            continue;
        };
        if row.kind_key != endpoint_kind_key(&normalized_method, &normalized_path) {
            continue;
        }
        let rule = ProxyOperationRule {
            method: normalized_method,
            path_template: normalized_path,
        };
        if proxy_authorization::operation_rule_matches(&rule, &method, canonical_path) {
            matches.push(row);
        }
    }
    if matches.len() != 1 {
        return Err(AppError::NotFound(
            "Service operation not found".to_string(),
        ));
    }
    let row = matches.pop().expect("one matching endpoint row");
    Ok(AuthorizedPlatformOperation {
        catalog_service: service,
        operation: row,
    })
}

pub async fn authorize_constrained(
    db: &mongodb::Database,
    catalog_service_id: &str,
    constrained_op: ConstrainedOp,
) -> AppResult<AuthorizedPlatformOperation> {
    let row = db
        .collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
        .find_one(doc! {
            "catalog_service_id": catalog_service_id,
            "kind_key": constrained_kind_key(constrained_op),
            "enabled": true,
        })
        .await?
        .ok_or_else(|| AppError::NotFound("Platform operation not found".to_string()))?;
    validate_constrained_row(&row, constrained_op)?;
    let service = validate_catalog_provider_for_authorization(db, catalog_service_id).await?;
    if service.slug != catalog_slug_for_constrained(constrained_op) {
        return Err(AppError::PlatformOperationUnavailable);
    }
    Ok(AuthorizedPlatformOperation {
        catalog_service: service,
        operation: row,
    })
}

fn validate_constrained_row(
    row: &PlatformOperationRow,
    expected_op: ConstrainedOp,
) -> AppResult<()> {
    if row.kind_key != constrained_kind_key(expected_op) {
        return Err(AppError::PlatformOperationUnavailable);
    }
    let valid = matches!(
        (&row.kind, &row.limits.per_request, expected_op),
        (
            PlatformOperationKind::Constrained {
                op: ConstrainedOp::Speak,
                config: ConstrainedConfig::Speak(_),
            },
            PerRequestCaps::Speak { .. },
            ConstrainedOp::Speak,
        ) | (
            PlatformOperationKind::Constrained {
                op: ConstrainedOp::CallAndSay,
                config: ConstrainedConfig::CallAndSay(_),
            },
            PerRequestCaps::CallAndSay { .. },
            ConstrainedOp::CallAndSay,
        ) | (
            PlatformOperationKind::Constrained {
                op: ConstrainedOp::FlightSearch,
                config: ConstrainedConfig::FlightSearch(_),
            },
            PerRequestCaps::FlightSearch { .. },
            ConstrainedOp::FlightSearch,
        )
    );
    if !valid || row.limits.per_user_per_day == Some(0) {
        return Err(AppError::PlatformOperationUnavailable);
    }
    Ok(())
}

async fn validate_catalog_provider_for_authorization(
    db: &mongodb::Database,
    catalog_service_id: &str,
) -> AppResult<DownstreamService> {
    let service = validate_catalog_provider(db, catalog_service_id)
        .await
        .map_err(|error| {
            tracing::error!(
                catalog_service_id,
                error = %error,
                "Platform credential catalog association is invalid"
            );
            AppError::PlatformOperationUnavailable
        })?;
    ensure_provider_promoted(db, catalog_service_id)
        .await
        .map_err(|error| {
            tracing::error!(
                catalog_service_id,
                error = %error,
                "Platform provider has not passed the vendor-terms promotion gate"
            );
            AppError::PlatformOperationUnavailable
        })?;
    Ok(service)
}

pub async fn materialize_authorized(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    authorization: AuthorizedPlatformOperation,
) -> AppResult<AuthorizedPlatformCredential> {
    let AuthorizedPlatformOperation {
        catalog_service,
        operation,
    } = authorization;
    let credential = db
        .collection::<PlatformCredential>(PLATFORM_CREDENTIALS)
        .find_one(doc! { "catalog_service_id": &catalog_service.id })
        .await?
        .ok_or(AppError::PlatformOperationUnavailable)?;
    let contract = validate_catalog_service_shape(&catalog_service)
        .map_err(|_| AppError::PlatformOperationUnavailable)?;
    if credential.auth_method != contract.auth_method
        || !credential
            .auth_key_name
            .eq_ignore_ascii_case(contract.auth_key_name)
        || credential.credential_encrypted.is_empty()
    {
        return Err(AppError::PlatformOperationUnavailable);
    }
    let plaintext = Zeroizing::new(
        encryption_keys
            .decrypt(&credential.credential_encrypted)
            .await
            .map_err(|error| {
                tracing::error!(
                    catalog_service_id = %catalog_service.id,
                    error = %error,
                    "Platform credential decryption failed"
                );
                AppError::PlatformOperationUnavailable
            })?,
    );
    let credential = Zeroizing::new(String::from_utf8(plaintext.to_vec()).map_err(|error| {
        tracing::error!(
            catalog_service_id = %catalog_service.id,
            error = %error,
            "Platform credential is not UTF-8"
        );
        AppError::PlatformOperationUnavailable
    })?);
    if credential.is_empty() {
        return Err(AppError::PlatformOperationUnavailable);
    }
    Ok(AuthorizedPlatformCredential {
        catalog_service,
        operation,
        credential,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::test_helpers::dummy_service;
    use crate::models::downstream_service::{CredentialFieldSpec, TokenExchangeConfig};
    use crate::models::platform_operation::{
        OperationBilling, OperationLimits, PerRequestCaps, PlatformOperationRow,
    };
    use crate::models::service_billing::BillingMetric;

    fn catalog_service(slug: &str) -> DownstreamService {
        let contract = provider_contract_for_slug(slug).expect("registered test provider");
        let mut service = dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = slug.to_string();
        service.name = slug.to_string();
        service.base_url = "https://vendor.example".to_string();
        service.auth_method = contract.auth_method.to_string();
        service.auth_key_name = contract.auth_key_name.to_string();
        service.is_active = true;
        service
    }

    fn token_exchange_config() -> TokenExchangeConfig {
        TokenExchangeConfig {
            endpoint: "https://vendor.example/oauth/token".to_string(),
            request_encoding: "json".to_string(),
            request_template: serde_json::json!({ "client_secret": "$client_secret" }),
            token_response_path: "access_token".to_string(),
            ttl_response_path: Some("expires_in".to_string()),
            default_ttl_secs: 3_600,
            injection: "bearer".to_string(),
            error_code_path: None,
            error_message_path: None,
            credential_fields: vec![CredentialFieldSpec {
                name: "client_secret".to_string(),
                label: "Client secret".to_string(),
                placeholder: None,
                secret: true,
            }],
        }
    }

    #[test]
    fn eligibility_is_registry_bounded_and_rejects_unsafe_catalog_shapes() {
        for slug in ["api-elevenlabs", "api-twilio", "duffel", "api-twitter"] {
            assert!(
                catalog_service_shape_error(&catalog_service(slug)).is_none(),
                "registered provider {slug} should be eligible"
            );
        }

        let mut unregistered = catalog_service("duffel");
        unregistered.slug = "unreviewed-provider".to_string();
        let mut inactive = catalog_service("duffel");
        inactive.is_active = false;
        let mut node_only = catalog_service("duffel");
        node_only.service_type = "ssh".to_string();
        let mut path_auth = catalog_service("duffel");
        path_auth.auth_method = "path".to_string();
        let mut query_auth = catalog_service("duffel");
        query_auth.auth_method = "query".to_string();
        let mut oauth = catalog_service("duffel");
        oauth.auth_type = Some("oauth2".to_string());
        let mut token_exchange = catalog_service("duffel");
        token_exchange.token_exchange_config = Some(token_exchange_config());
        let mut master_credential = catalog_service("duffel");
        master_credential.credential_encrypted = vec![1, 2, 3];

        for (label, service) in [
            ("unregistered", unregistered),
            ("inactive", inactive),
            ("node-only", node_only),
            ("path auth", path_auth),
            ("query auth", query_auth),
            ("OAuth", oauth),
            ("token exchange", token_exchange),
            ("master credential", master_credential),
        ] {
            assert!(
                catalog_service_shape_error(&service).is_some(),
                "{label} provider shape was accepted"
            );
        }
    }

    #[tokio::test]
    async fn vendor_terms_default_deny_and_promotion_never_enables_operations() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_provider_terms_gate").await
        else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("create platform provider indexes");
        let keys = crate::test_utils::test_encryption_keys();
        let service = catalog_service("duffel");
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert provider");
        let mut operation = PlatformOperationRow::new_endpoint(
            service.id.clone(),
            "GET".to_string(),
            "/air/offers/{id}".to_string(),
            "Read offer".to_string(),
            None,
            OperationLimits {
                per_request: PerRequestCaps::Endpoint,
                per_user_per_day: Some(10),
            },
            OperationBilling::free(BillingMetric::Requests),
            "admin".to_string(),
        );
        operation.enabled = false;
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(&operation)
            .await
            .expect("insert disabled operation");

        let error = set_credential(&db, &keys, &service.id, "secret", "admin")
            .await
            .expect_err("unpromoted provider must reject credentials");
        assert!(matches!(
            error,
            AppError::PlatformVendorProvisioningInvalid(_)
        ));
        promote_provider(&db, &service.id, false, "admin")
            .await
            .expect_err("terms refusal must deny promotion");
        assert_eq!(
            db.collection::<PlatformProviderPromotion>(PLATFORM_PROVIDER_PROMOTIONS)
                .count_documents(doc! {})
                .await
                .expect("count promotions"),
            0
        );

        let (first, second) = tokio::join!(
            promote_provider(&db, &service.id, true, "admin-a"),
            promote_provider(&db, &service.id, true, "admin-b"),
        );
        first.expect("first promotion");
        second.expect("idempotent concurrent promotion");
        assert_eq!(
            db.collection::<PlatformProviderPromotion>(PLATFORM_PROVIDER_PROMOTIONS)
                .count_documents(doc! { "catalog_service_id": &service.id })
                .await
                .expect("count provider promotions"),
            1
        );
        assert!(
            !db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
                .find_one(doc! { "_id": &operation.id })
                .await
                .expect("read operation")
                .expect("operation exists")
                .enabled,
            "promotion must not grant operation authority"
        );
    }

    #[tokio::test]
    async fn credential_write_is_redacted_and_replaces_without_duplicate_rows() {
        let Some(db) = crate::test_utils::connect_test_database("platform_credential_write").await
        else {
            eprintln!("skipping platform credential test: no MongoDB");
            return;
        };
        let keys = crate::test_utils::test_encryption_keys();
        let service = catalog_service("duffel");
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert catalog service");

        let first = set_credential_for_test(&db, &keys, &service.id, "first-secret", "admin")
            .await
            .expect("set credential");
        let second = set_credential_for_test(&db, &keys, &service.id, "second-secret", "admin")
            .await
            .expect("replace credential");

        assert_eq!(first.id, second.id);
        assert_eq!(
            db.collection::<PlatformCredential>(PLATFORM_CREDENTIALS)
                .count_documents(doc! { "catalog_service_id": &service.id })
                .await
                .expect("count credentials"),
            1
        );
        let stored = db
            .collection::<PlatformCredential>(PLATFORM_CREDENTIALS)
            .find_one(doc! { "catalog_service_id": &service.id })
            .await
            .expect("read credential")
            .expect("stored credential");
        let debug = format!("{stored:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("second-secret"));
        assert_ne!(stored.credential_encrypted, b"second-secret");

        let status = provider_lifecycle_status(&db, &service.id)
            .await
            .expect("read provider lifecycle");
        assert!(status.promotion.is_some());
        assert_eq!(status.credential.expect("credential status").id, stored.id);
    }

    #[tokio::test]
    async fn missing_promotion_denies_authorization_before_decryption() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_promotion_authorization_gate").await
        else {
            return;
        };
        let keys = crate::test_utils::test_encryption_keys();
        let service = catalog_service("duffel");
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert provider");
        set_credential_for_test(&db, &keys, &service.id, "duffel-secret", "admin")
            .await
            .expect("set credential");
        let mut operation = PlatformOperationRow::new_endpoint(
            service.id.clone(),
            "GET".to_string(),
            "/air/offers/{id}".to_string(),
            "Read offer".to_string(),
            None,
            OperationLimits {
                per_request: PerRequestCaps::Endpoint,
                per_user_per_day: Some(10),
            },
            OperationBilling::free(BillingMetric::Requests),
            "admin".to_string(),
        );
        operation.enabled = true;
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(&operation)
            .await
            .expect("insert enabled operation");
        db.collection::<PlatformProviderPromotion>(PLATFORM_PROVIDER_PROMOTIONS)
            .delete_one(doc! { "catalog_service_id": &service.id })
            .await
            .expect("remove promotion");
        let before = keys.decrypt_stats();
        let path = proxy_authorization::CanonicalPath::from_mcp_literal("/air/offers/offer-1")
            .expect("canonical path");

        let error = authorize_endpoint(&db, &service.id, "GET", &path)
            .await
            .expect_err("missing promotion must deny authorization");

        assert!(matches!(error, AppError::PlatformOperationUnavailable));
        assert_eq!(keys.decrypt_stats(), before);
    }

    #[tokio::test]
    async fn deleting_a_credential_disables_operations_before_removing_the_secret() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_credential_delete_lifecycle").await
        else {
            return;
        };
        let keys = crate::test_utils::test_encryption_keys();
        let service = catalog_service("duffel");
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert provider");
        set_credential_for_test(&db, &keys, &service.id, "duffel-secret", "admin")
            .await
            .expect("set credential");
        let mut operation = PlatformOperationRow::new_endpoint(
            service.id.clone(),
            "GET".to_string(),
            "/air/offers/{id}".to_string(),
            "Read offer".to_string(),
            None,
            OperationLimits {
                per_request: PerRequestCaps::Endpoint,
                per_user_per_day: Some(10),
            },
            OperationBilling::free(BillingMetric::Requests),
            "admin".to_string(),
        );
        operation.enabled = true;
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(&operation)
            .await
            .expect("insert enabled operation");

        let status = delete_credential(&db, &service.id)
            .await
            .expect("delete credential");

        assert!(status.promotion.is_some());
        assert!(status.credential.is_none());
        assert_eq!(status.enabled_operation_count, 0);
        assert!(
            db.collection::<PlatformCredential>(PLATFORM_CREDENTIALS)
                .find_one(doc! { "catalog_service_id": &service.id })
                .await
                .expect("read credential")
                .is_none()
        );
        assert!(
            !db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
                .find_one(doc! { "_id": &operation.id })
                .await
                .expect("read operation")
                .expect("operation exists")
                .enabled
        );
    }

    #[tokio::test]
    async fn no_authorizer_match_never_decrypts_the_configured_credential() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_no_authorizer_decrypt").await
        else {
            eprintln!("skipping platform authorizer test: no MongoDB");
            return;
        };
        let keys = crate::test_utils::test_encryption_keys();
        let service = catalog_service("duffel");
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert catalog service");
        set_credential_for_test(&db, &keys, &service.id, "duffel-secret", "admin")
            .await
            .expect("set credential");
        let before = keys.decrypt_stats();
        let path = proxy_authorization::CanonicalPath::from_mcp_literal("/air/offers")
            .expect("canonical path");

        let error = authorize_endpoint(&db, &service.id, "GET", &path)
            .await
            .expect_err("missing endpoint authorizer must deny");

        assert!(matches!(error, AppError::NotFound(_)));
        assert_eq!(keys.decrypt_stats(), before);
    }

    #[tokio::test]
    async fn exact_enabled_endpoint_authorizes_and_redacts_the_materialized_secret() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_endpoint_authorizer").await
        else {
            eprintln!("skipping platform authorizer test: no MongoDB");
            return;
        };
        let keys = crate::test_utils::test_encryption_keys();
        let service = catalog_service("duffel");
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert catalog service");
        set_credential_for_test(&db, &keys, &service.id, "duffel-secret", "admin")
            .await
            .expect("set credential");
        let mut row = PlatformOperationRow::new_endpoint(
            service.id.clone(),
            "POST".to_string(),
            "/air/offer_requests".to_string(),
            "Create offer request".to_string(),
            None,
            OperationLimits {
                per_request: PerRequestCaps::Endpoint,
                per_user_per_day: Some(10),
            },
            OperationBilling::free(BillingMetric::Requests),
            "admin".to_string(),
        );
        row.enabled = true;
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(&row)
            .await
            .expect("insert endpoint row");
        let path = proxy_authorization::CanonicalPath::from_mcp_literal("/air/offer_requests")
            .expect("canonical path");
        let decrypts_before = keys.decrypt_stats();

        let authorization = authorize_endpoint(&db, &service.id, "POST", &path)
            .await
            .expect("authorize endpoint");

        assert_eq!(authorization.operation().id, row.id);
        assert_eq!(keys.decrypt_stats(), decrypts_before);
        let authorization_debug = format!("{authorization:?}");
        assert!(!authorization_debug.contains("duffel-secret"));
        let authorized = materialize_authorized(&db, &keys, authorization)
            .await
            .expect("materialize credential after authorization");
        let debug = format!("{authorized:?}");
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("duffel-secret"));
        assert_eq!(authorized.into_proxy_target().credential, "duffel-secret");
    }

    #[tokio::test]
    async fn constrained_authorizer_binds_exact_provider_and_rejects_invalid_row_before_decrypt() {
        let Some(db) =
            crate::test_utils::connect_test_database("platform_constrained_authorizer").await
        else {
            eprintln!("skipping constrained platform authorizer test: no MongoDB");
            return;
        };
        let keys = crate::test_utils::test_encryption_keys();
        let service = catalog_service("api-elevenlabs");
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert ElevenLabs catalog service");
        set_credential_for_test(&db, &keys, &service.id, "elevenlabs-secret", "admin")
            .await
            .expect("set credential");
        let mut invalid = PlatformOperationRow::new_constrained(
            service.id.clone(),
            ConstrainedOp::Speak,
            ConstrainedConfig::CallAndSay(
                crate::models::platform_operation::CallAndSayOperationConfig {
                    allowed_destination_prefixes: vec!["+65".to_string()],
                    voice: "alice".to_string(),
                    account_sid: format!("AC{}", "1".repeat(32)),
                    call_from: "+16505550100".to_string(),
                },
            ),
            OperationLimits {
                per_request: PerRequestCaps::CallAndSay {
                    max_message_chars: 500,
                    max_duration_seconds: 600,
                },
                per_user_per_day: Some(3),
            },
            OperationBilling::free(BillingMetric::Requests),
            "admin".to_string(),
        );
        invalid.enabled = true;
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(&invalid)
            .await
            .expect("insert invalid tagged row");
        let before = keys.decrypt_stats();

        let error = authorize_constrained(&db, &service.id, ConstrainedOp::Speak)
            .await
            .expect_err("mismatched constrained row must fail closed");
        assert!(matches!(error, AppError::PlatformOperationUnavailable));
        assert_eq!(keys.decrypt_stats(), before);

        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .delete_one(doc! { "_id": &invalid.id })
            .await
            .expect("remove invalid row");
        let mut valid = PlatformOperationRow::new_constrained(
            service.id.clone(),
            ConstrainedOp::Speak,
            ConstrainedConfig::Speak(crate::models::platform_operation::SpeakOperationConfig {
                allowed_voice_ids: vec!["voice-a".to_string()],
                model_id: "eleven_multilingual_v2".to_string(),
                max_calls_per_user_per_day: 50,
            }),
            OperationLimits {
                per_request: PerRequestCaps::Speak { max_chars: 500 },
                per_user_per_day: None,
            },
            OperationBilling::free(BillingMetric::Requests),
            "admin".to_string(),
        );
        valid.enabled = true;
        db.collection::<PlatformOperationRow>(PLATFORM_OPERATIONS)
            .insert_one(&valid)
            .await
            .expect("insert valid constrained row");

        let authorization = authorize_constrained(&db, &service.id, ConstrainedOp::Speak)
            .await
            .expect("authorize exact constrained row");
        assert_eq!(authorization.operation().id, valid.id);
        assert_eq!(keys.decrypt_stats(), before);
        let authorized = materialize_authorized(&db, &keys, authorization)
            .await
            .expect("materialize exact constrained credential");
        assert_eq!(authorized.catalog_service_id(), service.id);
        assert_eq!(authorized.operation().id, valid.id);
        assert_eq!(
            authorized.into_proxy_target().credential,
            "elevenlabs-secret"
        );
    }

    #[test]
    fn endpoint_registry_defaults_to_safe_reads_and_one_exact_duffel_post() {
        for slug in ["api-elevenlabs", "api-twilio", "duffel", "api-twitter"] {
            normalize_endpoint_definition(slug, "GET", "/resources/{id}")
                .expect("registered providers allow safe reads");
            normalize_endpoint_definition(slug, "HEAD", "/resources/{id}")
                .expect("registered providers allow safe heads");
        }
        normalize_endpoint_definition("duffel", "POST", "/air/offer_requests")
            .expect("registered Duffel offer request");
        for (slug, method, path) in [
            ("duffel", "POST", "/air/orders"),
            ("api-twitter", "POST", "/2/tweets"),
            ("api-elevenlabs", "DELETE", "/v1/voices/{id}"),
            ("unknown", "GET", "/anything"),
        ] {
            assert!(
                normalize_endpoint_definition(slug, method, path).is_err(),
                "accepted unsafe endpoint {slug} {method} {path}"
            );
        }
    }
}
