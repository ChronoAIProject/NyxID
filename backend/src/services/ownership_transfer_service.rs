use bson::{Document, doc};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::api_key_mutation_service::{map_transaction_error, transaction_result};
use super::ownership_transfer_access as access;
use crate::errors::{AppError, AppResult};
use crate::models::{
    agent_service_binding::COLLECTION_NAME as AGENT_BINDINGS,
    channel_bot::{COLLECTION_NAME as BOTS, ChannelBot},
    channel_conversation::COLLECTION_NAME as CONVERSATIONS,
    downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
    oauth_state::COLLECTION_NAME as OAUTH_STATES,
    ownership_transfer::{COLLECTION_NAME as TRANSFERS, OwnershipTransfer},
    platform_settings::{COLLECTION_NAME as SETTINGS, PLATFORM_SETTINGS_ID},
    provider_config::COLLECTION_NAME as PROVIDERS,
    user::{COLLECTION_NAME as USERS, User, UserType},
    user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey},
    user_provider_token::COLLECTION_NAME as USER_PROVIDER_TOKENS,
    user_service::COLLECTION_NAME as USER_SERVICES,
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Service,
    ChannelBot,
}

impl ResourceKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::Service => "service",
            Self::ChannelBot => "channel_bot",
        }
    }

    pub fn collection(self) -> &'static str {
        match self {
            Self::Service => SERVICES,
            Self::ChannelBot => BOTS,
        }
    }
}

pub fn catalog_owner(service: &DownstreamService) -> &str {
    service
        .owner_user_id
        .as_deref()
        .unwrap_or(&service.created_by)
}

pub fn catalog_owner_filter(owner: &str) -> Document {
    doc! { "$or": [
        { "owner_user_id": owner },
        { "owner_user_id": null, "created_by": owner },
    ] }
}

