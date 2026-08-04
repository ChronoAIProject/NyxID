//! `nyxid channel-event` — HTTP Event Gateway CLI.
//!
//! Pushes device/analyzer events to
//! `POST /api/v1/channel-events/{conversation_id}`. The endpoint requires a
//! per-agent API key (`nyxid_ag_...`) that is bound to the target
//! conversation — session tokens are rejected. See the HTTP Event Gateway
//! section of the NyxID skill for background.

use std::io::{Read, Write};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;
use uuid::Uuid;

use crate::api::ApiClient;
use crate::cli::{ChannelEventChannelCommands, ChannelEventCommands, OutputFormat};
use crate::org_resolver::resolve_org_id;

pub async fn run(command: ChannelEventCommands) -> Result<()> {
    match command {
        ChannelEventCommands::Push {
            conversation_id,
            source,
            event_type,
            event_id,
            timestamp,
            payload_json,
            payload_file,
            metadata_json,
            api_key,
            api_key_env,
            base,
            output,
        } => {
            // Stdin can only be one thing at a time. If the payload is going
            // to come from stdin, the API key must be provided non-interactively
            // (--api-key or --api-key-env) — otherwise the prompt would
            // consume the first line of the piped JSON before the payload
            // reader sees it.
            let payload_from_stdin = payload_file.as_deref() == Some("-");
            let has_explicit_key = api_key.is_some() || api_key_env.is_some();
            if payload_from_stdin && !has_explicit_key {
                bail!(
                    "--payload-file - reads the payload from stdin, so the API key cannot be \
                     prompted interactively. Pass --api-key or --api-key-env."
                );
            }

            let api_key_value = resolve_api_key(api_key.as_deref(), api_key_env.as_deref())?;
            let payload = resolve_payload(payload_json.as_deref(), payload_file.as_deref())?;
            let metadata = parse_optional_json(metadata_json.as_deref(), "metadata")?;
            let resolved_event_id = event_id.unwrap_or_else(|| Uuid::new_v4().to_string());
            let resolved_timestamp = timestamp.unwrap_or_else(|| Utc::now().to_rfc3339());

            let mut envelope = serde_json::json!({
                "event_id": resolved_event_id,
                "source": source,
                "type": event_type,
                "timestamp": resolved_timestamp,
            });
            if let Some(p) = payload {
                envelope["payload"] = p;
            }
            if let Some(m) = metadata {
                envelope["metadata"] = m;
            }

            let base_url = base.resolved_base_url()?;
            // Disable the 401 refresh path: this endpoint is API-key only,
            // and a bad/revoked agent key must not silently fall back to a
            // saved session access token on the profile. Without this, a
            // 401 from the agent key would trigger ApiClient's refresh
            // retry and the request would go out with the session token,
            // returning a misleading "API key required" error.
            let mut api = ApiClient::new_with_profile(&base_url, api_key_value, base.profile)?
                .without_token_refresh();
            let path = format!("/channel-events/{conversation_id}");
            let response: Value = api.post(&path, &envelope).await?;

            match output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&response)?);
                }
                OutputFormat::Table => {
                    let status = response["status"].as_str().unwrap_or("-");
                    let returned_id = response["event_id"].as_str().unwrap_or("-");
                    eprintln!("Event forwarded.");
                    eprintln!();
                    eprintln!("Conversation: {conversation_id}");
                    eprintln!("Event ID:     {returned_id}");
                    eprintln!("Status:       {status}");
                }
            }
            Ok(())
        }
        ChannelEventCommands::Channel { command } => run_channel(command).await,
    }
}

