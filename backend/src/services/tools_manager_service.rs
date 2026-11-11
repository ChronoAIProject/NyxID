//! Database fences for limited Tool authority and first credential installation.
use crate::{
    errors::{AppError, AppResult},
    models::{downstream_service::DownstreamService, group::Group, role::Role, user::User},
};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{Document, doc},
};

/// Write every authority-bearing row in the same transaction as the mutation.
/// Revocation, group membership changes and role edits therefore conflict with
/// in-flight writes rather than permitting a stale authorization snapshot.
pub async fn fence_authority(
    db: &Database,
    session: &mut ClientSession,
    actor: &str,
) -> AppResult<bool> {
    fence_role_authority(
        db,
        session,
        actor,
        Some(crate::models::role::TOOLS_WRITE_PERMISSION),
    )
    .await
}

pub(crate) async fn fence_role_authority(
    db: &Database,
    session: &mut ClientSession,
    actor: &str,
    permission: Option<&str>,
) -> AppResult<bool> {
    role_authority(db, session, actor, permission, true).await
}

pub(crate) async fn role_authority(
    db: &Database,
    session: &mut ClientSession,
    actor: &str,
    permission: Option<&str>,
    write_fence: bool,
) -> AppResult<bool> {
    let user = db
        .collection::<User>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": actor, "is_active": true})
        .session(&mut *session)
        .await?
        .ok_or_else(|| AppError::Forbidden("Tool authority revoked".into()))?;
    let groups: Vec<Group> = db
        .collection::<Group>(crate::models::group::COLLECTION_NAME)
        .find(doc! {"_id": {"$in": &user.group_ids}})
        .session(&mut *session)
        .await?
        .stream(&mut *session)
        .try_collect()
        .await?;
    let mut ids = user.role_ids.clone();
    for g in &groups {
        ids.extend(g.role_ids.clone());
    }
    let roles: Vec<Role> = db
        .collection::<Role>(crate::models::role::COLLECTION_NAME)
        .find(doc! {"_id": {"$in": &ids}})
        .session(&mut *session)
        .await?
        .stream(&mut *session)
        .try_collect()
        .await?;
    let admin = roles.iter().any(|r| {
        r.is_system && r.client_id.is_none() && r.slug == "admin" && user.role_ids.contains(&r.id)
    });
    let operator = roles.iter().any(|r| {
        r.is_system
            && r.client_id.is_none()
            && r.slug == "operator"
            && user.role_ids.contains(&r.id)
    });
    if !admin
        && permission.is_some_and(|permission| {
            !operator
                || !roles
                    .iter()
                    .any(|r| r.permissions.iter().any(|p| p == permission))
        })
    {
        return Err(AppError::Forbidden("Tool authority revoked".into()));
    }
    if !write_fence {
        return Ok(admin);
    }
    // System roles are immutable. The user row binds direct system-role
    // assignments; only permission-granting group paths and mutable roles need
    // additional fences. A group granting a system permission still needs a
    // fence because its role_ids can change.
    let permission_roles: Vec<_> = roles
        .iter()
        .filter(|role| {
            !admin && permission.is_some_and(|p| role.permissions.iter().any(|value| value == p))
        })
        .collect();
    for (collection, id) in
        std::iter::once((crate::models::user::COLLECTION_NAME, user.id.as_str()))
            .chain(
                groups
                    .iter()
                    .filter(|group| {
                        permission_roles
                            .iter()
                            .any(|role| group.role_ids.contains(&role.id))
                    })
                    .map(|group| (crate::models::group::COLLECTION_NAME, group.id.as_str())),
            )
            .chain(
                permission_roles
                    .iter()
                    .filter(|role| !role.is_system)
                    .map(|role| (crate::models::role::COLLECTION_NAME, role.id.as_str())),
            )
    {
        db.collection::<Document>(collection)
            .update_one(doc! {"_id":id}, doc! {"$inc":{"authority_fence":1_i64}})
            .session(&mut *session)
            .await?;
    }
    Ok(admin)
}

