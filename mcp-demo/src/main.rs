//! Standalone stdio MCP demo binary for directory-listing introspection.
//!
//! Mirrors the curated tool surface of `backend/src/mcp_demo.rs` but
//! compiles in a fraction of the time because it depends only on
//! serde_json + std (no tokio, axum, mongodb, …). Use this when a
//! constrained build pipeline (e.g. Glama's scoring runners) can't
//! finish a full `cargo build -p nyxid` within its timeout.
//!
//! Transport: newline-delimited JSON-RPC 2.0 on stdin/stdout, per the
//! MCP stdio spec. Stderr is reserved for diagnostics.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};

const PROTOCOL_VERSION: &str = "2025-03-26";

fn main() -> std::io::Result<()> {
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout().lock();
    let reader = BufReader::new(stdin.lock());

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let req: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(err) => {
                eprintln!("nyxid-mcp-demo: failed to parse request: {err}");
                continue;
            }
        };

        // Notifications (no `id`) get no response per JSON-RPC 2.0.
        let Some(id) = req.get("id").cloned() else {
            continue;
        };

        let method = req.get("method").and_then(Value::as_str).unwrap_or("");
        let response = match method {
            "initialize" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {
                    "protocolVersion": PROTOCOL_VERSION,
                    "capabilities": { "tools": {} },
                    "serverInfo": {
                        "name": "nyxid",
                        "version": env!("CARGO_PKG_VERSION"),
                    },
                },
            }),
            "tools/list" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": { "tools": tool_definitions() },
            }),
            "tools/call" => {
                let tool_name = req
                    .get("params")
                    .and_then(|p| p.get("name"))
                    .and_then(Value::as_str)
                    .unwrap_or("(unknown)");
                json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "result": {
                        "content": [{
                            "type": "text",
                            "text": format!(
                                "This is the NyxID demo image. The '{tool_name}' tool is exposed for \
                                 directory introspection only and is not wired to a backing service. \
                                 To use NyxID's full tool surface, run an authenticated NyxID instance \
                                 and connect over the Streamable HTTP transport at /mcp. See \
                                 https://github.com/ChronoAIProject/NyxID."
                            ),
                        }],
                    },
                })
            }
            "ping" => json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": {},
            }),
            other => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method not found: {other}"),
                },
            }),
        };

        let serialized = serde_json::to_string(&response)?;
        stdout.write_all(serialized.as_bytes())?;
        stdout.write_all(b"\n")?;
        stdout.flush()?;
    }

    Ok(())
}

fn tool_definitions() -> Value {
    json!([
        {
            "name": "nyx_proxy_request",
            "description": "Forward an HTTP request through NyxID to any downstream service the agent has been granted access to. NyxID injects the appropriate credential at proxy time so the agent never holds raw API keys. Supports cloud APIs (OpenAI, GitHub, Slack, Lark, Telegram, etc.), internal REST endpoints, and localhost services reached over a NAT-pierced credential node. Every call is rate-limited and audit-logged per agent identity.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "service_slug": {
                        "type": "string",
                        "description": "Slug of the connected service (e.g. \"llm-openai\", \"api-github\"). Use nyx_list_services to enumerate the slugs visible to this agent."
                    },
                    "method": {
                        "type": "string",
                        "enum": ["GET", "POST", "PUT", "PATCH", "DELETE"],
                        "description": "HTTP method for the downstream request."
                    },
                    "path": {
                        "type": "string",
                        "description": "Path on the downstream service, beginning with '/'. NyxID prepends the service's base URL automatically."
                    },
                    "body": {
                        "type": ["object", "string", "null"],
                        "description": "Request body for POST/PUT/PATCH. Object values are JSON-encoded; string values are sent verbatim. Omit for GET/DELETE."
                    },
                    "headers": {
                        "type": "object",
                        "additionalProperties": { "type": "string" },
                        "description": "Additional headers to forward. Authorization / API-key headers are injected by NyxID and MUST NOT be set here."
                    }
                },
                "required": ["service_slug", "method", "path"]
            }
        },
        {
            "name": "nyx_list_services",
            "description": "Enumerate the downstream services this agent can call through NyxID. Returns each service's slug, display name, base URL, auth method, configured rate limits, and (when an OpenAPI spec is available) its callable endpoints. Use this for tool discovery before issuing nyx_proxy_request — agent-key scope determines which services are visible.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "include_endpoints": {
                        "type": "boolean",
                        "description": "When true, include the parsed OpenAPI endpoint list per service. Adds latency on services with large specs.",
                        "default": false
                    },
                    "category": {
                        "type": "string",
                        "enum": ["llm", "api", "ssh", "node", "internal"],
                        "description": "Optional filter by service category. Omit to list all categories."
                    }
                },
                "required": []
            }
        },
        {
            "name": "nyx_request_approval",
            "description": "Request human approval before performing a sensitive action. NyxID delivers the request to the user via push notification (Telegram or mobile app) and blocks the agent until the user approves, denies, or the request times out. Use this for destructive operations, financial transactions, or any action where unattended automation is inappropriate. Approvals can be granted ad-hoc per call or via pre-configured grant rules.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "action_summary": {
                        "type": "string",
                        "description": "One-line summary of what the agent intends to do, written for a human reader (e.g. \"Send $500 wire to vendor X\", \"Delete production database backup older than 30 days\")."
                    },
                    "details": {
                        "type": "string",
                        "description": "Optional longer description with the full context the user needs to decide. Keep under 1000 characters; rendered as plain text in the approval UI."
                    },
                    "service_slug": {
                        "type": "string",
                        "description": "Slug of the service the agent will call after approval. Used to scope approval grants and audit logs."
                    },
                    "timeout_secs": {
                        "type": "integer",
                        "description": "How long to wait for a human decision before failing. Bounded by the user's configured maximum.",
                        "minimum": 30,
                        "maximum": 3600,
                        "default": 300
                    }
                },
                "required": ["action_summary", "service_slug"]
            }
        },
        {
            "name": "nyx_exchange_identity",
            "description": "Exchange the agent's NyxID identity for a delegated access token bound to a downstream OIDC service (RFC 8693 token exchange). Lets the agent call APIs on behalf of a specific user without holding that user's long-lived credentials. The returned token carries the original user's identity claims while remaining auditable as an agent action.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "audience": {
                        "type": "string",
                        "description": "Target downstream service identifier (resource URI or audience claim) the exchanged token should be valid for."
                    },
                    "subject_user": {
                        "type": "string",
                        "description": "User ID or email of the principal whose identity the agent is acting on behalf of. Must already exist in NyxID and have granted the agent delegation rights."
                    },
                    "scope": {
                        "type": "string",
                        "description": "Space-separated OAuth scopes to request on the exchanged token. Must be a subset of what the subject_user has approved for this agent."
                    },
                    "ttl_secs": {
                        "type": "integer",
                        "description": "Requested token lifetime in seconds. Bounded by the audience's configured maximum (typically 900-3600 seconds).",
                        "minimum": 60,
                        "maximum": 7200,
                        "default": 900
                    }
                },
                "required": ["audience", "subject_user"]
            }
        }
    ])
}
