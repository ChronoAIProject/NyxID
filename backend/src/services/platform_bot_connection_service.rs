//! Administrator recovery for the exact reserved provider account. Ordinary owner
//! channel endpoints deliberately cannot mutate a converted shared bot.
use super::{
    api_key_mutation_service as tx, platform_bot_config_service as configs,
    platform_channel_service as channels,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{channel_bot::ChannelBot, platform_bot_config::PlatformBotConfig},
};
use bson::{Document, doc};
use std::collections::BTreeMap;

fn changed() -> AppError {
    AppError::Conflict("The shared bot connection changed. Reload and review it again.".into())
}

fn exact_bot(profile: &PlatformBotConfig, bot: &ChannelBot) -> AppResult<()> {
    let transport = profile.channel_transport.as_ref().ok_or_else(changed)?;
    if profile.deleted_at.is_some()
        || !bot.is_active
        || profile.channel_bot_id != bot.id
        || transport.platform != bot.platform
        || transport.platform_bot_id != bot.platform_bot_id
        || transport.owner_user_id != bot.user_id
        || transport.ownership_version != bot.ownership_version
    {
        return Err(AppError::Conflict("Restore the original bot and owner before repairing, or retire this profile. A different account requires a separate shared bot.".into()));
    }
    Ok(())
}

/// A read-only provider check never activates a manual webhook or adopts drift.
pub(crate) async fn verify(
    state: &AppState,
    id: &str,
    generation: i64,
    actor: &str,
) -> AppResult<PlatformBotConfig> {
    let profile = configs::get(&state.db, id).await?;
    if profile.generation != generation {
        return Err(changed());
    }
    let bot = super::channel_bot_service::get_bot(&state.db, &profile.channel_bot_id).await?;
    exact_bot(&profile, &bot)?;
    let adapter =
        super::channel_adapters::resolve_adapter(&bot.platform, &state.token_exchange_cache)?;
    let _refreshed_token = super::channel_credentials::resolve_bot_token(
        &state.db,
        &state.encryption_keys,
        adapter.as_ref(),
        &bot,
    )
    .await?;
    let mut read = state.db.client().start_session().await?;
    let snapshot = channels::CredentialSnapshot::load(&state.db, &mut read, &bot, false).await?;
    let token = snapshot.token(&state.encryption_keys).await?;
    let secrets = snapshot
        .secrets(&state.encryption_keys, adapter.as_ref())
        .await?;
    let billing = super::channel_billing_service::ChannelBilling::for_bot(
        &state.db,
        &state.billing,
        &bot,
        None,
    );
    adapter.validate_stored_verification(&bot)?;
    let identity = adapter
        .verify_bot_token(
            &state.http_client,
            &super::channel_platform::BotCredentials {
                token: &token,
                platform_bot_id: Some(&bot.platform_bot_id),
                platform_secrets: Some(&secrets),
                billing: billing.as_ref(),
            },
        )
        .await?;
    if identity.platform_bot_id != bot.platform_bot_id {
        return Err(changed());
    }
    if bot.platform == "telegram" {
        let webhook = super::telegram_new_api::TelegramApi::new(&state.http_client)
            .call(&token, "getWebhookInfo", serde_json::json!({}))
            .await?;
        super::telegram_new_admin::validate_manager_webhook(
            &webhook,
            &super::channel_bot_service::webhook_url(&state.config.base_url, &bot),
        )?;
    }
    if bot.platform == "x" && bot.webhook_registered {
        super::channel_connection_webhook_service::check_live(
            &state.db,
            &state.encryption_keys,
            &state.http_client,
            adapter.as_ref(),
            &bot,
            &state.config.base_url,
        )
        .await?;
    }
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run((&state.db,actor,&profile),|session,(db,actor,profile)|Box::pin(async move {
        let result: AppResult<()> = async {
            configs::fence_admin(db,actor,session).await?;
            if !channels::fence_transport(db,session,profile,None,true).await? {return Err(AppError::Conflict("Credentials or delivery changed. Use Repair connection, then resume the selected modes.".into()));}
            if db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME).update_one(doc! {"_id":&profile.id,"generation":generation,"deleted_at":bson::Bson::Null},doc! {"$inc":{"verification_revision":1_i64}}).session(session).await?.matched_count!=1 {return Err(changed());}
            Ok(())
        }.await;tx::transaction_result(result)
    })).await.map_err(tx::map_transaction_error)?;
    Ok(profile)
}

