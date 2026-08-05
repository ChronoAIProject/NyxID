use chrono::{Duration, Utc};
use mongodb::bson::{self, doc};
use mongodb::options::ReturnDocument;
use uuid::Uuid;

use crate::crypto::aes::EncryptionKeys;
use crate::crypto::token::{generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::connect_link::{
    COLLECTION_NAME as CONNECT_LINKS, ConnectLink, ConnectLinkStatus,
};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDERS, ProviderConfig};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::redaction::RedactedLen;
use crate::services::{audit_service, org_service, unified_key_service};

pub const CONNECT_LINK_PREFIX: &str = "nyx_clk_";
pub const DEFAULT_TTL_SECS: i64 = 15 * 60;
pub const MIN_TTL_SECS: i64 = 60;
pub const MAX_TTL_SECS: i64 = 60 * 60;
pub const MAX_WAIT_SECS: u64 = 120;

const MAX_LABEL_LEN: usize = 200;
const MAX_REQUESTED_BY_LEN: usize = 200;
const MAX_CALLBACK_URL_LEN: usize = 2048;

pub struct CreateInput {
    pub user_id: String,
    pub service_slug: String,
    pub label: Option<String>,
    pub requested_by: Option<String>,
    pub callback_url: Option<String>,
    pub ttl_secs: Option<i64>,
}

pub struct CreatedLink {
    pub link: ConnectLink,
    pub raw_token: String,
}

impl std::fmt::Debug for CreatedLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CreatedLink")
            .field("link", &self.link)
            .field("raw_token", &RedactedLen(self.raw_token.len()))
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct CatalogConnectInfo {
    pub service_id: String,
    pub service_slug: String,
    pub service_name: String,
    pub auth_method: String,
    pub auth_key_name: String,
    pub provider_id: Option<String>,
    pub provider_type: Option<String>,
    pub credential_mode: Option<String>,
    pub has_platform_oauth_credentials: bool,
    pub requires_gateway_url: bool,
    pub api_key_url: Option<String>,
    pub api_key_instructions: Option<String>,
}

impl CatalogConnectInfo {
    pub fn connect_method(&self) -> &'static str {
        match self.provider_type.as_deref() {
            Some("oauth2") => "oauth",
            Some("device_code") => "device_code",
            Some("api_key") => "api_key",
            _ if self.auth_method == "none" => "none",
            _ => "api_key",
        }
    }
}

#[derive(Debug)]
pub struct LinkView {
    pub link: ConnectLink,
    pub service: CatalogConnectInfo,
    pub completed_service_slug: Option<String>,
}

#[derive(Default)]
pub struct CompleteInput<'a> {
    pub credential: Option<&'a str>,
    pub endpoint_url: Option<&'a str>,
    pub oauth_client_id: Option<&'a str>,
    pub oauth_client_secret: Option<&'a str>,
}

impl std::fmt::Debug for CompleteInput<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompleteInput")
            .field(
                "credential",
                &self.credential.map(|value| RedactedLen(value.len())),
            )
            .field(
                "endpoint_url",
                &self.endpoint_url.map(|value| RedactedLen(value.len())),
            )
            .field(
                "oauth_client_id",
                &self.oauth_client_id.map(|value| RedactedLen(value.len())),
            )
            .field(
                "oauth_client_secret",
                &self
                    .oauth_client_secret
                    .map(|value| RedactedLen(value.len())),
            )
            .finish()
    }
}

#[derive(Debug)]
pub enum CompleteResult {
    Completed(LinkView),
    OauthRequired {
        view: LinkView,
        provider_id: String,
        connection_id: String,
    },
    DeviceCodeRequired {
        view: LinkView,
        provider_id: String,
        connection_id: String,
    },
}

