//! Real HTTP sender used by the MongoDB dispatch-window regressions.
use super::{DispatchWindow, LagoApi, LagoClient, LagoWallet, ProviderDispatch};
use crate::models::credits::Credits;
use mongodb::bson::{Document, doc};
use std::{
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

pub struct DispatchHttp {
    db: mongodb::Database,
    client: LagoClient,
    monotonic_age: Duration,
    wall_age: Duration,
    posts: Arc<AtomicUsize>,
    task: tokio::task::JoinHandle<()>,
}

impl DispatchHttp {
    pub async fn new(db: &mongodb::Database, monotonic_secs: u64, wall_secs: u64) -> Self {
        let posts = Arc::new(AtomicUsize::new(0));
        let creates = posts.clone();
        let voids = posts.clone();
        let app = axum::Router::new()
            .route(
                "/api/v1/wallets",
                axum::routing::post(move || {
                    creates.fetch_add(1, Ordering::SeqCst);
                    async {
                        axum::Json(serde_json::json!({
                            "wallet": {"lago_id": "provider-wallet", "credits_balance": "7"}
                        }))
                    }
                }),
            )
            .route(
                "/api/v1/wallet_transactions",
                axum::routing::post(move || {
                    voids.fetch_add(1, Ordering::SeqCst);
                    async {
                        axum::Json(serde_json::json!({
                            "wallet_transactions": [{"lago_id": "void-1"}]
                        }))
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let task = crate::services::shutdown::Runtime::default().spawn_worker(async move {
            axum::serve(listener, app).await.unwrap();
        });
        Self {
            db: db.clone(),
            client: LagoClient::new(format!("http://{address}"), "test-key".into()).unwrap(),
            monotonic_age: Duration::from_secs(monotonic_secs),
            wall_age: Duration::from_secs(wall_secs),
            posts,
            task,
        }
    }

    pub fn posts(&self) -> usize {
        self.posts.load(Ordering::SeqCst)
    }

    pub async fn create(
        &self,
        customer: &str,
        window: &DispatchWindow,
    ) -> ProviderDispatch<LagoWallet> {
        let saved = self
            .db
            .collection::<Document>("coordination_leases")
            .find_one(doc! {"_id": format!("billing:provision:{customer}")})
            .await
            .unwrap()
            .unwrap();
        assert!(
            saved
                .get_document("checkpoint")
                .unwrap()
                .get_datetime("dispatch_started_at")
                .is_ok()
        );
        let aged = window.aged_for_test(self.monotonic_age, self.wall_age);
        self.client.dispatch_wallet_create(customer, &aged).await
    }

    pub async fn void(
        &self,
        wallet: &str,
        amount: Credits,
        operation: &str,
        window: &DispatchWindow,
    ) -> ProviderDispatch<String> {
        let saved = self
            .db
            .collection::<Document>("billing_wallet")
            .find_one(doc! {"active_topup_expiry.operation_id": operation})
            .await
            .unwrap()
            .unwrap();
        assert!(
            saved
                .get_document("active_topup_expiry")
                .unwrap()
                .get_datetime("dispatch_started_at")
                .is_ok()
        );
        let aged = window.aged_for_test(self.monotonic_age, self.wall_age);
        self.client
            .dispatch_wallet_void(wallet, amount, operation, &aged)
            .await
    }
}

impl Drop for DispatchHttp {
    fn drop(&mut self) {
        self.task.abort();
    }
}
