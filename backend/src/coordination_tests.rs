use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::TryStreamExt;
use mongodb::IndexModel;

use crate::models::coordination::{
    CoordinationHolder, EVENT_DEDUP_COLLECTION_NAME, LEASE_COLLECTION_NAME,
    RATE_WINDOW_COLLECTION_NAME, REPLAY_COLLECTION_NAME, SLOT_COLLECTION_NAME,
};
use crate::services::coordination_service::{
    self, ClusterLeaseRuntime, EventDedupClaimResult, EventDedupStore, LeaseStore, RateWindowStore,
    ReplayStore, SlotStore,
};
use crate::test_utils::connect_test_database;

fn holder(instance: &str, generation: &str) -> CoordinationHolder {
    CoordinationHolder {
        instance_id: instance.to_string(),
        generation_id: generation.to_string(),
    }
}

#[tokio::test]
async fn coordination_collections_have_ttl_indexes() {
    let Some(db) = connect_test_database("coordination_indexes").await else {
        return;
    };
    coordination_service::ensure_indexes(&db)
        .await
        .expect("create coordination indexes");

    for collection_name in [
        LEASE_COLLECTION_NAME,
        REPLAY_COLLECTION_NAME,
        RATE_WINDOW_COLLECTION_NAME,
        SLOT_COLLECTION_NAME,
        EVENT_DEDUP_COLLECTION_NAME,
    ] {
        let indexes: Vec<IndexModel> = db
            .collection::<mongodb::bson::Document>(collection_name)
            .list_indexes()
            .await
            .expect("list coordination indexes")
            .try_collect()
            .await
            .expect("read coordination indexes");
        assert!(
            indexes.iter().any(|index| {
                index
                    .options
                    .as_ref()
                    .and_then(|options| options.expire_after)
                    == Some(Duration::from_secs(0))
            }),
            "{collection_name} must have an expires_at TTL index"
        );
    }
}

#[tokio::test]
async fn named_lease_is_exclusive_and_release_is_fenced() {
    let Some(db) = connect_test_database("coordination_lease").await else {
        return;
    };
    let first_holder = holder("pod-a", "generation-a");
    let second_holder = holder("pod-b", "generation-b");
    let ttl = Duration::from_secs(30);

    let first = LeaseStore::acquire(&db, "telegram-poller", &first_holder, ttl)
        .await
        .expect("acquire first lease")
        .expect("first holder wins");
    assert!(
        LeaseStore::acquire(&db, "telegram-poller", &second_holder, ttl)
            .await
            .expect("contending acquire")
            .is_none()
    );

    let mut stale = first.clone();
    stale.lease_id = uuid::Uuid::new_v4().to_string();
    assert!(
        !LeaseStore::renew(&db, &stale, ttl)
            .await
            .expect("stale renew")
    );
    assert!(
        !LeaseStore::release(&db, &stale)
            .await
            .expect("stale release")
    );
    assert!(
        LeaseStore::renew(&db, &first, ttl)
            .await
            .expect("owner renew")
    );
    assert!(
        LeaseStore::release(&db, &first)
            .await
            .expect("owner release")
    );
    assert!(
        LeaseStore::acquire(&db, "telegram-poller", &second_holder, ttl)
            .await
            .expect("acquire after release")
            .is_some()
    );
}

#[tokio::test]
async fn expired_named_lease_can_be_taken_over() {
    let Some(db) = connect_test_database("coordination_lease_expiry").await else {
        return;
    };
    let first = LeaseStore::acquire(
        &db,
        "oauth:key-1",
        &holder("pod-a", "generation-a"),
        Duration::from_millis(80),
    )
    .await
    .expect("acquire first")
    .expect("first lease");
    tokio::time::sleep(Duration::from_millis(120)).await;

    let replacement = LeaseStore::acquire(
        &db,
        "oauth:key-1",
        &holder("pod-b", "generation-b"),
        Duration::from_secs(30),
    )
    .await
    .expect("take over expired lease")
    .expect("replacement lease");
    assert_ne!(first.lease_id, replacement.lease_id);
    assert!(
        !LeaseStore::release(&db, &first)
            .await
            .expect("stale release")
    );
}

