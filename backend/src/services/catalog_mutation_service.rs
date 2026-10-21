//! Commit-time catalog authority shared by service and operation mutations.
use super::{audit_service::AuditActor, tools_manager_service};
use crate::{
    errors::{AppError, AppResult},
    models::downstream_service::{COLLECTION_NAME, DownstreamService, OfferingKind},
};
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};

#[derive(Clone)]
pub struct Authority {
    pub catalog_base_url: String,
    pub actor: AuditActor,
    /// Captured from the authenticated request, never inferred from a user row.
    pub first_party_human: bool,
    /// Bind privileged ingress validation to a live Admin role at commit.
    pub admin_required: bool,
}

pub async fn fence(
    db: &Database,
    session: &mut ClientSession,
    observed: &DownstreamService,
    authority: Option<&Authority>,
) -> AppResult<(DownstreamService, bool)> {
    check_parent(db, session, observed, authority, true).await
}

pub async fn validate_parent(
    db: &Database,
    session: &mut ClientSession,
    observed: &DownstreamService,
    authority: Option<&Authority>,
) -> AppResult<(DownstreamService, bool)> {
    check_parent(db, session, observed, authority, false).await
}

async fn check_parent(
    db: &Database,
    session: &mut ClientSession,
    observed: &DownstreamService,
    authority: Option<&Authority>,
    write_fence: bool,
) -> AppResult<(DownstreamService, bool)> {
    let live = db
        .collection::<DownstreamService>(COLLECTION_NAME)
        .find_one(doc! {"_id": &observed.id})
        .session(&mut *session)
        .await?
        .ok_or_else(|| {
            AppError::Conflict("Service no longer exists; reload before retrying".into())
        })?;
    if authority_projection(&live)? != authority_projection(observed)? {
        return Err(AppError::Conflict(
            "Service authority or contract changed; reload before retrying".into(),
        ));
    }
    let tool = live.offering_kind == OfferingKind::Tool;
    let mut admin = false;
    if let Some(authority) = authority {
        // Foreign shared rows and public manager-created rows require a live
        // Catalog permission. Legacy creators retain their existing fallback.
        let catalog = (live.catalog_manager_created && live.visibility != "private")
            || (is_shared_catalog_row(&live) && live.created_by != authority.actor.user_id);
        if (tool || catalog) && !authority.first_party_human {
            return Err(AppError::Forbidden(
                "Catalog mutation requires first-party human authority".into(),
            ));
        }
        let permission = if tool {
            Some(crate::models::role::TOOLS_WRITE_PERMISSION)
        } else if catalog {
            Some(crate::models::role::CATALOG_SERVICES_WRITE_PERMISSION)
        } else {
            None
        };
        admin = tools_manager_service::role_authority(
            db,
            session,
            &authority.actor.user_id,
            permission,
            write_fence,
        )
        .await?;
        if authority.admin_required && !admin {
            return Err(AppError::Forbidden("Admin authority revoked".into()));
        }
        if !admin {
            let allowed = if tool {
                live.tools_manager_created
                    && live.created_by == authority.actor.user_id
                    && live
                        .owner_user_id
                        .as_ref()
                        .is_none_or(|id| id == &authority.actor.user_id)
                    && tools_manager_service::eligible_manager_tool_in_db(
                        db,
                        &live,
                        Some(&mut *session),
                    )
                    .await?
            } else {
                live.owner_user_id.is_none()
                    && (live.created_by == authority.actor.user_id || is_shared_catalog_row(&live))
            };
            if !allowed {
                return Err(AppError::Forbidden(
                    "Service write authority changed".into(),
                ));
            }
        }
    }
    if write_fence {
        db.collection::<Document>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &live.id},
                doc! {"$inc": {"authority_fence": 1_i64}},
            )
            .session(&mut *session)
            .await?;
    }
    Ok((live, admin))
}

/// Catalog Managers can complete seeded and shared connectors. Tools keep
/// their separate authority, even when otherwise publicly visible.
pub fn is_shared_catalog_row(service: &DownstreamService) -> bool {
    service.owner_user_id.is_none()
        && service.visibility == "public"
        && service.service_type == "http"
        && service.service_category != "internal"
        && service.offering_kind != OfferingKind::Tool
}

fn authority_projection(service: &DownstreamService) -> AppResult<Document> {
    let mut row = bson::to_document(service).map_err(|e| AppError::Internal(e.to_string()))?;
    let mut projection = Document::new();
    for key in [
        "offering_kind",
        "owner_user_id",
        "created_by",
        "catalog_manager_created",
        "tools_manager_created",
        "visibility",
        "is_active",
        "provider_config_id",
        "service_type",
        "service_category",
        "base_url",
        "auth_method",
        "auth_scheme",
        "auth_key_name",
        "oauth_client_id",
        "credential_encrypted",
        "secret_context_versions",
        "platform_key",
        "tools_credential_frozen",
        "destination_targets",
        "identity_propagation_mode",
        "forward_access_token",
        "inject_delegation_token",
        "token_exchange_config",
        "default_request_headers",
        "ws_frame_injections",
        "anonymous_endpoints",
        "requires_user_credential",
    ] {
        if let Some(value) = row.remove(key) {
            projection.insert(key, value);
        }
    }
    Ok(projection)
}

