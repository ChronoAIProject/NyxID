//! Paid Machine runtime accounting. No lifecycle/provider effects occur here.
//!
//! Callers must abort their transaction on ANY error. Commit acceptance, funding
//! and lifecycle outbox together, then extend the physical deadline. Evidence
//! methods accept only caller-verified runtime observations, never HTTP input.
use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
use uuid::Uuid;

use super::{
    BillingService,
    funding::{self, MachineFundingCandidates},
    machine_runtime_math::{SECOND_NS, cumulative_seconds, quote_is_live},
    reservation::LayerReservation,
};
use crate::errors::{AppError, AppResult};
use crate::models::{
    billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
    credits::Credits,
    machine_runtime_billing::*,
    service_billing::{BillingMetric, PricingSyncStatus},
    usage_meter::{
        BillingLayer, COLLECTION_NAME as USAGE, CredentialClass, UsageFunding, UsageMeterRow,
        UsageStatus,
    },
};

pub const SKU: &str = "remote-machine-runtime";
pub const INITIAL_CREDITS_PER_SECOND: &str = "0.000040";
/// Defensive API ceiling, not the selected deployment window/stop margin.
pub const MAX_WINDOW_SECONDS: i64 = 300;
const ADMISSION_PROOF_SECONDS: i64 = 30;

mod evidence;
mod preflight;
mod prices;
pub use evidence::{MachineRuntimeDetail, RuntimeFundsEstimate};
pub use preflight::RuntimeQuotePreflight;
pub(super) use prices::verify_usage_provider_rate;

fn unavailable(reason: &str) -> AppError {
    AppError::BillingProviderUnavailable(reason.into())
}
fn conflict(reason: &str) -> AppError {
    AppError::Conflict(reason.into())
}
fn invalid(reason: &str) -> AppError {
    AppError::ValidationError(reason.into())
}
fn ns(time: DateTime<Utc>) -> AppResult<i64> {
    time.timestamp_nanos_opt()
        .ok_or_else(|| invalid("runtime timestamp is outside supported range"))
}
fn uuid(value: &str) -> AppResult<()> {
    let id = Uuid::parse_str(value).map_err(|_| invalid("runtime identity must be UUID v4"))?;
    if id.get_version_num() != 4 {
        return Err(invalid("runtime identity must be UUID v4"));
    }
    Ok(())
}

/// Server-side verified provider snapshot; private fields prevent forging None
/// or metering-only admission. Short-lived; never persisted as a bearer token.
pub struct PreparedRuntimeWindow {
    owner_id: String,
    wallet: BillingWallet,
    rate: MachineRuntimeRate,
    candidates: MachineFundingCandidates,
    checked_at: DateTime<Utc>,
    quote: Option<MachineRuntimeQuote>,
    run: Option<MachineRuntimeRun>,
    interval_id: String,
    from_ns: i64,
    until_ns: i64,
    seconds: i64,
}

impl PreparedRuntimeWindow {
    pub(crate) fn terms(&self) -> Option<&MachineRuntimeTerms> {
        self.quote
            .as_ref()
            .map(|quote| &quote.terms)
            .or_else(|| self.run.as_ref().map(|run| &run.terms))
    }
}

/// All fields must come from the owner-approved lifecycle intent. The quote
/// binds the displayed account_balance/no-cap/retention contract.
pub struct AcceptRuntimeQuote<'a> {
    pub owner_id: &'a str,
    pub node_id: &'a str,
    pub operation_id: &'a str,
    pub runtime_generation: i64,
    pub terms: &'a MachineRuntimeTerms,
}

#[derive(Clone, Debug)]
pub struct FundedRuntimeWindow {
    pub run_id: String,
    pub interval_id: String,
    pub usage_row_id: String,
    pub covered_until: DateTime<Utc>,
    pub rate_version: String,
}

