use chrono::Utc;
use hmac::{Hmac, Mac};
use reqwest::{Client, Method, Url};
use serde::{Deserialize, de::DeserializeOwned};
use sha2::Sha256;
use zeroize::Zeroizing;

use crate::errors::{AppError, AppResult};
use crate::models::billing_topup_session::StripePurchase;

const STRIPE_API: &str = "https://api.stripe.com/v1/";
const STRIPE_API_VERSION: &str = "2025-05-28.basil";

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StripeSettings {
    pub mode: StripeMode,
    pub account_id: String,
    pub portal_configuration_id: String,
    pub product_tax_code: String,
    pub offer_version: String,
    pub unit_amount_minor: i64,
    pub maximum_subtotal_minor: i64,
}

impl StripeSettings {
    pub fn from_env() -> Option<Self> {
        let raw = std::env::var("BILLING_STRIPE_PURCHASES").ok()?;
        let settings: Self = serde_json::from_str(&raw).expect("invalid BILLING_STRIPE_PURCHASES");
        assert!(
            settings.account_id.starts_with("acct_")
                && settings.account_id.len() > 5
                && settings.portal_configuration_id.starts_with("bpc_")
                && settings.portal_configuration_id.len() > 4
                && settings.product_tax_code.starts_with("txcd_")
                && settings.product_tax_code.len() > 5,
            "invalid Stripe account, portal configuration, or product tax code"
        );
        assert!(
            settings.unit_amount_minor > 0
                && !settings.offer_version.is_empty()
                && (1_000..99_999_999).contains(&settings.maximum_subtotal_minor)
                && settings.maximum_subtotal_minor / settings.unit_amount_minor
                    >= (999 / settings.unit_amount_minor + 1),
            "invalid Stripe purchase offer"
        );
        Some(settings)
    }
}

#[derive(Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StripeMode {
    Test,
    Live,
}

impl StripeMode {
    pub(crate) fn livemode(self) -> bool {
        matches!(self, Self::Live)
    }
    pub(super) fn checkout_prefix(self) -> &'static str {
        match self {
            Self::Test => "cs_test_",
            Self::Live => "cs_live_",
        }
    }
    fn key_prefix(self) -> &'static str {
        match self {
            Self::Test => "test_",
            Self::Live => "live_",
        }
    }
}

#[derive(Clone)]
pub(crate) struct Stripe {
    http: Client,
    api: Url,
    key: Zeroizing<String>,
    secret: Zeroizing<String>,
    previous_secret: Option<Zeroizing<String>>,
    pub(super) lago_secret: Zeroizing<String>,
    pub(super) settings: StripeSettings,
    return_url: String,
}

#[derive(Deserialize)]
struct Account {
    id: String,
}

#[derive(Deserialize)]
struct Customer {
    id: String,
    livemode: bool,
    #[serde(default)]
    deleted: bool,
    metadata: std::collections::HashMap<String, String>,
}

