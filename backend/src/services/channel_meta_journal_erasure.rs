//! Exact attribution of legacy saved-result receipts before inbox erasure.
use crate::{
    errors::{AppError, AppResult},
    models::channel_bot::ChannelBot,
};
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::Database;
use std::collections::BTreeSet;

/// Attribution reads omit multipart send state and stored provider receipts.
#[derive(serde::Deserialize)]
struct LegacyPlan {
    #[serde(rename = "_id")]
    id: String,
    user_id: String,
    source_id: String,
    logical_key: String,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    created_at: Option<chrono::DateTime<chrono::Utc>>,
}

fn attribution_fields() -> Document {
    doc! {"_id": 1, "user_id": 1, "source_id": 1, "logical_key": 1, "created_at": 1}
}

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
async fn bot_owners(
    db: &Database,
    bot: &str,
    current: Option<&str>,
) -> AppResult<BTreeSet<String>> {
    let mut owners: BTreeSet<String> = current.into_iter().map(str::to_owned).collect();
    let mut transfers = db
        .collection::<Document>(crate::models::ownership_transfer::COLLECTION_NAME)
        .find(doc! {"resource_kind": "channel_bot", "resource_id": bot})
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
    erase_attributed(
        db,
        &bot.id,
        Some(&bot.user_id),
        &bot.platform,
        bot.created_at,
        true,
    )
    .await
}

/// Preserve exact attribution on the original journal before its TTL sources
/// can disappear. Account erasure does not delete a former owner's receipt.
pub(crate) async fn attribute_legacy(db: &Database, bot: &ChannelBot) -> AppResult<()> {
    erase_attributed(
        db,
        &bot.id,
        Some(&bot.user_id),
        &bot.platform,
        bot.created_at,
        false,
    )
    .await
}

pub(crate) async fn erase_retired_legacy(
    db: &Database,
    bot_id: &str,
    grant: &crate::models::channel_meta::RetiredGrant,
) -> AppResult<()> {
    let account = db
        .collection::<Document>(crate::models::personal_erasure::COLLECTION_NAME)
        .find_one(doc! {"_id": &grant.account_erasure_id})
        .projection(doc! {"user_id":1})
        .await?
        .ok_or_else(|| attribution_required(bot_id))?;
    let owner = account
        .get_str("user_id")
        .map_err(|_| attribution_required(bot_id))?;
    erase_attributed(
        db,
        bot_id,
        Some(owner),
        &grant.platform,
        grant.bot_created_at,
        true,
    )
    .await
}

