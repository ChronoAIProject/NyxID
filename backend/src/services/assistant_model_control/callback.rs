use super::*;

/// Only this module can construct a callback's confirmation replacement. It
/// never bypasses connection selection, grants, generic Deny or final fencing.
pub(crate) struct Callback {
    pub(super) root: ModelControlRoot,
    pub(super) private: RootPrivate,
    pub(super) candidate: wire::PrepareRequest,
    pub(super) prepared: Option<PreparedDispatch>,
    pub(crate) read_observation: bool,
}
impl Callback {
    pub(crate) fn operation(&self) -> Value {
        json!({"kind":"legacy_inference","protocol":wire::PROTOCOL,"root_id":self.root.id,
            "envelope":digest(serde_json::to_value(&self.private.envelope).unwrap_or(Value::Null)),
            "method":"POST","path":self.candidate.relative_path,"query":self.candidate.query,
            "model":self.candidate.body["model"],"provider":self.private.catalog_id})
    }
    pub(crate) fn prepared(&self) -> Option<PreparedDispatch> {
        self.prepared.clone()
    }
}

pub(crate) async fn require_control_lineage(
    state: &AppState,
    chat: &acks::ChatAuthority,
) -> AppResult<bool> {
    Ok(state.db.collection::<ModelControlRoot>(COLLECTION_NAME).find_one(doc! {
        "user_id":&chat.user_id,"conversation_id":&chat.conversation_id,"api_key_id":&chat.api_key_id,
        "turn_ids":&chat.turn_id,"phase":{"$ne":"unactivated"},
    }).await?.is_some())
}

/// Missing control is relevant to the saved model transport, independently of
/// mutable inference metadata. Unrelated HTTP effects retain their own gate.
pub(crate) async fn require_http_control_lineage(
    state: &AppState,
    chat: &acks::ChatAuthority,
    connection: Option<&str>,
    target: &super::super::proxy_service::ProxyTarget,
    path: &str,
) -> AppResult<bool> {
    let path = path.trim_matches('/');
    let path = path.strip_prefix("v1/").unwrap_or(path);
    if chat.guest
        || !matches!(
            path,
            "responses" | "responses/compact" | "chat/completions" | "messages" | "embeddings"
        )
    {
        return Ok(false);
    }
    if target.service.inference.is_some() {
        return require_control_lineage(state, chat).await;
    }
    let mut related = vec![doc! {"card_arguments.catalog_id":&target.service.id}];
    if let Some(connection) = connection {
        related.push(doc! {"card_arguments.connection_id":connection});
    }
    Ok(state.db.collection::<ModelControlRoot>(COLLECTION_NAME).find_one(doc! {
        "user_id":&chat.user_id,"conversation_id":&chat.conversation_id,"api_key_id":&chat.api_key_id,
        "turn_ids":&chat.turn_id,"phase":{"$ne":"unactivated"},"$or":related,
    }).await?.is_some())
}

pub(crate) struct CallbackRequest<'a> {
    pub chat: &'a acks::ChatAuthority,
    pub connection: &'a str,
    pub owner: &'a str,
    pub target: &'a super::super::proxy_service::ProxyTarget,
    pub method: &'a str,
    pub path: &'a str,
    pub query: Option<&'a str>,
    pub body: &'a [u8],
    pub headers: &'a axum::http::HeaderMap,
    pub semantic_headers: Vec<(String, String)>,
    /// False for translated senders that do not use the final claim/observer.
    pub instrumented_transport: bool,
}

fn effective_header_binding(mut headers: Vec<(String, String)>) -> String {
    for (name, _) in &mut headers {
        name.make_ascii_lowercase();
    }
    // Preserve duplicate value order: the real merger uses it when several
    // values share a name. Sorting by name alone makes map order irrelevant.
    headers.sort_by(|a, b| a.0.cmp(&b.0));
    digest(json!(headers))
}

