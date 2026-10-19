//! Duration checkpoints reuse the normal reservations, funding split and ledger.
//! No monetary values or provider tariffs are defined here.
use super::{BillingService, meter::MeteredProxyContext, route_context::BillingRouteContext};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_voice::{FINALIZATION_HOURS, VoiceWindow, WINDOWS},
        service_billing::BillingMetric,
    },
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database,
    bson::{self, doc},
};

#[allow(dead_code)] // Phase 3 adapter API; no media is admitted in Phase 2.
pub const WINDOW_SECONDS: i64 = 30;

#[cfg_attr(not(test), allow(dead_code))]
pub fn window_id(session_id: &str, start_second: i64) -> AppResult<String> {
    if uuid::Uuid::parse_str(session_id).is_err()
        || !(0..1800).contains(&start_second)
        || start_second % WINDOW_SECONDS != 0
    {
        return Err(AppError::ValidationError(
            "Invalid voice billing window".into(),
        ));
    }
    Ok(format!("voice:{session_id}:{start_second}"))
}

/// Caller resolves live credential/owner ACL first; reserve before any provider spend.
/// Phase 3 supplies provider ownership and forced-close control around this primitive.
#[allow(dead_code)] // Phase 3 adapter API.
pub async fn reserve(
    db: &Database,
    billing: &BillingService,
    mut ctx: BillingRouteContext,
    session_id: &str,
    start_second: i64,
) -> AppResult<MeteredProxyContext> {
    let id = window_id(session_id, start_second)?;
    ctx.billing_request_id = id.clone();
    ctx.requested_voice_seconds = WINDOW_SECONDS;
    ctx.voice_initial_window = start_second == 0;
    if !ctx
        .platform_specs()
        .any(|(metric, _)| metric == BillingMetric::VoiceSeconds)
        || ctx.resale.is_some()
    {
        return Err(AppError::ValidationError(
            "Voice requires a duration lane without resale".into(),
        ));
    }
    use sha2::{Digest, Sha256};
    let identity = serde_json::to_vec(&(
        &ctx.actor_user_id,
        &ctx.billing_owner_id,
        &ctx.catalog_service_id,
        &ctx.user_service_id,
        &ctx.api_key_id,
        &ctx.credential_class,
        ctx.platform_specs().collect::<Vec<_>>(),
    ))
    .map_err(|_| AppError::Internal("Voice context serialization failed".into()))?;
    let context_identity = hex::encode(Sha256::digest(identity));
    let window = VoiceWindow {
        id: uuid::Uuid::new_v4().to_string(),
        billing_request_id: id.clone(),
        session_id: session_id.into(),
        user_id: ctx.actor_user_id.clone(),
        context_identity: context_identity.clone(),
        start_second,
        reserved_seconds: WINDOW_SECONDS,
        observed_seconds: 0,
        sealed: false,
        settled: false,
        uncertain: true,
        deadline: Utc::now() + Duration::hours(FINALIZATION_HOURS),
    };
    db.collection::<VoiceWindow>(WINDOWS).update_one(doc! {"billing_request_id":&id},
        doc! {"$setOnInsert":bson::to_document(&window).map_err(|_| AppError::Internal("Voice checkpoint serialization failed".into()))?})
        .upsert(true).await?;
    let saved = db
        .collection::<VoiceWindow>(WINDOWS)
        .find_one(doc! {"billing_request_id":&id})
        .await?
        .ok_or_else(|| AppError::Conflict("Voice checkpoint unavailable".into()))?;
    if saved.sealed || saved.context_identity != context_identity {
        return Err(AppError::Conflict(
            "Voice window finalized or context changed".into(),
        ));
    }
    let metered = billing.open(&ctx).await?;
    // A terminal checkpoint may race admission. Recheck before returning any
    // authority to spend; a crash here leaves only ordinary unforwarded holds.
    let current = db
        .collection::<VoiceWindow>(WINDOWS)
        .find_one(doc! {"billing_request_id":&id})
        .await?
        .ok_or_else(|| AppError::Conflict("Voice checkpoint unavailable".into()))?;
    if current.sealed {
        finalize(db, &id, current.uncertain).await?;
        return Err(AppError::Conflict("Voice window already finalized".into()));
    }
    Ok(metered)
}

/// Cumulative completed seconds across the entire call, including subsecond carry
/// in the adapter. Never round each window independently. Client clocks are forbidden.
#[cfg_attr(not(test), allow(dead_code))]
pub async fn observe(db: &Database, id: &str, cumulative_seconds: i64) -> AppResult<()> {
    let row = db
        .collection::<VoiceWindow>(WINDOWS)
        .find_one(doc! {"billing_request_id":id})
        .await?
        .ok_or_else(|| AppError::NotFound("Voice window not found".into()))?;
    if !(0..=1800).contains(&cumulative_seconds) {
        return Err(AppError::ValidationError("Invalid voice duration".into()));
    }
    let quantity = (cumulative_seconds - row.start_second).clamp(0, row.reserved_seconds);
    db.collection::<VoiceWindow>(WINDOWS)
        .update_one(
            doc! {"_id":&row.id,"sealed":false},
            doc! {"$max":{"observed_seconds":quantity}},
        )
        .await?;
    Ok(())
}

pub async fn finalize(db: &Database, id: &str, uncertain: bool) -> AppResult<()> {
    let rows = db.collection::<VoiceWindow>(WINDOWS);
    rows.update_one(
        doc! {"billing_request_id":id,"sealed":false},
        doc! {"$set":{"sealed":true,"uncertain":uncertain}},
    )
    .await?;
    let row = rows
        .find_one(doc! {"billing_request_id":id,"sealed":true})
        .await?
        .ok_or_else(|| AppError::NotFound("Voice window not found".into()))?;
    // An unknown tail is platform exposure; only observed units reach settlement.
    super::meter::settle_voice_window(db, id, row.observed_seconds, row.start_second == 0).await?;
    rows.update_one(
        doc! {"_id":&row.id,"sealed":true},
        doc! {"$set":{"settled":true}},
    )
    .await?;
    Ok(())
}

pub async fn reconcile(db: &Database) -> AppResult<u64> {
    let rows: Vec<VoiceWindow> = db.collection::<VoiceWindow>(WINDOWS).find(doc! {
        "settled":false,"$or":[{"sealed":true},{"deadline":{"$lte":bson::DateTime::from_chrono(Utc::now())}}]
    }).sort(doc! {"deadline":1}).limit(100).await?.try_collect().await?;
    let mut count = 0;
    for row in rows {
        finalize(db, &row.billing_request_id, true).await?;
        count += 1;
    }
    Ok(count)
}
