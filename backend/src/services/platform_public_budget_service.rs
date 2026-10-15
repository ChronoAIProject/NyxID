//! Exact cap bookkeeping and a durable bridge to existing sponsor usage meters.
//! This module never debits a wallet or emits a second billing ledger posting.
use super::{
    api_key_mutation_service as tx,
    billing::{self, BillingRouteContext, MeteredProxyContext},
    platform_manager_contract::*,
    platform_public_policy::VerifiedPublicContract,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        credits::Credits,
        platform_public_budget::*,
        platform_public_session::{COLLECTION_NAME as SESSIONS, PlatformPublicSession},
        usage_meter::{COLLECTION_NAME as METERS, UsageMeterRow, UsageStatus},
    },
};
use chrono::Utc;
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database, IndexModel,
    bson::{self, Document, doc},
    options::{IndexOptions, ReturnDocument},
};
use sha2::{Digest, Sha256};

fn denied() -> AppError {
    AppError::Conflict("Public budget authority changed or exhausted".into())
}
fn state_bson(state: PublicCallState) -> AppResult<bson::Bson> {
    bson::to_bson(&state).map_err(|_| denied())
}

pub(crate) fn route_digest(ctx: &BillingRouteContext) -> AppResult<String> {
    let value = serde_json::json!([
        ctx.billing_request_id,
        ctx.billing_owner_id,
        ctx.actor_user_id,
        ctx.api_key_id,
        ctx.catalog_service_id,
        ctx.user_service_id,
        ctx.service_slug,
        ctx.auth_method,
        ctx.credential_class,
        ctx.operation,
        ctx.platform_specs()
            .map(|(m, c)| (m, c, ctx.estimated_quantity(m)))
            .collect::<Vec<_>>(),
        ctx.resale.as_ref().map(|r| (
            &r.lago_metric_code,
            r.metric,
            ctx.estimated_quantity(r.metric)
        )),
        ctx.public_admission.as_ref().map(|a| (
            &a.call_id,
            &a.model_id,
            a.expires_at.timestamp_millis()
        ))
    ]);
    Ok(hex::encode(Sha256::digest(
        serde_json::to_vec(&value).map_err(|_| denied())?,
    )))
}

pub(crate) struct CallQuote {
    route: BillingRouteContext,
    maximum: Credits,
    input: i64,
    output: i64,
}
impl CallQuote {
    pub(crate) fn maximum(&self) -> Credits {
        self.maximum
    }
}

/// Quote all active components and resale at the pinned model's exact rates.
/// Only a verified contract may authorize this local helper for production.
pub(crate) async fn quote_dispatch(
    state: &AppState,
    proof: &VerifiedPublicContract,
    context: &PublicExecutionContext,
    key_id: &str,
    mut route: BillingRouteContext,
    operation: &PublicModelOperation,
    call_id: &str,
) -> AppResult<CallQuote> {
    if proof.policy_digest() != context.policy_digest
        || route.billing_owner_id != context.sponsor_owner_user_id
        || route.actor_user_id != context.sponsor_owner_user_id
        || route.api_key_id.as_deref() != Some(key_id)
        || route.catalog_service_id.as_deref() != Some(operation.service_id.as_str())
        || route.user_service_id.is_some()
        || route.pool_attempt.is_some()
        || route.node_intent != billing::NodeIntent::Direct
        || route.credential_class != crate::models::usage_meter::CredentialClass::NyxidManagedMaster
        || route.request_bytes > i64::from(operation.max_input_bytes)
        || operation.max_input_tokens == 0
        || operation.max_output_tokens == 0
        || operation.max_response_bytes == 0
        || operation.max_input_bytes == 0
        || !context.model_operations.iter().any(|op| {
            op.service_id == operation.service_id
                && op.model_id == operation.model_id
                && op.operation == operation.operation
                && op.execution_authority_digest == operation.execution_authority_digest
                && op.max_input_tokens == operation.max_input_tokens
                && op.max_output_tokens == operation.max_output_tokens
                && op.method == operation.method
                && op.canonical_path == operation.canonical_path
                && op.protocol == operation.protocol
                && op.endpoint_generation == operation.endpoint_generation
                && op.max_input_bytes == operation.max_input_bytes
                && op.max_response_bytes == operation.max_response_bytes
        })
        || uuid::Uuid::parse_str(call_id).is_err()
    {
        return Err(denied());
    }
    route.require_platform_price()?;
    route = route.with_platform_metering(true).with_native_text_bound(
        i64::from(operation.max_input_tokens),
        i64::from(operation.max_output_tokens),
        i64::from(operation.max_response_bytes),
    );
    route.public_admission = Some(billing::route_context::PublicBillingAdmission {
        call_id: call_id.into(),
        model_id: operation.model_id.clone(),
        expires_at: context.expires_at,
    });
    if route.platform_specs().any(|(metric, _)| {
        matches!(
            metric,
            crate::models::service_billing::BillingMetric::Images
                | crate::models::service_billing::BillingMetric::VoiceSeconds
        )
    }) {
        return Err(denied());
    }
    let layers = billing::reservation::estimate_layer_reservations(
        &state.db,
        &route,
        state.config.billing_rate_cache_ttl_secs,
    )
    .await?;
    let maximum = layers
        .iter()
        .map(|layer| layer.reserved_credits)
        .try_fold(Credits::ZERO, Credits::checked_add)?;
    if maximum < Credits::ZERO {
        return Err(denied());
    }
    Ok(CallQuote {
        route,
        maximum,
        input: i64::from(operation.max_input_tokens),
        output: i64::from(operation.max_output_tokens),
    })
}

