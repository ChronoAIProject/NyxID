//! Turn-fenced facts from the engine's HTTP model path. MCP inference tools use
//! their own transport and never enter this observer.
use std::sync::{
    Arc, OnceLock,
    atomic::{AtomicBool, AtomicU8, AtomicU16, Ordering},
};

use chrono::{DateTime, Utc};
use mongodb::{
    Database,
    bson::{self, doc},
};
use uuid::Uuid;

use super::assistant_nyxagent::{self as engine, TurnError};
use crate::{
    errors::{AppError, AppResult},
    models::assistant_conversation::{ActiveTurn, AssistantConversation, COLLECTION_NAME},
};

use super::assistant_engine_policy::MODEL_ATTRIBUTION_WINDOW;

#[derive(Clone)]
pub struct ModelRequest {
    db: Database,
    conversation: String,
    key: String,
    turn: String,
    attempt: String,
    completion: Arc<Completion>,
    setup: bool,
    /// Guests/shared collaboration retain their existing refusal presentation.
    approval_interactive: Option<bool>,
    presentation_loaded: bool,
}

struct PolicyScope {
    actor: String,
    catalog: String,
    stateless: bool,
}

struct Completion {
    started: std::time::Instant,
    db: Database,
    conversation: String,
    turn: String,
    done: AtomicBool,
    egress_admitted: AtomicBool,
    response_status: AtomicU16,
    approval_exempt: AtomicBool,
    policy_failure: AtomicU8,
    policy_scope: OnceLock<PolicyScope>,
}

impl Completion {
    async fn finish(&self) -> AppResult<()> {
        if self.done.swap(true, Ordering::SeqCst) {
            return Ok(());
        }
        complete_request(
            &self.db,
            &self.conversation,
            &self.turn,
            self.started.elapsed(),
        )
        .await?;
        Ok(())
    }
}

async fn complete_request(
    db: &Database,
    conversation: &str,
    turn: &str,
    elapsed: std::time::Duration,
) -> AppResult<()> {
    db.collection::<bson::Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id": conversation, "active_turn.turn_id": turn},
            vec![
                doc! {"$set": {"active_turn.timing.model_ms":{"$add":[{"$ifNull":["$active_turn.timing.model_ms",0]}, elapsed.as_millis().min(i64::MAX as u128) as i64]}, "active_turn.model_requests_active": {"$max": [0, {"$subtract": [
                    {"$ifNull": ["$active_turn.model_requests_active", 0]}, 1
                ]}]}}},
            ],
        )
        .await?;
    Ok(())
}

impl Drop for Completion {
    fn drop(&mut self) {
        if !self.done.load(Ordering::SeqCst) {
            let db = self.db.clone();
            let conversation = self.conversation.clone();
            let turn = self.turn.clone();
            let elapsed = self.started.elapsed();
            tokio::spawn(async move {
                let _ = complete_request(&db, &conversation, &turn, elapsed).await;
            });
        }
    }
}

impl ModelRequest {
    /// LLM ingress already authenticated the live conversation snapshot. Avoid
    /// a second conversation read on ordinary approval-free model traffic.
    pub(crate) fn from_auth(db: &Database, auth: &crate::mw::auth::AuthUser) -> Option<Self> {
        let chat = auth.assistant_chat.as_deref()?;
        let key = auth.api_key_id.as_deref()?;
        let turn = chat.turn_id.as_deref()?;
        if !chat.turn_live || chat.turn_stopped || chat.api_key_id != key {
            return None;
        }
        Some(Self {
            db: db.clone(),
            conversation: chat.conversation_id.clone(),
            key: key.into(),
            turn: turn.into(),
            attempt: Uuid::new_v4().to_string(),
            setup: false,
            approval_interactive: (!chat.guest
                && !chat.collaboration_thread
                && !chat
                    .org_agent_access
                    .as_ref()
                    .is_some_and(|a| a.shared().is_some()))
            .then_some(false),
            presentation_loaded: false,
            completion: Arc::new(Completion {
                started: std::time::Instant::now(),
                db: db.clone(),
                conversation: chat.conversation_id.clone(),
                turn: turn.into(),
                // This route previously had no model timing observer; preserve it.
                done: AtomicBool::new(true),
                egress_admitted: AtomicBool::new(false),
                response_status: AtomicU16::new(0),
                approval_exempt: AtomicBool::new(false),
                policy_failure: AtomicU8::new(0),
                policy_scope: OnceLock::new(),
            }),
        })
    }

