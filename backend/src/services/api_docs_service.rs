use std::net::{Ipv4Addr, SocketAddr};
use std::sync::LazyLock;
use std::time::{Duration, Instant};

use bytes::BytesMut;
use dashmap::DashMap;
use serde::{Deserialize, Serialize};

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::DownstreamService;

const OPENAPI_PROBE_PATHS: &[&str] = &[
    "/openapi.json",
    "/swagger.json",
    "/docs/openapi.json",
    "/.well-known/openapi",
];

const ASYNCAPI_PROBE_PATHS: &[&str] = &["/asyncapi.json", "/.well-known/asyncapi"];
const SCALAR_SCRIPT_SRC: &str = "https://cdn.jsdelivr.net";
const SPEC_FETCH_TIMEOUT: Duration = Duration::from_secs(5);
const SPEC_CACHE_TTL: Duration = Duration::from_secs(60);
const MAX_SPEC_RESPONSE_BYTES: usize = 5 * 1024 * 1024;
const MAX_SPEC_CACHE_ENTRIES: usize = 128;

static SPEC_CACHE: LazyLock<DashMap<String, CachedSpecEntry>> = LazyLock::new(DashMap::new);
static SPEC_FETCH_CLIENT: LazyLock<reqwest::Client> = LazyLock::new(|| {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("Failed to create hardened spec fetch client")
});

#[derive(Clone)]
struct CachedSpecEntry {
    spec: serde_json::Value,
    expires_at: Instant,
}

impl CachedSpecEntry {
    fn is_fresh(&self, now: Instant) -> bool {
        self.expires_at > now
    }
}

