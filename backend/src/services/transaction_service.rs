//! Bounded same-transaction commit retry with truthful indeterminate outcomes.
use crate::errors::{AppError, AppResult};
/// Scoped lifecycle evidence handle. Readers never accept rollback-prone local
/// completion markers; all nontransactional effects are majority acknowledged.
/// Transactions must additionally set their own snapshot/majority concerns.
pub(crate) fn durable_database(db: &mongodb::Database) -> mongodb::Database {
    use mongodb::options::{
        DatabaseOptions, ReadConcern, ReadPreference, SelectionCriteria, WriteConcern,
    };
    db.client().database_with_options(
        db.name(),
        DatabaseOptions::builder()
            .read_concern(ReadConcern::majority())
            .write_concern(WriteConcern::majority())
            .selection_criteria(SelectionCriteria::ReadPreference(ReadPreference::Primary))
            .build(),
    )
}
/// Retry a bounded DB-only transaction after definite aborts. Provider I/O
/// must stay outside this boundary; unknown commits retry only COMMIT on the
/// same transaction and never replay the mutation or its action audit.
pub(crate) async fn run_db_only<C: Sync, R: Send + 'static, F>(
    db: &mongodb::Database,
    context: &C,
    mut operation: F,
) -> AppResult<R>
where
    F: Send
        + for<'a> FnMut(
            &'a mut mongodb::ClientSession,
            &'a C,
        ) -> futures::future::BoxFuture<'a, AppResult<R>>,
{
    super::service_history::transaction::run_send(db, context, move |tx, context| {
        let future = operation(tx, context);
        Box::pin(async move { super::api_key_mutation_service::transaction_result(future.await) })
    })
    .await
    .map_err(super::api_key_mutation_service::map_transaction_error)
}

pub(crate) fn map_error(error: mongodb::error::Error) -> AppError {
    if error.contains_label("UnknownTransactionCommitResult") {
        indeterminate_commit()
    } else if error.contains_label("TransientTransactionError") {
        AppError::Conflict("Security state changed; restart the operation".into())
    } else {
        AppError::DatabaseError(error)
    }
}
pub(crate) fn indeterminate_commit() -> AppError {
    // The same proof may already have been consumed. Do not promise that
    // repeating verification in a new transaction is safe or will succeed.
    AppError::Internal("Transaction completion could not be confirmed".into())
}
enum CommitFailure {
    Unknown,
    Definite(AppError),
}
// An explicit Send future boundary keeps transaction completion usable from
// Axum handlers as well as background workers. Only COMMIT is injectable;
// callers cannot accidentally replay the transaction's proof consumption.
#[async_trait::async_trait]
trait Committer: Send {
    async fn try_commit(&mut self) -> Result<(), CommitFailure>;
}
#[async_trait::async_trait]
impl Committer for mongodb::ClientSession {
    async fn try_commit(&mut self) -> Result<(), CommitFailure> {
        match self.commit_transaction().await {
            Ok(()) => Ok(()),
            Err(error) if error.contains_label("UnknownTransactionCommitResult") => {
                Err(CommitFailure::Unknown)
            }
            Err(error) => Err(CommitFailure::Definite(map_error(error))),
        }
    }
}
async fn retry_commit(commit: &mut impl Committer) -> AppResult<()> {
    let mut was_unknown = false;
    for _ in 0..3 {
        match tokio::time::timeout(std::time::Duration::from_secs(2), commit.try_commit()).await {
            Ok(Ok(())) => return Ok(()),
            Ok(Err(CommitFailure::Definite(error))) => {
                return Err(if was_unknown {
                    indeterminate_commit()
                } else {
                    error
                });
            }
            Ok(Err(CommitFailure::Unknown)) | Err(_) => {
                was_unknown = true;
            }
        }
    }
    Err(indeterminate_commit())
}
pub(crate) async fn commit(session: &mut mongodb::ClientSession) -> AppResult<()> {
    // Retry COMMIT on this exact session/transaction, never its mutations.
    // MongoDB resolves repeated commitTransaction idempotently.
    retry_commit(session).await
}

#[cfg(test)]
mod tests {
    use super::*;
    struct ScriptedCommit {
        attempts: usize,
        outcomes: std::collections::VecDeque<Result<(), CommitFailure>>,
    }
    impl ScriptedCommit {
        fn new(outcomes: impl IntoIterator<Item = Result<(), CommitFailure>>) -> Self {
            Self {
                attempts: 0,
                outcomes: outcomes.into_iter().collect(),
            }
        }
    }
    #[async_trait::async_trait]
    impl Committer for ScriptedCommit {
        async fn try_commit(&mut self) -> Result<(), CommitFailure> {
            self.attempts += 1;
            self.outcomes
                .pop_front()
                .expect("unexpected commit attempt")
        }
    }
    #[tokio::test]
    async fn unknown_commit_then_definite_error_remains_indeterminate() {
        let mut commit = ScriptedCommit::new([
            Err(CommitFailure::Unknown),
            Err(CommitFailure::Definite(AppError::Conflict(
                "transaction aborted".into(),
            ))),
        ]);
        let error = retry_commit(&mut commit).await.unwrap_err();
        assert_eq!(commit.attempts, 2);
        assert!(matches!(error, AppError::Internal(_)));
        assert!(!error.to_string().contains("retry"));
    }
    #[tokio::test]
    async fn unknown_commit_retries_only_commit_and_never_promises_proof_retry() {
        let mut commit = ScriptedCommit::new([
            Err(CommitFailure::Unknown),
            Err(CommitFailure::Unknown),
            Ok(()),
        ]);
        retry_commit(&mut commit).await.unwrap();
        assert_eq!(commit.attempts, 3);
        let mut commit = ScriptedCommit::new((0..3).map(|_| Err(CommitFailure::Unknown)));
        let error = retry_commit(&mut commit).await.unwrap_err();
        assert_eq!(commit.attempts, 3);
        assert!(matches!(error, AppError::Internal(_)));
        assert!(!error.to_string().contains("retry"));
        let mut commit = ScriptedCommit::new([Err(CommitFailure::Definite(AppError::Conflict(
            "changed".into(),
        )))]);
        let error = retry_commit(&mut commit).await.unwrap_err();
        assert_eq!(commit.attempts, 1);
        assert!(matches!(error, AppError::Conflict(_)));
    }
}
