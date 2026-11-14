use super::lago_client::LagoApi;
use crate::errors::{AppError, AppResult};
use crate::models::billing_wallet::{BillingWallet, COLLECTION_NAME as BILLING_WALLET};
use crate::models::credits::Credits;
use crate::models::usage_meter::COLLECTION_NAME as USAGE_METER;
use chrono::Utc;
use mongodb::bson::{self, Bson, doc};
use serde_json::Value;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LagoWebhookAction {
    WalletRefreshed,
    WalletRefreshDeferred,
    EntitlementInvalidated,
    Ignored,
}
impl LagoWebhookAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WalletRefreshed => "wallet_refreshed",
            Self::WalletRefreshDeferred => "wallet_refresh_deferred",
            Self::EntitlementInvalidated => "entitlement_invalidated",
            Self::Ignored => "ignored",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LagoWebhookOutcome {
    pub action: LagoWebhookAction,
    pub owner_id: Option<String>,
    pub customer_id: Option<String>,
    pub balance_credits: Option<Credits>,
    pub pending_debits_cleared: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalletRefreshOutcome {
    pub owner_id: String,
    pub balance_credits: Credits,
    pub pending_debits_cleared: bool,
}

pub async fn handle_lago_webhook_event(
    db: &mongodb::Database,
    lago: Option<&dyn LagoApi>,
    event_type: &str,
    payload: &Value,
) -> AppResult<LagoWebhookOutcome> {
    if event_type.starts_with("customer.")
        || event_type.starts_with("subscription.")
        || event_type.starts_with("plan.")
    {
        let profiles =
            db.collection::<bson::Document>(crate::models::billing_tax_profile::COLLECTION_NAME);
        if let Some(customer) = extract_external_customer_id(payload) {
            if let Some(wallet) = db
                .collection::<BillingWallet>(BILLING_WALLET)
                .find_one(doc! {"lago_customer_id": &customer})
                .await?
            {
                profiles
                    .update_many(
                        doc! {"owner_id": &wallet.owner_id},
                        doc! {"$set": {"fetched_at": bson::DateTime::from_millis(0)}},
                    )
                    .await?;
                if let Some(lago) = lago
                    && let Err(error) = super::taxes::refresh(db, lago, &wallet).await
                {
                    // Keep the invalidated profile stale and complete unrelated
                    // entitlement handling (notably terminated subscriptions).
                    tracing::warn!(owner_id = %wallet.owner_id, %error, "Webhook tax refresh will retry");
                }
            }
        } else if let Some(plan) = payload.pointer("/plan/code").and_then(Value::as_str) {
            super::taxes::invalidate_plan(db, plan).await?;
            // Admissions refresh their own profile inline; the bounded sweep
            // refreshes the remaining subscribers in order of staleness.
        }
    }
    if is_lago_wallet_event(event_type) {
        let Some(customer_id) = extract_external_customer_id(payload) else {
            return Ok(ignored());
        };
        let lago = lago.ok_or_else(|| {
            AppError::BillingProviderUnavailable("Lago client is not configured".to_string())
        })?;
        super::exact_migration::require_ready(db).await?;
        let Some(wallet) = db
            .collection::<BillingWallet>(BILLING_WALLET)
            .find_one_and_update(
                doc! { "lago_customer_id": &customer_id },
                doc! { "$inc": { "snapshot_generation": 1_i64 } },
            )
            .return_document(mongodb::options::ReturnDocument::After)
            .await?
        else {
            return Ok(ignored_for_customer(customer_id));
        };
        let invoice_dedupe_key = if event_type == "invoice.paid_credit_added" {
            extract_paid_credit_dedupe_key(payload)
        } else {
            None
        };
        let balance_credits = match provider_effective_balance(lago, &wallet).await {
            Ok(balance) => balance,
            Err(error) => {
                queue_refresh(db, &wallet.id, invoice_dedupe_key).await?;
                return Err(error);
            }
        };
        return Ok(
            match refresh_wallet_balance_with_receipt(
                db,
                &wallet,
                balance_credits,
                invoice_dedupe_key.clone(),
            )
            .await?
            {
                Some(outcome) => {
                    if event_type == "invoice.paid_credit_added" {
                        super::ledger::record_wallet_credited(
                            db,
                            &outcome.owner_id,
                            &customer_id,
                            outcome.balance_credits,
                            invoice_dedupe_key,
                        )
                        .await;
                    }
                    LagoWebhookOutcome {
                        action: LagoWebhookAction::WalletRefreshed,
                        owner_id: Some(outcome.owner_id),
                        customer_id: Some(customer_id),
                        balance_credits: Some(outcome.balance_credits),
                        pending_debits_cleared: outcome.pending_debits_cleared,
                    }
                }
                None => LagoWebhookOutcome {
                    action: LagoWebhookAction::WalletRefreshDeferred,
                    owner_id: Some(wallet.owner_id),
                    customer_id: Some(customer_id),
                    balance_credits: None,
                    pending_debits_cleared: false,
                },
            },
        );
    }
    if is_lago_entitlement_event(event_type) {
        let outcome = invalidate_entitlement_decision(db, payload).await?;
        return Ok(match outcome {
            Some((owner_id, customer_id)) => LagoWebhookOutcome {
                action: LagoWebhookAction::EntitlementInvalidated,
                owner_id: Some(owner_id),
                customer_id,
                balance_credits: None,
                pending_debits_cleared: false,
            },
            None => ignored(),
        });
    }
    Ok(ignored())
}

fn ignored() -> LagoWebhookOutcome {
    LagoWebhookOutcome {
        action: LagoWebhookAction::Ignored,
        owner_id: None,
        customer_id: None,
        balance_credits: None,
        pending_debits_cleared: false,
    }
}

fn ignored_for_customer(customer_id: String) -> LagoWebhookOutcome {
    LagoWebhookOutcome {
        customer_id: Some(customer_id),
        ..ignored()
    }
}

fn is_lago_wallet_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "wallet.created"
            | "wallet.updated"
            | "wallet.terminated"
            | "wallet.depleted_ongoing_balance"
            | "wallet_transaction.created"
            | "wallet_transaction.updated"
            | "wallet_transaction.payment_failure"
            | "invoice.paid_credit_added"
    )
}

fn is_lago_entitlement_event(event_type: &str) -> bool {
    matches!(
        event_type,
        "subscription.started"
            | "subscription.updated"
            | "subscription.terminated"
            | "subscription.trial_ended"
            | "plan.updated"
            | "plan.deleted"
            | "feature.updated"
            | "feature.deleted"
    )
}

/// `snapshot` must be loaded before the provider read, never after it.
pub async fn refresh_wallet_balance(
    db: &mongodb::Database,
    snapshot: &BillingWallet,
    balance_credits: Credits,
) -> AppResult<Option<WalletRefreshOutcome>> {
    refresh_wallet_balance_with_receipt(db, snapshot, balance_credits, None).await
}

async fn refresh_wallet_balance_with_receipt(
    db: &mongodb::Database,
    snapshot: &BillingWallet,
    balance_credits: Credits,
    invoice_dedupe_key: Option<String>,
) -> AppResult<Option<WalletRefreshOutcome>> {
    super::exact_migration::require_ready(db).await?;
    let outcome = apply_balance(db, snapshot, balance_credits, None).await?;
    if outcome.is_none() {
        queue_refresh(db, &snapshot.id, invoice_dedupe_key).await?;
    }
    Ok(outcome)
}

async fn queue_refresh(
    db: &mongodb::Database,
    wallet_id: &str,
    invoice_dedupe_key: Option<String>,
) -> AppResult<()> {
    let mut update = doc! { "$set": {
        "requested_at": bson::DateTime::from_chrono(Utc::now()),
        "token": uuid::Uuid::new_v4().to_string(),
    } };
    if let Some(key) = invoice_dedupe_key {
        update.insert("$addToSet", doc! { "invoice_dedupe_keys": key });
    }
    db.collection::<bson::Document>("billing_wallet_refresh_requests")
        .update_one(doc! { "_id": wallet_id }, update)
        .upsert(true)
        .await?;
    Ok(())
}

/// All provider snapshots use settled balance less accrued, uninvoiced usage.
pub async fn provider_effective_balance(
    lago: &dyn LagoApi,
    wallet: &BillingWallet,
) -> AppResult<Credits> {
    let balance = lago.wallet_balance_exact(&wallet.lago_customer_id).await?;
    let accrued = match wallet.lago_subscription_id.as_deref() {
        Some(subscription) => {
            let usage = lago
                .current_usage(&wallet.lago_customer_id, subscription)
                .await?;
            super::lago_client::extract_current_usage_credits(&usage.raw)?
        }
        None => Credits::ZERO,
    };
    Ok(balance.checked_sub(accrued)?)
}