    async fn interactive(&self) -> AppResult<Option<bool>> {
        if self.presentation_loaded || self.approval_interactive.is_none() {
            return Ok(self.approval_interactive);
        }
        // Only a real approval requirement on an ineligible shape needs to
        // resolve presentation. No default/exempt path reaches this read.
        let row = self.db.collection::<AssistantConversation>(COLLECTION_NAME).find_one(
            doc! {"_id":&self.conversation,"credential_api_key_id":&self.key,"active_turn.turn_id":&self.turn,"active_turn.stop_requested":false}
        ).await?.ok_or(AppError::AssistantTurnRequired)?;
        Ok(Some(
            row.group_id.is_none()
                && row.channel.is_none()
                && row.active_turn.as_ref().is_some_and(|t| {
                    t.origin == crate::models::assistant_conversation::TurnOrigin::User
                        && t.channel_adapter_origin.is_none()
                        && t.platform_origin.is_none()
                }),
        ))
    }

    pub async fn capture(
        db: &Database,
        conversation: &str,
        key: &str,
        turn: &str,
    ) -> AppResult<Option<Self>> {
        let row = db
            .collection::<AssistantConversation>(COLLECTION_NAME)
            .find_one(doc! {"_id": conversation, "credential_api_key_id": key, "active_turn.turn_id": turn})
            .await?;
        if row
            .as_ref()
            .and_then(|r| engine::live_turn(r, Utc::now()))
            .is_some_and(|turn| approval_error(turn).is_some())
        {
            return Err(AppError::ApprovalUnsupported);
        }
        let approval_interactive = row
            .as_ref()
            .filter(|row| {
                !row.guest_turn && !row.shared_agent_thread && row.collaboration.is_none()
            })
            .map(|row| {
                row.group_id.is_none()
                    && row.channel.is_none()
                    && row.active_turn.as_ref().is_some_and(|t| {
                        t.origin == crate::models::assistant_conversation::TurnOrigin::User
                            && t.channel_adapter_origin.is_none()
                            && t.platform_origin.is_none()
                    })
            });
        let captured = row
            .as_ref()
            .and_then(|row| engine::live_turn(row, Utc::now()))
            .filter(|turn| !turn.stop_requested)
            .map(|turn| Self {
                db: db.clone(),
                approval_interactive,
                presentation_loaded: true,
                setup: row.as_ref().is_some_and(|r| r.guest_model_setup),
                conversation: conversation.into(),
                key: key.into(),
                turn: turn.turn_id.clone(),
                attempt: Uuid::new_v4().to_string(),
                completion: Arc::new(Completion {
                    started: std::time::Instant::now(),
                    db: db.clone(),
                    conversation: conversation.into(),
                    turn: turn.turn_id.clone(),
                    done: AtomicBool::new(true),
                    egress_admitted: AtomicBool::new(false),
                    response_status: AtomicU16::new(0),
                    approval_exempt: AtomicBool::new(false),
                    policy_failure: AtomicU8::new(0),
                    policy_scope: OnceLock::new(),
                }),
            });
        if let Some(request) = &captured {
            let matched = db
                .collection::<bson::Document>(COLLECTION_NAME)
                .update_one(
                    { let mut f = request.turn_fence(); f.insert("active_turn.model_refusal.code", doc! {"$nin": POLICY_CODES}); f },
                    vec![doc! {"$set": {
                        "active_turn.model_attempt_id": &request.attempt,
                        "active_turn.timing.model_calls":{"$add":[{"$ifNull":["$active_turn.timing.model_calls",0]},1]},
                        "active_turn.model_attribution_uncertain": {"$or": [
                            {"$ifNull": ["$active_turn.model_attribution_uncertain", false]},
                            {"$gt": [{"$ifNull": ["$active_turn.model_requests_active", 0]}, 0]}
                        ]},
                        "active_turn.model_requests_active": {"$add": [
                            {"$ifNull": ["$active_turn.model_requests_active", 0]}, 1
                        ]}
                    }}],
                )
                .await?
                .matched_count;
            if matched == 0 {
                return Err(AppError::AssistantTurnRequired);
            }
            request.completion.done.store(false, Ordering::SeqCst);
        }
        Ok(captured)
    }

