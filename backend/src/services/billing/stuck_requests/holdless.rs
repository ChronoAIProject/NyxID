//! Bounded housekeeping for dead requests with no remaining resources or final evidence.
use super::*;
use crate::{
    models::usage_meter::{UsageMeterRow, UsageStatus},
    services::{audit_service, billing::recovery_batch::Failures, transaction_service},
};
use mongodb::options::Hint;
use std::{
    collections::{BTreeMap, BTreeSet},
    time::Duration,
};

const BATCH_ROWS: i64 = 100;
const BUDGET: Duration = Duration::from_secs(1);
const LANE: &str = "holdless";
const EVENT: &str = "billing_holdless_requests_ended";

pub(super) async fn recover(db: &mongodb::Database) -> AppResult<()> {
    if !super::super::reconcile::continue_reconcile() {
        return Ok(());
    }
    let mut failures = Failures::default();
    let mut waits = BTreeMap::new();
    #[cfg(test)]
    if super::holdless_tests::UNTIMED.try_with(|_| ()).is_ok() {
        run(db, &mut failures, &mut waits).await?;
        return failures.finish(());
    }
    let result = match tokio::time::timeout(BUDGET, run(db, &mut failures, &mut waits)).await {
        Ok(result) => result,
        Err(_) => {
            *waits.entry("pass_time_budget").or_insert(0_u64) += 1;
            Ok(())
        }
    };
    if !waits.is_empty() {
        tracing::debug!(?waits, "Holdless request cleanup deferred");
    }
    result?;
    failures.finish(())
}

pub(super) fn page_filter(after: &str) -> Document {
    let mut query = doc! {"forwarded":true,"released":false};
    if !after.is_empty() {
        query.insert("_id", doc! {"$gt":after});
    }
    query
}

async fn run(
    db: &mongodb::Database,
    failures: &mut Failures,
    waits: &mut BTreeMap<&'static str, u64>,
) -> AppResult<()> {
    let state = db
        .collection::<Document>(STATE)
        .find_one(doc! {"_id":STATE_ID})
        .await?;
    if state
        .as_ref()
        .is_some_and(|s| present(s, "holdless_summary"))
    {
        // Finish the interrupted pass before opening another audit summary.
        failures.unit(flush_summary(db)).await;
        return Ok(());
    }
    let mut session = db.client().start_session().await?;
    match registry(db, &mut session).await {
        Ok(_) => {}
        Err(AppError::Internal(_)) => {
            *waits.entry("registry_unavailable").or_default() += 1;
            return Ok(());
        }
        Err(error) => return Err(error),
    }
    drop(session);
    let after = state
        .as_ref()
        .and_then(|s| s.get_str("holdless_cursor").ok())
        .unwrap_or("");
    let limit = super::super::recovery_batch::limit(BATCH_ROWS);
    let rows: Vec<Document> = db
        .collection::<Document>(METERS)
        .find(page_filter(after))
        .hint(Hint::Name(inventory::INDEX.into()))
        .sort(doc! {"_id":1})
        .projection(doc! {"_id":1,"billing_request_id":1})
        .limit(limit)
        .max_time(BUDGET)
        .await?
        .try_collect()
        .await?;
    if rows.is_empty() {
        if !after.is_empty() {
            save_cursor(db, "").await?;
        }
        return Ok(());
    }
    let pass = uuid::Uuid::new_v4().to_string();
    let mut seen = BTreeSet::new();
    let mut visited = 0;
    let mut ended = false;
    for candidate in &rows {
        if !super::super::reconcile::continue_reconcile() {
            break;
        }
        let Ok(id) = candidate.get_str("_id") else {
            failures.note(unavailable());
            visited += 1;
            continue;
        };
        let request = candidate
            .get_str("billing_request_id")
            .map_err(|_| unavailable());
        let result = match request {
            Ok(request) if seen.insert(request.to_owned()) => publish(db, request, id, &pass).await,
            Ok(_) => Ok(terminal::Outcome::Deferred("sibling_already_visited")),
            Err(error) => Err(error),
        };
        match result {
            Ok(terminal::Outcome::Published(rows)) => {
                ended = true;
                failures
                    .unit(super::super::meter::settle_persisted(db, rows))
                    .await;
            }
            Ok(terminal::Outcome::Deferred(reason)) => {
                *waits.entry(reason).or_default() += 1;
                save_cursor(db, id).await?;
            }
            Err(error) => {
                if recovery::publication_reason(&error) == "publication_conflict" {
                    *waits.entry("publication_conflict").or_default() += 1;
                } else {
                    failures.note(error);
                }
                save_cursor(db, id).await?;
            }
        }
        visited += 1;
    }
    if super::super::reconcile::continue_reconcile()
        && visited == rows.len()
        && rows.len() < limit as usize
    {
        save_cursor(db, "").await?;
    }
    if ended && super::super::reconcile::continue_reconcile() {
        failures.unit(flush_summary(db)).await;
    }
    Ok(())
}

