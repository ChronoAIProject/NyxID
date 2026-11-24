//! Indexed, restartable cleanup of one consent in a shared public inbox.
//! Cursors live on the grant scope. Sources remain until their dependants have
//! durable provenance; a successor's parent, credentials and pointers survive.
use bson::{Bson, Document, doc};
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};

use super::{BOTS, MetaService, grants, scopes};
use crate::{
    errors::AppResult,
    models::{
        channel_bot::ChannelBot,
        channel_meta::{GRANT_SCOPES, GrantRef},
    },
};

const PAGE: i64 = 100;
const MESSAGES: &str = crate::models::channel_message::COLLECTION_NAME;
const REPLIES: &str = crate::models::channel_reply_delivery::COLLECTION_NAME;
const PLANS: &str = crate::models::channel_text_delivery::COLLECTION_NAME;
const EVENTS: &str = crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME;
const THREADS: &str = crate::models::nyxbot_channel::THREADS_COLLECTION_NAME;
const CHANNELS: &str = crate::models::nyxbot_channel::COLLECTION_NAME;

/// Retained assistant provenance requires exact turn cleanup even when there
/// has only ever been one consent. An inbox with no scoped assistant history
/// retains the existing whole-inbox path.
pub(crate) async fn required(db: &Database, bot: &str, grant: &str) -> AppResult<bool> {
    if scopes::has_history(db, bot).await?
        || db
            .collection::<Document>(GRANT_SCOPES)
            .find_one(doc! {"bot_id":bot,"grant_id":grant,"purge":{"$exists":true}})
            .projection(doc! {"_id":1})
            .await?
            .is_some()
    {
        return Ok(true);
    }
    for (collection, filter) in [
        (
            "assistant_messages",
            doc! {"meta_grants":{"$elemMatch":{"bot_id":bot,"grant_id":grant}}},
        ),
        (
            "assistant_conversations",
            doc! {"meta_grant.bot_id":bot,"meta_grant.grant_id":grant},
        ),
        ("assistant_conversations", queued_filter(bot, grant)),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one(filter)
            .projection(doc! {"_id":1})
            .await?
            .is_some()
        {
            return Ok(true);
        }
    }
    Ok(false)
}

fn queued_filter(bot: &str, grant: &str) -> Document {
    let context = doc! {"meta_grant.bot_id":bot,"meta_grant.grant_id":grant};
    doc! {"$or":[
        {"pending_events":{"$elemMatch":{"channel_contexts":{"$elemMatch":&context}}}},
        {"active_turn.channel_context.meta_grant.bot_id":bot,"active_turn.channel_context.meta_grant.grant_id":grant},
        {"active_turn.events":{"$elemMatch":{"channel_contexts":{"$elemMatch":&context}}}},
    ]}
}

enum Authority {
    Signed(bson::DateTime),
    Manual(Document),
}

fn exact(scope: &Document) -> AppResult<Document> {
    Ok(
        doc! {"meta_grant.bot_id":scope.get_str("bot_id").map_err(|_|grants::incomplete())?,
        "meta_grant.grant_id":scope.get_str("grant_id").map_err(|_|grants::incomplete())?},
    )
}

async fn eligible(
    db: &Database,
    tx: &mut ClientSession,
    bot: &str,
    grant: &str,
    authority: &Authority,
) -> AppResult<Option<Document>> {
    let row = db
        .collection::<Document>(GRANT_SCOPES)
        .find_one(doc! {"bot_id":bot,"grant_id":grant})
        .session(&mut *tx)
        .await?
        .ok_or_else(grants::incomplete)?;
    let completed = grants::completed(&row, grant)?;
    if completed || row.get_str("status") == Ok("completed") {
        scopes::mark_complete(db, tx, bot, grant).await?;
        return Ok(None);
    }
    let row = scopes::synchronize_chronology(db, tx, &row).await?;
    if let Authority::Signed(cutoff) = authority {
        let authorized = *row
            .get_datetime("authorized_at")
            .map_err(|_| grants::incomplete())?;
        if authorized >= *cutoff {
            db.collection::<Document>(GRANT_SCOPES).update_one(doc! {"_id":row.get("_id").cloned(),
            "authorized_at":authorized,"erasure_cutoff":{"$type":"date","$lte":authorized}},
            doc! {"$set":{"erasure_requested":false},"$unset":{"erasure_cutoff":""},"$inc":{"revision":1_i64}})
            .session(tx).await?;
            return Ok(None);
        }
    } else if let Authority::Manual(obligation) = authority {
        let mut filter = doc! {"_id":bot,"is_active":false,"credential_source":obligation.get_str("credential_source").map_err(|_|grants::incomplete())?,
        "user_id":obligation.get_str("owner_id").map_err(|_|grants::incomplete())?,"meta_manual_purge":obligation.clone(),
        "meta_binding.generation":obligation.get_i64("generation").map_err(|_|grants::incomplete())?};
        filter.extend(grants::grant_filter(grant));
        if db
            .collection::<Document>(BOTS)
            .update_one(filter, doc! {"$inc":{"meta_observation_revision":1_i64}})
            .session(&mut *tx)
            .await?
            .matched_count
            != 1
        {
            return Err(grants::incomplete());
        }
    }
    if !matches!(row.get_str("status"), Ok("current" | "sealed" | "retired")) {
        return Err(grants::incomplete());
    }
    Ok(Some(row))
}

/// Return true only for exact completed proof or a verified cutoff exclusion.
/// An ordinary page returns false so the target retries promptly, independently
/// from its request's discovery and other children.
pub(crate) async fn pass(
    state: &crate::AppState,
    service: &MetaService<'_>,
    bot: &str,
    grant: &str,
    cutoff: bson::DateTime,
) -> AppResult<bool> {
    pass_authorized(state, service, bot, grant, &Authority::Signed(cutoff)).await
}