#[derive(Deserialize)]
pub(super) struct Checkout {
    pub id: String,
    pub url: Option<String>,
    pub livemode: bool,
    pub customer: String,
    pub client_reference_id: String,
    pub mode: String,
    pub status: String,
    pub payment_status: String,
    pub currency: String,
    pub amount_subtotal: i64,
    pub amount_total: i64,
    pub total_details: TotalDetails,
    pub automatic_tax: AutomaticTax,
    pub payment_intent: Option<String>,
    pub invoice: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct TotalDetails {
    pub amount_tax: i64,
    amount_discount: i64,
    amount_shipping: i64,
}

#[derive(Deserialize)]
pub(super) struct AutomaticTax {
    enabled: bool,
    pub status: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct StripeInvoice {
    pub id: String,
    pub livemode: bool,
    pub customer: String,
    pub status: String,
    pub currency: String,
    pub subtotal: i64,
    pub amount_paid: i64,
    pub amount_remaining: i64,
    pub hosted_invoice_url: Option<String>,
}

#[derive(Deserialize)]
pub(super) struct PaymentIntent {
    pub id: String,
    pub livemode: bool,
    pub customer: String,
    pub status: String,
    pub currency: String,
    pub amount_received: i64,
    pub latest_charge: Charge,
}

#[derive(Deserialize)]
pub(super) struct Charge {
    pub id: String,
    pub amount_refunded: i64,
    pub disputed: bool,
}

#[derive(Deserialize)]
pub(super) struct Refund {
    pub id: String,
    pub charge: Option<String>,
    pub payment_intent: Option<String>,
    pub currency: String,
    pub status: Option<String>,
    pub amount: i64,
}

#[derive(Deserialize)]
pub(super) struct Dispute {
    pub id: String,
    pub charge: String,
    pub payment_intent: Option<String>,
    pub livemode: bool,
    pub status: String,
    pub amount: i64,
    pub currency: String,
}

#[derive(Deserialize)]
pub(super) struct Event {
    #[serde(rename = "type")]
    pub kind: String,
    livemode: bool,
    account: Option<String>,
    pub data: EventData,
}

#[derive(Deserialize)]
pub(super) struct EventData {
    pub object: EventObject,
}

#[derive(Deserialize)]
pub(super) struct EventObject {
    pub id: String,
    pub client_reference_id: Option<String>,
    pub payment_intent: Option<String>,
}

impl Stripe {
    pub(super) fn new(settings: StripeSettings, frontend_url: &str) -> AppResult<Self> {
        let key = std::env::var("BILLING_STRIPE_PURCHASE_KEY").map_err(|_| unavailable())?;
        let secret =
            std::env::var("BILLING_STRIPE_PURCHASE_WEBHOOK_SECRET").map_err(|_| unavailable())?;
        if !key
            .strip_prefix("sk_")
            .or_else(|| key.strip_prefix("rk_"))
            .is_some_and(|key| key.starts_with(settings.mode.key_prefix()))
            || !secret.starts_with("whsec_")
        {
            return Err(unavailable());
        }
        let lago_secret =
            std::env::var("BILLING_LAGO_EXTERNAL_PURCHASE_SECRET").map_err(|_| unavailable())?;
        if lago_secret.len() < 32 {
            return Err(unavailable());
        }
        let mut stripe =
            Self::with_endpoint(settings, frontend_url, key, secret, lago_secret, STRIPE_API)?;
        if let Ok(previous) = std::env::var("BILLING_STRIPE_PURCHASE_PREVIOUS_WEBHOOK_SECRET") {
            if !previous.starts_with("whsec_") {
                return Err(unavailable());
            }
            stripe.previous_secret = Some(Zeroizing::new(previous));
        }
        Ok(stripe)
    }

    pub(crate) fn with_endpoint(
        settings: StripeSettings,
        frontend_url: &str,
        key: String,
        secret: String,
        lago_secret: String,
        endpoint: &str,
    ) -> AppResult<Self> {
        let mut return_url = Url::parse(frontend_url).map_err(|_| unavailable())?;
        if !matches!(return_url.scheme(), "http" | "https") || return_url.host_str().is_none() {
            return Err(unavailable());
        }
        return_url.set_path("/billing");
        return_url.set_query(None);
        return_url.set_fragment(None);
        Ok(Self {
            http: Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(std::time::Duration::from_secs(8))
                .build()
                .map_err(|_| unavailable())?,
            api: Url::parse(endpoint).map_err(|_| unavailable())?,
            key: Zeroizing::new(key),
            secret: Zeroizing::new(secret),
            previous_secret: None,
            lago_secret: Zeroizing::new(lago_secret),
            settings,
            return_url: return_url.into(),
        })
    }