pub async fn commit_update(
    db: &Database,
    observed: &DownstreamService,
    authority: &Authority,
    fields: &Document,
    filter: &Document,
    audit_key: &[u8],
) -> AppResult<DownstreamService> {
    let db = db.clone();
    let observed = observed.clone();
    let authority = authority.clone();
    let fields = fields.clone();
    let filter = filter.clone();
    let key = zeroize::Zeroizing::new(audit_key.to_vec());
    let context = (&db, &observed, &authority, &fields, &filter, &key);
    super::transaction_service::run_db_only(&db, &context, |session, context| {
        let &(db, observed, authority, fields, filter, key) = context;
        Box::pin(async move {
            let (live, admin) = fence(db, session, observed, Some(authority)).await?;
            let tool = live.offering_kind == OfferingKind::Tool;
            require_desired_kind(&live, fields, authority, admin)?;
            validate_update_fields(db, session, &live, fields, authority, admin).await?;
            if fields.get_str("offering_kind") == Ok("tool") {
                require_providerless_tool(db, session, &live).await?;
            }
            if tool && (fields.contains_key("credential_encrypted") || fields.contains_key("base_url")) && live.updated_at != observed.updated_at {
                return Err(AppError::Conflict("Tool credential or destination revision changed".into()));
            }
            if tool && !admin && (fields.contains_key("credential_encrypted") || fields.contains_key("base_url"))
                && (live.tools_credential_frozen || !live.credential_encrypted.is_empty()) {
                return Err(AppError::Forbidden("Tool credential and destination are frozen; ask an Admin".into()));
            }
            if let Ok(url) = fields.get_str("icon_url") {
                let id = super::catalog_icon_service::asset_id(&authority.catalog_base_url, url)?;
                super::catalog_icon_service::require_asset(db, id, Some(&mut *session)).await?;
            }
            if let Some(rules) = fields.get("anonymous_endpoints") {
                let rules: Vec<crate::models::downstream_service::AnonymousEndpointRule> = bson::from_bson(rules.clone()).map_err(|e| AppError::Internal(format!("Invalid anonymous rules: {e}")))?;
                super::anonymous_endpoint_service::validate_rules_for_service(&live, &rules)?;
            }
            if tool && (tools_manager_service::tool_contract_changed(&live, fields)?
                || fields.get_str("offering_kind").is_ok_and(|kind| kind != "tool")) {
                super::service_endpoint_service::demote_tool_in_session(db, session, &live.id, None, Some((&authority.actor, key))).await?;
            }
            let updated = db.collection::<DownstreamService>(COLLECTION_NAME).find_one_and_update(
                filter.clone(), doc! {"$set": &fields, "$unset": {"api_spec_url": ""}}
            ).return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?
                .ok_or_else(|| AppError::Conflict("Service changed or identity reconciliation is pending".into()))?;
            if tool {
                super::audit_service::log_actor_event_in_session(db, session, key, &authority.actor, "tools_manager_updated",
                    serde_json::json!({"service_id":live.id,"authority":if admin {"admin"} else {"tools_manager"},"fields":fields.keys().filter(|k| k.as_str() != "updated_at").collect::<Vec<_>>()})).await?;
            }
            Ok(updated)
        })
    }).await
}

