//! Assistant-only connection selection and exact organization consent.
//!
//! Caller arguments are proposals. Only an authenticated owner turn can select
//! a connection. Availability evidence and pool pins are produced by the server.
use std::collections::HashMap;

use bson::{Document, doc};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};
use serde_json::{Value, json};

use super::{
    assistant_acknowledgement_service as acks, mcp_service,
    service_preference_service as preferences,
};
use crate::{
    errors::{AppError, AppResult},
    models::{
        api_key::ApiKey, user_api_key::UserApiKey, user_endpoint::UserEndpoint,
        user_service::UserService,
    },
};

mod admission;
mod async_watch;
pub(crate) use async_watch::{AsyncContinuation, async_contract_binding};
mod egress;
mod lark_managed;
pub(crate) use egress::{HttpEgressGuard, OwnedHttpEgressGuard};
pub(crate) use lark_managed::LarkManagedGuard;
mod node_routes;
mod pools;
pub use admission::Admission;
pub use node_routes::verify_node_route;
pub use pools::pinned_pool_member;
mod bootstrap;
pub use bootstrap::{begin_model_attempt, finish_model_attempt};
mod authority;
pub use authority::connection_binding;
mod availability;
mod guest_model;
mod guest_model_defaults;
mod http;
mod receipts;
mod request_identity;
pub(crate) use receipts::HttpApprovalReceipt;
mod resolution;
mod selection;
use availability::verified_unavailable;
pub use availability::{NodeFailureEvidence, observe_transport};
#[cfg(test)]
pub(crate) use guest_model::{is_shared_chat, owner_model_route_allowed};
#[cfg(test)]
pub use http::check_http;
pub use http::{HttpProjection, check_http_projected, verify_resolved, verify_resolved_projected};
use request_identity::selection_fingerprint;
pub use request_identity::{bind_headers, http_operation};
pub(crate) use request_identity::{stateless_inference, stateless_model_transport};
pub use resolution::resolve_implicit_for_request;
#[cfg(test)]
use selection::owner_selector;
use selection::scope;
pub use selection::{bind_owner_selection, live_authority};
pub(crate) use selection::{personal_authority, require_catalog_only_selection};

pub const COLLECTION: &str = "assistant_service_execution";
pub const ACK_HEADER: &str = "x-nyxid-acknowledgement-id";
const MAX_EVIDENCE: usize = 32;

fn digest(value: Value) -> String {
    mcp_service::canonical_sha256(value)
}

/// Alternatives must preserve account as well as destination. Shared credential
/// identity is provable; unrelated OAuth authorizations are never equivalent just
/// because they have the same provider or URL. Inference has no implicit account
/// resource, and is ranked only inside its provider's immutable catalog group.
fn equivalent(a: &UserService, b: &UserService, inference: bool) -> bool {
    a.catalog_service_id.is_some()
        && a.catalog_service_id == b.catalog_service_id
        && a.service_type == b.service_type
        && (inference
            || (a.api_key_id.is_some() && a.api_key_id == b.api_key_id)
            || (a.auth_method == "none" && b.auth_method == "none"))
}

fn selection_matches(row: &Document, service: &UserService, binding: &str) -> bool {
    if row.get_str("selection_source") == Ok("owner") {
        row.get_str("owner_id") == Ok(service.user_id.as_str())
            && row.get("catalog_service_id")
                == bson::to_bson(&service.catalog_service_id).ok().as_ref()
    } else {
        row.get_str("binding") == Ok(binding)
    }
}

pub struct Decision {
    pub refusal: Option<Value>,
    pub admission: Option<Admission>,
    pub automatic: Option<acks::AutoAllowed>,
}

/// A machine job's dynamic requests use its bounded command grant. The
/// ingress identity is private and has already passed node-signature checks.
#[allow(clippy::too_many_arguments)]
pub fn check_job_http<'a>(
    state: &'a crate::AppState,
    job: &'a str,
    id: Option<&'a str>,
    owner: Option<&'a str>,
    target: &'a super::proxy_service::ProxyTarget,
    method: &'a str,
    path: &'a str,
    projection: &'a HttpProjection,
) -> futures::future::BoxFuture<'a, AppResult<Decision>> {
    Box::pin(check_job_http_inner(
        state,
        job,
        id,
        owner,
        target,
        (method, path),
        projection,
    ))
}

