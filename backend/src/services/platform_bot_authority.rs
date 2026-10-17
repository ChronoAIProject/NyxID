//! Separation of the platform manager from personal channel execution.

use mongodb::bson::{Document, doc};

use crate::errors::{AppError, AppResult};

/// Installations match a bot by channel ID, and by remote ID only for Telegram,
/// since other platforms can reuse the same numeric identifier.
pub(crate) fn installation_filter(bot_id: &str, platform: &str, platform_bot_id: &str) -> Document {
    if matches!(platform, "telegram" | "telegram-new") {
        doc! {"$or": [{"channel_bot_id": bot_id}, {"telegram_bot_id": platform_bot_id}]}
    } else {
        doc! {"channel_bot_id": bot_id}
    }
}

/// Remote-ID fallback applies only to the original Telegram namespace.
fn profile_filter(bot_id: &str, platform: &str, remote_id: &str) -> Document {
    if matches!(platform, "telegram" | "telegram-new") {
        doc! {"$or":[{"channel_bot_id":bot_id},{"manager_bot_id":remote_id,"native_entry":bson::Bson::Null}]}
    } else {
        doc! {"channel_bot_id":bot_id}
    }
}

/// Read persisted authority rather than trusting a cached channel/link snapshot.
/// This checks profile authority; callers retain their normal bot-liveness checks.
pub(crate) async fn permits_personal_routes(
    db: &mongodb::Database,
    bot_id: &str,
) -> AppResult<bool> {
    let bot = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id": bot_id})
        .await?;
    let Some(bot) = bot else {
        return Ok(false);
    };
    if bot.contains_key("platform_profile_id")
        || bot.contains_key("platform_installation_id")
        || bot.contains_key("platform_entry_version")
        || bot.contains_key("platform_entry_pending")
    {
        return Ok(false);
    }
    if db
        .collection::<Document>(crate::models::platform_installation::COLLECTION_NAME)
        .find_one(installation_filter(
            bot_id,
            bot.get_str("platform").unwrap_or(""),
            bot.get_str("platform_bot_id").unwrap_or(""),
        ))
        .await?
        .is_some()
    {
        return Ok(false);
    }
    // A paused or tombstoned profile must never restore personal routing.
    let profile = db
        .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
        .find_one(profile_filter(
            bot_id,
            bot.get_str("platform").unwrap_or(""),
            bot.get_str("platform_bot_id").unwrap_or(""),
        ))
        .await?;
    Ok(profile.is_none())
}

pub(crate) async fn require_personal_routes(db: &mongodb::Database, bot_id: &str) -> AppResult<()> {
    if !permits_personal_routes(db, bot_id).await? {
        return Err(AppError::Forbidden(
            "The platform manager cannot use personal channel routes".into(),
        ));
    }
    Ok(())
}

/// Refuse only bots that exist and are under platform control, so callers keep
/// their own not-found handling for missing bots.
pub(crate) async fn refuse_platform_bot(db: &mongodb::Database, bot_id: &str) -> AppResult<()> {
    let exists = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id": bot_id})
        .projection(doc! {"_id": 1})
        .await?
        .is_some();
    if exists {
        require_personal_routes(db, bot_id).await?;
    }
    Ok(())
}