/// Durable deferred refreshes run every second, independently of the ordinary
/// long reconcile interval. Delete only the token that was actually serviced.
pub fn spawn_refresh_worker(db: mongodb::Database, lago: Option<std::sync::Arc<dyn LagoApi>>) {
    let Some(lago) = lago else {
        return;
    };
    crate::services::shutdown::process().spawn_worker(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let Some(_work) = crate::services::shutdown::process()
                .tick(&mut interval)
                .await
            else {
                break;
            };
            if super::exact_migration::ready(&db).await.unwrap_or(false)
                && let Err(error) = service_refresh_requests(&db, lago.as_ref()).await
            {
                tracing::error!(%error, "deferred billing refresh failed");
            }
        }
    });
}
pub(super) async fn service_refresh_requests(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
) -> AppResult<()> {
    use futures::TryStreamExt;
    let requests = db.collection::<bson::Document>("billing_wallet_refresh_requests");
    let now = bson::DateTime::now();
    let rows: Vec<_> = requests
        .find(doc! {
            "$or": [
                { "lease_until": { "$exists": false } },
                { "lease_until": Bson::Null },
                { "lease_until": { "$lt": now } },
            ],
        })
        .sort(doc! { "requested_at": 1 })
        .limit(100)
        .await?
        .try_collect()
        .await?;
    for request in rows {
        let Some(_claim) = crate::services::shutdown::process().background() else {
            break;
        };
        let id = request
            .get_str("_id")
            .map_err(|e| AppError::Internal(e.to_string()))?;
        let lease_token = uuid::Uuid::new_v4().to_string();
        let claimed = requests
            .find_one_and_update(
                doc! {
                    "_id": id,
                    "token": request.get("token"),
                    "$or": [
                        { "lease_until": { "$exists": false } },
                        { "lease_until": Bson::Null },
                        { "lease_until": { "$lt": now } },
                    ],
                },
                doc! { "$set": {
                    "lease_token": &lease_token,
                    "lease_until": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::seconds(300)),
                } },
            )
            .with_options(mongodb::options::FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::After)
                .build())
            .await?;
        let Some(request) = claimed else {
            continue;
        };
        let Some(wallet) = db
            .collection::<BillingWallet>(BILLING_WALLET)
            .find_one(doc! { "_id": id })
            .await?
        else {
            requests
                .delete_one(doc! { "_id": id, "lease_token": &lease_token })
                .await?;
            continue;
        };
        let result = async {
            let balance = provider_effective_balance(lago, &wallet).await?;
            apply_balance(db, &wallet, balance, None).await
        }
        .await;
        if !matches!(result, Ok(Some(_))) {
            if let Err(error) = result {
                tracing::error!(wallet_id = id, %error, "deferred wallet refresh will retry");
            }
            // Rotate blocked requests so a busy/invalid wallet cannot starve
            // later top-ups beyond the bounded batch.
            requests
                .update_one(
                    doc! { "_id": id, "lease_token": &lease_token },
                    doc! {
                        "$set": {
                            "requested_at": bson::DateTime::from_chrono(Utc::now()),
                        }, "$unset": { "lease_token": "", "lease_until": "" },
                    },
                )
                .await?;
            continue;
        }
        if let Ok(Some(outcome)) = result
            && let Ok(dedupe_keys) = request.get_array("invoice_dedupe_keys")
        {
            for dedupe_key in dedupe_keys.iter().filter_map(Bson::as_str) {
                super::ledger::record_wallet_credited(
                    db,
                    &outcome.owner_id,
                    &wallet.lago_customer_id,
                    outcome.balance_credits,
                    Some(dedupe_key.to_owned()),
                )
                .await;
            }
        }
        let deleted = requests
            .delete_one(
                doc! { "_id": id, "token": request.get("token"), "lease_token": &lease_token },
            )
            .await?;
        if deleted.deleted_count == 0 {
            // A newer deferral must be eligible on the next pass. Release only
            // our lease, preserving any successor that claimed after expiry.
            requests
                .update_one(
                    doc! { "_id": id, "lease_token": &lease_token },
                    doc! { "$unset": { "lease_token": "", "lease_until": "" } },
                )
                .await?;
        }
    }
    Ok(())
}

/// Commit the provider snapshot, explicit adjustment and (if present) expiry
/// postings together. An active usage settlement retains the wallet lock until
/// its own journal is durable, so refresh cannot absorb an unjournaled debit.
pub(super) async fn apply_balance(
    db: &mongodb::Database,
    snapshot: &BillingWallet,
    balance: Credits,
    expiry: Option<&crate::models::billing_wallet::PurchasedCreditExpiryOperation>,
) -> AppResult<Option<WalletRefreshOutcome>> {
    for attempt in 0..16 {
        let mut session = db.client().start_session().await?;
        let database = db.clone();
        let starting_snapshot = snapshot.clone();
        let expiry_operation = expiry.cloned();
        let result = session
            .start_transaction()
            .write_concern(
                mongodb::options::WriteConcern::builder()
                    .w(mongodb::options::Acknowledgment::Majority)
                    .journal(true)
                    .build(),
            )
            .and_run2(async move |session| {
                let db = &database;
                let expiry = expiry_operation.as_ref();
                let operation =
                    apply_provider_snapshot(db, &starting_snapshot, balance, expiry, session).await;
                crate::services::api_key_mutation_service::transaction_result(operation)
            })
            .await;
        match result {
            Ok(value) => return Ok(value),
            Err(error) if super::ledger::is_duplicate_key_error(&error) && attempt < 15 => continue,
            Err(error) => {
                return Err(
                    crate::services::api_key_mutation_service::map_transaction_error(error),
                );
            }
        }
    }
    Err(AppError::Internal("wallet refresh contention".into()))
}

async fn invalidate_entitlement_decision(
    db: &mongodb::Database,
    payload: &Value,
) -> AppResult<Option<(String, Option<String>)>> {
    let customer_id = extract_external_customer_id(payload);
    let subscription_id = extract_external_subscription_id(payload);
    let mut filters = Vec::new();
    if let Some(customer_id) = customer_id.as_deref() {
        filters.push(doc! { "lago_customer_id": customer_id });
    }
    if let Some(subscription_id) = subscription_id.as_deref() {
        filters.push(doc! { "lago_subscription_id": subscription_id });
    }
    if filters.is_empty() {
        return Ok(None);
    }
    let filter = if filters.len() == 1 {
        filters.remove(0)
    } else {
        doc! { "$or": filters }
    };
    let now = Utc::now();
    let updated = db
        .collection::<BillingWallet>(BILLING_WALLET)
        .find_one_and_update(
            filter,
            doc! { "$set": { "updated_at": bson::DateTime::from_chrono(now) } },
        )
        .await?;
    Ok(updated.map(|wallet| (wallet.owner_id, customer_id)))
}

/// At-most-once key for `invoice.paid_credit_added` ledger entries: the
/// Lago invoice id, stable across Lago's webhook delivery retries.
fn extract_paid_credit_dedupe_key(payload: &Value) -> Option<String> {
    payload
        .get("invoice")
        .and_then(|invoice| invoice.get("lago_id"))
        .or_else(|| payload.get("lago_id"))
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| format!("lago-invoice:{value}"))
}

fn extract_external_customer_id(value: &Value) -> Option<String> {
    find_string_by_keys(
        value,
        &[
            "external_customer_id",
            "customer_external_id",
            "externalCustomerId",
        ],
    )
    .or_else(|| find_nested_external_id(value, "customer"))
}

fn extract_external_subscription_id(value: &Value) -> Option<String> {
    find_string_by_keys(
        value,
        &[
            "external_subscription_id",
            "subscription_external_id",
            "externalSubscriptionId",
        ],
    )
    .or_else(|| find_nested_external_id(value, "subscription"))
}

fn find_nested_external_id(value: &Value, object_key: &str) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(object) = map.get(object_key)
                && let Some(found) = find_string_by_keys(object, &["external_id", "externalId"])
            {
                return Some(found);
            }
            map.values()
                .find_map(|inner| find_nested_external_id(inner, object_key))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|inner| find_nested_external_id(inner, object_key)),
        _ => None,
    }
}

fn find_string_by_keys(value: &Value, keys: &[&str]) -> Option<String> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(found) = map
                    .get(*key)
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                {
                    return Some(found.to_string());
                }
            }
            map.values()
                .find_map(|inner| find_string_by_keys(inner, keys))
        }
        Value::Array(items) => items
            .iter()
            .find_map(|inner| find_string_by_keys(inner, keys)),
        _ => None,
    }
}

