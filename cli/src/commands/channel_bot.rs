use std::io::Write;

use anyhow::{Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{ChannelBotCommands, ChannelRouteCommands, OutputFormat};
use crate::commands::lark_permission::print_permission_block;
use crate::org_resolver::resolve_org_id;

async fn delete_bot_result(api: &mut ApiClient, id: &str) -> Result<Value> {
    if let Some(result) = api
        .delete_optional::<Value>(&format!("/channel-bots/{id}"))
        .await?
    {
        let cleanup = result["webhook_cleanup"]
            .as_str()
            .filter(|value| matches!(*value, "removed" | "failed"))
            .ok_or_else(|| anyhow::anyhow!("Invalid Aurinko deletion cleanup response"))?;
        Ok(serde_json::json!({"ok": true, "webhook_cleanup": cleanup}))
    } else {
        Ok(serde_json::json!({"ok": true}))
    }
}

pub async fn run(command: ChannelBotCommands) -> Result<()> {
    match command {
        ChannelBotCommands::Platforms { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api.get("/channel-platforms").await?;
            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
                OutputFormat::Table => {
                    let mut table = Table::new();
                    table.load_preset(UTF8_FULL_CONDENSED);
                    table.set_header([
                        "Platform",
                        "Enabled",
                        "Ingestion",
                        "Required fields",
                        "Media in",
                        "Media out",
                        "Edit",
                    ]);
                    for p in result["platforms"].as_array().into_iter().flatten() {
                        let joined = |values: &Value| {
                            values
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .collect::<Vec<_>>()
                                .join(", ")
                        };
                        let required = p["registration"]["fields"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|f| f["required"] == true)
                            .filter_map(|f| f["name"].as_str())
                            .collect::<Vec<_>>()
                            .join(", ");
                        table.add_row([
                            p["platform"].as_str().unwrap_or("-").to_string(),
                            p["enabled"].to_string(),
                            p["ingestion"]["mode"].as_str().unwrap_or("-").to_string(),
                            required,
                            joined(&p["capabilities"]["media"]["inbound"]),
                            joined(&p["capabilities"]["media"]["outbound"]),
                            p["capabilities"]["edit"].to_string(),
                        ]);
                    }
                    println!("{table}");
                }
            }
            Ok(())
        }

        ChannelBotCommands::Send {
            conversation,
            text,
            idempotency_key,
            auth,
        } => {
            uuid::Uuid::parse_str(&conversation)
                .map_err(|_| anyhow::anyhow!("--conversation must be a UUID"))?;
            if text.trim().is_empty() {
                bail!("--text must not be empty");
            }
            if idempotency_key
                .as_ref()
                .is_some_and(|key| key.is_empty() || key.chars().count() > 128)
            {
                bail!("--idempotency-key must contain 1 to 128 characters");
            }
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result: Value = api.post("/channel-relay/send", &serde_json::json!({
                "conversation_id": conversation, "message": { "text": text }, "idempotency_key": idempotency_key,
            })).await?;
            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
                OutputFormat::Table => println!(
                    "Platform accepted message {} (receipt: {}).",
                    result["message_id"].as_str().unwrap_or("-"),
                    result["platform_message_id"]
                        .as_str()
                        .unwrap_or("unavailable")
                ),
            }
            Ok(())
        }

        ChannelBotCommands::Register {
            platform,
            managed,
            bot_token,
            token_env,
            label,
            app_id,
            app_secret,
            app_secret_env,
            verification_token,
            encrypt_key,
            public_key,
            phone_number_id,
            waba_id,
            org,
            auth,
        } => {
            if managed {
                if [
                    bot_token.as_ref(),
                    token_env.as_ref(),
                    app_id.as_ref(),
                    app_secret.as_ref(),
                    app_secret_env.as_ref(),
                    verification_token.as_ref(),
                    encrypt_key.as_ref(),
                    public_key.as_ref(),
                    phone_number_id.as_ref(),
                    waba_id.as_ref(),
                ]
                .iter()
                .any(|value| value.is_some())
                {
                    bail!("--managed cannot be combined with credential flags");
                }
                let mut api = ApiClient::from_auth_checked(&auth).await?;
                let bootstrap_path = if platform == "telegram-new" {
                    "/channel-bots/telegram-new".to_string()
                } else {
                    format!(
                        "/channel-bots/managed-onboarding/{}",
                        urlencoding::encode(&platform)
                    )
                };
                let bootstrap: Value = api.get(&bootstrap_path).await?;
                if bootstrap["available"] != true {
                    bail!(
                        "Managed onboarding is not supported or not configured for this platform"
                    );
                }
                let frontend = crate::auth::fetch_frontend_url(
                    &auth.resolved_base_url()?,
                    auth.profile.as_deref(),
                )
                .await?;
                let mut url = reqwest::Url::parse(&format!("{frontend}/channel-bots"))?;
                url.query_pairs_mut().append_pair("connect", &platform);
                if let Some(label) = label {
                    url.query_pairs_mut().append_pair("label", &label);
                }
                if let Some(org) = org {
                    let mut api = ApiClient::from_auth_checked(&auth).await?;
                    let id = resolve_org_id(&mut api, &org).await?;
                    url.query_pairs_mut().append_pair("target_org_id", &id);
                }
                match auth.output {
                    OutputFormat::Json => println!(
                        "{}",
                        serde_json::json!({ "connect_url": url.as_str(), "browser_required": true })
                    ),
                    OutputFormat::Table => {
                        println!("{url}");
                        if platform == "telegram-new" {
                            eprintln!(
                                "Open this URL, sign in to NyxID, and choose Telegram. Create and approve your bot in Telegram, then return to connect it."
                            );
                        } else {
                            eprintln!(
                                "Open this URL, sign in to NyxID, and complete the account connection in your browser."
                            );
                        }
                    }
                }
                return Ok(());
            }
            if platform == "telegram-new" {
                bail!(
                    "Telegram requires browser approval for new bot creation. Use --managed to get its creation link."
                );
            }
            let label = label.ok_or_else(|| anyhow::anyhow!("--label is required"))?;
            let token = if matches!(platform.as_str(), "lark" | "feishu") {
                resolve_optional_secret(bot_token.as_deref(), token_env.as_deref())?
            } else {
                Some(resolve_secret(
                    bot_token.as_deref(),
                    token_env.as_deref(),
                    "bot token",
                )?)
            };
            let resolved_app_secret =
                resolve_optional_secret(app_secret.as_deref(), app_secret_env.as_deref())?;
            validate_platform_fields(
                &platform,
                phone_number_id.as_deref(),
                waba_id.as_deref(),
                resolved_app_secret.as_deref(),
            )?;
            let resolved_verification_token = verification_token.or_else(|| {
                (platform != "whatsapp" && platform != "aurinko")
                    .then(|| env_secret("NYXID_LARK_VERIFICATION_TOKEN"))
                    .flatten()
            });
            let resolved_encrypt_key = encrypt_key.or_else(|| {
                (platform != "whatsapp" && platform != "aurinko")
                    .then(|| env_secret("NYXID_LARK_ENCRYPT_KEY"))
                    .flatten()
            });

            if matches!(platform.as_str(), "lark" | "feishu")
                && resolved_verification_token
                    .as_deref()
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .is_none()
            {
                bail!(
                    "--verification-token is required for Lark/Feishu (or set NYXID_LARK_VERIFICATION_TOKEN)"
                );
            }

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };

            let mut body = serde_json::json!({
                "platform": platform,
                "label": label,
            });

            if let Some(token) = token {
                body["bot_token"] = Value::String(token);
            }
            if let Some(id) = app_id {
                body["app_id"] = Value::String(id);
            }
            if let Some(secret) = resolved_app_secret {
                body["app_secret"] = Value::String(secret);
            }
            if let Some(token) = resolved_verification_token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                body["verification_token"] = Value::String(token.to_string());
            }
            if let Some(key) = resolved_encrypt_key
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                body["encrypt_key"] = Value::String(key.to_string());
            }
            if let Some(key) = public_key {
                body["public_key"] = Value::String(key);
            }
            if let Some(id) = phone_number_id {
                body["phone_number_id"] = Value::String(id);
            }
            if let Some(id) = waba_id {
                body["waba_id"] = Value::String(id);
            }
            if let Some(ref org_id) = org {
                body["target_org_id"] = Value::String(org_id.clone());
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
                    print_webhook_setup(&result);
                    print_permission_block(&result);
                }
            }
            Ok(())
        }

        ChannelBotCommands::Update {
            x_events,
            bot_token,
            token_env,
            app_secret_env,
            id,
            label,
            verification_token,
            encrypt_key,
            app_id,
            app_secret,
            auth,
        } => {
            let app_secret =
                resolve_optional_secret(app_secret.as_deref(), app_secret_env.as_deref())?;
            let bot_token = resolve_optional_secret(bot_token.as_deref(), token_env.as_deref())?;
            let explicit_verification_token = verification_token.is_some();
            let explicit_encrypt_key = encrypt_key.is_some();
            let resolved_verification_token =
                verification_token.or_else(|| env_secret("NYXID_LARK_VERIFICATION_TOKEN"));
            let resolved_encrypt_key = match encrypt_key {
                Some(value) => Some(value),
                None => env_secret("NYXID_LARK_ENCRYPT_KEY"),
            };

            if resolved_verification_token
                .as_deref()
                .is_some_and(|value| value.trim().is_empty())
            {
                bail!("--verification-token cannot be blank");
            }

            let mut body = serde_json::json!({});
            let mut changed = false;
            if !x_events.is_empty() {
                body["x_events"] = serde_json::json!(x_events);
                changed = true;
            }
            if let Some(value) = bot_token {
                if value.trim().is_empty() {
                    bail!("Bot access token cannot be blank");
                }
                body["bot_token"] = Value::String(value.trim().to_string());
                changed = true;
            }

            if let Some(value) = label
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                body["label"] = Value::String(value.to_string());
                changed = true;
            }
            if let Some(value) = resolved_verification_token
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                body["verification_token"] = Value::String(value.to_string());
                changed = true;
            }
            // encrypt_key uses three-state semantics: omitted = no change;
            // explicit empty string = clear; non-empty = set.
            // Other fields (label, verification_token, app_id, app_secret)
            // only accept non-empty values.
            if let Some(value) = resolved_encrypt_key {
                body["encrypt_key"] = Value::String(value);
                changed = true;
            }
            if let Some(value) = app_id
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                body["app_id"] = Value::String(value.to_string());
                changed = true;
            }
            if let Some(value) = app_secret
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
            {
                body["app_secret"] = Value::String(value.to_string());
                changed = true;
            }

            if !changed {
                bail!("No update fields provided");
            }

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            if (!explicit_verification_token && body.get("verification_token").is_some())
                || (!explicit_encrypt_key && body.get("encrypt_key").is_some())
            {
                let bot: Value = api.get(&format!("/channel-bots/{id}")).await?;
                if matches!(bot["platform"].as_str(), Some("whatsapp" | "aurinko")) {
                    let fields = body.as_object_mut().expect("update body is an object");
                    if !explicit_verification_token {
                        fields.remove("verification_token");
                    }
                    if !explicit_encrypt_key {
                        fields.remove("encrypt_key");
                    }
                    if fields.is_empty() {
                        bail!("No update fields provided");
                    }
                }
            }
            let result: Value = api.patch(&format!("/channel-bots/{id}"), &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    eprintln!("Bot updated.");
                    eprintln!("ID:       {}", result["id"].as_str().unwrap_or(&id));
                    eprintln!("Platform: {}", result["platform"].as_str().unwrap_or("-"));
                    eprintln!("Label:    {}", result["label"].as_str().unwrap_or("-"));
                    eprintln!("Status:   {}", result["status"].as_str().unwrap_or("-"));
                    print_permission_block(&result);
                }
            }
            Ok(())
        }

        ChannelBotCommands::List { org, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };
            let path = match org.as_deref() {
                Some(id) => format!("/channel-bots?org_id={}", urlencoding::encode(id)),
                None => "/channel-bots".to_string(),
            };
            let bots: Value = api.get(&path).await?;

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
                            let short_id = crate::commands::short_id(id);
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
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
                    eprintln!(
                        "Credentials:    {}",
                        bot["credential_source"].as_str().unwrap_or("user")
                    );
                    eprintln!("Label:          {label}");
                    eprintln!("Bot ID:         {bot_user_id}");
                    eprintln!("Username:       {username}");
                    eprintln!("Status:         {status}");
                    if let Some(events) = bot["x_events"].as_array() {
                        let names = events.iter().filter_map(Value::as_str).collect::<Vec<_>>();
                        eprintln!("Events:         {}", names.join(", "));
                    }
                    if bot["webhook_ingestion"] == false {
                        eprintln!("Ingestion:      polling");
                        for (field, label) in [
                            ("connection_id", "Connection"),
                            ("last_polled_at", "Last polled"),
                            ("last_poll_notice", "Last poll notice"),
                            ("next_poll_at", "Next poll"),
                            ("poll_cursor", "Cursor"),
                            ("poll_backoff_until", "Backoff until"),
                        ] {
                            eprintln!("{label}: {}", bot[field].as_str().unwrap_or("-"));
                        }
                        eprintln!(
                            "Poll errors: {}",
                            bot["poll_error_count"].as_u64().unwrap_or(0)
                        );
                    } else {
                        eprintln!("Webhook:        {webhook}");
                    }
                    if let Some(error) = bot["error"].as_str() {
                        eprintln!("Error: {error}");
                    }
                    eprintln!("Active:         {active}");
                    eprintln!("Conversations:  {conversations}");
                    eprintln!("Created:        {created}");
                    eprintln!("Updated:        {updated}");
                    print_permission_block(&bot);
                }
            }
            Ok(())
        }

        ChannelBotCommands::Repair { id, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let setup: Value = api
                .post(
                    &format!("/channel-bots/{id}/managed-setup/repair"),
                    &serde_json::json!({}),
                )
                .await?;
            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&setup)?),
                OutputFormat::Table => {
                    for field in ["subscription", "webhook_override", "registration"] {
                        eprintln!("{field}: {}", setup[field].as_str().unwrap_or("-"));
                    }
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

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let result = delete_bot_result(&mut api, &id).await?;
            if result["webhook_cleanup"] == "failed" {
                eprintln!(
                    "Bot deleted locally. Remove its remaining subscription in the Aurinko dashboard."
                );
            }
            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&result)?),
                OutputFormat::Table => eprintln!("Bot deleted."),
            }
            Ok(())
        }

        ChannelBotCommands::Verify { id, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
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
            allow_agent_initiated,
            org,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };

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
            if allow_agent_initiated {
                body["allow_agent_initiated"] = Value::Bool(true);
            }
            if default_agent {
                body["default_agent"] = Value::Bool(true);
            }
            if let Some(ref org_id) = org {
                body["target_org_id"] = Value::String(org_id.clone());
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

        ChannelRouteCommands::List { bot_id, org, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };
            let mut params: Vec<String> = Vec::new();
            if let Some(id) = &bot_id {
                params.push(format!("bot_id={}", urlencoding::encode(id)));
            }
            if let Some(org_id) = &org {
                params.push(format!("org_id={}", urlencoding::encode(org_id)));
            }
            let path = if params.is_empty() {
                "/channel-conversations".to_string()
            } else {
                format!("/channel-conversations?{}", params.join("&"))
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
                            let short_id = crate::commands::short_id(id);
                            let bot = route["channel_bot_id"].as_str().unwrap_or("-");
                            let short_bot = crate::commands::short_id(bot);
                            let platform = route["platform"].as_str().unwrap_or("-");
                            let conv_id = route["platform_conversation_id"].as_str().unwrap_or("*");
                            let agent_key = route["agent_api_key_id"].as_str().unwrap_or("-");
                            let short_key = crate::commands::short_id(agent_key);
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
            allow_agent_initiated,
            active,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;

            let mut body = serde_json::Map::new();

            if let Some(key_id) = agent_key_id {
                body.insert("agent_api_key_id".into(), Value::String(key_id));
            }
            if let Some(v) = default_agent {
                body.insert("default_agent".into(), Value::Bool(v));
            }
            if let Some(v) = allow_agent_initiated {
                body.insert("allow_agent_initiated".into(), Value::Bool(v));
            }
            if let Some(v) = active {
                body.insert("is_active".into(), Value::Bool(v));
            }

            if body.is_empty() {
                bail!(
                    "No update fields provided. Use --agent-key-id, --default-agent, --allow-agent-initiated, or --active."
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

            let mut api = ApiClient::from_auth_checked(&auth).await?;
            api.delete_empty(&format!("/channel-conversations/{id}"))
                .await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({ "ok": true }))?
                ),
                OutputFormat::Table => eprintln!("Route deleted."),
            }
            Ok(())
        }
    }
}

