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

fn instruction_identity(w: &AsyncServiceOperation) -> AppResult<Document> {
    let mut identity = doc! {
        "_id":&w.id,"user_id":&w.user_id,"api_key_id":&w.api_key_id,"service_id":&w.service_id,
        "conversation_id":&w.conversation_id,"agent_id":&w.agent_id,
        "submit_endpoint_id":&w.submit_endpoint_id,"contract":bson::to_bson(&w.contract).map_err(|_| denied())?,
        "contract_binding":&w.contract_binding,"turn_id":&w.turn_id,
        "deadline":bson::DateTime::from_chrono(w.deadline),"expires_at":bson::DateTime::from_chrono(w.expires_at),
        "operation_id":&w.operation_id,
        "delivery.invocation_id":&w.delivery.invocation_id,
        "lease_id":&w.lease_id,"lease_until":{"$gt":bson::DateTime::now()},"state":&w.state,
    };
    if w.authority_version == 0 {
        identity.insert(
            "$or",
            vec![
                doc! {"authority_version":0},
                doc! {"authority_version":null},
            ],
        );
    } else {
        identity.insert("authority_version", i32::from(w.authority_version));
    }
    if w.operation_id.is_none() {
        identity.insert("cma.kind", "cma_agent");
        identity.insert("cma.agent_id", crate::services::cma_operation::poll_id(w));
    }
    Ok(identity)
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
        Box::pin(self.revalidate_inner(db, chat, true))
    }

    async fn revalidate_inner(
        &self,
        db: &Database,
        chat: &acks::ChatAuthority,
        require_instruction: bool,
    ) -> AppResult<()> {
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
        let service =
            crate::services::user_service_service::find_user_service_by_id(db, &w.service_id)
                .await?
                .ok_or_else(denied)?;
        if w.execution_task.is_some()
            && w.shared_execution
                .as_ref()
                .is_some_and(|binding| binding.consent_id.is_none())
            && service.user_id == chat.user_id
            && !super::super::execution_permission_dispatch::covers(
                chat,
                crate::models::execution_permission::ExecutionFloor::SharedConnectionConsent,
                &w.service_id,
            )
        {
            return Err(denied());
        }
        let instruction_required =
            require_instruction && instruction::covered(db, chat, &service).await?;
        if instruction_required && w.instruction_receipt_version != 1 {
            return Err(denied());
        }
        let key = crate::services::key_service::get_api_key(db, &w.user_id, &w.api_key_id).await?;
        if key.expires_at.is_some_and(|at| at <= now) {
            return Err(denied());
        }
        let auth = Box::pin(crate::mw::auth::api_key_auth_user_for_watch(db, &key, w)).await?;
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
        let scope = if let Some(access) = auth.org_agent_access.as_deref() {
            mcp_service::ServiceScope::Organization { access }
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
        if instruction_required {
            filter.insert("instruction_receipt_version", 1);
        }
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
        // Ordinary watch revalidation runs below, after a legacy organization
        // watch has had the chance to present its bounded continuation card.
        crate::services::shared_agent_transport::revalidate_watch(db, &self.watch).await?;
        if connection != self.watch.service_id {
            return Err(denied());
        }
        let service = super::super::user_service_service::find_user_service_by_id(db, connection)
            .await?
            .ok_or_else(denied)?;
        let binding = connection_binding(db, connection, &chat.api_key_id).await?;
        let mut authority = self.clone();
        let instruction_covered = instruction::covered(db, chat, &service).await?;
        if instruction_covered && authority.watch.instruction_receipt_version != 1 {
            // Status and result calls in one polling pass share the original
            // snapshot. Adopt only the exact leased watch's persisted approval.
            let mut approved = instruction_identity(&authority.watch)?;
            approved.insert("instruction_receipt_version", 1);
            approved.insert("execution_binding", &binding);
            if db
                .collection::<Document>(WATCHES)
                .find_one(approved)
                .projection(doc! {"_id":1})
                .await?
                .is_some()
            {
                authority.watch.instruction_receipt_version = 1;
                authority.watch.execution_binding = Some(binding.clone());
            }
        }
        if instruction_covered && authority.watch.instruction_receipt_version != 1 {
            authority.revalidate_inner(db, chat, false).await?;
            let w = &authority.watch;
            // One card covers only this existing remote operation and its declared
            // status/result/cancel lifecycle. It cannot authorize a new submission.
            let mut exact = json!({"watch_id":w.id,"service_id":connection,
                "remote_operation":crate::services::cma_operation::poll_id(w),
                "submit_endpoint_id":w.submit_endpoint_id,"contract":w.contract,
                "execution_binding":binding,"deadline":w.deadline,"expires_at":w.expires_at});
            let card = db.collection::<Document>("assistant_acknowledgements").find_one(doc! {
                "user_id":&chat.user_id,"conversation_id":&chat.conversation_id,"api_key_id":&chat.api_key_id,
                "tool_name":"nyxid.async_continuation","kind":"action","status":"allowed",
                "decided_by":"user","confirmation.reason":"org_connection",
                "arguments_digest":acks::arguments_digest(&exact),"expires_at":{"$gt":bson::DateTime::now()},
            }).await?;
            if let Some(card) = card {
                exact["acknowledgement_id"] = json!(card.get_str("_id").map_err(|_| denied())?);
            }
            let need = acks::ConfirmationNeed {
                org_instruction: acks::OrgInstructionRequirement::Required,
                class: acks::OwnerActionClass::Read,
                mode: None,
                webhook: false,
                policy: false,
                delegation: false,
            };
            if let Some(refusal)=acks::confirm_action(db,chat,"nyxid.async_continuation",&exact,&need,
                "Resume this existing organization operation's bounded status, result and cancellation watch").await? {
                let refusal = super::super::assistant_connection_service::org_refusal(db, chat, &service, refusal).await?;
                return Ok(Decision{refusal:Some(refusal),admission:None,automatic:None});
            }
            let mut identity = instruction_identity(w)?;
            identity.insert("execution_binding", &w.execution_binding);
            let changed = db
                .collection::<Document>(WATCHES)
                .update_one(
                    identity,
                    doc! {"$set":{"instruction_receipt_version":1,"execution_binding":&binding}},
                )
                .await?;
            if changed.matched_count != 1 {
                return Err(denied());
            }
            authority.watch.instruction_receipt_version = 1;
            authority.watch.execution_binding = Some(binding.clone());
        }
        authority.revalidate(db, chat).await?;
        // Legacy personal watches retain their bounded pre-upgrade contract.
        // Organization watches additionally require the instruction receipt.
        // New submissions must carry the snapshot produced by their real gate.
        if authority.watch.authority_version > 0 && authority.watch.execution_binding.is_none() {
            return Err(denied());
        }
        if authority
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
                bound_skill_read: false,
                owner_instruction_receipt: false,
                shared_detached: None,
                shared_permit: self
                    .watch
                    .shared_execution
                    .as_ref()
                    .and_then(crate::services::shared_agent_consent::Permit::from_execution),
                connection: connection.into(),
                binding,
                execution: Some(execution_projection_digest(
                    target,
                    !target.credential_deferred,
                )),
                execution_credential: !target.credential_deferred,
                operation: digest(operation.clone()),
                operation_identity: operation.clone(),
                native_execution: None,
                read: operation["method"] == "GET",
                api_key_id: chat.api_key_id.clone(),
                summary: "Async operation continuation".into(),
                job_id: None,
                async_continuation: Some(authority),
                chat: chat.clone(),
            }),
        })
    }
}

