//! Read-only inventory of unfinished funded rows. Age alone is not abandonment.
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Mutex,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
    options::{Hint, ReadConcern},
};
use serde::Deserialize;

use super::{GRANTS, METERS, PERIODS, WALLETS, live_index};
use crate::{
    db::next_document,
    errors::{AppError, AppResult},
    models::{
        credit_grant::CreditGrantSettlementLock,
        credits::Credits,
        service_billing::BillingMetric,
        usage_allowance_period::AllowanceSettlementLock,
        usage_meter::{UsageHold, UsageStatus},
    },
};

const MAX_ROWS: usize = 10_000;
const MAX_ALLOCATIONS: usize = 128;
const QUERY_TIMEOUT: Duration = Duration::from_secs(3);
const CLIENT_TIMEOUT: Duration = Duration::from_secs(4);
const MAX_READ_ATTEMPTS: usize = 8;

// Projection only; unrelated tax/policy/provider data is deliberately not read.
// Amounts and allocation/lock shapes use the same decoders as the hold verifier.
#[derive(Deserialize)]
struct Row {
    #[serde(flatten)]
    hold: UsageHold,
    #[serde(default)]
    wallet_id: Option<String>,
    status: UsageStatus,
    metric: BillingMetric,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    observed_at: DateTime<Utc>,
    resources: Vec<Document>,
}

#[derive(Deserialize)]
struct Resource<L> {
    #[serde(rename = "_id")]
    id: String,
    owner: String,
    active_settlement: Option<L>,
}

#[derive(Deserialize)]
struct WalletLock {
    row_id: String,
    #[serde(with = "crate::models::credits::whole")]
    reserved_credits: Credits,
    applied: bool,
}

fn invalid() -> AppError {
    AppError::Internal("Incomplete hold diagnostic row".into())
}

fn resource<L: serde::de::DeserializeOwned>(row: &Row, id: &str) -> AppResult<Resource<L>> {
    let value = row
        .resources
        .iter()
        .find(|r| r.get_str("_id").ok() == Some(id))
        .ok_or_else(invalid)?;
    let value: Resource<L> = bson::from_document(value.clone()).map_err(|_| invalid())?;
    if value.owner != row.hold.billing_owner_id || value.id != id {
        return Err(invalid());
    }
    Ok(value)
}

fn recorded(row: &Row, kind: &str, id: &str) -> bool {
    let settle = format!("{}:{kind}:{id}", row.hold.id);
    let release = format!("{}:{kind}-release:{id}", row.hold.id);
    let funding = row.hold.funding.as_ref();
    if kind == "grant" {
        funding.is_some_and(|f| {
            f.grant_consumptions
                .iter()
                .any(|r| r.operation_id == settle || r.operation_id == release)
        })
    } else {
        funding.is_some_and(|f| {
            f.allowance_consumptions
                .iter()
                .any(|r| r.operation_id == settle || r.operation_id == release)
        })
    }
}

fn applied(
    row: &Row,
    kind: &str,
    id: &str,
    operation: &str,
    usage_id: &str,
    applied: bool,
) -> bool {
    applied
        && usage_id == row.hold.id
        && (operation == format!("{}:{kind}:{id}", row.hold.id)
            || operation == format!("{}:{kind}-release:{id}", row.hold.id))
}

// Only positive, still-outstanding holds contribute resource counts. A row
// awaiting its final funding marker can have receipts for all holds: row=1,
// held=0, resources=0. This keeps the protocol handoff visible without calling
// already-returned capacity a hold.
fn outstanding(collection: &str, row: &Row) -> AppResult<BTreeMap<String, Credits>> {
    let mut holds = BTreeMap::new();
    let mut seen = BTreeSet::new();
    let mut add = |id: &str, amount: Credits| -> AppResult<()> {
        if amount < Credits::ZERO || !seen.insert(id.to_owned()) {
            return Err(invalid());
        }
        if amount > Credits::ZERO {
            holds.insert(id.to_owned(), amount);
        }
        Ok(())
    };
    match collection {
        WALLETS => {
            let id = row.wallet_id.as_deref().ok_or_else(invalid)?;
            let resource = resource::<WalletLock>(row, id)?;
            let mut amount = row.hold.reserved_credits;
            if let Some(lock) = resource
                .active_settlement
                .filter(|l| l.row_id == row.hold.id)
            {
                if lock.reserved_credits != amount {
                    return Err(invalid());
                }
                if lock.applied {
                    amount = Credits::ZERO;
                }
            }
            add(id, amount)?;
        }
        GRANTS => {
            let funding = row.hold.funding.as_ref().ok_or_else(invalid)?;
            for reservation in &funding.grant_reservations {
                let id = &reservation.grant_id;
                let resource = resource::<CreditGrantSettlementLock>(row, id)?;
                let mut amount = reservation.amount;
                if amount < Credits::ZERO {
                    return Err(invalid());
                }
                if recorded(row, "grant", id) {
                    amount = Credits::ZERO;
                } else if let Some(lock) = resource.active_settlement.filter(|l| {
                    applied(
                        row,
                        "grant",
                        id,
                        &l.operation_id,
                        &l.usage_row_id,
                        l.applied,
                    )
                }) {
                    if lock.reserved != amount
                        || lock.service_limit_key != reservation.service_limit_key
                    {
                        return Err(invalid());
                    }
                    amount = Credits::ZERO;
                }
                add(id, amount)?;
            }
        }
        PERIODS => {
            let funding = row.hold.funding.as_ref().ok_or_else(invalid)?;
            for reservation in &funding.allowance_reservations {
                let id = &reservation.period_id;
                let resource = resource::<AllowanceSettlementLock>(row, id)?;
                let mut amount = Credits::from_whole(reservation.quantity);
                if amount < Credits::ZERO {
                    return Err(invalid());
                }
                if recorded(row, "allowance", id) {
                    amount = Credits::ZERO;
                } else if let Some(lock) = resource.active_settlement.filter(|l| {
                    applied(
                        row,
                        "allowance",
                        id,
                        &l.operation_id,
                        &l.usage_row_id,
                        l.applied,
                    )
                }) {
                    if lock.reserved_quantity != reservation.quantity {
                        return Err(invalid());
                    }
                    amount = Credits::ZERO;
                }
                add(id, amount)?;
            }
        }
        _ => return Err(invalid()),
    }
    Ok(holds)
}

