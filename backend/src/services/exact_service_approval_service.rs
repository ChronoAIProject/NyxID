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
    ExactServiceApprovalReceipt, ExactServiceApprovalRedemption, ExactServiceRedemptionStatus,
};
use crate::models::service_approval_config::ApprovalMode;
use crate::services::billing::route_inventory::BillingEgressPermit;
use crate::services::{approval_service, mcp_service, notification_service, proxy_service};

const MAX_RECEIPT_RESPONSE_BYTES: usize = 8 * 1024 * 1024;

#[derive(Clone, Debug)]
pub struct ExactServiceApprovalCaller {
    pub actor_user_id: String,
    pub proxy_resolution_user_id: String,
    pub approval_owner_user_id: String,
    pub requester_type: String,
    pub requester_id: String,
    pub requester_label: Option<String>,
    pub api_key_id: Option<String>,
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
    pub endpoint_contract_digest: String,
    pub operation_digest: String,
    pub operation_id: String,
    pub operation_generation: i64,
    pub idempotency_key: String,
    pub arguments: serde_json::Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ExactServiceApprovalFence {
    pub catalog_digest: String,
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
    endpoint_contract_digest: String,
    operation_digest: String,
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
    validate_create(&input)?;
    let resolution = resolve_exact_catalog(
        state,
        caller,
        &input.user_service_id,
        &input.endpoint_id,
        &input.arguments,
    )
    .await?;
    ensure_digest(
        "catalog_digest",
        &input.catalog_digest,
        &resolution.catalog_digest,
    )?;
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

    let service = resolution.service();
    let endpoint = resolution.endpoint();
    let descriptor =
        mcp_service::build_mcp_operation_descriptor(service, endpoint, &input.arguments)?;
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
    let request_key = request_key(caller, &input);
    let binding = ExactServiceApprovalBinding {
        request_key,
        actor_user_id: caller.actor_user_id.clone(),
        user_service_id: input.user_service_id,
        endpoint_id: input.endpoint_id,
        catalog_digest: resolution.catalog_digest,
        endpoint_contract_digest: resolution.endpoint_contract_digest,
        operation_digest: resolution.operation_digest,
        operation_id: input.operation_id,
        operation_generation: input.operation_generation,
        effect_idempotency_key: input.idempotency_key,
        arguments: input.arguments,
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
    let request = load_bound_request(state, caller, request_id).await?;
    let request = expire_if_needed(state, request).await?;
    let state_value = current_state(state, caller, &request).await?;
    result_for(&request, state_value)
}

pub async fn redeem_request(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    request_id: &str,
    fence: ExactServiceApprovalFence,
    billing_egress_permit: BillingEgressPermit,
) -> AppResult<ExactServiceApprovalResult> {
    let request = load_bound_request(state, caller, request_id).await?;
    ensure_fence(&request, &fence)?;
    let request = expire_if_needed(state, request).await?;
    let state_value = current_state(state, caller, &request).await?;
    match state_value {
        ExactServiceApprovalState::Redeemed
        | ExactServiceApprovalState::Redeeming
        | ExactServiceApprovalState::Failed => return result_for(&request, state_value),
        ExactServiceApprovalState::Approved => {}
        other => return result_for(&request, other),
    }

    let now = Utc::now();
    let claimed = claim_redemption(&state.db, &request, now).await?;

    let claimed = match claimed {
        Some(claimed) => claimed,
        None => {
            let replay = load_bound_request(state, caller, request_id).await?;
            return result_for(&replay, redemption_state(&replay)?);
        }
    };

    let binding = claimed.exact_service.as_ref().unwrap();
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
        Err(error) => {
            let (terminal_state, status, failure_code) =
                match catalog_resolution_terminal_state(&error) {
                    Some(ExactServiceApprovalState::Revoked) => (
                        ExactServiceApprovalState::Revoked,
                        ExactServiceRedemptionStatus::Revoked,
                        "selector_revoked",
                    ),
                    Some(ExactServiceApprovalState::Drifted) => (
                        ExactServiceApprovalState::Drifted,
                        ExactServiceRedemptionStatus::Drifted,
                        "catalog_drift",
                    ),
                    _ => (
                        ExactServiceApprovalState::Failed,
                        ExactServiceRedemptionStatus::Failed,
                        safe_execution_failure_code(&error),
                    ),
                };
            let updated = persist_redemption(
                &state.db,
                request_id,
                ExactServiceApprovalRedemption {
                    status,
                    admitted_at: now,
                    completed_at: Some(Utc::now()),
                    receipt: None,
                    failure_code: Some(failure_code.to_string()),
                },
            )
            .await?;
            return result_for(&updated, terminal_state);
        }
    };
    if exact_authority_has_drift(
        binding,
        &resolution.service().service_id,
        &resolution.endpoint().endpoint_id,
        &resolution.catalog_digest,
        &resolution.endpoint_contract_digest,
        &resolution.operation_digest,
    ) {
        let updated = persist_redemption(
            &state.db,
            request_id,
            ExactServiceApprovalRedemption {
                status: ExactServiceRedemptionStatus::Drifted,
                admitted_at: now,
                completed_at: Some(Utc::now()),
                receipt: None,
                failure_code: Some("catalog_drift".to_string()),
            },
        )
        .await?;
        return result_for(&updated, ExactServiceApprovalState::Drifted);
    }
    let exec_ctx = mcp_service::McpExecContext {
        api_key_id: caller.api_key_id.as_deref(),
        allow_all_nodes: caller.allow_all_nodes,
        allowed_node_ids: &caller.allowed_node_ids,
    };
    let executed = mcp_service::execute_tool(
        &state.http_client,
        &state.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &state.billing,
        &caller.proxy_resolution_user_id,
        &caller.proxy_resolution_user_id,
        resolution.service(),
        resolution.endpoint(),
        &binding.arguments,
        &state.jwt_keys,
        &state.config,
        &state.connection_expiry_notifier,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        &exec_ctx,
        billing_egress_permit,
    )
    .await;

    let completed_at = Utc::now();
    let (redemption, terminal_state) = match executed {
        Ok((http_status, response_body)) => {
            let response_digest = format!(
                "sha256:{}",
                hex::encode(Sha256::digest(response_body.as_bytes()))
            );
            if response_body.len() > MAX_RECEIPT_RESPONSE_BYTES {
                (
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
                    },
                    ExactServiceApprovalState::Failed,
                )
            } else {
                (
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
                    },
                    ExactServiceApprovalState::Redeemed,
                )
            }
        }
        Err(error) => (
            ExactServiceApprovalRedemption {
                status: ExactServiceRedemptionStatus::Failed,
                admitted_at: now,
                completed_at: Some(completed_at),
                receipt: None,
                failure_code: Some(safe_execution_failure_code(&error).to_string()),
            },
            ExactServiceApprovalState::Failed,
        ),
    };
    let updated = persist_redemption(&state.db, request_id, redemption).await?;
    result_for(&updated, terminal_state)
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
    // Eligibility is deliberately NOT narrowed to the exact-visible view.
    // Generic-proxy operations are an approvable, shipped capability -- see
    // billing_integration_tests::billing_route_coverage_smoke, which approves
    // and redeems `nyx_generic_proxy_v1`. Hiding generic operations from
    // delegated discovery is about not advertising untyped operations, not an
    // authorization boundary: a caller already scoped to the service can reach
    // it through the generic proxy regardless.
    let endpoint_index = catalog.services[service_index]
        .endpoints
        .iter()
        .position(|endpoint| endpoint.endpoint_id == endpoint_id)
        .ok_or_else(|| AppError::NotFound("exact_endpoint_not_found".to_string()))?;
    // Whole-catalog fence, matching `/api/v1/mcp/config` and delegated
    // discovery. All three must hash the same bytes or a digest obtained from
    // one surface is rejected as drift by another.
    let catalog_digest = mcp_service::operation_catalog_digest(&catalog.services);
    let endpoint = &catalog.services[service_index].endpoints[endpoint_index];
    let endpoint_contract_digest = mcp_service::endpoint_contract_digest(endpoint);
    let operation_digest =
        mcp_service::exact_operation_digest(user_service_id, endpoint, arguments);
    Ok(ExactCatalogResolution {
        catalog,
        service_index,
        endpoint_index,
        catalog_digest,
        endpoint_contract_digest,
        operation_digest,
        marker: std::marker::PhantomData,
    })
}

