use super::{
    completeness_contract::{Basis, Boundary, FactRef, MAX_ITEMS, Number, Position, Statement},
    completeness_evidence::{self, Evidence},
    contract::{self, Event, Payload},
    inbox::{self, InboxError},
};
use crate::models::sandbox_usage_inbox::{
    EFFECTIVE_FACTS, EVENTS, EffectiveFact, Event as StoredEvent, STREAMS, Stream,
};
use mongodb::{ClientSession, Database, bson::doc};
use serde::Serialize;

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "status", content = "reason", rename_all = "snake_case")]
pub enum Decision {
    Pending(&'static str),
    Validated,
    Superseded(&'static str),
    Rejected(&'static str),
}

/// At a single inbox snapshot, establish exact validated stream positions and
/// compare both effective sets. A later change to this minute immediately
/// supersedes the statement, even before a replacement statement arrives.
pub async fn observe(
    db: &Database,
    session: &mut ClientSession,
    statement: &Statement,
) -> Result<Decision, InboxError> {
    if statement.producer_hold.is_some() {
        return Ok(Decision::Pending("producer_hold"));
    }
    let mut allocated = Vec::new();
    let mut measured = Vec::new();
    for position in &statement.positions {
        let evidence = match read_position(db, session, statement, position).await? {
            Ok(evidence) => evidence,
            Err(decision) => return Ok(decision),
        };
        match position.basis {
            Basis::Allocated => allocated = evidence,
            Basis::Measured => measured = evidence,
        }
    }
    let target = match statement.basis {
        Basis::Allocated => &allocated,
        Basis::Measured => &measured,
    };
    Ok(
        match completeness_evidence::verify(statement, &allocated, target) {
            Ok(()) => Decision::Validated,
            Err(reason) => Decision::Rejected(reason),
        },
    )
}

async fn read_position(
    db: &Database,
    session: &mut ClientSession,
    statement: &Statement,
    position: &Position,
) -> Result<Result<Vec<Evidence>, Decision>, InboxError> {
    let basis = match position.basis {
        Basis::Allocated => "allocated",
        Basis::Measured => "measured",
    };
    let Some(stream) = db
        .collection::<Stream>(STREAMS)
        .find_one(doc! {
            "namespace":&statement.producer_namespace,"sandbox_id":&statement.sandbox_id,
            "source_epoch":&statement.source_epoch,"basis":basis,
        })
        .session(&mut *session)
        .await?
    else {
        return Ok(Err(Decision::Pending("stream_missing")));
    };
    if stream.stream_id != position.stream_id || stream.owner_subject != statement.owner.subject {
        return Ok(Err(Decision::Rejected("stream_binding_changed")));
    }
    let sequence = i64::try_from(position.sequence.0).map_err(|_| InboxError::Corrupt)?;
    if stream.sequence < sequence {
        return Ok(Err(Decision::Pending("position_not_validated")));
    }
    let Some(head) = db
        .collection::<StoredEvent>(EVENTS)
        .find_one(doc! {
            "namespace":&statement.producer_namespace,"stream_id":&position.stream_id,
            "sequence":sequence,"validation":"validated",
        })
        .session(&mut *session)
        .await?
    else {
        return Ok(Err(Decision::Rejected("position_missing")));
    };
    let event = inbox::decode_stored(&head)?;
    if head.digest != position.digest || !matches_identity(statement, position, &event) {
        return Ok(Err(Decision::Rejected("position_conflict")));
    }
    if position.basis == Basis::Allocated {
        let Some(open) = db
            .collection::<StoredEvent>(EVENTS)
            .find_one(doc! {
                "namespace":&statement.producer_namespace,"stream_id":&position.stream_id,
                "sequence":1_i64,"validation":"validated",
            })
            .session(&mut *session)
            .await?
        else {
            return Err(InboxError::Corrupt);
        };
        let opening_event = inbox::decode_stored(&open)?;
        let Payload::StreamOpen { opening } = opening_event.payload() else {
            return Err(InboxError::Corrupt);
        };
        if !matches_identity(statement, position, &opening_event)
            || opening.capture_start_ms.get() > statement.scope.start_ms.0
            || opening.historical_missing_ranges.iter().any(|gap| {
                gap.start_ms.get() < statement.scope.end_ms.0
                    && gap.end_ms.get() > statement.scope.start_ms.0
            })
            || (statement.start_boundary == Boundary::Birth
                && (!opening.complete_lifecycle
                    || opening.capture_start_ms.get() != statement.scope.start_ms.0))
            || statement.start_boundary == Boundary::EpochStart
            || (matches!(
                statement.end_boundary,
                Boundary::Retirement | Boundary::EpochEnd
            ) && statement.capture_through_ms != statement.scope.end_ms)
        {
            return Ok(Err(Decision::Rejected("lifecycle_boundary_unproved")));
        }
    }
    let mut cursor = db
        .collection::<EffectiveFact>(EFFECTIVE_FACTS)
        .find(doc! {
            "namespace":&statement.producer_namespace,"stream_id":&position.stream_id,
            "bucket_start_ms":statement.minute_start_ms.0 as i64,
        })
        .limit(MAX_ITEMS as i64 + 1)
        .sort(doc! {"fact_id":1})
        .session(&mut *session)
        .await?;
    let mut evidence = Vec::new();
    while let Some(record) = cursor.next(&mut *session).await.transpose()? {
        if evidence.len() >= MAX_ITEMS {
            return Ok(Err(Decision::Pending("effective_fact_bound")));
        }
        let row = db
            .collection::<StoredEvent>(EVENTS)
            .find_one(doc! {
                "namespace":&statement.producer_namespace,"stream_id":&position.stream_id,
                "source_event_id":&record.source_event_id,"validation":"validated",
            })
            .session(&mut *session)
            .await?
            .ok_or(InboxError::Corrupt)?;
        if row.sequence > sequence {
            return Ok(Err(Decision::Superseded("effective_fact_advanced")));
        }
        let event = inbox::decode_stored(&row)?;
        if !matches_identity(statement, position, &event) {
            return Err(InboxError::Corrupt);
        }
        let fact = match event.payload() {
            Payload::Usage { facts } => facts
                .iter()
                .find(|fact| fact.fact_id.as_str() == record.fact_id),
            Payload::Correction { correction } if correction.fact_id.as_str() == record.fact_id => {
                Some(&correction.replacement)
            }
            _ => None,
        }
        .ok_or(InboxError::Corrupt)?;
        let source_revision = event
            .header()
            .source_refs
            .iter()
            .find(|source| source.kind.as_str() == "lifecycle")
            .map(|source| source.revision.get())
            .unwrap_or(0);
        evidence.push(Evidence {
            reference: FactRef {
                stream_id: position.stream_id.clone(),
                fact_id: record.fact_id,
                event_id: row.source_event_id,
                event_digest: row.digest,
                evidence_revision: Number(fact.evidence_revision.get()),
            },
            source_digest: event.header().source_digest.as_str().into(),
            source_revision,
            fact: fact.clone(),
        });
    }
    Ok(Ok(evidence))
}

fn matches_identity(statement: &Statement, position: &Position, event: &Event) -> bool {
    let header = event.header();
    header.producer_namespace.as_str() == statement.producer_namespace
        && header.stream_id.as_str() == position.stream_id
        && header.sandbox_id.as_str() == statement.sandbox_id
        && header.source_epoch.as_str() == statement.source_epoch
        && header.owner_ref.subject.as_str() == statement.owner.subject
        && header.owner_ref.issuer.as_str() == statement.owner.issuer
        && header.owner_ref.kind.as_str() == statement.owner.kind
        && matches!(
            (header.basis, position.basis),
            (contract::Basis::Allocated, Basis::Allocated)
                | (contract::Basis::Measured, Basis::Measured)
        )
}
