//! Disposable, request-scoped Chrono tool-agent POC for Assistant Direct mode.

pub mod prompt;
pub mod sse_decode;
pub mod tools;

use std::convert::Infallible;
use std::future::Future;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{HeaderValue, Request, header};
use bytes::Bytes;
use futures::StreamExt;
use serde::Serialize;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::AppState;
use crate::downstream_disconnect::ClientConnectionCancellation;
use crate::handlers::proxy::execute_admin_proxy;
use crate::mw::auth::AuthUser;
use crate::services::assistant_direct::{self, DirectChatRequest};
use crate::services::billing::route_inventory::{BillingEgressPermit, BillingRoutePolicy};
use crate::services::{approval_service, audit_service, mcp_approval, mcp_service};

use prompt::AgentPhase;
use sse_decode::{ChronoHopDecoder, HopResult, ReassembledToolCall};
use tools::{ModelToolResult, ReadOnlyRegistry};

pub const MAX_LLM_CALLS: usize = 8;
pub const MAX_TOOL_CALLS: usize = 8;
pub const MAX_AGENT_CONTENT_BYTES: usize = 128 * 1024;
pub const MAX_UPSTREAM_BODY_BYTES: usize = 448 * 1024;
pub const WALL_DEADLINE: Duration = Duration::from_secs(180);
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(30);
const HOP_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const TOOL_TIMEOUT: Duration = Duration::from_secs(60);

pub type AgentSseSender = mpsc::Sender<Result<Bytes, Infallible>>;

#[derive(Serialize)]
#[serde(tag = "type")]
enum AgentFrame<'a> {
    #[serde(rename = "run.started")]
    RunStarted {
        run_id: &'a str,
        model: &'a str,
        skill_slug: Option<&'a str>,
        effort: Option<&'a str>,
        limits: RunLimits,
    },
    #[serde(rename = "stage")]
    Stage {
        stage: &'a str,
        status: &'a str,
        detail: &'a str,
    },
    #[serde(rename = "text.delta")]
    TextDelta { stage: &'a str, text: &'a str },
    #[serde(rename = "tool.started")]
    ToolStarted {
        call_id: &'a str,
        index: usize,
        tool: &'a str,
        target: ToolTarget<'a>,
    },
    #[serde(rename = "tool.completed")]
    ToolCompleted {
        call_id: &'a str,
        tool: &'a str,
        outcome: &'a str,
        status: u16,
        duration_ms: u64,
        result_bytes: usize,
        truncated: bool,
    },
    #[serde(rename = "error")]
    Error { code: &'a str, message: &'a str },
    #[serde(rename = "done")]
    Done {
        status: &'static str,
        finish_reason: &'a str,
        tool_calls: usize,
        llm_calls: usize,
        duration_ms: u64,
    },
}

#[derive(Serialize)]
struct RunLimits {
    max_tool_calls: usize,
    max_llm_calls: usize,
    deadline_ms: u64,
}

#[derive(Serialize)]
struct ToolTarget<'a> {
    service_slug: &'a str,
    endpoint: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunError {
    Cancelled,
    Deadline,
    ContextOverflow,
    Upstream,
    Internal,
}

#[derive(Clone, Copy)]
struct AgentRunBudget {
    max_llm_calls: usize,
    max_tool_calls: usize,
    wall_deadline: Duration,
    first_byte_timeout: Duration,
    hop_idle_timeout: Duration,
    tool_timeout: Duration,
}

impl Default for AgentRunBudget {
    fn default() -> Self {
        Self {
            max_llm_calls: MAX_LLM_CALLS,
            max_tool_calls: MAX_TOOL_CALLS,
            wall_deadline: WALL_DEADLINE,
            first_byte_timeout: FIRST_BYTE_TIMEOUT,
            hop_idle_timeout: HOP_IDLE_TIMEOUT,
            tool_timeout: TOOL_TIMEOUT,
        }
    }
}

#[cfg(test)]
#[derive(Clone, Default)]
struct TestDispatchObserver {
    upstream_dispatches: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    tool_dispatches: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

struct RunContext {
    state: AppState,
    auth_user: AuthUser,
    request: DirectChatRequest,
    chrono_service_id: String,
    billing_policy: BillingRoutePolicy,
    billing_egress_permit: BillingEgressPermit,
    connection_extension: Option<ClientConnectionCancellation>,
    cancellation: CancellationToken,
    tx: AgentSseSender,
    run_id: String,
    started_at: Instant,
    llm_calls: usize,
    tool_calls: usize,
    ornn_skill_fetched: bool,
    in_flight_tool: Option<InFlightTool>,
    budget: AgentRunBudget,
    #[cfg(test)]
    test_dispatch_observer: TestDispatchObserver,
}

#[derive(Clone)]
struct InFlightTool {
    public_identity: PublicToolIdentity,
    service_slug: String,
    endpoint: String,
    started: Instant,
}

#[derive(Clone)]
struct PublicToolIdentity {
    call_id: String,
    tool: &'static str,
}

struct ToolCompletion<'a> {
    public_identity: &'a PublicToolIdentity,
    outcome: &'a str,
    result: &'a ModelToolResult,
    started: Instant,
    service_slug: &'a str,
    endpoint: &'a str,
    skill_version: Option<&'a str>,
}

pub(crate) struct RunInputs {
    pub state: AppState,
    pub auth_user: AuthUser,
    pub request: DirectChatRequest,
    pub chrono_service_id: String,
    pub billing_policy: BillingRoutePolicy,
    pub billing_egress_permit: BillingEgressPermit,
    pub connection_extension: Option<ClientConnectionCancellation>,
    pub cancellation: CancellationToken,
    pub tx: AgentSseSender,
}

pub(crate) async fn run(inputs: RunInputs) {
    let mut run = RunContext {
        state: inputs.state,
        auth_user: inputs.auth_user,
        request: inputs.request,
        chrono_service_id: inputs.chrono_service_id,
        billing_policy: inputs.billing_policy,
        billing_egress_permit: inputs.billing_egress_permit,
        connection_extension: inputs.connection_extension,
        cancellation: inputs.cancellation,
        tx: inputs.tx,
        run_id: uuid::Uuid::new_v4().to_string(),
        started_at: Instant::now(),
        llm_calls: 0,
        tool_calls: 0,
        ornn_skill_fetched: false,
        in_flight_tool: None,
        budget: AgentRunBudget::default(),
        #[cfg(test)]
        test_dispatch_observer: TestDispatchObserver::default(),
    };
    let cancellation = run.cancellation.clone();
    let wall_deadline = run.budget.wall_deadline;
    let run_future = run.execute();
    let outcome = await_run_outcome(cancellation, wall_deadline, run_future).await;
    if let Err(error) = outcome {
        run.settle_run_error(error).await;
    }
}

