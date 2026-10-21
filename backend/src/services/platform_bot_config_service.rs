#![allow(
    clippy::collapsible_if,
    clippy::too_many_arguments,
    clippy::needless_borrow,
    clippy::items_after_test_module
)]

use chrono::Utc;
use mongodb::options::ReturnDocument;
use mongodb::{
    Database, IndexModel,
    bson::{self, Document, doc},
};

use super::api_key_mutation_service::{map_transaction_error, transaction_result};
use crate::{
    errors::{AppError, AppResult},
    models::platform_bot_config::{COLLECTION_NAME, PlatformBotConfig, PlatformExecutionProfile},
};

/// Canonical live platform RBAC plus a write conflict fence on the acting person.
/// The handler separately verifies the first-party human session.
/// Whether any manager has been converted into a platform manager profile.
pub(crate) async fn any_profile(db: &Database) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"native_entry":bson::Bson::Null})
        .projection(doc! {"_id": 1})
        .await?
        .is_some())
}

pub(crate) async fn fence_funding_admin(
    db: &Database,
    actor: &str,
    funding: &str,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    if actor == funding {
        return fence_admin(db, actor, session).await;
    }
    if db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":funding,"user_type":"org","is_active":true},
            doc! {"$inc":{"platform_funding_revision":1_i64}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(AppError::ValidationError(
            "Platform funding requires an active dedicated organization wallet".into(),
        ));
    }
    if db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(doc! {"org_user_id":funding,"member_user_id":actor,"role":"admin","revoked_at":bson::Bson::Null},
            doc! {"$inc":{"platform_funding_revision":1_i64}})
        .session(&mut *session).await?.matched_count != 1 {
        return Err(AppError::Forbidden("Funding organization authority changed".into()));
    }
    Ok(())
}

fn require_first_party(first_party: bool) -> AppResult<()> {
    if !first_party {
        return Err(AppError::Forbidden(
            "A converted platform manager is changed only by a first-party administrator".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn admit_manager_admin(
    db: &Database,
    actor: &str,
    first_party: bool,
    manager: &str,
) -> AppResult<()> {
    let scoped = db
        .collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
        .find_one(doc! {"provider":format!("telegram-new/{manager}")})
        .await?
        .is_some();
    if db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"manager_bot_id": manager,"native_entry":bson::Bson::Null})
        .await?
        .is_none()
        && !scoped
    {
        return Ok(());
    }
    let db = db.clone();
    let actor = actor.to_owned();
    let manager = manager.to_owned();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                if db
                    .collection::<Document>(COLLECTION_NAME)
                    .find_one(doc! {"manager_bot_id": &manager,"native_entry":bson::Bson::Null})
                    .session(&mut *session)
                    .await?
                    .is_some()
                    || scoped
                {
                    require_first_party(first_party)?;
                    fence_admin(&db, &actor, session).await?;
                }
                Ok(())
            }
            .await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

pub(crate) async fn finalize_manager_delivery(
    db: &Database,
    actor: &str,
    first_party: bool,
    manager: &str,
    observation_started_at: String,
) -> AppResult<()> {
    let scoped = db
        .collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
        .find_one(doc! {"provider":format!("telegram-new/{manager}")})
        .await?
        .is_some();
    if db
        .collection::<Document>(COLLECTION_NAME)
        .find_one(doc! {"manager_bot_id": manager,"native_entry":bson::Bson::Null})
        .await?
        .is_none()
        && !scoped
    {
        db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME).update_one(
            doc! {"provider": {"$in":["telegram-new".to_owned(), format!("telegram-new/{}", manager)]}, "fields.manager_bot_id": manager},
            doc! {"$set": {"fields.webhook_ready": "true", "fields.observation_started_at": observation_started_at}}
        ).await?;
        return Ok(());
    }
    let db = db.clone();
    let actor = actor.to_owned();
    let manager = manager.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            if db.collection::<Document>(COLLECTION_NAME).find_one(doc! {"manager_bot_id": &manager,"native_entry":bson::Bson::Null}).session(&mut *session).await?.is_some() || scoped {
                require_first_party(first_party)?;
                fence_admin(&db, &actor, session).await?;
            }
            if db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME).update_one(
                doc! {"provider": {"$in":["telegram-new".to_owned(), format!("telegram-new/{}", &manager)]}, "fields.manager_bot_id": &manager},
                doc! {"$set": {"fields.webhook_ready": "true", "fields.observation_started_at": &observation_started_at}}
            ).session(&mut *session).await?.matched_count != 1 { return Err(AppError::Conflict("Manager configuration changed".into())); }
            Ok(())
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub(crate) async fn fence_admin(
    db: &Database,
    actor: &str,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    let user = db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id":actor,"is_active":true,"$or":[{"user_type":"person"},{"user_type":{"$exists":false}}]},
            doc! {"$inc": {"platform_admin_authority_revision": 1_i64}},
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Forbidden("Active human administrator required".into()))?;
    if !super::role_service::resolve_platform_role(db, &user)
        .await?
        .is_admin()
    {
        return Err(AppError::Forbidden("Admin access required".into()));
    }
    Ok(())
}

pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    super::platform_entry_migration::ensure_indexes(db).await?;
    // Reservations survive retirement and deletion. The ordinary-route and
    // admin-target lookups must also find those tombstones efficiently.
    db.collection::<Document>(COLLECTION_NAME).create_index(
        IndexModel::builder().keys(doc! {"native_entry.identity.platform":1,"native_entry.identity.platform_bot_id":1}).build()
    ).await?;
    db.collection::<Document>(crate::models::platform_bot_config::CLEANUP_COLLECTION)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"completed": 1, "retry_at": 1})
                .build(),
        )
        .await?;
    Ok(())
}

pub async fn list(db: &Database, before_id: Option<&str>) -> AppResult<Vec<PlatformBotConfig>> {
    use futures::TryStreamExt;
    let filter = before_id.map_or_else(|| doc! {}, |id| doc! {"_id": {"$lt": id}});
    db.collection::<PlatformBotConfig>(COLLECTION_NAME)
        .find(filter)
        .sort(doc! {"_id": -1})
        .limit(51)
        .await?
        .try_collect()
        .await
        .map_err(Into::into)
}

pub(crate) async fn get(db: &Database, id: &str) -> AppResult<PlatformBotConfig> {
    db.collection::<PlatformBotConfig>(COLLECTION_NAME)
        .find_one(doc! {"_id": id})
        .await?
        .ok_or_else(|| AppError::NotFound("Platform bot configuration not found".into()))
}

pub(crate) async fn convert(
    db: &Database,
    actor: &str,
    bot: &str,
    funding: &str,
) -> AppResult<PlatformBotConfig> {
    let row = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id": bot})
        .await?
        .ok_or_else(|| AppError::NotFound("Manager channel not found".into()))?;
    let manager = row
        .get_str("platform_bot_id")
        .map_err(|_| AppError::Conflict("Manager identity unavailable".into()))?;
    super::telegram_new_service::with_operation(
        db,
        &format!("telegram-manager-identity:{manager}"),
        convert_locked(db, actor, bot, funding, None),
    )
    .await
}

/// The admin handler is the only production caller. Conversion is permanent:
/// pause and deletion retain both the manager identity and the bot-row fence.
pub(crate) async fn convert_native(
    state: &crate::AppState,
    authority: &super::platform_entry_admin_service::NativeConversionAuthority,
) -> AppResult<PlatformBotConfig> {
    if let Some(previous) = authority.previous_profile() {
        return update_native(state, authority, previous).await;
    }
    convert_locked(
        &state.db,
        authority.actor(),
        &authority.bot().id,
        authority.funding(),
        Some(authority),
    )
    .await
}

