//! Symmetric, bounded transport reconciliation for linked channel bots.
use super::*;

pub(super) async fn authority(state: &AppState, row: &NyxbotChannel) -> Result<(), &'static str> {
    if !org_access_holds(state, row)
        .await
        .map_err(|_| "storage_unavailable")?
    {
        Box::pin(release_org_channel(state, row))
            .await
            .map_err(|_| "storage_unavailable")?;
        return Err("org_access_lost");
    }
    Ok(())
}

pub(super) async fn ready_to_swap(
    state: &AppState,
    row: &NyxbotChannel,
    gateway: bool,
) -> Result<(), &'static str> {
    authority(state, row).await?;
    if gateway_enabled(state, &row.user_id, &row.platform)
        .await
        .map_err(|_| "storage_unavailable")?
        != gateway
    {
        return Err("flag_changed");
    }
    if channel_answering(state, row)
        .await
        .map_err(|_| "storage_unavailable")?
    {
        return Err("answering");
    }
    Ok(())
}

/// Both transports retain the route identity and all its settings. Only its
/// callback key changes, atomically with the channel and behind both key fences.
pub(super) async fn swap_route(
    state: &AppState,
    row: &NyxbotChannel,
    next_key: &str,
    fence: bson::Document,
    update: bson::Document,
) -> AppResult<bool> {
    let route_id = row
        .route_id
        .clone()
        .ok_or_else(|| AppError::Conflict("route_missing".into()))?;
    let (owner, old_key) = (bot_owner(row).to_owned(), row.route_api_key_id.clone());
    let reroute =
        doc! {"$set": {"agent_api_key_id": next_key, "updated_at": bson::DateTime::now()}};
    let db = state.db.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let moved = db
                .collection::<NyxbotChannel>(CHANNELS)
                .update_one(fence.clone(), update.clone())
                .session(&mut *session)
                .await?;
            if moved.matched_count != 1 {
                return Ok(false);
            }
            let routed = db
                .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
                .update_one(
                    doc! {"_id": &route_id, "user_id": &owner, "is_active": true,
                    "agent_api_key_id": &old_key},
                    reroute.clone(),
                )
                .session(&mut *session)
                .await?;
            if routed.matched_count != 1 {
                return Err(mongodb::error::Error::custom("route_changed"));
            }
            Ok(true)
        })
        .await
        .map_err(AppError::from)
}

pub(super) async fn reconcile(state: &AppState) -> AppResult<()> {
    let now = Utc::now();
    let channels = state.db.collection::<NyxbotChannel>(CHANNELS);
    let stale = bson::DateTime::from_chrono(now - ChronoDuration::minutes(MOVE_STALE_MINUTES));
    if let Some(stuck) = channels.find_one_and_update(
        doc! {"$or": [
            {"transport": "direct", "pending_agent_api_key_id": {"$ne": null}, "gateway_attempted_at": {"$lt": stale}},
            {"transport": "gateway", "pending_route_api_key_id": {"$ne": null}, "relay_attempted_at": {"$lt": stale}}
        ]},
        vec![doc! {"$set": {"pending_agent_api_key_id": "$$REMOVE", "pending_route_api_key_id": "$$REMOVE",
            "binding_id": {"$cond": [{"$eq": ["$transport", "direct"]}, "$$REMOVE", "$binding_id"]}}}],
    ).await? {
        for (key, owner) in [
            (stuck.pending_agent_api_key_id.as_deref(), stuck.user_id.as_str()),
            (stuck.pending_route_api_key_id.as_deref(), bot_owner(&stuck)),
        ] {
            if let Some(key) = key { let _ = key_service::delete_api_key(&state.db, owner, key).await; }
        }
    }
    // A personal pilot only considers that person's relay bots. An org
    // override yields None, so its linking admins' org bots are included too;
    // the final decision still uses the existing live per-person resolver.
    let mut forward = vec![doc! {"platform": {"$in": ["telegram", "telegram-new"]}}];
    let mut backward = Vec::new();
    for (platform, flag) in feature_flag_service::NYXBOT_GATEWAY_FLAGS {
        match feature_flag_service::flag_enabled_people(&state.db, flag).await? {
            None => {
                forward.push(doc! {"platform": platform});
                backward.push(doc! {"platform": platform});
            }
            Some(people) => {
                if !people.is_empty() {
                    forward.push(doc! {"platform": platform, "user_id": {"$in": &people}});
                }
                backward.push(doc! {"platform": platform, "user_id": {"$nin": people}});
            }
        }
    }
    let recheck = bson::DateTime::from_chrono(now - ChronoDuration::minutes(MOVE_RECHECK_MINUTES));
    // Pending preparations must retain their key references until they finish
    // or the stale reaper clears them. In particular, relay retries become due
    // sooner than the stale interval and must not orphan a previous key.
    if let Some(row) = channels.find_one_and_update(
        doc! {"status": "active", "transport": "gateway", "pending_route_api_key_id": null, "$and": [
            {"$or": backward}, {"$or": [{"relay_attempted_at": null}, {"relay_attempted_at": {"$lte": recheck}}]}
        ]}, doc! {"$set": {"relay_attempted_at": bson::DateTime::now()}},
    ).sort(doc! {"relay_attempted_at": 1}).await? {
        if gateway_enabled(state, &row.user_id, &row.platform).await? {
            // Org/global overrides need a live per-person decision. Keep on
            // bots eligible for a later flag-off check, rotating oldest first.
            channels.update_one(doc! {"_id": &row.id, "transport": "gateway"},
                doc! {"$set": {"relay_attempted_at": recheck}}).await?;
        } else {
            let outcome = Box::pin(move_to_direct(state, &row)).await;
            audit(state, &row.user_id, "nyxbot_channel_transport_checked",
                json!({"channel_agent_id": row.id, "platform": row.platform,
                    "moved_to_direct": outcome.is_ok(), "error_code": outcome.err()})).await;
            return Ok(()); // At most one attempted move per sweep, either way.
        }
    }
    let due = bson::DateTime::from_chrono(now - ChronoDuration::hours(GATEWAY_RETRY_HOURS));
    let Some(row) = channels.find_one_and_update(
        doc! {"status": "active", "transport": "direct", "pending_agent_api_key_id": null,
            "owner_sender_ids.0": {"$exists": true},
            "$and": [{"$or": forward}, {"$or": [{"gateway_attempted_at": null}, {"gateway_attempted_at": {"$lt": due}}]}]},
        doc! {"$set": {"gateway_attempted_at": bson::DateTime::now()}},
    ).sort(doc! {"gateway_attempted_at": 1}).await? else { return Ok(()); };
    let outcome = match ready_to_swap(state, &row, true).await {
        Ok(()) => Box::pin(move_to_gateway(state, &row)).await,
        Err(code) => Err(code),
    };
    if matches!(outcome, Err("answering" | "flag_changed")) {
        channels.update_one(doc! {"_id": &row.id, "transport": "direct"},
            doc! {"$set": {"gateway_attempted_at": bson::DateTime::from_chrono(
                now - ChronoDuration::hours(GATEWAY_RETRY_HOURS) + ChronoDuration::minutes(MOVE_RECHECK_MINUTES))}}).await?;
    } else if let Err(code) = outcome {
        channels
            .update_one(
                doc! {"_id": &row.id, "transport": "direct"},
                doc! {"$set": {"gateway_fallback_at": bson::DateTime::now()}},
            )
            .await?;
        tracing::info!(code, platform = %row.platform, "NyxBot channel stays on NyxID's relay");
    }
    audit(
        state,
        &row.user_id,
        "nyxbot_channel_transport_checked",
        json!({"channel_agent_id": row.id, "platform": row.platform,
            "moved_to_gateway": outcome.is_ok(), "error_code": outcome.err()}),
    )
    .await;
    Ok(())
}