async fn await_run_outcome<F>(
    cancellation: CancellationToken,
    wall_deadline: Duration,
    future: F,
) -> Result<(), RunError>
where
    F: Future<Output = Result<(), RunError>>,
{
    tokio::select! {
        biased;
        () = cancellation.cancelled() => Err(RunError::Cancelled),
        result = tokio::time::timeout(wall_deadline, future) => {
            result.unwrap_or(Err(RunError::Deadline))
        }
    }
}

impl RunContext {
    async fn execute(&mut self) -> Result<(), RunError> {
        let model = self
            .request
            .model
            .as_deref()
            .unwrap_or(assistant_direct::DEFAULT_DIRECT_MODEL);
        self.send_frame(&AgentFrame::RunStarted {
            run_id: &self.run_id,
            model,
            skill_slug: self.request.skill_slug.as_deref(),
            effort: self.request.effort.as_deref(),
            limits: RunLimits {
                max_tool_calls: self.budget.max_tool_calls,
                max_llm_calls: self.budget.max_llm_calls,
                deadline_ms: self.budget.wall_deadline.as_millis() as u64,
            },
        })
        .await?;
        self.audit(
            "assistant_agent_poc_run_started",
            serde_json::json!({
                "run_id": self.run_id,
                "model": model,
                "max_llm_calls": self.budget.max_llm_calls,
                "max_tool_calls": self.budget.max_tool_calls
            }),
        );

        self.stage("understand", "started", "Loading connected read operations")
            .await?;
        self.check_cancelled()?;
        let catalog = mcp_service::load_operation_catalog(
            &self.state.db,
            &self.state.node_ws_manager,
            &self.auth_user.user_id.to_string(),
            mcp_service::NodeScope::Unrestricted,
            mcp_service::ServiceScope::Unrestricted,
        )
        .await
        .map_err(|_| {
            tracing::warn!(
                run_id = %self.run_id,
                failure_class = "operation_catalog_load_failed",
                "assistant_agent_poc_registry_failed"
            );
            RunError::Internal
        })?;
        // The canonical operation catalog intentionally omits executable
        // services with no publishable endpoint rows. Ornn has that production
        // shape, so resolve its authentic connected UserService separately from
        // the pre-publication metadata loader and use it only for the two fixed
        // POC descriptors.
        let connected_services = mcp_service::load_user_tools_all_scoped(
            &self.state.db,
            &self.state.node_ws_manager,
            &self.auth_user.user_id.to_string(),
            mcp_service::NodeScope::Unrestricted,
        )
        .await
        .map_err(|_| {
            tracing::warn!(
                run_id = %self.run_id,
                failure_class = "connected_service_load_failed",
                "assistant_agent_poc_registry_failed"
            );
            RunError::Internal
        })?;
        let ornn_service = tools::resolve_ornn_service(&connected_services);
        let registry = ReadOnlyRegistry::new(&connected_services, &catalog.services, ornn_service);
        let detail = format!(
            "{} services · {} read operations",
            registry.service_count(),
            registry.operation_count()
        );
        self.stage("understand", "completed", &detail).await?;

        let mut messages = self.base_messages();
        self.stage("plan", "started", "Building a bounded read-only plan")
            .await?;
        let plan = self
            .chrono_hop(
                &messages,
                AgentPhase::Plan,
                ToolMode::Disabled,
                Some("plan"),
            )
            .await?;
        self.stage("plan", "completed", "Plan ready").await?;
        self.stage("execute", "started", "Executing eligible typed reads")
            .await?;

        match planning_transition(&plan) {
            PlanningTransition::Execute => {
                messages.push(serde_json::json!({"role":"assistant","content":plan.text}));
            }
            PlanningTransition::ExecuteBatch => {
                self.execute_tool_batch(&registry, &plan, &mut messages)
                    .await?;
            }
            PlanningTransition::Done => {
                self.stage("execute", "completed", "Planning ended the run")
                    .await?;
                self.stage("final", "started", "No final answer was produced")
                    .await?;
                self.finish_success(&plan.finish_reason).await?;
                return Ok(());
            }
        }

        let finish_reason = loop {
            self.check_cancelled()?;
            let force_final = self.should_force_final();
            if force_final {
                messages.push(serde_json::json!({
                    "role": "system",
                    "content": "The execution budget is exhausted. Produce the best grounded final answer now; tools are disabled."
                }));
                self.stage("execute", "completed", "Execution budget settled")
                    .await?;
                self.stage("final", "started", "Writing the grounded answer")
                    .await?;
                let final_hop = self
                    .chrono_hop(
                        &messages,
                        AgentPhase::Final,
                        ToolMode::Disabled,
                        Some("final"),
                    )
                    .await?;
                self.append_skipped_tool_messages(&mut messages, &final_hop.tool_calls);
                break final_hop.finish_reason;
            }

            let hop = self
                .chrono_hop(&messages, AgentPhase::Execute, ToolMode::Enabled, None)
                .await?;
            if hop.finish_reason != "tool_calls" || hop.tool_calls.is_empty() {
                self.stage("execute", "completed", "Read execution complete")
                    .await?;
                self.stage("final", "started", "Writing the grounded answer")
                    .await?;
                if !hop.text.is_empty() {
                    self.text_delta("final", &hop.text).await?;
                }
                self.append_skipped_tool_messages(&mut messages, &hop.tool_calls);
                break hop.finish_reason;
            }

            self.execute_tool_batch(&registry, &hop, &mut messages)
                .await?;
        };

        self.finish_success(&finish_reason).await?;
        Ok(())
    }

    async fn finish_success(&self, finish_reason: &str) -> Result<(), RunError> {
        self.stage("final", "completed", "Grounded answer complete")
            .await?;
        self.send_frame(&AgentFrame::Done {
            status: "completed",
            finish_reason,
            tool_calls: self.tool_calls,
            llm_calls: self.llm_calls,
            duration_ms: elapsed_ms(self.started_at),
        })
        .await?;
        self.send_done().await?;
        self.audit_finished("completed", Some(finish_reason));
        Ok(())
    }

