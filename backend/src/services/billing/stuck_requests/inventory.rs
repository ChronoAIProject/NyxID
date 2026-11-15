use super::*;
use mongodb::{
    ClientSession,
    options::{Hint, ReadConcern},
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
    registry_failure: Option<ReadFailure>,
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
async fn scan_once(
    db: &mongodb::Database,
    tx: &mut ClientSession,
    stage: &std::sync::Mutex<&'static str>,
) -> AppResult<Inventory> {
    // Registry liveness is observational here; terminal recovery rechecks it
    // transactionally. The inventory transaction pins meter/resource history across
    // cursor batches, including on servers with no retained snapshot window.
    *stage.lock().unwrap() = "registry";
    let mut owner_session = db.client().start_session().await?;
    let registry = registry(db, &mut owner_session).await;
    if let Err(error) = &registry
        && super::super::hold_conservation::backlog::expired_snapshot(error)
    {
        return Err(registry.err().unwrap());
    }
    let registry_failure = registry
        .as_ref()
        .err()
        .map(|error| ReadFailure::from_error(error, "registry"));
    let registry = registry.ok();
    let mut out = Inventory {
        registry_readable: registry.is_some(),
        registry_failure,
        ..Default::default()
    };
    *stage.lock().unwrap() = "inventory";
    let mut cursor = db
        .collection::<Document>(METERS)
        .aggregate(pipeline())
        .hint(Hint::Name(INDEX.to_owned()))
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
        #[cfg(test)]
        if out.rows == 1 {
            super::tests::after_first_row().await;
        }
        if out.add(row, registry.as_ref()).is_err() {
            out.invalid += 1;
        }
    }
    Ok(out)
}
pub(crate) async fn diagnostics(
    db: &mongodb::Database,
) -> AppResult<Vec<super::super::BillingStartupDiagnostic>> {
    let started = std::time::Instant::now();
    let stage = std::sync::Mutex::new("session");
    let result = tokio::time::timeout(
        TIMEOUT,
        super::super::hold_conservation::backlog::retry_snapshot_read(|| async {
            let mut tx = db.client().start_session().await?;
            tx.start_transaction()
                .read_concern(ReadConcern::snapshot())
                .await?;
            let result = scan_once(db, &mut tx, &stage).await;
            // This is a read-only transaction. Abort releases the pinned history;
            // dropping a cancelled session also queues an abort in the driver.
            let failed_stage = *stage.lock().unwrap();
            *stage.lock().unwrap() = "snapshot_release";
            let cleanup = tx.abort_transaction().await;
            match result {
                Err(error) => {
                    *stage.lock().unwrap() = failed_stage;
                    Err(error)
                }
                Ok(inventory) => {
                    cleanup?;
                    Ok(inventory)
                }
            }
        }),
    )
    .await;
    let elapsed = started.elapsed().as_millis();
    let detail = match result {
        Ok(Ok(inventory)) if inventory.rows == 0 => return Ok(vec![]),
        Ok(Ok(inventory)) => {
            let reason = if let Some(failure) = &inventory.registry_failure {
                failure.metadata()
            } else if inventory.invalid != 0 {
                "reason=row_decode_error stage=decode".into()
            } else if inventory.truncated {
                "reason=row_limit stage=inventory".into()
            } else {
                "reason=ok".into()
            };
            format!("{}\n{reason} elapsed_ms={elapsed}", inventory.detail())
        }
        Ok(Err(error)) => ReadFailure::from_error(&error, *stage.lock().unwrap()).detail(elapsed),
        Err(_) => ReadFailure {
            reason: "deadline_exceeded",
            stage: *stage.lock().unwrap(),
            database_code: None,
        }
        .detail(elapsed),
    };
    Ok(vec![super::super::BillingStartupDiagnostic{
        code:"billing_stuck_requests".into(),summary:"Forwarded request recovery eligibility".into(),detail,
        remediation:"Read-only classification. Live includes the registry's 45-second window plus a two-minute margin. Legacy eligibility requires registration times for every live generation and an extra hour before the earliest registration. Missing, malformed, empty or unreadable registry evidence authorizes no cleanup. Amounts exclude returned holds and matching applied resource locks. Groups overlap in requests and resources; truncated totals are not population totals. No quantities or charges are changed by this diagnostic.".into(),detected_at:chrono::Utc::now(),
    }])
}

// Only stable metadata is exposed: driver messages can contain query values.
#[derive(Clone)]
struct ReadFailure {
    reason: &'static str,
    stage: &'static str,
    database_code: Option<i32>,
}
impl ReadFailure {
    fn from_error(error: &AppError, stage: &'static str) -> Self {
        use mongodb::error::ErrorKind;
        let mut code = None;
        let reason = match error {
            AppError::DatabaseError(error) => match error.kind.as_ref() {
                ErrorKind::Command(command) => {
                    code = Some(command.code);
                    match command.code {
                        50 | 262 => "query_timeout",
                        239 | 246 => "snapshot_unavailable",
                        27 => "index_unavailable",
                        // MongoDB uses BadValue for a missing named hint.
                        2 if command.message.contains("hint") => "index_unavailable",
                        _ => "database_error",
                    }
                }
                ErrorKind::BsonDeserialization(_) => "row_decode_error",
                _ => "database_unavailable",
            },
            _ if stage == "registry" => "registry_evidence_unavailable",
            _ => "row_decode_error",
        };
        Self {
            reason,
            stage,
            database_code: code,
        }
    }
    fn metadata(&self) -> String {
        let mut value = format!("reason={} stage={}", self.reason, self.stage);
        if let Some(code) = self.database_code {
            value.push_str(&format!(" database_code={code}"));
        }
        value
    }
    fn detail(&self, elapsed: u128) -> String {
        format!(
            "complete=false; {} elapsed_ms={elapsed}; no cleanup eligibility established",
            self.metadata()
        )
    }
}

#[cfg(test)]
mod failure_tests {
    use super::*;

    #[test]
    fn stuck_request_failure_codes_do_not_expose_driver_messages() {
        for (code, name, reason) in [
            (50, "MaxTimeMSExpired", "query_timeout"),
            (239, "SnapshotTooOld", "snapshot_unavailable"),
            (246, "SnapshotUnavailable", "snapshot_unavailable"),
            (27, "IndexNotFound", "index_unavailable"),
            (13, "Unauthorized", "database_error"),
        ] {
            let command: mongodb::error::CommandError = bson::from_document(doc! {
                "code":code,"codeName":name,"errmsg":"private-query-values",
            })
            .unwrap();
            let error = AppError::DatabaseError(mongodb::error::Error::from(
                mongodb::error::ErrorKind::Command(command),
            ));
            let detail = ReadFailure::from_error(&error, "inventory").detail(123);
            assert!(
                detail.contains(&format!(
                    "reason={reason} stage=inventory database_code={code} elapsed_ms=123"
                )),
                "{detail}"
            );
            assert!(!detail.contains("private-query-values"));
        }
    }
}
