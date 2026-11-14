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
use crate::models::coordination::LEASE_COLLECTION_NAME as COORDINATION_LEASES;
use crate::models::user::{COLLECTION_NAME as USERS, User};

use super::lago_client::{
    DispatchWindow, HTTP_TIMEOUT_SECS, LagoApi, OwnerProvisionInput, ProviderDispatch,
    WalletTopUpInput,
};
// Six client timeouts allow delayed responses/read replicas to settle; the
// provider's unique active customer/code index also fences a late first POST.
const WALLET_CREATE_QUIET_SECS: u64 = HTTP_TIMEOUT_SECS * 6;
use crate::services::coordination_service::{LeaseStore, LeaseToken, cluster_lease_runtime};

#[derive(Clone, Copy)]
pub struct WalletProvisioningOptions<'a> {
    pub plan_code: &'a str,
    pub default_overdraft_cap_credits: i64,
    pub tax_cache_ttl_secs: u64,
}

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
    let wallet = db
        .collection::<BillingWallet>(BILLING_WALLET)
        .find_one(doc! { "owner_id": owner_id })
        .await?;
    Ok(wallet)
}

pub async fn ensure_owner_wallet(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    owner_id: &str,
    options: WalletProvisioningOptions<'_>,
) -> AppResult<ProvisionedWallet> {
    let WalletProvisioningOptions {
        plan_code,
        default_overdraft_cap_credits,
        tax_cache_ttl_secs,
    } = options;
    super::exact_migration::require_ready(db).await?;
    if let Some(wallet) = get_wallet(db, owner_id).await? {
        return Ok(ProvisionedWallet {
            wallet,
            created: false,
        });
    }

    let runtime = cluster_lease_runtime();
    let name = format!("billing:provision:{owner_id}");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    let durable_db = db.client().database_with_options(
        db.name(),
        mongodb::options::DatabaseOptions::builder()
            .write_concern(
                mongodb::options::WriteConcern::builder()
                    .w(mongodb::options::Acknowledgment::Majority)
                    .journal(true)
                    .build(),
            )
            .build(),
    );
    let token = loop {
        if let Some(token) = runtime.acquire(&durable_db, &name).await? {
            break token;
        }
        if let Some(wallet) = get_wallet(db, owner_id).await? {
            return Ok(ProvisionedWallet {
                wallet,
                created: false,
            });
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(AppError::BillingProviderUnavailable(
                "Wallet provisioning is in progress; retry".into(),
            ));
        }
        tokio::time::sleep(
            runtime
                .contender_wait()
                .min(deadline.saturating_duration_since(tokio::time::Instant::now())),
        )
        .await;
    };
    // The commit removes this lease in its transaction. Keep polling that
    // transaction while renewal waits for its write lock.
    let result = tokio::select! {
        biased;
        result = provision_under_lease(
            &durable_db, lago, owner_id, plan_code,
            default_overdraft_cap_credits, &token,
        ) => Some(result),
        _ = runtime.run_while_renewed(&durable_db, &token, std::future::pending::<()>()) => None,
    };
    let result = result.unwrap_or_else(|| {
        Err(AppError::BillingProviderUnavailable(
            "Wallet provisioning lease lost; retry readback".into(),
        ))
    });
    match &result {
        Ok(provisioned) => {
            // New wallets delete the checkpoint atomically with their insert.
            // An existing wallet can instead have won during the cold recheck.
            if !provisioned.created {
                delete_provision_checkpoint(&durable_db, &token).await?;
            }
        }
        Err(_) => {
            LeaseStore::release(&durable_db, &token).await?;
        }
    }
    if let Ok(provisioned) = &result
        && provisioned.created
        && let Err(error) =
            super::taxes::refresh(db, lago, &provisioned.wallet, tax_cache_ttl_secs).await
    {
        tracing::warn!(%owner_id, %error, "Initial Lago tax profile refresh failed");
    }
    result
}

fn provision_token_filter(token: &LeaseToken) -> bson::Document {
    doc! {
        "_id": &token.name,
        "holder.instance_id": &token.holder.instance_id,
        "holder.generation_id": &token.holder.generation_id,
        "lease_id": &token.lease_id,
    }
}

async fn delete_provision_checkpoint(db: &mongodb::Database, token: &LeaseToken) -> AppResult<()> {
    db.collection::<bson::Document>(COORDINATION_LEASES)
        .delete_one(provision_token_filter(token))
        .await?;
    Ok(())
}

