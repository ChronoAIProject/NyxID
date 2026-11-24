//! Authenticated provider revocation/deletion requests. Local delivery closes
//! transactionally before any remote cleanup; only the captured grant is erased.
use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};

pub const REQUESTS: &str = "channel_meta_provider_requests";

fn invalid() -> AppError {
    AppError::ChannelWebhookVerificationFailed("Invalid Meta signed request".into())
}
pub(crate) fn verify_signed_request(
    secret: &str,
    request: &str,
) -> AppResult<(String, chrono::DateTime<Utc>)> {
    if request.len() > 16384 {
        return Err(invalid());
    }
    let (signature, payload) = request.split_once('.').ok_or_else(invalid)?;
    let signature = URL_SAFE_NO_PAD.decode(signature).map_err(|_| invalid())?;
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| invalid())?;
    mac.update(payload.as_bytes());
    mac.verify_slice(&signature).map_err(|_| invalid())?;
    let value: Value =
        serde_json::from_slice(&URL_SAFE_NO_PAD.decode(payload).map_err(|_| invalid())?)
            .map_err(|_| invalid())?;
    if value["algorithm"].as_str() != Some("HMAC-SHA256") {
        return Err(invalid());
    }
    let subject = meta::identity_value(&value["user_id"]).ok_or_else(invalid)?;
    let at = value["issued_at"]
        .as_i64()
        .and_then(|t| chrono::DateTime::from_timestamp(t, 0))
        .ok_or_else(invalid)?;
    if at > Utc::now() + Duration::seconds(60) || at < Utc::now() - Duration::days(1) {
        return Err(invalid());
    }
    Ok((subject, at))
}

fn grant_identity(platform: &str, app: &str, subject: &str) -> AppResult<String> {
    // Retain only a keyed revocation fence after erasure. Public provider IDs
    // must not be recoverable by enumerating an unkeyed identifier digest.
    super::super::audit_service::keyed_fingerprint(
        super::super::audit_service::audit_chain_hmac_key(),
        b"meta-grant-authority-v1",
        format!("{platform}:{app}:{subject}").as_bytes(),
    )
    .ok_or_else(|| AppError::Internal("Meta grant authority is unavailable".into()))
}

/// Account deletion has owner authority, not provider-wide erasure authority.
/// Serialize removal with signed request capture and retain only the keyed
/// context needed to resolve existing or first late provider deletion targets.
pub(crate) async fn retire_account_inbox(
    db: &mongodb::Database,
    owner: &str,
    bot_id: &str,
) -> AppResult<()> {
    #[cfg(test)]
    super::super::durable_wake_tests::after("meta_account_before_retirement").await;
    let args = (db.clone(), owner.to_owned(), bot_id.to_owned());
    super::super::transaction_service::run_db_only(db, &args, |tx, (db, owner, id)| {
        Box::pin(retire_account_inbox_in_session(db, tx, owner, id))
    })
    .await
}

