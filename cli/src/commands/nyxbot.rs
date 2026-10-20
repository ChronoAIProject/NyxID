//! First-party owner control. Credentials are resolved exclusively by ApiClient;
//! a rejected explicit/Agent Key credential never falls back to a saved owner.
use crate::{
    api::{ApiClient, ApiError},
    cli::{AuthArgs, OutputFormat},
};
use anyhow::{Context, Result, bail};
use clap::{Args, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{io::Read, path::PathBuf, time::Duration};
use tokio::time::Instant;
use uuid::Uuid;
const ROOT: &str = "/assistant/nyxagent";
const MAX_INPUT_BYTES: usize = 131_072;
const MAX_INPUT_CHARS: usize = 32_768;

#[derive(Subcommand)]
pub enum NyxbotCommands {
    /// Submit owner work; waits unless --no-wait is selected
    Run(RunArgs),
    /// Inspect the durable invocation and its bounded replies
    Show(InvocationArgs),
    /// Wait until completion or an owner decision; timeout never cancels work
    Wait(WaitArgs),
    /// Cancel only this invocation and its associated work
    Cancel(InvocationArgs),
    /// Discover owned agents and specialist IDs
    Agents(AuthArgs),
    /// List conversations, optionally filtered by agent
    Conversations {
        #[arg(long)]
        agent: Option<String>,
        #[arg(long)]
        cursor: Option<String>,
        #[arg(long,default_value_t=50,value_parser=clap::value_parser!(u32).range(1..=100))]
        limit: u32,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Read a conversation's durable messages and owner action cards
    History {
        conversation: String,
        #[arg(long)]
        before_seq: Option<u64>,
        #[arg(long,default_value_t=50,value_parser=clap::value_parser!(u32).range(1..=100))]
        limit: u32,
        #[command(flatten)]
        auth: AuthArgs,
    },
    /// Discover available model IDs
    Models(AuthArgs),
}
#[derive(Args)]
#[command(group(clap::ArgGroup::new("input").required(true).args(["text","file"])))]
pub struct RunArgs {
    /// Prompt text (use --file - to read stdin)
    #[arg(long)]
    pub text: Option<String>,
    /// UTF-8 prompt file, or - for stdin (bounded to 32,768 characters)
    #[arg(long)]
    pub file: Option<PathBuf>,
    /// Existing conversation; omit to create a new thread
    #[arg(long,conflicts_with_all=["agent","model"])]
    pub conversation: Option<String>,
    /// Owned agent ID for a new thread; defaults to NyxBot
    #[arg(long)]
    pub agent: Option<String>,
    /// Available model ID for a new thread
    #[arg(long)]
    pub model: Option<String>,
    /// UUID v4; reuse with the identical request after a lost response
    #[arg(long,value_parser=parse_id)]
    pub idempotency_key: Option<String>,
    /// Return after durable acceptance
    #[arg(long)]
    pub no_wait: bool,
    /// Total wait deadline, including HTTP response reads
    #[arg(long,default_value_t=300,value_parser=clap::value_parser!(u64).range(1..=86400))]
    pub timeout: u64,
    #[command(flatten)]
    pub auth: AuthArgs,
}
#[derive(Args)]
pub struct InvocationArgs {
    #[arg(value_parser=parse_id)]
    pub invocation: String,
    #[command(flatten)]
    pub auth: AuthArgs,
}
#[derive(Args)]
pub struct WaitArgs {
    #[arg(value_parser=parse_id)]
    pub invocation: String,
    #[arg(long,default_value_t=300,value_parser=clap::value_parser!(u64).range(1..=86400))]
    pub timeout: u64,
    #[command(flatten)]
    pub auth: AuthArgs,
}
fn parse_id(value: &str) -> std::result::Result<String, String> {
    if Uuid::parse_str(value).is_ok_and(|id| {
        id.get_variant() == uuid::Variant::RFC4122
            && id.get_version_num() == 4
            && id.to_string() == value
    }) {
        Ok(value.into())
    } else {
        Err("expected a canonical UUID v4".into())
    }
}
fn input(text: Option<String>, file: Option<PathBuf>) -> Result<String> {
    let value = match (text, file) {
        (Some(value), None) => value,
        (None, Some(path)) => {
            let reader: Box<dyn Read> = if path.as_os_str() == "-" {
                Box::new(std::io::stdin())
            } else {
                Box::new(std::fs::File::open(path).context("Cannot read prompt file")?)
            };
            read_input(reader)?
        }
        _ => bail!("Choose exactly one of --text or --file"),
    };
    validate_input(&value)?;
    Ok(value)
}
fn read_input(reader: impl Read) -> Result<String> {
    let mut bytes = Vec::new();
    reader
        .take(MAX_INPUT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .context("Cannot read prompt")?;
    if bytes.len() > MAX_INPUT_BYTES {
        bail!("Prompt exceeds 131,072 bytes");
    }
    String::from_utf8(bytes).context("Prompt must be UTF-8")
}
fn validate_input(value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > MAX_INPUT_BYTES
        || value.chars().count() > MAX_INPUT_CHARS
    {
        bail!("Prompt must contain 1–32,768 characters (at most 131,072 UTF-8 bytes)");
    }
    Ok(())
}

#[derive(Clone, Deserialize, Serialize)]
struct Receipt {
    invocation_id: String,
    conversation_id: String,
    turn_id: String,
    status: String,
    terminal: bool,
    #[serde(flatten)]
    details: serde_json::Map<String, Value>,
}
impl Receipt {
    fn validate(&self, key: &str) -> Result<()> {
        if self.invocation_id != key
            || parse_id(&self.invocation_id).is_err()
            || Uuid::parse_str(&self.turn_id).is_err()
            || !self.conversation_id.starts_with("nyxa-")
            || !matches!(
                self.status.as_str(),
                "running"
                    | "waiting"
                    | "requires_action"
                    | "requires_review"
                    | "completed"
                    | "failed"
                    | "cancelling"
                    | "cancelled"
                    | "deleted"
            )
            || self.terminal
                != matches!(
                    self.status.as_str(),
                    "completed" | "failed" | "cancelled" | "deleted"
                )
        {
            bail!("Invalid invocation response");
        }
        Ok(())
    }
    fn outcome(&self) -> (&'static str, i32) {
        match self.status.as_str() {
            "failed" => ("failed", 20),
            "cancelled" | "deleted" => ("cancelled", 21),
            "requires_action" | "requires_review" => ("owner_action_required", 22),
            "completed" => ("completed", 0),
            _ => ("accepted", 0),
        }
    }
    fn done_waiting(&self) -> bool {
        self.terminal || matches!(self.status.as_str(), "requires_action" | "requires_review")
    }
}
/// The complete output was already written once; main only selects the exit.
#[derive(Debug, thiserror::Error)]
#[error("NyxBot command outcome")]
pub struct OutcomeExit(pub i32);
#[derive(Serialize)]
struct Report {
    cli_status: String,
    invocation_id: String,
    invocation: Option<Receipt>,
    error: Option<&'static str>,
    http_status: Option<u16>,
    exit_code: i32,
    error_code: Option<i64>,
    reason: Option<String>,
    guidance: Option<&'static str>,
}
fn report(key: &str, receipt: Receipt) -> Report {
    let (status, exit) = receipt.outcome();
    Report {
        cli_status: status.into(),
        invocation_id: key.into(),
        invocation: Some(receipt),
        error: None,
        http_status: None,
        exit_code: exit,
        error_code: None,
        reason: None,
        guidance: None,
    }
}
#[derive(Serialize)]
struct CommandFailure {
    cli_status: &'static str,
    error: &'static str,
    http_status: Option<u16>,
    exit_code: i32,
    error_code: Option<i64>,
    reason: Option<String>,
    guidance: Option<&'static str>,
}

fn classify_failure(error: &anyhow::Error) -> CommandFailure {
    let mut result = CommandFailure {
        cli_status: "transport_unknown",
        error: "transport_error",
        http_status: None,
        exit_code: 24,
        error_code: None,
        reason: None,
        guidance: None,
    };
    if let Some(auth) = error.downcast_ref::<crate::auth::ReauthRequired>() {
        result.cli_status = "login_required";
        result.error = "login_required";
        result.exit_code = 3;
        result.reason = Some(auth.code.into());
        result.guidance = Some("Run nyxid login and choose account access.");
        return result;
    }
    if error
        .downcast_ref::<crate::auth::RefreshUnavailable>()
        .is_some()
    {
        result.cli_status = "renewal_uncertain";
        result.error = "renewal_uncertain";
        result.exit_code = 4;
        result.guidance = Some("Retry later; session renewal is temporarily unavailable.");
        return result;
    }
    if error
        .downcast_ref::<reqwest::Error>()
        .is_some_and(|e| e.is_decode())
        || error.downcast_ref::<serde_json::Error>().is_some()
    {
        result.cli_status = "protocol_error";
        result.error = "invalid_server_response";
        result.exit_code = 25;
        return result;
    }
    let api = error.downcast_ref::<ApiError>();
    let http = api.map(|e| e.status().as_u16());
    let reason = api
        .and_then(|e| e.body_json())
        .and_then(|v| v.get("details"))
        .and_then(|v| v.get("reason"))
        .and_then(Value::as_str)
        .filter(|s| s.len() <= 80 && s.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'))
        .map(str::to_owned);
    let guidance = if reason.as_deref() == Some("credential_type_unsupported") || http == Some(401)
    {
        Some(
            "A first-party account login is required. Run nyxid login and choose account access, or select an existing account profile. Credentials are never switched automatically.",
        )
    } else {
        None
    };
    CommandFailure {
        cli_status: if http.is_some() {
            "request_failed"
        } else {
            "transport_unknown"
        },
        error: if http.is_some() {
            "http_error"
        } else {
            "transport_error"
        },
        http_status: http,
        exit_code: if http.is_some() { 1 } else { 24 },
        error_code: api.and_then(|e| e.response()).map(|e| e.error_code),
        reason,
        guidance,
    }
}
fn failure(key: &str, error: &anyhow::Error, last: Option<Receipt>) -> Report {
    let mut failure = classify_failure(error);
    match failure.exit_code {
        3 => {
            failure.guidance = Some(
                "Run nyxid login and choose account access; recover this invocation before resubmitting.",
            )
        }
        4 => {
            failure.guidance = Some(
                "Retry inspection later; session renewal is temporarily unavailable. Keep the same invocation ID.",
            )
        }
        25 => return protocol_error(key),
        _ => {}
    }
    Report {
        cli_status: failure.cli_status.into(),
        invocation_id: key.into(),
        invocation: last,
        error: Some(failure.error),
        http_status: failure.http_status,
        exit_code: failure.exit_code,
        error_code: failure.error_code,
        reason: failure.reason,
        guidance: failure.guidance,
    }
}
fn timed_out(key: &str, last: Option<Receipt>) -> Report {
    Report {
        cli_status: "wait_timeout".into(),
        invocation_id: key.into(),
        invocation: last,
        error: Some("local_wait_timeout"),
        http_status: None,
        exit_code: 23,
        error_code: None,
        reason: None,
        guidance: None,
    }
}
fn protocol_error(key: &str) -> Report {
    Report {
        cli_status: "protocol_error".into(),
        invocation_id: key.into(),
        invocation: None,
        error: Some("invalid_invocation_response"),
        http_status: None,
        exit_code: 25,
        error_code: None,
        reason: None,
        guidance: None,
    }
}
async fn read_receipt(
    api: &mut ApiClient,
    key: &str,
    deadline: Instant,
) -> std::result::Result<Result<Receipt>, tokio::time::error::Elapsed> {
    tokio::time::timeout_at(deadline, api.get(&format!("{ROOT}/invocations/{key}"))).await
}
async fn wait(
    api: &mut ApiClient,
    key: &str,
    deadline: Instant,
    mut last: Option<Receipt>,
) -> Report {
    loop {
        if let Some(receipt) = last.as_ref() {
            if receipt.validate(key).is_err() {
                return protocol_error(key);
            }
            if receipt.done_waiting() {
                return report(key, last.take().expect("checked receipt"));
            }
        }
        let next = match read_receipt(api, key, deadline).await {
            Err(_) => return timed_out(key, last),
            Ok(Err(error)) => return failure(key, &error, last),
            Ok(Ok(value)) => value,
        };
        if next.validate(key).is_err() {
            return protocol_error(key);
        }
        if next.done_waiting() {
            return report(key, next);
        }
        last = Some(next);
        if tokio::time::timeout_at(deadline, tokio::time::sleep(Duration::from_secs(1)))
            .await
            .is_err()
        {
            return timed_out(key, last);
        }
    }
}

/// Removes terminal control introducers (including C1 and bidi controls), while
/// retaining ordinary newlines/tabs in a reply. It never interprets ANSI/OSC.
fn terminal_text(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .filter(|c| !matches!(*c,'\u{061c}'|'\u{200e}'|'\u{200f}'|'\u{202a}'..='\u{202e}'|'\u{2066}'..='\u{2069}'))
        .collect()
}
fn render(report: &Report, output: OutputFormat) -> Result<String> {
    if matches!(output, OutputFormat::Json) {
        return Ok(terminal_json(&serde_json::to_string(report)?));
    }
    let mut text = format!(
        "Invocation: {}\nOutcome: {}\n",
        report.invocation_id, report.cli_status
    );
    if let Some(reason) = &report.reason {
        text.push_str(&format!("Reason: {reason}\n"));
    }
    if let Some(code) = report.error_code {
        text.push_str(&format!("Error code: {code}\n"));
    }
    if let Some(guidance) = report.guidance {
        text.push_str(guidance);
        text.push('\n');
    }
    if let Some(receipt) = &report.invocation {
        text.push_str(&format!(
            "Conversation: {}\nTurn: {}\nStatus: {}\n",
            receipt.conversation_id, receipt.turn_id, receipt.status
        ));
        if let Some(reason) = receipt.details.get("error_code").and_then(Value::as_str) {
            text.push_str(&format!(
                "Status reason: {}\n",
                reason.chars().take(100).collect::<String>()
            ));
        }
        if let Some(replies) = receipt.details.get("replies").and_then(Value::as_array) {
            for reply in replies.iter().rev() {
                let conversation = reply
                    .get("conversation_id")
                    .and_then(Value::as_str)
                    .unwrap_or(&receipt.conversation_id);
                let turn = reply
                    .get("turn_id")
                    .and_then(Value::as_str)
                    .unwrap_or(&receipt.turn_id);
                if conversation != receipt.conversation_id || turn != receipt.turn_id {
                    text.push_str(&format!(
                        "\nReply from conversation {conversation}, turn {turn}:\n"
                    ));
                }
                if let Some(value) = reply.get("text").and_then(Value::as_str) {
                    text.push('\n');
                    text.push_str(value);
                    text.push('\n');
                }
                if reply.get("text_truncated").and_then(Value::as_bool) == Some(true) {
                    text.push_str(&format!("[Reply excerpt truncated. Read retained history: nyxid nyxbot history {conversation}]\n"));
                }
            }
        }
        if receipt
            .details
            .get("replies_truncated")
            .and_then(Value::as_bool)
            == Some(true)
        {
            text.push_str(&format!(
                "[Additional replies omitted. Read retained history: nyxid nyxbot history {}]\n",
                receipt.conversation_id
            ));
        }
        if receipt.status == "requires_action" || receipt.status == "requires_review" {
            text.push_str(
                "Review the invocation's conversations and owner action cards in NyxID.\n",
            );
        }
    }
    if report.exit_code == 23 || report.exit_code == 24 {
        text.push_str("Recover with nyxid nyxbot show or wait using this invocation ID; reuse the original idempotency key for a matching run retry.\n");
    }
    Ok(terminal_text(&text))
}

/// Preserve JSON values exactly while preventing terminal interpretation of
/// control characters that serde_json otherwise emits literally.
fn terminal_json(encoded: &str) -> String {
    use std::fmt::Write;
    let mut safe = String::with_capacity(encoded.len());
    for character in encoded.chars() {
        if matches!(character, '\u{7f}'..='\u{9f}' | '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        {
            write!(&mut safe, "\\u{:04x}", u32::from(character)).expect("write to String");
        } else {
            safe.push(character);
        }
    }
    safe
}

fn render_discovery(value: &Value, output: OutputFormat) -> Result<String> {
    let encoded = serde_json::to_string_pretty(value)?;
    Ok(if matches!(output, OutputFormat::Json) {
        terminal_json(&encoded)
    } else {
        terminal_text(&encoded)
    })
}
fn emit(report: Report, output: OutputFormat) -> Result<()> {
    println!("{}", render(&report, output)?);
    if report.exit_code != 0 {
        return Err(OutcomeExit(report.exit_code).into());
    }
    Ok(())
}

fn render_discovery_failure(failure: &CommandFailure, output: OutputFormat) -> Result<String> {
    if matches!(output, OutputFormat::Json) {
        return Ok(terminal_json(&serde_json::to_string(failure)?));
    }
    let mut text = format!("NyxBot discovery: {}\n", failure.cli_status);
    if let Some(status) = failure.http_status {
        text.push_str(&format!("HTTP status: {status}\n"));
    }
    if let Some(code) = failure.error_code {
        text.push_str(&format!("Error code: {code}\n"));
    }
    if let Some(reason) = &failure.reason {
        text.push_str(&format!("Reason: {reason}\n"));
    }
    if let Some(guidance) = failure.guidance {
        text.push_str(guidance);
        text.push('\n');
    }
    Ok(terminal_text(&text))
}

async fn read_discovery(
    api: &mut ApiClient,
    path: &str,
) -> std::result::Result<Value, CommandFailure> {
    match tokio::time::timeout(Duration::from_secs(30), api.get(path)).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(classify_failure(&error)),
        Err(_) => Err(CommandFailure {
            cli_status: "request_timeout",
            error: "local_request_timeout",
            http_status: None,
            exit_code: 23,
            error_code: None,
            reason: None,
            guidance: Some("Retry discovery later; this command does not submit work."),
        }),
    }
}

async fn discovery(auth: AuthArgs, path: String) -> Result<()> {
    let mut api = ApiClient::from_auth(&auth)?;
    match read_discovery(&mut api, &path).await {
        Ok(value) => {
            println!("{}", render_discovery(&value, auth.output)?);
            Ok(())
        }
        Err(failure) => {
            println!("{}", render_discovery_failure(&failure, auth.output)?);
            Err(OutcomeExit(failure.exit_code).into())
        }
    }
}
pub async fn run(command: NyxbotCommands) -> Result<()> {
    match command {
        NyxbotCommands::Run(args) => {
            let text = input(args.text, args.file)?;
            let key = args
                .idempotency_key
                .unwrap_or_else(|| Uuid::new_v4().to_string());
            let mut api = ApiClient::from_auth(&args.auth)?;
            let deadline = Instant::now() + Duration::from_secs(args.timeout);
            // A safe recoverable ID precedes network I/O, even when stdout is
            // redirected. Prompt text and credentials never enter this notice.
            eprintln!(
                "{}",
                json!({"event":"invocation_submission","invocation_id":key})
            );
            let body = json!({"idempotency_key":key,"text":text,"conversation_id":args.conversation,"agent_id":args.agent,"model":args.model});
            let receipt: Receipt = match tokio::time::timeout_at(
                deadline.min(Instant::now() + Duration::from_secs(30)),
                api.post(&format!("{ROOT}/invocations"), &body),
            )
            .await
            {
                Err(_) => return emit(timed_out(&key, None), args.auth.output),
                Ok(Err(error)) => return emit(failure(&key, &error, None), args.auth.output),
                Ok(Ok(receipt)) => receipt,
            };
            if receipt.validate(&key).is_err() {
                return emit(protocol_error(&key), args.auth.output);
            }
            let result = if args.no_wait {
                report(&key, receipt)
            } else {
                wait(&mut api, &key, deadline, Some(receipt)).await
            };
            emit(result, args.auth.output)
        }
        NyxbotCommands::Wait(args) => {
            let mut api = ApiClient::from_auth(&args.auth)?;
            let deadline = Instant::now() + Duration::from_secs(args.timeout);
            emit(
                wait(&mut api, &args.invocation, deadline, None).await,
                args.auth.output,
            )
        }
        NyxbotCommands::Show(args) => inspect(args, false).await,
        NyxbotCommands::Cancel(args) => inspect(args, true).await,
        NyxbotCommands::Agents(auth) => discovery(auth, format!("{ROOT}/agents")).await,
        NyxbotCommands::Models(auth) => discovery(auth, format!("{ROOT}/models")).await,
        NyxbotCommands::Conversations {
            auth,
            agent,
            cursor,
            limit,
        } => {
            let mut path = format!("{ROOT}/conversations?limit={limit}");
            if let Some(agent) = agent {
                path.push_str(&format!("&agent_id={}", urlencoding::encode(&agent)));
            }
            if let Some(cursor) = cursor {
                path.push_str(&format!("&cursor={}", urlencoding::encode(&cursor)));
            }
            discovery(auth, path).await
        }
        NyxbotCommands::History {
            auth,
            conversation,
            before_seq,
            limit,
        } => {
            let mut path = format!(
                "{ROOT}/conversations/{}?limit={limit}",
                urlencoding::encode(&conversation)
            );
            if let Some(seq) = before_seq {
                path.push_str(&format!("&before_seq={seq}"));
            }
            discovery(auth, path).await
        }
    }
}
async fn inspect(args: InvocationArgs, cancel: bool) -> Result<()> {
    let mut api = ApiClient::from_auth(&args.auth)?;
    let key = &args.invocation;
    let deadline = Instant::now() + Duration::from_secs(30);
    let result = if cancel {
        tokio::time::timeout_at(
            deadline,
            api.post::<Receipt, _>(&format!("{ROOT}/invocations/{key}/cancel"), &json!({})),
        )
        .await
    } else {
        read_receipt(&mut api, key, deadline).await
    };
    let result = match result {
        Err(_) => timed_out(key, None),
        Ok(Err(error)) => failure(key, &error, None),
        Ok(Ok(receipt)) => {
            if receipt.validate(key).is_err() {
                protocol_error(key)
            } else {
                report(key, receipt)
            }
        }
    };
    emit(result, args.auth.output)
}

#[cfg(test)]
#[path = "nyxbot_tests.rs"]
mod tests;
