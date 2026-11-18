use super::*;

pub(crate) fn eligible(context: &RequestContext, chat: Option<&acks::ChatAuthority>) -> bool {
    if matches!(context.ingress.get(), Some(PreparedAdapter::Unsupported)) {
        return false;
    }
    if !context.authoring_allowed()
        || matches!(
            context.descendant.as_deref(),
            Some(descendants::Descendant::Job(_))
        ) && context.machine.get().is_none()
        || chat.is_some_and(|c| {
            c.guest
                || c.execution_unsupported
                || c.platform_thread
                || c.platform_origin.is_some()
                || c.channel_adapter_origin.is_some()
                || c.confirmation_policy.is_some()
        })
    {
        return false;
    }
    if context.machine.get().is_some() {
        return matches!(context.ingress.get(), Some(PreparedAdapter::Machine { .. }))
            && chat.is_some_and(|c| c.execution_task.is_some());
    }
    if matches!(
        context.descendant.as_deref(),
        Some(descendants::Descendant::Watch(_))
    ) {
        return matches!(context.ingress.get(), Some(PreparedAdapter::Watch { .. }));
    }
    if context.native.is_some() {
        return matches!(context.ingress.get(), Some(PreparedAdapter::Native { .. }));
    }
    if context.voice_task {
        return chat.is_some_and(|c| c.turn_live && !c.turn_stopped && c.execution_task.is_some());
    }
    chat.is_none() && matches!(context.principal, Principal::ApiKey(_))
}

pub(super) async fn source(
    db: &Database,
    context: &RequestContext,
    chat: Option<&acks::ChatAuthority>,
) -> AppResult<Option<ProposalSource>> {
    if let Some(machine) = context.machine.get() {
        return Ok(Some(ProposalSource::Machine {
            binding: Box::new(machine.binding.clone()),
        }));
    }
    if let Some(native) = &context.native {
        return Ok(Some(native.source()));
    }
    if let Some(descendants::Descendant::Watch(w)) = context.descendant.as_deref() {
        let Some(PreparedAdapter::Watch { operation }) = context.ingress.get() else {
            return Err(refused());
        };
        return Ok(Some(ProposalSource::Watch {
            watch_id: w.id.clone(),
            operation: operation.clone(),
            operation_id: crate::services::cma_operation::poll_id(w)
                .ok_or_else(refused)?
                .into(),
        }));
    }
    if context.voice_task {
        let c = chat.ok_or_else(refused)?;
        let row=db.collection::<AssistantConversation>("assistant_conversations").find_one(doc!{"_id":&c.conversation_id,"user_id":&c.user_id,"active_turn.turn_id":&c.turn_id}).await?.ok_or_else(refused)?;
        let request_id = row
            .active_turn
            .and_then(|t| t.voice_request_id)
            .ok_or_else(refused)?;
        if db.collection::<Document>(crate::models::assistant_voice::REQUESTS).find_one(doc!{"_id":&request_id,"user_id":&context.actor,"execution_enrolled":true,"task_conversation_id":&c.conversation_id}).await?.is_none(){return Ok(None);}
        return Ok(Some(ProposalSource::Voice { request_id }));
    }
    Ok(Some(ProposalSource::External {
        pool: match context.ingress.get() {
            Some(PreparedAdapter::Pool { binding }) => Some(binding.clone()),
            _ => None,
        },
    }))
}

pub(super) fn adapter_identity(adapter: &PreparedAdapter) -> serde_json::Value {
    match adapter {
        PreparedAdapter::Machine { binding, .. } => json!({"adapter":"machine","binding":binding}),
        PreparedAdapter::Unsupported => json!({"adapter":"unsupported"}),
        PreparedAdapter::Http => json!({"adapter":"http"}),
        PreparedAdapter::Pool { binding } => json!({"adapter":"pool","binding":binding}),
        PreparedAdapter::Llm {
            provider,
            path,
            method,
            query,
            body,
            headers,
        } => {
            json!({"adapter":"llm","compiler":1,"provider":provider,"path":path,"method":method,"query":query,"body":body,"headers":headers})
        }
        PreparedAdapter::Native {
            uri,
            method,
            body,
            content_type,
            mcp,
        } => {
            json!({"adapter":"native","uri":uri,"method":method,"body":body,"content_type":content_type,"mcp":mcp})
        }
        PreparedAdapter::Watch { operation } => json!({"adapter":"watch","operation":operation}),
    }
}

