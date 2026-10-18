//! Admission commits every financial hold and its recovery usage rows together.
//! A crash/abort cannot leave an unrecorded grant, allowance, or wallet hold.
use super::{meter, reservation, route_context::BillingRouteContext};
use crate::errors::{AppError, AppResult};

#[derive(Debug)]
struct TransactionAppError(std::sync::Mutex<Option<AppError>>);

pub(super) fn transaction_result<T>(result: AppResult<T>) -> mongodb::error::Result<T> {
    result.map_err(|error| match error {
        AppError::DatabaseError(error) => error,
        other => {
            mongodb::error::Error::custom(TransactionAppError(std::sync::Mutex::new(Some(other))))
        }
    })
}

pub(super) fn map_transaction_error(error: mongodb::error::Error) -> AppError {
    let custom = error
        .get_custom::<TransactionAppError>()
        .and_then(|v| v.0.lock().ok()?.take());
    custom.unwrap_or(AppError::DatabaseError(error))
}

pub async fn open(
    db: &mongodb::Database,
    lago: Option<&dyn super::lago_client::LagoApi>,
    ctx: &BillingRouteContext,
    fail_closed: bool,
    ttl: u64,
) -> AppResult<meter::MeteredProxyContext> {
    if !ctx.has_billable_layers() {
        return meter::open(db, ctx, None).await;
    }
    if meter::has_complete_meter(db, ctx).await? {
        return Ok(meter::MeteredProxyContext::from_route(ctx));
    }
    let prepared = match reservation::prepare_admission(db, lago, ctx, fail_closed, ttl).await {
        Ok(prepared) => prepared,
        Err(error) => {
            if meter::has_complete_meter(db, ctx).await? {
                return Ok(meter::MeteredProxyContext::from_route(ctx));
            }
            return Err(error);
        }
    };
    // Help durable operations before entering the snapshot. A conflicting new
    // operation aborts admission below and is helped on the next bounded retry.
    for _ in 0..4 {
        if meter::has_complete_meter(db, ctx).await? {
            return Ok(meter::MeteredProxyContext::from_route(ctx));
        }
        let mut session = db.client().start_session().await?;
        let tx_db = db.clone();
        let tx_ctx = ctx.clone();
        let tx_prepared = prepared.clone();
        let result = session
            .start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .write_concern(mongodb::options::WriteConcern::majority())
            .and_run2(async move |session| {
                let db = &tx_db;
                let ctx = &tx_ctx;
                let prepared = &tx_prepared;
                let operation = async {
                    let holds = reservation::gate_and_reserve_in_session(
                        db,
                        ctx,
                        prepared.as_ref(),
                        session,
                    )
                    .await?;
                    meter::open_in_session(db, ctx, holds.as_ref(), session).await
                }
                .await;
                transaction_result(operation)
            })
            .await
            .map_err(map_transaction_error);
        // A winning concurrent admission can exhaust the last credit before
        // this snapshot retries. Reuse its complete meter before interpreting
        // any funding error or suspending the account.
        if result.is_err() && meter::has_complete_meter(db, ctx).await? {
            return Ok(meter::MeteredProxyContext::from_route(ctx));
        }
        match result {
            Err(AppError::Conflict(ref message))
                if message.starts_with("billing grant settlement pending:") =>
            {
                super::funding::help_grant_settlement_lock(
                    db,
                    message.trim_start_matches("billing grant settlement pending:"),
                )
                .await?;
                continue;
            }
            Err(AppError::Conflict(ref message))
                if message.starts_with("billing allowance settlement pending:") =>
            {
                super::funding::help_allowance_settlement_lock(
                    db,
                    message.trim_start_matches("billing allowance settlement pending:"),
                )
                .await?;
                continue;
            }
            Err(AppError::Conflict(ref message))
                if message == "billing funding changed during reservation; retry" =>
            {
                continue;
            }
            Err(AppError::WalletSuspended) => {
                reservation::suspend_wallet(db, &ctx.billing_owner_id).await?;
                return Err(AppError::WalletSuspended);
            }
            other => return other,
        }
    }
    Err(AppError::Conflict(
        "Credits are being updated; retry this request".into(),
    ))
}
