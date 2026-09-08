use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
    options::ReturnDocument,
};
use rand::{RngCore, rngs::OsRng};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;
use zeroize::Zeroizing;

use super::{
    api_key_credential_service as credentials, api_key_mutation_service as mutations,
    api_key_scope_service, api_key_validation, audit_service, auth_device_service, key_service,
    login_client_context::{PreviewOutput, context_preview, sanitize_context, sanitize_optional},
    node_service, org_service, user_service_service,
};
use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::{
    agent_key_login_request::{
        AgentKeyLoginRequest, AgentKeyLoginStatus as Status, COLLECTION_NAME,
    },
    api_key::{ApiKey, ApiKeyPurpose, COLLECTION_NAME as API_KEYS},
    api_key_credential::{
        ApiKeyCredential, COLLECTION_NAME as CREDENTIALS, CredentialRevokedReason,
    },
    auth_device_code::AuthDeviceClientIpAttribution,
    login_client_context::LoginClientContext,
    node::{COLLECTION_NAME as NODES, Node},
    user::{COLLECTION_NAME as USERS, User},
    user_service::{COLLECTION_NAME as SERVICES, UserService},
};

pub const REQUEST_TTL_SECS: i64 = 600;
pub const POLL_INTERVAL_SECS: u32 = 5;

pub fn error_outcome(error: &AppError) -> &'static str {
    match error {
        AppError::AgentKeyLoginPending => "pending",
        AppError::AgentKeyLoginSlowDown => "slow_down",
        AppError::AgentKeyLoginDenied => "denied",
        AppError::AgentKeyLoginExpired => "expired",
        AppError::AgentKeyLoginAlreadyDelivered => "already_delivered",
        AppError::AgentKeyLoginNotFound | AppError::AgentKeyLoginUserCodeInvalid => "not_found",
        AppError::AgentKeyLoginRateLimited => "rate_limit_hit",
        _ => "error",
    }
}

fn credential_label(context: &LoginClientContext, profile: Option<&str>) -> String {
    let client = sanitize_optional(context.client_label.clone(), 64);
    let profile = sanitize_optional(profile.map(str::to_owned), 64);
    let label = match (client, profile) {
        (Some(client), Some(profile)) => format!("{client} \u{00b7} {profile}"),
        (Some(value), None) | (None, Some(value)) => value,
        (None, None) => "NyxID CLI".into(),
    };
    sanitize_optional(Some(label), 96).expect("login label is nonempty")
}

#[derive(Serialize, ToSchema)]
#[schema(as = AgentKeyLoginRequestOutput)]
pub struct RequestOutput {
    pub device_code: String,
    pub user_code: String,
    pub expires_in: i64,
    pub interval: u32,
}

impl std::fmt::Debug for RequestOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentKeyRequestOutput")
            .finish_non_exhaustive()
    }
}

pub use crate::models::login_grant::{NewKeyInput, Selection};

#[derive(Clone, Debug, Serialize, ToSchema)]
#[schema(as = AgentKeyLoginResourceSummary)]
pub struct ResourceSummary {
    pub id: String,
    pub name: String,
    pub owner_id: String,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
#[schema(as = AgentKeyLoginKeySummary)]
pub struct KeySummary {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub owner_type: String,
    pub owner_id: String,
    pub owner_name: String,
    pub scopes: String,
    pub allow_all_services: bool,
    pub allow_all_nodes: bool,
    pub allowed_service_ids: Vec<String>,
    pub allowed_node_ids: Vec<String>,
    pub allowed_services: Vec<ResourceSummary>,
    pub allowed_nodes: Vec<ResourceSummary>,
    pub expires_at: Option<String>,
    pub rate_limit_per_second: Option<u32>,
    pub rate_limit_burst: Option<u32>,
    pub platform: Option<String>,
    pub created_now: bool,
}

#[derive(Debug, Serialize, ToSchema)]
#[schema(as = AgentKeyLoginLoginOptions)]
pub struct LoginOptions {
    pub keys: Vec<KeySummary>,
    pub services: Vec<ResourceSummary>,
    pub nodes: Vec<ResourceSummary>,
    pub orgs: Vec<ResourceSummary>,
}

pub struct LoginPreview {
    pub context: PreviewOutput,
    pub interval: u32,
}

pub struct Delivery {
    pub credential: Zeroizing<String>,
    pub credential_id: String,
    pub credential_expires_at: Option<String>,
    pub label: String,
    pub api_key: KeySummary,
}

impl std::fmt::Debug for Delivery {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentKeyDelivery").finish_non_exhaustive()
    }
}