#[cfg(test)]
pub async fn commit_manager_update(
    db: &Database,
    observed: &DownstreamService,
    _actor: &str,
    fields: &Document,
    audit_key: &[u8],
    audit_actor: &super::audit_service::AuditActor,
) -> AppResult<DownstreamService> {
    super::catalog_mutation_service::commit_update(
        db,
        observed,
        &super::catalog_mutation_service::Authority {
            actor: audit_actor.clone(),
            first_party_human: true,
            admin_required: false,
            catalog_base_aliases: Vec::new(),
            catalog_base_url: "http://localhost:3001".into(),
        },
        fields,
        &doc! {"_id":&observed.id},
        audit_key,
    )
    .await
}

/// Compare resolved execution/contract metadata, preserving publication on no-ops.
pub fn tool_contract_changed(service: &DownstreamService, fields: &Document) -> AppResult<bool> {
    let current =
        mongodb::bson::to_document(service).map_err(|e| AppError::Internal(e.to_string()))?;
    Ok([
        "anonymous_endpoints",
        "base_url",
        "openapi_spec_url",
        "asyncapi_spec_url",
        "service_type",
        "auth_method",
        "auth_scheme",
        "auth_key_name",
        "destination_targets",
        "token_exchange_config",
        "identity_propagation_mode",
        "forward_access_token",
        "inject_delegation_token",
        "default_request_headers",
        "ws_frame_injections",
        "custom_user_agent",
    ]
    .iter()
    .any(|k| fields.get(*k).is_some_and(|v| current.get(*k) != Some(v))))
}

pub fn eligible_manager_tool(s: &DownstreamService) -> bool {
    s.offering_kind == crate::models::downstream_service::OfferingKind::Tool
        && s.service_type == "http"
        && s.service_category == "internal"
        && !s.requires_user_credential
        && s.provider_config_id.is_none()
        && s.destination_targets.is_empty()
        && s.identity_propagation_mode == "none"
        && !s.forward_access_token
        && !s.inject_delegation_token
        && s.token_exchange_config.is_none()
        && s.default_request_headers.is_none()
        && s.ws_frame_injections.is_empty()
        && s.anonymous_endpoints.is_empty()
}

pub async fn eligible_manager_tool_in_db(
    db: &Database,
    service: &DownstreamService,
    session: Option<&mut ClientSession>,
) -> AppResult<bool> {
    if !eligible_manager_tool(service) {
        return Ok(false);
    }
    let coll =
        db.collection::<Document>(crate::models::service_provider_requirement::COLLECTION_NAME);
    let query = coll
        .find_one(doc! {"service_id": &service.id})
        .projection(doc! {"_id":1});
    let row = if let Some(session) = session {
        query.session(session).await?
    } else {
        query.await?
    };
    Ok(row.is_none())
}

/// List responses share one requirements lookup; the shape predicate stays the
/// same as single-row and commit-time eligibility.
pub async fn eligible_manager_tool_ids(
    db: &Database,
    services: &[DownstreamService],
) -> AppResult<std::collections::HashSet<String>> {
    let mut eligible: std::collections::HashSet<String> = services
        .iter()
        .filter(|service| eligible_manager_tool(service))
        .map(|service| service.id.clone())
        .collect();
    if eligible.is_empty() {
        return Ok(eligible);
    }
    let ids: Vec<&str> = eligible.iter().map(String::as_str).collect();
    let rows: Vec<Document> = db
        .collection::<Document>(crate::models::service_provider_requirement::COLLECTION_NAME)
        .find(doc! {"service_id":{"$in":ids}})
        .projection(doc! {"service_id":1,"_id":0})
        .await?
        .try_collect()
        .await?;
    for row in rows {
        let id = row.get_str("service_id").map_err(|_| {
            AppError::Internal("Provider requirement missing service identity".into())
        })?;
        eligible.remove(id);
    }
    Ok(eligible)
}
