//! Safe operator projection of C2 admission. Live contact exists only in this
//! process; persisted enrollment/cursors never initialize it.
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::models::audit_state::AuditState;
use nyxid::services::audit_witness::VerifiedContact;

use super::policy::{AdmissionFacts, AdmissionLimits, TierAdmission, WitnessState};

pub struct AuditRuntime {
    pub limits: AdmissionLimits,
    pub(super) configured: bool,
    pub(super) effects: OnceLock<super::intent::EffectContext>,
    pub(super) monitor: Mutex<super::monitor::MonitorState>,
    session: Mutex<WitnessSession>,
}

#[derive(Default)]
struct WitnessSession {
    // Only the authenticated challenge-response type can populate this field.
    contact: Option<VerifiedContact>,
    divergence: Option<AuditDivergence>,
}

/// Fixed evidence only. No event body, provider message or signing material.
#[derive(Clone, Debug, Serialize)]
pub struct AuditDivergence {
    pub seq: i64,
    pub local_hash_prefix: String,
    pub witnessed_hash_prefix: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AuditDiagnostics {
    pub witness: WitnessState,
    pub admission: TierAdmission,
    pub chain_id: Option<String>,
    pub pending_count: Option<u64>,
    pub oldest_pending_secs: Option<u64>,
    pub last_contact_secs: Option<u64>,
    pub local_head_seq: Option<i64>,
    pub witnessed_seq: Option<i64>,
    pub standard_closes_in_secs: Option<u64>,
    pub availability_closes_in_secs: Option<u64>,
    pub divergence: Option<AuditDivergence>,
    pub last_transition: Option<super::monitor::AuditTransition>,
    pub limits: AdmissionLimits,
}

impl AuditRuntime {
    pub async fn restore_incident(
        &self,
        db: &mongodb::Database,
        chain: &str,
    ) -> crate::errors::AppResult<()> {
        use crate::models::audit_witness_delivery::{COLLECTION_NAME, WitnessDivergence};
        if let Some(saved) = db
            .collection::<bson::Document>(COLLECTION_NAME)
            .find_one(bson::doc! {"_id": chain})
            .projection(bson::doc! {"divergence": 1})
            .await?
            && let Some(value) = saved
                .get("divergence")
                .filter(|v| !matches!(v, bson::Bson::Null))
        {
            let detail =
                bson::from_bson::<WitnessDivergence>(value.clone()).unwrap_or(WitnessDivergence {
                    seq: 0,
                    local_hash: String::new(),
                    witnessed_hash: String::new(),
                });
            self.mark_divergence(&detail);
        }
        Ok(())
    }

    pub(super) fn accept_contact(&self, contact: VerifiedContact) {
        let mut session = self.session.lock().expect("audit witness session poisoned");
        if session.divergence.is_none() {
            session.contact = Some(contact);
        }
    }

    pub(super) fn mark_divergence(
        &self,
        detail: &crate::models::audit_witness_delivery::WitnessDivergence,
    ) {
        let mut session = self.session.lock().expect("audit witness session poisoned");
        if session.divergence.is_none() {
            let prefix = |hash: &str| {
                if nyxid::services::audit_witness::valid_hash(hash) {
                    hash[..12].to_owned()
                } else {
                    String::new()
                }
            };
            session.divergence = Some(AuditDivergence {
                seq: detail.seq.max(0),
                local_hash_prefix: prefix(&detail.local_hash),
                witnessed_hash_prefix: prefix(&detail.witnessed_hash),
            });
        }
        session.contact = None;
    }

    /// Configuration/enrollment never implies contact. Starting while the
    /// witness is down retains unknown freshness until a live observation.
    pub fn new(limits: AdmissionLimits, configured: bool) -> Self {
        Self {
            limits,
            configured,
            effects: OnceLock::new(),
            monitor: Mutex::new(Default::default()),
            session: Mutex::new(WitnessSession::default()),
        }
    }

