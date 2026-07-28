use std::time::Duration;

use async_trait::async_trait;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::errors::{AppError, AppResult};

const HTTP_TIMEOUT_SECS: u64 = 20;

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
    async fn entitlements(&self, subscription_id: &str) -> AppResult<Vec<Entitlement>>;
    /// Per-unit rates for the plan's standard charges, used to refresh the
    /// local rate cache. Defaults to empty so fakes opt in explicitly.
    async fn plan_rates(&self, _plan_code: &str) -> AppResult<Vec<PlanRate>> {
        Ok(Vec::new())
    }
}

/// A per-unit price from a Lago plan charge, converted to micro-credits
/// (1 credit = 1 USD, matching the wallet rate_amount NyxID provisions).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanRate {
    pub lago_metric_code: String,
    pub credits_per_unit_micros: i64,
}

#[derive(Clone)]
pub struct LagoClient {
    base_url: String,
    api_key: String,
    payment_provider_code: Option<String>,
    http: reqwest::Client,
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
        let payment_details = self
            .generate_wallet_transaction_payment_url(&transaction.wallet_transaction_id)
            .await?;

        Ok(WalletTopUpCheckout {
            wallet_transaction_id: transaction.wallet_transaction_id,
            lago_invoice_id: transaction.lago_invoice_id,
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
}

impl LagoClient {
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
        let value = self
            .json_request(
                reqwest::Method::POST,
                &format!(
                    "wallet_transactions/{}/payment_url",
                    urlencoding::encode(wallet_transaction_id)
                ),
                None,
            )
            .await
            .map_err(lago_error_to_app)?;
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
        let properties = LagoEventProperties {
            quantity: quantity.max(0),
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
    pub quantity: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub service_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub layer: Option<String>,
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
                // Prefer the ongoing balance: Lago decrements credits_balance
                // only when a period invoice settles, while ongoing reflects
                // usage as it accrues — the number a prepaid gate cares about.
                "credits_ongoing_balance",
                "credits_balance",
                "credits_ongoing_usage_balance",
                "ongoing_balance",
                "balance_credits",
                "amount",
            ],
        )
    })
}

/// Parse a Lago decimal amount string ("0.000005", "0.01", "1") into
/// micro-credits without floating point. Digits beyond micro precision are
/// truncated. Returns None for negative or malformed values.
pub fn decimal_credits_to_micros(amount: &str) -> Option<i64> {
    let amount = amount.trim();
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
    let frac_digits: String = frac_part.chars().take(6).collect();
    if !frac_digits.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
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
    use serde_json::json;

    use super::{
        LagoApi, LagoClient, LagoError, LagoErrorKind, LagoEvent, LagoEventProperties,
        OwnerProvisionInput, PlanRate, classify_lago_failure, decimal_credits_to_micros,
        extract_active_wallet, extract_wallet_balance_credits, subscription_external_id,
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
                    quantity: 1,
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
