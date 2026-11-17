//! Platform-admin line operations. Suspension and retirement only reduce
//! execution authority and never require readiness; resume admits new effects.
use super::*;

#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineRevisionInput {
    pub revision: i64,
}
#[derive(Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RetireInput {
    pub revision: i64,
    /// Separate operator record that the supplier no longer bills the number.
    #[serde(default)]
    pub supplier_released_at: Option<DateTime<Utc>>,
}
#[derive(Clone, Debug, Serialize)]
pub struct LineOperation {
    pub line_id: String,
    pub claim_id: Option<String>,
    pub revision: i64,
    pub state: LineState,
    pub suspension: Option<Suspension>,
    /// Admitted calls still holding a slot. They release only on confirmed
    /// provider termination, so a positive count means termination is pending.
    pub live_calls: u64,
    pub termination_pending: bool,
}

async fn find_line(
    db: &Database,
    tx: &mut ClientSession,
    id: &str,
    revision: i64,
) -> AppResult<ManagedPhoneLine> {
    db.collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .find_one(doc! {"_id":id,"revision":revision})
        .session(tx)
        .await?
        .ok_or_else(|| conflict("Managed line revision changed"))
}
async fn owner_fence_only(db: &Database, tx: &mut ClientSession, owner: &str) -> AppResult<()> {
    if db
        .collection::<Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id":owner},
            doc! {"$inc":{"channel_adapter_admission_fence":1_i64}},
        )
        .session(tx)
        .await?
        .matched_count
        != 1
    {
        return Err(unavailable());
    }
    Ok(())
}
async fn outcome(
    state: &AppState,
    line_id: &str,
    claim_id: Option<String>,
) -> AppResult<LineOperation> {
    let line = line(&state.db, line_id).await?;
    let live = match &claim_id {
        Some(c) => super::super::listing::live_calls(&state.db, std::slice::from_ref(c))
            .await?
            .get(c)
            .copied()
            .unwrap_or(0),
        None => 0,
    };
    Ok(LineOperation {
        line_id: line.id,
        claim_id,
        revision: line.revision,
        state: line.state,
        suspension: line.suspension,
        live_calls: live,
        termination_pending: live > 0,
    })
}