#[derive(Clone, Debug)]
struct ApprovalTarget {
    service_id: String,
    service_name: String,
    service_slug: String,
    service_owner_user_id: String,
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
    if request.status != "pending" || request.expires_at > Utc::now() {
        return Ok(request);
    }
    let now = Utc::now();
    let updated = state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one_and_update(
            doc! { "_id": &request.id, "status": "pending", "expires_at": { "$lte": bson::DateTime::from_chrono(now) } },
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

async fn current_state(
    state: &AppState,
    caller: &ExactServiceApprovalCaller,
    request: &ApprovalRequest,
) -> AppResult<ExactServiceApprovalState> {
    match request.status.as_str() {
        "pending" => return Ok(ExactServiceApprovalState::Pending),
        "rejected" => return Ok(ExactServiceApprovalState::Denied),
        "expired" => return Ok(ExactServiceApprovalState::Expired),
        "approved" => {}
        _ => return Ok(ExactServiceApprovalState::Revoked),
    }
    if request
        .exact_service
        .as_ref()
        .and_then(|binding| binding.redemption.as_ref())
        .is_some()
    {
        return redemption_state(request);
    }
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
        Err(error) => match catalog_resolution_terminal_state(&error) {
            Some(state) => return Ok(state),
            None => return Err(error),
        },
    };
    if exact_authority_has_drift(
        binding,
        &resolution.service().service_id,
        &resolution.endpoint().endpoint_id,
        &resolution.catalog_digest,
        &resolution.endpoint_contract_digest,
        &resolution.operation_digest,
    ) {
        return Ok(ExactServiceApprovalState::Drifted);
    }
    let descriptor = mcp_service::build_mcp_operation_descriptor(
        resolution.service(),
        resolution.endpoint(),
        &binding.arguments,
    )?;
    let target = approval_target(state, caller, resolution.service()).await?;
    match approval_service::evaluate_and_check(
        &state.db,
        &caller.approval_owner_user_id,
        &target.service_owner_user_id,
        &target.service_id,
        &descriptor,
        Some(&caller.requester_type),
        &caller.requester_id,
        false,
    )
    .await?
    {
        approval_service::ApprovalOutcome::NeedsApproval(pending)
            if pending.resolution.mode == ApprovalMode::PerRequest =>
        {
            Ok(ExactServiceApprovalState::Approved)
        }
        _ => Ok(ExactServiceApprovalState::Revoked),
    }
}

fn redemption_state(request: &ApprovalRequest) -> AppResult<ExactServiceApprovalState> {
    let redemption = request
        .exact_service
        .as_ref()
        .and_then(|binding| binding.redemption.as_ref())
        .ok_or_else(|| AppError::Conflict("exact_service_redemption_missing".to_string()))?;
    Ok(match redemption.status {
        ExactServiceRedemptionStatus::Executing => ExactServiceApprovalState::Redeeming,
        ExactServiceRedemptionStatus::Completed => ExactServiceApprovalState::Redeemed,
        ExactServiceRedemptionStatus::Drifted => ExactServiceApprovalState::Drifted,
        ExactServiceRedemptionStatus::Revoked => ExactServiceApprovalState::Revoked,
        ExactServiceRedemptionStatus::Failed => ExactServiceApprovalState::Failed,
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

fn exact_authority_has_drift(
    binding: &ExactServiceApprovalBinding,
    user_service_id: &str,
    endpoint_id: &str,
    catalog_digest: &str,
    endpoint_contract_digest: &str,
    operation_digest: &str,
) -> bool {
    binding.user_service_id != user_service_id
        || binding.endpoint_id != endpoint_id
        || binding.catalog_digest != catalog_digest
        || binding.endpoint_contract_digest != endpoint_contract_digest
        || binding.operation_digest != operation_digest
}

fn ensure_fence(request: &ApprovalRequest, fence: &ExactServiceApprovalFence) -> AppResult<()> {
    let binding = request.exact_service.as_ref().unwrap();
    if binding.catalog_digest != fence.catalog_digest
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
    if input.operation_generation < 1 {
        return Err(AppError::BadRequest(
            "operation_generation must be positive".to_string(),
        ));
    }
    if !input.arguments.is_object() {
        return Err(AppError::BadRequest(
            "arguments must be a JSON object".to_string(),
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

fn request_key(caller: &ExactServiceApprovalCaller, input: &ExactServiceApprovalCreate) -> String {
    mcp_service::canonical_sha256(serde_json::json!({
        "contract_version": "nyxid-exact-approval-request.v1",
        "requester_type": caller.requester_type,
        "requester_id": caller.requester_id,
        "actor_user_id": caller.actor_user_id,
        "operation_id": input.operation_id,
        "operation_generation": input.operation_generation,
        "idempotency_key": input.idempotency_key,
    }))
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
            allow_all_services: true,
            allowed_service_ids: vec![],
            allow_all_nodes: true,
            allowed_node_ids: vec![],
        }
    }

    fn create() -> ExactServiceApprovalCreate {
        ExactServiceApprovalCreate {
            user_service_id: "service-alpha".to_string(),
            endpoint_id: "endpoint-alpha".to_string(),
            catalog_digest: "sha256:catalog".to_string(),
            endpoint_contract_digest: "sha256:contract".to_string(),
            operation_digest: "sha256:operation".to_string(),
            operation_id: "operation-alpha".to_string(),
            operation_generation: 3,
            idempotency_key: "idempotency-alpha".to_string(),
            arguments: serde_json::json!({"value": 1}),
        }
    }

    fn binding() -> ExactServiceApprovalBinding {
        let input = create();
        ExactServiceApprovalBinding {
            request_key: request_key(&caller(), &input),
            actor_user_id: caller().actor_user_id,
            user_service_id: input.user_service_id,
            endpoint_id: input.endpoint_id,
            catalog_digest: input.catalog_digest,
            endpoint_contract_digest: input.endpoint_contract_digest,
            operation_digest: input.operation_digest,
            operation_id: input.operation_id,
            operation_generation: input.operation_generation,
            effect_idempotency_key: input.idempotency_key,
            arguments: input.arguments,
            redemption: None,
        }
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
            request_key(&caller, &first),
            request_key(&caller, &conflicting_selector)
        );
    }

    #[test]
    fn redemption_fence_rejects_conflicting_replay() {
        let input = create();
        let request = approval_request(
            "request-alpha",
            "approved",
            Utc::now() + Duration::minutes(5),
            Some(binding()),
        );
        let mut fence = ExactServiceApprovalFence {
            catalog_digest: input.catalog_digest,
            operation_digest: input.operation_digest,
            operation_id: input.operation_id,
            operation_generation: input.operation_generation,
            idempotency_key: input.idempotency_key,
        };
        ensure_fence(&request, &fence).unwrap();
        fence.operation_generation += 1;
        assert!(matches!(
            ensure_fence(&request, &fence),
            Err(AppError::Conflict(_))
        ));
    }

    #[test]
    fn drift_and_revocation_are_typed_before_effect_dispatch() {
        let binding = binding();
        assert!(!exact_authority_has_drift(
            &binding,
            &binding.user_service_id,
            &binding.endpoint_id,
            &binding.catalog_digest,
            &binding.endpoint_contract_digest,
            &binding.operation_digest,
        ));
        assert!(exact_authority_has_drift(
            &binding,
            &binding.user_service_id,
            &binding.endpoint_id,
            &binding.catalog_digest,
            "sha256:changed-contract",
            &binding.operation_digest,
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
            .insert_many([denied, expired, drifted, revoked])
            .await
            .expect("insert terminal fixtures");

        for (request_id, expected) in [
            ("request-denied", ExactServiceApprovalState::Denied),
            ("request-expired", ExactServiceApprovalState::Expired),
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
}