async fn move_to_direct(state: &AppState, row: &NyxbotChannel) -> Result<(), &'static str> {
    if row.transport != "gateway" || canonical_platform(&row.platform) == "telegram" {
        return Err("not_movable");
    }
    ready_to_swap(state, row, false).await?;
    let key_owner = bot_owner(row);
    let callback = format!(
        "{}/api/v1/nyxbot/relay/{}",
        state.config.base_url.trim_end_matches('/'),
        row.id
    );
    let next = new_route_key(state, key_owner, &row.bot_label, &callback)
        .await
        .map_err(|_| "route_key_create_failed")?;
    let channels = state.db.collection::<NyxbotChannel>(CHANNELS);
    let fence = doc! {"_id": &row.id, "status": "active", "transport": "gateway",
    "route_api_key_id": &row.route_api_key_id, "agent_api_key_id": &row.agent_api_key_id};
    let discard = async |code| {
        let _ = key_service::delete_api_key(&state.db, key_owner, &next.id).await;
        let _ = channels
            .update_one(
                doc! {"_id": &row.id, "pending_route_api_key_id": &next.id},
                doc! {"$unset": {"pending_route_api_key_id": ""}},
            )
            .await;
        code
    };
    match channels
        .update_one(
            fence.clone(),
            doc! {"$set": {"pending_route_api_key_id": &next.id}},
        )
        .await
    {
        Ok(result) if result.matched_count == 1 => {}
        _ => return Err(discard("channel_changed").await),
    }
    if let Err(code) = ready_to_swap(state, row, false).await {
        return Err(discard(code).await);
    }
    let mut fence = fence;
    fence.insert("pending_route_api_key_id", &next.id);
    let update = doc! {"$set": {"transport": "direct", "route_api_key_id": &next.id, "updated_at": bson::DateTime::now()},
    "$unset": {"agent_api_key_id": "", "agent_key_ciphertext": "", "gateway_channel_id": "",
        "gateway_record_id": "", "gateway_version": "", "binding_id": "", "gateway_bot_id": "",
        "gateway_groups": "", "gateway_groups_retry_at": "", "gateway_attempted_at": "", "gateway_fallback_at": "",
        "pending_agent_api_key_id": "", "pending_route_api_key_id": ""}};
    match swap_route(state, row, &next.id, fence, update).await {
        Ok(true) => {}
        Ok(false) => return Err(discard("channel_changed").await),
        Err(_) => {
            if !load_channel(state, &row.user_id, &row.id)
                .await
                .is_ok_and(|now| now.route_api_key_id == next.id)
            {
                return Err(discard("swap_failed").await);
            }
        }
    }
    if let Some(channel_id) = row.gateway_channel_id.as_deref() {
        release_gateway_channel(
            state,
            &row.user_id,
            channel_id,
            row.gateway_version.unwrap_or(1),
        )
        .await;
    }
    if let Some(agent) = row.agent_api_key_id.as_deref() {
        let _ = key_service::delete_api_key(&state.db, &row.user_id, agent).await;
    }
    let _ = key_service::delete_api_key(&state.db, key_owner, &row.route_api_key_id).await;
    Ok(())
}