async fn retire_account_inbox_in_session(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    owner: &str,
    id: &str,
) -> AppResult<()> {
    super::super::org_channel_service::live_doc(
        db,
        tx,
        crate::models::user::COLLECTION_NAME,
        doc! {"_id":owner,"is_active":false,"platform_personal_deleting":true},
        true,
    )
    .await?;
    let Some(bot) = db
        .collection::<ChannelBot>(BOTS)
        .find_one(
            doc! {"_id":id,"user_id":owner,"is_active":false,"meta_binding.subscription":"removed"},
        )
        .session(&mut *tx)
        .await?
    else {
        if db
            .collection::<bson::Document>(BOTS)
            .find_one(doc! {"_id":id})
            .session(&mut *tx)
            .await?
            .is_none()
        {
            return Ok(());
        }
        return Err(conflict());
    };
    let binding = bot.meta_binding.as_ref().ok_or_else(conflict)?;
    db.collection::<bson::Document>("channel_meta_erasure_fences")
        .update_one(
            doc! {"_id":id},
            doc! {"$setOnInsert":{"requested_at":bson::DateTime::now()}},
        )
        .upsert(true)
        .session(&mut *tx)
        .await?;
    if binding.grant_subject_id.is_empty() {
        // A manual grant has no login subject to receive managed signed
        // callbacks. Do not invent a permanent authority for an empty ID.
        db.collection::<bson::Document>("channel_meta_erasure_fences")
            .update_one(
                doc! {"_id":id},
                doc! {"$set":{"requested_at":bson::DateTime::now()},"$unset":{"retired_grant":""}},
            )
            .upsert(true)
            .session(&mut *tx)
            .await?;
    } else {
        super::super::feature_flag_service::fence_meta_enrollment_in_session(db, &bot.platform, tx)
            .await?;
        let identity = grant_identity(&bot.platform, &binding.app_id, &binding.grant_subject_id)?;
        // The exact same write is acquired by accept_provider_request before
        // it reads either live bots or retired attribution. Neither order can
        // lose a captured obligation or acknowledge a wider successor grant.
        db.collection::<bson::Document>("channel_meta_grant_authority")
            .update_one(
                doc! {"identity":&identity},
                doc! {"$inc":{"revision":1_i64},
                "$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),"identity":&identity,
                    "platform":&bot.platform,"app_id":&binding.app_id,"outstanding":true}},
            )
            .upsert(true)
            .session(&mut *tx)
            .await?;
        if let Some(authorized) = binding.authorized_at {
            let second =
                chrono::DateTime::from_timestamp(authorized.timestamp(), 0).ok_or_else(invalid)?;
            db.collection::<bson::Document>("channel_meta_grant_authority")
                .update_one(
                    doc! {"identity":&identity},
                    doc! {"$max":{"last_authorized_at":bson::DateTime::from_chrono(second)}},
                )
                .session(&mut *tx)
                .await?;
            db.collection::<bson::Document>("channel_meta_grant_authority")
                .update_one(
                    doc! {"identity":&identity},
                    super::grants::authority_state(),
                )
                .session(&mut *tx)
                .await?;
        }
        let account = db
            .collection::<bson::Document>(crate::models::personal_erasure::COLLECTION_NAME)
            .find_one(doc! {"user_id":owner})
            .projection(doc! {"_id":1})
            .session(&mut *tx)
            .await?
            .ok_or_else(conflict)?;
        let retired = crate::models::channel_meta::RetiredGrant {
            grant_id: binding.grant_id.clone(),
            identity,
            platform: bot.platform.clone(),
            account_erasure_id: account.get_str("_id").map_err(|_| conflict())?.into(),
            bot_created_at: bot.created_at,
            authorized_at: binding.authorized_at,
        };
        let mut retired = bson::to_document(&retired).map_err(|_| invalid())?;
        retired.insert("app_id", &binding.app_id);
        let changed = db.collection::<bson::Document>("channel_meta_erasure_fences")
            .update_one(
                doc! {"_id":id,"local_data_erased_at":{"$exists":false},"completed_grant_id":{"$exists":false}},
                doc! {"$set":{"requested_at":bson::DateTime::now(),
                "retired_grant":retired}},
            )
            .session(&mut *tx)
            .await?;
        if changed.matched_count != 1 {
            let proof = db
                .collection::<bson::Document>("channel_meta_erasure_fences")
                .find_one(doc! {"_id":id})
                .session(&mut *tx)
                .await?
                .ok_or_else(super::grants::incomplete)?;
            if !super::grants::completed(&proof, &binding.grant_id)? {
                return Err(super::grants::incomplete());
            }
            super::grants::clear_completed_retired(db, tx, &proof).await?;
        }
    }
    for (collection, filter) in [
        (meta::CONTROLS, doc! {"bot_id":id}),
        (
            crate::models::coordination::EVENT_DEDUP_COLLECTION_NAME,
            doc! {"channel_bot_id":id},
        ),
        (SESSIONS, doc! {"bot_id":id}),
    ] {
        db.collection::<bson::Document>(collection)
            .delete_many(filter)
            .session(&mut *tx)
            .await?;
    }
    super::super::org_channel_service::defaults::erase_in_session(
        db,
        doc! {"channel_bot_id":id},
        tx,
    )
    .await?;
    if db
        .collection::<ChannelBot>(BOTS)
        .delete_one(doc! {"_id":id,"user_id":owner,
        "is_active":false,"meta_binding.grant_id":&binding.grant_id,
        "meta_binding.generation":binding.generation,"meta_binding.subscription":"removed"})
        .session(&mut *tx)
        .await?
        .deleted_count
        != 1
    {
        return Err(conflict());
    }
    Ok(())
}