    fn turn_fence(&self) -> bson::Document {
        doc! {"_id": &self.conversation, "active_turn.turn_id": &self.turn,
        "active_turn.stop_requested": false}
    }

    fn fence(&self) -> bson::Document {
        let mut fence = self.turn_fence();
        fence.insert("active_turn.model_attempt_id", &self.attempt);
        fence
    }

    fn nonterminal_fence(&self) -> bson::Document {
        let mut fence = self.fence();
        fence.insert(
            "active_turn.model_refusal.code",
            doc! {"$nin": POLICY_CODES},
        );
        fence
    }

    pub async fn refusal(&self, error: &AppError) -> AppResult<()> {
        if refusal_code(error).is_none() {
            return self.complete().await;
        }
        self.rejected(error).await
    }

    /// Only stable AppError metadata, never Display/debug strings or bodies.
    /// Called around the whole model proxy so pre-billing refusals are visible.
    pub async fn rejected(&self, error: &AppError) -> AppResult<()> {
        self.db
            .collection::<bson::Document>(COLLECTION_NAME)
            .update_one(
                self.nonterminal_fence(),
                doc! {"$set": {"active_turn.model_refusal": {
                    "code": error.error_key(), "http_status": i32::from(error.status_code().as_u16()),
                    "error_code": i64::from(error.error_code()),
                    "attempt_id": &self.attempt, "at": bson::DateTime::now()
                }}},
            )
            .await?;
        self.complete().await
    }

    pub async fn first_output(&self) -> AppResult<()> {
        let mut filter = self.turn_fence();
        filter.insert("active_turn.timing.first_output_ms", bson::Bson::Null);
        self.db.collection::<bson::Document>(COLLECTION_NAME).update_one(filter,
            vec![doc! {"$set":{"active_turn.timing.first_output_ms":{"$max":[0,{"$subtract":["$$NOW","$active_turn.started_at"]}]}}}]).await?;
        Ok(())
    }

    pub async fn complete(&self) -> AppResult<()> {
        self.completion.finish().await
    }

    pub(crate) fn egress_admitted(&self) -> bool {
        self.completion.egress_admitted.load(Ordering::SeqCst)
    }

    pub(crate) fn received_headers(&self, status: u16) {
        self.completion
            .response_status
            .store(status, Ordering::SeqCst);
    }

    pub(crate) fn response_status(&self) -> Option<u16> {
        match self.completion.response_status.load(Ordering::SeqCst) {
            0 => None,
            status => Some(status),
        }
    }