async fn run_channel(command: ChannelEventChannelCommands) -> Result<()> {
    match command {
        ChannelEventChannelCommands::Create {
            conversation_id,
            agent_key_id,
            conversation_type,
            org,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };

            let mut body = serde_json::json!({
                "platform": "device",
                "platform_conversation_id": conversation_id,
                "agent_api_key_id": agent_key_id,
            });
            if let Some(ct) = &conversation_type {
                body["platform_conversation_type"] = Value::String(ct.clone());
            }
            if let Some(org_id) = &org {
                body["target_org_id"] = Value::String(org_id.clone());
            }

            let result: Value = api.post("/channel-conversations", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let id = result["id"].as_str().unwrap_or("-");
                    let conv_id = result["platform_conversation_id"].as_str().unwrap_or("-");
                    let conv_type = result["platform_conversation_type"].as_str().unwrap_or("-");
                    eprintln!("Device channel created.");
                    eprintln!();
                    eprintln!("ID:              {id}");
                    eprintln!("Channel ID:      {conv_id}");
                    eprintln!("Type:            {conv_type}");
                    eprintln!("Agent Key:       {agent_key_id}");
                    eprintln!();
                    eprintln!(
                        "Push events with: nyxid channel-event push --conversation-id {id} ..."
                    );
                }
            }
            Ok(())
        }

        ChannelEventChannelCommands::List { org, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let org = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };
            let path = match &org {
                Some(o) => format!("/channel-conversations?org_id={}", urlencoding::encode(o)),
                None => "/channel-conversations".to_string(),
            };
            let response: Value = api.get(&path).await?;

            // The list endpoint returns bot routes AND device channels; filter
            // client-side so `nyxid channel-event channel list` only shows
            // the device ones.
            let all = response
                .get("conversations")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let devices: Vec<&Value> = all
                .iter()
                .filter(|c| c.get("platform").and_then(|v| v.as_str()) == Some("device"))
                .collect();

            match auth.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "conversations": devices,
                            "total": devices.len(),
                        }))?
                    );
                }
                OutputFormat::Table => {
                    if devices.is_empty() {
                        eprintln!("No device channels.");
                        return Ok(());
                    }
                    let mut table = Table::new();
                    table.load_preset(UTF8_FULL_CONDENSED);
                    table.set_header(["ID", "Channel ID", "Type", "Agent Key", "Active"]);
                    for conv in devices {
                        let id = conv["id"].as_str().unwrap_or("-");
                        let short_id = crate::commands::short_id(id);
                        let chan = conv["platform_conversation_id"].as_str().unwrap_or("-");
                        let ctype = conv["platform_conversation_type"].as_str().unwrap_or("-");
                        let agent = conv["agent_api_key_id"].as_str().unwrap_or("-");
                        let short_agent = crate::commands::short_id(agent);
                        let active = if conv["is_active"].as_bool().unwrap_or(false) {
                            "yes"
                        } else {
                            "no"
                        };
                        table.add_row([short_id, chan, ctype, short_agent, active]);
                    }
                    eprintln!("{table}");
                }
            }
            Ok(())
        }

        ChannelEventChannelCommands::Delete { id, yes, auth } => {
            if !yes {
                eprint!("Delete device channel {id}? [y/N] ");
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
                OutputFormat::Table => eprintln!("Device channel deleted."),
            }
            Ok(())
        }
    }
}

fn resolve_api_key(inline: Option<&str>, env_var: Option<&str>) -> Result<String> {
    if let Some(v) = inline {
        return Ok(v.to_string());
    }
    if let Some(var) = env_var {
        return std::env::var(var)
            .map_err(|_| anyhow::anyhow!("Environment variable {var} is not set"));
    }
    eprint!("Enter NyxID API key (nyxid_ag_...): ");
    std::io::stderr().flush()?;
    let mut input = String::new();
    std::io::stdin().read_line(&mut input)?;
    let trimmed = input.trim().to_string();
    if trimmed.is_empty() {
        bail!("API key is required");
    }
    Ok(trimmed)
}