pub(super) fn locator(
    context: &RequestContext,
    chat: Option<&acks::ChatAuthority>,
) -> AppResult<String> {
    if let Some(machine) = context.machine.get() {
        return fingerprint(
            &json!({"version":2,"actor":context.actor,"principal":context.principal,"key":chat.map(|c|&c.api_key_id),"machine":machine.binding,"occurrence":context.idempotency.get().unwrap_or(&context.operation_id)}),
        );
    }
    fingerprint(
        &json!({"version":2,"actor":context.actor,"principal":context.principal,"key":chat.map(|c|&c.api_key_id),"child":context.source_credential.get(),"occurrence":context.idempotency.get().unwrap_or(&context.operation_id)}),
    )
}

pub(in crate::services::execution_permission_dispatch) async fn existing(
    state: &crate::AppState,
    context: &RequestContext,
    chat: Option<&acks::ChatAuthority>,
    effect: &Effect<'_>,
    digest: &str,
    authority: &authority::Snapshot,
) -> AppResult<()> {
    let db = &state.db;
    let locator = locator(context, chat)?;
    if let Some(row) = db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! {"execution_proposal.version":2,"execution_proposal.v2.locator":locator})
        .await?
    {
        let p = proposal(&row)?;
        let source = source(db, context, chat).await?.ok_or_else(refused)?;
        let adapter = context
            .ingress
            .get()
            .cloned()
            .unwrap_or(PreparedAdapter::Http);
        let source_digest = fingerprint(
            &json!({"source":source,"task":chat.and_then(|c|c.execution_task.as_ref()),"key":p.key_id,"child":context.source_credential.get(),"subject":p.contract.subject,"audience":p.contract.context,"adapter":adapter_identity(&adapter)}),
        )?;
        if p.prepared_digest != digest
            || p.authority_digest != authority.digest()
            || p.contract.binding.connection_id != effect.connection
            || source_digest != p.source_digest
        {
            return Err(conflict());
        }
        return Err(pending(&row, &state.config.frontend_url));
    }
    Ok(())
}