    async fn execute_tool_batch(
        &mut self,
        registry: &ReadOnlyRegistry<'_>,
        hop: &HopResult,
        messages: &mut Vec<serde_json::Value>,
    ) -> Result<(), RunError> {
        append_assistant_tool_call_message(messages, hop);
        for call in &hop.tool_calls {
            let result = self.execute_call(registry, call).await?;
            append_tool_result_message(messages, call, &result);
        }
        Ok(())
    }

    fn should_force_final(&self) -> bool {
        self.llm_calls.saturating_add(1) >= self.budget.max_llm_calls
            || self.tool_calls >= self.budget.max_tool_calls
    }

    fn base_messages(&self) -> Vec<serde_json::Value> {
        self.request
            .messages
            .iter()
            .map(|message| serde_json::to_value(message).expect("direct message serializes"))
            .collect()
    }

    async fn chrono_hop(
        &mut self,
        messages: &[serde_json::Value],
        phase: AgentPhase,
        tool_mode: ToolMode,
        visible_stage: Option<&str>,
    ) -> Result<HopResult, RunError> {
        self.check_cancelled()?;
        if self.llm_calls >= self.budget.max_llm_calls {
            return Err(RunError::ContextOverflow);
        }
        self.llm_calls += 1;
        let body = build_hop_body(&self.request, messages, phase, tool_mode);
        let payload = serde_json::to_vec(&body).map_err(|_| RunError::Internal)?;
        if payload.len() > MAX_UPSTREAM_BODY_BYTES {
            return Err(RunError::ContextOverflow);
        }

        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/assistant/direct/agent")
            .header(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )
            .header(
                header::ACCEPT,
                HeaderValue::from_static("text/event-stream"),
            )
            .body(Body::from(payload))
            .map_err(|_| RunError::Internal)?;
        // This is the policy attached by the mounted route. It is copied,
        // never reconstructed or self-attested by the agent engine.
        request.extensions_mut().insert(self.billing_policy);
        if let Some(connection) = self.connection_extension.clone() {
            request.extensions_mut().insert(connection);
        }
        #[cfg(test)]
        self.test_dispatch_observer
            .upstream_dispatches
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let mut resolved_slug = String::new();
        let response = execute_admin_proxy(
            &self.state,
            &self.auth_user,
            &self.chrono_service_id,
            "chat/completions",
            request,
            Vec::new(),
            &mut resolved_slug,
        )
        .await
        .map_err(|_| {
            tracing::warn!(
                run_id = %self.run_id,
                hop = self.llm_calls,
                failure_class = "chrono_dispatch_failed",
                "assistant_agent_poc_hop_failed"
            );
            RunError::Upstream
        })?;
        if !response.status().is_success() {
            tracing::warn!(
                run_id = %self.run_id,
                hop = self.llm_calls,
                status = response.status().as_u16(),
                failure_class = "chrono_non_success_status",
                "assistant_agent_poc_hop_failed"
            );
            return Err(RunError::Upstream);
        }

        let mut stream = response.into_body().into_data_stream();
        let mut decoder = ChronoHopDecoder::default();
        let mut first = true;
        loop {
            self.check_cancelled()?;
            let timeout = if first {
                self.budget.first_byte_timeout
            } else {
                self.budget.hop_idle_timeout
            };
            let next = tokio::select! {
                biased;
                () = self.cancellation.cancelled() => return Err(RunError::Cancelled),
                result = tokio::time::timeout(timeout, stream.next()) => result,
            }
            .map_err(|_| {
                tracing::warn!(
                    run_id = %self.run_id,
                    hop = self.llm_calls,
                    failure_class = if first {
                        "chrono_first_byte_timeout"
                    } else {
                        "chrono_idle_timeout"
                    },
                    "assistant_agent_poc_hop_failed"
                );
                RunError::Upstream
            })?;
            first = false;
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|_| {
                tracing::warn!(
                    run_id = %self.run_id,
                    hop = self.llm_calls,
                    failure_class = "chrono_body_read_failed",
                    "assistant_agent_poc_hop_failed"
                );
                RunError::Upstream
            })?;
            let deltas = decoder.push(&chunk).map_err(|error| {
                tracing::warn!(
                    run_id = %self.run_id,
                    hop = self.llm_calls,
                    failure_class = %error,
                    "assistant_agent_poc_hop_decode_failed"
                );
                RunError::Upstream
            })?;
            if let Some(stage) = visible_stage {
                for delta in deltas {
                    self.text_delta(stage, &delta).await?;
                }
            }
        }
        // Logical completion never short-circuits transport consumption. The
        // decoder requires finish_reason, usage, and [DONE] at EOF.
        decoder.finish().map_err(|error| {
            tracing::warn!(
                run_id = %self.run_id,
                hop = self.llm_calls,
                failure_class = %error,
                "assistant_agent_poc_hop_decode_failed"
            );
            RunError::Upstream
        })
    }

    async fn execute_call(
        &mut self,
        registry: &ReadOnlyRegistry<'_>,
        call: &ReassembledToolCall,
    ) -> Result<ModelToolResult, RunError> {
        self.check_cancelled()?;
        let arguments: serde_json::Value =
            serde_json::from_str(&call.arguments).unwrap_or(serde_json::Value::Null);
        let (service_slug, endpoint_name) = tool_identity(registry, &call.name, &arguments);
        let public_identity = public_tool_identity(call, self.llm_calls);
        let started = Instant::now();
        self.start_tool(
            call,
            &public_identity,
            &service_slug,
            &endpoint_name,
            started,
        )
        .await?;

        if self.tool_calls >= self.budget.max_tool_calls {
            let result = ModelToolResult::synthetic("tool_call_budget_exhausted");
            self.complete_tool(ToolCompletion {
                public_identity: &public_identity,
                outcome: "skipped",
                result: &result,
                started,
                service_slug: &service_slug,
                endpoint: &endpoint_name,
                skill_version: None,
            })
            .await?;
            return Ok(result);
        }
        self.tool_calls += 1;

        let tool_timeout = self.budget.tool_timeout;
        let execution = self.execute_call_inner(registry, &call.name, &arguments);
        let result = match tokio::time::timeout(tool_timeout, execution).await {
            Ok(result) => result,
            Err(_) => {
                tracing::warn!(
                    run_id = %self.run_id,
                    call_id = %public_identity.call_id,
                    tool = public_identity.tool,
                    failure_class = "tool_timeout",
                    "assistant_agent_poc_tool_failed"
                );
                Ok((
                    ModelToolResult::synthetic("tool_timeout"),
                    "outcome_uncertain",
                    None,
                ))
            }
        };
        let (result, outcome, skill_version) = match result {
            Ok(value) => value,
            Err(error) => {
                tracing::warn!(
                    run_id = %self.run_id,
                    call_id = %public_identity.call_id,
                    tool = public_identity.tool,
                    failure_class = error,
                    "assistant_agent_poc_tool_failed"
                );
                (
                    ModelToolResult::synthetic(error),
                    if error == "denied_by_policy" {
                        "denied"
                    } else {
                        "failed"
                    },
                    None,
                )
            }
        };
        self.complete_tool(ToolCompletion {
            public_identity: &public_identity,
            outcome,
            result: &result,
            started,
            service_slug: &service_slug,
            endpoint: &endpoint_name,
            skill_version: skill_version.as_deref(),
        })
        .await?;
        Ok(result)
    }

    async fn start_tool(
        &mut self,
        call: &ReassembledToolCall,
        public_identity: &PublicToolIdentity,
        service_slug: &str,
        endpoint_name: &str,
        started: Instant,
    ) -> Result<(), RunError> {
        self.send_frame(&AgentFrame::ToolStarted {
            call_id: &public_identity.call_id,
            index: call.index,
            tool: public_identity.tool,
            target: ToolTarget {
                service_slug,
                endpoint: endpoint_name,
            },
        })
        .await?;
        self.in_flight_tool = Some(InFlightTool {
            public_identity: public_identity.clone(),
            service_slug: service_slug.to_string(),
            endpoint: endpoint_name.to_string(),
            started,
        });
        Ok(())
    }

    async fn execute_call_inner(
        &mut self,
        registry: &ReadOnlyRegistry<'_>,
        tool_name: &str,
        arguments: &serde_json::Value,
    ) -> Result<(ModelToolResult, &'static str, Option<String>), &'static str> {
        if !matches!(
            tool_name,
            "nyx_list_services"
                | "nyx_search_tools"
                | "nyx_call_tool"
                | "ornn_search_skills"
                | "ornn_get_skill"
        ) {
            return Err("unknown_tool");
        }
        tools::validate_tool_arguments(tool_name, arguments).map_err(|_| "invalid_args")?;
        match tool_name {
            "nyx_list_services" => {
                let query = arguments.get("query").and_then(serde_json::Value::as_str);
                let value = registry.list_services(query);
                Ok((
                    ModelToolResult::from_response(200, &value.to_string()),
                    "ok",
                    None,
                ))
            }
            "nyx_search_tools" => {
                let query = arguments["query"].as_str().expect("validated search query");
                let value = registry.search(query);
                Ok((
                    ModelToolResult::from_response(200, &value.to_string()),
                    "ok",
                    None,
                ))
            }
            "nyx_call_tool" => {
                let resolved_name = arguments
                    .get("tool_name")
                    .and_then(serde_json::Value::as_str)
                    .ok_or("invalid_args")?;
                let tool_args = arguments.get("arguments").ok_or("invalid_args")?;
                let operation = registry.resolve(resolved_name).ok_or("unknown_tool")?;
                tools::validate_operation_arguments(operation.endpoint, tool_args)
                    .map_err(|_| "invalid_args")?;
                self.execute_endpoint(operation.service, operation.endpoint, tool_args)
                    .await
                    .map(|result| {
                        let outcome = outcome_for_status(result.status);
                        (result, outcome, None)
                    })
            }
            "ornn_search_skills" => {
                let service = registry.ornn_service().ok_or("ornn_not_connected")?;
                let endpoint = tools::ornn_search_endpoint();
                let args =
                    tools::validate_ornn_search_args(arguments).map_err(|_| "invalid_args")?;
                let raw = self.execute_endpoint(service, &endpoint, &args).await?;
                if !(200..300).contains(&raw.status) {
                    return Ok((raw, "failed", None));
                }
                let requested_limit = args["limit"].as_u64().unwrap_or(10) as usize;
                let projected = tools::project_ornn_search(raw.server_body(), requested_limit)
                    .map_err(|_| "ornn_search_response_invalid")?;
                Ok((
                    ModelToolResult::from_response(200, &projected.to_string()),
                    "ok",
                    None,
                ))
            }
            "ornn_get_skill" => {
                if self.ornn_skill_fetched {
                    return Err("ornn_skill_already_fetched");
                }
                let service = registry.ornn_service().ok_or("ornn_not_connected")?;
                let endpoint = tools::ornn_get_endpoint();
                let args = tools::validate_ornn_get_args(arguments).map_err(|_| "invalid_args")?;
                self.ornn_skill_fetched = true;
                let raw = self.execute_endpoint(service, &endpoint, &args).await?;
                if !(200..300).contains(&raw.status) {
                    return Ok((raw, "failed", None));
                }
                let (fenced, version) = tools::extract_ornn_skill(raw.server_body())
                    .map_err(|_| "ornn_skill_package_invalid")?;
                Ok((ModelToolResult::from_response(200, &fenced), "ok", version))
            }
            _ => Err("unknown_tool"),
        }
    }

    async fn execute_endpoint(
        &self,
        service: &mcp_service::McpToolService,
        endpoint: &mcp_service::McpToolEndpoint,
        arguments: &serde_json::Value,
    ) -> Result<ModelToolResult, &'static str> {
        // The exact same predicate constructs the advertised view and guards
        // immediately before execution.
        if !tools::is_poc_operation_eligible(endpoint) {
            return Err("operation_not_allowed");
        }
        let prepared = mcp_service::prepare_proxy_tool_call(service, endpoint, arguments)
            .map_err(|_| "operation_not_allowed")?;
        let descriptor = prepared.operation_descriptor();
        let actor = self.auth_user.user_id.to_string();
        enforce_deny_only(
            &self.state.db,
            &actor,
            &self.auth_user.effective_approval_owner_user_id(),
            service,
            &descriptor,
        )
        .await?;
        let empty_nodes: Vec<String> = Vec::new();
        let exec_ctx = mcp_service::McpExecContext {
            api_key_id: None,
            allow_all_nodes: true,
            allowed_node_ids: &empty_nodes,
        };
        #[cfg(test)]
        self.test_dispatch_observer
            .tool_dispatches
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let (status, body) = mcp_service::execute_tool(
            &self.state.http_client,
            &self.state.db,
            &self.state.encryption_keys,
            &self.state.node_ws_manager,
            &self.state.billing,
            &actor,
            &actor,
            service,
            endpoint,
            prepared,
            &self.state.jwt_keys,
            &self.state.config,
            &self.state.connection_expiry_notifier,
            &self.state.token_exchange_cache,
            &self.state.cloud_response_cache,
            &exec_ctx,
            self.billing_egress_permit,
        )
        .await
        .map_err(|_| {
            tracing::warn!(
                run_id = %self.run_id,
                service_slug = %service.service_slug,
                endpoint = %endpoint.name,
                failure_class = "tool_execution_failed",
                "assistant_agent_poc_downstream_failed"
            );
            "tool_execution_failed"
        })?;
        Ok(ModelToolResult::from_response(status, &body))
    }

    async fn complete_tool(&mut self, completion: ToolCompletion<'_>) -> Result<(), RunError> {
        let duration_ms = elapsed_ms(completion.started);
        self.audit(
            "assistant_agent_poc_tool_call",
            tool_audit_data(&self.run_id, &completion, duration_ms),
        );
        self.in_flight_tool = None;
        self.send_frame(&AgentFrame::ToolCompleted {
            call_id: &completion.public_identity.call_id,
            tool: completion.public_identity.tool,
            outcome: completion.outcome,
            status: completion.result.status,
            duration_ms,
            result_bytes: completion.result.bytes,
            truncated: completion.result.truncated,
        })
        .await
    }

    async fn settle_in_flight_uncertain(&mut self) {
        let Some(tool) = self.in_flight_tool.take() else {
            return;
        };
        let result = ModelToolResult::synthetic("outcome_uncertain");
        let duration_ms = elapsed_ms(tool.started);
        self.audit(
            "assistant_agent_poc_tool_call",
            tool_audit_data(
                &self.run_id,
                &ToolCompletion {
                    public_identity: &tool.public_identity,
                    outcome: "outcome_uncertain",
                    result: &result,
                    started: tool.started,
                    service_slug: &tool.service_slug,
                    endpoint: &tool.endpoint,
                    skill_version: None,
                },
                duration_ms,
            ),
        );
        if !self.tx.is_closed() {
            let _ = self
                .send_frame(&AgentFrame::ToolCompleted {
                    call_id: &tool.public_identity.call_id,
                    tool: tool.public_identity.tool,
                    outcome: "outcome_uncertain",
                    status: result.status,
                    duration_ms,
                    result_bytes: result.bytes,
                    truncated: result.truncated,
                })
                .await;
        }
    }

    fn append_skipped_tool_messages(
        &self,
        messages: &mut Vec<serde_json::Value>,
        calls: &[ReassembledToolCall],
    ) {
        if calls.is_empty() {
            return;
        }
        messages.push(serde_json::json!({
            "role":"assistant",
            "content":null,
            "tool_calls": calls.iter().map(tool_call_value).collect::<Vec<_>>()
        }));
        for call in calls {
            messages.push(serde_json::json!({
                "role":"tool",
                "tool_call_id":call.id,
                "content":ModelToolResult::synthetic("tools_disabled").to_model_content()
            }));
        }
    }

    async fn stage(&self, stage: &str, status: &str, detail: &str) -> Result<(), RunError> {
        self.send_frame(&AgentFrame::Stage {
            stage,
            status,
            detail,
        })
        .await
    }

    async fn text_delta(&self, stage: &str, text: &str) -> Result<(), RunError> {
        self.send_frame(&AgentFrame::TextDelta { stage, text })
            .await
    }

    async fn send_frame(&self, frame: &AgentFrame<'_>) -> Result<(), RunError> {
        let json = serde_json::to_string(frame).map_err(|_| RunError::Internal)?;
        self.tx
            .send(Ok(Bytes::from(format!("data: {json}\n\n"))))
            .await
            .map_err(|_| RunError::Cancelled)
    }

    async fn send_done(&self) -> Result<(), RunError> {
        self.tx
            .send(Ok(Bytes::from_static(b"data: [DONE]\n\n")))
            .await
            .map_err(|_| RunError::Cancelled)
    }

    fn check_cancelled(&self) -> Result<(), RunError> {
        if self.cancellation.is_cancelled() {
            Err(RunError::Cancelled)
        } else {
            Ok(())
        }
    }

    async fn finish_error(&self, error: RunError) {
        if matches!(error, RunError::Cancelled) || self.tx.is_closed() {
            self.audit_finished("cancelled", None);
            return;
        }
        let (code, message) = match error {
            RunError::Deadline => ("deadline_exceeded", "The agent run exceeded its deadline."),
            RunError::ContextOverflow => (
                "context_overflow",
                "The bounded agent context is too large.",
            ),
            RunError::Upstream => ("upstream_failed", "The Chrono stream failed."),
            RunError::Internal => ("internal", "The agent run failed."),
            RunError::Cancelled => unreachable!(),
        };
        let _ = self.send_frame(&AgentFrame::Error { code, message }).await;
        let _ = self.send_done().await;
        self.audit_finished(code, None);
    }

    async fn settle_run_error(&mut self, error: RunError) {
        tracing::warn!(
            run_id = %self.run_id,
            failure_class = run_error_class(error),
            llm_calls = self.llm_calls,
            tool_calls = self.tool_calls,
            "assistant_agent_poc_run_failed"
        );
        self.settle_in_flight_uncertain().await;
        self.finish_error(error).await;
    }

    fn audit_finished(&self, status: &str, finish_reason: Option<&str>) {
        self.audit(
            "assistant_agent_poc_run_finished",
            serde_json::json!({
                "run_id": self.run_id,
                "status": status,
                "finish_reason": finish_reason,
                "tool_calls": self.tool_calls,
                "llm_calls": self.llm_calls,
                "duration_ms": elapsed_ms(self.started_at)
            }),
        );
    }

    fn audit(&self, event: &str, data: serde_json::Value) {
        audit_service::log_for_user(self.state.db.clone(), &self.auth_user, event, Some(data));
    }
}

