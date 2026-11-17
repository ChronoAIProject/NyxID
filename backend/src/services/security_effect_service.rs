//! Durable metadata-only effects committed by security mutations.

use std::{sync::Arc, time::Duration};

use bson::{Document, doc};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    options::{Hint, IndexOptions, ReturnDocument},
};
use uuid::Uuid;

use crate::{
    crypto::aes::EncryptionKeys,
    errors::{AppError, AppResult},
    models::security_effect_intent::{
        COLLECTION_NAME, IntentState, SecurityEffect, SecurityEffectIntent,
    },
};

const INTENT_VERSION: i32 = 1;
const DUE_INDEX: &str = "security_effect_due";
const DIAGNOSTIC_INDEX: &str = "security_effect_diagnostic";
const TERMINAL_TTL_INDEX: &str = "security_effect_terminal_ttl";
const TERMINAL_RETENTION: Duration = Duration::from_secs(30 * 24 * 60 * 60);

const CLAIM_LIFETIME: Duration = Duration::from_secs(60);
const POLL_INTERVAL: Duration = Duration::from_secs(5);
const PASS_LIMIT: i64 = 50;
const RETRY_BASE_SECONDS: i64 = 5;
const RETRY_MAX_SECONDS: i64 = 3600;
const CAE_MAX_ATTEMPTS: i64 = 8;

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<SecurityEffectIntent>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "version": 1, "state": 1, "next_attempt_at": 1, "_id": 1 })
                .options(IndexOptions::builder().name(DUE_INDEX.to_owned()).build())
                .build(),
        )
        .await?;
    db.collection::<SecurityEffectIntent>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "version": 1, "kind": 1, "state": 1, "created_at": 1 })
                .options(
                    IndexOptions::builder()
                        .name(DIAGNOSTIC_INDEX.to_owned())
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<SecurityEffectIntent>(COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "completed_at": 1 })
                .options(
                    IndexOptions::builder()
                        .name(TERMINAL_TTL_INDEX.to_owned())
                        .expire_after(TERMINAL_RETENTION)
                        .partial_filter_expression(doc! {
                            "version": INTENT_VERSION,
                            "state": { "$in": ["delivered", "failed", "uncertain"] },
                            "completed_at": { "$type": "date" },
                        })
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

pub fn new_intent(id: String, effect: SecurityEffect) -> SecurityEffectIntent {
    SecurityEffectIntent {
        id,
        version: INTENT_VERSION,
        effect,
        state: IntentState::Pending,
        attempts: 0,
        created_at: Utc::now(),
        // Immediately eligible without trusting the producer's wall clock.
        next_attempt_at: chrono::DateTime::UNIX_EPOCH,
        claim_token: None,
        completed_at: None,
        outcome: None,
    }
}

/// One write for all effects of this source transaction. No provider I/O
/// belongs here; aborting the transaction also discards every occurrence.
pub async fn enqueue_in_session(
    db: &Database,
    session: &mut ClientSession,
    intents: &[SecurityEffectIntent],
) -> AppResult<()> {
    if !intents.is_empty() {
        db.collection::<SecurityEffectIntent>(COLLECTION_NAME)
            .insert_many(intents)
            .session(session)
            .await?;
    }
    Ok(())
}

#[cfg(test)]
async fn enqueue(db: &Database, intent: &SecurityEffectIntent) -> AppResult<()> {
    db.collection::<SecurityEffectIntent>(COLLECTION_NAME)
        .insert_one(intent)
        .await?;
    Ok(())
}

fn due_filter() -> Document {
    doc! {
        "version": INTENT_VERSION,
        "kind": { "$in": ["audit", "cae"] },
        "state": { "$in": ["pending", "claimed", "sending"] },
        "$expr": { "$lte": ["$next_attempt_at", "$$NOW"] },
    }
}

fn exact_claim(intent: &SecurityEffectIntent) -> AppResult<Document> {
    let token = intent
        .claim_token
        .as_ref()
        .ok_or_else(|| AppError::Internal("Security effect has no claim".into()))?;
    Ok(doc! {
        "_id": &intent.id,
        "version": INTENT_VERSION,
        "claim_token": token,
    })
}

async fn claim(
    db: &Database,
    candidate: &SecurityEffectIntent,
) -> AppResult<Option<SecurityEffectIntent>> {
    let mut filter = due_filter();
    filter.insert("_id", &candidate.id);
    // Sending is a durable uncertainty barrier, never a retryable CAE claim.
    filter.insert("state", doc! { "$in": ["pending", "claimed"] });
    Ok(db
        .collection::<SecurityEffectIntent>(COLLECTION_NAME)
        .find_one_and_update(
            filter,
            vec![doc! { "$set": {
                "state": "claimed",
                "claim_token": Uuid::new_v4().to_string(),
                "attempts": { "$add": ["$attempts", 1] },
                "next_attempt_at": { "$add": ["$$NOW", CLAIM_LIFETIME.as_millis() as i64] },
            }}],
        )
        .return_document(ReturnDocument::After)
        .await?)
}

/// Last database operation before external dispatch. A successor or expired
/// holder cannot send. A crash after this point is uncertain, even if no bytes
/// actually left the process; the receiver has no deduplication contract.
pub(super) async fn begin_send(db: &Database, intent: &SecurityEffectIntent) -> AppResult<bool> {
    let mut filter = exact_claim(intent)?;
    filter.insert("state", "claimed");
    filter.insert("$expr", doc! { "$gt": ["$next_attempt_at", "$$NOW"] });
    Ok(db
        .collection::<SecurityEffectIntent>(COLLECTION_NAME)
        .update_one(
            filter,
            vec![doc! { "$set": {
                "state": "sending",
                "next_attempt_at": { "$add": ["$$NOW", CLAIM_LIFETIME.as_millis() as i64] },
            }}],
        )
        .await?
        .modified_count
        == 1)
}

pub(super) enum Outcome {
    Delivered,
    Discard,
    Retry(&'static str),
    Failed(&'static str),
    Uncertain(&'static str),
    LostClaim,
}

async fn record_outcome(
    db: &Database,
    intent: &SecurityEffectIntent,
    outcome: Outcome,
) -> AppResult<()> {
    if matches!(outcome, Outcome::Discard)
        || (matches!(outcome, Outcome::Delivered)
            && matches!(intent.effect, SecurityEffect::Audit { .. }))
    {
        // The chained row is the audit receipt. If this delete is lost, the
        // claim expires and a replay verifies the same UUID before deleting.
        let mut filter = exact_claim(intent)?;
        filter.insert("state", doc! { "$in": ["claimed", "sending"] });
        db.collection::<SecurityEffectIntent>(COLLECTION_NAME)
            .delete_one(filter)
            .await?;
        return Ok(());
    }
    let (state, reason) = match outcome {
        Outcome::Delivered => ("delivered", "delivered"),
        Outcome::Failed(reason) => ("failed", reason),
        Outcome::Uncertain(reason) => ("uncertain", reason),
        Outcome::LostClaim | Outcome::Discard => return Ok(()),
        Outcome::Retry(reason) => {
            if matches!(intent.effect, SecurityEffect::Cae { .. })
                && intent.attempts >= CAE_MAX_ATTEMPTS
            {
                ("failed", reason)
            } else {
                ("pending", reason)
            }
        }
    };
    let mut filter = exact_claim(intent)?;
    filter.insert("state", doc! { "$in": ["claimed", "sending"] });
    let mut fields = doc! { "state": state, "outcome": reason };
    if state == "pending" {
        let delay = (RETRY_BASE_SECONDS * (1_i64 << (intent.attempts - 1).clamp(0, 10)))
            .min(RETRY_MAX_SECONDS);
        fields.insert("next_attempt_at", doc! { "$add": ["$$NOW", delay * 1000] });
    } else {
        fields.insert("completed_at", "$$NOW");
    }
    let changed = db
        .collection::<SecurityEffectIntent>(COLLECTION_NAME)
        .update_one(filter, vec![doc! { "$set": fields }])
        .await?;
    if changed.modified_count == 1 && matches!(state, "failed" | "uncertain") {
        tracing::error!(
            intent_id = %intent.id,
            effect_kind = kind_name(intent),
            state,
            reason,
            "Security effect entered a terminal delivery state"
        );
    }
    Ok(())
}

async fn abandon_uncertain(db: &Database, candidate: &SecurityEffectIntent) -> AppResult<()> {
    let mut filter = exact_claim(candidate)?;
    filter.insert("state", "sending");
    filter.insert("$expr", doc! { "$lte": ["$next_attempt_at", "$$NOW"] });
    let changed = db
        .collection::<SecurityEffectIntent>(COLLECTION_NAME)
        .update_one(
            filter,
            vec![doc! { "$set": {
                "state": "uncertain",
                "outcome": "dispatch_receipt_missing",
                "completed_at": "$$NOW",
            }}],
        )
        .await?;
    if changed.modified_count == 1 {
        tracing::error!(
            intent_id = %candidate.id,
            effect_kind = kind_name(candidate),
            state = "uncertain",
            reason = "dispatch_receipt_missing",
            "Security effect entered an uncertain delivery state"
        );
    }
    Ok(())
}

fn kind_name(intent: &SecurityEffectIntent) -> &'static str {
    match &intent.effect {
        SecurityEffect::Audit { .. } => "audit",
        SecurityEffect::Cae { .. } => "cae",
    }
}

mod diagnostics;
pub use diagnostics::{SecurityEffectDiagnostics, diagnostics};

pub async fn sweep(db: &Database, keys: &crate::crypto::aes::EncryptionKeys) -> AppResult<()> {
    sweep_with_runtime(db, keys, &super::shutdown::process()).await
}

async fn sweep_with_runtime(
    db: &Database,
    keys: &EncryptionKeys,
    runtime: &super::shutdown::Runtime,
) -> AppResult<()> {
    let db = super::transaction_service::durable_database(db);
    let candidates: Vec<Document> = db
        .collection::<Document>(COLLECTION_NAME)
        .find(due_filter())
        .hint(Hint::Name(DUE_INDEX.to_owned()))
        .sort(doc! { "next_attempt_at": 1, "_id": 1 })
        .limit(PASS_LIMIT)
        .await?
        .try_collect()
        .await?;
    for candidate in candidates {
        let Some(_work) = runtime.background() else {
            break;
        };
        let candidate: SecurityEffectIntent = match bson::from_document(candidate) {
            Ok(candidate) => candidate,
            Err(_) => {
                tracing::error!("Invalid security effect metadata; continuing pass");
                continue;
            }
        };
        if candidate.state == IntentState::Sending {
            if abandon_uncertain(&db, &candidate).await.is_err() {
                tracing::error!(intent_id = %candidate.id, "Security effect uncertainty update failed; continuing pass");
            }
            continue;
        }
        let Some(intent) = (match claim(&db, &candidate).await {
            Ok(intent) => intent,
            Err(_) => {
                tracing::error!(intent_id = %candidate.id, "Security effect claim failed; continuing pass");
                continue;
            }
        }) else {
            continue;
        };
        let outcome = match &intent.effect {
            SecurityEffect::Audit { event } => {
                match super::audit_service::append_intent(&db, event).await {
                    Ok(()) => Outcome::Delivered,
                    Err(_) => {
                        tracing::error!(intent_id = %intent.id, "Required audit append failed; will retry");
                        Outcome::Retry("audit_append_failed")
                    }
                }
            }
            SecurityEffect::Cae { .. } if intent.attempts > CAE_MAX_ATTEMPTS => {
                Outcome::Failed("attempt_limit_reached")
            }
            SecurityEffect::Cae { .. } => {
                super::cae_webhook_service::deliver_intent(&db, keys, &intent)
                    .await
                    .unwrap_or(Outcome::Retry("preparation_unavailable"))
            }
        };
        if record_outcome(&db, &intent, outcome).await.is_err() {
            tracing::error!(intent_id = %intent.id, "Security effect outcome update failed; continuing pass");
        }
    }
    Ok(())
}

pub fn start(db: Database, keys: Arc<EncryptionKeys>) {
    super::shutdown::process().spawn_worker(async move {
        let mut interval = tokio::time::interval(POLL_INTERVAL);
        while let Some(_work) = super::shutdown::process().tick(&mut interval).await {
            if sweep(&db, &keys).await.is_err() {
                // Never log intent contents or database/provider text.
                tracing::error!("Security effect relay pass failed; will retry");
            }
        }
    });
}

#[cfg(test)]
mod tests;
