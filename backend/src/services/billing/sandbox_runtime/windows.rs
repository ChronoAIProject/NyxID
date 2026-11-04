use super::super::{funding, meter, reservation::LayerReservation, sandbox_runtime_math};
use super::*;
use crate::models::{
    downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
    usage_meter::BillingLayer,
};
use chrono::{Datelike, TimeZone};
use serde::Serialize;

/// The trusted producer supplies lifecycle identity, never a price or amount.
#[derive(Clone, Debug, Serialize)]
pub struct ReserveSandboxWindow {
    pub request_id: String,
    pub quote_id: String,
    pub owner_id: String,
    pub sandbox_id: String,
    pub runtime_generation: i64,
    pub operation_id: String,
    pub previous_window_id: Option<String>,
    pub seconds: i64,
    pub allocation: SandboxAllocation,
}

impl BillingService {
    pub async fn sandbox_evidence_cursor(
        &self,
        producer: &ProducerAuthority,
        sandbox: &str,
    ) -> AppResult<Option<SandboxBillingCursor>> {
        sandbox_id(sandbox)?;
        Ok(self
            .db
            .collection::<SandboxBillingCursor>(CURSOR_COLLECTION)
            .find_one(doc! {"producer_id": &producer.id, "sandbox_id": sandbox})
            .await?)
    }

    /// Read-before-admission is intentional: an ambiguous committed request
    /// remains recoverable after the provider fails or new admission is disabled.
    pub async fn sandbox_window_receipt(
        &self,
        producer: &ProducerAuthority,
        request_id: &str,
    ) -> AppResult<Option<SandboxBillingWindow>> {
        uuid(request_id)?;
        Ok(self
            .db
            .collection::<SandboxBillingWindow>(WINDOW_COLLECTION)
            .find_one(doc! {"producer_id":&producer.id,"request_id":request_id})
            .await?)
    }