struct ValidatedSpecFetchTarget {
    url: url::Url,
    host: String,
    resolved_addrs: Vec<SocketAddr>,
    requires_dns_pinning: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDocumentationMetadata {
    pub openapi_spec_url: Option<String>,
    pub asyncapi_spec_url: Option<String>,
    pub streaming_supported: bool,
}

pub async fn discover_service_docs(
    base_url: &str,
    explicit_openapi_spec_url: Option<String>,
    explicit_asyncapi_spec_url: Option<String>,
) -> ServiceDocumentationMetadata {
    let openapi_spec_url = match explicit_openapi_spec_url {
        Some(url) if !url.trim().is_empty() => {
            if fetch_json_spec(&url).await.is_ok() {
                Some(url)
            } else {
                None
            }
        }
        _ => discover_spec_url(base_url, OPENAPI_PROBE_PATHS).await,
    };

    let asyncapi_spec_url = match explicit_asyncapi_spec_url {
        Some(url) if !url.trim().is_empty() => {
            if fetch_json_spec(&url).await.is_ok() {
                Some(url)
            } else {
                None
            }
        }
        _ => discover_spec_url(base_url, ASYNCAPI_PROBE_PATHS).await,
    };

    let streaming_supported = if let Some(ref openapi_url) = openapi_spec_url {
        fetch_json_spec(openapi_url)
            .await
            .ok()
            .is_some_and(|spec| detect_streaming_from_openapi(&spec))
    } else {
        false
    } || asyncapi_spec_url.is_some();

    ServiceDocumentationMetadata {
        openapi_spec_url,
        asyncapi_spec_url,
        streaming_supported,
    }
}

pub fn is_auto_discovered_openapi_spec_url(base_url: &str, spec_url: &str) -> bool {
    is_probe_url(base_url, spec_url, OPENAPI_PROBE_PATHS)
}

pub fn is_auto_discovered_asyncapi_spec_url(base_url: &str, spec_url: &str) -> bool {
    is_probe_url(base_url, spec_url, ASYNCAPI_PROBE_PATHS)
}

pub async fn fetch_downstream_openapi_spec(
    service: &DownstreamService,
    proxy_base_url: &str,
) -> AppResult<serde_json::Value> {
    let spec_url = service
        .openapi_spec_url
        .as_deref()
        .ok_or_else(|| AppError::NotFound("Service has no OpenAPI spec configured".to_string()))?;

    let mut spec = fetch_json_spec(spec_url).await?;
    if spec.get("openapi").is_none() && spec.get("swagger").is_none() {
        return Err(AppError::BadRequest(
            "Downstream spec is not an OpenAPI or Swagger document".to_string(),
        ));
    }

    let base = proxy_base_url.trim_end_matches('/');
    let proxy_url = format!("{base}/api/v1/proxy/{}/", service.id);
    spec["servers"] = serde_json::json!([{
        "url": proxy_url,
        "description": "NyxID authenticated proxy"
    }]);
    spec["x-nyxid-service-id"] = serde_json::Value::String(service.id.clone());
    spec["x-nyxid-service-slug"] = serde_json::Value::String(service.slug.clone());

    Ok(spec)
}

pub async fn fetch_downstream_asyncapi_spec(
    service: &DownstreamService,
    proxy_base_url: &str,
) -> AppResult<serde_json::Value> {
    let spec_url = service
        .asyncapi_spec_url
        .as_deref()
        .ok_or_else(|| AppError::NotFound("Service has no AsyncAPI spec configured".to_string()))?;

    let mut spec = fetch_json_spec(spec_url).await?;
    if spec.get("asyncapi").is_none() {
        return Err(AppError::BadRequest(
            "Downstream spec is not an AsyncAPI document".to_string(),
        ));
    }

    spec["x-nyxid-service-id"] = serde_json::Value::String(service.id.clone());
    spec["x-nyxid-service-slug"] = serde_json::Value::String(service.slug.clone());
    spec["x-nyxid-proxy-base-url"] = serde_json::Value::String(format!(
        "{}/api/v1/proxy/{}/",
        proxy_base_url.trim_end_matches('/'),
        service.id
    ));

    Ok(spec)
}

pub fn render_scalar_html(title: &str, spec_url: &str) -> String {
    let escaped_title = escape_html(title);
    let escaped_spec_url = escape_html(spec_url);
    format!(
        r#"<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{escaped_title}</title>
    <style>
      html, body, #app {{ height: 100%; margin: 0; }}
      body {{ background: #0f172a; }}
    </style>
  </head>
  <body>
    <script
      id="api-reference"
      data-url="{escaped_spec_url}"
      data-layout="modern"
    ></script>
    <script src="https://cdn.jsdelivr.net/npm/@scalar/api-reference"></script>
  </body>
</html>"#
    )
}

pub fn render_catalog_html() -> &'static str {
    r#"<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>NyxID API Catalog</title>
    <style>
      :root {
        color-scheme: light;
        --bg: #f6efe2;
        --panel: rgba(255, 255, 255, 0.82);
        --ink: #18212f;
        --muted: #556070;
        --line: rgba(24, 33, 47, 0.12);
        --accent: #c76a34;
        --accent-soft: rgba(199, 106, 52, 0.12);
      }
      * { box-sizing: border-box; }
      body {
        margin: 0;
        font-family: "IBM Plex Sans", "Avenir Next", sans-serif;
        color: var(--ink);
        background:
          radial-gradient(circle at top left, rgba(199, 106, 52, 0.22), transparent 28%),
          radial-gradient(circle at top right, rgba(30, 97, 110, 0.18), transparent 24%),
          linear-gradient(160deg, #f7f0e4, #e6ecef);
      }
      main {
        max-width: 1100px;
        margin: 0 auto;
        padding: 40px 20px 64px;
      }
      h1 {
        margin: 0 0 8px;
        font-size: clamp(2rem, 4vw, 3rem);
        letter-spacing: -0.04em;
      }
      p {
        margin: 0;
        color: var(--muted);
        line-height: 1.6;
      }
      .panel {
        margin-top: 28px;
        background: var(--panel);
        border: 1px solid var(--line);
        border-radius: 20px;
        backdrop-filter: blur(16px);
        overflow: hidden;
        box-shadow: 0 20px 80px rgba(24, 33, 47, 0.08);
      }
      table {
        width: 100%;
        border-collapse: collapse;
      }
      th, td {
        padding: 16px 18px;
        text-align: left;
        border-bottom: 1px solid var(--line);
        vertical-align: top;
      }
      th {
        font-size: 0.78rem;
        letter-spacing: 0.08em;
        text-transform: uppercase;
        color: var(--muted);
      }
      td small {
        display: block;
        color: var(--muted);
        margin-top: 4px;
      }
      .badge {
        display: inline-flex;
        padding: 4px 10px;
        border-radius: 999px;
        background: var(--accent-soft);
        color: var(--accent);
        font-size: 0.78rem;
        font-weight: 600;
      }
      a {
        color: var(--ink);
        text-underline-offset: 0.16em;
      }
      #status {
        margin-top: 18px;
        font-size: 0.92rem;
      }
      @media (max-width: 720px) {
        th:nth-child(3), td:nth-child(3) { display: none; }
      }
    </style>
  </head>
  <body>
    <main>
      <h1>NyxID API Catalog</h1>
      <p>Discover NyxID proxy services, documentation status, and streaming capabilities from one place.</p>
      <p id="status">Loading catalog…</p>
      <div class="panel">
        <table>
          <thead>
            <tr>
              <th>Service</th>
              <th>Docs</th>
              <th>Streaming</th>
              <th>Proxy</th>
            </tr>
          </thead>
          <tbody id="catalog-body"></tbody>
        </table>
      </div>
    </main>
    <script>
      const body = document.getElementById('catalog-body');
      const status = document.getElementById('status');
      const createMessageRow = (message) => {
        const row = document.createElement('tr');
        const cell = document.createElement('td');
        cell.colSpan = 4;
        cell.textContent = message;
        row.appendChild(cell);
        return row;
      };
      const createLink = (href, label) => {
        const link = document.createElement('a');
        link.href = href;
        link.textContent = label;
        return link;
      };
      fetch('/api/v1/proxy/services', { credentials: 'include' })
        .then(async (response) => {
          if (!response.ok) {
            throw new Error(`Catalog request failed with ${response.status}`);
          }
          return response.json();
        })
        .then((payload) => {
          status.textContent = `${payload.total} services available`;
          if (!payload.services.length) {
            body.replaceChildren(createMessageRow('No proxyable services found.'));
            return;
          }
          const rows = payload.services.map((service) => {
            const row = document.createElement('tr');

            const serviceCell = document.createElement('td');
            const serviceName = document.createElement('strong');
            serviceName.textContent = service.name;
            const serviceSlug = document.createElement('small');
            serviceSlug.textContent = service.slug;
            serviceCell.append(serviceName, serviceSlug);

            const docsCell = document.createElement('td');
            if (service.docs_url) {
              docsCell.appendChild(createLink(service.docs_url, 'Scalar UI'));
            } else {
              docsCell.textContent = 'Unavailable';
            }
            if (service.openapi_url) {
              const openapi = document.createElement('small');
              openapi.appendChild(createLink(service.openapi_url, 'OpenAPI'));
              docsCell.appendChild(openapi);
            }
            if (service.asyncapi_url) {
              const asyncapi = document.createElement('small');
              asyncapi.appendChild(createLink(service.asyncapi_url, 'AsyncAPI'));
              docsCell.appendChild(asyncapi);
            }

            const streamingCell = document.createElement('td');
            if (service.streaming_supported) {
              const badge = document.createElement('span');
              badge.className = 'badge';
              badge.textContent = 'Streaming';
              streamingCell.appendChild(badge);
            } else {
              streamingCell.textContent = 'No';
            }

            const proxyCell = document.createElement('td');
            proxyCell.appendChild(
              createLink(
                service.proxy_url_slug.replace('{path}', ''),
                service.proxy_url_slug
              )
            );

            row.append(serviceCell, docsCell, streamingCell, proxyCell);
            return row;
          });
          body.replaceChildren(...rows);
        })
        .catch((error) => {
          status.textContent = error.message;
          body.replaceChildren(createMessageRow('Failed to load catalog.'));
        });
    </script>
  </body>
</html>"#
}

pub fn scalar_docs_csp() -> String {
    format!(
        "default-src 'none'; script-src {}; style-src 'unsafe-inline'; img-src 'self' data: https:; font-src 'self' data: https:; connect-src 'self'; frame-ancestors 'none'",
        SCALAR_SCRIPT_SRC
    )
}

pub fn catalog_csp() -> &'static str {
    "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; img-src 'self' data: https:; font-src 'self' data: https:; connect-src 'self'; frame-ancestors 'none'"
}

pub fn build_asyncapi_document(base_url: &str) -> serde_json::Value {
    let base = base_url.trim_end_matches('/');
    serde_json::json!({
        "asyncapi": "3.0.0",
        "info": {
            "title": "NyxID Streaming and WebSocket API",
            "version": env!("CARGO_PKG_VERSION"),
            "description": "AsyncAPI document for NyxID streaming protocols, including node agent WebSockets, SSH tunnels, MCP SSE transport, and downstream proxy streaming."
        },
        "servers": {
            "nyxid": {
                "host": base,
                "protocol": "https"
            }
        },
        "channels": {
            "nodeAgent": {
                "address": "/api/v1/nodes/ws",
                "messages": {
                    "register": { "$ref": "#/components/messages/Register" },
                    "auth": { "$ref": "#/components/messages/Auth" },
                    "proxyRequest": { "$ref": "#/components/messages/ProxyRequest" },
                    "proxyResponseStart": { "$ref": "#/components/messages/ProxyResponseStart" },
                    "proxyResponseChunk": { "$ref": "#/components/messages/ProxyResponseChunk" },
                    "proxyResponseEnd": { "$ref": "#/components/messages/ProxyResponseEnd" }
                }
            },
            "sshTunnel": {
                "address": "/api/v1/ssh/{service_id}",
                "messages": {
                    "sshBinaryFrame": { "$ref": "#/components/messages/SshBinaryFrame" }
                }
            },
            "mcpHttp": {
                "address": "/mcp",
                "messages": {
                    "streamableHttp": { "$ref": "#/components/messages/McpSseStream" }
                }
            },
            "proxySse": {
                "address": "/api/v1/proxy/{service_id}/{path}",
                "messages": {
                    "sseEvent": { "$ref": "#/components/messages/SseEvent" }
                }
            },
            "llmSse": {
                "address": "/api/v1/llm/{provider_slug}/v1/{path}",
                "messages": {
                    "sseEvent": { "$ref": "#/components/messages/SseEvent" }
                }
            }
        },
        "operations": {
            "connectNodeAgent": {
                "action": "send",
                "channel": { "$ref": "#/channels/nodeAgent" },
                "summary": "Register or authenticate a credential node over WebSocket"
            },
            "consumeNodeProxyStream": {
                "action": "receive",
                "channel": { "$ref": "#/channels/nodeAgent" },
                "summary": "Receive streaming proxy chunks from the node agent"
            },
            "openSshTunnel": {
                "action": "send",
                "channel": { "$ref": "#/channels/sshTunnel" },
                "summary": "Open an authenticated SSH-over-WebSocket tunnel"
            },
            "consumeSshTunnel": {
                "action": "receive",
                "channel": { "$ref": "#/channels/sshTunnel" },
                "summary": "Exchange raw SSH bytes over WebSocket binary frames"
            },
            "consumeMcpSse": {
                "action": "receive",
                "channel": { "$ref": "#/channels/mcpHttp" },
                "summary": "Consume MCP streamable HTTP events"
            },
            "consumeProxySse": {
                "action": "receive",
                "channel": { "$ref": "#/channels/proxySse" },
                "summary": "Consume downstream SSE through the authenticated proxy"
            }
        },
        "components": {
            "messages": {
                "Register": {
                    "name": "register",
                    "payload": {
                        "type": "object",
                        "required": ["type", "registration_token"],
                        "properties": {
                            "type": { "type": "string", "const": "register" },
                            "registration_token": { "type": "string" }
                        }
                    }
                },
                "Auth": {
                    "name": "auth",
                    "payload": {
                        "type": "object",
                        "required": ["type", "node_id", "auth_token"],
                        "properties": {
                            "type": { "type": "string", "const": "auth" },
                            "node_id": { "type": "string" },
                            "auth_token": { "type": "string" }
                        }
                    }
                },
                "ProxyRequest": {
                    "name": "proxy_request",
                    "payload": {
                        "type": "object",
                        "required": ["type", "request_id", "service_id", "path", "method"],
                        "properties": {
                            "type": { "type": "string", "const": "proxy_request" },
                            "request_id": { "type": "string" },
                            "service_id": { "type": "string" },
                            "path": { "type": "string" },
                            "method": { "type": "string" }
                        }
                    }
                },
                "ProxyResponseStart": {
                    "name": "proxy_response_start",
                    "payload": {
                        "type": "object",
                        "required": ["type", "request_id", "status"],
                        "properties": {
                            "type": { "type": "string", "const": "proxy_response_start" },
                            "request_id": { "type": "string" },
                            "status": { "type": "integer" }
                        }
                    }
                },
                "ProxyResponseChunk": {
                    "name": "proxy_response_chunk",
                    "payload": {
                        "type": "object",
                        "required": ["type", "request_id", "data"],
                        "properties": {
                            "type": { "type": "string", "const": "proxy_response_chunk" },
                            "request_id": { "type": "string" },
                            "data": { "type": "string", "description": "Base64-encoded chunk payload" }
                        }
                    }
                },
                "ProxyResponseEnd": {
                    "name": "proxy_response_end",
                    "payload": {
                        "type": "object",
                        "required": ["type", "request_id"],
                        "properties": {
                            "type": { "type": "string", "const": "proxy_response_end" },
                            "request_id": { "type": "string" }
                        }
                    }
                },
                "SshBinaryFrame": {
                    "name": "ssh_binary_frame",
                    "payload": {
                        "type": "string",
                        "format": "binary",
                        "description": "Raw SSH TCP payload encoded as a WebSocket binary frame"
                    }
                },
                "McpSseStream": {
                    "name": "mcp_stream",
                    "payload": {
                        "type": "string",
                        "description": "Streamable HTTP payload encoded as Server-Sent Events"
                    }
                },
                "SseEvent": {
                    "name": "sse_event",
                    "payload": {
                        "type": "string",
                        "description": "UTF-8 SSE event frame"
                    }
                }
            }
        }
    })
}

async fn discover_spec_url(base_url: &str, candidate_paths: &[&str]) -> Option<String> {
    let base = base_url.trim_end_matches('/');
    for path in candidate_paths {
        let candidate = format!("{base}{path}");
        if fetch_json_spec(&candidate).await.is_ok() {
            return Some(candidate);
        }
    }
    None
}

fn is_probe_url(base_url: &str, spec_url: &str, candidate_paths: &[&str]) -> bool {
    let base = base_url.trim_end_matches('/');
    candidate_paths
        .iter()
        .any(|path| format!("{base}{path}") == spec_url)
}

async fn fetch_json_spec(url: &str) -> AppResult<serde_json::Value> {
    let target = validate_spec_fetch_target(url).await?;
    let cache_key = target.url.to_string();
    if let Some(spec) = get_cached_spec(&cache_key) {
        return Ok(spec);
    }

    let response = if target.requires_dns_pinning {
        build_pinned_spec_fetch_client(&target)?
            .get(target.url.clone())
            .timeout(SPEC_FETCH_TIMEOUT)
            .send()
            .await
    } else {
        SPEC_FETCH_CLIENT
            .get(target.url.clone())
            .timeout(SPEC_FETCH_TIMEOUT)
            .send()
            .await
    }
    .map_err(|error| {
        tracing::warn!(url = %target.url, %error, "Failed to fetch downstream API spec");
        AppError::BadRequest("Failed to fetch spec".to_string())
    })?;

    if !response.status().is_success() {
        return Err(AppError::BadRequest(format!(
            "Spec returned HTTP {}",
            response.status()
        )));
    }

    let mut response = response;
    let mut body = BytesMut::new();
    while let Some(chunk) = response.chunk().await.map_err(|error| {
        tracing::warn!(url = %target.url, %error, "Failed to read downstream API spec body");
        AppError::BadRequest("Failed to read spec body".to_string())
    })? {
        if body.len() + chunk.len() > MAX_SPEC_RESPONSE_BYTES {
            tracing::warn!(
                url = %target.url,
                limit_bytes = MAX_SPEC_RESPONSE_BYTES,
                "Downstream API spec exceeded size limit"
            );
            return Err(AppError::BadRequest(format!(
                "Spec response exceeded the {} byte limit",
                MAX_SPEC_RESPONSE_BYTES
            )));
        }
        body.extend_from_slice(&chunk);
    }

    let spec = serde_json::from_slice(&body)
        .map_err(|e| AppError::BadRequest(format!("Spec was not valid JSON: {e}")))?;
    cache_spec(&cache_key, &spec);
    Ok(spec)
}

fn detect_streaming_from_openapi(spec: &serde_json::Value) -> bool {
    let Some(paths) = spec.get("paths").and_then(|value| value.as_object()) else {
        return false;
    };

    for path_item in paths.values() {
        let Some(path_object) = path_item.as_object() else {
            continue;
        };

        for method in ["get", "post", "put", "patch", "delete"] {
            let Some(operation) = path_object.get(method).and_then(|value| value.as_object())
            else {
                continue;
            };

            let Some(responses) = operation
                .get("responses")
                .and_then(|value| value.as_object())
            else {
                continue;
            };

            for response in responses.values() {
                let Some(content) = response.get("content").and_then(|value| value.as_object())
                else {
                    continue;
                };

                if content.contains_key("text/event-stream") {
                    return true;
                }
            }
        }
    }

    false
}

fn escape_html(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#x27;"),
            _ => escaped.push(ch),
        }
    }
    escaped
}

