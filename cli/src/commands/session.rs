use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{OutputFormat, SessionCommands};

pub async fn run(command: SessionCommands) -> Result<()> {
    match command {
        SessionCommands::Status { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let sessions: Value = api.get("/sessions").await?;
            let report = current_status(&sessions);
            match auth.output {
                OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
                OutputFormat::Table => {
                    eprintln!(
                        "Session: {}",
                        report["status"].as_str().unwrap_or("unavailable")
                    );
                    eprintln!(
                        "Refresh token expires: {}",
                        display_time(&report, "refresh_expires_at")
                    );
                    eprintln!(
                        "Absolute session expiry: {}",
                        display_time(&report, "session_expires_at")
                    );
                    if let Some(warning) = report["expiry_warning"].as_str() {
                        eprintln!("Warning: {warning}");
                    }
                }
            }
            Ok(())
        }
        SessionCommands::Refresh { auth } => {
            let report = crate::auth::force_refresh_session(&auth).await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": "refreshed",
                        "access_expires_at": report.access_expires_at,
                        "refresh_expires_at": report.refresh_expires_at,
                    }))?
                ),
                OutputFormat::Table => {
                    eprintln!("Session renewed.");
                    if let Some(exp) = report.access_expires_at {
                        eprintln!("Access token valid until:  {}", exp.to_rfc3339());
                    }
                    if let Some(exp) = report.refresh_expires_at {
                        eprintln!(
                            "Refresh token valid until: {} (rolling; cannot extend the absolute session expiry)",
                            exp.to_rfc3339()
                        );
                    }
                }
            }
            Ok(())
        }
        SessionCommands::List { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let mut sessions: Value = api.get("/sessions").await?;
            if let Some(rows) = sessions.as_array_mut() {
                for row in rows {
                    add_expiry_metadata(row);
                }
            }

            match auth.output {
                OutputFormat::Json => {
                    println!("{}", serde_json::to_string_pretty(&sessions)?);
                }
                OutputFormat::Table => {
                    let items = sessions.as_array();
                    if let Some(items) = items {
                        if items.is_empty() {
                            eprintln!("No active sessions.");
                            return Ok(());
                        }

                        let mut table = Table::new();
                        table.load_preset(UTF8_FULL_CONDENSED);
                        table.set_header([
                            "ID",
                            "Client",
                            "IP",
                            "Created",
                            "Refresh expires",
                            "Session expires (absolute)",
                        ]);

                        for session in items {
                            let id = session["id"]
                                .as_str()
                                .or(session["_id"].as_str())
                                .unwrap_or("-");
                            let short_id = crate::commands::short_id(id);
                            let client = session["user_agent"].as_str().unwrap_or("-");
                            let ip = session["ip_address"].as_str().unwrap_or("-");
                            let created = session["created_at"].as_str().unwrap_or("-");
                            let expires = display_time(session, "session_expires_at");
                            let refresh = display_time(session, "refresh_expires_at");
                            table.add_row([short_id, client, ip, created, refresh, expires]);
                            if let Some(warning) = session["expiry_warning"].as_str() {
                                eprintln!("Warning ({short_id}): {warning}");
                            }
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }
    }
}

fn display_time<'a>(row: &'a Value, field: &str) -> &'a str {
    row[field].as_str().unwrap_or("unknown")
}

/// Old replicas expose only expires_at. Never infer a fresh 30-day lifetime
/// from an access token's iat, which changes on rotation.
fn add_expiry_metadata(row: &mut Value) {
    if !row.is_object() {
        return;
    }
    if row.get("session_expires_at").is_none() {
        row["session_expires_at"] = row.get("expires_at").cloned().unwrap_or(Value::Null);
    }
    if row.get("refresh_expires_at").is_none() {
        row["refresh_expires_at"] = Value::Null;
    }
    row["expiry_warning"] = expiry_warning(row["session_expires_at"].as_str(), chrono::Utc::now())
        .map(Value::String)
        .unwrap_or(Value::Null);
}

