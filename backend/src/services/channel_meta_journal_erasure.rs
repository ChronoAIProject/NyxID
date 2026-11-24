//! Exact attribution of legacy saved-result receipts before inbox erasure.
use crate::{
    errors::{AppError, AppResult},
    models::{channel_bot::ChannelBot, channel_text_delivery::ChannelTextDelivery},
};
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::Database;
use std::collections::BTreeSet;

#[derive(Clone, PartialEq, Eq)]
enum Target {
    Bot(String),
    OtherPlatform,
}

fn attribution_required(bot: &str) -> AppError {
    tracing::warn!(
        bot_id = bot,
        reason = "legacy_saved_reply_attribution_required",
        "Meta erasure is pending exact legacy saved-reply attribution; see docs/META_CHANNELS.md"
    );
    AppError::Conflict("Legacy saved reply attribution is missing. Erasure remains pending; restore its verified source or saved handoff metadata as described in docs/META_CHANNELS.md".into())
}

pub(crate) async fn erase_legacy(db: &Database, bot: &ChannelBot) -> AppResult<()> {
    let plans = db.collection::<Document>(crate::models::channel_text_delivery::COLLECTION_NAME);
    let mut owners = BTreeSet::from([bot.user_id.clone()]);
    let mut transfers = db
        .collection::<Document>(crate::models::ownership_transfer::COLLECTION_NAME)
        .find(doc! {"resource_kind": "channel_bot", "resource_id": &bot.id})
        .projection(doc! {"previous_owner_user_id": 1, "new_owner_user_id": 1})
        .await?;
    while let Some(transfer) = transfers.try_next().await? {
        for field in ["previous_owner_user_id", "new_owner_user_id"] {
            if let Ok(owner) = transfer.get_str(field) {
                owners.insert(owner.to_owned());
            }
        }
    }
    let mut channels = db
        .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .find(doc! {"channel_bot_id": &bot.id})
        .projection(doc! {"user_id": 1, "bot_owner_id": 1})
        .await?;
    while let Some(channel) = channels.try_next().await? {
        for field in ["user_id", "bot_owner_id"] {
            if let Ok(owner) = channel.get_str(field) {
                owners.insert(owner.to_owned());
            }
        }
    }
    // These references are authoritative while present, but both collections
    // have a 30-day TTL. Delete their exact joins before erasing either parent.
    for (collection, source_field) in [
        (crate::models::channel_message::COLLECTION_NAME, "_id"),
        (
            crate::models::channel_reply_delivery::COLLECTION_NAME,
            "inbound_message_id",
        ),
    ] {
        let mut sources = db
            .collection::<Document>(collection)
            .find(doc! {"channel_bot_id": &bot.id})
            .projection(doc! {"user_id": 1, source_field: 1})
            .await?;
        while let Some(source) = sources.try_next().await? {
            if let Ok(owner) = source.get_str("user_id") {
                owners.insert(owner.to_owned());
            }
            let (Ok(owner), Ok(id)) = (source.get_str("user_id"), source.get_str(source_field))
            else {
                // Old/corrupt but explicitly bot-scoped rows are removed by
                // the ordinary derived cleanup. They cannot prove a journal's
                // ownership: refuse if a legacy saved plan needs that join.
                if let Ok(id) = source.get_str(source_field)
                    && plans
                        .find_one(doc! {"channel_bot_id": null, "source_id": id,
                        "logical_key": {"$regex": "^saved-result:"}})
                        .projection(doc! {"_id": 1})
                        .await?
                        .is_some()
                {
                    return Err(attribution_required(&bot.id));
                }
                continue;
            };
            plans
                .delete_many(doc! {"channel_bot_id": null, "user_id": owner, "source_id": id})
                .await?;
        }
    }
    for owner in owners {
        // Meta initiated posts and native gateway sends are forbidden. On dev,
        // its only legacy persistent text plans use SAVED_RELAY_KEY. Plans
        // predating this bot cannot belong to it; absent dates prove no cutoff.
        let mut legacy = db.collection::<ChannelTextDelivery>(crate::models::channel_text_delivery::COLLECTION_NAME)
            .find(doc! {"channel_bot_id": null, "user_id": &owner,
                "logical_key": {"$regex": "^saved-result:"},
                "$or": [{"created_at": {"$gte": bson::DateTime::from_chrono(bot.created_at)}}, {"created_at": null}]})
            .await?;
        while let Some(plan) = legacy.try_next().await? {
            match target(db, &plan, &bot.platform).await? {
                Some(Target::Bot(id)) if id == bot.id => {
                    plans
                        .delete_one(doc! {"_id": &plan.id, "user_id": &plan.user_id,
                        "source_id": &plan.source_id, "logical_key": &plan.logical_key,
                        "channel_bot_id": null})
                        .await?;
                }
                Some(_) => {} // Positive sibling/other-platform proof; retain it.
                None => return Err(attribution_required(&bot.id)),
            }
        }
    }
    Ok(())
}

