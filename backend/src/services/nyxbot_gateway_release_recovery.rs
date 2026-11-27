//! Read-only re-observation of parked Discord obligations. Old replicas leave
//! these existing states alone. Only original deletion authority can become due;
//! explicit preservation never grants permission to DELETE.
use super::*;

/// Closed diagnostic classes; never serialize upstream prose or identifiers.
#[derive(Clone, Copy, Debug, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Answer {
    NotRequested,
    NotFound,
    TerminalRefusalNotFound,
    CreateFencedAbsent,
    MatchingRecord,
    DifferentVersion,
    AuthRefused,
    TransportError,
    ServerError,
    InvalidResponse,
    RequestRefused,
}
impl Answer {
    pub(super) fn from_response(result: &AppResult<gateway::GatewayResponse>) -> Self {
        match result {
            Ok(r) => match r.status {
                404 => Self::NotFound,
                401 | 403 => Self::AuthRefused,
                409 => Self::DifferentVersion,
                500..=599 => Self::ServerError,
                200..=299 => Self::MatchingRecord,
                _ => Self::RequestRefused,
            },
            Err(AppError::ValidationError(code)) if code == "gateway_invalid_response" => {
                Self::InvalidResponse
            }
            Err(_) => Self::TransportError,
        }
    }
}

pub(crate) fn reason(code: Option<&str>) -> &'static str {
    match code {
        Some("create_not_observed" | "create_outcome_unconfirmed") => "create_outcome_unconfirmed",
        Some("auth_refused" | "gateway_authority_refused") => "auth_refused",
        Some("destination_changed" | "gateway_destination_changed") => "destination_changed",
        Some("live_reference") => "live_reference",
        Some("gateway_provider_setup_required") => "gateway_provider_setup_required",
        Some("gateway_setup_custody_required") => "gateway_setup_custody_required",
        Some("changed_version" | "remote_version_changed") => "changed_version",
        Some("owner_unavailable" | "owner_authority_unavailable") => "owner_unavailable",
        Some("preserved" | "remote_preserved") => "preserved",
        Some("identity_unconfirmed" | "remote_identity_invalid" | "retirement_proof_missing") => {
            "identity_unconfirmed"
        }
        Some("native_release_unresolved" | "native_source_draining") => "native_release_unresolved",
        Some("deletion_unconfirmed" | "release_unconfirmed") => "deletion_unconfirmed",
        Some("unavailable" | "gateway_unavailable" | "release_attempt_failed") => "unavailable",
        Some("retry_original") => "retry_original",
        _ => "unspecified",
    }
}

pub(super) async fn failed_audit(
    db: &Database,
    tx: &mut ClientSession,
    release: &Release,
    state: ReleaseState,
    code: &str,
    answer: Answer,
    retry_at: chrono::DateTime<Utc>,
) -> AppResult<()> {
    let answer = serde_json::to_value(answer)
        .map_err(|_| conflict())?
        .as_str()
        .ok_or_else(conflict)?
        .to_owned();
    let code = reason(Some(code));
    let now = Utc::now();
    if release.failure_audit.as_ref().is_some_and(|last| {
        last.state == state
            && last.code == code
            && last.gateway_answer == answer
            && now - last.at < Duration::hours(24)
    }) {
        return Ok(());
    }
    let stamp = crate::models::nyxbot_gateway_release::ReleaseFailureAudit {
        state,
        code: code.to_owned(),
        gateway_answer: answer.clone(),
        at: now,
    };
    db.collection::<Document>(RELEASES)
        .update_one(
            doc! {"_id":&release.id},
            doc! {"$set":{"failure_audit":bson::to_bson(&stamp).map_err(|_|conflict())?}},
        )
        .session(&mut *tx)
        .await?;
    audit(
        db, tx, &release.owner_user_id, "nyxbot_gateway_release_failed",
        json!({
            "obligation_id": release.id, "channel_id": release.channel_id,
            "channel_bot_id": release.channel_bot_id,
            "state": state, "previous_state": release.state,
            "stored_reason": if release.state == ReleaseState::Settled {"preserved"} else {reason(release.review_reason.as_deref().or(release.last_error.as_deref()))},
            "code": code, "gateway_answer": answer,
            "attempt": release.attempt_count, "retry_at": retry_at,
        }),
    ).await
}