async fn save_cursor(db: &mongodb::Database, after: &str) -> AppResult<()> {
    db.collection::<Document>(STATE)
        .update_one(
            doc! {"_id":STATE_ID},
            doc! {"$set":{"holdless_cursor":after}},
        )
        .upsert(true)
        .await?;
    Ok(())
}

async fn publish(
    db: &mongodb::Database,
    request: &str,
    cursor: &str,
    pass: &str,
) -> AppResult<terminal::Outcome> {
    let context = (
        db.clone(),
        request.to_owned(),
        cursor.to_owned(),
        pass.to_owned(),
    );
    let result =
        transaction_service::run_db_only(db, &context, |tx, (db, request, cursor, pass)| {
            Box::pin(in_session(db, tx, request, cursor, pass))
        })
        .await?;
    #[cfg(test)]
    if matches!(result, terminal::Outcome::Published(_)) {
        test_point(Phase::AfterCommit).await;
    }
    Ok(result)
}

async fn in_session(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    request: &str,
    cursor_id: &str,
    pass: &str,
) -> AppResult<terminal::Outcome> {
    let registry = match registry(db, tx).await {
        Ok(registry) => registry,
        Err(AppError::Internal(_)) => {
            return Ok(terminal::Outcome::Deferred("registry_unavailable"));
        }
        Err(error) => return Err(error),
    };
    let collection = db.collection::<Document>(METERS);
    let mut cursor = collection
        .find(doc! {"billing_request_id":request})
        .hint(Hint::Name("billing_request_id_1".into()))
        .limit(129)
        .max_time(BUDGET)
        .session(&mut *tx)
        .await?;
    let mut raw = read_documents(&mut cursor, tx).await?;
    if raw.len() > 128 {
        return Err(unavailable());
    }
    if raw.is_empty() {
        return Ok(terminal::Outcome::Deferred("request_disappeared"));
    }
    if let Some(reason) = request_blocker(&raw) {
        return Ok(terminal::Outcome::Deferred(reason));
    }
    for row in &raw {
        if present(row, "quantity")
            || row
                .get_document("usage_checkpoint")
                .ok()
                .is_some_and(|c| c.get_bool("final_report").unwrap_or(false))
        {
            return Ok(terminal::Outcome::Deferred("recorded_final_evidence"));
        }
        let class = classify_for(row, Some(&registry), true)?;
        if !class.eligible {
            return Ok(terminal::Outcome::Deferred(class.label));
        }
    }
    read_resources(db, tx, &mut raw, registry.now).await?;
    let mut rows = Vec::new();
    for raw in &raw {
        let row: UsageMeterRow = super::super::recovery_batch::decode(raw)?;
        if !zero_returns(&row)? {
            return Ok(terminal::Outcome::Deferred("recorded_funding_or_claim"));
        }
        if row.org_channel_guest.is_some()
            && let Some(hold) = db
                .collection::<Document>(crate::models::org_channel::HOLDS)
                .find_one(doc! {"_id":&row.id})
                .session(&mut *tx)
                .await?
        {
            let amount = Credits::from_bson(
                hold.get("reserved_credits")
                    .cloned()
                    .ok_or_else(unavailable)?,
                crate::models::credits::SCALE,
            )?;
            // The guest exposure reservation is independent of returned benefits.
            if amount != Credits::ZERO || hold.get_str("state").ok() != Some("reserved") {
                return Ok(terminal::Outcome::Deferred("guest_budget_protocol"));
            }
        }
        for alias in ["wallet_resources", "grant_resources", "allowance_resources"] {
            for resource in raw.get_array(alias).map_err(|_| unavailable())? {
                let resource = resource.as_document().ok_or_else(unavailable)?;
                if present(resource, "active_settlement") {
                    let lock = resource
                        .get_document("active_settlement")
                        .map_err(|_| unavailable())?;
                    let key = if alias == "wallet_resources" {
                        "row_id"
                    } else {
                        "usage_row_id"
                    };
                    if lock.get_str(key).map_err(|_| unavailable())? == row.id {
                        return Ok(terminal::Outcome::Deferred("money_protocol_in_progress"));
                    }
                }
            }
        }
        let mut proof = raw.clone();
        if let Ok(funding) = proof.get_document_mut("funding") {
            funding.insert("settled", false);
        }
        let holds = super::super::hold_conservation::backlog::request_holds(&proof)?;
        if holds.iter().any(|(_, held)| !held.is_empty()) || row.reserved_credits != Credits::ZERO {
            return Ok(terminal::Outcome::Deferred("remaining_hold"));
        }
        rows.push(row);
    }
    let state = db
        .collection::<Document>(STATE)
        .find_one(doc! {"_id":STATE_ID})
        .session(&mut *tx)
        .await?;
    let mut summary = read_summary(state.as_ref())?.unwrap_or_else(|| Summary::new(pass));
    if summary.pass_id != pass {
        return Err(AppError::Conflict("Holdless pass summary changed".into()));
    }
    summary.add(&rows, registry.now)?;
    #[cfg(test)]
    test_point(Phase::BeforeTerminal).await;
    let mut saved = Vec::new();
    for (index, row) in rows.iter().enumerate() {
        #[cfg(not(test))]
        let _ = index;
        #[cfg(test)]
        test_point(Phase::BeforeHoldlessRow(index)).await;
        let mut fields = super::super::meter::finalization_fields(
            0,
            row.model.clone(),
            None,
            None,
            None,
            chrono::Utc::now(),
        );
        fields.insert("updated_at", "$$NOW");
        fields.insert("finalized_at", "$$NOW");
        fields.insert("stuck_request_exit",doc! {"policy":policy::UnknownConsumptionPolicy::Waive.name(),"lane":LANE,"pass_id":{"$literal":pass},"ended_at":"$$NOW"});
        // Only usage metadata changes. Frozen zero funding also makes old
        // settlement readers return zero without revisiting resource histories.
        for key in [
            "total_charge",
            "allowance_funded",
            "grant_funded",
            "wallet_funded",
            "wallet_charge_credits",
            "wallet_tax_credits",
        ] {
            fields.insert(format!("funding.{key}"), Credits::ZERO);
        }
        fields.insert("funding.settled", true);
        fields.insert("funding.allowance_funded_quantity", 0_i64);
        fields.insert("funding.lago_billable_quantity_micros", 0_i64);
        fields.insert("funding.settled_at", "$$NOW");
        // Pipeline dotted assignment cannot construct a document through null.
        let funding = doc! {"$set":{"funding":{"$ifNull":["$funding",{}]}}};
        let value = db.collection::<UsageMeterRow>(METERS).find_one_and_update(doc! {
            "_id":&row.id,"status":"forwarded","forwarded":true,"released":false,
            "quantity":Bson::Null,"pending_platform_usage":Bson::Null,"pending_resale_quantity":Bson::Null,
        },vec![funding,doc! {"$set":fields}]).return_document(mongodb::options::ReturnDocument::After).session(&mut *tx).await?.ok_or_else(unavailable)?;
        saved.push(value);
        #[cfg(test)]
        test_point(Phase::AfterHoldlessRow(index)).await;
    }
    #[cfg(test)]
    test_point(Phase::AfterRows).await;
    db.collection::<Document>(STATE).update_one(doc! {"_id":STATE_ID},doc! {
        "$set":{"holdless_cursor":cursor_id,"holdless_summary":bson::to_bson(&summary).map_err(|_|unavailable())?,"holdless_last_ended_at":registry.now},
        "$inc":{"holdless_ended_requests":1_i64,"holdless_ended_rows":rows.len() as i64},
    }).upsert(true).session(&mut *tx).await?;
    #[cfg(test)]
    test_point(Phase::BeforeCommit).await;
    Ok(terminal::Outcome::Published(saved))
}

