//! Owner-requested relay Verify Token replacement; no transport orchestration.
use super::*;
use crate::services::{
    api_key_mutation_service::{map_transaction_error, transaction_result},
    audit_service::{self, AuditActor},
};
use mongodb::bson::{self, Bson};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

pub struct IssuedToken {
    pub verify_token: Zeroizing<String>,
    pub revision: i64,
}

impl std::fmt::Debug for IssuedToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("IssuedToken([REDACTED])")
    }
}

/// First-party bot-write authorization is required at the HTTP boundary.
/// Existing rollback state authorizes recovery even after the flag is disabled.
pub async fn issue(
    db: &Database,
    bot: &ChannelBot,
    revision: i64,
    actor: &AuditActor,
) -> AppResult<IssuedToken> {
    issue_inner(db, bot, revision, actor, "native_to_relay_rollback").await
}

/// Sweep-owned replacement used when Meta is being restored automatically.
/// It is still attributed to the bot owner, while the metadata purpose makes
/// the automatic path distinguishable from the one-time UI reveal.
pub async fn issue_for_restore(
    db: &Database,
    bot: &ChannelBot,
    revision: i64,
) -> AppResult<IssuedToken> {
    let actor = AuditActor {
        user_id: bot.user_id.clone(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    issue_inner(
        db,
        bot,
        revision,
        &actor,
        "native_to_relay_rollback_automatic",
    )
    .await
}

async fn issue_inner(
    db: &Database,
    bot: &ChannelBot,
    revision: i64,
    actor: &AuditActor,
    purpose: &str,
) -> AppResult<IssuedToken> {
    eligibility(bot).map_err(refusal)?;
    let next_revision = revision
        .checked_add(1)
        .filter(|next| revision > 0 && *next > 0)
        .ok_or_else(|| refusal("invalid_revision"))?;
    // Same 256-bit random hex token / SHA-256 storage contract as registration.
    let verify_token = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
    let hash = hex::encode(Sha256::digest(verify_token.as_bytes()));
    let mut session = db.client().start_session().await?;
    let db = db.clone();
    let bot = bot.clone();
    let actor = actor.clone();
    let purpose = purpose.to_owned();
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                let configs = db.collection::<NativeWhatsapp>(COLLECTION_NAME);
                let config = configs
                    .find_one(doc! {"_id": &bot.id})
                    .session(&mut *session)
                    .await?
                    .ok_or_else(|| refusal("native_whatsapp_rollback_required"))?;
                if !matches_bot(&config, &bot)
                    || config.callback.phase != CallbackPhase::AwaitingRelay
                {
                    return Err(refusal("native_whatsapp_rollback_required"));
                }
                if config.revision != revision {
                    return Err(refusal("settings_changed_reload"));
                }
                let now = bson::DateTime::now();
                // Write fence shared with settings/migration revision changes.
                if configs
                    .update_one(
                        doc! {"_id": &bot.id, "revision": revision,
                        "callback.phase": "awaiting_relay"},
                        doc! {"$inc": {"revision": 1_i64}, "$set": {
                            "updated_at": now,
                            "callback.relay_token_replaced_at": now,
                        }},
                    )
                    .session(&mut *session)
                    .await?
                    .matched_count
                    != 1
                {
                    return Err(refusal("settings_changed_reload"));
                }
                let ownership = if bot.ownership_version == 0 {
                    Bson::Document(doc! {"$in": [Bson::Int64(0), Bson::Null]})
                } else {
                    Bson::Int64(bot.ownership_version)
                };
                let filter = doc! {
                    "_id": &bot.id, "user_id": &bot.user_id,
                    "ownership_version": ownership, "platform": "whatsapp", "is_active": true,
                    "platform_bot_id": &bot.platform_bot_id,
                    "credential_source": {"$in": [Bson::String("user".into()), Bson::Null]},
                    "webhook_secret_hash": &bot.webhook_secret_hash,
                };
                let updated = db
                    .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
                    .update_one(
                        filter,
                        doc! {"$set": {"webhook_secret_hash": &hash, "updated_at": now}},
                    )
                    .session(&mut *session)
                    .await?;
                if updated.matched_count != 1 {
                    return Err(refusal("whatsapp_bot_changed_reload"));
                }
                audit_service::log_actor_event_in_transaction(
                    &db,
                    session,
                    &actor,
                    "channel_whatsapp_relay_verify_token_replaced",
                    json!({"bot_id": bot.id, "purpose": purpose,
                    "revision": next_revision, "previous_token_invalidated_immediately": true,
                    "storage": "hash_only"}),
                )
                .await?;
                Ok(())
            }
            .await;
            transaction_result(result)
        })
        .await
        .map_err(map_transaction_error)?;
    Ok(IssuedToken {
        verify_token,
        revision: next_revision,
    })
}