fn run_error_class(error: RunError) -> &'static str {
    match error {
        RunError::Cancelled => "cancelled",
        RunError::Deadline => "deadline_exceeded",
        RunError::ContextOverflow => "context_overflow",
        RunError::Upstream => "upstream_failed",
        RunError::Internal => "internal",
    }
}

pub(crate) async fn enforce_deny_only(
    db: &mongodb::Database,
    actor_user_id: &str,
    effective_approval_owner_user_id: &str,
    service: &mcp_service::McpToolService,
    descriptor: &crate::services::operation_descriptor::OperationDescriptor,
) -> Result<(), &'static str> {
    let target =
        mcp_approval::approval_target_for_tool(db, effective_approval_owner_user_id, service)
            .await
            .map_err(|_| "approval_target_failed")?;
    if approval_service::evaluate_deny_only(
        db,
        actor_user_id,
        &target.service_owner_user_id,
        &target.service_id,
        descriptor,
    )
    .await
    .map_err(|_| "approval_check_failed")?
    {
        return Err("denied_by_policy");
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum ToolMode {
    Disabled,
    Enabled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PlanningTransition {
    Execute,
    ExecuteBatch,
    Done,
}

fn planning_transition(hop: &HopResult) -> PlanningTransition {
    match hop.finish_reason.as_str() {
        "stop" => PlanningTransition::Execute,
        "tool_calls" if !hop.tool_calls.is_empty() => PlanningTransition::ExecuteBatch,
        "tool_calls" => PlanningTransition::Done,
        _ => PlanningTransition::Done,
    }
}

fn outcome_for_status(status: u16) -> &'static str {
    if (200..300).contains(&status) {
        "ok"
    } else {
        "failed"
    }
}

fn build_hop_body(
    request: &DirectChatRequest,
    messages: &[serde_json::Value],
    phase: AgentPhase,
    tool_mode: ToolMode,
) -> serde_json::Value {
    let mut body_messages = Vec::with_capacity(messages.len() + 1);
    body_messages.push(serde_json::json!({
        "role": "system",
        "content": prompt::compose_agent_system_prompt(request.skill_slug.as_deref(), phase)
    }));
    body_messages.extend_from_slice(messages);
    let mut body = serde_json::json!({
        "model": request.model.as_deref().unwrap_or(assistant_direct::DEFAULT_DIRECT_MODEL),
        "stream": true,
        "stream_options": {"include_usage": true},
        "messages": body_messages,
        "parallel_tool_calls": false,
        "tools": tools::agent_tool_definitions(),
        "tool_choice": match tool_mode { ToolMode::Enabled => "auto", ToolMode::Disabled => "none" }
    });
    if let Some(effort) = &request.effort {
        body["reasoning_effort"] = serde_json::Value::String(effort.clone());
    }
    body
}

fn assistant_tool_call_message(hop: &HopResult) -> serde_json::Value {
    serde_json::json!({
        "role":"assistant",
        "content": if hop.text.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(hop.text.clone()) },
        "tool_calls": hop.tool_calls.iter().map(tool_call_value).collect::<Vec<_>>()
    })
}