fn zero_returns(row: &UsageMeterRow) -> AppResult<bool> {
    let Some(f) = &row.funding else {
        return Ok(true);
    };
    if f.settlement_claim_id.is_some() || f.lago_carry_id.is_some() {
        return Ok(false);
    }
    for n in [
        f.total_charge,
        f.allowance_funded,
        f.grant_funded,
        f.wallet_funded,
        f.wallet_charge_credits,
        f.wallet_tax_credits,
    ] {
        if n.is_some_and(|n| n != Credits::ZERO) {
            return Ok(false);
        }
    }
    if f.lago_billable_quantity_micros.is_some_and(|n| n != 0)
        || f.allowance_funded_quantity.is_some_and(|n| n != 0)
    {
        return Ok(false);
    }
    for c in &f.grant_consumptions {
        if c.amount != Credits::ZERO {
            return Ok(false);
        }
        if c.operation_id != format!("{}:grant-release:{}", row.id, c.grant_id)
            || !f
                .grant_reservations
                .iter()
                .any(|r| r.grant_id == c.grant_id && r.service_limit_key == c.service_limit_key)
        {
            return Err(unavailable());
        }
    }
    for c in &f.allowance_consumptions {
        if c.quantity != 0 {
            return Ok(false);
        }
        if c.operation_id != format!("{}:allowance-release:{}", row.id, c.period_id)
            || !f
                .allowance_reservations
                .iter()
                .any(|r| r.period_id == c.period_id && r.allowance_id == c.allowance_id)
        {
            return Err(unavailable());
        }
    }
    Ok(true)
}

