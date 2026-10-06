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
            .find_one(doc! { "lago_customer_id": &customer_id })
            .await?
        else {
            return Ok(ignored_for_customer(customer_id));
        };
        let balance_credits = provider_effective_balance(lago, &wallet).await?;
        return Ok(
            match refresh_wallet_balance_with_receipt(
                db,
                &customer_id,
                balance_credits,
                if event_type == "invoice.paid_credit_added" {
                    extract_paid_credit_dedupe_key(payload)
                } else {
                    None
                },
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
                            extract_paid_credit_dedupe_key(payload),
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

pub async fn refresh_wallet_balance(
    db: &mongodb::Database,
    customer_id: &str,
    balance_credits: Credits,
) -> AppResult<Option<WalletRefreshOutcome>> {
    refresh_wallet_balance_with_receipt(db, customer_id, balance_credits, None).await
}

async fn refresh_wallet_balance_with_receipt(
    db: &mongodb::Database,
    customer_id: &str,
    balance_credits: Credits,
    invoice_dedupe_key: Option<String>,
) -> AppResult<Option<WalletRefreshOutcome>> {
    super::exact_migration::require_ready(db).await?;
    let Some(wallet) = db
        .collection::<BillingWallet>(BILLING_WALLET)
        .find_one(doc! { "lago_customer_id": customer_id })
        .await?
    else {
        return Ok(None);
    };
    let outcome = apply_balance(db, &wallet.id, balance_credits, None).await?;
    if outcome.is_none() {
        let set = doc! {
            "requested_at": bson::DateTime::from_chrono(Utc::now()),
            "token": uuid::Uuid::new_v4().to_string(),
        };
        let mut update = doc! { "$set": set };
        if let Some(invoice_dedupe_key) = invoice_dedupe_key {
            update.insert(
                "$addToSet",
                doc! { "invoice_dedupe_keys": invoice_dedupe_key },
            );
        }
        db.collection::<bson::Document>("billing_wallet_refresh_requests")
            .update_one(doc! { "_id": &wallet.id }, update)
            .upsert(true)
            .await?;
    }
    Ok(outcome)
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
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
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
            apply_balance(db, id, balance, None).await
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
    wallet_id: &str,
    balance: Credits,
    expiry: Option<&crate::models::billing_wallet::PurchasedCreditExpiryOperation>,
) -> AppResult<Option<WalletRefreshOutcome>> {
    for attempt in 0..16 {
        let mut session = db.client().start_session().await?;
        let database = db.clone();
        let wallet_identifier = wallet_id.to_string();
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
                let wallet_id = wallet_identifier.as_str();
                let expiry = expiry_operation.as_ref();
                let operation =
                    apply_provider_snapshot(db, wallet_id, balance, expiry, session).await;
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
    wallet_id: &str,
    balance: Credits,
    expiry: Option<&crate::models::billing_wallet::PurchasedCreditExpiryOperation>,
    session: &mut mongodb::ClientSession,
) -> AppResult<Option<WalletRefreshOutcome>> {
    let wallets = db.collection::<BillingWallet>(BILLING_WALLET);
    let Some(wallet) = wallets
        .find_one(doc! { "_id": wallet_id, "active_settlement": bson::Bson::Null })
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
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
        .update_one(doc! { "_id": wallet_id }, doc! { "$set": set })
        .session(&mut *session)
        .await?;
    Ok(Some(WalletRefreshOutcome {
        owner_id: wallet.owner_id,
        balance_credits: balance,
        pending_debits_cleared: clear && wallet.pending_lago_debits != Credits::ZERO,
    }))
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
            if let Some(barrier) = &self.balance_read_barrier {
                barrier.started.notify_one();
                barrier.resume.notified().await;
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
            service_id: Some("svc-1".to_string()),
            service_slug: Some("svc".to_string()),
            metric: BillingMetric::Requests,
            lago_metric_code: "platform_requests".to_string(),
            credential_class: CredentialClass::UserOwned,
            model: None,
            token_breakdown: None,
            audio_tokens: None,
            reserved_credits: crate::models::credits::Credits::from_whole(1),
            funding: None,
            quantity: Some(1),
            pending_resale_quantity: None,
            pending_platform_usage: None,
            pool_attempt: None,
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
        super::refresh_wallet_balance(&db, "owner", effective)
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
                "owner",
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
