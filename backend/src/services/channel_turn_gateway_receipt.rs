//! Receipt polling remains a branch of the event's text preparation claim.
use super::{SEND_SECS, lost};
use crate::{
    errors::AppResult,
    models::{
        channel_turn_preparation::Preparation,
        gateway_reply::{Failure, Observation, Outcome, ReceiptWait, Reply},
        nyxbot_channel::{EVENTS_COLLECTION_NAME as EVENTS, NyxbotEvent},
    },
};
use bson::{Document, doc};
use chrono::{DateTime, Duration, Utc};
use mongodb::{Database, options::ReturnDocument};

pub const DUE_INDEX: &str = "nyxbot_receipt_due";
pub fn due(now: DateTime<Utc>) -> Document {
    doc! {"delivery.receipt_due_at":{"$type":"date","$lte":bson::DateTime::from_chrono(now)},
    "delivery.version":1,"delivery.state":"unknown","delivery.preparation.version":1,
    "delivery.preparation.gateway_receipt.version":{"$in":[1,2]},
    "delivery.preparation.gateway_receipt.pending":true}
}
fn pending(event: &str) -> Document {
    doc! {"_id":event,"delivery.version":1,"delivery.state":"unknown",
    "delivery.preparation.version":1,"delivery.preparation.gateway_receipt.version":{"$in":[1,2]},
    "delivery.preparation.gateway_receipt.pending":true}
}
pub async fn acquire(db: &Database, event: &str, claim: &str) -> AppResult<Option<Preparation>> {
    let mut filter = pending(event);
    filter.insert("$expr", doc! {"$lte":["$delivery.receipt_due_at","$$NOW"]});
    let row = db
        .collection::<Document>(EVENTS)
        .find_one_and_update(
            filter,
            vec![doc! {"$set":{
                "delivery.claim_id":{"$literal":claim},
                "delivery.preparation.gateway_receipt.version":2,
                "delivery.preparation.lease_until":{"$add":["$$NOW",SEND_SECS*1000]},
                "delivery.receipt_due_at":{"$add":["$$NOW",{"$cond":[
                    {"$gte":["$delivery.preparation.gateway_receipt.attempts",4]},300_000,SEND_SECS*1000]}]},
                "delivery.preparation.gateway_receipt.attempts":{"$min":[4_294_967_295_i64,
                    {"$add":["$delivery.preparation.gateway_receipt.attempts",1]}]},
            }}],
        )
        .return_document(ReturnDocument::After)
        .await?;
    row.map(|row| {
        bson::from_document(
            row.get_document("delivery")
                .and_then(|d| d.get_document("preparation"))
                .map_err(|_| lost())?
                .clone(),
        )
        .map_err(|_| lost())
    })
    .transpose()
}
/// Recover only the frozen reply-target operation, using the original send
/// barrier as the clock anchor. Older senders still settle terminal unknown.
pub async fn recover(db: &Database, event: &str) -> AppResult<Option<NyxbotEvent>> {
    let observation = bson::to_bson(&Observation {
        failure: Some(Failure::Interrupted),
        ..Default::default()
    })
    .map_err(|_| lost())?;
    Ok(db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one_and_update(
            doc! {
                "_id":event,"delivery.version":1,"delivery.transport":"gateway",
                "delivery.state":"sending","delivery.preparation.version":1,
                "delivery.preparation.gateway_request_version":1,
                "delivery.preparation.gateway_request_hash":{"$regex":"^[0-9a-f]{64}$"},
                "delivery.preparation.gateway_receipt":bson::Bson::Null,
                "delivery.target_ciphertext":{"$type":["binData","array"]},
                "delivery.target_expires_at":{"$type":"date"},
                "expires_at":{"$type":"date"},
                "delivery.checked_at":{"$type":"date"},
                "delivery.preparation.lease_until":{"$type":"date"},
                "$expr":{"$and":[
                    {"$lte":["$delivery.preparation.lease_until","$$NOW"]},
                    {"$lte":["$delivery.checked_at","$$NOW"]}
                ]},
            },
            vec![doc! {"$set":{
                "delivery.state":"unknown","delivery.receipt_due_at":"$$NOW",
                "delivery.preparation.gateway_receipt":{
                    "version":2,"pending":true,"attempts":1,
                    "started_at":"$delivery.checked_at","admitted_at":bson::Bson::Null,
                    "deadline":{"$min":["$delivery.target_expires_at","$expires_at",
                        {"$add":["$delivery.checked_at",86_400_000_i64]}]},
                    "observation":{"$literal":observation},
                },
            }}],
        )
        .return_document(ReturnDocument::After)
        .await?)
}

