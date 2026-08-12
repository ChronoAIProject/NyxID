use std::io::{IsTerminal, Write};
use std::time::Duration;

use anyhow::{Result, bail};
use serde::{Deserialize, Serialize};
use tokio::time::{Instant, sleep};

use crate::api::ApiClient;
use crate::cli::{ConnectArgs, OutputFormat};

#[derive(Debug, Serialize)]
struct CreateConnectLinkRequest<'a> {
    service_slug: &'a str,
    label: Option<&'a str>,
}

#[derive(Clone, Deserialize, Serialize)]
struct CreateConnectLinkResponse {
    id: String,
    connect_url: String,
    expires_at: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ConnectedService {
    id: String,
    slug: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct ConnectLinkStatus {
    id: String,
    status: String,
    service_name: String,
    service_slug: String,
    expires_at: String,
    completed_at: Option<String>,
    connected_service: Option<ConnectedService>,
}

#[derive(Serialize)]
struct ConnectOutput<'a> {
    id: &'a str,
    connect_url: &'a str,
    expires_at: &'a str,
    status: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    service_slug: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    user_service_id: Option<&'a str>,
}

pub async fn run(args: ConnectArgs) -> Result<()> {
    let mut api = ApiClient::from_auth_checked(&args.auth).await?;
    let created: CreateConnectLinkResponse = api
        .post(
            "/connect-links",
            &CreateConnectLinkRequest {
                service_slug: &args.service_slug,
                label: args.label.as_deref(),
            },
        )
        .await?;

    eprintln!("Connect {} in your browser:", args.service_slug);
    eprintln!("  {}", created.connect_url);

    if args.no_wait {
        print_result(&args, &created, "pending", None)?;
        return Ok(());
    }

    maybe_open_browser(&created.connect_url);
    eprintln!("Waiting for the connection to complete...");
    let deadline = Instant::now() + Duration::from_secs(args.timeout);
    loop {
        let status: ConnectLinkStatus = api.get(&format!("/connect-links/{}", created.id)).await?;
        if is_terminal_status(&status.status) {
            match status.status.as_str() {
                "completed" => {
                    print_result(
                        &args,
                        &created,
                        &status.status,
                        status.connected_service.as_ref(),
                    )?;
                    return Ok(());
                }
                "expired" => bail!("Connect link expired before the service was connected"),
                "cancelled" => bail!("Connect link was cancelled"),
                other => bail!("Connect link ended in unexpected state '{other}'"),
            }
        }
        if Instant::now() >= deadline {
            bail!(
                "Timed out waiting for the connection. Resume polling with GET /api/v1/connect-links/{}",
                created.id
            );
        }
        sleep(Duration::from_secs(1)).await;
    }
}

fn maybe_open_browser(url: &str) {
    let interactive = std::io::stdin().is_terminal() && std::io::stderr().is_terminal();
    if !interactive {
        return;
    }
    eprint!("\nOpen in your browser? [Y/n] ");
    std::io::stderr().flush().ok();
    let mut answer = String::new();
    if std::io::stdin().read_line(&mut answer).is_ok() {
        let answer = answer.trim().to_ascii_lowercase();
        if (answer.is_empty() || answer == "y" || answer == "yes")
            && let Err(error) = crate::browser::open_browser(url)
        {
            eprintln!("Could not open browser: {error}. Paste the URL above manually.");
        }
    }
}

fn print_result(
    args: &ConnectArgs,
    created: &CreateConnectLinkResponse,
    status: &str,
    connected: Option<&ConnectedService>,
) -> Result<()> {
    let stdout = std::io::stdout();
    let mut output = stdout.lock();
    write_result(&mut output, args.auth.output, created, status, connected)
}

fn write_result(
    output: &mut impl Write,
    format: OutputFormat,
    created: &CreateConnectLinkResponse,
    status: &str,
    connected: Option<&ConnectedService>,
) -> Result<()> {
    match format {
        OutputFormat::Json => {
            writeln!(
                output,
                "{}",
                serde_json::to_string_pretty(&ConnectOutput {
                    id: &created.id,
                    connect_url: &created.connect_url,
                    expires_at: &created.expires_at,
                    status,
                    service_slug: connected.map(|service| service.slug.as_str()),
                    user_service_id: connected.map(|service| service.id.as_str()),
                })?
            )?;
        }
        OutputFormat::Table => {
            if let Some(service) = connected {
                writeln!(output, "Connected: {}", service.slug)?;
            } else {
                writeln!(
                    output,
                    "Connect link created. It expires at {}.",
                    created.expires_at
                )?;
            }
        }
    }
    Ok(())
}

fn is_terminal_status(status: &str) -> bool {
    matches!(status, "completed" | "expired" | "cancelled")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mock_auth_with_output;
    use clap::Parser;
    use wiremock::matchers::{body_json, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn connect_command_parses_dx_flags() {
        let cli = crate::cli::Cli::parse_from([
            "nyxid",
            "connect",
            "github",
            "--label",
            "Agent connection",
            "--no-wait",
            "--timeout",
            "120",
            "--output",
            "json",
        ]);
        let crate::cli::Commands::Connect(args) = cli.command else {
            panic!("expected connect command");
        };
        assert_eq!(args.service_slug, "github");
        assert_eq!(args.label.as_deref(), Some("Agent connection"));
        assert!(args.no_wait);
        assert_eq!(args.timeout, 120);
        assert!(matches!(args.auth.output, OutputFormat::Json));
    }

    #[test]
    fn only_documented_statuses_are_terminal() {
        assert!(!is_terminal_status("pending"));
        for status in ["completed", "expired", "cancelled"] {
            assert!(is_terminal_status(status));
        }
    }

    #[test]
    fn table_result_is_written_to_the_output_stream() {
        let created = CreateConnectLinkResponse {
            id: "1c4e4357-75ec-431a-8c88-9913475cfb78".to_string(),
            connect_url: "https://app.example.test/connect/nyx_clk_token".to_string(),
            expires_at: "2026-08-05T10:15:00Z".to_string(),
        };
        let connected = ConnectedService {
            id: "c859ff2a-9a25-4907-a4b4-c22bd33897af".to_string(),
            slug: "github".to_string(),
        };
        let mut output = Vec::new();

        write_result(
            &mut output,
            OutputFormat::Table,
            &created,
            "completed",
            Some(&connected),
        )
        .expect("write table result");

        assert_eq!(
            String::from_utf8(output).expect("UTF-8 output"),
            "Connected: github\n"
        );
    }

    #[tokio::test]
    async fn no_wait_creates_link_with_expected_request() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/connect-links"))
            .and(body_json(serde_json::json!({
                "service_slug": "github",
                "label": "Coding agent",
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "1c4e4357-75ec-431a-8c88-9913475cfb78",
                "connect_url": "https://app.example.test/connect/nyx_clk_token",
                "expires_at": "2026-08-05T10:15:00Z",
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ConnectArgs {
            service_slug: "github".to_string(),
            label: Some("Coding agent".to_string()),
            no_wait: true,
            timeout: 30,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("create connect link");
    }

    #[tokio::test]
    async fn waiting_returns_after_completed_status() {
        let server = MockServer::start().await;
        let link_id = "1c4e4357-75ec-431a-8c88-9913475cfb78";
        Mock::given(method("POST"))
            .and(path("/api/v1/connect-links"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": link_id,
                "connect_url": "https://app.example.test/connect/nyx_clk_token",
                "expires_at": "2026-08-05T10:15:00Z",
            })))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path(format!("/api/v1/connect-links/{link_id}")))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": link_id,
                "status": "completed",
                "service_name": "GitHub",
                "service_slug": "github",
                "expires_at": "2026-08-05T10:15:00Z",
                "completed_at": "2026-08-05T10:01:00Z",
                "connected_service": {
                    "id": "c859ff2a-9a25-4907-a4b4-c22bd33897af",
                    "slug": "github",
                },
            })))
            .expect(1)
            .mount(&server)
            .await;

        run(ConnectArgs {
            service_slug: "github".to_string(),
            label: None,
            no_wait: false,
            timeout: 30,
            auth: mock_auth_with_output(server.uri(), OutputFormat::Json),
        })
        .await
        .expect("wait for completed connect link");
    }
}