/// Explicit manual Delete is its own exact actor/owner/source/grant/generation
/// obligation. It never borrows a signed cutoff or another consent's scope.
pub(crate) async fn manual_pass(state: &crate::AppState, bot: &ChannelBot) -> AppResult<bool> {
    let binding = bot.meta_binding.as_ref().ok_or_else(grants::incomplete)?;
    if bot.is_active || bot.credential_source == "platform" {
        return Err(grants::incomplete());
    }
    let row = state
        .db
        .collection::<Document>(BOTS)
        .find_one(doc! {"_id":&bot.id,"user_id":&bot.user_id,
        "is_active":false,"meta_binding.generation":binding.generation})
        .await?
        .ok_or_else(grants::incomplete)?;
    let obligation = row
        .get_document("meta_manual_purge")
        .map_err(|_| grants::incomplete())?
        .clone();
    if obligation.get_str("owner_id") != Ok(bot.user_id.as_str())
        || obligation.get_str("credential_source") != Ok(bot.credential_source.as_str())
        || obligation.get_str("grant_id") != Ok(binding.grant_id.as_str())
        || obligation.get_i64("generation") != Ok(binding.generation)
        || obligation.get_str("actor_id").is_err()
        || obligation.get_datetime("requested_at").is_err()
    {
        return Err(grants::incomplete());
    }
    let authority = Authority::Manual(obligation);
    let service = MetaService::from(state);
    for _ in 0..8 {
        if pass_authorized(state, &service, &bot.id, &binding.grant_id, &authority).await? {
            return Ok(true);
        }
    }
    state.db.collection::<Document>(BOTS).update_one(doc! {"_id":&bot.id,"is_active":false,"meta_binding.grant_id":&binding.grant_id,
        "meta_binding.generation":binding.generation},doc! {"$set":{"meta_binding.next_reconcile_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::seconds(1))}}).await?;
    Ok(false)
}

async fn pass_authorized(
    state: &crate::AppState,
    service: &MetaService<'_>,
    bot: &str,
    grant: &str,
    authority: &Authority,
) -> AppResult<bool> {
    let claimed = crate::services::transaction_service::run_db_only(&state.db,
        &(&state.db,bot,grant,authority),|tx,&(db,bot,grant,authority)| {
        Box::pin(async move {
            let Some(scope)=eligible(db,tx,bot,grant,authority).await? else {return Ok(false)};
            if let Authority::Signed(cutoff)=authority
                && let Some(current)=db.collection::<ChannelBot>(BOTS).find_one(doc! {"_id":bot,"meta_binding.grant_id":grant})
                .session(&mut *tx).await?
                && !super::provider::apply_callback_state(db,tx,&current,true,cutoff).await? {return Ok(false)}
            let mut update=doc! {"$set":{"erasure_requested":true},"$inc":{"revision":1_i64}};
            if let Authority::Signed(cutoff)=authority {update.insert("$max",doc! {"erasure_cutoff":*cutoff});}
            let changed=db.collection::<Document>(GRANT_SCOPES).update_one(doc! {"_id":scope.get("_id").cloned(),
                "authorized_at":scope.get("authorized_at").cloned(),"status":{"$ne":"completed"}},
                update)
                .session(tx).await?;
            if changed.matched_count!=1 {return Err(grants::incomplete())}
            Ok(true)
        })
    }).await?;
    if !claimed {
        return Ok(true);
    }
    // Only the exact current grant owns remote subscription retirement. A's
    // deletion never borrows B's token or removes B's subscription.
    if let Some(current) = state
        .db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! {"_id":bot,"meta_binding.grant_id":grant})
        .await?
        && current
            .meta_binding
            .as_ref()
            .is_some_and(|binding| binding.subscription != "removed")
    {
        service.disconnect(&current).await?;
    }
    let stored = state
        .db
        .collection::<Document>(GRANT_SCOPES)
        .find_one(doc! {"bot_id":bot,"grant_id":grant})
        .await?
        .ok_or_else(grants::incomplete)?;
    if stored.get_bool("legacy_history") == Ok(true) && !stored.contains_key("purge")
        || stored
            .get_document("purge")
            .is_ok_and(|progress| progress.get_i32("phase") == Ok(-1))
    {
        let after = stored
            .get_document("purge")
            .ok()
            .and_then(|progress| progress.get_str("after").ok());
        let next = crate::services::channel_meta_journal_erasure::attribute_scope_page(
            &state.db,
            &stored,
            after,
            &state.token_exchange_cache,
        )
        .await?;
        crate::services::transaction_service::run_db_only(
            &state.db,
            &(&state.db, bot, grant, authority, &stored, &next),
            |tx, &(db, bot, grant, authority, observed, next)| {
                Box::pin(async move {
                    let Some(current) = eligible(db, tx, bot, grant, authority).await? else {
                        return Ok(());
                    };
                    let progress = next
                        .as_ref()
                        .map_or_else(|| doc! {"phase":0}, |after| doc! {"phase":-1,"after":after});
                    if db
                        .collection::<Document>(GRANT_SCOPES)
                        .update_one(
                            doc! {"_id":current.get("_id").cloned(),
                            "purge":observed.get("purge").cloned().unwrap_or(Bson::Null)},
                            doc! {"$set":{"purge":progress},"$inc":{"revision":1_i64}},
                        )
                        .session(tx)
                        .await?
                        .matched_count
                        != 1
                    {
                        return Err(grants::incomplete());
                    }
                    Ok(())
                })
            },
        )
        .await?;
        return Ok(false);
    }
    #[cfg(test)]
    crate::services::durable_wake_tests::after("meta_scoped_purge_claimed").await;
    let (done,repair)=crate::services::transaction_service::run_db_only(&state.db,
        &(&state.db,bot,grant,authority),|tx,&(db,bot,grant,authority)| {
        Box::pin(async move {
            let Some(scope)=eligible(db,tx,bot,grant,authority).await? else {return Ok((true,false))};
            let progress=scope.get_document("purge").cloned().unwrap_or_default();
            let phase=progress.get_i32("phase").unwrap_or(0);
            let result=step(db,tx,&scope,&progress,phase).await?;
            if result.done {return Ok((true,false))}
            let changed=db.collection::<Document>(GRANT_SCOPES).update_one(
                doc! {"_id":scope.get("_id").cloned(),"erasure_requested":true,"status":{"$ne":"completed"}},
                doc! {"$set":{"purge":result.progress},"$inc":{"revision":1_i64}}).session(tx).await?;
            if changed.matched_count!=1 {return Err(grants::incomplete())}
            Ok((false,result.repair))
        })
    }).await?;
    if repair {
        return Err(grants::incomplete());
    }
    Ok(done)
}

struct Step {
    progress: Document,
    done: bool,
    repair: bool,
}
impl Step {
    fn stay(progress: Document) -> Self {
        Self {
            progress,
            done: false,
            repair: false,
        }
    }
    fn next(phase: i32) -> Self {
        Self::stay(doc! {"phase":phase+1})
    }
}

async fn page(
    db: &Database,
    tx: &mut ClientSession,
    collection: &str,
    mut filter: Document,
    progress: &Document,
) -> AppResult<Vec<Document>> {
    if let Ok(after) = progress.get_str("after") {
        filter.insert("_id", doc! {"$gt":after});
    }
    let mut cursor = db
        .collection::<Document>(collection)
        .find(filter)
        .sort(doc! {"_id":1})
        .limit(PAGE)
        .session(&mut *tx)
        .await?;
    Ok(cursor.stream(tx).try_collect().await?)
}

fn advance(
    progress: &Document,
    phase: i32,
    rows: &[Document],
    unresolved: bool,
) -> AppResult<Step> {
    let mut progress = progress.clone();
    let unresolved = unresolved || progress.get_bool("unresolved") == Ok(true);
    if rows.len() < PAGE as usize {
        if unresolved {
            return Ok(Step {
                progress: doc! {"phase":phase},
                done: false,
                repair: true,
            });
        }
        return Ok(Step::next(phase));
    }
    progress.insert("phase", phase);
    progress.insert(
        "after",
        rows.last()
            .and_then(|row| row.get_str("_id").ok())
            .ok_or_else(grants::incomplete)?,
    );
    progress.insert("unresolved", unresolved);
    Ok(Step::stay(progress))
}

async fn source_grant(
    db: &Database,
    tx: &mut ClientSession,
    row: &Document,
    field: Option<&str>,
) -> AppResult<Option<GrantRef>> {
    let stamped = match row.get("meta_grant") {
        None | Some(Bson::Null) => None,
        Some(value) => {
            Some(bson::from_bson::<GrantRef>(value.clone()).map_err(|_| grants::incomplete())?)
        }
    };
    if let Some(stamped) = &stamped
        && (stamped.bot_id.is_empty()
            || stamped.generation < 0
            || row
                .get_str("channel_bot_id")
                .is_ok_and(|parent| parent != stamped.bot_id))
    {
        return Err(grants::incomplete());
    }
    if let Some(stamped) = &stamped {
        validate_reference(db, tx, stamped).await?;
    }
    if let Some(field) = field {
        let source = match row.get(field) {
            None | Some(Bson::Null) => return stamped.map(Some).ok_or_else(grants::incomplete),
            Some(Bson::String(source)) if !source.is_empty() => source,
            _ => return Err(grants::incomplete()),
        };
        let source = db
            .collection::<Document>(MESSAGES)
            .find_one(doc! {"_id":source})
            .session(&mut *tx)
            .await?;
        let Some(source) = source else {
            return stamped.map(Some).ok_or_else(grants::incomplete);
        };
        if row.get_str("user_id") != source.get_str("user_id") {
            // Native journals can belong to the linking administrator while
            // their relay source belongs to the organization. Require the
            // original exact route/key binding for that distinct owner.
            let binding=db.collection::<Document>(CHANNELS).find_one(doc! {
                "channel_bot_id":source.get_str("channel_bot_id").map_err(|_|grants::incomplete())?,
                "user_id":row.get_str("user_id").map_err(|_|grants::incomplete())?,
                "bot_owner_id":source.get_str("user_id").map_err(|_|grants::incomplete())?,
                "route_id":source.get_str("conversation_id").map_err(|_|grants::incomplete())?,
                "route_api_key_id":source.get_str("agent_api_key_id").map_err(|_|grants::incomplete())?,
            }).projection(doc! {"_id":1}).session(&mut *tx).await?;
            if binding.is_none() {
                return Err(grants::incomplete());
            }
        }
        let proven = scopes::source_document_in_session(db, tx, &source).await?;
        if stamped
            .as_ref()
            .is_some_and(|stamped| proven.as_ref() != Some(stamped))
        {
            return Err(grants::incomplete());
        }
        Ok(proven)
    } else {
        scopes::source_document_in_session(db, tx, row).await
    }
}

/// Original tags survive finite-lived joins, but their generation must still
/// belong to the independently retained installation. A typed tag alone cannot
/// extend a consent's range after its source has expired.
async fn validate_reference(
    db: &Database,
    tx: &mut ClientSession,
    proof: &GrantRef,
) -> AppResult<()> {
    let scope = db
        .collection::<Document>(GRANT_SCOPES)
        .find_one(doc! {"bot_id":&proof.bot_id,"grant_id":&proof.grant_id})
        .session(&mut *tx)
        .await?
        .ok_or_else(grants::incomplete)?;
    reference_in_range(proof, &scope)
}

fn reference_in_range(proof: &GrantRef, scope: &Document) -> AppResult<()> {
    let from = scope
        .get_i64("from_generation")
        .map_err(|_| grants::incomplete())?;
    let through = match scope.get("to_generation") {
        None | Some(Bson::Null) => None,
        Some(Bson::Int64(value)) => Some(*value),
        _ => return Err(grants::incomplete()),
    };
    if proof.bot_id.is_empty()
        || scope.get_str("bot_id") != Ok(proof.bot_id.as_str())
        || scope.get_str("grant_id") != Ok(proof.grant_id.as_str())
        || from < 0
        || proof.generation < from
        || through.is_some_and(|through| through < from || proof.generation > through)
        || !matches!(
            scope.get_str("status"),
            Ok("current" | "sealed" | "retired")
        )
    {
        return Err(grants::incomplete());
    }
    Ok(())
}

async fn attribute(
    db: &Database,
    tx: &mut ClientSession,
    scope: &Document,
    progress: &Document,
    phase: i32,
) -> AppResult<Step> {
    let bot = scope.get_str("bot_id").map_err(|_| grants::incomplete())?;
    let (collection, field) = match phase {
        0 => (MESSAGES, None),
        1 => (REPLIES, Some("inbound_message_id")),
        _ => (PLANS, Some("source_id")),
    };
    let rows = page(db, tx, collection, doc! {"channel_bot_id":bot}, progress).await?;
    let mut unresolved = false;
    for row in &rows {
        let field = if phase == 0 && row.get_str("direction") == Ok("outbound") {
            Some("reply_to_message_id")
        } else {
            field
        };
        let proof = match source_grant(db, tx, row, field).await {
            Ok(Some(proof)) if proof.bot_id == bot => proof,
            Ok(_) | Err(crate::errors::AppError::Conflict(_)) => {
                unresolved = true;
                continue;
            }
            Err(error) => return Err(error),
        };
        if row
            .get("meta_grant")
            .is_some_and(|value| !matches!(value, Bson::Null))
        {
            continue;
        }
        let changed=db.collection::<Document>(collection).update_one(doc! {"_id":row.get("_id").cloned(),
            "channel_bot_id":bot,"meta_grant":null},doc! {"$set":{"meta_grant":bson::to_bson(&proof).map_err(|_|grants::incomplete())?}})
            .session(&mut *tx).await?;
        if changed.matched_count != 1 {
            return Err(grants::incomplete());
        }
    }
    advance(progress, phase, &rows, unresolved)
}

/// Historical events have a channel parent. Walk that indexed relationship,
/// retaining only channel/record cursors, never copied participant/source rows.
async fn attribute_events(
    db: &Database,
    tx: &mut ClientSession,
    scope: &Document,
    progress: &Document,
    phase: i32,
) -> AppResult<Step> {
    let bot = scope.get_str("bot_id").map_err(|_| grants::incomplete())?;
    let mut progress = progress.clone();
    let channel = if let Ok(channel) = progress.get_str("channel") {
        channel.to_owned()
    } else {
        let mut filter = doc! {"channel_bot_id":bot};
        if let Ok(after) = progress.get_str("channel_after") {
            filter.insert("_id", doc! {"$gt":after});
        }
        let Some(row) = db
            .collection::<Document>(CHANNELS)
            .find_one(filter)
            .sort(doc! {"_id":1})
            .session(&mut *tx)
            .await?
        else {
            return if progress.get_bool("unresolved") == Ok(true) {
                Ok(Step {
                    progress: doc! {"phase":phase},
                    done: false,
                    repair: true,
                })
            } else {
                Ok(Step::next(phase))
            };
        };
        row.get_str("_id")
            .map_err(|_| grants::incomplete())?
            .to_owned()
    };
    let rows = page(db, tx, EVENTS, doc! {"channel_id":&channel}, &progress).await?;
    let mut unresolved = progress.get_bool("unresolved") == Ok(true);
    for row in &rows {
        let proof = match source_grant(db, tx, row, Some("relay_message_id")).await {
            Ok(Some(proof)) if proof.bot_id == bot => proof,
            Ok(_) | Err(crate::errors::AppError::Conflict(_)) => {
                unresolved = true;
                continue;
            }
            Err(error) => return Err(error),
        };
        if row
            .get("meta_grant")
            .is_some_and(|value| !matches!(value, Bson::Null))
        {
            continue;
        }
        db.collection::<Document>(EVENTS).update_one(doc! {"_id":row.get("_id").cloned(),"channel_id":&channel,"meta_grant":null},
            doc! {"$set":{"channel_bot_id":bot,"meta_grant":bson::to_bson(&proof).map_err(|_|grants::incomplete())?}}).session(&mut *tx).await?;
    }
    progress.insert("phase", phase);
    progress.insert("unresolved", unresolved);
    if rows.len() < PAGE as usize {
        progress.remove("channel");
        progress.remove("after");
        progress.insert("channel_after", channel);
    } else {
        progress.insert("channel", channel);
        progress.insert(
            "after",
            rows.last()
                .and_then(|row| row.get_str("_id").ok())
                .ok_or_else(grants::incomplete)?,
        );
    }
    Ok(Step::stay(progress))
}

async fn erase_page(
    db: &Database,
    tx: &mut ClientSession,
    scope: &Document,
    collection: &str,
    filter: Document,
    phase: i32,
) -> AppResult<Step> {
    let rows = page(db, tx, collection, filter.clone(), &Document::new()).await?;
    if rows.is_empty() {
        return Ok(Step::next(phase));
    }
    let bot = scope.get_str("bot_id").map_err(|_| grants::incomplete())?;
    let grant = scope
        .get_str("grant_id")
        .map_err(|_| grants::incomplete())?;
    for row in &rows {
        if matches!(collection, MESSAGES | REPLIES | PLANS) {
            if row.get_str("channel_bot_id") != Ok(bot) {
                return Err(grants::incomplete());
            }
            let field = match collection {
                REPLIES => Some("inbound_message_id"),
                PLANS => Some("source_id"),
                _ if row.get_str("direction") == Ok("outbound") => Some("reply_to_message_id"),
                _ => None,
            };
            let proof = source_grant(db, tx, row, field)
                .await?
                .ok_or_else(grants::incomplete)?;
            if proof.bot_id != bot || proof.grant_id != grant {
                return Err(grants::incomplete());
            }
        } else if matches!(
            collection,
            EVENTS
                | crate::models::org_channel::LINKS
                | crate::models::org_channel::CHALLENGES
                | crate::models::org_channel::LANES
        ) {
            if original_parent(db, tx, row).await? != bot {
                return Err(grants::incomplete());
            }
            if collection == EVENTS {
                let proof = source_grant(db, tx, row, Some("relay_message_id"))
                    .await?
                    .ok_or_else(grants::incomplete)?;
                if proof.bot_id != bot || proof.grant_id != grant {
                    return Err(grants::incomplete());
                }
            }
        }
    }
    let ids: Vec<_> = rows
        .iter()
        .map(|row| row.get("_id").cloned().ok_or_else(grants::incomplete))
        .collect::<AppResult<_>>()?;
    db.collection::<Document>(collection)
        .delete_many(doc! {"$and":[filter,{"_id":{"$in":ids}}]})
        .session(&mut *tx)
        .await?;
    // A scope write in the same transaction serializes every original writer
    // and its exact late acknowledgement with this inventory page.
    Ok(Step::stay(doc! {"phase":phase}))
}

async fn original_parent(
    db: &Database,
    tx: &mut ClientSession,
    row: &Document,
) -> AppResult<String> {
    let declared = match row.get("channel_bot_id") {
        None | Some(Bson::Null) => None,
        Some(Bson::String(bot)) => Some(bot.as_str()),
        _ => return Err(grants::incomplete()),
    };
    let channel = row
        .get_str("channel_id")
        .map_err(|_| grants::incomplete())?;
    if let Some(parent) = db
        .collection::<Document>(CHANNELS)
        .find_one(doc! {"_id":channel})
        .projection(doc! {"channel_bot_id":1})
        .session(&mut *tx)
        .await?
    {
        let bot = parent
            .get_str("channel_bot_id")
            .map_err(|_| grants::incomplete())?;
        if declared.is_some_and(|declared| declared != bot) {
            return Err(grants::incomplete());
        }
        return Ok(bot.to_owned());
    }
    if let Some(declared) = declared {
        return Ok(declared.to_owned());
    }
    for field in [
        "relay_message_id",
        "last_message_id",
        "last_relay_message_id",
    ] {
        if let Ok(source) = row.get_str(field)
            && let Some(source) = db
                .collection::<Document>(MESSAGES)
                .find_one(doc! {"_id":source})
                .session(&mut *tx)
                .await?
        {
            return Ok(source
                .get_str("channel_bot_id")
                .map_err(|_| grants::incomplete())?
                .to_owned());
        }
    }
    Err(grants::incomplete())
}

async fn clear_threads(
    db: &Database,
    tx: &mut ClientSession,
    scope: &Document,
    phase: i32,
) -> AppResult<Step> {
    let filter = exact(scope)?;
    let rows = page(db, tx, THREADS, filter.clone(), &Document::new()).await?;
    if rows.is_empty() {
        return Ok(Step::next(phase));
    }
    for row in rows {
        if original_parent(db, tx, &row).await?
            != scope.get_str("bot_id").map_err(|_| grants::incomplete())?
        {
            return Err(grants::incomplete());
        }
        for field in [
            "last_message_id",
            "last_relay_message_id",
            "event_ref_relay_message_id",
        ] {
            if let Ok(source) = row.get_str(field)
                && let Some(source) = db
                    .collection::<Document>(MESSAGES)
                    .find_one(doc! {"_id":source})
                    .session(&mut *tx)
                    .await?
            {
                let proof = source_grant(
                    db,
                    tx,
                    &source,
                    if source.get_str("direction") == Ok("outbound") {
                        Some("reply_to_message_id")
                    } else {
                        None
                    },
                )
                .await?
                .ok_or_else(grants::incomplete)?;
                if proof.bot_id != scope.get_str("bot_id").map_err(|_| grants::incomplete())?
                    || proof.grant_id
                        != scope
                            .get_str("grant_id")
                            .map_err(|_| grants::incomplete())?
                {
                    return Err(grants::incomplete());
                }
            }
        }
        let mut conditional = exact(scope)?;
        conditional.insert("_id", row.get("_id").cloned());
        for field in [
            "last_message_id",
            "last_relay_message_id",
            "event_ref_relay_message_id",
        ] {
            conditional.insert(field, row.get(field).cloned().unwrap_or(Bson::Null));
        }
        db.collection::<Document>(THREADS).update_one(conditional,doc! {"$set":{"owner_seen":false,"owner_chat":false},
            "$unset":{"meta_grant":"","conversation_id":"","last_message_id":"","last_relay_message_id":"","last_message_at":"",
                "platform_chat_id":"","platform_thread_id":"","title":"","event_ref_ciphertext":"","event_ref_expires_at":"",
                "event_ref_relay_message_id":"","event_ref_relay_binding":""}}).session(&mut *tx).await?;
    }
    Ok(Step::stay(doc! {"phase":phase}))
}

async fn erase_guest(
    db: &Database,
    tx: &mut ClientSession,
    scope: &Document,
    progress: &Document,
    phase: i32,
) -> AppResult<Step> {
    let mut filter = exact(scope)?;
    filter.insert("guest_turn", true);
    let Some(row) = db
        .collection::<Document>("assistant_conversations")
        .find_one(filter)
        .session(&mut *tx)
        .await?
    else {
        return Ok(Step::next(phase));
    };
    let id = row.get_str("_id").map_err(|_| grants::incomplete())?;
    let owner = row.get_str("user_id").map_err(|_| grants::incomplete())?;
    if row
        .get("active_turn")
        .is_some_and(|value| !matches!(value, Bson::Null))
        || row
            .get_array("pending_events")
            .is_ok_and(|events| !events.is_empty())
    {
        return Err(grants::incomplete());
    }
    // The parent tag cannot certify a populated legacy conversation that was
    // adopted by a preceding writer. Prove every retained turn positively,
    // then its children, in restartable pages before using the normal cascade.
    let mut progress = if progress.get_str("guest") == Ok(id) {
        progress.clone()
    } else {
        doc! {"phase":phase,"guest":id}
    };
    let stage = progress.get_i32("guest_stage").unwrap_or(0);
    let collections = [
        "assistant_messages",
        crate::models::assistant_tool_result::COLLECTION_NAME,
        crate::models::assistant_attachment::COLLECTION_NAME,
        crate::models::assistant_resource::COLLECTION_NAME,
        crate::models::generated_artifact::COLLECTION_NAME,
        crate::models::async_service_operation::COLLECTION_NAME,
    ];
    if let Some(collection) = collections.get(stage as usize) {
        let rows = page(
            db,
            tx,
            collection,
            doc! {"conversation_id":id,"user_id":owner},
            &progress,
        )
        .await?;
        for record in &rows {
            let proof = if stage == 0 {
                validate_turn_events(
                    db,
                    tx,
                    id,
                    record
                        .get_str("turn_id")
                        .map_err(|_| grants::incomplete())?,
                    scope,
                )
                .await?;
                single_grant_message(record, scope)?
            } else {
                let turn = record
                    .get_str("turn_id")
                    .map_err(|_| grants::incomplete())?;
                let witness = db
                    .collection::<Document>("assistant_messages")
                    .find_one(doc! {"conversation_id":id,"user_id":owner,"turn_id":turn})
                    .session(&mut *tx)
                    .await?
                    .ok_or_else(grants::incomplete)?;
                if *collection == crate::models::async_service_operation::COLLECTION_NAME
                    && !matches!(record.get_str("state"), Ok("cancelled" | "delivered"))
                {
                    return Err(grants::incomplete());
                }
                single_grant_message(&witness, scope)?
            };
            if stage != 0 {
                match record.get("meta_grant") {
                    None | Some(Bson::Null) => {}
                    Some(value) => {
                        let stored: GrantRef =
                            bson::from_bson(value.clone()).map_err(|_| grants::incomplete())?;
                        reference_in_range(&stored, scope)?;
                        // This original child already carries authority. Do
                        // not replace it with a different turn's witness.
                        if stored != proof {
                            return Err(grants::incomplete());
                        }
                        continue;
                    }
                }
                db.collection::<Document>(collection).update_one(doc! {"_id":record.get("_id").cloned(),"conversation_id":id,"user_id":owner,"meta_grant":null},
                    doc! {"$set":{"meta_grant":bson::to_bson(&proof).map_err(|_|grants::incomplete())?}}).session(&mut *tx).await?;
            }
        }
        if rows.len() < PAGE as usize {
            progress.insert("guest_stage", stage + 1);
            progress.remove("after");
        } else {
            progress.insert(
                "after",
                rows.last()
                    .and_then(|row| row.get_str("_id").ok())
                    .ok_or_else(grants::incomplete)?,
            );
        }
        return Ok(Step::stay(progress));
    }
    // Voice is unavailable to visitor execution. An unexpected child is not
    // evidence that it shares the parent's grant; preserve it for repair.
    if db
        .collection::<Document>("assistant_conversations")
        .find_one(doc! {"voice_parent_conversation_id":id})
        .projection(doc! {"_id":1})
        .session(&mut *tx)
        .await?
        .is_some()
    {
        return Err(grants::incomplete());
    }
    crate::services::assistant_nyxagent::delete_in_session(db, owner, id, tx).await?;
    Ok(Step::stay(doc! {"phase":phase}))
}

fn single_grant_message(row: &Document, scope: &Document) -> AppResult<GrantRef> {
    let proofs: Vec<GrantRef> = bson::from_bson(
        row.get("meta_grants")
            .cloned()
            .ok_or_else(grants::incomplete)?,
    )
    .map_err(|_| grants::incomplete())?;
    let first = proofs.first().ok_or_else(grants::incomplete)?;
    for proof in &proofs {
        reference_in_range(proof, scope)?;
    }
    Ok(first.clone())
}

fn context_matches(value: &Bson, bot: &str, grant: &str) -> bool {
    value
        .as_document()
        .and_then(|context| context.get_document("meta_grant").ok())
        .is_some_and(|proof| {
            proof.get_str("bot_id") == Ok(bot) && proof.get_str("grant_id") == Ok(grant)
        })
}

fn event_matches(event: &Bson, bot: &str, grant: &str) -> AppResult<bool> {
    let contexts = event
        .as_document()
        .and_then(|event| event.get_array("channel_contexts").ok());
    let Some(contexts) = contexts else {
        return Ok(false);
    };
    let matches = contexts
        .iter()
        .any(|context| context_matches(context, bot, grant));
    if matches
        && contexts
            .iter()
            .any(|context| !context_matches(context, bot, grant))
    {
        return Err(grants::incomplete());
    }
    Ok(matches)
}

async fn erase_queued(
    db: &Database,
    tx: &mut ClientSession,
    scope: &Document,
    phase: i32,
) -> AppResult<Step> {
    let bot = scope.get_str("bot_id").map_err(|_| grants::incomplete())?;
    let grant = scope
        .get_str("grant_id")
        .map_err(|_| grants::incomplete())?;
    let rows = page(
        db,
        tx,
        "assistant_conversations",
        queued_filter(bot, grant),
        &Document::new(),
    )
    .await?;
    if rows.is_empty() {
        return Ok(Step::next(phase));
    }
    for row in rows {
        let mut pending = Vec::new();
        for event in row.get_array("pending_events").cloned().unwrap_or_default() {
            if !event_matches(&event, bot, grant)? {
                pending.push(event)
            }
        }
        let active = row.get_document("active_turn").ok();
        let mut selected = active
            .and_then(|turn| turn.get("channel_context"))
            .is_some_and(|context| context_matches(context, bot, grant));
        let mut foreign = active
            .and_then(|turn| turn.get("channel_context"))
            .is_some_and(|context| {
                !matches!(context, Bson::Null) && !context_matches(context, bot, grant)
            });
        for event in active
            .and_then(|turn| turn.get_array("events").ok())
            .into_iter()
            .flatten()
        {
            if event_matches(event, bot, grant)? {
                selected = true
            } else {
                foreign = true
            }
        }
        if selected && foreign {
            return Err(grants::incomplete());
        }
        let mut fields = doc! {"pending_events":pending};
        if selected {
            fields.extend(doc! {"active_turn":null,"nyxagent_session_id":null,"nyxagent_last_response_id":null,"nyxagent_instruction_binding":null,
                "context_reset_at":bson::DateTime::now(),"context_reset_reason":"provider_erasure"});
        }
        let changed=db.collection::<Document>("assistant_conversations").update_one(doc! {"_id":row.get("_id").cloned(),
            "active_turn":row.get("active_turn").cloned().unwrap_or(Bson::Null),"pending_events":row.get("pending_events").cloned().unwrap_or(Bson::Null)},
            doc! {"$set":fields,"$inc":{"meta_erasure_revision":1_i64}}).session(&mut *tx).await?;
        if changed.matched_count != 1 {
            return Err(grants::incomplete());
        }
    }
    Ok(Step::stay(doc! {"phase":phase}))
}

async fn erase_turn(
    db: &Database,
    tx: &mut ClientSession,
    scope: &Document,
    phase: i32,
) -> AppResult<Step> {
    let bot = scope.get_str("bot_id").map_err(|_| grants::incomplete())?;
    let grant = scope
        .get_str("grant_id")
        .map_err(|_| grants::incomplete())?;
    let filter = doc! {"meta_grants":{"$elemMatch":{"bot_id":bot,"grant_id":grant}}};
    let Some(message) = db
        .collection::<Document>("assistant_messages")
        .find_one(filter.clone())
        .session(&mut *tx)
        .await?
    else {
        return Ok(Step::next(phase));
    };
    single_grant_message(&message, scope)?;
    let conversation = message
        .get_str("conversation_id")
        .map_err(|_| grants::incomplete())?;
    let owner = message
        .get_str("user_id")
        .map_err(|_| grants::incomplete())?;
    let turn = message
        .get_str("turn_id")
        .map_err(|_| grants::incomplete())?;
    validate_turn_events(db, tx, conversation, turn, scope).await?;
    if let Some(parent) = db
        .collection::<Document>("assistant_conversations")
        .find_one(doc! {"_id":conversation,"user_id":owner})
        .session(&mut *tx)
        .await?
    {
        if let Ok(active) = parent.get_document("active_turn")
            && active.get_str("turn_id") != Ok(turn)
        {
            return Ok(Step::stay(doc! {"phase":phase}));
        }
        // Fence the old worker before removing any turn child. A successor's
        // stored messages survive; its next execution rebuilds context from
        // the remaining history instead of an upstream session containing A.
        let mut parent_filter = doc! {"_id":conversation,"user_id":owner};
        parent_filter.insert(
            "active_turn",
            parent.get("active_turn").cloned().unwrap_or(Bson::Null),
        );
        if db.collection::<Document>("assistant_conversations").update_one(parent_filter,
            doc! {"$set":{"active_turn":null,"nyxagent_session_id":null,"nyxagent_last_response_id":null,"nyxagent_instruction_binding":null,
                "context_reset_reason":"provider_erasure","context_reset_at":bson::DateTime::now()},"$inc":{"meta_erasure_revision":1_i64}})
            .session(&mut *tx).await?.matched_count!=1 {return Err(grants::incomplete())}
    }
    let waiting=page(db,tx,crate::models::async_service_operation::COLLECTION_NAME,
        doc! {"user_id":owner,"conversation_id":conversation,"turn_id":turn,"state":{"$nin":["cancelled","delivered"]}},&Document::new()).await?;
    if !waiting.is_empty() {
        for operation in waiting {
            if matches!(
                operation.get_str("state"),
                Ok("submitting" | "waiting" | "ready" | "queued")
            ) {
                db.collection::<Document>(crate::models::async_service_operation::COLLECTION_NAME).update_one(
                    doc! {"_id":operation.get("_id").cloned(),"user_id":owner,"conversation_id":conversation,"turn_id":turn,
                        "state":operation.get("state").cloned()},
                    doc! {"$unset":{"secret_context_versions.result_encrypted":""},"$set":{"state":"cancelling",
                        "result_encrypted":null,"lease_id":null,"lease_until":null,"poll_at":bson::DateTime::now()}}).session(&mut *tx).await?;
            } else if operation.get_str("state") != Ok("cancelling") {
                return Err(grants::incomplete());
            }
        }
        return Ok(Step::stay(doc! {"phase":phase}));
    }
    for collection in [
        crate::models::assistant_tool_result::COLLECTION_NAME,
        crate::models::assistant_attachment::COLLECTION_NAME,
        crate::models::assistant_resource::COLLECTION_NAME,
        crate::models::async_service_operation::COLLECTION_NAME,
        crate::models::generated_artifact::COLLECTION_NAME,
    ] {
        let filter = doc! {"user_id":owner,"conversation_id":conversation,"turn_id":turn};
        let rows = page(db, tx, collection, filter.clone(), &Document::new()).await?;
        if !rows.is_empty() {
            let ids: Vec<_> = rows
                .into_iter()
                .filter_map(|row| row.get("_id").cloned())
                .collect();
            db.collection::<Document>(collection)
                .delete_many(doc! {"$and":[filter,{"_id":{"$in":ids}}]})
                .session(&mut *tx)
                .await?;
            return Ok(Step::stay(doc! {"phase":phase}));
        }
    }
    erase_checkpoint(db, tx, owner, conversation, turn).await?;
    // The evidence row stays until every child page is absent, making a crash
    // before the final message removal restartable without copying turn IDs.
    let mut filter = filter;
    filter.insert("conversation_id", conversation);
    filter.insert("user_id", owner);
    filter.insert("turn_id", turn);
    erase_page(db, tx, scope, "assistant_messages", filter, phase).await
}

async fn validate_turn_events(
    db: &Database,
    tx: &mut ClientSession,
    conversation: &str,
    turn: &str,
    scope: &Document,
) -> AppResult<()> {
    let bot = scope.get_str("bot_id").map_err(|_| grants::incomplete())?;
    let grant = scope
        .get_str("grant_id")
        .map_err(|_| grants::incomplete())?;
    if db
        .collection::<Document>(EVENTS)
        .find_one(doc! {"turn_id":turn,"$and":[
            {"$or":[{"conversation_id":conversation},{"resolved_conversation_id":conversation}]},
            {"$or":[{"meta_grant.bot_id":{"$ne":bot}},{"meta_grant.grant_id":{"$ne":grant}}]}
        ]})
        .projection(doc! {"_id":1})
        .session(tx)
        .await?
        .is_some()
    {
        return Err(grants::incomplete());
    }
    Ok(())
}

/// A checkpoint can contain a new output, an inherited goal and actions from
/// different turns. Redact their exact sources; never use its parent alone to
/// erase a successor's independently authored working context.
async fn erase_checkpoint(
    db: &Database,
    tx: &mut ClientSession,
    owner: &str,
    conversation: &str,
    turn: &str,
) -> AppResult<()> {
    let rows = db.collection::<Document>(crate::models::assistant_checkpoint::COLLECTION_NAME);
    let Some(row) = rows
        .find_one(doc! {"_id":conversation,"user_id":owner})
        .session(&mut *tx)
        .await?
    else {
        return Ok(());
    };
    let mut sources = std::collections::BTreeSet::new();
    let current = row.get_str("turn_id").map_err(|_| grants::incomplete())?;
    let mut unset = Document::new();
    if row
        .get("goal")
        .is_some_and(|value| !matches!(value, Bson::Null))
    {
        row.get_str("goal").map_err(|_| grants::incomplete())?;
        let source = row
            .get_str("goal_turn_id")
            .map_err(|_| grants::incomplete())?;
        sources.insert(source.to_owned());
        if source == turn {
            unset.insert("goal", "");
            unset.insert("goal_turn_id", "");
        }
    }
    if row
        .get("output")
        .is_some_and(|value| !matches!(value, Bson::Null))
    {
        row.get_str("output").map_err(|_| grants::incomplete())?;
        sources.insert(current.to_owned());
        if current == turn {
            unset.insert("output", "");
        }
    }
    let mut fields = Document::new();
    let mut action_changed = false;
    for field in ["completed_actions", "failed_actions"] {
        let mut actions = Vec::new();
        let stored = match row.get(field) {
            None => &[][..],
            Some(Bson::Array(actions)) => actions.as_slice(),
            _ => return Err(grants::incomplete()),
        };
        for action in stored {
            let action_source = action
                .as_document()
                .and_then(|action| action.get_str("turn_id").ok())
                .ok_or_else(grants::incomplete)?;
            sources.insert(action_source.to_owned());
            if action_source == turn {
                action_changed = true;
            } else {
                actions.push(action.clone());
            }
        }
        fields.insert(field, actions);
    }
    if let Some(proof) = row.get("source_turn_ids") {
        let proof: Vec<String> =
            bson::from_bson(proof.clone()).map_err(|_| grants::incomplete())?;
        if proof.into_iter().collect::<std::collections::BTreeSet<_>>() != sources {
            return Err(grants::incomplete());
        }
    }
    if row.get_str("last_successful_turn_id") == Ok(turn) {
        unset.insert("last_successful_turn_id", "");
        unset.insert("last_successful_at", "");
    }
    if unset.is_empty() && !action_changed {
        return Ok(());
    }
    sources.remove(turn);
    fields.insert("source_turn_ids", sources.into_iter().collect::<Vec<_>>());
    let mut update = doc! {"$set":fields};
    if !unset.is_empty() {
        update.insert("$unset", unset);
    }
    let changed = rows
        .update_one(
            doc! {"_id":conversation,"user_id":owner,"turn_id":current,
            "updated_at":row.get("updated_at").cloned()},
            update,
        )
        .session(tx)
        .await?;
    if changed.matched_count != 1 {
        return Err(grants::incomplete());
    }
    Ok(())
}

async fn finish(db: &Database, tx: &mut ClientSession, scope: &Document) -> AppResult<Step> {
    let bot = scope.get_str("bot_id").map_err(|_| grants::incomplete())?;
    let grant = scope
        .get_str("grant_id")
        .map_err(|_| grants::incomplete())?;
    if db
        .collection::<Document>("assistant_conversations")
        .find_one(queued_filter(bot, grant))
        .projection(doc! {"_id":1})
        .session(&mut *tx)
        .await?
        .is_some()
    {
        return Ok(Step::stay(doc! {"phase":4}));
    }
    // The scope write in the caller serializes this positive inventory with
    // every admitted writer. Revisit the responsible phase for an exact
    // straggler instead of certifying completion or wedging at phase 19.
    for (collection, phase) in [
        (MESSAGES, 11),
        (REPLIES, 8),
        (PLANS, 9),
        (EVENTS, 10),
        (THREADS, 7),
        ("assistant_conversations", 5),
        (crate::models::org_channel::LINKS, 12),
        (crate::models::org_channel::CHALLENGES, 13),
        (crate::models::org_channel::LANES, 14),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one(exact(scope)?)
            .projection(doc! {"_id":1})
            .session(&mut *tx)
            .await?
            .is_some()
        {
            return Ok(Step::stay(doc! {"phase":phase}));
        }
    }
    for (collection, filter, phase) in [
        (
            "assistant_messages",
            doc! {"meta_grants":{"$elemMatch":{"bot_id":bot,"grant_id":grant}}},
            6,
        ),
        (
            super::meta::CONTROLS,
            doc! {"bot_id":bot,"meta_grant_id":grant},
            15,
        ),
        (
            crate::models::coordination::EVENT_DEDUP_COLLECTION_NAME,
            doc! {"channel_bot_id":bot,"meta_grant_id":grant},
            16,
        ),
        (
            crate::models::channel_meta::SESSIONS,
            doc! {"_id":grant,"bot_id":bot},
            17,
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .find_one(filter)
            .projection(doc! {"_id":1})
            .session(&mut *tx)
            .await?
            .is_some()
        {
            return Ok(Step::stay(doc! {"phase":phase}));
        }
    }
    if let Some(parent) = db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! {"_id":bot})
        .session(&mut *tx)
        .await?
        && parent
            .meta_binding
            .as_ref()
            .is_some_and(|binding| binding.grant_id == grant)
    {
        if parent.is_active
            || parent
                .meta_binding
                .as_ref()
                .is_none_or(|binding| binding.subscription != "removed")
        {
            return Err(grants::incomplete());
        }
        crate::services::org_channel_service::defaults::erase_in_session(
            db,
            doc! {"channel_bot_id":bot},
            tx,
        )
        .await?;
        if db.collection::<ChannelBot>(BOTS).delete_one(doc! {"_id":bot,"is_active":false,"meta_binding.grant_id":grant,
            "meta_binding.generation":parent.meta_binding.as_ref().map(|binding|binding.generation),"meta_binding.subscription":"removed"})
            .session(&mut *tx).await?.deleted_count!=1 {return Err(grants::incomplete())}
    }
    scopes::mark_complete(db, tx, bot, grant).await?;
    db.collection::<Document>("channel_meta_erasure_fences")
        .update_one(
            doc! {"_id":bot,"retired_grant.grant_id":grant},
            doc! {"$unset":{"retired_grant":""}},
        )
        .session(tx)
        .await?;
    Ok(Step {
        progress: Document::new(),
        done: true,
        repair: false,
    })
}

async fn retire_shared(
    db: &Database,
    tx: &mut ClientSession,
    scope: &Document,
    phase: i32,
) -> AppResult<Step> {
    let bot = scope.get_str("bot_id").map_err(|_| grants::incomplete())?;
    let grant = scope
        .get_str("grant_id")
        .map_err(|_| grants::incomplete())?;
    if db
        .collection::<Document>(BOTS)
        .find_one(
            doc! {"_id":bot,"is_active":false,"meta_binding.grant_id":grant,
            "meta_binding.subscription":"removed"},
        )
        .session(&mut *tx)
        .await?
        .is_none()
    {
        return Ok(Step::next(phase));
    }
    let filter = doc! {"channel_bot_id":bot,"retired_by_transfer":{"$ne":true},"$or":[{"is_active":true},{"default_agent":true}]};
    let rows = page(
        db,
        tx,
        crate::models::channel_conversation::COLLECTION_NAME,
        filter,
        &Document::new(),
    )
    .await?;
    if !rows.is_empty() {
        let ids: Vec<_> = rows
            .into_iter()
            .filter_map(|row| row.get("_id").cloned())
            .collect();
        db.collection::<Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .update_many(
                doc! {"_id":{"$in":ids}},
                doc! {"$set":{"is_active":false,"default_agent":false}},
            )
            .session(&mut *tx)
            .await?;
        return Ok(Step::stay(doc! {"phase":phase}));
    }
    let rows = page(
        db,
        tx,
        CHANNELS,
        doc! {"channel_bot_id":bot,"status":{"$in":["active","pending"]}},
        &Document::new(),
    )
    .await?;
    if rows.is_empty() {
        return Ok(Step::next(phase));
    }
    let ids: Vec<_> = rows
        .into_iter()
        .filter_map(|row| row.get("_id").cloned())
        .collect();
    db.collection::<Document>(CHANNELS).update_many(doc! {"_id":{"$in":ids}},
        doc! {"$set":{"status":"disconnected","owner_sender_ids":[]},"$unset":{"link_code_hash":"","link_code_expires_at":""},
            "$inc":{"execution_policy_revision":1_i64}}).session(&mut *tx).await?;
    Ok(Step::stay(doc! {"phase":phase}))
}

async fn step(
    db: &Database,
    tx: &mut ClientSession,
    scope: &Document,
    progress: &Document,
    phase: i32,
) -> AppResult<Step> {
    let bot = scope.get_str("bot_id").map_err(|_| grants::incomplete())?;
    let grant = scope
        .get_str("grant_id")
        .map_err(|_| grants::incomplete())?;
    match phase {
        0..=2 => attribute(db, tx, scope, progress, phase).await,
        3 => attribute_events(db, tx, scope, progress, phase).await,
        4 => erase_queued(db, tx, scope, phase).await,
        5 => erase_guest(db, tx, scope, progress, phase).await,
        6 => erase_turn(db, tx, scope, phase).await,
        7 => clear_threads(db, tx, scope, phase).await,
        8..=14 => {
            let collection = match phase {
                8 => REPLIES,
                9 => PLANS,
                10 => EVENTS,
                11 => MESSAGES,
                12 => crate::models::org_channel::LINKS,
                13 => crate::models::org_channel::CHALLENGES,
                _ => crate::models::org_channel::LANES,
            };
            erase_page(db, tx, scope, collection, exact(scope)?, phase).await
        }
        15 | 16 => {
            let (collection, field) = if phase == 15 {
                (super::meta::CONTROLS, "bot_id")
            } else {
                (
                    crate::models::coordination::EVENT_DEDUP_COLLECTION_NAME,
                    "channel_bot_id",
                )
            };
            erase_page(
                db,
                tx,
                scope,
                collection,
                doc! {field:bot,"meta_grant_id":grant},
                phase,
            )
            .await
        }
        17 => {
            erase_page(
                db,
                tx,
                scope,
                crate::models::channel_meta::SESSIONS,
                doc! {"_id":grant,"bot_id":bot},
                phase,
            )
            .await
        }
        18 => retire_shared(db, tx, scope, phase).await,
        19 => finish(db, tx, scope).await,
        _ => Err(grants::incomplete()),
    }
}

pub(crate) async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::IndexModel;
    for collection in [
        MESSAGES,
        REPLIES,
        PLANS,
        EVENTS,
        THREADS,
        "assistant_conversations",
        crate::models::org_channel::LINKS,
        crate::models::org_channel::CHALLENGES,
        crate::models::org_channel::LANES,
    ] {
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"meta_grant.bot_id":1,"meta_grant.grant_id":1,"_id":1})
                    .build(),
            )
            .await?;
    }
    for collection in [MESSAGES, REPLIES, PLANS] {
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"channel_bot_id":1,"_id":1})
                    .build(),
            )
            .await?;
    }
    db.collection::<Document>(EVENTS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"channel_id":1,"_id":1})
                .build(),
        )
        .await?;
    for field in ["conversation_id", "resolved_conversation_id"] {
        db.collection::<Document>(EVENTS)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {field:1,"turn_id":1,"_id":1})
                    .build(),
            )
            .await?;
    }
    db.collection::<Document>("assistant_messages")
        .create_index(
            IndexModel::builder()
                .keys(doc! {"meta_grants.bot_id":1,"meta_grants.grant_id":1,"_id":1})
                .build(),
        )
        .await?;
    db.collection::<Document>(CHANNELS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"channel_bot_id":1,"_id":1})
                .build(),
        )
        .await?;
    for (collection, field) in [
        (super::meta::CONTROLS, "bot_id"),
        (
            crate::models::coordination::EVENT_DEDUP_COLLECTION_NAME,
            "channel_bot_id",
        ),
    ] {
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {field:1,"meta_grant_id":1,"_id":1})
                    .build(),
            )
            .await?;
    }
    for collection in [
        crate::models::assistant_tool_result::COLLECTION_NAME,
        crate::models::assistant_attachment::COLLECTION_NAME,
        crate::models::assistant_resource::COLLECTION_NAME,
        crate::models::async_service_operation::COLLECTION_NAME,
        crate::models::generated_artifact::COLLECTION_NAME,
    ] {
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"user_id":1,"conversation_id":1,"turn_id":1,"_id":1})
                    .build(),
            )
            .await?;
    }
    for prefix in [
        "pending_events.channel_contexts.meta_grant",
        "active_turn.channel_context.meta_grant",
        "active_turn.events.channel_contexts.meta_grant",
    ] {
        db.collection::<Document>("assistant_conversations")
            .create_index(
                IndexModel::builder()
                    .keys(
                        doc! {format!("{prefix}.bot_id"):1,format!("{prefix}.grant_id"):1,"_id":1},
                    )
                    .build(),
            )
            .await?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "channel_meta_scope_erasure_tests.rs"]
mod tests;