/// Recovery always pauses first and changes authority. Reverification can never
/// resurrect a previous consent proof or pending delivery after credential drift.
pub(crate) async fn repair(
    state: &AppState,
    id: &str,
    generation: i64,
    actor: &str,
    supplied: &BTreeMap<String, zeroize::Zeroizing<String>>,
    connection_id: Option<&str>,
) -> AppResult<PlatformBotConfig> {
    super::telegram_new_service::with_operation(&state.db,&format!("platform-channel-recovery:{id}"),async {
        let profile = configs::get(&state.db,id).await?;
        if profile.generation != generation {return Err(changed());}
        let mut bot = super::channel_bot_service::get_bot(&state.db,&profile.channel_bot_id).await?;
        exact_bot(&profile,&bot)?;
        let original_filter=super::channel_verification_service::credential_filter(&bot)?;
        let adapter = super::channel_adapters::resolve_adapter(&bot.platform,&state.token_exchange_cache)?;
        let descriptor = adapter.registration();
        let mut fields = super::channel_registration::RegistrationValues::default();
        if supplied.len()>16 {return Err(AppError::ValidationError("Too many connection fields".into()));}
        for (name,value) in supplied {
            let field = descriptor.fields.iter().find(|field|field.name==name && field.storage!="platform_bot_id")
                .ok_or_else(||AppError::ValidationError("Unsupported connection field".into()))?;
            if value.is_empty() || value.len()>16384 {return Err(AppError::ValidationError(format!("{} is blank or too large",field.label)));}
            fields.0.insert(field.name,value.as_str());
        }
        if !fields.0.is_empty() && bot.credential_source!="user" {return Err(AppError::ValidationError("Use Admin platform credentials for managed secrets; reconnect OAuth as the bot's owner, then repair here.".into()));}
        if let Some(connection) = connection_id {
            if bot.credential_source!="connection" {return Err(AppError::ValidationError("This bot does not use OAuth".into()));}
            bot.connection_id=Some(connection.into());
        }
        let mut set = doc! {};
        super::channel_bot_service::write_registration_fields(&state.encryption_keys,&descriptor,&fields,&mut set,&mut doc!{}).await?;
        let mut stored = bson::to_document(&bot).map_err(|_|changed())?;
        stored.extend(set);
        bot=bson::from_document(stored).map_err(|_|changed())?;
        let current = super::channel_credentials::resolve_bot_token(&state.db,&state.encryption_keys,adapter.as_ref(),&bot).await?;
        let token=adapter.updated_token(&current,&fields)?.unwrap_or(current);
        if !fields.0.is_empty() {bot.bot_token_encrypted=state.encryption_keys.encrypt(token.as_bytes()).await?;}
        let mut read=state.db.client().start_session().await?;
        read.start_transaction().await?;
        let snapshot=channels::CredentialSnapshot::load(&state.db,&mut read,&bot,false).await?;
        read.commit_transaction().await?;
        let token=snapshot.token(&state.encryption_keys).await?;
        let secrets=snapshot.secrets(&state.encryption_keys,adapter.as_ref()).await?;
        let checked_stamp=snapshot.stamp()?;
        let checked_bot=bot.clone();
        let billing=super::channel_billing_service::ChannelBilling::for_bot(&state.db,&state.billing,&bot,None);
        adapter.validate_stored_verification(&bot)?;
        let identity=adapter.verify_bot_token(&state.http_client,&super::channel_platform::BotCredentials {token:&token,platform_bot_id:Some(&bot.platform_bot_id),platform_secrets:Some(&secrets),billing:billing.as_ref()}).await?;
        if identity.platform_bot_id!=bot.platform_bot_id {return Err(AppError::ValidationError("Replacement credentials belong to another bot. Supply credentials for this exact provider account.".into()));}
        // Telegram's secret is renewed before registration; manual providers retain
        // their dashboard verification token and must deliver a signed event.
        let raw_secret=zeroize::Zeroizing::new(uuid::Uuid::new_v4().simple().to_string());
        if bot.platform=="telegram" {
            use sha2::{Digest,Sha256};
            bot.webhook_secret_hash=hex::encode(Sha256::digest(raw_secret.as_bytes()));
        }
        bot.status=if bot.platform=="x" {"active"} else {"pending_webhook"}.into();
        bot.webhook_registered=false;
        bot.updated_at=chrono::Utc::now();
        let mut session=state.db.client().start_session().await?;
        session.start_transaction().and_run((&state.db,actor,&profile,&bot,&original_filter,&checked_bot,&checked_stamp),|session,(db,actor,profile,bot,original_filter,checked_bot,checked_stamp)|Box::pin(async move {
            let result:AppResult<()> = async {
                configs::fence_admin(db,actor,session).await?;
                if channels::credential_stamp(db,session,checked_bot,true).await?!=checked_stamp.as_str() {return Err(changed());}
                let live=configs::get(db,&profile.id).await?;
                if live.generation!=generation {return Err(changed());}
                let mut filter=doc! {"_id":&profile.id,"generation":generation,"deleted_at":bson::Bson::Null};
                filter.insert("channel_transport",bson::to_bson(&profile.channel_transport).map_err(|_|changed())?);
                if db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME).update_one(filter,doc! {"$set":{"enabled":false,"updated_at":bson::DateTime::now()},"$inc":{"generation":1_i64}}).session(&mut *session).await?.matched_count!=1 {return Err(changed());}
                super::platform_bot_runtime_control_service::emergency_stop_in_session(db,session,&profile.id,false).await?;
                db.collection::<Document>(crate::models::platform_bot_runtime_controls::COLLECTION_NAME).update_one(doc! {"platform_config_id":&profile.id},doc! {"$inc":{"manager_authority_epoch":1_i64}}).session(&mut *session).await?;
                let mut changes=bson::to_document(bot).map_err(|_|changed())?; changes.remove("_id");
                changes.insert("platform_profile_id",&profile.id);
                if db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(original_filter.clone(),doc! {"$set":changes,"$unset":{"shared_discord_commands_ready":""},"$inc":{"personal_effect_revision":1_i64}}).session(&mut *session).await?.matched_count!=1 {return Err(changed());}
                let stamp=channels::credential_stamp(db,session,bot,true).await?;
                db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME).update_one(doc! {"_id":&profile.id},doc! {"$set":{"channel_transport.credential_stamp":stamp,"observation_id":uuid::Uuid::new_v4().to_string()}}).session(session).await?;
                Ok(())
            }.await;tx::transaction_result(result)
        })).await.map_err(tx::map_transaction_error)?;
        if bot.platform=="x" {
            // Existing service preserves transport billing/webhook policy. Free
            // connections without webhook credentials keep their polling cursor.
            super::channel_connection_webhook_service::configure(&state.db,&state.billing,&state.encryption_keys,&state.http_client,adapter.as_ref(),&bot,&state.config.base_url).await?;
        } else {
            adapter.setup_bot_webhook(&state.db,&state.http_client,&bot,&token,&super::channel_bot_service::webhook_url(&state.config.base_url,&bot),&raw_secret).await?;
            if descriptor.automatic_webhook {
                state.db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(doc! {"_id":&bot.id,"updated_at":bson::DateTime::from_chrono(bot.updated_at)},doc! {"$set":{"status":"active","webhook_registered":true}}).await?;
            }
        }
        configs::get(&state.db,id).await
    }).await
}

