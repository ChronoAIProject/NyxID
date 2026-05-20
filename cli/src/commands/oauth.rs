//! CLI commands for managing OAuth broker bindings (#549).
//!
//! Wraps the user-scope endpoints landed in commit #6:
//! - GET /api/v1/users/me/broker-bindings
//! - DELETE /api/v1/users/me/broker-bindings/{binding_hash}

use std::io::Write;

use anyhow::{Result, bail};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{BindingCommands, OauthCommands, OutputFormat};

const PREFIX_MIN_LEN: usize = 8;

pub async fn run(command: OauthCommands) -> Result<()> {
    match command {
        OauthCommands::Bindings { command } => run_bindings(command).await,
    }
}

async fn run_bindings(command: BindingCommands) -> Result<()> {
    match command {
        BindingCommands::List { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let response: Value = api.get("/users/me/broker-bindings").await?;
            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&response)?);
                }
                OutputFormat::Table => {
                    print_bindings_table(&response);
                }
            }
            Ok(())
        }

        BindingCommands::Show { id_or_hash, auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let response: Value = api.get("/users/me/broker-bindings").await?;
            let binding = resolve_binding(&response, &id_or_hash)?;
            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(binding)?);
                }
                OutputFormat::Table => {
                    print_binding_detail(binding);
                }
            }
            Ok(())
        }

        BindingCommands::Revoke {
            id_or_hash,
            yes,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let response: Value = api.get("/users/me/broker-bindings").await?;
            let binding = resolve_binding(&response, &id_or_hash)?;
            let binding_hash = binding["binding_hash"]
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("response missing binding_hash"))?
                .to_string();
            let label = binding["client_name"]
                .as_str()
                .or_else(|| binding["client_id"].as_str())
                .unwrap_or("this binding")
                .to_string();

            if !yes && !confirm(&format!("Revoke broker binding for {label}?"))? {
                return Ok(());
            }

            api.delete_empty(&format!("/users/me/broker-bindings/{binding_hash}"))
                .await?;

            match auth.output {
                OutputFormat::Json => {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "ok": true,
                            "binding_hash": binding_hash,
                            "resource_type": "broker_binding",
                            "status": "revoked",
                        }))?
                    );
                }
                OutputFormat::Table => {
                    eprintln!("Broker binding revoked.");
                }
            }
            Ok(())
        }
    }
}

fn resolve_binding<'a>(response: &'a Value, id_or_hash: &str) -> Result<&'a Value> {
    let bindings = response["bindings"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("malformed response: expected 'bindings' to be an array"))?;

    if id_or_hash.len() < PREFIX_MIN_LEN {
        bail!(
            "binding hash prefix must be at least {PREFIX_MIN_LEN} characters; got {}",
            id_or_hash.len()
        );
    }

    let matches: Vec<&Value> = bindings
        .iter()
        .filter(|b| {
            b["binding_hash"]
                .as_str()
                .is_some_and(|h| h.starts_with(id_or_hash))
        })
        .collect();

    match matches.len() {
        0 => bail!("no broker binding matches '{id_or_hash}'"),
        1 => Ok(matches[0]),
        n => bail!("ambiguous prefix '{id_or_hash}' matched {n} bindings; provide more characters"),
    }
}

fn print_bindings_table(response: &Value) {
    let bindings = response["bindings"].as_array();
    let bindings = match bindings {
        Some(bindings) => bindings,
        None => {
            eprintln!("No bindings.");
            return;
        }
    };
    if bindings.is_empty() {
        eprintln!("No bindings.");
        return;
    }

    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED).set_header(vec![
        "Hash (prefix)",
        "Application",
        "External account",
        "Scopes",
        "Created",
        "Last used",
    ]);

    for binding in bindings {
        let hash = binding["binding_hash"].as_str().unwrap_or("-");
        let hash_short = hash.chars().take(12).collect::<String>();
        let app = binding["client_name"]
            .as_str()
            .or_else(|| binding["client_id"].as_str())
            .unwrap_or("-")
            .to_string();
        let external = format_external_subject(&binding["external_subject"]);
        let scopes = binding["scopes"]
            .as_array()
            .map(|s| {
                s.iter()
                    .filter_map(Value::as_str)
                    .collect::<Vec<_>>()
                    .join(" ")
            })
            .unwrap_or_else(|| "-".to_string());
        let created = binding["created_at"].as_str().unwrap_or("-");
        let last_used = binding["last_used_at"].as_str().unwrap_or("-");

        table.add_row(vec![
            hash_short,
            app,
            external,
            scopes,
            created.to_string(),
            last_used.to_string(),
        ]);
    }

    println!("{table}");
}

fn print_binding_detail(binding: &Value) {
    let hash = binding["binding_hash"].as_str().unwrap_or("-");
    let client_id = binding["client_id"].as_str().unwrap_or("-");
    let app_name = binding["client_name"].as_str().unwrap_or(client_id);
    let external = format_external_subject(&binding["external_subject"]);
    let scopes = binding["scopes"]
        .as_array()
        .map(|s| {
            s.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| "-".to_string());
    let created = binding["created_at"].as_str().unwrap_or("-");
    let last_used = binding["last_used_at"].as_str().unwrap_or("-");

    eprintln!("Hash:              {hash}");
    eprintln!("Application:       {app_name}");
    eprintln!("Client ID:         {client_id}");
    eprintln!("External account:  {external}");
    eprintln!("Scopes:            {scopes}");
    eprintln!("Created:           {created}");
    eprintln!("Last used:         {last_used}");
}

fn format_external_subject(value: &Value) -> String {
    if value.is_null() {
        return "-".to_string();
    }
    let platform = value["platform"].as_str().unwrap_or("-");
    let tenant = value["tenant"].as_str();
    let user = value["external_user_id"].as_str().unwrap_or("-");
    match tenant {
        Some(t) => format!("{platform} · {t} · {user}"),
        None => format!("{platform} · {user}"),
    }
}

fn confirm(prompt: &str) -> Result<bool> {
    eprint!("{prompt} [y/N] ");
    std::io::stderr().flush()?;
    let mut answer = String::new();
    std::io::stdin().read_line(&mut answer)?;
    if !answer.trim().eq_ignore_ascii_case("y") {
        eprintln!("Cancelled.");
        return Ok(false);
    }
    Ok(true)
}
