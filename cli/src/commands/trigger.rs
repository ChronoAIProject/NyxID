use anyhow::{Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::{Value, json};

use crate::api::ApiClient;
use crate::cli::{
    OutputFormat, TriggerCommands, TriggerDeliveryArg, TriggerStatusArg, TriggerVerificationArg,
};
use crate::org_resolver::resolve_org_id;

pub async fn run(command: TriggerCommands) -> Result<()> {
    match command {
        TriggerCommands::Create {
            label,
            user_service_id,
            verification,
            signature_header,
            delivery,
            delivery_url,
            conversation_id,
            org,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let target_org_id = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };
            let body = create_body(
                &label,
                user_service_id.as_deref(),
                verification,
                &signature_header,
                delivery,
                delivery_url.as_deref(),
                conversation_id.as_deref(),
                target_org_id.as_deref(),
            )?;
            let result: Value = api.post("/triggers", &body).await?;
            print_created(output, &result)
        }
        TriggerCommands::List { org, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let path = match org {
                Some(raw) => {
                    let org_id = resolve_org_id(&mut api, &raw).await?;
                    format!("/triggers?org_id={}", urlencoding::encode(&org_id))
                }
                None => "/triggers".to_string(),
            };
            let result: Value = api.get(&path).await?;
            print_list(output, &result)
        }
        TriggerCommands::Show { id, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api
                .get(&format!("/triggers/{}", urlencoding::encode(&id)))
                .await?;
            print_one(output, &result)
        }
        TriggerCommands::Update {
            id,
            label,
            status,
            delivery,
            delivery_url,
            conversation_id,
            auth,
        } => {
            let output = auth.output;
            let mut body = json!({});
            if let Some(label) = label {
                body["label"] = Value::String(label);
            }
            if let Some(status) = status {
                body["status"] = Value::String(
                    match status {
                        TriggerStatusArg::Active => "active",
                        TriggerStatusArg::Disabled => "disabled",
                    }
                    .to_string(),
                );
            }
            if let Some(delivery) = delivery {
                body["delivery"] = delivery_value(
                    delivery,
                    delivery_url.as_deref(),
                    conversation_id.as_deref(),
                )?;
            } else if delivery_url.is_some() || conversation_id.is_some() {
                bail!("--delivery is required with --delivery-url or --conversation-id");
            }
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api
                .patch(&format!("/triggers/{}", urlencoding::encode(&id)), &body)
                .await?;
            print_updated(output, &result)
        }
        TriggerCommands::Delete { id, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.delete_empty(&format!("/triggers/{}", urlencoding::encode(&id)))
                .await?;
            match output {
                OutputFormat::Json => println!("{}", json!({ "deleted": true, "id": id })),
                OutputFormat::Table => println!("Trigger {id} deleted."),
            }
            Ok(())
        }
        TriggerCommands::RotateSecret { id, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api
                .post(
                    &format!("/triggers/{}/rotate-secret", urlencoding::encode(&id)),
                    &json!({}),
                )
                .await?;
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
                OutputFormat::Table => {
                    println!(
                        "Inbound secret: {}",
                        result.get("secret").and_then(Value::as_str).unwrap_or("-")
                    );
                }
            }
            Ok(())
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn create_body(
    label: &str,
    user_service_id: Option<&str>,
    verification: TriggerVerificationArg,
    signature_header: &str,
    delivery: TriggerDeliveryArg,
    delivery_url: Option<&str>,
    conversation_id: Option<&str>,
    target_org_id: Option<&str>,
) -> Result<Value> {
    let verification = match verification {
        TriggerVerificationArg::Bearer => json!({ "mode": "token", "location": "bearer" }),
        TriggerVerificationArg::Query => json!({ "mode": "token", "location": "query" }),
        TriggerVerificationArg::Hmac => {
            json!({ "mode": "hmac_sha256", "header_name": signature_header })
        }
    };
    Ok(json!({
        "label": label,
        "user_service_id": user_service_id,
        "verification": verification,
        "delivery": delivery_value(delivery, delivery_url, conversation_id)?,
        "target_org_id": target_org_id,
    }))
}

fn delivery_value(
    delivery: TriggerDeliveryArg,
    delivery_url: Option<&str>,
    conversation_id: Option<&str>,
) -> Result<Value> {
    match delivery {
        TriggerDeliveryArg::Webhook => Ok(json!({
            "type": "webhook",
            "url": delivery_url.ok_or_else(|| anyhow::anyhow!(
                "--delivery-url is required for webhook delivery"
            ))?,
        })),
        TriggerDeliveryArg::Agent => Ok(json!({
            "type": "agent",
            "conversation_id": conversation_id.ok_or_else(|| anyhow::anyhow!(
                "--conversation-id is required for agent delivery"
            ))?,
        })),
        TriggerDeliveryArg::Notification => {
            if delivery_url.is_some() || conversation_id.is_some() {
                bail!("notification delivery does not accept a URL or conversation ID");
            }
            Ok(json!({ "type": "notification" }))
        }
    }
}

fn print_created(output: OutputFormat, result: &Value) -> Result<()> {
    if matches!(output, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(result)?);
        return Ok(());
    }
    let trigger = result.get("trigger").unwrap_or(&Value::Null);
    print_detail(trigger);
    println!(
        "Inbound secret: {}",
        result.get("secret").and_then(Value::as_str).unwrap_or("-")
    );
    if let Some(secret) = result
        .get("delivery_signing_secret")
        .and_then(Value::as_str)
    {
        println!("Delivery signing secret: {secret}");
    }
    Ok(())
}

fn print_updated(output: OutputFormat, result: &Value) -> Result<()> {
    if matches!(output, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(result)?);
        return Ok(());
    }
    print_detail(result.get("trigger").unwrap_or(&Value::Null));
    if let Some(secret) = result
        .get("delivery_signing_secret")
        .and_then(Value::as_str)
    {
        println!("Delivery signing secret: {secret}");
    }
    Ok(())
}

