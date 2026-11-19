use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize};

use crate::{
    errors::{AppError, AppResult},
    models::billing_wallet::BillingWallet,
};

use super::LagoClient;

#[derive(Clone)]
pub struct CreditPeriodConfig {
    pub account_id: String,
    pub livemode: bool,
    pub secret: zeroize::Zeroizing<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ExternalPayment {
    pub purchase_id: String,
    pub stripe_account_id: String,
    pub stripe_customer_id: String,
    pub stripe_checkout_id: String,
    pub stripe_payment_intent_id: String,
    pub currency: String,
    pub credits: i64,
    pub subtotal_minor: i64,
    pub tax_minor: i64,
    pub total_minor: i64,
}

#[derive(Deserialize)]
pub struct ExternalPurchaseReceipt {
    #[serde(flatten)]
    pub payment: ExternalPayment,
    pub wallet_id: String,
    pub wallet_transaction_id: String,
    pub status: String,
    pub transaction_type: String,
    pub transaction_status: String,
    pub settled_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Deserialize)]
struct Capability {
    version: u32,
    wallet_id: String,
    invoice_created: bool,
    #[serde(default)]
    livemode: bool,
    stripe_account_id: String,
}

#[derive(Deserialize)]
struct ReceiptResponse {
    purchase: ExternalPurchaseReceipt,
}

#[derive(Serialize)]
pub struct ExternalReversal<'a> {
    #[serde(rename = "stripe_refund_id")]
    pub provider_id: &'a str,
    pub decision_id: &'a str,
    pub credits: i64,
    pub amount_minor: i64,
    pub protected_minor: i64,
    #[serde(skip)]
    pub cancel_only: bool,
}

#[derive(Deserialize)]
struct RefundResponse {
    refund: ExternalRefundReceipt,
}

#[derive(Deserialize)]
struct ExternalRefundReceipt {
    stripe_refund_id: String,
    decision_id: String,
    credits: i64,
    amount_minor: i64,
    wallet_transaction_id: String,
}

impl LagoClient {
    pub(super) async fn post_external_credit_refund(
        &self,
        wallet: &BillingWallet,
        purchase_id: &str,
        refund: &ExternalReversal<'_>,
        secret: &str,
    ) -> AppResult<String> {
        let wallet_id = wallet.lago_wallet_id.as_deref().ok_or_else(unavailable)?;
        let mut path = format!(
            "{}/{}/refunds",
            endpoint(wallet_id),
            urlencoding::encode(purchase_id)
        );
        if refund.cancel_only {
            path.push_str("/cancel");
        }
        let response = self.request(Method::POST, &path)
            .header("X-NyxID-Credit-Purchase-Key", secret)
            .json(&serde_json::json!({"external_customer_id": wallet.lago_customer_id, "refund": refund}))
            .send().await.map_err(|_| unavailable())?;
        let status = response.status();
        if matches!(
            status,
            StatusCode::CONFLICT | StatusCode::UNPROCESSABLE_ENTITY
        ) {
            let refusal: serde_json::Value = response.json().await.map_err(|_| unavailable())?;
            let code = refusal["code"].as_str();
            match (status, code) {
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    Some("external_purchase_refund_remainder_unproven"),
                ) => {
                    return Err(AppError::Conflict(
                        "Lago could not prove the approved credits are unused on this purchase"
                            .into(),
                    ));
                }
                (StatusCode::UNPROCESSABLE_ENTITY, Some("external_purchase_payment_mismatch"))
                | (StatusCode::CONFLICT, Some("external_purchase_payment_conflict")) => {
                    return Err(AppError::Conflict(
                        "Lago definitively refused this reversal decision".into(),
                    ));
                }
                _ => return Err(unavailable()),
            }
        }
        let response: RefundResponse = response
            .error_for_status()
            .map_err(|_| unavailable())?
            .json()
            .await
            .map_err(|_| unavailable())?;
        if response.refund.stripe_refund_id != refund.provider_id
            || response.refund.decision_id != refund.decision_id
            || response.refund.amount_minor != refund.amount_minor
            || response.refund.credits != refund.credits
            || response.refund.wallet_transaction_id.is_empty()
        {
            return Err(unavailable());
        }
        Ok(response.refund.wallet_transaction_id)
    }

    pub(super) async fn read_external_credit_capability(
        &self,
        wallet: &BillingWallet,
        account_id: &str,
        secret: &str,
    ) -> AppResult<()> {
        let wallet_id = wallet.lago_wallet_id.as_deref().ok_or_else(unavailable)?;
        let path = format!("{}/capability", endpoint(wallet_id));
        let response = self
            .request(Method::GET, &path)
            .header("X-NyxID-Credit-Purchase-Key", secret)
            .query(&[("external_customer_id", &wallet.lago_customer_id)])
            .send()
            .await
            .map_err(|_| unavailable())?;
        if response.status() == StatusCode::UNPROCESSABLE_ENTITY {
            let refusal: serde_json::Value = response.json().await.map_err(|_| unavailable())?;
            if refusal["code"] == "external_purchase_wallet_ineligible" {
                return Err(AppError::Conflict(
                    "Lago Stripe-funded wallet eligibility changed".into(),
                ));
            }
            return Err(unavailable());
        }
        let capability: Capability = response
            .error_for_status()
            .map_err(|_| unavailable())?
            .json()
            .await
            .map_err(|_| unavailable())?;
        if capability.version != 2
            || capability.wallet_id != wallet_id
            || capability.invoice_created
            || capability.stripe_account_id != account_id
            || !wallet
                .stripe_enrollment
                .as_ref()
                .is_some_and(|enrollment| enrollment.livemode == capability.livemode)
        {
            return Err(AppError::Conflict(
                "Lago external purchase connector identity changed".into(),
            ));
        }
        Ok(())
    }

    pub(super) async fn post_external_credit_purchase(
        &self,
        wallet: &BillingWallet,
        payment: &ExternalPayment,
        secret: &str,
    ) -> AppResult<ExternalPurchaseReceipt> {
        let wallet_id = wallet.lago_wallet_id.as_deref().ok_or_else(unavailable)?;
        let path = endpoint(wallet_id);
        let receipt_path = format!("{path}/{}", urlencoding::encode(&payment.purchase_id));
        let existing = self
            .request(Method::GET, &receipt_path)
            .header("X-NyxID-Credit-Purchase-Key", secret)
            .send()
            .await
            .map_err(|_| unavailable())?;
        let response = if existing.status() == StatusCode::NOT_FOUND {
            self.request(Method::POST, &path)
                .header("X-NyxID-Credit-Purchase-Key", secret)
                .json(&serde_json::json!({"external_customer_id": wallet.lago_customer_id, "purchase": payment}))
                .send().await.map_err(|_| unavailable())?
        } else {
            existing
        };
        let receipt: ReceiptResponse = response
            .error_for_status()
            .map_err(|_| unavailable())?
            .json()
            .await
            .map_err(|_| unavailable())?;
        Ok(receipt.purchase)
    }
}

fn endpoint(wallet_id: &str) -> String {
    format!(
        "wallets/{}/nyxid_external_credit_purchases",
        urlencoding::encode(wallet_id)
    )
}

pub(super) fn unavailable() -> AppError {
    AppError::BillingProviderUnavailable(
        "The Lago external credit purchase extension is unavailable or ineligible".into(),
    )
}
