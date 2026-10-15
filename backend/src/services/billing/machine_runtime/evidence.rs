use super::*;
use serde::Serialize;

impl BillingService {
    /// Caller must verify readiness against the current Sandbox/Pod/credential
    /// fence before invoking this in its observation transaction.
    pub async fn prove_machine_ready_in_session(
        &self,
        session: &mut ClientSession,
        run_id: &str,
        generation: i64,
        ready_at: DateTime<Utc>,
        evidence_id: &str,
    ) -> AppResult<()> {
        let mut run = load_run(&self.db, session, run_id).await?;
        check_evidence(&run, generation, ready_at, evidence_id)?;
        let ready = ns(ready_at)?;
        if let Some(existing) = run.ready_ns {
            return if existing == ready && run.ready_evidence_id.as_deref() == Some(evidence_id) {
                Ok(())
            } else {
                Err(conflict("runtime readiness already recorded"))
            };
        }
        if run.state != MachineRuntimeRunState::Accepted
            || ready < run.generation_admitted_ns
            || ready >= run.covered_until_ns
            || run.generation_floor_ns.is_some_and(|stop| ready < stop)
        {
            return Err(conflict(
                "runtime readiness lacks a live funded window or overlaps a predecessor",
            ));
        }
        run.ready_ns = Some(ready);
        run.proven_until_ns = Some(ready);
        run.ready_evidence_id = Some(evidence_id.into());
        run.proven_evidence_id = Some(evidence_id.into());
        run.state = MachineRuntimeRunState::Running;
        save_run(&self.db, session, &mut run).await
    }

    /// Evidence that the same runtime stayed running through this instant.
    /// Heartbeat loss is not a stop and cannot finalize the unproven tail.
    pub async fn prove_machine_running_in_session(
        &self,
        session: &mut ClientSession,
        run_id: &str,
        generation: i64,
        through: DateTime<Utc>,
        evidence_id: &str,
    ) -> AppResult<()> {
        let mut run = load_run(&self.db, session, run_id).await?;
        check_evidence(&run, generation, through, evidence_id)?;
        let through = ns(through)?;
        if !matches!(
            run.state,
            MachineRuntimeRunState::Running | MachineRuntimeRunState::StopPending
        ) || run.ready_ns.is_none_or(|ready| through < ready)
            || run.proven_until_ns.is_some_and(|old| through < old)
        {
            return Err(conflict("runtime running evidence is stale"));
        }
        run.proven_until_ns = Some(through);
        run.proven_evidence_id = Some(evidence_id.into());
        save_run(&self.db, session, &mut run).await
    }

    /// Fence this together with lifecycle stop/outbox. Holds remain untouched.
    /// `replacement` is internal controlled replacement, never owner resume.
    pub async fn request_machine_runtime_stop_in_session(
        &self,
        session: &mut ClientSession,
        run_id: &str,
        generation: i64,
        replacement: bool,
    ) -> AppResult<()> {
        let mut run = load_run(&self.db, session, run_id).await?;
        if run.runtime_generation != generation {
            return Err(conflict("runtime generation changed"));
        }
        if run.state == MachineRuntimeRunState::Closed {
            return Ok(());
        }
        if run.state == MachineRuntimeRunState::Stopped {
            // A human/system stop can narrow an already stopped replacement.
            // Its finite holds still settle normally; no new generation is
            // authorized by this change. Never widen a stopped contract.
            if !replacement && run.replacement_requested {
                run.replacement_requested = false;
                return save_run(&self.db, session, &mut run).await;
            }
            if replacement && !run.replacement_requested {
                return Err(conflict(
                    "a stopped contract cannot be renewed by replacement",
                ));
            }
            return Ok(());
        }
        if run.state == MachineRuntimeRunState::StopPending
            && run.replacement_requested != replacement
        {
            // An owner stop may narrow an internal replacement, never vice versa.
            if replacement {
                return Err(conflict(
                    "a stopped contract cannot be renewed by replacement",
                ));
            }
        }
        run.state = MachineRuntimeRunState::StopPending;
        run.replacement_requested = replacement;
        save_run(&self.db, session, &mut run).await
    }