/// Caller transaction reserves the maximum for the complete loop in both UTC
/// buckets. Policy revisions, sponsor changes and manager epochs do not reset
/// the profile's daily spent amount. Caller-supplied wall time is never used.
pub(crate) async fn reserve_loop_in_session(
    db: &Database,
    session: &mut ClientSession,
    proof: &VerifiedPublicContract,
    context: &PublicExecutionContext,
    key_id: &str,
    maximum: Credits,
    max_calls: u32,
) -> AppResult<PublicBudgetLoop> {
    if proof.policy_digest() != context.policy_digest
        || maximum < Credits::ZERO
        || max_calls == 0
        || max_calls > context.limits.max_model_calls
    {
        return Err(denied());
    }
    let row = fence_context(db, session, context, key_id).await?;
    if let Some(existing) = db
        .collection::<PublicBudgetLoop>(LOOPS)
        .find_one(
            doc! {"public_session_id":&context.public_session_id,"turn_id":&context.turn.turn_id},
        )
        .session(&mut *session)
        .await?
    {
        if existing.maximum != maximum
            || existing.credential_api_key_id != key_id
            || existing.policy_digest != context.policy_digest
            || existing.receipt_id != context.ingress.ingress.receipt_id
            || existing.claim_id != context.ingress.ingress.claim_id
            || existing.closed
        {
            return Err(denied());
        }
        return Ok(existing);
    }
    let id = uuid::Uuid::new_v4().to_string();
    let controls = db
        .collection::<crate::models::platform_bot_runtime_controls::PlatformBotRuntimeControls>(
            crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
        )
        .find_one(doc! {"platform_config_id":&row.profile_id})
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    let policy = controls.public_policy.ok_or_else(denied)?;
    let loop_row = PublicBudgetLoop {
        id: id.clone(),
        public_session_id: row.id,
        conversation_id: context.turn.conversation_id.clone(),
        turn_id: context.turn.turn_id.clone(),
        receipt_id: context.ingress.ingress.receipt_id.clone(),
        claim_id: context.ingress.ingress.claim_id.clone(),
        profile_id: row.profile_id.clone(),
        sender_id: row.sender_id,
        sponsor_owner_user_id: row.sponsor_owner_user_id,
        credential_api_key_id: key_id.into(),
        policy_digest: context.policy_digest.clone(),
        bucket_ids: vec![],
        day: Utc::now(),
        expires_at: context.expires_at,
        maximum,
        held: maximum,
        allocated: Credits::ZERO,
        spent: Credits::ZERO,
        calls: 0,
        max_calls: i64::from(max_calls),
        input_tokens: 0,
        output_tokens: 0,
        max_input_tokens: i64::from(context.limits.max_input_tokens),
        max_output_tokens: i64::from(context.limits.max_output_tokens),
        closed: false,
    };
    let mut fields = bson::to_document(&loop_row).map_err(|_| denied())?;
    fields.remove("_id");
    fields.insert(
        "day",
        doc! {"$dateTrunc":{"date":"$$NOW","unit":"day","timezone":"UTC"}},
    );
    fields.insert("expires_at",doc! {"$min":[bson::DateTime::from_chrono(context.expires_at),{"$dateAdd":{"startDate":{"$dateTrunc":{"date":"$$NOW","unit":"day","timezone":"UTC"}},"unit":"day","amount":1}}]});
    let mut saved = db
        .collection::<PublicBudgetLoop>(LOOPS)
        .find_one_and_update(doc! {"_id":&id}, vec![doc! {"$set":fields}])
        .upsert(true)
        .return_document(ReturnDocument::After)
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    if saved.expires_at <= Utc::now() {
        return Err(denied());
    }
    for (scope, cap) in [
        (
            format!("total:{}:{}", row.profile_id, saved.day.timestamp()),
            policy.total_daily_credits,
        ),
        (
            format!(
                "sender:{}:{}:{}",
                row.profile_id,
                row.sender_id,
                saved.day.timestamp()
            ),
            policy.sender_daily_credits,
        ),
    ] {
        let bucket=db.collection::<PublicBudgetBucket>(BUCKETS).find_one_and_update(doc! {"scope_key":&scope},
            doc! {"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),"scope_key":&scope,"held":Credits::ZERO.decimal(),"spent":Credits::ZERO.decimal()}})
            .upsert(true).return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(denied)?.id;
        if db
            .collection::<PublicBudgetBucket>(BUCKETS)
            .update_one(
                doc! {"_id":&bucket,
                "$expr":{"$lte":[{"$add":["$held","$spent",maximum.decimal()]},cap.decimal()]}},
                doc! {"$inc":{"held":maximum.decimal()}},
            )
            .session(&mut *session)
            .await?
            .matched_count
            != 1
        {
            return Err(denied());
        }
        saved.bucket_ids.push(bucket);
    }
    db.collection::<PublicBudgetLoop>(LOOPS)
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"bucket_ids":&saved.bucket_ids}},
        )
        .session(&mut *session)
        .await?;
    Ok(saved)
}