fn resolve_payload(
    payload_json: Option<&str>,
    payload_file: Option<&str>,
) -> Result<Option<Value>> {
    match (payload_json, payload_file) {
        (Some(_), Some(_)) => {
            bail!("Pass either --payload-json or --payload-file, not both")
        }
        (Some(json), None) => {
            Ok(Some(serde_json::from_str(json).with_context(|| {
                "--payload-json is not valid JSON".to_string()
            })?))
        }
        (None, Some(path)) => {
            let mut contents = String::new();
            if path == "-" {
                std::io::stdin().read_to_string(&mut contents)?;
            } else {
                contents = std::fs::read_to_string(path)
                    .with_context(|| format!("failed to read payload file {path}"))?;
            }
            Ok(Some(serde_json::from_str(&contents).with_context(
                || format!("payload file {path} is not valid JSON"),
            )?))
        }
        (None, None) => Ok(None),
    }
}

fn parse_optional_json(raw: Option<&str>, label: &str) -> Result<Option<Value>> {
    match raw {
        None => Ok(None),
        Some(s) => {
            Ok(Some(serde_json::from_str(s).with_context(|| {
                format!("--{label}-json is not valid JSON")
            })?))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::BaseUrlArgs;
    use crate::test_support::mock_auth;
    use wiremock::matchers::{body_partial_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // --- Push (command-level) ---

    #[tokio::test]
    async fn push_posts_event_envelope() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-events/conv-1"))
            .and(body_partial_json(serde_json::json!({
                "event_id": "ev-1",
                "source": "sensor",
                "type": "reading",
                "payload": { "temp": 21 }
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "accepted", "event_id": "ev-1"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelEventCommands::Push {
            conversation_id: "conv-1".to_string(),
            source: "sensor".to_string(),
            event_type: "reading".to_string(),
            event_id: Some("ev-1".to_string()),
            timestamp: Some("2026-01-01T00:00:00Z".to_string()),
            payload_json: Some(r#"{"temp":21}"#.to_string()),
            payload_file: None,
            metadata_json: None,
            api_key: Some("nyxid_ag_x".to_string()),
            api_key_env: None,
            base: BaseUrlArgs {
                base_url: Some(server.uri()),
                profile: None,
            },
            output: OutputFormat::Json,
        })
        .await
        .expect("push should succeed");
    }

    #[tokio::test]
    async fn push_from_stdin_requires_explicit_key() {
        // payload-file "-" reads stdin, so the key can't be prompted → bail
        // before any HTTP. No mock needed.
        let result = run(ChannelEventCommands::Push {
            conversation_id: "conv-1".to_string(),
            source: "sensor".to_string(),
            event_type: "reading".to_string(),
            event_id: None,
            timestamp: None,
            payload_json: None,
            payload_file: Some("-".to_string()),
            metadata_json: None,
            api_key: None,
            api_key_env: None,
            base: BaseUrlArgs {
                base_url: Some("http://127.0.0.1:1".to_string()),
                profile: None,
            },
            output: OutputFormat::Json,
        })
        .await;
        assert!(
            result.is_err(),
            "stdin payload without a key must be rejected"
        );
    }

    #[tokio::test]
    async fn channel_create_posts_device_conversation() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-conversations"))
            .and(body_partial_json(serde_json::json!({
                "platform": "device",
                "platform_conversation_id": "dev-1",
                "agent_api_key_id": "key-1"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "c1", "platform_conversation_id": "dev-1"
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelEventCommands::Channel {
            command: ChannelEventChannelCommands::Create {
                conversation_id: "dev-1".to_string(),
                agent_key_id: "key-1".to_string(),
                conversation_type: None,
                org: None,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("channel create should succeed");
    }

    #[tokio::test]
    async fn channel_delete_with_yes_deletes() {
        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/api/v1/channel-conversations/c1"))
            .respond_with(ResponseTemplate::new(204))
            .expect(1)
            .mount(&server)
            .await;

        run(ChannelEventCommands::Channel {
            command: ChannelEventChannelCommands::Delete {
                id: "c1".to_string(),
                yes: true,
                auth: mock_auth(server.uri()),
            },
        })
        .await
        .expect("channel delete should succeed");
    }

    // --- Pure parser helpers ---

    #[test]
    fn resolve_payload_rejects_both_sources() {
        assert!(resolve_payload(Some("{}"), Some("f.json")).is_err());
    }

    #[test]
    fn resolve_payload_parses_inline_json() {
        let v = resolve_payload(Some(r#"{"a":1}"#), None)
            .expect("ok")
            .expect("some");
        assert_eq!(v["a"], 1);
    }

    #[test]
    fn resolve_payload_rejects_invalid_json() {
        assert!(resolve_payload(Some("{not json"), None).is_err());
    }

    #[test]
    fn resolve_payload_none_when_absent() {
        assert!(resolve_payload(None, None).expect("ok").is_none());
    }

    #[test]
    fn resolve_api_key_returns_inline_value() {
        assert_eq!(
            resolve_api_key(Some("nyxid_ag_x"), None).expect("ok"),
            "nyxid_ag_x"
        );
    }

    #[test]
    fn parse_optional_json_rejects_invalid() {
        assert!(parse_optional_json(Some("{bad"), "metadata").is_err());
    }

    #[test]
    fn parse_optional_json_parses_valid() {
        let v = parse_optional_json(Some(r#"{"k":1}"#), "metadata")
            .expect("ok")
            .expect("some");
        assert_eq!(v["k"], 1);
    }

    #[test]
    fn parse_optional_json_none_when_absent() {
        assert!(parse_optional_json(None, "metadata").expect("ok").is_none());
    }

    #[tokio::test]
    async fn push_table_output() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-events/conv-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "status": "accepted", "event_id": "ev-auto"
            })))
            .mount(&server)
            .await;

        run(ChannelEventCommands::Push {
            conversation_id: "conv-1".to_string(),
            source: "sensor".to_string(),
            event_type: "reading".to_string(),
            event_id: None,
            timestamp: None,
            payload_json: None,
            payload_file: None,
            metadata_json: Some(r#"{"region":"us"}"#.to_string()),
            api_key: Some("nyxid_ag_x".to_string()),
            api_key_env: None,
            base: BaseUrlArgs {
                base_url: Some(server.uri()),
                profile: None,
            },
            output: OutputFormat::Table,
        })
        .await
        .expect("push table should succeed");
    }

    #[tokio::test]
    async fn channel_create_table_output() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/channel-conversations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "c1", "platform_conversation_id": "dev-1",
                "platform_conversation_type": "sensor"
            })))
            .mount(&server)
            .await;

        run(ChannelEventCommands::Channel {
            command: ChannelEventChannelCommands::Create {
                conversation_id: "dev-1".to_string(),
                agent_key_id: "key-1".to_string(),
                conversation_type: Some("sensor".to_string()),
                org: None,
                auth: crate::test_support::mock_auth_with_output(server.uri(), OutputFormat::Table),
            },
        })
        .await
        .expect("channel create table should succeed");
    }

    #[tokio::test]
    async fn channel_list_filters_device_platform() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/channel-conversations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "conversations": [
                    {"id": "c1", "platform": "device", "is_active": true,
                     "platform_conversation_id": "dev-1", "agent_api_key_id": "key-1"},
                    {"id": "c2", "platform": "telegram", "is_active": true}
                ]
            })))
            .mount(&server)
            .await;

        run(ChannelEventCommands::Channel {
            command: ChannelEventChannelCommands::List {
                org: None,
                auth: crate::test_support::mock_auth_with_output(server.uri(), OutputFormat::Table),
            },
        })
        .await
        .expect("channel list should succeed");
    }

    #[tokio::test]
    async fn channel_list_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/channel-conversations"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "conversations": []
            })))
            .mount(&server)
            .await;

        run(ChannelEventCommands::Channel {
            command: ChannelEventChannelCommands::List {
                org: None,
                auth: crate::test_support::mock_auth_with_output(server.uri(), OutputFormat::Table),
            },
        })
        .await
        .expect("empty channel list should succeed");
    }
}
