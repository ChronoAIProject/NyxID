//! External evidence is independent of a successful local HMAC walk. Persisted
//! witness cursors cannot establish fresh contact in a restarted process.
use super::{AuditDiagnostics, AuditRuntime};
use chrono::{DateTime, Utc};
use futures::TryStreamExt;
use mongodb::{ClientSession, Database, bson::doc};
use serde::Serialize;

use crate::{
    errors::AppResult,
    models::audit_state::{AuditState, COLLECTION_NAME},
    services::audit_chain_service::AuditEvidenceStatus,
};

/// Safe diagnostics, deliberately separate from the local verification result.
/// No model, event payload, credential, endpoint or verification key is exposed.
#[derive(Clone, Debug, Serialize)]
pub struct ExternalAuditEvidence {
    pub status: AuditEvidenceStatus,
    pub diagnostics: AuditDiagnostics,
    pub local_head_seq: Option<i64>,
    pub witnessed_seq: Option<i64>,
    pub pending_count: Option<i64>,
    pub oldest_pending_at: Option<DateTime<Utc>>,
}

/// One aggregation supplies the persisted backlog and Mongo server clock in
/// the same read. A process clock/restart cannot reset oldest_pending_at.
pub async fn external_evidence(
    db: &Database,
    runtime: &AuditRuntime,
) -> AppResult<ExternalAuditEvidence> {
    let document = db
        .collection::<mongodb::bson::Document>(COLLECTION_NAME)
        .aggregate(snapshot_pipeline())
        .await?
        .try_next()
        .await?;
    let (state, now) = decode_snapshot(document)?;
    let diagnostics = runtime.diagnostics(state.as_ref(), now);
    Ok(match state {
        Some(state) => ExternalAuditEvidence {
            // An enrollment/cursor stored in Mongo is never fresh external proof.
            status: if diagnostics.witness == super::policy::WitnessState::Diverged {
                AuditEvidenceStatus::Broken
            } else if diagnostics.witness == super::policy::WitnessState::Unavailable {
                AuditEvidenceStatus::Unavailable
            } else if diagnostics.witness == super::policy::WitnessState::Unwitnessed {
                AuditEvidenceStatus::Unwitnessed
            } else {
                AuditEvidenceStatus::Unverified
            },
            diagnostics,
            local_head_seq: Some(state.head_seq),
            witnessed_seq: state.witness_enrolled.then_some(state.witnessed_seq),
            pending_count: Some(state.pending_count),
            oldest_pending_at: state.oldest_pending_at,
        },
        None => ExternalAuditEvidence {
            diagnostics,
            status: AuditEvidenceStatus::Unavailable,
            local_head_seq: None,
            witnessed_seq: None,
            pending_count: None,
            oldest_pending_at: None,
        },
    })
}

fn snapshot_pipeline() -> Vec<bson::Document> {
    vec![
        doc! {"$match": {"singleton": true}},
        doc! {"$set": {"__audit_database_now": "$$NOW"}},
    ]
}

fn decode_snapshot(
    document: Option<bson::Document>,
) -> AppResult<(Option<AuditState>, DateTime<Utc>)> {
    match document {
        Some(document) => {
            let now = document
                .get_datetime("__audit_database_now")
                .map_err(|_| crate::errors::AppError::AuditIntegrityConflict)?
                .to_chrono();
            let state = bson::from_document::<AuditState>(document)
                .map_err(|_| crate::errors::AppError::AuditIntegrityConflict)?;
            Ok((Some(state), now))
        }
        None => Ok((None, Utc::now())),
    }
}

/// Admission and reservation share the caller's snapshot transaction and the
/// head CAS. No separate count query can race the eventual reservation.
pub(in crate::services) async fn load_in_session(
    db: &Database,
    session: &mut ClientSession,
) -> AppResult<(Option<AuditState>, DateTime<Utc>)> {
    let mut cursor = db
        .collection::<bson::Document>(COLLECTION_NAME)
        .aggregate(snapshot_pipeline())
        .session(&mut *session)
        .await?;
    decode_snapshot(cursor.next(session).await.transpose()?)
}
