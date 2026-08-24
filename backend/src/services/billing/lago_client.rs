use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::errors::{AppError, AppResult};

const HTTP_TIMEOUT_SECS: u64 = 20;
/// Lago attaches invoices to wallet transactions asynchronously; the payment
/// URL endpoint 422s until then. ~2.4s of retries covers a busy worker while
/// keeping the interactive checkout call responsive.
const PAYMENT_URL_MAX_ATTEMPTS: u32 = 5;
const PAYMENT_URL_RETRY_DELAY_MS: u64 = 600;
/// Invoice attachment + finalization for a wallet transaction usually
/// completes within a couple of seconds; ~6s of polling covers a busy
/// worker while keeping the interactive top-up call responsive.
const INVOICE_FINALIZE_MAX_ATTEMPTS: u32 = 10;
const INVOICE_FINALIZE_RETRY_DELAY_MS: u64 = 600;
const WALLET_TRANSACTION_PAGE_SIZE: usize = 100;
const MAX_WALLET_TRANSACTION_PAGES: u32 = 100;

#[async_trait]
pub trait LagoApi: Send + Sync {
    async fn ensure_customer(&self, owner: &OwnerProvisionInput) -> AppResult<String>;
    async fn ensure_subscription(&self, customer_id: &str, plan_code: &str) -> AppResult<String>;
    async fn ensure_wallet(&self, customer_id: &str) -> AppResult<LagoWallet> {
        Ok(LagoWallet {
            id: customer_id.to_string(),
            balance_credits: 0,
        })
    }

    async fn create_wallet_topup(
        &self,
        wallet_id: &str,
        _request: &WalletTopUpInput,
    ) -> AppResult<WalletTopUpCheckout> {
        Err(AppError::BillingProviderUnavailable(format!(
            "Lago wallet top-up is not supported for wallet '{wallet_id}'"
        )))
    }

    async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError>;
    async fn record_events_batch(&self, events: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError>;
    async fn current_usage(&self, customer_id: &str, subscription_id: &str)
    -> AppResult<LagoUsage>;
    async fn wallet_balance(&self, customer_id: &str) -> AppResult<i64>;
    async fn wallet_balance_micros(&self, customer_id: &str) -> AppResult<i64> {
        self.wallet_balance(customer_id)
            .await
            .map(|credits| credits.saturating_mul(1_000_000))
    }
    async fn entitlements(&self, subscription_id: &str) -> AppResult<Vec<Entitlement>>;
    /// Per-unit rates for the plan's standard charges, used to refresh the
    /// local rate cache. Defaults to empty so fakes opt in explicitly.
    async fn plan_rates(&self, _plan_code: &str) -> AppResult<Vec<PlanRate>> {
        Ok(Vec::new())
    }
    /// Credit (top-up) invoices for a customer, used to resolve payment
    /// outcomes for the top-up history. Defaults to empty for fakes.
    async fn credit_invoices(&self, _external_customer_id: &str) -> AppResult<Vec<InvoiceSummary>> {
        Ok(Vec::new())
    }
    /// A single invoice, or None when it does not exist.
    async fn invoice_summary(&self, _lago_invoice_id: &str) -> AppResult<Option<InvoiceSummary>> {
        Ok(None)
    }
    /// The downloadable PDF URL for an invoice; None while Lago is still
    /// generating the document.
    async fn invoice_download_url(&self, _lago_invoice_id: &str) -> AppResult<Option<String>> {
        Ok(None)
    }
    /// (wallet_transaction_id, invoice_id) pairs for a wallet, used to
    /// backfill invoice links on top-up sessions stored before the id was
    /// captured at creation. Defaults to empty for fakes.
    async fn wallet_transaction_invoices(
        &self,
        _wallet_id: &str,
    ) -> AppResult<Vec<(String, String)>> {
        Ok(Vec::new())
    }
    /// Settled wallet transactions used for per-purchase expiry accounting.
    async fn wallet_transactions(&self, _wallet_id: &str) -> AppResult<Vec<LagoWalletTransaction>> {
        Ok(Vec::new())
    }
    /// Remove credits from a wallet. Lago traceable wallets consume inbound
    /// transactions in priority/FIFO order, matching NyxID's expiry sweep.
    async fn void_wallet_credits(
        &self,
        wallet_id: &str,
        _amount_micros: i64,
        _operation_id: &str,
    ) -> AppResult<String> {
        Err(AppError::BillingProviderUnavailable(format!(
            "Lago wallet credit voiding is not supported for wallet '{wallet_id}'"
        )))
    }
    /// Create or update a NyxID-owned sum metric and its standard charge on
    /// the configured plan. Fakes opt out unless a pricing test needs it.
    async fn sync_standard_charge(
        &self,
        _plan_code: &str,
        _input: &ServicePriceSync,
    ) -> AppResult<()> {
        Err(AppError::BillingProviderUnavailable(
            "Lago service-price synchronization is not supported".to_string(),
        ))
    }
    /// Remove a NyxID-owned standard charge while preserving every other
    /// charge on the plan. Fakes opt out unless a pricing test needs it.
    async fn remove_standard_charge(&self, plan_code: &str, _metric_code: &str) -> AppResult<()> {
        Err(AppError::BillingProviderUnavailable(format!(
            "Lago service-price removal is not supported for plan '{plan_code}'"
        )))
    }
}

/// A trimmed Lago invoice used for top-up history and receipt downloads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InvoiceSummary {
    pub lago_id: String,
    pub number: String,
    pub total_amount_cents: i64,
    /// Lago lifecycle status: draft, finalized, voided, ...
    pub status: String,
    /// Payment outcome: pending, succeeded, failed.
    pub payment_status: String,
    pub external_customer_id: Option<String>,
    pub issuing_date: Option<String>,
}

/// A per-unit price from a Lago plan charge, converted to micro-credits
/// (1 credit = 1 USD, matching the wallet rate_amount NyxID provisions).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRate {
    pub lago_metric_code: String,
    pub credits_per_unit_micros: i64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ServicePriceSync {
    pub metric_code: String,
    pub metric_name: String,
    pub metric_description: String,
    pub credits_per_unit: String,
}

#[derive(Clone)]
pub struct LagoClient {
    base_url: String,
    api_key: String,
    payment_provider_code: Option<String>,
    http: reqwest::Client,
}

fn plan_update_with_standard_charge(
    response: &Value,
    metric_id: &str,
    input: &ServicePriceSync,
) -> AppResult<Value> {
    let mut plan = lago_plan_for_update(response)?;
    let existing = plan
        .remove("charges")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut charges = Vec::with_capacity(existing.len().saturating_add(1));
    let mut replaced = false;
    for charge in existing {
        let matches = charge_matches_metric(&charge, Some(metric_id), &input.metric_code);
        let mut charge = existing_charge_for_plan_update(charge)?;
        if matches {
            let object = charge.as_object_mut().ok_or_else(|| {
                AppError::BillingProviderUnavailable(
                    "Lago plan returned a non-object charge".to_string(),
                )
            })?;
            object.insert(
                "billable_metric_id".to_string(),
                Value::String(metric_id.to_string()),
            );
            object.insert(
                "charge_model".to_string(),
                Value::String("standard".to_string()),
            );
            object.insert("invoiceable".to_string(), Value::Bool(true));
            object.insert("pay_in_advance".to_string(), Value::Bool(false));
            object.insert("prorated".to_string(), Value::Bool(false));
            object.insert(
                "properties".to_string(),
                json!({ "amount": input.credits_per_unit }),
            );
            replaced = true;
        }
        charges.push(charge);
    }
    if !replaced {
        charges.push(json!({
            "billable_metric_id": metric_id,
            "charge_model": "standard",
            "invoiceable": true,
            "pay_in_advance": false,
            "prorated": false,
            "properties": { "amount": input.credits_per_unit },
        }));
    }
    plan.insert("charges".to_string(), Value::Array(charges));
    Ok(json!({ "plan": plan }))
}

fn plan_update_without_metric_charge(
    response: &Value,
    metric_code: &str,
) -> AppResult<Option<Value>> {
    let mut plan = lago_plan_for_update(response)?;
    let existing = plan
        .remove("charges")
        .and_then(|value| value.as_array().cloned())
        .unwrap_or_default();
    let mut removed = false;
    let mut charges = Vec::with_capacity(existing.len());
    for charge in existing {
        if charge_matches_metric(&charge, None, metric_code) {
            removed = true;
        } else {
            charges.push(existing_charge_for_plan_update(charge)?);
        }
    }
    if !removed {
        return Ok(None);
    }
    plan.insert("charges".to_string(), Value::Array(charges));
    Ok(Some(json!({ "plan": plan })))
}

fn lago_plan_for_update(response: &Value) -> AppResult<serde_json::Map<String, Value>> {
    let response_plan = response
        .get("plan")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            AppError::BillingProviderUnavailable(
                "Lago plan response did not include a plan object".to_string(),
            )
        })?;
    for required in [
        "name",
        "code",
        "interval",
        "amount_cents",
        "amount_currency",
    ] {
        if !response_plan.contains_key(required) {
            return Err(AppError::BillingProviderUnavailable(format!(
                "Lago plan response did not include required field '{required}'"
            )));
        }
    }
    // GET /plans/{code} also returns response-only fields and nested resources
    // such as fixed_charges. Sending those back is unsafe: their response ids
    // use `lago_id`, while the update API expects `id`, so Lago can recreate
    // them. Round-trip only the accepted plan scalars; charges are handled
    // separately because PUT replaces that complete array.
    let mut plan = serde_json::Map::new();
    for field in [
        "name",
        "invoice_display_name",
        "code",
        "interval",
        "description",
        "amount_cents",
        "amount_currency",
        "trial_period",
        "pay_in_advance",
        "bill_charges_monthly",
        "bill_fixed_charges_monthly",
    ] {
        if let Some(value) = response_plan.get(field) {
            plan.insert(field.to_string(), value.clone());
        }
    }
    if let Some(charges) = response_plan.get("charges") {
        plan.insert("charges".to_string(), charges.clone());
    }
    Ok(plan)
}

fn existing_charge_for_plan_update(mut charge: Value) -> AppResult<Value> {
    let object = charge.as_object_mut().ok_or_else(|| {
        AppError::BillingProviderUnavailable("Lago plan returned a non-object charge".to_string())
    })?;
    if object.get("id").is_none_or(|value| value.is_null()) {
        let id = object
            .get("lago_id")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                AppError::BillingProviderUnavailable(
                    "Lago existing plan charge did not include an id".to_string(),
                )
            })?
            .to_string();
        object.insert("id".to_string(), Value::String(id));
    }
    Ok(charge)
}

fn charge_matches_metric(charge: &Value, metric_id: Option<&str>, metric_code: &str) -> bool {
    metric_id.is_some_and(|id| value_string(charge, &["billable_metric_id"]).as_deref() == Some(id))
        || value_string(charge, &["billable_metric_code"]).as_deref() == Some(metric_code)
        || charge
            .get("billable_metric")
            .and_then(|metric| value_string(metric, &["code"]))
            .as_deref()
            == Some(metric_code)
}

impl LagoClient {
    pub fn new(base_url: String, api_key: String) -> AppResult<Self> {
        let base_url = base_url.trim().trim_end_matches('/').to_string();
        if base_url.is_empty() {
            return Err(AppError::BillingNotConfigured(
                "LAGO_API_URL is empty".to_string(),
            ));
        }
        if api_key.trim().is_empty() {
            return Err(AppError::BillingNotConfigured(
                "LAGO_API_KEY is empty".to_string(),
            ));
        }

        let http = reqwest::Client::builder()
            .timeout(Duration::from_secs(HTTP_TIMEOUT_SECS))
            .build()
            .map_err(|err| {
                AppError::Internal(format!("failed to build Lago HTTP client: {err}"))
            })?;

        Ok(Self {
            base_url,
            api_key,
            payment_provider_code: None,
            http,
        })
    }

