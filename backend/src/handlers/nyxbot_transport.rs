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
    let state = state.clone();
    let row = row.clone();
    let next_key = next_key.to_owned();
    let db = state.db.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let work:AppResult<_>=Box::pin(async {
            let Some(current)=db.collection::<NyxbotChannel>(CHANNELS).find_one(fence.clone()).session(&mut *session).await? else {return Ok(false);};
            crate::services::platform_bot_authority::fence_personal_routes(&db,&current.channel_bot_id,bot_owner(&current),session).await?;
            channel_routing_service::fence_route_key(&db,&next_key,session).await?;
            if current.transport=="gateway" {releases::retire_in_session(&state,&current,ReleaseReason::TransportMove,session).await?;}
            if db.collection::<NyxbotChannel>(CHANNELS).update_one(fence.clone(),update.clone()).session(&mut *session).await?.matched_count!=1 {return Ok(false);}
            let next=db.collection::<NyxbotChannel>(CHANNELS).find_one(doc! {"_id":&row.id}).session(&mut *session).await?
                .ok_or_else(|| AppError::Conflict("channel_changed".into()))?;
            if next.transport=="gateway" {
                releases::adopt_in_session(&db,&next,session).await?;
                db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id},doc! {"$unset":{"gateway_setup_release_id":""}}).session(&mut *session).await?;
            }
            if db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME).update_one(
                doc! {"_id":&route_id,"user_id":bot_owner(&row),"is_active":true,"agent_api_key_id":&row.route_api_key_id},
                doc! {"$set":{"agent_api_key_id":&next_key,"updated_at":bson::DateTime::now()}}).session(&mut *session).await?.matched_count!=1 {
                return Err(AppError::Conflict("route_changed".into()));
            }
            releases::revoke_in_session(&db,bot_owner(&row),&row.route_api_key_id,session).await?;
            if current.transport=="gateway" && let Some(key)=current.agent_api_key_id.as_deref() {releases::revoke_in_session(&db,&current.user_id,key,session).await?;}
            Ok(true)
        }).await;crate::services::api_key_mutation_service::transaction_result(work)
    }).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub(super) fn stale_move_cutoff(now: chrono::DateTime<Utc>) -> bson::DateTime {
    bson::DateTime::from_chrono(now - ChronoDuration::minutes(releases::SETUP_MINUTES))
}

pub(super) async fn reconcile(state: &AppState) -> AppResult<()> {
    let now = Utc::now();
    let channels = state.db.collection::<NyxbotChannel>(CHANNELS);
    let stale = stale_move_cutoff(now);
    if let Some(stuck)=channels.find_one(doc! {"status":"active","$or":[
        {"transport":"direct","pending_agent_api_key_id":{"$ne":null},"gateway_attempted_at":{"$lt":stale}},
        {"transport":"gateway","pending_route_api_key_id":{"$ne":null},"relay_attempted_at":{"$lt":stale}}
    ]}).await? {releases::retire_pending(state,&stuck).await?;}
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
            .update_one(doc! {"_id": &row.id, "transport": "direct"}, {
                let mut set = doc! {"gateway_fallback_at":bson::DateTime::now()};
                if row.platform == "discord" {
                    set.insert("gateway_native_activation", code);
                }
                doc! {"$set":set}
            })
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
    let mut set = doc! {"transport":"direct","gateway_no_progress":false,"route_api_key_id":&next.id,"updated_at":bson::DateTime::now()};
    if row.platform == "discord" && row.gateway_source.is_some() {
        set.insert("gateway_native_activation", "relay_draining");
    }
    let update = doc! {"$set": set,
    "$unset": {"agent_api_key_id": "", "agent_key_ciphertext": "", "gateway_channel_id": "",
        "gateway_source": "", "gateway_native_phase": "", "gateway_record_id": "", "gateway_threads": "", "gateway_version": "", "binding_id": "", "gateway_bot_id": "",
        "gateway_groups": "", "gateway_groups_retry_at": "", "gateway_attempted_at": "", "gateway_fallback_at": "",
        "pending_agent_api_key_id": "", "pending_route_api_key_id": ""}};
    match swap_route(state, row, &next.id, fence, update).await {
        Ok(true) => {}
        Ok(false) => return Err(discard("channel_changed").await),
        Err(_) => match load_channel(state, &row.user_id, &row.id).await {
            Ok(now) if now.route_api_key_id == next.id => {}
            Ok(_) => return Err(discard("swap_failed").await),
            Err(_) => return Err("storage_unavailable"),
        },
    }
    let ids = state
        .db
        .collection::<crate::models::nyxbot_gateway_release::NyxbotGatewayRelease>(
            crate::models::nyxbot_gateway_release::COLLECTION_NAME,
        )
        .find(doc! {"channel_id":&row.id,"state":"due"})
        .await
        .map_err(|_| "storage_unavailable")?
        .try_collect::<Vec<_>>()
        .await
        .map_err(|_| "storage_unavailable")?
        .into_iter()
        .map(|r| r.id)
        .collect::<Vec<_>>();
    releases::try_inline(state, &ids).await;
    Ok(())
}
