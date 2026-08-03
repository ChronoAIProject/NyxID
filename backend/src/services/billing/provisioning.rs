use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use sha2::Digest;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::billing_topup_session::{
    BillingTopUpSession, BillingTopUpStatus, COLLECTION_NAME as BILLING_TOPUP_SESSIONS,
};
use crate::models::billing_wallet::{
    BillingWallet, COLLECTION_NAME as BILLING_WALLET, CollectionState, PlanKind,
};
use crate::models::user::{COLLECTION_NAME as USERS, User};

use super::lago_client::{LagoApi, OwnerProvisionInput, WalletTopUpInput};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvisionedWallet {
    pub wallet: BillingWallet,
    pub created: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TopUpCheckout {
    pub session: BillingTopUpSession,
    pub reused: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BillingBackfillStats {
    pub scanned: u64,
    pub provisioned: u64,
    pub existing: u64,
    pub failed: u64,
}

pub async fn get_wallet(
    db: &mongodb::Database,
    owner_id: &str,
) -> AppResult<Option<BillingWallet>> {
    db.collection::<BillingWallet>(BILLING_WALLET)
        .find_one(doc! { "owner_id": owner_id })
        .await
        .map_err(Into::into)
}

pub async fn ensure_owner_wallet(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    owner_id: &str,
    plan_code: &str,
    default_overdraft_cap_credits: i64,
) -> AppResult<ProvisionedWallet> {
    if let Some(wallet) = get_wallet(db, owner_id).await? {
        return Ok(ProvisionedWallet {
            wallet,
            created: false,
        });
    }

    let owner = owner_provision_input(db, owner_id).await?;
    let customer_id = lago.ensure_customer(&owner).await?;
    let subscription_id = lago.ensure_subscription(&customer_id, plan_code).await?;
    let lago_wallet = lago.ensure_wallet(&customer_id).await?;
    let now = Utc::now();
    let wallet = BillingWallet {
        id: Uuid::new_v4().to_string(),
        owner_id: owner_id.to_string(),
        lago_customer_id: customer_id,
        lago_wallet_id: Some(lago_wallet.id),
        lago_subscription_id: Some(subscription_id),
        plan_kind: PlanKind::Prepaid,
        balance_credits: lago_wallet.balance_credits,
        reserved_credits: 0,
        pending_lago_debits: 0,
        has_payment_instrument: false,
        overdraft_cap_credits: default_overdraft_cap_credits,
        suspended: false,
        collection_state: CollectionState::Good,
        balance_synced_at: now,
        created_at: now,
        updated_at: now,
    };

    match db
        .collection::<BillingWallet>(BILLING_WALLET)
        .insert_one(&wallet)
        .await
    {
        Ok(_) => Ok(ProvisionedWallet {
            wallet,
            created: true,
        }),
        Err(error) if is_duplicate_key_error(&error) => {
            let existing = get_wallet(db, owner_id).await?.ok_or_else(|| {
                AppError::Internal(
                    "billing wallet insert raced but existing wallet was not found".to_string(),
                )
            })?;
            Ok(ProvisionedWallet {
                wallet: existing,
                created: false,
            })
        }
        Err(error) => Err(error.into()),
    }
}

pub async fn create_topup_checkout(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    owner_id: &str,
    plan_code: &str,
    default_overdraft_cap_credits: i64,
    amount_credits: i64,
    idempotency_key: &str,
) -> AppResult<TopUpCheckout> {
    if amount_credits <= 0 {
        return Err(AppError::BadRequest(
            "amount_credits must be greater than 0".to_string(),
        ));
    }
    if amount_credits > 10_000_000 {
        return Err(AppError::BadRequest(
            "amount_credits must be at most 10000000".to_string(),
        ));
    }
    let idempotency_key = normalize_idempotency_key(idempotency_key)?;
    if let Some(existing) = find_topup_session(db, owner_id, &idempotency_key).await? {
        if existing.amount_credits != amount_credits {
            return Err(AppError::Conflict(
                "idempotency_key was already used with a different amount_credits".to_string(),
            ));
        }
        if existing.payment_url.is_some()
            && matches!(existing.status, BillingTopUpStatus::CheckoutCreated)
        {
            return Ok(TopUpCheckout {
                session: existing,
                reused: true,
            });
        }
    }

    let provisioned =
        ensure_owner_wallet(db, lago, owner_id, plan_code, default_overdraft_cap_credits).await?;
    let Some(lago_wallet_id) = provisioned.wallet.lago_wallet_id.clone() else {
        return Err(AppError::BillingProviderUnavailable(
            "billing wallet is missing a Lago wallet id".to_string(),
        ));
    };

    let session_id = deterministic_topup_session_id(owner_id, &idempotency_key);
    let now = Utc::now();
    let pending_session = BillingTopUpSession {
        id: session_id.clone(),
        owner_id: owner_id.to_string(),
        idempotency_key,
        amount_credits,
        lago_wallet_id: lago_wallet_id.clone(),
        lago_wallet_transaction_id: None,
        lago_invoice_id: None,
        payment_url: None,
        payment_provider: None,
        status: BillingTopUpStatus::Pending,
        created_at: now,
        updated_at: now,
    };

    match db
        .collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS)
        .insert_one(&pending_session)
        .await
    {
        Ok(_) => {}
        Err(error) if is_duplicate_key_error(&error) => {
            let existing = find_topup_session_by_id(db, &pending_session.id)
                .await?
                .ok_or_else(|| {
                    AppError::Internal(
                        "billing top-up insert raced but existing session was not found"
                            .to_string(),
                    )
                })?;
            if existing.amount_credits != amount_credits {
                return Err(AppError::Conflict(
                    "idempotency_key was already used with a different amount_credits".to_string(),
                ));
            }
            if existing.payment_url.is_some()
                && matches!(existing.status, BillingTopUpStatus::CheckoutCreated)
            {
                return Ok(TopUpCheckout {
                    session: existing,
                    reused: true,
                });
            }
            if matches!(existing.status, BillingTopUpStatus::Failed) {
                let update = db
                    .collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS)
                    .update_one(
                        doc! { "_id": &existing.id, "status": "failed" },
                        doc! {
                            "$set": {
                                "status": "pending",
                                "updated_at": bson::DateTime::from_chrono(now),
                            }
                        },
                    )
                    .await?;
                if update.matched_count == 1 {
                    // This retry now owns the provider call.
                } else {
                    return Err(AppError::Conflict(
                        "billing top-up checkout is already being created".to_string(),
                    ));
                }
            } else {
                return Err(AppError::Conflict(
                    "billing top-up checkout is already being created".to_string(),
                ));
            }
        }
        Err(error) => return Err(error.into()),
    }

    let external_id = format!("nyxid-topup:{session_id}");
    match lago
        .create_wallet_topup(
            &lago_wallet_id,
            &WalletTopUpInput {
                external_id,
                amount_credits,
            },
        )
        .await
    {
        Ok(checkout) => {
            let mut set_doc = doc! {
                "lago_wallet_transaction_id": checkout.wallet_transaction_id,
                "payment_url": checkout.payment_url,
                "status": "checkout_created",
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            };
            if let Some(invoice_id) = checkout.lago_invoice_id {
                set_doc.insert("lago_invoice_id", invoice_id);
            }
            if let Some(provider) = checkout.payment_provider {
                set_doc.insert("payment_provider", provider);
            }
            db.collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS)
                .update_one(doc! { "_id": &session_id }, doc! { "$set": set_doc })
                .await?;
            let session = find_topup_session_by_id(db, &session_id)
                .await?
                .ok_or_else(|| {
                    AppError::Internal(
                        "billing top-up checkout session disappeared after update".to_string(),
                    )
                })?;
            // Ledger the top-up only once the provider checkout actually
            // exists; a pending stub that fails before this point cannot
            // move money and must not leave a phantom entry.
            super::ledger::record_topup_created(
                db,
                owner_id,
                &session.id,
                amount_credits,
                &lago_wallet_id,
            )
            .await;
            Ok(TopUpCheckout {
                session,
                reused: false,
            })
        }
        Err(error) => {
            db.collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS)
                .update_one(
                    doc! { "_id": &session_id, "status": "pending" },
                    doc! {
                        "$set": {
                            "status": "failed",
                            "updated_at": bson::DateTime::from_chrono(Utc::now()),
                        }
                    },
                )
                .await?;
            Err(error)
        }
    }
}