fn single_header<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> AppResult<Option<&'a str>> {
    let mut all = headers.get_all(name).iter();
    let value = all
        .next()
        .map(|h| h.to_str().map_err(|_| invalid()))
        .transpose()?;
    if all.next().is_some() {
        return Err(invalid());
    }
    Ok(value)
}

/// Ineligible transport retains its pre-negotiation contract only with a
/// verified unactivated root and no step. This never creates reusable consent.
pub(crate) async fn require_unactivated_exact_only(
    state: &AppState,
    chat: &acks::ChatAuthority,
    headers: &axum::http::HeaderMap,
) -> AppResult<()> {
    let raw = single_header(headers, wire::CONTROL_HEADER)?;
    if single_header(headers, wire::STEP_HEADER)?.is_some() {
        return Err(invalid());
    }
    if let Some(raw) = raw {
        let token = wire::SecretToken::parse(raw.into()).map_err(|_| invalid())?;
        let (row, _, _) = persistence::authenticated(state, chat, &token, None).await?;
        if row.phase != "unactivated" {
            return Err(invalid());
        }
    } else if require_control_lineage(state, chat).await? {
        return Err(invalid());
    }
    Ok(())
}

pub(crate) async fn callback(
    state: &AppState,
    request: CallbackRequest<'_>,
) -> AppResult<Option<Callback>> {
    let CallbackRequest {
        chat,
        connection,
        owner,
        target,
        method,
        path,
        query,
        body,
        headers,
        semantic_headers,
        instrumented_transport,
    } = request;
    let step_header = single_header(headers, wire::STEP_HEADER)?;
    let raw = single_header(headers, wire::CONTROL_HEADER)?;
    if chat.guest {
        return if raw.is_some() || step_header.is_some() {
            Err(invalid())
        } else {
            Ok(None)
        };
    }
    let Some(raw) = raw else {
        if step_header.is_some() {
            return Err(invalid());
        }
        if require_http_control_lineage(state, chat, Some(connection), target, path).await? {
            return Err(invalid());
        }
        return Ok(None);
    };
    let token = wire::SecretToken::parse(raw.into()).map_err(|_| invalid())?;
    let (row, mut private, _) = persistence::authenticated(state, chat, &token, None).await?;
    let path = path.trim_matches('/');
    let path = path.strip_prefix("v1/").unwrap_or(path);
    // Negotiation alone must not change legacy exact-only callbacks. Validate
    // the control even on an unsupported shape, and never accept a step before
    // activation. Once activated, every callback stays inside the protocol.
    if row.phase == "unactivated" && step_header.is_some() {
        return Err(invalid());
    }
    if target.service.inference.is_none()
        || !instrumented_transport
        || !matches!(
            target.auth_method.as_str(),
            "none" | "header" | "bearer" | "basic" | "query"
        )
        || method != "POST"
        || !matches!(path, "responses" | "responses/compact")
        || (path == "responses/compact" && row.phase == "unactivated" && private.preconsent.compact.is_none())
    {
        if row.phase == "unactivated" {
            // Share the admission marker's positive model-path/related-target
            // scope. Unsupported model sends cannot look like pristine history;
            // unrelated effects keep their ordinary contract.
            read_observation::unobserved_http(state,chat,Some(connection),target,path).await?;
        }
        return exact_only_before_activation(&row);
    }
    if body.len() > wire::MAX_PREPARE_BYTES {
        if row.phase == "unactivated" { read_observation::disable(state, chat, &row.id).await?; }
        return exact_only_before_activation(&row);
    }
    let body = match serde_json::from_slice::<Value>(body) {
        Ok(body) => body,
        Err(_) => {
            if row.phase == "unactivated" { read_observation::disable(state, chat, &row.id).await?; }
            return exact_only_before_activation(&row);
        }
    };
    let candidate = wire::PrepareRequest {
        protocol: wire::PROTOCOL.into(),
        relative_path: path.into(),
        query: query.map(str::to_owned),
        semantic_headers: super::super::proxy_service::collect_forward_headers_with_defaults(
            headers,
            &[],
        ),
        body,
        generation_proposal: None,
        preceding_receipt: None,
        tool_results: vec![],
    };
    if row.phase == "unactivated"
        && super::super::assistant_service_execution::stateless_inference(
            "responses",
            &candidate.body,
        )
    {
        let binding = super::super::assistant_service_execution::connection_binding(&state.db, connection, &chat.api_key_id).await?;
        let prepared = read_observation::prepare(state, chat, &token, &row, &candidate,
            read_observation::Authority { connection:connection.into(), binding,
                owner:owner.into(), catalog:target.service.id.clone(), headers:effective_header_binding(semantic_headers) }).await?;
        return Ok(prepared.map(|prepared| Callback { root:row, private, candidate, prepared:Some(prepared), read_observation:true }));
    }
    if row.phase == "unactivated" && path == "responses" && private.preconsent.seen {
        // A new generation card cannot erase known prior Read/tool history.
        // Preserve its legacy exact approval rather than invent a fresh root.
        read_observation::disable(state, chat, &row.id).await?;
        return Ok(None);
    }
    let parsed = if row.phase == "unactivated" && path == "responses/compact" {
        let saved = private.preconsent.compact.as_ref().ok_or_else(conflict)?;
        envelope::within_family(&saved.envelope, &candidate)
            .map(|(operation,input)| (saved.envelope.clone(),input,operation))
    } else { match private
        .envelope
        .as_ref()
        .filter(|_| row.phase != "unactivated")
    {
        Some(saved) => envelope::within_family(saved, &candidate)
            .map(|(operation, input)| (saved.clone(), input, operation)),
        None => envelope::envelope(&candidate)
            .map(|(envelope, input)| (envelope, input, StepOperation::Generation)),
    }};
    let (envelope, input, operation) = match parsed {
        Ok(candidate) => candidate,
        Err(_) if row.phase == "unactivated" && path != "responses/compact" => {
            read_observation::disable(state, chat, &row.id).await?;
            return Ok(None);
        }
        Err(error) => return Err(error),
    };
    if matches!(row.phase.as_str(), "denied" | "revoked") {
        return Err(invalid());
    }
    let binding = super::super::assistant_service_execution::connection_binding(
        &state.db,
        connection,
        &chat.api_key_id,
    )
    .await?;
    if row.phase == "unactivated" && path == "responses/compact" {
        initial_compact::callback(&private, &token, &candidate, &read_observation::Authority {
            connection:connection.into(),binding:binding.clone(),owner:owner.into(),catalog:target.service.id.clone(),
            headers:effective_header_binding(semantic_headers.clone()),
        })?;
    }
    let prepared = if row.phase == "unactivated" {
        if step_header.is_some() {
            return Err(invalid());
        }
        private.envelope = Some(envelope);
        private.initial_input = input;
        private.initial_operation = operation;
        private.initial_request_binding = Some(envelope::request_binding(&candidate));
        private.connection_id = Some(connection.into());
        private.connection_binding = Some(binding);
        private.owner_id = Some(owner.into());
        private.catalog_id = Some(target.service.id.clone());
        private.effective_headers_binding = Some(effective_header_binding(semantic_headers));
        None
    } else {
        if private.envelope.as_ref() != Some(&envelope)
            || private.connection_id.as_deref() != Some(connection)
            || private.owner_id.as_deref() != Some(owner)
            || private.connection_binding.as_deref() != Some(binding.as_str())
            || private.catalog_id.as_deref() != Some(target.service.id.as_str())
            || private.effective_headers_binding.as_deref()
                != Some(effective_header_binding(semantic_headers).as_str())
        {
            return Err(conflict());
        }
        if row.phase == "awaiting_decision" {
            return Ok(Some(Callback {
                root: row,
                private,
                candidate,
                prepared: None,
                read_observation: false,
            }));
        }
        let raw = step_header.ok_or_else(invalid)?;
        let step = private
            .steps
            .last()
            .filter(|s| {
                s.capability_hash == token_hash(raw)
                    && s.request_binding == envelope::request_binding(&candidate)
                    && s.operation == operation
                    && matches!(s.phase, StepPhase::Prepared)
            })
            .ok_or_else(conflict)?;
        Some(PreparedDispatch {
            state: state.clone(),
            root_id: row.id.clone(),
            step_id: step.id.clone(),
            request_binding: step.request_binding.clone(),
            control_hash: token_hash(token.expose()),
            step_hash: step.capability_hash.clone(),
            chat: chat.clone(),
            connection_id: connection.into(),
            connection_binding: binding,
            operation,
            read_observation: false,
        })
    };
    Ok(Some(Callback {
        root: row,
        private,
        candidate,
        prepared,
        read_observation: false,
    }))
}