pub async fn require_current_bot(db: &Database, bot: &ChannelBot) -> AppResult<()> {
    if db
        .collection::<Document>(BOTS)
        .find_one(doc! {
            "_id": &bot.id, "user_id": &bot.user_id, "is_active": true,
            "$expr": { "$eq": [ { "$ifNull": ["$ownership_version", 0_i64] }, bot.ownership_version ] },
        })
        .await?
        .is_none()
    {
        return Err(AppError::Conflict(
            "Channel bot changed; retry with its current owner and routes".into(),
        ));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub struct TransferPreview {
    pub kind: ResourceKind,
    pub resource_id: String,
    pub name: String,
    pub previous_owner_user_id: String,
    pub previous_owner_name: String,
    pub new_owner_user_id: String,
    pub destination_name: String,
    pub destination_type: &'static str,
    pub version: String,
    pub routes_to_retire: u64,
    pub updated_at: chrono::DateTime<Utc>,
    pub blockers: Vec<String>,
    pub moves_oauth_credential: bool,
    pub(crate) oauth_credential_id: Option<String>,
    pub(crate) oauth_connection_id: Option<String>,
    pub(crate) oauth_credential_updated_at: Option<chrono::DateTime<Utc>>,
}

async fn dependency_documents(
    db: &Database,
    session: &mut ClientSession,
    collection: &str,
    filter: Document,
    projection: Document,
) -> AppResult<Vec<Document>> {
    let mut cursor = db
        .collection::<Document>(collection)
        .find(filter)
        .projection(projection)
        .sort(doc! { "_id": 1 })
        .session(&mut *session)
        .await?;
    let mut documents = Vec::new();
    while cursor.advance(&mut *session).await? {
        documents.push(cursor.deserialize_current()?);
    }
    Ok(documents)
}

fn tag_dependencies(kind: &str, documents: Vec<Document>, version_material: &mut Vec<Document>) {
    for document in documents {
        version_material.push(doc! { "kind": kind, "value": document });
    }
}

async fn inspect_x_credential(
    db: &Database,
    session: &mut ClientSession,
    bot: &ChannelBot,
    blockers: &mut Vec<String>,
    version_material: &mut Vec<Document>,
    owned_refresh_lease: Option<&crate::services::coordination_service::LeaseToken>,
) -> AppResult<(
    Option<String>,
    Option<String>,
    Option<chrono::DateTime<Utc>>,
    bool,
)> {
    if bot.credential_source != "connection" {
        blockers
            .push("X ownership transfer requires its dedicated OAuth connection credential".into());
        return Ok((None, None, None, false));
    }
    let Some(key_id) = bot.connection_id.as_deref() else {
        blockers.push("The X bot is missing its OAuth connection credential".into());
        return Ok((None, None, None, false));
    };
    let Some(raw_key) = db
        .collection::<Document>(USER_API_KEYS)
        .find_one(doc! { "_id": key_id })
        .session(&mut *session)
        .await?
    else {
        blockers.push("The X bot's OAuth connection credential no longer exists".into());
        return Ok((Some(key_id.into()), None, None, false));
    };
    version_material.push(doc! { "kind": "oauth_credential", "value": raw_key.clone() });
    let key: UserApiKey = bson::from_document(raw_key).map_err(|error| {
        AppError::Internal(format!("Invalid X OAuth credential record: {error}"))
    })?;
    let mut candidate = true;
    if key.user_id != bot.user_id {
        blockers.push("The X OAuth credential belongs to a different owner".into());
        candidate = false;
    }
    if key.source.as_deref() != Some("channel_onboarding") {
        blockers.push(
            "Only the dedicated credential created by X channel onboarding can move with the bot"
                .into(),
        );
        candidate = false;
    }
    if key.credential_type != "oauth2"
        || key.credential_source.as_deref() != Some("platform")
        || key.user_oauth_client_id_encrypted.is_some()
        || key.user_oauth_client_secret_encrypted.is_some()
    {
        blockers.push("The X connection is not a NyxID-managed OAuth credential".into());
        candidate = false;
    }
    if key.status != "active"
        || key
            .access_token_encrypted
            .as_deref()
            .is_none_or(<[u8]>::is_empty)
        || key
            .refresh_token_encrypted
            .as_deref()
            .is_none_or(<[u8]>::is_empty)
    {
        blockers.push("The X OAuth credential is not active with live refresh access".into());
        candidate = false;
    }
    if crate::services::channel_adapters::x::REQUIRED_SCOPES
        .iter()
        .any(|required| {
            !key.token_scopes
                .as_deref()
                .unwrap_or_default()
                .split_whitespace()
                .any(|scope| scope == *required)
        })
    {
        blockers.push("The X OAuth credential is missing required channel permissions".into());
        candidate = false;
    }
    let Some(connection_id) = key.connection_id.as_deref() else {
        blockers.push("The X OAuth credential has no callback connection handle".into());
        return Ok((Some(key.id), None, Some(key.updated_at), false));
    };
    if key.source_id.as_deref() != Some(connection_id) {
        blockers.push("The X OAuth credential has inconsistent onboarding provenance".into());
        candidate = false;
    }
    if key.oauth_attempt_nonce.is_some() {
        blockers.push("An X OAuth authorization attempt is still in progress".into());
    }

    let refresh_lease_name =
        crate::services::user_token_service::user_api_key_refresh_lease_name(&key.id);
    let refresh_lease = db
        .collection::<Document>(crate::models::coordination::LEASE_COLLECTION_NAME)
        .find_one(doc! {
            "_id": &refresh_lease_name,
            "expires_at": { "$gt": bson::DateTime::now() },
        })
        .projection(doc! { "_id": 1, "lease_id": 1, "expires_at": 1 })
        .session(&mut *session)
        .await?;
    if let Some(refresh_lease) = refresh_lease
        && owned_refresh_lease.is_none_or(|owned| {
            refresh_lease.get_str("lease_id").ok() != Some(owned.lease_id.as_str())
        })
    {
        blockers.push("The X OAuth credential is currently refreshing".into());
        version_material.push(doc! { "kind": "oauth_refresh_lease", "value": refresh_lease });
    }

    let provider = match key.provider_config_id.as_deref() {
        Some(provider_id) => {
            db.collection::<Document>(PROVIDERS)
                .find_one(doc! { "_id": provider_id })
                .session(&mut *session)
                .await?
        }
        None => None,
    };
    if let Some(provider) = provider {
        let supported = provider.get_str("slug").ok() == Some("twitter")
            && provider.get_bool("is_active").unwrap_or(false);
        version_material.push(doc! { "kind": "oauth_provider", "value": provider });
        if !supported {
            blockers
                .push("The X OAuth credential is not bound to the active Twitter provider".into());
            candidate = false;
        }
    } else {
        blockers.push("The X OAuth provider is missing or inactive".into());
        candidate = false;
    }

    let services = dependency_documents(
        db,
        session,
        USER_SERVICES,
        doc! { "api_key_id": &key.id },
        doc! { "_id": 1, "user_id": 1, "api_key_id": 1, "is_active": 1 },
    )
    .await?;
    if !services.is_empty() {
        blockers.push("The X OAuth credential is also used by an AI service".into());
    }
    tag_dependencies("user_service", services, version_material);

    let bindings = dependency_documents(
        db,
        session,
        AGENT_BINDINGS,
        doc! { "user_api_key_id": &key.id },
        doc! { "_id": 1, "user_id": 1, "api_key_id": 1, "user_service_id": 1, "user_api_key_id": 1 },
    )
    .await?;
    if !bindings.is_empty() {
        blockers.push("The X OAuth credential is also used by an agent service binding".into());
    }
    tag_dependencies("agent_service_binding", bindings, version_material);

    let other_bots = dependency_documents(
        db,
        session,
        BOTS,
        doc! { "_id": { "$ne": &bot.id }, "connection_id": &key.id },
        doc! { "_id": 1, "user_id": 1, "platform": 1, "is_active": 1, "connection_id": 1 },
    )
    .await?;
    if !other_bots.is_empty() {
        blockers.push("The X OAuth credential is shared by another channel bot".into());
    }
    tag_dependencies("channel_bot", other_bots, version_material);

    let handles = key
        .source_id
        .iter()
        .chain(key.connection_id.iter())
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let provider_tokens = dependency_documents(
        db,
        session,
        USER_PROVIDER_TOKENS,
        doc! { "connection_id": { "$in": &handles } },
        doc! { "_id": 1, "user_id": 1, "provider_config_id": 1, "connection_id": 1, "status": 1 },
    )
    .await?;
    if !provider_tokens.is_empty() {
        blockers.push("The X OAuth handle is shared by a legacy provider connection".into());
    }
    tag_dependencies("user_provider_token", provider_tokens, version_material);

    let oauth_states = dependency_documents(
        db,
        session,
        OAUTH_STATES,
        doc! {
            "connection_id": { "$in": &handles },
            "expires_at": { "$gt": bson::DateTime::now() },
        },
        doc! { "_id": 1, "user_id": 1, "target_user_id": 1, "provider_config_id": 1, "connection_id": 1, "attempt_nonce": 1, "consumed": 1, "expires_at": 1 },
    )
    .await?;
    if !oauth_states.is_empty() {
        blockers.push("An X OAuth callback or device authorization is still in progress".into());
    }
    tag_dependencies("oauth_state", oauth_states, version_material);

    Ok((
        Some(key.id),
        Some(connection_id.to_string()),
        Some(key.updated_at),
        candidate,
    ))
}

pub async fn require_platform_admin(db: &Database, actor: &str) -> AppResult<()> {
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": actor, "is_active": true })
        .await?
        .ok_or_else(|| AppError::Forbidden("Active NyxID admin access required".into()))?;
    if user.user_type != UserType::Person
        || !super::role_service::resolve_platform_role(db, &user)
            .await?
            .is_admin()
    {
        return Err(AppError::Forbidden("NyxID admin access required".into()));
    }
    Ok(())
}

fn valid_id(value: &str) -> AppResult<()> {
    Uuid::parse_str(value)
        .map(|_| ())
        .map_err(|_| AppError::ValidationError("IDs must be UUIDs".into()))
}

async fn inspect(
    db: &Database,
    session: &mut ClientSession,
    kind: ResourceKind,
    resource_id: &str,
    destination: &str,
    capacity: u32,
    owned_refresh_lease: Option<&crate::services::coordination_service::LeaseToken>,
) -> AppResult<TransferPreview> {
    valid_id(resource_id)?;
    valid_id(destination)?;
    let destination_user = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": destination, "is_active": true })
        .session(&mut *session)
        .await?
        .ok_or_else(|| {
            AppError::ValidationError("Destination must be an active person or organization".into())
        })?;
    let raw = db
        .collection::<Document>(kind.collection())
        .find_one(doc! { "_id": resource_id, "is_active": true })
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::NotFound("Active resource not found".into()))?;
    let mut blockers = Vec::new();
    let mut routes_to_retire = 0;
    let mut version_material = Vec::new();
    let mut oauth_credential_id = None;
    let mut oauth_connection_id = None;
    let mut oauth_credential_updated_at = None;
    let mut moves_oauth_credential = false;
    let (owner, name) = match kind {
        ResourceKind::Service => {
            let service: DownstreamService = bson::from_document(raw.clone())
                .map_err(|e| AppError::Internal(format!("Invalid resource record: {e}")))?;
            if Uuid::parse_str(&service.created_by).is_err() {
                blockers.push("Seeded platform services remain owned by NyxID".into());
            }
            if service.service_type == "ssh" {
                blockers.push("SSH catalog records belong to their connected services and cannot be transferred independently".into());
            }
            if service.oauth_client_id.is_some() || service.auth_method == "oidc" {
                blockers.push("OIDC services require a separate OAuth client handover and cannot yet be transferred".into());
            }
            if super::retired_service_service::require_available(&service).is_err() {
                blockers.push("Retired platform services cannot be transferred".into());
            }
            (catalog_owner(&service).to_owned(), service.name)
        }
        ResourceKind::ChannelBot => {
            let bot: ChannelBot = bson::from_document(raw.clone())
                .map_err(|e| AppError::Internal(format!("Invalid resource record: {e}")))?;
            if bot.platform == "x" {
                (
                    oauth_credential_id,
                    oauth_connection_id,
                    oauth_credential_updated_at,
                    moves_oauth_credential,
                ) = inspect_x_credential(
                    db,
                    session,
                    &bot,
                    &mut blockers,
                    &mut version_material,
                    owned_refresh_lease,
                )
                .await?;
            } else {
                if bot.connection_id.is_some() {
                    blockers.push("This bot uses an owner-bound connection whose lifecycle cannot yet be transferred".into());
                }
                if !matches!(
                    bot.platform.as_str(),
                    "telegram" | "discord" | "lark" | "feishu" | "slack" | "whatsapp"
                ) {
                    blockers.push("This managed channel has owner-bound registration state. Reconnect it under the destination owner instead".into());
                }
            }
            if bot.poll_lease_until.is_some_and(|until| until > Utc::now()) {
                blockers.push("A channel poll is running. Retry after it completes".into());
            }
            if db
                .collection::<Document>(BOTS)
                .count_documents(doc! { "user_id": destination, "is_active": true })
                .session(&mut *session)
                .await?
                >= u64::from(capacity)
            {
                blockers.push("The destination has reached its channel bot limit".into());
            }
            let routes = dependency_documents(
                db,
                session,
                CONVERSATIONS,
                doc! { "channel_bot_id": resource_id, "retired_by_transfer": { "$ne": true } },
                doc! { "_id": 1, "user_id": 1, "is_active": 1, "default_agent": 1,
                "allow_agent_initiated": 1, "retired_by_transfer": 1 },
            )
            .await?;
            routes_to_retire = routes.len() as u64;
            tag_dependencies("route", routes, &mut version_material);
            (bot.user_id, bot.label)
        }
    };
    if owner == destination {
        blockers.push("The resource already belongs to this owner".into());
    }
    let previous_owner_name = db
        .collection::<Document>(USERS)
        .find_one(doc! { "_id": &owner })
        .projection(doc! { "display_name": 1, "email": 1 })
        .session(&mut *session)
        .await?
        .and_then(|user| {
            user.get_str("display_name")
                .or_else(|_| user.get_str("email"))
                .ok()
                .map(str::to_owned)
        })
        .unwrap_or_else(|| owner.clone());
    let mut hash = Sha256::new();
    hash.update(b"nyxid-ownership-transfer-v1\0");
    hash.update(
        bson::to_vec(&raw)
            .map_err(|e| AppError::Internal(format!("Cannot version resource: {e}")))?,
    );
    hash.update(destination.as_bytes());
    hash.update(routes_to_retire.to_be_bytes());
    for material in version_material {
        hash.update(
            bson::to_vec(&material)
                .map_err(|e| AppError::Internal(format!("Cannot version dependency: {e}")))?,
        );
    }
    let destination_type = if destination_user.user_type == UserType::Org {
        "org"
    } else {
        "person"
    };
    Ok(TransferPreview {
        kind,
        resource_id: resource_id.into(),
        name,
        previous_owner_user_id: owner,
        previous_owner_name,
        new_owner_user_id: destination.into(),
        destination_name: destination_user
            .display_name
            .unwrap_or(destination_user.email),
        destination_type,
        version: hex::encode(hash.finalize()),
        routes_to_retire,
        updated_at: raw
            .get_datetime("updated_at")
            .map_err(|_| AppError::Internal("Resource timestamp missing".into()))?
            .to_chrono(),
        blockers,
        moves_oauth_credential,
        oauth_credential_id,
        oauth_connection_id,
        oauth_credential_updated_at,
    })
}

#[cfg(test)]
pub async fn preview(
    db: &Database,
    actor: &str,
    kind: ResourceKind,
    resource_id: &str,
    destination: &str,
    capacity: u32,
) -> AppResult<TransferPreview> {
    preview_with_agent(db, actor, None, kind, resource_id, destination, capacity).await
}

pub async fn preview_with_agent(
    db: &Database,
    actor: &str,
    api_key_id: Option<&str>,
    kind: ResourceKind,
    resource_id: &str,
    destination: &str,
    capacity: u32,
) -> AppResult<TransferPreview> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    let owner = access::resource_owner(db, &mut session, kind, resource_id).await?;
    access::authorize(db, &mut session, actor, api_key_id, &owner, false).await?;
    let result = inspect(
        db,
        &mut session,
        kind,
        resource_id,
        destination,
        capacity,
        None,
    )
    .await;
    session.abort_transaction().await?;
    result
}

