use crate::{
    api::ApiClient,
    cli::{CatalogServiceArgs, OutputFormat},
    org_resolver::resolve_org_id,
};
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

impl CatalogServiceArgs {
    pub fn is_requested(&self) -> bool {
        self.catalog_admin
            || self.inference_protocol.is_some()
            || self.inference_model_list.is_some()
            || self.inference_realtime.is_some()
            || self.platform_key_enabled.is_some()
            || self.platform_key_audience.is_some()
            || !self.platform_key_allow.is_empty()
            || !self.platform_key_deny.is_empty()
            || self.byok_metric.is_some()
            || self.byok_price.is_some()
            || self.byok_free
            || self.platform_key_metric.is_some()
            || self.platform_key_price.is_some()
            || self.platform_key_free
    }
    pub async fn apply(&self, api: &mut ApiClient, body: &mut Value) -> Result<()> {
        self.apply_update(api, &json!({}), body).await
    }
    pub async fn apply_update(
        &self,
        api: &mut ApiClient,
        current: &Value,
        body: &mut Value,
    ) -> Result<()> {
        if self.inference_protocol.as_deref() == Some("none") {
            if self.inference_model_list.is_some() || self.inference_realtime.is_some() {
                bail!("Cannot clear inference and set its capabilities together");
            }
            body["inference"] = Value::Null;
        } else if self.inference_protocol.is_some()
            || self.inference_model_list.is_some()
            || self.inference_realtime.is_some()
        {
            let mut block = current["inference"].clone();
            if !block.is_object() {
                block = json!({"model_list": false, "realtime": false});
            }
            if let Some(protocol) = &self.inference_protocol {
                block["wire_protocol"] = protocol.clone().into();
            }
            if !block["wire_protocol"].is_string() {
                bail!("--inference-protocol is required when adding inference");
            }
            if let Some(value) = self.inference_model_list {
                block["model_list"] = value.into();
            }
            if let Some(value) = self.inference_realtime {
                block["realtime"] = value.into();
            }
            body["inference"] = block;
        }
        if self.platform_key_enabled.is_some()
            || self.platform_key_audience.is_some()
            || !self.platform_key_allow.is_empty()
            || !self.platform_key_deny.is_empty()
        {
            let mut block = current["platform_key"].clone();
            if !block.is_object() {
                block = json!({ "enabled": current["legacy_public_master"] == true, "audience": if current["legacy_public_master"] == true { "public" } else { "restricted" }, "allowed_owner_ids": [] });
            }
            if let Some(value) = self.platform_key_enabled {
                block["enabled"] = value.into();
            }
            if let Some(value) = &self.platform_key_audience {
                block["audience"] = value.clone().into();
            }
            let mut owners: Vec<String> = block["allowed_owner_ids"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            for owner in &self.platform_key_allow {
                owners.push(resolve_org_id(api, owner).await?);
            }
            for owner in &self.platform_key_deny {
                let id = resolve_org_id(api, owner).await?;
                owners.retain(|owner| owner != &id);
            }
            owners.sort();
            owners.dedup();
            block["allowed_owner_ids"] = json!(owners);
            body["platform_key"] = block;
        }
        let mut billing = current["billing"].clone();
        if !billing.is_object() {
            billing = json!({});
        }
        let mut changed = false;
        for (field, metric, price, free) in [
            (
                "byok_pricing",
                &self.byok_metric,
                &self.byok_price,
                self.byok_free,
            ),
            (
                "platform_key_pricing",
                &self.platform_key_metric,
                &self.platform_key_price,
                self.platform_key_free,
            ),
        ] {
            if free {
                billing[field] = Value::Null;
                changed = true;
            } else if metric.is_some() || price.is_some() {
                let mut lane = billing[field].clone();
                if !lane.is_object() {
                    lane = json!({"metric":"requests"});
                }
                if let Some(metric) = metric {
                    lane["metric"] = metric.clone().into();
                }
                if let Some(price) = price {
                    lane["credits_per_unit"] = price.clone().into();
                }
                if !lane["credits_per_unit"].is_string() {
                    bail!("A lane price is required when enabling charging");
                }
                billing[field] = lane;
                changed = true;
            }
        }
        if changed {
            body["billing"] = billing;
        }
        Ok(())
    }
}

pub fn credential_from_env(name: Option<&str>) -> Result<Option<String>> {
    name.map(|name| {
        let value = std::env::var(name)
            .with_context(|| format!("Credential environment variable {name} is not set"))?;
        if value.trim().is_empty() {
            bail!("Credential environment variable must not be empty");
        }
        Ok(value)
    })
    .transpose()
}
pub fn print_result(value: &Value, output: OutputFormat) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(value)?),
        OutputFormat::Table => eprintln!(
            "Service saved: {} ({})",
            value["slug"].as_str().unwrap_or("-"),
            super::credential_binding(value)
        ),
    }
    Ok(())
}
#[allow(clippy::too_many_arguments)]
pub async fn connect_platform(
    api: &mut ApiClient,
    slug: &str,
    label: Option<&str>,
    custom_slug: Option<&str>,
    org: Option<&str>,
    admin_only: bool,
    output: OutputFormat,
) -> Result<()> {
    let mut body = json!({ "service_slug": slug, "label": label.unwrap_or(slug), "use_platform_key": true, "admin_only": admin_only });
    if let Some(slug) = custom_slug {
        body["slug"] = slug.into();
    }
    if let Some(org) = org {
        body["target_org_id"] = resolve_org_id(api, org).await?.into();
    }
    let result: Value = api.post("/keys", &body).await?;
    print_result(&result, output)
}

