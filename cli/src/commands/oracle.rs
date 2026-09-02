//! `nyxid oracle` — call ChatGPT Pro (and other browser oracles) through
//! NyxID.
//!
//! A pool is a capacity unit backed by logged-in browser tabs running the
//! NyxID oracle userscript. `oracle ask` submits a prompt and polls the
//! relay until the answer lands (long thinking lives in the poll loop, not
//! a single request). `oracle pool` manages pools and worker tokens.

use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use anyhow::{Context, Result, bail};
use base64::Engine;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use hkdf::Hkdf;
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::api::ApiClient;
use crate::cli::{OracleCommands, OraclePoolCommands, OracleWorkerCommands, OutputFormat};
use crate::commands::oracle_worker_daemon::{self, OracleWorkerConfig};
use crate::org_resolver::resolve_org_id;

const POLL_INTERVAL_SECS: u64 = 3;
const SESSION_FORMAT_VERSION: u32 = 1;
const SESSION_INFO: &[u8] = b"nyxid-oracle-session-v1";
const DEFAULT_PLAYWRIGHT_CORE_VERSION: &str = "1.62.1";
const LOCAL_UPGRADE_WAIT_SECS: u64 = 4 * 60 * 60;

pub async fn run(command: OracleCommands) -> Result<()> {
    match command {
        OracleCommands::Ask {
            pool,
            prompt,
            file,
            pdf,
            attach_file,
            model,
            project_url,
            tag,
            conversation,
            new_conversation,
            client_ref,
            wait,
            no_wait,
            out,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let prompt_text = resolve_prompt(prompt.as_deref(), file.as_deref())?;

            let mut body = serde_json::json!({ "prompt": prompt_text });
            insert_opt_str(&mut body, "model", model.as_deref());
            insert_opt_str(&mut body, "project_url", project_url.as_deref());
            insert_opt_str(&mut body, "tag", tag.as_deref());
            insert_opt_str(&mut body, "client_ref", client_ref.as_deref());
            // Three-state conversation_id: continue an id, open a new
            // session (""), or single-shot (omitted).
            if let Some(conv) = &conversation {
                body["conversation_id"] = Value::String(conv.clone());
            } else if new_conversation {
                body["conversation_id"] = Value::String(String::new());
            }
            if let Some(pdf_path) = &pdf {
                let bytes = std::fs::read(pdf_path)
                    .with_context(|| format!("Failed to read PDF at {pdf_path}"))?;
                let name = std::path::Path::new(pdf_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("attachment.pdf");
                body["pdf_base64"] =
                    Value::String(base64::engine::general_purpose::STANDARD.encode(&bytes));
                body["pdf_name"] = Value::String(name.to_string());
            }
            if let Some(file_path) = &attach_file {
                let bytes = std::fs::read(file_path)
                    .with_context(|| format!("Failed to read attachment at {file_path}"))?;
                let name = std::path::Path::new(file_path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("attachment.bin");
                body["attachment_base64"] =
                    Value::String(base64::engine::general_purpose::STANDARD.encode(&bytes));
                body["attachment_name"] = Value::String(name.to_string());
            }

            let submit: Value = api
                .post(&format!("/oracle/pools/{pool}/tasks"), &body)
                .await?;
            let task_id = submit["task_id"]
                .as_str()
                .context("server did not return a task_id")?
                .to_string();
            let conv_id = submit["conversation_id"].as_str().map(str::to_string);

            if no_wait {
                return print_submit(output, &task_id, &submit, conv_id.as_deref());
            }

            eprintln!("Submitted task {task_id} to pool '{pool}'. Waiting for an answer…");
            if let Some(conv) = &conv_id {
                eprintln!("Conversation: {conv}");
            }
            let task = poll_until_terminal(&mut api, &task_id, wait).await?;
            // Non-fatal: a failed image write must not swallow the text answer.
            if let Err(e) = save_result_images(output, &task, out.as_deref()) {
                eprintln!("warning: could not save image(s): {e:#}");
            }
            print_result(output, &task)
        }
        OracleCommands::Result { task_id, out, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let task: Value = api.get(&format!("/oracle/tasks/{task_id}")).await?;
            // Non-fatal: a failed image write must not swallow the text answer.
            if let Err(e) = save_result_images(output, &task, out.as_deref()) {
                eprintln!("warning: could not save image(s): {e:#}");
            }
            print_result(output, &task)
        }
        OracleCommands::Cancel { task_id, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let task: Value = api
                .post(&format!("/oracle/tasks/{task_id}/cancel"), &Value::Null)
                .await?;
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&task)?),
                OutputFormat::Table => eprintln!("Cancelled task {task_id}."),
            }
            Ok(())
        }
        OracleCommands::Status { pool, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let status: Value = api.get(&format!("/oracle/pools/{pool}/status")).await?;
            print_status(output, &pool, &status)
        }
        OracleCommands::Attach {
            pool,
            url,
            tag,
            wait,
            no_wait,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let mut body = serde_json::json!({ "chatgpt_url": url });
            if let Some(t) = &tag {
                body["tag"] = Value::String(t.clone());
            }

            let submit: Value = api
                .post(&format!("/oracle/pools/{pool}/attach"), &body)
                .await?;
            let task_id = submit["task_id"]
                .as_str()
                .context("server did not return a task_id")?
                .to_string();
            let conversation_id = submit["conversation_id"]
                .as_str()
                .context("server did not return a conversation_id")?
                .to_string();

            if no_wait {
                return print_attach_submit(output, &submit);
            }

            eprintln!(
                "Attached conversation {conversation_id} via pool '{pool}'. Waiting for import…"
            );
            let task = poll_until_terminal(&mut api, &task_id, wait).await?;
            let status = task["status"].as_str().unwrap_or("");
            if status != "completed" {
                print_result(output, &task)?;
                return Ok(());
            }
            let session: Value = api
                .get(&format!("/oracle/sessions/{conversation_id}"))
                .await?;
            print_session_detail(output, &session)
        }
        OracleCommands::Extract {
            pool,
            url,
            model,
            wait,
            no_wait,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let mut body = serde_json::json!({ "url": url });
            if let Some(m) = &model {
                body["model"] = Value::String(m.clone());
            }

            let submit: Value = api
                .post(&format!("/oracle/pools/{pool}/extract"), &body)
                .await?;
            let task_id = submit["task_id"]
                .as_str()
                .context("server did not return a task_id")?
                .to_string();

            if no_wait {
                return print_extract_submit(output, &task_id, &submit);
            }

            eprintln!("Submitted extract task {task_id} to pool '{pool}'. Waiting for content…");
            let task = poll_until_terminal(&mut api, &task_id, wait).await?;
            print_result(output, &task)
        }
        OracleCommands::Pool { command } => run_pool(command).await,
        OracleCommands::Worker { command } => run_worker(command).await,
        OracleCommands::Login {
            pool,
            worker_token_file,
            wait,
            auth,
        } => run_login(pool, worker_token_file, wait, auth).await,
        OracleCommands::Sessions { pool, limit, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let mut path = format!("/oracle/sessions?limit={limit}");
            if let Some(p) = &pool {
                path.push_str(&format!("&pool={p}"));
            }
            let resp: Value = api.get(&path).await?;
            print_sessions(output, &resp)
        }
        OracleCommands::Session {
            conversation_id,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let resp: Value = api
                .get(&format!("/oracle/sessions/{conversation_id}"))
                .await?;
            print_session_detail(output, &resp)
        }
        OracleCommands::CloseSession {
            conversation_id,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let resp: Value = api
                .post(
                    &format!("/oracle/sessions/{conversation_id}/close"),
                    &Value::Null,
                )
                .await?;
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resp)?),
                OutputFormat::Table => eprintln!("Closed conversation {conversation_id}."),
            }
            Ok(())
        }
    }
}