pub(crate) async fn fence_grant_in_session(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    platform: &str,
    app: &str,
    subject: &str,
    started: chrono::DateTime<Utc>,
) -> AppResult<()> {
    let identity = grant_identity(platform, app, subject)?;
    let started_second =
        chrono::DateTime::from_timestamp(started.timestamp(), 0).ok_or_else(invalid)?;
    let row = db
        .collection::<bson::Document>("channel_meta_grant_authority")
        .find_one_and_update(
            doc! {"identity": &identity},
            doc! {
                "$inc": {"revision": 1_i64},
                "$set": {"platform": platform, "app_id": app,"outstanding":true},
                "$max": {"last_authorized_at": bson::DateTime::from_chrono(started_second)},
                "$setOnInsert": {"_id": uuid::Uuid::new_v4().to_string(), "identity": &identity},
            },
        )
        .upsert(true)
        .return_document(mongodb::options::ReturnDocument::After)
        .session(&mut *tx)
        .await?
        .ok_or_else(invalid)?;
    if row
        .get_datetime("revoked_at")
        .is_ok_and(|at| at.to_chrono() >= started_second)
    {
        return Err(meta::unreachable("reconnect_required"));
    }
    Ok(())
}

pub async fn accept_provider_request(
    service: &MetaService<'_>,
    platform: &str,
    request: &str,
    erase: bool,
) -> AppResult<Value> {
    // A deletion request only depends on the login identity and its signing
    // key. Clearing an unrelated webhook field must preserve this callback.
    let descriptor = adapter(platform)?
        .platform_credentials()
        .ok_or_else(invalid)?;
    let secrets = super::super::platform_credential_service::load_decrypted(
        service.db,
        service.keys,
        &descriptor,
    )
    .await?;
    let app = secrets.get("app_id").ok_or_else(invalid)?;
    let (subject, issued) =
        verify_signed_request(secrets.get("client_secret").ok_or_else(invalid)?, request)?;
    let request_hash = fingerprint(&format!("{platform}:{erase}:{request}"));
    let args = (
        service.db.clone(),
        platform.to_owned(),
        app.to_owned(),
        subject,
        issued,
        request_hash,
        erase,
        uuid::Uuid::new_v4().to_string(),
    );
    let id = super::super::transaction_service::run_db_only(
        service.db, &args,
        |tx, (db, platform, app, subject, issued, hash, erase, id)| {
            Box::pin(async move {
                if let Some(existing) = db.collection::<bson::Document>(REQUESTS)
                    .find_one(doc! {"request_hash": hash}).session(&mut *tx).await?
                {
                    return Ok(existing.get_str("_id").map_err(|_| invalid())?.to_owned());
                }
                let identity = grant_identity(platform, app, subject)?;
                // This write contends with exchange and installation. The
                // erasure bit is monotonic even if deauth arrives afterwards.
                db.collection::<bson::Document>("channel_meta_grant_authority").update_one(
                    doc! {"identity": &identity},
                    doc! {
                        "$inc": {"revision": 1_i64},
                        "$set": {"platform": platform,"app_id": app},
                        "$max": {"revoked_at": bson::DateTime::from_chrono(*issued), "erasure_requested": *erase},
                        "$setOnInsert": {"_id": uuid::Uuid::new_v4().to_string(), "identity": &identity},
                    },
                ).upsert(true).session(&mut *tx).await?;
                db.collection::<bson::Document>("channel_meta_grant_authority")
                    .update_one(doc! {"identity":&identity},super::grants::authority_state()).session(&mut *tx).await?;
                // Meta dates callbacks to the second. Cover the entire second
                // deterministically rather than missing a millisecond grant.
                let cutoff = bson::DateTime::from_chrono(*issued + Duration::seconds(1));
                let sessions = doc! {
                    "platform": platform, "app_id": app, "grant_subject_id": subject,
                    "created_at": {"$lt": cutoff},
                };
                if *erase {
                    db.collection::<OAuthSession>(SESSIONS).delete_many(sessions)
                        .session(&mut *tx).await?;
                } else {
                    let mut pending = sessions;
                    pending.insert("status", doc! {"$in": ["pending", "exchanging", "assets", "installing"]});
                    db.collection::<OAuthSession>(SESSIONS).update_many(
                        pending, doc! {"$set": {"status": "revoked"}, "$unset": {"assets_encrypted": ""}},
                    ).session(&mut *tx).await?;
                }
                let mut cursor = db.collection::<bson::Document>(BOTS).find(doc! {
                    "platform":platform,"meta_binding.app_id":app,"meta_binding.grant_subject_id":subject,
                }).session(&mut *tx).await?;
                let bots: Vec<bson::Document> = cursor.stream(&mut *tx).try_collect().await?;
                let mut targets = Vec::new();
                for raw in bots {
                    let id = raw.get_str("_id").map_err(|_|invalid())?;
                    let binding = raw.get_document("meta_binding").map_err(|_|invalid())?;
                    let grant = binding.get_str("grant_id").map_err(|_|invalid())?;
                    match binding.get_datetime("authorized_at") {
                        Ok(at) if at >= &cutoff => continue,
                        Ok(_) => {
                            let bot: ChannelBot = bson::from_document(raw.clone()).map_err(|_|invalid())?;
                            apply_callback_state(db,tx,&bot,*erase).await?;
                        }
                        Err(_) => {} // Exact identity, unresolved chronology: no destructive effect.
                    }
                    targets.push(doc! {"bot_id":id,"grant_id":grant});
                }
                if *erase {
                    let mut retired = db.collection::<bson::Document>("channel_meta_erasure_fences")
                        .find(doc! {"retired_grant.identity":&identity})
                        .projection(doc! {"retired_grant":1,"local_data_erased_at":1,"completed_grant_id":1}).session(&mut *tx).await?;
                    while let Some(row) = retired.stream(&mut *tx).try_next().await? {
                        let id = row.get_str("_id").map_err(|_|invalid())?;
                        let grant = row.get_document("retired_grant").and_then(|r|r.get_str("grant_id")).map_err(|_|invalid())?;
                        let retired = row.get_document("retired_grant").map_err(|_|invalid())?;
                        if retired.get_datetime("authorized_at").is_ok_and(|at|at >= &cutoff) { continue; }
                        if super::grants::completed(&row,grant).unwrap_or(false) { continue; }
                        if !targets.iter().any(|target|target.get_str("bot_id")==Ok(id) && target.get_str("grant_id")==Ok(grant)) {
                            targets.push(doc! {"bot_id":id,"grant_id":grant});
                        }
                    }
                }
                db.collection::<bson::Document>(REQUESTS).insert_one(doc! {
                    "_id": id, "request_hash": hash, "platform": platform,
                    "app_id": app,
                    "erase": erase, "signed_cutoff":cutoff, "status": "pending", "targets": targets,
                    "next_attempt_at": bson::DateTime::now(), "created_at": bson::DateTime::now(),
                }).session(&mut *tx).await?;
                Ok(id.clone())
            })
        },
    ).await?;
    Ok(
        json!({"confirmation_code":id,"url":format!("{}/api/v1/webhooks/channel/{platform}/deletions/{id}",service.config.base_url.trim_end_matches('/'))}),
    )
}