async fn check_job_http_inner(
    state: &crate::AppState,
    job: &str,
    id: Option<&str>,
    owner: Option<&str>,
    target: &super::proxy_service::ProxyTarget,
    operation: (&str, &str),
    projection: &HttpProjection,
) -> AppResult<Decision> {
    let Some(id) = id else {
        // Catalog/platform credentials have no UserService owner. Their ACL
        // and credential lane remain the ordinary proxy's responsibility;
        // detached execution still needs the exact live job declaration.
        super::assistant_job_service_use::validate_catalog(&state.db, job, &target.service.id)
            .await?;
        return Ok(Decision {
            admission: None,
            automatic: None,
            refusal: None,
        });
    };
    let owner = owner.ok_or(AppError::MachinePermissionRevoked)?;
    let auth = super::assistant_job_service_use::validate(&state.db, job, id, owner).await?;
    let chat = auth
        .assistant_chat
        .as_deref()
        .ok_or(AppError::MachinePermissionRevoked)?;
    verify_resolved_projected(
        &state.db,
        &state.encryption_keys,
        chat,
        id,
        target,
        operation.0,
        operation.1,
        projection,
    )
    .await?;
    Ok(Decision {
        refusal: None,
        automatic: None,
        admission: Some(Admission {
            shared_permit: None,
            connection: id.to_owned(),
            binding: connection_binding(&state.db, id, &chat.api_key_id).await?,
            execution: Some(execution_digest(target)),
            execution_credential: true,
            operation: digest(json!({"job_id":job})),
            operation_identity: json!({"job_id":job}),
            native_execution: None,
            read: false,
            api_key_id: chat.api_key_id.clone(),
            summary: "Machine job service use".into(),
            job_id: Some(job.to_owned()),
            async_continuation: None,
            chat: chat.clone(),
        }),
    })
}

/// Run after ACL/operation checks and concrete resolution, before any egress.
/// Exact calls are never rewritten. A refusal names the preferred exact proposal.
#[allow(clippy::too_many_arguments)]
pub fn check<'a>(
    db: &'a Database,
    nodes: &'a super::node_ws_manager::NodeWsManager,
    chat: &'a acks::ChatAuthority,
    connection_id: &'a str,
    effective_owner: &'a str,
    operation: &'a Value,
    facts: acks::ActionFacts,
    acknowledgement: Option<&'a str>,
    summary: &'a str,
    pool: bool,
    target: Option<&'a super::proxy_service::ProxyTarget>,
    model_consent: bool,
    semantic_input: Option<(&'a [u8], Option<&'a str>)>,
) -> futures::future::BoxFuture<'a, AppResult<Decision>> {
    Box::pin(check_inner(
        db,
        nodes,
        chat,
        connection_id,
        effective_owner,
        operation,
        facts,
        acknowledgement,
        summary,
        pool,
        target,
        model_consent,
        semantic_input,
        false,
    ))
}

/// Source replay is a new action, so it never borrows the initial read's
/// implicit continuation. An explicit resource card still requires exact
/// consumption and live authority, including the current access mode.
#[allow(clippy::too_many_arguments)]
pub(crate) fn check_source_resource<'a>(
    db: &'a Database,
    nodes: &'a super::node_ws_manager::NodeWsManager,
    chat: &'a acks::ChatAuthority,
    connection_id: &'a str,
    effective_owner: &'a str,
    operation: &'a Value,
    facts: acks::ActionFacts,
    acknowledgement: Option<&'a str>,
    summary: &'a str,
    target: &'a super::proxy_service::ProxyTarget,
    semantic_input: Option<(&'a [u8], Option<&'a str>)>,
) -> futures::future::BoxFuture<'a, AppResult<Decision>> {
    Box::pin(check_inner(
        db,
        nodes,
        chat,
        connection_id,
        effective_owner,
        operation,
        facts,
        acknowledgement,
        summary,
        false,
        Some(target),
        false,
        semantic_input,
        true,
    ))
}

// Service-choice temporaries need not occupy the stack while confirmation
// polls its nested MongoDB transaction. Preserve same-task drop semantics.
fn box_confirmation<'a, F>(
    make: impl FnOnce() -> F,
) -> futures::future::BoxFuture<'a, AppResult<Decision>>
where
    F: std::future::Future<Output = AppResult<Decision>> + Send + 'a,
{
    Box::pin(make())
}