pub async fn create(db: &mongodb::Database, input: CreateInput) -> AppResult<CreatedLink> {
    let service = load_catalog_info_by_slug(db, &input.service_slug).await?;
    if service.service_slug.trim().is_empty() {
        return Err(AppError::ValidationError(
            "service_slug must not be empty".to_string(),
        ));
    }
    let label = normalize_optional(input.label, MAX_LABEL_LEN, "label")?;
    let requested_by =
        normalize_optional(input.requested_by, MAX_REQUESTED_BY_LEN, "requested_by")?;
    let callback_url = validate_callback_url(input.callback_url.as_deref())?;
    let ttl_secs = input
        .ttl_secs
        .unwrap_or(DEFAULT_TTL_SECS)
        .clamp(MIN_TTL_SECS, MAX_TTL_SECS);

    let raw_token = format!("{CONNECT_LINK_PREFIX}{}", generate_random_token());
    let now = Utc::now();
    let link = ConnectLink {
        id: Uuid::new_v4().to_string(),
        user_id: input.user_id,
        service_slug: service.service_slug.clone(),
        service_id: service.service_id.clone(),
        label,
        requested_by,
        token_hash: hash_token(&raw_token),
        status: ConnectLinkStatus::Pending,
        callback_url,
        created_at: now,
        completed_at: None,
        expires_at: now + Duration::seconds(ttl_secs),
        completed_user_service_id: None,
        completion_claim_id: None,
    };

    db.collection::<ConnectLink>(CONNECT_LINKS)
        .insert_one(&link)
        .await?;

    Ok(CreatedLink { link, raw_token })
}

