use super::{
    inbox::{self, InboxError},
    inbox_authority::{self, Authority, READ_SCOPE, Registration, WRITE_SCOPE},
};
use bson::{DateTime, Document, doc};
use mongodb::Database;
use uuid::Uuid;

const NAMESPACE: &str = "cma-example";
const ISSUER: &str = "nyxid-example";
const OPEN: &[u8] =
    include_bytes!("../../../../tests/fixtures/cma_usage_v1/stream-open.canonical.json");
const USAGE: &[u8] =
    include_bytes!("../../../../tests/fixtures/cma_usage_v1/allocated.canonical.json");
const CORRECTION: &[u8] =
    include_bytes!("../../../../tests/fixtures/cma_usage_v1/correction.canonical.json");
const SEAL: &[u8] =
    include_bytes!("../../../../tests/fixtures/cma_usage_v1/stream-seal.canonical.json");

struct Fixture {
    db: Database,
    account: String,
    jti: String,
    scopes: String,
}

impl Fixture {
    async fn open(name: &str, namespace: &str) -> Self {
        let db = crate::test_utils::connect_transaction_test_database(name).await;
        Self::on(db, namespace).await
    }

    async fn on(db: Database, namespace: &str) -> Self {
        inbox::prepare(&db).await.unwrap();
        let account = Uuid::new_v4().to_string();
        let scopes = format!("{READ_SCOPE} {WRITE_SCOPE}");
        let now = DateTime::now();
        db.collection::<Document>("service_accounts")
            .insert_one(doc! {
                "_id":&account, "name":"CMA custody test",
                "client_id":format!("sa_{}", Uuid::new_v4().simple()),
                "client_secret_hash":"test-only", "secret_prefix":"test",
                "allowed_scopes":&scopes, "is_active":true, "credential_generation":0_i64,
                "purpose":"general", "created_by":"fixture", "created_at":now, "updated_at":now,
            })
            .await
            .unwrap();
        let jti = issue_token(&db, &account, &scopes, 0).await;
        let revision = inbox_authority::register(
            &db,
            Registration {
                account_id: &account,
                namespace,
                owner_issuer: ISSUER,
                expected_revision: 0,
                enabled: true,
                accepting: true,
            },
        )
        .await
        .unwrap();
        assert_eq!(revision, 1);
        Self {
            db,
            account,
            jti,
            scopes,
        }
    }

    async fn authority(&self, scope: &'static str) -> Authority {
        Authority::load(&self.db, &self.account, &self.jti, &self.scopes, scope)
            .await
            .unwrap()
    }
}

async fn issue_token(db: &Database, account: &str, scope: &str, generation: i64) -> String {
    let jti = Uuid::new_v4().to_string();
    let now = DateTime::now();
    db.collection::<Document>("service_account_tokens")
        .insert_one(doc! {
            "_id":Uuid::new_v4().to_string(), "jti":&jti,"service_account_id":account,
            "scope":scope, "revoked":false, "credential_generation":generation,
            "created_at":now, "expires_at":DateTime::from_millis(now.timestamp_millis() + 60_000),
        })
        .await
        .unwrap();
    jti
}

#[tokio::test]
async fn replay_retains_one_receipt_after_concurrent_submission_and_sealing() {
    let fx = Fixture::open("cma_custody_replay", NAMESPACE).await;
    let writer = fx.authority(WRITE_SCOPE).await;
    let reader = fx.authority(READ_SCOPE).await;
    assert!(matches!(
        inbox::accept(&fx.db, &writer, USAGE).await,
        Err(InboxError::Prerequisite("stream_not_open"))
    ));
    inbox::accept(&fx.db, &writer, OPEN).await.unwrap();
    assert!(matches!(
        inbox::accept(&fx.db, &writer, CORRECTION).await,
        Err(InboxError::Prerequisite("predecessor_missing"))
    ));
    let (left, right) = tokio::join!(
        inbox::accept(&fx.db, &writer, USAGE),
        inbox::accept(&fx.db, &writer, USAGE)
    );
    let (first, receipt) = left.unwrap();
    let (second, concurrent) = right.unwrap();
    assert_ne!(first, second);
    assert_eq!(receipt, concurrent);
    inbox::accept(&fx.db, &writer, CORRECTION).await.unwrap();
    inbox::accept(&fx.db, &writer, SEAL).await.unwrap();
    let (created, replay) = inbox::accept(&fx.db, &writer, USAGE).await.unwrap();
    assert!(!created);
    assert_eq!(receipt, replay);
    assert_eq!(
        Some(receipt.clone()),
        inbox::receipt(&fx.db, &reader, receipt.event_id.as_str())
            .await
            .unwrap()
    );
    assert_eq!(
        inbox::cursor(&fx.db, &reader, receipt.stream_id.as_str())
            .await
            .unwrap()
            .unwrap()
            .0,
        4
    );
    assert_eq!(
        fx.db
            .collection::<Document>("sandbox_usage_events_v1")
            .count_documents(doc! {})
            .await
            .unwrap(),
        4
    );
    assert_eq!(
        fx.db
            .collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn replaced_token_can_read_retained_custody_after_new_intake_closes() {
    let mut fx = Fixture::open("cma_custody_rotation", NAMESPACE).await;
    let stale = fx.authority(WRITE_SCOPE).await;
    let (_, receipt) = inbox::accept(&fx.db, &stale, OPEN).await.unwrap();
    inbox_authority::register(
        &fx.db,
        Registration {
            account_id: &fx.account,
            namespace: NAMESPACE,
            owner_issuer: ISSUER,
            expected_revision: 1,
            enabled: true,
            accepting: false,
        },
    )
    .await
    .unwrap();
    fx.db
        .collection::<Document>("service_accounts")
        .update_one(
            doc! {"_id":&fx.account},
            doc! {"$inc":{"credential_generation":1_i64}},
        )
        .await
        .unwrap();
    fx.jti = issue_token(&fx.db, &fx.account, &fx.scopes, 1).await;
    assert!(matches!(
        inbox::accept(&fx.db, &stale, USAGE).await,
        Err(InboxError::Forbidden)
    ));
    let read = fx.authority(READ_SCOPE).await;
    assert_eq!(
        inbox::receipt(&fx.db, &read, receipt.event_id.as_str())
            .await
            .unwrap(),
        Some(receipt.clone())
    );
    let write = fx.authority(WRITE_SCOPE).await;
    let (created, replay) = inbox::accept(&fx.db, &write, OPEN).await.unwrap();
    assert!(!created);
    assert_eq!(receipt, replay);
    assert!(matches!(
        inbox::accept(&fx.db, &write, USAGE).await,
        Err(InboxError::Unavailable)
    ));
    let foreign = Fixture::on(fx.db.clone(), "foreign-fixture").await;
    let other = foreign.authority(READ_SCOPE).await;
    assert_eq!(
        inbox::receipt(&fx.db, &other, receipt.event_id.as_str())
            .await
            .unwrap(),
        None
    );
    assert_eq!(
        inbox::cursor(&fx.db, &other, receipt.stream_id.as_str())
            .await
            .unwrap(),
        None
    );
}