fn pipeline(collection: &str, limit: usize) -> Vec<Document> {
    let (filter, path, owner) = match collection {
        WALLETS => (
            doc! {"released":false,"wallet_id":{"$type":"string"},"reserved_credits":{"$ne":Credits::ZERO}},
            "wallet_id",
            "owner_id",
        ),
        GRANTS => (
            doc! {"funding.settled":{"$ne":true},"funding.grant_reservations.grant_id":{"$exists":true}},
            "funding.grant_reservations.grant_id",
            "recipient_user_id",
        ),
        _ => (
            doc! {"funding.settled":{"$ne":true},"funding.allowance_reservations.period_id":{"$exists":true}},
            "funding.allowance_reservations.period_id",
            "owner_user_id",
        ),
    };
    let mut projection = doc! {"_id":1,"billing_owner_id":1,"wallet_id":1,"status":1,"metric":1,"created_at":1,"reserved_credits":1,"funding.settled":1,"observed_at":"$$NOW"};
    // Bound arrays before joining, without converting malformed/null values to
    // valid empty evidence. A sliced oversize array is rejected by the decoder.
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
    vec![
        doc! {"$match":filter},
        doc! {"$limit":(limit+1) as i64},
        doc! {"$project":projection},
        doc! {"$lookup":{"from":collection,"localField":path,"foreignField":"_id",
        "pipeline":[{"$project":{"_id":1,"owner":format!("${owner}"),"active_settlement":1}}],"as":"resources"}},
    ]
}

#[derive(Default)]
struct Group {
    rows: u64,
    held: Credits,
    resources: BTreeSet<String>,
    wallets: BTreeSet<String>,
    owners: BTreeSet<String>,
}

impl Group {
    fn add(&mut self, row: &Row, holds: &BTreeMap<String, Credits>) -> AppResult<()> {
        let total = holds
            .values()
            .try_fold(Credits::ZERO, |sum, value| sum.checked_add(*value))?;
        self.held = self.held.checked_add(total)?;
        self.rows += 1;
        self.resources.extend(holds.keys().cloned());
        if let Some(wallet) = &row.wallet_id {
            self.wallets.insert(wallet.clone());
        }
        self.owners.insert(row.hold.billing_owner_id.clone());
        Ok(())
    }
    fn detail(&self, unit: &str) -> String {
        format!(
            "unit={unit} rows={} held={} resources={} wallets={} owners={}",
            self.rows,
            self.held,
            self.resources.len(),
            self.wallets.len(),
            self.owners.len()
        )
    }
}

#[derive(Default)]
struct Inventory {
    groups: BTreeMap<(String, usize, String), Group>,
    totals: BTreeMap<String, Group>,
    rows: usize,
    invalid: usize,
    truncated: bool,
}

const AGE_LABELS: [&str; 5] = [
    "under_1h",
    "1h_to_24h",
    "1d_to_7d",
    "over_7d",
    "future_timestamp",
];

fn age(row: &Row) -> usize {
    let seconds = (row.observed_at - row.created_at).num_seconds();
    match seconds {
        ..0 => 4,
        0..3600 => 0,
        3600..86400 => 1,
        86400..604800 => 2,
        _ => 3,
    }
}

impl Inventory {
    fn add(&mut self, collection: &str, value: Document) -> AppResult<()> {
        self.rows += 1;
        let parsed: AppResult<(Row, BTreeMap<String, Credits>)> = (|| {
            let row: Row = bson::from_document(value).map_err(|_| invalid())?;
            if row.hold.funding.as_ref().is_some_and(|f| {
                [
                    f.grant_reservations.len(),
                    f.allowance_reservations.len(),
                    f.grant_consumptions.len(),
                    f.allowance_consumptions.len(),
                ]
                .iter()
                .any(|n| *n > MAX_ALLOCATIONS)
            }) {
                return Err(invalid());
            }
            let holds = outstanding(collection, &row)?;
            Ok((row, holds))
        })();
        let Ok((row, holds)) = parsed else {
            self.invalid += 1;
            return Ok(());
        };
        let status = serde_json::to_value(row.status)
            .map_err(|_| invalid())?
            .as_str()
            .ok_or_else(invalid)?
            .to_owned();
        let unit = if collection == PERIODS {
            serde_json::to_value(row.metric)
                .map_err(|_| invalid())?
                .as_str()
                .ok_or_else(invalid)?
                .to_owned()
        } else {
            "credits".into()
        };
        self.groups
            .entry((status, age(&row), unit.clone()))
            .or_default()
            .add(&row, &holds)?;
        self.totals.entry(unit).or_default().add(&row, &holds)?;
        Ok(())
    }
    fn detail(&self, kind: &str) -> Vec<String> {
        let complete = !self.truncated && self.invalid == 0;
        let mut lines = vec![format!(
            "{kind}: complete={complete} scanned_rows={} invalid_rows={} row_limit={} truncated={}; counts and sums cover decoded rows only",
            self.rows, self.invalid, MAX_ROWS, self.truncated
        )];
        let resources: BTreeSet<_> = self.totals.values().flat_map(|g| &g.resources).collect();
        let wallets: BTreeSet<_> = self.totals.values().flat_map(|g| &g.wallets).collect();
        let owners: BTreeSet<_> = self.totals.values().flat_map(|g| &g.owners).collect();
        lines.push(format!(
            "{kind} distinct resources={} wallets={} owners={}",
            resources.len(),
            wallets.len(),
            owners.len()
        ));
        for (unit, group) in &self.totals {
            lines.push(format!("{kind} total {}", group.detail(unit)));
        }
        for ((status, age, unit), group) in &self.groups {
            let age = AGE_LABELS[*age];
            lines.push(format!(
                "{kind} status={status} age={age} {}",
                group.detail(unit)
            ));
        }
        lines
    }
}