#[tokio::test]
async fn checkpoint_is_fenced_and_survives_lease_handoff() {
    let Some(db) = connect_test_database("coordination_lease_checkpoint").await else {
        eprintln!("skipping checkpoint lease test: no local MongoDB available");
        return;
    };
    let first_holder = holder("pod-a", "generation-a");
    let second_holder = holder("pod-b", "generation-b");
    let ttl = Duration::from_secs(30);
    let first = LeaseStore::acquire(&db, "telegram-poller", &first_holder, ttl)
        .await
        .expect("acquire first")
        .expect("first lease");

    assert!(
        LeaseStore::store_checkpoint(&db, &first, mongodb::bson::Bson::Int64(42))
            .await
            .expect("store checkpoint")
    );
    assert_eq!(
        LeaseStore::load_checkpoint(&db, &first)
            .await
            .expect("load checkpoint"),
        Some(mongodb::bson::Bson::Int64(42))
    );
    assert!(
        LeaseStore::release(&db, &first)
            .await
            .expect("release checkpoint lease")
    );
    assert!(
        !LeaseStore::store_checkpoint(&db, &first, mongodb::bson::Bson::Int64(43))
            .await
            .expect("stale checkpoint write")
    );

    let second = LeaseStore::acquire(&db, "telegram-poller", &second_holder, ttl)
        .await
        .expect("acquire replacement")
        .expect("replacement lease");
    assert_eq!(
        LeaseStore::load_checkpoint(&db, &second)
            .await
            .expect("replacement loads checkpoint"),
        Some(mongodb::bson::Bson::Int64(42))
    );
}

#[tokio::test]
async fn renewal_loss_cancels_fenced_operation() {
    struct DropSignal(Arc<AtomicBool>);
    impl Drop for DropSignal {
        fn drop(&mut self) {
            self.0.store(true, Ordering::SeqCst);
        }
    }

    let Some(db) = connect_test_database("coordination_lease_cancel").await else {
        eprintln!("skipping lease cancellation test: no local MongoDB available");
        return;
    };
    let runtime = ClusterLeaseRuntime::new(
        holder("pod-a", "generation-a"),
        Duration::from_millis(200),
        Duration::from_millis(20),
    );
    let lease = runtime
        .acquire(&db, "cancel-on-fence-loss")
        .await
        .expect("acquire lease")
        .expect("lease acquired");
    let release_token = lease.clone();
    let operation_db = db.clone();
    let dropped = Arc::new(AtomicBool::new(false));
    let operation_dropped = dropped.clone();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        runtime
            .run_while_renewed(&operation_db, &lease, async move {
                let _drop_signal = DropSignal(operation_dropped);
                let _ = started_tx.send(());
                futures::future::pending::<()>().await;
            })
            .await
    });

    started_rx.await.expect("operation started");
    assert!(
        LeaseStore::release(&db, &release_token)
            .await
            .expect("force lease loss")
    );
    let result = tokio::time::timeout(Duration::from_secs(1), task)
        .await
        .expect("renewal detected lease loss")
        .expect("operation task joined");
    assert!(result.is_none());
    assert!(dropped.load(Ordering::SeqCst));
}

#[tokio::test]
async fn replay_insert_is_first_writer_wins_across_callers() {
    let Some(db) = connect_test_database("coordination_replay").await else {
        return;
    };
    let db = Arc::new(db);
    let attempts = (0..12).map(|_| {
        let db = Arc::clone(&db);
        tokio::spawn(async move {
            ReplayStore::claim(&db, "dpop", "same-jti", Duration::from_secs(600))
                .await
                .expect("replay insert")
        })
    });
    let results = futures::future::join_all(attempts).await;
    assert_eq!(
        results
            .into_iter()
            .filter(|result| *result.as_ref().expect("task joined"))
            .count(),
        1
    );
}

#[tokio::test]
async fn fixed_window_counter_never_admits_above_the_global_limit() {
    let Some(db) = connect_test_database("coordination_rate").await else {
        return;
    };
    let db = Arc::new(db);
    let attempts = (0..24).map(|_| {
        let db = Arc::clone(&db);
        tokio::spawn(async move {
            RateWindowStore::admit(&db, "auth", "198.51.100.7", 5, Duration::from_secs(1))
                .await
                .expect("rate admission")
                .allowed
        })
    });
    let results = futures::future::join_all(attempts).await;
    assert_eq!(
        results
            .into_iter()
            .filter(|result| *result.as_ref().expect("task joined"))
            .count(),
        5
    );
    let separate = RateWindowStore::admit(&db, "auth", "203.0.113.9", 5, Duration::from_secs(1))
        .await
        .expect("separate rate admission");
    assert!(separate.allowed);
    assert_eq!(separate.remaining, 4);
    assert!(separate.reset_at > chrono::Utc::now());
}