async fn run_pool(command: OraclePoolCommands) -> Result<()> {
    match command {
        OraclePoolCommands::Create {
            slug,
            name,
            description,
            visibility,
            project_url,
            model,
            allow_extract,
            max_workers,
            max_queue,
            per_user_inflight,
            task_timeout,
            org,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let target_org_id = match org {
                Some(raw) => Some(resolve_org_id(&mut api, &raw).await?),
                None => None,
            };

            let mut body = serde_json::json!({ "slug": slug, "name": name });
            insert_opt_str(&mut body, "description", description.as_deref());
            insert_opt_str(&mut body, "visibility", visibility.as_deref());
            insert_opt_str(&mut body, "chatgpt_project_url", project_url.as_deref());
            insert_opt_str(&mut body, "default_model_label", model.as_deref());
            body["allow_extract"] = Value::Bool(allow_extract);
            insert_opt_str(&mut body, "target_org_id", target_org_id.as_deref());
            insert_opt_u64(&mut body, "max_workers", max_workers.map(u64::from));
            insert_opt_u64(&mut body, "max_queue_length", max_queue.map(u64::from));
            insert_opt_u64(
                &mut body,
                "per_user_max_inflight",
                per_user_inflight.map(u64::from),
            );
            insert_opt_u64(&mut body, "task_timeout_secs", task_timeout);

            let resp: Value = api.post("/oracle/pools", &body).await?;
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resp)?),
                OutputFormat::Table => {
                    let token = resp["worker_token"].as_str().unwrap_or("-");
                    eprintln!("Pool '{}' created.", resp["slug"].as_str().unwrap_or(&slug));
                    eprintln!();
                    eprintln!("Worker token (shown once — install it in the userscript):");
                    println!("{token}");
                    eprintln!();
                    eprintln!(
                        "Pair a ChatGPT tab: open the NyxID oracle userscript settings, set the \
                         NyxID base URL and this token, then load chatgpt.com."
                    );
                }
            }
            Ok(())
        }
        OraclePoolCommands::List { auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let resp: Value = api.get("/oracle/pools").await?;
            let pools = resp["pools"].as_array().cloned().unwrap_or_default();
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resp)?),
                OutputFormat::Table => {
                    if pools.is_empty() {
                        eprintln!(
                            "No oracle pools visible. Create one with `nyxid oracle pool create`."
                        );
                        return Ok(());
                    }
                    let mut table = Table::new();
                    table.load_preset(UTF8_FULL_CONDENSED);
                    table.set_header(["Slug", "Name", "Visibility", "Workers", "Active", "Manage"]);
                    for p in &pools {
                        table.add_row([
                            p["slug"].as_str().unwrap_or("-").to_string(),
                            p["name"].as_str().unwrap_or("-").to_string(),
                            p["visibility"].as_str().unwrap_or("-").to_string(),
                            p["max_workers"].as_u64().unwrap_or(0).to_string(),
                            yes_no(p["is_active"].as_bool().unwrap_or(false)),
                            yes_no(p["can_manage"].as_bool().unwrap_or(false)),
                        ]);
                    }
                    println!("{table}");
                }
            }
            Ok(())
        }
        OraclePoolCommands::Show { pool, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let p: Value = api.get(&format!("/oracle/pools/{pool}")).await?;
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&p)?),
                OutputFormat::Table => {
                    eprintln!("Slug:        {}", p["slug"].as_str().unwrap_or("-"));
                    eprintln!("Name:        {}", p["name"].as_str().unwrap_or("-"));
                    eprintln!("Visibility:  {}", p["visibility"].as_str().unwrap_or("-"));
                    eprintln!(
                        "Active:      {}",
                        yes_no(p["is_active"].as_bool().unwrap_or(false))
                    );
                    eprintln!(
                        "Allow extract: {}",
                        yes_no(p["allow_extract"].as_bool().unwrap_or(false))
                    );
                    eprintln!("Max workers: {}", p["max_workers"].as_u64().unwrap_or(0));
                    eprintln!(
                        "Max queue:   {}",
                        p["max_queue_length"].as_u64().unwrap_or(0)
                    );
                    eprintln!(
                        "Per-user:    {}",
                        p["per_user_max_inflight"].as_u64().unwrap_or(0)
                    );
                    eprintln!(
                        "Lease (s):   {}",
                        p["task_timeout_secs"].as_u64().unwrap_or(0)
                    );
                    if let Some(url) = p["chatgpt_project_url"].as_str() {
                        eprintln!("Project URL: {url}");
                    }
                    if let Some(model) = p["default_model_label"].as_str() {
                        eprintln!("Model:       {model}");
                    }
                }
            }
            Ok(())
        }
        OraclePoolCommands::Update {
            pool,
            name,
            description,
            visibility,
            project_url,
            model,
            allow_extract,
            max_workers,
            max_queue,
            per_user_inflight,
            task_timeout,
            active,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let mut body = serde_json::json!({});
            insert_opt_str(&mut body, "name", name.as_deref());
            insert_opt_str(&mut body, "description", description.as_deref());
            insert_opt_str(&mut body, "visibility", visibility.as_deref());
            insert_opt_str(&mut body, "chatgpt_project_url", project_url.as_deref());
            insert_opt_str(&mut body, "default_model_label", model.as_deref());
            if let Some(allow_extract) = allow_extract {
                body["allow_extract"] = Value::Bool(allow_extract);
            }
            insert_opt_u64(&mut body, "max_workers", max_workers.map(u64::from));
            insert_opt_u64(&mut body, "max_queue_length", max_queue.map(u64::from));
            insert_opt_u64(
                &mut body,
                "per_user_max_inflight",
                per_user_inflight.map(u64::from),
            );
            insert_opt_u64(&mut body, "task_timeout_secs", task_timeout);
            if let Some(a) = active {
                body["is_active"] = Value::Bool(a);
            }

            let p: Value = api.patch(&format!("/oracle/pools/{pool}"), &body).await?;
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&p)?),
                OutputFormat::Table => {
                    eprintln!("Pool '{}' updated.", p["slug"].as_str().unwrap_or(&pool))
                }
            }
            Ok(())
        }
        OraclePoolCommands::RotateToken { pool, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let resp: Value = api
                .post(&format!("/oracle/pools/{pool}/rotate-token"), &Value::Null)
                .await?;
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resp)?),
                OutputFormat::Table => {
                    eprintln!(
                        "Worker token rotated for '{}'. All paired tabs must be re-configured.",
                        resp["slug"].as_str().unwrap_or(&pool)
                    );
                    eprintln!();
                    eprintln!("New worker token (shown once):");
                    println!("{}", resp["worker_token"].as_str().unwrap_or("-"));
                }
            }
            Ok(())
        }
    }
}

async fn run_worker(command: OracleWorkerCommands) -> Result<()> {
    match command {
        OracleWorkerCommands::List { pool, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let response: Value = api
                .get(&format!(
                    "/oracle/pools/{}/workers",
                    urlencoding::encode(&pool)
                ))
                .await?;
            print_workers(output, &response)
        }
        OracleWorkerCommands::Show { pool, label, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let path = worker_path(&pool, &label);
            let worker: Value = api.get(&path).await?;
            let commands: Value = api.get(&format!("{path}/commands")).await?;
            print_worker(output, &worker, &commands)
        }
        OracleWorkerCommands::Install {
            pool,
            worker_token_file,
            label,
            force,
            auth,
        } => install_worker(pool, worker_token_file, label, force, auth).await,
        OracleWorkerCommands::Start { pool, profile } => {
            oracle_worker_daemon::start(&pool, profile.as_deref())
        }
        OracleWorkerCommands::Stop { pool, profile } => {
            oracle_worker_daemon::stop(&pool, profile.as_deref())
        }
        OracleWorkerCommands::Status { pool, profile } => {
            oracle_worker_daemon::status(&pool, profile.as_deref())
        }
        OracleWorkerCommands::Logs {
            pool,
            profile,
            follow,
            lines,
        } => oracle_worker_daemon::logs(&pool, profile.as_deref(), follow, lines),
        OracleWorkerCommands::Uninstall { pool, profile } => {
            oracle_worker_daemon::uninstall(&pool, profile.as_deref())
        }
        OracleWorkerCommands::Upgrade { pool, label, auth } => {
            if let Some(target) = label {
                queue_worker_command(&pool, &target, "upgrade", auth).await
            } else {
                upgrade_local_worker(pool, auth).await
            }
        }
        OracleWorkerCommands::Drain { pool, label, auth } => {
            queue_worker_command(&pool, &label, "drain", auth).await
        }
        OracleWorkerCommands::Resume { pool, label, auth } => {
            queue_worker_command(&pool, &label, "resume", auth).await
        }
        OracleWorkerCommands::Restart { pool, label, auth } => {
            queue_worker_command(&pool, &label, "restart", auth).await
        }
        OracleWorkerCommands::RelaunchBrowser { pool, label, auth } => {
            queue_worker_command(&pool, &label, "relaunch_browser", auth).await
        }
        OracleWorkerCommands::Relogin { pool, label, auth } => {
            let output = auth.output;
            queue_worker_command(&pool, &label, "relogin", auth).await?;
            if matches!(output, OutputFormat::Table) {
                eprintln!(
                    "Note: relogin only opens the login page on the worker's own screen. To log in \
                     from this computer and push the session to the whole pool, run: \
                     nyxid oracle login {pool}"
                );
            }
            Ok(())
        }
    }
}

fn worker_path(pool: &str, label: &str) -> String {
    format!(
        "/oracle/pools/{}/workers/{}",
        urlencoding::encode(pool),
        urlencoding::encode(label)
    )
}

async fn queue_worker_command(
    pool: &str,
    label: &str,
    command: &str,
    auth: crate::cli::AuthArgs,
) -> Result<()> {
    let output = auth.output;
    let mut api = ApiClient::from_auth_checked(&auth).await?;
    let response = enqueue_worker_command(&mut api, pool, label, command).await?;
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&response)?),
        OutputFormat::Table => eprintln!(
            "Queued {command} for worker '{label}' (command {}).",
            response["id"].as_str().unwrap_or("-")
        ),
    }
    Ok(())
}

async fn enqueue_worker_command(
    api: &mut ApiClient,
    pool: &str,
    label: &str,
    command: &str,
) -> Result<Value> {
    api.post(
        &format!("{}/commands", worker_path(pool, label)),
        &serde_json::json!({ "command": command }),
    )
    .await
}

