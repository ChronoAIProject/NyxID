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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_auth;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn settings_fetches_notification_settings() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/notifications/settings"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "push_enabled": true, "telegram_enabled": false,
                "telegram_connected": false, "approval_required": true
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(NotificationCommands::Settings {
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("settings should succeed");
    }

    #[tokio::test]
    async fn update_sends_only_changed_flags() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/notifications/settings"))
            .and(body_json(serde_json::json!({
                "approval_required": true, "push_enabled": false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(NotificationCommands::Update {
            approval_email: Some(true),
            approval_push: Some(false),
            approval_telegram: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("update should succeed");
    }

    #[tokio::test]
    async fn update_with_no_flags_makes_no_request() {
        // No mock mounted: an empty update must short-circuit before any HTTP.
        let server = MockServer::start().await;
        run(NotificationCommands::Update {
            approval_email: None,
            approval_push: None,
            approval_telegram: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("no-op update should succeed without a request");
    }

    #[tokio::test]
    async fn telegram_link_posts() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/notifications/telegram/link"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "link_code": "ABC123", "bot_username": "nyxid_bot"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(NotificationCommands::TelegramLink {
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("telegram link should succeed");
    }

    #[tokio::test]
    async fn telegram_disconnect_deletes() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/notifications/telegram"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(NotificationCommands::TelegramDisconnect {
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("telegram disconnect should succeed");
    }
}