/// Suspend an available or claimed line. Bindings are disabled and session
/// leases/generations fenced in the same transaction, so termination is durably
/// requested; immediate hangups are best effort and recovery retries the rest.
pub async fn suspend(
    state: &AppState,
    actor: &AuditActor,
    line_id: &str,
    input: LineRevisionInput,
) -> AppResult<LineOperation> {
    let db = transaction_service::durable_database(&state.db);
    let context = (
        &db,
        actor,
        line_id,
        input.revision,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    );
    let claim = transaction_service::run_db_only(&db, &context, |tx, c| {
        Box::pin(async move {
            let &(db, actor, line_id, revision, key) = c;
            eligibility::admin_fence(db, tx, actor).await?;
            let l = find_line(db, tx, line_id, revision).await?;
            if !matches!(l.state, LineState::Available | LineState::Claimed)
                || l.suspension.is_some()
            {
                return Err(conflict(
                    "Only available or claimed managed lines can be suspended",
                ));
            }
            let now = Utc::now();
            let mut claim_id = None;
            if l.state == LineState::Claimed {
                let id = l.current_claim_id.as_deref().ok_or_else(unavailable)?;
                let c = db
                    .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
                    .find_one(doc! {"_id":id,"line_id":&l.id,"state":"claimed"})
                    .session(&mut *tx)
                    .await?
                    .ok_or_else(unavailable)?;
                owner_fence_only(db, tx, &c.user_id).await?;
                if db.collection::<Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":&c.id,"fence":c.fence,"state":"claimed"},doc!{"$set":{"state":"suspended","suspension":"admin","suspended_at":bson::DateTime::from_chrono(now)},"$inc":{"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(conflict("Managed claim changed during suspension"));}
                disable(db, tx, &c.id, now).await?;
                claim_id = Some(c.id);
            }
            if db.collection::<Document>(ManagedPhoneLine::COLLECTION_NAME).update_one(doc!{"_id":&l.id,"revision":l.revision,"fence":l.fence,"state":bson::to_bson(&l.state).map_err(|_|unavailable())?},doc!{"$set":{"state":"suspended","suspension":"admin","updated_at":bson::DateTime::from_chrono(now)},"$inc":{"revision":1_i64,"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(conflict("Managed line changed during suspension"));}
            audit_service::log_actor_event_in_session(
                db,
                tx,
                key,
                actor,
                "managed_line_suspended",
                serde_json::json!({"line_id":l.id,"claim_id":claim_id,"revision":l.revision+1}),
            )
            .await?;
            Ok(claim_id)
        })
    })
    .await?;
    if let Some(claim) = &claim
        && drain(state, claim).await.is_err()
    {
        tracing::warn!(line_id=%line_id,"Managed line termination pending recovery");
    }
    outcome(state, line_id, claim).await
}

/// Resume an administratively suspended line. A claim whose paid coverage ended
/// during suspension becomes `suspended{unpaid}` with its retention window
/// starting now; it is never back-charged or silently renewed.
pub async fn resume(
    state: &AppState,
    actor: &AuditActor,
    line_id: &str,
    input: LineRevisionInput,
) -> AppResult<LineOperation> {
    super::super::require_ready(&state.db).await?;
    let db = transaction_service::durable_database(&state.db);
    let context = (
        &db,
        actor,
        line_id,
        input.revision,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    );
    let claim = transaction_service::run_db_only(&db, &context, |tx, c| {
        Box::pin(async move {
            let &(db, actor, line_id, revision, key) = c;
            eligibility::admin_fence(db, tx, actor).await?;
            let l = find_line(db, tx, line_id, revision).await?;
            if l.state != LineState::Suspended || l.suspension != Some(Suspension::Admin) {
                return Err(conflict(
                    "Only administratively suspended managed lines can be resumed",
                ));
            }
            let now = Utc::now();
            let mut next = ("available", None::<&str>);
            if let Some(id) = &l.current_claim_id {
                let c = db
                    .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
                    .find_one(doc! {"_id":id,"line_id":&l.id,"state":"suspended","suspension":"admin"})
                    .session(&mut *tx)
                    .await?
                    .ok_or_else(unavailable)?;
                owner_fence_only(db, tx, &c.user_id).await?;
                let periods = db.collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME);
                let covered = periods.find_one(doc!{"claim_id":&c.id,"funded":true,"covered_from":{"$lte":bson::DateTime::from_chrono(now)},"covered_until":{"$gt":bson::DateTime::from_chrono(now)}}).session(&mut *tx).await?.is_some();
                if covered {
                    if db.collection::<Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":&c.id,"fence":c.fence},doc!{"$set":{"state":"claimed","suspension":bson::Bson::Null,"suspended_at":bson::Bson::Null},"$inc":{"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(conflict("Managed claim changed during resume"));}
                    db.collection::<Document>(BINDINGS).update_many(doc!{"credential.managed_claim_id":&c.id,"credential.managed_claim_generation":c.managed_claim_generation},doc!{"$set":{"enabled":true,"updated_at":bson::DateTime::from_chrono(now)},"$inc":{"revision":1_i64}}).session(&mut *tx).await?;
                    next = ("claimed", None);
                } else {
                    if db.collection::<Document>(ManagedLineClaim::COLLECTION_NAME).update_one(doc!{"_id":&c.id,"fence":c.fence},doc!{"$set":{"state":"suspended","suspension":"unpaid","suspended_at":bson::DateTime::from_chrono(now)},"$inc":{"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(conflict("Managed claim changed during resume"));}
                    let last = periods.find_one(doc!{"claim_id":&c.id,"funded":true}).sort(doc!{"period_index":-1}).session(&mut *tx).await?.ok_or_else(unavailable)?;
                    sweep::notice(db, tx, &c, format!("unpaid:{}:{}", c.id, last.period_index), "unpaid", now).await?;
                    next = ("suspended", Some("unpaid"));
                }
            }
            if db.collection::<Document>(ManagedPhoneLine::COLLECTION_NAME).update_one(doc!{"_id":&l.id,"revision":l.revision,"fence":l.fence,"state":"suspended","suspension":"admin"},doc!{"$set":{"state":next.0,"suspension":next.1,"updated_at":bson::DateTime::from_chrono(now)},"$inc":{"revision":1_i64,"fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(conflict("Managed line changed during resume"));}
            audit_service::log_actor_event_in_session(
                db,
                tx,
                key,
                actor,
                "managed_line_admin_resumed",
                serde_json::json!({"line_id":l.id,"claim_id":l.current_claim_id,"revision":l.revision+1,"state":next.0,"suspension":next.1}),
            )
            .await?;
            Ok(l.current_claim_id.clone())
        })
    })
    .await?;
    outcome(state, line_id, claim).await
}

/// Retire unclaimed inventory locally, or record its separate supplier release.
/// Platform custody claims remain; a retired line never returns to inventory.
pub async fn retire(
    state: &AppState,
    actor: &AuditActor,
    line_id: &str,
    input: RetireInput,
) -> AppResult<LineOperation> {
    if input.supplier_released_at.is_some_and(|at| at > Utc::now()) {
        return Err(AppError::ValidationError(
            "Supplier release cannot be in the future".into(),
        ));
    }
    let db = transaction_service::durable_database(&state.db);
    let context = (
        &db,
        actor,
        line_id,
        input,
        state.audit_chain_hmac_key.as_ref().as_ref(),
    );
    transaction_service::run_db_only(&db, &context, |tx, c| {
        Box::pin(async move {
            let &(db, actor, line_id, input, key) = c;
            eligibility::admin_fence(db, tx, actor).await?;
            let l = find_line(db, tx, line_id, input.revision).await?;
            if l.current_claim_id.is_some() {
                return Err(conflict(
                    "Release the current claim before retiring this managed line",
                ));
            }
            let released = input
                .supplier_released_at
                .map(bson::DateTime::from_chrono)
                .map_or(bson::Bson::Null, bson::Bson::DateTime);
            let now = bson::DateTime::now();
            let (mut filter, set, event) = match (l.state, l.suspension) {
                (LineState::Retired, _) => {
                    if input.supplier_released_at.is_none() || l.supplier_released_at.is_some() {
                        return Err(conflict(
                            "Line is already retired; record a supplier release once",
                        ));
                    }
                    (
                        doc! {"state":"retired","supplier_released_at":bson::Bson::Null},
                        doc! {"supplier_released_at":released,"updated_at":now},
                        "managed_line_supplier_released",
                    )
                }
                (
                    LineState::Available | LineState::Cooldown | LineState::PendingReassignment,
                    None,
                )
                | (LineState::Suspended, Some(Suspension::Admin)) => (
                    doc! {"state":bson::to_bson(&l.state).map_err(|_|unavailable())?},
                    doc! {"state":"retired","suspension":bson::Bson::Null,"available_at":bson::Bson::Null,"supplier_released_at":released,"updated_at":now},
                    "managed_line_retired",
                ),
                _ => {
                    return Err(conflict(
                        "Only unclaimed, cooling down or administratively suspended lines can be retired",
                    ));
                }
            };
            filter.insert("_id", l.id.as_str());
            filter.insert("revision", l.revision);
            filter.insert("fence", l.fence);
            filter.insert("current_claim_id", bson::Bson::Null);
            if db
                .collection::<Document>(ManagedPhoneLine::COLLECTION_NAME)
                .update_one(filter, doc! {"$set":set,"$inc":{"revision":1_i64,"fence":1_i64}})
                .session(&mut *tx)
                .await?
                .matched_count
                != 1
            {
                return Err(conflict("Managed line changed during retirement"));
            }
            audit_service::log_actor_event_in_session(
                db,
                tx,
                key,
                actor,
                event,
                serde_json::json!({"line_id":l.id,"revision":l.revision+1,"supplier_released":input.supplier_released_at.is_some()}),
            )
            .await?;
            Ok(())
        })
    })
    .await?;
    outcome(state, line_id, None).await
}