pub(crate) async fn fence_context(
    db: &Database,
    session: &mut ClientSession,
    context: &PublicExecutionContext,
    key_id: &str,
) -> AppResult<PlatformPublicSession> {
    let ingress = &context.ingress;
    let manager = &ingress.manager;
    let row=db.collection::<PlatformPublicSession>(SESSIONS).find_one_and_update(doc! {
        "_id":&context.public_session_id,"credential_api_key_id":key_id,"sponsor_owner_user_id":&context.sponsor_owner_user_id,
        "policy_digest":&context.policy_digest,"nyxagent_profile":&context.nyxagent_profile,"revoked_at":bson::Bson::Null,
        "profile_id":&manager.profile_id,"manager_bot_id":&manager.manager_bot_id,"telegram_bot_id":manager.telegram_bot_id,
        "manager_authority_epoch":manager.manager_authority_epoch,"public_policy_revision":ingress.public_policy_revision,
        "sender_id":ingress.sender_id,"chat_id":ingress.chat_id,"expires_at":{"$gt":bson::DateTime::now()},
        "active_turn.turn_id":&context.turn.turn_id,"active_turn.conversation_id":&context.turn.conversation_id,
        "active_turn.receipt_id":&ingress.ingress.receipt_id,"active_turn.claim_id":&ingress.ingress.claim_id,
        "active_turn.stop_requested":false,"active_turn.expires_at":{"$gt":bson::DateTime::now()},
    },doc! {"$inc":{"effect_revision":1_i64}}).session(&mut *session).await?.ok_or_else(denied)?;
    if !super::platform_public_agent_service::fence_local_session_in_session(db, session, &row)
        .await?
    {
        return Err(denied());
    }
    for (collection, filter) in [
        (
            crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
            doc! {"platform_config_id":&manager.profile_id,
            "manager_bot_id":&manager.manager_bot_id,"telegram_bot_id":manager.telegram_bot_id,"manager_authority_epoch":manager.manager_authority_epoch,
            "public_policy_revision":ingress.public_policy_revision,"public_enabled":true,"emergency_stopped":false,"retired":false,
            "public_policy.sponsor_owner_user_id":&context.sponsor_owner_user_id,"public_policy.nyxagent_profile":&context.nyxagent_profile},
        ),
        (
            crate::models::user::COLLECTION_NAME,
            doc! {"_id":&context.sponsor_owner_user_id,"user_type":"org","is_active":true},
        ),
        (
            crate::models::api_key::COLLECTION_NAME,
            doc! {"_id":key_id,"purpose":"public_nyxbot","user_id":&context.sponsor_owner_user_id,"is_active":true},
        ),
        (
            crate::models::platform_manager_ingress::COLLECTION_NAME,
            doc! {"_id":&ingress.ingress.receipt_id,"claim_id":&ingress.ingress.claim_id,
            "phase":"executing","audience.kind":"public","execution.public_session_id":&context.public_session_id,
            "execution.turn_id":&context.turn.turn_id,"execution.conversation_id":&context.turn.conversation_id,
            "execution.payer_user_id":&context.sponsor_owner_user_id,"execution.public_policy_digest":&context.policy_digest},
        ),
    ] {
        if db
            .collection::<Document>(collection)
            .update_one(filter, doc! {"$inc":{"public_execution_fence":1_i64}})
            .session(&mut *session)
            .await?
            .matched_count
            != 1
        {
            return Err(denied());
        }
    }
    Ok(row)
}

