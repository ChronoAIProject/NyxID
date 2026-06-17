use std::io::Read;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use reqwest::header;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::api::{ApiClient, CLI_USER_AGENT, build_cli_http_client};
use crate::cli::{ComputeCommands, ComputePoolCommands, ComputeWorkerCommands, OutputFormat};
use crate::org_resolver::resolve_org_id;

pub async fn run(command: ComputeCommands) -> Result<()> {
    match command {
        ComputeCommands::Submit {
            pool,
            model,
            kind,
            input,
            priority,
            client_ref,
            wait,
            no_wait,
            auth,
        } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let input = read_json_input(&input)?;
            let mut body = serde_json::json!({
                "kind": kind,
                "model": model,
                "input": input,
                "priority": priority,
            });
            insert_opt_str(&mut body, "client_ref", client_ref.as_deref());
            let submit: Value = api
                .post(&format!("/compute/pools/{pool}/tasks"), &body)
                .await?;
            let task_id = submit["task_id"]
                .as_str()
                .context("server did not return task_id")?
                .to_string();

            if no_wait {
                return print_submit(output, &submit);
            }

            eprintln!("Submitted compute task {task_id} to pool '{pool}'. Waiting...");
            let task = poll_until_terminal(&mut api, &task_id, wait).await?;
            print_task(output, &task)
        }
        ComputeCommands::Result { task_id, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let task: Value = api.get(&format!("/compute/tasks/{task_id}")).await?;
            print_task(output, &task)
        }
        ComputeCommands::Cancel { task_id, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let task: Value = api
                .post(&format!("/compute/tasks/{task_id}/cancel"), &Value::Null)
                .await?;
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&task)?),
                OutputFormat::Table => eprintln!("Cancelled compute task {task_id}."),
            }
            Ok(())
        }
        ComputeCommands::Status { pool, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let status: Value = api.get(&format!("/compute/pools/{pool}/status")).await?;
            print_status(output, &pool, &status)
        }
        ComputeCommands::Pool { command } => run_pool(command).await,
        ComputeCommands::Worker { command } => run_worker(command).await,
    }
}

