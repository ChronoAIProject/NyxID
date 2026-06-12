//! `nyxid oracle` — call ChatGPT Pro (and other browser oracles) through
//! NyxID.
//!
//! A pool is a capacity unit backed by logged-in browser tabs running the
//! NyxID oracle userscript. `oracle ask` submits a prompt and polls the
//! relay until the answer lands (long thinking lives in the poll loop, not
//! a single request). `oracle pool` manages pools and worker tokens.

use std::io::Read;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use base64::Engine;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{OracleCommands, OraclePoolCommands, OutputFormat};
use crate::org_resolver::resolve_org_id;

const POLL_INTERVAL_SECS: u64 = 3;

pub async fn run(command: OracleCommands) -> Result<()> {
    match command {
        OracleCommands::Ask {
            pool,
            prompt,
            file,
            pdf,
            model,
            project_url,
            tag,
            conversation,
            new_conversation,
            client_ref,
            wait,
            no_wait,
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
            print_result(output, &task)
        }
        OracleCommands::Result { task_id, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let task: Value = api.get(&format!("/oracle/tasks/{task_id}")).await?;
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

fn print_result(output: OutputFormat, task: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(task)?),
        OutputFormat::Table => {
            let status = task["status"].as_str().unwrap_or("-");
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
            model: Some("chatgpt-5.5-pro".to_string()),
            project_url: None,
            tag: Some("smoke".to_string()),
            conversation: None,
            new_conversation: false,
            client_ref: None,
            wait: 3600,
            no_wait: true,
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
            model: None,
            project_url: None,
            tag: None,
            conversation: None,
            new_conversation: true,
            client_ref: None,
            wait: 3600,
            no_wait: true,
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
            model: None,
            project_url: Some("https://chatgpt.com/g/g-p-task/project".to_string()),
            tag: None,
            conversation: None,
            new_conversation: false,
            client_ref: None,
            wait: 3600,
            no_wait: true,
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
            model: None,
            project_url: None,
            tag: None,
            conversation: None,
            new_conversation: false,
            client_ref: None,
            wait: 30,
            no_wait: false,
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