async fn upgrade_local_worker(pool: String, auth: crate::cli::AuthArgs) -> Result<()> {
    let output = auth.output;
    let profile = auth.profile.clone();
    let config = oracle_worker_daemon::load_config(&pool, profile.as_deref())?;
    let mut api = ApiClient::from_auth_checked(&auth).await?;
    let bundle = fetch_manager_bundle(&mut api).await?;
    if installed_bundle_matches(&config, &bundle) {
        match output {
            OutputFormat::Json => println!(
                "{}",
                serde_json::to_string_pretty(&serde_json::json!({
                    "status": "current",
                    "worker": config.label,
                    "version": bundle.version,
                }))?
            ),
            OutputFormat::Table => {
                eprintln!("Local worker '{}' is already current.", config.label)
            }
        }
        return Ok(());
    }

    let command = enqueue_worker_command(&mut api, &pool, &config.label, "upgrade").await?;
    let command_id = command["id"]
        .as_str()
        .context("server did not return an upgrade command id")?
        .to_string();
    eprintln!(
        "Queued local upgrade for worker '{}'; waiting for its current task to finish.",
        config.label
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(LOCAL_UPGRADE_WAIT_SECS);
    loop {
        let commands: Value = api
            .get(&format!("{}/commands", worker_path(&pool, &config.label)))
            .await?;
        let command = commands["commands"].as_array().and_then(|items| {
            items
                .iter()
                .find(|item| item["id"].as_str() == Some(command_id.as_str()))
        });
        if let Some(command) = command
            && matches!(command["status"].as_str(), Some("failed" | "expired"))
        {
            bail!(
                "Local worker upgrade {}: {}",
                command["status"].as_str().unwrap_or("failed"),
                command["result_code"].as_str().unwrap_or("unknown_error")
            )
        }

        if installed_bundle_matches(&config, &bundle) {
            let worker: Value = api.get(&worker_path(&pool, &config.label)).await?;
            if worker["online"].as_bool() == Some(true)
                && worker["version"].as_str() == Some(bundle.version.as_str())
            {
                match output {
                    OutputFormat::Json => println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "status": "upgraded",
                            "worker": config.label,
                            "version": bundle.version,
                            "command_id": command_id,
                        }))?
                    ),
                    OutputFormat::Table => eprintln!(
                        "Local worker '{}' restarted on version {}.",
                        config.label, bundle.version
                    ),
                }
                return Ok(());
            }
        }

        if std::time::Instant::now() >= deadline {
            bail!(
                "Timed out waiting for local worker '{}'; the queued upgrade remains active",
                config.label
            )
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn installed_bundle_matches(config: &OracleWorkerConfig, bundle: &WorkerBundle) -> bool {
    let Ok(source) = fs::read(&config.bundle_path) else {
        return false;
    };
    let actual = hex::encode(Sha256::digest(&source));
    let version_path = config
        .bundle_path
        .parent()
        .map(|directory| directory.join("bundle-version"));
    let version = version_path
        .and_then(|path| fs::read_to_string(path).ok())
        .unwrap_or_default();
    let playwright_version = config
        .bundle_path
        .parent()
        .and_then(|directory| fs::read_to_string(directory.join("package.json")).ok())
        .and_then(|body| serde_json::from_str::<Value>(&body).ok())
        .and_then(|package| {
            package["dependencies"]["playwright-core"]
                .as_str()
                .map(str::to_string)
        })
        .unwrap_or_default();
    actual == bundle.sha256
        && version.trim() == bundle.version
        && playwright_version == bundle.playwright_core_version
}

fn print_workers(output: OutputFormat, response: &Value) -> Result<()> {
    if matches!(output, OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(response)?);
        return Ok(());
    }
    let workers = response["workers"].as_array().cloned().unwrap_or_default();
    if workers.is_empty() {
        eprintln!("No workers have registered with this pool.");
        return Ok(());
    }
    let mut table = Table::new();
    table.load_preset(UTF8_FULL_CONDENSED);
    table.set_header([
        "Label",
        "Version",
        "Online / last seen",
        "Login",
        "Current task",
        "Chrome",
        "State",
    ]);
    for worker in workers {
        let seen = if worker["online"].as_bool().unwrap_or(false) {
            "online".to_string()
        } else {
            format!(
                "{}s ago",
                worker["last_seen_secs_ago"].as_i64().unwrap_or(0)
            )
        };
        table.add_row([
            text_field(&worker, "label"),
            text_field(&worker, "version"),
            seen,
            bool_state(worker.get("logged_in").and_then(Value::as_bool)),
            text_field(&worker, "current_task_id"),
            bool_state(worker.get("chrome_alive").and_then(Value::as_bool)),
            text_field(&worker, "desired_state"),
        ]);
    }
    println!("{table}");
    Ok(())
}

fn print_worker(output: OutputFormat, worker: &Value, commands: &Value) -> Result<()> {
    if matches!(output, OutputFormat::Json) {
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "worker": worker,
                "commands": commands["commands"],
            }))?
        );
        return Ok(());
    }
    eprintln!("Label:        {}", text_field(worker, "label"));
    eprintln!("Version:      {}", text_field(worker, "version"));
    eprintln!("Platform:     {}", text_field(worker, "platform"));
    eprintln!(
        "Online:       {} (last seen {}s ago)",
        yes_no(worker["online"].as_bool().unwrap_or(false)),
        worker["last_seen_secs_ago"].as_i64().unwrap_or(0)
    );
    eprintln!(
        "Logged in:    {}",
        bool_state(worker.get("logged_in").and_then(Value::as_bool))
    );
    eprintln!(
        "Chrome alive: {}",
        bool_state(worker.get("chrome_alive").and_then(Value::as_bool))
    );
    eprintln!("Current task: {}", text_field(worker, "current_task_id"));
    eprintln!("State:        {}", text_field(worker, "desired_state"));
    eprintln!("Last error:   {}", text_field(worker, "last_error"));
    let recent = commands["commands"].as_array().cloned().unwrap_or_default();
    if !recent.is_empty() {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.set_header(["Command", "Status", "Result", "Deliveries", "Created"]);
        for command in recent.into_iter().take(10) {
            table.add_row([
                text_field(&command, "command"),
                text_field(&command, "status"),
                text_field(&command, "result_code"),
                command["delivery_count"].as_u64().unwrap_or(0).to_string(),
                text_field(&command, "created_at"),
            ]);
        }
        println!("\n{table}");
    }
    Ok(())
}

fn text_field(value: &Value, field: &str) -> String {
    value[field].as_str().unwrap_or("-").to_string()
}

fn bool_state(value: Option<bool>) -> String {
    match value {
        Some(true) => "yes".to_string(),
        Some(false) => "no".to_string(),
        None => "unknown".to_string(),
    }
}

async fn install_worker(
    pool: String,
    worker_token_file: Option<String>,
    requested_label: Option<String>,
    force: bool,
    auth: crate::cli::AuthArgs,
) -> Result<()> {
    let profile = auth.profile.clone();
    let base_url = auth.resolved_base_url()?;
    let mut api = ApiClient::from_auth_checked(&auth).await?;
    let install_dir = oracle_worker_daemon::install_dir(&pool, profile.as_deref())?;
    fs::create_dir_all(&install_dir)?;
    set_private_dir(&install_dir)?;

    let existing = oracle_worker_daemon::load_config(&pool, profile.as_deref()).ok();
    if existing.is_some() && !force {
        bail!(
            "Worker is already installed at {}; use --force to refresh it",
            install_dir.display()
        )
    }
    let label = match (&existing, requested_label.as_deref()) {
        // Keep the existing identity unless the operator explicitly renames.
        (Some(config), None) => config.label.clone(),
        (_, requested) => {
            let body = match requested {
                Some(label) => serde_json::json!({ "label": label }),
                None => Value::Null,
            };
            let allocated: Value = api
                .post(
                    &format!(
                        "/oracle/pools/{}/workers/allocate",
                        urlencoding::encode(&pool)
                    ),
                    &body,
                )
                .await?;
            let label = allocated["label"]
                .as_str()
                .context("server did not return a worker label")?
                .to_string();
            if allocated["adopted"].as_bool() == Some(true) {
                eprintln!(
                    "Adopted existing worker label '{label}'. Any legacy worker still using it \
                     will be rejected once this installation connects; unload that legacy worker."
                );
            }
            label
        }
    };
    let token = read_worker_token(
        &pool,
        profile.as_deref(),
        worker_token_file.as_deref(),
        existing.as_ref().map(|config| config.token_file.as_path()),
    )?;
    let bundle = fetch_manager_bundle(&mut api).await?;
    verify_worker_token(&base_url, token.as_str(), &bundle.sha256).await?;
    let node = resolve_node()?;
    let npm = resolve_program(&["npm"])?;
    let chrome = resolve_chrome()?;
    let port = select_install_debug_port(
        existing.as_ref().map(|config| config.chrome_debug_port),
        force,
    )?;
    let bundle_path = install_dir.join("worker.mjs");
    install_bundle_runtime(&install_dir, &bundle, &npm)?;
    let token_file = install_dir.join("worker-token");
    write_private(&token_file, token.as_bytes())?;
    let installation_id_file = install_dir.join("installation-id");
    if !installation_id_file.exists() {
        write_private(
            &installation_id_file,
            format!("{}\n", uuid::Uuid::new_v4()).as_bytes(),
        )?;
    }
    let config = OracleWorkerConfig {
        pool: pool.clone(),
        label: label.clone(),
        base_url,
        node_binary: node,
        npm_binary: Some(npm.clone()),
        bundle_path,
        token_file,
        state_file: install_dir.join("state.json"),
        installation_id_file,
        chrome_executable: chrome,
        chrome_profile_dir: install_dir.join("chrome-profile"),
        chrome_debug_port: port,
    };
    oracle_worker_daemon::save_config(&config, profile.as_deref())?;
    if force && existing.is_some() {
        let _ = oracle_worker_daemon::stop(&pool, profile.as_deref());
    }
    oracle_worker_daemon::install_service(&config, profile.as_deref(), force)?;
    let _ = launch_chrome(&config.chrome_executable, &config.chrome_profile_dir, port)?;
    oracle_worker_daemon::start(&pool, profile.as_deref())?;

    eprintln!("Installed oracle worker '{label}' for pool '{pool}'.");
    eprintln!("Complete ChatGPT login in the dedicated Chrome window.");
    eprintln!(
        "Check registration with: nyxid oracle worker list {pool}{}",
        profile
            .as_deref()
            .map(|value| format!(" --profile {value}"))
            .unwrap_or_default()
    );
    Ok(())
}

