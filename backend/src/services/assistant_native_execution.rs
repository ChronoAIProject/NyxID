//! Exact native resource consent, for adapters without HTTP ProxyTarget inputs.
use super::{assistant_acknowledgement_service as acks, assistant_service_execution as execution};
use crate::{
    AppState,
    errors::{AppError, AppResult},
};
use bson::{Document, doc};
use serde_json::{Value, json};

pub struct ResourceAdmission {
    collection: &'static str,
    id: String,
    binding: String,
    key_binding: String,
    chat: acks::ChatAuthority,
    automatic: Option<acks::AutoAllowed>,
    tool: String,
    summary: String,
}
fn binding(row: &Document) -> String {
    let mut row = row.clone();
    for field in [
        "created_at",
        "updated_at",
        "name",
        "description",
        "worker_token_hash",
    ] {
        row.remove(field);
    }
    super::mcp_service::canonical_sha256(json!(row))
}
impl ResourceAdmission {
    pub async fn revalidate(&self, db: &mongodb::Database) -> AppResult<()> {
        let live =
            execution::personal_authority(db, &self.chat, "assistant.native_egress", None, false)
                .await?;
        if key_binding(db, &live).await? != self.key_binding {
            return Err(AppError::Conflict(
                "Native execution authority changed; confirm again".into(),
            ));
        }
        let row = db
            .collection::<Document>(self.collection)
            .find_one(doc! {"_id":&self.id,"is_active":true})
            .await?
            .ok_or_else(|| AppError::Forbidden("Native service unavailable".into()))?;
        if binding(&row) != self.binding {
            return Err(AppError::Conflict(
                "Native service changed; confirm again".into(),
            ));
        }
        if self.collection == "oracle_pools" {
            let pool: crate::models::oracle_pool::OraclePool = bson::from_document(row)
                .map_err(|_| AppError::Internal("Native resource projection failed".into()))?;
            super::oracle_pool_service::ensure_can_submit(db, &live.user_id, &pool).await?;
        } else if self.collection == "downstream_services" {
            let service: crate::models::downstream_service::DownstreamService =
                bson::from_document(row)
                    .map_err(|_| AppError::Internal("Native resource projection failed".into()))?;
            let owner = super::proxy_service::find_effective_service_owner(
                db,
                &live.user_id,
                None,
                Some(&self.id),
            )
            .await?;
            if owner.is_none()
                && service.visibility == "private"
                && super::ownership_transfer_service::catalog_owner(&service) != live.user_id
            {
                return Err(AppError::Forbidden("SSH access revoked".into()));
            }
        }
        Ok(())
    }
    pub async fn receipt(&self, db: &mongodb::Database) {
        if let Some(auto) = self.automatic {
            acks::record_auto_allowed(
                db,
                &self.chat,
                &self.tool,
                &json!({"resource_id":self.id}),
                &auto,
                &self.summary,
            )
            .await;
        }
    }
}

async fn key_binding(db: &mongodb::Database, chat: &acks::ChatAuthority) -> AppResult<String> {
    let key = super::key_service::get_api_key(db, &chat.user_id, &chat.api_key_id).await?;
    if key.expires_at.is_some_and(|at| at <= chrono::Utc::now()) {
        return Err(AppError::Forbidden("Assistant credential expired".into()));
    }
    let access = super::org_agent_service::resolve_key_access(
        db,
        &key.user_id,
        key.assistant_agent_owner_id.as_deref(),
    )
    .await?;
    let allowed =
        super::key_service::effective_allowed_service_ids_with_access(db, &key, access.as_deref())
            .await?;
    Ok(super::mcp_service::canonical_sha256(
        json!({"key":key.id,"expires":key.expires_at,
        "scopes":key.assistant_operation_scopes,"services":allowed,"all":key.allow_all_services,
        "mode":acks::effective_access_mode(db, chat).await?}),
    ))
}
pub struct NativeResource<'a> {
    pub collection: &'static str,
    pub id: &'a str,
    pub owner: &'a str,
}