async fn provision_under_lease(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    owner_id: &str,
    plan_code: &str,
    default_overdraft_cap_credits: i64,
    token: &LeaseToken,
) -> AppResult<ProvisionedWallet> {
    if let Some(wallet) = get_wallet(db, owner_id).await? {
        return Ok(ProvisionedWallet {
            wallet,
            created: false,
        });
    }
    let checkpoint = LeaseStore::load_checkpoint(db, token).await?;
    let (customer_id, subscription_id) = if let Some(checkpoint) = checkpoint {
        let saved = checkpoint
            .as_document()
            .ok_or_else(|| AppError::Internal("Invalid wallet provisioning checkpoint".into()))?;
        let customer = saved
            .get_str("customer_id")
            .map_err(|e| AppError::Internal(e.to_string()))?
            .to_owned();
        let subscription = saved
            .get_str("subscription_id")
            .map_err(|e| AppError::Internal(e.to_string()))?
            .to_owned();
        (customer, subscription)
    } else {
        let owner = owner_provision_input(db, owner_id).await?;
        let customer = lago.ensure_customer(&owner).await?;
        let subscription = lago.ensure_subscription(&customer, plan_code).await?;
        (customer, subscription)
    };
    let lago_wallet = match lago.find_wallet(&customer_id).await? {
        Some(wallet) => wallet,
        None => {
            let mut filter = crate::services::coordination_service::active_lease_filter(
                &token.name,
                &token.holder,
                &token.lease_id,
            );
            filter.insert("$or", bson::bson!([
                { "checkpoint.wallet_dispatch_started": { "$ne": true } },
                { "$expr": { "$lte": [
                    "$checkpoint.dispatch_started_at",
                    { "$dateSubtract": { "startDate": "$$NOW", "unit": "second", "amount": WALLET_CREATE_QUIET_SECS as i64 } }
                ] } }
            ]));
            let started = db
                .collection::<bson::Document>(COORDINATION_LEASES)
                .update_one(
                    filter,
                    vec![doc! { "$set": {
                        "record_kind": "checkpoint",
                        "checkpoint.customer_id": { "$literal": &customer_id },
                        "checkpoint.subscription_id": { "$literal": &subscription_id },
                        "checkpoint.wallet_dispatch_started": true,
                        "checkpoint.dispatch_started_at": "$$NOW",
                    }}],
                )
                .await?;
            let dispatch_window = DispatchWindow::capture();
            if started.matched_count == 0 {
                return Err(AppError::BillingProviderUnavailable(
                    "Wallet creation is unresolved; awaiting readback or the two-minute quiet period".into(),
                ));
            }
            match lago
                .dispatch_wallet_create(&customer_id, &dispatch_window)
                .await
            {
                ProviderDispatch::Applied(wallet) => wallet,
                ProviderDispatch::AlreadyExists(error) => {
                    lago.find_wallet(&customer_id).await?.ok_or(error)?
                }
                ProviderDispatch::Refused(error) => {
                    db.collection::<bson::Document>(COORDINATION_LEASES)
                        .update_one(
                            provision_token_filter(token),
                            doc! { "$unset": {
                                "checkpoint.wallet_dispatch_started": "",
                                "checkpoint.dispatch_started_at": "",
                            }},
                        )
                        .await?;
                    return Err(error);
                }
                ProviderDispatch::Ambiguous(error) => return Err(error),
            }
        }
    };
    let now = Utc::now();
    let wallet = BillingWallet {
        snapshot_generation: 0,
        tax_generation: 0,
        tax_guard_token: None,
        id: Uuid::new_v4().to_string(),
        owner_id: owner_id.to_string(),
        lago_customer_id: customer_id,
        lago_wallet_id: Some(lago_wallet.id),
        lago_subscription_id: Some(subscription_id),
        plan_kind: PlanKind::Prepaid,
        balance_credits: lago_wallet.balance_credits,
        reserved_credits: crate::models::credits::Credits::from_whole(0),
        pending_lago_debits: crate::models::credits::Credits::from_whole(0),
        pending_topup_expiry_credits: crate::models::credits::Credits::from_whole(0),
        has_payment_instrument: false,
        overdraft_cap_credits: crate::models::credits::Credits::from_whole(
            default_overdraft_cap_credits,
        ),
        suspended: false,
        collection_state: CollectionState::Good,
        topup_expiry_checked_at: None,
        active_topup_expiry: None,
        balance_synced_at: now,
        created_at: now,
        updated_at: now,
    };

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        let mut session = db.client().start_session().await?;
        let database = db.clone();
        let new_wallet = wallet.clone();
        let commit_token = token.clone();
        let result = session
            .start_transaction()
            .write_concern(
                mongodb::options::WriteConcern::builder()
                    .w(mongodb::options::Acknowledgment::Majority)
                    .journal(true)
                    .build(),
            )
            .and_run2(async move |session| {
                let operation: AppResult<()> = async {
                    // There is no commit/delete crash gap: wallet, funding
                    // posting and exact-token checkpoint removal commit together.
                    let deleted = database
                        .collection::<bson::Document>(COORDINATION_LEASES)
                        .delete_one(provision_token_filter(&commit_token))
                        .session(&mut *session)
                        .await?;
                    if deleted.deleted_count != 1 {
                        return Err(AppError::BillingProviderUnavailable(
                            "Wallet provisioning lease changed before local commit; retry readback"
                                .into(),
                        ));
                    }
                    let mut document = bson::to_document(&new_wallet)
                        .map_err(|e| AppError::Internal(e.to_string()))?;
                    document.insert("exact_accounting_version", 2);
                    database
                        .collection::<bson::Document>(BILLING_WALLET)
                        .insert_one(document)
                        .session(&mut *session)
                        .await?;
                    if new_wallet.balance_credits != crate::models::credits::Credits::ZERO {
                        let account = format!("wallet:{}", new_wallet.owner_id);
                        let amount = new_wallet.balance_credits;
                        let postings = if amount > crate::models::credits::Credits::ZERO {
                            super::ledger::transfer(account, "external:lago".into(), amount)
                        } else {
                            super::ledger::transfer("external:lago".into(), account, -amount)
                        };
                        let entry = super::ledger::exact_entry(
                            &new_wallet.owner_id,
                            &new_wallet.id,
                            "wallet_funded",
                            format!("wallet-created:{}", new_wallet.id),
                            postings,
                            None,
                        );
                        super::ledger::append_in_session(&database, session, entry).await?;
                    }
                    Ok(())
                }
                .await;
                crate::services::api_key_mutation_service::transaction_result(operation)
            })
            .await;
        match result {
            Ok(()) => {
                return Ok(ProvisionedWallet {
                    wallet,
                    created: true,
                });
            }
            Err(error) if is_duplicate_key_error(&error) => {
                if let Some(existing) = get_wallet(db, owner_id).await? {
                    return Ok(ProvisionedWallet {
                        wallet: existing,
                        created: false,
                    });
                }
                if tokio::time::Instant::now() >= deadline {
                    return Err(error.into());
                }
            }
            Err(error) => {
                return Err(
                    crate::services::api_key_mutation_service::map_transaction_error(error),
                );
            }
        }
    }
}