async fn apply_provider_snapshot(
    db: &mongodb::Database,
    snapshot: &BillingWallet,
    balance: Credits,
    expiry: Option<&crate::models::billing_wallet::PurchasedCreditExpiryOperation>,
    session: &mut mongodb::ClientSession,
) -> AppResult<Option<WalletRefreshOutcome>> {
    let wallet_id = &snapshot.id;
    let wallets = db.collection::<BillingWallet>(BILLING_WALLET);
    let Some(wallet) = wallets
        .find_one(doc! { "_id": wallet_id, "active_settlement": bson::Bson::Null })
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    // Reservation/settlement counters may advance while Lago is read. Use the
    // current pending amount below; only provider-visible acknowledgements and
    // balance changes invalidate the read. Millisecond comparison matches BSON.
    if wallet.snapshot_generation != snapshot.snapshot_generation
        || wallet.balance_credits != snapshot.balance_credits
        || wallet.balance_synced_at.timestamp_millis()
            != snapshot.balance_synced_at.timestamp_millis()
        || wallet.lago_customer_id != snapshot.lago_customer_id
        || wallet.lago_subscription_id != snapshot.lago_subscription_id
        || wallet.pending_lago_debits < snapshot.pending_lago_debits
    {
        return Ok(None);
    }
    if let Some(expiry) = expiry {
        if !wallet.active_topup_expiry.as_ref().is_some_and(|current| {
            current.operation_id == expiry.operation_id
                && current.processing_token == expiry.processing_token
                && !current.wallet_balance_applied
        }) {
            return Ok(None);
        }
    } else if wallet.active_topup_expiry.is_some() {
        return Ok(None);
    }
    let clear = db
        .collection::<bson::Document>(USAGE_METER)
        .count_documents(doc! {
            "billing_owner_id": &wallet.owner_id,
            "status": "finalized",
            "wallet_id": { "$ne": null },
            "lago_acked": false,
        })
        .session(&mut *session)
        .await?
        == 0;
    let pending = if clear {
        Credits::ZERO
    } else {
        wallet.pending_lago_debits
    };
    let before = wallet
        .balance_credits
        .checked_sub(wallet.pending_lago_debits)?;
    let after = balance.checked_sub(pending)?;
    let expired = expiry.map_or(Credits::ZERO, |op| op.amount);
    let adjustment = after.checked_sub(before)?.checked_add(expired)?;
    if let Some(op) = expiry {
        let void_id = op
            .lago_void_transaction_id
            .as_deref()
            .ok_or_else(|| AppError::Internal("expiry has no provider receipt".into()))?;
        for item in &op.items {
            let mut entry = super::ledger::exact_entry(
                &wallet.owner_id,
                &item.reference_id,
                "topup_expired",
                format!("topup-expired:{}:{void_id}", item.reference_id),
                super::ledger::transfer(
                    "writeoff:topup_expired".into(),
                    format!("wallet:{}", wallet.owner_id),
                    item.amount,
                ),
                None,
            );
            entry.transaction_id = Some(void_id.into());
            entry.wallet_id = wallet.lago_wallet_id.clone();
            super::ledger::append_in_session(db, &mut *session, entry).await?;
        }
    }
    if adjustment != Credits::ZERO {
        let account = format!("wallet:{}", wallet.owner_id);
        let postings = if adjustment > Credits::ZERO {
            super::ledger::transfer(account, "external:lago".into(), adjustment)
        } else {
            super::ledger::transfer("external:lago".into(), account, -adjustment)
        };
        super::ledger::append_in_session(
            db,
            &mut *session,
            super::ledger::exact_entry(
                &wallet.owner_id,
                &wallet.id,
                "wallet_adjusted",
                format!("wallet-adjusted:{}", uuid::Uuid::new_v4()),
                postings,
                None,
            ),
        )
        .await?;
    }
    let now = bson::DateTime::from_chrono(Utc::now());
    let mut set = doc! {
        "balance_credits": balance,
        "pending_lago_debits": pending,
        "balance_synced_at": now,
        "updated_at": now,
    };
    if expiry.is_some() {
        set.insert("pending_topup_expiry_credits", Credits::ZERO);
        set.insert("active_topup_expiry.wallet_balance_applied", true);
        set.insert("active_topup_expiry.updated_at", now);
    }
    wallets
        .update_one(
            doc! { "_id": wallet_id },
            doc! { "$set": set, "$inc": { "snapshot_generation": 1_i64 } },
        )
        .session(&mut *session)
        .await?;
    Ok(Some(WalletRefreshOutcome {
        owner_id: wallet.owner_id,
        balance_credits: balance,
        pending_debits_cleared: clear && wallet.pending_lago_debits != Credits::ZERO,
    }))
}

#[cfg(test)]
pub(crate) async fn refresh_test_balance(
    db: &mongodb::Database,
    customer: &str,
    balance: Credits,
) -> AppResult<Option<WalletRefreshOutcome>> {
    let wallet = db
        .collection::<BillingWallet>(BILLING_WALLET)
        .find_one(doc! {"lago_customer_id": customer})
        .await?
        .ok_or_else(|| AppError::Internal("test wallet missing".into()))?;
    refresh_wallet_balance(db, &wallet, balance).await
}