async fn target(
    db: &Database,
    plan: &ChannelTextDelivery,
    platform: &str,
) -> AppResult<Option<Target>> {
    if let Some(source) = db
        .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
        .find_one(doc! {"_id": &plan.source_id, "user_id": &plan.user_id})
        .projection(doc! {"channel_bot_id": 1})
        .await?
        && let Ok(bot) = source.get_str("channel_bot_id")
    {
        return Ok(Some(Target::Bot(bot.to_owned())));
    }
    let receipts: Vec<Document> = db
        .collection::<Document>(crate::models::channel_reply_delivery::COLLECTION_NAME)
        .aggregate([
            doc! {"$match": {"inbound_message_id": &plan.source_id, "user_id": &plan.user_id}},
            doc! {"$group": {"_id": "$channel_bot_id"}},
            doc! {"$limit": 2},
        ])
        .max_time(std::time::Duration::from_secs(5))
        .await?
        .try_collect()
        .await?;
    if receipts.len() == 1
        && let Ok(bot) = receipts[0].get_str("_id")
    {
        return Ok(Some(Target::Bot(bot.to_owned())));
    }
    if !receipts.is_empty() {
        return Ok(None);
    }
    let Some((turn, destination)) = plan
        .logical_key
        .strip_prefix("saved-result:")
        .and_then(|key| key.split_once(':'))
    else {
        return Ok(None);
    };
    if turn.is_empty()
        || turn.len() > 128
        || destination.len() != 64
        || !destination
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Ok(None);
    }
    let mut replies = db
        .collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
        .find(doc! {"user_id": &plan.user_id, "turn_id": turn, "role": "assistant"})
        .projection(doc! {"handoff_delivery.targets": 1})
        .await?;
    let mut proven = None;
    while let Some(reply) = replies.try_next().await? {
        let Ok(targets) = reply
            .get_document("handoff_delivery")
            .and_then(|v| v.get_array("targets"))
        else {
            continue;
        };
        for saved in targets {
            let Some(saved) = saved.as_document() else {
                continue;
            };
            if saved
                .get_document("source")
                .and_then(|v| v.get_str("turn_id"))
                != Ok(turn)
            {
                continue;
            }
            let Ok(origin) = saved.get_document("origin") else {
                continue;
            };
            let Ok(origin) = bson::from_document::<
                crate::models::assistant_conversation::ChannelOrigin,
            >(origin.clone()) else {
                continue;
            };
            if super::channel_supplemental_delivery::saved_key(turn, &origin) != plan.logical_key {
                continue;
            }
            let found = if origin.platform != platform {
                Target::OtherPlatform
            } else {
                let Some(channel) = db
                    .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
                    .find_one(doc! {"_id": &origin.nyxbot_channel_id})
                    .projection(doc! {"channel_bot_id": 1})
                    .await?
                else {
                    return Ok(None);
                };
                let Ok(bot) = channel.get_str("channel_bot_id") else {
                    return Ok(None);
                };
                Target::Bot(bot.to_owned())
            };
            if proven.as_ref().is_some_and(|prior| prior != &found) {
                return Ok(None);
            }
            proven = Some(found);
        }
    }
    Ok(proven)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::assistant_conversation::ChannelOrigin;
    use crate::services::{
        channel_adapters::meta::tests::fixture, channel_supplemental_delivery,
        channel_text_delivery,
    };

    fn parent(owner: &str, bot: &str, id: &str, platform: &str) -> Document {
        doc! {"_id": id, "user_id": owner, "channel_bot_id": bot, "platform": platform,
        "bot_label": "Historical inbox", "transport": "direct", "status": "disconnected",
        "route_api_key_id": uuid::Uuid::new_v4().to_string(),
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now()}
    }
    fn saved(turn: &str, origin: &ChannelOrigin) -> Document {
        doc! {"origin": bson::to_bson(origin).unwrap(),
        "source": {"turn_id": turn, "event_id": null, "ids": [], "expected": true},
        "additional": false}
    }

    #[tokio::test]
    async fn expired_sources_use_exact_saved_destinations_and_unknown_history_keeps_erasure_pending()
     {
        let (state, _, _, mut bot) = fixture("instagram").await;
        bot.created_at = chrono::Utc::now() - chrono::Duration::days(60);
        bot.is_active = false;
        bot.meta_binding.as_mut().unwrap().subscription = "removed".into();
        state
            .db
            .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .replace_one(doc! {"_id": &bot.id}, &bot)
            .await
            .unwrap();
        let sibling = uuid::Uuid::new_v4().to_string();
        let own_channel = uuid::Uuid::new_v4().to_string();
        let sibling_channel = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
            .insert_many([
                parent(&bot.user_id, &bot.id, &own_channel, "instagram"),
                parent(&bot.user_id, &sibling, &sibling_channel, "instagram"),
            ])
            .await
            .unwrap();
        let origin = |channel: &str, platform: &str| ChannelOrigin {
            org_execution: None,
            native_event_id: None,
            thread: None,
            nyxbot_channel_id: channel.into(),
            partition: "direct-saved-owner".into(),
            platform: platform.into(),
        };
        let own = origin(&own_channel, "instagram");
        let other = origin(&sibling_channel, "instagram");
        let telegram = origin("removed-telegram-parent", "telegram");
        let turn = uuid::Uuid::new_v4().to_string();
        state.db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
            .insert_one(doc! {"_id": uuid::Uuid::new_v4().to_string(), "user_id": &bot.user_id,
                "turn_id": &turn, "role": "assistant", "handoff_delivery": {
                    "targets": [saved(&turn, &own), saved(&turn, &other), saved(&turn, &telegram)]}})
            .await.unwrap();
        let own_key = channel_supplemental_delivery::saved_key(&turn, &own);
        let other_key = channel_supplemental_delivery::saved_key(&turn, &other);
        let telegram_key = channel_supplemental_delivery::saved_key(&turn, &telegram);
        let missing_turn = uuid::Uuid::new_v4().to_string();
        let unknown_key = channel_supplemental_delivery::saved_key(&missing_turn, &own);
        for key in [&own_key, &other_key, &telegram_key, &unknown_key] {
            let plan = channel_text_delivery::begin(
                &state.db,
                key,
                &bot.user_id,
                &uuid::Uuid::new_v4().to_string(),
                "historical",
                1,
            )
            .await
            .unwrap();
            state.db.collection::<Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
                .update_one(doc! {"_id": &plan.id}, doc! {"$set": {
                    "created_at": bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::days(40)),
                    "parts.0.platform_message_id": "retained-provider-id", "parts.0.state": "sent",
                }}).await.unwrap();
        }
        let mut slack_bot = bot.clone();
        slack_bot.id = uuid::Uuid::new_v4().to_string();
        slack_bot.platform = "slack".into();
        slack_bot.is_active = true;
        slack_bot.meta_binding = None;
        state
            .db
            .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_one(&slack_bot)
            .await
            .unwrap();
        let slack_key = channel_supplemental_delivery::saved_key(
            &uuid::Uuid::new_v4().to_string(),
            &origin("expired-slack-channel", "slack"),
        );
        channel_text_delivery::begin_bound(
            &state.db,
            &slack_key,
            &slack_bot,
            "expired-slack-source",
            "new-slack-plan",
            1,
        )
        .await
        .unwrap();
        // Both TTL collections are already empty; attribution uses the exact
        // saved-result destination, not a guessed owner-wide receipt purge.
        let grant = bot.meta_binding.as_ref().unwrap().grant_id.as_str();
        let result =
            super::super::personal_erasure_service::erase_provider_inbox(&state, &bot.id, grant)
                .await;
        assert!(
            matches!(result, Err(AppError::Conflict(message)) if message.contains("attribution"))
        );
        assert!(
            state
                .db
                .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
                .find_one(doc! {"_id": &bot.id})
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            channel_text_delivery::load(&state.db, &other_key)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            channel_text_delivery::load(&state.db, &telegram_key)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            state
                .db
                .collection::<Document>("channel_meta_erasure_fences")
                .find_one(doc! {"_id": &bot.id, "local_data_erased_at": {"$exists": true}})
                .await
                .unwrap()
                .is_none()
        );
        // Operational recovery restores independently verified immutable
        // handoff metadata. It never asserts an inbox through a checkbox.
        state
            .db
            .collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
            .insert_one(
                doc! {"_id": uuid::Uuid::new_v4().to_string(), "user_id": &bot.user_id,
                "turn_id": &missing_turn, "role": "assistant",
                "handoff_delivery": {"targets": [saved(&missing_turn, &own)]}},
            )
            .await
            .unwrap();
        super::super::personal_erasure_service::erase_provider_inbox(&state, &bot.id, grant)
            .await
            .unwrap();
        for key in [&own_key, &unknown_key] {
            assert!(
                channel_text_delivery::load(&state.db, key)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        for key in [&other_key, &telegram_key, &slack_key] {
            assert!(
                channel_text_delivery::load(&state.db, key)
                    .await
                    .unwrap()
                    .is_some()
            );
        }
        // The retained non-identifying completion marker makes a crash retry
        // after deleting the retired bot and all its source rows idempotent.
        super::super::personal_erasure_service::erase_provider_inbox(&state, &bot.id, grant)
            .await
            .unwrap();
    }
}
