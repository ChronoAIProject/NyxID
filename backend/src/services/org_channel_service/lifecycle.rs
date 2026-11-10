//! Retire authority immediately; content cleanup waits for ordinary settlement.
use crate::{
    errors::AppResult,
    models::{assistant_conversation::AssistantConversation, org_channel::*},
};
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};

pub(crate) async fn retire_in_session(
    db: &Database,
    tx: &mut ClientSession,
    channel: &str,
    actor: Option<&str>,
) -> AppResult<()> {
    let mut filter = doc! {"channel_id":channel};
    if let Some(actor) = actor {
        filter.insert("user_id", actor);
    }
    db.collection::<Document>(LINKS)
        .update_many(
            filter.clone(),
            doc! {"$set":{"active":false,"revision":uuid::Uuid::new_v4().to_string()}},
        )
        .session(&mut *tx)
        .await?;
    db.collection::<Document>(crate::models::assistant_notification::COLLECTION_NAME)
        .delete_many({
            let mut notification_filter = doc! {"source.channel_id":channel};
            if let Some(actor) = actor {
                // An org-id guest lane has no person-owned feed rows; only
                // a person actor retires that person's private notifications.
                notification_filter.insert("user_id", actor);
            }
            notification_filter
        })
        .session(&mut *tx)
        .await?;
    db.collection::<Document>(CHALLENGES)
        .delete_many(filter)
        .session(&mut *tx)
        .await?;
    let mut filter = doc! {"$or":[{"channel.nyxbot_channel_id":channel,"channel.org_execution":{"$type":"object"}},
    {"reply_channel.nyxbot_channel_id":channel,"reply_channel.org_execution":{"$type":"object"}}]};
    if let Some(actor) = actor {
        filter.insert("user_id", actor);
    }
    let mut cursor = db
        .collection::<AssistantConversation>("assistant_conversations")
        .find(filter)
        .session(&mut *tx)
        .await?;
    let rows: Vec<AssistantConversation> = cursor.stream(&mut *tx).try_collect().await?;
    for row in rows {
        let keys = super::super::assistant_team_service::thread_key_ids(
            db,
            &row.user_id,
            std::slice::from_ref(&row),
            tx,
        )
        .await?;
        for key in keys {
            super::super::api_key_mutation_service::update_one(
                db,
                doc! {"_id":&key,"user_id":&row.user_id},
                doc! {"$set":{"is_active":false}},
                Some(&mut *tx),
            )
            .await?;
            super::super::assistant_agent_credential_service::invalidate_for_key(
                db,
                &row.user_id,
                &key,
                Some(&mut *tx),
            )
            .await?;
        }
        if row.active_turn.is_some() {
            db.collection::<Document>("assistant_conversations")
                .update_one(
                    doc! {"_id":&row.id},
                    doc! {"$set":{"active_turn.stop_requested":true}},
                )
                .session(&mut *tx)
                .await?;
        }
    }
    Ok(())
}

pub(crate) async fn delete_conversation(
    db: &Database,
    tx: &mut ClientSession,
    conversation: &str,
) -> AppResult<()> {
    db.collection::<Document>(crate::models::assistant_notification::COLLECTION_NAME)
        .delete_many(doc! {"source.conversation_id":conversation})
        .session(&mut *tx)
        .await?;
    for collection in [LANES, MESSAGES] {
        db.collection::<Document>(collection)
            .delete_many(doc! {"conversation_id":conversation})
            .session(&mut *tx)
            .await?;
    }
    // Provider recovery belongs to the transport admin, not the execution
    // actor. Conversation IDs are globally unique; purge its encrypted replay
    // with the actual conversation regardless of that separate ownership.
    db.collection::<Document>(crate::models::nyxbot_gateway_response::COLLECTION_NAME)
        .delete_many(doc! {"conversation_id":conversation})
        .session(&mut *tx)
        .await?;
    db.collection::<Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME)
        .update_many(doc!{"guest_setup_conversation_id":conversation,"turn_id":null},
            doc!{"$set":{"status":"refused","delivery.state":"refused"},"$unset":{"guest_setup_payload":""}}).session(tx).await?;
    Ok(())
}

