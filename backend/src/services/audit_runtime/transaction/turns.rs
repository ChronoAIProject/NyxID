//! Local FIFO scheduling before snapshots of the single persistent audit head.
//! Database CAS, integrity validation, durability and cross-process retries
//! remain authoritative. Only transaction owners acquire a turn.
use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, LazyLock, Mutex, Weak},
};
use tokio::sync::Mutex as AsyncMutex;

type Queue = AsyncMutex<()>;
static TURNS: LazyLock<Mutex<HashMap<String, Weak<Queue>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

tokio::task_local! {
    static ACTIVE_OWNER: ();
}

/// A nested owner cannot commit atomically with the outer transaction. Fail
/// before queue/session acquisition; never hang behind its own owned turn.
#[derive(Debug)]
pub(super) struct NestedTransactionOwner;

pub(in crate::services) async fn with_turn<F: Future>(
    db: &mongodb::Database,
    body: F,
) -> mongodb::error::Result<F::Output> {
    with_named_turn(db.name(), body).await
}

async fn with_named_turn<F: Future>(name: &str, body: F) -> mongodb::error::Result<F::Output> {
    if ACTIVE_OWNER.try_with(|()| ()).is_ok() {
        return Err(mongodb::error::Error::custom(NestedTransactionOwner));
    }
    let turn = {
        let mut turns = TURNS.lock().unwrap_or_else(|p| p.into_inner());
        // Weak registration retains neither completed fixtures nor queues.
        turns.retain(|_, turn| turn.strong_count() != 0);
        match turns.get(name).and_then(Weak::upgrade) {
            Some(turn) => turn,
            None => {
                let turn = Arc::new(AsyncMutex::new(()));
                turns.insert(name.to_owned(), Arc::downgrade(&turn));
                turn
            }
        }
    };
    ACTIVE_OWNER
        .scope((), async move {
            // Tokio's queue is FIFO, releases on every return/unwind/cancellation,
            // and removes a cancelled waiter without losing the successor's wakeup.
            // The caller's transaction future is never polled before this turn.
            let _turn = turn.lock_owned().await;
            Ok(body.await)
        })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::poll;

    #[tokio::test]
    async fn audit_transaction_owner_queue_is_fifo_and_cancelled_waiter_leaves_no_slot() {
        let name = uuid::Uuid::new_v4().to_string();
        let (release, wait) = tokio::sync::oneshot::channel();
        let mut owner = Box::pin(with_named_turn(&name, async { wait.await.unwrap() }));
        assert!(poll!(&mut owner).is_pending());
        let order = Mutex::new(Vec::new());
        let mut first = Box::pin(with_named_turn(&name, async {
            order.lock().unwrap().push(1)
        }));
        assert!(poll!(&mut first).is_pending());
        let mut cancelled = Box::pin(with_named_turn(&name, async {
            panic!("cancelled waiter ran")
        }));
        assert!(poll!(&mut cancelled).is_pending());
        let mut last = Box::pin(with_named_turn(&name, async {
            order.lock().unwrap().push(2)
        }));
        assert!(poll!(&mut last).is_pending());
        drop(cancelled);
        release.send(()).unwrap();
        owner.await.unwrap();
        // Poll the last waiter first: it must not bypass the first waiter.
        assert!(poll!(&mut last).is_pending());
        first.await.unwrap();
        last.await.unwrap();
        assert_eq!(*order.lock().unwrap(), [1, 2]);
    }

    #[tokio::test]
    async fn audit_transaction_owner_queue_cancelled_owner_releases_successor() {
        let name = uuid::Uuid::new_v4().to_string();
        let mut owner = Box::pin(with_named_turn(&name, std::future::pending::<()>()));
        assert!(poll!(&mut owner).is_pending());
        let mut next = Box::pin(with_named_turn(&name, async { 7 }));
        assert!(poll!(&mut next).is_pending());
        drop(owner);
        assert_eq!(next.await.unwrap(), 7);
    }

    #[tokio::test]
    async fn audit_transaction_owner_queue_nested_task_owner_fails_before_body_or_wait() {
        let name = uuid::Uuid::new_v4().to_string();
        let result = with_named_turn(&name, async {
            with_named_turn(&name, async { panic!("nested owner body ran") }).await
        })
        .await
        .unwrap();
        assert!(
            result
                .unwrap_err()
                .get_custom::<NestedTransactionOwner>()
                .is_some()
        );
        assert_eq!(with_named_turn(&name, async { 9 }).await.unwrap(), 9);
    }
}