async fn validate_spec_fetch_target(url: &str) -> AppResult<ValidatedSpecFetchTarget> {
    let parsed = url::Url::parse(url)
        .map_err(|_| AppError::BadRequest("Spec URL is invalid".to_string()))?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return Err(AppError::BadRequest(
            "Spec URL must use http or https".to_string(),
        ));
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| AppError::BadRequest("Spec URL must include a hostname".to_string()))?;
    let normalized_host = normalize_fetch_host(host);
    if is_blocked_fetch_hostname(&normalized_host) {
        return Err(AppError::BadRequest(
            "Spec URL must not target a private or internal hostname".to_string(),
        ));
    }

    let port = parsed
        .port_or_known_default()
        .ok_or_else(|| AppError::BadRequest("Spec URL must include a valid port".to_string()))?;
    let (resolved_addrs, requires_dns_pinning) =
        resolve_fetch_host_socket_addrs(&normalized_host, port).await?;
    if resolved_addrs.is_empty() {
        return Err(AppError::BadRequest(
            "Spec URL host did not resolve to any IP addresses".to_string(),
        ));
    }
    if resolved_addrs
        .iter()
        .map(SocketAddr::ip)
        .any(is_private_or_internal_ip)
    {
        return Err(AppError::BadRequest(
            "Spec URL must not resolve to private or internal IP addresses".to_string(),
        ));
    }

    let mut url = parsed;
    url.set_host(Some(&normalized_host))
        .map_err(|_| AppError::BadRequest("Spec URL hostname is invalid".to_string()))?;

    Ok(ValidatedSpecFetchTarget {
        url,
        host: normalized_host,
        resolved_addrs,
        requires_dns_pinning,
    })
}