    /// Physical-stop/fence observation must bind the trusted provider identities
    /// in the lifecycle record referenced by evidence_id. This does not infer a
    /// stop from a deadline, NotFound, WSS loss or a client-supplied timestamp.
    pub async fn prove_machine_stopped_in_session(
        &self,
        session: &mut ClientSession,
        run_id: &str,
        generation: i64,
        stopped_at: DateTime<Utc>,
        evidence_id: &str,
    ) -> AppResult<()> {
        let mut run = load_run(&self.db, session, run_id).await?;
        check_evidence(&run, generation, stopped_at, evidence_id)?;
        let stop = ns(stopped_at)?;
        if let Some(existing) = run.stop_ns {
            return if existing == stop && run.stop_evidence_id.as_deref() == Some(evidence_id) {
                Ok(())
            } else {
                Err(conflict("runtime stop evidence already fixed"))
            };
        }
        if run.state == MachineRuntimeRunState::Closed
            || run.ready_ns.is_some_and(|ready| stop < ready)
            || run.proven_until_ns.is_some_and(|proven| stop < proven)
        {
            return Err(conflict("runtime stop contradicts prior evidence"));
        }
        run.stop_ns = Some(stop);
        run.stop_evidence_id = Some(evidence_id.into());
        run.proven_evidence_id = Some(evidence_id.into());
        run.proven_until_ns = Some(stop);
        run.state = MachineRuntimeRunState::Stopped;
        save_run(&self.db, session, &mut run).await
    }

    /// The accepted lifecycle observation confirms stop by `observed_at`
    /// under its declared contract. Native Kubernetes observations establish
    /// control-plane stop, never a physical terminal instant or writer-isolation
    /// guarantee. Charges retain the last positively observed running instant;
    /// the observation timestamp only closes the uncharged pending tail.
    pub async fn prove_machine_stop_observed_in_session(
        &self,
        session: &mut ClientSession,
        run_id: &str,
        generation: i64,
        observed_at: DateTime<Utc>,
        evidence_id: &str,
    ) -> AppResult<()> {
        let mut run = load_run(&self.db, session, run_id).await?;
        check_evidence(&run, generation, observed_at, evidence_id)?;
        let observed = ns(observed_at)?;
        if let Some(existing) = run.stop_ns {
            return if existing == observed
                && run.stop_is_upper_bound
                && run.stop_evidence_id.as_deref() == Some(evidence_id)
            {
                Ok(())
            } else {
                Err(conflict("runtime stop evidence already fixed"))
            };
        }
        if run.state == MachineRuntimeRunState::Closed
            || run.ready_ns.is_some_and(|ready| observed < ready)
            || run.proven_until_ns.is_some_and(|proven| observed < proven)
        {
            return Err(conflict(
                "runtime stop observation contradicts prior evidence",
            ));
        }
        run.stop_ns = Some(observed);
        run.stop_is_upper_bound = true;
        run.stop_evidence_id = Some(evidence_id.into());
        // An unready generation has zero running duration. A ready generation
        // retains its prior proof identity and duration, even if polling lagged.
        if run.proven_until_ns.is_none() {
            run.proven_until_ns = Some(run.generation_admitted_ns.min(observed));
            run.proven_evidence_id = Some(evidence_id.into());
        }
        run.state = MachineRuntimeRunState::Stopped;
        save_run(&self.db, session, &mut run).await
    }