impl BillingService {
    /// Read-only recovery of an accepted lifecycle intent, including after quote
    /// expiry or provider outage. Returning a historical deadline grants no new
    /// runtime: callers must not extend it or treat a stopped run as resumed.
    #[allow(clippy::too_many_arguments)]
    pub async fn replay_machine_start(
        &self,
        input: AcceptRuntimeQuote<'_>,
        quote_id: &str,
        interval_id: &str,
        from: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> AppResult<Option<FundedRuntimeWindow>> {
        let quote = self
            .db
            .collection::<MachineRuntimeQuote>(QUOTE_COLLECTION)
            .find_one(doc! {"_id":quote_id,"owner_id":input.owner_id})
            .await?
            .ok_or_else(|| invalid("runtime quote not found"))?;
        let Some(run_id) = quote.accepted_run_id.as_ref() else {
            return Ok(None);
        };
        if quote.accepted_operation_id.as_deref() != Some(input.operation_id)
            || &quote.terms != input.terms
        {
            return Err(conflict("runtime replay intent differs"));
        }
        let run = self
            .db
            .collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":run_id,"owner_id":input.owner_id,"node_id":input.node_id})
            .await?
            .ok_or_else(|| conflict("runtime replay binding differs"))?;
        validate_quote_rate(&quote, &run.rate)?;
        if run.terms != quote.terms || run.operation_id != input.operation_id {
            return Err(conflict("runtime replay contract differs"));
        }
        let interval = self
            .db
            .collection::<MachineRuntimeInterval>(INTERVAL_COLLECTION)
            .find_one(doc! {"run_id":run_id,"interval_id":interval_id})
            .await?
            .ok_or_else(|| conflict("runtime replay window differs"))?;
        if interval.runtime_generation != input.runtime_generation
            || interval.covered_from_ns != ns(from)?
            || interval.covered_until_ns != ns(until)?
            || interval.rate_version != quote.rate_version
        {
            return Err(conflict("runtime replay coverage differs"));
        }
        funded_window(&run, &interval).map(Some)
    }

    /// No provisioning side effects: paid runtime prerequisites must exist.
    async fn runtime_prerequisites(
        &self,
        owner_id: &str,
        rate: &MachineRuntimeRate,
    ) -> AppResult<BillingWallet> {
        if !self.config.billing_enabled || self.config.billing_fail_closed {
            return Err(unavailable("paid Machine billing is unavailable"));
        }
        super::exact_migration::require_ready(&self.db).await?;
        ensure_person(&self.db, owner_id, None).await?;
        let wallet = self
            .db
            .collection::<BillingWallet>(WALLETS)
            .find_one(doc! {"owner_id": owner_id})
            .await?
            .ok_or_else(|| unavailable("Machine billing requires an existing wallet"))?;
        validate_wallet(&wallet)?;
        let lago = self
            .lago
            .as_deref()
            .ok_or_else(|| unavailable("billing provider is not configured"))?;
        let subscription = lago
            .runtime_subscription(wallet.lago_subscription_id.as_deref().unwrap())
            .await
            .map_err(|_| unavailable("runtime subscription could not be verified"))?;
        if subscription.status != "active"
            || subscription.external_id != wallet.lago_subscription_id.as_deref().unwrap()
            || subscription.external_customer_id != wallet.lago_customer_id
            || subscription.plan_code != rate.plan_code
        {
            return Err(unavailable(
                "runtime subscription is not active on the accepted billing plan",
            ));
        }
        let entitlements = lago
            .entitlements(wallet.lago_subscription_id.as_deref().unwrap())
            .await
            .map_err(|_| unavailable("billing subscription could not be verified"))?;
        if !entitlements
            .iter()
            .any(|e| e.code == "*" || e.code == SKU || e.code == rate.lago_metric_code)
        {
            return Err(AppError::PlanEntitlementRequired(
                "Machine runtime is not included in this subscription".into(),
            ));
        }
        self.verify_runtime_provider_rate(rate).await?;
        Ok(wallet)
    }

    pub async fn quote_machine_runtime(
        &self,
        owner_id: &str,
        terms: MachineRuntimeTerms,
        rate_version: &str,
    ) -> AppResult<MachineRuntimeQuote> {
        self.quote_machine_runtime_at(owner_id, terms, rate_version, Utc::now())
            .await
    }
    async fn quote_machine_runtime_at(
        &self,
        owner_id: &str,
        terms: MachineRuntimeTerms,
        rate_version: &str,
        now: DateTime<Utc>,
    ) -> AppResult<MachineRuntimeQuote> {
        validate_terms(&terms)?;
        let rate = self.runtime_rate(rate_version).await?;
        if rate.profile_version != terms.profile_version {
            return Err(invalid("runtime rate does not match requested profile"));
        }
        if rate.retired {
            return Err(unavailable("runtime rate is retired for new quotes"));
        }
        self.runtime_prerequisites(owner_id, &rate).await?;
        let quote = MachineRuntimeQuote {
            id: Uuid::new_v4().to_string(),
            owner_id: owner_id.into(),
            terms,
            rate_version: rate.id,
            credits_per_second: rate.credits_per_second,
            funding_mode: "account_balance".into(),
            no_instance_cap: true,
            cumulative_whole_seconds: true,
            paused_storage_free: true,
            created_at: now,
            accept_before: now + Duration::minutes(15),
            accepted_operation_id: None,
            accepted_run_id: None,
            accepted_at: None,
        };
        self.db
            .collection::<MachineRuntimeQuote>(QUOTE_COLLECTION)
            .insert_one(&quote)
            .await?;
        Ok(quote)
    }

    /// Perform provider I/O before the caller opens its lifecycle transaction.
    pub async fn prepare_machine_start(
        &self,
        owner_id: &str,
        quote_id: &str,
        interval_id: &str,
        from: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> AppResult<PreparedRuntimeWindow> {
        let quote = self
            .db
            .collection::<MachineRuntimeQuote>(QUOTE_COLLECTION)
            .find_one(doc! {"_id":quote_id,"owner_id":owner_id})
            .await?
            .ok_or_else(|| invalid("runtime quote not found"))?;
        let rate = self.runtime_rate(&quote.rate_version).await?;
        validate_quote_rate(&quote, &rate)?;
        let mut prepared = self
            .prepare_runtime_window(owner_id, rate, interval_id, from, until)
            .await?;
        prepared.quote = Some(quote);
        Ok(prepared)
    }

    /// Only an already accepted, still-running contract can renew. A top-up is
    /// not a call to this method and cannot change stopped/pending state.
    pub async fn prepare_machine_next_window(
        &self,
        owner_id: &str,
        run_id: &str,
        interval_id: &str,
        from: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> AppResult<PreparedRuntimeWindow> {
        let run = self
            .db
            .collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":run_id,"owner_id":owner_id})
            .await?
            .ok_or_else(|| invalid("runtime contract not found"))?;
        validate_run_binding(&run)?;
        if run.state != MachineRuntimeRunState::Running {
            return Err(conflict(
                "runtime is not running; owner acceptance is required to resume",
            ));
        }
        let mut prepared = self
            .prepare_runtime_window(owner_id, run.rate.clone(), interval_id, from, until)
            .await?;
        prepared.run = Some(run);
        Ok(prepared)
    }

    async fn prepare_runtime_window(
        &self,
        owner_id: &str,
        rate: MachineRuntimeRate,
        interval_id: &str,
        from: DateTime<Utc>,
        until: DateTime<Utc>,
    ) -> AppResult<PreparedRuntimeWindow> {
        uuid(interval_id)?;
        let now = Utc::now();
        let from_ns = ns(from)?;
        let until_ns = ns(until)?;
        let duration = until_ns
            .checked_sub(from_ns)
            .ok_or_else(|| invalid("invalid runtime window"))?;
        if duration <= 0
            || duration > MAX_WINDOW_SECONDS * SECOND_NS
            || until <= now
            || from > now + Duration::seconds(MAX_WINDOW_SECONDS)
        {
            return Err(invalid(
                "runtime reservation must cover one finite next window",
            ));
        }
        let seconds = duration
            .checked_add(SECOND_NS - 1)
            .ok_or_else(|| invalid("window overflow"))?
            / SECOND_NS;
        let wallet = self.runtime_prerequisites(owner_id, &rate).await?;
        let candidates =
            funding::prepare_machine_window_funding(&self.db, owner_id, None, SKU, now).await?;
        if !candidates
            .benefits_cover(&self.db, seconds, rate.credits_per_second)
            .await?
        {
            let lago = self
                .lago
                .as_deref()
                .ok_or_else(|| unavailable("billing provider is not configured"))?;
            super::taxes::ensure_fresh(
                &self.db,
                lago,
                &wallet,
                self.config.billing_rate_cache_ttl_secs,
            )
            .await?;
        }
        Ok(PreparedRuntimeWindow {
            owner_id: owner_id.into(),
            wallet,
            rate,
            candidates,
            checked_at: now,
            quote: None,
            run: None,
            interval_id: interval_id.into(),
            from_ns,
            until_ns,
            seconds,
        })
    }

    /// Commit this with the lifecycle operation/outbox. No deadline extension
    /// or resource effect is authorized until the surrounding transaction commits.
    pub async fn accept_machine_quote_in_session(
        &self,
        session: &mut ClientSession,
        prepared: &PreparedRuntimeWindow,
        input: AcceptRuntimeQuote<'_>,
    ) -> AppResult<FundedRuntimeWindow> {
        let now = Utc::now();
        uuid(input.node_id)?;
        uuid(input.operation_id)?;
        if input.owner_id != prepared.owner_id || input.runtime_generation <= 0 {
            return Err(invalid("runtime acceptance binding mismatch"));
        }
        let source = prepared
            .quote
            .as_ref()
            .ok_or_else(|| invalid("new runtime requires a quote"))?;
        let quotes = self.db.collection::<MachineRuntimeQuote>(QUOTE_COLLECTION);
        let quote = quotes
            .find_one(doc! {"_id":&source.id,"owner_id":input.owner_id})
            .session(&mut *session)
            .await?
            .ok_or_else(|| invalid("runtime quote not found"))?;
        validate_quote_rate(&quote, &prepared.rate)?;
        if &quote.terms != input.terms
            || quote.rate_version != prepared.rate.id
            || quote.credits_per_second != prepared.rate.credits_per_second
        {
            return Err(conflict("runtime quote terms changed"));
        }
        if let Some(run_id) = &quote.accepted_run_id {
            let run = self
                .db
                .collection::<MachineRuntimeRun>(RUN_COLLECTION)
                .find_one(doc! {"_id":run_id})
                .session(&mut *session)
                .await?
                .ok_or_else(|| conflict("accepted runtime contract missing"))?;
            validate_quote_rate(&quote, &run.rate)?;
            if quote.accepted_operation_id.as_deref() != Some(input.operation_id)
                || run.node_id != input.node_id
                || run.runtime_generation != input.runtime_generation
                || run.owner_id != input.owner_id
                || run.operation_id != input.operation_id
                || run.terms != quote.terms
            {
                return Err(conflict("runtime quote already accepted by another intent"));
            }
            return existing_window(&self.db, session, &run.id, prepared)
                .await?
                .ok_or_else(|| conflict("accepted runtime window differs from retry"));
        }
        if !quote_is_live(quote.created_at.timestamp_millis(), now.timestamp_millis())
            || now >= quote.accept_before
        {
            return Err(conflict("runtime quote acceptance window expired"));
        }
        self.check_runtime_admission_in_session(session, prepared, true)
            .await?;
        let heads = self.db.collection::<MachineRuntimeHead>(HEAD_COLLECTION);
        let previous = heads
            .find_one(doc! {"_id":input.node_id})
            .session(&mut *session)
            .await?;
        let last_stop = if let Some(head) = &previous {
            if head.owner_id != input.owner_id {
                return Err(conflict("runtime owner mismatch"));
            }
            let old = self
                .db
                .collection::<MachineRuntimeRun>(RUN_COLLECTION)
                .find_one(doc! {"_id":&head.active_run_id})
                .session(&mut *session)
                .await?
                .ok_or_else(|| conflict("prior runtime contract missing"))?;
            if old.state != MachineRuntimeRunState::Closed {
                return Err(conflict(
                    "prior runtime liability must be resolved before resume",
                ));
            }
            if input.runtime_generation <= old.runtime_generation {
                return Err(conflict("resume requires a new runtime generation"));
            }
            old.stop_ns
        } else {
            None
        };
        if last_stop.is_some_and(|stop| prepared.from_ns < stop) {
            return Err(conflict("runtime generations may not overlap"));
        }
        let mut run = MachineRuntimeRun {
            id: Uuid::new_v4().to_string(),
            node_id: input.node_id.into(),
            owner_id: input.owner_id.into(),
            quote_id: quote.id.clone(),
            operation_id: input.operation_id.into(),
            rate: prepared.rate.clone(),
            terms: quote.terms,
            state: MachineRuntimeRunState::Accepted,
            runtime_generation: input.runtime_generation,
            revision: 0,
            generation_admitted_ns: ns(now)?,
            ready_ns: None,
            proven_until_ns: None,
            proven_evidence_id: None,
            replacement_requested: false,
            stop_ns: None,
            stop_is_upper_bound: false,
            ready_evidence_id: None,
            stop_evidence_id: None,
            covered_until_ns: prepared.from_ns,
            last_window_from_ns: prepared.from_ns,
            accounted_until_ns: prepared.from_ns,
            cumulative_covered_ns: 0,
            cumulative_seconds: 0,
            generation_covered_start_ns: 0,
            platform_residual_ns: 0,
            platform_residual_upper_bound_ns: 0,
            generation_residual_recorded: false,
            generation_floor_ns: last_stop,
            created_at: now,
            updated_at: now,
        };
        let window = self
            .insert_runtime_window(session, prepared, &mut run)
            .await?;
        self.db
            .collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .insert_one(&run)
            .session(&mut *session)
            .await?;
        let head = MachineRuntimeHead {
            node_id: run.node_id.clone(),
            owner_id: run.owner_id.clone(),
            active_run_id: run.id.clone(),
            revision: previous.as_ref().map_or(0, |h| h.revision + 1),
            last_stop_ns: last_stop,
        };
        if let Some(previous) = previous {
            let result = heads
                .replace_one(
                    doc! {"_id":&head.node_id,"revision":previous.revision},
                    &head,
                )
                .session(&mut *session)
                .await?;
            if result.matched_count != 1 {
                return Err(conflict("runtime contract raced"));
            }
        } else {
            heads.insert_one(head).session(&mut *session).await?;
        }
        let result = quotes.update_one(doc! {"_id":&quote.id,"accepted_run_id":bson::Bson::Null},doc! {"$set":{
            "accepted_run_id":&run.id,"accepted_operation_id":input.operation_id,"accepted_at":bson::DateTime::from_chrono(now)}}).session(&mut *session).await?;
        if result.matched_count != 1 {
            return Err(conflict("runtime quote acceptance raced"));
        }
        Ok(window)
    }

    pub async fn reserve_machine_next_window_in_session(
        &self,
        session: &mut ClientSession,
        prepared: &PreparedRuntimeWindow,
    ) -> AppResult<FundedRuntimeWindow> {
        let source = prepared
            .run
            .as_ref()
            .ok_or_else(|| invalid("renewal requires an accepted runtime"))?;
        let runs = self.db.collection::<MachineRuntimeRun>(RUN_COLLECTION);
        let mut run = runs
            .find_one(doc! {"_id":&source.id,"owner_id":&prepared.owner_id})
            .session(&mut *session)
            .await?
            .ok_or_else(|| invalid("runtime contract not found"))?;
        validate_run_binding(&run)?;
        if !same_rate_contract(&run.rate, &prepared.rate) || run.terms != source.terms {
            return Err(conflict("runtime renewal contract changed"));
        }
        if let Some(window) = existing_window(&self.db, session, &run.id, prepared).await? {
            return Ok(window);
        }
        if run.state != MachineRuntimeRunState::Running
            || run.runtime_generation != source.runtime_generation
            || run.rate != source.rate
        {
            return Err(conflict("runtime renewal was fenced"));
        }
        if run.covered_until_ns != prepared.from_ns
            || ns(Utc::now())? >= run.covered_until_ns
            || ns(Utc::now())? < run.last_window_from_ns
            || prepared.from_ns < run.accounted_until_ns
        {
            return Err(conflict(
                "only the contiguous next runtime window may be reserved",
            ));
        }
        self.check_runtime_admission_in_session(session, prepared, false)
            .await?;
        let revision = run.revision;
        let result = self
            .insert_runtime_window(session, prepared, &mut run)
            .await?;
        run.revision += 1;
        let changed = runs
            .replace_one(doc! {"_id":&run.id,"revision":revision}, &run)
            .session(&mut *session)
            .await?;
        if changed.matched_count != 1 {
            return Err(conflict("runtime renewal raced"));
        }
        Ok(result)
    }

    async fn check_runtime_admission_in_session(
        &self,
        session: &mut ClientSession,
        prepared: &PreparedRuntimeWindow,
        _new_contract: bool,
    ) -> AppResult<()> {
        let now = Utc::now();
        if !self.config.billing_enabled
            || self.config.billing_fail_closed
            || self.lago.is_none()
            || now < prepared.checked_at
            || now >= prepared.checked_at + Duration::seconds(ADMISSION_PROOF_SECONDS)
            || ns(now)? >= prepared.until_ns
        {
            return Err(unavailable(
                "runtime admission proof expired or billing is unavailable",
            ));
        }
        ensure_person(&self.db, &prepared.owner_id, Some(session)).await?;
        let wallet = self
            .db
            .collection::<BillingWallet>(WALLETS)
            .find_one(doc! {"_id": &prepared.wallet.id, "owner_id": &prepared.owner_id})
            .session(&mut *session)
            .await?
            .ok_or_else(|| unavailable("runtime wallet missing"))?;
        validate_wallet(&wallet)?;
        if wallet.lago_subscription_id != prepared.wallet.lago_subscription_id
            || wallet.lago_wallet_id != prepared.wallet.lago_wallet_id
            || wallet.lago_customer_id != prepared.wallet.lago_customer_id
        {
            return Err(unavailable(
                "runtime provider binding changed after validation",
            ));
        }
        // Even a fully benefit-funded window depends on this live wallet and
        // provider binding. Write it in the transaction so a concurrent
        // suspension/rebinding conflicts rather than admitting from a stale
        // read-only snapshot when no wallet credits need to be held.
        self.db
            .collection::<Document>(WALLETS)
            .update_one(
                doc! {"_id":&wallet.id,"owner_id":&prepared.owner_id},
                doc! {"$inc":{"machine_runtime_admission_revision":1_i64}},
            )
            .session(&mut *session)
            .await?;
        let rate = self
            .db
            .collection::<MachineRuntimeRate>(RATE_COLLECTION)
            .find_one(doc! {"_id":&prepared.rate.id})
            .session(&mut *session)
            .await?
            .ok_or_else(|| unavailable("runtime rate missing"))?;
        if prices::validate_rate(&rate).is_err()
            || !same_rate_contract(&rate, &prepared.rate)
            || rate.sync_status != PricingSyncStatus::Synced
            || rate.verified_at.is_none()
        {
            return Err(unavailable("runtime price is not synchronized"));
        }
        // Conflict with administrative retirement/sync updates in this transaction.
        self.db
            .collection::<Document>(RATE_COLLECTION)
            .update_one(
                doc! {"_id":&rate.id},
                doc! {"$inc":{"admission_revision":1_i64}},
            )
            .session(&mut *session)
            .await?;
        Ok(())
    }

    async fn insert_runtime_window(
        &self,
        session: &mut ClientSession,
        prepared: &PreparedRuntimeWindow,
        run: &mut MachineRuntimeRun,
    ) -> AppResult<FundedRuntimeWindow> {
        let rate_pico = i64::try_from(prepared.rate.credits_per_second.pico())
            .map_err(|_| invalid("runtime rate overflow"))?;
        let reservation = funding::reserve_machine_window_in_session(
            &self.db,
            session,
            &run.owner_id,
            &prepared.wallet.id,
            &prepared.candidates,
            LayerReservation {
                layer: BillingLayer::Platform,
                metric: BillingMetric::MachineRunningSeconds,
                lago_metric_code: run.rate.lago_metric_code.clone(),
                estimated_quantity: prepared.seconds,
                credits_per_unit_micros: rate_pico / 1_000_000,
                credits_per_unit_pico: Some(rate_pico),
                reserved_credits: Credits::ZERO,
                allowance_reservations: vec![],
                grant_reservations: vec![],
                tax_cache_ttl_secs: self.config.billing_rate_cache_ttl_secs,
                applied_tax: None,
            },
            Utc::now(),
        )
        .await?;
        let layer = &reservation.layers[0];
        let interval = MachineRuntimeInterval {
            id: Uuid::new_v4().to_string(),
            run_id: run.id.clone(),
            node_id: run.node_id.clone(),
            runtime_generation: run.runtime_generation,
            rate_version: run.rate.id.clone(),
            interval_id: prepared.interval_id.clone(),
            usage_row_id: Uuid::new_v4().to_string(),
            covered_from_ns: prepared.from_ns,
            covered_until_ns: prepared.until_ns,
            reserved_seconds: prepared.seconds,
            reserved_total: run.rate.credits_per_second.checked_mul(prepared.seconds)?,
            evidence_id: None,
            evidenced_ns: None,
            quantity: None,
            settlement_complete: false,
            created_at: Utc::now(),
        };
        let now = Utc::now();
        let row = UsageMeterRow {
            id: interval.usage_row_id.clone(),
            rollup_pending: true,
            transaction_id: format!(
                "machine:{}:{}:{}:{}",
                run.node_id, run.runtime_generation, run.rate.id, prepared.interval_id
            ),
            billing_request_id: interval.id.clone(),
            layer: BillingLayer::Platform,
            flush_seq: None,
            billing_owner_id: run.owner_id.clone(),
            wallet_id: Some(reservation.wallet_id),
            actor_user_id: run.owner_id.clone(),
            api_key_id: None,
            assistant_source: None,
            service_id: None,
            user_service_id: None,
            service_slug: Some(SKU.into()),
            metric: BillingMetric::MachineRunningSeconds,
            lago_metric_code: run.rate.lago_metric_code.clone(),
            operation: None,
            credential_class: CredentialClass::NoAuth,
            model: None,
            reported_model: None,
            token_breakdown: None,
            audio_tokens: None,
            image_tokens: None,
            reserved_credits: layer.reserved_credits,
            funding: Some(UsageFunding {
                credits_per_unit_micros: layer.credits_per_unit_micros,
                credits_per_unit_pico: layer.credits_per_unit_pico,
                settlement_rate: Some(run.rate.credits_per_second),
                tax_accounting: true,
                applied_tax: layer.applied_tax.clone(),
                allowance_reservations: layer.allowance_reservations.clone(),
                grant_reservations: layer.grant_reservations.clone(),
                ..Default::default()
            }),
            quantity: None,
            pending_resale_quantity: None,
            pending_platform_usage: None,
            pool_attempt: None,
            machine_usage: None,
            status: UsageStatus::Reserved,
            forwarded: false,
            released: false,
            lago_acked: false,
            attempt: 0,
            settlement_attempts: 0,
            settlement_next_retry_at: None,
            created_at: now,
            updated_at: now,
            finalized_at: None,
            expires_at: None,
            last_error: None,
        };
        self.db
            .collection::<UsageMeterRow>(USAGE)
            .insert_one(row)
            .session(&mut *session)
            .await?;
        self.db
            .collection::<MachineRuntimeInterval>(INTERVAL_COLLECTION)
            .insert_one(&interval)
            .session(&mut *session)
            .await?;
        run.covered_until_ns = prepared.until_ns;
        run.last_window_from_ns = prepared.from_ns;
        run.updated_at = now;
        funded_window(run, &interval)
    }
}

fn validate_terms(terms: &MachineRuntimeTerms) -> AppResult<()> {
    if crate::services::remote_machine_service::controlled_profile(&terms.profile_version).is_err()
        || terms.policy_version.is_empty()
        || terms.policy_version.len() > 128
        || !terms.image_digest.starts_with("sha256:")
        || terms.image_digest.len() != 71
        || !terms.image_digest[7..]
            .bytes()
            .all(|c| c.is_ascii_hexdigit())
    {
        return Err(invalid("invalid runtime quote profile/image/policy"));
    }
    Ok(())
}

fn same_rate_contract(left: &MachineRuntimeRate, right: &MachineRuntimeRate) -> bool {
    left.id == right.id
        && left.profile_version == right.profile_version
        && left.metric == right.metric
        && left.lago_metric_code == right.lago_metric_code
        && left.plan_code == right.plan_code
        && left.credits_per_second == right.credits_per_second
}

fn validate_quote_rate(quote: &MachineRuntimeQuote, rate: &MachineRuntimeRate) -> AppResult<()> {
    validate_terms(&quote.terms)?;
    prices::validate_rate(rate)?;
    if quote.rate_version != rate.id
        || quote.terms.profile_version != rate.profile_version
        || quote.credits_per_second != rate.credits_per_second
    {
        return Err(conflict("runtime quote profile or rate mismatch"));
    }
    Ok(())
}

fn validate_run_binding(run: &MachineRuntimeRun) -> AppResult<()> {
    validate_terms(&run.terms)?;
    prices::validate_rate(&run.rate)?;
    if run.terms.profile_version != run.rate.profile_version {
        return Err(conflict("runtime contract profile and rate mismatch"));
    }
    Ok(())
}
fn validate_wallet(wallet: &BillingWallet) -> AppResult<()> {
    if wallet.is_suspended()
        || wallet.collection_state != crate::models::billing_wallet::CollectionState::Good
    {
        return Err(AppError::WalletSuspended);
    }
    if wallet.lago_customer_id.is_empty()
        || wallet.lago_wallet_id.as_deref().is_none_or(str::is_empty)
        || wallet
            .lago_subscription_id
            .as_deref()
            .is_none_or(str::is_empty)
    {
        return Err(unavailable(
            "runtime requires a provisioned wallet and subscription",
        ));
    }
    Ok(())
}
async fn ensure_person(
    db: &Database,
    owner_id: &str,
    session: Option<&mut ClientSession>,
) -> AppResult<()> {
    let filter = doc! {"_id":owner_id,"is_active":true,"$or":[{"user_type":"person"},{"user_type":{"$exists":false}}],
    "deletion_requested_at":bson::Bson::Null,"machine_restricted":{"$ne":true}};
    let users = db.collection::<Document>(crate::models::user::COLLECTION_NAME);
    if let Some(session) = session {
        // Touch live owner to conflict with concurrent disable/delete mutations.
        if users
            .update_one(filter, doc! {"$inc":{"machine_billing_revision":1_i64}})
            .session(session)
            .await?
            .matched_count
            != 1
        {
            return Err(AppError::Forbidden(
                "Machine requires an eligible personal owner".into(),
            ));
        }
    } else if users.find_one(filter).await?.is_none() {
        return Err(AppError::Forbidden(
            "Machine requires an eligible personal owner".into(),
        ));
    }
    Ok(())
}
async fn existing_window(
    db: &Database,
    session: &mut ClientSession,
    run_id: &str,
    p: &PreparedRuntimeWindow,
) -> AppResult<Option<FundedRuntimeWindow>> {
    let interval = db
        .collection::<MachineRuntimeInterval>(INTERVAL_COLLECTION)
        .find_one(doc! {"run_id":run_id,"interval_id":&p.interval_id})
        .session(&mut *session)
        .await?;
    let Some(interval) = interval else {
        return Ok(None);
    };
    if interval.covered_from_ns != p.from_ns
        || interval.covered_until_ns != p.until_ns
        || interval.rate_version != p.rate.id
    {
        return Err(conflict(
            "runtime interval identity reused with different coverage",
        ));
    }
    let run = db
        .collection::<MachineRuntimeRun>(RUN_COLLECTION)
        .find_one(doc! {"_id":run_id})
        .session(session)
        .await?
        .ok_or_else(|| conflict("runtime missing"))?;
    validate_run_binding(&run)?;
    if !same_rate_contract(&run.rate, &p.rate) || p.terms().is_none_or(|terms| terms != &run.terms)
    {
        return Err(conflict("runtime interval contract differs from retry"));
    }
    funded_window(&run, &interval).map(Some)
}
fn funded_window(
    run: &MachineRuntimeRun,
    interval: &MachineRuntimeInterval,
) -> AppResult<FundedRuntimeWindow> {
    validate_run_binding(run)?;
    if interval.rate_version != run.rate.id {
        return Err(conflict("runtime interval rate binding differs"));
    }
    Ok(FundedRuntimeWindow {
        run_id: run.id.clone(),
        interval_id: interval.interval_id.clone(),
        usage_row_id: interval.usage_row_id.clone(),
        covered_until: DateTime::from_timestamp_nanos(interval.covered_until_ns),
        rate_version: run.rate.id.clone(),
    })
}

/// Install before admitting any runtime. No TTL may erase unresolved liability.
pub async fn ensure_indexes(db: &Database) -> AppResult<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    for (collection, keys) in [
        (RATE_COLLECTION, doc! {"lago_metric_code":1}),
        (
            INTERVAL_COLLECTION,
            doc! {"node_id":1,"runtime_generation":1,"rate_version":1,"interval_id":1},
        ),
        (INTERVAL_COLLECTION, doc! {"run_id":1,"interval_id":1}),
        (INTERVAL_COLLECTION, doc! {"usage_row_id":1}),
        (RUN_COLLECTION, doc! {"quote_id":1}),
    ] {
        db.collection::<Document>(collection)
            .create_index(
                IndexModel::builder()
                    .keys(keys)
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await?;
    }
    for (collection, keys) in [
        (
            RATE_COLLECTION,
            doc! {"profile_version":1,"plan_code":1,"sync_status":1,"retired":1,"created_at":-1,"_id":-1},
        ),
        (INTERVAL_COLLECTION, doc! {"run_id":1,"covered_from_ns":1}),
        (
            INTERVAL_COLLECTION,
            doc! {"run_id":1,"settlement_complete":1,"covered_from_ns":1},
        ),
        (
            RUN_COLLECTION,
            doc! {"owner_id":1,"node_id":1,"created_at":-1},
        ),
        (RUN_COLLECTION, doc! {"state":1,"updated_at":1}),
        (QUOTE_COLLECTION, doc! {"owner_id":1,"created_at":-1}),
    ] {
        db.collection::<Document>(collection)
            .create_index(IndexModel::builder().keys(keys).build())
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