async fn resolve_fetch_host_socket_addrs(
    host: &str,
    port: u16,
) -> AppResult<(Vec<SocketAddr>, bool)> {
    if let Ok(ip) = parse_fetch_host_ip(host) {
        return Ok((vec![SocketAddr::new(ip, port)], false));
    }

    let resolved = tokio::net::lookup_host((host, port))
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to resolve spec host: {e}")))?;
    Ok((resolved.collect(), true))
}

fn parse_fetch_host_ip(host: &str) -> Result<std::net::IpAddr, std::net::AddrParseError> {
    normalize_fetch_host(host).parse()
}

fn is_blocked_fetch_hostname(host: &str) -> bool {
    matches!(
        normalize_fetch_host(host).as_str(),
        "localhost" | "metadata.google.internal"
    )
}

fn normalize_fetch_host(host: &str) -> String {
    host.trim()
        .trim_start_matches('[')
        .trim_end_matches(']')
        .trim_end_matches('.')
        .to_ascii_lowercase()
}

fn get_cached_spec(url: &str) -> Option<serde_json::Value> {
    let now = Instant::now();
    let entry = SPEC_CACHE.get(url)?;
    if entry.is_fresh(now) {
        return Some(entry.spec.clone());
    }

    drop(entry);
    SPEC_CACHE.remove(url);
    None
}