    /// Finalize one earliest window with its cumulative seconds and usage row
    /// in the caller transaction. None means evidence is still insufficient.
    /// Replays cannot advance the cumulative remainder a second time.
    pub async fn finalize_machine_interval_in_session(
        &self,
        session: &mut ClientSession,
        run_id: &str,
    ) -> AppResult<Option<String>> {
        let mut run = load_run(&self.db, session, run_id).await?;
        let intervals = self
            .db
            .collection::<MachineRuntimeInterval>(INTERVAL_COLLECTION);
        let Some(mut interval) = intervals
            .find_one(doc! {"run_id":run_id,"quantity":bson::Bson::Null})
            .sort(doc! {"covered_from_ns":1})
            .session(&mut *session)
            .await?
        else {
            return Ok(None);
        };
        if interval.runtime_generation != run.runtime_generation
            || interval.rate_version != run.rate.id
        {
            return Err(conflict(
                "prior runtime intervals require settlement before generation replacement",
            ));
        }
        let Some(proven) = run.proven_until_ns else {
            return Ok(None);
        };
        if run.stop_ns.is_none() && (run.ready_ns.is_none() || proven < interval.covered_until_ns) {
            return Ok(None);
        }
        if interval.covered_from_ns < run.accounted_until_ns {
            return Err(conflict("runtime intervals overlap"));
        }
        let evidenced = match run.ready_ns {
            Some(ready) => interval
                .covered_until_ns
                .min(proven)
                .min(run.stop_ns.unwrap_or(i64::MAX))
                .saturating_sub(interval.covered_from_ns.max(ready))
                .max(0),
            None => 0,
        };
        let (total, quantity) = cumulative_seconds(run.cumulative_covered_ns, evidenced)
            .ok_or_else(|| invalid("runtime cumulative duration overflow"))?;
        if quantity > interval.reserved_seconds {
            return Err(conflict("runtime evidence exceeds funded responsibility"));
        }
        interval.evidenced_ns = Some(evidenced);
        interval.quantity = Some(quantity);
        interval.evidence_id = run.proven_evidence_id.clone();
        let changed = intervals
            .replace_one(
                doc! {"_id":&interval.id,"quantity":bson::Bson::Null},
                &interval,
            )
            .session(&mut *session)
            .await?;
        if changed.matched_count != 1 {
            return Err(conflict("runtime interval finalization raced"));
        }
        let changed=self.db.collection::<UsageMeterRow>(USAGE).update_one(doc!{"_id":&interval.usage_row_id,"quantity":bson::Bson::Null,"released":false,
            "metric":"machine_running_seconds","funding.settlement_rate":run.rate.credits_per_second},doc!{"$set":{
            "quantity":quantity,"status":"finalized","forwarded":true,"finalized_at":bson::DateTime::from_chrono(Utc::now()),
            "updated_at":bson::DateTime::from_chrono(Utc::now())}}).session(&mut *session).await?;
        if changed.matched_count != 1 {
            return Err(conflict("runtime usage finalization raced"));
        }
        run.cumulative_covered_ns = total;
        run.cumulative_seconds = total / SECOND_NS;
        run.accounted_until_ns = interval.covered_until_ns;
        save_run(&self.db, session, &mut run).await?;
        Ok(Some(interval.usage_row_id))
    }

    /// Idempotent settlement through existing grant/wallet journal crash bridges.
    /// Generic billing reconciliation can retry the finalized rows as well.
    pub async fn settle_machine_runtime(&self, run_id: &str, limit: u32) -> AppResult<u64> {
        let intervals: Vec<MachineRuntimeInterval> = self
            .db
            .collection::<MachineRuntimeInterval>(INTERVAL_COLLECTION)
            .find(doc! {"run_id":run_id,"quantity":{"$ne":bson::Bson::Null},"settlement_complete":{"$ne":true}})
            .sort(doc! {"covered_from_ns":1})
            .limit(i64::from(limit.clamp(1, 100)))
            .await?
            .try_collect()
            .await?;
        let mut settled = 0;
        for interval in intervals {
            let row = self
                .db
                .collection::<UsageMeterRow>(USAGE)
                .find_one(doc! {"_id":&interval.usage_row_id})
                .await?
                .ok_or_else(|| conflict("runtime usage row missing"))?;
            if !row.released {
                super::super::reservation::claim_released_and_settle(&self.db, &row).await?;
            }
            let complete = self
                .db
                .collection::<UsageMeterRow>(USAGE)
                .count_documents(doc! {"_id":&row.id,"released":true,"funding.settled":true})
                .await?
                == 1;
            if complete {
                let marked = self
                    .db
                    .collection::<MachineRuntimeInterval>(INTERVAL_COLLECTION)
                    .update_one(
                        doc! {"_id":&interval.id,"settlement_complete":{"$ne":true}},
                        doc! {"$set":{"settlement_complete":true}},
                    )
                    .await?;
                settled += marked.modified_count;
            }
        }
        Ok(settled)
    }

    /// Closing is money finalization only. Lifecycle retains independent resource cleanup progress.
    /// Pending Lago emission remains a shared pending debit, never spendable.
    pub async fn close_machine_runtime_in_session(
        &self,
        session: &mut ClientSession,
        run_id: &str,
    ) -> AppResult<()> {
        let mut run = load_run(&self.db, session, run_id).await?;
        if run.state == MachineRuntimeRunState::Closed {
            return Ok(());
        }
        if run.state != MachineRuntimeRunState::Stopped
            || run.stop_ns.is_none()
            || run.replacement_requested
        {
            return Err(conflict(
                "runtime stop or replacement liability remains pending",
            ));
        }
        ensure_run_settled(&self.db, session, &run).await?;
        record_generation_residual(&mut run)?;
        run.state = MachineRuntimeRunState::Closed;
        save_run(&self.db, session, &mut run).await?;
        self.db
            .collection::<Document>(HEAD_COLLECTION)
            .update_one(
                doc! {"_id":&run.node_id,"active_run_id":&run.id},
                doc! {"$set":{"last_stop_ns":run.stop_ns},"$inc":{"revision":1_i64}},
            )
            .session(session)
            .await?;
        Ok(())
    }