fn expiry_warning(expiry: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> Option<String> {
    let expiry = chrono::DateTime::parse_from_rfc3339(expiry?).ok()?;
    if expiry > now + chrono::Duration::days(3) {
        return None;
    }
    Some(format!(
        "Account session reaches its absolute expiry at {}. Refresh cannot extend it; run `nyxid login` before then. For unattended use, authorize `nyxid login --agent-key` with suitable scopes and key expiry.",
        expiry.to_rfc3339()
    ))
}

fn current_status(sessions: &Value) -> Value {
    let current = sessions
        .as_array()
        .and_then(|rows| rows.iter().find(|row| row["is_current"] == true));
    let mut report = current.cloned().unwrap_or_else(|| serde_json::json!({}));
    add_expiry_metadata(&mut report);
    report["status"] = Value::String(
        if current.is_some() {
            "active"
        } else {
            "unavailable"
        }
        .into(),
    );
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{mock_auth, mock_auth_with_output};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[tokio::test]
    async fn list_sessions_json() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/sessions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "sess-12345678", "user_agent": "cli", "ip_address": "1.2.3.4",
                 "created_at": "2026-01-01", "expires_at": "2026-02-01"}
            ])))
            .expect(1)
            .mount(&server)
            .await;

        run(SessionCommands::List {
            auth: mock_auth(server.uri()),
        })
        .await
        .expect("session list json should succeed");
    }

    #[tokio::test]
    async fn list_sessions_table_renders_rows() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/sessions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
                {"id": "sess-abcdef99", "user_agent": "browser", "ip_address": "5.6.7.8",
                 "created_at": "2026-01-01", "expires_at": "2026-02-01"}
            ])))
            .mount(&server)
            .await;

        run(SessionCommands::List {
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("session list table should succeed");
    }

    #[tokio::test]
    async fn list_sessions_table_empty() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/sessions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
            .mount(&server)
            .await;

        run(SessionCommands::List {
            auth: mock_auth_with_output(server.uri(), OutputFormat::Table),
        })
        .await
        .expect("empty session list should succeed");
    }
}

#[cfg(test)]
mod expiry_tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn status_preserves_deadlines_and_warns_before_absolute_expiry() {
        let expiry = chrono::Utc::now() + chrono::Duration::days(2);
        let report = current_status(
            &json!([{"is_current": true, "expires_at": expiry.to_rfc3339(), "refresh_expires_at": "2026-12-01T00:00:00Z"}]),
        );
        assert_eq!(report["status"], "active");
        assert_eq!(report["session_expires_at"], expiry.to_rfc3339());
        assert_eq!(report["refresh_expires_at"], "2026-12-01T00:00:00Z");
        assert!(
            report["expiry_warning"]
                .as_str()
                .unwrap()
                .contains("--agent-key")
        );
    }
    #[test]
    fn status_on_old_replica_or_unbound_credentials_never_invents_deadlines() {
        let report =
            current_status(&json!([{"is_current":true, "expires_at":"2099-01-01T00:00:00Z"}]));
        assert!(report["refresh_expires_at"].is_null());
        assert!(report["expiry_warning"].is_null());
        assert_eq!(report["session_expires_at"], "2099-01-01T00:00:00Z");
        let report =
            current_status(&json!([{"is_current":false, "expires_at":"2099-01-01T00:00:00Z"}]));
        assert!(report["session_expires_at"].is_null());
        assert_eq!(report["status"], "unavailable");
    }
    #[test]
    fn warning_boundary_is_three_days() {
        let now = chrono::Utc::now();
        assert!(
            expiry_warning(Some(&(now + chrono::Duration::days(3)).to_rfc3339()), now).is_some()
        );
        assert!(
            expiry_warning(
                Some(
                    &(now + chrono::Duration::days(3) + chrono::Duration::seconds(1)).to_rfc3339()
                ),
                now
            )
            .is_none()
        );
    }
}