async fn erase_attributed(
    db: &Database,
    bot_id: &str,
    owner: Option<&str>,
    platform: &str,
    created_at: chrono::DateTime<chrono::Utc>,
    erase: bool,
) -> AppResult<()> {
    let plans = db.collection::<LegacyPlan>(crate::models::channel_text_delivery::COLLECTION_NAME);
    let owners = bot_owners(db, bot_id, owner).await?;
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
            .projection(attribution_fields())
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
                    {"user_id": null}
                ]}},
                doc! {"$group": {"_id": {"source": "$inbound_message_id", "owner": "$user_id", "bot": "$channel_bot_id"}}},
                doc! {"$project": {"_id": 0, "inbound_message_id": "$_id.source", "user_id": "$_id.owner", "channel_bot_id": "$_id.bot"}},
            ])
            .max_time(std::time::Duration::from_secs(5))
            .await?.try_collect().await?;
        // Native saved plans use the original event UUID as their source.
        // Personal conversation deletion may remove the handoff while this
        // independent delivery origin and channel parent remain available.
        let native: Vec<Document> = db
            .collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
            .find(doc! {"_id":{"$in":&sources}})
            .projection(doc! {"user_id":1,"channel_id":1,"partition":1,"delivery.origin":1})
            .max_time(std::time::Duration::from_secs(5))
            .await?
            .try_collect()
            .await?;
        let native_channels: Vec<_> = native
            .iter()
            .filter_map(|event| event.get_str("channel_id").ok())
            .collect();
        let parents: Vec<Document> = if native_channels.is_empty() {
            Vec::new()
        } else {
            db.collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
                .find(doc! {"_id":{"$in":native_channels}})
                .projection(doc! {"user_id":1,"channel_bot_id":1,"platform":1})
                .await?
                .try_collect()
                .await?
        };
        let mut remove = Vec::new();
        let mut attribute = std::collections::BTreeMap::<String, Vec<Document>>::new();
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
            for event in native
                .iter()
                .filter(|event| event.get_str("_id") == Ok(plan.source_id.as_str()))
            {
                match native_target(event, &parents, plan, bot_id, platform) {
                    Ok(Some(target)) => evidence.prove(target),
                    Ok(None) => {}
                    Err(()) => evidence.conflicting = true,
                }
            }
            if evidence.proven.is_none() && !evidence.conflicting {
                let canonical = plan.logical_key.starts_with("saved-result:");
                if !canonical && evidence.parent.is_none() {
                    // No exact join and no supported Meta saved-result key:
                    // retain this unrelated historical plan without guessing.
                    continue;
                }
                match saved_target(db, plan, &owners, platform).await? {
                    SavedTarget::Proven(target) => evidence.prove(target),
                    SavedTarget::Absent
                        if evidence.parent.is_none()
                            && plan.created_at.is_some_and(|created| created < created_at) =>
                    {
                        // Only absent evidence can exclude older history. An
                        // exact but inconsistent/incomplete handoff stays pending.
                        continue;
                    }
                    SavedTarget::Incomplete => evidence.conflicting = true,
                    SavedTarget::Absent => {}
                }
            }
            match evidence.target() {
                Some(Target::Bot(id)) if id == bot_id => remove.push(doc! {
                    "_id": &plan.id, "user_id": &plan.user_id,
                    "source_id": &plan.source_id, "logical_key": &plan.logical_key,
                }),
                Some(Target::Bot(id)) => {
                    // Preserve positive sibling proof before its finite-lived
                    // source disappears, on the original journal itself.
                    attribute.entry(id).or_default().push(doc! {
                        "_id": &plan.id, "user_id": &plan.user_id,
                        "source_id": &plan.source_id, "logical_key": &plan.logical_key,
                    });
                }
                Some(_) => {} // Positive sibling/other-platform proof; retain it.
                None => unresolved = true,
            }
        }
        for (parent, exact) in attribute {
            plans
                .update_many(
                    doc! {"channel_bot_id":null,"$or":exact},
                    doc! {"$set":{"channel_bot_id":parent}},
                )
                .await?;
        }
        if !remove.is_empty() {
            let exact = doc! {"channel_bot_id": null, "$or": remove};
            if erase {
                plans.delete_many(exact).await?;
            } else {
                plans
                    .update_many(exact, doc! {"$set":{"channel_bot_id":bot_id}})
                    .await?;
            }
        }
        if unresolved && erase {
            return Err(attribution_required(bot_id));
        }
    }
}

fn native_target(
    event: &Document,
    parents: &[Document],
    plan: &LegacyPlan,
    erased_bot: &str,
    erased_platform: &str,
) -> Result<Option<Target>, ()> {
    if event.get_str("user_id") != Ok(plan.user_id.as_str()) {
        return Ok(None);
    }
    let origin = event
        .get_document("delivery")
        .and_then(|delivery| delivery.get_document("origin"))
        .map_err(|_| ())?;
    let origin: crate::models::assistant_conversation::ChannelOrigin =
        bson::from_document(origin.clone()).map_err(|_| ())?;
    if event.get_str("channel_id") != Ok(origin.nyxbot_channel_id.as_str())
        || event.get_str("partition") != Ok(origin.partition.as_str())
        || origin
            .native_event_id
            .as_deref()
            .is_some_and(|id| id != plan.source_id)
    {
        return Err(());
    }
    let cache =
        std::sync::Arc::new(super::provider_token_exchange_service::TokenExchangeCache::new());
    if super::channel_adapters::resolve_adapter(&origin.platform, &cache).is_err() {
        return Err(());
    }
    if let Some(key) = plan.logical_key.strip_prefix("saved-result:") {
        let (turn, _) = key.split_once(':').ok_or(())?;
        if turn.is_empty()
            || super::channel_supplemental_delivery::saved_key(turn, &origin) != plan.logical_key
        {
            return Err(());
        }
    }
    let parent = parents
        .iter()
        .find(|parent| parent.get_str("_id") == Ok(origin.nyxbot_channel_id.as_str()))
        .ok_or(())?;
    if parent.get_str("user_id") != Ok(plan.user_id.as_str())
        || parent.get_str("platform") != Ok(origin.platform.as_str())
    {
        return Err(());
    }
    let bot = parent.get_str("channel_bot_id").map_err(|_| ())?;
    if bot == erased_bot && origin.platform != erased_platform {
        return Err(());
    }
    Ok(Some(Target::Bot(bot.to_owned())))
}