    /// In-memory request scope; never serialized or accepted from callers.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn configure_policy(
        &self,
        auth: &crate::mw::auth::AuthUser,
        service: &crate::models::downstream_service::DownstreamService,
        method: &str,
        path: &str,
        query: Option<&str>,
        headers: &axum::http::HeaderMap,
        body: &[u8],
        pool: Option<&str>,
    ) {
        if service.inference.is_some() {
            let _ = self.completion.policy_scope.set(PolicyScope {
                actor: auth.effective_approval_owner_user_id(),
                catalog: service.id.clone(),
                stateless: super::assistant_service_execution::assistant_model_transport(
                    auth, service, method, path, query, headers, body, pool,
                ),
            });
        }
    }

    pub(crate) fn approval_exempt(&self) -> bool {
        self.completion.approval_exempt.load(Ordering::SeqCst)
    }
    pub(crate) fn policy_failure(&self) -> Option<&'static str> {
        match self.completion.policy_failure.load(Ordering::SeqCst) {
            1 => Some(MODEL_APPROVAL_REQUIRED),
            2 => Some("model_policy_denied"),
            3 => Some("model_org_approval_required"),
            _ => None,
        }
    }

    pub(crate) async fn block_policy(&self, code: &str, error: &AppError) -> AppResult<bool> {
        let _ = self.completion.policy_failure.compare_exchange(
            0,
            match code {
                "model_policy_denied" => 2,
                "model_org_approval_required" => 3,
                _ => 1,
            },
            Ordering::SeqCst,
            Ordering::SeqCst,
        );
        // Guest/shared user-facing refusal behavior remains unchanged.
        if self.approval_interactive.is_none() {
            return Ok(false);
        }
        let mut filter = self.turn_fence();
        filter.insert("credential_api_key_id", &self.key);
        filter.insert(
            "active_turn.model_refusal.code",
            doc! {"$nin": POLICY_CODES},
        );
        let changed = self.db.collection::<bson::Document>(COLLECTION_NAME).update_one(filter,
            doc! {"$set":{"active_turn.model_refusal":{
                "code":code,"http_status":i32::from(error.status_code().as_u16()),
                "error_code":i64::from(error.error_code()),"attempt_id":&self.attempt,"at":bson::DateTime::now()
            }}}).await?.matched_count;
        Ok(changed == 1)
    }

    pub(crate) async fn approval_refused(
        &self,
        error: &AppError,
        _frontend: &str,
    ) -> AppResult<()> {
        if approval_failure_code(error).is_some() {
            self.block_policy(MODEL_APPROVAL_REQUIRED, error).await?;
        }
        Ok(())
    }

    async fn before_approval_request(&self, _frontend: &str, proposal: bool) -> AppResult<()> {
        let Some(interactive) = self.interactive().await? else {
            return Ok(());
        };
        if !interactive || proposal {
            let first = self
                .block_policy(MODEL_APPROVAL_REQUIRED, &AppError::ApprovalUnsupported)
                .await?;
            if !interactive || !first {
                return Err(AppError::ApprovalUnsupported);
            }
        }
        Ok(())
    }

    pub(super) fn is_setup(&self) -> bool {
        self.setup
    }

    pub async fn accepted(&self) -> AppResult<()> {
        if self.setup {
            self.db.collection::<bson::Document>(super::assistant_service_execution::COLLECTION).update_one(
                doc!{"kind":super::assistant_guest_model_setup::KIND,"conversation_id":&self.conversation,"status":"setting_up","until":{"$gt":bson::DateTime::now()}},
                doc!{"$set":{"model_accepted":true}}).await?;
        }
        let mut fence = self.nonterminal_fence();
        fence.insert("active_turn.model_refusal", doc! {"$ne": null});
        self.db
            .collection::<bson::Document>(COLLECTION_NAME)
            .update_one(fence, doc! {"$unset": {"active_turn.model_refusal": ""}})
            .await?;
        Ok(())
    }

    pub(crate) async fn dispatched(&self) -> AppResult<()> {
        self.completion
            .egress_admitted
            .store(true, Ordering::SeqCst);
        if self.setup {
            super::assistant_guest_model_setup::backoff::dispatched(&self.db, &self.conversation)
                .await?;
        }
        Ok(())
    }

    pub async fn fault(&self, kind: &str, attributable: bool) -> AppResult<()> {
        let mut fence = self.fence();
        // Stop is still a live captured turn; its cancellation is diagnostic.
        fence.remove("active_turn.stop_requested");
        let field = if kind == "consumer_cancelled" {
            "active_turn.model_stream_cancellation"
        } else {
            "active_turn.model_stream_fault"
        };
        self.db.collection::<bson::Document>(COLLECTION_NAME).update_one(fence,
            doc! {"$set": {field: {
                "kind": kind, "attempt_id": &self.attempt, "at": bson::DateTime::now(),
                "attributable": attributable && kind != "consumer_cancelled" && kind != "unknown"
            }}}).await?;
        Ok(())
    }
}

// The model observer is installed only by authenticated model proxy ingress.
// General service calls, guests and shared agents retain their existing gates.
tokio::task_local! { pub(crate) static CURRENT_MODEL: Option<ModelRequest>; }

pub(crate) async fn before_approval_request(frontend: &str, proposal: bool) -> AppResult<()> {
    if let Some(request) = CURRENT_MODEL.try_with(Clone::clone).ok().flatten() {
        request.before_approval_request(frontend, proposal).await?;
    }
    Ok(())
}

pub(crate) fn approval_failure_code(error: &AppError) -> Option<&'static str> {
    match error {
        AppError::ApprovalRequired { .. } => Some("approval_required"),
        AppError::ApprovalUnsupported => Some("approval_unsupported"),
        _ => None,
    }
}