    /// Evaluate a new operation in its existing business transaction. Callers
    /// must reserve all admission/terminal events with the same head CAS before
    /// commit. Replays/completion of previously reserved work are not new work.
    pub async fn require_new_in_session(
        &self,
        db: &mongodb::Database,
        session: &mut mongodb::ClientSession,
        tier: super::policy::AuditTier,
        reservations: u64,
    ) -> crate::errors::AppResult<()> {
        if reservations == 0 {
            return Err(crate::errors::AppError::AuditIntegrityConflict);
        }
        let (state, now) = super::load_in_session(db, session).await?;
        if tier != super::policy::AuditTier::Narrowing
            && (self.configured || state.as_ref().is_some_and(|s| s.witness_enrolled))
        {
            use crate::models::audit_witness_delivery::{COLLECTION_NAME, WitnessDivergence};
            if let Some(state) = &state
                && let Some(progress) = db
                    .collection::<bson::Document>(COLLECTION_NAME)
                    .find_one(bson::doc! {"_id": &state.id})
                    .projection(bson::doc! {"divergence": 1})
                    .session(&mut *session)
                    .await?
                && let Some(value) = progress
                    .get("divergence")
                    .filter(|value| !matches!(value, bson::Bson::Null))
            {
                let detail = bson::from_bson::<WitnessDivergence>(value.clone()).unwrap_or(
                    WitnessDivergence {
                        seq: 0,
                        local_hash: String::new(),
                        witnessed_hash: String::new(),
                    },
                );
                self.mark_divergence(&detail);
            }
        }
        let report = self.diagnostics(state.as_ref(), now);
        let facts = AdmissionFacts {
            configured_or_enrolled: self.configured
                || state.as_ref().is_some_and(|s| s.witness_enrolled),
            state_available: report.witness != WitnessState::Unavailable,
            diverged: report.divergence.is_some(),
            // At count L-1 one event can reserve the Lth slot. A bundle of N
            // events must fit completely; no partial terminal capacity promise.
            pending_count: report
                .pending_count
                .unwrap_or(0)
                .saturating_add(reservations - 1),
            oldest_pending_secs: report.oldest_pending_secs.unwrap_or(0),
            last_contact_secs: report.last_contact_secs,
        };
        facts.require(tier, &self.limits)
    }

