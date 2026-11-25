//! Durable, permanently shadow-only estimates for the evidence currently in custody.
use super::{
    contract::{Event, Fact, Metric, OwnerRef, Payload},
    inbox, inbox_shadow, policy, price_pins, pricing, pricing_schema,
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        credits::Credits,
        sandbox_usage_inbox::{
            EFFECTIVE_FACTS, EVENTS, EffectiveFact, Event as StoredEvent, HistoricalOwner,
            OWNER_PINS, STREAMS, Stream,
        },
        sandbox_usage_rates::{
            BUCKET_REVISIONS, COLLECTION_NAME as TARIFFS, ChargeMode, ShadowBucketInput,
            ShadowBucketMetric, ShadowBucketRevision, ShadowFactRef, ShadowStreamPosition,
            UsageScope, UsageTariff,
        },
    },
};
use bson::{DateTime as BsonDateTime, doc};
use chrono::{DateTime, Utc};
use mongodb::{ClientSession, Database, options::ReadConcern};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

const FACT_LIMIT: i64 = 1024;
const ROUNDING: &str = "sum_exact_per_metric_then_half_even_pico_v1";

pub struct Bucket<'a> {
    pub namespace: &'a str,
    pub sandbox_id: &'a str,
    pub source_epoch: &'a str,
    pub minute: DateTime<Utc>,
}

impl Bucket<'_> {
    fn stream_filter(&self) -> bson::Document {
        doc! {"namespace":self.namespace,"sandbox_id":self.sandbox_id,"source_epoch":self.source_epoch}
    }
    fn id(&self) -> AppResult<String> {
        let bytes = serde_json::to_vec(&(
            self.namespace,
            self.sandbox_id,
            self.source_epoch,
            self.minute.timestamp_millis(),
        ))
        .map_err(|_| corrupt())?;
        Ok(hex::encode(Sha256::digest(bytes)))
    }
}

pub async fn observe(
    db: &Database,
    issuer: &str,
    bucket: Bucket<'_>,
) -> AppResult<Option<ShadowBucketRevision>> {
    pricing_schema::require_enabled(db).await?;
    policy::minute(bucket.minute)?;
    policy::text(bucket.namespace)?;
    policy::text(bucket.sandbox_id)?;
    let epoch =
        Uuid::parse_str(bucket.source_epoch).map_err(|_| policy::invalid("Invalid usage epoch"))?;
    if epoch.to_string() != bucket.source_epoch
        || epoch.get_version() != Some(uuid::Version::Random)
        || epoch.get_variant() != uuid::Variant::RFC4122
    {
        return Err(policy::invalid("Invalid usage epoch"));
    }
    if bucket.minute > Utc::now() {
        return Err(policy::invalid("Cannot observe future usage"));
    }
    let Some(stream) = db
        .collection::<Stream>(STREAMS)
        .find_one(bucket.stream_filter())
        .read_concern(ReadConcern::majority())
        .await?
    else {
        return Ok(None);
    };
    let head = db
        .collection::<StoredEvent>(EVENTS)
        .find_one(doc! {"_id":&stream.head_id,"validation":"validated"})
        .read_concern(ReadConcern::majority())
        .await?
        .ok_or_else(corrupt)?;
    let event = inbox::decode_stored(&head).map_err(|_| corrupt())?;
    let source = &event.header().owner_ref;
    inbox_shadow::historical_owner(db, issuer, &event).await?;
    for _ in 0..8 {
        let mut session = match pricing_schema::begin(db).await {
            Ok(session) => session,
            Err(AppError::DatabaseError(ref error))
                if error.contains_label("TransientTransactionError") =>
            {
                continue;
            }
            Err(error) => return Err(error),
        };
        let result = observe_in_session(db, &mut session, &bucket, source).await;
        match result {
            Ok(revision) => match pricing_schema::commit(&mut session).await {
                Ok(()) => return Ok(Some(revision)),
                Err(AppError::DatabaseError(ref error))
                    if error.contains_label("TransientTransactionError") =>
                {
                    continue;
                }
                Err(error) => return Err(error),
            },
            Err(error) => {
                let _ = session.abort_transaction().await;
                if matches!(&error, AppError::DatabaseError(error) if error.contains_label("TransientTransactionError"))
                {
                    continue;
                }
                return Err(error);
            }
        }
    }
    Err(policy::unavailable(
        "Usage bucket contention; retry the same minute",
    ))
}