/// Recheck the field-dependent boundaries against live rows within the same
/// transaction as the mutation (also used by skill metadata updates).
pub(crate) async fn validate_update_fields(
    db: &Database,
    session: &mut ClientSession,
    live: &DownstreamService,
    fields: &Document,
    authority: &Authority,
    admin: bool,
) -> AppResult<()> {
    if let Some(scheme) = fields.get("auth_scheme") {
        let scheme: Option<String> = bson::from_bson(scheme.clone())
            .map_err(|_| AppError::ValidationError("Invalid auth_scheme".into()))?;
        super::catalog_auth_scheme::validate(&live.auth_method, scheme.as_deref())?;
        if !admin
            && (!authority.first_party_human
                || live.offering_kind == OfferingKind::Tool
                || !live.catalog_manager_created
                || live.created_by != authority.actor.user_id)
        {
            return Err(AppError::Forbidden(
                "Only an Admin or the owning Catalog Manager may change an authorization scheme"
                    .into(),
            ));
        }
        // Public marked rows already fence Catalog authority in check_parent.
        // Private rows retain creator semantics for other fields, but D9's
        // credential-injection field still requires a live Catalog Manager.
        if !admin && live.visibility == "private" {
            tools_manager_service::role_authority(
                db,
                session,
                &authority.actor.user_id,
                Some(crate::models::role::CATALOG_SERVICES_WRITE_PERMISSION),
                true,
            )
            .await?;
        }
    }
    if admin || live.offering_kind == OfferingKind::Tool {
        return Ok(());
    }
    // Unmarked legacy creators keep their existing field permissions. Their
    // ingress field validation is bound by the parent contract projection.
    if live.created_by == authority.actor.user_id
        && (!live.catalog_manager_created || live.visibility == "private")
    {
        return Ok(());
    }
    if fields.contains_key("is_active") && live.created_by != authority.actor.user_id {
        return Err(AppError::Forbidden(
            "Only an Admin may change activation on someone else's service".into(),
        ));
    }
    if fields
        .get_str("base_url")
        .is_ok_and(|url| url != live.base_url)
    {
        if !live.credential_encrypted.is_empty() || live.platform_key.is_some() {
            return Err(AppError::Forbidden(
                "Only an Admin may move a service carrying a shared credential or platform key"
                    .into(),
            ));
        }
        if let Some(provider_id) = &live.provider_config_id {
            let provider = db
                .collection::<crate::models::provider_config::ProviderConfig>(
                    crate::models::provider_config::COLLECTION_NAME,
                )
                .find_one(doc! {"_id":provider_id})
                .session(&mut *session)
                .await?
                .ok_or_else(|| AppError::Conflict("Provider no longer exists".into()))?;
            if provider.created_by != authority.actor.user_id {
                return Err(AppError::Forbidden(
                    "Only an Admin may move a service using someone else's provider".into(),
                ));
            }
            db.collection::<Document>(crate::models::provider_config::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":provider_id},
                    doc! {"$inc":{"authority_fence":1_i64}},
                )
                .session(&mut *session)
                .await?;
        }
    }
    Ok(())
}

pub(crate) fn require_desired_kind(
    live: &DownstreamService,
    fields: &Document,
    authority: &Authority,
    admin: bool,
) -> AppResult<()> {
    let desired = fields.get_str("offering_kind").ok();
    if (live.offering_kind == OfferingKind::Tool || desired == Some("tool"))
        && !authority.first_party_human
    {
        return Err(AppError::Forbidden(
            "Tool mutation requires first-party human authority".into(),
        ));
    }
    if desired.is_some_and(|kind| {
        kind != if live.offering_kind == OfferingKind::Tool {
            "tool"
        } else {
            "ai_service"
        }
    }) && !admin
    {
        return Err(AppError::Forbidden(
            "Offering conversion requires Admin authority".into(),
        ));
    }
    Ok(())
}

/// OIDC catalog helpers retain their creator permissions but bind both writes
/// to the live parent. A conversion cannot inherit an OIDC request's authority.
pub async fn commit_oidc_update(
    db: &Database,
    observed: &DownstreamService,
    authority: &Authority,
    client_fields: &Document,
    service_fields: &Document,
    event: &str,
    audit_key: &[u8],
) -> AppResult<()> {
    let db = db.clone();
    let observed = observed.clone();
    let authority = authority.clone();
    let client_fields = client_fields.clone();
    let service_fields = service_fields.clone();
    let event = event.to_string();
    let key = zeroize::Zeroizing::new(audit_key.to_vec());
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result = async {
                let (live, _) = fence(&db, session, &observed, Some(&authority)).await?;
                if live.offering_kind == OfferingKind::Tool || live.auth_method != "oidc" {
                    return Err(AppError::BadRequest(
                        "OIDC helpers require a non-Tool OIDC service".into(),
                    ));
                }
                let client_id = live.oauth_client_id.as_ref().ok_or_else(|| {
                    AppError::Internal("OIDC service missing oauth_client_id".into())
                })?;
                let updated = db
                    .collection::<Document>(crate::models::oauth_client::COLLECTION_NAME)
                    .update_one(doc! {"_id":client_id}, doc! {"$set":&client_fields})
                    .session(&mut *session)
                    .await?;
                if updated.matched_count != 1 {
                    return Err(AppError::NotFound("OAuth client not found".into()));
                }
                db.collection::<Document>(COLLECTION_NAME)
                    .update_one(doc! {"_id":&live.id}, doc! {"$set":&service_fields})
                    .session(&mut *session)
                    .await?;
                super::audit_service::log_actor_event_in_session(
                    &db,
                    session,
                    &key,
                    &authority.actor,
                    &event,
                    serde_json::json!({"service_id":live.id}),
                )
                .await?;
                Ok(())
            }
            .await;
            super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::api_key_mutation_service::map_transaction_error)
}

pub(crate) async fn require_providerless_tool(
    db: &Database,
    session: &mut ClientSession,
    service: &DownstreamService,
) -> AppResult<()> {
    if service.provider_config_id.is_some()
        || db
            .collection::<Document>(crate::models::service_provider_requirement::COLLECTION_NAME)
            .find_one(doc! {"service_id":&service.id})
            .session(session)
            .await?
            .is_some()
    {
        return Err(AppError::Conflict("Tools cannot use provider links or provider requirements; remove them before conversion".into()));
    }
    Ok(())
}