pub async fn backfill_existing_owner_wallets(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    plan_code: &str,
    default_overdraft_cap_credits: i64,
) -> AppResult<BillingBackfillStats> {
    let mut cursor = db
        .collection::<User>(USERS)
        .find(doc! { "is_active": true })
        .await?;
    let mut stats = BillingBackfillStats::default();
    while let Some(user) = cursor.try_next().await? {
        stats.scanned += 1;
        match ensure_owner_wallet(db, lago, &user.id, plan_code, default_overdraft_cap_credits)
            .await
        {
            Ok(outcome) if outcome.created => stats.provisioned += 1,
            Ok(_) => stats.existing += 1,
            Err(error) => {
                stats.failed += 1;
                tracing::warn!(
                    owner_id = %user.id,
                    error = %error,
                    "Billing wallet backfill failed for owner"
                );
            }
        }
    }
    Ok(stats)
}

async fn owner_provision_input(
    db: &mongodb::Database,
    owner_id: &str,
) -> AppResult<OwnerProvisionInput> {
    let owner = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": owner_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Billing owner not found".to_string()))?;
    Ok(OwnerProvisionInput {
        external_customer_id: owner.id,
        name: owner.display_name,
        email: if owner.email.trim().is_empty() {
            None
        } else {
            Some(owner.email)
        },
    })
}

