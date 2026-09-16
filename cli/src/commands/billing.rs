use std::time::{Duration, Instant};

use anyhow::{Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::api::ApiClient;
use crate::cli::{BillingCommands, BillingUsagePeriodArg, OutputFormat};

#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Serialize)]
pub struct BillingWalletResponse {
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
    pub wallet_credits_micros: Option<i64>,
    pub grant_credits_micros: Option<i64>,
    pub allowance_credits_micros: Option<i64>,
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
    pub wallet_credits_micros: Option<i64>,
    pub grant_credits_micros: Option<i64>,
    pub allowance_credits_micros: Option<i64>,
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
    let deadline = Instant::now() + timeout;
    let mut final_wallet = starting_wallet.clone();
    while Instant::now() < deadline {
        let wallet: BillingWalletResponse = api.get("/billing/wallet").await?;
        if wallet.balance_credits == starting_wallet.balance_credits + amount_credits {
            final_wallet = wallet;
            return Ok(TopUpVerificationResponse {
                topup,
                starting_balance_credits: starting_wallet.balance_credits,
                final_balance_credits: final_wallet.balance_credits,
                expected_balance_credits: starting_wallet.balance_credits + amount_credits,
                verified: true,
            });
        }
        final_wallet = wallet;
        tokio::time::sleep(poll_interval).await;
    }

    bail!(
        "timed out waiting for Lago/Stripe reconciliation: start={} final={} expected={}",
        starting_wallet.balance_credits,
        final_wallet.balance_credits,
        starting_wallet.balance_credits + amount_credits
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
            eprintln!("Balance:           {} credits", wallet.balance_credits);
            eprintln!("Available:         {} credits", wallet.available_credits);
            eprintln!("Reserved:          {} credits", wallet.reserved_credits);
            eprintln!("Pending Debits:    {} credits", wallet.pending_lago_debits);
            eprintln!(
                "Overdraft Cap:     {} credits",
                wallet.overdraft_cap_credits
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
            format_estimated_credits(usage.totals.estimated_credits_micros)
        ),
    ];
    for (label, micros, quantity) in [
        ("Funded by grants", usage.totals.grant_credits_micros, 0),
        (
            "Funded by allowances",
            usage.totals.allowance_credits_micros,
            usage.totals.allowance_quantity,
        ),
        ("Charged to wallet", usage.totals.wallet_credits_micros, 0),
    ] {
        if micros.is_some_and(|value| value > 0) || quantity > 0 {
            lines.push(format!("{label}: {}", format_estimated_credits(micros)));
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
            row.metric.clone(),
            row.quantity.to_string(),
            row.events.to_string(),
            if row.billable {
                format_estimated_credits(row.estimated_credits_micros)
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
    let grants = row.grant_credits_micros.is_some_and(|value| value > 0);
    let allowances =
        row.allowance_credits_micros.is_some_and(|value| value > 0) || row.allowance_quantity > 0;
    if !row.billable || !(grants || allowances) {
        return String::new();
    }
    let mut parts = Vec::new();
    if grants {
        parts.push(format!(
            "grants {}",
            format_estimated_credits(row.grant_credits_micros)
        ));
    }
    if allowances {
        let units = if row.allowance_quantity > 0 {
            format!(" ({} {})", row.allowance_quantity, row.metric)
        } else {
            String::new()
        };
        parts.push(format!(
            "allowance {}{units}",
            format_estimated_credits(row.allowance_credits_micros)
        ));
    }
    parts.push(format!(
        "wallet {}",
        format_estimated_credits(row.wallet_credits_micros)
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
            eprintln!(
                "Start Balance:     {} credits",
                response.starting_balance_credits
            );
            eprintln!(
                "Final Balance:     {} credits",
                response.final_balance_credits
            );
            eprintln!(
                "Expected Balance:  {} credits",
                response.expected_balance_credits
            );
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

fn format_estimated_credits(value: Option<i64>) -> String {
    match value {
        Some(micros) => format!("{:.6} credits", micros as f64 / 1_000_000.0),
        None => "-".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{BillingCommands, BillingUsagePeriodArg, OutputFormat};
    use crate::test_support::{mock_auth, mock_auth_with_output};
    use wiremock::matchers::{body_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

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