fn validate_platform_fields(
    platform: &str,
    phone_number_id: Option<&str>,
    waba_id: Option<&str>,
    app_secret: Option<&str>,
) -> Result<()> {
    if platform == "aurinko" && app_secret.is_none_or(|secret| secret.trim().is_empty()) {
        bail!(
            "--app-secret-env is required for Aurinko (application signing secret, distinct from the account access token)"
        );
    }
    if platform == "whatsapp" {
        let phone_number_id = phone_number_id.ok_or_else(|| {
            anyhow::anyhow!("--phone-number-id is required for WhatsApp Cloud API")
        })?;
        for (label, value) in [
            ("--phone-number-id", Some(phone_number_id)),
            ("--waba-id", waba_id),
        ] {
            if let Some(value) = value
                && (value.is_empty()
                    || value.len() > 32
                    || !value.bytes().all(|byte| byte.is_ascii_digit()))
            {
                bail!("{label} must be a numeric Meta identifier");
            }
        }
        if app_secret.is_none_or(|secret| secret.trim().is_empty()) {
            bail!("--app-secret-env is required for WhatsApp (Meta App Secret)");
        }
    } else if phone_number_id.is_some() || waba_id.is_some() {
        bail!("--phone-number-id and --waba-id are only supported for WhatsApp");
    }
    Ok(())
}