async fn read_resources(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    rows: &mut [Document],
    now: bson::DateTime,
) -> AppResult<()> {
    for (collection, owner, alias) in [
        ("billing_wallet", "owner_id", "wallet_resources"),
        ("credit_grants", "recipient_user_id", "grant_resources"),
        (
            "usage_allowance_periods",
            "owner_user_id",
            "allowance_resources",
        ),
    ] {
        let mut ids = BTreeSet::new();
        let mut row_ids = Vec::new();
        for raw in &*rows {
            let mut wanted = BTreeSet::new();
            let row: UsageMeterRow = super::super::recovery_batch::decode(raw)?;
            if collection == "billing_wallet" {
                wanted.extend(row.wallet_id);
            } else if let Some(f) = row.funding {
                if collection == "credit_grants" {
                    wanted.extend(f.grant_reservations.into_iter().map(|r| r.grant_id));
                } else {
                    wanted.extend(f.allowance_reservations.into_iter().map(|r| r.period_id));
                }
            }
            ids.extend(wanted.iter().cloned());
            row_ids.push(wanted);
        }
        let resources: Vec<Document> = if ids.is_empty() {
            vec![]
        } else {
            let mut cursor = db
                .collection::<Document>(collection)
                .find(doc! {"_id":{"$in":ids.into_iter().collect::<Vec<_>>()}})
                .projection(doc! {"_id":1,"owner":format!("${owner}"),"active_settlement":1})
                .hint(Hint::Name("_id_".into()))
                .max_time(BUDGET)
                .session(&mut *tx)
                .await?;
            read_documents(&mut cursor, tx).await?
        };
        let resources = resources
            .into_iter()
            .map(|r| {
                let id = r.get_str("_id").map_err(|_| unavailable())?.to_owned();
                Ok((id, r))
            })
            .collect::<AppResult<BTreeMap<_, _>>>()?;
        for (raw, wanted) in rows.iter_mut().zip(row_ids) {
            let linked: Vec<_> = wanted.iter().filter_map(|id| resources.get(id)).collect();
            raw.insert(alias, bson::to_bson(&linked).map_err(|_| unavailable())?);
            raw.insert("observed_at", now);
        }
    }
    Ok(())
}