pub(super) async fn fence_source(
    db: &Database,
    session: &mut ClientSession,
    row: &ApprovalRequest,
) -> AppResult<()> {
    let p = proposal(row)?;
    if row.expires_at <= Utc::now() || p.delivery_abandoned {
        return Err(refused());
    }
    if let Some(task) = &p.task {
        tasks::fence(db, session, task).await?;
    }
    // A person must still exist to decide: no service principal can approve itself.
    if db
        .collection::<Document>("users")
        .update_one(
            doc! {"_id":&row.user_id,"is_active":true,"$or":[{"user_type":"person"},{"user_type":{"$exists":false}}]},
            doc! {"$inc":{"execution_dispatch_fence":1_i64}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(refused());
    }
    if let Some(id) = &p.endpoint_digest {
        let endpoint = p
            .contract
            .operations
            .operations
            .first()
            .and_then(|o| o.endpoint_id.as_ref())
            .ok_or_else(refused)?;
        let current = db
            .collection::<Document>("service_endpoints")
            .find_one(doc! {"_id":endpoint})
            .session(&mut *session)
            .await?
            .ok_or_else(refused)?;
        if endpoint_digest(&current)? != *id {
            return Err(refused());
        }
        if db
            .collection::<Document>("service_endpoints")
            .update_one(
                doc! {"_id":endpoint},
                doc! {"$inc":{"execution_dispatch_fence":1_i64}},
            )
            .session(&mut *session)
            .await?
            .matched_count
            != 1
        {
            return Err(refused());
        }
    }
    let binding = match &p.source {
        ProposalSource::Machine { binding } => {
            if binding.actor_id != row.user_id || binding.key_id != p.key_id {return Err(refused());}
            binding.fence(db,session,p.task.as_ref().ok_or_else(refused)?).await?; None
        },
        ProposalSource::External { pool } => pool.as_ref().map(|pool| (
            "service_pools", doc! {"_id":&pool.id,"user_id":&pool.owner_id,"config_revision":pool.revision,"is_active":true,"members":{"$elemMatch":{"user_service_id":&p.contract.binding.connection_id,"enabled":true}}},
        )),
        ProposalSource::Native {
            policy_id,
            revision,
        } => Some((
            crate::models::permission_policy::COLLECTION_NAME,
            doc! {"_id":policy_id,"user_id":&row.user_id,"revision":*revision,"paused":false},
        )),
        ProposalSource::Voice { request_id } => Some((
            crate::models::assistant_voice::REQUESTS,
            doc! {"_id":request_id,"user_id":&row.user_id,"execution_enrolled":true,"state":{"$ne":"cancelled"},"task_conversation_id":&p.conversation_id},
        )),
        ProposalSource::Watch {
            watch_id,
            operation,
            operation_id,
        } => {
            let w = db
                .collection::<crate::models::async_service_operation::AsyncServiceOperation>(
                    "assistant_async_operations",
                )
                .find_one(doc! {"_id":watch_id,"user_id":&row.user_id,"api_key_id":&p.key_id})
                .session(&mut *session)
                .await?
                .ok_or_else(refused)?;
            if crate::services::cma_operation::poll_id(&w) != Some(operation_id.as_str())
                || !matches!(w.state.as_str(), "waiting" | "cancelling")
                || (w.state == "cancelling"
                    && Some(operation) != w.contract.cancel_operation.as_ref())
                || (operation != &w.contract.status_operation
                    && operation != &w.contract.result_operation
                    && Some(operation) != w.contract.cancel_operation.as_ref())
            {
                return Err(refused());
            }
            Some((
                "assistant_async_operations",
                doc! {"_id":watch_id,"state":w.state,"user_id":&row.user_id,"api_key_id":&p.key_id,"contract_binding":w.contract_binding,"expires_at":{"$gt":bson::DateTime::now()}},
            ))
        }
    };
    if let Some((collection, filter)) = binding
        && db
            .collection::<Document>(collection)
            .update_one(filter, doc! {"$inc":{"execution_dispatch_fence":1_i64}})
            .session(&mut *session)
            .await?
            .matched_count
            != 1
    {
        return Err(refused());
    }
    if let Some(conversation) = &p.conversation_id {
        let c = db
            .collection::<AssistantConversation>("assistant_conversations")
            .find_one(
                doc! {"_id":conversation,"user_id":&row.user_id,"credential_api_key_id":&p.key_id},
            )
            .session(&mut *session)
            .await?
            .ok_or_else(refused)?;
        crate::services::assistant_agent_credential_service::authority_in_session(db, &c, session)
            .await?;
        if let Some(expected) = &p.contract.binding.behavior_digest {
            let agent = c.agent_id.as_ref().ok_or_else(refused)?;
            let a = db
                .collection::<crate::models::assistant_agent::AssistantAgent>("assistant_agents")
                .find_one(doc! {"_id":agent,"destroyed_at":null})
                .session(&mut *session)
                .await?
                .ok_or_else(refused)?;
            let policy = db
                .collection::<crate::models::assistant_sharing::AgentSharing>(
                    crate::models::assistant_sharing::COLLECTION_NAME,
                )
                .find_one(doc! {"_id":agent})
                .session(&mut *session)
                .await?
                .ok_or_else(refused)?;
            if !policy.enabled {
                return Err(refused());
            }
            if db
                .collection::<Document>("assistant_agents")
                .update_one(
                    doc! {"_id":agent},
                    doc! {"$inc":{"execution_dispatch_fence":1_i64}},
                )
                .session(&mut *session)
                .await?
                .matched_count
                != 1
            {
                return Err(refused());
            }
            if crate::services::shared_agent_consent::behavior_for_agent(&a, policy.revision)
                != *expected
            {
                return Err(refused());
            }
        }
        let current_audience = if let Some(id) = c
            .collaboration
            .as_ref()
            .map(|b| &b.group_id)
            .or(c.group_id.as_ref())
        {
            let group = db
                .collection::<crate::models::assistant_group::AssistantGroup>("assistant_groups")
                .find_one(doc! {"_id":id})
                .session(&mut *session)
                .await?
                .ok_or_else(refused)?;
            if db
                .collection::<Document>("assistant_groups")
                .update_one(
                    doc! {"_id":id},
                    doc! {"$inc":{"execution_dispatch_fence":1_i64}},
                )
                .session(&mut *session)
                .await?
                .matched_count
                != 1
            {
                return Err(refused());
            }
            group_audience(&group)
        } else {
            Audience::Private {
                user_id: row.user_id.clone(),
            }
        };
        if current_audience != p.contract.context.audience {
            return Err(refused());
        }
        if c.collaboration.is_some() {
            crate::services::collaboration_group_service::runtime::fence_decision(
                db,
                &c,
                &row.user_id,
                session,
            )
            .await?;
        }
    }
    Ok(())
}

pub(super) async fn review_v2(
    db: &Database,
    context: &RequestContext,
    chat: Option<&acks::ChatAuthority>,
    effect: &Effect<'_>,
    reusable: bool,
    task: &Option<TaskAdmission>,
    gaps: &[contract::Gap],
) -> AppResult<Review> {
    if let Some(chat) = chat {
        return super::super::review(
            db,
            chat,
            effect,
            reusable,
            context.candidates(db, effect.connection).await?,
            gaps,
        )
        .await;
    }
    let connection = db
        .collection::<crate::models::user_service::UserService>("user_services")
        .find_one(doc! {"_id":effect.connection,"user_id":effect.owner})
        .await?
        .ok_or_else(refused)?;
    let query = url::form_urlencoded::parse(effect.query.unwrap_or("").as_bytes())
        .map(|(k, v)| {
            let value = if secret_key(&k) {
                "[redacted]".into()
            } else {
                v.to_string()
            };
            (k.to_string(), value)
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let headers = effect
        .headers
        .iter()
        .map(|(k, v)| {
            (
                k,
                if secret_key(k) {
                    "[redacted]"
                } else {
                    v.as_str()
                },
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut inputs = json!({"query":query,"headers":headers,"body":serde_json::from_slice::<serde_json::Value>(effect.body).unwrap_or_default()});
    redact(&mut inputs);
    Ok(Review {
        agent: "API key request".into(),
        connection: label(&connection.slug),
        operation: format!("{} {}", effect.method, label(effect.path)),
        audience: "Only this authenticated caller".into(),
        scope: if reusable {
            "This exact path, query and JSON input for this API key and audience"
        } else {
            "Only this retained operation with its exact inputs"
        }
        .into(),
        inputs: inputs.to_string().chars().take(12000).collect(),
        existing_permission: existing_permissions(
            context.candidates(db, effect.connection).await?,
            gaps,
        ),
        added_permission: effect
            .floors
            .iter()
            .map(|f| format!("{f:?}"))
            .collect::<Vec<_>>()
            .join(", "),
        reusable_unavailable: if context.source_credential.get().is_some() {
            Some("Login-child credentials support only this operation".into())
        } else if !reusable {
            Some("Only this operation is available for these inputs".into())
        } else if task.is_none() {
            Some("Task is unavailable: this request has no server-owned task".into())
        } else {
            None
        },
    })
}

/// Pin retries before pool ordering. The normal coordinator still compares the
/// exact effect/source digest, so a locator cannot select a different operation.
pub(crate) async fn pin_pool(
    db: &Database,
    auth: &crate::mw::auth::AuthUser,
    pool: &crate::models::service_pool::ServicePool,
    headers: &axum::http::HeaderMap,
) -> AppResult<Option<String>> {
    let Some(context) = auth
        .execution_permission
        .as_ref()
        .filter(|_| auth.assistant_chat.is_none())
    else {
        return Ok(None);
    };
    if pool.member_contract == crate::models::service_pool::PoolMemberContract::AiChat {
        tracing::info!(
            reason = "pool_transform_not_retainable",
            "Execution proposal uses legacy prompts"
        );
        context.retain_adapter(PreparedAdapter::Unsupported, headers);
        return Ok(None);
    }
    context.retain_adapter(
        PreparedAdapter::Pool {
            binding: crate::models::execution_proposal::ProposalPool {
                id: pool.id.clone(),
                owner_id: pool.user_id.clone(),
                revision: pool.config_revision,
            },
        },
        headers,
    );
    if !eligible(context, None) {
        return Ok(None);
    }
    let locator = locator(context, None)?;
    let Some(row) = db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! {"execution_proposal.version":2,"execution_proposal.v2.locator":locator})
        .await?
    else {
        return Ok(None);
    };
    let p = proposal(&row)?;
    if !matches!(&p.source,ProposalSource::External {pool:Some(binding)} if binding.id==pool.id && binding.owner_id==pool.user_id && binding.revision==pool.config_revision)
    {
        return Err(conflict());
    }
    Ok(Some(p.contract.binding.connection_id.clone()))
}