#[tokio::test]
async fn global_slots_enforce_cap_and_fence_release() {
    let Some(db) = connect_test_database("coordination_slots").await else {
        return;
    };
    let owner = holder("pod-a", "generation-a");
    let ttl = Duration::from_secs(30);
    let first = SlotStore::acquire(&db, "ssh", "user-1", 2, &owner, ttl)
        .await
        .expect("first slot")
        .expect("slot available");
    let second = SlotStore::acquire(&db, "ssh", "user-1", 2, &owner, ttl)
        .await
        .expect("second slot")
        .expect("slot available");
    assert_ne!(first.slot, second.slot);
    assert_eq!(first.namespace, "ssh");
    assert!(!first.scope_hash.is_empty());
    assert!(
        SlotStore::renew(&db, &first, ttl)
            .await
            .expect("slot renew")
    );
    assert!(
        SlotStore::acquire(&db, "ssh", "user-1", 2, &owner, ttl)
            .await
            .expect("third slot")
            .is_none()
    );

    let mut stale = first.clone();
    stale.lease_id = uuid::Uuid::new_v4().to_string();
    assert!(
        !SlotStore::release(&db, &stale)
            .await
            .expect("stale release")
    );
    assert!(
        SlotStore::release(&db, &first)
            .await
            .expect("owner release")
    );
    assert!(
        SlotStore::acquire(&db, "ssh", "user-1", 2, &owner, ttl)
            .await
            .expect("replacement slot")
            .is_some()
    );
}

#[tokio::test]
async fn event_dedup_claim_commit_and_release_are_atomic_and_fenced() {
    let Some(db) = connect_test_database("coordination_event_dedup").await else {
        return;
    };
    let db = Arc::new(db);
    let claim_ttl = Duration::from_secs(30);
    let dedup_ttl = Duration::from_secs(300);
    let attempts = (0..12).map(|_| {
        let db = Arc::clone(&db);
        tokio::spawn(async move {
            EventDedupStore::claim(&db, "channel", "conversation-1", "event-1", claim_ttl)
                .await
                .expect("concurrent claim")
        })
    });
    let mut winners = futures::future::join_all(attempts)
        .await
        .into_iter()
        .filter_map(|result| match result.expect("claim task joined") {
            EventDedupClaimResult::Claimed(claim) => Some(claim),
            EventDedupClaimResult::Duplicate => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(winners.len(), 1);
    let first = winners.pop().expect("one claim winner");
    assert!(
        EventDedupStore::renew(&db, &first, claim_ttl)
            .await
            .expect("claim renew")
    );
    assert!(matches!(
        EventDedupStore::claim(&db, "channel", "conversation-1", "event-1", claim_ttl)
            .await
            .expect("concurrent claim"),
        EventDedupClaimResult::Duplicate
    ));

    let mut stale = first.clone();
    stale.claim_id = uuid::Uuid::new_v4().to_string();
    assert!(
        !EventDedupStore::release(&db, &stale)
            .await
            .expect("stale release")
    );
    assert!(
        EventDedupStore::release(&db, &first)
            .await
            .expect("owner release")
    );

    let retry = EventDedupStore::claim(&db, "channel", "conversation-1", "event-1", claim_ttl)
        .await
        .expect("retry claim");
    let retry = match retry {
        EventDedupClaimResult::Claimed(claim) => claim,
        EventDedupClaimResult::Duplicate => panic!("released claim must be retryable"),
    };
    assert!(
        EventDedupStore::commit(&db, &retry, dedup_ttl)
            .await
            .expect("commit")
    );
    assert!(matches!(
        EventDedupStore::claim(&db, "channel", "conversation-1", "event-1", claim_ttl)
            .await
            .expect("claim committed key"),
        EventDedupClaimResult::Duplicate
    ));
}