pub(crate) const MODEL_APPROVAL_REQUIRED: &str = "model_approval_required";
const POLICY_CODES: &[&str] = &[
    MODEL_APPROVAL_REQUIRED,
    "model_policy_denied",
    "model_org_approval_required",
];
pub(crate) fn is_policy_failure(code: &str) -> bool {
    POLICY_CODES.contains(&code)
}
pub(crate) fn approval_error(turn: &ActiveTurn) -> Option<TurnError> {
    let fact = turn
        .model_refusal
        .as_ref()
        .filter(|f| is_policy_failure(&f.code))?;
    let mut error = TurnError::internal(&fact.code, "model_approval");
    error.message = approval_message(
        &fact.code,
        &super::assistant_links::AssistantPage::ApprovalSettings.path(),
    );
    Some(error)
}

pub(crate) fn approval_message(code: &str, settings_url: &str) -> String {
    let message = match code {
        "model_policy_denied" => {
            "An approval policy denies this model call, so the assistant could not reply. Review the model service's Deny rules in Approval settings; for an organization policy, ask its administrator."
        }
        "model_org_approval_required" => {
            "The organization's policy requires approval for this model service, which this turn cannot complete. Ask an organization administrator to review the model service policy in Approval settings before retrying."
        }
        _ => {
            "Your approval settings require approval for this model request, which this thread cannot present. Review the model service policy and global approval mode in Approval settings before retrying."
        }
    };
    format!("{message} {settings_url}")
}

/// Called after normal policy resolution, using only the data it already read.
/// The scope is installed exclusively on authenticated live model ingress.
pub(crate) fn exempt_personal_requirement(
    actor: &str,
    catalog: &str,
    resolution: &mut super::approval_service::ApprovalResolution,
) {
    use crate::models::service_approval_config::ApprovalEffect;
    if resolution.from_org_policy || resolution.effect != ApprovalEffect::RequireApproval {
        return;
    }
    if let Some(request) = current()
        && request
            .completion
            .policy_scope
            .get()
            .is_some_and(|s| s.stateless && s.actor == actor && s.catalog == catalog)
    {
        resolution.required = false;
        resolution.effect = ApprovalEffect::AutoAllow;
        request
            .completion
            .approval_exempt
            .store(true, Ordering::SeqCst);
    }
}

pub(crate) async fn capture_auth(
    db: &Database,
    auth: &crate::mw::auth::AuthUser,
) -> AppResult<Option<ModelRequest>> {
    let (Some(chat), Some(fence), Some(key)) = (
        auth.assistant_chat.as_deref(),
        auth.assistant_turn_fence.as_ref(),
        auth.api_key_id.as_deref(),
    ) else {
        return Ok(None);
    };
    if !chat.turn_live || chat.api_key_id != key {
        return Ok(None);
    }
    ModelRequest::capture(
        db,
        &fence.conversation_id,
        key,
        fence.turn_id.as_deref().unwrap_or_default(),
    )
    .await
}

/// Preserve the existing pre-materialization Deny, but make it terminal and
/// observable for model callbacks. Extra reads occur only on a proven Deny.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn denied_before_resolution(
    state: &crate::AppState,
    auth: &crate::mw::auth::AuthUser,
    catalog: &str,
    owner: &str,
    connection: Option<&str>,
    method: &str,
    path: &str,
) -> AppResult<()> {
    if auth.assistant_chat.is_none() || method != "POST" || protocol(path) == Protocol::Unknown {
        return Ok(());
    }
    let service = state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .find_one(doc! {"_id":catalog})
        .await?;
    if service.is_none_or(|s| s.inference.is_none()) {
        return Ok(());
    }
    let request = if let Some(request) = current() {
        Some(request)
    } else {
        capture_auth(&state.db, auth).await?
    };
    let Some(request) = request else {
        return Ok(());
    };
    let error = AppError::Forbidden("Operation denied by approval policy".into());
    request.block_policy("model_policy_denied", &error).await?;
    let connection = if let Some(connection) = connection {
        Some(connection.to_string())
    } else {
        state
            .db
            .collection::<bson::Document>("user_services")
            .find_one(doc! {"user_id":owner,"catalog_service_id":catalog,"is_active":true})
            .await?
            .and_then(|r| r.get_str("_id").ok().map(str::to_string))
    };
    super::audit_service::log_for_user(
        state.db.clone(),
        auth,
        "service_request",
        Some(serde_json::json!({
            "user_service_id":connection,"service_id":catalog,"owner_user_id":owner,
            "outcome":"denied","response_status":403,"failure_code":"model_policy_denied",
            "model_egress_admitted":false,"dispatch_state":"not_dispatched",
            "turn_id":request.turn,"execution_id":request.attempt
        })),
    );
    request.complete().await
}

