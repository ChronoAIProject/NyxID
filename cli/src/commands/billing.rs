use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::ApiClient;
use crate::cli::{BillingCommands, BillingUsagePeriodArg, OutputFormat};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct BillingWalletResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub balance: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reserved: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_debits: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pending_expiry: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub available_with_overdraft: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub overdraft_cap: Option<String>,

    pub owner_id: String,
    pub plan_kind: String,
    pub collection_state: String,
    pub balance_credits: i64,
    pub reserved_credits: i64,
    pub pending_lago_debits: i64,
    pub available_credits: i64,
    pub available_with_overdraft_credits: i64,
    pub has_payment_instrument: bool,
    pub overdraft_cap_credits: i64,
    pub suspended: bool,
    pub lago_customer_id: String,
    pub lago_subscription_id: Option<String>,
    pub lago_wallet_id: Option<String>,
    pub balance_synced_at: String,
    pub created_at: String,
    pub updated_at: String,
    pub created: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct BillingUsageResponse {
    pub owner_id: String,
    pub period: String,
    pub rows: Vec<BillingUsageRow>,
    pub totals: BillingUsageTotals,
    pub billing: BillingReadOnlyBlock,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct BillingUsageRow {
    pub service_slug: Option<String>,
    pub service_id: Option<String>,
    pub model: Option<String>,
    pub api_key_name: Option<String>,
    pub metric: String,
    pub lago_metric_code: String,
    pub layer: String,
    pub quantity: i64,
    pub requests: i64,
    pub bytes: i64,
    pub events: i64,
    pub lago_acked: bool,
    #[serde(default = "default_billable")]
    pub billable: bool,
    pub estimated_credits_micros: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_credits: Option<String>,
    pub wallet_credits_micros: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet_credits: Option<String>,
    pub grant_credits_micros: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_credits: Option<String>,
    pub allowance_credits_micros: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowance_credits: Option<String>,
    #[serde(default)]
    pub allowance_quantity: i64,
}

fn default_billable() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct BillingUsageTotals {
    pub quantity: i64,
    pub requests: i64,
    pub bytes: i64,
    pub events: i64,
    pub estimated_credits_micros: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_credits: Option<String>,
    pub wallet_credits_micros: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wallet_credits: Option<String>,
    pub grant_credits_micros: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_credits: Option<String>,
    pub allowance_credits_micros: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allowance_credits: Option<String>,
    #[serde(default)]
    pub allowance_quantity: i64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct BillingReadOnlyBlock {
    pub charging_enabled: bool,
    pub lago_configured: bool,
    pub source: String,
    pub rates_are_approximate: bool,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct TopUpResponse {
    pub owner_id: String,
    pub amount_credits: i64,
    pub idempotency_key: String,
    pub checkout_url: String,
    pub payment_provider: Option<String>,
    pub lago_wallet_transaction_id: Option<String>,
    pub lago_invoice_id: Option<String>,
    pub status: String,
    pub reused: bool,
}

#[derive(Debug, Serialize)]
struct ProvisionWalletRequest {
    owner_id: Option<String>,
}

#[derive(Debug, Serialize)]
struct TopUpRequest {
    amount_credits: i64,
    idempotency_key: String,
    owner_id: Option<String>,
}

pub fn usage_path(period: Option<BillingUsagePeriodArg>) -> String {
    match period {
        Some(period) => format!("/billing/usage?period={}", period.as_query_value()),
        None => "/billing/usage".to_string(),
    }
}

pub async fn get_usage(
    api: &mut ApiClient,
    period: Option<BillingUsagePeriodArg>,
) -> Result<serde_json::Value> {
    api.get(&usage_path(period)).await
}

pub async fn run(command: BillingCommands) -> Result<()> {
    match command {
        BillingCommands::Wallet {
            provision,
            owner_id,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let wallet: BillingWalletResponse = if provision {
                api.post(
                    "/billing/wallet",
                    &ProvisionWalletRequest {
                        owner_id: owner_id.clone(),
                    },
                )
                .await?
            } else {
                api.get("/billing/wallet").await?
            };
            print_wallet(&wallet, auth.output)
        }
        BillingCommands::Usage { period, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let usage = get_usage(&mut api, period).await?;
            print_usage(&usage, auth.output)
        }
        BillingCommands::Topup {
            amount_credits,
            idempotency_key,
            owner_id,
            open,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let key =
                idempotency_key.unwrap_or_else(|| format!("nyxid-cli-topup-{}", Uuid::new_v4()));
            let response: TopUpResponse = api
                .post(
                    "/billing/topup",
                    &TopUpRequest {
                        amount_credits,
                        idempotency_key: key,
                        owner_id,
                    },
                )
                .await?;
            if open && let Err(error) = crate::browser::open_browser(&response.checkout_url) {
                eprintln!("Could not open checkout URL: {error}");
            }
            print_topup(&response, auth.output)
        }
        BillingCommands::VerifyTopupFlow {
            amount_credits,
            idempotency_key,
            owner_id,
            open,
            timeout_secs,
            poll_interval_secs,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let response = verify_topup_flow(
                &mut api,
                amount_credits,
                idempotency_key,
                owner_id,
                open,
                Duration::from_secs(timeout_secs),
                Duration::from_secs(poll_interval_secs),
            )
            .await?;
            print_topup_verification(&response, auth.output)
        }
    }
}

async fn verify_topup_flow(
    api: &mut ApiClient,
    amount_credits: i64,
    idempotency_key: Option<String>,
    owner_id: Option<String>,
    open: bool,
    timeout: Duration,
    poll_interval: Duration,
) -> Result<TopUpVerificationResponse> {
    if amount_credits <= 0 {
        bail!("amount_credits must be greater than 0");
    }
    if timeout.is_zero() {
        bail!("timeout_secs must be greater than 0");
    }
    if poll_interval.is_zero() {
        bail!("poll_interval_secs must be greater than 0");
    }

    let starting_wallet: BillingWalletResponse = api.get("/billing/wallet").await?;
    let key =
        idempotency_key.unwrap_or_else(|| format!("nyxid-cli-verify-topup-{}", Uuid::new_v4()));
    let topup: TopUpResponse = api
        .post(
            "/billing/topup",
            &TopUpRequest {
                amount_credits,
                idempotency_key: key,
                owner_id,
            },
        )
        .await?;
    if topup.payment_provider.as_deref() != Some("stripe") {
        bail!(
            "top-up did not return Stripe as payment_provider: {:?}",
            topup.payment_provider
        );
    }
    if !topup.checkout_url.starts_with("https://") {
        bail!("top-up checkout_url must be HTTPS");
    }
    if topup.lago_wallet_transaction_id.is_none() {
        bail!("top-up response is missing lago_wallet_transaction_id");
    }
    if open && let Err(error) = crate::browser::open_browser(&topup.checkout_url) {
        eprintln!("Could not open checkout URL: {error}");
    }

    eprintln!("Complete the Stripe sandbox checkout, then leave this command running.");
    let start_exact = starting_wallet
        .balance
        .clone()
        .unwrap_or_else(|| starting_wallet.balance_credits.to_string());
    let expected_exact = add_whole_amount(&start_exact, amount_credits)?;
    let deadline = Instant::now() + timeout;
    let mut final_wallet = starting_wallet.clone();
    while Instant::now() < deadline {
        let wallet: BillingWalletResponse = api.get("/billing/wallet").await?;
        if add_whole_amount(
            &wallet
                .balance
                .clone()
                .unwrap_or_else(|| wallet.balance_credits.to_string()),
            0,
        )? == expected_exact
        {
            final_wallet = wallet;
            return Ok(TopUpVerificationResponse {
                topup,
                starting_balance_credits: starting_wallet.balance_credits,
                final_balance_credits: final_wallet.balance_credits,
                expected_balance_credits: legacy_balance_projection(&expected_exact)?,
                starting_balance: start_exact.clone(),
                final_balance: final_wallet
                    .balance
                    .clone()
                    .unwrap_or_else(|| final_wallet.balance_credits.to_string()),
                expected_balance: expected_exact.clone(),
                verified: true,
            });
        }
        final_wallet = wallet;
        tokio::time::sleep(poll_interval).await;
    }

    bail!(
        "timed out waiting for Lago/Stripe reconciliation: start={} final={} expected={}",
        start_exact,
        final_wallet
            .balance
            .clone()
            .unwrap_or_else(|| final_wallet.balance_credits.to_string()),
        expected_exact
    )
}

fn print_wallet(wallet: &BillingWalletResponse, output: OutputFormat) -> Result<()> {
    match output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(wallet)?);
        }
        OutputFormat::Table => {
            eprintln!("Billing Wallet");
            eprintln!();
            eprintln!("Owner:             {}", wallet.owner_id);
            eprintln!("Plan:              {}", wallet.plan_kind);
            eprintln!("Status:            {}", wallet.collection_state);
            eprintln!(
                "Balance:           {} credits",
                wallet
                    .balance
                    .clone()
                    .unwrap_or_else(|| wallet.balance_credits.to_string())
            );
            eprintln!(
                "Available:         {} credits",
                wallet
                    .available
                    .clone()
                    .unwrap_or_else(|| wallet.available_credits.to_string())
            );
            eprintln!(
                "Reserved:          {} credits",
                wallet
                    .reserved
                    .clone()
                    .unwrap_or_else(|| wallet.reserved_credits.to_string())
            );
            eprintln!(
                "Pending Debits:    {} credits",
                wallet
                    .pending_debits
                    .clone()
                    .unwrap_or_else(|| wallet.pending_lago_debits.to_string())
            );
            eprintln!(
                "Overdraft Cap:     {} credits",
                wallet
                    .overdraft_cap
                    .clone()
                    .unwrap_or_else(|| wallet.overdraft_cap_credits.to_string())
            );
            eprintln!("Suspended:         {}", wallet.suspended);
            if wallet.created {
                eprintln!("Created:           true");
            }
        }
    }
    Ok(())
}

