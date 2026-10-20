//! Categorical state transitions commit through the ordinary safe-event outbox.
//! An in-memory observation is published as a transition only after that commit.
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use chrono::{DateTime, Utc};
use mongodb::Database;
use serde::Serialize;
use uuid::Uuid;

use crate::{
    errors::{AppError, AppResult},
    models::audit_outbox::{AuditOwner, AuditPrincipal, AuditStage, SystemAuditActor},
    services::{
        audit_outbox_service::{self, AuditEvent},
        audit_state_service,
    },
};

use super::{
    AuditDiagnostics, AuditRuntime,
    policy::{TierAdmission, WitnessState},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct TransitionKey {
    witness: WitnessState,
    admission: TierAdmission,
}

#[derive(Default)]
pub(super) struct MonitorState {
    last: Option<TransitionKey>,
    pub transition: Option<AuditTransition>,
    pending: Option<PendingTransition>,
    last_error_log: Option<Instant>,
}

struct PendingTransition {
    key: TransitionKey,
    operation_id: String,
    event_id: String,
    at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuditTransition {
    pub at: DateTime<Utc>,
    pub reason: &'static str,
}

fn reason(state: WitnessState) -> &'static str {
    match state {
        WitnessState::Unwitnessed => "unwitnessed",
        WitnessState::Ready => "ready",
        WitnessState::Stale => "stale",
        WitnessState::Backlogged => "backlogged",
        WitnessState::Diverged => "diverged",
        WitnessState::Unavailable => "unavailable",
    }
}

fn action(state: WitnessState) -> &'static str {
    match state {
        WitnessState::Unwitnessed => "audit_witness_unwitnessed",
        WitnessState::Ready => "audit_witness_ready",
        WitnessState::Stale => "audit_witness_stale",
        WitnessState::Backlogged => "audit_witness_backlogged",
        WitnessState::Diverged => "audit_witness_diverged",
        WitnessState::Unavailable => "audit_witness_unavailable",
    }
}

impl MonitorState {
    fn should_error_log(&mut self, now: Instant) -> bool {
        if self
            .last_error_log
            .is_some_and(|last| now.duration_since(last) < Duration::from_secs(60))
        {
            return false;
        }
        self.last_error_log = Some(now);
        true
    }
}

pub fn spawn_monitor(db: Database, runtime: Arc<AuditRuntime>) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if record_transition(&db, &runtime).await.is_err() {
                let mut monitor = runtime.monitor.lock().expect("audit monitor poisoned");
                if monitor.should_error_log(Instant::now()) {
                    tracing::error!(
                        "Audit witness transition recording unavailable; retrying durable event"
                    );
                }
            }
        }
    });
}

async fn commit_transition(
    db: &Database,
    operation_id: &str,
    event_id: &str,
    key: TransitionKey,
) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    let event = AuditEvent {
        operation_id: operation_id.to_owned(),
        stage: AuditStage::Committed,
        principal: AuditPrincipal::System {
            actor: SystemAuditActor::Maintenance,
        },
        owner: AuditOwner::Platform,
        action: if key.witness == WitnessState::Diverged
            && key.admission.availability == super::policy::Admission::Closed
        {
            "audit_witness_diverged_availability_closed"
        } else if key.witness == WitnessState::Backlogged
            && key.admission.availability == super::policy::Admission::Closed
        {
            "audit_witness_backlogged_availability_closed"
        } else {
            action(key.witness)
        },
        target_id: None,
    };
    for attempt in 0..8 {
        session
            .start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .write_concern(audit_state_service::majority())
            .await?;
        let result = match audit_outbox_service::enqueue_in_session(
            db,
            &mut session,
            event_id,
            &event,
        )
        .await
        {
            Ok(_) => audit_state_service::commit(&mut session)
                .await
                .map_err(AppError::from),
            Err(error) => Err(error),
        };
        match result {
            Err(AppError::DatabaseError(error))
                if attempt < 7 && error.contains_label("TransientTransactionError") =>
            {
                let _ = session.abort_transaction().await;
                tokio::task::yield_now().await;
            }
            result => return result,
        }
    }
    unreachable!("last transition attempt returns")
}

