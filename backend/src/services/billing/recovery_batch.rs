//! Per-selected-unit bookkeeping, with no durable retry state of its own.
use crate::errors::{AppError, AppResult};
use mongodb::bson::{self, Document};
use serde::de::DeserializeOwned;

pub(crate) fn decode<T: DeserializeOwned>(raw: &Document) -> AppResult<T> {
    bson::from_document(raw.clone()).map_err(|e| mongodb::error::Error::from(e).into())
}

// Scope the error annotation to maintenance. No request error contract changes,
// no string classification, and a later unrelated failure stays a failure.
tokio::task_local! {static RECOVERY_UNIT: std::cell::RefCell<Option<AppError>>;}
pub(crate) async fn work<T>(
    future: impl std::future::Future<Output = AppResult<T>>,
) -> AppResult<T> {
    RECOVERY_UNIT
        .scope(std::cell::RefCell::new(None), async {
            let result = future.await;
            match RECOVERY_UNIT.with(|error| error.borrow_mut().take()) {
                Some(error) => Err(error),
                None => result,
            }
        })
        .await
}
pub(crate) fn waiting(source: AppError) -> AppError {
    if RECOVERY_UNIT.try_with(|_| ()).is_ok() {
        AppError::BillingRecoveryWaiting {
            source: Box::new(source),
        }
    } else {
        source
    }
}
/// Preserve a request caller's existing scheduled-retry result, while making
/// that same processing error visible to a maintenance unit after durable retry.
pub(crate) fn retry_outcome<T>(value: T, error: AppError) -> AppResult<T> {
    // Existing callers continue other charges after saving this charge's retry
    // state. Report the first failure when the whole selected unit finishes.
    let _ = RECOVERY_UNIT.try_with(|slot| {
        if !is_wait(&error) && slot.borrow().is_none() {
            *slot.borrow_mut() = Some(error);
        }
    });
    Ok(value)
}

pub(crate) fn is_wait(error: &AppError) -> bool {
    matches!(error, AppError::BillingRecoveryWaiting { .. })
}

#[derive(Default)]
pub(crate) struct Failures {
    count: u64,
    first: Option<AppError>,
}
impl Failures {
    pub(crate) fn note(&mut self, error: AppError) {
        if is_wait(&error) {
            return;
        }
        self.count += 1;
        if self.first.is_none() {
            self.first = Some(error);
        }
    }
    pub(crate) fn absorb<T>(&mut self, result: AppResult<T>) -> AppResult<Option<T>> {
        match result {
            Ok(value) => Ok(Some(value)),
            Err(AppError::BillingRecoveryFailed {
                failed_units,
                source,
            }) => {
                self.count += failed_units;
                if self.first.is_none() {
                    self.first = Some(*source);
                }
                Ok(None)
            }
            Err(error) => Err(error),
        }
    }
    pub(crate) async fn unit<T>(
        &mut self,
        work: impl std::future::Future<Output = AppResult<T>>,
    ) -> Option<T> {
        match self::work(work).await {
            Ok(value) => Some(value),
            Err(error) => {
                self.note(error);
                None
            }
        }
    }
    pub(crate) fn finish<T>(self, value: T) -> AppResult<T> {
        match self.first {
            Some(source) => Err(AppError::BillingRecoveryFailed {
                failed_units: self.count,
                source: Box::new(source),
            }),
            None => Ok(value),
        }
    }
}

// Small deterministic batch-boundary fixtures exercise the production loop,
// without timing a large collection or changing normal test/production limits.
#[cfg(test)]
tokio::task_local! {pub(crate) static TEST_BATCH_LIMIT: i64;}
pub(crate) fn limit(production: i64) -> i64 {
    #[cfg(test)]
    if let Ok(test) = TEST_BATCH_LIMIT.try_with(|n| *n) {
        return production.min(test);
    }
    production
}

pub(crate) fn document_rejected(error: &mongodb::error::Error) -> bool {
    use mongodb::error::{ErrorKind, WriteFailure};
    match error.kind.as_ref() {
        ErrorKind::Command(e) => e.code == 121,
        ErrorKind::Write(WriteFailure::WriteError(e)) => e.code == 121,
        _ => false,
    }
}