async fn observe_in_session(
    db: &Database,
    session: &mut ClientSession,
    bucket: &Bucket<'_>,
    source: &OwnerRef,
) -> AppResult<ShadowBucketRevision> {
    let owner_subject = source.subject.as_str();
    let mut streams = db
        .collection::<Stream>(STREAMS)
        .find(bucket.stream_filter())
        .sort(doc! {"basis":1,"stream_id":1})
        .limit(3)
        .session(&mut *session)
        .await?;
    let mut positions = Vec::new();
    while let Some(row) = streams.next(&mut *session).await.transpose()? {
        if row.owner_subject != owner_subject || positions.len() == 2 {
            return Err(corrupt());
        }
        let head = db.collection::<StoredEvent>(EVENTS)
            .find_one(doc! {"_id":&row.head_id,"namespace":bucket.namespace,"stream_id":&row.stream_id,"validation":"validated","sequence":row.sequence,"digest":&row.head_digest})
            .session(&mut *session).await?.ok_or_else(corrupt)?;
        let head_event = inbox::decode_stored(&head).map_err(|_| corrupt())?;
        if &head_event.header().owner_ref != source
            || head_event.header().sandbox_id.as_str() != bucket.sandbox_id
            || head_event.header().source_epoch.as_str() != bucket.source_epoch
        {
            return Err(corrupt());
        }
        positions.push(ShadowStreamPosition {
            basis: row.basis,
            stream_id: row.stream_id,
            validated_sequence: row.sequence,
            validated_digest: row.head_digest,
        });
    }
    if positions.is_empty() {
        return Err(corrupt());
    }
    let historical_owner = db
        .collection::<HistoricalOwner>(OWNER_PINS)
        .find_one(bucket.stream_filter())
        .session(&mut *session)
        .await?;
    let pin = match historical_owner {
        Some(owner) => {
            if owner.owner_issuer != source.issuer.as_str()
                || owner.owner_subject != owner_subject
                || owner.owner_id != owner_subject
            {
                return Err(corrupt());
            }
            let scope = UsageScope {
                producer_namespace: owner.namespace,
                sandbox_id: owner.sandbox_id,
                source_epoch: owner.source_epoch,
                owner_id: owner.owner_id,
            };
            Some(
                price_pins::pin_in_session(
                    db,
                    session,
                    &scope,
                    &policy::scope_key(&scope)?,
                    bucket.minute,
                )
                .await?,
            )
        }
        None => None,
    };
    let tariff = match pin.as_ref().and_then(|pin| pin.tariff_id.as_ref()) {
        Some(id) => Some(
            db.collection::<UsageTariff>(TARIFFS)
                .find_one(doc! {"_id":id})
                .session(&mut *session)
                .await?
                .ok_or_else(corrupt)?,
        ),
        None => None,
    };
    let stream_ids = positions
        .iter()
        .map(|position| position.stream_id.clone())
        .collect::<Vec<_>>();
    let mut cursor = db.collection::<EffectiveFact>(EFFECTIVE_FACTS)
        .find(doc! {"namespace":bucket.namespace,"stream_id":{"$in":stream_ids},"bucket_start_ms":bucket.minute.timestamp_millis()})
        .sort(doc! {"stream_id":1,"fact_id":1}).limit(FACT_LIMIT+1).session(&mut *session).await?;
    let mut refs = Vec::new();
    let mut fractions = BTreeMap::<Metric, (u128, u128)>::new();
    // Contiguous custody and observed resource facts alone do not certify completeness.
    let mut unresolved = BTreeSet::from([
        "minute_completeness_unproven".to_string(),
        "legacy_cutover_unverified".to_string(),
    ]);
    if pin.is_none() {
        unresolved.insert("owner_unresolved".into());
    }
    while let Some(stored) = cursor.next(&mut *session).await.transpose()? {
        if refs.len() >= FACT_LIMIT as usize {
            return Err(policy::unavailable(
                "Usage shadow bucket exceeds the evidence bound",
            ));
        }
        let row = db.collection::<StoredEvent>(EVENTS)
            .find_one(doc! {"namespace":bucket.namespace,"stream_id":&stored.stream_id,"source_event_id":&stored.source_event_id,"validation":"validated"})
            .session(&mut *session).await?.ok_or_else(corrupt)?;
        let event = inbox::decode_stored(&row).map_err(|_| corrupt())?;
        if &event.header().owner_ref != source
            || event.header().sandbox_id.as_str() != bucket.sandbox_id
            || event.header().source_epoch.as_str() != bucket.source_epoch
        {
            return Err(corrupt());
        }
        let fact = effective_fact(&event, &stored.fact_id).ok_or_else(corrupt)?;
        refs.push(ShadowFactRef {
            stream_id: stored.stream_id,
            fact_id: stored.fact_id.clone(),
            event_id: row.source_event_id,
            event_digest: row.digest,
            evidence_revision: fact.evidence_revision.get() as i64,
        });
        if let Some(pin) = &pin {
            for plan in pricing::shadow_plan(&event, &stored.fact_id, pin, tariff.as_ref())? {
                match plan.disposition {
                    pricing::Disposition::Candidate {
                        numerator_pico,
                        divisor,
                    } => {
                        let entry = fractions.entry(plan.metric).or_insert((0, divisor));
                        if entry.1 != divisor {
                            return Err(corrupt());
                        }
                        entry.0 = entry.0.checked_add(numerator_pico).ok_or_else(corrupt)?;
                    }
                    pricing::Disposition::Pending(reason) => {
                        unresolved.insert(format!("{}:{reason:?}", pricing::code(plan.metric)));
                    }
                    pricing::Disposition::NonBillable(_) => {}
                }
            }
        }
    }
    if let Some(pin) = &pin {
        use crate::models::sandbox_usage_rates::Selection;
        for (basis, selection) in [
            ("allocated", Selection::Allocated),
            ("measured", Selection::Measured),
        ] {
            if [
                pin.dimensions.cpu,
                pin.dimensions.memory,
                pin.dimensions.disk,
            ]
            .contains(&selection)
                && !positions.iter().any(|position| position.basis == basis)
            {
                unresolved.insert(format!("{basis}_stream_missing"));
            }
        }
        if pin.tariff_id.is_none() {
            unresolved.insert("tariff_unresolved".into());
        }
    }
    if refs.is_empty() {
        unresolved.insert("minute_evidence_missing".into());
    }
    let provisional_metrics = fractions
        .into_iter()
        .map(|(metric, (numerator, divisor))| {
            Ok(ShadowBucketMetric {
                metric_code: pricing::code(metric).into(),
                numerator_pico: numerator.to_string(),
                divisor: divisor.to_string(),
                rounded_candidate: round_half_even(numerator, divisor)?,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    let provisional_total = Credits::checked_sum(
        provisional_metrics
            .iter()
            .map(|metric| metric.rounded_candidate),
    )?;
    let input = ShadowBucketInput {
        price_pin: pin,
        facts: refs,
        provisional_metrics,
        unresolved: unresolved.into_iter().collect(),
        rounding: ROUNDING.into(),
    };
    // Positions record the observed validated heads, while the digest changes only
    // when this minute's effective evidence or pricing provenance changes.
    let input_digest = hex::encode(Sha256::digest(
        serde_json::to_vec(&input).map_err(|_| corrupt())?,
    ));
    let revisions = db.collection::<ShadowBucketRevision>(BUCKET_REVISIONS);
    let bucket_id = bucket.id()?;
    let previous = revisions
        .find_one(doc! {"bucket_id":&bucket_id})
        .sort(doc! {"revision":-1})
        .session(&mut *session)
        .await?;
    if let Some(previous) = &previous {
        if previous.schema_version != 1 || previous.owner_subject != owner_subject {
            return Err(corrupt());
        }
        if previous.input_digest == input_digest {
            return Ok(previous.clone());
        }
    }
    let revision = ShadowBucketRevision {
        id: Uuid::new_v4().to_string(),
        bucket_id,
        schema_version: 1,
        revision: previous
            .as_ref()
            .map_or(Some(1), |prior| prior.revision.checked_add(1))
            .ok_or_else(corrupt)?,
        previous_revision_id: previous.as_ref().map(|prior| prior.id.clone()),
        namespace: bucket.namespace.into(),
        sandbox_id: bucket.sandbox_id.into(),
        source_epoch: bucket.source_epoch.into(),
        owner_subject: owner_subject.into(),
        minute: BsonDateTime::from_chrono(bucket.minute).to_chrono(),
        charge_mode: ChargeMode::Shadow,
        positions,
        input,
        input_digest,
        provisional_total,
        provisional_delta: provisional_total.checked_sub(
            previous
                .as_ref()
                .map_or(Credits::ZERO, |prior| prior.provisional_total),
        )?,
    };
    revisions
        .insert_one(&revision)
        .session(&mut *session)
        .await?;
    Ok(revision)
}

fn effective_fact<'a>(event: &'a Event, id: &str) -> Option<&'a Fact> {
    match event.payload() {
        Payload::Usage { facts } => facts.iter().find(|fact| fact.fact_id.as_str() == id),
        Payload::Correction { correction } if correction.fact_id.as_str() == id => {
            Some(&correction.replacement)
        }
        _ => None,
    }
}

fn round_half_even(numerator: u128, divisor: u128) -> AppResult<Credits> {
    let quotient = numerator / divisor;
    let remainder = numerator % divisor;
    let round_up = remainder > divisor - remainder
        || (remainder == divisor - remainder && !quotient.is_multiple_of(2));
    let rounded = quotient
        .checked_add(u128::from(round_up))
        .ok_or_else(corrupt)?;
    Ok(Credits::from_pico(
        i128::try_from(rounded).map_err(|_| corrupt())?,
    )?)
}

fn corrupt() -> AppError {
    AppError::Internal("Stored Sandbox shadow bucket evidence is inconsistent".into())
}