pub(crate) fn stateless_for(actor: &str, catalog: &str) -> bool {
    current().is_some_and(|r| {
        r.completion
            .policy_scope
            .get()
            .is_some_and(|s| s.stateless && s.actor == actor && s.catalog == catalog)
    })
}

pub(crate) fn current() -> Option<ModelRequest> {
    CURRENT_MODEL.try_with(Clone::clone).ok().flatten()
}

pub(crate) async fn evaluated_policy(
    actor: &str,
    catalog: &str,
    outcome: &super::approval_service::ApprovalOutcome,
) -> AppResult<()> {
    use super::approval_service::ApprovalOutcome;
    let Some(request) = current().filter(|r| {
        r.completion
            .policy_scope
            .get()
            .is_some_and(|s| s.actor == actor && s.catalog == catalog)
    }) else {
        return Ok(());
    };
    match outcome {
        ApprovalOutcome::Denied => {
            request
                .block_policy(
                    "model_policy_denied",
                    &AppError::Forbidden("Operation denied by approval policy".into()),
                )
                .await?;
        }
        ApprovalOutcome::NeedsApproval(p) if p.resolution.from_org_policy => {
            request
                .block_policy(
                    "model_org_approval_required",
                    &AppError::ApprovalUnsupported,
                )
                .await?;
            if request.approval_interactive.is_some() {
                return Err(AppError::ApprovalUnsupported);
            }
        }
        // Personal non-stateless requests retain existing Full-access and
        // exact-permission behavior. Only a still-needed approval reaches
        // before_approval_request, which refuses unsupported presentation.
        _ => {}
    }
    Ok(())
}

pub fn refusal_code(error: &AppError) -> Option<&'static str> {
    if matches!(error, AppError::AssistantModelAuthorityChanged) {
        Some("model_authority_changed")
    } else {
        billing_failure_code(error.error_key())
    }
}

pub fn billing_failure_code(code: &str) -> Option<&'static str> {
    match code {
        "insufficient_credits" => Some("insufficient_credits"),
        "wallet_suspended" => Some("wallet_suspended"),
        "plan_entitlement_required" => Some("plan_entitlement_required"),
        "billing_not_configured" => Some("billing_not_configured"),
        "billing_provider_unavailable" => Some("billing_provider_unavailable"),
        _ => None,
    }
}

pub fn attribute(error: &TurnError, turn: &ActiveTurn, now: DateTime<Utc>) -> TurnError {
    if error.code != "cancelled"
        && let Some(blocked) = approval_error(turn)
    {
        return if error.code == blocked.code {
            error.clone()
        } else {
            blocked
        };
    }
    let generic = (error.code == "assistant_unavailable"
        && error.upstream_code.is_some()
        && matches!(error.stage, "stream" | "upstream_response"))
        || (error.code == "invalid_stream" && error.stage == "stream");
    let billing_wrapper =
        error.code == "nyxid_error" && matches!(error.stage, "stream" | "upstream_response");
    if !(generic || billing_wrapper) || turn.model_attribution_uncertain {
        return error.clone();
    }
    let recent = |at: DateTime<Utc>| {
        at <= now
            && now.signed_duration_since(at)
                <= chrono::Duration::from_std(MODEL_ATTRIBUTION_WINDOW)
                    .expect("bounded attribution window")
    };
    let code = turn
        .model_refusal
        .as_ref()
        .filter(|fact| {
            turn.model_attempt_id.as_deref() == Some(&fact.attempt_id) && recent(fact.at)
        })
        .and_then(|fact| {
            (fact.code == "model_authority_changed"
                && fact.http_status == Some(403)
                && fact.error_code == Some(12104))
            .then_some("model_authority_changed")
            .or_else(|| billing_failure_code(&fact.code))
            .or_else(|| {
                (fact.code == "guest_model_route_required"
                    && fact.http_status == Some(403)
                    && fact.error_code == Some(9000))
                .then_some("guest_model_route_required")
            })
        })
        .or_else(|| {
            turn.model_stream_fault
                .as_ref()
                .filter(|fact| {
                    generic
                        && turn.model_attempt_id.as_deref() == Some(&fact.attempt_id)
                        && fact.attributable
                        && recent(fact.at)
                        && matches!(
                            fact.kind.as_str(),
                            "transport_error" | "provider_error" | "missing_terminal"
                        )
                })
                .map(|_| "model_interrupted")
        });
    let Some(code) = code else {
        return error.clone();
    };
    let mut attributed = error.clone();
    let classified = TurnError::new(code);
    attributed.code = classified.code;
    attributed.message = classified.message;
    let reason = turn
        .model_route_diagnostic
        .as_ref()
        .filter(|fact| {
            fact.attempt_id.is_some() && fact.attempt_id == turn.model_attempt_id && recent(fact.at)
        })
        .map(|fact| fact.reason.as_str());
    attributed.with_model_route_reason(reason)
}