async fn scan_once(
    db: &Database,
    session: &mut ClientSession,
    collection: &str,
    limit: usize,
    stage: &Mutex<&'static str>,
) -> AppResult<Inventory> {
    *stage.lock().unwrap() = "aggregate";
    let mut cursor = db
        .collection::<Document>(METERS)
        .aggregate(pipeline(collection, limit))
        .hint(Hint::Keys(live_index(collection)))
        .max_time(QUERY_TIMEOUT)
        .batch_size(256)
        .session(&mut *session)
        .await?;
    let mut result = Inventory::default();
    loop {
        *stage.lock().unwrap() = "cursor";
        let Some(row) = next_document(&mut cursor, session).await? else {
            break;
        };
        if result.rows == limit {
            result.truncated = true;
            break;
        }
        *stage.lock().unwrap() = "decode";
        result.add(collection, row)?;
        #[cfg(test)]
        if result.rows == 1 {
            tests::pause_cursor().await;
        }
    }
    Ok(result)
}

pub(in crate::services::billing) fn expired_snapshot(error: &AppError) -> bool {
    matches!(error, AppError::DatabaseError(error)
        if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command)
            if matches!(command.code, 239 | 246)))
}

pub(in crate::services::billing) async fn retry_snapshot_read<T, F, Fut>(
    mut read: F,
) -> AppResult<T>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = AppResult<T>>,
{
    for attempt in 1..=MAX_READ_ATTEMPTS {
        match read().await {
            Err(error) if attempt < MAX_READ_ATTEMPTS && expired_snapshot(&error) => {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            result => return result,
        }
    }
    unreachable!()
}

// Only stable metadata is exposed: driver messages may contain query values.
#[derive(Debug)]
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
                        2 if command.message.contains("hint") => "index_unavailable",
                        _ => "database_error",
                    }
                }
                ErrorKind::BsonDeserialization(_) => "row_decode_error",
                _ => "database_unavailable",
            },
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
}

