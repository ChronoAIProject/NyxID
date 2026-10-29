use super::*;

/// Deterministic server-owned provider translation applied to both the held
/// target and its fresh authoritative resolution. Never constructed from input.
pub struct HttpProjection {
    pub apply_agent_override: bool,
    pub replace_headers: bool,
    pub base_url: Option<String>,
    pub extra_headers: Vec<(String, String)>,
    pub(crate) machine_git: Option<crate::models::downstream_service::GitHttp>,
}
impl Default for HttpProjection {
    fn default() -> Self {
        Self {
            apply_agent_override: true,
            replace_headers: false,
            base_url: None,
            extra_headers: Vec::new(),
            machine_git: None,
        }
    }
}
impl HttpProjection {
    pub(crate) fn for_machine(
        ingress: Option<&crate::services::machine_gateway_service::Ingress>,
    ) -> Self {
        Self {
            machine_git: ingress.and_then(|ingress| ingress.git.clone()),
            ..Self::default()
        }
    }

    pub fn apply(&self, target: &mut crate::services::proxy_service::ProxyTarget) {
        if let Some(base) = &self.base_url {
            target.base_url.clone_from(base);
            target.target_id = None;
            target.workspace_destinations_pending = false;
        }
    }
    pub(super) fn headers(
        &self,
        target: &crate::services::proxy_service::ProxyTarget,
        headers: &axum::http::HeaderMap,
    ) -> Vec<(String, String)> {
        if self.replace_headers {
            return self.extra_headers.clone();
        }
        let mut headers = crate::services::proxy_service::collect_forward_headers_with_defaults(
            headers,
            &[
                target.catalog_default_headers.as_slice(),
                target.user_service_default_headers.as_slice(),
            ],
        );
        for (name, value) in &self.extra_headers {
            headers.retain(|(existing, _)| !existing.eq_ignore_ascii_case(name));
            headers.push((name.clone(), value.clone()));
        }
        headers
    }
}

/// Compare the held concrete execution with a newly resolved exact connection.
/// Reading current rows alone cannot authorize a stale already-resolved target.
#[allow(clippy::too_many_arguments)]
pub fn verify_resolved<'a>(
    db: &'a Database,
    keys: &'a crate::crypto::aes::EncryptionKeys,
    chat: &'a acks::ChatAuthority,
    id: &'a str,
    actual: &'a crate::services::proxy_service::ProxyTarget,
    method: &'a str,
    path: &'a str,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(async move {
        verify_resolved_projected(
            db,
            keys,
            chat,
            id,
            actual,
            method,
            path,
            &HttpProjection::default(),
        )
        .await
    })
}