fn append_assistant_tool_call_message(messages: &mut Vec<serde_json::Value>, hop: &HopResult) {
    messages.push(assistant_tool_call_message(hop));
}

fn append_tool_result_message(
    messages: &mut Vec<serde_json::Value>,
    call: &ReassembledToolCall,
    result: &ModelToolResult,
) {
    messages.push(serde_json::json!({
        "role": "tool",
        "tool_call_id": call.id,
        "content": result.to_model_content()
    }));
}

fn tool_call_value(call: &ReassembledToolCall) -> serde_json::Value {
    serde_json::json!({
        "id": call.id,
        "type": "function",
        "function": {"name":call.name,"arguments":call.arguments}
    })
}

fn tool_identity(
    registry: &ReadOnlyRegistry<'_>,
    logical_tool: &str,
    arguments: &serde_json::Value,
) -> (String, String) {
    match logical_tool {
        "nyx_call_tool" => arguments
            .get("tool_name")
            .and_then(serde_json::Value::as_str)
            .and_then(|name| registry.resolve(name))
            .map(|operation| {
                (
                    operation.service.service_slug.clone(),
                    operation.endpoint.name.clone(),
                )
            })
            .unwrap_or_else(|| ("nyxid".to_string(), "unknown_operation".to_string())),
        "ornn_search_skills" => ("ornn-api".to_string(), "skill_search".to_string()),
        "ornn_get_skill" => ("ornn-api".to_string(), "skill_fetch".to_string()),
        "nyx_search_tools" => ("nyxid".to_string(), "tool_catalog".to_string()),
        "nyx_list_services" => ("nyxid".to_string(), "connected_services".to_string()),
        _ => ("nyxid".to_string(), "unknown_tool".to_string()),
    }
}

