use super::selection::Source;
use super::*;
use mongodb::{
    ClientSession,
    options::{Hint, ReadConcern},
};
use std::collections::{BTreeMap, BTreeSet};
use std::future::IntoFuture;
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
pub(super) const PAGE_ROWS: usize = 128;
const COUNT_KEYS: i64 = 100_001;
pub(super) fn pipeline(source: Source, after: &str, limit: usize) -> Vec<Document> {
    let mut projection = doc! {"_id":1,"billing_request_id":1,"billing_owner_id":1,"wallet_id":1,"actor_user_id":1,
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
        doc! {"$match":source.page_filter(after)},
        doc! {"$sort":{"_id":1}},
        doc! {"$limit":limit as i64},
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
        "pipeline":[{"$limit":129},{"$project":{"_id":0,"billing_request_id":1,"metric":1,"actor_user_id":1,
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
    candidates: BTreeSet<String>,
    holdless_lower_bound: Option<i64>,
    forwarded_count_capped: bool,
    invalid: usize,
    truncated: bool,
    registry_readable: bool,
    registry_failure: Option<ReadFailure>,
    requests: BTreeSet<String>,
    groups: BTreeMap<(String, bool, String, String), Group>,
    holds: BTreeMap<(String, bool, String, String, String, String), Group>,
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
        let holds = super::super::hold_conservation::backlog::request_holds(&row)?;
        if holds.iter().all(|(_, amounts)| amounts.is_empty()) {
            return Ok(());
        }
        let c = classify(&row, registry)?;
        let request = row
            .get_str("billing_request_id")
            .map_err(|_| unavailable())?
            .to_owned();
        let now = *row.get_datetime("observed_at").map_err(|_| unavailable())?;
        let created = *row.get_datetime("created_at").map_err(|_| unavailable())?;
        let age = age(now, created).to_owned();
        let metric = row.get_str("metric").map_err(|_| unavailable())?;
        self.rows += 1;
        self.requests.insert(request.clone());
        let identity_kind = voice::kind(&row).to_owned();
        let key = (
            c.label.to_owned(),
            c.eligible,
            age.clone(),
            identity_kind.clone(),
        );
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
                    identity_kind.clone(),
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
        lines.push(format!(
            "candidate_rows={} counts_are_lower_bounds={} page_rows={PAGE_ROWS}",
            self.candidates.len(),
            self.truncated || self.invalid != 0
        ));
        if let Some(bound) = self.holdless_lower_bound {
            lines.push(format!("holdless_rows_at_least={bound} forwarded_count_capped={} count_key_limit={COUNT_KEYS}; conservative bound excludes all potential holds, including receipts and overlapping funding", self.forwarded_count_capped));
        } else {
            lines.push("holdless_rows_at_least=unknown".into());
        }
        for ((class, eligible, age, voice_kind), g) in &self.groups {
            lines.push(format!(
                "class={class} eligible={eligible} age={age} rows={} requests={}{}",
                g.rows,
                g.requests.len(),
                if voice_kind.is_empty() {
                    String::new()
                } else {
                    format!(" voice_kind={voice_kind}")
                }
            ));
        }
        for ((class, eligible, age, kind, unit, voice_kind), g) in &self.holds {
            lines.push(format!("class={class} eligible={eligible} age={age} kind={kind} unit={unit} rows={} requests={} held={} resources={}{}",g.rows,g.requests.len(),g.held,g.resources.len(),if voice_kind.is_empty(){String::new()}else{format!(" voice_kind={voice_kind}")}));
        }
        lines.join("\n")
    }
}
#[derive(Default, serde::Deserialize)]
#[serde(default)]
struct RecoveryTotals {
    ended_requests: i64,
    ended_rows: i64,
    holdless_ended_requests: i64,
    holdless_ended_rows: i64,
}

impl RecoveryTotals {
    fn read(state: Option<Document>) -> Result<Self, ReadFailure> {
        let invalid = || ReadFailure {
            reason: "row_decode_error",
            stage: "ended_totals",
            database_code: None,
        };
        let totals: Self = bson::from_document(state.unwrap_or_default()).map_err(|_| invalid())?;
        if [
            totals.ended_requests,
            totals.ended_rows,
            totals.holdless_ended_requests,
            totals.holdless_ended_rows,
        ]
        .iter()
        .any(|n| *n < 0)
        {
            return Err(invalid());
        }
        Ok(totals)
    }
}

impl Inventory {
    fn presentation(
        &self,
        totals: Option<&RecoveryTotals>,
        failure: Option<&ReadFailure>,
    ) -> (super::super::DiagnosticSeverity, String, String) {
        use super::super::diagnostic_text::{
            InventoryIssue, counted, inventory_status, number, oldest,
        };
        let mut issues = BTreeSet::new();
        let mut sentences = Vec::new();
        if self.invalid != 0 {
            issues.insert(InventoryIssue::InvalidRows);
            sentences.push(format!(
                "{} invalid {} {} found.",
                number(self.invalid as u64),
                if self.invalid == 1 { "row" } else { "rows" },
                if self.invalid == 1 { "was" } else { "were" }
            ));
        }
        if self.truncated {
            issues.insert(failure.map_or(InventoryIssue::RowLimit, |failure| {
                InventoryIssue::scan_failure(failure.reason)
            }));
            sentences.push("The hold scan is incomplete.".into());
        }
        if !self.registry_readable {
            issues.insert(InventoryIssue::RegistryRead);
            sentences.push("Process ownership evidence could not be read.".into());
        }
        if self.holdless_lower_bound.is_none() {
            issues.insert(InventoryIssue::HoldlessCount);
            sentences.push("The count of rows with no holds could not be read.".into());
        }
        if totals.is_none() {
            issues.insert(InventoryIssue::RecoveryTotals);
            sentences.push("Recovery totals could not be read.".into());
        }
        let (severity, remediation) = inventory_status(&issues);
        let partial = self.truncated || self.invalid != 0;
        let mut kinds = Vec::new();
        for (kind, label) in [
            ("wallet", "wallet credits"),
            ("grant", "grant credits"),
            ("allowance", "allowance units"),
        ] {
            let rows: usize = self
                .holds
                .iter()
                .filter(|((_, _, _, found, _, _), _)| found == kind)
                .map(|(_, group)| group.rows)
                .sum();
            if rows != 0 {
                kinds.push(format!("{} with {label}", number(rows as u64)));
            }
        }
        if self.requests.is_empty() {
            sentences.push(
                if partial {
                    "No requests with holds could be counted in the rows read."
                } else {
                    "No requests with holds were found."
                }
                .into(),
            );
        } else {
            sentences.push(format!(
                "{}{} {} holds across {}{}.",
                if partial { "At least " } else { "" },
                counted(self.requests.len() as u64, "request", "requests"),
                if self.requests.len() == 1 {
                    "has"
                } else {
                    "have"
                },
                counted(self.rows as u64, "billing row", "billing rows"),
                if kinds.is_empty() {
                    String::new()
                } else {
                    format!(", including {}", kinds.join(", "))
                },
            ));
            let age = oldest(self.groups.keys().map(|(_, _, age, _)| age.as_str()));
            if !age.is_empty() {
                sentences.push(age);
            }
        }
        let mut eligible: BTreeSet<&String> = BTreeSet::new();
        let mut running: BTreeSet<&String> = BTreeSet::new();
        let mut finalizing: BTreeSet<&String> = BTreeSet::new();
        for ((class, can_recover, _, _), group) in &self.groups {
            if *can_recover {
                eligible.extend(&group.requests);
            } else if matches!(
                class.as_str(),
                "owned_by_live_generation" | "voice_live_session"
            ) {
                running.extend(&group.requests);
            } else if matches!(
                class.as_str(),
                "final_quantity_awaiting_retry"
                    | "durable_finalization_intent"
                    | "has_final_checkpoint"
                    | "money_protocol_in_progress"
            ) {
                finalizing.extend(&group.requests);
            }
        }
        if !eligible.is_empty() {
            sentences.push(format!(
                "The recovery sweep can release unused holds from eligible reservations in {} after rechecking them.",
                counted(eligible.len() as u64, "request", "requests")
            ));
        }
        for (requests, description) in [
            (&running, "that belong to work that is still running"),
            (&finalizing, "being finalized"),
        ] {
            if !requests.is_empty() {
                sentences.push(format!(
                    "{} {} reservations {description}.",
                    counted(requests.len() as u64, "request", "requests"),
                    if requests.len() == 1 { "has" } else { "have" }
                ));
            }
        }
        let other = self
            .requests
            .iter()
            .filter(|r| !eligible.contains(r) && !running.contains(r) && !finalizing.contains(r))
            .count();
        if other != 0 {
            sentences.push(format!(
                "{} {} ownership evidence or another recovery path.",
                counted(other as u64, "request", "requests"),
                if other == 1 { "awaits" } else { "await" }
            ));
        }
        if let Some(bound) = self.holdless_lower_bound {
            sentences.push(format!(
                "At least {} sent request {} {} no funds, so no money is tied up in {}.",
                number(bound as u64),
                if bound == 1 { "row" } else { "rows" },
                if bound == 1 { "holds" } else { "hold" },
                if bound == 1 { "it" } else { "them" }
            ));
        }
        if let Some(totals) = totals {
            sentences.push(format!(
                "Cleanup has closed {} such {} so far ({}).",
                number(totals.holdless_ended_rows as u64),
                if totals.holdless_ended_rows == 1 {
                    "row"
                } else {
                    "rows"
                },
                counted(totals.holdless_ended_requests as u64, "request", "requests")
            ));
            if totals.ended_requests != 0 || totals.ended_rows != 0 {
                sentences.push(format!(
                    "The recovery sweep has ended {} covering {}.",
                    counted(totals.ended_requests as u64, "request", "requests"),
                    counted(totals.ended_rows as u64, "billing row", "billing rows")
                ));
            }
        }
        (severity, sentences.join(" "), remediation)
    }
}

#[cfg(test)]
mod presentation_tests;

pub(super) fn count_pipeline() -> Vec<Document> {
    vec![doc! {"$limit":COUNT_KEYS}, doc! {"$count":"rows"}]
}
async fn count_keys(db: &mongodb::Database, tx: &mut ClientSession, index: &str) -> AppResult<i64> {
    let mut cursor = db
        .collection::<Document>(METERS)
        .aggregate(count_pipeline())
        .hint(Hint::Name(index.to_owned()))
        .max_time(std::time::Duration::from_millis(500))
        .session(&mut *tx)
        .await?;
    let row = next_document(&mut cursor, tx).await?;
    match row {
        None => Ok(0),
        Some(row) => match row.get("rows") {
            Some(Bson::Int32(n)) => Ok(i64::from(*n)),
            Some(Bson::Int64(n)) => Ok(*n),
            _ => Err(unavailable()),
        },
    }
}
async fn scan_once(
    db: &mongodb::Database,
    tx: &mut ClientSession,
    stage: &std::sync::Mutex<&'static str>,
    out: &mut Inventory,
) -> AppResult<()> {
    // Registry liveness is observational; terminal recovery rechecks it in its
    // transaction. Meter rows, receipts, resource locks and counts share one
    // pinned read-only snapshot, including between pages and candidate sources.
    *stage.lock().unwrap() = "registry";
    let mut owner_session = db.client().start_session().await?;
    let registry = registry(db, &mut owner_session).await;
    out.registry_failure = registry
        .as_ref()
        .err()
        .map(|error| ReadFailure::from_error(error, "registry"));
    let registry = registry.ok();
    out.registry_readable = registry.is_some();

    *stage.lock().unwrap() = "holdless_count";
    let forwarded = count_keys(db, tx, INDEX).await?;
    let mut potential = 0;
    // Deliberately count entries in each named partial index, with no filter:
    // count its entries, without re-fetching documents to evaluate the partial
    // expression. These indexes are non-multikey, one entry per candidate row.
    // Sum is an upper bound on the union (overlapping sources count twice).
    // If any source is capped, it alone exhausts the capped total: report zero,
    // never subtract a truncated candidate scan and overstate hold-less rows.
    for source in Source::ALL {
        potential += count_keys(db, tx, source.index()).await?;
    }
    out.holdless_lower_bound = Some((forwarded - potential).max(0));
    out.forwarded_count_capped = forwarded == COUNT_KEYS;

    *stage.lock().unwrap() = "inventory";
    let mut after = [String::new(), String::new(), String::new()];
    let mut done = [false; 3];
    while done.iter().any(|finished| !finished) {
        for (i, source) in Source::ALL.into_iter().enumerate() {
            if done[i] {
                continue;
            }
            let mut cursor = db
                .collection::<Document>(METERS)
                .aggregate(pipeline(source, &after[i], PAGE_ROWS))
                .hint(Hint::Name(source.index().to_owned()))
                .max_time(std::time::Duration::from_millis(500))
                .batch_size(PAGE_ROWS as u32)
                .session(&mut *tx)
                .await?;
            let mut page = read_documents(&mut cursor, tx).await?;
            let count = page.len();
            *stage.lock().unwrap() = "voice_evidence";
            voice::read(db, tx, &mut page).await?;
            *stage.lock().unwrap() = "inventory";
            for row in page {
                after[i] = row.get_str("_id").map_err(|_| unavailable())?.to_owned();
                if out.candidates.contains(&after[i]) {
                    continue;
                }
                if out.candidates.len() == MAX_ROWS {
                    out.truncated = true;
                    return Ok(());
                }
                out.candidates.insert(after[i].clone());
                if out.add(row, registry.as_ref()).is_err() {
                    out.invalid += 1;
                }
                #[cfg(test)]
                if out.candidates.len() == 1 {
                    super::tests::after_first_row().await;
                }
            }
            done[i] = count < PAGE_ROWS;
        }
    }
    Ok(())
}
pub(crate) async fn diagnostics(
    db: &mongodb::Database,
) -> AppResult<Vec<super::super::BillingStartupDiagnostic>> {
    let started = std::time::Instant::now();
    let stage = std::sync::Mutex::new("session");
    // Progress lives outside the cancellable future. A deadline or later-page
    // error retains the already classified prefix, explicitly as lower bounds.
    let mut inventory = Inventory::default();
    let result = tokio::time::timeout(TIMEOUT, async {
        for attempt in 0..8 {
            let mut tx = db.client().start_session().await?;
            tx.start_transaction()
                .read_concern(ReadConcern::snapshot())
                .await?;
            let result = scan_once(db, &mut tx, &stage, &mut inventory).await;
            let failed_stage = *stage.lock().unwrap();
            *stage.lock().unwrap() = "snapshot_release";
            let cleanup = tx.abort_transaction().await;
            match result {
                Err(error) => {
                    *stage.lock().unwrap() = failed_stage;
                    if inventory.candidates.is_empty()
                        && attempt < 7
                        && super::super::hold_conservation::backlog::expired_snapshot(&error)
                    {
                        inventory = Inventory::default();
                        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                        continue;
                    }
                    return Err(error);
                }
                Ok(()) => return cleanup.map_err(AppError::from),
            }
        }
        unreachable!()
    })
    .await;
    let failure = match result {
        Ok(Ok(())) => None,
        Ok(Err(error)) => Some(ReadFailure::from_error(&error, *stage.lock().unwrap())),
        Err(_) => Some(ReadFailure {
            reason: "deadline_exceeded",
            stage: *stage.lock().unwrap(),
            database_code: None,
        }),
    };
    inventory.truncated |= failure.is_some();
    let present = failure.is_some()
        || !inventory.registry_readable
        || !inventory.candidates.is_empty()
        || inventory.holdless_lower_bound != Some(0);
    let reason = if let Some(failure) = failure.as_ref().or(inventory.registry_failure.as_ref()) {
        failure.metadata()
    } else if inventory.invalid != 0 {
        "reason=row_decode_error stage=decode".into()
    } else if inventory.truncated {
        "reason=row_limit stage=inventory".into()
    } else {
        "reason=ok".into()
    };
    let mut technical_details = format!(
        "{}\n{reason} elapsed_ms={}",
        inventory.detail(),
        started.elapsed().as_millis()
    );
    let state = tokio::time::timeout(
        TIMEOUT,
        db.collection::<Document>(STATE)
            .find_one(doc! {"_id": STATE_ID})
            .max_time(std::time::Duration::from_secs(2))
            .into_future(),
    )
    .await;
    let totals = match state {
        Ok(Ok(state)) => RecoveryTotals::read(state),
        Ok(Err(error)) => Err(ReadFailure::from_error(
            &AppError::DatabaseError(error),
            "ended_totals",
        )),
        Err(_) => Err(ReadFailure {
            reason: "deadline_exceeded",
            stage: "ended_totals",
            database_code: None,
        }),
    };
    let totals = match totals {
        Ok(totals) => {
            if !present && totals.ended_requests == 0 && totals.holdless_ended_requests == 0 {
                return Ok(vec![]);
            }
            technical_details.push_str(&format!("\nended_requests={} ended_rows={}; remaining rows and requests are classified above", totals.ended_requests, totals.ended_rows));
            technical_details.push_str(&format!(
                "\nholdless_ended_requests={} holdless_ended_rows={}",
                totals.holdless_ended_requests, totals.holdless_ended_rows
            ));
            Some(totals)
        }
        Err(failure) => {
            technical_details.push_str(&format!(
                "\nended totals unavailable; {}",
                failure.detail(started.elapsed().as_millis())
            ));
            None
        }
    };
    let (severity, detail, remediation) = inventory.presentation(totals.as_ref(), failure.as_ref());
    Ok(vec![super::super::BillingStartupDiagnostic {
        code: "billing_stuck_requests".into(),
        severity,
        technical_details: Some(technical_details),
        summary: "Sent requests with remaining holds".into(),
        detail,
        remediation,
        detected_at: chrono::Utc::now(),
    }])
}

// Only stable metadata is exposed: driver messages can contain query values.
#[derive(Clone, Debug)]
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