fn cache_spec(url: &str, spec: &serde_json::Value) {
    prune_stale_cache_entries(Instant::now());
    if !SPEC_CACHE.contains_key(url) && SPEC_CACHE.len() >= MAX_SPEC_CACHE_ENTRIES {
        evict_oldest_cache_entry();
    }

    SPEC_CACHE.insert(
        url.to_string(),
        CachedSpecEntry {
            spec: spec.clone(),
            expires_at: Instant::now() + SPEC_CACHE_TTL,
        },
    );
}

fn prune_stale_cache_entries(now: Instant) {
    SPEC_CACHE.retain(|_, entry| entry.is_fresh(now));
}

fn evict_oldest_cache_entry() {
    let oldest_key = SPEC_CACHE
        .iter()
        .min_by_key(|entry| entry.expires_at)
        .map(|entry| entry.key().clone());
    if let Some(oldest_key) = oldest_key {
        SPEC_CACHE.remove(&oldest_key);
    }
}

fn build_pinned_spec_fetch_client(target: &ValidatedSpecFetchTarget) -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(10))
        .pool_idle_timeout(Duration::from_secs(90))
        .redirect(reqwest::redirect::Policy::none())
        .resolve_to_addrs(&target.host, &target.resolved_addrs)
        .build()
        .map_err(|e| AppError::Internal(format!("Failed to build pinned spec fetch client: {e}")))
}