#[cfg(test)]
mod tests {
    use super::{
        LagoApi, LagoWebhookAction, extract_external_customer_id, extract_external_subscription_id,
        handle_lago_webhook_event,
    };
    use crate::models::billing_wallet::{BillingWallet, CollectionState, PlanKind};
    use crate::models::service_billing::BillingMetric;
    use crate::models::usage_meter::{BillingLayer, CredentialClass, UsageMeterRow, UsageStatus};
    use crate::services::billing::lago_client::{
        Entitlement, LagoAck, LagoError, LagoEvent, LagoUsage, OwnerProvisionInput,
    };
    use crate::test_utils::connect_test_database;
    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use mongodb::bson::doc;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::Notify;
    use uuid::Uuid;
    #[derive(Default)]
    struct BalanceReadBarrier {
        started: Notify,
        resume: Notify,
    }
    #[derive(Clone, Default)]
    struct BalanceLago {
        balance_credits: i64,
        accrued_cents: Option<serde_json::Value>,
        balance_read_barrier: Option<Arc<BalanceReadBarrier>>,
        balance_reads: Option<Arc<std::sync::atomic::AtomicUsize>>,
        usage_reads: Option<Arc<std::sync::atomic::AtomicUsize>>,
        balance_error: bool,
    }
    #[async_trait]
    impl LagoApi for BalanceLago {
        async fn ensure_customer(
            &self,
            owner: &OwnerProvisionInput,
        ) -> crate::errors::AppResult<String> {
            Ok(owner.external_customer_id.clone())
        }
        async fn ensure_subscription(
            &self,
            customer_id: &str,
            _plan_code: &str,
        ) -> crate::errors::AppResult<String> {
            Ok(customer_id.to_string())
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
            if let Some(reads) = &self.usage_reads {
                reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            Ok(LagoUsage {
                customer_id: customer_id.to_string(),
                subscription_id: subscription_id.to_string(),
                raw: self
                    .accrued_cents
                    .as_ref()
                    .map(|v| {
                        json!({
                        "amount_cents":v}
                        )
                    })
                    .unwrap_or_else(|| json!({})),
            })
        }
        async fn wallet_balance(&self, _customer_id: &str) -> crate::errors::AppResult<i64> {
            if let Some(reads) = &self.balance_reads {
                reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }
            if let Some(barrier) = &self.balance_read_barrier {
                barrier.started.notify_one();
                barrier.resume.notified().await;
            }
            if self.balance_error {
                return Err(crate::errors::AppError::BillingProviderUnavailable(
                    "test provider read failed".into(),
                ));
            }
            Ok(self.balance_credits)
        }
        async fn entitlements(
            &self,
            _subscription_id: &str,
        ) -> crate::errors::AppResult<Vec<Entitlement>> {
            Ok(Vec::new())
        }
    }
    fn wallet(owner_id: &str, pending_lago_debits: i64) -> BillingWallet {
        let now = Utc::now();
        BillingWallet {
            snapshot_generation: 0,
            id: Uuid::new_v4().to_string(),
            owner_id: owner_id.to_string(),
            lago_customer_id: owner_id.to_string(),
            lago_wallet_id: Some(format!("{owner_id}:wallet")),
            lago_subscription_id: Some(format!("{owner_id}:starter")),
            plan_kind: PlanKind::Prepaid,
            balance_credits: crate::models::credits::Credits::from_whole(10),
            reserved_credits: crate::models::credits::Credits::from_whole(0),
            pending_lago_debits: crate::models::credits::Credits::from_whole(pending_lago_debits),
            pending_topup_expiry_credits: crate::models::credits::Credits::from_whole(0),
            has_payment_instrument: false,
            overdraft_cap_credits: crate::models::credits::Credits::from_whole(0),
            suspended: false,
            collection_state: CollectionState::Good,
            topup_expiry_checked_at: None,
            active_topup_expiry: None,
            balance_synced_at: now - Duration::minutes(10),
            created_at: now,
            updated_at: now,
        }
    }
    fn usage_row(owner_id: &str, wallet_id: &str, lago_acked: bool) -> UsageMeterRow {
        let now = Utc::now();
        UsageMeterRow {
            model_tariff: None,
            org_channel_guest: None,
            rollup_pending: true,
            id: Uuid::new_v4().to_string(),
            transaction_id: Uuid::new_v4().to_string(),
            billing_request_id: Uuid::new_v4().to_string(),
            layer: BillingLayer::Platform,
            flush_seq: None,
            billing_owner_id: owner_id.to_string(),
            wallet_id: Some(wallet_id.to_string()),
            actor_user_id: owner_id.to_string(),
            api_key_id: None,
            assistant_source: None,
            user_service_id: None,
            service_id: Some("svc-1".to_string()),
            service_slug: Some("svc".to_string()),
            metric: BillingMetric::Requests,
            lago_metric_code: "platform_requests".to_string(),
            operation: None,
            credential_class: CredentialClass::UserOwned,
            model: None,
            reported_model: None,
            token_breakdown: None,
            audio_tokens: None,
            image_tokens: None,
            reserved_credits: crate::models::credits::Credits::from_whole(1),
            funding: None,
            quantity: Some(1),
            pending_resale_quantity: None,
            pending_platform_usage: None,
            pool_attempt: None,
            machine_usage: None,
            status: UsageStatus::Finalized,
            forwarded: true,
            released: true,
            lago_acked,
            attempt: 0,
            settlement_attempts: 0,
            settlement_next_retry_at: None,
            created_at: now,
            updated_at: now,
            finalized_at: Some(now),
            expires_at: None,
            last_error: None,
        }
    }
    #[test]
    fn extracts_customer_and_subscription_from_lago_payloads() {
        let payload = json!({
                    "webhook_type": "subscription.started",
                    "subscription": {
                        "external_id": "owner-1:starter",
                        "customer": {
         "external_id": "owner-1" }
                    }
                }
        );
        assert_eq!(
            extract_external_customer_id(&payload).as_deref(),
            Some("owner-1")
        );
        assert_eq!(
            extract_external_subscription_id(&payload).as_deref(),
            Some("owner-1:starter")
        );
    }
    #[tokio::test]
    async fn wallet_webhook_refreshes_balance_and_clears_accounted_pending_debits() {
        let Some(db) = connect_test_database("lago_webhook_wallet_clear").await else {
            return;
        };
        let owner_id = "owner-webhook-clear";
        db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .insert_one(wallet(owner_id, 7))
            .await
            .expect("insert wallet");
        let outcome = handle_lago_webhook_event(
            &db,
            Some(&BalanceLago {
                balance_credits: 25,
                accrued_cents: None,
                ..Default::default()
            }),
            "wallet.updated",
            &json!({
             "webhook_type": "wallet.updated",
             "wallet": {
             "external_customer_id": owner_id }
             }
            ),
        )
        .await
        .expect("handle webhook");
        let saved = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": owner_id })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        assert_eq!(outcome.action, LagoWebhookAction::WalletRefreshed);
        assert_eq!(
            saved.balance_credits,
            crate::models::credits::Credits::from_whole(25)
        );
        assert_eq!(
            saved.pending_lago_debits,
            crate::models::credits::Credits::from_whole(0)
        );
        assert!(outcome.pending_debits_cleared);
    }
    #[tokio::test]
    async fn wallet_webhook_keeps_pending_debits_when_unacked_usage_exists() {
        let Some(db) = connect_test_database("lago_webhook_wallet_unacked").await else {
            return;
        };
        let owner_id = "owner-webhook-unacked";
        let wallet = wallet(owner_id, 7);
        let wallet_id = wallet.id.clone();
        db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .insert_one(&wallet)
            .await
            .expect("insert wallet");
        db.collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .insert_one(usage_row(owner_id, &wallet_id, false))
            .await
            .expect("insert usage row");
        let outcome = handle_lago_webhook_event(
            &db,
            Some(&BalanceLago {
                balance_credits: 25,
                accrued_cents: None,
                ..Default::default()
            }),
            "wallet_transaction.created",
            &json!({
             "webhook_type": "wallet_transaction.created",
             "wallet_transaction": {
             "wallet": {
             "external_customer_id": owner_id }
             }
             }
            ),
        )
        .await
        .expect("handle webhook");
        let saved = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": owner_id })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        assert_eq!(outcome.action, LagoWebhookAction::WalletRefreshed);
        assert_eq!(
            saved.balance_credits,
            crate::models::credits::Credits::from_whole(25)
        );
        assert_eq!(
            saved.pending_lago_debits,
            crate::models::credits::Credits::from_whole(7)
        );
        assert!(!outcome.pending_debits_cleared);
    }
    #[tokio::test]
    async fn subscription_webhook_invalidates_matching_wallet_marker() {
        let Some(db) = connect_test_database("lago_webhook_subscription").await else {
            return;
        };
        let owner_id = "owner-webhook-subscription";
        db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .insert_one(wallet(owner_id, 0))
            .await
            .expect("insert wallet");
        let outcome = handle_lago_webhook_event(
            &db,
            None,
            "subscription.started",
            &json!({
                            "webhook_type": "subscription.started",
                            "subscription": {
                                "external_id": format!("{owner_id}:starter"),
                                "customer": {
             "external_id": owner_id }
                            }
                        }
            ),
        )
        .await
        .expect("handle webhook");
        assert_eq!(outcome.action, LagoWebhookAction::EntitlementInvalidated);
        assert_eq!(outcome.owner_id.as_deref(), Some(owner_id));
    }
    #[tokio::test]
    async fn deferred_topup_refresh_uses_effective_balance_without_flip_flop() {
        use crate::models::credits::Credits;
        use mongodb::bson::Document;
        let db = connect_test_database("webhook_deferred_effective")
            .await
            .unwrap();
        let wallet = wallet("owner", 0);
        db.collection::<BillingWallet>("billing_wallet")
            .insert_one(&wallet)
            .await
            .unwrap();
        crate::services::billing::exact_migration::migrate_one(&db, "billing_wallet", &wallet.id)
            .await
            .unwrap();
        db.collection::<Document>("billing_wallet")
            .update_one(
                doc! { "_id": &wallet.id },
                doc! { "$set": { "active_settlement": { "row_id": "inflight" } } },
            )
            .await
            .unwrap();
        let lago = BalanceLago {
            balance_credits: 25,
            accrued_cents: Some(json!(500)),
            ..Default::default()
        };
        let outcome = handle_lago_webhook_event(
            &db,
            Some(&lago),
            "invoice.paid_credit_added",
            &json!({
                "invoice": { "lago_id": "invoice-deferred" },
                "wallet": { "external_customer_id": "owner" }
            }),
        )
        .await
        .unwrap();
        assert_eq!(outcome.action, LagoWebhookAction::WalletRefreshDeferred);
        assert_eq!(
            db.collection::<Document>("billing_wallet_refresh_requests")
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        db.collection::<Document>("billing_wallet")
            .update_one(
                doc! { "_id": &wallet.id },
                doc! { "$unset": { "active_settlement": "" } },
            )
            .await
            .unwrap();
        super::service_refresh_requests(&db, &lago).await.unwrap();
        assert_eq!(
            db.collection::<Document>("billing_wallet_refresh_requests")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let saved = db
            .collection::<BillingWallet>("billing_wallet")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.balance_credits, Credits::from_whole(20));
        let count = db
            .collection::<Document>("billing_ledger")
            .count_documents(doc! { "movement": "wallet_adjusted" })
            .await
            .unwrap();
        assert_eq!(count, 1);
        let effective = super::provider_effective_balance(&lago, &saved)
            .await
            .unwrap();
        super::refresh_wallet_balance(&db, &saved, effective)
            .await
            .unwrap();
        assert_eq!(
            db.collection::<Document>("billing_ledger")
                .count_documents(doc! { "movement": "wallet_adjusted" })
                .await
                .unwrap(),
            count
        );
        assert_eq!(
            db.collection::<Document>("billing_ledger")
                .count_documents(doc! { "movement": "wallet_credited", "dedupe_key": "lago-invoice:invoice-deferred" })
                .await
                .unwrap(),
            1
        );
        let invalid = BalanceLago {
            balance_credits: 25,
            accrued_cents: Some(json!("invalid")),
            ..Default::default()
        };
        assert!(
            super::provider_effective_balance(&invalid, &saved)
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn snapshot_order_wallet_late_provider_read_is_dropped() {
        use crate::models::credits::Credits;
        use mongodb::bson::Document;
        for (old, new) in [(25, 40), (100, 60)] {
            let db =
                crate::test_utils::connect_transaction_test_database("snapshot_wallet_race").await;
            let wallet = wallet("owner", 0);
            db.collection::<BillingWallet>("billing_wallet")
                .insert_one(&wallet)
                .await
                .unwrap();
            crate::services::billing::exact_migration::migrate_one(
                &db,
                "billing_wallet",
                &wallet.id,
            )
            .await
            .unwrap();
            let barrier = Arc::new(BalanceReadBarrier::default());
            let old_lago = BalanceLago {
                balance_credits: old,
                balance_read_barrier: Some(barrier.clone()),
                ..Default::default()
            };
            let new_lago = BalanceLago {
                balance_credits: new,
                ..Default::default()
            };
            let payload = json!({"wallet":{"external_customer_id":"owner"}});
            let (old_result, ()) = tokio::join!(
                handle_lago_webhook_event(&db, Some(&old_lago), "wallet.updated", &payload),
                async {
                    barrier.started.notified().await;
                    let newer = handle_lago_webhook_event(
                        &db,
                        Some(&new_lago),
                        "invoice.paid_credit_added",
                        &payload,
                    )
                    .await
                    .unwrap();
                    assert_eq!(newer.action, LagoWebhookAction::WalletRefreshed);
                    barrier.resume.notify_one();
                }
            );
            let saved = db
                .collection::<BillingWallet>("billing_wallet")
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                saved.balance_credits,
                Credits::from_whole(new),
                "late provider snapshot overwrote newer balance"
            );
            assert_eq!(
                old_result.unwrap().action,
                LagoWebhookAction::WalletRefreshDeferred
            );
            assert_eq!(
                db.collection::<Document>("billing_ledger")
                    .count_documents(doc! {"movement":"wallet_adjusted"})
                    .await
                    .unwrap(),
                1
            );
            assert_eq!(
                db.collection::<Document>("billing_wallet_refresh_requests")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                1
            );
            super::super::account_reconciliation::run_once(&db)
                .await
                .unwrap();
            let status = db
                .collection::<crate::models::chain_verify_status::ChainVerifyStatus>(
                    "chain_verify_status",
                )
                .find_one(doc! {"_id":"billing_accounts"})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                status.outcome,
                crate::models::chain_verify_status::ChainVerifyOutcome::Ok
            );
            db.drop().await.unwrap();
        }
    }

    async fn snapshot_expiry_operation(
        db: &mongodb::Database,
        wallet: &BillingWallet,
    ) -> crate::models::billing_wallet::PurchasedCreditExpiryOperation {
        let now = Utc::now();
        let operation = crate::models::billing_wallet::PurchasedCreditExpiryOperation {
            operation_id: "expiry".into(),
            processing_token: "worker".into(),
            lease_until: now + Duration::minutes(10),
            amount: crate::models::credits::Credits::from_whole(1),
            items: vec![crate::models::billing_wallet::PurchasedCreditExpiryItem {
                lago_purchase_transaction_id: "purchase".into(),
                reference_id: "purchase".into(),
                amount: crate::models::credits::Credits::from_whole(1),
                settled_at: now - Duration::days(366),
            }],
            dispatch_attempts: 0,
            processing_lease_until: None,
            processing_lease_token: None,
            dispatch_started_at: None,
            unresolved_reported_at: None,
            lago_void_transaction_id: Some("void".into()),
            wallet_balance_applied: false,
            history_applied: true,
            created_at: now,
            updated_at: now,
        };
        db.collection::<mongodb::bson::Document>("billing_wallet")
            .update_one(
                doc! { "_id": &wallet.id },
                doc! { "$set": {
                    "active_topup_expiry": mongodb::bson::to_bson(&operation).unwrap(),
                    "pending_topup_expiry_credits": operation.amount,
                } },
            )
            .await
            .unwrap();
        operation
    }

    async fn snapshot_entry(
        db: &mongodb::Database,
        wallet: &BillingWallet,
        lago: &BalanceLago,
        entry: &str,
    ) {
        match entry {
            "reconcile" => {
                let runner = super::super::reconcile::BillingReconciler::new(
                    db.clone(),
                    None,
                    Arc::new(crate::test_utils::test_app_config()),
                );
                runner
                    .refresh_wallet_balances(
                        lago,
                        &mut super::super::reconcile::ReconcileStats::default(),
                    )
                    .await
                    .unwrap();
            }
            "deferred" => {
                super::queue_refresh(db, &wallet.id, None).await.unwrap();
                // A second worker may only take over after the first lease expires.
                db.collection::<mongodb::bson::Document>("billing_wallet_refresh_requests")
                    .update_one(
                        doc! {"_id":&wallet.id},
                        doc! {"$set":{"lease_until":mongodb::bson::DateTime::from_millis(0)}},
                    )
                    .await
                    .unwrap();
                super::service_refresh_requests(db, lago).await.unwrap();
            }
            "webhook" | "paid_credit" => {
                let event = if entry == "paid_credit" {
                    "invoice.paid_credit_added"
                } else {
                    "wallet.updated"
                };
                let payload = json!({
                    "wallet": {"external_customer_id": wallet.lago_customer_id},
                    "invoice": {"lago_id": "snapshot-paid"},
                });
                handle_lago_webhook_event(db, Some(lago), event, &payload)
                    .await
                    .unwrap();
            }
            "expiry" => {
                let now = Utc::now();
                let operation = snapshot_expiry_operation(db, wallet).await;
                assert_eq!(
                    super::super::topup_expiry::complete_expiry_operation(
                        db, lago, wallet, operation, now
                    )
                    .await
                    .unwrap(),
                    1
                );
            }
            _ => panic!("unknown test entry"),
        }
    }

    async fn assert_snapshot_accounts(db: &mongodb::Database) {
        super::super::account_reconciliation::run_once(db)
            .await
            .unwrap();
        let status = db
            .collection::<crate::models::chain_verify_status::ChainVerifyStatus>(
                "chain_verify_status",
            )
            .find_one(doc! {"_id":"billing_accounts"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            status.outcome,
            crate::models::chain_verify_status::ChainVerifyOutcome::Ok,
            "{:?}",
            status.break_detail
        );
    }

    async fn assert_refresh_queue_converges(
        db: &mongodb::Database,
        balance: i64,
        expected_fetches: usize,
    ) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let reads = Arc::new(AtomicUsize::new(0));
        let lago = BalanceLago {
            balance_credits: balance,
            balance_reads: Some(reads.clone()),
            ..Default::default()
        };
        for _ in 0..3 {
            super::service_refresh_requests(db, &lago).await.unwrap();
            assert_eq!(
                reads.load(Ordering::SeqCst),
                expected_fetches,
                "an empty queue must not fetch again"
            );
            assert_eq!(
                db.collection::<mongodb::bson::Document>("billing_wallet_refresh_requests")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
        }
        let saved = db
            .collection::<BillingWallet>("billing_wallet")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.balance_credits,
            crate::models::credits::Credits::from_whole(balance)
        );
        assert_snapshot_accounts(db).await;
    }

    #[tokio::test]
    async fn snapshot_order_wallet_webhook_burst_publishes_last_and_drains_once() {
        use crate::models::credits::Credits;
        use mongodb::bson::Document;
        const BURST: usize = 8;
        for last_finishes_first in [false, true] {
            let db =
                crate::test_utils::connect_transaction_test_database("wallet_webhook_burst").await;
            let wallet = wallet("owner", 0);
            db.collection::<BillingWallet>("billing_wallet")
                .insert_one(&wallet)
                .await
                .unwrap();
            super::super::exact_migration::migrate_one(&db, "billing_wallet", &wallet.id)
                .await
                .unwrap();
            let mut tasks = Vec::new();
            for n in 0..BURST {
                let barrier = Arc::new(BalanceReadBarrier::default());
                let lago = BalanceLago {
                    balance_credits: 20 + n as i64,
                    balance_read_barrier: Some(barrier.clone()),
                    ..Default::default()
                };
                let database = db.clone();
                let payload = json!({"wallet":{"external_customer_id":wallet.lago_customer_id}});
                let task = tokio::spawn(async move {
                    handle_lago_webhook_event(&database, Some(&lago), "wallet.updated", &payload)
                        .await
                        .unwrap()
                });
                barrier.started.notified().await;
                tasks.push((barrier, task));
            }
            // All generations were captured before any response may publish.
            // Exercise both the last read finishing first and finishing last.
            let (last_barrier, last_task) = tasks.pop().unwrap();
            let mut last_task = Some(last_task);
            if last_finishes_first {
                last_barrier.resume.notify_one();
                assert_eq!(
                    last_task.take().unwrap().await.unwrap().action,
                    LagoWebhookAction::WalletRefreshed
                );
            }
            for (barrier, _) in &tasks {
                barrier.resume.notify_one();
            }
            for (_, task) in tasks {
                assert_eq!(
                    task.await.unwrap().action,
                    LagoWebhookAction::WalletRefreshDeferred
                );
                assert_eq!(
                    db.collection::<Document>("billing_wallet_refresh_requests")
                        .count_documents(doc! {})
                        .await
                        .unwrap(),
                    1
                );
            }
            if let Some(last_task) = last_task {
                last_barrier.resume.notify_one();
                assert_eq!(
                    last_task.await.unwrap().action,
                    LagoWebhookAction::WalletRefreshed
                );
            }
            let final_balance = 20 + BURST as i64 - 1;
            let saved = db
                .collection::<BillingWallet>("billing_wallet")
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(saved.balance_credits, Credits::from_whole(final_balance));
            assert_eq!(saved.snapshot_generation, BURST as i64 + 1);
            assert_eq!(
                db.collection::<Document>("billing_ledger")
                    .count_documents(doc! {"movement":"wallet_adjusted"})
                    .await
                    .unwrap(),
                1
            );
            assert_refresh_queue_converges(&db, final_balance, 1).await;
            assert_eq!(
                db.collection::<Document>("billing_ledger")
                    .count_documents(doc! {"movement":"wallet_adjusted"})
                    .await
                    .unwrap(),
                1
            );
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn snapshot_order_wallet_whole_replacement_dropping_generation_still_rejects() {
        use crate::models::credits::Credits;
        use mongodb::bson::{DateTime, Document};
        // A legacy replacement after publication drops the additive generation.
        // At zero the balance, timestamp or pending-debit decrease detects it; at seven the reset
        // itself suffices even when the observable balance/time are unchanged.
        for (generation, pending, new_balance, change_timestamp) in [
            (0, 0, 40, false),
            (0, 0, 10, true),
            (0, 7, 10, false),
            (7, 0, 10, false),
        ] {
            let db =
                crate::test_utils::connect_transaction_test_database("wallet_generation_reset")
                    .await;
            let mut wallet = wallet("owner", pending);
            wallet.snapshot_generation = generation;
            db.collection::<BillingWallet>("billing_wallet")
                .insert_one(&wallet)
                .await
                .unwrap();
            super::super::exact_migration::migrate_one(&db, "billing_wallet", &wallet.id)
                .await
                .unwrap();
            let barrier = Arc::new(BalanceReadBarrier::default());
            let old = BalanceLago {
                balance_credits: 25,
                balance_read_barrier: Some(barrier.clone()),
                ..Default::default()
            };
            tokio::join!(snapshot_entry(&db, &wallet, &old, "reconcile"), async {
                barrier.started.notified().await;
                let snapshot = db
                    .collection::<BillingWallet>("billing_wallet")
                    .find_one(doc! {})
                    .await
                    .unwrap()
                    .unwrap();
                assert!(
                    super::refresh_wallet_balance(&db, &snapshot, Credits::from_whole(new_balance))
                        .await
                        .unwrap()
                        .is_some()
                );
                let mut legacy = db
                    .collection::<Document>("billing_wallet")
                    .find_one(doc! {})
                    .await
                    .unwrap()
                    .unwrap();
                legacy.remove("snapshot_generation");
                // Preserve unrelated raw migration metadata while simulating
                // an old whole-document writer omitting the unknown field.
                let synced = if change_timestamp {
                    wallet.balance_synced_at + Duration::seconds(1)
                } else {
                    wallet.balance_synced_at
                };
                legacy.insert("balance_synced_at", DateTime::from_chrono(synced));
                db.collection::<Document>("billing_wallet")
                    .replace_one(doc! {"_id":&wallet.id}, legacy)
                    .await
                    .unwrap();
                barrier.resume.notify_one();
            });
            let saved = db
                .collection::<BillingWallet>("billing_wallet")
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(saved.snapshot_generation, 0);
            assert_eq!(saved.balance_credits, Credits::from_whole(new_balance));
            assert_eq!(
                db.collection::<Document>("billing_ledger")
                    .count_documents(doc! {"movement":"wallet_adjusted"})
                    .await
                    .unwrap(),
                u64::from(new_balance != 10 || pending != 0)
            );
            assert_eq!(
                db.collection::<Document>("billing_wallet_refresh_requests")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                1
            );
            assert_refresh_queue_converges(&db, new_balance, 1).await;
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn snapshot_order_wallet_entry_point_matrix_and_expired_queue_lease() {
        use crate::models::credits::Credits;
        use mongodb::bson::Document;
        for (old_entry, new_entry) in [
            ("reconcile", "deferred"),
            ("deferred", "reconcile"),
            ("deferred", "webhook"),
            ("webhook", "deferred"),
            ("reconcile", "expiry"),
            ("deferred", "expiry"),
            ("reconcile", "paid_credit"),
            ("deferred", "paid_credit"),
            ("deferred", "deferred"),
        ] {
            for (old, new) in [(25, 40), (100, 60)] {
                let db =
                    crate::test_utils::connect_transaction_test_database("snapshot_wallet_matrix")
                        .await;
                let wallet = wallet("owner", 0);
                db.collection::<BillingWallet>("billing_wallet")
                    .insert_one(&wallet)
                    .await
                    .unwrap();
                super::super::exact_migration::migrate_one(&db, "billing_wallet", &wallet.id)
                    .await
                    .unwrap();
                let barrier = Arc::new(BalanceReadBarrier::default());
                let old_lago = BalanceLago {
                    balance_credits: old,
                    balance_read_barrier: Some(barrier.clone()),
                    ..Default::default()
                };
                let new_lago = BalanceLago {
                    balance_credits: new,
                    ..Default::default()
                };
                let mut request_after_winner = None;
                let ((), ()) =
                    tokio::join!(snapshot_entry(&db, &wallet, &old_lago, old_entry), async {
                        barrier.started.notified().await;
                        snapshot_entry(&db, &wallet, &new_lago, new_entry).await;
                        assert_snapshot_accounts(&db).await;
                        request_after_winner = db
                            .collection::<Document>("billing_wallet_refresh_requests")
                            .find_one(doc! {})
                            .await
                            .unwrap();
                        barrier.resume.notify_one();
                    });
                let saved = db
                    .collection::<BillingWallet>("billing_wallet")
                    .find_one(doc! {})
                    .await
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    saved.balance_credits,
                    Credits::from_whole(new),
                    "{old_entry} -> {new_entry}, {old} -> {new}"
                );
                assert_eq!(
                    db.collection::<Document>("billing_ledger")
                        .count_documents(doc! {"movement":"wallet_adjusted"})
                        .await
                        .unwrap(),
                    1
                );
                let already_serviced = old_entry == "deferred" && new_entry == "deferred";
                let expected_fetches = usize::from(!already_serviced);
                assert_eq!(
                    db.collection::<Document>("billing_wallet_refresh_requests")
                        .count_documents(doc! {})
                        .await
                        .unwrap(),
                    expected_fetches as u64
                );
                if old_entry == "deferred" {
                    let request = db
                        .collection::<Document>("billing_wallet_refresh_requests")
                        .find_one(doc! {})
                        .await
                        .unwrap();
                    assert_eq!(
                        request.as_ref().and_then(|row| row.get("token")),
                        request_after_winner
                            .as_ref()
                            .and_then(|row| row.get("token")),
                        "a deferred loser must not rotate or recreate the request",
                    );
                    if let Some(request) = request {
                        assert!(!request.contains_key("lease_token"));
                        assert!(!request.contains_key("lease_until"));
                    }
                }
                assert_snapshot_accounts(&db).await;
                assert_refresh_queue_converges(&db, new, expected_fetches).await;
                db.drop().await.unwrap();
            }
        }
    }

    #[tokio::test]
    async fn snapshot_order_expiry_loses_to_webhook_and_resumes_from_receipt() {
        use super::super::topup_expiry::complete_expiry_operation;
        use crate::models::credits::Credits;
        use mongodb::bson::Document;
        let db =
            crate::test_utils::connect_transaction_test_database("snapshot_expiry_loser").await;
        let wallet = wallet("owner", 0);
        db.collection::<BillingWallet>("billing_wallet")
            .insert_one(&wallet)
            .await
            .unwrap();
        super::super::exact_migration::migrate_one(&db, "billing_wallet", &wallet.id)
            .await
            .unwrap();
        let operation = snapshot_expiry_operation(&db, &wallet).await;
        let barrier = Arc::new(BalanceReadBarrier::default());
        let old = BalanceLago {
            balance_credits: 25,
            balance_read_barrier: Some(barrier.clone()),
            ..Default::default()
        };
        let new = BalanceLago {
            balance_credits: 40,
            ..Default::default()
        };
        let (result, ()) = tokio::join!(
            complete_expiry_operation(&db, &old, &wallet, operation, Utc::now()),
            async {
                barrier.started.notified().await;
                // The notification invalidates the read, but its own refresh
                // respects the active expiry bridge and queues instead.
                let result = handle_lago_webhook_event(
                    &db,
                    Some(&new),
                    "wallet.updated",
                    &json!({"wallet":{"external_customer_id":wallet.lago_customer_id}}),
                )
                .await
                .unwrap();
                assert_eq!(result.action, LagoWebhookAction::WalletRefreshDeferred);
                barrier.resume.notify_one();
            }
        );
        assert_eq!(result.unwrap(), 0);
        let saved = db
            .collection::<BillingWallet>("billing_wallet")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.balance_credits, wallet.balance_credits);
        assert_eq!(saved.pending_topup_expiry_credits, Credits::from_whole(1));
        let pending = saved.active_topup_expiry.clone().unwrap();
        assert_eq!(pending.lago_void_transaction_id.as_deref(), Some("void"));
        assert!(!pending.wallet_balance_applied);
        assert_eq!(
            db.collection::<Document>("billing_ledger")
                .count_documents(doc! {
                    "movement":{"$in":["wallet_adjusted","topup_expired"]}
                })
                .await
                .unwrap(),
            0
        );
        assert_snapshot_accounts(&db).await;
        assert_eq!(
            complete_expiry_operation(&db, &new, &saved, pending, Utc::now())
                .await
                .unwrap(),
            1
        );
        super::service_refresh_requests(&db, &new).await.unwrap();
        let saved = db
            .collection::<BillingWallet>("billing_wallet")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.balance_credits, Credits::from_whole(40));
        assert_eq!(saved.pending_topup_expiry_credits, Credits::ZERO);
        assert!(saved.active_topup_expiry.is_none());
        assert_eq!(
            db.collection::<Document>("billing_ledger")
                .count_documents(doc! {"movement":"topup_expired"})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            db.collection::<Document>("billing_wallet_refresh_requests")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_snapshot_accounts(&db).await;
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn snapshot_order_r3_expiry_rejection_uses_its_receipt_without_queueing() {
        use super::super::topup_expiry::complete_expiry_operation;
        use crate::models::credits::Credits;
        use mongodb::bson::Document;
        use std::sync::atomic::{AtomicUsize, Ordering};

        let db =
            crate::test_utils::connect_transaction_test_database("snapshot_expiry_no_queue").await;
        let wallet = wallet("owner", 0);
        db.collection::<BillingWallet>("billing_wallet")
            .insert_one(&wallet)
            .await
            .unwrap();
        super::super::exact_migration::migrate_one(&db, "billing_wallet", &wallet.id)
            .await
            .unwrap();
        let operation = snapshot_expiry_operation(&db, &wallet).await;
        let barrier = Arc::new(BalanceReadBarrier::default());
        let old = BalanceLago {
            balance_credits: 25,
            balance_read_barrier: Some(barrier.clone()),
            ..Default::default()
        };
        let (result, ()) = tokio::join!(
            complete_expiry_operation(&db, &old, &wallet, operation, Utc::now()),
            async {
                barrier.started.notified().await;
                db.collection::<Document>("billing_wallet")
                    .update_one(
                        doc! {"_id": &wallet.id},
                        doc! {"$inc": {"snapshot_generation": 1_i64}},
                    )
                    .await
                    .unwrap();
                barrier.resume.notify_one();
            }
        );
        assert_eq!(result.unwrap(), 0);
        let queued = db
            .collection::<Document>("billing_wallet_refresh_requests")
            .count_documents(doc! {})
            .await
            .unwrap();
        let balance_reads = Arc::new(AtomicUsize::new(0));
        let usage_reads = Arc::new(AtomicUsize::new(0));
        let fresh = BalanceLago {
            balance_credits: 40,
            balance_reads: Some(balance_reads.clone()),
            usage_reads: Some(usage_reads.clone()),
            ..Default::default()
        };
        for _ in 0..3 {
            super::service_refresh_requests(&db, &fresh).await.unwrap();
        }
        let worker_reads = (
            balance_reads.load(Ordering::SeqCst),
            usage_reads.load(Ordering::SeqCst),
        );
        let saved = db
            .collection::<BillingWallet>("billing_wallet")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        let pending = saved.active_topup_expiry.clone().unwrap();
        assert_eq!(pending.lago_void_transaction_id.as_deref(), Some("void"));
        assert!(!pending.wallet_balance_applied);
        // BalanceLago has no void implementation: recovery must use the receipt.
        assert_eq!(
            complete_expiry_operation(&db, &fresh, &saved, pending, Utc::now())
                .await
                .unwrap(),
            1
        );
        let saved = db
            .collection::<BillingWallet>("billing_wallet")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.balance_credits, Credits::from_whole(40));
        assert_eq!(saved.pending_topup_expiry_credits, Credits::ZERO);
        assert!(saved.active_topup_expiry.is_none());
        assert_eq!(
            db.collection::<Document>("billing_ledger")
                .count_documents(doc! {"movement": "topup_expired"})
                .await
                .unwrap(),
            1
        );
        assert_snapshot_accounts(&db).await;
        db.drop().await.unwrap();
        assert_eq!(
            queued, 0,
            "expiry already has a durable retry; it must not create a refresh request"
        );
        assert_eq!(
            worker_reads,
            (0, 0),
            "idle refresh ticks must not poll a pending expiry"
        );
    }

    #[tokio::test]
    async fn snapshot_order_r3_provider_failure_queues_only_paid_credit_receipts() {
        use mongodb::bson::Document;
        let mut observations = Vec::new();
        for (event, paid) in [
            ("wallet.updated", false),
            ("invoice.paid_credit_added", true),
        ] {
            let db = crate::test_utils::connect_transaction_test_database("snapshot_error_receipt")
                .await;
            let wallet = wallet("owner", 0);
            db.collection::<BillingWallet>("billing_wallet")
                .insert_one(&wallet)
                .await
                .unwrap();
            super::super::exact_migration::migrate_one(&db, "billing_wallet", &wallet.id)
                .await
                .unwrap();
            let failure = BalanceLago {
                balance_error: true,
                ..Default::default()
            };
            let payload = json!({
                "wallet": {"external_customer_id": wallet.lago_customer_id},
                "invoice": {"lago_id": "paid-receipt"},
                "lago_id": "unrelated-event-id",
            });
            assert!(
                handle_lago_webhook_event(&db, Some(&failure), event, &payload)
                    .await
                    .is_err()
            );
            let queued = db
                .collection::<Document>("billing_wallet_refresh_requests")
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
            let keys = queued.get_array("invoice_dedupe_keys").ok().cloned();
            let fresh = BalanceLago {
                balance_credits: 40,
                ..Default::default()
            };
            for _ in 0..2 {
                super::service_refresh_requests(&db, &fresh).await.unwrap();
            }
            assert_eq!(
                db.collection::<Document>("billing_wallet_refresh_requests")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
            let memos = db
                .collection::<Document>("billing_ledger")
                .count_documents(doc! {"movement": "wallet_credited"})
                .await
                .unwrap();
            assert_snapshot_accounts(&db).await;
            db.drop().await.unwrap();
            observations.push((event, paid, keys, memos));
        }
        for (event, paid, keys, memos) in observations {
            let expected = paid.then(|| {
                vec![mongodb::bson::Bson::String(
                    "lago-invoice:paid-receipt".into(),
                )]
            });
            assert_eq!(
                keys, expected,
                "{event} must only retain a receipt for a paid-credit event"
            );
            assert_eq!(memos, u64::from(paid), "{event} memo count");
        }
    }

    #[tokio::test]
    async fn snapshot_order_wallet_detects_old_style_write() {
        use crate::models::credits::Credits;
        use mongodb::bson::Document;
        let db =
            crate::test_utils::connect_transaction_test_database("snapshot_wallet_legacy").await;
        let wallet = wallet("owner", 0);
        db.collection::<BillingWallet>("billing_wallet")
            .insert_one(&wallet)
            .await
            .unwrap();
        super::super::exact_migration::migrate_one(&db, "billing_wallet", &wallet.id)
            .await
            .unwrap();
        let barrier = Arc::new(BalanceReadBarrier::default());
        let old_lago = BalanceLago {
            balance_credits: 25,
            balance_read_barrier: Some(barrier.clone()),
            ..Default::default()
        };
        tokio::join!(
            snapshot_entry(&db, &wallet, &old_lago, "reconcile"),
            async {
                barrier.started.notified().await;
                // New transaction maintains real checkpoints; erase only its additive
                // generation to emulate the legacy transaction's observable shape.
                let snapshot = db
                    .collection::<BillingWallet>("billing_wallet")
                    .find_one(doc! {})
                    .await
                    .unwrap()
                    .unwrap();
                super::refresh_wallet_balance(&db, &snapshot, Credits::from_whole(40))
                    .await
                    .unwrap();
                db.collection::<Document>("billing_wallet")
                    .update_one(doc! {}, doc! {"$unset":{"snapshot_generation":""}})
                    .await
                    .unwrap();
                barrier.resume.notify_one();
            }
        );
        let saved = db
            .collection::<BillingWallet>("billing_wallet")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.balance_credits, Credits::from_whole(40));
        assert_eq!(
            db.collection::<Document>("billing_ledger")
                .count_documents(doc! {"movement":"wallet_adjusted"})
                .await
                .unwrap(),
            1
        );
        assert_snapshot_accounts(&db).await;
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn snapshot_order_wallet_ack_command_cost_and_duplicate_do_not_reinvalidate() {
        use mongodb::event::{EventHandler, command::CommandEvent};
        let commands = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
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
            "wallet_ack_counts",
            handler,
        )
        .await
        .unwrap();
        let wallet = wallet("owner", 0);
        db.collection::<BillingWallet>("billing_wallet")
            .insert_one(&wallet)
            .await
            .unwrap();
        let row = usage_row("owner", &wallet.id, false);
        db.collection::<UsageMeterRow>("usage_meter")
            .insert_one(&row)
            .await
            .unwrap();
        for first in [true, false] {
            commands.lock().unwrap().clear();
            super::super::reconcile::mark_lago_acked(&db, &row.id)
                .await
                .unwrap();
            let measured = std::mem::take(&mut *commands.lock().unwrap());
            let expected = if first {
                vec![
                    "findAndModify:usage_meter",
                    "update:billing_wallet",
                    "commitTransaction:",
                ]
            } else {
                vec!["findAndModify:usage_meter", "commitTransaction:"]
            };
            assert_eq!(measured, expected);
            println!("WALLET_ACK_COMMANDS first={first}: {measured:?}");
            let saved = db
                .collection::<BillingWallet>("billing_wallet")
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(saved.snapshot_generation, 1);
            assert_eq!(saved.balance_credits, wallet.balance_credits);
            assert_eq!(saved.pending_lago_debits, wallet.pending_lago_debits);
            assert!(
                db.collection::<UsageMeterRow>("usage_meter")
                    .find_one(doc! {"_id":&row.id})
                    .await
                    .unwrap()
                    .unwrap()
                    .lago_acked
            );
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn snapshot_order_wallet_ack_invalidates_pending_clear() {
        use crate::models::credits::Credits;
        use mongodb::bson::Document;
        let db = crate::test_utils::connect_transaction_test_database("snapshot_wallet_ack").await;
        let wallet = wallet("owner", 7);
        db.collection::<BillingWallet>("billing_wallet")
            .insert_one(&wallet)
            .await
            .unwrap();
        super::super::exact_migration::migrate_one(&db, "billing_wallet", &wallet.id)
            .await
            .unwrap();
        let row = usage_row("owner", &wallet.id, false);
        db.collection::<UsageMeterRow>("usage_meter")
            .insert_one(&row)
            .await
            .unwrap();
        let barrier = Arc::new(BalanceReadBarrier::default());
        let old_lago = BalanceLago {
            balance_credits: 10,
            balance_read_barrier: Some(barrier.clone()),
            ..Default::default()
        };
        tokio::join!(
            snapshot_entry(&db, &wallet, &old_lago, "reconcile"),
            async {
                barrier.started.notified().await;
                super::super::reconcile::mark_lago_acked(&db, &row.id)
                    .await
                    .unwrap();
                barrier.resume.notify_one();
            }
        );
        let saved = db
            .collection::<BillingWallet>("billing_wallet")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.pending_lago_debits, Credits::from_whole(7));
        assert_eq!(
            db.collection::<Document>("billing_ledger")
                .count_documents(doc! {"movement":"wallet_adjusted"})
                .await
                .unwrap(),
            0
        );
        assert_snapshot_accounts(&db).await;
        super::service_refresh_requests(
            &db,
            &BalanceLago {
                balance_credits: 3,
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let saved = db
            .collection::<BillingWallet>("billing_wallet")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.pending_lago_debits, Credits::ZERO);
        assert_eq!(saved.balance_credits, Credits::from_whole(3));
        assert_snapshot_accounts(&db).await;
        db.drop().await.unwrap();
    }

    async fn assert_deferred_refresh_interleaving(replace_lease: bool) {
        use crate::models::credits::Credits;
        use mongodb::bson::{Bson, DateTime, Document};

        let db = connect_test_database("webhook_refresh_lease_race")
            .await
            .unwrap();
        let wallet = wallet("owner", 0);
        let wallets = db.collection::<Document>("billing_wallet");
        db.collection::<BillingWallet>("billing_wallet")
            .insert_one(&wallet)
            .await
            .unwrap();
        crate::services::billing::exact_migration::migrate_one(&db, "billing_wallet", &wallet.id)
            .await
            .unwrap();
        wallets
            .update_one(
                doc! { "_id": &wallet.id },
                doc! { "$set": { "active_settlement": { "row_id": "inflight" } } },
            )
            .await
            .unwrap();
        let requests = db.collection::<Document>("billing_wallet_refresh_requests");
        let defer = |balance, invoice: &str| {
            super::refresh_wallet_balance_with_receipt(
                &db,
                &wallet,
                Credits::from_whole(balance),
                Some(format!("lago-invoice:{invoice}")),
            )
        };
        assert!(defer(25, "first").await.unwrap().is_none());
        let barrier = Arc::new(BalanceReadBarrier::default());
        let first_lago = BalanceLago {
            balance_credits: 25,
            balance_read_barrier: Some(barrier.clone()),
            ..Default::default()
        };
        let successor_until = DateTime::from_chrono(Utc::now() + Duration::minutes(5));
        let interleave = async {
            // Pause the real worker after claiming, before it returns its snapshot.
            barrier.started.notified().await;
            let claimed = requests.find_one(doc! {}).await.unwrap().unwrap();
            assert!(claimed.get_datetime("lease_until").unwrap() > &DateTime::now());
            assert!(defer(40, "second").await.unwrap().is_none());
            let newer = requests.find_one(doc! {}).await.unwrap().unwrap();
            assert_ne!(claimed.get("token"), newer.get("token"));
            assert_eq!(claimed.get("lease_token"), newer.get("lease_token"));
            assert_eq!(
                newer.get_array("invoice_dedupe_keys").unwrap(),
                &[
                    Bson::from("lago-invoice:first"),
                    Bson::from("lago-invoice:second")
                ]
            );
            if replace_lease {
                // Model a successor claim while the original provider fetch is in flight.
                requests
                    .update_one(
                        doc! { "_id": &wallet.id },
                        doc! { "$set": {
                            "lease_token": "successor",
                            "lease_until": successor_until,
                        } },
                    )
                    .await
                    .unwrap();
            }
            wallets
                .update_one(
                    doc! { "_id": &wallet.id },
                    doc! { "$unset": { "active_settlement": "" } },
                )
                .await
                .unwrap();
            barrier.resume.notify_one();
            newer
        };
        let (result, newer) = tokio::time::timeout(std::time::Duration::from_secs(30), async {
            tokio::join!(
                super::service_refresh_requests(&db, &first_lago),
                interleave
            )
        })
        .await
        .expect("refresh interleaving must finish without waiting for lease expiry");
        result.unwrap();
        let pending = requests.find_one(doc! {}).await.unwrap().unwrap();
        assert_eq!(pending.get("token"), newer.get("token"));
        assert!(
            pending
                .get_array("invoice_dedupe_keys")
                .unwrap()
                .contains(&Bson::from("lago-invoice:second"))
        );
        let next_lago = BalanceLago {
            balance_credits: 40,
            ..Default::default()
        };
        if replace_lease {
            assert_eq!(pending.get_str("lease_token").unwrap(), "successor");
            assert_eq!(
                *pending.get_datetime("lease_until").unwrap(),
                successor_until
            );
        } else {
            assert!(!pending.contains_key("lease_token"));
            assert!(!pending.contains_key("lease_until"));
        }
        // No sleep or lease expiry: the newer request must run on the next pass,
        // unless a successor still owns it.
        super::service_refresh_requests(&db, &next_lago)
            .await
            .unwrap();
        let saved = db
            .collection::<BillingWallet>("billing_wallet")
            .find_one(doc! { "_id": &wallet.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.balance_credits,
            Credits::from_whole(if replace_lease { 25 } else { 40 })
        );
        assert_eq!(
            requests.count_documents(doc! {}).await.unwrap(),
            u64::from(replace_lease)
        );
        let ledger = db.collection::<Document>("billing_ledger");
        assert_eq!(
            ledger
                .count_documents(
                    doc! { "movement": "wallet_credited", "dedupe_key": "lago-invoice:first" }
                )
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            ledger
                .count_documents(
                    doc! { "movement": "wallet_credited", "dedupe_key": "lago-invoice:second" }
                )
                .await
                .unwrap(),
            u64::from(!replace_lease)
        );
    }

    #[tokio::test]
    async fn deferred_refresh_services_newer_request_on_next_pass() {
        assert_deferred_refresh_interleaving(false).await;
    }

    #[tokio::test]
    async fn deferred_refresh_preserves_successor_lease() {
        assert_deferred_refresh_interleaving(true).await;
    }
}