fn code_hash(key: &[u8], code: &str) -> String {
    auth_device_service::hmac_hex(key, format!("agent-key-login:{code}").as_bytes())
}

fn normalized_code(raw: &str) -> AppResult<String> {
    auth_device_service::normalize_user_code(raw)
        .map_err(|_| AppError::AgentKeyLoginUserCodeInvalid)
}

pub async fn request(
    db: &Database,
    hmac_key: &[u8],
    context: LoginClientContext,
    requested_profile: Option<String>,
) -> AppResult<RequestOutput> {
    let mut context = sanitize_context(context);
    context.requested_profile =
        sanitize_optional(requested_profile.or(context.requested_profile), 64);
    for attempt in 0..5 {
        let mut random = Zeroizing::new([0u8; 32]);
        OsRng.fill_bytes(random.as_mut());
        let device_code = format!("nyx_akl_{}", hex::encode(random.as_slice()));
        let user_code = auth_device_service::generate_user_code();
        let now = Utc::now();
        let row = AgentKeyLoginRequest {
            id: Uuid::new_v4().to_string(),
            device_code_hmac: code_hash(hmac_key, &device_code),
            user_code_hmac: code_hash(hmac_key, &user_code),
            status: Status::Pending,
            poll_interval_secs: POLL_INTERVAL_SECS,
            slow_down_increments: 0,
            last_polled_at: None,
            client_ip_hmac: context
                .client_ip
                .as_deref()
                .map(|ip| code_hash(hmac_key, ip)),
            context: context.clone(),
            approved_user_id: None,
            approver_ip_hmac: None,
            api_key_id: None,
            credential_id: None,
            key_was_created: false,
            key_created_by_approval: false,
            delivery_credential_encrypted: None,
            approved_at: None,
            delivered_at: None,
            denied_at: None,
            denied_by_user_id: None,
            created_at: now,
            expires_at: now + Duration::seconds(REQUEST_TTL_SECS),
        };
        match db
            .collection::<AgentKeyLoginRequest>(COLLECTION_NAME)
            .insert_one(&row)
            .await
        {
            Ok(_) => {
                tracing::Span::current().record("row_id", row.id.as_str());
                return Ok(RequestOutput {
                    device_code,
                    user_code: auth_device_service::format_user_code(&user_code),
                    expires_in: REQUEST_TTL_SECS,
                    interval: POLL_INTERVAL_SECS,
                });
            }
            Err(error)
                if attempt < 4
                    && matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(e)) if e.code == 11000) =>
            {
                continue;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Err(AppError::Internal(
        "Agent Key login code allocation failed".into(),
    ))
}

async fn find_by_user_code(
    db: &Database,
    hmac_key: &[u8],
    raw: &str,
) -> AppResult<AgentKeyLoginRequest> {
    db.collection::<AgentKeyLoginRequest>(COLLECTION_NAME)
        .find_one(doc! {"user_code_hmac": code_hash(hmac_key, &normalized_code(raw)?)})
        .await?
        .ok_or(AppError::AgentKeyLoginUserCodeInvalid)
        .inspect(record_request_identity)
}

fn record_request_identity(row: &AgentKeyLoginRequest) {
    let span = tracing::Span::current();
    span.record("row_id", row.id.as_str());
    if let Some(id) = row.api_key_id.as_deref() {
        span.record("api_key_id", id);
    }
    if let Some(id) = row.credential_id.as_deref() {
        span.record("credential_id", id);
    }
}

fn decision_error(row: &AgentKeyLoginRequest) -> AppError {
    if row.expires_at <= Utc::now() {
        return AppError::AgentKeyLoginExpired;
    }
    match row.status {
        Status::Denied => AppError::AgentKeyLoginDenied,
        Status::Expired => AppError::AgentKeyLoginExpired,
        Status::Pending => AppError::AgentKeyLoginPending,
        Status::Approved | Status::Delivered => AppError::AgentKeyLoginAlreadyDelivered,
    }
}

async fn current_decision_error(db: &Database, id: &str) -> AppResult<AppError> {
    let row = db
        .collection::<AgentKeyLoginRequest>(COLLECTION_NAME)
        .find_one(doc! {"_id": id})
        .await?
        .ok_or(AppError::AgentKeyLoginNotFound)?;
    Ok(decision_error(&row))
}

pub async fn eligible_key(db: &Database, actor: &str, id: &str) -> AppResult<ApiKey> {
    if legacy_parent_reserved(db, id).await? {
        return Err(AppError::AgentKeyLoginKeyIneligible);
    }
    eligible_delivery_key(db, actor, id).await
}

async fn legacy_parent_reserved(db: &Database, id: &str) -> AppResult<bool> {
    // Old sweepers can delete these parents without a reuse fence. Do not admit
    // a new consumer until the old exchange has completed its delivery.
    Ok(db
        .collection::<AgentKeyLoginRequest>(COLLECTION_NAME)
        .find_one(doc! {
            "api_key_id": id, "key_was_created": true, "status": {"$ne": "delivered"},
        })
        .await?
        .is_some())
}

async fn eligible_delivery_key(db: &Database, actor: &str, id: &str) -> AppResult<ApiKey> {
    let key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! {"_id": id})
        .await?
        .ok_or(AppError::AgentKeyLoginKeyIneligible)?;
    if !key_is_eligible(&key)
        || !org_service::resolve_owner_access(db, actor, &key.user_id)
            .await?
            .can_write()
    {
        return Err(AppError::AgentKeyLoginKeyIneligible);
    }
    Ok(key)
}

