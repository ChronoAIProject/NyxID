//! A session accepted by the journal can only be created by starting a transaction.
use mongodb::{ClientSession, Database, error::Result};

#[derive(Debug)]
pub(super) struct ConcurrentHeadCreation;
pub(super) fn retryable(error: &mongodb::error::Error) -> bool {
    error.contains_label("TransientTransactionError")
        || error.get_custom::<ConcurrentHeadCreation>().is_some()
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
    loop {
        transaction.session.start_transaction().await?;
        match callback(&mut transaction).await {
            Ok(value) => loop {
                match transaction.session.commit_transaction().await {
                    Ok(()) => return Ok(value),
                    Err(error)
                        if error.contains_label("UnknownTransactionCommitResult")
                            && tokio::time::Instant::now() < deadline =>
                    {
                        continue;
                    }
                    Err(error)
                        if error.contains_label("TransientTransactionError")
                            && tokio::time::Instant::now() < deadline =>
                    {
                        break;
                    }
                    Err(error) => return Err(error),
                }
            },
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