    pub async fn reserve_sandbox_window(
        &self,
        producer: &ProducerAuthority,
        input: ReserveSandboxWindow,
    ) -> AppResult<SandboxBillingWindow> {
        for id in [
            &input.request_id,
            &input.quote_id,
            &input.owner_id,
            &input.operation_id,
        ] {
            uuid(id)?;
        }
        sandbox_id(&input.sandbox_id)?;
        if let Some(id) = &input.previous_window_id {
            uuid(id)?;
        }
        let quantities =
            sandbox_runtime_math::finite_quantities(units(&input.allocation), input.seconds)
                .map_err(invalid)?;
        if input.runtime_generation <= 0 || quantities.iter().all(|q| *q == 0) {
            return Err(invalid(
                "Sandbox funding requires a generation and nonzero allocation",
            ));
        }
        let request_digest = digest(&input)?;
        if let Some(receipt) = self
            .sandbox_window_receipt(producer, &input.request_id)
            .await?
        {
            return replay(receipt, &request_digest);
        }
        let service_id = self.require_sandbox_admission()?;
        super::super::exact_migration::require_ready(&self.db).await?;
        let quote = self.db.collection::<SandboxBillingQuote>(QUOTE_COLLECTION)
            .find_one(doc! {"_id":&input.quote_id,"producer_id":&producer.id,"owner_id":&input.owner_id,"sandbox_id":&input.sandbox_id}).await?
            .ok_or_else(|| invalid("Sandbox quote does not match this request"))?;
        if quote.service_id != service_id
            || units(&input.allocation)
                .iter()
                .zip(units(&quote.maximum_allocation))
                .any(|(used, max)| *used > max)
        {
            return Err(conflict(
                "Sandbox allocation or catalog differs from its quote",
            ));
        }
        let service = self
            .db
            .collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"_id":service_id,"is_active":true})
            .await?
            .ok_or_else(|| unavailable("CMA service is unavailable"))?;
        if service.slug != quote.service_slug {
            return Err(conflict(
                "CMA service prices changed; a new quote is required",
            ));
        }
        let wallet = self.sandbox_wallet(&quote.owner_id).await?;
        if wallet.id != quote.wallet_id
            || wallet.lago_customer_id != quote.lago_customer_id
            || wallet.lago_subscription_id.as_deref() != Some(quote.lago_subscription_id.as_str())
        {
            return Err(conflict("Sandbox billing account changed"));
        }
        self.verify_sandbox_admission_provider(&quote).await?;
        if let Some(lago) = self.lago.as_deref() {
            let _ = super::super::taxes::ensure_fresh(
                &self.db,
                lago,
                &wallet,
                self.config.billing_rate_cache_ttl_secs,
            )
            .await;
        }
        let mut candidates: Vec<(usize, funding::RuntimeFundingCandidates)> = Vec::with_capacity(3);
        for (index, quantity) in quantities.iter().enumerate() {
            if *quantity > 0 {
                candidates.push((
                    index,
                    funding::prepare_sandbox_window_funding(
                        &self.db,
                        &quote.owner_id,
                        Some(&quote.service_id),
                        &quote.service_slug,
                        METRICS[index],
                        units(&input.allocation)[index],
                        input.seconds,
                        Utc::now(),
                    )
                    .await?,
                ));
            }
        }
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
            fence_owner(&self.db, &mut session, &quote.owner_id).await?;
            // All admissions serialize with wallet suspension/rebinding and
            // ordinary catalog edits, including edits racing provider readback.
            let changed = self.db.collection::<Document>(WALLETS).update_one(doc! {
                "_id":&quote.wallet_id,"owner_id":&quote.owner_id,"lago_customer_id":&quote.lago_customer_id,
                "lago_subscription_id":&quote.lago_subscription_id,"collection_state":"good",
            }, doc! {"$inc":{"sandbox_billing_revision":1_i64}}).session(&mut session).await?;
            if changed.matched_count != 1 { return Err(conflict("Sandbox wallet admission changed")); }
            let changed = self.db.collection::<Document>(SERVICES).update_one(doc! {
                "_id":&service.id,"is_active":true,"updated_at":bson::DateTime::from_chrono(service.updated_at),
                "billing":bson::to_bson(&service.billing).map_err(|_| unavailable("Invalid catalog billing configuration"))?,
            },doc! {"$inc":{"sandbox_billing_revision":1_i64}}).session(&mut session).await?;
            if changed.matched_count != 1 { return Err(conflict("Sandbox service changed during admission")); }
            let mut accepted = self.db.collection::<SandboxBillingQuote>(QUOTE_COLLECTION)
                .find_one(doc! {"_id":&quote.id}).session(&mut session).await?
                .ok_or_else(|| conflict("Sandbox quote disappeared"))?;
            let now = Utc::now();
            if let Some(operation) = &accepted.accepted_operation_id {
                if operation != &input.operation_id { return Err(conflict("Sandbox accepted operation differs")); }
            } else {
                if accepted.accept_before <= now { return Err(conflict("Sandbox quote expired")); }
                accepted.accepted_operation_id = Some(input.operation_id.clone());
                accepted.accepted_at = Some(now);
            }
            let cursor = self.db.collection::<SandboxBillingCursor>(CURSOR_COLLECTION)
                .find_one(doc! {"producer_id":&producer.id,"sandbox_id":&quote.sandbox_id}).session(&mut session).await?;
            let from = next_coverage(cursor.as_ref(), &input, now.timestamp())?;
            if cursor.as_ref().is_none_or(|previous| previous.runtime_generation != input.runtime_generation)
                && quote.accepted_at.is_some() {
                return Err(conflict("A new Sandbox generation requires a previously unaccepted quote"));
            }
            if let Some(previous) = &cursor
                && previous.runtime_generation != input.runtime_generation
                && self.db.collection::<Document>(WINDOW_COLLECTION).count_documents(doc! {
                    "producer_id":&producer.id,"sandbox_id":&quote.sandbox_id,"finalized_at":bson::Bson::Null,
                }).session(&mut session).await? != 0 {
                return Err(conflict("Prior Sandbox generation requires complete usage evidence"));
            }
            let until = super::super::sandbox_rates::coverage_limit(&self.db, &mut session,
                &quote.rate_card_id, from, window_until(from, input.seconds)?).await?;
            let quantities = sandbox_runtime_math::finite_quantities(units(&input.allocation), until - from).map_err(invalid)?;
            let mut window = SandboxBillingWindow {
                id:Uuid::new_v4().to_string(), producer_id:producer.id.clone(), request_id:input.request_id.clone(),
                request_digest:request_digest.clone(), quote_id:quote.id.clone(), owner_id:quote.owner_id.clone(), sandbox_id:quote.sandbox_id.clone(),
                runtime_generation:input.runtime_generation, covered_from:from, covered_until:until, allocation:input.allocation.clone(),
                components:vec![], evidence_digest:None, first_usage_revision:None, last_usage_revision:None, created_at:now, finalized_at:None,
            };
            let context = prices::route_context(&quote, window.id.clone());
            for (index, candidate) in &candidates {
                let price = &quote.prices[*index];
                let rate = i64::try_from(price.credits_per_unit.pico()).map_err(|_| invalid("Sandbox rate overflow"))?;
                let reservation = funding::reserve_runtime_window_in_session(&self.db, &mut session, &quote.owner_id, &quote.wallet_id,
                    candidate, LayerReservation {
                        layer:BillingLayer::Platform, metric:price.metric, lago_metric_code:price.lago_metric_code.clone(), estimated_quantity:quantities[*index],
                        credits_per_unit_micros:rate / 1_000_000, credits_per_unit_pico:Some(rate), reserved_credits:Credits::ZERO,
                        allowance_reservations:vec![], grant_reservations:vec![], tax_cache_ttl_secs:self.config.billing_rate_cache_ttl_secs, applied_tax:None,
                    }, now).await?;
                let mut row = meter::reserved_row(&context, BillingLayer::Platform, price.metric, price.lago_metric_code.clone(), Some(&reservation), None);
                row.funding.as_mut().ok_or_else(|| unavailable("Sandbox funding result is missing"))?.settlement_rate = Some(price.credits_per_unit);
                window.components.push(SandboxWindowComponent { usage_row_id:row.id.clone(), metric:price.metric, reserved_quantity:quantities[*index], finalized_quantity:None });
                self.db.collection::<UsageMeterRow>(USAGE).insert_one(&row).session(&mut session).await?;
            }
            self.db.collection::<SandboxBillingWindow>(WINDOW_COLLECTION).insert_one(&window).session(&mut session).await?;
            self.db.collection::<SandboxBillingQuote>(QUOTE_COLLECTION).replace_one(doc! {"_id":&accepted.id}, &accepted).session(&mut session).await?;
            let next = SandboxBillingCursor { id:cursor.as_ref().map(|c| c.id.clone()).unwrap_or_else(|| Uuid::new_v4().to_string()),
                producer_id:producer.id.clone(), sandbox_id:quote.sandbox_id.clone(), owner_id:quote.owner_id.clone(), quote_id:quote.id.clone(),
                runtime_generation:input.runtime_generation, covered_until:until, last_window_id:window.id.clone(),
                last_evidence_revision:cursor.as_ref().and_then(|c| c.last_evidence_revision),
                evidence_covered_until:cursor.as_ref().and_then(|c| c.evidence_covered_until),
                last_evidence_window_id:cursor.as_ref().and_then(|c| c.last_evidence_window_id.clone()), updated_at:now };
            if let Some(previous) = cursor {
                let changed = self.db.collection::<SandboxBillingCursor>(CURSOR_COLLECTION)
                    .replace_one(doc! {"_id":&previous.id,"last_window_id":&previous.last_window_id}, &next).session(&mut session).await?;
                if changed.matched_count != 1 { return Err(conflict("Sandbox window admission raced")); }
            } else {
                self.db.collection::<SandboxBillingCursor>(CURSOR_COLLECTION).insert_one(&next).session(&mut session).await?;
            }
            Ok(window)
        }.await;
        match result {
            Ok(window) => {
                session.commit_transaction().await?;
                Ok(window)
            }
            Err(error) => {
                let _ = session.abort_transaction().await;
                // Another replica may have committed this exact operation.
                if let Some(receipt) = self
                    .sandbox_window_receipt(producer, &input.request_id)
                    .await?
                {
                    return replay(receipt, &request_digest);
                }
                Err(error)
            }
        }
    }
}

