//! Legacy connected-wait semantics. A dropped waiter can never become background work.
use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

pub(crate) struct WaitGuard {
    db: Database,
    id: String,
    armed: AtomicBool,
    pub(crate) cancellation: tokio_util::sync::CancellationToken,
}
impl WaitGuard {
    pub(super) fn new(
        db: Database,
        id: String,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Self {
        Self {
            db,
            id,
            armed: AtomicBool::new(true),
            cancellation,
        }
    }
    pub(crate) fn disarm(&self) {
        self.armed.store(false, Ordering::Release);
    }
}
impl Drop for WaitGuard {
    fn drop(&mut self) {
        if self.armed.load(Ordering::Acquire) && tokio::runtime::Handle::try_current().is_ok() {
            let db = self.db.clone();
            let id = self.id.clone();
            crate::services::shutdown::process().spawn_cleanup(async move {
                // A transaction cancelled at commit can resolve after its caller
                // drops. Retry that cleanup briefly; the durable deadline and
                // background=false fence remain fail-closed after a process crash.
                for _ in 0..3 {
                    if abandon(&db, &id, "approval_wait_disconnected")
                        .await
                        .is_ok_and(|found| found)
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
                }
            });
        }
    }
}

pub(crate) async fn abandon(db: &Database, id: &str, code: &str) -> AppResult<bool> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db,id,code), |session,(db,id,code)| Box::pin(async move {
        let result: AppResult<bool> = async {
            let Some(row) = db.collection::<ApprovalRequest>(REQUESTS).find_one_and_update(
                doc! {"_id":*id,"execution_proposal.version":2,"execution_proposal.v2.background_allowed":{"$ne":true},
                    "execution_proposal.v2.state":{"$in":["pending","queued","preparing","waiting"]}},
                doc! {"$set":{"status":"expired","expires_at":bson::DateTime::now(),
                    "execution_proposal.v2.state":"not_sent","execution_proposal.v2.failure_code":*code},
                    "$unset":{"execution_proposal.v2.payload":"","execution_proposal.v2.result":""}},
            ).session(&mut *session).await? else { return Ok(false); };
            super::super::lifecycle::retire_operations(db, session, std::slice::from_ref(&row.id)).await?;
            audit(db, session, &row.user_id, "execution_permission_wait_abandoned", &row.id).await?;
            Ok(true)
        }.await;
        crate::services::api_key_mutation_service::transaction_result(result)
    })).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

fn wait_error(id: &str, reason: &str, frontend_url: &str) -> AppError {
    crate::services::approval_service::map_wait_for_decision_error(
        AppError::Forbidden(reason.into()),
        id,
        frontend_url,
    )
}

pub(crate) async fn wait(
    state: &crate::AppState,
    context: &RequestContext,
    request: &ApprovalRequest,
) -> AppResult<bool> {
    let db = &state.db;
    let keys = &state.encryption_keys;
    let frontend_url = &state.config.frontend_url;
    let p = proposal(request)?;
    if p.background_allowed {
        return Ok(false);
    }
    if p.effect_id != context.operation_id {
        return Err(refused());
    }
    let guard = context.wait_guard.get().cloned().ok_or_else(refused)?;
    let token = context.cancellation.get().cloned().unwrap_or_default();
    loop {
        if token.is_cancelled() {
            abandon(db, &request.id, "approval_wait_disconnected").await?;
            guard.disarm();
            return Err(AppError::ClientDisconnected);
        }
        if p.waiter_until.is_none_or(|deadline| deadline <= Utc::now()) {
            abandon(db, &request.id, "approval_wait_timed_out").await?;
            guard.disarm();
            return Err(wait_error(
                &request.id,
                "Approval request timed out",
                frontend_url,
            ));
        }
        let row = db
            .collection::<ApprovalRequest>(REQUESTS)
            .find_one(doc! {"_id":&request.id})
            .await?
            .ok_or_else(missing)?;
        match row.status.as_str() {
            "approved" => {
                let (resume, _) = delivery::claim_inner(db, keys, &row, Some(guard))
                    .await?
                    .ok_or_else(refused)?;
                context
                    .connected_resume
                    .set(Arc::new(resume))
                    .map_err(|_| refused())?;
                return Ok(true);
            }
            "rejected" => {
                guard.disarm();
                return Err(wait_error(
                    &row.id,
                    "Approval request was rejected",
                    frontend_url,
                ));
            }
            "expired" => {
                guard.disarm();
                return Err(wait_error(
                    &row.id,
                    if proposal(&row)?.failure_code.as_deref() == Some("approval_wait_timed_out") {
                        "Approval request timed out"
                    } else {
                        "Approval request expired"
                    },
                    frontend_url,
                ));
            }
            "pending" => {}
            _ => return Err(refused()),
        }
        tokio::select! {
            _ = token.cancelled() => {},
            _ = tokio::time::sleep(std::time::Duration::from_millis(1000)) => {},
        }
    }
}

pub(crate) async fn complete(db: &Database, resume: &Resume, received: bool) -> AppResult<()> {
    db.collection::<Document>(REQUESTS).update_one(
        doc! {"_id":&resume.request_id,"execution_proposal.version":2,
            "execution_proposal.v2.claim_id":&resume.claim_id,"execution_proposal.v2.state":"dispatched"},
        doc! {"$set":{"execution_proposal.v2.state":if received {"completed"} else {"unknown"},
            "execution_proposal.v2.failure_code":if received {None} else {Some("provider_outcome_unknown")}},
            "$unset":{"execution_proposal.v2.payload":""}},
    ).await?;
    Ok(())
}