fn key_is_eligible(key: &ApiKey) -> bool {
    key.is_active
        && key.purpose == ApiKeyPurpose::General
        && key.expires_at.is_none_or(|expiry| expiry > Utc::now())
}

pub async fn key_summary(db: &Database, key: &ApiKey, created_now: bool) -> AppResult<KeySummary> {
    let owner = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &key.user_id})
        .await?
        .ok_or(AppError::AgentKeyLoginKeyIneligible)?;
    let services: Vec<UserService> = db
        .collection::<UserService>(SERVICES)
        .find(doc! {"_id": {"$in": &key.allowed_service_ids}})
        .await?
        .try_collect()
        .await?;
    let nodes: Vec<Node> = db
        .collection::<Node>(NODES)
        .find(doc! {"_id": {"$in": &key.allowed_node_ids}})
        .await?
        .try_collect()
        .await?;
    Ok(KeySummary {
        id: key.id.clone(),
        name: key.name.clone(),
        key_prefix: key.key_prefix.clone(),
        owner_type: if owner.user_type.is_org() {
            "org"
        } else {
            "personal"
        }
        .into(),
        owner_id: owner.id.clone(),
        owner_name: owner.display_name.unwrap_or(owner.email),
        scopes: key.scopes.clone(),
        allow_all_services: key.allow_all_services,
        allow_all_nodes: key.allow_all_nodes,
        allowed_service_ids: key.allowed_service_ids.clone(),
        allowed_node_ids: key.allowed_node_ids.clone(),
        allowed_services: key
            .allowed_service_ids
            .iter()
            .map(|id| {
                let row = services.iter().find(|row| &row.id == id);
                ResourceSummary {
                    id: id.clone(),
                    name: row.map_or_else(|| "Unavailable service".into(), |r| r.slug.clone()),
                    owner_id: row.map_or_else(|| key.user_id.clone(), |r| r.user_id.clone()),
                }
            })
            .collect(),
        allowed_nodes: key
            .allowed_node_ids
            .iter()
            .map(|id| {
                let row = nodes.iter().find(|row| &row.id == id);
                ResourceSummary {
                    id: id.clone(),
                    name: row.map_or_else(|| "Unavailable node".into(), |r| r.name.clone()),
                    owner_id: row.map_or_else(|| key.user_id.clone(), |r| r.user_id.clone()),
                }
            })
            .collect(),
        expires_at: key.expires_at.map(|d| d.to_rfc3339()),
        rate_limit_per_second: key.rate_limit_per_second,
        rate_limit_burst: key.rate_limit_burst,
        platform: key.platform.clone(),
        created_now,
    })
}