    /// Controlled replacement retains the accepted rate. The prior generation
    /// must have accepted lifecycle stop evidence and settled holds before funding a new one.
    pub async fn prepare_machine_replacement(
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
        if run.state != MachineRuntimeRunState::Stopped || !run.replacement_requested {
            return Err(conflict("runtime is not awaiting controlled replacement"));
        }
        let mut p = self
            .prepare_runtime_window(owner_id, run.rate.clone(), interval_id, from, until)
            .await?;
        p.run = Some(run);
        Ok(p)
    }
    pub async fn reserve_machine_replacement_in_session(
        &self,
        session: &mut ClientSession,
        p: &PreparedRuntimeWindow,
        new_generation: i64,
    ) -> AppResult<FundedRuntimeWindow> {
        let source = p
            .run
            .as_ref()
            .ok_or_else(|| invalid("replacement requires a contract"))?;
        let mut run = load_run(&self.db, session, &source.id).await?;
        validate_run_binding(&run)?;
        if !same_rate_contract(&run.rate, &p.rate) || run.terms != source.terms {
            return Err(conflict("runtime replacement contract changed"));
        }
        if let Some(window) = existing_window(&self.db, session, &run.id, p).await? {
            if run.runtime_generation != new_generation {
                return Err(conflict("replacement retry generation changed"));
            }
            return Ok(window);
        }
        if run.state != MachineRuntimeRunState::Stopped
            || !run.replacement_requested
            || new_generation <= run.runtime_generation
            || run.stop_ns.is_none_or(|stop| p.from_ns < stop)
        {
            return Err(conflict("replacement lacks finalized lifecycle stop"));
        }
        ensure_run_settled(&self.db, session, &run).await?;
        record_generation_residual(&mut run)?;
        self.check_runtime_admission_in_session(session, p, false)
            .await?;
        run.generation_floor_ns = run.stop_ns;
        run.runtime_generation = new_generation;
        run.generation_covered_start_ns = run.cumulative_covered_ns;
        run.generation_residual_recorded = false;
        run.generation_admitted_ns = ns(Utc::now())?;
        run.ready_ns = None;
        run.proven_until_ns = None;
        run.ready_evidence_id = None;
        run.proven_evidence_id = None;
        run.stop_ns = None;
        run.stop_is_upper_bound = false;
        run.stop_evidence_id = None;
        run.replacement_requested = false;
        run.state = MachineRuntimeRunState::Accepted;
        run.accounted_until_ns = p.from_ns;
        let funded = self.insert_runtime_window(session, p, &mut run).await?;
        save_run(&self.db, session, &mut run).await?;
        Ok(funded)
    }
}

fn record_generation_residual(run: &mut MachineRuntimeRun) -> AppResult<()> {
    if run.generation_residual_recorded {
        return Ok(());
    }
    if let (Some(ready), Some(stop)) = (run.ready_ns, run.stop_ns) {
        let elapsed = stop
            .checked_sub(ready)
            .ok_or_else(|| invalid("runtime elapsed overflow"))?;
        let covered = run
            .cumulative_covered_ns
            .checked_sub(run.generation_covered_start_ns)
            .ok_or_else(|| invalid("runtime covered duration overflow"))?;
        let residual = elapsed
            .checked_sub(covered)
            .filter(|v| *v >= 0)
            .ok_or_else(|| conflict("runtime funded evidence exceeds elapsed time"))?;
        let total = if run.stop_is_upper_bound {
            &mut run.platform_residual_upper_bound_ns
        } else {
            &mut run.platform_residual_ns
        };
        *total = total
            .checked_add(residual)
            .ok_or_else(|| invalid("runtime residual overflow"))?;
    }
    run.generation_residual_recorded = true;
    Ok(())
}