pub(crate) async fn prepare_call_in_session(
    db: &Database,
    session: &mut ClientSession,
    context: &PublicExecutionContext,
    loop_id: &str,
    quote: &CallQuote,
) -> AppResult<PublicCallReservation> {
    let admission = quote.route.public_admission.as_ref().ok_or_else(denied)?;
    let digest = route_digest(&quote.route)?;
    if let Some(existing) = db
        .collection::<PublicCallReservation>(CALLS)
        .find_one(doc! {"_id":&admission.call_id})
        .session(&mut *session)
        .await?
    {
        if existing.loop_id != loop_id
            || existing.route_digest != digest
            || existing.maximum != quote.maximum
        {
            return Err(denied());
        }
        return Ok(existing);
    }
    let key = quote.route.api_key_id.as_deref().ok_or_else(denied)?;
    fence_context(db, session, context, key).await?;
    let loop_row=db.collection::<PublicBudgetLoop>(LOOPS).find_one_and_update(doc! {"_id":loop_id,"closed":false,
        "public_session_id":&context.public_session_id,"turn_id":&context.turn.turn_id,"credential_api_key_id":key,
        "expires_at":{"$gt":bson::DateTime::now()},"$expr":{"$and":[{"$lt":["$calls","$max_calls"]},
        {"$lte":[{"$add":["$allocated","$spent",quote.maximum.decimal()]},"$maximum"]},
        {"$lte":[{"$add":["$input_tokens",quote.input]},"$max_input_tokens"]},
        {"$lte":[{"$add":["$output_tokens",quote.output]},"$max_output_tokens"]}]}},
        doc! {"$inc":{"allocated":quote.maximum.decimal(),"calls":1_i64,"input_tokens":quote.input,"output_tokens":quote.output}})
        .return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(denied)?;
    let row = PublicCallReservation {
        id: admission.call_id.clone(),
        loop_id: loop_id.into(),
        sponsor_owner_user_id: loop_row.sponsor_owner_user_id,
        credential_api_key_id: key.into(),
        billing_request_id: quote.route.billing_request_id.clone(),
        route_digest: digest,
        model_id: admission.model_id.clone(),
        state: PublicCallState::Prepared,
        maximum: quote.maximum,
        cap_charge: Credits::ZERO,
        meter_row_ids: vec![],
        next_reconcile_at: Utc::now(),
        expires_at: loop_row.expires_at.min(admission.expires_at),
    };
    db.collection::<PublicCallReservation>(CALLS)
        .insert_one(&row)
        .session(&mut *session)
        .await?;
    Ok(row)
}

pub(crate) async fn fund_call(
    state: &AppState,
    quote: &CallQuote,
) -> AppResult<MeteredProxyContext> {
    state.billing.open_required(&quote.route).await
}