pub async fn options(
    db: &Database,
    hmac_key: &[u8],
    actor: &str,
    user_code: &str,
) -> AppResult<LoginOptions> {
    let row = find_by_user_code(db, hmac_key, user_code).await?;
    if row.expires_at <= Utc::now() || row.status != Status::Pending {
        return Err(decision_error(&row));
    }
    options_for_actor(db, actor).await
}

pub async fn options_for_actor(db: &Database, actor: &str) -> AppResult<LoginOptions> {
    let mut owners = vec![actor.to_string()];
    let mut orgs = Vec::new();
    for membership in org_service::list_memberships_for_member(db, actor, false).await? {
        if membership.role.can_admin()
            && org_service::resolve_owner_access(db, actor, &membership.org_user_id)
                .await?
                .can_write()
        {
            let owner = db
                .collection::<User>(USERS)
                .find_one(doc! {"_id": &membership.org_user_id, "is_active": true})
                .await?;
            if let Some(owner) = owner {
                owners.push(owner.id.clone());
                orgs.push(ResourceSummary {
                    id: owner.id.clone(),
                    owner_id: owner.id,
                    name: owner.display_name.unwrap_or(owner.email),
                });
            }
        }
    }
    let mut keys = Vec::new();
    for owner in owners {
        for key in key_service::list_api_keys(db, &owner).await? {
            if key_is_eligible(&key) && !legacy_parent_reserved(db, &key.id).await? {
                keys.push(key_summary(db, &key, false).await?);
            }
        }
    }
    let services = user_service_service::list_user_services_with_sources(db, actor)
        .await?
        .into_iter()
        .filter(|entry| match &entry.source {
            user_service_service::CredentialSource::Personal => true,
            user_service_service::CredentialSource::Org { allowed, .. } => *allowed,
        })
        .map(|entry| ResourceSummary {
            id: entry.service.id,
            name: entry.service.slug,
            owner_id: entry.service.user_id,
        })
        .collect();
    let nodes = node_service::list_user_nodes(db, actor)
        .await?
        .into_iter()
        .map(|entry| ResourceSummary {
            id: entry.node.id,
            name: entry.node.name,
            owner_id: entry.node.user_id,
        })
        .collect();
    Ok(LoginOptions {
        keys,
        services,
        nodes,
        orgs,
    })
}