#[allow(clippy::too_many_arguments)]
pub fn verify_resolved_projected<'a>(
    db: &'a Database,
    keys: &'a crate::crypto::aes::EncryptionKeys,
    chat: &'a acks::ChatAuthority,
    id: &'a str,
    actual: &'a crate::services::proxy_service::ProxyTarget,
    method: &'a str,
    path: &'a str,
    projection: &'a HttpProjection,
) -> futures::future::BoxFuture<'a, AppResult<()>> {
    Box::pin(verify_resolved_projected_inner(
        db, keys, chat, id, actual, method, path, projection,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn verify_resolved_projected_inner(
    db: &Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    chat: &acks::ChatAuthority,
    id: &str,
    actual: &crate::services::proxy_service::ProxyTarget,
    method: &str,
    path: &str,
    projection: &HttpProjection,
) -> AppResult<()> {
    let before = connection_binding(db, id, &chat.api_key_id).await?;
    let mut fresh = crate::services::proxy_service::resolve_proxy_target_by_user_service_id(
        db,
        keys,
        &chat.user_id,
        id,
        None,
        None,
        crate::services::proxy_service::ProxyExecutionContext::new(
            None,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
        )
        .with_assistant_access(chat.org_agent_access.as_deref()),
    )
    .await?
    .ok_or_else(|| AppError::NotFound("Service connection unavailable".into()))?;
    let canonical = crate::services::proxy_authorization::CanonicalPath::from_mcp_built(path)?;
    crate::services::destination_routing::resolve_target(
        &mut fresh.target,
        method,
        &canonical,
        Some(actual.target_id.as_deref()),
    )?;
    let key = db
        .collection::<ApiKey>("api_keys")
        .find_one(doc! {"_id":&chat.api_key_id,"is_active":true})
        .await?
        .ok_or_else(|| AppError::Forbidden("Assistant credential revoked".into()))?;
    if projection.apply_agent_override
        && key
            .assistant_agent_owner_id
            .as_deref()
            .is_none_or(|owner| owner == chat.user_id)
        && let Some(override_credential) =
            crate::services::proxy_service::resolve_agent_credential_override(
                db,
                keys,
                &chat.user_id,
                &chat.api_key_id,
                id,
                &fresh.target,
                None,
            )
            .await?
    {
        fresh.target.credential = override_credential;
    }
    projection.apply(&mut fresh.target);
    if let Some(git) = projection.machine_git.as_ref() {
        crate::services::machine_gateway_service::apply_live_git_target(
            db,
            &mut fresh.target,
            fresh.master_credential,
            git,
        )
        .await?;
    }
    if before != connection_binding(db, id, &chat.api_key_id).await?
        || execution_digest(actual) != execution_digest(&fresh.target)
    {
        return Err(AppError::Conflict(
            "Resolved service connection changed before execution".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub fn check_http<'a>(
    state: &'a crate::AppState,
    auth: &'a crate::mw::auth::AuthUser,
    id: Option<&'a str>,
    owner: Option<&'a str>,
    target: &'a crate::services::proxy_service::ProxyTarget,
    method: &'a str,
    path: &'a str,
    query: Option<&'a str>,
    body: &'a [u8],
    acknowledgement: Option<&'a str>,
    pool: Option<&'a str>,
    headers: &'a axum::http::HeaderMap,
) -> futures::future::BoxFuture<'a, AppResult<Decision>> {
    Box::pin(async move {
        check_http_projected(
            state,
            auth,
            id,
            owner,
            target,
            method,
            path,
            query,
            body,
            acknowledgement,
            pool,
            headers,
            &HttpProjection::default(),
        )
        .await
    })
}

#[allow(clippy::too_many_arguments)]
pub fn check_http_projected<'a>(
    state: &'a crate::AppState,
    auth: &'a crate::mw::auth::AuthUser,
    id: Option<&'a str>,
    owner: Option<&'a str>,
    target: &'a crate::services::proxy_service::ProxyTarget,
    method: &'a str,
    path: &'a str,
    query: Option<&'a str>,
    body: &'a [u8],
    acknowledgement: Option<&'a str>,
    pool: Option<&'a str>,
    headers: &'a axum::http::HeaderMap,
    projection: &'a HttpProjection,
) -> futures::future::BoxFuture<'a, AppResult<Decision>> {
    Box::pin(check_http_projected_inner(
        state,
        auth,
        id,
        owner,
        target,
        method,
        path,
        query,
        body,
        acknowledgement,
        pool,
        headers,
        projection,
    ))
}

#[allow(clippy::too_many_arguments)]
async fn check_http_projected_inner(
    state: &crate::AppState,
    auth: &crate::mw::auth::AuthUser,
    id: Option<&str>,
    owner: Option<&str>,
    target: &crate::services::proxy_service::ProxyTarget,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
    acknowledgement: Option<&str>,
    pool: Option<&str>,
    headers: &axum::http::HeaderMap,
    projection: &HttpProjection,
) -> AppResult<Decision> {
    if auth
        .org_agent_access
        .as_ref()
        .is_some_and(|a| a.shared().is_some())
        && id.is_none()
    {
        return Err(AppError::Forbidden(
            "Shared execution requires an exact service connection".into(),
        ));
    }
    if let Some(chat) = auth.assistant_chat.as_deref()
        && !chat.turn_live
        && let Some(owner) = owner
        && state
            .db
            .collection::<crate::models::user::User>("users")
            .find_one(doc! {"_id":owner,"is_active":true})
            .await?
            .is_some_and(|row| row.user_type == crate::models::user::UserType::Org)
    {
        return Err(AppError::AssistantTurnRequired);
    }
    let Some(chat) = auth.assistant_chat.as_deref() else {
        return Ok(Decision {
            refusal: None,
            admission: None,
            automatic: None,
        });
    };
    let method_override = mcp_service::http_carries_method_override(method, headers, query, body);
    let generation = !method_override
        && super::request_identity::legacy_model_generation(target, method, path, body);
    // Separate route identity from admission: Codex metadata remains legacy for
    // owners, including approval, exact selection and fallback classification.
    let model_route = (generation && pool.is_none())
        .then(|| super::request_identity::model_route_operation(target, method, path, query, body))
        .flatten()
        .map(|mut route| {
            bind_headers(&mut route, &projection.headers(target, headers));
            route
        });
    let mut operation = http_operation(target, method, path, query, body);
    bind_headers(&mut operation, &projection.headers(target, headers));
    let stateless = operation["kind"] == "inference";
    let guest_inference = chat.guest && target.service.inference.is_some();
    if guest_inference {
        if !chat.turn_live {
            return Err(AppError::GuestModelRouteRequired);
        }
        operation = model_route
            .clone()
            .ok_or(AppError::GuestModelRouteRequired)?;
        super::guest_model::require(&state.db, chat, id, target, &mut operation).await?;
    }
    let Some(id) = id else {
        if !chat.guest
            && chat.turn_live
            && let Some(route) = model_route.as_ref()
        {
            super::guest_model::remember_owner(&state.db, chat, None, target, route).await?;
        }
        return Ok(Decision {
            refusal: None,
            admission: None,
            automatic: None,
        });
    };
    if pool.is_none() {
        verify_resolved_projected(
            &state.db,
            &state.encryption_keys,
            chat,
            id,
            target,
            method,
            path,
            projection,
        )
        .await?;
    }
    if let Some(pool) = pool {
        operation["pool_id"] = json!(pool);
        let row = state
            .db
            .collection::<crate::models::service_pool::ServicePool>("service_pools")
            .find_one(doc! {"_id":pool,"is_active":true})
            .await?
            .ok_or_else(|| AppError::Conflict("Service pool unavailable".into()))?;
        operation["pool_revision"] = json!(row.config_revision);
    }
    if method_override
        && chat
            .org_agent_access
            .as_ref()
            .is_some_and(|a| a.shared().is_some())
    {
        return Err(AppError::Forbidden(
            "Shared calls cannot override HTTP methods".into(),
        ));
    }
    // Shared turns run as the acting person. The owner-only guest model route
    // is deliberately selected only by `guest_inference` above.
    let model_transport = !chat.guest && generation;
    let legacy_inference = model_transport && (!chat.turn_live || !stateless);
    if legacy_inference {
        // This only preserves the existing model transport scope exception. It
        // neither installs turn consent nor proves cross-account equivalence.
        operation["kind"] = json!("legacy_inference");
    }
    let raw_inference = chat.turn_live && stateless && !chat.guest && !method_override;
    let mut model_consent = false;
    let mut consent_binding = None;
    if raw_inference {
        operation["kind"] = json!("raw_inference");

        operation
            .as_object_mut()
            .expect("operation object")
            .remove("body");
        let mut selected_filter = scope(chat)?;
        selected_filter.insert("kind", "selection");
        let selected = state
            .db
            .collection::<Document>(COLLECTION)
            .find_one(selected_filter)
            .await?;
        let inherited = selected
            .as_ref()
            .and_then(|row| row.get_document("operation").ok())
            .filter(|op| op.get_str("kind") == Ok("raw_inference"))
            .and_then(|op| op.get_str("input_admission").ok())
            .map(str::to_owned);
        let input_admission = if let Some(inherited) = inherited {
            inherited
        } else {
            let mut filter = scope(chat)?;
            filter.insert("kind", "input_admission");
            if let Some(input) = state
                .db
                .collection::<Document>(COLLECTION)
                .find_one(filter)
                .await?
            {
                input
                    .get_str("input_admission")
                    .map_err(|_| AppError::Forbidden("Invalid input admission".into()))?
                    .to_owned()
            } else {
                // Rolling compatibility for turns admitted before this version.
                // Only the first server-stored initiating message is eligible.
                let input = state.db.collection::<Document>("assistant_messages").find_one(doc! {
                    "user_id": &chat.user_id, "conversation_id": &chat.conversation_id, "turn_id": &chat.turn_id,
                    "role": {"$in": ["user", "event", "orchestrator", "group", "trigger"]},
                }).sort(doc! {"seq": 1}).await?
                    .ok_or_else(|| AppError::Forbidden("Model use needs a server-admitted turn input".into()))?;
                digest(
                    json!({"id": input.get_str("_id").ok(), "turn_id": &chat.turn_id,
                    "origin": input.get("origin"), "text": input.get_str("text").ok()}),
                )
            }
        };
        operation["input_admission"] = json!(input_admission);
        let mut grant_filter = scope(chat)?;
        grant_filter.extend(doc! {"kind": "model_consent", "connection_id": id, "operation": digest(operation.clone())});
        if let Some(grant) = state
            .db
            .collection::<Document>(COLLECTION)
            .find_one(grant_filter)
            .await?
        {
            if grant.get_str("binding")
                == Ok(connection_binding(&state.db, id, &chat.api_key_id)
                    .await?
                    .as_str())
            {
                model_consent = true;
                consent_binding = grant.get_str("binding").ok().map(str::to_owned);
            } else {
                return Err(AppError::Conflict(
                    "Model-use consent expired because the connection changed".into(),
                ));
            }
        }
    }
    let read = matches!(method, "GET" | "HEAD" | "OPTIONS") && !method_override;
    let destructive = method == "DELETE" || method_override;
    let summary = if raw_inference {
        format!(
            "Use {} / {} for this turn",
            crate::services::assistant_nyxagent::identifier(&target.service.slug),
            crate::services::assistant_nyxagent::identifier(
                operation["model"].as_str().unwrap_or("model")
            )
        )
    } else {
        format!(
            "{} {} using {}",
            method,
            crate::services::assistant_nyxagent::identifier(path),
            crate::services::assistant_nyxagent::identifier(&target.service.slug)
        )
    };
    let decision = check(
        &state.db,
        &state.node_ws_manager,
        chat,
        id,
        owner.unwrap_or(&chat.user_id),
        &operation,
        acks::ActionFacts::of(acks::service_action_class(
            read || raw_inference || guest_inference || (legacy_inference && stateless),
            destructive,
        )),
        acknowledgement,
        &summary,
        pool.is_some(),
        Some(target),
        model_consent,
        Some((
            body,
            headers
                .get("content-type")
                .and_then(|value| value.to_str().ok()),
        )),
    )
    .await?;
    if let Some(binding) = consent_binding.as_deref() {
        validate_model_consent_snapshot(decision.admission.as_ref(), binding)?;
    }
    if chat.turn_live && model_transport {
        super::bootstrap::observe_model_callback(&state.db, chat, decision.refusal.as_ref())
            .await?;
    }
    if let Some(admission) = decision
        .admission
        .as_ref()
        .filter(|_| raw_inference && !model_consent)
    {
        persist_model_consent(&state.db, chat, admission, target, &operation).await?;
    }
    if let Some(admission) = decision.admission.as_ref() {
        admission.revalidate(&state.db, id, Some(target)).await?;
    }
    if model_transport
        && chat.turn_live
        && decision.admission.is_some()
        && let Some(route) = model_route.as_ref()
    {
        super::guest_model::remember_owner(&state.db, chat, Some(id), target, route).await?;
    }
    Ok(decision)
}

// Publish only the authority that actually admitted the operation. A newer
// database snapshot must never upgrade an old approval into a new credential.
pub(super) async fn persist_model_consent(
    db: &Database,
    chat: &acks::ChatAuthority,
    admission: &Admission,
    target: &crate::services::proxy_service::ProxyTarget,
    operation: &Value,
) -> AppResult<()> {
    admission
        .revalidate(db, &admission.connection, Some(target))
        .await?;
    let mut grant = scope(chat)?;
    grant.extend(
        doc! {"_id":uuid::Uuid::new_v4().to_string(), "kind":"model_consent",
        "connection_id":&admission.connection, "binding":&admission.binding,
        "operation":digest(operation.clone()),
        "expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(35))},
    );
    crate::services::erasure_admission::insert_owned(db, COLLECTION, &chat.user_id, &grant).await?;
    Ok(())
}

pub(super) fn validate_model_consent_snapshot(
    admission: Option<&Admission>,
    binding: &str,
) -> AppResult<()> {
    if admission.is_some_and(|admission| admission.binding != binding) {
        return Err(AppError::Conflict(
            "Model-use authority changed during admission".into(),
        ));
    }
    Ok(())
}
