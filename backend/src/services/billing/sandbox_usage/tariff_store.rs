use super::{
    policy::{effective_minute, invalid, minute, scope_key, text},
    pricing, pricing_schema,
};
use crate::{
    errors::{AppError, AppResult},
    models::sandbox_usage_rates::{
        COLLECTION_NAME, DimensionPolicy, LegacyCoverage, POLICIES, UsagePolicy, UsageRate,
        UsageScope, UsageTariff,
    },
    services::audit_service::{self, AuditActor},
};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};
use std::collections::BTreeMap;

pub struct PublishTariff {
    pub producer_namespace: String,
    pub expected_version: i64,
    pub effective_from: DateTime<Utc>,
    pub rates: BTreeMap<String, String>,
}
pub struct PublishPolicy {
    pub scope: UsageScope,
    pub expected_version: i64,
    pub effective_from: DateTime<Utc>,
    pub dimensions: DimensionPolicy,
    pub legacy_coverage: Option<LegacyCoverage>,
}

fn next_version(version: i64) -> AppResult<i64> {
    version
        .checked_add(1)
        .filter(|v| *v > 0)
        .ok_or_else(|| invalid("Invalid expected usage version"))
}
fn validate_rates(rates: &BTreeMap<String, String>) -> AppResult<Vec<UsageRate>> {
    if rates.is_empty() || rates.len() > 6 {
        return Err(invalid("Supply between one and six usage rates"));
    }
    rates
        .iter()
        .map(|(code, value)| {
            let metric = pricing::METRICS
                .into_iter()
                .find(|m| pricing::code(*m) == code)
                .ok_or_else(|| invalid("Unsupported usage metric"))?;
            let normalized = super::super::pricing::normalize_price(value)?;
            Ok(UsageRate {
                metric_code: code.clone(),
                definition_version: pricing::definition(metric).into(),
                charge_unit: pricing::unit(metric),
                credits_per_unit: normalized.parse()?,
                provider_proof: None,
            })
        })
        .collect()
}

pub async fn publish_tariff(
    db: &Database,
    input: PublishTariff,
    actor: &AuditActor,
    key: &[u8],
) -> AppResult<UsageTariff> {
    pricing_schema::require_enabled(db).await?;
    text(&input.producer_namespace)?;
    let now = Utc::now();
    let card = UsageTariff {
        id: uuid::Uuid::new_v4().to_string(),
        schema_version: 1,
        producer_namespace: input.producer_namespace,
        version: next_version(input.expected_version)?,
        effective_from: effective_minute(input.effective_from, now)?,
        rates: validate_rates(&input.rates)?,
        created_by: actor.user_id.clone(),
        created_at: at(now).to_chrono(),
    };
    let mut session = pricing_schema::begin(db).await?;
    let latest = db
        .collection::<UsageTariff>(COLLECTION_NAME)
        .find_one(doc! {"producer_namespace":&card.producer_namespace})
        .sort(doc! {"version":-1})
        .session(&mut session)
        .await?;
    if latest.as_ref().is_some_and(|r| r.schema_version != 1) {
        return Err(invalid("Unsupported usage tariff schema"));
    }
    if latest.as_ref().map_or(0, |r| r.version) != input.expected_version
        || latest
            .as_ref()
            .is_some_and(|r| r.effective_from >= card.effective_from)
    {
        return Err(AppError::Conflict(
            "Usage tariff history changed; reload before publishing".into(),
        ));
    }
    db.collection::<UsageTariff>(COLLECTION_NAME)
        .insert_one(&card)
        .session(&mut session)
        .await?;
    audit_service::log_actor_event_in_session(db, &mut session, key, actor, "sandbox_usage_tariff_published",
        serde_json::json!({"tariff_id":card.id,"version":card.version,"effective_from":card.effective_from,"shadow_only":true})).await?;
    pricing_schema::commit(&mut session).await?;
    Ok(card)
}

pub async fn publish_policy(
    db: &Database,
    input: PublishPolicy,
    actor: &AuditActor,
    key: &[u8],
) -> AppResult<UsagePolicy> {
    pricing_schema::require_enabled(db).await?;
    let now = Utc::now();
    if let Some(legacy) = &input.legacy_coverage {
        minute(legacy.covered_until)?;
        if legacy.manifest_digest.len() != 64
            || !legacy
                .manifest_digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("Invalid legacy coverage manifest digest"));
        }
    }
    let policy = UsagePolicy {
        id: uuid::Uuid::new_v4().to_string(),
        schema_version: 1,
        scope_key: scope_key(&input.scope)?,
        scope: input.scope,
        version: next_version(input.expected_version)?,
        effective_from: effective_minute(input.effective_from, now)?,
        dimensions: input.dimensions,
        legacy_coverage: input.legacy_coverage,
        created_by: actor.user_id.clone(),
        created_at: at(now).to_chrono(),
    };
    let mut session = pricing_schema::begin(db).await?;
    let latest = db
        .collection::<UsagePolicy>(POLICIES)
        .find_one(doc! {"scope_key":&policy.scope_key})
        .sort(doc! {"version":-1})
        .session(&mut session)
        .await?;
    if latest.as_ref().is_some_and(|r| r.schema_version != 1) {
        return Err(invalid("Unsupported usage policy schema"));
    }
    if latest.as_ref().map_or(0, |p| p.version) != input.expected_version
        || latest
            .as_ref()
            .is_some_and(|p| p.effective_from >= policy.effective_from)
    {
        return Err(AppError::Conflict(
            "Usage policy history changed; reload before publishing".into(),
        ));
    }
    if let Some(prior) = latest.as_ref().and_then(|p| p.legacy_coverage.as_ref())
        && policy
            .legacy_coverage
            .as_ref()
            .is_none_or(|next| next.covered_until < prior.covered_until)
    {
        return Err(invalid("Legacy charged coverage cannot shrink"));
    }
    db.collection::<UsagePolicy>(POLICIES)
        .insert_one(&policy)
        .session(&mut session)
        .await?;
    audit_service::log_actor_event_in_session(db, &mut session, key, actor, "sandbox_usage_policy_published",
        serde_json::json!({"policy_id":policy.id,"version":policy.version,"effective_from":policy.effective_from,"shadow_only":true})).await?;
    pricing_schema::commit(&mut session).await?;
    Ok(policy)
}

pub async fn list_tariffs(db: &Database, namespace: &str) -> AppResult<Vec<UsageTariff>> {
    pricing_schema::require_enabled(db).await?;
    text(namespace)?;
    let rows: Vec<UsageTariff> = db
        .collection::<UsageTariff>(COLLECTION_NAME)
        .find(doc! {"producer_namespace":namespace})
        .sort(doc! {"version":-1})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    if rows.iter().any(|r| r.schema_version != 1) {
        return Err(invalid("Unsupported usage tariff schema"));
    }
    Ok(rows)
}

pub async fn list_policies(db: &Database, scope: &UsageScope) -> AppResult<Vec<UsagePolicy>> {
    pricing_schema::require_enabled(db).await?;
    let rows: Vec<UsagePolicy> = db
        .collection::<UsagePolicy>(POLICIES)
        .find(doc! {"scope_key":scope_key(scope)?})
        .sort(doc! {"version":-1})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    if rows
        .iter()
        .any(|r| r.schema_version != 1 || &r.scope != scope)
    {
        return Err(invalid("Unsupported usage policy binding"));
    }
    Ok(rows)
}

pub(super) fn at(at: DateTime<Utc>) -> bson::DateTime {
    bson::DateTime::from_chrono(at)
}