async fn find_topup_session(
    db: &mongodb::Database,
    owner_id: &str,
    idempotency_key: &str,
) -> AppResult<Option<BillingTopUpSession>> {
    db.collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS)
        .find_one(doc! {
            "owner_id": owner_id,
            "idempotency_key": idempotency_key,
        })
        .await
        .map_err(Into::into)
}

async fn find_topup_session_by_id(
    db: &mongodb::Database,
    session_id: &str,
) -> AppResult<Option<BillingTopUpSession>> {
    db.collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS)
        .find_one(doc! { "_id": session_id })
        .await
        .map_err(Into::into)
}

fn normalize_idempotency_key(idempotency_key: &str) -> AppResult<String> {
    let value = idempotency_key.trim();
    if value.is_empty() {
        return Err(AppError::BadRequest(
            "idempotency_key is required".to_string(),
        ));
    }
    if value.len() > 128 {
        return Err(AppError::BadRequest(
            "idempotency_key must be at most 128 characters".to_string(),
        ));
    }
    if !value
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | ':' | '.'))
    {
        return Err(AppError::BadRequest(
            "idempotency_key contains unsupported characters".to_string(),
        ));
    }
    Ok(value.to_string())
}

fn deterministic_topup_session_id(owner_id: &str, idempotency_key: &str) -> String {
    let digest = sha2::Sha256::digest(format!("{owner_id}:{idempotency_key}").as_bytes());
    format!("btu_{}", hex::encode(&digest[..16]))
}