    /// Link newly created customers to this Lago payment provider connection
    /// (a "stripe" connection code) so top-up checkout URLs can be generated.
    pub fn with_payment_provider_code(mut self, code: Option<String>) -> Self {
        self.payment_provider_code = code;
        self
    }

    fn url(&self, path: &str) -> String {
        let path = path.trim_start_matches('/');
        if self.base_url.ends_with("/api/v1") {
            format!("{}/{}", self.base_url, path)
        } else {
            format!("{}/api/v1/{}", self.base_url, path)
        }
    }

    fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, self.url(path))
            .bearer_auth(&self.api_key)
            .header(reqwest::header::ACCEPT, "application/json")
    }

    async fn json_request(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<Value>,
    ) -> Result<Value, LagoError> {
        let mut builder = self.request(method, path);
        if let Some(body) = body {
            builder = builder.json(&body);
        }

        let response = builder.send().await.map_err(LagoError::from_reqwest)?;
        let status = response.status();
        let text = response.text().await.unwrap_or_default();
        let json = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);

        if status.is_success() {
            Ok(json)
        } else {
            Err(LagoError::from_response(status, json, text))
        }
    }

    async fn get_one_by_external_id(
        &self,
        resource: &str,
        wrapper_key: &str,
        external_id: &str,
    ) -> AppResult<Option<Value>> {
        let path = format!("{}/{}", resource, urlencoding::encode(external_id));
        match self.json_request(reqwest::Method::GET, &path, None).await {
            Ok(value) => Ok(value.get(wrapper_key).cloned().or(Some(value))),
            Err(error) if error.status == Some(StatusCode::NOT_FOUND) => Ok(None),
            Err(error) => Err(lago_error_to_app(error)),
        }
    }
}

#[async_trait]
impl LagoApi for LagoClient {
    async fn ensure_customer(&self, owner: &OwnerProvisionInput) -> AppResult<String> {
        if let Some(existing) = self
            .get_one_by_external_id("customers", "customer", &owner.external_customer_id)
            .await?
            && value_string(&existing, &["external_id"])
                .as_deref()
                .is_some_and(|id| id == owner.external_customer_id)
        {
            return Ok(owner.external_customer_id.clone());
        }

        let mut customer = json!({
            "external_id": owner.external_customer_id,
            "name": owner.name,
            "email": owner.email,
        });
        if let Some(code) = &self.payment_provider_code {
            customer["billing_configuration"] = json!({
                "payment_provider": "stripe",
                "payment_provider_code": code,
                "sync_with_provider": true,
            });
        }
        let body = json!({ "customer": customer });
        match self
            .json_request(reqwest::Method::POST, "customers", Some(body))
            .await
        {
            Ok(_) => Ok(owner.external_customer_id.clone()),
            Err(error) if error.is_conflict_like() => Ok(owner.external_customer_id.clone()),
            Err(error) => Err(lago_error_to_app(error)),
        }
    }

    async fn ensure_subscription(&self, customer_id: &str, plan_code: &str) -> AppResult<String> {
        let external_id = subscription_external_id(customer_id, plan_code);
        if let Some(existing) = self
            .get_one_by_external_id("subscriptions", "subscription", &external_id)
            .await?
            && value_string(&existing, &["external_id"])
                .as_deref()
                .is_some_and(|id| id == external_id)
        {
            return Ok(external_id);
        }

        let body = json!({
            "subscription": {
                "external_customer_id": customer_id,
                "external_id": external_id,
                "plan_code": plan_code,
                "billing_time": "calendar",
            }
        });
        match self
            .json_request(reqwest::Method::POST, "subscriptions", Some(body))
            .await
        {
            Ok(_) => Ok(subscription_external_id(customer_id, plan_code)),
            Err(error) if error.is_conflict_like() => {
                Ok(subscription_external_id(customer_id, plan_code))
            }
            Err(error) => Err(lago_error_to_app(error)),
        }
    }

    async fn ensure_wallet(&self, customer_id: &str) -> AppResult<LagoWallet> {
        if let Some(wallet) = self.get_wallet_by_customer_id(customer_id).await? {
            return Ok(wallet);
        }

        let body = json!({
            "wallet": {
                "external_customer_id": customer_id,
                "currency": "USD",
                // Lago requires a credit-to-currency rate on wallet creation;
                // NyxID credits are denominated 1:1 with the wallet currency.
                "rate_amount": "1",
            }
        });
        match self
            .json_request(reqwest::Method::POST, "wallets", Some(body))
            .await
        {
            Ok(value) => extract_wallet(&value).ok_or_else(|| {
                AppError::BillingProviderUnavailable(
                    "Lago wallet creation response did not include a wallet id".to_string(),
                )
            }),
            Err(error) if error.is_conflict_like() => self
                .get_wallet_by_customer_id(customer_id)
                .await?
                .ok_or_else(|| {
                    AppError::BillingProviderUnavailable(
                        "Lago reported an existing wallet but it could not be read".to_string(),
                    )
                }),
            Err(error) => Err(lago_error_to_app(error)),
        }
    }

    async fn create_wallet_topup(
        &self,
        wallet_id: &str,
        request: &WalletTopUpInput,
    ) -> AppResult<WalletTopUpCheckout> {
        let body = json!({
            "wallet_transaction": {
                "wallet_id": wallet_id,
                // Lago validates credit amounts as decimal strings and
                // rejects JSON numbers with invalid_paid_credits.
                "paid_credits": request.amount_credits.to_string(),
                // granted_credits are FREE/promotional in Lago and ADDITIVE to
                // paid_credits. A paid top-up must grant ONLY the purchased
                // amount, so this is 0; otherwise the customer receives 2N
                // credits for an N payment. See #1050.
                "granted_credits": "0",
                "external_id": request.external_id,
                "invoice_requires_successful_payment": true,
            }
        });
        let value = self
            .json_request(reqwest::Method::POST, "wallet_transactions", Some(body))
            .await
            .map_err(lago_error_to_app)?;

        let transaction = extract_wallet_topup_transaction(&value).ok_or_else(|| {
            AppError::BillingProviderUnavailable(
                "Lago top-up response did not include a wallet transaction id".to_string(),
            )
        })?;
        // The invoice must be finalized BEFORE the first payment URL
        // request. On Lago >= 1.50 an early request does not fail: it
        // permanently caches a Stripe session snapshotted from the $0
        // draft invoice, leaving an unpayable checkout.
        let lago_invoice_id = self
            .await_topup_invoice_finalized(wallet_id, &transaction)
            .await?;
        let payment_details = self
            .generate_wallet_transaction_payment_url(&transaction.wallet_transaction_id)
            .await?;

        Ok(WalletTopUpCheckout {
            wallet_transaction_id: transaction.wallet_transaction_id,
            lago_invoice_id: Some(lago_invoice_id),
            payment_url: payment_details.payment_url,
            payment_provider: Some(payment_details.payment_provider),
        })
    }