/// Erasure removes hidden execution history through the existing conversation
/// cleanup. Member-owned private homes remain the member's own history. Stop
/// and wait for active work; never discard a conversation before settlement.
pub(crate) async fn erase_hidden(db: &Database, filter: Document) -> AppResult<bool> {
    let rows: Vec<AssistantConversation> = db
        .collection("assistant_conversations")
        .find(filter)
        .limit(100)
        .await?
        .try_collect()
        .await?;
    if rows.is_empty() {
        return Ok(true);
    }
    for row in rows {
        if row.active_turn.is_some() {
            db.collection::<Document>("assistant_conversations")
                .update_one(
                    doc! {"_id":&row.id,"active_turn":{"$ne":null}},
                    doc! {"$set":{"active_turn.stop_requested":true}},
                )
                .await?;
            return Ok(false);
        }
        let context = (db.clone(), row.user_id.clone(), row.id.clone());
        let children = crate::services::transaction_service::run_db_only(
            db,
            &context,
            |tx, (db, actor, id)| {
                Box::pin(async move {
                    let fenced = db
                        .collection::<Document>("assistant_conversations")
                        .update_one(
                            doc! {"_id":id,"user_id":actor,"active_turn":null},
                            doc! {"$inc":{"org_channel_fence":1_i64}},
                        )
                        .session(&mut *tx)
                        .await?;
                    if fenced.matched_count != 1 {
                        return Err(crate::errors::AppError::Conflict(
                            "Channel history is still settling".into(),
                        ));
                    }
                    let (_, children) = Box::pin(
                        crate::services::assistant_nyxagent::delete_in_session(db, actor, id, tx),
                    )
                    .await?;
                    Ok(children)
                })
            },
        )
        .await?;
        crate::services::api_key_credential_service::audit_revocations(
            db,
            &children,
            crate::models::api_key_credential::CredentialRevokedReason::ParentRevoked,
        );
    }
    Ok(false)
}

/// Called from the existing erasure job, after its ordinary settlement barriers.
/// Financial evidence follows usage retention; live holds must never be dropped.
pub(crate) async fn erase_person(db: &Database, person: &str) -> AppResult<()> {
    db.collection::<Document>(LINKS).update_many(doc!{"user_id":person},doc!{"$set":{"active":false,"user_id":"erased","revision":uuid::Uuid::new_v4().to_string()}}).await?;
    db.collection::<Document>(CHALLENGES)
        .delete_many(doc! {"user_id":person})
        .await?;
    db.collection::<Document>(LANES)
        .delete_many(doc! {"actor_id":person})
        .await?;
    db.collection::<Document>(MESSAGES)
        .delete_many(doc! {"$or":[{"author_user_id":person},{"actor_id":person}]})
        .await?;
    Ok(())
}

/// Membership removal tombstones identities and revokes only this actor's lanes.
/// A subsequent rejoin requires fresh confirmation; it cannot revive old keys.
pub(crate) async fn revoke_member_in_session(
    db: &Database,
    tx: &mut ClientSession,
    org: &str,
    actor: &str,
) -> AppResult<()> {
    let mut cursor = db
        .collection::<Document>(LINKS)
        .find(doc! {"org_id":org,"user_id":actor,"active":true})
        .session(&mut *tx)
        .await?;
    let rows: Vec<Document> = cursor.stream(&mut *tx).try_collect().await?;
    let channels: std::collections::BTreeSet<_> = rows
        .iter()
        .filter_map(|r| r.get_str("channel_id").ok())
        .collect();
    for channel in channels {
        retire_in_session(db, tx, channel, Some(actor)).await?;
    }
    Ok(())
}

/// An org can be deleted only after execution/settlement blockers have cleared.
/// Financial records keep their ordinary accounting retention.
pub(crate) async fn erase_org(db: &Database, org: &str) -> AppResult<()> {
    while !erase_hidden(db, doc! {"channel.org_execution.org_id":org}).await? {
        if db
            .collection::<Document>("assistant_conversations")
            .find_one(doc! {"channel.org_execution.org_id":org,"active_turn":{"$ne":null}})
            .await?
            .is_some()
        {
            return Err(crate::errors::AppError::Conflict(
                "Organization channel turns must settle before deletion".into(),
            ));
        }
    }
    for collection in [LINKS, CHALLENGES, LANES] {
        db.collection::<Document>(collection)
            .delete_many(doc! {"org_id":org})
            .await?;
    }
    db.collection::<Document>(MESSAGES)
        .delete_many(doc! {"user_id":org})
        .await?;
    Ok(())
}
