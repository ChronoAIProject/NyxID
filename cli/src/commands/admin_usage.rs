use anyhow::{Result, bail};
use chrono::Duration;
use serde_json::Value;

use super::billing_support as support;
use crate::cli::AdminUsageArgs;

pub async fn run(args: AdminUsageArgs) -> Result<()> {
    support::require_human(&args.auth)?;
    let mut query = vec![
        ("page", args.page.to_string()),
        ("per_page", args.per_page.to_string()),
    ];
    match (&args.from, &args.to, &args.period) {
        (Some(from), Some(to), None) => {
            let from = support::timestamp(from)?;
            let to = support::timestamp(to)?;
            if from >= to || to - from > Duration::days(31) {
                bail!("Usage window must be positive and at most 31 days");
            }
            query.extend([
                ("from", support::rfc3339(from)),
                ("to", support::rfc3339(to)),
            ]);
        }
        (None, None, period) => {
            let period = period.as_deref().unwrap_or("24h");
            if !["24h", "7d", "30d"].contains(&period) {
                bail!("period must be 24h, 7d or 30d");
            }
            query.push(("period", period.to_owned()));
        }
        _ => bail!("Use either period or from + to"),
    }
    if args.page == 0
        || !(1..=100).contains(&args.per_page)
        || args
            .page
            .checked_sub(1)
            .and_then(|n| n.checked_mul(args.per_page))
            .is_none_or(|n| n > i64::MAX as u64)
    {
        bail!("Invalid usage pagination");
    }
    if let Some(user) = &args.user {
        query.push(("user", support::uuid(user)?));
    }
    let mut api = support::human_api(&args.auth).await?;
    if let Some(service) = &args.service {
        let row = support::service(&mut api, service).await?;
        query.push(("service", support::text(&row["id"])));
    }
    let query = query
        .into_iter()
        .map(|(key, value)| format!("{key}={}", urlencoding::encode(&value)))
        .collect::<Vec<_>>()
        .join("&");
    let value: Value = api.get(&format!("/admin/usage?{query}")).await?;
    support::print(&value, args.auth.output, || human_output(&value))
}

fn money(usage: &Value, key: &str) -> String {
    usage[key]
        .as_str()
        .map(str::to_owned)
        .or_else(|| {
            usage[format!("{key}_micros")].as_i64().map(|micros| {
                let pico = i128::from(micros) * 1_000_000;
                support::credits(pico)
            })
        })
        .unwrap_or_else(|| "unknown".into())
}

fn usage_row(service: &str, class: &str, usage: &Value) -> Vec<String> {
    let mut row = vec![service.into(), class.into()];
    for metric in ["input_tokens", "output_tokens", "cache_read_tokens"] {
        row.push(support::text(&usage["quantities"][metric]));
    }
    for cost in ["gross_cost", "wallet_cost", "grant_cost", "allowance_cost"] {
        row.push(money(usage, cost));
    }
    row
}

fn human_output(value: &Value) -> String {
    let mut rows = vec![usage_row("Total", "all", &value["totals"])];
    for service in value["by_service"].as_array().into_iter().flatten() {
        let name = support::text(&service["service_slug"]);
        rows.push(usage_row(&name, "all", service));
        for class in service["by_credential_class"]
            .as_array()
            .into_iter()
            .flatten()
        {
            rows.push(usage_row(
                &name,
                &support::text(&class["credential_class"]),
                class,
            ));
        }
    }
    support::table(
        &[
            "Service",
            "Credential class",
            "Input tokens",
            "Output tokens",
            "Cache read tokens",
            "Gross credits",
            "Wallet credits",
            "Grant credits",
            "Allowance credits",
        ],
        rows,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_auth;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path, query_param},
    };
    fn args(server: &MockServer) -> AdminUsageArgs {
        AdminUsageArgs {
            period: None,
            from: None,
            to: None,
            service: None,
            user: None,
            page: 1,
            per_page: 25,
            auth: mock_auth(server.uri()),
        }
    }

    #[tokio::test]
    async fn usage_sends_bounded_window_service_uuid_user_and_pagination() {
        let server = MockServer::start().await;
        let id = "00000000-0000-4000-8000-000000000001";
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/services/{id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":id})))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/admin/usage"))
            .and(query_param("service", id))
            .and(query_param("user", id))
            .and(query_param("from", "2026-10-19T00:00:00Z"))
            .and(query_param("to", "2026-10-20T00:00:00Z"))
            .and(query_param("page", "2"))
            .and(query_param("per_page", "50"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
            .expect(1)
            .mount(&server)
            .await;
        let mut args = args(&server);
        args.from = Some("2026-10-19T00:00:00Z".into());
        args.to = Some("2026-10-20T00:00:00Z".into());
        args.service = Some(id.into());
        args.user = Some(id.into());
        args.page = 2;
        args.per_page = 50;
        run(args).await.unwrap();
    }

    #[tokio::test]
    async fn usage_periods_and_invalid_windows_are_validated_locally() {
        for period in ["24h", "7d", "30d"] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path("/api/v1/admin/usage"))
                .and(query_param("period", period))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
                .expect(1)
                .mount(&server)
                .await;
            let mut args = args(&server);
            args.period = Some(period.into());
            run(args).await.unwrap();
        }
        let server = MockServer::start().await;
        for (from, to, period) in [
            (
                Some("2026-01-01T00:00:00Z"),
                Some("2026-02-02T00:00:00Z"),
                None,
            ),
            (
                Some("2026-01-02T00:00:00Z"),
                Some("2026-01-01T00:00:00Z"),
                None,
            ),
            (Some("bad"), Some("bad"), None),
            (Some("2026-01-01T00:00:00Z"), None, None),
            (
                Some("2026-01-01T00:00:00Z"),
                Some("2026-01-02T00:00:00Z"),
                Some("24h"),
            ),
        ] {
            let mut args = args(&server);
            args.from = from.map(str::to_owned);
            args.to = to.map(str::to_owned);
            args.period = period.map(str::to_owned);
            assert!(run(args).await.is_err());
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }

    #[test]
    fn usage_prints_token_classes_and_all_exact_costs_without_float_conversion() {
        let usage = json!({
            "quantities": {
                "input_tokens": 91,
                "output_tokens": 5,
                "cache_read_tokens": 10
            },
            "gross_cost": "123456789012345.123456789012",
            "wallet_cost": "0.000000000001",
            "grant_cost": "0.000000000002",
            "allowance_cost": "0.000000000003"
        });
        let mut service = usage.clone();
        service["service_slug"] = json!("telnyx-platform");
        service["gross_cost"] = json!("0.123456789012");
        let mut class = usage.clone();
        class["credential_class"] = json!("nyxid_managed_master");
        class["gross_cost"] = json!("0.987654321098");
        service["by_credential_class"] = json!([class]);
        let output = human_output(&json!({"totals":usage,"by_service":[service]}));
        for exact in [
            "123456789012345.123456789012",
            "0.000000000001",
            "0.000000000002",
            "0.000000000003",
            "nyxid_managed_master",
            "telnyx-platform",
            "0.123456789012",
            "0.987654321098",
            "91",
        ] {
            assert!(output.contains(exact), "{exact}");
        }
        assert_eq!(
            money(&json!({"gross_cost_micros":1}), "gross_cost"),
            "0.000001"
        );
    }
}