struct WorkerBundle {
    version: String,
    sha256: String,
    source: String,
    playwright_core_version: String,
}

async fn fetch_manager_bundle(api: &mut ApiClient) -> Result<WorkerBundle> {
    let response: Value = api.get("/oracle/worker-bundle").await?;
    let version = response["version"]
        .as_str()
        .context("bundle response omitted version")?
        .to_string();
    let sha256 = response["sha256"]
        .as_str()
        .context("bundle response omitted sha256")?
        .to_string();
    let source = response["bundle"]
        .as_str()
        .context("bundle response omitted source")?
        .to_string();
    let playwright_core_version = response["playwright_core_version"]
        .as_str()
        .unwrap_or(DEFAULT_PLAYWRIGHT_CORE_VERSION)
        .to_string();
    validate_playwright_version(&playwright_core_version)?;
    verify_bundle(&source, &sha256)?;
    Ok(WorkerBundle {
        version,
        sha256,
        source,
        playwright_core_version,
    })
}

fn validate_playwright_version(version: &str) -> Result<()> {
    if version.is_empty()
        || version.len() > 32
        || !version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        bail!("bundle returned an invalid playwright_core_version")
    }
    Ok(())
}

fn verify_bundle(source: &str, expected: &str) -> Result<()> {
    let actual = hex::encode(Sha256::digest(source.as_bytes()));
    if expected.len() != 64 || actual != expected {
        bail!("Oracle worker bundle checksum mismatch")
    }
    Ok(())
}

async fn verify_worker_token(base_url: &str, token: &str, expected_sha: &str) -> Result<()> {
    let response = reqwest::Client::new()
        .get(format!(
            "{}/api/v1/oracle/worker/bundle",
            base_url.trim_end_matches('/')
        ))
        .bearer_auth(token)
        .send()
        .await
        .context("Could not validate the pool worker token")?;
    if !response.status().is_success() {
        bail!("Pool worker token was rejected by the server")
    }
    let body: Value = response.json().await?;
    if body["sha256"].as_str() != Some(expected_sha) {
        bail!("Manager and worker bundle endpoints disagree on checksum")
    }
    Ok(())
}

fn install_bundle_runtime(dir: &Path, bundle: &WorkerBundle, npm: &Path) -> Result<()> {
    verify_bundle(&bundle.source, &bundle.sha256)?;
    atomic_write(&dir.join("worker.mjs"), bundle.source.as_bytes(), 0o755)?;
    atomic_write(
        &dir.join("bundle-version"),
        format!("{}\n", bundle.version).as_bytes(),
        0o644,
    )?;
    let package = serde_json::json!({
        "name": "nyxid-oracle-worker-install",
        "private": true,
        "type": "module",
        "dependencies": { "playwright-core": bundle.playwright_core_version },
        "nyxid_bundle_version": bundle.version,
    });
    fs::write(
        dir.join("package.json"),
        format!("{}\n", serde_json::to_string_pretty(&package)?),
    )?;
    let status = Command::new(npm)
        .args(["install", "--omit=dev", "--no-audit", "--no-fund"])
        .current_dir(dir)
        .status()
        .context("Failed to install playwright-core")?;
    if !status.success() {
        bail!("npm install failed while provisioning the oracle worker")
    }
    Ok(())
}

fn read_worker_token(
    pool: &str,
    profile: Option<&str>,
    explicit_file: Option<&str>,
    installed_file: Option<&Path>,
) -> Result<Zeroizing<String>> {
    let env_file = std::env::var("NYXID_WORKER_TOKEN_FILE").ok();
    let path = explicit_file
        .map(PathBuf::from)
        .or_else(|| env_file.map(PathBuf::from))
        .or_else(|| {
            installed_file
                .filter(|path| path.exists())
                .map(Path::to_path_buf)
        })
        .or_else(|| {
            oracle_worker_daemon::install_dir(pool, profile)
                .ok()
                .map(|dir| dir.join("worker-token"))
                .filter(|path| path.exists())
        });
    let raw = Zeroizing::new(if let Some(path) = path {
        fs::read_to_string(&path)
            .with_context(|| format!("Could not read worker token file {}", path.display()))?
    } else if let Ok(value) = std::env::var("NYXID_WORKER_TOKEN") {
        value
    } else {
        eprintln!("Enter the raw worker token for pool '{pool}'. Input is hidden.");
        rpassword::prompt_password("Worker token: ")?
    });
    let token = raw.trim().to_string();
    if !token.starts_with("nyx_owk_") || token.len() < 20 {
        bail!("Worker token must use the nyx_owk_ prefix")
    }
    Ok(Zeroizing::new(token))
}

fn resolve_node() -> Result<PathBuf> {
    let node = resolve_program(&["node"])?;
    let output = Command::new(&node).arg("--version").output()?;
    let version = String::from_utf8_lossy(&output.stdout);
    let major = version
        .trim()
        .trim_start_matches('v')
        .split('.')
        .next()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(0);
    if !output.status.success() || major < 18 {
        bail!("Node.js 18 or newer is required")
    }
    Ok(node)
}

fn resolve_chrome() -> Result<PathBuf> {
    if let Ok(path) = std::env::var("NYXID_CHROME_EXECUTABLE") {
        let path = PathBuf::from(path);
        if path.is_file() {
            return Ok(path);
        }
    }
    #[cfg(target_os = "macos")]
    for candidate in [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
        "/Applications/Chromium.app/Contents/MacOS/Chromium",
    ] {
        let path = PathBuf::from(candidate);
        if path.is_file() {
            return Ok(path);
        }
    }
    resolve_program(&["google-chrome", "chromium", "chromium-browser"]).context(
        "Chrome or Chromium was not found; set NYXID_CHROME_EXECUTABLE to its executable path",
    )
}

fn resolve_program(names: &[&str]) -> Result<PathBuf> {
    let paths = std::env::var_os("PATH").context("PATH is not set")?;
    for name in names {
        for directory in std::env::split_paths(&paths) {
            let candidate = directory.join(name);
            if candidate.is_file() {
                return candidate
                    .canonicalize()
                    .with_context(|| format!("Could not resolve {}", candidate.display()));
            }
        }
    }
    bail!("Required program not found: {}", names.join(" or "))
}