/// Issue a selected key and its child inside the caller's authorization transaction.
#[allow(clippy::too_many_arguments)]
pub async fn issue_selected(
    db: &Database,
    actor: &str,
    request_id: &str,
    context: &LoginClientContext,
    profile: Option<&str>,
    selection: &Selection,
    credential_expires_at: Option<DateTime<Utc>>,
    key_id: &str,
    credential_id: &str,
    secret: &str,
    session: &mut mongodb::ClientSession,
) -> AppResult<ApiKey> {
    if matches!(selection, Selection::Existing { .. }) && legacy_parent_reserved(db, key_id).await?
    {
        return Err(AppError::AgentKeyLoginKeyIneligible);
    }
    if let Selection::New(input) = selection {
        let owner = api_key_scope_service::resolve_scope_owner_id(
            db,
            actor,
            input.target_org_id.as_deref(),
        )
        .await?;
        let expires_at = input
            .expires_at
            .as_deref()
            .filter(|s| !s.is_empty())
            .map(api_key_validation::parse_expires_at)
            .transpose()?;
        credentials::credential_expiry(expires_at, credential_expires_at)?;
        api_key_validation::resolve_create_allow_all(
            &input.allowed_service_ids,
            Some(input.allow_all_services),
            "allow_all_services",
            "allowed_service_ids",
        )?;
        api_key_validation::resolve_create_allow_all(
            &input.allowed_node_ids,
            Some(input.allow_all_nodes),
            "allow_all_nodes",
            "allowed_node_ids",
        )?;
        let created = key_service::create_login_api_key(
            db,
            &owner,
            actor,
            key_id,
            input,
            expires_at,
            &mut *session,
        )
        .await?;
        drop(Zeroizing::new(created.full_key));
    }
    let parent = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! {"_id": key_id, "is_active": true})
        .session(&mut *session)
        .await?
        .ok_or(AppError::AgentKeyLoginKeyIneligible)?;
    if !key_is_eligible(&parent)
        || !org_service::resolve_owner_access(db, actor, &parent.user_id)
            .await?
            .can_write()
    {
        return Err(AppError::AgentKeyLoginKeyIneligible);
    }
    credentials::issue(
        db,
        &parent,
        credential_id,
        request_id,
        &credential_label(context, profile),
        credential_expires_at,
        secret,
        &mut *session,
    )
    .await?;
    db.collection::<ApiKeyCredential>(CREDENTIALS)
        .update_one(
            doc! {"_id": credential_id},
            doc! {"$set": {"is_active": true}},
        )
        .session(&mut *session)
        .await?;
    Ok(parent)
}

pub async fn preview(
    db: &Database,
    hmac_key: &[u8],
    user_code: &str,
    viewer_ip: Option<&str>,
    attribution: AuthDeviceClientIpAttribution,
) -> AppResult<LoginPreview> {
    let mut row = find_by_user_code(db, hmac_key, user_code).await?;
    if row.expires_at <= Utc::now()
        && row.status != Status::Delivered
        && row.status != Status::Denied
    {
        expire(db, &row).await?;
        row.status = Status::Expired;
    }
    let ready = row.delivery_credential_encrypted.is_some() || row.status == Status::Delivered;
    let status = if row.status == Status::Approved && !ready {
        Status::Pending
    } else {
        row.status
    };
    Ok(LoginPreview {
        context: context_preview(
            row.context,
            row.created_at,
            row.expires_at,
            status,
            viewer_ip,
            attribution,
        ),
        interval: row.poll_interval_secs,
    })
}

