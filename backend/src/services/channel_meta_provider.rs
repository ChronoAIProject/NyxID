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

pub(super) fn grant_identity(platform: &str, app: &str, subject: &str) -> AppResult<String> {
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
    if let Some(proof) = db
        .collection::<bson::Document>("channel_meta_erasure_fences")
        .find_one(doc! {"_id":id})
        .session(&mut *tx)
        .await?
        && super::grants::completed(&proof, &binding.grant_id)?
    {
        super::scopes::mark_complete(db, tx, id, &binding.grant_id).await?;
        super::grants::clear_completed_retired(db, tx, &proof).await?;
        let changed=db.collection::<ChannelBot>(BOTS).delete_one(doc! {"_id":id,"user_id":owner,"is_active":false,
            "meta_binding.grant_id":&binding.grant_id,"meta_binding.generation":binding.generation,"meta_binding.subscription":"removed"}).session(&mut *tx).await?;
        if changed.deleted_count != 1 {
            return Err(conflict());
        }
        return Ok(());
    }
    let scope = super::scopes::ensure_current(db, tx, &bot, false).await?;
    if binding.grant_subject_id.is_empty() {
        // No provider subject can claim this manual installation. Account
        // retirement needs no permanent managed grant or scope obligation.
        db.collection::<bson::Document>(crate::models::channel_meta::GRANT_SCOPES)
            .delete_one(doc! {"_id":&scope.id,"identity":null,"outstanding":false})
            .session(&mut *tx)
            .await?;
    } else {
        let changed=db.collection::<bson::Document>(crate::models::channel_meta::GRANT_SCOPES).update_one(doc! {"_id":&scope.id,"status":"current"},
            doc! {"$set":{"status":"retired","to_generation":binding.generation,"sealed_at":bson::DateTime::now()}}).session(&mut *tx).await?;
        if changed.matched_count != 1 && scope.status != "completed" {
            return Err(conflict());
        }
    }
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
                    let cutoff = bson::DateTime::from_chrono(*issued + Duration::seconds(1));
                    db.collection::<bson::Document>(REQUESTS).update_one(
                        doc! {"_id":existing.get("_id").cloned(),"request_hash":hash},
                        doc! {"$set":{"signed_cutoff":cutoff}}).session(&mut *tx).await?;
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
                if *erase {
                    db.collection::<bson::Document>("channel_meta_grant_authority").update_one(doc! {"identity":&identity},
                        doc! {"$max":{"erasure_cutoff":bson::DateTime::from_chrono(*issued+Duration::seconds(1))}}).session(&mut *tx).await?;
                }
                db.collection::<bson::Document>("channel_meta_grant_authority")
                    .update_one(doc! {"identity":&identity},super::grants::authority_state()).session(&mut *tx).await?;
                // Meta dates callbacks to the second. Cover the entire second
                // deterministically rather than missing a millisecond grant.
                let cutoff = bson::DateTime::from_chrono(*issued + Duration::seconds(1));
                let sessions = doc! {"platform":platform,"app_id":app,"grant_subject_id":subject,"created_at":{"$lt":cutoff}};
                let mut sessions=db.collection::<bson::Document>(SESSIONS).find(sessions).projection(doc! {"_id":1}).limit(101).session(&mut *tx).await?;
                let sessions:Vec<bson::Document>=sessions.stream(&mut *tx).try_collect().await?;
                let session_overflow=sessions.len()>100;
                let ids:Vec<_>=sessions.iter().take(100).map(|row|row.get("_id").cloned()).collect();
                if !ids.is_empty() {
                    if *erase {db.collection::<OAuthSession>(SESSIONS).delete_many(doc! {"_id":{"$in":ids}}).session(&mut *tx).await?;}
                    else {db.collection::<OAuthSession>(SESSIONS).update_many(doc! {"_id":{"$in":ids},"status":{"$in":["pending","exchanging","assets","installing"]}},
                        doc! {"$set":{"status":"revoked"},"$unset":{"assets_encrypted":""}}).session(&mut *tx).await?;}
                }
                let mut cursor = db.collection::<bson::Document>(BOTS).find(doc! {
                    "platform":platform,"meta_binding.app_id":app,"meta_binding.grant_subject_id":subject,
                }).sort(doc! {"_id":1}).limit(101).session(&mut *tx).await?;
                let bots: Vec<bson::Document> = cursor.stream(&mut *tx).try_collect().await?;
                let mut compatibility_pending=bots.len()>100 || session_overflow;
                let mut targets = Vec::new();
                let mut seen=std::collections::BTreeSet::new();
                let mut excluded_scopes=Vec::<String>::new();
                for raw in bots.into_iter().take(100) {
                    let id = raw.get_str("_id").map_err(|_|invalid())?;
                    let binding = raw.get_document("meta_binding").map_err(|_|invalid())?;
                    let grant = binding.get_str("grant_id").map_err(|_|invalid())?;
                    match binding.get_datetime("authorized_at") {
                        Ok(at) if at >= &cutoff => continue,
                        Ok(_) => {
                            let bot: ChannelBot = bson::from_document(raw.clone()).map_err(|_|invalid())?;
                            if !apply_callback_state(db,tx,&bot,*erase,&cutoff).await? {continue;}
                        }
                        Err(_) => {} // Exact identity, unresolved chronology: no destructive effect.
                    }
                    let mut target=doc! {"bot_id":id,"grant_id":grant};
                    if let Some(scope)=db.collection::<bson::Document>(crate::models::channel_meta::GRANT_SCOPES)
                        .find_one(doc! {"bot_id":id,"grant_id":grant}).projection(doc! {"_id":1}).session(&mut *tx).await?
                        && let Ok(scope_id)=scope.get_str("_id") {
                        target.insert("scope_id",scope_id);
                        excluded_scopes.push(scope_id.into());
                    }
                    seen.insert((id.to_owned(),grant.to_owned()));
                    targets.push(target);
                }
                if *erase {
                    let mut retired = db.collection::<bson::Document>("channel_meta_erasure_fences")
                        .find(doc! {"retired_grant.identity":&identity})
                        .projection(doc! {"retired_grant":1,"local_data_erased_at":1,"completed_grant_id":1}).sort(doc! {"_id":1}).limit(101).session(&mut *tx).await?;
                    let mut count=0;
                    while let Some(row) = retired.stream(&mut *tx).try_next().await? {
                        count+=1;
                        if count>100 {compatibility_pending=true;break;}
                        let id = row.get_str("_id").map_err(|_|invalid())?;
                        let grant = row.get_document("retired_grant").and_then(|r|r.get_str("grant_id")).map_err(|_|invalid())?;
                        let retired = row.get_document("retired_grant").map_err(|_|invalid())?;
                        if retired.get_datetime("authorized_at").is_ok_and(|at|at >= &cutoff) { continue; }
                        if super::grants::completed(&row,grant).unwrap_or(false) { continue; }
                        if seen.insert((id.to_owned(),grant.to_owned())) {
                            if targets.len()==100 {compatibility_pending=true;break;}
                            targets.push(doc! {"bot_id":id,"grant_id":grant});
                        }
                    }
                }
                db.collection::<bson::Document>(REQUESTS).insert_one(doc! {
                    "_id": id, "request_hash": hash, "platform": platform,
                    "app_id": app,
                    "erase": erase, "signed_cutoff":cutoff, "status": "pending", "targets": targets,
                    "scope_capture":{"identity":&identity,"done":false,"sessions_done":false,"excluded":excluded_scopes},
                    "compatibility_pending":compatibility_pending,
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
    for _ in 0..2 {
        super::targets::expire_completed_receipts(service.db).await?;
    }
    Ok(())
}

/// Capped compatibility adoption for pre-scope draft records. Only an overflow
/// uses this indexed app walk; new installations already have scope identity.
/// The request retains keyed identity and cursors, never the raw subject.
async fn capture_compatibility(
    db: &mongodb::Database,
    job: &bson::Document,
    claim: &str,
) -> AppResult<bson::Document> {
    if job.get_bool("compatibility_pending") != Ok(true)
        || !job.get_array("targets").map_err(|_| invalid())?.is_empty()
    {
        return Ok(job.clone());
    }
    super::super::transaction_service::run_db_only(db,&(db,job,claim),|tx,&(db,job,claim)| {
        Box::pin(async move {
            let capture=job.get_document("scope_capture").map_err(|_|invalid())?;
            let identity=capture.get_str("identity").map_err(|_|invalid())?;
            let platform=job.get_str("platform").map_err(|_|invalid())?;
            let app=job.get_str("app_id").map_err(|_|invalid())?;
            let retired=capture.get_str("compatibility_phase")==Ok("retired");
            let sessions=capture.get_str("compatibility_phase")==Ok("sessions");
            let mut filter=if retired {doc! {"retired_grant.identity":identity}} else if sessions {doc! {"platform":platform,"app_id":app}} else {doc! {"platform":platform,"meta_binding.app_id":app}};
            if let Ok(after)=capture.get_str("compatibility_after") {filter.insert("_id",doc! {"$gt":after});}
            let collection=if retired {"channel_meta_erasure_fences"} else if sessions {SESSIONS} else {BOTS};
            let mut cursor=db.collection::<bson::Document>(collection).find(filter).sort(doc! {"_id":1}).limit(SCOPE_BATCH).session(&mut *tx).await?;
            let rows:Vec<bson::Document>=cursor.stream(&mut *tx).try_collect().await?;
            let mut targets=Vec::new();
            if !retired {
                db.collection::<bson::Document>("channel_meta_grant_authority").update_one(doc! {"identity":identity},doc! {"$inc":{"revision":1_i64}}).session(&mut *tx).await?;
            }
            for row in &rows {
                if sessions {
                    let Some(subject)=row.get_str("grant_subject_id").ok() else {continue;};
                    if !subject.is_empty() && grant_identity(platform,app,subject)?==identity {
                        db.collection::<bson::Document>(SESSIONS).update_one(doc! {"_id":row.get("_id").cloned(),"grant_subject_id":subject},
                            doc! {"$set":{"grant_identity":identity}}).session(&mut *tx).await?;
                    }
                } else if retired {
                    let context=row.get_document("retired_grant").map_err(|_|super::grants::incomplete())?;
                    let grant=context.get_str("grant_id").map_err(|_|super::grants::incomplete())?;
                    if super::grants::completed(row,grant)? {continue;}
                    targets.push(doc! {"bot_id":row.get_str("_id").map_err(|_|invalid())?,"grant_id":grant});
                } else {
                    let binding=row.get_document("meta_binding").map_err(|_|super::grants::incomplete())?;
                    let subject=binding.get_str("grant_subject_id").map_err(|_|super::grants::incomplete())?;
                    if subject.is_empty() || grant_identity(platform,app,subject)?!=identity {continue;}
                    let bot:ChannelBot=bson::from_document(row.clone()).map_err(|_|super::grants::incomplete())?;
                    // Retirement/cutover contends on this bot before attribution
                    // can disappear. No historical ownership is inferred here.
                    let changed=db.collection::<ChannelBot>(BOTS).update_one(doc! {"_id":&bot.id,
                        "meta_binding.grant_id":&binding.get_str("grant_id").map_err(|_|super::grants::incomplete())?,
                        "meta_binding.generation":bot.meta_binding.as_ref().map(|binding|binding.generation)},
                        doc! {"$inc":{"meta_observation_revision":1_i64}}).session(&mut *tx).await?;
                    if changed.matched_count!=1 {return Err(conflict());}
                    super::scopes::ensure_current(db,tx,&bot,false).await?;
                }
            }
            let exhausted=rows.len()<SCOPE_BATCH as usize;
            let mut fields=doc! {"targets":targets};
            let mut unset=doc! {};
            if exhausted && !retired && !sessions && job.get_bool("erase")==Ok(true) {
                fields.insert("scope_capture.compatibility_phase","retired");
                unset.insert("scope_capture.compatibility_after","");
            } else if exhausted && !sessions {
                fields.insert("scope_capture.compatibility_phase","sessions");
                unset.insert("scope_capture.compatibility_after","");
            } else if exhausted {
                fields.insert("compatibility_pending",false);
                unset.insert("scope_capture.compatibility_after","");
                unset.insert("scope_capture.compatibility_phase","");
            } else if let Some(last)=rows.last() {fields.insert("scope_capture.compatibility_after",last.get_str("_id").map_err(|_|invalid())?);}
            let updated=db.collection::<bson::Document>(REQUESTS).find_one_and_update(doc! {
                "_id":job.get("_id").cloned(),"claim_id":claim,"status":"pending","targets":{"$size":0},"scope_capture":capture,
            },doc! {"$set":fields,"$unset":unset}).return_document(mongodb::options::ReturnDocument::After).session(tx).await?
                .ok_or_else(conflict)?;
            Ok(updated)
        })
    }).await
}

async fn capture_sessions(
    db: &mongodb::Database,
    job: &bson::Document,
    claim: &str,
) -> AppResult<bson::Document> {
    if job.get_document("scope_capture").is_err()
        || job
            .get_document("scope_capture")
            .is_ok_and(|capture| capture.get_bool("sessions_done") != Ok(false))
    {
        return Ok(job.clone());
    }
    super::super::transaction_service::run_db_only(db,&(db,job,claim),|tx,&(db,job,claim)| {
        Box::pin(async move {
            let capture=job.get_document("scope_capture").map_err(|_|invalid())?;
            let mut filter=doc! {"grant_identity":capture.get_str("identity").map_err(|_|invalid())?};
            if let Ok(after)=capture.get_str("sessions_after") {filter.insert("_id",doc! {"$gt":after});}
            let mut cursor=db.collection::<bson::Document>(SESSIONS).find(filter).projection(doc! {"_id":1,"created_at":1,"status":1})
                .sort(doc! {"_id":1}).limit(SCOPE_BATCH).session(&mut *tx).await?;
            let rows:Vec<bson::Document>=cursor.stream(&mut *tx).try_collect().await?;
            let cutoff=job.get_datetime("signed_cutoff").map_err(|_|super::grants::incomplete())?;
            for row in &rows {
                if row.get_datetime("created_at").map_err(|_|super::grants::incomplete())?>=cutoff {continue;}
                let exact=doc! {"_id":row.get("_id").cloned(),"grant_identity":capture.get_str("identity").map_err(|_|invalid())?,"created_at":row.get("created_at").cloned()};
                if job.get_bool("erase")==Ok(true) {db.collection::<bson::Document>(SESSIONS).delete_one(exact).session(&mut *tx).await?;}
                else if matches!(row.get_str("status"),Ok("pending"|"exchanging"|"assets"|"installing")) {
                    db.collection::<bson::Document>(SESSIONS).update_one(exact,doc! {"$set":{"status":"revoked"},"$unset":{"assets_encrypted":""}}).session(&mut *tx).await?;
                }
            }
            let mut fields=doc! {"scope_capture.sessions_done":rows.len()<SCOPE_BATCH as usize};
            if let Some(last)=rows.last() {fields.insert("scope_capture.sessions_after",last.get_str("_id").map_err(|_|invalid())?);}
            db.collection::<bson::Document>(REQUESTS).find_one_and_update(doc! {"_id":job.get("_id").cloned(),"claim_id":claim,"scope_capture":capture},doc! {"$set":fields})
                .return_document(mongodb::options::ReturnDocument::After).session(tx).await?.ok_or_else(conflict)
        })
    }).await
}

const SCOPE_BATCH: i64 = 100;
pub(super) async fn capture_scopes(
    db: &mongodb::Database,
    job: &bson::Document,
    claim: &str,
) -> AppResult<bson::Document> {
    let job = capture_compatibility(db, job, claim).await?;
    if job.get_bool("compatibility_pending") == Ok(true) {
        return Ok(job);
    }
    let job = capture_sessions(db, &job, claim).await?;
    if job
        .get_document("scope_capture")
        .is_ok_and(|capture| capture.get_bool("sessions_done") == Ok(false))
    {
        return Ok(job);
    }
    let Some(capture) = job
        .get_document("scope_capture")
        .ok()
        .filter(|capture| capture.get_bool("done") == Ok(false))
    else {
        return Ok(job.clone());
    };
    if !job.get_array("targets").map_err(|_| invalid())?.is_empty() {
        return Ok(job.clone());
    }
    let identity = capture.get_str("identity").map_err(|_| invalid())?;
    let cutoff = *job
        .get_datetime("signed_cutoff")
        .map_err(|_| super::grants::incomplete())?;
    let context = (db, &job, claim, identity, cutoff);
    super::super::transaction_service::run_db_only(db,&context,|tx,&(db,job,claim,identity,cutoff)| {
        Box::pin(async move {
            let capture=job.get_document("scope_capture").map_err(|_|invalid())?;
            let mut id=doc! {"$nin":capture.get_array("excluded").cloned().unwrap_or_default()};
            if let Ok(after)=capture.get_str("after") {id.insert("$gt",after);}
            let mut cursor=db.collection::<bson::Document>(crate::models::channel_meta::GRANT_SCOPES)
                .find(doc! {"identity":identity,"outstanding":true,"_id":id})
                .sort(doc! {"_id":1}).limit(SCOPE_BATCH).session(&mut *tx).await?;
            let rows:Vec<bson::Document>=cursor.stream(&mut *tx).try_collect().await?;
            let mut targets=Vec::new();
            for stored in &rows {
                let row=super::scopes::synchronize_chronology(db,tx,stored).await?;
                if row.get_datetime("authorized_at").is_ok_and(|at|at>=&cutoff) {continue;}
                let bot=row.get_str("bot_id").map_err(|_|super::grants::incomplete())?;
                let grant=row.get_str("grant_id").map_err(|_|super::grants::incomplete())?;
                let scope=row.get_str("_id").map_err(|_|super::grants::incomplete())?;
                targets.push(doc! {"bot_id":bot,"grant_id":grant,"scope_id":scope});
                // Unknown chronology is captured but never fenced destructively.
                if job.get_bool("erase")==Ok(true) && row.get_datetime("authorized_at").is_ok() {
                    let changed=db.collection::<bson::Document>(crate::models::channel_meta::GRANT_SCOPES)
                        .update_one(doc! {"_id":scope,"authorized_at":row.get("authorized_at").cloned(),"outstanding":true},
                            doc! {"$set":{"erasure_requested":true},"$max":{"erasure_cutoff":cutoff},"$inc":{"revision":1_i64}})
                        .session(&mut *tx).await?;
                    if changed.matched_count!=1 {return Err(super::grants::incomplete());}
                }
            }
            let mut fields=doc! {"targets":targets,"scope_capture.done":rows.len()<SCOPE_BATCH as usize};
            if let Some(last)=rows.last() {fields.insert("scope_capture.after",last.get_str("_id").map_err(|_|invalid())?);}
            let changed=db.collection::<bson::Document>(REQUESTS).find_one_and_update(doc! {
                "_id":job.get("_id").cloned(),"claim_id":claim,"status":"pending","targets":{"$size":0},
                "scope_capture":capture,
            },doc! {"$set":fields}).return_document(mongodb::options::ReturnDocument::After).session(tx).await?
                .ok_or_else(conflict)?;
            Ok(changed)
        })
    }).await
}

async fn reconcile_job(
    state: &crate::AppState,
    service: &MetaService<'_>,
    job: &bson::Document,
    claim: &str,
) -> AppResult<()> {
    // Move each observed array page atomically. New and earlier request formats
    // both recover if a process stops between discovery and this handoff.
    let mut job = job.clone();
    for _ in 0..2 {
        job = super::targets::transfer_page(&state.db, &job, claim).await?;
    }
    if job.get_array("targets").is_ok_and(Vec::is_empty) {
        job = capture_scopes(&state.db, &job, claim).await?;
        job = super::targets::transfer_page(&state.db, &job, claim).await?;
    }
    let id = job.get_str("_id").map_err(|_| invalid())?;
    let erase = job.get_bool("erase").map_err(|_| invalid())?;
    for target in super::targets::due(&state.db, id, bson::DateTime::now()).await? {
        let bot = target.get_str("bot_id").map_err(|_| invalid())?;
        let grant = target.get_str("grant_id").map_err(|_| invalid())?;
        let result = reconcile_target(
            state,
            service,
            erase,
            bot,
            grant,
            job.get_datetime("signed_cutoff").ok(),
            target.get_str("scope_id").ok(),
        )
        .await;
        if let Err(error) = &result {
            tracing::warn!(
                error_code = error.error_code(),
                "Meta inbox cleanup deferred"
            );
        }
        super::targets::settle(
            &state.db,
            id,
            claim,
            &target,
            result.err().map(|error| error.error_code()),
        )
        .await?;
    }
    // A deferred child never blocks a later discovery page. Cursor and its
    // array handoff commit together; transfer to children is idempotent.
    if let Some(current) = state
        .db
        .collection::<bson::Document>(REQUESTS)
        .find_one(doc! {"_id":id,"status":"pending","claim_id":claim})
        .await?
        && current.get_array("targets").is_ok_and(Vec::is_empty)
    {
        job = capture_scopes(&state.db, &current, claim).await?;
        job = super::targets::transfer_page(&state.db, &job, claim).await?;
    }
    super::targets::finish_or_reschedule(&state.db, &job, claim).await
}

async fn reconcile_target(
    state: &crate::AppState,
    service: &MetaService<'_>,
    erase: bool,
    bot_id: &str,
    grant_id: &str,
    cutoff: Option<&bson::DateTime>,
    scope_id: Option<&str>,
) -> AppResult<()> {
    if !erase && cutoff.is_none() {
        return Err(super::grants::incomplete());
    }
    // Earlier queued jobs may omit scope_id. Resolve the captured bot/grant
    // before deciding whether any whole-inbox eraser is authorized.
    let mut scope_filter = doc! {"bot_id":bot_id,"grant_id":grant_id};
    if let Some(id) = scope_id {
        scope_filter.insert("_id", id);
    }
    let mut scope = state
        .db
        .collection::<bson::Document>(crate::models::channel_meta::GRANT_SCOPES)
        .find_one(scope_filter)
        .await?;
    if let Some(row) = scope.as_ref() {
        scope = Some(
            super::super::transaction_service::run_db_only(
                &state.db,
                &(&state.db, row),
                |tx, &(db, row)| Box::pin(super::scopes::synchronize_chronology(db, tx, row)),
            )
            .await?,
        );
    }
    if scope_id.is_some() && scope.is_none() {
        return Err(super::grants::incomplete());
    }
    if let Some(row) = &scope {
        if row.get_str("status") == Ok("completed") {
            return super::super::transaction_service::run_db_only(
                &state.db,
                &(&state.db, bot_id, grant_id),
                |tx, &(db, bot, grant)| Box::pin(super::scopes::mark_complete(db, tx, bot, grant)),
            )
            .await;
        }
        if let Some(cutoff) = cutoff {
            let authorized = row
                .get_datetime("authorized_at")
                .map_err(|_| super::grants::incomplete())?;
            if authorized >= cutoff {
                // A verified repair can exclude every request that claimed
                // this scope. Release only when the latest captured erasure
                // cutoff is also excluded; another request keeps its fence.
                if erase {
                    let id = row
                        .get_str("_id")
                        .map_err(|_| super::grants::incomplete())?;
                    state.db.collection::<bson::Document>(crate::models::channel_meta::GRANT_SCOPES).update_one(doc! {
                        "_id":id,"authorized_at":authorized,"erasure_cutoff":{"$type":"date","$lte":authorized},
                    },doc! {"$set":{"erasure_requested":false},"$unset":{"erasure_cutoff":""},"$inc":{"revision":1_i64}}).await?;
                }
                return Ok(());
            }
        }
    }
    if erase && super::scopes::has_history(&state.db, bot_id).await? {
        // The scoped eraser owns multi-grant records. Never fall through
        // to a whole-inbox purge or acknowledge unprocessed prior data.
        return Err(super::grants::incomplete());
    }
    let cutoff = if let Some(cutoff) = cutoff {
        cutoff
    } else {
        let proof = state
            .db
            .collection::<bson::Document>("channel_meta_erasure_fences")
            .find_one(doc! {"_id":bot_id})
            .await?
            .ok_or_else(super::grants::incomplete)?;
        if erase && super::grants::completed(&proof, grant_id)? {
            return super::super::personal_erasure_service::erase_provider_inbox(
                state, bot_id, grant_id,
            )
            .await;
        }
        return Err(super::grants::incomplete());
    };
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
            #[cfg(test)]
            super::super::durable_wake_tests::after("meta_provider_before_transition").await;
            let applied = super::super::transaction_service::run_db_only(
                &state.db,
                &(&state.db, &bot, erase, cutoff),
                |tx, &(db, bot, erase, cutoff)| {
                    Box::pin(async move { apply_callback_state(db, tx, bot, erase, cutoff).await })
                },
            )
            .await?;
            if !applied {
                return Ok(());
            }
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
    cutoff: &bson::DateTime,
) -> AppResult<bool> {
    // The live row and monotonic erasure bit belong to this transaction, not
    // the worker's earlier snapshot. A delayed deauth cannot undo deletion.
    let mut filter =
        doc! {"_id":&bot.id,"user_id":&bot.user_id,"platform_bot_id":&bot.platform_bot_id};
    filter.extend(super::grants::grant_filter(
        &bot.meta_binding.as_ref().ok_or_else(invalid)?.grant_id,
    ));
    let bot = db
        .collection::<ChannelBot>(BOTS)
        .find_one(filter)
        .session(&mut *tx)
        .await?
        .ok_or_else(conflict)?;
    let binding = bot.meta_binding.as_ref().ok_or_else(invalid)?;
    let authorized = binding
        .authorized_at
        .ok_or_else(super::grants::incomplete)?;
    let scope = super::scopes::ensure_current(db, tx, &bot, false).await?;
    let authorized = bson::DateTime::from_chrono(authorized);
    db.collection::<bson::Document>(crate::models::channel_meta::GRANT_SCOPES)
        .update_one(
            doc! {"_id":&scope.id},
            doc! {"$set":{"authorized_at":authorized}},
        )
        .session(&mut *tx)
        .await?;
    if authorized >= *cutoff {
        db.collection::<bson::Document>(crate::models::channel_meta::GRANT_SCOPES).update_one(doc! {"_id":&scope.id,
            "erasure_cutoff":{"$type":"date","$lte":authorized}},
            doc! {"$set":{"erasure_requested":false},"$unset":{"erasure_cutoff":""},"$inc":{"revision":1_i64}}).session(&mut *tx).await?;
        return Ok(false);
    }
    if erase {
        db.collection::<bson::Document>(crate::models::channel_meta::GRANT_SCOPES).update_one(doc! {"_id":&scope.id},
            doc! {"$set":{"erasure_requested":true},"$max":{"erasure_cutoff":cutoff},"$inc":{"revision":1_i64}}).session(&mut *tx).await?;
    }
    let advance = bot.is_active && !binding.provider_revoked;
    let keep = !erase && !binding.erasure_requested && bot.is_active;
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
    Ok(true)
}