fn print_webhook_setup(result: &Value) {
    if let Some(url) = result["webhook_url"].as_str() {
        eprintln!("Callback URL: {url}");
    }
    // The one-time dashboard verification value is intentionally shown to the
    // operator once (same posture as API-key and client-secret creation output).
    if let Some(dashboard_value) = result["webhook_secret"].as_str() {
        let label = result["webhook_secret_label"]
            .as_str()
            .unwrap_or("Webhook secret");
        eprintln!("{label} (shown once): {dashboard_value}");
    }
    for instruction in result["setup_instructions"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
    {
        eprintln!("  {instruction}");
    }
}

#[cfg(test)]
mod whatsapp_tests {
    use super::*;
    use clap::Parser;

    #[test]
    fn channel_whatsapp_registration_requires_phone_and_secret() {
        assert!(
            validate_platform_fields("whatsapp", Some("123456"), Some("654321"), Some("secret"))
                .is_ok()
        );
        assert!(validate_platform_fields("whatsapp", None, None, Some("secret")).is_err());
        assert!(validate_platform_fields("whatsapp", Some("123456"), None, Some(" ")).is_err());
        assert!(
            validate_platform_fields("whatsapp", Some("+1 555"), None, Some("secret")).is_err()
        );
        assert!(
            validate_platform_fields("whatsapp", Some("123456"), Some("../bad"), Some("secret"))
                .is_err()
        );
        assert!(validate_platform_fields("telegram", Some("123456"), None, None).is_err());
    }

    #[test]
    fn channel_whatsapp_cli_accepts_registration_and_rotation_flags() {
        assert!(
            crate::cli::Cli::try_parse_from([
                "nyxid",
                "channel-bot",
                "register",
                "--platform",
                "whatsapp",
                "--label",
                "Support",
                "--token-env",
                "WHATSAPP_ACCESS_TOKEN",
                "--phone-number-id",
                "123456",
                "--app-secret-env",
                "META_APP_SECRET",
                "--waba-id",
                "654321",
            ])
            .is_ok()
        );
        assert!(
            crate::cli::Cli::try_parse_from([
                "nyxid",
                "channel-bot",
                "update",
                "bot-id",
                "--token-env",
                "WHATSAPP_ACCESS_TOKEN",
                "--app-secret-env",
                "META_APP_SECRET",
            ])
            .is_ok()
        );
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

fn env_secret(var: &str) -> Option<String> {
    std::env::var(var)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{env_lock, mock_auth, mock_auth_with_output};
    use wiremock::matchers::{body_partial_json, method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    const ORG_UUID: &str = "00000000-0000-0000-0000-0000000000bb";

    #[tokio::test]
    async fn platforms_fetches_authoritative_catalog_in_both_output_modes() {
        let server = MockServer::start().await;
        Mock::given(method("GET")).and(path("/api/v1/channel-platforms"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"platforms":[{
                "platform":"telegram","enabled":true,"ingestion":{"mode":"webhook"},
                "registration":{"fields":[{"name":"bot_token","required":true}]},
                "capabilities":{"edit":true,"media":{"inbound":["image","file"],"outbound":["image","file"]}}
            }]}))).expect(2).mount(&server).await;
        for output in [OutputFormat::Json, OutputFormat::Table] {
            let mut auth = mock_auth(server.uri());
            auth.output = output;
            run(ChannelBotCommands::Platforms { auth }).await.unwrap();
        }
    }

    #[tokio::test]
    async fn x_managed_arguments_bootstrap_and_show_are_generic() {
        use clap::Parser;
        crate::cli::Cli::try_parse_from([
            "nyxid",
            "channel-bot",
            "register",
            "--platform",
            "x",
            "--managed",
        ])
        .unwrap();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/channel-bots/managed-onboarding/x"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"available": true, "flow": "oauth_connection"}),
                ),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/public/config"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"frontend_url": "https://nyxid.example"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET")).and(path("/api/v1/channel-bots/x-bot"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "x-bot", "platform": "x", "webhook_ingestion": false,
                "credential_source": "connection", "connection_id": "connection", "poll_cursor": "100", "poll_error_count": 5,
                "last_polled_at": "2026-09-08T00:00:00Z", "next_poll_at": null, "status": "failed", "error": "Reconnect the channel account."})))
            .expect(1).mount(&server).await;
        let mut command = register(server.uri(), "x", None, None);
        if let ChannelBotCommands::Register { managed, .. } = &mut command {
            *managed = true;
        }
        run(command).await.unwrap();
        run(ChannelBotCommands::Show {
            id: "x-bot".into(),
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn managed_signup_discovers_browser_url_without_posting_credentials() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/channel-bots/managed-onboarding/whatsapp"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "available": true })),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/channel-bots/managed-onboarding/telegram"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "available": false })),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/public/config"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "frontend_url": "https://nyxid.example" })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut command = register(server.uri(), "whatsapp", None, None);
        if let ChannelBotCommands::Register { managed, label, .. } = &mut command {
            *managed = true;
            *label = None;
        }
        run(command).await.unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        let mut command = register(server.uri(), "telegram", None, None);
        if let ChannelBotCommands::Register { managed, .. } = &mut command {
            *managed = true;
        }
        assert!(
            run(command)
                .await
                .unwrap_err()
                .to_string()
                .contains("not supported")
        );
    }

    #[tokio::test]
    async fn telegram_new_signup_discovers_browser_url_without_requesting_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/channel-bots/telegram-new"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"available": true})),
            )
            .expect(1)
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/api/v1/public/config"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "frontend_url": "https://nyxid.example" })),
            )
            .expect(1)
            .mount(&server)
            .await;
        let mut command = register(server.uri(), "telegram-new", None, None);
        if let ChannelBotCommands::Register { managed, label, .. } = &mut command {
            *managed = true;
            *label = None;
        }
        run(command).await.unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        let command = register(server.uri(), "telegram-new", None, None);
        assert!(
            run(command)
                .await
                .unwrap_err()
                .to_string()
                .contains("--managed")
        );
    }

    fn register(
        uri: String,
        platform: &str,
        bot_token: Option<&str>,
        verification_token: Option<&str>,
    ) -> ChannelBotCommands {
        ChannelBotCommands::Register {
            phone_number_id: None,
            managed: false,
            waba_id: None,
            platform: platform.to_string(),
            bot_token: bot_token.map(str::to_string),
            token_env: None,
            label: Some("support".to_string()),
            app_id: None,
            app_secret: None,
            app_secret_env: None,
            verification_token: verification_token.map(str::to_string),
            encrypt_key: None,
            public_key: None,
            org: None,
            auth: mock_auth(uri),
        }
    }

    #[tokio::test]
    async fn register_telegram_posts_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-bots"))
            .and(body_partial_json(serde_json::json!({
                "platform": "telegram", "bot_token": "tok-123", "label": "support"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "bot-1", "status": "active"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(register(server.uri(), "telegram", Some("tok-123"), None))
            .await
            .expect("telegram register should succeed");
    }

    #[tokio::test]
    async fn channel_whatsapp_registration_and_rotation_use_separate_credentials() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-bots"))
            .and(wiremock::matchers::body_json(serde_json::json!({
                "platform": "whatsapp", "bot_token": "meta-token", "label": "support",
                "phone_number_id": "123456", "waba_id": "654321", "app_secret": "meta-secret"
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "bot-1", "status": "pending_webhook", "webhook_secret": "verify-token",
                "webhook_secret_label": "Verify Token"
            })))
            .expect(1)
            .mount(&server)
            .await;
        let mut command = register(server.uri(), "whatsapp", Some("meta-token"), None);
        if let ChannelBotCommands::Register {
            phone_number_id,
            waba_id,
            app_secret,
            ..
        } = &mut command
        {
            *phone_number_id = Some("123456".to_string());
            *waba_id = Some("654321".to_string());
            *app_secret = Some("meta-secret".to_string());
        }
        run(command).await.unwrap();
        Mock::given(method("PATCH"))
            .and(path("/api/v1/channel-bots/bot-1"))
            .and(wiremock::matchers::body_json(
                serde_json::json!({"bot_token": "new-token", "app_secret": "new-secret"}),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"id": "bot-1", "platform": "whatsapp"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        run(ChannelBotCommands::Update {
            x_events: vec![],
            bot_token: Some("new-token".to_string()),
            token_env: None,
            app_secret_env: None,
            id: "bot-1".to_string(),
            label: None,
            verification_token: None,
            encrypt_key: None,
            app_id: None,
            app_secret: Some("new-secret".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn register_lark_and_feishu_without_bot_token() {
        for platform in ["lark", "feishu"] {
            let server = MockServer::start().await;
            Mock::given(method("POST"))
                .and(path("/api/v1/channel-bots"))
                .and(wiremock::matchers::body_json(serde_json::json!({
                    "platform": platform,
                    "label": "support",
                    "app_id": "cli_app",
                    "app_secret": "app-secret",
                    "verification_token": "vtok"
                })))
                .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                    "id": "bot-2", "status": "active"
                })))
                .expect(1)
                .mount(&server)
                .await;
            let mut command = register(server.uri(), platform, None, Some("vtok"));
            if let ChannelBotCommands::Register {
                app_id, app_secret, ..
            } = &mut command
            {
                *app_id = Some("cli_app".to_string());
                *app_secret = Some("app-secret".to_string());
            }
            run(command).await.unwrap();
        }
    }

    #[tokio::test]
    async fn register_lark_includes_verification_token() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-bots"))
            .and(body_partial_json(serde_json::json!({
                "platform": "lark", "verification_token": "vtok"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "bot-2", "status": "pending_webhook"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(register(server.uri(), "lark", Some("tok"), Some("vtok")))
            .await
            .expect("lark register should succeed");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn register_lark_without_verification_token_fails() {
        let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
        let server = MockServer::start().await;
        // SAFETY: env mutation serialized by env_lock.
        unsafe {
            std::env::remove_var("NYXID_LARK_VERIFICATION_TOKEN");
        }
        let result = run(register(server.uri(), "lark", Some("tok"), None)).await;
        assert!(
            result.is_err(),
            "lark without a verification token must be rejected"
        );
    }

    #[tokio::test]
    async fn update_sends_changed_label() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/v1/channel-bots/bot-1"))
            .and(body_partial_json(serde_json::json!({ "label": "renamed" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "bot-1", "label": "renamed"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Update {
            x_events: vec![],
            bot_token: None,
            token_env: None,
            app_secret_env: None,
            id: "bot-1".to_string(),
            label: Some("renamed".to_string()),
            verification_token: None,
            encrypt_key: None,
            app_id: None,
            app_secret: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("update should succeed");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn update_with_no_fields_fails() {
        let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
        let server = MockServer::start().await;
        // SAFETY: env mutation serialized by env_lock.
        unsafe {
            std::env::remove_var("NYXID_LARK_VERIFICATION_TOKEN");
            std::env::remove_var("NYXID_LARK_ENCRYPT_KEY");
        }
        let result = run(ChannelBotCommands::Update {
            x_events: vec![],
            bot_token: None,
            token_env: None,
            app_secret_env: None,
            id: "bot-1".to_string(),
            label: None,
            verification_token: None,
            encrypt_key: None,
            app_id: None,
            app_secret: None,
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "empty update must be rejected");
    }

    #[tokio::test]
    async fn list_fetches_bots() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/channel-bots"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "bots": [{"id": "bot-1", "platform": "telegram", "status": "active"}]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::List {
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("list should succeed");
    }

    #[tokio::test]
    async fn verify_posts_to_verify_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-bots/bot-1/verify"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "active", "webhook_registered": true
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Verify {
            id: "bot-1".to_string(),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("verify should succeed");
    }

    #[tokio::test]
    async fn delete_with_yes_deletes() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/channel-bots/bot-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Delete {
            id: "bot-1".to_string(),
            yes: true,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("delete should succeed");
    }

    #[tokio::test]
    async fn aurinko_delete_json_preserves_cleanup_outcome_and_legacy_204() {
        for outcome in [Some("removed"), Some("failed"), None] {
            let server = MockServer::start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(403))
                .expect(0)
                .mount(&server)
                .await;
            let response = match outcome {
                Some(value) => ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"webhook_cleanup":value})),
                None => ResponseTemplate::new(204),
            };
            Mock::given(method("DELETE"))
                .and(path("/api/v1/channel-bots/bot-1"))
                .respond_with(response)
                .expect(1)
                .mount(&server)
                .await;
            let mut api = ApiClient::from_auth_checked(&mock_auth(server.uri()))
                .await
                .unwrap();
            let json_output = delete_bot_result(&mut api, "bot-1").await.unwrap();
            let expected = match outcome {
                Some(value) => serde_json::json!({"ok":true, "webhook_cleanup":value}),
                None => serde_json::json!({"ok":true}),
            };
            assert_eq!(json_output, expected);
        }
    }

    #[tokio::test]
    async fn repair_posts_to_managed_setup_and_reports_errors() {
        for status in [200, 403, 429] {
            let server = MockServer::start().await;
            Mock::given(method("POST")).and(path("/api/v1/channel-bots/bot-1/managed-setup/repair"))
                .respond_with(ResponseTemplate::new(status).set_body_json(if status == 200 {
                    serde_json::json!({ "subscription": "subscribed", "webhook_override": "configured", "registration": "registered" })
                } else { serde_json::json!({ "error": "forbidden", "error_code": 1003, "message": "Request denied" }) }))
                .expect(1).mount(&server).await;
            let result = run(ChannelBotCommands::Repair {
                id: "bot-1".into(),
                auth: mock_auth(server.uri()),
            })
            .await;
            assert_eq!(result.is_ok(), status == 200);
        }
    }

    #[test]
    fn repair_cli_requires_bot_id() {
        use clap::Parser;
        assert!(crate::cli::Cli::try_parse_from(["nyxid", "channel-bot", "repair"]).is_err());
        assert!(
            matches!(crate::cli::Cli::try_parse_from(["nyxid", "channel-bot", "repair", "bot-1"]).unwrap().command,
            crate::cli::Commands::ChannelBot { command: ChannelBotCommands::Repair { id, .. } } if id == "bot-1")
        );
    }

    // --- Route subcommands ---

    #[tokio::test]
    async fn route_create_posts_conversation() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-conversations"))
            .and(body_partial_json(serde_json::json!({
                "channel_bot_id": "bot-1", "agent_api_key_id": "key-1"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "route-1", "platform": "telegram"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Route {
            command: ChannelRouteCommands::Create {
                bot_id: "bot-1".to_string(),
                agent_key_id: "key-1".to_string(),
                conversation_id: None,
                conversation_type: None,
                sender_id: None,
                default_agent: false,
                allow_agent_initiated: false,
                org: None,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("route create should succeed");
        let requests = server.received_requests().await.unwrap();
        let body: Value = requests[0].body_json().unwrap();
        assert!(body.get("allow_agent_initiated").is_none());
    }

    #[tokio::test]
    async fn route_update_with_no_fields_fails() {
        let server = MockServer::start().await;
        let result = run(ChannelBotCommands::Route {
            command: ChannelRouteCommands::Update {
                id: "route-1".to_string(),
                agent_key_id: None,
                default_agent: None,
                allow_agent_initiated: None,
                active: None,
                auth: mock_auth(server.uri()),
            },
        })
        .await;
        assert!(result.is_err(), "empty route update must be rejected");
    }

    #[tokio::test]
    async fn route_delete_with_yes_deletes() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/channel-conversations/route-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Route {
            command: ChannelRouteCommands::Delete {
                id: "route-1".to_string(),
                yes: true,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("route delete should succeed");
    }

    // --- Pure secret resolvers ---

    #[test]
    fn resolve_secret_returns_inline_value() {
        assert_eq!(
            resolve_secret(Some("tok"), None, "bot token").expect("ok"),
            "tok"
        );
    }

    #[test]
    fn resolve_optional_secret_none_when_absent() {
        assert!(resolve_optional_secret(None, None).expect("ok").is_none());
    }

    #[test]
    fn resolve_secret_reads_env_var() {
        let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
        // SAFETY: env mutation serialized by env_lock.
        unsafe {
            std::env::set_var("NYXID_TEST_BOT_TOKEN", "from-env");
        }
        let got = resolve_secret(None, Some("NYXID_TEST_BOT_TOKEN"), "bot token").expect("ok");
        unsafe {
            std::env::remove_var("NYXID_TEST_BOT_TOKEN");
        }
        assert_eq!(got, "from-env");
    }

    // --- Register: optional fields + table output ---

    #[tokio::test]
    async fn register_lark_includes_all_optional_fields() {
        let server = MockServer::start().await;
        // app_id / app_secret / encrypt_key / public_key are each added to
        // the body only when present — assert they all round-trip.
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-bots"))
            .and(body_partial_json(serde_json::json!({
                "platform": "lark",
                "app_id": "cli_app",
                "app_secret": "secret-x",
                "verification_token": "vtok",
                "encrypt_key": "ekey",
                "public_key": "pkey"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "bot-3", "status": "pending_webhook"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Register {
            phone_number_id: None,
            managed: false,
            waba_id: None,
            platform: "lark".to_string(),
            bot_token: Some("tok".to_string()),
            token_env: None,
            label: Some("support".to_string()),
            app_id: Some("cli_app".to_string()),
            app_secret: Some("secret-x".to_string()),
            app_secret_env: None,
            verification_token: Some("vtok".to_string()),
            encrypt_key: Some("ekey".to_string()),
            public_key: Some("pkey".to_string()),
            org: None,
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("lark full register should succeed");
    }

    #[tokio::test]
    async fn register_with_org_sets_target_org_id() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-bots"))
            .and(body_partial_json(
                serde_json::json!({ "target_org_id": ORG_UUID }),
            ))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "id": "bot-4", "status": "active" })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Register {
            phone_number_id: None,
            managed: false,
            waba_id: None,
            platform: "telegram".to_string(),
            bot_token: Some("tok".to_string()),
            token_env: None,
            label: Some("support".to_string()),
            app_id: None,
            app_secret: None,
            app_secret_env: None,
            verification_token: None,
            encrypt_key: None,
            public_key: None,
            org: Some(ORG_UUID.to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("org-scoped register should succeed");
    }

    // --- Update: per-field branches + blank guard + table output ---

    #[tokio::test]
    async fn update_sends_all_changed_fields() {
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/v1/channel-bots/bot-1"))
            .and(body_partial_json(serde_json::json!({
                "verification_token": "vtok2",
                "encrypt_key": "ekey2",
                "app_id": "cli_app2",
                "app_secret": "secret2"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "bot-1", "label": "support"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Update {
            x_events: vec![],
            bot_token: None,
            token_env: None,
            app_secret_env: None,
            id: "bot-1".to_string(),
            label: None,
            verification_token: Some("vtok2".to_string()),
            encrypt_key: Some("ekey2".to_string()),
            app_id: Some("cli_app2".to_string()),
            app_secret: Some("secret2".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("update with all fields should succeed");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn update_blank_verification_token_is_rejected() {
        let _guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
        let server = MockServer::start().await;
        // SAFETY: env mutation serialized by env_lock.
        unsafe {
            std::env::remove_var("NYXID_LARK_VERIFICATION_TOKEN");
        }
        let result = run(ChannelBotCommands::Update {
            x_events: vec![],
            bot_token: None,
            token_env: None,
            app_secret_env: None,
            id: "bot-1".to_string(),
            label: None,
            verification_token: Some("   ".to_string()),
            encrypt_key: None,
            app_id: None,
            app_secret: None,
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(result.is_err(), "blank verification token must be rejected");
    }

    // --- List: table + empty + org-scoped ---

    #[tokio::test]
    async fn list_with_org_scopes_path() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/channel-bots"))
            .and(query_param("org_id", ORG_UUID))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({ "bots": [] })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::List {
            org: Some(ORG_UUID.to_string()),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("org-scoped list should succeed");
    }

    // --- Show (was entirely untested) ---

    #[tokio::test]
    async fn show_fetches_bot() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/channel-bots/bot-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "bot-1", "platform": "telegram", "status": "active"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Show {
            id: "bot-1".to_string(),
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("show should succeed");
    }

    #[tokio::test]
    async fn show_table_renders_detail() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/channel-bots/bot-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "bot-1", "platform": "telegram", "label": "support",
                "platform_bot_username": "supportbot", "status": "active",
                "webhook_registered": true, "is_active": true, "conversations_count": 3,
                "created_at": "2026-05-20T00:00:00Z", "updated_at": "2026-05-20T01:00:00Z"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Show {
            id: "bot-1".to_string(),
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("show table should succeed");
    }

    #[tokio::test]
    async fn show_accepts_telegram_manager_credentials_and_readiness_in_both_formats() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/channel-bots/manager-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "manager-1", "platform": "telegram", "label": "Public manager",
                "credential_source": "telegram_manager", "status": "failed",
                "platform_bot_username": "ManagerBot", "webhook_registered": false,
                "is_active": true, "conversations_count": 1,
                "error": "Telegram manager is not ready. Check Admin Platform Credentials."
            })))
            .expect(2)
            .mount(&server)
            .await;

        for output in [OutputFormat::Json, OutputFormat::Table] {
            run(ChannelBotCommands::Show {
                id: "manager-1".to_string(),
                auth: mock_auth_with_output(server.uri(), output),
            })
            .await
            .expect("show should accept manager credentials and readiness without secrets");
        }
    }

    // --- Verify table output ---

    #[tokio::test]
    async fn verify_table_output() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-bots/bot-1/verify"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "active", "webhook_registered": true
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Verify {
            id: "bot-1".to_string(),
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("verify table should succeed");
    }

    #[tokio::test]
    async fn delete_table_output() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/channel-bots/bot-1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Delete {
            id: "bot-1".to_string(),
            yes: true,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("delete table should succeed");
    }

    // --- Route: create options + table, list, update ---

    #[tokio::test]
    async fn route_create_with_all_options() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-conversations"))
            .and(body_partial_json(serde_json::json!({
                "channel_bot_id": "bot-1",
                "agent_api_key_id": "key-1",
                "platform_conversation_id": "conv-9",
                "platform_conversation_type": "group",
                "platform_sender_id": "user-9",
                "default_agent": true,
                "allow_agent_initiated": true,
                "target_org_id": ORG_UUID
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "route-1", "platform": "telegram", "default_agent": true
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Route {
            command: ChannelRouteCommands::Create {
                bot_id: "bot-1".to_string(),
                agent_key_id: "key-1".to_string(),
                conversation_id: Some("conv-9".to_string()),
                conversation_type: Some("group".to_string()),
                sender_id: Some("user-9".to_string()),
                default_agent: true,
                allow_agent_initiated: true,
                org: Some(ORG_UUID.to_string()),
                auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
            },
        })
        .await
        .expect("route create with options should succeed");
    }

    #[tokio::test]
    async fn route_list_renders_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/channel-conversations"))
            .and(query_param("bot_id", "bot-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "conversations": [{
                    "id": "route-abcdef12", "channel_bot_id": "bot-abcdef12",
                    "platform": "telegram", "platform_conversation_id": "conv-9",
                    "agent_api_key_id": "key-abcdef12", "default_agent": true, "is_active": true
                }]
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Route {
            command: ChannelRouteCommands::List {
                bot_id: Some("bot-1".to_string()),
                org: None,
                auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
            },
        })
        .await
        .expect("route list should succeed");
    }

    #[tokio::test]
    async fn route_list_handles_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/channel-conversations"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "conversations": [] })),
            )
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Route {
            command: ChannelRouteCommands::List {
                bot_id: None,
                org: None,
                auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
            },
        })
        .await
        .expect("empty route list should succeed");
    }

    #[tokio::test]
    async fn route_update_sets_fields() {
        let server = MockServer::start().await;
        Mock::given(method("PUT"))
            .and(path("/api/v1/channel-conversations/route-1"))
            .and(body_partial_json(serde_json::json!({
                "agent_api_key_id": "key-2", "default_agent": true, "is_active": false
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelBotCommands::Route {
            command: ChannelRouteCommands::Update {
                id: "route-1".to_string(),
                agent_key_id: Some("key-2".to_string()),
                default_agent: Some(true),
                allow_agent_initiated: None,
                active: Some(false),
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("route update should succeed");
    }
    #[tokio::test]
    async fn send_posts_target_text_and_idempotency_key() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/api/v1/channel-relay/send"))
            .and(body_partial_json(serde_json::json!({ "conversation_id": ORG_UUID, "message": { "text": "hello" }, "idempotency_key": "job-done" })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "message_id": "message", "platform_message_id": "receipt" })))
            .expect(1).mount(&server).await;
        run(ChannelBotCommands::Send {
            conversation: ORG_UUID.into(),
            text: "hello".into(),
            idempotency_key: Some("job-done".into()),
            auth: mock_auth(server.uri()),
        })
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn route_update_can_enable_and_disable_initiated_messages_alone() {
        for enabled in [true, false] {
            let server = MockServer::start().await;
            Mock::given(method("PUT"))
                .and(path("/api/v1/channel-conversations/route"))
                .and(body_partial_json(
                    serde_json::json!({ "allow_agent_initiated": enabled }),
                ))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({ "id": "route" })),
                )
                .expect(1)
                .mount(&server)
                .await;
            run(ChannelBotCommands::Route {
                command: ChannelRouteCommands::Update {
                    id: "route".into(),
                    agent_key_id: None,
                    default_agent: None,
                    allow_agent_initiated: Some(enabled),
                    active: None,
                    auth: mock_auth(server.uri()),
                },
            })
            .await
            .unwrap();
        }
    }

    #[test]
    fn initiated_flags_parse_for_create_update_and_send() {
        use clap::Parser;
        for args in [
            vec![
                "nyxid",
                "channel-bot",
                "route",
                "create",
                "--bot-id",
                "bot",
                "--agent-key-id",
                "key",
                "--allow-agent-initiated",
            ],
            vec![
                "nyxid",
                "channel-bot",
                "route",
                "update",
                "route",
                "--allow-agent-initiated",
            ],
            vec![
                "nyxid",
                "channel-bot",
                "route",
                "update",
                "route",
                "--allow-agent-initiated",
                "false",
            ],
            vec![
                "nyxid",
                "channel-bot",
                "send",
                "--conversation",
                ORG_UUID,
                "--text",
                "hello",
            ],
        ] {
            assert!(crate::cli::Cli::try_parse_from(args).is_ok());
        }
    }
}

#[cfg(test)]
mod aurinko_tests {
    use super::*;
    use clap::Parser;
    #[test]
    fn aurinko_registration_and_rotation_accept_account_token_and_signing_secret() {
        assert!(validate_platform_fields("aurinko", None, None, Some("signing-secret")).is_ok());
        assert!(validate_platform_fields("aurinko", None, None, None).is_err());
        assert!(
            crate::cli::Cli::try_parse_from([
                "nyxid",
                "channel-bot",
                "register",
                "--platform",
                "aurinko",
                "--label",
                "Mailbox",
                "--token-env",
                "AURINKO_ACCOUNT_TOKEN",
                "--app-secret-env",
                "AURINKO_SIGNING_SECRET"
            ])
            .is_ok()
        );
        assert!(
            crate::cli::Cli::try_parse_from([
                "nyxid",
                "channel-bot",
                "update",
                "bot-id",
                "--token-env",
                "AURINKO_ACCOUNT_TOKEN",
                "--app-secret-env",
                "AURINKO_SIGNING_SECRET"
            ])
            .is_ok()
        );
    }
}

#[cfg(test)]
mod x_events_tests {
    use super::*;
    use clap::Parser;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{body_json, method, path},
    };

    #[tokio::test]
    async fn x_event_flags_validate_and_send_the_selected_subscription_set() {
        for value in ["likes", "dm,unknown", ""] {
            assert!(
                crate::cli::Cli::try_parse_from([
                    "nyxid",
                    "channel-bot",
                    "update",
                    "bot",
                    "--x-events",
                    value
                ])
                .is_err()
            );
        }
        let server = MockServer::start().await;
        Mock::given(method("PATCH"))
            .and(path("/api/v1/channel-bots/bot"))
            .and(body_json(
                serde_json::json!({"x_events": ["dm", "chat", "mentions", "replies", "posts"]}),
            ))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "bot"})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let parsed = crate::cli::Cli::try_parse_from([
            "nyxid",
            "channel-bot",
            "update",
            "bot",
            "--x-events",
            "dm,chat,mentions,replies,posts",
        ])
        .unwrap();
        if let crate::cli::Commands::ChannelBot {
            command: ChannelBotCommands::Update { x_events, .. },
        } = parsed.command
        {
            run(ChannelBotCommands::Update {
                x_events,
                bot_token: None,
                token_env: None,
                app_secret_env: None,
                id: "bot".into(),
                label: None,
                verification_token: None,
                encrypt_key: None,
                app_id: None,
                app_secret: None,
                auth: crate::cli::AuthArgs {
                    base_url: Some(server.uri()),
                    access_token: Some("test-key".into()),
                    access_token_env: "NYXID_X_EVENTS_TEST_TOKEN".into(),
                    profile: None,
                    output: OutputFormat::Json,
                },
            })
            .await
            .unwrap();
        } else {
            panic!("expected X event update");
        }
    }
}
