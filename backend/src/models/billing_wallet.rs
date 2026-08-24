use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const COLLECTION_NAME: &str = "billing_wallet";

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PlanKind {
    Prepaid,
    Subscription,
    Hybrid,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CollectionState {
    Good,
    PastDue,
    Suspended,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct PurchasedCreditExpiryItem {
    pub lago_purchase_transaction_id: String,
    pub reference_id: String,
    pub amount_micros: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub settled_at: DateTime<Utc>,
}

/// Durable crash bridge for one Lago purchased-credit expiry debit.
///
/// This stays embedded on the wallet until the provider debit, exact balance,
/// top-up history, and billing-ledger entries have all been reconciled.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct PurchasedCreditExpiryOperation {
    pub operation_id: String,
    pub processing_token: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub lease_until: DateTime<Utc>,
    pub amount_micros: i64,
    pub items: Vec<PurchasedCreditExpiryItem>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lago_void_transaction_id: Option<String>,
    #[serde(default)]
    pub wallet_balance_applied: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct BillingWallet {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub lago_customer_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lago_wallet_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lago_subscription_id: Option<String>,
    pub plan_kind: PlanKind,
    #[serde(default)]
    pub balance_credits: i64,
    #[serde(default)]
    pub reserved_credits: i64,
    #[serde(default)]
    pub pending_lago_debits: i64,
    /// Whole-credit conservative hold while an external expiry debit is in
    /// flight. The exact microcredit balance is read back from Lago before
    /// this hold is released.
    #[serde(default)]
    pub pending_topup_expiry_credits: i64,
    #[serde(default)]
    pub has_payment_instrument: bool,
    #[serde(default)]
    pub overdraft_cap_credits: i64,
    #[serde(default)]
    pub suspended: bool,
    pub collection_state: CollectionState,
    /// Fairness cursor for the bounded purchased-credit expiry sweep.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub topup_expiry_checked_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schema(ignore)]
    pub active_topup_expiry: Option<PurchasedCreditExpiryOperation>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub balance_synced_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl BillingWallet {
    pub fn available_credits(&self) -> i64 {
        self.balance_credits
            .saturating_sub(self.reserved_credits)
            .saturating_sub(self.pending_lago_debits)
            .saturating_sub(self.pending_topup_expiry_credits)
    }

    pub fn available_with_overdraft_credits(&self) -> i64 {
        self.balance_credits
            .saturating_add(self.overdraft_cap_credits)
            .saturating_sub(self.reserved_credits)
            .saturating_sub(self.pending_lago_debits)
            .saturating_sub(self.pending_topup_expiry_credits)
    }

    pub fn is_suspended(&self) -> bool {
        self.suspended || self.collection_state == CollectionState::Suspended
    }
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{BillingWallet, CollectionState, PlanKind};

    #[test]
    fn available_balance_subtracts_holds_and_pending_debits() {
        let now = Utc::now();
        let wallet = BillingWallet {
            id: "wallet-1".to_string(),
            owner_id: "owner-1".to_string(),
            lago_customer_id: "owner-1".to_string(),
            lago_wallet_id: Some("lago-wallet-1".to_string()),
            lago_subscription_id: None,
            plan_kind: PlanKind::Prepaid,
            balance_credits: 100,
            reserved_credits: 30,
            pending_lago_debits: 25,
            pending_topup_expiry_credits: 5,
            has_payment_instrument: false,
            overdraft_cap_credits: 0,
            suspended: false,
            collection_state: CollectionState::Good,
            topup_expiry_checked_at: None,
            active_topup_expiry: None,
            balance_synced_at: now,
            created_at: now,
            updated_at: now,
        };

        assert_eq!(wallet.available_credits(), 40);
    }
}