pub async fn request_status(db: &mongodb::Database, platform: &str, id: &str) -> AppResult<Value> {
    if uuid::Uuid::parse_str(id).is_err() {
        return Err(AppError::NotFound("Deletion request not found".into()));
    }
    let row = db
        .collection::<bson::Document>(REQUESTS)
        .find_one(doc! {"_id":id,"platform":platform,"erase":true})
        .await?
        .ok_or_else(|| AppError::NotFound("Deletion request not found".into()))?;
    Ok(json!({"status":row.get_str("status").unwrap_or("pending")}))
}

pub(crate) async fn reconcile_requests(state: &crate::AppState) -> AppResult<()> {
    reconcile_with_service(state, &MetaService::from(state)).await
}
pub(crate) async fn reconcile_with_service(
    state: &crate::AppState,
    service: &MetaService<'_>,
) -> AppResult<()> {
    super::grants::reconcile_authority_state(service.db).await?;
    for _ in 0..2 {
        let claim = uuid::Uuid::new_v4().to_string();
        let Some(job)=state.db.collection::<bson::Document>(REQUESTS).find_one_and_update(
            doc! {"status":"pending","next_attempt_at":{"$lte":bson::DateTime::now()}},
            doc! {"$set":{"claim_id": &claim, "next_attempt_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(15))}},
        ).sort(doc! {"next_attempt_at":1,"_id":1}).return_document(mongodb::options::ReturnDocument::After).await? else {break;};
        let result = reconcile_job(state, service, &job, &claim).await;
        if let Err(error) = result {
            tracing::warn!(
                error_code = error.error_code(),
                "Meta provider request reconciliation deferred"
            );
        }
    }
    Ok(())
}

async fn reconcile_job(
    state: &crate::AppState,
    service: &MetaService<'_>,
    job: &bson::Document,
    claim: &str,
) -> AppResult<()> {
    let job_id = job.get_str("_id").map_err(|_| invalid())?;
    let erase = job.get_bool("erase").map_err(|_| invalid())?;
    let requests = state.db.collection::<bson::Document>(REQUESTS);
    let claimed = doc! {"_id": job_id, "status": "pending", "claim_id": claim};
    let now = bson::DateTime::now();
    let retry_at = bson::DateTime::from_chrono(Utc::now() + Duration::minutes(15));
    let mut next_attempt = retry_at;
    let mut attempted = 0;
    for target in job.get_array("targets").map_err(|_| invalid())? {
        let target = target.as_document().ok_or_else(invalid)?;
        if let Ok(later) = target.get_datetime("retry_at")
            && later > &now
        {
            next_attempt = next_attempt.min(*later);
            continue;
        }
        if attempted == 8 {
            next_attempt = now;
            break;
        }
        attempted += 1;
        let bot_id = target.get_str("bot_id").map_err(|_| invalid())?;
        let grant_id = target.get_str("grant_id").map_err(|_| invalid())?;
        let identity = doc! {"bot_id": bot_id, "grant_id": grant_id};
        let mut filter = claimed.clone();
        filter.insert("targets", doc! {"$elemMatch": &identity});
        let update = match reconcile_target(
            state,
            service,
            erase,
            bot_id,
            grant_id,
            job.get_datetime("signed_cutoff")
                .map_err(|_| super::grants::incomplete())?,
        )
        .await
        {
            Ok(()) => doc! {"$pull": {"targets": identity}},
            Err(error) => {
                tracing::warn!(
                    error_code = error.error_code(),
                    "Meta inbox cleanup deferred"
                );
                doc! {"$set": {"targets.$.retry_at": retry_at}}
            }
        };
        if requests.update_one(filter, update).await?.matched_count == 0 {
            return Ok(());
        }
    }
    let mut empty = claimed.clone();
    empty.insert("targets", doc! {"$size": 0});
    let completed = requests
        .update_one(
            empty,
            doc! {
                "$set": {"status": "completed", "completed_at": bson::DateTime::now(),
                    "expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::days(90))},
                "$unset": {"targets": "", "claim_id": "", "next_attempt_at": ""}
            },
        )
        .await?;
    if completed.matched_count == 0 {
        requests
            .update_one(
                claimed,
                doc! {"$set": {"next_attempt_at": next_attempt}, "$unset": {"claim_id": ""}},
            )
            .await?;
    }
    Ok(())
}

async fn reconcile_target(
    state: &crate::AppState,
    service: &MetaService<'_>,
    erase: bool,
    bot_id: &str,
    grant_id: &str,
    cutoff: &bson::DateTime,
) -> AppResult<()> {
    let raw = state
        .db
        .collection::<bson::Document>(BOTS)
        .find_one(doc! {"_id":bot_id})
        .await?;
    if let Some(raw) = &raw {
        let binding = raw
            .get_document("meta_binding")
            .map_err(|_| super::grants::incomplete())?;
        if binding.get_str("grant_id") == Ok(grant_id) {
            let authorized = binding
                .get_datetime("authorized_at")
                .map_err(|_| super::grants::incomplete())?;
            if authorized >= cutoff {
                return Ok(());
            }
            let bot: ChannelBot =
                bson::from_document(raw.clone()).map_err(|_| super::grants::incomplete())?;
            super::super::transaction_service::run_db_only(
                &state.db,
                &(&state.db, &bot, erase),
                |tx, &(db, bot, erase)| {
                    Box::pin(async move { apply_callback_state(db, tx, bot, erase).await })
                },
            )
            .await?;
        }
    } else if erase {
        let proof = state
            .db
            .collection::<bson::Document>("channel_meta_erasure_fences")
            .find_one(doc! {"_id":bot_id})
            .await?
            .ok_or_else(super::grants::incomplete)?;
        if !super::grants::completed(&proof, grant_id)? {
            let retired = proof
                .get_document("retired_grant")
                .map_err(|_| super::grants::incomplete())?;
            if retired.get_str("grant_id") != Ok(grant_id) {
                return Err(super::grants::incomplete());
            }
            let authorized = retired
                .get_datetime("authorized_at")
                .map_err(|_| super::grants::incomplete())?;
            if authorized >= cutoff {
                return Ok(());
            }
        }
    }
    let bot = state
        .db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! {"_id": bot_id})
        .await?;
    match bot {
        Some(bot)
            if bot
                .meta_binding
                .as_ref()
                .is_some_and(|binding| binding.grant_id == grant_id) =>
        {
            if erase
                && bot.meta_binding.as_ref().is_some_and(|binding| {
                    !binding.grant_subject_id.is_empty() && binding.authorized_at.is_none()
                })
            {
                return Err(AppError::Conflict("The retained Meta grant authorization time needs verified repair before deletion can complete".into()));
            }
            if bot.is_active
                && !erase
                && bot
                    .meta_binding
                    .as_ref()
                    .is_some_and(|binding| binding.provider_revoked)
            {
                // A verified deauth retires credentials but keeps the
                // reconnectable management identity for this grant.
                let (filter, update) = credential_update(
                    &bot,
                    doc! {"_id": &bot.id, "meta_binding.grant_id":grant_id, "meta_binding.provider_revoked":true},
                    doc! {"bot_token_encrypted":binary(Vec::new())},
                    doc! {"secret_context_versions.bot_token_encrypted":""},
                )?;
                if state
                    .db
                    .collection::<ChannelBot>(BOTS)
                    .update_one(filter, update)
                    .await?
                    .matched_count
                    != 1
                {
                    return Err(conflict());
                }
            } else {
                if bot.is_active {
                    return Err(conflict());
                }
                service.disconnect(&bot).await?;
            }
            if erase {
                super::super::personal_erasure_service::erase_provider_inbox(
                    state, bot_id, grant_id,
                )
                .await?;
            }
        }
        None if erase => {
            // A crash after removing the bot must still clear any derived
            // records before the durable target can be acknowledged.
            super::super::personal_erasure_service::erase_provider_inbox(state, bot_id, grant_id)
                .await?;
        }
        _ => {} // A successor grant is outside this request's authority.
    }
    Ok(())
}

/// Caller has proved the exact grant lies before the authenticated cutoff.
/// Capture and delayed chronology recovery use the same local transition.
async fn apply_callback_state(
    db: &mongodb::Database,
    tx: &mut mongodb::ClientSession,
    bot: &ChannelBot,
    erase: bool,
) -> AppResult<()> {
    let binding = bot.meta_binding.as_ref().ok_or_else(invalid)?;
    let advance = bot.is_active && !binding.provider_revoked;
    let keep = !erase && bot.is_active;
    let subscription = if keep && binding.operator_suspended {
        "suspended"
    } else if keep {
        "reconnect_required"
    } else if binding.subscription == "removed" {
        "removed"
    } else {
        "removal_pending"
    };
    let mut update = doc! {"$set":{
    "is_active":keep,"status":if keep && binding.operator_suspended {"suspended"} else if keep {"failed"} else {"inactive"},
    "webhook_registered":false,"meta_binding.provider_revoked":binding.provider_revoked || !erase,
    "meta_binding.erasure_requested":binding.erasure_requested || erase,
    "meta_binding.grant_reconnect_required":binding.grant_reconnect_required || !erase,
    "meta_binding.subscription":subscription,"meta_binding.next_reconcile_at":bson::DateTime::now(),
    "updated_at":bson::DateTime::now()}};
    if advance {
        update.insert("$inc", doc! {"meta_binding.generation":1_i64});
    }
    if db
        .collection::<ChannelBot>(BOTS)
        .update_one(
            doc! {"_id":&bot.id,
            "meta_binding.grant_id":&binding.grant_id,"meta_binding.generation":binding.generation,
            "meta_binding.authorized_at":binding.authorized_at.map(bson::DateTime::from_chrono)},
            update,
        )
        .session(&mut *tx)
        .await?
        .matched_count
        != 1
    {
        return Err(conflict());
    }
    Ok(())
}