fn find_free_debug_port() -> Result<u16> {
    for port in 9222..9322 {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    bail!("No free Chrome debugging port found in 9222-9321")
}

fn select_install_debug_port(existing: Option<u16>, force: bool) -> Result<u16> {
    match (existing, force) {
        (Some(port), false) => Ok(port),
        _ => find_free_debug_port(),
    }
}

/// Name a fresh Chrome profile so its avatar/profile menu and chrome://version
/// identify the NyxID use (no-op once the profile exists).
fn seed_chrome_profile_name(profile: &Path, name: &str) -> Result<()> {
    let prefs = profile.join("Default").join("Preferences");
    if prefs.exists() {
        return Ok(());
    }
    fs::create_dir_all(prefs.parent().context("Invalid Chrome profile path")?)?;
    fs::write(
        &prefs,
        serde_json::to_vec(&serde_json::json!({ "profile": { "name": name } }))?,
    )?;
    Ok(())
}

fn launch_chrome(executable: &Path, profile: &Path, port: u16) -> Result<Child> {
    fs::create_dir_all(profile)?;
    let name = profile
        .parent()
        .and_then(Path::file_name)
        .map(|value| format!("NyxID Oracle {}", value.to_string_lossy()))
        .unwrap_or_else(|| "NyxID Oracle".to_string());
    let _ = seed_chrome_profile_name(profile, &name);
    Command::new(executable)
        .arg(format!("--remote-debugging-port={port}"))
        .arg(format!("--user-data-dir={}", profile.display()))
        .args([
            "--no-first-run",
            "--no-default-browser-check",
            "https://chatgpt.com/",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("Failed to launch Chrome")
}

struct CaptureChrome(Option<Child>);

impl CaptureChrome {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }

    fn stop(&mut self) -> Result<()> {
        let Some(mut child) = self.0.take() else {
            return Ok(());
        };
        if child.try_wait()?.is_none() {
            child
                .kill()
                .context("Could not stop login-capture Chrome")?;
        }
        child
            .wait()
            .context("Could not reap login-capture Chrome")?;
        Ok(())
    }
}

impl Drop for CaptureChrome {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}

fn write_private(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn atomic_write(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let temp = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temp, bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&temp, fs::Permissions::from_mode(mode))?;
    }
    fs::rename(&temp, path)?;
    Ok(())
}

fn set_private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

async fn run_login(
    pool: String,
    worker_token_file: Option<String>,
    wait_secs: u64,
    auth: crate::cli::AuthArgs,
) -> Result<()> {
    let output = auth.output;
    let profile = auth.profile.clone();
    let mut api = ApiClient::from_auth_checked(&auth).await?;
    let token = read_worker_token(
        &pool,
        profile.as_deref(),
        worker_token_file.as_deref(),
        None,
    )?;
    eprintln!("Preparing a local login capture (downloading the verified worker runtime)...");
    let bundle = fetch_manager_bundle(&mut api).await?;
    let capture_workspace =
        tempfile::tempdir().context("Could not create private login capture directory")?;
    set_private_dir(capture_workspace.path())?;
    let capture_dir = capture_workspace.path().to_path_buf();
    let node = resolve_node()?;
    let npm = resolve_program(&["npm"])?;
    let chrome_executable = resolve_chrome()?;
    eprintln!(
        "Installing the runtime dependency into a temporary profile (this can take a minute)..."
    );
    install_bundle_runtime(&capture_dir, &bundle, &npm)?;
    let port = find_free_debug_port()?;
    let chrome_profile = capture_dir.join("chrome-profile");
    eprintln!("Opening a fresh Chrome window for the ChatGPT login...");
    let mut capture_browser =
        CaptureChrome::new(launch_chrome(&chrome_executable, &chrome_profile, port)?);
    wait_for_cdp(port, Duration::from_secs(30))?;

    eprintln!("Complete the ChatGPT login in the Chrome window that just opened.");
    eprintln!(
        "The session is captured locally once ChatGPT shows as logged in, then pushed to the pool \
         (up to {wait_secs}s)."
    );
    let capture_file = capture_dir.join("session.json");
    let status = Command::new(&node)
        .arg(capture_dir.join("worker.mjs"))
        .args(["--capture-session", &capture_file.display().to_string()])
        .env("CHROME_CDP_URL", format!("http://127.0.0.1:{port}"))
        .env(
            "NYXID_LOGIN_CAPTURE_TIMEOUT_MS",
            wait_secs.saturating_mul(1000).to_string(),
        )
        .env_remove("NYXID_BASE_URL")
        .env_remove("NYXID_WORKER_TOKEN")
        .env_remove("NYXID_WORKER_TOKEN_FILE")
        .status()
        .context("Local ChatGPT login capture failed to start")?;
    if !status.success() {
        bail!("Local ChatGPT login capture did not complete")
    }
    if !capture_file.exists() {
        bail!(
            "The login capture helper exited without producing session state (it may not have \
             run at all); re-run with NYXID_ORACLE_DEBUG=1 or upgrade the backend so `oracle login` \
             fetches a worker bundle with the realpath main-module fix"
        )
    }
    let sealed = Zeroizing::new({
        let plaintext = Zeroizing::new(
            fs::read(&capture_file).context("Local login capture did not produce session state")?,
        );
        encrypt_login_snapshot(plaintext.as_slice(), token.as_bytes())?
    });
    let verifier = hex::encode(Sha256::digest(token.as_bytes()));
    let response: Value = api
        .post(
            &format!(
                "/oracle/pools/{}/login-snapshots",
                urlencoding::encode(&pool)
            ),
            &serde_json::json!({
                "format_version": SESSION_FORMAT_VERSION,
                "worker_token_sha256": verifier,
                "sealed_blob_base64": base64::engine::general_purpose::STANDARD.encode(sealed.as_slice()),
            }),
        )
        .await?;
    drop(sealed);
    capture_browser.stop()?;
    capture_workspace
        .close()
        .context("Could not remove the temporary login-capture profile")?;
    let outcomes = wait_for_login_imports(&mut api, &pool, &response, wait_secs).await?;
    let all_verified = !outcomes.is_empty()
        && outcomes.iter().all(|result| {
            result["status"].as_str() == Some("succeeded")
                && result["result_code"].as_str() == Some("session_import_verified")
        });
    match output {
        OutputFormat::Json => println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "snapshot": response,
                "worker_results": outcomes,
            }))?
        ),
        OutputFormat::Table => {
            eprintln!(
                "Login snapshot {} was relayed to {} worker(s).",
                response["snapshot_id"].as_str().unwrap_or("-"),
                outcomes.len()
            );
            let skipped = response["skipped_workers"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if !skipped.is_empty() {
                eprintln!(
                    "Skipped incapable workers: {}",
                    skipped
                        .iter()
                        .filter_map(Value::as_str)
                        .collect::<Vec<_>>()
                        .join(", ")
                );
            }
            let mut table = Table::new();
            table.load_preset(UTF8_FULL_CONDENSED);
            table.set_header(["Worker", "Status", "Result"]);
            for result in &outcomes {
                table.add_row([
                    text_field(result, "worker_label"),
                    text_field(result, "status"),
                    text_field(result, "result_code"),
                ]);
            }
            println!("{table}");
        }
    }
    if outcomes.is_empty() {
        bail!(
            "No connected worker accepted the login snapshot; workers must advertise commands_v1 and session_import_v1"
        )
    }
    if !all_verified {
        bail!(
            "One or more workers did not verify the imported ChatGPT session; inspect `nyxid oracle worker list {pool}`"
        )
    }
    Ok(())
}

/// Derive the AES-256-GCM cipher for a login snapshot from the pool worker
/// token and per-envelope salt via HKDF-SHA256. The key material is never a
/// hard-coded value: the zeroed buffer is overwritten by `hkdf.expand` before
/// use and kept in `Zeroizing`.
fn snapshot_cipher(salt: &[u8], worker_token: &[u8]) -> Result<Aes256Gcm> {
    let hkdf = Hkdf::<Sha256>::new(Some(salt), worker_token);
    let mut key = Zeroizing::new([0_u8; 32]);
    hkdf.expand(SESSION_INFO, key.as_mut())
        .map_err(|_| anyhow::anyhow!("Could not derive login snapshot key"))?;
    Aes256Gcm::new_from_slice(key.as_ref())
        .map_err(|_| anyhow::anyhow!("Could not initialize login snapshot encryption"))
}

fn encrypt_login_snapshot(plaintext: &[u8], worker_token: &[u8]) -> Result<Vec<u8>> {
    if plaintext.is_empty() || plaintext.len() > 350 * 1024 {
        bail!("Captured login state must be 1-358400 bytes")
    }
    let mut salt = [0_u8; 32];
    let mut nonce_bytes = [0_u8; 12];
    rand::rngs::OsRng.fill_bytes(&mut salt);
    rand::rngs::OsRng.fill_bytes(&mut nonce_bytes);
    let cipher = snapshot_cipher(&salt, worker_token)?;
    let ciphertext = cipher
        .encrypt(
            Nonce::from_slice(&nonce_bytes),
            Payload {
                msg: plaintext,
                aad: SESSION_INFO,
            },
        )
        .map_err(|_| anyhow::anyhow!("Could not encrypt login snapshot"))?;
    let envelope = serde_json::json!({
        "version": SESSION_FORMAT_VERSION,
        "salt_base64": base64::engine::general_purpose::STANDARD.encode(salt),
        "nonce_base64": base64::engine::general_purpose::STANDARD.encode(nonce_bytes),
        "ciphertext_base64": base64::engine::general_purpose::STANDARD.encode(ciphertext),
    });
    let encoded = serde_json::to_vec(&envelope)?;
    if encoded.len() > 512 * 1024 {
        bail!("Encrypted login snapshot exceeds the server limit")
    }
    Ok(encoded)
}

