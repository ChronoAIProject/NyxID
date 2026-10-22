use anyhow::Result;
use comfy_table::{Table, presets::UTF8_FULL_CONDENSED};
use serde_json::Value;

use crate::api::ApiClient;
use crate::cli::{OutputFormat, SessionCommands};

pub async fn run(command: SessionCommands) -> Result<()> {
    match command {
        SessionCommands::Status { auth } => run_status(&auth).await,
        SessionCommands::Refresh { auth } => {
            let report = crate::auth::force_refresh_session(&auth).await?;
            match auth.output {
                OutputFormat::Json => println!(
                    "{}",
                    serde_json::to_string_pretty(&serde_json::json!({
                        "status": "refreshed",
                        "access_expires_at": report.access_expires_at,
                        "refresh_expires_at": report.refresh_expires_at,
                        "session_expires_at": report.session_expires_at,
                    }))?
                ),
                OutputFormat::Table => {
                    eprintln!("Session renewed.");
                    if let Some(exp) = report.access_expires_at {
                        eprintln!("Access token valid until:  {}", exp.to_rfc3339());
                    }
                    if let Some(exp) = report.refresh_expires_at {
                        eprintln!(
                            "Refresh token valid until: {} (rolls forward on every renewal)",
                            exp.to_rfc3339()
                        );
                    }
                }
            }
            Ok(())
        }
        SessionCommands::List { auth } => {
            let mut api = ApiClient::from_auth_checked(&auth).await?;
            let sessions: Value = api.get("/sessions").await?;

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
                        table.set_header(["ID", "Current", "Client", "IP", "Created", "Expires"]);

                        for session in items {
                            let id = session["id"]
                                .as_str()
                                .or(session["_id"].as_str())
                                .unwrap_or("-");
                            let short_id = crate::commands::short_id(id);
                            let client = session["user_agent"].as_str().unwrap_or("-");
                            let ip = session["ip_address"].as_str().unwrap_or("-");
                            let created = session["created_at"].as_str().unwrap_or("-");
                            let expires = session["expires_at"].as_str().unwrap_or("-");
                            table.add_row([
                                short_id,
                                if session["is_current"].as_bool() == Some(true) {
                                    "yes"
                                } else {
                                    ""
                                },
                                client,
                                ip,
                                created,
                                expires,
                            ]);
                        }
                        eprintln!("{table}");
                    }
                }
            }
            Ok(())
        }
    }
}

#[derive(Debug)]
pub(crate) struct SessionStatusExit(pub i32);
impl std::fmt::Display for SessionStatusExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "session status exit {}", self.0)
    }
}
impl std::error::Error for SessionStatusExit {}

async fn run_status(auth: &crate::cli::AuthArgs) -> Result<()> {
    let (report, exit) = saved_status(auth).await;
    match auth.output {
        OutputFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
        OutputFormat::Table => eprintln!("{}", serde_json::to_string_pretty(&report)?),
    }
    if exit == 0 {
        Ok(())
    } else {
        Err(SessionStatusExit(exit).into())
    }
}