#[allow(clippy::too_many_arguments)]
async fn check_inner(
    db: &Database,
    nodes: &super::node_ws_manager::NodeWsManager,
    chat: &acks::ChatAuthority,
    connection_id: &str,
    effective_owner: &str,
    operation: &Value,
    facts: acks::ActionFacts,
    acknowledgement: Option<&str>,
    summary: &str,
    pool: bool,
    target: Option<&super::proxy_service::ProxyTarget>,
    model_consent: bool,
    semantic_input: Option<(&[u8], Option<&str>)>,
    resource_replay: bool,
) -> AppResult<Decision> {
    chat.ensure_live_turn(db, None, None, "assistant.service_execution")
        .await?;
    let service =
        super::user_service_service::get_user_service(db, effective_owner, connection_id).await?;
    if let Some(access) = chat.org_agent_access.as_ref().and_then(|a| a.shared()) {
        if !chat.turn_live || chat.turn_stopped {
            return Err(AppError::AssistantTurnRequired);
        }
        let model = matches!(
            operation["kind"].as_str(),
            Some("raw_inference" | "legacy_inference")
        ) && access.allows_model_connection(&service);
        if pool || !(access.allows_connection(&service) || model) {
            return Err(AppError::Forbidden(
                "Shared connection is unavailable".into(),
            ));
        }
        if let Some(selected) = selection::selected(db, chat).await?
            && let Ok(selected_id) = selected.get_str("connection_id")
            && selected_id != connection_id
            && service.catalog_service_id.is_some()
            && (selected.get_str("catalog_service_id").ok()
                == service.catalog_service_id.as_deref()
                || selected
                    .get_document("operation")
                    .ok()
                    .and_then(|op| op.get_str("provider").ok())
                    == service.catalog_service_id.as_deref())
        {
            return Err(AppError::Forbidden("Use the person's exact selected connection; choose a new connection before changing accounts".into()));
        }
        let binding = connection_binding(db, connection_id, &chat.api_key_id).await?;
        if service.user_id == chat.user_id
            && let Some(refusal) =
                super::shared_agent_consent::prompt(db, chat, access, &service, &binding, operation)
                    .await?
        {
            return Ok(Decision {
                refusal: Some(refusal),
                admission: None,
                automatic: None,
            });
        }
        // Connection consent is independent of the actor's action policy.
        // Always may remember a connection; it never bypasses an action card.
        let gate = acks::confirmation_needed(db, chat, facts, false).await?;
        if let Some(need) = gate.ask() {
            let args = json!({"connection_id": connection_id, "execution_binding": binding,
                "operation": operation, "acknowledgement_id": acknowledgement});
            if let Some(refusal) =
                acks::confirm_action(db, chat, "nyxid.service_execution", &args, &need, summary)
                    .await?
            {
                return Ok(Decision {
                    refusal: Some(refusal),
                    admission: None,
                    automatic: None,
                });
            }
        }
        let permit = if service.user_id == chat.user_id {
            match super::shared_agent_consent::authorize(
                db, chat, access, &service, &binding, operation,
            )
            .await?
            {
                Ok(permit) => Some(permit),
                Err(refusal) => {
                    return Ok(Decision {
                        refusal: Some(refusal),
                        admission: None,
                        automatic: None,
                    });
                }
            }
        } else {
            None
        };
        return Ok(Decision {
            refusal: None,
            automatic: gate.auto_allowed(),
            admission: Some(Admission {
                shared_permit: permit,
                connection: connection_id.into(),
                binding,
                execution: target.map(execution_digest),
                execution_credential: true,
                operation: digest(operation.clone()),
                operation_identity: operation.clone(),
                native_execution: None,
                read: facts.class == acks::OwnerActionClass::Read,
                api_key_id: chat.api_key_id.clone(),
                summary: summary.into(),
                job_id: None,
                async_continuation: None,
                chat: chat.clone(),
            }),
        });
    }
    let owner_row = db
        .collection::<crate::models::user::User>("users")
        .find_one(doc! {"_id":effective_owner,"is_active":true})
        .await?
        .ok_or_else(|| AppError::Forbidden("Service owner unavailable".into()))?;
    let org = owner_row.user_type == crate::models::user::UserType::Org;
    if org {
        scope(chat)?;
    }
    let access =
        super::org_service::resolve_owner_access(db, &chat.user_id, effective_owner).await?;
    let eligible = match &access {
        super::org_service::OwnerAccess::Direct
        | super::org_service::OwnerAccess::AsOrgAdmin { .. } => true,
        super::org_service::OwnerAccess::AsOrgMember { role, .. } => {
            role.can_proxy() && !service.admin_only
        }
        super::org_service::OwnerAccess::Forbidden => false,
    };
    if !eligible || !access.allows_resource(connection_id) {
        return Err(AppError::Forbidden("Service access revoked".into()));
    }
    let binding = connection_binding(db, connection_id, &chat.api_key_id).await?;
    if pool
        && let Some(failure) =
            availability::pool_predispatch_failure(db, chat, connection_id, &binding, operation)
                .await?
    {
        return Err(AppError::PoolAttemptTransport(failure));
    }
    let pool_members: Vec<String> = if pool {
        if let Some(id) = operation["pool_id"].as_str() {
            db.collection::<crate::models::service_pool::ServicePool>("service_pools")
                .find_one(doc! {"_id": id, "is_active": true})
                .await?
                .map(|pool| {
                    pool.members
                        .into_iter()
                        .map(|member| member.user_service_id)
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    let mut args = json!({"connection_id": connection_id, "effective_owner_id": effective_owner,
        "execution_binding": binding, "operation": operation});
    let mut continuation = None;
    if acknowledgement.is_none() && !model_consent && !resource_replay {
        // Only a server-validated owner continuation may present a card implicitly
        // (model bootstrap has no opportunity to add an acknowledgement header).
        if let Some(turn) = chat.turn_id.as_deref() {
            continuation = db
                .collection::<Document>("assistant_messages")
                .find_one(
                    doc! {"user_id": &chat.user_id, "conversation_id": &chat.conversation_id,
                    "turn_id": turn, "role": "user", "continuation_of": {"$type": "string"}},
                )
                .await?
                .and_then(|row| row.get_str("continuation_of").ok().map(str::to_owned));
        }
    }
    // A continuation carries authority for only its card's exact operation.
    // Subsequent tool/model rounds must not implicitly replay a used card.
    if let Some(id) = continuation.as_deref() {
        let bound = db.collection::<Document>(COLLECTION).find_one(doc! {
            "_id": id, "kind": "card_binding", "user_id": &chat.user_id,
            "conversation_id": &chat.conversation_id, "api_key_id": &chat.api_key_id,
            "connection_id": connection_id, "binding": &binding,
            "operation": bson::to_bson(operation).map_err(|_| AppError::Internal("Operation binding failed".into()))?,
            "expires_at": {"$gt": bson::DateTime::now()},
        }).await?;
        if bound.is_none() {
            continuation = None;
        }
    }
    let acknowledgement = acknowledgement.or(continuation.as_deref());
    if let Some(id) = acknowledgement {
        args["acknowledgement_id"] = json!(id);
    }
    let selected = selection::selected(db, chat).await?;
    if let Some(selected) = selected.as_ref()
        && let Ok(selected_id) = selected.get_str("connection_id")
    {
        let selected_service =
            super::user_service_service::find_user_service_by_id(db, selected_id).await?;
        let catalog = selected
            .get_str("catalog_service_id")
            .ok()
            .or_else(|| {
                selected
                    .get_document("operation")
                    .ok()
                    .and_then(|op| op.get_str("provider").ok())
            })
            .or_else(|| {
                selected_service
                    .as_ref()
                    .and_then(|row| row.catalog_service_id.as_deref())
            });
        let same_group = preferences::group_key(selected_id, catalog)
            == preferences::group_key(connection_id, service.catalog_service_id.as_deref());
        let applies = selected
            .get("operation")
            .is_none_or(|op| bson::to_bson(operation).is_ok_and(|current| current == *op));
        let pool_fallback = pool
            && selected.get_str("selection_source") != Ok("owner")
            && operation["pool_id"].as_str().is_some_and(|pool| {
                selected
                    .get_document("operation")
                    .ok()
                    .and_then(|op| op.get_str("pool_id").ok())
                    == Some(pool)
            })
            && if let Some(row) = selected_service.as_ref() {
                verified_unavailable(db, nodes, chat, row, operation).await?
            } else {
                false
            };
        if (same_group || selected_id == connection_id)
            && !pool_fallback
            && applies
            && (selected_id != connection_id || !selection_matches(selected, &service, &binding))
        {
            let (reason, instructions) = match selected_service.as_ref() {
                None => (
                    "selected_connection_unavailable",
                    "The owner already selected this connection, but it is now removed, disabled or inaccessible. Enable or reconnect it in Connections, or restore access. Ask the owner for a replacement only if they want a different account; do not substitute one.",
                ),
                Some(_) if selected_id == connection_id => (
                    "selected_connection_changed",
                    "The selected connection's identity or confirmed operation binding changed. Ask the owner to review the changed connection before selecting it again. Do not use another account.",
                ),
                Some(_) => (
                    "wrong_connection",
                    "The owner already selected an exact connection. Use nyx__search_tools with query set to selected_connection_id to find that connection's tool, preserving the operation and arguments. If it cannot be addressed unambiguously, report that limitation. Do not ask the owner to repeat their selection or use another account.",
                ),
            };
            return Ok(Decision {
                refusal: Some(json!({"error":"service_selection_required",
                    "reason":reason,"selected_connection_id":selected_id,"instructions":instructions})),
                admission: None,
                automatic: None,
            });
        }
    }
    let explicit = selected.as_ref().is_some_and(|row| {
        row.get_str("connection_id") == Ok(connection_id)
            && selection_matches(row, &service, &binding)
            && row
                .get("operation")
                .is_none_or(|bound| bson::to_bson(operation).is_ok_and(|current| current == *bound))
    });

    if !explicit && service.catalog_service_id.is_some() {
        let key = db
            .collection::<ApiKey>("api_keys")
            .find_one(doc! {"_id": &chat.api_key_id, "is_active": true})
            .await?
            .ok_or_else(|| AppError::Forbidden("Assistant credential unavailable".into()))?;
        let request_access = super::org_agent_service::resolve_credential_access(db, &key).await?;
        let allowed_services = super::key_service::effective_allowed_service_ids_with_access(
            db,
            &key,
            request_access.as_deref(),
        )
        .await?;
        let (allow_all_nodes, allowed_nodes) =
            super::org_agent_service::key_nodes(db, &key, request_access.as_deref()).await?;
        let memberships =
            super::org_service::list_memberships_for_member(db, &chat.user_id, false).await?;
        let rows = super::user_service_service::list_user_services_with_sources_selected_in_group(
            db,
            &chat.user_id,
            false,
            false,
            &memberships,
            super::user_service_service::ServiceSelection {
                ids: None,
                catalog_service_id: service.catalog_service_id.as_deref(),
            },
        )
        .await?;
        let order = preferences::get(db, &chat.user_id)
            .await?
            .map(|row| row.ordered)
            .unwrap_or_default();
        let positions: HashMap<_, _> = order
            .iter()
            .enumerate()
            .map(|(i, id)| (id.as_str(), i))
            .collect();
        let inference = matches!(
            operation["kind"].as_str(),
            Some("inference" | "raw_inference")
        );
        let mut candidates: Vec<_> = rows
            .into_iter()
            .filter(|row| {
                !matches!(
                    row.source,
                    super::user_service_service::CredentialSource::Org { allowed: false, .. }
                ) && (chat.turn_live || row.service.user_id == chat.user_id)
                    && (key.allow_all_services
                        || allowed_services.contains(&row.service.id)
                        || matches!(
                            operation["kind"].as_str(),
                            Some("raw_inference" | "legacy_inference")
                        ))
                    && (allow_all_nodes
                        || row
                            .service
                            .node_id
                            .as_ref()
                            .is_none_or(|id| allowed_nodes.contains(id)))
                    && row.service.catalog_service_id == service.catalog_service_id
            })
            .map(|row| row.service)
            .collect();
        candidates.retain(|candidate| {
            candidate.id == connection_id || !pool_members.contains(&candidate.id)
        });
        candidates.sort_by_key(|row| {
            (
                row.user_id != chat.user_id,
                positions
                    .get(row.id.as_str())
                    .copied()
                    .unwrap_or(usize::MAX),
            )
        });
        for candidate in candidates {
            if candidate.id == connection_id {
                break;
            }
            // Eligibility is independent of availability. A connection denied
            // this operation cannot block a permitted lower-ranked connection.
            if let (Some(method), Some(path)) =
                (operation["method"].as_str(), operation["path"].as_str())
            {
                let path = super::proxy_authorization::CanonicalPath::from_mcp_built(path)?;
                let scopes = &key.assistant_operation_scopes;
                if super::agent_operation_scope_service::authorize(
                    scopes,
                    &candidate.id,
                    candidate.catalog_service_id.as_deref(),
                    operation["endpoint_id"].as_str(),
                    method,
                    &path,
                    false,
                    false,
                )
                .is_err()
                {
                    continue;
                }
                if let Some((body, content_type)) = semantic_input {
                    if super::agent_operation_scope_service::check_inputs(
                        scopes,
                        &candidate.id,
                        candidate.catalog_service_id.as_deref(),
                        operation["endpoint_id"].as_str(),
                        method,
                        &path,
                        operation["query"].as_str(),
                        body,
                        content_type,
                    )
                    .is_err()
                    {
                        continue;
                    }
                } else if super::agent_operation_scope_service::applicable(
                    scopes,
                    &candidate.id,
                    candidate.catalog_service_id.as_deref(),
                )
                .any(|scope| scope.operations.iter().any(|op| op.inputs.is_some()))
                {
                    // Without semantic inputs an input-limited alternative is
                    // unproven, never an excuse to widen its grant.
                    continue;
                }
            }
            if verified_unavailable(db, nodes, chat, &candidate, operation).await? {
                continue;
            }
            if !equivalent(&service, &candidate, inference) {
                return Ok(Decision {
                    refusal: Some(json!({"error": "service_target_selection_required",
                    "proposed_connection_id": connection_id, "instructions": "These connections may access different accounts. Ask the owner to start a new message with /service <connection UUID>, followed by their request on the next line. Do not replace an account-specific call with another connection."})),
                    admission: None,
                    automatic: None,
                });
            }
            let endpoint = super::service_history::collection::<UserEndpoint>(db, "user_endpoints")
                .find_one(doc! {"_id": &candidate.endpoint_id, "user_id": &candidate.user_id})
                .await?;
            let proposed = super::service_history::collection::<UserEndpoint>(db, "user_endpoints")
                .find_one(doc! {"_id": &service.endpoint_id, "user_id": &service.user_id})
                .await?;
            if !endpoint
                .zip(proposed)
                .is_some_and(|(a, b)| a.url == b.url && a.openapi_spec_url == b.openapi_spec_url)
            {
                continue;
            }

            return Ok(Decision {
                refusal: Some(json!({"error": "service_priority_required",
                "preferred_connection_id": candidate.id, "preferred_connection_slug": candidate.slug,
                "instructions": "Propose this exact connection for the same operation. Do not change the requested account, resource, provider or model. The owner can select another connection for one turn with /service <connection UUID>. Policy refusal, declined consent, invalid arguments, missing resources and empty results are not outages."})),
                admission: None,
                automatic: None,
            });
        }
    }
    if chat.guest {
        return Ok(Decision {
            refusal: None,
            admission: Some(Admission {
                shared_permit: None,
                connection: connection_id.into(),
                binding,
                execution: target.map(|target| execution_projection_digest(target, !pool)),
                execution_credential: !pool,
                operation: digest(operation.clone()),
                operation_identity: operation.clone(),
                native_execution: None,
                read: facts.class == acks::OwnerActionClass::Read,
                api_key_id: chat.api_key_id.clone(),
                summary: summary.to_owned(),
                job_id: None,
                async_continuation: None,
                chat: chat.clone(),
            }),
            automatic: None,
        });
    }
    box_confirmation(move || async move {
        let summary = if org {
            let owner = db
                .collection::<crate::models::user::User>("users")
                .find_one(doc! {"_id": effective_owner, "is_active": true})
                .await?
                .ok_or_else(|| AppError::Forbidden("Organization unavailable".into()))?;
            format!(
                "Organization connection {} ({}) in {}: {}",
                super::assistant_nyxagent::identifier(&service.slug),
                connection_id,
                super::assistant_nyxagent::identifier(
                    owner
                        .display_name
                        .as_deref()
                        .unwrap_or(owner.slug.as_deref().unwrap_or(&owner.id))
                ),
                summary
            )
        } else {
            summary.to_owned()
        };
        let gate = if resource_replay && acknowledgement.is_some() {
            acks::Gate::Ask(acks::ConfirmationNeed { class: facts.class, mode: None, webhook: false, policy: true, delegation: false })
        } else { acks::confirmation_needed(db, chat, facts, org && !model_consent).await? };
        if gate.ask().is_some() {
            // Legacy idle calls can retain their existing automatic behavior, but
            // cannot mint or redeem turn-bound consent without an admitted turn.
            scope(chat)?;
        }
        if let Some(need) = gate.ask()
            && let Some(refusal) =
                acks::confirm_action(db, chat, "nyxid.service_execution", &args, &need, &summary)
                    .await?
        {
            if refusal["error"] == "acknowledgement_required"
                && let Some(id) = refusal["acknowledgement_id"].as_str()
            {
                db.collection::<Document>(COLLECTION).update_one(doc! {"_id": id}, doc! {"$setOnInsert": {
                            "kind": "card_binding", "user_id": &chat.user_id, "conversation_id": &chat.conversation_id,
                            "api_key_id": &chat.api_key_id, "turn_id": &chat.turn_id, "connection_id": connection_id,
                            "owner_id": effective_owner, "catalog_service_id": &service.catalog_service_id,
                            "binding": &binding, "operation": bson::to_bson(operation).map_err(|_| AppError::Internal("Operation binding failed".into()))?,
                            "expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::minutes(25)),
                        }}).upsert(true).await?;
            }
            return Ok(Decision {
                refusal: Some(refusal),
                admission: None,
                automatic: None,
            });
        }
        Ok(Decision {
            refusal: None,
            admission: Some(Admission {
                shared_permit: None,
                connection: connection_id.into(),
                binding,
                execution: target.map(|target| execution_projection_digest(target, !pool)),
                execution_credential: !pool,
                operation: digest(operation.clone()),
                operation_identity: operation.clone(),
                native_execution: None,
                read: facts.class == acks::OwnerActionClass::Read,
                api_key_id: chat.api_key_id.clone(),
                summary: summary.to_owned(),
                job_id: None,
                async_continuation: None,
                chat: chat.clone(),
            }),
            automatic: gate.auto_allowed(),
        })
    })
    .await
}

fn execution_digest(target: &super::proxy_service::ProxyTarget) -> String {
    execution_projection_digest(target, true)
}

/// Held execution projections never leave this process. A random process key
/// binds credential equality without producing a reusable unkeyed fingerprint.
/// Persisted consent and connection bindings use their separate versioned inputs.
fn held_credential_fingerprint(credential: &[u8]) -> String {
    use hmac::{Hmac, Mac};
    use rand::{RngCore, rngs::OsRng};
    use std::sync::LazyLock;
    use zeroize::Zeroizing;

    static KEY: LazyLock<Zeroizing<[u8; 32]>> = LazyLock::new(|| {
        let mut key = Zeroizing::new([0; 32]);
        OsRng.fill_bytes(key.as_mut());
        key
    });
    let mut mac = Hmac::<sha2::Sha256>::new_from_slice(KEY.as_ref())
        .expect("HMAC-SHA256 accepts a 32-byte key");
    mac.update(b"nyxid:assistant-held-execution-credential:v1\0");
    mac.update(credential);
    hex::encode(mac.finalize().into_bytes())
}

fn execution_projection_digest(
    target: &super::proxy_service::ProxyTarget,
    credential: bool,
) -> String {
    digest(
        json!({"url": target.base_url, "target": target.target_id, "auth_method": target.auth_method,
        "auth_key_name": target.auth_key_name, "auth_scheme": target.service.auth_scheme,
        "custom_user_agent": target.service.custom_user_agent, "category": target.service.service_category,
        "requires_user_credential": target.service.requires_user_credential, "credential": credential.then(|| held_credential_fingerprint(target.credential.as_bytes())),
        "catalog_headers": target.catalog_default_headers, "connection_headers": target.user_service_default_headers,
        "identity": [target.service.identity_propagation_mode.clone(), target.service.delegation_token_scope.clone()],
        "identity_fields": [target.service.identity_include_user_id, target.service.identity_include_email, target.service.identity_include_name,
            target.service.forward_access_token, target.service.inject_delegation_token],
        "audience": target.service.identity_jwt_audience, "policy": target.service.proxy_operation_policy,
        "exchange": target.service.token_exchange_config, "ws": target.ws_frame_injections}),
    )
}

#[cfg(test)]
#[path = "assistant_service_execution_tests.rs"]
mod tests;
