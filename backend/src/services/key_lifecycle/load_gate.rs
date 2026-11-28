//! Recorded, paired fixed-load evidence. A new declaration or measurement
//! supersedes the previous result in the same revision as its immutable entry.
use super::*;
use rand::{Rng, SeedableRng};
use serde::Deserialize;
use std::sync::OnceLock;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Protocol {
    pub schema_version: u32,
    pub registry_digest: String,
    pub baseline_sha256: String,
    pub candidate_sha256: String,
    pub workload_sha256: String,
    pub rig_ref: String,
    pub slo_ref: String,
    pub offered_rps: u32,
    pub minimum_rps: u32,
    pub operations_per_arm: u32,
    pub max_p95_us: u32,
    pub max_p99_us: u32,
    pub confidence_bps: u32,
    pub margin_bps: u32,
    pub paired_rounds: u32,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Sample {
    pub posted: u32,
    pub posting_span_us: u64,
    pub completed: u32,
    pub errors: u32,
    pub elapsed_us: u64,
    pub p95_us: u32,
    pub p99_us: u32,
    pub source_commands: u64,
    pub foreground_census_commands: u32,
    pub kms_calls: u64,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Round {
    pub base: Sample,
    pub base_repeat: Sample,
    pub candidate_idle: Sample,
    pub candidate_capture: Sample,
    pub candidate_publication: Sample,
    pub authenticated_cuts: u32,
    pub writes_during_publication: u32,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Environment {
    Shared,
    Isolated,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Measurement {
    pub schema_version: u32,
    pub protocol_id: String,
    pub environment: Environment,
    pub rig_ref: String,
    pub workload_sha256: String,
    pub baseline_sha256: String,
    pub candidate_sha256: String,
    pub offered_rps: u32,
    pub started_ms: i64,
    pub finished_ms: i64,
    pub rounds: Vec<Round>,
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(super) async fn executable() -> AppResult<&'static str> {
    static DIGEST: OnceLock<String> = OnceLock::new();
    if let Some(value) = DIGEST.get() {
        return Ok(value);
    }
    let value = tokio::task::spawn_blocking(|| -> std::io::Result<String> {
        #[cfg(target_os = "linux")]
        let mut file = std::fs::File::open("/proc/self/exe")?;
        #[cfg(not(target_os = "linux"))]
        let mut file = std::fs::File::open(std::env::current_exe()?)?;
        let mut hash = Sha256::new();
        std::io::copy(&mut file, &mut hash)?;
        Ok(hex::encode(hash.finalize()))
    })
    .await
    .map_err(|_| corrupt())?
    .map_err(|_| corrupt())?;
    Ok(DIGEST.get_or_init(|| value))
}

impl Protocol {
    async fn validate(&self) -> AppResult<()> {
        contract::uuid(&self.rig_ref)?;
        contract::uuid(&self.slo_ref)?;
        if self.schema_version != 1
            || self.margin_bps != 100
            || self.confidence_bps != 9500
            || !(8..=64).contains(&self.paired_rounds)
            || self.operations_per_arm < 400
            || self.minimum_rps == 0
            || self.offered_rps < self.minimum_rps
            || self.max_p95_us == 0
            || self.max_p99_us < self.max_p95_us
            || !digest(&self.workload_sha256)
            || !digest(&self.baseline_sha256)
            || self.baseline_sha256 == self.candidate_sha256
            || !crate::services::encrypted_record_census::report::retirement::current_registry(
                &self.registry_digest,
            )
            || self.candidate_sha256 != executable().await?
        {
            return Err(invalid());
        }
        Ok(())
    }
}

impl Measurement {
    fn matches(&self, plan: &Protocol, declared: &Receipt, received_ms: i64) -> bool {
        self.schema_version == 1
            && self.protocol_id == declared.entry_id
            && self.rig_ref == plan.rig_ref
            && self.workload_sha256 == plan.workload_sha256
            && self.baseline_sha256 == plan.baseline_sha256
            && self.candidate_sha256 == plan.candidate_sha256
            && self.offered_rps == plan.offered_rps
            && self.rounds.len() == plan.paired_rounds as usize
            && declared.recorded_at_ms < self.started_ms
            && self.started_ms < self.finished_ms
            && self.finished_ms <= received_ms
            && self
                .rounds
                .iter()
                .flat_map(|r| {
                    [
                        &r.base,
                        &r.base_repeat,
                        &r.candidate_idle,
                        &r.candidate_capture,
                        &r.candidate_publication,
                    ]
                })
                .map(|s| u128::from(s.elapsed_us))
                .sum::<u128>()
                <= (i128::from(self.finished_ms) - i128::from(self.started_ms)) as u128 * 1000
    }

    fn qualifies(&self, plan: &Protocol) -> bool {
        if self.environment != Environment::Isolated {
            return false;
        }
        let mut comparisons: [Vec<[f64; 3]>; 4] = Default::default();
        for round in &self.rounds {
            if round.authenticated_cuts == 0
                || round.writes_during_publication == 0
                || round.writes_during_publication > plan.operations_per_arm
            {
                return false;
            }
            let metrics = |s: &Sample| {
                let rate = f64::from(s.completed) * 1e6 / s.elapsed_us as f64;
                let offered = f64::from(plan.offered_rps);
                let measured =
                    f64::from(s.posted.saturating_sub(1)) * 1e6 / s.posting_span_us as f64;
                (s.posted == plan.operations_per_arm
                    && s.completed == s.posted
                    && s.posting_span_us > 0
                    && s.posting_span_us <= s.elapsed_us
                    && (0.99 * offered..=1.01 * offered).contains(&measured)
                    && u64::from(s.p99_us) <= s.elapsed_us
                    && s.errors == 0
                    && s.elapsed_us > 0
                    && s.p95_us > 0
                    && s.p95_us <= s.p99_us
                    && s.p95_us <= plan.max_p95_us
                    && s.p99_us <= plan.max_p99_us
                    && rate >= f64::from(plan.minimum_rps)
                    && rate <= 1.01 * f64::from(plan.offered_rps)
                    && s.source_commands > 0
                    && s.foreground_census_commands == 0
                    && s.source_commands == round.base.source_commands
                    && s.kms_calls == round.base.kms_calls)
                    .then_some([rate, f64::from(s.p95_us), f64::from(s.p99_us)])
            };
            let Some(base) = metrics(&round.base) else {
                return false;
            };
            for (series, arm) in comparisons.iter_mut().zip([
                &round.base_repeat,
                &round.candidate_idle,
                &round.candidate_capture,
                &round.candidate_publication,
            ]) {
                let Some(value) = metrics(arm) else {
                    return false;
                };
                series.push(std::array::from_fn(|i| (value[i] / base[i]).ln()));
            }
        }
        comparisons.iter().all(|sample| paired_equivalence(sample))
    }
}

// Deterministic paired bootstrap of mean log ratios, with 95% percentile
// endpoints. Identical resampled round indices apply to all three metrics.
fn paired_equivalence(sample: &[[f64; 3]]) -> bool {
    let mut rng = rand::rngs::StdRng::seed_from_u64(195507);
    let mut estimates: [Vec<f64>; 3] = Default::default();
    for _ in 0..10_000 {
        let mut sum = [0.; 3];
        for _ in sample {
            let row = sample[rng.gen_range(0..sample.len())];
            for i in 0..3 {
                sum[i] += row[i];
            }
        }
        for i in 0..3 {
            estimates[i].push(sum[i] / sample.len() as f64);
        }
    }
    estimates.iter_mut().all(|values| {
        values.sort_unstable_by(f64::total_cmp);
        values[249] > 0.99_f64.ln() && values[9749] < 1.01_f64.ln()
    })
}

fn valid_metadata(row: &Evidence) -> bool {
    row.schema_version == 1
        && row.receipt.revision.checked_sub(1) == Some(row.previous_revision)
        && row.receipt.recorded_at_ms > 0
        && contract::uuid(&row.receipt.entry_id).is_ok()
        && contract::uuid(&row.evidence_ref).is_ok()
        && contract::label(&row.owner).is_ok()
        && !row.producer.is_empty()
        && !row.issuer.is_empty()
}

fn declaration(row: Evidence, ring: &Scope, scope_revision: i64) -> AppResult<(Protocol, Receipt)> {
    if row.input_scope != *ring
        || row.receipt.scope != *ring
        || !valid_metadata(&row)
        || row.receipt.revision < scope_revision
    {
        return Err(stale());
    }
    let Change::DeclareLoadProtocol { protocol } =
        serde_json::from_value(row.assertion).map_err(|_| corrupt())?
    else {
        return Err(corrupt());
    };
    Ok((protocol, row.receipt))
}

pub(super) async fn record(
    db: &Database,
    tx: &mut ClientSession,
    head: &Document,
    ring: &Scope,
    receipt: &Receipt,
    change: &Change,
) -> AppResult<Document> {
    if &receipt.scope != ring {
        return Err(stale());
    }
    match change {
        Change::DeclareLoadProtocol { protocol } => {
            protocol.validate().await?;
            Ok(doc! {"$set":{"load_protocol":&receipt.entry_id},"$unset":{"load_result":""}})
        }
        Change::RecordLoadResult { result } => {
            if head.get_str("load_protocol") != Ok(&result.protocol_id) {
                return Err(stale());
            }
            let record = db
                .collection::<Evidence>(ENTRIES)
                .find_one(doc! {"_id":&result.protocol_id})
                .session(tx)
                .await?
                .ok_or_else(stale)?;
            let (plan, prior) = declaration(record, ring, field(head, "scope_revision")?)?;
            if !result.matches(&plan, &prior, receipt.recorded_at_ms) {
                return Err(invalid());
            }
            Ok(doc! {"$set":{"load_result":&receipt.entry_id}})
        }
        _ => Err(invalid()),
    }
}

pub(super) async fn verified(db: &Database, head: &Document) -> AppResult<()> {
    let result_id = head.get_str("load_result").map_err(|_| stale())?;
    let protocol_id = head.get_str("load_protocol").map_err(|_| stale())?;
    let ring: Scope = field(head, "scope")?;
    let entries = db.collection::<Evidence>(ENTRIES);
    let plan = entries
        .find_one(doc! {"_id":protocol_id})
        .await?
        .ok_or_else(stale)?;
    let (plan, prior) = declaration(plan, &ring, field(head, "scope_revision")?)?;
    plan.validate().await?;
    let record = entries
        .find_one(doc! {"_id":result_id})
        .await?
        .ok_or_else(stale)?;
    if !valid_metadata(&record)
        || record.receipt.recorded_at_ms
            > head
                .get_datetime("recorded_at")
                .map_err(|_| corrupt())?
                .timestamp_millis()
        || record.input_scope != ring
        || record.receipt.scope != ring
        || prior.entry_id != protocol_id
        || record.receipt.entry_id != result_id
        || record.receipt.revision <= prior.revision
        || record.receipt.revision > field::<i64>(head, "revision")?
    {
        return Err(stale());
    }
    let Change::RecordLoadResult { result } =
        serde_json::from_value(record.assertion).map_err(|_| corrupt())?
    else {
        return Err(corrupt());
    };
    if !result.matches(&plan, &prior, record.receipt.recorded_at_ms)
        || !tokio::task::spawn_blocking(move || result.qualifies(&plan))
            .await
            .map_err(|_| corrupt())?
    {
        return Err(stale());
    }
    Ok(())
}
