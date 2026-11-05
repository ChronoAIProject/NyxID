use super::*;
use crate::services::billing::lago_client as lago;
use std::sync::Mutex;

/// Every LagoApi entry point, including default methods, records and fails.
#[derive(Default)]
pub(super) struct NoLago {
    calls: Mutex<Vec<&'static str>>,
}

impl NoLago {
    fn called(&self, method: &'static str) -> ! {
        self.calls.lock().unwrap().push(method);
        panic!("read-only wallet view contacted Lago: {method}")
    }

    pub(super) fn assert_unused(&self) {
        assert_eq!(*self.calls.lock().unwrap(), Vec::<&str>::new());
    }
}

#[async_trait]
impl LagoApi for NoLago {
    async fn tax_config(&self, _: &str, _: &str) -> AppResult<lago::LagoTaxConfig> {
        self.called("tax_config")
    }
    async fn ensure_customer(&self, _: &OwnerProvisionInput) -> AppResult<String> {
        self.called("ensure_customer")
    }
    async fn ensure_subscription(&self, _: &str, _: &str) -> AppResult<String> {
        self.called("ensure_subscription")
    }
    async fn ensure_wallet(&self, _: &str) -> AppResult<LagoWallet> {
        self.called("ensure_wallet")
    }
    async fn create_wallet_topup(
        &self,
        _: &str,
        _: &lago::WalletTopUpInput,
    ) -> AppResult<lago::WalletTopUpCheckout> {
        self.called("create_wallet_topup")
    }
    async fn record_event(&self, _: &LagoEvent) -> Result<LagoAck, LagoError> {
        self.called("record_event")
    }
    async fn record_events_batch(&self, _: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        self.called("record_events_batch")
    }
    async fn current_usage(&self, _: &str, _: &str) -> AppResult<LagoUsage> {
        self.called("current_usage")
    }
    async fn wallet_balance(&self, _: &str) -> AppResult<i64> {
        self.called("wallet_balance")
    }
    async fn wallet_balance_exact(&self, _: &str) -> AppResult<Credits> {
        self.called("wallet_balance_exact")
    }
    async fn wallet_balance_credits(&self, _: &str) -> AppResult<Credits> {
        self.called("wallet_balance_credits")
    }
    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        self.called("entitlements")
    }
    async fn runtime_subscription(&self, _: &str) -> AppResult<lago::RuntimeSubscription> {
        self.called("runtime_subscription")
    }
    async fn plan_rates(&self, _: &str) -> AppResult<lago::PlanRates> {
        self.called("plan_rates")
    }
    async fn subscription_invoices(&self, _: &str, _: u32, _: u32) -> AppResult<Value> {
        self.called("subscription_invoices")
    }
    async fn credit_invoices(&self, _: &str) -> AppResult<Vec<lago::InvoiceSummary>> {
        self.called("credit_invoices")
    }
    async fn invoice_summary(&self, _: &str) -> AppResult<Option<lago::InvoiceSummary>> {
        self.called("invoice_summary")
    }
    async fn invoice_download_url(&self, _: &str) -> AppResult<Option<String>> {
        self.called("invoice_download_url")
    }
    async fn wallet_transaction_invoices(&self, _: &str) -> AppResult<Vec<(String, String)>> {
        self.called("wallet_transaction_invoices")
    }
    async fn wallet_transactions(&self, _: &str) -> AppResult<Vec<lago::LagoWalletTransaction>> {
        self.called("wallet_transactions")
    }
    async fn void_wallet_credits(&self, _: &str, _: Credits, _: &str) -> AppResult<String> {
        self.called("void_wallet_credits")
    }
    async fn sync_standard_charge(&self, _: &str, _: &lago::ServicePriceSync) -> AppResult<()> {
        self.called("sync_standard_charge")
    }
    async fn remove_standard_charge(&self, _: &str, _: &str) -> AppResult<()> {
        self.called("remove_standard_charge")
    }
}
