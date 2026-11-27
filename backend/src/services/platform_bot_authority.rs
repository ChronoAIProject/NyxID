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

/// Preserve exact provider/remote reservations across local channel replacement.
pub(crate) fn profile_filter(bot_id: &str, platform: &str, remote_id: &str) -> Document {
    if matches!(platform, "telegram" | "telegram-new") {
        doc! {"$or":[{"channel_bot_id":bot_id},{"manager_bot_id":remote_id,"native_entry":bson::Bson::Null}]}
    } else {
        doc! {"$or":[{"channel_bot_id":bot_id},{"native_entry.identity.platform":platform,"native_entry.identity.platform_bot_id":remote_id}]}
    }
}

/// One fresh observation, passed only through a single provider-boundary check.
/// It contains no authority for a later effect and is never retained by a sender.
pub(crate) struct PersonalRouteCheck {
    bot_id: String,
    bot: Option<Document>,
    allowed: bool,
}

impl PersonalRouteCheck {
    pub(crate) fn allows(&self, bot_id: &str) -> bool {
        self.bot_id == bot_id && self.allowed
    }

    pub(crate) async fn guest_mode(
        &self,
        db: &mongodb::Database,
    ) -> AppResult<super::channel_guest_access::GuestMode> {
        super::channel_guest_access::select_mode_for_snapshot(
            db,
            &self.bot_id,
            self.bot.as_ref().filter(|_| self.allowed),
        )
        .await
    }
}

pub(crate) async fn read_personal_routes(
    db: &mongodb::Database,
    bot_id: &str,
) -> AppResult<PersonalRouteCheck> {
    let bot = db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one(doc! {"_id":bot_id})
        .await?;
    let mut checked = PersonalRouteCheck {
        bot_id: bot_id.into(),
        bot,
        allowed: false,
    };
    let Some(bot) = checked.bot.as_ref() else {
        return Ok(checked);
    };
    if [
        "platform_profile_id",
        "platform_installation_id",
        "platform_entry_version",
        "platform_entry_pending",
    ]
    .iter()
    .any(|field| bot.contains_key(*field))
    {
        return Ok(checked);
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
        return Ok(checked);
    }
    // Paused and tombstoned reservations still deny personal routing.
    checked.allowed = db
        .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
        .find_one(profile_filter(
            bot_id,
            bot.get_str("platform").unwrap_or(""),
            bot.get_str("platform_bot_id").unwrap_or(""),
        ))
        .await?
        .is_none();
    Ok(checked)
}

pub(crate) async fn permits_personal_routes(
    db: &mongodb::Database,
    bot_id: &str,
) -> AppResult<bool> {
    permits_with(db, bot_id, None).await
}

pub(crate) async fn permits_with(
    db: &mongodb::Database,
    bot_id: &str,
    checked: Option<&PersonalRouteCheck>,
) -> AppResult<bool> {
    match checked {
        Some(checked) => Ok(checked.allows(bot_id)),
        None => Ok(read_personal_routes(db, bot_id).await?.allowed),
    }
}

pub(crate) async fn require_with(
    db: &mongodb::Database,
    bot_id: &str,
    checked: Option<&PersonalRouteCheck>,
) -> AppResult<()> {
    if !permits_with(db, bot_id, checked).await? {
        return Err(AppError::Forbidden(
            "The platform manager cannot use personal channel routes".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn bot_with(
    db: &mongodb::Database,
    bot_id: &str,
    checked: Option<&PersonalRouteCheck>,
) -> AppResult<crate::models::channel_bot::ChannelBot> {
    let Some(checked) = checked else {
        return super::channel_bot_service::get_bot(db, bot_id).await;
    };
    let bot = checked
        .bot
        .as_ref()
        .filter(|_| checked.bot_id == bot_id)
        .ok_or_else(|| AppError::ChannelBotNotFound(bot_id.into()))?;
    bson::from_document(bot.clone())
        .map_err(|_| AppError::Internal("Invalid channel bot metadata".into()))
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
        for (id, platform, allowed) in
            [("replaced-x", "x", false), ("same-id-slack", "slack", true)]
        {
            bots.insert_one(doc!{"_id":id,"user_id":"owner","platform":platform,"platform_bot_id":"123","is_active":true}).await.unwrap();
            assert_eq!(permits_personal_routes(&db, id).await.unwrap(), allowed);
            let mut session = db.client().start_session().await.unwrap();
            session.start_transaction().await.unwrap();
            assert_eq!(
                fence_personal_routes(&db, id, "owner", &mut session)
                    .await
                    .is_ok(),
                allowed
            );
            session.abort_transaction().await.unwrap();
        }
        // Retirement does not free the remote identity for a new local UUID.
        profiles
            .update_one(
                doc! {"_id":"native-profile"},
                doc! {"$set":{"deleted_at":bson::DateTime::now()}},
            )
            .await
            .unwrap();
        assert!(!permits_personal_routes(&db, "replaced-x").await.unwrap());
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
