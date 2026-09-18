use bson::{Document, doc};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::api_key_mutation_service::{map_transaction_error, transaction_result};
use crate::errors::{AppError, AppResult};
use crate::models::{
    channel_bot::{COLLECTION_NAME as BOTS, ChannelBot},
    channel_conversation::COLLECTION_NAME as CONVERSATIONS,
    downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
    ownership_transfer::{COLLECTION_NAME as TRANSFERS, OwnershipTransfer},
    platform_settings::{COLLECTION_NAME as SETTINGS, PLATFORM_SETTINGS_ID},
    user::{COLLECTION_NAME as USERS, User, UserType},
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
            "updated_at": bson::DateTime::from_chrono(bot.updated_at),
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
            if bot.connection_id.is_some() {
                blockers.push("This bot uses an owner-bound OAuth connection. Reconnect it under the destination owner instead".into());
            }
            if !matches!(
                bot.platform.as_str(),
                "telegram" | "discord" | "lark" | "feishu" | "slack" | "whatsapp"
            ) {
                blockers.push("This managed channel has owner-bound registration state. Reconnect it under the destination owner instead".into());
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
            routes_to_retire = db
                .collection::<Document>(CONVERSATIONS)
                .count_documents(
                    doc! { "channel_bot_id": resource_id, "retired_by_transfer": { "$ne": true } },
                )
                .session(&mut *session)
                .await?;
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
    })
}

pub async fn preview(
    db: &Database,
    actor: &str,
    kind: ResourceKind,
    resource_id: &str,
    destination: &str,
    capacity: u32,
) -> AppResult<TransferPreview> {
    require_platform_admin(db, actor).await?;
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    let result = inspect(db, &mut session, kind, resource_id, destination, capacity).await;
    session.abort_transaction().await?;
    result
}

pub struct TransferCommand<'a> {
    pub actor: &'a str,
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
        kind,
        resource_id,
        destination,
        request_id,
        expected_version,
        capacity,
    } = command;
    require_platform_admin(db, actor).await?;
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
    let resource_id = resource_id.to_owned();
    let destination = destination.to_owned();
    let request_id = request_id.to_owned();
    let expected_version = expected_version.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<OwnershipTransfer> = async {
            if let Some(receipt) = db.collection::<OwnershipTransfer>(TRANSFERS)
                .find_one(doc! { "_id": &request_id }).session(&mut *session).await?
            {
                if receipt.actor_user_id != actor || receipt.resource_kind != kind.name()
                    || receipt.resource_id != resource_id || receipt.new_owner_user_id != destination
                    || receipt.preview_version != expected_version
                {
                    return Err(AppError::Conflict("Transfer request ID was already used for another operation".into()));
                }
                return Ok(receipt);
            }
            let reviewed = inspect(&db, session, kind, &resource_id, &destination, capacity).await?;
            if reviewed.version != expected_version {
                return Err(AppError::Conflict("The resource changed. Review a fresh transfer preview".into()));
            }
            if !reviewed.blockers.is_empty() {
                return Err(AppError::Conflict(reviewed.blockers.join(". ")));
            }
            // Conflict with destination disable/delete and with bot registrations
            // using the existing global capacity fence.
            let owner = db.collection::<Document>(USERS).update_one(
                doc! { "_id": &destination, "is_active": true },
                doc! { "$inc": { "ownership_transfer_revision": 1_i64 } },
            ).session(&mut *session).await?;
            if owner.matched_count != 1 {
                return Err(AppError::Conflict("Destination is no longer active".into()));
            }
            let now = bson::DateTime::from_millis(bson::DateTime::now().timestamp_millis().max(reviewed.updated_at.timestamp_millis().saturating_add(1)));
            let mut retired_routes = 0;
            let owner_field = match kind {
                ResourceKind::Service => "owner_user_id",
                ResourceKind::ChannelBot => {
                    db.collection::<Document>(SETTINGS).update_one(
                        doc! { "_id": PLATFORM_SETTINGS_ID },
                        doc! { "$inc": { "channel_bot_registration_revision": 1_i64 } },
                    ).upsert(true).session(&mut *session).await?;
                    retired_routes = db.collection::<Document>(CONVERSATIONS).update_many(
                        doc! { "channel_bot_id": &resource_id, "retired_by_transfer": { "$ne": true } },
                        doc! { "$set": { "is_active": false, "default_agent": false,
                            "allow_agent_initiated": false, "retired_by_transfer": true, "updated_at": now } },
                    ).session(&mut *session).await?.modified_count;
                    "user_id"
                }
            };
            let mut set = doc! { "updated_at": now };
            set.insert(owner_field, &destination);
            db.collection::<Document>(kind.collection()).update_one(
                doc! { "_id": &resource_id, "is_active": true }, doc! { "$set": set },
            ).session(&mut *session).await?;
            let receipt = OwnershipTransfer {
                id: request_id.clone(), actor_user_id: actor.clone(), resource_kind: kind.name().into(),
                resource_id: resource_id.clone(), previous_owner_user_id: reviewed.previous_owner_user_id,
                new_owner_user_id: destination.clone(), preview_version: expected_version.clone(),
                retired_routes, created_at: now.to_chrono(),
            };
            db.collection::<OwnershipTransfer>(TRANSFERS).insert_one(&receipt).session(&mut *session).await?;
            Ok(receipt)
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
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
