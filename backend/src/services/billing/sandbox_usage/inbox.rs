use super::contract::{Basis, Clock, Event, Payload, Receipt, decode_event};
use super::inbox_authority::{Authority, WRITE_SCOPE};
use super::inbox_cursor::{self, CursorPage};
use super::inbox_facts;
use super::inbox_promotion::promote_stream;
use crate::models::sandbox_usage_inbox::{
    AcceptedSequence, EFFECTIVE_FACTS, EVENTS, Event as StoredEvent, OWNER_PINS, PRODUCERS,
    Producer, STREAMS, Stream, Validation,
};
use bson::{Binary, Document, doc, spec::BinarySubtype};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    options::{Acknowledgment, IndexOptions, ReadConcern, WriteConcern},
};
use uuid::Uuid;

const MAX_ATTEMPTS: usize = 4;
const MAX_EVENT_BYTES: usize = 512 * 1024;

#[derive(Debug, thiserror::Error)]
pub enum InboxError {
    #[error("invalid evidence: {0}")]
    Invalid(&'static str),
    #[error("conflicting evidence: {0}")]
    Conflict(&'static str),
    #[error("missing prerequisite: {0}")]
    Prerequisite(&'static str),
    #[error("usage producer is unauthorized")]
    Forbidden,
    #[error("usage custody is unavailable")]
    Unavailable,
    #[error("stored usage evidence is inconsistent")]
    Corrupt,
    #[error("usage custody store is unavailable")]
    Database(#[from] mongodb::error::Error),
}

pub(super) fn clock() -> Clock {
    Clock {
        now_ms: Utc::now().timestamp_millis() as u64,
        allowed_clock_skew_ms: 30_000,
    }
}

pub struct ReceiptObservation {
    pub receipt: Receipt,
    pub validation: Validation,
    pub validation_reason: Option<String>,
}

fn index_definitions() -> [(&'static str, Vec<(Document, bool)>); 5] {
    [
        (PRODUCERS, vec![(doc! {"namespace":1}, true)]),
        (
            EVENTS,
            vec![
                (doc! {"namespace":1, "source_event_id":1}, true),
                (doc! {"namespace":1, "stream_id":1, "sequence":1}, true),
                (doc! {"owner_subject":1}, false),
            ],
        ),
        (
            STREAMS,
            vec![
                (doc! {"namespace":1, "stream_id":1}, true),
                (
                    doc! {"namespace":1, "sandbox_id":1, "source_epoch":1, "basis":1},
                    true,
                ),
                (doc! {"owner_subject":1}, false),
                (doc! {"promotion_ready":1, "_id":1}, false),
            ],
        ),
        (
            EFFECTIVE_FACTS,
            vec![
                (doc! {"namespace":1, "stream_id":1, "fact_id":1}, true),
                (
                    doc! {"namespace":1, "stream_id":1, "bucket_start_ms":1},
                    false,
                ),
                (doc! {"owner_subject":1}, false),
            ],
        ),
        (
            OWNER_PINS,
            vec![
                (doc! {"namespace":1,"sandbox_id":1,"source_epoch":1}, true),
                (doc! {"owner_id":1}, false),
            ],
        ),
    ]
}

pub async fn prepare(db: &Database) -> Result<(), InboxError> {
    for (collection, indexes) in index_definitions() {
        let models = indexes.into_iter().map(|(keys, unique)| {
            IndexModel::builder()
                .keys(keys)
                .options(IndexOptions::builder().unique(unique).build())
                .build()
        });
        db.collection::<Document>(collection)
            .create_indexes(models)
            .await?;
    }
    Ok(())
}

pub(super) async fn require_prepared(db: &Database) -> Result<(), InboxError> {
    for (collection, mut expected) in index_definitions() {
        let collection = db.collection::<Document>(collection);
        let mut indexes = match collection.list_indexes().await {
            Ok(indexes) => indexes,
            Err(error)
                if matches!(error.kind.as_ref(),
                mongodb::error::ErrorKind::Command(command) if command.code == 26) =>
            {
                return Err(InboxError::Unavailable);
            }
            Err(error) => return Err(error.into()),
        };
        while let Some(index) = indexes.try_next().await? {
            let options = index.options.unwrap_or_default();
            if options.expire_after.is_some() {
                return Err(InboxError::Unavailable);
            }
            if options.sparse.unwrap_or_default() || options.partial_filter_expression.is_some() {
                continue;
            }
            expected.retain(|(keys, unique)| {
                *keys != index.keys || *unique != options.unique.unwrap_or_default()
            });
        }
        if !expected.is_empty() {
            return Err(InboxError::Unavailable);
        }
    }
    Ok(())
}

pub async fn accept(
    db: &Database,
    authority: &Authority,
    bytes: &[u8],
) -> Result<(bool, ReceiptObservation), InboxError> {
    authority.require_scope(WRITE_SCOPE)?;
    if bytes.len() > MAX_EVENT_BYTES {
        return Err(InboxError::Invalid("event_size"));
    }
    let event = decode_event(bytes, clock()).map_err(InboxError::Invalid)?;
    let header = event.header();
    if header.producer_namespace.as_str() != authority.namespace()
        || header.owner_ref.issuer.as_str() != authority.owner_issuer()
    {
        return Err(InboxError::Forbidden);
    }
    if !header.owner_ref.subject.as_str().bytes().all(|byte| {
        byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b':' | b'.' | b'|')
    }) {
        return Err(InboxError::Invalid("nonopaque_owner_reference"));
    }
    for _ in 0..MAX_ATTEMPTS {
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .read_concern(ReadConcern::snapshot())
            .write_concern(WriteConcern::builder().w(Acknowledgment::Majority).build())
            .await?;
        let result = async {
            authority.fence(db, &mut session).await?;
            let result = append(db, &mut session, &event, authority.accepting()).await?;
            commit(&mut session).await?;
            Ok(result)
        }
        .await;
        match result {
            Ok((created, observation, promotion_ready)) => {
                if promotion_ready
                    && let Err(error) =
                        promote_stream(db, authority.namespace(), header.stream_id.as_str()).await
                {
                    tracing::warn!(error = %error, "Usage custody validation remains pending");
                }
                return Ok((created, observation));
            }
            Err(error) => {
                if matches!(&error, InboxError::Database(error) if error.contains_label("UnknownTransactionCommitResult"))
                {
                    // Check the exact accepted event through a majority read. A
                    // missing row leaves an uncertain outcome for the sender to recover.
                    return match receipt_in_namespace(
                        db,
                        authority.namespace(),
                        header.event_id.as_str(),
                    )
                    .await?
                    {
                        Some(receipt) if receipt.receipt.matches(&event) => Ok((false, receipt)),
                        Some(_) => Err(InboxError::Conflict("event_digest_changed")),
                        None => Err(InboxError::Unavailable),
                    };
                }
                let _ = session.abort_transaction().await;
                if !matches!(&error, InboxError::Database(error) if retryable(error)) {
                    return Err(error);
                }
            }
        }
    }
    Err(InboxError::Unavailable)
}

fn retryable(error: &mongodb::error::Error) -> bool {
    error.contains_label("TransientTransactionError")
        || super::super::ledger::is_duplicate_key_error(error)
}

pub(super) async fn commit(session: &mut ClientSession) -> Result<(), InboxError> {
    for _ in 0..MAX_ATTEMPTS - 1 {
        match session.commit_transaction().await {
            Ok(()) => return Ok(()),
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => continue,
            Err(error) => return Err(error.into()),
        }
    }
    session.commit_transaction().await?;
    Ok(())
}

async fn append(
    db: &Database,
    session: &mut ClientSession,
    event: &Event,
    accepting: bool,
) -> Result<(bool, ReceiptObservation, bool), InboxError> {
    let header = event.header();
    let namespace = header.producer_namespace.as_str();
    let stream = header.stream_id.as_str();
    let event_id = header.event_id.as_str();
    let sequence = header.sequence.get() as i64;
    let events = db.collection::<StoredEvent>(EVENTS);
    let streams = db.collection::<Stream>(STREAMS);

    if let Some(row) = events
        .find_one(doc! {"namespace": namespace, "source_event_id":event_id})
        .session(&mut *session)
        .await?
    {
        if row.digest != event.digest().as_str() || row.canonical.bytes != event.canonical_bytes() {
            return Err(InboxError::Conflict("event_digest_changed"));
        }
        return Ok((false, receipt_observation(&row)?, false));
    }
    if !accepting {
        return Err(InboxError::Unavailable);
    }
    let existing = streams
        .find_one(doc! {"namespace":namespace, "stream_id":stream})
        .session(&mut *session)
        .await?;
    let row_id = Uuid::new_v4().to_string();
    let mut validation = Validation::Validated;
    let mut promotion_ready = false;
    match existing {
        Some(mut current) => {
            if sequence <= current.sequence
                || current.pending.iter().any(|item| item.sequence == sequence)
            {
                return Err(InboxError::Conflict("sequence_already_used"));
            }
            let previous = events
                .find_one(doc! {"_id":&current.head_id, "namespace":namespace, "stream_id":stream})
                .session(&mut *session)
                .await?
                .ok_or(InboxError::Corrupt)?;
            let head = decode_stored(&previous)?;
            if previous.digest != current.head_digest || previous.sequence != current.sequence {
                return Err(InboxError::Corrupt);
            }
            let old_head = current.head_id.clone();
            if current.sequence.checked_add(1) == Some(sequence) {
                advance_head(db, session, &mut current, event, &head, &row_id).await?;
            } else {
                if !matches!(event.payload(), Payload::Usage { .. })
                    || sequence.saturating_sub(current.sequence)
                        > inbox_cursor::MAX_PENDING_DISTANCE
                {
                    return Err(InboxError::Prerequisite("predecessor_missing"));
                }
                event
                    .validate_stream_of(&head)
                    .map_err(InboxError::Conflict)?;
                validation = Validation::Pending;
                current.pending.push(AcceptedSequence {
                    sequence,
                    digest: event.digest().as_str().into(),
                });
                current.pending.sort_unstable_by_key(|item| item.sequence);
            }
            promotion_ready = current.promotion_ready;
            let updated = streams
                .replace_one(doc! {"_id":&current.id, "head_id":old_head}, current)
                .session(&mut *session)
                .await?;
            if updated.matched_count != 1 {
                return Err(InboxError::Unavailable);
            }
        }
        None => {
            let Payload::StreamOpen { opening } = event.payload() else {
                return Err(InboxError::Prerequisite("stream_not_open"));
            };
            let basis = match header.basis {
                Basis::Allocated => "allocated",
                Basis::Measured => "measured",
            };
            let key = doc! {"namespace":namespace, "sandbox_id":header.sandbox_id.as_str(),
            "source_epoch":header.source_epoch.as_str(), "basis":basis};
            if streams
                .find_one(key)
                .session(&mut *session)
                .await?
                .is_some()
            {
                return Err(InboxError::Conflict("source_stream_already_exists"));
            }
            let capture_start_ms = opening.capture_start_ms.get() as i64;
            let initial = Stream {
                id: Uuid::new_v4().to_string(),
                namespace: namespace.into(),
                stream_id: stream.into(),
                owner_subject: header.owner_ref.subject.as_str().into(),
                sandbox_id: header.sandbox_id.as_str().into(),
                source_epoch: header.source_epoch.as_str().into(),
                basis: basis.into(),
                capture_start_ms,
                last_fact_end_ms: capture_start_ms,
                sequence,
                head_id: row_id.clone(),
                head_digest: event.digest().as_str().into(),
                pending: Vec::new(),
                promotion_ready: false,
            };
            streams.insert_one(initial).session(&mut *session).await?;
        }
    }
    let accepted_at_ms = clock().now_ms.max(header.produced_at_ms.get());
    let receipt_id = Uuid::new_v4().to_string();
    let row = StoredEvent {
        id: row_id,
        namespace: namespace.into(),
        source_event_id: event_id.into(),
        stream_id: stream.into(),
        owner_subject: header.owner_ref.subject.as_str().into(),
        sequence,
        canonical: Binary {
            subtype: BinarySubtype::Generic,
            bytes: event.canonical_bytes().to_vec(),
        },
        digest: event.digest().as_str().into(),
        receipt_id: receipt_id.clone(),
        validation,
        validation_reason: None,
        accepted_at: DateTime::from_timestamp_millis(accepted_at_ms as i64)
            .ok_or(InboxError::Corrupt)?,
    };
    events.insert_one(row).session(&mut *session).await?;
    Ok((
        true,
        ReceiptObservation {
            receipt: event.custody_receipt(receipt_id, accepted_at_ms),
            validation,
            validation_reason: None,
        },
        promotion_ready,
    ))
}

pub(super) async fn advance_head(
    db: &Database,
    session: &mut ClientSession,
    stream: &mut Stream,
    event: &Event,
    head: &Event,
    row_id: &str,
) -> Result<(), InboxError> {
    event
        .validate_successor_of(head)
        .map_err(InboxError::Conflict)?;
    stream.last_fact_end_ms = inbox_facts::apply(
        db,
        session,
        event,
        stream.capture_start_ms,
        stream.last_fact_end_ms,
    )
    .await?;
    stream.sequence = event.header().sequence.get() as i64;
    stream.head_id = row_id.into();
    stream.head_digest = event.digest().as_str().into();
    stream.promotion_ready = stream
        .pending
        .first()
        .is_some_and(|next| stream.sequence.checked_add(1) == Some(next.sequence));
    Ok(())
}

pub(super) fn decode_stored(row: &StoredEvent) -> Result<Event, InboxError> {
    let accepted_at =
        u64::try_from(row.accepted_at.timestamp_millis()).map_err(|_| InboxError::Corrupt)?;
    let event = decode_event(
        &row.canonical.bytes,
        Clock {
            now_ms: accepted_at,
            allowed_clock_skew_ms: 0,
        },
    )
    .map_err(|_| InboxError::Corrupt)?;
    let header = event.header();
    if row.digest != event.digest().as_str()
        || row.source_event_id != header.event_id.as_str()
        || row.namespace != header.producer_namespace.as_str()
        || row.stream_id != header.stream_id.as_str()
        || row.owner_subject != header.owner_ref.subject.as_str()
        || row.sequence != header.sequence.get() as i64
    {
        return Err(InboxError::Corrupt);
    }
    Ok(event)
}

fn stored_receipt(row: &StoredEvent) -> Result<Receipt, InboxError> {
    let event = decode_stored(row)?;
    if Uuid::parse_str(&row.receipt_id).ok().is_none_or(|id| {
        id.get_version() != Some(uuid::Version::Random)
            || id.get_variant() != uuid::Variant::RFC4122
            || id.to_string() != row.receipt_id
    }) {
        return Err(InboxError::Corrupt);
    }
    Ok(event.custody_receipt(
        row.receipt_id.clone(),
        row.accepted_at.timestamp_millis() as u64,
    ))
}

fn receipt_observation(row: &StoredEvent) -> Result<ReceiptObservation, InboxError> {
    Ok(ReceiptObservation {
        receipt: stored_receipt(row)?,
        validation: row.validation,
        validation_reason: row.validation_reason.clone(),
    })
}

pub async fn receipt(
    db: &Database,
    authority: &Authority,
    event_id: &str,
) -> Result<Option<ReceiptObservation>, InboxError> {
    authority.authorize_read(db).await?;
    receipt_in_namespace(db, authority.namespace(), event_id).await
}

async fn receipt_in_namespace(
    db: &Database,
    namespace: &str,
    event_id: &str,
) -> Result<Option<ReceiptObservation>, InboxError> {
    let row = db
        .collection::<StoredEvent>(EVENTS)
        .find_one(doc! {"namespace":namespace, "source_event_id":event_id})
        .read_concern(ReadConcern::majority())
        .await?;
    row.as_ref().map(receipt_observation).transpose()
}

pub async fn cursor(
    db: &Database,
    authority: &Authority,
    stream_id: &str,
    page: Option<&str>,
) -> Result<Option<CursorPage>, InboxError> {
    authority.authorize_read(db).await?;
    let stream = db
        .collection::<Stream>(STREAMS)
        .find_one(doc! {"namespace":authority.namespace(), "stream_id":stream_id})
        .read_concern(ReadConcern::majority())
        .await?;
    stream
        .map(|stream| inbox_cursor::snapshot(stream, page))
        .transpose()
}

pub async fn producer(db: &Database, actor_id: &str) -> Result<Producer, InboxError> {
    db.collection::<Producer>(PRODUCERS)
        .find_one(doc! {"_id":actor_id, "enabled":true})
        .read_concern(ReadConcern::majority())
        .await?
        .ok_or(InboxError::Forbidden)
}
