use chrono::Utc;
use mongodb::bson::{self, Bson, doc};
use serde_json::Value;

use crate::errors::{AppError, AppResult};
use crate::models::billing_wallet::{BillingWallet, COLLECTION_NAME as BILLING_WALLET};
use crate::models::usage_meter::COLLECTION_NAME as USAGE_METER;

use super::lago_client::LagoApi;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LagoWebhookAction {
    WalletRefreshed,
    EntitlementInvalidated,
    Ignored,
}

impl LagoWebhookAction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::WalletRefreshed => "wallet_refreshed",
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
    pub balance_credits: Option<i64>,
    pub pending_debits_cleared: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalletRefreshOutcome {
    pub owner_id: String,
    pub balance_credits: i64,
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
        let balance_credits = lago.wallet_balance(&customer_id).await?;
        return Ok(
            match refresh_wallet_balance(db, &customer_id, balance_credits).await? {
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
                None => ignored_for_customer(customer_id),
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
    balance_credits: i64,
) -> AppResult<Option<WalletRefreshOutcome>> {
    for _ in 0..2 {
        let Some(wallet) = db
            .collection::<BillingWallet>(BILLING_WALLET)
            .find_one(doc! { "lago_customer_id": customer_id })
            .await?
        else {
            return Ok(None);
        };

        let has_unacked_debits = has_unacked_wallet_debits(db, &wallet.owner_id).await?;
        let clear_pending_debits = !has_unacked_debits;
        let now = Utc::now();

        let mut filter = doc! { "_id": &wallet.id };
        if clear_pending_debits {
            filter.insert("pending_lago_debits", wallet.pending_lago_debits);
        }

        let mut set_doc = doc! {
            "balance_credits": balance_credits,
            "balance_synced_at": bson::DateTime::from_chrono(now),
            "updated_at": bson::DateTime::from_chrono(now),
        };
        if clear_pending_debits {
            set_doc.insert("pending_lago_debits", Bson::Int64(0));
        }

        let update = db
            .collection::<BillingWallet>(BILLING_WALLET)
            .update_one(filter, doc! { "$set": set_doc })
            .await?;
        if update.matched_count == 1 {
            return Ok(Some(WalletRefreshOutcome {
                owner_id: wallet.owner_id,
                balance_credits,
                pending_debits_cleared: clear_pending_debits && wallet.pending_lago_debits != 0,
            }));
        }
    }

    Ok(None)
}

async fn has_unacked_wallet_debits(db: &mongodb::Database, owner_id: &str) -> AppResult<bool> {
    let count = db
        .collection::<mongodb::bson::Document>(USAGE_METER)
        .count_documents(doc! {
            "billing_owner_id": owner_id,
            "status": "finalized",
            "wallet_id": { "$ne": null },
            "lago_acked": false,
        })
        .await?;
    Ok(count > 0)
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

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use chrono::{Duration, Utc};
    use mongodb::bson::doc;
    use serde_json::json;
    use uuid::Uuid;

    use crate::models::billing_wallet::{BillingWallet, CollectionState, PlanKind};
    use crate::models::service_billing::BillingMetric;
    use crate::models::usage_meter::{BillingLayer, CredentialClass, UsageMeterRow, UsageStatus};
    use crate::services::billing::lago_client::{
        Entitlement, LagoAck, LagoError, LagoEvent, LagoUsage, OwnerProvisionInput,
    };
    use crate::test_utils::connect_test_database;

    use super::{
        LagoApi, LagoWebhookAction, extract_external_customer_id, extract_external_subscription_id,
        handle_lago_webhook_event,
    };

    #[derive(Clone)]
    struct BalanceLago {
        balance_credits: i64,
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
                raw: json!({}),
            })
        }

        async fn wallet_balance(&self, _customer_id: &str) -> crate::errors::AppResult<i64> {
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
            balance_credits: 10,
            reserved_credits: 0,
            pending_lago_debits,
            pending_topup_expiry_credits: 0,
            has_payment_instrument: false,
            overdraft_cap_credits: 0,
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
            reserved_credits: 1,
            funding: None,
            quantity: Some(1),
            base_fee_micros: None,
            base_fee_applied: false,
            base_fee_applied_credits: 0,
            deferred_quantity: None,
            deferred_attempts: 0,
            deferred_next_retry_at: None,
            pending_resale_quantity: None,
            pending_platform_secondary_quantity: None,
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
                "customer": { "external_id": "owner-1" }
            }
        });

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
            }),
            "wallet.updated",
            &json!({ "webhook_type": "wallet.updated", "wallet": { "external_customer_id": owner_id } }),
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
        assert_eq!(saved.balance_credits, 25);
        assert_eq!(saved.pending_lago_debits, 0);
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
            }),
            "wallet_transaction.created",
            &json!({ "webhook_type": "wallet_transaction.created", "wallet_transaction": { "wallet": { "external_customer_id": owner_id } } }),
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
        assert_eq!(saved.balance_credits, 25);
        assert_eq!(saved.pending_lago_debits, 7);
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
                    "customer": { "external_id": owner_id }
                }
            }),
        )
        .await
        .expect("handle webhook");

        assert_eq!(outcome.action, LagoWebhookAction::EntitlementInvalidated);
        assert_eq!(outcome.owner_id.as_deref(), Some(owner_id));
    }
}
