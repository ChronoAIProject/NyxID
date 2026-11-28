use super::{contract::Event, inbox, price_pins, pricing};
use crate::{
    errors::{AppError, AppResult},
    models::{
        sandbox_usage_inbox::{
            EFFECTIVE_FACTS, EVENTS, EffectiveFact, Event as StoredEvent, HistoricalOwner,
            OWNER_PINS, Validation,
        },
        sandbox_usage_rates::{COLLECTION_NAME as TARIFFS, UsageScope, UsageTariff},
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
};
use bson::doc;
use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    options::{Acknowledgment, ReadConcern, WriteConcern},
};
use serde_json::{Value, json};
use uuid::Uuid;

/// Price the effective version of one retained fact using its original usage time.
/// This lookup and the pinned result carry no authority to post wallet movements.
pub async fn inspect(
    db: &Database,
    nyxid_issuer: &str,
    namespace: &str,
    stream_id: &str,
    fact_id: &str,
) -> AppResult<Option<Value>> {
    let stored = db
        .collection::<EffectiveFact>(EFFECTIVE_FACTS)
        .find_one(doc! {"namespace":namespace,"stream_id":stream_id,"fact_id":fact_id})
        .read_concern(ReadConcern::majority())
        .await?;
    let Some(stored) = stored else {
        return Ok(None);
    };
    let row = db
        .collection::<StoredEvent>(EVENTS)
        .find_one(doc! {"namespace":namespace,"stream_id":stream_id,"source_event_id":&stored.source_event_id})
        .read_concern(ReadConcern::majority())
        .await?
        .filter(|row| matches!(row.validation, Validation::Validated))
        .ok_or_else(corrupt)?;
    let event = inbox::decode_stored(&row).map_err(|_| corrupt())?;
    let Some(owner) = historical_owner(db, nyxid_issuer, &event).await? else {
        return Ok(Some(json!({
            "fact_id":fact_id, "source_event_id":row.source_event_id,
            "owner_status":"unresolved", "financial_status":"shadow", "plans":[],
        })));
    };
    let scope = UsageScope {
        producer_namespace: owner.namespace,
        sandbox_id: owner.sandbox_id,
        source_epoch: owner.source_epoch,
        owner_id: owner.owner_id,
    };
    let minute = DateTime::from_timestamp_millis(stored.bucket_start_ms).ok_or_else(corrupt)?;
    let pin = price_pins::pin(db, &scope, minute).await?;
    let tariff = match pin.tariff_id.as_deref() {
        Some(tariff_id) => Some(
            db.collection::<UsageTariff>(TARIFFS)
                .find_one(doc! {"_id":tariff_id})
                .read_concern(ReadConcern::majority())
                .await?
                .ok_or_else(corrupt)?,
        ),
        None => None,
    };
    let plans = pricing::shadow_plan(&event, fact_id, &pin, tariff.as_ref())?
        .into_iter()
        .map(|plan| {
            let result = match plan.disposition {
                pricing::Disposition::Candidate { numerator_pico, divisor } => json!({
                    "status":"candidate", "numerator_pico":numerator_pico.to_string(), "divisor":divisor.to_string()
                }),
                pricing::Disposition::Pending(reason) => {
                    use pricing::Pending::*;
                    let reason = match reason {
                        IncompleteEvidence => "incomplete_evidence",
                        MissingTariff => "missing_tariff",
                        LegacyCoverageUnknown => "legacy_coverage_unknown",
                    };
                    json!({"status":"pending", "reason":reason})
                }
                pricing::Disposition::NonBillable(reason) => {
                    use pricing::NonBillable::*;
                    let reason = match reason {
                        Shadow => "shadow",
                        Free => "free",
                        Excluded => "excluded",
                        OtherBasis => "other_basis",
                        LegacyCoverage => "legacy_coverage",
                    };
                    json!({"status":"nonbillable","reason":reason})
                }
            };
            json!({
                "metric":pricing::code(plan.metric), "pin_id":plan.pin_id,
                "tariff_id":plan.tariff_id, "billing_period":plan.billing_period, "result":result,
            })
        })
        .collect::<Vec<_>>();
    Ok(Some(json!({
        "fact_id":fact_id, "source_event_id":row.source_event_id,
        "owner_status":"resolved", "financial_status":"shadow", "plans":plans,
    })))
}

pub(super) async fn historical_owner(
    db: &Database,
    nyxid_issuer: &str,
    event: &Event,
) -> AppResult<Option<HistoricalOwner>> {
    let header = event.header();
    let source = &header.owner_ref;
    let namespace = header.producer_namespace.as_str();
    let sandbox_id = header.sandbox_id.as_str();
    let source_epoch = header.source_epoch.as_str();
    let key = doc! {"namespace":namespace,"sandbox_id":sandbox_id,"source_epoch":source_epoch};
    let pins = db.collection::<HistoricalOwner>(OWNER_PINS);
    if let Some(pin) = pins
        .find_one(key.clone())
        .read_concern(ReadConcern::majority())
        .await?
    {
        return verified_pin(pin, event).map(Some);
    }
    if source.issuer.as_str() != nyxid_issuer {
        return Ok(None);
    }
    let subject = source.subject.as_str();
    if Uuid::parse_str(subject)
        .ok()
        .is_none_or(|id| id.to_string() != subject)
    {
        return Ok(None);
    }
    let user = db
        .collection::<User>(USERS)
        .find_one(doc! {"_id":subject,"is_active":true})
        .read_concern(ReadConcern::majority())
        .await?;
    let Some(user) = user else {
        return Ok(None);
    };
    let kind = match user.user_type {
        UserType::Person => "person",
        UserType::Org => "organization",
    };
    if !matches!(source.kind.as_str(), "unresolved") && source.kind.as_str() != kind {
        return Ok(None);
    }
    let pin = HistoricalOwner {
        id: Uuid::new_v4().to_string(),
        namespace: namespace.into(),
        sandbox_id: sandbox_id.into(),
        source_epoch: source_epoch.into(),
        owner_issuer: nyxid_issuer.into(),
        owner_subject: subject.into(),
        owner_id: user.id,
        resolved_at: Utc::now(),
    };
    match pins
        .insert_one(&pin)
        .write_concern(WriteConcern::builder().w(Acknowledgment::Majority).build())
        .await
    {
        Ok(_) => Ok(Some(pin)),
        Err(error) if super::super::ledger::is_duplicate_key_error(&error) => {
            let retained = pins
                .find_one(key)
                .read_concern(ReadConcern::majority())
                .await?
                .ok_or_else(corrupt)?;
            verified_pin(retained, event).map(Some)
        }
        Err(error) => Err(error.into()),
    }
}

fn verified_pin(pin: HistoricalOwner, event: &Event) -> AppResult<HistoricalOwner> {
    let source = &event.header().owner_ref;
    if pin.owner_issuer != source.issuer.as_str()
        || pin.owner_subject != source.subject.as_str()
        || pin.owner_id != pin.owner_subject
    {
        return Err(AppError::Conflict(
            "The usage epoch already has a different owner".into(),
        ));
    }
    Ok(pin)
}

fn corrupt() -> AppError {
    AppError::Internal("Stored Sandbox usage evidence is inconsistent".into())
}
