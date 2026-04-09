use std::io::Write;

use anyhow::{Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{ChannelBotCommands, ChannelRouteCommands, OutputFormat};

pub async fn run(command: ChannelBotCommands) -> Result<()> {
    match command {
        ChannelBotCommands::Register {
            platform,
            bot_token,
            token_env,
            label,
            app_id,
            app_secret,
            app_secret_env,
            public_key,
            auth,
        } => {
            let token = resolve_secret(bot_token.as_deref(), token_env.as_deref(), "bot token")?;
            let resolved_app_secret =
                resolve_optional_secret(app_secret.as_deref(), app_secret_env.as_deref())?;

            let mut api = ApiClient::from_auth(&auth)?;

            let mut body = serde_json::json!({
                "platform": platform,
                "bot_token": token,
                "label": label,
            });

            if let Some(id) = app_id {
                body["app_id"] = Value::String(id);
            }
            if let Some(secret) = resolved_app_secret {
                body["app_secret"] = Value::String(secret);
            }
            if let Some(key) = public_key {
                body["public_key"] = Value::String(key);
            }

            let result: Value = api.post("/channel-bots", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let id = result["id"].as_str().unwrap_or("-");
                    let username = result["platform_bot_username"].as_str().unwrap_or("-");
                    let status = result["status"].as_str().unwrap_or("-");

                    eprintln!("Bot registered.");
                    eprintln!();
                    eprintln!("ID:       {id}");
                    eprintln!("Platform: {platform}");
                    eprintln!("Username: {username}");
                    eprintln!("Status:   {status}");
                }
            }
            Ok(())
        }

        ChannelBotCommands::List { auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let bots: Value = api.get("/channel-bots").await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&bots)?);
                }
                OutputFormat::Table => {
                    let items = bots
                        .get("bots")
                        .and_then(|v| v.as_array())
                        .or_else(|| bots.as_array());

                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No bots registered.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header([
                            "ID", "Platform", "Username", "Label", "Status", "Webhook",
                        ]);

                        for bot in items {
                            let id = bot["id"].as_str().unwrap_or("-");
                            let short_id = if id.len() > 8 { &id[..8] } else { id };
                            let platform = bot["platform"].as_str().unwrap_or("-");
                            let username = bot["platform_bot_username"].as_str().unwrap_or("-");
                            let label = bot["label"].as_str().unwrap_or("-");
                            let status = bot["status"].as_str().unwrap_or("-");
                            let webhook = if bot["webhook_registered"].as_bool().unwrap_or(false) {
                                "yes"
                            } else {
                                "no"
                            };
                            table.add_row([short_id, platform, username, label, status, webhook]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        ChannelBotCommands::Show { id, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let bot: Value = api.get(&format!("/channel-bots/{id}")).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&bot)?);
                }
                OutputFormat::Table => {
                    let bot_id = bot["id"].as_str().unwrap_or(&id);
                    let platform = bot["platform"].as_str().unwrap_or("-");
                    let label = bot["label"].as_str().unwrap_or("-");
                    let bot_user_id = bot["platform_bot_id"].as_str().unwrap_or("-");
                    let username = bot["platform_bot_username"].as_str().unwrap_or("-");
                    let status = bot["status"].as_str().unwrap_or("-");
                    let webhook = if bot["webhook_registered"].as_bool().unwrap_or(false) {
                        "yes"
                    } else {
                        "no"
                    };
                    let active = if bot["is_active"].as_bool().unwrap_or(false) {
                        "yes"
                    } else {
                        "no"
                    };
                    let conversations = bot["conversations_count"]
                        .as_u64()
                        .map(|c| c.to_string())
                        .unwrap_or_else(|| "-".to_string());
                    let created = bot["created_at"].as_str().unwrap_or("-");
                    let updated = bot["updated_at"].as_str().unwrap_or("-");

                    eprintln!("ID:             {bot_id}");
                    eprintln!("Platform:       {platform}");
                    eprintln!("Label:          {label}");
                    eprintln!("Bot ID:         {bot_user_id}");
                    eprintln!("Username:       {username}");
                    eprintln!("Status:         {status}");
                    eprintln!("Webhook:        {webhook}");
                    eprintln!("Active:         {active}");
                    eprintln!("Conversations:  {conversations}");
                    eprintln!("Created:        {created}");
                    eprintln!("Updated:        {updated}");
                }
            }
            Ok(())
        }

        ChannelBotCommands::Delete { id, yes, auth } => {
            if !yes {
                eprint!("Delete bot {id}? This will also remove all conversation routes. [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            let mut api = ApiClient::from_auth(&auth)?;
            api.delete_empty(&format!("/channel-bots/{id}")).await?;
            eprintln!("Bot deleted.");
            Ok(())
        }

        ChannelBotCommands::Verify { id, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let result: Value = api
                .post(
                    &format!("/channel-bots/{id}/verify"),
                    &serde_json::json!({}),
                )
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let status = result["status"].as_str().unwrap_or("-");
                    let webhook = if result["webhook_registered"].as_bool().unwrap_or(false) {
                        "yes"
                    } else {
                        "no"
                    };
                    eprintln!("Verification complete.");
                    eprintln!("Status:  {status}");
                    eprintln!("Webhook: {webhook}");
                }
            }
            Ok(())
        }

        ChannelBotCommands::Route { command } => run_route(command).await,
    }
}

