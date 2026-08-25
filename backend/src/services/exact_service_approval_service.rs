use chrono::Utc;
use mongodb::Database;
use mongodb::bson::{self, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::approval_request::{
    ApprovalRequest, COLLECTION_NAME as REQUESTS, ExactServiceApprovalBinding,
    ExactServiceApprovalReceipt, ExactServiceApprovalRedemption,
    ExactServiceExecutionAuthorityBinding, ExactServiceRedemptionStatus,
};
use crate::models::service_approval_config::ApprovalMode;
use crate::services::billing::route_inventory::BillingEgressPermit;
use crate::services::{
    approval_service, execution_authority, mcp_service, node_routing_service, notification_service,
    proxy_service,
};

const MAX_RECEIPT_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
const EXECUTION_EFFECT_TIMEOUT_SECS: u64 = 10 * 60;
/// A retry must not classify a still-running provider request as abandoned.
/// Exact buffered execution is bounded below this interval. Once it elapses,
/// the outcome is terminally unknown and is never replayed automatically
/// because the provider effect may have succeeded.
const EXECUTION_OUTCOME_RECOVERY_GRACE_SECS: i64 = 15 * 60;
const PROVIDER_OUTCOME_UNKNOWN: &str = "provider_outcome_unknown";
pub const DELEGATED_REQUESTER_TYPE: &str = "delegated";
pub const EXACT_VIEW_DIGEST_REQUIRED: &str = "exact_view_digest_required";
pub const DELEGATED_CATALOG_SCOPE_REQUIRED: &str = "delegated_catalog_scope_required";

#[derive(Clone, Debug)]
pub struct ExactServiceApprovalCaller {
    pub actor_user_id: String,
    pub proxy_resolution_user_id: String,
    pub approval_owner_user_id: String,
    pub requester_type: String,
    pub requester_id: String,
    pub requester_label: Option<String>,
    pub api_key_id: Option<String>,
    pub has_catalog_read: bool,
    pub allow_all_services: bool,
    pub allowed_service_ids: Vec<String>,
    pub allow_all_nodes: bool,
    pub allowed_node_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExactServiceApprovalCreate {
    pub user_service_id: String,
    pub endpoint_id: String,
    pub catalog_digest: String,
    #[serde(default)]
    pub exact_view_digest: Option<String>,
    pub endpoint_contract_digest: String,
    pub operation_digest: String,
    pub operation_id: String,
    /// Optional discovery generation echoed by the caller. NyxID always uses
    /// the live producer value when one exists; this field is informational at
    /// create because the digest and endpoint-contract fences prove freshness.
    #[serde(default)]
    pub operation_generation: Option<i64>,
    pub idempotency_key: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExactServiceApprovalFence {
    pub catalog_digest: String,
    #[serde(default)]
    pub exact_view_digest: Option<String>,
    pub operation_digest: String,
    pub operation_id: String,
    pub operation_generation: i64,
    pub idempotency_key: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ExactServiceApprovalState {
    Pending,
    Approved,
    Denied,
    Expired,
    Revoked,
    Drifted,
    Redeeming,
    Redeemed,
    Failed,
}

#[derive(Clone, Debug, Serialize)]
pub struct ExactServiceApprovalResult {
    pub request_id: String,
    pub state: ExactServiceApprovalState,
    pub user_service_id: String,
    pub endpoint_id: String,
    pub catalog_digest: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exact_view_digest: Option<String>,
    pub endpoint_contract_digest: String,
    pub operation_digest: String,
    pub operation_id: String,
    pub operation_generation: i64,
    pub idempotency_key: String,
    pub expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub receipt: Option<ExactServiceApprovalReceipt>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_code: Option<String>,
}

struct ExactCatalogResolution<'a> {
    catalog: mcp_service::McpOperationCatalog,
    service_index: usize,
    endpoint_index: usize,
    catalog_digest: String,
    exact_view_digest: String,
    legacy_exact_view_digest: String,
    in_exact_view: bool,
    endpoint_contract_digest: String,
    operation_digest: String,
    operation_generation: i64,
    producer_generation_bound: bool,
    marker: std::marker::PhantomData<&'a ()>,
}

impl ExactCatalogResolution<'_> {
    fn service(&self) -> &mcp_service::McpToolService {
        &self.catalog.services[self.service_index]
    }

    fn endpoint(&self) -> &mcp_service::McpToolEndpoint {
        &self.service().endpoints[self.endpoint_index]
    }
}

pub async fn create_request(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    input: ExactServiceApprovalCreate,
) -> AppResult<ExactServiceApprovalResult> {
    // Scope rejection is deliberately before validation and catalog resolution.
    // The digest requirement itself is checked after exact-view membership so
    // delegated out-of-view targets preserve their established NotFound error.
    ensure_delegated_exact_authority(caller, input.exact_view_digest.as_deref(), false)?;
    validate_create(&input)?;
    let resolution = resolve_exact_catalog(
        state,
        caller,
        &input.user_service_id,
        &input.endpoint_id,
        &input.arguments,
    )
    .await?;
    let producer_operation_id = resolution.endpoint().endpoint_id.clone();
    if input.operation_id != producer_operation_id {
        return Err(AppError::Conflict(
            "exact_service_operation_id_drift".to_string(),
        ));
    }
    ensure_digest(
        "catalog_digest",
        &input.catalog_digest,
        &resolution.catalog_digest,
    )?;
    if resolution.in_exact_view {
        ensure_delegated_exact_authority(caller, input.exact_view_digest.as_deref(), true)?;
        if let Some(exact_view_digest) = input.exact_view_digest.as_deref() {
            ensure_exact_view_digest(exact_view_digest, &resolution)?;
        }
    } else if input.exact_view_digest.is_some() {
        return Err(AppError::BadRequest(
            "exact_view_digest_not_applicable".to_string(),
        ));
    }
    ensure_digest(
        "endpoint_contract_digest",
        &input.endpoint_contract_digest,
        &resolution.endpoint_contract_digest,
    )?;
    ensure_digest(
        "operation_digest",
        &input.operation_digest,
        &resolution.operation_digest,
    )?;
    if input
        .operation_generation
        .is_some_and(|provided| provided != resolution.operation_generation)
    {
        tracing::debug!(
            provided_operation_generation = input.operation_generation,
            producer_operation_generation = resolution.operation_generation,
            producer_generation_bound = resolution.producer_generation_bound,
            "Ignoring caller operation_generation mismatch at exact-approval create"
        );
    }
    let request_key = request_key(
        caller,
        &producer_operation_id,
        resolution.operation_generation,
        &input.idempotency_key,
    );
    reject_legacy_request_replay(
        state,
        caller,
        &producer_operation_id,
        &input.idempotency_key,
    )
    .await?;

    let service = resolution.service();
    let endpoint = resolution.endpoint();
    let provider_idempotency_key = service
        .durable_endpoint_metadata
        .get(&endpoint.endpoint_id)
        .filter(|metadata| metadata.supports_idempotency_key)
        .map(|_| input.idempotency_key.as_str());
    let descriptor = mcp_service::prepare_exact_proxy_tool_call(
        service,
        endpoint,
        &input.arguments,
        provider_idempotency_key,
    )?
    .operation_descriptor();
    let approval_target = approval_target(state, caller, service).await?;
    let pending = match approval_service::evaluate_and_check(
        &state.db,
        &caller.approval_owner_user_id,
        &approval_target.service_owner_user_id,
        &approval_target.service_id,
        &descriptor,
        Some(&caller.requester_type),
        &caller.requester_id,
        false,
        approval_target.is_auto_connected,
    )
    .await?
    {
        approval_service::ApprovalOutcome::NeedsApproval(pending)
            if pending.resolution.mode == ApprovalMode::PerRequest =>
        {
            pending
        }
        approval_service::ApprovalOutcome::NeedsApproval(_) => {
            return Err(AppError::Conflict(
                "exact_service_requires_per_request_mode".to_string(),
            ));
        }
        approval_service::ApprovalOutcome::Denied => {
            return Err(AppError::Forbidden(
                "exact_service_operation_denied".to_string(),
            ));
        }
        approval_service::ApprovalOutcome::Allowed { .. } => {
            return Err(AppError::Conflict(
                "exact_service_approval_not_required".to_string(),
            ));
        }
    };

    let notify_user_ids = approval_service::approval_notification_recipients(
        &state.db,
        &caller.approval_owner_user_id,
        &pending,
    )
    .await?;
    let timeout_recipient = notify_user_ids
        .first()
        .ok_or_else(|| AppError::Internal("approval recipient list is empty".to_string()))?;
    let channel = notification_service::get_or_create_channel(&state.db, timeout_recipient).await?;
    let execution_authority = resolve_execution_authority(
        state,
        caller,
        &input.user_service_id,
        &service.service_slug,
        ExecutionResolutionMode::ReadOnlySnapshot,
    )
    .await?;
    let binding = ExactServiceApprovalBinding {
        request_key,
        actor_user_id: caller.actor_user_id.clone(),
        user_service_id: input.user_service_id,
        endpoint_id: input.endpoint_id,
        catalog_digest: resolution.catalog_digest,
        // Store the pre-additive v2 digest during the bounded rolling window so
        // a pre-deploy replica can revalidate a row created by this replica.
        exact_view_digest: resolution
            .in_exact_view
            .then(|| resolution.legacy_exact_view_digest.clone()),
        exact_view_digest_binding: resolution
            .in_exact_view
            .then(|| resolution.exact_view_digest.clone()),
        endpoint_contract_digest: resolution.endpoint_contract_digest,
        operation_digest: resolution.operation_digest,
        operation_id: producer_operation_id,
        operation_generation: resolution.operation_generation,
        producer_generation_bound: resolution.producer_generation_bound,
        effect_idempotency_key: input.idempotency_key,
        arguments: input.arguments,
        // Old and new replicas validate the same resolved target through the
        // projection each understands during an ordinary rolling deployment.
        execution_authority_digest: Some(execution_authority.legacy_digest),
        execution_authority_binding: Some(ExactServiceExecutionAuthorityBinding {
            projection_version: execution_authority::CONTRACT_VERSION.to_string(),
            digest: execution_authority.digest,
        }),
        redemption: None,
    };
    let operation = approval_service::ApprovalRequestOperation::from_descriptor(
        &descriptor,
        pending.resolution.grant_scope.clone(),
    )
    .with_exact_service(binding);
    let request = approval_service::create_approval_request(
        &state.db,
        &state.config,
        &state.http_client,
        state.fcm_auth.as_deref(),
        state.apns_auth.as_deref(),
        &pending.primary_owner_user_id,
        &approval_target.service_id,
        &approval_target.service_name,
        &approval_target.service_slug,
        &pending.requester_type,
        &pending.requester_id,
        caller.requester_label.as_deref(),
        operation,
        ApprovalMode::PerRequest,
        channel.approval_timeout_secs,
        notify_user_ids,
        pending.resolution.from_org_policy,
    )
    .await?;

    result_for(&request, ExactServiceApprovalState::Pending)
}

pub async fn observe_request(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    request_id: &str,
) -> AppResult<ExactServiceApprovalResult> {
    ensure_delegated_exact_authority(caller, None, false)?;
    let request = load_bound_request(state, caller, request_id).await?;
    let request = expire_if_needed(state, request).await?;
    let observed = current_state(state, caller, &request).await?;
    let mut result = result_for(&request, observed.state)?;
    if result.failure_code.is_none() {
        result.failure_code = observed.failure_code;
    }
    Ok(result)
}

pub async fn redeem_request(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    request_id: &str,
    fence: ExactServiceApprovalFence,
    billing_egress_permit: BillingEgressPermit,
) -> AppResult<ExactServiceApprovalResult> {
    // Redeem must reject a delegated omission before loading or claiming the
    // request. Unlike create, membership has already been fixed in the stored
    // binding, so this check can run at the service boundary immediately.
    ensure_delegated_exact_authority(caller, fence.exact_view_digest.as_deref(), true)?;
    let request = load_bound_request(state, caller, request_id).await?;
    ensure_fence(&request, &fence)?;
    let request = expire_if_needed(state, request).await?;
    let request = recover_stale_execution(&state.db, request, Utc::now()).await?;
    let observed = persisted_state(&request)?;
    match observed.state {
        ExactServiceApprovalState::Redeemed
        | ExactServiceApprovalState::Redeeming
        | ExactServiceApprovalState::Failed => return result_for(&request, observed.state),
        ExactServiceApprovalState::Approved => {}
        other => {
            let mut result = result_for(&request, other)?;
            if result.failure_code.is_none() {
                result.failure_code = observed.failure_code.or_else(|| match other {
                    ExactServiceApprovalState::Revoked => Some("selector_revoked".to_string()),
                    ExactServiceApprovalState::Drifted => Some("catalog_drift".to_string()),
                    _ => None,
                });
            }
            return Ok(result);
        }
    }

    let now = Utc::now();
    let claimed = claim_redemption(&state.db, &request, now).await?;

    let claimed = match claimed {
        Some(claimed) => claimed,
        None => {
            return reload_after_lost_redemption_claim(state, caller, request_id).await;
        }
    };

    let deadline = execution_deadline(now);
    let post_claim = async {
        let binding = claimed.exact_service.as_ref().unwrap();
        let resolution = match evaluate_live_authority(state, caller, &claimed).await {
            LiveAuthorityEvaluation::Matched(resolution) => resolution,
            LiveAuthorityEvaluation::Terminal(terminal) => {
                let updated = persist_redemption(
                    &state.db,
                    request_id,
                    ExactServiceApprovalRedemption {
                        status: terminal.status,
                        admitted_at: now,
                        completed_at: Some(Utc::now()),
                        receipt: None,
                        failure_code: Some(terminal.failure_code.to_string()),
                    },
                )
                .await?;
                return result_for(&updated, terminal.state);
            }
            LiveAuthorityEvaluation::Error { failure_code, .. } => {
                let updated = persist_redemption(
                    &state.db,
                    request_id,
                    ExactServiceApprovalRedemption {
                        status: ExactServiceRedemptionStatus::Failed,
                        admitted_at: now,
                        completed_at: Some(Utc::now()),
                        receipt: None,
                        failure_code: Some(failure_code.to_string()),
                    },
                )
                .await?;
                return result_for(&updated, ExactServiceApprovalState::Failed);
            }
        };
        // Credential materialization may refresh or mint provider credentials and
        // update usage state, so it runs only after every shared read-only gate.
        // Its digest is compared again to close changes between snapshot and use.
        let execution = match evaluate_execution_authority(
            state,
            caller,
            binding,
            &claimed.service_slug,
            ExecutionResolutionMode::MaterializeForExecution,
        )
        .await
        {
            ExecutionAuthorityEvaluation::Matched(execution) => execution,
            ExecutionAuthorityEvaluation::Terminal(terminal) => {
                let updated = persist_redemption(
                    &state.db,
                    request_id,
                    ExactServiceApprovalRedemption {
                        status: terminal.status,
                        admitted_at: now,
                        completed_at: Some(Utc::now()),
                        receipt: None,
                        failure_code: Some(terminal.failure_code.to_string()),
                    },
                )
                .await?;
                return result_for(&updated, terminal.state);
            }
        };
        let exec_ctx = mcp_service::McpExecContext {
            api_key_id: caller.api_key_id.as_deref(),
            allow_all_nodes: caller.allow_all_nodes,
            allowed_node_ids: &caller.allowed_node_ids,
        };
        let provider_idempotency_key = resolution
            .service()
            .durable_endpoint_metadata
            .get(&resolution.endpoint().endpoint_id)
            .filter(|metadata| metadata.supports_idempotency_key)
            .map(|_| binding.effect_idempotency_key.as_str());
        let prepared = match mcp_service::prepare_exact_proxy_tool_call(
            resolution.service(),
            resolution.endpoint(),
            &binding.arguments,
            provider_idempotency_key,
        ) {
            Ok(prepared) => prepared,
            Err(_) => {
                let updated = persist_redemption(
                    &state.db,
                    request_id,
                    ExactServiceApprovalRedemption {
                        status: ExactServiceRedemptionStatus::Failed,
                        admitted_at: now,
                        completed_at: Some(Utc::now()),
                        receipt: None,
                        failure_code: Some("operation_not_allowed".to_string()),
                    },
                )
                .await?;
                return result_for(&updated, ExactServiceApprovalState::Failed);
            }
        };
        let node_route = match frozen_node_route(state, &execution, &exec_ctx).await {
            Ok(route) => route,
            Err(error) => {
                let updated = persist_redemption(
                    &state.db,
                    request_id,
                    ExactServiceApprovalRedemption {
                        status: ExactServiceRedemptionStatus::Failed,
                        admitted_at: now,
                        completed_at: Some(Utc::now()),
                        receipt: None,
                        failure_code: Some(safe_execution_failure_code(&error).to_string()),
                    },
                )
                .await?;
                return result_for(&updated, ExactServiceApprovalState::Failed);
            }
        };
        let has_cred_for_fallback =
            execution.resolution.has_server_credential && node_route.is_none();
        let billing_context_builder =
            mcp_service::McpBillingRouteContextBuilder::from_user_service_resolution(
                &caller.proxy_resolution_user_id,
                &execution.resolution,
            );
        let executed = mcp_service::execute_tool_resolved(
            &state.http_client,
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &state.billing,
            &caller.proxy_resolution_user_id,
            &caller.proxy_resolution_user_id,
            resolution.service(),
            resolution.endpoint(),
            prepared,
            &state.jwt_keys,
            &state.config,
            &state.connection_expiry_notifier,
            &state.token_exchange_cache,
            &state.cloud_response_cache,
            &exec_ctx,
            billing_egress_permit,
            execution.resolution.target,
            node_route,
            has_cred_for_fallback,
            billing_context_builder,
        )
        .await;

        let completed_at = Utc::now();
        let redemption = match executed {
            Ok(mcp_service::McpToolExecutionOutcome::Response((http_status, response_body))) => {
                let response_digest = format!(
                    "sha256:{}",
                    hex::encode(Sha256::digest(response_body.as_bytes()))
                );
                if response_body.len() > MAX_RECEIPT_RESPONSE_BYTES {
                    ExactServiceApprovalRedemption {
                        status: ExactServiceRedemptionStatus::Failed,
                        admitted_at: now,
                        completed_at: Some(completed_at),
                        receipt: Some(ExactServiceApprovalReceipt {
                            http_status,
                            response_body: String::new(),
                            response_digest,
                        }),
                        failure_code: Some("provider_response_too_large".to_string()),
                    }
                } else {
                    ExactServiceApprovalRedemption {
                        status: ExactServiceRedemptionStatus::Completed,
                        admitted_at: now,
                        completed_at: Some(completed_at),
                        receipt: Some(ExactServiceApprovalReceipt {
                            http_status,
                            response_body,
                            response_digest,
                        }),
                        failure_code: None,
                    }
                }
            }
            Ok(mcp_service::McpToolExecutionOutcome::ProviderOutcomeUnknown(_)) => {
                ExactServiceApprovalRedemption {
                    status: ExactServiceRedemptionStatus::Failed,
                    admitted_at: now,
                    completed_at: Some(completed_at),
                    receipt: None,
                    failure_code: Some(PROVIDER_OUTCOME_UNKNOWN.to_string()),
                }
            }
            Ok(mcp_service::McpToolExecutionOutcome::ProviderUnreachable(_)) => {
                ExactServiceApprovalRedemption {
                    status: ExactServiceRedemptionStatus::Failed,
                    admitted_at: now,
                    completed_at: Some(completed_at),
                    receipt: None,
                    failure_code: Some("provider_unreachable".to_string()),
                }
            }
            Err(error) => ExactServiceApprovalRedemption {
                status: ExactServiceRedemptionStatus::Failed,
                admitted_at: now,
                completed_at: Some(completed_at),
                receipt: None,
                failure_code: Some(safe_execution_failure_code(&error).to_string()),
            },
        };
        let updated = persist_redemption_or_load(state, caller, request_id, redemption).await?;
        let persisted = persisted_state(&updated)?;
        if persisted.state == ExactServiceApprovalState::Redeeming {
            return Err(AppError::Conflict(
                "exact_service_redemption_state_conflict".to_string(),
            ));
        }
        result_for(&updated, persisted.state)
    };

    match tokio::time::timeout_at(deadline, post_claim).await {
        Ok(result) => result,
        Err(_) => {
            let updated = persist_redemption_or_load(
                state,
                caller,
                request_id,
                ExactServiceApprovalRedemption {
                    status: ExactServiceRedemptionStatus::Failed,
                    admitted_at: now,
                    completed_at: Some(Utc::now()),
                    receipt: None,
                    failure_code: Some(PROVIDER_OUTCOME_UNKNOWN.to_string()),
                },
            )
            .await?;
            let persisted = persisted_state(&updated)?;
            if persisted.state == ExactServiceApprovalState::Redeeming {
                return Err(AppError::Conflict(
                    "exact_service_redemption_state_conflict".to_string(),
                ));
            }
            result_for(&updated, persisted.state)
        }
    }
}

fn execution_deadline(admitted_at: chrono::DateTime<Utc>) -> tokio::time::Instant {
    let expires_at = admitted_at + chrono::Duration::seconds(EXECUTION_EFFECT_TIMEOUT_SECS as i64);
    let remaining = (expires_at - Utc::now())
        .to_std()
        .unwrap_or(std::time::Duration::ZERO);
    tokio::time::Instant::now() + remaining
}

async fn claim_redemption(
    db: &Database,
    request: &ApprovalRequest,
    admitted_at: chrono::DateTime<Utc>,
) -> AppResult<Option<ApprovalRequest>> {
    let binding = request
        .exact_service
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("not_an_exact_service_request".to_string()))?;
    Ok(db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one_and_update(
            doc! {
                "_id": &request.id,
                "status": "approved",
                "expires_at": { "$gt": bson::DateTime::from_chrono(admitted_at) },
                "exact_service.request_key": &binding.request_key,
                "exact_service.redemption": { "$exists": false },
            },
            doc! { "$set": {
                "exact_service.redemption": bson::to_bson(&ExactServiceApprovalRedemption {
                    status: ExactServiceRedemptionStatus::Executing,
                    admitted_at,
                    completed_at: None,
                    receipt: None,
                    failure_code: None,
                }).map_err(|error| AppError::Internal(error.to_string()))?,
            }},
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?)
}

async fn reload_after_lost_redemption_claim(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    request_id: &str,
) -> AppResult<ExactServiceApprovalResult> {
    let request = load_bound_request(state, caller, request_id).await?;
    let request = expire_if_needed(state, request).await?;
    let request = recover_stale_execution(&state.db, request, Utc::now()).await?;
    let observed = persisted_state(&request)?;
    let mut result = result_for(&request, observed.state)?;
    if result.failure_code.is_none() {
        result.failure_code = observed.failure_code;
    }
    Ok(result)
}

/// Recover an abandoned provider attempt without replaying it.
///
/// The provider may have committed the effect before the process crashed or
/// MongoDB rejected the terminal receipt write. Because that ambiguity cannot
/// be resolved atomically for arbitrary providers, the only at-most-once-safe
/// recovery is a durable `provider_outcome_unknown` terminal. A human or a
/// provider-specific reconciler may investigate it; the exact approval itself
/// can never dispatch again.
async fn recover_stale_execution(
    db: &Database,
    request: ApprovalRequest,
    now: chrono::DateTime<Utc>,
) -> AppResult<ApprovalRequest> {
    let Some(redemption) = request
        .exact_service
        .as_ref()
        .and_then(|binding| binding.redemption.as_ref())
        .filter(|redemption| redemption.status == ExactServiceRedemptionStatus::Executing)
    else {
        return Ok(request);
    };
    let recovery_at =
        redemption.admitted_at + chrono::Duration::seconds(EXECUTION_OUTCOME_RECOVERY_GRACE_SECS);
    if now < recovery_at {
        return Ok(request);
    }

    let unknown = ExactServiceApprovalRedemption {
        status: ExactServiceRedemptionStatus::Failed,
        admitted_at: redemption.admitted_at,
        completed_at: Some(now),
        receipt: None,
        failure_code: Some(PROVIDER_OUTCOME_UNKNOWN.to_string()),
    };
    let updated = db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one_and_update(
            doc! {
                "_id": &request.id,
                "status": "approved",
                "exact_service.redemption.status": "executing",
                "exact_service.redemption.admitted_at":
                    bson::DateTime::from_chrono(redemption.admitted_at),
            },
            doc! { "$set": {
                "exact_service.redemption": bson::to_bson(&unknown)
                    .map_err(|error| AppError::Internal(error.to_string()))?,
            }},
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    match updated {
        Some(updated) => Ok(updated),
        None => db
            .collection::<ApprovalRequest>(REQUESTS)
            .find_one(doc! { "_id": &request.id })
            .await?
            .ok_or_else(|| AppError::NotFound("exact_service_request_not_found".to_string())),
    }
}

async fn persist_redemption(
    db: &Database,
    request_id: &str,
    redemption: ExactServiceApprovalRedemption,
) -> AppResult<ApprovalRequest> {
    db.collection::<ApprovalRequest>(REQUESTS)
        .find_one_and_update(
            doc! {
                "_id": request_id,
                "exact_service.redemption.status": "executing",
            },
            doc! { "$set": {
                "exact_service.redemption": bson::to_bson(&redemption)
                    .map_err(|error| AppError::Internal(error.to_string()))?,
            }},
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?
        .ok_or_else(|| AppError::Conflict("exact_service_redemption_state_conflict".to_string()))
}

async fn persist_redemption_or_load(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    request_id: &str,
    redemption: ExactServiceApprovalRedemption,
) -> AppResult<ApprovalRequest> {
    match persist_redemption(&state.db, request_id, redemption).await {
        Ok(updated) => Ok(updated),
        Err(AppError::Conflict(message))
            if message == "exact_service_redemption_state_conflict" =>
        {
            // A timeout/recovery or an already-issued terminal write may win
            // the compare-and-set. Replay the durable terminal instead of
            // overwriting it or dispatching the effect again.
            load_bound_request(state, caller, request_id).await
        }
        Err(error) => Err(error),
    }
}

struct ResolvedExecution {
    resolution: proxy_service::UserServiceResolution,
    configured_fallback_node_ids: Vec<String>,
    digest: String,
    legacy_digest: String,
}

#[derive(Clone, Copy)]
enum ExecutionResolutionMode {
    ReadOnlySnapshot,
    MaterializeForExecution,
}

async fn resolve_execution_authority(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    user_service_id: &str,
    service_slug: &str,
    mode: ExecutionResolutionMode,
) -> AppResult<ResolvedExecution> {
    let mut resolution = match mode {
        ExecutionResolutionMode::ReadOnlySnapshot => {
            proxy_service::read_proxy_authority_snapshot_by_user_service_id(
                &state.db,
                &state.encryption_keys,
                &caller.proxy_resolution_user_id,
                user_service_id,
                Some(service_slug),
            )
            .await?
        }
        ExecutionResolutionMode::MaterializeForExecution => {
            proxy_service::resolve_proxy_target_by_user_service_id(
                &state.db,
                &state.encryption_keys,
                &caller.proxy_resolution_user_id,
                user_service_id,
                Some(service_slug),
                None,
                Some(&state.connection_expiry_notifier),
            )
            .await?
        }
    }
    .ok_or_else(|| AppError::NotFound(format!("User service '{service_slug}' not found")))?;

    let mut override_identity = None;
    if let Some(api_key_id) = caller.api_key_id.as_deref() {
        match mode {
            ExecutionResolutionMode::ReadOnlySnapshot => {
                if let Some(identity) = proxy_service::read_agent_credential_override_identity(
                    &state.db,
                    &caller.proxy_resolution_user_id,
                    api_key_id,
                    user_service_id,
                )
                .await?
                {
                    override_identity = Some(execution_authority::OverrideCredentialIdentity {
                        api_key_id: identity.api_key_id,
                        credential_epoch: identity.credential_epoch,
                    });
                }
            }
            ExecutionResolutionMode::MaterializeForExecution => {
                if let Some(override_cred) =
                    proxy_service::resolve_agent_credential_override_identity(
                        &state.db,
                        &state.encryption_keys,
                        &caller.proxy_resolution_user_id,
                        api_key_id,
                        user_service_id,
                        Some(&state.connection_expiry_notifier),
                    )
                    .await?
                {
                    resolution.target.credential = override_cred.credential.clone();
                    override_identity = Some(execution_authority::OverrideCredentialIdentity {
                        api_key_id: override_cred.api_key_id,
                        credential_epoch: override_cred.credential_epoch,
                    });
                }
            }
        }
    }

    let effective_owner = resolution
        .org_routing
        .as_ref()
        .map(|routing| routing.org_user_id.as_str())
        .unwrap_or(caller.proxy_resolution_user_id.as_str());
    let configured_fallback_node_ids = node_routing_service::list_configured_binding_node_ids(
        &state.db,
        effective_owner,
        &resolution.target.service.id,
    )
    .await?;
    let projection = execution_authority::build_projection(
        &resolution,
        override_identity.as_ref(),
        configured_fallback_node_ids.clone(),
    );
    let digest = execution_authority::digest(&projection);
    let legacy_digest = execution_authority::legacy_digest(&projection);
    Ok(ResolvedExecution {
        resolution,
        configured_fallback_node_ids,
        digest,
        legacy_digest,
    })
}

struct ExecutionAuthorityTerminal {
    state: ExactServiceApprovalState,
    status: ExactServiceRedemptionStatus,
    failure_code: &'static str,
}

enum ExecutionAuthorityEvaluation {
    Matched(Box<ResolvedExecution>),
    Terminal(ExecutionAuthorityTerminal),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExecutionAuthorityDigestDecision {
    Matched,
    Drifted,
    UnsupportedVersion,
}

fn execution_authority_terminal(
    state: ExactServiceApprovalState,
    status: ExactServiceRedemptionStatus,
    failure_code: &'static str,
) -> ExecutionAuthorityEvaluation {
    ExecutionAuthorityEvaluation::Terminal(authority_terminal(state, status, failure_code))
}

fn authority_terminal(
    state: ExactServiceApprovalState,
    status: ExactServiceRedemptionStatus,
    failure_code: &'static str,
) -> ExecutionAuthorityTerminal {
    ExecutionAuthorityTerminal {
        state,
        status,
        failure_code,
    }
}

/// Pure stored-vs-live authority decision shared by observe and redeem.
fn evaluate_authority(
    binding: &ExactServiceApprovalBinding,
    live: ResolvedExecution,
) -> ExecutionAuthorityEvaluation {
    match execution_authority_digest_decision(binding, &live.digest, &live.legacy_digest) {
        ExecutionAuthorityDigestDecision::Matched => {
            ExecutionAuthorityEvaluation::Matched(Box::new(live))
        }
        ExecutionAuthorityDigestDecision::Drifted => execution_authority_terminal(
            ExactServiceApprovalState::Drifted,
            ExactServiceRedemptionStatus::Drifted,
            "execution_authority_drift",
        ),
        ExecutionAuthorityDigestDecision::UnsupportedVersion => execution_authority_terminal(
            ExactServiceApprovalState::Drifted,
            ExactServiceRedemptionStatus::Drifted,
            "execution_authority_version_unsupported",
        ),
    }
}

fn execution_authority_digest_decision(
    binding: &ExactServiceApprovalBinding,
    live_digest: &str,
    live_legacy_digest: &str,
) -> ExecutionAuthorityDigestDecision {
    match binding.execution_authority_binding.as_ref() {
        Some(stored) if stored.projection_version == execution_authority::CONTRACT_VERSION => {
            if stored.digest == live_digest {
                ExecutionAuthorityDigestDecision::Matched
            } else {
                ExecutionAuthorityDigestDecision::Drifted
            }
        }
        Some(stored)
            if stored.projection_version == execution_authority::LEGACY_CONTRACT_VERSION =>
        {
            if stored.digest == live_legacy_digest {
                ExecutionAuthorityDigestDecision::Matched
            } else {
                ExecutionAuthorityDigestDecision::Drifted
            }
        }
        Some(_) => ExecutionAuthorityDigestDecision::UnsupportedVersion,
        None => match binding.execution_authority_digest.as_deref() {
            Some(stored) if stored == live_legacy_digest => {
                ExecutionAuthorityDigestDecision::Matched
            }
            Some(_) => ExecutionAuthorityDigestDecision::Drifted,
            // Rows created before execution-authority digests existed retain
            // main's expiry-bounded behavior and skip this one gate.
            None => ExecutionAuthorityDigestDecision::Matched,
        },
    }
}

async fn evaluate_execution_authority(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    binding: &ExactServiceApprovalBinding,
    service_slug: &str,
    mode: ExecutionResolutionMode,
) -> ExecutionAuthorityEvaluation {
    match resolve_execution_authority(state, caller, &binding.user_service_id, service_slug, mode)
        .await
    {
        Ok(live) => evaluate_authority(binding, live),
        Err(error) if catalog_resolution_terminal_state(&error).is_some() => {
            execution_authority_terminal(
                ExactServiceApprovalState::Revoked,
                ExactServiceRedemptionStatus::Revoked,
                "selector_revoked",
            )
        }
        Err(error) => execution_authority_terminal(
            ExactServiceApprovalState::Failed,
            ExactServiceRedemptionStatus::Failed,
            safe_execution_failure_code(&error),
        ),
    }
}

async fn frozen_node_route(
    state: &AppState,
    execution: &ResolvedExecution,
    exec_ctx: &mcp_service::McpExecContext<'_>,
) -> AppResult<Option<node_routing_service::NodeRoute>> {
    let effective_primary = execution
        .resolution
        .node_id
        .as_deref()
        .filter(|node_id| !node_id.is_empty());
    if !exec_ctx.allow_all_nodes
        && let Some(node_id) = effective_primary
        && !exec_ctx.allowed_node_ids.contains(&node_id.to_string())
    {
        return Err(AppError::ApiKeyScopeForbidden(
            "API key does not have access to this node".to_string(),
        ));
    }
    let Some(primary_node_id) = effective_primary else {
        return Ok(None);
    };
    let mut fallback_node_ids = Vec::new();
    for node_id in &execution.configured_fallback_node_ids {
        if node_id == primary_node_id {
            continue;
        }
        if !exec_ctx.allow_all_nodes && !exec_ctx.allowed_node_ids.contains(node_id) {
            continue;
        }
        if node_routing_service::is_node_id_dispatchable(
            &state.db,
            node_id,
            state.node_ws_manager.as_ref(),
        )
        .await?
        {
            fallback_node_ids.push(node_id.clone());
        }
    }
    Ok(Some(node_routing_service::NodeRoute {
        node_id: primary_node_id.to_string(),
        fallback_node_ids,
    }))
}

async fn resolve_exact_catalog(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    user_service_id: &str,
    endpoint_id: &str,
    arguments: &serde_json::Value,
) -> AppResult<ExactCatalogResolution<'static>> {
    let node_scope = if caller.allow_all_nodes {
        mcp_service::NodeScope::Unrestricted
    } else {
        mcp_service::NodeScope::Allowed(caller.allowed_node_ids.as_slice())
    };
    let service_scope = if caller.allow_all_services {
        mcp_service::ServiceScope::Unrestricted
    } else {
        mcp_service::ServiceScope::Allowed(caller.allowed_service_ids.as_slice())
    };
    let catalog = mcp_service::load_operation_catalog(
        &state.db,
        state.node_ws_manager.as_ref(),
        &caller.proxy_resolution_user_id,
        node_scope,
        service_scope,
    )
    .await?;
    let service_index = catalog
        .services
        .iter()
        .position(|service| {
            service.service_id == user_service_id
                && matches!(
                    &service.source,
                    mcp_service::McpToolSource::UserManaged { user_service_id: exact, .. }
                        if exact == user_service_id
                )
        })
        .ok_or_else(|| AppError::NotFound("exact_user_service_not_found".to_string()))?;
    // Delegated callers must be in the exact view. Non-delegated callers may
    // select an out-of-view service. A durable producer generation is bound
    // when available; instance-spec operations deliberately have none.
    let in_exact_view = exact_view_membership(caller, &catalog.services[service_index])?;
    let endpoint_index = catalog.services[service_index]
        .endpoints
        .iter()
        .position(|endpoint| endpoint.endpoint_id == endpoint_id)
        .ok_or_else(|| AppError::NotFound("exact_endpoint_not_found".to_string()))?;
    // Compute both approval fences from their canonical owners. The exact-view
    // projection is shared with delegated discovery; the broad catalog fence
    // stays byte-compatible with `/api/v1/mcp/config`.
    let catalog_digest = mcp_service::operation_catalog_digest(&catalog.services);
    let exact_view = mcp_service::exact_operation_view(&catalog.services);
    let exact_view_digest = mcp_service::exact_operation_view_digest(&exact_view);
    let legacy_exact_view_digest = mcp_service::legacy_exact_operation_view_digest(&exact_view);
    let endpoint = &catalog.services[service_index].endpoints[endpoint_index];
    let producer_operation_generation =
        mcp_service::producer_operation_generation(&catalog.services[service_index], endpoint);
    let operation_generation = producer_operation_generation.unwrap_or(0);
    let endpoint_contract_digest = mcp_service::endpoint_contract_digest(endpoint);
    let operation_digest =
        mcp_service::exact_operation_digest(user_service_id, endpoint, arguments);
    Ok(ExactCatalogResolution {
        catalog,
        service_index,
        endpoint_index,
        catalog_digest,
        exact_view_digest,
        legacy_exact_view_digest,
        in_exact_view,
        endpoint_contract_digest,
        operation_digest,
        operation_generation,
        producer_generation_bound: producer_operation_generation.is_some(),
        marker: std::marker::PhantomData,
    })
}

fn exact_view_membership(
    caller: &ExactServiceApprovalCaller,
    service: &mcp_service::McpToolService,
) -> AppResult<bool> {
    let in_exact_view = mcp_service::is_exact_visible(service);
    if caller.requester_type == DELEGATED_REQUESTER_TYPE && !in_exact_view {
        Err(AppError::NotFound(
            "exact_operation_not_in_exact_view".to_string(),
        ))
    } else {
        Ok(in_exact_view)
    }
}

fn ensure_delegated_exact_authority(
    caller: &ExactServiceApprovalCaller,
    provided_digest: Option<&str>,
    require_digest: bool,
) -> AppResult<()> {
    if caller.requester_type != DELEGATED_REQUESTER_TYPE {
        return Ok(());
    }
    if !caller.has_catalog_read {
        return Err(AppError::Forbidden(
            DELEGATED_CATALOG_SCOPE_REQUIRED.to_string(),
        ));
    }
    if require_digest && provided_digest.is_none() {
        return Err(AppError::BadRequest(EXACT_VIEW_DIGEST_REQUIRED.to_string()));
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct ApprovalTarget {
    service_id: String,
    service_name: String,
    service_slug: String,
    service_owner_user_id: String,
    is_auto_connected: bool,
}

async fn approval_target(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    service: &mcp_service::McpToolService,
) -> AppResult<ApprovalTarget> {
    let user_service_id = match &service.source {
        mcp_service::McpToolSource::UserManaged {
            user_service_id, ..
        } => user_service_id,
        mcp_service::McpToolSource::Platform { .. } => {
            return Err(AppError::BadRequest(
                "exact_service_requires_user_service".to_string(),
            ));
        }
    };
    let hint = proxy_service::find_approval_resolution_hint_by_user_service_id(
        &state.db,
        &caller.approval_owner_user_id,
        user_service_id,
        Some(&service.service_slug),
        None,
    )
    .await?
    .ok_or_else(|| AppError::NotFound("exact_user_service_not_found".to_string()))?;
    Ok(ApprovalTarget {
        service_id: hint.service_id,
        service_name: service.service_name.clone(),
        service_slug: service.service_slug.clone(),
        service_owner_user_id: hint.service_owner_id,
        is_auto_connected: hint.is_auto_connected,
    })
}

async fn load_bound_request(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    request_id: &str,
) -> AppResult<ApprovalRequest> {
    let request = state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! { "_id": request_id })
        .await?
        .ok_or_else(|| AppError::NotFound("exact_service_request_not_found".to_string()))?;
    if request.exact_service.is_none()
        || request.requester_type != caller.requester_type
        || request.requester_id != caller.requester_id
        || request
            .exact_service
            .as_ref()
            .is_some_and(|binding| binding.actor_user_id != caller.actor_user_id)
    {
        return Err(AppError::Forbidden(
            "exact_service_request_binding_mismatch".to_string(),
        ));
    }
    Ok(request)
}

async fn expire_if_needed(
    state: &AppState,
    request: ApprovalRequest,
) -> AppResult<ApprovalRequest> {
    let exact_unredeemed = request.exact_service.as_ref().is_some_and(|binding| {
        binding.redemption.is_none() && matches!(request.status.as_str(), "pending" | "approved")
    });
    if !exact_unredeemed || request.expires_at > Utc::now() {
        return Ok(request);
    }
    let now = Utc::now();
    let updated = state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one_and_update(
            doc! {
                "_id": &request.id,
                "status": &request.status,
                "expires_at": { "$lte": bson::DateTime::from_chrono(now) },
                "exact_service": { "$exists": true },
                "exact_service.redemption": { "$exists": false },
            },
            doc! { "$set": { "status": "expired", "decided_at": bson::DateTime::from_chrono(now) } },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;
    match updated {
        Some(updated) => Ok(updated),
        None => state
            .db
            .collection::<ApprovalRequest>(REQUESTS)
            .find_one(doc! { "_id": &request.id })
            .await?
            .ok_or_else(|| AppError::NotFound("exact_service_request_not_found".to_string())),
    }
}

struct ObservedApprovalState {
    state: ExactServiceApprovalState,
    failure_code: Option<String>,
}

fn observed(state: ExactServiceApprovalState, failure_code: Option<&str>) -> ObservedApprovalState {
    ObservedApprovalState {
        state,
        failure_code: failure_code.map(str::to_string),
    }
}

fn persisted_state(request: &ApprovalRequest) -> AppResult<ObservedApprovalState> {
    match request.status.as_str() {
        "pending" => Ok(observed(ExactServiceApprovalState::Pending, None)),
        "rejected" => Ok(observed(ExactServiceApprovalState::Denied, None)),
        "expired" => Ok(observed(ExactServiceApprovalState::Expired, None)),
        "approved"
            if request
                .exact_service
                .as_ref()
                .and_then(|binding| binding.redemption.as_ref())
                .is_some() =>
        {
            redemption_state(request)
        }
        "approved" => Ok(observed(ExactServiceApprovalState::Approved, None)),
        _ => Ok(observed(ExactServiceApprovalState::Revoked, None)),
    }
}

async fn current_state(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    request: &ApprovalRequest,
) -> AppResult<ObservedApprovalState> {
    let persisted = persisted_state(request)?;
    if persisted.state != ExactServiceApprovalState::Approved {
        return Ok(persisted);
    }
    match evaluate_live_authority(state, caller, request).await {
        LiveAuthorityEvaluation::Matched(_) => {
            Ok(observed(ExactServiceApprovalState::Approved, None))
        }
        LiveAuthorityEvaluation::Terminal(terminal) => {
            Ok(observed(terminal.state, Some(terminal.failure_code)))
        }
        LiveAuthorityEvaluation::Error { error, .. } => Err(error),
    }
}

enum LiveAuthorityEvaluation {
    Matched(ExactCatalogResolution<'static>),
    Terminal(ExecutionAuthorityTerminal),
    Error {
        error: AppError,
        failure_code: &'static str,
    },
}

async fn evaluate_live_authority(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    request: &ApprovalRequest,
) -> LiveAuthorityEvaluation {
    let binding = request.exact_service.as_ref().unwrap();
    let resolution = match resolve_exact_catalog(
        state,
        caller,
        &binding.user_service_id,
        &binding.endpoint_id,
        &binding.arguments,
    )
    .await
    {
        Ok(resolution) => resolution,
        Err(error) if catalog_resolution_terminal_state(&error).is_some() => {
            return LiveAuthorityEvaluation::Terminal(authority_terminal(
                ExactServiceApprovalState::Revoked,
                ExactServiceRedemptionStatus::Revoked,
                "selector_revoked",
            ));
        }
        Err(error) => {
            let failure_code = safe_execution_failure_code(&error);
            return LiveAuthorityEvaluation::Error {
                error,
                failure_code,
            };
        }
    };
    let live_authority = ExactAuthoritySnapshot {
        user_service_id: &resolution.service().service_id,
        endpoint_id: &resolution.endpoint().endpoint_id,
        catalog_digest: &resolution.catalog_digest,
        exact_view_digest: &resolution.exact_view_digest,
        legacy_exact_view_digest: &resolution.legacy_exact_view_digest,
        endpoint_contract_digest: &resolution.endpoint_contract_digest,
        operation_digest: &resolution.operation_digest,
        operation_generation: resolution.operation_generation,
    };
    if exact_authority_has_drift(binding, &live_authority) {
        return LiveAuthorityEvaluation::Terminal(authority_terminal(
            ExactServiceApprovalState::Drifted,
            ExactServiceRedemptionStatus::Drifted,
            "catalog_drift",
        ));
    }
    match approval_policy_is_live(state, caller, &resolution, &binding.arguments).await {
        Ok(true) => {}
        Ok(false) => {
            return LiveAuthorityEvaluation::Terminal(authority_terminal(
                ExactServiceApprovalState::Revoked,
                ExactServiceRedemptionStatus::Revoked,
                "selector_revoked",
            ));
        }
        Err(error) => {
            return LiveAuthorityEvaluation::Error {
                error,
                failure_code: "authorization_revalidation_failed",
            };
        }
    }
    match evaluate_execution_authority(
        state,
        caller,
        binding,
        &request.service_slug,
        ExecutionResolutionMode::ReadOnlySnapshot,
    )
    .await
    {
        ExecutionAuthorityEvaluation::Matched(_) => LiveAuthorityEvaluation::Matched(resolution),
        ExecutionAuthorityEvaluation::Terminal(terminal) => {
            LiveAuthorityEvaluation::Terminal(terminal)
        }
    }
}

async fn approval_policy_is_live(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    resolution: &ExactCatalogResolution<'_>,
    arguments: &serde_json::Value,
) -> AppResult<bool> {
    let descriptor = mcp_service::build_mcp_operation_descriptor(
        resolution.service(),
        resolution.endpoint(),
        arguments,
    )?;
    let target = approval_target(state, caller, resolution.service()).await?;
    Ok(matches!(
        approval_service::evaluate_and_check(
            &state.db,
            &caller.approval_owner_user_id,
            &target.service_owner_user_id,
            &target.service_id,
            &descriptor,
            Some(&caller.requester_type),
            &caller.requester_id,
            false,
            target.is_auto_connected,
        )
        .await?,
        approval_service::ApprovalOutcome::NeedsApproval(pending)
            if pending.resolution.mode == ApprovalMode::PerRequest
    ))
}

fn redemption_state(request: &ApprovalRequest) -> AppResult<ObservedApprovalState> {
    let redemption = request
        .exact_service
        .as_ref()
        .and_then(|binding| binding.redemption.as_ref())
        .ok_or_else(|| AppError::Conflict("exact_service_redemption_missing".to_string()))?;
    Ok(ObservedApprovalState {
        state: match redemption.status {
            ExactServiceRedemptionStatus::Executing => ExactServiceApprovalState::Redeeming,
            ExactServiceRedemptionStatus::Completed => ExactServiceApprovalState::Redeemed,
            ExactServiceRedemptionStatus::Drifted => ExactServiceApprovalState::Drifted,
            ExactServiceRedemptionStatus::Revoked => ExactServiceApprovalState::Revoked,
            ExactServiceRedemptionStatus::Failed => ExactServiceApprovalState::Failed,
        },
        failure_code: redemption.failure_code.clone(),
    })
}

fn catalog_resolution_terminal_state(error: &AppError) -> Option<ExactServiceApprovalState> {
    match error {
        AppError::NotFound(_)
        | AppError::ApiKeyScopeForbidden(_)
        | AppError::ApiKeyScopeInactive
        | AppError::ApiKeyScopeNotFound(_) => Some(ExactServiceApprovalState::Revoked),
        _ => None,
    }
}

#[derive(Clone, Copy)]
struct ExactAuthoritySnapshot<'a> {
    user_service_id: &'a str,
    endpoint_id: &'a str,
    catalog_digest: &'a str,
    exact_view_digest: &'a str,
    legacy_exact_view_digest: &'a str,
    endpoint_contract_digest: &'a str,
    operation_digest: &'a str,
    operation_generation: i64,
}

fn exact_authority_has_drift(
    binding: &ExactServiceApprovalBinding,
    live: &ExactAuthoritySnapshot<'_>,
) -> bool {
    binding.user_service_id != live.user_service_id
        || binding.endpoint_id != live.endpoint_id
        || binding.catalog_digest != live.catalog_digest
        || binding.exact_view_digest.as_deref().is_some_and(|stored| {
            stored != live.exact_view_digest && stored != live.legacy_exact_view_digest
        })
        || binding
            .exact_view_digest_binding
            .as_deref()
            .is_some_and(|stored| stored != live.exact_view_digest)
        || binding.endpoint_contract_digest != live.endpoint_contract_digest
        || binding.operation_digest != live.operation_digest
        || (binding.producer_generation_bound
            && binding.operation_generation != live.operation_generation)
}

fn ensure_fence(request: &ApprovalRequest, fence: &ExactServiceApprovalFence) -> AppResult<()> {
    let binding = request.exact_service.as_ref().unwrap();
    if binding.catalog_digest != fence.catalog_digest
        || fence.exact_view_digest.as_ref().is_some_and(|provided| {
            binding.exact_view_digest.as_ref() != Some(provided)
                && binding.exact_view_digest_binding.as_ref() != Some(provided)
        })
        || binding.operation_digest != fence.operation_digest
        || binding.operation_id != fence.operation_id
        || binding.operation_generation != fence.operation_generation
        || binding.effect_idempotency_key != fence.idempotency_key
    {
        return Err(AppError::Conflict(
            "exact_service_redemption_conflict".to_string(),
        ));
    }
    Ok(())
}

fn result_for(
    request: &ApprovalRequest,
    state: ExactServiceApprovalState,
) -> AppResult<ExactServiceApprovalResult> {
    let binding = request
        .exact_service
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("not_an_exact_service_request".to_string()))?;
    let redemption = binding.redemption.as_ref();
    Ok(ExactServiceApprovalResult {
        request_id: request.id.clone(),
        state,
        user_service_id: binding.user_service_id.clone(),
        endpoint_id: binding.endpoint_id.clone(),
        catalog_digest: binding.catalog_digest.clone(),
        exact_view_digest: binding.exact_view_digest.clone(),
        endpoint_contract_digest: binding.endpoint_contract_digest.clone(),
        operation_digest: binding.operation_digest.clone(),
        operation_id: binding.operation_id.clone(),
        operation_generation: binding.operation_generation,
        idempotency_key: binding.effect_idempotency_key.clone(),
        expires_at: request.expires_at.to_rfc3339(),
        receipt: redemption.and_then(|value| value.receipt.clone()),
        failure_code: redemption.and_then(|value| value.failure_code.clone()),
    })
}

fn validate_create(input: &ExactServiceApprovalCreate) -> AppResult<()> {
    for (field, value) in [
        ("user_service_id", input.user_service_id.as_str()),
        ("endpoint_id", input.endpoint_id.as_str()),
        ("catalog_digest", input.catalog_digest.as_str()),
        (
            "endpoint_contract_digest",
            input.endpoint_contract_digest.as_str(),
        ),
        ("operation_digest", input.operation_digest.as_str()),
        ("operation_id", input.operation_id.as_str()),
        ("idempotency_key", input.idempotency_key.as_str()),
    ] {
        if value.trim().is_empty() {
            return Err(AppError::BadRequest(format!("{field} is required")));
        }
    }
    if !input.arguments.is_object() {
        return Err(AppError::BadRequest(
            "arguments must be a JSON object".to_string(),
        ));
    }
    if input
        .exact_view_digest
        .as_deref()
        .is_some_and(|digest| digest.trim().is_empty())
    {
        return Err(AppError::BadRequest(
            "exact_view_digest must not be empty when provided".to_string(),
        ));
    }
    Ok(())
}

fn ensure_digest(field: &str, provided: &str, authoritative: &str) -> AppResult<()> {
    if provided == authoritative {
        Ok(())
    } else {
        Err(AppError::Conflict(format!("exact_service_{field}_drift")))
    }
}

fn ensure_exact_view_digest(
    provided: &str,
    authoritative: &ExactCatalogResolution<'_>,
) -> AppResult<()> {
    if provided == authoritative.exact_view_digest
        || provided == authoritative.legacy_exact_view_digest
    {
        Ok(())
    } else {
        Err(AppError::Conflict(
            "exact_service_exact_view_digest_drift".to_string(),
        ))
    }
}

fn request_key(
    caller: &ExactServiceApprovalCaller,
    producer_operation_id: &str,
    operation_generation: i64,
    effect_idempotency_key: &str,
) -> String {
    mcp_service::canonical_sha256(serde_json::json!({
        "contract_version": "nyxid-exact-approval-request.v1",
        "requester_type": caller.requester_type,
        "requester_id": caller.requester_id,
        "actor_user_id": caller.actor_user_id,
        "operation_id": producer_operation_id,
        "operation_generation": operation_generation,
        "idempotency_key": effect_idempotency_key,
    }))
}

async fn reject_legacy_request_replay(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    producer_operation_id: &str,
    effect_idempotency_key: &str,
) -> AppResult<()> {
    let existing = state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! {
            "requester_type": &caller.requester_type,
            "requester_id": &caller.requester_id,
            "exact_service.actor_user_id": &caller.actor_user_id,
            "exact_service.endpoint_id": producer_operation_id,
            "exact_service.effect_idempotency_key": effect_idempotency_key,
            "exact_service.producer_generation_bound": { "$ne": true },
        })
        .await?;
    if existing.is_some() {
        return Err(AppError::Conflict(
            "exact_service_request_conflict".to_string(),
        ));
    }
    Ok(())
}

fn safe_execution_failure_code(error: &AppError) -> &'static str {
    match error {
        AppError::ApiKeyScopeForbidden(_)
        | AppError::ApiKeyScopeInactive
        | AppError::ApiKeyScopeNotFound(_) => "authorization_revoked",
        AppError::NotFound(_) => "selector_revoked",
        AppError::NodeOffline(_) | AppError::NodeProxyTimeout => "provider_unavailable",
        _ => "provider_execution_failed",
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{Router, routing::any};
    use chrono::Duration;

    use super::*;

    fn caller() -> ExactServiceApprovalCaller {
        ExactServiceApprovalCaller {
            actor_user_id: "user-alpha".to_string(),
            proxy_resolution_user_id: "user-alpha".to_string(),
            approval_owner_user_id: "user-alpha".to_string(),
            requester_type: "delegated".to_string(),
            requester_id: "client-alpha".to_string(),
            requester_label: None,
            api_key_id: None,
            has_catalog_read: true,
            allow_all_services: true,
            allowed_service_ids: vec![],
            allow_all_nodes: true,
            allowed_node_ids: vec![],
        }
    }

    fn caller_for(user_id: &str) -> ExactServiceApprovalCaller {
        let mut value = caller();
        value.actor_user_id = user_id.to_string();
        value.proxy_resolution_user_id = user_id.to_string();
        value.approval_owner_user_id = user_id.to_string();
        value
    }

    fn exact_test_service(
        user_service_id: &str,
        owner_id: &str,
        catalog_service_id: &str,
        slug: &str,
    ) -> mcp_service::McpToolService {
        mcp_service::McpToolService {
            service_id: catalog_service_id.to_string(),
            service_name: "Exact test service".to_string(),
            service_slug: slug.to_string(),
            description: None,
            service_category: "user_service".to_string(),
            endpoints: Vec::new(),
            durable_endpoint_metadata: HashMap::new(),
            source: mcp_service::McpToolSource::UserManaged {
                user_service_id: user_service_id.to_string(),
                catalog_service_id: Some(catalog_service_id.to_string()),
                effective_owner_id: owner_id.to_string(),
                node_id: None,
                has_server_credential: true,
            },
            executable: true,
            is_generic_proxy: false,
            invalid_openapi_contract: false,
            recommended_skills: Vec::new(),
            proxy_operation_policy: None,
        }
    }

    async fn insert_exact_test_service(
        db: &Database,
        owner_id: &str,
        user_service_id: &str,
        catalog_service_id: &str,
        slug: &str,
        source: Option<&str>,
    ) {
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user_endpoint::UserEndpoint>(
            crate::models::user_endpoint::COLLECTION_NAME,
        )
        .insert_one(crate::test_utils::test_user_endpoint(
            &endpoint_id,
            owner_id,
            "Exact test service",
            "https://exact.invalid",
            None,
            Some(catalog_service_id),
        ))
        .await
        .unwrap();
        let mut service = crate::test_utils::test_user_service(
            user_service_id,
            owner_id,
            slug,
            &endpoint_id,
            Some(catalog_service_id),
            None,
        );
        service.source = source.map(str::to_string);
        db.collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .insert_one(service)
        .await
        .unwrap();
    }

    fn create() -> ExactServiceApprovalCreate {
        ExactServiceApprovalCreate {
            user_service_id: "service-alpha".to_string(),
            endpoint_id: "endpoint-alpha".to_string(),
            catalog_digest: "sha256:catalog".to_string(),
            exact_view_digest: Some("sha256:exact-view".to_string()),
            endpoint_contract_digest: "sha256:contract".to_string(),
            operation_digest: "sha256:operation".to_string(),
            operation_id: "endpoint-alpha".to_string(),
            operation_generation: Some(3),
            idempotency_key: "idempotency-alpha".to_string(),
            arguments: serde_json::json!({"value": 1}),
        }
    }

    fn binding() -> ExactServiceApprovalBinding {
        let input = create();
        let operation_generation = 3;
        let exact_view_digest_binding = input.exact_view_digest.clone();
        ExactServiceApprovalBinding {
            request_key: request_key(
                &caller(),
                &input.endpoint_id,
                operation_generation,
                &input.idempotency_key,
            ),
            actor_user_id: caller().actor_user_id,
            user_service_id: input.user_service_id,
            endpoint_id: input.endpoint_id,
            catalog_digest: input.catalog_digest,
            exact_view_digest: input.exact_view_digest,
            exact_view_digest_binding,
            endpoint_contract_digest: input.endpoint_contract_digest,
            operation_digest: input.operation_digest,
            operation_id: input.operation_id,
            operation_generation,
            producer_generation_bound: true,
            effect_idempotency_key: input.idempotency_key,
            arguments: input.arguments,
            execution_authority_digest: None,
            execution_authority_binding: None,
            redemption: None,
        }
    }

    #[test]
    fn execution_authority_versions_are_rolling_compatible() {
        let live_v2 = "sha256:live-v2";
        let live_v1 = "sha256:live-v1";
        let mut current = binding();
        current.execution_authority_digest = Some(live_v1.to_string());
        current.execution_authority_binding = Some(ExactServiceExecutionAuthorityBinding {
            projection_version: execution_authority::CONTRACT_VERSION.to_string(),
            digest: live_v2.to_string(),
        });
        assert_eq!(
            execution_authority_digest_decision(&current, live_v2, live_v1),
            ExecutionAuthorityDigestDecision::Matched
        );

        let mut drifted = current.clone();
        drifted.execution_authority_binding.as_mut().unwrap().digest =
            "sha256:changed-v2".to_string();
        assert_eq!(
            execution_authority_digest_decision(&drifted, live_v2, live_v1),
            ExecutionAuthorityDigestDecision::Drifted
        );

        let mut old_projection = current.clone();
        let old_binding = old_projection.execution_authority_binding.as_mut().unwrap();
        old_binding.projection_version = execution_authority::LEGACY_CONTRACT_VERSION.to_string();
        old_binding.digest = live_v1.to_string();
        assert_eq!(
            execution_authority_digest_decision(&old_projection, live_v2, live_v1),
            ExecutionAuthorityDigestDecision::Matched
        );

        let mut legacy_row = binding();
        legacy_row.execution_authority_digest = Some(live_v1.to_string());
        assert_eq!(
            execution_authority_digest_decision(&legacy_row, live_v2, live_v1),
            ExecutionAuthorityDigestDecision::Matched
        );
        assert_eq!(
            execution_authority_digest_decision(&legacy_row, live_v2, "sha256:changed-v1"),
            ExecutionAuthorityDigestDecision::Drifted
        );

        let pre_digest_row = binding();
        assert_eq!(
            execution_authority_digest_decision(&pre_digest_row, live_v2, live_v1),
            ExecutionAuthorityDigestDecision::Matched,
            "pre-digest rows retain main's expiry-bounded authority behavior"
        );
    }

    fn authority_snapshot(binding: &ExactServiceApprovalBinding) -> ExactAuthoritySnapshot<'_> {
        ExactAuthoritySnapshot {
            user_service_id: &binding.user_service_id,
            endpoint_id: &binding.endpoint_id,
            catalog_digest: &binding.catalog_digest,
            exact_view_digest: binding
                .exact_view_digest_binding
                .as_deref()
                .or(binding.exact_view_digest.as_deref())
                .unwrap(),
            legacy_exact_view_digest: binding.exact_view_digest.as_deref().unwrap(),
            endpoint_contract_digest: &binding.endpoint_contract_digest,
            operation_digest: &binding.operation_digest,
            operation_generation: binding.operation_generation,
        }
    }

    #[tokio::test]
    async fn exact_auto_connected_global_default_is_not_required() {
        let Some(db) = crate::test_utils::connect_test_database("exact_auto_global_default").await
        else {
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let user_service_id = uuid::Uuid::new_v4().to_string();
        let catalog_service_id = uuid::Uuid::new_v4().to_string();
        let slug = "exact-auto-global";
        insert_exact_test_service(
            &db,
            &actor_id,
            &user_service_id,
            &catalog_service_id,
            slug,
            Some(crate::models::user_service::AUTO_PROVISION_SOURCE),
        )
        .await;
        let channel = crate::models::notification_channel::NotificationChannel {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: actor_id.clone(),
            telegram_chat_id: Some(1234),
            telegram_username: Some("exact-test".to_string()),
            telegram_enabled: true,
            telegram_link_code: None,
            telegram_link_code_expires_at: None,
            approval_timeout_secs: 30,
            grant_expiry_days: 30,
            approval_required: true,
            push_enabled: false,
            push_devices: Vec::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        db.collection::<crate::models::notification_channel::NotificationChannel>(
            crate::models::notification_channel::COLLECTION_NAME,
        )
        .insert_one(channel)
        .await
        .unwrap();

        let service = exact_test_service(&user_service_id, &actor_id, &catalog_service_id, slug);
        let caller = caller_for(&actor_id);
        let target = approval_target(
            &crate::test_utils::test_app_state(db.clone()),
            &caller,
            &service,
        )
        .await
        .unwrap();
        let outcome = approval_service::evaluate_and_check(
            &db,
            &actor_id,
            &target.service_owner_user_id,
            &target.service_id,
            &crate::services::operation_descriptor::build_mcp_descriptor("POST", "/items", None),
            Some("delegated"),
            "exact-client",
            false,
            target.is_auto_connected,
        )
        .await
        .unwrap();

        assert!(matches!(
            outcome,
            approval_service::ApprovalOutcome::Allowed { required: false }
        ));
        assert_eq!(
            db.collection::<ApprovalRequest>(REQUESTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn exact_auto_connected_explicit_per_service_config_still_requires_approval() {
        let Some(db) = crate::test_utils::connect_test_database("exact_auto_explicit_config").await
        else {
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let user_service_id = uuid::Uuid::new_v4().to_string();
        let catalog_service_id = uuid::Uuid::new_v4().to_string();
        let slug = "exact-auto-explicit";
        insert_exact_test_service(
            &db,
            &actor_id,
            &user_service_id,
            &catalog_service_id,
            slug,
            Some(crate::models::user_service::AUTO_PROVISION_SOURCE),
        )
        .await;
        let now = Utc::now();
        db.collection::<crate::models::service_approval_config::ServiceApprovalConfig>(
            crate::models::service_approval_config::COLLECTION_NAME,
        )
        .insert_one(
            crate::models::service_approval_config::ServiceApprovalConfig {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: actor_id.clone(),
                service_id: catalog_service_id.clone(),
                service_name: "Exact test service".to_string(),
                approval_required: true,
                approval_mode: ApprovalMode::PerRequest,
                rules: Vec::new(),
                default_effect: None,
                created_at: now,
                updated_at: now,
            },
        )
        .await
        .unwrap();

        let service = exact_test_service(&user_service_id, &actor_id, &catalog_service_id, slug);
        let caller = caller_for(&actor_id);
        let state = crate::test_utils::test_app_state(db.clone());
        let target = approval_target(&state, &caller, &service).await.unwrap();
        let outcome = approval_service::evaluate_and_check(
            &db,
            &actor_id,
            &target.service_owner_user_id,
            &target.service_id,
            &crate::services::operation_descriptor::build_mcp_descriptor("POST", "/items", None),
            Some("delegated"),
            "exact-client",
            false,
            target.is_auto_connected,
        )
        .await
        .unwrap();

        assert!(matches!(
            outcome,
            approval_service::ApprovalOutcome::NeedsApproval(pending)
                if pending.resolution.mode == ApprovalMode::PerRequest
        ));
    }

    fn approval_request(
        id: &str,
        status: &str,
        expires_at: chrono::DateTime<Utc>,
        exact_service: Option<ExactServiceApprovalBinding>,
    ) -> ApprovalRequest {
        let idempotency_key = exact_service
            .as_ref()
            .map(|value| value.request_key.clone())
            .unwrap_or_else(|| format!("generic-{id}"));
        ApprovalRequest {
            id: id.to_string(),
            user_id: "user-alpha".to_string(),
            service_id: "catalog-alpha".to_string(),
            service_name: "Service".to_string(),
            service_slug: "service".to_string(),
            requester_type: "delegated".to_string(),
            requester_id: "client-alpha".to_string(),
            requester_label: None,
            operation_summary: "write".to_string(),
            action_description: None,
            http_method: Some("POST".to_string()),
            resource: Some("/items".to_string()),
            verb: Some("write".to_string()),
            grant_scope: None,
            tool_name: None,
            tool_call_id: None,
            tool_arguments: None,
            is_destructive: None,
            approval_mode: ApprovalMode::PerRequest,
            status: status.to_string(),
            idempotency_key,
            notification_channel: None,
            telegram_message_id: None,
            telegram_chat_id: None,
            expires_at,
            decided_at: (status != "pending").then(Utc::now),
            decision_channel: (status != "pending").then(|| "web".to_string()),
            decision_idempotency_key: None,
            notify_user_ids: vec!["user-alpha".to_string()],
            from_org_policy: false,
            exact_service,
            created_at: Utc::now(),
        }
    }

    async fn create_bound_approval(
        db: &Database,
        exact_service: ExactServiceApprovalBinding,
    ) -> AppResult<ApprovalRequest> {
        let state = crate::test_utils::test_app_state(db.clone());
        approval_service::create_approval_request(
            db,
            &state.config,
            &state.http_client,
            None,
            None,
            "user-alpha",
            "catalog-alpha",
            "Service",
            "service",
            "delegated",
            "client-alpha",
            None,
            approval_service::ApprovalRequestOperation {
                operation_summary: "write".to_string(),
                action_description: None,
                http_method: Some("POST".to_string()),
                resource: Some("/items".to_string()),
                verb: Some("write".to_string()),
                grant_scope: None,
                exact_service: Some(exact_service),
            },
            ApprovalMode::PerRequest,
            300,
            vec!["user-alpha".to_string()],
            false,
        )
        .await
    }

    #[test]
    fn request_key_is_bound_to_requester_and_operation_identity_not_selector() {
        let caller = caller();
        let first = create();
        let mut conflicting_selector = create();
        conflicting_selector.endpoint_id = "endpoint-beta".to_string();
        assert_eq!(
            request_key(&caller, &first.operation_id, 3, &first.idempotency_key),
            request_key(
                &caller,
                &first.operation_id,
                3,
                &conflicting_selector.idempotency_key,
            )
        );
        assert_ne!(
            request_key(&caller, &first.operation_id, 3, &first.idempotency_key),
            request_key(&caller, &first.operation_id, 4, &first.idempotency_key),
            "the producer generation, not caller input, is part of idempotency identity"
        );
        assert_eq!(
            request_key(&caller, &first.operation_id, 3, &first.idempotency_key),
            mcp_service::canonical_sha256(serde_json::json!({
                "contract_version": "nyxid-exact-approval-request.v1",
                "requester_type": caller.requester_type,
                "requester_id": caller.requester_id,
                "actor_user_id": caller.actor_user_id,
                "operation_id": first.operation_id,
                "operation_generation": 3,
                "idempotency_key": first.idempotency_key,
            })),
            "rolling old and new servers must derive the same v1 unique key"
        );
    }

    #[test]
    fn redemption_fence_rejects_conflicting_replay() {
        let input = create();
        let mut exact_binding = binding();
        exact_binding.exact_view_digest = Some("sha256:legacy-exact-view".to_string());
        exact_binding.exact_view_digest_binding = Some("sha256:additive-exact-view".to_string());
        let request = approval_request(
            "request-alpha",
            "approved",
            Utc::now() + Duration::minutes(5),
            Some(exact_binding),
        );
        let mut fence = ExactServiceApprovalFence {
            catalog_digest: input.catalog_digest,
            exact_view_digest: Some("sha256:legacy-exact-view".to_string()),
            operation_digest: input.operation_digest,
            operation_id: input.operation_id,
            operation_generation: 3,
            idempotency_key: input.idempotency_key,
        };
        ensure_fence(&request, &fence).expect("legacy exact-view digest echo is accepted");
        fence.exact_view_digest = Some("sha256:additive-exact-view".to_string());
        ensure_fence(&request, &fence).expect("additive exact-view digest echo is accepted");
        fence.exact_view_digest = None;
        ensure_fence(&request, &fence).expect("legacy client may omit additive exact-view fence");
        fence.exact_view_digest = Some("sha256:unrecognized-exact-view".to_string());
        assert!(matches!(
            ensure_fence(&request, &fence),
            Err(AppError::Conflict(_))
        ));
        fence.exact_view_digest = Some("sha256:additive-exact-view".to_string());
        fence.operation_generation += 1;
        assert!(matches!(
            ensure_fence(&request, &fence),
            Err(AppError::Conflict(_))
        ));
    }

    #[test]
    fn drift_and_revocation_are_typed_before_effect_dispatch() {
        let binding = binding();
        let live = authority_snapshot(&binding);
        assert!(!exact_authority_has_drift(&binding, &live));
        assert!(exact_authority_has_drift(
            &binding,
            &ExactAuthoritySnapshot {
                endpoint_contract_digest: "sha256:changed-contract",
                ..live
            },
        ));
        assert!(exact_authority_has_drift(
            &binding,
            &ExactAuthoritySnapshot {
                exact_view_digest: "sha256:changed-exact-view",
                legacy_exact_view_digest: "sha256:changed-legacy-exact-view",
                ..live
            },
        ));
        let mut legacy_binding = binding.clone();
        legacy_binding.exact_view_digest = None;
        legacy_binding.exact_view_digest_binding = None;
        assert!(!exact_authority_has_drift(
            &legacy_binding,
            &ExactAuthoritySnapshot {
                exact_view_digest: "sha256:any-exact-view",
                ..authority_snapshot(&binding)
            },
        ));
        let mut unratified = binding.clone();
        unratified.producer_generation_bound = false;
        assert!(!exact_authority_has_drift(
            &unratified,
            &ExactAuthoritySnapshot {
                operation_generation: unratified.operation_generation + 1,
                ..authority_snapshot(&unratified)
            },
        ));
        assert!(exact_authority_has_drift(
            &binding,
            &ExactAuthoritySnapshot {
                operation_generation: binding.operation_generation + 1,
                ..live
            },
        ));
        let mut legacy_exact_view = binding.clone();
        legacy_exact_view.exact_view_digest = Some("sha256:legacy-exact-view".to_string());
        legacy_exact_view.exact_view_digest_binding = None;
        assert!(!exact_authority_has_drift(
            &legacy_exact_view,
            &ExactAuthoritySnapshot {
                exact_view_digest: "sha256:current-exact-view",
                legacy_exact_view_digest: "sha256:legacy-exact-view",
                ..authority_snapshot(&legacy_exact_view)
            },
        ));
        let mut dual_digest_binding = binding.clone();
        dual_digest_binding.exact_view_digest = Some("sha256:legacy-exact-view".to_string());
        dual_digest_binding.exact_view_digest_binding =
            Some("sha256:current-exact-view".to_string());
        assert!(exact_authority_has_drift(
            &dual_digest_binding,
            &ExactAuthoritySnapshot {
                exact_view_digest: "sha256:changed-current-exact-view",
                legacy_exact_view_digest: "sha256:legacy-exact-view",
                ..authority_snapshot(&dual_digest_binding)
            },
        ));
        assert_eq!(
            catalog_resolution_terminal_state(&AppError::NotFound(
                "exact_endpoint_not_found".to_string()
            )),
            Some(ExactServiceApprovalState::Revoked)
        );
        assert_eq!(
            catalog_resolution_terminal_state(&AppError::Internal("transient".to_string())),
            None
        );
    }

    #[test]
    fn fail_closed_matrix_keeps_each_fence_and_live_selector_mutation_distinct() {
        let input = create();
        let request = approval_request(
            "request-matrix",
            "approved",
            Utc::now() + Duration::minutes(5),
            Some(binding()),
        );
        let mut wrong_operation = ExactServiceApprovalFence {
            catalog_digest: input.catalog_digest.clone(),
            exact_view_digest: input.exact_view_digest.clone(),
            operation_digest: input.operation_digest.clone(),
            operation_id: input.operation_id.clone(),
            operation_generation: 3,
            idempotency_key: input.idempotency_key.clone(),
        };
        wrong_operation.operation_digest.push_str("-changed");
        assert!(matches!(
            ensure_fence(&request, &wrong_operation),
            Err(AppError::Conflict(message)) if message == "exact_service_redemption_conflict"
        ));

        let mut wrong_idempotency = ExactServiceApprovalFence {
            catalog_digest: input.catalog_digest,
            exact_view_digest: input.exact_view_digest,
            operation_digest: input.operation_digest,
            operation_id: input.operation_id,
            operation_generation: 3,
            idempotency_key: input.idempotency_key,
        };
        wrong_idempotency.idempotency_key.push_str("-changed");
        assert!(matches!(
            ensure_fence(&request, &wrong_idempotency),
            Err(AppError::Conflict(message)) if message == "exact_service_redemption_conflict"
        ));

        // A provider rebinding changes the live exact view (because the
        // producer-owned catalog_service_id is inside that view), while a
        // contract mutation changes only the endpoint contract fence.
        let binding = binding();
        let live = authority_snapshot(&binding);
        assert!(exact_authority_has_drift(
            &binding,
            &ExactAuthoritySnapshot {
                exact_view_digest: "sha256:provider-rebound-exact-view",
                legacy_exact_view_digest: "sha256:provider-rebound-legacy-view",
                ..live
            },
        ));
        assert!(exact_authority_has_drift(
            &binding,
            &ExactAuthoritySnapshot {
                endpoint_contract_digest: "sha256:contract-mutated",
                ..live
            },
        ));
        assert_eq!(
            catalog_resolution_terminal_state(&AppError::NotFound(
                "deactivated_user_service".to_string()
            )),
            Some(ExactServiceApprovalState::Revoked)
        );
        assert_eq!(
            safe_execution_failure_code(&AppError::NodeOffline("unbound".to_string())),
            "provider_unavailable"
        );
    }

    #[test]
    fn exact_view_membership_rejects_only_delegated_generic_targets() {
        let generic_service = mcp_service::McpToolService {
            service_id: "generic-service".to_string(),
            service_name: "Generic Service".to_string(),
            service_slug: "generic-service".to_string(),
            description: None,
            service_category: "connection".to_string(),
            endpoints: Vec::new(),
            durable_endpoint_metadata: HashMap::new(),
            source: mcp_service::McpToolSource::UserManaged {
                user_service_id: "generic-service".to_string(),
                catalog_service_id: None,
                effective_owner_id: "user-alpha".to_string(),
                node_id: None,
                has_server_credential: true,
            },
            executable: true,
            is_generic_proxy: true,
            invalid_openapi_contract: false,
            recommended_skills: Vec::new(),
            proxy_operation_policy: None,
        };

        let delegated_error = exact_view_membership(&caller(), &generic_service)
            .expect_err("delegated callers are confined to exact-view membership");
        assert!(matches!(
            delegated_error,
            AppError::NotFound(message) if message == "exact_operation_not_in_exact_view"
        ));

        let mut access_token_caller = caller();
        access_token_caller.requester_type = "access_token".to_string();
        assert!(
            !exact_view_membership(&access_token_caller, &generic_service)
                .expect("non-delegated callers retain generic-target eligibility")
        );
    }

    /// Named so the blank-digest table below stays readable; the tuple shape
    /// otherwise trips clippy's `type_complexity` under CI's `-D warnings`.
    type BlankDigestMutator = fn(&mut ExactServiceApprovalCreate);

    #[test]
    fn create_rejects_empty_or_whitespace_digest_fields() {
        let fields: [(&str, BlankDigestMutator); 3] = [
            (
                "catalog_digest",
                |input: &mut ExactServiceApprovalCreate| {
                    input.catalog_digest = "  ".to_string();
                },
            ),
            (
                "endpoint_contract_digest",
                |input: &mut ExactServiceApprovalCreate| {
                    input.endpoint_contract_digest = String::new();
                },
            ),
            (
                "operation_digest",
                |input: &mut ExactServiceApprovalCreate| {
                    input.operation_digest = "\t".to_string();
                },
            ),
        ];
        for (field, mutate) in fields {
            let mut input = create();
            mutate(&mut input);
            assert!(
                matches!(validate_create(&input), Err(AppError::BadRequest(message)) if message == format!("{field} is required")),
                "{field} must reject empty input"
            );
        }

        let mut input = create();
        input.exact_view_digest = Some(" \n ".to_string());
        assert!(matches!(
            validate_create(&input),
            Err(AppError::BadRequest(message))
                if message == "exact_view_digest must not be empty when provided"
        ));
    }

    #[test]
    fn digest_mismatch_is_rejected_without_recomputing_a_fixture_value() {
        let err = ensure_digest("catalog_digest", "sha256:caller", "sha256:server")
            .expect_err("caller and server fences must agree");
        assert!(matches!(
            err,
            AppError::Conflict(message) if message == "exact_service_catalog_digest_drift"
        ));
    }

    #[test]
    fn non_delegated_omission_still_persists_server_fence() {
        let mut input = create();
        input.exact_view_digest = None;
        validate_create(&input).expect("legacy callers may omit the additive fence");

        let mut legacy_caller = caller();
        legacy_caller.requester_type = "access_token".to_string();
        assert!(ensure_delegated_exact_authority(&legacy_caller, None, true).is_ok());

        let mut binding = binding();
        binding.exact_view_digest = Some("sha256:server-persisted".to_string());
        assert_eq!(
            result_for(
                &approval_request(
                    "request-server-fence",
                    "pending",
                    Utc::now() + Duration::minutes(5),
                    Some(binding),
                ),
                ExactServiceApprovalState::Pending,
            )
            .expect("serialize persisted result")
            .exact_view_digest
            .as_deref(),
            Some("sha256:server-persisted")
        );
    }

    #[test]
    fn delegated_caller_requires_catalog_scope() {
        let mut delegated = caller();
        delegated.has_catalog_read = false;
        assert!(matches!(
            ensure_delegated_exact_authority(&delegated, None, false),
            Err(AppError::Forbidden(message))
                if message == DELEGATED_CATALOG_SCOPE_REQUIRED
        ));
    }

    #[test]
    fn delegated_create_rejects_omitted_exact_view_digest() {
        let delegated = caller();
        assert!(matches!(
            ensure_delegated_exact_authority(&delegated, None, true),
            Err(AppError::BadRequest(message)) if message == EXACT_VIEW_DIGEST_REQUIRED
        ));
        ensure_delegated_exact_authority(&delegated, Some("sha256:view"), true)
            .expect("delegated caller with catalog scope and digest is authorized");
    }

    #[test]
    fn delegated_redeem_rejects_omitted_fence_digest() {
        let delegated = caller();
        assert!(matches!(
            ensure_delegated_exact_authority(&delegated, None, true),
            Err(AppError::BadRequest(message)) if message == EXACT_VIEW_DIGEST_REQUIRED
        ));
    }

    #[test]
    fn delegated_out_of_view_error_precedence_preserved() {
        let delegated = caller();
        let generic_service = mcp_service::McpToolService {
            service_id: "generic-service".to_string(),
            service_name: "Generic Service".to_string(),
            service_slug: "generic-service".to_string(),
            description: None,
            service_category: "connection".to_string(),
            endpoints: Vec::new(),
            durable_endpoint_metadata: HashMap::new(),
            source: mcp_service::McpToolSource::UserManaged {
                user_service_id: "generic-service".to_string(),
                catalog_service_id: None,
                effective_owner_id: "user-alpha".to_string(),
                node_id: None,
                has_server_credential: true,
            },
            executable: true,
            is_generic_proxy: true,
            invalid_openapi_contract: false,
            recommended_skills: Vec::new(),
            proxy_operation_policy: None,
        };
        ensure_delegated_exact_authority(&delegated, None, false)
            .expect("scope gate passes before membership resolution");
        assert!(matches!(
            exact_view_membership(&delegated, &generic_service),
            Err(AppError::NotFound(message)) if message == "exact_operation_not_in_exact_view"
        ));
    }

    #[tokio::test]
    async fn legacy_delegated_generic_redeem_is_revoked_before_provider_effect() {
        let Some(db) =
            crate::test_utils::connect_test_database("exact_approval_legacy_delegated_generic")
                .await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let provider_calls = Arc::new(AtomicUsize::new(0));
        let handler_calls = provider_calls.clone();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind provider effect spy");
        let provider_addr = listener.local_addr().expect("provider spy address");
        let provider = tokio::spawn(async move {
            axum::serve(
                listener,
                Router::new().fallback(any(move || {
                    let handler_calls = handler_calls.clone();
                    async move {
                        handler_calls.fetch_add(1, Ordering::SeqCst);
                        "unexpected provider effect"
                    }
                })),
            )
            .await
            .expect("serve provider effect spy");
        });

        let user_service_id = "00000000-0000-4000-8000-000000000611";
        let user_endpoint_id = "00000000-0000-4000-8000-000000000612";
        db.collection::<crate::models::user_endpoint::UserEndpoint>(
            crate::models::user_endpoint::COLLECTION_NAME,
        )
        .insert_one(crate::test_utils::test_user_endpoint(
            user_endpoint_id,
            "user-alpha",
            "Legacy Generic",
            &format!("http://{provider_addr}"),
            None,
            None,
        ))
        .await
        .expect("insert legacy generic endpoint");
        db.collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .insert_one(crate::test_utils::test_user_service(
            user_service_id,
            "user-alpha",
            "legacy-generic",
            user_endpoint_id,
            None,
            None,
        ))
        .await
        .expect("insert legacy generic service");

        let catalog = mcp_service::load_operation_catalog(
            &db,
            state.node_ws_manager.as_ref(),
            "user-alpha",
            mcp_service::NodeScope::Unrestricted,
            mcp_service::ServiceScope::Unrestricted,
        )
        .await
        .expect("load legacy generic catalog");
        let generic_service = catalog
            .services
            .iter()
            .find(|service| service.service_id == user_service_id)
            .expect("legacy generic service is in the unprojected catalog");
        let generic_endpoint = generic_service
            .endpoints
            .iter()
            .find(|endpoint| endpoint.endpoint_id == mcp_service::GENERIC_PROXY_ENDPOINT_ID)
            .expect("legacy generic selector exists");
        let arguments = serde_json::json!({"method": "GET", "path": "/effect"});
        let mut legacy_binding = binding();
        legacy_binding.request_key = "legacy-delegated-generic-request-key".to_string();
        legacy_binding.user_service_id = user_service_id.to_string();
        legacy_binding.endpoint_id = generic_endpoint.endpoint_id.clone();
        legacy_binding.catalog_digest = mcp_service::operation_catalog_digest(&catalog.services);
        legacy_binding.exact_view_digest = Some(mcp_service::exact_operation_view_digest(
            &mcp_service::exact_operation_view(&catalog.services),
        ));
        legacy_binding.endpoint_contract_digest =
            mcp_service::endpoint_contract_digest(generic_endpoint);
        legacy_binding.operation_digest =
            mcp_service::exact_operation_digest(user_service_id, generic_endpoint, &arguments);
        legacy_binding.operation_id = "legacy-delegated-generic-operation".to_string();
        legacy_binding.operation_generation = 1;
        legacy_binding.effect_idempotency_key = "legacy-delegated-generic-idempotency".to_string();
        legacy_binding.arguments = arguments;

        let request_id = "00000000-0000-4000-8000-000000000613";
        db.collection::<ApprovalRequest>(REQUESTS)
            .insert_one(approval_request(
                request_id,
                "approved",
                Utc::now() + Duration::minutes(5),
                Some(legacy_binding.clone()),
            ))
            .await
            .expect("insert approved legacy delegated generic request");
        let result = redeem_request(
            &state,
            &caller(),
            request_id,
            ExactServiceApprovalFence {
                catalog_digest: legacy_binding.catalog_digest,
                exact_view_digest: legacy_binding.exact_view_digest,
                operation_digest: legacy_binding.operation_digest,
                operation_id: legacy_binding.operation_id,
                operation_generation: legacy_binding.operation_generation,
                idempotency_key: legacy_binding.effect_idempotency_key,
            },
            crate::services::billing::route_inventory::enforce_billing_egress_classification(
                Some(
                    crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                        crate::services::billing::route_inventory::BillingIngress::Mcp,
                    ),
                ),
                crate::services::billing::route_inventory::BillingIngress::Mcp,
            )
            .expect("construct metered MCP permit"),
        )
        .await
        .expect("legacy delegated generic redeem returns a typed terminal result");
        assert_eq!(result.state, ExactServiceApprovalState::Revoked);
        assert_eq!(result.failure_code.as_deref(), Some("selector_revoked"));
        assert_eq!(provider_calls.load(Ordering::SeqCst), 0);
        provider.abort();
    }

    #[tokio::test]
    async fn exact_create_replays_same_authority_and_rejects_changed_selector() {
        let Some(db) =
            crate::test_utils::connect_test_database("exact_approval_create_replay").await
        else {
            return;
        };
        let first_binding = binding();
        let first = create_bound_approval(&db, first_binding.clone())
            .await
            .expect("create exact approval");
        let replay = create_bound_approval(&db, first_binding.clone())
            .await
            .expect("replay exact approval");
        assert_eq!(replay.id, first.id);
        let mut legacy_retry = first_binding.clone();
        legacy_retry.exact_view_digest = None;
        let replay = create_bound_approval(&db, legacy_retry)
            .await
            .expect("legacy retry without exact-view digest remains idempotent");
        assert_eq!(replay.id, first.id);

        let mut changed_selector = first_binding.clone();
        changed_selector.endpoint_id = "endpoint-beta".to_string();
        assert!(matches!(
            create_bound_approval(&db, changed_selector).await,
            Err(AppError::Conflict(message)) if message == "exact_service_request_conflict"
        ));

        let mut changed_actor = first_binding;
        changed_actor.actor_user_id = "user-beta".to_string();
        assert!(matches!(
            create_bound_approval(&db, changed_actor).await,
            Err(AppError::Conflict(message)) if message == "exact_service_request_conflict"
        ));
    }

    #[tokio::test]
    async fn concurrent_rolling_old_and_new_creates_share_one_semantic_effect_identity() {
        let Some(db) =
            crate::test_utils::connect_test_database("exact_approval_rolling_request_key").await
        else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("install exact semantic effect unique index");
        let mut old_server_binding = binding();
        old_server_binding.producer_generation_bound = false;
        old_server_binding.operation_generation = 2;
        old_server_binding.request_key = request_key(
            &caller(),
            &old_server_binding.endpoint_id,
            2,
            &old_server_binding.effect_idempotency_key,
        );

        let new_server_binding = binding();
        assert_ne!(
            old_server_binding.request_key, new_server_binding.request_key,
            "old caller-owned and new producer-owned generations can derive different request keys"
        );
        let (old_result, new_result) = tokio::join!(
            create_bound_approval(&db, old_server_binding),
            create_bound_approval(&db, new_server_binding),
        );
        let winner = match (old_result, new_result) {
            (Ok(winner), Err(AppError::Conflict(message)))
            | (Err(AppError::Conflict(message)), Ok(winner))
                if message == "exact_service_request_conflict" =>
            {
                winner
            }
            other => panic!("expected one winner and one typed semantic conflict, got {other:?}"),
        };
        assert_eq!(
            db.collection::<ApprovalRequest>(REQUESTS)
                .count_documents(doc! {})
                .await
                .expect("count requests after rolling collision"),
            1,
            "the rolling retry must not create a second approval/effect identity"
        );
        assert_eq!(
            db.collection::<ApprovalRequest>(REQUESTS)
                .find_one(doc! { "_id": &winner.id })
                .await
                .expect("reload winning request")
                .map(|request| request.id),
            Some(winner.id),
        );
    }

    #[tokio::test]
    async fn semantic_legacy_replay_precheck_ignores_status_and_caller_generation() {
        let Some(db) =
            crate::test_utils::connect_test_database("exact_approval_legacy_replay_precheck").await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());

        for (index, (status, caller_generation)) in [
            ("pending", None),
            ("approved", Some(-99)),
            ("redeemed", Some(3)),
        ]
        .into_iter()
        .enumerate()
        {
            let suffix = index + 1;
            let mut input = create();
            input.operation_id = format!("operation-legacy-{suffix}");
            input.idempotency_key = format!("idempotency-legacy-{suffix}");
            input.operation_generation = caller_generation;

            let mut legacy_binding = binding();
            legacy_binding.request_key = format!("arbitrary-old-request-key-{suffix}");
            legacy_binding.operation_id = input.operation_id.clone();
            legacy_binding.operation_generation = 90 + suffix as i64;
            legacy_binding.producer_generation_bound = false;
            legacy_binding.effect_idempotency_key = input.idempotency_key.clone();
            if status == "redeemed" {
                legacy_binding.redemption = Some(ExactServiceApprovalRedemption {
                    status: ExactServiceRedemptionStatus::Completed,
                    admitted_at: Utc::now(),
                    completed_at: Some(Utc::now()),
                    receipt: None,
                    failure_code: None,
                });
            }
            db.collection::<ApprovalRequest>(REQUESTS)
                .insert_one(approval_request(
                    &format!("legacy-request-{suffix}"),
                    status,
                    Utc::now() + Duration::minutes(5),
                    Some(legacy_binding),
                ))
                .await
                .expect("insert arbitrary legacy request");

            assert!(matches!(
                reject_legacy_request_replay(
                    &state,
                    &caller(),
                    &input.endpoint_id,
                    &input.idempotency_key,
                )
                .await,
                Err(AppError::Conflict(message)) if message == "exact_service_request_conflict"
            ));
        }
    }

    #[tokio::test]
    async fn bound_load_rejects_requester_mismatch_and_generic_approval() {
        let Some(db) = crate::test_utils::connect_test_database("exact_approval_binding").await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let exact = approval_request(
            "request-exact",
            "pending",
            Utc::now() + Duration::minutes(5),
            Some(binding()),
        );
        let generic = approval_request(
            "request-generic",
            "pending",
            Utc::now() + Duration::minutes(5),
            None,
        );
        db.collection::<ApprovalRequest>(REQUESTS)
            .insert_many([exact, generic])
            .await
            .expect("insert approval fixtures");

        let mut mismatched = caller();
        mismatched.requester_id = "client-beta".to_string();
        assert!(matches!(
            load_bound_request(&state, &mismatched, "request-exact").await,
            Err(AppError::Forbidden(message)) if message == "exact_service_request_binding_mismatch"
        ));
        assert!(matches!(
            load_bound_request(&state, &caller(), "request-generic").await,
            Err(AppError::Forbidden(message)) if message == "exact_service_request_binding_mismatch"
        ));
    }

    #[tokio::test]
    async fn pending_expiry_and_persisted_terminal_states_are_stable() {
        let Some(db) = crate::test_utils::connect_test_database("exact_approval_terminals").await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let denied = approval_request(
            "request-denied",
            "rejected",
            Utc::now() + Duration::minutes(5),
            Some(binding()),
        );
        let expired = approval_request(
            "request-expired",
            "pending",
            Utc::now() - Duration::seconds(1),
            Some(binding()),
        );
        let expired_approved = approval_request(
            "request-expired-approved",
            "approved",
            Utc::now() - Duration::seconds(1),
            Some(binding()),
        );
        let mut drifted_binding = binding();
        drifted_binding.redemption = Some(ExactServiceApprovalRedemption {
            status: ExactServiceRedemptionStatus::Drifted,
            admitted_at: Utc::now(),
            completed_at: Some(Utc::now()),
            receipt: None,
            failure_code: Some("catalog_drift".to_string()),
        });
        let drifted = approval_request(
            "request-drifted",
            "approved",
            Utc::now() + Duration::minutes(5),
            Some(drifted_binding),
        );
        let mut revoked_binding = binding();
        revoked_binding.redemption = Some(ExactServiceApprovalRedemption {
            status: ExactServiceRedemptionStatus::Revoked,
            admitted_at: Utc::now(),
            completed_at: Some(Utc::now()),
            receipt: None,
            failure_code: Some("selector_revoked".to_string()),
        });
        let revoked = approval_request(
            "request-revoked",
            "approved",
            Utc::now() + Duration::minutes(5),
            Some(revoked_binding),
        );
        db.collection::<ApprovalRequest>(REQUESTS)
            .insert_many([denied, expired, expired_approved, drifted, revoked])
            .await
            .expect("insert terminal fixtures");

        for (request_id, expected) in [
            ("request-denied", ExactServiceApprovalState::Denied),
            ("request-expired", ExactServiceApprovalState::Expired),
            (
                "request-expired-approved",
                ExactServiceApprovalState::Expired,
            ),
            ("request-drifted", ExactServiceApprovalState::Drifted),
            ("request-revoked", ExactServiceApprovalState::Revoked),
        ] {
            let observed = observe_request(&state, &caller(), request_id)
                .await
                .expect("observe terminal request");
            assert_eq!(observed.state, expected);
        }
    }

    #[tokio::test]
    async fn redemption_claim_is_atomic_and_terminal_receipt_is_exactly_replayed() {
        let Some(db) = crate::test_utils::connect_test_database("exact_approval_claim").await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let request = approval_request(
            "request-claim",
            "approved",
            Utc::now() + Duration::minutes(5),
            Some(binding()),
        );
        db.collection::<ApprovalRequest>(REQUESTS)
            .insert_one(&request)
            .await
            .expect("insert claim fixture");

        let admitted_at = Utc::now();
        let (first, second) = tokio::join!(
            claim_redemption(&db, &request, admitted_at),
            claim_redemption(&db, &request, admitted_at),
        );
        let winners = [first.expect("first claim"), second.expect("second claim")]
            .into_iter()
            .filter(Option::is_some)
            .count();
        assert_eq!(winners, 1);

        let receipt = ExactServiceApprovalReceipt {
            http_status: 200,
            response_body: "{\"ok\":true}".to_string(),
            response_digest: "sha256:receipt".to_string(),
        };
        persist_redemption(
            &db,
            &request.id,
            ExactServiceApprovalRedemption {
                status: ExactServiceRedemptionStatus::Completed,
                admitted_at,
                completed_at: Some(Utc::now()),
                receipt: Some(receipt.clone()),
                failure_code: None,
            },
        )
        .await
        .expect("persist winning receipt");

        assert!(matches!(
            persist_redemption(
                &db,
                &request.id,
                ExactServiceApprovalRedemption {
                    status: ExactServiceRedemptionStatus::Failed,
                    admitted_at,
                    completed_at: Some(Utc::now()),
                    receipt: None,
                    failure_code: Some("conflicting_writer".to_string()),
                },
            )
            .await,
            Err(AppError::Conflict(message)) if message == "exact_service_redemption_state_conflict"
        ));

        let replay = observe_request(&state, &caller(), &request.id)
            .await
            .expect("replay terminal receipt");
        assert_eq!(replay.state, ExactServiceApprovalState::Redeemed);
        assert_eq!(replay.receipt, Some(receipt));
    }

    #[tokio::test]
    async fn lost_claim_reloads_concurrent_denial_and_expiry_without_redemption_conflict() {
        let Some(db) = crate::test_utils::connect_test_database("exact_approval_lost_claim").await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let requests = db.collection::<ApprovalRequest>(REQUESTS);

        let denied_stale = approval_request(
            "request-concurrently-denied",
            "approved",
            Utc::now() + Duration::minutes(5),
            Some(binding()),
        );
        requests
            .insert_one(&denied_stale)
            .await
            .expect("insert approved denial-race fixture");
        requests
            .update_one(
                doc! { "_id": &denied_stale.id, "status": "approved" },
                doc! { "$set": { "status": "rejected" } },
            )
            .await
            .expect("concurrently deny stale request");
        assert!(
            claim_redemption(&db, &denied_stale, Utc::now())
                .await
                .expect("losing denial-race claim")
                .is_none()
        );
        let denied = reload_after_lost_redemption_claim(&state, &caller(), &denied_stale.id)
            .await
            .expect("reload concurrent denial without requiring redemption state");
        assert_eq!(denied.state, ExactServiceApprovalState::Denied);

        let expired_stale = approval_request(
            "request-expired-before-claim",
            "approved",
            Utc::now() - Duration::seconds(1),
            Some(binding()),
        );
        requests
            .insert_one(&expired_stale)
            .await
            .expect("insert expired approved fixture");
        assert!(
            claim_redemption(&db, &expired_stale, Utc::now())
                .await
                .expect("expired claim")
                .is_none(),
            "an approval expired at admission must never be claimed"
        );
        let expired = reload_after_lost_redemption_claim(&state, &caller(), &expired_stale.id)
            .await
            .expect("reload expiry without requiring redemption state");
        assert_eq!(expired.state, ExactServiceApprovalState::Expired);
        assert!(
            requests
                .find_one(doc! { "_id": &expired_stale.id })
                .await
                .expect("reload expired row")
                .and_then(|request| request.exact_service)
                .and_then(|binding| binding.redemption)
                .is_none(),
            "a lost claim must not synthesize redemption state"
        );
    }

    #[tokio::test]
    async fn producer_generation_drift_after_claim_is_a_durable_terminal() {
        let Some(db) =
            crate::test_utils::connect_test_database("exact_approval_post_claim_generation_drift")
                .await
        else {
            return;
        };
        let request = approval_request(
            "request-post-claim-generation",
            "approved",
            Utc::now() + Duration::minutes(5),
            Some(binding()),
        );
        db.collection::<ApprovalRequest>(REQUESTS)
            .insert_one(&request)
            .await
            .expect("insert post-claim drift fixture");
        let admitted_at = Utc::now();
        let claimed = claim_redemption(&db, &request, admitted_at)
            .await
            .expect("claim request")
            .expect("claim wins");
        let binding = claimed.exact_service.as_ref().unwrap();

        assert!(exact_authority_has_drift(
            binding,
            &ExactAuthoritySnapshot {
                operation_generation: binding.operation_generation + 1,
                ..authority_snapshot(binding)
            },
        ));
        let terminal = persist_redemption(
            &db,
            &request.id,
            ExactServiceApprovalRedemption {
                status: ExactServiceRedemptionStatus::Drifted,
                admitted_at,
                completed_at: Some(Utc::now()),
                receipt: None,
                failure_code: Some("catalog_drift".to_string()),
            },
        )
        .await
        .expect("persist post-claim generation drift");
        let observed = redemption_state(&terminal).expect("read durable terminal");
        assert_eq!(observed.state, ExactServiceApprovalState::Drifted);
        assert_eq!(observed.failure_code.as_deref(), Some("catalog_drift"));
    }

    #[tokio::test]
    async fn omitted_terminal_write_recovers_to_unknown_without_effect_replay() {
        let Some(db) =
            crate::test_utils::connect_test_database("exact_approval_stale_execution_recovery")
                .await
        else {
            return;
        };
        let state = crate::test_utils::test_app_state(db.clone());
        let requests = db.collection::<ApprovalRequest>(REQUESTS);
        let fresh_request = approval_request(
            "request-live-execution",
            "approved",
            Utc::now() + Duration::minutes(30),
            Some(binding()),
        );
        requests
            .insert_one(&fresh_request)
            .await
            .expect("insert live-execution fixture");
        let fresh_admitted_at = Utc::now();
        let fresh_claimed = claim_redemption(&db, &fresh_request, fresh_admitted_at)
            .await
            .expect("claim live-execution fixture")
            .expect("live claim wins");
        let still_live = recover_stale_execution(
            &db,
            fresh_claimed,
            fresh_admitted_at + Duration::seconds(EXECUTION_OUTCOME_RECOVERY_GRACE_SECS - 1),
        )
        .await
        .expect("fresh claim is not recovered early");
        assert_eq!(
            redemption_state(&still_live).unwrap().state,
            ExactServiceApprovalState::Redeeming,
            "a concurrent retry must not terminate a provider attempt inside the grace window"
        );

        let request = approval_request(
            "request-stale-execution",
            "approved",
            Utc::now() + Duration::minutes(30),
            Some(binding()),
        );
        requests
            .insert_one(&request)
            .await
            .expect("insert stale-execution fixture");

        let admitted_at = Utc::now() - Duration::seconds(EXECUTION_OUTCOME_RECOVERY_GRACE_SECS + 1);
        let claimed = claim_redemption(&db, &request, admitted_at)
            .await
            .expect("claim fault-injection fixture")
            .expect("claim wins");
        assert_eq!(
            redemption_state(&claimed).unwrap().state,
            ExactServiceApprovalState::Redeeming
        );

        // Fault injection: model a crash or Mongo failure after provider
        // dispatch by deliberately omitting the normal terminal receipt write.
        let retry = reload_after_lost_redemption_claim(&state, &caller(), &request.id)
            .await
            .expect("retry recovers stale executing state");
        assert_eq!(retry.state, ExactServiceApprovalState::Failed);
        assert_eq!(
            retry.failure_code.as_deref(),
            Some(PROVIDER_OUTCOME_UNKNOWN)
        );
        assert!(retry.receipt.is_none());

        let recovered = requests
            .find_one(doc! { "_id": &request.id })
            .await
            .expect("load recovered row")
            .expect("recovered row exists");
        assert!(
            claim_redemption(&db, &recovered, Utc::now())
                .await
                .expect("retry claim query")
                .is_none(),
            "an unknown provider outcome must never dispatch a second effect"
        );
        assert!(matches!(
            persist_redemption(
                &db,
                &request.id,
                ExactServiceApprovalRedemption {
                    status: ExactServiceRedemptionStatus::Completed,
                    admitted_at,
                    completed_at: Some(Utc::now()),
                    receipt: None,
                    failure_code: None,
                },
            )
            .await,
            Err(AppError::Conflict(message))
                if message == "exact_service_redemption_state_conflict"
        ));

        let second_retry = reload_after_lost_redemption_claim(&state, &caller(), &request.id)
            .await
            .expect("repeated retry replays durable unknown terminal");
        assert_eq!(second_retry.state, ExactServiceApprovalState::Failed);
        assert_eq!(
            second_retry.failure_code.as_deref(),
            Some(PROVIDER_OUTCOME_UNKNOWN)
        );
    }
}