#[derive(serde::Serialize, serde::Deserialize)]
struct Bucket {
    layer: String,
    metric: String,
    requests: i64,
    rows: i64,
}
#[derive(serde::Serialize, serde::Deserialize)]
struct Summary {
    pass_id: String,
    requests: i64,
    rows: i64,
    groups: Vec<Bucket>,
    min_age_seconds: i64,
    max_age_seconds: i64,
}
impl Summary {
    fn new(pass: &str) -> Self {
        Self {
            pass_id: pass.into(),
            requests: 0,
            rows: 0,
            groups: vec![],
            min_age_seconds: i64::MAX,
            max_age_seconds: 0,
        }
    }
    fn add(&mut self, rows: &[UsageMeterRow], now: bson::DateTime) -> AppResult<()> {
        self.requests = self.requests.checked_add(1).ok_or_else(unavailable)?;
        self.rows = self
            .rows
            .checked_add(rows.len() as i64)
            .ok_or_else(unavailable)?;
        let mut groups = BTreeMap::new();
        for row in rows {
            let age = (now.to_chrono() - row.created_at).num_seconds().max(0);
            self.min_age_seconds = self.min_age_seconds.min(age);
            self.max_age_seconds = self.max_age_seconds.max(age);
            *groups
                .entry((row.layer.as_transaction_suffix(), row.metric.as_str()))
                .or_insert(0_i64) += 1;
        }
        for ((layer, metric), rows) in groups {
            if let Some(b) = self
                .groups
                .iter_mut()
                .find(|b| b.layer == layer && b.metric == metric)
            {
                b.requests += 1;
                b.rows += rows;
            } else {
                self.groups.push(Bucket {
                    layer: layer.into(),
                    metric: metric.into(),
                    requests: 1,
                    rows,
                });
            }
        }
        Ok(())
    }
}
fn read_summary(state: Option<&Document>) -> AppResult<Option<Summary>> {
    match state.and_then(|s| s.get("holdless_summary")) {
        None | Some(Bson::Null) => Ok(None),
        Some(Bson::Document(value)) => bson::from_document(value.clone())
            .map(Some)
            .map_err(|_| unavailable()),
        _ => Err(unavailable()),
    }
}
async fn flush_summary(db: &mongodb::Database) -> AppResult<()> {
    transaction_service::run_db_only(db, db, |tx, db| {
        Box::pin(async move {
            let state = db
                .collection::<Document>(STATE)
                .find_one(doc! {"_id":STATE_ID})
                .session(&mut *tx)
                .await?;
            let Some(summary) = read_summary(state.as_ref())? else {
                return Ok(());
            };
            let key = audit_service::audit_chain_hmac_key().ok_or_else(unavailable)?;
            audit_service::log_system_event_in_session(db,tx,key,EVENT,serde_json::json!({
            "policy":policy::UnknownConsumptionPolicy::Waive.name(),"pass_id":summary.pass_id,
            "requests":summary.requests,"rows":summary.rows,"groups":summary.groups,
            "min_age_seconds":summary.min_age_seconds,"max_age_seconds":summary.max_age_seconds,
        })).await?;
            #[cfg(test)]
            test_point(Phase::AfterAudit).await;
            db.collection::<Document>(STATE)
                .update_one(
                    doc! {"_id":STATE_ID},
                    doc! {"$unset":{"holdless_summary":""}},
                )
                .session(&mut *tx)
                .await?;
            #[cfg(test)]
            test_point(Phase::AfterHoldlessSummary).await;
            Ok(())
        })
    })
    .await
}

pub(in crate::services::billing) fn marked(row: &UsageMeterRow) -> bool {
    row.stuck_request_exit
        .as_ref()
        .is_some_and(|exit| exit.lane.as_deref() == Some(LANE))
}

pub(in crate::services::billing) fn validate_terminal(row: &UsageMeterRow) -> AppResult<()> {
    if !marked(row)
        || row
            .stuck_request_exit
            .as_ref()
            .is_none_or(|exit| exit.policy != policy::UnknownConsumptionPolicy::Waive.name())
        || row.quantity != Some(0)
        || !matches!(row.status, UsageStatus::Finalized | UsageStatus::Failed)
        || row.reserved_credits != Credits::ZERO
        || row.funding.as_ref().is_none_or(|f| {
            !f.settled
                || f.wallet_charge_credits != Some(Credits::ZERO)
                || f.lago_billable_quantity_micros != Some(0)
        })
        || !zero_returns(row)?
    {
        return Err(unavailable());
    }
    Ok(())
}

pub(in crate::services::billing) async fn release(
    db: &mongodb::Database,
    row: &UsageMeterRow,
) -> AppResult<bool> {
    validate_terminal(row)?;
    let now = chrono::Utc::now();
    let expires_at = now + chrono::Duration::days(30);
    let result=db.collection::<Document>(METERS).update_one(doc! {
        "_id":&row.id,"status":{"$in":["finalized","failed"]},"quantity":0_i64,"released":false,
        "stuck_request_exit.lane":LANE,"reserved_credits":Credits::ZERO,
        "funding.settled":true,"funding.wallet_charge_credits":Credits::ZERO,"funding.lago_billable_quantity_micros":0_i64,
    },doc! {"$set":{"status":"finalized","released":true,"lago_acked":true,"expires_at":bson::DateTime::from_chrono(expires_at),"updated_at":bson::DateTime::from_chrono(now)},"$unset":{"settlement_next_retry_at":"","last_error":""}}).await?;
    if result.matched_count == 0
        && db
            .collection::<Document>(METERS)
            .find_one(doc! {"_id":&row.id,"released":true})
            .await?
            .is_none()
    {
        return Err(unavailable());
    }
    Ok(result.modified_count > 0)
}
