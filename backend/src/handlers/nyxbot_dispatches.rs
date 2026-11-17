use super::*;

#[derive(serde::Deserialize)]
pub struct Query {
    pub(super) dispatched_at: Option<String>,
}

pub async fn get(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Query(query): axum::extract::Query<Query>,
) -> Response {
    provider_refusal::scope(
        &state.db,
        provider_refusal::Endpoint::Dispatches,
        crate::services::security_readiness_service::without_diagnostics(get_inner(
            &state, &headers, query,
        )),
    )
    .await
}
async fn get_inner(state: &AppState, headers: &HeaderMap, query: Query) -> Response {
    let Some(key) = headers
        .get("idempotency-key")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 256)
    else {
        return problem(StatusCode::BAD_REQUEST, "invalid_request");
    };
    let at = match query.dispatched_at {
        Some(at) => match at
            .parse::<i64>()
            .ok()
            .filter(|n| *n >= 0)
            .and_then(|n| chrono::DateTime::from_timestamp(n, 0))
        {
            Some(at) => Some(at),
            None => return problem(StatusCode::BAD_REQUEST, "invalid_request"),
        },
        None => None,
    };
    match crate::services::platform_entry_gateway_service::observe_authority(state, headers).await {
        Ok(Some(_)) => return problem(StatusCode::BAD_REQUEST, "dispatch_identity_unavailable"),
        Ok(None) => {}
        Err(error) => return provider_refusal::error(error),
    }
    let (row, custom) = match provider_event_binding_mode(state, headers, true, true, true).await {
        Ok(Some((binding, _))) => {
            let destination=state.db.collection::<NyxbotChannel>(CHANNELS).find_one(doc! {
                "_id":&binding.destination_channel_id,"user_id":&binding.user_id,"status":"active",
            }).await;
            match destination {
                Ok(Some(row)) => {
                    let agent =
                        match crate::services::nyxbot_event_binding_service::authorize_provider(
                            state, &binding,
                        )
                        .await
                        {
                            Ok(agent) => agent,
                            Err(error) => return provider_refusal::error(error),
                        };
                    if !(row.agent_id.as_deref() == Some(agent.id.as_str())
                        || (row.agent_id.is_none()
                            && agent.is_nyxbot()
                            && agent.user_id == row.user_id))
                    {
                        return problem(StatusCode::FORBIDDEN, "destination_target_changed");
                    }
                    if let Err(error) =
                        crate::services::platform_bot_authority::require_personal_routes(
                            &state.db,
                            &row.channel_bot_id,
                        )
                        .await
                    {
                        return provider_refusal::error(error);
                    }
                    match org_access(state, &row).await {
                        Ok(OrgAccess::Holds) => {}
                        Ok(OrgAccess::Lost) => {
                            return problem(StatusCode::FORBIDDEN, "destination_org_access_lost");
                        }
                        _ => {
                            return problem(
                                StatusCode::SERVICE_UNAVAILABLE,
                                "provider_unavailable",
                            );
                        }
                    }
                    if row.gateway_source.is_some() || row.transport != "direct" {
                        return problem(StatusCode::FORBIDDEN, "destination_relay_required");
                    }
                    (row, Some(binding.id))
                }
                Ok(None) => return problem(StatusCode::NOT_FOUND, "conversation_not_found"),
                Err(_) => return problem(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable"),
            }
        }
        Ok(None) => match provider_channel_mode(state, headers, true).await {
            Ok((row, _)) => (row, None),
            Err(response) => return response,
        },
        Err(response) => return response,
    };
    provider_refusal::channel(&row.user_id, &row.id);
    match state
        .db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {
            "_id":&row.user_id,"is_active":true,"platform_personal_deleting":{"$ne":true},
        })
        .projection(doc! {"_id":1})
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return problem(StatusCode::UNAUTHORIZED, "not_authenticated"),
        Err(_) => return problem(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable"),
    }
    let readiness = match crate::services::security_readiness_service::report(
        &state.db,
        dispatch_identity::READINESS,
    )
    .await
    {
        Ok(report) if report.serving_ready => report,
        _ => {
            return problem(
                StatusCode::SERVICE_UNAVAILABLE,
                "response_delivery_not_ready",
            );
        }
    };
    match dispatch_identity::lookup(
        &state.db,
        &state.encryption_keys,
        &row,
        key,
        custom.as_deref(),
        at,
        readiness.active_since,
    )
    .await
    {
        Ok(dispatch_identity::Lookup::Found(found)) => {
            ([("cache-control", "no-store")], Json(found)).into_response()
        }
        Ok(dispatch_identity::Lookup::IdentityUnavailable) => {
            problem(StatusCode::BAD_REQUEST, "dispatch_identity_unavailable")
        }
        Err(_) => problem(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable"),
    }
}