fn print_one(output: OutputFormat, trigger: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(trigger)?),
        OutputFormat::Table => print_detail(trigger),
    }
    Ok(())
}

fn print_list(output: OutputFormat, result: &Value) -> Result<()> {
    if matches!(output, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(result)?);
        return Ok(());
    }
    let items = result
        .get("triggers")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header(["ID", "Label", "Status", "Verification", "Delivery"]);
    for trigger in items {
        table.add_row([
            crate::commands::short_id(text(trigger, "id")).to_string(),
            text(trigger, "label").to_string(),
            text(trigger, "status").to_string(),
            nested_text(trigger, "verification", "mode").to_string(),
            nested_text(trigger, "delivery", "type").to_string(),
        ]);
    }
    println!("{table}");
    Ok(())
}

fn print_detail(trigger: &Value) {
    println!("ID:           {}", text(trigger, "id"));
    println!("Label:        {}", text(trigger, "label"));
    println!("Status:       {}", text(trigger, "status"));
    println!(
        "Verification: {}",
        nested_text(trigger, "verification", "mode")
    );
    println!("Delivery:     {}", nested_text(trigger, "delivery", "type"));
    println!("Inbound URL:  {}", text(trigger, "inbound_url"));
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("-")
}

fn nested_text<'a>(value: &'a Value, parent: &str, key: &str) -> &'a str {
    value
        .get(parent)
        .and_then(|value| value.get(key))
        .and_then(Value::as_str)
        .unwrap_or("-")
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn trigger_create_parses() {
        let cli = crate::cli::Cli::parse_from([
            "nyxid",
            "trigger",
            "create",
            "--label",
            "Build complete",
            "--verification",
            "hmac",
            "--delivery",
            "agent",
            "--conversation-id",
            "conversation-id",
        ]);
        assert!(matches!(cli.command, crate::cli::Commands::Trigger { .. }));
    }

    #[test]
    fn create_body_matches_wire_contract() {
        let body = create_body(
            "Build complete",
            Some("service-id"),
            TriggerVerificationArg::Hmac,
            "X-Hub-Signature-256",
            TriggerDeliveryArg::Agent,
            None,
            Some("conversation-id"),
            None,
        )
        .expect("create body");
        assert_eq!(body["verification"]["mode"], "hmac_sha256");
        assert_eq!(body["delivery"]["type"], "agent");
        assert_eq!(body["delivery"]["conversation_id"], "conversation-id");
    }

    #[test]
    fn webhook_requires_delivery_url() {
        assert!(delivery_value(TriggerDeliveryArg::Webhook, None, None).is_err());
    }

    #[test]
    fn management_subcommands_parse() {
        for args in [
            vec!["nyxid", "trigger", "list"],
            vec!["nyxid", "trigger", "show", "trigger-id"],
            vec![
                "nyxid",
                "trigger",
                "update",
                "trigger-id",
                "--status",
                "disabled",
            ],
            vec!["nyxid", "trigger", "delete", "trigger-id"],
            vec!["nyxid", "trigger", "rotate-secret", "trigger-id"],
        ] {
            let cli = crate::cli::Cli::try_parse_from(args).expect("parse trigger subcommand");
            assert!(matches!(cli.command, crate::cli::Commands::Trigger { .. }));
        }
    }

    #[test]
    fn query_verification_and_notification_delivery_match_wire_contract() {
        let body = create_body(
            "Notification trigger",
            None,
            TriggerVerificationArg::Query,
            "X-Hub-Signature-256",
            TriggerDeliveryArg::Notification,
            None,
            None,
            Some("org-id"),
        )
        .expect("create body");
        assert_eq!(body["verification"]["mode"], "token");
        assert_eq!(body["verification"]["location"], "query");
        assert_eq!(body["delivery"]["type"], "notification");
        assert_eq!(body["target_org_id"], "org-id");
    }
}