fn replay(receipt: SandboxBillingWindow, digest: &str) -> AppResult<SandboxBillingWindow> {
    if receipt.request_digest != digest {
        return Err(conflict(
            "Sandbox request ID was already used with different input",
        ));
    }
    Ok(receipt)
}

fn next_coverage(
    cursor: Option<&SandboxBillingCursor>,
    input: &ReserveSandboxWindow,
    now: i64,
) -> AppResult<i64> {
    let prospective = now
        .checked_add(1)
        .ok_or_else(|| invalid("Sandbox timestamp overflow"))?;
    let Some(previous) = cursor else {
        if input.previous_window_id.is_some() {
            return Err(conflict("Sandbox has no prior funded window"));
        }
        return Ok(prospective);
    };
    if previous.owner_id != input.owner_id
        || input.previous_window_id.as_deref() != Some(previous.last_window_id.as_str())
    {
        return Err(conflict("Sandbox owner or previous window differs"));
    }
    if previous.runtime_generation == input.runtime_generation {
        if previous.quote_id != input.quote_id
            || previous.covered_until <= now
            || previous.covered_until > now.saturating_add(30)
        {
            return Err(conflict(
                "Sandbox renewal requires its current quote and the final 30 seconds of live coverage",
            ));
        }
        return Ok(previous.covered_until);
    }
    if input.runtime_generation
        != previous
            .runtime_generation
            .checked_add(1)
            .ok_or_else(|| invalid("Sandbox generation overflow"))?
        || input.quote_id == previous.quote_id
        || previous.covered_until > now
    {
        return Err(conflict(
            "A new Sandbox generation requires a fresh quote after prior coverage ends",
        ));
    }
    Ok(prospective)
}

