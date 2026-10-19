//! Exact certificate/session issuance for assistant SSH callers. Session consent
//! authorizes one bounded session, not individual bytes or terminal commands.
use super::{assistant_acknowledgement_service as acks, assistant_service_execution as execution};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{downstream_service::SshServiceConfig, user_service::UserService},
};
use serde_json::{Value, json};

pub(crate) struct SessionAdmission {
    admission: Option<execution::Admission>,
    catalog_admission: Option<super::assistant_native_execution::ResourceAdmission>,
    automatic: Option<acks::AutoAllowed>,
    connection: Option<UserService>,
    chat: acks::ChatAuthority,
    catalog_projection: Option<(String, SshServiceConfig)>,
    deadline: tokio::time::Instant,
}
impl SessionAdmission {
    pub(crate) fn connection(&self) -> Option<&UserService> {
        self.connection.as_ref()
    }
    pub(crate) fn revalidate<'a>(
        &'a self,
        state: &'a AppState,
        node: Option<&'a str>,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(async move {
            if tokio::time::Instant::now() >= self.deadline {
                return Err(AppError::Forbidden("Approved SSH session expired".into()));
            }
            if let (Some(admission), Some(connection)) =
                (self.admission.as_ref(), self.connection.as_ref())
            {
                admission
                    .revalidate(&state.db, &connection.id, None)
                    .await?;
                if let Some(node) = node {
                    super::assistant_native_execution::verify_ssh_node(
                        state,
                        &self.chat,
                        &connection.id,
                        node,
                    )
                    .await?;
                }
            }
            if let Some((catalog, config)) = self.catalog_projection.as_ref() {
                let current = super::ssh_service::get_ssh_service(&state.db, catalog).await?;
                if serde_json::to_value(&current).ok() != serde_json::to_value(config).ok() {
                    return Err(AppError::Conflict(
                        "Resolved SSH session config changed".into(),
                    ));
                }
            }
            if let Some(admission) = self.catalog_admission.as_ref() {
                admission.revalidate(&state.db).await?;
                if let Some(node) = node {
                    let key = super::key_service::get_api_key(
                        &state.db,
                        &self.chat.user_id,
                        &self.chat.api_key_id,
                    )
                    .await?;
                    let access = super::org_agent_service::resolve_key_access(
                        &state.db,
                        &key.user_id,
                        key.assistant_agent_owner_id.as_deref(),
                    )
                    .await?;
                    let (all, nodes) =
                        super::org_agent_service::key_nodes(&state.db, &key, access.as_deref())
                            .await?;
                    if !all && !nodes.iter().any(|id| id == node) {
                        return Err(AppError::ApiKeyScopeForbidden(
                            "SSH node outside current authority".into(),
                        ));
                    }
                }
            }
            Ok(())
        })
    }
    pub(crate) async fn receipt(&self, state: &AppState) {
        if let Some(admission) = self.admission.as_ref() {
            admission
                .receipt(&state.db, &self.chat, self.automatic)
                .await;
        }
        if let Some(admission) = self.catalog_admission.as_ref() {
            admission.receipt(&state.db).await;
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn admit<'a>(
    state: &'a AppState,
    chat: Option<&'a acks::ChatAuthority>,
    connection: Option<&'a UserService>,
    config: &'a SshServiceConfig,
    catalog_id: &'a str,
    kind: &'a str,
    parameters: Value,
    acknowledgement: Option<&'a str>,
    ttl_secs: u64,
) -> futures::future::BoxFuture<'a, AppResult<Result<Option<SessionAdmission>, Value>>> {
    Box::pin(async move {
        let Some(chat) = chat else {
            return Ok(Ok(None));
        };
        if chat.guest {
            return Err(AppError::Forbidden(
                "Guests cannot open SSH sessions".into(),
            ));
        }
        let ttl_secs = ttl_secs.clamp(1, 3600);
        let operation = json!({"kind":"ssh", "retry_tool":kind, "session_kind":kind,
        "parameters":super::mcp_service::canonical_sha256(parameters), "ttl_secs":ttl_secs});
        let Some(connection) = connection else {
            execution::require_catalog_only_selection(&state.db, chat, catalog_id).await?;
            let catalog = state
                .db
                .collection::<crate::models::downstream_service::DownstreamService>(
                    "downstream_services",
                )
                .find_one(bson::doc! {"_id":catalog_id,"is_active":true})
                .await?
                .ok_or_else(|| AppError::NotFound("SSH service unavailable".into()))?;
            if serde_json::to_value(super::ssh_service::ensure_ssh_service(&catalog)?).ok()
                != serde_json::to_value(config).ok()
            {
                return Err(AppError::Conflict(
                    "Resolved SSH session config changed".into(),
                ));
            }
            let owner = super::ownership_transfer_service::catalog_owner(&catalog);
            let mut args = operation;
            if let Some(ack) = acknowledgement {
                args["acknowledgement_id"] = json!(ack);
            }
            let admission = match super::assistant_native_execution::resource(
                &state.db,
                Some(chat),
                super::assistant_native_execution::NativeResource {
                    collection: "downstream_services",
                    id: catalog_id,
                    owner,
                },
                kind,
                &args,
                acks::ActionFacts::of(acks::OwnerActionClass::Change),
                &format!("Issue {kind} for at most {ttl_secs} seconds"),
            )
            .await?
            {
                Ok(admission) => admission,
                Err(refusal) => return Ok(Err(refusal)),
            };
            return Ok(Ok(Some(SessionAdmission {
                admission: None,
                catalog_admission: admission,
                automatic: None,
                connection: None,
                chat: chat.clone(),
                catalog_projection: Some((catalog_id.to_owned(), config.clone())),
                deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(ttl_secs),
            })));
        };
        let mut decision = execution::check(
            &state.db,
            &state.node_ws_manager,
            chat,
            &connection.id,
            &connection.user_id,
            &operation,
            acks::ActionFacts::of(acks::OwnerActionClass::Change),
            acknowledgement,
            &format!("Issue {kind} for at most {ttl_secs} seconds"),
            false,
            None,
            false,
            None,
        )
        .await?;
        if let Some(refusal) = decision.refusal {
            return Ok(Err(refusal));
        }
        let mut admission = decision
            .admission
            .take()
            .ok_or(AppError::AssistantTurnRequired)?;
        admission.bind_ssh(connection, config);
        // Check the held native projection as well as the freshly read binding.
        admission
            .revalidate(&state.db, &connection.id, None)
            .await?;
        Ok(Ok(Some(SessionAdmission {
            admission: Some(admission),
            catalog_admission: None,
            automatic: decision.automatic,
            connection: Some(connection.clone()),
            chat: chat.clone(),
            catalog_projection: None,
            deadline: tokio::time::Instant::now() + std::time::Duration::from_secs(ttl_secs),
        })))
    })
}