fn exact_only_before_activation(row: &ModelControlRoot) -> AppResult<Option<Callback>> {
    if row.phase == "unactivated" {
        Ok(None)
    } else {
        Err(unsupported())
    }
}

pub(crate) async fn observe(
    state: &AppState,
    chat: &acks::ChatAuthority,
    callback: &Callback,
) -> AppResult<Value> {
    if callback.root.phase == "awaiting_decision" {
        let card = state.db.collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>("assistant_acknowledgements")
            .find_one(doc! {"_id":&callback.root.acknowledgement_id,"user_id":&chat.user_id,"conversation_id":&chat.conversation_id}).await?.ok_or_else(conflict)?;
        return Ok(acks::refusal(&card));
    }
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run2(async |session| {
        let result = async {
            let (mut row, mut private) = persistence::load(state, &callback.root.id, Some(&mut *session)).await?;
            persistence::require_row(&row, chat)?;
            if row.phase != "unactivated" || row.revision != callback.root.revision { return Err(conflict()); }
            private.envelope = callback.private.envelope.clone();
            private.initial_input = callback.private.initial_input.clone();
            private.initial_operation = callback.private.initial_operation;
            private.initial_request_binding = callback.private.initial_request_binding.clone();
            private.connection_id = callback.private.connection_id.clone();
            private.connection_binding = callback.private.connection_binding.clone();
            private.owner_id = callback.private.owner_id.clone();
            private.catalog_id = callback.private.catalog_id.clone();
            private.effective_headers_binding = callback.private.effective_headers_binding.clone();
            let args = json!({"protocol":wire::PROTOCOL,"root_id":row.id,
                "connection_id":private.connection_id,"catalog_id":private.catalog_id,"binding":private.connection_binding,
                "envelope":digest(serde_json::to_value(&private.envelope).map_err(|_| invalid())?),
                "initial_operation":private.initial_operation,"initial_request":private.initial_request_binding});
            let connection=state.db.collection::<crate::models::user_service::UserService>("user_services")
                .find_one(doc!{"_id":&private.connection_id,"user_id":&private.owner_id})
                .session(&mut *session).await?.ok_or_else(invalid)?;
            let label=super::super::assistant_nyxagent::identifier(&connection.slug);
            let model=super::super::assistant_nyxagent::identifier(callback.candidate.body["model"].as_str().ok_or_else(invalid)?);
            let cause=super::super::assistant_service_execution::inference_fallback_reason(&callback.candidate.relative_path,&callback.candidate.body)
                .ok_or_else(invalid)?;
            let initial=if private.initial_operation==StepOperation::Compact {
                "This initial consent binds the complete proposed generation configuration and this exact compact input before either authorized operation. Any known earlier Read output and host-tool results have been reconciled; caller-supplied initial history does not prove past tool execution."
            } else {"The first authorized generation preserves all exact preceding input items and replaces only the final user item with saved Allow narration."};
            let summary=format!("Connection {label} ({}), model {model}. {} Approval covers this Responses generation and standalone compact family: exact generation configuration and declared host tools stay fixed; compact sends the same model and exact instructions, with no generation-only options or tools. {initial} Approval includes the same exact opaque generation metadata options; they are not treated as automatic Read. Only server-observed output and genuine host-tool results may advance subsequent history. Complete observed compact output must replace the entire next input without pruning. Changed configuration, metadata or regenerated native context requires separate review.",connection.id,cause.reusable_message());
            let mut card = acks::request_in_session(&state.db, chat, acks::Request {
                kind:"action",service:Some((&connection.id,&label,&label)),tool:Some("nyxid.model_control_v1"),arguments:Some(&args),
                summary:&summary,platform:false,
            }, None, session).await?;
            // This is always explicit owner Write, regardless of an automatic
            // Read policy or legacy generic-approval exemption.
            let confirmation = crate::models::assistant_acknowledgement::ActionConfirmation {
                    risk:"change".into(), access_mode:Some(acks::effective_access_mode(&state.db, chat).await?), reason:"policy".into(),
                };
            state.db.collection::<Document>("assistant_acknowledgements").update_one(doc! {"_id":&card.id},
                doc! {"$set":{"confirmation":bson::to_bson(&confirmation).map_err(|_| invalid())?}}).session(&mut *session).await?;
            card.confirmation = Some(confirmation);
            row.acknowledgement_id = Some(card.id.clone());
            row.card_arguments = Some(args.clone());
            row.phase = "awaiting_decision".into();
            private.card_arguments = Some(args);
            persistence::save(state, &mut row, &private, session).await?;
            Ok(acks::refusal(&card))
        }.await;
        super::super::api_key_mutation_service::transaction_result(result)
    }).await.map_err(super::super::api_key_mutation_service::map_transaction_error)
}

