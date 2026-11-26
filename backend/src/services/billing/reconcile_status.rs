//! Pass-boundary outcomes, published with the existing lease release. Leaf
//! retries and financial protocols retain their own errors and durable state.
use std::collections::BTreeMap;
use std::time::Duration;

use mongodb::{
    Database,
    bson::{self, Bson, Document, doc},
};
use serde::Deserialize;

use super::{BillingStartupDiagnostic, reconcile::RECONCILE_LEASE};
use crate::errors::{AppError, AppResult};
use crate::models::coordination::LEASE_COLLECTION_NAME;
use crate::services::coordination_service::{LeaseToken, active_lease_filter};

const STEPS: &[&str] = &[
    "exact_readiness",
    "taxes_before_recovery",
    "model_recovery",
    "pool_recovery",
    "voice_recovery",
    "managed_recovery",
    "abandon_unforwarded",
    "funding_releases",
    "grant_expiry",
    "schedules",
    "grant_ledger",
    "ledger_anchor",
    "settlement_recovery",
    "topup_expiry",
    "service_prices",
    "model_prices",
    "sandbox_prices",
    "machine_prices",
    "plan_repair",
    "taxes_after_prices",
    "usage_push",
    "usage_comparison",
    "wallet_balances",
    "rate_cache",
    "status_publication",
    "metering_retention",
];

const VERSION: i64 = 1;

struct State {
    raw: Document,
    failures: BTreeMap<String, Failure>,
}
impl Default for State {
    fn default() -> Self {
        Self {
            raw: doc! {"version":VERSION,"failures":{}},
            failures: BTreeMap::new(),
        }
    }
}

enum Checkpoint {
    Current(State),
    Newer(i64),
    Malformed,
}

#[derive(Deserialize)]
struct Failure {
    consecutive_passes: i64,
    first_failed_at: bson::DateTime,
    last_failed_at: bson::DateTime,
    error_code: u32,
    #[serde(default)]
    failing_units: Option<u64>,
}

fn unreadable_code() -> u32 {
    AppError::Internal("Billing reconcile status unreadable".into()).error_code()
}

fn decode(value: Option<Bson>) -> Checkpoint {
    let raw = match value {
        None | Some(Bson::Null) => return Checkpoint::Current(State::default()),
        Some(Bson::Document(raw)) => raw,
        _ => return Checkpoint::Malformed,
    };
    let version = match raw.get("version") {
        Some(Bson::Int32(n)) => i64::from(*n),
        Some(Bson::Int64(n)) => *n,
        _ => return Checkpoint::Malformed,
    };
    // A newer schema is opaque, including its entry shapes. Only its own
    // readers can update it; older holders still complete their monetary work.
    if version > VERSION {
        return Checkpoint::Newer(version);
    }
    if version != VERSION {
        return Checkpoint::Malformed;
    }
    let Ok(entries) = raw.get_document("failures") else {
        return Checkpoint::Malformed;
    };
    let mut failures = BTreeMap::new();
    for (step, entry) in entries {
        let Ok(failure) = bson::from_bson::<Failure>(entry.clone()) else {
            return Checkpoint::Malformed;
        };
        if failure.consecutive_passes <= 0
            || failure.error_code == 0
            || failure
                .failing_units
                .is_some_and(|n| n == 0 || n > i64::MAX as u64)
        {
            return Checkpoint::Malformed;
        }
        // Step names can come from a newer binary using this same schema.
        failures.insert(step.clone(), failure);
    }
    Checkpoint::Current(State { raw, failures })
}

pub(super) struct Pass {
    previous: Checkpoint,
    // Only attempted steps appear. Drain/config skips cannot clear a failure or
    // count as another failure; a successful attempt resets its streak.
    outcomes: BTreeMap<&'static str, Option<(u32, Option<u64>)>>,
    usage_warming: bool,
}

impl Pass {
    pub(super) fn new(checkpoint: Option<Bson>) -> Self {
        Self {
            previous: decode(checkpoint),
            outcomes: BTreeMap::new(),
            usage_warming: false,
        }
    }

    pub(super) fn observe<T>(&mut self, step: &'static str, result: AppResult<T>) -> AppResult<T> {
        debug_assert!(STEPS.contains(&step));
        crate::services::job_health::checkpoint(result.is_ok());
        if step == "usage_comparison" && result.is_ok() && super::reconcile::continue_reconcile() {
            self.usage_warming = true;
        }
        // Leaf batches return Ok(partial_stats) at a cooperative stop. That
        // does not prove that previously failing work in their tail succeeded.
        if result.is_err() || super::reconcile::continue_reconcile() {
            self.outcomes.insert(
                step,
                result.as_ref().err().map(|error| {
                    (
                        error.error_code(),
                        match error {
                            AppError::BillingRecoveryFailed { failed_units, .. } => {
                                Some(*failed_units)
                            }
                            _ => None,
                        },
                    )
                }),
            );
        }
        result
    }

    pub(super) fn record<T>(&mut self, step: &'static str, result: AppResult<T>) -> Option<T> {
        self.observe(step, result).ok()
    }

    pub(super) fn result<T>(&self, result: AppResult<T>) -> AppResult<T> {
        let mut failures: Vec<_> = self
            .outcomes
            .iter()
            .filter_map(|(step, code)| {
                code.map(|(code, units)| {
                    let mut label = format!("{step}:{code}");
                    if let Some(units) = units {
                        label.push_str(&format!(" failing_units={units}"));
                    }
                    if *step == "voice_recovery" {
                        label.push_str(" (billing_voice_window_recovery_failed)");
                    }
                    label
                })
            })
            .collect();
        if matches!(self.previous, Checkpoint::Malformed) {
            failures.push(format!("status_unreadable:{}", unreadable_code()));
        }
        if failures.is_empty() {
            result
        } else {
            Err(AppError::Internal(format!(
                "Billing reconcile failed steps: {}",
                failures.join(", ")
            )))
        }
    }