pub(crate) async fn fence_personal_routes(
    db: &mongodb::Database,
    bot_id: &str,
    owner: &str,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    let bot = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id": bot_id, "user_id": owner, "platform_profile_id": {"$exists": false}, "platform_installation_id": {"$exists": false}, "platform_entry_version": {"$exists": false}, "platform_entry_pending": {"$exists": false}},
            doc! {"$inc": {"route_revision": 1_i64}},
        )
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Forbidden("Personal route authority is unavailable".into()))?;
    if db
        .collection::<Document>(crate::models::platform_installation::COLLECTION_NAME)
        .find_one(installation_filter(
            bot_id,
            bot.get_str("platform").unwrap_or(""),
            bot.get_str("platform_bot_id").unwrap_or(""),
        ))
        .session(&mut *session)
        .await?
        .is_some()
    {
        return Err(AppError::Forbidden(
            "This installation requires its reviewed setup authority".into(),
        ));
    }
    if db
        .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
        .find_one(profile_filter(
            bot_id,
            bot.get_str("platform").unwrap_or(""),
            bot.get_str("platform_bot_id").unwrap_or(""),
        ))
        .session(&mut *session)
        .await?
        .is_some()
    {
        return Err(AppError::Forbidden(
            "The platform manager cannot use personal channel routes".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn same_remote_native_entry_does_not_confer_telegram_platform_authority() {
        let db = crate::test_utils::connect_transaction_test_database(
            "native_remote_authority_namespace",
        )
        .await;
        let bots = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME);
        let profiles =
            db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME);
        bots.insert_one(
            doc! {"_id":"ordinary-telegram","user_id":"owner","platform":"telegram",
            "platform_bot_id":"123","credential_source":"user","is_active":true},
        )
        .await
        .unwrap();
        profiles.insert_one(doc! {"_id":"native-profile","channel_bot_id":"native-channel","manager_bot_id":"123",
            "identity_version":2,"enabled":true,"native_entry":{"identity":{"version":2,"platform":"x",
                "channel_bot_id":"native-channel","platform_bot_id":"123"}}}).await.unwrap();
        assert!(
            permits_personal_routes(&db, "ordinary-telegram")
                .await
                .unwrap()
        );
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        fence_personal_routes(&db, "ordinary-telegram", "owner", &mut session)
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        // An own-channel profile remains a refusal even if its marker is missing.
        bots.insert_one(
            doc! {"_id":"native-channel","user_id":"owner","platform":"x","platform_bot_id":"123"},
        )
        .await
        .unwrap();
        assert!(
            !permits_personal_routes(&db, "native-channel")
                .await
                .unwrap()
        );
        profiles.insert_one(doc! {"_id":"legacy-profile","channel_bot_id":"legacy-channel","manager_bot_id":"123",
            "identity_version":1,"enabled":false}).await.unwrap();
        assert!(
            !permits_personal_routes(&db, "ordinary-telegram")
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn platform_onboarding_legacy_manager_is_opt_in_and_tombstone_is_closed() {
        let db =
            crate::test_utils::connect_transaction_test_database("platform_route_opt_in").await;
        let bots = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME);
        bots.insert_one(doc! {"_id": "legacy", "platform": "telegram", "platform_bot_id": "123", "credential_source": "telegram_manager"}).await.unwrap();
        assert!(
            !permits_personal_routes(&db, "completely-missing")
                .await
                .unwrap()
        );
        assert!(permits_personal_routes(&db, "legacy").await.unwrap());
        let profiles =
            db.collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME);
        profiles.insert_one(doc! {"_id": uuid::Uuid::new_v4().to_string(), "channel_bot_id": "legacy", "manager_bot_id": "123", "enabled": false}).await.unwrap();
        assert!(!permits_personal_routes(&db, "legacy").await.unwrap());
        profiles.update_many(doc! {}, doc! {"$set": {"deleted_at": bson::DateTime::now(), "execution_profile": "unknown_future_profile"}}).await.unwrap();
        assert!(!permits_personal_routes(&db, "legacy").await.unwrap());
        bots.insert_one(doc! {"_id": "replacement", "platform": "telegram", "platform_bot_id": "123", "credential_source": "user"}).await.unwrap();
        assert!(!permits_personal_routes(&db, "replacement").await.unwrap());
        bots.insert_one(doc! {"_id": "orphan", "platform": "telegram", "platform_profile_id": "missing-profile"}).await.unwrap();
        assert!(!permits_personal_routes(&db, "orphan").await.unwrap());
        bots.delete_one(doc! {"_id": "legacy"}).await.unwrap();
        assert!(!permits_personal_routes(&db, "legacy").await.unwrap());
        db.drop().await.unwrap();
        drop(bots);
        drop(profiles);
        db.client().clone().shutdown().await;
    }
}