    async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
        let body = json!({ "event": event });
        self.json_request(reqwest::Method::POST, "events", Some(body))
            .await
            .map(|_| LagoAck {
                transaction_id: event.transaction_id.clone(),
            })
    }

    async fn record_events_batch(&self, events: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        if events.is_empty() {
            return Ok(Vec::new());
        }
        let body = json!({ "events": events });
        match self
            .json_request(reqwest::Method::POST, "events/batch", Some(body))
            .await
        {
            Ok(_) => Ok(events
                .iter()
                .map(|event| LagoAck {
                    transaction_id: event.transaction_id.clone(),
                })
                .collect()),
            Err(error)
                if matches!(
                    error.status,
                    Some(StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED)
                ) =>
            {
                let mut acks = Vec::with_capacity(events.len());
                for event in events {
                    acks.push(self.record_event(event).await?);
                }
                Ok(acks)
            }
            Err(error) => Err(error),
        }
    }

    async fn current_usage(
        &self,
        customer_id: &str,
        subscription_id: &str,
    ) -> AppResult<LagoUsage> {
        let path = if subscription_id.trim().is_empty() {
            format!(
                "customers/{}/current_usage",
                urlencoding::encode(customer_id)
            )
        } else {
            format!(
                "customers/{}/current_usage?external_subscription_id={}",
                urlencoding::encode(customer_id),
                urlencoding::encode(subscription_id)
            )
        };
        let value = self
            .json_request(reqwest::Method::GET, &path, None)
            .await
            .map_err(lago_error_to_app)?;
        Ok(LagoUsage {
            customer_id: customer_id.to_string(),
            subscription_id: subscription_id.to_string(),
            raw: value,
        })
    }

    async fn wallet_balance(&self, customer_id: &str) -> AppResult<i64> {
        let value = self
            .json_request(
                reqwest::Method::GET,
                &format!(
                    "wallets?external_customer_id={}",
                    urlencoding::encode(customer_id)
                ),
                None,
            )
            .await
            .map_err(lago_error_to_app)?;
        extract_active_wallet_balance_credits(&value).ok_or_else(|| {
            AppError::BillingProviderUnavailable(
                "Lago wallet balance response did not include a balance".to_string(),
            )
        })
    }

    async fn entitlements(&self, subscription_id: &str) -> AppResult<Vec<Entitlement>> {
        let path = format!(
            "subscriptions/{}/entitlements",
            urlencoding::encode(subscription_id)
        );
        let value = self
            .json_request(reqwest::Method::GET, &path, None)
            .await
            .map_err(lago_error_to_app)?;
        Ok(value
            .get("entitlements")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        value_string(item, &["code", "feature_code"]).map(|code| Entitlement {
                            code,
                            raw: item.clone(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn wallet_transaction_invoices(
        &self,
        wallet_id: &str,
    ) -> AppResult<Vec<(String, String)>> {
        let value = self
            .json_request(
                reqwest::Method::GET,
                &format!(
                    "wallets/{}/wallet_transactions?per_page=100",
                    urlencoding::encode(wallet_id)
                ),
                None,
            )
            .await
            .map_err(lago_error_to_app)?;
        Ok(value
            .get("wallet_transactions")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(|item| {
                        Some((
                            value_string(item, &["lago_id"])?,
                            value_string(item, &["lago_invoice_id"])?,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn wallet_transactions(&self, wallet_id: &str) -> AppResult<Vec<LagoWalletTransaction>> {
        let mut transactions = Vec::new();
        for page in 1..=MAX_WALLET_TRANSACTION_PAGES {
            let value = self
                .json_request(
                    reqwest::Method::GET,
                    &format!(
                        "wallets/{}/wallet_transactions?per_page={WALLET_TRANSACTION_PAGE_SIZE}&page={page}",
                        urlencoding::encode(wallet_id)
                    ),
                    None,
                )
                .await
                .map_err(lago_error_to_app)?;
            let items = value
                .get("wallet_transactions")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            let item_count = items.len();
            transactions.extend(items.iter().filter_map(wallet_transaction_from_value));
            if item_count < WALLET_TRANSACTION_PAGE_SIZE {
                break;
            }
            if page == MAX_WALLET_TRANSACTION_PAGES {
                tracing::warn!(
                    wallet_id,
                    max_pages = MAX_WALLET_TRANSACTION_PAGES,
                    page_size = WALLET_TRANSACTION_PAGE_SIZE,
                    "Lago wallet transaction history reached the safety cap; skipping expiry because FIFO history is incomplete"
                );
                return Err(AppError::BillingProviderUnavailable(format!(
                    "Lago wallet '{wallet_id}' transaction history exceeds the safe pagination limit"
                )));
            }
        }
        Ok(transactions)
    }

    async fn void_wallet_credits(
        &self,
        wallet_id: &str,
        amount_micros: i64,
        operation_id: &str,
    ) -> AppResult<String> {
        if amount_micros <= 0 {
            return Err(AppError::ValidationError(
                "wallet void amount must be positive".to_string(),
            ));
        }
        // Lago wallet credits have five decimal places. Transaction amounts
        // returned by Lago are therefore multiples of 10 microcredits.
        let amount = micros_to_lago_credits(amount_micros);
        let transaction_name = purchased_credit_expiry_transaction_name(operation_id);
        let value = self
            .json_request(
                reqwest::Method::POST,
                "wallet_transactions",
                Some(json!({
                    "wallet_transaction": {
                        "wallet_id": wallet_id,
                        "voided_credits": amount,
                        "name": transaction_name,
                    }
                })),
            )
            .await
            .map_err(lago_error_to_app)?;
        value
            .get("wallet_transactions")
            .and_then(Value::as_array)
            .and_then(|items| items.first())
            .and_then(|item| value_string(item, &["lago_id", "id"]))
            .ok_or_else(|| {
                AppError::BillingProviderUnavailable(
                    "Lago wallet void response had no transaction id".to_string(),
                )
            })
    }

    async fn wallet_balance_micros(&self, customer_id: &str) -> AppResult<i64> {
        let value = self
            .json_request(
                reqwest::Method::GET,
                &format!(
                    "wallets?external_customer_id={}",
                    urlencoding::encode(customer_id)
                ),
                None,
            )
            .await
            .map_err(lago_error_to_app)?;
        extract_active_wallet_balance_micros(&value).ok_or_else(|| {
            AppError::BillingProviderUnavailable(
                "Lago wallet response did not include a balance".to_string(),
            )
        })
    }

    async fn credit_invoices(&self, external_customer_id: &str) -> AppResult<Vec<InvoiceSummary>> {
        let value = self
            .json_request(
                reqwest::Method::GET,
                &format!(
                    "invoices?external_customer_id={}&invoice_type=credit&per_page=100",
                    urlencoding::encode(external_customer_id)
                ),
                None,
            )
            .await
            .map_err(lago_error_to_app)?;
        Ok(value
            .get("invoices")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(invoice_summary_from_value)
                    .collect()
            })
            .unwrap_or_default())
    }

    async fn invoice_summary(&self, lago_invoice_id: &str) -> AppResult<Option<InvoiceSummary>> {
        let path = format!("invoices/{}", urlencoding::encode(lago_invoice_id));
        match self.json_request(reqwest::Method::GET, &path, None).await {
            Ok(value) => Ok(value.get("invoice").and_then(invoice_summary_from_value)),
            Err(error) if error.status == Some(StatusCode::NOT_FOUND) => Ok(None),
            Err(error) => Err(lago_error_to_app(error)),
        }
    }

    async fn invoice_download_url(&self, lago_invoice_id: &str) -> AppResult<Option<String>> {
        // POST download triggers PDF generation when the document does not
        // exist yet; file_url stays null until Lago's worker finishes, so
        // retry briefly before reporting "still generating".
        let path = format!("invoices/{}/download", urlencoding::encode(lago_invoice_id));
        let mut attempt = 0;
        loop {
            attempt += 1;
            let value = self
                .json_request(reqwest::Method::POST, &path, None)
                .await
                .map_err(lago_error_to_app)?;
            let file_url = value
                .get("invoice")
                .and_then(|invoice| value_string(invoice, &["file_url"]))
                .filter(|url| !url.is_empty());
            if file_url.is_some() || attempt >= PAYMENT_URL_MAX_ATTEMPTS {
                return Ok(file_url);
            }
            tokio::time::sleep(Duration::from_millis(PAYMENT_URL_RETRY_DELAY_MS)).await;
        }
    }

    async fn plan_rates(&self, plan_code: &str) -> AppResult<Vec<PlanRate>> {
        let path = format!("plans/{}", urlencoding::encode(plan_code));
        let value = self
            .json_request(reqwest::Method::GET, &path, None)
            .await
            .map_err(lago_error_to_app)?;
        let charges = value
            .get("plan")
            .and_then(|plan| plan.get("charges"))
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();

        let mut rates = Vec::new();
        for charge in &charges {
            let Some(code) = value_string(charge, &["billable_metric_code"]).or_else(|| {
                charge
                    .get("billable_metric")
                    .and_then(|metric| value_string(metric, &["code"]))
            }) else {
                continue;
            };
            let model = value_string(charge, &["charge_model"]).unwrap_or_default();
            if model != "standard" {
                tracing::warn!(
                    metric_code = %code,
                    charge_model = %model,
                    "Skipping non-standard Lago charge in rate cache refresh"
                );
                continue;
            }
            let Some(micros) = charge
                .get("properties")
                .and_then(|properties| value_string(properties, &["amount"]))
                .as_deref()
                .and_then(decimal_credits_to_micros)
            else {
                tracing::warn!(
                    metric_code = %code,
                    "Skipping Lago charge with unparseable amount in rate cache refresh"
                );
                continue;
            };
            rates.push(PlanRate {
                lago_metric_code: code,
                credits_per_unit_micros: micros,
            });
        }
        Ok(rates)
    }

    async fn sync_standard_charge(
        &self,
        plan_code: &str,
        input: &ServicePriceSync,
    ) -> AppResult<()> {
        let metric_path = format!(
            "billable_metrics/{}",
            urlencoding::encode(&input.metric_code)
        );
        let metric = match self
            .json_request(reqwest::Method::GET, &metric_path, None)
            .await
        {
            Ok(value) => value,
            Err(error) if error.status == Some(StatusCode::NOT_FOUND) => {
                let body = json!({
                    "billable_metric": {
                        "name": input.metric_name,
                        "code": input.metric_code,
                        "description": input.metric_description,
                        "aggregation_type": "sum_agg",
                        "field_name": "quantity",
                        "recurring": false,
                    }
                });
                self.json_request(reqwest::Method::POST, "billable_metrics", Some(body))
                    .await
                    .map_err(lago_error_to_app)?
            }
            Err(error) => return Err(lago_error_to_app(error)),
        };
        let metric = metric.get("billable_metric").unwrap_or(&metric);
        let metric_id = value_string(metric, &["lago_id", "id"]).ok_or_else(|| {
            AppError::BillingProviderUnavailable(
                "Lago billable metric response did not include an id".to_string(),
            )
        })?;
        let aggregation = value_string(metric, &["aggregation_type"]).unwrap_or_default();
        let field_name = value_string(metric, &["field_name"]).unwrap_or_default();
        if aggregation != "sum_agg" || field_name != "quantity" {
            return Err(AppError::BillingProviderUnavailable(format!(
                "Lago metric '{}' exists with incompatible aggregation; expected sum_agg(quantity)",
                input.metric_code
            )));
        }

        // Names and descriptions remain NyxID-owned too. Lago does not allow
        // changing aggregation semantics once a metric has attached usage,
        // so only descriptive fields are updated on an existing metric.
        self.json_request(
            reqwest::Method::PUT,
            &metric_path,
            Some(json!({
                "billable_metric": {
                    "name": input.metric_name,
                    "description": input.metric_description,
                }
            })),
        )
        .await
        .map_err(lago_error_to_app)?;

        let plan_path = format!("plans/{}", urlencoding::encode(plan_code));
        let plan = self
            .json_request(reqwest::Method::GET, &plan_path, None)
            .await
            .map_err(lago_error_to_app)?;
        // Lago has no charge-scoped plan endpoint. PUT /plans/{code} replaces
        // the complete charges array, so the update payload must round-trip
        // every unrelated charge (including its update `id`) and every plan
        // field returned by GET. A newly inserted charge intentionally has no
        // id; Lago assigns it during this plan update.
        let body = plan_update_with_standard_charge(&plan, &metric_id, input)?;
        self.json_request(reqwest::Method::PUT, &plan_path, Some(body))
            .await
            .map_err(lago_error_to_app)?;
        Ok(())
    }

    async fn remove_standard_charge(&self, plan_code: &str, metric_code: &str) -> AppResult<()> {
        let plan_path = format!("plans/{}", urlencoding::encode(plan_code));
        let plan = self
            .json_request(reqwest::Method::GET, &plan_path, None)
            .await
            .map_err(lago_error_to_app)?;
        let Some(body) = plan_update_without_metric_charge(&plan, metric_code)? else {
            return Ok(());
        };
        self.json_request(reqwest::Method::PUT, &plan_path, Some(body))
            .await
            .map_err(lago_error_to_app)?;
        Ok(())
    }
}

impl LagoClient {
    /// Wait for the invoice of a freshly created wallet transaction to be
    /// attached and finalized. Lago does both asynchronously (typically
    /// within a couple of seconds), and requesting the payment URL earlier
    /// bakes the draft state into the checkout session.
    async fn await_topup_invoice_finalized(
        &self,
        wallet_id: &str,
        transaction: &WalletTopUpTransaction,
    ) -> AppResult<String> {
        let mut invoice_id = transaction.lago_invoice_id.clone();
        let mut attempt = 0;
        loop {
            attempt += 1;
            if invoice_id.is_none() {
                invoice_id = self
                    .find_transaction_invoice_id(wallet_id, &transaction.wallet_transaction_id)
                    .await?;
            }
            if let Some(id) = invoice_id.as_deref() {
                let value = self
                    .json_request(
                        reqwest::Method::GET,
                        &format!("invoices/{}", urlencoding::encode(id)),
                        None,
                    )
                    .await
                    .map_err(lago_error_to_app)?;
                let status = value
                    .get("invoice")
                    .and_then(|invoice| value_string(invoice, &["status"]))
                    .unwrap_or_default();
                if status == "finalized" {
                    return Ok(id.to_string());
                }
            }
            if attempt >= INVOICE_FINALIZE_MAX_ATTEMPTS {
                return Err(AppError::BillingProviderUnavailable(
                    "Lago top-up invoice was not finalized in time; retry the top-up".to_string(),
                ));
            }
            tokio::time::sleep(Duration::from_millis(INVOICE_FINALIZE_RETRY_DELAY_MS)).await;
        }
    }

    /// Resolve the invoice id of a wallet transaction whose create response
    /// did not carry one yet (invoice attachment is asynchronous).
    async fn find_transaction_invoice_id(
        &self,
        wallet_id: &str,
        wallet_transaction_id: &str,
    ) -> AppResult<Option<String>> {
        let value = self
            .json_request(
                reqwest::Method::GET,
                &format!(
                    "wallets/{}/wallet_transactions?per_page=50",
                    urlencoding::encode(wallet_id)
                ),
                None,
            )
            .await
            .map_err(lago_error_to_app)?;
        Ok(value
            .get("wallet_transactions")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| {
                        value_string(item, &["lago_id"]).as_deref() == Some(wallet_transaction_id)
                    })
                    .and_then(|item| value_string(item, &["lago_invoice_id"]))
            }))
    }

    async fn get_wallet_by_customer_id(&self, customer_id: &str) -> AppResult<Option<LagoWallet>> {
        let value = self
            .json_request(
                reqwest::Method::GET,
                &format!(
                    "wallets?external_customer_id={}",
                    urlencoding::encode(customer_id)
                ),
                None,
            )
            .await
            .map_err(lago_error_to_app)?;
        Ok(extract_active_wallet(&value))
    }

    async fn generate_wallet_transaction_payment_url(
        &self,
        wallet_transaction_id: &str,
    ) -> AppResult<WalletTransactionPaymentDetails> {
        let path = format!(
            "wallet_transactions/{}/payment_url",
            urlencoding::encode(wallet_transaction_id)
        );
        let mut attempt = 0;
        let value = loop {
            attempt += 1;
            match self.json_request(reqwest::Method::POST, &path, None).await {
                Ok(value) => break value,
                // Lago attaches the invoice to a wallet transaction
                // asynchronously: immediately after creation, the payment
                // URL endpoint rejects with this code until Lago's worker
                // catches up. Retry briefly before surfacing the error.
                Err(error)
                    if attempt < PAYMENT_URL_MAX_ATTEMPTS
                        && error.status == Some(StatusCode::UNPROCESSABLE_ENTITY)
                        && error
                            .message
                            .contains("wallet_transaction_has_no_attached_invoice") =>
                {
                    tracing::debug!(
                        wallet_transaction_id,
                        attempt,
                        "Lago invoice not attached yet; retrying payment URL"
                    );
                    tokio::time::sleep(Duration::from_millis(PAYMENT_URL_RETRY_DELAY_MS)).await;
                }
                Err(error) => return Err(lago_error_to_app(error)),
            }
        };
        extract_wallet_transaction_payment_details(&value).ok_or_else(|| {
            AppError::BillingProviderUnavailable(
                "Lago wallet transaction payment URL response did not include a payment URL"
                    .to_string(),
            )
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct OwnerProvisionInput {
    pub external_customer_id: String,
    pub name: Option<String>,
    pub email: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LagoWallet {
    pub id: String,
    pub balance_credits: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletTopUpInput {
    pub external_id: String,
    pub amount_credits: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletTopUpCheckout {
    pub wallet_transaction_id: String,
    pub lago_invoice_id: Option<String>,
    pub payment_url: String,
    pub payment_provider: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletTopUpTransaction {
    pub wallet_transaction_id: String,
    pub lago_invoice_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LagoWalletTransaction {
    pub id: String,
    pub status: String,
    pub transaction_status: String,
    pub transaction_type: String,
    pub credit_amount_micros: i64,
    pub remaining_credit_micros: Option<i64>,
    pub name: Option<String>,
    pub settled_at: Option<chrono::DateTime<chrono::Utc>>,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WalletTransactionPaymentDetails {
    pub payment_url: String,
    pub payment_provider: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LagoEvent {
    pub transaction_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_customer_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub external_subscription_id: Option<String>,
    pub code: String,
    pub timestamp: i64,
    pub properties: LagoEventProperties,
}

impl LagoEvent {
    pub fn from_usage_row(
        row: &crate::models::usage_meter::UsageMeterRow,
        subscription_id: Option<String>,
    ) -> Option<Self> {
        let quantity = row.quantity?;
        let quantity = match row
            .funding
            .as_ref()
            .and_then(|funding| funding.lago_billable_quantity_micros)
        {
            Some(micros) => decimal_quantity_value(micros.max(0)),
            None => serde_json::Value::from(quantity.max(0)),
        };
        let properties = LagoEventProperties {
            quantity,
            model: row.model.clone(),
            service_code: row.service_slug.clone(),
            layer: Some(row.layer.as_transaction_suffix().to_string()),
        };

        Some(Self {
            transaction_id: row.transaction_id.clone(),
            external_customer_id: if subscription_id.is_some() {
                None
            } else {
                Some(row.billing_owner_id.clone())
            },
            external_subscription_id: subscription_id,
            code: row.lago_metric_code.clone(),
            timestamp: row.finalized_at.unwrap_or(row.updated_at).timestamp(),
            properties,
        })
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct LagoEventProperties {
    pub quantity: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
}

fn decimal_quantity_value(micros: i64) -> serde_json::Value {
    if micros % 1_000_000 == 0 {
        return serde_json::Value::from(micros / 1_000_000);
    }
    // Lago's event API examples encode numeric custom properties as strings,
    // and sum_agg casts the configured field to a decimal before aggregation:
    // https://getlago.com/docs/api-reference/events/create
    // Keep whole quantities as JSON numbers for backward compatibility, but
    // use a string for fractional units so no precision passes through f64.
    let whole = micros / 1_000_000;
    let fractional = micros % 1_000_000;
    serde_json::Value::String(format!("{whole}.{fractional:06}"))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LagoAck {
    pub transaction_id: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct LagoUsage {
    pub customer_id: String,
    pub subscription_id: String,
    pub raw: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entitlement {
    pub code: String,
    pub raw: Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LagoErrorKind {
    Duplicate,
    DeadLetter,
    Retry,
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LagoError {
    pub status: Option<StatusCode>,
    pub code: Option<String>,
    pub message: String,
    pub kind: LagoErrorKind,
}

impl LagoError {
    pub fn retry(message: impl Into<String>) -> Self {
        Self {
            status: None,
            code: None,
            message: message.into(),
            kind: LagoErrorKind::Retry,
        }
    }

    pub fn dead_letter(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            status: Some(StatusCode::UNPROCESSABLE_ENTITY),
            code: Some(code.into()),
            message: message.into(),
            kind: LagoErrorKind::DeadLetter,
        }
    }

    pub fn duplicate(message: impl Into<String>) -> Self {
        Self {
            status: Some(StatusCode::UNPROCESSABLE_ENTITY),
            code: Some("transaction_id_taken".to_string()),
            message: message.into(),
            kind: LagoErrorKind::Duplicate,
        }
    }

    fn from_reqwest(error: reqwest::Error) -> Self {
        Self {
            status: error.status(),
            code: None,
            message: error.to_string(),
            kind: LagoErrorKind::Unavailable,
        }
    }

    fn from_response(status: StatusCode, json: Value, raw_text: String) -> Self {
        let code = lago_error_code(&json);
        // Keep Lago's error_details in the message: a bare "Unprocessable
        // Entity" hides which field validation failed.
        let message = match (lago_error_message(&json), json.get("error_details")) {
            (Some(message), Some(details)) if !details.is_null() => {
                format!("{message}: {details}")
            }
            (Some(message), _) => message,
            (None, _) => raw_text,
        };
        let kind = classify_lago_failure(status, code.as_deref(), &json);
        Self {
            status: Some(status),
            code,
            message,
            kind,
        }
    }

    pub fn is_conflict_like(&self) -> bool {
        // A 422 is a conflict only when classification saw a duplicate
        // indicator in the body; other validation errors (e.g. a missing
        // required field) must surface instead of being read back as an
        // existing resource.
        self.status == Some(StatusCode::CONFLICT) || self.kind == LagoErrorKind::Duplicate
    }
}

pub fn classify_lago_failure(
    status: StatusCode,
    code: Option<&str>,
    body: &Value,
) -> LagoErrorKind {
    if status == StatusCode::TOO_MANY_REQUESTS || status.is_server_error() {
        return LagoErrorKind::Retry;
    }

    if status == StatusCode::UNPROCESSABLE_ENTITY {
        if matches!(code, Some("transaction_id_taken"))
            || body_contains(body, "value_already_exist")
        {
            return LagoErrorKind::Duplicate;
        }
        if matches!(
            code,
            Some(
                "billable_metric_not_found"
                    | "subscription_not_found"
                    | "customer_not_found"
                    | "invalid_subscription"
                    | "closed_period"
                    | "wallet_not_found"
            )
        ) || body_contains_any(
            body,
            &[
                "billable_metric_not_found",
                "subscription_not_found",
                "customer_not_found",
                "closed_period",
                "terminated",
            ],
        ) {
            return LagoErrorKind::DeadLetter;
        }
    }

    if status.is_client_error() {
        LagoErrorKind::DeadLetter
    } else {
        LagoErrorKind::Retry
    }
}

pub fn subscription_external_id(customer_id: &str, plan_code: &str) -> String {
    format!("{}:{}", customer_id, plan_code)
}

fn lago_error_to_app(error: LagoError) -> AppError {
    AppError::BillingProviderUnavailable(error.message)
}

fn value_string(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter()
        .find_map(|key| value.get(*key).and_then(Value::as_str))
        .map(ToString::to_string)
}

fn lago_error_code(value: &Value) -> Option<String> {
    value_string(value, &["code", "error_code"])
}

fn lago_error_message(value: &Value) -> Option<String> {
    value_string(value, &["message", "error"])
}

pub fn extract_wallet_balance_credits(value: &Value) -> Option<i64> {
    find_wallet_object(value).and_then(|wallet| {
        json_i64_path(
            wallet,
            &[
                // OSS Lago's ongoing balance refresh is premium-gated and can
                // be stale. Use the settled balance, then subtract accrued
                // current_usage in reconciliation and expiry accounting.
                "credits_balance",
                "credits_ongoing_balance",
                "credits_ongoing_usage_balance",
                "ongoing_balance",
                "balance_credits",
                "amount",
            ],
        )
    })
}

/// Parse one Lago invoice object into the trimmed summary shape.
fn invoice_summary_from_value(value: &Value) -> Option<InvoiceSummary> {
    Some(InvoiceSummary {
        lago_id: value_string(value, &["lago_id"])?,
        number: value_string(value, &["number"]).unwrap_or_default(),
        total_amount_cents: value
            .get("total_amount_cents")
            .and_then(Value::as_i64)
            .unwrap_or(0),
        status: value_string(value, &["status"]).unwrap_or_default(),
        payment_status: value_string(value, &["payment_status"]).unwrap_or_default(),
        external_customer_id: value
            .get("customer")
            .and_then(|customer| value_string(customer, &["external_id"])),
        issuing_date: value_string(value, &["issuing_date"]),
    })
}

/// Parse a Lago decimal amount string ("0.000005", "0.01", "1") into
/// micro-credits without floating point. Digits beyond micro precision are
/// truncated. Returns None for negative or malformed values.
pub fn decimal_credits_to_micros(amount: &str) -> Option<i64> {
    let amount = amount.trim();
    if amount.is_empty() {
        return None;
    }
    let (int_part, frac_part) = match amount.split_once('.') {
        Some((int_part, frac_part)) => (int_part, frac_part),
        None => (amount, ""),
    };
    if int_part.starts_with('-') || frac_part.contains('-') {
        return None;
    }
    let int_value: i64 = if int_part.is_empty() {
        0
    } else {
        int_part.parse().ok()?
    };
    if !frac_part.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    let frac_digits: String = frac_part.chars().take(6).collect();
    let frac_value: i64 = if frac_digits.is_empty() {
        0
    } else {
        let padded = format!("{frac_digits:0<6}");
        padded.parse().ok()?
    };
    int_value
        .checked_mul(1_000_000)
        .and_then(|micros| micros.checked_add(frac_value))
}

/// Extract the accrued period usage in cents from a Lago current_usage
/// response (`{"customer_usage": {"total_amount_cents": ...}}`).
pub fn extract_current_usage_amount_cents(value: &Value) -> Option<i64> {
    let usage = value.get("customer_usage").unwrap_or(value);
    usage
        .get("total_amount_cents")
        .and_then(json_i64_value)
        .or_else(|| usage.get("amount_cents").and_then(json_i64_value))
}

/// Pick the first non-terminated wallet from a Lago wallets list response.
/// Lago keeps terminated wallets in the collection and may list them before
/// the active one; wallets without a status field count as active. Returns
/// None when the response is not a wallets list.
fn active_wallet_value(value: &Value) -> Option<&Value> {
    match value.get("wallets") {
        Some(Value::Array(items)) => items
            .iter()
            .filter(|item| item.is_object())
            .find(|item| item.get("status").and_then(Value::as_str) != Some("terminated")),
        _ => None,
    }
}

/// Extract a wallet from a Lago wallets list, skipping terminated entries.
pub fn extract_active_wallet(value: &Value) -> Option<LagoWallet> {
    if matches!(value.get("wallets"), Some(Value::Array(_))) {
        return active_wallet_value(value).and_then(extract_wallet);
    }
    extract_wallet(value)
}

/// Extract a balance from a Lago wallets list, skipping terminated entries.
pub fn extract_active_wallet_balance_credits(value: &Value) -> Option<i64> {
    if matches!(value.get("wallets"), Some(Value::Array(_))) {
        return active_wallet_value(value).and_then(extract_wallet_balance_credits);
    }
    extract_wallet_balance_credits(value)
}

fn extract_active_wallet_balance_micros(value: &Value) -> Option<i64> {
    let wallet = if matches!(value.get("wallets"), Some(Value::Array(_))) {
        active_wallet_value(value)?
    } else {
        find_wallet_object(value)?
    };
    [
        "credits_balance",
        "credits_ongoing_balance",
        "credits_ongoing_usage_balance",
        "ongoing_balance",
        "balance_credits",
        "amount",
    ]
    .iter()
    .find_map(|key| match wallet.get(*key)? {
        Value::String(value) => decimal_credits_to_micros(value),
        Value::Number(value) => decimal_credits_to_micros(&value.to_string()),
        _ => None,
    })
}

pub fn extract_wallet(value: &Value) -> Option<LagoWallet> {
    let wallet = find_wallet_object(value)?;
    let id =
        value_string(wallet, &["id", "lago_id", "wallet_id", "lago_wallet_id"]).or_else(|| {
            find_string_by_keys(wallet, &["id", "lago_id", "wallet_id", "lago_wallet_id"])
        })?;
    let balance_credits = extract_wallet_balance_credits(wallet).unwrap_or(0);
    Some(LagoWallet {
        id,
        balance_credits,
    })
}

pub fn extract_wallet_topup_transaction(value: &Value) -> Option<WalletTopUpTransaction> {
    let transaction = find_wallet_transaction_object(value).unwrap_or(value);
    let wallet_transaction_id = find_string_by_keys(
        transaction,
        &[
            "id",
            "lago_id",
            "wallet_transaction_id",
            "lago_wallet_transaction_id",
        ],
    )?;
    let lago_invoice_id = find_string_by_keys(
        transaction,
        &[
            "lago_invoice_id",
            "invoice_id",
            "invoice_lago_id",
            "lago_invoice_external_id",
        ],
    );

    Some(WalletTopUpTransaction {
        wallet_transaction_id,
        lago_invoice_id,
    })
}

fn wallet_transaction_from_value(value: &Value) -> Option<LagoWalletTransaction> {
    let created_at = value_string(value, &["created_at"])?
        .parse::<chrono::DateTime<chrono::Utc>>()
        .ok()?;
    Some(LagoWalletTransaction {
        id: value_string(value, &["lago_id", "id"])?,
        status: value_string(value, &["status"]).unwrap_or_default(),
        transaction_status: value_string(value, &["transaction_status"]).unwrap_or_default(),
        transaction_type: value_string(value, &["transaction_type"]).unwrap_or_default(),
        credit_amount_micros: value_string(value, &["credit_amount"])
            .as_deref()
            .and_then(decimal_credits_to_micros)
            .unwrap_or(0),
        remaining_credit_micros: value.get("remaining_credit_amount").and_then(|remaining| {
            match remaining {
                Value::Null => None,
                Value::String(value) => decimal_credits_to_micros(value),
                Value::Number(value) => decimal_credits_to_micros(&value.to_string()),
                _ => None,
            }
        }),
        name: value_string(value, &["name"]),
        settled_at: value_string(value, &["settled_at"])
            .and_then(|value| value.parse::<chrono::DateTime<chrono::Utc>>().ok()),
        created_at,
    })
}

pub fn purchased_credit_expiry_transaction_name(operation_id: &str) -> String {
    format!("NyxID purchased-credit expiry {operation_id}")
}

fn micros_to_lago_credits(micros: i64) -> String {
    let micros = micros.max(0) / 10 * 10;
    let whole = micros / 1_000_000;
    let fractional = (micros % 1_000_000) / 10;
    format!("{whole}.{fractional:05}")
}

pub fn extract_wallet_transaction_payment_details(
    value: &Value,
) -> Option<WalletTransactionPaymentDetails> {
    let details = value.get("wallet_transaction_payment_details")?;
    let payment_url = value_string(details, &["payment_url"])?;
    let payment_provider = value_string(details, &["payment_provider"])?;

    Some(WalletTransactionPaymentDetails {
        payment_url,
        payment_provider,
    })
}

fn find_wallet_object(value: &Value) -> Option<&Value> {
    match value {
        Value::Object(map) => {
            if map.keys().any(|key| {
                matches!(
                    key.as_str(),
                    "credits_balance"
                        | "credits_ongoing_balance"
                        | "credits_ongoing_usage_balance"
                        | "ongoing_balance"
                        | "balance_credits"
                        | "amount"
                )
            }) {
                return Some(value);
            }

            if let Some(wallet) = map.get("wallet") {
                if wallet.is_object() {
                    return Some(wallet);
                }
                if let Some(found) = find_wallet_object(wallet) {
                    return Some(found);
                }
            }
            if let Some(wallets) = map.get("wallets") {
                match wallets {
                    Value::Array(items) => {
                        if let Some(wallet) = items.iter().find(|item| item.is_object()) {
                            return Some(wallet);
                        }
                    }
                    Value::Object(_) => return Some(wallets),
                    _ => {}
                }
                if let Some(found) = find_wallet_object(wallets) {
                    return Some(found);
                }
            }

            map.values().find_map(find_wallet_object)
        }
        Value::Array(items) => items.iter().find_map(find_wallet_object),
        _ => None,
    }
}

fn find_wallet_transaction_object(value: &Value) -> Option<&Value> {
    match value {
        Value::Object(map) => {
            if map.keys().any(|key| {
                matches!(
                    key.as_str(),
                    "payment_url"
                        | "checkout_url"
                        | "hosted_payment_url"
                        | "hosted_invoice_url"
                        | "invoice_url"
                        | "lago_invoice_id"
                        | "invoice_id"
                        | "transaction_status"
                        | "transaction_type"
                        | "credit_amount"
                        | "invoice_requires_successful_payment"
                )
            }) {
                return Some(value);
            }

            for key in ["wallet_transaction", "wallet_transactions", "transaction"] {
                if let Some(found) = map.get(key).and_then(find_wallet_transaction_object) {
                    return Some(found);
                }
            }

            map.values().find_map(find_wallet_transaction_object)
        }
        Value::Array(items) => items.iter().find_map(find_wallet_transaction_object),
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

fn json_i64_path(value: &Value, keys: &[&str]) -> Option<i64> {
    match value {
        Value::Object(map) => {
            for key in keys {
                if let Some(parsed) = map.get(*key).and_then(json_i64_value) {
                    return Some(parsed);
                }
            }
            for key in keys {
                if let Some(parsed) = map.get(*key).and_then(|inner| json_i64_path(inner, keys)) {
                    return Some(parsed);
                }
            }
            None
        }
        _ => json_i64_value(value),
    }
}

fn json_i64_value(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|value| value.round() as i64)),
        Value::String(value) => value.parse::<i64>().ok().or_else(|| {
            value
                .parse::<f64>()
                .ok()
                .map(|parsed| parsed.round() as i64)
        }),
        Value::Object(map) => map.values().find_map(json_i64_value),
        _ => None,
    }
}

fn body_contains(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(s) => s.eq_ignore_ascii_case(needle) || s.contains(needle),
        Value::Array(items) => items.iter().any(|item| body_contains(item, needle)),
        Value::Object(map) => map
            .iter()
            .any(|(key, value)| key.contains(needle) || body_contains(value, needle)),
        _ => false,
    }
}

fn body_contains_any(value: &Value, needles: &[&str]) -> bool {
    needles.iter().any(|needle| body_contains(value, needle))
}

#[cfg(test)]
mod tests {
    use reqwest::StatusCode;
    use serde_json::{Value, json};

    use super::{
        LagoApi, LagoClient, LagoError, LagoErrorKind, LagoEvent, LagoEventProperties,
        OwnerProvisionInput, PlanRate, ServicePriceSync, WALLET_TRANSACTION_PAGE_SIZE,
        classify_lago_failure, decimal_credits_to_micros, extract_active_wallet,
        extract_wallet_balance_credits, subscription_external_id,
    };

    async fn spawn_lago_mock(app: axum::Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind mock Lago listener");
        let addr = listener.local_addr().expect("mock Lago addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("mock Lago server");
        });
        format!("http://{addr}")
    }

    fn service_price_sync() -> ServicePriceSync {
        ServicePriceSync {
            metric_code: "platform_svc_llm-openai".to_string(),
            metric_name: "OpenAI platform usage".to_string(),
            metric_description: "NyxID-managed platform usage price".to_string(),
            credits_per_unit: "0.125".to_string(),
        }
    }

    fn plan_response(charges: serde_json::Value) -> serde_json::Value {
        json!({
            "plan": {
                "lago_id": "plan-id",
                "created_at": "2026-08-22T00:00:00Z",
                "name": "Standard",
                "code": "standard",
                "interval": "monthly",
                "amount_cents": 2500,
                "amount_currency": "USD",
                "pay_in_advance": false,
                "bill_charges_monthly": true,
                "customers_count": 12,
                "fixed_charges": [{
                    "lago_id": "fixed-charge-id",
                    "add_on_code": "support"
                }],
                "charges": charges,
            }
        })
    }

    #[tokio::test]
    async fn service_price_sync_creates_missing_metric_and_charge() {
        async fn metric_missing() -> axum::http::StatusCode {
            axum::http::StatusCode::NOT_FOUND
        }
        async fn create_metric(
            axum::Json(body): axum::Json<serde_json::Value>,
        ) -> axum::Json<serde_json::Value> {
            assert_eq!(body["billable_metric"]["code"], "platform_svc_llm-openai");
            assert_eq!(body["billable_metric"]["aggregation_type"], "sum_agg");
            assert_eq!(body["billable_metric"]["field_name"], "quantity");
            axum::Json(json!({
                "billable_metric": {
                    "lago_id": "metric-1",
                    "aggregation_type": "sum_agg",
                    "field_name": "quantity"
                }
            }))
        }
        async fn update_metric() -> axum::Json<serde_json::Value> {
            axum::Json(json!({ "billable_metric": { "lago_id": "metric-1" } }))
        }
        async fn get_plan() -> axum::Json<serde_json::Value> {
            axum::Json(plan_response(json!([{
                "lago_id": "legacy-charge-id",
                "lago_billable_metric_id": "legacy-metric-id",
                "billable_metric_code": "platform_tokens",
                "charge_model": "standard",
                "invoiceable": true,
                "properties": { "amount": "0.000005" },
                "filters": [{ "key": "model", "values": ["gpt-5"] }]
            }])))
        }
        async fn update_plan(
            axum::Json(body): axum::Json<serde_json::Value>,
        ) -> axum::Json<serde_json::Value> {
            let plan = &body["plan"];
            assert_eq!(plan["name"], "Standard");
            assert_eq!(plan["code"], "standard");
            assert_eq!(plan["interval"], "monthly");
            assert_eq!(plan["amount_cents"], 2500);
            assert_eq!(plan["amount_currency"], "USD");
            assert_eq!(plan["bill_charges_monthly"], true);
            assert!(plan.get("lago_id").is_none());
            assert!(plan.get("created_at").is_none());
            assert!(plan.get("customers_count").is_none());
            assert!(plan.get("fixed_charges").is_none());
            let charges = plan["charges"].as_array().expect("charges array");
            assert_eq!(charges.len(), 2);
            assert_eq!(charges[0]["id"], "legacy-charge-id");
            assert_eq!(charges[0]["lago_billable_metric_id"], "legacy-metric-id");
            assert_eq!(charges[0]["billable_metric_code"], "platform_tokens");
            assert_eq!(charges[0]["properties"]["amount"], "0.000005");
            assert_eq!(charges[0]["filters"][0]["values"][0], "gpt-5");
            assert!(charges[1].get("id").is_none());
            assert_eq!(charges[1]["billable_metric_id"], "metric-1");
            assert_eq!(charges[1]["charge_model"], "standard");
            assert_eq!(charges[1]["properties"]["amount"], "0.125");
            axum::Json(plan_response(Value::Array(charges.clone())))
        }

        let base_url = spawn_lago_mock(
            axum::Router::new()
                .route(
                    "/api/v1/billable_metrics/platform_svc_llm-openai",
                    axum::routing::get(metric_missing).put(update_metric),
                )
                .route(
                    "/api/v1/billable_metrics",
                    axum::routing::post(create_metric),
                )
                .route(
                    "/api/v1/plans/standard",
                    axum::routing::get(get_plan).put(update_plan),
                ),
        )
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        client
            .sync_standard_charge("standard", &service_price_sync())
            .await
            .expect("sync service price");
    }

    #[tokio::test]
    async fn service_price_sync_updates_existing_metric_and_charge() {
        async fn get_metric() -> axum::Json<serde_json::Value> {
            axum::Json(json!({
                "billable_metric": {
                    "lago_id": "metric-1",
                    "aggregation_type": "sum_agg",
                    "field_name": "quantity"
                }
            }))
        }
        async fn update_metric(
            axum::Json(body): axum::Json<serde_json::Value>,
        ) -> axum::Json<serde_json::Value> {
            assert_eq!(body["billable_metric"]["name"], "OpenAI platform usage");
            axum::Json(json!({ "billable_metric": { "lago_id": "metric-1" } }))
        }
        async fn get_plan() -> axum::Json<serde_json::Value> {
            axum::Json(plan_response(json!([
                {
                    "id": "unrelated-charge-id",
                    "lago_billable_metric_id": "unrelated-metric-id",
                    "billable_metric_code": "platform_requests",
                    "charge_model": "standard",
                    "properties": { "amount": "0.01" }
                },
                {
                    "lago_id": "existing-charge-id",
                    "lago_billable_metric_id": "metric-1",
                    "billable_metric_code": "platform_svc_llm-openai",
                    "charge_model": "standard",
                    "properties": { "amount": "0.25" },
                    "invoiceable": false
                }
            ])))
        }
        async fn update_plan(
            axum::Json(body): axum::Json<serde_json::Value>,
        ) -> axum::Json<serde_json::Value> {
            let plan = &body["plan"];
            assert_eq!(plan["name"], "Standard");
            assert_eq!(plan["amount_currency"], "USD");
            let charges = plan["charges"].as_array().expect("charges array");
            assert_eq!(charges.len(), 2);
            assert_eq!(charges[0]["id"], "unrelated-charge-id");
            assert_eq!(charges[0]["properties"]["amount"], "0.01");
            assert_eq!(charges[1]["id"], "existing-charge-id");
            assert_eq!(charges[1]["properties"]["amount"], "0.125");
            assert_eq!(charges[1]["invoiceable"], true);
            axum::Json(plan_response(Value::Array(charges.clone())))
        }

        let base_url = spawn_lago_mock(
            axum::Router::new()
                .route(
                    "/api/v1/billable_metrics/platform_svc_llm-openai",
                    axum::routing::get(get_metric).put(update_metric),
                )
                .route(
                    "/api/v1/plans/standard",
                    axum::routing::get(get_plan).put(update_plan),
                ),
        )
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        client
            .sync_standard_charge("standard", &service_price_sync())
            .await
            .expect("sync service price");
    }

    #[tokio::test]
    async fn service_price_removal_preserves_unrelated_plan_charges() {
        async fn get_plan() -> axum::Json<serde_json::Value> {
            axum::Json(plan_response(json!([
                {
                    "lago_id": "legacy-charge-id",
                    "billable_metric_code": "platform_tokens",
                    "charge_model": "standard",
                    "properties": { "amount": "0.000005" }
                },
                {
                    "lago_id": "owned-charge-id",
                    "billable_metric_code": "platform_svc_llm-openai",
                    "charge_model": "standard",
                    "properties": { "amount": "0.125" }
                }
            ])))
        }
        async fn update_plan(
            axum::Json(body): axum::Json<serde_json::Value>,
        ) -> axum::Json<serde_json::Value> {
            let plan = &body["plan"];
            assert_eq!(plan["name"], "Standard");
            assert_eq!(plan["interval"], "monthly");
            let charges = plan["charges"].as_array().expect("charges array");
            assert_eq!(charges.len(), 1);
            assert_eq!(charges[0]["id"], "legacy-charge-id");
            assert_eq!(charges[0]["billable_metric_code"], "platform_tokens");
            axum::Json(plan_response(Value::Array(charges.clone())))
        }
        let base_url = spawn_lago_mock(axum::Router::new().route(
            "/api/v1/plans/standard",
            axum::routing::get(get_plan).put(update_plan),
        ))
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        client
            .remove_standard_charge("standard", "platform_svc_llm-openai")
            .await
            .expect("remove service price");
    }

    #[tokio::test]
    async fn service_price_sync_rejects_incompatible_existing_metric() {
        async fn get_metric() -> axum::Json<serde_json::Value> {
            axum::Json(json!({
                "billable_metric": {
                    "lago_id": "metric-1",
                    "aggregation_type": "count_agg",
                    "field_name": null
                }
            }))
        }
        let base_url = spawn_lago_mock(axum::Router::new().route(
            "/api/v1/billable_metrics/platform_svc_llm-openai",
            axum::routing::get(get_metric),
        ))
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let error = client
            .sync_standard_charge("standard", &service_price_sync())
            .await
            .expect_err("incompatible metric must fail");

        assert!(error.to_string().contains("incompatible aggregation"));
    }

    #[test]
    fn duplicate_transaction_is_success_class() {
        let body = json!({
            "status": 422,
            "code": "validation_errors",
            "error_details": {
                "0": { "transaction_id": ["value_already_exist"] }
            }
        });

        assert_eq!(
            classify_lago_failure(
                StatusCode::UNPROCESSABLE_ENTITY,
                Some("validation_errors"),
                &body
            ),
            LagoErrorKind::Duplicate
        );
    }

    #[tokio::test]
    async fn payment_url_retries_until_lago_attaches_the_invoice() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let handler_calls = calls.clone();
        let base_url = spawn_lago_mock(axum::Router::new().route(
            "/api/v1/wallet_transactions/txn-race/payment_url",
            axum::routing::post(move || {
                let calls = handler_calls.clone();
                async move {
                    if calls.fetch_add(1, AtomicOrdering::SeqCst) < 2 {
                        (
                            axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                            axum::Json(json!({
                                "status": 422,
                                "error": "Unprocessable Entity",
                                "code": "validation_errors",
                                "error_details": {
                                    "base": ["wallet_transaction_has_no_attached_invoice"]
                                }
                            })),
                        )
                    } else {
                        (
                            axum::http::StatusCode::OK,
                            axum::Json(json!({
                                "wallet_transaction_payment_details": {
                                    "payment_url": "https://pay.example/ready",
                                    "payment_provider": "stripe"
                                }
                            })),
                        )
                    }
                }
            }),
        ))
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let details = client
            .generate_wallet_transaction_payment_url("txn-race")
            .await
            .expect("payment url after retries");

        assert_eq!(details.payment_url, "https://pay.example/ready");
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 3);
    }

    #[tokio::test]
    async fn payment_url_does_not_retry_other_validation_errors() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let handler_calls = calls.clone();
        let base_url = spawn_lago_mock(axum::Router::new().route(
            "/api/v1/wallet_transactions/txn-bad/payment_url",
            axum::routing::post(move || {
                let calls = handler_calls.clone();
                async move {
                    calls.fetch_add(1, AtomicOrdering::SeqCst);
                    (
                        axum::http::StatusCode::UNPROCESSABLE_ENTITY,
                        axum::Json(json!({
                            "status": 422,
                            "error": "Unprocessable Entity",
                            "code": "validation_errors",
                            "error_details": { "base": ["no_linked_payment_provider"] }
                        })),
                    )
                }
            }),
        ))
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let error = client
            .generate_wallet_transaction_payment_url("txn-bad")
            .await
            .expect_err("non-race validation errors fail fast");

        assert!(error.to_string().contains("no_linked_payment_provider"));
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
    }

    #[tokio::test]
    async fn credit_invoices_parse_payment_outcomes_and_customer() {
        async fn list_invoices(
            axum::extract::Query(query): axum::extract::Query<
                std::collections::HashMap<String, String>,
            >,
        ) -> axum::Json<serde_json::Value> {
            assert_eq!(
                query.get("external_customer_id").map(String::as_str),
                Some("owner-1")
            );
            assert_eq!(
                query.get("invoice_type").map(String::as_str),
                Some("credit")
            );
            axum::Json(json!({
                "invoices": [
                    {
                        "lago_id": "inv-paid",
                        "number": "CHR-001-001",
                        "total_amount_cents": 100,
                        "status": "finalized",
                        "payment_status": "succeeded",
                        "issuing_date": "2026-07-29",
                        "customer": { "external_id": "owner-1" }
                    },
                    {
                        "lago_id": "inv-open",
                        "number": "CHR-001-002",
                        "total_amount_cents": 10000,
                        "status": "finalized",
                        "payment_status": "pending",
                        "customer": { "external_id": "owner-1" }
                    }
                ]
            }))
        }

        let base_url = spawn_lago_mock(
            axum::Router::new().route("/api/v1/invoices", axum::routing::get(list_invoices)),
        )
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let invoices = client.credit_invoices("owner-1").await.expect("invoices");

        assert_eq!(invoices.len(), 2);
        assert_eq!(invoices[0].lago_id, "inv-paid");
        assert_eq!(invoices[0].payment_status, "succeeded");
        assert_eq!(invoices[0].external_customer_id.as_deref(), Some("owner-1"));
        assert_eq!(invoices[1].total_amount_cents, 10000);
    }

    #[tokio::test]
    async fn invoice_download_retries_until_the_pdf_exists() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        let calls = std::sync::Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        let base_url = spawn_lago_mock(axum::Router::new().route(
            "/api/v1/invoices/inv-1/download",
            axum::routing::post(move || {
                let calls = counter.clone();
                async move {
                    let file_url = if calls.fetch_add(1, AtomicOrdering::SeqCst) < 1 {
                        serde_json::Value::Null
                    } else {
                        json!("https://lago.example/receipt.pdf")
                    };
                    axum::Json(json!({
                        "invoice": { "lago_id": "inv-1", "file_url": file_url }
                    }))
                }
            }),
        ))
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let url = client
            .invoice_download_url("inv-1")
            .await
            .expect("download url");

        assert_eq!(url.as_deref(), Some("https://lago.example/receipt.pdf"));
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 2);
    }

    #[test]
    fn decimal_credits_parse_to_micros_without_float_drift() {
        assert_eq!(decimal_credits_to_micros("0.000005"), Some(5));
        assert_eq!(decimal_credits_to_micros("0.01"), Some(10_000));
        assert_eq!(decimal_credits_to_micros("1"), Some(1_000_000));
        assert_eq!(decimal_credits_to_micros("2.5"), Some(2_500_000));
        assert_eq!(decimal_credits_to_micros(" 0.25 "), Some(250_000));
        // Sub-micro digits truncate; malformed and negative values reject.
        assert_eq!(decimal_credits_to_micros("0.0000019"), Some(1));
        assert_eq!(decimal_credits_to_micros("-1"), None);
        assert_eq!(decimal_credits_to_micros("abc"), None);
        assert_eq!(decimal_credits_to_micros("1.2x"), None);
        assert_eq!(decimal_credits_to_micros("1.123456x"), None);
        assert_eq!(decimal_credits_to_micros(""), None);
        assert_eq!(
            decimal_credits_to_micros("1000000001.25"),
            Some(1_000_000_001_250_000)
        );
    }

    #[tokio::test]
    async fn plan_rates_parse_standard_charges_and_skip_others() {
        async fn get_plan() -> axum::Json<serde_json::Value> {
            axum::Json(json!({
                "plan": {
                    "code": "starter",
                    "charges": [
                        {
                            "billable_metric_code": "platform_tokens",
                            "charge_model": "standard",
                            "properties": { "amount": "0.000005" }
                        },
                        {
                            "billable_metric_code": "platform_requests",
                            "charge_model": "standard",
                            "properties": { "amount": "0.01" }
                        },
                        {
                            "billable_metric_code": "resale_tokens",
                            "charge_model": "graduated",
                            "properties": {}
                        }
                    ]
                }
            }))
        }

        let base_url = spawn_lago_mock(
            axum::Router::new().route("/api/v1/plans/starter", axum::routing::get(get_plan)),
        )
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let rates = client.plan_rates("starter").await.expect("plan rates");

        assert_eq!(
            rates,
            vec![
                PlanRate {
                    lago_metric_code: "platform_tokens".to_string(),
                    credits_per_unit_micros: 5,
                },
                PlanRate {
                    lago_metric_code: "platform_requests".to_string(),
                    credits_per_unit_micros: 10_000,
                },
            ]
        );
    }

    #[test]
    fn validation_error_without_duplicate_indicator_is_not_conflict_like() {
        let body = json!({
            "status": 422,
            "error": "Unprocessable Entity",
            "code": "validation_errors",
            "error_details": { "rate_amount": ["is not a number"] }
        });
        let error = LagoError::from_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            body.clone(),
            body.to_string(),
        );

        assert!(!error.is_conflict_like());
    }

    #[test]
    fn validation_error_with_duplicate_indicator_is_conflict_like() {
        let body = json!({
            "status": 422,
            "error": "Unprocessable Entity",
            "code": "validation_errors",
            "error_details": { "external_id": ["value_already_exist"] }
        });
        let error = LagoError::from_response(
            StatusCode::UNPROCESSABLE_ENTITY,
            body.clone(),
            body.to_string(),
        );

        assert!(error.is_conflict_like());
    }

    #[test]
    fn extract_active_wallet_skips_terminated_wallets() {
        let body = json!({
            "wallets": [
                { "lago_id": "w-terminated", "status": "terminated", "credits_balance": "5.0" },
                { "lago_id": "w-active", "status": "active", "credits_balance": "3.0" }
            ]
        });

        let wallet = extract_active_wallet(&body).expect("active wallet");
        assert_eq!(wallet.id, "w-active");

        let all_terminated = json!({
            "wallets": [
                { "lago_id": "w-terminated", "status": "terminated", "credits_balance": "5.0" }
            ]
        });
        assert!(extract_active_wallet(&all_terminated).is_none());
    }

    #[test]
    fn missing_billable_metric_is_dead_letter_class() {
        let body = json!({ "code": "billable_metric_not_found" });

        assert_eq!(
            classify_lago_failure(
                StatusCode::UNPROCESSABLE_ENTITY,
                Some("billable_metric_not_found"),
                &body
            ),
            LagoErrorKind::DeadLetter
        );
    }

    #[test]
    fn rate_limit_and_server_errors_retry() {
        assert_eq!(
            classify_lago_failure(StatusCode::TOO_MANY_REQUESTS, None, &json!({})),
            LagoErrorKind::Retry
        );
        assert_eq!(
            classify_lago_failure(StatusCode::BAD_GATEWAY, None, &json!({})),
            LagoErrorKind::Retry
        );
    }

    #[test]
    fn wallet_balance_extracts_common_lago_shapes() {
        assert_eq!(
            extract_wallet_balance_credits(&json!({
                "wallet": { "credits_balance": "42.4" }
            })),
            Some(42)
        );
        assert_eq!(
            extract_wallet_balance_credits(&json!({
                "wallets": [{ "credits_ongoing_balance": "12.0" }]
            })),
            Some(12)
        );
        assert_eq!(
            extract_wallet_balance_credits(&json!({
                "wallets": [{
                    "credits_balance": "9.0",
                    "credits_ongoing_balance": "99.0"
                }]
            })),
            Some(9),
            "OSS settled balance must win over stale premium ongoing balance"
        );
    }

    #[tokio::test]
    async fn ensure_customer_gets_existing_customer_before_create() {
        async fn get_customer() -> axum::Json<serde_json::Value> {
            axum::Json(json!({
                "customer": { "external_id": "owner-1" }
            }))
        }

        async fn create_customer() -> axum::http::StatusCode {
            axum::http::StatusCode::INTERNAL_SERVER_ERROR
        }

        let base_url = spawn_lago_mock(
            axum::Router::new()
                .route(
                    "/api/v1/customers/owner-1",
                    axum::routing::get(get_customer),
                )
                .route("/api/v1/customers", axum::routing::post(create_customer)),
        )
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let customer_id = client
            .ensure_customer(&OwnerProvisionInput {
                external_customer_id: "owner-1".to_string(),
                name: Some("Owner One".to_string()),
                email: None,
            })
            .await
            .expect("ensure customer");

        assert_eq!(customer_id, "owner-1");
    }

    #[tokio::test]
    async fn ensure_customer_links_payment_provider_when_configured() {
        async fn get_customer() -> axum::http::StatusCode {
            axum::http::StatusCode::NOT_FOUND
        }

        async fn create_customer(
            axum::Json(body): axum::Json<serde_json::Value>,
        ) -> axum::Json<serde_json::Value> {
            let billing = &body["customer"]["billing_configuration"];
            assert_eq!(billing["payment_provider"].as_str(), Some("stripe"));
            assert_eq!(billing["payment_provider_code"].as_str(), Some("sandbox"));
            assert_eq!(billing["sync_with_provider"].as_bool(), Some(true));
            axum::Json(json!({ "customer": { "external_id": "owner-1" } }))
        }

        let base_url = spawn_lago_mock(
            axum::Router::new()
                .route(
                    "/api/v1/customers/owner-1",
                    axum::routing::get(get_customer),
                )
                .route("/api/v1/customers", axum::routing::post(create_customer)),
        )
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string())
            .expect("client")
            .with_payment_provider_code(Some("sandbox".to_string()));

        let customer_id = client
            .ensure_customer(&OwnerProvisionInput {
                external_customer_id: "owner-1".to_string(),
                name: Some("Owner One".to_string()),
                email: None,
            })
            .await
            .expect("ensure customer");

        assert_eq!(customer_id, "owner-1");
    }

    #[tokio::test]
    async fn wallet_balance_reads_by_external_customer_id() {
        async fn get_wallet(
            axum::extract::Query(query): axum::extract::Query<
                std::collections::HashMap<String, String>,
            >,
        ) -> axum::Json<serde_json::Value> {
            assert_eq!(
                query.get("external_customer_id").map(String::as_str),
                Some("owner-1")
            );
            axum::Json(json!({
                "wallets": [{ "credits_balance": "77" }]
            }))
        }

        let base_url = spawn_lago_mock(
            axum::Router::new().route("/api/v1/wallets", axum::routing::get(get_wallet)),
        )
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let balance = client
            .wallet_balance("owner-1")
            .await
            .expect("wallet balance");

        assert_eq!(balance, 77);
    }

    #[tokio::test]
    async fn create_wallet_topup_does_not_grant_free_credits() {
        async fn create_topup(
            axum::Json(body): axum::Json<serde_json::Value>,
        ) -> axum::Json<serde_json::Value> {
            let wt = &body["wallet_transaction"];
            // Bug #1050: granted_credits are FREE/promotional in Lago, so a paid
            // top-up must grant ONLY the paid amount — granted_credits MUST be 0,
            // otherwise the customer receives 2N credits for an N payment.
            // Amounts are decimal strings: Lago rejects JSON numbers.
            assert_eq!(wt["paid_credits"].as_str(), Some("500"), "paid_credits");
            assert_eq!(
                wt["granted_credits"].as_str(),
                Some("0"),
                "granted_credits must be 0 for a paid top-up"
            );
            assert_eq!(
                wt["invoice_requires_successful_payment"].as_bool(),
                Some(true),
                "paid top-up must require successful payment before settlement"
            );
            axum::Json(json!({
                "wallet_transaction": {
                    "id": "txn-1",
                    "lago_invoice_id": "invoice-1"
                }
            }))
        }

        async fn generate_payment_url() -> axum::Json<serde_json::Value> {
            axum::Json(json!({
                "wallet_transaction_payment_details": {
                    "payment_url": "https://pay.example/checkout",
                    "payment_provider": "stripe"
                }
            }))
        }

        let base_url = spawn_lago_mock(
            axum::Router::new()
                .route(
                    "/api/v1/wallet_transactions",
                    axum::routing::post(create_topup),
                )
                .route(
                    "/api/v1/wallet_transactions/txn-1/payment_url",
                    axum::routing::post(generate_payment_url),
                )
                .route(
                    "/api/v1/invoices/invoice-1",
                    axum::routing::get(|| async {
                        axum::Json(json!({
                            "invoice": { "lago_id": "invoice-1", "status": "finalized" }
                        }))
                    }),
                ),
        )
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let checkout = client
            .create_wallet_topup(
                "wallet-1",
                &super::WalletTopUpInput {
                    external_id: "topup-1".to_string(),
                    amount_credits: 500,
                },
            )
            .await
            .expect("create wallet topup");

        assert_eq!(checkout.payment_url, "https://pay.example/checkout");
        assert_eq!(checkout.lago_invoice_id.as_deref(), Some("invoice-1"));
        assert_eq!(checkout.payment_provider.as_deref(), Some("stripe"));
    }

    #[tokio::test]
    async fn create_wallet_topup_waits_for_invoice_finalization() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        let invoice_polls = std::sync::Arc::new(AtomicUsize::new(0));
        let finalized_before_url = std::sync::Arc::new(AtomicUsize::new(0));
        let polls = invoice_polls.clone();
        let gate = finalized_before_url.clone();
        let gate_check = finalized_before_url.clone();

        let base_url = spawn_lago_mock(
            axum::Router::new()
                .route(
                    "/api/v1/wallet_transactions",
                    // Create response carries no invoice id yet: attachment
                    // is asynchronous on the Lago side.
                    axum::routing::post(|| async {
                        axum::Json(json!({
                            "wallet_transaction": { "id": "txn-wait" }
                        }))
                    }),
                )
                .route(
                    "/api/v1/wallets/wallet-1/wallet_transactions",
                    axum::routing::get(|| async {
                        axum::Json(json!({
                            "wallet_transactions": [
                                { "lago_id": "txn-wait", "lago_invoice_id": "invoice-wait" }
                            ]
                        }))
                    }),
                )
                .route(
                    "/api/v1/invoices/invoice-wait",
                    axum::routing::get(move || {
                        let polls = polls.clone();
                        let gate = gate.clone();
                        async move {
                            let status = if polls.fetch_add(1, AtomicOrdering::SeqCst) < 1 {
                                "draft"
                            } else {
                                gate.store(1, AtomicOrdering::SeqCst);
                                "finalized"
                            };
                            axum::Json(json!({
                                "invoice": { "lago_id": "invoice-wait", "status": status }
                            }))
                        }
                    }),
                )
                .route(
                    "/api/v1/wallet_transactions/txn-wait/payment_url",
                    axum::routing::post(move || {
                        let gate = gate_check.clone();
                        async move {
                            assert_eq!(
                                gate.load(AtomicOrdering::SeqCst),
                                1,
                                "payment URL must not be requested before finalization"
                            );
                            axum::Json(json!({
                                "wallet_transaction_payment_details": {
                                    "payment_url": "https://pay.example/finalized",
                                    "payment_provider": "stripe"
                                }
                            }))
                        }
                    }),
                ),
        )
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let checkout = client
            .create_wallet_topup(
                "wallet-1",
                &super::WalletTopUpInput {
                    external_id: "topup-wait".to_string(),
                    amount_credits: 1,
                },
            )
            .await
            .expect("create wallet topup");

        assert_eq!(checkout.payment_url, "https://pay.example/finalized");
        assert_eq!(checkout.lago_invoice_id.as_deref(), Some("invoice-wait"));
        assert!(invoice_polls.load(AtomicOrdering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn create_wallet_topup_never_mints_a_url_for_an_unfinalized_invoice() {
        use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};

        let url_calls = std::sync::Arc::new(AtomicUsize::new(0));
        let url_counter = url_calls.clone();

        let base_url = spawn_lago_mock(
            axum::Router::new()
                .route(
                    "/api/v1/wallet_transactions",
                    axum::routing::post(|| async {
                        axum::Json(json!({
                            "wallet_transaction": {
                                "id": "txn-stuck",
                                "lago_invoice_id": "invoice-stuck"
                            }
                        }))
                    }),
                )
                .route(
                    "/api/v1/invoices/invoice-stuck",
                    axum::routing::get(|| async {
                        axum::Json(json!({
                            "invoice": { "lago_id": "invoice-stuck", "status": "draft" }
                        }))
                    }),
                )
                .route(
                    "/api/v1/wallet_transactions/txn-stuck/payment_url",
                    axum::routing::post(move || {
                        let calls = url_counter.clone();
                        async move {
                            calls.fetch_add(1, AtomicOrdering::SeqCst);
                            axum::Json(json!({}))
                        }
                    }),
                ),
        )
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let error = client
            .create_wallet_topup(
                "wallet-1",
                &super::WalletTopUpInput {
                    external_id: "topup-stuck".to_string(),
                    amount_credits: 1,
                },
            )
            .await
            .expect_err("must fail when the invoice never finalizes");

        assert!(error.to_string().contains("not finalized"));
        assert_eq!(
            url_calls.load(AtomicOrdering::SeqCst),
            0,
            "a draft invoice must never get a checkout session"
        );
    }

    #[tokio::test]
    async fn create_wallet_topup_rejects_undocumented_payment_url_response_shape() {
        async fn create_topup(
            axum::Json(body): axum::Json<serde_json::Value>,
        ) -> axum::Json<serde_json::Value> {
            let wt = &body["wallet_transaction"];
            assert_eq!(wt["paid_credits"].as_str(), Some("500"), "paid_credits");
            assert_eq!(
                wt["granted_credits"].as_str(),
                Some("0"),
                "granted_credits must be 0 for a paid top-up"
            );
            assert_eq!(
                wt["invoice_requires_successful_payment"].as_bool(),
                Some(true),
                "paid top-up must require successful payment before settlement"
            );
            axum::Json(json!({
                "wallet_transaction": {
                    "lago_id": "txn-1",
                    "lago_invoice_id": "invoice-1"
                }
            }))
        }

        async fn generate_payment_url() -> axum::Json<serde_json::Value> {
            axum::Json(json!({
                "wallet_transaction_payment_details": {
                    "checkout_url": "https://pay.example/generated",
                    "payment_provider_code": "stripe"
                }
            }))
        }

        let base_url = spawn_lago_mock(
            axum::Router::new()
                .route(
                    "/api/v1/wallet_transactions",
                    axum::routing::post(create_topup),
                )
                .route(
                    "/api/v1/wallet_transactions/txn-1/payment_url",
                    axum::routing::post(generate_payment_url),
                )
                .route(
                    "/api/v1/invoices/invoice-1",
                    axum::routing::get(|| async {
                        axum::Json(json!({
                            "invoice": { "lago_id": "invoice-1", "status": "finalized" }
                        }))
                    }),
                ),
        )
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let error = client
            .create_wallet_topup(
                "wallet-1",
                &super::WalletTopUpInput {
                    external_id: "topup-1".to_string(),
                    amount_credits: 500,
                },
            )
            .await
            .expect_err("undocumented payment URL response shape must fail");

        assert!(
            error
                .to_string()
                .contains("payment URL response did not include a payment URL")
        );
    }

    #[tokio::test]
    async fn ensure_subscription_treats_create_conflict_as_existing() {
        async fn subscription_not_found() -> axum::http::StatusCode {
            axum::http::StatusCode::NOT_FOUND
        }

        async fn create_subscription() -> (axum::http::StatusCode, axum::Json<serde_json::Value>) {
            (
                axum::http::StatusCode::CONFLICT,
                axum::Json(json!({ "code": "value_already_exist", "error": "exists" })),
            )
        }

        let base_url = spawn_lago_mock(
            axum::Router::new()
                .route(
                    "/api/v1/subscriptions/owner-1:starter",
                    axum::routing::get(subscription_not_found),
                )
                .route(
                    "/api/v1/subscriptions",
                    axum::routing::post(create_subscription),
                ),
        )
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let subscription_id = client
            .ensure_subscription("owner-1", "starter")
            .await
            .expect("ensure subscription");

        assert_eq!(
            subscription_id,
            subscription_external_id("owner-1", "starter")
        );
    }

    #[tokio::test]
    async fn wallet_transactions_parse_traceable_purchase_fields() {
        let base_url = spawn_lago_mock(axum::Router::new().route(
            "/api/v1/wallets/wallet-1/wallet_transactions",
            axum::routing::get(|| async {
                axum::Json(json!({
                    "wallet_transactions": [{
                        "lago_id": "txn-purchase-1",
                        "status": "settled",
                        "transaction_status": "purchased",
                        "transaction_type": "inbound",
                        "credit_amount": "12.50000",
                        "remaining_credit_amount": "3.25000",
                        "settled_at": "2025-08-20T12:00:00Z",
                        "created_at": "2025-08-20T11:59:00Z"
                    }]
                }))
            }),
        ))
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let transactions = client
            .wallet_transactions("wallet-1")
            .await
            .expect("wallet transactions");

        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].id, "txn-purchase-1");
        assert_eq!(transactions[0].credit_amount_micros, 12_500_000);
        assert_eq!(transactions[0].remaining_credit_micros, Some(3_250_000));
        assert_eq!(
            transactions[0].settled_at.map(|value| value.timestamp()),
            Some(1_755_691_200)
        );
    }

    #[tokio::test]
    async fn void_wallet_credits_sends_lago_five_decimal_amount() {
        async fn create_void(
            axum::Json(body): axum::Json<serde_json::Value>,
        ) -> axum::Json<serde_json::Value> {
            let transaction = &body["wallet_transaction"];
            assert_eq!(transaction["wallet_id"], "wallet-1");
            assert_eq!(transaction["voided_credits"], "1.23456");
            assert_eq!(
                transaction["name"],
                "NyxID purchased-credit expiry expiry-operation-1"
            );
            axum::Json(json!({
                "wallet_transactions": [{ "lago_id": "txn-void-1" }]
            }))
        }

        let base_url = spawn_lago_mock(axum::Router::new().route(
            "/api/v1/wallet_transactions",
            axum::routing::post(create_void),
        ))
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let transaction_id = client
            .void_wallet_credits("wallet-1", 1_234_567, "expiry-operation-1")
            .await
            .expect("void credits");

        assert_eq!(transaction_id, "txn-void-1");
    }

    #[tokio::test]
    async fn wallet_transaction_pagination_fails_closed_at_safety_cap() {
        async fn full_page() -> axum::Json<serde_json::Value> {
            let transactions: Vec<_> = (0..WALLET_TRANSACTION_PAGE_SIZE)
                .map(|index| {
                    json!({
                        "lago_id": format!("transaction-{index}"),
                        "status": "settled",
                        "transaction_status": "purchased",
                        "transaction_type": "inbound",
                        "credit_amount": "1.00000",
                        "remaining_credit_amount": "1.00000",
                        "created_at": "2025-01-01T00:00:00Z"
                    })
                })
                .collect();
            axum::Json(json!({ "wallet_transactions": transactions }))
        }
        let base_url = spawn_lago_mock(axum::Router::new().route(
            "/api/v1/wallets/wallet-1/wallet_transactions",
            axum::routing::get(full_page),
        ))
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let error = client
            .wallet_transactions("wallet-1")
            .await
            .expect_err("truncated history must fail closed");

        assert!(error.to_string().contains("safe pagination limit"));
    }

    #[test]
    fn usage_event_uses_wallet_funded_decimal_quantity() {
        let now = chrono::Utc::now();
        let mut row = crate::models::usage_meter::UsageMeterRow {
            id: "row-1".to_string(),
            transaction_id: "tx-1".to_string(),
            billing_request_id: "request-1".to_string(),
            layer: crate::models::usage_meter::BillingLayer::Platform,
            flush_seq: None,
            billing_owner_id: "owner-1".to_string(),
            wallet_id: Some("wallet-1".to_string()),
            actor_user_id: "owner-1".to_string(),
            api_key_id: None,
            service_id: Some("service-1".to_string()),
            service_slug: Some("service-one".to_string()),
            metric: crate::models::service_billing::BillingMetric::Requests,
            lago_metric_code: "platform_svc_service-one".to_string(),
            credential_class: crate::models::usage_meter::CredentialClass::UserOwned,
            model: None,
            token_breakdown: None,
            reserved_credits: 1,
            funding: Some(crate::models::usage_meter::UsageFunding {
                settled: true,
                wallet_charge_credits: Some(1),
                lago_billable_quantity_micros: Some(250_000),
                settled_at: Some(now),
                ..Default::default()
            }),
            quantity: Some(1),
            pending_resale_quantity: None,
            status: crate::models::usage_meter::UsageStatus::Finalized,
            forwarded: true,
            released: true,
            lago_acked: false,
            attempt: 0,
            settlement_attempts: 0,
            settlement_next_retry_at: None,
            created_at: now,
            updated_at: now,
            finalized_at: Some(now),
            expires_at: None,
            last_error: None,
        };

        let event = LagoEvent::from_usage_row(&row, Some("subscription-1".to_string()))
            .expect("Lago event");

        assert_eq!(event.properties.quantity, json!("0.250000"));

        row.funding
            .as_mut()
            .expect("funding")
            .lago_billable_quantity_micros = Some(2_000_000);
        let whole_event = LagoEvent::from_usage_row(&row, Some("subscription-1".to_string()))
            .expect("whole-number Lago event");
        assert_eq!(whole_event.properties.quantity, json!(2));
    }

    #[tokio::test]
    async fn batch_event_push_falls_back_to_single_event_endpoint() {
        async fn batch_unsupported() -> axum::http::StatusCode {
            axum::http::StatusCode::NOT_FOUND
        }

        async fn create_event() -> axum::Json<serde_json::Value> {
            axum::Json(json!({ "event": { "lago_id": "evt_1" } }))
        }

        let base_url = spawn_lago_mock(
            axum::Router::new()
                .route(
                    "/api/v1/events/batch",
                    axum::routing::post(batch_unsupported),
                )
                .route("/api/v1/events", axum::routing::post(create_event)),
        )
        .await;
        let client = LagoClient::new(base_url, "test-key".to_string()).expect("client");

        let acks = client
            .record_events_batch(&[LagoEvent {
                transaction_id: "tx-1".to_string(),
                external_customer_id: Some("owner-1".to_string()),
                external_subscription_id: None,
                code: "platform_requests".to_string(),
                timestamp: 1,
                properties: LagoEventProperties {
                    quantity: serde_json::json!(1),
                    model: None,
                    service_code: Some("svc".to_string()),
                    layer: Some("platform".to_string()),
                },
            }])
            .await
            .expect("batch fallback");

        assert_eq!(acks[0].transaction_id, "tx-1");
    }
}