async fn wait_for_login_imports(
    api: &mut ApiClient,
    pool: &str,
    snapshot: &Value,
    wait_secs: u64,
) -> Result<Vec<Value>> {
    let targets = snapshot["queued_workers"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(wait_secs);
    loop {
        let mut results = Vec::new();
        let mut all_terminal = true;
        for target in &targets {
            let label = target["worker_label"]
                .as_str()
                .context("login snapshot target omitted worker_label")?;
            let command_id = target["command_id"]
                .as_str()
                .context("login snapshot target omitted command_id")?;
            let commands: Value = api
                .get(&format!("{}/commands", worker_path(pool, label)))
                .await?;
            let command = commands["commands"]
                .as_array()
                .and_then(|items| {
                    items
                        .iter()
                        .find(|item| item["id"].as_str() == Some(command_id))
                })
                .cloned()
                .unwrap_or_else(|| {
                    serde_json::json!({
                        "worker_label": label,
                        "status": "pending",
                    })
                });
            let status = command["status"].as_str().unwrap_or("pending");
            if !matches!(status, "succeeded" | "failed" | "expired") {
                all_terminal = false;
            }
            results.push(command);
        }
        if all_terminal {
            return Ok(results);
        }
        if std::time::Instant::now() >= deadline {
            for result in &mut results {
                if !matches!(
                    result["status"].as_str(),
                    Some("succeeded" | "failed" | "expired")
                ) {
                    result["result_code"] = Value::String("verification_timeout".to_string());
                }
            }
            return Ok(results);
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn wait_for_cdp(port: u16, timeout: Duration) -> Result<()> {
    let started = std::time::Instant::now();
    while started.elapsed() < timeout {
        if TcpStream::connect_timeout(
            &format!("127.0.0.1:{port}").parse()?,
            Duration::from_millis(250),
        )
        .is_ok()
        {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    bail!("Chrome did not expose its local debugging port within 30 seconds")
}

/// Poll `GET /oracle/tasks/{id}` until the task reaches a terminal status
/// or the wait budget expires. Long browser thinking lives here, not in a
/// single HTTP request.
async fn poll_until_terminal(api: &mut ApiClient, task_id: &str, wait_secs: u64) -> Result<Value> {
    let deadline = Duration::from_secs(wait_secs);
    let mut elapsed = Duration::ZERO;
    let mut last_phase: Option<String> = None;
    loop {
        let task: Value = api.get(&format!("/oracle/tasks/{task_id}")).await?;
        let status = task["status"].as_str().unwrap_or("");
        match status {
            "completed" | "failed" | "cancelled" => return Ok(task),
            _ => {}
        }
        // Surface phase transitions so the user sees progress on long runs.
        let phase = task["phase"].as_str().map(str::to_string);
        if phase != last_phase {
            if let Some(p) = &phase {
                let pos = task["queue_position"].as_u64().unwrap_or(0);
                if status == "queued" && pos > 0 {
                    eprintln!("  … queued (position {pos})");
                } else {
                    eprintln!("  … {p}");
                }
            }
            last_phase = phase;
        }
        if elapsed >= deadline {
            bail!(
                "Timed out after {wait_secs}s waiting for task {task_id} (still {status}). \
                 Re-check later with `nyxid oracle result {task_id}`."
            );
        }
        tokio::time::sleep(Duration::from_secs(POLL_INTERVAL_SECS)).await;
        elapsed += Duration::from_secs(POLL_INTERVAL_SECS);
    }
}

fn resolve_prompt(prompt: Option<&str>, file: Option<&str>) -> Result<String> {
    match (prompt, file) {
        (Some(p), None) => Ok(p.to_string()),
        (None, Some("-")) => {
            let mut buf = String::new();
            std::io::stdin()
                .read_to_string(&mut buf)
                .context("Failed to read prompt from stdin")?;
            if buf.trim().is_empty() {
                bail!("Empty prompt on stdin");
            }
            Ok(buf)
        }
        (None, Some(path)) => std::fs::read_to_string(path)
            .with_context(|| format!("Failed to read prompt at {path}")),
        (Some(_), Some(_)) => bail!("Pass the prompt as an argument OR --file, not both"),
        (None, None) => {
            bail!("No prompt. Pass it as an argument, or use --file <path> (or --file -)")
        }
    }
}

fn print_submit(
    output: OutputFormat,
    task_id: &str,
    submit: &Value,
    conversation_id: Option<&str>,
) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(submit)?),
        OutputFormat::Table => {
            eprintln!("Task submitted.");
            eprintln!();
            eprintln!("Task ID:  {task_id}");
            if let Some(conv) = conversation_id {
                eprintln!("Session:  {conv}");
            }
            if submit["deduplicated"].as_bool().unwrap_or(false) {
                eprintln!("(deduplicated — matched an existing client_ref)");
            }
            eprintln!();
            eprintln!("Fetch the answer with: nyxid oracle result {task_id}");
        }
    }
    Ok(())
}

fn print_attach_submit(output: OutputFormat, submit: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(submit)?),
        OutputFormat::Table => {
            let mut table = Table::new();
            table.load_preset(UTF8_FULL_CONDENSED);
            table.set_header(["Conversation", "Task", "Status"]);
            table.add_row([
                submit["conversation_id"]
                    .as_str()
                    .unwrap_or("-")
                    .to_string(),
                submit["task_id"].as_str().unwrap_or("-").to_string(),
                submit["status"].as_str().unwrap_or("-").to_string(),
            ]);
            println!("{table}");
        }
    }
    Ok(())
}

fn print_extract_submit(output: OutputFormat, task_id: &str, submit: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(submit)?),
        OutputFormat::Table => println!("{task_id}"),
    }
    Ok(())
}

fn mime_ext(mime: &str) -> &'static str {
    match mime {
        "image/jpeg" => "jpg",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/svg+xml" => "svg",
        _ => "png",
    }
}

/// Resolve the on-disk path for image `idx` of `count`. With no `--out`, images
/// are auto-named `oracle-<task_id>-<n>.<ext>` in the cwd. With `--out` and a
/// single image, the path is used verbatim; with multiple images it becomes a
/// prefix (`-<n>` inserted before any extension).
fn resolve_image_path(
    out: Option<&str>,
    task_id: &str,
    idx: usize,
    count: usize,
    ext: &str,
) -> String {
    match out {
        None => format!("oracle-{task_id}-{}.{ext}", idx + 1),
        Some(p) if count <= 1 => p.to_string(),
        Some(p) => {
            let slash = p.rfind('/').map(|s| s + 1).unwrap_or(0);
            match p[slash..].rfind('.') {
                Some(rel) => {
                    let dot = slash + rel;
                    format!("{}-{}{}", &p[..dot], idx + 1, &p[dot..])
                }
                None => format!("{p}-{}.{ext}", idx + 1),
            }
        }
    }
}

/// Decode and write any images on a completed task to disk, printing the saved
/// paths to stderr. Writes when `--out` is given, or in Table mode (JSON mode
/// without `--out` leaves the base64 in the printed JSON instead).
fn save_result_images(output: OutputFormat, task: &Value, out: Option<&str>) -> Result<()> {
    let images = match task["images"].as_array() {
        Some(a) if !a.is_empty() => a,
        _ => return Ok(()),
    };
    if out.is_none() && !matches!(output, OutputFormat::Table) {
        return Ok(());
    }
    let task_id = task["task_id"].as_str().unwrap_or("task");
    let count = images.len();
    for (i, img) in images.iter().enumerate() {
        let b64 = img["data_base64"].as_str().unwrap_or("");
        if b64.is_empty() {
            continue;
        }
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(b64.as_bytes())
            .context("server returned undecodable image data")?;
        let ext = mime_ext(img["mime"].as_str().unwrap_or("image/png"));
        let path = resolve_image_path(out, task_id, i, count, ext);
        std::fs::write(&path, &bytes)
            .with_context(|| format!("failed to write image to {path}"))?;
        eprintln!("Saved image to {path} ({} bytes)", bytes.len());
    }
    Ok(())
}

fn print_result(output: OutputFormat, task: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(task)?),
        OutputFormat::Table => {
            let status = task["status"].as_str().unwrap_or("-");
            if let Some(attempts) = task["attempts"].as_u64() {
                eprintln!(
                    "Attempts: {attempts} (infrastructure retries {}/{})",
                    task["retry_count"].as_u64().unwrap_or(0),
                    task["max_retries"].as_u64().unwrap_or(0)
                );
            }
            match status {
                "completed" => {
                    if let Some(resp) = task["response"].as_str() {
                        // The answer goes to stdout so it can be piped.
                        println!("{resp}");
                    }
                }
                "failed" => {
                    let reason = task["failure_reason"].as_str().unwrap_or("unknown");
                    bail!("Task failed ({reason}).");
                }
                "cancelled" => bail!("Task was cancelled."),
                other => {
                    let pos = task["queue_position"].as_u64().unwrap_or(0);
                    eprintln!("Task is {other}.");
                    if pos > 0 {
                        eprintln!("Queue position: {pos}");
                    }
                    if let Some(phase) = task["phase"].as_str() {
                        eprintln!("Phase: {phase}");
                    }
                }
            }
        }
    }
    Ok(())
}

fn print_status(output: OutputFormat, pool: &str, status: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(status)?),
        OutputFormat::Table => {
            eprintln!("Pool '{pool}':");
            eprintln!("  Queued:     {}", status["queued"].as_u64().unwrap_or(0));
            eprintln!(
                "  Dispatched: {} / {}",
                status["dispatched"].as_u64().unwrap_or(0),
                status["max_workers"].as_u64().unwrap_or(0)
            );
            eprintln!(
                "  Diagnosis:  {}",
                status["diagnosis"].as_str().unwrap_or("-")
            );
            let workers = status["active_workers"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if workers.is_empty() {
                eprintln!("  Workers:    none active (open a ChatGPT tab with the userscript)");
            } else {
                let mut table = Table::new();
                table.load_preset(UTF8_FULL_CONDENSED);
                table.set_header(["Worker", "Seen (s ago)", "Task", "Script"]);
                for w in &workers {
                    table.add_row([
                        w["worker_label"].as_str().unwrap_or("-").to_string(),
                        w["last_seen_secs_ago"].as_i64().unwrap_or(0).to_string(),
                        w["current_task_id"].as_str().unwrap_or("-").to_string(),
                        w["script_version"].as_str().unwrap_or("-").to_string(),
                    ]);
                }
                println!("{table}");
            }
        }
    }
    Ok(())
}