#[cfg(test)]
pub async fn before_effect(db: &Database, event: &str, claim: &str) -> AppResult<()> {
    before_gateway_effect(db, event, None, claim).await
}

pub(crate) async fn before_gateway_effect(
    db: &Database,
    event: &str,
    source: Option<&crate::models::nyxbot_channel::NyxbotEvent>,
    claim: &str,
) -> AppResult<()> {
    let mut filter = pending(event);
    filter.insert("delivery.claim_id", claim);
    filter.insert(
        "$expr",
        doc! {"$and":[
            {"$gt":["$delivery.preparation.lease_until","$$NOW"]},
            {"$gt":["$delivery.preparation.gateway_receipt.deadline","$$NOW"]}
        ]},
    );
    let pipeline = vec![doc! {"$set":{
        "delivery.gateway_attempted":true,
        "delivery.preparation.lease_until":{"$add":["$$NOW",SEND_SECS*1000]},
        "delivery.receipt_due_at":{"$max":["$delivery.receipt_due_at",
            {"$add":["$$NOW",SEND_SECS*1000]}]},
    }}];
    super::commit_text_effect(db, source, filter, pipeline).await
}

pub async fn plan(db: &Database, event: &str, claim: &str, hash: &str) -> AppResult<()> {
    let result = db
        .collection::<Document>(EVENTS)
        .update_one(
            doc! {
                "_id":event,"delivery.state":"preparing_text","delivery.claim_id":claim,
                "$or":[{"delivery.preparation.gateway_request_hash":bson::Bson::Null},
                    {"delivery.preparation.gateway_request_hash":hash}],
                "$expr":{"$gt":["$delivery.preparation.lease_until","$$NOW"]},
            },
            doc! {"$set":{"delivery.preparation.gateway_request_hash":hash,"delivery.preparation.gateway_request_version":1}},
        )
        .await?;
    if result.matched_count == 1 {
        Ok(())
    } else {
        Err(lost())
    }
}
fn backoff(attempts: u32) -> i64 {
    match attempts {
        0 | 1 => 15,
        2 => 30,
        3 => 60,
        4 => 120,
        _ => 300,
    }
}

