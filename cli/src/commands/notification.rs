use anyhow::Result;
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{NotificationCommands, OutputFormat};

pub async fn run(command: NotificationCommands) -> Result<()> {
    match command {
        NotificationCommands::Settings { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let settings: Value = api.get("/notifications/settings").await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&settings)?);
                }
                OutputFormat::Table => {
                    let push = settings["push_enabled"]
                        .as_bool()
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let telegram = settings["telegram_enabled"]
                        .as_bool()
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let telegram_connected =
                        settings["telegram_connected"].as_bool().unwrap_or(false);
                    let approval = settings["approval_required"]
                        .as_bool()
                        .map(|b| b.to_string())
                        .unwrap_or_else(|| "-".to_string());

                    eprintln!("Notification Settings");
                    eprintln!();
                    eprintln!("Push Enabled:        {push}");
                    eprintln!("Telegram Enabled:    {telegram}");
                    eprintln!("Telegram Connected:  {telegram_connected}");
                    eprintln!("Approval Required:   {approval}");
                }
            }
            Ok(())
        }

        NotificationCommands::Update {
            approval_email,
            approval_push,
            approval_telegram,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let mut body = serde_json::Map::new();

            if let Some(v) = approval_email {
                body.insert("approval_required".into(), Value::Bool(v));
            }
            if let Some(v) = approval_push {
                body.insert("push_enabled".into(), Value::Bool(v));
            }
            if let Some(v) = approval_telegram {
                body.insert("telegram_enabled".into(), Value::Bool(v));
            }

            if body.is_empty() {
                eprintln!(
                    "No updates specified. Use --approval-email, --approval-push, or --approval-telegram."
                );
                return Ok(());
            }

            let result: Value = api.put("/notifications/settings", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("Notification settings updated.");
                }
            }
            Ok(())
        }

        NotificationCommands::TelegramLink { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api
                .post("/notifications/telegram/link", &serde_json::json!({}))
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let code = result["link_code"].as_str().unwrap_or("-");
                    let bot = result["bot_username"].as_str().unwrap_or("-");
                    let instructions = result["instructions"].as_str().unwrap_or("");

                    eprintln!("Telegram Link");
                    eprintln!();
                    eprintln!("Code: {code}");
                    eprintln!("Bot:  @{bot}");
                    if !instructions.is_empty() {
                        eprintln!();
                        eprintln!("{instructions}");
                    }
                }
            }
            Ok(())
        }

        NotificationCommands::TelegramDisconnect { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.delete_empty("/notifications/telegram").await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
                ),
                OutputFormat::Table => eprintln!("Telegram disconnected."),
            }
            Ok(())
        }
    }
}
