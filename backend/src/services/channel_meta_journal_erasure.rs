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

const BATCH: i64 = 100;

/// Plan ownership follows the bot, not the person who linked an organization
/// bot to a personal specialist. Conversation ownership is resolved separately.
async fn bot_owners(db: &Database, bot: &ChannelBot) -> AppResult<BTreeSet<String>> {
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
    Ok(owners)
}

#[derive(Default)]
struct Evidence {
    proven: Option<Target>,
    // An ownerless row identifies a parent but cannot prove a plan's owner.
    // Independent owner-bearing evidence can recover it, never contradict it.
    parent: Option<String>,
    conflicting: bool,
}
impl Evidence {
    fn prove(&mut self, target: Target) {
        if self.proven.as_ref().is_some_and(|prior| prior != &target) {
            self.conflicting = true;
        }
        self.proven = Some(target);
    }
    fn observe(&mut self, row: &Document, owner: &str, source_field: &str) {
        let Ok(parent) = row.get_str("channel_bot_id") else {
            return;
        };
        match row.get_str("user_id") {
            Ok(found) if found == owner => self.prove(Target::Bot(parent.into())),
            Err(_) => {
                if self.parent.as_deref().is_some_and(|prior| prior != parent) {
                    self.conflicting = true;
                }
                self.parent = Some(parent.into());
            }
            _ => {}
        }
        debug_assert!(row.get_str(source_field).is_ok());
    }
    fn target(self) -> Option<Target> {
        if self.conflicting
            || self
                .parent
                .as_ref()
                .is_some_and(|parent| self.proven.as_ref() != Some(&Target::Bot(parent.clone())))
        {
            None
        } else {
            self.proven
        }
    }
}

pub(crate) async fn erase_legacy(db: &Database, bot: &ChannelBot) -> AppResult<()> {
    let plans =
        db.collection::<ChannelTextDelivery>(crate::models::channel_text_delivery::COLLECTION_NAME);
    let owners = bot_owners(db, bot).await?;
    let owners: Vec<_> = owners.into_iter().collect();
    let mut after = None;
    loop {
        // Retained owner/source evidence also covers historical noncanonical
        // keys. Only expired-source recovery assumes the saved-result format.
        // New parented plans need neither source nor channel-history scans.
        let mut filter = doc! {"channel_bot_id": null, "user_id": {"$in": &owners}};
        if let Some(id) = &after {
            filter.insert("_id", doc! {"$gt": id});
        }
        let batch: Vec<_> = plans
            .find(filter)
            .sort(doc! {"_id": 1})
            .limit(BATCH)
            .await?
            .try_collect()
            .await?;
        let Some(last) = batch.last() else {
            return Ok(());
        };
        after = Some(last.id.clone());
        let sources: Vec<_> = batch.iter().map(|plan| plan.source_id.clone()).collect();
        let messages: Vec<Document> = db
            .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
            .find(doc! {"_id": {"$in": &sources}})
            .projection(doc! {"user_id": 1, "channel_bot_id": 1})
            .await?
            .try_collect()
            .await?;
        // Group multipart receipts on the server. Both branches have a
        // supporting index, including the historical ownerless scoped case.
        let receipts: Vec<Document> = db
            .collection::<Document>(crate::models::channel_reply_delivery::COLLECTION_NAME)
            .aggregate([
                doc! {"$match": {"inbound_message_id": {"$in": &sources}, "$or": [
                    {"user_id": {"$in": &owners}},
                    {"user_id": null, "channel_bot_id": &bot.id}
                ]}},
                doc! {"$group": {"_id": {"source": "$inbound_message_id", "owner": "$user_id", "bot": "$channel_bot_id"}}},
                doc! {"$project": {"_id": 0, "inbound_message_id": "$_id.source", "user_id": "$_id.owner", "channel_bot_id": "$_id.bot"}},
            ])
            .max_time(std::time::Duration::from_secs(5))
            .await?.try_collect().await?;
        let mut remove = Vec::new();
        let mut unresolved = false;
        for plan in &batch {
            let mut evidence = Evidence::default();
            for message in messages
                .iter()
                .filter(|row| row.get_str("_id") == Ok(plan.source_id.as_str()))
            {
                evidence.observe(message, &plan.user_id, "_id");
            }
            for receipt in receipts
                .iter()
                .filter(|row| row.get_str("inbound_message_id") == Ok(plan.source_id.as_str()))
            {
                evidence.observe(receipt, &plan.user_id, "inbound_message_id");
            }
            if evidence.proven.is_none() && !evidence.conflicting {
                let canonical = plan.logical_key.starts_with("saved-result:")
                    && plan.created_at >= bot.created_at;
                if !canonical && evidence.parent.is_none() {
                    // No exact join and no supported Meta saved-result key:
                    // retain this unrelated historical plan without guessing.
                    continue;
                }
                if let Some(target) = saved_target(db, plan, &owners, &bot.platform).await? {
                    evidence.prove(target);
                }
            }
            match evidence.target() {
                Some(Target::Bot(id)) if id == bot.id => remove.push(doc! {
                    "_id": &plan.id, "user_id": &plan.user_id,
                    "source_id": &plan.source_id, "logical_key": &plan.logical_key,
                }),
                Some(_) => {} // Positive sibling/other-platform proof; retain it.
                None => unresolved = true,
            }
        }
        if !remove.is_empty() {
            plans
                .delete_many(doc! {"channel_bot_id": null, "$or": remove})
                .await?;
        }
        if unresolved {
            return Err(attribution_required(&bot.id));
        }
    }
}