    async fn request<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        form: &[(&str, String)],
        idempotency_key: Option<&str>,
    ) -> AppResult<T> {
        let mut request = self
            .http
            .request(
                method.clone(),
                self.api.join(path).map_err(|_| unavailable())?,
            )
            .bearer_auth(self.key.as_str())
            .header("Stripe-Version", STRIPE_API_VERSION);
        if !form.is_empty() {
            request = request.form(form);
        }
        if let Some(key) = idempotency_key {
            request = request.header("Idempotency-Key", key);
        }
        let response = request.send().await.map_err(|_| unavailable())?;
        if !response.status().is_success() {
            tracing::warn!(%method, status = %response.status(), "Stripe billing request failed");
            return Err(unavailable());
        }
        response.json().await.map_err(|_| unavailable())
    }

    pub(crate) fn credit_period_config(
        &self,
    ) -> super::lago_client::external_purchases::CreditPeriodConfig {
        super::lago_client::external_purchases::CreditPeriodConfig {
            account_id: self.settings.account_id.clone(),
            livemode: self.settings.mode.livemode(),
            secret: self.lago_secret.clone(),
        }
    }

    pub(super) async fn verify_account(&self) -> AppResult<()> {
        let account: Account = self.request(Method::GET, "account", &[], None).await?;
        if account.id != self.settings.account_id {
            return Err(unavailable());
        }
        Ok(())
    }

    pub(super) async fn verify_customer(&self, owner_id: &str, customer_id: &str) -> AppResult<()> {
        self.verify_account().await?;
        let id = urlencoding::encode(customer_id);
        let customer: Customer = self
            .request(Method::GET, &format!("customers/{id}"), &[], None)
            .await?;
        if customer.livemode != self.settings.mode.livemode()
            || customer.deleted
            || customer.id != customer_id
            || customer.metadata.get("nyxid_owner_id").map(String::as_str) != Some(owner_id)
        {
            return Err(unavailable());
        }
        Ok(())
    }

    pub(super) fn return_url(&self, owner_id: &str) -> String {
        format!(
            "{}?ownerId={}",
            self.return_url,
            urlencoding::encode(owner_id)
        )
    }

    pub(super) async fn portal(&self, owner_id: &str, customer_id: &str) -> AppResult<String> {
        #[derive(Deserialize)]
        struct Portal {
            livemode: bool,
            customer: String,
            url: String,
        }
        self.verify_customer(owner_id, customer_id).await?;
        let portal: Portal = self
            .request(
                Method::POST,
                "billing_portal/sessions",
                &[
                    ("customer", customer_id.into()),
                    (
                        "configuration",
                        self.settings.portal_configuration_id.clone(),
                    ),
                    ("return_url", self.return_url(owner_id)),
                ],
                None,
            )
            .await?;
        if portal.livemode != self.settings.mode.livemode() || portal.customer != customer_id {
            return Err(unavailable());
        }
        hosted_url(&portal.url, "billing.stripe.com")
    }

    pub(super) async fn create_checkout(
        &self,
        purchase_id: &str,
        owner_id: &str,
        purchase: &StripePurchase,
        amount_credits: i64,
    ) -> AppResult<Checkout> {
        self.verify_customer(owner_id, &purchase.customer_id)
            .await?;
        self.request(
            Method::POST,
            "checkout/sessions",
            &[
                ("mode", "payment".into()),
                ("customer", purchase.customer_id.clone()),
                ("client_reference_id", purchase_id.into()),
                ("success_url", purchase.return_url.clone()),
                ("cancel_url", purchase.return_url.clone()),
                (
                    "expires_at",
                    purchase.checkout_expires_at.timestamp().to_string(),
                ),
                ("automatic_tax[enabled]", "true".into()),
                ("adaptive_pricing[enabled]", "false".into()),
                ("billing_address_collection", "required".into()),
                ("customer_update[address]", "auto".into()),
                ("customer_update[name]", "auto".into()),
                ("tax_id_collection[enabled]", "true".into()),
                ("invoice_creation[enabled]", "true".into()),
                ("line_items[0][quantity]", amount_credits.to_string()),
                ("line_items[0][price_data][currency]", "usd".into()),
                (
                    "line_items[0][price_data][unit_amount]",
                    purchase.unit_amount_minor.to_string(),
                ),
                (
                    "line_items[0][price_data][tax_behavior]",
                    "exclusive".into(),
                ),
                (
                    "line_items[0][price_data][product_data][name]",
                    "NyxID credits".into(),
                ),
                (
                    "line_items[0][price_data][product_data][tax_code]",
                    purchase.product_tax_code.clone(),
                ),
                ("metadata[nyxid_purchase_id]", purchase_id.into()),
                (
                    "payment_intent_data[metadata][nyxid_purchase_id]",
                    purchase_id.into(),
                ),
                (
                    "invoice_creation[invoice_data][metadata][nyxid_purchase_id]",
                    purchase_id.into(),
                ),
            ],
            Some(purchase_id),
        )
        .await
    }