/// The final dispatch permission is spent once. An uncertain response never
/// turns the same public call ID into permission to retry provider work.
pub(crate) async fn admit_dispatch_in_session(
    db: &Database,
    session: &mut ClientSession,
    context: &PublicExecutionContext,
    call_id: &str,
) -> AppResult<()> {
    let row = db
        .collection::<PublicCallReservation>(CALLS)
        .find_one(doc! {"_id":call_id,"state":"funded"})
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    fence_context(db, session, context, &row.credential_api_key_id).await?;
    let loop_row=db.collection::<PublicBudgetLoop>(LOOPS).find_one_and_update(doc! {"_id":&row.loop_id,"closed":false,
        "public_session_id":&context.public_session_id,"turn_id":&context.turn.turn_id,"expires_at":{"$gt":bson::DateTime::now()}},
        doc! {"$inc":{"dispatch_fence":1_i64}}).session(&mut *session).await?.ok_or_else(denied)?;
    if row.meter_row_ids.is_empty() || loop_row.sponsor_owner_user_id != row.sponsor_owner_user_id {
        return Err(denied());
    }
    for id in &row.meter_row_ids {
        if db.collection::<UsageMeterRow>(METERS).update_one(doc! {"_id":id,"billing_request_id":&row.billing_request_id,
            "billing_owner_id":&row.sponsor_owner_user_id,"api_key_id":&row.credential_api_key_id,"status":"reserved","released":false,"forwarded":false},
            doc! {"$set":{"forwarded":true,"status":"forwarded"}}).session(&mut *session).await?.matched_count!=1 {return Err(denied());}
    }
    if db
        .collection::<PublicCallReservation>(CALLS)
        .update_one(
            doc! {"_id":call_id,"state":"funded","expires_at":{"$gt":bson::DateTime::now()}},
            doc! {"$set":{"state":"dispatch_admitted"}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(denied());
    }
    Ok(())
}

/// Existing meters are the sole actual-cost source. Allowance-covered units
/// still consume gross cap cost; funding precedence and wallet posting stay in billing.
async fn actual_cost(db: &Database, row: &PublicCallReservation) -> AppResult<Option<Credits>> {
    let rows: Vec<UsageMeterRow> = db
        .collection::<UsageMeterRow>(METERS)
        .find(doc! {"_id":{"$in":&row.meter_row_ids}})
        .await?
        .try_collect()
        .await?;
    if rows.len() != row.meter_row_ids.len() || rows.is_empty() {
        return Ok(None);
    }
    let mut total = Credits::ZERO;
    for meter in rows {
        if meter.billing_owner_id != row.sponsor_owner_user_id
            || meter.api_key_id.as_deref() != Some(row.credential_api_key_id.as_str())
            || meter.billing_request_id != row.billing_request_id
        {
            return Err(denied());
        }
        let Some(funding) = meter.funding.filter(|funding| funding.settled) else {
            return Ok(None);
        };
        let Some(cost) = funding.total_charge else {
            return Ok(None);
        };
        if cost < Credits::ZERO {
            return Err(denied());
        }
        total = total.checked_add(cost)?;
    }
    Ok(Some(total))
}

async fn post_cap(
    db: &Database,
    session: &mut ClientSession,
    row: &PublicCallReservation,
    cost: Credits,
    final_cost: bool,
) -> AppResult<()> {
    if cost < Credits::ZERO || cost > row.maximum {
        return Err(denied());
    }
    let loop_row = db
        .collection::<PublicBudgetLoop>(LOOPS)
        .find_one(doc! {"_id":&row.loop_id})
        .session(&mut *session)
        .await?
        .ok_or_else(denied)?;
    let previous = row.cap_charge;
    let delta = cost.checked_sub(previous)?;
    let first = row.state != PublicCallState::Conservative;
    let hold_delta = if first {
        cost.checked_neg()?
    } else {
        Credits::ZERO
    };
    for id in &loop_row.bucket_ids {
        if db
            .collection::<PublicBudgetBucket>(BUCKETS)
            .update_one(
                doc! {"_id":id},
                doc! {"$inc":{"held":hold_delta.decimal(),"spent":delta.decimal()}},
            )
            .session(&mut *session)
            .await?
            .matched_count
            != 1
        {
            return Err(denied());
        }
    }
    let mut increments = doc! {"held":hold_delta.decimal(),"spent":delta.decimal()};
    if first {
        increments.insert("allocated", row.maximum.checked_neg()?.decimal());
    }
    db.collection::<PublicBudgetLoop>(LOOPS)
        .update_one(doc! {"_id":&row.loop_id}, doc! {"$inc":increments})
        .session(&mut *session)
        .await?;
    if db
        .collection::<PublicCallReservation>(CALLS)
        .update_one(
            doc! {"_id":&row.id,"state":state_bson(row.state)?},
            doc! {"$set":{
            "state":if final_cost {"settled"} else {"conservative"},"cap_charge":cost.decimal()}},
        )
        .session(&mut *session)
        .await?
        .matched_count
        != 1
    {
        return Err(denied());
    }
    Ok(())
}

pub(crate) async fn reconcile_call(db: &Database, id: &str) -> AppResult<()> {
    reconcile_call_inner(db, id).await?;
    db.collection::<PublicCallReservation>(CALLS).update_one(doc! {"_id":id,"state":{"$nin":["settled","released"]}},
        vec![doc! {"$set":{"next_reconcile_at":{"$min":[{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":30}},
            {"$cond":[{"$gt":["$expires_at","$$NOW"]},"$expires_at",{"$dateAdd":{"startDate":"$$NOW","unit":"second","amount":30}}]}]}}}]).await?;
    Ok(())
}

async fn reconcile_call_inner(db: &Database, id: &str) -> AppResult<()> {
    let Some(row) = db
        .collection::<PublicCallReservation>(CALLS)
        .find_one(doc! {"_id":id})
        .await?
    else {
        return Ok(());
    };
    if matches!(
        row.state,
        PublicCallState::Settled | PublicCallState::Released
    ) {
        return Ok(());
    }
    if matches!(
        row.state,
        PublicCallState::Prepared | PublicCallState::Funded | PublicCallState::Cancelling
    ) {
        if row.expires_at > Utc::now() {
            return Ok(());
        }
        db.collection::<PublicCallReservation>(CALLS)
            .update_one(
                doc! {"_id":id,"state":{"$in":["prepared","funded"]}},
                doc! {"$set":{"state":"cancelling"}},
            )
            .await?;
        if db
            .collection::<PublicCallReservation>(CALLS)
            .count_documents(doc! {"_id":id,"state":"cancelling"})
            .limit(1)
            .await?
            == 0
        {
            return Ok(());
        }
        billing::reservation::release_unforwarded_rows(
            db,
            &row.billing_request_id,
            UsageStatus::Failed,
            Some("public admission expired before dispatch"),
        )
        .await?;
    }
    let actual = if matches!(
        row.state,
        PublicCallState::DispatchAdmitted | PublicCallState::Conservative
    ) {
        actual_cost(db, &row).await?
    } else {
        None
    };
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db.clone(),id.to_owned(),actual),|session,(db,id,actual)|Box::pin(async move {
        let result:AppResult<_>=async {
            let row=db.collection::<PublicCallReservation>(CALLS).find_one(doc! {"_id":&*id}).session(&mut *session).await?.ok_or_else(denied)?;
            match row.state {
                PublicCallState::Cancelling => {
                    db.collection::<PublicBudgetLoop>(LOOPS).update_one(doc! {"_id":&row.loop_id},doc! {"$inc":{"allocated":row.maximum.checked_neg()?.decimal()}}).session(&mut *session).await?;
                    db.collection::<PublicCallReservation>(CALLS).update_one(doc! {"_id":&row.id,"state":"cancelling"},doc! {"$set":{"state":"released"}}).session(&mut *session).await?;
                },
                PublicCallState::DispatchAdmitted|PublicCallState::Conservative => {
                    if let Some(actual)=*actual {post_cap(db,session,&row,actual,true).await?;}
                    else if row.state==PublicCallState::DispatchAdmitted && row.expires_at<=Utc::now() {post_cap(db,session,&row,row.maximum,false).await?;}
                },_=>{},
            } Ok(())
        }.await;tx::transaction_result(result)
    })).await.map_err(tx::map_transaction_error)
}