// A provider event must belong to one UTC invoice month. Near the boundary
// return shorter coverage; its receipt is the only runtime deadline authority.
fn window_until(from: i64, seconds: i64) -> AppResult<i64> {
    let date = chrono::DateTime::from_timestamp(from, 0)
        .ok_or_else(|| invalid("Invalid Sandbox timestamp"))?;
    let (year, month) = if date.month() == 12 {
        (date.year() + 1, 1)
    } else {
        (date.year(), date.month() + 1)
    };
    let boundary = Utc
        .with_ymd_and_hms(year, month, 1, 0, 0, 0)
        .single()
        .ok_or_else(|| invalid("Invalid Sandbox month boundary"))?;
    Ok(from
        .checked_add(seconds)
        .ok_or_else(|| invalid("Sandbox coverage overflow"))?
        .min(boundary.timestamp()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> ReserveSandboxWindow {
        ReserveSandboxWindow {
            request_id: "request".into(),
            quote_id: "quote".into(),
            owner_id: "owner".into(),
            sandbox_id: "sandbox".into(),
            runtime_generation: 1,
            operation_id: "operation".into(),
            previous_window_id: Some("prior".into()),
            seconds: 300,
            allocation: SandboxAllocation {
                cpu_millis: 1000,
                memory_mib: 1024,
                disk_gib: 10,
            },
        }
    }
    fn cursor() -> SandboxBillingCursor {
        SandboxBillingCursor {
            id: "cursor".into(),
            producer_id: "producer".into(),
            sandbox_id: "sandbox".into(),
            owner_id: "owner".into(),
            quote_id: "quote".into(),
            runtime_generation: 1,
            covered_until: 120,
            last_window_id: "prior".into(),
            last_evidence_revision: None,
            evidence_covered_until: None,
            last_evidence_window_id: None,
            updated_at: Utc::now(),
        }
    }
    #[test]
    fn renewal_preserves_contiguity_and_cannot_prefetch_or_reopen_expired_coverage() {
        let previous = cursor();
        let input = input();
        assert_eq!(next_coverage(Some(&previous), &input, 100).unwrap(), 120);
        assert!(next_coverage(Some(&previous), &input, 89).is_err());
        assert!(next_coverage(Some(&previous), &input, 120).is_err());
        let mut wrong = input.clone();
        wrong.previous_window_id = Some("older".into());
        assert!(next_coverage(Some(&previous), &wrong, 100).is_err());
        wrong = input;
        wrong.owner_id = "another-owner".into();
        assert!(next_coverage(Some(&previous), &wrong, 100).is_err());
    }
    #[test]
    fn replacement_requires_next_generation_fresh_quote_and_nonoverlapping_coverage() {
        let previous = cursor();
        let mut next = input();
        next.runtime_generation = 2;
        next.quote_id = "new-quote".into();
        assert!(next_coverage(Some(&previous), &next, 119).is_err());
        assert_eq!(next_coverage(Some(&previous), &next, 120).unwrap(), 121);
        next.quote_id = "quote".into();
        assert!(next_coverage(Some(&previous), &next, 120).is_err());
        next.quote_id = "new-quote".into();
        next.runtime_generation = 3;
        assert!(next_coverage(Some(&previous), &next, 120).is_err());
    }
    #[test]
    fn provider_window_stays_within_one_utc_month_including_year_rollover() {
        for (year, month, day) in [(2026, 11, 30), (2026, 12, 31), (2028, 2, 29)] {
            let from = Utc
                .with_ymd_and_hms(year, month, day, 23, 59, 50)
                .single()
                .unwrap()
                .timestamp();
            assert_eq!(window_until(from, 300).unwrap(), from + 10);
        }
        let from = Utc
            .with_ymd_and_hms(2026, 11, 3, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp();
        assert_eq!(window_until(from, 300).unwrap(), from + 300);
    }
}