async fn load_run(
    db: &Database,
    session: &mut ClientSession,
    id: &str,
) -> AppResult<MachineRuntimeRun> {
    db.collection::<MachineRuntimeRun>(RUN_COLLECTION)
        .find_one(doc! {"_id":id})
        .session(session)
        .await?
        .ok_or_else(|| invalid("runtime contract not found"))
}
async fn save_run(
    db: &Database,
    session: &mut ClientSession,
    run: &mut MachineRuntimeRun,
) -> AppResult<()> {
    let revision = run.revision;
    run.revision = revision
        .checked_add(1)
        .ok_or_else(|| invalid("runtime revision overflow"))?;
    run.updated_at = Utc::now();
    if db
        .collection::<MachineRuntimeRun>(RUN_COLLECTION)
        .replace_one(doc! {"_id":&run.id,"revision":revision}, &*run)
        .session(session)
        .await?
        .matched_count
        != 1
    {
        return Err(conflict("runtime observation raced"));
    }
    Ok(())
}
fn check_evidence(
    run: &MachineRuntimeRun,
    generation: i64,
    time: DateTime<Utc>,
    id: &str,
) -> AppResult<()> {
    if run.runtime_generation != generation
        || id.is_empty()
        || id.len() > 256
        || time > Utc::now() + Duration::seconds(1)
    {
        return Err(conflict("runtime evidence binding is invalid"));
    }
    Ok(())
}
async fn ensure_run_settled(
    db: &Database,
    session: &mut ClientSession,
    run: &MachineRuntimeRun,
) -> AppResult<()> {
    let pending = db
        .collection::<MachineRuntimeInterval>(INTERVAL_COLLECTION)
        .find_one(doc! {"run_id":&run.id,"settlement_complete":{"$ne":true}})
        .session(&mut *session)
        .await?;
    if pending.is_some() {
        return Err(conflict("runtime settlement remains pending"));
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeIntervalDetail {
    pub interval_id: String,
    pub runtime_generation: i64,
    pub rate_version: String,
    pub covered_from: DateTime<Utc>,
    pub covered_until: DateTime<Utc>,
    pub usage_row_id: String,
    pub quantity: Option<i64>,
    pub settled: bool,
    pub reserved_credits: Credits,
    pub settled_credits: Credits,
    pub allowance_funded: Credits,
    pub grant_funded: Credits,
    pub wallet_funded: Credits,
}
#[derive(Clone, Debug, Serialize)]
pub struct RuntimeFundsEstimate {
    pub wallet_available: Credits,
    pub wallet_reserved: Credits,
    pub pending_debits: Credits,
    pub pending_expiry: Credits,
    pub matching_allowance_seconds: i64,
    pub available_grants: Credits,
    pub estimated_seconds: String,
    pub shared_account_estimate_only: bool,
}
#[derive(Clone, Debug, Serialize)]
pub struct MachineRuntimeDetail {
    pub node_id: String,
    pub run_id: String,
    pub state: MachineRuntimeRunState,
    pub rate_version: String,
    pub accepted_credits_per_second: Credits,
    pub covered_until: DateTime<Utc>,
    pub cumulative_seconds: i64,
    pub settled_credits: Credits,
    pub reserved_credits: Credits,
    pub pending_credits: Credits,
    pub funds: RuntimeFundsEstimate,
    pub intervals: Vec<RuntimeIntervalDetail>,
    pub next_interval_offset: Option<u64>,
    pub attribution: &'static str,
    pub service_costs_included: bool,
}

impl BillingService {
    /// Safe owner projection. Service charges remain in existing usage rows and
    /// are never copied into these runtime totals. The caller owns live HTTP ACL.
    pub async fn machine_runtime_detail(
        &self,
        owner_id: &str,
        run_id: &str,
        offset: u64,
    ) -> AppResult<MachineRuntimeDetail> {
        let run = self
            .db
            .collection::<MachineRuntimeRun>(RUN_COLLECTION)
            .find_one(doc! {"_id":run_id,"owner_id":owner_id})
            .await?
            .ok_or_else(|| invalid("runtime contract not found"))?;
        let funds = self.runtime_funds_estimate(owner_id, &run.rate).await?;
        // One bounded server-side reduction plus at most 100 indexed row reads.
        // This is a projection over the existing usage records, not a second fee.
        let pending_run = matches!(
            run.state,
            MachineRuntimeRunState::StopPending | MachineRuntimeRunState::Stopped
        );
        let totals: Vec<Document> = self.db.collection::<Document>(INTERVAL_COLLECTION)
            .aggregate(vec![
                doc! {"$match":{"run_id":run_id}},
                doc! {"$lookup":{"from":USAGE,"localField":"usage_row_id","foreignField":"_id","as":"usage"}},
                doc! {"$unwind":{"path":"$usage","preserveNullAndEmptyArrays":true}},
                doc! {"$group":{"_id":bson::Bson::Null,"count":{"$sum":1_i64},
                    "missing":{"$sum":{"$cond":[{"$eq":[{"$type":"$usage"},"missing"]},1_i64,0_i64]}},
                    "settled":{"$sum":{"$cond":["$usage.released",{"$ifNull":["$usage.funding.total_charge",Credits::ZERO]},Credits::ZERO]}},
                    "reserved":{"$sum":{"$cond":["$usage.released",Credits::ZERO,"$reserved_total"]}},
                    "pending":{"$sum":{"$cond":[{"$and":[{"$ne":["$usage.released",true]},
                        {"$or":[pending_run,{"$ne":["$quantity",bson::Bson::Null]}]}]},"$reserved_total",Credits::ZERO]}}
                }},
            ]).max_time(std::time::Duration::from_secs(5)).await?.try_collect().await?;
        #[derive(serde::Deserialize, Default)]
        struct Totals {
            count: u64,
            missing: u64,
            #[serde(with = "crate::models::credits::whole")]
            settled: Credits,
            #[serde(with = "crate::models::credits::whole")]
            reserved: Credits,
            #[serde(with = "crate::models::credits::whole")]
            pending: Credits,
        }
        let totals: Totals = totals
            .into_iter()
            .next()
            .map(bson::from_document)
            .transpose()
            .map_err(|_| unavailable("runtime billing detail is invalid"))?
            .unwrap_or_default();
        if totals.missing != 0 {
            return Err(conflict("runtime usage missing"));
        }
        let mut intervals = Vec::new();
        let mut cursor = self
            .db
            .collection::<MachineRuntimeInterval>(INTERVAL_COLLECTION)
            .find(doc! {"run_id":run_id})
            .sort(doc! {"covered_from_ns":1})
            .skip(offset)
            .limit(100)
            .max_time(std::time::Duration::from_secs(5))
            .await?;
        while let Some(interval) = cursor.try_next().await? {
            let row = self
                .db
                .collection::<UsageMeterRow>(USAGE)
                .find_one(doc! {"_id":&interval.usage_row_id})
                .await?
                .ok_or_else(|| conflict("runtime usage missing"))?;
            let f = row.funding.as_ref();
            let cost = f.and_then(|f| f.total_charge).unwrap_or(Credits::ZERO);
            intervals.push(RuntimeIntervalDetail {
                interval_id: interval.interval_id,
                runtime_generation: interval.runtime_generation,
                rate_version: interval.rate_version,
                covered_from: DateTime::from_timestamp_nanos(interval.covered_from_ns),
                covered_until: DateTime::from_timestamp_nanos(interval.covered_until_ns),
                usage_row_id: row.id,
                quantity: interval.quantity,
                settled: row.released,
                reserved_credits: if row.released {
                    Credits::ZERO
                } else {
                    interval.reserved_total
                },
                settled_credits: if row.released { cost } else { Credits::ZERO },
                allowance_funded: f.and_then(|f| f.allowance_funded).unwrap_or(Credits::ZERO),
                grant_funded: f.and_then(|f| f.grant_funded).unwrap_or(Credits::ZERO),
                wallet_funded: f.and_then(|f| f.wallet_funded).unwrap_or(Credits::ZERO),
            });
        }
        let after = offset
            .checked_add(intervals.len() as u64)
            .ok_or_else(|| invalid("runtime page overflow"))?;
        let next = (totals.count > after).then_some(after);
        Ok(MachineRuntimeDetail {
            node_id: run.node_id,
            run_id: run.id,
            state: run.state,
            rate_version: run.rate.id,
            accepted_credits_per_second: run.rate.credits_per_second,
            covered_until: DateTime::from_timestamp_nanos(run.covered_until_ns),
            cumulative_seconds: run.cumulative_seconds,
            settled_credits: totals.settled,
            reserved_credits: totals.reserved,
            pending_credits: totals.pending,
            funds,
            intervals,
            next_interval_offset: next,
            attribution: "machine_runtime",
            service_costs_included: false,
        })
    }
}
