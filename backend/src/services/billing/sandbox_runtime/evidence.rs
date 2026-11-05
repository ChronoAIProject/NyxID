use super::super::sandbox_runtime_math::{self, IntervalEvidence};
use super::*;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct SandboxIntervalEvidence {
    pub revision: i64,
    pub started_at: i64,
    pub ended_at: i64,
    pub allocation: SandboxAllocation,
    /// Original ledger arithmetic, checked before intersecting with coverage.
    pub quantity: SandboxUsageQuantity,
}

#[derive(Clone, Debug, Serialize)]
pub struct SandboxEvidence {
    pub window_id: String,
    pub owner_id: String,
    pub sandbox_id: String,
    pub runtime_generation: i64,
    pub intervals: Vec<SandboxIntervalEvidence>,
}

impl BillingService {
    /// Persist verified evidence independently of new-admission/provider flags.
    /// An outage must not strand a previously admitted, recoverable liability.
    pub async fn finalize_sandbox_window(
        &self,
        producer: &ProducerAuthority,
        input: SandboxEvidence,
    ) -> AppResult<SandboxBillingWindow> {
        uuid(&input.window_id)?;
        uuid(&input.owner_id)?;
        sandbox_id(&input.sandbox_id)?;
        if input.intervals.is_empty()
            || input.intervals.len() > sandbox_runtime_math::MAX_EVIDENCE_INTERVALS
        {
            return Err(invalid(
                "Sandbox evidence must contain 1 to 1024 retained intervals",
            ));
        }
        let evidence_digest = digest(&input)?;
        let mut session = self.db.client().start_session().await?;
        session
            .start_transaction()
            .write_concern(
                mongodb::options::WriteConcern::builder()
                    .w(mongodb::options::Acknowledgment::Majority)
                    .build(),
            )
            .await?;
        let result: AppResult<SandboxBillingWindow> = async {
            fence_producer(&self.db, &mut session, producer).await?;
            let mut window = self.db.collection::<SandboxBillingWindow>(WINDOW_COLLECTION)
                .find_one(doc! {"_id":&input.window_id,"producer_id":&producer.id,"owner_id":&input.owner_id,
                    "sandbox_id":&input.sandbox_id,"runtime_generation":input.runtime_generation})
                .session(&mut session).await?.ok_or_else(|| invalid("Sandbox evidence identity does not match a funded window"))?;
            if let Some(existing) = &window.evidence_digest {
                if existing != &evidence_digest { return Err(conflict("Sandbox window already has different evidence")); }
                return Ok(window);
            }
            let now = Utc::now();
            if input.intervals.iter().any(|interval| interval.ended_at > now.timestamp()) {
                return Err(invalid("Sandbox evidence cannot attest future usage"));
            }
            let intervals = input.intervals.iter().map(|interval| IntervalEvidence {
                revision:interval.revision, started_at:interval.started_at, ended_at:interval.ended_at,
                allocation:units(&interval.allocation), quantity:quantities(&interval.quantity),
            }).collect::<Vec<_>>();
            let quantities = sandbox_runtime_math::evidenced_quantities(window.covered_from, window.covered_until,
                units(&window.allocation), &intervals).map_err(invalid)?;
            let cursor = self.db.collection::<SandboxBillingCursor>(CURSOR_COLLECTION)
                .find_one(doc! {"producer_id": &producer.id, "sandbox_id": &window.sandbox_id, "owner_id": &window.owner_id})
                .session(&mut session).await?
                .ok_or_else(|| conflict("Sandbox evidence cursor is missing"))?;
            let earliest = self.db.collection::<SandboxBillingWindow>(WINDOW_COLLECTION)
                .find_one(doc! {"producer_id": &producer.id, "sandbox_id": &window.sandbox_id, "evidence_digest": bson::Bson::Null})
                .sort(doc! {"covered_from": 1})
                .session(&mut session).await?
                .ok_or_else(|| conflict("Sandbox evidence window is missing"))?;
            if earliest.id != window.id {
                return Err(conflict("Submit evidence for earlier funded Sandbox windows first"));
            }
            validate_evidence_cursor(&cursor, &window, &input.intervals)?;
            let usage_at = chrono::DateTime::from_timestamp(window.covered_until - 1, 0)
                .ok_or_else(|| invalid("Invalid Sandbox usage timestamp"))?;
            let quote = self.db.collection::<SandboxBillingQuote>(QUOTE_COLLECTION)
                .find_one(doc! {"_id":&window.quote_id,"producer_id":&producer.id,"owner_id":&window.owner_id,"sandbox_id":&window.sandbox_id})
                .session(&mut session).await?.ok_or_else(|| unavailable("Accepted Sandbox quote is missing"))?;
            for interval in &input.intervals {
                let payload_digest = digest(interval)?;
                let filter = doc! {"producer_id":&producer.id,"sandbox_id":&window.sandbox_id,"revision":interval.revision};
                let evidence = self.db.collection::<SandboxLedgerEvidence>(EVIDENCE_COLLECTION);
                if let Some(existing) = evidence.find_one(filter).session(&mut session).await? {
                    if existing.owner_id != window.owner_id || existing.payload_digest != payload_digest {
                        return Err(conflict("Retained Sandbox ledger revision differs from previously accepted evidence"));
                    }
                } else {
                    evidence.insert_one(SandboxLedgerEvidence { id:Uuid::new_v4().to_string(), producer_id:producer.id.clone(),
                        owner_id:window.owner_id.clone(), sandbox_id:window.sandbox_id.clone(), revision:interval.revision, payload_digest,
                        started_at:interval.started_at, ended_at:interval.ended_at, allocation:interval.allocation.clone(), quantity:interval.quantity.clone(),
                    }).session(&mut session).await?;
                }
            }
            for component in &mut window.components {
                let index = METRICS.iter().position(|metric| *metric == component.metric)
                    .ok_or_else(|| unavailable("Invalid Sandbox window component"))?;
                let price = quote.prices.iter().find(|price| price.metric == component.metric)
                    .ok_or_else(|| unavailable("Accepted Sandbox price is missing"))?;
                let quantity = quantities[index];
                if quantity > component.reserved_quantity { return Err(conflict("Sandbox evidence exceeds its reserved quantity")); }
                let changed = self.db.collection::<UsageMeterRow>(USAGE).update_one(doc! {
                    "_id":&component.usage_row_id,"billing_request_id":&window.id,"billing_owner_id":&window.owner_id,"wallet_id":&quote.wallet_id,
                    "metric":component.metric.as_str(),"lago_metric_code":&price.lago_metric_code,
                    "quantity":bson::Bson::Null,"released":false,"funding.settlement_rate":price.credits_per_unit,
                }, doc! {"$set":{"quantity":quantity,"status":"finalized","forwarded":true,
                    "finalized_at":bson::DateTime::from_chrono(usage_at),"updated_at":bson::DateTime::from_chrono(now)}})
                    .session(&mut session).await?;
                if changed.matched_count != 1 { return Err(conflict("Sandbox usage finalization raced")); }
                component.finalized_quantity = Some(quantity);
            }
            // The cursor and usage finalization commit together. Both admission
            // and consumption write this row, fencing concurrent replicas.
            let changed = self.db.collection::<Document>(CURSOR_COLLECTION)
                .update_one(doc! {"_id": &cursor.id}, doc! {"$set": {
                    "last_evidence_revision": input.intervals.last().map(|interval| interval.revision),
                    "evidence_covered_until": window.covered_until,
                    "last_evidence_window_id": &window.id,
                    "updated_at": bson::DateTime::from_chrono(now),
                }}).session(&mut session).await?;
            if changed.matched_count != 1 {
                return Err(conflict("Sandbox evidence cursor changed"));
            }
            window.evidence_digest = Some(evidence_digest.clone());
            window.first_usage_revision = input.intervals.first().map(|interval| interval.revision);
            window.last_usage_revision = input.intervals.last().map(|interval| interval.revision);
            window.finalized_at = Some(now);
            let changed = self.db.collection::<SandboxBillingWindow>(WINDOW_COLLECTION)
                .replace_one(doc! {"_id":&window.id,"evidence_digest":bson::Bson::Null}, &window).session(&mut session).await?;
            if changed.matched_count != 1 { return Err(conflict("Sandbox evidence finalization raced")); }
            Ok(window)
        }.await;
        let window = match result {
            Ok(window) => {
                session.commit_transaction().await?;
                window
            }
            Err(error) => {
                let _ = session.abort_transaction().await;
                return Err(error);
            }
        };
        // A finalized row is the existing reconciler's durable recovery work.
        // Provider/funding outages never erase accepted evidence or return a
        // false assertion that the transaction was not committed.
        for component in &window.components {
            let row = self
                .db
                .collection::<UsageMeterRow>(USAGE)
                .find_one(doc! {"_id":&component.usage_row_id})
                .await?;
            if let Some(row) = row
                && !row.released
                && super::super::reservation::claim_released_and_settle(&self.db, &row)
                    .await
                    .is_err()
            {
                tracing::warn!(window_id = %window.id, "Sandbox settlement will retry through billing reconciliation");
            }
        }
        Ok(window)
    }
}