fn strict_read(dir: &std::path::Path, name: &str) -> Result<Option<String>> {
    match std::fs::read_to_string(dir.join(name)) {
        Ok(value) => Ok((!value.trim().is_empty()).then(|| value.trim().to_owned())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

pub(crate) async fn saved_status(auth: &crate::cli::AuthArgs) -> (Value, i32) {
    use crate::auth::{RefreshRejection, RefreshRejectionReason, jwt_exp_from_token};
    let mut report = serde_json::json!({
        "status": "unknown", "profile": auth.profile.as_deref().unwrap_or("default"),
        "auth_kind": "none", "local_state": "storage_error", "renewal": "unknown",
        "access_expires_at": null, "refresh_issued_at": null, "refresh_expires_at": null,
        "refresh_issued_lifetime_secs": null, "refresh_expired": null,
        "server_check": "skipped", "skip_reason": "storage_error", "session_expires_at": null,
        "renew_by": null, "renew_by_source": "local_refresh",
        "refresh_usability_tested": false,
        "credential_overrides_ignored": auth.access_token_env != "NYXID_ACCESS_TOKEN"
            || crate::auth::resolve_access_token_override(auth).is_some(),
    });
    let snapshot = async {
        let _lock = crate::auth::acquire_refresh_lock_async(auth.profile.as_deref()).await?;
        let dir = crate::auth::token_dir_for_profile(auth.profile.as_deref())?;
        let kind = strict_read(&dir, "auth_kind")?;
        let agent = kind.as_deref() == Some("agent_key");
        let access = strict_read(&dir, if agent { "token" } else { "access_token" })?;
        let refresh = strict_read(&dir, "refresh_token")?;
        let generation = strict_read(&dir, "login_generation")?;
        let saved_destination = strict_read(&dir, "base_url")?;
        let destination = auth.base_url.clone().or_else(|| saved_destination.clone());
        if let Some(destination) = &destination {
            crate::auth::login_exchange::normalized_destination(destination)?;
        }
        if let Some(saved) = &saved_destination {
            anyhow::ensure!(
                crate::auth::login_exchange::normalized_destination(saved)?
                    == crate::auth::login_exchange::normalized_destination(
                        destination
                            .as_deref()
                            .expect("saved destination is captured")
                    )?,
                "Profile destination differs"
            );
        }
        Ok::<_, anyhow::Error>((agent, access, refresh, generation, destination))
    }
    .await;
    let (agent, access, refresh, _generation, destination) = match snapshot {
        Ok(value) => value,
        Err(_) => {
            report["error"] = "profile_storage_error".into();
            return (report, 1);
        }
    };
    if agent {
        report["auth_kind"] = "agent_key".into();
        report["local_state"] = "ok".into();
        report["skip_reason"] = "agent_key".into();
        report["status"] = "saved".into();
        return (report, 0);
    }
    report["auth_kind"] = if access.is_some() || refresh.is_some() {
        "session"
    } else {
        "none"
    }
    .into();
    let access_exp = access.as_deref().and_then(jwt_exp_from_token);
    let refresh_exp = refresh.as_deref().and_then(jwt_exp_from_token);
    let issued = refresh.as_deref().and_then(|token| {
        use base64::Engine;
        let decoded = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(token.split('.').nth(1)?)
            .ok()?;
        let body: Value = serde_json::from_slice(&decoded).ok()?;
        chrono::DateTime::from_timestamp(body["iat"].as_i64()?, 0)
    });
    let expired = refresh_exp.map(|expiry| expiry <= chrono::Utc::now());
    report["access_expires_at"] = serde_json::json!(access_exp);
    report["refresh_expires_at"] = serde_json::json!(refresh_exp);
    report["refresh_issued_at"] = serde_json::json!(issued);
    report["refresh_issued_lifetime_secs"] = serde_json::json!(
        issued
            .zip(refresh_exp)
            .map(|(iat, exp)| (exp - iat).num_seconds())
    );
    report["refresh_expired"] = serde_json::json!(expired);
    report["renew_by"] = serde_json::json!(refresh_exp);
    let local = if access.is_none() && refresh.is_none() {
        "no_credentials"
    } else if refresh.is_none() {
        "access_only"
    } else if expired == Some(true) {
        "refresh_expired"
    } else if refresh_exp.is_none() {
        "refresh_unparseable"
    } else if access.is_none() {
        "refresh_only"
    } else {
        "ok"
    };
    report["local_state"] = local.into();
    let eligible = access_exp.is_some_and(|expiry| expiry > chrono::Utc::now());
    if eligible {
        let checked = match destination.as_deref() {
            Some(destination) => {
                check_server(
                    destination,
                    access.as_deref().expect("expiry implies access"),
                    auth.profile.as_deref(),
                )
                .await
            }
            None => serde_json::json!({
                "server_check": "unavailable", "unavailable_reason": "profile_destination_missing"
            }),
        };
        for (key, value) in checked.as_object().expect("server report object") {
            report[key] = value.clone();
        }
        report.as_object_mut().unwrap().remove("skip_reason");
        if report["server_check"] == "verified" {
            let server_exp = report["session_expires_at"]
                .as_str()
                .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
                .map(|s| s.with_timezone(&chrono::Utc));
            report["renew_by"] = serde_json::json!(match (refresh_exp, server_exp) {
                (Some(local), Some(server)) => Some(local.min(server)),
                (local, _) => local,
            });
            report["renew_by_source"] = "local_refresh_and_server_session".into();
        }
    } else {
        report["skip_reason"] = if access.is_none() {
            if refresh.is_some() {
                "no_access_token"
            } else {
                "no_session"
            }
        } else if access_exp.is_none() {
            "access_unparseable"
        } else {
            "access_token_expired"
        }
        .into();
    }
    let error = match local {
        "no_credentials" | "access_only" => Some("refresh_token_not_found"),
        "refresh_expired" => Some("refresh_token_expired"),
        _ if report["server_check"] == "session_ended" => {
            let body = serde_json::json!({"message": report["server_message"]});
            Some(match RefreshRejection::classify(Some(&body)).reason {
                RefreshRejectionReason::Revoked => "session_revoked",
                RefreshRejectionReason::SessionExpired => "session_expired",
                _ => "session_ended",
            })
        }
        _ => None,
    };
    report["status"] = if error.is_some() {
        "login_required"
    } else {
        "saved"
    }
    .into();
    if let Some(error) = error {
        report["error"] = error.into();
        (report, 3)
    } else {
        (report, 0)
    }
}

async fn check_server(destination: &str, access: &str, profile: Option<&str>) -> Value {
    let client = match crate::api::build_credential_http_client_for_url(profile, destination) {
        Ok(client) => client,
        Err(_) => return serde_json::json!({"server_check": "unavailable"}),
    };
    let response = match client
        .get(format!(
            "{}/api/v1/sessions",
            destination.trim_end_matches('/')
        ))
        .bearer_auth(access)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
    {
        Ok(response) => response,
        Err(_) => return serde_json::json!({"server_check": "unavailable"}),
    };
    let status = response.status();
    let body = response.json::<Value>().await.ok();
    let mut result = serde_json::json!({"http_status": status.as_u16()});
    let state = if status.is_success() {
        match body.as_ref().and_then(Value::as_array) {
            Some(items)
                if items.iter().all(|row| {
                    row["is_current"].is_boolean()
                        && row["expires_at"]
                            .as_str()
                            .is_some_and(|v| chrono::DateTime::parse_from_rfc3339(v).is_ok())
                }) =>
            {
                if let Some(row) = items.iter().find(|row| row["is_current"] == true) {
                    result["session_expires_at"] = row["expires_at"].clone();
                    "verified"
                } else {
                    "no_current_session"
                }
            }
            _ => "invalid_response",
        }
    } else if matches!(status.as_u16(), 401 | 403) {
        let message = body
            .as_ref()
            .and_then(|b| b["message"].as_str())
            .unwrap_or("");
        result["server_error"] = body
            .as_ref()
            .map(|b| b["error"].clone())
            .unwrap_or(Value::Null);
        result["access_token_expired_on_server"] =
            (result["server_error"] == "token_expired").into();
        let reason = match message.strip_prefix("Unauthorized: ").unwrap_or(message) {
            "Session has been revoked" => Some("revoked"),
            "Session has expired" => Some("expired"),
            "Session has ended" | "Session expired or revoked" => Some("ended"),
            _ => None,
        };
        if let Some(reason) = reason.filter(|_| result["server_error"] != "token_expired") {
            result["session_end_reason"] = reason.into();
            result["server_message"] = message.into();
            "session_ended"
        } else {
            "rejected"
        }
    } else if status.is_server_error() || matches!(status.as_u16(), 408 | 429) {
        "unavailable"
    } else {
        "unexpected_status"
    };
    result["server_check"] = state.into();
    result
}

pub(crate) async fn lookup_current_session_expiry(
    client: &reqwest::Client,
    destination: &str,
    access: &str,
) -> Option<chrono::DateTime<chrono::Utc>> {
    let response = client
        .get(format!(
            "{}/api/v1/sessions",
            destination.trim_end_matches('/')
        ))
        .bearer_auth(access)
        .timeout(std::time::Duration::from_secs(10))
        .send()
        .await
        .ok()?;
    if !response.status().is_success() {
        return None;
    }
    let rows: Value = response.json().await.ok()?;
    let current = rows
        .as_array()?
        .iter()
        .find(|row| row["is_current"] == true)?;
    chrono::DateTime::parse_from_rfc3339(current["expires_at"].as_str()?)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc))
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
mod status_tests {
    use super::*;
    use crate::auth::refresh_tests::{Home, args, jwt, runtime};
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, path},
    };
    #[test]
    fn saved_status_decision_table_and_cross_products_never_refresh() {
        let _home = Home::new();
        runtime().block_on(async {
            let server = MockServer::start().await;
            let mut auth = args(&server.uri());
            let dir = crate::auth::token_dir_for_profile(auth.profile.as_deref()).unwrap();
            std::fs::create_dir_all(&dir).unwrap();
            let future = jwt(chrono::Utc::now().timestamp() + 3600);
            let expired = jwt(1);
            let server_exp = (chrono::Utc::now() + chrono::Duration::minutes(30)).to_rfc3339();
            let cases = [
                (
                    "verified",
                    200,
                    serde_json::json!([{"is_current":true,"expires_at":server_exp}]),
                ),
                ("no_current_session", 200, serde_json::json!([])),
                (
                    "invalid_response",
                    200,
                    serde_json::json!({"unexpected":true}),
                ),
                (
                    "rejected",
                    401,
                    serde_json::json!({"error":"token_expired","message":"Token has expired"}),
                ),
                (
                    "rejected",
                    401,
                    serde_json::json!({"error":"unauthorized","message":"Rejected"}),
                ),
                (
                    "rejected",
                    403,
                    serde_json::json!({"error":"forbidden","message":"Forbidden"}),
                ),
                (
                    "session_ended",
                    401,
                    serde_json::json!({"message":"Unauthorized: Session has been revoked"}),
                ),
                (
                    "session_ended",
                    401,
                    serde_json::json!({"message":"Session has expired"}),
                ),
                (
                    "session_ended",
                    401,
                    serde_json::json!({"message":"Session has ended"}),
                ),
                (
                    "session_ended",
                    401,
                    serde_json::json!({"message":"Session expired or revoked"}),
                ),
                ("unexpected_status", 404, serde_json::json!({})),
                ("unavailable", 503, serde_json::json!({})),
                ("unavailable", 408, serde_json::json!({})),
                ("unavailable", 429, serde_json::json!({})),
            ];
            for (state, status, body) in cases {
                for (refresh, local, local_exit) in [
                    (Some(future.as_str()), "ok", 0),
                    (Some(expired.as_str()), "refresh_expired", 3),
                    (Some("opaque"), "refresh_unparseable", 0),
                    (None, "access_only", 3),
                ] {
                    server.reset().await;
                    Mock::given(path("/api/v1/auth/refresh"))
                        .respond_with(ResponseTemplate::new(500))
                        .expect(0)
                        .mount(&server)
                        .await;
                    Mock::given(path("/api/v1/sessions"))
                        .and(header("authorization", format!("Bearer {future}")))
                        .respond_with(ResponseTemplate::new(status).set_body_json(body.clone()))
                        .expect(1)
                        .mount(&server)
                        .await;
                    std::fs::write(dir.join("access_token"), &future).unwrap();
                    match refresh {
                        Some(r) => std::fs::write(dir.join("refresh_token"), r).unwrap(),
                        None => {
                            let _ = std::fs::remove_file(dir.join("refresh_token"));
                        }
                    }
                    auth.access_token = Some("ignored-caller".into());
                    let (report, exit) = saved_status(&auth).await;
                    assert_eq!(report["server_check"], state, "{report}");
                    assert_eq!(report["local_state"], local);
                    assert_eq!(
                        exit,
                        if local_exit == 3 || state == "session_ended" {
                            3
                        } else {
                            0
                        },
                        "{report}"
                    );
                    assert_eq!(report["renewal"], "unknown");
                    assert_eq!(report["credential_overrides_ignored"], true);
                    if state == "verified" {
                        assert_eq!(
                            report["renew_by_source"],
                            "local_refresh_and_server_session"
                        );
                    }
                    if local == "refresh_expired" {
                        assert_eq!(report["error"], "refresh_token_expired");
                    }
                    server.verify().await;
                }
            }
            server.reset().await;
            Mock::given(path("/api/v1/auth/refresh"))
                .respond_with(ResponseTemplate::new(500))
                .expect(0)
                .mount(&server)
                .await;
            Mock::given(path("/api/v1/sessions"))
                .respond_with(ResponseTemplate::new(500))
                .expect(0)
                .mount(&server)
                .await;
            for (access, refresh, local, skip, exit) in [
                (None, None, "no_credentials", "no_session", 3),
                (
                    None,
                    Some(future.as_str()),
                    "refresh_only",
                    "no_access_token",
                    0,
                ),
                (
                    Some(expired.as_str()),
                    Some(future.as_str()),
                    "ok",
                    "access_token_expired",
                    0,
                ),
                (
                    Some("opaque"),
                    Some("opaque"),
                    "refresh_unparseable",
                    "access_unparseable",
                    0,
                ),
                (
                    Some("opaque"),
                    Some(future.as_str()),
                    "ok",
                    "access_unparseable",
                    0,
                ),
            ] {
                for (name, value) in [("access_token", access), ("refresh_token", refresh)] {
                    match value {
                        Some(v) => std::fs::write(dir.join(name), v).unwrap(),
                        None => {
                            let _ = std::fs::remove_file(dir.join(name));
                        }
                    }
                }
                let (report, actual) = saved_status(&auth).await;
                assert_eq!(actual, exit, "{report}");
                assert_eq!(report["local_state"], local);
                assert_eq!(report["skip_reason"], skip);
            }
            std::fs::write(dir.join("auth_kind"), "agent_key").unwrap();
            let (report, exit) = saved_status(&auth).await;
            assert_eq!(exit, 0);
            assert_eq!(report["auth_kind"], "agent_key");
            std::fs::remove_file(dir.join("auth_kind")).unwrap();
            std::fs::create_dir(dir.join("auth_kind")).unwrap();
            let (report, exit) = saved_status(&auth).await;
            assert_eq!(exit, 1);
            assert_eq!(report["local_state"], "storage_error");
        });
    }
    #[test]
    fn saved_status_captures_one_login_before_concurrent_replacement() {
        let _home = Home::new();
        runtime().block_on(async {
            let server = MockServer::start().await;
            let auth = args(&server.uri());
            let profile = auth.profile.as_deref();
            let original = jwt(chrono::Utc::now().timestamp() + 3600);
            crate::auth::save_tokens_for(profile, &original, Some(&original)).unwrap();
            let replacement = jwt(chrono::Utc::now().timestamp() + 7200);
            let next = replacement.clone();
            Mock::given(path("/api/v1/sessions"))
                .and(header("authorization", format!("Bearer {original}")))
                .respond_with(move |_: &wiremock::Request| {
                    crate::auth::refresh_tests::replace_login(Some("refresh-1992"), || {
                        crate::auth::save_tokens_for(Some("refresh-1992"), &next, Some(&next))
                    })
                    .unwrap();
                    ResponseTemplate::new(200).set_body_json(serde_json::json!([]))
                })
                .expect(1)
                .mount(&server)
                .await;
            let (report, exit) = saved_status(&auth).await;
            assert_eq!(exit, 0);
            assert_eq!(
                report["access_expires_at"],
                serde_json::json!(crate::auth::jwt_exp_from_token(&original))
            );
            assert_eq!(
                crate::auth::read_saved_token_for(profile).unwrap(),
                replacement
            );
        });
    }
    #[test]
    fn saved_status_without_a_destination_still_reports_definitive_local_states() {
        let _home = Home::new();
        runtime().block_on(async {
            let mut auth = args("http://localhost:1");
            auth.base_url = None;
            let dir = crate::auth::token_dir_for_profile(auth.profile.as_deref()).unwrap();
            let (report, exit) = saved_status(&auth).await;
            assert_eq!(exit, 3, "{report}");
            assert_eq!(report["local_state"], "no_credentials");
            assert_eq!(report["error"], "refresh_token_not_found");
            assert_eq!(report["credential_overrides_ignored"], true);
            let token = jwt(chrono::Utc::now().timestamp() + 3600);
            std::fs::write(dir.join("refresh_token"), &token).unwrap();
            let (report, exit) = saved_status(&auth).await;
            assert_eq!(exit, 0, "{report}");
            assert_eq!(report["local_state"], "refresh_only");
            assert_eq!(report["skip_reason"], "no_access_token");
            std::fs::write(dir.join("access_token"), &token).unwrap();
            let (report, exit) = saved_status(&auth).await;
            assert_eq!(exit, 0, "{report}");
            assert_eq!(report["server_check"], "unavailable");
            assert_eq!(report["unavailable_reason"], "profile_destination_missing");
            std::fs::write(dir.join("auth_kind"), "agent_key").unwrap();
            let (report, exit) = saved_status(&auth).await;
            assert_eq!(exit, 0, "{report}");
            assert_eq!(report["auth_kind"], "agent_key");
            assert_eq!(report["skip_reason"], "agent_key");
        });
    }
}

#[cfg(test)]
#[test]
fn status_offline_is_unknown_and_renewable_credentials_are_preserved() {
    use crate::auth::refresh_tests::{Home, args, jwt, runtime};
    let _home = Home::new();
    runtime().block_on(async {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let auth = args(&format!("http://{address}"));
        let token = jwt(chrono::Utc::now().timestamp() + 3600);
        crate::auth::save_tokens_for(auth.profile.as_deref(), &token, Some(&token)).unwrap();
        let (report, exit) = saved_status(&auth).await;
        assert_eq!(exit, 0);
        assert_eq!(report["server_check"], "unavailable");
        assert_eq!(report["renewal"], "unknown");
        assert_eq!(
            crate::auth::read_saved_refresh_token_for(auth.profile.as_deref()).unwrap(),
            token
        );
    });
}