    pub(super) async fn checkout(&self, id: &str) -> AppResult<Checkout> {
        let id = urlencoding::encode(id);
        self.request(Method::GET, &format!("checkout/sessions/{id}"), &[], None)
            .await
    }

    pub(super) async fn payment(&self, id: &str) -> AppResult<PaymentIntent> {
        let id = urlencoding::encode(id);
        self.request(
            Method::GET,
            &format!("payment_intents/{id}?expand%5B%5D=latest_charge"),
            &[],
            None,
        )
        .await
    }

    pub(super) async fn refunds(&self, charge_id: &str) -> AppResult<Vec<Refund>> {
        self.charge_list("refunds", charge_id, |refund: &Refund| &refund.id)
            .await
    }

    pub(super) async fn disputes(&self, charge_id: &str) -> AppResult<Vec<Dispute>> {
        self.charge_list("disputes", charge_id, |dispute: &Dispute| &dispute.id)
            .await
    }

    async fn charge_list<T: DeserializeOwned>(
        &self,
        resource: &str,
        charge_id: &str,
        identity: impl Fn(&T) -> &str,
    ) -> AppResult<Vec<T>> {
        #[derive(Deserialize)]
        struct Page<T> {
            data: Vec<T>,
            has_more: bool,
        }
        let id = urlencoding::encode(charge_id);
        let base_path = format!("{resource}?charge={id}&limit=100");
        let mut path = base_path.clone();
        let mut items = Vec::new();
        loop {
            let page: Page<T> = self.request(Method::GET, &path, &[], None).await?;
            if !page.has_more {
                items.extend(page.data);
                return Ok(items);
            }
            let next = page
                .data
                .last()
                .map(&identity)
                .filter(|cursor| !cursor.is_empty())
                .ok_or_else(unavailable)?;
            let next_path = format!("{base_path}&starting_after={}", urlencoding::encode(next));
            if path == next_path {
                return Err(unavailable());
            }
            items.extend(page.data);
            path = next_path;
        }
    }

    pub(super) async fn invoice(&self, id: &str) -> AppResult<StripeInvoice> {
        let id = urlencoding::encode(id);
        self.request(Method::GET, &format!("invoices/{id}"), &[], None)
            .await
    }

