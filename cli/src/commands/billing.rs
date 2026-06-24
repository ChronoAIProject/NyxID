use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{BillingCommands, OutputFormat};

pub async fn run(command: BillingCommands) -> Result<()> {
    match command {
        BillingCommands::Wallet { auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let wallet: Value = api.get("/billing/wallet").await?;
            print_wallet(output, &wallet)
        }
        BillingCommands::Usage { period, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let usage: Value = api
                .get(&format!("/usage?period={}", urlencoding::encode(&period)))
                .await?;
            print_usage(output, &usage)
        }
    }
}

fn print_wallet(output: OutputFormat, wallet: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(wallet)?),
        OutputFormat::Table => {
            eprintln!("Billing wallet");
            eprintln!("Owner:             {}", value_str(wallet, "owner_id"));
            eprintln!("Status:            {}", value_str(wallet, "status"));
            eprintln!(
                "Charging enabled:  {}",
                yes_no(wallet_bool(wallet, "charging_enabled"))
            );
            eprintln!(
                "Lago configured:   {}",
                yes_no(wallet_bool(wallet, "lago_configured"))
            );
            eprintln!(
                "Wallet configured: {}",
                yes_no(wallet_bool(wallet, "wallet_configured"))
            );
            eprintln!(
                "Available credits: {}",
                optional_credits(wallet.get("available_credits"))
            );
            eprintln!(
                "Balance credits:   {}",
                optional_credits(wallet.get("balance_credits"))
            );
            eprintln!(
                "Reserved credits:  {}",
                optional_credits(wallet.get("reserved_credits"))
            );
            eprintln!(
                "Pending debits:    {}",
                optional_credits(wallet.get("pending_lago_debits"))
            );
        }
    }
    Ok(())
}

fn print_usage(output: OutputFormat, usage: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(usage)?),
        OutputFormat::Table => {
            let totals = usage.get("totals").unwrap_or(&Value::Null);
            eprintln!("Billing usage");
            eprintln!("Owner:   {}", value_str(usage, "owner_id"));
            eprintln!("Period:  {}", value_str(usage, "period"));
            eprintln!(
                "Totals:  {} units, {} requests, {} bytes, {} events",
                value_i64(totals, "quantity").unwrap_or(0),
                value_i64(totals, "requests").unwrap_or(0),
                value_i64(totals, "bytes").unwrap_or(0),
                value_i64(totals, "events").unwrap_or(0)
            );
            if let Some(credits) = value_i64(totals, "estimated_credits_micros") {
                eprintln!("Approx credits: {}", format_micros(credits));
            }

            let rows = usage
                .get("rows")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            if rows.is_empty() {
                eprintln!("No billing usage for this period.");
                return Ok(());
            }

            let mut table = Table::new();
            table.load_preset(UTF8_FULL_CONDENSED);
            table.set_header([
                "Service",
                "Layer",
                "Metric",
                "Quantity",
                "Events",
                "Lago",
                "Approx Credits",
            ]);
            for row in rows {
                table.add_row([
                    value_str(row, "service_slug"),
                    value_str(row, "layer"),
                    value_str(row, "metric"),
                    value_i64(row, "quantity").unwrap_or(0).to_string(),
                    value_i64(row, "events").unwrap_or(0).to_string(),
                    if wallet_bool(row, "lago_acked") {
                        "acked".to_string()
                    } else {
                        "pending".to_string()
                    },
                    value_i64(row, "estimated_credits_micros")
                        .map(format_micros)
                        .unwrap_or_else(|| "-".to_string()),
                ]);
            }
            eprintln!("{table}");
        }
    }
    Ok(())
}

fn value_str(value: &Value, key: &str) -> String {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or("-")
        .to_string()
}

fn value_i64(value: &Value, key: &str) -> Option<i64> {
    value.get(key).and_then(Value::as_i64)
}

fn wallet_bool(value: &Value, key: &str) -> bool {
    value.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn yes_no(value: bool) -> &'static str {
    if value { "yes" } else { "no" }
}

fn optional_credits(value: Option<&Value>) -> String {
    value
        .and_then(Value::as_i64)
        .map(|credits| credits.to_string())
        .unwrap_or_else(|| "-".to_string())
}

fn format_micros(micros: i64) -> String {
    let sign = if micros < 0 { "-" } else { "" };
    let abs = micros.saturating_abs();
    let whole = abs / 1_000_000;
    let fractional = abs % 1_000_000;
    format!("{sign}{whole}.{fractional:06}")
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::cli::{BillingCommands, Cli, Commands, OutputFormat};
    use crate::test_support::mock_auth_with_output;
    use clap::Parser;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn billing_commands_parse() {
        let wallet =
            Cli::try_parse_from(["nyxid", "billing", "wallet"]).expect("billing wallet parses");
        assert!(matches!(
            wallet.command,
            Commands::Billing {
                command: BillingCommands::Wallet { .. }
            }
        ));

        let usage = Cli::try_parse_from(["nyxid", "billing", "usage", "--period", "7d"])
            .expect("billing usage parses");
        match usage.command {
            Commands::Billing {
                command: BillingCommands::Usage { period, .. },
            } => assert_eq!(period, "7d"),
            _ => panic!("expected billing usage command"),
        }
    }

    #[tokio::test]
    async fn wallet_fetches_billing_wallet() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/billing/wallet"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "owner_id": "owner-1",
                "charging_enabled": false,
                "lago_configured": true,
                "wallet_configured": false,
                "status": "not_configured",
                "balance_credits": null,
                "reserved_credits": null,
                "pending_lago_debits": null,
                "available_credits": null,
                "source": "usage_meter",
                "invoices": []
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(BillingCommands::Wallet {
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("wallet should succeed");
    }

    #[tokio::test]
    async fn usage_fetches_spec_usage_route_with_period() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/usage"))
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
                    "charging_enabled": false,
                    "lago_configured": true,
                    "source": "usage_meter",
                    "rates_are_approximate": true
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(BillingCommands::Usage {
            period: "7d".to_string(),
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("usage should succeed");
    }
}
