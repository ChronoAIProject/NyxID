use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, Document, doc};
use mongodb::options::{FindOptions, ReturnDocument};
use mongodb::{ClientSession, Database};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Arc;

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::approval_grant::{ApprovalGrant, COLLECTION_NAME as GRANTS};
use crate::models::approval_request::{
    ApprovalRequest, COLLECTION_NAME as REQUESTS, ExactServiceApprovalBinding,
};
use crate::models::notification_channel::{COLLECTION_NAME as CHANNELS, NotificationChannel};
use crate::models::security_effect_intent::SecurityEffectIntent;
use crate::models::service_approval_config::{
    ApprovalEffect, ApprovalMode, ApprovalRule, COLLECTION_NAME as SERVICE_APPROVAL_CONFIGS,
    ServiceApprovalConfig,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::services::api_key_mutation_service::{map_transaction_error, transaction_result};
use crate::services::approval_policy;
use crate::services::audit_service::{AuditActor, required};
use crate::services::notification_service;
use crate::services::operation_descriptor::OperationDescriptor;
use crate::services::push_service::{ApnsAuth, FcmAuth};
use crate::telemetry::{TelemetryClient, TelemetryContext, TelemetryEvent, emit_event};

/// Caller interaction capability, independent of authorization and transport.
/// Disabling requests never bypasses policy or invalidates an existing grant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ApprovalOptions {
    pub allow_request: bool,
}

impl Default for ApprovalOptions {
    fn default() -> Self {
        Self {
            allow_request: true,
        }
    }
}

impl ApprovalOptions {
    pub const NO_REQUEST: Self = Self {
        allow_request: false,
    };

    pub fn require_request_support(self) -> AppResult<()> {
        if self.allow_request {
            Ok(())
        } else {
            Err(AppError::ApprovalUnsupported)
        }
    }
}

/// Resolve the approval mode for a (user, service) pair.
async fn resolve_approval_mode(
    db: &Database,
    user_id: &str,
    service_id: &str,
    session: &mut ClientSession,
) -> AppResult<ApprovalMode> {
    let per_service = db
        .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
        .find_one(doc! { "user_id": user_id, "service_id": service_id })
        .session(session)
        .await?;

    Ok(per_service
        .filter(|c| !c.disabled)
        .map(|c| c.approval_mode)
        .unwrap_or_default())
}

/// Resolve the explicit preference without materializing lazy notification settings.
pub async fn user_approval_mode(
    db: &Database,
    user_id: &str,
) -> AppResult<approval_policy::GlobalApprovalMode> {
    Ok(approval_policy::GlobalApprovalMode::from_stored(
        user_global_approval_setting(db, user_id).await?,
    ))
}

/// Notification delivery never authorizes an otherwise gated effect. The
/// unset preference allows reads and requires approval for writes/destruction.
pub async fn user_requires_approval(
    db: &Database,
    user_id: &str,
    verb: &crate::models::service_approval_config::ApprovalVerb,
) -> AppResult<bool> {
    Ok(user_approval_mode(db, user_id).await?.requires(verb))
}

// `requires_approval_for_service` was removed in favor of the org-aware
// `resolve_org_aware_approval` below. Callers should use that function so
// the org-policy cascade applies for org-shared services.

/// Outcome of resolving the approval policy for a proxy call. Captures
/// who the request "belongs" to (`primary_owner_user_id`), what mode it
/// runs in, and whether it triggered at all.
///
/// **Resolution semantics** (used by [`resolve_org_aware_approval`]):
///
/// 1. If the service is **org-owned** AND the org has set a per-service
///    `ServiceApprovalConfig`, that config wins absolutely. The org admin
///    has made an explicit choice for the shared resource. The request
///    `primary_owner_user_id` is the org's user_id; grants live under the
///    org so the next call from the same actor reuses it.
/// 2. Otherwise -- personal service, OR org-owned without an org policy --
///    the actor's per-service config or active global setting applies.
///    The request `primary_owner_user_id` is the actor.
/// 3. For an auto-connected `UserService`, the global setting is not an
///    implicit fallback. An explicit actor/org config still wins as above.
///
/// This is the cleanest semantic: the resource owner's policy is
/// authoritative when set, and falls back to the actor's preference when
/// the owner has not configured one.
#[derive(Debug, Clone)]
pub struct ApprovalResolution {
    pub fresh_decision: bool,
    pub required: bool,
    pub mode: ApprovalMode,
    pub effect: ApprovalEffect,
    pub grant_scope: Option<String>,
    /// User the request is created under. For org-policy requests this
    /// is the org user_id; for actor-policy requests this is the actor.
    pub primary_owner_user_id: String,
    /// True when resolution came from the org's per-service config rather
    /// than the actor's settings. The proxy handler uses this to populate
    /// `notify_user_ids` with the org's admin list instead of `[actor]`.
    pub from_org_policy: bool,
}

#[derive(Debug, Clone)]
pub struct PendingApproval {
    pub options: ApprovalOptions,
    pub resolution: ApprovalResolution,
    pub primary_owner_user_id: String,
    pub requester_type: String,
    pub requester_id: String,
}

#[derive(Debug, Clone)]
pub enum ApprovalOutcome {
    Allowed { required: bool },
    Denied,
    NeedsApproval(PendingApproval),
}

/// Resolve the effective approval policy for a proxy call, accounting for
/// org-owned services that may carry their own per-service approval config.
///
/// `actor_user_id` is the human/API key making the call. `service_owner_user_id`
/// is the user_id that owns the resolved `UserService` -- the actor for
/// personal services, the org for org-shared ones.
/// `api_key_id` explicitly identifies the calling NyxID key, or is `None`
/// when the caller has no key. Org policy evaluation retains its existing semantics.
pub async fn resolve_org_aware_approval(
    db: &Database,
    actor_user_id: &str,
    service_owner_user_id: &str,
    service_id: &str,
    descriptor: &OperationDescriptor,
    is_auto_connected: bool,
    api_key_id: Option<&str>,
) -> AppResult<ApprovalResolution> {
    // Step 1: if the resolved service is org-owned and the org has a
    // policy, use it. We detect "org-owned" by looking up the owner's
    // `user_type`, NOT by comparing `actor_user_id` to
    // `service_owner_user_id` -- for org-owned NyxID API keys and
    // service accounts, both are the org id, but the request still
    // needs to fan out to the org's admins instead of being treated as
    // a self-decided personal request.
    let service_owner_is_org = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": service_owner_user_id })
        .await?
        .is_some_and(|u| u.user_type.is_org());
    if service_owner_is_org
        && let Some(org_config) = db
            .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
            .find_one(doc! {
                "user_id": service_owner_user_id,
                "service_id": service_id,
                "disabled": {"$ne":true},
            })
            .await?
    {
        // Org rules retain the keyless cascade in this phase. The caller key
        // selects only actor rules until the separate org-policy rollout (P1b).
        let decision = approval_policy::evaluate(&org_config, descriptor, None).unwrap_or(
            approval_policy::ApprovalPolicyDecision {
                effect: ApprovalEffect::AutoAllow,
                mode: org_config.approval_mode.clone(),
                grant_scope: None,
            },
        );
        return Ok(ApprovalResolution {
            fresh_decision: false,
            required: decision.effect == ApprovalEffect::RequireApproval,
            mode: decision.mode,
            effect: decision.effect,
            grant_scope: decision.grant_scope,
            primary_owner_user_id: service_owner_user_id.to_string(),
            from_org_policy: true,
        });
    }

    // Step 2: fall back to the actor's policy (existing behavior).
    let actor_config = db
        .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
        .find_one(doc! { "user_id": actor_user_id, "service_id": service_id })
        .await?;

    let actor_decision = actor_config
        .as_ref()
        .and_then(|cfg| approval_policy::evaluate(cfg, descriptor, api_key_id));
    let (required, mode, effect, grant_scope) = if let Some(decision) = actor_decision {
        (
            decision.effect == ApprovalEffect::RequireApproval,
            decision.mode,
            decision.effect,
            decision.grant_scope,
        )
    } else {
        // Auto-connected services are platform-managed and deliberately have
        // no user credential. The global notification setting is an implicit
        // fallback, so it must not turn these services into approval gates.
        // Explicit actor/org ServiceApprovalConfig rows are handled above and
        // still apply verbatim.
        let required = if is_auto_connected {
            false
        } else {
            user_requires_approval(db, actor_user_id, &descriptor.verb).await?
        };
        (
            required,
            ApprovalMode::default(),
            if required {
                ApprovalEffect::RequireApproval
            } else {
                ApprovalEffect::AutoAllow
            },
            None,
        )
    };

    Ok(ApprovalResolution {
        fresh_decision: actor_config.as_ref().is_some_and(|cfg| {
            approval_policy::key_requires_fresh_decision(cfg, descriptor, api_key_id)
        }),
        required,
        mode,
        effect,
        grant_scope,
        primary_owner_user_id: actor_user_id.to_string(),
        from_org_policy: false,
    })
}

pub async fn evaluate_deny_only(
    db: &Database,
    actor_user_id: &str,
    service_owner_user_id: &str,
    service_id: &str,
    descriptor: &OperationDescriptor,
    is_auto_connected: bool,
    api_key_id: Option<&str>,
) -> AppResult<bool> {
    let resolution = resolve_org_aware_approval(
        db,
        actor_user_id,
        service_owner_user_id,
        service_id,
        descriptor,
        is_auto_connected,
        api_key_id,
    )
    .await?;
    let mut resolution = resolution;
    super::assistant_model_observer::exempt_personal_requirement(
        actor_user_id,
        service_id,
        &mut resolution,
    );
    if resolution.required && resolution.fresh_decision {
        return Err(AppError::ApprovalUnsupported);
    }
    Ok(resolution.effect == ApprovalEffect::Deny)
}

