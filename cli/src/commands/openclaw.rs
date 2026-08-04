use std::io::Write;

use anyhow::{Context, Result, bail};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{OpenClawCommands, OutputFormat};

pub async fn run(command: OpenClawCommands) -> Result<()> {
    match command {
        OpenClawCommands::Setup {
            url,
            token_env,
            auth,
        } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;

            let gateway_url = match url {
                Some(u) => u,
                None => {
                    eprint!("OpenClaw gateway URL: ");
                    std::io::stderr().flush()?;
                    let mut input = String::new();
                    std::io::stdin().read_line(&mut input)?;
                    let trimmed = input.trim().to_string();
                    if trimmed.is_empty() {
                        bail!("Gateway URL is required");
                    }
                    trimmed
                }
            };

            let credential = if let Some(env_var) = &token_env {
                std::env::var(env_var)
                    .with_context(|| format!("Environment variable {env_var} not set"))?
            } else {
                rpassword::prompt_password("Bearer token: ")?
            };
            if credential.is_empty() {
                bail!("Bearer token is required");
            }

            let body = serde_json::json!({
                "service_slug": "llm-openclaw",
                "credential": credential,
                "endpoint_url": gateway_url,
                "label": "OpenClaw",
            });

            let result: Value = api.post("/keys", &body).await?;

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&result)?);
                }
                OutputFormat::Table => {
                    let slug = result["slug"]
                        .as_str()
                        .or(result["service_slug"].as_str())
                        .unwrap_or("llm-openclaw");
                    let endpoint = result["endpoint_url"].as_str().unwrap_or(&gateway_url);
                    let status = result["status"].as_str().unwrap_or("active");

                    eprintln!("OpenClaw configured!");
                    eprintln!();
                    eprintln!("Slug:      {slug}");
                    eprintln!("Endpoint:  {endpoint}");
                    eprintln!("Status:    {status}");
                    eprintln!();
                    eprintln!("Proxy URL: {}/api/v1/proxy/s/{slug}/", api.base_url_root());
                    eprintln!();
                    eprintln!("Generate MCP config:");
                    eprintln!(
                        "  nyxid mcp config --tool claude-code --base-url {}",
                        auth.resolved_base_url()?
                    );
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::run;
    use crate::cli::OpenClawCommands;
    use crate::test_support::{env_lock, mock_auth};
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // `url` is passed explicitly so the command never blocks on stdin;
    // the bearer token is sourced from `token_env` so we exercise the
    // non-interactive credential path.
    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn setup_posts_expected_key_body() {
        let _guard = env_lock().lock().expect("env lock");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/keys"))
            .and(body_json(serde_json::json!({
                "service_slug": "llm-openclaw",
                "credential": "ocw-secret",
                "endpoint_url": "https://gateway.example.com",
                "label": "OpenClaw",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "slug": "llm-openclaw",
                "endpoint_url": "https://gateway.example.com",
                "status": "active"
            })))
            .expect(1)
            .mount(&server)
            .await;

        // SAFETY: env mutation serialized by env_lock above.
        unsafe {
            std::env::set_var("NYXID_TEST_OCW_TOKEN", "ocw-secret");
        }
        let result = run(OpenClawCommands::Setup {
            url: Some("https://gateway.example.com".to_string()),
            token_env: Some("NYXID_TEST_OCW_TOKEN".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await;
        unsafe {
            std::env::remove_var("NYXID_TEST_OCW_TOKEN");
        }
        result.expect("setup via env token should succeed");
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn setup_errors_when_token_env_unset() {
        let _guard = env_lock().lock().expect("env lock");
        // No server interaction expected: credential resolution fails first.
        let server = MockServer::start().await;

        // SAFETY: env mutation serialized by env_lock above.
        unsafe {
            std::env::remove_var("NYXID_TEST_OCW_MISSING");
        }
        let result = run(OpenClawCommands::Setup {
            url: Some("https://gateway.example.com".to_string()),
            token_env: Some("NYXID_TEST_OCW_MISSING".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await;
        assert!(
            result.is_err(),
            "missing token env var should surface as an error"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn setup_errors_when_token_env_empty() {
        let _guard = env_lock().lock().expect("env lock");
        let server = MockServer::start().await;

        // SAFETY: env mutation serialized by env_lock above.
        unsafe {
            std::env::set_var("NYXID_TEST_OCW_EMPTY", "");
        }
        let result = run(OpenClawCommands::Setup {
            url: Some("https://gateway.example.com".to_string()),
            token_env: Some("NYXID_TEST_OCW_EMPTY".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await;
        unsafe {
            std::env::remove_var("NYXID_TEST_OCW_EMPTY");
        }
        assert!(
            result.is_err(),
            "empty bearer token should be rejected before any HTTP call"
        );
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)]
    async fn setup_surfaces_server_error() {
        let _guard = env_lock().lock().expect("env lock");
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/keys"))
            .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
            .mount(&server)
            .await;

        // SAFETY: env mutation serialized by env_lock above.
        unsafe {
            std::env::set_var("NYXID_TEST_OCW_5XX", "ocw-secret");
        }
        let result = run(OpenClawCommands::Setup {
            url: Some("https://gateway.example.com".to_string()),
            token_env: Some("NYXID_TEST_OCW_5XX".to_string()),
            auth: mock_auth(server.uri()),
        })
        .await;
        unsafe {
            std::env::remove_var("NYXID_TEST_OCW_5XX");
        }
        assert!(result.is_err(), "5xx should surface as an error");
    }
}
