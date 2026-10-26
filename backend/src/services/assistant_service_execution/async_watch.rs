use super::*;
use crate::models::async_service_operation::{AsyncServiceOperation, COLLECTION_NAME as WATCHES};

/// A server-only capability for one declared status/result/cancel operation.
/// It never supplies a live turn or grants general service execution.
#[derive(Clone)]
pub(crate) struct AsyncContinuation {
    watch: AsyncServiceOperation,
    name: String,
    endpoint_contract: String,
    nodes: std::sync::Arc<crate::services::node_ws_manager::NodeWsManager>,
}

fn denied() -> AppError {
    AppError::Forbidden("Async operation authority changed".into())
}

pub(crate) fn async_contract_binding(
    service: &mcp_service::McpToolService,
    submit: &mcp_service::McpToolEndpoint,
) -> AppResult<String> {
    let contract = submit.async_operation.as_ref().ok_or_else(denied)?;
    crate::services::async_service_operation::validate_endpoints(contract, service, submit)?;
    let mut endpoints = vec![mcp_service::endpoint_contract_digest(submit)];
    for name in [&contract.status_operation, &contract.result_operation]
        .into_iter()
        .chain(contract.cancel_operation.as_ref())
    {
        let endpoint = service
            .endpoints
            .iter()
            .find(|e| &e.name == name)
            .ok_or_else(denied)?;
        endpoints.push(mcp_service::endpoint_contract_digest(endpoint));
    }
    Ok(digest(json!(endpoints)))
}

impl AsyncContinuation {
    pub(crate) fn new(
        watch: &AsyncServiceOperation,
        endpoint: &mcp_service::McpToolEndpoint,
        nodes: std::sync::Arc<crate::services::node_ws_manager::NodeWsManager>,
    ) -> AppResult<Self> {
        let name = endpoint.name.as_str();
        if crate::services::cma_operation::poll_id(watch).is_none()
            || watch.lease_id.is_none()
            || !(name == watch.contract.status_operation
                || name == watch.contract.result_operation
                || Some(name) == watch.contract.cancel_operation.as_deref())
        {
            return Err(denied());
        }
        Ok(Self {
            watch: watch.clone(),
            name: name.into(),
            endpoint_contract: mcp_service::endpoint_contract_digest(endpoint),
            nodes,
        })
    }