fn public_tool_identity(call: &ReassembledToolCall, hop_number: usize) -> PublicToolIdentity {
    PublicToolIdentity {
        call_id: format!("tool-{hop_number}-{}", call.index),
        tool: safe_tool_name(&call.name),
    }
}

fn safe_tool_name(name: &str) -> &'static str {
    match name {
        "nyx_list_services" => "nyx_list_services",
        "nyx_search_tools" => "nyx_search_tools",
        "nyx_call_tool" => "nyx_call_tool",
        "ornn_search_skills" => "ornn_search_skills",
        "ornn_get_skill" => "ornn_get_skill",
        _ => "unknown_tool",
    }
}

fn tool_audit_data(
    run_id: &str,
    completion: &ToolCompletion<'_>,
    duration_ms: u64,
) -> serde_json::Value {
    serde_json::json!({
        "run_id": run_id,
        "call_id": completion.public_identity.call_id,
        "tool": completion.public_identity.tool,
        "service_slug": completion.service_slug,
        "endpoint": completion.endpoint,
        "outcome": completion.outcome,
        "status": completion.result.status,
        "bytes": completion.result.bytes,
        "truncated": completion.result.truncated,
        "duration_ms": duration_ms,
        "ornn_skill_id": if completion.public_identity.tool == "ornn_get_skill" { Some(tools::ORNN_DEMO_SKILL_GUID) } else { None },
        "ornn_skill_version": completion.skill_version
    })
}