pub async fn resource(
    db: &mongodb::Database,
    chat: Option<&acks::ChatAuthority>,
    resource: NativeResource<'_>,
    tool: &str,
    arguments: &Value,
    facts: acks::ActionFacts,
    summary: &str,
) -> AppResult<Result<Option<ResourceAdmission>, Value>> {
    let NativeResource {
        collection,
        id,
        owner,
    } = resource;
    let Some(chat) = chat else {
        return Ok(Ok(None));
    };
    chat.ensure_live_turn(db, None, None, "assistant.native_execution")
        .await?;
    if chat.guest {
        return Err(AppError::Forbidden(
            "Guest native execution is unavailable".into(),
        ));
    }
    let row = db
        .collection::<Document>(collection)
        .find_one(doc! {"_id":id,"is_active":true})
        .await?
        .ok_or_else(|| AppError::NotFound("Native service unavailable".into()))?;
    let actual_owner = if collection == "downstream_services" {
        let service: crate::models::downstream_service::DownstreamService =
            bson::from_document(row.clone())
                .map_err(|_| AppError::Internal("Native resource projection failed".into()))?;
        super::ownership_transfer_service::catalog_owner(&service).to_owned()
    } else {
        row.get_str("user_id").unwrap_or_default().to_owned()
    };
    if actual_owner != owner {
        return Err(AppError::Forbidden("Native resource owner changed".into()));
    }
    let binding = binding(&row);
    let key_binding = key_binding(db, chat).await?;
    let mut semantic = arguments.clone();
    let ack = semantic
        .as_object_mut()
        .and_then(|args| args.remove("acknowledgement_id"));
    let mut exact = json!({"resource_id":id,"owner_id":owner,"authority":binding,"key_authority":key_binding,
        "operation":super::mcp_service::canonical_sha256(semantic)});
    if let Some(ack) = ack {
        exact["acknowledgement_id"] = ack;
    }
    let owner_row = db
        .collection::<crate::models::user::User>("users")
        .find_one(doc! {"_id":owner,"is_active":true})
        .await?
        .ok_or_else(|| AppError::Forbidden("Resource owner unavailable".into()))?;
    let org = owner_row.user_type == crate::models::user::UserType::Org;
    if org {
        chat.turn_fence().require_live()?;
    }
    let summary = if org {
        format!(
            "Organization service {} ({}) in {}: {}",
            super::assistant_nyxagent::identifier(row.get_str("slug").unwrap_or(id)),
            id,
            super::assistant_nyxagent::identifier(
                owner_row
                    .display_name
                    .as_deref()
                    .unwrap_or(owner_row.slug.as_deref().unwrap_or(&owner_row.id))
            ),
            summary
        )
    } else {
        summary.to_owned()
    };
    let gate = acks::confirmation_needed(db, chat, facts, org).await?;
    if gate.ask().is_some() {
        chat.turn_fence().require_live()?;
    }
    if let Some(need) = gate.ask()
        && let Some(refusal) = acks::confirm_action(db, chat, tool, &exact, &need, &summary).await?
    {
        return Ok(Err(refusal));
    }
    Ok(Ok(Some(ResourceAdmission {
        collection,
        id: id.to_owned(),
        binding,
        key_binding,
        chat: chat.clone(),
        automatic: gate.auto_allowed(),
        tool: tool.to_owned(),
        summary,
    })))
}

/// Native SSH resolves a concrete visible connection without losing exact
/// connection slugs to a second catalog lookup. Catalog routes use saved order.
pub async fn ssh_connection(
    state: &AppState,
    actor: &str,
    reference: &str,
) -> AppResult<crate::models::user_service::UserService> {
    let visible =
        super::user_service_service::list_user_services_with_sources(&state.db, actor).await?;
    let mut rows: Vec<_> = visible
        .into_iter()
        .filter(|row| {
            row.service.service_type == "ssh"
                && !matches!(
                    row.source,
                    super::user_service_service::CredentialSource::Org { allowed: false, .. }
                )
        })
        .map(|row| row.service)
        .collect();
    if let Some(index) = rows
        .iter()
        .position(|row| row.id == reference || row.slug == reference)
    {
        return Ok(rows.remove(index));
    }
    let catalog = state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .find_one(doc! {"slug":reference,"service_type":"ssh","is_active":true})
        .await?;
    let catalog = catalog
        .as_ref()
        .map(|row| row.id.as_str())
        .unwrap_or(reference);
    rows.retain(|row| row.catalog_service_id.as_deref() == Some(catalog));
    let order = super::service_preference_service::get(&state.db, actor)
        .await?
        .map(|p| p.ordered)
        .unwrap_or_default();
    rows.sort_by_key(|row| {
        (
            row.user_id != actor,
            order
                .iter()
                .position(|id| id == &row.id)
                .unwrap_or(usize::MAX),
        )
    });
    rows.into_iter()
        .next()
        .ok_or_else(|| AppError::NotFound("SSH service unavailable".into()))
}