pub(crate) fn inference_summary(value: &Value) -> String {
    let block = &value["inference"];
    let Some(protocol) = block["wire_protocol"].as_str() else {
        return "-".into();
    };
    format!(
        "{} / {}{}{}{}",
        protocol,
        block["binding"].as_str().unwrap_or("user"),
        if block["model_list"] == true {
            " / models"
        } else {
            ""
        },
        if block["realtime"] == true {
            " / realtime"
        } else {
            ""
        },
        block["status_slug"]
            .as_str()
            .map(|s| format!(" / status: {s}"))
            .unwrap_or_default()
    )
}

pub(crate) fn binding_prompt(entry: &Value) -> String {
    if entry["platform_key"]["pricing"].is_null()
        && entry["byok_pricing"].is_null()
        && entry["billing"]["platform_billable"] == true
    {
        return "Use NyxID's key? Current service/plan pricing applies [Y/n]".into();
    }
    format!(
        "Use NyxID's key ({}) instead of your own key ({})? [Y/n]",
        lane_price_label(Some(&entry["platform_key"]["pricing"])),
        lane_price_label(Some(&entry["byok_pricing"]))
    )
}

pub(crate) fn lane_price_label(value: Option<&Value>) -> String {
    let Some(value) = value else {
        return "not configured".into();
    };
    let Some(amount) = value["credits_per_unit"].as_str() else {
        return "free".into();
    };
    let unit = match value["metric"].as_str() {
        Some("tokens") => "token",
        Some("requests") => "request",
        Some("bytes") => "byte",
        _ => "unit",
    };
    let status = match value["sync_status"].as_str() {
        Some("synced") | None => "",
        _ => " (price pending; current billing applies)",
    };
    format!("{amount} credits / {unit}{status}")
}

pub(crate) fn platform_config_label(value: &Value) -> String {
    if value["platform_key"].is_null() {
        return if value["legacy_public_master"] == true {
            "enabled, public (implicit)"
        } else {
            "not configured"
        }
        .into();
    }
    format!(
        "{}, {}",
        if value["platform_key"]["enabled"] == true {
            "enabled"
        } else {
            "disabled"
        },
        value["platform_key"]["audience"]
            .as_str()
            .unwrap_or("restricted")
    )
}

pub(crate) fn catalog_error(error: anyhow::Error) -> anyhow::Error {
    if error
        .downcast_ref::<crate::api::ApiError>()
        .is_some_and(|e| e.status() == reqwest::StatusCode::NOT_FOUND)
    {
        error.context("--catalog-admin requires a catalog service ID or catalog slug; a connection ID cannot identify the admin catalog row")
    } else {
        error
    }
}

