use super::*;
use mongodb::{
    ClientSession,
    options::{CollectionOptions, ReadConcern},
};
use std::collections::{BTreeMap, BTreeSet};
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(4);
pub(crate) const INDEX: &str = "usage_meter_forwarded_unreleased";

pub(crate) fn index() -> mongodb::IndexModel {
    mongodb::IndexModel::builder()
        .keys(doc! {"forwarded":1,"released":1,"_id":1})
        .options(
            mongodb::options::IndexOptions::builder()
                .name(INDEX.to_owned())
                .partial_filter_expression(doc! {"forwarded":true,"released":false})
                .build(),
        )
        .build()
}
pub(super) fn pipeline() -> Vec<Document> {
    let mut projection = doc! {"_id":1,"billing_request_id":1,"billing_owner_id":1,"wallet_id":1,
    "status":1,"forwarded":1,"released":1,"metric":1,"created_at":1,"forwarded_owner":1,
    "quantity":1,"pending_platform_usage":1,"pending_resale_quantity":1,
    "pool_attempt":1,"realtime_session.closed":1,"model_tariff.tariff_id":1,"managed_execution_id":1,
    "usage_checkpoint.final_report":1,"reserved_credits":1,"funding.settled":1,
    "funding.settlement_claim_id":1,"observed_at":"$$NOW"};
    for field in [
        "grant_reservations",
        "allowance_reservations",
        "grant_consumptions",
        "allowance_consumptions",
    ] {
        let path = format!("$funding.{field}");
        projection.insert(
            format!("funding.{field}"),
            doc! {"$cond":[{"$isArray":&path},{"$slice":[&path,(MAX_ALLOCATIONS+1) as i64]},&path]},
        );
    }
    let mut stages = vec![
        doc! {"$match":{"forwarded":true,"released":false}},
        doc! {"$sort":{"_id":1}},
        doc! {"$limit":(MAX_ROWS+1) as i64},
        doc! {"$project":projection},
    ];
    for (collection, path, owner, alias) in [
        (
            "billing_wallet",
            "wallet_id",
            "owner_id",
            "wallet_resources",
        ),
        (
            "credit_grants",
            "funding.grant_reservations.grant_id",
            "recipient_user_id",
            "grant_resources",
        ),
        (
            "usage_allowance_periods",
            "funding.allowance_reservations.period_id",
            "owner_user_id",
            "allowance_resources",
        ),
    ] {
        stages.push(doc!{"$lookup":{"from":collection,"localField":path,"foreignField":"_id",
            "pipeline":[{"$project":{"_id":1,"owner":format!("${owner}"),"active_settlement":1}}],"as":alias}});
    }
    stages.push(doc!{"$lookup":{"from":METERS,"localField":"billing_request_id","foreignField":"billing_request_id",
        "pipeline":[{"$limit":129},{"$project":{"_id":0,"billing_request_id":1,"metric":1,
            "pool_attempt":1,"realtime_session.closed":1,"managed_execution_id":1,"model_tariff.tariff_id":1,
            "pending_platform_usage.requests":1,"pending_resale_quantity":1,"usage_checkpoint.final_report":1,"usage_checkpoint.recovered":1}}],"as":"request_rows"}});
    stages
}
#[derive(Default)]
struct Group {
    rows: usize,
    requests: BTreeSet<String>,
    held: Credits,
    resources: BTreeSet<String>,
}
#[derive(Default)]
struct Inventory {
    rows: usize,
    invalid: usize,
    truncated: bool,
    registry_readable: bool,
    requests: BTreeSet<String>,
    groups: BTreeMap<(String, bool, String), Group>,
    holds: BTreeMap<(String, bool, String, String, String), Group>,
}
fn age(now: bson::DateTime, created: bson::DateTime) -> &'static str {
    match now
        .timestamp_millis()
        .saturating_sub(created.timestamp_millis())
        / 1000
    {
        ..0 => "future_timestamp",
        0..3600 => "under_1h",
        3600..86400 => "1h_to_24h",
        86400..604800 => "1d_to_7d",
        _ => "over_7d",
    }
}
impl Inventory {
    fn add(&mut self, row: Document, registry: Option<&Registry>) -> AppResult<()> {
        let c = classify(&row, registry)?;
        let request = row
            .get_str("billing_request_id")
            .map_err(|_| unavailable())?
            .to_owned();
        let now = *row.get_datetime("observed_at").map_err(|_| unavailable())?;
        let created = *row.get_datetime("created_at").map_err(|_| unavailable())?;
        let age = age(now, created).to_owned();
        let metric = row.get_str("metric").map_err(|_| unavailable())?;
        let holds = super::super::hold_conservation::backlog::request_holds(&row)?;
        self.requests.insert(request.clone());
        let key = (c.label.to_owned(), c.eligible, age.clone());
        let g = self.groups.entry(key).or_default();
        g.rows += 1;
        g.requests.insert(request.clone());
        for (kind, amounts) in holds {
            let unit = if kind == "allowance" {
                metric
            } else {
                "credits"
            };
            if amounts.is_empty() {
                continue;
            }
            let total = amounts
                .values()
                .try_fold(Credits::ZERO, |s, a| s.checked_add(*a))?;
            let g = self
                .holds
                .entry((
                    c.label.into(),
                    c.eligible,
                    age.clone(),
                    kind.into(),
                    unit.into(),
                ))
                .or_default();
            g.held = g.held.checked_add(total)?;
            g.rows += 1;
            g.requests.insert(request.clone());
            g.resources.extend(amounts.into_keys());
        }
        Ok(())
    }
    fn detail(&self) -> String {
        let mut lines = vec![format!(
            "complete={} registry_readable={} rows={} distinct_requests={} invalid_rows={} row_limit={} truncated={}",
            !self.truncated && self.invalid == 0 && self.registry_readable,
            self.registry_readable,
            self.rows,
            self.requests.len(),
            self.invalid,
            MAX_ROWS,
            self.truncated
        )];
        for ((class, eligible, age), g) in &self.groups {
            lines.push(format!(
                "class={class} eligible={eligible} age={age} rows={} requests={}",
                g.rows,
                g.requests.len()
            ));
        }
        for ((class, eligible, age, kind, unit), g) in &self.holds {
            lines.push(format!("class={class} eligible={eligible} age={age} kind={kind} unit={unit} rows={} requests={} held={} resources={}",g.rows,g.requests.len(),g.held,g.resources.len()));
        }
        lines.join("\n")
    }
}
async fn scan_once(db: &mongodb::Database, tx: &mut ClientSession) -> AppResult<Inventory> {
    // Ownership is an observation for this read-only page; terminal recovery
    // rechecks it transactionally. Do not carry its snapshot timestamp into
    // the money aggregate: outside a transaction MongoDB does not pin history.
    // Meter rows and all joined resource locks still share one fresh snapshot.
    let mut owner_session = db.client().start_session().await?;
    let registry = registry(db, &mut owner_session).await;
    if let Err(error) = &registry
        && super::super::hold_conservation::backlog::expired_snapshot(error)
    {
        return Err(registry.err().unwrap());
    }
    let registry = registry.ok();
    let mut out = Inventory {
        registry_readable: registry.is_some(),
        ..Default::default()
    };
    let mut cursor = db
        .collection_with_options::<Document>(
            METERS,
            CollectionOptions::builder()
                .read_concern(ReadConcern::snapshot())
                .build(),
        )
        .aggregate(pipeline())
        .max_time(std::time::Duration::from_secs(3))
        .batch_size(256)
        .session(&mut *tx)
        .await?;
    while let Some(row) = cursor.stream(&mut *tx).try_next().await? {
        if out.rows == MAX_ROWS {
            out.truncated = true;
            break;
        }
        out.rows += 1;
        if out.add(row, registry.as_ref()).is_err() {
            out.invalid += 1;
        }
    }
    Ok(out)
}
pub(crate) async fn diagnostics(
    db: &mongodb::Database,
) -> AppResult<Vec<super::super::BillingStartupDiagnostic>> {
    let result = tokio::time::timeout(
        TIMEOUT,
        super::super::hold_conservation::backlog::retry_snapshot_read(|| async {
            let mut tx = db.client().start_session().snapshot(true).await?;
            scan_once(db, &mut tx).await
        }),
    )
    .await;
    let detail = match result {
        Ok(Ok(inventory)) if inventory.rows == 0 => return Ok(vec![]),
        Ok(Ok(inventory)) => inventory.detail(),
        _ => "complete=false; forwarded request inventory could not be read; no cleanup eligibility established".into(),
    };
    Ok(vec![super::super::BillingStartupDiagnostic{
        code:"billing_stuck_requests".into(),summary:"Forwarded request recovery eligibility".into(),detail,
        remediation:"Read-only classification. Live includes the registry's 45-second window plus a two-minute margin. Legacy eligibility requires registration times for every live generation and an extra hour before the earliest registration. Missing, malformed, empty or unreadable registry evidence authorizes no cleanup. Amounts exclude returned holds and matching applied resource locks. Groups overlap in requests and resources; truncated totals are not population totals. No quantities or charges are changed by this diagnostic.".into(),detected_at:chrono::Utc::now(),
    }])
}
