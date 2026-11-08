//! Owner-requested native Discord retry. Reuses transport preparation and swap;
//! the existing attempted-at CAS coordinates this action with the sweep.
use super::*;

pub(super) const RETRY_MINUTES: i64 = 10;

pub(super) fn fallback_reason(row: &NyxbotChannel) -> Option<String> {
    if row.platform != "discord" || row.transport != "direct" {
        return None;
    }
    let code = row.gateway_native_activation.as_deref()?;
    if matches!(code, "relay_ready" | "relay_draining") {
        return None;
    }
    let status = if row.status == "active" && row.delivery_status.as_deref() != Some("failing") {
        "Working on NyxID relay."
    } else {
        "NyxID relay selected; check channel health for delivery status."
    };
    Some(format!(
        "{status} Native gateway activation did not complete: {}.",
        row.gateway_native_activation_detail
            .as_deref()
            .unwrap_or(code)
    ))
}

pub(super) fn next_automatic_retry(row: &NyxbotChannel) -> Option<chrono::DateTime<Utc>> {
    if row.platform != "discord" || row.status != "active" || row.transport != "direct" {
        return None;
    }
    // The sweep treats a missing attempted-at as already due. Do not invent
    // an extra 23-hour delay for legacy rows carrying only a fallback time.
    row.gateway_attempted_at
        .map(|at| at + ChronoDuration::hours(GATEWAY_RETRY_HOURS))
        .or(row.gateway_fallback_at)
}

fn refused(code: &str) -> AppError {
    if code == "org_access_lost" {
        AppError::Forbidden(code.into())
    } else {
        AppError::Conflict(code.into())
    }
}

pub async fn retry_channel(
    State(state): State<AppState>,
    auth: crate::mw::auth::AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    crate::handlers::login_client_context::require_first_party_human(&auth)?;
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    Ok(Json(Box::pin(retry(&state, &owner, &id)).await?))
}

pub(crate) async fn retry(state: &AppState, owner: &str, id: &str) -> AppResult<Value> {
    let row = load_channel(state, owner, id).await?;
    if row.platform != "discord" || row.status != "active" || row.transport != "direct" {
        return Err(refused("native_gateway_retry_not_available"));
    }
    if row.owner_sender_ids.is_empty() {
        return Err(refused("owner_verification_required"));
    }
    transport::ready_to_swap(state, &row, true)
        .await
        .map_err(refused)?;
    let now = Utc::now();
    if row
        .gateway_attempted_at
        .is_some_and(|at| at + ChronoDuration::minutes(RETRY_MINUTES) > now)
    {
        return Err(refused("native_gateway_retry_rate_limited"));
    }
    let channels = state.db.collection::<NyxbotChannel>(CHANNELS);
    // Sweeps and owner requests compete for this exact timestamp; only one
    // may start. Preparing keys are never replaced by another request.
    let result = channels.update_one(
        doc! {"_id":&row.id,"user_id":owner,"status":"active","transport":"direct",
            "route_api_key_id":&row.route_api_key_id,"gateway_attempted_at":row.gateway_attempted_at.map(bson::DateTime::from_chrono),
            "pending_agent_api_key_id":null,"pending_route_api_key_id":null},
        doc! {"$set":{"gateway_attempted_at":bson::DateTime::from_chrono(now),"gateway_native_activation_detail":bson::Bson::Null,"gateway_native_activation_operation":bson::Bson::Null}},
    ).await?;
    if result.matched_count != 1 {
        return Err(refused("native_gateway_retry_busy"));
    }
    let outcome = match transport::ready_to_swap(state, &row, true).await {
        Ok(()) => Box::pin(move_to_gateway(state, &row)).await,
        Err(code) => Err(code),
    };
    if let Err(code) = outcome {
        channels.update_one(
            doc! {"_id":&row.id,"user_id":owner,"status":"active","transport":"direct",
                "route_api_key_id":&row.route_api_key_id,"gateway_attempted_at":bson::DateTime::from_chrono(now)},
            doc! {"$set":{"gateway_native_activation":code,"gateway_fallback_at":bson::DateTime::now()}},
        ).await?;
    }
    let current = load_channel(state, owner, id).await?;
    let details = gateway_diagnostic_audit(
        json!({"channel_agent_id":id,"platform":row.platform,
        "moved_to_gateway":outcome.is_ok(),"error_code":outcome.err()}),
        &current,
    );
    audit(
        state,
        owner,
        "nyxbot_channel_native_gateway_retried",
        details,
    )
    .await;
    Ok(
        json!({"channel_agent":ChannelAgentResponse::from(&current),"moved_to_gateway":outcome.is_ok(),
        "code":outcome.err().unwrap_or("native_gateway_selected")}),
    )
}