async fn update_native(
    state: &crate::AppState,
    authority: &super::platform_entry_admin_service::NativeConversionAuthority,
    previous: &PlatformBotConfig,
) -> AppResult<PlatformBotConfig> {
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run((state, authority, previous), |session, (state, authority, previous)| Box::pin(async move {
        transaction_result(async {
            authority.fence_in_session(&state.db, session).await?;
            let configuration = authority.configuration();
            if state.db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(
                doc! {"_id":&authority.bot().id,"platform_profile_id":&previous.id,"platform_entry_version":2,"platform_entry_pending.profile_id":&previous.id,"platform_entry_pending.actor":authority.actor()},
                doc! {"$unset":{"platform_entry_pending":""},"$inc":{"route_revision":1_i64}}).session(&mut *session).await?.matched_count!=1 {return Err(AppError::Conflict("Native setup changed; review it again".into()));}
            let updated=state.db.collection::<PlatformBotConfig>(COLLECTION_NAME).find_one_and_update(
                doc! {"_id":&previous.id,"generation":previous.generation,"enabled":false,"deleted_at":bson::Bson::Null},
                doc! {"$set":{"manager_bot_id":&configuration.identity.platform_bot_id,"native_entry":bson::to_bson(configuration).map_err(|_|AppError::Internal("Cannot encode native entry".into()))?,
                    "observation_id":&configuration.credential_fingerprint,"configured_by":authority.actor(),"funding_owner_user_id":authority.funding(),"enabled":false,"updated_at":bson::DateTime::now()},"$inc":{"generation":1_i64}})
                .return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?.ok_or_else(||AppError::Conflict("Native setup changed; review it again".into()))?;
            let entry=super::platform_manager_contract::EntryIdentity::NativeV2 {platform_entry:configuration.identity.clone()};
            let mut fields=super::platform_entry_scope::entry_fields(&entry);
            fields.extend(doc! {"manager_bot_id":&configuration.identity.platform_bot_id,"personal_enabled":false,"public_enabled":false,"updated_at":bson::DateTime::now()});
            state.db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME).update_one(
                doc! {"platform_config_id":&previous.id,"retired":false,"personal_enabled":false,"public_enabled":false},doc! {"$set":fields}).session(&mut *session).await?;
            Ok(updated)
        }.await)
    })).await.map_err(map_transaction_error)
}

/// These deterministic blockers are checked before reserving native conversion
/// and again in the final conversion transaction. Routing mutations serialize
/// against the bot's route_revision claim.
pub(super) async fn conversion_prerequisites_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    bot_id: &str,
) -> AppResult<()> {
    if db
        .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .find_one(doc! {"channel_bot_id": bot_id, "$or": [
            {"status": "pending"},
            {"pending_agent_api_key_id": {"$ne": bson::Bson::Null}},
            {"pending_route_api_key_id": {"$ne": bson::Bson::Null}},
            {"status": "active", "transport": "gateway", "gateway_channel_id": bson::Bson::Null}
        ]})
        .session(&mut *session)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Finish or cancel the channel transport attempt before platform conversion".into(),
        ));
    }
    let mut routes = db
        .collection::<Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .find(doc! {"channel_bot_id":bot_id})
        .session(&mut *session)
        .await?;
    let mut route_keys = Vec::<String>::new();
    while let Some(route) = routes.next(&mut *session).await.transpose()? {
        if let Ok(key) = route.get_str("agent_api_key_id") {
            route_keys.push(key.into());
        }
    }
    // Route callback keys may be shared with the owner's other bots;
    // revoking one would silently break those unrelated routes.
    if db.collection::<Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .find_one(doc! {"agent_api_key_id": {"$in": &route_keys}, "channel_bot_id": {"$ne": bot_id}, "is_active": true})
        .session(&mut *session).await?.is_some() {
        return Err(AppError::Conflict("A manager route key is shared with another bot. Give the manager routes a dedicated key before platform conversion".into()));
    }
    Ok(())
}

async fn convert_locked(
    db: &Database,
    actor: &str,
    channel_bot_id: &str,
    funding_owner: &str,
    native: Option<&super::platform_entry_admin_service::NativeConversionAuthority>,
) -> AppResult<PlatformBotConfig> {
    let access = super::org_service::resolve_owner_access(db, actor, funding_owner).await?;
    if !access.can_write() {
        return Err(AppError::Forbidden(
            "Funding organization write authority required".into(),
        ));
    }
    let id = native
        .map(|authority| authority.profile_id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    let db = db.clone();
    let actor = actor.to_owned();
    let bot_id = channel_bot_id.to_owned();
    let funding = funding_owner.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db, actor, bot_id, funding, id, native), |session, (db, actor, bot_id, funding, id, native)| Box::pin(async move {
        let native = *native;
        let db = db.clone();
        let actor = actor.clone();
        let bot_id = bot_id.clone();
        let funding = funding.clone();
        let id = id.clone();
        let result: AppResult<_> = async {
            fence_admin(&db, &actor, session).await?;
            conversion_prerequisites_in_session(&db,session,&bot_id).await?;
            fence_funding_admin(&db,&actor,&funding,session).await?;
            if let Some(authority) = native { authority.fence_in_session(&db, session).await?; }
            let mut target = doc! {"_id": &bot_id, "is_active": true, "platform_profile_id": {"$exists": false}};
            let mut values = doc! {"platform_profile_id": &id};
            if native.is_some() { values.insert("platform_entry_version",2); }
            else {
                target.insert("credential_source","telegram_manager");
                target.insert("platform_entry_pending",doc! {"$exists":false});
            }
            let bot = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                .find_one_and_update(target,
                    doc! {"$set":values,"$unset":{"platform_entry_pending":""},"$inc":{"route_revision":1_i64}})
                .session(&mut *session).await?
                .ok_or_else(|| AppError::Conflict("Manager channel is unavailable or already platform managed".into()))?;
            let manager_id = bot.get_str("platform_bot_id").map_err(|_| AppError::Conflict("Manager identity unavailable".into()))?;
            let (observation, execution_profile, identity_version, native_entry) = if let Some(authority) = native {
                let configuration=authority.configuration();
                if configuration.identity.channel_bot_id!=bot_id || configuration.identity.platform_bot_id!=manager_id {return Err(AppError::Conflict("Native conversion target changed".into()));}
                (configuration.credential_fingerprint.clone(),PlatformExecutionProfile::NativeEntry,Some(2),Some(configuration.clone()))
            } else {
                let manager = db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
                    .find_one(doc! {"provider": {"$in":["telegram-new".to_owned(), format!("telegram-new/{}", manager_id)]}, "fields.manager_bot_id": manager_id,"fields.webhook_ready":"true"})
                    .session(&mut *session).await?.ok_or_else(||AppError::Conflict("Manager configuration is not ready".into()))?;
                let observation=manager.get_document("fields").ok().and_then(|fields|fields.get_str("observation_id").ok())
                    .ok_or_else(||AppError::Conflict("Manager observation identity unavailable".into()))?.to_owned();
                (observation,PlatformExecutionProfile::PlatformOnboarding,Some(1),None)
            };
            let now = Utc::now();
            let profile = PlatformBotConfig { guide: None,
                id:id.clone(),channel_bot_id:bot_id.clone(),manager_bot_id:manager_id.into(),
                identity_version,native_entry,observation_id:observation,execution_profile,
                generation:1,enabled:false,funding_owner_user_id:funding.clone(),configured_by:actor.clone(),
                created_at:now,updated_at:now,deleted_at:None };
            db.collection::<PlatformBotConfig>(COLLECTION_NAME).insert_one(&profile).session(&mut *session).await?;

            let mut key_ids = Vec::<String>::new();
            let mut channel_ids = Vec::<String>::new();
            let mut channels = db.collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
                .find(doc! {"channel_bot_id": &bot_id}).session(&mut *session).await?;
            while let Some(channel) = channels.next(&mut *session).await.transpose()? {
                if let Ok(id) = channel.get_str("_id") {
                    channel_ids.push(id.into());
                    if channel.get_str("transport").ok() == Some("gateway") && channel.get_str("gateway_channel_id").is_ok() {
                        queue_cleanup(&db, &profile.id, "gateway", id, channel.get_str("user_id").unwrap_or(""), session).await?;
                    }
                }
                for field in ["route_api_key_id", "agent_api_key_id", "pending_route_api_key_id", "pending_agent_api_key_id"] {
                    if let Ok(id) = channel.get_str(field) { key_ids.push(id.into()); }
                }
            }
            let mut routes = db.collection::<Document>(crate::models::channel_conversation::COLLECTION_NAME)
                .find(doc! {"channel_bot_id": &bot_id}).session(&mut *session).await?;
            let mut route_keys = Vec::<String>::new();
            while let Some(route) = routes.next(&mut *session).await.transpose()? {
                if let Ok(key) = route.get_str("agent_api_key_id") { route_keys.push(key.into()); }
            }
            key_ids.extend(route_keys);
            let conversations = db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME);
            let filter = doc! {"$or": [
                {"channel.nyxbot_channel_id": {"$in": &channel_ids}},
                {"reply_channel.nyxbot_channel_id": {"$in": &channel_ids}},
                {"deliver_also.nyxbot_channel_id": {"$in": &channel_ids}}
            ]};
            let mut affected = conversations.find(filter.clone()).session(&mut *session).await?;
            while let Some(row) = affected.next(&mut *session).await.transpose()? {
                if let Ok(key) = row.get_str("credential_api_key_id") { key_ids.push(key.into()); }
                let user = row.get_str("user_id").map_err(|_| AppError::Conflict("Conversation owner unavailable".into()))?;
                let conversation_id = row.get_str("_id").map_err(|_| AppError::Conflict("Conversation identity unavailable".into()))?;
                let cleanup_id = queue_cleanup(&db, &profile.id, "conversation", conversation_id, user, session).await?;
                conversations.update_one(doc! {"_id": conversation_id, "user_id": user}, doc! {"$set": {"platform_cleanup_id": &cleanup_id}}).session(&mut *session).await?;
                if row.get_document("active_turn").is_ok() {
                    conversations.update_one(doc! {"_id": conversation_id}, doc! {"$set": {"active_turn.stop_requested": true}}).session(&mut *session).await?;
                }
            }
            // Existing home context stays private, but loses the contaminated
            // upstream binding and every channel reply/turn capability.
            conversations.update_many(filter, doc! {"$set": {
                "platform_cleanup_pending": true, "channel": bson::Bson::Null,
                "reply_channel": bson::Bson::Null, "deliver_also": [],
                "nyxagent_session_id": bson::Bson::Null, "nyxagent_instruction_binding": bson::Bson::Null,
                "nyxagent_last_response_id": bson::Bson::Null,
                "context_reset_at": bson::DateTime::from_chrono(now), "context_reset_reason": "platform_conversion"
            }}).session(&mut *session).await?;
            super::api_key_mutation_service::update_many(&db, doc! {"_id": {"$in": &key_ids}}, doc! {"$set": {"is_active": false}}, Some(&mut *session)).await?;
            db.collection::<Document>(crate::models::api_key_credential::COLLECTION_NAME)
                .update_many(doc! {"api_key_id": {"$in": &key_ids}, "is_active": true}, doc! {"$set": {"is_active": false, "revoked_at": bson::DateTime::from_chrono(now), "revoked_reason": "parent_revoked"}})
                .session(&mut *session).await?;
            db.collection::<Document>(crate::models::assistant_agent_credential::COLLECTION_NAME)
                .delete_many(doc! {"api_key_id": {"$in": &key_ids}}).session(&mut *session).await?;
            db.collection::<Document>(crate::models::channel_conversation::COLLECTION_NAME)
                .update_many(doc! {"channel_bot_id": &bot_id}, doc! {"$set": {"is_active": false, "default_agent": false, "updated_at": bson::DateTime::from_chrono(now)}})
                .session(&mut *session).await?;
            db.collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
                .update_many(doc! {"channel_bot_id": &bot_id}, doc! {"$set": {"status": "disconnected", "last_error": "platform_conversion", "updated_at": bson::DateTime::from_chrono(now)}, "$unset": {"link_code_hash": "", "link_code_expires_at": "", "agent_key_ciphertext": ""}, "$inc": {"follow_binding_generation": 1_i64}})
                .session(&mut *session).await?;
            Ok(profile)
        }.await;
        transaction_result(result)
    })).await.map_err(map_transaction_error)
}

