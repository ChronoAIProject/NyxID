//! Telegram setup refusals retain remote custody independently of a new connect.
use super::*;
use crate::models::channel_bot::ChannelBot;
use futures::future::BoxFuture;

pub(super) fn known_provider_setup_failure(
    db: &Database,
    release: &Release,
) -> BoxFuture<'static, AppResult<bool>> {
    let telegram_setup =
        release.platform == "telegram" && release.reason == ReleaseReason::SetupFailed;
    let known = release.review_reason.as_deref() == Some("gateway_provider_setup_required");
    let db = db.clone();
    let filter = doc! {"_id":&release.channel_id,"user_id":&release.owner_user_id,"last_error":"gateway_provider_not_registered"};
    Box::pin(async move {
        if !telegram_setup {
            return Ok(false);
        }
        if known {
            return Ok(true);
        }
        Ok(db
            .collection::<Document>(CHANNELS)
            .find_one(filter)
            .projection(doc! {"_id":1})
            .await?
            .is_some())
    })
}

/// Only an exact observed missing-provider refusal gets a short backoff. Replay
/// capability advertisements, archived generic 400s and outstanding release
/// obligations are not admission requirements. After 60 seconds the owner can
/// retry normally, with no operator reset and no prerequisite on provider replay.
pub(crate) fn preflight(
    state: &AppState,
    bot: &ChannelBot,
    owner: &str,
) -> BoxFuture<'static, AppResult<()>> {
    let db = state.db.clone();
    let filter = doc! {
        "channel_bot_id":&bot.id,"user_id":owner,"status":"failed",
        "last_error":"gateway_provider_not_registered",
        "created_at":{"$gte":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(60))},
        "$expr":{"$and":[
            {"$eq":[{"$ifNull":["$bot_owner_id",{"$ifNull":["$org_specialist.org_id","$user_id"]}]},&bot.user_id]},
            {"$eq":[{"$ifNull":["$bot_ownership_version",0_i64]},bot.ownership_version]}
        ]},
    };
    Box::pin(async move {
        let recent = db
            .collection::<Document>(CHANNELS)
            .find_one(filter)
            .sort(doc! {"created_at":-1,"_id":-1})
            .projection(doc! {"_id":1})
            .await?;
        if recent.is_some() {
            return Err(AppError::ValidationError(
                "gateway_provider_not_registered".into(),
            ));
        }
        Ok(())
    })
}