async fn run_pool(command: ComputePoolCommands) -> Result<()> {
    match command {
        ComputePoolCommands::Create {
            slug,
            name,
            description,
            visibility,
            scheduling_policy,
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
            insert_opt_str(&mut body, "scheduling_policy", scheduling_policy.as_deref());
            insert_opt_str(&mut body, "target_org_id", target_org_id.as_deref());
            insert_opt_u64(&mut body, "max_workers", max_workers.map(u64::from));
            insert_opt_u64(&mut body, "max_queue_length", max_queue.map(u64::from));
            insert_opt_u64(
                &mut body,
                "per_user_max_inflight",
                per_user_inflight.map(u64::from),
            );
            insert_opt_u64(&mut body, "task_timeout_secs", task_timeout);

            let resp: Value = api.post("/compute/pools", &body).await?;
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resp)?),
                OutputFormat::Table => {
                    eprintln!(
                        "Compute pool '{}' created.",
                        resp["slug"].as_str().unwrap_or("-")
                    );
                    eprintln!();
                    eprintln!("Worker token (shown once):");
                    println!("{}", resp["worker_token"].as_str().unwrap_or("-"));
                    eprintln!();
                    eprintln!("Install this token only on trusted GPU/Mac worker hosts.");
                }
            }
            Ok(())
        }
        ComputePoolCommands::List { auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let resp: Value = api.get("/compute/pools").await?;
            let pools = resp["pools"].as_array().cloned().unwrap_or_default();
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resp)?),
                OutputFormat::Table => {
                    if pools.is_empty() {
                        eprintln!("No compute pools visible.");
                        return Ok(());
                    }
                    let mut table = Table::new();
                    table.load_preset(UTF8_FULL_CONDENSED);
                    table.set_header(["Slug", "Name", "Visibility", "Policy", "Workers", "Active"]);
                    for p in pools {
                        table.add_row([
                            p["slug"].as_str().unwrap_or("-").to_string(),
                            p["name"].as_str().unwrap_or("-").to_string(),
                            p["visibility"].as_str().unwrap_or("-").to_string(),
                            p["scheduling_policy"].as_str().unwrap_or("-").to_string(),
                            p["max_workers"].as_u64().unwrap_or(0).to_string(),
                            yes_no(p["is_active"].as_bool().unwrap_or(false)),
                        ]);
                    }
                    println!("{table}");
                }
            }
            Ok(())
        }
        ComputePoolCommands::Show { pool, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let p: Value = api.get(&format!("/compute/pools/{pool}")).await?;
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&p)?),
                OutputFormat::Table => {
                    eprintln!("Slug:        {}", p["slug"].as_str().unwrap_or("-"));
                    eprintln!("Name:        {}", p["name"].as_str().unwrap_or("-"));
                    eprintln!("Visibility:  {}", p["visibility"].as_str().unwrap_or("-"));
                    eprintln!(
                        "Policy:      {}",
                        p["scheduling_policy"].as_str().unwrap_or("-")
                    );
                    eprintln!(
                        "Active:      {}",
                        yes_no(p["is_active"].as_bool().unwrap_or(false))
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
                }
            }
            Ok(())
        }
        ComputePoolCommands::Update {
            pool,
            name,
            description,
            visibility,
            scheduling_policy,
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
            insert_opt_str(&mut body, "scheduling_policy", scheduling_policy.as_deref());
            insert_opt_u64(&mut body, "max_workers", max_workers.map(u64::from));
            insert_opt_u64(&mut body, "max_queue_length", max_queue.map(u64::from));
            insert_opt_u64(
                &mut body,
                "per_user_max_inflight",
                per_user_inflight.map(u64::from),
            );
            insert_opt_u64(&mut body, "task_timeout_secs", task_timeout);
            if let Some(active) = active {
                body["is_active"] = Value::Bool(active);
            }
            let p: Value = api.patch(&format!("/compute/pools/{pool}"), &body).await?;
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&p)?),
                OutputFormat::Table => {
                    eprintln!(
                        "Compute pool '{}' updated.",
                        p["slug"].as_str().unwrap_or(&pool)
                    )
                }
            }
            Ok(())
        }
        ComputePoolCommands::RotateToken { pool, auth } => {
            let output = auth.output;
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let resp: Value = api
                .post(&format!("/compute/pools/{pool}/rotate-token"), &Value::Null)
                .await?;
            match output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&resp)?),
                OutputFormat::Table => {
                    eprintln!(
                        "Worker token rotated for '{}'.",
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

async fn run_worker(command: ComputeWorkerCommands) -> Result<()> {
    match command {
        ComputeWorkerCommands::Run {
            worker,
            token,
            token_env,
            endpoint_url,
            backend_token_env,
            backend,
            models,
            gpu_name,
            host_kind,
            node_id,
            max_concurrency,
            poll_interval_secs,
            ack_interval_secs,
            request_timeout_secs,
            base,
        } => {
            let base_url = base.resolved_base_url()?.trim_end_matches('/').to_string();
            let worker_token = resolve_worker_token(token, &token_env)?;
            let backend_token = backend_token_env
                .as_deref()
                .and_then(|name| std::env::var(name).ok())
                .filter(|value| !value.trim().is_empty());
            let worker_client = build_cli_http_client(base.profile.as_deref())?;
            let local_client = reqwest::Client::builder()
                .user_agent(CLI_USER_AGENT)
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(request_timeout_secs))
                .build()
                .context("failed to build local backend HTTP client")?;
            let config = WorkerRunConfig {
                worker,
                base_url,
                worker_token,
                endpoint_url,
                backend_token,
                poll_interval: Duration::from_secs(poll_interval_secs.max(1)),
                ack_interval: Duration::from_secs(ack_interval_secs.max(1)),
                capabilities: WorkerCapabilitiesBody {
                    node_id,
                    host_kind,
                    gpu_name,
                    backend,
                    models,
                    max_concurrency,
                    current_inflight: Some(0),
                    worker_version: Some(format!("nyxid-cli/{}", env!("CARGO_PKG_VERSION"))),
                },
            };
            worker_loop(worker_client, local_client, config).await
        }
    }
}

#[derive(Clone)]
struct WorkerRunConfig {
    worker: String,
    base_url: String,
    worker_token: String,
    endpoint_url: String,
    backend_token: Option<String>,
    poll_interval: Duration,
    ack_interval: Duration,
    capabilities: WorkerCapabilitiesBody,
}

#[derive(Clone, Serialize)]
struct WorkerCapabilitiesBody {
    #[serde(skip_serializing_if = "Option::is_none")]
    node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    host_kind: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    gpu_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    backend: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    models: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_concurrency: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    current_inflight: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    worker_version: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum WorkerPollResponse {
    Idle,
    Task {
        task_id: String,
        kind: String,
        model: String,
        input: Value,
    },
}

#[derive(Deserialize)]
struct WorkerAckResponse {
    status: String,
}

async fn worker_loop(
    worker_client: reqwest::Client,
    local_client: reqwest::Client,
    config: WorkerRunConfig,
) -> Result<()> {
    eprintln!(
        "Compute worker '{}' polling {} and executing via {}",
        config.worker, config.base_url, config.endpoint_url
    );

    loop {
        match poll_worker_task(&worker_client, &config).await? {
            WorkerPollResponse::Idle => {
                tokio::time::sleep(config.poll_interval).await;
            }
            WorkerPollResponse::Task {
                task_id,
                kind,
                model,
                input,
            } => {
                let task = WorkerTask {
                    task_id,
                    kind,
                    model,
                    input,
                };
                eprintln!(
                    "Claimed compute task {} (kind={}, model={})",
                    task.task_id, task.kind, task.model
                );
                let outcome = execute_with_heartbeats(
                    worker_client.clone(),
                    local_client.clone(),
                    config.clone(),
                    task.clone(),
                )
                .await;
                match outcome {
                    Ok(WorkerTaskOutcome::Completed(output)) => {
                        submit_worker_result(
                            &worker_client,
                            &config,
                            &task.task_id,
                            Some(output),
                            None,
                        )
                        .await?;
                        eprintln!("Completed compute task {}", task.task_id);
                    }
                    Ok(WorkerTaskOutcome::Cancelled) => {
                        eprintln!(
                            "Compute task {} was cancelled; local request aborted",
                            task.task_id
                        );
                    }
                    Err(err) => {
                        let reason = format!("{err:#}");
                        submit_worker_result(
                            &worker_client,
                            &config,
                            &task.task_id,
                            None,
                            Some(&reason),
                        )
                        .await?;
                        eprintln!("Failed compute task {}: {}", task.task_id, reason);
                    }
                }
            }
        }
    }
}

#[derive(Clone)]
struct WorkerTask {
    task_id: String,
    kind: String,
    model: String,
    input: Value,
}

enum WorkerTaskOutcome {
    Completed(Value),
    Cancelled,
}

async fn execute_with_heartbeats(
    worker_client: reqwest::Client,
    local_client: reqwest::Client,
    config: WorkerRunConfig,
    task: WorkerTask,
) -> Result<WorkerTaskOutcome> {
    let request_config = config.clone();
    let request_task = task.clone();
    let handle = tokio::spawn(async move {
        execute_local_task(&local_client, &request_config, &request_task).await
    });
    tokio::pin!(handle);

    loop {
        tokio::select! {
            result = &mut handle => {
                let output = result.context("local backend request task panicked")??;
                return Ok(WorkerTaskOutcome::Completed(output));
            }
            _ = tokio::time::sleep(config.ack_interval) => {
                let ack = worker_ack(&worker_client, &config, &task.task_id, "running").await?;
                if ack.status == "cancelled" {
                    handle.abort();
                    let _ = (&mut handle).await;
                    return Ok(WorkerTaskOutcome::Cancelled);
                }
            }
        }
    }
}

async fn poll_worker_task(
    client: &reqwest::Client,
    config: &WorkerRunConfig,
) -> Result<WorkerPollResponse> {
    let url = format!(
        "{}/api/v1/compute/worker/task?worker={}",
        config.base_url,
        urlencoding::encode(&config.worker)
    );
    let body = serde_json::json!({
        "capabilities": config.capabilities,
    });
    send_worker_json(client.post(url), config, &body).await
}

async fn worker_ack(
    client: &reqwest::Client,
    config: &WorkerRunConfig,
    task_id: &str,
    phase: &str,
) -> Result<WorkerAckResponse> {
    let url = format!("{}/api/v1/compute/worker/ack", config.base_url);
    let mut capabilities = config.capabilities.clone();
    capabilities.current_inflight = Some(1);
    let body = serde_json::json!({
        "task_id": task_id,
        "worker": config.worker,
        "phase": phase,
        "capabilities": capabilities,
    });
    send_worker_json(client.post(url), config, &body).await
}

async fn submit_worker_result(
    client: &reqwest::Client,
    config: &WorkerRunConfig,
    task_id: &str,
    output: Option<Value>,
    failure_reason: Option<&str>,
) -> Result<()> {
    let url = format!("{}/api/v1/compute/worker/result", config.base_url);
    let body = serde_json::json!({
        "task_id": task_id,
        "worker": config.worker,
        "output": output,
        "failure_reason": failure_reason,
    });
    let _: Value = send_worker_json(client.post(url), config, &body).await?;
    Ok(())
}

async fn send_worker_json<T: for<'de> Deserialize<'de>>(
    request: reqwest::RequestBuilder,
    config: &WorkerRunConfig,
    body: &Value,
) -> Result<T> {
    let response = request
        .bearer_auth(&config.worker_token)
        .json(body)
        .send()
        .await
        .context("failed to call NyxID compute worker API")?;
    parse_json_response(response).await
}

async fn execute_local_task(
    client: &reqwest::Client,
    config: &WorkerRunConfig,
    task: &WorkerTask,
) -> Result<Value> {
    let mut body = task.input.clone();
    if let Some(obj) = body.as_object_mut() {
        obj.entry("model".to_string())
            .or_insert_with(|| Value::String(task.model.clone()));
    }

    let mut request = client.post(&config.endpoint_url).json(&body);
    if let Some(token) = &config.backend_token {
        request = request.header(header::AUTHORIZATION, format!("Bearer {token}"));
    }
    let response = request
        .send()
        .await
        .context("local compute backend request failed")?;
    parse_json_response(response).await
}

async fn parse_json_response<T: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
) -> Result<T> {
    let status = response.status();
    let text = response
        .text()
        .await
        .context("failed to read HTTP response")?;
    if !status.is_success() {
        let trimmed = text.chars().take(500).collect::<String>();
        anyhow::bail!("HTTP {status}: {trimmed}");
    }
    serde_json::from_str(&text).context("HTTP response was not valid JSON")
}

fn resolve_worker_token(token: Option<String>, token_env: &str) -> Result<String> {
    if let Some(token) = token
        && !token.trim().is_empty()
    {
        return Ok(token);
    }
    let token = std::env::var(token_env)
        .with_context(|| format!("missing worker token; pass --token or set {token_env}"))?;
    if token.trim().is_empty() {
        anyhow::bail!("worker token in {token_env} is empty");
    }
    Ok(token)
}

fn read_json_input(raw: &str) -> Result<Value> {
    let text = if raw == "-" {
        let mut input = String::new();
        std::io::stdin().read_to_string(&mut input)?;
        input
    } else if let Some(path) = raw.strip_prefix('@') {
        std::fs::read_to_string(path).with_context(|| format!("failed to read {path}"))?
    } else {
        raw.to_string()
    };
    serde_json::from_str(&text).context("compute --input must be valid JSON")
}

async fn poll_until_terminal(api: &mut ApiClient, task_id: &str, wait_secs: u64) -> Result<Value> {
    let start = Instant::now();
    loop {
        let task: Value = api.get(&format!("/compute/tasks/{task_id}")).await?;
        let status = task["status"].as_str().unwrap_or("");
        if matches!(status, "completed" | "failed" | "cancelled") {
            return Ok(task);
        }
        if start.elapsed() >= Duration::from_secs(wait_secs) {
            anyhow::bail!(
                "Timed out after {wait_secs}s waiting for compute task {task_id}; re-check with `nyxid compute result {task_id}`."
            );
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

fn print_submit(output: OutputFormat, submit: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(submit)?),
        OutputFormat::Table => {
            println!("{}", submit["task_id"].as_str().unwrap_or("-"));
        }
    }
    Ok(())
}

fn print_task(output: OutputFormat, task: &Value) -> Result<()> {
    match output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(task)?),
        OutputFormat::Table => match task["status"].as_str().unwrap_or("-") {
            "completed" => {
                if let Some(output) = task.get("output") {
                    println!("{}", serde_json::to_string_pretty(output)?);
                } else {
                    println!("null");
                }
            }
            "failed" => anyhow::bail!(
                "Compute task failed: {}",
                task["failure_reason"].as_str().unwrap_or("unknown")
            ),
            status => {
                eprintln!("Status: {status}");
                eprintln!("Task ID: {}", task["task_id"].as_str().unwrap_or("-"));
                eprintln!(
                    "Queue position: {}",
                    task["queue_position"].as_u64().unwrap_or(0)
                );
            }
        },
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
                "  Dispatched: {}",
                status["dispatched"].as_u64().unwrap_or(0)
            );
            eprintln!(
                "  Max workers: {}",
                status["max_workers"].as_u64().unwrap_or(0)
            );
            let workers = status["active_workers"]
                .as_array()
                .cloned()
                .unwrap_or_default();
            if workers.is_empty() {
                eprintln!("  Active workers: none");
                return Ok(());
            }
            let mut table = Table::new();
            table.load_preset(UTF8_FULL_CONDENSED);
            table.set_header(["Worker", "Backend", "GPU", "Models", "Task"]);
            for w in workers {
                let models = w["models"]
                    .as_array()
                    .map(|items| {
                        items
                            .iter()
                            .filter_map(|v| v.as_str())
                            .take(3)
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default();
                table.add_row([
                    w["worker_label"].as_str().unwrap_or("-").to_string(),
                    w["backend"].as_str().unwrap_or("-").to_string(),
                    w["gpu_name"].as_str().unwrap_or("-").to_string(),
                    models,
                    w["current_task_id"].as_str().unwrap_or("-").to_string(),
                ]);
            }
            println!("{table}");
        }
    }
    Ok(())
}

fn insert_opt_str(body: &mut Value, key: &str, value: Option<&str>) {
    if let Some(value) = value {
        body[key] = Value::String(value.to_string());
    }
}

fn insert_opt_u64(body: &mut Value, key: &str, value: Option<u64>) {
    if let Some(value) = value {
        body[key] = Value::Number(value.into());
    }
}

fn yes_no(value: bool) -> String {
    if value { "yes" } else { "no" }.to_string()
}