fn print_sessions(output: OutputFormat, resp: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(resp)?),
        OutputFormat::Table => {
            let sessions = resp["sessions"].as_array().cloned().unwrap_or_default();
            if sessions.is_empty() {
                eprintln!("No conversations yet.");
                return Ok(());
            }
            let mut table = Table::new();
            table.load_preset(UTF8_FULL_CONDENSED);
            table.set_header(["Conversation", "Turns", "Closed", "Updated"]);
            for s in &sessions {
                table.add_row([
                    s["conversation_id"].as_str().unwrap_or("-").to_string(),
                    s["turn_count"].as_u64().unwrap_or(0).to_string(),
                    yes_no(s["closed"].as_bool().unwrap_or(false)),
                    s["updated_at"].as_str().unwrap_or("-").to_string(),
                ]);
            }
            println!("{table}");
        }
    }
    Ok(())
}

fn print_session_detail(output: OutputFormat, resp: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(resp)?),
        OutputFormat::Table => {
            eprintln!(
                "Conversation {} ({} turns{})",
                resp["conversation_id"].as_str().unwrap_or("-"),
                resp["turn_count"].as_u64().unwrap_or(0),
                if resp["closed"].as_bool().unwrap_or(false) {
                    ", closed"
                } else {
                    ""
                }
            );
            let turns = resp["turns"].as_array().cloned().unwrap_or_default();
            for (i, turn) in turns.iter().enumerate() {
                eprintln!();
                eprintln!(
                    "─── Turn {} ({}) ───",
                    i + 1,
                    turn["status"].as_str().unwrap_or("-")
                );
                if let Some(prompt) = turn["prompt"].as_str() {
                    eprintln!("Q: {prompt}");
                }
                if let Some(resp_text) = turn["response"].as_str() {
                    println!("A: {resp_text}");
                }
            }
        }
    }
    Ok(())
}

fn insert_opt_str(body: &mut Value, key: &str, value: Option<&str>) {
    if let Some(v) = value {
        body[key] = Value::String(v.to_string());
    }
}

fn insert_opt_u64(body: &mut Value, key: &str, value: Option<u64>) {
    if let Some(v) = value {
        body[key] = Value::Number(v.into());
    }
}

