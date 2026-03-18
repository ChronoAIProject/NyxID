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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceDocumentationMetadata {
    pub openapi_spec_url: Option<String>,
    pub asyncapi_spec_url: Option<String>,
    pub streaming_supported: bool,
}

pub async fn discover_service_docs(
    client: &reqwest::Client,
    base_url: &str,
    explicit_openapi_spec_url: Option<String>,
    explicit_asyncapi_spec_url: Option<String>,
) -> ServiceDocumentationMetadata {
    let openapi_spec_url = match explicit_openapi_spec_url {
        Some(url) if !url.trim().is_empty() => {
            if fetch_json_spec(client, &url).await.is_ok() {
                Some(url)
            } else {
                None
            }
        }
        _ => discover_spec_url(client, base_url, OPENAPI_PROBE_PATHS).await,
    };

    let asyncapi_spec_url = match explicit_asyncapi_spec_url {
        Some(url) if !url.trim().is_empty() => {
            if fetch_json_spec(client, &url).await.is_ok() {
                Some(url)
            } else {
                None
            }
        }
        _ => discover_spec_url(client, base_url, ASYNCAPI_PROBE_PATHS).await,
    };

    let streaming_supported = if let Some(ref openapi_url) = openapi_spec_url {
        fetch_json_spec(client, openapi_url)
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

pub async fn fetch_downstream_openapi_spec(
    client: &reqwest::Client,
    service: &DownstreamService,
    proxy_base_url: &str,
) -> AppResult<serde_json::Value> {
    let spec_url = service
        .openapi_spec_url
        .as_deref()
        .ok_or_else(|| AppError::NotFound("Service has no OpenAPI spec configured".to_string()))?;

    let mut spec = fetch_json_spec(client, spec_url).await?;
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
    client: &reqwest::Client,
    service: &DownstreamService,
    proxy_base_url: &str,
) -> AppResult<serde_json::Value> {
    let spec_url = service
        .asyncapi_spec_url
        .as_deref()
        .ok_or_else(|| AppError::NotFound("Service has no AsyncAPI spec configured".to_string()))?;

    let mut spec = fetch_json_spec(client, spec_url).await?;
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
    format!(
        r#"<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1" />
    <title>{title}</title>
    <style>
      html, body, #app {{ height: 100%; margin: 0; }}
      body {{ background: #0f172a; }}
    </style>
  </head>
  <body>
    <script
      id="api-reference"
      data-url="{spec_url}"
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
            body.innerHTML = '<tr><td colspan="4">No proxyable services found.</td></tr>';
            return;
          }
          body.innerHTML = payload.services.map((service) => {
            const docs = [
              service.docs_url ? `<a href="${service.docs_url}">Scalar UI</a>` : 'Unavailable',
              service.openapi_url ? `<small><a href="${service.openapi_url}">OpenAPI</a></small>` : '',
              service.asyncapi_url ? `<small><a href="${service.asyncapi_url}">AsyncAPI</a></small>` : ''
            ].join('');
            return `<tr>
              <td><strong>${service.name}</strong><small>${service.slug}</small></td>
              <td>${docs}</td>
              <td>${service.streaming_supported ? '<span class="badge">Streaming</span>' : 'No'}</td>
              <td><a href="${service.proxy_url_slug.replace('{path}', '')}">${service.proxy_url_slug}</a></td>
            </tr>`;
          }).join('');
        })
        .catch((error) => {
          status.textContent = error.message;
          body.innerHTML = '<tr><td colspan="4">Failed to load catalog.</td></tr>';
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
            "description": "AsyncAPI document for NyxID streaming protocols, including node agent WebSockets, MCP SSE transport, and downstream proxy streaming."
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

async fn discover_spec_url(
    client: &reqwest::Client,
    base_url: &str,
    candidate_paths: &[&str],
) -> Option<String> {
    let base = base_url.trim_end_matches('/');
    for path in candidate_paths {
        let candidate = format!("{base}{path}");
        if fetch_json_spec(client, &candidate).await.is_ok() {
            return Some(candidate);
        }
    }
    None
}

async fn fetch_json_spec(client: &reqwest::Client, url: &str) -> AppResult<serde_json::Value> {
    let response = client
        .get(url)
        .timeout(std::time::Duration::from_secs(5))
        .send()
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to fetch spec: {e}")))?;

    if !response.status().is_success() {
        return Err(AppError::BadRequest(format!(
            "Spec returned HTTP {}",
            response.status()
        )));
    }

    let body = response
        .text()
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read spec body: {e}")))?;

    serde_json::from_str(&body)
        .map_err(|e| AppError::BadRequest(format!("Spec was not valid JSON: {e}")))
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

#[cfg(test)]
mod tests {
    use super::{
        ServiceDocumentationMetadata, build_asyncapi_document, catalog_csp,
        detect_streaming_from_openapi, render_scalar_html, scalar_docs_csp,
    };

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
    }

    #[test]
    fn scalar_html_embeds_spec_url() {
        let html = render_scalar_html("Docs", "/api/v1/docs/openapi.json");
        assert!(html.contains("/api/v1/docs/openapi.json"));
        assert!(html.contains("@scalar/api-reference"));
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
}