fn print_usage(usage: &serde_json::Value, output: OutputFormat) -> Result<()> {
    let rendered = format_usage(usage, output)?;
    match output {
        OutputFormat::Json => println!("{rendered}"),
        OutputFormat::Table => eprintln!("{rendered}"),
    }
    Ok(())
}

fn format_usage(usage: &serde_json::Value, output: OutputFormat) -> Result<String> {
    if matches!(output, OutputFormat::Json) {
        return Ok(serde_json::to_string_pretty(usage)?);
    }
    let usage = BillingUsageResponse::deserialize(usage)?;
    let mut lines = vec![
        format!("Billing Usage ({})", usage.period),
        String::new(),
        format!("Owner:             {}", usage.owner_id),
        format!(
            "Charging Enabled:  {}",
            usage.billing.charging_enabled && usage.billing.lago_configured
        ),
        format!(
            "Estimated Cost:    {}",
            format_estimated_credits(
                usage.totals.estimated_credits.as_deref(),
                usage.totals.estimated_credits_micros
            )
        ),
    ];
    for (label, exact, micros, quantity) in [
        (
            "Funded by grants",
            usage.totals.grant_credits.as_deref(),
            usage.totals.grant_credits_micros,
            0,
        ),
        (
            "Funded by allowances",
            usage.totals.allowance_credits.as_deref(),
            usage.totals.allowance_credits_micros,
            usage.totals.allowance_quantity,
        ),
        (
            "Charged to wallet",
            usage.totals.wallet_credits.as_deref(),
            usage.totals.wallet_credits_micros,
            0,
        ),
    ] {
        if nonzero_amount(exact, micros) || quantity > 0 {
            lines.push(format!(
                "{label}: {}",
                format_estimated_credits(exact, micros)
            ));
        }
    }
    lines.push(String::new());
    if usage.rows.is_empty() {
        lines.push("No usage in this period.".to_string());
        return Ok(lines.join("\n"));
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header([
        "Service", "Model", "Agent", "Layer", "Metric", "Quantity", "Events", "Cost", "Status",
        "Funding",
    ]);
    for row in &usage.rows {
        table.add_row([
            row.service_slug
                .as_deref()
                .or(row.service_id.as_deref())
                .unwrap_or("-")
                .to_string(),
            row.model.as_deref().unwrap_or("-").to_string(),
            row.api_key_name.as_deref().unwrap_or("-").to_string(),
            row.layer.clone(),
            super::billing_units::label(&row.metric, false).to_string(),
            row.quantity.to_string(),
            row.events.to_string(),
            if row.billable {
                format_estimated_credits(
                    row.estimated_credits.as_deref(),
                    row.estimated_credits_micros,
                )
            } else {
                "free".to_string()
            },
            if !row.billable {
                "free"
            } else if row.lago_acked {
                "acked"
            } else {
                "pending"
            }
            .to_string(),
            format_usage_funding(row),
        ]);
    }
    lines.push(table.to_string());
    Ok(lines.join("\n"))
}

fn format_usage_funding(row: &BillingUsageRow) -> String {
    let grants = nonzero_amount(row.grant_credits.as_deref(), row.grant_credits_micros);
    let allowances = nonzero_amount(
        row.allowance_credits.as_deref(),
        row.allowance_credits_micros,
    ) || row.allowance_quantity > 0;
    if !row.billable || !(grants || allowances) {
        return String::new();
    }
    let mut parts = Vec::new();
    if grants {
        parts.push(format!(
            "grants {}",
            format_estimated_credits(row.grant_credits.as_deref(), row.grant_credits_micros)
        ));
    }
    if allowances {
        let units = if row.allowance_quantity > 0 {
            format!(
                " ({} {})",
                row.allowance_quantity,
                super::billing_units::label(&row.metric, false)
            )
        } else {
            String::new()
        };
        parts.push(format!(
            "allowance {}{units}",
            format_estimated_credits(
                row.allowance_credits.as_deref(),
                row.allowance_credits_micros
            )
        ));
    }
    parts.push(format!(
        "wallet {}",
        format_estimated_credits(row.wallet_credits.as_deref(), row.wallet_credits_micros)
    ));
    parts.join(" · ")
}

fn print_topup(response: &TopUpResponse, output: OutputFormat) -> Result<()> {
    match output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(response)?);
        }
        OutputFormat::Table => {
            eprintln!("Billing Top-up");
            eprintln!();
            eprintln!("Owner:             {}", response.owner_id);
            eprintln!("Amount:            {} credits", response.amount_credits);
            eprintln!("Status:            {}", response.status);
            eprintln!("Reused:            {}", response.reused);
            eprintln!("Checkout URL:      {}", response.checkout_url);
            if let Some(invoice_id) = &response.lago_invoice_id {
                eprintln!("Lago Invoice:      {invoice_id}");
            }
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
struct TopUpVerificationResponse {
    topup: TopUpResponse,
    starting_balance: String,
    final_balance: String,
    expected_balance: String,
    starting_balance_credits: i64,
    final_balance_credits: i64,
    expected_balance_credits: i64,
    verified: bool,
}

fn print_topup_verification(
    response: &TopUpVerificationResponse,
    output: OutputFormat,
) -> Result<()> {
    match output {
        OutputFormat::Json => {
            println!("{}", serde_json::to_string_pretty(response)?);
        }
        OutputFormat::Table => {
            eprintln!("Billing Top-up Verification");
            eprintln!();
            eprintln!("Verified:          {}", response.verified);
            eprintln!("Start Balance:     {} credits", response.starting_balance);
            eprintln!("Final Balance:     {} credits", response.final_balance);
            eprintln!("Expected Balance:  {} credits", response.expected_balance);
            eprintln!("Checkout URL:      {}", response.topup.checkout_url);
            if let Some(provider) = &response.topup.payment_provider {
                eprintln!("Payment Provider:  {provider}");
            }
            if let Some(transaction_id) = &response.topup.lago_wallet_transaction_id {
                eprintln!("Lago Transaction:  {transaction_id}");
            }
            if let Some(invoice_id) = &response.topup.lago_invoice_id {
                eprintln!("Lago Invoice:      {invoice_id}");
            }
        }
    }
    Ok(())
}

fn format_estimated_credits(exact: Option<&str>, value: Option<i64>) -> String {
    if let Some(exact) = exact {
        return format!("{exact} credits");
    }
    match value {
        Some(micros) => format!(
            "{}{}.{:06} credits",
            if micros < 0 { "-" } else { "" },
            micros.unsigned_abs() / 1_000_000,
            micros.unsigned_abs() % 1_000_000
        ),
        None => "-".to_string(),
    }
}

fn nonzero_amount(exact: Option<&str>, legacy: Option<i64>) -> bool {
    exact.map_or_else(
        || legacy.is_some_and(|n| n > 0),
        |s| s.bytes().any(|b| matches!(b, b'1'..=b'9')),
    )
}

/// Legacy output is only a bounded display projection of the exact result.
fn legacy_balance_projection(value: &str) -> Result<i64> {
    let whole = value.split_once('.').map_or(value, |(whole, _)| whole);
    Ok(whole
        .parse::<i128>()?
        .clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64)
}

/// The verification command only adds a whole-credit checkout amount. Preserve
/// the decimal fractional suffix exactly, including balances greater than 2^53.
fn add_whole_amount(value: &str, amount: i64) -> Result<String> {
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    let fraction = fraction.trim_end_matches('0');
    if !fraction.bytes().all(|b| b.is_ascii_digit()) || fraction.len() > 12 {
        bail!("invalid exact wallet amount");
    }
    let scale = 1_000_000_000_000_i128;
    let integer = whole.parse::<i128>()?;
    let fractional = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<i128>()?
            .checked_mul(10_i128.pow(12 - fraction.len() as u32))
            .ok_or_else(|| anyhow::anyhow!("wallet amount overflow"))?
    };
    let signed_fraction = if value.starts_with('-') {
        -fractional
    } else {
        fractional
    };
    let total = integer
        .checked_mul(scale)
        .and_then(|n| n.checked_add(signed_fraction))
        .and_then(|n| n.checked_add(i128::from(amount) * scale))
        .ok_or_else(|| anyhow::anyhow!("wallet amount overflow"))?;
    let fraction = format!("{:012}", total.unsigned_abs() % scale as u128);
    let fraction = fraction.trim_end_matches('0');
    Ok(format!(
        "{}{}{}{}",
        if total < 0 { "-" } else { "" },
        total.unsigned_abs() / scale as u128,
        if fraction.is_empty() { "" } else { "." },
        fraction
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{BillingCommands, BillingUsagePeriodArg, OutputFormat};
    use crate::test_support::{mock_auth, mock_auth_with_output};
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn exact_output_preserves_sub_micro_and_large_balances() {
        let mut payload = usage_json();
        {
            let item = &mut payload["rows"][0];
            item["estimated_credits"] = serde_json::json!("0.0356864");
            item["grant_credits"] = serde_json::json!("0.0356864");
            item["wallet_credits"] = serde_json::json!("0");
        }
        payload["totals"]["estimated_credits"] = serde_json::json!("0.0356864");
        payload["totals"]["grant_credits"] = serde_json::json!("0.0356864");
        let output = format_usage(&payload, OutputFormat::Table).unwrap();
        assert!(output.contains("0.0356864 credits"));
        assert_eq!(
            format_estimated_credits(Some("0.0000008"), Some(0)),
            "0.0000008 credits"
        );
        assert_eq!(
            add_whole_amount("999999999999999999.999999999999", 1).unwrap(),
            "1000000000000000000.999999999999"
        );
        assert_eq!(
            legacy_balance_projection("9223372036854775808.6").unwrap(),
            i64::MAX
        );
        assert_eq!(legacy_balance_projection("-0.6").unwrap(), 0);
    }

    #[test]
    fn usage_path_targets_billing_usage() {
        assert_eq!(usage_path(None), "/billing/usage");
        assert_eq!(
            usage_path(Some(BillingUsagePeriodArg::Last7Days)),
            "/billing/usage?period=7d"
        );
    }

    #[test]
    fn usage_free_rows_render_free_cost_and_status() {
        let mut payload = usage_json();
        payload["rows"][0]["billable"] = serde_json::json!(false);
        payload["rows"][0]["estimated_credits_micros"] = serde_json::json!(0);
        payload["totals"]["estimated_credits_micros"] = serde_json::json!(0);
        let output = format_usage(&payload, OutputFormat::Table).unwrap();
        let row = output
            .lines()
            .find(|line| line.contains("chrono-llm-public"))
            .unwrap();
        assert_eq!(
            row.split_whitespace()
                .filter(|cell| *cell == "free")
                .count(),
            2
        );
        assert!(!row.contains("pending"));
        assert!(!row.contains("acked"));
        assert!(!row.contains("credits"));
        assert!(!output.contains("Funded by"));
        assert!(!output.contains("Charged to wallet"));
    }

    #[test]
    fn usage_grant_funding_renders_gross_cost_and_split() {
        let mut payload = usage_json();
        {
            let funding = &mut payload["rows"][0];
            funding["billable"] = serde_json::json!(true);
            funding["model"] = serde_json::json!("test-model");
            funding["api_key_name"] = serde_json::json!("My agent");
            funding["wallet_credits_micros"] = serde_json::json!(0);
            funding["grant_credits_micros"] = serde_json::json!(2440);
            funding["allowance_credits_micros"] = serde_json::json!(0);
            funding["allowance_quantity"] = serde_json::json!(0);
        }
        payload["totals"]["grant_credits_micros"] = serde_json::json!(2440);
        payload["totals"]["wallet_credits_micros"] = serde_json::json!(0);
        let output = format_usage(&payload, OutputFormat::Table).unwrap();
        assert!(output.contains("Estimated Cost:    0.002440 credits"));
        assert!(output.contains("Funded by grants: 0.002440 credits"));
        assert!(output.contains("grants 0.002440 credits · wallet 0.000000 credits"));
        assert!(output.contains("test-model"));
        assert!(output.contains("My agent"));
        assert!(!output.contains("Funded by allowances"));
        assert!(!output.contains("Charged to wallet"));
    }

    #[test]
    fn usage_mixed_funding_renders_allowance_units_and_totals() {
        let mut payload = usage_json();
        let funding = serde_json::json!({
            "wallet_credits_micros": 240,
            "grant_credits_micros": 1000,
            "allowance_credits_micros": 1200,
            "allowance_quantity": 1200
        });
        payload["rows"][0]
            .as_object_mut()
            .unwrap()
            .extend(funding.as_object().unwrap().clone());
        payload["totals"]
            .as_object_mut()
            .unwrap()
            .extend(funding.as_object().unwrap().clone());
        let output = format_usage(&payload, OutputFormat::Table).unwrap();
        assert!(output.contains("grants 0.001000 credits · allowance 0.001200 credits (1200 tokens) · wallet 0.000240 credits"));
        assert!(output.contains("Funded by grants: 0.001000 credits"));
        assert!(output.contains("Funded by allowances: 0.001200 credits"));
        assert!(output.contains("Charged to wallet: 0.000240 credits"));
    }

    #[test]
    fn usage_older_server_payload_defaults_to_billable() {
        let payload = usage_json();
        let usage: BillingUsageResponse = serde_json::from_value(payload.clone()).unwrap();
        let row = &usage.rows[0];
        assert!(row.billable);
        assert_eq!(row.model, None);
        assert_eq!(row.api_key_name, None);
        assert_eq!(row.wallet_credits_micros, None);
        assert_eq!(row.grant_credits_micros, None);
        assert_eq!(row.allowance_credits_micros, None);
        assert_eq!(row.allowance_quantity, 0);
        assert_eq!(usage.totals.wallet_credits_micros, None);
        assert_eq!(usage.totals.grant_credits_micros, None);
        assert_eq!(usage.totals.allowance_credits_micros, None);
        assert_eq!(usage.totals.allowance_quantity, 0);
        let output = format_usage(&payload, OutputFormat::Table).unwrap();
        assert!(output.contains("0.002440 credits"));
        assert!(output.contains("pending"));
        assert!(!output.contains("Funded by"));
    }

    #[test]
    fn usage_json_preserves_raw_response_fields_without_adding_defaults() {
        let old = usage_json();
        let mut current = old.clone();
        current["rows"][0]["billable"] = serde_json::json!(true);
        current["rows"][0]["grant_credits_micros"] = serde_json::json!(2440);
        current["rows"][0]["api_key_id"] = serde_json::json!("key-id");
        current["rows"][0]["token_breakdown"] = serde_json::json!({"prompt_tokens": 2440});
        for payload in [old, current] {
            let output = format_usage(&payload, OutputFormat::Json).unwrap();
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&output).unwrap(),
                payload
            );
        }
    }

    fn usage_json() -> serde_json::Value {
        serde_json::json!({
            "owner_id": "owner-1",
            "period": "7d",
            "rows": [{
                "service_slug": "chrono-llm-public",
                "service_id": "service-1",
                "metric": "tokens",
                "lago_metric_code": "platform_tokens",
                "layer": "platform",
                "quantity": 2440,
                "requests": 0,
                "bytes": 0,
                "events": 1,
                "lago_acked": false,
                "estimated_credits_micros": 2440
            }],
            "totals": {
                "quantity": 2440,
                "requests": 0,
                "bytes": 0,
                "events": 1,
                "estimated_credits_micros": 2440
            },
            "billing": {
                "charging_enabled": true,
                "lago_configured": true,
                "source": "usage_meter",
                "rates_are_approximate": true
            }
        })
    }

    #[tokio::test]
    async fn wallet_show_calls_wallet_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/billing/wallet"))
            .respond_with(ResponseTemplate::new(200).set_body_json(wallet_json(false)))
            .expect(1)
            .mount(&server)
            .await;

        run(BillingCommands::Wallet {
            provision: false,
            owner_id: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("wallet should succeed");
    }

    #[tokio::test]
    async fn wallet_provision_posts_owner_scope() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/billing/wallet"))
            .and(body_json(serde_json::json!({ "owner_id": "owner-1" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(wallet_json(true)))
            .expect(1)
            .mount(&server)
            .await;

        run(BillingCommands::Wallet {
            provision: true,
            owner_id: Some("owner-1".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("wallet provision should succeed");
    }

    #[tokio::test]
    async fn usage_reads_selected_period() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/billing/usage"))
            .and(query_param("period", "7d"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "owner_id": "owner-1",
                "period": "7d",
                "rows": [],
                "totals": {
                    "quantity": 0,
                    "requests": 0,
                    "bytes": 0,
                    "events": 0,
                    "estimated_credits_micros": null
                },
                "billing": {
                    "charging_enabled": true,
                    "lago_configured": true,
                    "source": "usage_meter",
                    "rates_are_approximate": true
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(BillingCommands::Usage {
            period: Some(BillingUsagePeriodArg::Last7Days),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("usage should succeed");
    }

    #[tokio::test]
    async fn topup_posts_amount_and_idempotency_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/billing/topup"))
            .and(body_json(serde_json::json!({
                "amount_credits": 50,
                "idempotency_key": "topup-key-123",
                "owner_id": null
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "owner_id": "owner-1",
                "amount_credits": 50,
                "idempotency_key": "topup-key-123",
                "checkout_url": "https://checkout.example.com/session",
                "payment_provider": "stripe",
                "lago_wallet_transaction_id": "txn-1",
                "lago_invoice_id": "invoice-1",
                "status": "checkout_created",
                "reused": false
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(BillingCommands::Topup {
            amount_credits: 50,
            idempotency_key: Some("topup-key-123".to_string()),
            owner_id: None,
            open: false,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("topup should succeed");
    }

    #[tokio::test]
    async fn verify_topup_flow_polls_until_exact_paid_balance_is_reconciled() {
        let base_url =
            spawn_verify_topup_server(vec![10, 11], topup_json(1, "verify-key-123")).await;

        let mut api = ApiClient::new(&base_url, "test-token".to_string()).expect("api client");
        let result = verify_topup_flow(
            &mut api,
            1,
            Some("verify-key-123".to_string()),
            None,
            false,
            Duration::from_secs(1),
            Duration::from_millis(1),
        )
        .await
        .expect("verification should succeed");

        assert!(result.verified);
        assert_eq!(result.starting_balance_credits, 10);
        assert_eq!(result.final_balance_credits, 11);
        assert_eq!(result.expected_balance_credits, 11);
        assert_eq!(result.topup.payment_provider.as_deref(), Some("stripe"));
    }

    #[tokio::test]
    async fn verify_topup_flow_rejects_double_credit_reconciliation() {
        let base_url =
            spawn_verify_topup_server(vec![10, 12], topup_json(1, "verify-key-456")).await;

        let mut api = ApiClient::new(&base_url, "test-token".to_string()).expect("api client");
        let error = verify_topup_flow(
            &mut api,
            1,
            Some("verify-key-456".to_string()),
            None,
            false,
            Duration::from_millis(5),
            Duration::from_millis(1),
        )
        .await
        .expect_err("double-credit reconciliation must fail");

        assert!(error.to_string().contains("expected=11"));
    }

    fn wallet_json(created: bool) -> serde_json::Value {
        serde_json::json!({
            "owner_id": "owner-1",
            "plan_kind": "prepaid",
            "collection_state": "good",
            "balance_credits": 100,
            "reserved_credits": 10,
            "pending_lago_debits": 5,
            "available_credits": 85,
            "available_with_overdraft_credits": 85,
            "has_payment_instrument": false,
            "overdraft_cap_credits": 0,
            "suspended": false,
            "lago_customer_id": "customer-1",
            "lago_subscription_id": "subscription-1",
            "lago_wallet_id": "wallet-1",
            "balance_synced_at": "2026-06-26T00:00:00Z",
            "created_at": "2026-06-26T00:00:00Z",
            "updated_at": "2026-06-26T00:00:00Z",
            "created": created
        })
    }

    #[derive(Clone)]
    struct VerifyTopupState {
        balances: std::sync::Arc<Vec<i64>>,
        wallet_reads: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        topup: std::sync::Arc<serde_json::Value>,
    }

    async fn spawn_verify_topup_server(balances: Vec<i64>, topup: serde_json::Value) -> String {
        async fn get_wallet(
            axum::extract::State(state): axum::extract::State<VerifyTopupState>,
        ) -> axum::Json<serde_json::Value> {
            let index = state
                .wallet_reads
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let balance = state
                .balances
                .get(index)
                .or_else(|| state.balances.last())
                .copied()
                .expect("test balances must not be empty");
            axum::Json(wallet_json_with_balance(balance))
        }

        async fn create_topup(
            axum::extract::State(state): axum::extract::State<VerifyTopupState>,
        ) -> axum::Json<serde_json::Value> {
            axum::Json((*state.topup).clone())
        }

        let state = VerifyTopupState {
            balances: std::sync::Arc::new(balances),
            wallet_reads: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            topup: std::sync::Arc::new(topup),
        };
        let app = axum::Router::new()
            .route("/api/v1/billing/wallet", axum::routing::get(get_wallet))
            .route("/api/v1/billing/topup", axum::routing::post(create_topup))
            .with_state(state);
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("test server addr");
        tokio::spawn(async move {
            axum::serve(listener, app).await.expect("test server");
        });
        format!("http://{addr}")
    }

    fn topup_json(amount_credits: i64, idempotency_key: &str) -> serde_json::Value {
        serde_json::json!({
            "owner_id": "owner-1",
            "amount_credits": amount_credits,
            "idempotency_key": idempotency_key,
            "checkout_url": "https://checkout.stripe.test/session",
            "payment_provider": "stripe",
            "lago_wallet_transaction_id": "txn-1",
            "lago_invoice_id": "invoice-1",
            "status": "checkout_created",
            "reused": false
        })
    }

    fn wallet_json_with_balance(balance_credits: i64) -> serde_json::Value {
        serde_json::json!({
            "owner_id": "owner-1",
            "plan_kind": "prepaid",
            "collection_state": "good",
            "balance_credits": balance_credits,
            "reserved_credits": 0,
            "pending_lago_debits": 0,
            "available_credits": balance_credits,
            "available_with_overdraft_credits": balance_credits,
            "has_payment_instrument": false,
            "overdraft_cap_credits": 0,
            "suspended": false,
            "lago_customer_id": "customer-1",
            "lago_subscription_id": "subscription-1",
            "lago_wallet_id": "wallet-1",
            "balance_synced_at": "2026-06-26T00:00:00Z",
            "created_at": "2026-06-26T00:00:00Z",
            "updated_at": "2026-06-26T00:00:00Z",
            "created": false
        })
    }
}