#[allow(clippy::too_many_arguments)]
pub async fn approve(
    db: &Database,
    encryption: &EncryptionKeys,
    hmac_key: &[u8],
    actor: &str,
    user_code: &str,
    selection: Selection,
    credential_expires_at: Option<DateTime<Utc>>,
    approver_ip: Option<&str>,
    approver_user_agent: Option<&str>,
) -> AppResult<()> {
    let row = find_by_user_code(db, hmac_key, user_code).await?;
    if row.status != Status::Pending || row.expires_at <= Utc::now() {
        return Err(decision_error(&row));
    }
    let (key_id, created_now) = match &selection {
        Selection::Existing { api_key_id } => {
            (eligible_key(db, actor, api_key_id).await?.id, false)
        }
        Selection::New(_) => (Uuid::new_v4().to_string(), true),
    };
    let credential_id = Uuid::new_v4().to_string();
    let secret = credentials::generate_secret();
    let encrypted = encryption.encrypt(secret.as_bytes()).await?;
    let mut session = db.client().start_session().await?;
    // Claim, key insertion, child issuance, and encrypted delivery are committed
    // together. A cancelled future or process crash cannot leave a partial grant.
    let request_id = row.id.clone();
    tracing::Span::current().record("row_id", request_id.as_str());
    tracing::Span::current().record("api_key_id", key_id.as_str());
    tracing::Span::current().record("credential_id", credential_id.as_str());
    let owner_type = {
        let db = db.clone();
        let actor = actor.to_string();
        let key_id = key_id.clone();
        let credential_id = credential_id.clone();
        let approver_ip_hmac = approver_ip.map(|ip| code_hash(hmac_key, ip));
        session.start_transaction().and_run2(async move |session| {
        let operation: AppResult<String> = async {
            let claimed = db.collection::<AgentKeyLoginRequest>(COLLECTION_NAME).find_one_and_update(
                doc! {"_id": &row.id, "status": "pending", "expires_at": {"$gt": bson::DateTime::from_chrono(Utc::now())}},
                doc! {"$set": {"status": "approved"}},
            ).session(&mut *session).return_document(ReturnDocument::Before).await?;
            if claimed.is_none() {
                return Err(current_decision_error(&db, &row.id).await?);
            }
            let parent = issue_selected(&db, &actor, &row.id, &row.context,
                row.context.requested_profile.as_deref(), &selection, credential_expires_at,
                &key_id, &credential_id, &secret, &mut *session).await?;
            let owner_type = key_summary(&db, &parent, created_now).await?.owner_type;
            let now = Utc::now();
            if row.expires_at <= now { return Err(AppError::AgentKeyLoginExpired); }
            db.collection::<AgentKeyLoginRequest>(COLLECTION_NAME).update_one(
                doc! {"_id": &row.id, "status": "approved"},
                doc! {"$set": {"approved_user_id": &actor, "api_key_id": &key_id,
                    "credential_id": &credential_id, "key_was_created": false,
                    "key_created_by_approval": created_now,
                    "approver_ip_hmac": &approver_ip_hmac,
                    "approved_at": bson::DateTime::from_chrono(now),
                    "expires_at": bson::DateTime::from_chrono(now + Duration::seconds(60)),
                    "delivery_credential_encrypted": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted.clone()}}},
            ).session(&mut *session).await?;
            Ok(owner_type)
        }.await;
        mutations::transaction_result(operation)
    }).await.map_err(mutations::map_transaction_error)?
    };
    audit_service::log_async(
        db.clone(),
        Some(actor.into()),
        "agent_key_login_approved".into(),
        Some(
            serde_json::json!({"request_id": request_id, "api_key_id": key_id, "credential_id": credential_id, "key_was_created": created_now, "owner_type": owner_type}),
        ),
        approver_ip.map(str::to_owned),
        approver_user_agent.map(str::to_owned),
        None,
        None,
    );
    Ok(())
}

pub async fn deny(
    db: &Database,
    hmac_key: &[u8],
    actor: &str,
    user_code: &str,
    approver_ip: Option<&str>,
    approver_user_agent: Option<&str>,
) -> AppResult<()> {
    let row = find_by_user_code(db, hmac_key, user_code).await?;
    let now = Utc::now();
    if row.status != Status::Pending || row.expires_at <= now {
        return Err(decision_error(&row));
    }
    let denied = db.collection::<AgentKeyLoginRequest>(COLLECTION_NAME).find_one_and_update(
        doc! {"_id": &row.id, "status": "pending", "expires_at": {"$gt": bson::DateTime::from_chrono(now)}},
        doc! {"$set": {"status": "denied", "denied_at": bson::DateTime::from_chrono(now), "denied_by_user_id": actor}},
    ).await?;
    if denied.is_none() {
        return Err(current_decision_error(db, &row.id).await?);
    }
    audit_service::log_async(
        db.clone(),
        Some(actor.into()),
        "agent_key_login_denied".into(),
        Some(serde_json::json!({"request_id": row.id})),
        approver_ip.map(str::to_owned),
        approver_user_agent.map(str::to_owned),
        None,
        None,
    );
    Ok(())
}

