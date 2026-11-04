//! Strict readback for existing service charges used by prepaid CMA windows.
use super::*;

fn invalid() -> AppError {
    AppError::BillingProviderUnavailable("Sandbox provider price proof is invalid".into())
}

impl LagoClient {
    pub(super) async fn sandbox_subscription_impl(
        &self,
        subscription_id: &str,
        metric_codes: &[String],
    ) -> AppResult<SandboxProviderSnapshot> {
        if subscription_id.is_empty()
            || subscription_id != subscription_id.trim()
            || metric_codes.len() != 3
            || metric_codes
                .iter()
                .any(|code| code.is_empty() || code.len() > 255)
            || metric_codes
                .iter()
                .collect::<std::collections::HashSet<_>>()
                .len()
                != 3
        {
            return Err(invalid());
        }
        let value = self
            .get_one_by_external_id("subscriptions", "subscription", subscription_id)
            .await?
            .ok_or_else(invalid)?;
        let snapshot = subscription_prices(&value, subscription_id, metric_codes)?;
        for code in metric_codes {
            let path = format!("billable_metrics/{}", urlencoding::encode(code));
            let value = self
                .json_request(reqwest::Method::GET, &path, None)
                .await
                .map_err(lago_error_to_app)?;
            let metric = value.get("billable_metric").ok_or_else(invalid)?;
            if metric.get("code").and_then(Value::as_str) != Some(code.as_str())
                || metric.get("aggregation_type").and_then(Value::as_str) != Some("sum_agg")
                || metric.get("field_name").and_then(Value::as_str) != Some("quantity")
                || metric.get("recurring").and_then(Value::as_bool) != Some(false)
            {
                return Err(invalid());
            }
        }
        Ok(snapshot)
    }
}