    pub(super) async fn release(&self, db: &Database, token: &LeaseToken) -> AppResult<bool> {
        let rows = db.collection::<Document>(LEASE_COLLECTION_NAME);
        let filter = active_lease_filter(&token.name, &token.holder, &token.lease_id);
        let mut set = doc! {"expires_at": bson::DateTime::from_millis(0), "updated_at": "$$NOW"};
        let mut clocks = Document::new();
        let previous = match &self.previous {
            Checkpoint::Current(state) => Some(state),
            Checkpoint::Malformed => None,
            Checkpoint::Newer(_) => {
                // Release the token without changing any checkpoint byte.
                set.insert("record_kind", "checkpoint");
                let n = rows
                    .update_one(filter, vec![doc! {"$set":set}])
                    .await?
                    .matched_count;
                crate::services::shutdown::process().record_release("cluster", n);
                return Ok(n == 1);
            }
        };
        let mut fresh = previous.map_or_else(|| State::default().raw, |state| state.raw.clone());
        let mut failures = fresh.get_document("failures").cloned().unwrap_or_default();
        // Work from the complete prior map, so unrecognized names and additive
        // entry fields survive even when this holder clears its own failures.
        for (step, outcome) in &self.outcomes {
            if let Some((code, units)) = outcome {
                let before = previous.and_then(|state| state.failures.get(*step));
                let mut entry = failures.get_document(*step).cloned().unwrap_or_default();
                entry.insert(
                    "consecutive_passes",
                    before.map_or(1, |f| f.consecutive_passes.saturating_add(1)),
                );
                entry.insert("error_code", i64::from(*code));
                if let Some(units) = units {
                    entry.insert("failing_units", i64::try_from(*units).unwrap_or(i64::MAX));
                } else {
                    entry.remove("failing_units");
                }
                if before.is_none() {
                    clocks.insert(
                        format!("checkpoint.failures.{step}.first_failed_at"),
                        "$$NOW",
                    );
                }
                clocks.insert(
                    format!("checkpoint.failures.{step}.last_failed_at"),
                    "$$NOW",
                );
                failures.insert(*step, entry);
            } else {
                failures.remove(*step);
            }
        }
        let warming = self.usage_warming || fresh.contains_key("usage_warming_since");
        if self.usage_warming && !fresh.contains_key("usage_warming_since") {
            clocks.insert("checkpoint.usage_warming_since", "$$NOW");
        }
        if failures.is_empty() && !warming {
            let n = rows.delete_one(filter).await?.deleted_count;
            crate::services::shutdown::process().record_release("cluster", n);
            return Ok(n == 1);
        }
        fresh.insert("failures", failures);
        // Literal copying preserves extensions containing '$' or expression
        // shapes. Only this pass's known timestamp fields are evaluated below.
        set.insert("checkpoint", doc! {"$literal":fresh});
        set.insert("record_kind", "checkpoint");
        let mut update = vec![doc! {"$set":set}];
        if !clocks.is_empty() {
            update.push(doc! {"$set":clocks});
        }
        let n = rows.update_one(filter, update).await?.matched_count;
        crate::services::shutdown::process().record_release("cluster", n);
        Ok(n == 1)
    }
}

pub(super) async fn diagnostics(db: &Database) -> Vec<BillingStartupDiagnostic> {
    let read = db
        .collection::<Document>(LEASE_COLLECTION_NAME)
        .find_one(doc! {"_id":RECONCILE_LEASE})
        .projection(doc! {"checkpoint":1})
        .max_time(Duration::from_secs(2))
        .await;
    let mut items = Vec::new();
    let (detail, detected_at) = match read {
        Err(error) => (
            format!(
                "reason=status_read_failed error_code={}",
                AppError::from(error).error_code()
            ),
            chrono::Utc::now(),
        ),
        Ok(row) => match decode(row.and_then(|mut r| r.remove("checkpoint"))) {
            Checkpoint::Malformed => (
                format!(
                    "reason=status_unreadable error_code={}; replaced by fresh status on the next completed pass",
                    unreadable_code()
                ),
                chrono::Utc::now(),
            ),
            Checkpoint::Newer(version) => (
                format!(
                    "reason=status_newer_version version={version} supported_version={VERSION}; checkpoint retained for a compatible holder"
                ),
                chrono::Utc::now(),
            ),
            Checkpoint::Current(state) => {
                if state.failures.is_empty() {
                    return items;
                }
                let detail = state.failures.iter().map(|(step, f)| format!(
                    "step={step} consecutive_passes={} first_failed_at={} last_failed_at={} error_code={}{}",
                    f.consecutive_passes, f.first_failed_at.to_chrono().to_rfc3339(),
                    f.last_failed_at.to_chrono().to_rfc3339(), f.error_code,
                    f.failing_units.map_or(String::new(), |n| format!(" failing_units={n}")),
                )).collect::<Vec<_>>().join("; ");
                let first = state
                    .failures
                    .values()
                    .map(|f| f.first_failed_at)
                    .min()
                    .unwrap();
                (detail, first.to_chrono())
            }
        },
    };
    items.push(BillingStartupDiagnostic {
        severity: super::DiagnosticSeverity::Action,
        technical_details: None,
        code: "billing_reconcile_steps".into(),
        summary: "Billing maintenance has failed steps".into(),
        detail,
        remediation: "Failed steps retry on later passes; successful attempted steps clear their failure status. Other independent steps continue.".into(),
        detected_at,
    });
    items
}
