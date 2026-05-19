use anyhow::{Context, Result};
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;
use tokio::io::AsyncWriteExt;

use crate::api::ApiClient;
use crate::cli::{OutputFormat, ProxyCommands};

pub async fn run(command: ProxyCommands) -> Result<()> {
    match command {
        ProxyCommands::Discover { auth } => {
            let mut api = ApiClient::from_auth(&auth)?;
            let services: Value = api.get("/proxy/services").await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&services)?);
                }
                OutputFormat::Table => {
                    let items = services
                        .get("services")
                        .and_then(|v| v.as_array())
                        .or_else(|| services.as_array());
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No proxyable services found.");
                            eprintln!("Use `nyxid service add <slug>` to add a service.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header(["Slug", "Name", "Status", "Proxy URL"]);

                        for svc in items {
                            let slug = svc["slug"]
                                .as_str()
                                .or(svc["service_slug"].as_str())
                                .unwrap_or("-");
                            let name = svc["name"]
                                .as_str()
                                .or(svc["label"].as_str())
                                .unwrap_or("-");
                            let status = svc["status"].as_str().unwrap_or("active");
                            let proxy_url =
                                format!("{}/api/v1/proxy/s/{slug}/", api.base_url_root());
                            table.add_row([slug, name, status, &proxy_url]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }

        ProxyCommands::Request {
            service,
            path,
            method,
            data,
            headers,
            stream,
            by_id,
            via_service,
            auth,
        } => {
            let mut api = ApiClient::from_auth(&auth)?;

            // Build proxy path
            let trimmed_path = path.trim_start_matches('/');
            let mut proxy_path = if by_id {
                if trimmed_path.is_empty() {
                    format!("/proxy/{service}")
                } else {
                    format!("/proxy/{service}/{trimmed_path}")
                }
            } else if trimmed_path.is_empty() {
                format!("/proxy/s/{service}")
            } else {
                format!("/proxy/s/{service}/{trimmed_path}")
            };

            // Append ?_nyxid_via= so the server uses a specific
            // UserService instead of the auto-resolution cascade.
            if let Some(ref us_id) = via_service {
                let sep = if proxy_path.contains('?') { "&" } else { "?" };
                proxy_path.push_str(&format!("{sep}_nyxid_via={}", urlencoding::encode(us_id)));
            }

            // Parse headers
            let parsed_headers: Vec<(String, String)> = headers
                .iter()
                .filter_map(|h| {
                    let mut parts = h.splitn(2, ':');
                    let key = parts.next()?.trim().to_string();
                    let value = parts.next()?.trim().to_string();
                    Some((key, value))
                })
                .collect();

            // Resolve request body (supports binary via @file or stdin)
            let body: Option<Vec<u8>> = match data.as_deref() {
                Some("-") => {
                    let mut buf = Vec::new();
                    std::io::Read::read_to_end(&mut std::io::stdin(), &mut buf)
                        .context("Failed to read stdin")?;
                    Some(buf)
                }
                Some(d) if d.starts_with('@') => {
                    let path = &d[1..];
                    Some(
                        std::fs::read(path)
                            .with_context(|| format!("Failed to read file: {path}"))?,
                    )
                }
                Some(d) => Some(d.as_bytes().to_vec()),
                None => None,
            };

            let resp = api
                .proxy_request(&method, &proxy_path, &parsed_headers, body.as_deref())
                .await?;

            let status = resp.status();
            if !status.is_success() {
                let body = resp.text().await.unwrap_or_default();
                eprintln!("Proxy request failed (HTTP {status})");
                println!("{body}");
                return Ok(());
            }

            if stream {
                let mut stdout = tokio::io::stdout();
                let mut byte_stream = resp.bytes_stream();
                use futures::StreamExt;
                while let Some(chunk) = byte_stream.next().await {
                    let bytes = chunk.context("Failed to read response chunk")?;
                    let bytes: &[u8] = &bytes;
                    stdout
                        .write_all(bytes)
                        .await
                        .context("Failed to write to stdout")?;
                    stdout.flush().await.context("Failed to flush stdout")?;
                }
            } else {
                let body = resp.text().await.context("Failed to read response body")?;
                println!("{body}");
            }

            Ok(())
        }
    }
}
