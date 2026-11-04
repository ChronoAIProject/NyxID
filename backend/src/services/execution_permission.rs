//! Pure contract evaluator. Deliberately not called by any execution dispatcher.
//! Inputs must be server-resolved snapshots, never model/caller assertions.
#![allow(dead_code)] // Reader-first contract; dispatch integration is #2194.

use chrono::{DateTime, TimeDelta, Utc};

use super::{agent_operation_scope_service as scopes, proxy_authorization::CanonicalPath};
use crate::models::{approval_grant::ApprovalGrant, execution_permission::*};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Gap {
    MissingPermission,
    Subject,
    Binding,
    Context,
    Operation,
    Validity,
    Floor(ExecutionFloor),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Prohibition {
    Ceiling,
    Guest,
    UnsupportedVersion,
    InvalidContract,
    Revoked,
    Task,
    ClaimUsed,
    Resource,
    IneligibleDecider,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Decision {
    /// Coverage is not dispatch authorization. Exact/durable claims, live
    /// fences and provider replay rules must still be enforced by the caller.
    Covered {
        operation_claim_required: bool,
    },
    RequestableGap(Gap),
    Prohibited(Prohibition),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvaluationError {
    AuthorityUnavailable,
    ResourceVerificationRequired,
    OutcomeUncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LiveAuthority {
    Allowed,
    Denied,
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ClaimState {
    Available,
    Used,
    Uncertain,
}

/// Loaded by trusted admission/recovery code from an existing root and its
/// recorded descendant edge. TaskRef alone must never populate this evidence.
pub struct TaskEvidence<'a> {
    pub task: &'a TaskRef,
    pub actor_user_id: &'a str,
    pub audience: &'a Audience,
    pub hard_deadline: DateTime<Utc>,
    pub active: bool,
    pub admitted_descendant: bool,
}

/// Result of the existing adapter's live checks, bound to the actual prepared
/// effect and authority. No adapter or network is invoked by this evaluator.
pub struct ResourceEvidence<'a> {
    pub policy_digest: &'a str,
    pub prepared_digest: &'a str,
    pub authority_digest: &'a str,
    pub allowed: bool,
}

pub struct PreparedOperation<'a> {
    pub id: &'a str,
    pub digest: &'a str,
    pub catalog_service_id: Option<&'a str>,
    pub endpoint_id: Option<&'a str>,
    pub method: &'a str,
    pub path: &'a CanonicalPath,
    pub query: Option<&'a str>,
    pub body: &'a [u8],
    pub content_type: Option<&'a str>,
}

pub struct Request<'a> {
    pub now: DateTime<Utc>,
    pub subject: &'a Subject,
    pub binding: &'a ConnectionBinding,
    pub audience: &'a Audience,
    pub conversation_id: &'a str,
    pub operation: PreparedOperation<'a>,
    pub task: Option<TaskEvidence<'a>>,
    pub resource: Option<ResourceEvidence<'a>>,
    pub floors: &'a [ExecutionFloor],
    pub authority: LiveAuthority,
    pub eligible_decider: bool,
    pub claim: ClaimState,
    /// Each parent/child ceiling must cover this same effect. Never combine
    /// fragments into broader authority or infer permission from these ceilings.
    pub delegation_ceilings: &'a [crate::models::agent_operation_scope::AgentOperationScope],
}

pub fn satisfiable(floor: ExecutionFloor) -> bool {
    matches!(
        floor,
        ExecutionFloor::AccessModeRisk
            | ExecutionFloor::PersonalServiceApproval
            | ExecutionFloor::SharedConnectionConsent
    )
}

pub fn legacy_expiry(validity: &Validity) -> DateTime<Utc> {
    match validity {
        Validity::Operation { expires_at, .. } => *expires_at,
        Validity::Task { hard_deadline, .. } => *hard_deadline,
        Validity::Window { ends_at, .. } => *ends_at,
        Validity::Persistent => {
            DateTime::from_timestamp(PERSISTENT_TTL_SECONDS, 0).expect("documented UTC sentinel")
        }
    }
}

pub fn terminal_expiry(now: DateTime<Utc>) -> DateTime<Utc> {
    now.checked_add_signed(TimeDelta::seconds(TERMINAL_RETENTION_SECONDS))
        .unwrap_or(DateTime::<Utc>::MAX_UTC)
}

/// Canonical existing execution projection excludes OAuth refresh token bytes.
pub fn connection_binding(
    owner: &str,
    projection: &super::execution_authority::ExecutionAuthorityProjection,
    behavior_digest: Option<String>,
) -> ConnectionBinding {
    ConnectionBinding {
        connection_id: projection.user_service_id.clone(),
        owner_user_id: owner.into(),
        credential_id: projection
            .credential
            .override_api_key_id
            .clone()
            .or(projection.credential.api_key_id.clone()),
        credential_epoch: projection
            .credential
            .override_epoch
            .unwrap_or(projection.credential.credential_epoch),
        authority_digest: super::execution_authority::digest(projection),
        behavior_digest,
    }
}

fn gap(request: &Request<'_>, reason: Gap) -> Decision {
    if request.eligible_decider {
        Decision::RequestableGap(reason)
    } else {
        Decision::Prohibited(Prohibition::IneligibleDecider)
    }
}

fn operation_covered(
    scope: &crate::models::agent_operation_scope::AgentOperationScope,
    request: &Request<'_>,
) -> bool {
    let op = &request.operation;
    if scope.catalog_service_id.as_deref() != op.catalog_service_id || scope.operations.is_empty() {
        return false;
    }
    let scopes =
        std::collections::BTreeMap::from([(request.binding.connection_id.clone(), scope.clone())]);
    scopes::authorize(
        &scopes,
        &request.binding.connection_id,
        op.catalog_service_id,
        op.endpoint_id,
        op.method,
        op.path,
        false,
        false,
    )
    .is_ok()
        && scopes::check_inputs(
            &scopes,
            &request.binding.connection_id,
            op.catalog_service_id,
            op.endpoint_id,
            op.method,
            op.path,
            op.query,
            op.body,
            op.content_type,
        )
        .is_ok()
}

fn preflight(request: &Request<'_>) -> Result<Option<Decision>, EvaluationError> {
    if request.subject.lane == ExecutionLane::Guest {
        return Ok(Some(Decision::Prohibited(Prohibition::Guest)));
    }
    match request.authority {
        LiveAuthority::Denied => return Ok(Some(Decision::Prohibited(Prohibition::Ceiling))),
        LiveAuthority::Unavailable => return Err(EvaluationError::AuthorityUnavailable),
        LiveAuthority::Allowed => {}
    }
    if request
        .delegation_ceilings
        .iter()
        .any(|scope| !operation_covered(scope, request))
    {
        return Ok(Some(Decision::Prohibited(Prohibition::Ceiling)));
    }
    Ok(None)
}

fn valid_contract(permission: &ExecutionContract) -> bool {
    let principal_id = match &permission.subject.principal {
        Principal::Agent(id)
        | Principal::ApiKey(id)
        | Principal::OauthClient(id)
        | Principal::ServiceAccount(id) => id,
    };
    let audience_valid = match &permission.context.audience {
        Audience::Private { user_id } => user_id == &permission.subject.actor_user_id,
        Audience::Group { group_id, revision } => !group_id.is_empty() && *revision > 0,
        Audience::Channel {
            binding_id,
            revision,
        } => !binding_id.is_empty() && *revision > 0,
    };
    let validity_valid = match &permission.validity {
        Validity::Operation {
            operation_id,
            prepared_digest,
            ..
        } => !operation_id.is_empty() && !prepared_digest.is_empty(),
        Validity::Task { task, .. } => !task.root_id.is_empty(),
        Validity::Window { starts_at, ends_at } => starts_at < ends_at,
        Validity::Persistent => true,
    };
    !principal_id.is_empty()
        && audience_valid
        && validity_valid
        && permission.operations.revision > 0
        && permission
            .binding
            .credential_id
            .as_ref()
            .is_none_or(|id| !id.is_empty())
        && !matches!(&permission.context.conversation, ConversationScope::Exact { conversation_id } if conversation_id.is_empty())
}

pub fn evaluate(grant: &ApprovalGrant, request: &Request<'_>) -> Result<Decision, EvaluationError> {
    if let Some(decision) = preflight(request)? {
        return Ok(decision);
    }
    let Some(extension) = &grant.execution else {
        return Ok(gap(request, Gap::MissingPermission));
    };
    if extension.version != VERSION {
        return Ok(Decision::Prohibited(Prohibition::UnsupportedVersion));
    }
    let Some(permission) = &extension.v1 else {
        return Ok(Decision::Prohibited(Prohibition::InvalidContract));
    };
    if grant.revoked {
        return Ok(Decision::Prohibited(Prohibition::Revoked));
    }
    if !valid_contract(permission)
        || grant.scope.as_deref() != Some(LEGACY_SCOPE)
        || permission.revision < 1
        || permission.subject.actor_user_id.is_empty()
        || permission.binding.connection_id.is_empty()
        || permission.binding.owner_user_id.is_empty()
        || permission.binding.authority_digest.is_empty()
        || permission.binding.credential_epoch < 0
        || permission.satisfies.is_empty()
        || permission
            .satisfies
            .iter()
            .any(|floor| !satisfiable(*floor))
        || (permission.subject.lane == ExecutionLane::Shared
            && permission
                .binding
                .behavior_digest
                .as_ref()
                .is_none_or(String::is_empty))
        || grant.expires_at != legacy_expiry(&permission.validity)
    {
        return Ok(Decision::Prohibited(Prohibition::InvalidContract));
    }
    if &permission.subject != request.subject {
        return Ok(gap(request, Gap::Subject));
    }
    if &permission.binding != request.binding {
        return Ok(gap(request, Gap::Binding));
    }
    if &permission.context.audience != request.audience
        || matches!(&permission.context.conversation,
        ConversationScope::Exact {conversation_id} if conversation_id != request.conversation_id)
    {
        return Ok(gap(request, Gap::Context));
    }
    if !operation_covered(&permission.operations, request) {
        return Ok(gap(request, Gap::Operation));
    }
    match &permission.validity {
        Validity::Operation {
            operation_id,
            prepared_digest,
            expires_at,
        } => {
            if operation_id != request.operation.id
                || prepared_digest != request.operation.digest
                || request.now >= *expires_at
            {
                return Ok(gap(request, Gap::Validity));
            }
            match request.claim {
                ClaimState::Available => {}
                ClaimState::Used => return Ok(Decision::Prohibited(Prohibition::ClaimUsed)),
                ClaimState::Uncertain => return Err(EvaluationError::OutcomeUncertain),
            }
        }
        Validity::Task {
            task,
            hard_deadline,
        } => {
            if request.task.as_ref().is_none_or(|evidence| {
                evidence.task != task
                    || !evidence.active
                    || !evidence.admitted_descendant
                    || evidence.actor_user_id != request.subject.actor_user_id
                    || evidence.audience != request.audience
                    || *hard_deadline > evidence.hard_deadline
                    || request.now >= *hard_deadline
                    || task.root_id.is_empty()
            }) {
                return Ok(Decision::Prohibited(Prohibition::Task));
            }
        }
        Validity::Window { starts_at, ends_at }
            if ends_at <= starts_at || request.now < *starts_at || request.now >= *ends_at =>
        {
            return Ok(gap(request, Gap::Validity));
        }
        _ => {}
    }
    if let Some(policy) = &permission.resource_policy {
        if policy.version != 1
            || nyxid_permissions::adapter_for_policy(policy)
                .and_then(|adapter| adapter.validate_policy(policy))
                .is_err()
        {
            return Ok(Decision::Prohibited(Prohibition::InvalidContract));
        }
        let digest = super::mcp_service::canonical_sha256(
            serde_json::to_value(policy).expect("policy serialization"),
        );
        let Some(evidence) = &request.resource else {
            return Err(EvaluationError::ResourceVerificationRequired);
        };
        if !evidence.allowed
            || evidence.policy_digest != digest
            || evidence.prepared_digest != request.operation.digest
            || evidence.authority_digest != request.binding.authority_digest
        {
            return Ok(Decision::Prohibited(Prohibition::Resource));
        }
    }
    for floor in request.floors {
        if !satisfiable(*floor) || !permission.satisfies.contains(floor) {
            return Ok(gap(request, Gap::Floor(*floor)));
        }
        if *floor == ExecutionFloor::SharedConnectionConsent
            && (request.subject.lane != ExecutionLane::Shared
                || request.binding.owner_user_id != request.subject.actor_user_id)
        {
            return Ok(Decision::Prohibited(Prohibition::Ceiling));
        }
        if *floor == ExecutionFloor::PersonalServiceApproval
            && request.binding.owner_user_id != request.subject.actor_user_id
        {
            return Ok(Decision::Prohibited(Prohibition::Ceiling));
        }
    }
    Ok(Decision::Covered {
        operation_claim_required: matches!(permission.validity, Validity::Operation { .. }),
    })
}

/// Prefer fully covering reusable authority over an operation claim. Each row
/// must independently cover the whole effect; permissions are never spliced.
pub fn evaluate_candidates<'a>(
    grants: &'a [ApprovalGrant],
    request: &Request<'_>,
) -> Result<(Option<&'a str>, Decision), EvaluationError> {
    if let Some(decision) = preflight(request)? {
        return Ok((None, decision));
    }
    let mut candidates: Vec<_> = grants.iter().collect();
    candidates.sort_by_key(|grant| {
        match grant
            .execution
            .as_ref()
            .and_then(|e| e.v1.as_ref())
            .map(|p| &p.validity)
        {
            Some(Validity::Persistent) => 0,
            Some(Validity::Window { .. }) => 1,
            Some(Validity::Task { .. }) => 2,
            _ => 3,
        }
    });
    let mut error = None;
    let mut requestable = None;
    let mut prohibited = None;
    for grant in candidates {
        let decision = evaluate(grant, request);
        if let Ok(covered @ Decision::Covered { .. }) = decision {
            return Ok((Some(&grant.id), covered));
        }
        match decision {
            Err(value) => {
                error.get_or_insert(value);
            }
            Ok(value @ Decision::RequestableGap(_)) => {
                requestable.get_or_insert(value);
            }
            Ok(value @ Decision::Prohibited(_)) => {
                prohibited.get_or_insert(value);
            }
            Ok(Decision::Covered { .. }) => unreachable!("covered returned above"),
        }
    }
    // A fully verified candidate wins. Otherwise an operational failure cannot
    // be converted to a prompt; an eligible gap precedes unrelated invalid rows.
    if let Some(error) = error {
        return Err(error);
    }
    Ok((
        None,
        requestable
            .or(prohibited)
            .unwrap_or_else(|| gap(request, Gap::MissingPermission)),
    ))
}

#[cfg(test)]
#[path = "execution_permission_tests.rs"]
pub(crate) mod tests;