pub(crate) async fn disable(
    db: &Database,
    id: &str,
    generation: i64,
    delete: bool,
    actor: &str,
) -> AppResult<PlatformBotConfig> {
    let actor = actor.to_owned();
    let db = db.clone();
    let id = id.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            fence_admin(&db, &actor, session).await?;
            let mut set = doc! {"enabled": false, "updated_at": bson::DateTime::now()};
            if delete { set.insert("deleted_at", bson::DateTime::now()); }
            let row = db.collection::<PlatformBotConfig>(COLLECTION_NAME).find_one_and_update(
                doc! {"_id": &id, "generation": generation, "deleted_at": bson::Bson::Null},
                doc! {"$set": set, "$inc": {"generation": 1_i64}}
            ).return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?
                .ok_or_else(|| AppError::Conflict("Platform configuration changed; refresh before deciding".into()))?;
            if delete {
                db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                    .update_one(doc! {"_id": &row.channel_bot_id, "platform_profile_id": &row.id},
                        doc! {"$set": {"is_active": false, "status": "deleted", "updated_at": bson::DateTime::now()}, "$inc": {"route_revision": 1_i64}})
                    .session(&mut *session).await?;
            }
            super::platform_bot_runtime_control_service::emergency_stop_in_session(&db, session, &id, delete).await?;
            Ok(row)
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub(crate) async fn invalidate_manager(db: &Database, manager: &str) -> AppResult<()> {
    db.collection::<Document>(COLLECTION_NAME).update_many(doc! {"manager_bot_id": manager,"native_entry":bson::Bson::Null},
        doc! {"$set": {"enabled": false, "updated_at": bson::DateTime::now()}, "$inc": {"generation": 1_i64}}).await?;
    Ok(())
}

pub(crate) fn require_telegram_manager(row: &PlatformBotConfig) -> AppResult<()> {
    if row.native_entry.is_some()
        || row.identity_version.is_some_and(|version| version != 1)
        || row.execution_profile != PlatformExecutionProfile::PlatformOnboarding
        || !row
            .manager_bot_id
            .parse::<i64>()
            .is_ok_and(|id| id > 0 && id.to_string() == row.manager_bot_id)
    {
        return Err(AppError::Conflict(
            "Telegram manager operation requires a legacy Telegram profile".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn verify_manager(
    service: &super::telegram_new_service::TelegramNewService<'_>,
    id: &str,
    generation: i64,
    actor: &str,
) -> AppResult<PlatformBotConfig> {
    let row = get(service.db, id).await?;
    require_telegram_manager(&row)?;
    if row.deleted_at.is_some() || row.generation != generation {
        return Err(AppError::Conflict("Platform configuration changed".into()));
    }
    let service = service
        .for_manager(
            row.manager_bot_id
                .parse()
                .map_err(|_| AppError::Conflict("Invalid manager identity".into()))?,
        )
        .await?;
    let (manager, _, values) = service.manager().await?;
    if manager.to_string() != row.manager_bot_id
        || values.get("observation_id") != Some(row.observation_id.as_str())
    {
        return Err(AppError::Conflict(
            "Platform manager identity changed".into(),
        ));
    }
    let token = values
        .get(super::channel_adapters::telegram_new::MANAGER_TOKEN)
        .ok_or_else(|| AppError::Conflict("Manager configuration unavailable".into()))?;
    let webhook = service
        .api
        .call(token, "getWebhookInfo", serde_json::json!({}))
        .await?;
    super::telegram_new_admin::validate_manager_webhook(&webhook, &service.manager_callback())?;
    let current = get(service.db, id).await?;
    require_telegram_manager(&current)?;
    if current.generation != generation || current.deleted_at.is_some() {
        return Err(AppError::Conflict(
            "Platform configuration changed during verification".into(),
        ));
    }
    let db = service.db.clone();
    let actor = actor.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            fence_admin(&db, &actor, session).await?;
            if db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id": &row.id, "generation": generation, "deleted_at": bson::Bson::Null},
                doc! {"$inc": {"verification_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 { return Err(AppError::Conflict("Platform configuration changed".into())); }
            db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                .update_one(doc! {"_id": &row.channel_bot_id, "platform_profile_id": &row.id, "is_active": true},
                    doc! {"$set": {"status": "active", "webhook_registered": true, "updated_at": bson::DateTime::now()}}).session(&mut *session).await?;
            Ok(())
        }.await; transaction_result(result)
    }).await.map_err(map_transaction_error)?;
    Ok(current)
}

async fn queue_cleanup(
    db: &Database,
    profile: &str,
    kind: &str,
    resource: &str,
    owner: &str,
    session: &mut mongodb::ClientSession,
) -> AppResult<String> {
    let mut turn_id = None;
    let mut machine_node_ids = Vec::new();
    let mut async_operation_ids = Vec::new();
    let mut machine_jobs = Vec::new();
    let mut gateway_channel_id = None;
    let mut gateway_version = None;
    if kind == "conversation" {
        if let Some(conversation) = db
            .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .find_one(doc! {"_id": resource, "user_id": owner})
            .session(&mut *session)
            .await?
        {
            if let Ok(turn) = conversation.get_document("active_turn") {
                turn_id = turn.get_str("turn_id").ok().map(str::to_owned);
                if let Ok(nodes) = turn.get_array("machine_node_ids") {
                    machine_node_ids = nodes
                        .iter()
                        .filter_map(|node| node.as_str().map(str::to_owned))
                        .collect();
                }
            }
        }
        let mut operations = db.collection::<Document>(crate::models::async_service_operation::COLLECTION_NAME)
            .find(doc! {"conversation_id": resource, "user_id": owner, "state": {"$nin": ["delivered", "cancelled"]}}).limit(1025).session(&mut *session).await?;
        while let Some(operation) = operations.next(&mut *session).await.transpose()? {
            async_operation_ids.push(
                operation
                    .get_str("_id")
                    .map_err(|_| AppError::Conflict("Operation identity unavailable".into()))?
                    .to_owned(),
            );
        }
        let mut jobs = db
            .collection::<Document>(crate::models::machine_job::COLLECTION_NAME)
            .find(doc! {"conversation_id": resource, "user_id": owner, "state": "running"})
            .limit(1025)
            .session(&mut *session)
            .await?;
        while let Some(job) = jobs.next(&mut *session).await.transpose()? {
            machine_jobs.push(crate::models::platform_bot_config::CleanupMachineJob {
                id: job
                    .get_str("_id")
                    .map_err(|_| AppError::Conflict("Machine job identity unavailable".into()))?
                    .into(),
                node_id: job
                    .get_str("node_id")
                    .map_err(|_| AppError::Conflict("Machine node identity unavailable".into()))?
                    .into(),
            });
        }
        let mut nodes: std::collections::BTreeSet<_> = machine_node_ids.iter().cloned().collect();
        nodes.extend(machine_jobs.iter().map(|job| job.node_id.clone()));
        for node in nodes {
            if let Some(row) = db
                .collection::<Document>(crate::models::node::COLLECTION_NAME)
                .find_one(doc! {"_id": node})
                .session(&mut *session)
                .await?
            {
                if row.get_str("status").ok() != Some("online")
                    || row
                        .get_document("machine")
                        .ok()
                        .and_then(|profile| profile.get_bool("exact_cancel").ok())
                        != Some(true)
                {
                    return Err(AppError::Conflict("Stop outstanding machine work or upgrade and reconnect its node before conversion".into()));
                }
            }
        }
        if async_operation_ids.len() > 1024
            || machine_jobs.len() > 1024
            || machine_node_ids.len() > 1024
        {
            return Err(AppError::Conflict(
                "Stop outstanding work before converting the manager".into(),
            ));
        }
    } else if kind == "gateway" {
        let row = db
            .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
            .find_one(doc! {"_id": resource, "user_id": owner})
            .session(&mut *session)
            .await?
            .ok_or_else(|| AppError::Conflict("Gateway identity unavailable".into()))?;
        gateway_channel_id = row.get_str("gateway_channel_id").ok().map(str::to_owned);
        gateway_version = row
            .get_i64("gateway_version")
            .ok()
            .or_else(|| row.get_i32("gateway_version").ok().map(i64::from));
        if gateway_channel_id.is_none() || gateway_version.is_none_or(|version| version <= 0) {
            return Err(AppError::Conflict(
                "Verify the gateway binding before conversion".into(),
            ));
        }
    }
    let now = Utc::now();
    let row = crate::models::platform_bot_config::PlatformBotCleanup {
        id: uuid::Uuid::new_v4().to_string(),
        platform_config_id: profile.into(),
        kind: kind.into(),
        resource_id: resource.into(),
        owner_user_id: owner.into(),
        capture_version: 1,
        turn_id,
        async_operation_ids,
        machine_jobs,
        machine_node_ids,
        gateway_channel_id,
        gateway_version,
        completed: false,
        claim_id: None,
        retry_at: now,
        created_at: now,
    };
    db.collection::<crate::models::platform_bot_config::PlatformBotCleanup>(
        crate::models::platform_bot_config::CLEANUP_COLLECTION,
    )
    .insert_one(&row)
    .session(&mut *session)
    .await?;
    if kind == "conversation" && (!row.machine_jobs.is_empty() || !row.machine_node_ids.is_empty())
    {
        // Remote cancellation has its own exact-resource outbox. A disconnected
        // node must not keep the old home thread locked after its turn settles.
        let mut machine = row.clone();
        machine.id = uuid::Uuid::new_v4().to_string();
        machine.kind = "machine".into();
        machine.async_operation_ids.clear();
        machine.retry_at += chrono::Duration::seconds(1);
        db.collection::<crate::models::platform_bot_config::PlatformBotCleanup>(
            crate::models::platform_bot_config::CLEANUP_COLLECTION,
        )
        .insert_one(machine)
        .session(&mut *session)
        .await?;
    }
    Ok(row.id)
}

pub(crate) async fn claim_cleanup(
    db: &Database,
) -> AppResult<Option<crate::models::platform_bot_config::PlatformBotCleanup>> {
    Ok(db.collection::<crate::models::platform_bot_config::PlatformBotCleanup>(crate::models::platform_bot_config::CLEANUP_COLLECTION)
        .find_one_and_update(doc! {"completed": false, "$expr": {"$lte": ["$retry_at", "$$NOW"]}},
            vec![doc! {"$set": {"claim_id": uuid::Uuid::new_v4().to_string(), "retry_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": 120_i64}}}}])
        .sort(doc! {"retry_at": 1}).return_document(mongodb::options::ReturnDocument::After).await?)
}

pub(crate) async fn admit_cleanup_effect(
    db: &Database,
    task: &crate::models::platform_bot_config::PlatformBotCleanup,
) -> AppResult<()> {
    let db = db.clone();
    let task = task.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            if task.capture_version != 1 { return Err(AppError::Conflict("Cleanup requires captured authority".into())); }
            if db.collection::<Document>(crate::models::platform_bot_config::CLEANUP_COLLECTION).update_one(
                doc! {"_id": &task.id, "claim_id": &task.claim_id, "completed": false, "$expr": {"$gt": ["$retry_at", "$$NOW"]}},
                vec![doc! {"$set": {"effect_revision": {"$add": [{"$ifNull": ["$effect_revision", 0_i64]}, 1_i64]}, "retry_at": {"$dateAdd": {"startDate": "$$NOW", "unit": "second", "amount": 120_i64}}}}]).session(&mut *session).await?.matched_count != 1 {
                return Err(AppError::Conflict("Cleanup claim changed or expired".into()));
            }
            if task.kind == "conversation" {
                let conversations = db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME);
                if let Some(row) = conversations.find_one(doc! {"_id": &task.resource_id, "user_id": &task.owner_user_id}).session(&mut *session).await? {
                    if row.get_str("platform_cleanup_id").ok() != Some(task.id.as_str()) || row.get_bool("platform_cleanup_pending").ok() != Some(true) {
                        return Err(AppError::Conflict("Conversation cleanup authority changed".into()));
                    }
                    if let Ok(turn) = row.get_document("active_turn") {
                        if turn.get_str("turn_id").ok() != task.turn_id.as_deref() { return Err(AppError::Conflict("Conversation turn changed".into())); }
                    }
                    if let Some(turn) = &task.turn_id { conversations.update_one(doc! {"_id": &task.resource_id, "user_id": &task.owner_user_id, "active_turn.turn_id": turn},
                        doc! {"$set": {"active_turn.stop_requested": true}}).session(&mut *session).await?; }
                    conversations.update_one(doc! {"_id": &task.resource_id, "user_id": &task.owner_user_id},
                        doc! {"$pull": {"pending_events": {"id": {"$in": &task.async_operation_ids}}}}).session(&mut *session).await?;
                }
                db.collection::<Document>(crate::models::async_service_operation::COLLECTION_NAME).update_many(
                    doc! {"_id": {"$in": &task.async_operation_ids}, "user_id": &task.owner_user_id, "conversation_id": &task.resource_id, "state": {"$in": ["submitting", "waiting", "ready", "queued"]}},
                    doc! {"$unset":{"secret_context_versions.result_encrypted":""},"$set": {"state": "cancelling", "result_encrypted": bson::Bson::Null, "lease_id": bson::Bson::Null, "lease_until": bson::Bson::Null, "poll_at": bson::DateTime::now()}}).session(&mut *session).await?;
                db.collection::<Document>(crate::models::machine_job::COLLECTION_NAME).update_many(
                    doc! {"_id": {"$in": task.machine_jobs.iter().map(|job| job.id.clone()).collect::<Vec<_>>()}, "user_id": &task.owner_user_id, "conversation_id": &task.resource_id, "state": "running"},
                    doc! {"$set": {"state": "finished", "finished_at": bson::DateTime::now()}}).session(&mut *session).await?;
            }
            Ok(())
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

pub(crate) async fn complete_cleanup(
    db: &Database,
    row: &crate::models::platform_bot_config::PlatformBotCleanup,
) -> AppResult<()> {
    let db = db.clone();
    let row = row.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<()> = async {
            let completed = db.collection::<Document>(crate::models::platform_bot_config::CLEANUP_COLLECTION)
                .update_one(doc! {"_id": &row.id, "claim_id": &row.claim_id, "completed": false, "$expr": {"$gt": ["$retry_at", "$$NOW"]}}, doc! {"$set": {"completed": true}})
                .session(&mut *session).await?;
            if completed.matched_count != 1 { return Err(AppError::Conflict("Cleanup claim changed".into())); }
            if row.kind == "conversation" {
                let conversations = db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME);
                if conversations.find_one(doc! {"_id": &row.resource_id, "user_id": &row.owner_user_id}).session(&mut *session).await?.is_some()
                    && conversations.update_one(doc! {"_id": &row.resource_id, "user_id": &row.owner_user_id, "active_turn": bson::Bson::Null, "platform_cleanup_id": &row.id},
                        doc! {"$unset": {"platform_cleanup_pending": "", "platform_cleanup_id": ""}}).session(&mut *session).await?.matched_count != 1 {
                    return Err(AppError::Conflict("Cleanup turn has not settled".into()));
                }
            }
            Ok(())
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
}

#[cfg(test)]
#[allow(clippy::items_after_test_module)]
mod tests {
    use super::*;
    use crate::test_utils::{connect_transaction_test_database, test_app_state, test_user};

    async fn fixture(name: &str) -> (crate::AppState, String, String, String) {
        let db = connect_transaction_test_database(name).await;
        ensure_indexes(&db).await.unwrap();
        let actor = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&actor, crate::models::user::UserType::Person))
            .await
            .unwrap();
        super::super::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        let admin_id = super::super::role_service::get_platform_role_ids(&db)
            .await
            .unwrap()
            .admin;
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &actor},
                doc! {"$set": {"role_ids": [&admin_id]}},
            )
            .await
            .unwrap();
        let funding =
            super::super::org_service::create_org_user(&db, "Platform funding", None, None)
                .await
                .unwrap()
                .id;
        super::super::org_service::create_membership(
            &db,
            &funding,
            &actor,
            crate::models::org_membership::OrgRole::Admin,
            crate::models::org_membership::MemberScopeSource::Inherit,
            None,
        )
        .await
        .unwrap();
        let bot = uuid::Uuid::new_v4().to_string();
        db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).insert_one(doc! {
            "_id": &bot, "user_id": &actor, "platform": "telegram", "credential_source": "telegram_manager", "platform_bot_id": "123", "is_active": true,
        }).await.unwrap();
        db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME).insert_one(doc! {
            "_id": uuid::Uuid::new_v4().to_string(), "provider": "telegram-new",
            "fields": {"manager_bot_id": "123", "webhook_ready": "true", "observation_id": "observation"}
        }).await.unwrap();
        (test_app_state(db), actor, funding, bot)
    }

    #[tokio::test]
    async fn telegram_manager_admin_conversion_and_rotation_do_not_select_same_id_native_entry() {
        let (state, actor, funding, bot) = fixture("native_manager_remote_namespace").await;
        let native_id = uuid::Uuid::new_v4().to_string();
        let native_channel = uuid::Uuid::new_v4().to_string();
        let rows = state.db.collection::<Document>(COLLECTION_NAME);
        rows.insert_one(
            doc! {"_id":&native_id,"channel_bot_id":&native_channel,"manager_bot_id":"123",
            "identity_version":2,"enabled":true,"generation":5_i64,
            "native_entry":{"identity":{"version":2,"platform":"x","channel_bot_id":&native_channel,
                "platform_bot_id":"123"}}},
        )
        .await
        .unwrap();
        assert!(!any_profile(&state.db).await.unwrap());
        admit_manager_admin(&state.db, &actor, false, "123")
            .await
            .unwrap();
        finalize_manager_delivery(&state.db, &actor, false, "123", "fresh-observation".into())
            .await
            .unwrap();
        let legacy = convert(&state.db, &actor, &bot, &funding).await.unwrap();
        assert_ne!(legacy.id, native_id);
        assert!(any_profile(&state.db).await.unwrap());
        assert!(
            admit_manager_admin(&state.db, &actor, false, "123")
                .await
                .is_err()
        );
        rows.update_one(doc! {"_id":&legacy.id}, doc! {"$set":{"enabled":true}})
            .await
            .unwrap();
        invalidate_manager(&state.db, "123").await.unwrap();
        let old = rows
            .find_one(doc! {"_id":&legacy.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!old.get_bool("enabled").unwrap());
        assert_eq!(old.get_i64("generation").unwrap(), legacy.generation + 1);
        let native = rows
            .find_one(doc! {"_id":&native_id})
            .await
            .unwrap()
            .unwrap();
        assert!(native.get_bool("enabled").unwrap());
        assert_eq!(native.get_i64("generation").unwrap(), 5);
        assert_eq!(
            native
                .get_document("native_entry")
                .unwrap()
                .get_document("identity")
                .unwrap()
                .get_str("platform")
                .unwrap(),
            "x"
        );
    }

    #[tokio::test]
    async fn platform_onboarding_final_admin_fence_rejects_demotion_and_disabled_actor() {
        let (state, actor, _, _) = fixture("platform_admin_final_fence").await;
        let user = state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id": &actor})
            .await
            .unwrap()
            .unwrap();
        assert!(
            super::super::role_service::resolve_platform_role(&state.db, &user)
                .await
                .unwrap()
                .is_admin()
        );
        // A readiness snapshot cannot confer mutation authority after a live demotion.
        state
            .db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &actor},
                doc! {"$set": {"role_ids": [], "is_admin": true}},
            )
            .await
            .unwrap();
        let mut session = state.db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(matches!(
            fence_admin(&state.db, &actor, &mut session).await,
            Err(AppError::Forbidden(_))
        ));
        session.abort_transaction().await.unwrap();
        let admin = super::super::role_service::get_platform_role_ids(&state.db)
            .await
            .unwrap()
            .admin;
        state
            .db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &actor},
                doc! {"$set": {"role_ids": [admin], "is_active": false}},
            )
            .await
            .unwrap();
        session.start_transaction().await.unwrap();
        assert!(matches!(
            fence_admin(&state.db, &actor, &mut session).await,
            Err(AppError::Forbidden(_))
        ));
        session.abort_transaction().await.unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_conversion_rejects_foreign_funding_and_pending_transport() {
        let (basic, admin, _, basic_bot) = fixture("platform_convert_basic_admin").await;
        basic
            .db
            .collection::<Document>(crate::models::org_membership::COLLECTION_NAME)
            .delete_many(doc! {"member_user_id":&admin})
            .await
            .unwrap();
        let basic_profile = convert(&basic.db, &admin, &basic_bot, &admin)
            .await
            .unwrap();
        assert!(!basic_profile.enabled);
        assert_eq!(basic_profile.funding_owner_user_id, admin);
        basic.db.drop().await.unwrap();

        let (state, actor, funding, bot) = fixture("platform_convert_authority").await;
        let foreign =
            super::super::org_service::create_org_user(&state.db, "Foreign funding", None, None)
                .await
                .unwrap()
                .id;
        assert!(matches!(
            convert(&state.db, &actor, &bot, &foreign).await,
            Err(AppError::Forbidden(_))
        ));
        let foreign_person = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &foreign_person,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        assert!(matches!(
            convert(&state.db, &actor, &bot, &foreign_person).await,
            Err(AppError::Forbidden(_))
        ));
        let channels = state
            .db
            .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME);
        let link = uuid::Uuid::new_v4().to_string();
        channels.insert_one(doc! {"_id": &link, "channel_bot_id": &bot, "status": "pending", "user_id": &actor, "transport": "gateway"}).await.unwrap();
        assert!(matches!(
            convert(&state.db, &actor, &bot, &funding).await,
            Err(AppError::Conflict(_))
        ));
        channels
            .update_one(doc! {"_id": &link}, doc! {"$set": {"status": "active"}})
            .await
            .unwrap();
        assert!(matches!(
            convert(&state.db, &actor, &bot, &funding).await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            state
                .db
                .collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        channels.delete_one(doc! {"_id": &link}).await.unwrap();
        let profile = convert(&state.db, &actor, &bot, &funding).await.unwrap();
        assert!(!profile.enabled);
        assert_eq!(profile.funding_owner_user_id, funding);
        assert!(matches!(
            convert(&state.db, &actor, &bot, &funding).await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            state
                .db
                .collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        let deleted = disable(&state.db, &profile.id, profile.generation, true, &actor)
            .await
            .unwrap();
        assert!(deleted.deleted_at.is_some());
        let stored = state
            .db
            .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
            .find_one(doc! {"_id": &bot})
            .await
            .unwrap()
            .unwrap();
        assert!(!stored.get_bool("is_active").unwrap());
        assert_eq!(stored.get_str("platform_profile_id").unwrap(), profile.id);
        assert!(
            !super::super::platform_bot_authority::permits_personal_routes(&state.db, &bot)
                .await
                .unwrap()
        );
        assert!(matches!(
            disable(&state.db, &profile.id, profile.generation, false, &actor).await,
            Err(AppError::Conflict(_))
        ));
    }

    #[tokio::test]
    async fn platform_onboarding_conversion_preserves_stopping_turn_and_settles_cleanup() {
        use super::super::assistant_nyxagent as engine;
        let (state, actor, funding, bot) = fixture("platform_convert_stopping").await;
        engine::ensure_indexes(&state.db).await.unwrap();
        let request: engine::TurnRequest =
            serde_json::from_value(serde_json::json!({"text": "Owner test"})).unwrap();
        let row = engine::begin_turn(&state.db, &actor, &request, &state.encryption_keys)
            .await
            .unwrap();
        let key = row.credential_api_key_id.clone();
        let channel = uuid::Uuid::new_v4().to_string();
        state.db.collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME).insert_one(doc! {
            "_id": &channel, "channel_bot_id": &bot, "user_id": &actor, "status": "active", "transport": "direct",
        }).await.unwrap();
        state.db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .update_one(doc! {"_id": &row.id}, doc! {"$set": {"reply_channel": {"nyxbot_channel_id": &channel, "partition": "p", "platform": "telegram"}, "nyxagent_session_id": "private-session"}}).await.unwrap();
        let standalone_key = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<Document>(crate::models::api_key::COLLECTION_NAME)
            .insert_one(doc! {"_id": &standalone_key, "user_id": &actor, "is_active": true})
            .await
            .unwrap();
        state.db.collection::<Document>(crate::models::channel_conversation::COLLECTION_NAME).insert_one(doc! {"_id": uuid::Uuid::new_v4().to_string(), "channel_bot_id": &bot, "agent_api_key_id": &standalone_key, "is_active": true}).await.unwrap();
        let shared_route = uuid::Uuid::new_v4().to_string();
        state.db.collection::<Document>(crate::models::channel_conversation::COLLECTION_NAME).insert_one(doc! {"_id": &shared_route, "channel_bot_id": "unrelated-bot", "agent_api_key_id": &standalone_key, "is_active": true}).await.unwrap();
        assert!(matches!(
            convert(&state.db, &actor, &bot, &funding).await,
            Err(AppError::Conflict(_))
        ));
        state
            .db
            .collection::<Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &shared_route},
                doc! {"$set": {"is_active": false}},
            )
            .await
            .unwrap();
        let profile = convert(&state.db, &actor, &bot, &funding).await.unwrap();
        let stopped = engine::get(&state.db, &actor, &row.id).await.unwrap();
        assert_eq!(
            stopped.active_turn.as_ref().unwrap().turn_id,
            row.active_turn.as_ref().unwrap().turn_id
        );
        assert!(stopped.active_turn.as_ref().unwrap().stop_requested);
        assert!(stopped.reply_channel.is_none());
        assert!(stopped.nyxagent_session_id.is_none());
        for id in [&key, &standalone_key] {
            assert!(
                !state
                    .db
                    .collection::<Document>(crate::models::api_key::COLLECTION_NAME)
                    .find_one(doc! {"_id": id})
                    .await
                    .unwrap()
                    .unwrap()
                    .get_bool("is_active")
                    .unwrap()
            );
        }
        let next: engine::TurnRequest = serde_json::from_value(
            serde_json::json!({"text": "New owner turn", "conversation_id": row.id}),
        )
        .unwrap();
        assert!(matches!(
            engine::begin_turn(&state.db, &actor, &next, &state.encryption_keys).await,
            Err(AppError::Conflict(_))
        ));
        let mut state = state;
        state.config.connect_link_expiry_sweep_interval_secs = 0;
        crate::handlers::platform_bots::sweep_cleanup(&state)
            .await
            .unwrap();
        let settled = engine::get(&state.db, &actor, &row.id).await.unwrap();
        assert!(settled.active_turn.is_none());
        assert_eq!(
            state
                .db
                .collection::<Document>(crate::models::platform_bot_config::CLEANUP_COLLECTION)
                .count_documents(doc! {"platform_config_id": &profile.id, "completed": false})
                .await
                .unwrap(),
            0
        );
        assert!(
            engine::begin_turn(&state.db, &actor, &next, &state.encryption_keys)
                .await
                .is_ok()
        );
    }
    #[tokio::test]
    async fn platform_onboarding_stale_cleanup_cannot_cancel_a_new_owner_turn_or_operations() {
        use super::super::assistant_nyxagent as engine;
        let (state, actor, funding, bot) = fixture("platform_cleanup_stale").await;
        engine::ensure_indexes(&state.db).await.unwrap();
        let input: engine::TurnRequest =
            serde_json::from_value(serde_json::json!({"text": "Old turn"})).unwrap();
        let row = engine::begin_turn(&state.db, &actor, &input, &state.encryption_keys)
            .await
            .unwrap();
        let old_turn = row.active_turn.as_ref().unwrap().turn_id.clone();
        let link = uuid::Uuid::new_v4().to_string();
        state.db.collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME).insert_one(doc! {"_id": &link, "channel_bot_id": &bot, "user_id": &actor, "status": "active", "transport": "direct"}).await.unwrap();
        state.db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME).update_one(doc! {"_id": &row.id}, doc! {"$set": {"reply_channel": {"nyxbot_channel_id": &link, "partition": "p", "platform": "telegram"}}}).await.unwrap();
        let operations = state
            .db
            .collection::<Document>(crate::models::async_service_operation::COLLECTION_NAME);
        let jobs = state
            .db
            .collection::<Document>(crate::models::machine_job::COLLECTION_NAME);
        let old_op = uuid::Uuid::new_v4().to_string();
        let old_job = uuid::Uuid::new_v4().to_string();
        operations.insert_one(doc! {"_id": &old_op, "user_id": &actor, "conversation_id": &row.id, "state": "waiting"}).await.unwrap();
        jobs.insert_one(doc! {"_id": &old_job, "user_id": &actor, "conversation_id": &row.id, "node_id": "removed-node", "state": "running"}).await.unwrap();
        convert(&state.db, &actor, &bot, &funding).await.unwrap();
        let a = claim_cleanup(&state.db).await.unwrap().unwrap();
        state
            .db
            .collection::<Document>(crate::models::platform_bot_config::CLEANUP_COLLECTION)
            .update_one(
                doc! {"_id": &a.id},
                doc! {"$set": {"retry_at": bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        let b = claim_cleanup(&state.db).await.unwrap().unwrap();
        assert_ne!(a.claim_id, b.claim_id);
        assert!(admit_cleanup_effect(&state.db, &a).await.is_err());
        admit_cleanup_effect(&state.db, &b).await.unwrap();
        assert_eq!(
            operations
                .find_one(doc! {"_id": &old_op})
                .await
                .unwrap()
                .unwrap()
                .get_str("state")
                .unwrap(),
            "cancelling"
        );
        assert_eq!(
            jobs.find_one(doc! {"_id": &old_job})
                .await
                .unwrap()
                .unwrap()
                .get_str("state")
                .unwrap(),
            "finished"
        );
        engine::force_stop(&state.db, &actor, &row.id, &old_turn)
            .await
            .unwrap();
        let settled = engine::get(&state.db, &actor, &row.id).await.unwrap();
        assert!(settled.active_turn.is_none());
        assert!(settled.platform_cleanup_pending);
        assert_eq!(settled.platform_cleanup_id.as_deref(), Some(b.id.as_str()));
        let blocked: engine::TurnRequest = serde_json::from_value(
            serde_json::json!({"text": "Must remain blocked", "conversation_id": row.id}),
        )
        .unwrap();
        assert!(matches!(
            engine::begin_turn(&state.db, &actor, &blocked, &state.encryption_keys).await,
            Err(AppError::Conflict(_))
        ));
        complete_cleanup(&state.db, &b).await.unwrap();

        let next: engine::TurnRequest = serde_json::from_value(
            serde_json::json!({"text": "New turn", "conversation_id": row.id}),
        )
        .unwrap();
        let next = engine::begin_turn(&state.db, &actor, &next, &state.encryption_keys)
            .await
            .unwrap();
        let new_op = uuid::Uuid::new_v4().to_string();
        let new_job = uuid::Uuid::new_v4().to_string();
        operations.insert_one(doc! {"_id": &new_op, "user_id": &actor, "conversation_id": &row.id, "state": "waiting"}).await.unwrap();
        jobs.insert_one(doc! {"_id": &new_job, "user_id": &actor, "conversation_id": &row.id, "node_id": "removed-node", "state": "running"}).await.unwrap();
        assert!(admit_cleanup_effect(&state.db, &a).await.is_err());
        assert!(complete_cleanup(&state.db, &a).await.is_err());
        assert!(
            !engine::force_stop(&state.db, &actor, &row.id, &old_turn)
                .await
                .unwrap()
        );
        let live = engine::get(&state.db, &actor, &row.id).await.unwrap();
        assert_eq!(
            live.active_turn.as_ref().unwrap().turn_id,
            next.active_turn.as_ref().unwrap().turn_id
        );
        assert!(!live.active_turn.as_ref().unwrap().stop_requested);
        assert_eq!(
            operations
                .find_one(doc! {"_id": &new_op})
                .await
                .unwrap()
                .unwrap()
                .get_str("state")
                .unwrap(),
            "waiting"
        );
        assert_eq!(
            jobs.find_one(doc! {"_id": &new_job})
                .await
                .unwrap()
                .unwrap()
                .get_str("state")
                .unwrap(),
            "running"
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_gateway_cleanup_survives_deleted_local_row_and_uses_captured_version()
     {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path, query_param},
        };
        let (state, actor, funding, bot) = fixture("platform_cleanup_gateway").await;
        let server = MockServer::start().await;
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.slug = crate::handlers::nyxbot::GATEWAY_SLUG.into();
        service.base_url = server.uri();
        service.requires_user_credential = false;
        state
            .db
            .collection::<crate::models::downstream_service::DownstreamService>(
                crate::models::downstream_service::COLLECTION_NAME,
            )
            .insert_one(&service)
            .await
            .unwrap();
        let link = uuid::Uuid::new_v4().to_string();
        let channels = state
            .db
            .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME);
        channels.insert_one(doc! {"_id": &link, "channel_bot_id": &bot, "user_id": &actor, "status": "active", "transport": "gateway", "gateway_channel_id": "old-remote", "gateway_version": 7_i64}).await.unwrap();
        convert(&state.db, &actor, &bot, &funding).await.unwrap();
        let task = claim_cleanup(&state.db).await.unwrap().unwrap();
        assert_eq!(task.gateway_channel_id.as_deref(), Some("old-remote"));
        assert_eq!(task.gateway_version, Some(7));
        channels.delete_one(doc! {"_id": &link}).await.unwrap();
        Mock::given(method("GET"))
            .and(path("/channels/old-remote"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"version": 7})),
            )
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/channels/old-remote"))
            .and(query_param("expected_version", "7"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;
        crate::handlers::nyxbot::cleanup_platform_gateway(&state, &task)
            .await
            .unwrap();
        complete_cleanup(&state.db, &task).await.unwrap();
        state.db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn platform_onboarding_conversion_rejects_old_machine_nodes_without_mutation() {
        let (state, actor, funding, bot) = fixture("platform_cleanup_old_node").await;
        let link = uuid::Uuid::new_v4().to_string();
        let conversation = uuid::Uuid::new_v4().to_string();
        state.db.collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME).insert_one(doc! {"_id": &link, "channel_bot_id": &bot, "user_id": &actor, "status": "active", "transport": "direct"}).await.unwrap();
        state.db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME).insert_one(doc! {"_id": &conversation, "user_id": &actor, "reply_channel": {"nyxbot_channel_id": &link}, "active_turn": {"turn_id": "old-turn", "stop_requested": false, "machine_node_ids": ["old-node"]}}).await.unwrap();
        state.db.collection::<Document>(crate::models::node::COLLECTION_NAME).insert_one(doc! {"_id": "old-node", "status": "online", "machine": {"version": 1, "shell": true}}).await.unwrap();
        assert!(matches!(
            convert(&state.db, &actor, &bot, &funding).await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            state
                .db
                .collection::<Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let row = state
            .db
            .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
            .find_one(doc! {"_id": &conversation})
            .await
            .unwrap()
            .unwrap();
        assert!(
            !row.get_document("active_turn")
                .unwrap()
                .get_bool("stop_requested")
                .unwrap()
        );
        assert!(!row.contains_key("platform_cleanup_pending"));
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_missing_conversation_cleanup_terminates() {
        let (state, actor, funding, bot) = fixture("platform_cleanup_missing_conversation").await;
        let link = uuid::Uuid::new_v4().to_string();
        let conversation = uuid::Uuid::new_v4().to_string();
        state.db.collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME).insert_one(doc! {"_id": &link, "channel_bot_id": &bot, "user_id": &actor, "status": "active", "transport": "direct"}).await.unwrap();
        let conversations = state
            .db
            .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME);
        conversations.insert_one(doc! {"_id": &conversation, "user_id": &actor, "reply_channel": {"nyxbot_channel_id": &link}, "active_turn": bson::Bson::Null}).await.unwrap();
        convert(&state.db, &actor, &bot, &funding).await.unwrap();
        conversations
            .delete_one(doc! {"_id": &conversation})
            .await
            .unwrap();
        crate::handlers::platform_bots::sweep_cleanup(&state)
            .await
            .unwrap();
        assert_eq!(
            state
                .db
                .collection::<Document>(crate::models::platform_bot_config::CLEANUP_COLLECTION)
                .count_documents(doc! {"completed": false})
                .await
                .unwrap(),
            0
        );
        state.db.drop().await.unwrap();
    }
}

