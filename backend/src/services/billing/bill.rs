//! Read-only bill projection. Provider totals are authoritative; computed tax
//! rows explain current usage, whose Lago response has no per-tax breakdown.
use super::lago_client::LagoApi;
use crate::{
    errors::{AppError, AppResult},
    models::{
        billing_tax_profile::BillingTaxProfile,
        billing_wallet::BillingWallet,
        credits::{Credits, SCALE},
    },
};
use serde_json::Value;
use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct BillTax {
    pub code: String,
    pub name: String,
    pub rate_percent: Credits,
    pub amount: Credits,
}
#[derive(Debug)]
pub struct BillCharge {
    pub metric_code: String,
    pub name: String,
    pub units: String,
    pub amount: Credits,
}
#[derive(Debug)]
pub struct CurrentBill {
    pub currency: String,
    pub from: String,
    pub to: String,
    pub charges: Vec<BillCharge>,
    pub subtotal: Credits,
    pub taxes: Vec<BillTax>,
    pub tax_breakdown_available: bool,
    pub tax_total: Credits,
    pub rounding: Credits,
    pub total: Credits,
}
#[derive(Debug)]
pub struct UsageInvoice {
    pub id: String,
    pub number: String,
    pub issuing_date: String,
    pub currency: String,
    pub payment_status: String,
    pub subtotal: Credits,
    pub taxes: Vec<BillTax>,
    pub tax_total: Credits,
    pub total: Credits,
    pub prepaid_credits_applied: Credits,
    pub amount_due: Credits,
}
#[derive(Debug)]
pub struct InvoicePage {
    pub invoices: Vec<UsageInvoice>,
    pub total: u64,
}
fn invalid() -> AppError {
    AppError::BillingProviderUnavailable("Lago bill data is incomplete".into())
}
fn text(value: &Value, key: &str) -> AppResult<String> {
    value
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(invalid)
}
fn decimal(value: &Value) -> AppResult<Credits> {
    value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string())
        .parse()
        .map_err(|_| invalid())
}
fn cents(value: &Value, key: &str) -> AppResult<Credits> {
    let n = value.get(key).ok_or_else(invalid)?;
    let n = n
        .as_i64()
        .or_else(|| n.as_str()?.parse().ok())
        .ok_or_else(invalid)?;
    Credits::from_pico(i128::from(n) * (SCALE / 100)).map_err(Into::into)
}
fn rounded_cent(value: Credits) -> AppResult<Credits> {
    let cent = SCALE / 100;
    Credits::from_pico((value.pico().checked_add(cent / 2).ok_or_else(invalid)? / cent) * cent)
        .map_err(Into::into)
}
pub fn project_current(raw: &Value, profile: &BillingTaxProfile) -> AppResult<CurrentBill> {
    let usage = raw.get("customer_usage").unwrap_or(raw);
    let mut charges = Vec::new();
    let mut taxes: BTreeMap<(String, Credits), BillTax> = BTreeMap::new();
    for charge in usage
        .get("charges_usage")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?
    {
        let metric = charge.get("billable_metric").ok_or_else(invalid)?;
        let code = text(metric, "code")?;
        let amount = cents(charge, "amount_cents")?;
        for tax in super::taxes::snapshot(profile, &code)?.taxes {
            let item = taxes
                .entry((tax.code.clone(), tax.rate_percent))
                .or_insert(BillTax {
                    code: tax.code,
                    name: tax.name,
                    rate_percent: tax.rate_percent,
                    amount: Credits::ZERO,
                });
            item.amount = item
                .amount
                .checked_add(super::taxes::tax_half_up(amount, item.rate_percent)?)?;
        }
        let name = charge
            .pointer("/charge/invoice_display_name")
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
            .map(str::to_owned)
            .unwrap_or(text(metric, "name")?);
        let units = text(charge, "units")?;
        if amount != Credits::ZERO || decimal(&Value::String(units.clone()))? != Credits::ZERO {
            charges.push(BillCharge {
                metric_code: code,
                name,
                units,
                amount,
            });
        }
    }
    let mut taxes: Vec<_> = taxes.into_values().collect();
    for tax in &mut taxes {
        tax.amount = rounded_cent(tax.amount)?;
    }
    let tax_total = cents(usage, "taxes_amount_cents")?;
    let mut rounding =
        tax_total.checked_sub(Credits::checked_sum(taxes.iter().map(|t| t.amount))?)?;
    // current_usage has no applied-tax rows. Only explain a cent-level residue;
    // a larger mismatch may mean the cached tax names/rates no longer apply.
    let tolerance = (taxes.len() as i128 + 1) * (SCALE / 100);
    let tax_breakdown_available = rounding.pico().abs() <= tolerance;
    if !tax_breakdown_available {
        taxes.clear();
        rounding = Credits::ZERO;
    }
    Ok(CurrentBill {
        currency: text(usage, "currency")?,
        from: text(usage, "from_datetime")?,
        to: text(usage, "to_datetime")?,
        charges,
        subtotal: cents(usage, "amount_cents")?,
        taxes,
        tax_breakdown_available,
        tax_total,
        rounding,
        total: cents(usage, "total_amount_cents")?,
    })
}
pub async fn current(
    db: &mongodb::Database,
    lago: &dyn LagoApi,
    wallet: &BillingWallet,
    ttl: u64,
) -> AppResult<CurrentBill> {
    let profile = super::taxes::ensure_fresh(db, lago, wallet, ttl).await?;
    let subscription = wallet.lago_subscription_id.as_deref().ok_or_else(invalid)?;
    let usage = lago
        .current_usage(&wallet.lago_customer_id, subscription)
        .await?;
    project_current(&usage.raw, &profile)
}
pub fn project_invoices(raw: &Value, customer_id: &str) -> AppResult<InvoicePage> {
    let mut invoices = Vec::new();
    for invoice in raw
        .get("invoices")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?
    {
        // Defense in depth against adapters that ignore provider filters. Never
        // expose another customer's invoice, including its existence.
        if invoice["invoice_type"] != "subscription"
            || invoice["status"] != "finalized"
            || invoice
                .pointer("/customer/external_id")
                .and_then(Value::as_str)
                != Some(customer_id)
        {
            continue;
        }
        let taxes = invoice
            .get("applied_taxes")
            .and_then(Value::as_array)
            .ok_or_else(invalid)?
            .iter()
            .map(|t| {
                Ok(BillTax {
                    code: text(t, "tax_code")?,
                    name: text(t, "tax_name")?,
                    rate_percent: decimal(t.get("tax_rate").ok_or_else(invalid)?)?,
                    amount: cents(t, "amount_cents")?,
                })
            })
            .collect::<AppResult<Vec<_>>>()?;
        invoices.push(UsageInvoice {
            id: text(invoice, "lago_id")?,
            number: text(invoice, "number")?,
            issuing_date: text(invoice, "issuing_date")?,
            currency: text(invoice, "currency")?,
            payment_status: text(invoice, "payment_status")?,
            subtotal: cents(invoice, "sub_total_excluding_taxes_amount_cents")?,
            taxes,
            tax_total: cents(invoice, "taxes_amount_cents")?,
            total: cents(invoice, "sub_total_including_taxes_amount_cents")?,
            prepaid_credits_applied: cents(invoice, "prepaid_credit_amount_cents")?,
            amount_due: cents(invoice, "total_due_amount_cents")?,
        });
    }
    Ok(InvoicePage {
        total: raw
            .pointer("/meta/total_count")
            .and_then(Value::as_u64)
            .ok_or_else(invalid)?,
        invoices,
    })
}
pub async fn past(
    lago: &dyn LagoApi,
    wallet: &BillingWallet,
    page: u32,
    per_page: u32,
) -> AppResult<InvoicePage> {
    project_invoices(
        &lago
            .subscription_invoices(&wallet.lago_customer_id, page, per_page)
            .await?,
        &wallet.lago_customer_id,
    )
}
