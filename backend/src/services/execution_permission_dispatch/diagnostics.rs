//! Operational telemetry, not product analytics (which would read consent).
//! Only closed classifications and the server occurrence UUID leave this module.
use super::*;
use std::sync::Mutex;

pub(super) async fn observe<T>(
    context: Option<&Arc<RequestContext>>,
    operation: impl std::future::Future<Output = AppResult<T>>,
) -> AppResult<T> {
    if let Some(context) = context {
        context.observation.start();
    }
    let result = operation.await;
    if let Some(context) = context {
        context.observation.finish(context, &result);
    }
    result
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Decision {
    pub outcome: &'static str,
    pub cause: &'static str,
}
impl Decision {
    pub(super) const fn new(outcome: &'static str, cause: &'static str) -> Self {
        Self { outcome, cause }
    }
    pub(super) fn gap(gap: contract::Gap) -> Self {
        Self::new(
            "requestable_gap",
            match gap {
                contract::Gap::MissingPermission => "permission_missing",
                contract::Gap::Subject => "subject_changed",
                contract::Gap::Binding => "connection_binding_changed",
                contract::Gap::Context => "context_changed",
                contract::Gap::Operation => "operation_or_inputs_outside_scope",
                contract::Gap::Validity => "validity_not_covering",
                contract::Gap::Floor(_) => "additional_floor_required",
            },
        )
    }
    pub(super) fn prohibited(reason: contract::Prohibition) -> Self {
        Self::new(
            "hard_denial",
            match reason {
                contract::Prohibition::Ceiling => "authority_ceiling_denied",
                contract::Prohibition::Guest => "guest_ineligible",
                contract::Prohibition::UnsupportedVersion => "permission_version_unsupported",
                contract::Prohibition::InvalidContract => "permission_contract_invalid",
                contract::Prohibition::Revoked => "permission_revoked",
                contract::Prohibition::Task => "task_not_live",
                contract::Prohibition::ClaimUsed => "operation_claim_used",
                contract::Prohibition::Resource => "resource_policy_denied",
                contract::Prohibition::IneligibleDecider => "decider_ineligible",
            },
        )
    }
    pub(super) fn failure(reason: contract::EvaluationError) -> Self {
        Self::new(
            "operational_error",
            match reason {
                contract::EvaluationError::AuthorityUnavailable => "authority_unavailable",
                contract::EvaluationError::ResourceVerificationRequired => {
                    "resource_verification_unavailable"
                }
                contract::EvaluationError::OutcomeUncertain => "provider_outcome_unknown",
            },
        )
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub(super) enum Stage {
    Enrollment,
    Task,
    Policy,
    #[default]
    Authority,
    Coverage,
    Proposal,
}
impl Stage {
    fn name(self) -> &'static str {
        match self {
            Self::Enrollment => "enrollment",
            Self::Task => "task",
            Self::Policy => "policy",
            Self::Authority => "authority",
            Self::Coverage => "coverage",
            Self::Proposal => "proposal",
        }
    }
    fn error(self, error: &AppError) -> Decision {
        use AppError::*;
        match error {
            SecurityReadinessUnavailable => {
                Decision::new("operational_error", "readiness_unavailable")
            }
            DatabaseError(_) | Internal(_) | OAuthAuthorityUnavailable => Decision::new(
                "operational_error",
                match self {
                    Self::Enrollment => "enrollment_resolution_unavailable",
                    _ => "authority_unavailable",
                },
            ),
            ApprovalRequired { .. } | ApprovalFailed { .. } => {
                Decision::new("requestable_gap", "permission_review_required")
            }
            ClientDisconnected => Decision::new("operational_error", "admission_abandoned"),
            _ if error.status_code().is_server_error() => {
                Decision::new("operational_error", "dependency_unavailable")
            }
            _ => Decision::new(
                "hard_denial",
                match self {
                    Self::Enrollment => "enrollment_drained",
                    Self::Task => "task_not_live",
                    Self::Policy => "policy_denied",
                    Self::Proposal => "proposal_not_live",
                    _ => "authority_ceiling_denied",
                },
            ),
        }
    }
}

#[derive(Default)]
struct Pending {
    stage: Stage,
    decision: Option<Decision>,
}

#[derive(Default)]
pub(super) struct Observation {
    pending: Mutex<Pending>,
    pub(super) reported: OnceLock<Decision>,
    started: std::sync::atomic::AtomicBool,
}
impl Observation {
    pub(super) fn start(&self) {
        self.started
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    pub(super) fn abandoned(&self, context: &RequestContext) {
        if self.started.load(std::sync::atomic::Ordering::Relaxed) && self.reported.get().is_none()
        {
            self.finish::<()>(context, &Err(AppError::ClientDisconnected));
        }
    }
    pub(super) fn stage(&self, stage: Stage) {
        self.pending.lock().unwrap_or_else(|e| e.into_inner()).stage = stage;
    }
    pub(super) fn decide(&self, decision: Decision) {
        self.pending
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .decision = Some(decision);
    }
    pub(super) fn finish<T>(&self, context: &RequestContext, result: &AppResult<T>) {
        let pending = self.pending.lock().unwrap_or_else(|e| e.into_inner());
        let decision = match result {
            Ok(_) => pending
                .decision
                .unwrap_or(Decision::new("legacy", "legacy_no_eligible_floor")),
            Err(error) => match pending.decision {
                Some(decision)
                    if decision.outcome == "hard_denial"
                        || decision.outcome == "operational_error" =>
                {
                    decision
                }
                Some(decision)
                    if matches!(
                        error,
                        AppError::ApprovalRequired { .. } | AppError::ApprovalFailed { .. }
                    ) =>
                {
                    decision
                }
                _ => pending.stage.error(error),
            },
        };
        // Recursion after a connected approval and prepared MCP re-entry share
        // the same occurrence. Emit the final aggregate, never each candidate.
        if self.reported.set(decision).is_ok() {
            // Only a whole matching contract supplies these fields. Never log
            // candidate contracts, resource selectors, or input-value digests.
            let permit = match context.decision.get() {
                Some(Resolution::Covered(permit)) if decision.outcome == "covered" => Some(permit),
                _ => None,
            };
            let validity = permit
                .and_then(|p| p.grant.execution.as_ref()?.v1.as_ref())
                .map(|c| match c.validity {
                    Validity::Operation { .. } => "operation",
                    Validity::Task { .. } => "task",
                    Validity::Window { .. } => "window",
                    Validity::Persistent => "persistent",
                });
            macro_rules! emit {
                ($level:expr) => {
                    tracing::event!(target: "nyxid::execution_permission", $level,
                    event = "execution_permission_decision",
                    operation_id = %context.operation_id,
                    route = if matches!(context.ingress.get(), Some(crate::models::execution_proposal::PreparedAdapter::Unsupported)) {
                        "legacy_unretainable_effect"
                    } else if decision.outcome == "legacy" { decision.cause } else { "explicit" },
                    decision = decision.outcome,
                    cause = decision.cause,
                    stage = pending.stage.name(),
                    resumed = context.resume().is_some(),
                    covered_lane = ?permit.map(|p| p.subject.lane),
                    covered_validity = validity,
                    covered_floors = ?permit.map(|p| p.floors.as_slice()),
                    operation_claim_required = validity == Some("operation"),
                    "Execution permission admission");
                };
            }
            if decision.outcome == "legacy" {
                emit!(tracing::Level::DEBUG);
            } else {
                emit!(tracing::Level::INFO);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn poison(context: &RequestContext) {
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let _guard = context.observation.pending.lock().unwrap();
                panic!("poison diagnostic fixture");
            }))
            .is_err()
        );
        assert!(context.observation.pending.is_poisoned());
    }

    #[test]
    fn migration_poisoned_diagnostics_finish_and_drop_without_a_second_panic() {
        let context = RequestContext::enrolled(Some(1), "owner", "key", None).unwrap();
        poison(&context);
        context.observation.start();
        context.observation.stage(Stage::Coverage);
        context
            .observation
            .decide(Decision::new("legacy", "legacy_flag_off"));
        context.observation.finish(&context, &Ok::<_, AppError>(()));
        assert_eq!(
            context.observation.reported.get().unwrap().cause,
            "legacy_flag_off"
        );
        drop(context);

        // Drop runs during unwinding, with an unfinished, poisoned observation.
        // A second panic here would abort the test process, not be caught.
        assert!(
            catch_unwind(AssertUnwindSafe(|| {
                let context = RequestContext::enrolled(Some(1), "owner", "key", None).unwrap();
                poison(&context);
                context.observation.start();
                panic!("unwind an abandoned admission");
            }))
            .is_err()
        );
    }
}