fn fence(row: &Release) -> Document {
    doc! {"_id":&row.id, "state":bson::to_bson(&row.state).expect("release state"),
    "revision":row.revision, "claim_id":&row.claim_id,
    "$expr":{"$gt":["$lease_until","$$NOW"]}}
}

async fn claim_review(db: &Database) -> AppResult<Option<Release>> {
    // The dormant native guard intentionally lacks deletion authority. Its
    // parent is the only row eligible for observation and versioned cleanup.
    let filter = doc! {"platform":"discord", "native_drain_for":bson::Bson::Null,
    "$or":[{"state":"review_required"}, {"state":"settled",
        "resolution":{"$nin":accepted_resolutions_bson()}}],
    "$expr":{"$and":[
        {"$lte":[{"$ifNull":["$next_attempt_at",bson::DateTime::from_millis(0)]},"$$NOW"]},
        {"$lte":[{"$ifNull":["$lease_until",bson::DateTime::from_millis(0)]},"$$NOW"]}]}};
    Ok(db
        .collection::<Release>(RELEASES)
        .find_one_and_update(
            filter,
            vec![doc! {"$set":{
                "claim_id":Uuid::new_v4().to_string(),
                "lease_until":{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":120}},
                "revision":{"$add":[{"$ifNull":["$revision",0]},1]},
                "attempt_count":{"$add":[{"$ifNull":["$attempt_count",0]},1]},
            }}],
        )
        .sort(doc! {"next_attempt_at":1,"_id":1})
        .return_document(ReturnDocument::After)
        .await?)
}

async fn reobserve(state: &AppState, row: Release) -> AppResult<()> {
    let ticket = state
        .shutdown
        .register_release(super::super::shutdown::Release {
            safe_after_stop: false,
            db: state.db.clone(),
            collection: RELEASES,
            filter: doc! {"_id":&row.id,"claim_id":&row.claim_id,"revision":row.revision},
            update: doc! {"$set":{"lease_until":bson::DateTime::from_millis(0)}},
        });
    let (observation, answer) = observe_with_answer(state, &row).await;
    let preserved = row.state == ReleaseState::Settled;
    let (next, code, version) = match observation {
        // A known identity was already acknowledged. Its positive absence has
        // the same meaning as in the normal worker, including after preservation.
        Ok(Observation::Missing) if row.gateway_channel_id.is_some() => {
            (ReleaseState::Settled, "missing", None)
        }
        Ok(Observation::Matching { .. }) if !preserved => {
            (ReleaseState::Due, "retry_original", None)
        }
        Ok(Observation::Matching { version, .. }) => (row.state, "remote_preserved", Some(version)),
        Ok(Observation::Missing) => (row.state, "create_outcome_unconfirmed", None),
        Ok(Observation::Changed { version }) => (row.state, "remote_version_changed", version),
        Ok(Observation::DestinationChanged) => (row.state, "gateway_destination_changed", None),
        Ok(Observation::LiveReference) => (row.state, "live_reference", None),
        Ok(Observation::OwnerUnavailable) => (row.state, "owner_authority_unavailable", None),
        Ok(Observation::Malformed) => (row.state, "remote_identity_invalid", None),
        Err(AppError::Forbidden(_)) => (row.state, "gateway_authority_refused", None),
        Err(_) => (row.state, "gateway_unavailable", None),
    };
    let settled = code == "missing";
    // Own every transaction capture, including codes that currently happen
    // to be literals; the retryable async closure must not capture &str.
    let code = code.to_owned();
    let db = state.db.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |tx| {
        let work: AppResult<()> = Box::pin(async {
            // Even a read-only recovery cannot settle a newly referenced row.
            let live = db.collection::<Document>(CHANNELS).find_one(live_filter(&row))
                .session(&mut *tx).await?.is_some();
            let (next, code, settled) = if live {
                (row.state, "live_reference".to_owned(), false)
            } else { (next, code.clone(), settled) };
            let now = Utc::now();
            let retry_at = now + Duration::seconds(backoff(row.attempt_count));
            let mut set = doc! {"state":bson::to_bson(&next).map_err(|_|conflict())?,
                "last_error":&code,"last_gateway_answer":bson::to_bson(&answer).map_err(|_|conflict())?,"observed_version":version,"claim_id":bson::Bson::Null,
                "lease_until":bson::Bson::Null,"updated_at":bson::DateTime::from_chrono(now),
                "next_attempt_at":bson::DateTime::from_chrono(if next==ReleaseState::Due {now} else {retry_at})};
            if row.review_reason.is_none() {
                set.insert("review_reason", reason(row.last_error.as_deref()));
            }
            if settled {
                set.insert("resolution", "missing");
                set.insert("settled_at", bson::DateTime::from_chrono(now));
            }
            if db.collection::<Document>(RELEASES).update_one(fence(&row),
                doc! {"$set":set,"$inc":{"revision":1_i64}}).session(&mut *tx).await?.matched_count != 1 {
                return Ok(());
            }
            if settled || next==ReleaseState::Due {
                audit(&db, tx, &row.owner_user_id,
                    if settled {"nyxbot_gateway_release_settled"} else {"nyxbot_gateway_release_queued"},
                    json!({"obligation_id":row.id,"channel_id":row.channel_id,"reason":code})).await?;
            } else {
                failed_audit(&db, tx, &row, next, &code, answer, retry_at).await?;
            }
            Ok(())
        }).await;
        transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)?;
    ticket.completed();
    Ok(())
}

pub(super) async fn sweep(state: &AppState) {
    // A sequential bounded pass makes idle replicas perform one claim query,
    // rather than ten. Each network call retains the existing timeout.
    for _ in 0..10 {
        let Some(_work) = state.shutdown.background() else {
            break;
        };
        let row = match claim_review(&state.db).await {
            Ok(Some(row)) => row,
            Ok(None) => break,
            Err(_) => {
                tracing::warn!(
                    code = "gateway_release_recovery_claim_failed",
                    "Gateway release recovery claim deferred"
                );
                break;
            }
        };
        let obligation_id = row.id.clone();
        let channel_id = row.channel_id.clone();
        if reobserve(state, row).await.is_err() {
            tracing::warn!(%obligation_id, %channel_id,
                code = "gateway_release_recovery_row_failed", "Gateway release recovery row deferred");
        }
    }
}

/// Dormant guards on an adopted native channel are expected. An overdue guard
/// on a direct/retired channel must not remain silent if its parent is missing.
pub(super) async fn record_guard_deferred(db: &Database, guard: &Document) {
    if guard_deferred(db, guard).await.is_err() {
        tracing::warn!(
            obligation_id = guard.get_str("_id").unwrap_or("unknown"),
            code = "gateway_release_guard_diagnostic_failed",
            "Gateway release guard diagnostic deferred"
        );
    }
}

async fn guard_deferred(db: &Database, guard: &Document) -> AppResult<()> {
    let created = guard
        .get_datetime("created_at")
        .map_err(|_| conflict())?
        .to_chrono();
    if Utc::now() - created < Duration::minutes(SETUP_MINUTES)
        || guard
            .get_datetime("next_attempt_at")
            .is_ok_and(|at| at.to_chrono() > Utc::now())
    {
        return Ok(());
    }
    let db = db.clone();
    let guard = guard.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |tx| {
        let work: AppResult<()> = Box::pin(async {
            if db.collection::<Document>(CHANNELS).find_one(doc! {
                "_id":guard.get_str("channel_id").map_err(|_|conflict())?,
                "status":{"$in":["active","pending"]}, "transport":"gateway",
                "binding_id":guard.get_str("native_binding_id").map_err(|_|conflict())?,
            }).session(&mut *tx).await?.is_some() { return Ok(()); }
            let retry_at = Utc::now()+Duration::hours(1);
            let changed = db.collection::<Release>(RELEASES).find_one_and_update(doc! {
                "_id":guard.get("_id").cloned().ok_or_else(conflict)?,"state":"review_required",
                "$expr":{"$lte":[{"$ifNull":["$next_attempt_at",bson::DateTime::from_millis(0)]},"$$NOW"]},
            },doc! {"$set":{"next_attempt_at":bson::DateTime::from_chrono(retry_at)},"$inc":{"revision":1_i64}})
                .session(&mut *tx).await?;
            if let Some(row) = changed {
                failed_audit(&db,tx,&row, row.state, "native_release_unresolved", Answer::NotRequested, retry_at).await?;
            }
            Ok(())
        }).await;
        transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[tokio::test]
    async fn legacy_claim_without_optional_timestamps_or_diagnostics_deserializes() {
        let db = crate::test_utils::connect_transaction_test_database("release_legacy_claim").await;
        let id = Uuid::new_v4().to_string();
        db.collection::<Document>(RELEASES)
            .insert_one(doc! {
                "_id":&id,"schema_version":1,"owner_user_id":"owner","channel_id":"channel",
                "channel_bot_id":"bot","bot_owner_id":"owner","platform":"discord",
                "gateway_service_id":"gateway","gateway_base_url":"http://localhost:1",
                "creator_issuer":"issuer","binding_id":"binding","agent_api_key_id":"agent",
                "route_api_key_id":"route","state":"review_required","reason":"setup",
                "last_error":"create_outcome_unconfirmed","attempt_count":8_i64,"revision":0_i64,
                "dispatch_started_at":bson::DateTime::from_millis(0),
                "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
            })
            .await
            .unwrap();
        // retire_attempt reads this typed shape before the recovery claim.
        let read = db
            .collection::<Release>(RELEASES)
            .find_one(doc! {"_id":&id})
            .await
            .unwrap()
            .unwrap();
        assert!(read.next_attempt_at.is_none());
        assert!(read.failure_audit.is_none());
        db.collection::<Document>(RELEASES)
            .update_one(doc! {"_id":&id}, doc! {"$unset":{"revision":""}})
            .await
            .unwrap();
        // The claim pipeline also tolerates missing revision; its typed return
        // must not fail after acquiring the lease on a legacy row.
        let claimed = claim_review(&db).await.unwrap().unwrap();
        assert_eq!(claimed.id, id);
        assert_eq!(claimed.attempt_count, 9);
        assert!(claimed.next_attempt_at.is_none());
        assert!(claimed.review_reason.is_none());
        assert!(claimed.last_gateway_answer.is_none());
        assert!(claimed.failure_audit.is_none());
        assert!(claimed.claim_id.is_some());
        assert!(claimed.lease_until.unwrap() > Utc::now());
        assert!(claim_review(&db).await.unwrap().is_none());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn empty_recovery_pass_issues_one_indexed_claim() {
        use mongodb::event::{EventHandler, command::CommandEvent};
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = calls.clone();
        let handler = EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && event.command.get_str("findAndModify") == Ok(RELEASES)
            {
                seen.fetch_add(1, Ordering::SeqCst);
            }
        });
        let db =
            crate::test_utils::connect_test_database_with_command_handler("release_idle", handler)
                .await
                .unwrap();
        ensure_indexes(&db).await.unwrap();
        sweep(&crate::test_utils::test_app_state(db.clone())).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        db.drop().await.unwrap();
    }
}