#[allow(clippy::too_many_arguments)]
pub async fn evaluate_and_check(
    db: &Database,
    actor_user_id: &str,
    service_owner_user_id: &str,
    service_id: &str,
    descriptor: &OperationDescriptor,
    requester_type: Option<&str>,
    requester_id: &str,
    bypass_approval_flow: bool,
    is_auto_connected: bool,
    api_key_id: Option<&str>,
) -> AppResult<ApprovalOutcome> {
    evaluate_and_check_with_options(
        db,
        actor_user_id,
        service_owner_user_id,
        service_id,
        descriptor,
        requester_type,
        requester_id,
        bypass_approval_flow,
        is_auto_connected,
        ApprovalOptions::default(),
        api_key_id,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn evaluate_and_check_with_options(
    db: &Database,
    actor_user_id: &str,
    service_owner_user_id: &str,
    service_id: &str,
    descriptor: &OperationDescriptor,
    requester_type: Option<&str>,
    requester_id: &str,
    bypass_approval_flow: bool,
    is_auto_connected: bool,
    options: ApprovalOptions,
    api_key_id: Option<&str>,
) -> AppResult<ApprovalOutcome> {
    let mut resolution = resolve_org_aware_approval(
        db,
        actor_user_id,
        service_owner_user_id,
        service_id,
        descriptor,
        is_auto_connected,
        api_key_id,
    )
    .await?;

    if resolution.effect == ApprovalEffect::Deny {
        return Ok(ApprovalOutcome::Denied);
    }

    if resolution.required
        && !resolution.from_org_policy
        && requester_type == Some("access_token")
        && super::assistant_steering::personal_control(actor_user_id, descriptor)
    {
        return Ok(ApprovalOutcome::Allowed { required: false });
    }

    super::assistant_model_observer::exempt_personal_requirement(
        actor_user_id,
        service_id,
        &mut resolution,
    );

    if !resolution.required || bypass_approval_flow {
        return Ok(ApprovalOutcome::Allowed {
            required: resolution.required,
        });
    }

    let requester_type = requester_type
        .ok_or_else(|| AppError::Forbidden("Session auth does not require approval".to_string()))?;
    let primary_owner_user_id = resolution.primary_owner_user_id.clone();
    let has_grant = if resolution.mode == ApprovalMode::Grant {
        check_approval(
            db,
            &primary_owner_user_id,
            service_id,
            requester_type,
            requester_id,
            resolution.from_org_policy,
            descriptor,
        )
        .await?
    } else {
        false
    };

    if has_grant {
        return Ok(ApprovalOutcome::Allowed {
            required: resolution.required,
        });
    }

    Ok(ApprovalOutcome::NeedsApproval(PendingApproval {
        options,
        resolution,
        primary_owner_user_id,
        requester_type: requester_type.to_string(),
        requester_id: requester_id.to_string(),
    }))
}

/// An assistant approval bypass is returned to the dispatcher so its receipt
/// is written only after successful execution. Explicit deny rules still win.
/// Every caller explicitly supplies its NyxID key, or `None` when none exists.
#[allow(clippy::too_many_arguments)]
pub async fn evaluate_for_chat(
    db: &Database,
    actor: &str,
    service_owner: &str,
    service_id: &str,
    descriptor: &OperationDescriptor,
    requester_type: Option<&str>,
    requester_id: &str,
    bypass: bool,
    auto_connected: bool,
    chat: Option<&super::assistant_acknowledgement_service::ChatAuthority>,
    api_key_id: Option<&str>,
) -> AppResult<(
    ApprovalOutcome,
    Option<super::assistant_acknowledgement_service::AutoAllowed>,
)> {
    use super::assistant_acknowledgement_service as acks;
    if chat.is_some_and(|c| {
        c.org_agent_access
            .as_ref()
            .and_then(|a| a.shared())
            .is_some_and(|a| service_owner == a.owner() && actor != service_owner)
    }) {
        let owner_policy = resolve_org_aware_approval(
            db,
            service_owner,
            service_owner,
            service_id,
            descriptor,
            auto_connected,
            api_key_id,
        )
        .await?;
        if owner_policy.required || owner_policy.effect == ApprovalEffect::Deny {
            return Err(AppError::Forbidden(
                "Shared agents cannot request or use the owner's approvals".into(),
            ));
        }
    }
    let outcome = evaluate_and_check_with_options(
        db,
        actor,
        service_owner,
        service_id,
        descriptor,
        requester_type,
        requester_id,
        bypass,
        auto_connected,
        chat.map_or_else(ApprovalOptions::default, |chat| chat.approval_options()),
        api_key_id,
    )
    .await?;
    super::assistant_model_observer::evaluated_policy(actor, service_id, &outcome).await?;
    if let ApprovalOutcome::NeedsApproval(ref pending) = outcome
        && !pending.resolution.fresh_decision
        && let Some(chat) = chat.filter(|chat| !chat.guest)
        && acks::effective_access_mode(db, chat).await?
            == crate::models::assistant_settings::AccessMode::Autopilot
    {
        let outcome = ApprovalOutcome::Allowed {
            required: pending.resolution.required,
        };
        let receipt = Some(acks::AutoAllowed {
            class: match descriptor.verb {
                crate::models::service_approval_config::ApprovalVerb::Read => {
                    acks::OwnerActionClass::Read
                }
                crate::models::service_approval_config::ApprovalVerb::Destructive => {
                    acks::OwnerActionClass::Destructive
                }
                _ => acks::OwnerActionClass::Change,
            },
            mode: crate::models::assistant_settings::AccessMode::Autopilot,
            reason: "approval",
        });
        return Ok((outcome, receipt));
    }
    Ok((outcome, None))
}

pub async fn record_chat_approval(
    db: &Database,
    chat: Option<&super::assistant_acknowledgement_service::ChatAuthority>,
    auto: Option<super::assistant_acknowledgement_service::AutoAllowed>,
    service_id: &str,
    service_name: &str,
    descriptor: &OperationDescriptor,
) {
    if let (Some(chat), Some(auto)) = (chat, auto) {
        let operation = descriptor.operation_summary();
        super::assistant_acknowledgement_service::record_auto_allowed(
            db,
            chat,
            "service_approval",
            &serde_json::json!({"service_id":service_id,"operation":operation}),
            &auto,
            &format!("{} on {}", operation, service_name),
        )
        .await;
    }
}

pub async fn approval_notification_recipients(
    db: &Database,
    approval_owner_user_id: &str,
    pending: &PendingApproval,
) -> AppResult<Vec<String>> {
    // Called only after the dispatcher applies existing execution grants.
    // Refuse before materializing notification defaults or creating a request.
    pending.options.require_request_support()?;
    if !pending.resolution.from_org_policy {
        return Ok(vec![approval_owner_user_id.to_string()]);
    }

    let primary_owner = &pending.primary_owner_user_id;
    let mut admins = crate::services::org_service::list_admin_user_ids(db, primary_owner).await?;
    admins.sort();
    admins.dedup();
    if admins.is_empty() {
        return Err(AppError::OrgApprovalNoAdmin(format!(
            "Org {primary_owner} has an approval policy on this service but no active admins to decide. Add an admin to the org and retry."
        )));
    }
    Ok(admins)
}

async fn user_global_approval_setting(db: &Database, user_id: &str) -> AppResult<Option<bool>> {
    let channel = db
        .collection::<NotificationChannel>(CHANNELS)
        .find_one(doc! { "user_id": user_id })
        .await?;
    Ok(channel.and_then(|c| c.approval_required))
}

/// Pure resolution helper kept for unit-testable semantics. Used to be the
/// final step of the now-removed `requires_approval_for_service`. Tests
/// still exercise it directly.
#[cfg(test)]
fn resolve_approval_requirement(per_service: Option<bool>, global: Option<bool>) -> bool {
    per_service.unwrap_or_else(|| {
        approval_policy::GlobalApprovalMode::from_stored(global)
            .requires(&crate::models::service_approval_config::ApprovalVerb::Write)
    })
}

/// Check whether the request has a valid (non-expired, non-revoked) approval grant.
/// Returns `Ok(true)` if access is granted, `Ok(false)` if approval is needed.
///
/// When `org_scoped` is `true` the lookup accepts either a new org-scoped
/// grant (any requester under the owning org, see ChronoAIProject/NyxID#364)
/// **or** a pre-existing legacy grant that was written under the org
/// `user_id` for the caller's own `requester_type`/`requester_id` before the
/// `org_scoped` field was introduced. The legacy branch keeps the original
/// per-requester semantics so already-approved requesters don't get prompted
/// again after upgrade; it intentionally does **not** widen those grants to
/// other members of the org.
///
/// When `org_scoped` is `false` the lookup keeps the per-requester behavior
/// for personal grants and excludes `org_scoped: true` rows so the two
/// classes stay disjoint.
pub async fn check_approval(
    db: &Database,
    user_id: &str,
    service_id: &str,
    requester_type: &str,
    requester_id: &str,
    org_scoped: bool,
    descriptor: &OperationDescriptor,
) -> AppResult<bool> {
    let now = bson::DateTime::from_chrono(Utc::now());

    let filter = if org_scoped {
        // $or: new org-scoped grant OR legacy same-requester grant. A legacy
        // grant is any row without `org_scoped: true` (i.e. missing the
        // field, or explicitly false) -- there's no other way to spot rows
        // written before the field was added. Using `$ne: true` matches both.
        doc! {
            "user_id": user_id,
            "service_id": service_id,
            "revoked": false,
            "expires_at": { "$gt": now },
            "$or": [
                { "org_scoped": true },
                {
                    "org_scoped": { "$ne": true },
                    "requester_type": requester_type,
                    "requester_id": requester_id,
                },
            ],
        }
    } else {
        doc! {
            "user_id": user_id,
            "service_id": service_id,
            "requester_type": requester_type,
            "requester_id": requester_id,
            "org_scoped": { "$ne": true },
            "revoked": false,
            "expires_at": { "$gt": now },
        }
    };

    let grants: Vec<ApprovalGrant> = db
        .collection::<ApprovalGrant>(GRANTS)
        .find(filter)
        .await?
        .try_collect()
        .await?;

    Ok(grants
        .iter()
        .any(|grant| approval_policy::grant_scope_covers(grant.scope.as_deref(), descriptor)))
}

/// Create an approval request.
///
/// Grant mode keeps the legacy dedupe behavior for a pending
/// `(user, service, requester)` tuple. When `from_org_policy` is true the
/// pending dedupe key collapses the requester dimension so concurrent calls
/// from *different org members* against the same org-owned service share a
/// single pending request (instead of each triggering its own admin prompt).
/// Per-request mode always creates a distinct pending request so concurrent
/// calls cannot piggyback on a single approval.
///
/// `notify_user_ids` is the list of users who will be notified and are
/// authorized to decide the request. For personal approvals this is
/// `[user_id]`. For org-policy approvals (where the org owns the resource
/// and has set a per-service approval config) this is the org's admin
/// user_ids resolved at request creation time. The list is persisted on
/// the request so the decide endpoint can authorize without re-resolving
/// org membership at decision time.
///
/// If `notify_user_ids` is empty the function defaults to `[user_id]` --
/// preserves the existing single-recipient semantic for callers that
/// don't yet thread org context through.
///
/// `from_org_policy` is persisted on the request so `process_decision` can
/// mint an org-scoped grant on approval (see ChronoAIProject/NyxID#364).
#[derive(Clone, Debug)]
pub struct ApprovalRequestOperation {
    pub fresh_decision: bool,
    pub caller_api_key_id: Option<String>,
    pub protocol: Option<String>,
    pub(super) connected_request_id: Option<String>,
    pub(super) connected_deadline: Option<chrono::DateTime<Utc>>,
    pub approval: ApprovalOptions,
    pub assistant_group: Option<crate::models::approval_request::GroupApprovalBinding>,
    pub operation_summary: String,
    pub action_description: Option<String>,
    pub http_method: Option<String>,
    pub resource: Option<String>,
    pub verb: Option<String>,
    pub grant_scope: Option<String>,
    pub exact_service: Option<ExactServiceApprovalBinding>,
}

impl ApprovalRequestOperation {
    pub fn from_descriptor(descriptor: &OperationDescriptor, grant_scope: Option<String>) -> Self {
        Self {
            fresh_decision: false,
            caller_api_key_id: None,
            protocol: Some(descriptor.protocol.summary_prefix().into()),
            connected_request_id: None,
            connected_deadline: None,
            approval: ApprovalOptions::default(),
            assistant_group: None,
            operation_summary: descriptor.operation_summary(),
            action_description: Some(descriptor.summary.clone()),
            http_method: descriptor.method.clone(),
            resource: descriptor.resource.clone(),
            verb: Some(descriptor.verb.as_str().to_string()),
            grant_scope,
            exact_service: None,
        }
    }

    pub(crate) fn apply_connected_budget(&mut self, timeout_secs: u32) {
        if let Some(deadline) = connected_budget_deadline(timeout_secs) {
            self.connected_deadline = Some(
                self.connected_deadline
                    .map_or(deadline, |existing| existing.min(deadline)),
            );
        }
    }

    pub fn with_exact_service(mut self, binding: ExactServiceApprovalBinding) -> Self {
        self.exact_service = Some(binding);
        self
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn create_approval_request(
    db: &Database,
    config: &AppConfig,
    http_client: &reqwest::Client,
    fcm_auth: Option<&FcmAuth>,
    apns_auth: Option<&ApnsAuth>,
    user_id: &str,
    service_id: &str,
    service_name: &str,
    service_slug: &str,
    requester_type: &str,
    requester_id: &str,
    requester_label: Option<&str>,
    operation: ApprovalRequestOperation,
    approval_mode: ApprovalMode,
    timeout_secs: u32,
    notify_user_ids: Vec<String>,
    from_org_policy: bool,
) -> AppResult<ApprovalRequest> {
    operation.approval.require_request_support()?;
    let notify_user_ids = if let Some(group) = &operation.assistant_group {
        vec![group.actor_user_id.clone()]
    } else if notify_user_ids.is_empty() {
        vec![user_id.to_string()]
    } else {
        notify_user_ids
    };

    let collection = db.collection::<ApprovalRequest>(REQUESTS);
    let mut idempotency_key = operation
        .exact_service
        .as_ref()
        .map(|binding| binding.request_key.clone())
        .unwrap_or_else(|| {
            compute_pending_request_idempotency_key(
                &approval_mode,
                user_id,
                service_id,
                requester_type,
                requester_id,
                from_org_policy,
            )
        });
    // An explicit wait budget must not shorten another caller's grant prompt.
    if approval_mode == ApprovalMode::Grant
        && CONNECTED_WAIT_BUDGET.try_with(|_| ()).is_ok()
        && let Some(deadline) = operation.connected_deadline
    {
        idempotency_key = format!("{idempotency_key}:until:{}", deadline.timestamp_millis());
    }
    if let Some(group) = &operation.assistant_group {
        // A card from another chain or private thread can never be coalesced.
        idempotency_key = format!(
            "{idempotency_key}:{}:{}",
            group.conversation_id, group.request_id
        );
    }
    // Callers coalesce exactly as before unless the owner's own policy for this
    // service has key-limited rules, where each key needs its own decision.
    if let Some(key) = &operation.caller_api_key_id
        && !from_org_policy
        && personal_policy_is_key_limited(db, user_id, service_id).await?
    {
        idempotency_key = format!("{idempotency_key}:key:{key}");
    }
    let mut inserted_request: Option<ApprovalRequest> = None;
    for _attempt in 0..2 {
        if let Some(binding) = operation.exact_service.as_ref()
            && let Some(existing) = collection
                .find_one(doc! { "exact_service.request_key": &binding.request_key })
                .await?
        {
            if existing
                .exact_service
                .as_ref()
                .is_some_and(|existing| same_exact_service_authority(existing, binding))
                && existing.requester_type == requester_type
                && existing.requester_id == requester_id
            {
                return Ok(existing);
            }
            return Err(AppError::Conflict(
                "exact_service_request_conflict".to_string(),
            ));
        }
        if let Some(binding) = operation.exact_service.as_ref()
            && collection
                .find_one(exact_service_semantic_effect_filter(
                    requester_type,
                    requester_id,
                    binding,
                ))
                .await?
                .is_some()
        {
            return Err(AppError::Conflict(
                "exact_service_request_conflict".to_string(),
            ));
        }

        // Check for existing pending request with the same idempotency key.
        // This handles normal idempotent retries and the winner in concurrent inserts.
        if let Some(existing) = collection
            .find_one(doc! {
                "idempotency_key": &idempotency_key,
                "status": "pending",
                "execution_proposal": null,
            })
            .await?
        {
            return Ok(existing);
        }

        let now = approval_now();
        let expires_at = operation
            .connected_deadline
            .unwrap_or_else(|| now + Duration::seconds(i64::from(timeout_secs)));

        let grant_expiry_days_snapshot = if approval_mode == ApprovalMode::Grant {
            let policy = get_config(db, user_id, service_id)
                .await?
                .filter(|cfg| !cfg.disabled)
                .and_then(|cfg| cfg.grant_expiry_days);
            Some(crate::models::approval_request::GrantExpiryDefault {
                policy_override: policy.is_some(),
                days: match policy {
                    Some(days) => days,
                    None => {
                        notification_service::get_or_create_channel(db, user_id)
                            .await?
                            .grant_expiry_days
                    }
                },
            })
        } else {
            None
        };
        let request = ApprovalRequest {
            fresh_decision: operation.fresh_decision,
            caller_api_key_id: operation.caller_api_key_id.clone(),
            approval_protocol: operation.protocol.clone(),
            grant_expiry_days_snapshot,
            activity_outcome: None,
            activity_execution: None,
            execution_proposal: None,
            assistant_group: operation.assistant_group.clone(),
            id: operation
                .connected_request_id
                .clone()
                .unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
            user_id: user_id.to_string(),
            service_id: service_id.to_string(),
            service_name: service_name.to_string(),
            service_slug: service_slug.to_string(),
            requester_type: requester_type.to_string(),
            requester_id: requester_id.to_string(),
            requester_label: requester_label.map(String::from),
            operation_summary: operation.operation_summary.clone(),
            action_description: operation.action_description.clone(),
            http_method: operation.http_method.clone(),
            resource: operation.resource.clone(),
            verb: operation.verb.clone(),
            grant_scope: operation.grant_scope.clone(),
            tool_name: None,
            tool_call_id: None,
            tool_arguments: None,
            is_destructive: None,
            approval_mode: approval_mode.clone(),
            status: "pending".to_string(),
            idempotency_key: idempotency_key.clone(),
            notification_channel: None,
            telegram_message_id: None,
            telegram_chat_id: None,
            expires_at,
            decided_at: None,
            decision_channel: None,
            decision_idempotency_key: None,
            notify_user_ids: notify_user_ids.clone(),
            from_org_policy,
            exact_service: operation.exact_service.clone(),
            created_at: now,
        };

        match super::erasure_admission::insert_owned(db, REQUESTS, user_id, &request).await {
            Ok(_) => {
                inserted_request = Some(request);
                break;
            }
            Err(AppError::DatabaseError(e)) if is_duplicate_key_error(&e) => {
                // Concurrent insert race: another request inserted/processed first.
                // Retry once to read the pending row or create a new row if no longer pending.
                continue;
            }
            Err(error) => return Err(error),
        }
    }

    let request = inserted_request.ok_or_else(|| {
        if operation.exact_service.is_some() {
            AppError::Conflict("exact_service_request_conflict".to_string())
        } else {
            AppError::Conflict("Approval request conflict, please retry".to_string())
        }
    })?;

    let notify = || {
        notify_service_approval(
            db,
            config,
            http_client,
            fcm_auth,
            apns_auth,
            &notify_user_ids,
            &request,
        )
    };
    let updated = if let Some(deadline) = operation.connected_deadline {
        let remaining = (deadline - approval_now()).to_std().unwrap_or_default();
        if remaining.is_zero() {
            None
        } else {
            tokio::time::timeout(remaining, notify())
                .await
                .unwrap_or(Ok(None))?
        }
    } else {
        notify().await?
    };
    Ok(updated.unwrap_or(request))
}

async fn notify_service_approval(
    db: &Database,
    config: &AppConfig,
    http_client: &reqwest::Client,
    fcm_auth: Option<&FcmAuth>,
    apns_auth: Option<&ApnsAuth>,
    notify_user_ids: &[String],
    request: &ApprovalRequest,
) -> AppResult<Option<ApprovalRequest>> {
    let collection = db.collection::<ApprovalRequest>(REQUESTS);
    // Fan out notifications to every recipient. The first recipient with a
    // configured channel "wins" the telegram_chat_id / telegram_message_id
    // slots on the request (so the existing edit-on-decision flow works for
    // at least one recipient). Other recipients get their own messages but
    // their channel/message ids are not stored on the request -- the
    // decision-time edit will only update one of them. Trade-off accepted
    // for now; a fuller fix would store per-recipient delivery state.
    //
    // For org-policy requests, resolve the owning org's display name
    // exactly once and pass it into every fan-out call so Telegram and
    // push notifications can render org-aware wording. A failed lookup
    // degrades gracefully to the generic template (`None`) rather than
    // blocking the request.
    let org_name: Option<String> = if request.from_org_policy {
        match crate::services::org_service::get_org_user(db, &request.user_id).await {
            Ok(org_user) => org_user.display_name.clone(),
            Err(e) => {
                tracing::warn!(
                    org_id = %request.user_id,
                    error = %e,
                    "Failed to resolve org display name for approval notification"
                );
                None
            }
        }
    } else {
        None
    };

    let mut all_channels: Vec<String> = Vec::new();
    let mut primary_chat_id: Option<i64> = None;
    let mut primary_message_id: Option<i64> = None;
    let mut delivered_to_anyone = false;
    let mut last_err: Option<AppError> = None;

    for recipient in notify_user_ids {
        match notification_service::send_approval_notification(
            db,
            config,
            http_client,
            fcm_auth,
            apns_auth,
            recipient,
            request,
            org_name.as_deref(),
        )
        .await
        {
            Ok(result) => {
                delivered_to_anyone = true;
                for ch in result.channels {
                    if !all_channels.contains(&ch) {
                        all_channels.push(ch);
                    }
                }
                if primary_chat_id.is_none() {
                    primary_chat_id = result.telegram_chat_id;
                    primary_message_id = result.telegram_message_id;
                }
            }
            Err(e) => {
                tracing::warn!(
                    recipient = %recipient,
                    error = %e,
                    "Approval notification failed for one recipient"
                );
                last_err = Some(e);
            }
        }
    }

    if !delivered_to_anyone {
        // All recipients failed: log but still return the request so the
        // user can approve via the web UI. Mirrors the previous behavior
        // when the single recipient had no channel configured.
        if let Some(err) = last_err {
            tracing::warn!(
                request_id = %request.id,
                error = %err,
                "Approval notification failed for all recipients"
            );
        }
        return Ok(None);
    }

    let channel_name = all_channels.join(",");
    let update = doc! {
        "$set": {
            "notification_channel": &channel_name,
            "telegram_chat_id": primary_chat_id,
            "telegram_message_id": primary_message_id,
        }
    };
    collection
        .update_one(doc! { "_id": &request.id }, update)
        .await?;

    Ok(collection.find_one(doc! {"_id": &request.id}).await?)
}

fn same_exact_service_authority(
    left: &ExactServiceApprovalBinding,
    right: &ExactServiceApprovalBinding,
) -> bool {
    left.request_key == right.request_key
        && left.actor_user_id == right.actor_user_id
        && left.user_service_id == right.user_service_id
        && left.endpoint_id == right.endpoint_id
        && left.catalog_digest == right.catalog_digest
        && (left.exact_view_digest.is_none()
            || right.exact_view_digest.is_none()
            || left.exact_view_digest == right.exact_view_digest)
        && left.endpoint_contract_digest == right.endpoint_contract_digest
        && left.operation_digest == right.operation_digest
        && left.operation_id == right.operation_id
        && left.operation_generation == right.operation_generation
        && left.producer_generation_bound == right.producer_generation_bound
        && left.effect_idempotency_key == right.effect_idempotency_key
        && left.arguments == right.arguments
}

fn exact_service_semantic_effect_filter(
    requester_type: &str,
    requester_id: &str,
    binding: &ExactServiceApprovalBinding,
) -> Document {
    doc! {
        "requester_type": requester_type,
        "requester_id": requester_id,
        "exact_service.actor_user_id": &binding.actor_user_id,
        "exact_service.endpoint_id": &binding.endpoint_id,
        "exact_service.effect_idempotency_key": &binding.effect_idempotency_key,
    }
}

/// Create a tool approval request (from an external caller such as Aevatar).
///
/// Uses sentinel `service_id: "tool_approval"` and maps the tool name into
/// proxy-oriented fields so the existing notification and decision pipeline
/// works unchanged.
#[allow(clippy::too_many_arguments)]
pub async fn create_tool_approval_request(
    db: &Database,
    config: &AppConfig,
    http_client: &reqwest::Client,
    fcm_auth: Option<&FcmAuth>,
    apns_auth: Option<&ApnsAuth>,
    user_id: &str,
    tool_name: &str,
    tool_call_id: Option<&str>,
    tool_arguments: Option<&str>,
    is_destructive: bool,
    requester_type: &str,
    requester_id: &str,
    requester_label: Option<&str>,
    options: ApprovalOptions,
) -> AppResult<ApprovalRequest> {
    options.require_request_support()?;
    // Materialize notification delivery/lifetime defaults without choosing a
    // global approval preference on the user's behalf.
    let timeout_secs = notification_service::get_or_create_channel(db, user_id)
        .await?
        .approval_timeout_secs;

    let collection = db.collection::<ApprovalRequest>(REQUESTS);

    // Tool approvals are always unique (per_request semantics).
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    let now = Utc::now();
    let expires_at = now + Duration::seconds(i64::from(timeout_secs));

    let operation_summary = format!("tool:{tool_name}");
    let action_description = tool_arguments.map(|args| format!("{tool_name}({args})"));

    let request = ApprovalRequest {
        fresh_decision: false,
        caller_api_key_id: None,
        approval_protocol: None,
        grant_expiry_days_snapshot: None,
        activity_outcome: None,
        activity_execution: None,
        execution_proposal: None,
        assistant_group: None,
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        service_id: "tool_approval".to_string(),
        service_name: tool_name.to_string(),
        service_slug: "tool".to_string(),
        requester_type: requester_type.to_string(),
        requester_id: requester_id.to_string(),
        requester_label: requester_label.map(String::from),
        operation_summary,
        action_description,
        http_method: None,
        resource: None,
        verb: None,
        grant_scope: None,
        tool_name: Some(tool_name.to_string()),
        tool_call_id: tool_call_id.map(String::from),
        tool_arguments: tool_arguments.map(String::from),
        is_destructive: Some(is_destructive),
        approval_mode: ApprovalMode::PerRequest,
        status: "pending".to_string(),
        idempotency_key,
        notification_channel: None,
        telegram_message_id: None,
        telegram_chat_id: None,
        expires_at,
        decided_at: None,
        decision_channel: None,
        decision_idempotency_key: None,
        // Tool approvals are always personal: the agent calling
        // `POST /api/v1/approvals/requests` is asking the actor to
        // approve a specific tool invocation. Org cascade does not apply.
        notify_user_ids: vec![user_id.to_string()],
        from_org_policy: false,
        exact_service: None,
        created_at: now,
    };

    super::erasure_admission::insert_owned(db, REQUESTS, user_id, &request).await?;

    // Send notification through existing pipeline. Tool approvals are
    // always personal (see `from_org_policy: false` above), so no org
    // context is ever passed here.
    match notification_service::send_approval_notification(
        db,
        config,
        http_client,
        fcm_auth,
        apns_auth,
        user_id,
        &request,
        None,
    )
    .await
    {
        Ok(result) => {
            let channel_name = result.channels.join(",");
            let update = doc! {
                "$set": {
                    "notification_channel": &channel_name,
                    "telegram_chat_id": result.telegram_chat_id,
                    "telegram_message_id": result.telegram_message_id,
                }
            };
            collection
                .update_one(doc! { "_id": &request.id }, update)
                .await?;

            let updated = collection
                .find_one(doc! { "_id": &request.id })
                .await?
                .unwrap_or(request);

            Ok(updated)
        }
        Err(e) => {
            tracing::warn!("Failed to send tool approval notification: {e}");
            Ok(request)
        }
    }
}

pub(crate) enum DecisionSource {
    Web(AuditActor),
    Telegram,
}

impl DecisionSource {
    fn channel(&self) -> &'static str {
        match self {
            Self::Web(_) => "web",
            Self::Telegram => "telegram",
        }
    }

    fn intent(
        &self,
        request_id: &str,
        service_id: &str,
        owner_id: &str,
        approved: bool,
    ) -> AppResult<SecurityEffectIntent> {
        let owner;
        let actor = match self {
            Self::Web(actor) => actor,
            Self::Telegram => {
                owner = AuditActor {
                    user_id: owner_id.to_string(),
                    ip_address: None,
                    user_agent: None,
                    api_key_id: None,
                    api_key_name: None,
                };
                &owner
            }
        };
        required::intent_preserving_actor(
            actor,
            "approval_decision",
            Some(serde_json::json!({
                "request_id": required::text(required::TextField::Identifier, request_id),
                "service_id": required::text(required::TextField::Identifier, service_id),
                "approved": approved, "channel": self.channel(),
            })),
        )
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub async fn process_decision(
    db: &Database,
    config: &AppConfig,
    http_client: &reqwest::Client,
    fcm_auth: Option<Arc<FcmAuth>>,
    apns_auth: Option<Arc<ApnsAuth>>,
    request_id: &str,
    approved: bool,
    duration_sec: Option<i64>,
    idempotency_key: Option<&str>,
    decision_channel: &str,
) -> AppResult<ApprovalRequest> {
    let current = read_decision_request(db, request_id).await?;
    let source = if decision_channel == "telegram" {
        DecisionSource::Telegram
    } else {
        DecisionSource::Web(AuditActor {
            user_id: current.request.user_id.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        })
    };
    Box::pin(process_decision_with_access(
        db,
        config,
        http_client,
        fcm_auth,
        apns_auth,
        current,
        approved,
        duration_sec,
        idempotency_key,
        source,
        None,
    ))
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn process_decision_with_access(
    db: &Database,
    config: &AppConfig,
    http_client: &reqwest::Client,
    fcm_auth: Option<Arc<FcmAuth>>,
    apns_auth: Option<Arc<ApnsAuth>>,
    decision_request: DecisionRequest,
    approved: bool,
    duration_sec: Option<i64>,
    idempotency_key: Option<&str>,
    source: DecisionSource,
    snapshot: Option<&Arc<super::org_agent_service::RequestAccess>>,
) -> AppResult<ApprovalRequest> {
    let DecisionRequest {
        request: current,
        mut session,
    } = decision_request;
    super::execution_permission_dispatch::proposals::legacy_refusal(&current)?;
    if let Some(binding) = &current.assistant_group {
        super::org_group_service::authorize_approval(
            db,
            &current,
            &binding.actor_user_id,
            snapshot,
        )
        .await?;
    }
    let collaboration = if let Some(binding) = &current.assistant_group
        && super::collaboration_group_service::marked(db, &binding.group_id).await?
    {
        Some(binding.clone())
    } else {
        None
    };
    let now = approval_now();
    let mut update = doc! {
        "status": if approved { "approved" } else { "rejected" },
        "decided_at": bson::DateTime::from_chrono(now),
        "decision_channel": source.channel(),
    };
    if let Some(key) = idempotency_key {
        update.insert("decision_idempotency_key", key);
    }
    // The handler has already authorized this request. Bind its identity and
    // group to the write, including for legacy documents missing optional fields.
    let mut filter = doc! {
        "_id": &current.id,
        "status": "pending",
        "user_id": &current.user_id,
        "service_id": &current.service_id,
        "execution_proposal": bson::Bson::Null,
    };
    if let Some(binding) = &current.assistant_group {
        filter.extend(doc! {
            "assistant_group.group_id": &binding.group_id,
            "assistant_group.request_id": &binding.request_id,
            "assistant_group.actor_user_id": &binding.actor_user_id,
            "assistant_group.conversation_id": &binding.conversation_id,
        });
    } else {
        filter.insert("assistant_group", bson::Bson::Null);
    }
    let decision = DecisionMutation {
        filter,
        update,
        collaboration,
        approved,
        duration_sec,
        now,
        grant_id: uuid::Uuid::new_v4().to_string(),
        intent: source.intent(&current.id, &current.service_id, &current.user_id, approved)?,
    };
    let updated = match Box::pin(decision.commit(db, &mut session)).await? {
        DecisionOutcome::Applied(updated) => *updated,
        DecisionOutcome::ModeConflict => {
            return Err(AppError::Conflict(
                "Service approval mode has changed; this request is no longer valid".into(),
            ));
        }
        DecisionOutcome::Unmatched => {
            let existing = get_request(db, &current.id).await?;
            if existing.status == "expired"
                || (existing.status == "pending" && existing.expires_at <= approval_now())
            {
                return Err(AppError::Forbidden("Approval request expired".into()));
            }
            if is_idempotent_replay(
                &existing.status,
                existing.decision_idempotency_key.as_deref(),
                idempotency_key,
                approved,
            )? {
                return Ok(existing);
            }
            return Err(AppError::Conflict("already_decided".into()));
        }
    };

    // Notify channels about the decision (best-effort, non-blocking).
    let request_clone = updated.clone();
    let config_clone = config.clone();
    let http_clone = http_client.clone();
    let db_clone = db.clone();
    let fcm_auth_clone = fcm_auth.clone();
    let apns_auth_clone = apns_auth.clone();
    crate::services::shutdown::process().spawn(async move {
        let _ = notification_service::notify_decision(
            &config_clone,
            &http_clone,
            fcm_auth_clone.as_deref(),
            apns_auth_clone.as_deref(),
            &db_clone,
            &request_clone,
            approved,
        )
        .await;
    });

    Ok(updated)
}

enum DecisionOutcome {
    Applied(Box<ApprovalRequest>),
    ModeConflict,
    Unmatched,
}

struct DecisionMutation {
    filter: Document,
    update: Document,
    collaboration: Option<crate::models::approval_request::GroupApprovalBinding>,
    approved: bool,
    duration_sec: Option<i64>,
    now: chrono::DateTime<Utc>,
    grant_id: String,
    intent: SecurityEffectIntent,
}

// Only a collision creating the lazy channel can retry the decision as a whole.
// A duplicate grant or audit ID must surface as a failed decision.
#[derive(Debug)]
struct ChannelCreationRace;

impl DecisionMutation {
    async fn commit(
        self,
        db: &Database,
        session: &mut ClientSession,
    ) -> AppResult<DecisionOutcome> {
        let context = Arc::new((db.clone(), self));
        let mut raced = false;
        loop {
            let context = context.clone();
            let result = session
                .start_transaction()
                .read_concern(mongodb::options::ReadConcern::snapshot())
                .write_concern(mongodb::options::WriteConcern::majority())
                .and_run2(async move |session| {
                    let (db, mutation) = &*context;
                    transaction_result(mutation.apply(db, session).await)
                })
                .await;
            match result {
                Err(error) if !raced && error.get_custom::<ChannelCreationRace>().is_some() => {
                    raced = true
                }
                result => return result.map_err(map_transaction_error),
            }
        }
    }

    async fn apply(
        &self,
        db: &Database,
        session: &mut ClientSession,
    ) -> AppResult<DecisionOutcome> {
        use super::collaboration_group_service::runtime;
        use crate::models::assistant_conversation::{
            AssistantConversation, COLLECTION_NAME as CONVERSATIONS,
        };
        let mut filter = self.filter.clone();
        let conversation = if let Some(binding) = &self.collaboration {
            let row = db
                .collection::<AssistantConversation>(CONVERSATIONS)
                .find_one(doc! {
                    "_id": &binding.conversation_id,
                    "collaboration.request_id": &binding.request_id,
                })
                .session(&mut *session)
                .await?
                .ok_or_else(|| AppError::NotFound("Approval request not found".into()))?;
            runtime::fence_decision(db, &row, &binding.actor_user_id, session).await?;
            Some(row)
        } else {
            None
        };
        let requests = db.collection::<ApprovalRequest>(REQUESTS);
        filter.insert(
            "expires_at",
            doc! {"$gt":bson::DateTime::from_chrono(approval_now())},
        );
        let Some(updated) = requests
            .find_one_and_update(filter, doc! {"$set":self.update.clone()})
            .return_document(ReturnDocument::After)
            .session(&mut *session)
            .await?
        else {
            return Ok(DecisionOutcome::Unmatched);
        };
        if let Some(row) = conversation
            && row.active_turn.is_none()
        {
            runtime::queue_decision(db, &row, session).await?;
        }
        if self.approved
            && requires_legacy_grant_mode_recheck(&updated)
            && resolve_approval_mode(db, &updated.user_id, &updated.service_id, session).await?
                != ApprovalMode::Grant
        {
            requests
                .update_one(
                    doc! {"_id":&updated.id,"status":"approved"},
                    doc! {"$set":{"status":"expired"}},
                )
                .session(&mut *session)
                .await?;
            return Ok(DecisionOutcome::ModeConflict);
        }
        if self.approved && updated.approval_mode == ApprovalMode::Grant {
            self.create_grant(db, &updated, session).await?;
        }
        super::security_effect_service::enqueue_in_session(
            db,
            session,
            std::slice::from_ref(&self.intent),
        )
        .await?;
        Ok(DecisionOutcome::Applied(Box::new(updated)))
    }

    async fn create_grant(
        &self,
        db: &Database,
        updated: &ApprovalRequest,
        session: &mut ClientSession,
    ) -> AppResult<()> {
        let channels = db.collection::<NotificationChannel>(CHANNELS);
        let channel = match channels
            .find_one(doc! {"user_id":&updated.user_id})
            .session(&mut *session)
            .await?
        {
            Some(channel) => channel,
            None => {
                let channel = notification_service::default_channel(&updated.user_id);
                channels
                    .insert_one(&channel)
                    .session(&mut *session)
                    .await
                    .map_err(|error| {
                        if is_duplicate_key_error(&error) {
                            mongodb::error::Error::custom(ChannelCreationRace)
                        } else {
                            error
                        }
                    })?;
                channel
            }
        };
        // The protection's own duration wins over the account-wide default.
        let policy_days = db
            .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
            .find_one(doc! {"user_id":&updated.user_id,"service_id":&updated.service_id})
            .session(&mut *session)
            .await?
            .filter(|config| !config.disabled)
            .and_then(|config| config.grant_expiry_days);
        let snapshot = updated.grant_expiry_days_snapshot.as_ref();
        let default_days = snapshot
            .map(|s| s.days)
            .or(policy_days)
            .unwrap_or(channel.grant_expiry_days);
        let policy_cap = snapshot
            .filter(|s| s.policy_override)
            .map(|s| s.days)
            .or_else(|| {
                if snapshot.is_none() {
                    policy_days
                } else {
                    None
                }
            });
        let duration = self
            .duration_sec
            .map(|seconds| policy_cap.map_or(seconds, |days| seconds.min(i64::from(days) * 86400)));
        let grant_expiry = resolve_grant_expiry(self.now, duration, default_days);
        let grant = ApprovalGrant {
            execution_review: None,
            execution_last_used_at: None,
            execution_use: None,
            execution: None,
            id: self.grant_id.clone(),
            user_id: updated.user_id.clone(),
            service_id: updated.service_id.clone(),
            service_name: updated.service_name.clone(),
            requester_type: updated.requester_type.clone(),
            requester_id: updated.requester_id.clone(),
            requester_label: updated.requester_label.clone(),
            approval_request_id: updated.id.clone(),
            scope: updated.grant_scope.clone(),
            granted_at: self.now,
            expires_at: grant_expiry,
            revoked: false,
            org_scoped: updated.from_org_policy,
        };

        db.collection::<ApprovalGrant>(GRANTS)
            .insert_one(&grant)
            .session(session)
            .await?;
        Ok(())
    }
}

fn requires_legacy_grant_mode_recheck(request: &ApprovalRequest) -> bool {
    request.approval_mode == ApprovalMode::Grant && request.grant_scope.is_none()
}

fn is_idempotent_replay(
    existing_status: &str,
    existing_decision_idempotency_key: Option<&str>,
    incoming_idempotency_key: Option<&str>,
    approved: bool,
) -> AppResult<bool> {
    let already_decided = existing_status == "approved" || existing_status == "rejected";

    if !already_decided {
        return Err(AppError::Conflict("decision_state_conflict".to_string()));
    }

    if let Some(key) = incoming_idempotency_key
        && existing_decision_idempotency_key == Some(key)
    {
        let existing_approved = existing_status == "approved";
        if existing_approved == approved {
            return Ok(true);
        }
        return Err(AppError::Conflict(
            "idempotency_key_reused_with_different_decision".to_string(),
        ));
    }

    Ok(false)
}

fn resolve_grant_expiry(
    now: chrono::DateTime<Utc>,
    duration_sec: Option<i64>,
    default_days: u32,
) -> chrono::DateTime<Utc> {
    if let Some(duration_sec) = duration_sec {
        return now + Duration::seconds(duration_sec);
    }
    now + Duration::days(i64::from(default_days))
}

/// Expire pending requests that have passed their expiry time.
/// Called by the background task.
pub async fn expire_pending_requests(
    db: &Database,
    config: &AppConfig,
    http_client: &reqwest::Client,
    fcm_auth: Option<&FcmAuth>,
    apns_auth: Option<&ApnsAuth>,
    telemetry: Option<&Arc<TelemetryClient>>,
) -> AppResult<u64> {
    let sweep_time = bson::DateTime::from_chrono(Utc::now());
    let sweep_marker = format!("expiry:{}", uuid::Uuid::new_v4());

    let expired: Vec<ApprovalRequest> = db
        .collection::<ApprovalRequest>(REQUESTS)
        .find(doc! {
            "status": "pending",
            "execution_proposal": null,
            "expires_at": { "$lte": sweep_time },
        })
        .with_options(
            FindOptions::builder()
                .sort(doc! { "expires_at": 1 })
                .limit(100)
                .build(),
        )
        .await?
        .try_collect()
        .await?;

    if expired.is_empty() {
        return Ok(0);
    }

    // Batch update status to "expired", but ONLY if still pending.
    // This prevents a race where a user approves/rejects a request between
    // the find() above and this update_many(). Without the status guard,
    // the sweep could overwrite an "approved" status back to "expired".
    let ids: Vec<&str> = expired.iter().map(|r| r.id.as_str()).collect();
    let count = expire_pending_ids(db, &ids, sweep_time, &sweep_marker).await?;
    if count == 0 {
        return Ok(0);
    }

    // Re-query to get only the requests that were actually expired by the
    // update_many above. A request that was approved/rejected between the
    // initial find() and the update_many() will NOT have status "expired"
    // in the DB (the status guard prevented the overwrite), so it will be
    // excluded here. Matching on this sweep's unique marker also prevents
    // overlapping sweep tasks from re-sending each other's expiry side
    // effects for the same request IDs, even across multiple server
    // instances.
    let actually_expired = requests_updated_by_expiry_sweep(db, expired, &sweep_marker).await?;

    // Edit Telegram messages for expired requests (best-effort)
    for req in &actually_expired {
        if req
            .notification_channel
            .as_deref()
            .is_some_and(|ch| ch.contains("telegram"))
            && let (Some(chat_id), Some(message_id)) =
                (req.telegram_chat_id, req.telegram_message_id)
            && let Some(bot_token) = config.telegram_bot_token.as_deref()
        {
            let http = http_client.clone();
            let token = bot_token.to_string();
            let svc_name = req.service_name.clone();
            crate::services::shutdown::process().spawn(async move {
                let _ = crate::services::telegram_service::edit_message_after_decision(
                    &http,
                    &token,
                    chat_id,
                    message_id,
                    false,
                    &format!("{svc_name} (expired)"),
                )
                .await;
            });
        }
    }

    // Send silent push to update mobile app UI for expired requests
    // (best-effort). For org-policy requests `user_id` is the org (no
    // channel); the app clients that need the expiry ping are every admin
    // that was notified at request creation time. Fall back to
    // `[req.user_id]` for legacy rows without `notify_user_ids` (see
    // ChronoAIProject/NyxID#370).
    for req in &actually_expired {
        let mut data = std::collections::HashMap::new();
        data.insert("type".to_string(), "approval_expired".to_string());
        data.insert("request_id".to_string(), req.id.clone());
        let recipients: Vec<String> = if req.notify_user_ids.is_empty() {
            vec![req.user_id.clone()]
        } else {
            req.notify_user_ids.clone()
        };
        for recipient in recipients {
            let _ = notification_service::send_silent_push_to_user(
                db,
                config,
                http_client,
                fcm_auth,
                apns_auth,
                &recipient,
                &data,
            )
            .await;
        }
    }

    // Telemetry: emit `approval.expired` per actually-expired row. Background
    // sweep has no request headers, so use the default `backend` context; no
    // auth context either (sweep runs unauthenticated), so api_key_id = None.
    for req in &actually_expired {
        emit_event(
            telemetry.map(Arc::as_ref),
            &req.user_id.to_string(),
            None,
            &TelemetryContext::default(),
            TelemetryEvent::ApprovalExpired {
                service_slug: req.service_slug.clone(),
                mode: req.approval_mode.as_str().to_string(),
            },
        )
        .await;
    }

    Ok(count)
}

/// Shared pending-only transition for time expiry and obsolete chat asks.
async fn expire_pending_ids(
    db: &Database,
    ids: &[&str],
    decided_at: bson::DateTime,
    marker: &str,
) -> AppResult<u64> {
    if ids.is_empty() {
        return Ok(0);
    }
    Ok(db
        .collection::<ApprovalRequest>(REQUESTS)
        .update_many(
            doc! {"_id": {"$in": ids}, "status": "pending"},
            doc! {"$set": {"status": "expired", "decided_at": decided_at,
            "decision_idempotency_key": marker}},
        )
        .await?
        .modified_count)
}

/// Retire only approvals bound to actual assistant credentials. General agent
/// keys and explicit denials retain their lifecycle. Called on policy writes
/// and startup, never while reading history or pending cards.
pub(super) async fn retire_full_access_chat_requests(
    db: &Database,
    user: Option<&str>,
    agent: Option<&str>,
    modes: &mut super::assistant_acknowledgement_service::AccessModeCache,
    sweep_time: bson::DateTime,
) -> AppResult<()> {
    use crate::models::{
        api_key::{ApiKey, COLLECTION_NAME as KEYS},
        assistant_agent_credential::{AssistantAgentCredential, COLLECTION_NAME as CREDENTIALS},
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
    };
    let mut filter = doc! {"execution_proposal": null, "fresh_decision":{"$ne":true}, "status": "pending", "requester_type": "user",
    "requester_label": {"$ne": bson::Bson::Null}, "created_at": {"$lte": sweep_time}};
    if let Some(user) = user {
        filter.insert("requester_id", user);
    }
    let marker = format!("expiry:full-access:{}", uuid::Uuid::new_v4());
    let mut identities = std::collections::HashMap::new();
    loop {
        let pending: Vec<_> = db
            .collection::<ApprovalRequest>(REQUESTS)
            .find(filter.clone())
            .sort(doc! {"_id": 1})
            .limit(100)
            .await?
            .try_collect()
            .await?;
        let Some(last) = pending.last() else { break };
        filter.insert("_id", doc! {"$gt": &last.id});
        let mut ids = Vec::new();
        for request in pending {
            let Some(label) = request.requester_label.as_deref() else {
                continue;
            };
            let identity = (request.requester_id.clone(), label.to_owned());
            if !identities.contains_key(&identity) {
                let key = db
                    .collection::<ApiKey>(KEYS)
                    .find_one(doc! {"user_id": &request.requester_id, "name": label})
                    .await?;
                let thread = if let Some(key) = key {
                    let credential = db
                        .collection::<AssistantAgentCredential>(CREDENTIALS)
                        .find_one(doc! {"user_id": &request.requester_id, "api_key_id": &key.id})
                        .await?;
                    if let Some(credential) = credential {
                        db.collection::<AssistantConversation>(CONVERSATIONS)
                            .find_one(doc! {"_id": &credential.conversation_id, "user_id": &request.requester_id})
                            .await?
                    } else {
                        None
                    }
                } else {
                    None
                };
                identities.insert(
                    identity.clone(),
                    thread.map(|t| (t.agent_id.unwrap_or_default(), t.guest_turn)),
                );
            }
            let Some(Some((requester, guest))) = identities.get(&identity) else {
                continue;
            };
            if *guest || agent.is_some_and(|target| target != requester) {
                continue;
            }
            if super::assistant_acknowledgement_service::cached_full_access(
                db,
                modes,
                &request.requester_id,
                requester,
            )
            .await?
            {
                ids.push(request.id);
            }
        }
        let ids: Vec<_> = ids.iter().map(String::as_str).collect();
        expire_pending_ids(db, &ids, sweep_time, &marker).await?;
    }
    Ok(())
}

async fn requests_updated_by_expiry_sweep(
    db: &Database,
    candidates: Vec<ApprovalRequest>,
    sweep_marker: &str,
) -> AppResult<Vec<ApprovalRequest>> {
    if candidates.is_empty() {
        return Ok(vec![]);
    }

    let ids: Vec<&str> = candidates
        .iter()
        .map(|request| request.id.as_str())
        .collect();
    let expired_ids: HashSet<String> = db
        .collection::<bson::Document>(REQUESTS)
        .find(doc! {
            "_id": { "$in": &ids },
            "status": "expired",
            "decision_idempotency_key": sweep_marker,
        })
        .with_options(FindOptions::builder().projection(doc! { "_id": 1 }).build())
        .await?
        .try_collect::<Vec<bson::Document>>()
        .await?
        .into_iter()
        .filter_map(|doc| doc.get_str("_id").ok().map(str::to_owned))
        .collect();

    Ok(retain_requests_with_ids(candidates, &expired_ids))
}

fn retain_requests_with_ids(
    requests: Vec<ApprovalRequest>,
    allowed_ids: &HashSet<String>,
) -> Vec<ApprovalRequest> {
    requests
        .into_iter()
        .filter(|request| allowed_ids.contains(&request.id))
        .collect()
}

pub(crate) struct WaitBudget {
    pub received_at: chrono::DateTime<Utc>,
    pub seconds: u32,
}

pub(crate) fn connected_budget_deadline(timeout_secs: u32) -> Option<chrono::DateTime<Utc>> {
    CONNECTED_WAIT_BUDGET
        .try_with(|budget| {
            budget.received_at + Duration::seconds(i64::from(budget.seconds.min(timeout_secs)))
        })
        .ok()
}

tokio::task_local! {
    pub(crate) static CONNECTED_WAIT_BUDGET: WaitBudget;
    pub(crate) static CALLBACK_INGRESS: chrono::DateTime<Utc>;
}

#[cfg(test)]
tokio::task_local! {
    pub(crate) static APPROVAL_CLOCK: tokio::sync::watch::Receiver<chrono::DateTime<Utc>>;
}

fn approval_now() -> chrono::DateTime<Utc> {
    #[cfg(test)]
    if let Ok(now) = APPROVAL_CLOCK.try_with(|clock| *clock.borrow()) {
        return now;
    }
    Utc::now()
}

async fn wait_until_expiry(expires_at: chrono::DateTime<Utc>) {
    #[cfg(test)]
    if let Ok(mut clock) = APPROVAL_CLOCK.try_with(Clone::clone) {
        clock
            .wait_for(|now| *now >= expires_at)
            .await
            .expect("approval fixture owns the clock");
        return;
    }
    let remaining = (expires_at - Utc::now()).to_std().unwrap_or_default();
    tokio::time::sleep(remaining).await;
}

const NYXBOT_CONNECTED_WRITE_SECS: u32 = 80;

/// A per-request HTTP/MCP card has exactly one caller. Arm before insertion:
/// cancellation can drop the handler while publishing or notifying the owner.
/// Grant cards can have several waiters and have their own decision lifetime.
pub struct ConnectedWait {
    db: Database,
    occurrence: Option<String>,
}

impl ConnectedWait {
    pub fn new(
        db: &Database,
        mode: &ApprovalMode,
        operation: &mut ApprovalRequestOperation,
    ) -> Self {
        let occurrence =
            (*mode == ApprovalMode::PerRequest).then(|| uuid::Uuid::new_v4().to_string());
        operation.connected_request_id.clone_from(&occurrence);
        Self {
            db: db.clone(),
            occurrence,
        }
    }

    pub(crate) async fn for_service(
        db: &Database,
        mode: &ApprovalMode,
        operation: &mut ApprovalRequestOperation,
        descriptor: &OperationDescriptor,
        chat: Option<&super::assistant_acknowledgement_service::ChatAuthority>,
        timeout_secs: u32,
    ) -> AppResult<Self> {
        use super::operation_descriptor::Protocol;
        use crate::models::{assistant_conversation, service_approval_config::ApprovalVerb};
        if *mode == ApprovalMode::PerRequest
            && matches!(descriptor.protocol, Protocol::Http | Protocol::Mcp)
            && matches!(
                descriptor.verb,
                ApprovalVerb::Write | ApprovalVerb::Destructive
            )
            && let Ok(ingress) = CALLBACK_INGRESS.try_with(|at| *at)
            && let Some(chat) = chat.filter(|c| {
                c.is_orchestrator()
                    && c.turn_live
                    && !c.guest
                    && !c.execution_voice_task
                    && !c.invocation_controlled
                    && !c.platform_thread
            })
            && let Some(turn) = &chat.turn_id
        {
            // A thread key also serves triggers and background events. Match the
            // exact user-addressed turn. Older turns omit origin and decode as User.
            let interactive = db
                .collection::<Document>(assistant_conversation::COLLECTION_NAME)
                .find_one(
                    doc! {"_id": &chat.conversation_id, "credential_api_key_id": &chat.api_key_id,
                    "active_turn.turn_id": turn, "active_turn.stop_requested": {"$ne": true},
                    "active_turn.origin": {"$in": [null, "user", "channel", "group"]},
                    "active_turn.trigger_run_id": null, "active_turn.automation_work": null,
                    "automation_thread": {"$ne": true}},
                )
                .projection(doc! {"_id": 1})
                .await?
                .is_some();
            if interactive {
                operation.connected_deadline = Some(
                    ingress
                        + Duration::seconds(i64::from(
                            timeout_secs.min(NYXBOT_CONNECTED_WRITE_SECS),
                        )),
                );
            }
        }
        operation.apply_connected_budget(timeout_secs);
        Ok(Self::new(db, mode, operation))
    }

    pub async fn wait(
        mut self,
        request: &ApprovalRequest,
        cancellation: &tokio_util::sync::CancellationToken,
        timeout_secs: u32,
    ) -> AppResult<()> {
        let result = tokio::select! {
            biased;
            () = cancellation.cancelled() => Err(AppError::ClientDisconnected),
            () = wait_until_expiry(request.expires_at) => Err(AppError::Forbidden(
                if connected_budget_deadline(timeout_secs).is_some() { "Approval request timed out" } else { "Approval request expired" }.into()
            )),
            result = poll_for_decision(&self.db, &request.id, timeout_secs) => result,
        };
        if result.is_err()
            && let Some(id) = self.occurrence.as_deref()
        {
            expire_connected(&self.db, id).await?;
        }
        self.occurrence = None;
        result
    }
}

async fn expire_connected(db: &Database, id: &str) -> AppResult<bool> {
    // Only the unique waiter can reach dispatch, and this guard is still
    // armed. An approval that raced its cancellation is also unsent.
    let updated = db
        .collection::<ApprovalRequest>(REQUESTS)
        .update_one(
            doc! {"_id": id, "status":{"$in":["pending","approved"]}, "approval_mode":"per_request",
            "exact_service":null, "execution_proposal":null},
            doc! {"$set":{"status":"expired"}, "$min":{"expires_at":bson::DateTime::now()}},
        )
        .await?;
    Ok(updated.matched_count == 1)
}

impl Drop for ConnectedWait {
    fn drop(&mut self) {
        let Some(id) = self.occurrence.take() else {
            return;
        };
        let db = self.db.clone();
        crate::services::shutdown::process().spawn_cleanup(async move {
            // An insert already sent to Mongo may finish after the future drops.
            for _ in 0..3 {
                match expire_connected(&db, &id).await {
                    Ok(true) => return,
                    Ok(false) => {}
                    Err(_) => {
                        tracing::warn!(request_id=%id, "Failed to expire disconnected approval")
                    }
                }
                tokio::time::sleep(std::time::Duration::from_millis(250)).await;
            }
        });
    }
}

/// Block until an approval decision is made or the timeout expires.
/// Returns Ok(()) if approved, Err if rejected/expired/timeout.
pub async fn wait_for_decision(
    db: &Database,
    request_id: &str,
    timeout_secs: u32,
) -> AppResult<()> {
    let poll = poll_for_decision(db, request_id, timeout_secs);
    if let Some(deadline) = connected_budget_deadline(timeout_secs) {
        tokio::select! {
            biased;
            () = wait_until_expiry(deadline) => Err(AppError::Forbidden("Approval request timed out".into())),
            result = poll => result,
        }
    } else {
        poll.await
    }
}

async fn poll_for_decision(db: &Database, request_id: &str, timeout_secs: u32) -> AppResult<()> {
    let poll_interval = std::time::Duration::from_millis(1000);
    let deadline = approval_now() + Duration::seconds(i64::from(timeout_secs));

    loop {
        tokio::time::sleep(poll_interval).await;

        let request = get_request(db, request_id).await?;
        super::execution_permission_dispatch::proposals::legacy_refusal(&request)?;

        match request.status.as_str() {
            "approved" => return Ok(()),
            "rejected" => {
                return Err(AppError::Forbidden(
                    "Approval request was rejected".to_string(),
                ));
            }
            "expired" => {
                return Err(AppError::Forbidden("Approval request expired".to_string()));
            }
            "pending" => {
                if approval_now() >= request.expires_at {
                    return Err(AppError::Forbidden("Approval request expired".into()));
                }
                if approval_now() >= deadline {
                    return Err(AppError::Forbidden(
                        "Approval request timed out".to_string(),
                    ));
                }
            }
            other => {
                return Err(AppError::Internal(format!(
                    "Unknown approval status: {other}"
                )));
            }
        }
    }
}

/// Convert user-facing approval decision failures into a richer error while
/// preserving unrelated backend errors so their original status/redaction
/// behavior remains intact.
pub fn map_wait_for_decision_error(
    error: AppError,
    request_id: &str,
    frontend_url: &str,
) -> AppError {
    match error {
        AppError::Forbidden(reason) => AppError::ApprovalFailed {
            request_id: request_id.to_string(),
            approve_url: super::assistant_links::AssistantPage::Approvals.url(frontend_url),
            reason,
        },
        other => other,
    }
}

/// Model-facing outcome of a proxy approval wait. A returned identifier is
/// not evidence of a still-pending card or a dispatched operation.
pub fn wait_failure_result(error: AppError, request_id: &str, frontend: &str) -> serde_json::Value {
    let reason = match &error {
        AppError::Forbidden(reason) => reason.as_str(),
        _ => "",
    };
    wait_failure_details(
        reason,
        request_id,
        &super::assistant_links::AssistantPage::Approvals.url(frontend),
    )
}

pub(crate) fn wait_failure_details(
    reason: &str,
    request_id: &str,
    review_url: &str,
) -> serde_json::Value {
    let (code, status, message) = match reason {
        "Approval request expired" => (
            "approval_expired",
            "expired",
            "The approval request expired. Nothing was sent by this call. Ask the owner before requesting approval again.",
        ),
        "Approval request timed out" => (
            "approval_wait_timed_out",
            "unavailable",
            "This call stopped waiting for approval and sent nothing. Check the request in Approvals before requesting another; do not claim a card is still waiting.",
        ),
        "Approval request was rejected" => (
            "approval_denied",
            "rejected",
            "The approval request was denied. Nothing was sent by this call. Do not request it again unless the owner asks.",
        ),
        _ => (
            "approval_unavailable",
            "unavailable",
            "The approval state could not be verified. Nothing was sent by this call; check Approvals before retrying.",
        ),
    };
    serde_json::json!({"error":code,"code":code,"status":status,"request_id":request_id,
        "message":message,"instructions":message,"review_url":review_url})
}

/// Listing-scope descriptor for one admin org branch on the
/// opt-in `include_admin_orgs=true` listing paths. Handlers pre-resolve
/// each admin membership's `allowed_service_ids` (which live in
/// `UserService.id` space) into the set of concrete storage-space
/// `service_id`s that can match an `ApprovalRequest` / `ApprovalGrant`
/// row, so the Mongo filter itself enforces scope. This keeps
/// pagination correct: a scoped admin's empty-page case from the
/// post-fetch filter is eliminated, because we never fetch rows we
/// can't return.
///
/// `service_id_scope` semantics:
/// - `None` — unscoped admin. Every org-owned row (subject to the
///   `from_org_policy` / `org_scoped` flag) passes.
/// - `Some(vec)` — scoped admin. Only rows whose `service_id` is in
///   `vec` pass. An empty `vec` means the admin's scope resolved to no
///   storage-space ids — the caller should drop this branch entirely
///   so the admin sees nothing from that org.
#[derive(Debug, Clone)]
pub struct OrgFilterBranch {
    pub org_id: String,
    pub service_id_scope: Option<Vec<String>>,
}

/// Build the Mongo filter for listing approval requests.
///
/// Empty `admin_branches` preserves the legacy per-user shape so
/// callers that don't opt in to the admin-org union get byte-identical
/// behavior. When non-empty, branches are emitted as `$or` alternatives
/// keyed on each org's `user_id` + `from_org_policy: true`, with a
/// per-branch `service_id: { $in: ... }` when the admin is scoped.
/// Scoped admins whose resolved storage-space id list is empty are
/// dropped from the filter (they see no org rows from that
/// membership).
fn build_requests_filter(user_id: &str, admin_branches: &[OrgFilterBranch]) -> bson::Document {
    let mut branches: Vec<bson::Document> = Vec::new();
    // Personal branch always included so the caller's own rows are
    // part of the unified result.
    branches.push(doc! { "user_id": user_id });

    for branch in admin_branches {
        match &branch.service_id_scope {
            Some(ids) if ids.is_empty() => {
                // Scoped admin with no services in scope — emit nothing.
                continue;
            }
            Some(ids) => {
                let mut ids_bson = bson::Array::new();
                for id in ids {
                    ids_bson.push(bson::Bson::String(id.clone()));
                }
                branches.push(doc! {
                    "user_id": &branch.org_id,
                    "from_org_policy": true,
                    "service_id": { "$in": ids_bson },
                });
            }
            None => {
                branches.push(doc! {
                    "user_id": &branch.org_id,
                    "from_org_policy": true,
                });
            }
        }
    }

    if branches.len() == 1 {
        // Only the personal branch survived. Fall back to the legacy
        // flat shape so callers see the same Document they always did.
        return branches.remove(0);
    }
    let mut arr = bson::Array::new();
    for b in branches {
        arr.push(bson::Bson::Document(b));
    }
    doc! { "$or": arr }
}

/// List approval requests for a user (for history page).
///
/// `admin_branches` widens the listing to also include org-policy
/// requests owned by each supplied admin org, with per-branch scope
/// applied in the Mongo filter so pagination is correct. Callers
/// that don't want the union pass `&[]` and get the historic
/// personal-only result.
///
/// `statuses` accepts zero, one, or many status values. Empty slice
/// means "no status filter" (all statuses returned). Single value
/// becomes an equality predicate (`status: "pending"`). Multiple
/// values become `status: { $in: [...] }` so the history view can
/// ask for "everything except pending" in a single query — the
/// alternative (filter client-side after pagination) stranded
/// decided rows behind pages full of admin-org PENDING items.
pub async fn list_requests(
    db: &Database,
    user_id: &str,
    admin_branches: &[OrgFilterBranch],
    statuses: &[&str],
    page: u64,
    per_page: u64,
) -> AppResult<(Vec<ApprovalRequest>, u64)> {
    list_requests_with_proposals(db, user_id, admin_branches, statuses, page, per_page, false).await
}

#[allow(clippy::too_many_arguments)]
pub async fn list_requests_with_proposals(
    db: &Database,
    user_id: &str,
    admin_branches: &[OrgFilterBranch],
    statuses: &[&str],
    page: u64,
    per_page: u64,
    human: bool,
) -> AppResult<(Vec<ApprovalRequest>, u64)> {
    let mut filter = build_requests_filter(user_id, admin_branches);
    // Org group cards have an explicit participant ACL and are projected by
    // the group transcript, not the organization-wide approvals inbox.
    if human {
        filter = doc! {"$and":[filter,{"$or":[{"assistant_group":null,"execution_proposal":null},{"user_id":user_id,"execution_proposal.version":{"$exists":true}}]}]};
    } else {
        filter.insert("assistant_group", bson::Bson::Null);
        filter.insert("execution_proposal", bson::Bson::Null);
    }
    match statuses {
        [] => {}
        [single] => {
            filter.insert("status", *single);
        }
        many => {
            let mut arr = bson::Array::new();
            for s in many {
                arr.push(bson::Bson::String((*s).to_string()));
            }
            filter.insert("status", doc! { "$in": arr });
        }
    }

    let total = db
        .collection::<ApprovalRequest>(REQUESTS)
        .count_documents(filter.clone())
        .await?;

    let offset = (page.saturating_sub(1)) * per_page;
    let requests: Vec<ApprovalRequest> = db
        .collection::<ApprovalRequest>(REQUESTS)
        .find(filter)
        .with_options(
            FindOptions::builder()
                .sort(doc! { "created_at": -1 })
                .skip(offset)
                .limit(i64::try_from(per_page).unwrap_or(100))
                .build(),
        )
        .await?
        .try_collect()
        .await?;

    Ok((requests, total))
}

/// Get a single approval request by ID (for status polling).
pub async fn get_request(db: &Database, request_id: &str) -> AppResult<ApprovalRequest> {
    read_request(db, request_id, None).await
}

// Keep the snapshot transaction causally after the card read used for authorization.
pub(crate) struct DecisionRequest {
    pub request: ApprovalRequest,
    session: ClientSession,
}

pub(crate) async fn read_decision_request(
    db: &Database,
    request_id: &str,
) -> AppResult<DecisionRequest> {
    let mut session = db.client().start_session().causal_consistency(true).await?;
    let request = read_request(db, request_id, Some(&mut session)).await?;
    Ok(DecisionRequest { request, session })
}

async fn read_request(
    db: &Database,
    request_id: &str,
    session: Option<&mut ClientSession>,
) -> AppResult<ApprovalRequest> {
    let requests = db.collection::<ApprovalRequest>(REQUESTS);
    let find = requests.find_one(doc! { "_id": request_id });
    let request = match session {
        Some(session) => find.session(session).await?,
        None => find.await?,
    };
    request.ok_or_else(|| AppError::NotFound("Approval request not found".to_string()))
}

fn service_config_supports_grants(config: &ServiceApprovalConfig) -> bool {
    // Inventory keeps remembered approvals manageable while a policy is disabled.
    // Execution independently resolves the active policy before grant reuse.
    config.approval_mode == ApprovalMode::Grant
        || config.rules.iter().any(|rule| {
            rule.effect == ApprovalEffect::RequireApproval && rule.mode == ApprovalMode::Grant
        })
}

/// Build the Mongo filter for listing approval grants. Empty
/// `admin_branches` preserves the legacy per-user shape. When
/// non-empty, branches are emitted as `$or` alternatives. Each
/// branch carries its OWN grant-mode service-id set so one owner's
/// `grant_mode` config never leaks another owner's grants (the
/// cross-owner bug codex flagged in round 2). Per-branch scope is
/// also intersected with the mode set so a scoped admin never sees
/// rows outside their `allowed_service_ids`.
///
/// `grant_mode_by_owner` maps an owner `user_id` → service ids with
/// grant capability, whether from the top-level mode or a rule. An
/// owner missing from the map has no grant-capable services and
/// contributes no grants.
fn build_grants_filter(
    user_id: &str,
    admin_branches: &[OrgFilterBranch],
    grant_mode_by_owner: &std::collections::HashMap<String, Vec<String>>,
    now: bson::DateTime,
) -> bson::Document {
    let mut branches: Vec<bson::Document> = Vec::new();

    // Personal branch: only include if the caller has at least one
    // service in grant mode. Otherwise we emit no personal branch at
    // all, matching the legacy "no configs → no grants" behavior.
    if let Some(actor_mode_ids) = grant_mode_by_owner.get(user_id)
        && !actor_mode_ids.is_empty()
    {
        let mut ids_bson = bson::Array::new();
        for id in actor_mode_ids {
            ids_bson.push(bson::Bson::String(id.clone()));
        }
        branches.push(doc! {
            "user_id": user_id,
            "service_id": { "$in": ids_bson },
        });
    }

    // Admin org branches. Each intersects the org's own grant-mode
    // service ids with the admin's scope (if any). Skip branches that
    // resolve to an empty set.
    for branch in admin_branches {
        let Some(mode_ids) = grant_mode_by_owner.get(&branch.org_id) else {
            continue;
        };
        if mode_ids.is_empty() {
            continue;
        }
        let effective: Vec<String> = match &branch.service_id_scope {
            None => mode_ids.clone(),
            Some(scope_ids) => mode_ids
                .iter()
                .filter(|sid| scope_ids.iter().any(|s| s == *sid))
                .cloned()
                .collect(),
        };
        if effective.is_empty() {
            continue;
        }
        let mut ids_bson = bson::Array::new();
        for id in &effective {
            ids_bson.push(bson::Bson::String(id.clone()));
        }
        branches.push(doc! {
            "user_id": &branch.org_id,
            "org_scoped": true,
            "service_id": { "$in": ids_bson },
        });
    }

    let mut filter = doc! {
        "revoked": false,
        "expires_at": { "$gt": now },
    };

    if branches.is_empty() {
        // Nothing matches. Use a predicate that's cheap for Mongo to
        // evaluate as false (synthetic sentinel on the `_id` field).
        filter.insert("_id", doc! { "$in": bson::Array::new() });
    } else if branches.len() == 1 {
        // Inline single branch so the filter stays flat and indexed.
        let only = branches.remove(0);
        for (k, v) in only {
            filter.insert(k, v);
        }
    } else {
        let mut arr = bson::Array::new();
        for b in branches {
            arr.push(bson::Bson::Document(b));
        }
        filter.insert("$or", bson::Bson::Array(arr));
    }

    filter
}

/// List active approval grants for a user.
///
/// Legacy rows require services with current grant capability. Execution rows
/// are listed independently with the same owner/admin visibility boundaries.
/// Capability may come from top-level `Grant` mode (including legacy
/// configs with a missing field) or a `require_approval` rule whose
/// mode is `Grant`. Services with no config default to `PerRequest`
/// and remain excluded. This read-time filter acts as a safety net
/// for write-time races or partial failures during policy changes
/// (see #146).
///
/// `admin_branches` widens the listing to also include org-scoped
/// grants owned by each supplied admin org, with per-branch scope
/// (`service_id_scope`) intersected against that org's own grant-mode
/// configs in the Mongo filter. Callers that don't opt in pass
/// `&[]`.
pub async fn list_grants(
    db: &Database,
    user_id: &str,
    admin_branches: &[OrgFilterBranch],
    page: u64,
    per_page: u64,
) -> AppResult<(Vec<ApprovalGrant>, u64)> {
    let now = bson::DateTime::from_chrono(Utc::now());

    // Collect grant-mode configs for the actor AND each admin org,
    // but keep them grouped BY owner. The per-owner grouping is the
    // fix for the round-2 cross-owner leak: owner A's "grant" config
    // must not allow owner B's grants through.
    let mut config_owner_ids: Vec<String> = vec![user_id.to_string()];
    for b in admin_branches {
        if !config_owner_ids.iter().any(|id| id == &b.org_id) {
            config_owner_ids.push(b.org_id.clone());
        }
    }
    let mut ids_bson = bson::Array::new();
    for id in &config_owner_ids {
        ids_bson.push(bson::Bson::String(id.clone()));
    }
    // Load the owners' complete configs and resolve grant capability in Rust.
    // Filtering on raw BSON `approval_mode` would miss both legacy documents
    // where the field is absent and rule-level grant modes.
    let configs: Vec<ServiceApprovalConfig> = db
        .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
        .find(doc! { "user_id": { "$in": ids_bson } })
        .await?
        .try_collect()
        .await?;
    let mut grant_mode_by_owner: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for c in configs.into_iter().filter(service_config_supports_grants) {
        grant_mode_by_owner
            .entry(c.user_id)
            .or_default()
            .push(c.service_id);
    }
    // De-dupe per owner.
    for v in grant_mode_by_owner.values_mut() {
        let set: HashSet<String> = v.drain(..).collect();
        v.extend(set);
    }

    let legacy = build_grants_filter(user_id, admin_branches, &grant_mode_by_owner, now);
    // Explicit execution permission is independent of legacy Grant mode. Keep
    // the same per-owner/admin service visibility; no additional lookups.
    let mut execution_owners = vec![doc! {"user_id": user_id}];
    for branch in admin_branches {
        let mut owner = doc! {"user_id": &branch.org_id, "org_scoped": true};
        if let Some(ids) = &branch.service_id_scope {
            owner.insert("service_id", doc! {"$in": ids});
        }
        execution_owners.push(owner);
    }
    let filter = doc! {"$or": [legacy, doc! {
        "execution": {"$type": "object"}, "revoked": false,
        "expires_at": {"$gt": now}, "$or": execution_owners,
    }]};

    let total = db
        .collection::<ApprovalGrant>(GRANTS)
        .count_documents(filter.clone())
        .await?;

    let offset = (page.saturating_sub(1)) * per_page;
    let grants: Vec<ApprovalGrant> = db
        .collection::<ApprovalGrant>(GRANTS)
        .find(filter)
        .with_options(
            FindOptions::builder()
                .sort(doc! { "granted_at": -1 })
                .skip(offset)
                .limit(i64::try_from(per_page).unwrap_or(100))
                .build(),
        )
        .await?
        .try_collect()
        .await?;

    Ok((grants, total))
}

#[derive(Clone, Copy)]
pub(crate) enum GrantRevokeSource {
    Web,
    AssistantAction,
}

impl GrantRevokeSource {
    fn intent(
        self,
        actor: &AuditActor,
        user_id: &str,
        grant_id: &str,
    ) -> AppResult<crate::models::security_effect_intent::SecurityEffectIntent> {
        let mut data = serde_json::json!({
            "grant_id": required::text(required::TextField::Identifier, grant_id),
            "owner_user_id": required::text(required::TextField::Identifier, user_id),
        });
        if matches!(self, Self::AssistantAction) {
            data["assistant_action"] = true.into();
        }
        required::intent_preserving_actor(actor, "approval_grant_revoked", Some(data))
    }
}

pub(crate) async fn revoke_grant_with_audit(
    db: &Database,
    user_id: &str,
    grant_id: &str,
    actor: &AuditActor,
    source: GrantRevokeSource,
) -> AppResult<()> {
    use super::api_key_mutation_service::{
        abort_transaction, map_transaction_error, transaction_result,
    };
    let intents = [source.intent(actor, user_id, grant_id)?];
    let filter = doc! { "_id": grant_id, "user_id": user_id };
    let update = revoke_grant_update(Utc::now());
    let db = db.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .write_concern(mongodb::options::WriteConcern::majority())
        .and_run2(async move |session| {
            let result = db
                .collection::<ApprovalGrant>(GRANTS)
                .update_one(filter.clone(), update.clone())
                .session(&mut *session)
                .await?;
            // Legacy grants can already match the pipeline on a repeated revoke.
            if result.matched_count != 1 {
                return Err(abort_transaction(AppError::NotFound(
                    "Grant not found".into(),
                )));
            }
            transaction_result(
                super::security_effect_service::enqueue_in_session(&db, session, &intents).await,
            )
        })
        .await
        .map_err(map_transaction_error)
}

#[cfg(test)]
pub(crate) async fn revoke_grant(db: &Database, user_id: &str, grant_id: &str) -> AppResult<()> {
    let actor = AuditActor {
        user_id: user_id.into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    revoke_grant_with_audit(db, user_id, grant_id, &actor, GrantRevokeSource::Web).await
}

/// Revoke all grants for a user.
#[allow(dead_code)]
pub async fn revoke_all_grants(db: &Database, user_id: &str) -> AppResult<u64> {
    let result = db
        .collection::<ApprovalGrant>(GRANTS)
        .update_many(
            doc! { "user_id": user_id, "revoked": false },
            revoke_grant_update(Utc::now()),
        )
        .await?;

    Ok(result.modified_count)
}

/// List per-service approval configs for a user.
pub async fn list_service_approval_configs(
    db: &Database,
    user_id: &str,
) -> AppResult<Vec<ServiceApprovalConfig>> {
    let configs: Vec<ServiceApprovalConfig> = db
        .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
        .find(doc! { "user_id": user_id })
        .await?
        .try_collect()
        .await?;

    Ok(configs)
}

pub(crate) struct ConfigAudit {
    pub actor: AuditActor,
    pub user_service_id: Option<String>,
}

impl ConfigAudit {
    fn metadata(&self, owner: &str, service: &str) -> serde_json::Value {
        use required::{TextField::Identifier, text};
        serde_json::json!({
            "service_id": text(Identifier, service),
            "user_service_id": self.user_service_id.as_deref().map(|id| text(Identifier, id)),
            "policy_owner_user_id": text(Identifier, owner),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn set_intent(
        &self,
        owner: &str,
        service: &str,
        name: &str,
        required: bool,
        mode: &ApprovalMode,
        rule_count: usize,
        effect: Option<&ApprovalEffect>,
    ) -> AppResult<SecurityEffectIntent> {
        let mut data = self.metadata(owner, service);
        data["service_name"] = required::text(required::TextField::DisplayName, name).into();
        data["approval_required"] = required.into();
        data["approval_mode"] = mode.as_str().into();
        data["approval_rule_count"] = rule_count.into();
        data["default_effect"] = serde_json::json!(effect.map(ApprovalEffect::as_str));
        required::intent_preserving_actor(&self.actor, "service_approval_config_set", Some(data))
    }

    fn delete_intent(&self, owner: &str, service: &str) -> AppResult<SecurityEffectIntent> {
        required::intent_preserving_actor(
            &self.actor,
            "service_approval_config_deleted",
            Some(self.metadata(owner, service)),
        )
    }
}

#[cfg(test)]
fn test_config_audit(id: &str) -> ConfigAudit {
    ConfigAudit {
        actor: AuditActor {
            user_id: id.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        user_service_id: None,
    }
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub(crate) async fn set_service_approval_config(
    db: &Database,
    user_id: &str,
    service_id: &str,
    service_name: &str,
    approval_required: Option<bool>,
    approval_mode: Option<&ApprovalMode>,
    rules: Option<&[ApprovalRule]>,
    default_effect: Option<&ApprovalEffect>,
) -> AppResult<ServiceApprovalConfig> {
    // These component fixtures exercise policy semantics on a supporting fleet.
    // Mixed-version tests use the real handler/options API and seed their own evidence.
    if rules.is_some_and(|rules| rules.iter().any(|rule| !rule.api_key_ids.is_empty())) {
        super::security_readiness_service::ready_test_replica(db).await;
    }
    set_config(
        db,
        user_id,
        service_id,
        service_name,
        approval_required,
        approval_mode,
        rules,
        default_effect,
        test_config_audit(user_id),
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn set_config(
    db: &Database,
    user_id: &str,
    service_id: &str,
    service_name: &str,
    approval_required: Option<bool>,
    approval_mode: Option<&ApprovalMode>,
    rules: Option<&[ApprovalRule]>,
    default_effect: Option<&ApprovalEffect>,
    audit: ConfigAudit,
) -> AppResult<ServiceApprovalConfig> {
    set_config_with_options(
        db,
        user_id,
        service_id,
        service_name,
        approval_required,
        approval_mode,
        rules,
        default_effect,
        audit,
        ConfigWriteOptions::default(),
    )
    .await
}

pub const RULES_READINESS: &str = "approvals:personal-rules-v1";

#[derive(Clone, Default)]
pub(crate) struct ConfigWriteOptions {
    pub grant_expiry_days: Option<Option<u32>>,
    pub default_effect: Option<Option<ApprovalEffect>>,
    pub disabled: Option<bool>,
    /// Empty means the editor observed no policy.
    pub expected_revision: Option<String>,
    #[cfg(test)]
    pub collision_hook: Option<super::api_key_mutation_service::TransactionCollisionHook>,
}

pub fn config_revision(config: &ServiceApprovalConfig) -> AppResult<String> {
    use sha2::{Digest, Sha256};
    let bytes = bson::to_vec(config)
        .map_err(|_| AppError::Internal("Could not read policy revision".into()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

/// Carry only personal rule membership across the committed key lineage.
/// Rotation has already taken the owner's write fence when inserting the
/// successor, so even a policy being created concurrently is serialized.
pub(crate) async fn rotate_personal_rule_memberships_in_session(
    db: &Database,
    owner: &str,
    actor: &str,
    predecessor: &str,
    successor: &str,
    session: &mut ClientSession,
) -> AppResult<()> {
    let configs = db.collection::<Document>(SERVICE_APPROVAL_CONFIGS);
    let filter = doc! {"user_id": owner, "rules.api_key_ids": predecessor};
    let mut cursor = configs
        .find(filter.clone())
        .projection(doc! {"_id": 1, "service_id": 1, "rules": 1})
        .sort(doc! {"_id": 1})
        .session(&mut *session)
        .await?;
    let rows: Vec<Document> = cursor.stream(&mut *session).try_collect().await?;
    if rows.is_empty()
        || db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": owner})
            .session(&mut *session)
            .await?
            .is_some_and(|user| user.user_type.is_org())
    {
        return Ok(());
    }
    let ids: Vec<(&str, &str)> = rows
        .iter()
        .map(|row| {
            row.get_str("_id")
                .and_then(|id| row.get_str("service_id").map(|service| (id, service)))
        })
        .collect::<Result<_, _>>()
        .map_err(|_| AppError::Internal("Invalid approval policy identity".into()))?;
    // Operate on BSON so absent legacy and unrelated rule fields stay intact.
    for (row, (id, _)) in rows.iter().zip(&ids) {
        let mut rules = row
            .get_array("rules")
            .map_err(|_| AppError::Internal("Invalid approval rules".into()))?
            .clone();
        for rule in &mut rules {
            let rule = rule
                .as_document_mut()
                .ok_or_else(|| AppError::Internal("Invalid approval rule".into()))?;
            let Ok(members) = rule.get_array_mut("api_key_ids") else {
                continue;
            };
            if !members
                .iter()
                .any(|member| member.as_str() == Some(predecessor))
            {
                continue;
            }
            let mut successor_seen = false;
            members.retain_mut(|member| {
                if member.as_str() == Some(predecessor) {
                    *member = successor.into();
                }
                if member.as_str() == Some(successor) {
                    return !std::mem::replace(&mut successor_seen, true);
                }
                true
            });
        }
        let result = configs
            .update_one(
                doc! {"_id": *id, "user_id": owner},
                doc! {"$set": {"rules": rules, "updated_at": bson::DateTime::now()}},
            )
            .session(&mut *session)
            .await?;
        if result.matched_count != 1 {
            return Err(AppError::Conflict(
                "Approval policy changed during key rotation".into(),
            ));
        }
    }
    let mut metadata = required::identifier_pairs("policies", ["policy_id", "service_id"], &ids);
    metadata.insert("policy_count".into(), ids.len().into());
    for (field, id) in [
        ("policy_owner_user_id", owner),
        ("predecessor_key_id", predecessor),
        ("successor_key_id", successor),
    ] {
        metadata.insert(
            field.into(),
            required::text(required::TextField::Identifier, id).into(),
        );
    }
    let audit = AuditActor {
        user_id: actor.into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    let intent = required::intent_preserving_actor(
        &audit,
        "service_approval_config_key_rotated",
        Some(metadata.into()),
    )?;
    super::security_effect_service::enqueue_in_session(db, session, &[intent]).await
}

/// Guard both newly selected and retained IDs against a concurrent rotation.
/// The owner fence is also written by key insertion in the rotation transaction.
async fn fence_rule_memberships_in_session(
    db: &Database,
    owner: &str,
    service_id: &str,
    ids: &[&str],
    session: &mut ClientSession,
    #[cfg(test)] collision_hook: Option<&super::api_key_mutation_service::TransactionCollisionHook>,
) -> AppResult<()> {
    if ids.is_empty() {
        return Ok(());
    }
    #[cfg(test)]
    if let Some(hook) = collision_hook {
        hook.begin_attempt();
    }
    let current = db
        .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
        .find_one(doc! {"user_id": owner, "service_id": service_id})
        .session(&mut *session)
        .await?;
    #[cfg(test)]
    if let Some(hook) = collision_hook {
        hook.after_reads().await;
    }
    super::erasure_admission::fence_in_session(db, owner, session).await?;
    let keys = db.collection::<Document>(crate::models::api_key::COLLECTION_NAME);
    if keys
        .find_one(doc! {"user_id": owner, "rotation_predecessor_id": {"$in": ids}})
        .session(&mut *session)
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "An approval rule API key was rotated; reload before saving".into(),
        ));
    }
    let previous: HashSet<&str> = current
        .iter()
        .flat_map(|cfg| &cfg.rules)
        .flat_map(|rule| rule.api_key_ids.iter().map(String::as_str))
        .collect();
    let new_ids: Vec<&str> = ids
        .iter()
        .copied()
        .filter(|id| !previous.contains(id))
        .collect();
    if !new_ids.is_empty() {
        let count = keys
            .count_documents(
                doc! {"_id": {"$in": new_ids}, "user_id": owner, "is_active": true,
                "purpose": {"$nin": ["public_nyxbot", "shared_entry_transport"]},
                "$or": [{"expires_at": null}, {"expires_at": {"$gt": bson::DateTime::now()}}]},
            )
            .session(&mut *session)
            .await?;
        if count != new_ids.len() as u64 {
            return Err(AppError::ValidationError(
                "Approval rules can only be limited to your own active API keys".into(),
            ));
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn set_config_with_options(
    db: &Database,
    user_id: &str,
    service_id: &str,
    service_name: &str,
    approval_required: Option<bool>,
    approval_mode: Option<&ApprovalMode>,
    rules: Option<&[ApprovalRule]>,
    default_effect: Option<&ApprovalEffect>,
    audit: ConfigAudit,
    options: ConfigWriteOptions,
) -> AppResult<ServiceApprovalConfig> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let filter = doc! { "user_id": user_id, "service_id": service_id };
    let existing_document = db
        .collection::<Document>(SERVICE_APPROVAL_CONFIGS)
        .find_one(filter.clone())
        .await?;
    let existing: Option<ServiceApprovalConfig> = existing_document
        .clone()
        .map(bson::from_document)
        .transpose()
        .map_err(|_| AppError::Internal("Could not read policy".into()))?;
    if let Some(days) = options.grant_expiry_days.flatten()
        && !(MIN_GRANT_EXPIRY_DAYS..=MAX_GRANT_EXPIRY_DAYS).contains(&days)
    {
        return Err(AppError::ValidationError(
            "grant_expiry_days must be between 1 and 365".into(),
        ));
    }
    if let Some(expected) = &options.expected_revision {
        let actual = existing
            .as_ref()
            .map(config_revision)
            .transpose()?
            .unwrap_or_default();
        if expected != &actual {
            return Err(AppError::Conflict(
                "Approval policy changed; reload before saving".into(),
            ));
        }
    }
    let (approval_required, approval_mode, rules, default_effect) = resolve_service_config_update(
        existing.as_ref(),
        approval_required,
        approval_mode,
        rules,
        default_effect,
    )?;

    let default_effect = options.default_effect.clone().unwrap_or(default_effect);
    let days = options
        .grant_expiry_days
        .unwrap_or_else(|| existing.as_ref().and_then(|cfg| cfg.grant_expiry_days));
    let disabled = options
        .disabled
        .unwrap_or_else(|| existing.as_ref().is_some_and(|cfg| cfg.disabled));
    // Callers without a reviewed revision keep the original contract: every
    // save is written, audited and reconciled, and the last write wins.
    let legacy = options.expected_revision.is_none();
    if !legacy
        && let Some(existing) = &existing
        && existing.approval_required == approval_required
        && existing.approval_mode == approval_mode
        && existing.rules == rules
        && existing.default_effect == default_effect
        && existing.grant_expiry_days == days
        && existing.disabled == disabled
    {
        return Ok(existing.clone());
    }
    if disabled || days.is_some() || rules.iter().any(|r| !r.api_key_ids.is_empty()) {
        super::security_readiness_service::require_serving(db, RULES_READINESS).await?;
    }
    let reconcile = legacy
        || existing.as_ref().is_none_or(|old| {
            old.approval_required != approval_required
                || old.approval_mode != approval_mode
                || old.rules != rules
                || old.default_effect != default_effect
        });
    let mut intent = audit.set_intent(
        user_id,
        service_id,
        service_name,
        approval_required,
        &approval_mode,
        rules.len(),
        default_effect.as_ref(),
    )?;
    if let crate::models::security_effect_intent::SecurityEffect::Audit { event } =
        &mut intent.effect
        && let Some(data) = event.event_data.as_mut()
    {
        if options.grant_expiry_days.is_some() {
            data["grant_expiry_days"] = serde_json::json!(days);
        }
        if options.disabled.is_some() {
            data["disabled"] = serde_json::json!(disabled);
        }
    }
    let rules_bson = bson::to_bson(&rules)
        .map_err(|e| AppError::Internal(format!("Failed to serialize approval rules: {e}")))?;
    let effect_bson = bson::to_bson(&default_effect).map_err(|e| {
        AppError::Internal(format!("Failed to serialize approval default effect: {e}"))
    })?;
    let config_id = uuid::Uuid::new_v4().to_string();
    let mut set = doc! {
        "approval_required": approval_required,
        "approval_mode": approval_mode.as_str(),
        "rules": rules_bson,
        "default_effect": effect_bson,
        "service_name": service_name,
        "updated_at": now,
    };
    let mut set_on_insert = doc! {
        "_id": &config_id,
        "user_id": user_id,
        "service_id": service_id,
        "created_at": now,
    };
    // A legacy writer cannot see the newer fields, so it never rewrites them.
    let newer = doc! { "disabled": disabled, "grant_expiry_days": days.map(i64::from) };
    if legacy {
        set_on_insert.extend(newer);
    } else {
        set.extend(newer);
    }
    let update = doc! { "$set": set, "$setOnInsert": set_on_insert };
    let ids: HashSet<&str> = rules
        .iter()
        .flat_map(|rule| rule.api_key_ids.iter().map(String::as_str))
        .collect();
    let ids: Vec<&str> = ids.into_iter().collect();
    let mut write_filter = filter.clone();
    // Two audited creators that observed absence must contend for the unique
    // owner/service key even if a transaction body is retried after a conflict.
    if legacy && existing.is_some() {
        // Last write wins for callers without a reviewed revision.
    } else if existing.is_some() {
        // Compare the complete observed policy, including same-millisecond
        // edits, so retrying a transaction cannot overwrite another editor.
        let observed = existing_document
            .as_ref()
            .expect("existing policy document");
        for (field, value) in observed {
            write_filter.insert(field, value.clone());
        }
        for field in [
            "disabled",
            "grant_expiry_days",
            "rules",
            "default_effect",
            "approval_mode",
        ] {
            if !observed.contains_key(field) {
                write_filter.insert(field, doc! {"$exists":false});
            }
        }
    } else {
        write_filter.insert("_id", &config_id);
    }
    for _attempt in 0..2 {
        let context = (
            db,
            &write_filter,
            &update,
            &intent,
            existing.is_none(),
            reconcile,
            legacy,
            user_id,
            service_id,
            &ids,
            &options,
        );
        let result = super::service_history::transaction::run_send(
            db,
            &context,
            |session,
             &(
                source_db,
                upsert_filter,
                update,
                intent,
                create,
                reconcile,
                legacy,
                owner,
                service_id,
                ids,
                _options,
            )| {
                Box::pin(async move {
                    transaction_result(
                        fence_rule_memberships_in_session(
                            source_db,
                            owner,
                            service_id,
                            ids,
                            session,
                            #[cfg(test)]
                            _options.collision_hook.as_ref(),
                        )
                        .await,
                    )?;
                    let row =
                        upsert_config(source_db, upsert_filter, update, create, session).await?;
                    if reconcile && legacy {
                        transaction_result(
                            legacy_reconcile_grant_state_in_session(source_db, &row, session).await,
                        )?;
                    } else if reconcile {
                        transaction_result(
                            reconcile_grant_state_in_session(source_db, &row, session).await,
                        )?;
                    }
                    transaction_result(
                        super::security_effect_service::enqueue_in_session(
                            source_db,
                            session,
                            std::slice::from_ref(intent),
                        )
                        .await,
                    )?;
                    Ok(row)
                })
            },
        )
        .await;
        let cfg = match result {
            Ok(cfg) => cfg,
            Err(error) if error.get_custom::<ConfigCreationRace>().is_some() => {
                if options.expected_revision.is_some() || existing.is_some() {
                    return Err(AppError::Conflict(
                        "Approval policy changed; reload before saving".into(),
                    ));
                }
                // A legacy caller has no reviewed revision. Preserve its
                // creation-race contract by returning the current winner and
                // reconciling that policy, including a winner written by an
                // older replica. The losing proposal produces no audit event.
                let context = (db, &filter);
                let winner = super::service_history::transaction::run_send(
                    db,
                    &context,
                    |session, &(db, filter)| {
                        Box::pin(async move {
                            let winner = db
                                .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
                                .find_one(filter.clone())
                                .session(&mut *session)
                                .await?;
                            if let Some(winner) = &winner {
                                transaction_result(
                                    legacy_reconcile_grant_state_in_session(db, winner, session)
                                        .await,
                                )?;
                            }
                            Ok(winner)
                        })
                    },
                )
                .await
                .map_err(map_transaction_error)?;
                let Some(winner) = winner else {
                    continue;
                };
                winner
            }
            Err(error) => return Err(map_transaction_error(error)),
        };
        return Ok(cfg);
    }
    Err(AppError::Conflict(
        "Per-service approval config update conflicted, please retry".into(),
    ))
}

#[derive(Debug)]
struct ConfigCreationRace;

async fn upsert_config(
    db: &Database,
    filter: &Document,
    update: &Document,
    create: bool,
    session: &mut ClientSession,
) -> mongodb::error::Result<ServiceApprovalConfig> {
    let collection = db.collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS);
    collection
        .find_one_and_update(filter.clone(), update.clone())
        .upsert(create)
        .return_document(ReturnDocument::After)
        .session(session)
        .await
        .map_err(|error| {
            if is_duplicate_key_error(&error) {
                mongodb::error::Error::custom(ConfigCreationRace)
            } else {
                error
            }
        })?
        .ok_or_else(|| {
            super::api_key_mutation_service::abort_transaction(AppError::Conflict(
                "Approval policy changed; reload before saving".into(),
            ))
        })
}

pub const MIN_GRANT_EXPIRY_DAYS: u32 = 1;
pub const MAX_GRANT_EXPIRY_DAYS: u32 = 365;

/// Delete a per-service approval config (revert to global default).
/// Because the global default mode is `PerRequest`, deleting any override
/// effectively switches the service to `per_request`. Legacy service grants,
/// pending requests and the audit intent change atomically with the policy.
#[cfg(test)]
pub(crate) async fn delete_service_approval_config(
    db: &Database,
    user_id: &str,
    service_id: &str,
    audit: ConfigAudit,
) -> AppResult<()> {
    delete_config_with_revision(db, user_id, service_id, audit, None).await
}

pub(crate) async fn delete_config_with_revision(
    db: &Database,
    user_id: &str,
    service_id: &str,
    audit: ConfigAudit,
    expected_revision: Option<&str>,
) -> AppResult<()> {
    let intent = audit.delete_intent(user_id, service_id)?;
    let filter = doc! {"user_id":user_id,"service_id":service_id};
    let context = (db, &filter, &intent, expected_revision, user_id, service_id);
    let deleted = super::service_history::transaction::run_send(
        db,
        &context,
        |session, &(db, filter, intent, expected_revision, user_id, service_id)| {
            Box::pin(async move {
                if let Some(expected) = expected_revision {
                    let current = db.collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
                        .find_one(filter.clone())
                        .session(&mut *session)
                        .await?;
                    if let Some(current) = current {
                        let actual = transaction_result(config_revision(&current))?;
                        if expected != actual {
                            return transaction_result(Err(AppError::Conflict(
                                "Approval policy changed; reload before removing".into(),
                            )));
                        }
                    }
                }
                let count = db.collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
                    .delete_one(filter.clone())
                    .session(&mut *session)
                    .await?
                    .deleted_count;
                // Legacy DELETE stays a full revocation: every active grant for
                // the service (including execution permissions) and pending
                // grant-mode requests, even when a retry finds the policy gone.
                // It now commits with the deletion instead of after it.
                if expected_revision.is_none() {
                    db.collection::<ApprovalGrant>(GRANTS)
                        .update_many(
                            doc! {"user_id": user_id, "service_id": service_id, "revoked": false},
                            revoke_grant_update(Utc::now()),
                        )
                        .session(&mut *session)
                        .await?;
                    db.collection::<ApprovalRequest>(REQUESTS)
                        .update_many(
                            doc! {
                                "user_id": user_id, "service_id": service_id, "status": "pending",
                                "approval_mode": {"$in": ["grant", null]},
                            },
                            doc! {"$set": {"status": "expired", "decided_at": bson::DateTime::now()}},
                        )
                        .session(&mut *session)
                        .await?;
                } else if count == 1 {
                    // Revision-aware removal preserves execution permissions
                    // and proposals.
                    db.collection::<ApprovalGrant>(GRANTS)
                        .update_many(
                            doc! {"user_id": user_id, "service_id": service_id, "revoked": false, "execution": null},
                            doc! {"$set": {"revoked": true}},
                        )
                        .session(&mut *session)
                        .await?;
                    db.collection::<ApprovalRequest>(REQUESTS)
                        .update_many(
                            doc! {"user_id": user_id, "service_id": service_id, "status": "pending", "execution_proposal": null},
                            doc! {"$set": {"status": "expired", "decided_at": bson::DateTime::now()}},
                        )
                        .session(&mut *session)
                        .await?;
                }
                if count == 1 {
                    transaction_result(
                        super::security_effect_service::enqueue_in_session(
                            db, session, std::slice::from_ref(intent),
                        ).await,
                    )?;
                }
                Ok(count)
            })
        },
    )
    .await
    .map_err(map_transaction_error)?;

    if expected_revision.is_none() && deleted == 0 {
        return Err(AppError::NotFound(
            "Per-service approval config not found".to_string(),
        ));
    }

    Ok(())
}

/// Whether an enabled personal policy limits any rule to specific API keys.
async fn personal_policy_is_key_limited(
    db: &Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(SERVICE_APPROVAL_CONFIGS)
        .count_documents(doc! {
            "user_id": user_id,
            "service_id": service_id,
            "disabled": {"$ne": true},
            "rules.api_key_ids.0": {"$exists": true},
        })
        .limit(1)
        .await?
        > 0)
}

fn compute_idempotency_key(
    user_id: &str,
    service_id: &str,
    requester_type: &str,
    requester_id: &str,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(user_id.as_bytes());
    hasher.update(b":");
    hasher.update(service_id.as_bytes());
    hasher.update(b":");
    hasher.update(requester_type.as_bytes());
    hasher.update(b":");
    hasher.update(requester_id.as_bytes());
    hex::encode(hasher.finalize())
}

/// Dedupe key for an org-policy pending request. The key intentionally omits
/// `requester_type` / `requester_id` so that concurrent calls from different
/// org members against the same org-owned service collapse into a single
/// pending request (rather than producing per-member prompts that each have
/// to be decided separately). A sentinel string keeps the key distinct from
/// the personal `compute_idempotency_key` output, so a pre-existing personal
/// pending row on the same `(user, service)` cannot collide with an org row.
fn compute_org_idempotency_key(user_id: &str, service_id: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(user_id.as_bytes());
    hasher.update(b":");
    hasher.update(service_id.as_bytes());
    hasher.update(b":org:*");
    hex::encode(hasher.finalize())
}

fn compute_pending_request_idempotency_key(
    approval_mode: &ApprovalMode,
    user_id: &str,
    service_id: &str,
    requester_type: &str,
    requester_id: &str,
    from_org_policy: bool,
) -> String {
    match approval_mode {
        ApprovalMode::Grant if from_org_policy => compute_org_idempotency_key(user_id, service_id),
        ApprovalMode::Grant => {
            compute_idempotency_key(user_id, service_id, requester_type, requester_id)
        }
        ApprovalMode::PerRequest => uuid::Uuid::new_v4().to_string(),
    }
}

fn resolve_service_config_update(
    existing: Option<&ServiceApprovalConfig>,
    approval_required: Option<bool>,
    approval_mode: Option<&ApprovalMode>,
    rules: Option<&[ApprovalRule]>,
    default_effect: Option<&ApprovalEffect>,
) -> AppResult<(
    bool,
    ApprovalMode,
    Vec<ApprovalRule>,
    Option<ApprovalEffect>,
)> {
    let granular_field_supplied = rules.is_some() || default_effect.is_some();
    let resolved_required =
        match approval_required.or_else(|| existing.map(|c| c.approval_required)) {
            Some(required) => required,
            None if granular_field_supplied => false,
            None => {
                return Err(AppError::ValidationError(
                    "approval_required is required when creating a new per-service approval config"
                        .to_string(),
                ));
            }
        };

    let resolved_mode = approval_mode
        .cloned()
        .or_else(|| existing.map(|config| config.approval_mode.clone()))
        .unwrap_or_default();

    let resolved_rules = rules
        .map(<[ApprovalRule]>::to_vec)
        .or_else(|| existing.map(|config| config.rules.clone()))
        .unwrap_or_default();
    if existing.is_none_or(|config| config.rules != resolved_rules) {
        if resolved_rules.len() > approval_policy::MAX_APPROVAL_RULES {
            return Err(AppError::ValidationError("Too many approval rules".into()));
        }
        let changed: Vec<_> = resolved_rules
            .iter()
            .filter(|rule| existing.is_none_or(|cfg| !cfg.rules.contains(rule)))
            .cloned()
            .collect();
        approval_policy::validate_rules(&changed).map_err(AppError::ValidationError)?;
    }

    // Omission always preserves the stored default. Explicit null clears it.
    let resolved_default_effect = default_effect
        .cloned()
        .or_else(|| existing.and_then(|config| config.default_effect.clone()));

    Ok((
        resolved_required,
        resolved_mode,
        resolved_rules,
        resolved_default_effect,
    ))
}

#[cfg(test)]
async fn revoke_grants_for_service(
    db: &Database,
    user_id: &str,
    service_id: &str,
    unscoped_only: bool,
) -> AppResult<()> {
    let mut filter = doc! {
        "user_id": user_id,
        "service_id": service_id,
        "revoked": false,
    };
    if unscoped_only {
        filter.insert("scope", bson::Bson::Null);
    }
    db.collection::<ApprovalGrant>(GRANTS)
        .update_many(filter, revoke_grant_update(Utc::now()))
        .await?;
    Ok(())
}

/// Existing legacy expiry is unchanged. Explicit permissions use a bounded
/// terminal TTL, including persistent rows; no extra read and no index change.
pub(crate) fn revoke_grant_update(now: chrono::DateTime<Utc>) -> Vec<bson::Document> {
    vec![doc! {"$set": {
        "revoked": true,
        "expires_at": {"$cond": [
            {"$eq": [{"$type": "$execution"}, "object"]},
            bson::DateTime::from_chrono(super::execution_permission::terminal_expiry(now)),
            "$expires_at"
        ]}
    }}]
}

fn is_duplicate_key_error(e: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    match e.kind.as_ref() {
        ErrorKind::Write(WriteFailure::WriteError(write)) => write.code == 11000,
        ErrorKind::Command(command) => command.code == 11000,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[tokio::test]
    async fn approval_change_intent_bson_maxima() {
        let raw = "x".repeat(4096);
        let actor = AuditActor {
            user_id: raw.clone(),
            ip_address: Some(raw.clone()),
            user_agent: Some(raw.clone()),
            api_key_id: Some(raw.clone()),
            api_key_name: Some(raw.clone()),
        };
        let audit = ConfigAudit {
            actor: actor.clone(),
            user_service_id: Some(raw.clone()),
        };
        let ctx = crate::turn_trace::TurnContext::for_turn(&Uuid::new_v4().to_string());
        let intents = crate::turn_trace::scope(ctx, async {
            [
                DecisionSource::Web(actor)
                    .intent(&raw, &raw, &raw, true)
                    .unwrap(),
                DecisionSource::Telegram
                    .intent(&raw, &raw, &raw, true)
                    .unwrap(),
                audit
                    .set_intent(
                        &raw,
                        &raw,
                        &raw,
                        true,
                        &ApprovalMode::PerRequest,
                        approval_policy::MAX_APPROVAL_RULES,
                        Some(&ApprovalEffect::RequireApproval),
                    )
                    .unwrap(),
                audit.delete_intent(&raw, &raw).unwrap(),
            ]
        })
        .await;
        for intent in intents {
            let crate::models::security_effect_intent::SecurityEffect::Audit { ref event } =
                intent.effect
            else {
                panic!("audit")
            };
            let data = event.event_data.as_ref().unwrap();
            assert_eq!(data["service_id"].as_str().unwrap().len(), 256);
            assert!(
                data["service_id"]
                    .as_str()
                    .unwrap()
                    .ends_with("...[truncated]")
            );
            if let Some(name) = data.get("service_name") {
                assert_eq!(name.as_str().unwrap().len(), 512);
            }
            let total = bson::to_vec(&intent).unwrap().len();
            assert!(total < 16 * 1024);
            println!(
                "APPROVAL_MAX type={} channel={} event_bson={} intent_bson={}",
                event.event_type,
                data.get("channel")
                    .and_then(|v| v.as_str())
                    .unwrap_or("none"),
                bson::to_vec(event).unwrap().len(),
                total
            );
        }
    }

    #[tokio::test]
    async fn grant_revoke_audit_maximum_bson() {
        let raw = "x".repeat(4096);
        let actor = AuditActor {
            user_id: raw.clone(),
            ip_address: Some(raw.clone()),
            user_agent: Some(raw.clone()),
            api_key_id: Some(raw.clone()),
            api_key_name: Some(raw.clone()),
        };
        let context = crate::turn_trace::TurnContext::for_turn(&uuid::Uuid::new_v4().to_string());
        for source in [GrantRevokeSource::Web, GrantRevokeSource::AssistantAction] {
            let intent = crate::turn_trace::scope(context, async {
                source.intent(&actor, &raw, &raw).unwrap()
            })
            .await;
            let crate::models::security_effect_intent::SecurityEffect::Audit { ref event } =
                intent.effect
            else {
                panic!("expected audit intent");
            };
            let data = event.event_data.as_ref().unwrap();
            for field in ["grant_id", "owner_user_id"] {
                let value = data[field].as_str().unwrap();
                assert_eq!(value.len(), 256);
                assert!(value.ends_with("...[truncated]"));
            }
            let bytes = bson::to_vec(event).unwrap().len();
            let total = bson::to_vec(&intent).unwrap().len();
            assert!(total < 16 * 1024);
            println!(
                "REVOKE_MAX assistant={} event_bson={bytes} intent_bson={total}",
                matches!(source, GrantRevokeSource::AssistantAction)
            );
        }
    }

    #[tokio::test]
    async fn approval_wait_2705_only_interactive_nyxbot_service_writes_use_callback_deadline() {
        use crate::services::{
            assistant_authority_tests::orchestrator_fixture, operation_descriptor,
        };
        let f = orchestrator_fixture("nyxbot_deadline_scope").await;
        let now = Utc::now();
        let http = operation_descriptor::build_http_descriptor("POST", "/items", None);
        let read = operation_descriptor::build_http_descriptor("GET", "/items", None);
        let ssh = operation_descriptor::build_ssh_descriptor(
            operation_descriptor::SshOperationKind::Exec,
            None,
        );
        let mut specialist = f.chat.clone();
        specialist.role = crate::models::assistant_conversation::AgentRole::Subagent;
        let deadline = |mode: ApprovalMode, descriptor: OperationDescriptor, chat| {
            let db = &f.state.db;
            let caller_key = &f.chat.api_key_id;
            async move {
                let mut operation = ApprovalRequestOperation::from_descriptor(&descriptor, None);
                operation.fresh_decision = true;
                operation.caller_api_key_id = Some(caller_key.clone());
                let expected_key = operation.caller_api_key_id.clone();
                let expected_protocol = operation.protocol.clone();
                let mut waiter =
                    ConnectedWait::for_service(db, &mode, &mut operation, &descriptor, chat, 300)
                        .await
                        .unwrap();
                assert!(operation.fresh_decision);
                assert_eq!(operation.caller_api_key_id, expected_key);
                assert_eq!(operation.protocol, expected_protocol);
                assert_eq!(operation.connected_request_id, waiter.occurrence);
                // This policy check created no row to clean up.
                waiter.occurrence = None;
                operation.connected_deadline
            }
        };
        CALLBACK_INGRESS
            .scope(now, async {
                assert_eq!(
                    deadline(ApprovalMode::PerRequest, http.clone(), Some(&f.chat)).await,
                    Some(now + Duration::seconds(80))
                );
                assert_eq!(
                    deadline(ApprovalMode::PerRequest, http.clone(), None).await,
                    None
                );
                assert_eq!(
                    deadline(ApprovalMode::Grant, http.clone(), Some(&f.chat)).await,
                    None
                );
                assert_eq!(
                    deadline(ApprovalMode::PerRequest, read, Some(&f.chat)).await,
                    None
                );
                assert_eq!(
                    deadline(ApprovalMode::PerRequest, ssh, Some(&f.chat)).await,
                    None
                );
                assert_eq!(
                    deadline(ApprovalMode::PerRequest, http.clone(), Some(&specialist)).await,
                    None
                );
                f.state
                    .db
                    .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id": &f.row.id},
                        doc! {"$set": {"active_turn.origin": "trigger"}},
                    )
                    .await
                    .unwrap();
                assert_eq!(
                    deadline(ApprovalMode::PerRequest, http, Some(&f.chat)).await,
                    None
                );
            })
            .await;
    }

    #[tokio::test]
    async fn approval_wait_2874_grant_requests_share_a_prompt_unless_the_caller_sets_a_budget() {
        let db = crate::test_utils::connect_test_database("approval_grant_wait_budget")
            .await
            .expect("configured Mongo required");
        let state = crate::test_utils::test_app_state(db.clone());
        let owner = Uuid::new_v4().to_string();
        let create = |deadline| {
            let mut operation = ApprovalRequestOperation::from_descriptor(&test_operation(), None);
            operation.connected_deadline = Some(deadline);
            create_approval_request(
                &db,
                &state.config,
                &state.http_client,
                None,
                None,
                &owner,
                "test-service",
                "Test service",
                "test-service",
                "api_key",
                "agent",
                None,
                operation,
                ApprovalMode::Grant,
                300,
                vec![],
                false,
            )
        };

        let ingress = Utc::now();
        let deadline = ingress + Duration::seconds(i64::from(NYXBOT_CONNECTED_WRITE_SECS));
        let shared = create(deadline).await.unwrap();
        let later = create(deadline + Duration::seconds(1)).await.unwrap();
        assert_eq!(later.id, shared.id);
        assert_eq!(later.status, "pending");
        assert_eq!(
            shared.idempotency_key,
            compute_idempotency_key(&owner, "test-service", "api_key", "agent")
        );

        let budget = WaitBudget {
            received_at: ingress,
            seconds: NYXBOT_CONNECTED_WRITE_SECS,
        };
        let budgeted = CONNECTED_WAIT_BUDGET
            .scope(budget, create(deadline))
            .await
            .unwrap();
        assert_ne!(budgeted.id, shared.id);
        assert_eq!(budgeted.status, "pending");
    }

    #[tokio::test]
    async fn noninteractive_callers_refuse_new_service_and_tool_requests_before_side_effects() {
        let db = crate::test_utils::connect_test_database("approval_no_request_writers")
            .await
            .expect("configured Mongo required");
        let state = crate::test_utils::test_app_state(db.clone());
        let owner = Uuid::new_v4().to_string();
        let descriptor = test_operation();
        let mut operation = ApprovalRequestOperation::from_descriptor(&descriptor, None);
        operation.approval = ApprovalOptions::NO_REQUEST;
        let service_request = create_approval_request(
            &db,
            &state.config,
            &state.http_client,
            None,
            None,
            &owner,
            "test-service",
            "Test service",
            "test-service",
            "api_key",
            "test-caller",
            None,
            operation,
            ApprovalMode::PerRequest,
            300,
            vec![owner.clone()],
            false,
        )
        .await;
        assert!(matches!(
            service_request,
            Err(AppError::ApprovalUnsupported)
        ));
        let tool_request = create_tool_approval_request(
            &db,
            &state.config,
            &state.http_client,
            None,
            None,
            &owner,
            "test_tool",
            None,
            None,
            false,
            "api_key",
            "test-caller",
            None,
            ApprovalOptions::NO_REQUEST,
        )
        .await;
        assert!(matches!(tool_request, Err(AppError::ApprovalUnsupported)));
        for name in [REQUESTS, CHANNELS] {
            assert_eq!(
                db.collection::<Document>(name)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0,
                "{name}"
            );
        }
        // Other callers retain the ordinary default, including request creation.
        let supported = create_tool_approval_request(
            &db,
            &state.config,
            &state.http_client,
            None,
            None,
            &owner,
            "test_tool",
            None,
            None,
            false,
            "api_key",
            "test-caller",
            None,
            ApprovalOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(supported.status, "pending");
        assert_eq!(
            db.collection::<Document>(REQUESTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
    }

    #[tokio::test]
    async fn new_approval_requests_respect_account_erasure_at_persistence() {
        use crate::models::user::UserType;
        use crate::test_utils::{connect_transaction_test_database, test_app_state, test_user};
        let db = connect_transaction_test_database("approval_erasure_admission").await;
        let state = test_app_state(db.clone());
        let owner = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        super::super::personal_erasure_service::ensure_indexes(&db)
            .await
            .unwrap();

        let service_request = |requester| {
            create_approval_request(
                &db,
                &state.config,
                &state.http_client,
                None,
                None,
                &owner,
                "test-service",
                "Test service",
                "test-service",
                "api_key",
                requester,
                None,
                ApprovalRequestOperation::from_descriptor(&test_operation(), None),
                ApprovalMode::PerRequest,
                300,
                vec![owner.clone()],
                false,
            )
        };
        let tool_request = |requester| {
            create_tool_approval_request(
                &db,
                &state.config,
                &state.http_client,
                None,
                None,
                &owner,
                "test_tool",
                None,
                None,
                false,
                "api_key",
                requester,
                None,
                ApprovalOptions::default(),
            )
        };
        assert_eq!(
            service_request("before-erasure").await.unwrap().status,
            "pending"
        );
        assert_eq!(
            tool_request("before-erasure").await.unwrap().status,
            "pending"
        );
        super::super::personal_erasure_service::accept(&db, &owner, &owner, None)
            .await
            .unwrap();
        let filter = doc! {"user_id": &owner};
        let requests = db.collection::<Document>(REQUESTS);
        let channels = db.collection::<Document>(CHANNELS);
        let before = requests.count_documents(filter.clone()).await.unwrap();
        channels.delete_many(filter.clone()).await.unwrap();

        assert!(matches!(
            service_request("after-erasure").await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            tool_request("after-erasure").await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            requests.count_documents(filter.clone()).await.unwrap(),
            before
        );
        assert_eq!(channels.count_documents(filter).await.unwrap(), 0);
    }

    #[tokio::test]
    async fn noninteractive_policy_retains_denials_and_existing_grants() {
        let db = crate::test_utils::connect_test_database("approval_no_request_policy")
            .await
            .expect("configured Mongo required");
        let owner = Uuid::new_v4().to_string();
        let service = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let read =
            crate::services::operation_descriptor::build_http_descriptor("GET", "/status", None);
        assert!(matches!(
            evaluate_and_check_with_options(
                &db,
                &owner,
                &owner,
                &service,
                &read,
                Some("api_key"),
                "ordinary-client",
                false,
                false,
                ApprovalOptions::NO_REQUEST,
                None,
            )
            .await
            .unwrap(),
            ApprovalOutcome::Allowed { required: false }
        ));
        set_service_approval_config(
            &db,
            &owner,
            &service,
            "Test",
            Some(true),
            Some(&ApprovalMode::Grant),
            None,
            None,
        )
        .await
        .unwrap();
        let descriptor = test_operation();
        let evaluate = || {
            evaluate_and_check_with_options(
                &db,
                &owner,
                &owner,
                &service,
                &descriptor,
                Some("api_key"),
                "ordinary-client",
                false,
                false,
                ApprovalOptions::NO_REQUEST,
                None,
            )
        };
        let ApprovalOutcome::NeedsApproval(pending) = evaluate().await.unwrap() else {
            panic!("The missing grant must still require approval");
        };
        assert!(matches!(
            approval_notification_recipients(&db, &owner, &pending).await,
            Err(AppError::ApprovalUnsupported)
        ));
        assert!(matches!(
            evaluate_and_check_with_options(
                &db,
                &owner,
                &owner,
                &service,
                &descriptor,
                Some("api_key"),
                "ordinary-client",
                true,
                false,
                ApprovalOptions::NO_REQUEST,
                None,
            )
            .await
            .unwrap(),
            ApprovalOutcome::Allowed { required: true }
        ));
        insert_grant(
            &db,
            &make_grant(&owner, &service, "api_key", "ordinary-client", false, 1),
        )
        .await;
        assert!(matches!(
            evaluate().await.unwrap(),
            ApprovalOutcome::Allowed { required: true }
        ));
        set_service_approval_config(
            &db,
            &owner,
            &service,
            "Test",
            Some(true),
            Some(&ApprovalMode::Grant),
            None,
            Some(&ApprovalEffect::Deny),
        )
        .await
        .unwrap();
        assert!(matches!(evaluate().await.unwrap(), ApprovalOutcome::Denied));
        assert!(matches!(
            evaluate_and_check_with_options(
                &db,
                &owner,
                &owner,
                &service,
                &descriptor,
                Some("api_key"),
                "ordinary-client",
                true,
                false,
                ApprovalOptions::NO_REQUEST,
                None,
            )
            .await
            .unwrap(),
            ApprovalOutcome::Denied
        ));
        assert_eq!(
            db.collection::<Document>(REQUESTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<Document>(CHANNELS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn first_tool_request_uses_new_timeout_and_preserves_legacy_read_default() {
        let db = crate::test_utils::connect_test_database("tool_first_request_timeout")
            .await
            .expect("configured Mongo required");
        let state = crate::test_utils::test_app_state(db.clone());
        let owner = uuid::Uuid::new_v4().to_string();
        assert_eq!(
            db.collection::<NotificationChannel>(CHANNELS)
                .count_documents(doc! {"user_id": &owner})
                .await
                .unwrap(),
            0
        );
        for expected in [300, 300, 30] {
            if expected == 30 {
                db.collection::<NotificationChannel>(CHANNELS)
                    .update_one(
                        doc! {"user_id": &owner},
                        doc! {"$unset": {"approval_timeout_secs": ""}},
                    )
                    .await
                    .unwrap();
            }
            let request = create_tool_approval_request(
                &db,
                &state.config,
                &state.http_client,
                None,
                None,
                &owner,
                "fixture_tool",
                None,
                None,
                false,
                "api_key",
                "fixture_requester",
                None,
                ApprovalOptions::default(),
            )
            .await
            .unwrap();
            assert_eq!(request.status, "pending");
            assert!(request.notification_channel.is_none());
            assert_eq!(
                (request.expires_at - request.created_at).num_seconds(),
                expected
            );
            let stored = get_request(&db, &request.id).await.unwrap();
            assert_eq!(
                stored.expires_at.timestamp_millis(),
                request.expires_at.timestamp_millis()
            );
            assert_eq!(
                (stored.expires_at - stored.created_at).num_seconds(),
                expected
            );
            assert_eq!(
                notification_service::get_or_create_channel(&db, &owner)
                    .await
                    .unwrap()
                    .approval_timeout_secs,
                expected as u32
            );
        }
    }

    #[tokio::test]
    async fn required_web_approval_without_delivery_creates_lists_decides_and_expires() {
        use axum::{
            Json,
            extract::{Path, State},
            http::HeaderMap,
        };
        let db = crate::test_utils::connect_test_database("required_web_approval")
            .await
            .expect("configured Mongo required");
        let state = crate::test_utils::test_app_state(db.clone());
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        // No prior settings GET, channel insert, device or provider credentials.
        assert!(
            user_requires_approval(
                &db,
                &owner,
                &crate::models::service_approval_config::ApprovalVerb::Write
            )
            .await
            .unwrap()
        );
        assert_eq!(
            db.collection::<NotificationChannel>(CHANNELS)
                .count_documents(doc! {"user_id": &owner})
                .await
                .unwrap(),
            0
        );
        let operation = crate::services::operation_descriptor::build_http_descriptor(
            "POST",
            "/v1/actions",
            None,
        );
        for mode in 0..4 {
            let mut channel = notification_service::get_or_create_channel(&db, &owner)
                .await
                .unwrap();
            match mode {
                0 => {}
                1 => {
                    channel.telegram_enabled = true;
                    channel.telegram_chat_id = Some(42);
                } // server provider absent
                2 => {
                    channel.push_enabled = true;
                    channel.push_devices = vec![crate::models::notification_channel::DeviceToken {
                        device_id: "dummy".into(),
                        platform: "fcm".into(),
                        token: "dummy-registration".into(),
                        device_name: None,
                        app_id: None,
                        registered_at: Utc::now(),
                        last_used_at: None,
                    }];
                }
                _ => {
                    channel.push_devices.clear();
                    channel.telegram_chat_id = None;
                    channel.telegram_enabled = false;
                } // removed/invalid device and disconnected channel
            }
            db.collection::<NotificationChannel>(CHANNELS)
                .replace_one(doc! {"_id": &channel.id}, &channel)
                .await
                .unwrap();
            assert!(!notification_service::approval_delivery_configured(
                &channel,
                &state.config,
                false,
                false
            ));
            for decision in [Some(true), Some(false), None] {
                let service = uuid::Uuid::new_v4().to_string();
                assert!(matches!(
                    evaluate_and_check(
                        &db,
                        &owner,
                        &owner,
                        &service,
                        &operation,
                        Some("api_key"),
                        "agent",
                        false,
                        false,
                        None,
                    )
                    .await
                    .unwrap(),
                    ApprovalOutcome::NeedsApproval(_)
                ));
                let request = create_approval_request(
                    &db,
                    &state.config,
                    &state.http_client,
                    None,
                    None,
                    &owner,
                    &service,
                    "Fixture",
                    "fixture",
                    "api_key",
                    "agent",
                    None,
                    ApprovalRequestOperation::from_descriptor(&operation, None),
                    ApprovalMode::PerRequest,
                    300,
                    vec![],
                    false,
                )
                .await
                .unwrap();
                assert_eq!(request.status, "pending");
                assert!(request.notification_channel.is_none());
                let (listed, _) = list_requests(&db, &owner, &[], &["pending"], 1, 100)
                    .await
                    .unwrap();
                assert!(listed.iter().any(|r| r.id == request.id));
                if let Some(approved) = decision {
                    let Json(result) = crate::handlers::approvals::decide_request(
                        State(state.clone()),
                        crate::test_utils::test_auth_user(&owner),
                        Path(request.id.clone()),
                        Default::default(),
                        HeaderMap::new(),
                        Json(crate::handlers::approvals::DecideRequest {
                            approved,
                            duration_sec: None,
                        }),
                    )
                    .await
                    .unwrap();
                    assert_eq!(
                        result.status,
                        if approved { "approved" } else { "rejected" }
                    );
                    assert_eq!(
                        wait_for_decision(&db, &request.id, 1).await.is_ok(),
                        approved
                    );
                    assert_eq!(
                        get_request(&db, &request.id)
                            .await
                            .unwrap()
                            .decision_channel
                            .as_deref(),
                        Some("web")
                    );
                } else {
                    db.collection::<ApprovalRequest>(REQUESTS).update_one(doc! {"_id": &request.id}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
                    assert_eq!(
                        expire_pending_requests(
                            &db,
                            &state.config,
                            &state.http_client,
                            None,
                            None,
                            None
                        )
                        .await
                        .unwrap(),
                        1
                    );
                    assert!(wait_for_decision(&db, &request.id, 1).await.is_err());
                }
                assert_eq!(
                    db.collection::<ApprovalGrant>(GRANTS)
                        .count_documents(doc! {"service_id": &service})
                        .await
                        .unwrap(),
                    0
                );
            }
        }
        // Clearing the choice restores writes-only, without changing any
        // previously stored true/false preference on read.
        db.collection::<NotificationChannel>(CHANNELS)
            .update_one(
                doc! {"user_id": &owner},
                doc! {"$unset": {"approval_required": ""}},
            )
            .await
            .unwrap();
        assert!(
            user_requires_approval(
                &db,
                &owner,
                &crate::models::service_approval_config::ApprovalVerb::Write
            )
            .await
            .unwrap()
        );
        assert!(
            !user_requires_approval(
                &db,
                &owner,
                &crate::models::service_approval_config::ApprovalVerb::Read
            )
            .await
            .unwrap()
        );
    }

    #[tokio::test]
    async fn writes_only_default_survives_lazy_tool_request_and_preserves_guest_org_and_deny_controls()
     {
        use crate::models::service_approval_config::ApprovalVerb;
        use approval_policy::GlobalApprovalMode;
        let db = crate::test_utils::connect_test_database("writes_only_policy_controls")
            .await
            .expect("real Mongo required");
        let owner = uuid::Uuid::new_v4().to_string();
        let org = uuid::Uuid::new_v4().to_string();
        let service = uuid::Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_many([
                crate::test_utils::test_user(&owner, crate::models::user::UserType::Person),
                crate::test_utils::test_user(&org, crate::models::user::UserType::Org),
            ])
            .await
            .unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        for verb in [
            ApprovalVerb::Read,
            ApprovalVerb::Write,
            ApprovalVerb::Destructive,
        ] {
            let mut operation = test_operation();
            operation.verb = verb.clone();
            let outcome = evaluate_and_check(
                &db,
                &owner,
                &owner,
                &service,
                &operation,
                Some("api_key"),
                "guest",
                false,
                false,
                None,
            )
            .await
            .unwrap();
            if verb == ApprovalVerb::Read {
                assert!(matches!(
                    outcome,
                    ApprovalOutcome::Allowed { required: false }
                ));
                crate::services::agent_operation_scope_service::check_guest_approval(
                    true, &outcome,
                )
                .unwrap();
            } else {
                assert!(matches!(outcome, ApprovalOutcome::NeedsApproval(_)));
                assert!(
                    crate::services::agent_operation_scope_service::check_guest_approval(
                        true, &outcome
                    )
                    .is_err()
                );
            }
        }
        assert_eq!(
            db.collection::<ApprovalRequest>(REQUESTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        create_tool_approval_request(
            &db,
            &state.config,
            &state.http_client,
            None,
            None,
            &owner,
            "fixture_write",
            None,
            None,
            false,
            "api_key",
            "agent",
            None,
            ApprovalOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(
            user_approval_mode(&db, &owner).await.unwrap(),
            GlobalApprovalMode::WritesOnly
        );
        assert!(
            !user_requires_approval(&db, &owner, &ApprovalVerb::Read)
                .await
                .unwrap()
        );
        let raw = db
            .collection::<Document>(CHANNELS)
            .find_one(doc! {"user_id":&owner})
            .await
            .unwrap()
            .unwrap();
        assert!(!raw.contains_key("approval_required"));
        // Existing stored booleans keep their exact meaning, even for rows
        // lazily created by the unreleased W9.3 build (no provenance marker).
        for (stored, mode) in [
            (true, GlobalApprovalMode::All),
            (false, GlobalApprovalMode::Off),
        ] {
            db.collection::<Document>(CHANNELS)
                .update_one(
                    doc! {"user_id":&owner},
                    doc! {"$set":{"approval_required":stored}},
                )
                .await
                .unwrap();
            notification_service::get_or_create_channel(&db, &owner)
                .await
                .unwrap();
            assert_eq!(user_approval_mode(&db, &owner).await.unwrap(), mode);
            for verb in [
                ApprovalVerb::Read,
                ApprovalVerb::Write,
                ApprovalVerb::Destructive,
            ] {
                assert_eq!(
                    user_requires_approval(&db, &owner, &verb).await.unwrap(),
                    stored
                );
            }
            let after = db
                .collection::<Document>(CHANNELS)
                .find_one(doc! {"user_id":&owner})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(after.get_bool("approval_required").unwrap(), stored);
        }
        // An org's explicit policy wins over the actor's explicit Off.
        set_service_approval_config(
            &db,
            &org,
            &service,
            "Org resource",
            Some(true),
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let read = test_operation();
        let org_outcome = evaluate_and_check(
            &db,
            &owner,
            &org,
            &service,
            &read,
            Some("api_key"),
            "agent",
            false,
            false,
            None,
        )
        .await
        .unwrap();
        assert!(
            matches!(org_outcome,ApprovalOutcome::NeedsApproval(ref p) if p.resolution.from_org_policy && p.primary_owner_user_id == org)
        );
        assert!(matches!(
            evaluate_and_check(
                &db, &owner, &org, &service, &read, None, "session", true, false, None,
            )
            .await
            .unwrap(),
            ApprovalOutcome::Allowed { required: true }
        ));
        set_service_approval_config(
            &db,
            &org,
            &service,
            "Org resource",
            None,
            None,
            None,
            Some(&ApprovalEffect::Deny),
        )
        .await
        .unwrap();
        assert!(matches!(
            evaluate_and_check(
                &db, &owner, &org, &service, &read, None, "session", true, true, None,
            )
            .await
            .unwrap(),
            ApprovalOutcome::Denied
        ));
    }

    fn test_operation() -> OperationDescriptor {
        crate::services::operation_descriptor::build_http_descriptor("GET", "/v1/models", None)
    }

    fn make_request(id: &str, status: &str) -> ApprovalRequest {
        let now = Utc::now();
        ApprovalRequest {
            fresh_decision: false,
            caller_api_key_id: None,
            approval_protocol: None,
            grant_expiry_days_snapshot: None,
            activity_outcome: None,
            activity_execution: None,
            execution_proposal: None,
            assistant_group: None,
            id: id.to_string(),
            user_id: "user-1".to_string(),
            service_id: "service-1".to_string(),
            service_name: "OpenAI API".to_string(),
            service_slug: "openai".to_string(),
            requester_type: "service_account".to_string(),
            requester_id: "requester-1".to_string(),
            requester_label: Some("CI Pipeline".to_string()),
            operation_summary: "proxy:POST /v1/chat/completions".to_string(),
            action_description: None,
            http_method: Some("POST".to_string()),
            resource: Some("/v1/chat/completions".to_string()),
            verb: Some("write".to_string()),
            grant_scope: None,
            status: status.to_string(),
            idempotency_key: format!("idem-{id}"),
            notification_channel: Some("telegram".to_string()),
            telegram_message_id: Some(12345),
            telegram_chat_id: Some(67890),
            expires_at: now,
            decided_at: None,
            decision_channel: None,
            decision_idempotency_key: None,
            tool_name: None,
            tool_call_id: None,
            tool_arguments: None,
            is_destructive: None,
            approval_mode: ApprovalMode::PerRequest,
            notify_user_ids: vec![],
            from_org_policy: false,
            exact_service: None,
            created_at: now,
        }
    }

    #[tokio::test]
    async fn execution_revocation_preserves_legacy_expiry_and_unscoped_filter() {
        let db = crate::test_utils::connect_test_database("execution_permission_revoke_filter")
            .await
            .expect("review Mongo required");
        let rows = db.collection::<ApprovalGrant>(GRANTS);
        let execution = super::super::execution_permission::tests::fixture();
        let mut legacy = execution.clone();
        legacy.id = uuid::Uuid::new_v4().to_string();
        legacy.execution = None;
        legacy.scope = None;
        legacy.expires_at = chrono::DateTime::from_timestamp_millis(
            (Utc::now() + chrono::Duration::hours(1)).timestamp_millis(),
        )
        .unwrap();
        rows.insert_many([&execution, &legacy]).await.unwrap();
        revoke_grants_for_service(&db, "actor", "catalog", true)
            .await
            .unwrap();
        let current = rows
            .find_one(doc! {"_id": &execution.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!current.revoked);
        assert_eq!(current.expires_at, execution.expires_at);
        let old = rows
            .find_one(doc! {"_id": &legacy.id})
            .await
            .unwrap()
            .unwrap();
        assert!(old.revoked);
        assert_eq!(old.expires_at, legacy.expires_at);
        revoke_grants_for_service(&db, "actor", "catalog", false)
            .await
            .unwrap();
        let current = rows
            .find_one(doc! {"_id": &execution.id})
            .await
            .unwrap()
            .unwrap();
        assert!(current.revoked);
        assert!(current.expires_at < Utc::now() + chrono::Duration::days(31));
        db.drop().await.unwrap();
    }

    #[test]
    fn grant_mode_recheck_only_applies_to_legacy_unscoped_grants() {
        let mut request = make_request("req-1", "pending");
        request.approval_mode = ApprovalMode::Grant;
        request.grant_scope = None;
        assert!(requires_legacy_grant_mode_recheck(&request));

        request.grant_scope = Some("v1:http:post:write:/v1/chat/**".to_string());
        assert!(!requires_legacy_grant_mode_recheck(&request));

        request.approval_mode = ApprovalMode::PerRequest;
        request.grant_scope = None;
        assert!(!requires_legacy_grant_mode_recheck(&request));
    }

    #[test]
    fn exact_semantic_effect_lookup_uses_producer_endpoint_identity() {
        let binding = ExactServiceApprovalBinding {
            request_key: "request-key".to_string(),
            actor_user_id: "actor-1".to_string(),
            user_service_id: "user-service-1".to_string(),
            endpoint_id: "producer-endpoint".to_string(),
            catalog_digest: "catalog-digest".to_string(),
            exact_view_digest: Some("view-digest".to_string()),
            exact_view_digest_binding: None,
            endpoint_contract_digest: "contract-digest".to_string(),
            operation_digest: "operation-digest".to_string(),
            operation_id: "legacy-caller-operation".to_string(),
            operation_generation: 1,
            producer_generation_bound: true,
            effect_idempotency_key: "effect-key".to_string(),
            arguments: serde_json::json!({}),
            execution_authority_digest: None,
            execution_authority_binding: None,
            optional_provider_authority_digest: None,
            redemption: None,
        };

        let filter = exact_service_semantic_effect_filter("delegated", "client-1", &binding);
        assert_eq!(
            filter
                .get_str("exact_service.endpoint_id")
                .expect("semantic lookup uses endpoint identity"),
            "producer-endpoint"
        );
        assert!(!filter.contains_key("exact_service.operation_id"));
    }

    #[test]
    fn resolve_approval_requirement_prefers_per_service_true_over_global_false() {
        assert!(resolve_approval_requirement(Some(true), Some(false)));
    }

    #[test]
    fn resolve_approval_requirement_prefers_per_service_false_over_global_true() {
        assert!(!resolve_approval_requirement(Some(false), Some(true)));
    }

    #[test]
    fn resolve_approval_requirement_falls_back_to_global_when_no_per_service() {
        assert!(resolve_approval_requirement(None, Some(true)));
        assert!(!resolve_approval_requirement(None, Some(false)));
    }

    #[test]
    fn resolve_approval_requirement_defaults_to_required_when_no_settings() {
        assert!(resolve_approval_requirement(None, None));
    }

    #[test]
    fn grant_mode_idempotency_key_deterministic() {
        let key1 = compute_pending_request_idempotency_key(
            &ApprovalMode::Grant,
            "user1",
            "svc1",
            "sa",
            "req1",
            false,
        );
        let key2 = compute_pending_request_idempotency_key(
            &ApprovalMode::Grant,
            "user1",
            "svc1",
            "sa",
            "req1",
            false,
        );
        assert_eq!(key1, key2);
    }

    #[test]
    fn grant_mode_idempotency_key_differs_for_different_inputs() {
        let key1 = compute_pending_request_idempotency_key(
            &ApprovalMode::Grant,
            "user1",
            "svc1",
            "sa",
            "req1",
            false,
        );
        let key2 = compute_pending_request_idempotency_key(
            &ApprovalMode::Grant,
            "user2",
            "svc1",
            "sa",
            "req1",
            false,
        );
        assert_ne!(key1, key2);
    }

    #[test]
    fn grant_mode_idempotency_key_is_hex_sha256() {
        let key = compute_pending_request_idempotency_key(
            &ApprovalMode::Grant,
            "u",
            "s",
            "t",
            "r",
            false,
        );
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn per_request_idempotency_key_is_unique_uuid() {
        let key1 = compute_pending_request_idempotency_key(
            &ApprovalMode::PerRequest,
            "user1",
            "svc1",
            "sa",
            "req1",
            false,
        );
        let key2 = compute_pending_request_idempotency_key(
            &ApprovalMode::PerRequest,
            "user1",
            "svc1",
            "sa",
            "req1",
            false,
        );

        assert_ne!(key1, key2);
        assert!(uuid::Uuid::parse_str(&key1).is_ok());
        assert!(uuid::Uuid::parse_str(&key2).is_ok());
    }

    #[test]
    fn org_policy_grant_mode_idempotency_key_ignores_requester() {
        // Regression guard for ChronoAIProject/NyxID#364: when an org-policy
        // grant-mode request is pending, a second call from a different
        // member of the same org must collapse onto the same idempotency key
        // so both members wait on a single admin decision instead of each
        // spawning their own approval prompt.
        let key_member_a = compute_pending_request_idempotency_key(
            &ApprovalMode::Grant,
            "org-1",
            "svc-1",
            "api_key",
            "member-a-key",
            true,
        );
        let key_member_b = compute_pending_request_idempotency_key(
            &ApprovalMode::Grant,
            "org-1",
            "svc-1",
            "api_key",
            "member-b-key",
            true,
        );
        assert_eq!(key_member_a, key_member_b);
    }

    #[test]
    fn org_policy_grant_mode_idempotency_key_is_hex_sha256() {
        let key = compute_org_idempotency_key("org-1", "svc-1");
        assert_eq!(key.len(), 64);
        assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn org_policy_grant_mode_idempotency_key_differs_from_personal() {
        // A personal pending request (from_org_policy=false) for the same
        // (user, service, requester) tuple must not collide with the
        // org-policy wildcard key, so an org request cannot silently inherit
        // a stale personal pending row.
        let personal = compute_pending_request_idempotency_key(
            &ApprovalMode::Grant,
            "org-1",
            "svc-1",
            "api_key",
            "member-a-key",
            false,
        );
        let org = compute_pending_request_idempotency_key(
            &ApprovalMode::Grant,
            "org-1",
            "svc-1",
            "api_key",
            "member-a-key",
            true,
        );
        assert_ne!(personal, org);
    }

    #[test]
    fn idempotent_replay_allows_same_key_same_decision() {
        let result = is_idempotent_replay("approved", Some("k1"), Some("k1"), true).expect("ok");
        assert!(result);
    }

    #[test]
    fn idempotent_replay_rejects_same_key_different_decision() {
        let result = is_idempotent_replay("approved", Some("k1"), Some("k1"), false);
        assert!(matches!(
            result,
            Err(AppError::Conflict(msg)) if msg == "idempotency_key_reused_with_different_decision"
        ));
    }

    #[test]
    fn idempotent_replay_rejects_without_matching_key() {
        let result = is_idempotent_replay("approved", Some("k1"), Some("k2"), true).expect("ok");
        assert!(!result);
    }

    #[test]
    fn resolve_grant_expiry_prefers_duration_seconds() {
        let now = Utc::now();
        let expiry = resolve_grant_expiry(now, Some(3600), 30);
        assert_eq!(expiry, now + Duration::seconds(3600));
    }

    #[test]
    fn resolve_grant_expiry_falls_back_to_default_days() {
        let now = Utc::now();
        let expiry = resolve_grant_expiry(now, None, 30);
        assert_eq!(expiry, now + Duration::days(30));
    }

    #[test]
    fn idempotent_replay_returns_conflict_for_expired_status() {
        // When a request has been marked "expired" (by the background sweep),
        // is_idempotent_replay should return Err(Conflict) because "expired"
        // is not a user decision ("approved"/"rejected").
        let result = is_idempotent_replay("expired", None, Some("k1"), true);
        assert!(matches!(result, Err(AppError::Conflict(msg)) if msg == "decision_state_conflict"));
    }

    #[test]
    fn idempotent_replay_returns_conflict_for_pending_status() {
        // "pending" is not a decided state either.
        let result = is_idempotent_replay("pending", None, Some("k1"), true);
        assert!(matches!(result, Err(AppError::Conflict(msg)) if msg == "decision_state_conflict"));
    }

    #[test]
    fn retain_requests_with_ids_excludes_requests_not_updated_by_sweep() {
        let candidates = vec![
            make_request("req-expired", "pending"),
            make_request("req-approved", "pending"),
        ];
        let actually_expired_ids = HashSet::from_iter([String::from("req-expired")]);

        let retained = retain_requests_with_ids(candidates, &actually_expired_ids);

        assert_eq!(retained.len(), 1);
        assert_eq!(retained[0].id, "req-expired");
    }

    /// Regression test for #96: simulates the race window between find() and
    /// update_many() in expire_pending_requests(). When a user approves a
    /// request after the sweep's initial find() but before the guarded
    /// update_many(), the sweep must NOT send side effects (Telegram edits,
    /// push notifications) for that request.
    ///
    /// This test exercises the filtering pipeline that runs after
    /// update_many(): only requests whose IDs appear in the "actually
    /// expired" set (those that matched `status: "pending"` at update time
    /// AND carry this sweep's unique marker) should trigger side effects.
    #[test]
    fn race_window_approved_request_excluded_from_expiry_side_effects() {
        // Scenario: sweep finds 3 pending requests that have passed their
        // expiry time. Between find() and update_many():
        //  - req-2 was approved by the user (status changed to "approved")
        //  - req-3 was rejected by the user (status changed to "rejected")
        // Only req-1 remained "pending" and was actually expired.
        let candidates = vec![
            make_request("req-1", "pending"),
            make_request("req-2", "pending"), // concurrently approved
            make_request("req-3", "pending"), // concurrently rejected
        ];

        // After update_many with `status: "pending"` guard, only req-1 was
        // modified. The re-query (requests_updated_by_expiry_sweep) returns
        // only IDs that have status "expired" + this sweep's marker.
        let actually_expired_ids = HashSet::from_iter([String::from("req-1")]);

        let side_effect_targets = retain_requests_with_ids(candidates, &actually_expired_ids);

        // req-2 and req-3 must NOT appear — they were decided by the user
        // during the race window and must not receive expiry side effects.
        assert_eq!(side_effect_targets.len(), 1);
        assert_eq!(side_effect_targets[0].id, "req-1");
    }

    /// Regression test for #96: when ALL requests in the sweep's find()
    /// were concurrently decided by users, the side-effect list must be
    /// completely empty.
    #[test]
    fn race_window_all_requests_decided_yields_no_side_effects() {
        let candidates = vec![
            make_request("req-a", "pending"),
            make_request("req-b", "pending"),
        ];

        // None of the candidates were actually expired (all were decided
        // between find() and update_many()).
        let actually_expired_ids: HashSet<String> = HashSet::new();

        let side_effect_targets = retain_requests_with_ids(candidates, &actually_expired_ids);

        assert!(
            side_effect_targets.is_empty(),
            "No side effects should be emitted when all requests were decided during the race window"
        );
    }

    #[test]
    fn resolve_service_config_update_handles_defaults_for_key_limited_rules() {
        let keyed = ApprovalRule {
            mandatory_review: false,
            methods: vec![],
            resource_pattern: "*".to_string(),
            verbs: vec![],
            effect: ApprovalEffect::Deny,
            mode: ApprovalMode::PerRequest,
            api_key_ids: vec!["key-a".to_string()],
        };
        let existing = ServiceApprovalConfig {
            disabled: false,
            grant_expiry_days: None,
            id: uuid::Uuid::new_v4().to_string(),
            user_id: "user1".to_string(),
            service_id: "svc1".to_string(),
            service_name: "OpenAI".to_string(),
            approval_required: true,
            approval_mode: ApprovalMode::PerRequest,
            rules: vec![],
            default_effect: Some(ApprovalEffect::RequireApproval),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let rules = [keyed];

        // Partial clients omit the default to preserve its existing meaning.
        // The HTTP tri-state options support an explicit null to clear it.
        let (_, _, _, preserved) =
            resolve_service_config_update(Some(&existing), None, None, Some(&rules), None)
                .expect("ok");
        assert_eq!(preserved, Some(ApprovalEffect::RequireApproval));

        // Key rules plus a default for everyone else is a valid combined policy.
        let (_, _, _, combined) = resolve_service_config_update(
            Some(&existing),
            None,
            None,
            Some(&rules),
            Some(&ApprovalEffect::AutoAllow),
        )
        .expect("ok");
        assert_eq!(combined, Some(ApprovalEffect::AutoAllow));
    }

    #[test]
    fn resolve_service_config_update_preserves_existing_mode_when_omitted() {
        let existing = ServiceApprovalConfig {
            disabled: false,
            grant_expiry_days: None,
            id: uuid::Uuid::new_v4().to_string(),
            user_id: "user1".to_string(),
            service_id: "svc1".to_string(),
            service_name: "OpenAI".to_string(),
            approval_required: true,
            approval_mode: ApprovalMode::Grant,
            rules: vec![],
            default_effect: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let (approval_required, approval_mode, rules, default_effect) =
            resolve_service_config_update(Some(&existing), Some(false), None, None, None)
                .expect("ok");

        assert!(!approval_required);
        assert_eq!(approval_mode, ApprovalMode::Grant);
        assert!(rules.is_empty());
        assert_eq!(default_effect, None);
    }

    #[test]
    fn resolve_service_config_update_requires_approval_required_for_new_config() {
        let result =
            resolve_service_config_update(None, None, Some(&ApprovalMode::Grant), None, None);
        assert!(matches!(result, Err(AppError::ValidationError(_))));
    }

    #[test]
    fn invisible_card_2072_proxy_wait_failure_states_are_not_pending() {
        for (reason, code, status) in [
            ("Approval request expired", "approval_expired", "expired"),
            (
                "Approval request timed out",
                "approval_wait_timed_out",
                "unavailable",
            ),
            (
                "Approval request was rejected",
                "approval_denied",
                "rejected",
            ),
            ("private diagnostic", "approval_unavailable", "unavailable"),
        ] {
            let result = wait_failure_result(
                AppError::Forbidden(reason.into()),
                "request",
                "https://app.example/",
            );
            assert_eq!(result["code"], code);
            assert_eq!(result["status"], status);
            assert_eq!(
                result["review_url"],
                "https://app.example/assistant/approvals"
            );
            assert!(!result.to_string().contains("private diagnostic"));
        }
    }

    #[tokio::test]
    async fn invisible_card_2072_proxy_history_retains_expiry_and_uses_same_scope() {
        let f = crate::services::assistant_authority_tests::orchestrator_fixture(
            "card2072_proxy_history",
        )
        .await;
        let key =
            crate::services::key_service::get_api_key(&f.state.db, &f.owner, &f.chat.api_key_id)
                .await
                .unwrap();
        let mut rows = Vec::new();
        for (id, status, past) in [
            ("pending", "pending", false),
            ("lazy-expired", "pending", true),
            ("denied", "rejected", false),
            ("allowed", "approved", true),
            ("expired", "expired", true),
        ] {
            let mut row = make_request(id, status);
            row.user_id = f.owner.clone();
            row.requester_id = f.owner.clone();
            row.requester_type = "user".into();
            row.requester_label = Some(key.name.clone());
            row.expires_at = Utc::now() + Duration::seconds(if past { -1 } else { 60 });
            rows.push(row);
        }
        let mut foreign = rows[0].clone();
        foreign.id = "other-chat".into();
        foreign.requester_label = Some("another key".into());
        rows.push(foreign);
        f.state
            .db
            .collection::<ApprovalRequest>(REQUESTS)
            .insert_many(rows)
            .await
            .unwrap();
        let history = crate::services::assistant_nyxagent::approval_history(
            &f.state.db,
            &f.owner,
            &key.id,
            &[],
        )
        .await
        .unwrap();
        assert_eq!(history.len(), 5);
        for (id, status) in [
            ("lazy-expired", "pending"),
            ("allowed", "approved"),
            ("expired", "expired"),
        ] {
            assert_eq!(history.iter().find(|r| r.id == id).unwrap().status, status);
        }
        assert!(
            history
                .iter()
                .all(|r| r.review_path == "/assistant/approvals")
        );
        let error = wait_for_decision(&f.state.db, "lazy-expired", 60)
            .await
            .unwrap_err();
        assert_eq!(
            wait_failure_result(error, "lazy-expired", "https://app.example")["code"],
            "approval_expired"
        );
    }

    #[test]
    fn map_wait_for_decision_error_wraps_user_decision_failures() {
        let error = map_wait_for_decision_error(
            AppError::Forbidden("Approval request timed out".to_string()),
            "req-1",
            "https://app.nyxid.dev/",
        );

        assert!(matches!(
            error,
            AppError::ApprovalFailed {
                request_id,
                approve_url,
                reason,
            } if request_id == "req-1"
                && approve_url == "https://app.nyxid.dev/assistant/approvals"
                && reason == "Approval request timed out"
        ));
    }

    #[test]
    fn map_wait_for_decision_error_preserves_internal_failures() {
        let error = map_wait_for_decision_error(
            AppError::DatabaseError(mongodb::error::Error::custom("db unavailable")),
            "req-1",
            "https://app.nyxid.dev",
        );

        assert!(matches!(error, AppError::DatabaseError(_)));
    }

    fn unscoped_branch(org_id: &str) -> OrgFilterBranch {
        OrgFilterBranch {
            org_id: org_id.to_string(),
            service_id_scope: None,
        }
    }

    fn scoped_branch(org_id: &str, ids: &[&str]) -> OrgFilterBranch {
        OrgFilterBranch {
            org_id: org_id.to_string(),
            service_id_scope: Some(ids.iter().map(|s| s.to_string()).collect()),
        }
    }

    #[test]
    fn build_requests_filter_without_admin_branches_keeps_legacy_shape() {
        // Empty admin_branches must produce the historic single-owner
        // filter so callers that don't opt in see byte-identical
        // behavior.
        let filter = build_requests_filter("user-1", &[]);
        assert_eq!(filter.get_str("user_id").unwrap(), "user-1");
        assert!(filter.get("$or").is_none());
    }

    #[test]
    fn build_requests_filter_with_unscoped_admin_unions_every_org_row() {
        let filter = build_requests_filter(
            "user-1",
            &[unscoped_branch("org-a"), unscoped_branch("org-b")],
        );
        let branches = filter.get_array("$or").expect("$or present");
        assert_eq!(branches.len(), 3);

        assert_eq!(
            branches[0]
                .as_document()
                .unwrap()
                .get_str("user_id")
                .unwrap(),
            "user-1"
        );
        // Each org branch keys on its own org_id and requires
        // from_org_policy = true. No service_id restriction.
        for (i, org) in ["org-a", "org-b"].iter().enumerate() {
            let b = branches[i + 1].as_document().unwrap();
            assert_eq!(b.get_str("user_id").unwrap(), *org);
            assert!(b.get_bool("from_org_policy").unwrap());
            assert!(b.get("service_id").is_none());
        }
    }

    #[test]
    fn build_requests_filter_scoped_admin_applies_service_id_in() {
        let filter =
            build_requests_filter("user-1", &[scoped_branch("org-a", &["svc-1", "svc-2"])]);
        let branches = filter.get_array("$or").expect("$or present");
        assert_eq!(branches.len(), 2);
        let org = branches[1].as_document().unwrap();
        let ids = org
            .get_document("service_id")
            .unwrap()
            .get_array("$in")
            .unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0].as_str().unwrap(), "svc-1");
        assert_eq!(ids[1].as_str().unwrap(), "svc-2");
    }

    #[test]
    fn build_requests_filter_scoped_admin_with_empty_scope_drops_branch() {
        // An admin whose scope resolves to no storage-space ids (e.g.
        // every allowed UserService was deleted) must not widen the
        // caller's view at all. The filter should collapse back to
        // the legacy personal-only shape.
        let filter = build_requests_filter("user-1", &[scoped_branch("org-a", &[])]);
        assert_eq!(filter.get_str("user_id").unwrap(), "user-1");
        assert!(filter.get("$or").is_none());
    }

    #[test]
    fn build_grants_filter_without_admin_branches_keeps_legacy_shape() {
        let now = bson::DateTime::from_chrono(chrono::Utc::now());
        let mut mode = std::collections::HashMap::new();
        mode.insert("user-1".to_string(), vec!["svc-1".to_string()]);
        let filter = build_grants_filter("user-1", &[], &mode, now);
        assert_eq!(filter.get_str("user_id").unwrap(), "user-1");
        assert!(filter.get("$or").is_none());
        // Grant-mode restriction is inlined on the same document.
        assert!(filter.get("service_id").is_some());
    }

    #[test]
    fn build_grants_filter_keeps_grant_mode_per_owner() {
        // The round-2 cross-owner leak: actor has svc-1 in grant mode
        // but org-a has svc-1 in per_request mode. Org-a must not
        // contribute any branch, so actor's grants for svc-1 show up
        // while org-a's grants for svc-1 do not.
        let now = bson::DateTime::from_chrono(chrono::Utc::now());
        let mut mode = std::collections::HashMap::new();
        mode.insert("user-1".to_string(), vec!["svc-1".to_string()]);
        // org-a has NO entry in the map → no grants from org-a.

        let filter = build_grants_filter("user-1", &[unscoped_branch("org-a")], &mode, now);
        // Only the actor branch survives. Should be inlined, not an $or.
        assert_eq!(filter.get_str("user_id").unwrap(), "user-1");
        assert!(filter.get("$or").is_none());
    }

    #[test]
    fn build_grants_filter_scoped_admin_intersects_scope_with_mode() {
        // Admin is scoped to [svc-1, svc-2], org has svc-2 and svc-3
        // in grant mode. Effective set = {svc-2}.
        let now = bson::DateTime::from_chrono(chrono::Utc::now());
        let mut mode = std::collections::HashMap::new();
        mode.insert(
            "org-a".to_string(),
            vec!["svc-2".to_string(), "svc-3".to_string()],
        );
        let filter = build_grants_filter(
            "user-1",
            &[scoped_branch("org-a", &["svc-1", "svc-2"])],
            &mode,
            now,
        );
        // No actor grant-mode entries, so only the org branch is
        // present — gets inlined since it's the sole branch.
        assert_eq!(filter.get_str("user_id").unwrap(), "org-a");
        assert!(filter.get_bool("org_scoped").unwrap());
        let ids = filter
            .get_document("service_id")
            .unwrap()
            .get_array("$in")
            .unwrap();
        assert_eq!(ids.len(), 1);
        assert_eq!(ids[0].as_str().unwrap(), "svc-2");
    }

    #[test]
    fn build_grants_filter_no_owners_in_grant_mode_returns_empty_match() {
        // Neither the actor nor the admin org has any service in
        // grant mode → the filter must match nothing, not leak
        // unrelated grants via a degenerate $or.
        let now = bson::DateTime::from_chrono(chrono::Utc::now());
        let mode: std::collections::HashMap<String, Vec<String>> = std::collections::HashMap::new();
        let filter = build_grants_filter("user-1", &[unscoped_branch("org-a")], &mode, now);
        // Empty-match sentinel on _id keeps the query valid and
        // indexed while matching zero rows.
        let id_clause = filter.get_document("_id").unwrap();
        let in_arr = id_clause.get_array("$in").unwrap();
        assert_eq!(in_arr.len(), 0);
    }

    #[test]
    fn service_config_grant_capability_includes_rule_level_grant_mode() {
        let mut config = ServiceApprovalConfig {
            disabled: false,
            grant_expiry_days: None,
            id: "cfg-1".to_string(),
            user_id: "user-1".to_string(),
            service_id: "svc-1".to_string(),
            service_name: "Service".to_string(),
            approval_required: true,
            approval_mode: ApprovalMode::PerRequest,
            rules: vec![],
            default_effect: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        config.rules.push(ApprovalRule {
            mandatory_review: false,
            methods: vec!["POST".to_string()],
            resource_pattern: "/v1/chat/**".to_string(),
            verbs: vec![],
            effect: ApprovalEffect::RequireApproval,
            mode: ApprovalMode::Grant,
            api_key_ids: vec![],
        });

        assert!(service_config_supports_grants(&config));
    }

    // ================================================================
    // Integration tests (require running MongoDB)
    // ================================================================

    use crate::models::user::UserType;
    use crate::test_utils::{connect_test_database, test_user};

    #[tokio::test]
    async fn full_access_startup_retires_chat_only_approvals_with_guarded_expiry_lifecycle() {
        use crate::models::{
            assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
            assistant_settings::AccessMode,
        };
        use crate::services::{
            assistant_acknowledgement_service as acks, assistant_settings_service as settings,
        };
        let f = crate::services::assistant_authority_tests::fixture("approval_startup_retirement")
            .await;
        let actor = crate::services::audit_service::AuditActor {
            user_id: f.owner.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        };
        crate::services::assistant_team_service::set_agent_policy(
            &f.state.db,
            &actor,
            &f.chat.agent_id,
            crate::services::assistant_team_service::AgentPolicy {
                model: None,
                access_mode_override: Some(Some(AccessMode::Nyxmode)),
            },
        )
        .await
        .unwrap();
        settings::update(
            &f.state.db,
            &f.owner,
            settings::Update {
                access_mode: Some(AccessMode::Autopilot),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let nyxbot = f
            .state
            .db
            .collection::<AssistantConversation>(CONVERSATIONS)
            .find_one(doc! {"_id": &f.nyxbot_thread})
            .await
            .unwrap()
            .unwrap();
        let full_key = crate::services::key_service::get_api_key(
            &f.state.db,
            &f.owner,
            &nyxbot.credential_api_key_id,
        )
        .await
        .unwrap();
        let auto_key =
            crate::services::key_service::get_api_key(&f.state.db, &f.owner, &f.chat.api_key_id)
                .await
                .unwrap();
        let general_id = crate::services::assistant_authority_tests::ordinary_key(&f).await;
        let general = crate::services::key_service::get_api_key(&f.state.db, &f.owner, &general_id)
            .await
            .unwrap();
        let mut requests = Vec::new();
        for (id, label, status) in [
            ("full", full_key.name.as_str(), "pending"),
            ("fresh", full_key.name.as_str(), "pending"),
            ("auto", auto_key.name.as_str(), "pending"),
            ("general", general.name.as_str(), "pending"),
            ("decided", full_key.name.as_str(), "approved"),
        ] {
            let mut request = make_request(id, status);
            request.fresh_decision = id == "fresh";
            request.user_id = f.owner.clone();
            request.requester_id = f.owner.clone();
            request.requester_type = "user".into();
            request.requester_label = Some(label.into());
            request.expires_at = Utc::now() + Duration::minutes(10);
            request.notification_channel = None;
            requests.push(request);
        }
        f.state
            .db
            .collection::<ApprovalRequest>(REQUESTS)
            .insert_many(requests)
            .await
            .unwrap();
        assert_eq!(
            crate::services::assistant_nyxagent::pending_approvals(
                &f.state.db,
                &f.owner,
                &full_key.id
            )
            .await
            .unwrap()
            .len(),
            2,
            "reads retain legacy and fresh asks"
        );
        acks::retire_full_access_confirmations(&f.state.db, None, None)
            .await
            .unwrap();
        let full = get_request(&f.state.db, "full").await.unwrap();
        assert_eq!(full.status, "expired");
        assert!(full.decided_at.is_some());
        assert!(
            full.decision_idempotency_key
                .as_deref()
                .unwrap()
                .starts_with("expiry:full-access:")
        );
        acks::retire_full_access_confirmations(&f.state.db, None, None)
            .await
            .unwrap();
        let again = get_request(&f.state.db, "full").await.unwrap();
        assert_eq!(full.decided_at, again.decided_at);
        assert_eq!(
            full.decision_idempotency_key,
            again.decision_idempotency_key
        );
        for id in ["auto", "general", "fresh"] {
            let kept = get_request(&f.state.db, id).await.unwrap();
            assert_eq!(kept.status, "pending");
            assert!(kept.decided_at.is_none());
        }
        // A decision won between the candidate read and the expiry write.
        assert_eq!(
            expire_pending_ids(
                &f.state.db,
                &["decided"],
                bson::DateTime::now(),
                "expiry:race"
            )
            .await
            .unwrap(),
            0
        );
        assert_eq!(
            get_request(&f.state.db, "decided").await.unwrap().status,
            "approved"
        );
        crate::services::assistant_team_service::set_agent_policy(
            &f.state.db,
            &actor,
            &f.chat.agent_id,
            crate::services::assistant_team_service::AgentPolicy {
                model: None,
                access_mode_override: Some(None),
            },
        )
        .await
        .unwrap();
        let auto = get_request(&f.state.db, "auto").await.unwrap();
        assert_eq!(
            auto.status, "expired",
            "agent write retires approval-only asks"
        );
        assert!(auto.decided_at.is_some());
        assert_ne!(
            full.decision_idempotency_key, auto.decision_idempotency_key,
            "each sweep has a unique marker"
        );
        assert_eq!(
            get_request(&f.state.db, "general").await.unwrap().status,
            "pending"
        );
        assert_eq!(
            get_request(&f.state.db, "fresh").await.unwrap().status,
            "pending"
        );
        f.state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn full_access_chat_bypass_evaluates_policy_and_grants_once() {
        use crate::models::assistant_settings::AccessMode;
        use mongodb::event::{EventHandler, command::CommandEvent};
        use std::sync::{
            Mutex,
            atomic::{AtomicBool, Ordering},
        };
        let f = crate::services::assistant_authority_tests::orchestrator_fixture(
            "approval_single_evaluation",
        )
        .await;
        crate::services::assistant_settings_service::update(
            &f.state.db,
            &f.owner,
            crate::services::assistant_settings_service::Update {
                access_mode: Some(AccessMode::Autopilot),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let service = uuid::Uuid::new_v4().to_string();
        insert_config(
            &f.state.db,
            &make_service_config(&f.owner, &service, true, ApprovalMode::Grant),
        )
        .await;
        let queries = Arc::new(Mutex::new(Vec::new()));
        let record = queries.clone();
        let enabled = Arc::new(AtomicBool::new(true));
        let monitor = enabled.clone();
        let handler = EventHandler::callback(move |event| {
            if monitor.load(Ordering::SeqCst)
                && let CommandEvent::Started(event) = event
                && let Ok(collection) = event.command.get_str("find")
            {
                record.lock().unwrap().push(collection.to_owned());
            }
        });
        let mut options = mongodb::options::ClientOptions::parse(
            std::env::var("NYXID_TEST_DATABASE_URL").unwrap(),
        )
        .await
        .unwrap();
        options.command_event_handler = Some(handler);
        let db = mongodb::Client::with_options(options)
            .unwrap()
            .database(f.state.db.name());
        let descriptor =
            crate::services::operation_descriptor::build_http_descriptor("POST", "/things", None);
        let (outcome, receipt) = evaluate_for_chat(
            &db,
            &f.owner,
            &f.owner,
            &service,
            &descriptor,
            Some("user"),
            &f.owner,
            false,
            false,
            Some(&f.chat),
            None,
        )
        .await
        .unwrap();
        enabled.store(false, Ordering::SeqCst);
        assert!(matches!(
            outcome,
            ApprovalOutcome::Allowed { required: true }
        ));
        assert_eq!(receipt.unwrap().reason, "approval");
        let queries = queries.lock().unwrap().clone();
        assert_eq!(
            queries
                .iter()
                .filter(|q| q.as_str() == SERVICE_APPROVAL_CONFIGS)
                .count(),
            1
        );
        assert_eq!(queries.iter().filter(|q| q.as_str() == GRANTS).count(), 1);
        db.collection::<Document>(SERVICE_APPROVAL_CONFIGS).update_one(
            doc! {"service_id": &service}, doc! {"$set": {"rules": [{"resource_pattern": "*", "effect": "require_approval", "mode": "per_request", "api_key_ids": [&f.chat.api_key_id]}]}},
        ).await.unwrap();
        for key in [f.chat.api_key_id.as_str(), "another-key"] {
            let (outcome, receipt) = evaluate_for_chat(
                &db,
                &f.owner,
                &f.owner,
                &service,
                &descriptor,
                Some("api_key"),
                &f.owner,
                false,
                false,
                Some(&f.chat),
                Some(key),
            )
            .await
            .unwrap();
            if key == f.chat.api_key_id {
                assert!(
                    matches!(outcome, ApprovalOutcome::NeedsApproval(pending) if pending.resolution.fresh_decision)
                );
                assert!(receipt.is_none());
            } else {
                assert!(matches!(outcome, ApprovalOutcome::Allowed { .. }));
            }
        }
        db.collection::<Document>(SERVICE_APPROVAL_CONFIGS)
            .update_one(doc! {"service_id": &service}, doc! {"$set": {"rules": []}})
            .await
            .unwrap();

        crate::services::assistant_settings_service::update(
            &db,
            &f.owner,
            crate::services::assistant_settings_service::Update {
                access_mode: Some(AccessMode::Nyxmode),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (outcome, receipt) = evaluate_for_chat(
            &db,
            &f.owner,
            &f.owner,
            &service,
            &descriptor,
            Some("user"),
            &f.owner,
            false,
            false,
            Some(&f.chat),
            None,
        )
        .await
        .unwrap();
        assert!(matches!(outcome, ApprovalOutcome::NeedsApproval(_)));
        assert!(receipt.is_none());
        db.collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
            .update_one(
                doc! {"service_id": &service},
                doc! {"$set": {"default_effect": "deny"}},
            )
            .await
            .unwrap();
        crate::services::assistant_settings_service::update(
            &db,
            &f.owner,
            crate::services::assistant_settings_service::Update {
                access_mode: Some(AccessMode::Autopilot),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let (outcome, receipt) = evaluate_for_chat(
            &db,
            &f.owner,
            &f.owner,
            &service,
            &descriptor,
            Some("user"),
            &f.owner,
            false,
            false,
            Some(&f.chat),
            None,
        )
        .await
        .unwrap();
        assert!(matches!(outcome, ApprovalOutcome::Denied));
        assert!(receipt.is_none());
        f.state.db.drop().await.unwrap();
    }

    /// Helper to insert an ApprovalRequest directly into the DB.
    async fn insert_request(db: &mongodb::Database, req: &ApprovalRequest) {
        db.collection::<ApprovalRequest>(REQUESTS)
            .insert_one(req)
            .await
            .expect("insert approval request");
    }

    /// Helper to insert an ApprovalGrant directly into the DB.
    async fn insert_grant(db: &mongodb::Database, grant: &ApprovalGrant) {
        db.collection::<ApprovalGrant>(GRANTS)
            .insert_one(grant)
            .await
            .expect("insert approval grant");
    }

    /// Helper to insert a NotificationChannel directly into the DB.
    async fn insert_channel(db: &mongodb::Database, channel: &NotificationChannel) {
        db.collection::<NotificationChannel>(CHANNELS)
            .insert_one(channel)
            .await
            .expect("insert notification channel");
    }

    /// Helper to insert a ServiceApprovalConfig directly into the DB.
    async fn insert_config(db: &mongodb::Database, config: &ServiceApprovalConfig) {
        db.collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
            .insert_one(config)
            .await
            .expect("insert service approval config");
    }

    fn make_channel(user_id: &str, approval_required: bool) -> NotificationChannel {
        let now = Utc::now();
        NotificationChannel {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            telegram_chat_id: Some(1234),
            telegram_username: Some("test".to_string()),
            telegram_enabled: true,
            telegram_link_code: None,
            telegram_link_code_expires_at: None,
            approval_timeout_secs: 30,
            grant_expiry_days: 30,
            approval_required: Some(approval_required),
            group_mentions_enabled: true,
            push_enabled: false,
            push_devices: vec![],
            created_at: now,
            updated_at: now,
        }
    }

    fn make_service_config(
        user_id: &str,
        service_id: &str,
        approval_required: bool,
        mode: ApprovalMode,
    ) -> ServiceApprovalConfig {
        let now = Utc::now();
        ServiceApprovalConfig {
            disabled: false,
            grant_expiry_days: None,
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            service_id: service_id.to_string(),
            service_name: "Test Service".to_string(),
            approval_required,
            approval_mode: mode,
            rules: vec![],
            default_effect: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn make_grant(
        user_id: &str,
        service_id: &str,
        requester_type: &str,
        requester_id: &str,
        revoked: bool,
        expires_in_days: i64,
    ) -> ApprovalGrant {
        let now = Utc::now();
        ApprovalGrant {
            execution_review: None,
            execution_last_used_at: None,
            execution_use: None,
            execution: None,
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            service_id: service_id.to_string(),
            service_name: "Test Service".to_string(),
            requester_type: requester_type.to_string(),
            requester_id: requester_id.to_string(),
            requester_label: None,
            approval_request_id: uuid::Uuid::new_v4().to_string(),
            scope: None,
            granted_at: now,
            expires_at: now + chrono::Duration::days(expires_in_days),
            revoked,
            org_scoped: false,
        }
    }

    fn make_pending_request_with_user(
        user_id: &str,
        service_id: &str,
        mode: ApprovalMode,
    ) -> ApprovalRequest {
        let now = Utc::now();
        ApprovalRequest {
            fresh_decision: false,
            caller_api_key_id: None,
            approval_protocol: None,
            grant_expiry_days_snapshot: None,
            activity_outcome: None,
            activity_execution: None,
            execution_proposal: None,
            assistant_group: None,
            id: uuid::Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            service_id: service_id.to_string(),
            service_name: "Test Service".to_string(),
            service_slug: "test-svc".to_string(),
            requester_type: "service_account".to_string(),
            requester_id: "requester-1".to_string(),
            requester_label: None,
            operation_summary: "proxy:POST /test".to_string(),
            action_description: None,
            http_method: Some("POST".to_string()),
            resource: Some("/test".to_string()),
            verb: Some("write".to_string()),
            grant_scope: None,
            tool_name: None,
            tool_call_id: None,
            tool_arguments: None,
            is_destructive: None,
            approval_mode: mode,
            status: "pending".to_string(),
            idempotency_key: uuid::Uuid::new_v4().to_string(),
            notification_channel: None,
            telegram_message_id: None,
            telegram_chat_id: None,
            expires_at: now + chrono::Duration::seconds(300),
            decided_at: None,
            decision_channel: None,
            decision_idempotency_key: None,
            notify_user_ids: vec![user_id.to_string()],
            from_org_policy: false,
            exact_service: None,
            created_at: now,
        }
    }

    // --- resolve_approval_mode ---

    #[tokio::test]
    async fn resolve_approval_mode_returns_default_when_no_config() {
        let Some(db) = connect_test_database("appr_svc_resolve_mode_default").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let mut session = db.client().start_session().await.unwrap();
        let mode = resolve_approval_mode(&db, &user_id, &service_id, &mut session)
            .await
            .unwrap();
        assert_eq!(mode, ApprovalMode::default());
    }

    #[tokio::test]
    async fn resolve_approval_mode_returns_per_service_config() {
        let Some(db) = connect_test_database("appr_svc_resolve_mode_cfg").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let config = make_service_config(&user_id, &service_id, true, ApprovalMode::Grant);
        insert_config(&db, &config).await;

        let mut session = db.client().start_session().await.unwrap();
        let mode = resolve_approval_mode(&db, &user_id, &service_id, &mut session)
            .await
            .unwrap();
        assert_eq!(mode, ApprovalMode::Grant);
    }

    // --- user_requires_approval ---

    #[tokio::test]
    async fn user_requires_approval_true_before_first_settings_read() {
        let Some(db) = connect_test_database("appr_svc_user_req_no_ch").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();

        let result = user_requires_approval(
            &db,
            &user_id,
            &crate::models::service_approval_config::ApprovalVerb::Write,
        )
        .await
        .unwrap();
        assert!(result);
        assert_eq!(
            notification_service::get_or_create_channel(&db, &user_id)
                .await
                .unwrap()
                .approval_required,
            None
        );
        assert!(
            !user_requires_approval(
                &db,
                &user_id,
                &crate::models::service_approval_config::ApprovalVerb::Read
            )
            .await
            .unwrap()
        );
    }

    #[tokio::test]
    async fn user_requires_approval_reads_channel_flag() {
        let Some(db) = connect_test_database("appr_svc_user_req_ch").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let channel = make_channel(&user_id, true);
        insert_channel(&db, &channel).await;

        let result = user_requires_approval(
            &db,
            &user_id,
            &crate::models::service_approval_config::ApprovalVerb::Write,
        )
        .await
        .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn required_approval_survives_channel_disconnect_and_reconnect() {
        let Some(db) = connect_test_database("appr_svc_suspended_global").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let mut channel = make_channel(&user_id, true);
        channel.telegram_chat_id = None;
        channel.telegram_username = None;
        channel.telegram_enabled = false;
        insert_channel(&db, &channel).await;

        let operation = test_operation();
        let suspended = evaluate_and_check(
            &db,
            &user_id,
            &user_id,
            &service_id,
            &operation,
            Some("api_key"),
            "agent-1",
            false,
            false,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(suspended, ApprovalOutcome::NeedsApproval(_)));
        assert_eq!(
            db.collection::<ApprovalRequest>(REQUESTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );

        db.collection::<NotificationChannel>(CHANNELS)
            .update_one(
                doc! { "_id": &channel.id },
                doc! {
                    "$set": {
                        "telegram_enabled": true,
                        "telegram_chat_id": 1234_i64,
                    }
                },
            )
            .await
            .unwrap();

        let resumed = evaluate_and_check(
            &db,
            &user_id,
            &user_id,
            &service_id,
            &operation,
            Some("api_key"),
            "agent-1",
            false,
            false,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(resumed, ApprovalOutcome::NeedsApproval(_)));
    }

    // --- check_approval ---

    #[tokio::test]
    async fn check_approval_returns_false_when_no_grants() {
        let Some(db) = connect_test_database("appr_svc_chk_no_grants").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();

        let result = check_approval(
            &db,
            &user_id,
            "svc-1",
            "sa",
            "req-1",
            false,
            &test_operation(),
        )
        .await
        .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn check_approval_returns_true_for_valid_grant() {
        let Some(db) = connect_test_database("appr_svc_chk_valid_grant").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let grant = make_grant(&user_id, &service_id, "sa", "req-1", false, 30);
        insert_grant(&db, &grant).await;

        let result = check_approval(
            &db,
            &user_id,
            &service_id,
            "sa",
            "req-1",
            false,
            &test_operation(),
        )
        .await
        .unwrap();
        assert!(result);
    }

    #[tokio::test]
    async fn check_approval_returns_false_for_revoked_grant() {
        let Some(db) = connect_test_database("appr_svc_chk_revoked").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let grant = make_grant(&user_id, &service_id, "sa", "req-1", true, 30);
        insert_grant(&db, &grant).await;

        let result = check_approval(
            &db,
            &user_id,
            &service_id,
            "sa",
            "req-1",
            false,
            &test_operation(),
        )
        .await
        .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn check_approval_returns_false_for_expired_grant() {
        let Some(db) = connect_test_database("appr_svc_chk_expired").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        // Already expired (negative days)
        let grant = make_grant(&user_id, &service_id, "sa", "req-1", false, -1);
        insert_grant(&db, &grant).await;

        let result = check_approval(
            &db,
            &user_id,
            &service_id,
            "sa",
            "req-1",
            false,
            &test_operation(),
        )
        .await
        .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn check_approval_returns_false_for_wrong_requester() {
        let Some(db) = connect_test_database("appr_svc_chk_wrong_req").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let grant = make_grant(&user_id, &service_id, "sa", "req-1", false, 30);
        insert_grant(&db, &grant).await;

        // Different requester_id
        let result = check_approval(
            &db,
            &user_id,
            &service_id,
            "sa",
            "req-DIFFERENT",
            false,
            &test_operation(),
        )
        .await
        .unwrap();
        assert!(!result);
    }

    #[tokio::test]
    async fn check_approval_org_scoped_grant_matches_any_requester() {
        let Some(db) = connect_test_database("appr_svc_chk_org_scoped").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let mut grant = make_grant(&user_id, &service_id, "sa", "req-1", false, 30);
        grant.org_scoped = true;
        insert_grant(&db, &grant).await;

        // Different requester, but org_scoped=true check matches
        let result = check_approval(
            &db,
            &user_id,
            &service_id,
            "sa",
            "req-DIFFERENT",
            true,
            &test_operation(),
        )
        .await
        .unwrap();
        assert!(result);
    }

    // --- get_request ---

    #[tokio::test]
    async fn get_request_returns_existing_request() {
        let Some(db) = connect_test_database("appr_svc_get_req_exists").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let req = make_pending_request_with_user(&user_id, "svc-1", ApprovalMode::PerRequest);
        insert_request(&db, &req).await;

        let result = get_request(&db, &req.id).await.unwrap();
        assert_eq!(result.id, req.id);
        assert_eq!(result.user_id, user_id);
        assert_eq!(result.status, "pending");
    }

    #[tokio::test]
    async fn get_request_returns_not_found_for_missing_id() {
        let Some(db) = connect_test_database("appr_svc_get_req_missing").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };

        let result = get_request(&db, "nonexistent-id").await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    // --- list_requests ---

    #[tokio::test]
    async fn list_requests_returns_user_requests_sorted_by_created_at() {
        let Some(db) = connect_test_database("appr_svc_list_req_sort").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let other_user_id = uuid::Uuid::new_v4().to_string();

        // Insert 3 requests for user, 1 for another user
        for i in 0..3 {
            let mut req =
                make_pending_request_with_user(&user_id, "svc-1", ApprovalMode::PerRequest);
            req.created_at = Utc::now() - chrono::Duration::minutes(i);
            insert_request(&db, &req).await;
        }
        let other_req =
            make_pending_request_with_user(&other_user_id, "svc-1", ApprovalMode::PerRequest);
        insert_request(&db, &other_req).await;

        let (requests, total) = list_requests(&db, &user_id, &[], &[], 1, 10).await.unwrap();
        assert_eq!(total, 3);
        assert_eq!(requests.len(), 3);
        // All should belong to user_id
        assert!(requests.iter().all(|r| r.user_id == user_id));
    }

    #[tokio::test]
    async fn list_requests_filters_by_status() {
        let Some(db) = connect_test_database("appr_svc_list_req_filter").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();

        let pending_req =
            make_pending_request_with_user(&user_id, "svc-1", ApprovalMode::PerRequest);
        insert_request(&db, &pending_req).await;

        let mut approved_req =
            make_pending_request_with_user(&user_id, "svc-2", ApprovalMode::PerRequest);
        approved_req.status = "approved".to_string();
        insert_request(&db, &approved_req).await;

        let (requests, total) = list_requests(&db, &user_id, &[], &["pending"], 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 1);
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].status, "pending");
    }

    #[tokio::test]
    async fn list_requests_pagination_works() {
        let Some(db) = connect_test_database("appr_svc_list_req_page").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();

        for _ in 0..5 {
            let req = make_pending_request_with_user(&user_id, "svc-1", ApprovalMode::PerRequest);
            insert_request(&db, &req).await;
        }

        let (page1, total) = list_requests(&db, &user_id, &[], &[], 1, 2).await.unwrap();
        assert_eq!(total, 5);
        assert_eq!(page1.len(), 2);

        let (page2, _) = list_requests(&db, &user_id, &[], &[], 2, 2).await.unwrap();
        assert_eq!(page2.len(), 2);

        let (page3, _) = list_requests(&db, &user_id, &[], &[], 3, 2).await.unwrap();
        assert_eq!(page3.len(), 1);
    }

    #[tokio::test]
    async fn list_requests_multi_status_filter() {
        let Some(db) = connect_test_database("appr_svc_list_req_multi_st").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();

        let pending_req =
            make_pending_request_with_user(&user_id, "svc-1", ApprovalMode::PerRequest);
        insert_request(&db, &pending_req).await;

        let mut approved_req =
            make_pending_request_with_user(&user_id, "svc-2", ApprovalMode::PerRequest);
        approved_req.status = "approved".to_string();
        insert_request(&db, &approved_req).await;

        let mut rejected_req =
            make_pending_request_with_user(&user_id, "svc-3", ApprovalMode::PerRequest);
        rejected_req.status = "rejected".to_string();
        insert_request(&db, &rejected_req).await;

        // Filter for approved and rejected
        let (requests, total) = list_requests(&db, &user_id, &[], &["approved", "rejected"], 1, 10)
            .await
            .unwrap();
        assert_eq!(total, 2);
        assert_eq!(requests.len(), 2);
        assert!(
            requests
                .iter()
                .all(|r| r.status == "approved" || r.status == "rejected")
        );
    }

    // --- revoke_grant ---

    #[tokio::test]
    async fn revoke_grant_marks_grant_as_revoked() {
        let Some(db) = connect_test_database("appr_svc_revoke_grant").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let grant = make_grant(&user_id, &service_id, "sa", "req-1", false, 30);
        insert_grant(&db, &grant).await;

        revoke_grant(&db, &user_id, &grant.id).await.unwrap();

        // Verify the grant is revoked in DB
        let revoked = db
            .collection::<ApprovalGrant>(GRANTS)
            .find_one(doc! { "_id": &grant.id })
            .await
            .unwrap()
            .unwrap();
        assert!(revoked.revoked);
    }

    #[tokio::test]
    async fn revoke_grant_returns_not_found_for_wrong_user() {
        let Some(db) = connect_test_database("appr_svc_revoke_wrong_user").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let grant = make_grant(&user_id, &service_id, "sa", "req-1", false, 30);
        insert_grant(&db, &grant).await;

        let result = revoke_grant(&db, "other-user", &grant.id).await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn revoke_grant_returns_not_found_for_nonexistent_grant() {
        let Some(db) = connect_test_database("appr_svc_revoke_missing").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };

        let result = revoke_grant(&db, "user-1", "nonexistent-grant").await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    // --- revoke_all_grants ---

    #[tokio::test]
    async fn revoke_all_grants_revokes_all_for_user() {
        let Some(db) = connect_test_database("appr_svc_revoke_all").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let g1 = make_grant(&user_id, &service_id, "sa", "req-1", false, 30);
        let g2 = make_grant(&user_id, &service_id, "sa", "req-2", false, 30);
        insert_grant(&db, &g1).await;
        insert_grant(&db, &g2).await;

        let count = revoke_all_grants(&db, &user_id).await.unwrap();
        assert_eq!(count, 2);

        // Verify both grants are revoked
        let result = check_approval(
            &db,
            &user_id,
            &service_id,
            "sa",
            "req-1",
            false,
            &test_operation(),
        )
        .await
        .unwrap();
        assert!(!result);
    }

    // --- list_service_approval_configs ---

    #[tokio::test]
    async fn list_service_approval_configs_returns_all_for_user() {
        let Some(db) = connect_test_database("appr_svc_list_cfg").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let other_user_id = uuid::Uuid::new_v4().to_string();

        let c1 = make_service_config(&user_id, "svc-1", true, ApprovalMode::Grant);
        let c2 = make_service_config(&user_id, "svc-2", false, ApprovalMode::PerRequest);
        let c3 = make_service_config(&other_user_id, "svc-1", true, ApprovalMode::Grant);
        insert_config(&db, &c1).await;
        insert_config(&db, &c2).await;
        insert_config(&db, &c3).await;

        let configs = list_service_approval_configs(&db, &user_id).await.unwrap();
        assert_eq!(configs.len(), 2);
        assert!(configs.iter().all(|c| c.user_id == user_id));
    }

    // --- set_service_approval_config ---

    #[tokio::test]
    async fn set_service_approval_config_creates_new_config() {
        let Some(db) = connect_test_database("appr_svc_set_cfg_new").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let config = set_service_approval_config(
            &db,
            &user_id,
            &service_id,
            "Test Service",
            Some(true),
            Some(&ApprovalMode::Grant),
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(config.user_id, user_id);
        assert_eq!(config.service_id, service_id);
        assert!(config.approval_required);
        assert_eq!(config.approval_mode, ApprovalMode::Grant);
    }

    #[tokio::test]
    async fn set_service_approval_config_updates_existing() {
        let Some(db) = connect_test_database("appr_svc_set_cfg_upd").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        // Create first
        set_service_approval_config(
            &db,
            &user_id,
            &service_id,
            "Test Service",
            Some(true),
            Some(&ApprovalMode::Grant),
            None,
            None,
        )
        .await
        .unwrap();

        // Update
        let updated = set_service_approval_config(
            &db,
            &user_id,
            &service_id,
            "Test Service",
            Some(false),
            Some(&ApprovalMode::PerRequest),
            None,
            None,
        )
        .await
        .unwrap();

        assert!(!updated.approval_required);
        assert_eq!(updated.approval_mode, ApprovalMode::PerRequest);
    }

    #[tokio::test]
    async fn set_service_approval_config_to_per_request_revokes_grants() {
        let Some(db) = connect_test_database("appr_svc_set_cfg_revoke").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        // Create a grant
        let grant = make_grant(&user_id, &service_id, "sa", "req-1", false, 30);
        insert_grant(&db, &grant).await;

        // Switch to per_request -- should revoke grants
        set_service_approval_config(
            &db,
            &user_id,
            &service_id,
            "Test Service",
            Some(true),
            Some(&ApprovalMode::PerRequest),
            None,
            None,
        )
        .await
        .unwrap();

        let still_valid = check_approval(
            &db,
            &user_id,
            &service_id,
            "sa",
            "req-1",
            false,
            &test_operation(),
        )
        .await
        .unwrap();
        assert!(!still_valid);
    }

    #[tokio::test]
    async fn set_service_approval_config_reconciles_rule_scoped_and_service_wide_grants() {
        let Some(db) = connect_test_database("appr_svc_set_cfg_rule_grants").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let scope = "v1:http:post:write:/v1/chat/**";

        let mut rule_scoped = make_grant(
            &user_id,
            &service_id,
            "api_key",
            "rule-requester",
            false,
            30,
        );
        rule_scoped.scope = Some(scope.to_string());
        let service_wide = make_grant(
            &user_id,
            &service_id,
            "api_key",
            "legacy-requester",
            false,
            30,
        );
        insert_grant(&db, &rule_scoped).await;
        insert_grant(&db, &service_wide).await;

        let grant_rule = ApprovalRule {
            mandatory_review: false,
            methods: vec!["POST".to_string()],
            resource_pattern: "/v1/chat/**".to_string(),
            verbs: vec![],
            effect: ApprovalEffect::RequireApproval,
            mode: ApprovalMode::Grant,
            api_key_ids: vec![],
        };
        set_service_approval_config(
            &db,
            &user_id,
            &service_id,
            "Test Service",
            Some(true),
            Some(&ApprovalMode::PerRequest),
            Some(std::slice::from_ref(&grant_rule)),
            None,
        )
        .await
        .unwrap();

        let grants = db.collection::<ApprovalGrant>(GRANTS);
        let retained = grants
            .find_one(doc! { "_id": &rule_scoped.id })
            .await
            .unwrap()
            .unwrap();
        let revoked_legacy = grants
            .find_one(doc! { "_id": &service_wide.id })
            .await
            .unwrap()
            .unwrap();
        assert!(!retained.revoked, "rule-scoped grant should remain active");
        assert!(
            revoked_legacy.revoked,
            "service-wide grant requires top-level grant mode"
        );

        set_service_approval_config(
            &db,
            &user_id,
            &service_id,
            "Test Service",
            Some(true),
            Some(&ApprovalMode::PerRequest),
            Some(&[]),
            None,
        )
        .await
        .unwrap();

        let revoked_rule = grants
            .find_one(doc! { "_id": &rule_scoped.id })
            .await
            .unwrap()
            .unwrap();
        assert!(
            revoked_rule.revoked,
            "scoped grant should be revoked after grant capability is removed"
        );
    }

    #[tokio::test]
    async fn stale_scoped_grant_cannot_bypass_current_per_request_rule() {
        let Some(db) = connect_test_database("appr_svc_stale_scope_policy_gate").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let requester_id = "rule-requester";

        let mut stale_grant = make_grant(&user_id, &service_id, "api_key", requester_id, false, 30);
        stale_grant.scope = Some("v1:http:*:*:*".to_string());
        insert_grant(&db, &stale_grant).await;

        let rules = vec![
            ApprovalRule {
                mandatory_review: false,
                methods: vec!["DELETE".to_string()],
                resource_pattern: "/v1/admin/**".to_string(),
                verbs: vec![],
                effect: ApprovalEffect::RequireApproval,
                mode: ApprovalMode::PerRequest,
                api_key_ids: vec![],
            },
            ApprovalRule {
                mandatory_review: false,
                methods: vec!["POST".to_string()],
                resource_pattern: "/v1/chat/**".to_string(),
                verbs: vec![],
                effect: ApprovalEffect::RequireApproval,
                mode: ApprovalMode::Grant,
                api_key_ids: vec![],
            },
        ];
        set_service_approval_config(
            &db,
            &user_id,
            &service_id,
            "Test Service",
            Some(true),
            Some(&ApprovalMode::PerRequest),
            Some(&rules),
            None,
        )
        .await
        .unwrap();

        let retained = db
            .collection::<ApprovalGrant>(GRANTS)
            .find_one(doc! { "_id": &stale_grant.id })
            .await
            .unwrap()
            .unwrap();
        assert!(!retained.revoked, "conservative cleanup retains the scope");

        let operation = crate::services::operation_descriptor::build_http_descriptor(
            "DELETE",
            "/v1/admin/users/42",
            None,
        );
        let outcome = evaluate_and_check(
            &db,
            &user_id,
            &user_id,
            &service_id,
            &operation,
            Some("api_key"),
            requester_id,
            false,
            false,
            None,
        )
        .await
        .unwrap();

        let ApprovalOutcome::NeedsApproval(pending) = outcome else {
            panic!("current per-request rule must not reuse the stale grant");
        };
        assert_eq!(pending.resolution.mode, ApprovalMode::PerRequest);
    }

    // --- delete_service_approval_config ---

    #[tokio::test]
    async fn delete_service_approval_config_removes_config() {
        let Some(db) = connect_test_database("appr_svc_del_cfg").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let config = make_service_config(&user_id, &service_id, true, ApprovalMode::Grant);
        insert_config(&db, &config).await;

        delete_service_approval_config(&db, &user_id, &service_id, test_config_audit(&user_id))
            .await
            .unwrap();

        // Verify it's gone
        let configs = list_service_approval_configs(&db, &user_id).await.unwrap();
        assert!(configs.is_empty());
    }

    #[tokio::test]
    async fn delete_service_approval_config_returns_not_found() {
        let Some(db) = connect_test_database("appr_svc_del_cfg_missing").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };

        let result = delete_service_approval_config(
            &db,
            "user-1",
            "nonexistent-svc",
            test_config_audit("user-1"),
        )
        .await;
        assert!(matches!(result, Err(AppError::NotFound(_))));
    }

    #[tokio::test]
    async fn delete_service_approval_config_revokes_grants() {
        let Some(db) = connect_test_database("appr_svc_del_cfg_revoke").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let config = make_service_config(&user_id, &service_id, true, ApprovalMode::Grant);
        insert_config(&db, &config).await;

        let grant = make_grant(&user_id, &service_id, "sa", "req-1", false, 30);
        insert_grant(&db, &grant).await;

        delete_service_approval_config(&db, &user_id, &service_id, test_config_audit(&user_id))
            .await
            .unwrap();

        let still_valid = check_approval(
            &db,
            &user_id,
            &service_id,
            "sa",
            "req-1",
            false,
            &test_operation(),
        )
        .await
        .unwrap();
        assert!(!still_valid);
    }

    // --- resolve_org_aware_approval ---

    #[tokio::test]
    async fn resolve_org_aware_approval_falls_back_to_actor_when_no_org_config() {
        let Some(db) = connect_test_database("appr_svc_org_aware_fallback").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        // Create an org user (so the code knows the owner is an org)
        let org_user = test_user(&org_user_id, UserType::Org);
        db.collection::<User>(USERS)
            .insert_one(&org_user)
            .await
            .unwrap();

        // No org config, actor has a per-service config
        let actor_config = make_service_config(&actor_id, &service_id, true, ApprovalMode::Grant);
        insert_config(&db, &actor_config).await;

        let operation = test_operation();
        let resolution = resolve_org_aware_approval(
            &db,
            &actor_id,
            &org_user_id,
            &service_id,
            &operation,
            false,
            None,
        )
        .await
        .unwrap();
        assert!(resolution.required);
        assert_eq!(resolution.mode, ApprovalMode::Grant);
        assert_eq!(resolution.primary_owner_user_id, actor_id);
        assert!(!resolution.from_org_policy);
    }

    #[tokio::test]
    async fn resolve_org_aware_approval_uses_org_config_when_present() {
        let Some(db) = connect_test_database("appr_svc_org_aware_org_cfg").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        // Create an org user
        let org_user = test_user(&org_user_id, UserType::Org);
        db.collection::<User>(USERS)
            .insert_one(&org_user)
            .await
            .unwrap();

        // Org has a per-service config
        let org_config = make_service_config(&org_user_id, &service_id, true, ApprovalMode::Grant);
        insert_config(&db, &org_config).await;

        let operation = test_operation();
        let resolution = resolve_org_aware_approval(
            &db,
            &actor_id,
            &org_user_id,
            &service_id,
            &operation,
            false,
            None,
        )
        .await
        .unwrap();
        assert!(resolution.required);
        assert_eq!(resolution.mode, ApprovalMode::Grant);
        assert_eq!(resolution.primary_owner_user_id, org_user_id);
        assert!(resolution.from_org_policy);
    }

    #[tokio::test]
    async fn resolve_org_aware_approval_actor_policy_when_owner_is_person() {
        let Some(db) = connect_test_database("appr_svc_org_aware_person").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        // Create a person user (not an org)
        let person_user = test_user(&actor_id, UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&person_user)
            .await
            .unwrap();

        // Actor has global approval enabled via notification channel
        let channel = make_channel(&actor_id, true);
        insert_channel(&db, &channel).await;

        let operation = test_operation();
        let resolution = resolve_org_aware_approval(
            &db,
            &actor_id,
            &actor_id,
            &service_id,
            &operation,
            false,
            None,
        )
        .await
        .unwrap();
        assert!(resolution.required);
        assert_eq!(resolution.mode, ApprovalMode::default());
        assert_eq!(resolution.primary_owner_user_id, actor_id);
        assert!(!resolution.from_org_policy);
    }

    #[tokio::test]
    async fn resolve_org_aware_approval_skips_global_default_for_auto_connected_service() {
        let Some(db) = connect_test_database("appr_svc_auto_global_default").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        insert_channel(&db, &make_channel(&actor_id, true)).await;

        let resolution = resolve_org_aware_approval(
            &db,
            &actor_id,
            &actor_id,
            &service_id,
            &test_operation(),
            true,
            None,
        )
        .await
        .unwrap();

        assert!(!resolution.required);
        assert_eq!(resolution.effect, ApprovalEffect::AutoAllow);

        let outcome = evaluate_and_check(
            &db,
            &actor_id,
            &actor_id,
            &service_id,
            &test_operation(),
            Some("api_key"),
            "agent-1",
            false,
            true,
            None,
        )
        .await
        .unwrap();
        assert!(matches!(
            outcome,
            ApprovalOutcome::Allowed { required: false }
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
    async fn resolve_org_aware_approval_keeps_explicit_actor_config_for_auto_connected_service() {
        let Some(db) = connect_test_database("appr_svc_auto_explicit_config").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let mut suspended_channel = make_channel(&actor_id, true);
        suspended_channel.telegram_chat_id = None;
        suspended_channel.telegram_username = None;
        suspended_channel.telegram_enabled = false;
        insert_channel(&db, &suspended_channel).await;

        let cases = [
            (false, None, ApprovalEffect::AutoAllow, false),
            (true, None, ApprovalEffect::RequireApproval, true),
            (
                false,
                Some(ApprovalEffect::Deny),
                ApprovalEffect::Deny,
                false,
            ),
        ];
        for (approval_required, default_effect, expected_effect, expected_required) in cases {
            let service_id = uuid::Uuid::new_v4().to_string();
            let mut config = make_service_config(
                &actor_id,
                &service_id,
                approval_required,
                ApprovalMode::PerRequest,
            );
            config.default_effect = default_effect;
            insert_config(&db, &config).await;

            let resolution = resolve_org_aware_approval(
                &db,
                &actor_id,
                &actor_id,
                &service_id,
                &test_operation(),
                true,
                None,
            )
            .await
            .unwrap();

            assert_eq!(resolution.required, expected_required);
            assert_eq!(resolution.effect, expected_effect);
        }
    }

    #[tokio::test]
    async fn resolve_org_aware_approval_keeps_explicit_org_config_for_auto_connected_service() {
        let Some(db) = connect_test_database("appr_svc_auto_explicit_org_config").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&org_user_id, UserType::Org))
            .await
            .unwrap();

        let mut org_config =
            make_service_config(&org_user_id, &service_id, false, ApprovalMode::PerRequest);
        org_config.default_effect = Some(ApprovalEffect::Deny);
        insert_config(&db, &org_config).await;

        let resolution = resolve_org_aware_approval(
            &db,
            &actor_id,
            &org_user_id,
            &service_id,
            &test_operation(),
            true,
            None,
        )
        .await
        .unwrap();

        assert!(!resolution.required);
        assert_eq!(resolution.effect, ApprovalEffect::Deny);
        assert!(resolution.from_org_policy);
    }

    // --- list_grants ---

    #[tokio::test]
    async fn list_grants_returns_active_grants_for_grant_mode_services() {
        let Some(db) = connect_test_database("appr_svc_list_grants").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        // Set up grant mode config
        let config = make_service_config(&user_id, &service_id, true, ApprovalMode::Grant);
        insert_config(&db, &config).await;

        // Insert a valid grant
        let grant = make_grant(&user_id, &service_id, "sa", "req-1", false, 30);
        insert_grant(&db, &grant).await;

        let (grants, total) = list_grants(&db, &user_id, &[], 1, 10).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(grants.len(), 1);
        assert_eq!(grants[0].id, grant.id);
    }

    #[tokio::test]
    async fn list_grants_includes_legacy_config_without_stored_approval_mode() {
        let Some(db) = connect_test_database("appr_svc_list_grants_legacy_mode").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let config = make_service_config(&user_id, &service_id, true, ApprovalMode::Grant);
        insert_config(&db, &config).await;
        db.collection::<bson::Document>(SERVICE_APPROVAL_CONFIGS)
            .update_one(
                doc! { "_id": &config.id },
                doc! { "$unset": { "approval_mode": "" } },
            )
            .await
            .unwrap();

        let grant = make_grant(&user_id, &service_id, "sa", "req-1", false, 30);
        insert_grant(&db, &grant).await;

        let (grants, total) = list_grants(&db, &user_id, &[], 1, 10).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(grants[0].id, grant.id);
    }

    #[tokio::test]
    async fn list_grants_includes_rule_level_grant_mode_service() {
        let Some(db) = connect_test_database("appr_svc_list_grants_rule_mode").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let mut config = make_service_config(&user_id, &service_id, true, ApprovalMode::PerRequest);
        config.rules.push(ApprovalRule {
            mandatory_review: false,
            methods: vec!["POST".to_string()],
            resource_pattern: "/v1/chat/**".to_string(),
            verbs: vec![],
            effect: ApprovalEffect::RequireApproval,
            mode: ApprovalMode::Grant,
            api_key_ids: vec![],
        });
        insert_config(&db, &config).await;

        let mut grant = make_grant(&user_id, &service_id, "sa", "req-1", false, 30);
        grant.scope = Some("v1:http:post:write:/v1/chat/**".to_string());
        insert_grant(&db, &grant).await;

        let (grants, total) = list_grants(&db, &user_id, &[], 1, 10).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(grants[0].id, grant.id);
    }

    #[tokio::test]
    async fn list_grants_excludes_per_request_mode_services() {
        let Some(db) = connect_test_database("appr_svc_list_grants_pr").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        // Service is in per_request mode (not grant)
        let config = make_service_config(&user_id, &service_id, true, ApprovalMode::PerRequest);
        insert_config(&db, &config).await;

        // Insert a grant (lingering from before mode switch)
        let grant = make_grant(&user_id, &service_id, "sa", "req-1", false, 30);
        insert_grant(&db, &grant).await;

        let (grants, total) = list_grants(&db, &user_id, &[], 1, 10).await.unwrap();
        assert_eq!(total, 0);
        assert!(grants.is_empty());
    }

    #[tokio::test]
    async fn list_grants_excludes_revoked_and_expired() {
        let Some(db) = connect_test_database("appr_svc_list_grants_exc").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();

        let config = make_service_config(&user_id, &service_id, true, ApprovalMode::Grant);
        insert_config(&db, &config).await;

        // Revoked grant
        let revoked = make_grant(&user_id, &service_id, "sa", "req-1", true, 30);
        insert_grant(&db, &revoked).await;

        // Expired grant
        let expired = make_grant(&user_id, &service_id, "sa", "req-2", false, -1);
        insert_grant(&db, &expired).await;

        // Active grant
        let active = make_grant(&user_id, &service_id, "sa", "req-3", false, 30);
        insert_grant(&db, &active).await;

        let (grants, total) = list_grants(&db, &user_id, &[], 1, 10).await.unwrap();
        assert_eq!(total, 1);
        assert_eq!(grants[0].id, active.id);
    }

    // --- expire_pending_requests ---

    #[tokio::test]
    async fn expire_pending_requests_expires_stale_requests() {
        let Some(db) = connect_test_database("appr_svc_expire_pending").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let config = crate::test_utils::test_app_config();
        let http_client = reqwest::Client::new();

        // Insert a request that expired 10 seconds ago
        let mut req = make_pending_request_with_user(&user_id, "svc-1", ApprovalMode::PerRequest);
        req.expires_at = Utc::now() - chrono::Duration::seconds(10);
        insert_request(&db, &req).await;

        // Insert a request that has not expired yet
        let req2 = make_pending_request_with_user(&user_id, "svc-2", ApprovalMode::PerRequest);
        insert_request(&db, &req2).await;

        let count = expire_pending_requests(&db, &config, &http_client, None, None, None)
            .await
            .unwrap();
        assert_eq!(count, 1);

        // Verify the first request is expired
        let expired = get_request(&db, &req.id).await.unwrap();
        assert_eq!(expired.status, "expired");

        // Verify the second is still pending
        let still_pending = get_request(&db, &req2.id).await.unwrap();
        assert_eq!(still_pending.status, "pending");
    }
    #[tokio::test]
    async fn caller_key_reaches_all_evaluation_layers_without_changing_bypass_or_fallback() {
        let db = crate::test_utils::connect_test_database("approval_caller_key")
            .await
            .unwrap();
        let owner = Uuid::new_v4().to_string();
        let service = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        crate::test_utils::set_test_approval_preference(&db, &owner, false).await;
        db.collection::<Document>(crate::models::api_key::COLLECTION_NAME)
            .insert_one(doc! {"_id": "key-a", "user_id": &owner, "is_active": true})
            .await
            .unwrap();
        let operation = test_operation();
        for effect in [ApprovalEffect::Deny, ApprovalEffect::RequireApproval] {
            let mut rule: ApprovalRule = serde_json::from_value(serde_json::json!({"resource_pattern":"*","effect":effect,"mode":"per_request","api_key_ids":["key-a"]})).unwrap();
            for limited in [true, false] {
                if !limited {
                    rule.api_key_ids.clear();
                }
                set_service_approval_config(
                    &db,
                    &owner,
                    &service,
                    "Caller key",
                    Some(true),
                    Some(&ApprovalMode::PerRequest),
                    Some(&[rule.clone()]),
                    None,
                )
                .await
                .unwrap();
                for key in [Some("key-a"), Some("key-b"), None] {
                    let applies = !limited || key == Some("key-a");
                    let resolution = resolve_org_aware_approval(
                        &db, &owner, &owner, &service, &operation, false, key,
                    )
                    .await
                    .unwrap();
                    assert_eq!(
                        resolution.effect,
                        if applies {
                            effect.clone()
                        } else {
                            ApprovalEffect::AutoAllow
                        }
                    );
                    let deny_only =
                        evaluate_deny_only(&db, &owner, &owner, &service, &operation, false, key)
                            .await;
                    if limited && applies && effect == ApprovalEffect::RequireApproval {
                        assert!(
                            matches!(deny_only, Err(AppError::ApprovalUnsupported)),
                            "a key-specific Ask cannot be silently satisfied on a noninteractive path"
                        );
                    } else {
                        assert_eq!(
                            deny_only.unwrap(),
                            applies && effect == ApprovalEffect::Deny
                        );
                    }
                    for bypass in [false, true] {
                        let (outcome, _) = evaluate_for_chat(
                            &db,
                            &owner,
                            &owner,
                            &service,
                            &operation,
                            Some("api_key"),
                            "requester-is-independent",
                            bypass,
                            false,
                            None,
                            key,
                        )
                        .await
                        .unwrap();
                        assert_eq!(
                            matches!(outcome, ApprovalOutcome::Denied),
                            applies && effect == ApprovalEffect::Deny
                        );
                        assert_eq!(
                            matches!(outcome, ApprovalOutcome::NeedsApproval(_)),
                            applies && effect == ApprovalEffect::RequireApproval && !bypass
                        );
                    }
                }
            }
        }
        // A key-only policy must not suppress the existing implicit global floor.
        let rule = serde_json::from_value(
            serde_json::json!({"resource_pattern":"*","effect":"deny","api_key_ids":["key-a"]}),
        )
        .unwrap();
        set_service_approval_config(
            &db,
            &owner,
            &service,
            "Caller key",
            None,
            None,
            Some(&[rule]),
            None,
        )
        .await
        .unwrap();
        crate::test_utils::set_test_approval_preference(&db, &owner, true).await;
        for key in [Some("key-b"), None] {
            assert!(
                resolve_org_aware_approval(&db, &owner, &owner, &service, &operation, false, key)
                    .await
                    .unwrap()
                    .required
            );
            assert!(
                !resolve_org_aware_approval(&db, &owner, &owner, &service, &operation, true, key)
                    .await
                    .unwrap()
                    .required
            );
        }
        crate::test_utils::drop_test_database(&db).await;
    }
}

pub(crate) async fn get_config(
    db: &Database,
    owner: &str,
    service: &str,
) -> AppResult<Option<ServiceApprovalConfig>> {
    Ok(db
        .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
        .find_one(doc! {"user_id":owner,"service_id":service})
        .await?)
}

/// The original policy-save reconciliation for callers without a reviewed
/// revision: a non-grant policy revokes every active service grant (scoped
/// grants survive when rules can still grant) and expires pending grant-mode
/// requests in the same scope.
async fn legacy_reconcile_grant_state_in_session(
    db: &Database,
    config: &ServiceApprovalConfig,
    session: &mut ClientSession,
) -> AppResult<()> {
    if config.disabled || config.approval_mode == ApprovalMode::Grant {
        return Ok(());
    }
    let unscoped_only = service_config_supports_grants(config);
    let mut grants = doc! {
        "user_id": &config.user_id, "service_id": &config.service_id, "revoked": false,
    };
    let mut requests = doc! {
        "user_id": &config.user_id, "service_id": &config.service_id,
        "status": "pending", "approval_mode": { "$in": ["grant", null] },
    };
    if unscoped_only {
        grants.insert("scope", bson::Bson::Null);
        requests.insert("grant_scope", bson::Bson::Null);
    }
    db.collection::<ApprovalGrant>(GRANTS)
        .update_many(grants, revoke_grant_update(Utc::now()))
        .session(&mut *session)
        .await?;
    db.collection::<ApprovalRequest>(REQUESTS)
        .update_many(
            requests,
            doc! { "$set": { "status": "expired", "decided_at": bson::DateTime::now() } },
        )
        .session(&mut *session)
        .await?;
    Ok(())
}

/// Policy, pending-request invalidation and grant reconciliation commit once.
/// Key-only changes retain unrelated grants and explicit execution permissions;
/// every use still evaluates the live operation policy before consulting a grant.
async fn reconcile_grant_state_in_session(
    db: &Database,
    config: &ServiceApprovalConfig,
    session: &mut ClientSession,
) -> AppResult<()> {
    if config.disabled {
        return Ok(());
    }
    let key_only = !config.rules.is_empty()
        && config.rules.iter().all(|r| !r.api_key_ids.is_empty())
        && config.default_effect.is_none();
    if !key_only && config.approval_mode != ApprovalMode::Grant {
        let mut filter = doc! {"user_id":&config.user_id,"service_id":&config.service_id,"revoked":false,"execution":null};
        if service_config_supports_grants(config) {
            filter.insert("scope", bson::Bson::Null);
        }
        db.collection::<ApprovalGrant>(GRANTS)
            .update_many(filter, doc! {"$set":{"revoked":true}})
            .session(&mut *session)
            .await?;
    }
    let mut cursor = db.collection::<ApprovalRequest>(REQUESTS).find(doc! {"user_id":&config.user_id,"service_id":&config.service_id,"status":"pending","execution_proposal":null}).session(&mut *session).await?;
    let pending: Vec<ApprovalRequest> = cursor.stream(&mut *session).try_collect().await?;
    for request in pending {
        let has_key_rules = config.rules.iter().any(|rule| !rule.api_key_ids.is_empty());
        let Some(protocol) = request.approval_protocol.as_deref() else {
            // Older requests do not record the authenticated key or protocol.
            // Retain them; the admitted caller's live-policy recheck governs use.
            if !has_key_rules
                && request.approval_mode == ApprovalMode::Grant
                && !service_config_supports_grants(config)
            {
                db.collection::<ApprovalRequest>(REQUESTS)
                    .update_one(
                        doc! {"_id":&request.id,"status":"pending"},
                        doc! {"$set":{"status":"expired","decided_at":bson::DateTime::now()}},
                    )
                    .session(&mut *session)
                    .await?;
            }
            continue;
        };
        let protocol = match protocol {
            "proxy" => super::operation_descriptor::Protocol::Http,
            "mcp" => super::operation_descriptor::Protocol::Mcp,
            "ssh" => super::operation_descriptor::Protocol::Ssh,
            "llm" => super::operation_descriptor::Protocol::Llm,
            _ => continue,
        };
        let verb = match request.verb.as_deref() {
            Some("read") => crate::models::service_approval_config::ApprovalVerb::Read,
            Some("write") => crate::models::service_approval_config::ApprovalVerb::Write,
            Some("destructive") => {
                crate::models::service_approval_config::ApprovalVerb::Destructive
            }
            _ => continue,
        };
        let descriptor = OperationDescriptor {
            protocol,
            verb,
            method: request.http_method.clone(),
            resource: request.resource.clone(),
            summary: request.operation_summary.clone(),
        };
        let key = request.caller_api_key_id.as_deref();
        if let Some(decision) = approval_policy::evaluate(config, &descriptor, key)
            && (decision.effect != ApprovalEffect::RequireApproval
                || decision.mode != request.approval_mode)
        {
            db.collection::<ApprovalRequest>(REQUESTS)
                .update_one(
                    doc! {"_id":&request.id,"status":"pending"},
                    doc! {"$set":{"status":"expired","decided_at":bson::DateTime::now()}},
                )
                .session(&mut *session)
                .await?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod policy_durability_tests {
    use super::*;
    use crate::models::service_approval_config::ApprovalVerb;
    use crate::{handlers::approvals, test_utils::*};
    use axum::{
        Json,
        extract::{Path, Query, State},
    };

    async fn fixture(name: &str) -> (crate::AppState, String, String) {
        let db = connect_transaction_test_database(name).await;
        super::super::security_readiness_service::ready_test_replica(&db).await;
        let owner = uuid::Uuid::new_v4().to_string();
        let service = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&owner, crate::models::user::UserType::Person))
            .await
            .unwrap();
        db.collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .insert_one(test_user_service(
            &service, &owner, "fixture", "endpoint", None, None,
        ))
        .await
        .unwrap();
        let now = bson::DateTime::now();
        db.collection::<Document>(SERVICE_APPROVAL_CONFIGS)
            .insert_one(doc! {
                "_id": uuid::Uuid::new_v4().to_string(),
                "user_id": &owner,
                "service_id": &service,
                "service_name": "fixture",
                "approval_required": true,
                "approval_mode": "grant",
                "default_effect": "deny",
                "rules": [
                    {
                        "methods": ["EXEC"],
                        "resource_pattern": "git *",
                        "verbs": ["write"],
                        "effect": "require_approval",
                        "mode": "grant"
                    },
                    {
                        "methods": ["GET"],
                        "resource_pattern": "/status",
                        "verbs": [],
                        "effect": "deny",
                        "mode": "per_request"
                    },
                    {
                        "methods": ["GET"],
                        "resource_pattern": "/**",
                        "verbs": [],
                        "effect": "auto_allow",
                        "mode": "per_request"
                    }
                ],
                "created_at": now,
                "updated_at": now
            })
            .await
            .unwrap();
        db.collection::<Document>(GRANTS)
            .insert_one(doc! {
                "_id": "legacy-grant",
                "user_id": &owner,
                "service_id": &service,
                "requester_type": "user",
                "requester_id": &owner,
                "scope": null,
                "revoked": false,
                "expires_at": bson::DateTime::from_chrono(Utc::now()+Duration::days(7))
            })
            .await
            .unwrap();
        (test_app_state(db), owner, service)
    }

    async fn put(
        state: &crate::AppState,
        owner: &str,
        service: &str,
        body: serde_json::Value,
    ) -> AppResult<axum::Json<approvals::ServiceApprovalConfigItem>> {
        approvals::set_service_config(
            State(state.clone()),
            test_auth_user(owner),
            Default::default(),
            Path(service.into()),
            Query(Default::default()),
            Json(serde_json::from_value(body).unwrap()),
        )
        .await
    }

    async fn rotation_key(db: &Database, owner: &str) -> String {
        super::super::key_service::create_api_key(
            db,
            owner,
            "rotation fixture",
            "proxy",
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn rotation_carries_ordered_personal_rules_without_moving_approval_identities() {
        use super::super::key_service::{self, ApiKeyRotationOutcome};
        let (state, owner, service) = fixture("policy_rotation_identity").await;
        let db = &state.db;
        let predecessor = rotation_key(db, &owner).await;
        let sibling = rotation_key(db, &owner).await;
        let successor = Uuid::new_v4().to_string();
        let configs = db.collection::<Document>(SERVICE_APPROVAL_CONFIGS);
        let original = get_config(db, &owner, &service).await.unwrap().unwrap();
        let rules = doc! {"rules": [
            {"methods": ["POST"], "resource_pattern": "/ask", "effect": "require_approval", "mode": "per_request", "api_key_ids": [&predecessor]},
            {"methods": ["POST"], "resource_pattern": "/deny", "effect": "deny", "api_key_ids": [&sibling, &predecessor]},
            {"methods": ["POST"], "resource_pattern": "/allow", "effect": "auto_allow", "api_key_ids": [&predecessor]},
            {"methods": ["GET"], "resource_pattern": "*", "effect": "auto_allow"}
        ], "default_effect": null, "grant_expiry_days": null, "disabled": false};
        configs
            .update_one(doc! {"_id": &original.id}, doc! {"$set": rules})
            .await
            .unwrap();
        let mut disabled = configs
            .find_one(doc! {"_id": &original.id})
            .await
            .unwrap()
            .unwrap();
        disabled.insert("_id", "disabled-policy");
        disabled.insert("service_id", "disabled-service");
        disabled.insert("disabled", true);
        disabled.insert("grant_expiry_days", 11);
        disabled.insert(
            "rules",
            vec![doc! {"effect": "deny", "api_key_ids": [&predecessor, &sibling, &successor]}],
        );
        configs.insert_one(&disabled).await.unwrap();
        let mut other = disabled.clone();
        other.insert("_id", "other-owner-policy");
        other.insert("user_id", Uuid::new_v4().to_string());
        configs.insert_one(&other).await.unwrap();
        let before = configs
            .find_one(doc! {"_id": &original.id})
            .await
            .unwrap()
            .unwrap();
        let reviewed = get_config(db, &owner, &service).await.unwrap().unwrap();
        let reviewed_revision = config_revision(&reviewed).unwrap();
        let now = bson::DateTime::now();
        let grant = doc! {"_id": "predecessor-grant", "user_id": &owner, "service_id": &service,
        "service_name": "fixture", "requester_type": "api_key", "requester_id": &predecessor,
        "requester_label": null, "approval_request_id": "predecessor-request", "scope": null,
        "granted_at": now, "expires_at": bson::DateTime::from_chrono(Utc::now()+Duration::days(1)), "revoked": false};
        db.collection::<Document>(GRANTS)
            .insert_one(&grant)
            .await
            .unwrap();
        let pending = doc! {"_id": "predecessor-request", "user_id": &owner, "service_id": &service,
        "requester_id": &predecessor, "caller_api_key_id": &predecessor, "status": "pending", "fresh_decision": true};
        db.collection::<Document>(REQUESTS)
            .insert_one(&pending)
            .await
            .unwrap();
        let intents =
            db.collection::<Document>(crate::models::security_effect_intent::COLLECTION_NAME);
        let rotation_audit = doc! {"event.event_type": "service_approval_config_key_rotated"};
        // Failure after the rule writes must roll back the entire rotation.
        db.create_collection(crate::models::security_effect_intent::COLLECTION_NAME)
            .await
            .unwrap();
        db.run_command(
            doc! {"collMod": crate::models::security_effect_intent::COLLECTION_NAME,
            "validator": {"fixture_required": {"$exists": true}}},
        )
        .await
        .unwrap();
        let rotate = || {
            key_service::rotate_api_key_with_scope_authorization_and_id(
                db,
                &state.encryption_keys,
                &owner,
                Some(&owner),
                &predecessor,
                &successor,
            )
        };
        assert!(rotate().await.is_err());
        assert!(
            key_service::get_api_key(db, &owner, &predecessor)
                .await
                .is_ok()
        );
        assert!(
            key_service::get_api_key(db, &owner, &successor)
                .await
                .is_err()
        );
        assert_eq!(
            configs
                .find_one(doc! {"_id": &original.id})
                .await
                .unwrap()
                .unwrap(),
            before
        );
        assert_eq!(
            configs
                .find_one(doc! {"_id": "disabled-policy"})
                .await
                .unwrap()
                .unwrap(),
            disabled
        );
        assert_eq!(
            intents
                .count_documents(rotation_audit.clone())
                .await
                .unwrap(),
            0
        );
        db.run_command(doc! {"collMod": crate::models::security_effect_intent::COLLECTION_NAME, "validator": {}}).await.unwrap();
        assert!(matches!(
            rotate().await.unwrap(),
            ApiKeyRotationOutcome::Created(_)
        ));
        assert!(
            key_service::get_api_key(db, &owner, &predecessor)
                .await
                .is_err()
        );
        let carried = get_config(db, &owner, &service).await.unwrap().unwrap();
        let mut expected = reviewed.clone();
        for rule in &mut expected.rules {
            for member in &mut rule.api_key_ids {
                if member == &predecessor {
                    *member = successor.clone();
                }
            }
        }
        expected.updated_at = carried.updated_at;
        assert_eq!(
            bson::to_document(&carried).unwrap(),
            bson::to_document(&expected).unwrap()
        );
        assert_ne!(config_revision(&carried).unwrap(), reviewed_revision);
        let disabled_after = configs
            .find_one(doc! {"_id": "disabled-policy"})
            .await
            .unwrap()
            .unwrap();
        disabled.insert(
            "rules",
            vec![doc! {"effect": "deny", "api_key_ids": [&successor, &sibling]}],
        );
        disabled.insert(
            "updated_at",
            disabled_after
                .get_datetime("updated_at")
                .unwrap()
                .to_owned(),
        );
        assert_eq!(disabled_after, disabled);
        assert_eq!(
            configs
                .find_one(doc! {"_id": "other-owner-policy"})
                .await
                .unwrap()
                .unwrap(),
            other
        );
        assert_eq!(
            db.collection::<Document>(GRANTS)
                .find_one(doc! {"_id": "predecessor-grant"})
                .await
                .unwrap()
                .unwrap(),
            grant
        );
        assert_eq!(
            db.collection::<Document>(REQUESTS)
                .find_one(doc! {"_id": "predecessor-request"})
                .await
                .unwrap()
                .unwrap(),
            pending
        );
        let ask = super::super::operation_descriptor::build_http_descriptor("POST", "/ask", None);
        let deny = super::super::operation_descriptor::build_http_descriptor("POST", "/deny", None);
        let allow =
            super::super::operation_descriptor::build_http_descriptor("POST", "/allow", None);
        crate::test_utils::set_test_approval_preference(db, &owner, true).await;
        for (operation, key, effect) in [
            (&ask, &successor, ApprovalEffect::RequireApproval),
            (&deny, &successor, ApprovalEffect::Deny),
            (&allow, &successor, ApprovalEffect::AutoAllow),
            (&deny, &sibling, ApprovalEffect::Deny),
            (&ask, &sibling, ApprovalEffect::AutoAllow),
        ] {
            let resolution = resolve_org_aware_approval(
                db,
                &owner,
                &owner,
                &service,
                operation,
                false,
                Some(key),
            )
            .await
            .unwrap();
            assert_eq!(resolution.effect, effect);
        }
        assert!(
            check_approval(db, &owner, &service, "api_key", &predecessor, false, &ask)
                .await
                .unwrap()
        );
        assert!(
            !check_approval(db, &owner, &service, "api_key", &successor, false, &ask)
                .await
                .unwrap()
        );
        let outcome = evaluate_and_check_with_options(
            db,
            &owner,
            &owner,
            &service,
            &ask,
            Some("api_key"),
            &successor,
            false,
            false,
            ApprovalOptions::default(),
            Some(&successor),
        )
        .await
        .unwrap();
        assert!(matches!(outcome, ApprovalOutcome::NeedsApproval(_)));
        assert!(matches!(
            put(
                &state,
                &owner,
                &service,
                serde_json::json!({"expected_revision": reviewed_revision, "rules": reviewed.rules})
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert!(matches!(
            rotate().await.unwrap(),
            ApiKeyRotationOutcome::AlreadyCommitted(_)
        ));
        assert_eq!(
            intents
                .count_documents(rotation_audit.clone())
                .await
                .unwrap(),
            1
        );
        let audit = intents
            .find_one(rotation_audit.clone())
            .await
            .unwrap()
            .unwrap();
        let metadata = audit
            .get_document("event")
            .unwrap()
            .get_document("event_data")
            .unwrap();
        assert_eq!(
            metadata.get_str("predecessor_key_id"),
            Ok(predecessor.as_str())
        );
        assert_eq!(metadata.get_str("successor_key_id"), Ok(successor.as_str()));
        assert_eq!(metadata.get_array("policies").unwrap().len(), 2);
        let new_key = rotation_key(db, &owner).await;
        assert!(
            !carried
                .rules
                .iter()
                .any(|rule| rule.api_key_ids.contains(&new_key))
        );
        let raw_carried = configs
            .find_one(doc! {"_id": &original.id})
            .await
            .unwrap()
            .unwrap();
        key_service::delete_api_key(db, &owner, &successor)
            .await
            .unwrap();
        assert_eq!(
            configs
                .find_one(doc! {"_id": &original.id})
                .await
                .unwrap()
                .unwrap(),
            raw_carried
        );
        assert_eq!(intents.count_documents(rotation_audit).await.unwrap(), 1);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn rotation_and_keyed_policy_writes_serialize_for_first_membership_and_legacy_saves() {
        use super::super::{
            api_key_mutation_service::TransactionCollisionHook,
            key_service::{self, ApiKeyRotationOutcome},
        };
        for (create, legacy) in [(true, false), (true, true), (false, false), (false, true)] {
            let (state, owner, service) = fixture("policy_rotation_race").await;
            let db = &state.db;
            let predecessor = rotation_key(db, &owner).await;
            let successor = Uuid::new_v4().to_string();
            let configs = db.collection::<Document>(SERVICE_APPROVAL_CONFIGS);
            let rules: Vec<ApprovalRule> = serde_json::from_value(
                serde_json::json!([{"effect": "deny", "api_key_ids": [&predecessor]}]),
            )
            .unwrap();
            if create {
                configs
                    .delete_one(doc! {"user_id": &owner, "service_id": &service})
                    .await
                    .unwrap();
            } else if legacy {
                configs
                    .update_one(
                        doc! {"user_id": &owner, "service_id": &service},
                        doc! {"$set": {"rules": bson::to_bson(&rules).unwrap()}},
                    )
                    .await
                    .unwrap();
            }
            let current = get_config(db, &owner, &service).await.unwrap();
            let expected_revision = (!legacy).then(|| {
                current
                    .as_ref()
                    .map(|cfg| config_revision(cfg).unwrap())
                    .unwrap_or_default()
            });
            let barrier = Arc::new(tokio::sync::Barrier::new(2));
            let save_hook = TransactionCollisionHook::new(barrier.clone());
            let rotation_hook = TransactionCollisionHook::new(barrier);
            let (save, rotation) = tokio::join!(
                set_config_with_options(
                    db,
                    &owner,
                    &service,
                    "fixture",
                    Some(false),
                    None,
                    Some(&rules),
                    None,
                    test_config_audit(&owner),
                    ConfigWriteOptions {
                        expected_revision,
                        collision_hook: Some(save_hook.clone()),
                        ..Default::default()
                    }
                ),
                key_service::rotate_api_key_with_scope_authorization_and_id_with_collision_hook(
                    db,
                    &state.encryption_keys,
                    &owner,
                    Some(&owner),
                    &predecessor,
                    &successor,
                    rotation_hook.clone()
                )
            );
            assert!(matches!(
                rotation.unwrap(),
                ApiKeyRotationOutcome::Created(_)
            ));
            assert!(save_hook.attempts() > 1 || rotation_hook.attempts() > 1);
            let after = get_config(db, &owner, &service).await.unwrap();
            if save.is_ok() || (!create && legacy) {
                assert_eq!(
                    after.as_ref().unwrap().rules[0].api_key_ids,
                    vec![successor.clone()]
                );
            } else {
                assert!(matches!(
                    save,
                    Err(AppError::Conflict(_)) | Err(AppError::ValidationError(_))
                ));
            }
            assert!(after.iter().all(|cfg| {
                cfg.rules
                    .iter()
                    .all(|r| !r.api_key_ids.contains(&predecessor))
            }));
            // The old client cannot restore the inactive predecessor even without a revision.
            assert!(matches!(
                set_config_with_options(
                    db,
                    &owner,
                    &service,
                    "fixture",
                    Some(false),
                    None,
                    Some(&rules),
                    None,
                    test_config_audit(&owner),
                    ConfigWriteOptions::default()
                )
                .await,
                Err(AppError::Conflict(_))
            ));
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn exact_no_op_preserves_legacy_policy_grant_and_pending_bytes() {
        let (state, owner, service) = fixture("policy_exact_noop").await;
        let configs = state.db.collection::<Document>(SERVICE_APPROVAL_CONFIGS);
        let before = configs
            .find_one(doc! {"service_id": &service})
            .await
            .unwrap()
            .unwrap();
        let cfg = get_config(&state.db, &owner, &service)
            .await
            .unwrap()
            .unwrap();
        let grant = state
            .db
            .collection::<Document>(GRANTS)
            .find_one(doc! {"_id": "legacy-grant"})
            .await
            .unwrap()
            .unwrap();
        let _ = put(
            &state,
            &owner,
            &service,
            serde_json::json!({
                "approval_required": cfg.approval_required,
                "approval_mode": cfg.approval_mode,
                "rules": cfg.rules,
                "default_effect": cfg.default_effect,
                "grant_expiry_days": null,
                "disabled": false,
                "expected_revision": config_revision(&cfg).unwrap()
            }),
        )
        .await
        .unwrap();
        assert_eq!(
            before,
            configs
                .find_one(doc! {"service_id": &service})
                .await
                .unwrap()
                .unwrap()
        );
        assert_eq!(
            grant,
            state
                .db
                .collection::<Document>(GRANTS)
                .find_one(doc! {"_id": "legacy-grant"})
                .await
                .unwrap()
                .unwrap()
        );
        assert_eq!(
            state
                .db
                .collection::<Document>(crate::models::security_effect_intent::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn invalid_days_handler_leaves_policy_and_grants_unchanged() {
        let (state, owner, service) = fixture("policy_invalid_days").await;
        let before = get_config(&state.db, &owner, &service)
            .await
            .unwrap()
            .unwrap();
        assert!(put(&state,&owner,&service,serde_json::json!({"approval_mode": "per_request","rules": [],"default_effect": "deny","grant_expiry_days": 366})).await.is_err());
        assert_eq!(
            config_revision(&before).unwrap(),
            config_revision(
                &get_config(&state.db, &owner, &service)
                    .await
                    .unwrap()
                    .unwrap()
            )
            .unwrap()
        );
        let grant = state
            .db
            .collection::<Document>(GRANTS)
            .find_one(doc! {"_id": "legacy-grant"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(grant.get_bool("revoked"), Ok(false));
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_edits_never_mix_rules_and_days_and_stale_save_cannot_resurrect() {
        let (state, owner, service) = fixture("policy_concurrent_days").await;
        let cfg = get_config(&state.db, &owner, &service)
            .await
            .unwrap()
            .unwrap();
        let revision = config_revision(&cfg).unwrap();
        let (a, b) = tokio::join!(
            put(
                &state,
                &owner,
                &service,
                serde_json::json!({"rules": [],"default_effect": "deny","grant_expiry_days": 1,"expected_revision": revision})
            ),
            put(
                &state,
                &owner,
                &service,
                serde_json::json!({"rules": [],"default_effect": "auto_allow","grant_expiry_days": 9,"expected_revision": revision})
            )
        );
        assert_ne!(a.is_ok(), b.is_ok());
        let cfg = get_config(&state.db, &owner, &service)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(
            (cfg.default_effect.as_ref(), cfg.grant_expiry_days),
            (Some(ApprovalEffect::Deny), Some(1)) | (Some(ApprovalEffect::AutoAllow), Some(9))
        ));
        let current = config_revision(&cfg).unwrap();
        delete_config_with_revision(
            &state.db,
            &owner,
            &service,
            test_config_audit(&owner),
            Some(&current),
        )
        .await
        .unwrap();
        assert!(matches!(put(&state,&owner,&service,serde_json::json!({"rules": cfg.rules,"approval_required": true,"expected_revision": current})).await,Err(AppError::Conflict(_))));
        assert!(
            get_config(&state.db, &owner, &service)
                .await
                .unwrap()
                .is_none()
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn disabled_policy_restores_global_default_without_rewriting_rules() {
        let (state, owner, service) = fixture("policy_disabled").await;
        let cfg = get_config(&state.db, &owner, &service)
            .await
            .unwrap()
            .unwrap();
        let _ = put(
            &state,
            &owner,
            &service,
            serde_json::json!({"disabled": true,"expected_revision": config_revision(&cfg).unwrap()}),
        )
        .await
        .unwrap();
        let disabled = get_config(&state.db, &owner, &service)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cfg.rules, disabled.rules);
        assert_eq!(cfg.default_effect, disabled.default_effect);
        let read =
            crate::services::operation_descriptor::build_http_descriptor("GET", "/status", None);
        let result =
            resolve_org_aware_approval(&state.db, &owner, &owner, &service, &read, false, None)
                .await
                .unwrap();
        assert_eq!(result.effect, ApprovalEffect::AutoAllow);
        assert!(!approval_policy::has_policy_for_key(&disabled, None));
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn issued_duration_defaults_are_frozen_and_explicit_shorter_decisions_survive() {
        let (state, owner, service) = fixture("policy_frozen_expiry").await;
        state
            .db
            .collection::<Document>(SERVICE_APPROVAL_CONFIGS)
            .update_one(
                doc! {"service_id": &service},
                doc! {"$set": {"grant_expiry_days": 2}},
            )
            .await
            .unwrap();
        let descriptor =
            crate::services::operation_descriptor::build_http_descriptor("POST", "/write", None);
        for custom in [Some(30_i64 * 86400), Some(3600)] {
            let requester = uuid::Uuid::new_v4().to_string();
            let request = create_approval_request(
                &state.db,
                &state.config,
                &state.http_client,
                None,
                None,
                &owner,
                &service,
                "Fixture",
                "fixture",
                "user",
                &requester,
                None,
                ApprovalRequestOperation::from_descriptor(&descriptor, None),
                ApprovalMode::Grant,
                300,
                vec![owner.clone()],
                false,
            )
            .await
            .unwrap();
            assert_eq!(request.grant_expiry_days_snapshot.as_ref().unwrap().days, 2);
            state
                .db
                .collection::<Document>(SERVICE_APPROVAL_CONFIGS)
                .update_one(
                    doc! {"service_id": &service},
                    doc! {"$set": {"grant_expiry_days": 9}},
                )
                .await
                .unwrap();
            process_decision(
                &state.db,
                &state.config,
                &state.http_client,
                None,
                None,
                &request.id,
                true,
                custom,
                None,
                "mobile",
            )
            .await
            .unwrap();
            let grant = state
                .db
                .collection::<ApprovalGrant>(GRANTS)
                .find_one(doc! {"approval_request_id": &request.id})
                .await
                .unwrap()
                .unwrap();
            let expected = custom.unwrap().min(2 * 86400);
            assert!(((grant.expires_at - grant.granted_at).num_seconds() - expected).abs() <= 1);
            state
                .db
                .collection::<Document>(SERVICE_APPROVAL_CONFIGS)
                .update_one(
                    doc! {"service_id": &service},
                    doc! {"$set": {"grant_expiry_days": 2}},
                )
                .await
                .unwrap();
        }
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn mixed_fleet_refuses_new_rule_authoring_without_partial_mutation() {
        let (state, owner, service) = fixture("policy_mixed_fleet").await;
        let before = get_config(&state.db, &owner, &service)
            .await
            .unwrap()
            .unwrap();
        state
            .db
            .collection::<Document>(crate::models::security_replica::COLLECTION_NAME)
            .update_many(doc! {}, doc! {"$pull": {"capabilities": RULES_READINESS}})
            .await
            .unwrap();
        assert!(matches!(
            put(
                &state,
                &owner,
                &service,
                serde_json::json!({"grant_expiry_days": 3,"default_effect": "auto_allow"})
            )
            .await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        assert_eq!(
            config_revision(&before).unwrap(),
            config_revision(
                &get_config(&state.db, &owner, &service)
                    .await
                    .unwrap()
                    .unwrap()
            )
            .unwrap()
        );
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn pending_reconciliation_uses_exact_key_and_ssh_evidence() {
        let (state, owner, service) = fixture("policy_pending_evidence").await;
        state
            .db
            .collection::<Document>(crate::models::api_key::COLLECTION_NAME)
            .insert_one(doc! {"_id": "key-a", "user_id": &owner, "is_active": true})
            .await
            .unwrap();
        let now = bson::DateTime::now();
        for (id, key, protocol) in [
            ("a", Some("key-a"), Some("ssh")),
            ("b", Some("key-b"), Some("ssh")),
            ("legacy", None, None),
        ] {
            state.db.collection::<Document>(REQUESTS).insert_one(doc! {
                "_id": id,"user_id": &owner,"service_id": &service,"service_name": "Fixture","service_slug": "fixture",
                "requester_type": "user","requester_id": &owner,"caller_api_key_id": key,"approval_protocol": protocol,
                "operation_summary": "ssh:EXEC rm file","http_method": "EXEC","resource": "rm file","verb": "write",
                "idempotency_key": id,"status": "pending","approval_mode": "grant","created_at": now,
                "expires_at": bson::DateTime::from_chrono(Utc::now()+Duration::minutes(10))
            }).await.unwrap();
        }
        let rules:Vec<ApprovalRule>=serde_json::from_value(serde_json::json!([
            {"methods": ["EXEC"],"resource_pattern": "rm *","verbs": ["write"],"effect": "deny","api_key_ids": ["key-a"]},
            {"methods": [],"resource_pattern": "*","verbs": [],"effect": "require_approval","mode": "grant"}
        ])).unwrap();
        set_config_with_options(
            &state.db,
            &owner,
            &service,
            "Fixture",
            Some(true),
            Some(&ApprovalMode::Grant),
            Some(&rules),
            Some(&ApprovalEffect::RequireApproval),
            test_config_audit(&owner),
            // A revision-aware editor saving the reviewed policy; callers
            // without a revision keep the legacy reconciliation contract.
            ConfigWriteOptions {
                expected_revision: Some(
                    config_revision(
                        &get_config(&state.db, &owner, &service)
                            .await
                            .unwrap()
                            .unwrap(),
                    )
                    .unwrap(),
                ),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        for (id, status) in [("a", "expired"), ("b", "pending"), ("legacy", "pending")] {
            assert_eq!(get_request(&state.db, id).await.unwrap().status, status);
        }
        let descriptor = OperationDescriptor {
            protocol: super::super::operation_descriptor::Protocol::Ssh,
            verb: ApprovalVerb::Write,
            method: Some("EXEC".into()),
            resource: Some("rm file".into()),
            summary: "Remove file".into(),
        };
        assert!(
            recheck_after_decision(
                &state.db,
                &owner,
                &owner,
                &service,
                &descriptor,
                false,
                Some("key-a"),
                &ApprovalMode::Grant
            )
            .await
            .is_err()
        );
        recheck_after_decision(
            &state.db,
            &owner,
            &owner,
            &service,
            &descriptor,
            false,
            Some("key-b"),
            &ApprovalMode::Grant,
        )
        .await
        .unwrap();
        state.db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn hidden_expired_keys_and_org_auth_cannot_author_personal_scopes() {
        let (state, owner, service) = fixture("policy_key_ownership").await;
        for (id, purpose, expiry) in [
            ("public", "public_nyxbot", None),
            ("shared", "shared_entry_transport", None),
            (
                "expired",
                "agent",
                Some(bson::DateTime::from_chrono(
                    Utc::now() - Duration::minutes(1),
                )),
            ),
        ] {
            state.db.collection::<Document>(crate::models::api_key::COLLECTION_NAME).insert_one(doc! {"_id": id,"user_id": &owner,"is_active": true,"purpose": purpose,"expires_at": expiry}).await.unwrap();
            assert!(matches!(
                put(
                    &state,
                    &owner,
                    &service,
                    serde_json::json!({"rules": [{"effect": "deny","api_key_ids": [id]}]})
                )
                .await,
                Err(AppError::ValidationError(_))
            ));
        }
        state
            .db
            .collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(doc! {"_id": &owner}, doc! {"$set": {"user_type": "org"}})
            .await
            .unwrap();
        for body in [
            serde_json::json!({"disabled": true}),
            serde_json::json!({"rules": [{"effect": "auto_allow","api_key_ids": ["org-key"]}]}),
        ] {
            assert!(matches!(
                put(&state, &owner, &service, body).await,
                Err(AppError::ValidationError(_))
            ));
        }
        state.db.drop().await.unwrap();
    }

    #[test]
    fn scoped_ask_is_fresh_but_ordinary_ask_keeps_legacy_behavior() {
        let mut cfg = bson::from_document::<ServiceApprovalConfig>(doc! {
            "_id": "policy",
            "user_id": "owner",
            "service_id": "service",
            "service_name": "Service",
            "approval_required": true,
            "created_at": bson::DateTime::now(),
            "updated_at": bson::DateTime::now()
        })
        .unwrap();
        let operation =
            crate::services::operation_descriptor::build_http_descriptor("POST", "/write", None);
        cfg.rules = vec![ApprovalRule {
            mandatory_review: false,
            methods: vec![],
            resource_pattern: "*".into(),
            verbs: vec![ApprovalVerb::Write],
            effect: ApprovalEffect::RequireApproval,
            mode: ApprovalMode::PerRequest,
            api_key_ids: vec!["key-a".into()],
        }];
        assert!(approval_policy::key_requires_fresh_decision(
            &cfg,
            &operation,
            Some("key-a")
        ));
        assert!(!approval_policy::key_requires_fresh_decision(
            &cfg,
            &operation,
            Some("key-b")
        ));
        cfg.rules[0].api_key_ids.clear();
        assert!(!approval_policy::key_requires_fresh_decision(
            &cfg,
            &operation,
            Some("key-a")
        ));
        cfg.disabled = true;
        assert!(approval_policy::evaluate(&cfg, &operation, None).is_none());
    }
}

/// Waiters retain their authenticated caller and prepared descriptor. Recheck
/// those live inputs after a decision so even a pre-snapshot pending record
/// cannot resume through a subsequently added Deny or fresh per-request floor.
#[allow(clippy::too_many_arguments)]
pub async fn recheck_after_decision(
    db: &Database,
    actor: &str,
    owner: &str,
    service: &str,
    descriptor: &OperationDescriptor,
    auto_connected: bool,
    key: Option<&str>,
    issued_mode: &ApprovalMode,
) -> AppResult<()> {
    let current =
        resolve_org_aware_approval(db, actor, owner, service, descriptor, auto_connected, key)
            .await?;
    if current.effect == ApprovalEffect::Deny
        || (current.required
            && current.mode == ApprovalMode::PerRequest
            && *issued_mode == ApprovalMode::Grant)
    {
        return Err(AppError::Forbidden(
            "Approval policy changed; request approval again".into(),
        ));
    }
    Ok(())
}