fn is_private_or_internal_ip(ip: std::net::IpAddr) -> bool {
    match ip {
        std::net::IpAddr::V4(ipv4) => {
            ipv4.is_loopback()
                || ipv4.is_private()
                || ipv4.is_link_local()
                || ipv4.is_unspecified()
                || ipv4.is_broadcast()
                || is_rfc6598_cgnat(ipv4)
                || ipv4.octets()[0] == 169 && ipv4.octets()[1] == 254
        }
        std::net::IpAddr::V6(ipv6) => {
            ipv6.is_loopback()
                || ipv6.is_unspecified()
                || (ipv6.segments()[0] & 0xfe00) == 0xfc00
                || (ipv6.segments()[0] & 0xffc0) == 0xfe80
                || ipv6
                    .to_ipv4_mapped()
                    .is_some_and(|mapped| is_private_or_internal_ip(mapped.into()))
        }
    }
}

fn is_rfc6598_cgnat(ipv4: Ipv4Addr) -> bool {
    ipv4.octets()[0] == 100 && (64..=127).contains(&ipv4.octets()[1])
}

#[cfg(test)]
mod tests {
    use super::{
        CachedSpecEntry, MAX_SPEC_CACHE_ENTRIES, ServiceDocumentationMetadata,
        build_asyncapi_document, cache_spec, catalog_csp, detect_streaming_from_openapi,
        get_cached_spec, render_scalar_html, scalar_docs_csp, validate_spec_fetch_target,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn detects_streaming_media_type_in_openapi() {
        let spec = serde_json::json!({
            "openapi": "3.1.0",
            "paths": {
                "/stream": {
                    "get": {
                        "responses": {
                            "200": {
                                "content": {
                                    "text/event-stream": {
                                        "schema": { "type": "string" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        assert!(detect_streaming_from_openapi(&spec));
    }

    #[test]
    fn ignores_non_streaming_openapi_specs() {
        let spec = serde_json::json!({
            "openapi": "3.1.0",
            "paths": {
                "/users": {
                    "get": {
                        "responses": {
                            "200": {
                                "content": {
                                    "application/json": {
                                        "schema": { "type": "object" }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        });

        assert!(!detect_streaming_from_openapi(&spec));
    }

    #[test]
    fn asyncapi_document_uses_requested_base_url() {
        let doc = build_asyncapi_document("https://nyxid.example.com");
        assert_eq!(doc["servers"]["nyxid"]["host"], "https://nyxid.example.com");
        assert_eq!(
            doc["channels"]["sshTunnel"]["address"],
            "/api/v1/ssh/{service_id}"
        );
    }

    #[test]
    fn scalar_html_embeds_spec_url() {
        let html = render_scalar_html("Docs", "/api/v1/docs/openapi.json");
        assert!(html.contains("/api/v1/docs/openapi.json"));
        assert!(html.contains("@scalar/api-reference"));
    }

    #[test]
    fn scalar_html_escapes_untrusted_values() {
        let html = render_scalar_html("<Docs>", "\"/api/v1/docs/openapi.json\"");
        assert!(html.contains("&lt;Docs&gt;"));
        assert!(html.contains("&quot;/api/v1/docs/openapi.json&quot;"));
        assert!(!html.contains("<title><Docs></title>"));
    }

    #[test]
    fn documentation_metadata_serializes() {
        let metadata = ServiceDocumentationMetadata {
            openapi_spec_url: Some("https://example.com/openapi.json".to_string()),
            asyncapi_spec_url: None,
            streaming_supported: true,
        };
        let json = serde_json::to_value(metadata).expect("serialize metadata");
        assert_eq!(json["streaming_supported"], true);
    }

    #[test]
    fn scalar_docs_csp_allows_scalar_script_source() {
        let csp = scalar_docs_csp();
        assert!(csp.contains("https://cdn.jsdelivr.net"));
        assert!(csp.contains("connect-src 'self'"));
    }

    #[test]
    fn catalog_csp_allows_inline_script_for_embedded_catalog_page() {
        let csp = catalog_csp();
        assert!(csp.contains("script-src 'unsafe-inline'"));
    }

    #[tokio::test]
    async fn spec_fetch_validation_rejects_private_targets() {
        assert!(
            validate_spec_fetch_target("http://100.64.0.1/openapi.json")
                .await
                .is_err()
        );
        assert!(
            validate_spec_fetch_target("http://127.0.0.1/openapi.json")
                .await
                .is_err()
        );
        assert!(
            validate_spec_fetch_target("https://[::1]/openapi.json")
                .await
                .is_err()
        );
        assert!(
            validate_spec_fetch_target("http://metadata.google.internal/openapi.json")
                .await
                .is_err()
        );
    }

    #[test]
    fn cached_specs_are_returned_while_fresh() {
        super::SPEC_CACHE.clear();
        let url = "https://example.com/openapi.json";
        let spec = serde_json::json!({ "openapi": "3.1.0" });
        cache_spec(url, &spec);

        assert_eq!(get_cached_spec(url), Some(spec));
    }

    #[test]
    fn stale_cache_entries_are_evicted() {
        super::SPEC_CACHE.clear();
        let url = "https://example.com/stale-openapi.json";
        super::SPEC_CACHE.insert(
            url.to_string(),
            CachedSpecEntry {
                spec: serde_json::json!({ "openapi": "3.1.0" }),
                expires_at: Instant::now() - Duration::from_secs(1),
            },
        );

        assert!(get_cached_spec(url).is_none());
        assert!(!super::SPEC_CACHE.contains_key(url));
    }

    #[test]
    fn cache_spec_evicts_oldest_entry_when_capacity_is_reached() {
        super::SPEC_CACHE.clear();

        for idx in 0..MAX_SPEC_CACHE_ENTRIES {
            super::SPEC_CACHE.insert(
                format!("https://example.com/spec-{idx}.json"),
                CachedSpecEntry {
                    spec: serde_json::json!({ "openapi": "3.1.0", "idx": idx }),
                    expires_at: Instant::now() + Duration::from_secs(120 + idx as u64),
                },
            );
        }

        cache_spec(
            "https://example.com/spec-new.json",
            &serde_json::json!({ "openapi": "3.1.0", "idx": "new" }),
        );

        assert_eq!(super::SPEC_CACHE.len(), MAX_SPEC_CACHE_ENTRIES);
        assert!(!super::SPEC_CACHE.contains_key("https://example.com/spec-0.json"));
        assert!(super::SPEC_CACHE.contains_key("https://example.com/spec-new.json"));
    }
}