pub(crate) async fn configure_guide(
    state: &crate::AppState,
    actor: &str,
    id: &str,
    generation: i64,
    guide: crate::models::platform_bot_config::GuideConfiguration,
    enabled: bool,
) -> AppResult<PlatformBotConfig> {
    if super::platform_bot_runtime_control_service::get(&state.db, id)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Use manager runtime controls for this profile".into(),
        ));
    }
    super::platform_setup_service::validate_limits(&guide.limits)?;
    if guide.anonymous_daily_credits <= crate::models::credits::Credits::ZERO
        || guide.anonymous_daily_credits > guide.limits.daily_credits
        || !(1..=1000).contains(&guide.platform_turns_per_hour)
        || !(1..=32).contains(&guide.platform_concurrent_turns)
    {
        return Err(AppError::ValidationError(
            "Guide quotas are outside supported bounds".into(),
        ));
    }
    let mut profile = get(&state.db, id).await?;
    if profile.generation != generation || profile.deleted_at.is_some() {
        return Err(AppError::Conflict("Platform configuration changed".into()));
    }
    profile.configured_by = actor.to_owned();
    profile.guide = Some(guide.clone());
    super::platform_inference_service::sponsor_owner(state, &profile).await?;
    let reviewed_service =
        super::platform_inference_service::selected_service(state, &profile).await?;
    if enabled {
        super::platform_inference_service::validate_model_contract(state, &profile).await?;
    }
    if enabled && !state.billing.billing_enabled() {
        return Err(AppError::Conflict(
            "Enable funded billing before enabling the public guide".into(),
        ));
    }
    if enabled {
        super::platform_inference_service::probe_funding(
            state,
            &profile,
            &profile.funding_owner_user_id,
            &guide.limits,
        )
        .await?;
    }
    let db = state.db.clone();
    let profile = profile.clone();
    let actor = actor.to_owned();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            fence_admin(&db, &actor, session).await?;
            if db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME)
                .find_one(doc! {"platform_config_id": &profile.id}).session(&mut *session).await?.is_some() {
                return Err(AppError::Conflict("Use manager runtime controls for this profile".into()));
            }
            let live_service = db.collection::<crate::models::downstream_service::DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
                .find_one(doc! {"_id": &reviewed_service.id}).session(&mut *session).await?
                .ok_or_else(|| AppError::Conflict("Inference service is unavailable".into()))?;
            if super::platform_inference_service::readiness_projection(&live_service) != super::platform_inference_service::readiness_projection(&reviewed_service)
                || db.collection::<Document>(crate::models::downstream_service::COLLECTION_NAME).update_one(doc! {"_id": &live_service.id},
                    doc! {"$inc": {"platform_readiness_revision": 1_i64}}).session(&mut *session).await?.matched_count != 1 {
                return Err(AppError::Conflict("Inference service changed during readiness; validate it again".into()));
            }
            if db.collection::<Document>(crate::models::user::COLLECTION_NAME).find_one_and_update(
                doc! {"_id": &profile.funding_owner_user_id, "user_type": "org", "is_active": true}, doc! {"$inc": {"platform_funding_revision": 1_i64}}).session(&mut *session).await?.is_none()
                || db.collection::<Document>(crate::models::org_membership::COLLECTION_NAME).find_one_and_update(
                    doc! {"org_user_id": &profile.funding_owner_user_id, "member_user_id": &actor, "role": "admin", "revoked_at": bson::Bson::Null}, doc! {"$inc": {"platform_funding_revision": 1_i64}}).session(&mut *session).await?.is_none() {
                return Err(AppError::Forbidden("Funding consent changed".into()));
            }
            if db.collection::<Document>(crate::models::platform_credential::COLLECTION_NAME).find_one(
                doc! {"provider": {"$in":["telegram-new".to_owned(), format!("telegram-new/{}", &profile.manager_bot_id)]}, "fields.manager_bot_id": &profile.manager_bot_id, "fields.observation_id": &profile.observation_id, "fields.webhook_ready": "true"})
                .session(&mut *session).await?.is_none() { return Err(AppError::Conflict("Manager delivery is unavailable".into())); }
            if db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(
                doc! {"_id": &profile.channel_bot_id, "platform_profile_id": &profile.id, "is_active": true}, doc! {"$inc": {"route_revision": 1_i64}})
                .session(&mut *session).await?.matched_count != 1 { return Err(AppError::Conflict("Manager connection is unavailable".into())); }
            db.collection::<PlatformBotConfig>(COLLECTION_NAME).find_one_and_update(
                doc! {"_id": &profile.id, "generation": generation, "deleted_at": bson::Bson::Null},
                doc! {"$set": {"guide": bson::to_bson(&guide).map_err(|_| AppError::ValidationError("Invalid guide configuration".into()))?, "configured_by": &actor, "enabled": enabled, "updated_at": bson::DateTime::now()}, "$inc": {"generation": 1_i64}})
                .return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(|| AppError::Conflict("Platform configuration changed".into()))
        }.await; super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::api_key_mutation_service::map_transaction_error)
}