/// Joins the original decision transaction. Old exact-body cards are never
/// adopted; only a new root's exact private card digest can change its decision.
pub(crate) async fn decision_in_session(
    db: &Database,
    row: &crate::models::assistant_acknowledgement::AssistantAcknowledgement,
    session: &mut ClientSession,
) -> AppResult<()> {
    if row.tool_name.as_deref() != Some("nyxid.model_control_v1") {
        return Ok(());
    }
    if row.decider != "user"
        || row.decided_by.as_deref() != Some("user")
        || !matches!(row.status.as_str(), "allowed" | "denied")
    {
        return Err(invalid());
    }
    // Root/card association was committed with card creation. Status transition
    // and durable decision live in this same transaction, including crash paths.
    let collection = db.collection::<ModelControlRoot>(COLLECTION_NAME);
    let root = collection.find_one(doc! {
        "acknowledgement_id":&row.id,"user_id":&row.user_id,"conversation_id":&row.conversation_id,"api_key_id":&row.api_key_id,
    }).session(&mut *session).await?.ok_or_else(invalid)?;
    let saved = root.card_arguments.as_ref().ok_or_else(invalid)?;
    if saved["root_id"] != root.id
        // Pending cards from the retired wire revision may still be Denied.
        // Allow cannot migrate their saved consent into the new protocol.
        || !(saved["protocol"] == wire::PROTOCOL
            || (row.status == "denied" && saved["protocol"].as_str().is_some_and(wire::is_retired)))
        || row.arguments_digest.as_deref() != Some(acks::arguments_digest(saved).as_str())
    {
        return Err(invalid());
    }
    let changed = collection.update_one(doc! {
        "_id":&root.id,"revision":root.revision,
        "acknowledgement_id":&row.id,"user_id":&row.user_id,"conversation_id":&row.conversation_id,
        "api_key_id":&row.api_key_id,"phase":"awaiting_decision","expires_at":{"$gt":bson::DateTime::now()},
    }, doc! {"$set":{"phase":if row.status == "allowed" {"allowed"} else {"denied"},
        // Tombstones remain durable; use expires no later than the saved card.
        "expires_at":bson::DateTime::from_chrono(root.expires_at.min(row.expires_at))},"$inc":{"revision":1}}).session(session).await?;
    if changed.modified_count != 1 {
        return Err(conflict());
    }
    Ok(())
}
