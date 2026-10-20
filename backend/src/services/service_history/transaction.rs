//! A session accepted by the journal can only be created by starting a transaction.
use mongodb::{ClientSession, Database, error::Result};

use crate::services::{audit_runtime::transaction as audit_transaction, audit_state_service};

#[derive(Debug)]
pub(super) struct ConcurrentHeadCreation;
pub(super) fn retryable(error: &mongodb::error::Error) -> bool {
    !error.contains_label("UnknownTransactionCommitResult")
        && (audit_transaction::retryable_database(error)
            || error.get_custom::<ConcurrentHeadCreation>().is_some())
}

pub struct Transaction {
    session: ClientSession,
}
impl std::ops::Deref for Transaction {
    type Target = ClientSession;
    fn deref(&self) -> &Self::Target {
        &self.session
    }
}
impl std::ops::DerefMut for Transaction {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.session
    }
}
impl<'a> From<&'a mut Transaction> for &'a mut ClientSession {
    fn from(value: &'a mut Transaction) -> Self {
        &mut value.session
    }
}

pub async fn run<R, F>(db: &Database, mut callback: F) -> Result<R>
where
    F: for<'a> AsyncFnMut(&'a mut Transaction) -> Result<R>,
{
    let mut transaction = Transaction {
        session: db.client().start_session().await?,
    };
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(120);
    for attempt in 1..=audit_transaction::MAX_BODY_ATTEMPTS {
        transaction
            .session
            .start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .write_concern(audit_state_service::majority())
            .await?;
        match callback(&mut transaction).await {
            Ok(value) => {
                for commit_attempt in 1..=audit_transaction::MAX_COMMIT_ATTEMPTS {
                    match transaction.session.commit_transaction().await {
                        Ok(()) => return Ok(value),
                        Err(error)
                            if error.contains_label("UnknownTransactionCommitResult")
                                && commit_attempt < audit_transaction::MAX_COMMIT_ATTEMPTS
                                && tokio::time::Instant::now() < deadline =>
                        {
                            audit_transaction::backoff(commit_attempt).await;
                            continue;
                        }
                        // Uncertainty dominates transient labeling; never abort or
                        // replay a possibly committed callback, including at budget.
                        Err(error) if error.contains_label("UnknownTransactionCommitResult") => {
                            return Err(error);
                        }
                        Err(error)
                            if retryable(&error)
                                && attempt < audit_transaction::MAX_BODY_ATTEMPTS
                                && tokio::time::Instant::now() < deadline =>
                        {
                            let _ = transaction.session.abort_transaction().await;
                            break;
                        }
                        Err(error) => return Err(error),
                    }
                }
            }
            Err(error) => {
                if error.contains_label("UnknownTransactionCommitResult") {
                    return Err(error);
                }
                let _ = transaction.session.abort_transaction().await;
                if !retryable(&error)
                    || attempt == audit_transaction::MAX_BODY_ATTEMPTS
                    || tokio::time::Instant::now() >= deadline
                {
                    return Err(error);
                }
            }
        }
        audit_transaction::backoff(attempt).await;
    }
    unreachable!("last bounded body attempt returns")
}
