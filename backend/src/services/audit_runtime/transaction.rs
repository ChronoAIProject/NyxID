//! Bounded transaction ownership for DB-only business mutations and their audit.
//! Stable logical/event identities belong to the context, outside the callback.
use futures::future::BoxFuture;
use mongodb::{ClientSession, Database, error::ErrorKind};
use rand::Rng;

use crate::errors::{AppError, AppResult};

mod turns;
pub(in crate::services) use turns::with_turn;

/// A finite attempt budget, including the first body execution. This is not a
/// change to Mongo/server/request timeouts. No integrity error is retryable.
pub const MAX_BODY_ATTEMPTS: u32 = 32;
pub const MAX_COMMIT_ATTEMPTS: u32 = 8;

/// Retry the entire DB-only callback on a transient transaction/WriteConflict.
/// Never put network effects, secret delivery or newly generated identities in
/// this callback. Unknown commits retry only commit on the SAME transaction;
/// exhaustion returns the database error without reexecuting the body.
pub async fn run<C, T, F>(db: &Database, context: &C, body: F) -> AppResult<T>
where
    C: Sync,
    T: Send,
    F: for<'a> FnMut(&'a mut ClientSession, &'a C) -> BoxFuture<'a, AppResult<T>> + Send,
{
    run_observed(db, context, body, |_, _| {}).await
}

/// Per-invocation attempt observation supports tests/benchmarks without global
/// counters or event material. Counts include first attempts and commit retries.
pub(crate) async fn run_observed<C, T, F, O>(
    db: &Database,
    context: &C,
    body: F,
    observe: O,
) -> AppResult<T>
where
    C: Sync,
    T: Send,
    F: for<'a> FnMut(&'a mut ClientSession, &'a C) -> BoxFuture<'a, AppResult<T>> + Send,
    O: FnMut(u32, u32) + Send,
{
    run_classified(db, context, body, observe, retryable).await
}

/// Existing owners can add a typed, definite-abort contention signal without
/// copying this loop. Unknown commit and all budgets remain enforced here.
pub(in crate::services) async fn run_classified<C, T, F, O, P>(
    db: &Database,
    context: &C,
    mut body: F,
    mut observe: O,
    retry_body: P,
) -> AppResult<T>
where
    C: Sync,
    T: Send,
    F: for<'a> FnMut(&'a mut ClientSession, &'a C) -> BoxFuture<'a, AppResult<T>> + Send,
    O: FnMut(u32, u32) + Send,
    P: Fn(&AppError) -> bool + Send,
{
    with_turn(db, async move {
        let mut session = db.client().start_session().await?;
        let mut commits = 0;
        for attempt in 1..=MAX_BODY_ATTEMPTS {
            observe(attempt, commits);
            session
                .start_transaction()
                .read_concern(mongodb::options::ReadConcern::snapshot())
                .write_concern(crate::services::audit_state_service::majority())
                .await?;
            let result = match body(&mut session, context).await {
                Ok(value) => commit_observed(&mut session, || {
                    commits += 1;
                    observe(attempt, commits);
                })
                .await
                .map(|()| value)
                .map_err(AppError::from),
                Err(error) => Err(error),
            };
            match result {
                Ok(value) => return Ok(value),
                // An uncertain commit must never cause callback replay or abort.
                Err(AppError::DatabaseError(error))
                    if error.contains_label("UnknownTransactionCommitResult") =>
                {
                    return Err(AppError::DatabaseError(error));
                }
                Err(error) => {
                    let _ = session.abort_transaction().await;
                    if attempt == MAX_BODY_ATTEMPTS
                        || !matches!(&error, AppError::DatabaseError(_))
                        || !retry_body(&error)
                    {
                        return Err(error);
                    }
                    backoff(attempt).await;
                }
            }
        }
        unreachable!("last bounded body attempt returns")
    })
    .await?
}

pub(super) fn retryable(error: &AppError) -> bool {
    let AppError::DatabaseError(error) = error else {
        return false;
    };
    retryable_database(error)
}

/// Existing typed transaction owners share the core contention classification.
pub(in crate::services) fn retryable_database(error: &mongodb::error::Error) -> bool {
    if error.contains_label("UnknownTransactionCommitResult") {
        return false;
    }
    error.contains_label("TransientTransactionError")
        || match error.kind.as_ref() {
            // Preserve unlabeled server WriteConflict as contention too.
            ErrorKind::Command(error) => error.code == 112,
            ErrorKind::Write(mongodb::error::WriteFailure::WriteError(error)) => error.code == 112,
            _ => false,
        }
}

pub(crate) async fn backoff(attempt: u32) {
    let ceiling = 5_u64 * (1_u64 << attempt.min(5));
    let delay = rand::thread_rng().gen_range(1..=ceiling);
    tokio::time::sleep(std::time::Duration::from_millis(delay)).await;
}

pub async fn commit(session: &mut ClientSession) -> mongodb::error::Result<()> {
    commit_observed(session, || {}).await
}

async fn commit_observed<O>(
    session: &mut ClientSession,
    mut observe: O,
) -> mongodb::error::Result<()>
where
    O: FnMut() + Send,
{
    for attempt in 1..=MAX_COMMIT_ATTEMPTS {
        observe();
        match session.commit_transaction().await {
            Err(error)
                if error.contains_label("UnknownTransactionCommitResult")
                    && attempt < MAX_COMMIT_ATTEMPTS =>
            {
                backoff(attempt).await;
            }
            result => return result,
        }
    }
    unreachable!("last bounded commit attempt returns")
}

#[cfg(test)]
mod tests;