async fn run_route(command: ChannelRouteCommands) -> Result<()> {
    match command {
        ChannelRouteCommands::Create {
            bot_id,
            agent_key_id,
            conversation_id,
            conversation_type,
            sender_id,
            default_agent,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;

            let mut body = serde_json::json!({
                "channel_bot_id": bot_id,
                "agent_api_key_id": agent_key_id,
            });

            if let Some(conv_id) = &conversation_id {
                body["platform_conversation_id"] = Value::String(conv_id.clone());
            }
            if let Some(conv_type) = &conversation_type {
                body["platform_conversation_type"] = Value::String(conv_type.clone());
            }
            if let Some(sid) = &sender_id {
                body["platform_sender_id"] = Value::String(sid.clone());
            }
            if default_agent {
                body["default_agent"] = Value::Bool(true);
            }

            let result: Value = api.post("/channel-conversations", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let id = result["id"].as_str().unwrap_or("-");
                    let platform = result["platform"].as_str().unwrap_or("-");
                    let conv_id = result["platform_conversation_id"].as_str().unwrap_or("*");
                    let is_default = result["default_agent"].as_bool().unwrap_or(false);

                    eprintln!("Route created.");
                    eprintln!();
                    eprintln!("ID:              {id}");
                    eprintln!("Platform:        {platform}");
                    eprintln!("Conversation:    {conv_id}");
                    eprintln!("Agent Key:       {agent_key_id}");
                    eprintln!("Default Agent:   {}", if is_default { "yes" } else { "no" });
                }
            }
            Ok(())
        }

        ChannelRouteCommands::List { bot_id, auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let path = match &bot_id {
                Some(id) => format!("/channel-conversations?bot_id={id}"),
                None => "/channel-conversations".to_string(),
            };
            let routes: Value = api.get(&path).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&routes)?);
                }
                OutputFormat::Table => {
                    let items = routes
                        .get("conversations")
                        .and_then(|v| v.as_array())
                        .or_else(|| routes.as_array());

                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No conversation routes.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header([
                            "ID",
                            "Bot ID",
                            "Platform",
                            "Conversation",
                            "Agent Key",
                            "Default",
                            "Active",
                        ]);

                        for route in items {
                            let id = route["id"].as_str().unwrap_or("-");
                            let short_id = if id.len() > 8 { &id[..8] } else { id };
                            let bot = route["channel_bot_id"].as_str().unwrap_or("-");
                            let short_bot = if bot.len() > 8 { &bot[..8] } else { bot };
                            let platform = route["platform"].as_str().unwrap_or("-");
                            let conv_id = route["platform_conversation_id"].as_str().unwrap_or("*");
                            let agent_key = route["agent_api_key_id"].as_str().unwrap_or("-");
                            let short_key = if agent_key.len() > 8 {
                                &agent_key[..8]
                            } else {
                                agent_key
                            };
                            let is_default = if route["default_agent"].as_bool().unwrap_or(false) {
                                "yes"
                            } else {
                                "no"
                            };
                            let active = if route["is_active"].as_bool().unwrap_or(false) {
                                "yes"
                            } else {
                                "no"
                            };
                            table.add_row([
                                short_id, short_bot, platform, conv_id, short_key, is_default,
                                active,
                            ]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        ChannelRouteCommands::Update {
            id,
            agent_key_id,
            default_agent,
            active,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;

            let mut body = serde_json::Map::new();

            if let Some(key_id) = agent_key_id {
                body.insert("agent_api_key_id".into(), Value::String(key_id));
            }
            if let Some(v) = default_agent {
                body.insert("default_agent".into(), Value::Bool(v));
            }
            if let Some(v) = active {
                body.insert("is_active".into(), Value::Bool(v));
            }

            if body.is_empty() {
                bail!(
                    "No update fields provided. Use --agent-key-id, --default-agent, or --active."
                );
            }

            let result: Value = api
                .put(&format!("/channel-conversations/{id}"), &body)
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("Route {id} updated.");
                }
            }
            Ok(())
        }

        ChannelRouteCommands::Delete { id, yes, auth } => {
            if !yes {
                eprint!("Delete conversation route {id}? [y/N] ");
                std::io::stderr().flush()?;
                let mut answer = String::new();
                std::io::stdin().read_line(&mut answer)?;
                if !answer.trim().eq_ignore_ascii_case("y") {
                    eprintln!("Cancelled.");
                    return Ok(());
                }
            }

            let mut api = ApiClient::from_auth(&auth)?;
            api.delete_empty(&format!("/channel-conversations/{id}"))
                .await?;
            eprintln!("Route deleted.");
            Ok(())
        }
    }
}

/// Resolve a required secret from an inline value, an environment variable, or
/// an interactive prompt.
fn resolve_secret(inline: Option<&str>, env_var: Option<&str>, label: &str) -> Result<String> {
    if let Some(val) = inline {
        return Ok(val.to_string());
    }
    if let Some(var) = env_var {
        return std::env::var(var)
            .map_err(|_| anyhow::anyhow!("Environment variable {var} is not set"));
    }
    eprint!("Enter {label}: ");
    std::io::stderr().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        bail!("{label} is required");
    }
    Ok(trimmed)
}

/// Resolve an optional secret from an inline value or an environment variable.
fn resolve_optional_secret(inline: Option<&str>, env_var: Option<&str>) -> Result<Option<String>> {
    if let Some(val) = inline {
        return Ok(Some(val.to_string()));
    }
    if let Some(var) = env_var {
        let val = std::env::var(var)
            .map_err(|_| anyhow::anyhow!("Environment variable {var} is not set"))?;
        return Ok(Some(val));
    }
    Ok(None)
}