fn subscription_prices(
    value: &Value,
    subscription_id: &str,
    metric_codes: &[String],
) -> AppResult<SandboxProviderSnapshot> {
    let subscription: RuntimeSubscription =
        serde_json::from_value(value.clone()).map_err(|_| invalid())?;
    if subscription.external_id != subscription_id
        || subscription.external_customer_id.trim().is_empty()
        || subscription.external_customer_id.trim() != subscription.external_customer_id
        || subscription.status != "active"
    {
        return Err(invalid());
    }
    let plan = value.get("plan").ok_or_else(invalid)?;
    if plan.get("code").and_then(Value::as_str) != Some(subscription.plan_code.as_str())
        || subscription.plan_code.is_empty()
        || plan.get("amount_currency").and_then(Value::as_str) != Some("USD")
    {
        return Err(invalid());
    }
    // Never fall back to the ordinary plan when the subscription's customized
    // charge array is missing. That would silently accept a different tariff.
    let charges = plan
        .get("charges")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let mut rates = Vec::with_capacity(metric_codes.len());
    for code in metric_codes {
        let matches: Vec<_> = charges
            .iter()
            .filter(|charge| {
                value_string(charge, &["billable_metric_code"])
                    .or_else(|| {
                        charge
                            .get("billable_metric")
                            .and_then(|metric| value_string(metric, &["code"]))
                    })
                    .as_deref()
                    == Some(code.as_str())
            })
            .collect();
        if matches.len() != 1 {
            return Err(invalid());
        }
        let charge = matches[0];
        if charge.get("charge_model").and_then(Value::as_str) != Some("standard")
            || charge.get("pay_in_advance").and_then(Value::as_bool) != Some(false)
            || charge.get("invoiceable").and_then(Value::as_bool) != Some(true)
            || charge.get("prorated").and_then(Value::as_bool) != Some(false)
            || charge.get("min_amount_cents").and_then(Value::as_i64) != Some(0)
            || charge
                .get("filters")
                .is_some_and(|v| !v.is_null() && v.as_array().is_none_or(|v| !v.is_empty()))
        {
            return Err(invalid());
        }
        let amount = charge
            .get("properties")
            .and_then(|p| p.get("amount"))
            .and_then(|amount| match amount {
                Value::String(s) => Some(s.clone()),
                Value::Number(n) => Some(n.to_string()),
                _ => None,
            })
            .ok_or_else(invalid)?;
        let price: Credits = amount.parse().map_err(|_| invalid())?;
        let pico = i64::try_from(price.pico()).map_err(|_| invalid())?;
        if !(0..=super::super::amounts::MAX_PRICE_PICO).contains(&pico) {
            return Err(invalid());
        }
        rates.push(PlanRate {
            lago_metric_code: code.clone(),
            credits_per_unit_pico: Some(pico),
            credits_per_unit_micros: pico / 1_000_000,
        });
    }
    Ok(SandboxProviderSnapshot {
        subscription,
        rates,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn effective_subscription_requires_every_nonrecurring_sum_quantity_metric() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        let (subscription, codes) = fixture();
        Mock::given(method("GET"))
            .and(path("/api/v1/subscriptions/sub"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"subscription":subscription})),
            )
            .mount(&server)
            .await;
        for code in &codes {
            Mock::given(method("GET")).and(path(format!("/api/v1/billable_metrics/{code}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"billable_metric":{
                    "code":code,"aggregation_type":"sum_agg","field_name":"quantity","recurring":false
                }}))).mount(&server).await;
        }
        let client = LagoClient::new(server.uri(), "test-only".into()).unwrap();
        assert_eq!(
            client
                .sandbox_subscription("sub", &codes)
                .await
                .unwrap()
                .rates
                .len(),
            3
        );
        server.reset().await;
        let (subscription, codes) = fixture();
        Mock::given(method("GET"))
            .and(path("/api/v1/subscriptions/sub"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"subscription":subscription})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path("/api/v1/billable_metrics/cpu"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"billable_metric":{
                "code":"cpu","aggregation_type":"sum_agg","field_name":"quantity","recurring":true
            }}))).mount(&server).await;
        assert!(client.sandbox_subscription("sub", &codes).await.is_err());
    }

    fn fixture() -> (Value, Vec<String>) {
        let codes = ["cpu", "memory", "disk"].map(str::to_owned).to_vec();
        let charges: Vec<_> = codes
            .iter()
            .map(|code| {
                json!({
                    "billable_metric_code":code,"charge_model":"standard","pay_in_advance":false,
                    "invoiceable":true,"prorated":false,"min_amount_cents":0,"filters":[],
                    "properties":{"amount":"0.000000010001"}
                })
            })
            .collect();
        (
            json!({"external_id":"sub","external_customer_id":"owner","plan_code":"starter",
            "status":"active","plan":{"code":"starter","amount_currency":"USD","charges":charges}}),
            codes,
        )
    }

    #[test]
    fn uses_exact_effective_subscription_price() {
        let (value, codes) = fixture();
        let proof = subscription_prices(&value, "sub", &codes).unwrap();
        assert_eq!(proof.rates.len(), 3);
        assert_eq!(proof.rates[0].credits_per_unit_pico, Some(10001));
        assert_eq!(proof.rates[0].credits_per_unit_micros, 0);
    }

    #[test]
    fn rejects_missing_or_mismatched_subscription_binding_and_charge_array() {
        let (value, codes) = fixture();
        for (key, replacement) in [
            ("external_id", json!("other")),
            ("status", json!("terminated")),
            ("plan", Value::Null),
        ] {
            let mut changed = value.clone();
            changed[key] = replacement;
            assert!(subscription_prices(&changed, "sub", &codes).is_err());
        }
        let mut changed = value.clone();
        changed["plan"]["code"] = json!("different");
        assert!(subscription_prices(&changed, "sub", &codes).is_err());
        let mut changed = value;
        changed["plan"]["charges"] = json!([]);
        assert!(subscription_prices(&changed, "sub", &codes).is_err());
    }

    #[test]
    fn rejects_duplicate_filtered_nonstandard_and_invalid_prices() {
        let (value, codes) = fixture();
        for (key, replacement) in [
            ("charge_model", json!("graduated")),
            ("pay_in_advance", json!(true)),
            ("invoiceable", json!(false)),
            ("prorated", json!(true)),
            ("min_amount_cents", json!(1)),
            ("filters", json!([{"properties":{"amount":"1"}}])),
            ("properties", json!({"amount":"0.0000000000001"})),
            ("properties", json!({"amount":"-1"})),
        ] {
            let mut changed = value.clone();
            changed["plan"]["charges"][0][key] = replacement;
            assert!(subscription_prices(&changed, "sub", &codes).is_err());
        }
        let mut changed = value;
        let duplicate = changed["plan"]["charges"][0].clone();
        changed["plan"]["charges"]
            .as_array_mut()
            .unwrap()
            .push(duplicate);
        assert!(subscription_prices(&changed, "sub", &codes).is_err());
    }
}