/// One monitor owns each runtime. It retains replay identities across unknown
/// commit results; completion/incident events are not new Standard admissions.
pub(super) async fn record_transition(db: &Database, runtime: &AuditRuntime) -> AppResult<()> {
    let report = super::external_evidence(db, runtime).await?.diagnostics;
    let key = TransitionKey {
        witness: report.witness,
        admission: report.admission,
    };
    let planned = {
        let mut monitor = runtime.monitor.lock().expect("audit monitor poisoned");
        if monitor.pending.is_none() && monitor.last != Some(key) {
            monitor.pending = Some(PendingTransition {
                key,
                operation_id: Uuid::new_v4().to_string(),
                event_id: Uuid::new_v4().to_string(),
                at: Utc::now(),
            });
        }
        monitor
            .pending
            .as_ref()
            .map(|p| (p.operation_id.clone(), p.event_id.clone(), p.key))
    };
    if let Some((operation, event, state)) = planned {
        commit_transition(db, &operation, &event, state).await?;
        let mut monitor = runtime.monitor.lock().expect("audit monitor poisoned");
        let pending = monitor.pending.take().expect("single runtime monitor");
        monitor.last = Some(pending.key);
        monitor.transition = Some(AuditTransition {
            at: pending.at,
            reason: reason(pending.key.witness),
        });
        if matches!(
            pending.key.witness,
            WitnessState::Ready | WitnessState::Unwitnessed
        ) {
            tracing::info!(
                witness = reason(pending.key.witness),
                "Audit witness state transition"
            );
        } else if monitor.should_error_log(Instant::now()) {
            log_alarm(&report);
        }
    } else {
        let mut monitor = runtime.monitor.lock().expect("audit monitor poisoned");
        if !matches!(key.witness, WitnessState::Ready | WitnessState::Unwitnessed)
            && monitor.should_error_log(Instant::now())
        {
            log_alarm(&report);
        }
    }
    Ok(())
}

fn log_alarm(report: &AuditDiagnostics) {
    tracing::error!(witness = reason(report.witness), admission = ?report.admission,
        pending_count = report.pending_count, oldest_pending_secs = report.oldest_pending_secs,
        last_contact_secs = report.last_contact_secs, local_head_seq = report.local_head_seq,
        witnessed_seq = report.witnessed_seq, "Audit witness admission alarm");
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::audit_outbox::{PENDING_COLLECTION_NAME, PendingAuditEvent};
    use mongodb::bson::doc;

    #[test]
    fn audit_c2_alarm_repeats_and_flapping_are_limited_to_once_per_minute() {
        let mut state = MonitorState::default();
        let now = Instant::now();
        assert!(state.should_error_log(now));
        assert!(!state.should_error_log(now + Duration::from_secs(59)));
        assert!(state.should_error_log(now + Duration::from_secs(60)));
    }

    #[tokio::test]
    async fn audit_c2_transitions_are_system_events_and_repeated_state_does_not_enqueue_again() {
        let db = crate::test_utils::connect_transaction_test_database("audit_c2_transitions").await;
        let runtime = AuditRuntime::new(Default::default(), true);
        record_transition(&db, &runtime).await.unwrap();
        record_transition(&db, &runtime).await.unwrap();
        assert_eq!(
            db.collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        let event = db
            .collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(event.event.action, "audit_witness_stale");
        assert!(matches!(
            event.event.principal,
            AuditPrincipal::System { .. }
        ));
        assert_eq!(
            runtime
                .monitor
                .lock()
                .unwrap()
                .transition
                .as_ref()
                .unwrap()
                .reason,
            "stale"
        );
        audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap();
        assert_eq!(
            audit_state_service::load(&db).await.unwrap().pending_count,
            1
        );
        let report =
            crate::services::audit_chain_service::verify_chain(&db, &[2; 32], None, None, None)
                .await
                .unwrap();
        assert!(report.break_info.is_none());
    }
}