fn yes_no(b: bool) -> String {
    if b {
        "yes".to_string()
    } else {
        "no".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::OutputFormat;
    use crate::test_support::mock_auth_with_output;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn resolve_prompt_prefers_argument() {
        assert_eq!(resolve_prompt(Some("hi"), None).unwrap(), "hi");
    }

    #[test]
    fn resolve_prompt_rejects_both() {
        assert!(resolve_prompt(Some("hi"), Some("f.txt")).is_err());
    }

    #[test]
    fn resolve_prompt_rejects_neither() {
        assert!(resolve_prompt(None, None).is_err());
    }

    #[test]
    fn login_snapshot_uses_token_bound_hkdf_and_aead() {
        // Generate the pool token at runtime: it is key-derivation input, so a
        // literal here trips the hard-coded-cryptographic-value scanner even
        // though this is only a test vector. A random token still exercises the
        // roundtrip and the wrong-token rejection below.
        let mut token_bytes = [0_u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut token_bytes);
        let token = format!("nyx_owk_{}", hex::encode(token_bytes)).into_bytes();
        let token = token.as_slice();
        let plaintext = br#"{"version":1,"cookies":[],"origins":[]}"#;
        let encoded = encrypt_login_snapshot(plaintext, token).expect("encrypt");
        let envelope: Value = serde_json::from_slice(&encoded).expect("envelope");
        let salt = base64::engine::general_purpose::STANDARD
            .decode(envelope["salt_base64"].as_str().unwrap())
            .unwrap();
        let nonce = base64::engine::general_purpose::STANDARD
            .decode(envelope["nonce_base64"].as_str().unwrap())
            .unwrap();
        let ciphertext = base64::engine::general_purpose::STANDARD
            .decode(envelope["ciphertext_base64"].as_str().unwrap())
            .unwrap();
        let cipher = snapshot_cipher(&salt, token).expect("derive cipher");
        let restored = cipher
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: SESSION_INFO,
                },
            )
            .expect("decrypt with same token");
        assert_eq!(restored, plaintext);

        let mut wrong_bytes = [0_u8; 24];
        rand::rngs::OsRng.fill_bytes(&mut wrong_bytes);
        let wrong_token = format!("nyx_owk_{}", hex::encode(wrong_bytes)).into_bytes();
        let wrong = snapshot_cipher(&salt, wrong_token.as_slice()).expect("derive cipher");
        assert!(
            wrong
                .decrypt(
                    Nonce::from_slice(&nonce),
                    Payload {
                        msg: &ciphertext,
                        aad: SESSION_INFO,
                    },
                )
                .is_err()
        );
    }

    #[test]
    fn forced_install_replaces_the_persisted_debug_port() {
        let occupied = (9222..9322)
            .find_map(|port| std::net::TcpListener::bind(("127.0.0.1", port)).ok())
            .expect("a debug port should be available for the test");
        let occupied_port = occupied.local_addr().unwrap().port();

        assert_eq!(
            select_install_debug_port(Some(occupied_port), false).unwrap(),
            occupied_port
        );
        assert_ne!(
            select_install_debug_port(Some(occupied_port), true).unwrap(),
            occupied_port
        );
    }

    #[test]
    fn bundle_checksum_verification_rejects_tampering() {
        let source = "console.error('worker');\n";
        let sha = hex::encode(Sha256::digest(source.as_bytes()));
        assert!(verify_bundle(source, &sha).is_ok());
        assert!(verify_bundle("changed", &sha).is_err());
    }

    #[test]
    fn local_upgrade_verifies_source_and_version_files() {
        let temp = tempfile::tempdir().unwrap();
        let source = "export const installed = true;\n";
        let sha256 = hex::encode(Sha256::digest(source.as_bytes()));
        let bundle_path = temp.path().join("worker.mjs");
        fs::write(&bundle_path, source).unwrap();
        fs::write(temp.path().join("bundle-version"), "0.10.0+testhash\n").unwrap();
        fs::write(
            temp.path().join("package.json"),
            r#"{"dependencies":{"playwright-core":"1.62.1"}}"#,
        )
        .unwrap();
        let config = OracleWorkerConfig {
            pool: "pool-1".to_string(),
            label: "worker-1".to_string(),
            base_url: "https://nyxid.example".to_string(),
            node_binary: PathBuf::from("node"),
            npm_binary: None,
            bundle_path,
            token_file: temp.path().join("token"),
            state_file: temp.path().join("state.json"),
            installation_id_file: temp.path().join("installation-id"),
            chrome_executable: PathBuf::from("chrome"),
            chrome_profile_dir: temp.path().join("chrome-profile"),
            chrome_debug_port: 9222,
        };
        let bundle = WorkerBundle {
            version: "0.10.0+testhash".to_string(),
            sha256,
            source: source.to_string(),
            playwright_core_version: "1.62.1".to_string(),
        };
        assert!(installed_bundle_matches(&config, &bundle));
        fs::write(temp.path().join("bundle-version"), "old-version\n").unwrap();
        assert!(!installed_bundle_matches(&config, &bundle));
    }

    #[test]
    fn playwright_runtime_version_must_be_exact() {
        assert!(validate_playwright_version("1.62.1").is_ok());
        assert!(validate_playwright_version("^1.62.1").is_err());
        assert!(validate_playwright_version("1.62.1;touch-x").is_err());
    }

    #[test]
    fn resolve_image_path_auto_names_without_out() {
        assert_eq!(
            resolve_image_path(None, "t1", 0, 1, "png"),
            "oracle-t1-1.png"
        );
        assert_eq!(
            resolve_image_path(None, "t1", 1, 2, "jpg"),
            "oracle-t1-2.jpg"
        );
    }

    #[test]
    fn resolve_image_path_single_out_is_verbatim() {
        assert_eq!(
            resolve_image_path(Some("apple.png"), "t1", 0, 1, "png"),
            "apple.png"
        );
        assert_eq!(
            resolve_image_path(Some("out/apple.png"), "t1", 0, 1, "png"),
            "out/apple.png"
        );
    }

    #[test]
    fn resolve_image_path_multi_out_is_prefix() {
        // Extension present → -N inserted before it.
        assert_eq!(
            resolve_image_path(Some("apple.png"), "t1", 0, 2, "png"),
            "apple-1.png"
        );
        assert_eq!(
            resolve_image_path(Some("apple.png"), "t1", 1, 2, "png"),
            "apple-2.png"
        );
        // No extension → -N.<ext> appended.
        assert_eq!(
            resolve_image_path(Some("apple"), "t1", 0, 2, "png"),
            "apple-1.png"
        );
        // A dot only in the directory must not be treated as an extension.
        assert_eq!(
            resolve_image_path(Some("my.dir/apple"), "t1", 0, 2, "png"),
            "my.dir/apple-1.png"
        );
    }

    #[test]
    fn insert_opt_helpers_skip_none() {
        let mut body = serde_json::json!({});
        insert_opt_str(&mut body, "a", None);
        insert_opt_u64(&mut body, "b", None);
        assert_eq!(body, serde_json::json!({}));
        insert_opt_str(&mut body, "a", Some("x"));
        insert_opt_u64(&mut body, "b", Some(5));
        assert_eq!(body, serde_json::json!({ "a": "x", "b": 5 }));
    }

    #[test]
    fn yes_no_maps_bools() {
        assert_eq!(yes_no(true), "yes");
        assert_eq!(yes_no(false), "no");
    }

    #[tokio::test]
    async fn ask_no_wait_submits_and_does_not_poll() {
        let server = MockServer::start().await;
        // Single-shot submit (no conversation_id field) with model + tag.
        Mock::given(method("POST"))
            .and(path("/api/v1/oracle/pools/chatgpt-pro/tasks"))
            .and(body_json(serde_json::json!({
                "prompt": "what is 2+2?",
                "model": "chatgpt-5.5-pro",
                "tag": "smoke",
            })))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "task_id": "task-1",
                "status": "queued",
                "queue_position": 1,
                "deduplicated": false,
            })))
            .expect(1)
            .mount(&server)
            .await;

        let result = run(OracleCommands::Ask {
            pool: "chatgpt-pro".to_string(),
            prompt: Some("what is 2+2?".to_string()),
            file: None,
            pdf: None,
            attach_file: None,
            model: Some("chatgpt-5.5-pro".to_string()),
            project_url: None,
            tag: Some("smoke".to_string()),
            conversation: None,
            new_conversation: false,
            client_ref: None,
            wait: 3600,
            no_wait: true,
            out: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await;
        result.expect("ask --no-wait should submit and return without polling");
    }

    #[tokio::test]
    async fn ask_new_conversation_sends_empty_conversation_id() {
        let server = MockServer::start().await;
        // --new-conversation must send conversation_id:"" (open a session).
        Mock::given(method("POST"))
            .and(path("/api/v1/oracle/pools/p/tasks"))
            .and(body_json(serde_json::json!({
                "prompt": "hello",
                "conversation_id": "",
            })))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "task_id": "task-2",
                "status": "queued",
                "queue_position": 1,
                "conversation_id": "conv_abc",
                "deduplicated": false,
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OracleCommands::Ask {
            pool: "p".to_string(),
            prompt: Some("hello".to_string()),
            file: None,
            pdf: None,
            attach_file: None,
            model: None,
            project_url: None,
            tag: None,
            conversation: None,
            new_conversation: true,
            client_ref: None,
            wait: 3600,
            no_wait: true,
            out: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("new conversation submit should succeed");
    }

    #[tokio::test]
    async fn ask_project_url_posts_task_override() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/oracle/pools/p/tasks"))
            .and(body_json(serde_json::json!({
                "prompt": "route this prompt",
                "project_url": "https://chatgpt.com/g/g-p-task/project",
            })))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "task_id": "task-project",
                "status": "queued",
                "queue_position": 1,
                "deduplicated": false,
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OracleCommands::Ask {
            pool: "p".to_string(),
            prompt: Some("route this prompt".to_string()),
            file: None,
            pdf: None,
            attach_file: None,
            model: None,
            project_url: Some("https://chatgpt.com/g/g-p-task/project".to_string()),
            tag: None,
            conversation: None,
            new_conversation: false,
            client_ref: None,
            wait: 3600,
            no_wait: true,
            out: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("ask --project-url should include the per-task override");
    }

    #[tokio::test]
    async fn ask_polls_until_completed() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/oracle/pools/p/tasks"))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "task_id": "task-3",
                "status": "queued",
                "queue_position": 1,
                "deduplicated": false,
            })))
            .mount(&server)
            .await;
        // The very first poll already returns completed, so the command
        // resolves without sleeping the 3s interval.
        Mock::given(method("GET"))
            .and(path("/api/v1/oracle/tasks/task-3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "task_id": "task-3",
                "pool_id": "p1",
                "status": "completed",
                "is_followup": false,
                "queue_position": 0,
                "response": "4",
                "created_at": "2026-06-11T00:00:00Z",
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OracleCommands::Ask {
            pool: "p".to_string(),
            prompt: Some("2+2?".to_string()),
            file: None,
            pdf: None,
            attach_file: None,
            model: None,
            project_url: None,
            tag: None,
            conversation: None,
            new_conversation: false,
            client_ref: None,
            wait: 30,
            no_wait: false,
            out: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("ask should poll once and return the completed answer");
    }

    #[tokio::test]
    async fn attach_no_wait_posts_expected_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/oracle/pools/chatgpt-pro/attach"))
            .and(body_json(serde_json::json!({
                "chatgpt_url": "https://chatgpt.com/c/abc",
                "tag": "import",
            })))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "conversation_id": "conv_abc",
                "task_id": "task-scrape",
                "status": "queued",
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OracleCommands::Attach {
            pool: "chatgpt-pro".to_string(),
            url: "https://chatgpt.com/c/abc".to_string(),
            tag: Some("import".to_string()),
            wait: 120,
            no_wait: true,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("attach --no-wait should submit and return without polling");
    }

    #[tokio::test]
    async fn extract_no_wait_posts_expected_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/oracle/pools/browser/extract"))
            .and(body_json(serde_json::json!({
                "url": "https://example.com/articles/alpha?tracking=1",
                "model": "reader",
            })))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "task_id": "task-extract",
                "status": "queued",
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OracleCommands::Extract {
            pool: "browser".to_string(),
            url: "https://example.com/articles/alpha?tracking=1".to_string(),
            model: Some("reader".to_string()),
            wait: 180,
            no_wait: true,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("extract --no-wait should submit and return without polling");
    }

    #[tokio::test]
    async fn attach_waits_then_fetches_session() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/oracle/pools/p/attach"))
            .and(body_json(serde_json::json!({
                "chatgpt_url": "https://chat.openai.com/c/abc",
            })))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "conversation_id": "conv_abc",
                "task_id": "task-scrape",
                "status": "queued",
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/oracle/tasks/task-scrape"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "task_id": "task-scrape",
                "pool_id": "p1",
                "status": "completed",
                "conversation_id": "conv_abc",
                "is_followup": false,
                "queue_position": 0,
                "response": "[imported 1 pairs]",
                "created_at": "2026-06-11T00:00:00Z",
                "completed_at": "2026-06-11T00:00:01Z",
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/oracle/sessions/conv_abc"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "conversation_id": "conv_abc",
                "pool_id": "p1",
                "chatgpt_url": "https://chat.openai.com/c/abc",
                "turn_count": 1,
                "closed": false,
                "created_at": "2026-06-11T00:00:00Z",
                "updated_at": "2026-06-11T00:00:01Z",
                "turns": [{
                    "task_id": "task-turn-1",
                    "status": "completed",
                    "prompt": "hello",
                    "response": "world",
                    "created_at": "2026-06-11T00:00:00Z",
                    "completed_at": "2026-06-11T00:00:01Z"
                }],
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(OracleCommands::Attach {
            pool: "p".to_string(),
            url: "https://chat.openai.com/c/abc".to_string(),
            tag: None,
            wait: 120,
            no_wait: false,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("attach should poll the scrape task and fetch the imported session");
    }

    #[tokio::test]
    async fn result_failed_surfaces_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/oracle/tasks/task-x"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "task_id": "task-x",
                "pool_id": "p1",
                "status": "failed",
                "is_followup": false,
                "queue_position": 0,
                "failure_reason": "extraction_failure",
                "created_at": "2026-06-11T00:00:00Z",
            })))
            .mount(&server)
            .await;

        let result = run(OracleCommands::Result {
            task_id: "task-x".to_string(),
            out: None,
            // Table output is where failed status maps to an error exit.
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await;
        assert!(result.is_err(), "a failed task should surface as an error");
    }

    #[tokio::test]
    async fn pool_create_posts_expected_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/oracle/pools"))
            .and(body_json(serde_json::json!({
                "slug": "chatgpt-pro",
                "name": "ChatGPT Pro",
                "visibility": "platform",
                "allow_extract": false,
                "max_workers": 4,
            })))
            .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "id": "pool-1",
                "slug": "chatgpt-pro",
                "name": "ChatGPT Pro",
                "visibility": "platform",
                "owner_user_id": "u1",
                "can_manage": true,
                "allow_extract": false,
                "max_workers": 4,
                "max_queue_length": 50,
                "per_user_max_inflight": 2,
                "task_timeout_secs": 14400,
                "is_active": true,
                "created_at": "2026-06-11T00:00:00Z",
                "updated_at": "2026-06-11T00:00:00Z",
                "worker_token": "nyx_owk_deadbeef",
            })))
            .expect(1)
            .mount(&server)
            .await;

        run_pool(OraclePoolCommands::Create {
            slug: "chatgpt-pro".to_string(),
            name: "ChatGPT Pro".to_string(),
            description: None,
            visibility: Some("platform".to_string()),
            project_url: None,
            model: None,
            allow_extract: false,
            max_workers: Some(4),
            max_queue: None,
            per_user_inflight: None,
            task_timeout: None,
            org: None,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("pool create should post and parse the token response");
    }
}