#[tracing::instrument(
    name = "agent_key_login.poll",
    skip_all,
    fields(row_id, api_key_id, credential_id, outcome)
)]
pub async fn poll(
    db: &Database,
    encryption: &EncryptionKeys,
    hmac_key: &[u8],
    device_code: &str,
    poller_ip: Option<&str>,
    poller_user_agent: Option<&str>,
) -> AppResult<Delivery> {
    let result: AppResult<Delivery> = async {
    let collection = db.collection::<AgentKeyLoginRequest>(COLLECTION_NAME);
    let row = collection
        .find_one(doc! {"device_code_hmac": code_hash(hmac_key, device_code)})
        .await?
        .ok_or(AppError::AgentKeyLoginNotFound)?;
    record_request_identity(&row);
    let now = Utc::now();
    if row.status == Status::Delivered {
        return Err(AppError::AgentKeyLoginAlreadyDelivered);
    }
    if row.status == Status::Denied {
        return Err(AppError::AgentKeyLoginDenied);
    }
    if row.expires_at <= now || row.status == Status::Expired {
        expire(db, &row).await?;
        return Err(AppError::AgentKeyLoginExpired);
    }
    if row.status == Status::Pending || row.delivery_credential_encrypted.is_none() {
        let slow = row.last_polled_at.is_some_and(|last| {
            now - last
                < Duration::seconds(
                    i64::from(row.poll_interval_secs) + i64::from(row.slow_down_increments) * 5,
                )
        });
        let mut update = doc! {"$set": {"last_polled_at": bson::DateTime::from_chrono(now)}};
        if slow {
            update.insert("$inc", doc! {"slow_down_increments": 1});
        }
        collection.update_one(doc! {"_id": &row.id}, update).await?;
        return Err(if slow {
            AppError::AgentKeyLoginSlowDown
        } else {
            AppError::AgentKeyLoginPending
        });
    }
    // Resolve and decrypt before consuming the exchange. A failed read or KMS
    // call leaves the approved row available for retry and expiry cleanup.
    let delivery = prepare_delivery(db, encryption, &row).await?;
    let now = Utc::now();
    let delivered = collection.find_one_and_update(
        doc! {"_id": &row.id, "status": "approved", "expires_at": {"$gt": bson::DateTime::from_chrono(now)}, "delivery_credential_encrypted": {"$ne": bson::Bson::Null}},
        doc! {"$set": {"status": "delivered", "delivered_at": bson::DateTime::from_chrono(now), "last_polled_at": bson::DateTime::from_chrono(now)}, "$unset": {"delivery_credential_encrypted": ""}},
    ).return_document(ReturnDocument::Before).await?;
    let delivered = match delivered {
        Some(delivered) => delivered,
        None => return Err(current_decision_error(db, &row.id).await?),
    };
    audit_service::log_async(
        db.clone(),
        delivered.approved_user_id,
        "agent_key_login_delivered".into(),
        Some(
            serde_json::json!({"request_id": delivered.id, "api_key_id": delivery.api_key.id, "credential_id": delivery.credential_id}),
        ),
        poller_ip.map(str::to_owned),
        poller_user_agent.map(str::to_owned),
        None,
        None,
    );
    Ok(delivery)
    }.await;
    let outcome = result
        .as_ref()
        .map_or_else(|error| error_outcome(error), |_| "delivered");
    tracing::Span::current().record("outcome", outcome);
    tracing::info!(
        outcome,
        error_code = result.as_ref().err().map(AppError::error_code),
        "agent_key_login.poll.outcome"
    );
    result
}

async fn prepare_delivery(
    db: &Database,
    encryption: &EncryptionKeys,
    row: &AgentKeyLoginRequest,
) -> AppResult<Delivery> {
    prepare_credential_delivery(
        db,
        encryption,
        row.credential_id.as_deref(),
        row.approved_user_id.as_deref(),
        row.delivery_credential_encrypted.as_deref(),
        row.key_was_created || row.key_created_by_approval,
    )
    .await
}