async fn saved_target(
    db: &Database,
    plan: &ChannelTextDelivery,
    owners: &[String],
    platform: &str,
) -> AppResult<Option<Target>> {
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
        .find(doc! {"turn_id": turn, "role": "assistant"})
        .projection(doc! {"user_id": 1, "handoff_delivery.targets": 1})
        .limit(65)
        .max_time(std::time::Duration::from_secs(5))
        .await?;
    let mut evidence = Evidence::default();
    let mut read = 0;
    while let Some(reply) = replies.try_next().await? {
        read += 1;
        if read > 64 {
            return Ok(None);
        }
        let Ok(person) = reply.get_str("user_id") else {
            continue;
        };
        let Ok(targets) = reply
            .get_document("handoff_delivery")
            .and_then(|v| v.get_array("targets"))
        else {
            continue;
        };
        for saved in targets.iter().filter_map(|target| target.as_document()) {
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
            let channel = db
                .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
                .find_one(doc! {"_id": &origin.nyxbot_channel_id})
                .projection(
                    doc! {"channel_bot_id": 1, "user_id": 1, "bot_owner_id": 1, "platform": 1},
                )
                .await?;
            let found = if let Some(channel) = channel {
                let owner = channel
                    .get_str("bot_owner_id")
                    .or_else(|_| channel.get_str("user_id"));
                if channel.get_str("user_id") != Ok(person)
                    || channel.get_str("platform") != Ok(origin.platform.as_str())
                {
                    return Ok(None);
                }
                if origin.platform != platform && person == plan.user_id {
                    // Native org bots journal under the linking person. Its
                    // verified different-platform destination cannot belong to
                    // this person's Meta inbox, irrespective of the bot owner.
                    Target::OtherPlatform
                } else {
                    if !owner.is_ok_and(|owner| owners.iter().any(|id| id == owner)) {
                        return Ok(None);
                    }
                    let Ok(bot) = channel.get_str("channel_bot_id") else {
                        return Ok(None);
                    };
                    Target::Bot(bot.to_owned())
                }
            } else if origin.platform != platform && person == plan.user_id {
                // A retained non-Meta destination proves this cannot be a Meta
                // plan even when its old channel was already removed.
                Target::OtherPlatform
            } else {
                return Ok(None);
            };
            evidence.prove(found);
        }
    }
    Ok(evidence.target())
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
        let unknown = channel_text_delivery::load(&state.db, &unknown_key)
            .await
            .unwrap()
            .unwrap();
        state
            .db
            .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(doc! {"_id": &unknown.source_id, "channel_bot_id": &bot.id})
            .await
            .unwrap();
        // The ownerless source is a constraint, not proof. Independent handoff
        // recovery below must work without editing or deleting this row.
        // Both TTL collections have no owner-bearing source; attribution uses the exact
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
    fn plan(owner: &str, source: &str, parent: Option<&str>) -> Document {
        let mut row = doc! {"_id": uuid::Uuid::new_v4().to_string(),
        "logical_key": format!("saved-result:{}:{}", uuid::Uuid::new_v4(), "a".repeat(64)),
        "user_id": owner, "source_id": source, "fingerprint": "retained",
        "parts": [{"state": "sent", "platform_message_id": "provider-id"}],
        "created_at": bson::DateTime::now()};
        if let Some(parent) = parent {
            row.insert("channel_bot_id", parent);
        }
        row
    }

    #[tokio::test]
    async fn org_bot_legacy_plans_resolve_personal_specialist_handoffs_without_claiming_the_admins_receipts()
     {
        let (state, _, _server, mut bot) = fixture("instagram").await;
        let admin = bot.user_id.clone();
        let org = uuid::Uuid::new_v4().to_string();
        bot.user_id = org.clone();
        bot.created_at -= chrono::Duration::days(60);
        let own_channel = uuid::Uuid::new_v4().to_string();
        let sibling_channel = uuid::Uuid::new_v4().to_string();
        let sibling = uuid::Uuid::new_v4().to_string();
        let mut own_parent = parent(&admin, &bot.id, &own_channel, "instagram");
        own_parent.insert("bot_owner_id", &org);
        let mut sibling_parent = parent(&admin, &sibling, &sibling_channel, "instagram");
        sibling_parent.insert("bot_owner_id", &org);
        state
            .db
            .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
            .insert_many([own_parent, sibling_parent])
            .await
            .unwrap();
        let turn = uuid::Uuid::new_v4().to_string();
        let origin = |channel: &str| ChannelOrigin {
            org_execution: None,
            native_event_id: None,
            thread: None,
            nyxbot_channel_id: channel.into(),
            partition: "private-owner".into(),
            platform: "instagram".into(),
        };
        let own = origin(&own_channel);
        let other = origin(&sibling_channel);
        state.db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
            .insert_one(doc! {"_id": uuid::Uuid::new_v4().to_string(), "user_id": &admin,
            "turn_id": &turn, "role": "assistant", "handoff_delivery": {"targets": [saved(&turn, &own), saved(&turn, &other)]}})
            .await.unwrap();
        let own_key = channel_supplemental_delivery::saved_key(&turn, &own);
        let sibling_key = channel_supplemental_delivery::saved_key(&turn, &other);
        for key in [&own_key, &sibling_key] {
            channel_text_delivery::begin(
                &state.db,
                key,
                &org,
                &uuid::Uuid::new_v4().to_string(),
                "old",
                1,
            )
            .await
            .unwrap();
        }
        let personal = plan(
            &admin,
            "unrelated-personal-source-without-any-history",
            None,
        );
        state
            .db
            .collection::<Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
            .insert_one(&personal)
            .await
            .unwrap();
        erase_legacy(&state.db, &bot).await.unwrap();
        assert!(
            channel_text_delivery::load(&state.db, &own_key)
                .await
                .unwrap()
                .is_none()
        );
        for key in [
            sibling_key.as_str(),
            personal.get_str("logical_key").unwrap(),
        ] {
            assert!(
                channel_text_delivery::load(&state.db, key)
                    .await
                    .unwrap()
                    .is_some()
            );
        }
    }

    #[tokio::test]
    async fn ownerless_metadata_accepts_independent_owner_receipts_but_rejects_contradictory_parents()
     {
        for contradiction in [false, true] {
            let (state, _, _server, bot) = fixture("instagram").await;
            let source = uuid::Uuid::new_v4().to_string();
            let row = plan(&bot.user_id, &source, None);
            let key = row.get_str("logical_key").unwrap();
            state
                .db
                .collection::<Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
                .insert_one(&row)
                .await
                .unwrap();
            state
                .db
                .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
                .insert_one(doc! {"_id": &source, "channel_bot_id": &bot.id})
                .await
                .unwrap();
            let receipt = uuid::Uuid::new_v4().to_string();
            let parent = if contradiction {
                uuid::Uuid::new_v4().to_string()
            } else {
                bot.id.clone()
            };
            let receipts = state
                .db
                .collection::<Document>(crate::models::channel_reply_delivery::COLLECTION_NAME);
            receipts
                .insert_one(doc! {"_id": &receipt, "user_id": &bot.user_id,
                "inbound_message_id": &source, "channel_bot_id": parent})
                .await
                .unwrap();
            let result = erase_legacy(&state.db, &bot).await;
            if contradiction {
                assert!(matches!(result, Err(AppError::Conflict(_))));
                assert!(
                    channel_text_delivery::load(&state.db, key)
                        .await
                        .unwrap()
                        .is_some()
                );
                receipts
                    .update_one(
                        doc! {"_id": &receipt},
                        doc! {"$set": {"channel_bot_id": &bot.id}},
                    )
                    .await
                    .unwrap();
                erase_legacy(&state.db, &bot).await.unwrap();
            } else {
                result.unwrap();
            }
            assert!(
                channel_text_delivery::load(&state.db, key)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }

    #[tokio::test]
    async fn legacy_erasure_uses_bounded_source_batches_and_parented_plans_need_no_source_scan() {
        let (state, _, _server, bot) = fixture("instagram").await;
        super::super::channel_reply_delivery::ensure_indexes(&state.db)
            .await
            .unwrap();
        super::super::assistant_nyxagent::ensure_indexes(&state.db)
            .await
            .unwrap();
        let turn_indexes: Vec<_> = state
            .db
            .collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(
            turn_indexes
                .iter()
                .any(|index| index.keys == doc! {"turn_id": 1, "role": 1})
        );
        let indexes: Vec<_> = state
            .db
            .collection::<Document>(crate::models::channel_reply_delivery::COLLECTION_NAME)
            .list_indexes()
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(
            indexes
                .iter()
                .any(|index| index.keys == doc! {"user_id": 1, "inbound_message_id": 1})
        );
        let plans = state
            .db
            .collection::<Document>(crate::models::channel_text_delivery::COLLECTION_NAME);
        let messages = state
            .db
            .collection::<Document>(crate::models::channel_message::COLLECTION_NAME);
        let mut parented = Vec::new();
        let mut sources = Vec::new();
        for _ in 0..205 {
            let source = uuid::Uuid::new_v4().to_string();
            parented.push(plan(&bot.user_id, &source, Some(&bot.id)));
            sources.push(doc! {"_id": source, "user_id": &bot.user_id, "channel_bot_id": &bot.id});
        }
        plans.insert_many(parented).await.unwrap();
        messages.insert_many(sources).await.unwrap();
        let (db, commands) = super::super::durable_wake_tests::monitored(&state.db).await;
        erase_legacy(&db, &bot).await.unwrap();
        let observed = super::super::durable_wake_tests::take(&commands);
        assert_eq!(
            observed
                .get("delete:channel_text_deliveries")
                .copied()
                .unwrap_or(0),
            0
        );
        assert_eq!(
            observed.get("find:channel_messages").copied().unwrap_or(0),
            0
        );
        for command in ["find:nyxbot_channels", "find:assistant_messages"] {
            assert_eq!(observed.get(command).copied().unwrap_or(0), 0, "{command}");
        }
        let mut legacy = Vec::new();
        let mut receipts = Vec::new();
        for index in 0..205 {
            let source = uuid::Uuid::new_v4().to_string();
            let mut historical = plan(&bot.user_id, &source, None);
            if index % 2 == 0 {
                historical.insert("logical_key", format!("legacy-result-{index}"));
            }
            legacy.push(historical);
            receipts.push(
                doc! {"_id": uuid::Uuid::new_v4().to_string(), "user_id": &bot.user_id,
                "inbound_message_id": source, "channel_bot_id": &bot.id},
            );
        }
        let sibling = uuid::Uuid::new_v4().to_string();
        let source = uuid::Uuid::new_v4().to_string();
        let retained = plan(&bot.user_id, &source, None);
        receipts.push(
            doc! {"_id": uuid::Uuid::new_v4().to_string(), "user_id": &bot.user_id,
            "inbound_message_id": source, "channel_bot_id": sibling},
        );
        legacy.push(retained.clone());
        plans.insert_many(legacy).await.unwrap();
        state
            .db
            .collection::<Document>(crate::models::channel_reply_delivery::COLLECTION_NAME)
            .insert_many(receipts)
            .await
            .unwrap();
        erase_legacy(&db, &bot).await.unwrap();
        let observed = super::super::durable_wake_tests::take(&commands);
        assert_eq!(
            observed
                .get("delete:channel_text_deliveries")
                .copied()
                .unwrap_or(0),
            3
        );
        assert_eq!(
            observed
                .get("aggregate:channel_reply_deliveries")
                .copied()
                .unwrap_or(0),
            3
        );
        assert_eq!(
            plans
                .count_documents(doc! {"channel_bot_id": null})
                .await
                .unwrap(),
            1
        );
        assert!(
            plans
                .find_one(doc! {"_id": retained.get_str("_id").unwrap()})
                .await
                .unwrap()
                .is_some()
        );
    }
    #[tokio::test]
    async fn personal_meta_erasure_preserves_verified_org_native_handoffs_owned_by_the_linking_person()
     {
        let (state, _, _server, bot) = fixture("instagram").await;
        let org = uuid::Uuid::new_v4().to_string();
        let native_bot = uuid::Uuid::new_v4().to_string();
        let native_channel = uuid::Uuid::new_v4().to_string();
        let mut channel = parent(&bot.user_id, &native_bot, &native_channel, "discord");
        channel.insert("bot_owner_id", org);
        state
            .db
            .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
            .insert_one(channel)
            .await
            .unwrap();
        let origin = ChannelOrigin {
            org_execution: None,
            native_event_id: Some("retained-native-event".into()),
            thread: None,
            nyxbot_channel_id: native_channel,
            partition: "private-admin".into(),
            platform: "discord".into(),
        };
        let turn = uuid::Uuid::new_v4().to_string();
        let key = channel_supplemental_delivery::saved_key(&turn, &origin);
        channel_text_delivery::begin(
            &state.db,
            &key,
            &bot.user_id,
            "expired-native-source",
            "old-native",
            1,
        )
        .await
        .unwrap();
        state.db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
            .insert_one(doc! {"_id": uuid::Uuid::new_v4().to_string(), "user_id": &bot.user_id,
                "turn_id": &turn, "role": "assistant", "handoff_delivery": {"targets": [saved(&turn, &origin)]}})
            .await.unwrap();
        erase_legacy(&state.db, &bot).await.unwrap();
        assert!(
            channel_text_delivery::load(&state.db, &key)
                .await
                .unwrap()
                .is_some()
        );
    }
}