fn elapsed_ms(started: Instant) -> u64 {
    started.elapsed().as_millis().try_into().unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    const TEST_USER_ID: &str = "a026fd00-bd86-4284-9832-9e5e65fc8f50";

    fn request() -> DirectChatRequest {
        assistant_direct::validate_direct_request(
            br#"{"messages":[{"role":"user","content":"hello"}],"model":"gpt-5.5"}"#,
        )
        .unwrap()
    }

    async fn test_run(
        budget: AgentRunBudget,
    ) -> (
        RunContext,
        mpsc::Receiver<Result<Bytes, Infallible>>,
        TestDispatchObserver,
    ) {
        let state = crate::test_utils::test_app_state_no_db().await;
        let (tx, rx) = mpsc::channel(32);
        let observer = TestDispatchObserver::default();
        let billing_policy =
            BillingRoutePolicy::Metered(crate::services::billing::BillingIngress::Proxy);
        let billing_egress_permit =
            crate::services::billing::route_inventory::enforce_billing_egress_classification(
                Some(billing_policy),
                crate::services::billing::BillingIngress::Proxy,
            )
            .unwrap();
        (
            RunContext {
                state,
                auth_user: crate::test_utils::test_auth_user(TEST_USER_ID),
                request: request(),
                chrono_service_id: "unused-chrono-service".to_string(),
                billing_policy,
                billing_egress_permit,
                connection_extension: None,
                cancellation: CancellationToken::new(),
                tx,
                run_id: "run-test".to_string(),
                started_at: Instant::now(),
                llm_calls: 0,
                tool_calls: 0,
                ornn_skill_fetched: false,
                in_flight_tool: None,
                budget,
                test_dispatch_observer: observer.clone(),
            },
            rx,
            observer,
        )
    }

    fn drain_frames(rx: &mut mpsc::Receiver<Result<Bytes, Infallible>>) -> String {
        let mut frames = String::new();
        while let Ok(item) = rx.try_recv() {
            frames.push_str(std::str::from_utf8(&item.unwrap()).unwrap());
        }
        frames
    }

    fn assert_started_tools_settle_before_terminal(frames: &str) {
        let terminal = frames
            .find("\"type\":\"error\"")
            .or_else(|| frames.find("\"type\":\"done\""))
            .expect("writable run has a terminal frame");
        let completed = frames
            .match_indices("\"type\":\"tool.completed\"")
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        for (ordinal, (started_at, _)) in frames
            .match_indices("\"type\":\"tool.started\"")
            .enumerate()
        {
            let completed_at = *completed.get(ordinal).expect("started tool settles");
            assert!(started_at < completed_at && completed_at < terminal);
        }
        assert_eq!(
            frames.matches("\"type\":\"tool.started\"").count(),
            completed.len()
        );
    }

    #[test]
    fn every_hop_is_streamed_with_usage_and_tools_are_phase_controlled() {
        let request = request();
        let plan = build_hop_body(&request, &[], AgentPhase::Plan, ToolMode::Disabled);
        let execute = build_hop_body(&request, &[], AgentPhase::Execute, ToolMode::Enabled);
        assert_eq!(plan["stream"], true);
        assert_eq!(plan["stream_options"]["include_usage"], true);
        assert_eq!(plan["tool_choice"], "none");
        assert_eq!(execute["tool_choice"], "auto");
        assert_eq!(execute["parallel_tool_calls"], false);
        assert_eq!(execute["tools"].as_array().unwrap().len(), 5);
    }

    #[tokio::test]
    async fn preflight_body_cap_is_fail_closed_before_upstream_dispatch() {
        let (mut run, _rx, observer) = test_run(AgentRunBudget::default()).await;
        let huge = serde_json::json!({"role":"tool","content":"x".repeat(MAX_UPSTREAM_BODY_BYTES)});
        let error = run
            .chrono_hop(&[huge], AgentPhase::Final, ToolMode::Disabled, None)
            .await
            .unwrap_err();
        assert_eq!(error, RunError::ContextOverflow);
        assert_eq!(observer.upstream_dispatches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn reconstructed_assistant_message_precedes_ordered_tool_replies() {
        let hop = HopResult {
            text: String::new(),
            finish_reason: "tool_calls".to_string(),
            saw_usage: true,
            tool_calls: vec![
                ReassembledToolCall {
                    index: 0,
                    id: "call-a".into(),
                    name: "nyx_list_services".into(),
                    arguments: "{}".into(),
                },
                ReassembledToolCall {
                    index: 1,
                    id: "call-b".into(),
                    name: "nyx_list_services".into(),
                    arguments: "{}".into(),
                },
            ],
        };
        let (mut run, _rx, _observer) = test_run(AgentRunBudget::default()).await;
        let services = Vec::new();
        let registry = ReadOnlyRegistry::new(&services, &services, None);
        let mut messages = Vec::new();
        run.execute_tool_batch(&registry, &hop, &mut messages)
            .await
            .unwrap();
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[1]["tool_call_id"], "call-a");
        assert_eq!(messages[2]["tool_call_id"], "call-b");
    }

    #[tokio::test]
    async fn in_flight_deadline_settles_tool_before_writable_terminal() {
        let (mut run, mut rx, _observer) = test_run(AgentRunBudget::default()).await;
        let call = hop("tool_calls", true).tool_calls.remove(0);
        let public = public_tool_identity(&call, 1);
        run.start_tool(
            &call,
            &public,
            "test-service",
            "test-endpoint",
            Instant::now(),
        )
        .await
        .unwrap();

        let outcome = await_run_outcome(
            run.cancellation.clone(),
            Duration::from_millis(5),
            std::future::pending(),
        )
        .await;
        assert_eq!(outcome, Err(RunError::Deadline));
        run.settle_run_error(outcome.unwrap_err()).await;

        let frames = drain_frames(&mut rx);
        let started = frames.find("\"type\":\"tool.started\"").unwrap();
        let settled = frames.find("\"type\":\"tool.completed\"").unwrap();
        let uncertain = frames.find("\"outcome\":\"outcome_uncertain\"").unwrap();
        let terminal = frames.find("\"code\":\"deadline_exceeded\"").unwrap();
        let done = frames.find("data: [DONE]").unwrap();
        assert!(
            started < settled && settled <= uncertain && uncertain < terminal && terminal < done
        );
        assert_started_tools_settle_before_terminal(&frames);
    }

    #[tokio::test]
    async fn cancelled_run_prevents_further_hops_and_tool_dispatch() {
        let (mut run, rx, observer) = test_run(AgentRunBudget::default()).await;
        let call = hop("tool_calls", true).tool_calls.remove(0);
        let public = public_tool_identity(&call, 1);
        run.start_tool(
            &call,
            &public,
            "test-service",
            "test-endpoint",
            Instant::now(),
        )
        .await
        .unwrap();
        drop(rx);
        run.cancellation.cancel();
        let outcome = await_run_outcome(
            run.cancellation.clone(),
            Duration::from_secs(1),
            std::future::pending(),
        )
        .await;
        assert_eq!(outcome, Err(RunError::Cancelled));
        run.settle_run_error(outcome.unwrap_err()).await;

        let hop_error = run
            .chrono_hop(&[], AgentPhase::Execute, ToolMode::Enabled, None)
            .await
            .unwrap_err();
        assert_eq!(hop_error, RunError::Cancelled);
        let next_call = hop("tool_calls", true).tool_calls.remove(0);
        let services = Vec::new();
        let registry = ReadOnlyRegistry::new(&services, &services, None);
        assert_eq!(
            run.execute_call(&registry, &next_call).await.unwrap_err(),
            RunError::Cancelled
        );
        assert_eq!(observer.upstream_dispatches.load(Ordering::SeqCst), 0);
        assert_eq!(observer.tool_dispatches.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn force_final_arithmetic_and_tool_budget_exhaustion_use_real_paths() {
        let budget = AgentRunBudget {
            max_llm_calls: 3,
            max_tool_calls: 1,
            ..AgentRunBudget::default()
        };
        let (mut run, mut rx, observer) = test_run(budget).await;
        run.llm_calls = 1;
        assert!(!run.should_force_final());
        run.llm_calls = 2;
        assert!(run.should_force_final());
        run.llm_calls = 0;
        run.tool_calls = 1;
        assert!(run.should_force_final());

        let call = hop("tool_calls", true).tool_calls.remove(0);
        let services = Vec::new();
        let registry = ReadOnlyRegistry::new(&services, &services, None);
        let result = run.execute_call(&registry, &call).await.unwrap();
        assert!(
            result
                .to_model_content()
                .contains("tool_call_budget_exhausted")
        );
        assert_eq!(observer.tool_dispatches.load(Ordering::SeqCst), 0);
        let frames = drain_frames(&mut rx);
        assert!(frames.contains("\"type\":\"tool.started\""));
        assert!(frames.contains("\"type\":\"tool.completed\""));
        assert!(frames.contains("\"outcome\":\"skipped\""));
        run.finish_error(RunError::Deadline).await;
        let frames = format!("{frames}{}", drain_frames(&mut rx));
        assert_started_tools_settle_before_terminal(&frames);
    }

    fn hop(reason: &str, with_tool_call: bool) -> HopResult {
        HopResult {
            text: "plan".to_string(),
            finish_reason: reason.to_string(),
            saw_usage: true,
            tool_calls: if with_tool_call {
                vec![ReassembledToolCall {
                    index: 0,
                    id: "call-plan".to_string(),
                    name: "nyx_list_services".to_string(),
                    arguments: "{}".to_string(),
                }]
            } else {
                Vec::new()
            },
        }
    }

    #[test]
    fn planning_transition_table_is_deterministic() {
        assert_eq!(
            planning_transition(&hop("stop", false)),
            PlanningTransition::Execute
        );
        assert_eq!(
            planning_transition(&hop("tool_calls", true)),
            PlanningTransition::ExecuteBatch
        );
        for reason in ["length", "content_filter", "future_reason"] {
            assert_eq!(
                planning_transition(&hop(reason, false)),
                PlanningTransition::Done,
                "planning reason {reason} must not dispatch an execution hop"
            );
        }
    }

    #[test]
    fn non_success_http_status_is_a_failed_tool_outcome() {
        assert_eq!(outcome_for_status(200), "ok");
        assert_eq!(outcome_for_status(299), "ok");
        assert_eq!(outcome_for_status(300), "failed");
        assert_eq!(outcome_for_status(404), "failed");
        assert_eq!(outcome_for_status(500), "failed");
    }

    #[test]
    fn model_controlled_tool_identity_never_reaches_frames_or_audit_metadata() {
        let secret_id = "call-accessToken=super-secret";
        let secret_name = "unknown_{\"password\":\"super-secret\"}";
        let call = ReassembledToolCall {
            index: 7,
            id: secret_id.to_string(),
            name: secret_name.to_string(),
            arguments: "{}".to_string(),
        };
        let public = public_tool_identity(&call, 3);
        assert_eq!(public.call_id, "tool-3-7");
        assert_eq!(public.tool, "unknown_tool");

        let frame = serde_json::to_string(&AgentFrame::ToolStarted {
            call_id: &public.call_id,
            index: call.index,
            tool: public.tool,
            target: ToolTarget {
                service_slug: "nyxid",
                endpoint: "unknown_tool",
            },
        })
        .unwrap();
        let completed_frame = serde_json::to_string(&AgentFrame::ToolCompleted {
            call_id: &public.call_id,
            tool: public.tool,
            outcome: "failed",
            status: 0,
            duration_ms: 1,
            result_bytes: 0,
            truncated: false,
        })
        .unwrap();
        let result = ModelToolResult::synthetic("unknown_tool");
        let audit = tool_audit_data(
            "run-safe",
            &ToolCompletion {
                public_identity: &public,
                outcome: "failed",
                result: &result,
                started: Instant::now(),
                service_slug: "nyxid",
                endpoint: "unknown_tool",
                skill_version: None,
            },
            1,
        )
        .to_string();
        for serialized in [frame, completed_frame, audit] {
            assert!(!serialized.contains(secret_id));
            assert!(!serialized.contains(secret_name));
            assert!(!serialized.contains("super-secret"));
            assert!(serialized.contains("unknown_tool"));
        }

        let continuation = tool_call_value(&call).to_string();
        assert!(continuation.contains(secret_id));
        assert!(continuation.contains("super-secret"));
    }
}
