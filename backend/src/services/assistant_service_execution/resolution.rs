use super::*;

type HttpResolutionRequest<'a> = (
    &'a str,
    &'a str,
    Option<&'a str>,
    &'a [u8],
    &'a axum::http::HeaderMap,
    &'a HttpProjection,
);

/// Implicit provider resolution chooses before credential materialization.
/// Exact UUID/connection-slug routes continue through their exact resolver.
pub async fn resolve_implicit_for_request(
    state: &crate::AppState,
    auth: &crate::mw::auth::AuthUser,
    catalog: &str,
    request: Option<HttpResolutionRequest<'_>>,
) -> AppResult<Option<crate::services::proxy_service::UserServiceResolution>> {
    let Some(chat) = auth.assistant_chat.as_deref() else {
        return crate::services::proxy_service::resolve_proxy_target_from_user_service(
            &state.db,
            &state.encryption_keys,
            &state.node_ws_manager,
            &auth.proxy_resolution_user_id(),
            None,
            Some(catalog),
            crate::services::proxy_service::ProxyExecutionContext::new(
                Some(&state.connection_expiry_notifier),
                state.platform_user_rate_limit,
            )
            .with_assistant_access(auth.org_agent_access.as_deref()),
        )
        .await;
    };
    chat.ensure_live_turn(&state.db, None, None, "assistant.model_resolution")
        .await?;
    let inference_catalog = state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .find_one(doc! {"_id": catalog, "is_active": true})
        .await?
        .is_some_and(|row| row.inference.is_some());
    let shared = auth
        .org_agent_access
        .as_ref()
        .and_then(|access| access.shared());
    let mut rows: Vec<UserService> = if let Some(shared) = shared {
        shared.catalog_connections(catalog, inference_catalog)
    } else {
        let memberships = crate::services::org_service::list_memberships_for_member(
            &state.db,
            &chat.user_id,
            false,
        )
        .await?;
        let rows =
        crate::services::user_service_service::list_user_services_with_sources_selected_in_group(
            &state.db,
            &chat.user_id,
            false,
            false,
            &memberships,
            crate::services::user_service_service::ServiceSelection {
                ids: None,
                catalog_service_id: Some(catalog),
            },
        )
        .await?;
        rows.into_iter()
            .filter(|row| {
                !matches!(
                    row.source,
                    crate::services::user_service_service::CredentialSource::Org {
                        allowed: false,
                        ..
                    }
                ) && (chat.turn_live || row.service.user_id == chat.user_id)
                    && (auth.allow_all_services
                        || auth.allowed_service_ids.contains(&row.service.id)
                        || inference_catalog)
            })
            .map(|row| row.service)
            .collect()
    };
    let order = preferences::get(&state.db, &chat.user_id)
        .await?
        .map(|row| row.ordered)
        .unwrap_or_default();
    let positions: HashMap<_, _> = order
        .iter()
        .enumerate()
        .map(|(i, id)| (id.as_str(), i))
        .collect();
    let selected = selection::selected(&state.db, chat).await?;
    let selected_id = selected
        .as_ref()
        .and_then(|row| row.get_str("connection_id").ok());
    rows.sort_by_key(|row| {
        (
            Some(row.id.as_str()) != selected_id,
            row.user_id != chat.user_id,
            positions
                .get(row.id.as_str())
                .copied()
                .unwrap_or(usize::MAX),
        )
    });
    let selection_applies = selected.as_ref().is_some_and(|selection| {
        selection.get_str("catalog_service_id") == Ok(catalog)
            || selection
                .get_document("operation")
                .ok()
                .and_then(|op| op.get_str("provider").ok())
                == Some(catalog)
            || selected_id.is_some_and(|id| rows.iter().any(|row| row.id == id))
    });
    let selected_in_group = selected_id.filter(|_| selection_applies);
    if let Some(selected) = selected_in_group {
        rows.retain(|row| row.id == selected);
        if rows.is_empty() {
            return Err(AppError::Forbidden(
                "Selected connection is unavailable or outside current authority".into(),
            ));
        }
    }
    let legacy_personal = shared.is_none()
        && selected_in_group.is_none()
        && crate::services::proxy_service::user_has_legacy_personal_connection(
            &state.db,
            &chat.user_id,
            None,
            Some(catalog),
        )
        .await?;
    for row in rows {
        // Main's migration contract: migrated personal, legacy personal, org.
        // None hands control to the existing legacy credential resolver.
        if row.user_id != chat.user_id && legacy_personal {
            return Ok(None);
        }
        if let Some((method, path, query, body, headers, _)) = request {
            let canonical =
                crate::services::proxy_authorization::CanonicalPath::from_mcp_built(path)?;
            if crate::services::agent_operation_scope_service::authorize(
                &auth.assistant_operation_scopes,
                &row.id,
                Some(catalog),
                None,
                method,
                &canonical,
                false,
                false,
            )
            .is_err()
                || crate::services::agent_operation_scope_service::check_inputs(
                    &auth.assistant_operation_scopes,
                    &row.id,
                    Some(catalog),
                    None,
                    method,
                    &canonical,
                    query,
                    body,
                    headers.get("content-type").and_then(|v| v.to_str().ok()),
                )
                .is_err()
            {
                if selected_in_group.is_some() || shared.is_some() {
                    return Err(AppError::Forbidden(
                        "Selected connection does not permit this operation".into(),
                    ));
                }
                continue;
            }
        }
        match crate::services::proxy_service::resolve_proxy_target_by_user_service_id(
            &state.db,
            &state.encryption_keys,
            &chat.user_id,
            &row.id,
            None,
            Some(catalog),
            crate::services::proxy_service::ProxyExecutionContext::new(
                Some(&state.connection_expiry_notifier),
                state.platform_user_rate_limit,
            )
            .with_assistant_access(auth.org_agent_access.as_deref()),
        )
        .await
        {
            Ok(Some(mut resolved)) => {
                if shared.is_none()
                    && selected_in_group.is_none()
                    && let Some((method, path, query, body, headers, projection)) = request
                {
                    projection.apply(&mut resolved.target);
                    if chat.turn_live
                        && let Some(operation) = super::request_identity::raw_identity(
                            &resolved.target,
                            method,
                            path,
                            query,
                            body,
                            headers,
                            projection,
                        )
                    {
                        if verified_unavailable(
                            &state.db,
                            &state.node_ws_manager,
                            chat,
                            &row,
                            &operation,
                        )
                        .await?
                        {
                            continue;
                        }
                        let mut evidence = scope(chat)?;
                        evidence.extend(doc! {"kind":"unavailable_bucket","entries":{"$elemMatch":{
                                "connection_id":&row.id,"binding":connection_binding(&state.db,&row.id,&chat.api_key_id).await?,
                                "selection_operation":selection_fingerprint(&operation),"expires_at":{"$gt":bson::DateTime::now()}}}});
                        if state
                            .db
                            .collection::<Document>(COLLECTION)
                            .find_one(evidence)
                            .await?
                            .is_some()
                        {
                            continue;
                        }
                    }
                }
                return Ok(Some(resolved));
            }
            Ok(None) if selected_in_group.is_some() || shared.is_some() => {
                return Err(AppError::Forbidden(
                    "Selected connection is unavailable".into(),
                ));
            }
            Ok(None) => continue,
            Err(AppError::CredentialUnavailable(_))
                if shared.is_none() && selected_id != Some(row.id.as_str()) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    Ok(None)
}