/// The claim, bot diagnostic and audit commit together, including first admission.
#[allow(clippy::too_many_arguments)]
pub async fn record(
    db: &Database,
    event_id: &str,
    claim: &str,
    bot_id: &str,
    reply: &Reply,
    previous: Option<&ReceiptWait>,
    deadline: DateTime<Utc>,
    reason: &str,
) -> AppResult<bool> {
    use crate::services::{api_key_mutation_service as tx, audit_service};
    // A later local refusal cannot erase an earlier gateway attempt.
    let prior = previous
        .filter(|_| reply.outcome == Outcome::Rejected && reply.observation.status.is_none())
        .map(|wait| unknown(wait.observation.clone()));
    let reply = prior.as_ref().unwrap_or(reply);
    let local_refusal = reply.outcome == Outcome::Rejected && reply.observation.status.is_none();
    let now = Utc::now();
    let deadline = previous.map_or(deadline, |p| deadline.min(p.deadline));
    let retry = reply.outcome == Outcome::Pending && deadline > now;
    let admitted = previous
        .and_then(|p| p.admitted_at)
        .or_else(|| reply.observation.admitted().then_some(now));
    let started = previous.map_or(now, |p| p.started_at);
    let wait = ReceiptWait {
        version: 2,
        pending: retry,
        started_at: started,
        admitted_at: admitted,
        deadline,
        attempts: previous.map_or(1, |p| p.attempts),
        observation: reply.observation.clone(),
    };
    let status = match reply.outcome {
        Outcome::Confirmed => "sent",
        Outcome::Rejected => "refused",
        _ => "unknown",
    };
    let next = (now + Duration::seconds(backoff(wait.attempts)))
        .max(reply.observation.retry_after.unwrap_or(now))
        .min(deadline);
    let mut filter = if previous.is_some() {
        pending(event_id)
    } else {
        doc! {"_id":event_id,"delivery.version":1,"delivery.preparation.version":1,
        "delivery.state":{"$in":["sending","preparing_text"]}}
    };
    if local_refusal {
        filter.insert("delivery.state", "preparing_text");
        filter.insert("delivery.gateway_attempted", doc! {"$ne":true});
    }
    filter.insert("delivery.claim_id", claim);
    filter.insert(
        "$expr",
        doc! {"$gt":["$delivery.preparation.lease_until","$$NOW"]},
    );
    let mut set =
        doc! {"delivery.state":status,"delivery.checked_at":bson::DateTime::from_chrono(now)};
    if !local_refusal {
        set.insert(
            "delivery.gateway_observation",
            bson::to_bson(&reply.observation).map_err(|_| lost())?,
        );
        set.insert("delivery.gateway_reason", reason);
    }
    if retry || previous.is_some() {
        set.insert(
            "delivery.preparation.gateway_receipt",
            bson::to_bson(&wait).map_err(|_| lost())?,
        );
    }
    let mut update = doc! {"$set":set};
    if retry {
        update
            .get_document_mut("$set")
            .map_err(|_| lost())?
            .insert("delivery.receipt_due_at", bson::DateTime::from_chrono(next));
    } else {
        update.insert("$unset", doc! {"delivery.receipt_due_at":""});
    }
    let admission =
        reply.observation.admitted() && previous.is_none_or(|p| p.admitted_at.is_none());
    let outcome = if retry {
        "receipt_pending"
    } else {
        match reply.outcome {
            Outcome::Confirmed => "sent",
            Outcome::Rejected => "rejected",
            _ => "unknown",
        }
    };
    let health_reason = if reply.outcome == Outcome::Rejected {
        "gateway_reply_rejected"
    } else if admitted.is_some() {
        "gateway_reply_unconfirmed"
    } else if reply.observation.status == Some(503)
        && reply.observation.code
            == Some(crate::models::gateway_reply::Code::RoutedReplyUnavailable)
    {
        "gateway_reply_unavailable"
    } else if let Some(failure) = reply.observation.failure {
        failure.health_reason(retry)
    } else {
        "gateway_reply_unknown"
    };
    let observation = reply.observation.clone();
    let confirmed = reply.outcome == Outcome::Confirmed;
    let bot_id = bot_id.to_owned();
    let reason = reason.to_owned();
    let db = db.clone();
    let rejected = reply.outcome == Outcome::Rejected;
    let mut session = db.client().start_session().await?;
    let committed_turn = session.start_transaction().and_run(
        (db, filter, update, bot_id, observation, reason, started, now, retry, confirmed, rejected, admission, outcome, health_reason, admitted, local_refusal),
        |session,(db,filter,update,bot_id,observation,reason,started,now,retry,confirmed,rejected,admission,outcome,health_reason,admitted,local_refusal)| {
            Box::pin(async move {
                let operation: AppResult<Option<String>> = async {
                    let Some(raw_event) = db.collection::<Document>(EVENTS)
                        .find_one_and_update(filter.clone(),update.clone())
                        .session(&mut *session).await? else { return Ok(None); };
                    let notice = raw_event.get_document("delivery").ok()
                        .and_then(|delivery| delivery.get_document("preparation").ok())
                        .and_then(|preparation| preparation.get_document("notice").ok())
                        .map(|notice| bson::from_document::<crate::models::channel_text_delivery::TextDeliveryNotice>(notice.clone()))
                        .transpose().map_err(|_| lost())?;
                    let event: NyxbotEvent = bson::from_document(raw_event).map_err(|_| lost())?;
                    if *local_refusal {
                        return Ok(Some(event.turn_id.unwrap_or_default()));
                    }
                    if !*retry && !*confirmed && !*rejected {
                        crate::services::channel_reply_notice::late_unconfirmed(db,session,&event).await?;
                    }
                    let time = bson::DateTime::from_chrono(*started);
                    let health = doc! {"_id":bot_id,"$and":[
                        {"$or":[{"diagnostics.last_outbound_at":bson::Bson::Null},{"diagnostics.last_outbound_at":{"$lte":time}}]},
                        {"$or":[{"diagnostics.last_outbound_error.at":bson::Bson::Null},{"diagnostics.last_outbound_error.at":{"$lte":time}}]}
                    ]};
                    let diagnostic = if *confirmed {
                        doc! {"diagnostics.last_outbound_at":bson::DateTime::from_chrono(*now),"diagnostics.last_outbound_error":bson::Bson::Null}
                    } else {
                        doc! {"diagnostics.last_outbound_error":{"code":"10005","reason":*health_reason,"at":time}}
                    };
                    db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
                        .update_one(health,doc! {"$set":diagnostic}).session(&mut *session).await?;
                    for outcome in [admission.then_some("receipt_pending"), (!*retry).then_some(*outcome)].into_iter().flatten() {
                        let actor = audit_service::AuditActor {user_id:event.user_id.clone(),ip_address:None,user_agent:None,api_key_id:None,api_key_name:None};
                        let mut audit = serde_json::json!({
                            "event_id":event.id,"conversation_id":event.conversation_id,"turn_id":event.turn_id,
                            "transport":"gateway","outcome":outcome,"reason":reason,
                            "http_status_class":observation.status_class(),"gateway_code":observation.code,
                            "gateway_admitted":admitted.is_some(),"gateway_failure":observation.failure
                        });
                        if outcome != "receipt_pending" {
                            audit["images_attempted"] = notice.as_ref().map_or(0, |n| n.attempted).into();
                            audit["images_sent"] = notice.as_ref().map_or(0, |n| n.sent).into();
                            audit["images_failed"] = notice.as_ref().map_or(0, |n| n.failed).into();
                            audit["images_unknown"] = notice.as_ref().map_or(0, |n| n.unknown).into();
                            audit["images_skipped"] = notice.as_ref().map_or(0, |n| n.skipped).into();
                        }
                        audit_service::log_actor_event_in_transaction(db,session,&actor,"nyxbot_channel_answer_delivery",audit).await?;
                    }
                    Ok(Some(event.turn_id.unwrap_or_default()))
                }.await;
                tx::transaction_result(operation)
            })
        },
    ).await.map_err(tx::map_transaction_error)?;
    if let Some(turn) = &committed_turn {
        // Trace emits only after the successful transaction, never on a retry.
        // The existing trace enum has no pending outcome.
        if admission {
            crate::turn_trace::milestone(
                turn,
                crate::turn_trace::Stage::Channel,
                None,
                Some(crate::turn_trace::Outcome::Unknown),
            );
        }
        if !retry {
            crate::turn_trace::milestone(
                turn,
                crate::turn_trace::Stage::Channel,
                None,
                Some(if confirmed {
                    crate::turn_trace::Outcome::Confirmed
                } else if rejected {
                    crate::turn_trace::Outcome::Rejected
                } else {
                    crate::turn_trace::Outcome::Unknown
                }),
            );
        }
    }
    Ok(committed_turn.is_some())
}

pub fn unknown(observation: Observation) -> Reply {
    Reply {
        outcome: Outcome::Unknown,
        observation,
    }
}

impl Failure {
    fn health_reason(self, pending: bool) -> &'static str {
        match (self, pending) {
            (Self::NoResponse, true) => "gateway_reply_no_response_pending",
            (Self::NoResponse, false) => "gateway_reply_no_response",
            (Self::ServerError, true) => "gateway_reply_server_error_pending",
            (Self::ServerError, false) => "gateway_reply_server_error",
            (Self::UnreadableResponse, true) => "gateway_reply_unreadable_response_pending",
            (Self::UnreadableResponse, false) => "gateway_reply_unreadable_response",
            (Self::Interrupted, true) => "gateway_reply_interrupted_pending",
            (Self::Interrupted, false) => "gateway_reply_interrupted",
        }
    }
}