pub async fn ssh_gate(
    state: &AppState,
    chat: Option<&acks::ChatAuthority>,
    connection: &crate::models::user_service::UserService,
    config: &crate::models::downstream_service::SshServiceConfig,
    arguments: &Value,
) -> AppResult<execution::Decision> {
    let Some(chat) = chat else {
        return Ok(execution::Decision {
            admission: None,
            automatic: None,
            refusal: None,
        });
    };
    if chat.guest {
        return Err(AppError::Forbidden(
            "Guests cannot execute SSH operations".into(),
        ));
    }
    let mut semantic = arguments.clone();
    let ack = semantic
        .as_object_mut()
        .and_then(|map| map.remove("acknowledgement_id"))
        .and_then(|v| v.as_str().map(str::to_owned));
    let operation = json!({"kind":"ssh","retry_tool":"nyx__ssh_exec","arguments":super::mcp_service::canonical_sha256(semantic)});
    let mut decision = execution::check(
        &state.db,
        &state.node_ws_manager,
        chat,
        &connection.id,
        &connection.user_id,
        &operation,
        acks::ActionFacts::of(acks::OwnerActionClass::Destructive),
        ack.as_deref(),
        "Execute the approved SSH operation",
        false,
        None,
        false,
        None,
    )
    .await?;
    if let Some(admission) = decision.admission.as_mut() {
        admission.bind_ssh(connection, config);
    }
    Ok(decision)
}

pub async fn ssh_route(
    state: &AppState,
    row: &crate::models::user_service::UserService,
) -> AppResult<super::node_routing_service::NodeRoute> {
    let configured = super::node_routing_service::list_dispatchable_binding_node_ids(
        &state.db,
        &row.user_id,
        row.catalog_service_id.as_deref().unwrap_or(&row.id),
        &state.node_ws_manager,
    )
    .await?;
    let primary = row
        .node_id
        .clone()
        .filter(|id| !id.is_empty())
        .or_else(|| configured.first().cloned())
        .ok_or_else(|| AppError::NodeOffline("SSH has no credential node".into()))?;
    Ok(super::node_routing_service::NodeRoute {
        node_id: primary.clone(),
        fallback_node_ids: configured.into_iter().filter(|id| id != &primary).collect(),
    })
}

/// The attempted node is an execution principal. Recheck its exact configured
/// identity and the live key's effective node grant after all intervening awaits.
pub async fn verify_ssh_node(
    state: &AppState,
    chat: &acks::ChatAuthority,
    connection: &str,
    node: &str,
) -> AppResult<()> {
    let live = execution::personal_authority(
        &state.db,
        chat,
        "assistant.ssh_egress",
        Some(connection),
        true,
    )
    .await?;
    let row = super::user_service_service::find_user_service_by_id(&state.db, connection)
        .await?
        .ok_or_else(|| AppError::NotFound("SSH connection unavailable".into()))?;
    let key = super::key_service::get_api_key(&state.db, &live.user_id, &live.api_key_id).await?;
    let access = super::org_agent_service::resolve_key_access(
        &state.db,
        &key.user_id,
        key.assistant_agent_owner_id.as_deref(),
    )
    .await?;
    let (all, nodes) =
        super::org_agent_service::key_nodes(&state.db, &key, access.as_deref()).await?;
    let mut configured = super::node_routing_service::list_configured_binding_node_ids(
        &state.db,
        &row.user_id,
        row.catalog_service_id.as_deref().unwrap_or(&row.id),
    )
    .await?;
    configured.extend(row.node_id);
    if !configured.iter().any(|id| id == node) || (!all && !nodes.iter().any(|id| id == node)) {
        return Err(AppError::ApiKeyScopeForbidden(
            "SSH credential node is outside current authority".into(),
        ));
    }
    let owner =
        super::org_service::resolve_owner_access(&state.db, &live.user_id, &row.user_id).await?;
    if !owner.allows_resource(node) {
        return Err(AppError::Forbidden(
            "SSH credential node access revoked".into(),
        ));
    }
    Ok(())
}

pub(super) fn ssh_projection(
    connection: &crate::models::user_service::UserService,
    config: &crate::models::downstream_service::SshServiceConfig,
) -> Value {
    let mut row = serde_json::to_value(connection).expect("SSH connection projection");
    for field in [
        "created_at",
        "updated_at",
        "name",
        "description",
        "icon_url",
    ] {
        row.as_object_mut().expect("connection").remove(field);
    }
    json!({"connection":row,"config":config})
}