pub fn build_connect_url(frontend_url: &str, raw_token: &str) -> AppResult<String> {
    let frontend_url = frontend_url.trim().trim_end_matches('/');
    if frontend_url.is_empty() {
        return Err(AppError::Internal(
            "connect-link frontend URL is not configured".to_string(),
        ));
    }
    let connect_url = format!("{frontend_url}/connect/{raw_token}");
    let parsed = url::Url::parse(&connect_url)
        .map_err(|_| AppError::Internal("connect-link frontend URL is invalid".to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(AppError::Internal(
            "connect-link frontend URL is invalid".to_string(),
        ));
    }
    Ok(parsed.to_string())
}

pub async fn preview(db: &mongodb::Database, raw_token: &str) -> AppResult<LinkView> {
    let link = find_by_raw_token(db, raw_token).await?;
    let link = claim_expiry(db, link, None).await?;
    view_for_link(db, link).await
}

pub async fn get_for_actor(
    db: &mongodb::Database,
    actor_user_id: &str,
    link_id: &str,
) -> AppResult<LinkView> {
    let link = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find_one(doc! { "_id": link_id })
        .await?
        .ok_or(AppError::ConnectLinkNotFound)?;
    ensure_actor_can_manage(db, actor_user_id, &link).await?;
    let link = claim_expiry(db, link, None).await?;
    view_for_link(db, link).await
}

pub async fn cancel(
    db: &mongodb::Database,
    actor_user_id: &str,
    link_id: &str,
) -> AppResult<LinkView> {
    let current = get_for_actor(db, actor_user_id, link_id).await?;
    match current.link.status {
        ConnectLinkStatus::Expired => return Err(AppError::ConnectLinkExpired),
        ConnectLinkStatus::Completed => return Err(AppError::ConnectLinkAlreadyCompleted),
        ConnectLinkStatus::Cancelled => return Ok(current),
        ConnectLinkStatus::Pending => {}
    }

    let updated = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find_one_and_update(
            doc! {
                "_id": link_id,
                "status": "pending",
                "completion_claim_id": null,
            },
            doc! { "$set": { "status": "cancelled" } },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or(AppError::ConnectLinkAlreadyCompleted)?;

    view_for_link(db, updated).await
}

#[allow(clippy::too_many_arguments)]
pub async fn complete(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    actor_user_id: &str,
    raw_token: &str,
    input: CompleteInput<'_>,
    hosted_mode: bool,
) -> AppResult<CompleteResult> {
    let current = find_by_raw_token(db, raw_token).await?;
    ensure_actor_can_manage(db, actor_user_id, &current).await?;
    let current = claim_expiry(db, current, Some(actor_user_id)).await?;
    ensure_pending(&current)?;
    let catalog = load_catalog_info_by_id(db, &current.service_id).await?;

    if let Some(service_id) = current.completed_user_service_id.clone() {
        return resume_existing_completion(db, current, catalog, &service_id).await;
    }

    let claim_id = Uuid::new_v4().to_string();
    let claimed = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find_one_and_update(
            doc! {
                "_id": &current.id,
                "status": "pending",
                "completion_claim_id": null,
                "completed_user_service_id": null,
                "expires_at": { "$gt": bson::DateTime::from_chrono(Utc::now()) },
            },
            doc! { "$set": { "completion_claim_id": &claim_id } },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or(AppError::ConnectLinkAlreadyCompleted)?;

    let credential = input.credential.unwrap_or("").trim();
    if catalog.connect_method() == "api_key" && credential.is_empty() {
        release_claim(db, &claimed.id, &claim_id).await;
        return Err(AppError::ValidationError(
            "credential must not be empty".to_string(),
        ));
    }
    if catalog.requires_gateway_url && input.endpoint_url.is_none_or(|url| url.trim().is_empty()) {
        release_claim(db, &claimed.id, &claim_id).await;
        return Err(AppError::ValidationError(
            "endpoint_url is required for this service".to_string(),
        ));
    }

    let oauth_credentials = match (
        input.oauth_client_id.map(str::trim),
        input.oauth_client_secret.map(str::trim),
    ) {
        (None, None) => unified_key_service::OauthClientCredentialsInput::None,
        (Some(id), Some(secret)) if !id.is_empty() && !secret.is_empty() => {
            unified_key_service::OauthClientCredentialsInput::Raw {
                client_id: id,
                client_secret: secret,
            }
        }
        _ => {
            release_claim(db, &claimed.id, &claim_id).await;
            return Err(AppError::ValidationError(
                "oauth_client_id and oauth_client_secret must be supplied together".to_string(),
            ));
        }
    };

    let label = claimed
        .label
        .as_deref()
        .unwrap_or(&catalog.service_name)
        .to_string();
    let created = unified_key_service::create_key(
        db,
        encryption_keys,
        &claimed.user_id,
        actor_user_id,
        Some(&claimed.service_slug),
        input.endpoint_url.map(str::trim),
        credential,
        &label,
        None,
        None,
        None,
        None,
        None,
        None,
        unified_key_service::OpenApiSpecUrlInput::Inherit,
        None,
        false,
        oauth_credentials,
        hosted_mode,
    )
    .await;

    let created = match created {
        Ok(created) => created,
        Err(error) => {
            release_claim(db, &claimed.id, &claim_id).await;
            return Err(error);
        }
    };

    let service_id = created.service.id.clone();
    let pending_oauth = created
        .api_key
        .as_ref()
        .is_some_and(|key| key.status == "pending_auth" && key.connection_id.is_some());
    if pending_oauth {
        let updated = store_provisioned_service(db, &claimed.id, &claim_id, &service_id).await?;
        let provider_id = catalog.provider_id.clone().ok_or_else(|| {
            AppError::Internal(
                "OAuth catalog service is missing provider configuration".to_string(),
            )
        })?;
        let connection_id = created
            .api_key
            .and_then(|key| key.connection_id)
            .ok_or_else(|| AppError::Internal("OAuth connection id was not created".to_string()))?;
        let view = view_for_link(db, updated).await?;
        return if catalog.connect_method() == "device_code" {
            Ok(CompleteResult::DeviceCodeRequired {
                view,
                provider_id,
                connection_id,
            })
        } else {
            Ok(CompleteResult::OauthRequired {
                view,
                provider_id,
                connection_id,
            })
        };
    }

    let completed = finish_claim(db, &claimed.id, &claim_id, &service_id).await?;
    Ok(CompleteResult::Completed(
        view_for_link(db, completed).await?,
    ))
}

pub async fn complete_oauth_callback(
    db: &mongodb::Database,
    connect_link_id: &str,
    owner_user_id: &str,
    connection_id: &str,
) -> AppResult<LinkView> {
    let api_key = db
        .collection::<UserApiKey>(USER_API_KEYS)
        .find_one(doc! {
            "user_id": owner_user_id,
            "connection_id": connection_id,
            "status": "active",
        })
        .await?
        .ok_or(AppError::ConnectLinkNotFound)?;
    let service = db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! {
            "user_id": owner_user_id,
            "api_key_id": &api_key.id,
            "is_active": true,
        })
        .await?
        .ok_or(AppError::ConnectLinkNotFound)?;
    let link = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find_one(doc! {
            "_id": connect_link_id,
            "user_id": owner_user_id,
            "completed_user_service_id": &service.id,
        })
        .await?
        .ok_or(AppError::ConnectLinkNotFound)?;
    let link = claim_expiry(db, link, Some(owner_user_id)).await?;
    ensure_pending(&link)?;

    let updated = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find_one_and_update(
            doc! {
                "_id": &link.id,
                "status": "pending",
                "completion_claim_id": null,
            },
            doc! {
                "$set": {
                    "status": "completed",
                    "completed_at": bson::DateTime::from_chrono(Utc::now()),
                },
                "$unset": { "completion_claim_id": "" },
            },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or(AppError::ConnectLinkAlreadyCompleted)?;
    view_for_link(db, updated).await
}

pub async fn wait_for_status(
    db: &mongodb::Database,
    actor_user_id: &str,
    link_id: &str,
    timeout_secs: u64,
) -> AppResult<LinkView> {
    let timeout_secs = timeout_secs.clamp(1, MAX_WAIT_SECS);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    loop {
        let view = get_for_actor(db, actor_user_id, link_id).await?;
        if view.link.status != ConnectLinkStatus::Pending || tokio::time::Instant::now() >= deadline
        {
            return Ok(view);
        }
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }
}

async fn resume_existing_completion(
    db: &mongodb::Database,
    link: ConnectLink,
    catalog: CatalogConnectInfo,
    service_id: &str,
) -> AppResult<CompleteResult> {
    let service = db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! { "_id": service_id, "user_id": &link.user_id, "is_active": true })
        .await?
        .ok_or(AppError::ConnectLinkNotFound)?;
    let key = match service.api_key_id.as_deref() {
        Some(key_id) => {
            db.collection::<UserApiKey>(USER_API_KEYS)
                .find_one(doc! { "_id": key_id, "user_id": &link.user_id })
                .await?
        }
        None => None,
    };

    if key.as_ref().is_none_or(|key| key.status == "active") {
        let updated = db
            .collection::<ConnectLink>(CONNECT_LINKS)
            .find_one_and_update(
                doc! { "_id": &link.id, "status": "pending" },
                doc! {
                    "$set": {
                        "status": "completed",
                        "completed_at": bson::DateTime::from_chrono(Utc::now()),
                    },
                    "$unset": { "completion_claim_id": "" },
                },
            )
            .return_document(ReturnDocument::After)
            .await?
            .ok_or(AppError::ConnectLinkAlreadyCompleted)?;
        return Ok(CompleteResult::Completed(view_for_link(db, updated).await?));
    }

    let key = key.ok_or(AppError::ConnectLinkNotFound)?;
    let provider_id = catalog
        .provider_id
        .clone()
        .ok_or(AppError::ConnectLinkNotFound)?;
    let connection_id = key.connection_id.ok_or(AppError::ConnectLinkNotFound)?;
    let view = view_for_link(db, link).await?;
    if catalog.connect_method() == "device_code" {
        Ok(CompleteResult::DeviceCodeRequired {
            view,
            provider_id,
            connection_id,
        })
    } else {
        Ok(CompleteResult::OauthRequired {
            view,
            provider_id,
            connection_id,
        })
    }
}

async fn store_provisioned_service(
    db: &mongodb::Database,
    link_id: &str,
    claim_id: &str,
    service_id: &str,
) -> AppResult<ConnectLink> {
    db.collection::<ConnectLink>(CONNECT_LINKS)
        .find_one_and_update(
            doc! { "_id": link_id, "status": "pending", "completion_claim_id": claim_id },
            doc! {
                "$set": { "completed_user_service_id": service_id },
                "$unset": { "completion_claim_id": "" },
            },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or(AppError::ConnectLinkAlreadyCompleted)
}

async fn finish_claim(
    db: &mongodb::Database,
    link_id: &str,
    claim_id: &str,
    service_id: &str,
) -> AppResult<ConnectLink> {
    db.collection::<ConnectLink>(CONNECT_LINKS)
        .find_one_and_update(
            doc! { "_id": link_id, "status": "pending", "completion_claim_id": claim_id },
            doc! {
                "$set": {
                    "status": "completed",
                    "completed_at": bson::DateTime::from_chrono(Utc::now()),
                    "completed_user_service_id": service_id,
                },
                "$unset": { "completion_claim_id": "" },
            },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or(AppError::ConnectLinkAlreadyCompleted)
}

async fn release_claim(db: &mongodb::Database, link_id: &str, claim_id: &str) {
    if let Err(error) = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .update_one(
            doc! { "_id": link_id, "status": "pending", "completion_claim_id": claim_id },
            doc! { "$unset": { "completion_claim_id": "" } },
        )
        .await
    {
        tracing::error!(link_id, %error, "failed to release connect-link completion claim");
    }
}

async fn find_by_raw_token(db: &mongodb::Database, raw_token: &str) -> AppResult<ConnectLink> {
    validate_raw_token(raw_token)?;
    db.collection::<ConnectLink>(CONNECT_LINKS)
        .find_one(doc! { "token_hash": hash_token(raw_token) })
        .await?
        .ok_or(AppError::ConnectLinkNotFound)
}

async fn ensure_actor_can_manage(
    db: &mongodb::Database,
    actor_user_id: &str,
    link: &ConnectLink,
) -> AppResult<()> {
    let access = org_service::resolve_owner_access(db, actor_user_id, &link.user_id).await?;
    if access.can_write() {
        Ok(())
    } else {
        Err(AppError::ConnectLinkNotFound)
    }
}

fn ensure_pending(link: &ConnectLink) -> AppResult<()> {
    match link.status {
        ConnectLinkStatus::Pending => Ok(()),
        ConnectLinkStatus::Completed => Err(AppError::ConnectLinkAlreadyCompleted),
        ConnectLinkStatus::Expired => Err(AppError::ConnectLinkExpired),
        ConnectLinkStatus::Cancelled => Err(AppError::ConnectLinkCancelled),
    }
}

async fn claim_expiry(
    db: &mongodb::Database,
    link: ConnectLink,
    actor_user_id: Option<&str>,
) -> AppResult<ConnectLink> {
    if link.status != ConnectLinkStatus::Pending || link.expires_at > Utc::now() {
        return Ok(link);
    }
    let updated = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find_one_and_update(
            doc! {
                "_id": &link.id,
                "status": "pending",
                "completion_claim_id": null,
            },
            doc! {
                "$set": { "status": "expired" },
                "$unset": { "completion_claim_id": "" },
            },
        )
        .return_document(ReturnDocument::After)
        .await?;
    if let Some(ref updated) = updated {
        audit_service::log_async(
            db.clone(),
            Some(actor_user_id.unwrap_or(&updated.user_id).to_string()),
            "connect_link_expired-on-claim".to_string(),
            Some(serde_json::json!({
                "connect_link_id": &updated.id,
                "service_id": &updated.service_id,
                "service_slug": &updated.service_slug,
            })),
            None,
            None,
            None,
            None,
        );
    }
    match updated {
        Some(updated) => Ok(updated),
        None => db
            .collection::<ConnectLink>(CONNECT_LINKS)
            .find_one(doc! { "_id": &link.id })
            .await?
            .ok_or(AppError::ConnectLinkNotFound),
    }
}

async fn view_for_link(db: &mongodb::Database, link: ConnectLink) -> AppResult<LinkView> {
    let service = load_catalog_info_by_id(db, &link.service_id).await?;
    let completed_service_slug = match link.completed_user_service_id.as_deref() {
        Some(id) if link.status == ConnectLinkStatus::Completed => db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": id, "user_id": &link.user_id })
            .await?
            .map(|service| service.slug),
        _ => None,
    };
    Ok(LinkView {
        link,
        service,
        completed_service_slug,
    })
}

async fn load_catalog_info_by_slug(
    db: &mongodb::Database,
    slug: &str,
) -> AppResult<CatalogConnectInfo> {
    let slug = slug.trim();
    if slug.is_empty() {
        return Err(AppError::ValidationError(
            "service_slug must not be empty".to_string(),
        ));
    }
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "slug": slug, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Catalog service '{slug}' not found")))?;
    catalog_info(db, service).await
}

async fn load_catalog_info_by_id(
    db: &mongodb::Database,
    service_id: &str,
) -> AppResult<CatalogConnectInfo> {
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": service_id })
        .await?
        .ok_or(AppError::ConnectLinkNotFound)?;
    catalog_info(db, service).await
}

async fn catalog_info(
    db: &mongodb::Database,
    service: DownstreamService,
) -> AppResult<CatalogConnectInfo> {
    if service.service_type == "ssh" {
        return Err(AppError::BadRequest(
            "SSH services cannot be connected through a hosted connect link".to_string(),
        ));
    }
    let provider = match service.provider_config_id.as_deref() {
        Some(provider_id) => {
            db.collection::<ProviderConfig>(PROVIDERS)
                .find_one(doc! { "_id": provider_id, "is_active": true })
                .await?
        }
        None => None,
    };
    Ok(CatalogConnectInfo {
        service_id: service.id,
        service_slug: service.slug,
        service_name: service.name,
        auth_method: service.auth_method,
        auth_key_name: service.auth_key_name,
        provider_id: provider.as_ref().map(|provider| provider.id.clone()),
        provider_type: provider
            .as_ref()
            .map(|provider| provider.provider_type.clone()),
        credential_mode: provider
            .as_ref()
            .map(|provider| provider.credential_mode.clone()),
        has_platform_oauth_credentials: provider.as_ref().is_some_and(|provider| {
            crate::services::user_credentials_service::provider_has_admin_oauth_credentials(
                provider,
            )
        }),
        requires_gateway_url: provider
            .as_ref()
            .is_some_and(|provider| provider.requires_gateway_url),
        api_key_url: provider
            .as_ref()
            .and_then(|provider| provider.api_key_url.clone()),
        api_key_instructions: provider
            .as_ref()
            .and_then(|provider| provider.api_key_instructions.clone()),
    })
}

fn normalize_optional(
    value: Option<String>,
    max_len: usize,
    field: &str,
) -> AppResult<Option<String>> {
    let Some(value) = value else {
        return Ok(None);
    };
    let value = value.trim();
    if value.is_empty() {
        return Ok(None);
    }
    if value.len() > max_len {
        return Err(AppError::ValidationError(format!(
            "{field} must be at most {max_len} characters"
        )));
    }
    Ok(Some(value.to_string()))
}

pub fn validate_callback_url(raw: Option<&str>) -> AppResult<Option<String>> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok(None);
    }
    if raw.len() > MAX_CALLBACK_URL_LEN {
        return Err(AppError::ValidationError(
            "callback_url is too long".to_string(),
        ));
    }
    let parsed = url::Url::parse(raw).map_err(|_| {
        AppError::ValidationError("callback_url must be an absolute HTTP(S) URL".to_string())
    })?;
    if !matches!(parsed.scheme(), "http" | "https")
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(AppError::ValidationError(
            "callback_url must be an absolute HTTP(S) URL without userinfo or a fragment"
                .to_string(),
        ));
    }
    Ok(Some(parsed.to_string()))
}

fn validate_raw_token(raw_token: &str) -> AppResult<()> {
    let Some(secret) = raw_token.strip_prefix(CONNECT_LINK_PREFIX) else {
        return Err(AppError::ConnectLinkNotFound);
    };
    if secret.len() != 64 || !secret.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::ConnectLinkNotFound);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::test_helpers::dummy_service;
    use crate::test_utils::connect_test_database;

    async fn insert_catalog_service(db: &mongodb::Database, suffix: &str) -> DownstreamService {
        let mut service = dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.slug = format!("connect-link-{suffix}-{}", Uuid::new_v4());
        service.name = format!("Connect Link {suffix}");
        service.auth_method = "bearer".to_string();
        service.auth_key_name = "Authorization".to_string();
        service.requires_user_credential = true;
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&service)
            .await
            .expect("insert catalog service");
        service
    }

    #[test]
    fn callback_url_accepts_absolute_http_and_https() {
        assert!(validate_callback_url(Some("https://agent.example/callback?run=1")).is_ok());
        assert!(validate_callback_url(Some("http://localhost:4400/done")).is_ok());
    }

    #[test]
    fn callback_url_rejects_unsafe_shapes() {
        for value in [
            "/relative",
            "ftp://example.com/callback",
            "https://user:pass@example.com/callback",
            "https://example.com/callback#token",
        ] {
            assert!(
                validate_callback_url(Some(value)).is_err(),
                "accepted {value}"
            );
        }
    }

    #[test]
    fn raw_token_requires_prefix_and_32_hex_bytes() {
        let valid = format!("{CONNECT_LINK_PREFIX}{}", "ab".repeat(32));
        assert!(validate_raw_token(&valid).is_ok());
        assert!(validate_raw_token("nyx_clk_short").is_err());
        assert!(validate_raw_token(&format!("{CONNECT_LINK_PREFIX}{}", "zz".repeat(32))).is_err());
    }

    #[test]
    fn ttl_bounds_match_hosted_link_contract() {
        assert_eq!(DEFAULT_TTL_SECS, 900);
        assert_eq!(MIN_TTL_SECS, 60);
        assert_eq!(MAX_TTL_SECS, 3600);
        assert_eq!(MAX_WAIT_SECS, 120);
    }

    #[test]
    fn connect_url_contains_single_use_token_in_path() {
        let token = format!("nyx_clk_{}", "ab".repeat(32));
        let url = build_connect_url("https://app.example.test/", &token).expect("connect URL");
        assert_eq!(url, format!("https://app.example.test/connect/{token}"));
    }

    #[test]
    fn secret_bearing_service_inputs_redact_debug_output() {
        let input = CompleteInput {
            credential: Some("api-secret"),
            endpoint_url: Some("https://gateway.example.test"),
            oauth_client_id: Some("client-id"),
            oauth_client_secret: Some("client-secret"),
        };
        let debug = format!("{input:?}");
        for secret in ["api-secret", "client-id", "client-secret"] {
            assert!(!debug.contains(secret));
        }
    }

    #[tokio::test]
    async fn create_persists_only_token_hash_and_preview_is_non_mutating() {
        let Some(db) = connect_test_database("connect_link_create_preview").await else {
            return;
        };
        let service = insert_catalog_service(&db, "preview").await;
        let owner = Uuid::new_v4().to_string();
        let created = create(
            &db,
            CreateInput {
                user_id: owner,
                service_slug: service.slug.clone(),
                label: Some("Production".to_string()),
                requested_by: Some("test-agent".to_string()),
                callback_url: None,
                ttl_secs: None,
            },
        )
        .await
        .expect("create link");

        assert!(created.raw_token.starts_with(CONNECT_LINK_PREFIX));
        let stored = db
            .collection::<ConnectLink>(CONNECT_LINKS)
            .find_one(doc! { "_id": &created.link.id })
            .await
            .expect("query link")
            .expect("stored link");
        assert_eq!(stored.token_hash, hash_token(&created.raw_token));
        assert!(!stored.token_hash.contains(&created.raw_token));

        let view = preview(&db, &created.raw_token)
            .await
            .expect("preview link");
        assert_eq!(view.link.status, ConnectLinkStatus::Pending);
        assert_eq!(view.link.label.as_deref(), Some("Production"));
        assert_eq!(view.service.service_slug, service.slug);
    }

    #[tokio::test]
    async fn cancel_is_creator_scoped_and_terminal() {
        let Some(db) = connect_test_database("connect_link_cancel").await else {
            return;
        };
        let service = insert_catalog_service(&db, "cancel").await;
        let owner = Uuid::new_v4().to_string();
        let created = create(
            &db,
            CreateInput {
                user_id: owner.clone(),
                service_slug: service.slug,
                label: None,
                requested_by: None,
                callback_url: None,
                ttl_secs: None,
            },
        )
        .await
        .expect("create link");

        let denied = get_for_actor(&db, &Uuid::new_v4().to_string(), &created.link.id).await;
        assert!(matches!(denied, Err(AppError::ConnectLinkNotFound)));
        let cancelled = cancel(&db, &owner, &created.link.id)
            .await
            .expect("cancel link");
        assert_eq!(cancelled.link.status, ConnectLinkStatus::Cancelled);
        assert!(matches!(
            complete(
                &db,
                &crate::test_utils::test_encryption_keys(),
                &owner,
                &created.raw_token,
                CompleteInput::default(),
                false,
            )
            .await,
            Err(AppError::ConnectLinkCancelled)
        ));
    }

    #[tokio::test]
    async fn expired_link_is_claimed_once_and_stays_observable() {
        let Some(db) = connect_test_database("connect_link_expiry").await else {
            return;
        };
        let service = insert_catalog_service(&db, "expiry").await;
        let owner = Uuid::new_v4().to_string();
        let created = create(
            &db,
            CreateInput {
                user_id: owner.clone(),
                service_slug: service.slug,
                label: None,
                requested_by: None,
                callback_url: None,
                ttl_secs: None,
            },
        )
        .await
        .expect("create link");
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": { "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)) } },
            )
            .await
            .expect("expire link");

        let first = get_for_actor(&db, &owner, &created.link.id)
            .await
            .expect("claim expiry");
        let second = get_for_actor(&db, &owner, &created.link.id)
            .await
            .expect("read expired link");
        assert_eq!(first.link.status, ConnectLinkStatus::Expired);
        assert_eq!(second.link.status, ConnectLinkStatus::Expired);
    }

    #[tokio::test]
    async fn expiry_read_does_not_steal_an_active_completion_claim() {
        let Some(db) = connect_test_database("connect_link_expiry_active_claim").await else {
            return;
        };
        let service = insert_catalog_service(&db, "expiry-active-claim").await;
        let owner = Uuid::new_v4().to_string();
        let created = create(
            &db,
            CreateInput {
                user_id: owner.clone(),
                service_slug: service.slug,
                label: None,
                requested_by: None,
                callback_url: None,
                ttl_secs: None,
            },
        )
        .await
        .expect("create link");
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": {
                    "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)),
                    "completion_claim_id": "active-claim",
                } },
            )
            .await
            .expect("expire claimed link");

        let view = get_for_actor(&db, &owner, &created.link.id)
            .await
            .expect("read claimed link");
        assert_eq!(view.link.status, ConnectLinkStatus::Pending);
        assert_eq!(
            view.link.completion_claim_id.as_deref(),
            Some("active-claim")
        );
    }
}
