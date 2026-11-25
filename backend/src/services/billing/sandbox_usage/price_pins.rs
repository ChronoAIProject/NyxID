//! One immutable decision for both evidence bases at a resource minute.
use super::{
    policy::{invalid, minute, scope_key, shadow, unavailable},
    pricing_schema,
    tariff_store::at,
};
use crate::{
    errors::{AppError, AppResult},
    models::sandbox_usage_rates::{
        COLLECTION_NAME, ChargeMode, PINS, POLICIES, PricePin, UsagePolicy, UsageScope, UsageTariff,
    },
};
use chrono::{DateTime, Utc};
use mongodb::{ClientSession, Database, bson::doc};

/// Internal recovery read for a known scope, not an unauthenticated public API.
/// N2 supplies the immutable historical billing owner after custody validation.
pub async fn recover(
    db: &Database,
    scope: &UsageScope,
    time: DateTime<Utc>,
) -> AppResult<Option<PricePin>> {
    minute(time)?;
    let pin = db
        .collection::<PricePin>(PINS)
        .find_one(doc! {"scope_key":scope_key(scope)?,"minute":at(time)})
        .read_concern(mongodb::options::ReadConcern::majority())
        .await?;
    if pin
        .as_ref()
        .is_some_and(|p| p.schema_version != 1 || &p.scope != scope)
    {
        return Err(invalid("Unsupported usage price pin binding"));
    }
    Ok(pin)
}

pub async fn pin(db: &Database, scope: &UsageScope, time: DateTime<Utc>) -> AppResult<PricePin> {
    pricing_schema::require_enabled(db).await?;
    minute(time)?;
    if time > Utc::now() {
        return Err(invalid("Cannot pin future usage"));
    }
    let scope_key = scope_key(scope)?;
    for _ in 0..8 {
        let attempt = async {
            let mut session = pricing_schema::begin(db).await?;
            let result = pin_in_session(db, &mut session, scope, &scope_key, time).await?;
            pricing_schema::commit(&mut session).await?;
            Ok(result)
        }
        .await;
        match attempt {
            Err(AppError::DatabaseError(ref e))
                if e.contains_label("TransientTransactionError") =>
            {
                continue;
            }
            result => return result,
        }
    }
    Err(unavailable(
        "Usage pricing contention; retry the same scope and minute",
    ))
}

pub(super) async fn pin_in_session(
    db: &Database,
    session: &mut ClientSession,
    scope: &UsageScope,
    key: &str,
    time: DateTime<Utc>,
) -> AppResult<PricePin> {
    let pins = db.collection::<PricePin>(PINS);
    if let Some(pin) = pins
        .find_one(doc! {"scope_key":key,"minute":at(time)})
        .session(&mut *session)
        .await?
    {
        if pin.schema_version != 1 || &pin.scope != scope {
            return Err(invalid("Unsupported usage price pin binding"));
        }
        return Ok(pin);
    }
    let policy = db
        .collection::<UsagePolicy>(POLICIES)
        .find_one(doc! {"scope_key":key,"effective_from":{"$lte":at(time)}})
        .sort(doc! {"effective_from":-1})
        .session(&mut *session)
        .await?;
    let tariff = db.collection::<UsageTariff>(COLLECTION_NAME).find_one(doc! {"producer_namespace":&scope.producer_namespace,"effective_from":{"$lte":at(time)}})
        .sort(doc! {"effective_from":-1}).session(&mut *session).await?;
    if policy
        .as_ref()
        .is_some_and(|p| p.schema_version != 1 || &p.scope != scope)
        || tariff.as_ref().is_some_and(|t| t.schema_version != 1)
    {
        return Err(invalid("Unsupported usage pricing schema"));
    }
    let pin = PricePin {
        id: uuid::Uuid::new_v4().to_string(),
        schema_version: 1,
        scope_key: key.into(),
        scope: scope.clone(),
        minute: time,
        charge_mode: ChargeMode::Shadow,
        policy_id: policy.as_ref().map(|p| p.id.clone()),
        policy_version: policy.as_ref().map(|p| p.version),
        dimensions: policy
            .as_ref()
            .map_or_else(shadow, |p| p.dimensions.clone()),
        legacy_coverage: policy.as_ref().and_then(|p| p.legacy_coverage.clone()),
        tariff_id: tariff.as_ref().map(|t| t.id.clone()),
        tariff_version: tariff.as_ref().map(|t| t.version),
        created_at: at(Utc::now()).to_chrono(),
    };
    pins.insert_one(&pin).session(session).await?;
    Ok(pin)
}