/// A recent, unambiguous failure of this turn's exact model attempt explains a
/// generic NyxAgent failure. Success clears it; old/parallel attempts never do.
pub fn failure_reason(
    error: &TurnError,
    turn: &ActiveTurn,
    now: DateTime<Utc>,
) -> Option<Box<crate::models::assistant_message::TurnFailureReason>> {
    use crate::models::assistant_message::TurnFailureReason;
    if is_policy_failure(error.code)
        && let Some(fact) = &turn.model_refusal
    {
        return Some(Box::new(TurnFailureReason {
            source: "nyxid".into(),
            stage: "model_approval".into(),
            code: error.code.into(),
            http_status: fact.http_status,
            nyxid_error_code: fact.error_code,
            model_route: None,
        }));
    }
    if error.stage == "guest_setup" && error.code == "guest_model_route_required" {
        return error
            .metadata
            .as_ref()
            .and_then(|m| m.failure_reason.clone());
    }
    let model_route = turn
        .model_route_diagnostic
        .as_ref()
        .filter(|fact| {
            !turn.model_attribution_uncertain
                && fact.attempt_id.is_some()
                && fact.attempt_id == turn.model_attempt_id
                && fact.at <= now
                && now.signed_duration_since(fact.at)
                    <= chrono::Duration::from_std(MODEL_ATTRIBUTION_WINDOW).expect("bounded window")
        })
        .cloned()
        .map(Box::new);
    if matches!(error.stage, "stream" | "upstream_response")
        && matches!(
            error.upstream_code.as_deref(),
            Some("turn_failed" | "nyxid_error")
        )
        && !turn.model_attribution_uncertain
        && let Some(fact) = turn.model_refusal.as_ref().filter(|fact| {
            turn.model_attempt_id.as_deref() == Some(&fact.attempt_id)
                && fact.at <= now
                && now.signed_duration_since(fact.at)
                    <= chrono::Duration::from_std(MODEL_ATTRIBUTION_WINDOW).expect("bounded window")
                && fact.http_status.is_some()
                && fact.error_code.is_some()
        })
    {
        return Some(Box::new(TurnFailureReason {
            model_route: model_route.clone(),
            source: "nyxid".into(),
            stage: "model_proxy".into(),
            code: fact.code.clone(),
            http_status: fact.http_status,
            nyxid_error_code: fact.error_code,
        }));
    }
    error.upstream_code.as_ref().map(|code| {
        Box::new(TurnFailureReason {
            model_route: model_route.clone(),
            source: "nyxagent".into(),
            stage: error.stage.into(),
            code: code.to_string(),
            http_status: error.http_status,
            nyxid_error_code: None,
        })
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Protocol {
    Responses,
    ChatCompletions,
    Anthropic,
    Unknown,
}

pub fn protocol(path: &str) -> Protocol {
    let path = path.trim_matches('/');
    if path == "responses" || path.ends_with("/responses") {
        Protocol::Responses
    } else if path == "chat/completions" || path.ends_with("/chat/completions") {
        Protocol::ChatCompletions
    } else if path == "messages" || path.ends_with("/messages") {
        Protocol::Anthropic
    } else {
        Protocol::Unknown
    }
}

pub struct StreamObserver {
    output: bool,
    output_taken: bool,
    protocol: Protocol,
    terminal: bool,
    failed: bool,
    decoder: super::sse_parser::BoundedEventDecoder,
    invalid_framing: bool,
}
impl StreamObserver {
    pub fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            output: false,
            output_taken: false,
            terminal: false,
            failed: false,
            decoder: super::sse_parser::BoundedEventDecoder::with_max_bytes(
                super::assistant_engine_policy::MODEL_STREAM_OBSERVER_MAX_BUFFER_BYTES,
            ),
            invalid_framing: false,
        }
    }
    pub fn push_byte(&mut self, byte: u8) -> Option<super::sse_parser::SseEvent> {
        match self.decoder.push_byte(byte)? {
            Ok(event) => {
                self.event(event.event_type.as_deref(), &event.data);
                Some(event)
            }
            Err(_) => {
                self.invalidate();
                None
            }
        }
    }

    pub fn event(&mut self, event: Option<&str>, data: &str) {
        if self.protocol == Protocol::Unknown || self.invalid_framing {
            return;
        }
        if self.protocol == Protocol::ChatCompletions && data.trim() == "[DONE]" {
            self.terminal = true;
            return;
        }
        if data.is_empty() {
            return;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(data) else {
            self.invalidate();
            return;
        };
        let kind = value["type"].as_str().or(event).unwrap_or_default();
        if kind == "error"
            || value.get("error").is_some_and(|v| !v.is_null())
            || matches!(kind, "response.failed" | "response.incomplete")
        {
            self.failed = true;
        }
        self.output |= match self.protocol {
            Protocol::Responses => {
                matches!(
                    kind,
                    "response.output_text.delta"
                        | "response.function_call_arguments.delta"
                        | "response.reasoning_summary_text.delta"
                ) && value["delta"].as_str().is_some_and(|s| !s.is_empty())
            }
            Protocol::ChatCompletions => value["choices"].as_array().is_some_and(|choices| {
                choices.iter().any(|choice| {
                    choice["delta"]["content"]
                        .as_str()
                        .is_some_and(|s| !s.is_empty())
                        || choice["delta"]["tool_calls"]
                            .as_array()
                            .is_some_and(|calls| !calls.is_empty())
                })
            }),
            Protocol::Anthropic => {
                kind == "content_block_delta"
                    && ["text", "partial_json", "thinking"]
                        .iter()
                        .any(|key| value["delta"][*key].as_str().is_some_and(|s| !s.is_empty()))
            }
            Protocol::Unknown => false,
        };
        self.terminal |= match self.protocol {
            Protocol::Responses => matches!(
                kind,
                "response.completed" | "response.failed" | "response.incomplete"
            ),
            Protocol::ChatCompletions => data.trim() == "[DONE]",
            Protocol::Anthropic => kind == "message_stop",
            Protocol::Unknown => false,
        };
    }
    pub fn take_first_output(&mut self) -> bool {
        if !self.output || self.output_taken || self.invalid_framing {
            return false;
        }
        self.output_taken = true;
        true
    }
    pub fn provider_failed(&self) -> bool {
        self.failed && !self.invalid_framing
    }
    pub fn invalidate(&mut self) {
        self.invalid_framing = true;
    }
    pub fn framing_invalid(&self) -> bool {
        self.invalid_framing
    }
    pub fn transport_fault(&self) -> (&'static str, bool) {
        if self.invalid_framing {
            ("unknown", false)
        } else {
            ("transport_error", true)
        }
    }
    pub fn eof(&self) -> Option<&'static str> {
        if self.invalid_framing || self.decoder.has_incomplete_event() {
            Some("unknown")
        } else if self.failed {
            Some("provider_error")
        } else if self.protocol == Protocol::Unknown {
            Some("unknown")
        } else if !self.terminal {
            Some("missing_terminal")
        } else {
            None
        }
    }
}

#[cfg(test)]
#[test]
fn model_2142_authority_refusal_is_actionable_and_never_replaces_identity_or_retries() {
    use super::assistant_nyxagent::{Recovery, RecoveryAction};
    let error = AppError::AssistantModelAuthorityChanged;
    assert_eq!(error.status_code().as_u16(), 403);
    assert_eq!(refusal_code(&error), Some("model_authority_changed"));
    let classified = TurnError::new(error.error_key());
    assert_eq!(classified.code, "model_authority_changed");
    assert!(classified.message.contains("Review the completed actions"));
    let mut recovery = Recovery::default();
    assert_eq!(
        recovery.decide(403, error.error_key(), true),
        RecoveryAction::Fail
    );
    assert!(!recovery.replaced && !recovery.rebound && recovery.retries == 0);
}

#[cfg(test)]
#[path = "assistant_model_approval_tests.rs"]
mod approval_tests;