pub struct TransferCommand<'a> {
    pub actor: &'a str,
    pub api_key_id: Option<&'a str>,
    pub kind: ResourceKind,
    pub resource_id: &'a str,
    pub destination: &'a str,
    pub request_id: &'a str,
    pub expected_version: &'a str,
    pub capacity: u32,
}

pub async fn transfer(db: &Database, command: TransferCommand<'_>) -> AppResult<OwnershipTransfer> {
    let TransferCommand {
        actor,
        api_key_id,
        kind,
        resource_id,
        destination,
        request_id,
        expected_version,
        capacity,
    } = command;
    valid_id(request_id)?;
    if Uuid::parse_str(request_id).is_ok_and(|id| id.get_version_num() != 4) {
        return Err(AppError::ValidationError(
            "Transfer request ID must be a UUID v4".into(),
        ));
    }
    valid_id(resource_id)?;
    valid_id(destination)?;
    if expected_version.len() != 64 || !expected_version.bytes().all(|c| c.is_ascii_hexdigit()) {
        return Err(AppError::ValidationError(
            "A valid transfer preview is required".into(),
        ));
    }
    let db = db.clone();
    let actor = actor.to_owned();
    let api_key_id = api_key_id.map(str::to_owned);
    let resource_id = resource_id.to_owned();
    let destination = destination.to_owned();
    let request_id = request_id.to_owned();
    let expected_version = expected_version.to_owned();
    let receipt_matches = {
        let actor = actor.clone();
        let api_key_id = api_key_id.clone();
        let resource_id = resource_id.clone();
        let destination = destination.clone();
        let expected_version = expected_version.clone();
        move |receipt: &OwnershipTransfer| {
            receipt.actor_user_id == actor
                && receipt.actor_api_key_id == api_key_id
                && receipt.resource_kind == kind.name()
                && receipt.resource_id == resource_id
                && receipt.new_owner_user_id == destination
                && receipt.preview_version == expected_version
        }
    };
    if let Some(receipt) = db
        .collection::<OwnershipTransfer>(TRANSFERS)
        .find_one(doc! { "_id": &request_id })
        .await?
    {
        return if receipt_matches(&receipt) {
            let mut session = db.client().start_session().await?;
            access::authorize(
                &db,
                &mut session,
                &actor,
                api_key_id.as_deref(),
                &receipt.previous_owner_user_id,
                false,
            )
            .await?;
            Ok(receipt)
        } else {
            Err(AppError::Conflict(
                "Transfer request ID was already used for another operation".into(),
            ))
        };
    }

    access::require_resource_access(&db, &actor, api_key_id.as_deref(), kind, &resource_id).await?;
    let refresh_key_id = if kind == ResourceKind::ChannelBot {
        db.collection::<Document>(BOTS)
            .find_one(doc! { "_id": &resource_id, "platform": "x", "is_active": true })
            .projection(doc! { "connection_id": 1 })
            .await?
            .and_then(|bot| bot.get_str("connection_id").ok().map(str::to_owned))
    } else {
        None
    };
    let refresh_runtime = crate::services::coordination_service::cluster_lease_runtime();
    let refresh_lease = if let Some(key_id) = refresh_key_id.as_deref() {
        let name = crate::services::user_token_service::user_api_key_refresh_lease_name(key_id);
        Some(refresh_runtime.acquire(&db, &name).await?.ok_or_else(|| {
            AppError::Conflict(
                "The X OAuth credential is refreshing. Retry the transfer shortly".into(),
            )
        })?)
    } else {
        None
    };
    let fresh_oauth_handle = Uuid::new_v4().to_string();
    let transaction_db = db.clone();
    let operation = {
        let db = db.clone();
        let request_id = request_id.clone();
        let receipt_matches = receipt_matches.clone();
        let refresh_lease = refresh_lease.clone();
        crate::services::service_history::transaction::run(
            &transaction_db,
            async move |transaction| {
                let result: AppResult<OwnershipTransfer> = async {
            if let Some(receipt) = db
                .collection::<OwnershipTransfer>(TRANSFERS)
                .find_one(doc! { "_id": &request_id })
                .session(&mut **transaction)
                .await?
            {
                if !receipt_matches(&receipt) {
                    return Err(AppError::Conflict(
                        "Transfer request ID was already used for another operation".into(),
                    ));
                }
                access::authorize(&db, &mut **transaction, &actor, api_key_id.as_deref(), &receipt.previous_owner_user_id, false).await?;
                return Ok(receipt);
            }
            let owner = access::resource_owner(&db, &mut **transaction, kind, &resource_id).await?;
            access::authorize(&db, &mut **transaction, &actor, api_key_id.as_deref(), &owner, true).await?;
            let reviewed = inspect(
                &db,
                &mut **transaction,
                kind,
                &resource_id,
                &destination,
                capacity,
                refresh_lease.as_ref(),
            )
            .await?;
            if reviewed.version != expected_version {
                return Err(AppError::Conflict(
                    "The resource changed. Review a fresh transfer preview".into(),
                ));
            }
            if reviewed.moves_oauth_credential
                && reviewed.oauth_credential_id.as_deref() != refresh_key_id.as_deref()
            {
                return Err(AppError::Conflict(
                    "The X OAuth credential changed. Review a fresh transfer preview".into(),
                ));
            }
            if !reviewed.blockers.is_empty() {
                return Err(AppError::Conflict(reviewed.blockers.join(". ")));
            }
            let owner = db
                .collection::<Document>(USERS)
                .update_one(
                    doc! { "_id": &destination, "is_active": true },
                    doc! { "$inc": { "ownership_transfer_revision": 1_i64 } },
                )
                .session(&mut **transaction)
                .await?;
            if owner.matched_count != 1 {
                return Err(AppError::Conflict("Destination is no longer active".into()));
            }
            let latest_timestamp = reviewed
                .oauth_credential_updated_at
                .unwrap_or(reviewed.updated_at)
                .max(reviewed.updated_at)
                .timestamp_millis()
                .saturating_add(1);
            let now = bson::DateTime::from_millis(
                bson::DateTime::now().timestamp_millis().max(latest_timestamp),
            );

            if let Some(key_id) = reviewed.oauth_credential_id.as_deref() {
                let old_connection_id = reviewed.oauth_connection_id.as_deref().ok_or_else(|| {
                    AppError::Conflict(
                        "The X OAuth credential changed. Review a fresh transfer preview".into(),
                    )
                })?;
                let key_update = crate::services::service_history::collection::<UserApiKey>(
                    &db,
                    USER_API_KEYS,
                )
                .update_one(
                    doc! {
                        "_id": key_id,
                        "user_id": &reviewed.previous_owner_user_id,
                        "source": "channel_onboarding",
                        "source_id": old_connection_id,
                        "connection_id": old_connection_id,
                        "status": "active",
                    },
                    doc! { "$set": {
                        "user_id": &destination,
                        "connection_id": &fresh_oauth_handle,
                        "source_id": &fresh_oauth_handle,
                        "updated_at": now,
                    } },
                )
                .session(&mut *transaction)
                .await?;
                if key_update.matched_count != 1 {
                    return Err(AppError::Conflict(
                        "The X OAuth credential changed. Review a fresh transfer preview".into(),
                    ));
                }
            }

            let mut retired_routes = 0;
            let owner_field = match kind {
                ResourceKind::Service => "owner_user_id",
                ResourceKind::ChannelBot => {
                    db.collection::<Document>(SETTINGS)
                        .update_one(
                            doc! { "_id": PLATFORM_SETTINGS_ID },
                            doc! { "$inc": { "channel_bot_registration_revision": 1_i64 } },
                        )
                        .upsert(true)
                        .session(&mut **transaction)
                        .await?;
                    retired_routes = db
                        .collection::<Document>(CONVERSATIONS)
                        .update_many(
                            doc! { "channel_bot_id": &resource_id, "retired_by_transfer": { "$ne": true } },
                            doc! { "$set": { "is_active": false, "default_agent": false,
                                "allow_agent_initiated": false, "retired_by_transfer": true, "updated_at": now } },
                        )
                        .session(&mut **transaction)
                        .await?
                        .modified_count;
                    if retired_routes != reviewed.routes_to_retire {
                        return Err(AppError::Conflict(
                            "Channel routes changed. Review a fresh transfer preview".into(),
                        ));
                    }
                    "user_id"
                }
            };
            let mut set = doc! { "updated_at": now };
            set.insert(owner_field, &destination);
            let mut update = doc! { "$set": set };
            if kind == ResourceKind::ChannelBot {
                update.insert("$inc", doc! { "ownership_version": 1_i64 });
            }
            let mut resource_filter = doc! { "_id": &resource_id, "is_active": true };
            match kind {
                ResourceKind::Service => {
                    resource_filter.insert(
                        "$and",
                        vec![catalog_owner_filter(&reviewed.previous_owner_user_id)],
                    );
                }
                ResourceKind::ChannelBot => {
                    resource_filter.insert("user_id", &reviewed.previous_owner_user_id);
                }
            }
            let resource_update = db
                .collection::<Document>(kind.collection())
                .update_one(resource_filter, update)
                .session(&mut **transaction)
                .await?;
            if resource_update.matched_count != 1 {
                return Err(AppError::Conflict(
                    "The resource owner changed. Review a fresh transfer preview".into(),
                ));
            }
            let receipt = OwnershipTransfer {
                id: request_id.clone(),
                actor_user_id: actor.clone(),
                actor_api_key_id: api_key_id.clone(),
                resource_kind: kind.name().into(),
                resource_id: resource_id.clone(),
                previous_owner_user_id: reviewed.previous_owner_user_id,
                new_owner_user_id: destination.clone(),
                preview_version: expected_version.clone(),
                retired_routes,
                created_at: now.to_chrono(),
            };
            db.collection::<OwnershipTransfer>(TRANSFERS)
                .insert_one(&receipt)
                .session(&mut **transaction)
                .await?;
            Ok(receipt)
        }
        .await;
                transaction_result(result)
            },
        )
    };
    let result = if let Some(lease) = refresh_lease.as_ref() {
        refresh_runtime
            .run_while_renewed(&db, lease, operation)
            .await
    } else {
        Some(operation.await)
    };
    if let Some(lease) = refresh_lease.as_ref()
        && let Err(error) =
            crate::services::coordination_service::LeaseStore::release(&db, lease).await
    {
        tracing::warn!(lease_name = %lease.name, error = %error, "Failed to release ownership transfer OAuth lease");
    }
    match result {
        Some(result) => result.map_err(map_transaction_error),
        None => {
            let receipt = db
                .collection::<OwnershipTransfer>(TRANSFERS)
                .find_one(doc! { "_id": &request_id })
                .await?;
            match receipt {
                Some(receipt) if receipt_matches(&receipt) => Ok(receipt),
                Some(_) => Err(AppError::Conflict(
                    "Transfer request ID was already used for another operation".into(),
                )),
                None => Err(AppError::Internal(
                    "Ownership transfer outcome is uncertain; retry with the same request ID"
                        .into(),
                )),
            }
        }
    }
}

