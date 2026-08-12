use chrono::{Duration, Utc};
use futures::TryStreamExt;
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
use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDERS, ProviderConfig};
use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::redaction::RedactedLen;
use crate::services::{audit_service, oauth_service, org_service, unified_key_service};

pub const CONNECT_LINK_PREFIX: &str = "nyx_clk_";
pub const DEFAULT_TTL_SECS: i64 = 15 * 60;
pub const MIN_TTL_SECS: i64 = 60;
pub const MAX_TTL_SECS: i64 = 60 * 60;
pub const MAX_WAIT_SECS: u64 = 120;
pub const CLAIM_STALE_SECS: i64 = 5 * 60;
pub const PINNED_GRACE_SECS: i64 = 30 * 60;
pub const WEBHOOK_REDISPATCH_STALE_SECS: i64 = 120;
pub const WEBHOOK_MAX_DISPATCH_CYCLES: u32 = 5;

const MAX_LABEL_LEN: usize = 200;
const MAX_REQUESTED_BY_LEN: usize = 200;
const MAX_CALLBACK_URL_LEN: usize = 2048;
const MAX_LAST_ERROR_LEN: usize = 100;

pub struct CreateInput {
    pub user_id: String,
    pub service_slug: String,
    pub label: Option<String>,
    pub requested_by: Option<String>,
    pub callback_url: Option<String>,
    pub ttl_secs: Option<i64>,
    pub oauth_client_id: Option<String>,
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
    let supplied_requested_by =
        normalize_optional(input.requested_by, MAX_REQUESTED_BY_LEN, "requested_by")?;
    let (callback_url, requesting_app) = resolve_requesting_app(
        db,
        input.oauth_client_id.as_deref(),
        input.callback_url.as_deref(),
    )
    .await?;
    let requested_by = requesting_app
        .as_ref()
        .map(|client| client.client_name.clone())
        .or(supplied_requested_by);
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
        requesting_app_id: requesting_app.as_ref().map(|client| client.id.clone()),
        requesting_app_name: requesting_app
            .as_ref()
            .map(|client| client.client_name.clone()),
        token_hash: hash_token(&raw_token),
        status: ConnectLinkStatus::Pending,
        callback_url,
        created_at: now,
        completed_at: None,
        expires_at: now + Duration::seconds(ttl_secs),
        completed_user_service_id: None,
        completion_claim_id: None,
        completion_claim_at: None,
        last_error: None,
        last_error_at: None,
        webhook_event_reserved_at: None,
        webhook_event_id: None,
        webhook_event_status: None,
        webhook_event_attempts: 0,
        webhook_event_delivered_at: None,
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
    let current = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find_one(doc! { "_id": link_id })
        .await?
        .ok_or(AppError::ConnectLinkNotFound)?;
    ensure_actor_can_manage(db, actor_user_id, &current).await?;
    let current = claim_expiry(db, current, Some(actor_user_id)).await?;
    match current.status {
        ConnectLinkStatus::Expired => return Err(AppError::ConnectLinkExpired),
        ConnectLinkStatus::Completed => return Err(AppError::ConnectLinkAlreadyCompleted),
        ConnectLinkStatus::Cancelled => return view_for_link(db, current).await,
        ConnectLinkStatus::Pending => {}
    }

