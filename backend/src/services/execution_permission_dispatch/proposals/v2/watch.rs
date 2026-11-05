use super::*;
use crate::models::async_service_operation::AsyncServiceOperation;

pub(crate) enum Admission {
    Fresh,
    Resume(Box<(ApprovalRequest, Resume)>),
    Result(crate::services::mcp_service::ToolResponse),
}

pub(crate) async fn admit(
    state: &crate::AppState,
    auth: &mut crate::mw::auth::AuthUser,
    w: &AsyncServiceOperation,
    operation: &str,
) -> AppResult<Admission> {
    let Some(context) = &auth.execution_permission else {
        return Ok(Admission::Fresh);
    };
    let current = state
        .db
        .collection::<Document>("assistant_async_operations")
        .find_one(doc! {"_id":&w.id,"lease_id":&w.lease_id})
        .await?
        .ok_or_else(refused)?;
    let Some(pending) = current.get_document("execution_pending").ok() else {
        context.retain_adapter(
            PreparedAdapter::Watch {
                operation: operation.into(),
            },
            &axum::http::HeaderMap::new(),
        );
        return Ok(Admission::Fresh);
    };
    let id = pending.get_str("request_id").map_err(|_| refused())?;
    let pending_operation = pending.get_str("operation").map_err(|_| refused())?;
    if pending_operation != operation {
        if Some(operation) == w.contract.cancel_operation.as_deref() {
            delivery::cancel(&state.db, id, "watch_cancelled").await?;
            clear(&state.db, w, id).await?;
            context.retain_adapter(
                PreparedAdapter::Watch {
                    operation: operation.into(),
                },
                &axum::http::HeaderMap::new(),
            );
            return Ok(Admission::Fresh);
        }
        return Err(AppError::ApprovalRequired {
            request_id: id.into(),
        });
    }
    let row = state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! {"_id":id,"user_id":&w.user_id,"execution_proposal.v2.key_id":&w.api_key_id})
        .await?
        .ok_or_else(refused)?;
    let p = proposal(&row)?;
    if p.state == "completed" {
        let result = super::readback::decrypt_result(state, &row)
            .await?
            .ok_or_else(refused)?;
        if result.partial {
            return Err(refused());
        }
        // Status is a repeatable observation. Keep result/cancel receipts until
        // the watch's terminal update commits, so a crashed worker cannot send
        // either operation again after its lease rolls over.
        if operation == w.contract.status_operation {
            clear(&state.db, w, id).await?;
        }
        return Ok(Admission::Result(
            crate::services::mcp_service::ToolResponse::execution_receipt(
                result.status,
                result.body,
            ),
        ));
    }
    if row.status != "approved" || !matches!(p.state.as_str(), "queued" | "preparing") {
        return Err(super::pending(&row, &state.config.frontend_url));
    }
    let Some((resume, adapter)) = delivery::claim(state, &row).await? else {
        return Err(super::pending(&row, &state.config.frontend_url));
    };
    if !matches!(&adapter,PreparedAdapter::Watch{operation:o} if o==operation) {
        return Err(refused());
    }
    delivery::attach_resume(auth, resume.clone())?;
    auth.execution_permission
        .as_ref()
        .ok_or_else(refused)?
        .retain_adapter(adapter, &axum::http::HeaderMap::new());
    Ok(Admission::Resume(Box::new((row, resume))))
}
async fn clear(db: &Database, w: &AsyncServiceOperation, id: &str) -> AppResult<()> {
    if db.collection::<Document>("assistant_async_operations").update_one(doc!{"_id":&w.id,"lease_id":&w.lease_id,"lease_until":{"$gt":bson::DateTime::now()},"execution_pending.request_id":id},doc!{"$unset":{"execution_pending":""}}).await?.matched_count!=1{return Err(refused());}
    Ok(())
}
pub(crate) async fn complete(
    state: &crate::AppState,
    w: &AsyncServiceOperation,
    admission: Admission,
    outcome: AppResult<crate::services::mcp_service::ToolResponse>,
) -> AppResult<crate::services::mcp_service::ToolResponse> {
    let Admission::Resume(pair) = admission else {
        return outcome;
    };
    let (row, resume) = *pair;
    let response = outcome
        .as_ref()
        .map(|r| ExecutionResult {
            status: r.status,
            body: r.text.clone(),
            headers: Vec::new(),
            partial: r.media.is_some() || r.text.len() + 4096 > MAX_PAYLOAD,
        })
        .ok();
    let current = state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! {"_id":&row.id,"execution_proposal.v2.claim_id":&resume.claim_id})
        .await?
        .ok_or_else(refused)?;
    let sent = proposal(&current)?.state == "dispatched";
    if !sent && let Err(AppError::ApprovalRequired { request_id }) = &outcome {
        state.db.collection::<Document>(REQUESTS).update_one(
            doc! {"_id": &row.id, "execution_proposal.v2.claim_id": &resume.claim_id, "execution_proposal.v2.state": "preparing"},
            doc! {"$set": {"execution_proposal.v2.state": "waiting", "execution_proposal.v2.independent_request_id": request_id}},
        ).await?;
        return outcome;
    }

    let next = if sent && response.is_some() {
        "completed"
    } else if sent {
        "unknown"
    } else {
        "not_sent"
    };
    delivery::finish(
        state,
        &row,
        &resume,
        next,
        if next == "completed" {
            None
        } else {
            Some("watch_operation_unavailable")
        },
        response,
    )
    .await?;
    if next == "completed"
        && matches!(&proposal(&row)?.source, ProposalSource::Watch {operation,..} if operation == &w.contract.status_operation)
    {
        clear(&state.db, w, &row.id).await?;
    }
    outcome
}