enum SavedTarget {
    Absent,
    Incomplete,
    Proven(Target),
}

async fn saved_target(
    db: &Database,
    plan: &LegacyPlan,
    owners: &[String],
    platform: &str,
) -> AppResult<SavedTarget> {
    let Some((turn, destination)) = plan
        .logical_key
        .strip_prefix("saved-result:")
        .and_then(|key| key.split_once(':'))
    else {
        return Ok(SavedTarget::Absent);
    };
    if turn.is_empty()
        || turn.len() > 128
        || destination.len() != 64
        || !destination
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
    {
        return Ok(SavedTarget::Absent);
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
            return Ok(SavedTarget::Incomplete);
        }
        let Ok(targets) = reply
            .get_document("handoff_delivery")
            .and_then(|v| v.get_array("targets"))
        else {
            continue;
        };
        for saved in targets.iter().filter_map(|target| target.as_document()) {
            let Ok(origin) = saved.get_document("origin") else {
                continue;
            };
            // Destination identity does not depend on provenance fields such
            // as platform. Establish an exact match before validating them.
            #[derive(serde::Deserialize)]
            struct DestinationThread {
                child_id: String,
                generation: i64,
                channel_generation: i64,
                agent_id: String,
            }
            #[derive(serde::Deserialize)]
            struct Destination {
                nyxbot_channel_id: String,
                partition: String,
                thread: Option<DestinationThread>,
            }
            let Ok(identity) = bson::from_document::<Destination>(origin.clone()) else {
                continue;
            };
            let binding = identity.thread.as_ref().map(|thread| {
                (
                    thread.child_id.as_str(),
                    thread.generation,
                    thread.channel_generation,
                    thread.agent_id.as_str(),
                )
            });
            if super::channel_supplemental_delivery::saved_destination_key(
                turn,
                &identity.nyxbot_channel_id,
                &identity.partition,
                binding,
            ) != plan.logical_key
            {
                continue;
            }
            let Ok(origin) = bson::from_document::<
                crate::models::assistant_conversation::ChannelOrigin,
            >(origin.clone()) else {
                return Ok(SavedTarget::Incomplete);
            };
            if super::channel_adapters::resolve_adapter(
                &origin.platform,
                &std::sync::Arc::new(
                    super::provider_token_exchange_service::TokenExchangeCache::new(),
                ),
            )
            .is_err()
            {
                return Ok(SavedTarget::Incomplete);
            }
            if saved
                .get_document("source")
                .and_then(|v| v.get_str("turn_id"))
                != Ok(turn)
            {
                return Ok(SavedTarget::Incomplete);
            }
            let Ok(person) = reply.get_str("user_id") else {
                return Ok(SavedTarget::Incomplete);
            };
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
                    return Ok(SavedTarget::Incomplete);
                }
                let native_person = origin.platform != platform && person == plan.user_id;
                if !native_person && !owner.is_ok_and(|owner| owners.iter().any(|id| id == owner)) {
                    return Ok(SavedTarget::Incomplete);
                }
                // Preserve a retained parent's concrete identity, including
                // foreign org native bots. It must agree with any ownerless
                // source/receipt constraint before erasure may finish.
                let Ok(bot) = channel.get_str("channel_bot_id") else {
                    return Ok(SavedTarget::Incomplete);
                };
                Target::Bot(bot.to_owned())
            } else if origin.platform != platform && person == plan.user_id {
                // A retained non-Meta destination proves this cannot be a Meta
                // plan even when its old channel was already removed.
                Target::OtherPlatform
            } else {
                return Ok(SavedTarget::Incomplete);
            };
            evidence.prove(found);
        }
    }
    if evidence.conflicting {
        Ok(SavedTarget::Incomplete)
    } else {
        Ok(evidence
            .target()
            .map(SavedTarget::Proven)
            .unwrap_or(SavedTarget::Absent))
    }
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
    async fn deleted_saved_handoff_recovers_native_sibling_from_retained_event_and_persists_parent()
    {
        let (state, _, _server, mut bot) = fixture("instagram").await;
        stop_for_erasure(&state, &mut bot).await;
        let native_bot = uuid::Uuid::new_v4().to_string();
        let channel = uuid::Uuid::new_v4().to_string();
        let mut native_parent = parent(&bot.user_id, &native_bot, &channel, "discord");
        native_parent.insert("bot_owner_id", uuid::Uuid::new_v4().to_string());
        state
            .db
            .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
            .insert_one(native_parent)
            .await
            .unwrap();
        let event = uuid::Uuid::new_v4().to_string();
        let conversation = uuid::Uuid::new_v4().to_string();
        let turn = uuid::Uuid::new_v4().to_string();
        let origin = ChannelOrigin {
            org_execution: None,
            native_event_id: Some(event.clone()),
            thread: None,
            nyxbot_channel_id: channel.clone(),
            partition: "private-native".into(),
            platform: "discord".into(),
        };
        let key = channel_supplemental_delivery::saved_key(&turn, &origin);
        let receipt = channel_text_delivery::begin(
            &state.db,
            &key,
            &bot.user_id,
            &event,
            "native-receipt",
            1,
        )
        .await
        .unwrap();
        let now = bson::DateTime::now();
        state.db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME).insert_one(doc! {
            "_id":&conversation,"user_id":&bot.user_id,"title":"Personal saved answer","model":"test",
            "credential_api_key_id":"","message_count":1_i64,"created_at":now,"updated_at":now}).await.unwrap();
        state.db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME).insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"user_id":&bot.user_id,"conversation_id":&conversation,
            "turn_id":&turn,"role":"assistant","handoff_delivery":{"targets":[saved(&turn,&origin)]}}).await.unwrap();
        state.db.collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME).insert_one(doc! {
            "_id":&event,"user_id":&bot.user_id,"channel_id":&channel,"partition":&origin.partition,"event_id":"native-source",
            "status":"completed","conversation_id":&conversation,"turn_id":&turn,"created_at":now,"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::days(30)),
            "delivery":{"version":1,"state":"sent","origin":bson::to_bson(&origin).unwrap(),"sender_id":"native-sender",
                "guest":false,"addressed":true,"transport":"gateway","stream_deadline":now,"checked_at":now}}).await.unwrap();
        super::super::assistant_nyxagent::delete(&state.db, &bot.user_id, &conversation)
            .await
            .unwrap();
        assert!(
            state
                .db
                .collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
                .find_one(doc! {"turn_id":&turn})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            state
                .db
                .collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
                .find_one(doc! {"_id":&event})
                .await
                .unwrap()
                .is_some()
        );
        let unknown_source = uuid::Uuid::new_v4().to_string();
        let unknown_key = format!("saved-result:{}:{}", uuid::Uuid::new_v4(), "a".repeat(64));
        channel_text_delivery::begin(
            &state.db,
            &unknown_key,
            &bot.user_id,
            &unknown_source,
            "unresolved",
            1,
        )
        .await
        .unwrap();
        assert!(matches!(
            erase_legacy(&state.db, &bot).await,
            Err(AppError::Conflict(_))
        ));
        let plans = state
            .db
            .collection::<Document>(crate::models::channel_text_delivery::COLLECTION_NAME);
        assert!(
            plans
                .find_one(doc! {"_id":&receipt.id,"channel_bot_id":&native_bot})
                .await
                .unwrap()
                .is_some()
        );
        // The original native source can now expire without losing sibling proof.
        state
            .db
            .collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
            .delete_one(doc! {"_id":event})
            .await
            .unwrap();
        state
            .db
            .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(doc! {
            "_id":unknown_source,"user_id":&bot.user_id,"channel_bot_id":&bot.id})
            .await
            .unwrap();
        erase_legacy(&state.db, &bot).await.unwrap();
        assert!(
            plans
                .find_one(doc! {"_id":&receipt.id,"channel_bot_id":native_bot})
                .await
                .unwrap()
                .is_some()
        );
    }

    fn query_counter(stats: &Document, name: &str) -> i64 {
        match stats.get(name).unwrap() {
            bson::Bson::Int32(value) => i64::from(*value),
            bson::Bson::Int64(value) => *value,
            _ => panic!("unexpected query-plan counter"),
        }
    }

    #[tokio::test]
    async fn legacy_erasure_uses_bounded_source_batches_and_parented_plans_need_no_source_scan() {
        let (state, _, _server, bot) = fixture("instagram").await;
        channel_text_delivery::ensure_indexes(&state.db)
            .await
            .unwrap();
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
        let empty = state
            .db
            .run_command(doc! {
                "explain": {"find": crate::models::channel_text_delivery::COLLECTION_NAME,
                    "filter": {"user_id": {"$in": [&bot.user_id]}, "channel_bot_id": null},
                    "sort": {"_id": 1}, "projection": attribution_fields(), "limit": 100},
                "verbosity": "executionStats",
            })
            .await
            .unwrap();
        let stats = empty.get_document("executionStats").unwrap();
        assert_eq!(query_counter(stats, "nReturned"), 0);
        assert_eq!(query_counter(stats, "totalDocsExamined"), 0);
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
            // Required timestamps do not guarantee chronological ordering in
            // retained/recovered rows. Exact source proof still requires erasure.
            if index % 3 == 0 {
                historical.insert(
                    "created_at",
                    bson::DateTime::from_chrono(bot.created_at - chrono::Duration::days(60)),
                );
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
        // Match production's unhinted first and continuation pages, including
        // its attribution-only projection. A forced hint would hide bad plans.
        let mut after = None;
        for page in 0..2 {
            let mut filter = doc! {"user_id": {"$in": [&bot.user_id]}, "channel_bot_id": null};
            if let Some(id) = &after {
                filter.insert("_id", doc! {"$gt": id});
            }
            let explanation = state
                .db
                .run_command(doc! {
                    "explain": {"find": crate::models::channel_text_delivery::COLLECTION_NAME,
                        "filter": filter.clone(), "sort": {"_id": 1},
                        "projection": attribution_fields(), "limit": 100},
                    "verbosity": "executionStats",
                })
                .await
                .unwrap();
            let stats = explanation.get_document("executionStats").unwrap();
            let metric = |name| query_counter(stats, name);
            assert_eq!(metric("nReturned"), 100);
            assert!(metric("totalDocsExamined") <= 100, "page {page}: {stats:?}");
            assert!(metric("totalKeysExamined") <= 101, "page {page}: {stats:?}");
            // An equally bounded _id plan is valid when every visited row
            // matches. Verify observed work, not one optimizer's plan name.
            eprintln!(
                "unhinted legacy attribution page {page}: 100 returned, {} documents and {} index keys examined",
                metric("totalDocsExamined"),
                metric("totalKeysExamined")
            );
            let actual: Vec<Document> = plans
                .find(filter)
                .projection(attribution_fields())
                .sort(doc! {"_id": 1})
                .limit(100)
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            after = Some(actual.last().unwrap().get_str("_id").unwrap().to_owned());
        }
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

    #[tokio::test]
    async fn exact_predating_destination_with_missing_or_mismatched_nested_turn_stays_pending() {
        for fault in [
            "nested-missing",
            "nested-different",
            "platform-missing",
            "platform-empty",
            "platform-unknown",
            "thread-provenance",
        ] {
            let (state, _, _server, mut bot) = fixture("instagram").await;
            stop_for_erasure(&state, &mut bot).await;
            let channel = uuid::Uuid::new_v4().to_string();
            state
                .db
                .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
                .insert_one(parent(&bot.user_id, &bot.id, &channel, "instagram"))
                .await
                .unwrap();
            let origin = ChannelOrigin {
                org_execution: None,
                native_event_id: None,
                thread: (fault == "thread-provenance").then(|| {
                    Box::new(crate::models::channel_thread_follow::ThreadTurnBinding {
                        sender_kind: None,
                        child_id: uuid::Uuid::new_v4().to_string(),
                        source_message_id: "original".into(),
                        sender_id: "sender".into(),
                        guest: true,
                        revision: 1,
                        generation: 1,
                        channel_generation: 1,
                        agent_id: uuid::Uuid::new_v4().to_string(),
                        queued: false,
                    })
                }),
                nyxbot_channel_id: channel,
                partition: "private-owner".into(),
                platform: "instagram".into(),
            };
            let turn = uuid::Uuid::new_v4().to_string();
            let key = channel_supplemental_delivery::saved_key(&turn, &origin);
            let receipt = channel_text_delivery::begin(
                &state.db,
                &key,
                &bot.user_id,
                "expired-source",
                "historical",
                1,
            )
            .await
            .unwrap();
            state.db.collection::<Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
                .update_one(doc! {"_id":&receipt.id},doc! {"$set":{"created_at":bson::DateTime::from_chrono(bot.created_at-chrono::Duration::days(1)),
                    "parts.0.platform_message_id":"retained-provider-id"}}).await.unwrap();
            let mut target = saved(&turn, &origin);
            match fault {
                "nested-missing" => {
                    target.get_document_mut("source").unwrap().remove("turn_id");
                }
                "nested-different" => {
                    target
                        .get_document_mut("source")
                        .unwrap()
                        .insert("turn_id", "different");
                }
                "platform-missing" => {
                    target
                        .get_document_mut("origin")
                        .unwrap()
                        .remove("platform");
                }
                "platform-empty" => {
                    target
                        .get_document_mut("origin")
                        .unwrap()
                        .insert("platform", "");
                }
                "platform-unknown" => {
                    target
                        .get_document_mut("origin")
                        .unwrap()
                        .insert("platform", "unknown-platform");
                }
                _ => {
                    target
                        .get_document_mut("origin")
                        .unwrap()
                        .get_document_mut("thread")
                        .unwrap()
                        .remove("sender_id");
                }
            }
            let message = uuid::Uuid::new_v4().to_string();
            state
                .db
                .collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
                .insert_one(
                    doc! {"_id":&message,"user_id":&bot.user_id,"turn_id":&turn,"role":"assistant",
                    "handoff_delivery":{"targets":[target]}},
                )
                .await
                .unwrap();
            let grant = &bot.meta_binding.as_ref().unwrap().grant_id;
            assert!(matches!(
                super::super::personal_erasure_service::erase_provider_inbox(
                    &state, &bot.id, grant
                )
                .await,
                Err(AppError::Conflict(_))
            ));
            assert!(
                channel_text_delivery::load(&state.db, &key)
                    .await
                    .unwrap()
                    .is_some()
            );
            assert!(
                state
                    .db
                    .collection::<Document>("channel_meta_erasure_fences")
                    .find_one(doc! {"_id":&bot.id,"local_data_erased_at":{"$exists":true}})
                    .await
                    .unwrap()
                    .is_none()
            );
            state
                .db
                .collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&message},
                    doc! {"$set":{"handoff_delivery.targets.0":saved(&turn,&origin)}},
                )
                .await
                .unwrap();
            super::super::personal_erasure_service::erase_provider_inbox(&state, &bot.id, grant)
                .await
                .unwrap();
            assert!(
                channel_text_delivery::load(&state.db, &key)
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }

    async fn stop_for_erasure(state: &crate::AppState, bot: &mut ChannelBot) {
        bot.is_active = false;
        bot.meta_binding.as_mut().unwrap().subscription = "removed".into();
        state
            .db
            .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .replace_one(doc! {"_id": &bot.id}, &*bot)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn predating_canonical_handoff_is_resolved_before_excluding_unproven_old_history() {
        let (state, _, _server, mut bot) = fixture("instagram").await;
        stop_for_erasure(&state, &mut bot).await;
        let migrations = state.db.collection::<Document>(
            crate::services::assistant_access_policy::MIGRATIONS_COLLECTION_NAME,
        );
        migrations.insert_many([
            doc! {"_id":"own-defaults","migration":"issue-2482","channel_bot_id":&bot.id,
                "org_id":&bot.user_id,"channel_id":"retired-channel","binding_revision":"own-binding"},
            doc! {"_id":"sibling-defaults","migration":"issue-2482","channel_bot_id":"retained-sibling",
                "org_id":&bot.user_id,"channel_id":"sibling-channel","binding_revision":"sibling-binding"},
        ]).await.unwrap();
        let channel = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
            .insert_one(parent(&bot.user_id, &bot.id, &channel, "instagram"))
            .await
            .unwrap();
        let origin = ChannelOrigin {
            org_execution: None,
            native_event_id: None,
            thread: None,
            nyxbot_channel_id: channel,
            partition: "private-owner".into(),
            platform: "instagram".into(),
        };
        let turn = uuid::Uuid::new_v4().to_string();
        let known_key = channel_supplemental_delivery::saved_key(&turn, &origin);
        let unknown_key =
            channel_supplemental_delivery::saved_key(&uuid::Uuid::new_v4().to_string(), &origin);
        for key in [&known_key, &unknown_key] {
            let plan = channel_text_delivery::begin(
                &state.db,
                key,
                &bot.user_id,
                &uuid::Uuid::new_v4().to_string(),
                "retained-provider-id",
                1,
            )
            .await
            .unwrap();
            state.db.collection::<Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
                .update_one(doc! {"_id": &plan.id}, doc! {"$set": {
                    "created_at": bson::DateTime::from_chrono(bot.created_at - chrono::Duration::days(1)),
                    "parts.0.platform_message_id": "historical-provider-id"}}).await.unwrap();
        }
        state.db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
            .insert_one(doc! {"_id": uuid::Uuid::new_v4().to_string(), "user_id": &bot.user_id,
                "turn_id": &turn, "role": "assistant", "handoff_delivery": {"targets": [saved(&turn, &origin)]}})
            .await.unwrap();
        super::super::personal_erasure_service::erase_provider_inbox(
            &state,
            &bot.id,
            &bot.meta_binding.as_ref().unwrap().grant_id,
        )
        .await
        .unwrap();
        assert!(
            channel_text_delivery::load(&state.db, &known_key)
                .await
                .unwrap()
                .is_none()
        );
        // No exact join or handoff ties this older plan to the deleted inbox.
        assert!(
            channel_text_delivery::load(&state.db, &unknown_key)
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
                .is_some()
        );
        assert!(
            migrations
                .find_one(doc! {"_id":"own-defaults"})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            migrations
                .find_one(doc! {"_id":"sibling-defaults"})
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn predating_exact_handoff_with_contradictory_owner_stays_pending_until_verified_repair()
    {
        let (state, _, _server, mut bot) = fixture("instagram").await;
        stop_for_erasure(&state, &mut bot).await;
        let channel = uuid::Uuid::new_v4().to_string();
        let mut metadata = parent(&bot.user_id, &bot.id, &channel, "instagram");
        metadata.insert("user_id", "contradictory-conversation-owner");
        state
            .db
            .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
            .insert_one(metadata)
            .await
            .unwrap();
        let origin = ChannelOrigin {
            org_execution: None,
            native_event_id: None,
            thread: None,
            nyxbot_channel_id: channel.clone(),
            partition: "private-owner".into(),
            platform: "instagram".into(),
        };
        let turn = uuid::Uuid::new_v4().to_string();
        let key = channel_supplemental_delivery::saved_key(&turn, &origin);
        let plan = channel_text_delivery::begin(
            &state.db,
            &key,
            &bot.user_id,
            "expired-contradictory-source",
            "retained-provider-id",
            1,
        )
        .await
        .unwrap();
        state.db.collection::<Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
            .update_one(doc! {"_id":&plan.id}, doc! {"$set":{
                "created_at":bson::DateTime::from_chrono(bot.created_at - chrono::Duration::days(1)),
                "parts.0.platform_message_id":"historical-provider-id"}}).await.unwrap();
        state.db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
            .insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&bot.user_id,
                "turn_id":&turn,"role":"assistant","handoff_delivery":{"targets":[saved(&turn,&origin)]}})
            .await.unwrap();
        let grant = &bot.meta_binding.as_ref().unwrap().grant_id;
        let result =
            super::super::personal_erasure_service::erase_provider_inbox(&state, &bot.id, grant)
                .await;
        assert!(matches!(result, Err(AppError::Conflict(_))));
        assert!(
            channel_text_delivery::load(&state.db, &key)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            state
                .db
                .collection::<Document>("channel_meta_erasure_fences")
                .find_one(doc! {"_id":&bot.id,"local_data_erased_at":{"$exists":true}})
                .await
                .unwrap()
                .is_none()
        );
        // Operational repair restores the independently verified parent owner.
        state
            .db
            .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&channel},
                doc! {"$set":{"user_id":&bot.user_id}},
            )
            .await
            .unwrap();
        super::super::personal_erasure_service::erase_provider_inbox(&state, &bot.id, grant)
            .await
            .unwrap();
        assert!(
            channel_text_delivery::load(&state.db, &key)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn agreeing_ownerless_sibling_and_saved_parent_allow_other_platform_inbox_erasure() {
        let (state, _, _server, mut bot) = fixture("messenger").await;
        stop_for_erasure(&state, &mut bot).await;
        let sibling = uuid::Uuid::new_v4().to_string();
        let channel = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
            .insert_one(parent(&bot.user_id, &sibling, &channel, "instagram"))
            .await
            .unwrap();
        let origin = ChannelOrigin {
            org_execution: None,
            native_event_id: None,
            thread: None,
            nyxbot_channel_id: channel,
            partition: "private-owner".into(),
            platform: "instagram".into(),
        };
        let turn = uuid::Uuid::new_v4().to_string();
        let key = channel_supplemental_delivery::saved_key(&turn, &origin);
        let saved_plan = channel_text_delivery::begin(
            &state.db,
            &key,
            &bot.user_id,
            "ownerless-sibling-source",
            "retained",
            1,
        )
        .await
        .unwrap();
        state
            .db
            .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(doc! {"_id": &saved_plan.source_id, "channel_bot_id": &sibling})
            .await
            .unwrap();
        state.db.collection::<Document>(crate::models::assistant_message::COLLECTION_NAME)
            .insert_one(doc! {"_id": uuid::Uuid::new_v4().to_string(), "user_id": &bot.user_id,
            "turn_id": &turn, "role": "assistant", "handoff_delivery": {"targets": [saved(&turn, &origin)]}}).await.unwrap();
        super::super::personal_erasure_service::erase_provider_inbox(
            &state,
            &bot.id,
            &bot.meta_binding.as_ref().unwrap().grant_id,
        )
        .await
        .unwrap();
        assert!(
            state
                .db
                .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
                .find_one(doc! {"_id": &bot.id})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            channel_text_delivery::load(&state.db, &key)
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            state
                .db
                .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
                .find_one(doc! {"_id": &saved_plan.source_id, "channel_bot_id": &sibling})
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn contradictory_source_and_receipt_parents_in_both_owner_arrangements_keep_erasure_pending()
     {
        for ownerless_receipt in [false, true] {
            let (state, _, _server, mut bot) = fixture("instagram").await;
            stop_for_erasure(&state, &mut bot).await;
            let source = uuid::Uuid::new_v4().to_string();
            let legacy = plan(&bot.user_id, &source, None);
            state
                .db
                .collection::<Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
                .insert_one(&legacy)
                .await
                .unwrap();
            let mut message = doc! {"_id": &source, "channel_bot_id": &bot.id};
            let mut receipt = doc! {"_id": uuid::Uuid::new_v4().to_string(), "inbound_message_id": &source,
            "channel_bot_id": uuid::Uuid::new_v4().to_string()};
            if ownerless_receipt {
                message.insert("user_id", &bot.user_id);
            } else {
                receipt.insert("user_id", &bot.user_id);
            }
            state
                .db
                .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
                .insert_one(message)
                .await
                .unwrap();
            let receipts = state
                .db
                .collection::<Document>(crate::models::channel_reply_delivery::COLLECTION_NAME);
            receipts.insert_one(&receipt).await.unwrap();
            let grant = bot.meta_binding.as_ref().unwrap().grant_id.as_str();
            let result = super::super::personal_erasure_service::erase_provider_inbox(
                &state, &bot.id, grant,
            )
            .await;
            assert!(matches!(result, Err(AppError::Conflict(_))));
            assert!(
                channel_text_delivery::load(&state.db, legacy.get_str("logical_key").unwrap())
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
            assert!(
                receipts
                    .find_one(doc! {"_id": receipt.get_str("_id").unwrap()})
                    .await
                    .unwrap()
                    .is_some()
            );
            // Correcting independently verified parent evidence allows the same
            // pending obligation to retry without discarding the ownerless row.
            receipts
                .update_one(
                    doc! {"_id": receipt.get_str("_id").unwrap()},
                    doc! {"$set": {"channel_bot_id": &bot.id}},
                )
                .await
                .unwrap();
            super::super::personal_erasure_service::erase_provider_inbox(&state, &bot.id, grant)
                .await
                .unwrap();
            assert!(
                channel_text_delivery::load(&state.db, legacy.get_str("logical_key").unwrap())
                    .await
                    .unwrap()
                    .is_none()
            );
        }
    }
}
