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
use serde_json::Value;
use uuid::Uuid;

use crate::api::ApiClient;
use crate::cli::{ChannelEventCommands, OutputFormat};

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
