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
pub(crate) const INDETERMINATE_COMMIT_MESSAGE: &str =
    "Transaction completion could not be confirmed";

pub(crate) fn is_indeterminate_commit(error: &AppError) -> bool {
    matches!(error, AppError::Internal(message) if message == INDETERMINATE_COMMIT_MESSAGE)
}

pub(crate) fn indeterminate_commit() -> AppError {
    // The same proof may already have been consumed. Do not promise that
    // repeating verification in a new transaction is safe or will succeed.
    AppError::Internal(INDETERMINATE_COMMIT_MESSAGE.into())
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
    #[test]
    fn only_the_expected_internal_message_is_indeterminate() {
        let error = indeterminate_commit();
        assert!(matches!(
            &error,
            AppError::Internal(message) if message == INDETERMINATE_COMMIT_MESSAGE
        ));
        assert!(is_indeterminate_commit(&error));
        assert!(!is_indeterminate_commit(&AppError::Internal(
            "Another internal failure".into(),
        )));
        assert!(!is_indeterminate_commit(&AppError::DatabaseError(
            mongodb::error::Error::custom("plain database failure"),
        )));
        let wrapped =
            super::super::api_key_mutation_service::abort_transaction(indeterminate_commit());
        let recovered = super::super::api_key_mutation_service::map_transaction_error(wrapped);
        assert!(matches!(&recovered, AppError::Internal(_)));
        assert!(is_indeterminate_commit(&recovered));
    }

    #[tokio::test]
    async fn indeterminate_commit_preserves_internal_wire_response_and_error_log() {
        use axum::{http::StatusCode, response::IntoResponse};
        use tracing_subscriber::layer::SubscriberExt;

        struct RecordedEvent {
            level: tracing::Level,
            error: String,
        }

        #[derive(Clone, Default)]
        struct Events(std::sync::Arc<std::sync::Mutex<Vec<RecordedEvent>>>);

        #[derive(Default)]
        struct ErrorField(String);

        impl tracing::field::Visit for ErrorField {
            fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
                if field.name() == "error" {
                    self.0 = format!("{value:?}");
                }
            }
        }

        impl<S: tracing::Subscriber> tracing_subscriber::Layer<S> for Events {
            fn on_event(
                &self,
                event: &tracing::Event<'_>,
                _context: tracing_subscriber::layer::Context<'_, S>,
            ) {
                let mut error = ErrorField::default();
                event.record(&mut error);
                self.0.lock().unwrap().push(RecordedEvent {
                    level: *event.metadata().level(),
                    error: error.0,
                });
            }
        }

        let events = Events::default();
        let subscriber = tracing_subscriber::registry().with(events.clone());
        let _guard = tracing::subscriber::set_default(subscriber);
        tracing::callsite::rebuild_interest_cache();
        let response = indeterminate_commit().into_response();
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["error"], "internal_error");
        assert!(!body.to_string().contains(INDETERMINATE_COMMIT_MESSAGE));
        let recorded = events.0.lock().unwrap();
        assert_eq!(recorded.len(), 1);
        assert_eq!(recorded[0].level, tracing::Level::ERROR);
        assert!(recorded[0].error.contains(INDETERMINATE_COMMIT_MESSAGE));
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
        assert!(is_indeterminate_commit(&error));
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
        assert!(is_indeterminate_commit(&error));
        assert!(!error.to_string().contains("retry"));
        let mut commit = ScriptedCommit::new([Err(CommitFailure::Definite(AppError::Conflict(
            "changed".into(),
        )))]);
        let error = retry_commit(&mut commit).await.unwrap_err();
        assert_eq!(commit.attempts, 1);
        assert!(matches!(error, AppError::Conflict(_)));
    }
}