/// Bind Lago emission to the accepted quote and exact persisted meter row.
/// No ordinary wallet lookup can silently redirect previously funded usage.
pub(in crate::services::billing) async fn verify_usage_provider_rate(
    db: &Database,
    lago: &dyn super::super::lago_client::LagoApi,
    row: &UsageMeterRow,
) -> AppResult<Option<String>> {
    if !METRICS.contains(&row.metric) {
        return Ok(None);
    }
    let window = db.collection::<SandboxBillingWindow>(WINDOW_COLLECTION)
        .find_one(doc! {"_id":&row.billing_request_id,"owner_id":&row.billing_owner_id,"finalized_at":{"$ne":bson::Bson::Null}})
        .await?.ok_or_else(|| unavailable("Sandbox usage lacks accepted evidence"))?;
    let component = window
        .components
        .iter()
        .find(|component| component.usage_row_id == row.id && component.metric == row.metric)
        .ok_or_else(|| unavailable("Sandbox usage is not bound to its funded window"))?;
    let quote = db.collection::<SandboxBillingQuote>(QUOTE_COLLECTION)
        .find_one(doc! {"_id":&window.quote_id,"owner_id":&window.owner_id,"sandbox_id":&window.sandbox_id,"producer_id":&window.producer_id})
        .await?.ok_or_else(|| unavailable("Sandbox accepted quote is missing"))?;
    let price = quote
        .prices
        .iter()
        .find(|price| price.metric == row.metric)
        .ok_or_else(|| unavailable("Sandbox accepted price is missing"))?;
    if row.finalized_at.map(|at| at.timestamp()) != Some(window.covered_until - 1)
        || component.finalized_quantity != row.quantity
        || row.quantity.is_none()
        || row.wallet_id.as_deref() != Some(quote.wallet_id.as_str())
        || row.service_id.as_deref() != Some(quote.service_id.as_str())
        || row.lago_metric_code != price.lago_metric_code
        || row.layer != crate::models::usage_meter::BillingLayer::Platform
        || row
            .funding
            .as_ref()
            .and_then(|funding| funding.settlement_rate)
            != Some(price.credits_per_unit)
    {
        return Err(unavailable(
            "Sandbox settled usage differs from its accepted quote",
        ));
    }
    if let Err(error) = prices::verify_provider(lago, &quote).await {
        super::super::sandbox_rates::request_resync(db, &quote.rate_card_id).await?;
        return Err(error);
    }
    Ok(Some(quote.lago_subscription_id))
}

fn validate_evidence_cursor(
    cursor: &SandboxBillingCursor,
    window: &SandboxBillingWindow,
    intervals: &[SandboxIntervalEvidence],
) -> AppResult<()> {
    let Some(last_revision) = cursor.last_evidence_revision else {
        return Ok(());
    };
    let first = intervals
        .first()
        .ok_or_else(|| invalid("Sandbox evidence is empty"))?;
    if first.revision < last_revision {
        return Err(conflict("Sandbox evidence precedes its durable cursor"));
    }
    if cursor.evidence_covered_until == Some(window.covered_from)
        && first.revision != last_revision
        && last_revision.checked_add(1) != Some(first.revision)
    {
        return Err(conflict(
            "Sandbox evidence skips retained revisions between funded windows",
        ));
    }
    // A single retained interval may span adjacent funded windows. Its digest
    // is checked below before the cursor can reuse that boundary revision.
    Ok(())
}
