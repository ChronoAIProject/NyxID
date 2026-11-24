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
                "$set": {"platform": platform, "app_id": app},
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
                        "$setOnInsert": {"_id": uuid::Uuid::new_v4().to_string(), "identity": identity},
                    },
                ).upsert(true).session(&mut *tx).await?;
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
                let mut cursor = db.collection::<ChannelBot>(BOTS).find(doc! {
                    "platform": platform, "meta_binding.app_id": app,
                    "meta_binding.grant_subject_id": subject,
                    "meta_binding.authorized_at": {"$lt": cutoff},
                }).session(&mut *tx).await?;
                let bots: Vec<ChannelBot> = cursor.stream(&mut *tx).try_collect().await?;
                let mut targets = Vec::new();
                for bot in bots {
                    let binding = bot.meta_binding.as_ref().ok_or_else(invalid)?;
                    let generation = binding.generation;
                    let advance = bot.is_active && !binding.provider_revoked;
                    let keep_identity = !*erase && bot.is_active;
                    let subscription = if keep_identity && binding.operator_suspended {
                        "suspended"
                    } else if keep_identity {
                        "reconnect_required"
                    } else if binding.subscription == "removed" {
                        "removed"
                    } else {
                        "removal_pending"
                    };
                    let mut update = doc! {"$set": {
                        "is_active": keep_identity,
                        "status": if keep_identity && binding.operator_suspended {"suspended"} else if keep_identity {"failed"} else {"inactive"},
                        "webhook_registered": false,
                        "meta_binding.provider_revoked": binding.provider_revoked || !*erase,
                        "meta_binding.erasure_requested": binding.erasure_requested || *erase,
                        "meta_binding.grant_reconnect_required": binding.grant_reconnect_required || !*erase,
                        "meta_binding.subscription": subscription,
                        "meta_binding.next_reconcile_at": bson::DateTime::now(),
                        "updated_at": bson::DateTime::now(),
                    }};
                    if advance {
                        update.insert("$inc", doc! {"meta_binding.generation": 1_i64});
                    }
                    let changed = db.collection::<ChannelBot>(BOTS).update_one(
                        doc! {"_id": &bot.id, "meta_binding.generation": generation}, update,
                    ).session(&mut *tx).await?;
                    if changed.matched_count != 1 { return Err(conflict()); }
                    targets.push(doc! {
                        "bot_id": &bot.id, "grant_id": &binding.grant_id,
                        "generation": generation + i64::from(advance),
                    });
                }
                db.collection::<bson::Document>(REQUESTS).insert_one(doc! {
                    "_id": id, "request_hash": hash, "platform": platform,
                    "app_id": app,
                    "erase": erase, "status": "pending", "targets": targets,
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
        let update = match reconcile_target(state, service, erase, bot_id, grant_id).await {
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
) -> AppResult<()> {
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
            if bot.is_active
                && !erase
                && bot
                    .meta_binding
                    .as_ref()
                    .is_some_and(|binding| binding.provider_revoked)
            {
                // A verified deauth retires credentials but keeps the
                // reconnectable management identity for this grant.
                state.db.collection::<bson::Document>(BOTS).update_one(
                        doc! {"_id": &bot.id, "meta_binding.grant_id": grant_id, "meta_binding.provider_revoked": true},
                        doc! {"$set": {"bot_token_encrypted": binary(Vec::new())}},
                    ).await?;
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