pub(crate) fn route<'a>(
    state: &'a AppState,
    admission: Option<&'a SessionAdmission>,
    actor: &'a str,
    catalog: &'a str,
) -> futures::future::BoxFuture<'a, AppResult<Option<super::node_routing_service::NodeRoute>>> {
    Box::pin(async move {
        if let Some(row) = admission.and_then(SessionAdmission::connection) {
            let configured = super::node_routing_service::list_configured_binding_node_ids(
                &state.db,
                &row.user_id,
                row.catalog_service_id.as_deref().unwrap_or(&row.id),
            )
            .await?;
            if row.node_id.as_ref().is_none_or(|id| id.is_empty()) && configured.is_empty() {
                return Ok(None);
            }
            return super::assistant_native_execution::ssh_route(state, row)
                .await
                .map(Some);
        }
        let route = super::node_routing_service::resolve_node_route(
            &state.db,
            actor,
            catalog,
            &state.node_ws_manager,
        )
        .await?;
        if admission.is_some() && route.is_none() {
            // Catalog-only assistant sessions may use direct TCP only when no
            // credential-node route is configured. An offline route is not direct.
            let owner = super::proxy_service::find_effective_service_owner(
                &state.db,
                actor,
                None,
                Some(catalog),
            )
            .await?;
            if !super::node_routing_service::list_configured_binding_node_ids(
                &state.db,
                owner.as_deref().unwrap_or(actor),
                catalog,
            )
            .await?
            .is_empty()
            {
                return Err(AppError::NodeOffline(
                    "Configured SSH credential nodes are offline".into(),
                ));
            }
        }
        Ok(route)
    })
}

/// Existing sessions stop on elapsed consent or revoked live authority. Ordinary
/// client sessions retain their existing lifetime and supervision contract.
pub(crate) async fn monitor(
    state: &AppState,
    admission: Option<&SessionAdmission>,
    node: Option<&str>,
) {
    let Some(admission) = admission else {
        return std::future::pending().await;
    };
    loop {
        if admission.revalidate(state, node).await.is_err() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
}
