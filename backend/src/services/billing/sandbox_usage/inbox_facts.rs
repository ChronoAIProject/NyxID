use super::contract::{Event, Fact, Payload};
use super::inbox::{InboxError, decode_stored};
use super::validation::{has_compute, has_home};
use crate::models::sandbox_usage_inbox::{
    EFFECTIVE_FACTS, EVENTS, EffectiveFact, Event as StoredEvent,
};
use mongodb::{
    ClientSession, Collection, Database,
    bson::{Document, doc},
};
use uuid::Uuid;

pub(super) async fn apply(
    db: &Database,
    session: &mut ClientSession,
    event: &Event,
    begin: i64,
    previous_end: i64,
) -> Result<i64, InboxError> {
    let facts = db.collection::<EffectiveFact>(EFFECTIVE_FACTS);
    let header = event.header();
    let namespace = header.producer_namespace.as_str();
    let stream = header.stream_id.as_str();
    let event_id = header.event_id.as_str();
    let owner = header.owner_ref.subject.as_str();
    match event.payload() {
        Payload::Usage { facts: additions } => {
            let mut end = previous_end;
            for fact in additions {
                let row = fact_row(fact, namespace, stream, event_id, owner);
                if row.start_ms < begin {
                    return Err(InboxError::Conflict("fact_before_capture"));
                }
                let original = facts
                    .find_one(
                        doc! {"namespace":namespace, "stream_id":stream, "fact_id":&row.fact_id},
                    )
                    .session(&mut *session)
                    .await?;
                if original.is_some() {
                    return Err(InboxError::Conflict("fact_already_exists"));
                }
                ensure_nonoverlapping(&facts, session, &row).await?;
                end = end.max(row.end_ms);
                facts.insert_one(row).session(&mut *session).await?;
            }
            Ok(end)
        }
        Payload::Correction { correction } => {
            let row = facts
                .find_one(doc! {"namespace":namespace, "stream_id":stream, "fact_id":correction.fact_id.as_str()})
                .session(&mut *session)
                .await?
                .ok_or(InboxError::Prerequisite("predecessor_missing"))?;
            if row.source_event_id != correction.expected_predecessor_event_id.as_str() {
                return Err(InboxError::Conflict("fact_head_changed"));
            }
            let accepted = db
                .collection::<StoredEvent>(EVENTS)
                .find_one(doc! {"namespace":namespace, "stream_id":stream, "source_event_id":&row.source_event_id})
                .session(&mut *session)
                .await?
                .ok_or(InboxError::Corrupt)?;
            let predecessor = decode_stored(&accepted)?;
            event
                .validate_correction_of(&predecessor)
                .map_err(InboxError::Conflict)?;
            let mut replacement =
                fact_row(&correction.replacement, namespace, stream, event_id, owner);
            replacement.id = row.id;
            ensure_nonoverlapping(&facts, session, &replacement).await?;
            let outcome = facts
                .replace_one(
                    doc! {"_id":&replacement.id, "source_event_id":&row.source_event_id},
                    &replacement,
                )
                .session(&mut *session)
                .await?;
            if outcome.matched_count != 1 {
                return Err(InboxError::Conflict("fact_head_changed"));
            }
            Ok(previous_end)
        }
        Payload::StreamSeal { seal } => {
            if (seal.sealed_at_ms.get() as i64) < begin.max(previous_end) {
                return Err(InboxError::Conflict("seal_before_fact_end"));
            }
            Ok(previous_end)
        }
        Payload::StreamOpen { .. } => Ok(previous_end),
    }
}

fn fact_row(fact: &Fact, namespace: &str, stream: &str, event: &str, owner: &str) -> EffectiveFact {
    let resource = &fact.resource;
    let (runtime_id, runtime_generation) = if has_compute(fact) {
        (
            resource.runtime_id.as_ref().map(|id| id.as_str().into()),
            resource.runtime_generation.map(|v| v.get() as i64),
        )
    } else {
        (None, None)
    };
    let (home_id, home_generation) = if has_home(fact) {
        (
            resource.home_id.as_ref().map(|id| id.as_str().into()),
            resource.home_generation.as_ref().map(|v| v.as_str().into()),
        )
    } else {
        (None, None)
    };
    EffectiveFact {
        id: Uuid::new_v4().to_string(),
        namespace: namespace.into(),
        stream_id: stream.into(),
        owner_subject: owner.into(),
        fact_id: fact.fact_id.as_str().into(),
        source_event_id: event.into(),
        bucket_start_ms: fact.bucket_start_ms.get() as i64,
        start_ms: fact.start_ms.get() as i64,
        end_ms: fact.end_ms.get() as i64,
        runtime_id,
        runtime_generation,
        home_id,
        home_generation,
    }
}

async fn ensure_nonoverlapping(
    facts: &Collection<EffectiveFact>,
    session: &mut ClientSession,
    row: &EffectiveFact,
) -> Result<(), InboxError> {
    let mut bindings = Vec::<Document>::new();
    if let (Some(id), Some(generation)) = (&row.runtime_id, row.runtime_generation) {
        bindings.push(doc! {"runtime_id":id, "runtime_generation":generation});
    }
    if let (Some(id), Some(generation)) = (&row.home_id, &row.home_generation) {
        bindings.push(doc! {"home_id":id, "home_generation":generation});
    }
    if bindings.is_empty() {
        return Ok(());
    }
    let overlap = facts
        .find_one(doc! {
            "namespace": &row.namespace,
            "stream_id": &row.stream_id,
            "_id": {"$ne": &row.id},
            "bucket_start_ms": row.bucket_start_ms,
            "start_ms": {"$lt": row.end_ms},
            "end_ms": {"$gt": row.start_ms},
            "$or": bindings,
        })
        .session(session)
        .await?;
    if overlap.is_some() {
        return Err(InboxError::Conflict("overlapping_resource_facts"));
    }
    Ok(())
}
