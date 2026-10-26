//! A session accepted by the journal can only be created by starting a transaction.
use mongodb::{ClientSession, Database, error::Result};

#[derive(Debug)]
pub(super) struct ConcurrentHeadCreation;
pub(super) fn retryable(error: &mongodb::error::Error) -> bool {
    error.contains_label("TransientTransactionError")
        || error
            .get_custom::<crate::services::erasure_admission::OwnerFenceCreationRace>()
            .is_some()
        || error.get_custom::<ConcurrentHeadCreation>().is_some()
        || error
            .get_custom::<crate::services::audit_chain_service::AuditHeadCollision>()
            .is_some()
}

fn commit_error(error: mongodb::error::Error, was_unknown: bool) -> mongodb::error::Error {
    if was_unknown || error.contains_label("UnknownTransactionCommitResult") {
        crate::services::api_key_mutation_service::abort_transaction(
            crate::services::transaction_service::indeterminate_commit(),
        )
    } else {
        error
    }
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
    let deadline = transaction_deadline();
    loop {
        transaction
            .session
            .start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .write_concern(mongodb::options::WriteConcern::majority())
            .await?;
        match callback(&mut transaction).await {
            Ok(value) => {
                let mut commit_was_unknown = false;
                loop {
                    match commit_transaction(&mut transaction.session).await {
                        Ok(()) => return Ok(value),
                        Err(error)
                            if error.contains_label("UnknownTransactionCommitResult")
                                && tokio::time::Instant::now() < deadline =>
                        {
                            commit_was_unknown = true;
                            continue;
                        }
                        Err(error)
                            if !commit_was_unknown
                                && error.contains_label("TransientTransactionError")
                                && tokio::time::Instant::now() < deadline =>
                        {
                            break;
                        }
                        Err(error) => return Err(commit_error(error, commit_was_unknown)),
                    }
                }
            }
            Err(error) => {
                let _ = transaction.session.abort_transaction().await;
                if !retryable(&error) || tokio::time::Instant::now() >= deadline {
                    return Err(error);
                }
            }
        }
        tokio::task::yield_now().await;
    }
}

// Explicit Send callback boundary for credential copy transactions. Owning
// each retry snapshot avoids higher-ranked async-closure lifetime inference
// leaking into callers that spawn credential resolution.
pub(crate) async fn run_send<R, C: Sync, F>(
    db: &Database,
    context: &C,
    mut callback: F,
) -> Result<R>
where
    R: Send,
    F: Send
        + for<'a> FnMut(
            &'a mut Transaction,
            &'a C,
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<R>> + Send + 'a>>,
{
    let mut transaction = Transaction {
        session: db.client().start_session().await?,
    };
    let deadline = transaction_deadline();
    loop {
        transaction
            .session
            .start_transaction()
            .read_concern(mongodb::options::ReadConcern::snapshot())
            .write_concern(mongodb::options::WriteConcern::majority())
            .await?;
        match callback(&mut transaction, context).await {
            Ok(value) => {
                let mut commit_was_unknown = false;
                loop {
                    match commit_transaction(&mut transaction.session).await {
                        Ok(()) => return Ok(value),
                        Err(error)
                            if error.contains_label("UnknownTransactionCommitResult")
                                && tokio::time::Instant::now() < deadline =>
                        {
                            commit_was_unknown = true;
                            continue;
                        }
                        Err(error)
                            if !commit_was_unknown
                                && error.contains_label("TransientTransactionError")
                                && tokio::time::Instant::now() < deadline =>
                        {
                            break;
                        }
                        Err(error) => return Err(commit_error(error, commit_was_unknown)),
                    }
                }
            }
            Err(error) => {
                let _ = transaction.session.abort_transaction().await;
                if !retryable(&error) || tokio::time::Instant::now() >= deadline {
                    return Err(error);
                }
            }
        }
        tokio::task::yield_now().await;
    }
}

fn transaction_deadline() -> tokio::time::Instant {
    #[cfg(test)]
    if let Ok(budget) = COMMIT_RESPONSE_LOSS.try_with(|fault| fault.borrow().budget) {
        return tokio::time::Instant::now() + budget;
    }
    tokio::time::Instant::now() + std::time::Duration::from_secs(120)
}

async fn commit_transaction(session: &mut ClientSession) -> Result<()> {
    session.commit_transaction().await?;
    #[cfg(test)]
    if COMMIT_RESPONSE_LOSS
        .try_with(|fault| {
            let mut fault = fault.borrow_mut();
            fault.attempts += 1;
            if fault.remaining == 0 {
                false
            } else {
                fault.remaining -= 1;
                true
            }
        })
        .unwrap_or(false)
    {
        // The database committed; model losing only its acknowledgement.
        let concern: mongodb::error::WriteConcernError = bson::from_document(bson::doc! {
            "code":64,"errmsg":"injected lost commit acknowledgement",
            "errorLabels":["UnknownTransactionCommitResult"]
        })
        .unwrap();
        return Err(mongodb::error::ErrorKind::Write(
            mongodb::error::WriteFailure::WriteConcernError(concern),
        )
        .into());
    }
    Ok(())
}

#[cfg(test)]
pub(crate) struct CommitResponseLoss {
    pub remaining: usize,
    pub attempts: usize,
    pub budget: std::time::Duration,
}
#[cfg(test)]
tokio::task_local! {
    pub(crate) static COMMIT_RESPONSE_LOSS: std::cell::RefCell<CommitResponseLoss>;
}