    pub fn diagnostics(
        &self,
        state: Option<&AuditState>,
        database_now: DateTime<Utc>,
    ) -> AuditDiagnostics {
        let session = self.session.lock().expect("audit witness session poisoned");
        let oldest_pending_secs = state.map(|s| match s.oldest_pending_at {
            Some(at) => database_now.signed_duration_since(at).num_seconds().max(0) as u64,
            // A positive count without an age is corrupt bookkeeping, never a
            // fresh backlog. Treat it as a closed backlog, not a reset clock.
            None if s.pending_count > 0 => super::policy::MAX_AGE_SECS,
            None => 0,
        });
        let pending_count = state.and_then(|s| u64::try_from(s.pending_count).ok());
        let facts = AdmissionFacts {
            configured_or_enrolled: self.configured || state.is_some_and(|s| s.witness_enrolled),
            state_available: state.is_some_and(|s| {
                pending_count.is_some()
                    && s.next_order >= s.pending_count
                    && (s.pending_count == 0) == s.oldest_pending_at.is_none()
                    && s.head_seq >= 0
                    && s.witnessed_seq >= 0
                    && s.witnessed_seq <= s.head_seq
            }),
            diverged: session.divergence.is_some(),
            pending_count: pending_count.unwrap_or(0),
            oldest_pending_secs: oldest_pending_secs.unwrap_or(0),
            last_contact_secs: session.contact.as_ref().map(|c| c.age().as_secs()),
        };
        let closes_in = |limit: super::policy::TierLimit| {
            (facts.state_available && facts.configured_or_enrolled && facts.pending_count > 0).then(
                || {
                    if facts.pending_count >= limit.pending_count {
                        0
                    } else {
                        limit
                            .pending_age_secs
                            .saturating_sub(facts.oldest_pending_secs)
                    }
                },
            )
        };
        AuditDiagnostics {
            witness: facts.witness_state(&self.limits),
            admission: facts.tiers(&self.limits),
            chain_id: state.map(|s| s.id.clone()),
            pending_count,
            oldest_pending_secs,
            last_contact_secs: facts.last_contact_secs,
            local_head_seq: state.map(|s| s.head_seq),
            witnessed_seq: state
                .filter(|s| s.witness_enrolled)
                .map(|s| s.witnessed_seq),
            standard_closes_in_secs: if facts.diverged {
                Some(0)
            } else {
                closes_in(self.limits.standard)
            },
            availability_closes_in_secs: closes_in(self.limits.availability),
            divergence: session.divergence.clone(),
            last_transition: self
                .monitor
                .lock()
                .expect("audit monitor poisoned")
                .transition
                .clone(),
            limits: self.limits.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::audit_state_service;
    use mongodb::bson::doc;

    #[tokio::test]
    async fn audit_c2_shared_incident_fences_standard_without_blocking_narrowing_or_bounded_availability()
     {
        use super::super::policy::AuditTier;
        let db =
            crate::test_utils::connect_transaction_test_database("audit_c2_shared_incident").await;
        let state = audit_state_service::load(&db).await.unwrap();
        // Admission reads only the incident projection. Corrupt unrelated
        // witness-cursor data cannot turn a witness incident into auth lockout.
        db.collection::<bson::Document>(crate::models::audit_witness_delivery::COLLECTION_NAME)
            .insert_one(doc! {"_id": &state.id, "divergence": "malformed incident", "receipt": "damaged cursor"}).await.unwrap();
        let runtime = AuditRuntime::new(Default::default(), true);
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        runtime
            .require_new_in_session(&db, &mut session, AuditTier::Narrowing, 1)
            .await
            .unwrap();
        runtime
            .require_new_in_session(&db, &mut session, AuditTier::Availability, 1)
            .await
            .unwrap();
        assert!(matches!(
            runtime
                .require_new_in_session(&db, &mut session, AuditTier::Standard, 1)
                .await,
            Err(crate::errors::AppError::AuditWitnessDiverged)
        ));
        assert!(matches!(
            runtime
                .require_new_in_session(&db, &mut session, AuditTier::Effect, 1)
                .await,
            Err(crate::errors::AppError::AuditWitnessDiverged)
        ));
        session.abort_transaction().await.unwrap();
    }

    #[tokio::test]
    async fn audit_c2_restart_unreachable_preserves_pending_age_without_replaying_receipts() {
        let db = crate::test_utils::connect_transaction_test_database("audit_c2_restart").await;
        let now = Utc::now();
        let old = now - chrono::Duration::seconds(3_600);
        db.collection::<mongodb::bson::Document>(crate::models::audit_state::COLLECTION_NAME)
            .update_one(
                doc! {"singleton": true},
                doc! {"$set": {
                    "witness_enrolled": true, "witnessed_seq": 0_i64, "next_order": 1_i64,
                    "pending_count": 1_i64, "oldest_pending_at": bson::DateTime::from_chrono(old)
                }},
            )
            .await
            .unwrap();
        let state = audit_state_service::load(&db).await.unwrap();
        for _restart in 0..2 {
            let runtime = AuditRuntime::new(AdmissionLimits::default(), true);
            let report = runtime.diagnostics(Some(&state), now);
            assert!(report.last_contact_secs.is_none());
            assert_eq!(report.oldest_pending_secs, Some(3_600));
            assert_eq!(report.standard_closes_in_secs, Some(0));
            assert_eq!(report.availability_closes_in_secs, Some(82_800));
            assert_eq!(
                report.admission.standard,
                super::super::policy::Admission::Closed
            );
            assert_eq!(
                report.admission.availability,
                super::super::policy::Admission::Alarm
            );
            assert_ne!(report.witness, WitnessState::Ready);
        }
        let quiet = AuditState {
            pending_count: 0,
            oldest_pending_at: None,
            ..state
        };
        let report =
            AuditRuntime::new(AdmissionLimits::default(), true).diagnostics(Some(&quiet), now);
        assert_eq!(report.witness, WitnessState::Stale);
        assert!(report.last_contact_secs.is_none());
    }
    #[tokio::test]
    async fn audit_c2_transaction_reads_backlog_and_requires_whole_reservation_bundle() {
        use super::super::policy::AuditTier;
        use crate::errors::{AppError, AuditAdmissionReason};
        let db = crate::test_utils::connect_transaction_test_database("audit_c2_transaction").await;
        db.collection::<mongodb::bson::Document>(crate::models::audit_state::COLLECTION_NAME)
            .update_one(
                doc! {"singleton": true},
                doc! {"$set": {
                    "witness_enrolled": true, "pending_count": 99_999_i64, "next_order": 99_999_i64,
                    "oldest_pending_at": bson::DateTime::now(),
                }},
            )
            .await
            .unwrap();
        let runtime = AuditRuntime::new(AdmissionLimits::default(), true);
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        // Unknown contact alarms DB-only work but cannot refuse it.
        runtime
            .require_new_in_session(&db, &mut session, AuditTier::Standard, 1)
            .await
            .unwrap();
        assert!(matches!(
            runtime
                .require_new_in_session(&db, &mut session, AuditTier::Standard, 2)
                .await,
            Err(AppError::AuditAdmissionClosed {
                reason: AuditAdmissionReason::Backlogged
            })
        ));
        runtime
            .require_new_in_session(&db, &mut session, AuditTier::Availability, 2)
            .await
            .unwrap();
        runtime
            .require_new_in_session(&db, &mut session, AuditTier::Narrowing, 1_000_000)
            .await
            .unwrap();
        assert!(matches!(
            runtime
                .require_new_in_session(&db, &mut session, AuditTier::Effect, 1)
                .await,
            Err(AppError::AuditAdmissionClosed {
                reason: AuditAdmissionReason::WitnessUnavailable
            })
        ));
        session.abort_transaction().await.unwrap();
    }
    #[tokio::test]
    async fn audit_c2_divergence_reports_incident_and_keeps_narrowing_and_bounded_availability() {
        use super::super::policy::Admission;
        let db = crate::test_utils::connect_transaction_test_database("audit_c2_divergence").await;
        let state = audit_state_service::load(&db).await.unwrap();
        let runtime = AuditRuntime::new(AdmissionLimits::default(), true);
        runtime.session.lock().unwrap().divergence = Some(AuditDivergence {
            seq: 1,
            local_hash_prefix: "aaaaaaaaaaaa".into(),
            witnessed_hash_prefix: "bbbbbbbbbbbb".into(),
        });
        let report = super::super::external_evidence(&db, &runtime)
            .await
            .unwrap()
            .diagnostics;
        assert_eq!(report.witness, WitnessState::Diverged);
        assert_eq!(report.admission.narrowing, Admission::Alarm);
        assert_eq!(report.admission.availability, Admission::Alarm);
        assert_eq!(report.admission.standard, Admission::Closed);
        assert_eq!(report.admission.effect, Admission::Closed);
        assert_eq!(report.standard_closes_in_secs, Some(0));
        assert_eq!(report.divergence.unwrap().seq, 1);
        assert!(report.last_contact_secs.is_none());
        let old = Utc::now() - chrono::Duration::seconds(86_400);
        let backlog = AuditState {
            pending_count: 1,
            next_order: 1,
            oldest_pending_at: Some(old),
            ..state
        };
        let report = runtime.diagnostics(Some(&backlog), Utc::now());
        assert_eq!(report.admission.narrowing, Admission::Alarm);
        assert_eq!(report.admission.availability, Admission::Closed);
    }
}