    pub(super) fn verify_event(&self, signature: &str, body: &[u8]) -> AppResult<Event> {
        let invalid = || AppError::BadRequest("Invalid Stripe webhook signature".into());
        let fields: Vec<_> = signature
            .split(',')
            .filter_map(|part| part.split_once('='))
            .collect();
        let timestamp = fields
            .iter()
            .find(|(name, _)| *name == "t")
            .ok_or_else(invalid)?
            .1;
        let value: i64 = timestamp.parse().map_err(|_| invalid())?;
        if Utc::now().timestamp().abs_diff(value) > 300 {
            return Err(invalid());
        }
        let matched = std::iter::once(&self.secret)
            .chain(self.previous_secret.iter())
            .any(|secret| {
                let Ok(mut mac) = Hmac::<Sha256>::new_from_slice(secret.as_bytes()) else {
                    return false;
                };
                mac.update(timestamp.as_bytes());
                mac.update(b".");
                mac.update(body);
                fields
                    .iter()
                    .filter(|(name, _)| *name == "v1")
                    .any(|(_, digest)| {
                        hex::decode(digest)
                            .is_ok_and(|bytes| mac.clone().verify_slice(&bytes).is_ok())
                    })
            });
        if !matched {
            return Err(invalid());
        }
        let event: Event = serde_json::from_slice(body)
            .map_err(|_| AppError::BadRequest("Invalid Stripe webhook body".into()))?;
        if event.livemode != self.settings.mode.livemode()
            || event
                .account
                .as_ref()
                .is_some_and(|id| id != &self.settings.account_id)
        {
            return Err(AppError::BadRequest(
                "Unexpected Stripe webhook account or mode".into(),
            ));
        }
        Ok(event)
    }
}

impl Checkout {
    pub(super) fn verify_identity(
        &self,
        purchase_id: &str,
        purchase: &StripePurchase,
    ) -> AppResult<()> {
        if self.livemode != purchase.livemode
            || self.customer != purchase.customer_id
            || self.client_reference_id != purchase_id
            || self.mode != "payment"
            || !self.automatic_tax.enabled
            || self.currency != purchase.currency
            || self.amount_subtotal != purchase.subtotal_minor
        {
            return Err(unavailable());
        }
        Ok(())
    }

    pub(super) fn verify_paid(&self) -> AppResult<()> {
        if self.status != "complete"
            || self.payment_status != "paid"
            || self.automatic_tax.status.as_deref() != Some("complete")
            || self.total_details.amount_tax < 0
            || self.total_details.amount_discount != 0
            || self.total_details.amount_shipping != 0
            || self
                .amount_subtotal
                .checked_add(self.total_details.amount_tax)
                != Some(self.amount_total)
        {
            return Err(unavailable());
        }
        Ok(())
    }
}

pub(super) fn hosted_url(value: &str, host: &str) -> AppResult<String> {
    let url = Url::parse(value).map_err(|_| unavailable())?;
    if url.scheme() != "https"
        || url.host_str() != Some(host)
        || !url.username().is_empty()
        || url.password().is_some()
        || url.port().is_some()
    {
        return Err(unavailable());
    }
    Ok(value.into())
}

fn unavailable() -> AppError {
    AppError::BillingProviderUnavailable("Stripe billing could not be verified".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };

    #[tokio::test]
    async fn stripe_refund_list_pages_without_a_livemode_field() {
        let api = MockServer::start().await;
        let stripe = Stripe::with_endpoint(
            StripeSettings {
                mode: StripeMode::Test,
                account_id: "acct_fixture".into(),
                portal_configuration_id: "bpc_fixture".into(),
                product_tax_code: "txcd_fixture".into(),
                offer_version: "fixture".into(),
                unit_amount_minor: 100,
                maximum_subtotal_minor: 10_000,
            },
            "http://localhost:3000",
            "sk_test_fixture".into(),
            "whsec_fixture".into(),
            "connector-fixture".into(),
            &format!("{}/", api.uri()),
        )
        .unwrap();
        let item = |id: &str| {
            json!({"id":id,"charge":"ch_fixture","payment_intent":"pi_fixture",
            "amount":100,"currency":"usd","status":"succeeded"})
        };
        Mock::given(method("GET"))
            .and(path("/refunds"))
            .and(query_param("starting_after", "re_first"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data":[item("re_second")],"has_more":false,
            })))
            .expect(1)
            .mount(&api)
            .await;
        Mock::given(method("GET"))
            .and(path("/refunds"))
            .and(query_param("charge", "ch_fixture"))
            .and(|request: &wiremock::Request| {
                !request
                    .url
                    .query_pairs()
                    .any(|(key, _)| key == "starting_after")
            })
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data":[item("re_first")],"has_more":true,
            })))
            .expect(1)
            .mount(&api)
            .await;
        let entries = stripe.refunds("ch_fixture").await.unwrap();
        assert_eq!(
            entries
                .iter()
                .map(|refund| refund.id.as_str())
                .collect::<Vec<_>>(),
            ["re_first", "re_second"]
        );
    }
}