async fn scan(db: &Database, collection: &str, limit: usize) -> Result<Inventory, ReadFailure> {
    let stage = Mutex::new("session");
    // Pin each kind's rows and joined locks across the aggregate and getMore.
    // Retrying an outside-transaction snapshot cannot protect its history.
    // Every retry discards its totals and starts a fresh read-only transaction.
    let result = tokio::time::timeout(
        CLIENT_TIMEOUT,
        retry_snapshot_read(|| async {
            *stage.lock().unwrap() = "session";
            let mut session = db.client().start_session().await?;
            *stage.lock().unwrap() = "snapshot_start";
            session
                .start_transaction()
                .read_concern(ReadConcern::snapshot())
                .await?;
            let result = scan_once(db, &mut session, collection, limit, &stage).await;
            let failed_stage = *stage.lock().unwrap();
            *stage.lock().unwrap() = "snapshot_release";
            // No writes to commit. Cancellation also queues an abort on drop.
            let cleanup = session.abort_transaction().await;
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
    let stage = *stage.lock().unwrap();
    match result {
        Ok(Ok(inventory)) => Ok(inventory),
        Ok(Err(error)) => Err(ReadFailure::from_error(&error, stage)),
        Err(_) => Err(ReadFailure {
            reason: "deadline_exceeded",
            stage,
            database_code: None,
        }),
    }
}

impl Inventory {
    fn sentence(&self, kind: &str) -> String {
        use super::super::diagnostic_text::{counted, oldest};
        let decoded = (self.rows - self.invalid) as u64;
        if decoded == 0 {
            return String::new();
        }
        let resources: BTreeSet<_> = self.totals.values().flat_map(|g| &g.resources).collect();
        if resources.is_empty() {
            return format!(
                "{} unfinished {kind} billing {} {} no remaining holds.",
                super::super::diagnostic_text::number(decoded),
                if decoded == 1 { "row" } else { "rows" },
                if decoded == 1 { "has" } else { "have" },
            );
        }
        let (held, singular, plural) = match kind {
            "wallet" => ("Wallet credits", "wallet", "wallets"),
            "grant" => ("Grant credits", "grant", "grants"),
            "allowance" => ("Allowance units", "period", "periods"),
            _ => unreachable!(),
        };
        let age = oldest(
            self.groups
                .iter()
                .filter(|(_, group)| group.held > Credits::ZERO)
                .map(|((_, age, _), _)| AGE_LABELS[*age]),
        );
        format!(
            "{held} are held in {} across {}. {age}",
            counted(resources.len() as u64, singular, plural),
            counted(decoded, "unfinished billing row", "unfinished billing rows")
        )
    }
}

struct KindScan {
    kind: &'static str,
    inventory: Result<Inventory, ReadFailure>,
    elapsed_ms: u128,
}

fn describe(
    scans: Vec<KindScan>,
    detected_at: DateTime<Utc>,
) -> Option<super::super::BillingStartupDiagnostic> {
    use super::super::diagnostic_text::{InventoryIssue, inventory_status};
    let mut issues = BTreeSet::new();
    let mut lines = Vec::new();
    let mut problems = Vec::new();
    let mut sentences = Vec::new();
    let mut present = false;
    let mut unread = false;
    let mut partial = false;
    for KindScan {
        kind,
        inventory,
        elapsed_ms,
    } in scans
    {
        match inventory {
            Ok(inventory) => {
                present |= inventory.rows > 0 || inventory.truncated;
                lines.extend(inventory.detail(kind));
                let reason = if inventory.invalid != 0 {
                    issues.insert(InventoryIssue::InvalidRows);
                    problems.push(format!(
                        "{} {kind} {} could not be decoded.",
                        super::super::diagnostic_text::number(inventory.invalid as u64),
                        if inventory.invalid == 1 {
                            "row"
                        } else {
                            "rows"
                        }
                    ));
                    "reason=row_decode_error stage=decode"
                } else if inventory.truncated {
                    "reason=row_limit stage=inventory"
                } else {
                    "reason=ok"
                };
                if inventory.truncated {
                    issues.insert(InventoryIssue::RowLimit);
                    problems.push(format!(
                        "The {kind} scan stopped before all rows were read."
                    ));
                }
                partial |= inventory.invalid != 0 || inventory.truncated;
                lines.push(format!("{kind}: {reason} elapsed_ms={elapsed_ms}"));
                let sentence = inventory.sentence(kind);
                if !sentence.is_empty() {
                    sentences.push(sentence);
                }
            }
            Err(failure) => {
                issues.insert(InventoryIssue::scan_failure(failure.reason));
                present = true;
                unread = true;
                problems.push(format!("The {kind} inventory could not be read."));
                lines.push(format!(
                    "{kind}: complete=false; {} elapsed_ms={elapsed_ms}; no totals available",
                    failure.metadata(),
                ));
            }
        }
    }
    if !present {
        return None;
    }
    let (severity, remediation) = inventory_status(&issues);
    if partial {
        problems.push("Counts cover only decoded rows.".into());
    }
    problems.extend(sentences);
    Some(super::super::BillingStartupDiagnostic {
        code: "billing_hold_backlog".into(),
        severity,
        technical_details: Some(lines.join("\n")),
        summary: if unread {
            "Billing hold inventory could not be fully read"
        } else {
            "Holds for unfinished requests"
        }
        .into(),
        detail: problems.join(" "),
        remediation,
        detected_at,
    })
}

pub(super) async fn diagnostics(
    db: &Database,
) -> AppResult<Vec<super::super::BillingStartupDiagnostic>> {
    let mut scans = Vec::new();
    for (kind, collection) in [
        ("wallet", WALLETS),
        ("grant", GRANTS),
        ("allowance", PERIODS),
    ] {
        let started = Instant::now();
        let inventory = scan(db, collection, MAX_ROWS).await;
        scans.push(KindScan {
            kind,
            inventory,
            elapsed_ms: started.elapsed().as_millis(),
        });
    }
    Ok(describe(scans, Utc::now()).into_iter().collect())
}

#[cfg(test)]
mod presentation_tests;

/// Share the hold verifier's decoders, receipt and applied-lock subtraction.
/// The caller's one snapshot aggregate supplies the three indexed resource joins.
pub(in crate::services::billing) fn request_holds(
    value: &Document,
) -> AppResult<Vec<(&'static str, BTreeMap<String, Credits>)>> {
    let mut out = Vec::new();
    for (kind, collection, alias) in [
        ("wallet", WALLETS, "wallet_resources"),
        ("grant", GRANTS, "grant_resources"),
        ("allowance", PERIODS, "allowance_resources"),
    ] {
        let mut value = value.clone();
        value.insert("resources", value.get(alias).cloned().ok_or_else(invalid)?);
        let row: Row = bson::from_document(value).map_err(|_| invalid())?;
        if row.hold.funding.as_ref().is_some_and(|f| {
            [
                f.grant_reservations.len(),
                f.allowance_reservations.len(),
                f.grant_consumptions.len(),
                f.allowance_consumptions.len(),
            ]
            .iter()
            .any(|n| *n > MAX_ALLOCATIONS)
        }) {
            return Err(invalid());
        }
        let mut holds = BTreeMap::new();
        let present = match kind {
            "wallet" => row.hold.reserved_credits != Credits::ZERO,
            "grant" => row
                .hold
                .funding
                .as_ref()
                .is_some_and(|f| !f.settled && !f.grant_reservations.is_empty()),
            _ => row
                .hold
                .funding
                .as_ref()
                .is_some_and(|f| !f.settled && !f.allowance_reservations.is_empty()),
        };
        if present {
            holds = outstanding(collection, &row)?;
        }
        out.push((kind, holds));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::Bson;
    use std::sync::{Arc, Mutex};

    #[tokio::test]
    async fn integrity_inventory_uses_operator_sentences() {
        let (db, _) = fixture().await;
        db.collection::<Document>(METERS)
            .insert_one(row("request", "forwarded", 1))
            .await
            .unwrap();
        let result = diagnostics(&db).await.unwrap();
        db.drop().await.unwrap();
        assert!(
            !result[0].detail.contains("complete="),
            "{}",
            result[0].detail
        );
        assert!(result[0].detail.contains("under an hour old"));
    }

    tokio::task_local! {
        static CURSOR_PAUSE: Duration;
    }
    pub(super) async fn pause_cursor() {
        if let Ok(delay) = CURSOR_PAUSE.try_with(|delay| *delay) {
            tokio::time::sleep(delay).await;
        }
    }

    #[tokio::test]
    async fn hold_diagnostics_pins_slow_cursor_under_concurrent_writes() {
        let (db, commands) = fixture().await;
        db.collection::<Document>(METERS)
            .insert_many((0..600).map(|i| row(&format!("r-{i}"), "forwarded", 4000)))
            .await
            .unwrap();
        let read = CURSOR_PAUSE.scope(Duration::from_millis(100), async {
            for collection in [WALLETS, GRANTS, PERIODS] {
                let result = scan(&db, collection, 1000).await.unwrap();
                assert_eq!(result.rows, 600);
                assert_eq!(result.invalid, 0);
                let (unit, amount) = match collection {
                    WALLETS => ("credits", Credits::from_whole(1200)),
                    GRANTS => ("credits", "600.00048".parse().unwrap()),
                    _ => ("tokens", Credits::from_whole(600)),
                };
                assert_eq!(result.totals[unit].held, amount);
            }
        });
        // Keep advancing database time while the consumer is deliberately slow,
        // like a coverage-instrumented reader. No synthetic server error is used.
        let writer = async {
            loop {
                db.collection::<Document>(WALLETS)
                    .update_one(doc! {"_id":"w"}, doc! {"$inc":{"unrelated_revision":1}})
                    .await
                    .unwrap();
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        };
        tokio::pin!(writer);
        tokio::select! {
            () = read => {},
            () = &mut writer => unreachable!(),
        }
        assert!(
            commands
                .lock()
                .unwrap()
                .iter()
                .filter(|s| *s == "getMore")
                .count()
                >= 6
        );
        db.drop().await.unwrap();
    }

    #[test]
    fn hold_diagnostics_failure_metadata_is_stable_and_private() {
        for (code, message, reason) in [
            (50, "private-query-values", "query_timeout"),
            (262, "private-query-values", "query_timeout"),
            (239, "private-query-values", "snapshot_unavailable"),
            (246, "private-query-values", "snapshot_unavailable"),
            (27, "private-query-values", "index_unavailable"),
            (2, "hint: private-query-values", "index_unavailable"),
            (13, "private-query-values", "database_error"),
        ] {
            let command: mongodb::error::CommandError =
                bson::from_document(doc! {"code":code,"errmsg":message}).unwrap();
            let error = AppError::DatabaseError(mongodb::error::ErrorKind::Command(command).into());
            let failure = ReadFailure::from_error(&error, "cursor");
            assert_eq!(
                failure.metadata(),
                format!("reason={reason} stage=cursor database_code={code}")
            );
            assert!(!failure.metadata().contains("private-query-values"));
        }
    }

    #[tokio::test]
    async fn hold_diagnostics_timeout_aborts_snapshot_and_next_read_recovers() {
        let (db, commands) = fixture().await;
        db.collection::<Document>(METERS)
            .insert_one(row("r", "forwarded", 4000))
            .await
            .unwrap();
        commands.lock().unwrap().clear();
        let started = Instant::now();
        let result = CURSOR_PAUSE
            .scope(
                CLIENT_TIMEOUT + Duration::from_secs(1),
                scan(&db, GRANTS, 100),
            )
            .await;
        let failure = result.err().unwrap();
        assert_eq!(failure.reason, "deadline_exceeded");
        assert_eq!(failure.stage, "decode");
        assert_eq!(failure.database_code, None);
        assert!(started.elapsed() >= CLIENT_TIMEOUT);
        // Cancelling the query drops the session and the driver queues an abort.
        tokio::time::timeout(Duration::from_secs(2), async {
            while !commands
                .lock()
                .unwrap()
                .iter()
                .any(|name| name == "abortTransaction")
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let result = scan(&db, GRANTS, 100).await.unwrap();
        assert_eq!(result.totals["credits"].held, "1.0000008".parse().unwrap());
        db.drop().await.unwrap();
    }

    fn assert_inventory_commands(commands: &[String]) {
        let expired = commands
            .iter()
            .filter(|name| name.as_str() == "snapshot_expired")
            .count();
        let aborts = commands
            .iter()
            .filter(|name| name.as_str() == "abortTransaction")
            .count();
        let aggregates = commands
            .iter()
            .filter(|name| name.as_str() == "aggregate")
            .count();
        assert_eq!(
            commands.len(),
            aggregates + expired + aborts,
            "unexpected command: {commands:?}"
        );
        assert_eq!(
            aggregates - expired,
            3,
            "one successful read per kind: {commands:?}"
        );
        assert!(aggregates <= 3 * MAX_READ_ATTEMPTS);
        assert!(aborts >= 3 && aborts <= aggregates * 2, "{commands:?}");
        println!(
            "hold backlog commands: aggregates={aggregates} aborts={aborts} expired={expired}"
        );
    }

    fn snapshot_error(code: i32) -> AppError {
        let command: mongodb::error::CommandError =
            bson::from_document(doc! {"code":code}).unwrap();
        AppError::DatabaseError(mongodb::error::ErrorKind::Command(command).into())
    }

    #[tokio::test(start_paused = true)]
    async fn hold_diagnostics_snapshot_retry_policy_and_shared_deadline() {
        use std::cell::Cell;
        for code in [239, 246, 2, 13, 50] {
            let calls = Cell::new(0);
            let result = retry_snapshot_read(|| {
                calls.set(calls.get() + 1);
                std::future::ready(Err::<(), _>(snapshot_error(code)))
            })
            .await;
            assert_eq!(
                calls.get(),
                if matches!(code, 239 | 246) {
                    MAX_READ_ATTEMPTS
                } else {
                    1
                }
            );
            assert!(matches!(result, Err(AppError::DatabaseError(error))
                if matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Command(command) if command.code == code)));
        }
        let calls = Cell::new(0);
        let started = tokio::time::Instant::now();
        let result = tokio::time::timeout(
            CLIENT_TIMEOUT,
            retry_snapshot_read(|| async {
                calls.set(calls.get() + 1);
                tokio::time::sleep(Duration::from_secs(3)).await;
                Err::<(), _>(snapshot_error(239))
            }),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(calls.get(), 2);
        assert_eq!(started.elapsed(), CLIENT_TIMEOUT);
    }

    #[tokio::test]
    async fn hold_diagnostics_snapshot_restart_discards_totals_and_reads_new_lock_and_receipt() {
        use std::cell::Cell;
        let (db, _) = fixture().await;
        db.collection::<Document>(METERS)
            .insert_one(row("r", "finalized", 4000))
            .await
            .unwrap();
        let attempts = Cell::new(0);
        let result = retry_snapshot_read(|| async {
            attempts.set(attempts.get() + 1);
            let result = scan(&db, GRANTS, 100).await.map_err(|failure| AppError::Internal(failure.metadata()))?;
            if attempts.get() == 1 {
                assert_eq!(result.totals["credits"].held, "1.0000008".parse().unwrap());
                db.collection::<Document>(GRANTS).update_one(doc! {"_id":"g"},doc! {"$set":{"active_settlement":{
                    "operation_id":"r:grant:g","usage_row_id":"r","reserved":"0.0000008".parse::<Credits>().unwrap(),
                    "consume":Credits::ZERO,"applied":true,"updated_at":bson::DateTime::now(),
                }}}).await?;
                return Err(snapshot_error(239));
            }
            assert_eq!(result.totals["credits"].held, Credits::from_whole(1));
            if attempts.get() == 2 {
                // A failed cursor's accumulated totals must be discarded. The
                // next snapshot follows the lock-to-receipt handoff exactly once.
                db.collection::<Document>(METERS).update_one(doc! {"_id":"r"},doc! {"$set":{
                    "funding.grant_consumptions":[{"operation_id":"r:grant:g","grant_id":"g","amount":Credits::ZERO}],
                }}).await?;
                db.collection::<Document>(GRANTS).update_one(doc! {"_id":"g"},doc! {"$unset":{"active_settlement":""}}).await?;
                return Err(snapshot_error(246));
            }
            Ok(result)
        }).await.unwrap();
        assert_eq!(attempts.get(), 3);
        assert_eq!(result.rows, 1);
        assert_eq!(result.totals["credits"].held, Credits::from_whole(1));
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn hold_diagnostics_unavailable_is_explicit_and_does_not_hide_other_codes_or_latch() {
        let (db, _) = fixture().await;
        db.collection::<Document>("billing_rate_diagnostics")
            .insert_one(doc! {"_id":"p","rejected_metrics":["broken"]})
            .await
            .unwrap();
        let indexes = db.collection::<Document>(METERS);
        indexes
            .drop_index("funding.grant_reservations.grant_id_1_funding.settled_1__id_1")
            .await
            .unwrap();
        let result = super::super::super::startup_diagnostics(&db).await.unwrap();
        assert!(result.iter().any(|d| d.code == "billing_invalid_rates"));
        let inventory = result
            .iter()
            .find(|d| d.code == "billing_hold_backlog")
            .unwrap();
        assert!(inventory.summary.contains("could not be fully read"));
        assert!(
            inventory
                .technical_details.as_ref().unwrap()
                .contains("grant: complete=false; reason=index_unavailable stage=aggregate database_code=2 elapsed_ms=")
        );
        indexes
            .create_indexes(crate::db::billing_hold_indexes())
            .await
            .unwrap();
        let result = super::super::super::startup_diagnostics(&db).await.unwrap();
        assert!(result.iter().any(|d| d.code == "billing_invalid_rates"));
        assert!(result.iter().all(|d| d.code != "billing_hold_backlog"));
        db.drop().await.unwrap();
    }

    async fn fixture() -> (Database, Arc<Mutex<Vec<String>>>) {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let record = commands.clone();
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            "hold_backlog",
            mongodb::event::EventHandler::callback(move |event| {
                match event {
                    mongodb::event::command::CommandEvent::Started(event) => record.lock().unwrap().push(event.command_name),
                    mongodb::event::command::CommandEvent::Failed(event) if matches!(event.failure.kind.as_ref(), mongodb::error::ErrorKind::Command(error) if matches!(error.code, 239 | 246)) => record.lock().unwrap().push("snapshot_expired".into()),
                    _ => {},
                }
            }),
        )
        .await;
        db.collection::<Document>(METERS)
            .create_indexes(crate::db::billing_hold_indexes())
            .await
            .unwrap();
        db.collection::<Document>(WALLETS)
            .insert_one(doc! {"_id":"w","owner_id":"o"})
            .await
            .unwrap();
        db.collection::<Document>(GRANTS)
            .insert_many([
                doc! {"_id":"g","recipient_user_id":"o"},
                doc! {"_id":"g2","recipient_user_id":"o"},
            ])
            .await
            .unwrap();
        db.collection::<Document>(PERIODS)
            .insert_one(doc! {"_id":"p","owner_user_id":"o"})
            .await
            .unwrap();
        (db, commands)
    }
    fn row(id: &str, status: &str, age_secs: i64) -> Document {
        doc! {"_id":id,"billing_owner_id":"o","wallet_id":"w","status":status,
        "released":false,"forwarded":true,"quantity":Bson::Null,"metric":"tokens",
        "created_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(age_secs)),
        "reserved_credits":Credits::from_whole(2),"funding":{
            "settled":false,
            "grant_reservations":[{"grant_id":"g","amount":"0.0000008".parse::<Credits>().unwrap()},{"grant_id":"g2","amount":Credits::from_whole(1)}],
            "allowance_reservations":[{"allowance_id":"a","period_id":"p","quantity":1_i64}],
        }}
    }
    async fn inventory(db: &Database, collection: &str, limit: usize) -> Inventory {
        scan(db, collection, limit).await.unwrap()
    }
    #[tokio::test]
    async fn hold_diagnostics_status_age_counts_deduplicate_resources_and_are_read_only() {
        let (db, commands) = fixture().await;
        let mut rows = Vec::new();
        for status in [
            "reserved",
            "forwarded",
            "finalized",
            "failed",
            "abandoned",
            "dead_letter",
        ] {
            for age in [1, 3601, 86401, 604801] {
                rows.push(row(&format!("{status}-{age}"), status, age));
            }
        }
        let mut closed = row("closed", "dead_letter", 900000);
        closed.insert("released", true);
        closed
            .get_document_mut("funding")
            .unwrap()
            .insert("settled", true);
        rows.push(closed);
        let mut metering = row("metering", "forwarded", 900000);
        metering.insert("wallet_id", Bson::Null);
        metering.insert("reserved_credits", Credits::ZERO);
        metering.insert("funding", Bson::Null);
        rows.push(metering);
        db.collection::<Document>(METERS)
            .insert_many(rows)
            .await
            .unwrap();
        commands.lock().unwrap().clear();
        let result = diagnostics(&db).await.unwrap();
        let read_commands = commands.lock().unwrap().clone();
        let detail = &result[0].technical_details.as_ref().unwrap();
        assert!(
            detail.contains(
                "grant total unit=credits rows=24 held=24.0000192 resources=2 wallets=1 owners=1"
            ),
            "{detail}"
        );
        assert!(
            detail.contains(
                "allowance total unit=tokens rows=24 held=24 resources=1 wallets=1 owners=1"
            ),
            "{detail}"
        );
        assert!(
            detail.contains(
                "wallet total unit=credits rows=24 held=48 resources=1 wallets=1 owners=1"
            ),
            "{detail}"
        );
        for kind in ["wallet", "grant", "allowance"] {
            for status in [
                "reserved",
                "forwarded",
                "finalized",
                "failed",
                "abandoned",
                "dead_letter",
            ] {
                for age in ["under_1h", "1h_to_24h", "1d_to_7d", "over_7d"] {
                    assert!(
                        detail.contains(&format!("{kind} status={status} age={age}")),
                        "{detail}"
                    );
                }
            }
        }
        assert_inventory_commands(&read_commands);
        println!("HOLD_DIAGNOSTICS_COMMANDS populated={read_commands:?}; per_request_delta=0");
        assert_eq!(
            db.collection::<Document>(METERS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            26
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn hold_diagnostics_receipts_applied_locks_and_snapshot_handoffs() {
        let (db, _) = fixture().await;
        db.collection::<Document>(METERS)
            .insert_one(row("r", "finalized", 4000))
            .await
            .unwrap();
        // Unlike an outside-transaction snapshot session, a transaction pins
        // the historical view while these acknowledged lock steps advance it.
        let mut session = db.client().start_session().await.unwrap();
        session
            .start_transaction()
            .read_concern(ReadConcern::snapshot())
            .await
            .unwrap();
        assert_eq!(
            scan_once(&db, &mut session, GRANTS, 100, &Mutex::new("aggregate"))
                .await
                .unwrap()
                .totals["credits"]
                .held,
            "1.0000008".parse().unwrap()
        );
        for applied in [false, true] {
            db.collection::<Document>(GRANTS).update_one(doc! {"_id":"g"},doc! {"$set":{"active_settlement":{
                "operation_id":"r:grant:g","usage_row_id":"r","reserved":"0.0000008".parse::<Credits>().unwrap(),
                "consume":Credits::ZERO,"applied":applied,"updated_at":bson::DateTime::now(),
            }}}).await.unwrap();
            db.collection::<Document>(PERIODS).update_one(doc! {"_id":"p"},doc! {"$set":{"active_settlement":{
                "operation_id":"r:allowance-release:p","usage_row_id":"r","reserved_quantity":1_i64,
                "consume_quantity":0_i64,"applied":applied,"updated_at":bson::DateTime::now(),
            }}}).await.unwrap();
            db.collection::<Document>(WALLETS)
                .update_one(
                    doc! {"_id":"w"},
                    doc! {"$set":{"active_settlement":{
                        "row_id":"r","reserved_credits":Credits::from_whole(2),"applied":applied,
                    }}},
                )
                .await
                .unwrap();
            assert_eq!(
                inventory(&db, GRANTS, 100).await.totals["credits"].held,
                if applied {
                    Credits::from_whole(1)
                } else {
                    "1.0000008".parse().unwrap()
                }
            );
            assert_eq!(
                inventory(&db, PERIODS, 100).await.totals["tokens"].held,
                Credits::from_whole(i64::from(!applied))
            );
            assert_eq!(
                inventory(&db, WALLETS, 100).await.totals["credits"].held,
                Credits::from_whole(2 * i64::from(!applied))
            );
        }
        // The original read snapshot still sees the un-applied resource and row.
        assert_eq!(
            scan_once(&db, &mut session, GRANTS, 100, &Mutex::new("aggregate"))
                .await
                .unwrap()
                .totals["credits"]
                .held,
            "1.0000008".parse().unwrap()
        );
        session.abort_transaction().await.unwrap();
        db.collection::<Document>(METERS).update_one(doc! {"_id":"r"},doc! {"$set":{
            "released":true,
            "funding.grant_consumptions":[{"operation_id":"r:grant:g","grant_id":"g","amount":Credits::ZERO}],
            "funding.allowance_consumptions":[{"operation_id":"r:allowance-release:p","allowance_id":"a","period_id":"p","quantity":0_i64}],
        }}).await.unwrap();
        for collection in [WALLETS, GRANTS, PERIODS] {
            db.collection::<Document>(collection)
                .update_many(doc! {}, doc! {"$unset":{"active_settlement":""}})
                .await
                .unwrap();
        }
        assert_eq!(inventory(&db, WALLETS, 100).await.rows, 0);
        assert_eq!(
            inventory(&db, GRANTS, 100).await.totals["credits"].held,
            Credits::from_whole(1)
        );
        let allowance = inventory(&db, PERIODS, 100).await;
        assert_eq!(allowance.totals["tokens"].rows, 1);
        assert_eq!(allowance.totals["tokens"].held, Credits::ZERO);
        assert!(allowance.totals["tokens"].resources.is_empty());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn hold_diagnostics_malformed_missing_legacy_and_bounded_rows() {
        let (db, _) = fixture().await;
        let mut legacy = row("legacy", "forwarded", 4000);
        legacy.get_document_mut("funding").unwrap().insert(
            "grant_reservations",
            vec![doc! {"grant_id":"g","amount_micros":2_i64,"amount":Credits::from_whole(3)}],
        );
        let mut missing = row("missing", "forwarded", 4000);
        missing.get_document_mut("funding").unwrap().insert(
            "grant_reservations",
            vec![doc! {"grant_id":"absent","amount":Credits::from_whole(1)}],
        );
        let mut bad = row("bad", "forwarded", 4000);
        bad.insert("created_at", "broken");
        let mut oversize = row("oversize", "forwarded", 4000);
        oversize.get_document_mut("funding").unwrap().insert(
            "grant_reservations",
            vec![doc! {"grant_id":"g","amount":Credits::from_whole(1)}; MAX_ALLOCATIONS + 1],
        );
        db.collection::<Document>(METERS)
            .insert_many([legacy, missing, bad, oversize])
            .await
            .unwrap();
        let result = inventory(&db, GRANTS, 100).await;
        assert_eq!(result.rows, 4);
        assert_eq!(result.invalid, 3);
        assert_eq!(result.totals["credits"].held, Credits::from_whole(3));
        assert!(result.detail("grant")[0].contains("complete=false"));
        let bounded = inventory(&db, GRANTS, 1).await;
        assert_eq!(bounded.rows, 1);
        assert!(bounded.truncated);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn hold_diagnostics_no_wallet_separate_allowance_units_and_query_failure() {
        let (db, _) = fixture().await;
        let mut first = row("no-wallet", "forwarded", 4000);
        first.insert("wallet_id", Bson::Null);
        let mut second = row("images", "reserved", 10);
        second.insert("metric", "images");
        second.get_document_mut("funding").unwrap().insert(
            "allowance_reservations",
            vec![doc! {"allowance_id":"a2","period_id":"p2","quantity":2_i64}],
        );
        db.collection::<Document>(PERIODS)
            .insert_one(doc! {"_id":"p2","owner_user_id":"o"})
            .await
            .unwrap();
        db.collection::<Document>(METERS)
            .insert_many([first, second])
            .await
            .unwrap();
        let result = diagnostics(&db).await.unwrap();
        let detail = &result[0].technical_details.as_ref().unwrap();
        assert!(
            detail.contains(
                "allowance total unit=tokens rows=1 held=1 resources=1 wallets=0 owners=1"
            ),
            "{detail}"
        );
        assert!(
            detail.contains(
                "allowance total unit=images rows=1 held=2 resources=1 wallets=1 owners=1"
            ),
            "{detail}"
        );
        assert!(
            detail.contains("allowance distinct resources=2 wallets=1 owners=1"),
            "{detail}"
        );
        db.collection::<Document>(METERS)
            .drop_index("funding.grant_reservations.grant_id_1_funding.settled_1__id_1")
            .await
            .unwrap();
        let failed = diagnostics(&db).await.unwrap();
        assert!(
            failed[0]
                .technical_details.as_ref().unwrap()
                .contains("grant: complete=false; reason=index_unavailable stage=aggregate database_code=2 elapsed_ms=")
        );
        assert!(
            failed[0]
                .technical_details
                .as_ref()
                .unwrap()
                .contains("allowance: complete=true")
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn hold_diagnostics_query_plans_are_indexed_and_empty_cost_is_constant() {
        let (db, commands) = fixture().await;
        let mut closed = Vec::new();
        for i in 0..3000 {
            let mut r = row(&format!("history-{i}"), "finalized", 900000);
            r.insert("released", true);
            r.get_document_mut("funding")
                .unwrap()
                .insert("settled", true);
            closed.push(r);
        }
        db.collection::<Document>(METERS)
            .insert_many(closed)
            .await
            .unwrap();
        commands.lock().unwrap().clear();
        assert!(diagnostics(&db).await.unwrap().is_empty());
        assert_inventory_commands(&commands.lock().unwrap());
        db.collection::<Document>(METERS)
            .insert_one(row("live", "forwarded", 4000))
            .await
            .unwrap();
        for collection in [WALLETS, GRANTS, PERIODS] {
            let explain = db
                .run_command(
                    doc! {"explain":{"aggregate":METERS,"pipeline":pipeline(collection,MAX_ROWS),
                    "hint":live_index(collection),"cursor":{}},"verbosity":"executionStats"},
                )
                .await
                .unwrap();
            let text = format!("{explain:?}");
            assert!(!text.contains("COLLSCAN"), "{text}");
            assert!(text.contains("IXSCAN"), "{text}");
            assert!(text.contains("isPartial\": Boolean(true)"), "{text}");
            let cursor = explain
                .get_array("stages")
                .unwrap()
                .iter()
                .filter_map(Bson::as_document)
                .find_map(|stage| stage.get_document("$cursor").ok())
                .unwrap();
            let stats = cursor.get_document("executionStats").unwrap();
            let number = |key: &str| {
                stats
                    .get(key)
                    .and_then(|v| v.as_i64().or_else(|| v.as_i32().map(i64::from)))
                    .unwrap()
            };
            assert!(
                number("totalKeysExamined") <= 6,
                "settled history must not dominate the lookup: {text}"
            );
            assert_eq!(number("totalDocsExamined"), 1, "{text}");
            println!(
                "HOLD_DIAGNOSTICS_EXPLAIN {collection}: keys={} docs={} {text}",
                number("totalKeysExamined"),
                number("totalDocsExamined")
            );
        }
        db.drop().await.unwrap();
    }
}