pub async fn create_topup_checkout(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    owner_id: &str,
    options: WalletProvisioningOptions<'_>,
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

    let provisioned = ensure_owner_wallet(db, lago, owner_id, options).await?;
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
        paid_at: None,
        credits_expire_at: None,
        expired_credits: crate::models::credits::Credits::ZERO,
        credits_expired_at: None,
        expiry_void_transaction_id: None,
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
    options: WalletProvisioningOptions<'_>,
) -> AppResult<BillingBackfillStats> {
    let mut cursor = db
        .collection::<User>(USERS)
        .find(doc! { "is_active": true })
        .await?;
    let mut stats = BillingBackfillStats::default();
    while let Some(user) = cursor.try_next().await? {
        stats.scanned += 1;
        match ensure_owner_wallet(db, lago, &user.id, options).await {
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
        WalletProvisioningOptions, create_topup_checkout, deterministic_topup_session_id,
        ensure_owner_wallet, normalize_idempotency_key,
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

    fn wallet_options(cap: i64) -> WalletProvisioningOptions<'static> {
        WalletProvisioningOptions {
            plan_code: "starter",
            default_overdraft_cap_credits: cap,
            tax_cache_ttl_secs: 900,
        }
    }

    #[tokio::test]
    async fn ensure_owner_wallet_is_idempotent_and_persists_lago_ids() {
        let Some(db) = connect_test_database("billing_provision_wallet").await else {
            return;
        };
        let owner_id = insert_owner(&db, "owner@example.com").await;
        let lago = FakeLago::default();

        let first = ensure_owner_wallet(&db, &lago, &owner_id, wallet_options(7))
            .await
            .expect("provision wallet");
        let second = ensure_owner_wallet(&db, &lago, &owner_id, wallet_options(7))
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
        assert_eq!(
            first.wallet.balance_credits,
            crate::models::credits::Credits::from_whole(123)
        );
        assert_eq!(
            first.wallet.overdraft_cap_credits,
            crate::models::credits::Credits::from_whole(7)
        );

        let count = db
            .collection::<BillingWallet>(BILLING_WALLET)
            .count_documents(doc! { "owner_id": &owner_id })
            .await
            .expect("count wallets");
        assert_eq!(count, 1);
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 1);
        let entry = db
            .collection::<crate::models::billing_ledger::BillingLedgerEntry>("billing_ledger")
            .find_one(doc! { "movement": "wallet_funded" })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(entry.postings[0].account, format!("wallet:{owner_id}"));
        assert_eq!(entry.postings[1].account, "external:lago");
        assert_eq!(
            db.collection::<bson::Document>("billing_ledger")
                .count_documents(doc! { "movement": "opening_balance" })
                .await
                .unwrap(),
            0
        );
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
        let lago = FakeLago::with_wallet_barrier(1);

        let (first, second) = tokio::join!(
            ensure_owner_wallet(
                &db,
                &lago,
                &owner_id,
                WalletProvisioningOptions {
                    plan_code: "starter",
                    default_overdraft_cap_credits: 7,
                    tax_cache_ttl_secs: 900
                }
            ),
            ensure_owner_wallet(
                &db,
                &lago,
                &owner_id,
                WalletProvisioningOptions {
                    plan_code: "starter",
                    default_overdraft_cap_credits: 7,
                    tax_cache_ttl_secs: 900
                }
            ),
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
        let count = db
            .collection::<BillingWallet>(BILLING_WALLET)
            .count_documents(doc! { "owner_id": &owner_id })
            .await
            .expect("count wallets after concurrent provision");
        assert_eq!(count, 1);
        let calls = (
            lago.customer_calls.load(Ordering::SeqCst),
            lago.subscription_calls.load(Ordering::SeqCst),
            lago.wallet_creates.load(Ordering::SeqCst),
        );
        db.drop().await.unwrap();
        assert_eq!(calls, (1, 1, 1));
    }

    #[tokio::test]
    async fn ambiguous_wallet_create_recovers_without_a_second_create() {
        let Some(db) = connect_test_database("billing_provision_wallet_provider_failure").await
        else {
            return;
        };
        let owner_id = insert_owner(&db, "wallet-failure@example.com").await;
        let lago = FakeLago::with_wallet_failures(1);

        let error = ensure_owner_wallet(&db, &lago, &owner_id, wallet_options(7))
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

        let retry = ensure_owner_wallet(&db, &lago, &owner_id, wallet_options(7))
            .await
            .expect("retry wallet provisioning");
        assert!(retry.created);
        assert_eq!(retry.wallet.owner_id, owner_id);
        let calls = lago.wallet_creates.load(Ordering::SeqCst);
        db.drop().await.unwrap();
        assert_eq!(calls, 1);
    }

    #[tokio::test]
    async fn provisioning_2468_takeover_reads_back_while_first_create_is_paused() {
        let db = connect_test_database("provision_takeover").await.unwrap();
        db.collection::<BillingWallet>(BILLING_WALLET)
            .create_index(
                IndexModel::builder()
                    .keys(doc! { "owner_id": 1 })
                    .options(IndexOptions::builder().unique(true).build())
                    .build(),
            )
            .await
            .unwrap();
        let owner = insert_owner(&db, "takeover@example.com").await;
        let lago = FakeLago::with_wallet_barrier(2);
        let (first, ()) = tokio::join!(
            ensure_owner_wallet(&db, &lago, &owner, wallet_options(0)),
            async {
                lago.wallet_entered.notified().await;
                db.collection::<bson::Document>("coordination_leases")
                    .update_one(
                        doc! { "_id": format!("billing:provision:{owner}") },
                        doc! { "$set": { "expires_at": bson::DateTime::from_millis(0) } },
                    )
                    .await
                    .unwrap();
                let error = ensure_owner_wallet(&db, &lago, &owner, wallet_options(0))
                    .await
                    .unwrap_err();
                assert!(error.to_string().contains("unresolved"));
                assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 1);
                lago.wallet_barrier.as_ref().unwrap().wait().await;
            }
        );
        assert!(
            first.is_err(),
            "a stale token cannot commit a wallet and erase a successor checkpoint"
        );
        let recovered = ensure_owner_wallet(&db, &lago, &owner, wallet_options(0))
            .await
            .unwrap();
        assert!(recovered.created);
        assert_eq!(lago.customer_calls.load(Ordering::SeqCst), 1);
        assert_eq!(lago.subscription_calls.load(Ordering::SeqCst), 1);
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 1);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn provisioning_2468_hidden_receipt_does_not_repeat_inside_quiet_period() {
        let db = connect_test_database("provision_hidden_receipt")
            .await
            .unwrap();
        let owner = insert_owner(&db, "hidden@example.com").await;
        let lago = FakeLago::with_wallet_failures(1);
        assert!(
            ensure_owner_wallet(&db, &lago, &owner, wallet_options(0))
                .await
                .is_err()
        );
        let receipt = lago.wallet_receipt.lock().unwrap().take().unwrap();
        for _ in 0..3 {
            let error = ensure_owner_wallet(&db, &lago, &owner, wallet_options(0))
                .await
                .unwrap_err();
            assert!(error.to_string().contains("unresolved"));
        }
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 1);
        assert_eq!(lago.customer_calls.load(Ordering::SeqCst), 1);
        assert_eq!(lago.subscription_calls.load(Ordering::SeqCst), 1);
        *lago.wallet_receipt.lock().unwrap() = Some(receipt);
        let recovered = ensure_owner_wallet(&db, &lago, &owner, wallet_options(0))
            .await
            .unwrap();
        assert!(recovered.created);
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 1);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn provisioning_r2_refusal_retries_and_success_deletes_checkpoint() {
        let db = connect_test_database("provision_r2_refusal").await.unwrap();
        let owner = insert_owner(&db, "refusal@example.com").await;
        let lago = FakeLago::default();
        lago.wallet_refusals.store(1, Ordering::SeqCst);
        assert!(
            ensure_owner_wallet(&db, &lago, &owner, wallet_options(0))
                .await
                .is_err()
        );
        let retry = ensure_owner_wallet(&db, &lago, &owner, wallet_options(0)).await;
        let checkpoints = db
            .collection::<bson::Document>("coordination_leases")
            .count_documents(doc! {"_id": format!("billing:provision:{owner}")})
            .await
            .unwrap();
        db.drop().await.unwrap();
        assert!(
            retry.is_ok(),
            "definite refusal must permit retry: {retry:?}"
        );
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 2);
        assert_eq!(checkpoints, 0, "success must remove checkpoint");
    }

    #[tokio::test]
    async fn provisioning_r2_quiet_period_then_one_retry() {
        let db = connect_test_database("provision_r2_quiet").await.unwrap();
        let owner = insert_owner(&db, "quiet@example.com").await;
        let lago = FakeLago::with_wallet_failures(1);
        assert!(
            ensure_owner_wallet(&db, &lago, &owner, wallet_options(0))
                .await
                .is_err()
        );
        lago.wallet_receipt.lock().unwrap().take();
        db.collection::<bson::Document>("coordination_leases")
            .update_one(
                doc! {"_id": format!("billing:provision:{owner}")},
                doc! {"$set": {"checkpoint.dispatch_started_at": bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::seconds(60))}},
            )
            .await
            .unwrap();
        for _ in 0..3 {
            assert!(
                ensure_owner_wallet(&db, &lago, &owner, wallet_options(0))
                    .await
                    .is_err()
            );
        }
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 1);
        db.collection::<bson::Document>("coordination_leases")
            .update_one(
                doc! {"_id": format!("billing:provision:{owner}")},
                doc! {"$set": {"checkpoint.dispatch_started_at": bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::seconds(125))}},
            )
            .await
            .unwrap();
        let (a, b) = tokio::join!(
            ensure_owner_wallet(&db, &lago, &owner, wallet_options(0)),
            ensure_owner_wallet(&db, &lago, &owner, wallet_options(0))
        );
        let count = db
            .collection::<bson::Document>("coordination_leases")
            .count_documents(doc! {"_id": format!("billing:provision:{owner}")})
            .await
            .unwrap();
        db.drop().await.unwrap();
        assert!(
            a.is_ok() && b.is_ok(),
            "retry after quiet period: {a:?} / {b:?}"
        );
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 2);
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn provisioning_r2_late_first_create_overlaps_retry_with_one_unique_wallet() {
        let db = connect_test_database("provision_r2_late").await.unwrap();
        let owner = insert_owner(&db, "late@example.com").await;
        let lago = FakeLago {
            enforce_code_uniqueness: true,
            ..FakeLago::with_wallet_barrier(2)
        };
        let (first, second) = tokio::join!(
            ensure_owner_wallet(&db, &lago, &owner, wallet_options(0)),
            async {
                lago.wallet_entered.notified().await;
                db.collection::<bson::Document>("coordination_leases").update_one(
                    doc! {"_id": format!("billing:provision:{owner}")},
                    doc! {"$set": {"expires_at": bson::DateTime::from_millis(0), "checkpoint.dispatch_started_at": bson::DateTime::from_millis(0)}}
                ).await.unwrap();
                let second = ensure_owner_wallet(&db, &lago, &owner, wallet_options(0)).await;
                if second.is_err() && lago.wallet_creates.load(Ordering::SeqCst) == 1 {
                    lago.wallet_barrier.as_ref().unwrap().wait().await;
                }
                second
            }
        );
        let rows = db
            .collection::<BillingWallet>(BILLING_WALLET)
            .count_documents(doc! {"owner_id": &owner})
            .await
            .unwrap();
        db.drop().await.unwrap();
        assert!(
            second.is_ok(),
            "successor should create/read back after quiet period: {second:?}; first: {first:?}"
        );
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 2);
        assert_eq!(lago.accepted_wallets.load(Ordering::SeqCst), 1);
        assert_eq!(rows, 1);
    }

    #[tokio::test]
    async fn provisioning_r2_success_removes_checkpoint() {
        let db = connect_test_database("provision_r2_cleanup").await.unwrap();
        let owner = insert_owner(&db, "cleanup@example.com").await;
        ensure_owner_wallet(&db, &FakeLago::default(), &owner, wallet_options(0))
            .await
            .unwrap();
        let count = db
            .collection::<bson::Document>("coordination_leases")
            .count_documents(doc! {"_id": format!("billing:provision:{owner}")})
            .await
            .unwrap();
        db.drop().await.unwrap();
        assert_eq!(count, 0, "checkpoint leaked after successful wallet commit");
    }

    async fn assert_provisioning_sending_window(monotonic: u64, wall: u64, sends: bool) {
        let db = connect_test_database("provision_sending_window")
            .await
            .unwrap();
        let owner = insert_owner(&db, "dispatch@example.com").await;
        let http = Arc::new(
            super::super::lago_client::dispatch_test_support::DispatchHttp::new(
                &db, monotonic, wall,
            )
            .await,
        );
        let lago = FakeLago {
            dispatch_http: Some(http.clone()),
            ..Default::default()
        };
        let result = ensure_owner_wallet(&db, &lago, &owner, wallet_options(0)).await;
        let saved = db
            .collection::<bson::Document>("coordination_leases")
            .find_one(doc! {"_id": format!("billing:provision:{owner}")})
            .await
            .unwrap();
        let count = http.posts();
        db.drop().await.unwrap();
        assert_eq!(
            count,
            usize::from(sends),
            "expired sender must make no HTTP request"
        );
        assert_eq!(result.is_ok(), sends);
        if !sends {
            let saved = saved.unwrap();
            let checkpoint = saved.get_document("checkpoint").unwrap();
            assert_eq!(checkpoint.get_bool("wallet_dispatch_started"), Ok(true));
            assert!(checkpoint.get_datetime("dispatch_started_at").is_ok());
            assert!(saved.get_datetime("expires_at").unwrap() <= &bson::DateTime::now());
        } else {
            assert!(saved.is_none());
        }
    }

    #[tokio::test]
    async fn provisioning_sending_window_rejects_monotonic_delay() {
        assert_provisioning_sending_window(31, 0, false).await;
    }
    #[tokio::test]
    async fn provisioning_sending_window_rejects_wall_clock_delay() {
        assert_provisioning_sending_window(0, 31, false).await;
    }
    #[tokio::test]
    async fn provisioning_sending_window_allows_prompt_send() {
        assert_provisioning_sending_window(1, 1, true).await;
    }

    #[tokio::test]
    async fn shutdown_provisioning_request_finishes_or_retains_ttl_on_abort() {
        for abort in [false, true] {
            let db = connect_test_database("shutdown_provisioning_request")
                .await
                .unwrap();
            let owner = insert_owner(&db, "drain@example.com").await;
            let lago = FakeLago::with_wallet_barrier(2);
            let shutdown = crate::services::shutdown::Runtime::default();
            let request = shutdown.request();
            let mut operation =
                Box::pin(ensure_owner_wallet(&db, &lago, &owner, wallet_options(0)));
            tokio::select! {
                result = &mut operation => panic!("provisioning unexpectedly finished: {result:?}"),
                () = lago.wallet_entered.notified() => {},
            }
            shutdown.begin_drain();
            assert!(shutdown.background().is_none());
            assert_eq!(shutdown.counts().http_requests, 1);
            assert_eq!(shutdown.counts().background, 0);
            if abort {
                drop(operation);
                let checkpoint = db
                    .collection::<bson::Document>("coordination_leases")
                    .find_one(doc! {"_id":format!("billing:provision:{owner}")})
                    .await
                    .unwrap()
                    .unwrap();
                assert!(checkpoint.get_datetime("expires_at").unwrap() > &bson::DateTime::now());
                assert!(
                    checkpoint
                        .get_document("checkpoint")
                        .unwrap()
                        .get_datetime("dispatch_started_at")
                        .is_ok()
                );
                let peer = crate::services::coordination_service::ClusterLeaseRuntime::new(
                    crate::models::coordination::CoordinationHolder {
                        instance_id: "provision-peer".into(),
                        generation_id: "peer-generation".into(),
                    },
                    std::time::Duration::from_secs(30),
                    std::time::Duration::from_secs(10),
                );
                assert!(
                    peer.acquire(&db, &format!("billing:provision:{owner}"))
                        .await
                        .unwrap()
                        .is_none()
                );
            } else {
                let (result, _) =
                    tokio::join!(operation, lago.wallet_barrier.as_ref().unwrap().wait());
                assert!(result.unwrap().created);
                assert_eq!(
                    db.collection::<bson::Document>("coordination_leases")
                        .count_documents(doc! {})
                        .await
                        .unwrap(),
                    0
                );
            }
            drop(request);
            assert_eq!(shutdown.counts().http_requests, 0);
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn provisioning_2468_command_counts() {
        use mongodb::event::{EventHandler, command::CommandEvent};
        let commands = Arc::new(std::sync::Mutex::new(Vec::new()));
        let recorded = commands.clone();
        let handler = EventHandler::<CommandEvent>::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                let collection = event.command.get_str(&event.command_name).unwrap_or("");
                recorded
                    .lock()
                    .unwrap()
                    .push(format!("{}:{}", event.command_name, collection));
            }
        });
        let db = crate::test_utils::connect_test_database_with_command_handler(
            "provision_counts",
            handler,
        )
        .await
        .unwrap();
        let owner = insert_owner(&db, "counts@example.com").await;
        let lago = FakeLago::default();
        super::super::exact_migration::require_ready(&db)
            .await
            .unwrap();
        commands.lock().unwrap().clear();
        ensure_owner_wallet(&db, &lago, &owner, wallet_options(0))
            .await
            .unwrap();
        println!(
            "PROVISION_COMMANDS cold: {:?}",
            std::mem::take(&mut *commands.lock().unwrap())
        );
        ensure_owner_wallet(&db, &lago, &owner, wallet_options(0))
            .await
            .unwrap();
        let warm = std::mem::take(&mut *commands.lock().unwrap());
        println!("PROVISION_COMMANDS warm: {warm:?}");
        assert_eq!(warm, vec!["find:billing_wallet"]);
        super::super::startup_diagnostics(&db).await.unwrap();
        println!(
            "PROVISION_COMMANDS integrity_diagnostics: {:?}",
            std::mem::take(&mut *commands.lock().unwrap())
        );
        let now = bson::DateTime::now();
        db.collection::<bson::Document>(BILLING_WALLET)
            .update_one(
                doc! { "owner_id": &owner },
                doc! { "$set": { "active_topup_expiry": {
                    "operation_id": "uncertain", "processing_token": "old-holder",
                    "lease_until": bson::DateTime::from_millis(0),
                    "amount": crate::models::credits::Credits::from_whole(1),
                    "items": [], "created_at": now, "updated_at": now,
                    "dispatch_started_at": now,
                } } },
            )
            .await
            .unwrap();
        commands.lock().unwrap().clear();
        let diagnostics = super::super::startup_diagnostics(&db).await.unwrap();
        assert!(
            diagnostics
                .iter()
                .any(|d| d.code == "billing_expiry_unresolved")
        );
        println!(
            "PROVISION_COMMANDS integrity_unresolved: {:?}",
            std::mem::take(&mut *commands.lock().unwrap())
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn create_topup_checkout_returns_hosted_url_and_reuses_idempotency_key() {
        let Some(db) = connect_test_database("billing_topup_checkout").await else {
            return;
        };
        super::super::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
            super::super::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        ));
        let owner_id = insert_owner(&db, "topup@example.com").await;
        let lago = FakeLago::default();

        let first = create_topup_checkout(&db, &lago, &owner_id, wallet_options(0), 50, "topup-1")
            .await
            .expect("create checkout");
        let second = create_topup_checkout(&db, &lago, &owner_id, wallet_options(0), 50, "topup-1")
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
            .count_documents(doc! { "owner_id": &owner_id, "movement": "topup_created" })
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

        create_topup_checkout(&db, &lago, &owner_id, wallet_options(0), 50, "topup-1")
            .await
            .expect("create checkout");
        let error = create_topup_checkout(&db, &lago, &owner_id, wallet_options(0), 51, "topup-1")
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
                create_topup_checkout(&db, &lago, &owner_id, wallet_options(0), amount, "topup-np")
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
            wallet_options(0),
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
            wallet_options(0),
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
        customer_calls: Arc<AtomicUsize>,
        subscription_calls: Arc<AtomicUsize>,
        wallet_receipt: Arc<std::sync::Mutex<Option<LagoWallet>>>,
        wallet_creates: Arc<AtomicUsize>,
        wallet_refusals: Arc<AtomicUsize>,
        dispatch_http: Option<Arc<super::super::lago_client::dispatch_test_support::DispatchHttp>>,
        enforce_code_uniqueness: bool,
        accepted_wallets: Arc<AtomicUsize>,
        topup_creates: Arc<AtomicUsize>,
        wallet_barrier: Option<Arc<Barrier>>,
        wallet_entered: Arc<tokio::sync::Notify>,
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
        async fn tax_config(
            &self,
            _: &str,
            _: &str,
        ) -> crate::errors::AppResult<crate::services::billing::lago_client::LagoTaxConfig>
        {
            Ok(crate::services::billing::taxes::zero_config())
        }
        async fn ensure_customer(
            &self,
            owner: &OwnerProvisionInput,
        ) -> crate::errors::AppResult<String> {
            self.customer_calls.fetch_add(1, Ordering::SeqCst);
            Ok(owner.external_customer_id.clone())
        }

        async fn ensure_subscription(
            &self,
            customer_id: &str,
            plan_code: &str,
        ) -> crate::errors::AppResult<String> {
            self.subscription_calls.fetch_add(1, Ordering::SeqCst);
            Ok(format!("{customer_id}:{plan_code}"))
        }

        async fn find_wallet(&self, _: &str) -> crate::errors::AppResult<Option<LagoWallet>> {
            Ok(self.wallet_receipt.lock().unwrap().clone())
        }

        async fn ensure_wallet(&self, customer_id: &str) -> crate::errors::AppResult<LagoWallet> {
            self.wallet_creates.fetch_add(1, Ordering::SeqCst);
            if take_failure(&self.wallet_refusals) {
                return Err(crate::errors::AppError::BillingProviderUnavailable(
                    "definite refusal".into(),
                ));
            }
            if let Some(barrier) = &self.wallet_barrier {
                self.wallet_entered.notify_one();
                barrier.wait().await;
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            }
            {
                let mut receipt = self.wallet_receipt.lock().unwrap();
                if self.enforce_code_uniqueness && receipt.is_some() {
                    return Err(crate::errors::AppError::BillingProviderUnavailable(
                        "provider code conflict".into(),
                    ));
                }
                *receipt = Some(LagoWallet {
                    id: format!("{customer_id}:wallet"),
                    balance_credits: crate::models::credits::Credits::from_whole(123),
                });
                self.accepted_wallets.fetch_add(1, Ordering::SeqCst);
            }
            if take_failure(&self.wallet_failures_remaining) {
                return Err(crate::errors::AppError::BillingProviderUnavailable(
                    "simulated Lago wallet failure".to_string(),
                ));
            }
            Ok(LagoWallet {
                id: format!("{customer_id}:wallet"),
                balance_credits: crate::models::credits::Credits::from_whole(123),
            })
        }

        async fn dispatch_wallet_create(
            &self,
            customer_id: &str,
            window: &super::super::lago_client::DispatchWindow,
        ) -> super::super::lago_client::ProviderDispatch<LagoWallet> {
            use super::super::lago_client::ProviderDispatch;
            if let Some(http) = &self.dispatch_http {
                return http.create(customer_id, window).await;
            }
            let refusing = self.wallet_refusals.load(Ordering::SeqCst) > 0;
            match self.ensure_wallet(customer_id).await {
                Ok(value) => ProviderDispatch::Applied(value),
                Err(error) if refusing => ProviderDispatch::Refused(error),
                Err(error)
                    if self.enforce_code_uniqueness
                        && self.wallet_receipt.lock().unwrap().is_some() =>
                {
                    ProviderDispatch::AlreadyExists(error)
                }
                Err(error) => ProviderDispatch::Ambiguous(error),
            }
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
            execution_permission_enrollment: None,
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
