use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::Database;
use mongodb::bson::{self, doc};
use mongodb::options::{FindOneAndUpdateOptions, FindOptions, ReturnDocument};
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::sync::Arc;

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::approval_grant::{ApprovalGrant, COLLECTION_NAME as GRANTS};
use crate::models::approval_request::{ApprovalRequest, COLLECTION_NAME as REQUESTS};
use crate::models::notification_channel::{COLLECTION_NAME as CHANNELS, NotificationChannel};
use crate::models::service_approval_config::{
    ApprovalEffect, ApprovalMode, ApprovalRule, COLLECTION_NAME as SERVICE_APPROVAL_CONFIGS,
    ServiceApprovalConfig,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::services::approval_policy;
use crate::services::notification_service;
use crate::services::operation_descriptor::OperationDescriptor;
use crate::services::push_service::{ApnsAuth, FcmAuth};
use crate::telemetry::{TelemetryClient, TelemetryContext, TelemetryEvent, emit_event};

/// Resolve the approval mode for a (user, service) pair.
pub async fn resolve_approval_mode(
    db: &Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<ApprovalMode> {
    let per_service = db
        .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
        .find_one(doc! { "user_id": user_id, "service_id": service_id })
        .await?;

    Ok(per_service.map(|c| c.approval_mode).unwrap_or_default())
}

/// Check whether a user has the global approval system enabled.
pub async fn user_requires_approval(db: &Database, user_id: &str) -> AppResult<bool> {
    Ok(user_global_approval_setting(db, user_id)
        .await?
        .unwrap_or(false))
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
///    the actor's per-service or global setting applies (existing behavior).
///    The request `primary_owner_user_id` is the actor.
///
/// This is the cleanest semantic: the resource owner's policy is
/// authoritative when set, and falls back to the actor's preference when
/// the owner has not configured one.
#[derive(Debug, Clone)]
pub struct ApprovalResolution {
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
pub async fn resolve_org_aware_approval(
    db: &Database,
    actor_user_id: &str,
    service_owner_user_id: &str,
    service_id: &str,
    descriptor: &OperationDescriptor,
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
            })
            .await?
    {
        let decision = approval_policy::evaluate(&org_config, descriptor);
        return Ok(ApprovalResolution {
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

    let (required, mode, effect, grant_scope) = if let Some(cfg) = actor_config {
        let decision = approval_policy::evaluate(&cfg, descriptor);
        (
            decision.effect == ApprovalEffect::RequireApproval,
            decision.mode,
            decision.effect,
            decision.grant_scope,
        )
    } else {
        let required = user_requires_approval(db, actor_user_id).await?;
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
) -> AppResult<bool> {
    let resolution = resolve_org_aware_approval(
        db,
        actor_user_id,
        service_owner_user_id,
        service_id,
        descriptor,
    )
    .await?;
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
) -> AppResult<ApprovalOutcome> {
    let resolution = resolve_org_aware_approval(
        db,
        actor_user_id,
        service_owner_user_id,
        service_id,
        descriptor,
    )
    .await?;

    if resolution.effect == ApprovalEffect::Deny {
        return Ok(ApprovalOutcome::Denied);
    }

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
        resolution,
        primary_owner_user_id,
        requester_type: requester_type.to_string(),
        requester_id: requester_id.to_string(),
    }))
}

pub async fn approval_notification_recipients(
    db: &Database,
    approval_owner_user_id: &str,
    pending: &PendingApproval,
) -> AppResult<Vec<String>> {
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
    Ok(channel.map(|c| c.approval_required))
}

/// Pure resolution helper kept for unit-testable semantics. Used to be the
/// final step of the now-removed `requires_approval_for_service`. Tests
/// still exercise it directly.
#[cfg(test)]
fn resolve_approval_requirement(per_service: Option<bool>, global: Option<bool>) -> bool {
    per_service.or(global).unwrap_or(false)
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
    pub operation_summary: String,
    pub action_description: Option<String>,
    pub http_method: Option<String>,
    pub resource: Option<String>,
    pub verb: Option<String>,
    pub grant_scope: Option<String>,
}

impl ApprovalRequestOperation {
    pub fn from_descriptor(descriptor: &OperationDescriptor, grant_scope: Option<String>) -> Self {
        Self {
            operation_summary: descriptor.operation_summary(),
            action_description: Some(descriptor.summary.clone()),
            http_method: descriptor.method.clone(),
            resource: descriptor.resource.clone(),
            verb: Some(descriptor.verb.as_str().to_string()),
            grant_scope,
        }
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
    let notify_user_ids = if notify_user_ids.is_empty() {
        vec![user_id.to_string()]
    } else {
        notify_user_ids
    };

    let collection = db.collection::<ApprovalRequest>(REQUESTS);
    let idempotency_key = compute_pending_request_idempotency_key(
        &approval_mode,
        user_id,
        service_id,
        requester_type,
        requester_id,
        from_org_policy,
    );
    let mut inserted_request: Option<ApprovalRequest> = None;
    for _attempt in 0..2 {
        // Check for existing pending request with the same idempotency key.
        // This handles normal idempotent retries and the winner in concurrent inserts.
        if let Some(existing) = collection
            .find_one(doc! {
                "idempotency_key": &idempotency_key,
                "status": "pending",
            })
            .await?
        {
            return Ok(existing);
        }

        let now = Utc::now();
        let expires_at = now + Duration::seconds(i64::from(timeout_secs));

        let request = ApprovalRequest {
            id: uuid::Uuid::new_v4().to_string(),
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
            created_at: now,
        };

        match collection.insert_one(&request).await {
            Ok(_) => {
                inserted_request = Some(request);
                break;
            }
            Err(e) if is_duplicate_key_error(&e) => {
                // Concurrent insert race: another request inserted/processed first.
                // Retry once to read the pending row or create a new row if no longer pending.
                continue;
            }
            Err(e) => return Err(AppError::DatabaseError(e)),
        }
    }

    let request = inserted_request
        .ok_or_else(|| AppError::Conflict("Approval request conflict, please retry".to_string()))?;

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

    for recipient in &notify_user_ids {
        match notification_service::send_approval_notification(
            db,
            config,
            http_client,
            fcm_auth,
            apns_auth,
            recipient,
            &request,
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
        return Ok(request);
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

    let updated = collection
        .find_one(doc! { "_id": &request.id })
        .await?
        .unwrap_or(request);

    Ok(updated)
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
) -> AppResult<ApprovalRequest> {
    let channel = db
        .collection::<NotificationChannel>(CHANNELS)
        .find_one(doc! { "user_id": user_id })
        .await?;

    let timeout_secs = channel
        .as_ref()
        .map(|c| c.approval_timeout_secs)
        .unwrap_or(30);

    let collection = db.collection::<ApprovalRequest>(REQUESTS);

    // Tool approvals are always unique (per_request semantics).
    let idempotency_key = uuid::Uuid::new_v4().to_string();

    let now = Utc::now();
    let expires_at = now + Duration::seconds(i64::from(timeout_secs));

    let operation_summary = format!("tool:{tool_name}");
    let action_description = tool_arguments.map(|args| format!("{tool_name}({args})"));

    let request = ApprovalRequest {
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
        created_at: now,
    };

    collection
        .insert_one(&request)
        .await
        .map_err(AppError::DatabaseError)?;

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

/// Process a user's approval decision (from Telegram callback or web UI).
/// Atomically updates status from "pending" to "approved"/"rejected".
/// On approval: creates an ApprovalGrant with the user's configured expiry.
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
    let now = Utc::now();
    let new_status = if approved { "approved" } else { "rejected" };
    let collection = db.collection::<ApprovalRequest>(REQUESTS);
    let mut update_set = doc! {
        "status": new_status,
        "decided_at": bson::DateTime::from_chrono(now),
        "decision_channel": decision_channel,
    };
    if let Some(key) = idempotency_key {
        update_set.insert("decision_idempotency_key", key);
    }

    // Atomic update: only process if status is still "pending"
    let updated = collection
        .find_one_and_update(
            doc! {
                "_id": request_id,
                "status": "pending",
            },
            doc! {
                "$set": update_set
            },
        )
        .with_options(
            FindOneAndUpdateOptions::builder()
                .return_document(ReturnDocument::After)
                .build(),
        )
        .await?;

    let updated = match updated {
        Some(updated) => updated,
        None => {
            let existing = get_request(db, request_id).await?;
            // Provide a specific error for expired requests instead of the
            // generic "decision_state_conflict" from is_idempotent_replay.
            if existing.status == "expired" {
                return Err(AppError::Forbidden("Approval request expired".to_string()));
            }
            if is_idempotent_replay(
                &existing.status,
                existing.decision_idempotency_key.as_deref(),
                idempotency_key,
                approved,
            )? {
                return Ok(existing);
            }
            return Err(AppError::Conflict("already_decided".to_string()));
        }
    };

    // Guard: reject decisions on grant-mode requests if the service has since
    // switched to per_request. Normally these requests are cancelled at
    // mode-switch time (#153), but this catches any that slip through a
    // TOCTOU race. We roll back the decision so the request stays "pending"
    // (and will be picked up by the next expiry sweep).
    if approved && requires_legacy_grant_mode_recheck(&updated) {
        let current_mode = resolve_approval_mode(db, &updated.user_id, &updated.service_id).await?;
        if current_mode != ApprovalMode::Grant {
            // Roll back the decision atomically. The filter guards against a
            // concurrent rollback so we don't clobber a different status.
            let rollback = db
                .collection::<ApprovalRequest>(REQUESTS)
                .update_one(
                    doc! { "_id": request_id, "status": new_status },
                    doc! { "$set": {
                        "status": "expired",
                        "decided_at": bson::DateTime::from_chrono(now),
                    }},
                )
                .await
                .map_err(AppError::DatabaseError)?;

            if rollback.matched_count == 0 {
                // Another process already changed the status (e.g. expiry sweep).
                // The request is no longer "approved", so the conflict is resolved.
                tracing::warn!(
                    request_id,
                    "Rollback matched no document; concurrent state change"
                );
            }

            return Err(AppError::Conflict(
                "Service approval mode has changed; this request is no longer valid".to_string(),
            ));
        }
    }

    // On approval: create a grant when the request was originally created
    // in grant mode AND the service is still in grant mode (verified above).
    //
    // Note: a TOCTOU race exists if a concurrent request switches the service
    // to per_request between the check above and the insert_one below. If
    // that happens, the grant is inert: the proxy handler skips grants in
    // per_request mode, and list_grants() filters them out at read time.
    // The grant will expire naturally.
    //
    // For org-policy requests (`updated.from_org_policy == true`) the grant
    // is minted as `org_scoped` so it covers every member of the owning org
    // for its full lifetime (see ChronoAIProject/NyxID#364). Channel defaults
    // (grant expiry days) fall back to the request owner, which for org-policy
    // is the org itself (no channel row) -- `get_or_create_channel` creates a
    // default row whose `grant_expiry_days` is the system default, so the
    // expiry computation still works.
    if approved && updated.approval_mode == ApprovalMode::Grant {
        let channel = notification_service::get_or_create_channel(db, &updated.user_id).await?;
        let grant_expiry = resolve_grant_expiry(now, duration_sec, channel.grant_expiry_days);

        let grant = ApprovalGrant {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: updated.user_id.clone(),
            service_id: updated.service_id.clone(),
            service_name: updated.service_name.clone(),
            requester_type: updated.requester_type.clone(),
            requester_id: updated.requester_id.clone(),
            requester_label: updated.requester_label.clone(),
            approval_request_id: updated.id.clone(),
            scope: updated.grant_scope.clone(),
            granted_at: now,
            expires_at: grant_expiry,
            revoked: false,
            org_scoped: updated.from_org_policy,
        };

        db.collection::<ApprovalGrant>(GRANTS)
            .insert_one(&grant)
            .await?;
    }

    // Notify channels about the decision (best-effort, non-blocking).
    let request_clone = updated.clone();
    let config_clone = config.clone();
    let http_clone = http_client.clone();
    let db_clone = db.clone();
    let fcm_auth_clone = fcm_auth.clone();
    let apns_auth_clone = apns_auth.clone();
    tokio::spawn(async move {
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
    let expire_result = db
        .collection::<ApprovalRequest>(REQUESTS)
        .update_many(
            doc! { "_id": { "$in": &ids }, "status": "pending" },
            doc! { "$set": {
                "status": "expired",
                "decided_at": sweep_time,
                "decision_idempotency_key": &sweep_marker,
            } },
        )
        .await?;

    let count = expire_result.modified_count;
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
            tokio::spawn(async move {
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
        );
    }

    Ok(count)
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

/// Block until an approval decision is made or the timeout expires.
/// Returns Ok(()) if approved, Err if rejected/expired/timeout.
pub async fn wait_for_decision(
    db: &Database,
    request_id: &str,
    timeout_secs: u32,
) -> AppResult<()> {
    let poll_interval = std::time::Duration::from_millis(1000);
    let deadline = Utc::now() + Duration::seconds(i64::from(timeout_secs));

    loop {
        tokio::time::sleep(poll_interval).await;

        let request = get_request(db, request_id).await?;

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
                if Utc::now() >= deadline {
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
            approve_url: format!("{}/approvals/history", frontend_url.trim_end_matches('/')),
            reason,
        },
        other => other,
    }
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
    let mut filter = build_requests_filter(user_id, admin_branches);
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
    db.collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! { "_id": request_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Approval request not found".to_string()))
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
/// `grant_mode_by_owner` maps an owner `user_id` → its grant-mode
/// service ids. An owner missing from the map has no services in
/// grant mode and contributes no grants.
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
/// Only returns grants for services currently in `Grant` mode.
/// Grants for services in `PerRequest` mode (or with no config,
/// which defaults to `PerRequest`) are excluded even if they
/// haven't been revoked yet. This read-time filter acts as a
/// safety net for any write-time race conditions or partial
/// failures during mode switches (see #146).
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
    let configs: Vec<ServiceApprovalConfig> = db
        .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
        .find(doc! {
            "user_id": { "$in": ids_bson },
            "approval_mode": "grant",
        })
        .await?
        .try_collect()
        .await?;
    let mut grant_mode_by_owner: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for c in configs {
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

    let filter = build_grants_filter(user_id, admin_branches, &grant_mode_by_owner, now);

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

/// Revoke a specific approval grant.
pub async fn revoke_grant(db: &Database, user_id: &str, grant_id: &str) -> AppResult<()> {
    let result = db
        .collection::<ApprovalGrant>(GRANTS)
        .update_one(
            doc! {
                "_id": grant_id,
                "user_id": user_id,
            },
            doc! { "$set": { "revoked": true } },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("Grant not found".to_string()));
    }

    Ok(())
}

/// Revoke all grants for a user.
#[allow(dead_code)]
pub async fn revoke_all_grants(db: &Database, user_id: &str) -> AppResult<u64> {
    let result = db
        .collection::<ApprovalGrant>(GRANTS)
        .update_many(
            doc! { "user_id": user_id, "revoked": false },
            doc! { "$set": { "revoked": true } },
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

/// Set a per-service approval config (atomic upsert).
/// If a config already exists for (user, service), it is updated.
/// Otherwise, a new config is created. Uses `findOneAndUpdate` with
/// `upsert: true` to avoid race conditions from concurrent requests.
#[allow(clippy::too_many_arguments)]
pub async fn set_service_approval_config(
    db: &Database,
    user_id: &str,
    service_id: &str,
    service_name: &str,
    approval_required: Option<bool>,
    approval_mode: Option<&ApprovalMode>,
    rules: Option<&[ApprovalRule]>,
    default_effect: Option<&ApprovalEffect>,
) -> AppResult<ServiceApprovalConfig> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let collection = db.collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS);
    let filter = doc! { "user_id": user_id, "service_id": service_id };
    let existing = collection.find_one(filter.clone()).await?;
    let (approval_required, approval_mode, rules, default_effect) = resolve_service_config_update(
        existing.as_ref(),
        approval_required,
        approval_mode,
        rules,
        default_effect,
    )?;

    for _attempt in 0..2 {
        let rules_bson = bson::to_bson(&rules)
            .map_err(|e| AppError::Internal(format!("Failed to serialize approval rules: {e}")))?;
        let default_effect_bson = bson::to_bson(&default_effect).map_err(|e| {
            AppError::Internal(format!("Failed to serialize approval default effect: {e}"))
        })?;
        let config = collection
            .find_one_and_update(
                filter.clone(),
                doc! {
                    "$set": {
                        "approval_required": approval_required,
                        "approval_mode": approval_mode.as_str(),
                        "rules": rules_bson,
                        "default_effect": default_effect_bson,
                        "service_name": service_name,
                        "updated_at": now,
                    },
                    "$setOnInsert": {
                        "_id": uuid::Uuid::new_v4().to_string(),
                        "user_id": user_id,
                        "service_id": service_id,
                        "created_at": now,
                    }
                },
            )
            .with_options(
                FindOneAndUpdateOptions::builder()
                    .upsert(true)
                    .return_document(ReturnDocument::After)
                    .build(),
            )
            .await;

        match config {
            Ok(Some(cfg)) => {
                // When the persisted mode is per_request, clean up stale state:
                // 1. Revoke active grants so they no longer authorize proxy calls (#146)
                // 2. Cancel pending grant-mode requests so they can't be approved (#153)
                // Both operations are idempotent, so retries after partial failure
                // still clean up.
                if cfg.approval_mode == ApprovalMode::PerRequest {
                    revoke_grants_for_service(db, user_id, service_id).await?;
                    cancel_pending_grant_requests(db, user_id, service_id).await?;
                }
                return Ok(cfg);
            }
            Ok(None) => {
                return Err(AppError::Internal(
                    "Upsert returned no document".to_string(),
                ));
            }
            Err(e) if is_duplicate_key_error(&e) => {
                // Concurrent upserts can race on the unique (user_id, service_id) index.
                // Read-after-write resolves to the winning document.
                if let Some(existing) = collection.find_one(filter.clone()).await? {
                    if existing.approval_mode == ApprovalMode::PerRequest {
                        revoke_grants_for_service(db, user_id, service_id).await?;
                        cancel_pending_grant_requests(db, user_id, service_id).await?;
                    }
                    return Ok(existing);
                }
                continue;
            }
            Err(e) => return Err(AppError::DatabaseError(e)),
        }
    }

    Err(AppError::Conflict(
        "Per-service approval config update conflicted, please retry".to_string(),
    ))
}

/// Delete a per-service approval config (revert to global default).
/// Because the global default mode is `PerRequest`, deleting any override
/// effectively switches the service to `per_request`. Active grants are
/// revoked unconditionally so they don't linger (see #146). The revoke is
/// idempotent, so retries after partial failure still clean up.
pub async fn delete_service_approval_config(
    db: &Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<()> {
    let result = db
        .collection::<ServiceApprovalConfig>(SERVICE_APPROVAL_CONFIGS)
        .delete_one(doc! { "user_id": user_id, "service_id": service_id })
        .await?;

    // Always revoke grants and cancel pending grant-mode requests regardless
    // of deleted_count. If a previous attempt deleted the config but failed on
    // cleanup, this retry still cleans up. Both operations are no-ops when
    // nothing matches.
    revoke_grants_for_service(db, user_id, service_id).await?;
    cancel_pending_grant_requests(db, user_id, service_id).await?;

    if result.deleted_count == 0 {
        return Err(AppError::NotFound(
            "Per-service approval config not found".to_string(),
        ));
    }

    Ok(())
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
    approval_policy::validate_rules(&resolved_rules).map_err(AppError::ValidationError)?;

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

/// Cancel all pending grant-mode approval requests for a (user, service) pair.
/// Called when a service switches to `per_request` mode so stale grant requests
/// are no longer actionable (see #153).
///
/// Uses `$in: ["grant", null]` to also catch legacy requests that pre-date the
/// `approval_mode` field -- those deserialize as `Grant` via
/// `legacy_approval_mode_default` but have no stored value in MongoDB.
async fn cancel_pending_grant_requests(
    db: &Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());
    db.collection::<ApprovalRequest>(REQUESTS)
        .update_many(
            doc! {
                "user_id": user_id,
                "service_id": service_id,
                "status": "pending",
                "approval_mode": { "$in": ["grant", null] },
            },
            doc! { "$set": { "status": "expired", "decided_at": now } },
        )
        .await?;
    Ok(())
}

/// Revoke all active (non-revoked) grants for a (user, service) pair.
/// Called after switching a service to `per_request` mode so stale grants
/// no longer appear in the UI (see #146).
async fn revoke_grants_for_service(
    db: &Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<()> {
    db.collection::<ApprovalGrant>(GRANTS)
        .update_many(
            doc! {
                "user_id": user_id,
                "service_id": service_id,
                "revoked": false,
            },
            doc! { "$set": { "revoked": true } },
        )
        .await?;
    Ok(())
}

fn is_duplicate_key_error(e: &mongodb::error::Error) -> bool {
    if let mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(we)) =
        e.kind.as_ref()
    {
        return we.code == 11000;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_operation() -> OperationDescriptor {
        crate::services::operation_descriptor::build_http_descriptor("GET", "/v1/models", None)
    }

    fn make_request(id: &str, status: &str) -> ApprovalRequest {
        let now = Utc::now();
        ApprovalRequest {
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
            created_at: now,
        }
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
    fn resolve_approval_requirement_defaults_to_false_when_no_settings() {
        assert!(!resolve_approval_requirement(None, None));
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
    fn resolve_service_config_update_preserves_existing_mode_when_omitted() {
        let existing = ServiceApprovalConfig {
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
                && approve_url == "https://app.nyxid.dev/approvals/history"
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

    // ================================================================
    // Integration tests (require running MongoDB)
    // ================================================================

    use crate::models::user::UserType;
    use crate::test_utils::{connect_test_database, test_user};

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
            telegram_chat_id: None,
            telegram_username: None,
            telegram_enabled: false,
            telegram_link_code: None,
            telegram_link_code_expires_at: None,
            approval_timeout_secs: 30,
            grant_expiry_days: 30,
            approval_required,
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

        let mode = resolve_approval_mode(&db, &user_id, &service_id)
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

        let mode = resolve_approval_mode(&db, &user_id, &service_id)
            .await
            .unwrap();
        assert_eq!(mode, ApprovalMode::Grant);
    }

    // --- user_requires_approval ---

    #[tokio::test]
    async fn user_requires_approval_false_when_no_channel() {
        let Some(db) = connect_test_database("appr_svc_user_req_no_ch").await else {
            eprintln!("skipping: no MongoDB");
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();

        let result = user_requires_approval(&db, &user_id).await.unwrap();
        assert!(!result);
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

        let result = user_requires_approval(&db, &user_id).await.unwrap();
        assert!(result);
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

        delete_service_approval_config(&db, &user_id, &service_id)
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

        let result = delete_service_approval_config(&db, "user-1", "nonexistent-svc").await;
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

        delete_service_approval_config(&db, &user_id, &service_id)
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
        let resolution =
            resolve_org_aware_approval(&db, &actor_id, &org_user_id, &service_id, &operation)
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
        let resolution =
            resolve_org_aware_approval(&db, &actor_id, &org_user_id, &service_id, &operation)
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
        let resolution =
            resolve_org_aware_approval(&db, &actor_id, &actor_id, &service_id, &operation)
                .await
                .unwrap();
        assert!(resolution.required);
        assert_eq!(resolution.mode, ApprovalMode::default());
        assert_eq!(resolution.primary_owner_user_id, actor_id);
        assert!(!resolution.from_org_policy);
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
}