impl Admission {
    /// Persist exactly the authority that passed submission consent, never a
    /// second read that could bless a replacement credential/configuration.
    pub(crate) async fn bind_async_submission(
        &self,
        db: &Database,
        watch: &str,
        target: &crate::services::proxy_service::ProxyTarget,
    ) -> AppResult<()> {
        let shared = self.shared_execution()?;
        if let Some(binding) = &shared {
            crate::services::shared_agent_transport::revalidate_permission_submission(
                db, binding, &self.chat,
            )
            .await?;
        }
        // Only an unchanged, personal default connection can be observed through
        // the owner's REST proxy. Override/shared credentials must never be relabelled.
        let is_cma = db
            .collection::<Document>(WATCHES)
            .find_one(doc! {"_id":watch,"cma":{"$ne":null}})
            .projection(doc! {"_id":1})
            .await?
            .is_some();
        let observer_binding =
            if !is_cma || target.target_id.is_some() || self.chat.guest || shared.is_some() {
                None
            } else {
                let binding = crate::services::cma_session_origin::personal_binding(
                    db,
                    &self.chat.user_id,
                    &self.connection,
                    Some(&self.api_key_id),
                )
                .await?;
                if binding.is_some()
                    && connection_binding(db, &self.connection, &self.api_key_id).await?
                        != self.binding
                {
                    return Err(denied());
                }
                binding
            };
        let mut fields = doc! {
            "execution_binding": &self.binding,
            "instruction_receipt_version": if self.owner_instruction_receipt { 1 } else { 0 },
            "shared_execution": bson::to_bson(&shared).map_err(|_| denied())?,
            "contract_binding": self.operation_identity["async_contract"].as_str().ok_or_else(denied)?,
        };
        if is_cma {
            fields.insert("cma.observer_binding", observer_binding);
        }
        let updated = db.collection::<Document>(WATCHES).update_one(doc! {
            "_id":watch,"user_id":&self.chat.user_id,"conversation_id":&self.chat.conversation_id,
            "api_key_id":&self.api_key_id,"turn_id":&self.chat.turn_id,"service_id":&self.connection,
            "state":{"$in":["submitting","cancelling"]},"deadline":{"$gt":bson::DateTime::now()},
            "$or":[{"execution_binding":null},{"execution_binding":&self.binding}],
        }, doc! {"$set":fields}).await?;
        if updated.matched_count != 1 {
            return Err(denied());
        }
        Ok(())
    }
}