pub(crate) async fn fetch_catalog_service(api: &mut ApiClient, reference: &str) -> Result<Value> {
    match api.get(&format!("/services/{reference}")).await {
        Ok(service) => Ok(service),
        Err(error)
            if uuid::Uuid::parse_str(reference).is_err()
                && error
                    .downcast_ref::<crate::api::ApiError>()
                    .is_some_and(|e| e.status() == reqwest::StatusCode::NOT_FOUND) =>
        {
            // Discovery entries intentionally omit catalog IDs. Resolve a
            // listed slug from admin service responses, which carry the ID
            // required for catalog updates. Unlisted rows can still use IDs.
            let listing: Value = api.get("/services").await.map_err(catalog_error)?;
            listing["services"]
                .as_array()
                .and_then(|rows| rows.iter().find(|row| row["slug"] == reference))
                .cloned()
                .ok_or_else(|| catalog_error(error))
        }
        Err(error) => Err(catalog_error(error)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{Cli, Commands, ServiceCommands};
    use clap::Parser;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };
    #[test]
    fn platform_and_admin_flags_parse_and_conflict() {
        let cli =
            Cli::try_parse_from(["nyxid", "service", "add", "llm-xai", "--platform-key"]).unwrap();
        assert!(matches!(
            cli.command,
            Commands::Service {
                command: ServiceCommands::Add {
                    platform_key: true,
                    ..
                }
            }
        ));
        assert!(matches!(
            Cli::try_parse_from(["nyxid", "keys"]).unwrap().command,
            Commands::Keys(_)
        ));
        for flag in [
            "--oauth",
            "--device-code",
            "--custom",
            "--ws-frame-clear",
            "--no-wait",
        ] {
            assert!(
                Cli::try_parse_from(["nyxid", "service", "add", "llm-xai", "--platform-key", flag])
                    .is_err()
            );
        }
        assert!(
            Cli::try_parse_from([
                "nyxid",
                "service",
                "update",
                "id",
                "--use-platform-key",
                "--use-own-key"
            ])
            .is_err()
        );
        assert!(
            Cli::try_parse_from([
                "nyxid",
                "service",
                "update",
                "id",
                "--inference-protocol",
                "unknown"
            ])
            .is_err()
        );
        let cli = Cli::try_parse_from([
            "nyxid",
            "service",
            "update",
            "id",
            "--platform-key-enabled",
            "true",
            "--platform-key-audience",
            "restricted",
            "--byok-free",
            "--platform-key-price",
            "0.125",
            "--platform-key-metric",
            "tokens",
        ])
        .unwrap();
        assert!(
            matches!(cli.command, Commands::Service { command: ServiceCommands::Update { catalog, .. } } if catalog.is_requested() && catalog.byok_free)
        );
    }
    #[tokio::test]
    async fn platform_connection_uses_only_unified_provisioning() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/api/v1/keys"))
            .and(body_json(json!({ "service_slug":"llm-xai", "label":"llm-xai", "use_platform_key":true, "admin_only":false })))
            .respond_with(ResponseTemplate::new(201).set_body_json(json!({"slug":"llm-xai", "credential_binding":"platform"})))
            .expect(1).mount(&server).await;
        let auth = crate::test_support::mock_auth(server.uri());
        let mut api = ApiClient::from_auth_checked(&auth).await.unwrap();
        connect_platform(&mut api, "llm-xai", None, None, None, false, auth.output)
            .await
            .unwrap();
    }
    #[tokio::test]
    async fn admin_lane_update_preserves_other_charges_and_restricted_owners() {
        let server = MockServer::start().await;
        let auth = crate::test_support::mock_auth(server.uri());
        let mut api = ApiClient::from_auth_checked(&auth).await.unwrap();
        let owner = uuid::Uuid::new_v4().to_string();
        let current = json!({"platform_key":{"enabled":true,"audience":"restricted","allowed_owner_ids":[owner]},
            "billing":{"resale_billable":true,"platform_billable":true,"platform_key_pricing":{"metric":"tokens","credits_per_unit":"2"}}});
        let args = CatalogServiceArgs {
            byok_price: Some("0.1".into()),
            platform_key_deny: vec![owner],
            ..Default::default()
        };
        let mut body = json!({});
        args.apply_update(&mut api, &current, &mut body)
            .await
            .unwrap();
        assert_eq!(body["billing"]["resale_billable"], true);
        assert_eq!(
            body["billing"]["platform_key_pricing"],
            current["billing"]["platform_key_pricing"]
        );
        assert_eq!(body["platform_key"]["allowed_owner_ids"], json!([]));
        assert_eq!(body["billing"]["byok_pricing"]["credits_per_unit"], "0.1");
    }
    #[test]
    fn renders_inference_and_old_binding_fallback() {
        assert_eq!(inference_summary(&json!({})), "-");
        let summary = inference_summary(
            &json!({"inference":{"wire_protocol":"openai_completions","binding":"user","status_slug":"xai","model_list":true,"realtime":true}}),
        );
        assert_eq!(
            summary,
            "openai_completions / user / models / realtime / status: xai"
        );
        assert_eq!(
            super::super::credential_binding(
                &json!({"credential_binding":"platform","api_key_id":"retained"})
            ),
            "platform"
        );
        assert_eq!(super::super::credential_binding(&json!({})), "user");
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::cli::{Cli, Commands, ServiceCommands};
    use clap::Parser;
    #[tokio::test]
    async fn catalog_slug_resolves_admin_service_list_without_discovery_ids() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/services/chrono-llm-public"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let row =
            json!({"id":"catalog-id", "slug":"chrono-llm-public", "legacy_public_master":true});
        Mock::given(method("GET"))
            .and(path("/api/v1/services"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"services":[row.clone()]})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut api = ApiClient::new(&server.uri(), "token".into()).unwrap();
        assert_eq!(
            fetch_catalog_service(&mut api, "chrono-llm-public")
                .await
                .unwrap(),
            row
        );
    }
    #[test]
    fn price_labels_cover_free_missing_pending_and_synced() {
        assert_eq!(lane_price_label(None), "not configured");
        assert_eq!(lane_price_label(Some(&Value::Null)), "free");
        assert_eq!(
            lane_price_label(Some(
                &json!({"metric":"tokens","credits_per_unit":"0.01","sync_status":"synced"})
            )),
            "0.01 credits / token"
        );
        for status in ["pending", "failed"] {
            assert_eq!(
                lane_price_label(Some(
                    &json!({"metric":"requests","credits_per_unit":"2","sync_status":status})
                )),
                "2 credits / request (price pending; current billing applies)"
            );
        }
    }
    #[test]
    fn admin_show_parses_and_labels_implicit_public_config() {
        let cli = Cli::try_parse_from([
            "nyxid",
            "service",
            "show",
            "chrono-llm-public",
            "--catalog-admin",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Commands::Service {
                command: ServiceCommands::Show {
                    catalog_admin: true,
                    ..
                }
            }
        ));
        assert_eq!(
            platform_config_label(&json!({"legacy_public_master":true,"platform_key":null})),
            "enabled, public (implicit)"
        );
        assert_eq!(
            platform_config_label(&json!({"platform_key":{"enabled":false,"audience":"public"}})),
            "disabled, public"
        );
    }
    #[tokio::test]
    async fn catalog_404_explains_catalog_identity_and_implicit_updates_preserve_public() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        let id = "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa";
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/services/{id}")))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        let mut api = ApiClient::new(&server.uri(), "token".into()).unwrap();
        let error = fetch_catalog_service(&mut api, id).await.unwrap_err();
        assert!(error.to_string().contains("catalog service ID"));
        let args = CatalogServiceArgs {
            platform_key_enabled: Some(true),
            ..Default::default()
        };
        let mut body = json!({});
        args.apply_update(&mut api, &json!({"legacy_public_master":true}), &mut body)
            .await
            .unwrap();
        assert_eq!(body["platform_key"]["audience"], "public");
    }
}