    let mut filter = doc! {
        "_id": link_id,
        "status": "pending",
    };
    filter.extend(claim_available_filter(Utc::now()));
    let updated = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find_one_and_update(
            filter,
            doc! {
                "$set": { "status": "cancelled" },
                "$unset": {
                    "completion_claim_id": "",
                    "completion_claim_at": "",
                    "last_error": "",
                    "last_error_at": "",
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?;
    let Some(updated) = updated else {
        return Err(completion_conflict_error(db, link_id).await?);
    };

    view_for_link(db, updated).await
}

pub async fn cancel_by_token(
    db: &mongodb::Database,
    actor_user_id: &str,
    raw_token: &str,
) -> AppResult<LinkView> {
    let link = find_by_raw_token(db, raw_token).await?;
    ensure_actor_can_manage(db, actor_user_id, &link).await?;
    cancel(db, actor_user_id, &link.id).await
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
    let claim_at = Utc::now();
    let mut claim_filter = doc! {
        "_id": &current.id,
        "status": "pending",
        "completed_user_service_id": null,
        "expires_at": { "$gt": bson::DateTime::from_chrono(claim_at) },
    };
    claim_filter.extend(claim_available_filter(claim_at));

    // A stale takeover can duplicate provisioning if the prior process created
    // a service and died before pinning its id. Re-provisioning is intentional:
    // takeover is allowed only while completed_user_service_id remains null.
    let claimed = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find_one_and_update(
            claim_filter,
            doc! { "$set": {
                "completion_claim_id": &claim_id,
                "completion_claim_at": bson::DateTime::from_chrono(claim_at),
            } },
        )
        .return_document(ReturnDocument::After)
        .await?;
    let Some(claimed) = claimed else {
        return Err(completion_conflict_error(db, &current.id).await?);
    };

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
    if let Some(app_id) = claimed.requesting_app_id.as_deref()
        && let Err(error) = crate::services::user_service_service::set_source_app_id(
            db,
            &claimed.user_id,
            &service_id,
            app_id,
            &claimed.id,
        )
        .await
    {
        tracing::warn!(
            connect_link_id = %claimed.id,
            user_service_id = %service_id,
            requesting_app_id = %app_id,
            %error,
            "failed to record connect-link service provenance"
        );
    }
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
                "$unset": {
                    "completion_claim_id": "",
                    "completion_claim_at": "",
                    "last_error": "",
                    "last_error_at": "",
                },
            },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or(AppError::ConnectLinkAlreadyCompleted)?;
    view_for_link(db, updated).await
}

pub async fn record_provider_error(
    db: &mongodb::Database,
    connect_link_id: &str,
    owner_user_id: &str,
    error: &str,
) -> AppResult<bool> {
    if error.is_empty()
        || error.len() > MAX_LAST_ERROR_LEN
        || !error
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    {
        return Err(AppError::ValidationError(
            "connect-link provider error code is invalid".to_string(),
        ));
    }
    let result = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .update_one(
            doc! {
                "_id": connect_link_id,
                "user_id": owner_user_id,
                "status": "pending",
            },
            doc! { "$set": {
                "last_error": error,
                "last_error_at": bson::DateTime::from_chrono(Utc::now()),
            } },
        )
        .await?;
    Ok(result.modified_count == 1)
}

pub async fn wait_for_status(
    db: &mongodb::Database,
    actor_user_id: &str,
    link_id: &str,
    timeout_secs: u64,
) -> AppResult<LinkView> {
    let timeout_secs = timeout_secs.clamp(1, MAX_WAIT_SECS);
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(timeout_secs);
    let collection = db.collection::<ConnectLink>(CONNECT_LINKS);
    let mut link = collection
        .find_one(doc! { "_id": link_id })
        .await?
        .ok_or(AppError::ConnectLinkNotFound)?;
    ensure_actor_can_manage(db, actor_user_id, &link).await?;

    loop {
        link = claim_expiry(db, link, Some(actor_user_id)).await?;
        if link.status != ConnectLinkStatus::Pending || tokio::time::Instant::now() >= deadline {
            return view_for_link(db, link).await;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        link = collection
            .find_one(doc! { "_id": link_id })
            .await?
            .ok_or(AppError::ConnectLinkNotFound)?;
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
                    "$unset": {
                        "completion_claim_id": "",
                        "completion_claim_at": "",
                        "last_error": "",
                        "last_error_at": "",
                    },
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
                "$unset": {
                    "completion_claim_id": "",
                    "completion_claim_at": "",
                    "last_error": "",
                    "last_error_at": "",
                },
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
                "$unset": {
                    "completion_claim_id": "",
                    "completion_claim_at": "",
                    "last_error": "",
                    "last_error_at": "",
                },
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
            doc! { "$unset": {
                "completion_claim_id": "",
                "completion_claim_at": "",
            } },
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

fn claim_available_filter(now: chrono::DateTime<Utc>) -> bson::Document {
    let stale_before = now - Duration::seconds(CLAIM_STALE_SECS);
    doc! {
        "$or": [
            { "completion_claim_id": null },
            { "completion_claim_at": null },
            { "completion_claim_at": { "$lte": bson::DateTime::from_chrono(stale_before) } },
        ]
    }
}

async fn completion_conflict_error(db: &mongodb::Database, link_id: &str) -> AppResult<AppError> {
    let link = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find_one(doc! { "_id": link_id })
        .await?
        .ok_or(AppError::ConnectLinkNotFound)?;
    Ok(match link.status {
        ConnectLinkStatus::Completed => AppError::ConnectLinkAlreadyCompleted,
        ConnectLinkStatus::Expired => AppError::ConnectLinkExpired,
        ConnectLinkStatus::Cancelled => AppError::ConnectLinkCancelled,
        ConnectLinkStatus::Pending => AppError::ConnectLinkCompletionInProgress,
    })
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
    let now = Utc::now();
    let expires_at = if link.completed_user_service_id.is_some() {
        link.expires_at + Duration::seconds(PINNED_GRACE_SECS)
    } else {
        link.expires_at
    };
    if link.status != ConnectLinkStatus::Pending || expires_at > now {
        return Ok(link);
    }
    let expiry_filter = doc! {
        "$or": [
            {
                "completed_user_service_id": null,
                "expires_at": { "$lte": bson::DateTime::from_chrono(now) },
            },
            {
                "completed_user_service_id": { "$ne": null },
                "expires_at": {
                    "$lte": bson::DateTime::from_chrono(
                        now - Duration::seconds(PINNED_GRACE_SECS)
                    ),
                },
            },
        ]
    };
    let filter = doc! {
        "_id": &link.id,
        "status": "pending",
        "$and": [expiry_filter, claim_available_filter(now)],
    };
    let updated = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find_one_and_update(
            filter,
            doc! {
                "$set": { "status": "expired" },
                "$unset": {
                    "completion_claim_id": "",
                    "completion_claim_at": "",
                },
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

/// Expire abandoned app-bound links without requiring their creator to poll.
/// The per-link claim and terminal-event reservation keep concurrent sweeps
/// idempotent across server instances.
pub async fn expire_pending_app_links(
    db: &mongodb::Database,
    dispatcher: &crate::services::developer_webhook_service::DeveloperWebhookDispatcher,
) -> AppResult<u64> {
    let now = Utc::now();
    let links: Vec<ConnectLink> = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find(doc! {
            "status": "pending",
            "requesting_app_id": { "$type": "string" },
            "$or": [
                {
                    "completed_user_service_id": null,
                    "expires_at": { "$lte": bson::DateTime::from_chrono(now) },
                },
                {
                    "completed_user_service_id": { "$ne": null },
                    "expires_at": {
                        "$lte": bson::DateTime::from_chrono(
                            now - Duration::seconds(PINNED_GRACE_SECS)
                        ),
                    },
                },
            ],
        })
        .await?
        .try_collect()
        .await?;

    let mut expired = 0;
    for link in links {
        let link_id = link.id.clone();
        let updated = claim_expiry(db, link, None).await?;
        if updated.status == ConnectLinkStatus::Expired {
            dispatch_terminal_webhook_if_needed(db, dispatcher, &link_id).await;
            expired += 1;
        }
    }
    redispatch_terminal_webhooks(db, dispatcher).await?;
    Ok(expired)
}

/// Atomically reserve the single terminal lifecycle event and begin its first
/// delivery cycle. The link document remains the durable outbox until the
/// event is delivered or exhausts the bounded redispatch cycles.
pub async fn dispatch_terminal_webhook_if_needed(
    db: &mongodb::Database,
    dispatcher: &crate::services::developer_webhook_service::DeveloperWebhookDispatcher,
    link_id: &str,
) {
    let event_id = Uuid::new_v4().to_string();
    let now = Utc::now();
    let claimed = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find_one_and_update(
            doc! {
                "_id": link_id,
                "requesting_app_id": { "$type": "string" },
                "status": { "$in": ["completed", "cancelled", "expired"] },
                "$or": [
                    { "webhook_event_reserved_at": null },
                    { "webhook_event_reserved_at": { "$exists": false } },
                ],
            },
            doc! { "$set": {
                "webhook_event_reserved_at": bson::DateTime::from_chrono(now),
                "webhook_event_id": &event_id,
                "webhook_event_status": "pending",
                "webhook_event_attempts": 1_i32,
            }},
        )
        .return_document(ReturnDocument::After)
        .await;
    let link = match claimed {
        Ok(Some(link)) => link,
        Ok(None) => return,
        Err(error) => {
            tracing::warn!(link_id, %error, "failed to reserve connect-link webhook event");
            return;
        }
    };
    spawn_terminal_webhook_delivery(db.clone(), dispatcher.clone(), link);
}

/// Reclaim stale outbox reservations. A compare-and-swap on the cycle count
/// keeps concurrent sweepers from dispatching the same cycle.
pub async fn redispatch_terminal_webhooks(
    db: &mongodb::Database,
    dispatcher: &crate::services::developer_webhook_service::DeveloperWebhookDispatcher,
) -> AppResult<u64> {
    let stale_before = Utc::now() - Duration::seconds(WEBHOOK_REDISPATCH_STALE_SECS);
    let candidates: Vec<ConnectLink> = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find(doc! {
            "requesting_app_id": { "$type": "string" },
            "status": { "$in": ["completed", "cancelled", "expired"] },
            "webhook_event_status": "pending",
            "webhook_event_delivered_at": null,
            "webhook_event_reserved_at": {
                "$lte": bson::DateTime::from_chrono(stale_before),
            },
            "webhook_event_attempts": { "$lt": WEBHOOK_MAX_DISPATCH_CYCLES as i64 },
        })
        .await?
        .try_collect()
        .await?;
    let mut claimed_count = 0;
    for candidate in candidates {
        let claimed = db
            .collection::<ConnectLink>(CONNECT_LINKS)
            .find_one_and_update(
                doc! {
                    "_id": &candidate.id,
                    "webhook_event_status": "pending",
                    "webhook_event_attempts": candidate.webhook_event_attempts as i64,
                    "webhook_event_reserved_at": {
                        "$lte": bson::DateTime::from_chrono(stale_before),
                    },
                },
                doc! {
                    "$set": {
                        "webhook_event_reserved_at": bson::DateTime::from_chrono(Utc::now()),
                    },
                    "$inc": { "webhook_event_attempts": 1_i32 },
                },
            )
            .return_document(ReturnDocument::After)
            .await?;
        if let Some(link) = claimed {
            claimed_count += 1;
            spawn_terminal_webhook_delivery(db.clone(), dispatcher.clone(), link);
        }
    }
    abandon_stale_exhausted_webhooks(db).await?;
    Ok(claimed_count)
}

async fn abandon_stale_exhausted_webhooks(db: &mongodb::Database) -> AppResult<()> {
    let stale_before = Utc::now() - Duration::seconds(WEBHOOK_REDISPATCH_STALE_SECS);
    let exhausted: Vec<ConnectLink> = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .find(doc! {
            "webhook_event_status": "pending",
            "webhook_event_attempts": { "$gte": WEBHOOK_MAX_DISPATCH_CYCLES as i64 },
            "webhook_event_reserved_at": {
                "$lte": bson::DateTime::from_chrono(stale_before),
            },
        })
        .await?
        .try_collect()
        .await?;
    for link in exhausted {
        abandon_terminal_webhook(
            db,
            &link,
            crate::services::webhook_delivery_service::DeliveryFailure {
                attempts: 0,
                reason: "dispatch_cycle_cap_reached",
                last_status: None,
            },
        )
        .await;
    }
    Ok(())
}

fn spawn_terminal_webhook_delivery(
    db: mongodb::Database,
    dispatcher: crate::services::developer_webhook_service::DeveloperWebhookDispatcher,
    link: ConnectLink,
) {
    tokio::spawn(async move {
        let (Some(app_id), Some(event_id), Some(event_type)) = (
            link.requesting_app_id.as_deref(),
            link.webhook_event_id.as_deref(),
            terminal_webhook_event_type(link.status),
        ) else {
            return;
        };
        let status = event_type.trim_start_matches("connect_link.");
        let result = dispatcher
            .deliver_for_app(
                &db,
                app_id,
                event_id,
                event_type,
                serde_json::json!({
                    "user_id": &link.user_id,
                    "connect_link_id": &link.id,
                    "service_id": &link.service_id,
                    "service_slug": &link.service_slug,
                    "status": status,
                    "user_service_id": &link.completed_user_service_id,
                    "completed_at": &link.completed_at,
                    "expires_at": &link.expires_at,
                }),
            )
            .await;
        match result {
            Ok(()) => {
                if let Err(error) = db
                    .collection::<ConnectLink>(CONNECT_LINKS)
                    .update_one(
                        doc! {
                            "_id": &link.id,
                            "webhook_event_id": event_id,
                            "webhook_event_status": "pending",
                        },
                        doc! { "$set": {
                            "webhook_event_status": "delivered",
                            "webhook_event_delivered_at": bson::DateTime::from_chrono(Utc::now()),
                        }},
                    )
                    .await
                {
                    tracing::warn!(connect_link_id = %link.id, %error, "failed to mark terminal webhook delivered");
                }
            }
            Err(failure) if link.webhook_event_attempts >= WEBHOOK_MAX_DISPATCH_CYCLES => {
                abandon_terminal_webhook(&db, &link, failure).await;
            }
            Err(failure) => {
                tracing::warn!(
                    connect_link_id = %link.id,
                    event_id,
                    cycle = link.webhook_event_attempts,
                    reason = failure.reason,
                    "terminal webhook cycle failed; outbox remains pending"
                );
            }
        }
    });
}

async fn abandon_terminal_webhook(
    db: &mongodb::Database,
    link: &ConnectLink,
    failure: crate::services::webhook_delivery_service::DeliveryFailure,
) {
    let Some(event_id) = link.webhook_event_id.as_deref() else {
        return;
    };
    let updated = db
        .collection::<ConnectLink>(CONNECT_LINKS)
        .update_one(
            doc! {
                "_id": &link.id,
                "webhook_event_id": event_id,
                "webhook_event_status": "pending",
            },
            doc! { "$set": { "webhook_event_status": "abandoned" } },
        )
        .await;
    if !matches!(updated, Ok(result) if result.modified_count == 1) {
        return;
    }
    if let (Some(app_id), Some(event_type)) = (
        link.requesting_app_id.as_deref(),
        terminal_webhook_event_type(link.status),
    ) {
        crate::services::developer_webhook_service::record_terminal_delivery_failure(
            db, app_id, event_id, event_type, failure,
        )
        .await;
    }
}

fn terminal_webhook_event_type(status: ConnectLinkStatus) -> Option<&'static str> {
    match status {
        ConnectLinkStatus::Completed => Some("connect_link.completed"),
        ConnectLinkStatus::Cancelled => Some("connect_link.cancelled"),
        ConnectLinkStatus::Expired => Some("connect_link.expired"),
        ConnectLinkStatus::Pending => None,
    }
}

pub async fn dispatch_terminal_webhook_by_token_if_needed(
    db: &mongodb::Database,
    dispatcher: &crate::services::developer_webhook_service::DeveloperWebhookDispatcher,
    raw_token: &str,
) {
    let Ok(link) = find_by_raw_token(db, raw_token).await else {
        return;
    };
    dispatch_terminal_webhook_if_needed(db, dispatcher, &link.id).await;
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

async fn resolve_requesting_app(
    db: &mongodb::Database,
    oauth_client_id: Option<&str>,
    callback_url: Option<&str>,
) -> AppResult<(Option<String>, Option<OauthClient>)> {
    let Some(client_id) = oauth_client_id else {
        return Ok((validate_callback_url(callback_url)?, None));
    };

    let callback_url = validate_app_callback_url(callback_url)?;
    let client = match callback_url.as_deref() {
        Some(callback_url) => oauth_service::validate_client(db, client_id, callback_url).await?,
        None => db
            .collection::<OauthClient>(OAUTH_CLIENTS)
            .find_one(doc! { "_id": client_id, "is_active": true })
            .await?
            .ok_or_else(|| AppError::NotFound("OAuth client not found".to_string()))?,
    };
    Ok((callback_url, Some(client)))
}

fn validate_app_callback_url(raw: Option<&str>) -> AppResult<Option<String>> {
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
    let parsed = url::Url::parse(raw)
        .map_err(|_| AppError::ValidationError("callback_url must be absolute".to_string()))?;
    if matches!(parsed.scheme(), "javascript" | "data" | "file")
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || parsed.fragment().is_some()
    {
        return Err(AppError::ValidationError(
            "callback_url must not contain userinfo or a fragment".to_string(),
        ));
    }
    Ok(Some(raw.to_string()))
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

pub fn terminal_callback_url(link: &ConnectLink) -> AppResult<Option<String>> {
    let status = match link.status {
        ConnectLinkStatus::Pending => return Ok(None),
        ConnectLinkStatus::Completed => "completed",
        ConnectLinkStatus::Expired => "expired",
        ConnectLinkStatus::Cancelled => "cancelled",
    };
    let Some(callback_url) = link.callback_url.as_deref() else {
        return Ok(None);
    };
    let mut parsed = url::Url::parse(callback_url).map_err(|_| {
        AppError::Internal("stored connect-link callback URL is invalid".to_string())
    })?;
    let existing_pairs: Vec<(String, String)> = parsed
        .query_pairs()
        .filter(|(key, _)| key != "status" && key != "connect_link_id")
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    parsed.set_query(None);
    parsed.set_fragment(None);
    {
        let mut query = parsed.query_pairs_mut();
        query.extend_pairs(existing_pairs.iter().map(|(key, value)| (key, value)));
        query.append_pair("status", status);
        query.append_pair("connect_link_id", &link.id);
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

    async fn insert_oauth_client(
        db: &mongodb::Database,
        name: &str,
        redirect_uris: Vec<String>,
    ) -> OauthClient {
        let now = Utc::now();
        let client = OauthClient {
            id: Uuid::new_v4().to_string(),
            client_name: name.to_string(),
            client_secret_hash: String::new(),
            redirect_uris,
            allowed_scopes: "openid profile".to_string(),
            scope_provenance: Default::default(),
            grant_types: "authorization_code refresh_token".to_string(),
            client_type: "public".to_string(),
            is_active: true,
            delegation_scopes: String::new(),
            default_service_catalog_slugs: Vec::new(),
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            connection_webhook_url: None,
            connection_webhook_secret_encrypted: None,
            connection_webhook_key_id: None,
            connection_webhook_enabled: false,
            created_by: Some("test".to_string()),
            created_at: now,
            updated_at: now,
        };
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(&client)
            .await
            .expect("insert OAuth client");
        client
    }

    async fn create_test_link(db: &mongodb::Database, suffix: &str) -> (String, CreatedLink) {
        let service = insert_catalog_service(db, suffix).await;
        let owner = Uuid::new_v4().to_string();
        let created = create(
            db,
            CreateInput {
                user_id: owner.clone(),
                service_slug: service.slug,
                label: Some(format!("Connect {suffix}")),
                requested_by: None,
                callback_url: None,
                ttl_secs: None,
                oauth_client_id: None,
            },
        )
        .await
        .expect("create test link");
        (owner, created)
    }

    async fn provision_test_link(
        db: &mongodb::Database,
        owner: &str,
        created: &CreatedLink,
    ) -> String {
        let result = complete(
            db,
            &crate::test_utils::test_encryption_keys(),
            owner,
            &created.raw_token,
            CompleteInput {
                credential: Some("test-secret"),
                ..CompleteInput::default()
            },
            false,
        )
        .await
        .expect("provision test link");
        match result {
            CompleteResult::Completed(view) => view
                .link
                .completed_user_service_id
                .expect("completed service id"),
            CompleteResult::OauthRequired { .. } | CompleteResult::DeviceCodeRequired { .. } => {
                panic!("API-key test service should complete immediately")
            }
        }
    }

    fn test_oauth_provider() -> ProviderConfig {
        ProviderConfig {
            id: Uuid::new_v4().to_string(),
            slug: format!("connect-link-oauth-{}", Uuid::new_v4()),
            name: "Connect Link OAuth".to_string(),
            description: None,
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://auth.example.test/authorize".to_string()),
            token_url: Some("https://auth.example.test/token".to_string()),
            revocation_url: None,
            revocation: None,
            default_scopes: None,
            client_id_encrypted: Some(vec![1, 2, 3]),
            client_secret_encrypted: Some(vec![4, 5, 6]),
            supports_pkce: true,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "test".to_string(),
            revocation_seed_version: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
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

    #[tokio::test]
    async fn app_callback_exact_match_uses_authenticated_app_identity() {
        let Some(db) = connect_test_database("connect_link_app_exact_callback").await else {
            return;
        };
        let service = insert_catalog_service(&db, "app-exact").await;
        let callback = "https://desktop.example.test/connect/return";
        let client = insert_oauth_client(&db, "Desktop App", vec![callback.to_string()]).await;
        let created = create(
            &db,
            CreateInput {
                user_id: Uuid::new_v4().to_string(),
                service_slug: service.slug,
                label: None,
                requested_by: Some("spoofed request name".to_string()),
                callback_url: Some(callback.to_string()),
                ttl_secs: None,
                oauth_client_id: Some(client.id.clone()),
            },
        )
        .await
        .expect("create app connect link");

        assert_eq!(created.link.callback_url.as_deref(), Some(callback));
        assert_eq!(
            created.link.requesting_app_id.as_deref(),
            Some(client.id.as_str())
        );
        assert_eq!(
            created.link.requesting_app_name.as_deref(),
            Some("Desktop App")
        );
        assert_eq!(created.link.requested_by.as_deref(), Some("Desktop App"));
    }

    #[tokio::test]
    async fn app_callback_accepts_registered_custom_scheme() {
        let Some(db) = connect_test_database("connect_link_app_custom_callback").await else {
            return;
        };
        let service = insert_catalog_service(&db, "app-custom").await;
        let callback = "desktop-app://connect/return";
        let client = insert_oauth_client(&db, "Native App", vec![callback.to_string()]).await;
        let created = create(
            &db,
            CreateInput {
                user_id: Uuid::new_v4().to_string(),
                service_slug: service.slug,
                label: None,
                requested_by: None,
                callback_url: Some(callback.to_string()),
                ttl_secs: None,
                oauth_client_id: Some(client.id),
            },
        )
        .await
        .expect("registered custom callback accepted");
        assert_eq!(created.link.callback_url.as_deref(), Some(callback));
    }

    #[tokio::test]
    async fn app_callback_rejects_unregistered_uri() {
        let Some(db) = connect_test_database("connect_link_app_callback_reject").await else {
            return;
        };
        let service = insert_catalog_service(&db, "app-reject").await;
        let client = insert_oauth_client(
            &db,
            "Desktop App",
            vec!["https://desktop.example.test/registered".to_string()],
        )
        .await;
        let result = create(
            &db,
            CreateInput {
                user_id: Uuid::new_v4().to_string(),
                service_slug: service.slug,
                label: None,
                requested_by: None,
                callback_url: Some("https://other.example.test/return".to_string()),
                ttl_secs: None,
                oauth_client_id: Some(client.id),
            },
        )
        .await;
        assert!(matches!(result, Err(AppError::InvalidRedirectUri)));
    }

    #[tokio::test]
    async fn terminal_callback_replaces_reserved_params_for_every_outcome() {
        let Some(db) = connect_test_database("connect_link_terminal_callbacks").await else {
            return;
        };
        let (_, created) = create_test_link(&db, "terminal-callbacks").await;
        let mut link = created.link;
        link.callback_url = Some(
            "https://desktop.example.test/return?flow=abc&status=old&connect_link_id=old"
                .to_string(),
        );

        assert!(
            terminal_callback_url(&link)
                .expect("pending callback")
                .is_none()
        );
        for (status, expected) in [
            (ConnectLinkStatus::Completed, "completed"),
            (ConnectLinkStatus::Cancelled, "cancelled"),
            (ConnectLinkStatus::Expired, "expired"),
        ] {
            link.status = status;
            let callback = terminal_callback_url(&link)
                .expect("build terminal callback")
                .expect("callback exists");
            let parsed = url::Url::parse(&callback).expect("parse terminal callback");
            let pairs: std::collections::HashMap<_, _> = parsed.query_pairs().collect();
            assert_eq!(pairs.get("flow").map(|value| value.as_ref()), Some("abc"));
            assert_eq!(
                pairs.get("status").map(|value| value.as_ref()),
                Some(expected)
            );
            assert_eq!(
                pairs.get("connect_link_id").map(|value| value.as_ref()),
                Some(link.id.as_str())
            );
            assert!(!callback.contains(CONNECT_LINK_PREFIX));
        }
    }

    #[tokio::test]
    async fn terminal_webhook_reservation_is_single_use() {
        let Some(db) = connect_test_database("connect_link_terminal_webhook_reservation").await
        else {
            return;
        };
        let (_, created) = create_test_link(&db, "terminal-webhook").await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<axum::body::Bytes>();
        let receiver = axum::Router::new().route(
            "/events",
            axum::routing::post(move |body: axum::body::Bytes| {
                let tx = tx.clone();
                async move {
                    tx.send(body).expect("capture terminal webhook");
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind terminal webhook receiver");
        let address = listener.local_addr().expect("terminal receiver address");
        tokio::spawn(async move {
            axum::serve(listener, receiver)
                .await
                .expect("serve terminal webhook receiver")
        });
        let client = insert_oauth_client(&db, "Webhook App", Vec::new()).await;
        let keys = std::sync::Arc::new(crate::test_utils::test_encryption_keys());
        let encrypted_secret = keys
            .encrypt(b"terminal-webhook-secret")
            .await
            .expect("encrypt terminal webhook secret");
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .update_one(
                doc! { "_id": &client.id },
                doc! { "$set": {
                    "connection_webhook_url": format!("http://{address}/events"),
                    "connection_webhook_secret_encrypted": bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: encrypted_secret,
                    },
                    "connection_webhook_enabled": true,
                }},
            )
            .await
            .expect("configure terminal webhook");
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": {
                    "status": "cancelled",
                    "requesting_app_id": &client.id,
                }},
            )
            .await
            .expect("make link terminal");
        let dispatcher =
            crate::services::developer_webhook_service::DeveloperWebhookDispatcher::new(
                reqwest::Client::new(),
                keys,
            );

        dispatch_terminal_webhook_if_needed(&db, &dispatcher, &created.link.id).await;
        let body = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
            .await
            .expect("terminal webhook delivery")
            .expect("terminal webhook body");
        let envelope: serde_json::Value =
            serde_json::from_slice(&body).expect("parse terminal webhook");
        assert_eq!(envelope["event_type"], "connect_link.cancelled");
        assert_eq!(envelope["data"]["user_id"], created.link.user_id);
        assert_eq!(envelope["data"]["connect_link_id"], created.link.id);
        assert_eq!(envelope["data"]["status"], "cancelled");

        dispatch_terminal_webhook_if_needed(&db, &dispatcher, &created.link.id).await;
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
                .await
                .is_err()
        );
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                let stored = db
                    .collection::<ConnectLink>(CONNECT_LINKS)
                    .find_one(doc! { "_id": &created.link.id })
                    .await
                    .unwrap()
                    .expect("terminal link");
                if stored.webhook_event_status
                    == Some(crate::models::connect_link::ConnectLinkWebhookStatus::Delivered)
                {
                    assert!(stored.webhook_event_delivered_at.is_some());
                    assert_eq!(stored.webhook_event_attempts, 1);
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("delivery state persisted");
    }

    #[tokio::test]
    async fn stale_reserved_terminal_webhook_is_redelivered_once() {
        let Some(db) = connect_test_database("connect_link_outbox_redispatch").await else {
            return;
        };
        let (_, created) = create_test_link(&db, "outbox-redispatch").await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<axum::body::Bytes>();
        let receiver = axum::Router::new().route(
            "/events",
            axum::routing::post(move |body: axum::body::Bytes| {
                let tx = tx.clone();
                async move {
                    tx.send(body).expect("capture redispatch");
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind redispatch receiver");
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, receiver).await.unwrap() });
        let client = insert_oauth_client(&db, "Outbox App", Vec::new()).await;
        let keys = std::sync::Arc::new(crate::test_utils::test_encryption_keys());
        let encrypted = keys.encrypt(b"outbox-secret").await.unwrap();
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .update_one(
                doc! { "_id": &client.id },
                doc! { "$set": {
                    "connection_webhook_url": format!("http://{address}/events"),
                    "connection_webhook_secret_encrypted": bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: encrypted,
                    },
                    "connection_webhook_key_id": "key_outbox",
                    "connection_webhook_enabled": true,
                }},
            )
            .await
            .unwrap();
        let event_id = Uuid::new_v4().to_string();
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": {
                    "status": "cancelled",
                    "requesting_app_id": &client.id,
                    "webhook_event_id": &event_id,
                    "webhook_event_status": "pending",
                    "webhook_event_attempts": 1_i32,
                    "webhook_event_reserved_at": bson::DateTime::from_chrono(
                        Utc::now() - Duration::seconds(WEBHOOK_REDISPATCH_STALE_SECS + 1)
                    ),
                }},
            )
            .await
            .unwrap();
        let dispatcher =
            crate::services::developer_webhook_service::DeveloperWebhookDispatcher::new(
                reqwest::Client::new(),
                keys,
            );

        assert_eq!(
            redispatch_terminal_webhooks(&db, &dispatcher)
                .await
                .unwrap(),
            1
        );
        let body = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
            .await
            .expect("redispatch received")
            .expect("redispatch body");
        let envelope: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(envelope["event_id"], event_id);
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        assert_eq!(
            redispatch_terminal_webhooks(&db, &dispatcher)
                .await
                .unwrap(),
            0
        );
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn stale_terminal_webhook_at_cycle_cap_is_abandoned_and_audited() {
        let Some(db) = connect_test_database("connect_link_outbox_cap").await else {
            return;
        };
        let (_, created) = create_test_link(&db, "outbox-cap").await;
        let client = insert_oauth_client(&db, "Capped Outbox App", Vec::new()).await;
        let event_id = Uuid::new_v4().to_string();
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": {
                    "status": "expired",
                    "requesting_app_id": &client.id,
                    "webhook_event_id": &event_id,
                    "webhook_event_status": "pending",
                    "webhook_event_attempts": WEBHOOK_MAX_DISPATCH_CYCLES as i64,
                    "webhook_event_reserved_at": bson::DateTime::from_chrono(
                        Utc::now() - Duration::seconds(WEBHOOK_REDISPATCH_STALE_SECS + 1)
                    ),
                }},
            )
            .await
            .unwrap();
        let dispatcher =
            crate::services::developer_webhook_service::DeveloperWebhookDispatcher::new(
                reqwest::Client::new(),
                std::sync::Arc::new(crate::test_utils::test_encryption_keys()),
            );

        assert_eq!(
            redispatch_terminal_webhooks(&db, &dispatcher)
                .await
                .unwrap(),
            0
        );
        let stored = db
            .collection::<ConnectLink>(CONNECT_LINKS)
            .find_one(doc! { "_id": &created.link.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.webhook_event_status,
            Some(crate::models::connect_link::ConnectLinkWebhookStatus::Abandoned)
        );
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            loop {
                if db
                    .collection::<crate::models::audit_log::AuditLog>(
                        crate::models::audit_log::COLLECTION_NAME,
                    )
                    .find_one(doc! {
                        "event_type": "connection_webhook_delivery_failed",
                        "event_data.event_id": &event_id,
                    })
                    .await
                    .unwrap()
                    .is_some()
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            }
        })
        .await
        .expect("abandonment audit persisted");
        assert_eq!(
            redispatch_terminal_webhooks(&db, &dispatcher)
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn expiry_sweep_expires_app_link_and_dispatches_once_without_polling() {
        let Some(db) = connect_test_database("connect_link_expiry_sweep_dispatch").await else {
            return;
        };
        let (_, created) = create_test_link(&db, "expiry-sweep-dispatch").await;
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<axum::body::Bytes>();
        let receiver = axum::Router::new().route(
            "/events",
            axum::routing::post(move |body: axum::body::Bytes| {
                let tx = tx.clone();
                async move {
                    tx.send(body).expect("capture expiry webhook");
                    axum::http::StatusCode::NO_CONTENT
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind expiry webhook receiver");
        let address = listener.local_addr().expect("expiry receiver address");
        tokio::spawn(async move {
            axum::serve(listener, receiver)
                .await
                .expect("serve expiry webhook receiver")
        });

        let client = insert_oauth_client(&db, "Expiry Webhook App", Vec::new()).await;
        let keys = std::sync::Arc::new(crate::test_utils::test_encryption_keys());
        let encrypted_secret = keys
            .encrypt(b"expiry-webhook-secret")
            .await
            .expect("encrypt expiry webhook secret");
        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .update_one(
                doc! { "_id": &client.id },
                doc! { "$set": {
                    "connection_webhook_url": format!("http://{address}/events"),
                    "connection_webhook_secret_encrypted": bson::Binary {
                        subtype: bson::spec::BinarySubtype::Generic,
                        bytes: encrypted_secret,
                    },
                    "connection_webhook_enabled": true,
                }},
            )
            .await
            .expect("configure expiry webhook");
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": {
                    "requesting_app_id": &client.id,
                    "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)),
                }},
            )
            .await
            .expect("make app link overdue");
        let dispatcher =
            crate::services::developer_webhook_service::DeveloperWebhookDispatcher::new(
                reqwest::Client::new(),
                keys,
            );

        assert_eq!(expire_pending_app_links(&db, &dispatcher).await.unwrap(), 1);
        let body = tokio::time::timeout(std::time::Duration::from_secs(3), rx.recv())
            .await
            .expect("expiry webhook delivery")
            .expect("expiry webhook body");
        let envelope: serde_json::Value =
            serde_json::from_slice(&body).expect("parse expiry webhook");
        assert_eq!(envelope["event_type"], "connect_link.expired");
        assert_eq!(envelope["data"]["connect_link_id"], created.link.id);

        assert_eq!(expire_pending_app_links(&db, &dispatcher).await.unwrap(), 0);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(100), rx.recv())
                .await
                .is_err()
        );
        let stored = db
            .collection::<ConnectLink>(CONNECT_LINKS)
            .find_one(doc! { "_id": &created.link.id })
            .await
            .unwrap()
            .expect("expired link");
        assert_eq!(stored.status, ConnectLinkStatus::Expired);
    }

    #[tokio::test]
    async fn expiry_sweep_ignores_non_app_links() {
        let Some(db) = connect_test_database("connect_link_expiry_sweep_non_app").await else {
            return;
        };
        let (_, created) = create_test_link(&db, "expiry-sweep-non-app").await;
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": {
                    "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)),
                }},
            )
            .await
            .expect("make first-party link overdue");
        let dispatcher =
            crate::services::developer_webhook_service::DeveloperWebhookDispatcher::new(
                reqwest::Client::new(),
                std::sync::Arc::new(crate::test_utils::test_encryption_keys()),
            );

        assert_eq!(expire_pending_app_links(&db, &dispatcher).await.unwrap(), 0);
        let stored = db
            .collection::<ConnectLink>(CONNECT_LINKS)
            .find_one(doc! { "_id": &created.link.id })
            .await
            .unwrap()
            .expect("first-party link");
        assert_eq!(stored.status, ConnectLinkStatus::Pending);
    }

    #[tokio::test]
    async fn expiry_sweep_preserves_pinned_link_within_grace() {
        let Some(db) = connect_test_database("connect_link_expiry_sweep_pinned_grace").await else {
            return;
        };
        let (_, created) = create_test_link(&db, "expiry-sweep-pinned-grace").await;
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": {
                    "requesting_app_id": "app-id",
                    "completed_user_service_id": "pinned-service-id",
                    "expires_at": bson::DateTime::from_chrono(
                        Utc::now() - Duration::seconds(PINNED_GRACE_SECS - 1)
                    ),
                }},
            )
            .await
            .expect("pin app link within grace");
        let dispatcher =
            crate::services::developer_webhook_service::DeveloperWebhookDispatcher::new(
                reqwest::Client::new(),
                std::sync::Arc::new(crate::test_utils::test_encryption_keys()),
            );

        assert_eq!(expire_pending_app_links(&db, &dispatcher).await.unwrap(), 0);
        let stored = db
            .collection::<ConnectLink>(CONNECT_LINKS)
            .find_one(doc! { "_id": &created.link.id })
            .await
            .unwrap()
            .expect("pinned link");
        assert_eq!(stored.status, ConnectLinkStatus::Pending);
    }

    #[tokio::test]
    async fn app_completion_stamps_connect_link_service_provenance() {
        let Some(db) = connect_test_database("connect_link_service_provenance").await else {
            return;
        };
        let app_id = Uuid::new_v4().to_string();
        let (owner, created) = create_test_link(&db, "provenance-success").await;
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": { "requesting_app_id": &app_id } },
            )
            .await
            .expect("bind requesting app");

        let completed = complete(
            &db,
            &crate::test_utils::test_encryption_keys(),
            &owner,
            &created.raw_token,
            CompleteInput {
                credential: Some("test-secret"),
                ..CompleteInput::default()
            },
            false,
        )
        .await
        .expect("complete app connect link");
        let CompleteResult::Completed(view) = completed else {
            panic!("API-key link should complete")
        };
        let service = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! {
                "_id": view.link.completed_user_service_id.expect("service id")
            })
            .await
            .unwrap()
            .expect("provisioned service");
        assert_eq!(service.source.as_deref(), Some("connect_link"));
        assert_eq!(service.source_id.as_deref(), Some(created.link.id.as_str()));
        assert_eq!(service.source_app_id.as_deref(), Some(app_id.as_str()));
    }

    #[tokio::test]
    async fn provenance_write_failure_does_not_abort_completed_link() {
        let Some(db) = connect_test_database("connect_link_provenance_failure_isolated").await
        else {
            return;
        };
        let app_id = Uuid::new_v4().to_string();
        let mut existing = crate::test_utils::test_user_service(
            &Uuid::new_v4().to_string(),
            &Uuid::new_v4().to_string(),
            "existing-provenance",
            &Uuid::new_v4().to_string(),
            None,
            None,
        );
        existing.source_app_id = Some(app_id.clone());
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&existing)
            .await
            .expect("insert existing provenance row");
        db.collection::<UserService>(USER_SERVICES)
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! { "source_app_id": 1 })
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .unique(true)
                            .partial_filter_expression(
                                doc! { "source_app_id": { "$type": "string" } },
                            )
                            .build(),
                    )
                    .build(),
            )
            .await
            .expect("create provenance failure index");

        let (owner, created) = create_test_link(&db, "provenance-failure").await;
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": { "requesting_app_id": &app_id } },
            )
            .await
            .expect("bind requesting app");

        let completed = complete(
            &db,
            &crate::test_utils::test_encryption_keys(),
            &owner,
            &created.raw_token,
            CompleteInput {
                credential: Some("test-secret"),
                ..CompleteInput::default()
            },
            false,
        )
        .await
        .expect("provenance failure must not fail completion");
        let CompleteResult::Completed(view) = completed else {
            panic!("API-key link should complete")
        };
        assert_eq!(view.link.status, ConnectLinkStatus::Completed);
        let service_id = view
            .link
            .completed_user_service_id
            .expect("completed service id");
        let service = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": service_id })
            .await
            .unwrap()
            .expect("provisioned service");
        assert!(service.source_app_id.is_none());
    }

    #[test]
    fn raw_token_requires_prefix_and_32_hex_bytes() {
        let valid = format!("{CONNECT_LINK_PREFIX}{}", "ab".repeat(32));
        assert!(validate_raw_token(&valid).is_ok());
        assert!(validate_raw_token("nyx_clk_short").is_err());
        assert!(validate_raw_token(&format!("{CONNECT_LINK_PREFIX}{}", "zz".repeat(32))).is_err());
    }

    #[test]
    fn terminal_webhook_event_names_cover_every_terminal_status() {
        assert_eq!(
            terminal_webhook_event_type(ConnectLinkStatus::Completed),
            Some("connect_link.completed")
        );
        assert_eq!(
            terminal_webhook_event_type(ConnectLinkStatus::Cancelled),
            Some("connect_link.cancelled")
        );
        assert_eq!(
            terminal_webhook_event_type(ConnectLinkStatus::Expired),
            Some("connect_link.expired")
        );
        assert_eq!(
            terminal_webhook_event_type(ConnectLinkStatus::Pending),
            None
        );
    }

    #[test]
    fn ttl_bounds_match_hosted_link_contract() {
        assert_eq!(DEFAULT_TTL_SECS, 900);
        assert_eq!(MIN_TTL_SECS, 60);
        assert_eq!(MAX_TTL_SECS, 3600);
        assert_eq!(MAX_WAIT_SECS, 120);
        assert_eq!(CLAIM_STALE_SECS, 300);
        assert_eq!(PINNED_GRACE_SECS, 1800);
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
                oauth_client_id: None,
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
                oauth_client_id: None,
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
                oauth_client_id: None,
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
                oauth_client_id: None,
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
                    "completion_claim_at": bson::DateTime::from_chrono(Utc::now()),
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

    #[tokio::test]
    async fn stale_completion_claim_is_taken_over_and_completed() {
        let Some(db) = connect_test_database("connect_link_stale_takeover").await else {
            return;
        };
        let (owner, created) = create_test_link(&db, "stale-takeover").await;
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": {
                    "completion_claim_id": "abandoned-claim",
                    "completion_claim_at": bson::DateTime::from_chrono(
                        Utc::now() - Duration::seconds(CLAIM_STALE_SECS + 1)
                    ),
                } },
            )
            .await
            .expect("seed stale claim");

        let service_id = provision_test_link(&db, &owner, &created).await;
        let stored = db
            .collection::<ConnectLink>(CONNECT_LINKS)
            .find_one(doc! { "_id": &created.link.id })
            .await
            .expect("read completed link")
            .expect("completed link");
        assert_eq!(stored.status, ConnectLinkStatus::Completed);
        assert_eq!(
            stored.completed_user_service_id.as_deref(),
            Some(service_id.as_str())
        );
        assert!(stored.completion_claim_id.is_none());
        assert!(stored.completion_claim_at.is_none());
    }

    #[tokio::test]
    async fn stale_completion_claim_can_be_cancelled() {
        let Some(db) = connect_test_database("connect_link_stale_cancel").await else {
            return;
        };
        let (owner, created) = create_test_link(&db, "stale-cancel").await;
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": {
                    "completion_claim_id": "abandoned-claim",
                    "completion_claim_at": bson::DateTime::from_chrono(
                        Utc::now() - Duration::seconds(CLAIM_STALE_SECS + 1)
                    ),
                } },
            )
            .await
            .expect("seed stale claim");

        let view = cancel(&db, &owner, &created.link.id)
            .await
            .expect("cancel stale claim");
        assert_eq!(view.link.status, ConnectLinkStatus::Cancelled);
        assert!(view.link.completion_claim_id.is_none());
        assert!(view.link.completion_claim_at.is_none());
    }

    #[tokio::test]
    async fn expired_link_with_stale_completion_claim_reaches_expired() {
        let Some(db) = connect_test_database("connect_link_stale_expiry").await else {
            return;
        };
        let (owner, created) = create_test_link(&db, "stale-expiry").await;
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": {
                    "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)),
                    "completion_claim_id": "abandoned-claim",
                    "completion_claim_at": bson::DateTime::from_chrono(
                        Utc::now() - Duration::seconds(CLAIM_STALE_SECS + 1)
                    ),
                } },
            )
            .await
            .expect("seed expired stale claim");

        let view = get_for_actor(&db, &owner, &created.link.id)
            .await
            .expect("expire stale claim");
        assert_eq!(view.link.status, ConnectLinkStatus::Expired);
        assert!(view.link.completion_claim_id.is_none());
        assert!(view.link.completion_claim_at.is_none());
    }

    #[tokio::test]
    async fn fresh_completion_claim_blocks_complete_and_cancel_with_conflict() {
        let Some(db) = connect_test_database("connect_link_fresh_claim_conflict").await else {
            return;
        };
        let (owner, created) = create_test_link(&db, "fresh-claim-conflict").await;
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": {
                    "completion_claim_id": "active-claim",
                    "completion_claim_at": bson::DateTime::from_chrono(Utc::now()),
                } },
            )
            .await
            .expect("seed active claim");

        let completion = complete(
            &db,
            &crate::test_utils::test_encryption_keys(),
            &owner,
            &created.raw_token,
            CompleteInput {
                credential: Some("test-secret"),
                ..CompleteInput::default()
            },
            false,
        )
        .await;
        assert!(matches!(
            completion,
            Err(AppError::ConnectLinkCompletionInProgress)
        ));
        assert!(matches!(
            cancel(&db, &owner, &created.link.id).await,
            Err(AppError::ConnectLinkCompletionInProgress)
        ));
    }

    #[tokio::test]
    async fn initial_completion_claim_still_rejects_expired_link() {
        let Some(db) = connect_test_database("connect_link_initial_claim_expired").await else {
            return;
        };
        let (owner, created) = create_test_link(&db, "initial-claim-expired").await;
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": {
                    "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)),
                } },
            )
            .await
            .expect("expire link");

        let result = complete(
            &db,
            &crate::test_utils::test_encryption_keys(),
            &owner,
            &created.raw_token,
            CompleteInput {
                credential: Some("test-secret"),
                ..CompleteInput::default()
            },
            false,
        )
        .await;
        assert!(matches!(result, Err(AppError::ConnectLinkExpired)));
    }

    #[tokio::test]
    async fn oauth_callback_completes_pinned_service_after_link_expiry() {
        let Some(db) = connect_test_database("connect_link_oauth_after_expiry").await else {
            return;
        };
        let (owner, created) = create_test_link(&db, "oauth-after-expiry").await;
        let service_id = provision_test_link(&db, &owner, &created).await;
        let service = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &service_id })
            .await
            .expect("read provisioned service")
            .expect("provisioned service");
        let key_id = service.api_key_id.expect("provisioned API key");
        let connection_id = Uuid::new_v4().to_string();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                doc! { "_id": &key_id },
                doc! { "$set": {
                    "connection_id": &connection_id,
                    "status": "active",
                } },
            )
            .await
            .expect("mark OAuth key active");
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! {
                    "$set": {
                        "status": "pending",
                        "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)),
                    },
                    "$unset": { "completed_at": "" },
                },
            )
            .await
            .expect("restore pending OAuth link");

        assert!(
            record_provider_error(&db, &created.link.id, &owner, "provider_access_denied")
                .await
                .expect("record provider decline")
        );
        let declined = get_for_actor(&db, &owner, &created.link.id)
            .await
            .expect("read declined link");
        assert_eq!(
            declined.link.last_error.as_deref(),
            Some("provider_access_denied")
        );
        assert!(declined.link.last_error_at.is_some());

        let view = complete_oauth_callback(&db, &created.link.id, &owner, &connection_id)
            .await
            .expect("complete expired OAuth callback");
        assert_eq!(view.link.status, ConnectLinkStatus::Completed);
        assert_eq!(
            view.link.completed_user_service_id.as_deref(),
            Some(service_id.as_str())
        );
        assert!(view.link.last_error.is_none());
        assert!(view.link.last_error_at.is_none());
    }

    #[tokio::test]
    async fn device_poll_resumes_pinned_active_service_after_link_expiry() {
        let Some(db) = connect_test_database("connect_link_device_after_expiry").await else {
            return;
        };
        let (owner, created) = create_test_link(&db, "device-after-expiry").await;
        let service_id = provision_test_link(&db, &owner, &created).await;
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! {
                    "$set": {
                        "status": "pending",
                        "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)),
                    },
                    "$unset": { "completed_at": "" },
                },
            )
            .await
            .expect("restore pending device link");

        let result = complete(
            &db,
            &crate::test_utils::test_encryption_keys(),
            &owner,
            &created.raw_token,
            CompleteInput::default(),
            false,
        )
        .await
        .expect("resume expired device link");
        let CompleteResult::Completed(view) = result else {
            panic!("active pinned service should complete")
        };
        assert_eq!(view.link.status, ConnectLinkStatus::Completed);
        assert_eq!(
            view.link.completed_user_service_id.as_deref(),
            Some(service_id.as_str())
        );
    }

    #[tokio::test]
    async fn pinned_link_past_grace_expires_and_refuses_callback_and_resume() {
        let Some(db) = connect_test_database("connect_link_pinned_grace_expired").await else {
            return;
        };
        let (owner, created) = create_test_link(&db, "pinned-grace-expired").await;
        let service_id = provision_test_link(&db, &owner, &created).await;
        let service = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &service_id })
            .await
            .expect("read provisioned service")
            .expect("provisioned service");
        let key_id = service.api_key_id.expect("provisioned API key");
        let connection_id = Uuid::new_v4().to_string();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                doc! { "_id": &key_id },
                doc! { "$set": {
                    "connection_id": &connection_id,
                    "status": "active",
                } },
            )
            .await
            .expect("mark provisioned key active");
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! {
                    "$set": {
                        "status": "pending",
                        "expires_at": bson::DateTime::from_chrono(
                            Utc::now() - Duration::seconds(PINNED_GRACE_SECS + 1)
                        ),
                    },
                    "$unset": { "completed_at": "" },
                },
            )
            .await
            .expect("restore pinned link past grace");

        let view = get_for_actor(&db, &owner, &created.link.id)
            .await
            .expect("read expired pinned link");
        assert_eq!(view.link.status, ConnectLinkStatus::Expired);

        let callback = complete_oauth_callback(&db, &created.link.id, &owner, &connection_id).await;
        assert!(matches!(callback, Err(AppError::ConnectLinkExpired)));

        let resumed = complete(
            &db,
            &crate::test_utils::test_encryption_keys(),
            &owner,
            &created.raw_token,
            CompleteInput::default(),
            false,
        )
        .await;
        assert!(matches!(resumed, Err(AppError::ConnectLinkExpired)));
    }

    #[tokio::test]
    async fn pinned_oauth_resume_stops_after_finalization_grace() {
        let Some(db) = connect_test_database("connect_link_oauth_resume_grace").await else {
            return;
        };
        let (owner, created) = create_test_link(&db, "oauth-resume-grace").await;
        let service_id = provision_test_link(&db, &owner, &created).await;
        let service = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &service_id })
            .await
            .expect("read provisioned service")
            .expect("provisioned service");
        let key_id = service.api_key_id.expect("provisioned API key");
        let connection_id = Uuid::new_v4().to_string();
        let provider = test_oauth_provider();
        db.collection::<ProviderConfig>(PROVIDERS)
            .insert_one(&provider)
            .await
            .expect("insert OAuth provider");
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .update_one(
                doc! { "_id": &created.link.service_id },
                doc! { "$set": { "provider_config_id": &provider.id } },
            )
            .await
            .expect("configure catalog OAuth provider");
        db.collection::<UserApiKey>(USER_API_KEYS)
            .update_one(
                doc! { "_id": &key_id },
                doc! { "$set": {
                    "connection_id": &connection_id,
                    "status": "pending_auth",
                } },
            )
            .await
            .expect("mark provisioned key pending OAuth");
        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! {
                    "$set": {
                        "status": "pending",
                        "expires_at": bson::DateTime::from_chrono(
                            Utc::now() - Duration::seconds(1)
                        ),
                    },
                    "$unset": { "completed_at": "" },
                },
            )
            .await
            .expect("restore pinned OAuth link within grace");

        let within_grace = complete(
            &db,
            &crate::test_utils::test_encryption_keys(),
            &owner,
            &created.raw_token,
            CompleteInput::default(),
            false,
        )
        .await
        .expect("resume pinned OAuth link within grace");
        assert!(matches!(
            within_grace,
            CompleteResult::OauthRequired {
                connection_id: resumed_connection_id,
                ..
            } if resumed_connection_id == connection_id
        ));

        db.collection::<ConnectLink>(CONNECT_LINKS)
            .update_one(
                doc! { "_id": &created.link.id },
                doc! { "$set": {
                    "expires_at": bson::DateTime::from_chrono(
                        Utc::now() - Duration::seconds(PINNED_GRACE_SECS + 1)
                    ),
                } },
            )
            .await
            .expect("move pinned OAuth link past grace");

        let after_grace = complete(
            &db,
            &crate::test_utils::test_encryption_keys(),
            &owner,
            &created.raw_token,
            CompleteInput::default(),
            false,
        )
        .await;
        assert!(matches!(after_grace, Err(AppError::ConnectLinkExpired)));
    }
}