pub(crate) async fn close_loop_in_session(
    db: &Database,
    session: &mut ClientSession,
    id: &str,
) -> AppResult<bool> {
    let Some(row) = db
        .collection::<PublicBudgetLoop>(LOOPS)
        .find_one(doc! {"_id":id,"closed":false})
        .session(&mut *session)
        .await?
    else {
        return Ok(true);
    };
    if db.collection::<PublicCallReservation>(CALLS).count_documents(doc! {"loop_id":id,"state":{"$in":["prepared","funded","dispatch_admitted","cancelling"]}}).session(&mut *session).await?>0 {return Ok(false);}
    for bucket in &row.bucket_ids {
        db.collection::<PublicBudgetBucket>(BUCKETS)
            .update_one(
                doc! {"_id":bucket},
                doc! {"$inc":{"held":row.held.checked_neg()?.decimal()}},
            )
            .session(&mut *session)
            .await?;
    }
    db.collection::<PublicBudgetLoop>(LOOPS)
        .update_one(
            doc! {"_id":id,"closed":false},
            doc! {"$set":{"closed":true,"held":Credits::ZERO.decimal()}},
        )
        .session(&mut *session)
        .await?;
    Ok(true)
}

pub async fn sweep(db: &Database, limit: u32) -> AppResult<RecoveryBatch> {
    let limit = i64::from(limit.clamp(1, 100));
    let mut processed = 0;
    let calls: Vec<PublicCallReservation> = db
        .collection::<PublicCallReservation>(CALLS)
        .find(doc! {"state":{"$nin":["settled","released"]},"next_reconcile_at":{"$lte":bson::DateTime::now()}})
        .sort(doc! {"expires_at":1,"_id":1})
        .limit(limit)
        .await?
        .try_collect()
        .await?;
    for call in calls {
        reconcile_call(db, &call.id).await?;
        processed += 1;
    }
    let loops: Vec<PublicBudgetLoop> = db
        .collection::<PublicBudgetLoop>(LOOPS)
        .find(doc! {"closed":false,"expires_at":{"$lte":bson::DateTime::now()}})
        .sort(doc! {"expires_at":1,"_id":1})
        .limit(limit)
        .await?
        .try_collect()
        .await?;
    for row in loops {
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .and_run((db.clone(), row.id), |session, (db, id)| {
                Box::pin(async move {
                    tx::transaction_result(close_loop_in_session(db, session, id).await)
                })
            })
            .await
            .map_err(tx::map_transaction_error)?;
        processed += 1;
    }
    Ok(RecoveryBatch {
        processed,
        has_more: processed >= limit as u32,
    })
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    for (collection, keys) in [
        (LOOPS, doc! {"closed":1,"expires_at":1,"_id":1}),
        (CALLS, doc! {"next_reconcile_at":1,"expires_at":1,"_id":1}),
        (CALLS, doc! {"loop_id":1,"state":1}),
    ] {
        db.collection::<Document>(collection)
            .create_index(IndexModel::builder().keys(keys).build())
            .await?;
    }
    db.collection::<Document>(BUCKETS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"scope_key":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<Document>(LOOPS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"public_session_id":1,"turn_id":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    db.collection::<Document>(CALLS)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"billing_request_id":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::platform_bot_runtime_controls::{
        PlatformBotRuntimeControls, PublicRuntimePolicy,
    };
    use crate::models::{service_billing::BillingMetric, usage_meter::CredentialClass};
    use crate::services::billing::{BillingIngress, NodeIntent};
    use chrono::Duration;

    async fn context(
        db: &Database,
        sender: i64,
        sponsor: Option<&str>,
    ) -> (PublicExecutionContext, String) {
        let mut row =
            super::super::platform_public_agent_service::tests::fixture(db, sender, false).await;
        if let Some(sponsor) = sponsor {
            row.sponsor_owner_user_id = sponsor.into();
            db.collection::<PlatformPublicSession>(SESSIONS)
                .update_one(
                    doc! {"_id":&row.id},
                    doc! {"$set":{"sponsor_owner_user_id":sponsor}},
                )
                .await
                .unwrap();
            db.collection::<Document>(crate::models::api_key::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&row.credential_api_key_id},
                    doc! {"$set":{"user_id":sponsor}},
                )
                .await
                .unwrap();
        }
        db.collection::<Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&row.sponsor_owner_user_id},
                doc! {"$setOnInsert":{"user_type":"org","is_active":true}},
            )
            .upsert(true)
            .await
            .unwrap();
        let now = Utc::now();
        let controls = PlatformBotRuntimeControls {
            id: uuid::Uuid::new_v4().to_string(),
            contract_version: 1,
            platform_config_id: row.profile_id.clone(),
            manager_bot_id: row.manager_bot_id.clone(),
            telegram_bot_id: row.telegram_bot_id,
            manager_authority_epoch: row.manager_authority_epoch,
            personal_control_revision: 1,
            public_policy_revision: row.public_policy_revision,
            personal_enabled: false,
            public_enabled: true,
            emergency_stopped: false,
            retired: false,
            public_policy: Some(PublicRuntimePolicy {
                sponsor_owner_user_id: row.sponsor_owner_user_id.clone(),
                nyxagent_profile: row.nyxagent_profile.clone(),
                total_daily_credits: Credits::from_pico(10).unwrap(),
                sender_daily_credits: Credits::from_pico(10).unwrap(),
            }),
            effect_revision: 0,
            cleanup_pending: false,
            created_at: now,
            updated_at: now,
        };
        db.collection::<PlatformBotRuntimeControls>(
            crate::models::platform_bot_runtime_controls::COLLECTION_NAME,
        )
        .update_one(
            doc! {"platform_config_id":&row.profile_id},
            doc! {"$setOnInsert":bson::to_document(&controls).unwrap()},
        )
        .upsert(true)
        .await
        .unwrap();
        let turn = row.active_turn.as_ref().unwrap();
        let ctx = PublicExecutionContext {
            ingress: PublicIngress {
                ingress: IngressClaim {
                    receipt_id: turn.receipt_id.clone(),
                    claim_id: turn.claim_id.clone(),
                },
                manager: ManagerAuthority {
                    profile_id: row.profile_id.clone(),
                    manager_bot_id: row.manager_bot_id.clone(),
                    telegram_bot_id: row.telegram_bot_id,
                    manager_authority_epoch: row.manager_authority_epoch,
                },
                sender_id: sender,
                chat_id: sender,
                public_policy_revision: row.public_policy_revision,
            },
            public_session_id: row.id.clone(),
            turn: TurnRef {
                conversation_id: turn.conversation_id.clone(),
                turn_id: turn.turn_id.clone(),
            },
            sponsor_owner_user_id: row.sponsor_owner_user_id.clone(),
            policy_digest: row.policy_digest.clone(),
            nyxagent_profile: row.nyxagent_profile.clone(),
            model_operations: vec![],
            limits: PublicExecutionLimits {
                max_duration_secs: 60,
                max_model_calls: 2,
                max_tool_calls: 3,
                max_continuations: 0,
                max_input_tokens: 20,
                max_output_tokens: 10,
                max_input_bytes: 100,
                max_context_bytes: 100,
                max_output_bytes: 100,
            },
            expires_at: turn.expires_at,
        };
        db.collection::<Document>(crate::models::platform_manager_ingress::COLLECTION_NAME).insert_one(doc! {
            "_id":&turn.receipt_id,"claim_id":&turn.claim_id,"phase":"executing","audience":{"kind":"public"},
            "execution":{"public_session_id":&row.id,"turn_id":&turn.turn_id,"conversation_id":&turn.conversation_id,
                "payer_user_id":&row.sponsor_owner_user_id,"public_policy_digest":&row.policy_digest},
        }).await.unwrap();
        (ctx, row.credential_api_key_id)
    }
    async fn reserve(
        db: Database,
        ctx: PublicExecutionContext,
        key: String,
        maximum: Credits,
    ) -> AppResult<PublicBudgetLoop> {
        let mut session = db.client().start_session().await?;
        session
            .start_transaction()
            .and_run((db, ctx, key), |session, (db, ctx, key)| {
                Box::pin(async move {
                    tx::transaction_result(
                        reserve_loop_in_session(
                            db,
                            session,
                            &VerifiedPublicContract::for_local_tests(ctx.policy_digest.clone()),
                            ctx,
                            key,
                            maximum,
                            2,
                        )
                        .await,
                    )
                })
            })
            .await
            .map_err(tx::map_transaction_error)
    }
    fn quote(ctx: &PublicExecutionContext, key: &str, maximum: Credits) -> CallQuote {
        let mut route = BillingRouteContext::new(
            BillingIngress::Proxy,
            uuid::Uuid::new_v4().to_string(),
            ctx.sponsor_owner_user_id.clone(),
            ctx.sponsor_owner_user_id.clone(),
            Some(key.into()),
            None,
            Some("catalog".into()),
            Some("model".into()),
            NodeIntent::Direct,
            "bearer".into(),
            CredentialClass::NyxidManagedMaster,
            BillingMetric::Requests,
            None,
            false,
        )
        .with_platform_metering(true);
        route.public_admission = Some(billing::route_context::PublicBillingAdmission {
            call_id: uuid::Uuid::new_v4().to_string(),
            model_id: "pinned-model".into(),
            expires_at: ctx.expires_at,
        });
        CallQuote {
            route,
            maximum,
            input: 10,
            output: 5,
        }
    }
    #[tokio::test]
    async fn exact_total_cap_serializes_competing_senders_without_revision_reset() {
        let db =
            crate::test_utils::connect_transaction_test_database("public_exact_competition").await;
        ensure_indexes(&db).await.unwrap();
        let (a, key_a) = context(&db, 5, None).await;
        let (b, key_b) = context(&db, 6, Some(&a.sponsor_owner_user_id)).await;
        let (left, right) = tokio::join!(
            reserve(db.clone(), a, key_a, Credits::from_pico(6).unwrap()),
            reserve(db.clone(), b, key_b, Credits::from_pico(6).unwrap())
        );
        assert_ne!(left.is_ok(), right.is_ok());
        let buckets: Vec<PublicBudgetBucket> = db
            .collection::<PublicBudgetBucket>(BUCKETS)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(buckets.len(), 2);
        assert!(
            buckets
                .iter()
                .all(|b| b.held == Credits::from_pico(6).unwrap() && b.spent == Credits::ZERO)
        );
        assert_eq!(
            db.collection::<PublicBudgetLoop>(LOOPS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
    }
    #[tokio::test]
    async fn caller_abort_rolls_back_both_cap_scopes_and_loop_identity() {
        let db = crate::test_utils::connect_transaction_test_database("public_exact_abort").await;
        let (ctx, key) = context(&db, 5, None).await;
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        reserve_loop_in_session(
            &db,
            &mut session,
            &VerifiedPublicContract::for_local_tests(ctx.policy_digest.clone()),
            &ctx,
            &key,
            Credits::from_pico(5).unwrap(),
            2,
        )
        .await
        .unwrap();
        session.abort_transaction().await.unwrap();
        assert_eq!(
            db.collection::<Document>(BUCKETS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<Document>(LOOPS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    #[tokio::test]
    async fn dispatch_is_once_only_and_crash_cap_charge_refunds_only_known_exact_difference() {
        let db =
            crate::test_utils::connect_transaction_test_database("public_exact_dispatch").await;
        ensure_indexes(&db).await.unwrap();
        let (ctx, key) = context(&db, 5, None).await;
        let loop_row = reserve(
            db.clone(),
            ctx.clone(),
            key.clone(),
            Credits::from_pico(10).unwrap(),
        )
        .await
        .unwrap();
        let quote = quote(&ctx, &key, Credits::from_pico(6).unwrap());
        let mut session = db.client().start_session().await.unwrap();
        session.start_transaction().await.unwrap();
        let call = prepare_call_in_session(&db, &mut session, &ctx, &loop_row.id, &quote)
            .await
            .unwrap();
        assert_eq!(
            prepare_call_in_session(&db, &mut session, &ctx, &loop_row.id, &quote)
                .await
                .unwrap()
                .id,
            call.id
        );
        session.commit_transaction().await.unwrap();
        // This test isolates cap/dispatch recovery. Funding transaction atomicity
        // and real benefit holds are verified separately in billing::funding.
        billing::meter::open(&db, &quote.route, None).await.unwrap();
        let meters: Vec<UsageMeterRow> = db
            .collection::<UsageMeterRow>(METERS)
            .find(doc! {"billing_request_id":&call.billing_request_id})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        let ids: Vec<_> = meters.iter().map(|m| m.id.clone()).collect();
        db.collection::<PublicCallReservation>(CALLS)
            .update_one(
                doc! {"_id":&call.id},
                doc! {"$set":{"state":"funded","meter_row_ids":ids}},
            )
            .await
            .unwrap();
        session.start_transaction().await.unwrap();
        admit_dispatch_in_session(&db, &mut session, &ctx, &call.id)
            .await
            .unwrap();
        session.abort_transaction().await.unwrap();
        assert_eq!(
            db.collection::<UsageMeterRow>(METERS)
                .count_documents(doc! {"forwarded":true})
                .await
                .unwrap(),
            0
        );
        session.start_transaction().await.unwrap();
        admit_dispatch_in_session(&db, &mut session, &ctx, &call.id)
            .await
            .unwrap();
        session.commit_transaction().await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            admit_dispatch_in_session(&db, &mut session, &ctx, &call.id)
                .await
                .is_err()
        );
        session.abort_transaction().await.unwrap();
        db.collection::<PublicCallReservation>(CALLS).update_one(doc! {"_id":&call.id},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
        reconcile_call(&db, &call.id).await.unwrap();
        reconcile_call(&db, &call.id).await.unwrap();
        session.start_transaction().await.unwrap();
        assert!(
            close_loop_in_session(&db, &mut session, &loop_row.id)
                .await
                .unwrap()
        );
        session.commit_transaction().await.unwrap();
        let buckets: Vec<PublicBudgetBucket> = db
            .collection::<PublicBudgetBucket>(BUCKETS)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(
            buckets
                .iter()
                .all(|b| b.held == Credits::ZERO && b.spent == Credits::from_pico(6).unwrap())
        );
        db.collection::<UsageMeterRow>(METERS).update_many(doc! {"billing_request_id":&call.billing_request_id},doc! {"$set":{"funding.settled":true,"funding.total_charge":Credits::from_pico(3).unwrap().decimal()}}).await.unwrap();
        reconcile_call(&db, &call.id).await.unwrap();
        reconcile_call(&db, &call.id).await.unwrap();
        let buckets: Vec<PublicBudgetBucket> = db
            .collection::<PublicBudgetBucket>(BUCKETS)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert!(
            buckets
                .iter()
                .all(|b| b.held == Credits::ZERO && b.spent == Credits::from_pico(3).unwrap())
        );
    }
}