pub async fn prepare_credential_delivery(
    db: &Database,
    encryption: &EncryptionKeys,
    credential_id: Option<&str>,
    approved_user_id: Option<&str>,
    encrypted: Option<&[u8]>,
    key_was_created: bool,
) -> AppResult<Delivery> {
    let id = credential_id.ok_or(AppError::AgentKeyCredentialNotFound)?;
    let child = db
        .collection::<ApiKeyCredential>(CREDENTIALS)
        .find_one(doc! {"_id": id, "is_active": true})
        .await?
        .ok_or(AppError::AgentKeyCredentialNotFound)?;
    let parent = eligible_delivery_key(
        db,
        approved_user_id.ok_or(AppError::AgentKeyLoginKeyIneligible)?,
        &child.api_key_id,
    )
    .await?;
    credentials::credential_expiry(parent.expires_at, child.expires_at)?;
    let plaintext = Zeroizing::new(
        encryption
            .decrypt(encrypted.ok_or(AppError::AgentKeyCredentialNotFound)?)
            .await?,
    );
    let credential = Zeroizing::new(
        String::from_utf8(plaintext.to_vec())
            .map_err(|_| AppError::Internal("Invalid Agent Key delivery encoding".into()))?,
    );
    let api_key = key_summary(db, &parent, key_was_created).await?;
    Ok(Delivery {
        credential,
        credential_id: child.id,
        credential_expires_at: child.expires_at.map(|d| d.to_rfc3339()),
        label: child.label,
        api_key,
    })
}

async fn cleanup(db: &Database, row: &AgentKeyLoginRequest) -> AppResult<()> {
    if let Some(id) = row.credential_id.as_deref() {
        credentials::revoke(db, id, CredentialRevokedReason::UndeliveredExpired).await?;
    }
    // Parent configuration belongs to the approver and may already be reused.
    // Its primary secret was discarded at creation; only this child can leak
    // usable authority from an abandoned exchange, so revoke that child alone.
    Ok(())
}

async fn expire(db: &Database, row: &AgentKeyLoginRequest) -> AppResult<()> {
    let collection = db.collection::<AgentKeyLoginRequest>(COLLECTION_NAME);
    // Claim expiry before revoking anything: a concurrent delivery must either
    // win the same row or leave its credential to this durable cleanup marker.
    let current = collection.find_one_and_update(
        doc! {"_id": &row.id, "status": {"$in": ["pending", "approved", "expired"]}, "expires_at": {"$lte": bson::DateTime::from_chrono(Utc::now())}},
        doc! {"$set": {"status": "expired"}, "$unset": {"delivery_credential_encrypted": ""}},
    ).return_document(ReturnDocument::After).await?;
    if let Some(current) = current {
        cleanup(db, &current).await?;
        collection
            .update_one(
                doc! {"_id": &row.id, "status": "expired"},
                doc! {"$set": {"credential_id": bson::Bson::Null}},
            )
            .await?;
    }
    Ok(())
}

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SweepResult {
    pub succeeded: u64,
    pub failed: u64,
}

pub async fn sweep_expired(db: &Database) -> AppResult<SweepResult> {
    let mut cursor = db.collection::<AgentKeyLoginRequest>(COLLECTION_NAME)
        .find(doc! {"status": {"$in": ["pending", "approved", "expired"]}, "expires_at": {"$lte": bson::DateTime::from_chrono(Utc::now())}}).await?;
    let mut result = SweepResult::default();
    while let Some(row) = cursor.try_next().await? {
        match expire(db, &row).await {
            Ok(()) => result.succeeded += 1,
            Err(error) => {
                result.failed += 1;
                tracing::error!(row_id = %row.id, error_code = error.error_code(), outcome = "cleanup_failed", "agent_key_login.sweep.row");
            }
        }
    }
    tracing::info!(
        succeeded = result.succeeded,
        failed = result.failed,
        "agent_key_login.sweep"
    );
    Ok(result)
}

pub async fn self_metadata(
    db: &Database,
    key_id: &str,
    credential_id: &str,
) -> AppResult<(KeySummary, ApiKeyCredential)> {
    let child = db
        .collection::<ApiKeyCredential>(CREDENTIALS)
        .find_one(doc! {"_id": credential_id, "api_key_id": key_id, "is_active": true})
        .await?
        .ok_or(AppError::AgentKeyCredentialNotFound)?;
    let parent = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! {"_id": key_id, "is_active": true})
        .await?
        .ok_or(AppError::AgentKeyLoginKeyIneligible)?;
    Ok((key_summary(db, &parent, false).await?, child))
}

#[cfg(test)]
mod tests;