fn is_duplicate_key_error(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    use async_trait::async_trait;
    use mongodb::{IndexModel, bson::doc, options::IndexOptions};
    use tokio::sync::Barrier;
    use uuid::Uuid;

    use crate::models::billing_topup_session::{
        BillingTopUpSession, BillingTopUpStatus, COLLECTION_NAME as BILLING_TOPUP_SESSIONS,
    };
    use crate::models::billing_wallet::{BillingWallet, COLLECTION_NAME as BILLING_WALLET};
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserProfileConfig, UserType};
    use crate::services::billing::lago_client::{
        Entitlement, LagoAck, LagoError, LagoEvent, LagoUsage, LagoWallet, OwnerProvisionInput,
        WalletTopUpCheckout, WalletTopUpInput,
    };
    use crate::test_utils::connect_test_database;

    use super::{
        create_topup_checkout, deterministic_topup_session_id, ensure_owner_wallet,
        normalize_idempotency_key,
    };

    #[test]
    fn idempotency_key_validation_rejects_unsafe_values() {
        assert!(normalize_idempotency_key("topup-1").is_ok());
        assert!(normalize_idempotency_key("").is_err());
        assert!(normalize_idempotency_key("top up").is_err());
    }

    #[test]
    fn topup_session_id_is_stable_and_prefixed() {
        let first = deterministic_topup_session_id("owner", "key");
        let second = deterministic_topup_session_id("owner", "key");

        assert_eq!(first, second);
        assert!(first.starts_with("btu_"));
    }

    #[tokio::test]
    async fn ensure_owner_wallet_is_idempotent_and_persists_lago_ids() {
        let Some(db) = connect_test_database("billing_provision_wallet").await else {
            return;
        };
        let owner_id = insert_owner(&db, "owner@example.com").await;
        let lago = FakeLago::default();

        let first = ensure_owner_wallet(&db, &lago, &owner_id, "starter", 7)
            .await
            .expect("provision wallet");
        let second = ensure_owner_wallet(&db, &lago, &owner_id, "starter", 7)
            .await
            .expect("replay provision wallet");

        assert!(first.created);
        assert!(!second.created);
        assert_eq!(first.wallet.owner_id, owner_id);
        assert_eq!(first.wallet.lago_customer_id, owner_id);
        assert_eq!(
            first.wallet.lago_subscription_id.as_deref(),
            Some(format!("{owner_id}:starter").as_str())
        );
        assert_eq!(
            first.wallet.lago_wallet_id.as_deref(),
            Some(format!("{owner_id}:wallet").as_str())
        );
        assert_eq!(first.wallet.balance_credits, 123);
        assert_eq!(first.wallet.overdraft_cap_credits, 7);

        let count = db
            .collection::<BillingWallet>(BILLING_WALLET)
            .count_documents(doc! { "owner_id": &owner_id })
            .await
            .expect("count wallets");
        assert_eq!(count, 1);
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn concurrent_wallet_provisioning_persists_one_owner_wallet() {
        let Some(db) = connect_test_database("billing_provision_wallet_concurrent").await else {
            return;
        };
        db.collection::<BillingWallet>(BILLING_WALLET)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "owner_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .expect("create unique billing owner index");
        let owner_id = insert_owner(&db, "concurrent@example.com").await;
        let lago = FakeLago::with_wallet_barrier(2);

        let (first, second) = tokio::join!(
            ensure_owner_wallet(&db, &lago, &owner_id, "starter", 7),
            ensure_owner_wallet(&db, &lago, &owner_id, "starter", 7),
        );
        let first = first.expect("first concurrent provision");
        let second = second.expect("second concurrent provision");

        assert_ne!(first.created, second.created);
        assert_eq!(first.wallet.id, second.wallet.id);
        assert_eq!(first.wallet.owner_id, second.wallet.owner_id);
        assert_eq!(
            first.wallet.lago_customer_id,
            second.wallet.lago_customer_id
        );
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 2);
        let count = db
            .collection::<BillingWallet>(BILLING_WALLET)
            .count_documents(doc! { "owner_id": &owner_id })
            .await
            .expect("count wallets after concurrent provision");
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn wallet_provider_failure_leaves_provisioning_retryable() {
        let Some(db) = connect_test_database("billing_provision_wallet_provider_failure").await
        else {
            return;
        };
        let owner_id = insert_owner(&db, "wallet-failure@example.com").await;
        let lago = FakeLago::with_wallet_failures(1);

        let error = ensure_owner_wallet(&db, &lago, &owner_id, "starter", 7)
            .await
            .expect_err("first wallet provider call should fail");
        assert!(matches!(
            error,
            crate::errors::AppError::BillingProviderUnavailable(_)
        ));
        assert_eq!(
            db.collection::<BillingWallet>(BILLING_WALLET)
                .count_documents(doc! { "owner_id": &owner_id })
                .await
                .expect("count wallets after provider failure"),
            0
        );

        let retry = ensure_owner_wallet(&db, &lago, &owner_id, "starter", 7)
            .await
            .expect("retry wallet provisioning");
        assert!(retry.created);
        assert_eq!(retry.wallet.owner_id, owner_id);
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn create_topup_checkout_returns_hosted_url_and_reuses_idempotency_key() {
        let Some(db) = connect_test_database("billing_topup_checkout").await else {
            return;
        };
        super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new([3u8; 32]));
        let owner_id = insert_owner(&db, "topup@example.com").await;
        let lago = FakeLago::default();

        let first = create_topup_checkout(&db, &lago, &owner_id, "starter", 0, 50, "topup-1")
            .await
            .expect("create checkout");
        let second = create_topup_checkout(&db, &lago, &owner_id, "starter", 0, 50, "topup-1")
            .await
            .expect("replay checkout");

        assert!(!first.reused);
        assert!(second.reused);
        assert_eq!(first.session.id, second.session.id);
        assert_eq!(
            first.session.payment_url.as_deref(),
            Some("https://pay.example/checkout")
        );
        assert_eq!(
            first.session.lago_wallet_transaction_id.as_deref(),
            Some("txn_topup-1")
        );
        assert_eq!(
            first.session.lago_invoice_id.as_deref(),
            Some("invoice_topup-1")
        );

        let session_count = db
            .collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS)
            .count_documents(doc! { "owner_id": &owner_id })
            .await
            .expect("count top-up sessions");
        assert_eq!(session_count, 1);
        assert_eq!(lago.topup_creates.load(Ordering::SeqCst), 1);

        // Exactly one ledger entry: appended when the provider checkout was
        // created, not duplicated by the idempotent replay.
        let ledger_count = db
            .collection::<crate::models::billing_ledger::BillingLedgerEntry>(
                crate::models::billing_ledger::COLLECTION_NAME,
            )
            .count_documents(doc! { "owner_id": &owner_id, "event_type": "topup_created" })
            .await
            .expect("count ledger entries");
        assert_eq!(ledger_count, 1);
    }

    #[tokio::test]
    async fn topup_rejects_idempotency_reuse_with_different_amount() {
        let Some(db) = connect_test_database("billing_topup_conflict").await else {
            return;
        };
        let owner_id = insert_owner(&db, "conflict@example.com").await;
        let lago = FakeLago::default();

        create_topup_checkout(&db, &lago, &owner_id, "starter", 0, 50, "topup-1")
            .await
            .expect("create checkout");
        let error = create_topup_checkout(&db, &lago, &owner_id, "starter", 0, 51, "topup-1")
            .await
            .expect_err("amount mismatch must fail");

        assert!(matches!(error, crate::errors::AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn topup_rejects_non_positive_amount() {
        let Some(db) = connect_test_database("billing_topup_nonpositive").await else {
            return;
        };
        let owner_id = insert_owner(&db, "nonpositive@example.com").await;
        let lago = FakeLago::default();

        for amount in [0_i64, -100] {
            let error =
                create_topup_checkout(&db, &lago, &owner_id, "starter", 0, amount, "topup-np")
                    .await
                    .expect_err("non-positive amount must be rejected");
            assert!(
                matches!(error, crate::errors::AppError::BadRequest(_)),
                "amount_credits = {amount} should be rejected with BadRequest, got {error:?}"
            );
        }
        // Guard rejects before any Lago call or top-up session write.
        assert_eq!(lago.topup_creates.load(Ordering::SeqCst), 0);
        let session_count = db
            .collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS)
            .count_documents(doc! { "owner_id": &owner_id })
            .await
            .expect("count top-up sessions");
        assert_eq!(session_count, 0);
    }

    #[tokio::test]
    async fn topup_provider_failure_marks_session_failed_and_allows_retry() {
        let Some(db) = connect_test_database("billing_topup_provider_failure").await else {
            return;
        };
        let owner_id = insert_owner(&db, "topup-failure@example.com").await;
        let lago = FakeLago::with_topup_failures(1);

        let error = create_topup_checkout(
            &db,
            &lago,
            &owner_id,
            "starter",
            0,
            50,
            "topup-provider-failure",
        )
        .await
        .expect_err("first top-up provider call should fail");
        assert!(matches!(
            error,
            crate::errors::AppError::BillingProviderUnavailable(_)
        ));
        let failed = db
            .collection::<BillingTopUpSession>(BILLING_TOPUP_SESSIONS)
            .find_one(doc! { "owner_id": &owner_id })
            .await
            .expect("read failed top-up session")
            .expect("failed top-up session exists");
        assert_eq!(failed.status, BillingTopUpStatus::Failed);
        assert!(failed.payment_url.is_none());

        let retry = create_topup_checkout(
            &db,
            &lago,
            &owner_id,
            "starter",
            0,
            50,
            "topup-provider-failure",
        )
        .await
        .expect("retry top-up checkout");
        assert_eq!(retry.session.status, BillingTopUpStatus::CheckoutCreated);
        assert!(retry.session.payment_url.is_some());
        assert_eq!(lago.topup_creates.load(Ordering::SeqCst), 2);
    }

    #[derive(Clone, Default)]
    struct FakeLago {
        wallet_creates: Arc<AtomicUsize>,
        topup_creates: Arc<AtomicUsize>,
        wallet_barrier: Option<Arc<Barrier>>,
        wallet_failures_remaining: Arc<AtomicUsize>,
        topup_failures_remaining: Arc<AtomicUsize>,
    }

    impl FakeLago {
        fn with_wallet_barrier(parties: usize) -> Self {
            Self {
                wallet_barrier: Some(Arc::new(Barrier::new(parties))),
                ..Self::default()
            }
        }

        fn with_wallet_failures(failures: usize) -> Self {
            Self {
                wallet_failures_remaining: Arc::new(AtomicUsize::new(failures)),
                ..Self::default()
            }
        }

        fn with_topup_failures(failures: usize) -> Self {
            Self {
                topup_failures_remaining: Arc::new(AtomicUsize::new(failures)),
                ..Self::default()
            }
        }
    }

    #[async_trait]
    impl super::LagoApi for FakeLago {
        async fn ensure_customer(
            &self,
            owner: &OwnerProvisionInput,
        ) -> crate::errors::AppResult<String> {
            Ok(owner.external_customer_id.clone())
        }

        async fn ensure_subscription(
            &self,
            customer_id: &str,
            plan_code: &str,
        ) -> crate::errors::AppResult<String> {
            Ok(format!("{customer_id}:{plan_code}"))
        }

        async fn ensure_wallet(&self, customer_id: &str) -> crate::errors::AppResult<LagoWallet> {
            self.wallet_creates.fetch_add(1, Ordering::SeqCst);
            if let Some(barrier) = &self.wallet_barrier {
                barrier.wait().await;
            }
            if take_failure(&self.wallet_failures_remaining) {
                return Err(crate::errors::AppError::BillingProviderUnavailable(
                    "simulated Lago wallet failure".to_string(),
                ));
            }
            Ok(LagoWallet {
                id: format!("{customer_id}:wallet"),
                balance_credits: 123,
            })
        }

        async fn create_wallet_topup(
            &self,
            _wallet_id: &str,
            _request: &WalletTopUpInput,
        ) -> crate::errors::AppResult<WalletTopUpCheckout> {
            self.topup_creates.fetch_add(1, Ordering::SeqCst);
            if take_failure(&self.topup_failures_remaining) {
                return Err(crate::errors::AppError::BillingProviderUnavailable(
                    "simulated Lago top-up failure".to_string(),
                ));
            }
            Ok(WalletTopUpCheckout {
                wallet_transaction_id: "txn_topup-1".to_string(),
                lago_invoice_id: Some("invoice_topup-1".to_string()),
                payment_url: "https://pay.example/checkout".to_string(),
                payment_provider: Some("stripe".to_string()),
            })
        }

        async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
            Ok(LagoAck {
                transaction_id: event.transaction_id.clone(),
            })
        }

        async fn record_events_batch(
            &self,
            events: &[LagoEvent],
        ) -> Result<Vec<LagoAck>, LagoError> {
            Ok(events
                .iter()
                .map(|event| LagoAck {
                    transaction_id: event.transaction_id.clone(),
                })
                .collect())
        }

        async fn current_usage(
            &self,
            customer_id: &str,
            subscription_id: &str,
        ) -> crate::errors::AppResult<LagoUsage> {
            Ok(LagoUsage {
                customer_id: customer_id.to_string(),
                subscription_id: subscription_id.to_string(),
                raw: serde_json::json!({}),
            })
        }

        async fn wallet_balance(&self, _customer_id: &str) -> crate::errors::AppResult<i64> {
            Ok(123)
        }

        async fn entitlements(
            &self,
            _subscription_id: &str,
        ) -> crate::errors::AppResult<Vec<Entitlement>> {
            Ok(Vec::new())
        }
    }

    fn take_failure(remaining: &AtomicUsize) -> bool {
        remaining
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                value.checked_sub(1)
            })
            .is_ok()
    }

    async fn insert_owner(db: &mongodb::Database, email: &str) -> String {
        let now = chrono::Utc::now();
        let id = Uuid::new_v4().to_string();
        let user = User {
            id: id.clone(),
            email: email.to_string(),
            password_hash: None,
            display_name: Some("Billing Owner".to_string()),
            slug: None,
            avatar_url: None,
            email_verified: true,
            email_verification_token: None,
            password_reset_token: None,
            password_reset_expires_at: None,
            is_active: true,
            is_admin: false,
            is_operator: false,
            role_ids: vec![],
            group_ids: vec![],
            invite_code_id: None,
            mfa_enabled: false,
            social_provider: None,
            social_provider_id: None,
            user_type: UserType::Person,
            primary_org_id: None,
            created_at: now,
            updated_at: now,
            last_login_at: None,
            profile_config: UserProfileConfig::default(),
        };
        db.collection::<User>(USERS)
            .insert_one(user)
            .await
            .expect("insert test owner");
        id
    }
}