pub(crate) async fn configure_discord_commands(
    state: &AppState,
    id: &str,
    generation: i64,
    actor: &str,
) -> AppResult<PlatformBotConfig> {
    super::telegram_new_service::with_operation(&state.db,&format!("platform-channel-recovery:{id}"),async {
        let profile=configs::get(&state.db,id).await?;
        if profile.generation!=generation {return Err(changed());}
        let bot=super::channel_bot_service::get_bot(&state.db,&profile.channel_bot_id).await?;
        exact_bot(&profile,&bot)?;
        if bot.platform!="discord" {return Err(AppError::ValidationError("Discord command setup requires a Discord bot".into()));}
        let adapter=super::channel_adapters::resolve_adapter("discord",&state.token_exchange_cache)?;
        let token=super::channel_credentials::resolve_bot_token(&state.db,&state.encryption_keys,adapter.as_ref(),&bot).await?;
        let filter=super::channel_verification_service::credential_filter(&bot)?;
        let mut session=state.db.client().start_session().await?;
        session.start_transaction().and_run((&state.db,actor,&profile,&filter),|session,(db,actor,profile,filter)|Box::pin(async move {
            let result:AppResult<()> = async {
                configs::fence_admin(db,actor,session).await?;
                if db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME).update_one(doc! {"_id":&profile.id,"generation":generation,"deleted_at":bson::Bson::Null},doc! {"$inc":{"connection_setup_revision":1_i64}}).session(&mut *session).await?.matched_count!=1 || db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(filter.clone(),doc! {"$inc":{"personal_effect_revision":1_i64}}).session(session).await?.matched_count!=1 {return Err(changed());}
                Ok(())
            }.await;tx::transaction_result(result)
        })).await.map_err(tx::map_transaction_error)?;
        super::channel_adapters::discord::DiscordAdapter::default().configure_shared_commands(&state.http_client,&token,&bot.platform_bot_id).await?;
        let mut session=state.db.client().start_session().await?;
        session.start_transaction().and_run((&state.db,actor,&profile,&filter),|session,(db,actor,profile,filter)|Box::pin(async move {
            let result:AppResult<()> = async {
                configs::fence_admin(db,actor,session).await?;
                if db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME).update_one(doc! {"_id":&profile.id,"generation":generation,"deleted_at":bson::Bson::Null},doc! {"$inc":{"connection_setup_revision":1_i64}}).session(&mut *session).await?.matched_count!=1 || db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(filter.clone(),doc! {"$set":{"shared_discord_commands_ready":true},"$inc":{"personal_effect_revision":1_i64}}).session(session).await?.matched_count!=1 {return Err(changed());}
                Ok(())
            }.await;tx::transaction_result(result)
        })).await.map_err(tx::map_transaction_error)?;
        Ok(profile)
    }).await
}
