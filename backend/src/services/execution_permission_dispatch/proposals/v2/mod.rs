//! Non-chat review occurrences. The source is a fence, never bearer authority.
use super::*;
use crate::models::execution_proposal::{
    ExecutionResult, PreparedAdapter, ProposalSource, ProposalV2, RetainedEffect,
};
pub(crate) mod blocking;
mod decision;
mod delivery;
mod readback;
mod source;
pub(crate) mod watch;
pub(crate) use decision::decide;
pub(crate) use delivery::{attach_resume, cancel_in_session, fence_resume, resume_voice, sweep};
pub(crate) use readback::{ExecutionView, decrypt_result, readback, view};
pub(in crate::services::execution_permission_dispatch) use source::existing;
use source::*;
pub(crate) use source::{eligible, pin_pool};
pub(crate) const READINESS: &str = "permissions:execution-authoring-v2";

pub(crate) fn proposal(row: &ApprovalRequest) -> AppResult<&ProposalV2> {
    row.execution_proposal
        .as_ref()
        .filter(|e| e.version == 2 && e.v1.is_none())
        .and_then(|e| e.v2.as_ref())
        .ok_or_else(conflict)
}
fn aad(id: &str) -> String {
    format!("execution-proposal:v2:{id}")
}
pub(crate) fn pending(row: &ApprovalRequest, frontend_url: &str) -> AppError {
    if (row.status == "pending" || row.status == "approved") && row.expires_at > Utc::now() {
        AppError::ApprovalRequired {
            request_id: row.id.clone(),
        }
    } else {
        AppError::ApprovalFailed {
            request_id: row.id.clone(),
            reason: "Review this occurrence’s execution status; it will not be repeated".into(),
            approve_url: approve_url(frontend_url),
        }
    }
}
#[allow(clippy::too_many_arguments)]
pub(in crate::services::execution_permission_dispatch) async fn create(
    state: &crate::AppState,
    context: &RequestContext,
    chat: Option<&acks::ChatAuthority>,
    effect: &Effect<'_>,
    subject: Subject,
    binding: ConnectionBinding,
    audience: Audience,
    digest: String,
    snapshot: &authority::Snapshot,
    gaps: &[contract::Gap],
) -> AppResult<Option<ApprovalRequest>> {
    let db = &state.db;
    let keys = &state.encryption_keys;
    if !eligible(context, chat) || effect.floors.is_empty() {
        return Ok(None);
    }
    if effect.floors.iter().any(|f| !contract::satisfiable(*f)) {
        return Err(refused());
    }
    // External keys are hash-only. Replaying without the original forwarded
    // bearer would change the reviewed request; retaining that bearer is not
    // an option. Keep these sources on their existing prompt chain.
    if effect.target.service.forward_access_token {
        tracing::info!(
            reason = "source_bearer_not_retainable",
            "Execution proposal uses legacy prompts"
        );
        return Ok(None);
    }
    let endpoint_digest = if let Some(id) = effect
        .endpoint
        .filter(|id| *id != crate::services::mcp_service::GENERIC_PROXY_ENDPOINT_ID)
    {
        let Some(row) = db
            .collection::<Document>("service_endpoints")
            .find_one(doc! {"_id":id,"service_id":&effect.target.service.id})
            .await?
        else {
            tracing::info!(
                reason = "endpoint_not_recoverable",
                "Execution proposal uses legacy prompts"
            );
            return Ok(None);
        };
        if row.get_document("async_operation").is_ok() {
            tracing::info!(
                reason = "async_submission_not_retainable",
                "Execution proposal uses legacy prompts"
            );
            return Ok(None);
        }
        Some(endpoint_digest(&row)?)
    } else {
        None
    };
    let retained = match retain(effect, context) {
        Ok(v) => v,
        Err(reason) => {
            tracing::info!(reason, "Execution proposal uses legacy prompts");
            return Ok(None);
        }
    };
    let retained = RetainedEffect {
        effect: retained,
        adapter: context
            .ingress
            .get()
            .cloned()
            .unwrap_or(PreparedAdapter::Http),
    };
    let encoded = Zeroizing::new(serde_json::to_vec(&retained).map_err(encoding)?);
    // Reserve room for the envelope's wrapped key/nonce/tag as well as plaintext.
    if encoded.len() + 2048 > MAX_PAYLOAD {
        tracing::info!(
            reason = "payload_too_large",
            "Execution proposal uses legacy prompts"
        );
        return Ok(None);
    }
    crate::services::security_readiness_service::require(db, READINESS).await?;
    let Some(source) = source(db, context, chat).await? else {
        tracing::info!(
            reason = "source_not_recoverable",
            "Execution proposal uses legacy prompts"
        );
        return Ok(None);
    };
    let key = chat
        .map(|c| c.api_key_id.clone())
        .or_else(|| match &context.principal {
            Principal::ApiKey(k) => Some(k.clone()),
            _ => None,
        })
        .ok_or_else(refused)?;
    let task = chat.and_then(|c| c.execution_task.clone());
    if task
        .as_ref()
        .is_some_and(|t| t.actor_user_id != context.actor || t.audience != audience)
    {
        return Err(refused());
    }
    let now = Utc::now();
    let background_allowed = matches!(
        source,
        ProposalSource::Watch { .. } | ProposalSource::Voice { .. }
    ) || context
        .async_requested
        .load(std::sync::atomic::Ordering::Relaxed);
    // These proposals ask only the acting person to consent to personal floors.
    // This is the same first recipient/channel used by the legacy approval path.
    let timeout = if background_allowed {
        600
    } else {
        crate::services::notification_service::approval_timeout_secs(db, &context.actor).await?
    };
    let mut end = now + Duration::seconds(i64::from(timeout));
    if let Some(t) = &task {
        end = end.min(t.hard_deadline);
    }
    let key_row = db
        .collection::<crate::models::api_key::ApiKey>("api_keys")
        .find_one(doc! {"_id":&key,"user_id":&context.actor,"is_active":true})
        .await?
        .ok_or_else(refused)?;
    if let Some(expiry) = key_row.expires_at {
        end = end.min(expiry);
    }
    if let Some(child) = context.source_credential.get() {
        let row = db
            .collection::<Document>("api_key_credentials")
            .find_one(doc! {"_id":child,"api_key_id":&key,"is_active":true,"revoked_at":null})
            .await?
            .ok_or_else(refused)?;
        if let Ok(expiry) = row.get_datetime("expires_at") {
            end = end.min(expiry.to_chrono());
        }
    }
    if let Some(descendants::Descendant::Watch(w)) = context.descendant.as_deref() {
        end = end.min(w.deadline).min(w.expires_at);
    }
    if end <= now {
        return Err(refused());
    }
    let id = Uuid::new_v4().to_string();
    let encrypted = keys.encrypt_with_aad(&encoded, aad(&id).as_bytes()).await?;
    if encrypted.len() > MAX_PAYLOAD {
        tracing::info!(
            reason = "encrypted_payload_too_large",
            "Execution proposal uses legacy prompts"
        );
        return Ok(None);
    }
    let mut reusable = context.source_credential.get().is_none()
        && reusable_shape(
            effect.method,
            effect.body,
            effect.content_type,
            &scope_headers(
                effect.headers,
                context.generated_correlation.get().map(String::as_str),
            ),
        )
        && !sensitive_inputs(effect);
    let compiled = scope(effect, reusable).or_else(|error| {
        if reusable {
            reusable = false;
            scope(effect, false)
        } else {
            Err(error)
        }
    });
    let operations = match compiled {
        Ok(v) => v,
        Err(_) => {
            tracing::info!(
                reason = "inputs_not_representable",
                "Execution proposal uses legacy prompts"
            );
            return Ok(None);
        }
    };
    let contract = ExecutionContract {
        subject,
        binding,
        context: ExecutionContext {
            audience,
            conversation: chat.map_or(ConversationScope::SameAudience, |c| {
                ConversationScope::Exact {
                    conversation_id: c.conversation_id.clone(),
                }
            }),
        },
        operations,
        resource_policy: context.native.as_ref().map(|n| n.policy().clone()),
        validity: Validity::Operation {
            operation_id: context.operation_id.clone(),
            prepared_digest: digest.clone(),
            expires_at: end,
        },
        satisfies: effect.floors.to_vec(),
        revision: 1,
    };
    let authority_digest = snapshot.digest();
    let locator = locator(context, chat)?;
    let source_digest = fingerprint(
        &json!({"source":source,"task":task,"key":key,"child":context.source_credential.get(),"subject":contract.subject,"audience":contract.context,"adapter":adapter_identity(&retained.adapter)}),
    )?;
    let gap_key = fingerprint(
        &json!({"locator":locator,"source":source_digest,"digest":digest,"authority":authority_digest,"binding":contract.binding,"floors":effect.floors}),
    )?;

    let mut choices = vec![("operation", "This operation", contract.validity.clone())];
    if reusable {
        if let Some(t) = &task {
            choices.push((
                "task",
                "This task",
                Validity::Task {
                    task: t.task.clone(),
                    hard_deadline: t.hard_deadline,
                },
            ));
        }
        choices.extend([
            (
                "window",
                "For one hour",
                Validity::Window {
                    starts_at: now,
                    ends_at: now + Duration::hours(1),
                },
            ),
            (
                "persistent",
                "Until revoked or this account binding changes",
                Validity::Persistent,
            ),
        ]);
    }
    let choices = choices
        .into_iter()
        .map(|(id, label, validity)| {
            Ok(Choice {
                id: id.into(),
                label: label.into(),
                digest: fingerprint(&json!({"scope":contract,"choice":validity}))?,
                validity,
            })
        })
        .collect::<AppResult<Vec<_>>>()?;
    let p = ProposalV2 {
        background_allowed,
        waiter_until: (!background_allowed).then_some(end),
        source,
        locator,
        source_digest,
        revision: 1,
        gap_key: gap_key.clone(),
        effect_id: context.operation_id.clone(),
        prepared_digest: digest,
        authority_digest,
        key_id: key,
        conversation_id: chat.map(|c| c.conversation_id.clone()),
        source_turn_id: chat.and_then(|c| c.turn_id.clone()),
        source_credential_id: context.source_credential.get().cloned(),
        endpoint_digest,
        selected_choice_digest: None,
        notification_claimed_at: None,
        wake_at: None,
        maintenance_at: None,
        delivery_abandoned: false,
        independent_request_id: None,
        independent_card_id: None,
        delivered_turn_id: None,
        task: task.clone(),
        report_to: chat.and_then(|c| c.report_to.clone()),
        invocation_id: chat.and_then(|c| c.invocation_id.clone()),
        contract,
        choices,
        review: review_v2(db, context, chat, effect, reusable, &task, gaps).await?,
        payload: Some(binary(encrypted)),
        state: "pending".into(),
        grant_id: None,
        selected_choice: None,
        claim_id: None,
        resume_turn_id: None,
        claimed_at: None,
        result: None,
        failure_code: None,
    };
    let group_binding = if let Some(chat) = chat {
        crate::services::org_group_service::approval_binding(
            db,
            chat.execution_group_id.as_deref(),
            &chat.user_id,
            Some(&chat.api_key_id),
        )
        .await?
    } else {
        None
    };
    let request = ApprovalRequest {
        execution_proposal: Some(ExecutionProposal {
            version: 2,
            v1: None,
            v2: Some(p),
        }),
        assistant_group: group_binding,
        id,
        user_id: context.actor.clone(),
        service_id: effect.target.service.id.clone(),
        service_name: label(&effect.target.service.name),
        service_slug: label(&effect.target.service.slug),
        requester_type: "api_key".into(),
        requester_id: key_row.id.clone(),
        requester_label: Some(label(&key_row.name)),
        operation_summary: format!("{} {}", effect.method, label(effect.path)),
        action_description: Some("Review an execution permission".into()),
        http_method: Some(effect.method.into()),
        resource: Some(label(effect.path)),
        verb: None,
        grant_scope: Some(LEGACY_SCOPE.into()),
        tool_name: None,
        tool_call_id: None,
        tool_arguments: None,
        is_destructive: None,
        approval_mode: crate::models::service_approval_config::ApprovalMode::PerRequest,
        status: "pending".into(),
        idempotency_key: format!("execution:v2:{gap_key}"),
        notification_channel: None,
        telegram_message_id: None,
        telegram_chat_id: None,
        expires_at: end,
        decided_at: None,
        decision_channel: None,
        decision_idempotency_key: None,
        notify_user_ids: vec![context.actor.clone()],
        from_org_policy: false,
        exact_service: None,
        created_at: now,
    };
    if !background_allowed {
        let _ = context.wait_guard.set(Arc::new(blocking::WaitGuard::new(
            db.clone(),
            request.id.clone(),
            context.cancellation.get().cloned().unwrap_or_default(),
        )));
    }
    let mut session = db.client().start_session().await?;
    let committed = session.start_transaction().and_run((db,context,chat,&request,snapshot),|session,(db,context,chat,request,snapshot)|Box::pin(async move {
        let result:AppResult<ApprovalRequest>=async {
            require_enrolled(db,&context.actor,Some(session)).await?;
            let p=proposal(request)?;
            fence_source(db, session, request).await?;
            if let Some(chat) = chat.filter(|_| context.descendant.is_none()) {
                let (row,_,_)=Box::pin(acks::fence(db,chat,session)).await?;
                super::super::super::assistant_access_policy::require_same_live_turn(chat,&row)?;
            }
            if let Some(source) = &context.descendant { source.check(db,Some(session)).await?; }
            snapshot.fence(db,session).await?;
            let collection=db.collection::<ApprovalRequest>(REQUESTS);
            if let Some(existing)=collection.find_one(doc!{"execution_proposal.v2.locator":&p.locator}).session(&mut *session).await? {
                let previous = proposal(&existing)?;
                if previous.gap_key != p.gap_key { return Err(conflict()); }
                return Ok(existing);
            }
            if collection.count_documents(doc!{"user_id":&context.actor,"execution_proposal.version":2,"execution_proposal.v2.state":{"$in":["pending","queued","preparing","waiting"]}}).session(&mut *session).await?>=32 {return Err(AppError::Forbidden("Too many pending permission requests".into()));}
            collection.insert_one(*request).session(&mut *session).await?;
            if let ProposalSource::Watch{watch_id,operation,..}=&p.source
                && db.collection::<Document>("assistant_async_operations").update_one(doc!{"_id":watch_id,"execution_pending":null},doc!{"$set":{"execution_pending":{"request_id":&request.id,"operation":operation}}}).session(&mut *session).await?.matched_count!=1{return Err(conflict());}
            audit(db,session,&context.actor,"execution_permission_requested",&request.id).await?;
            Ok((*request).clone())
        }.await;
        crate::services::api_key_mutation_service::transaction_result(result)
    })).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    // Send before the connected waiter starts polling. Failure cannot roll back
    // the committed review; the sweep can claim an unclaimed notification.
    if delivery::notify_once(state, &committed).await.is_err() {
        tracing::warn!(
            reason = "execution_notification_failed",
            "Execution permission notification failed"
        );
    }
    Ok(Some(committed))
}

#[cfg(test)]
pub(crate) use delivery::execute;

#[cfg(test)]
pub(crate) async fn claim_for_test(
    state: &crate::AppState,
    row: &ApprovalRequest,
) -> AppResult<Option<(Resume, PreparedAdapter)>> {
    delivery::claim(state, row).await
}