/// Admin setup sees eligible unconverted transports, independently of the
/// ordinary owner's list. Persisted reservations (including tombstones) win.
pub(crate) async fn conversion_targets(
    db: &Database,
    before: Option<&str>,
) -> AppResult<(Vec<crate::models::channel_bot::ChannelBot>, Option<String>)> {
    use futures::TryStreamExt;
    let mut filter = doc! {"is_active":true,"platform_profile_id":{"$exists":false},"$or":[
        {"platform":"telegram","credential_source":"telegram_manager"},
        {"platform":{"$in":["slack","discord","whatsapp","lark","feishu"]},"status":"active"},
        {"platform":"x","status":"active","$or":[{"webhook_registered":true},{"platform_entry_pending":{"$exists":true}}]}
    ]};
    if let Some(id) = before {
        filter.insert("_id", doc! {"$lt":id});
    }
    let mut rows = db
        .collection::<crate::models::channel_bot::ChannelBot>(
            crate::models::channel_bot::COLLECTION_NAME,
        )
        .find(filter)
        .sort(doc! {"_id":-1})
        .limit(51)
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    let next = if rows.len() > 50 {
        rows.truncate(50);
        rows.last().map(|bot| bot.id.clone())
    } else {
        None
    };
    let mut eligible = Vec::new();
    for bot in rows {
        let filter = if bot.platform == "telegram" {
            doc! {"manager_bot_id":&bot.platform_bot_id,"native_entry":bson::Bson::Null}
        } else {
            doc! {"native_entry.identity.platform":&bot.platform,"native_entry.identity.platform_bot_id":&bot.platform_bot_id}
        };
        if db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(filter)
            .projection(doc! {"_id":1})
            .await?
            .is_none()
        {
            eligible.push(bot);
        }
    }
    Ok((eligible, next))
}

pub(crate) async fn display_names(
    db: &Database,
    profiles: &[PlatformBotConfig],
) -> AppResult<std::collections::HashMap<String, String>> {
    use futures::TryStreamExt;
    let bots=db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).find(doc!{"_id":{"$in":profiles.iter().map(|profile|profile.channel_bot_id.clone()).collect::<Vec<_>>()}}).projection(doc!{"_id":1,"label":1}).await?.try_collect::<Vec<_>>().await?;
    Ok(bots
        .into_iter()
        .filter_map(|row| {
            let label = row.get_str("label").ok()?;
            if label.trim().is_empty() || label.len() > 256 || label.chars().any(char::is_control) {
                return None;
            }
            Some((row.get_str("_id").ok()?.to_owned(), label.to_owned()))
        })
        .collect())
}