pub async fn list_resources(
    db: &Database,
    kind: ResourceKind,
    owner: Option<&str>,
    search: Option<&str>,
    offset: u64,
) -> AppResult<Vec<Document>> {
    let mut filter = doc! { "is_active": true };
    if let Some(owner) = owner {
        valid_id(owner)?;
        match kind {
            ResourceKind::Service => {
                filter.insert("$and", vec![catalog_owner_filter(owner)]);
            }
            ResourceKind::ChannelBot => {
                filter.insert("user_id", owner);
            }
        }
    }
    if let Some(search) = search.filter(|value| !value.is_empty()) {
        if search.len() > 200 {
            return Err(AppError::ValidationError("Search is too long".into()));
        }
        let pattern = regex::escape(search);
        filter.insert(
            "$or",
            vec![
                doc! { "name": { "$regex": &pattern, "$options": "i" } },
                doc! { "label": { "$regex": &pattern, "$options": "i" } },
                doc! { "slug": { "$regex": &pattern, "$options": "i" } },
                doc! { "_id": search },
            ],
        );
    }
    Ok(db
        .collection::<Document>(kind.collection())
        .find(filter)
        .projection(
            doc! { "_id": 1, "name": 1, "label": 1, "slug": 1, "platform": 1,
            "created_by": 1, "owner_user_id": 1, "user_id": 1 },
        )
        .sort(doc! { "_id": 1 })
        .skip(offset)
        .limit(51)
        .await?
        .try_collect()
        .await?)
}