    pub(super) fn revalidate<'a>(
        &'a self,
        db: &'a Database,
        chat: &'a acks::ChatAuthority,
    ) -> futures::future::BoxFuture<'a, AppResult<()>> {
        Box::pin(self.revalidate_inner(db, chat))
    }

    async fn revalidate_inner(&self, db: &Database, chat: &acks::ChatAuthority) -> AppResult<()> {
        let w = &self.watch;
        let now = Utc::now();
        let invocation_cleanup_status = w.delivery.invocation_id.is_some()
            && w.state == "cancelling"
            && self.name == w.contract.status_operation;
        if chat.user_id != w.user_id
            || chat.api_key_id != w.api_key_id
            || chat.conversation_id != w.conversation_id
            || chat.agent_id != w.agent_id
            || w.expires_at <= now
            || (Some(self.name.as_str()) != w.contract.cancel_operation.as_deref()
                && !invocation_cleanup_status
                && w.deadline <= now)
        {
            return Err(denied());
        }
        let key = crate::services::key_service::get_api_key(db, &w.user_id, &w.api_key_id).await?;
        if key.expires_at.is_some_and(|at| at <= now) {
            return Err(denied());
        }
        let auth = Box::pin(crate::mw::auth::api_key_auth_user(
            db, &key, None, None, None,
        ))
        .await?;
        crate::services::org_agent_service::authorize_execution(db, &auth, Some(&w.service_id))
            .await?;
        if !auth.allow_all_services && !auth.allowed_service_ids.contains(&w.service_id) {
            return Err(denied());
        }
        let nodes = if auth.allow_all_nodes {
            mcp_service::NodeScope::Unrestricted
        } else {
            mcp_service::NodeScope::Allowed(&auth.allowed_node_ids)
        };
        let scope = if auth
            .assistant_agent_owner_id
            .as_deref()
            .is_some_and(|owner| owner != w.user_id)
        {
            mcp_service::ServiceScope::Allowed(&auth.allowed_service_ids)
        } else {
            mcp_service::ServiceScope::Unrestricted
        };
        let catalog = Box::pin(mcp_service::load_operation_catalog(
            db,
            &self.nodes,
            &w.user_id,
            nodes,
            scope,
        ))
        .await?;
        let service = catalog
            .services
            .iter()
            .find(|s| s.service_id == w.service_id)
            .ok_or_else(denied)?;
        let submit = service
            .endpoints
            .iter()
            .find(|e| e.endpoint_id == w.submit_endpoint_id)
            .ok_or_else(denied)?;
        if submit.async_operation.as_ref() != Some(&w.contract) {
            return Err(denied());
        }
        crate::services::async_service_operation::validate_endpoints(&w.contract, service, submit)?;
        let contract = async_contract_binding(service, submit)?;
        if w.contract_binding
            .as_ref()
            .is_some_and(|approved| approved != &contract)
            || (w.authority_version > 0 && w.contract_binding.is_none())
        {
            return Err(denied());
        }
        let endpoint = service
            .endpoints
            .iter()
            .find(|e| e.name == self.name)
            .ok_or_else(denied)?;
        if self.endpoint_contract != mcp_service::endpoint_contract_digest(endpoint) {
            return Err(denied());
        }
        let args =
            json!({w.contract.id_parameter.clone():crate::services::cma_operation::poll_id(w)});
        mcp_service::prepare_proxy_tool_call(service, endpoint, &args)?
            .authorize_agent_operations(&auth.assistant_operation_scopes, service, endpoint)?;
        let fresh = acks::for_key(db, &w.user_id, Some(&w.api_key_id))
            .await?
            .ok_or_else(denied)?;
        if fresh.agent_id != w.agent_id || fresh.conversation_id != w.conversation_id {
            return Err(denied());
        }
        let mut filter = doc! {
            "_id":&w.id,"user_id":&w.user_id,"conversation_id":&w.conversation_id,
            "api_key_id":&w.api_key_id,"service_id":&w.service_id,
            "submit_endpoint_id":&w.submit_endpoint_id,"operation_id":&w.operation_id,
            "contract":bson::to_bson(&w.contract).map_err(|_| denied())?,
            "state":&w.state,"lease_id":&w.lease_id,"lease_until":{"$gt":bson::DateTime::now()},
            "execution_binding":&w.execution_binding,
            "delivery.invocation_id":&w.delivery.invocation_id,
        };
        if w.operation_id.is_none() {
            filter.insert("cma.kind", "cma_agent");
            filter.insert("cma.agent_id", crate::services::cma_operation::poll_id(w));
        }
        if w.authority_version == 0 {
            filter.insert(
                "$or",
                vec![
                    doc! {"authority_version":0},
                    doc! {"authority_version":null},
                ],
            );
        } else {
            filter.insert("authority_version", i32::from(w.authority_version));
        }
        if db
            .collection::<Document>(WATCHES)
            .find_one(filter)
            .await?
            .is_none()
        {
            return Err(denied());
        }
        if let Some(invocation) = w.delivery.invocation_id.as_deref() {
            let mut root = doc! {
                "_id":invocation,"user_id":&w.user_id,"terminal_status":null,
            };
            if w.state == "cancelling" {
                // This capability can only cancel the exact leased operation
                // once and read its declared status to confirm the outcome.
                // Root cancellation must not prevent that bounded cleanup.
                if self.name != w.contract.status_operation
                    && Some(self.name.as_str()) != w.contract.cancel_operation.as_deref()
                {
                    return Err(denied());
                }
                let attempt = doc! {
                    "_id":&w.id,"user_id":&w.user_id,"state":"cancelling",
                    "lease_id":&w.lease_id,"lease_until":{"$gt":bson::DateTime::now()},
                    "cancel_attempted":true,
                };
                if db
                    .collection::<Document>(WATCHES)
                    .find_one(attempt)
                    .await?
                    .is_none()
                {
                    return Err(denied());
                }
            } else {
                root.insert("cancel_requested", false);
                root.insert("conversation_deleted", false);
            }
            if db
                .collection::<Document>(crate::models::assistant_invocation::COLLECTION_NAME)
                .find_one(root)
                .projection(doc! {"_id":1})
                .await?
                .is_none()
            {
                return Err(denied());
            }
        }
        Ok(())
    }

    pub(crate) async fn admit(
        &self,
        db: &Database,
        chat: &acks::ChatAuthority,
        connection: &str,
        target: &crate::services::proxy_service::ProxyTarget,
        operation: &Value,
    ) -> AppResult<Decision> {
        self.revalidate(db, chat).await?;
        if connection != self.watch.service_id {
            return Err(denied());
        }
        let binding = connection_binding(db, connection, &chat.api_key_id).await?;
        // Legacy watches were admitted under the pre-upgrade exact async
        // contract. Keep their bounded lifecycle, including organization use.
        // New submissions must carry the snapshot produced by their real gate.
        if self.watch.authority_version > 0 && self.watch.execution_binding.is_none() {
            return Err(denied());
        }
        if self
            .watch
            .execution_binding
            .as_ref()
            .is_some_and(|approved| approved != &binding)
        {
            return Err(denied());
        }
        Ok(Decision {
            refusal: None,
            automatic: None,
            admission: Some(Admission {
                connection: connection.into(),
                binding,
                execution: Some(execution_digest(target)),
                execution_credential: true,
                operation: digest(operation.clone()),
                operation_identity: operation.clone(),
                native_execution: None,
                read: operation["method"] == "GET",
                api_key_id: chat.api_key_id.clone(),
                summary: "Async operation continuation".into(),
                job_id: None,
                async_continuation: Some(self.clone()),
                chat: chat.clone(),
            }),
        })
    }
}

impl Admission {
    /// Persist exactly the authority that passed submission consent, never a
    /// second read that could bless a replacement credential/configuration.
    pub(crate) async fn bind_async_submission(&self, db: &Database, watch: &str) -> AppResult<()> {
        let updated = db.collection::<Document>(WATCHES).update_one(doc! {
            "_id":watch,"user_id":&self.chat.user_id,"conversation_id":&self.chat.conversation_id,
            "api_key_id":&self.api_key_id,"turn_id":&self.chat.turn_id,"service_id":&self.connection,
            "state":{"$in":["submitting","cancelling"]},"deadline":{"$gt":bson::DateTime::now()},
            "$or":[{"execution_binding":null},{"execution_binding":&self.binding}],
        }, doc! {"$set":{"execution_binding":&self.binding,
            "contract_binding":self.operation_identity["async_contract"].as_str().ok_or_else(denied)?}}).await?;
        if updated.matched_count != 1 {
            return Err(denied());
        }
        Ok(())
    }
}
